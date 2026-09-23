#[cfg(windows)]
use super::codex_app_watcher::process_command_line;
use super::process_control::kill_process_tree;
use crate::{
    accounts::{
        auth_error_is_login_expired, get_codex_state_value, lookup_store_account,
        mark_account_auth_error, profile_id_from_account, profile_id_from_tokens_value,
        read_api_key_from_auth, read_api_key_from_provider_config, read_auth_value,
        read_store_value, set_api_mode, set_subscription_mode, write_account_auth,
    },
    api_config::API_PROVIDER_ID,
    app_log::{account_label, log_event, log_event_once},
    blocking_task::run_blocking,
    codex_config::{
        read_root_config, read_table_config, remove_config_values, remove_remote_control_config,
        remove_table_config, set_config_values, set_table_config,
    },
    json_util::string_field,
    paths::{app_data_dir, auth_path},
    settings::{
        default_api_mode, read_settings_value, remote_control_config_enabled_from_settings,
        remote_control_enabled_from_settings, remote_control_suspended_by_subscription,
        update_settings_value, REMOTE_CONTROL_ENABLED_SETTING_KEY,
    },
};
use backend_status::{
    fetch_remote_control_backend_environment_status,
    remote_control_status_from_backend_environment, truncate_remote_control_error_text,
};
use serde_json::{json, Value};
use std::fs;

mod backend_status;

const REMOTE_CONTROL_ACCOUNT_SETTING_KEY: &str = "codex_remote_control_account_id";

const API_WIRE: &str = "responses";

const REMOTE_CONTROL_LOGIN_EXPIRED_AUTO_DISABLE_MESSAGE: &str = "当前控制账号过期，远程控制关闭";

const REMOTE_CONTROL_ACCOUNT_INVALID_AUTO_DISABLE_MESSAGE: &str =
    "控制账号登录已失效，已关闭远程控制";

const REMOTE_CONTROL_MISSING_ACCOUNT_AUTO_DISABLE_MESSAGE: &str =
    "远程控制账号不存在，已关闭远程控制并切换到 API 模式";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RemoteControlRuntimeTarget {
    MixedApi,
    Subscription,
    Api,
}

fn remote_control_runtime_target(settings: &Value) -> RemoteControlRuntimeTarget {
    if remote_control_enabled_from_settings(settings) {
        RemoteControlRuntimeTarget::MixedApi
    } else if remote_control_suspended_by_subscription(settings) {
        RemoteControlRuntimeTarget::Subscription
    } else {
        RemoteControlRuntimeTarget::Api
    }
}

fn remote_control_account_id_from_settings(settings: &Value) -> String {
    string_field(settings, REMOTE_CONTROL_ACCOUNT_SETTING_KEY)
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RemoteControlAccountIssue {
    Missing(String),
    LoginExpired(String),
}

impl RemoteControlAccountIssue {
    fn account_id(&self) -> &str {
        match self {
            Self::Missing(account_id) | Self::LoginExpired(account_id) => account_id,
        }
    }

    fn reason(&self) -> &'static str {
        match self {
            Self::Missing(_) => "missing_account",
            Self::LoginExpired(_) => "login_expired",
        }
    }

    fn message(&self) -> &'static str {
        match self {
            Self::Missing(_) => REMOTE_CONTROL_MISSING_ACCOUNT_AUTO_DISABLE_MESSAGE,
            Self::LoginExpired(_) => REMOTE_CONTROL_ACCOUNT_INVALID_AUTO_DISABLE_MESSAGE,
        }
    }
}

fn remote_control_account_login_expired(account: &Value) -> bool {
    let custom = account.get("custom").unwrap_or(&Value::Null);
    if string_field(custom, "auth_status") != "error" {
        return false;
    }

    let auth_error = custom.get("auth_error").unwrap_or(&Value::Null);
    let text = format!(
        "{} {} {} {}",
        string_field(custom, "auth_status_message"),
        string_field(auth_error, "code"),
        string_field(auth_error, "message"),
        string_field(auth_error, "raw_message")
    );
    auth_error_is_login_expired(&text)
}

fn remote_control_account_issue_with_lookup<F>(
    settings: &Value,
    lookup: F,
) -> Result<Option<RemoteControlAccountIssue>, String>
where
    F: FnOnce(&str) -> Result<Option<Value>, String>,
{
    if !remote_control_config_enabled_from_settings(settings) {
        return Ok(None);
    }

    let account_id = remote_control_account_id_from_settings(settings);
    if account_id.is_empty() {
        return Ok(Some(RemoteControlAccountIssue::Missing(account_id)));
    }

    let Some(account) = lookup(&account_id)? else {
        return Ok(Some(RemoteControlAccountIssue::Missing(account_id)));
    };
    if remote_control_account_login_expired(&account) {
        return Ok(Some(RemoteControlAccountIssue::LoginExpired(account_id)));
    }
    Ok(None)
}

fn remote_control_account_issue(
    settings: &Value,
) -> Result<Option<RemoteControlAccountIssue>, String> {
    remote_control_account_issue_with_lookup(settings, lookup_store_account)
}

