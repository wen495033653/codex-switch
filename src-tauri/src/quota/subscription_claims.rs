//! The plan badge and subscription expiry shown on an account card come from the
//! id_token claims `chatgpt_plan_type` / `chatgpt_subscription_active_until`. Those
//! claims only change when the token is re-issued through a refresh_token grant, so a
//! renewal or plan change stays invisible until the next token refresh. This module
//! detects stale claims from the fresh `/wham/usage` plan_type and re-issues the token.

use super::auth_refresh::refresh_stored_account_tokens;
use crate::{
    accounts::{
        account_id_from_account, decode_jwt_payload, find_store_account, mark_account_auth_error,
    },
    events::emit_store_updated,
    json_util::{raw_string_field, string_field},
    time_util::parse_rfc3339_seconds,
};
use serde_json::Value;
use tauri::AppHandle;
use time::OffsetDateTime;

/// Stale claims trigger at most one token re-issue per account per day. A re-issued
/// id_token can still carry the old subscription snapshot (observed on 2026-09-14:
/// `last_checked` stayed at the expired date after two refresh_token grants), so
/// rotating the refresh_token on every background quota refresh would be pure cost.
const STALE_CLAIMS_REFRESH_MIN_INTERVAL_SECONDS: i64 = 24 * 60 * 60;
const UNPAID_PLAN_TYPES: [&str; 3] = ["free", "guest", "unknown"];

fn auth_claims(account: &Value) -> Option<Value> {
    let id_token = account
        .get("tokens")
        .and_then(|tokens| tokens.get("id_token"))
        .and_then(Value::as_str)?;
    decode_jwt_payload(id_token)
        .ok()?
        .get("https://api.openai.com/auth")
        .cloned()
}

pub(crate) fn stale_subscription_claims_reason(
    account: &Value,
    usage_info: &Value,
    now: i64,
) -> Option<String> {
    let usage_plan_type = string_field(usage_info, "plan_type").to_ascii_lowercase();
    if usage_plan_type.is_empty() {
        return None;
    }
    let auth = auth_claims(account)?;
    let claim_plan_type = string_field(&auth, "chatgpt_plan_type").to_ascii_lowercase();
    if claim_plan_type != usage_plan_type {
        return Some(format!(
            "id_token plan_type={claim_plan_type:?} usage plan_type={usage_plan_type:?}"
        ));
    }

    if UNPAID_PLAN_TYPES.contains(&usage_plan_type.as_str()) {
        return None;
    }
    let active_until = raw_string_field(&auth, "chatgpt_subscription_active_until");
    let active_until_at = parse_rfc3339_seconds(&active_until)?;
    if active_until_at <= now {
        return Some(format!(
            "id_token subscription_active_until={active_until} passed while usage plan_type={usage_plan_type:?}"
        ));
    }
    None
}

pub(crate) fn stale_claims_refresh_allowed(account: &Value, now: i64) -> bool {
    let custom = account.get("custom").unwrap_or(&Value::Null);
    match parse_rfc3339_seconds(&raw_string_field(custom, "auth_last_refresh_at")) {
        Some(last_refresh_at) => now - last_refresh_at >= STALE_CLAIMS_REFRESH_MIN_INTERVAL_SECONDS,
        None => true,
    }
}

fn account_log_label(account: &Value) -> String {
    account_id_from_account(account)
        .map(|account_id| account_id.chars().take(8).collect())
        .unwrap_or_else(|_| "unknown".to_string())
}

pub(crate) fn subscription_claims_summary(account: &Value) -> String {
    let auth = auth_claims(account).unwrap_or(Value::Null);
    format!(
        "plan_type={:?} active_until={:?} last_checked={:?}",
        string_field(&auth, "chatgpt_plan_type"),
        string_field(&auth, "chatgpt_subscription_active_until"),
        string_field(&auth, "chatgpt_subscription_last_checked")
    )
}

