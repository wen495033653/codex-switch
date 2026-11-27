use crate::{
    accounts::{
        access_token_from_account, account_id_from_account, find_store_account, get_codex_models,
        http_error_message, post_codex_prompt, INTERACTIVE_REQUEST_TIMEOUT_MS,
    },
    api_config::normalize_api_base_url,
    app_log::{account_label, log_event_once, truncate_for_log},
    blocking_task::run_blocking,
    codex_app_server::codex_desktop_cli_version,
};
use serde_json::{json, Value};

const DEFAULT_API_TEST_MODEL: &str = "gpt-6.2-sol";

/// The most output one test question may ask for (the pelican SVG is the largest).
const API_PROMPT_MAX_OUTPUT_TOKENS: u32 = 64_000;

/// A reasoning model may think for minutes on a test question.
const PROMPT_TEST_TIMEOUT_MS: u64 = 300_000;

#[tauri::command]
pub(crate) async fn test_api_base_url(
    base_url: String,
    api_key: String,
    model: Option<String>,
) -> Result<Value, String> {
    let base_url = normalize_api_base_url(&base_url)?;
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
                return Ok(models_stage_failure(
                    &base_url,
                    &test_model,
                    format!("模型列表请求失败: {error}"),
                    api_test_error_response(&models_url, &error),
                ));
            }
        };
        let models_status = u16_field(&models_response, "status");
        if !(200..300).contains(&models_status) {
            let message = if models_status == 401 || models_status == 403 {
                format!("模型列表认证失败 HTTP {models_status}")
            } else {
                format!("模型列表返回 HTTP {models_status}")
            };
            return Ok(models_stage_failure(
                &base_url,
                &test_model,
                message,
                models_response,
            ));
        }

        let Some(payload) = models_response.get("json").filter(|value| !value.is_null()) else {
            return Ok(models_stage_failure(
                &base_url,
                &test_model,
                "模型列表响应不是有效 JSON".to_string(),
                models_response,
            ));
        };
        let Some(models) = payload.get("data").and_then(Value::as_array) else {
            return Ok(models_stage_failure(
                &base_url,
                &test_model,
                "模型列表响应缺少 data 数组".to_string(),
                models_response,
            ));
        };
        if models.is_empty() {
            return Ok(models_stage_failure(
                &base_url,
                &test_model,
                "模型列表为空".to_string(),
                models_response,
            ));
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
                return Ok(responses_stage_failure(
                    &base_url,
                    &selected_model,
                    models.len(),
                    &model_ids,
                    &models_response,
                    &responses_request_snapshot,
                    format!("Responses 测试请求失败: {error}"),
                    api_test_error_response(&responses_url, &error),
                ));
            }
        };
        let responses_status = u16_field(&responses_response, "status");
        if !(200..300).contains(&responses_status) {
            return Ok(responses_stage_failure(
                &base_url,
                &selected_model,
                models.len(),
                &model_ids,
                &models_response,
                &responses_request_snapshot,
                format!("Responses 测试返回 HTTP {responses_status}"),
                responses_response,
            ));
        }

        let Some(responses_payload) = responses_response
            .get("json")
            .filter(|value| !value.is_null())
        else {
            return Ok(responses_stage_failure(
                &base_url,
                &selected_model,
                models.len(),
                &model_ids,
                &models_response,
                &responses_request_snapshot,
                "Responses 测试响应不是有效 JSON".to_string(),
                responses_response,
            ));
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

fn models_stage_failure(
    base_url: &str,
    test_model: &str,
    message: String,
    models_response: Value,
) -> Value {
    json!({
        "ok": false,
        "stage": "models",
        "baseUrl": base_url,
        "message": message,
        "testModel": test_model,
        "modelsResponse": models_response,
        "responsesRequest": Value::Null,
        "responsesResponse": Value::Null,
        "tokenUsage": Value::Null
    })
}

#[allow(clippy::too_many_arguments)]
fn responses_stage_failure(
    base_url: &str,
    selected_model: &str,
    model_count: usize,
    model_ids: &[String],
    models_response: &Value,
    responses_request_snapshot: &Value,
    message: String,
    responses_response: Value,
) -> Value {
    json!({
        "ok": false,
        "stage": "responses",
        "baseUrl": base_url,
        "message": message,
        "testModel": selected_model,
        "modelCount": model_count,
        "modelIds": model_ids,
        "selectedModel": selected_model,
        "modelsResponse": models_response,
        "responsesRequest": responses_request_snapshot,
        "responsesResponse": responses_response,
        "tokenUsage": Value::Null
    })
}

/// Sends one test question to the profile's Responses endpoint with the test model and returns
/// what came back: the answer text, the token usage and how long it took. The questions and
/// their grading live in the page (`utils/intelligenceTests.js`).
#[tauri::command]
pub(crate) async fn test_api_prompt(
    base_url: String,
    api_key: String,
    model: Option<String>,
    input: String,
    max_output_tokens: u32,
) -> Result<Value, String> {
    let base_url = normalize_api_base_url(&base_url)?;
    let api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        return Err("API Key 不能为空".to_string());
    }
    validate_prompt_test(&input, max_output_tokens)?;
    let model = normalize_api_test_model(model);
    run_blocking("发送测试题", move || {
        api_prompt_test(&base_url, &api_key, &model, &input, max_output_tokens)
    })
    .await
}

