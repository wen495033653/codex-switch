use super::*;
use crate::json_util::non_empty_string_field;

fn usage_error_message(error: &Value) -> String {
    non_empty_string_field(error, "message").unwrap_or_else(|| "Usage refresh failed".to_string())
}

fn update_account_usage_preserve_tokens(
    account: &Value,
    usage_result: Result<Value, Value>,
) -> Result<Value, String> {
    let custom = set_usage_result(account.get("custom"), usage_result);
    add_account_to_store(account_with_custom(account, custom), false)
}

/// Subscription renewal data lives behind a second endpoint, so every successful quota
/// refresh also re-reads it. A failed subscription read keeps the stored snapshot and the
/// quota result, which is why the previous store value is the fallback here.
fn store_with_refreshed_subscription(profile_id: &str, store: Value) -> Value {
    refresh_account_subscription(profile_id, INTERACTIVE_REQUEST_TIMEOUT_MS).unwrap_or(store)
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
    let previous_refresh_token = refresh_token_from_account(&account);
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

pub(super) fn refresh_account_impl(id: String) -> Result<Value, String> {
    let target_profile_id = id.trim();
    if target_profile_id.is_empty() {
        return Err("account_id 无效".to_string());
    }

    let account = find_store_account(target_profile_id)?;
    let account_id = account_id_from_account(&account)?;
    let access_token = access_token_from_account(&account);

    if !access_token.is_empty() {
        match get_usage(&access_token, &account_id, INTERACTIVE_REQUEST_TIMEOUT_MS) {
            Ok(usage_info) => {
                let store = update_account_usage_preserve_tokens(&account, Ok(usage_info))?;
                let store = store_with_refreshed_subscription(target_profile_id, store);
                return Ok(json!({
                    "ok": true,
                    "message": "配额已刷新",
                    "store": store
                }));
            }
            Err(error) if !error_state_is_auth_rejected(&error) => {
                let message = usage_error_message(&error);
                let code = raw_string_field(&error, "code");
                let store = update_account_usage_preserve_tokens(&account, Err(error))?;
                return Ok(json!({
                    "ok": false,
                    "message": format!("配额刷新失败\n{message}"),
                    "code": code,
                    "store": store
                }));
            }
            Err(_) => {}
        }
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
        INTERACTIVE_REQUEST_TIMEOUT_MS,
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
        let message = usage_error_message(&error);
        return Ok(json!({
            "ok": false,
            "message": format!("Subscription refreshed, but quota refresh failed\n{message}"),
            "code": raw_string_field(&error, "code"),
            "store": store
        }));
    }

    let store = store_with_refreshed_subscription(&context.profile_id, store);
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
    use std::{env, thread, time::Duration};

    fn claims_summary(account: &Value) -> String {
        let auth = account
            .get("tokens")
            .and_then(|tokens| tokens.get("id_token"))
            .and_then(Value::as_str)
            .and_then(|id_token| decode_jwt_payload(id_token).ok())
            .and_then(|claims| claims.get("https://api.openai.com/auth").cloned())
            .unwrap_or(Value::Null);
        format!(
            "plan_type={:?} active_until={:?} last_checked={:?}",
            string_field(&auth, "chatgpt_plan_type"),
            string_field(&auth, "chatgpt_subscription_active_until"),
            string_field(&auth, "chatgpt_subscription_last_checked")
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

    /// Real-environment check for the explicit token re-issue path (查看 Refresh Token ->
    /// 刷新 Refresh Token). It rotates the tokens of the live account named by
    /// CODEX_SWITCH_REAL_REFRESH_PROFILE_ID and prints the id_token subscription claims
    /// before and after, then re-reads accounts.json after a delay to prove the rotated
    /// tokens persisted. Observed on 2026-09-14 for two pro accounts: the claims did not
    /// change, so a refresh_token grant does not fetch a new subscription expiry.
    #[test]
    #[ignore = "rotates the refresh_token of a live account; set CODEX_SWITCH_REAL_REFRESH_PROFILE_ID"]
    fn real_token_reissue_keeps_or_updates_subscription_claims() {
        let Ok(profile_id) = env::var("CODEX_SWITCH_REAL_REFRESH_PROFILE_ID") else {
            eprintln!("CODEX_SWITCH_REAL_REFRESH_PROFILE_ID 未设置，跳过真实重签");
            return;
        };
        // main.rs installs the provider at startup; the test binary has to do it itself.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let before = find_store_account(&profile_id).expect("account exists");
        eprintln!("[real-reissue] before: {}", claims_summary(&before));

        let result = refresh_account_token_impl(profile_id.clone()).expect("command runs");
        eprintln!(
            "[real-reissue] result ok={} message={:?}",
            result["ok"], result["message"]
        );
        assert_eq!(result["ok"], Value::Bool(true), "{result}");

        thread::sleep(Duration::from_secs(3));
        let after = find_store_account(&profile_id).expect("account still exists");
        eprintln!("[real-reissue] after: {}", claims_summary(&after));
        assert_ne!(
            access_token(&before),
            access_token(&after),
            "rotated access_token must persist in accounts.json"
        );
    }
}
