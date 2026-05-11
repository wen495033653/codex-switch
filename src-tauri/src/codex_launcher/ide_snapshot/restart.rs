use super::{
    detect::{
        build_ide_summary, normalize_executable_path, normalize_ide_entries,
        process_entry_executable_path, process_entry_pid,
    },
    *,
};

pub(crate) fn restart_from_ide_snapshot<F>(
    snapshot: &Value,
    before_relaunch: F,
) -> Result<Value, String>
where
    F: FnOnce(),
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

    let pids: Vec<u64> = entries
        .iter()
        .map(process_entry_pid)
        .filter(|pid| *pid > 0)
        .collect();
    let mut executables: Vec<String> = entries
        .iter()
        .map(process_entry_executable_path)
        .filter(|path| !path.trim().is_empty())
        .collect();
    executables.sort_by_key(|path| normalize_executable_path(path));
    executables.dedup_by_key(|path| normalize_executable_path(path));

    for pid in &pids {
        let _ = kill_process_tree(*pid);
    }
    let mut alive = wait_for_pids_exit(&pids, 12_000);
    if !alive.is_empty() {
        for pid in &alive {
            let _ = kill_process_tree(*pid);
        }
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

    before_relaunch();

    let mut restarted_paths = HashSet::new();
    for executable in executables {
        if relaunch_executable_with_retry(&executable) {
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
