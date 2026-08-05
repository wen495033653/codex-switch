use super::*;

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
        let provider_key = read_api_key_from_provider_config();
        let api_key = if provider_key.is_empty() {
            read_api_key_from_auth()
        } else {
            provider_key
        };
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

    update_settings_value(&json!({ "codex_active_mode": "chatgpt" }))?;
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
    runtime: State<'_, Arc<IdeRuntime>>,
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
    let session_sync_enabled = codex_session_sync_enabled(&settings);
    let ide_reopen = build_ide_reopen_payload(
        runtime.inner().as_ref(),
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
        eprintln!("记录订阅 token 统计归属失败: {err}");
    }
    refresh_active_account_usage_in_background(app);
    Ok(attach_ide_reopen(
        store_payload_from_store(store, Some(&message)),
        ide_reopen,
    ))
}

pub(super) fn switch_api_mode_impl(
    runtime: State<'_, Arc<IdeRuntime>>,
    profile_id: Option<String>,
) -> Result<Value, String> {
    let requested_profile_id = profile_id.unwrap_or_default().trim().to_string();
    let mut settings = read_settings_value()?;
    if !requested_profile_id.is_empty() {
        let exists = settings
            .get("api_profiles")
            .and_then(Value::as_array)
            .is_some_and(|profiles| {
                profiles
                    .iter()
                    .any(|profile| string_field(profile, "id") == requested_profile_id)
            });
        if !exists {
            return Err("API 配置不存在".to_string());
        }
        settings = update_settings_value(&json!({
            "active_api_profile_id": requested_profile_id
        }))?;
    }
    let profile = settings
        .get("api_mode")
        .cloned()
        .unwrap_or_else(default_api_mode);
    if string_field(&profile, "base_url").is_empty() {
        return Err("API Base URL 不能为空".to_string());
    }
    let active_profile_id = string_field(&profile, "id");
    set_api_mode(&profile)?;
    let state = get_codex_state_value();
    if raw_string_field(&state, "mode") != "api" {
        return Err("切换失败：Codex 未进入 API 模式".to_string());
    }
    let settings_before_sync = update_settings_value(&json!({ "codex_active_mode": "api" }))?;
    if bool_field(&settings_before_sync, "codex_remote_control_enabled") {
        sync_remote_control_runtime_for_current_settings("switch_api_mode")?;
    }
    let settings = read_settings_value()?;
    let fallback_applied =
        missing_remote_control_account_fallback_applied(&settings_before_sync, &settings);
    if let Err(err) =
        crate::usage_stats::record_attribution("api_profile", &active_profile_id, "api")
    {
        eprintln!("记录 API token 统计归属失败: {err}");
    }
    let session_sync_enabled = codex_session_sync_enabled(&settings);
    let ide_reopen = build_ide_reopen_payload(
        runtime.inner().as_ref(),
        active_profile_id.clone(),
        true,
        session_sync_enabled.then(|| "api".to_string()),
    );
    let message = if fallback_applied && session_sync_enabled && ide_reopen.is_some() {
        "远程控制账号不存在，已关闭远程控制并切换到 API 模式；重新打开 IDE 前会同步会话".to_string()
    } else if fallback_applied {
        "远程控制账号不存在，已关闭远程控制并切换到 API 模式".to_string()
    } else if session_sync_enabled && ide_reopen.is_some() {
        "已切换到 API 模式；重新打开 IDE 前会同步会话".to_string()
    } else {
        "已切换到 API 模式".to_string()
    };
    Ok(attach_settings(
        attach_ide_reopen(store_payload(Some(&message))?, ide_reopen),
        settings,
    ))
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
}
