use super::remote_control::{
    preview_remote_control_runtime_for_current_settings,
    sync_remote_control_runtime_for_current_settings,
};
use super::{
    cdp::{
        codex_processes_have_cdp_launch, inject_codex_mobile_no_replace_hook,
        launch_codex_with_cdp_hooks, CodexCdpLaunchHooks,
    },
    codex_app_watcher::{
        codex_processes_log_value, disable_automatic_codex_app_open, CodexAppOpenOutcome,
        CodexProcess,
    },
    process_control::{
        kill_root_process_trees, launch_codex_process_with_options, root_pids, wait_for_pids_exit,
    },
};
use crate::{
    codex_sessions::{
        preview_codex_sessions_to_current_mode_now_from,
        sync_codex_sessions_to_current_mode_now_from,
    },
    session_manager::migrate_legacy_codex_data_for_current_home,
    session_sync_diagnostics::log_session_sync_event,
    settings::{read_settings_value, remote_control_enabled_from_settings},
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
    )?;
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

fn session_sync_pending_for_relaunch(
    trigger: &str,
    enabled: bool,
    command: Option<&str>,
) -> Result<bool, String> {
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
        return Ok(false);
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
            Ok(true)
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
            Ok(false)
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
            Err(err)
        }
    }
}

fn remote_control_runtime_pending_for_relaunch(
    trigger: &str,
    command: Option<&str>,
) -> Result<bool, String> {
    remote_control_runtime_pending_from_preview(
        preview_remote_control_runtime_for_current_settings(trigger),
        trigger,
        command,
    )
}

// Like the session sync preflight: a failed check is returned before Codex is closed.
fn remote_control_runtime_pending_from_preview(
    preview: Result<bool, String>,
    trigger: &str,
    command: Option<&str>,
) -> Result<bool, String> {
    match preview {
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
            Ok(true)
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
            Ok(false)
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
            Err(err)
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
        session_sync_pending_for_relaunch(command, actions.session_sync_enabled, Some(command))?;
    let remote_control_runtime_pending =
        remote_control_runtime_pending_for_relaunch(command, Some(command))?;
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
    let process_tree = super::codex_app_watcher::codex_process_tree(processes);
    let pids = process_tree.iter().map(|(pid, _)| *pid).collect::<Vec<_>>();
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
            "rootPids": root_pids(&process_tree),
            "executables": executables.clone()
        }),
    );

    close_then_sync_or_relaunch(
        origin,
        &pids,
        post_exit_session_sync,
        || {
            kill_root_process_trees(&process_tree)?;
            let alive = wait_for_pids_exit(&pids, 12_000);
            if !alive.is_empty() {
                return Err(format!(
                    "Codex 进程未能在 12000ms 内退出，存活 PID: {alive:?}"
                ));
            }
            Ok(())
        },
        sync_codex_sessions_to_current_mode_now_from,
        || {
            relaunch_codex_after_exit(
                &executables,
                mode,
                origin,
                post_exit_session_sync,
                post_exit_remote_control_runtime_sync,
            )
        },
    )
}

fn close_then_sync_or_relaunch(
    origin: CodexRelaunchOrigin,
    pids: &[u64],
    session_sync_pending: bool,
    close: impl FnOnce() -> Result<(), String>,
    sync: impl FnOnce(&str) -> Result<usize, String>,
    after_exit: impl FnOnce() -> Result<usize, String>,
) -> Result<usize, String> {
    let close_error = match close() {
        Ok(()) => return after_exit(),
        Err(error) => error,
    };
    disable_automatic_codex_app_open();
    let trigger = match origin {
        CodexRelaunchOrigin::Watcher => "codex_app_close_failed_watcher",
        CodexRelaunchOrigin::AppCommand => "codex_app_close_failed_app_command",
    };
    // A failed close permits one file sync, never post-exit config/CDP/launch operations.
    let sync_result = session_sync_pending.then(|| sync(trigger));
    let error = match &sync_result {
        Some(Ok(updated)) => format!(
            "{close_error}；已直接同步会话文件（更新 {updated} 项）；未重启，不再自动尝试关闭"
        ),
        Some(Err(sync_error)) => {
            format!("Codex 未关闭，直接同步会话文件失败：{sync_error}；未重启，不再自动尝试关闭")
        }
        None => format!("{close_error}；没有待同步的会话数据；未重启，不再自动尝试关闭"),
    };
    log_session_sync_event(
        "codex_app_relaunch_processes_close_error",
        json!({
            "origin": format!("{origin:?}"), "trigger": trigger, "pids": pids,
            "closeError": close_error, "error": error,
            "sessionSyncAttempted": session_sync_pending,
            "sessionSyncSucceeded": sync_result.as_ref().map(Result::is_ok),
            "updated": sync_result.as_ref().and_then(|result| result.as_ref().ok()),
            "sessionSyncError": sync_result.as_ref().and_then(|result| result.as_ref().err()),
            "restarted": false, "retry": false
        }),
    );
    // Even when files synced, closing failed: propagate the terminal result so the watcher
    // disables later attempts. A file write does not prove the running app reloaded its data.
    Err(error)
}

