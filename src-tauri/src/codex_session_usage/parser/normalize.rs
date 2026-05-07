use super::{
    event::{FlexibleNumber, SessionEvent, SessionRateLimitWindow, SessionRateLimits},
    usage_info_fetched_at_seconds,
};
use crate::time_util::parse_rfc3339_seconds;
use serde_json::{json, Value};

fn number_value(value: Option<&FlexibleNumber>) -> f64 {
    value.and_then(FlexibleNumber::as_f64).unwrap_or(0.0)
}

fn normalize_window(window: Option<&SessionRateLimitWindow>) -> Value {
    let Some(window) = window else {
        return Value::Null;
    };
    let used_percent = number_value(window.used_percent.as_ref());
    let limit_window_seconds = {
        let seconds = number_value(window.limit_window_seconds.as_ref());
        if seconds > 0.0 {
            seconds
        } else {
            number_value(window.window_minutes.as_ref()) * 60.0
        }
    };
    let reset_at = {
        let value = number_value(window.reset_at.as_ref());
        if value > 0.0 {
            value
        } else {
            number_value(window.resets_at.as_ref())
        }
    };
    if limit_window_seconds <= 0.0 || reset_at <= 0.0 {
        return Value::Null;
    }
    json!({
        "used_percent": used_percent,
        "limit_window_seconds": limit_window_seconds,
        "reset_at": reset_at
    })
}

fn normalize_session_usage_info(rate_limits: &SessionRateLimits, fetched_at: &str) -> Value {
    if parse_rfc3339_seconds(fetched_at).is_none() {
        return Value::Null;
    }

    let primary_window = normalize_window(rate_limits.primary.as_ref());
    let secondary_window = normalize_window(rate_limits.secondary.as_ref());
    if primary_window.is_null() && secondary_window.is_null() {
        return Value::Null;
    }

    json!({
        "rate_limit": {
            "primary_window": primary_window,
            "secondary_window": secondary_window
        },
        "fetched_at": fetched_at.to_string()
    })
}

pub(super) fn usage_info_from_line(line: &str) -> Option<Value> {
    if !line.contains("\"token_count\"") || !line.contains("\"rate_limits\"") {
        return None;
    }
    let event: SessionEvent = serde_json::from_str(line).ok()?;
    if event.event_type != "event_msg" {
        return None;
    }
    let payload = event.payload?;
    if payload.payload_type != "token_count" {
        return None;
    }
    let rate_limits = payload.rate_limits?;
    let usage_info = normalize_session_usage_info(&rate_limits, &event.timestamp);
    if usage_info.is_null() {
        None
    } else {
        Some(usage_info)
    }
}

pub(crate) fn newer_usage_info(current: Option<Value>, candidate: Value) -> Option<Value> {
    let Some(current) = current else {
        return Some(candidate);
    };

    match (
        usage_info_fetched_at_seconds(&candidate),
        usage_info_fetched_at_seconds(&current),
    ) {
        (Some(candidate_at), Some(current_at)) if candidate_at >= current_at => Some(candidate),
        (Some(_), None) => Some(candidate),
        _ => Some(current),
    }
}
