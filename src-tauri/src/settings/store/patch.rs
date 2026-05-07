use super::super::{
    normalize::normalize_settings, normalize_background_refresh_interval_minutes, DEFAULT_API_NAME,
};
use crate::{
    api_config::normalize_api_base_url,
    json_util::{bool_field, has_key, raw_string_field, string_field, value_u64_field},
    proxy_config::{normalize_proxy_display_url, normalize_proxy_url},
};
use serde_json::{json, Map, Value};

fn normalize_api_mode_patch(patch: &Value) -> Result<Value, String> {
    let api_mode = patch.get("api_mode").unwrap_or(&Value::Null);
    let name = string_field(api_mode, "name");
    let base_url = string_field(api_mode, "base_url");
    Ok(json!({
        "name": if name.is_empty() { DEFAULT_API_NAME.to_string() } else { name },
        "base_url": if base_url.is_empty() {
            String::new()
        } else {
            normalize_api_base_url(&base_url)?
        },
        "api_key": string_field(api_mode, "api_key")
    }))
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
    if has_key(patch, "api_mode") {
        object.insert("api_mode".to_string(), normalize_api_mode_patch(patch)?);
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
}