/// Re-issues the stored tokens when `usage_info` proves the id_token subscription
/// claims are stale. Returns whether a token refresh was performed.
pub(crate) fn refresh_stale_subscription_claims(
    app: &AppHandle,
    profile_id: &str,
    usage_info: &Value,
) -> bool {
    let account = match find_store_account(profile_id) {
        Ok(account) => account,
        Err(err) => {
            eprintln!("[subscription-claims] 读取账号失败，跳过订阅信息检查: {err}");
            return false;
        }
    };
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let Some(reason) = stale_subscription_claims_reason(&account, usage_info, now) else {
        return false;
    };
    let label = account_log_label(&account);
    if !stale_claims_refresh_allowed(&account, now) {
        eprintln!(
            "[subscription-claims] account={label} 订阅信息已过期，但距上次 token 刷新不足 {STALE_CLAIMS_REFRESH_MIN_INTERVAL_SECONDS}s，本轮跳过: {reason}"
        );
        return false;
    }

    eprintln!("[subscription-claims] account={label} 订阅信息已过期，重新签发 token: {reason}");
    match refresh_stored_account_tokens(profile_id) {
        Ok(store) => {
            emit_store_updated(app, store);
            match find_store_account(profile_id) {
                Ok(refreshed) => eprintln!(
                    "[subscription-claims] account={label} token 已重新签发，当前 {}",
                    subscription_claims_summary(&refreshed)
                ),
                Err(err) => eprintln!(
                    "[subscription-claims] account={label} token 已重新签发，但回读账号失败: {err}"
                ),
            }
            true
        }
        Err(err) => {
            eprintln!("[subscription-claims] account={label} 重新签发 token 失败: {err}");
            if let Ok(store) = mark_account_auth_error(profile_id, &err) {
                emit_store_updated(app, store);
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose, Engine as _};
    use serde_json::json;

    const NOW: i64 = 1_789_400_000; // 2026-09-14T15:33:20Z

    fn account(plan_type: &str, active_until: &str, last_refresh_at: &str) -> Value {
        let claims = json!({
            "email": "fixture@example.invalid",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "fixture-account-id",
                "chatgpt_plan_type": plan_type,
                "chatgpt_subscription_active_until": active_until,
                "chatgpt_subscription_last_checked": "2026-09-04T08:31:15Z"
            }
        });
        let payload = general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
        json!({
            "tokens": {
                "id_token": format!("fixture.{payload}.fixture"),
                "access_token": "fixture-access",
                "refresh_token": "fixture-refresh",
                "account_id": "fixture-account-id"
            },
            "custom": {
                "auth_last_refresh_at": last_refresh_at
            }
        })
    }

    fn usage(plan_type: &str) -> Value {
        json!({ "plan_type": plan_type })
    }

    #[test]
    fn plan_type_change_marks_claims_stale() {
        let reason = stale_subscription_claims_reason(
            &account("plus", "2026-10-04T00:00:00Z", ""),
            &usage("pro"),
            NOW,
        )
        .unwrap();

        assert!(reason.contains("plan_type=\"plus\""), "{reason}");
        assert!(reason.contains("plan_type=\"pro\""), "{reason}");
    }

    #[test]
    fn passed_active_until_with_paid_plan_marks_claims_stale() {
        let reason = stale_subscription_claims_reason(
            &account("pro", "2026-09-04T12:29:43+00:00", ""),
            &usage("pro"),
            NOW,
        )
        .unwrap();

        assert!(reason.contains("2026-09-04T12:29:43+00:00"), "{reason}");
    }

    #[test]
    fn future_active_until_with_same_plan_is_fresh() {
        assert_eq!(
            stale_subscription_claims_reason(
                &account("pro", "2026-09-19T21:44:23+00:00", ""),
                &usage("pro"),
                NOW
            ),
            None
        );
    }

    #[test]
    fn free_plan_ignores_active_until() {
        assert_eq!(
            stale_subscription_claims_reason(
                &account("free", "2026-09-04T00:00:00Z", ""),
                &usage("free"),
                NOW
            ),
            None
        );
    }

    #[test]
    fn usage_without_plan_type_gives_no_evidence() {
        assert_eq!(
            stale_subscription_claims_reason(
                &account("pro", "2026-09-04T00:00:00Z", ""),
                &usage(""),
                NOW
            ),
            None
        );
    }

    #[test]
    fn recent_token_refresh_throttles_stale_claims_refresh() {
        let recent = account("pro", "2026-09-04T00:00:00Z", "2026-09-13T16:00:00Z");
        let old = account("pro", "2026-09-04T00:00:00Z", "2026-09-13T15:00:00Z");
        let never = account("pro", "2026-09-04T00:00:00Z", "");

        assert!(!stale_claims_refresh_allowed(&recent, NOW));
        assert!(stale_claims_refresh_allowed(&old, NOW));
        assert!(stale_claims_refresh_allowed(&never, NOW));
    }
}
