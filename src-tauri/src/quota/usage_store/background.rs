use super::super::subscription::refresh_account_subscription;
use super::retry::get_usage_with_auth_retry;
use crate::{
    accounts::{
        account_with_custom, add_account_to_store, find_store_account, set_usage_result,
        INTERACTIVE_REQUEST_TIMEOUT_MS,
    },
    events::emit_store_updated,
};
use std::thread;
use tauri::AppHandle;

pub(crate) fn sync_account_usage_in_background(
    app: AppHandle,
    profile_id: String,
    account_id: String,
    access_token: String,
) {
    thread::spawn(move || {
        let usage_result = get_usage_with_auth_retry(
            &app,
            &profile_id,
            &account_id,
            &access_token,
            INTERACTIVE_REQUEST_TIMEOUT_MS,
        );
        let Ok(account) = find_store_account(&profile_id) else {
            return;
        };
        let usage_ok = usage_result.is_ok();
        let custom = set_usage_result(account.get("custom"), usage_result);
        if let Ok(store) = add_account_to_store(account_with_custom(&account, custom), false) {
            emit_store_updated(&app, store);
        }
        if !usage_ok {
            return;
        }
        if let Some(store) =
            refresh_account_subscription(&profile_id, INTERACTIVE_REQUEST_TIMEOUT_MS)
        {
            emit_store_updated(&app, store);
        }
    });
}
