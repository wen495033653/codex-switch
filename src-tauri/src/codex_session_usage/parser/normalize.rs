use super::{
    event::{FlexibleNumber, SessionEvent, SessionRateLimitWindow, SessionRateLimits},
    usage_info_fetched_at_seconds,
};
use crate::time_util::parse_rfc3339_seconds;
use serde_json::{json, Value};

const RESET_AT_SAME_WINDOW_TOLERANCE_RATIO: f64 = 0.02;
const RESET_AT_SAME_WINDOW_TOLERANCE_MAX_SECONDS: f64 = 5.0 * 60.0;

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

fn number_field(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

fn same_window_reset_tolerance(limit_window_seconds: f64) -> f64 {
    (limit_window_seconds * RESET_AT_SAME_WINDOW_TOLERANCE_RATIO)
        .min(RESET_AT_SAME_WINDOW_TOLERANCE_MAX_SECONDS)
}

fn is_same_rate_limit_window(left: &Value, right: &Value) -> bool {
    let Some(left_limit_window_seconds) = number_field(left, "limit_window_seconds") else {
        return false;
    };
    let Some(right_limit_window_seconds) = number_field(right, "limit_window_seconds") else {
        return false;
    };
    if (left_limit_window_seconds - right_limit_window_seconds).abs() > 1.0 {
        return false;
    }

    let Some(left_reset_at) = number_field(left, "reset_at") else {
        return false;
    };
    let Some(right_reset_at) = number_field(right, "reset_at") else {
        return false;
    };
    let tolerance = same_window_reset_tolerance(left_limit_window_seconds);
    (left_reset_at - right_reset_at).abs() <= tolerance
}

fn usage_window<'a>(usage_info: &'a Value, key: &str) -> Option<&'a Value> {
    usage_info
        .get("rate_limit")
        .and_then(|rate_limit| rate_limit.get(key))
}

fn usage_window_mut<'a>(usage_info: &'a mut Value, key: &str) -> Option<&'a mut Value> {
    usage_info
        .get_mut("rate_limit")
        .and_then(|rate_limit| rate_limit.get_mut(key))
}

fn merge_window_used_percent(base: &mut Value, other: &Value, key: &str) {
    let should_update = {
        let Some(base_window) = usage_window(base, key) else {
            return;
        };
        let Some(other_window) = usage_window(other, key) else {
            return;
        };
        if !is_same_rate_limit_window(base_window, other_window) {
            return;
        }
        let base_used_percent = number_field(base_window, "used_percent").unwrap_or(0.0);
        let other_used_percent = number_field(other_window, "used_percent").unwrap_or(0.0);
        other_used_percent > base_used_percent
    };

    if should_update {
        let Some(base_window) = usage_window_mut(base, key) else {
            return;
        };
        let Some(base_window) = base_window.as_object_mut() else {
            return;
        };
        let Some(other_used_percent) =
            usage_window(other, key).and_then(|window| number_field(window, "used_percent"))
        else {
            return;
        };
        base_window.insert("used_percent".to_string(), json!(other_used_percent));
    }
}

fn merge_same_window_used_percent(mut base: Value, other: &Value) -> Value {
    merge_window_used_percent(&mut base, other, "primary_window");
    merge_window_used_percent(&mut base, other, "secondary_window");
    base
}

pub(crate) fn newer_usage_info(current: Option<Value>, candidate: Value) -> Option<Value> {
    let Some(current) = current else {
        return Some(candidate);
    };

    match (
        usage_info_fetched_at_seconds(&candidate),
        usage_info_fetched_at_seconds(&current),
    ) {
        (Some(candidate_at), Some(current_at)) if candidate_at >= current_at => {
            Some(merge_same_window_used_percent(candidate, &current))
        }
        (Some(_), Some(_)) => Some(merge_same_window_used_percent(current, &candidate)),
        (Some(_), None) => Some(candidate),
        _ => Some(current),
    }
}
