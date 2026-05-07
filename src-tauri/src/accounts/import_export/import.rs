use crate::{
    accounts::{
        account_from_exchange, exchange_refresh_token, import_store_accounts,
        store_payload_from_store, ImportTokenResult,
    },
    json_util::string_field,
};
use serde_json::Value;
use std::thread;

pub(crate) fn import_one_refresh_token(refresh_token: String) -> ImportTokenResult {
    match exchange_refresh_token(&refresh_token) {
        Ok(exchange) => {
            let account_id = string_field(&exchange, "account_id");
            let access_token = string_field(&exchange, "access_token");
            let usage_result = crate::accounts::get_usage(&access_token, &account_id, 30_000);
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
