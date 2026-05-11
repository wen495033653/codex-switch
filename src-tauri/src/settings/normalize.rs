use super::{
    normalize_background_refresh_interval_minutes, DEFAULT_API_NAME, DEFAULT_CODEX_PROXY_URL,
};
use crate::{
    api_config::normalize_api_base_url,
    json_util::{bool_field, raw_string_field, string_field, value_u64_field},
    proxy_config::normalize_proxy_display_url,
};
use serde_json::{json, Value};

fn normalize_api_mode(data: &Value) -> Value {
    let name = string_field(data, "name");
    let base_url = string_field(data, "base_url");
    json!({
        "name": if name.is_empty() { DEFAULT_API_NAME.to_string() } else { name },
        "base_url": if base_url.is_empty() {
            String::new()
        } else {
            normalize_api_base_url(&base_url).unwrap_or(base_url)
        },
        "api_key": string_field(data, "api_key")
    })
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
        "codex_session_sync_enabled": data
            .get("codex_session_sync_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        "codex_active_mode": normalize_codex_active_mode(data),
        "api_promo_bar_open": bool_field(data, "api_promo_bar_open"),
        "mask_account_name": bool_field(data, "mask_account_name"),
        "ui_theme": ui_theme,
        "api_mode": normalize_api_mode(data.get("api_mode").unwrap_or(&Value::Null)),
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
