use crate::{
    accounts::{read_api_key_from_auth, set_api_mode},
    json_util::string_field,
    settings::default_api_mode,
};
use serde_json::Value;

pub(crate) mod account;
pub(crate) mod general;
pub(crate) mod quota;

fn apply_complete_api_mode_profile_if_active(settings: &Value) -> Result<(), String> {
    if string_field(settings, "codex_active_mode") != "api" {
        return Ok(());
    }

    let profile = settings
        .get("api_mode")
        .cloned()
        .unwrap_or_else(default_api_mode);
    if string_field(&profile, "base_url").is_empty() {
        return Ok(());
    }
    if string_field(&profile, "api_key").is_empty() && read_api_key_from_auth().is_empty() {
        return Ok(());
    }

    set_api_mode(&profile)
}

pub(crate) use general::sync_codex_model_instructions_config_for_current_settings;
