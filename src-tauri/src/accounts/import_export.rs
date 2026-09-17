use super::{
    account_builders::account_from_exchange,
    oauth_tokens::{decode_jwt_payload, exchange_refresh_token},
    store::{import_store_accounts, store_payload_from_store},
    usage::BACKGROUND_REQUEST_TIMEOUT_MS,
};
use crate::{
    accounts::ImportTokenResult,
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

fn import_one_refresh_token(refresh_token: String) -> ImportTokenResult {
    match exchange_refresh_token(&refresh_token) {
        Ok(exchange) => {
            let account_id = string_field(&exchange, "account_id");
            let access_token = string_field(&exchange, "access_token");
            let usage_result = crate::accounts::get_usage(
                &access_token,
                &account_id,
                BACKGROUND_REQUEST_TIMEOUT_MS,
            );
            let usage_ok = usage_result.is_ok();
            match account_from_exchange(&exchange, None, usage_result) {
                Ok(account) => ImportTokenResult {
                    account: Some(account),
                    usage_ok,
                },
                Err(_error) => ImportTokenResult {
                    account: None,
                    usage_ok: false,
                },
            }
        }
        Err(_error) => ImportTokenResult {
            account: None,
            usage_ok: false,
        },
    }
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

    for handle in handles {
        results.push(handle.join().unwrap_or(ImportTokenResult {
            account: None,
            usage_ok: false,
        }));
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
        return Err("导入失败：refresh_token 全部不可用".to_string());
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
        message.push_str(&format!("，token 失效 {failed_count} 个"));
    }
    if usage_failed_count > 0 {
        message.push_str(&format!("，配额刷新失败 {usage_failed_count} 个"));
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
