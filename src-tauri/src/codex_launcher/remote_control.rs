use super::{
    get_alive_pids, hide_command_window, json_as_array, kill_process_tree, parse_json_output,
    run_pwsh,
};
use crate::{
    accounts::{find_store_account, read_api_key_from_auth},
    api_config::API_PROVIDER_ID,
    codex_config::{read_root_config, remove_remote_control_config},
    codex_sessions::{
        collect_recent_codex_rollout_files_for_remote_control, current_session_provider,
        sync_codex_rollout_files_to_provider_for_remote_control,
    },
    json_util::{bool_field, string_field},
    paths::{app_data_dir, codex_dir, config_path, ensure_parent_dir},
    proxy_config::normalize_proxy_url,
    session_sync_diagnostics::log_session_sync_event,
    settings::{default_api_mode, read_settings_value, update_settings_value},
    time_util::now_string,
};
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection, OpenFlags};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration as StdDuration, Instant},
};

const REMOTE_CONTROL_ENABLED_SETTING_KEY: &str = "codex_remote_control_enabled";
const LEGACY_REMOTE_CONTROL_HOOK_SETTING_KEY: &str = "codex_remote_control_hook_enabled";
const REMOTE_CONTROL_ACCOUNT_SETTING_KEY: &str = "codex_remote_control_account_id";
const REMOTE_CONTROL_FEATURES_TABLE: &str = "features";
const REMOTE_CONTROL_CONFIG_KEY: &str = "remote_control";
const LEGACY_REMOTE_CONNECTIONS_CONFIG_KEY: &str = "remote_connections";
const REMOTE_CONTROL_BACKEND_ERROR_EVENT: &str = "remote_control_backend_error";
const REMOTE_CONTROL_HELPER_ERROR_EVENT: &str = "remote_control_helper_error";
const REMOTE_CONTROL_HELPER_SPAWN_EVENT: &str = "remote_control_helper_spawn";
const REMOTE_CONTROL_HELPER_STATUS_EVENT: &str = "remote_control_helper_status";
const REMOTE_CONTROL_HELPER_WS_CONNECTED_EVENT: &str = "remote_control_helper_ws_connected";
const REMOTE_CONTROL_BACKEND_ERROR_TEXT_MAX_LEN: usize = 1800;
const REMOTE_CONTROL_ENVIRONMENTS_ENDPOINT: &str =
    "https://chatgpt.com/backend-api/wham/remote/control/environments";
const REMOTE_CONTROL_BACKEND_STATUS_TIMEOUT_MS: u64 = 6_000;
pub(crate) const CODEX_REMOTE_CONTROL_HOME_ENV: &str = "CODEX_SWITCH_REMOTE_CONTROL_HOME";
const CODEX_REMOTE_CONTROL_HELPER_ENV: &str = "CODEX_SWITCH_REMOTE_CONTROL_HELPER";
const CODEX_INTERNAL_ORIGINATOR_OVERRIDE_ENV: &str = "CODEX_INTERNAL_ORIGINATOR_OVERRIDE";
const CODEX_DESKTOP_ORIGINATOR: &str = "Codex Desktop";
const REMOTE_CONTROL_HELPER_CONNECT_DELAY_MS: u64 = 1_200;
const REMOTE_CONTROL_HELPER_STATUS_POLL_MS: u64 = 15_000;
const REMOTE_CONTROL_HELPER_WS_READ_TIMEOUT_MS: u64 = 1_000;
const REMOTE_CONTROL_HELPER_NO_PROXY_VALUE: &str = "localhost,127.0.0.1,::1";
const REMOTE_CONTROL_UPSTREAM_WS_URL: &str =
    "wss://chatgpt.com/backend-api/wham/remote/control/server";
const REMOTE_CONTROL_UPSTREAM_PATH: &str = "/backend-api/wham/remote/control/server";
const REMOTE_CONTROL_HISTORY_DIRS: [&str; 2] = ["sessions", "archived_sessions"];
const REMOTE_CONTROL_SESSION_INDEX_FILE: &str = "session_index.jsonl";
const REMOTE_CONTROL_RECENT_HISTORY_LIMIT: usize = 50;

#[derive(Clone, Copy)]
struct RemoteControlHelperProcess {
    pid: u64,
    port: u16,
}

#[derive(Default)]
struct RemoteControlHistorySyncOutcome {
    files_copied: usize,
    session_index_changed: bool,
    global_state_copied: bool,
    state_threads_merged: usize,
    rollout_files_updated: usize,
}

#[derive(Default)]
struct RemoteControlStateMergeOutcome {
    changed: usize,
    thread_ids: HashSet<String>,
    rollout_paths: Vec<PathBuf>,
}

static REMOTE_CONTROL_HELPER_PROCESS: OnceLock<Mutex<Option<RemoteControlHelperProcess>>> =
    OnceLock::new();

pub(crate) fn remote_control_enabled_from_settings(settings: &Value) -> bool {
    bool_field(settings, REMOTE_CONTROL_ENABLED_SETTING_KEY)
        || bool_field(settings, LEGACY_REMOTE_CONTROL_HOOK_SETTING_KEY)
}

fn remote_control_account_id_from_settings(settings: &Value) -> String {
    string_field(settings, REMOTE_CONTROL_ACCOUNT_SETTING_KEY)
}

fn remote_control_account_tokens(account_id: &str) -> Result<Value, String> {
    let account = find_store_account(account_id)
        .map_err(|_| format!("app远程控制账号不存在，请重新选择: {account_id}"))?;
    account
        .get("tokens")
        .cloned()
        .ok_or_else(|| "app远程控制订阅账号缺少 tokens".to_string())
}

fn validate_remote_control_account_id(account_id: &str) -> Result<(), String> {
    if account_id.is_empty() {
        return Err("app远程控制账号不能为空".to_string());
    }
    remote_control_account_tokens(account_id).map(|_| ())
}

fn validate_remote_control_enable_prerequisites() -> Result<(), String> {
    let settings = read_settings_value()?;
    let account_id = remote_control_account_id_from_settings(&settings);
    if account_id.is_empty() {
        return Err("app远程控制需要先单独选择一个订阅账号".to_string());
    }
    validate_remote_control_account_id(&account_id)?;
    remote_control_api_session_profile_from_settings(&settings).map(|_| ())
}

fn validate_remote_control_runtime_preview_prerequisites(settings: &Value) -> Result<(), String> {
    let account_id = remote_control_account_id_from_settings(settings);
    if account_id.is_empty() {
        return Err("app远程控制需要先单独选择一个订阅账号".to_string());
    }
    remote_control_account_tokens(&account_id)?;

    let api_mode = settings
        .get("api_mode")
        .cloned()
        .unwrap_or_else(default_api_mode);
    if string_field(&api_mode, "base_url").is_empty() {
        return Err("app远程控制会话流量走 API 需要先配置 API 模式 base_url".to_string());
    }

    let mut api_key = string_field(&api_mode, "api_key");
    if api_key.is_empty() {
        api_key = read_api_key_from_auth();
    }
    if api_key.trim().is_empty() {
        return Err("app远程控制会话流量走 API 需要先配置 API Key".to_string());
    }

    remote_control_helper_proxy_url_from_settings(settings).map(|_| ())
}

fn remote_control_subscription_home_dir() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join("remote-control-codex-home"))
}

fn format_remote_control_toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn remote_control_api_session_profile_from_settings(
    settings: &Value,
) -> Result<(String, String, String), String> {
    let api_mode = settings
        .get("api_mode")
        .cloned()
        .unwrap_or_else(default_api_mode);
    let base_url = string_field(&api_mode, "base_url");
    if base_url.is_empty() {
        return Err("app远程控制会话流量走 API 需要先配置 API 模式 base_url".to_string());
    }

    let mut api_key = string_field(&api_mode, "api_key");
    if api_key.is_empty() {
        api_key = read_api_key_from_auth();
    }
    if api_key.trim().is_empty() {
        return Err("app远程控制会话流量走 API 需要先配置 API Key".to_string());
    }

    let model = read_root_config()
        .ok()
        .and_then(|config| {
            config
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();

    Ok((base_url, api_key, model))
}

fn remote_control_subscription_config_text(api_base_url: &str, model: &str) -> String {
    let mut lines = Vec::new();
    if !model.is_empty() {
        lines.push(format!(
            "model = {}",
            format_remote_control_toml_string(model)
        ));
    }
    lines.push(format!(
        "model_provider = {}",
        format_remote_control_toml_string(API_PROVIDER_ID)
    ));
    lines.push("cli_auth_credentials_store = \"file\"".to_string());
    lines.push(String::new());
    lines.push("[features]".to_string());
    lines.push("remote_control = true".to_string());
    lines.push(String::new());
    lines.push(format!("[model_providers.{API_PROVIDER_ID}]"));
    lines.push(format!(
        "name = {}",
        format_remote_control_toml_string(API_PROVIDER_ID)
    ));
    lines.push(format!(
        "base_url = {}",
        format_remote_control_toml_string(api_base_url)
    ));
    lines.push("env_key = \"OPENAI_API_KEY\"".to_string());
    lines.push("supports_websockets = false".to_string());
    lines.push("requires_openai_auth = false".to_string());
    lines.push(String::new());
    lines.join("\n")
}

fn sync_remote_control_history_from_root(
    home: &Path,
) -> Result<RemoteControlHistorySyncOutcome, String> {
    let source_home = codex_dir()?;
    if source_home == home {
        return Ok(RemoteControlHistorySyncOutcome::default());
    }
    let current_provider = current_session_provider()?;

    let mut outcome = RemoteControlHistorySyncOutcome::default();
    let main_to_remote =
        merge_remote_control_state_threads_recent(&source_home, home, API_PROVIDER_ID)?;
    let (files_copied, rollout_files_updated) = copy_recent_remote_control_rollouts(
        &source_home,
        home,
        &main_to_remote.rollout_paths,
        API_PROVIDER_ID,
    )?;
    outcome.files_copied += files_copied;
    outcome.rollout_files_updated += rollout_files_updated;
    outcome.session_index_changed |= merge_remote_control_session_index_recent(
        &source_home.join(REMOTE_CONTROL_SESSION_INDEX_FILE),
        &home.join(REMOTE_CONTROL_SESSION_INDEX_FILE),
        &main_to_remote.thread_ids,
    )?;
    outcome.state_threads_merged += main_to_remote.changed;

    let remote_to_main =
        merge_remote_control_state_threads_recent(home, &source_home, &current_provider)?;
    let (files_copied, rollout_files_updated) = copy_recent_remote_control_rollouts(
        home,
        &source_home,
        &remote_to_main.rollout_paths,
        &current_provider,
    )?;
    outcome.files_copied += files_copied;
    outcome.rollout_files_updated += rollout_files_updated;
    outcome.session_index_changed |= merge_remote_control_session_index_recent(
        &home.join(REMOTE_CONTROL_SESSION_INDEX_FILE),
        &source_home.join(REMOTE_CONTROL_SESSION_INDEX_FILE),
        &remote_to_main.thread_ids,
    )?;
    outcome.state_threads_merged += remote_to_main.changed;

    if outcome.files_copied > 0
        || outcome.session_index_changed
        || outcome.state_threads_merged > 0
        || outcome.rollout_files_updated > 0
    {
        log_session_sync_event(
            "codex_remote_control_history_synced",
            json!({
                "sourceHome": source_home,
                "targetHome": home,
                "limit": REMOTE_CONTROL_RECENT_HISTORY_LIMIT,
                "mainToRemoteThreads": main_to_remote.thread_ids.len(),
                "remoteToMainThreads": remote_to_main.thread_ids.len(),
                "remoteToMainProvider": current_provider,
                "filesCopied": outcome.files_copied,
                "sessionIndexChanged": outcome.session_index_changed,
                "globalStateCopied": outcome.global_state_copied,
                "stateThreadsMerged": outcome.state_threads_merged,
                "rolloutFilesUpdated": outcome.rollout_files_updated
            }),
        );
    }
    Ok(outcome)
}

fn copy_remote_control_history_file_if_newer(source: &Path, target: &Path) -> Result<bool, String> {
    if !source.exists() {
        return Ok(false);
    }
    let source_metadata = fs::metadata(source)
        .map_err(|err| format!("读取远程控制历史源文件失败 {}: {err}", source.display()))?;
    let should_copy = match fs::metadata(target) {
        Ok(target_metadata) => match (source_metadata.modified(), target_metadata.modified()) {
            (Ok(source_modified), Ok(target_modified)) => source_modified > target_modified,
            _ => true,
        },
        Err(err) if err.kind() == ErrorKind::NotFound => true,
        Err(err) => {
            return Err(format!(
                "读取远程控制历史目标文件失败 {}: {err}",
                target.display()
            ))
        }
    };
    if !should_copy {
        return Ok(false);
    }

    ensure_parent_dir(target)?;
    fs::copy(source, target).map_err(|err| {
        format!(
            "复制远程控制历史文件失败 {} -> {}: {err}",
            source.display(),
            target.display()
        )
    })?;
    if let Ok(source_modified) = source_metadata.modified() {
        fs::OpenOptions::new()
            .write(true)
            .open(target)
            .and_then(|file| file.set_modified(source_modified))
            .map_err(|err| {
                format!(
                    "恢复远程控制历史文件修改时间失败 {}: {err}",
                    target.display()
                )
            })?;
    }
    Ok(true)
}

fn copy_recent_remote_control_rollouts(
    source_home: &Path,
    target_home: &Path,
    state_rollout_paths: &[PathBuf],
    target_provider: &str,
) -> Result<(usize, usize), String> {
    let rollout_dirs = REMOTE_CONTROL_HISTORY_DIRS
        .iter()
        .map(|dir_name| source_home.join(dir_name))
        .collect::<Vec<_>>();
    let mut source_paths = collect_recent_codex_rollout_files_for_remote_control(
        &rollout_dirs,
        REMOTE_CONTROL_RECENT_HISTORY_LIMIT,
    )?;
    let mut seen = source_paths.iter().cloned().collect::<HashSet<_>>();
    for path in state_rollout_paths {
        if path.is_file() && seen.insert(path.clone()) {
            source_paths.push(path.clone());
        }
    }

    let mut copied = 0;
    let mut target_paths = Vec::new();
    let mut seen_targets = HashSet::new();
    for source_path in source_paths {
        let target_path = PathBuf::from(map_remote_control_history_path(
            &source_path.to_string_lossy(),
            source_home,
            target_home,
        ));
        if copy_remote_control_history_file_if_newer(&source_path, &target_path)? {
            copied += 1;
        }
        if target_path.is_file() && seen_targets.insert(target_path.clone()) {
            target_paths.push(target_path);
        }
    }
    let rollout_files_updated =
        sync_codex_rollout_files_to_provider_for_remote_control(target_paths, target_provider)?;
    Ok((copied, rollout_files_updated))
}

fn merge_remote_control_session_index_recent(
    source: &Path,
    target: &Path,
    allowed_ids: &HashSet<String>,
) -> Result<bool, String> {
    if allowed_ids.is_empty() {
        return Ok(false);
    }
    merge_remote_control_session_index_filtered(source, target, Some(allowed_ids))
}

fn merge_remote_control_session_index_filtered(
    source: &Path,
    target: &Path,
    allowed_ids: Option<&HashSet<String>>,
) -> Result<bool, String> {
    if !source.exists() {
        return Ok(false);
    }
    let source_text = fs::read_to_string(source).map_err(|err| {
        format!(
            "读取远程控制 session_index 源文件失败 {}: {err}",
            source.display()
        )
    })?;
    let target_text = match fs::read_to_string(target) {
        Ok(text) => text,
        Err(err) if err.kind() == ErrorKind::NotFound => String::new(),
        Err(err) => {
            return Err(format!(
                "读取远程控制 session_index 目标文件失败 {}: {err}",
                target.display()
            ))
        }
    };

    let mut entries = Vec::<RemoteControlSessionIndexEntry>::new();
    let mut merged_lines = Vec::new();
    push_remote_control_session_index_lines(&target_text, allowed_ids, &mut entries);
    push_remote_control_session_index_lines(&source_text, allowed_ids, &mut entries);
    entries.sort_by(|a, b| {
        b.sort_key
            .cmp(&a.sort_key)
            .then_with(|| b.line.cmp(&a.line))
    });
    let mut seen_ids = HashSet::new();
    let mut seen_raw_lines = HashSet::new();
    for entry in entries {
        if let Some(id) = entry.id {
            if seen_ids.insert(id) {
                merged_lines.push(entry.line);
            }
        } else if allowed_ids.is_none() && seen_raw_lines.insert(entry.line.clone()) {
            merged_lines.push(entry.line);
        }
    }
    let merged_text = if merged_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", merged_lines.join("\n"))
    };
    if merged_text == target_text {
        return Ok(false);
    }

    ensure_parent_dir(target)?;
    fs::write(target, merged_text).map_err(|err| {
        format!(
            "写入远程控制 session_index 目标文件失败 {}: {err}",
            target.display()
        )
    })?;
    Ok(true)
}

struct RemoteControlSessionIndexEntry {
    id: Option<String>,
    sort_key: String,
    line: String,
}

fn push_remote_control_session_index_lines(
    text: &str,
    allowed_ids: Option<&HashSet<String>>,
    entries: &mut Vec<RemoteControlSessionIndexEntry>,
) {
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let id = remote_control_session_index_line_id(line);
        if let Some(allowed_ids) = allowed_ids {
            let Some(id) = id.as_ref() else {
                continue;
            };
            if !allowed_ids.contains(id) {
                continue;
            }
        }
        entries.push(RemoteControlSessionIndexEntry {
            id,
            sort_key: remote_control_session_index_line_sort_key(line),
            line: line.to_string(),
        });
    }
}

