use crate::{
    accounts::{
        profile_id_from_account, read_store_value, set_usage_state, sort_accounts_by_last_used,
        write_store_value,
    },
    json_util::raw_string_field,
    time_util::now_string,
};
use serde_json::Value;

fn usage_window_used_percent(usage: &Value, key: &str) -> Option<f64> {
    usage
        .get("rate_limit")
        .and_then(|rate_limit| rate_limit.get(key))
        .and_then(|window| window.get("used_percent"))
        .and_then(|value| match value {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.parse::<f64>().ok(),
            _ => None,
        })
}

fn is_limit_window_changed(old_usage: &Value, new_usage: &Value) -> bool {
    ["primary_window", "secondary_window"].iter().any(|key| {
        usage_window_used_percent(old_usage, key) != usage_window_used_percent(new_usage, key)
    })
}

fn update_account_usage_result_in_store(
    mut store: Value,
    profile_id: &str,
    usage_result: Result<Value, Value>,
) -> Result<Value, String> {
    let accounts = store
        .get_mut("accounts")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "accounts.json 数据结构无效".to_string())?;
    let index = accounts
        .iter()
        .position(|account| profile_id_from_account(account).unwrap_or_default() == profile_id)
        .ok_or_else(|| "账号不存在".to_string())?;

    let old_usage = accounts[index]
        .get("custom")
        .and_then(|custom| custom.get("usage_info"))
        .cloned()
        .unwrap_or(Value::Null);

    let next_custom = match usage_result {
        Ok(usage_info) => {
            let should_touch = !old_usage.is_null()
                && old_usage != usage_info
                && is_limit_window_changed(&old_usage, &usage_info);
            let mut custom = set_usage_state(
                accounts[index].get("custom"),
                "ok",
                "",
                Some(usage_info),
                Value::Null,
            );
            if should_touch {
                custom["last_used_at"] = Value::String(now_string());
            }
            custom
        }
        Err(error) => {
            let message = raw_string_field(&error, "message")
                .chars()
                .next()
                .map(|_| raw_string_field(&error, "message"))
                .unwrap_or_else(|| "Usage refresh failed, please refresh manually".to_string());
            set_usage_state(
                accounts[index].get("custom"),
                "error",
                &message,
                None,
                error,
            )
        }
    };

    accounts[index]["custom"] = next_custom;
    sort_accounts_by_last_used(accounts);
    write_store_value(&store)?;
    Ok(store)
}

pub(crate) fn update_account_usage_result(
    profile_id: &str,
    usage_result: Result<Value, Value>,
) -> Result<Value, String> {
    update_account_usage_result_in_store(read_store_value()?, profile_id, usage_result)
}

fn stored_usage_info(store: &Value, profile_id: &str) -> Value {
    store
        .get("accounts")
        .and_then(Value::as_array)
        .and_then(|accounts| {
            accounts
                .iter()
                .find(|account| profile_id_from_account(account).unwrap_or_default() == profile_id)
        })
        .and_then(|account| account.get("custom"))
        .and_then(|custom| custom.get("usage_info"))
        .cloned()
        .unwrap_or(Value::Null)
}

/// Codex session token_count events never report usage-reset credits and older
/// events omit plan_type, so session-sourced usage keeps the last API-reported values.
fn inherit_session_usage_fields(previous: &Value, mut usage_info: Value) -> Value {
    if raw_string_field(&usage_info, "plan_type").is_empty() {
        let previous_plan_type = raw_string_field(previous, "plan_type");
        if !previous_plan_type.is_empty() {
            usage_info["plan_type"] = Value::String(previous_plan_type);
        }
    }
    if usage_info.get("reset_credits").is_none_or(Value::is_null) {
        if let Some(previous_credits) = previous
            .get("reset_credits")
            .filter(|value| !value.is_null())
        {
            usage_info["reset_credits"] = previous_credits.clone();
        }
    }
    usage_info
}

pub(crate) fn update_active_account_usage_result(
    profile_id: &str,
    usage_result: Result<Value, Value>,
) -> Result<Option<Value>, String> {
    let store = read_store_value()?;
    if raw_string_field(&store, "active_id") != profile_id {
        return Ok(None);
    }
    let previous_usage = stored_usage_info(&store, profile_id);
    let usage_result =
        usage_result.map(|usage_info| inherit_session_usage_fields(&previous_usage, usage_info));
    update_account_usage_result_in_store(store, profile_id, usage_result).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session_usage(plan_type: &str) -> Value {
        json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 5.0,
                    "limit_window_seconds": 604800.0,
                    "reset_at": 1789889905.0
                },
                "secondary_window": null
            },
            "plan_type": plan_type,
            "reset_credits": null,
            "fetched_at": "2026-09-14T04:00:00Z"
        })
    }

    #[test]
    fn session_usage_inherits_reset_credits_and_missing_plan_type() {
        let previous = json!({
            "plan_type": "pro",
            "reset_credits": { "available_count": 3, "applicable_available_count": 0 }
        });

        let merged = inherit_session_usage_fields(&previous, session_usage(""));

        assert_eq!(merged["plan_type"], json!("pro"));
        assert_eq!(
            merged["reset_credits"],
            json!({ "available_count": 3, "applicable_available_count": 0 })
        );
        assert_eq!(
            merged["rate_limit"]["primary_window"]["used_percent"],
            json!(5.0)
        );
    }

    #[test]
    fn session_plan_type_wins_over_stored_plan_type() {
        let previous = json!({ "plan_type": "plus", "reset_credits": null });

        let merged = inherit_session_usage_fields(&previous, session_usage("pro"));

        assert_eq!(merged["plan_type"], json!("pro"));
        assert_eq!(merged["reset_credits"], Value::Null);
    }
}
