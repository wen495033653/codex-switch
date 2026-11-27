use super::update_command_settings;
use crate::{
    accounts::store_payload,
    app_log::log_event,
    blocking_task::run_blocking,
    codex_config::{ensure_config_file, remove_config_values, set_config_values},
    codex_launcher::apply_codex_proxy_env_state_to_settings,
    desktop::{sync_system_auto_start, validate_system_auto_start},
    json_util::{bool_field, has_key},
    model_instructions::{
        enabled_in_settings as model_instructions_enabled, prepare_model_instructions_for_enable,
        resolve_model_instructions_file, ModelInstructionsPreparation,
        CONFIG_KEY as MODEL_INSTRUCTIONS_CONFIG_KEY,
    },
    paths::app_data_dir,
    settings::{read_settings_value, update_settings_value},
};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tauri::{path::BaseDirectory, AppHandle, Manager};

#[tauri::command]
pub(crate) async fn get_store() -> Result<Value, String> {
    run_blocking("读取账号", || store_payload(None)).await
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
pub(crate) async fn open_data_dir() -> Result<Value, String> {
    run_blocking("打开数据目录", open_data_dir_impl).await
}

fn open_data_dir_impl() -> Result<Value, String> {
    let path = app_data_dir()?;
    fs::create_dir_all(&path).map_err(|err| format!("创建数据目录失败: {err}"))?;
    open::that(&path).map_err(|err| format!("打开数据目录失败: {err}"))?;
    Ok(json!({
        "ok": true,
        "path": path.to_string_lossy().to_string()
    }))
}

#[tauri::command]
pub(crate) async fn get_settings() -> Result<Value, String> {
    run_blocking("读取设置", || {
        Ok(json!({
            "ok": true,
            "settings": apply_codex_proxy_env_state_to_settings(read_settings_value()?)?
        }))
    })
    .await
}

#[tauri::command]
pub(crate) async fn update_settings(app: AppHandle, patch: Value) -> Result<Value, String> {
    run_blocking("保存设置", move || update_settings_impl(&app, &patch)).await
}

fn update_settings_impl(app: &AppHandle, patch: &Value) -> Result<Value, String> {
    let should_sync_auto_start =
        has_key(patch, "auto_start") || has_key(patch, "auto_start_launch_mode");
    let desired_auto_start = if should_sync_auto_start {
        Some(if has_key(patch, "auto_start") {
            bool_field(patch, "auto_start")
        } else {
            bool_field(&read_settings_value()?, "auto_start")
        })
    } else {
        None
    };
    if let Some(enabled) = desired_auto_start {
        validate_system_auto_start(enabled)?;
    }
    let settings = apply_codex_proxy_env_state_to_settings(update_command_settings(patch)?)?;
    if let Some(enabled) = desired_auto_start {
        sync_system_auto_start(app, enabled)?;
    }
    Ok(json!({
        "ok": true,
        "message": "设置已保存",
        "settings": settings
    }))
}

#[tauri::command]
pub(crate) async fn set_codex_model_instructions_enabled(
    app: AppHandle,
    enabled: bool,
    overwrite_local: Option<bool>,
) -> Result<Value, String> {
    run_blocking("切换模型指令", move || {
        set_codex_model_instructions_enabled_impl(&app, enabled, overwrite_local)
    })
    .await
}

fn set_codex_model_instructions_enabled_impl(
    app: &AppHandle,
    enabled: bool,
    overwrite_local: Option<bool>,
) -> Result<Value, String> {
    let backup_path = if enabled {
        let preparation =
            prepare_model_instructions_for_enable(app, overwrite_local).map_err(|err| {
                log_event(
                    "model_instructions_prepare_error",
                    json!({ "enabled": enabled, "overwriteLocal": overwrite_local, "error": err }),
                );
                err
            })?;
        match preparation {
            ModelInstructionsPreparation::Confirmation {
                path,
                exists,
                backup_path,
            } => {
                return Ok(json!({
                    "ok": false,
                    "confirmationRequired": true,
                    "path": path,
                    "exists": exists,
                    "backupPath": backup_path
                }));
            }
            ModelInstructionsPreparation::Ready { path, backup_path } => {
                set_config_values(vec![(MODEL_INSTRUCTIONS_CONFIG_KEY, path)]).map_err(|err| {
                    format!("保存模型指令配置失败: {err}；本地文件备份: {backup_path:?}")
                })?;
                backup_path
            }
        }
    } else {
        sync_codex_model_instructions_config(app, false)?;
        None
    };

    let settings = apply_codex_proxy_env_state_to_settings(update_settings_value(&json!({
        "codex_model_instructions_enabled": enabled
    }))?)?;
    let message = if enabled {
        "GPT 破甲已替换，重启 Codex 后生效。"
    } else {
        "GPT 破甲已关闭，重启 Codex 后生效。"
    };

    Ok(json!({
        "ok": true,
        "message": message,
        "restartRequired": true,
        "backupPath": backup_path,
        "settings": settings
    }))
}

pub(crate) fn sync_codex_model_instructions_config_for_current_settings(
    app: &AppHandle,
) -> Result<(), String> {
    let settings = read_settings_value()?;
    sync_codex_model_instructions_config(app, model_instructions_enabled(&settings))
}

fn sync_codex_model_instructions_config(app: &AppHandle, enabled: bool) -> Result<(), String> {
    if enabled {
        let path = resolve_model_instructions_file(app)?;
        set_config_values(vec![(MODEL_INSTRUCTIONS_CONFIG_KEY, path)])
    } else {
        remove_config_values(&[MODEL_INSTRUCTIONS_CONFIG_KEY])
    }
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
pub(crate) async fn open_external_url(url: String) -> Result<Value, String> {
    run_blocking("打开外部链接", move || open_external_url_impl(&url)).await
}

fn open_external_url_impl(url: &str) -> Result<Value, String> {
    let target = url.trim();
    if !(target.starts_with("https://") || target.starts_with("http://")) {
        return Err("外部链接仅支持 http/https".to_string());
    }
    open::that(target).map_err(|err| format!("打开外部链接失败: {err}"))?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub(crate) async fn open_codex_config_toml() -> Result<Value, String> {
    run_blocking("打开 config.toml", || {
        let path = ensure_config_file()?;
        open::that(&path).map_err(|err| format!("打开 config.toml 失败: {err}"))?;
        Ok(json!({
            "ok": true,
            "path": path.to_string_lossy().to_string()
        }))
    })
    .await
}

#[tauri::command]
pub(crate) async fn list_brand_voice_files(app: AppHandle) -> Result<Value, String> {
    run_blocking("读取语音包", move || {
        // The pack ships with the app; a directory that cannot be found or read is a broken
        // install, not an empty pack.
        let dir = app
            .path()
            .resolve("voice-pack", BaseDirectory::Resource)
            .map_err(|err| format!("定位语音包目录失败: {err}"))?;
        let files = collect_mp3_files(&dir)?;

        Ok(json!({
            "ok": true,
            "files": files
        }))
    })
    .await
}

fn collect_mp3_files(dir: &Path) -> Result<Vec<String>, String> {
    let read_error = |err: std::io::Error| format!("读取语音包目录失败 {}: {err}", dir.display());
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(read_error)? {
        let path = entry.map_err(read_error)?.path();
        let is_mp3 = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("mp3"));
        if is_mp3 && path.is_file() {
            files.push(path.to_string_lossy().to_string());
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod voice_pack_tests {
    use super::*;

    #[test]
    fn voice_files_are_listed_and_a_missing_pack_is_an_error() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("codex-switch-voice-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("b.mp3"), b"").unwrap();
        fs::write(dir.join("a.MP3"), b"").unwrap();
        fs::write(dir.join("notes.txt"), b"").unwrap();

        let files = collect_mp3_files(&dir).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        let missing = collect_mp3_files(&dir).unwrap_err();

        let names = files
            .iter()
            .map(|file| {
                Path::new(file)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(names, ["a.MP3", "b.mp3"]);
        assert!(missing.contains("读取语音包目录失败"), "{missing}");
    }
}
