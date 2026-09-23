//! Application logging. Every backend event goes through `log_event`; where it ends up depends
//! on the event:
//!
//! - `*_error` events are appended to `logs/codex-switch-errors.jsonl`.
//! - Errors and the key events in `TIMELINE_EVENTS` (one per user action or state change) are
//!   appended to `logs/codex-switch-events.jsonl`, so a failure can be read with what led to it.
//! - Debug builds also show errors, timeline events and `DEV_ONLY_EVENTS` (timer-driven) in the
//!   dev log window. Other events are fine-grained steps and are not kept anywhere.
//!
//! Both files rotate once at 5 MB (one older generation is kept). Release builds are windowed
//! apps without a console, so nothing here may rely on stderr.

use crate::time_util::now_string;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};
use tauri::{AppHandle, Emitter};

const DEV_LOG_EVENT: &str = "dev-log";
// Stable code survives the String-based command/watcher error chain.
pub(crate) const PROCESS_ELEVATION_WARNING: &str = "[PROCESS_ELEVATION_MISMATCH]";
const MAX_DEV_LOG_BUFFER: usize = 300;
const LOG_DIR_NAME: &str = "logs";
// A retry loop can log one error every few seconds; one rotated generation bounds each file at
// twice this size.
const LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;

struct LogFile {
    name: &'static str,
    rotated_name: &'static str,
}

const ERROR_LOG: LogFile = LogFile {
    name: "codex-switch-errors.jsonl",
    rotated_name: "codex-switch-errors.1.jsonl",
};
const EVENT_LOG: LogFile = LogFile {
    name: "codex-switch-events.jsonl",
    rotated_name: "codex-switch-events.1.jsonl",
};

/// Key events recorded on disk. Each happens once per user action or state change, never on a
/// timer, so the file reads as a timeline.
const TIMELINE_EVENTS: &[&str] = &[
    "app_start",
    "codex_desktop_support_status",
    "codex_app_restart_command_finish",
    "codex_app_relaunch_processes_start",
    "codex_app_relaunch_processes_finish",
    "codex_app_relaunch_executable_finish",
    "codex_app_process_kill_finish",
    "codex_app_process_kill_tree_exited",
    "codex_app_launch_confirmation_finish",
    "codex_app_multi_open_finish",
    "codex_remote_control_runtime_updated",
    "codex_remote_control_runtime_applied",
    "codex_remote_control_auto_disabled",
    "session_sync_finish",
    "session_sync_state_db_summary",
    "ide_reopen_confirm_finish",
    "ide_reopen_discard_without_config_apply",
    "ide_reopen_snapshot_process_gone",
    "account_subscription_updated",
];

/// Shown in the dev log of debug builds only: these run on the watcher's 60 s reconcile and
/// would bury the timeline.
const DEV_ONLY_EVENTS: &[&str] = &[
    "codex_app_open_handler_finish",
    "session_sync_preflight_finish",
];

