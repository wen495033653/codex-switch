use crate::time_util::now_string;
use serde_json::{json, Map, Value};
use std::{
    collections::VecDeque,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};
use tauri::{AppHandle, Emitter};

mod runtime_log;

const DEV_LOG_EVENT: &str = "dev-log";
// Stable code survives the String-based command/watcher error chain.
pub(crate) const PROCESS_ELEVATION_WARNING: &str = "[PROCESS_ELEVATION_MISMATCH]";
const MAX_DEV_LOG_BUFFER: usize = 300;
const ERROR_LOG_DIR_NAME: &str = "logs";
const ERROR_LOG_FILE_NAME: &str = "codex-switch-errors.jsonl";
const ERROR_LOG_ROTATED_FILE_NAME: &str = "codex-switch-errors.1.jsonl";
// A watcher retry loop can log one error every few seconds; one rotated generation bounds the
// log at twice this size.
const ERROR_LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;

static ERROR_LOG_WRITE_LOCK: Mutex<()> = Mutex::new(());
static LOG_WRITE_ERROR: Mutex<Option<String>> = Mutex::new(None);
const MAX_RUNTIME_LOG_ENTRIES: usize = 500;

static DEV_LOG_APP: OnceLock<AppHandle> = OnceLock::new();
static DEV_LOG_BUFFER: OnceLock<Mutex<Vec<Value>>> = OnceLock::new();
static DEV_LOG_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn dev_log_buffer() -> &'static Mutex<Vec<Value>> {
    DEV_LOG_BUFFER.get_or_init(|| Mutex::new(Vec::new()))
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
    } else {
        "debug"
    }
}

fn is_error_event(event: &str) -> bool {
    event.ends_with("_error") || event == "session_sync_error"
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
        || event.starts_with("codex_desktop_")
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
        "codex_app_watcher_on_open_panic_error" => "Codex App Watcher 打开处理异常",
        "codex_app_watcher_panic_error" => "Codex App Watcher 异常退出",
        "codex_app_process_kill_error" => "进程关闭失败",
        "codex_app_launch_confirmation_error" => "Codex 启动检查失败",
        "codex_app_restart_command_error" => "Codex 重启失败",
        "codex_app_multi_open_error" => "独立 Codex 启动失败",
        "codex_app_multi_open_show_error" => "显示 Codex 窗口失败",
        "codex_desktop_data_migration_error" => "Codex Desktop 数据迁移失败",
        "codex_app_instance_data_migration_error" => "Codex 多开数据迁移失败",
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
            &[("sessionSyncEnabled", "会话同步已启用")],
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
        "codex_app_watcher_scan_error"
        | "codex_app_watcher_on_open_error"
        | "codex_app_watcher_on_open_panic_error"
        | "codex_app_watcher_panic_error" => Some(pick_labeled_details(
            details,
            &[("error", "错误"), ("retry", "将重试")],
        )),
        "codex_desktop_data_migration_error" => Some(pick_labeled_details(
            details,
            &[("root", "Codex home"), ("error", "错误")],
        )),
        "codex_app_instance_data_migration_error" => Some(pick_labeled_details(
            details,
            &[("codexHome", "Codex home"), ("error", "错误")],
        )),
        "codex_app_process_kill_error"
        | "codex_app_multi_open_error"
        | "codex_app_multi_open_show_error"
        | "codex_app_restart_command_error"
        | "codex_app_process_kill_finish"
        | "codex_app_process_kill_tree_exited"
        | "codex_app_launch_confirmation_error"
        | "codex_app_launch_confirmation_finish" => Some(details.clone()),
        _ => None,
    }
}

