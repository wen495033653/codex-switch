use super::{
    codex_processes_have_plugin_unlock, kill_process_tree, launch_codex_with_plugins,
    relaunch_executable_with_retry, wait_for_pids_exit, CodexAppOpenOutcome, CodexProcess,
};
use crate::{
    codex_sessions::sync_codex_sessions_to_current_mode_now, json_util::bool_field,
    settings::read_settings_value,
};
use serde_json::{json, Value};
use std::{path::Path, thread, time::Duration as StdDuration};

const RELAUNCH_DELAY_MS: u64 = 1_500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodexRelaunchMode {
    Normal,
    Plugin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CodexAppOpenActions {
    plugin_unlock_enabled: bool,
    session_sync_enabled: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CodexAppOpenStatus {
    session_sync_updated: bool,
    plugin_unlock_applied: bool,
}

impl CodexAppOpenActions {
    fn from_settings(settings: &Value) -> Self {
        Self {
            plugin_unlock_enabled: bool_field(settings, "codex_plugins_enabled"),
            session_sync_enabled: settings
                .get("codex_session_sync_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        }
    }

    fn enabled(self) -> bool {
        self.plugin_unlock_enabled || self.session_sync_enabled
    }
}

pub(crate) fn handle_codex_app_open(
    processes: &[CodexProcess],
) -> Result<CodexAppOpenOutcome, String> {
    let actions = codex_app_open_actions()?;
    if !actions.enabled() {
        return Ok(CodexAppOpenOutcome::default());
    }

    let mut session_sync_updated = false;
    if actions.session_sync_enabled {
        match sync_codex_sessions_to_current_mode_now() {
            Ok(updated) => session_sync_updated = updated > 0,
            Err(err) => eprintln!("Codex app 会话同步失败: {err}"),
        }
    }

    let status = CodexAppOpenStatus {
        session_sync_updated,
        plugin_unlock_applied: actions.plugin_unlock_enabled
            && codex_processes_have_plugin_unlock(processes),
    };
    let Some(relaunch_mode) = codex_relaunch_mode_for_app_open(actions, status) else {
        return Ok(CodexAppOpenOutcome::default());
    };

    let restarted =
        relaunch_running_codex_processes(processes, relaunch_mode, CodexRelaunchOrigin::Watcher)?;
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
    if actions.session_sync_enabled && status.session_sync_updated {
        return Some(if actions.plugin_unlock_enabled {
            CodexRelaunchMode::Plugin
        } else {
            CodexRelaunchMode::Normal
        });
    }
    if actions.plugin_unlock_enabled && !status.plugin_unlock_applied {
        return Some(CodexRelaunchMode::Plugin);
    }
    None
}

pub(crate) fn restart_current_codex_app_for_plugin_setting() -> Result<Value, String> {
    let processes = super::codex_app_watcher::refresh_current_codex_app_processes()?;
    if processes.is_empty() {
        return Ok(json!({
            "ok": true,
            "message": "未检测到正在运行的 Codex app"
        }));
    }

    let restarted = relaunch_running_codex_processes(
        &processes,
        codex_relaunch_mode_for_current_settings()?,
        CodexRelaunchOrigin::AppCommand,
    )?;
    Ok(json!({
        "ok": true,
        "message": if restarted > 0 { "Codex app 已重启" } else { "未能重新打开 Codex app" },
        "restarted": restarted > 0,
        "restartedCount": restarted
    }))
}

pub(crate) fn restart_current_codex_app_normal() -> Result<Value, String> {
    let processes = super::codex_app_watcher::refresh_current_codex_app_processes()?;
    if processes.is_empty() {
        return Ok(json!({
            "ok": true,
            "message": "未检测到正在运行的 Codex app"
        }));
    }

    let restarted = relaunch_running_codex_processes(
        &processes,
        codex_relaunch_mode_for_current_settings()?,
        CodexRelaunchOrigin::AppCommand,
    )?;
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
    Ok(if actions.plugin_unlock_enabled {
        CodexRelaunchMode::Plugin
    } else {
        CodexRelaunchMode::Normal
    })
}

pub(crate) fn relaunch_codex_executable_for_current_settings(
    executable: &str,
) -> Result<bool, String> {
    let path = Path::new(executable);
    if !path.exists() {
        return Ok(false);
    }
    let mode = codex_relaunch_mode_for_current_settings()?;
    let executables = vec![executable.to_string()];
    super::codex_app_watcher::expect_codex_app_open_for_executables(&executables);
    match relaunch_codex_executable(executable, mode) {
        Ok(restarted) => Ok(restarted),
        Err(err) => {
            super::codex_app_watcher::clear_expected_codex_app_open_for_executables(&executables);
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

    for pid in &pids {
        let _ = kill_process_tree(*pid);
    }
    let alive = wait_for_pids_exit(&pids, 12_000);
    if !alive.is_empty() {
        return Err("Codex app 进程未能退出".to_string());
    }

    thread::sleep(StdDuration::from_millis(RELAUNCH_DELAY_MS));

    if origin == CodexRelaunchOrigin::AppCommand {
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
                return Err(err);
            }
        }
        thread::sleep(StdDuration::from_millis(120));
    }

    Ok(restarted)
}

fn relaunch_codex_executable(executable: &str, mode: CodexRelaunchMode) -> Result<bool, String> {
    match mode {
        CodexRelaunchMode::Plugin => {
            launch_codex_with_plugins(Path::new(executable))?;
            Ok(true)
        }
        CodexRelaunchMode::Normal => relaunch_executable_with_retry(executable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn codex_app_open_actions_keep_plugin_and_session_actions_separate() {
        let session_only = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": false,
            "codex_session_sync_enabled": true
        }));
        let plugin_only = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": true,
            "codex_session_sync_enabled": false
        }));
        let disabled = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": false,
            "codex_session_sync_enabled": false
        }));

        assert!(session_only.enabled());
        assert!(session_only.session_sync_enabled);
        assert!(!session_only.plugin_unlock_enabled);
        assert!(plugin_only.enabled());
        assert!(plugin_only.plugin_unlock_enabled);
        assert!(!plugin_only.session_sync_enabled);
        assert!(!disabled.enabled());
    }

    #[test]
    fn codex_app_open_relaunches_only_for_session_changes_or_missing_plugin_unlock() {
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
        let session_and_plugin = CodexAppOpenActions::from_settings(&json!({
            "codex_plugins_enabled": true,
            "codex_session_sync_enabled": true
        }));

        assert_eq!(
            codex_relaunch_mode_for_app_open(
                session_only,
                CodexAppOpenStatus {
                    session_sync_updated: true,
                    plugin_unlock_applied: false
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
            Some(CodexRelaunchMode::Plugin)
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(
                plugin_only,
                CodexAppOpenStatus {
                    session_sync_updated: false,
                    plugin_unlock_applied: true
                }
            ),
            None
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(
                session_and_plugin,
                CodexAppOpenStatus {
                    session_sync_updated: false,
                    plugin_unlock_applied: true
                }
            ),
            None
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(
                session_and_plugin,
                CodexAppOpenStatus {
                    session_sync_updated: true,
                    plugin_unlock_applied: true
                }
            ),
            Some(CodexRelaunchMode::Plugin)
        );
        assert_eq!(
            codex_relaunch_mode_for_app_open(
                session_disabled,
                CodexAppOpenStatus {
                    session_sync_updated: true,
                    plugin_unlock_applied: false
                }
            ),
            None
        );
    }
}