fn relaunch_codex_after_exit(
    executables: &[String],
    mode: CodexRelaunchMode,
    origin: CodexRelaunchOrigin,
    post_exit_session_sync: bool,
    post_exit_remote_control_runtime_sync: bool,
) -> Result<usize, String> {
    let restarted = reopen_after_post_exit_steps(
        || apply_codex_config_after_process_exit(origin, post_exit_remote_control_runtime_sync),
        || {
            if post_exit_session_sync {
                sync_codex_sessions_after_process_exit(origin)
            } else {
                Ok(())
            }
        },
        || relaunch_closed_codex(executables, mode, origin),
    )?;
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

// Codex is already closed here. A failed config apply or session sync still reopens it once;
// the failure is returned afterwards so the watcher records it and stops retrying.
fn reopen_after_post_exit_steps(
    apply_config: impl FnOnce() -> Result<(), String>,
    sync_sessions: impl FnOnce() -> Result<(), String>,
    relaunch: impl FnOnce() -> Result<usize, String>,
) -> Result<usize, String> {
    let config_result = apply_config();
    let session_sync_result = sync_sessions();
    let relaunch_result = relaunch();
    let errors = [
        relaunch_result.as_ref().err(),
        config_result.as_ref().err(),
        session_sync_result.as_ref().err(),
    ]
    .into_iter()
    .flatten()
    .cloned()
    .collect::<Vec<_>>();
    if errors.is_empty() {
        relaunch_result
    } else {
        Err(errors.join("；"))
    }
}

fn relaunch_closed_codex(
    executables: &[String],
    mode: CodexRelaunchMode,
    origin: CodexRelaunchOrigin,
) -> Result<usize, String> {
    thread::sleep(StdDuration::from_millis(RELAUNCH_DELAY_MS));

    if origin == CodexRelaunchOrigin::AppCommand {
        log_session_sync_event(
            "codex_app_relaunch_processes_expect_open",
            json!({
                "origin": format!("{origin:?}"),
                "executables": executables
            }),
        );
        super::codex_app_watcher::expect_app_command_codex_app_open_for_executables(executables);
    }

    let mut restarted = 0usize;
    for executable in executables {
        match relaunch_codex_executable(executable, mode) {
            Ok(true) => restarted += 1,
            Ok(false) => {}
            Err(err) => {
                if origin == CodexRelaunchOrigin::AppCommand {
                    super::codex_app_watcher::clear_expected_codex_app_open_for_executables(
                        executables,
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
            super::codex_app_watcher::clear_expected_codex_app_open_for_executables(executables);
        }
        return Err(format!("未能重新打开 Codex，可执行路径: {executables:?}"));
    }
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
        sync_remote_control_runtime_for_post_exit(context)?
    } else {
        json!({ "changed": false, "skipped": true })
    };

    Ok(json!({
        "remoteControl": remote_control_changed
    }))
}

fn sync_remote_control_runtime_for_post_exit(context: &str) -> Result<Value, String> {
    match sync_remote_control_runtime_for_current_settings(context) {
        Ok(changed) => Ok(json!({ "changed": changed })),
        Err(err) => {
            log_session_sync_event(
                "codex_app_relaunch_processes_post_exit_remote_control_runtime_error",
                json!({
                    "context": context,
                    "error": err
                }),
            );
            Err(err)
        }
    }
}

fn sync_codex_sessions_after_process_exit(origin: CodexRelaunchOrigin) -> Result<(), String> {
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
    let result = sync_codex_sessions_to_current_mode_now_from(trigger);
    match &result {
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
    result.map(|_| ())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn close_failure_log(pid: u64) -> Value {
        crate::session_sync_diagnostics::get_dev_log_entries()
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|entry| {
                entry["details"]["event"] == "codex_app_relaunch_processes_close_error"
                    && entry["details"]["details"]["pids"] == json!([pid])
            })
            .unwrap()
            .clone()
    }

    #[test]
    fn close_failure_syncs_once_without_launch_for_both_origins() {
        let failures = [
            "[PROCESS_ELEVATION_MISMATCH] callerElevated=false, targetElevated=true",
            "taskkill exitCode=128; access denied",
            "Codex 进程未能在 12000ms 内退出，存活 PID: [42]",
        ];
        for (i, origin) in [
            CodexRelaunchOrigin::Watcher,
            CodexRelaunchOrigin::AppCommand,
        ]
        .into_iter()
        .enumerate()
        {
            for (j, failure) in failures.iter().enumerate() {
                let pid = 910_000 + (i * failures.len() + j) as u64;
                let calls = RefCell::new(Vec::new());
                let error = close_then_sync_or_relaunch(
                    origin,
                    &[pid],
                    true,
                    || {
                        calls.borrow_mut().push("close");
                        Err((*failure).into())
                    },
                    |trigger| {
                        calls.borrow_mut().push("sync");
                        assert_eq!(
                            trigger,
                            if i == 0 {
                                "codex_app_close_failed_watcher"
                            } else {
                                "codex_app_close_failed_app_command"
                            }
                        );
                        Ok(j)
                    },
                    || panic!("must not configure, inject CDP or launch after close failure"),
                )
                .unwrap_err();
                assert_eq!(*calls.borrow(), ["close", "sync"]);
                assert!(error.contains(failure));
                assert!(error.contains(&format!("更新 {j} 项")));
                let entry = close_failure_log(pid);
                assert_eq!(entry["level"], if j == 0 { "warn" } else { "error" });
                let details = &entry["details"]["details"];
                assert_eq!(details["closeError"], *failure);
                assert_eq!(details["error"], error);
                assert_eq!(details["updated"], j);
                assert_eq!(details["sessionSyncAttempted"], true);
                assert_eq!(details["sessionSyncSucceeded"], true);
                assert!(details["sessionSyncError"].is_null());
                assert_eq!(details["restarted"], false);
                assert_eq!(details["retry"], false);
            }
        }
    }

    #[test]
    fn close_failure_with_sync_error_retains_both_causes_and_is_error_not_warn() {
        let close_error = "[PROCESS_ELEVATION_MISMATCH] fixture permission mismatch";
        let sync_error = "fixture SQLite error: database is locked (code 5)";
        let error = close_then_sync_or_relaunch(
            CodexRelaunchOrigin::Watcher,
            &[910_010],
            true,
            || Err(close_error.into()),
            |_| Err(sync_error.into()),
            || panic!("must not launch after sync failure"),
        )
        .unwrap_err();
        assert!(error.contains(sync_error));
        assert!(!error.contains(crate::session_sync_diagnostics::PROCESS_ELEVATION_WARNING));
        let entry = close_failure_log(910_010);
        assert_eq!(entry["level"], "error");
        let details = &entry["details"]["details"];
        assert_eq!(details["closeError"], close_error);
        assert_eq!(details["sessionSyncError"], sync_error);
        assert_eq!(details["sessionSyncSucceeded"], false);
        assert!(details["updated"].is_null());
        assert_eq!(details["retry"], false);
    }

    #[test]
    fn close_failure_without_pending_sync_does_not_write_or_launch() {
        let error = close_then_sync_or_relaunch(
            CodexRelaunchOrigin::AppCommand,
            &[910_011],
            false,
            || Err("fixture close failed".into()),
            |_| panic!("disabled or unchanged sessions must not be written"),
            || panic!("must not launch"),
        )
        .unwrap_err();
        assert!(error.contains("没有待同步"));
        let entry = close_failure_log(910_011);
        assert_eq!(entry["details"]["details"]["sessionSyncAttempted"], false);
        assert!(entry["details"]["details"]["sessionSyncSucceeded"].is_null());
    }

    #[test]
    fn successful_close_keeps_post_exit_flow_and_propagates_its_result() {
        for result in [Ok(1), Err("fixture post-exit error".to_string())] {
            let calls = RefCell::new(Vec::new());
            let actual = close_then_sync_or_relaunch(
                CodexRelaunchOrigin::Watcher,
                &[910_012],
                true,
                || {
                    calls.borrow_mut().push("close");
                    Ok(())
                },
                |_| panic!("must not perform running-file sync when close succeeded"),
                || {
                    calls.borrow_mut().push("after_exit");
                    result.clone()
                },
            );
            assert_eq!(actual, result);
            assert_eq!(*calls.borrow(), ["close", "after_exit"]);
        }
    }

    #[test]
    #[ignore = "requires an explicit isolated Codex fixture home; never uses the real home"]
    fn isolated_close_failure_syncs_real_files_without_relaunch() {
        let fixture = std::env::var_os("CODEX_SWITCH_CLOSE_FAILURE_TEST_HOME")
            .map(std::path::PathBuf::from)
            .expect("set CODEX_SWITCH_CLOSE_FAILURE_TEST_HOME to the prepared sandbox .codex");
        assert_eq!(crate::paths::codex_dir().unwrap(), fixture);
        let error = close_then_sync_or_relaunch(
            CodexRelaunchOrigin::Watcher,
            &[910_013],
            true,
            || {
                Err(
                    "[PROCESS_ELEVATION_MISMATCH] isolated fixture, no termination attempted"
                        .into(),
                )
            },
            sync_codex_sessions_to_current_mode_now_from,
            || panic!("must not launch"),
        )
        .unwrap_err();
        assert!(error.contains("已直接同步会话文件"), "{error}");
        let connection = rusqlite::Connection::open(fixture.join("state_5.sqlite")).unwrap();
        let provider: String = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id='close-failure-fixture'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(provider, "openai");
        let rollout =
            std::fs::read_to_string(fixture.join("sessions/rollout-fixture.jsonl")).unwrap();
        let first: Value = serde_json::from_str(rollout.lines().next().unwrap()).unwrap();
        assert_eq!(first["payload"]["model_provider"], "openai");
        println!("{}", close_failure_log(910_013));

        // An open Codex database may hold a writer lock. Exercise the real SQLite error,
        // while the same failed-close branch must still never reach the launch callback.
        connection
            .execute("UPDATE threads SET model_provider='api'", [])
            .unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        let error = close_then_sync_or_relaunch(
            CodexRelaunchOrigin::Watcher,
            &[910_014],
            true,
            || Err("[PROCESS_ELEVATION_MISMATCH] isolated busy fixture".into()),
            sync_codex_sessions_to_current_mode_now_from,
            || panic!("must not launch after a locked database error"),
        )
        .unwrap_err();
        connection.execute_batch("ROLLBACK").unwrap();
        assert!(error.contains("database is locked"), "{error}");
        let entry = close_failure_log(910_014);
        assert_eq!(entry["level"], "error");
        assert_eq!(entry["details"]["details"]["sessionSyncSucceeded"], false);
        assert_eq!(entry["details"]["details"]["restarted"], false);
        assert_eq!(entry["details"]["details"]["retry"], false);
        println!("{entry}");
    }

    #[test]
    fn post_exit_failures_still_reopen_once_then_return_every_cause() {
        let config_error = "fixture remote control runtime: config.toml locked";
        let sync_error = "fixture SQLite error: database is locked (code 5)";
        let launch_error = "fixture launch: early exit Some(1)";
        for (config, sync, launch, expected) in [
            (Ok(()), Ok(()), Ok(1), Ok(1)),
            (Err(config_error), Ok(()), Ok(1), Err(vec![config_error])),
            (Ok(()), Err(sync_error), Ok(1), Err(vec![sync_error])),
            (
                Err(config_error),
                Err(sync_error),
                Ok(1),
                Err(vec![config_error, sync_error]),
            ),
            (
                Err(config_error),
                Ok(()),
                Err(launch_error),
                Err(vec![launch_error, config_error]),
            ),
        ] {
            let calls = RefCell::new(Vec::new());
            let result = reopen_after_post_exit_steps(
                || {
                    calls.borrow_mut().push("config");
                    config.map_err(str::to_string)
                },
                || {
                    calls.borrow_mut().push("sync");
                    sync.map_err(str::to_string)
                },
                || {
                    calls.borrow_mut().push("launch");
                    launch.map_err(str::to_string)
                },
            );
            assert_eq!(*calls.borrow(), ["config", "sync", "launch"]);
            match expected {
                Ok(restarted) => assert_eq!(result, Ok(restarted)),
                Err(causes) => assert_eq!(result.unwrap_err(), causes.join("；")),
            }
        }
    }

    #[test]
    fn remote_control_preflight_error_is_returned_before_closing() {
        let command = "restart_current_codex_app_normal";
        assert_eq!(
            remote_control_runtime_pending_from_preview(Ok(true), command, Some(command)),
            Ok(true)
        );
        assert_eq!(
            remote_control_runtime_pending_from_preview(Ok(false), command, Some(command)),
            Ok(false)
        );
        let error = "fixture preflight: settings.json invalid json at line 3";
        assert_eq!(
            remote_control_runtime_pending_from_preview(Err(error.into()), command, Some(command)),
            Err(error.to_string())
        );
        let logs = crate::session_sync_diagnostics::get_dev_log_entries();
        assert!(
            logs.as_array().unwrap().iter().any(|entry| {
                entry["details"]["event"]
                    == "codex_app_restart_command_remote_control_runtime_error"
                    && entry["details"]["details"]["错误"] == error
            }),
            "{logs}"
        );
    }

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
