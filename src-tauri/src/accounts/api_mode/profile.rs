mod model;

use super::*;
use crate::settings::{default_api_mode, read_settings_value};
use model::ApiModeProfile;

impl ApiModeProfile {
    pub(super) fn api_key_or_auth_file(&self) -> String {
        if self.api_key.is_empty() {
            read_api_key_from_auth()
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

fn api_mode_provider_config(profile: &ApiModeProfile) -> Vec<(&'static str, Value)> {
    vec![
        ("name", Value::String(profile.provider_name())),
        ("base_url", Value::String(profile.base_url.clone())),
        ("wire_api", Value::String("responses".to_string())),
        ("supports_websockets", Value::Bool(true)),
        ("requires_openai_auth", Value::Bool(true)),
    ]
}

pub(crate) fn set_api_mode(profile: &Value) -> Result<(), String> {
    let profile = ApiModeProfile::from_value(profile)?;
    let api_key = profile.api_key_or_auth_file();
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

    let state = super::state::get_codex_state_value();
    if raw_string_field(&state, "mode") == "api"
        && raw_string_field(&state, "model_provider") == API_PROVIDER_ID
    {
        return Ok(false);
    }

    set_api_mode(&profile)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    #[test]
    fn api_mode_provider_config_enables_responses_websockets() {
        let profile = ApiModeProfile {
            name: "OpenAI".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: "sk-test".to_string(),
        };

        let config: Map<String, Value> = api_mode_provider_config(&profile)
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect();

        assert_eq!(
            config.get("wire_api").and_then(Value::as_str),
            Some("responses")
        );
        assert_eq!(
            config.get("supports_websockets").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            config.get("requires_openai_auth").and_then(Value::as_bool),
            Some(true)
        );
    }
}
