use super::super::{
    model::{normalize_account, profile_id_from_account, sort_accounts_by_last_used},
    persistence::mutate_store,
};
use super::query::store_account_index;
use crate::{json_util::raw_string_field, time_util::now_string};
use serde_json::Value;

fn store_accounts_mut(store: &mut Value) -> Result<&mut Vec<Value>, String> {
    store
        .get_mut("accounts")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "accounts.json 数据结构无效".to_string())
}

fn keep_previous_timestamps(account: &mut Value, previous: &Value) {
    let Some(previous_custom) = previous.get("custom") else {
        return;
    };
    for field in ["created_at", "last_used_at"] {
        if let Some(value) = previous_custom.get(field).and_then(Value::as_str) {
            if !value.is_empty() {
                account["custom"][field] = Value::String(value.to_string());
            }
        }
    }
}

/// Adds a new account or replaces the stored one with the same profile id. Use this only
/// for accounts built from fresh credentials (OAuth, refresh_token import, auth.json
/// capture); an update derived from a stored account goes through `update_store_account`.
pub(crate) fn add_account_to_store(account: Value, mark_active: bool) -> Result<Value, String> {
    let mut account = normalize_account(&account)?;
    let profile_id = profile_id_from_account(&account)?;
    mutate_store(|store| {
        let accounts = store_accounts_mut(store)?;
        if let Some(index) = accounts.iter().position(|existing| {
            profile_id_from_account(existing).unwrap_or_default() == profile_id
        }) {
            keep_previous_timestamps(&mut account, &accounts[index]);
            accounts[index] = account;
        } else {
            accounts.push(account);
        }
        sort_accounts_by_last_used(accounts);
        if mark_active {
            store["active_id"] = Value::String(profile_id);
        }
        Ok(())
    })
    .map(|(store, ())| store)
}

fn replace_store_account(
    store: &mut Value,
    profile_id: &str,
    update: impl FnOnce(&Value) -> Result<Value, String>,
) -> Result<(), String> {
    let accounts = store_accounts_mut(store)?;
    let index =
        store_account_index(accounts, profile_id).ok_or_else(|| "账号不存在".to_string())?;
    let stored_profile_id = profile_id_from_account(&accounts[index])?;
    let mut next = update(&accounts[index])?;
    // The builders produce `{tokens, custom}`; the stored identity must survive the update.
    next["profile_id"] = Value::String(stored_profile_id);
    accounts[index] = normalize_account(&next)?;
    sort_accounts_by_last_used(accounts);
    Ok(())
}

/// Rebuilds one stored account from its state on disk at the moment of the write. `update`
/// receives the current account, never a copy read before a network request, so tokens
/// rotated or fields written in the meantime are kept. A missing account is an error and
/// is never re-created.
pub(crate) fn update_store_account(
    profile_id: &str,
    update: impl FnOnce(&Value) -> Result<Value, String>,
) -> Result<Value, String> {
    mutate_store(|store| replace_store_account(store, profile_id, update)).map(|(store, ())| store)
}

/// Same as `update_store_account`, but only while `profile_id` is still the active account.
/// Returns `None` without writing when the active account changed.
pub(crate) fn update_active_store_account(
    profile_id: &str,
    update: impl FnOnce(&Value) -> Result<Value, String>,
) -> Result<Option<Value>, String> {
    mutate_store(|store| {
        if raw_string_field(store, "active_id") != profile_id {
            return Ok(false);
        }
        replace_store_account(store, profile_id, update)?;
        Ok(true)
    })
    .map(|(store, updated)| updated.then_some(store))
}

pub(crate) fn mark_store_account_used(profile_id: &str) -> Result<Value, String> {
    mutate_store(|store| {
        let accounts = store_accounts_mut(store)?;
        let account = accounts
            .iter_mut()
            .find(|account| profile_id_from_account(account).unwrap_or_default() == profile_id)
            .ok_or_else(|| "账号不存在".to_string())?;
        account["custom"]["last_used_at"] = Value::String(now_string());
        sort_accounts_by_last_used(accounts);
        store["active_id"] = Value::String(profile_id.to_string());
        Ok(())
    })
    .map(|(store, ())| store)
}

pub(crate) fn remove_store_account(profile_id: &str) -> Result<Value, String> {
    mutate_store(|store| {
        let accounts = store_accounts_mut(store)?;
        let before = accounts.len();
        accounts
            .retain(|account| profile_id_from_account(account).unwrap_or_default() != profile_id);
        if accounts.len() == before {
            return Err("账号不存在".to_string());
        }
        if raw_string_field(store, "active_id") == profile_id {
            store["active_id"] = Value::String(String::new());
        }
        Ok(())
    })
    .map(|(store, ())| store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::account_with_custom;
    use serde_json::json;

    fn stored_account(account_id: &str, refresh_token: &str) -> Value {
        json!({
            "profile_id": account_id,
            "tokens": {
                "id_token": "id",
                "access_token": format!("access-{refresh_token}"),
                "refresh_token": refresh_token,
                "account_id": account_id
            },
            "custom": {
                "created_at": "2026-01-01T00:00:00Z",
                "last_used_at": "2026-01-01T00:00:00Z"
            }
        })
    }

    fn store_with(accounts: Vec<Value>) -> Value {
        json!({ "version": 3, "active_id": "", "accounts": accounts })
    }

    #[test]
    fn update_builds_on_current_tokens_not_an_earlier_copy() {
        // An earlier read saw "refresh-old"; another writer has since rotated it.
        let earlier_copy = stored_account("acct-1", "refresh-old");
        let mut store = store_with(vec![stored_account("acct-1", "refresh-new")]);

        replace_store_account(&mut store, "acct-1", |current| {
            let mut custom = earlier_copy["custom"].clone();
            custom["auth_status"] = json!("refreshing");
            Ok(account_with_custom(current, custom))
        })
        .unwrap();

        let account = &store["accounts"][0];
        assert_eq!(account["tokens"]["refresh_token"], "refresh-new");
        assert_eq!(account["custom"]["auth_status"], "refreshing");
        assert_eq!(account["profile_id"], "acct-1");
    }

    #[test]
    fn update_of_deleted_account_fails_without_recreating_it() {
        let mut store = store_with(vec![stored_account("acct-other", "refresh")]);

        let err = replace_store_account(&mut store, "acct-deleted", |current| Ok(current.clone()))
            .unwrap_err();

        assert_eq!(err, "账号不存在");
        assert_eq!(store["accounts"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn update_accepts_legacy_account_id_and_keeps_stored_profile_id() {
        let mut stored = stored_account("acct-1", "refresh");
        stored["profile_id"] = json!("acct-1:user@example.com");
        let mut store = store_with(vec![stored]);

        replace_store_account(&mut store, "acct-1", |current| Ok(current.clone())).unwrap();

        assert_eq!(
            store["accounts"][0]["profile_id"],
            "acct-1:user@example.com"
        );
    }
}
