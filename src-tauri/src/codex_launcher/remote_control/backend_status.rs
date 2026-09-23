use crate::json_util::string_field;
use serde_json::{json, Value};
use std::time::Duration as StdDuration;

const REMOTE_CONTROL_ENVIRONMENTS_ENDPOINT: &str =
    "https://chatgpt.com/backend-api/wham/remote/control/environments";

const REMOTE_CONTROL_BACKEND_STATUS_TIMEOUT_MS: u64 = 6_000;

const REMOTE_CONTROL_BACKEND_ERROR_TEXT_MAX_LEN: usize = 1800;

pub(super) fn fetch_remote_control_backend_environment_status(
    account: &Value,
) -> Result<Value, String> {
    let tokens = account
        .get("tokens")
        .ok_or_else(|| "远程控制订阅账号缺少 tokens".to_string())?;
    let access_token = string_field(tokens, "access_token");
    if access_token.is_empty() {
        return Err("远程控制订阅账号缺少 tokens.access_token".to_string());
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
    // An unread body would otherwise look like an empty one: "HTTP 500 body: " or a JSON parse
    // error that hides the real read failure.
    let text = response.text().map_err(|err| {
        format!(
            "读取 ChatGPT 桌面状态响应失败: HTTP {}: {err}",
            status.as_u16()
        )
    })?;
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
            || text.contains("refresh_token_invalidated")
            || text.contains("session has ended")
            || text.contains("authentication token is expired")
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

pub(super) fn truncate_remote_control_error_text(text: &str) -> String {
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

pub(super) fn remote_control_status_from_backend_environment(environment: &Value) -> Option<Value> {
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
            "message": "codex 未找到",
            "title": remote_control_environment_status_title(environment)
        })),
        Some("found") => {
            let title = remote_control_environment_status_title(environment);
            if environment.get("online").and_then(Value::as_bool) == Some(true) {
                return Some(json!({
                    "state": "active",
                    "status": "desktop_online",
                    "message": "codex 在线",
                    "title": title
                }));
            }
            Some(json!({
                "state": "warning",
                "status": "desktop_offline",
                "message": "codex 未打开",
                "title": title
            }))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            Some("codex 在线")
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
            Some("codex 未打开")
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
            Some("codex 未找到")
        );
    }

    #[test]
    fn backend_error_message_detects_expired_authentication_token() {
        let raw = r#"HTTP 401 body: {"detail":"Provided authentication token is expired. Please try signing in again."}"#;

        assert_eq!(
            remote_control_backend_error_message(None, raw),
            Some(("login_expired", "控制账号登录已过期，请重新登录"))
        );
        assert_eq!(
            remote_control_backend_error_message(
                None,
                "HTTP 401: refresh_token_invalidated; Your session has ended."
            ),
            Some(("login_expired", "控制账号登录已过期，请重新登录"))
        );
    }
}
