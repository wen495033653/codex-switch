use super::{
    codex_processes_have_cdp_launch, inject_codex_mobile_no_replace_hook, kill_process_tree,
    launch_codex_process_with_options, launch_codex_with_cdp_hooks, wait_for_pids_exit,
    CodexAppOpenOutcome, CodexCdpLaunchHooks, CodexProcess,
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
    session_manager::migrate_legacy_codex_data_for_current_home,
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
    remote_control_enabled: bool,
    session_sync_enabled: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CodexAppOpenStatus {
    session_sync_pending: bool,
    cdp_launch_applied: bool,
}

pub(crate) struct CodexAppInstanceLaunch {
    pub(crate) launched: bool,
}

impl CodexAppOpenActions {
    fn from_settings(settings: &Value) -> Self {
        Self {
            remote_control_enabled: remote_control_enabled_from_settings(settings),
            session_sync_enabled: settings
                .get("codex_session_sync_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        }
    }

    fn enabled(self) -> bool {
        self.remote_control_enabled || self.session_sync_enabled
    }
}

pub(crate) fn handle_codex_app_open(
    processes: &[CodexProcess],
) -> Result<CodexAppOpenOutcome, String> {
    trigger_legacy_codex_data_migration();
    let actions = codex_app_open_actions()?;
    log_session_sync_event(
        "codex_app_open_handler_start",
        json!({
            "processes": codex_processes_log_value(processes),
            "remoteControlEnabled": actions.remote_control_enabled,
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
        if status.cdp_launch_applied && actions.remote_control_enabled {
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

fn trigger_legacy_codex_data_migration() {
    thread::spawn(|| {
        for delay in [0, 2, 5] {
            if delay > 0 {
                thread::sleep(StdDuration::from_secs(delay));
            }
            match migrate_legacy_codex_data_for_current_home() {
                Ok(report) => {
                    let completed = report.get("completed").and_then(Value::as_bool) == Some(true);
                    log_session_sync_event("codex_desktop_data_migration", report);
                    if completed {
                        break;
                    }
                }
                Err(err) => {
                    log_session_sync_event(
                        "codex_desktop_data_migration_error",
                        json!({ "error": err }),
                    );
                    break;
                }
            }
        }
    });
}

fn codex_app_open_actions() -> Result<CodexAppOpenActions, String> {
    read_settings_value().map(|settings| CodexAppOpenActions::from_settings(&settings))
}

fn codex_relaunch_mode_for_app_open(
    actions: CodexAppOpenActions,
    status: CodexAppOpenStatus,
) -> Option<CodexRelaunchMode> {
    if actions.session_sync_enabled && status.session_sync_pending {
        return Some(CodexRelaunchMode::Cdp(
            codex_cdp_launch_hooks_for_watch_open(actions),
        ));
    }
    None
}

fn codex_cdp_launch_hooks_for_watch_open(actions: CodexAppOpenActions) -> CodexCdpLaunchHooks {
    CodexCdpLaunchHooks {
        codex_mobile_no_replace: actions.remote_control_enabled,
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
            "message": "未检测到正在运行的 Codex"
        }));
    }

    let actions = codex_app_open_actions()?;
    let session_sync_pending =
        session_sync_pending_for_relaunch(command, actions.session_sync_enabled, Some(command));
    let remote_control_runtime_pending =
        remote_control_runtime_pending_for_relaunch(command, Some(command));
    let restarted = relaunch_running_codex_processes(
        &processes,
        CodexRelaunchMode::Normal,
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
        "message": if restarted > 0 { "Codex 已重启" } else { "未能重新打开 Codex" },
        "restarted": restarted > 0,
        "restartedCount": restarted
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodexRelaunchOrigin {
    Watcher,
    AppCommand,
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
    let mode = CodexRelaunchMode::Normal;
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
        return Ok(CodexAppInstanceLaunch { launched: false });
    }
    let mode = CodexRelaunchMode::Normal;
    log_session_sync_event(
        "codex_app_instance_launch_start",
        json!({
            "executable": executable,
            "mode": format!("{mode:?}")
        }),
    );
    let launched = launch_codex_process_with_options(executable, args, envs)?;
    Ok(CodexAppInstanceLaunch { launched })
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
        return Err("未检测到 Codex 可执行路径".to_string());
    }
    for executable in &executables {
        if !Path::new(executable).is_file() {
            return Err(format!(
                "Codex 可执行文件不存在，未结束现有进程: {executable}"
            ));
        }
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
        kill_process_tree(*pid)?;
    }
    let alive = wait_for_pids_exit(&pids, 12_000);
    if !alive.is_empty() {
        return Err(format!(
            "Codex 进程未能在 12000ms 内退出，存活 PID: {alive:?}"
        ));
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

    if restarted == 0 {
        if origin == CodexRelaunchOrigin::AppCommand {
            super::codex_app_watcher::clear_expected_codex_app_open_for_executables(&executables);
        }
        return Err(format!("未能重新打开 Codex，可执行路径: {executables:?}"));
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
        CodexRelaunchMode::Normal => launch_codex_process_with_options(executable, &[], &[]),
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
    #[test]
    fn retired_plugin_flag_never_triggers_restart_or_cdp() {
        for enabled in [false, true] {
            let actions = CodexAppOpenActions::from_settings(&json!({
                "codex_plugins_enabled": enabled,
                "codex_remote_control_enabled": false,
                "codex_session_sync_enabled": false
            }));
            assert!(!actions.enabled());
            assert_eq!(
                codex_relaunch_mode_for_app_open(actions, CodexAppOpenStatus::default()),
                None
            );
        }
    }
    #[test]
    fn pending_session_sync_preserves_mobile_hook_without_plugin() {
        for remote in [false, true] {
            let actions = CodexAppOpenActions::from_settings(&json!({
                "codex_remote_control_enabled": remote,
                "codex_session_sync_enabled": true
            }));
            assert_eq!(
                codex_relaunch_mode_for_app_open(actions, CodexAppOpenStatus::default()),
                None
            );
            assert_eq!(
                codex_relaunch_mode_for_app_open(
                    actions,
                    CodexAppOpenStatus {
                        session_sync_pending: true,
                        ..CodexAppOpenStatus::default()
                    }
                ),
                Some(CodexRelaunchMode::Cdp(CodexCdpLaunchHooks {
                    codex_mobile_no_replace: remote
                }))
            );
        }
    }
}
