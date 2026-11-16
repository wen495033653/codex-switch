use super::number::to_number;
use crate::{
    json_util::{non_empty_string_field, string_field},
    time_util::{now_string, parse_rfc3339_millis},
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
        "applicable_available_count": to_count(source.get("applicable_available_count")),
        "expires_at_ms": source.get("expires_at_ms").and_then(Value::as_array)
    })
}

pub(crate) fn apply_reset_credit_details(
    reset_credits: &mut Value,
    details: &Value,
) -> Result<(), String> {
    let available_count = details["available_count"].as_u64().ok_or_else(|| {
        format!(
            "重置券明细 available_count 无效: {}",
            details["available_count"]
        )
    })?;
    let credits = details["credits"]
        .as_array()
        .ok_or_else(|| "重置券明细 credits 不是数组".to_string())?;
    let mut expires_at_ms = Vec::new();
    for (index, credit) in credits.iter().enumerate() {
        if credit["status"] != "available" {
            continue;
        }
        let raw = &credit["expires_at"];
        let expiry =
            if raw.is_null() {
                None
            } else {
                Some(raw.as_str().and_then(parse_rfc3339_millis).ok_or_else(|| {
                    format!("重置券明细第 {} 项 expires_at 无效: {raw}", index + 1)
                })?)
            };
        expires_at_ms.push(expiry);
    }
    expires_at_ms.sort_by_key(|expiry| (expiry.is_none(), *expiry));
    reset_credits["available_count"] = json!(available_count);
    reset_credits["expires_at_ms"] = json!(expires_at_ms);
    Ok(())
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
            json!({ "available_count": 3, "applicable_available_count": 0, "expires_at_ms": null })
        );
        assert_eq!(
            usage["rate_limit"]["primary_window"]["used_percent"],
            json!(12.0)
        );
        assert_eq!(usage["fetched_at"], json!("2026-09-14T03:50:05Z"));
    }

    #[test]
    fn reset_credit_details_keep_each_available_expiry_through_storage() {
        let mut stored = normalize_usage_info(Some(&wham_usage_response()));
        apply_reset_credit_details(
            &mut stored["reset_credits"],
            &json!({
                "available_count": 4,
                "credits": [
                    {"status": "available", "expires_at": "2026-12-01T22:04:48.783637Z"},
                    {"status": "expired", "expires_at": "2026-11-17T20:28:16Z"},
                    {"status": "available", "expires_at": null},
                    {"status": "available", "expires_at": "2026-11-30T20:28:16.668636Z"},
                    {"status": "redeemed", "expires_at": "2026-12-15T00:00:00Z"},
                    {"status": "available", "expires_at": "2026-11-30T20:28:16.668636Z"}
                ]
            }),
        )
        .unwrap();
        assert_eq!(
            stored["reset_credits"],
            json!({
                "available_count": 4,
                "applicable_available_count": 0,
                "expires_at_ms": [1796070496668_i64, 1796070496668_i64, 1796162688783_i64, null]
            })
        );

        let again = normalize_usage_info(Some(&stored));

        assert_eq!(again, stored);
    }

    #[test]
    fn malformed_reset_credit_details_leave_expiries_unsynced() {
        let mut stored = normalize_usage_info(Some(&wham_usage_response()));
        let reset_credits = &mut stored["reset_credits"];
        let summary = reset_credits.clone();
        let error = apply_reset_credit_details(
            reset_credits,
            &json!({
                "available_count": 1,
                "credits": [{"status": "available", "expires_at": "not-a-date"}]
            }),
        )
        .unwrap_err();
        assert!(error.contains("expires_at") && error.contains("not-a-date"));
        assert_eq!(*reset_credits, summary);
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
