use super::{
    kill_process_tree, launch_codex_with_plugins, relaunch_executable_with_retry,
    wait_for_pids_exit, CodexAppOpenOutcome, CodexProcess,
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

    if actions.session_sync_enabled {
        if let Err(err) = sync_codex_sessions_to_current_mode_now() {
            eprintln!("Codex app 会话同步失败: {err}");
        }
    }

    if !actions.plugin_unlock_enabled {
        return Ok(CodexAppOpenOutcome::default());
    }

    let restarted = relaunch_running_codex_processes(
        processes,
        CodexRelaunchMode::Plugin,
        CodexRelaunchOrigin::Watcher,
    )?;
    Ok(CodexAppOpenOutcome {
        relaunch_expected: restarted > 0,
    })
}

fn codex_app_open_actions() -> Result<CodexAppOpenActions, String> {
    read_settings_value().map(|settings| CodexAppOpenActions::from_settings(&settings))
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
}
