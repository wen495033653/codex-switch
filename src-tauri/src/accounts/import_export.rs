use super::{
    account_builders::account_from_exchange,
    oauth_tokens::{decode_jwt_payload, exchange_refresh_token},
    store::{import_store_accounts, store_payload_from_store},
    usage::BACKGROUND_REQUEST_TIMEOUT_MS,
};
use crate::{
    accounts::ImportTokenResult,
    app_log::log_event,
    json_util::{raw_string_field, string_field},
};
use serde_json::{json, Value};
use std::{collections::HashSet, thread};
use time::OffsetDateTime;

pub(crate) fn local_date_for_filename() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        u8::from(now.month()),
        now.day()
    )
}

pub(crate) fn build_export_account_item(account: &Value) -> Option<Value> {
    let tokens = account.get("tokens").unwrap_or(&Value::Null);
    let refresh_token = raw_string_field(tokens, "refresh_token");
    if refresh_token.is_empty() {
        return None;
    }

    let email = decode_jwt_payload(&raw_string_field(tokens, "id_token"))
        .ok()
        .map(|claims| string_field(&claims, "email"))
        .unwrap_or_default();

    Some(json!({
        "email": email,
        "account_id": raw_string_field(tokens, "account_id"),
        "refresh_token": refresh_token
    }))
}

fn failed_import(error: String) -> ImportTokenResult {
    ImportTokenResult {
        account: None,
        usage_ok: false,
        error: Some(error),
    }
}

fn import_one_refresh_token(refresh_token: String) -> ImportTokenResult {
    let exchange = match exchange_refresh_token(&refresh_token) {
        Ok(exchange) => exchange,
        Err(error) => return failed_import(error),
    };
    let account_id = string_field(&exchange, "account_id");
    let access_token = string_field(&exchange, "access_token");
    let usage_result =
        crate::accounts::get_usage(&access_token, &account_id, BACKGROUND_REQUEST_TIMEOUT_MS);
    let usage_ok = usage_result.is_ok();
    match account_from_exchange(&exchange, None, usage_result) {
        Ok(account) => ImportTokenResult {
            account: Some(account),
            usage_ok,
            error: None,
        },
        Err(error) => failed_import(error),
    }
}

/// Groups failure reasons for the result message, most frequent first, so the user can
/// tell an expired token from a network or rate-limit failure.
fn failure_summary(results: &[ImportTokenResult]) -> String {
    let mut reasons: Vec<(String, usize)> = Vec::new();
    for error in results.iter().filter_map(|result| result.error.as_deref()) {
        let reason = error.lines().collect::<Vec<_>>().join(" / ");
        match reasons.iter_mut().find(|(known, _)| *known == reason) {
            Some((_, count)) => *count += 1,
            None => reasons.push((reason, 1)),
        }
    }
    reasons.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    reasons
        .iter()
        .take(3)
        .map(|(reason, count)| format!("{reason}（{count} 个）"))
        .collect::<Vec<_>>()
        .join("；")
}

pub(crate) fn import_accounts_from_refresh_tokens(
    refresh_tokens: Vec<String>,
    overwrite: bool,
) -> Result<Value, String> {
    let handles: Vec<_> = refresh_tokens
        .into_iter()
        .map(|refresh_token| thread::spawn(move || import_one_refresh_token(refresh_token)))
        .collect();
    let mut results = Vec::new();

    for (index, handle) in handles.into_iter().enumerate() {
        let result = handle
            .join()
            .unwrap_or_else(|_| failed_import("导入线程异常退出".to_string()));
        if let Some(error) = &result.error {
            // The position in the import file identifies the entry; the token is never logged.
            log_event(
                "account_import_token_error",
                json!({ "index": index, "error": error }),
            );
        }
        results.push(result);
    }

    let imported_count = results
        .iter()
        .filter(|result| result.account.is_some())
        .count();
    let accounts: Vec<Value> = results
        .iter()
        .filter_map(|result| result.account.clone())
        .collect();
    if accounts.is_empty() {
        return Err(format!(
            "导入失败：refresh_token 全部不可用\n{}",
            failure_summary(&results)
        ));
    }

    let failed_count = results
        .iter()
        .filter(|result| result.account.is_none())
        .count();
    let usage_failed_count = results
        .iter()
        .filter(|result| result.account.is_some() && !result.usage_ok)
        .count();
    let store = import_store_accounts(accounts, overwrite)?;

    let mut message = format!("导入成功 {imported_count} 个账号");
    if failed_count > 0 {
        message.push_str(&format!("，导入失败 {failed_count} 个"));
    }
    if usage_failed_count > 0 {
        message.push_str(&format!("，配额刷新失败 {usage_failed_count} 个"));
    }
    if failed_count > 0 {
        message.push_str(&format!("\n失败原因：{}", failure_summary(&results)));
    }

    Ok(store_payload_from_store(store, Some(&message)))
}

fn push_refresh_token_from_value(out: &mut Vec<String>, value: &Value) {
    if let Some(text) = value.as_str() {
        let token = text.trim();
        if !token.is_empty() {
            out.push(token.to_string());
        }
        return;
    }

    let token = string_field(value, "refresh_token");
    if !token.is_empty() {
        out.push(token);
    }
}

pub(crate) fn extract_refresh_tokens_from_data(data: &Value) -> Vec<String> {
    let mut raw_tokens = Vec::new();

    if let Some(items) = data.as_array() {
        for item in items {
            push_refresh_token_from_value(&mut raw_tokens, item);
        }
    } else if data.is_object() {
        if let Some(items) = data.get("refresh_tokens").and_then(Value::as_array) {
            for item in items {
                push_refresh_token_from_value(&mut raw_tokens, item);
            }
        }
        if let Some(items) = data.get("tokens").and_then(Value::as_array) {
            for item in items {
                push_refresh_token_from_value(&mut raw_tokens, item);
            }
        }
        if let Some(items) = data.get("accounts").and_then(Value::as_array) {
            for item in items {
                if let Some(tokens) = item.get("tokens") {
                    push_refresh_token_from_value(&mut raw_tokens, tokens);
                } else {
                    push_refresh_token_from_value(&mut raw_tokens, item);
                }
            }
        }
    }

    let mut seen = HashSet::new();
    raw_tokens
        .into_iter()
        .filter_map(|token| {
            let token = token.trim().to_string();
            if token.is_empty() || !seen.insert(token.clone()) {
                return None;
            }
            Some(token)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_summary_groups_reasons_by_frequency() {
        let results = vec![
            failed_import("Refresh Token 刷新失败\nHTTP 429".to_string()),
            failed_import(
                "Refresh Token 刷新失败\nHTTP 401\nerror.code: invalid_grant".to_string(),
            ),
            failed_import("Refresh Token 刷新失败\nHTTP 429".to_string()),
            ImportTokenResult {
                account: Some(json!({})),
                usage_ok: true,
                error: None,
            },
        ];

        assert_eq!(
            failure_summary(&results),
            "Refresh Token 刷新失败 / HTTP 429（2 个）；Refresh Token 刷新失败 / HTTP 401 / error.code: invalid_grant（1 个）"
        );
    }
}
