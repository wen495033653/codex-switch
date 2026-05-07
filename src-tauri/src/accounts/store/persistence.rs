use super::model::{account_id_from_account, empty_store, normalize_store_data};
use crate::{
    accounts::get_codex_state_value,
    json_file::{read_json_file, write_json_file},
    json_util::raw_string_field,
    paths::accounts_path,
};
use serde_json::Value;

pub(crate) fn read_store_value() -> Result<Value, String> {
    let path = accounts_path()?;
    if !path.exists() {
        let store = empty_store();
        write_store_value(&store)?;
        return Ok(store);
    }

    let parsed = read_json_file(&path, "accounts.json")?;
    normalize_store_data(&parsed)
}

pub(crate) fn write_store_value(store: &Value) -> Result<(), String> {
    let path = accounts_path()?;
    let normalized = normalize_store_data(store)?;
    write_json_file(&path, "accounts.json", &normalized)
}

pub(crate) fn read_store_with_active_sync() -> Result<Value, String> {
    let mut store = read_store_value()?;
    let state = get_codex_state_value();
    let account_id = raw_string_field(&state, "account_id");
    if raw_string_field(&state, "mode") == "chatgpt"
        && !account_id.is_empty()
        && store
            .get("accounts")
            .and_then(Value::as_array)
            .is_some_and(|accounts| {
                accounts.iter().any(|account| {
                    account_id_from_account(account).unwrap_or_default() == account_id
                })
            })
    {
        store["active_id"] = Value::String(account_id);
        write_store_value(&store)?;
    }
    Ok(store)
}
