use crate::{
    accounts::{read_api_key_from_auth, set_api_mode},
    codex_launcher::sync_remote_control_runtime_for_current_settings,
    json_util::string_field,
    settings::{default_api_mode, remote_control_enabled_from_settings},
};
use serde_json::Value;

pub(crate) mod account;
pub(crate) mod general;
pub(crate) mod quota;

#[derive(Debug, PartialEq)]
enum ApiProfileApply {
    Skip,
    /// With remote control on, Codex runs on the mixed config built from this profile; the
    /// plain API config would drop the control account's tokens and the provider's bearer.
    RemoteControlMixed,
    PlainApi(Value),
}

fn api_profile_apply(
    settings: &Value,
    auth_file_api_key: impl FnOnce() -> String,
) -> ApiProfileApply {
    if string_field(settings, "codex_active_mode") != "api" {
        return ApiProfileApply::Skip;
    }

    let profile = settings
        .get("api_mode")
        .cloned()
        .unwrap_or_else(default_api_mode);
    if string_field(&profile, "base_url").is_empty() {
        return ApiProfileApply::Skip;
    }
    if string_field(&profile, "api_key").is_empty() && auth_file_api_key().is_empty() {
        return ApiProfileApply::Skip;
    }
    if remote_control_enabled_from_settings(settings) {
        return ApiProfileApply::RemoteControlMixed;
    }
    ApiProfileApply::PlainApi(profile)
}

fn apply_complete_api_mode_profile_if_active(settings: &Value) -> Result<(), String> {
    match api_profile_apply(settings, read_api_key_from_auth) {
        ApiProfileApply::Skip => Ok(()),
        ApiProfileApply::RemoteControlMixed => {
            sync_remote_control_runtime_for_current_settings("update_settings").map(|_| ())
        }
        ApiProfileApply::PlainApi(profile) => set_api_mode(&profile),
    }
}

pub(crate) use general::sync_codex_model_instructions_config_for_current_settings;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn saving_an_api_profile_keeps_the_remote_control_mixed_config() {
        let profile = json!({ "base_url": "https://relay.example/v1", "api_key": "key" });
        let settings = |remote_control: bool, mode: &str| {
            json!({
                "codex_active_mode": mode,
                "codex_remote_control_enabled": remote_control,
                "api_mode": profile.clone()
            })
        };
        let no_auth_key = || String::new();

        assert_eq!(
            api_profile_apply(&settings(true, "api"), no_auth_key),
            ApiProfileApply::RemoteControlMixed
        );
        assert_eq!(
            api_profile_apply(&settings(false, "api"), no_auth_key),
            ApiProfileApply::PlainApi(profile.clone())
        );
        assert_eq!(
            api_profile_apply(&settings(true, "chatgpt"), no_auth_key),
            ApiProfileApply::Skip
        );
        let mut without_key = settings(false, "api");
        without_key["api_mode"]["api_key"] = json!("");
        assert_eq!(
            api_profile_apply(&without_key, no_auth_key),
            ApiProfileApply::Skip
        );
        assert_eq!(
            api_profile_apply(&without_key, || "from-auth".to_string()),
            ApiProfileApply::PlainApi(without_key["api_mode"].clone())
        );
    }
}