fn missing_remote_control_account_fallback_patch() -> Value {
    json!({
        REMOTE_CONTROL_ENABLED_SETTING_KEY: false,
        REMOTE_CONTROL_ACCOUNT_SETTING_KEY: Value::Null,
        "codex_active_mode": "api"
    })
}

pub(crate) fn reset_remote_control_to_api_mode_settings() -> Result<Value, String> {
    update_settings_value(&missing_remote_control_account_fallback_patch())
}

fn remote_control_sync_changed(legacy_runtime_changed: bool, runtime_config_changed: bool) -> bool {
    legacy_runtime_changed || runtime_config_changed
}

fn missing_account_fallback_changes_live_runtime_target(
    before: RemoteControlRuntimeTarget,
    after: RemoteControlRuntimeTarget,
) -> bool {
    before == RemoteControlRuntimeTarget::Subscription && after == RemoteControlRuntimeTarget::Api
}

fn remote_control_account(account_id: &str) -> Result<Value, String> {
    if account_id.trim().is_empty() {
        return Err("远程控制需要先单独选择一个订阅账号".to_string());
    }
    lookup_store_account(account_id)?
        .ok_or_else(|| format!("远程控制账号不存在，请重新选择: {account_id}"))
}

fn validate_remote_control_account_id(account_id: &str) -> Result<(), String> {
    remote_control_account(account_id).map(|_| ())
}

fn active_api_profile(settings: &Value) -> Value {
    settings
        .get("api_mode")
        .cloned()
        .unwrap_or_else(default_api_mode)
}

fn api_key_from_settings_or_runtime(profile: &Value) -> String {
    let api_key = string_field(profile, "api_key");
    if !api_key.trim().is_empty() {
        return api_key.trim().to_string();
    }

    let provider_key = read_api_key_from_provider_config();
    if !provider_key.trim().is_empty() {
        return provider_key.trim().to_string();
    }

    read_api_key_from_auth().trim().to_string()
}

fn remote_control_api_session_profile_from_settings(
    settings: &Value,
) -> Result<(String, String), String> {
    let api_mode = active_api_profile(settings);
    let base_url = string_field(&api_mode, "base_url");
    if base_url.is_empty() {
        return Err("远程控制会话流量走 API 需要先配置 API 模式 base_url".to_string());
    }

    let api_key = api_key_from_settings_or_runtime(&api_mode);
    if api_key.trim().is_empty() {
        return Err("远程控制会话流量走 API 需要先配置 API Key".to_string());
    }

    Ok((base_url, api_key))
}

fn subscription_mode_active(settings: &Value) -> bool {
    remote_control_suspended_by_subscription(settings)
        || string_field(&get_codex_state_value(), "mode") == "chatgpt"
}

fn validate_remote_control_enable_prerequisites() -> Result<(), String> {
    let desktop_status = super::codex_desktop_support_status();
    if desktop_status.get("supported").and_then(Value::as_bool) != Some(true) {
        return Err(desktop_status
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("请先安装新版 ChatGPT Desktop")
            .to_string());
    }
    let settings = read_settings_value()?;
    if subscription_mode_active(&settings) {
        return Err("订阅模式下不可开启远程控制，请先切换到 API 模式".to_string());
    }
    let account_id = remote_control_account_id_from_settings(&settings);
    validate_remote_control_account_id(&account_id)?;
    remote_control_api_session_profile_from_settings(&settings).map(|_| ())
}

fn remote_control_mixed_provider_config(
    api_base_url: &str,
    api_key: &str,
) -> Vec<(&'static str, Value)> {
    vec![
        ("name", Value::String(API_PROVIDER_ID.to_string())),
        ("wire_api", Value::String(API_WIRE.to_string())),
        ("base_url", Value::String(api_base_url.to_string())),
        ("supports_websockets", Value::Bool(false)),
        ("requires_openai_auth", Value::Bool(true)),
        (
            "experimental_bearer_token",
            Value::String(api_key.trim().to_string()),
        ),
    ]
}

fn apply_remote_control_mixed_config(settings: &Value) -> Result<(), String> {
    let account_id = remote_control_account_id_from_settings(settings);
    let account = remote_control_account(&account_id)?;
    let (api_base_url, api_key) = remote_control_api_session_profile_from_settings(settings)?;

    write_account_auth(&account)?;
    set_config_values(vec![
        ("model_provider", API_PROVIDER_ID.to_string()),
        ("cli_auth_credentials_store", "file".to_string()),
    ])?;
    remove_config_values(&[
        "preferred_auth_method",
        "forced_login_method",
        "openai_base_url",
    ])?;
    set_table_config(
        &format!("model_providers.{API_PROVIDER_ID}"),
        remote_control_mixed_provider_config(&api_base_url, &api_key),
    )?;
    remove_remote_control_config()?;
    log_event(
        "codex_remote_control_runtime_applied",
        json!({
            "mode": "api_remote_control",
            "remoteControl": true,
            "account": account_label(&account_id),
            "provider": API_PROVIDER_ID
        }),
    );
    Ok(())
}

