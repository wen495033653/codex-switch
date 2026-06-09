use super::*;
use std::path::Path;
use tauri::{path::BaseDirectory, Manager};

#[tauri::command]
pub(crate) fn get_store() -> Result<Value, String> {
    store_payload(None)
}

#[tauri::command]
pub(crate) fn get_app_version() -> Value {
    let mut version = env!("CARGO_PKG_VERSION").to_string();
    if cfg!(debug_assertions) {
        version.push_str("-dev");
    }
    json!({
        "ok": true,
        "version": version
    })
}

#[tauri::command]
pub(crate) fn get_data_dir() -> Result<Value, String> {
    Ok(json!({
        "ok": true,
        "path": app_data_dir()?.to_string_lossy().to_string()
    }))
}

#[tauri::command]
pub(crate) fn open_data_dir() -> Result<Value, String> {
    let path = app_data_dir()?;
    fs::create_dir_all(&path).map_err(|err| format!("创建数据目录失败: {err}"))?;
    open::that(&path).map_err(|err| format!("打开数据目录失败: {err}"))?;
    Ok(json!({
        "ok": true,
        "path": path.to_string_lossy().to_string()
    }))
}

#[tauri::command]
pub(crate) fn get_settings() -> Result<Value, String> {
    Ok(json!({
        "ok": true,
        "settings": apply_codex_proxy_env_state_to_settings(read_settings_value()?)?
    }))
}

#[tauri::command]
pub(crate) fn update_settings(app: AppHandle, patch: Value) -> Result<Value, String> {
    let should_sync_auto_start =
        has_key(&patch, "auto_start") || has_key(&patch, "auto_start_launch_mode");
    let should_apply_api_mode = has_key(&patch, "api_mode");
    let desired_auto_start = if should_sync_auto_start {
        Some(if has_key(&patch, "auto_start") {
            bool_field(&patch, "auto_start")
        } else {
            bool_field(&read_settings_value()?, "auto_start")
        })
    } else {
        None
    };
    if let Some(enabled) = desired_auto_start {
        validate_system_auto_start(enabled)?;
    }
    let settings = apply_codex_proxy_env_state_to_settings(update_settings_value(&patch)?)?;
    if should_apply_api_mode {
        apply_complete_api_mode_profile_if_active(&settings)?;
    }
    if let Some(enabled) = desired_auto_start {
        sync_system_auto_start(&app, enabled)?;
    }
    Ok(json!({
        "ok": true,
        "message": "设置已保存",
        "settings": settings
    }))
}

#[tauri::command]
pub(crate) fn copy_text(text: String) -> Result<Value, String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|err| format!("打开剪贴板失败: {err}"))?;
    clipboard
        .set_text(text)
        .map_err(|err| format!("写入剪贴板失败: {err}"))?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub(crate) fn open_external_url(url: String) -> Result<Value, String> {
    let target = url.trim();
    if !(target.starts_with("https://") || target.starts_with("http://")) {
        return Err("外部链接仅支持 http/https".to_string());
    }
    open::that(target).map_err(|err| format!("打开外部链接失败: {err}"))?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub(crate) async fn test_api_base_url(base_url: String, api_key: String) -> Result<Value, String> {
    let base_url = crate::api_config::normalize_api_base_url(&base_url)?;
    let api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        return Err("API Key 不能为空".to_string());
    }
    let test_url = format!("{}/models", base_url.trim_end_matches('/'));

    let result = tauri::async_runtime::spawn_blocking(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(12))
            .build()
            .map_err(|err| format!("创建测试请求失败: {err}"))?;
        let response = client
            .get(&test_url)
            .bearer_auth(api_key)
            .send()
            .map_err(|err| format!("模型列表请求失败: {err}"))?;
        let status = response.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(format!("模型列表认证失败 HTTP {}", status.as_u16()));
        }
        if !status.is_success() {
            return Err(format!("模型列表返回 HTTP {}", status.as_u16()));
        }

        let body = response
            .text()
            .map_err(|err| format!("读取模型列表失败: {err}"))?;
        let payload: Value = serde_json::from_str(&body)
            .map_err(|err| format!("模型列表响应不是有效 JSON: {err}"))?;
        let models = payload
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| "模型列表响应缺少 data 数组".to_string())?;
        if models.is_empty() {
            return Err("模型列表为空".to_string());
        }

        Ok(json!({
            "ok": true,
            "message": format!("Base URL 可用，模型 {} 个", models.len())
        }))
    })
    .await
    .map_err(|err| format!("等待 Base URL 测试失败: {err}"))??;

    Ok(result)
}

#[tauri::command]
pub(crate) fn open_codex_config_toml() -> Result<Value, String> {
    let path = ensure_config_file()?;
    open::that(&path).map_err(|err| format!("打开 config.toml 失败: {err}"))?;
    Ok(json!({
        "ok": true,
        "path": path.to_string_lossy().to_string()
    }))
}

#[tauri::command]
pub(crate) fn list_brand_voice_files(app: AppHandle) -> Value {
    let files = app
        .path()
        .resolve("voice-pack", BaseDirectory::Resource)
        .ok()
        .map(|dir| collect_mp3_files(&dir))
        .unwrap_or_default();

    json!({
        "ok": true,
        "files": files
    })
}

fn collect_mp3_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut files: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("mp3"))
        })
        .map(|path| path.to_string_lossy().to_string())
        .collect();

    files.sort();
    files
}
