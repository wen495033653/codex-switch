use super::store::normalize_tokens;
use crate::json_file::{read_json_file, write_json_file};
use crate::{paths::auth_path, time_util::now_string};
use serde_json::{json, Value};

pub(crate) fn read_auth_value() -> Result<Value, String> {
    let path = auth_path()?;
    if !path.exists() {
        return Err(format!("auth.json 不存在: {}", path.display()));
    }
    read_json_file(&path, "auth.json")
}

/// Reads auth.json; `Ok(None)` when Codex has not written one yet, which is a normal state.
pub(super) fn read_auth_value_if_exists() -> Result<Option<Value>, String> {
    let path = auth_path()?;
    if !path.exists() {
        return Ok(None);
    }
    read_json_file(&path, "auth.json").map(Some)
}

pub(crate) fn write_auth_value(auth: &Value) -> Result<(), String> {
    let path = auth_path()?;
    let mut data = auth.clone();
    if let Some(object) = data.as_object_mut() {
        object.remove("usage_info");
    }
    write_json_file(&path, "auth.json", &data)
}

pub(crate) fn write_api_auth(api_key: &str) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("API Key 不能为空".to_string());
    }
    write_auth_value(&json!({
        "auth_mode": "apikey",
        "OPENAI_API_KEY": key
    }))
}

pub(crate) fn write_account_auth(account: &Value) -> Result<(), String> {
    let tokens = account
        .get("tokens")
        .cloned()
        .ok_or_else(|| "账号缺少 tokens".to_string())?;
    write_auth_value(&json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": Value::Null,
        "tokens": tokens,
        "last_refresh": now_string()
    }))
}

pub(crate) fn auth_to_account(auth: &Value) -> Result<Value, String> {
    let tokens = normalize_tokens(auth.get("tokens"))?;
    Ok(json!({
        "tokens": tokens,
        "custom": {
            "created_at": "",
            "last_used_at": "",
            "usage_info": auth.get("usage_info").cloned().unwrap_or(Value::Null)
        }
    }))
}
