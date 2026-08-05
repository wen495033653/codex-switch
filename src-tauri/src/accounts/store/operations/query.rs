use crate::accounts::{account_id_from_account, profile_id_from_account};
use serde_json::Value;

use super::super::persistence::read_store_value;

fn lookup_store_account_in_value(store: &Value, profile_id: &str) -> Result<Option<Value>, String> {
    let accounts = store
        .get("accounts")
        .and_then(Value::as_array)
        .ok_or_else(|| "accounts.json 数据结构无效".to_string())?;

    if let Some(account) = accounts
        .iter()
        .find(|account| profile_id_from_account(account).unwrap_or_default() == profile_id)
    {
        return Ok(Some(account.clone()));
    }

    let legacy_matches = accounts
        .iter()
        .filter(|account| account_id_from_account(account).unwrap_or_default() == profile_id)
        .collect::<Vec<_>>();
    if legacy_matches.len() == 1 {
        return Ok(Some(legacy_matches[0].clone()));
    }

    Ok(None)
}

pub(crate) fn lookup_store_account(profile_id: &str) -> Result<Option<Value>, String> {
    let store = read_store_value()?;
    lookup_store_account_in_value(&store, profile_id)
}

pub(crate) fn find_store_account(profile_id: &str) -> Result<Value, String> {
    lookup_store_account(profile_id)?.ok_or_else(|| "账号不存在".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn account(profile_id: &str, account_id: &str) -> Value {
        json!({
            "profile_id": profile_id,
            "tokens": {
                "id_token": "id-token",
                "access_token": "access-token",
                "refresh_token": "refresh-token",
                "account_id": account_id
            }
        })
    }

    #[test]
    fn lookup_store_account_reports_missing_without_using_filesystem() {
        let store = json!({
            "accounts": [account("profile-present", "account-present")]
        });

        assert_eq!(
            lookup_store_account_in_value(&store, "profile-missing").unwrap(),
            None
        );
    }

    #[test]
    fn lookup_store_account_keeps_invalid_store_distinct_from_missing() {
        let err = lookup_store_account_in_value(&json!({ "accounts": {} }), "profile-missing")
            .unwrap_err();

        assert_eq!(err, "accounts.json 数据结构无效");
    }
}
