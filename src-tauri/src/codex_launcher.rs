use crate::{
    accounts::{
        find_store_account, mark_store_account_used, random_urlsafe, set_api_mode,
        set_subscription_mode, store_payload, write_account_auth,
    },
    json_util::{bool_field, raw_string_field, string_field, value_u64_field},
    paths::codex_dir,
    proxy_config::{
        assert_proxy_ready, build_proxy_environment, normalize_proxy_display_url,
        normalize_proxy_url,
    },
    settings::{default_api_mode, read_settings_value},
    time_util::now_string,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
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
}

#[derive(Default)]
pub(crate) struct IdeRuntime {
    snapshots: Mutex<HashMap<String, IdePending>>,
}

mod codex_app;
mod ide_snapshot;
mod process_control;
mod scripts;
mod shell;

pub(crate) use codex_app::*;
pub(crate) use ide_snapshot::*;
pub(crate) use process_control::*;
pub(crate) use scripts::*;
pub(crate) use shell::*;
