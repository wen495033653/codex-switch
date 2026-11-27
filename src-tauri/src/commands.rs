use crate::{
    accounts::{get_codex_state_value, read_api_key_from_auth, set_api_mode},
    codex_launcher::sync_remote_control_runtime_for_current_settings,
    json_util::{has_key, string_field},
    settings::{
        default_api_mode, remote_control_enabled_from_settings, update_settings_value,
        update_settings_value_checked,
    },
};
use serde_json::Value;
use std::sync::{Mutex, MutexGuard};

pub(crate) mod account;
pub(crate) mod api_test;
pub(crate) mod general;
pub(crate) mod quota;

// Serialize explicit switches with profile edits. Otherwise an edit could validate against
// profile A, then remove or replace profile B during the operation which makes B current.
static API_PROFILE_OPERATION_LOCK: Mutex<()> = Mutex::new(());

fn lock_api_profile_operation() -> Result<MutexGuard<'static, ()>, String> {
    API_PROFILE_OPERATION_LOCK
        .lock()
        .map_err(|_| "API 配置操作锁异常".to_string())
}

fn update_command_settings(patch: &Value) -> Result<Value, String> {
    if !["api_mode", "api_profiles", "active_api_profile_id"]
        .iter()
        .any(|key| has_key(patch, key))
    {
        return update_settings_value(patch);
    }
    let _guard = lock_api_profile_operation()?;
    let api_active = string_field(&get_codex_state_value(), "mode") == "api";
    let mut apply = false;
    let settings = update_settings_value_checked(patch, |current, next| {
        apply = validate_api_settings_update(current, next, api_active)?;
        Ok(())
    })?;
    if apply {
        apply_complete_api_mode_profile_if_active(&settings)?;
        // A remote-control sync may have disabled a missing or expired control account.
        crate::settings::read_settings_value()
    } else {
        Ok(settings)
    }
}

/// The generic settings command may edit the contents of a profile, but only the switch
/// command may change the owner of API traffic while Codex is in API mode.
fn validate_api_settings_update(
    current: &Value,
    next: &Value,
    api_active: bool,
) -> Result<bool, String> {
    if !api_active {
        return Ok(false);
    }
    let current_id = string_field(current, "active_api_profile_id");
    let current_still_exists = next["api_profiles"].as_array().is_some_and(|profiles| {
        profiles
            .iter()
            .any(|profile| string_field(profile, "id") == current_id)
    });
    if !current_still_exists {
        return Err("请先切换到其他 API 配置，再删除当前配置".to_string());
    }
    if current_id != string_field(next, "active_api_profile_id") {
        return Err("请使用切换按钮切换 API 配置".to_string());
    }
    Ok(["base_url", "api_key"]
        .iter()
        .any(|key| string_field(&current["api_mode"], key) != string_field(&next["api_mode"], key)))
}

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
