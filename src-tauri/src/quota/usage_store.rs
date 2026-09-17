use super::{
    auth_refresh::refresh_stored_account_tokens, subscription::refresh_account_subscription,
};
use crate::{
    accounts::{
        access_token_from_account, account_with_custom, add_account_to_store, build_error_state,
        error_state_is_auth_rejected, find_store_account, get_usage, mark_account_auth_error,
        profile_id_from_account, read_store_value, set_usage_result, sort_accounts_by_last_used,
        write_store_value, INTERACTIVE_REQUEST_TIMEOUT_MS,
    },
    codex_session_usage::inherit_stored_usage_fields,
    events::emit_store_updated,
    json_util::raw_string_field,
    time_util::now_string,
};
use serde_json::Value;
use std::thread;
use tauri::AppHandle;

pub(crate) fn sync_account_usage_in_background(
    app: AppHandle,
    profile_id: String,
    account_id: String,
    access_token: String,
) {
    thread::spawn(move || {
        let usage_result = get_usage_with_auth_retry(
            &app,
            &profile_id,
            &account_id,
            &access_token,
            INTERACTIVE_REQUEST_TIMEOUT_MS,
        );
        let Ok(account) = find_store_account(&profile_id) else {
            return;
        };
        let usage_ok = usage_result.is_ok();
        let custom = set_usage_result(account.get("custom"), usage_result);
        if let Ok(store) = add_account_to_store(account_with_custom(&account, custom), false) {
            emit_store_updated(&app, store);
        }
        if !usage_ok {
            return;
        }
        if let Some(store) =
            refresh_account_subscription(&profile_id, INTERACTIVE_REQUEST_TIMEOUT_MS)
        {
            emit_store_updated(&app, store);
        }
    });
}

pub(super) fn get_usage_with_auth_retry(
    app: &AppHandle,
    profile_id: &str,
    account_id: &str,
    access_token: &str,
    timeout_ms: u64,
) -> Result<Value, Value> {
    match get_usage(access_token, account_id, timeout_ms) {
        Ok(usage_info) => Ok(usage_info),
        Err(error) if error_state_is_auth_rejected(&error) => {
            match refresh_stored_account_tokens(profile_id) {
                Ok(store) => {
                    emit_store_updated(app, store);
                    let refreshed = find_store_account(profile_id)
                        .map_err(|err| build_error_state(&err, "auth_refresh_failed", "", 0, ""))?;
                    get_usage(
                        &access_token_from_account(&refreshed),
                        account_id,
                        timeout_ms,
                    )
                }
                Err(refresh_err) => {
                    if let Ok(store) = mark_account_auth_error(profile_id, &refresh_err) {
                        emit_store_updated(app, store);
                    }
                    Err(error)
                }
            }
        }
        Err(error) => Err(error),
    }
}

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

pub(super) fn update_account_usage_result(
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

pub(super) fn update_active_account_usage_result(
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
