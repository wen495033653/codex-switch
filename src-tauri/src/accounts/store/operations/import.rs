use super::super::{
    model::{account_id_from_account, normalize_store_data, sort_accounts_by_last_used},
    persistence::{read_store_value, write_store_value},
};
use crate::accounts::STORE_VERSION;
use serde_json::{json, Value};
use std::collections::HashMap;

pub(crate) fn import_store_accounts(
    incoming_accounts: Vec<Value>,
    overwrite: bool,
) -> Result<Value, String> {
    let incoming = normalize_store_data(&json!({
        "version": STORE_VERSION,
        "active_id": "",
        "accounts": incoming_accounts
    }))?;

    if overwrite {
        write_store_value(&incoming)?;
        return Ok(incoming);
    }

    let mut store = read_store_value()?;
    let mut merged = HashMap::new();

    for account in store
        .get("accounts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        if let Ok(account_id) = account_id_from_account(&account) {
            merged.insert(account_id, account);
        }
    }

    for account in incoming
        .get("accounts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        if let Ok(account_id) = account_id_from_account(&account) {
            merged.insert(account_id, account);
        }
    }

    let mut accounts: Vec<Value> = merged.into_values().collect();
    sort_accounts_by_last_used(&mut accounts);
    store["accounts"] = Value::Array(accounts);
    write_store_value(&store)?;
    Ok(store)
}
