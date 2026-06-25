use super::detect_ide_app;
use crate::session_sync_diagnostics::log_session_sync_event;
use crate::time_util::now_string;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::Path,
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration as StdDuration, Instant},
};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

const WATCHER_INTERVAL_MS: u64 = 5_000;
const TAKEOVER_GRACE_MS: u64 = 500;
const PENDING_RELAUNCH_TTL_MS: u64 = 30_000;
const SUPPRESSED_OPEN_TTL_MS: u64 = 30_000;
const OPEN_ABSENCE_RESET_MS: u64 = 3_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodexProcess {
    pub(crate) pid: u64,
    parent_pid: u64,
    pub(crate) executable_path: String,
    pub(crate) command_line: String,
}

#[derive(Default)]
pub(crate) struct CodexAppOpenOutcome {
    pub(crate) relaunch_expected: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CodexAppOpenSignature {
    root_pids: Vec<u64>,
}

#[derive(Clone, Debug, Default)]
struct CodexAppWatcherSnapshot {
    captured_at: String,
    processes: Vec<CodexProcess>,
    error: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct ExpectedCodexAppOpen {
    executables: Vec<String>,
    source: String,
    until: Option<Instant>,
}

#[derive(Clone, Debug, Default)]
struct SuppressedCodexAppOpen {
    count: usize,
    source: String,
    until: Option<Instant>,
}

static CURRENT_CODEX_APP_PROCESSES: OnceLock<Mutex<CodexAppWatcherSnapshot>> = OnceLock::new();
static EXPECTED_CODEX_APP_OPEN: OnceLock<Mutex<ExpectedCodexAppOpen>> = OnceLock::new();
static SUPPRESSED_CODEX_APP_OPENS: OnceLock<Mutex<SuppressedCodexAppOpen>> = OnceLock::new();

fn current_codex_app_processes_state() -> &'static Mutex<CodexAppWatcherSnapshot> {
    CURRENT_CODEX_APP_PROCESSES.get_or_init(|| Mutex::new(CodexAppWatcherSnapshot::default()))
}

fn expected_codex_app_open_state() -> &'static Mutex<ExpectedCodexAppOpen> {
    EXPECTED_CODEX_APP_OPEN.get_or_init(|| Mutex::new(ExpectedCodexAppOpen::default()))
}

fn suppressed_codex_app_open_state() -> &'static Mutex<SuppressedCodexAppOpen> {
    SUPPRESSED_CODEX_APP_OPENS.get_or_init(|| Mutex::new(SuppressedCodexAppOpen::default()))
}

