mod model;

use super::*;
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
    for legacy_provider_id in LEGACY_API_PROVIDER_IDS {
        remove_table_config(&format!("model_providers.{legacy_provider_id}"))?;
    }
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
        vec![
            ("name", Value::String(profile.provider_name())),
            ("base_url", Value::String(profile.base_url)),
            ("requires_openai_auth", Value::Bool(true)),
        ],
    )?;
    for legacy_provider_id in LEGACY_API_PROVIDER_IDS {
        remove_table_config(&format!("model_providers.{legacy_provider_id}"))?;
    }
    Ok(())
}
