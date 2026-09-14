use crate::{
    accounts::{
        profile_id_from_account, read_store_value, set_usage_result, sort_accounts_by_last_used,
        write_store_value,
    },
    codex_session_usage::inherit_stored_usage_fields,
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

    // A changed limit window means the account was actually used, so it moves to the front.
    let touches_last_used = matches!(
        &usage_result,
        Ok(usage_info)
            if !old_usage.is_null()
                && old_usage != *usage_info
                && is_limit_window_changed(&old_usage, usage_info)
    );
    let mut next_custom = set_usage_result(accounts[index].get("custom"), usage_result);
    if touches_last_used {
        next_custom["last_used_at"] = Value::String(now_string());
    }

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
        usage_result.map(|usage_info| inherit_stored_usage_fields(&previous_usage, usage_info));
    update_account_usage_result_in_store(store, profile_id, usage_result).map(Some)
}
