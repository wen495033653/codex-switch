use super::super::{
    normalize::normalize_settings, normalize_background_refresh_interval_minutes, DEFAULT_API_NAME,
    DEFAULT_API_PROFILE_ID,
};
use crate::{
    api_config::normalize_api_base_url,
    json_util::{bool_field, has_key, raw_string_field, string_field, value_u64_field},
    proxy_config::{normalize_proxy_display_url, normalize_proxy_url},
};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

fn fallback_api_profile_id(index: usize) -> String {
    if index == 0 {
        DEFAULT_API_PROFILE_ID.to_string()
    } else {
        format!("api-{index}")
    }
}

fn normalize_api_profile_patch(profile: &Value, fallback_id: &str) -> Result<Value, String> {
    let id = string_field(profile, "id");
    let name = string_field(profile, "name");
    let base_url = string_field(profile, "base_url");
    Ok(json!({
        "id": if id.is_empty() { fallback_id.to_string() } else { id },
        "name": if name.is_empty() { DEFAULT_API_NAME.to_string() } else { name },
        "base_url": if base_url.is_empty() {
            String::new()
        } else {
            normalize_api_base_url(&base_url)?
        },
        "api_key": string_field(profile, "api_key")
    }))
}

fn normalize_api_mode_patch(patch: &Value, fallback_id: &str) -> Result<Value, String> {
    normalize_api_profile_patch(patch.get("api_mode").unwrap_or(&Value::Null), fallback_id)
}

fn normalize_api_profiles_patch(profiles: &Value) -> Result<Value, String> {
    let mut ids = HashSet::new();
    let mut normalized = Vec::new();
    for (index, profile) in profiles
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let fallback_id = fallback_api_profile_id(index);
        let normalized_profile = normalize_api_profile_patch(profile, &fallback_id)?;
        let id = string_field(&normalized_profile, "id");
        if !ids.insert(id.clone()) {
            return Err(format!("API 配置 ID 重复：{id}"));
        }
        normalized.push(normalized_profile);
    }
    Ok(Value::Array(normalized))
}

fn current_active_api_profile_id(object: &Map<String, Value>) -> String {
    let id = object
        .get("active_api_profile_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if id.is_empty() {
        DEFAULT_API_PROFILE_ID.to_string()
    } else {
        id
    }
}

fn upsert_api_profile(object: &mut Map<String, Value>, profile: &Value) {
    let profile_id = string_field(profile, "id");
    let mut profiles = object
        .get("api_profiles")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| {
            object
                .get("api_mode")
                .cloned()
                .map(|api_mode| vec![api_mode])
                .unwrap_or_default()
        });

    if let Some(existing) = profiles
        .iter_mut()
        .find(|existing| string_field(existing, "id") == profile_id)
    {
        *existing = profile.clone();
    } else {
        profiles.push(profile.clone());
    }

    object.insert("api_profiles".to_string(), Value::Array(profiles));
}

