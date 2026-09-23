use super::detect::{
    build_ide_summary, normalize_executable_path, normalize_ide_entries,
    process_entry_executable_path, process_entry_parent_pid, process_entry_pid,
    process_entry_start_time,
};
use crate::{
    codex_launcher::process_control::{
        kill_root_process_trees, relaunch_executable_with_retry, wait_for_pids_exit,
    },
    json_util::string_field,
    session_sync_diagnostics::log_session_sync_event,
};
use serde_json::{json, Value};
use std::{collections::HashSet, thread, time::Duration as StdDuration};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

pub(crate) fn restart_from_ide_snapshot<F>(
    snapshot: &Value,
    before_relaunch: F,
) -> Result<Value, String>
where
    F: FnOnce() -> Result<(), String>,
{
    let entries = normalize_ide_entries(
        snapshot
            .get("entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    );
    if entries.is_empty() {
        return Ok(json!({
            "restarted": false,
            "restartedCount": 0,
            "summary": []
        }));
    }

    let live_entries = entries_still_running(&entries);
    if live_entries.len() != entries.len() {
        let live_pids = live_entries
            .iter()
            .map(process_entry_pid)
            .collect::<HashSet<_>>();
        log_session_sync_event(
            "ide_reopen_snapshot_process_gone",
            json!({
                "skippedPids": entries
                    .iter()
                    .map(process_entry_pid)
                    .filter(|pid| !live_pids.contains(pid))
                    .collect::<Vec<_>>(),
                "livePids": live_pids.iter().collect::<Vec<_>>()
            }),
        );
    }
    let process_tree: Vec<(u64, u64)> = live_entries
        .iter()
        .map(|entry| (process_entry_pid(entry), process_entry_parent_pid(entry)))
        .filter(|(pid, _)| *pid > 0)
        .collect();
    let pids: Vec<u64> = process_tree.iter().map(|(pid, _)| *pid).collect();
    let mut executables: Vec<String> = entries
        .iter()
        .map(process_entry_executable_path)
        .filter(|path| !path.trim().is_empty())
        .collect();
    executables.sort_by_key(|path| normalize_executable_path(path));
    executables.dedup_by_key(|path| normalize_executable_path(path));

    kill_root_process_trees(&process_tree)?;
    let mut alive = wait_for_pids_exit(&pids, 12_000);
    if !alive.is_empty() {
        let alive_tree = process_tree
            .iter()
            .copied()
            .filter(|(pid, _)| alive.contains(pid))
            .collect::<Vec<_>>();
        kill_root_process_trees(&alive_tree)?;
        alive = wait_for_pids_exit(&alive, 6_000);
    }
    if !alive.is_empty() {
        let summary = build_ide_summary(&entries);
        let names = summary
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|item| string_field(&item, "displayName"))
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>()
            .join("、");
        return Err(format!(
            "部分{}进程未能退出，请手动关闭后重试",
            if names.is_empty() {
                "编辑器".to_string()
            } else {
                names
            }
        ));
    }

    before_relaunch()?;

    let mut restarted_paths = HashSet::new();
    for executable in executables {
        let restarted = if is_codex_executable(&entries, &executable) {
            crate::codex_launcher::codex_app_open::relaunch_codex_executable_for_current_settings(
                &executable,
            )?
        } else {
            relaunch_executable_with_retry(&executable)?;
            true
        };
        if restarted {
            restarted_paths.insert(normalize_executable_path(&executable));
        }
        thread::sleep(StdDuration::from_millis(120));
    }

    let restarted_entries: Vec<Value> = entries
        .iter()
        .filter(|entry| {
            restarted_paths.contains(&normalize_executable_path(&process_entry_executable_path(
                entry,
            )))
        })
        .cloned()
        .collect();
    let summary = if restarted_entries.is_empty() {
        build_ide_summary(&entries)
    } else {
        build_ide_summary(&restarted_entries)
    };

    Ok(json!({
        "restarted": !restarted_paths.is_empty(),
        "restartedCount": restarted_paths.len(),
        "summary": summary
    }))
}

fn entry_matches_process(entry: &Value, start_time: u64, executable: Option<&str>) -> bool {
    process_entry_start_time(entry) == start_time
        && executable.is_some_and(|path| {
            normalize_executable_path(path)
                == normalize_executable_path(&process_entry_executable_path(entry))
        })
}

/// Entries whose process is still the one the snapshot saw: same PID, start time and
/// executable. A snapshot waits for the user's confirmation with no time limit, and a PID
/// freed in the meantime can belong to an unrelated process by then, so only confirmed
/// entries are terminated. Executables of gone entries are still reopened, as before.
fn entries_still_running(entries: &[Value]) -> Vec<Value> {
    let refresh_kind = ProcessRefreshKind::nothing()
        .with_exe(UpdateKind::Always)
        .without_tasks();
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind);
    entries
        .iter()
        .filter(|entry| {
            u32::try_from(process_entry_pid(entry))
                .ok()
                .and_then(|pid| system.process(Pid::from_u32(pid)))
                .is_some_and(|process| {
                    let executable = process.exe().map(|path| path.to_string_lossy());
                    entry_matches_process(entry, process.start_time(), executable.as_deref())
                })
        })
        .cloned()
        .collect()
}

fn is_codex_executable(entries: &[Value], executable: &str) -> bool {
    let target = normalize_executable_path(executable);
    entries.iter().any(|entry| {
        string_field(entry, "kind") == "codex"
            && normalize_executable_path(&process_entry_executable_path(entry)) == target
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current_process_entry(start_time_offset: u64, executable: &str) -> Value {
        let pid = std::process::id();
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            true,
            ProcessRefreshKind::nothing(),
        );
        let start_time = system
            .process(Pid::from_u32(pid))
            .expect("test process is visible")
            .start_time();
        json!({
            "pid": pid,
            "parentPid": 0,
            "startTime": start_time + start_time_offset,
            "executablePath": executable
        })
    }

    #[test]
    fn only_the_same_live_process_is_confirmed() {
        let executable = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let same = current_process_entry(0, &executable);
        let reused_pid = current_process_entry(1, &executable);
        let other_executable = current_process_entry(0, r"C:\Other\Code.exe");

        let live = entries_still_running(&[same.clone(), reused_pid, other_executable]);

        assert_eq!(live, vec![same]);
    }
}
