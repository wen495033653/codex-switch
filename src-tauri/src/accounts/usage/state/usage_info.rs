use super::number::to_number;
use crate::{
    json_util::{non_empty_string_field, string_field},
    time_util::now_string,
};
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

fn to_count(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|item| item as i64)),
        Some(Value::String(text)) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
    .filter(|count| *count >= 0)
}

/// `/wham/usage` reports Codex usage-reset credits as `rate_limit_reset_credits`;
/// stored usage_info keeps them as `reset_credits`. Both shapes normalize here.
fn normalize_reset_credits(raw: &Value) -> Value {
    let source = raw
        .get("rate_limit_reset_credits")
        .or_else(|| raw.get("reset_credits"))
        .filter(|value| value.is_object());
    let Some(source) = source else {
        return Value::Null;
    };
    let Some(available_count) = to_count(source.get("available_count")) else {
        return Value::Null;
    };
    json!({
        "available_count": available_count,
        "applicable_available_count": to_count(source.get("applicable_available_count"))
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
        "plan_type": string_field(raw, "plan_type"),
        "reset_credits": normalize_reset_credits(raw),
        "fetched_at": non_empty_string_field(raw, "fetched_at").unwrap_or_else(now_string)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wham_usage_response() -> Value {
        json!({
            "plan_type": "pro",
            "rate_limit": {
                "allowed": true,
                "limit_reached": false,
                "primary_window": {
                    "used_percent": 12,
                    "limit_window_seconds": 604800,
                    "reset_after_seconds": 1000,
                    "reset_at": 1789963286
                },
                "secondary_window": null
            },
            "rate_limit_reset_credits": {
                "available_count": 3,
                "applicable_available_count": 0
            },
            "fetched_at": "2026-09-14T03:50:05Z"
        })
    }

    #[test]
    fn wham_usage_keeps_plan_type_and_reset_credits() {
        let usage = normalize_usage_info(Some(&wham_usage_response()));

        assert_eq!(usage["plan_type"], json!("pro"));
        assert_eq!(
            usage["reset_credits"],
            json!({ "available_count": 3, "applicable_available_count": 0 })
        );
        assert_eq!(
            usage["rate_limit"]["primary_window"]["used_percent"],
            json!(12.0)
        );
        assert_eq!(usage["fetched_at"], json!("2026-09-14T03:50:05Z"));
    }

    #[test]
    fn stored_usage_info_round_trips_reset_credits() {
        let stored = normalize_usage_info(Some(&wham_usage_response()));

        let again = normalize_usage_info(Some(&stored));

        assert_eq!(again, stored);
    }

    #[test]
    fn missing_plan_type_and_reset_credits_stay_empty() {
        let usage = normalize_usage_info(Some(&json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 1,
                    "limit_window_seconds": 18000,
                    "reset_at": 1789373984
                }
            }
        })));

        assert_eq!(usage["plan_type"], json!(""));
        assert_eq!(usage["reset_credits"], Value::Null);
    }

    #[test]
    fn reset_credits_without_available_count_are_not_reported() {
        let mut raw = wham_usage_response();
        raw["rate_limit_reset_credits"] = json!({ "applicable_available_count": 1 });

        let usage = normalize_usage_info(Some(&raw));

        assert_eq!(usage["reset_credits"], Value::Null);
    }
}
