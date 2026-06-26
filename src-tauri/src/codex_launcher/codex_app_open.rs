use super::{
    codex_processes_have_cdp_launch, inject_codex_cdp_hooks, inject_codex_mobile_no_replace_hook,
    kill_process_tree, launch_codex_with_cdp_hooks,
    launch_codex_with_optional_cdp_hooks_with_options, launch_executable_with_options,
    relaunch_executable_with_retry, wait_for_pids_exit, CodexAppOpenOutcome, CodexCdpLaunchHooks,
    CodexProcess,
};
use crate::{
    codex_launcher::{
        preview_remote_control_runtime_for_current_settings, remote_control_enabled_from_settings,
        sync_remote_control_runtime_for_current_settings,
    },
    codex_sessions::{
        preview_codex_sessions_to_current_mode_now_from,
        sync_codex_sessions_to_current_mode_now_from,
    },
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
    remote_control_enabled: bool,
    delete_button_enabled: bool,
    session_sync_enabled: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CodexAppOpenStatus {
    session_sync_pending: bool,
    cdp_launch_applied: bool,
}

pub(crate) struct CodexAppInstanceLaunch {
    pub(crate) launched: bool,
    pub(crate) hook_warning: Option<String>,
}

impl CodexAppOpenActions {
    fn from_settings(settings: &Value) -> Self {
        Self {
            plugin_unlock_enabled: bool_field(settings, "codex_plugins_enabled"),
            remote_control_enabled: remote_control_enabled_from_settings(settings),
            delete_button_enabled: bool_field(settings, "codex_delete_button_enabled"),
            session_sync_enabled: settings
                .get("codex_session_sync_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        }
    }

    fn enabled(self) -> bool {
        self.plugin_unlock_enabled
            || self.remote_control_enabled
            || self.delete_button_enabled
            || self.session_sync_enabled
    }

    fn cdp_launch_enabled(self) -> bool {
        self.plugin_unlock_enabled || self.delete_button_enabled
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
            "remoteControlEnabled": actions.remote_control_enabled,
            "deleteButtonEnabled": actions.delete_button_enabled,
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

    let session_sync_pending = session_sync_pending_for_relaunch(
        "codex_app_open_handler",
        actions.session_sync_enabled,
        None,
    );
    sync_remote_control_runtime_for_open_if_pending("codex_app_open_handler");

    let status = CodexAppOpenStatus {
        session_sync_pending,
        cdp_launch_applied: codex_processes_have_cdp_launch(processes),
    };
    let Some(relaunch_mode) = codex_relaunch_mode_for_app_open(actions, status) else {
        if status.cdp_launch_applied {
            if let Some(hooks) = codex_cdp_launch_hooks_for_actions(actions) {
                match inject_codex_cdp_hooks(processes, hooks) {
                    Ok(injected) => log_session_sync_event(
                        "codex_app_open_handler_cdp_hook_injected",
                        json!({
                            "hook": "codex_cdp_hooks",
                            "pluginUnlock": hooks.plugin_unlock,
                            "codexMobileNoReplace": hooks.codex_mobile_no_replace,
                            "deleteButton": hooks.delete_button,
                            "injectedCount": injected
                        }),
                    ),
                    Err(err) => log_session_sync_event(
                        "codex_app_open_handler_cdp_hook_error",
                        json!({
                            "hook": "codex_cdp_hooks",
                            "pluginUnlock": hooks.plugin_unlock,
                            "codexMobileNoReplace": hooks.codex_mobile_no_replace,
                            "deleteButton": hooks.delete_button,
                            "error": err
                        }),
                    ),
                }
            } else if actions.remote_control_enabled {
                match inject_codex_mobile_no_replace_hook(processes) {
                    Ok(injected) => log_session_sync_event(
                        "codex_app_open_handler_cdp_hook_injected",
                        json!({ "hook": "codex_mobile_no_replace", "injectedCount": injected }),
                    ),
                    Err(err) => log_session_sync_event(
                        "codex_app_open_handler_cdp_hook_error",
                        json!({ "hook": "codex_mobile_no_replace", "error": err }),
                    ),
                }
            }
        }
        log_session_sync_event(
            "codex_app_open_handler_finish",
            json!({ "relaunchExpected": false }),
        );
        return Ok(CodexAppOpenOutcome::default());
    };

    let restarted = relaunch_running_codex_processes(
        processes,
        relaunch_mode,
        CodexRelaunchOrigin::Watcher,
        session_sync_pending,
        false,
    )?;
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
    if actions.session_sync_enabled && status.session_sync_pending {
        return Some(CodexRelaunchMode::Cdp(
            codex_cdp_launch_hooks_for_watch_open(actions),
        ));
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

fn codex_cdp_launch_hooks_for_watch_open(actions: CodexAppOpenActions) -> CodexCdpLaunchHooks {
    CodexCdpLaunchHooks {
        plugin_unlock: actions.plugin_unlock_enabled,
        codex_mobile_no_replace: actions.remote_control_enabled,
        delete_button: actions.delete_button_enabled,
    }
}

fn codex_cdp_launch_hooks_for_actions(actions: CodexAppOpenActions) -> Option<CodexCdpLaunchHooks> {
    if actions.cdp_launch_enabled() {
        Some(CodexCdpLaunchHooks {
            plugin_unlock: actions.plugin_unlock_enabled,
            codex_mobile_no_replace: actions.remote_control_enabled,
            delete_button: actions.delete_button_enabled,
        })
    } else {
        None
    }
}

fn session_sync_pending_for_relaunch(trigger: &str, enabled: bool, command: Option<&str>) -> bool {
    if !enabled {
        let event = if command.is_some() {
            "codex_app_restart_command_session_sync_skip"
        } else {
            "codex_app_open_handler_session_sync_skip"
        };
        let details = if let Some(command) = command {
            json!({ "command": command, "reason": "setting_disabled" })
        } else {
            json!({ "reason": "setting_disabled" })
        };
        log_session_sync_event(event, details);
        return false;
    }

    match preview_codex_sessions_to_current_mode_now_from(trigger) {
        Ok(updated) if updated > 0 => {
            let event = if command.is_some() {
                "codex_app_restart_command_session_sync_deferred"
            } else {
                "codex_app_open_handler_session_sync_deferred"
            };
            let details = if let Some(command) = command {
                json!({
                    "command": command,
                    "reason": "sync_after_process_exit",
                    "updated": updated
                })
            } else {
                json!({
                    "reason": "sync_after_process_exit",
                    "updated": updated
                })
            };
            log_session_sync_event(event, details);
            true
        }
        Ok(updated) => {
            let event = if command.is_some() {
                "codex_app_restart_command_session_sync_skip"
            } else {
                "codex_app_open_handler_session_sync_skip"
            };
            let details = if let Some(command) = command {
                json!({
                    "command": command,
                    "reason": "no_session_changes",
                    "updated": updated
                })
            } else {
                json!({
                    "reason": "no_session_changes",
                    "updated": updated
                })
            };
            log_session_sync_event(event, details);
            false
        }
        Err(err) => {
            let event = if command.is_some() {
                "codex_app_restart_command_session_sync_error"
            } else {
                "codex_app_open_handler_session_sync_preflight_error"
            };
            let details = if let Some(command) = command {
                json!({
                    "command": command,
                    "error": err
                })
            } else {
                json!({
                    "trigger": trigger,
                    "error": err
                })
            };
            log_session_sync_event(event, details);
            true
        }
    }
}

fn remote_control_runtime_pending_for_relaunch(trigger: &str, command: Option<&str>) -> bool {
    match preview_remote_control_runtime_for_current_settings(trigger) {
        Ok(true) => {
            let event = if command.is_some() {
                "codex_app_restart_command_remote_control_runtime_deferred"
            } else {
                "codex_app_open_handler_remote_control_runtime_deferred"
            };
            let details = if let Some(command) = command {
                json!({
                    "command": command,
                    "reason": "sync_after_process_exit"
                })
            } else {
                json!({
                    "reason": "sync_after_process_exit"
                })
            };
            log_session_sync_event(event, details);
            true
        }
        Ok(false) => {
            let event = if command.is_some() {
                "codex_app_restart_command_remote_control_runtime_skip"
            } else {
                "codex_app_open_handler_remote_control_runtime_skip"
            };
            let details = if let Some(command) = command {
                json!({
                    "command": command,
                    "reason": "runtime_current"
                })
            } else {
                json!({
                    "reason": "runtime_current"
                })
            };
            log_session_sync_event(event, details);
            false
        }
        Err(err) => {
            let event = if command.is_some() {
                "codex_app_restart_command_remote_control_runtime_error"
            } else {
                "codex_app_open_handler_remote_control_runtime_error"
            };
            let details = if let Some(command) = command {
                json!({
                    "command": command,
                    "error": err
                })
            } else {
                json!({
                    "trigger": trigger,
                    "error": err
                })
            };
            log_session_sync_event(event, details);
            false
        }
    }
}

fn sync_remote_control_runtime_for_open_if_pending(trigger: &str) {
    match preview_remote_control_runtime_for_current_settings(trigger) {
        Ok(true) => match sync_remote_control_runtime_for_current_settings(trigger) {
            Ok(changed) => log_session_sync_event(
                "codex_app_open_handler_remote_control_runtime_applied",
                json!({
                    "reason": "runtime_pending",
                    "changed": changed
                }),
            ),
            Err(err) => log_session_sync_event(
                "codex_app_open_handler_remote_control_runtime_error",
                json!({
                    "trigger": trigger,
                    "error": err
                }),
            ),
        },
        Ok(false) => log_session_sync_event(
            "codex_app_open_handler_remote_control_runtime_skip",
            json!({ "reason": "runtime_current" }),
        ),
        Err(err) => log_session_sync_event(
            "codex_app_open_handler_remote_control_runtime_error",
            json!({
                "trigger": trigger,
                "error": err
            }),
        ),
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
    let session_sync_pending =
        session_sync_pending_for_relaunch(command, actions.session_sync_enabled, Some(command));
    let remote_control_runtime_pending =
        remote_control_runtime_pending_for_relaunch(command, Some(command));
    let restarted = relaunch_running_codex_processes(
        &processes,
        codex_relaunch_mode_for_actions(actions),
        CodexRelaunchOrigin::AppCommand,
        session_sync_pending,
        remote_control_runtime_pending,
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
    let session_sync_pending =
        session_sync_pending_for_relaunch(command, actions.session_sync_enabled, Some(command));
    let remote_control_runtime_pending =
        remote_control_runtime_pending_for_relaunch(command, Some(command));
    let restarted = relaunch_running_codex_processes(
        &processes,
        codex_relaunch_mode_for_actions(actions),
        CodexRelaunchOrigin::AppCommand,
        session_sync_pending,
        remote_control_runtime_pending,
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

pub(crate) fn launch_codex_app_instance_for_current_settings_with_options(
    executable: &str,
    args: &[String],
    envs: &[(String, String)],
) -> Result<CodexAppInstanceLaunch, String> {
    let path = Path::new(executable);
    if !path.exists() {
        log_session_sync_event(
            "codex_app_instance_launch_skip",
            json!({
                "executable": executable,
                "reason": "missing_executable"
            }),
        );
        return Ok(CodexAppInstanceLaunch {
            launched: false,
            hook_warning: None,
        });
    }
    let mode = codex_relaunch_mode_for_current_settings()?;
    log_session_sync_event(
        "codex_app_instance_launch_start",
        json!({
            "executable": executable,
            "mode": format!("{mode:?}")
        }),
    );
    match mode {
        CodexRelaunchMode::Cdp(hooks) => {
            let hook_warning =
                launch_codex_with_optional_cdp_hooks_with_options(path, hooks, args, envs)?;
            if let Some(error) = &hook_warning {
                log_session_sync_event(
                    "codex_app_instance_launch_cdp_hook_warning",
                    json!({
                        "executable": executable,
                        "mode": format!("{mode:?}"),
                        "error": error
                    }),
                );
            }
            Ok(CodexAppInstanceLaunch {
                launched: true,
                hook_warning,
            })
        }
        CodexRelaunchMode::Normal => {
            let launched = launch_executable_with_options(executable, args, envs)?;
            Ok(CodexAppInstanceLaunch {
                launched,
                hook_warning: None,
            })
        }
    }
}

fn relaunch_running_codex_processes(
    processes: &[CodexProcess],
    mode: CodexRelaunchMode,
    origin: CodexRelaunchOrigin,
    post_exit_session_sync: bool,
    post_exit_remote_control_runtime_sync: bool,
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

    apply_codex_config_after_process_exit(origin, post_exit_remote_control_runtime_sync)?;

    if post_exit_session_sync {
        sync_codex_sessions_after_process_exit(origin);
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
        super::codex_app_watcher::expect_app_command_codex_app_open_for_executables(&executables);
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

fn apply_codex_config_after_process_exit(
    origin: CodexRelaunchOrigin,
    sync_remote_control_runtime: bool,
) -> Result<(), String> {
    let context = match origin {
        CodexRelaunchOrigin::Watcher => "codex_app_relaunch_after_exit_watcher",
        CodexRelaunchOrigin::AppCommand => "codex_app_relaunch_after_exit_app_command",
    };
    log_session_sync_event(
        "codex_app_relaunch_processes_post_exit_config_apply_start",
        json!({
            "origin": format!("{origin:?}"),
            "context": context
        }),
    );
    match apply_codex_config_for_current_settings(context, sync_remote_control_runtime) {
        Ok(details) => {
            log_session_sync_event(
                "codex_app_relaunch_processes_post_exit_config_apply_finish",
                json!({
                    "origin": format!("{origin:?}"),
                    "context": context,
                    "details": details
                }),
            );
            Ok(())
        }
        Err(err) => {
            log_session_sync_event(
                "codex_app_relaunch_processes_post_exit_config_apply_error",
                json!({
                    "origin": format!("{origin:?}"),
                    "context": context,
                    "error": err.clone()
                }),
            );
            Err(err)
        }
    }
}

fn apply_codex_config_for_current_settings(
    context: &str,
    sync_remote_control_runtime: bool,
) -> Result<Value, String> {
    let remote_control_changed = if sync_remote_control_runtime {
        sync_remote_control_runtime_for_post_exit(context)
    } else {
        json!({ "changed": false, "skipped": true })
    };

    Ok(json!({
        "remoteControl": remote_control_changed
    }))
}

fn sync_remote_control_runtime_for_post_exit(context: &str) -> Value {
    match sync_remote_control_runtime_for_current_settings(context) {
        Ok(changed) => json!({ "changed": changed }),
        Err(err) => {
            let error = err.clone();
            log_session_sync_event(
                "codex_app_relaunch_processes_post_exit_remote_control_runtime_error",
                json!({
                    "context": context,
                    "error": error
                }),
            );
            json!({ "changed": false, "error": err })
        }
    }
}

fn sync_codex_sessions_after_process_exit(origin: CodexRelaunchOrigin) {
    let trigger = match origin {
        CodexRelaunchOrigin::Watcher => "codex_app_relaunch_after_exit_watcher",
        CodexRelaunchOrigin::AppCommand => "codex_app_relaunch_after_exit_app_command",
    };
    log_session_sync_event(
        "codex_app_relaunch_processes_post_exit_session_sync_start",
        json!({
            "origin": format!("{origin:?}"),
            "trigger": trigger
        }),
    );
    match sync_codex_sessions_to_current_mode_now_from(trigger) {
        Ok(updated) => log_session_sync_event(
            "codex_app_relaunch_processes_post_exit_session_sync_finish",
            json!({
                "origin": format!("{origin:?}"),
                "trigger": trigger,
                "updated": updated
            }),
        ),
        Err(err) => log_session_sync_event(
            "codex_app_relaunch_processes_post_exit_session_sync_error",
            json!({
                "origin": format!("{origin:?}"),
                "trigger": trigger,
                "error": err
            }),
        ),
    }
}

fn relaunch_codex_executable(executable: &str, mode: CodexRelaunchMode) -> Result<bool, String> {
    match mode {
        CodexRelaunchMode::Cdp(hooks) => {
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
            "codex_remote_control_enabled": false,
            "codex_delete_button_enabled": false,
            "codex_session_sync_enabled": true
        }));
        let plugin_only = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": true,
            "codex_remote_control_enabled": false,
            "codex_delete_button_enabled": false,
            "codex_session_sync_enabled": false
        }));
        let remote_control_only = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": false,
            "codex_remote_control_enabled": true,
            "codex_delete_button_enabled": false,
            "codex_session_sync_enabled": false
        }));
        let delete_button_only = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": false,
            "codex_remote_control_enabled": false,
            "codex_delete_button_enabled": true,
            "codex_session_sync_enabled": false
        }));
        let disabled = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": false,
            "codex_remote_control_enabled": false,
            "codex_delete_button_enabled": false,
            "codex_session_sync_enabled": false
        }));

        assert!(session_only.enabled());
        assert!(session_only.session_sync_enabled);
        assert!(!session_only.plugin_unlock_enabled);
        assert!(plugin_only.enabled());
        assert!(plugin_only.plugin_unlock_enabled);
        assert!(plugin_only.cdp_launch_enabled());
        assert!(!plugin_only.session_sync_enabled);
        assert!(remote_control_only.enabled());
        assert!(remote_control_only.remote_control_enabled);
        assert!(!remote_control_only.plugin_unlock_enabled);
        assert!(!remote_control_only.delete_button_enabled);
        assert!(!remote_control_only.session_sync_enabled);
        assert!(delete_button_only.enabled());
        assert!(delete_button_only.delete_button_enabled);
        assert!(delete_button_only.cdp_launch_enabled());
        assert!(!delete_button_only.session_sync_enabled);
        assert!(!disabled.enabled());
    }

    #[test]
    fn codex_app_open_relaunches_only_for_pending_work_or_missing_cdp_launch() {
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
            "codex_remote_control_enabled": true,
            "codex_session_sync_enabled": false
        }));
        let delete_button_only = CodexAppOpenActions::from_settings(&json!({
            "codex_delete_button_enabled": true,
            "codex_session_sync_enabled": false
        }));
        let session_and_remote_control = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": false,
            "codex_remote_control_enabled": true,
            "codex_session_sync_enabled": true
        }));
        let session_and_plugin = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": true,
            "codex_session_sync_enabled": true
        }));
        let plugin_and_remote_control = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": true,
            "codex_remote_control_enabled": true,
            "codex_session_sync_enabled": false
        }));
        let plugin_hooks = CodexCdpLaunchHooks {
            plugin_unlock: true,
            codex_mobile_no_replace: false,
            delete_button: false,
        };
        let cdp_only_hooks = CodexCdpLaunchHooks {
            plugin_unlock: false,
            codex_mobile_no_replace: false,
            delete_button: false,
        };
        let mobile_no_replace_hooks = CodexCdpLaunchHooks {
            plugin_unlock: false,
            codex_mobile_no_replace: true,
            delete_button: false,
        };
        let delete_button_hooks = CodexCdpLaunchHooks {
            plugin_unlock: false,
            codex_mobile_no_replace: false,
            delete_button: true,
        };
        let plugin_remote_control_hooks = CodexCdpLaunchHooks {
            plugin_unlock: true,
            codex_mobile_no_replace: true,
            delete_button: false,
        };

        assert_eq!(
            codex_relaunch_mode_for_app_open(
                session_only,
                CodexAppOpenStatus {
                    session_sync_pending: true,
                    ..CodexAppOpenStatus::default()
                }
            ),
            Some(CodexRelaunchMode::Cdp(cdp_only_hooks))
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
                    cdp_launch_applied: true,
                    ..CodexAppOpenStatus::default()
                }
            ),
            None
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(remote_control_only, CodexAppOpenStatus::default()),
            None
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(delete_button_only, CodexAppOpenStatus::default()),
            Some(CodexRelaunchMode::Cdp(delete_button_hooks))
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(
                remote_control_only,
                CodexAppOpenStatus {
                    cdp_launch_applied: true,
                    ..CodexAppOpenStatus::default()
                }
            ),
            None
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(
                session_and_remote_control,
                CodexAppOpenStatus {
                    session_sync_pending: true,
                    ..CodexAppOpenStatus::default()
                }
            ),
            Some(CodexRelaunchMode::Cdp(mobile_no_replace_hooks))
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(
                plugin_and_remote_control,
                CodexAppOpenStatus::default()
            ),
            Some(CodexRelaunchMode::Cdp(plugin_remote_control_hooks))
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(
                session_and_plugin,
                CodexAppOpenStatus {
                    cdp_launch_applied: true,
                    ..CodexAppOpenStatus::default()
                }
            ),
            None
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(
                session_and_plugin,
                CodexAppOpenStatus {
                    session_sync_pending: true,
                    cdp_launch_applied: true,
                    ..CodexAppOpenStatus::default()
                }
            ),
            Some(CodexRelaunchMode::Cdp(plugin_hooks))
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(
                session_disabled,
                CodexAppOpenStatus {
                    session_sync_pending: true,
                    ..CodexAppOpenStatus::default()
                }
            ),
            None
        );
    }
}
