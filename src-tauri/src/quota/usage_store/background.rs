use super::retry::get_usage_with_auth_retry;
use crate::{
    accounts::{add_account_to_store, find_store_account, set_usage_state},
    events::emit_store_updated,
    json_util::raw_string_field,
};
use serde_json::{json, Value};
use std::thread;
use tauri::AppHandle;

const AUTO_QUOTA_TIMEOUT_MS: u64 = 10_000;

pub(crate) fn sync_account_usage_in_background(
    app: AppHandle,
    account_id: String,
    access_token: String,
) {
    thread::spawn(move || {
        let usage_result =
            get_usage_with_auth_retry(&app, &account_id, &access_token, AUTO_QUOTA_TIMEOUT_MS);
        let Ok(account) = find_store_account(&account_id) else {
            return;
        };
        let tokens = account.get("tokens").cloned().unwrap_or(Value::Null);
        let custom = match usage_result {
            Ok(usage_info) => set_usage_state(
                account.get("custom"),
                "ok",
                "",
                Some(usage_info),
                Value::Null,
            ),
            Err(error) => {
                let message = raw_string_field(&error, "message")
                    .chars()
                    .next()
                    .map(|_| raw_string_field(&error, "message"))
                    .unwrap_or_else(|| "Usage sync failed, please try again later".to_string());
                set_usage_state(account.get("custom"), "error", &message, None, error)
            }
        };

        if let Ok(store) =
            add_account_to_store(json!({ "tokens": tokens, "custom": custom }), false)
        {
            emit_store_updated(&app, store);
        }
    });
}
