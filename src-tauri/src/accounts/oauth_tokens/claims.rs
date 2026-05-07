use base64::{engine::general_purpose, Engine as _};
use serde_json::Value;
use time::{format_description::well_known::Rfc3339, Duration as TimeDuration, OffsetDateTime};

pub(super) fn number_i64(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|item| i64::try_from(item).ok()))
            .or_else(|| number.as_f64().map(|item| item as i64)),
        Some(Value::String(text)) => text.parse::<i64>().ok(),
        _ => None,
    }
}

pub(super) fn format_unix_timestamp(timestamp: i64) -> String {
    OffsetDateTime::from_unix_timestamp(timestamp)
        .ok()
        .and_then(|time| time.format(&Rfc3339).ok())
        .unwrap_or_default()
}

pub(crate) fn decode_jwt_payload(token: &str) -> Result<Value, String> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or_else(|| "id_token 格式无效".to_string())?;
    let bytes = general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| general_purpose::URL_SAFE.decode(payload))
        .map_err(|err| format!("id_token payload 解码失败: {err}"))?;
    serde_json::from_slice(&bytes).map_err(|err| format!("id_token payload 解析失败: {err}"))
}

pub(super) fn account_id_from_claims(claims: &Value) -> Result<String, String> {
    let account_id = claims
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if account_id.is_empty() {
        return Err("JWT 缺少 chatgpt_account_id".to_string());
    }
    Ok(account_id)
}

pub(super) fn resolve_auth_expires_at(claims: &Value, expires_in_seconds: Option<i64>) -> String {
    if let Some(expires_in) = expires_in_seconds.filter(|value| *value > 0) {
        return (OffsetDateTime::now_utc() + TimeDuration::seconds(expires_in))
            .format(&Rfc3339)
            .unwrap_or_default();
    }

    number_i64(claims.get("exp"))
        .filter(|value| *value > 0)
        .map(format_unix_timestamp)
        .unwrap_or_default()
}