/// The same test question sent with a subscription account, through the Codex backend Codex
/// itself uses with that login. Returns what `test_api_prompt` returns.
#[tauri::command]
pub(crate) async fn test_subscription_prompt(
    profile_id: String,
    model: Option<String>,
    input: String,
    max_output_tokens: u32,
) -> Result<Value, String> {
    validate_prompt_test(&input, max_output_tokens)?;
    let model = normalize_api_test_model(model);
    run_blocking("发送测试题", move || {
        subscription_prompt_test(profile_id.trim(), &model, &input, max_output_tokens)
    })
    .await
}

/// The models a subscription account can test: what Codex Desktop's own CLI version gets listed
/// for that login, without the hidden ones.
#[tauri::command]
pub(crate) async fn list_subscription_models(profile_id: String) -> Result<Value, String> {
    run_blocking("读取订阅模型列表", move || {
        subscription_models(profile_id.trim())
    })
    .await
}

fn subscription_models(profile_id: &str) -> Result<Value, String> {
    let account = find_store_account(profile_id)?;
    let account_id = account_id_from_account(&account)?;
    let access_token = access_token_from_account(&account);
    if access_token.is_empty() {
        return Err("账号缺少 access_token".to_string());
    }
    let log_failure = |stage: &str, error: &str| {
        log_event_once(
            "subscription_models_error",
            json!({
                "account": account_label(profile_id),
                "stage": stage,
                "error": error,
                "handling": "shown_in_dialog"
            }),
        );
    };
    let client_version = match codex_desktop_cli_version() {
        Ok(version) => version,
        Err(error) => {
            log_failure("codex_desktop_cli_version", &error);
            return Ok(json!({
                "ok": false,
                "cliUnavailable": true,
                "error": error
            }));
        }
    };
    let models = match get_codex_models(
        &access_token,
        &account_id,
        &client_version,
        INTERACTIVE_REQUEST_TIMEOUT_MS,
    ) {
        Ok(models) => models,
        Err(error) => {
            log_failure("get_codex_models", &error);
            return Ok(json!({
                "ok": false,
                "clientVersion": client_version,
                "error": error
            }));
        }
    };
    let mut visible = models
        .iter()
        .filter(|model| model.get("visibility").and_then(Value::as_str) == Some("list"))
        .filter_map(|model| {
            let slug = model.get("slug").and_then(Value::as_str)?;
            let priority = model
                .get("priority")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MAX);
            Some((priority, slug.to_string()))
        })
        .collect::<Vec<_>>();
    visible.sort();
    Ok(json!({
        "ok": true,
        "clientVersion": client_version,
        "models": visible.into_iter().map(|(_, slug)| slug).collect::<Vec<_>>()
    }))
}

