use super::*;

fn codex_session_sync_enabled(settings: &Value) -> bool {
    settings
        .get("codex_session_sync_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn sync_codex_sessions_if_enabled(settings: &Value, target_provider: &str) -> Result<bool, String> {
    if codex_session_sync_enabled(settings) {
        sync_codex_session_index_then_queue_rollouts(target_provider)
    } else {
        Ok(false)
    }
}

pub(super) fn capture_current_impl() -> Result<Value, String> {
    let auth = read_auth_value()?;
    let codex_state = get_codex_state_value();
    if raw_string_field(&codex_state, "mode") == "api" {
        let api_key = read_api_key_from_auth();
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

    set_subscription_mode()?;
    let exchange = exchange_refresh_token(refresh_token)?;
    let account_id = string_field(&exchange, "account_id");
    let access_token = string_field(&exchange, "access_token");
    let account = account_from_exchange_syncing(&exchange, None)?;
    let store = add_account_to_store(account, false)?;
    sync_account_usage_in_background(app, account_id, access_token);
    Ok(store_payload_from_store(
        store,
        Some("已通过 refresh_token 导入账号，正在同步配额"),
    ))
}

pub(super) fn delete_account_impl(id: String) -> Result<Value, String> {
    let account_id = id.trim();
    if account_id.is_empty() {
        return Err("account_id 无效".to_string());
    }
    let store = remove_store_account(account_id)?;
    Ok(store_payload_from_store(store, Some("已删除")))
}

pub(super) fn switch_account_impl(
    app: AppHandle,
    id: String,
    runtime: State<'_, Arc<IdeRuntime>>,
) -> Result<Value, String> {
    let account_id = id.trim();
    if account_id.is_empty() {
        return Err("account_id 无效".to_string());
    }
    let settings = read_settings_value()?;
    let account = find_store_account(account_id)?;
    write_account_auth(&account)?;
    let session_sync_result = sync_codex_sessions_if_enabled(&settings, "openai");
    let store = mark_store_account_used(account_id)?;
    refresh_active_account_usage_in_background(app);
    let ide_reopen =
        build_ide_reopen_payload(runtime.inner().as_ref(), account_id.to_string(), false);
    let message = match session_sync_result {
        Ok(true) => "已切换到订阅模式；会话正在后台同步".to_string(),
        Ok(false) => "已切换到订阅模式".to_string(),
        Err(err) => format!("已切换到订阅模式；会话同步失败：{err}"),
    };
    Ok(attach_ide_reopen(
        store_payload_from_store(store, Some(&message)),
        ide_reopen,
    ))
}

pub(super) fn switch_api_mode_impl(runtime: State<'_, Arc<IdeRuntime>>) -> Result<Value, String> {
    let settings = read_settings_value()?;
    let profile = settings
        .get("api_mode")
        .cloned()
        .unwrap_or_else(default_api_mode);
    set_api_mode(&profile)?;
    let state = get_codex_state_value();
    if raw_string_field(&state, "mode") != "api" {
        return Err("切换失败：Codex 未进入 API 模式".to_string());
    }
    let session_sync_result = sync_codex_sessions_if_enabled(&settings, "api");
    let ide_reopen = build_ide_reopen_payload(runtime.inner().as_ref(), String::new(), true);
    let message = match session_sync_result {
        Ok(true) => "已切换到 API 模式；会话正在后台同步".to_string(),
        Ok(false) => "已切换到 API 模式".to_string(),
        Err(err) => format!("已切换到 API 模式；会话同步失败：{err}"),
    };
    Ok(attach_ide_reopen(
        store_payload(Some(&message))?,
        ide_reopen,
    ))
}
