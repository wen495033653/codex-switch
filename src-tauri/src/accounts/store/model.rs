mod account;
mod store;
mod tokens;

use crate::{accounts::STORE_VERSION, json_util::raw_string_field};
use serde_json::{json, Value};

use account::StoreAccount;
use store::AccountStore;
use tokens::AccountTokens;

const TOKENS_FIELD: &str = "tokens";
const CUSTOM_FIELD: &str = "custom";
const ACCOUNT_ID_FIELD: &str = "account_id";
const ID_TOKEN_FIELD: &str = "id_token";
const ACCESS_TOKEN_FIELD: &str = "access_token";
pub(super) const REFRESH_TOKEN_FIELD: &str = "refresh_token";
const CREATED_AT_FIELD: &str = "created_at";
pub(super) const LAST_USED_AT_FIELD: &str = "last_used_at";

pub(crate) fn empty_store() -> Value {
    AccountStore::empty().to_value()
}

pub(crate) fn normalize_tokens(value: Option<&Value>) -> Result<Value, String> {
    AccountTokens::from_value(value).map(|tokens| tokens.to_value())
}

pub(crate) fn account_id_from_account(account: &Value) -> Result<String, String> {
    AccountTokens::from_account_value(account).map(|tokens| tokens.account_id().to_string())
}

pub(crate) fn normalize_account(value: &Value) -> Result<Value, String> {
    StoreAccount::normalize(value).map(|account| account.to_value())
}

pub(crate) fn normalize_store_data(data: &Value) -> Result<Value, String> {
    AccountStore::normalize(data).map(|store| store.to_value())
}

pub(crate) fn sort_accounts_by_last_used(accounts: &mut [Value]) {
    accounts.sort_by(|a, b| {
        let a_time = a
            .get("custom")
            .and_then(|custom| custom.get(LAST_USED_AT_FIELD))
            .and_then(Value::as_str)
            .unwrap_or("");
        let b_time = b
            .get("custom")
            .and_then(|custom| custom.get(LAST_USED_AT_FIELD))
            .and_then(Value::as_str)
            .unwrap_or("");
        b_time.cmp(a_time)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(account_id: &str, last_used_at: &str, refresh_token: &str) -> Value {
        json!({
            "tokens": {
                "id_token": format!("id-{account_id}"),
                "access_token": format!("access-{account_id}"),
                "refresh_token": refresh_token,
                "account_id": account_id
            },
            "custom": {
                "created_at": "2026-05-05T00:00:00Z",
                "last_used_at": last_used_at
            }
        })
    }

    fn account_ids(store: &Value) -> Vec<String> {
        store
            .get("accounts")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(account_id_from_account)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn normalize_store_deduplicates_by_account_id_and_keeps_last_item() {
        let store = normalize_store_data(&json!({
            "version": STORE_VERSION,
            "active_id": "acct-1",
            "accounts": [
                account("acct-1", "2026-05-05T00:00:00Z", "old-token"),
                account("acct-2", "2026-05-05T02:00:00Z", "token-2"),
                account("acct-1", "2026-05-05T03:00:00Z", "new-token")
            ]
        }))
        .unwrap();

        assert_eq!(account_ids(&store), vec!["acct-1", "acct-2"]);
        let first_refresh_token = store
            .get("accounts")
            .and_then(Value::as_array)
            .and_then(|accounts| accounts.first())
            .and_then(|account| account.get("tokens"))
            .map(|tokens| raw_string_field(tokens, REFRESH_TOKEN_FIELD))
            .unwrap();
        assert_eq!(first_refresh_token, "new-token");
    }

    #[test]
    fn normalize_store_sorts_accounts_by_last_used_descending() {
        let store = normalize_store_data(&json!({
            "version": STORE_VERSION,
            "active_id": "",
            "accounts": [
                account("older", "2026-05-05T00:00:00Z", "token-older"),
                account("newer", "2026-05-05T01:00:00Z", "token-newer")
            ]
        }))
        .unwrap();

        assert_eq!(account_ids(&store), vec!["newer", "older"]);
    }

    #[test]
    fn normalize_tokens_requires_refresh_token() {
        let result = normalize_tokens(Some(&json!({
            "id_token": "id",
            "access_token": "access",
            "account_id": "acct"
        })));

        assert_eq!(result.unwrap_err(), "账号缺少 tokens.refresh_token");
    }
}
