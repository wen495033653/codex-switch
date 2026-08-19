use super::{mutation::add_account_to_store, query::find_store_account};
use crate::accounts::{
    build_error_state, get_codex_state_value, set_auth_state, write_account_auth,
};
use crate::codex_launcher::remote_control_enabled_from_settings;
use crate::json_util::raw_string_field;
use crate::settings::{read_settings_value, update_settings_value};
use serde_json::{json, Value};

use super::super::persistence::read_store_value;

pub(crate) fn sync_auth_file_if_active(profile_id: &str) -> Result<(), String> {
    let store = read_store_value()?;
    let state = get_codex_state_value();
    let settings = read_settings_value()?;
    if !auth_file_uses_profile(&store, &state, &settings, profile_id) {
        return Ok(());
    }
    let account = find_store_account(profile_id)?;
    write_account_auth(&account)
}

fn auth_file_uses_profile(
    store: &Value,
    state: &Value,
    settings: &Value,
    profile_id: &str,
) -> bool {
    let active_subscription_profile = raw_string_field(store, "active_id") == profile_id
        && raw_string_field(state, "mode") == "chatgpt"
        && raw_string_field(state, "profile_id") == profile_id;
    let active_remote_control_profile = remote_control_enabled_from_settings(settings)
        && raw_string_field(settings, "codex_remote_control_account_id") == profile_id;

    active_subscription_profile || active_remote_control_profile
}

pub(crate) fn mark_account_auth_error(profile_id: &str, message: &str) -> Result<Value, String> {
    let account = find_store_account(profile_id)?;
    let tokens = account.get("tokens").cloned().unwrap_or(Value::Null);
    let custom = set_auth_state(
        account.get("custom"),
        "error",
        message,
        build_error_state(message, "auth_refresh_failed", "", 0, ""),
        None,
        None,
    );
    let store = add_account_to_store(
        json!({
            "tokens": tokens,
            "custom": custom
        }),
        false,
    )?;
    disable_selected_remote_control_after_login_expired(profile_id, message)?;
    Ok(store)
}

pub(crate) fn auth_error_is_login_expired(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    [
        "refresh_token_invalidated",
        "refresh token invalidated",
        "session has ended",
        "invalid_grant",
        "unauthorized",
        "authorization expired",
        "authentication token is expired",
        "登录已失效",
        "登录已过期",
        "请重新登录",
    ]
    .iter()
    .any(|pattern| text.contains(pattern))
}

fn remote_control_disable_patch_for_auth_error(
    settings: &Value,
    profile_id: &str,
    message: &str,
) -> Option<Value> {
    if !auth_error_is_login_expired(message)
        || settings
            .get("codex_remote_control_enabled")
            .and_then(Value::as_bool)
            != Some(true)
        || raw_string_field(settings, "codex_remote_control_account_id") != profile_id
    {
        return None;
    }

    Some(json!({
        "codex_remote_control_enabled": false
    }))
}

fn disable_selected_remote_control_after_login_expired(
    profile_id: &str,
    message: &str,
) -> Result<(), String> {
    if !auth_error_is_login_expired(message) {
        return Ok(());
    }

    let settings = read_settings_value()?;
    let Some(patch) = remote_control_disable_patch_for_auth_error(&settings, profile_id, message)
    else {
        return Ok(());
    };
    update_settings_value(&patch)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_control_profile_uses_auth_file_in_api_mode() {
        let profile_id = "profile-remote";
        assert!(auth_file_uses_profile(
            &json!({ "active_id": "" }),
            &json!({ "mode": "api", "profile_id": "" }),
            &json!({
                "codex_remote_control_enabled": true,
                "codex_remote_control_account_id": profile_id,
                "codex_active_mode": "api"
            }),
            profile_id,
        ));
    }

    #[test]
    fn suspended_remote_control_profile_does_not_use_auth_file() {
        let profile_id = "profile-remote";
        assert!(!auth_file_uses_profile(
            &json!({ "active_id": "" }),
            &json!({ "mode": "api", "profile_id": "" }),
            &json!({
                "codex_remote_control_enabled": true,
                "codex_remote_control_account_id": profile_id,
                "codex_active_mode": "chatgpt"
            }),
            profile_id,
        ));
    }

    #[test]
    fn active_subscription_profile_still_uses_auth_file() {
        let profile_id = "profile-subscription";
        assert!(auth_file_uses_profile(
            &json!({ "active_id": profile_id }),
            &json!({ "mode": "chatgpt", "profile_id": profile_id }),
            &json!({
                "codex_remote_control_enabled": false,
                "codex_active_mode": "chatgpt"
            }),
            profile_id,
        ));
    }

    fn enabled_settings(account_id: &str) -> Value {
        json!({
            "codex_remote_control_enabled": true,
            "codex_remote_control_account_id": account_id,
            "codex_active_mode": "chatgpt"
        })
    }

    #[test]
    fn selected_expired_control_account_only_disables_remote_control() {
        let settings = enabled_settings("profile-selected");
        let patch = remote_control_disable_patch_for_auth_error(
            &settings,
            "profile-selected",
            "HTTP 401: refresh_token_invalidated; Your session has ended.",
        )
        .expect("selected expired account should disable remote control");

        assert_eq!(patch, json!({ "codex_remote_control_enabled": false }));
        assert!(patch.get("codex_remote_control_account_id").is_none());
        assert!(patch.get("codex_active_mode").is_none());
    }

    #[test]
    fn transient_auth_error_does_not_disable_remote_control() {
        let settings = enabled_settings("profile-selected");

        assert_eq!(
            remote_control_disable_patch_for_auth_error(
                &settings,
                "profile-selected",
                "network timeout"
            ),
            None
        );
    }

    #[test]
    fn expired_unselected_account_does_not_disable_remote_control() {
        let settings = enabled_settings("profile-selected");

        assert_eq!(
            remote_control_disable_patch_for_auth_error(
                &settings,
                "profile-other",
                "refresh_token_invalidated"
            ),
            None
        );
    }

    #[test]
    fn already_disabled_remote_control_stays_unchanged() {
        let mut settings = enabled_settings("profile-selected");
        settings["codex_remote_control_enabled"] = json!(false);

        assert_eq!(
            remote_control_disable_patch_for_auth_error(
                &settings,
                "profile-selected",
                "Your session has ended. Please log in again."
            ),
            None
        );
    }
}