/// The dev log's one-line description; events not listed fall back to a generic text.
const EVENT_MESSAGES: &[(&str, &str)] = &[
    ("app_start", "应用启动"),
    ("codex_desktop_support_status", "Codex Desktop 兼容状态"),
    ("codex_app_restart_command_finish", "“重启 Codex”完成"),
    ("codex_app_relaunch_processes_start", "开始结束并重开 Codex"),
    (
        "codex_app_relaunch_processes_finish",
        "结束并重开 Codex 完成",
    ),
    ("codex_app_relaunch_executable_finish", "Codex 已重新打开"),
    ("codex_app_process_kill_finish", "结束进程树完成"),
    (
        "codex_app_process_kill_tree_exited",
        "进程树已退出（结束命令本身报错）",
    ),
    ("codex_app_launch_confirmation_finish", "启动确认完成"),
    ("codex_app_multi_open_finish", "多开窗口已打开"),
    ("codex_app_open_handler_finish", "Codex 打开处理完成"),
    (
        "codex_remote_control_runtime_updated",
        "远程控制运行时已更新",
    ),
    (
        "codex_remote_control_runtime_applied",
        "远程控制运行时已应用",
    ),
    ("codex_remote_control_auto_disabled", "远程控制已自动关闭"),
    ("session_sync_finish", "会话 provider 同步完成"),
    ("session_sync_preflight_finish", "会话 provider 预检查完成"),
    (
        "session_sync_state_db_summary",
        "Codex state DB provider 同步摘要",
    ),
    ("ide_reopen_confirm_finish", "编辑器重开完成"),
    ("ide_reopen_discard_without_config_apply", "忽略编辑器重开"),
    (
        "ide_reopen_snapshot_process_gone",
        "快照中的编辑器进程已不在，跳过结束",
    ),
    ("account_subscription_updated", "订阅信息已更新"),
    ("session_sync_error", "会话 provider 同步失败"),
    ("session_sync_preflight_error", "会话 provider 预检查失败"),
    (
        "session_sync_rollout_file_error",
        "rollout 文件 provider 处理失败",
    ),
    ("codex_app_process_kill_error", "结束进程树失败"),
    ("codex_app_launch_confirmation_error", "启动确认失败"),
    (
        "codex_app_relaunch_processes_close_error",
        "Codex 关闭失败，已停止重启流程",
    ),
    (
        "codex_app_relaunch_processes_error",
        "结束并重开 Codex 失败",
    ),
    ("codex_app_relaunch_executable_error", "Codex 重新打开失败"),
    ("codex_app_watcher_scan_error", "Codex watcher 扫描失败"),
    (
        "codex_app_watcher_on_open_error",
        "Codex watcher 打开处理失败",
    ),
    (
        "codex_app_watcher_on_open_panic_error",
        "Codex watcher 打开处理异常",
    ),
    ("codex_app_watcher_panic_error", "Codex watcher 异常退出"),
    ("codex_remote_control_helper_error", "远程控制 helper 失败"),
    (
        "codex_desktop_data_migration_error",
        "Codex Desktop 数据迁移失败",
    ),
    (
        "codex_app_instance_data_migration_error",
        "Codex 多开数据迁移失败",
    ),
];

static LOG_WRITE_LOCK: Mutex<()> = Mutex::new(());
static DEV_LOG_APP: OnceLock<AppHandle> = OnceLock::new();
static DEV_LOG_BUFFER: OnceLock<Mutex<Vec<Value>>> = OnceLock::new();
static DEV_LOG_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn dev_log_buffer() -> &'static Mutex<Vec<Value>> {
    DEV_LOG_BUFFER.get_or_init(|| Mutex::new(Vec::new()))
}

fn is_error_event(event: &str) -> bool {
    event.ends_with("_error")
}

fn is_timeline_event(event: &str) -> bool {
    TIMELINE_EVENTS.contains(&event)
}

fn event_level(event: &str, details: &Value) -> &'static str {
    if details
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|error| error.contains(PROCESS_ELEVATION_WARNING))
    {
        "warn"
    } else if is_error_event(event) {
        "error"
    } else if event.ends_with("_skip") {
        "warn"
    } else if is_timeline_event(event) {
        "info"
    } else {
        "debug"
    }
}

fn event_message(event: &str) -> &'static str {
    EVENT_MESSAGES
        .iter()
        .find(|(name, _)| *name == event)
        .map(|(_, message)| *message)
        .unwrap_or(if is_error_event(event) {
            "错误（详见事件名与详情）"
        } else {
            "事件（详见事件名与详情）"
        })
}

fn dev_log_source(event: &str) -> &'static str {
    if event.starts_with("session_sync_") {
        "会话同步"
    } else if event.starts_with("codex_app_")
        || event.starts_with("codex_desktop_")
        || event.starts_with("codex_remote_control_")
        || event.starts_with("codex_state_")
    {
        "Codex App"
    } else if event.starts_with("ide_reopen_") {
        "IDE 重开"
    } else if event.starts_with("account_") || event.starts_with("active_account_") {
        "账号"
    } else if event.starts_with("session_manager_") {
        "会话管理"
    } else if event.starts_with("usage_stats_") || event.starts_with("codex_session_usage_") {
        "用量统计"
    } else if event.starts_with("oauth_") {
        "登录"
    } else if event.starts_with("background_refresh_") || event.starts_with("refresh_all_") {
        "定时刷新"
    } else if event.starts_with("app_") || event.starts_with("window_state_") {
        "应用"
    } else {
        "调试"
    }
}