fn restore_api_config_after_remote_control_disabled(settings: &Value) -> Result<(), String> {
    let api_mode = active_api_profile(settings);
    if string_field(&api_mode, "base_url").is_empty() {
        remove_table_config(&format!("model_providers.{API_PROVIDER_ID}"))?;
        return Ok(());
    }
    set_api_mode(&api_mode)
}

fn legacy_remote_control_home_removed() -> Result<bool, String> {
    let home = app_data_dir()?.join("remote-control-codex-home");
    if !home.exists() {
        return Ok(false);
    }
    fs::remove_dir_all(&home)
        .map_err(|err| format!("删除旧远程控制 home 失败 {}: {err}", home.display()))?;
    Ok(true)
}

// Old versions started `codex.exe app-server ... --enable remote_control` themselves. The list scan
// reads only names; command lines are read for `codex.exe` candidates alone. The sysinfo refresh has
// no error result: a command line it cannot read (for example an elevated process) stays empty and
// does not match, as WMI returned a null CommandLine for it before.
#[cfg(windows)]
fn legacy_remote_control_helper_pids() -> Vec<u64> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
    let candidates = system
        .processes()
        .iter()
        .filter(|(_, process)| {
            is_legacy_remote_control_helper_name(&process.name().to_string_lossy())
        })
        .map(|(pid, _)| *pid)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Vec::new();
    }
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&candidates),
        false,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .without_tasks(),
    );
    let mut pids = candidates
        .iter()
        .filter_map(|pid| system.process(*pid))
        .filter(|process| {
            is_legacy_remote_control_helper_command_line(&process_command_line(process))
        })
        .map(|process| u64::from(process.pid().as_u32()))
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids
}

#[cfg(not(windows))]
fn legacy_remote_control_helper_pids() -> Vec<u64> {
    Vec::new()
}

// Same test as the former `$_.Name -ieq "codex.exe"`.
#[cfg(any(windows, test))]
fn is_legacy_remote_control_helper_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("codex.exe")
}

// Same tests as the former case-insensitive `-match "\bapp-server\b"` and
// `-match "--enable\s+remote_control"`; word characters are letters, digits and `_`.
#[cfg(any(windows, test))]
fn is_legacy_remote_control_helper_command_line(command_line: &str) -> bool {
    let command_line = command_line.to_lowercase();
    contains_whole_word(&command_line, "app-server")
        && contains_enable_remote_control(&command_line)
}

#[cfg(any(windows, test))]
fn contains_whole_word(text: &str, word: &str) -> bool {
    let is_word_char = |ch: char| ch.is_alphanumeric() || ch == '_';
    text.match_indices(word).any(|(index, _)| {
        !text[..index].chars().next_back().is_some_and(is_word_char)
            && !text[index + word.len()..]
                .chars()
                .next()
                .is_some_and(is_word_char)
    })
}

#[cfg(any(windows, test))]
fn contains_enable_remote_control(text: &str) -> bool {
    const FLAG: &str = "--enable";
    text.match_indices(FLAG).any(|(index, _)| {
        let rest = &text[index + FLAG.len()..];
        let value = rest.trim_start();
        value.len() < rest.len() && value.starts_with("remote_control")
    })
}

fn stop_legacy_remote_control_helpers() -> Result<usize, String> {
    let mut stopped = 0;
    for pid in legacy_remote_control_helper_pids() {
        if kill_process_tree(pid)? {
            stopped += 1;
        }
    }
    Ok(stopped)
}

fn cleanup_legacy_remote_control_runtime() -> Result<bool, String> {
    let stopped = stop_legacy_remote_control_helpers()?;
    let home_removed = legacy_remote_control_home_removed()?;
    Ok(stopped > 0 || home_removed)
}

