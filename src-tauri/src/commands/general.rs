use super::apply_complete_api_mode_profile_if_active;
use crate::{
    accounts::{http_error_message, store_payload},
    app_log::{log_event_once, truncate_for_log},
    blocking_task::run_blocking,
    codex_config::ensure_config_file,
    codex_launcher::apply_codex_proxy_env_state_to_settings,
    desktop::{sync_system_auto_start, validate_system_auto_start},
    json_util::{bool_field, has_key},
    paths::app_data_dir,
    settings::{read_settings_value, update_settings_value},
};
use crate::{
    codex_config::{remove_config_values, set_config_values},
    model_instructions::{
        prepare_model_instructions_for_enable, resolve_model_instructions_file,
        ModelInstructionsPreparation, CONFIG_KEY as MODEL_INSTRUCTIONS_CONFIG_KEY,
        SETTING_KEY as MODEL_INSTRUCTIONS_SETTING_KEY,
    },
};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tauri::AppHandle;
use tauri::{path::BaseDirectory, Manager};

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
    let should_apply_api_mode = has_key(patch, "api_mode");
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
    let mut settings = apply_codex_proxy_env_state_to_settings(update_settings_value(patch)?)?;
    if should_apply_api_mode {
        apply_complete_api_mode_profile_if_active(&settings)?;
        // The remote-control sync may have turned remote control off (its account is gone).
        settings = apply_codex_proxy_env_state_to_settings(read_settings_value()?)?;
    }
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
                crate::app_log::log_event(
                    "model_instructions_prepare_error",
                    json!({ "enabled": enabled, "overwriteLocal": overwrite_local, "error": err }),
                );
                err
            })?;
        match preparation {
            ModelInstructionsPreparation::Confirmation { path } => {
                return Ok(json!({
                    "ok": false,
                    "confirmationRequired": true,
                    "path": path
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
        "gpt破限已启用，重启 Codex 后生效。"
    } else {
        "gpt破限已关闭，重启 Codex 后生效。"
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

fn model_instructions_enabled(settings: &Value) -> bool {
    settings
        .get(MODEL_INSTRUCTIONS_SETTING_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(true)
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

const DEFAULT_API_TEST_MODEL: &str = "gpt-6.1-sol";

/// The most output one test question may ask for (the pelican SVG is the largest).
const API_PROMPT_MAX_OUTPUT_TOKENS: u32 = 64_000;

/// A reasoning model may think for minutes on a test question.
const API_PROMPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

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

    run_blocking("测试 Base URL", move || -> Result<Value, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(12))
            .build()
            .map_err(|err| format!("创建测试请求失败: {err}"))?;

        let models_url = api_test_endpoint(&base_url, "models");
        let models_response = match client.get(&models_url).bearer_auth(&api_key).send() {
            Ok(response) => read_api_test_response(&models_url, response)?,
            Err(err) => {
                // The cause chain tells DNS, TLS, proxy and timeout failures apart.
                let error = http_error_message(err);
                return Ok(json!({
                    "ok": false,
                    "stage": "models",
                    "baseUrl": base_url,
                    "message": format!("模型列表请求失败: {error}"),
                    "testModel": test_model,
                    "modelsResponse": api_test_error_response(&models_url, &error),
                    "responsesRequest": Value::Null,
                    "responsesResponse": Value::Null,
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
                "responsesRequest": Value::Null,
                "responsesResponse": Value::Null,
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
                "responsesRequest": Value::Null,
                "responsesResponse": Value::Null,
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
                "responsesRequest": Value::Null,
                "responsesResponse": Value::Null,
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
                "responsesRequest": Value::Null,
                "responsesResponse": Value::Null,
                "tokenUsage": Value::Null
            }));
        }

        let model_ids = api_test_model_ids(models);
        let selected_model = test_model.clone();

        let responses_url = api_test_endpoint(&base_url, "responses");
        let responses_request = json!({
            "model": selected_model,
            "input": "Reply with OK.",
            "max_output_tokens": 128,
            "stream": false
        });
        let responses_request_snapshot = json!({
            "endpoint": responses_url,
            "body": responses_request
        });
        let responses_response = match client
            .post(&responses_url)
            .bearer_auth(&api_key)
            .json(&responses_request)
            .send()
        {
            Ok(response) => read_api_test_response(&responses_url, response)?,
            Err(err) => {
                let error = http_error_message(err);
                return Ok(json!({
                    "ok": false,
                    "stage": "responses",
                    "baseUrl": base_url,
                    "message": format!("Responses 测试请求失败: {error}"),
                    "testModel": selected_model,
                    "modelCount": models.len(),
                    "modelIds": model_ids,
                    "selectedModel": selected_model,
                    "modelsResponse": models_response,
                    "responsesRequest": responses_request_snapshot,
                    "responsesResponse": api_test_error_response(&responses_url, &error),
                    "tokenUsage": Value::Null
                }));
            }
        };
        let responses_status = u16_field(&responses_response, "status");
        if !(200..300).contains(&responses_status) {
            return Ok(json!({
                "ok": false,
                "stage": "responses",
                "baseUrl": base_url,
                "message": format!("Responses 测试返回 HTTP {responses_status}"),
                "testModel": selected_model,
                "modelCount": models.len(),
                "modelIds": model_ids,
                "selectedModel": selected_model,
                "modelsResponse": models_response,
                "responsesRequest": responses_request_snapshot,
                "responsesResponse": responses_response,
                "tokenUsage": Value::Null
            }));
        }

        let Some(responses_payload) = responses_response
            .get("json")
            .filter(|value| !value.is_null())
        else {
            return Ok(json!({
                "ok": false,
                "stage": "responses",
                "baseUrl": base_url,
                "message": "Responses 测试响应不是有效 JSON",
                "testModel": selected_model,
                "modelCount": models.len(),
                "modelIds": model_ids,
                "selectedModel": selected_model,
                "modelsResponse": models_response,
                "responsesRequest": responses_request_snapshot,
                "responsesResponse": responses_response,
                "tokenUsage": Value::Null
            }));
        };

        let token_usage = api_test_token_usage(responses_payload);

        Ok(json!({
            "ok": true,
            "stage": "complete",
            "baseUrl": base_url,
            "message": "Base URL 可用，Responses 测试成功",
            "testModel": selected_model,
            "modelCount": models.len(),
            "modelIds": model_ids,
            "selectedModel": selected_model,
            "modelsResponse": models_response,
            "responsesRequest": responses_request_snapshot,
            "responsesResponse": responses_response,
            "tokenUsage": token_usage
        }))
    })
    .await
}

/// Sends one test question to the profile's Responses endpoint with the test model and returns
/// what came back: the answer text, the token usage and how long it took. The questions and
/// their grading live in the page (`utils/apiIntelligence.js`).
#[tauri::command]
pub(crate) async fn test_api_prompt(
    base_url: String,
    api_key: String,
    model: Option<String>,
    input: String,
    max_output_tokens: u32,
) -> Result<Value, String> {
    let base_url = crate::api_config::normalize_api_base_url(&base_url)?;
    let api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        return Err("API Key 不能为空".to_string());
    }
    if input.trim().is_empty() {
        return Err("测试题不能为空".to_string());
    }
    if max_output_tokens == 0 || max_output_tokens > API_PROMPT_MAX_OUTPUT_TOKENS {
        return Err(format!(
            "测试题的输出上限必须在 1 到 {API_PROMPT_MAX_OUTPUT_TOKENS} 之间: {max_output_tokens}"
        ));
    }
    let model = normalize_api_test_model(model);
    run_blocking("发送测试题", move || {
        api_prompt_test(&base_url, &api_key, &model, &input, max_output_tokens)
    })
    .await
}

fn api_prompt_test(
    base_url: &str,
    api_key: &str,
    model: &str,
    input: &str,
    max_output_tokens: u32,
) -> Result<Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(API_PROMPT_TIMEOUT)
        .build()
        .map_err(|err| format!("创建测试请求失败: {err}"))?;
    let endpoint = api_test_endpoint(base_url, "responses");
    let body = json!({
        "model": model,
        "input": input,
        "max_output_tokens": max_output_tokens,
        "stream": false
    });
    let started = std::time::Instant::now();
    let sent = client
        .post(&endpoint)
        .bearer_auth(api_key)
        .json(&body)
        .send();
    let duration_ms = |started: std::time::Instant| {
        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
    };
    let response = match sent {
        Ok(response) => read_api_test_response(&endpoint, response)?,
        Err(err) => {
            let error = http_error_message(err);
            log_event_once(
                "api_prompt_test_error",
                json!({
                    "endpoint": endpoint,
                    "model": model,
                    "error": error,
                    "handling": "shown_in_result"
                }),
            );
            return Ok(json!({
                "ok": false,
                "model": model,
                "durationMs": duration_ms(started),
                "error": format!("测试请求失败: {error}")
            }));
        }
    };
    let duration_ms = duration_ms(started);
    let status = u16_field(&response, "status");
    let payload = response.get("json").filter(|value| !value.is_null());
    let Some(payload) = payload.filter(|_| (200..300).contains(&status)) else {
        let body_text = response.get("body").and_then(Value::as_str).unwrap_or("");
        let error = if (200..300).contains(&status) {
            "测试响应不是有效 JSON".to_string()
        } else {
            format!("测试请求返回 HTTP {status}")
        };
        log_event_once(
            "api_prompt_test_error",
            json!({
                "endpoint": endpoint,
                "model": model,
                "status": status,
                "body": truncate_for_log(body_text, 300),
                "handling": "shown_in_result"
            }),
        );
        return Ok(json!({
            "ok": false,
            "model": model,
            "durationMs": duration_ms,
            "httpStatus": status,
            "error": error,
            "body": truncate_for_log(body_text, 300)
        }));
    };
    let usage = payload.get("usage").unwrap_or(&Value::Null);
    Ok(json!({
        "ok": true,
        "model": model,
        "durationMs": duration_ms,
        "httpStatus": status,
        "responseModel": payload.get("model"),
        "responseStatus": payload.get("status"),
        "incompleteReason": payload.pointer("/incomplete_details/reason"),
        "outputText": response_output_text(payload),
        "usage": {
            "inputTokens": api_test_token_number(usage, "input_tokens"),
            "outputTokens": api_test_token_number(usage, "output_tokens"),
            "reasoningTokens": usage
                .get("output_tokens_details")
                .and_then(|details| api_test_token_number(details, "reasoning_tokens"))
        }
    }))
}

/// The answer text: every `output_text` part of the `message` items in `output`.
fn response_output_text(payload: &Value) -> String {
    let parts = payload
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("output_text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>();
    parts.join("\n")
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
mod api_test_model_tests {
    use super::*;

    #[test]
    fn defaults_to_gpt6_without_overriding_explicit_models() {
        assert_eq!(normalize_api_test_model(None), "gpt-6.1-sol");
        assert_eq!(normalize_api_test_model(Some("  ".into())), "gpt-6.1-sol");
        assert_eq!(
            normalize_api_test_model(Some(" custom-model ".into())),
            "custom-model"
        );
    }
}

#[cfg(test)]
mod api_prompt_tests {
    use super::*;

    #[test]
    fn the_answer_is_every_output_text_part_of_the_messages() {
        let payload = json!({
            "output": [
                { "type": "reasoning", "summary": [] },
                {
                    "type": "message",
                    "content": [
                        { "type": "output_text", "text": "答案：29" },
                        { "type": "refusal", "refusal": "no" }
                    ]
                },
                { "type": "message", "content": [{ "type": "output_text", "text": "second" }] }
            ]
        });
        assert_eq!(response_output_text(&payload), "答案：29\nsecond");
        assert_eq!(response_output_text(&json!({ "output": [] })), "");
        assert_eq!(response_output_text(&json!({})), "");
    }
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
