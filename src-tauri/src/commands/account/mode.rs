use crate::{
    accounts::{
        account_from_exchange_syncing, add_account_to_store, auth_to_account,
        exchange_refresh_token, find_store_account, get_codex_state_value, lookup_store_account,
        mark_store_account_used, prepare_api_mode, profile_id_from_account, read_auth_value,
        read_runtime_api_key, read_store_value, remove_store_account, set_subscription_mode,
        store_payload, store_payload_from_store, sync_auth_file_if_active, write_account_auth,
    },
    codex_launcher::{
        attach_ide_reopen, build_ide_reopen_payload, stop_codex_api_quota_hooks,
        sync_remote_control_runtime_for_current_settings, IdeRuntime,
    },
    commands::lock_api_profile_operation,
    json_util::{bool_field, raw_string_field, string_field},
    quota::{refresh_active_account_usage_in_background, sync_account_usage_in_background},
    settings::{read_settings_value, update_settings_value, update_settings_value_checked},
    usage_stats::with_api_switch_attribution,
};
use serde_json::{json, Value};
use tauri::AppHandle;

/// The switch itself succeeded; only the token statistics lose track of who owns the next
/// sessions, so the failure is recorded rather than returned.
fn log_attribution_error(owner_type: &str, error: String) {
    crate::app_log::log_event(
        "usage_stats_attribution_error",
        json!({ "ownerType": owner_type, "error": error }),
    );
}

