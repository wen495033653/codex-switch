use super::{json_as_array, kill_process_tree, parse_json_output, run_pwsh};
use crate::{
    accounts::{
        find_store_account, profile_id_from_account, profile_id_from_tokens_value,
        read_api_key_from_auth, read_api_key_from_provider_config, read_auth_value, set_api_mode,
        write_account_auth,
    },
    api_config::API_PROVIDER_ID,
    codex_config::{
        read_root_config, read_table_config, remove_config_values, remove_remote_control_config,
        remove_table_config, set_config_values, set_table_config,
    },
    json_util::{bool_field, string_field},
    paths::app_data_dir,
    session_sync_diagnostics::log_session_sync_event,
    settings::{default_api_mode, read_settings_value, update_settings_value},
};
use serde_json::{json, Value};
use std::{fs, time::Duration as StdDuration};

const REMOTE_CONTROL_ENABLED_SETTING_KEY: &str = "codex_remote_control_enabled";
const LEGACY_REMOTE_CONTROL_HOOK_SETTING_KEY: &str = "codex_remote_control_hook_enabled";
const REMOTE_CONTROL_ACCOUNT_SETTING_KEY: &str = "codex_remote_control_account_id";
const API_WIRE: &str = "responses";
const REMOTE_CONTROL_ENVIRONMENTS_ENDPOINT: &str =
    "https://chatgpt.com/backend-api/wham/remote/control/environments";
const REMOTE_CONTROL_BACKEND_STATUS_TIMEOUT_MS: u64 = 6_000;
const REMOTE_CONTROL_BACKEND_ERROR_TEXT_MAX_LEN: usize = 1800;

pub(crate) fn remote_control_enabled_from_settings(settings: &Value) -> bool {
    bool_field(settings, REMOTE_CONTROL_ENABLED_SETTING_KEY)
        || bool_field(settings, LEGACY_REMOTE_CONTROL_HOOK_SETTING_KEY)
}

fn remote_control_account_id_from_settings(settings: &Value) -> String {
    string_field(settings, REMOTE_CONTROL_ACCOUNT_SETTING_KEY)
}

fn remote_control_account(account_id: &str) -> Result<Value, String> {
    if account_id.trim().is_empty() {
        return Err("app远程控制需要先单独选择一个订阅账号".to_string());
    }
    find_store_account(account_id)
        .map_err(|_| format!("app远程控制账号不存在，请重新选择: {account_id}"))
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
        return Err("app远程控制会话流量走 API 需要先配置 API 模式 base_url".to_string());
    }

    let api_key = api_key_from_settings_or_runtime(&api_mode);
    if api_key.trim().is_empty() {
        return Err("app远程控制会话流量走 API 需要先配置 API Key".to_string());
    }

    Ok((base_url, api_key))
}