pub(crate) fn current_codex_app_processes_value() -> Result<Value, String> {
    let snapshot = current_codex_app_processes_state()
        .lock()
        .map_err(|_| "Codex app watcher 状态锁异常".to_string())?
        .clone();
    let pids = codex_root_pids(&snapshot.processes);
    let all_pids = codex_pids(&snapshot.processes);
    let entries = snapshot
        .processes
        .iter()
        .map(|process| {
            json!({
                "pid": process.pid,
                "parentPid": process.parent_pid,
                "name": executable_name(&process.executable_path),
                "executablePath": process.executable_path,
                "kind": "codex",
                "displayName": "Codex app"
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "capturedAt": if snapshot.captured_at.is_empty() { now_string() } else { snapshot.captured_at },
        "pids": pids,
        "allPids": all_pids,
        "processCount": snapshot.processes.len(),
        "entries": entries,
        "error": snapshot.error
    }))
}

pub(crate) fn refresh_current_codex_app_processes() -> Result<Vec<CodexProcess>, String> {
    let processes = running_codex_processes()?;
    update_current_codex_app_processes(processes.clone(), None);
    Ok(processes)
}

pub(crate) fn expect_codex_app_open_for_executables(executables: &[String]) {
    expect_codex_app_open_for_executables_from(executables, "expected_relaunch");
}

pub(crate) fn expect_app_command_codex_app_open_for_executables(executables: &[String]) {
    expect_codex_app_open_for_executables_from(executables, "app_command");
}

fn expect_codex_app_open_for_executables_from(executables: &[String], source: &str) {
    let keys = normalize_executable_keys(executables.iter().map(String::as_str));
    if keys.is_empty() {
        log_session_sync_event(
            "codex_app_watcher_expect_open_skip",
            json!({ "reason": "empty_executables", "source": source }),
        );
        return;
    }
    if let Ok(mut expected) = expected_codex_app_open_state().lock() {
        expected.executables = keys;
        expected.source = source.to_string();
        expected.until = Some(Instant::now() + StdDuration::from_millis(PENDING_RELAUNCH_TTL_MS));
        log_session_sync_event(
            "codex_app_watcher_expect_open_set",
            json!({
                "executables": expected.executables.clone(),
                "source": expected.source.clone(),
                "ttlMs": PENDING_RELAUNCH_TTL_MS
            }),
        );
    }
}

pub(crate) fn clear_expected_codex_app_open_for_executables(executables: &[String]) {
    let keys = normalize_executable_keys(executables.iter().map(String::as_str));
    if keys.is_empty() {
        return;
    }
    if let Ok(mut expected) = expected_codex_app_open_state().lock() {
        if expected.executables == keys {
            expected.executables.clear();
            expected.source.clear();
            expected.until = None;
            log_session_sync_event(
                "codex_app_watcher_expect_open_cleared",
                json!({ "executables": keys }),
            );
        }
    }
}

pub(crate) fn suppress_next_codex_app_open_handler(source: &str) {
    if let Ok(mut suppressed) = suppressed_codex_app_open_state().lock() {
        suppressed.count = suppressed.count.saturating_add(1);
        suppressed.source = source.to_string();
        suppressed.until = Some(Instant::now() + StdDuration::from_millis(SUPPRESSED_OPEN_TTL_MS));
        log_session_sync_event(
            "codex_app_watcher_suppress_open_set",
            json!({
                "source": suppressed.source.clone(),
                "count": suppressed.count,
                "ttlMs": SUPPRESSED_OPEN_TTL_MS
            }),
        );
    }
}

pub(crate) fn clear_suppressed_codex_app_open_handler(source: &str) {
    if let Ok(mut suppressed) = suppressed_codex_app_open_state().lock() {
        if suppressed.count == 0 || suppressed.source != source {
            return;
        }
        suppressed.count -= 1;
        if suppressed.count == 0 {
            suppressed.source.clear();
            suppressed.until = None;
        }
        log_session_sync_event(
            "codex_app_watcher_suppress_open_cleared",
            json!({
                "source": source,
                "remaining": suppressed.count
            }),
        );
    }
}

pub(crate) fn start_codex_app_open_watcher<F>(on_open: F)
where
    F: Fn(&[CodexProcess]) -> Result<CodexAppOpenOutcome, String> + Send + 'static,
{
    if !cfg!(windows) {
        log_session_sync_event(
            "codex_app_watcher_not_started",
            json!({ "reason": "non_windows" }),
        );
        return;
    }

    log_session_sync_event("codex_app_watcher_started", json!({}));
    thread::spawn(move || watch_codex_app(on_open));
}

fn watch_codex_app<F>(on_open: F)
where
    F: Fn(&[CodexProcess]) -> Result<CodexAppOpenOutcome, String>,
{
    let mut candidate_signature: Option<CodexAppOpenSignature> = None;
    let mut candidate_since: Option<Instant> = None;
    let mut open_signature: Option<CodexAppOpenSignature> = None;
    let mut pending_relaunch_executables = Vec::<String>::new();
    let mut pending_relaunch_until: Option<Instant> = None;
    let mut open_absence_since: Option<Instant> = None;
    let mut baseline_current_processes = true;

    loop {
        let now = Instant::now();
        if until_expired(pending_relaunch_until, now) {
            pending_relaunch_executables.clear();
            pending_relaunch_until = None;
        }

        let processes = match running_codex_processes() {
            Ok(processes) => processes,
            Err(err) => {
                eprintln!("Codex app watcher 检测失败: {err}");
                log_session_sync_event(
                    "codex_app_watcher_scan_error",
                    json!({ "error": err.clone() }),
                );
                update_current_codex_app_processes(Vec::new(), Some(err));
                reset_candidate(&mut candidate_signature, &mut candidate_since);
                sleep_interval();
                continue;
            }
        };
        update_current_codex_app_processes(processes.clone(), None);

        if processes.is_empty() {
            reset_candidate(&mut candidate_signature, &mut candidate_since);
            if pending_relaunch_executables.is_empty()
                && open_signature.is_some()
                && open_absence_elapsed(&mut open_absence_since, now)
            {
                log_session_sync_event(
                    "codex_app_watcher_open_signature_reset",
                    json!({ "reason": "process_absence_elapsed" }),
                );
                open_signature = None;
            }
            if baseline_current_processes {
                log_session_sync_event(
                    "codex_app_watcher_baseline_empty",
                    json!({ "reason": "no_processes_on_first_scan" }),
                );
                baseline_current_processes = false;
            }
            sleep_interval();
            continue;
        }

        open_absence_since = None;
        let signature = codex_open_signature(&processes);
        let executable_keys = codex_executable_keys(&processes);
        if let Some(expected_source) =
            take_expected_codex_app_open_source_if_matches(&executable_keys, now)
        {
            log_session_sync_event(
                "codex_app_watcher_expected_open_matched",
                json!({
                    "action": "skip_on_open_handler",
                    "source": expected_source,
                    "signature": codex_open_signature_log_value(&signature),
                    "executables": executable_keys.clone(),
                    "processes": codex_processes_log_value(&processes)
                }),
            );
            open_signature = Some(signature.clone());
            reset_candidate(&mut candidate_signature, &mut candidate_since);
            sleep_interval();
            continue;
        }
        if !pending_relaunch_executables.is_empty()
            && executable_keys == pending_relaunch_executables
        {
            log_session_sync_event(
                "codex_app_watcher_pending_relaunch_matched",
                json!({
                    "action": "mark_open_without_handler",
                    "signature": codex_open_signature_log_value(&signature),
                    "executables": executable_keys.clone()
                }),
            );
            open_signature = Some(signature.clone());
            pending_relaunch_executables.clear();
            pending_relaunch_until = None;
        }

        if baseline_current_processes {
            log_session_sync_event(
                "codex_app_watcher_baseline_existing_processes",
                json!({
                    "action": "set_open_signature_without_handler",
                    "signature": codex_open_signature_log_value(&signature),
                    "executables": executable_keys.clone(),
                    "processes": codex_processes_log_value(&processes)
                }),
            );
            candidate_signature = Some(signature.clone());
            open_signature = Some(signature.clone());
            pending_relaunch_executables.clear();
            pending_relaunch_until = None;
            open_absence_since = None;
            baseline_current_processes = false;
            sleep_interval();
            continue;
        }

        if open_signature.as_ref() == Some(&signature) {
            reset_candidate(&mut candidate_signature, &mut candidate_since);
            sleep_interval();
            continue;
        }

        if candidate_signature.as_ref() != Some(&signature) {
            log_session_sync_event(
                "codex_app_watcher_open_candidate_seen",
                json!({
                    "signature": codex_open_signature_log_value(&signature),
                    "executables": executable_keys.clone(),
                    "processes": codex_processes_log_value(&processes),
                    "graceMs": TAKEOVER_GRACE_MS
                }),
            );
            candidate_signature = Some(signature.clone());
            candidate_since = Some(now);
            sleep_interval();
            continue;
        }

        if candidate_since
            .map(|started| started.elapsed() < StdDuration::from_millis(TAKEOVER_GRACE_MS))
            .unwrap_or(true)
        {
            sleep_interval();
            continue;
        }

        if let Some(source) = take_suppressed_codex_app_open_source(now) {
            log_session_sync_event(
                "codex_app_watcher_suppressed_open_matched",
                json!({
                    "action": "skip_on_open_handler",
                    "source": source,
                    "signature": codex_open_signature_log_value(&signature),
                    "executables": executable_keys.clone(),
                    "processes": codex_processes_log_value(&processes)
                }),
            );
            open_signature = Some(signature.clone());
            reset_candidate(&mut candidate_signature, &mut candidate_since);
            sleep_interval();
            continue;
        }

        open_signature = Some(signature.clone());
        log_session_sync_event(
            "codex_app_watcher_on_open_invoke",
            json!({
                "signature": codex_open_signature_log_value(&signature),
                "executables": executable_keys.clone(),
                "processes": codex_processes_log_value(&processes)
            }),
        );
        match on_open(&processes) {
            Ok(outcome) if outcome.relaunch_expected => {
                log_session_sync_event(
                    "codex_app_watcher_on_open_finish",
                    json!({
                        "relaunchExpected": true,
                        "executables": executable_keys.clone()
                    }),
                );
                pending_relaunch_executables = executable_keys;
                pending_relaunch_until =
                    Some(Instant::now() + StdDuration::from_millis(PENDING_RELAUNCH_TTL_MS));
            }
            Ok(_) => {
                log_session_sync_event(
                    "codex_app_watcher_on_open_finish",
                    json!({ "relaunchExpected": false }),
                );
            }
            Err(err) => {
                eprintln!("Codex app 打开后处理失败: {err}");
                log_session_sync_event("codex_app_watcher_on_open_error", json!({ "error": err }));
            }
        }
        reset_candidate(&mut candidate_signature, &mut candidate_since);

        sleep_interval();
    }
}

fn codex_processes_log_value(processes: &[CodexProcess]) -> Value {
    Value::Array(
        processes
            .iter()
            .map(|process| {
                json!({
                    "pid": process.pid,
                    "parentPid": process.parent_pid,
                    "executablePath": process.executable_path.as_str()
                })
            })
            .collect(),
    )
}

fn codex_open_signature_log_value(signature: &CodexAppOpenSignature) -> Value {
    json!({ "rootPids": signature.root_pids.clone() })
}

fn reset_candidate(
    candidate_signature: &mut Option<CodexAppOpenSignature>,
    candidate_since: &mut Option<Instant>,
) {
    *candidate_signature = None;
    *candidate_since = None;
}

fn sleep_interval() {
    thread::sleep(StdDuration::from_millis(WATCHER_INTERVAL_MS));
}

fn until_expired(until: Option<Instant>, now: Instant) -> bool {
    until.is_some_and(|deadline| now >= deadline)
}

fn open_absence_elapsed(absence_since: &mut Option<Instant>, now: Instant) -> bool {
    let started = *absence_since.get_or_insert(now);
    now.duration_since(started) >= StdDuration::from_millis(OPEN_ABSENCE_RESET_MS)
}

fn take_expected_codex_app_open_source_if_matches(
    executable_keys: &[String],
    now: Instant,
) -> Option<String> {
    let Ok(mut expected) = expected_codex_app_open_state().lock() else {
        return None;
    };
    if until_expired(expected.until, now) {
        expected.executables.clear();
        expected.source.clear();
        expected.until = None;
        return None;
    }
    if expected.executables != executable_keys {
        return None;
    }
    let source = expected.source.clone();
    expected.executables.clear();
    expected.source.clear();
    expected.until = None;
    Some(source)
}

fn take_suppressed_codex_app_open_source(now: Instant) -> Option<String> {
    let Ok(mut suppressed) = suppressed_codex_app_open_state().lock() else {
        return None;
    };
    if until_expired(suppressed.until, now) {
        suppressed.count = 0;
        suppressed.source.clear();
        suppressed.until = None;
        return None;
    }
    if suppressed.count == 0 {
        return None;
    }
    suppressed.count -= 1;
    let source = suppressed.source.clone();
    if suppressed.count == 0 {
        suppressed.source.clear();
        suppressed.until = None;
    }
    Some(source)
}

fn normalize_executable_key(path: &str) -> String {
    path.trim().to_ascii_lowercase().replace('/', "\\")
}

fn executable_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string()
}