fn codex_session_sync_enabled(settings: &Value) -> bool {
    settings
        .get("codex_session_sync_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn remote_control_config_enabled(settings: &Value) -> bool {
    bool_field(settings, "codex_remote_control_enabled")
}

fn remote_control_reset_required_before_account_deletion<F>(
    settings: &Value,
    deleted_profile_id: &str,
    lookup: F,
) -> Result<bool, String>
where
    F: FnOnce(&str) -> Result<Option<Value>, String>,
{
    if !remote_control_config_enabled(settings) {
        return Ok(false);
    }

    let selected_account_id = string_field(settings, "codex_remote_control_account_id");
    if selected_account_id.is_empty() {
        return Ok(true);
    }

    let Some(selected_account) = lookup(&selected_account_id)? else {
        return Ok(true);
    };
    Ok(profile_id_from_account(&selected_account)? == deleted_profile_id)
}

fn missing_remote_control_account_fallback_applied(before: &Value, after: &Value) -> bool {
    remote_control_config_enabled(before)
        && !bool_field(after, "codex_remote_control_enabled")
        && string_field(after, "codex_remote_control_account_id").is_empty()
        && string_field(after, "codex_active_mode") == "api"
}

fn attach_settings(mut response: Value, settings: Value) -> Value {
    if let Some(response) = response.as_object_mut() {
        response.insert("settings".to_string(), settings);
    }
    response
}

fn attach_remote_control_runtime_result(
    mut response: Value,
    changed: bool,
    restart_required: bool,
) -> Value {
    if let Some(response) = response.as_object_mut() {
        response.insert("changed".to_string(), json!(changed));
        response.insert("restartRequired".to_string(), json!(restart_required));
    }
    response
}

pub(super) fn capture_current_impl() -> Result<Value, String> {
    let auth = read_auth_value()?;
    let codex_state = get_codex_state_value();
    if raw_string_field(&codex_state, "mode") == "api" {
        let api_key = read_runtime_api_key();
        if !api_key.is_empty() {
            let current = read_settings_value()?;
            let current_api = current.get("api_mode").unwrap_or(&Value::Null);
            update_settings_value(&json!({
                "api_mode": {
                    "name": string_field(current_api, "name"),
                    "base_url": raw_string_field(&codex_state, "openai_base_url"),
                    "api_key": api_key
                }
            }))?;
            return store_payload(Some("已保存当前 API 模式配置"));
        }
        return store_payload(Some(
            "已识别当前为 API 模式，但 auth.json 中没有可保存的 API Key",
        ));
    }

    let account = auth_to_account(&auth)?;
    let store = add_account_to_store(account, true)?;
    Ok(store_payload_from_store(store, Some("已保存当前账号")))
}

pub(super) fn import_refresh_token_impl(app: AppHandle, token: String) -> Result<Value, String> {
    let refresh_token = token.trim();
    if refresh_token.is_empty() {
        return Err("refresh_token 不能为空".to_string());
    }

    let exchange = exchange_refresh_token(refresh_token)?;
    let account_id = string_field(&exchange, "account_id");
    let access_token = string_field(&exchange, "access_token");
    let account = account_from_exchange_syncing(&exchange, None)?;
    let profile_id = profile_id_from_account(&account)?;
    let store = add_account_to_store(account, false)?;
    sync_auth_file_if_active(&profile_id)?;
    sync_account_usage_in_background(app, profile_id, account_id, access_token);
    Ok(store_payload_from_store(
        store,
        Some("已通过 refresh_token 导入账号，正在同步配额"),
    ))
}

pub(super) fn delete_account_impl(id: String) -> Result<Value, String> {
    let profile_id = id.trim();
    if profile_id.is_empty() {
        return Err("account_id 无效".to_string());
    }
    let settings_before = read_settings_value()?;
    let should_reset_remote_control = remote_control_reset_required_before_account_deletion(
        &settings_before,
        profile_id,
        lookup_store_account,
    )?;
    let codex_app_running = if should_reset_remote_control {
        crate::codex_launcher::remote_control_codex_app_running()?
    } else {
        false
    };
    let subscription_runtime_before_reset =
        string_field(&settings_before, "codex_active_mode") == "chatgpt";
    let (settings, changed) = if should_reset_remote_control {
        let settings = crate::codex_launcher::reset_remote_control_to_api_mode_settings()?;
        let runtime_changed =
            sync_remote_control_runtime_for_current_settings("delete_account_prepare")?;
        (
            settings,
            runtime_changed || subscription_runtime_before_reset,
        )
    } else {
        (settings_before.clone(), false)
    };
    let store = remove_store_account(profile_id)?;
    let fallback_applied =
        missing_remote_control_account_fallback_applied(&settings_before, &settings);
    let restart_required = codex_app_running && changed;
    let message = if fallback_applied && restart_required {
        "已删除；远程控制账号已重置并切换到 API 模式，重启 Codex 后生效"
    } else if fallback_applied {
        "已删除；远程控制账号已重置并切换到 API 模式"
    } else {
        "已删除"
    };
    Ok(attach_remote_control_runtime_result(
        attach_settings(store_payload_from_store(store, Some(message)), settings),
        changed,
        restart_required,
    ))
}

pub(super) fn switch_account_impl(
    app: AppHandle,
    id: String,
    runtime: &IdeRuntime,
) -> Result<Value, String> {
    let profile_id = id.trim();
    if profile_id.is_empty() {
        return Err("account_id 无效".to_string());
    }
    let settings = read_settings_value()?;
    let account = find_store_account(profile_id)?;
    write_account_auth(&account)?;
    set_subscription_mode()?;
    update_settings_value(&json!({ "codex_active_mode": "chatgpt" }))?;
    stop_codex_api_quota_hooks()?;
    let session_sync_enabled = codex_session_sync_enabled(&settings);
    let ide_reopen = build_ide_reopen_payload(
        runtime,
        profile_id.to_string(),
        false,
        session_sync_enabled.then(|| "openai".to_string()),
    );
    let message = if session_sync_enabled && ide_reopen.is_some() {
        "已切换到订阅模式；重新打开 IDE 前会同步会话".to_string()
    } else {
        "已切换到订阅模式".to_string()
    };
    let store = mark_store_account_used(profile_id)?;
    if let Err(err) = crate::usage_stats::record_attribution("subscription", profile_id, "openai") {
        log_attribution_error("subscription", err);
    }
    refresh_active_account_usage_in_background(app);
    Ok(attach_ide_reopen(
        store_payload_from_store(store, Some(&message)),
        ide_reopen,
    ))
}

pub(super) fn switch_api_mode_impl(
    runtime: &IdeRuntime,
    profile_id: Option<String>,
) -> Result<Value, String> {
    let _guard = lock_api_profile_operation()?;
    let (active_profile_id, store, settings_before_sync) =
        apply_api_profile_switch(profile_id.as_deref()).inspect_err(|error| {
            crate::app_log::log_event(
                "api_mode_switch_error",
                json!({
                    "profileId": profile_id.as_deref().map(crate::app_log::account_label), "error": error
                }),
            );
        })?;

    // At this point the plain provider and its bookkeeping are committed. Remote control uses
    // the same provider; if its separate sync fails, report the failure without hiding the
    // applied API profile or pretending that it was never switched.
    let mut settings = settings_before_sync.clone();
    let mut sync_errors = Vec::new();
    if bool_field(&settings_before_sync, "codex_remote_control_enabled") {
        if let Err(error) = sync_remote_control_runtime_for_current_settings("switch_api_mode") {
            sync_errors.push(error);
        }
        match read_settings_value() {
            Ok(current) => settings = current,
            Err(error) => sync_errors.push(error),
        }
    }
    let fallback_applied =
        missing_remote_control_account_fallback_applied(&settings_before_sync, &settings);
    let session_sync_enabled = codex_session_sync_enabled(&settings);
    let ide_reopen = build_ide_reopen_payload(
        runtime,
        active_profile_id,
        true,
        session_sync_enabled.then(|| "api".to_string()),
    );
    let message = if !sync_errors.is_empty() {
        "API 配置和账单归属已切换，但远程控制同步失败"
    } else if fallback_applied && session_sync_enabled && ide_reopen.is_some() {
        "远程控制账号不存在，已关闭远程控制并切换到 API 模式；重新打开 IDE 前会同步会话"
    } else if fallback_applied {
        "远程控制账号不存在，已关闭远程控制并切换到 API 模式"
    } else if session_sync_enabled && ide_reopen.is_some() {
        "已切换到 API 模式；重新打开 IDE 前会同步会话"
    } else {
        "已切换到 API 模式"
    };
    let mut response = attach_settings(
        attach_ide_reopen(store_payload_from_store(store, Some(message)), ide_reopen),
        settings,
    );
    if !sync_errors.is_empty() {
        let error = sync_errors.join("；");
        let control_account =
            string_field(&settings_before_sync, "codex_remote_control_account_id");
        let log_error = if control_account.is_empty() {
            error.clone()
        } else {
            error.replace(
                &control_account,
                &crate::app_log::account_label(&control_account),
            )
        };
        crate::app_log::log_event(
            "codex_remote_control_runtime_sync_error",
            json!({ "context": "switch_api_mode", "handling": "api_profile_applied", "error": log_error }),
        );
        response["api_switch_runtime_error"] = Value::String(format!("{message}：{error}"));
    }
    Ok(response)
}

fn api_profile_for_switch<'a>(
    settings: &'a Value,
    requested_id: Option<&str>,
) -> Result<&'a Value, String> {
    let id = requested_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| string_field(settings, "active_api_profile_id"));
    let profile = settings
        .get("api_profiles")
        .and_then(Value::as_array)
        .and_then(|profiles| {
            profiles
                .iter()
                .find(|profile| string_field(profile, "id") == id)
        })
        .ok_or_else(|| "API 配置不存在".to_string())?;
    if string_field(profile, "base_url").is_empty() {
        return Err("API Base URL 不能为空".to_string());
    }
    if string_field(profile, "api_key").is_empty() {
        return Err("API Key 不能为空".to_string());
    }
    Ok(profile)
}

