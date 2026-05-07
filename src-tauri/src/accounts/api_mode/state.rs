use super::{profile::read_api_key_from_auth, *};

pub(crate) fn get_codex_state_value() -> Value {
    let auth = read_auth_value().unwrap_or_else(|_| json!({}));
    let root_config = read_root_config().unwrap_or_default();
    let auth_mode = raw_string_field(&auth, "auth_mode");
    let preferred_auth_method = root_config
        .get("preferred_auth_method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let forced_login_method = root_config
        .get("forced_login_method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let model_provider = root_config
        .get("model_provider")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let provider_config = if model_provider.is_empty() {
        Map::new()
    } else {
        read_table_config(&format!("model_providers.{model_provider}")).unwrap_or_default()
    };
    let provider_name = provider_config
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let provider_base_url = provider_config
        .get("base_url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let wire_api = provider_config
        .get("wire_api")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let supports_websockets = provider_config
        .get("supports_websockets")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let openai_base_url = root_config
        .get("openai_base_url")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(&provider_base_url)
        .to_string();
    let api_key_present = !read_api_key_from_auth().is_empty();
    let account_id = auth
        .get("tokens")
        .and_then(|tokens| tokens.get("account_id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let mode = if auth_mode == "apikey"
        || api_key_present
        || preferred_auth_method == "api"
        || forced_login_method == "api"
    {
        "api"
    } else if auth_mode == "chatgpt" || !account_id.is_empty() {
        "chatgpt"
    } else {
        "unknown"
    };

    json!({
        "mode": mode,
        "auth_mode": auth_mode,
        "preferred_auth_method": preferred_auth_method,
        "forced_login_method": forced_login_method,
        "model_provider": model_provider,
        "provider_name": provider_name,
        "wire_api": wire_api,
        "supports_websockets": supports_websockets,
        "openai_base_url": openai_base_url,
        "api_key_present": api_key_present,
        "account_id": account_id
    })
}