fn validate_prompt_test(input: &str, max_output_tokens: u32) -> Result<(), String> {
    if input.trim().is_empty() {
        return Err("测试题不能为空".to_string());
    }
    if max_output_tokens == 0 || max_output_tokens > API_PROMPT_MAX_OUTPUT_TOKENS {
        return Err(format!(
            "测试题的输出上限必须在 1 到 {API_PROMPT_MAX_OUTPUT_TOKENS} 之间: {max_output_tokens}"
        ));
    }
    Ok(())
}

fn elapsed_ms(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn api_prompt_test(
    base_url: &str,
    api_key: &str,
    model: &str,
    input: &str,
    max_output_tokens: u32,
) -> Result<Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_millis(PROMPT_TEST_TIMEOUT_MS))
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
                "durationMs": elapsed_ms(started),
                "error": format!("测试请求失败: {error}")
            }));
        }
    };
    let duration_ms = elapsed_ms(started);
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
    Ok(prompt_test_answer(model, duration_ms, status, payload))
}

fn subscription_prompt_test(
    profile_id: &str,
    model: &str,
    input: &str,
    max_output_tokens: u32,
) -> Result<Value, String> {
    let account = find_store_account(profile_id)?;
    let account_id = account_id_from_account(&account)?;
    let access_token = access_token_from_account(&account);
    if access_token.is_empty() {
        return Err("账号缺少 access_token".to_string());
    }
    let failure = SubscriptionPromptFailure { profile_id, model };
    let started = std::time::Instant::now();
    let sent = post_codex_prompt(
        &access_token,
        &account_id,
        model,
        input,
        max_output_tokens,
        PROMPT_TEST_TIMEOUT_MS,
    );
    let response = match sent {
        Ok(response) => response,
        Err(err) => {
            let error = format!("测试请求失败: {}", http_error_message(err));
            return Ok(failure.result(elapsed_ms(started), None, error, None));
        }
    };
    let status = response.status().as_u16();
    let body = match response.text() {
        Ok(body) => body,
        Err(err) => {
            let error = format!("读取测试响应失败: {}", http_error_message(err));
            return Ok(failure.result(elapsed_ms(started), Some(status), error, None));
        }
    };
    let duration_ms = elapsed_ms(started);
    if !(200..300).contains(&status) {
        let error = format!("测试请求返回 HTTP {status}");
        return Ok(failure.result(duration_ms, Some(status), error, Some(&body)));
    }
    let payload = match codex_stream_final_response(&body) {
        Ok(payload) => payload,
        Err(error) => return Ok(failure.result(duration_ms, Some(status), error, None)),
    };
    if payload.get("status").and_then(Value::as_str) == Some("failed") {
        let detail = payload
            .get("error")
            .map(Value::to_string)
            .unwrap_or_default();
        let error = "测试回答失败".to_string();
        return Ok(failure.result(duration_ms, Some(status), error, Some(&detail)));
    }
    Ok(prompt_test_answer(model, duration_ms, status, &payload))
}

struct SubscriptionPromptFailure<'a> {
    profile_id: &'a str,
    model: &'a str,
}

impl SubscriptionPromptFailure<'_> {
    /// Logs the failure and returns it as the question's result.
    fn result(
        &self,
        duration_ms: u64,
        http_status: Option<u16>,
        error: String,
        body: Option<&str>,
    ) -> Value {
        let body = body.map(|text| truncate_for_log(text, 300));
        log_event_once(
            "subscription_prompt_test_error",
            json!({
                "account": account_label(self.profile_id),
                "model": self.model,
                "status": http_status,
                "error": error,
                "body": body,
                "handling": "shown_in_result"
            }),
        );
        json!({
            "ok": false,
            "model": self.model,
            "durationMs": duration_ms,
            "httpStatus": http_status,
            "error": error,
            "body": body
        })
    }
}

