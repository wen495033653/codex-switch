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

const DEFAULT_API_TEST_MODEL: &str = "gpt-5.5";

#[tauri::command]
pub(crate) async fn test_api_base_url(
    base_url: String,
    api_key: String,
    model: Option<String>,
) -> Result<Value, String> {
    let base_url = crate::api_config::normalize_api_base_url(&base_url)?;
    let api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        return Err("API Key 不能为空".to_string());
    }
    let test_model = normalize_api_test_model(model);

    let result = tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(12))
            .build()
            .map_err(|err| format!("创建测试请求失败: {err}"))?;

        let models_url = api_test_endpoint(&base_url, "models");
        let models_response = match client.get(&models_url).bearer_auth(&api_key).send() {
            Ok(response) => read_api_test_response(&models_url, response)?,
            Err(err) => {
                return Ok(json!({
                    "ok": false,
                    "stage": "models",
                    "baseUrl": base_url,
                    "message": format!("模型列表请求失败: {err}"),
                    "testModel": test_model,
                    "modelsResponse": api_test_error_response(&models_url, &err.to_string()),
                    "chatResponse": Value::Null,
                    "tokenUsage": Value::Null
                }));
            }
        };
        let models_status = u16_field(&models_response, "status");
        if !(200..300).contains(&models_status) {
            let message = if models_status == 401 || models_status == 403 {
                format!("模型列表认证失败 HTTP {models_status}")
            } else {
                format!("模型列表返回 HTTP {models_status}")
            };
            return Ok(json!({
                "ok": false,
                "stage": "models",
                "baseUrl": base_url,
                "message": message,
                "testModel": test_model,
                "modelsResponse": models_response,
                "chatResponse": Value::Null,
                "tokenUsage": Value::Null
            }));
        }

        let Some(payload) = models_response.get("json").filter(|value| !value.is_null()) else {
            return Ok(json!({
                "ok": false,
                "stage": "models",
                "baseUrl": base_url,
                "message": "模型列表响应不是有效 JSON",
                "testModel": test_model,
                "modelsResponse": models_response,
                "chatResponse": Value::Null,
                "tokenUsage": Value::Null
            }));
        };
        let Some(models) = payload.get("data").and_then(Value::as_array) else {
            return Ok(json!({
                "ok": false,
                "stage": "models",
                "baseUrl": base_url,
                "message": "模型列表响应缺少 data 数组",
                "testModel": test_model,
                "modelsResponse": models_response,
                "chatResponse": Value::Null,
                "tokenUsage": Value::Null
            }));
        };
        if models.is_empty() {
            return Ok(json!({
                "ok": false,
                "stage": "models",
                "baseUrl": base_url,
                "message": "模型列表为空",
                "testModel": test_model,
                "modelsResponse": models_response,
                "chatResponse": Value::Null,
                "tokenUsage": Value::Null
            }));
        }

        let model_ids = api_test_model_ids(models);
        let selected_model = test_model.clone();

        let chat_url = api_test_endpoint(&base_url, "chat/completions");
        let chat_request = json!({
            "model": selected_model,
            "messages": [
                {
                    "role": "user",
                    "content": "Reply with OK."
                }
            ],
            "max_tokens": 8,
            "temperature": 0,
            "stream": false
        });
        let chat_request_snapshot = json!({
            "endpoint": chat_url,
            "body": chat_request
        });
        let chat_response = match client
            .post(&chat_url)
            .bearer_auth(&api_key)
            .json(&chat_request)
            .send()
        {
            Ok(response) => read_api_test_response(&chat_url, response)?,
            Err(err) => {
                return Ok(json!({
                    "ok": false,
                    "stage": "chat",
                    "baseUrl": base_url,
                    "message": format!("Chat 测试请求失败: {err}"),
                    "testModel": selected_model,
                    "modelCount": models.len(),
                    "modelIds": model_ids,
                    "selectedModel": selected_model,
                    "modelsResponse": models_response,
                    "chatRequest": chat_request_snapshot,
                    "chatResponse": api_test_error_response(&chat_url, &err.to_string()),
                    "tokenUsage": Value::Null
                }));
            }
        };
        let chat_status = u16_field(&chat_response, "status");
        if !(200..300).contains(&chat_status) {
            return Ok(json!({
                "ok": false,
                "stage": "chat",
                "baseUrl": base_url,
                "message": format!("Chat 测试返回 HTTP {chat_status}"),
                "testModel": selected_model,
                "modelCount": models.len(),
                "modelIds": model_ids,
                "selectedModel": selected_model,
                "modelsResponse": models_response,
                "chatRequest": chat_request_snapshot,
                "chatResponse": chat_response,
                "tokenUsage": Value::Null
            }));
        }

        let Some(chat_payload) = chat_response.get("json").filter(|value| !value.is_null()) else {
            return Ok(json!({
                "ok": false,
                "stage": "chat",
                "baseUrl": base_url,
                "message": "Chat 测试响应不是有效 JSON",
                "testModel": selected_model,
                "modelCount": models.len(),
                "modelIds": model_ids,
                "selectedModel": selected_model,
                "modelsResponse": models_response,
                "chatRequest": chat_request_snapshot,
                "chatResponse": chat_response,
                "tokenUsage": Value::Null
            }));
        };

        let token_usage = api_test_token_usage(chat_payload);

        Ok(json!({
            "ok": true,
            "stage": "complete",
            "baseUrl": base_url,
            "message": "Base URL 可用，Chat 测试成功",
            "testModel": selected_model,
            "modelCount": models.len(),
            "modelIds": model_ids,
            "selectedModel": selected_model,
            "modelsResponse": models_response,
            "chatRequest": chat_request_snapshot,
            "chatResponse": chat_response,
            "tokenUsage": token_usage
        }))
    })
    .await
    .map_err(|err| format!("等待 Base URL 测试失败: {err}"))??;

    Ok(result)
}

