//! Keeps `custom.subscription` in sync with `/backend-api/subscriptions`.
//!
//! The renewal date shown on an account card used to come from the id_token claim
//! `chatgpt_subscription_active_until`, which OpenAI does not always refresh. Every quota
//! refresh therefore also reads the subscription endpoint; the claim stays as the fallback
//! for accounts whose endpoint read has never succeeded.

use crate::app_log::account_label;
use crate::{
    accounts::{
        access_token_from_account, account_id_from_account, account_with_custom,
        find_store_account, get_subscription, set_subscription_state, update_store_account,
    },
    app_log::log_event,
    json_util::{raw_string_field, string_field, value_u64_field},
};
use serde_json::{json, Value};

fn log_subscription_error(profile_id: &str, error: &Value) {
    log_event(
        "account_subscription_refresh_error",
        json!({
            "account": account_label(profile_id),
            "code": raw_string_field(error, "code"),
            "status": value_u64_field(error, "status"),
            "message": raw_string_field(error, "message"),
            "rawMessage": raw_string_field(error, "raw_message"),
            "handling": "keep_previous_snapshot"
        }),
    );
}

/// `fetched_at` is stamped locally on every read (the endpoint does not send one), so it is
/// left out of the comparison; otherwise every refresh would count as a change.
fn same_subscription_snapshot(previous: &Value, next: &Value) -> bool {
    let without_fetched_at = |value: &Value| {
        let mut value = value.clone();
        if let Some(object) = value.as_object_mut() {
            object.remove("fetched_at");
        }
        value
    };
    without_fetched_at(previous) == without_fetched_at(next)
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
            log_event(
                "account_subscription_account_read_error",
                json!({
                    "account": account_label(profile_id),
                    "error": err,
                    "handling": "skip_subscription_refresh"
                }),
            );
            return None;
        }
    };
    let account_id = account_id_from_account(&account).unwrap_or_default();
    let access_token = access_token_from_account(&account);
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

    let mut changed = false;
    let update = update_store_account(profile_id, |latest| {
        let previous = latest
            .get("custom")
            .and_then(|custom| custom.get("subscription"))
            .unwrap_or(&Value::Null);
        if same_subscription_snapshot(previous, &subscription) {
            return Ok(latest.clone());
        }
        changed = true;
        let custom = set_subscription_state(latest.get("custom"), subscription.clone());
        Ok(account_with_custom(latest, custom))
    });
    match update {
        Ok(_) if !changed => None,
        Ok(store) => {
            log_event(
                "account_subscription_updated",
                json!({
                    "account": account_label(profile_id),
                    "activeUntil": string_field(&subscription, "active_until"),
                    "willRenew": subscription["will_renew"],
                    "isDelinquent": subscription["is_delinquent"]
                }),
            );
            Some(store)
        }
        Err(err) => {
            log_event(
                "account_subscription_store_error",
                json!({ "account": account_label(profile_id), "error": err }),
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn snapshot_comparison_ignores_the_local_fetch_time() {
        let stored = json!({
            "active_until": "2026-10-10T13:30:31Z",
            "plan_type": "pro",
            "will_renew": true,
            "is_delinquent": false,
            "fetched_at": "2026-09-14T07:00:00Z"
        });
        let mut refetched = stored.clone();
        refetched["fetched_at"] = json!("2026-09-23T07:00:00Z");
        let mut renewed = refetched.clone();
        renewed["active_until"] = json!("2026-11-10T13:30:31Z");

        assert!(same_subscription_snapshot(&stored, &refetched));
        assert!(!same_subscription_snapshot(&stored, &renewed));
        assert!(!same_subscription_snapshot(&Value::Null, &refetched));
    }
    use crate::accounts::{decode_jwt_payload, BACKGROUND_REQUEST_TIMEOUT_MS};
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

        refresh_account_subscription(&profile_id, BACKGROUND_REQUEST_TIMEOUT_MS);

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
