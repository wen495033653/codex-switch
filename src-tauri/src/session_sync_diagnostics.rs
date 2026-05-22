use crate::time_util::now_string;
use serde_json::{json, Map, Value};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex, OnceLock,
};
use tauri::{AppHandle, Emitter};

const DEV_LOG_EVENT: &str = "dev-log";
const MAX_DEV_LOG_BUFFER: usize = 300;

static DEV_LOG_APP: OnceLock<AppHandle> = OnceLock::new();
static DEV_LOG_BUFFER: OnceLock<Mutex<Vec<Value>>> = OnceLock::new();
static DEV_LOG_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn dev_log_buffer() -> &'static Mutex<Vec<Value>> {
    DEV_LOG_BUFFER.get_or_init(|| Mutex::new(Vec::new()))
}

fn event_level(event: &str) -> &'static str {
    if event.ends_with("_error") || event == "session_sync_error" {
        "error"
    } else if event.ends_with("_skip") {
        "warn"
    } else {
        "debug"
    }
}

fn pick_labeled_details(details: &Value, keys: &[(&str, &str)]) -> Value {
    let mut summary = Map::new();
    for (key, label) in keys {
        if let Some(value) = details.get(*key) {
            summary.insert((*label).to_string(), value.clone());
        }
    }
    Value::Object(summary)
}

fn phase_label(details: &Value) -> Value {
    let phase = details.get("phase").and_then(Value::as_str).unwrap_or("");
    Value::String(match phase {
        "before" => "同步前".to_string(),
        "after" => "同步后".to_string(),
        _ => phase.to_string(),
    })
}

fn session_sync_state_db_summary_details(details: &Value) -> Value {
    let mut summary = Map::new();
    for (key, label) in [
        ("trigger", "触发来源"),
        ("targetProvider", "目标 provider"),
        ("stateDb", "state DB"),
        ("updated", "本阶段更新数量"),
    ] {
        if let Some(value) = details.get(key) {
            summary.insert(label.to_string(), value.clone());
        }
    }
    summary.insert("阶段".to_string(), phase_label(details));
    if let Some(value) = details.get("summary") {
        summary.insert("provider 分布和待改明细".to_string(), value.clone());
    }
    Value::Object(summary)
}

fn dev_log_source(event: &str) -> &'static str {
    if event.starts_with("session_sync_") {
        "会话同步"
    } else if event.starts_with("codex_app_") || event == "app_start" {
        "Codex App"
    } else if event.starts_with("ide_reopen_") {
        "IDE 重开"
    } else {
        "调试"
    }
}

fn dev_log_message(event: &str) -> &'static str {
    match event {
        "app_start" => "应用启动",
        "codex_app_open_handler_start" => "Codex App 打开处理开始",
        "codex_app_open_handler_skip" => "跳过 Codex App 打开处理",
        "codex_app_open_handler_session_sync_skip" => "跳过打开前会话同步",
        "codex_app_open_handler_finish" => "Codex App 打开处理完成",
        "codex_app_restart_command_start" => "Codex App 重启命令开始",
        "codex_app_restart_command_skip" => "跳过 Codex App 重启命令",
        "codex_app_restart_command_session_sync_skip" => "跳过重启前会话同步",
        "codex_app_restart_command_session_sync_error" => "重启前会话同步失败",
        "codex_app_restart_command_finish" => "Codex App 重启命令完成",
        "codex_app_relaunch_executable_skip" => "跳过 Codex App 可执行文件重启",
        "codex_app_relaunch_executable_expect_open" => "等待 Codex App 重新打开",
        "codex_app_relaunch_executable_finish" => "Codex App 可执行文件重启完成",
        "codex_app_relaunch_executable_error" => "Codex App 可执行文件重启失败",
        "codex_app_relaunch_processes_start" => "开始重启 Codex App 进程",
        "codex_app_relaunch_processes_finish" => "重启 Codex App 进程完成",
        "codex_app_relaunch_processes_error" => "重启 Codex App 进程失败",
        "session_sync_current_mode_resolve_start" => "开始解析当前 Codex provider",
        "session_sync_current_mode_resolved" => "当前 Codex provider 已解析",
        "session_sync_start" => "开始同步会话 provider",
        "session_sync_finish" => "会话 provider 同步完成",
        "session_sync_error" => "会话 provider 同步失败",
        "session_sync_state_db_missing" => "Codex state DB 不存在，跳过",
        "session_sync_state_db_summary" => "Codex state DB provider 同步明细",
        "session_sync_rollout_selection" => "已选择待检查的 rollout 文件",
        "session_sync_rollout_file_processed" => "rollout 文件 provider 处理完成",
        "session_sync_rollout_file_error" => "rollout 文件 provider 处理失败",
        "session_sync_rollout_batch_finish" => "rollout 文件 provider 批处理完成",
        "ide_reopen_confirm_start" => "IDE 重开确认开始",
        "ide_reopen_session_sync_skip" => "跳过 IDE 重开前会话同步",
        "ide_reopen_confirm_finish" => "IDE 重开确认完成",
        "codex_app_watcher_scan_error" => "Codex App Watcher 扫描失败",
        "codex_app_watcher_on_open_error" => "Codex App Watcher 打开处理失败",
        _ => "未知调试事件",
    }
}

