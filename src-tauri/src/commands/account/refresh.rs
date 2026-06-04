use super::*;
use crate::json_util::value_u64_field;

const MANUAL_QUOTA_TIMEOUT_MS: u64 = 10_000;

fn is_auth_retryable_usage_error(error: &Value) -> bool {
    matches!(value_u64_field(error, "status"), Some(401 | 403))
}

fn usage_error_message(error: &Value, fallback: &str) -> String {
    raw_string_field(error, "message")
        .chars()
        .next()
        .map(|_| raw_string_field(error, "message"))
        .unwrap_or_else(|| fallback.to_string())
}

fn account_with_usage_result(account: &Value, usage_result: Result<Value, Value>) -> Value {
    let tokens = account.get("tokens").cloned().unwrap_or(Value::Null);
    let custom = match usage_result {
        Ok(usage_info) => set_usage_state(
            account.get("custom"),
            "ok",
            "",
            Some(usage_info),
            Value::Null,
        ),
        Err(error) => set_usage_state(
            account.get("custom"),
            "error",
            &usage_error_message(&error, "Usage refresh failed, please refresh manually"),
            None,
            error,
        ),
    };
    json!({
        "tokens": tokens,
        "custom": custom
    })
}

fn update_account_usage_preserve_tokens(
    account: &Value,
    usage_result: Result<Value, Value>,
) -> Result<Value, String> {
    add_account_to_store(account_with_usage_result(account, usage_result), false)
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

pub(super) fn refresh_account_impl(id: String) -> Result<Value, String> {
    let target_profile_id = id.trim();
    if target_profile_id.is_empty() {
        return Err("account_id 无效".to_string());
    }

    let account = find_store_account(target_profile_id)?;
    let account_id = account_id_from_account(&account)?;
    let access_token = account
        .get("tokens")
        .and_then(|tokens| tokens.get("access_token"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    if !access_token.is_empty() {
        match get_usage(&access_token, &account_id, MANUAL_QUOTA_TIMEOUT_MS) {
            Ok(usage_info) => {
                let store = update_account_usage_preserve_tokens(&account, Ok(usage_info))?;
                return Ok(json!({
                    "ok": true,
                    "message": "配额已刷新",
                    "store": store
                }));
            }
            Err(error) if !is_auth_retryable_usage_error(&error) => {
                let message = usage_error_message(&error, "Usage refresh failed");
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
