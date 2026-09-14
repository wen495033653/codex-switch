//! Subscription snapshot taken from `/backend-api/subscriptions`.
//!
//! The id_token claim `chatgpt_subscription_active_until` is a snapshot that OpenAI does
//! not always refresh: on 2026-09-14 three accounts kept an expired claim through repeated
//! refresh_token grants while the endpoint reported renewal dates weeks in the future.
//! The endpoint is therefore authoritative and the claim is only a fallback.

use crate::{json_util::string_field, time_util::now_string};
use serde_json::{json, Value};

fn optional_bool(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Bool(flag)) => json!(flag),
        _ => Value::Null,
    }
}

pub(crate) fn normalize_subscription(value: Option<&Value>) -> Value {
    let raw = value.unwrap_or(&Value::Null);
    if !raw.is_object() {
        return Value::Null;
    }
    let active_until = string_field(raw, "active_until");
    if active_until.is_empty() {
        return Value::Null;
    }
    let fetched_at = {
        let stored = string_field(raw, "fetched_at");
        if stored.is_empty() {
            now_string()
        } else {
            stored
        }
    };
    json!({
        "active_until": active_until,
        "plan_type": string_field(raw, "plan_type"),
        "will_renew": optional_bool(raw.get("will_renew")),
        "is_delinquent": optional_bool(raw.get("is_delinquent")),
        "fetched_at": fetched_at
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint_response() -> Value {
        json!({
            "id": "subscription-id",
            "plan_type": "pro",
            "seats_in_use": 1,
            "active_start": "2026-08-04T12:29:43Z",
            "active_until": "2026-10-10T13:30:31Z",
            "billing_period": "monthly",
            "will_renew": true,
            "is_delinquent": false,
            "grace_period_end_timestamp": null,
            "fetched_at": "2026-09-14T07:00:00Z"
        })
    }

    #[test]
    fn endpoint_response_keeps_renewal_fields() {
        let subscription = normalize_subscription(Some(&endpoint_response()));

        assert_eq!(
            subscription,
            json!({
                "active_until": "2026-10-10T13:30:31Z",
                "plan_type": "pro",
                "will_renew": true,
                "is_delinquent": false,
                "fetched_at": "2026-09-14T07:00:00Z"
            })
        );
    }

    #[test]
    fn stored_subscription_round_trips() {
        let stored = normalize_subscription(Some(&endpoint_response()));

        assert_eq!(normalize_subscription(Some(&stored)), stored);
    }

    #[test]
    fn response_without_active_until_is_not_stored() {
        let mut raw = endpoint_response();
        raw["active_until"] = Value::Null;

        assert_eq!(normalize_subscription(Some(&raw)), Value::Null);
        assert_eq!(normalize_subscription(None), Value::Null);
    }

    #[test]
    fn missing_flags_stay_unknown_instead_of_false() {
        let subscription = normalize_subscription(Some(&json!({
            "active_until": "2026-10-10T13:30:31Z"
        })));

        assert_eq!(subscription["will_renew"], Value::Null);
        assert_eq!(subscription["is_delinquent"], Value::Null);
        assert_eq!(subscription["plan_type"], json!(""));
        assert!(!string_field(&subscription, "fetched_at").is_empty());
    }
}