fn dev_log_details(event: &str, details: &Value) -> Option<Value> {
    match event {
        "app_start" => Some(json!({})),
        "codex_app_open_handler_start" => Some(pick_labeled_details(
            details,
            &[
                ("pluginUnlockEnabled", "Plugin 解锁已启用"),
                ("sessionSyncEnabled", "会话同步已启用"),
            ],
        )),
        "codex_app_open_handler_skip" | "codex_app_open_handler_session_sync_skip" => {
            Some(pick_labeled_details(details, &[("reason", "原因")]))
        }
        "codex_app_open_handler_finish" => Some(pick_labeled_details(
            details,
            &[("relaunchExpected", "预计重启")],
        )),
        "codex_app_restart_command_start" => {
            Some(pick_labeled_details(details, &[("command", "命令")]))
        }
        "codex_app_restart_command_skip" | "codex_app_restart_command_session_sync_skip" => Some(
            pick_labeled_details(details, &[("command", "命令"), ("reason", "原因")]),
        ),
        "codex_app_restart_command_session_sync_error" => Some(pick_labeled_details(
            details,
            &[("command", "命令"), ("error", "错误")],
        )),
        "codex_app_restart_command_finish" => Some(pick_labeled_details(
            details,
            &[
                ("command", "命令"),
                ("restarted", "已重启"),
                ("restartedCount", "重启数量"),
            ],
        )),
        "codex_app_relaunch_executable_skip" => {
            Some(pick_labeled_details(details, &[("reason", "原因")]))
        }
        "codex_app_relaunch_executable_expect_open" => {
            Some(pick_labeled_details(details, &[("mode", "模式")]))
        }
        "codex_app_relaunch_executable_finish" => Some(pick_labeled_details(
            details,
            &[("mode", "模式"), ("restarted", "已重启")],
        )),
        "codex_app_relaunch_executable_error" => Some(pick_labeled_details(
            details,
            &[("mode", "模式"), ("error", "错误")],
        )),
        "codex_app_relaunch_processes_start" => Some(pick_labeled_details(
            details,
            &[("origin", "来源"), ("mode", "模式")],
        )),
        "codex_app_relaunch_processes_finish" => Some(pick_labeled_details(
            details,
            &[
                ("origin", "来源"),
                ("mode", "模式"),
                ("restartedCount", "重启数量"),
            ],
        )),
        "codex_app_relaunch_processes_error" => Some(pick_labeled_details(
            details,
            &[("origin", "来源"), ("mode", "模式"), ("error", "错误")],
        )),
        "session_sync_current_mode_resolve_start" => {
            Some(pick_labeled_details(details, &[("trigger", "触发来源")]))
        }
        "session_sync_current_mode_resolved" => Some(pick_labeled_details(
            details,
            &[("trigger", "触发来源"), ("targetProvider", "目标 provider")],
        )),
        "session_sync_start" => Some(pick_labeled_details(
            details,
            &[("trigger", "触发来源"), ("targetProvider", "目标 provider")],
        )),
        "session_sync_finish" => Some(pick_labeled_details(
            details,
            &[
                ("trigger", "触发来源"),
                ("targetProvider", "目标 provider"),
                ("stateDbUpdated", "state DB 更新数量"),
                ("rolloutFilesUpdated", "rollout 文件更新数量"),
                ("updated", "总更新数量"),
            ],
        )),
        "session_sync_error" => Some(pick_labeled_details(
            details,
            &[
                ("trigger", "触发来源"),
                ("targetProvider", "目标 provider"),
                ("stateDbUpdated", "state DB 更新数量"),
                ("rolloutFilesUpdated", "rollout 文件更新数量"),
                ("updated", "总更新数量"),
                ("errors", "错误"),
            ],
        )),
        "session_sync_state_db_missing" => Some(pick_labeled_details(
            details,
            &[
                ("trigger", "触发来源"),
                ("targetProvider", "目标 provider"),
                ("stateDb", "state DB"),
            ],
        )),
        "session_sync_state_db_summary" => Some(session_sync_state_db_summary_details(details)),
        "session_sync_rollout_selection" => Some(pick_labeled_details(
            details,
            &[
                ("trigger", "触发来源"),
                ("targetProvider", "目标 provider"),
                ("scanDirs", "扫描目录"),
                ("recentLimit", "最近文件限制"),
                ("extraRolloutPaths", "额外固定文件"),
                ("selectedCount", "选中文件数"),
                ("selectedFiles", "选中文件"),
            ],
        )),
        "session_sync_rollout_file_processed" => Some(pick_labeled_details(
            details,
            &[
                ("trigger", "触发来源"),
                ("targetProvider", "目标 provider"),
                ("path", "文件"),
                ("changed", "是否改动"),
                ("rewrittenLines", "重写行数"),
                ("providerChange", "provider 变更"),
                ("sessionMetaBefore", "session_meta 同步前"),
                ("sessionMetaAfter", "session_meta 同步后"),
            ],
        )),
        "session_sync_rollout_file_error" => Some(pick_labeled_details(
            details,
            &[
                ("trigger", "触发来源"),
                ("targetProvider", "目标 provider"),
                ("path", "文件"),
                ("error", "错误"),
            ],
        )),
        "session_sync_rollout_batch_finish" => Some(pick_labeled_details(
            details,
            &[
                ("trigger", "触发来源"),
                ("targetProvider", "目标 provider"),
                ("updatedFiles", "更新文件数量"),
                ("providerChanges", "provider 变更"),
            ],
        )),
        "ide_reopen_confirm_start" => Some(pick_labeled_details(
            details,
            &[
                ("apiMode", "API 模式"),
                ("accountIdPresent", "账号 ID 存在"),
                ("sessionSyncProvider", "会话同步 provider"),
            ],
        )),
        "ide_reopen_session_sync_skip" => {
            Some(pick_labeled_details(details, &[("reason", "原因")]))
        }
        "ide_reopen_confirm_finish" => Some(pick_labeled_details(
            details,
            &[("sessionSyncWarning", "会话同步警告")],
        )),
        "codex_app_watcher_scan_error" | "codex_app_watcher_on_open_error" => {
            Some(pick_labeled_details(details, &[("error", "错误")]))
        }
        _ => None,
    }
}

pub(crate) fn init_session_sync_diagnostics(app: AppHandle) {
    let _ = DEV_LOG_APP.set(app);
}

pub(crate) fn log_session_sync_event(event: &str, details: Value) {
    if !cfg!(debug_assertions) {
        return;
    }

    let Some(details) = dev_log_details(event, &details) else {
        return;
    };

    let payload = json!({
        "sequence": DEV_LOG_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        "level": event_level(event),
        "source": dev_log_source(event),
        "message": dev_log_message(event),
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

#[tauri::command]
pub(crate) fn get_dev_log_entries() -> Value {
    if !cfg!(debug_assertions) {
        return Value::Array(Vec::new());
    }

    let entries = dev_log_buffer()
        .lock()
        .map(|buffer| buffer.clone())
        .unwrap_or_default();
    Value::Array(entries)
}
