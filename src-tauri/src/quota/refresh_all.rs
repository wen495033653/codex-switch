use crate::{
    accounts::{
        access_token_from_account, account_id_from_account, profile_id_from_account,
        read_store_with_active_sync, BACKGROUND_REQUEST_TIMEOUT_MS,
    },
    events::emit_store_updated,
    json_util::{bool_field, value_u64_field},
    quota::{
        subscription::refresh_account_subscription,
        usage_store::{get_usage_with_auth_retry, update_account_usage_result},
    },
    session_sync_diagnostics::log_session_sync_event,
    settings::{
        normalize_background_refresh_interval_minutes, read_settings_value,
        BACKGROUND_REFRESH_DEFAULT_INTERVAL_MINUTES,
    },
    time_util::{now_string, parse_rfc3339_seconds},
};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    thread,
    time::Duration as StdDuration,
};
use tauri::{AppHandle, Emitter};
use time::OffsetDateTime;

pub(crate) fn begin_refresh_all_quotas(
    app: AppHandle,
    runtime: Arc<RefreshAllRuntime>,
    source: &'static str,
) -> Result<Value, String> {
    let store = read_store_with_active_sync()?;
    let targets = refresh_targets_from_store(&store);
    let status = match start_refresh_all_status_if_idle(
        runtime.as_ref(),
        json!({
            "running": true,
            "total": targets.len(),
            "completed": 0,
            "updated": 0,
            "failed": 0,
            "started_at": now_string(),
            "finished_at": "",
            "message": if targets.is_empty() { "没有可刷新的账号" } else { "后台刷新中" },
            "source": source
        }),
    ) {
        Ok(status) => status,
        Err(running_status) => {
            return Ok(json!({
                "ok": true,
                "message": "后台刷新仍在进行中",
                "started": false,
                "status": running_status,
                "store": store
            }));
        }
    };
    emit_refresh_all_status(&app, status.clone());
    start_refresh_all_quotas_in_background(app, runtime, targets);

    Ok(json!({
        "ok": true,
        "message": "已开始后台刷新配额",
        "started": true,
        "status": status,
        "store": store
    }))
}

fn background_refresh_settings() -> Result<(bool, u64), String> {
    let settings = read_settings_value()?;
    let enabled = settings
        .get("background_refresh_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let interval_minutes = normalize_background_refresh_interval_minutes(value_u64_field(
        &settings,
        "background_refresh_interval_minutes",
    ));
    Ok((enabled, interval_minutes))
}

pub(crate) fn start_background_quota_auto_refresher(
    app: AppHandle,
    runtime: Arc<RefreshAllRuntime>,
) {
    thread::spawn(move || loop {
        let (enabled, interval_minutes) = match background_refresh_settings() {
            Ok(value) => value,
            Err(err) => {
                log_session_sync_event(
                    "background_refresh_settings_read_error",
                    json!({
                        "error": err,
                        "handling": "use_defaults",
                        "enabled": true,
                        "intervalMinutes": BACKGROUND_REFRESH_DEFAULT_INTERVAL_MINUTES
                    }),
                );
                (true, BACKGROUND_REFRESH_DEFAULT_INTERVAL_MINUTES)
            }
        };
        if enabled {
            match read_store_with_active_sync() {
                Ok(store) => {
                    if has_due_background_quota_refresh(&store, interval_minutes) {
                        if let Err(err) =
                            begin_refresh_all_quotas(app.clone(), Arc::clone(&runtime), "auto")
                        {
                            log_session_sync_event(
                                "background_refresh_start_error",
                                json!({ "error": err, "intervalMinutes": interval_minutes }),
                            );
                        }
                    }
                }
                Err(err) => log_session_sync_event(
                    "background_refresh_store_read_error",
                    json!({
                        "error": err,
                        "handling": "skip_this_round",
                        "intervalMinutes": interval_minutes
                    }),
                ),
            }
        }
        thread::sleep(StdDuration::from_secs(interval_minutes * 60));
    });
}

pub(crate) struct RefreshAllRuntime {
    status: Mutex<Value>,
}

fn default_refresh_all_status() -> Value {
    json!({
        "running": false,
        "total": 0,
        "completed": 0,
        "updated": 0,
        "failed": 0,
        "started_at": "",
        "finished_at": "",
        "message": "",
        "source": ""
    })
}

impl Default for RefreshAllRuntime {
    fn default() -> Self {
        Self {
            status: Mutex::new(default_refresh_all_status()),
        }
    }
}

pub(crate) fn get_refresh_all_status_value(runtime: &RefreshAllRuntime) -> Value {
    runtime
        .status
        .lock()
        .map(|status| status.clone())
        .unwrap_or_else(|_| default_refresh_all_status())
}

/// Checks and claims the "running" flag under one lock, so a manual refresh and the timer
/// cannot both start a refresh-all pass. Returns the current status when one is running.
fn start_refresh_all_status_if_idle(
    runtime: &RefreshAllRuntime,
    status: Value,
) -> Result<Value, Value> {
    let mut current = runtime
        .status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if bool_field(&current, "running") {
        return Err(current.clone());
    }
    *current = status.clone();
    Ok(status)
}

fn update_refresh_all_status_value<F>(runtime: &RefreshAllRuntime, update: F) -> Value
where
    F: FnOnce(Value) -> Value,
{
    if let Ok(mut current) = runtime.status.lock() {
        let next = update(current.clone());
        *current = next.clone();
        return next;
    }
    default_refresh_all_status()
}

fn emit_refresh_all_status(app: &AppHandle, status: Value) {
    let _ = app.emit("refresh-all-status", json!({ "status": status }));
}

