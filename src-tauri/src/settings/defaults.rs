pub(crate) use crate::api_config::DEFAULT_API_NAME;
use serde_json::{json, Value};

pub(crate) const DEFAULT_API_PROFILE_ID: &str = "default";
pub(crate) const DEFAULT_CODEX_PROXY_URL: &str = "127.0.0.1:10808";
pub(crate) const BACKGROUND_REFRESH_DEFAULT_INTERVAL_MINUTES: u64 = 30;
const BACKGROUND_REFRESH_MIN_INTERVAL_MINUTES: u64 = 1;
const BACKGROUND_REFRESH_MAX_INTERVAL_MINUTES: u64 = 24 * 60;

pub(crate) fn default_api_profile() -> Value {
    json!({
        "id": DEFAULT_API_PROFILE_ID,
        "name": DEFAULT_API_NAME,
        "base_url": "",
        "api_key": ""
    })
}

pub(crate) fn default_api_profiles() -> Value {
    json!([default_api_profile()])
}

pub(crate) fn default_api_mode() -> Value {
    default_api_profile()
}

pub(crate) fn normalize_background_refresh_interval_minutes(value: Option<u64>) -> u64 {
    value
        .unwrap_or(BACKGROUND_REFRESH_DEFAULT_INTERVAL_MINUTES)
        .clamp(
            BACKGROUND_REFRESH_MIN_INTERVAL_MINUTES,
            BACKGROUND_REFRESH_MAX_INTERVAL_MINUTES,
        )
}

pub(crate) fn default_settings() -> Value {
    json!({
        "dismissed_update_version": "",
        "close_window_behavior": "tray",
        "auto_start": true,
        "auto_start_launch_mode": "tray",
        "auto_check_updates": true,
        "background_refresh_enabled": true,
        "background_refresh_interval_minutes": BACKGROUND_REFRESH_DEFAULT_INTERVAL_MINUTES,
        "codex_proxy_url": DEFAULT_CODEX_PROXY_URL,
        "codex_proxy_env_enabled": false,
        "codex_plugins_enabled": false,
        "codex_remote_control_enabled": false,
        "codex_remote_control_account_id": "",
        "codex_computer_use_repair_guard_enabled": false,
        "codex_delete_button_enabled": false,
        "codex_session_sync_enabled": true,
        "codex_active_mode": "",
        "mask_account_name": false,
        "ui_theme": "light",
        "active_api_profile_id": DEFAULT_API_PROFILE_ID,
        "api_profiles": default_api_profiles(),
        "api_mode": default_api_mode(),
        "api_test_results": {},
        "window_bounds": {
            "width": 0,
            "height": 0
        },
        "window_is_maximized": false
    })
}