pub(super) fn apply_settings_patch(
    object: &mut Map<String, Value>,
    patch: &Value,
) -> Result<(), String> {
    if has_key(patch, "dismissed_update_version") {
        object.insert(
            "dismissed_update_version".to_string(),
            Value::String(string_field(patch, "dismissed_update_version")),
        );
    }
    if has_key(patch, "close_window_behavior") {
        object.insert(
            "close_window_behavior".to_string(),
            Value::String("tray".to_string()),
        );
    }
    if has_key(patch, "auto_start") {
        object.insert(
            "auto_start".to_string(),
            Value::Bool(bool_field(patch, "auto_start")),
        );
    }
    if has_key(patch, "auto_check_updates") {
        object.insert(
            "auto_check_updates".to_string(),
            Value::Bool(bool_field(patch, "auto_check_updates")),
        );
    }
    if has_key(patch, "background_refresh_enabled") {
        object.insert(
            "background_refresh_enabled".to_string(),
            Value::Bool(bool_field(patch, "background_refresh_enabled")),
        );
    }
    if has_key(patch, "background_refresh_interval_minutes") {
        object.insert(
            "background_refresh_interval_minutes".to_string(),
            json!(normalize_background_refresh_interval_minutes(
                value_u64_field(patch, "background_refresh_interval_minutes",)
            )),
        );
    }
    if has_key(patch, "codex_proxy_url") {
        let proxy_url = raw_string_field(patch, "codex_proxy_url");
        normalize_proxy_url(&proxy_url)?;
        object.insert(
            "codex_proxy_url".to_string(),
            Value::String(normalize_proxy_display_url(&proxy_url)),
        );
    }
    if has_key(patch, "codex_proxy_env_enabled") {
        object.insert(
            "codex_proxy_env_enabled".to_string(),
            Value::Bool(bool_field(patch, "codex_proxy_env_enabled")),
        );
    }
    if has_key(patch, "codex_session_sync_enabled") {
        object.insert(
            "codex_session_sync_enabled".to_string(),
            Value::Bool(
                patch
                    .get("codex_session_sync_enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            ),
        );
    }
    if has_key(patch, "codex_active_mode") {
        let value = match string_field(patch, "codex_active_mode").as_str() {
            "api" => "api",
            "chatgpt" => "chatgpt",
            _ => "",
        };
        object.insert(
            "codex_active_mode".to_string(),
            Value::String(value.to_string()),
        );
    }
    if has_key(patch, "mask_account_name") {
        object.insert(
            "mask_account_name".to_string(),
            Value::Bool(bool_field(patch, "mask_account_name")),
        );
    }
    if has_key(patch, "ui_theme") {
        let value = match string_field(patch, "ui_theme").as_str() {
            "light" => "light",
            _ => "dark",
        };
        object.insert("ui_theme".to_string(), Value::String(value.to_string()));
    }
    if has_key(patch, "api_profiles") {
        object.insert(
            "api_profiles".to_string(),
            normalize_api_profiles_patch(patch.get("api_profiles").unwrap_or(&Value::Null))?,
        );
    }
    if has_key(patch, "active_api_profile_id") {
        object.insert(
            "active_api_profile_id".to_string(),
            Value::String(string_field(patch, "active_api_profile_id")),
        );
    }
    if has_key(patch, "api_mode") {
        let fallback_id = current_active_api_profile_id(object);
        let api_mode = normalize_api_mode_patch(patch, &fallback_id)?;
        let api_profile_id = string_field(&api_mode, "id");
        object.insert("api_mode".to_string(), api_mode.clone());
        object.insert(
            "active_api_profile_id".to_string(),
            Value::String(api_profile_id),
        );
        upsert_api_profile(object, &api_mode);
    }
    if has_key(patch, "window_bounds") {
        let bounds = normalize_settings(&json!({
            "window_bounds": patch.get("window_bounds").unwrap_or(&Value::Null)
        }));
        object.insert(
            "window_bounds".to_string(),
            bounds
                .get("window_bounds")
                .cloned()
                .unwrap_or_else(|| json!({ "width": 0, "height": 0 })),
        );
    }
    if has_key(patch, "window_is_maximized") {
        object.insert(
            "window_is_maximized".to_string(),
            Value::Bool(bool_field(patch, "window_is_maximized")),
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_settings_patch_normalizes_api_base_url() {
        let mut object = Map::new();

        apply_settings_patch(
            &mut object,
            &json!({
                "api_mode": {
                    "base_url": "gpt-pool.com",
                    "api_key": "test-key"
                }
            }),
        )
        .unwrap();

        assert_eq!(
            object
                .get("api_mode")
                .and_then(|api_mode| api_mode.get("base_url"))
                .and_then(Value::as_str),
            Some("https://gpt-pool.com/v1")
        );
    }

    #[test]
    fn apply_settings_patch_updates_active_api_profile() {
        let mut object = Map::new();
        object.insert("active_api_profile_id".to_string(), json!("backup"));
        object.insert(
            "api_profiles".to_string(),
            json!([
                {
                    "id": "default",
                    "name": "Default",
                    "base_url": "https://default.example.com/v1",
                    "api_key": "default-key"
                },
                {
                    "id": "backup",
                    "name": "Backup",
                    "base_url": "https://backup.example.com/v1",
                    "api_key": "old-key"
                }
            ]),
        );

        apply_settings_patch(
            &mut object,
            &json!({
                "api_mode": {
                    "name": "Backup",
                    "base_url": "backup.example.com",
                    "api_key": "new-key"
                }
            }),
        )
        .unwrap();

        let backup = object
            .get("api_profiles")
            .and_then(Value::as_array)
            .and_then(|profiles| {
                profiles
                    .iter()
                    .find(|profile| string_field(profile, "id") == "backup")
            })
            .unwrap();

        assert_eq!(
            backup.get("api_key").and_then(Value::as_str),
            Some("new-key")
        );
        assert_eq!(
            backup.get("base_url").and_then(Value::as_str),
            Some("https://backup.example.com/v1")
        );
    }

    #[test]
    fn apply_settings_patch_rejects_duplicate_api_profile_ids() {
        let mut object = Map::new();

        let err = apply_settings_patch(
            &mut object,
            &json!({
                "api_profiles": [
                    {
                        "id": "same",
                        "base_url": "https://a.example.com/v1"
                    },
                    {
                        "id": "same",
                        "base_url": "https://b.example.com/v1"
                    }
                ]
            }),
        )
        .unwrap_err();

        assert_eq!(err, "API 配置 ID 重复：same");
    }

    #[test]
    fn apply_settings_patch_rejects_invalid_api_base_url() {
        let mut object = Map::new();

        let err = apply_settings_patch(
            &mut object,
            &json!({
                "api_mode": {
                    "base_url": "https://gpt-pool.com/v1?debug=1",
                    "api_key": "test-key"
                }
            }),
        )
        .unwrap_err();

        assert_eq!(err, "API Base URL 不能包含 query 或 fragment");
    }

    #[test]
    fn apply_settings_patch_updates_codex_session_sync_enabled() {
        let mut object = Map::new();

        apply_settings_patch(
            &mut object,
            &json!({
                "codex_session_sync_enabled": false
            }),
        )
        .unwrap();

        assert_eq!(
            object
                .get("codex_session_sync_enabled")
                .and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn apply_settings_patch_updates_codex_active_mode() {
        let mut object = Map::new();

        apply_settings_patch(
            &mut object,
            &json!({
                "codex_active_mode": "api"
            }),
        )
        .unwrap();

        assert_eq!(
            object.get("codex_active_mode").and_then(Value::as_str),
            Some("api")
        );
    }
}