#[derive(Clone)]
struct RefreshTarget {
    pub(super) profile_id: String,
    pub(super) account_id: String,
    pub(super) access_token: String,
}

fn refresh_targets_from_store(store: &Value) -> Vec<RefreshTarget> {
    store
        .get("accounts")
        .and_then(Value::as_array)
        .map(|accounts| {
            accounts
                .iter()
                .filter_map(|account| {
                    let profile_id = profile_id_from_account(account).ok()?;
                    let account_id = account_id_from_account(account).ok()?;
                    let access_token = access_token_from_account(account);
                    if profile_id.is_empty() || account_id.is_empty() || access_token.is_empty() {
                        return None;
                    }
                    Some(RefreshTarget {
                        profile_id,
                        account_id,
                        access_token,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn quota_refresh_target(account: &Value) -> bool {
    let profile_id = profile_id_from_account(account).unwrap_or_default();
    let account_id = account_id_from_account(account).unwrap_or_default();
    !profile_id.is_empty()
        && !account_id.is_empty()
        && !access_token_from_account(account).is_empty()
}

fn quota_refresh_timestamp_seconds(account: &Value) -> Option<i64> {
    let custom = account.get("custom")?;
    let usage_fetched_at = custom
        .get("usage_info")
        .and_then(|usage| usage.get("fetched_at"))
        .and_then(Value::as_str)
        .and_then(parse_rfc3339_seconds);
    let usage_error_at = custom
        .get("usage_error")
        .and_then(|error| error.get("time"))
        .and_then(Value::as_str)
        .and_then(parse_rfc3339_seconds);

    match (usage_fetched_at, usage_error_at) {
        (Some(fetched_at), Some(error_at)) => Some(fetched_at.max(error_at)),
        (Some(fetched_at), None) => Some(fetched_at),
        (None, Some(error_at)) => Some(error_at),
        (None, None) => None,
    }
}

fn should_background_refresh_account_quota(
    account: &Value,
    now: i64,
    interval_seconds: i64,
) -> bool {
    if !quota_refresh_target(account) {
        return false;
    }

    quota_refresh_timestamp_seconds(account)
        .map(|refreshed_at| now.saturating_sub(refreshed_at) >= interval_seconds)
        .unwrap_or(true)
}

fn has_due_background_quota_refresh(store: &Value, interval_minutes: u64) -> bool {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let interval_seconds = i64::try_from(interval_minutes.saturating_mul(60)).unwrap_or(i64::MAX);
    store
        .get("accounts")
        .and_then(Value::as_array)
        .is_some_and(|accounts| {
            accounts.iter().any(|account| {
                should_background_refresh_account_quota(account, now, interval_seconds)
            })
        })
}

fn start_refresh_all_quotas_in_background(
    app: AppHandle,
    runtime: Arc<RefreshAllRuntime>,
    targets: Vec<RefreshTarget>,
) {
    thread::spawn(move || {
        if targets.is_empty() {
            let status = update_refresh_all_status_value(runtime.as_ref(), |mut current| {
                current["running"] = Value::Bool(false);
                current["finished_at"] = Value::String(now_string());
                current["message"] = Value::String("没有可刷新的账号".to_string());
                current
            });
            emit_refresh_all_status(&app, status);
            return;
        }

        for target in targets {
            let usage_result = get_usage_with_auth_retry(
                &app,
                &target.profile_id,
                &target.account_id,
                &target.access_token,
                BACKGROUND_REQUEST_TIMEOUT_MS,
            );
            let usage_ok = usage_result.is_ok();
            let store_result = update_account_usage_result(&target.profile_id, usage_result);
            let store_update_ok = store_result.is_ok();
            if let Ok(store) = store_result {
                emit_store_updated(&app, store);
            }
            if usage_ok && store_update_ok {
                if let Some(store) =
                    refresh_account_subscription(&target.profile_id, BACKGROUND_REQUEST_TIMEOUT_MS)
                {
                    emit_store_updated(&app, store);
                }
            }

            let status = update_refresh_all_status_value(runtime.as_ref(), |mut current| {
                let completed = value_u64_field(&current, "completed").unwrap_or(0) + 1;
                let success = usage_ok && store_update_ok;
                let updated =
                    value_u64_field(&current, "updated").unwrap_or(0) + if success { 1 } else { 0 };
                let failed =
                    value_u64_field(&current, "failed").unwrap_or(0) + if success { 0 } else { 1 };
                let total = value_u64_field(&current, "total").unwrap_or(0);
                current["completed"] = json!(completed);
                current["updated"] = json!(updated);
                current["failed"] = json!(failed);
                current["message"] = Value::String(format!("后台刷新中（{completed}/{total}）"));
                current
            });
            emit_refresh_all_status(&app, status);
        }

        match read_store_with_active_sync() {
            Ok(store) => emit_store_updated(&app, store),
            Err(err) => log_session_sync_event(
                "refresh_all_final_store_read_error",
                json!({ "error": err }),
            ),
        }
        let status = update_refresh_all_status_value(runtime.as_ref(), |mut current| {
            let updated = value_u64_field(&current, "updated").unwrap_or(0);
            let failed = value_u64_field(&current, "failed").unwrap_or(0);
            current["running"] = Value::Bool(false);
            current["finished_at"] = Value::String(now_string());
            current["message"] = Value::String(if failed > 0 {
                format!("已刷新 {updated} 个账号，{failed} 个失败")
            } else {
                format!("已刷新 {updated} 个账号")
            });
            current
        });
        emit_refresh_all_status(&app, status);
    });
}
