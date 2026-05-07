use super::super::auth_refresh::refresh_stored_account_tokens;
use crate::{
    accounts::{build_error_state, find_store_account, get_usage, mark_account_auth_error},
    events::emit_store_updated,
    json_util::value_u64_field,
};
use serde_json::Value;
use tauri::AppHandle;

fn is_auth_retryable_usage_error(error: &Value) -> bool {
    matches!(value_u64_field(error, "status"), Some(401 | 403))
}

pub(crate) fn get_usage_with_auth_retry(
    app: &AppHandle,
    account_id: &str,
    access_token: &str,
    timeout_ms: u64,
) -> Result<Value, Value> {
    match get_usage(access_token, account_id, timeout_ms) {
        Ok(usage_info) => Ok(usage_info),
        Err(error) if is_auth_retryable_usage_error(&error) => {
            match refresh_stored_account_tokens(account_id) {
                Ok(store) => {
                    emit_store_updated(app, store);
                    let refreshed = find_store_account(account_id)
                        .map_err(|err| build_error_state(&err, "auth_refresh_failed", "", 0, ""))?;
                    let refreshed_access_token = refreshed
                        .get("tokens")
                        .and_then(|tokens| tokens.get("access_token"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    get_usage(refreshed_access_token, account_id, timeout_ms)
                }
                Err(refresh_err) => {
                    if let Ok(store) = mark_account_auth_error(account_id, &refresh_err) {
                        emit_store_updated(app, store);
                    }
                    Err(error)
                }
            }
        }
        Err(error) => Err(error),
    }
}
