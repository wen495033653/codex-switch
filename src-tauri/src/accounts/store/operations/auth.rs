use super::{mutation::add_account_to_store, query::find_store_account};
use crate::accounts::{
    build_error_state, get_codex_state_value, set_auth_state, write_account_auth,
};
use crate::json_util::raw_string_field;
use serde_json::{json, Value};

use super::super::persistence::read_store_value;

pub(crate) fn sync_auth_file_if_active(profile_id: &str) -> Result<(), String> {
    let store = read_store_value()?;
    if raw_string_field(&store, "active_id") != profile_id {
        return Ok(());
    }
    let state = get_codex_state_value();
    if raw_string_field(&state, "mode") != "chatgpt"
        || raw_string_field(&state, "profile_id") != profile_id
    {
        return Ok(());
    }
    let account = find_store_account(profile_id)?;
    write_account_auth(&account)
}

pub(crate) fn mark_account_auth_error(profile_id: &str, message: &str) -> Result<Value, String> {
    let account = find_store_account(profile_id)?;
    let tokens = account.get("tokens").cloned().unwrap_or(Value::Null);
    let custom = set_auth_state(
        account.get("custom"),
        "error",
        message,
        build_error_state(message, "auth_refresh_failed", "", 0, ""),
        None,
        None,
    );
    add_account_to_store(
        json!({
            "tokens": tokens,
            "custom": custom
        }),
        false,
    )
}
