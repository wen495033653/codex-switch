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

fn session_sync_state_db_summary_details(details: &Value) -> Value {
    let mut summary = Map::new();
    for (key, label) in [
        ("trigger", "触发来源"),
        ("targetProvider", "目标 provider"),
        ("updated", "更新数量"),
    ] {
        if let Some(value) = details.get(key) {
            summary.insert(label.to_string(), value.clone());
        }
    }
    if let Some(value) = details.get("summary") {
        summary.insert("provider 变更摘要".to_string(), value.clone());
    }
    Value::Object(summary)
}

fn dev_log_source(event: &str) -> &'static str {
    if event.starts_with("session_sync_") {
        "会话同步"
    } else if event.starts_with("codex_app_")
        || event.starts_with("codex_remote_control_")
        || event == "app_start"
    {
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
        "codex_remote_control_runtime_applied" => "远程控制运行时已应用",
        "codex_remote_control_runtime_updated" => "远程控制运行时已更新",
        "codex_remote_control_subscription_home_prepared" => "远程控制订阅 home 已准备",
        "codex_remote_control_history_synced" => "远程控制历史已同步",
        "codex_remote_control_subscription_home_removed" => "远程控制订阅 home 已删除",
        "codex_remote_control_helper_spawn" => "远程控制 helper 已启动",
        "codex_remote_control_helper_keep_running" => "远程控制 helper 已在运行",
        "codex_remote_control_helper_stale_stop" => "远程控制旧 helper 已清理",
        "codex_remote_control_helper_stop" => "远程控制 helper 已停止",
        "codex_remote_control_helper_error" => "远程控制 helper 失败",
        "codex_app_open_handler_start" => "Codex App 打开处理开始",
        "codex_app_open_handler_skip" => "跳过 Codex App 打开处理",
        "codex_app_open_handler_remote_control_runtime_ready" => "远程控制运行时已准备",
        "codex_app_open_handler_remote_control_runtime_skip" => "跳过打开前远程控制运行态同步",
        "codex_app_open_handler_remote_control_runtime_deferred" => {
            "远程控制运行态延后到 Codex App 关闭后"
        }
        "codex_app_open_handler_remote_control_runtime_error" => "远程控制运行时准备失败",
        "codex_app_open_handler_session_sync_skip" => "跳过打开前会话同步",
        "codex_app_open_handler_session_sync_deferred" => "会话同步延后到 Codex App 关闭后",
        "codex_app_open_handler_session_sync_preflight_error" => "打开前会话同步预检查失败",
        "codex_app_open_handler_finish" => "Codex App 打开处理完成",
        "codex_app_restart_command_start" => "Codex App 重启命令开始",
        "codex_app_restart_command_skip" => "跳过 Codex App 重启命令",
        "codex_app_restart_command_session_sync_skip" => "跳过重启前会话同步",
        "codex_app_restart_command_session_sync_deferred" => "会话同步延后到 Codex App 关闭后",
        "codex_app_restart_command_session_sync_error" => "重启前会话同步失败",
        "codex_app_restart_command_remote_control_runtime_skip" => "跳过重启前远程控制运行态同步",
        "codex_app_restart_command_remote_control_runtime_deferred" => {
            "远程控制运行态延后到 Codex App 关闭后"
        }
        "codex_app_restart_command_remote_control_runtime_error" => {
            "重启前远程控制运行态预检查失败"
        }
        "codex_app_restart_command_finish" => "Codex App 重启命令完成",
        "codex_app_relaunch_executable_skip" => "跳过 Codex App 可执行文件重启",
        "codex_app_relaunch_executable_expect_open" => "等待 Codex App 重新打开",
        "codex_app_relaunch_executable_finish" => "Codex App 可执行文件重启完成",
        "codex_app_relaunch_executable_error" => "Codex App 可执行文件重启失败",
        "codex_app_relaunch_processes_start" => "开始重启 Codex App 进程",
        "codex_app_relaunch_processes_post_exit_config_apply_start" => {
            "Codex App 关闭后开始同步运行态配置"
        }
        "codex_app_relaunch_processes_post_exit_config_apply_finish" => {
            "Codex App 关闭后运行态配置同步完成"
        }
        "codex_app_relaunch_processes_post_exit_config_apply_error" => {
            "Codex App 关闭后运行态配置同步失败"
        }
        "codex_app_relaunch_processes_post_exit_remote_control_runtime_error" => {
            "远程控制运行时更新失败"
        }
        "codex_app_relaunch_processes_post_exit_session_sync_start" => {
            "Codex App 关闭后开始同步会话 provider"
        }
        "codex_app_relaunch_processes_post_exit_session_sync_finish" => {
            "Codex App 关闭后会话 provider 同步完成"
        }
        "codex_app_relaunch_processes_post_exit_session_sync_error" => {
            "Codex App 关闭后会话 provider 同步失败"
        }
        "codex_app_relaunch_processes_finish" => "重启 Codex App 进程完成",
        "codex_app_relaunch_processes_error" => "重启 Codex App 进程失败",
        "session_sync_current_mode_resolve_start" => "开始解析当前 Codex provider",
        "session_sync_current_mode_resolved" => "当前 Codex provider 已解析",
        "session_sync_start" => "开始同步会话 provider",
        "session_sync_finish" => "会话 provider 同步完成",
        "session_sync_error" => "会话 provider 同步失败",
        "session_sync_preflight_finish" => "会话 provider 预检查完成",
        "session_sync_preflight_error" => "会话 provider 预检查失败",
        "session_sync_state_db_missing" => "Codex state DB 不存在，跳过",
        "session_sync_state_db_summary" => "Codex state DB provider 同步摘要",
        "session_sync_rollout_selection" => "rollout 文件扫描摘要",
        "session_sync_rollout_file_error" => "rollout 文件 provider 处理失败",
        "session_sync_rollout_batch_finish" => "rollout 文件 provider 批处理完成",
        "ide_reopen_confirm_start" => "IDE 重开确认开始",
        "ide_reopen_session_sync_skip" => "跳过 IDE 重开前会话同步",
        "ide_reopen_confirm_finish" => "IDE 重开确认完成",
        "ide_reopen_discard_without_config_apply" => "忽略 IDE 重开",
        "codex_app_watcher_scan_error" => "Codex App Watcher 扫描失败",
        "codex_app_watcher_on_open_error" => "Codex App Watcher 打开处理失败",
        _ => "未知调试事件",
    }
}

