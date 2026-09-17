use super::*;
use crate::{
    api_config::normalize_api_base_url,
    json_util::string_field,
    settings::{default_api_mode, read_settings_value},
};
use serde_json::Value;

const API_WIRE_RESPONSES: &str = "responses";

impl ApiModeProfile {
    pub(super) fn api_key_or_auth_file(&self) -> String {
        if self.api_key.is_empty() {
            let provider_key = read_api_key_from_provider_config().trim().to_string();
            if provider_key.is_empty() {
                read_api_key_from_auth()
            } else {
                provider_key
            }
        } else {
            self.api_key.clone()
        }
    }
}

pub(crate) fn set_subscription_mode() -> Result<(), String> {
    set_config_values(vec![("cli_auth_credentials_store", "file".to_string())])?;
    remove_config_values(&[
        "preferred_auth_method",
        "forced_login_method",
        "openai_base_url",
        "model_provider",
    ])?;
    remove_table_config(&format!("model_providers.{API_PROVIDER_ID}"))?;
    Ok(())
}

pub(crate) fn read_api_key_from_auth() -> String {
    read_auth_value()
        .ok()
        .and_then(|auth| {
            auth.get("OPENAI_API_KEY")
                .and_then(Value::as_str)
                .map(|value| value.trim().to_string())
        })
        .unwrap_or_default()
}

pub(crate) fn read_api_key_from_provider_config() -> String {
    let model_provider = read_root_config()
        .ok()
        .and_then(|config| {
            config
                .get("model_provider")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    if model_provider.is_empty() {
        return String::new();
    }

    read_table_config(&format!("model_providers.{model_provider}"))
        .ok()
        .and_then(|config| {
            config
                .get("experimental_bearer_token")
                .and_then(Value::as_str)
                .map(|value| value.trim().to_string())
        })
        .unwrap_or_default()
}

fn api_mode_provider_config(profile: &ApiModeProfile) -> Vec<(&'static str, Value)> {
    vec![
        ("name", Value::String(API_PROVIDER_ID.to_string())),
        ("base_url", Value::String(profile.base_url.clone())),
        ("wire_api", Value::String(API_WIRE_RESPONSES.to_string())),
        ("supports_websockets", Value::Bool(false)),
        ("requires_openai_auth", Value::Bool(true)),
    ]
}

pub(crate) fn set_api_mode(profile: &Value) -> Result<(), String> {
    let profile = ApiModeProfile::from_value(profile)?;
    let api_key = profile.api_key_or_auth_file().trim().to_string();
    write_api_auth(&api_key)?;
    set_config_values(vec![
        ("model_provider", API_PROVIDER_ID.to_string()),
        ("cli_auth_credentials_store", "file".to_string()),
    ])?;
    remove_config_values(&[
        "preferred_auth_method",
        "forced_login_method",
        "openai_base_url",
    ])?;
    set_table_config(
        &format!("model_providers.{API_PROVIDER_ID}"),
        api_mode_provider_config(&profile),
    )?;
    Ok(())
}

pub(crate) fn restore_api_mode_if_selected() -> Result<bool, String> {
    let settings = read_settings_value()?;
    if raw_string_field(&settings, "codex_active_mode") != "api" {
        return Ok(false);
    }

    let profile = settings
        .get("api_mode")
        .cloned()
        .unwrap_or_else(default_api_mode);
    if string_field(&profile, "base_url").is_empty() {
        return Ok(false);
    }

    let state = get_codex_state_value();
    if raw_string_field(&state, "mode") == "api"
        && raw_string_field(&state, "model_provider") == API_PROVIDER_ID
    {
        return Ok(false);
    }

    set_api_mode(&profile)?;
    Ok(true)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ApiModeProfile {
    pub(super) base_url: String,
    pub(super) api_key: String,
}

impl ApiModeProfile {
    pub(super) fn from_value(value: &Value) -> Result<Self, String> {
        Ok(Self {
            base_url: normalize_api_base_url(&string_field(value, "base_url"))?,
            api_key: string_field(value, "api_key"),
        })
    }
}

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
    let api_key_present =
        !read_api_key_from_auth().is_empty() || !read_api_key_from_provider_config().is_empty();
    let account_id = auth
        .get("tokens")
        .and_then(|tokens| tokens.get("account_id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let profile_id = profile_id_from_tokens_value(auth.get("tokens")).unwrap_or_default();

    let api_provider_ready = model_provider == API_PROVIDER_ID && !provider_base_url.is_empty();
    let api_credentials_ready = auth_mode == "apikey"
        || api_key_present
        || preferred_auth_method == "api"
        || forced_login_method == "api";

    let mode = if api_credentials_ready && api_provider_ready {
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
        "api_provider_ready": api_provider_ready,
        "account_id": account_id,
        "profile_id": profile_id
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    #[test]
    fn api_mode_provider_config_uses_responses_wire_api_and_openai_auth() {
        let profile = ApiModeProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: "sk-test".to_string(),
        };

        let config: Map<String, Value> = api_mode_provider_config(&profile)
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect();

        assert_eq!(config.get("name").and_then(Value::as_str), Some("api"));
        assert_eq!(
            config.get("wire_api").and_then(Value::as_str),
            Some(API_WIRE_RESPONSES)
        );
        assert_eq!(
            config.get("supports_websockets").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            config.get("requires_openai_auth").and_then(Value::as_bool),
            Some(true)
        );
    }
}
