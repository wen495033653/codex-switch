use super::{error::normalize_error_state, usage_info::normalize_usage_info};
use crate::json_util::raw_string_field;
use serde_json::{json, Value};

pub(crate) fn set_auth_state(
    custom: Option<&Value>,
    status: &str,
    message: &str,
    error: Value,
    last_refresh_at: Option<&str>,
    expires_at: Option<&str>,
) -> Value {
    let mut next = normalize_custom(custom);
    next["auth_status"] = Value::String(status.to_string());
    next["auth_status_message"] = Value::String(message.to_string());
    next["auth_error"] = if status == "error" {
        error
    } else {
        Value::Null
    };
    if let Some(value) = last_refresh_at {
        next["auth_last_refresh_at"] = Value::String(value.to_string());
    }
    if let Some(value) = expires_at {
        next["auth_expires_at"] = Value::String(value.to_string());
    }
    normalize_custom(Some(&next))
}

pub(crate) fn set_usage_state(
    custom: Option<&Value>,
    status: &str,
    message: &str,
    usage_info: Option<Value>,
    error: Value,
) -> Value {
    let mut next = normalize_custom(custom);
    if let Some(value) = usage_info {
        next["usage_info"] = value;
    }
    next["usage_status"] = Value::String(status.to_string());
    next["usage_status_message"] = Value::String(message.to_string());
    next["usage_error"] = if status == "error" {
        error
    } else {
        Value::Null
    };
    normalize_custom(Some(&next))
}

pub(crate) fn normalize_custom(value: Option<&Value>) -> Value {
    let raw = value.unwrap_or(&Value::Null);
    let auth_error = normalize_error_state(raw.get("auth_error"));
    let usage_error = normalize_error_state(raw.get("usage_error"));
    let mut auth_status = raw_string_field(raw, "auth_status");
    let mut usage_status = raw_string_field(raw, "usage_status");

    if auth_status.is_empty() {
        auth_status = if auth_error.is_null() {
            "active".to_string()
        } else {
            "error".to_string()
        };
    }
    if !matches!(auth_status.as_str(), "active" | "refreshing" | "error") {
        auth_status = "active".to_string();
    }

    let usage_info = normalize_usage_info(raw.get("usage_info"));
    if usage_status.is_empty() {
        usage_status = if !usage_error.is_null() {
            "error".to_string()
        } else if !usage_info.is_null() {
            "ok".to_string()
        } else {
            "missing".to_string()
        };
    }
    if !matches!(
        usage_status.as_str(),
        "ok" | "syncing" | "missing" | "error"
    ) {
        usage_status = "ok".to_string();
    }

    json!({
        "created_at": raw_string_field(raw, "created_at"),
        "last_used_at": raw_string_field(raw, "last_used_at"),
        "usage_info": usage_info,
        "auth_status": auth_status,
        "auth_status_message": raw_string_field(raw, "auth_status_message"),
        "auth_error": if auth_status == "error" { auth_error } else { Value::Null },
        "auth_last_refresh_at": raw_string_field(raw, "auth_last_refresh_at"),
        "auth_expires_at": raw_string_field(raw, "auth_expires_at"),
        "usage_status": usage_status,
        "usage_status_message": raw_string_field(raw, "usage_status_message"),
        "usage_error": if usage_status == "error" { usage_error } else { Value::Null }
    })
}
