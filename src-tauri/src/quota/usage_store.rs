use super::{
    auth_refresh::refresh_stored_account_tokens, subscription::refresh_account_subscription,
};
use crate::{
    accounts::{
        access_token_from_account, account_with_custom, build_error_state,
        error_state_is_auth_rejected, find_store_account, get_usage, mark_account_auth_error,
        set_usage_result, update_active_store_account, update_store_account,
        INTERACTIVE_REQUEST_TIMEOUT_MS,
    },
    app_log::{account_label, log_event, truncate_for_log},
    codex_session_usage::inherit_stored_usage_fields,
    events::emit_store_updated,
    json_util::{raw_string_field, value_u64_field},
    time_util::now_string,
};
use serde_json::{json, Value};
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
        let usage_ok = usage_result.is_ok();
        match store_account_usage_result(&profile_id, usage_result) {
            Ok(store) => emit_store_updated(&app, store),
            Err(err) => {
                log_event(
                    "account_usage_sync_store_error",
                    json!({
                        "account": account_label(&profile_id),
                        "usageOk": usage_ok,
                        "error": err
                    }),
                );
                return;
            }
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

pub(super) fn get_usage_with_auth_retry(
    app: &AppHandle,
    profile_id: &str,
    account_id: &str,
    access_token: &str,
    timeout_ms: u64,
) -> Result<Value, Value> {
    match get_usage(access_token, account_id, timeout_ms) {
        Ok(usage_info) => Ok(usage_info),
        Err(error) if error_state_is_auth_rejected(&error) => {
            // Evidence for the open question whether a 403 here is ever a Cloudflare page rather
            // than a rejected token (docs/development/account-store.md, "待定").
            log_event(
                "account_usage_rejected_error",
                json!({
                    "account": account_label(profile_id),
                    "status": value_u64_field(&error, "status"),
                    "code": raw_string_field(&error, "code"),
                    "message": raw_string_field(&error, "message"),
                    // Error bodies can be whole HTML pages; the start tells what answered.
                    "rawMessage": truncate_for_log(&raw_string_field(&error, "raw_message"), 300),
                    "action": "rotate_tokens"
                }),
            );
            match refresh_stored_account_tokens(profile_id, Some(access_token)) {
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
                    let mark_error = match mark_account_auth_error(profile_id, &refresh_err) {
                        Ok(store) => {
                            emit_store_updated(app, store);
                            None
                        }
                        Err(mark_err) => Some(mark_err),
                    };
                    log_event(
                        "account_auth_retry_refresh_error",
                        json!({
                            "account": account_label(profile_id),
                            "error": refresh_err,
                            "markError": mark_error
                        }),
                    );
                    Err(error)
                }
            }
        }
        Err(error) => Err(error),
    }
}

fn usage_window_used_percent(usage: &Value, key: &str) -> Option<f64> {
    usage
        .get("rate_limit")
        .and_then(|rate_limit| rate_limit.get(key))
        .and_then(|window| window.get("used_percent"))
        .and_then(|value| match value {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.parse::<f64>().ok(),
            _ => None,
        })
}

fn is_limit_window_changed(old_usage: &Value, new_usage: &Value) -> bool {
    ["primary_window", "secondary_window"].iter().any(|key| {
        usage_window_used_percent(old_usage, key) != usage_window_used_percent(new_usage, key)
    })
}

fn stored_usage_info(account: &Value) -> Value {
    account
        .get("custom")
        .and_then(|custom| custom.get("usage_info"))
        .cloned()
        .unwrap_or(Value::Null)
}

/// The account as it should be stored after one usage read. A changed limit window means
/// the account was actually used, so it moves to the front.
fn account_with_usage_result(account: &Value, usage_result: Result<Value, Value>) -> Value {
    let old_usage = stored_usage_info(account);
    let touches_last_used = matches!(
        &usage_result,
        Ok(usage_info)
            if !old_usage.is_null()
                && old_usage != *usage_info
                && is_limit_window_changed(&old_usage, usage_info)
    );
    let mut next_custom = set_usage_result(account.get("custom"), usage_result);
    if touches_last_used {
        next_custom["last_used_at"] = Value::String(now_string());
    }
    account_with_custom(account, next_custom)
}

/// Stores one usage read on the account as it is on disk now, keeping its current tokens.
pub(crate) fn store_account_usage_result(
    profile_id: &str,
    usage_result: Result<Value, Value>,
) -> Result<Value, String> {
    update_store_account(profile_id, |account| {
        let custom = set_usage_result(account.get("custom"), usage_result);
        Ok(account_with_custom(account, custom))
    })
}

pub(super) fn update_account_usage_result(
    profile_id: &str,
    usage_result: Result<Value, Value>,
) -> Result<Value, String> {
    update_store_account(profile_id, |account| {
        Ok(account_with_usage_result(account, usage_result))
    })
}

pub(super) fn update_active_account_usage_result(
    profile_id: &str,
    usage_result: Result<Value, Value>,
) -> Result<Option<Value>, String> {
    update_active_store_account(profile_id, |account| {
        let previous_usage = stored_usage_info(account);
        let usage_result =
            usage_result.map(|usage_info| inherit_stored_usage_fields(&previous_usage, usage_info));
        Ok(account_with_usage_result(account, usage_result))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account_with_usage(used_percent: f64) -> Value {
        json!({
            "tokens": { "account_id": "acct" },
            "custom": {
                "last_used_at": "2026-01-01T00:00:00Z",
                "usage_info": {
                    "rate_limit": { "primary_window": { "used_percent": used_percent } }
                }
            }
        })
    }

    #[test]
    fn changed_limit_window_moves_account_to_front() {
        let next = account_with_usage_result(
            &account_with_usage(10.0),
            Ok(json!({ "rate_limit": { "primary_window": { "used_percent": 20.0 } } })),
        );

        assert_ne!(next["custom"]["last_used_at"], "2026-01-01T00:00:00Z");
    }

    #[test]
    fn unchanged_limit_window_keeps_last_used_at() {
        let next = account_with_usage_result(
            &account_with_usage(10.0),
            Ok(json!({ "rate_limit": { "primary_window": { "used_percent": 10.0 } } })),
        );

        assert_eq!(next["custom"]["last_used_at"], "2026-01-01T00:00:00Z");
    }
}