fn validate_remote_control_enable_prerequisites() -> Result<(), String> {
    let settings = read_settings_value()?;
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
    log_session_sync_event(
        "codex_remote_control_runtime_applied",
        json!({
            "mode": "api_remote_control",
            "remoteControl": true,
            "accountId": account_id,
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
        .map_err(|err| format!("删除旧 app远程控制 home 失败 {}: {err}", home.display()))?;
    Ok(true)
}

fn legacy_remote_control_helper_pids() -> Vec<u64> {
    if !cfg!(windows) {
        return Vec::new();
    }

    let script = r#"
$ErrorActionPreference = "Stop"
$helpers = Get-CimInstance Win32_Process | Where-Object {
  $_.Name -ieq "codex.exe" `
    -and $_.CommandLine -match "\bapp-server\b" `
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
        .filter_map(|pid| pid.as_u64())
        .collect()
}

fn stop_legacy_remote_control_helpers() -> usize {
    legacy_remote_control_helper_pids()
        .into_iter()
        .filter(|pid| kill_process_tree(*pid))
        .count()
}

fn cleanup_legacy_remote_control_runtime() -> Result<bool, String> {
    let stopped = stop_legacy_remote_control_helpers();
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

fn active_auth_matches_remote_control_account(settings: &Value) -> bool {
    let account_id = remote_control_account_id_from_settings(settings);
    if account_id.is_empty() {
        return false;
    }
    let Ok(account) = remote_control_account(&account_id) else {
        return false;
    };
    let Ok(expected_profile_id) = profile_id_from_account(&account) else {
        return false;
    };

    let Ok(auth) = read_auth_value() else {
        return false;
    };
    if string_field(&auth, "auth_mode") != "chatgpt" {
        return false;
    }

    profile_id_from_tokens_value(auth.get("tokens"))
        .map(|profile_id| profile_id == expected_profile_id)
        .unwrap_or(false)
}

fn remote_control_mixed_config_applied(settings: &Value) -> bool {
    let Ok((api_base_url, api_key)) = remote_control_api_session_profile_from_settings(settings)
    else {
        return false;
    };
    if !active_auth_matches_remote_control_account(settings) {
        return false;
    }

    let Ok(root_config) = read_root_config() else {
        return false;
    };
    if root_config.get("model_provider").and_then(Value::as_str) != Some(API_PROVIDER_ID) {
        return false;
    }

    let Ok(provider) = read_table_config(&format!("model_providers.{API_PROVIDER_ID}")) else {
        return false;
    };

    provider_string_field(&provider, "base_url") == api_base_url
        && provider_string_field(&provider, "wire_api") == API_WIRE
        && provider_bool_field(&provider, "requires_openai_auth")
        && provider_string_field(&provider, "experimental_bearer_token") == api_key
}

fn remote_control_mixed_config_present() -> bool {
    let Ok(root_config) = read_root_config() else {
        return false;
    };
    if root_config.get("model_provider").and_then(Value::as_str) != Some(API_PROVIDER_ID) {
        return false;
    }
    read_table_config(&format!("model_providers.{API_PROVIDER_ID}"))
        .ok()
        .is_some_and(|provider| {
            !provider_string_field(&provider, "experimental_bearer_token").is_empty()
                && provider_bool_field(&provider, "requires_openai_auth")
        })
}

pub(crate) fn preview_remote_control_runtime_for_current_settings(
    _trigger: &str,
) -> Result<bool, String> {
    let settings = read_settings_value()?;
    if remote_control_enabled_from_settings(&settings) {
        validate_remote_control_enable_prerequisites()?;
        return Ok(!remote_control_mixed_config_applied(&settings));
    }

    Ok(remote_control_mixed_config_present()
        || legacy_remote_control_home_removed_pending()
        || !legacy_remote_control_helper_pids().is_empty())
}

fn legacy_remote_control_home_removed_pending() -> bool {
    app_data_dir()
        .map(|dir| dir.join("remote-control-codex-home").exists())
        .unwrap_or(false)
}

pub(crate) fn sync_remote_control_runtime_for_current_settings(
    context: &str,
) -> Result<bool, String> {
    let settings = read_settings_value()?;
    let mut changed = cleanup_legacy_remote_control_runtime()?;

    if remote_control_enabled_from_settings(&settings) {
        let pending = !remote_control_mixed_config_applied(&settings);
        apply_remote_control_mixed_config(&settings)?;
        changed |= pending;
    } else {
        let pending = remote_control_mixed_config_present();
        restore_api_config_after_remote_control_disabled(&settings)?;
        remove_remote_control_config()?;
        changed |= pending;
    }

    if changed {
        log_session_sync_event(
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

pub(crate) fn restart_remote_control_runtime_for_current_settings(
    context: &str,
) -> Result<bool, String> {
    sync_remote_control_runtime_for_current_settings(context)
}

fn remote_control_backend_environment_status(settings: &Value) -> Option<Value> {
    if !remote_control_enabled_from_settings(settings)
        || !remote_control_mixed_config_applied(settings)
    {
        return None;
    }

    match fetch_remote_control_backend_environment_status(settings) {
        Ok(status) => Some(status),
        Err(err) => Some(json!({
            "status": "lookup_failed",
            "message": "桌面状态查询失败",
            "raw": truncate_remote_control_error_text(&err)
        })),
    }
}

fn fetch_remote_control_backend_environment_status(settings: &Value) -> Result<Value, String> {
    let account_id = remote_control_account_id_from_settings(settings);
    let account = remote_control_account(&account_id)?;
    let tokens = account
        .get("tokens")
        .ok_or_else(|| "app远程控制订阅账号缺少 tokens".to_string())?;
    let access_token = string_field(tokens, "access_token");
    if access_token.is_empty() {
        return Err("app远程控制订阅账号缺少 tokens.access_token".to_string());
    }
    let chatgpt_account_id = string_field(tokens, "account_id");
    let display_names = remote_control_local_display_names();
    if display_names.is_empty() {
        return Ok(json!({
            "status": "missing",
            "message": "无法读取本机设备名，不能匹配桌面状态"
        }));
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(StdDuration::from_millis(
            REMOTE_CONTROL_BACKEND_STATUS_TIMEOUT_MS,
        ))
        .build()
        .map_err(|err| format!("创建 ChatGPT 桌面状态客户端失败: {err}"))?;
    let mut request = client
        .get(REMOTE_CONTROL_ENVIRONMENTS_ENDPOINT)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header(
            "User-Agent",
            format!("codex-switch/{}", env!("CARGO_PKG_VERSION")),
        );
    if !chatgpt_account_id.is_empty() {
        request = request.header("chatgpt-account-id", chatgpt_account_id);
    }

    let response = request
        .send()
        .map_err(|err| format!("读取 ChatGPT 桌面状态失败: {err}"))?;
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if !status.is_success() {
        let raw = format!("HTTP {} body: {text}", status.as_u16());
        if let Some((kind, message)) = remote_control_backend_error_message(None, &raw) {
            return Ok(json!({
                "status": "errored",
                "kind": kind,
                "message": message,
                "raw": truncate_remote_control_error_text(&raw)
            }));
        }
        return Err(raw);
    }

    let data: Value =
        serde_json::from_str(&text).map_err(|err| format!("解析 ChatGPT 桌面状态失败: {err}"))?;
    remote_control_backend_environment_summary_from_items(&data, &display_names)
}

fn remote_control_local_display_names() -> Vec<String> {
    let mut names = Vec::new();
    for key in ["COMPUTERNAME", "HOSTNAME"] {
        let name = std::env::var(key).unwrap_or_default().trim().to_string();
        if !name.is_empty()
            && !names
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&name))
        {
            names.push(name);
        }
    }
    names
}

fn remote_control_backend_environment_summary_from_items(
    data: &Value,
    display_names: &[String],
) -> Result<Value, String> {
    let items = data
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| "ChatGPT 桌面状态响应缺少 items".to_string())?;
    let display_name_matches = |item: &&Value| {
        let display_name = string_field(item, "display_name");
        display_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&display_name))
    };
    let current = items
        .iter()
        .filter(display_name_matches)
        .max_by_key(|item| {
            (
                item.get("online").and_then(Value::as_bool) == Some(true),
                remote_control_environment_is_codex_desktop(item),
                string_field(item, "last_seen_at"),
            )
        });

    let Some(current) = current else {
        return Ok(json!({
            "status": "missing",
            "message": "ChatGPT 后端没有找到这台桌面",
            "localDisplayNames": display_names
        }));
    };

    let display_name = string_field(current, "display_name");
    let client_name = string_field(current, "client_name");
    let same_display_name_count = items
        .iter()
        .filter(|item| string_field(item, "display_name").eq_ignore_ascii_case(&display_name))
        .count();
    let offline_same_display_name_count = items
        .iter()
        .filter(|item| {
            string_field(item, "display_name").eq_ignore_ascii_case(&display_name)
                && item.get("online").and_then(Value::as_bool) == Some(false)
        })
        .count();

    Ok(json!({
        "status": "found",
        "environmentId": current.get("env_id").cloned().unwrap_or(Value::Null),
        "displayName": display_name,
        "online": current.get("online").cloned().unwrap_or(Value::Null),
        "installationId": current.get("installation_id").cloned().unwrap_or(Value::Null),
        "clientType": current.get("client_type").cloned().unwrap_or(Value::Null),
        "originator": current.get("originator").cloned().unwrap_or(Value::Null),
        "clientName": client_name,
        "lastSeenAt": current.get("last_seen_at").cloned().unwrap_or(Value::Null),
        "sameDisplayNameCount": same_display_name_count,
        "offlineSameDisplayNameCount": offline_same_display_name_count
    }))
}

fn remote_control_environment_is_codex_desktop(item: &Value) -> bool {
    let text = [
        string_field(item, "client_name"),
        string_field(item, "originator"),
        string_field(item, "client_type"),
    ]
    .join(" ")
    .to_ascii_lowercase();
    text.contains("codex desktop") || text.contains("codex_desktop")
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

fn remote_control_backend_error_message(
    marker_kind: Option<&str>,
    raw_text: &str,
) -> Option<(&'static str, &'static str)> {
    let text = raw_text.to_ascii_lowercase();
    match marker_kind {
        Some("mfa_required") => Some(("mfa_required", "需要先为当前账号完成 MFA 认证")),
        Some("login_expired") => Some(("login_expired", "控制账号登录已过期，请重新登录")),
        Some("enrollment_failed") => Some(("enrollment_failed", "远程控制连接失败")),
        _ if text.contains("multi-factor authentication required") => {
            Some(("mfa_required", "需要先为当前账号完成 MFA 认证"))
        }
        _ if text.contains("refresh_token_reused")
            || text.contains("refresh token has already been used")
            || text.contains("please log out and sign in again") =>
        {
            Some(("login_expired", "控制账号登录已过期，请重新登录"))
        }
        _ if text.contains("remote control server enrollment failed")
            || text.contains("enrollment failed")
            || text.contains("http 403 forbidden") =>
        {
            Some(("enrollment_failed", "远程控制连接失败"))
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

fn remote_control_status_from_backend_environment(environment: &Value) -> Option<Value> {
    match environment.get("status").and_then(Value::as_str) {
        Some("errored") => {
            let kind = environment
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("backend_error");
            Some(json!({
                "state": remote_control_backend_issue_state(kind),
                "status": kind,
                "message": environment.get("message").cloned().unwrap_or_else(|| json!("桌面状态查询失败")),
                "raw": environment.get("raw").cloned().unwrap_or(Value::Null)
            }))
        }
        Some("lookup_failed") => Some(json!({
            "state": "warning",
            "status": "backend_lookup_failed",
            "message": environment.get("message").cloned().unwrap_or_else(|| json!("桌面状态查询失败")),
            "raw": environment.get("raw").cloned().unwrap_or(Value::Null)
        })),
        Some("missing") => Some(json!({
            "state": "warning",
            "status": "desktop_missing",
            "message": "codex app 未找到",
            "title": remote_control_environment_status_title(environment)
        })),
        Some("found") => {
            let title = remote_control_environment_status_title(environment);
            if environment.get("online").and_then(Value::as_bool) == Some(true) {
                return Some(json!({
                    "state": "active",
                    "status": "desktop_online",
                    "message": "codex app 在线",
                    "title": title
                }));
            }
            Some(json!({
                "state": "warning",
                "status": "desktop_offline",
                "message": "codex app 未打开",
                "title": title
            }))
        }
        _ => None,
    }
}

fn remote_control_status_value(settings: &Value, backend_environment: Option<&Value>) -> Value {
    let enabled = remote_control_enabled_from_settings(settings);
    if !enabled {
        return json!({
            "state": "muted",
            "status": "disabled",
            "message": "未启用"
        });
    }

    let account_id = remote_control_account_id_from_settings(settings);
    if account_id.is_empty() {
        return json!({
            "state": "error",
            "status": "missing_account",
            "message": "需要选择订阅账号"
        });
    }
    if let Err(err) = validate_remote_control_account_id(&account_id) {
        return json!({
            "state": "error",
            "status": "invalid_account",
            "message": "订阅账号无效",
            "raw": err
        });
    }
    if let Err(err) = remote_control_api_session_profile_from_settings(settings) {
        return json!({
            "state": "error",
            "status": "missing_api",
            "message": "缺少 API 配置",
            "raw": err
        });
    }
    if remote_control_mixed_config_applied(settings) {
        if let Some(environment) = backend_environment {
            if let Some(status) = remote_control_status_from_backend_environment(environment) {
                return status;
            }
        }
        return json!({
            "state": "active",
            "status": "applied",
            "message": "配置已应用"
        });
    }

    json!({
        "state": "warning",
        "status": "pending_restart",
        "message": "重启 Codex app 后生效",
        "raw": "远程控制配置待应用"
    })
}

#[tauri::command]
pub(crate) fn get_codex_remote_control_status() -> Result<Value, String> {
    let settings = read_settings_value()?;
    let backend_environment = remote_control_backend_environment_status(&settings);
    let connection_status = remote_control_status_value(&settings, backend_environment.as_ref());
    Ok(json!({
        "ok": true,
        "enabled": remote_control_enabled_from_settings(&settings),
        "accountId": remote_control_account_id_from_settings(&settings),
        "backendEnvironment": backend_environment,
        "connectionStatus": connection_status
    }))
}

#[tauri::command]
pub(crate) fn set_codex_remote_control_enabled(enabled: bool) -> Result<Value, String> {
    let codex_app_running =
        !super::codex_app_watcher::refresh_current_codex_app_processes()?.is_empty();
    if enabled {
        validate_remote_control_enable_prerequisites()?;
    }

    let settings = update_settings_value(&json!({
        REMOTE_CONTROL_ENABLED_SETTING_KEY: enabled,
        "codex_active_mode": "api"
    }))?;
    let settings = super::codex_app::apply_codex_proxy_env_state_to_settings(settings)?;
    let changed =
        sync_remote_control_runtime_for_current_settings("set_codex_remote_control_enabled")?;
    let restart_required = codex_app_running && changed;

    Ok(json!({
        "ok": true,
        "message": if enabled {
            if restart_required {
                "app远程控制已启用，重启 Codex app 后生效"
            } else {
                "app远程控制已启用"
            }
        } else if restart_required {
            "app远程控制已关闭，重启 Codex app 后恢复 API 模式"
        } else {
            "app远程控制已关闭"
        },
        "settings": settings,
        "changed": changed,
        "restartRequired": restart_required,
        "configDeferred": false
    }))
}

#[tauri::command]
pub(crate) fn set_codex_remote_control_account_id(id: String) -> Result<Value, String> {
    let account_id = id.trim();
    validate_remote_control_account_id(account_id)?;

    let codex_app_running =
        !super::codex_app_watcher::refresh_current_codex_app_processes()?.is_empty();
    let settings = update_settings_value(&json!({
        REMOTE_CONTROL_ACCOUNT_SETTING_KEY: account_id
    }))?;
    let settings = super::codex_app::apply_codex_proxy_env_state_to_settings(settings)?;
    let changed = if remote_control_enabled_from_settings(&settings) {
        sync_remote_control_runtime_for_current_settings("set_codex_remote_control_account_id")?
    } else {
        false
    };
    let restart_required = codex_app_running && changed;

    Ok(json!({
        "ok": true,
        "message": if restart_required {
            "app远程控制账号已更新，重启 Codex app 后生效"
        } else {
            "app远程控制账号已更新"
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
    fn backend_environment_summary_prefers_online_desktop_and_counts_offline_duplicates() {
        let data = json!({
            "items": [
                {
                    "env_id": "env_old",
                    "display_name": "DESKTOP-2KU3M74",
                    "online": false,
                    "client_name": "Codex Desktop",
                    "last_seen_at": "2026-06-14T01:00:00Z"
                },
                {
                    "env_id": "env_current",
                    "display_name": "DESKTOP-2KU3M74",
                    "online": true,
                    "originator": "Codex Desktop",
                    "client_name": "Codex Desktop",
                    "last_seen_at": "2026-06-14T02:00:00Z"
                }
            ]
        });
        let status = remote_control_backend_environment_summary_from_items(
            &data,
            &[String::from("desktop-2ku3m74")],
        )
        .expect("backend items should be summarized");

        assert_eq!(
            status.get("environmentId").and_then(Value::as_str),
            Some("env_current")
        );
        assert_eq!(status.get("online").and_then(Value::as_bool), Some(true));
        assert_eq!(
            status
                .get("offlineSameDisplayNameCount")
                .and_then(Value::as_u64),
            Some(1)
        );
    }

    #[test]
    fn backend_environment_status_reports_online_with_duplicate_hint() {
        let environment = json!({
            "status": "found",
            "environmentId": "env_current",
            "displayName": "DESKTOP-2KU3M74",
            "online": true,
            "clientName": "Codex Desktop",
            "lastSeenAt": "2026-06-14T02:00:00Z",
            "offlineSameDisplayNameCount": 2
        });
        let status = remote_control_status_from_backend_environment(&environment)
            .expect("found environment should map to a connection status");

        assert_eq!(status.get("state").and_then(Value::as_str), Some("active"));
        assert_eq!(
            status.get("status").and_then(Value::as_str),
            Some("desktop_online")
        );
        assert_eq!(
            status.get("message").and_then(Value::as_str),
            Some("codex app 在线")
        );
        assert!(status.get("raw").is_none());
    }

    #[test]
    fn backend_environment_status_reports_offline_as_warning() {
        let environment = json!({
            "status": "found",
            "environmentId": "env_current",
            "displayName": "DESKTOP-2KU3M74",
            "online": false,
            "clientName": "Codex Desktop"
        });
        let status = remote_control_status_from_backend_environment(&environment)
            .expect("found environment should map to a connection status");

        assert_eq!(status.get("state").and_then(Value::as_str), Some("warning"));
        assert_eq!(
            status.get("status").and_then(Value::as_str),
            Some("desktop_offline")
        );
        assert_eq!(
            status.get("message").and_then(Value::as_str),
            Some("codex app 未打开")
        );
    }

    #[test]
    fn backend_environment_status_reports_missing_as_warning() {
        let environment = json!({
            "status": "missing",
            "message": "ChatGPT 后端没有找到这台桌面"
        });
        let status = remote_control_status_from_backend_environment(&environment)
            .expect("missing environment should map to a connection status");

        assert_eq!(status.get("state").and_then(Value::as_str), Some("warning"));
        assert_eq!(
            status.get("status").and_then(Value::as_str),
            Some("desktop_missing")
        );
        assert_eq!(
            status.get("message").and_then(Value::as_str),
            Some("codex app 未找到")
        );
    }
}