fn provider_bool_field(provider: &serde_json::Map<String, Value>, key: &str) -> bool {
    provider.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn provider_string_field(provider: &serde_json::Map<String, Value>, key: &str) -> String {
    provider
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

// The applied/present checks answer "does the live config already match". An absent selection,
// account, auth.json or provider table is a "no"; a file or store that cannot be read or parsed
// is returned as an error instead of being read as "not applied".
fn active_auth_matches_remote_control_account(settings: &Value) -> Result<bool, String> {
    let account_id = remote_control_account_id_from_settings(settings);
    if account_id.is_empty() {
        return Ok(false);
    }
    let Some(account) = lookup_store_account(&account_id)? else {
        return Ok(false);
    };
    let expected_profile_id = profile_id_from_account(&account)?;

    if !auth_path()?.exists() {
        return Ok(false);
    }
    let auth = read_auth_value()?;
    if string_field(&auth, "auth_mode") != "chatgpt" {
        return Ok(false);
    }

    Ok(profile_id_from_tokens_value(auth.get("tokens"))? == expected_profile_id)
}

fn remote_control_mixed_config_applied(settings: &Value) -> Result<bool, String> {
    // Without a complete API profile the mixed config cannot have been applied.
    let Ok((api_base_url, api_key)) = remote_control_api_session_profile_from_settings(settings)
    else {
        return Ok(false);
    };
    if !active_auth_matches_remote_control_account(settings)? {
        return Ok(false);
    }

    let root_config = read_root_config()?;
    if root_config.get("model_provider").and_then(Value::as_str) != Some(API_PROVIDER_ID) {
        return Ok(false);
    }

    let provider = read_table_config(&format!("model_providers.{API_PROVIDER_ID}"))?;

    Ok(provider_string_field(&provider, "base_url") == api_base_url
        && provider_string_field(&provider, "wire_api") == API_WIRE
        && provider_bool_field(&provider, "requires_openai_auth")
        && provider_string_field(&provider, "experimental_bearer_token") == api_key)
}

fn remote_control_mixed_config_present() -> Result<bool, String> {
    let root_config = read_root_config()?;
    if root_config.get("model_provider").and_then(Value::as_str) != Some(API_PROVIDER_ID) {
        return Ok(false);
    }
    let provider = read_table_config(&format!("model_providers.{API_PROVIDER_ID}"))?;
    Ok(
        !provider_string_field(&provider, "experimental_bearer_token").is_empty()
            && provider_bool_field(&provider, "requires_openai_auth"),
    )
}

pub(crate) fn preview_remote_control_runtime_for_current_settings(
    _trigger: &str,
) -> Result<bool, String> {
    let settings = read_settings_value()?;
    if remote_control_account_issue(&settings)?.is_some() {
        return Ok(true);
    }
    if remote_control_enabled_from_settings(&settings) {
        validate_remote_control_enable_prerequisites()?;
        return Ok(!remote_control_mixed_config_applied(&settings)?);
    }

    Ok(remote_control_mixed_config_present()?
        || legacy_remote_control_home_removed_pending()?
        || !legacy_remote_control_helper_pids().is_empty())
}

fn legacy_remote_control_home_removed_pending() -> Result<bool, String> {
    Ok(app_data_dir()?.join("remote-control-codex-home").exists())
}

pub(crate) fn sync_remote_control_runtime_for_current_settings(
    context: &str,
) -> Result<bool, String> {
    let mut settings = read_settings_value()?;
    let runtime_target_before_fallback = remote_control_runtime_target(&settings);
    let account_issue = remote_control_account_issue(&settings)?;

    match account_issue.as_ref() {
        Some(RemoteControlAccountIssue::Missing(_)) => {
            settings = reset_remote_control_to_api_mode_settings()?;
        }
        Some(RemoteControlAccountIssue::LoginExpired(_)) => {
            settings = update_settings_value(&json!({
                REMOTE_CONTROL_ENABLED_SETTING_KEY: false
            }))?;
        }
        None => {}
    }
    let runtime_target = remote_control_runtime_target(&settings);
    let fallback_changed_runtime_target = matches!(
        account_issue.as_ref(),
        Some(RemoteControlAccountIssue::Missing(_))
    ) && missing_account_fallback_changes_live_runtime_target(
        runtime_target_before_fallback,
        runtime_target,
    );
    let legacy_runtime_changed = cleanup_legacy_remote_control_runtime()?;

    let runtime_config_changed = match runtime_target {
        RemoteControlRuntimeTarget::MixedApi => {
            let pending = !remote_control_mixed_config_applied(&settings)?;
            apply_remote_control_mixed_config(&settings)?;
            pending
        }
        RemoteControlRuntimeTarget::Subscription => {
            let pending = remote_control_mixed_config_present()?;
            set_subscription_mode()?;
            remove_remote_control_config()?;
            pending
        }
        RemoteControlRuntimeTarget::Api => {
            let pending = remote_control_mixed_config_present()?;
            restore_api_config_after_remote_control_disabled(&settings)?;
            remove_remote_control_config()?;
            pending
        }
    } || fallback_changed_runtime_target;
    let changed = remote_control_sync_changed(legacy_runtime_changed, runtime_config_changed);

    if let Some(issue) = account_issue {
        log_event(
            "codex_remote_control_auto_disabled",
            json!({
                "context": context,
                "reason": issue.reason(),
                "account": account_label(issue.account_id()),
                "message": issue.message()
            }),
        );
    }

    if changed {
        log_event(
            "codex_remote_control_runtime_updated",
            json!({
                "context": context,
                "mode": "api_remote_control",
                "remoteControl": remote_control_enabled_from_settings(&settings)
            }),
        );
    }
    Ok(changed)
}

fn remote_control_backend_environment_status(settings: &Value) -> Result<Option<Value>, String> {
    if !remote_control_enabled_from_settings(settings)
        || !remote_control_mixed_config_applied(settings)?
    {
        return Ok(None);
    }

    let account_id = remote_control_account_id_from_settings(settings);
    let status = remote_control_account(&account_id)
        .and_then(|account| fetch_remote_control_backend_environment_status(&account));
    Ok(match status {
        Ok(status) => Some(status),
        Err(err) => Some(json!({
            "status": "lookup_failed",
            "message": "桌面状态查询失败",
            "raw": truncate_remote_control_error_text(&err)
        })),
    })
}

fn remote_control_status_value(
    settings: &Value,
    backend_environment: Option<&Value>,
) -> Result<Value, String> {
    if !remote_control_config_enabled_from_settings(settings) {
        return Ok(json!({
            "state": "muted",
            "status": "disabled",
            "message": "未启用"
        }));
    }
    if remote_control_suspended_by_subscription(settings) {
        return Ok(json!({
            "state": "muted",
            "status": "subscription_mode",
            "message": "订阅模式不可用"
        }));
    }

    let account_id = remote_control_account_id_from_settings(settings);
    if account_id.is_empty() {
        return Ok(json!({
            "state": "error",
            "status": "missing_account",
            "message": "需要选择订阅账号"
        }));
    }
    if let Err(err) = validate_remote_control_account_id(&account_id) {
        return Ok(json!({
            "state": "error",
            "status": "invalid_account",
            "message": "订阅账号无效",
            "raw": err
        }));
    }
    if let Err(err) = remote_control_api_session_profile_from_settings(settings) {
        return Ok(json!({
            "state": "error",
            "status": "missing_api",
            "message": "缺少 API 配置",
            "raw": err
        }));
    }
    if remote_control_mixed_config_applied(settings)? {
        if let Some(environment) = backend_environment {
            if let Some(status) = remote_control_status_from_backend_environment(environment) {
                return Ok(status);
            }
        }
        return Ok(json!({
            "state": "active",
            "status": "applied",
            "message": "配置已应用"
        }));
    }

    Ok(json!({
        "state": "warning",
        "status": "pending_restart",
        "message": "重启 Codex 后生效",
        "raw": "远程控制配置待应用"
    }))
}

fn remote_control_status_is_login_expired(status: &Value) -> bool {
    status.get("status").and_then(Value::as_str) == Some("login_expired")
}

fn disable_remote_control_after_login_expired() -> Result<(Value, bool), String> {
    let current_settings = read_settings_value()?;
    let account_id = remote_control_account_id_from_settings(&current_settings);
    if !account_id.is_empty() {
        if let Err(err) = mark_account_auth_error(&account_id, "控制账号登录已过期，请重新登录")
        {
            log_event(
                "codex_remote_control_account_status_update_failed",
                json!({
                    "reason": "login_expired",
                    "error": err
                }),
            );
        }
    }
    let settings = update_settings_value(&json!({
        REMOTE_CONTROL_ENABLED_SETTING_KEY: false
    }))?;
    let settings = super::proxy_env::apply_codex_proxy_env_state_to_settings(settings)?;
    let changed = sync_remote_control_runtime_for_current_settings("remote_control_login_expired")?;
    Ok((settings, changed))
}

pub(crate) fn remote_control_codex_app_running() -> Result<bool, String> {
    Ok(!super::codex_app_watcher::refresh_current_codex_app_processes()?.is_empty())
}

fn attach_remote_control_auto_disabled_response(
    mut response: Value,
    settings: Value,
    message: &str,
    changed: bool,
    restart_required: bool,
    runtime_error: Option<String>,
    process_status_error: Option<String>,
) -> Value {
    if let Some(response) = response.as_object_mut() {
        response.insert("autoDisabled".to_string(), json!(true));
        response.insert("message".to_string(), json!(message));
        response.insert("settings".to_string(), settings);
        if let Ok(store) = read_store_value() {
            response.insert("store".to_string(), store);
        }
        response.insert("changed".to_string(), json!(changed));
        response.insert("restartRequired".to_string(), json!(restart_required));
        if let Some(error) = runtime_error {
            response.insert("runtimeError".to_string(), Value::String(error));
        }
        if let Some(error) = process_status_error {
            response.insert("processStatusError".to_string(), Value::String(error));
        }
    }
    response
}

#[tauri::command]
pub(crate) async fn get_codex_remote_control_status() -> Result<Value, String> {
    run_blocking("检测远程控制状态", || {
        // Polled every 4s while remote control is on; the UI shows every failure, the log keeps
        // each distinct one once.
        get_codex_remote_control_status_impl().inspect_err(|err| {
            log_event_once("codex_remote_control_status_error", json!({ "error": err }));
        })
    })
    .await
}

fn get_codex_remote_control_status_impl() -> Result<Value, String> {
    let mut settings = read_settings_value()?;
    let account_issue = remote_control_account_issue(&settings)?;
    let mut auto_disable_message = None;
    let mut changed = false;
    let mut restart_required = false;
    let mut runtime_error = None;
    let mut process_status_error = None;

    if let Some(issue) = account_issue {
        let codex_app_running = match remote_control_codex_app_running() {
            Ok(running) => running,
            Err(err) => {
                process_status_error = Some(err);
                false
            }
        };
        match sync_remote_control_runtime_for_current_settings("remote_control_account_invalid") {
            Ok(runtime_changed) => {
                changed = runtime_changed;
                restart_required =
                    runtime_changed && (codex_app_running || process_status_error.is_some());
            }
            Err(err) => {
                runtime_error = Some(err);
            }
        }
        settings = read_settings_value()?;
        if remote_control_config_enabled_from_settings(&settings) {
            return Err(runtime_error.unwrap_or_else(|| "自动关闭远程控制失败，请重试".to_string()));
        }
        auto_disable_message = Some(issue.message());
        log_event(
            "codex_remote_control_auto_disabled",
            json!({
                "reason": issue.reason(),
                "account": account_label(issue.account_id()),
                "changed": changed,
                "restartRequired": restart_required,
                "runtimeError": runtime_error.clone(),
                "processStatusError": process_status_error.clone()
            }),
        );
    }

    let backend_environment = remote_control_backend_environment_status(&settings)?;
    let mut connection_status =
        remote_control_status_value(&settings, backend_environment.as_ref())?;

    if auto_disable_message.is_none()
        && remote_control_status_is_login_expired(&connection_status)
        && remote_control_enabled_from_settings(&settings)
    {
        let codex_app_running = remote_control_codex_app_running()?;
        let (disabled_settings, runtime_changed) = disable_remote_control_after_login_expired()?;
        settings = disabled_settings;
        changed = runtime_changed;
        restart_required = codex_app_running && runtime_changed;
        auto_disable_message = Some(REMOTE_CONTROL_LOGIN_EXPIRED_AUTO_DISABLE_MESSAGE);
        if let Some(status) = connection_status.as_object_mut() {
            status.insert(
                "message".to_string(),
                json!(REMOTE_CONTROL_LOGIN_EXPIRED_AUTO_DISABLE_MESSAGE),
            );
            status.insert("autoDisabled".to_string(), json!(true));
        }
        log_event(
            "codex_remote_control_auto_disabled",
            json!({
                "reason": "login_expired",
                "changed": changed,
                "restartRequired": restart_required
            }),
        );
    }

    let mut response = json!({
        "ok": true,
        "enabled": remote_control_config_enabled_from_settings(&settings),
        "effectiveEnabled": remote_control_enabled_from_settings(&settings),
        "accountId": remote_control_account_id_from_settings(&settings),
        "codex_state": get_codex_state_value(),
        "backendEnvironment": backend_environment,
        "connectionStatus": connection_status
    });
    if let Some(message) = auto_disable_message {
        response = attach_remote_control_auto_disabled_response(
            response,
            settings,
            message,
            changed,
            restart_required,
            runtime_error,
            if changed { process_status_error } else { None },
        );
    } else if !remote_control_config_enabled_from_settings(&settings) {
        if let Some(response) = response.as_object_mut() {
            response.insert("settings".to_string(), settings);
            if let Ok(store) = read_store_value() {
                response.insert("store".to_string(), store);
            }
        }
    }
    Ok(response)
}

#[tauri::command]
pub(crate) async fn set_codex_remote_control_enabled(enabled: bool) -> Result<Value, String> {
    run_blocking("切换远程控制", move || {
        set_codex_remote_control_enabled_impl(enabled)
    })
    .await
}

fn set_codex_remote_control_enabled_impl(enabled: bool) -> Result<Value, String> {
    let codex_app_running =
        !super::codex_app_watcher::refresh_current_codex_app_processes()?.is_empty();
    if enabled {
        validate_remote_control_enable_prerequisites()?;
    }

    let settings_patch = if enabled {
        json!({
            REMOTE_CONTROL_ENABLED_SETTING_KEY: enabled,
            "codex_active_mode": "api"
        })
    } else {
        json!({
            REMOTE_CONTROL_ENABLED_SETTING_KEY: enabled
        })
    };
    let settings = update_settings_value(&settings_patch)?;
    let settings = super::proxy_env::apply_codex_proxy_env_state_to_settings(settings)?;
    let changed =
        sync_remote_control_runtime_for_current_settings("set_codex_remote_control_enabled")?;
    let restart_required = codex_app_running && changed;

    Ok(json!({
        "ok": true,
        "message": if enabled {
            if restart_required {
                "远程控制已启用，重启 Codex 后生效"
            } else {
                "远程控制已启用"
            }
        } else if restart_required {
            "远程控制已关闭，重启 Codex 后恢复 API 模式"
        } else {
            "远程控制已关闭"
        },
        "settings": settings,
        "changed": changed,
        "restartRequired": restart_required,
        "configDeferred": false
    }))
}

#[tauri::command]
pub(crate) async fn set_codex_remote_control_account_id(id: String) -> Result<Value, String> {
    run_blocking("更新远程控制账号", move || {
        set_codex_remote_control_account_id_impl(&id)
    })
    .await
}

fn set_codex_remote_control_account_id_impl(id: &str) -> Result<Value, String> {
    let account_id = id.trim();
    validate_remote_control_account_id(account_id)?;

    let codex_app_running =
        !super::codex_app_watcher::refresh_current_codex_app_processes()?.is_empty();
    let settings = update_settings_value(&json!({
        REMOTE_CONTROL_ACCOUNT_SETTING_KEY: account_id
    }))?;
    let settings = super::proxy_env::apply_codex_proxy_env_state_to_settings(settings)?;
    let changed = if remote_control_enabled_from_settings(&settings) {
        sync_remote_control_runtime_for_current_settings("set_codex_remote_control_account_id")?
    } else {
        false
    };
    let restart_required = codex_app_running && changed;

    Ok(json!({
        "ok": true,
        "message": if restart_required {
            "远程控制账号已更新，重启 Codex 后生效"
        } else {
            "远程控制账号已更新"
        },
        "settings": settings,
        "changed": changed,
        "restartRequired": restart_required
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    fn provider_map(values: Vec<(&'static str, Value)>) -> Map<String, Value> {
        values
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    #[test]
    fn mixed_provider_config_uses_bearer_token_and_openai_auth() {
        let config = provider_map(remote_control_mixed_provider_config(
            "https://api.example.com/v1",
            "sk-test",
        ));

        assert_eq!(config.get("name").and_then(Value::as_str), Some("api"));
        assert_eq!(
            config.get("wire_api").and_then(Value::as_str),
            Some("responses")
        );
        assert_eq!(
            config.get("requires_openai_auth").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            config
                .get("experimental_bearer_token")
                .and_then(Value::as_str),
            Some("sk-test")
        );
    }

    #[test]
    fn remote_control_enabled_accepts_legacy_setting() {
        assert!(remote_control_enabled_from_settings(&json!({
            "codex_remote_control_hook_enabled": true
        })));
    }

    #[test]
    fn remote_control_enabled_suspends_in_subscription_mode() {
        let settings = json!({
            "codex_remote_control_enabled": true,
            "codex_active_mode": "chatgpt"
        });

        assert!(remote_control_config_enabled_from_settings(&settings));
        assert!(!remote_control_enabled_from_settings(&settings));
    }

    #[test]
    fn missing_remote_control_account_is_distinguished_from_store_errors() {
        let settings = json!({
            "codex_remote_control_enabled": true,
            "codex_remote_control_account_id": "profile-missing",
            "codex_active_mode": "chatgpt"
        });

        let missing = remote_control_account_issue_with_lookup(&settings, |_| Ok(None))
            .expect("missing account should be a recoverable state");
        assert_eq!(
            missing,
            Some(RemoteControlAccountIssue::Missing(
                "profile-missing".to_string()
            ))
        );

        let err = remote_control_account_issue_with_lookup(
            &settings,
            |_| -> Result<Option<Value>, String> {
                Err("读取 accounts.json 失败: invalid json".to_string())
            },
        )
        .unwrap_err();
        assert_eq!(err, "读取 accounts.json 失败: invalid json");
    }

    #[test]
    fn enabled_remote_control_without_selection_is_reconciled_without_account_lookup() {
        let settings = json!({
            "codex_remote_control_enabled": true,
            "codex_remote_control_account_id": "",
            "codex_active_mode": "chatgpt"
        });

        let missing = remote_control_account_issue_with_lookup(
            &settings,
            |_| -> Result<Option<Value>, String> {
                panic!("empty selection must not read the account store")
            },
        )
        .unwrap();

        assert_eq!(
            missing,
            Some(RemoteControlAccountIssue::Missing(String::new()))
        );
    }

    #[test]
    fn invalidated_refresh_token_is_classified_as_login_expired() {
        let settings = json!({
            "codex_remote_control_enabled": true,
            "codex_remote_control_account_id": "profile-expired",
            "codex_active_mode": "api"
        });
        let account = json!({
            "custom": {
                "auth_status": "error",
                "auth_status_message": "HTTP 401: refresh_token_invalidated; Your session has ended. Please log in again."
            }
        });

        let issue = remote_control_account_issue_with_lookup(&settings, |_| Ok(Some(account)))
            .expect("invalidated refresh token should be classified");

        assert_eq!(
            issue,
            Some(RemoteControlAccountIssue::LoginExpired(
                "profile-expired".to_string()
            ))
        );
    }

    #[test]
    fn transient_auth_refresh_error_does_not_disable_remote_control() {
        let settings = json!({
            "codex_remote_control_enabled": true,
            "codex_remote_control_account_id": "profile-network-error",
            "codex_active_mode": "api"
        });
        let account = json!({
            "custom": {
                "auth_status": "error",
                "auth_status_message": "network timeout while refreshing token"
            }
        });

        let issue = remote_control_account_issue_with_lookup(&settings, |_| Ok(Some(account)))
            .expect("transient refresh errors should remain retryable");

        assert_eq!(issue, None);
    }

    // A copy of this test binary named codex.exe runs the sleeping process fixture; the extra
    // arguments are libtest filters that only put the helper flags on its command line.
    #[cfg(windows)]
    #[test]
    fn legacy_helper_scan_reads_real_codex_process_command_lines() {
        use std::process::{Command, Stdio};
        let dir =
            std::env::temp_dir().join(format!("codex-switch-legacy-helper-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let helper = dir.join("codex.exe");
        fs::copy(std::env::current_exe().unwrap(), &helper).unwrap();
        let spawn = |extra: &[&str]| {
            let mut command = Command::new(&helper);
            command
                .args([
                    "--exact",
                    "codex_launcher::process_control::tests::process_fixture",
                    "--nocapture",
                ])
                .args(extra)
                .env("CODEX_SWITCH_PROCESS_FIXTURE", "sleep")
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            super::super::shell::hide_command_window(&mut command);
            command.spawn().unwrap()
        };
        let mut legacy = spawn(&["app-server", "flag --enable remote_control"]);
        let mut desktop = spawn(&["app-server"]);

        let found = legacy_remote_control_helper_pids();

        for child in [&mut legacy, &mut desktop] {
            child.kill().unwrap();
            child.wait().unwrap();
        }
        let legacy_found = found.contains(&u64::from(legacy.id()));
        let desktop_found = found.contains(&u64::from(desktop.id()));
        // Windows keeps the image of a just-exited executable (and a scanner may hold a fresh
        // copy) locked for a moment, so removing the temp copy is retried. A copy that stays
        // behind in the temp directory does not change what the scan returned.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while let Err(err) = fs::remove_dir_all(&dir) {
            if std::time::Instant::now() >= deadline {
                eprintln!("fixture cleanup left {}: {err}", dir.display());
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(legacy_found, "{found:?}");
        assert!(!desktop_found, "{found:?}");
    }

    #[test]
    fn legacy_helper_matching_keeps_former_powershell_filter() {
        assert!(is_legacy_remote_control_helper_name("codex.exe"));
        assert!(is_legacy_remote_control_helper_name("CODEX.EXE"));
        assert!(!is_legacy_remote_control_helper_name("codex"));
        assert!(!is_legacy_remote_control_helper_name("ChatGPT.exe"));

        // Arguments exactly as the old helper spawn passed them.
        for command_line in [
            r"C:\App\resources\codex.exe app-server --listen ws://127.0.0.1:4500 --analytics-default-enabled --enable remote_control",
            r#""C:\App\resources\codex.exe" APP-SERVER --ENABLE    REMOTE_CONTROL"#,
            "codex.exe app-server --enable\tremote_control_v2",
            "codex.exe x--enable remote_control -- app-server",
        ] {
            assert!(
                is_legacy_remote_control_helper_command_line(command_line),
                "{command_line}"
            );
        }
        for command_line in [
            "codex.exe app-server --analytics-default-enabled",
            "codex.exe app-server2 --enable remote_control",
            "codex.exe myapp-server --enable remote_control",
            "codex.exe app_server --enable remote_control",
            "codex.exe app-server --enabled remote_control",
            "codex.exe app-server --enable=remote_control",
            "codex.exe app-server --enable",
            "",
        ] {
            assert!(
                !is_legacy_remote_control_helper_command_line(command_line),
                "{command_line}"
            );
        }
    }

    #[test]
    fn subscription_to_api_fallback_is_reported_for_restart_decisions() {
        assert!(!remote_control_sync_changed(false, false));
        assert!(remote_control_sync_changed(false, true));
        assert!(remote_control_sync_changed(true, false));
        assert!(missing_account_fallback_changes_live_runtime_target(
            RemoteControlRuntimeTarget::Subscription,
            RemoteControlRuntimeTarget::Api
        ));
        assert!(!missing_account_fallback_changes_live_runtime_target(
            RemoteControlRuntimeTarget::MixedApi,
            RemoteControlRuntimeTarget::Api
        ));
    }

    #[test]
    fn disabled_remote_control_preserves_subscription_runtime_target() {
        let settings = json!({
            "codex_remote_control_enabled": false,
            "codex_active_mode": "chatgpt"
        });

        assert_eq!(
            remote_control_runtime_target(&settings),
            RemoteControlRuntimeTarget::Subscription
        );
    }

    #[test]
    fn auto_disabled_status_response_carries_settings_and_runtime_state() {
        let settings = json!({
            "codex_remote_control_enabled": false,
            "codex_remote_control_account_id": "",
            "codex_active_mode": "api"
        });
        let response = attach_remote_control_auto_disabled_response(
            json!({ "ok": true }),
            settings.clone(),
            REMOTE_CONTROL_MISSING_ACCOUNT_AUTO_DISABLE_MESSAGE,
            true,
            true,
            Some("runtime failed".to_string()),
            Some("process check failed".to_string()),
        );

        assert_eq!(
            response.get("autoDisabled").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(response.get("settings"), Some(&settings));
        assert_eq!(
            response.get("message").and_then(Value::as_str),
            Some(REMOTE_CONTROL_MISSING_ACCOUNT_AUTO_DISABLE_MESSAGE)
        );
        assert_eq!(
            response.get("runtimeError").and_then(Value::as_str),
            Some("runtime failed")
        );
        assert_eq!(
            response.get("processStatusError").and_then(Value::as_str),
            Some("process check failed")
        );
    }

    #[test]
    fn remote_control_status_detects_login_expired() {
        let status = json!({
            "state": "warning",
            "status": "login_expired",
            "message": "控制账号登录已过期，请重新登录"
        });

        assert!(remote_control_status_is_login_expired(&status));
    }
}
