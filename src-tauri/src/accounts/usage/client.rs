//! HTTP reads against the ChatGPT backend. Every endpoint this app talks to lives here,
//! together with the client identity Cloudflare expects in front of those paths.

use super::state::{build_error_state, normalize_subscription, normalize_usage_info};
use crate::json_util::raw_string_field;
use reqwest::blocking::{Client, RequestBuilder};
use serde_json::Value;
use std::time::Duration as StdDuration;

/// Codex CLI identity. Cloudflare only lets `/backend-api/subscriptions` through for this
/// user agent, so it stays pinned rather than following the real Codex release number.
const CODEX_USER_AGENT: &str = "codex_cli_rs/0.76.0 (Debian 13.0.0; x86_64) WindowsTerminal";

struct ChatgptEndpoint {
    url: &'static str,
    path: &'static str,
    label: &'static str,
    failure_code: &'static str,
}

const USAGE_ENDPOINT: ChatgptEndpoint = ChatgptEndpoint {
    url: "https://chatgpt.com/backend-api/wham/usage",
    path: "/backend-api/wham/usage",
    label: "quota",
    failure_code: "usage_sync_failed",
};

const SUBSCRIPTION_ENDPOINT: ChatgptEndpoint = ChatgptEndpoint {
    url: "https://chatgpt.com/backend-api/subscriptions",
    path: "/backend-api/subscriptions",
    label: "subscription",
    failure_code: "subscription_sync_failed",
};

impl ChatgptEndpoint {
    fn failure(&self, message: &str) -> Value {
        build_error_state(message, self.failure_code, "", 0, self.path)
    }

    fn rejected(&self, status: u16, body: &str) -> Value {
        let error = parse_endpoint_error(status, body);
        let code = if error.code.is_empty() {
            self.failure_code
        } else {
            &error.code
        };
        build_error_state(&error.message, code, &error.raw_message, status, self.path)
    }

    /// Sends the request and returns the JSON body; every failure becomes an error state
    /// tagged with this endpoint's path so the UI and logs can tell the two reads apart.
    fn fetch_json(&self, request: Result<RequestBuilder, reqwest::Error>) -> Result<Value, Value> {
        let response = request
            .and_then(RequestBuilder::send)
            .map_err(|err| self.failure(&err.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            return Err(self.rejected(status.as_u16(), &body));
        }
        response
            .json()
            .map_err(|err| self.failure(&format!("Failed to parse {} response: {err}", self.label)))
    }
}

fn blocking_client(timeout_ms: u64, http1_only: bool) -> Result<Client, reqwest::Error> {
    let mut builder = Client::builder().timeout(StdDuration::from_millis(timeout_ms));
    if http1_only {
        builder = builder.http1_only();
    }
    builder.build()
}

pub(crate) struct EndpointError {
    pub(crate) message: String,
    pub(crate) code: String,
    pub(crate) raw_message: String,
}

pub(crate) fn parse_endpoint_error(status: u16, text: &str) -> EndpointError {
    let parsed: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let err_obj = parsed
        .get("error")
        .filter(|value| value.is_object())
        .unwrap_or(&parsed);
    let err_type = raw_string_field(err_obj, "type");
    let err_code = {
        let code = raw_string_field(err_obj, "code");
        if code.is_empty() {
            raw_string_field(&parsed, "error")
        } else {
            code
        }
    };
    let err_message = {
        let message = raw_string_field(err_obj, "message");
        if message.is_empty() {
            raw_string_field(&parsed, "error_description")
        } else {
            message
        }
    };

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
    EndpointError {
        message,
        code,
        raw_message,
    }
}

pub(crate) fn get_usage(
    access_token: &str,
    account_id: &str,
    timeout_ms: u64,
) -> Result<Value, Value> {
    let request = blocking_client(timeout_ms, false).map(|client| {
        client
            .get(USAGE_ENDPOINT.url)
            .bearer_auth(access_token)
            .header("Content-Type", "application/json")
            .header("User-Agent", CODEX_USER_AGENT)
            .header("chatgpt-account-id", account_id)
    });
    let data = USAGE_ENDPOINT.fetch_json(request)?;
    let usage = normalize_usage_info(Some(&data));
    if usage.is_null() {
        return Err(USAGE_ENDPOINT.failure("配额响应缺少 rate_limit"));
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
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("account_id", account_id)
        .finish();
    let request = blocking_client(timeout_ms, true).map(|client| {
        client
            .get(format!("{}?{query}", SUBSCRIPTION_ENDPOINT.url))
            .bearer_auth(access_token)
            .header("Accept", "application/json")
            .header("ChatGPT-Account-ID", account_id)
            .header("OpenAI-Beta", "codex-1")
            .header("Originator", "Codex Desktop")
            .header("User-Agent", CODEX_USER_AGENT)
    });
    let data = SUBSCRIPTION_ENDPOINT.fetch_json(request)?;
    let subscription = normalize_subscription(Some(&data));
    if subscription.is_null() {
        return Err(SUBSCRIPTION_ENDPOINT.failure("订阅响应缺少 active_until"));
    }
    Ok(subscription)
}
