use super::{
    default_api_profile, normalize_background_refresh_interval_minutes, DEFAULT_API_NAME,
    DEFAULT_API_PROFILE_ID, DEFAULT_CODEX_PROXY_URL,
};
use crate::{
    api_config::normalize_api_base_url,
    json_util::{bool_field, raw_string_field, string_field, value_u64_field},
    proxy_config::normalize_proxy_display_url,
};
use serde_json::{json, Value};

#[derive(Clone)]
struct ApiProfileState {
    active_id: String,
    active_profile: Value,
    profiles: Vec<Value>,
}

fn fallback_api_profile_id(index: usize) -> String {
    if index == 0 {
        DEFAULT_API_PROFILE_ID.to_string()
    } else {
        format!("api-{index}")
    }
}

fn normalized_api_profile_id(data: &Value, fallback_id: &str) -> String {
    let id = string_field(data, "id");
    if id.is_empty() {
        fallback_id.to_string()
    } else {
        id
    }
}

fn normalize_api_profile(data: &Value, fallback_id: &str) -> Value {
    let name = string_field(data, "name");
    let base_url = string_field(data, "base_url");
    json!({
        "id": normalized_api_profile_id(data, fallback_id),
        "name": if name.is_empty() { DEFAULT_API_NAME.to_string() } else { name },
        "base_url": if base_url.is_empty() {
            String::new()
        } else {
            normalize_api_base_url(&base_url).unwrap_or(base_url)
        },
        "api_key": string_field(data, "api_key")
    })
}

fn api_profile_id(profile: &Value) -> String {
    string_field(profile, "id")
}

fn normalize_api_profiles_state(data: &Value) -> ApiProfileState {
    let fallback_profile = normalize_api_profile(
        data.get("api_mode").unwrap_or(&Value::Null),
        DEFAULT_API_PROFILE_ID,
    );
    let raw_profiles = data
        .get("api_profiles")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut profiles: Vec<Value> = Vec::new();

    for (index, item) in raw_profiles.iter().enumerate() {
        let fallback_id = fallback_api_profile_id(index);
        let profile = normalize_api_profile(item, &fallback_id);
        let profile_id = api_profile_id(&profile);
        if profile_id.is_empty()
            || profiles
                .iter()
                .any(|existing| api_profile_id(existing) == profile_id)
        {
            continue;
        }
        profiles.push(profile);
    }

    if profiles.is_empty() {
        profiles.push(fallback_profile);
    }

    let requested_active_id = string_field(data, "active_api_profile_id");
    let active_id = if requested_active_id.is_empty() {
        api_profile_id(&profiles[0])
    } else if profiles
        .iter()
        .any(|profile| api_profile_id(profile) == requested_active_id)
    {
        requested_active_id
    } else {
        api_profile_id(&profiles[0])
    };
    let active_profile = profiles
        .iter()
        .find(|profile| api_profile_id(profile) == active_id)
        .cloned()
        .unwrap_or_else(default_api_profile);

    ApiProfileState {
        active_id,
        active_profile,
        profiles,
    }
}

fn normalize_codex_active_mode(data: &Value) -> String {
    match string_field(data, "codex_active_mode").as_str() {
        "api" => "api".to_string(),
        "chatgpt" => "chatgpt".to_string(),
        _ => String::new(),
    }
}

fn normalize_auto_start_launch_mode(data: &Value) -> String {
    match string_field(data, "auto_start_launch_mode").as_str() {
        "window" => "window".to_string(),
        _ => "tray".to_string(),
    }
}

