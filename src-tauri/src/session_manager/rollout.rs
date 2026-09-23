use super::{
    model::{ConversationMessage, SessionSummary, ThreadDynamicToolMetadata},
    util::{backup_stamp, first_non_empty, hex_bytes, non_empty, truncate_text},
};
use crate::{json_util::raw_string_field, time_util::parse_rfc3339_seconds};
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn conversation_title_from_summary(summary: &SessionSummary) -> String {
    summary
        .title
        .clone()
        .or_else(|| summary.first_user_message.clone())
        .map(|value| truncate_text(&value, 80))
        .unwrap_or_else(|| "未命名会话".to_string())
}

pub(super) fn parse_session_file_for_list(path: &Path) -> Result<SessionSummary, String> {
    parse_session_file_with_limit(path, false, Some(240))
}

#[cfg(test)]
pub(super) fn parse_session_file(
    path: &Path,
    include_messages: bool,
) -> Result<SessionSummary, String> {
    parse_session_file_with_limit(path, include_messages, None)
}

fn parse_session_file_with_limit(
    path: &Path,
    include_messages: bool,
    max_lines: Option<usize>,
) -> Result<SessionSummary, String> {
    let file = fs::File::open(path)
        .map_err(|err| format!("读取会话文件失败 {}: {err}", path.display()))?;
    let reader = BufReader::new(file);
    let mut summary = SessionSummary::default();
    let mut valid_lines = 0usize;
    let mut event_messages = Vec::new();
    let mut fallback_messages = Vec::new();

    for (line_index, line) in reader.lines().enumerate() {
        if max_lines.is_some_and(|limit| line_index >= limit) {
            break;
        }
        let line = line.map_err(|err| format!("读取会话文件失败 {}: {err}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        valid_lines += 1;
        let timestamp = non_empty(raw_string_field(&value, "timestamp"));
        update_summary_times(&mut summary, timestamp.as_deref());
        let event_type = raw_string_field(&value, "type");
        let payload = value.get("payload").unwrap_or(&Value::Null);

        if event_type == "session_meta" {
            set_first(&mut summary.id, non_empty(raw_string_field(payload, "id")));
            set_first(
                &mut summary.cwd,
                non_empty(raw_string_field(payload, "cwd")),
            );
            set_first(&mut summary.source, session_source(payload));
            set_first(
                &mut summary.thread_source,
                non_empty(raw_string_field(payload, "thread_source")),
            );
            set_first(
                &mut summary.model_provider,
                non_empty(raw_string_field(payload, "model_provider")),
            );
            set_first(
                &mut summary.cli_version,
                non_empty(raw_string_field(payload, "cli_version")),
            );
            set_first(
                &mut summary.agent_nickname,
                non_empty(raw_string_field(payload, "agent_nickname")),
            );
            set_first(
                &mut summary.agent_role,
                non_empty(raw_string_field(payload, "agent_role")),
            );
            set_first(
                &mut summary.agent_path,
                non_empty(raw_string_field(payload, "agent_path")),
            );
            set_first(
                &mut summary.history_mode,
                non_empty(raw_string_field(payload, "history_mode")),
            );
            set_first(
                &mut summary.parent_thread_id,
                session_parent_thread_id(payload),
            );
            if summary.dynamic_tools.is_empty() {
                summary.dynamic_tools = session_dynamic_tools(payload);
            }
            continue;
        }

        if event_type == "turn_context" {
            set_first(
                &mut summary.cwd,
                non_empty(raw_string_field(payload, "cwd")),
            );
            continue;
        }

        let payload_type = raw_string_field(payload, "type");
        if event_type == "event_msg" {
            if payload_type == "task_started" {
                set_first(
                    &mut summary.cwd,
                    non_empty(raw_string_field(payload, "cwd")),
                );
                set_first(
                    &mut summary.model,
                    non_empty(raw_string_field(payload, "model")),
                );
                set_first(
                    &mut summary.reasoning_effort,
                    non_empty(raw_string_field(payload, "effort")),
                );
                set_first(
                    &mut summary.approval_mode,
                    non_empty(raw_string_field(payload, "approval_policy")),
                );
                if summary.sandbox_policy.is_none() {
                    if let Some(policy) = payload.get("sandbox_policy") {
                        summary.sandbox_policy = serde_json::to_string(policy).ok();
                    }
                }
            } else if payload_type == "thread_name_updated" {
                set_first(
                    &mut summary.title,
                    first_non_empty(&[
                        raw_string_field(payload, "thread_name"),
                        raw_string_field(payload, "title"),
                        raw_string_field(payload, "name"),
                    ]),
                );
            } else if payload_type == "user_message" {
                if let Some(text) = readable_payload_text(payload) {
                    push_readable_message(
                        &mut event_messages,
                        "user",
                        text,
                        timestamp.clone(),
                        include_messages,
                    );
                }
            } else if payload_type == "agent_message" {
                if let Some(text) = readable_payload_text(payload) {
                    push_readable_message(
                        &mut event_messages,
                        "assistant",
                        text,
                        timestamp.clone(),
                        include_messages,
                    );
                }
            }
        } else if event_type == "response_item" && payload_type == "message" {
            let role = raw_string_field(payload, "role");
            if role == "user" || role == "assistant" {
                if let Some(text) = readable_payload_text(payload) {
                    push_readable_message(
                        &mut fallback_messages,
                        &role,
                        text,
                        timestamp.clone(),
                        include_messages,
                    );
                }
            }
        }
    }

    if valid_lines == 0 {
        summary.parse_error = Some("没有识别到有效 JSONL 事件".to_string());
    }

    let chosen = if event_messages.is_empty() {
        fallback_messages
    } else {
        event_messages
    };
    for message in &chosen {
        if message.role == "user" && summary.first_user_message.is_none() {
            summary.first_user_message = Some(message.text.clone());
        }
        if summary.preview.is_none() {
            summary.preview = Some(truncate_text(&message.text, 120));
        }
    }
    summary.messages = chosen;
    Ok(summary)
}

fn session_source(payload: &Value) -> Option<String> {
    let source = payload.get("source")?;
    if let Some(value) = source.as_str().map(str::to_string).and_then(non_empty) {
        return Some(value);
    }
    if source.is_object() || source.is_array() {
        return serde_json::to_string(source).ok();
    }
    None
}

fn session_parent_thread_id(payload: &Value) -> Option<String> {
    first_non_empty(&[
        raw_string_field(payload, "parent_thread_id"),
        raw_string_field(payload, "forked_from_id"),
        payload
            .pointer("/source/subagent/thread_spawn/parent_thread_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    ])
}

fn session_dynamic_tools(payload: &Value) -> Vec<ThreadDynamicToolMetadata> {
    let Some(items) = payload.get("dynamic_tools").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut tools = Vec::new();
    for item in items {
        let item_type = raw_string_field(item, "type");
        if item_type == "namespace" {
            let namespace = non_empty(raw_string_field(item, "name"));
            for tool in item
                .get("tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(tool) = dynamic_tool_metadata(tool, namespace.clone()) {
                    tools.push(tool);
                }
            }
        } else if let Some(tool) = dynamic_tool_metadata(item, None) {
            tools.push(tool);
        }
    }
    tools
}

fn dynamic_tool_metadata(
    value: &Value,
    namespace: Option<String>,
) -> Option<ThreadDynamicToolMetadata> {
    if raw_string_field(value, "type") != "function" {
        return None;
    }
    let name = non_empty(raw_string_field(value, "name"))?;
    let input_schema = value
        .get("inputSchema")
        .or_else(|| value.get("input_schema"))
        .and_then(|schema| serde_json::to_string(schema).ok())
        .unwrap_or_else(|| "{}".to_string());
    Some(ThreadDynamicToolMetadata {
        name,
        description: raw_string_field(value, "description"),
        input_schema,
        defer_loading: value
            .get("deferLoading")
            .or_else(|| value.get("defer_loading"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        namespace,
    })
}

pub(super) fn readable_payload_text(payload: &Value) -> Option<String> {
    let message = raw_string_field(payload, "message");
    if !message.trim().is_empty() {
        return Some(message.trim().to_string());
    }

    if let Some(text_elements) = payload.get("text_elements").and_then(Value::as_array) {
        let text = text_elements
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            return Some(text);
        }
    }

    match payload.get("content") {
        Some(Value::String(text)) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Some(Value::Array(items)) => {
            let text = items
                .iter()
                .filter_map(|item| {
                    item.get("text")
                        .and_then(Value::as_str)
                        .or_else(|| item.as_str())
                })
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        _ => None,
    }
}

fn push_readable_message(
    messages: &mut Vec<ConversationMessage>,
    role: &str,
    text: String,
    timestamp: Option<String>,
    include_messages: bool,
) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    if include_messages || messages.is_empty() {
        messages.push(ConversationMessage {
            role: role.to_string(),
            text,
            timestamp,
            offset: None,
        });
    }
}

fn update_summary_times(summary: &mut SessionSummary, timestamp: Option<&str>) {
    let Some(timestamp) = timestamp else {
        return;
    };
    if parse_rfc3339_seconds(timestamp).is_none() {
        return;
    }
    if summary.created_at.is_none() {
        summary.created_at = Some(timestamp.to_string());
    }
    if summary
        .updated_at
        .as_deref()
        .and_then(parse_rfc3339_seconds)
        .is_none_or(|current| parse_rfc3339_seconds(timestamp).unwrap_or(current) >= current)
    {
        summary.updated_at = Some(timestamp.to_string());
    }
}

pub(super) fn copy_session_with_new_id(
    source: &Path,
    target: &Path,
    old_id: &str,
    new_id: &str,
) -> Result<(), String> {
    let content = fs::read_to_string(source)
        .map_err(|err| format!("读取会话文件失败 {}: {err}", source.display()))?;
    let output = rewrite_session_id_content(&content, old_id, new_id)?;
    fs::write(target, output).map_err(|err| {
        format!(
            "写入修改 ID 后的会话失败 {} -> {}: {err}",
            source.display(),
            target.display()
        )
    })
}

pub(super) fn rewrite_session_id_content(
    content: &str,
    old_id: &str,
    new_id: &str,
) -> Result<String, String> {
    let mut output = String::with_capacity(content.len());
    for segment in content.split_inclusive('\n') {
        let (line, line_ending) = split_line_ending(segment);
        if line.trim().is_empty() {
            output.push_str(segment);
            continue;
        }
        let mut value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => {
                output.push_str(segment);
                continue;
            }
        };
        replace_exact_string_value(&mut value, old_id, new_id);
        let updated = serde_json::to_string(&value)
            .map_err(|err| format!("序列化修改 ID 后的会话失败: {err}"))?;
        output.push_str(&updated);
        output.push_str(line_ending);
    }
    Ok(output)
}

fn replace_exact_string_value(value: &mut Value, old_value: &str, new_value: &str) {
    match value {
        Value::String(text) if text == old_value => *text = new_value.to_string(),
        Value::Array(items) => {
            for item in items {
                replace_exact_string_value(item, old_value, new_value);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                replace_exact_string_value(item, old_value, new_value);
            }
        }
        _ => {}
    }
}

fn split_line_ending(segment: &str) -> (&str, &str) {
    if let Some(line) = segment.strip_suffix("\r\n") {
        (line, "\r\n")
    } else if let Some(line) = segment.strip_suffix('\n') {
        (line, "\n")
    } else {
        (segment, "")
    }
}

fn set_first(target: &mut Option<String>, value: Option<String>) {
    if target.is_none() {
        *target = value;
    }
}

pub(super) fn new_session_id(seed: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let digest = Sha256::digest(format!("{seed}-{nanos}-{}", backup_stamp()).as_bytes());
    let hex = hex_bytes(&digest);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}