fn apply_api_profile_switch(requested_id: Option<&str>) -> Result<(String, Value, Value), String> {
    let before = read_settings_value()?;
    let profile = api_profile_for_switch(&before, requested_id)?;
    let profile_id = string_field(profile, "id");
    let edit = prepare_api_mode(profile)?;
    let store = read_store_value()?;
    let settings = with_api_switch_attribution(&profile_id, |attribution| {
        edit.apply_then(|| {
            let state = get_codex_state_value();
            if string_field(&state, "mode") != "api"
                || string_field(&state, "openai_base_url") != string_field(profile, "base_url")
            {
                return Err("切换失败：Codex 未进入 API 模式".to_string());
            }
            let applied = update_settings_value(&json!({
                "active_api_profile_id": profile_id, "codex_active_mode": "api"
            }))?;
            if let Err(error) = attribution.commit() {
                let restoration = update_settings_value_checked(
                    &json!({
                        "active_api_profile_id": before["active_api_profile_id"],
                        "codex_active_mode": before["codex_active_mode"]
                    }),
                    |current, _| {
                        if current["active_api_profile_id"] != applied["active_api_profile_id"]
                            || current["codex_active_mode"] != applied["codex_active_mode"]
                        {
                            return Err("当前选择已被其他操作修改，未覆盖".to_string());
                        }
                        Ok(())
                    },
                );
                return Err(match restoration {
                    Ok(_) => error,
                    Err(rollback_error) => format!("{error}；恢复 API 选择失败：{rollback_error}"),
                });
            }
            Ok(applied)
        })
    })?;
    Ok((profile_id, store, settings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_remote_control_fallback_is_detected_for_command_response() {
        let before = json!({
            "codex_remote_control_enabled": true,
            "codex_remote_control_account_id": "profile-missing",
            "codex_active_mode": "chatgpt"
        });
        let after = json!({
            "codex_remote_control_enabled": false,
            "codex_remote_control_account_id": "",
            "codex_active_mode": "api"
        });

        assert!(missing_remote_control_account_fallback_applied(
            &before, &after
        ));
    }

    #[test]
    fn deleting_selected_remote_control_account_requests_central_sync() {
        let settings = json!({
            "codex_remote_control_enabled": true,
            "codex_remote_control_account_id": "profile-selected",
            "codex_active_mode": "chatgpt"
        });
        let selected_account = json!({ "profile_id": "profile-selected" });

        assert!(remote_control_reset_required_before_account_deletion(
            &settings,
            "profile-selected",
            |_| Ok(Some(selected_account))
        )
        .unwrap());
    }

    #[test]
    fn explicit_switch_uses_only_the_requested_complete_profile() {
        let settings = json!({
            "active_api_profile_id": "a",
            "api_profiles": [
                { "id": "a", "base_url": "https://a.example/v1", "api_key": "fixture-a" },
                { "id": "b", "base_url": "https://b.example/v1", "api_key": "fixture-b" },
                { "id": "without-key", "base_url": "https://bad.example/v1" },
                { "id": "without-url", "api_key": "fixture-bad" }
            ]
        });
        assert_eq!(api_profile_for_switch(&settings, None).unwrap()["id"], "a");
        assert_eq!(
            api_profile_for_switch(&settings, Some("b")).unwrap()["id"],
            "b"
        );
        for (id, error) in [
            ("removed", "API 配置不存在"),
            ("without-key", "API Key 不能为空"),
            ("without-url", "API Base URL 不能为空"),
        ] {
            assert_eq!(
                api_profile_for_switch(&settings, Some(id)).unwrap_err(),
                error
            );
        }
    }

    #[test]
    #[ignore = "writes fixture files; run with CODEX_SWITCH_API_TEST_ROOT, HOME, USERPROFILE and APPDATA isolated"]
    fn isolated_api_switch_keeps_files_and_usage_attribution_in_sync() {
        use crate::{
            commands::update_command_settings,
            paths::{app_data_dir, auth_path, config_path, home_dir},
        };
        use rusqlite::Connection;
        use std::{env, fs, path::PathBuf};

        let root = PathBuf::from(env::var_os("CODEX_SWITCH_API_TEST_ROOT").unwrap());
        assert!(root.is_absolute() && root.is_dir());
        assert_eq!(home_dir().unwrap(), root.join("home"));
        let app_data = app_data_dir().unwrap();
        assert!(app_data.starts_with(&root) && app_data != root);
        assert!(!app_data.exists());
        assert!(!auth_path().unwrap().exists());
        assert!(!config_path().unwrap().exists());

        let a = json!({ "id": "a", "name": "A", "base_url": "https://a.example/v1", "api_key": "fixture-a" });
        let b = json!({ "id": "b", "name": "B", "base_url": "https://b.example/v1", "api_key": "fixture-b" });
        update_settings_value(&json!({ "api_profiles": [&a, &b] })).unwrap();
        apply_api_profile_switch(Some("a")).unwrap();
        let auth_before = fs::read(auth_path().unwrap()).unwrap();
        let config_before = fs::read(config_path().unwrap()).unwrap();
        let conn = Connection::open(app_data.join("usage-stats.sqlite")).unwrap();
        let owners = || {
            conn.prepare("SELECT owner_id FROM attribution WHERE provider = 'api' ORDER BY id")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        let assert_unchanged = || {
            assert_eq!(owners(), ["a"]);
            assert_eq!(read_settings_value().unwrap()["active_api_profile_id"], "a");
            assert_eq!(fs::read(auth_path().unwrap()).unwrap(), auth_before);
            assert_eq!(fs::read(config_path().unwrap()).unwrap(), config_before);
        };

        let deleting_a = update_command_settings(&json!({ "api_profiles": [&b] })).unwrap_err();
        assert!(deleting_a.contains("先切换"));
        assert!(update_command_settings(&json!({ "active_api_profile_id": "b" })).is_err());
        assert!(apply_api_profile_switch(Some("missing")).is_err());
        assert_eq!(
            update_command_settings(&json!({ "api_profiles": [&a] })).unwrap()["api_profiles"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        update_command_settings(&json!({ "api_profiles": [&a, &b] })).unwrap();
        let mut edited_b = b.clone();
        edited_b["api_key"] = json!("fixture-edited-b");
        update_command_settings(&json!({ "api_profiles": [&a, &edited_b] })).unwrap();
        assert_unchanged();

        conn.execute_batch("CREATE TRIGGER reject_owner BEFORE INSERT ON attribution WHEN NEW.owner_id = 'b' BEGIN SELECT RAISE(ABORT, 'fixture reject attribution'); END;").unwrap();
        assert!(apply_api_profile_switch(Some("b"))
            .unwrap_err()
            .contains("fixture reject attribution"));
        assert_unchanged();
        conn.execute_batch("DROP TRIGGER reject_owner;").unwrap();

        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let deny_replace = fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(auth_path().unwrap())
                .unwrap();
            assert!(apply_api_profile_switch(Some("b"))
                .unwrap_err()
                .contains("auth.json"));
            drop(deny_replace);
            assert_unchanged();

            let deny_replace = fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(app_data.join("settings.json"))
                .unwrap();
            assert!(apply_api_profile_switch(Some("b"))
                .unwrap_err()
                .contains("settings.json"));
            drop(deny_replace);
            assert_unchanged();
        }

        let (_, _, applied) = apply_api_profile_switch(Some("b")).unwrap();
        assert_eq!(applied["active_api_profile_id"], "b");
        assert_eq!(
            read_auth_value().unwrap()["OPENAI_API_KEY"],
            "fixture-edited-b"
        );
        assert_eq!(
            get_codex_state_value()["openai_base_url"],
            "https://b.example/v1"
        );
        assert_eq!(owners(), ["a", "b"]);
        let removed = update_command_settings(&json!({ "api_profiles": [&edited_b] })).unwrap();
        assert_eq!(removed["active_api_profile_id"], "b");
        edited_b["api_key"] = json!("fixture-new-b");
        update_command_settings(&json!({ "api_profiles": [&edited_b] })).unwrap();
        assert_eq!(
            read_auth_value().unwrap()["OPENAI_API_KEY"],
            "fixture-new-b"
        );
        assert_eq!(owners(), ["a", "b"]);

        crate::accounts::write_account_auth(&json!({ "tokens": { "account_id": "fixture" } }))
            .unwrap();
        set_subscription_mode().unwrap();
        update_settings_value(&json!({ "codex_active_mode": "chatgpt" })).unwrap();
        let removed_current_default =
            update_command_settings(&json!({ "api_profiles": [&a] })).unwrap();
        assert_eq!(removed_current_default["active_api_profile_id"], "a");
        assert_eq!(get_codex_state_value()["mode"], "chatgpt");
        assert_eq!(owners(), ["a", "b"]);
    }
}
