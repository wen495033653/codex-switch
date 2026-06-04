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

pub(crate) fn update_active_account_usage_result(
    profile_id: &str,
    usage_result: Result<Value, Value>,
) -> Result<Option<Value>, String> {
    let store = read_store_value()?;
    if raw_string_field(&store, "active_id") != profile_id {
        return Ok(None);
    }
    update_account_usage_result_in_store(store, profile_id, usage_result).map(Some)
}
