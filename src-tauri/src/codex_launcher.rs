use crate::{
    accounts::{
        find_store_account, mark_store_account_used, random_urlsafe, set_api_mode,
        set_subscription_mode, store_payload, write_account_auth,
    },
    json_util::{bool_field, raw_string_field, string_field, value_u64_field},
    paths::codex_dir,
    proxy_config::{normalize_proxy_display_url, normalize_proxy_url},
    settings::{default_api_mode, read_settings_value, update_settings_value},
    time_util::now_string,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration as StdDuration, Instant},
};
use tauri::State;
use time::OffsetDateTime;

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

mod codex_app;
mod codex_app_open;
mod codex_app_watcher;
mod ide_snapshot;
mod plugins;
mod process_control;
mod remote_control;
mod scripts;
mod shell;

pub(crate) use codex_app::*;
pub(crate) use codex_app_watcher::{CodexAppOpenOutcome, CodexProcess};
pub(crate) use ide_snapshot::*;
pub(crate) use plugins::*;
pub(crate) use process_control::*;
pub(crate) use remote_control::*;
pub(crate) use scripts::*;
pub(crate) use shell::*;

pub(crate) fn start_codex_app_watcher() {
    codex_app_watcher::start_codex_app_open_watcher(codex_app_open::handle_codex_app_open);
}

#[tauri::command]
pub(crate) fn get_current_codex_app_processes() -> Result<Value, String> {
    codex_app_watcher::current_codex_app_processes_value()
}

#[tauri::command]
pub(crate) fn restart_current_codex_app_for_plugin_setting() -> Result<Value, String> {
    codex_app_open::restart_current_codex_app_for_plugin_setting()
}

#[tauri::command]
pub(crate) fn restart_current_codex_app_normal() -> Result<Value, String> {
    codex_app_open::restart_current_codex_app_normal()
}
