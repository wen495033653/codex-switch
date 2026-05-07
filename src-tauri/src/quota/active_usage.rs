use super::usage_store::update_active_account_usage_result;
use crate::{
    accounts::{account_id_from_account, get_codex_state_value, read_store_with_active_sync},
    codex_session_usage,
    events::emit_store_updated,
    json_util::raw_string_field,
    time_util::parse_rfc3339_seconds,
};
use serde_json::{json, Value};
use std::{thread, time::Duration as StdDuration};
use tauri::AppHandle;

const ACTIVE_QUOTA_INTERVAL_SECONDS: u64 = 60;

struct ActiveQuotaTarget {
    account_id: String,
    min_fetched_at: Option<i64>,
}

fn account_usage_min_fetched_at(account: &Value) -> Option<i64> {
    let custom = account.get("custom").unwrap_or(&Value::Null);
    [
        parse_rfc3339_seconds(&raw_string_field(custom, "last_used_at")),
        custom
            .get("usage_info")
            .and_then(codex_session_usage::usage_info_fetched_at_seconds),
    ]
    .into_iter()
    .flatten()
    .max()
}

fn active_quota_refresh_target() -> Result<Option<ActiveQuotaTarget>, String> {
    let state = get_codex_state_value();
    if raw_string_field(&state, "mode") != "chatgpt" {
        return Ok(None);
    }
    let active_account_id = raw_string_field(&state, "account_id");
    if active_account_id.is_empty() {
        return Ok(None);
    }

    let store = read_store_with_active_sync()?;
    if raw_string_field(&store, "active_id") != active_account_id {
        return Ok(None);
    }

    let account = store
        .get("accounts")
        .and_then(Value::as_array)
        .and_then(|accounts| {
            accounts.iter().find(|account| {
                account_id_from_account(account).unwrap_or_default() == active_account_id
            })
        });
    let Some(account) = account else {
        return Ok(None);
    };

    Ok(Some(ActiveQuotaTarget {
        account_id: active_account_id,
        min_fetched_at: account_usage_min_fetched_at(account),
    }))
}

fn refresh_active_account_usage_once(app: &AppHandle) -> Result<Value, String> {
    let Some(target) = active_quota_refresh_target()? else {
        return Ok(json!({
            "ok": true,
            "skipped": true
        }));
    };

    let Some(usage_info) = codex_session_usage::latest_usage_info()? else {
        return Ok(json!({
            "ok": true,
            "skipped": true,
            "reason": "codex_session_usage_missing"
        }));
    };

    if let (Some(latest), Some(min_fetched_at)) = (
        codex_session_usage::usage_info_fetched_at_seconds(&usage_info),
        target.min_fetched_at,
    ) {
        if latest <= min_fetched_at {
            return Ok(json!({
                "ok": true,
                "skipped": true,
                "reason": "codex_session_usage_stale"
            }));
        }
    }

    match update_active_account_usage_result(&target.account_id, Ok(usage_info))? {
        Some(store) => {
            emit_store_updated(app, store);
            Ok(json!({
                "ok": true
            }))
        }
        None => Ok(json!({
            "ok": true,
            "skipped": true,
            "reason": "active_account_changed"
        })),
    }
}

pub(crate) fn refresh_active_account_usage_in_background(app: AppHandle) {
    thread::spawn(move || {
        if let Err(err) = refresh_active_account_usage_once(&app) {
            eprintln!("当前账号配额同步失败: {err}");
        }
    });
}

pub(crate) fn start_active_quota_auto_refresher(app: AppHandle) {
    thread::spawn(move || loop {
        if let Err(err) = refresh_active_account_usage_once(&app) {
            eprintln!("当前账号配额同步失败: {err}");
        }
        thread::sleep(StdDuration::from_secs(ACTIVE_QUOTA_INTERVAL_SECONDS));
    });
}
