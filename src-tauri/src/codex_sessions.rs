mod global_state;
mod rollouts;
mod state_threads;
mod support;
#[cfg(test)]
mod tests;

pub(crate) use global_state::rewrite_global_state_file;

use crate::{
    accounts::{get_codex_state_value, restore_api_mode_if_selected},
    api_config::API_PROVIDER_ID,
    json_util::raw_string_field,
    paths::codex_dir,
    session_sync_diagnostics::log_session_sync_event,
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{Mutex, MutexGuard},
};
use {
    global_state::{
        preview_global_state_workspace_roots_with_diagnostics,
        sync_global_state_workspace_roots_with_diagnostics,
    },
    rollouts::{
        preview_codex_session_rollout_dirs_to_provider_with_diagnostics,
        sync_codex_session_rollout_dirs_to_provider_with_diagnostics,
    },
    state_threads::{
        pinned_thread_rollout_paths_if_exists,
        preview_codex_state_threads_to_provider_with_diagnostics,
        sync_codex_state_threads_to_provider_with_diagnostics,
    },
    support::{global_state_path, state_db_path},
};

const OPENAI_PROVIDER_ID: &str = "openai";

static CODEX_SESSION_IO_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn lock_codex_session_io(action: &str) -> Result<MutexGuard<'static, ()>, String> {
    CODEX_SESSION_IO_LOCK
        .lock()
        .map_err(|_| format!("{action} I/O 状态已损坏"))
}

fn session_rollout_dir_paths() -> Result<Vec<PathBuf>, String> {
    let codex_dir = codex_dir()?;
    Ok(vec![
        codex_dir.join("sessions"),
        codex_dir.join("archived_sessions"),
    ])
}

pub(crate) fn current_session_provider() -> Result<String, String> {
    restore_api_mode_if_selected()?;
    let state = get_codex_state_value();
    let model_provider = raw_string_field(&state, "model_provider");
    if !model_provider.is_empty() {
        return Ok(model_provider);
    }

    match raw_string_field(&state, "mode").as_str() {
        "api" => Ok(API_PROVIDER_ID.to_string()),
        "chatgpt" => Ok(OPENAI_PROVIDER_ID.to_string()),
        _ => Err("当前 Codex 模式未知，无法同步会话".to_string()),
    }
}

fn normalize_target_provider(target_provider: &str) -> Result<String, String> {
    let target_provider = target_provider.trim();
    if target_provider.is_empty() {
        return Err("当前 Codex provider 为空，无法同步会话".to_string());
    }
    Ok(target_provider.to_string())
}

pub(crate) fn sync_codex_sessions_to_current_mode_now_from(trigger: &str) -> Result<usize, String> {
    log_session_sync_event(
        "session_sync_current_mode_resolve_start",
        json!({ "trigger": trigger }),
    );
    let target_provider = current_session_provider()?;
    log_session_sync_event(
        "session_sync_current_mode_resolved",
        json!({
            "trigger": trigger,
            "targetProvider": target_provider
        }),
    );
    sync_codex_sessions_to_provider_now_from(&target_provider, trigger)
}

pub(crate) fn preview_codex_sessions_to_current_mode_now_from(
    trigger: &str,
) -> Result<usize, String> {
    log_session_sync_event(
        "session_sync_preflight_current_mode_resolve_start",
        json!({ "trigger": trigger }),
    );
    let target_provider = current_session_provider()?;
    log_session_sync_event(
        "session_sync_preflight_current_mode_resolved",
        json!({
            "trigger": trigger,
            "targetProvider": target_provider
        }),
    );
    preview_codex_sessions_to_provider_now_from(&target_provider, trigger)
}

pub(crate) fn preview_codex_sessions_to_provider_now_from(
    target_provider: &str,
    trigger: &str,
) -> Result<usize, String> {
    let target_provider = normalize_target_provider(target_provider)?;
    log_session_sync_event(
        "session_sync_preflight_start",
        json!({
            "trigger": trigger,
            "targetProvider": target_provider
        }),
    );
    let _guard = lock_codex_session_io("Codex 会话同步")?;
    let mut updated = 0;
    let mut state_db_updated = 0;
    let mut rollout_files_updated = 0;
    let mut global_state_updated = 0;
    let mut errors = Vec::new();

    match preview_codex_state_threads_to_provider_if_exists(&target_provider, trigger) {
        Ok(count) => {
            state_db_updated = count;
            updated += count;
        }
        Err(err) => errors.push(err),
    }
    match preview_codex_session_rollouts_to_provider_if_exists(&target_provider, trigger) {
        Ok(count) => {
            rollout_files_updated = count;
            updated += count;
        }
        Err(err) => errors.push(err),
    }
    match preview_codex_global_state_workspace_roots_if_exists(trigger) {
        Ok(count) => {
            global_state_updated = count;
            updated += count;
        }
        Err(err) => errors.push(err),
    }

    if errors.is_empty() {
        log_session_sync_event(
            "session_sync_preflight_finish",
            json!({
                "trigger": trigger,
                "targetProvider": target_provider,
                "stateDbUpdated": state_db_updated,
                "rolloutFilesUpdated": rollout_files_updated,
                "globalStateUpdated": global_state_updated,
                "updated": updated
            }),
        );
        Ok(updated)
    } else {
        log_session_sync_event(
            "session_sync_preflight_error",
            json!({
                "trigger": trigger,
                "targetProvider": target_provider,
                "stateDbUpdated": state_db_updated,
                "rolloutFilesUpdated": rollout_files_updated,
                "globalStateUpdated": global_state_updated,
                "updated": updated,
                "errors": errors.clone()
            }),
        );
        Err(format!(
            "预检查 Codex 会话同步失败，预计更新 {updated} 项：{}",
            errors.join("；")
        ))
    }
}