fn update_current_codex_app_processes(processes: Vec<CodexProcess>, error: Option<String>) {
    if let Ok(mut snapshot) = current_codex_app_processes_state().lock() {
        snapshot.captured_at = now_string();
        snapshot.processes = processes;
        snapshot.error = error;
    }
}

fn normalize_executable_keys<'a>(paths: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut executables = paths
        .map(normalize_executable_key)
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    executables.sort();
    executables.dedup();
    executables
}

fn codex_executable_keys(processes: &[CodexProcess]) -> Vec<String> {
    normalize_executable_keys(
        processes
            .iter()
            .map(|process| process.executable_path.as_str()),
    )
}

fn codex_open_signature(processes: &[CodexProcess]) -> CodexAppOpenSignature {
    CodexAppOpenSignature {
        root_pids: codex_root_pids(processes),
    }
}

fn codex_pids(processes: &[CodexProcess]) -> Vec<u64> {
    processes.iter().map(|process| process.pid).collect()
}

fn codex_root_pids(processes: &[CodexProcess]) -> Vec<u64> {
    let all_pids = processes
        .iter()
        .map(|process| process.pid)
        .collect::<HashSet<_>>();
    let mut root_pids = processes
        .iter()
        .filter(|process| process.parent_pid == 0 || !all_pids.contains(&process.parent_pid))
        .map(|process| process.pid)
        .collect::<Vec<_>>();
    root_pids.sort_unstable();
    root_pids.dedup();
    root_pids
}