/// Logs identify an account by the first 8 characters of its profile id (the ChatGPT account
/// id prefix): never the full profile id, which ends with the email, nor tokens.
pub(crate) fn account_label(profile_id: &str) -> String {
    profile_id.chars().take(8).collect()
}

/// Long evidence (an HTML error page, a command's full output) is kept to its start, which is
/// enough to tell what answered, plus its full length.
pub(crate) fn truncate_for_log(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((index, _)) => format!("{}…（共 {} 字符）", &text[..index], text.chars().count()),
        None => text.to_string(),
    }
}

#[cfg(not(test))]
fn log_path(file: &LogFile) -> Result<PathBuf, String> {
    Ok(crate::paths::app_data_dir()?
        .join(LOG_DIR_NAME)
        .join(file.name))
}

fn append_log(path: &Path, rotated_name: &str, event: &str, details: &Value) -> Result<(), String> {
    let _guard = LOG_WRITE_LOCK
        .lock()
        .map_err(|_| "日志写入锁异常".to_string())?;
    let dir = path
        .parent()
        .ok_or_else(|| format!("日志路径无父目录: {}", path.display()))?;
    fs::create_dir_all(dir).map_err(|err| format!("创建日志目录失败 {}: {err}", dir.display()))?;
    if fs::metadata(path).is_ok_and(|metadata| metadata.len() >= LOG_MAX_BYTES) {
        let rotated = dir.join(rotated_name);
        fs::rename(path, &rotated).map_err(|err| {
            format!(
                "轮转日志失败 {} -> {}: {err}",
                path.display(),
                rotated.display()
            )
        })?;
    }
    let mut line = json!({
        "timestamp": now_string(),
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "event": event,
        "level": event_level(event, details),
        "details": details
    })
    .to_string();
    line.push('\n');
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(line.as_bytes()))
        .map_err(|err| format!("写入日志失败 {}: {err}", path.display()))
}

// Unit tests exercise error paths; they must not write into the user's real data directory, so
// the files are only written outside tests (the writer itself is tested with explicit paths).
#[cfg(not(test))]
fn persist(file: &LogFile, event: &str, details: &Value) {
    if let Err(err) =
        log_path(file).and_then(|path| append_log(&path, file.rotated_name, event, details))
    {
        // The log itself cannot be written; stderr is the only place left to say so.
        eprintln!("{err}");
    }
}

pub(crate) fn init_app_log(app: AppHandle) {
    let _ = DEV_LOG_APP.set(app);
}

pub(crate) fn log_event(event: &str, details: Value) {
    let error = is_error_event(event);
    let timeline = error || is_timeline_event(event);
    #[cfg(not(test))]
    {
        if error {
            persist(&ERROR_LOG, event, &details);
        }
        if timeline {
            persist(&EVENT_LOG, event, &details);
        }
    }

    let shown = if cfg!(debug_assertions) {
        timeline || DEV_ONLY_EVENTS.contains(&event)
    } else {
        error
    };
    if !shown {
        return;
    }
    let payload = json!({
        "sequence": DEV_LOG_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        "level": event_level(event, &details),
        "source": dev_log_source(event),
        "message": event_message(event),
        "details": {
            "timestamp": now_string(),
            "event": event,
            "details": details
        }
    });

    if let Ok(mut buffer) = dev_log_buffer().lock() {
        buffer.push(payload.clone());
        if buffer.len() > MAX_DEV_LOG_BUFFER {
            let overflow = buffer.len() - MAX_DEV_LOG_BUFFER;
            buffer.drain(0..overflow);
        }
    }

    if let Some(app) = DEV_LOG_APP.get() {
        let _ = app.emit(DEV_LOG_EVENT, payload);
    }
}

/// For paths that run every few seconds or minutes: the same failure would be appended on every
/// round, so each distinct event and details pair is logged once per run.
pub(crate) fn log_event_once(event: &str, details: Value) {
    static LOGGED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let first = LOGGED
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map(|mut logged| logged.insert(format!("{event}\n{details}")))
        .unwrap_or(true);
    if first {
        log_event(event, details);
    }
}