fn dev_log_event_visible(event: &str) -> bool {
    if is_error_event(event) {
        return true;
    }

    matches!(
        event,
        "app_start"
            | "codex_app_process_kill_finish"
            | "codex_app_process_kill_tree_exited"
            | "codex_app_launch_confirmation_finish"
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

fn error_log_path() -> Result<PathBuf, String> {
    Ok(crate::paths::app_data_dir()?
        .join(ERROR_LOG_DIR_NAME)
        .join(ERROR_LOG_FILE_NAME))
}

// Keep the established filename and rotation so existing error history remains readable.
// It now also contains selected success/warning results, including in release builds.
fn append_error_log(path: &Path, event: &str, details: &Value) -> Result<(), String> {
    let _guard = ERROR_LOG_WRITE_LOCK
        .lock()
        .map_err(|_| "错误日志写入锁异常".to_string())?;
    let dir = path
        .parent()
        .ok_or_else(|| format!("错误日志路径无父目录: {}", path.display()))?;
    fs::create_dir_all(dir)
        .map_err(|err| format!("创建错误日志目录失败 {}: {err}", dir.display()))?;
    if fs::metadata(path).is_ok_and(|metadata| metadata.len() >= ERROR_LOG_MAX_BYTES) {
        let rotated = dir.join(ERROR_LOG_ROTATED_FILE_NAME);
        fs::rename(path, &rotated).map_err(|err| {
            format!(
                "轮转错误日志失败 {} -> {}: {err}",
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
        "level": runtime_log_view(event, details).map(|entry| entry["level"].clone())
            .unwrap_or_else(|| json!(event_level(event, details))),
        "details": details
    })
    .to_string();
    line.push('\n');
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(line.as_bytes()))
        .map_err(|err| format!("写入错误日志失败 {}: {err}", path.display()))
}

pub(crate) fn init_session_sync_diagnostics(app: AppHandle) {
    let _ = DEV_LOG_APP.set(app);
}

pub(crate) fn log_session_sync_event(event: &str, details: Value) {
    // Unit tests exercise error paths; they must not write into the user's real data directory.
    // Production disk logging was confirmed on 2026-09-20; see docs/development/codex-restart.md.
    #[cfg(not(test))]
    if runtime_log_view(event, &details).is_some() {
        if let Err(err) = error_log_path().and_then(|path| append_error_log(&path, event, &details))
        {
            eprintln!("{} event={event}: {err}", now_string());
            if let Ok(mut failure) = LOG_WRITE_ERROR.lock() {
                *failure = Some(err);
            }
        }
    }
    if !cfg!(debug_assertions) && !is_error_event(event) {
        return;
    }
    if !dev_log_event_visible(event) {
        return;
    }

    let level = event_level(event, &details);
    let Some(details) = dev_log_details(event, &details) else {
        return;
    };

    let payload = json!({
        "sequence": DEV_LOG_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        "level": level,
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

fn runtime_log_view(event: &str, details: &Value) -> Option<Value> {
    runtime_log::project(
        event,
        details,
        event_level(event, details),
        dev_log_message(event),
    )
}

fn read_runtime_log_entries(path: &Path) -> Result<Vec<Value>, String> {
    let _guard = ERROR_LOG_WRITE_LOCK
        .lock()
        .map_err(|_| "日志读取锁异常".to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| "日志路径无父目录".to_string())?;
    let mut entries = VecDeque::new();
    for file_path in [parent.join(ERROR_LOG_ROTATED_FILE_NAME), path.to_path_buf()] {
        let file = match fs::File::open(&file_path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("读取日志失败 {}: {error}", file_path.display())),
        };
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| {
                format!(
                    "读取日志失败 {} 第 {} 行: {error}",
                    file_path.display(),
                    index + 1
                )
            })?;
            let raw: Value = serde_json::from_str(&line).map_err(|error| {
                format!(
                    "日志格式错误 {} 第 {} 行: {error}",
                    file_path.display(),
                    index + 1
                )
            })?;
            let event = raw.get("event").and_then(Value::as_str).ok_or_else(|| {
                format!("日志缺少 event {} 第 {} 行", file_path.display(), index + 1)
            })?;
            let timestamp = raw
                .get("timestamp")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    format!(
                        "日志缺少 timestamp {} 第 {} 行",
                        file_path.display(),
                        index + 1
                    )
                })?;
            if let Some(mut entry) = runtime_log_view(event, &raw["details"]) {
                entry["id"] = json!(format!(
                    "{}:{}",
                    file_path.file_name().unwrap_or_default().to_string_lossy(),
                    index
                ));
                entry["timestamp"] = json!(timestamp);
                entry["version"] = raw["version"].clone();
                entries.push_back(entry);
                if entries.len() > MAX_RUNTIME_LOG_ENTRIES {
                    entries.pop_front();
                }
            }
        }
    }
    Ok(entries.into_iter().rev().collect())
}

#[tauri::command]
pub(crate) async fn get_runtime_log_entries() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let entries = read_runtime_log_entries(&error_log_path()?)?;
        let write_error = LOG_WRITE_ERROR
            .lock()
            .map_err(|_| "日志状态锁异常".to_string())?
            .clone();
        Ok(json!({"entries": entries, "writeError": write_error}))
    })
    .await
    .map_err(|error| format!("读取运行日志任务失败: {error}"))?
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

    #[test]
    fn runtime_log_reads_durable_success_warning_error_and_legacy_history_newest_first() {
        let path = unique_temp_log_path("runtime-levels");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{\"timestamp\":\"2026-09-20T01:00:00Z\",\"event\":\"session_sync_error\",\"details\":{\"error\":\"legacy error\"}}\n").unwrap();
        append_error_log(&path, "session_sync_finish", &json!({"updated": 3})).unwrap();
        append_error_log(
            &path,
            "session_sync_preflight_finish",
            &json!({"updated": 2, "rolloutFilesUpdated": 2}),
        )
        .unwrap();
        append_error_log(&path, "codex_app_watcher_on_open_error", &json!({"retry": false,
            "error": format!("{PROCESS_ELEVATION_WARNING} callerElevated=false, targetElevated=true")})).unwrap();
        let entries = read_runtime_log_entries(&path).unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0]["level"], "warn");
        assert_eq!(entries[0]["title"], "自动处理已暂停");
        assert_eq!(entries[1]["level"], "warn");
        assert_eq!(entries[2]["level"], "success");
        assert_eq!(entries[3]["level"], "error");
        let disk = fs::read_to_string(&path).unwrap();
        let preflight: Value = serde_json::from_str(disk.lines().nth(2).unwrap()).unwrap();
        assert_eq!(preflight["level"], "warn");
        println!("runtime log fixture: {}", path.display());
    }

    #[test]
    fn runtime_log_reads_rotated_entries_and_limits_to_latest_500() {
        let path = unique_temp_log_path("runtime-retention");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let rotated = path.parent().unwrap().join(ERROR_LOG_ROTATED_FILE_NAME);
        let mut lines = String::new();
        for i in 0..501 {
            lines.push_str(&json!({"event": "session_sync_finish", "timestamp": "2026-09-20T01:00:00Z", "details": {"updated": i}}).to_string());
            lines.push('\n');
        }
        fs::write(rotated, lines).unwrap();
        append_error_log(&path, "session_sync_finish", &json!({"updated": 501})).unwrap();
        let entries = read_runtime_log_entries(&path).unwrap();
        assert_eq!(entries.len(), 500);
        assert_eq!(entries[0]["fields"][0]["value"], 501);
        assert_eq!(entries[499]["fields"][0]["value"], 2);
    }

    #[test]
    fn runtime_log_io_and_parse_failures_are_not_reported_as_empty_success() {
        let path = unique_temp_log_path("runtime-errors");
        assert!(read_runtime_log_entries(&path).unwrap().is_empty());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "broken json\n").unwrap();
        assert!(read_runtime_log_entries(&path)
            .unwrap_err()
            .contains("第 1 行"));
        // A regular file used as the directory triggers a real write error, without permission assumptions.
        let error = append_error_log(
            &path.join("not-a-directory.jsonl"),
            "session_sync_error",
            &json!({"error": "test"}),
        )
        .unwrap_err();
        assert!(error.contains("创建错误日志目录失败"));
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
            append_error_log(&path, event, &details).unwrap();
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

    fn unique_temp_log_path(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir()
            .join(format!("codex-switch-{name}-{stamp}"))
            .join(ERROR_LOG_DIR_NAME)
            .join(ERROR_LOG_FILE_NAME)
    }

    #[test]
    fn error_log_appends_one_json_line_per_event_with_full_details() {
        let path = unique_temp_log_path("error-log");
        append_error_log(
            &path,
            "codex_app_process_kill_error",
            &json!({"pid": 42, "error": "taskkill /F /T /PID 42: exit code 128"}),
        )
        .unwrap();
        append_error_log(&path, "session_sync_error", &json!({"error": "second"})).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
        let lines = text.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["event"], "codex_app_process_kill_error");
        assert_eq!(first["details"]["pid"], 42);
        assert_eq!(
            first["details"]["error"],
            "taskkill /F /T /PID 42: exit code 128"
        );
        assert_eq!(first["version"], env!("CARGO_PKG_VERSION"));
        assert!(!first["timestamp"].as_str().unwrap().is_empty());
        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["event"], "session_sync_error");
    }

    #[test]
    fn watcher_failure_log_persists_no_retry_and_process_context() {
        let path = unique_temp_log_path("watcher-no-retry");
        let details = json!({
            "error": "taskkill /F /T /PID 42: exitCode=128; access denied",
            "retry": false,
            "disabledUntil": "codex_switch_restart",
            "processes": [{"pid": 42, "parentPid": 1, "startedAt": 100, "executablePath": "Codex/ChatGPT.exe"}]
        });
        append_error_log(&path, "codex_app_watcher_on_open_error", &details).unwrap();
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
    fn error_log_rotates_once_the_size_limit_is_reached() {
        let path = unique_temp_log_path("error-log-rotate");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![b'x'; ERROR_LOG_MAX_BYTES as usize]).unwrap();

        append_error_log(&path, "session_sync_error", &json!({"error": "after"})).unwrap();

        let rotated = path.parent().unwrap().join(ERROR_LOG_ROTATED_FILE_NAME);
        let rotated_len = fs::metadata(&rotated).unwrap().len();
        let current = fs::read_to_string(&path).unwrap();
        fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
        assert_eq!(rotated_len, ERROR_LOG_MAX_BYTES);
        assert_eq!(current.lines().count(), 1);
        assert!(current.contains("\"after\""));
    }
}
