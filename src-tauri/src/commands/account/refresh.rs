use super::*;

const MANUAL_QUOTA_TIMEOUT_MS: u64 = 10_000;

fn usage_error_message(error: &Value, fallback: &str) -> String {
    raw_string_field(error, "message")
        .chars()
        .next()
        .map(|_| raw_string_field(error, "message"))
        .unwrap_or_else(|| fallback.to_string())
}

pub(super) struct AccountRefreshContext {
    pub(super) account: Value,
    pub(super) exchange: Value,
    pub(super) profile_id: String,
    pub(super) account_id: String,
    pub(super) previous_refresh_token: String,
}

pub(super) enum AccountRefreshStart {
    Ready(AccountRefreshContext),
    Failed(Value),
}

fn prepare_account_refresh(id: String) -> Result<AccountRefreshStart, String> {
    let target_profile_id = id.trim();
    if target_profile_id.is_empty() {
        return Err("account_id 无效".to_string());
    }
    let account = find_store_account(target_profile_id)?;
    let expected_account_id = account_id_from_account(&account)?;
    let previous_refresh_token = account
        .get("tokens")
        .and_then(|tokens| tokens.get("refresh_token"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let exchange = match exchange_refresh_token(&previous_refresh_token) {
        Ok(value) => value,
        Err(err) => {
            return Ok(AccountRefreshStart::Failed(auth_error_payload(
                target_profile_id,
                &err,
            )?))
        }
    };
    let account_id = string_field(&exchange, "account_id");
    if account_id != expected_account_id {
        let message = "刷新后账号标识不一致";
        return Ok(AccountRefreshStart::Failed(auth_error_payload(
            target_profile_id,
            message,
        )?));
    }

    Ok(AccountRefreshStart::Ready(AccountRefreshContext {
        account,
        exchange,
        profile_id: target_profile_id.to_string(),
        account_id,
        previous_refresh_token,
    }))
}

fn auth_error_payload(profile_id: &str, message: &str) -> Result<Value, String> {
    let store = mark_account_auth_error(profile_id, message)?;
    Ok(json!({
        "ok": false,
        "message": message,
        "code": "",
        "store": store
    }))
}

/// Manual refresh re-issues the tokens first: the plan badge and subscription expiry
/// come from id_token claims that only change on a refresh_token grant, so refreshing
/// quota with the old access_token would keep showing the pre-renewal subscription.
pub(super) fn refresh_account_impl(id: String) -> Result<Value, String> {
    let target_profile_id = id.trim();
    if target_profile_id.is_empty() {
        return Err("account_id 无效".to_string());
    }
    refresh_account_with_token_refresh(target_profile_id.to_string())
}

fn refresh_account_with_token_refresh(id: String) -> Result<Value, String> {
    let context = match prepare_account_refresh(id)? {
        AccountRefreshStart::Ready(context) => context,
        AccountRefreshStart::Failed(payload) => return Ok(payload),
    };

    let old_usage = context
        .account
        .get("custom")
        .and_then(|custom| custom.get("usage_info"))
        .cloned()
        .unwrap_or(Value::Null);
    let usage_result = get_usage(
        &string_field(&context.exchange, "access_token"),
        &context.account_id,
        MANUAL_QUOTA_TIMEOUT_MS,
    );
    let usage_error = usage_result.as_ref().err().cloned();
    let next_account = account_from_exchange(
        &context.exchange,
        context.account.get("custom"),
        usage_result,
    )?;
    let store = add_account_to_store(next_account, false)?;
    sync_auth_file_if_active(&context.profile_id)?;

    if let Some(error) = usage_error {
        let message = usage_error_message(&error, "Usage refresh failed");
        return Ok(json!({
            "ok": false,
            "message": format!("Subscription refreshed, but quota refresh failed\n{message}"),
            "code": raw_string_field(&error, "code"),
            "store": store
        }));
    }

    let new_account = find_store_account(&context.profile_id)?;
    let new_usage = new_account
        .get("custom")
        .and_then(|custom| custom.get("usage_info"))
        .cloned()
        .unwrap_or(Value::Null);
    let refresh_token_changed =
        string_field(&context.exchange, "refresh_token") != context.previous_refresh_token;
    let usage_changed = old_usage != new_usage;
    Ok(json!({
        "ok": true,
        "message": if refresh_token_changed || usage_changed {
            "账号信息已刷新（订阅与配额已更新）"
        } else {
            "账号信息已刷新"
        },
        "store": store
    }))
}

pub(super) fn refresh_account_token_impl(id: String) -> Result<Value, String> {
    let context = match prepare_account_refresh(id)? {
        AccountRefreshStart::Ready(context) => context,
        AccountRefreshStart::Failed(payload) => return Ok(payload),
    };
    let next_account =
        account_from_exchange_preserve_usage(&context.exchange, context.account.get("custom"))?;
    let store = add_account_to_store(next_account, false)?;
    sync_auth_file_if_active(&context.profile_id)?;
    Ok(json!({
        "ok": true,
        "message": "Refresh Token 已刷新",
        "refresh_token": string_field(&context.exchange, "refresh_token"),
        "store": store
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quota::subscription_claims::subscription_claims_summary;
    use std::{env, thread, time::Duration};

    fn usage_summary(account: &Value) -> String {
        let usage = account
            .get("custom")
            .and_then(|custom| custom.get("usage_info"))
            .cloned()
            .unwrap_or(Value::Null);
        format!(
            "usage plan_type={:?} reset_credits={} fetched_at={:?}",
            string_field(&usage, "plan_type"),
            usage.get("reset_credits").cloned().unwrap_or(Value::Null),
            string_field(&usage, "fetched_at")
        )
    }

    fn access_token(account: &Value) -> String {
        account
            .get("tokens")
            .and_then(|tokens| tokens.get("access_token"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    /// Real-environment check for the manual refresh path. It re-issues the tokens of
    /// the live account named by CODEX_SWITCH_REAL_REFRESH_PROFILE_ID (same effect as
    /// clicking 刷新配额) and prints the id_token subscription claims before and after,
    /// then re-reads accounts.json after a delay to prove the rotated tokens persisted.
    #[test]
    #[ignore = "rotates the refresh_token of a live account; set CODEX_SWITCH_REAL_REFRESH_PROFILE_ID"]
    fn real_manual_refresh_reissues_subscription_claims() {
        let Ok(profile_id) = env::var("CODEX_SWITCH_REAL_REFRESH_PROFILE_ID") else {
            eprintln!("CODEX_SWITCH_REAL_REFRESH_PROFILE_ID 未设置，跳过真实刷新");
            return;
        };
        // main.rs installs the provider at startup; the test binary has to do it itself.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let before = find_store_account(&profile_id).expect("account exists");
        eprintln!(
            "[real-refresh] before: {}",
            subscription_claims_summary(&before)
        );
        eprintln!("[real-refresh] before: {}", usage_summary(&before));

        let result = refresh_account_impl(profile_id.clone()).expect("refresh command runs");
        eprintln!(
            "[real-refresh] result ok={} message={:?}",
            result["ok"], result["message"]
        );
        assert_eq!(result["ok"], Value::Bool(true), "{result}");

        thread::sleep(Duration::from_secs(3));
        let after = find_store_account(&profile_id).expect("account still exists");
        eprintln!(
            "[real-refresh] after: {}",
            subscription_claims_summary(&after)
        );
        eprintln!("[real-refresh] after: {}", usage_summary(&after));
        assert_ne!(
            access_token(&before),
            access_token(&after),
            "rotated access_token must persist in accounts.json"
        );
    }
}