fn running_codex_processes() -> Result<Vec<CodexProcess>, String> {
    let refresh_kind = ProcessRefreshKind::nothing()
        .with_cmd(UpdateKind::Always)
        .with_exe(UpdateKind::Always)
        .without_tasks();
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh_kind);

    let mut processes = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let executable_path = process
                .exe()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default();
            if executable_path.trim().is_empty() {
                return None;
            }
            let name = process.name().to_string_lossy().to_string();
            let (kind, _) = detect_ide_app(&name, &executable_path)?;
            if kind != "codex" {
                return None;
            }
            Some(CodexProcess {
                pid: u64::from(pid.as_u32()),
                parent_pid: process
                    .parent()
                    .map(|pid| u64::from(pid.as_u32()))
                    .unwrap_or(0),
                executable_path,
                command_line: process_command_line(process),
            })
        })
        .collect::<Vec<_>>();
    processes.sort_by_key(|process| process.pid);
    processes.dedup_by_key(|process| process.pid);
    Ok(processes)
}

fn process_command_line(process: &sysinfo::Process) -> String {
    process
        .cmd()
        .iter()
        .map(|item| item.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_executable_keys_normalize_and_deduplicate_paths() {
        let processes = vec![
            CodexProcess {
                pid: 1,
                parent_pid: 0,
                executable_path: r"C:\Codex\codex.exe".to_string(),
                command_line: String::new(),
            },
            CodexProcess {
                pid: 2,
                parent_pid: 1,
                executable_path: "c:/codex/codex.exe".to_string(),
                command_line: String::new(),
            },
        ];

        assert_eq!(
            codex_executable_keys(&processes),
            vec![r"c:\codex\codex.exe"]
        );
    }

    #[test]
    fn codex_root_pids_returns_processes_without_codex_parent() {
        let processes = vec![
            CodexProcess {
                pid: 10,
                parent_pid: 1,
                executable_path: r"C:\Codex\codex.exe".to_string(),
                command_line: String::new(),
            },
            CodexProcess {
                pid: 11,
                parent_pid: 10,
                executable_path: r"C:\Codex\codex.exe".to_string(),
                command_line: String::new(),
            },
            CodexProcess {
                pid: 12,
                parent_pid: 10,
                executable_path: r"C:\Codex\codex.exe".to_string(),
                command_line: String::new(),
            },
        ];

        assert_eq!(codex_root_pids(&processes), vec![10]);
    }

    #[test]
    fn codex_open_signature_only_tracks_root_pids() {
        let first = vec![
            CodexProcess {
                pid: 10,
                parent_pid: 1,
                executable_path: r"C:\Codex\codex.exe".to_string(),
                command_line: String::new(),
            },
            CodexProcess {
                pid: 11,
                parent_pid: 10,
                executable_path: r"C:\Codex\codex.exe".to_string(),
                command_line: String::new(),
            },
        ];
        let restarted = vec![
            CodexProcess {
                pid: 20,
                parent_pid: 1,
                executable_path: r"C:\Codex\codex.exe".to_string(),
                command_line: String::new(),
            },
            CodexProcess {
                pid: 21,
                parent_pid: 20,
                executable_path: r"C:\Codex\codex.exe".to_string(),
                command_line: String::new(),
            },
        ];
        let moved = vec![
            CodexProcess {
                pid: 10,
                parent_pid: 1,
                executable_path: r"D:\Codex\codex.exe".to_string(),
                command_line: String::new(),
            },
            CodexProcess {
                pid: 11,
                parent_pid: 10,
                executable_path: r"D:\Codex\codex.exe".to_string(),
                command_line: String::new(),
            },
        ];

        assert_eq!(
            codex_executable_keys(&first),
            codex_executable_keys(&restarted)
        );
        assert_ne!(
            codex_open_signature(&first),
            codex_open_signature(&restarted)
        );
        assert_eq!(codex_open_signature(&first), codex_open_signature(&moved));
    }

    #[test]
    fn app_command_expected_open_skips_handler_once() {
        let executables = vec![r"C:\Codex\AppCommand\codex.exe".to_string()];
        let executable_keys = normalize_executable_keys(executables.iter().map(String::as_str));

        expect_app_command_codex_app_open_for_executables(&executables);

        assert_eq!(
            take_expected_codex_app_open_source_if_matches(&executable_keys, Instant::now()),
            Some("app_command".to_string())
        );
        assert_eq!(
            take_expected_codex_app_open_source_if_matches(&executable_keys, Instant::now()),
            None
        );
    }
}
