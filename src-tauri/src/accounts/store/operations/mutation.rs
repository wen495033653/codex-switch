use super::super::{
    model::{normalize_account, profile_id_from_account, sort_accounts_by_last_used},
    persistence::{read_store_value, write_store_value},
};
use crate::{json_util::raw_string_field, time_util::now_string};
use serde_json::Value;

pub(crate) fn add_account_to_store(account: Value, mark_active: bool) -> Result<Value, String> {
    let mut account = normalize_account(&account)?;
    let profile_id = profile_id_from_account(&account)?;
    let mut store = read_store_value()?;
    let accounts = store
        .get_mut("accounts")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "accounts.json 数据结构无效".to_string())?;
    if let Some(index) = accounts
        .iter()
        .position(|existing| profile_id_from_account(existing).unwrap_or_default() == profile_id)
    {
        let previous = accounts[index].clone();
        if let Some(previous_custom) = previous.get("custom") {
            if let Some(created_at) = previous_custom.get("created_at").and_then(Value::as_str) {
                if !created_at.is_empty() {
                    account["custom"]["created_at"] = Value::String(created_at.to_string());
                }
            }
            if let Some(last_used_at) = previous_custom.get("last_used_at").and_then(Value::as_str)
            {
                if !last_used_at.is_empty() {
                    account["custom"]["last_used_at"] = Value::String(last_used_at.to_string());
                }
            }
        }
        accounts[index] = account;
    } else {
        accounts.push(account);
    }
    sort_accounts_by_last_used(accounts);
    if mark_active {
        store["active_id"] = Value::String(profile_id);
    }
    write_store_value(&store)?;
    Ok(store)
}

pub(crate) fn mark_store_account_used(profile_id: &str) -> Result<Value, String> {
    let mut store = read_store_value()?;
    let accounts = store
        .get_mut("accounts")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "accounts.json 数据结构无效".to_string())?;
    let account = accounts
        .iter_mut()
        .find(|account| profile_id_from_account(account).unwrap_or_default() == profile_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    account["custom"]["last_used_at"] = Value::String(now_string());
    sort_accounts_by_last_used(accounts);
    store["active_id"] = Value::String(profile_id.to_string());
    write_store_value(&store)?;
    Ok(store)
}

pub(crate) fn remove_store_account(profile_id: &str) -> Result<Value, String> {
    let mut store = read_store_value()?;
    let accounts = store
        .get_mut("accounts")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "accounts.json 数据结构无效".to_string())?;
    let before = accounts.len();
    accounts.retain(|account| profile_id_from_account(account).unwrap_or_default() != profile_id);
    if accounts.len() == before {
        return Err("账号不存在".to_string());
    }
    if raw_string_field(&store, "active_id") == profile_id {
        store["active_id"] = Value::String(String::new());
    }
    write_store_value(&store)?;
    Ok(store)
}
