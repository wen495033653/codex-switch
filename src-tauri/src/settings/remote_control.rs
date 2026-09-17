use crate::json_util::{bool_field, string_field};
use serde_json::Value;

pub(crate) const REMOTE_CONTROL_ENABLED_SETTING_KEY: &str = "codex_remote_control_enabled";
const LEGACY_REMOTE_CONTROL_HOOK_SETTING_KEY: &str = "codex_remote_control_hook_enabled";

pub(crate) fn remote_control_config_enabled_from_settings(settings: &Value) -> bool {
    bool_field(settings, REMOTE_CONTROL_ENABLED_SETTING_KEY)
        || bool_field(settings, LEGACY_REMOTE_CONTROL_HOOK_SETTING_KEY)
}

pub(crate) fn remote_control_suspended_by_subscription(settings: &Value) -> bool {
    string_field(settings, "codex_active_mode") == "chatgpt"
}

pub(crate) fn remote_control_enabled_from_settings(settings: &Value) -> bool {
    remote_control_config_enabled_from_settings(settings)
        && !remote_control_suspended_by_subscription(settings)
}
