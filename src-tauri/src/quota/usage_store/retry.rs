use super::super::auth_refresh::refresh_stored_account_tokens;
use crate::{
    accounts::{
        access_token_from_account, build_error_state, error_state_is_auth_rejected,
        find_store_account, get_usage, mark_account_auth_error,
    },
    events::emit_store_updated,
};
use serde_json::Value;
use tauri::AppHandle;

pub(crate) fn get_usage_with_auth_retry(
    app: &AppHandle,
    profile_id: &str,
    account_id: &str,
    access_token: &str,
    timeout_ms: u64,
) -> Result<Value, Value> {
    match get_usage(access_token, account_id, timeout_ms) {
        Ok(usage_info) => Ok(usage_info),
        Err(error) if error_state_is_auth_rejected(&error) => {
            match refresh_stored_account_tokens(profile_id) {
                Ok(store) => {
                    emit_store_updated(app, store);
                    let refreshed = find_store_account(profile_id)
                        .map_err(|err| build_error_state(&err, "auth_refresh_failed", "", 0, ""))?;
                    get_usage(
                        &access_token_from_account(&refreshed),
                        account_id,
                        timeout_ms,
                    )
                }
                Err(refresh_err) => {
                    if let Ok(store) = mark_account_auth_error(profile_id, &refresh_err) {
                        emit_store_updated(app, store);
                    }
                    Err(error)
                }
            }
        }
        Err(error) => Err(error),
    }
}