#[tauri::command]
pub(crate) fn get_dev_log_entries() -> Value {
    let entries = dev_log_buffer()
        .lock()
        .map(|buffer| buffer.clone())
        .unwrap_or_default();
    Value::Array(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn dev_log_entry(event: &str) -> Option<Value> {
        get_dev_log_entries()
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|entry| entry["details"]["event"] == event)
            .cloned()
    }

    #[test]
    fn error_events_reach_the_dev_log_with_their_full_details() {
        let details = json!({ "account": "fixture1", "error": "fixture: HTTP 403", "status": 403 });
        log_event("account_fixture_dev_log_error", details.clone());

        let entry = dev_log_entry("account_fixture_dev_log_error").expect("error event is shown");
        assert_eq!(entry["level"], "error");
        assert_eq!(entry["source"], "账号");
        assert_eq!(entry["message"], "错误（详见事件名与详情）");
        assert_eq!(entry["details"]["details"], details);
    }

    #[test]
    fn timeline_and_dev_only_events_are_shown_but_other_steps_are_not() {
        log_event(
            "ide_reopen_snapshot_process_gone",
            json!({ "skippedPids": [1] }),
        );
        log_event(
            "codex_app_open_handler_finish",
            json!({ "relaunchExpected": false }),
        );
        log_event(
            "codex_app_watcher_expect_open_set",
            json!({ "source": "fixture" }),
        );

        let timeline = dev_log_entry("ide_reopen_snapshot_process_gone").unwrap();
        assert_eq!(timeline["level"], "info");
        assert_eq!(timeline["message"], "快照中的编辑器进程已不在，跳过结束");
        assert_eq!(timeline["details"]["details"]["skippedPids"], json!([1]));
        assert_eq!(
            dev_log_entry("codex_app_open_handler_finish").unwrap()["level"],
            "debug"
        );
        assert!(dev_log_entry("codex_app_watcher_expect_open_set").is_none());
    }

    #[test]
    fn timeline_holds_no_timer_driven_event() {
        for event in DEV_ONLY_EVENTS {
            assert!(!is_timeline_event(event), "{event}");
        }
        assert!(!is_timeline_event("codex_desktop_data_migration"));
        assert!(is_error_event("session_sync_error"));
    }

    #[test]
    fn account_label_and_truncation_keep_logs_short_and_free_of_emails() {
        assert_eq!(account_label("6eb799f4-745a:user@example.com"), "6eb799f4");
        assert_eq!(truncate_for_log("short", 300), "short");
        let page = "<html>".repeat(100);
        let cut = truncate_for_log(&page, 300);
        assert!(cut.starts_with("<html><html>"));
        assert!(cut.ends_with("…（共 600 字符）"), "{cut}");
    }

    #[test]
    fn permission_warning_stays_warn_through_disk_and_watcher_error_chain() {
        let path = unique_temp_log_path("permission-warning");
        let details = json!({"error": format!("{PROCESS_ELEVATION_WARNING} callerElevated=false, targetElevated=true"),
            "terminationAttempted": false, "retry": false});
        for event in [
            "codex_app_process_kill_error",
            "codex_app_watcher_on_open_error",
        ] {
            assert_eq!(event_level(event, &details), "warn");
            append_log(&path, ERROR_LOG.rotated_name, event, &details).unwrap();
        }
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        for line in text.lines() {
            let entry: Value = serde_json::from_str(line).unwrap();
            assert_eq!(entry["level"], "warn");
            assert_eq!(entry["details"], details);
        }
        assert_eq!(
            event_level(
                "codex_app_process_kill_error",
                &json!({"error": "access denied"})
            ),
            "error"
        );
    }

    #[test]
    fn polled_failure_is_logged_once_per_distinct_details() {
        let count = |root: &str| {
            get_dev_log_entries()
                .as_array()
                .unwrap()
                .iter()
                .filter(|entry| entry["details"]["details"]["codexHome"] == root)
                .count()
        };
        let event = "codex_app_instance_data_migration_error";
        for _ in 0..3 {
            log_event_once(event, json!({"codexHome": "fixture-once-a", "error": "x"}));
        }
        log_event_once(event, json!({"codexHome": "fixture-once-b", "error": "x"}));
        assert_eq!(count("fixture-once-a"), 1);
        assert_eq!(count("fixture-once-b"), 1);
    }

    fn unique_temp_log_path(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir()
            .join(format!("codex-switch-{name}-{stamp}"))
            .join(LOG_DIR_NAME)
            .join(ERROR_LOG.name)
    }

    #[test]
    fn close_failure_sync_outcome_is_persisted_without_hiding_sync_errors() {
        let path = unique_temp_log_path("close-failure-sync");
        let close_error = format!("{PROCESS_ELEVATION_WARNING} fixture permission mismatch");
        for (error, sync_error, level) in [
            (
                format!("{close_error}；已直接同步会话文件"),
                Value::Null,
                "warn",
            ),
            (
                "直接同步会话文件失败：database is locked (code 5)".into(),
                json!("database is locked (code 5)"),
                "error",
            ),
        ] {
            let details = json!({"pids": [42], "trigger": "codex_app_close_failed_watcher",
                "closeError": close_error, "error": error, "sessionSyncError": sync_error,
                "sessionSyncAttempted": true, "sessionSyncSucceeded": sync_error.is_null(),
                "retry": false, "restarted": false});
            append_log(
                &path,
                ERROR_LOG.rotated_name,
                "codex_app_relaunch_processes_close_error",
                &details,
            )
            .unwrap();
            let content = fs::read_to_string(&path).unwrap();
            let saved: Value = serde_json::from_str(content.lines().last().unwrap()).unwrap();
            assert_eq!(saved["level"], level);
            assert_eq!(saved["details"], details);
        }
        println!("close failure sync log fixture: {}", path.display());
    }

    #[test]
    fn log_appends_one_json_line_per_event_with_full_details() {
        let path = unique_temp_log_path("error-log");
        append_log(
            &path,
            ERROR_LOG.rotated_name,
            "codex_app_process_kill_error",
            &json!({"pid": 42, "error": "taskkill /F /T /PID 42: exit code 128"}),
        )
        .unwrap();
        append_log(
            &path,
            ERROR_LOG.rotated_name,
            "session_sync_finish",
            &json!({"updated": 3}),
        )
        .unwrap();

        let text = fs::read_to_string(&path).unwrap();
        fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
        let lines = text.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["event"], "codex_app_process_kill_error");
        assert_eq!(first["level"], "error");
        assert_eq!(first["details"]["pid"], 42);
        assert_eq!(
            first["details"]["error"],
            "taskkill /F /T /PID 42: exit code 128"
        );
        assert_eq!(first["version"], env!("CARGO_PKG_VERSION"));
        assert!(!first["timestamp"].as_str().unwrap().is_empty());
        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["event"], "session_sync_finish");
        assert_eq!(second["level"], "info");
    }

    #[test]
    fn watcher_failure_log_persists_no_retry_and_process_context() {
        let path = unique_temp_log_path("watcher-no-retry");
        let details = json!({
            "error": "taskkill /F /T /PID 42: exitCode=128; access denied",
            "retry": false,
            "disabledUntil": "codex_switch_restart",
            "processes": [{"executablePath": "Codex/ChatGPT.exe", "processes": [{"pid": 42, "parentPid": 1, "startedAt": 100}]}]
        });
        append_log(
            &path,
            ERROR_LOG.rotated_name,
            "codex_app_watcher_on_open_error",
            &details,
        )
        .unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 1);
        let entry: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(entry["event"], "codex_app_watcher_on_open_error");
        assert_eq!(entry["details"], details);
        assert_eq!(entry["pid"], std::process::id());
        assert!(!entry["timestamp"].as_str().unwrap().is_empty());
        println!("watcher failure log verified: {}", path.display());
    }

    #[test]
    fn log_rotates_once_the_size_limit_is_reached() {
        let path = unique_temp_log_path("error-log-rotate");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![b'x'; LOG_MAX_BYTES as usize]).unwrap();

        append_log(
            &path,
            EVENT_LOG.rotated_name,
            "session_sync_error",
            &json!({"error": "after"}),
        )
        .unwrap();

        let rotated = path.parent().unwrap().join(EVENT_LOG.rotated_name);
        let rotated_len = fs::metadata(&rotated).unwrap().len();
        let current = fs::read_to_string(&path).unwrap();
        fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
        assert_eq!(rotated_len, LOG_MAX_BYTES);
        assert_eq!(current.lines().count(), 1);
        assert!(current.contains("\"after\""));
    }
}
