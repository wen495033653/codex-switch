use super::{
    codex_processes_have_cdp_launch, kill_process_tree, launch_codex_with_cdp_hooks,
    relaunch_executable_with_retry, wait_for_pids_exit, CodexAppOpenOutcome, CodexCdpLaunchHooks,
    CodexProcess,
};
use crate::{
    codex_launcher::{prepare_remote_control_hook, remote_control_hook_enabled_from_settings},
    codex_sessions::sync_codex_sessions_to_current_mode_now_from,
    json_util::bool_field,
    session_sync_diagnostics::log_session_sync_event,
    settings::read_settings_value,
};
use serde_json::{json, Value};
use std::{path::Path, thread, time::Duration as StdDuration};

const RELAUNCH_DELAY_MS: u64 = 1_500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodexRelaunchMode {
    Normal,
    Cdp(CodexCdpLaunchHooks),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CodexAppOpenActions {
    plugin_unlock_enabled: bool,
    remote_control_hook_enabled: bool,
    session_sync_enabled: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CodexAppOpenStatus {
    session_sync_updated: bool,
    cdp_launch_applied: bool,
}

impl CodexAppOpenActions {
    fn from_settings(settings: &Value) -> Self {
        Self {
            plugin_unlock_enabled: bool_field(settings, "codex_plugins_enabled"),
            remote_control_hook_enabled: remote_control_hook_enabled_from_settings(settings),
            session_sync_enabled: settings
                .get("codex_session_sync_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        }
    }

    fn enabled(self) -> bool {
        self.plugin_unlock_enabled || self.remote_control_hook_enabled || self.session_sync_enabled
    }
}

pub(crate) fn handle_codex_app_open(
    processes: &[CodexProcess],
) -> Result<CodexAppOpenOutcome, String> {
    let actions = codex_app_open_actions()?;
    log_session_sync_event(
        "codex_app_open_handler_start",
        json!({
            "processes": codex_processes_log_value(processes),
            "pluginUnlockEnabled": actions.plugin_unlock_enabled,
            "remoteControlHookEnabled": actions.remote_control_hook_enabled,
            "sessionSyncEnabled": actions.session_sync_enabled
        }),
    );
    if !actions.enabled() {
        log_session_sync_event(
            "codex_app_open_handler_skip",
            json!({
                "reason": "all_actions_disabled",
                "processes": codex_processes_log_value(processes)
            }),
        );
        return Ok(CodexAppOpenOutcome::default());
    }

    let mut session_sync_updated = false;
    if actions.remote_control_hook_enabled {
        prepare_remote_control_hook("codex_app_open_handler")?;
    }

    if actions.session_sync_enabled {
        match sync_codex_sessions_to_current_mode_now_from("codex_app_open_handler") {
            Ok(updated) => session_sync_updated = updated > 0,
            Err(err) => eprintln!("Codex app 会话同步失败: {err}"),
        }
    } else {
        log_session_sync_event(
            "codex_app_open_handler_session_sync_skip",
            json!({ "reason": "setting_disabled" }),
        );
    }

    let status = CodexAppOpenStatus {
        session_sync_updated,
        cdp_launch_applied: codex_processes_have_cdp_launch(processes),
    };
    let Some(relaunch_mode) = codex_relaunch_mode_for_app_open(actions, status) else {
        log_session_sync_event(
            "codex_app_open_handler_finish",
            json!({ "relaunchExpected": false }),
        );
        return Ok(CodexAppOpenOutcome::default());
    };

    let restarted =
        relaunch_running_codex_processes(processes, relaunch_mode, CodexRelaunchOrigin::Watcher)?;
    log_session_sync_event(
        "codex_app_open_handler_finish",
        json!({ "relaunchExpected": restarted > 0 }),
    );
    Ok(CodexAppOpenOutcome {
        relaunch_expected: restarted > 0,
    })
}

fn codex_app_open_actions() -> Result<CodexAppOpenActions, String> {
    read_settings_value().map(|settings| CodexAppOpenActions::from_settings(&settings))
}

fn codex_relaunch_mode_for_app_open(
    actions: CodexAppOpenActions,
    status: CodexAppOpenStatus,
) -> Option<CodexRelaunchMode> {
    let cdp_hooks = codex_cdp_launch_hooks_for_actions(actions);
    if actions.session_sync_enabled && status.session_sync_updated {
        return Some(if let Some(hooks) = cdp_hooks {
            CodexRelaunchMode::Cdp(hooks)
        } else {
            CodexRelaunchMode::Normal
        });
    }
    if let Some(hooks) = cdp_hooks {
        if !status.cdp_launch_applied {
            return Some(CodexRelaunchMode::Cdp(hooks));
        }
    }
    None
}

fn codex_relaunch_mode_for_actions(actions: CodexAppOpenActions) -> CodexRelaunchMode {
    if let Some(hooks) = codex_cdp_launch_hooks_for_actions(actions) {
        CodexRelaunchMode::Cdp(hooks)
    } else {
        CodexRelaunchMode::Normal
    }
}

fn codex_cdp_launch_hooks_for_actions(actions: CodexAppOpenActions) -> Option<CodexCdpLaunchHooks> {
    if actions.plugin_unlock_enabled || actions.remote_control_hook_enabled {
        Some(CodexCdpLaunchHooks {
            plugin_unlock: actions.plugin_unlock_enabled,
            remote_control_hook: actions.remote_control_hook_enabled,
        })
    } else {
        None
    }
}

fn sync_codex_sessions_for_app_command(
    command: &str,
    actions: CodexAppOpenActions,
) -> Result<(), String> {
    if !actions.session_sync_enabled {
        log_session_sync_event(
            "codex_app_restart_command_session_sync_skip",
            json!({
                "command": command,
                "reason": "setting_disabled"
            }),
        );
        return Ok(());
    }

    log_session_sync_event(
        "codex_app_restart_command_session_sync_start",
        json!({
            "command": command,
            "directSessionSync": true
        }),
    );
    match sync_codex_sessions_to_current_mode_now_from(command) {
        Ok(updated) => {
            log_session_sync_event(
                "codex_app_restart_command_session_sync_finish",
                json!({
                    "command": command,
                    "directSessionSync": true,
                    "updated": updated,
                    "openHandlerMayBeSkippedByExpectedOpen": true
                }),
            );
            Ok(())
        }
        Err(err) => {
            log_session_sync_event(
                "codex_app_restart_command_session_sync_error",
                json!({
                    "command": command,
                    "directSessionSync": true,
                    "error": err.clone()
                }),
            );
            Err(format!("Codex app 会话同步失败，已取消重启：{err}"))
        }
    }
}

pub(crate) fn restart_current_codex_app_for_plugin_setting() -> Result<Value, String> {
    let command = "restart_current_codex_app_for_plugin_setting";
    let processes = super::codex_app_watcher::refresh_current_codex_app_processes()?;
    log_session_sync_event(
        "codex_app_restart_command_start",
        json!({
            "command": command,
            "processes": codex_processes_log_value(&processes)
        }),
    );
    if processes.is_empty() {
        log_session_sync_event(
            "codex_app_restart_command_skip",
            json!({
                "command": command,
                "reason": "no_running_codex_app"
            }),
        );
        return Ok(json!({
            "ok": true,
            "message": "未检测到正在运行的 Codex app"
        }));
    }

    let actions = codex_app_open_actions()?;
    sync_codex_sessions_for_app_command(command, actions)?;
    let restarted = relaunch_running_codex_processes(
        &processes,
        codex_relaunch_mode_for_actions(actions),
        CodexRelaunchOrigin::AppCommand,
    )?;
    log_session_sync_event(
        "codex_app_restart_command_finish",
        json!({
            "command": command,
            "restarted": restarted > 0,
            "restartedCount": restarted
        }),
    );
    Ok(json!({
        "ok": true,
        "message": if restarted > 0 { "Codex app 已重启" } else { "未能重新打开 Codex app" },
        "restarted": restarted > 0,
        "restartedCount": restarted
    }))
}

pub(crate) fn restart_current_codex_app_normal() -> Result<Value, String> {
    let command = "restart_current_codex_app_normal";
    let processes = super::codex_app_watcher::refresh_current_codex_app_processes()?;
    log_session_sync_event(
        "codex_app_restart_command_start",
        json!({
            "command": command,
            "processes": codex_processes_log_value(&processes)
        }),
    );
    if processes.is_empty() {
        log_session_sync_event(
            "codex_app_restart_command_skip",
            json!({
                "command": command,
                "reason": "no_running_codex_app"
            }),
        );
        return Ok(json!({
            "ok": true,
            "message": "未检测到正在运行的 Codex app"
        }));
    }

    let actions = codex_app_open_actions()?;
    sync_codex_sessions_for_app_command(command, actions)?;
    let restarted = relaunch_running_codex_processes(
        &processes,
        codex_relaunch_mode_for_actions(actions),
        CodexRelaunchOrigin::AppCommand,
    )?;
    log_session_sync_event(
        "codex_app_restart_command_finish",
        json!({
            "command": command,
            "restarted": restarted > 0,
            "restartedCount": restarted
        }),
    );
    Ok(json!({
        "ok": true,
        "message": if restarted > 0 { "Codex app 已重启" } else { "未能重新打开 Codex app" },
        "restarted": restarted > 0,
        "restartedCount": restarted
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodexRelaunchOrigin {
    Watcher,
    AppCommand,
}

fn codex_relaunch_mode_for_current_settings() -> Result<CodexRelaunchMode, String> {
    let actions = codex_app_open_actions()?;
    Ok(codex_relaunch_mode_for_actions(actions))
}

pub(crate) fn relaunch_codex_executable_for_current_settings(
    executable: &str,
) -> Result<bool, String> {
    let path = Path::new(executable);
    if !path.exists() {
        log_session_sync_event(
            "codex_app_relaunch_executable_skip",
            json!({
                "executable": executable,
                "reason": "missing_executable"
            }),
        );
        return Ok(false);
    }
    let mode = codex_relaunch_mode_for_current_settings()?;
    let executables = vec![executable.to_string()];
    log_session_sync_event(
        "codex_app_relaunch_executable_expect_open",
        json!({
            "executable": executable,
            "mode": format!("{mode:?}")
        }),
    );
    super::codex_app_watcher::expect_codex_app_open_for_executables(&executables);
    match relaunch_codex_executable(executable, mode) {
        Ok(restarted) => {
            log_session_sync_event(
                "codex_app_relaunch_executable_finish",
                json!({
                    "executable": executable,
                    "mode": format!("{mode:?}"),
                    "restarted": restarted
                }),
            );
            Ok(restarted)
        }
        Err(err) => {
            super::codex_app_watcher::clear_expected_codex_app_open_for_executables(&executables);
            log_session_sync_event(
                "codex_app_relaunch_executable_error",
                json!({
                    "executable": executable,
                    "mode": format!("{mode:?}"),
                    "error": err.clone()
                }),
            );
            Err(err)
        }
    }
}

fn relaunch_running_codex_processes(
    processes: &[CodexProcess],
    mode: CodexRelaunchMode,
    origin: CodexRelaunchOrigin,
) -> Result<usize, String> {
    let pids = processes
        .iter()
        .map(|process| process.pid)
        .collect::<Vec<_>>();
    let mut executables = processes
        .iter()
        .map(|process| process.executable_path.clone())
        .filter(|path| !path.trim().is_empty())
        .collect::<Vec<_>>();
    executables.sort_by_key(|path| path.trim().to_ascii_lowercase());
    executables.dedup_by_key(|path| path.trim().to_ascii_lowercase());
    if executables.is_empty() {
        return Err("未检测到 Codex app 可执行路径".to_string());
    }
    log_session_sync_event(
        "codex_app_relaunch_processes_start",
        json!({
            "origin": format!("{origin:?}"),
            "mode": format!("{mode:?}"),
            "pids": pids.clone(),
            "executables": executables.clone()
        }),
    );

    for pid in &pids {
        let _ = kill_process_tree(*pid);
    }
    let alive = wait_for_pids_exit(&pids, 12_000);
    if !alive.is_empty() {
        return Err("Codex app 进程未能退出".to_string());
    }

    thread::sleep(StdDuration::from_millis(RELAUNCH_DELAY_MS));

    if origin == CodexRelaunchOrigin::AppCommand {
        log_session_sync_event(
            "codex_app_relaunch_processes_expect_open",
            json!({
                "origin": format!("{origin:?}"),
                "executables": executables.clone()
            }),
        );
        super::codex_app_watcher::expect_codex_app_open_for_executables(&executables);
    }

    let mut restarted = 0usize;
    for executable in &executables {
        match relaunch_codex_executable(executable, mode) {
            Ok(true) => restarted += 1,
            Ok(false) => {}
            Err(err) => {
                if origin == CodexRelaunchOrigin::AppCommand {
                    super::codex_app_watcher::clear_expected_codex_app_open_for_executables(
                        &executables,
                    );
                }
                log_session_sync_event(
                    "codex_app_relaunch_processes_error",
                    json!({
                        "origin": format!("{origin:?}"),
                        "mode": format!("{mode:?}"),
                        "error": err.clone()
                    }),
                );
                return Err(err);
            }
        }
        thread::sleep(StdDuration::from_millis(120));
    }

    log_session_sync_event(
        "codex_app_relaunch_processes_finish",
        json!({
            "origin": format!("{origin:?}"),
            "mode": format!("{mode:?}"),
            "restartedCount": restarted
        }),
    );
    Ok(restarted)
}

fn relaunch_codex_executable(executable: &str, mode: CodexRelaunchMode) -> Result<bool, String> {
    match mode {
        CodexRelaunchMode::Cdp(hooks) => {
            if hooks.remote_control_hook {
                prepare_remote_control_hook("codex_app_relaunch")?;
            }
            launch_codex_with_cdp_hooks(Path::new(executable), hooks)?;
            Ok(true)
        }
        CodexRelaunchMode::Normal => relaunch_executable_with_retry(executable),
    }
}

fn codex_processes_log_value(processes: &[CodexProcess]) -> Value {
    Value::Array(
        processes
            .iter()
            .map(|process| {
                json!({
                    "pid": process.pid,
                    "executablePath": process.executable_path.as_str()
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn codex_app_open_actions_keep_plugin_and_session_actions_separate() {
        let session_only = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": false,
            "codex_remote_control_hook_enabled": false,
            "codex_session_sync_enabled": true
        }));
        let plugin_only = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": true,
            "codex_remote_control_hook_enabled": false,
            "codex_session_sync_enabled": false
        }));
        let remote_control_only = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": false,
            "codex_remote_control_hook_enabled": true,
            "codex_session_sync_enabled": false
        }));
        let disabled = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": false,
            "codex_remote_control_hook_enabled": false,
            "codex_session_sync_enabled": false
        }));

        assert!(session_only.enabled());
        assert!(session_only.session_sync_enabled);
        assert!(!session_only.plugin_unlock_enabled);
        assert!(plugin_only.enabled());
        assert!(plugin_only.plugin_unlock_enabled);
        assert!(!plugin_only.session_sync_enabled);
        assert!(remote_control_only.enabled());
        assert!(remote_control_only.remote_control_hook_enabled);
        assert!(!remote_control_only.plugin_unlock_enabled);
        assert!(!remote_control_only.session_sync_enabled);
        assert!(!disabled.enabled());
    }

    #[test]
    fn codex_app_open_relaunches_only_for_session_changes_or_missing_cdp_launch() {
        let session_only = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": false,
            "codex_session_sync_enabled": true
        }));
        let session_disabled = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": false,
            "codex_session_sync_enabled": false
        }));
        let plugin_only = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": true,
            "codex_session_sync_enabled": false
        }));
        let remote_control_only = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": false,
            "codex_remote_control_hook_enabled": true,
            "codex_session_sync_enabled": false
        }));
        let session_and_plugin = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": true,
            "codex_session_sync_enabled": true
        }));
        let plugin_hooks = CodexCdpLaunchHooks {
            plugin_unlock: true,
            remote_control_hook: false,
        };
        let remote_control_hooks = CodexCdpLaunchHooks {
            plugin_unlock: false,
            remote_control_hook: true,
        };

        assert_eq!(
            codex_relaunch_mode_for_app_open(
                session_only,
                CodexAppOpenStatus {
                    session_sync_updated: true,
                    cdp_launch_applied: false
                }
            ),
            Some(CodexRelaunchMode::Normal)
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(session_only, CodexAppOpenStatus::default()),
            None
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(plugin_only, CodexAppOpenStatus::default()),
            Some(CodexRelaunchMode::Cdp(plugin_hooks))
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(
                plugin_only,
                CodexAppOpenStatus {
                    session_sync_updated: false,
                    cdp_launch_applied: true
                }
            ),
            None
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(remote_control_only, CodexAppOpenStatus::default()),
            Some(CodexRelaunchMode::Cdp(remote_control_hooks))
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(
                session_and_plugin,
                CodexAppOpenStatus {
                    session_sync_updated: false,
                    cdp_launch_applied: true
                }
            ),
            None
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(
                session_and_plugin,
                CodexAppOpenStatus {
                    session_sync_updated: true,
                    cdp_launch_applied: true
                }
            ),
            Some(CodexRelaunchMode::Cdp(plugin_hooks))
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(
                session_disabled,
                CodexAppOpenStatus {
                    session_sync_updated: true,
                    cdp_launch_applied: false
                }
            ),
            None
        );
    }
}
