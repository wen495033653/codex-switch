//! Keeps `custom.subscription` in sync with `/backend-api/subscriptions`.
//!
//! The renewal date shown on an account card used to come from the id_token claim
//! `chatgpt_subscription_active_until`, which OpenAI does not always refresh. Every quota
//! refresh therefore also reads the subscription endpoint; the claim stays as the fallback
//! for accounts whose endpoint read has never succeeded.

use crate::{
    accounts::{
        account_id_from_account, add_account_to_store, find_store_account, get_subscription,
        set_subscription_state,
    },
    json_util::{raw_string_field, string_field, value_u64_field},
};
use serde_json::{json, Value};

fn account_log_label(profile_id: &str) -> String {
    profile_id.chars().take(8).collect()
}

fn log_subscription_error(profile_id: &str, error: &Value) {
    eprintln!(
        "[subscription] account={} 订阅信息刷新失败 code={:?} status={:?} message={:?} raw={:?}",
        account_log_label(profile_id),
        raw_string_field(error, "code"),
        value_u64_field(error, "status"),
        raw_string_field(error, "message"),
        raw_string_field(error, "raw_message")
    );
}

fn account_access_token(account: &Value) -> String {
    account
        .get("tokens")
        .and_then(|tokens| tokens.get("access_token"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Reads the subscription snapshot for one stored account and persists it. Credentials are
/// taken from the store rather than from the caller, so a token rotated during the quota
/// refresh is used here too. A failed read keeps the previous snapshot instead of clearing
/// it, so one network error does not blank the renewal date. Returns the updated store only
/// when the stored snapshot actually changed.
pub(crate) fn refresh_account_subscription(profile_id: &str, timeout_ms: u64) -> Option<Value> {
    let account = match find_store_account(profile_id) {
        Ok(account) => account,
        Err(err) => {
            eprintln!(
                "[subscription] account={} 读取账号失败，跳过订阅信息刷新: {err}",
                account_log_label(profile_id)
            );
            return None;
        }
    };
    let account_id = account_id_from_account(&account).unwrap_or_default();
    let access_token = account_access_token(&account);
    if account_id.is_empty() || access_token.is_empty() {
        return None;
    }

    let subscription = match get_subscription(&access_token, &account_id, timeout_ms) {
        Ok(subscription) => subscription,
        Err(error) => {
            log_subscription_error(profile_id, &error);
            return None;
        }
    };

    let latest = find_store_account(profile_id).unwrap_or(account);
    let previous = latest
        .get("custom")
        .and_then(|custom| custom.get("subscription"))
        .cloned()
        .unwrap_or(Value::Null);
    if previous == subscription {
        return None;
    }

    let tokens = latest.get("tokens").cloned().unwrap_or(Value::Null);
    let custom = set_subscription_state(latest.get("custom"), subscription.clone());
    match add_account_to_store(json!({ "tokens": tokens, "custom": custom }), false) {
        Ok(store) => {
            eprintln!(
                "[subscription] account={} 订阅信息已更新 active_until={:?} will_renew={} is_delinquent={}",
                account_log_label(profile_id),
                string_field(&subscription, "active_until"),
                subscription["will_renew"],
                subscription["is_delinquent"]
            );
            Some(store)
        }
        Err(err) => {
            eprintln!(
                "[subscription] account={} 订阅信息写入失败: {err}",
                account_log_label(profile_id)
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::decode_jwt_payload;
    use std::env;

    fn claim_active_until(account: &Value) -> String {
        account
            .get("tokens")
            .and_then(|tokens| tokens.get("id_token"))
            .and_then(Value::as_str)
            .and_then(|id_token| decode_jwt_payload(id_token).ok())
            .and_then(|claims| claims.get("https://api.openai.com/auth").cloned())
            .map(|auth| string_field(&auth, "chatgpt_subscription_active_until"))
            .unwrap_or_default()
    }

    /// Real-environment check for the subscription endpoint. It reads the live account named
    /// by CODEX_SWITCH_REAL_SUBSCRIPTION_PROFILE_ID and prints the stored snapshot next to the
    /// id_token claim. Read-only against OpenAI: no token is rotated.
    #[test]
    #[ignore = "calls the live subscriptions endpoint; set CODEX_SWITCH_REAL_SUBSCRIPTION_PROFILE_ID"]
    fn real_subscription_endpoint_fills_renewal_date() {
        let Ok(profile_id) = env::var("CODEX_SWITCH_REAL_SUBSCRIPTION_PROFILE_ID") else {
            eprintln!("CODEX_SWITCH_REAL_SUBSCRIPTION_PROFILE_ID 未设置，跳过真实订阅查询");
            return;
        };
        // main.rs installs the provider at startup; the test binary has to do it itself.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let before = find_store_account(&profile_id).expect("account exists");
        eprintln!(
            "[real-subscription] before: claim active_until={:?} stored={}",
            claim_active_until(&before),
            before["custom"]["subscription"]
        );

        refresh_account_subscription(&profile_id, 30_000);

        let after = find_store_account(&profile_id).expect("account still exists");
        let stored = after["custom"]["subscription"].clone();
        eprintln!("[real-subscription] after: stored={stored}");
        assert!(
            !stored.is_null(),
            "subscription snapshot must be stored for {profile_id}"
        );
        assert!(
            !string_field(&stored, "active_until").is_empty(),
            "stored snapshot must carry active_until"
        );
    }
}
