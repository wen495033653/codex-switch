use crate::accounts::{account_id_from_account, profile_id_from_account};
use serde_json::Value;

use super::super::persistence::read_store_value;

pub(crate) fn find_store_account(profile_id: &str) -> Result<Value, String> {
    let store = read_store_value()?;
    let accounts = store
        .get("accounts")
        .and_then(Value::as_array)
        .ok_or_else(|| "accounts.json 数据结构无效".to_string())?;

    if let Some(account) = accounts
        .iter()
        .find(|account| profile_id_from_account(account).unwrap_or_default() == profile_id)
    {
        return Ok(account.clone());
    }

    let legacy_matches = accounts
        .iter()
        .filter(|account| account_id_from_account(account).unwrap_or_default() == profile_id)
        .collect::<Vec<_>>();
    if legacy_matches.len() == 1 {
        return Ok(legacy_matches[0].clone());
    }

    Err("账号不存在".to_string())
}
