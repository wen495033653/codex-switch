use super::*;

pub(crate) fn account_from_exchange(
    exchange: &Value,
    previous_custom: Option<&Value>,
    usage_result: Result<Value, Value>,
) -> Result<Value, String> {
    let account_id = string_field(exchange, "account_id");
    if account_id.is_empty() {
        return Err("刷新结果缺少 account_id".to_string());
    }

    let tokens = json!({
        "id_token": string_field(exchange, "id_token"),
        "access_token": string_field(exchange, "access_token"),
        "refresh_token": string_field(exchange, "refresh_token"),
        "account_id": account_id
    });
    normalize_tokens(Some(&tokens))?;

    let mut custom = normalize_custom(previous_custom);
    if raw_string_field(&custom, "created_at").is_empty() {
        custom["created_at"] = Value::String(now_string());
    }
    if raw_string_field(&custom, "last_used_at").is_empty() {
        custom["last_used_at"] = custom
            .get("created_at")
            .cloned()
            .unwrap_or_else(|| Value::String(now_string()));
    }
    custom = set_auth_state(
        Some(&custom),
        "active",
        "",
        Value::Null,
        Some(&string_field(exchange, "last_refresh_at")),
        Some(&string_field(exchange, "expires_at")),
    );
    custom = match usage_result {
        Ok(usage_info) => set_usage_state(Some(&custom), "ok", "", Some(usage_info), Value::Null),
        Err(error) => {
            let message = raw_string_field(&error, "message")
                .chars()
                .next()
                .map(|_| raw_string_field(&error, "message"))
                .unwrap_or_else(|| "配额刷新失败，请稍后手动刷新".to_string());
            set_usage_state(Some(&custom), "error", &message, None, error)
        }
    };

    Ok(json!({
        "tokens": tokens,
        "custom": custom
    }))
}

pub(crate) fn account_from_exchange_syncing(
    exchange: &Value,
    previous_custom: Option<&Value>,
) -> Result<Value, String> {
    let account_id = string_field(exchange, "account_id");
    if account_id.is_empty() {
        return Err("刷新结果缺少 account_id".to_string());
    }

    let tokens = json!({
        "id_token": string_field(exchange, "id_token"),
        "access_token": string_field(exchange, "access_token"),
        "refresh_token": string_field(exchange, "refresh_token"),
        "account_id": account_id
    });
    normalize_tokens(Some(&tokens))?;

    let mut custom = normalize_custom(previous_custom);
    if raw_string_field(&custom, "created_at").is_empty() {
        custom["created_at"] = Value::String(now_string());
    }
    if raw_string_field(&custom, "last_used_at").is_empty() {
        custom["last_used_at"] = custom
            .get("created_at")
            .cloned()
            .unwrap_or_else(|| Value::String(now_string()));
    }
    custom = set_auth_state(
        Some(&custom),
        "active",
        "",
        Value::Null,
        Some(&string_field(exchange, "last_refresh_at")),
        Some(&string_field(exchange, "expires_at")),
    );
    custom = set_usage_state(
        Some(&custom),
        "syncing",
        "配额同步中，请稍候...",
        None,
        Value::Null,
    );

    Ok(json!({
        "tokens": tokens,
        "custom": custom
    }))
}

pub(crate) fn account_from_exchange_preserve_usage(
    exchange: &Value,
    previous_custom: Option<&Value>,
) -> Result<Value, String> {
    let account_id = string_field(exchange, "account_id");
    if account_id.is_empty() {
        return Err("刷新结果缺少 account_id".to_string());
    }
    let tokens = json!({
        "id_token": string_field(exchange, "id_token"),
        "access_token": string_field(exchange, "access_token"),
        "refresh_token": string_field(exchange, "refresh_token"),
        "account_id": account_id
    });
    normalize_tokens(Some(&tokens))?;
    let custom = set_auth_state(
        previous_custom,
        "active",
        "",
        Value::Null,
        Some(&string_field(exchange, "last_refresh_at")),
        Some(&string_field(exchange, "expires_at")),
    );
    Ok(json!({
        "tokens": tokens,
        "custom": custom
    }))
}
