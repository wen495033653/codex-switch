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
    let settings = apply_codex_proxy_env_state_to_settings(update_settings_value(&patch)?)?;
    if should_sync_auto_start {
        sync_system_auto_start(&app, bool_field(&settings, "auto_start"))?;
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
