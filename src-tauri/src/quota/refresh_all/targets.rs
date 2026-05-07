use crate::{accounts::account_id_from_account, time_util::parse_rfc3339_seconds};
use serde_json::Value;
use time::OffsetDateTime;

#[derive(Clone)]
pub(super) struct RefreshTarget {
    pub(super) account_id: String,
    pub(super) access_token: String,
}

pub(super) fn refresh_targets_from_store(store: &Value) -> Vec<RefreshTarget> {
    store
        .get("accounts")
        .and_then(Value::as_array)
        .map(|accounts| {
            accounts
                .iter()
                .filter_map(|account| {
                    let account_id = account_id_from_account(account).ok()?;
                    let access_token = account
                        .get("tokens")
                        .and_then(|tokens| tokens.get("access_token"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if account_id.is_empty() || access_token.is_empty() {
                        return None;
                    }
                    Some(RefreshTarget {
                        account_id,
                        access_token,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn quota_refresh_target(account: &Value) -> bool {
    let account_id = account_id_from_account(account).unwrap_or_default();
    let access_token = account
        .get("tokens")
        .and_then(|tokens| tokens.get("access_token"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    !account_id.is_empty() && !access_token.is_empty()
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

pub(super) fn has_due_background_quota_refresh(store: &Value, interval_minutes: u64) -> bool {
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
