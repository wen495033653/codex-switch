use super::number::to_number;
use crate::{json_util::raw_string_field, time_util::now_string};
use serde_json::{json, Value};

fn normalize_usage_window(value: Option<&Value>) -> Value {
    let raw = value.unwrap_or(&Value::Null);
    let used_percent = to_number(raw.get("used_percent"));
    let limit_window_seconds = {
        let seconds = to_number(raw.get("limit_window_seconds"));
        if seconds > 0.0 {
            seconds
        } else {
            to_number(raw.get("window_minutes")) * 60.0
        }
    };
    let reset_at = {
        let value = to_number(raw.get("reset_at"));
        if value > 0.0 {
            value
        } else {
            to_number(raw.get("resets_at"))
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

pub(crate) fn normalize_usage_info(value: Option<&Value>) -> Value {
    let raw = value.unwrap_or(&Value::Null);
    let rate_limit = raw.get("rate_limit").unwrap_or(&Value::Null);
    if !rate_limit.is_object() {
        return Value::Null;
    }
    json!({
        "rate_limit": {
            "primary_window": normalize_usage_window(rate_limit.get("primary_window")),
            "secondary_window": normalize_usage_window(rate_limit.get("secondary_window"))
        },
        "fetched_at": raw_string_field(raw, "fetched_at")
            .chars()
            .next()
            .map(|_| raw_string_field(raw, "fetched_at"))
            .unwrap_or_else(now_string)
    })
}
