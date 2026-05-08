use super::*;

const MANUAL_QUOTA_TIMEOUT_MS: u64 = 10_000;

pub(super) struct AccountRefreshContext {
    pub(super) account: Value,
    pub(super) exchange: Value,
    pub(super) account_id: String,
    pub(super) previous_refresh_token: String,
}

pub(super) enum AccountRefreshStart {
    Ready(AccountRefreshContext),
    Failed(Value),
}

fn prepare_account_refresh(id: String) -> Result<AccountRefreshStart, String> {
    let target_id = id.trim();
    if target_id.is_empty() {
        return Err("account_id 无效".to_string());
    }
    let account = find_store_account(target_id)?;
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
                target_id, &err,
            )?))
        }
    };
    let account_id = string_field(&exchange, "account_id");
    if account_id != target_id {
        let message = "刷新后账号标识不一致";
        return Ok(AccountRefreshStart::Failed(auth_error_payload(
            target_id, message,
        )?));
    }

    Ok(AccountRefreshStart::Ready(AccountRefreshContext {
        account,
        exchange,
        account_id,
        previous_refresh_token,
    }))
}

fn auth_error_payload(target_id: &str, message: &str) -> Result<Value, String> {
    let store = mark_account_auth_error(target_id, message)?;
    Ok(json!({
        "ok": false,
        "message": message,
        "code": "",
        "store": store
    }))
}

pub(super) fn refresh_account_impl(id: String) -> Result<Value, String> {
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
    sync_auth_file_if_active(&context.account_id)?;

    if let Some(error) = usage_error {
        let message = raw_string_field(&error, "message")
            .chars()
            .next()
            .map(|_| raw_string_field(&error, "message"))
            .unwrap_or_else(|| "Usage refresh failed".to_string());
        return Ok(json!({
            "ok": false,
            "message": format!("Subscription refreshed, but quota refresh failed\n{message}"),
            "code": raw_string_field(&error, "code"),
            "store": store
        }));
    }

    let new_account = find_store_account(&context.account_id)?;
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
    sync_auth_file_if_active(&context.account_id)?;
    Ok(json!({
        "ok": true,
        "message": "Refresh Token 已刷新",
        "refresh_token": string_field(&context.exchange, "refresh_token"),
        "store": store
    }))
}