fn dev_log_details(event: &str, details: &Value) -> Option<Value> {
    match event {
        "app_start" => Some(json!({})),
        "codex_remote_control_runtime_applied" => Some(pick_labeled_details(
            details,
            &[("context", "上下文"), ("remoteControl", "remote_control")],
        )),
        "codex_remote_control_runtime_updated" => Some(pick_labeled_details(
            details,
            &[
                ("context", "上下文"),
                ("remoteControl", "remote_control"),
                ("reason", "原因"),
            ],
        )),
        "codex_remote_control_subscription_home_prepared" => Some(pick_labeled_details(
            details,
            &[
                ("sessionProvider", "会话 provider"),
                ("apiBaseUrl", "API base_url"),
            ],
        )),
        "codex_remote_control_history_synced" => Some(pick_labeled_details(
            details,
            &[
                ("filesCopied", "复制文件数"),
                ("sessionIndexChanged", "session_index 已更新"),
                ("globalStateCopied", "global state 已复制"),
                ("stateThreadsMerged", "state threads 合并数"),
                ("rolloutFilesUpdated", "rollout provider 更新数"),
            ],
        )),
        "codex_remote_control_subscription_home_removed" => Some(json!({})),
        "codex_remote_control_helper_spawn" => Some(pick_labeled_details(
            details,
            &[("context", "上下文"), ("pid", "PID"), ("port", "端口")],
        )),
        "codex_remote_control_helper_keep_running" => Some(pick_labeled_details(
            details,
            &[
                ("context", "上下文"),
                ("pid", "PID"),
                ("port", "端口"),
                ("staleStopped", "已清理旧 helper 数"),
            ],
        )),
        "codex_remote_control_helper_stale_stop" => Some(pick_labeled_details(
            details,
            &[("context", "上下文"), ("count", "数量"), ("pids", "PID")],
        )),
        "codex_remote_control_helper_stop" => Some(pick_labeled_details(
            details,
            &[
                ("context", "上下文"),
                ("pid", "PID"),
                ("stopped", "已停止"),
                ("staleStopped", "已清理旧 helper 数"),
            ],
        )),
        "codex_remote_control_helper_error" => Some(pick_labeled_details(
            details,
            &[("context", "上下文"), ("error", "错误")],
        )),
        "codex_app_open_handler_start" => Some(pick_labeled_details(
            details,
            &[
                ("pluginUnlockEnabled", "Plugin 解锁已启用"),
                ("sessionSyncEnabled", "会话同步已启用"),
            ],
        )),
        "codex_app_open_handler_skip"
        | "codex_app_open_handler_remote_control_runtime_skip"
        | "codex_app_open_handler_remote_control_runtime_deferred"
        | "codex_app_open_handler_session_sync_skip"
        | "codex_app_open_handler_session_sync_deferred" => {
            Some(pick_labeled_details(details, &[("reason", "原因")]))
        }
        "codex_app_open_handler_remote_control_runtime_ready" => Some(pick_labeled_details(
            details,
            &[("helperChanged", "helper 已更新")],
        )),
        "codex_app_open_handler_remote_control_runtime_error"
        | "codex_app_restart_command_remote_control_runtime_error"
        | "codex_app_relaunch_processes_post_exit_remote_control_runtime_error" => {
            Some(pick_labeled_details(
                details,
                &[
                    ("context", "上下文"),
                    ("trigger", "触发来源"),
                    ("command", "命令"),
                    ("error", "错误"),
                ],
            ))
        }
        "codex_app_open_handler_session_sync_preflight_error" => Some(pick_labeled_details(
            details,
            &[("trigger", "触发来源"), ("error", "错误")],
        )),
        "codex_app_open_handler_finish" => Some(pick_labeled_details(
            details,
            &[("relaunchExpected", "预计重启")],
        )),
        "codex_app_restart_command_start" => {
            Some(pick_labeled_details(details, &[("command", "命令")]))
        }
        "codex_app_restart_command_skip"
        | "codex_app_restart_command_session_sync_skip"
        | "codex_app_restart_command_remote_control_runtime_skip" => Some(pick_labeled_details(
            details,
            &[
                ("command", "命令"),
                ("reason", "原因"),
                ("updated", "更新数量"),
            ],
        )),
        "codex_app_restart_command_session_sync_deferred"
        | "codex_app_restart_command_remote_control_runtime_deferred" => {
            Some(pick_labeled_details(
                details,
                &[
                    ("command", "命令"),
                    ("reason", "原因"),
                    ("updated", "更新数量"),
                ],
            ))
        }
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
        "codex_app_relaunch_processes_post_exit_config_apply_start" => Some(pick_labeled_details(
            details,
            &[("origin", "来源"), ("context", "上下文")],
        )),
        "codex_app_relaunch_processes_post_exit_config_apply_finish" => Some(pick_labeled_details(
            details,
            [
                ("origin", "来源"),
                ("context", "上下文"),
                ("details", "应用明细"),
            ]
            .as_slice(),
        )),
        "codex_app_relaunch_processes_post_exit_config_apply_error" => Some(pick_labeled_details(
            details,
            [("origin", "来源"), ("context", "上下文"), ("error", "错误")].as_slice(),
        )),
        "codex_app_relaunch_processes_post_exit_session_sync_start" => Some(pick_labeled_details(
            details,
            &[("origin", "来源"), ("trigger", "触发来源")],
        )),
        "codex_app_relaunch_processes_post_exit_session_sync_finish" => Some(pick_labeled_details(
            details,
            [
                ("origin", "来源"),
                ("trigger", "触发来源"),
                ("updated", "更新数量"),
            ]
            .as_slice(),
        )),
        "codex_app_relaunch_processes_post_exit_session_sync_error" => Some(pick_labeled_details(
            details,
            [
                ("origin", "来源"),
                ("trigger", "触发来源"),
                ("error", "错误"),
            ]
            .as_slice(),
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
        "session_sync_finish" | "session_sync_preflight_finish" => Some(pick_labeled_details(
            details,
            &[
                ("trigger", "触发来源"),
                ("targetProvider", "目标 provider"),
                ("stateDbUpdated", "state DB 更新数量"),
                ("rolloutFilesUpdated", "rollout 文件更新数量"),
                ("updated", "总更新数量"),
            ],
        )),
        "session_sync_error" | "session_sync_preflight_error" => Some(pick_labeled_details(
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
                ("selectedCount", "选中文件数"),
                ("extraRolloutCount", "固定文件数"),
            ],
        )),
        "session_sync_rollout_file_error" | "session_sync_preflight_rollout_file_error" => {
            Some(pick_labeled_details(
                details,
                &[
                    ("trigger", "触发来源"),
                    ("targetProvider", "目标 provider"),
                    ("path", "文件"),
                    ("error", "错误"),
                ],
            ))
        }
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
        "ide_reopen_discard_without_config_apply" => Some(pick_labeled_details(
            details,
            &[
                ("apiMode", "API 模式"),
                ("accountIdPresent", "账号 ID 存在"),
                ("sessionSyncProvider", "会话同步 provider"),
            ],
        )),
        "codex_app_watcher_scan_error" | "codex_app_watcher_on_open_error" => {
            Some(pick_labeled_details(details, &[("error", "错误")]))
        }
        _ => None,
    }
}

fn dev_log_event_visible(event: &str) -> bool {
    if event.ends_with("_error") || event == "session_sync_error" {
        return true;
    }

    matches!(
        event,
        "app_start"
            | "codex_remote_control_runtime_updated"
            | "codex_remote_control_runtime_applied"
            | "codex_remote_control_helper_spawn"
            | "codex_remote_control_helper_stop"
            | "codex_remote_control_helper_stale_stop"
            | "codex_app_open_handler_finish"
            | "codex_app_restart_command_finish"
            | "codex_app_relaunch_executable_finish"
            | "codex_app_relaunch_processes_finish"
            | "session_sync_finish"
            | "session_sync_preflight_finish"
            | "session_sync_state_db_summary"
            | "ide_reopen_confirm_finish"
            | "ide_reopen_discard_without_config_apply"
    )
}

pub(crate) fn init_session_sync_diagnostics(app: AppHandle) {
    let _ = DEV_LOG_APP.set(app);
}

pub(crate) fn log_session_sync_event(event: &str, details: Value) {
    if !cfg!(debug_assertions) {
        return;
    }
    if !dev_log_event_visible(event) {
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
