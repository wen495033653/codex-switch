use crate::accounts::account_id_from_account;
use serde_json::Value;

use super::super::persistence::read_store_value;

pub(crate) fn find_store_account(account_id: &str) -> Result<Value, String> {
    let store = read_store_value()?;
    store
        .get("accounts")
        .and_then(Value::as_array)
        .and_then(|accounts| {
            accounts
                .iter()
                .find(|account| account_id_from_account(account).unwrap_or_default() == account_id)
        })
        .cloned()
        .ok_or_else(|| "账号不存在".to_string())
}