pub(crate) fn normalize_settings(data: &Value) -> Value {
    let ui_theme = match string_field(data, "ui_theme").as_str() {
        "dark" => "dark",
        _ => "light",
    };
    let api_profiles_state = normalize_api_profiles_state(data);
    let background_refresh_interval_minutes = normalize_background_refresh_interval_minutes(
        value_u64_field(data, "background_refresh_interval_minutes"),
    );
    let has_codex_proxy_url = data.get("codex_proxy_url").is_some();
    let raw_codex_proxy_url = if has_codex_proxy_url {
        raw_string_field(data, "codex_proxy_url")
    } else {
        DEFAULT_CODEX_PROXY_URL.to_string()
    };
    let codex_proxy_url = normalize_proxy_display_url(&raw_codex_proxy_url);
    let window_bounds = data.get("window_bounds").unwrap_or(&Value::Null);
    let width = window_bounds
        .get("width")
        .and_then(Value::as_f64)
        .filter(|value| *value > 0.0)
        .map(|value| value.round() as i64)
        .unwrap_or(0);
    let height = window_bounds
        .get("height")
        .and_then(Value::as_f64)
        .filter(|value| *value > 0.0)
        .map(|value| value.round() as i64)
        .unwrap_or(0);

    json!({
        "dismissed_update_version": string_field(data, "dismissed_update_version"),
        "close_window_behavior": "tray",
        "auto_start": data
            .get("auto_start")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        "auto_start_launch_mode": normalize_auto_start_launch_mode(data),
        "auto_check_updates": data
            .get("auto_check_updates")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        "background_refresh_enabled": data
            .get("background_refresh_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        "background_refresh_interval_minutes": background_refresh_interval_minutes,
        "codex_proxy_url": codex_proxy_url,
        "codex_proxy_env_enabled": bool_field(data, "codex_proxy_env_enabled"),
        "codex_plugins_enabled": bool_field(data, "codex_plugins_enabled"),
        "codex_remote_control_enabled": bool_field(data, "codex_remote_control_enabled")
            || bool_field(data, "codex_remote_control_hook_enabled"),
        "codex_remote_control_account_id": string_field(data, "codex_remote_control_account_id"),
        "codex_delete_button_enabled": bool_field(data, "codex_delete_button_enabled"),
        "codex_session_sync_enabled": data
            .get("codex_session_sync_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        "codex_active_mode": normalize_codex_active_mode(data),
        "api_promo_bar_open": bool_field(data, "api_promo_bar_open"),
        "mask_account_name": bool_field(data, "mask_account_name"),
        "ui_theme": ui_theme,
        "active_api_profile_id": api_profiles_state.active_id,
        "api_profiles": api_profiles_state.profiles,
        "api_mode": api_profiles_state.active_profile,
        "window_bounds": {
            "width": width,
            "height": height
        },
        "window_is_maximized": bool_field(data, "window_is_maximized")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_settings_uses_default_proxy_url() {
        let settings = normalize_settings(&json!({}));

        assert_eq!(
            settings.get("codex_proxy_url").and_then(Value::as_str),
            Some(DEFAULT_CODEX_PROXY_URL)
        );
    }

    #[test]
    fn normalize_settings_preserves_empty_proxy_url_when_explicitly_cleared() {
        let settings = normalize_settings(&json!({
            "codex_proxy_url": ""
        }));

        assert_eq!(
            settings.get("codex_proxy_url").and_then(Value::as_str),
            Some("")
        );
    }

    #[test]
    fn normalize_settings_accepts_proxy_url_with_http_scheme() {
        let settings = normalize_settings(&json!({
            "codex_proxy_url": "http://127.0.0.1:10808"
        }));

        assert_eq!(
            settings.get("codex_proxy_url").and_then(Value::as_str),
            Some(DEFAULT_CODEX_PROXY_URL)
        );
    }

    #[test]
    fn normalize_settings_disables_codex_proxy_env_by_default() {
        let settings = normalize_settings(&json!({}));

        assert_eq!(
            settings
                .get("codex_proxy_env_enabled")
                .and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn normalize_settings_disables_codex_plugins_by_default() {
        let settings = normalize_settings(&json!({}));

        assert_eq!(
            settings
                .get("codex_plugins_enabled")
                .and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn normalize_settings_preserves_codex_plugins_enabled() {
        let settings = normalize_settings(&json!({
            "codex_plugins_enabled": true
        }));

        assert_eq!(
            settings
                .get("codex_plugins_enabled")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn normalize_settings_disables_codex_remote_control_by_default() {
        let settings = normalize_settings(&json!({}));

        assert_eq!(
            settings
                .get("codex_remote_control_enabled")
                .and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn normalize_settings_preserves_codex_remote_control_enabled() {
        let settings = normalize_settings(&json!({
            "codex_remote_control_enabled": true
        }));

        assert_eq!(
            settings
                .get("codex_remote_control_enabled")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn normalize_settings_preserves_codex_delete_button_enabled() {
        let settings = normalize_settings(&json!({
            "codex_delete_button_enabled": true
        }));

        assert_eq!(
            settings
                .get("codex_delete_button_enabled")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn normalize_settings_preserves_legacy_codex_remote_control_hook_enabled() {
        let settings = normalize_settings(&json!({
            "codex_remote_control_hook_enabled": true
        }));

        assert_eq!(
            settings
                .get("codex_remote_control_enabled")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn normalize_settings_preserves_codex_remote_control_account_id() {
        let settings = normalize_settings(&json!({
            "codex_remote_control_account_id": "acct-remote"
        }));

        assert_eq!(
            settings
                .get("codex_remote_control_account_id")
                .and_then(Value::as_str),
            Some("acct-remote")
        );
    }

    #[test]
    fn normalize_settings_normalizes_api_base_url() {
        let settings = normalize_settings(&json!({
            "api_mode": {
                "base_url": "gpt-pool.com",
                "api_key": "test-key"
            }
        }));

        assert_eq!(
            settings
                .get("api_mode")
                .and_then(|api_mode| api_mode.get("base_url"))
                .and_then(Value::as_str),
            Some("https://gpt-pool.com/v1")
        );
    }

    #[test]
    fn normalize_settings_migrates_single_api_mode_to_profiles() {
        let settings = normalize_settings(&json!({
            "api_mode": {
                "name": "Pool",
                "base_url": "gpt-pool.com",
                "api_key": "test-key"
            }
        }));

        assert_eq!(
            settings
                .get("active_api_profile_id")
                .and_then(Value::as_str),
            Some(DEFAULT_API_PROFILE_ID)
        );
        assert_eq!(
            settings
                .get("api_profiles")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            settings
                .get("api_mode")
                .and_then(|api_mode| api_mode.get("name"))
                .and_then(Value::as_str),
            Some("Pool")
        );
    }

    #[test]
    fn normalize_settings_uses_active_api_profile() {
        let settings = normalize_settings(&json!({
            "active_api_profile_id": "backup",
            "api_profiles": [
                {
                    "id": "default",
                    "name": "Default",
                    "base_url": "https://default.example.com/v1",
                    "api_key": "default-key"
                },
                {
                    "id": "backup",
                    "name": "Backup",
                    "base_url": "backup.example.com",
                    "api_key": "backup-key"
                }
            ]
        }));

        assert_eq!(
            settings
                .get("api_mode")
                .and_then(|api_mode| api_mode.get("name"))
                .and_then(Value::as_str),
            Some("Backup")
        );
        assert_eq!(
            settings
                .get("api_mode")
                .and_then(|api_mode| api_mode.get("base_url"))
                .and_then(Value::as_str),
            Some("https://backup.example.com/v1")
        );
    }

    #[test]
    fn normalize_settings_enables_codex_session_sync_by_default() {
        let settings = normalize_settings(&json!({}));

        assert_eq!(
            settings
                .get("codex_session_sync_enabled")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn normalize_settings_uses_tray_auto_start_launch_mode_by_default() {
        let settings = normalize_settings(&json!({}));

        assert_eq!(
            settings
                .get("auto_start_launch_mode")
                .and_then(Value::as_str),
            Some("tray")
        );
    }

    #[test]
    fn normalize_settings_preserves_tray_auto_start_launch_mode() {
        let settings = normalize_settings(&json!({
            "auto_start_launch_mode": "tray"
        }));

        assert_eq!(
            settings
                .get("auto_start_launch_mode")
                .and_then(Value::as_str),
            Some("tray")
        );
    }

    #[test]
    fn normalize_settings_preserves_known_codex_active_mode() {
        let settings = normalize_settings(&json!({
            "codex_active_mode": "api"
        }));

        assert_eq!(
            settings.get("codex_active_mode").and_then(Value::as_str),
            Some("api")
        );
    }

    #[test]
    fn normalize_settings_closes_api_promo_bar_by_default() {
        let settings = normalize_settings(&json!({}));

        assert_eq!(
            settings.get("api_promo_bar_open").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn normalize_settings_preserves_api_promo_bar_state() {
        let settings = normalize_settings(&json!({
            "api_promo_bar_open": true
        }));

        assert_eq!(
            settings.get("api_promo_bar_open").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn normalize_settings_drops_unknown_codex_active_mode() {
        let settings = normalize_settings(&json!({
            "codex_active_mode": "other"
        }));

        assert_eq!(
            settings.get("codex_active_mode").and_then(Value::as_str),
            Some("")
        );
    }
}
