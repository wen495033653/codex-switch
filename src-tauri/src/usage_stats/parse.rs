use super::model::{ParsedSession, TimestampValue, TokenUsage, TokenUsageEvent};
use crate::{
    json_util::{raw_string_field, string_field},
    time_util::parse_rfc3339_seconds,
};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
};

pub(super) fn parse_session_file(path: &Path) -> Result<ParsedSession, String> {
    let file = fs::File::open(path)
        .map_err(|err| format!("读取 Codex session 文件失败 {}: {err}", path.display()))?;
    let reader = BufReader::new(file);
    let mut parsed = ParsedSession::default();
    for line in reader.lines() {
        let line =
            line.map_err(|err| format!("读取 Codex session 文件失败 {}: {err}", path.display()))?;
        parse_session_line(&line, &mut parsed);
    }
    Ok(parsed)
}

pub(super) fn parse_session_line(line: &str, parsed: &mut ParsedSession) {
    if !line.contains("\"session_meta\"")
        && !line.contains("\"turn_context\"")
        && !line.contains("\"token_count\"")
    {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    match raw_string_field(&value, "type").as_str() {
        "session_meta" => parse_session_meta_line(&value, parsed),
        "turn_context" => parse_turn_context_line(&value, parsed),
        "event_msg" => parse_event_msg_line(&value, parsed),
        _ => {}
    }
}

fn parse_session_meta_line(value: &Value, parsed: &mut ParsedSession) {
    let payload = value.get("payload").unwrap_or(&Value::Null);
    let session_id = string_field(payload, "id");
    if !session_id.is_empty() {
        parsed.session_id = session_id;
    }
    let provider = string_field(payload, "model_provider");
    if !provider.is_empty() {
        parsed.provider = provider;
    }
    update_model_from_payload(payload, parsed);
    if let Some(timestamp) = timestamp_from_payload(value, payload) {
        parsed.started_at = Some(timestamp);
    }
}

fn parse_turn_context_line(value: &Value, parsed: &mut ParsedSession) {
    let payload = value.get("payload").unwrap_or(&Value::Null);
    update_model_from_payload(payload, parsed);
}

fn parse_event_msg_line(value: &Value, parsed: &mut ParsedSession) {
    let payload = value.get("payload").unwrap_or(&Value::Null);
    if string_field(payload, "type") != "token_count" {
        return;
    }
    let Some(info) = payload.get("info") else {
        return;
    };
    let Some(total_usage) = info.get("total_token_usage") else {
        return;
    };
    let usage = TokenUsage {
        input_tokens: u64_field(total_usage, "input_tokens"),
        cached_input_tokens: u64_field(total_usage, "cached_input_tokens"),
        output_tokens: u64_field(total_usage, "output_tokens"),
        reasoning_output_tokens: u64_field(total_usage, "reasoning_output_tokens"),
        total_tokens: u64_field(total_usage, "total_tokens"),
    };
    if usage.total_tokens == 0 {
        return;
    }
    let Some(timestamp) = timestamp_from_payload(value, payload) else {
        return;
    };
    record_token_count_delta(parsed, &usage, timestamp.seconds);
    let should_update = parsed
        .usage
        .as_ref()
        .map(|current| usage.total_tokens >= current.total_tokens)
        .unwrap_or(true);
    if should_update {
        parsed.usage = Some(usage);
        parsed.updated_at = Some(timestamp);
        parsed.model_context_window = optional_u64_field(info, "model_context_window");
    }
}

/// Stores what this token_count added since the previous one. The today / 7 day / 30 day
/// windows are summed from these events in `aggregate`, so the scan does not depend on "now".
fn record_token_count_delta(
    parsed: &mut ParsedSession,
    usage: &TokenUsage,
    timestamp_seconds: i64,
) {
    let delta = parsed
        .previous_event_usage
        .as_ref()
        .map(|previous| {
            if usage.total_tokens >= previous.total_tokens {
                usage.saturating_delta(previous)
            } else {
                usage.clone()
            }
        })
        .unwrap_or_else(|| usage.clone());
    parsed.previous_event_usage = Some(usage.clone());
    if !delta.has_tokens() {
        return;
    }

    parsed.token_events.push(TokenUsageEvent {
        timestamp_seconds,
        usage: delta,
    });
}

fn timestamp_from_payload(root: &Value, payload: &Value) -> Option<TimestampValue> {
    let raw = string_field(payload, "timestamp");
    let raw = if raw.is_empty() {
        string_field(root, "timestamp")
    } else {
        raw
    };
    parse_rfc3339_seconds(&raw).map(|seconds| TimestampValue { raw, seconds })
}

fn update_model_from_payload(payload: &Value, parsed: &mut ParsedSession) {
    for key in ["model", "model_slug", "selected_model", "current_model"] {
        let model = string_field(payload, key);
        if !model.is_empty() {
            parsed.model = model;
            return;
        }
    }
}

fn u64_field(value: &Value, key: &str) -> u64 {
    optional_u64_field(value, key).unwrap_or(0)
}

fn optional_u64_field(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(|raw| {
        raw.as_u64()
            .or_else(|| raw.as_i64().and_then(|number| u64::try_from(number).ok()))
            .or_else(|| {
                raw.as_str()
                    .and_then(|text| text.trim().parse::<u64>().ok())
            })
    })
}
