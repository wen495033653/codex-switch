use super::state::{build_error_state, normalize_subscription, normalize_usage_info};
use crate::{
    accounts::{CHATGPT_SUBSCRIPTION_ENDPOINT, CHATGPT_USAGE_ENDPOINT},
    json_util::raw_string_field,
};
use serde_json::Value;
use std::time::Duration as StdDuration;

const CODEX_USER_AGENT: &str = "codex_cli_rs/0.76.0 (Debian 13.0.0; x86_64) WindowsTerminal";
const SUBSCRIPTION_PATH: &str = "/backend-api/subscriptions";

pub(crate) fn parse_endpoint_error(status: u16, text: &str) -> (String, String, String) {
    let parsed: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let err_obj = parsed
        .get("error")
        .filter(|value| value.is_object())
        .unwrap_or(&parsed);
    let err_type = raw_string_field(err_obj, "type");
    let err_code = raw_string_field(err_obj, "code")
        .chars()
        .next()
        .map(|_| raw_string_field(err_obj, "code"))
        .unwrap_or_else(|| raw_string_field(&parsed, "error"));
    let err_message = raw_string_field(err_obj, "message")
        .chars()
        .next()
        .map(|_| raw_string_field(err_obj, "message"))
        .unwrap_or_else(|| raw_string_field(&parsed, "error_description"));

    let message = if err_code == "deactivated_workspace" {
        "Workspace has been deactivated".to_string()
    } else if !err_message.is_empty() {
        err_message.clone()
    } else if status == 401 || status == 403 {
        "Authorization expired, please sign in again".to_string()
    } else if status == 429 {
        "Too many requests, please try again later".to_string()
    } else if status >= 500 {
        "Service temporarily unavailable, please try again later".to_string()
    } else {
        "Request failed, please try again later".to_string()
    };

    let raw_message = if !err_message.is_empty() {
        err_message
    } else {
        text.replace(char::is_whitespace, " ")
            .trim()
            .chars()
            .take(300)
            .collect()
    };
    let code = if !err_code.is_empty() {
        err_code
    } else {
        err_type
    };
    (message, code, raw_message)
}

pub(crate) fn get_usage(
    access_token: &str,
    account_id: &str,
    timeout_ms: u64,
) -> Result<Value, Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(StdDuration::from_millis(timeout_ms))
        .build()
        .map_err(|err| {
            let message = err.to_string();
            build_error_state(
                &message,
                "usage_sync_failed",
                "",
                0,
                "/backend-api/wham/usage",
            )
        })?;
    let response = client
        .get(CHATGPT_USAGE_ENDPOINT)
        .bearer_auth(access_token)
        .header("Content-Type", "application/json")
        .header("User-Agent", CODEX_USER_AGENT)
        .header("chatgpt-account-id", account_id)
        .send()
        .map_err(|err| {
            let message = err.to_string();
            build_error_state(
                &message,
                "usage_sync_failed",
                "",
                0,
                "/backend-api/wham/usage",
            )
        })?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().unwrap_or_default();
        let (message, code, raw_message) = parse_endpoint_error(status.as_u16(), &text);
        return Err(build_error_state(
            &message,
            if code.is_empty() {
                "usage_sync_failed"
            } else {
                &code
            },
            &raw_message,
            status.as_u16(),
            "/backend-api/wham/usage",
        ));
    }

    let data: Value = response.json().map_err(|err| {
        let message = format!("Failed to parse quota response: {err}");
        build_error_state(
            &message,
            "usage_sync_failed",
            "",
            0,
            "/backend-api/wham/usage",
        )
    })?;
    let usage = normalize_usage_info(Some(&data));
    if usage.is_null() {
        return Err(build_error_state(
            "配额响应缺少 rate_limit",
            "usage_sync_failed",
            "",
            0,
            "/backend-api/wham/usage",
        ));
    }
    Ok(usage)
}

/// Reads the authoritative subscription snapshot for one ChatGPT account.
///
/// Cloudflare answers this path with an HTML challenge unless the request carries the Codex
/// client identity headers and speaks HTTP/1.1. Verified on 2026-09-14 against a live
/// account: the same request returned HTTP 403 HTML over HTTP/2 and 200 over HTTP/1.1, and
/// dropping `Accept`, `ChatGPT-Account-ID`, `OpenAI-Beta` or the Codex `User-Agent`
/// produced intermittent 403s. `/wham/usage` has no such restriction and keeps HTTP/2.
pub(crate) fn get_subscription(
    access_token: &str,
    account_id: &str,
    timeout_ms: u64,
) -> Result<Value, Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(StdDuration::from_millis(timeout_ms))
        .http1_only()
        .build()
        .map_err(|err| {
            build_error_state(
                &err.to_string(),
                "subscription_sync_failed",
                "",
                0,
                SUBSCRIPTION_PATH,
            )
        })?;
    let mut url = url::Url::parse(CHATGPT_SUBSCRIPTION_ENDPOINT).map_err(|err| {
        build_error_state(
            &format!("订阅接口地址无效: {err}"),
            "subscription_sync_failed",
            "",
            0,
            SUBSCRIPTION_PATH,
        )
    })?;
    url.query_pairs_mut().append_pair("account_id", account_id);
    let response = client
        .get(url)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .header("ChatGPT-Account-ID", account_id)
        .header("OpenAI-Beta", "codex-1")
        .header("Originator", "Codex Desktop")
        .header("User-Agent", CODEX_USER_AGENT)
        .send()
        .map_err(|err| {
            build_error_state(
                &err.to_string(),
                "subscription_sync_failed",
                "",
                0,
                SUBSCRIPTION_PATH,
            )
        })?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().unwrap_or_default();
        let (message, code, raw_message) = parse_endpoint_error(status.as_u16(), &text);
        return Err(build_error_state(
            &message,
            if code.is_empty() {
                "subscription_sync_failed"
            } else {
                &code
            },
            &raw_message,
            status.as_u16(),
            SUBSCRIPTION_PATH,
        ));
    }

    let data: Value = response.json().map_err(|err| {
        build_error_state(
            &format!("Failed to parse subscription response: {err}"),
            "subscription_sync_failed",
            "",
            0,
            SUBSCRIPTION_PATH,
        )
    })?;
    let subscription = normalize_subscription(Some(&data));
    if subscription.is_null() {
        return Err(build_error_state(
            "订阅响应缺少 active_until",
            "subscription_sync_failed",
            "",
            0,
            SUBSCRIPTION_PATH,
        ));
    }
    Ok(subscription)
}
