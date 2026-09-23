use crate::{
    blocking_task::run_blocking,
    session_sync_diagnostics::log_session_sync_event,
    settings::{remote_control_enabled_from_settings, update_settings_value},
};
use serde_json::{json, Value};
use std::time::Instant;
use tauri::AppHandle;

mod cdp;
mod codex_app_instances;
mod codex_app_open;
mod codex_app_watcher;
pub(crate) mod ide_snapshot;
mod process_control;
#[cfg(windows)]
mod process_permissions;
mod proxy_env;
pub(crate) mod remote_control;
mod shell;

pub(crate) use codex_app_instances::{codex_desktop_cli_source_path, codex_desktop_support_status};
pub(crate) use codex_app_watcher::CodexProcess;
pub(crate) use ide_snapshot::{attach_ide_reopen, build_ide_reopen_payload, IdeRuntime};
pub(crate) use proxy_env::apply_codex_proxy_env_state_to_settings;
pub(crate) use remote_control::{
    remote_control_codex_app_running, reset_remote_control_to_api_mode_settings,
    sync_remote_control_runtime_for_current_settings,
};

pub(crate) fn start_codex_app_watcher() {
    codex_app_watcher::start_codex_app_open_watcher(codex_app_open::handle_codex_app_open);
}

#[tauri::command]
pub(crate) async fn get_current_codex_app_processes() -> Result<Value, String> {
    run_blocking("检测 Codex 进程", || {
        codex_app_watcher::current_codex_app_processes_value(&codex_desktop_support_status())
    })
    .await
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
pub(crate) async fn open_codex_app_instance(
    app: AppHandle,
    payload: Value,
) -> Result<Value, String> {
    run_blocking("打开 Codex 多开实例", move || {
        codex_app_instances::open_codex_app_instance(app, payload)
    })
    .await
}

#[tauri::command]
pub(crate) async fn show_codex_app_instance(payload: Value) -> Result<Value, String> {
    run_blocking("显示 Codex 多开窗口", move || {
        codex_app_instances::show_codex_app_instance(payload)
    })
    .await
}

#[tauri::command]
pub(crate) async fn get_codex_app_instance_status() -> Result<Value, String> {
    run_blocking(
        "读取 Codex 多开状态",
        codex_app_instances::get_codex_app_instance_status,
    )
    .await
}

async fn run_codex_restart(
    command: &'static str,
    restart: impl FnOnce() -> Result<Value, String> + Send + 'static,
) -> Result<Value, String> {
    let started = Instant::now();
    let result = run_blocking("重启 Codex", restart).await;
    if let Err(err) = &result {
        crate::session_sync_diagnostics::log_session_sync_event(
            "codex_app_restart_command_error",
            json!({ "command": command, "elapsedMs": started.elapsed().as_millis(), "error": err }),
        );
    }
    result
}

#[tauri::command]
pub(crate) async fn set_codex_proxy_env_enabled(
    enabled: bool,
    proxy_url: String,
) -> Result<Value, String> {
    run_blocking("保存 Codex 代理", move || {
        set_codex_proxy_env_enabled_impl(enabled, &proxy_url)
    })
    .await
}

fn set_codex_proxy_env_enabled_impl(enabled: bool, proxy_url: &str) -> Result<Value, String> {
    let proxy_url = proxy_env::set_codex_proxy_env_file_enabled(enabled, proxy_url)?;
    let mut patch = json!({
        "codex_proxy_env_enabled": enabled
    });
    if enabled {
        patch["codex_proxy_url"] = Value::String(proxy_url.clone());
    }
    let settings =
        proxy_env::apply_codex_proxy_env_state_to_settings(update_settings_value(&patch)?)?;
    let remote_control_runtime = sync_remote_control_runtime_after_proxy_change(&settings);

    Ok(json!({
        "ok": true,
        "message": if enabled {
            "Codex 代理配置已启用，重启 Codex 后生效。"
        } else {
            "Codex 代理配置已关闭，重启 Codex 后生效。"
        },
        "restartRequired": true,
        "settings": settings,
        "env_path": proxy_env::codex_env_path()?.to_string_lossy().to_string(),
        "proxy_url": proxy_url,
        "remoteControl": remote_control_runtime
    }))
}

fn sync_remote_control_runtime_after_proxy_change(settings: &Value) -> Value {
    if !remote_control_enabled_from_settings(settings) {
        return json!({ "changed": false });
    }

    match sync_remote_control_runtime_for_current_settings("set_codex_proxy_env_enabled") {
        Ok(changed) => json!({ "changed": changed }),
        Err(err) => {
            let error = err.clone();
            log_session_sync_event(
                "codex_remote_control_helper_error",
                json!({
                    "context": "set_codex_proxy_env_enabled",
                    "error": error
                }),
            );
            json!({ "changed": false, "error": err })
        }
    }
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

#[cfg(test)]
mod proxy_settings_tests {
    use super::{proxy_env::codex_env_path, *};
    use std::{fs, path::PathBuf};

    #[test]
    #[ignore = "requires isolated USERPROFILE and APPDATA"]
    fn proxy_save_reports_restart_and_preserves_other_env_values() {
        let test_home = PathBuf::from(
            std::env::var("CODEX_SWITCH_TEST_HOME").expect("explicit fixture home required"),
        );
        assert_eq!(crate::paths::home_dir().unwrap(), test_home);
        assert!(crate::paths::app_data_dir()
            .unwrap()
            .starts_with(&test_home));
        let path = codex_env_path().unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "FIXTURE_VALUE=preserved\n").unwrap();
        update_settings_value(&json!({"codex_remote_control_enabled": false})).unwrap();
        let enabled = set_codex_proxy_env_enabled_impl(true, "127.0.0.1:10808").unwrap();
        assert_eq!(enabled["restartRequired"], true);
        assert!(enabled["message"].as_str().unwrap().contains("重启 Codex"));
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("FIXTURE_VALUE=preserved"));
        assert!(content.contains("HTTP_PROXY=http://127.0.0.1:10808"));
        let disabled = set_codex_proxy_env_enabled_impl(false, "").unwrap();
        assert_eq!(disabled["restartRequired"], true);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "FIXTURE_VALUE=preserved\n"
        );
        assert!(set_codex_proxy_env_enabled_impl(true, "")
            .unwrap_err()
            .contains("代理地址不能为空"));
    }
}