/// The `response` of the event that ends a Codex stream: `response.completed`,
/// `response.incomplete` or `response.failed`.
fn codex_stream_final_response(body: &str) -> Result<Value, String> {
    let events = body
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
        .collect::<Vec<_>>();
    let event_type = |event: &Value| {
        event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    for event in events.iter().rev() {
        match event_type(event).as_str() {
            "response.completed" | "response.incomplete" | "response.failed" => {
                return event
                    .get("response")
                    .filter(|response| response.is_object())
                    .cloned()
                    .ok_or_else(|| format!("测试流的 {} 事件缺少 response", event_type(event)));
            }
            "error" => return Err(format!("测试流返回错误: {event}")),
            _ => {}
        }
    }
    Err(format!(
        "测试流没有结束事件（共 {} 个事件，最后一个是 {}）",
        events.len(),
        events.last().map(event_type).unwrap_or_default()
    ))
}

/// A question that got an answer: the answer text, the token usage and how long it took.
fn prompt_test_answer(model: &str, duration_ms: u64, status: u16, payload: &Value) -> Value {
    let usage = payload.get("usage").unwrap_or(&Value::Null);
    json!({
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
    })
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

#[cfg(test)]
mod api_test_model_tests {
    use super::*;

    #[test]
    fn defaults_to_gpt_6_2_sol_without_overriding_explicit_models() {
        assert_eq!(normalize_api_test_model(None), "gpt-6.2-sol");
        assert_eq!(normalize_api_test_model(Some("  ".into())), "gpt-6.2-sol");
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

    fn sse(events: &[Value]) -> String {
        events
            .iter()
            .map(|event| {
                format!(
                    "event: {}\ndata: {event}\n\n",
                    event["type"].as_str().unwrap()
                )
            })
            .collect()
    }

    // Shapes as the Codex backend sent them on 2026-10-18.
    #[test]
    fn the_answer_comes_from_the_event_that_ends_the_stream() {
        let completed = json!({
            "status": "completed",
            "model": "gpt-6.1-sol",
            "output": [{ "type": "message", "content": [{ "type": "output_text", "text": "OK" }] }],
            "usage": {
                "input_tokens": 9,
                "output_tokens": 5,
                "output_tokens_details": { "reasoning_tokens": 0 }
            }
        });
        let body = sse(&[
            json!({ "type": "response.created", "response": { "status": "in_progress" } }),
            json!({ "type": "response.output_text.delta", "delta": "OK" }),
            json!({ "type": "response.completed", "response": completed }),
        ]);
        let answer = prompt_test_answer(
            "gpt-6.1-sol",
            2100,
            200,
            &codex_stream_final_response(&body).unwrap(),
        );
        assert_eq!(answer["ok"], true);
        assert_eq!(answer["outputText"], "OK");
        assert_eq!(answer["responseModel"], "gpt-6.1-sol");
        assert_eq!(answer["usage"]["outputTokens"], 5);
        assert_eq!(answer["usage"]["reasoningTokens"], 0);

        let incomplete = sse(&[json!({
            "type": "response.incomplete",
            "response": { "status": "incomplete", "incomplete_details": { "reason": "max_output_tokens" } }
        })]);
        let answer = prompt_test_answer(
            "m",
            1,
            200,
            &codex_stream_final_response(&incomplete).unwrap(),
        );
        assert_eq!(answer["incompleteReason"], "max_output_tokens");

        let failed = sse(&[json!({
            "type": "response.failed",
            "response": { "status": "failed", "error": { "code": "server_error", "message": "boom" } }
        })]);
        assert_eq!(
            codex_stream_final_response(&failed).unwrap()["status"],
            "failed"
        );
    }

    #[test]
    fn a_stream_without_its_end_event_is_an_error() {
        let body = sse(&[
            json!({ "type": "response.created", "response": {} }),
            json!({ "type": "response.output_text.delta", "delta": "O" }),
        ]);
        assert_eq!(
            codex_stream_final_response(&body).unwrap_err(),
            "测试流没有结束事件（共 2 个事件，最后一个是 response.output_text.delta）"
        );
        let error = sse(&[json!({ "type": "error", "message": "bad" })]);
        assert!(codex_stream_final_response(&error)
            .unwrap_err()
            .starts_with("测试流返回错误: "));
        assert!(codex_stream_final_response("")
            .unwrap_err()
            .starts_with("测试流没有结束事件（共 0 个事件"));
    }
}