pub(crate) fn sync_codex_sessions_to_provider_now_from(
    target_provider: &str,
    trigger: &str,
) -> Result<usize, String> {
    let target_provider = normalize_target_provider(target_provider)?;
    log_session_sync_event(
        "session_sync_start",
        json!({
            "trigger": trigger,
            "targetProvider": target_provider
        }),
    );
    let _guard = lock_codex_session_io("Codex 会话同步")?;
    let mut updated = 0;
    let mut state_db_updated = 0;
    let mut rollout_files_updated = 0;
    let mut global_state_updated = 0;
    let mut errors = Vec::new();

    match sync_codex_state_threads_to_provider_if_exists(&target_provider, trigger) {
        Ok(count) => {
            state_db_updated = count;
            updated += count;
        }
        Err(err) => errors.push(err),
    }
    match sync_codex_session_rollouts_to_provider_if_exists(&target_provider, trigger) {
        Ok(count) => {
            rollout_files_updated = count;
            updated += count;
        }
        Err(err) => errors.push(err),
    }
    match sync_codex_global_state_workspace_roots_if_exists(trigger) {
        Ok(count) => {
            global_state_updated = count;
            updated += count;
        }
        Err(err) => errors.push(err),
    }

    if errors.is_empty() {
        log_session_sync_event(
            "session_sync_finish",
            json!({
                "trigger": trigger,
                "targetProvider": target_provider,
                "stateDbUpdated": state_db_updated,
                "rolloutFilesUpdated": rollout_files_updated,
                "globalStateUpdated": global_state_updated,
                "updated": updated
            }),
        );
        Ok(updated)
    } else {
        log_session_sync_event(
            "session_sync_error",
            json!({
                "trigger": trigger,
                "targetProvider": target_provider,
                "stateDbUpdated": state_db_updated,
                "rolloutFilesUpdated": rollout_files_updated,
                "globalStateUpdated": global_state_updated,
                "updated": updated,
                "errors": errors.clone()
            }),
        );
        Err(format!(
            "同步 Codex 会话失败，已更新 {updated} 项：{}",
            errors.join("；")
        ))
    }
}

fn preview_codex_session_rollouts_to_provider_if_exists(
    target_provider: &str,
    trigger: &str,
) -> Result<usize, String> {
    preview_codex_session_rollout_dirs_to_provider_with_diagnostics(
        &session_rollout_dir_paths()?,
        target_provider,
        &pinned_thread_rollout_paths_if_exists()?,
        Some(trigger),
    )
}

fn sync_codex_session_rollouts_to_provider_if_exists(
    target_provider: &str,
    trigger: &str,
) -> Result<usize, String> {
    sync_codex_session_rollout_dirs_to_provider_with_diagnostics(
        &session_rollout_dir_paths()?,
        target_provider,
        &pinned_thread_rollout_paths_if_exists()?,
        Some(trigger),
    )
}

fn sync_codex_state_threads_to_provider_if_exists(
    target_provider: &str,
    trigger: &str,
) -> Result<usize, String> {
    let state_db = state_db_path()?;
    sync_codex_state_threads_to_provider_with_diagnostics(&state_db, target_provider, Some(trigger))
}

fn preview_codex_state_threads_to_provider_if_exists(
    target_provider: &str,
    trigger: &str,
) -> Result<usize, String> {
    let state_db = state_db_path()?;
    preview_codex_state_threads_to_provider_with_diagnostics(
        &state_db,
        target_provider,
        Some(trigger),
        // Auxiliary cwd/user-event repairs must not force a Codex relaunch.
        false,
    )
}

fn sync_codex_global_state_workspace_roots_if_exists(trigger: &str) -> Result<usize, String> {
    let path = global_state_path()?;
    sync_global_state_workspace_roots_with_diagnostics(&path, Some(trigger))
}

fn preview_codex_global_state_workspace_roots_if_exists(trigger: &str) -> Result<usize, String> {
    let path = global_state_path()?;
    preview_global_state_workspace_roots_with_diagnostics(&path, Some(trigger))
}
