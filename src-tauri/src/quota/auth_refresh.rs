use crate::{
    accounts::{
        access_token_from_account, account_from_exchange_preserve_usage, account_id_from_account,
        account_with_custom, exchange_refresh_token, find_store_account, mark_account_auth_error,
        normalize_custom, normalize_tokens, profile_id_from_account, read_store_value,
        refresh_token_from_account, set_auth_state, sync_auth_file_if_active, update_store_account,
    },
    events::emit_store_updated,
    json_util::{raw_string_field, string_field},
    session_sync_diagnostics::log_session_sync_event,
    time_util::parse_rfc3339_seconds,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    sync::{Condvar, Mutex},
    thread,
    time::Duration as StdDuration,
};
use tauri::AppHandle;
use time::OffsetDateTime;

const AUTO_AUTH_INTERVAL_SECONDS: u64 = 15 * 60;
const AUTO_AUTH_REFRESH_LEAD_SECONDS: i64 = 30 * 60;
const AUTO_AUTH_FALLBACK_REFRESH_SECONDS: i64 = 24 * 60 * 60;

/// Accounts whose refresh_token is being exchanged right now. OpenAI rotates the
/// refresh_token on every exchange and rejects the old one with `refresh_token_reused`, so
/// two exchanges of the same stored token would mark a healthy account as failed.
static ROTATING_ACCOUNTS: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static ROTATION_FINISHED: Condvar = Condvar::new();

struct AccountRotationGuard {
    profile_id: String,
}

impl Drop for AccountRotationGuard {
    fn drop(&mut self) {
        let mut rotating = ROTATING_ACCOUNTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(set) = rotating.as_mut() {
            set.remove(&self.profile_id);
        }
        ROTATION_FINISHED.notify_all();
    }
}

/// Waits until no other thread is exchanging this account's token, then claims it. The set
/// only holds profile ids, so a poisoned lock carries no half-updated state.
fn lock_account_rotation(profile_id: &str) -> AccountRotationGuard {
    let mut rotating = ROTATING_ACCOUNTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    while rotating
        .as_ref()
        .is_some_and(|set| set.contains(profile_id))
    {
        rotating = ROTATION_FINISHED
            .wait(rotating)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    rotating
        .get_or_insert_with(HashSet::new)
        .insert(profile_id.to_string());
    AccountRotationGuard {
        profile_id: profile_id.to_string(),
    }
}

/// Logs identify an account by the first 8 characters of its profile id (the ChatGPT account
/// id prefix), never the full id, email or tokens.
pub(super) fn account_log_label(profile_id: &str) -> String {
    profile_id.chars().take(8).collect()
}

fn should_auto_refresh_account(account: &Value) -> bool {
    if normalize_tokens(account.get("tokens")).is_err() {
        return false;
    }

    let custom = normalize_custom(account.get("custom"));
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

fn mark_account_auth_refreshing(profile_id: &str, message: &str) -> Result<Value, String> {
    update_store_account(profile_id, |account| {
        let custom = set_auth_state(
            account.get("custom"),
            "refreshing",
            message,
            Value::Null,
            None,
            None,
        );
        Ok(account_with_custom(account, custom))
    })
}

/// Exchanges the stored refresh_token and persists the new tokens before returning, so a
/// rotated token is on disk even if a later step fails. The caller holds the rotation guard.
fn rotate_account_tokens(profile_id: &str, account: &Value) -> Result<Value, String> {
    let expected_account_id = account_id_from_account(account)?;
    let exchange = exchange_refresh_token(&refresh_token_from_account(account))?;
    let refreshed_account_id = string_field(&exchange, "account_id");
    if refreshed_account_id.is_empty() {
        return Err("刷新结果缺少 account_id".to_string());
    }
    if refreshed_account_id != expected_account_id {
        return Err("刷新后账号标识不一致".to_string());
    }

    let store = update_store_account(profile_id, |latest| {
        account_from_exchange_preserve_usage(&exchange, latest.get("custom"))
    })?;
    sync_auth_file_if_active(profile_id)?;
    Ok(store)
}

/// The only way stored refresh_tokens are exchanged. Exchanges for one account run one at a
/// time. `stale_access_token` is the access token the caller saw rejected or scheduled for
/// renewal; when the stored access token no longer matches it, another caller already
/// rotated the tokens and the current store is returned without a second exchange. `None`
/// always exchanges (an explicit user request).
pub(crate) fn refresh_stored_account_tokens(
    profile_id: &str,
    stale_access_token: Option<&str>,
) -> Result<Value, String> {
    let _guard = lock_account_rotation(profile_id);
    let account = find_store_account(profile_id)?;
    if stale_access_token.is_some_and(|stale| access_token_from_account(&account) != stale) {
        return read_store_value();
    }
    rotate_account_tokens(profile_id, &account)
}

enum DueRefresh {
    Rotated(Value),
    AlreadyRotated,
}

fn refresh_due_account(
    app: &AppHandle,
    profile_id: &str,
    scheduled_access_token: &str,
) -> Result<DueRefresh, String> {
    let _guard = lock_account_rotation(profile_id);
    let account = find_store_account(profile_id)?;
    if access_token_from_account(&account) != scheduled_access_token {
        return Ok(DueRefresh::AlreadyRotated);
    }
    emit_store_updated(
        app,
        mark_account_auth_refreshing(profile_id, "认证刷新中，请稍候...")?,
    );
    rotate_account_tokens(profile_id, &account).map(DueRefresh::Rotated)
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
        let Ok(profile_id) = profile_id_from_account(&account) else {
            continue;
        };

        match refresh_due_account(app, &profile_id, &access_token_from_account(&account)) {
            Ok(DueRefresh::Rotated(store)) => {
                updated += 1;
                emit_store_updated(app, store);
            }
            Ok(DueRefresh::AlreadyRotated) => {}
            Err(err) => {
                failed += 1;
                let mark_error = match mark_account_auth_error(&profile_id, &err) {
                    Ok(store) => {
                        emit_store_updated(app, store);
                        None
                    }
                    Err(mark_err) => Some(mark_err),
                };
                log_session_sync_event(
                    "account_auth_auto_refresh_error",
                    json!({
                        "account": account_log_label(&profile_id),
                        "error": err,
                        "markError": mark_error
                    }),
                );
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
            log_session_sync_event(
                "account_auth_auto_refresh_pass_error",
                json!({ "error": err, "nextRunSeconds": AUTO_AUTH_INTERVAL_SECONDS }),
            );
        }
        thread::sleep(StdDuration::from_secs(AUTO_AUTH_INTERVAL_SECONDS));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    #[test]
    fn rotation_guard_serializes_same_account_and_allows_others() {
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let handles = (0..4)
            .map(|_| {
                let active = Arc::clone(&active);
                let max_active = Arc::clone(&max_active);
                thread::spawn(move || {
                    let _guard = lock_account_rotation("profile-serialized");
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_active.fetch_max(now, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(20));
                    active.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(max_active.load(Ordering::SeqCst), 1);

        let _first = lock_account_rotation("profile-a");
        let other = thread::spawn(|| {
            let _second = lock_account_rotation("profile-b");
        });
        other.join().unwrap();
    }

    #[test]
    fn persisted_refreshing_status_does_not_block_auto_refresh() {
        let account = json!({
            "tokens": {
                "id_token": "id",
                "access_token": "access",
                "refresh_token": "refresh",
                "account_id": "acct"
            },
            "custom": { "auth_status": "refreshing" }
        });

        assert!(should_auto_refresh_account(&account));
    }
}
