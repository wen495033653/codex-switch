use crate::{
    accounts::*,
    codex_config::ensure_config_file,
    codex_launcher::{
        apply_codex_proxy_env_state_to_settings, attach_ide_reopen, build_ide_reopen_payload,
        IdeRuntime,
    },
    codex_sessions::sync_codex_session_index_then_queue_rollouts,
    desktop::sync_system_auto_start,
    json_util::{bool_field, has_key, raw_string_field, string_field},
    paths::app_data_dir,
    quota::*,
    settings::{default_api_mode, read_settings_value, update_settings_value},
};
use serde_json::{json, Value};
use std::{collections::HashSet, fs, sync::Arc};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};

mod account;
mod general;
mod quota;

pub(crate) use account::*;
pub(crate) use general::*;
pub(crate) use quota::*;