fn normalize_api_test_model(model: Option<String>) -> String {
    let value = model.unwrap_or_default().trim().to_string();
    if value.is_empty() {
        DEFAULT_API_TEST_MODEL.to_string()
    } else {
        value
    }
}

fn api_test_endpoint(base_url: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn read_api_test_response(
    endpoint: &str,
    response: reqwest::blocking::Response,
) -> Result<Value, String> {
    let status = response.status();
    let status_text = status.canonical_reason().unwrap_or("").to_string();
    let body = response
        .text()
        .map_err(|err| format!("读取测试响应失败: {err}"))?;
    let parsed_json = serde_json::from_str::<Value>(&body).ok();

    Ok(json!({
        "endpoint": endpoint,
        "status": status.as_u16(),
        "statusText": status_text,
        "body": body,
        "json": parsed_json
    }))
}

fn api_test_error_response(endpoint: &str, error: &str) -> Value {
    json!({
        "endpoint": endpoint,
        "error": error
    })
}

fn u16_field(value: &Value, key: &str) -> u16 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|raw| u16::try_from(raw).ok())
        .unwrap_or(0)
}

fn api_test_model_ids(models: &[Value]) -> Vec<String> {
    models
        .iter()
        .filter_map(|model| model.get("id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn api_test_token_usage(payload: &Value) -> Value {
    let usage = payload.get("usage").cloned().unwrap_or(Value::Null);
    let prompt_tokens = api_test_token_number(&usage, "prompt_tokens")
        .or_else(|| api_test_token_number(&usage, "input_tokens"));
    let completion_tokens = api_test_token_number(&usage, "completion_tokens")
        .or_else(|| api_test_token_number(&usage, "output_tokens"));
    let total_tokens = api_test_token_number(&usage, "total_tokens").or_else(|| {
        prompt_tokens
            .zip(completion_tokens)
            .map(|(prompt, completion)| prompt + completion)
    });

    json!({
        "totalTokens": total_tokens,
        "promptTokens": prompt_tokens,
        "completionTokens": completion_tokens,
        "raw": usage
    })
}

fn api_test_token_number(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(|raw| {
        raw.as_u64()
            .or_else(|| raw.as_i64().and_then(|num| u64::try_from(num).ok()))
    })
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
