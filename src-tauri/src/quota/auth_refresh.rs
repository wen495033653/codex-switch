use crate::{
    accounts::{
        account_from_exchange_preserve_usage, account_id_from_account, add_account_to_store,
        exchange_refresh_token, find_store_account, mark_account_auth_error, normalize_custom,
        normalize_tokens, read_store_value, set_auth_state, sync_auth_file_if_active,
    },
    events::emit_store_updated,
    json_util::{raw_string_field, string_field},
    time_util::parse_rfc3339_seconds,
};
use serde_json::{json, Value};
use std::{thread, time::Duration as StdDuration};
use tauri::AppHandle;
use time::OffsetDateTime;

const AUTO_AUTH_INTERVAL_SECONDS: u64 = 15 * 60;
const AUTO_AUTH_REFRESH_LEAD_SECONDS: i64 = 30 * 60;
const AUTO_AUTH_FALLBACK_REFRESH_SECONDS: i64 = 24 * 60 * 60;

fn should_auto_refresh_account(account: &Value) -> bool {
    if normalize_tokens(account.get("tokens")).is_err() {
        return false;
    }

    let custom = normalize_custom(account.get("custom"));
    if raw_string_field(&custom, "auth_status") == "refreshing" {
        return false;
    }

    let now = OffsetDateTime::now_utc().unix_timestamp();
    if let Some(expires_at) = parse_rfc3339_seconds(&raw_string_field(&custom, "auth_expires_at")) {
        return expires_at - now <= AUTO_AUTH_REFRESH_LEAD_SECONDS;
    }

    if let Some(last_refresh_at) =
        parse_rfc3339_seconds(&raw_string_field(&custom, "auth_last_refresh_at"))
    {
        return now - last_refresh_at >= AUTO_AUTH_FALLBACK_REFRESH_SECONDS;
    }

    true
}

fn mark_account_auth_refreshing(account_id: &str, message: &str) -> Result<Value, String> {
    let account = find_store_account(account_id)?;
    let tokens = account.get("tokens").cloned().unwrap_or(Value::Null);
    let custom = set_auth_state(
        account.get("custom"),
        "refreshing",
        message,
        Value::Null,
        None,
        None,
    );
    add_account_to_store(json!({ "tokens": tokens, "custom": custom }), false)
}

pub(crate) fn refresh_stored_account_tokens(account_id: &str) -> Result<Value, String> {
    let account = find_store_account(account_id)?;
    let previous_refresh_token = account
        .get("tokens")
        .and_then(|tokens| tokens.get("refresh_token"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let exchange = exchange_refresh_token(&previous_refresh_token)?;
    let refreshed_account_id = string_field(&exchange, "account_id");
    if refreshed_account_id.is_empty() {
        return Err("刷新结果缺少 account_id".to_string());
    }
    if refreshed_account_id != account_id {
        return Err("刷新后账号标识不一致".to_string());
    }

    let latest = find_store_account(account_id).unwrap_or(account);
    let next_account = account_from_exchange_preserve_usage(&exchange, latest.get("custom"))?;
    let store = add_account_to_store(next_account, false)?;
    sync_auth_file_if_active(account_id)?;
    Ok(store)
}

fn refresh_due_account_tokens_once(app: &AppHandle) -> Result<Value, String> {
    let store = read_store_value()?;
    let due_accounts = store
        .get("accounts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(should_auto_refresh_account)
        .collect::<Vec<_>>();

    let mut updated = 0_u64;
    let mut failed = 0_u64;
    for account in due_accounts {
        let Ok(account_id) = account_id_from_account(&account) else {
            continue;
        };

        if let Ok(store) = mark_account_auth_refreshing(&account_id, "认证刷新中，请稍候...")
        {
            emit_store_updated(app, store);
        }

        match refresh_stored_account_tokens(&account_id) {
            Ok(store) => {
                updated += 1;
                emit_store_updated(app, store);
            }
            Err(err) => {
                failed += 1;
                if let Ok(store) = mark_account_auth_error(&account_id, &err) {
                    emit_store_updated(app, store);
                }
            }
        }
    }

    Ok(json!({
        "ok": true,
        "updated": updated,
        "failed": failed
    }))
}

pub(crate) fn start_account_token_auto_refresher(app: AppHandle) {
    thread::spawn(move || loop {
        if let Err(err) = refresh_due_account_tokens_once(&app) {
            eprintln!("认证自动刷新失败: {err}");
        }
        thread::sleep(StdDuration::from_secs(AUTO_AUTH_INTERVAL_SECONDS));
    });
}