fn remote_control_session_index_line_id(line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line).ok()?;
    let id = value.get("id")?.as_str()?.trim();
    (!id.is_empty()).then(|| id.to_string())
}

fn remote_control_session_index_line_sort_key(line: &str) -> String {
    let value: Value = serde_json::from_str(line).unwrap_or(Value::Null);
    ["updated_at", "created_at", "timestamp"]
        .iter()
        .find_map(|key| {
            let value = value.get(*key)?.as_str()?.trim();
            (!value.is_empty()).then(|| value.replace('Z', "+00:00"))
        })
        .unwrap_or_default()
}

fn merge_remote_control_state_threads_recent(
    source_home: &Path,
    target_home: &Path,
    target_provider: &str,
) -> Result<RemoteControlStateMergeOutcome, String> {
    merge_remote_control_state_threads_with_limit(
        source_home,
        target_home,
        target_provider,
        REMOTE_CONTROL_RECENT_HISTORY_LIMIT,
    )
}

fn merge_remote_control_state_threads_with_limit(
    source_home: &Path,
    target_home: &Path,
    target_provider: &str,
    limit: usize,
) -> Result<RemoteControlStateMergeOutcome, String> {
    let source_db = remote_control_state_db_path(source_home);
    let target_db = remote_control_state_db_path(target_home);
    if !source_db.exists() {
        return Ok(RemoteControlStateMergeOutcome::default());
    }
    if !target_db.exists() {
        copy_remote_control_state_db(&source_db, &target_db)?;
        if limit != usize::MAX {
            clear_remote_control_state_threads(&target_db)?;
        }
    }

    let source = Connection::open_with_flags(
        &source_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| {
        format!(
            "打开远程控制历史源 state DB 失败 {}: {err}",
            source_db.display()
        )
    })?;
    let mut target = Connection::open(&target_db).map_err(|err| {
        format!(
            "打开远程控制历史目标 state DB 失败 {}: {err}",
            target_db.display()
        )
    })?;
    source
        .busy_timeout(StdDuration::from_millis(3000))
        .map_err(|err| format!("配置远程控制历史源 state DB 等待超时失败: {err}"))?;
    target
        .busy_timeout(StdDuration::from_millis(3000))
        .map_err(|err| format!("配置远程控制历史目标 state DB 等待超时失败: {err}"))?;

    let source_columns = sqlite_table_columns(&source, "threads")?;
    let target_columns = sqlite_table_columns(&target, "threads")?;
    let source_column_set: HashSet<String> = source_columns.iter().cloned().collect();
    let target_column_set: HashSet<String> = target_columns.iter().cloned().collect();
    let columns = target_columns
        .into_iter()
        .filter(|column| source_column_set.contains(column))
        .collect::<Vec<_>>();
    if !columns.iter().any(|column| column == "id") {
        return Err("远程控制历史 state DB threads 缺少 id 列".to_string());
    }
    let id_column_index = columns
        .iter()
        .position(|column| column == "id")
        .ok_or_else(|| "远程控制历史 state DB threads 缺少 id 列".to_string())?;

    let select_sql = format!(
        "SELECT {} FROM threads{}{}",
        columns
            .iter()
            .map(|column| quote_sqlite_identifier(column))
            .collect::<Vec<_>>()
            .join(", "),
        sqlite_threads_recent_order_clause(&source_column_set),
        if limit == usize::MAX {
            String::new()
        } else {
            format!(" LIMIT {}", limit)
        }
    );
    let mut statement = source
        .prepare(&select_sql)
        .map_err(|err| format!("读取远程控制历史源 threads 失败: {err}"))?;
    let mut source_rows = statement
        .query([])
        .map_err(|err| format!("查询远程控制历史源 threads 失败: {err}"))?;
    let insert_sql = format!(
        "INSERT OR IGNORE INTO threads ({}) VALUES ({})",
        columns
            .iter()
            .map(|column| quote_sqlite_identifier(column))
            .collect::<Vec<_>>()
            .join(", "),
        (1..=columns.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let update_sql = if limit == usize::MAX {
        None
    } else {
        remote_control_thread_update_sql(&columns)
    };

    let transaction = target
        .transaction()
        .map_err(|err| format!("开始远程控制历史 state DB 同步事务失败: {err}"))?;
    let mut merged = 0;
    let mut thread_ids = HashSet::new();
    let mut rollout_paths = Vec::new();
    while let Some(row) = source_rows
        .next()
        .map_err(|err| format!("读取远程控制历史源 thread 行失败: {err}"))?
    {
        let mut values = Vec::with_capacity(columns.len());
        let mut source_rollout_path = None;
        for (index, column) in columns.iter().enumerate() {
            let mut value = row
                .get::<_, SqlValue>(index)
                .map_err(|err| format!("读取远程控制历史源 thread 字段失败 {column}: {err}"))?;
            if column == "model_provider" {
                value = SqlValue::Text(target_provider.to_string());
            } else if column == "rollout_path" {
                if let SqlValue::Text(path) = value {
                    source_rollout_path = Some(PathBuf::from(&path));
                    value = SqlValue::Text(map_remote_control_history_path(
                        &path,
                        source_home,
                        target_home,
                    ));
                }
            }
            values.push(value);
        }
        let thread_id = match values.get(id_column_index) {
            Some(SqlValue::Text(id)) if !id.trim().is_empty() => id.clone(),
            _ => continue,
        };
        if let Some(path) = source_rollout_path {
            rollout_paths.push(path);
        }
        let source_sort_key = remote_control_thread_sort_key(&columns, &values);
        let inserted = transaction
            .execute(&insert_sql, params_from_iter(values.iter()))
            .map_err(|err| format!("写入远程控制历史目标 thread 失败: {err}"))?;
        let mut row_changes = inserted;
        if inserted == 0 {
            if let Some(update_sql) = update_sql.as_deref() {
                let target_values =
                    query_remote_control_thread_values(&transaction, &columns, &thread_id)?;
                if let Some(target_values) = target_values {
                    let target_sort_key = remote_control_thread_sort_key(&columns, &target_values);
                    if source_sort_key > target_sort_key {
                        let mut update_values = columns
                            .iter()
                            .zip(values.iter())
                            .filter(|(column, _value)| column.as_str() != "id")
                            .map(|(_column, value)| value.clone())
                            .collect::<Vec<_>>();
                        update_values.push(SqlValue::Text(thread_id.clone()));
                        row_changes += transaction
                            .execute(update_sql, params_from_iter(update_values.iter()))
                            .map_err(|err| format!("更新远程控制历史目标 thread 失败: {err}"))?;
                    }
                }
            }
        }
        merged += row_changes;
        thread_ids.insert(thread_id);
    }
    let provider_updates = if target_column_set.contains("model_provider") {
        normalize_remote_control_state_provider_for_ids(&transaction, &thread_ids, target_provider)?
    } else {
        0
    };
    let path_updates = if target_column_set.contains("rollout_path") {
        normalize_remote_control_state_rollout_paths_for_ids(
            &transaction,
            source_home,
            target_home,
            &thread_ids,
        )?
    } else {
        0
    };
    transaction
        .commit()
        .map_err(|err| format!("保存远程控制历史 state DB 同步结果失败: {err}"))?;
    Ok(RemoteControlStateMergeOutcome {
        changed: merged + provider_updates + path_updates,
        thread_ids,
        rollout_paths,
    })
}

fn remote_control_thread_update_sql(columns: &[String]) -> Option<String> {
    let assignments = columns
        .iter()
        .filter(|column| column.as_str() != "id")
        .enumerate()
        .map(|(index, column)| format!("{} = ?{}", quote_sqlite_identifier(column), index + 1))
        .collect::<Vec<_>>();
    if assignments.is_empty() {
        return None;
    }
    Some(format!(
        "UPDATE threads SET {} WHERE id = ?{}",
        assignments.join(", "),
        assignments.len() + 1
    ))
}

fn query_remote_control_thread_values(
    connection: &Connection,
    columns: &[String],
    thread_id: &str,
) -> Result<Option<Vec<SqlValue>>, String> {
    let sql = format!(
        "SELECT {} FROM threads WHERE id = ?1",
        columns
            .iter()
            .map(|column| quote_sqlite_identifier(column))
            .collect::<Vec<_>>()
            .join(", ")
    );
    match connection.query_row(&sql, [thread_id], |row| {
        let mut values = Vec::with_capacity(columns.len());
        for index in 0..columns.len() {
            values.push(row.get::<_, SqlValue>(index)?);
        }
        Ok(values)
    }) {
        Ok(values) => Ok(Some(values)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(format!("读取远程控制历史目标 thread 失败: {err}")),
    }
}

fn remote_control_thread_sort_key(columns: &[String], values: &[SqlValue]) -> Vec<String> {
    let mut sort_key = Vec::new();
    for column_name in ["updated_at_ms", "updated_at", "created_at_ms", "created_at"] {
        let value = columns
            .iter()
            .position(|column| column == column_name)
            .and_then(|index| values.get(index));
        sort_key.push(remote_control_sql_sort_value(value));
    }
    let id_value = columns
        .iter()
        .position(|column| column == "id")
        .and_then(|index| values.get(index));
    sort_key.push(remote_control_sql_sort_value(id_value));
    sort_key
}

fn remote_control_sql_sort_value(value: Option<&SqlValue>) -> String {
    match value {
        Some(SqlValue::Integer(value)) => format!("{value:020}"),
        Some(SqlValue::Real(value)) if value.is_finite() => format!("{value:020.6}"),
        Some(SqlValue::Text(value)) => {
            let value = value.trim().replace('Z', "+00:00");
            value
                .parse::<i64>()
                .map(|number| format!("{number:020}"))
                .unwrap_or(value)
        }
        _ => String::new(),
    }
}

fn copy_remote_control_state_db(source_db: &Path, target_db: &Path) -> Result<(), String> {
    ensure_parent_dir(target_db)?;
    let source = Connection::open_with_flags(
        source_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| {
        format!(
            "打开远程控制历史源 state DB 失败 {}: {err}",
            source_db.display()
        )
    })?;
    let target = target_db.to_string_lossy().to_string();
    source.execute("VACUUM INTO ?1", [target]).map_err(|err| {
        format!(
            "复制远程控制历史 state DB 失败 {}: {err}",
            target_db.display()
        )
    })?;
    Ok(())
}

fn clear_remote_control_state_threads(target_db: &Path) -> Result<(), String> {
    let connection = Connection::open(target_db).map_err(|err| {
        format!(
            "打开远程控制历史目标 state DB 失败 {}: {err}",
            target_db.display()
        )
    })?;
    connection
        .execute("DELETE FROM threads", [])
        .map_err(|err| format!("清空远程控制历史目标 threads 失败: {err}"))?;
    Ok(())
}

fn normalize_remote_control_state_provider_for_ids(
    connection: &Connection,
    thread_ids: &HashSet<String>,
    target_provider: &str,
) -> Result<usize, String> {
    let mut updated = 0;
    for thread_id in thread_ids {
        updated += connection
            .execute(
                "UPDATE threads
                 SET model_provider = ?1
                 WHERE id = ?2
                    AND (model_provider IS NULL OR model_provider <> ?1)",
                params![target_provider, thread_id],
            )
            .map_err(|err| format!("归一远程控制历史 thread provider 失败: {err}"))?;
    }
    Ok(updated)
}

fn normalize_remote_control_state_rollout_paths_for_ids(
    connection: &Connection,
    source_home: &Path,
    target_home: &Path,
    thread_ids: &HashSet<String>,
) -> Result<usize, String> {
    let mut updated = 0;
    for thread_id in thread_ids {
        let rollout_path = match connection.query_row(
            "SELECT rollout_path FROM threads WHERE id = ?1",
            [thread_id],
            |row| row.get::<_, String>(0),
        ) {
            Ok(path) => path,
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(err) => return Err(format!("读取远程控制历史目标 rollout_path 失败: {err}")),
        };
        let mapped = map_remote_control_history_path(&rollout_path, source_home, target_home);
        if mapped == rollout_path {
            continue;
        }
        connection
            .execute(
                "UPDATE threads SET rollout_path = ?1 WHERE id = ?2",
                params![mapped, thread_id],
            )
            .map_err(|err| format!("更新远程控制历史目标 rollout_path 失败: {err}"))?;
        updated += 1;
    }
    Ok(updated)
}

fn sqlite_threads_recent_order_clause(column_set: &HashSet<String>) -> String {
    let mut terms = Vec::new();
    for column in ["updated_at_ms", "updated_at", "created_at_ms", "created_at"] {
        if column_set.contains(column) {
            terms.push(format!("{} DESC", quote_sqlite_identifier(column)));
        }
    }
    terms.push(format!("{} DESC", quote_sqlite_identifier("id")));
    format!(" ORDER BY {}", terms.join(", "))
}

fn sqlite_table_columns(connection: &Connection, table: &str) -> Result<Vec<String>, String> {
    let sql = format!("PRAGMA table_info({})", quote_sqlite_identifier(table));
    let mut statement = connection
        .prepare(&sql)
        .map_err(|err| format!("读取 SQLite 表结构失败 {table}: {err}"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| format!("查询 SQLite 表结构失败 {table}: {err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("读取 SQLite 表结构失败 {table}: {err}"))?;
    Ok(columns)
}

fn quote_sqlite_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn map_remote_control_history_path(path: &str, source_home: &Path, target_home: &Path) -> String {
    let source_home = source_home.to_string_lossy().replace('/', "\\");
    let target_home = target_home.to_string_lossy().replace('/', "\\");
    let (verbatim_prefix, comparable_path) = path
        .strip_prefix(r"\\?\")
        .map(|stripped| (r"\\?\", stripped))
        .unwrap_or(("", path));
    let comparable_path = comparable_path.replace('/', "\\");
    if comparable_path.len() >= source_home.len()
        && comparable_path[..source_home.len()].eq_ignore_ascii_case(&source_home)
        && comparable_path[source_home.len()..]
            .chars()
            .next()
            .is_none_or(|ch| ch == '\\' || ch == '/')
    {
        return format!(
            "{verbatim_prefix}{target_home}{}",
            &comparable_path[source_home.len()..]
        );
    }
    path.to_string()
}

pub(crate) fn prepare_remote_control_subscription_home() -> Result<PathBuf, String> {
    let settings = read_settings_value()?;
    let account_id = remote_control_account_id_from_settings(&settings);
    if account_id.is_empty() {
        return Err("app远程控制需要先单独选择一个订阅账号".to_string());
    }
    let tokens = remote_control_account_tokens(&account_id)?;
    let (api_base_url, api_key, model) =
        remote_control_api_session_profile_from_settings(&settings)?;
    let config_text = remote_control_subscription_config_text(&api_base_url, &model);
    let home = remote_control_subscription_home_dir()?;
    fs::create_dir_all(&home).map_err(|err| format!("创建远程控制订阅 home 失败: {err}"))?;
    fs::write(
        home.join("auth.json"),
        serde_json::to_string_pretty(&json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": api_key,
            "tokens": tokens,
            "last_refresh": now_string()
        }))
        .map_err(|err| format!("生成远程控制订阅 auth.json 失败: {err}"))?,
    )
    .map_err(|err| format!("写入远程控制订阅 auth.json 失败: {err}"))?;
    fs::write(home.join("config.toml"), config_text)
        .map_err(|err| format!("写入远程控制订阅 config.toml 失败: {err}"))?;
    sync_remote_control_history_from_root(&home)?;
    log_session_sync_event(
        "codex_remote_control_subscription_home_prepared",
        json!({
            "path": home,
            "accountId": account_id,
            "sessionProvider": API_PROVIDER_ID,
            "apiBaseUrl": api_base_url
        }),
    );
    Ok(home)
}

fn remove_remote_control_subscription_home() -> Result<bool, String> {
    let home = remote_control_subscription_home_dir()?;
    if !home.exists() {
        return Ok(false);
    }
    sync_remote_control_history_from_root(&home)?;
    fs::remove_dir_all(&home).map_err(|err| format!("删除远程控制订阅 home 失败: {err}"))?;
    log_session_sync_event(
        "codex_remote_control_subscription_home_removed",
        json!({
            "path": home
        }),
    );
    Ok(true)
}

fn remote_control_helper_process_state() -> &'static Mutex<Option<RemoteControlHelperProcess>> {
    REMOTE_CONTROL_HELPER_PROCESS.get_or_init(|| Mutex::new(None))
}

fn current_remote_control_helper_running() -> bool {
    let Ok(state) = remote_control_helper_process_state().lock() else {
        return false;
    };
    state
        .as_ref()
        .is_some_and(|current| remote_control_helper_is_running(current.pid))
}

fn root_remote_control_config_present() -> Result<bool, String> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(false);
    }
    let raw = fs::read_to_string(&path)
        .map_err(|err| format!("读取 config.toml 失败 {}: {err}", path.display()))?;
    let mut in_features = false;
    for line in raw.lines() {
        let normalized = line.trim();
        if normalized.starts_with('[') && normalized.ends_with(']') {
            in_features = normalized == format!("[{REMOTE_CONTROL_FEATURES_TABLE}]");
            continue;
        }
        if !in_features || normalized.is_empty() || normalized.starts_with('#') {
            continue;
        }
        let Some((key, _value)) = normalized.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key == REMOTE_CONTROL_CONFIG_KEY || key == LEGACY_REMOTE_CONNECTIONS_CONFIG_KEY {
            return Ok(true);
        }
    }
    Ok(false)
}

fn remote_control_subscription_home_ready(home: &Path) -> bool {
    home.exists() && home.join("auth.json").exists() && home.join("config.toml").exists()
}

fn remote_control_runtime_needs_rebuild(
    enabled: bool,
    helper_running: bool,
    unmanaged_helper_running: bool,
    subscription_home_exists: bool,
    subscription_home_ready: bool,
    root_config_present: bool,
) -> bool {
    if enabled {
        !helper_running || !subscription_home_ready || root_config_present
    } else {
        helper_running
            || unmanaged_helper_running
            || subscription_home_exists
            || root_config_present
    }
}

pub(crate) fn preview_remote_control_runtime_for_current_settings(
    _context: &str,
) -> Result<bool, String> {
    let settings = read_settings_value()?;
    let enabled = remote_control_enabled_from_settings(&settings);
    if enabled {
        validate_remote_control_runtime_preview_prerequisites(&settings)?;
    }
    let helper_running = current_remote_control_helper_running();
    let unmanaged_helper_running = if helper_running {
        false
    } else {
        !remote_control_helper_pids().is_empty()
    };
    let home = remote_control_subscription_home_dir()?;
    let subscription_home_exists = home.exists();
    let subscription_home_ready = remote_control_subscription_home_ready(&home);
    let root_config_present = root_remote_control_config_present()?;

    Ok(remote_control_runtime_needs_rebuild(
        enabled,
        helper_running,
        unmanaged_helper_running,
        subscription_home_exists,
        subscription_home_ready,
        root_config_present,
    ))
}

fn write_remote_control_marker_event(event: &str, details: Value) {
    let Ok(path) = remote_control_runtime_marker_path() else {
        return;
    };
    if ensure_parent_dir(&path).is_err() {
        return;
    }
    let mut object = serde_json::Map::new();
    object.insert("event".to_string(), Value::String(event.to_string()));
    object.insert("pid".to_string(), json!(std::process::id()));
    object.insert("time".to_string(), Value::String(now_string()));
    if let Some(details) = details.as_object() {
        for (key, value) in details {
            object.insert(key.clone(), value.clone());
        }
    }
    let line = format!("{}\n", Value::Object(object));
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(line.as_bytes()));
}

fn remote_control_helper_error(message: &str, details: Value) {
    write_remote_control_marker_event(
        REMOTE_CONTROL_HELPER_ERROR_EVENT,
        json!({
            "message": message,
            "details": details
        }),
    );
}

fn record_remote_control_helper_status(params: &Value) {
    let status = params
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    write_remote_control_marker_event(
        REMOTE_CONTROL_HELPER_STATUS_EVENT,
        json!({
            "status": status,
            "serverName": params.get("serverName").cloned().unwrap_or(Value::String(String::new())),
            "installationId": params.get("installationId").cloned().unwrap_or(Value::String(String::new())),
            "environmentId": params.get("environmentId").cloned().unwrap_or(Value::Null)
        }),
    );
    if status == "errored" {
        write_remote_control_marker_event(
            REMOTE_CONTROL_BACKEND_ERROR_EVENT,
            json!({
                "kind": "enrollment_failed",
                "text": "remoteControl/status returned errored"
            }),
        );
    }
}

fn remote_control_helper_is_running(pid: u64) -> bool {
    get_alive_pids(&[pid]).contains(&pid)
}

fn remote_control_helper_pids() -> Vec<u64> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let script = r#"
$ErrorActionPreference = "Stop"
$helpers = Get-CimInstance Win32_Process | Where-Object {
  $_.Name -ieq "codex.exe" `
    -and $_.CommandLine -match "\bapp-server\b" `
    -and $_.CommandLine -match "--listen\s+ws://127\.0\.0\.1:" `
    -and $_.CommandLine -match "--enable\s+remote_control"
} | Select-Object -ExpandProperty ProcessId
$helpers | ConvertTo-Json -Depth 2 -Compress
"#;
    run_pwsh(script)
        .ok()
        .and_then(|output| parse_json_output(&output, json!([])).ok())
        .map(json_as_array)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_u64())
        .collect()
}

fn stop_stale_remote_control_helpers(context: &str, keep_pid: Option<u64>) -> usize {
    let stale_pids = remote_control_helper_pids()
        .into_iter()
        .filter(|pid| Some(*pid) != keep_pid)
        .collect::<Vec<_>>();
    if stale_pids.is_empty() {
        return 0;
    }

    let stopped_pids = stale_pids
        .into_iter()
        .filter(|pid| kill_process_tree(*pid))
        .collect::<Vec<_>>();
    let stopped_count = stopped_pids.len();
    if !stopped_pids.is_empty() {
        let stopped_pids_for_log = stopped_pids.clone();
        log_session_sync_event(
            "codex_remote_control_helper_stale_stop",
            json!({
                "context": context,
                "pids": stopped_pids_for_log,
                "count": stopped_count
            }),
        );
        write_remote_control_marker_event(
            "remote_control_helper_stale_stop",
            json!({
                "pids": stopped_pids,
                "count": stopped_count
            }),
        );
    }
    stopped_count
}

fn clear_remote_control_helper_process(pid: u64) {
    if let Ok(mut state) = remote_control_helper_process_state().lock() {
        if state.as_ref().is_some_and(|current| current.pid == pid) {
            *state = None;
        }
    }
}

fn select_remote_control_helper_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|err| format!("分配远程控制 helper 端口失败: {err}"))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|err| format!("读取远程控制 helper 端口失败: {err}"))
}

fn remote_control_helper_proxy_url_from_settings(
    settings: &Value,
) -> Result<Option<String>, String> {
    if !bool_field(settings, "codex_proxy_env_enabled") {
        return Ok(None);
    }

    let proxy_url = normalize_proxy_url(&string_field(settings, "codex_proxy_url"))?;
    if proxy_url.is_empty() {
        return Err("app远程控制代理已启用，但代理地址为空".to_string());
    }
    Ok(Some(proxy_url))
}

fn write_remote_control_helper_proxy_env(
    home: &Path,
    proxy_url: Option<&str>,
) -> Result<bool, String> {
    let path = home.join(".env");
    let Some(proxy_url) = proxy_url else {
        if path.exists() {
            fs::remove_file(&path).map_err(|err| {
                format!("清理远程控制 helper 代理配置失败 {}: {err}", path.display())
            })?;
        }
        return Ok(false);
    };

    let content = format!(
        "HTTP_PROXY={proxy_url}\nHTTPS_PROXY={proxy_url}\nALL_PROXY={proxy_url}\nNO_PROXY={REMOTE_CONTROL_HELPER_NO_PROXY_VALUE}\n"
    );
    fs::write(&path, content)
        .map_err(|err| format!("写入远程控制 helper 代理配置失败 {}: {err}", path.display()))?;
    Ok(true)
}

fn apply_remote_control_helper_proxy_env(command: &mut Command, proxy_url: Option<&str>) -> bool {
    let Some(proxy_url) = proxy_url else {
        return false;
    };

    command
        .env("HTTP_PROXY", proxy_url)
        .env("HTTPS_PROXY", proxy_url)
        .env("ALL_PROXY", proxy_url)
        .env("NO_PROXY", REMOTE_CONTROL_HELPER_NO_PROXY_VALUE)
        .env("http_proxy", proxy_url)
        .env("https_proxy", proxy_url)
        .env("all_proxy", proxy_url)
        .env("no_proxy", REMOTE_CONTROL_HELPER_NO_PROXY_VALUE);
    true
}

fn remote_control_api_key_from_home(home: &Path) -> Result<String, String> {
    let path = home.join("auth.json");
    let text = fs::read_to_string(&path)
        .map_err(|err| format!("读取远程控制订阅 auth.json 失败 {}: {err}", path.display()))?;
    let auth: Value = serde_json::from_str(&text)
        .map_err(|err| format!("解析远程控制订阅 auth.json 失败 {}: {err}", path.display()))?;
    let api_key = string_field(&auth, "OPENAI_API_KEY");
    if api_key.trim().is_empty() {
        return Err("app远程控制会话流量走 API 需要先配置 API Key".to_string());
    }
    Ok(api_key)
}

fn remote_control_state_db_path(home: &Path) -> PathBuf {
    home.join("state_5.sqlite")
}

fn cleanup_remote_control_relay_enrollments(home: &Path) -> Result<usize, String> {
    let path = remote_control_state_db_path(home);
    if !path.exists() {
        return Ok(0);
    }
    let connection = Connection::open(&path).map_err(|err| {
        format!(
            "打开远程控制 enrollment state DB 失败 {}: {err}",
            path.display()
        )
    })?;
    let deleted = connection
        .execute(
            "DELETE FROM remote_control_enrollments
             WHERE websocket_url LIKE ?1",
            [format!("ws://127.0.0.1:%{REMOTE_CONTROL_UPSTREAM_PATH}")],
        )
        .map_err(|err| format!("清理远程控制本地 relay enrollment 失败: {err}"))?;
    if deleted > 0 {
        write_remote_control_marker_event(
            "remote_control_relay_enrollment_cleanup",
            json!({
                "deleted": deleted,
                "upstream": REMOTE_CONTROL_UPSTREAM_WS_URL
            }),
        );
    }
    Ok(deleted)
}

fn remote_control_codex_cli_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CODEX_CLI_PATH").map(PathBuf::from) {
        if path.exists() {
            return Ok(path);
        }
    }

    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "LOCALAPPDATA 环境变量不存在，无法定位 Codex CLI".to_string())?;
    let bin_dir = local_app_data.join("OpenAI").join("Codex").join("bin");
    let mut candidates = Vec::<(std::time::SystemTime, PathBuf)>::new();
    if let Ok(entries) = fs::read_dir(&bin_dir) {
        for entry in entries.flatten() {
            let path = entry.path().join("codex.exe");
            if !path.exists() {
                continue;
            }
            let modified = fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            candidates.push((modified, path));
        }
    }
    candidates.sort_by_key(|(modified, _path)| *modified);
    candidates
        .pop()
        .map(|(_modified, path)| path)
        .ok_or_else(|| {
            format!(
                "未找到 Codex CLI binary: {}",
                bin_dir.join("*").join("codex.exe").display()
            )
        })
}

fn ensure_remote_control_helper_for_settings(
    context: &str,
    settings: &Value,
    prepared_home: Option<PathBuf>,
) -> Result<bool, String> {
    if !remote_control_enabled_from_settings(settings) {
        return stop_remote_control_helper(context);
    }

    if let Ok(mut state) = remote_control_helper_process_state().lock() {
        if let Some(current) = *state {
            if remote_control_helper_is_running(current.pid) {
                let stale_stopped = stop_stale_remote_control_helpers(context, Some(current.pid));
                log_session_sync_event(
                    "codex_remote_control_helper_keep_running",
                    json!({
                        "context": context,
                        "pid": current.pid,
                        "port": current.port,
                        "staleStopped": stale_stopped
                    }),
                );
                return Ok(false);
            }
            *state = None;
        }
    }

    stop_stale_remote_control_helpers(context, None);
    let home = match prepared_home {
        Some(home) => home,
        None => prepare_remote_control_subscription_home()?,
    };
    let api_key = remote_control_api_key_from_home(&home)?;
    let helper_proxy_url = remote_control_helper_proxy_url_from_settings(settings)?;
    let helper_proxy_enabled =
        write_remote_control_helper_proxy_env(&home, helper_proxy_url.as_deref())?;
    let stale_relay_enrollments_deleted = cleanup_remote_control_relay_enrollments(&home)?;
    let codex_cli = remote_control_codex_cli_path()?;
    let port = select_remote_control_helper_port()?;
    let args = vec![
        "app-server".to_string(),
        "--listen".to_string(),
        format!("ws://127.0.0.1:{port}"),
        "--analytics-default-enabled".to_string(),
        "--enable".to_string(),
        "remote_control".to_string(),
    ];
    let mut command = Command::new(&codex_cli);
    apply_remote_control_helper_proxy_env(&mut command, helper_proxy_url.as_deref());
    command
        .args(args)
        .env("CODEX_HOME", &home)
        .env(CODEX_REMOTE_CONTROL_HOME_ENV, &home)
        .env(CODEX_REMOTE_CONTROL_HELPER_ENV, "1")
        .env(
            CODEX_INTERNAL_ORIGINATOR_OVERRIDE_ENV,
            CODEX_DESKTOP_ORIGINATOR,
        )
        .env("OPENAI_API_KEY", api_key)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    hide_command_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|err| format!("启动远程控制 helper app-server 失败: {err}"))?;
    let pid = u64::from(child.id());
    let stderr = child.stderr.take();
    if let Ok(mut state) = remote_control_helper_process_state().lock() {
        *state = Some(RemoteControlHelperProcess { pid, port });
    }
    log_session_sync_event(
        "codex_remote_control_helper_spawn",
        json!({
            "context": context,
            "pid": pid,
            "port": port,
            "codexCli": codex_cli,
            "home": home,
            "proxyEnabled": helper_proxy_enabled,
            "staleRelayEnrollmentsDeleted": stale_relay_enrollments_deleted
        }),
    );
    write_remote_control_marker_event(
        REMOTE_CONTROL_HELPER_SPAWN_EVENT,
        json!({
            "pid": pid,
            "port": port,
            "command": codex_cli.to_string_lossy(),
            "home": home.to_string_lossy(),
            "proxyEnabled": helper_proxy_enabled,
            "staleRelayEnrollmentsDeleted": stale_relay_enrollments_deleted
        }),
    );
    if let Some(stderr) = stderr {
        thread::spawn(move || watch_remote_control_helper_stderr(stderr));
    }
    thread::spawn(move || run_remote_control_helper_ws(port));
    thread::spawn(move || {
        let status = child.wait();
        sync_remote_control_history_for_enabled_settings("remote_control_helper_exit");
        write_remote_control_marker_event(
            "remote_control_helper_exit",
            json!({
                "pid": pid,
                "status": status.as_ref().map(|status| status.to_string()).unwrap_or_else(|err| err.to_string())
            }),
        );
        clear_remote_control_helper_process(pid);
    });
    Ok(true)
}

pub(crate) fn stop_remote_control_helper(context: &str) -> Result<bool, String> {
    let current = remote_control_helper_process_state()
        .lock()
        .map_err(|_| "远程控制 helper 状态已损坏".to_string())?
        .take();
    let Some(current) = current else {
        return Ok(stop_stale_remote_control_helpers(context, None) > 0);
    };
    let stopped = kill_process_tree(current.pid);
    let stale_stopped = stop_stale_remote_control_helpers(context, Some(current.pid));
    log_session_sync_event(
        "codex_remote_control_helper_stop",
        json!({
            "context": context,
            "pid": current.pid,
            "stopped": stopped,
            "staleStopped": stale_stopped
        }),
    );
    write_remote_control_marker_event(
        "remote_control_helper_stop",
        json!({
            "pid": current.pid,
            "stopped": stopped
        }),
    );
    Ok(stopped)
}

fn watch_remote_control_helper_stderr(mut stderr: impl Read) {
    let mut buffer = [0u8; 1024];
    loop {
        let read = match stderr.read(&mut buffer) {
            Ok(0) => return,
            Ok(read) => read,
            Err(_) => return,
        };
        let text = String::from_utf8_lossy(&buffer[..read]).to_string();
        if let Some((kind, _message)) = remote_control_backend_error_message(None, &text) {
            write_remote_control_marker_event(
                REMOTE_CONTROL_BACKEND_ERROR_EVENT,
                json!({
                    "kind": kind,
                    "text": truncate_remote_control_error_text(&text)
                }),
            );
        }
    }
}

fn run_remote_control_helper_ws(port: u16) {
    thread::sleep(StdDuration::from_millis(
        REMOTE_CONTROL_HELPER_CONNECT_DELAY_MS,
    ));
    let mut stream = match connect_remote_control_helper_ws(port) {
        Ok(stream) => stream,
        Err(err) => {
            remote_control_helper_error("websocket_connect_failed", json!({ "error": err }));
            return;
        }
    };
    write_remote_control_marker_event(
        REMOTE_CONTROL_HELPER_WS_CONNECTED_EVENT,
        json!({ "port": port }),
    );
    let mut next_id = 1i64;
    for (method, params) in [
        (
            "initialize",
            json!({
                "clientInfo": {
                    "name": "codex-switch-remote-control",
                    "title": "Codex Switch Remote Control",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "experimentalApi": true,
                    "requestAttestation": false
                }
            }),
        ),
        (
            "experimentalFeature/enablement/set",
            json!({ "enablement": { "remote_control": true } }),
        ),
        ("remoteControl/enable", Value::Null),
        ("remoteControl/status/read", Value::Null),
    ] {
        if let Err(err) =
            send_remote_control_helper_request(&mut stream, &mut next_id, method, params)
        {
            remote_control_helper_error("websocket_send_failed", json!({ "error": err }));
            return;
        }
    }

    let mut last_poll = Instant::now();
    loop {
        if last_poll.elapsed() >= StdDuration::from_millis(REMOTE_CONTROL_HELPER_STATUS_POLL_MS) {
            if let Err(err) = send_remote_control_helper_request(
                &mut stream,
                &mut next_id,
                "remoteControl/status/read",
                Value::Null,
            ) {
                remote_control_helper_error("websocket_poll_failed", json!({ "error": err }));
                return;
            }
            last_poll = Instant::now();
        }
        match read_remote_control_ws_text(&mut stream) {
            Ok(Some(message)) => handle_remote_control_helper_message(&message),
            Ok(None) => {}
            Err(err) => {
                remote_control_helper_error("websocket_read_failed", json!({ "error": err }));
                return;
            }
        }
    }
}

fn connect_remote_control_helper_ws(port: u16) -> Result<TcpStream, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .map_err(|err| format!("连接远程控制 helper WebSocket 失败: {err}"))?;
    stream
        .set_read_timeout(Some(StdDuration::from_millis(
            REMOTE_CONTROL_HELPER_WS_READ_TIMEOUT_MS,
        )))
        .map_err(|err| format!("设置远程控制 helper read timeout 失败: {err}"))?;
    stream
        .set_write_timeout(Some(StdDuration::from_secs(5)))
        .map_err(|err| format!("设置远程控制 helper write timeout 失败: {err}"))?;
    let request = format!(
        "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: AAAAAAAAAAAAAAAAAAAAAA==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("发送远程控制 helper WebSocket 握手失败: {err}"))?;
    let mut response = Vec::new();
    let mut buffer = [0u8; 512];
    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|err| format!("读取远程控制 helper WebSocket 握手失败: {err}"))?;
        if read == 0 {
            return Err("远程控制 helper WebSocket 握手提前关闭".to_string());
        }
        response.extend_from_slice(&buffer[..read]);
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if response.len() > 8192 {
            return Err("远程控制 helper WebSocket 握手响应过大".to_string());
        }
    }
    let response_text = String::from_utf8_lossy(&response);
    if !response_text.starts_with("HTTP/1.1 101") && !response_text.starts_with("HTTP/1.0 101") {
        return Err("远程控制 helper WebSocket 握手未升级协议".to_string());
    }
    Ok(stream)
}

fn send_remote_control_helper_request(
    stream: &mut TcpStream,
    next_id: &mut i64,
    method: &str,
    params: Value,
) -> Result<(), String> {
    let id = *next_id;
    *next_id += 1;
    let message = if params.is_null() {
        json!({ "id": id, "method": method })
    } else {
        json!({ "id": id, "method": method, "params": params })
    };
    send_remote_control_ws_text(stream, &message.to_string())
}

fn send_remote_control_ws_text(stream: &mut TcpStream, text: &str) -> Result<(), String> {
    let payload = text.as_bytes();
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x81);
    if payload.len() < 126 {
        frame.push(0x80 | payload.len() as u8);
    } else if payload.len() <= u16::MAX as usize {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    let mask = [0x13u8, 0x37, 0x5a, 0xc0];
    frame.extend_from_slice(&mask);
    for (index, byte) in payload.iter().enumerate() {
        frame.push(byte ^ mask[index % 4]);
    }
    stream
        .write_all(&frame)
        .map_err(|err| format!("发送远程控制 helper WebSocket frame 失败: {err}"))
}

fn read_remote_control_ws_text(stream: &mut TcpStream) -> Result<Option<String>, String> {
    loop {
        let mut header = [0u8; 2];
        if let Err(err) = stream.read_exact(&mut header) {
            return if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) {
                Ok(None)
            } else {
                Err(format!("读取远程控制 helper WebSocket frame 失败: {err}"))
            };
        }
        let opcode = header[0] & 0x0f;
        let masked = header[1] & 0x80 != 0;
        let mut len = u64::from(header[1] & 0x7f);
        if len == 126 {
            let mut buffer = [0u8; 2];
            stream
                .read_exact(&mut buffer)
                .map_err(|err| format!("读取远程控制 helper frame 长度失败: {err}"))?;
            len = u64::from(u16::from_be_bytes(buffer));
        } else if len == 127 {
            let mut buffer = [0u8; 8];
            stream
                .read_exact(&mut buffer)
                .map_err(|err| format!("读取远程控制 helper frame 长度失败: {err}"))?;
            len = u64::from_be_bytes(buffer);
        }
        let mut mask = [0u8; 4];
        if masked {
            stream
                .read_exact(&mut mask)
                .map_err(|err| format!("读取远程控制 helper frame mask 失败: {err}"))?;
        }
        if len > 16 * 1024 * 1024 {
            return Err("远程控制 helper WebSocket frame 过大".to_string());
        }
        let mut payload = vec![0u8; len as usize];
        stream
            .read_exact(&mut payload)
            .map_err(|err| format!("读取远程控制 helper payload 失败: {err}"))?;
        if masked {
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % 4];
            }
        }
        match opcode {
            0x1 => {
                return String::from_utf8(payload)
                    .map(Some)
                    .map_err(|err| format!("远程控制 helper WebSocket 文本不是 UTF-8: {err}"));
            }
            0x8 => return Err("远程控制 helper WebSocket 已关闭".to_string()),
            0x9 => send_remote_control_ws_frame(stream, 0xA, &payload)?,
            0xA => {}
            _ => {}
        }
    }
}

fn send_remote_control_ws_frame(
    stream: &mut TcpStream,
    opcode: u8,
    payload: &[u8],
) -> Result<(), String> {
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x80 | opcode);
    if payload.len() < 126 {
        frame.push(0x80 | payload.len() as u8);
    } else if payload.len() <= u16::MAX as usize {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    let mask = [0x13u8, 0x37, 0x5a, 0xc0];
    frame.extend_from_slice(&mask);
    for (index, byte) in payload.iter().enumerate() {
        frame.push(byte ^ mask[index % 4]);
    }
    stream
        .write_all(&frame)
        .map_err(|err| format!("发送远程控制 helper WebSocket frame 失败: {err}"))
}

fn handle_remote_control_helper_message(text: &str) {
    let Ok(message) = serde_json::from_str::<Value>(text) else {
        return;
    };
    if message.get("method").and_then(Value::as_str) == Some("remoteControl/status/changed") {
        if let Some(params) = message.get("params") {
            record_remote_control_helper_status(params);
            sync_remote_control_history_for_enabled_settings("remote_control_status_changed");
        }
        return;
    }
    if let Some(error) = message.get("error") {
        let text = error.to_string();
        let kind = remote_control_backend_error_message(None, &text)
            .map(|(kind, _message)| kind)
            .unwrap_or("enrollment_failed");
        write_remote_control_marker_event(
            REMOTE_CONTROL_BACKEND_ERROR_EVENT,
            json!({
                "kind": kind,
                "text": truncate_remote_control_error_text(&text)
            }),
        );
        return;
    }
    if let Some(result) = message.get("result") {
        if result.get("status").and_then(Value::as_str).is_some() {
            record_remote_control_helper_status(result);
            sync_remote_control_history_for_enabled_settings("remote_control_status_read");
        }
    }
}

fn sync_remote_control_history_for_enabled_settings(trigger: &str) {
    let result = (|| -> Result<(), String> {
        let settings = read_settings_value()?;
        if !remote_control_enabled_from_settings(&settings) {
            return Ok(());
        }
        let home = remote_control_subscription_home_dir()?;
        if !home.exists() {
            return Ok(());
        }
        sync_remote_control_history_from_root(&home)?;
        Ok(())
    })();

    if let Err(err) = result {
        log_session_sync_event(
            "codex_remote_control_history_sync_error",
            json!({
                "trigger": trigger,
                "error": err
            }),
        );
    }
}

pub(crate) fn sync_remote_control_runtime_for_current_settings(
    context: &str,
) -> Result<bool, String> {
    let settings = read_settings_value()?;
    let enabled = remote_control_enabled_from_settings(&settings);
    let changed = if enabled {
        let home = prepare_remote_control_subscription_home()?;
        let helper_changed =
            ensure_remote_control_helper_for_settings(context, &settings, Some(home))?;
        let root_config_changed = remove_remote_control_config()?;
        helper_changed || root_config_changed
    } else {
        stop_remote_control_helper(context)?;
        let home_removed = remove_remote_control_subscription_home()?;
        remove_remote_control_config()? || home_removed
    };
    if changed {
        log_session_sync_event(
            "codex_remote_control_runtime_applied",
            json!({
                "context": context,
                "remoteControl": enabled
            }),
        );
    }
    Ok(changed)
}

pub(crate) fn restart_remote_control_runtime_for_current_settings(
    context: &str,
) -> Result<bool, String> {
    let settings = read_settings_value()?;
    if !remote_control_enabled_from_settings(&settings) {
        return Ok(false);
    }
    let stopped = stop_remote_control_helper(context)?;
    let synced = sync_remote_control_runtime_for_current_settings(context)?;
    Ok(stopped || synced)
}

pub(crate) fn remote_control_runtime_marker_path() -> Result<PathBuf, String> {
    Ok(crate::paths::app_data_dir()?
        .join("hooks")
        .join("codex-remote-control-runtime.jsonl"))
}

#[tauri::command]
pub(crate) fn get_codex_remote_control_status() -> Result<Value, String> {
    let settings = read_settings_value()?;
    let enabled = remote_control_enabled_from_settings(&settings);
    let account_id = remote_control_account_id_from_settings(&settings);
    let marker_path = remote_control_runtime_marker_path()?;
    let marker_text = fs::read_to_string(&marker_path).ok();
    let backend_error = marker_text
        .as_deref()
        .and_then(latest_remote_control_backend_error);
    let helper_status = marker_text
        .as_deref()
        .and_then(latest_remote_control_helper_status);
    let backend_environment = if backend_error.is_none() {
        helper_status
            .as_ref()
            .and_then(remote_control_backend_environment_status_for_helper)
    } else {
        None
    };
    let connection_status = remote_control_connection_status(
        enabled,
        backend_error.as_ref(),
        helper_status.as_ref(),
        backend_environment.as_ref(),
    );

    Ok(json!({
        "ok": true,
        "enabled": enabled,
        "accountId": account_id,
        "markerPath": marker_path,
        "backendError": backend_error,
        "helperStatus": helper_status,
        "backendEnvironment": backend_environment,
        "connectionStatus": connection_status
    }))
}

fn latest_remote_control_backend_error(marker_text: &str) -> Option<Value> {
    let events = marker_text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect::<Vec<_>>();
    let start_index = events
        .iter()
        .rposition(|event| {
            matches!(
                event.get("event").and_then(Value::as_str),
                Some("loaded" | "app_server_spawn")
            )
        })
        .map(|index| index + 1)
        .unwrap_or(0);

    let mut generic_error = None;
    for index in (start_index..events.len()).rev() {
        let Some(error) = remote_control_backend_error_from_marker(events[index].clone()) else {
            continue;
        };
        if remote_control_backend_error_recovered_by_later_status(&events[index + 1..]) {
            return None;
        }
        if error.get("kind").and_then(Value::as_str) == Some("enrollment_failed") {
            generic_error.get_or_insert(error);
            continue;
        }
        return Some(error);
    }

    generic_error
}

fn latest_remote_control_helper_status(marker_text: &str) -> Option<Value> {
    let events = marker_text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect::<Vec<_>>();
    let start_index = events
        .iter()
        .rposition(|event| event.get("event").and_then(Value::as_str) == Some("loaded"))
        .map(|index| index + 1)
        .unwrap_or(0);

    events
        .into_iter()
        .skip(start_index)
        .rev()
        .find_map(remote_control_helper_status_from_marker)
}

fn remote_control_helper_status_from_marker(value: Value) -> Option<Value> {
    let event = value.get("event").and_then(Value::as_str)?;
    match event {
        REMOTE_CONTROL_HELPER_STATUS_EVENT => {
            let status = value
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            Some(json!({
                "status": status,
                "message": remote_control_helper_status_message(status),
                "serverName": value.get("serverName").cloned().unwrap_or(Value::Null),
                "installationId": value.get("installationId").cloned().unwrap_or(Value::Null),
                "environmentId": value.get("environmentId").cloned().unwrap_or(Value::Null),
                "time": value.get("time").cloned().unwrap_or(Value::Null)
            }))
        }
        REMOTE_CONTROL_HELPER_ERROR_EVENT => Some(json!({
            "status": "errored",
            "message": "app远程控制 helper 启动失败。",
            "raw": value.get("message").cloned().unwrap_or(Value::Null),
            "time": value.get("time").cloned().unwrap_or(Value::Null)
        })),
        REMOTE_CONTROL_HELPER_WS_CONNECTED_EVENT => Some(json!({
            "status": "starting",
            "message": "app远程控制 helper 已连接本地 app-server，正在 enrollment。",
            "time": value.get("time").cloned().unwrap_or(Value::Null)
        })),
        REMOTE_CONTROL_HELPER_SPAWN_EVENT => Some(json!({
            "status": "starting",
            "message": "app远程控制 helper 已拉起，正在连接本地 app-server。",
            "time": value.get("time").cloned().unwrap_or(Value::Null)
        })),
        _ => None,
    }
}

fn remote_control_helper_status_message(status: &str) -> &'static str {
    match status {
        "connected" => "桌面端已连上 ChatGPT，可用 app 控制。",
        "connecting" => "桌面端正在连接 ChatGPT 远程控制服务。",
        "errored" => "app远程控制 helper enrollment 失败。",
        "disabled" => "app远程控制尚未启用。",
        _ => "app远程控制 helper 状态未知。",
    }
}

fn remote_control_backend_environment_status_for_helper(helper_status: &Value) -> Option<Value> {
    if helper_status.get("status").and_then(Value::as_str) != Some("connected") {
        return None;
    }

    match fetch_remote_control_backend_environment_status(helper_status) {
        Ok(status) => status,
        Err(err) => Some(json!({
            "status": "lookup_failed",
            "message": "桌面端已连上 ChatGPT，app 状态读取失败。",
            "raw": truncate_remote_control_error_text(&err)
        })),
    }
}

fn fetch_remote_control_backend_environment_status(
    helper_status: &Value,
) -> Result<Option<Value>, String> {
    let environment_id = string_field(helper_status, "environmentId");
    if environment_id.is_empty() {
        return Ok(None);
    }

    let home = remote_control_subscription_home_dir()?;
    let auth_path = home.join("auth.json");
    let auth_text = fs::read_to_string(&auth_path).map_err(|err| {
        format!(
            "读取远程控制订阅 auth.json 失败 {}: {err}",
            auth_path.display()
        )
    })?;
    let auth: Value = serde_json::from_str(&auth_text).map_err(|err| {
        format!(
            "解析远程控制订阅 auth.json 失败 {}: {err}",
            auth_path.display()
        )
    })?;
    let tokens = auth
        .get("tokens")
        .ok_or_else(|| "远程控制订阅 auth.json 缺少 tokens".to_string())?;
    let access_token = string_field(tokens, "access_token");
    if access_token.is_empty() {
        return Err("远程控制订阅 auth.json 缺少 tokens.access_token".to_string());
    }
    let account_id = string_field(tokens, "account_id");
    let installation_id = string_field(helper_status, "installationId");

    let client = reqwest::blocking::Client::builder()
        .timeout(StdDuration::from_millis(
            REMOTE_CONTROL_BACKEND_STATUS_TIMEOUT_MS,
        ))
        .build()
        .map_err(|err| format!("创建 ChatGPT 远程控制状态客户端失败: {err}"))?;
    let mut request = client
        .get(REMOTE_CONTROL_ENVIRONMENTS_ENDPOINT)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header(
            "User-Agent",
            format!("codex-switch/{}", env!("CARGO_PKG_VERSION")),
        );
    if !account_id.is_empty() {
        request = request.header("chatgpt-account-id", account_id);
    }
    if !installation_id.is_empty() {
        request = request.header("x-codex-installation-id", installation_id);
    }

    let response = request
        .send()
        .map_err(|err| format!("读取 ChatGPT 远程控制环境失败: {err}"))?;
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        let raw = format!("HTTP {} body: {text}", status.as_u16());
        if let Some((kind, message)) = remote_control_backend_error_message(None, &raw) {
            return Ok(Some(json!({
                "status": "errored",
                "kind": kind,
                "message": message,
                "raw": truncate_remote_control_error_text(&raw)
            })));
        }
        return Err(format!(
            "读取 ChatGPT 远程控制环境失败: HTTP {}",
            status.as_u16()
        ));
    }

    let data: Value = serde_json::from_str(&text)
        .map_err(|err| format!("解析 ChatGPT 远程控制环境失败: {err}"))?;
    Ok(remote_control_backend_environment_summary_from_items(
        &data,
        &environment_id,
    ))
}

fn remote_control_backend_environment_summary_from_items(
    data: &Value,
    environment_id: &str,
) -> Option<Value> {
    let items = data.get("items").and_then(Value::as_array)?;
    let current = items
        .iter()
        .find(|item| item.get("env_id").and_then(Value::as_str) == Some(environment_id));

    let Some(current) = current else {
        return Some(json!({
            "status": "missing",
            "environmentId": environment_id,
            "message": "ChatGPT 后端没有找到当前桌面连接。"
        }));
    };

    let display_name = string_field(current, "display_name");
    let client_name = string_field(current, "client_name");
    let same_display_name_count = if display_name.is_empty() {
        0
    } else {
        items
            .iter()
            .filter(|item| string_field(item, "display_name") == display_name)
            .count()
    };
    let offline_same_display_name_count = if display_name.is_empty() {
        0
    } else {
        items
            .iter()
            .filter(|item| {
                string_field(item, "display_name") == display_name
                    && item.get("online").and_then(Value::as_bool) == Some(false)
            })
            .count()
    };

    Some(json!({
        "status": "found",
        "environmentId": environment_id,
        "displayName": display_name,
        "online": current.get("online").cloned().unwrap_or(Value::Null),
        "installationId": current.get("installation_id").cloned().unwrap_or(Value::Null),
        "clientType": current.get("client_type").cloned().unwrap_or(Value::Null),
        "originator": current.get("originator").cloned().unwrap_or(Value::Null),
        "clientName": client_name,
        "lastSeenAt": current.get("last_seen_at").cloned().unwrap_or(Value::Null),
        "appClientConnected": remote_control_client_name_indicates_app(&client_name),
        "sameDisplayNameCount": same_display_name_count,
        "offlineSameDisplayNameCount": offline_same_display_name_count
    }))
}

fn remote_control_client_name_indicates_app(client_name: &str) -> bool {
    let name = client_name.trim().to_ascii_lowercase();
    if name.is_empty() {
        return false;
    }
    if name.starts_with("codex-switch") {
        return false;
    }
    name.contains("chatgpt")
        || name.contains("ios")
        || name.contains("android")
        || name.contains("mobile")
        || name.contains("app")
}

fn remote_control_connection_status(
    enabled: bool,
    backend_error: Option<&Value>,
    helper_status: Option<&Value>,
    backend_environment: Option<&Value>,
) -> Value {
    if !enabled {
        return json!({
            "status": "disabled",
            "state": "muted",
            "message": "app远程控制尚未启用。"
        });
    }

    if let Some(error) = backend_error {
        let kind = error
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("backend_error");
        return json!({
            "status": kind,
            "state": remote_control_backend_issue_state(kind),
            "message": error.get("message").cloned().unwrap_or_else(|| json!("app远程控制后端连接失败。")),
            "raw": error.get("raw").cloned().unwrap_or(Value::Null)
        });
    }

    if let Some(environment) = backend_environment {
        match environment.get("status").and_then(Value::as_str) {
            Some("errored") => {
                let kind = environment
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("backend_error");
                return json!({
                    "status": kind,
                    "state": remote_control_backend_issue_state(kind),
                    "message": environment.get("message").cloned().unwrap_or_else(|| json!("app远程控制后端连接失败。")),
                    "raw": environment.get("raw").cloned().unwrap_or(Value::Null)
                });
            }
            Some("lookup_failed") => {
                return json!({
                    "status": "backend_lookup_failed",
                    "state": "muted",
                    "message": environment.get("message").cloned().unwrap_or_else(|| json!("桌面端已连上 ChatGPT，app 状态读取失败。")),
                    "raw": environment.get("raw").cloned().unwrap_or(Value::Null)
                });
            }
            Some("missing") => {
                return json!({
                    "status": "backend_environment_missing",
                    "state": "error",
                    "message": environment.get("message").cloned().unwrap_or_else(|| json!("ChatGPT 后端没有找到当前桌面连接。"))
                });
            }
            Some("found") => {
                if environment
                    .get("appClientConnected")
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    return json!({
                        "status": "app_recently_seen",
                        "state": "active",
                        "message": "服务正常。",
                        "raw": remote_control_environment_status_title(environment)
                    });
                }
                if environment.get("online").and_then(Value::as_bool) == Some(true) {
                    return json!({
                        "status": "desktop_ready",
                        "state": "active",
                        "message": "桌面端已在线，可用 app 控制。",
                        "raw": remote_control_environment_status_title(environment)
                    });
                }
                return json!({
                    "status": "backend_environment_offline",
                    "state": "error",
                    "message": "ChatGPT 后端显示当前桌面离线。",
                    "raw": remote_control_environment_status_title(environment)
                });
            }
            _ => {}
        }
    }

    if let Some(helper) = helper_status {
        let status = helper
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let state = if status == "errored" {
            "error"
        } else {
            "muted"
        };
        return json!({
            "status": status,
            "state": state,
            "message": helper.get("message").cloned().unwrap_or_else(|| json!(remote_control_helper_status_message(status))),
            "raw": helper.get("raw").cloned().unwrap_or(Value::Null)
        });
    }

    json!({
        "status": "unknown",
        "state": "muted",
        "message": "app远程控制状态未知。"
    })
}

fn remote_control_environment_status_title(environment: &Value) -> String {
    let mut parts = Vec::new();
    for (key, label) in [
        ("displayName", "设备"),
        ("environmentId", "environment"),
        ("clientName", "client"),
        ("lastSeenAt", "last_seen"),
    ] {
        let value = string_field(environment, key);
        if !value.is_empty() {
            parts.push(format!("{label}: {value}"));
        }
    }
    parts.join(" · ")
}

fn remote_control_backend_issue_state(kind: &str) -> &'static str {
    match kind {
        "login_expired" | "mfa_required" => "warning",
        _ => "error",
    }
}

fn remote_control_backend_error_recovered_by_later_status(events: &[Value]) -> bool {
    events
        .iter()
        .rev()
        .find(|event| {
            event.get("event").and_then(Value::as_str) == Some(REMOTE_CONTROL_HELPER_STATUS_EVENT)
        })
        .and_then(|event| event.get("status").and_then(Value::as_str))
        == Some("connected")
}

fn remote_control_backend_error_from_marker(value: Value) -> Option<Value> {
    if value.get("event").and_then(Value::as_str) != Some(REMOTE_CONTROL_BACKEND_ERROR_EVENT) {
        return None;
    }

    let raw_text = value.get("text").and_then(Value::as_str).unwrap_or("");
    let marker_kind = value.get("kind").and_then(Value::as_str);
    let (kind, message) = remote_control_backend_error_message(marker_kind, raw_text)?;
    Some(json!({
        "kind": kind,
        "message": message,
        "raw": truncate_remote_control_error_text(raw_text),
        "time": value.get("time").cloned().unwrap_or(Value::Null)
    }))
}

fn remote_control_backend_error_message(
    marker_kind: Option<&str>,
    raw_text: &str,
) -> Option<(&'static str, &'static str)> {
    let text = raw_text.to_ascii_lowercase();
    match marker_kind {
        Some("mfa_required") => Some(("mfa_required", "需要先为当前账号完成 MFA 认证。")),
        Some("login_expired") => Some(("login_expired", "控制账号登录已过期，请重新登录。")),
        Some("enrollment_failed") => Some(("enrollment_failed", "app远程控制 enrollment 失败。")),
        _ if text.contains("multi-factor authentication required") => {
            Some(("mfa_required", "需要先为当前账号完成 MFA 认证。"))
        }
        _ if text.contains("refresh_token_reused")
            || text.contains("refresh token has already been used")
            || text.contains("please log out and sign in again") =>
        {
            Some(("login_expired", "控制账号登录已过期，请重新登录。"))
        }
        _ if text.contains("remote control server enrollment failed")
            || text.contains("enrollment failed")
            || text.contains("http 403 forbidden") =>
        {
            Some(("enrollment_failed", "app远程控制 enrollment 失败。"))
        }
        _ => None,
    }
}

fn truncate_remote_control_error_text(text: &str) -> String {
    let text = text.trim();
    if text.len() <= REMOTE_CONTROL_BACKEND_ERROR_TEXT_MAX_LEN {
        return text.to_string();
    }
    format!(
        "{}...",
        &text[..text
            .char_indices()
            .take_while(|(index, _)| *index <= REMOTE_CONTROL_BACKEND_ERROR_TEXT_MAX_LEN)
            .last()
            .map(|(index, ch)| index + ch.len_utf8())
            .unwrap_or(REMOTE_CONTROL_BACKEND_ERROR_TEXT_MAX_LEN)]
    )
}

#[tauri::command]
pub(crate) fn set_codex_remote_control_enabled(enabled: bool) -> Result<Value, String> {
    let codex_app_running =
        !super::codex_app_watcher::refresh_current_codex_app_processes()?.is_empty();
    if enabled {
        validate_remote_control_enable_prerequisites()?;
    }
    let settings = update_settings_value(&json!({
        REMOTE_CONTROL_ENABLED_SETTING_KEY: enabled
    }))?;
    let settings = super::codex_app::apply_codex_proxy_env_state_to_settings(settings)?;
    let changed =
        sync_remote_control_runtime_for_current_settings("set_codex_remote_control_enabled")?;

    Ok(json!({
        "ok": true,
        "message": if enabled {
            if codex_app_running {
                "app远程控制已启用，远控服务正在独立启动"
            } else {
                "app远程控制已启用"
            }
        } else if codex_app_running {
            "app远程控制已关闭，远控服务已停止"
        } else {
            "app远程控制已关闭"
        },
        "settings": settings,
        "changed": changed,
        "configDeferred": false
    }))
}

#[tauri::command]
pub(crate) fn set_codex_remote_control_account_id(id: String) -> Result<Value, String> {
    let account_id = id.trim();
    validate_remote_control_account_id(account_id)?;

    let settings = update_settings_value(&json!({
        REMOTE_CONTROL_ACCOUNT_SETTING_KEY: account_id
    }))?;
    let settings = super::codex_app::apply_codex_proxy_env_state_to_settings(settings)?;
    let helper_restarted = if remote_control_enabled_from_settings(&settings) {
        restart_remote_control_runtime_for_current_settings("set_codex_remote_control_account_id")?
    } else {
        false
    };

    Ok(json!({
        "ok": true,
        "message": "app远程控制账号已更新",
        "settings": settings,
        "helperRestarted": helper_restarted
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env, fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static REMOTE_CONTROL_TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_remote_control_test_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let counter = REMOTE_CONTROL_TEST_DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
        env::temp_dir().join(format!(
            "codex-switch-remote-control-{name}-{}-{stamp}-{counter}",
            std::process::id()
        ))
    }

    fn create_remote_control_history_state_db(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let connection = Connection::open(path).unwrap();
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT NOT NULL,
                    model_provider TEXT NOT NULL,
                    title TEXT NOT NULL,
                    updated_at_ms INTEGER
                )",
                [],
            )
            .unwrap();
    }

    fn path_text_starts_with_home(path_text: &str, home: &Path) -> bool {
        let normalized_path = path_text.replace('\\', "/");
        let normalized_home = home.to_string_lossy().replace('\\', "/");
        normalized_path.starts_with(&normalized_home)
    }

    #[test]
    fn remote_control_runtime_rebuild_is_not_pending_when_enabled_runtime_is_current() {
        let pending = remote_control_runtime_needs_rebuild(true, true, false, true, true, false);

        assert!(!pending);
    }

    #[test]
    fn remote_control_runtime_rebuild_is_pending_when_enabled_helper_is_missing() {
        let pending = remote_control_runtime_needs_rebuild(true, false, false, true, true, false);

        assert!(pending);
    }

    #[test]
    fn remote_control_runtime_rebuild_is_pending_when_disabled_runtime_artifacts_remain() {
        let pending = remote_control_runtime_needs_rebuild(false, false, true, false, false, false);

        assert!(pending);
    }

    #[test]
    fn remote_control_account_id_from_settings_uses_configured_account() {
        let settings = json!({
            "codex_remote_control_account_id": "remote-account"
        });

        let account_id = remote_control_account_id_from_settings(&settings);

        assert_eq!(account_id, "remote-account");
    }

    #[test]
    fn remote_control_account_id_from_settings_does_not_fall_back_to_active_account() {
        let settings = json!({
            "codex_remote_control_account_id": ""
        });

        let account_id = remote_control_account_id_from_settings(&settings);

        assert_eq!(account_id, "");
    }

    #[test]
    fn remote_control_backend_error_message_identifies_mfa_required() {
        let (_kind, message) = remote_control_backend_error_message(
            None,
            r#"HTTP 403 Forbidden body: {"detail":"Multi-factor authentication required"}"#,
        )
        .expect("MFA error should be recognized");

        assert!(message.contains("MFA"));
    }

    #[test]
    fn remote_control_backend_error_message_marks_login_expired_as_account_action() {
        let (kind, message) = remote_control_backend_error_message(
            None,
            "refresh_token_reused: Please log out and sign in again.",
        )
        .expect("login expired error should be recognized");

        assert_eq!(kind, "login_expired");
        assert!(message.contains("登录"));
        assert!(!message.contains("失败"));
    }

    #[test]
    fn latest_remote_control_backend_error_uses_latest_marker_entry() {
        let marker_text = r#"{"event":"app_server_spawn","time":"2026-05-24T09:59:00.000Z"}
{"event":"remote_control_backend_error","time":"2026-05-24T10:00:00.000Z","kind":"enrollment_failed","text":"older"}
{"event":"remote_control_backend_error","time":"2026-05-24T10:01:00.000Z","kind":"mfa_required","text":"HTTP 403 Forbidden body: {\"detail\":\"Multi-factor authentication required\"}"}"#;

        let error = latest_remote_control_backend_error(marker_text)
            .expect("latest marker error should be parsed");

        assert_eq!(
            error.get("kind").and_then(Value::as_str),
            Some("mfa_required")
        );
        assert_eq!(
            error.get("time").and_then(Value::as_str),
            Some("2026-05-24T10:01:00.000Z")
        );
    }

    #[test]
    fn latest_remote_control_backend_error_prefers_specific_error_over_generic_poll() {
        let marker_text = r#"{"event":"app_server_spawn","time":"2026-05-24T09:59:00.000Z"}
{"event":"remote_control_backend_error","time":"2026-05-24T10:00:00.000Z","kind":"login_expired","text":"refresh_token_reused"}
{"event":"remote_control_helper_status","time":"2026-05-24T10:01:00.000Z","status":"errored"}
{"event":"remote_control_backend_error","time":"2026-05-24T10:01:01.000Z","kind":"enrollment_failed","text":"remoteControl/status returned errored"}"#;

        let error =
            latest_remote_control_backend_error(marker_text).expect("specific error should win");

        assert_eq!(
            error.get("kind").and_then(Value::as_str),
            Some("login_expired")
        );
    }

    #[test]
    fn latest_remote_control_backend_error_ignores_errors_before_latest_spawn() {
        let marker_text = r#"{"event":"remote_control_backend_error","time":"2026-05-24T10:00:00.000Z","kind":"mfa_required","text":"old"}
{"event":"app_server_spawn","time":"2026-05-24T10:02:00.000Z"}"#;

        assert!(latest_remote_control_backend_error(marker_text).is_none());
    }

    #[test]
    fn latest_remote_control_backend_error_is_cleared_by_later_connected_status() {
        let marker_text = r#"{"event":"app_server_spawn","time":"2026-05-24T09:59:00.000Z"}
{"event":"remote_control_helper_status","time":"2026-05-24T10:00:00.000Z","status":"errored"}
{"event":"remote_control_backend_error","time":"2026-05-24T10:00:01.000Z","kind":"enrollment_failed","text":"remoteControl/status returned errored"}
{"event":"remote_control_helper_status","time":"2026-05-24T10:01:00.000Z","status":"connected","serverName":"desktop"}"#;

        assert!(latest_remote_control_backend_error(marker_text).is_none());
    }

    #[test]
    fn latest_remote_control_backend_error_is_not_cleared_by_connecting_status() {
        let marker_text = r#"{"event":"app_server_spawn","time":"2026-05-24T09:59:00.000Z"}
{"event":"remote_control_backend_error","time":"2026-05-24T10:00:01.000Z","kind":"enrollment_failed","text":"remoteControl/status returned errored"}
{"event":"remote_control_helper_status","time":"2026-05-24T10:01:00.000Z","status":"connecting"}"#;

        let error = latest_remote_control_backend_error(marker_text)
            .expect("connecting should not clear a backend error");

        assert_eq!(
            error.get("kind").and_then(Value::as_str),
            Some("enrollment_failed")
        );
    }

    #[test]
    fn latest_remote_control_helper_status_uses_latest_status() {
        let marker_text = r#"{"event":"remote_control_helper_spawn","time":"2026-05-24T10:00:00.000Z"}
{"event":"remote_control_helper_status","time":"2026-05-24T10:01:00.000Z","status":"connecting"}
{"event":"remote_control_helper_status","time":"2026-05-24T10:02:00.000Z","status":"connected","serverName":"desktop"}"#;

        let status = latest_remote_control_helper_status(marker_text)
            .expect("latest helper status should be parsed");

        assert_eq!(
            status.get("status").and_then(Value::as_str),
            Some("connected")
        );
        assert_eq!(
            status.get("message").and_then(Value::as_str),
            Some("桌面端已连上 ChatGPT，可用 app 控制。")
        );
    }

    #[test]
    fn remote_control_backend_environment_summary_detects_app_client() {
        let data = json!({
            "items": [
                {
                    "env_id": "env_current",
                    "display_name": "DESKTOP-2KU3M74",
                    "online": true,
                    "installation_id": "install-current",
                    "client_type": "CODEX_DESKTOP_APP",
                    "originator": "Codex Desktop",
                    "client_name": "codex_chatgpt_ios_remote",
                    "last_seen_at": "2026-05-25T13:00:04Z"
                },
                {
                    "env_id": "env_old",
                    "display_name": "DESKTOP-2KU3M74",
                    "online": false,
                    "client_name": "codex-switch-status-probe"
                }
            ]
        });

        let status = remote_control_backend_environment_summary_from_items(&data, "env_current")
            .expect("current environment should be found");

        assert_eq!(
            status.get("appClientConnected").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            status.get("sameDisplayNameCount").and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(
            status
                .get("offlineSameDisplayNameCount")
                .and_then(Value::as_u64),
            Some(1)
        );
    }

    #[test]
    fn remote_control_connection_status_reports_desktop_ready() {
        let helper_status = json!({
            "status": "connected",
            "message": "桌面端已连上 ChatGPT，可用 app 控制。"
        });
        let environment = json!({
            "status": "found",
            "environmentId": "env_current",
            "displayName": "DESKTOP-2KU3M74",
            "online": true,
            "clientName": "codex-switch-remote-control",
            "appClientConnected": false
        });

        let status =
            remote_control_connection_status(true, None, Some(&helper_status), Some(&environment));

        assert_eq!(
            status.get("status").and_then(Value::as_str),
            Some("desktop_ready")
        );
        assert_eq!(status.get("state").and_then(Value::as_str), Some("active"));
        assert_eq!(
            status.get("message").and_then(Value::as_str),
            Some("桌面端已在线，可用 app 控制。")
        );
    }

    #[test]
    fn remote_control_connection_status_reports_app_recently_seen() {
        let helper_status = json!({
            "status": "connected",
            "message": "桌面端已连上 ChatGPT，可用 app 控制。"
        });
        let environment = json!({
            "status": "found",
            "environmentId": "env_current",
            "displayName": "DESKTOP-2KU3M74",
            "online": true,
            "clientName": "codex_chatgpt_ios_remote",
            "appClientConnected": true
        });

        let status =
            remote_control_connection_status(true, None, Some(&helper_status), Some(&environment));

        assert_eq!(
            status.get("status").and_then(Value::as_str),
            Some("app_recently_seen")
        );
        assert_eq!(status.get("state").and_then(Value::as_str), Some("active"));
        assert_eq!(
            status.get("message").and_then(Value::as_str),
            Some("服务正常。")
        );
    }

    #[test]
    fn remote_control_connection_status_reports_login_expired_as_warning() {
        let backend_error = json!({
            "kind": "login_expired",
            "message": "控制账号登录已过期，请重新登录。"
        });

        let status = remote_control_connection_status(true, Some(&backend_error), None, None);

        assert_eq!(
            status.get("status").and_then(Value::as_str),
            Some("login_expired")
        );
        assert_eq!(status.get("state").and_then(Value::as_str), Some("warning"));
        assert_eq!(
            status.get("message").and_then(Value::as_str),
            Some("控制账号登录已过期，请重新登录。")
        );
    }

    #[test]
    fn latest_remote_control_helper_status_reports_helper_error() {
        let marker_text = r#"{"event":"remote_control_helper_error","time":"2026-05-24T10:00:00.000Z","message":"missing_remote_control_home"}"#;

        let status = latest_remote_control_helper_status(marker_text)
            .expect("helper error should be parsed");

        assert_eq!(
            status.get("status").and_then(Value::as_str),
            Some("errored")
        );
        assert_eq!(
            status.get("raw").and_then(Value::as_str),
            Some("missing_remote_control_home")
        );
    }

    #[test]
    fn remote_control_subscription_config_uses_api_provider_for_sessions() {
        let config =
            remote_control_subscription_config_text("https://api.example.com/v1", "gpt-5.5");

        assert!(config.contains("model = \"gpt-5.5\""));
        assert!(config.contains("model_provider = \"api\""));
        assert!(config.contains("[features]\nremote_control = true"));
        assert!(config.contains("[model_providers.api]"));
        assert!(config.contains("base_url = \"https://api.example.com/v1\""));
        assert!(config.contains("env_key = \"OPENAI_API_KEY\""));
        assert!(config.contains("requires_openai_auth = false"));
    }

    #[test]
    fn remote_control_history_path_maps_root_home_to_isolated_home() {
        let source_home = PathBuf::from(r"C:\codex-home");
        let target_home = PathBuf::from(r"C:\codex-remote-home");

        let mapped = map_remote_control_history_path(
            r"\\?\C:\codex-home\sessions\rollout.jsonl",
            &source_home,
            &target_home,
        );

        assert_eq!(mapped, r"\\?\C:\codex-remote-home\sessions\rollout.jsonl");
    }

    #[test]
    fn remote_control_session_index_merge_keeps_existing_target_line() {
        let root = unique_remote_control_test_dir("session-index");
        let source = root.join("source.jsonl");
        let target = root.join("target.jsonl");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            &source,
            "{\"id\":\"same\",\"thread_name\":\"source\",\"updated_at\":\"2026-05-24T00:00:00Z\"}\n",
        )
        .unwrap();
        fs::write(
            &target,
            "{\"id\":\"same\",\"thread_name\":\"target\",\"updated_at\":\"2026-05-25T00:00:00Z\"}\n",
        )
        .unwrap();

        let changed = merge_remote_control_session_index_filtered(&source, &target, None).unwrap();
        let merged = fs::read_to_string(&target).unwrap();

        fs::remove_dir_all(&root).unwrap();

        assert!(!changed);
        assert!(merged.contains("\"thread_name\":\"target\""));
        assert!(!merged.contains("\"thread_name\":\"source\""));
    }

    #[test]
    fn remote_control_history_state_merge_preserves_enrollment_and_remote_threads() {
        let root = unique_remote_control_test_dir("state-root");
        let remote = unique_remote_control_test_dir("state-remote");
        let source_db = root.join("state_5.sqlite");
        let target_db = remote.join("state_5.sqlite");
        create_remote_control_history_state_db(&source_db);
        create_remote_control_history_state_db(&target_db);
        let source = Connection::open(&source_db).unwrap();
        source
            .execute(
                "INSERT INTO threads (id, rollout_path, model_provider, title)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    "source-thread",
                    root.join("sessions")
                        .join("rollout-source.jsonl")
                        .to_string_lossy()
                        .to_string(),
                    "openai",
                    "source"
                ],
            )
            .unwrap();
        source
            .execute(
                "INSERT INTO threads (id, rollout_path, model_provider, title)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    "remote-thread",
                    root.join("sessions")
                        .join("rollout-conflicting-source.jsonl")
                        .to_string_lossy()
                        .to_string(),
                    "openai",
                    "source-conflict"
                ],
            )
            .unwrap();
        drop(source);
        let target = Connection::open(&target_db).unwrap();
        target
            .execute(
                "CREATE TABLE remote_control_enrollments (
                    websocket_url TEXT NOT NULL,
                    account_id TEXT NOT NULL,
                    app_server_client_name TEXT NOT NULL,
                    server_id TEXT NOT NULL,
                    environment_id TEXT NOT NULL,
                    server_name TEXT NOT NULL,
                    updated_at INTEGER NOT NULL,
                    PRIMARY KEY (websocket_url, account_id, app_server_client_name)
                )",
                [],
            )
            .unwrap();
        target
            .execute(
                "INSERT INTO remote_control_enrollments
                 (websocket_url, account_id, app_server_client_name, server_id, environment_id, server_name, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    "ws://127.0.0.1:1/backend-api/wham/remote/control/server",
                    "account",
                    "client",
                    "server",
                    "environment",
                    "desktop",
                    1_i64
                ],
            )
            .unwrap();
        target
            .execute(
                "INSERT INTO threads (id, rollout_path, model_provider, title)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    "remote-thread",
                    remote
                        .join("sessions")
                        .join("rollout-remote.jsonl")
                        .to_string_lossy()
                        .to_string(),
                    "openai",
                    "remote"
                ],
            )
            .unwrap();
        drop(target);

        merge_remote_control_state_threads_with_limit(&root, &remote, API_PROVIDER_ID, usize::MAX)
            .unwrap();

        let connection = Connection::open(&target_db).unwrap();
        let source_row: (String, String) = connection
            .query_row(
                "SELECT rollout_path, model_provider FROM threads WHERE id = 'source-thread'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let remote_row: (String, String, String) = connection
            .query_row(
                "SELECT rollout_path, model_provider, title FROM threads WHERE id = 'remote-thread'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        let enrollments: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM remote_control_enrollments",
                [],
                |row| row.get(0),
            )
            .unwrap();
        drop(connection);

        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&remote).unwrap();

        assert!(path_text_starts_with_home(&source_row.0, &remote));
        assert_eq!(source_row.1, "api");
        assert!(path_text_starts_with_home(&remote_row.0, &remote));
        assert_eq!(remote_row.1, "api");
        assert_eq!(remote_row.2, "remote");
        assert_eq!(enrollments, 1);
    }

    #[test]
    fn remote_control_recent_state_merge_updates_older_target_thread() {
        let root = unique_remote_control_test_dir("state-recent-source-newer");
        let remote = unique_remote_control_test_dir("state-recent-target-older");
        let source_db = root.join("state_5.sqlite");
        let target_db = remote.join("state_5.sqlite");
        create_remote_control_history_state_db(&source_db);
        create_remote_control_history_state_db(&target_db);
        let source = Connection::open(&source_db).unwrap();
        source
            .execute(
                "INSERT INTO threads (id, rollout_path, model_provider, title, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    "same-thread",
                    root.join("sessions")
                        .join("rollout-source.jsonl")
                        .to_string_lossy()
                        .to_string(),
                    "openai",
                    "source newer",
                    2000_i64
                ],
            )
            .unwrap();
        drop(source);
        let target = Connection::open(&target_db).unwrap();
        target
            .execute(
                "INSERT INTO threads (id, rollout_path, model_provider, title, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    "same-thread",
                    remote
                        .join("sessions")
                        .join("rollout-target.jsonl")
                        .to_string_lossy()
                        .to_string(),
                    "openai",
                    "target older",
                    1000_i64
                ],
            )
            .unwrap();
        drop(target);

        merge_remote_control_state_threads_recent(&root, &remote, API_PROVIDER_ID).unwrap();

        let connection = Connection::open(&target_db).unwrap();
        let row: (String, String, String, i64) = connection
            .query_row(
                "SELECT rollout_path, model_provider, title, updated_at_ms
                 FROM threads WHERE id = 'same-thread'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        drop(connection);

        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&remote).unwrap();

        assert!(path_text_starts_with_home(&row.0, &remote));
        assert_eq!(row.1, "api");
        assert_eq!(row.2, "source newer");
        assert_eq!(row.3, 2000);
    }

    #[test]
    fn remote_control_recent_state_merge_keeps_newer_target_thread() {
        let root = unique_remote_control_test_dir("state-recent-source-older");
        let remote = unique_remote_control_test_dir("state-recent-target-newer");
        let source_db = root.join("state_5.sqlite");
        let target_db = remote.join("state_5.sqlite");
        create_remote_control_history_state_db(&source_db);
        create_remote_control_history_state_db(&target_db);
        let source = Connection::open(&source_db).unwrap();
        source
            .execute(
                "INSERT INTO threads (id, rollout_path, model_provider, title, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    "same-thread",
                    root.join("sessions")
                        .join("rollout-source.jsonl")
                        .to_string_lossy()
                        .to_string(),
                    "openai",
                    "source older",
                    1000_i64
                ],
            )
            .unwrap();
        drop(source);
        let target = Connection::open(&target_db).unwrap();
        target
            .execute(
                "INSERT INTO threads (id, rollout_path, model_provider, title, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    "same-thread",
                    remote
                        .join("sessions")
                        .join("rollout-target.jsonl")
                        .to_string_lossy()
                        .to_string(),
                    "openai",
                    "target newer",
                    2000_i64
                ],
            )
            .unwrap();
        drop(target);

        merge_remote_control_state_threads_recent(&root, &remote, API_PROVIDER_ID).unwrap();

        let connection = Connection::open(&target_db).unwrap();
        let row: (String, String, String, i64) = connection
            .query_row(
                "SELECT rollout_path, model_provider, title, updated_at_ms
                 FROM threads WHERE id = 'same-thread'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        drop(connection);

        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&remote).unwrap();

        assert!(path_text_starts_with_home(&row.0, &remote));
        assert_eq!(row.1, "api");
        assert_eq!(row.2, "target newer");
        assert_eq!(row.3, 2000);
    }

    #[test]
    fn remote_control_relay_enrollment_cleanup_removes_stale_local_rows() {
        let home = unique_remote_control_test_dir("relay-enrollment");
        let state_db = home.join("state_5.sqlite");
        fs::create_dir_all(&home).unwrap();
        let connection = Connection::open(&state_db).unwrap();
        connection
            .execute(
                "CREATE TABLE remote_control_enrollments (
                    websocket_url TEXT NOT NULL,
                    account_id TEXT NOT NULL,
                    app_server_client_name TEXT NOT NULL,
                    server_id TEXT NOT NULL,
                    environment_id TEXT NOT NULL,
                    server_name TEXT NOT NULL,
                    updated_at INTEGER NOT NULL,
                    PRIMARY KEY (websocket_url, account_id, app_server_client_name)
                )",
                [],
            )
            .unwrap();
        for (url, updated_at) in [
            (
                "ws://127.0.0.1:54353/backend-api/wham/remote/control/server",
                1_i64,
            ),
            (
                "ws://127.0.0.1:60984/backend-api/wham/remote/control/server",
                3_i64,
            ),
            (REMOTE_CONTROL_UPSTREAM_WS_URL, 2_i64),
        ] {
            connection
                .execute(
                    "INSERT INTO remote_control_enrollments
                     (websocket_url, account_id, app_server_client_name, server_id, environment_id, server_name, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        url,
                        "account",
                        "",
                        format!("server-{updated_at}"),
                        format!("environment-{updated_at}"),
                        "desktop",
                        updated_at
                    ],
                )
                .unwrap();
        }
        drop(connection);

        let deleted = cleanup_remote_control_relay_enrollments(&home).unwrap();

        let connection = Connection::open(&state_db).unwrap();
        let local_rows: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM remote_control_enrollments WHERE websocket_url LIKE ?1",
                [format!("ws://127.0.0.1:%{REMOTE_CONTROL_UPSTREAM_PATH}")],
                |row| row.get(0),
            )
            .unwrap();
        let upstream_rows: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM remote_control_enrollments WHERE websocket_url = ?1",
                [REMOTE_CONTROL_UPSTREAM_WS_URL],
                |row| row.get(0),
            )
            .unwrap();
        let total_rows: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM remote_control_enrollments",
                [],
                |row| row.get(0),
            )
            .unwrap();
        drop(connection);

        fs::remove_dir_all(&home).unwrap();

        assert_eq!(deleted, 2);
        assert_eq!(local_rows, 0);
        assert_eq!(upstream_rows, 1);
        assert_eq!(total_rows, 1);
    }

    #[test]
    fn remote_control_history_state_merge_creates_missing_target_db() {
        let root = unique_remote_control_test_dir("state-root-copy");
        let remote = unique_remote_control_test_dir("state-remote-copy");
        let source_db = root.join("state_5.sqlite");
        let target_db = remote.join("state_5.sqlite");
        create_remote_control_history_state_db(&source_db);
        let source = Connection::open(&source_db).unwrap();
        source
            .execute(
                "INSERT INTO threads (id, rollout_path, model_provider, title)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    "source-thread",
                    root.join("sessions")
                        .join("rollout-source.jsonl")
                        .to_string_lossy()
                        .to_string(),
                    "openai",
                    "source"
                ],
            )
            .unwrap();
        drop(source);

        merge_remote_control_state_threads_with_limit(&root, &remote, API_PROVIDER_ID, usize::MAX)
            .unwrap();

        let connection = Connection::open(&target_db).unwrap();
        let source_row: (String, String) = connection
            .query_row(
                "SELECT rollout_path, model_provider FROM threads WHERE id = 'source-thread'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        drop(connection);

        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&remote).unwrap();

        assert!(path_text_starts_with_home(&source_row.0, &remote));
        assert_eq!(source_row.1, "api");
    }
}
