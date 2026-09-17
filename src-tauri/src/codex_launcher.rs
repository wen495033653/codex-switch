use serde_json::{json, Value};
use std::{collections::HashMap, sync::Mutex, time::Instant};
use tauri::AppHandle;

pub(crate) struct IdePending {
    snapshot: Value,
    account_id: String,
    api_mode: bool,
    session_sync_provider: Option<String>,
}

#[derive(Default)]
pub(crate) struct IdeRuntime {
    snapshots: Mutex<HashMap<String, IdePending>>,
}

mod cdp;
pub(crate) mod codex_app;
mod codex_app_instances;
mod codex_app_open;
mod codex_app_watcher;
pub(crate) mod ide_snapshot;
mod process_control;
pub(crate) mod remote_control;
mod shell;

pub(crate) use codex_app::apply_codex_proxy_env_state_to_settings;
pub(crate) use codex_app_instances::{codex_desktop_cli_source_path, codex_desktop_support_status};
pub(crate) use codex_app_watcher::CodexProcess;
pub(crate) use ide_snapshot::{attach_ide_reopen, build_ide_reopen_payload};
pub(crate) use remote_control::{
    remote_control_codex_app_running, reset_remote_control_to_api_mode_settings,
    sync_remote_control_runtime_for_current_settings,
};

pub(crate) fn start_codex_app_watcher() {
    codex_app_watcher::start_codex_app_open_watcher(codex_app_open::handle_codex_app_open);
}

#[tauri::command]
pub(crate) async fn get_current_codex_app_processes() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        codex_app_watcher::current_codex_app_processes_value(&codex_desktop_support_status())
    })
    .await
    .map_err(|err| format!("后台检测 Codex 进程失败: {err}"))?
}

#[tauri::command]
pub(crate) async fn restart_current_codex_app_normal() -> Result<Value, String> {
    run_codex_restart(
        "restart_current_codex_app_normal",
        codex_app_open::restart_current_codex_app_normal,
    )
    .await
}

#[tauri::command]
pub(crate) fn open_codex_app_instance(app: AppHandle, payload: Value) -> Result<Value, String> {
    codex_app_instances::open_codex_app_instance(app, payload)
}

#[tauri::command]
pub(crate) fn show_codex_app_instance(payload: Value) -> Result<Value, String> {
    codex_app_instances::show_codex_app_instance(payload)
}

#[tauri::command]
pub(crate) fn get_codex_app_instance_status() -> Result<Value, String> {
    codex_app_instances::get_codex_app_instance_status()
}

async fn run_codex_restart(
    command: &'static str,
    restart: impl FnOnce() -> Result<Value, String> + Send + 'static,
) -> Result<Value, String> {
    let started = Instant::now();
    let result = tauri::async_runtime::spawn_blocking(restart)
        .await
        .map_err(|err| format!("后台重启 Codex 任务失败: {err}"))
        .and_then(|result| result);
    if let Err(err) = &result {
        crate::session_sync_diagnostics::log_session_sync_event(
            "codex_app_restart_command_error",
            json!({ "command": command, "elapsedMs": started.elapsed().as_millis(), "error": err }),
        );
    }
    result
}

#[cfg(test)]
mod restart_tests {
    use super::*;
    use std::thread;

    #[test]
    fn restart_runs_off_calling_thread_and_returns_result() {
        let caller = thread::current().id();
        let result = tauri::async_runtime::block_on(run_codex_restart("test_restart", move || {
            assert_ne!(caller, thread::current().id());
            Ok(json!({ "ok": true, "restartedCount": 1 }))
        }))
        .unwrap();
        assert_eq!(result["restartedCount"], 1);
    }

    #[test]
    fn restart_returns_worker_failure() {
        let result =
            tauri::async_runtime::block_on(run_codex_restart("test_restart_failure", || {
                Err("fixture: process did not exit".to_string())
            }));
        assert_eq!(result.unwrap_err(), "fixture: process did not exit");
    }
}
