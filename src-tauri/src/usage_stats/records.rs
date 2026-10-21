//! Reads the per-response usage Codex writes into rollout files.
//!
//! Every completed Responses API response leaves one `token_usage_record` line whose `usage` is
//! the API's `response.completed.usage` field by field, keyed by the API `response_id`. The
//! model, provider and service tier are not on that line: they are the ones in effect when it
//! was written, i.e. from the latest `turn_context`, `thread_settings_applied` or `session_meta`
//! line before it. (A compaction request is recorded before its turn's `turn_context`, so
//! "latest line before it" is the rule, not "same turn id".)
//!
//! The record is written when the response completed. Its request was sent right after the last
//! line before the response's first output item (a tool output, the user's message, a
//! `token_count` event); Codex writes an `item_completed` event just before each item, so those
//! events do not count. A response with no output item in the rollout (a compaction) has no
//! known start.
//!
//! Its first token is when its earliest reasoning or reply began: newer Codex versions put that
//! in the `started_at_ms` of the `item_completed` event for the item (the rule CC Switch uses).
//! A response that opens with a tool call, or one written by an older Codex, has none here.

use super::model::TokenUsage;
use crate::{
    json_util::{raw_string_field, string_field},
    time_util::{parse_rfc3339_millis, parse_rfc3339_seconds},
};
use memchr::{memchr, memmem};
use serde::Deserialize;
use serde_json::Value;
use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
};

const RESUME_CHECK_BYTES: u64 = 64;

/// Where the previous read of one rollout file stopped, and the settings in effect there.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct RolloutCursor {
    pub(super) offset: u64,
    /// The bytes right before `offset`. Rollout files only grow, except when they are rewritten
    /// in place (the session sync changes the provider line); a file that no longer matches is
    /// read again from the start, and the records seen before are skipped by `response_id`.
    pub(super) resume_check: Vec<u8>,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) service_tier: String,
    /// The last line before the first output item of the response being read (ms since the
    /// epoch, 0 before any line), i.e. when its request was sent.
    pub(super) request_started_ms: i64,
    /// Whether an output item of that response was read.
    pub(super) output_started: bool,
    /// The earliest `started_at_ms` of a reasoning or reply item read since the previous
    /// record (ms since the epoch).
    pub(super) first_output_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RolloutRecord {
    pub(super) response_id: String,
    pub(super) thread_id: String,
    pub(super) timestamp_seconds: i64,
    /// When the record was written, i.e. the response completed (ms since the epoch).
    pub(super) completed_ms: i64,
    /// When its request was sent; `None` without an output item to tell.
    pub(super) request_started_ms: Option<i64>,
    /// When its first reasoning or reply began, between the request and the record; `None`
    /// when the rollout does not say.
    pub(super) first_output_ms: Option<i64>,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) service_tier: String,
    pub(super) usage: TokenUsage,
}

/// A complete line that should hold a record or a setting but could not be read. The line is
/// left out and reading continues after it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SkippedLine {
    pub(super) offset: u64,
    pub(super) reason: String,
}

pub(super) struct RolloutRead {
    pub(super) cursor: RolloutCursor,
    pub(super) records: Vec<RolloutRecord>,
    pub(super) skipped: Vec<SkippedLine>,
    pub(super) restarted: bool,
}

/// Reads the complete lines appended since `previous`. A trailing line without a newline is
/// still being written and is left for the next read.
pub(super) fn read_new_records(
    path: &Path,
    previous: Option<&RolloutCursor>,
) -> io::Result<RolloutRead> {
    let mut file = fs::File::open(path)?;
    let (mut cursor, restarted) = match previous {
        Some(previous) if resumes_at(&mut file, previous)? => (previous.clone(), false),
        Some(_) => (RolloutCursor::default(), true),
        None => (RolloutCursor::default(), false),
    };
    file.seek(SeekFrom::Start(cursor.offset))?;

    let mut reader = BufReader::new(file);
    let mut records = Vec::new();
    let mut skipped = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 || line.last() != Some(&b'\n') {
            break;
        }
        if let Err(reason) = apply_line(&line, &mut cursor, &mut records) {
            skipped.push(SkippedLine {
                offset: cursor.offset,
                reason,
            });
        }
        cursor.offset += read as u64;
    }

    let mut file = reader.into_inner();
    let check_len = cursor.offset.min(RESUME_CHECK_BYTES);
    cursor.resume_check = vec![0; check_len as usize];
    file.seek(SeekFrom::Start(cursor.offset - check_len))?;
    file.read_exact(&mut cursor.resume_check)?;
    Ok(RolloutRead {
        cursor,
        records,
        skipped,
        restarted,
    })
}

fn resumes_at(file: &mut fs::File, previous: &RolloutCursor) -> io::Result<bool> {
    let check_len = previous.resume_check.len() as u64;
    if previous.offset < check_len || file.metadata()?.len() < previous.offset {
        return Ok(false);
    }
    let mut check = vec![0; check_len as usize];
    file.seek(SeekFrom::Start(previous.offset - check_len))?;
    file.read_exact(&mut check)?;
    Ok(check == previous.resume_check)
}

// Lines of tool output run to megabytes, so the search is the SIMD one: comparing every window
// took a minute to read the 2.3 GB of rollouts a re-read covers.
fn contains(line: &[u8], needle: &[u8]) -> bool {
    memchr::memmem::find(line, needle).is_some()
}

fn apply_line(
    line: &[u8],
    cursor: &mut RolloutCursor,
    records: &mut Vec<RolloutRecord>,
) -> Result<(), String> {
    match line_head(line).map(|head| (head.timestamp_ms, head.kind)) {
        Some((_, LineKind::Output)) => cursor.output_started = true,
        Some((_, LineKind::ItemCompleted)) => {
            if let Some(started_ms) = model_output_started_ms(line) {
                cursor.first_output_ms = Some(
                    cursor
                        .first_output_ms
                        .map_or(started_ms, |earliest| earliest.min(started_ms)),
                );
            }
        }
        Some((timestamp_ms, LineKind::Other)) if !cursor.output_started => {
            cursor.request_started_ms = timestamp_ms;
        }
        _ => {}
    }
    // Most lines are messages and tool output; only these four kinds are parsed.
    let relevant = [
        b"\"token_usage_record\"".as_slice(),
        b"\"turn_context\"",
        b"\"thread_settings_applied\"",
        b"\"session_meta\"",
    ];
    if !relevant.iter().any(|needle| contains(line, needle)) {
        return Ok(());
    }
    let value: Value =
        serde_json::from_slice(line).map_err(|err| format!("JSON 解析失败: {err}"))?;
    let payload = value.get("payload").unwrap_or(&Value::Null);
    match raw_string_field(&value, "type").as_str() {
        "token_usage_record" => {
            let request_started_ms = Some(cursor.request_started_ms)
                .filter(|started| cursor.output_started && *started > 0);
            cursor.output_started = false;
            let first_output_ms = cursor.first_output_ms.take();
            let record = parse_record(&value, payload, cursor, request_started_ms, first_output_ms);
            if let Ok(record) = &record {
                cursor.request_started_ms = record.completed_ms;
            }
            records.push(record?);
        }
        "turn_context" => set_if_present(&mut cursor.model, payload, "model"),
        "session_meta" => set_if_present(&mut cursor.provider, payload, "model_provider"),
        "event_msg" if raw_string_field(payload, "type") == "thread_settings_applied" => {
            let settings = payload.get("thread_settings").unwrap_or(&Value::Null);
            set_if_present(&mut cursor.model, settings, "model");
            set_if_present(&mut cursor.provider, settings, "model_provider_id");
            // null: the thread sends no service_tier, which runs on standard processing.
            if settings.get("service_tier").is_some() {
                cursor.service_tier = string_field(settings, "service_tier");
            }
        }
        _ => {}
    }
    Ok(())
}

fn set_if_present(target: &mut String, object: &Value, key: &str) {
    let value = string_field(object, key);
    if !value.is_empty() {
        *target = value;
    }
}

fn parse_record(
    value: &Value,
    payload: &Value,
    cursor: &RolloutCursor,
    request_started_ms: Option<i64>,
    first_output_ms: Option<i64>,
) -> Result<RolloutRecord, String> {
    let response_id = required_string(payload, "response_id")?;
    let thread_id = required_string(payload, "thread_id")?;
    let timestamp = raw_string_field(value, "timestamp");
    let invalid_time = || format!("token_usage_record 时间无效: {timestamp:?}");
    let timestamp_seconds = parse_rfc3339_seconds(&timestamp).ok_or_else(invalid_time)?;
    let completed_ms = parse_rfc3339_millis(&timestamp).ok_or_else(invalid_time)?;
    let usage = payload
        .get("usage")
        .filter(|usage| usage.is_object())
        .ok_or_else(|| "token_usage_record 缺少 usage".to_string())?;
    let request_started_ms = request_started_ms.filter(|started| *started <= completed_ms);
    Ok(RolloutRecord {
        response_id,
        thread_id,
        timestamp_seconds,
        completed_ms,
        request_started_ms,
        first_output_ms: first_output_ms.filter(|started| {
            request_started_ms.is_some_and(|request| request <= *started)
                && *started <= completed_ms
        }),
        provider: cursor.provider.clone(),
        model: cursor.model.clone(),
        service_tier: cursor.service_tier.clone(),
        usage: TokenUsage {
            input_tokens: usage_field(usage, "input_tokens")?,
            cached_input_tokens: usage_field(usage, "cached_input_tokens")?,
            cache_write_input_tokens: usage_field(usage, "cache_write_input_tokens")?,
            output_tokens: usage_field(usage, "output_tokens")?,
            reasoning_output_tokens: usage_field(usage, "reasoning_output_tokens")?,
            total_tokens: usage_field(usage, "total_tokens")?,
        },
    })
}

/// The `started_at_ms` of an `item_completed` line about a reasoning or reply item. Tool runs and
/// user messages are reported the same way but are not model output. The field comes after the
/// item, so the whole line is parsed; only lines naming one of those two types get that far.
fn model_output_started_ms(line: &[u8]) -> Option<i64> {
    if !contains(line, b"\"Reasoning\"") && !contains(line, b"\"AgentMessage\"") {
        return None;
    }
    let value: Value = serde_json::from_slice(line).ok()?;
    let payload = value.get("payload")?;
    let item_type = payload.get("item")?.get("type")?.as_str()?;
    if item_type != "Reasoning" && item_type != "AgentMessage" {
        return None;
    }
    payload.get("started_at_ms")?.as_i64()
}

fn required_string(object: &Value, key: &str) -> Result<String, String> {
    let value = string_field(object, key);
    if value.is_empty() {
        return Err(format!("token_usage_record 缺少 {key}"));
    }
    Ok(value)
}

// `cache_write_input_tokens` is newer than the other fields; Codex reads a missing one as 0.
fn usage_field(usage: &Value, key: &str) -> Result<u64, String> {
    match usage.get(key) {
        None if key == "cache_write_input_tokens" => Ok(0),
        None => Err(format!("token_usage_record 缺少 usage.{key}")),
        Some(raw) => raw
            .as_u64()
            .ok_or_else(|| format!("token_usage_record usage.{key} 不是非负整数: {raw}")),
    }
}

/// What a line means for the timing of the response being read.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LineKind {
    /// An item the model produced: the response is streaming.
    Output,
    /// Written right before each item it reports on.
    ItemCompleted,
    /// Anything else (inputs, settings, events, the record itself).
    Other,
}

#[derive(Debug, PartialEq)]
struct LineHead {
    timestamp_ms: i64,
    kind: LineKind,
}

/// The `response_item` types the model produces. Inputs (user and developer messages, tool
/// outputs, other agents' messages) are the other types.
const OUTPUT_ITEM_TYPES: &[&[u8]] = &[
    b"reasoning",
    b"function_call",
    b"custom_tool_call",
    b"local_shell_call",
    b"web_search_call",
    b"image_generation_call",
    b"tool_search_call",
];

/// How far into a line its head is looked for; tool output lines run to megabytes.
const HEAD_BYTES: usize = 512;

/// The timestamp and kind of a line. Codex writes `timestamp`, `ordinal`, `type` and then
/// `payload` starting with its `type`, so those are read from the first bytes; a line laid out
/// otherwise is parsed for those fields alone.
fn line_head(line: &[u8]) -> Option<LineHead> {
    written_line_head(line).or_else(|| parsed_line_head(line))
}

fn written_line_head(line: &[u8]) -> Option<LineHead> {
    let head = &line[..line.len().min(HEAD_BYTES)];
    let rest = head.strip_prefix(b"{\"timestamp\":\"")?;
    let end = memchr(b'"', rest)?;
    let timestamp_ms = parse_rfc3339_millis(std::str::from_utf8(&rest[..end]).ok()?)?;
    let rest = &rest[end..];
    let type_at = memmem::find(rest, b",\"type\":\"")?;
    if memmem::find(&rest[..type_at], b"\"payload\"").is_some() {
        return None;
    }
    let rest = &rest[type_at + 9..];
    let kind_end = memchr(b'"', rest)?;
    let kind = &rest[..kind_end];
    let payload = rest[kind_end..].strip_prefix(b"\",\"payload\":{\"type\":\"");
    let kind = match kind {
        b"response_item" => {
            let payload = payload?;
            let item_end = memchr(b'"', payload)?;
            let item = &payload[..item_end];
            if item == b"message" {
                let role_at = memmem::find(payload, b"\"role\":\"")? + 8;
                let role_end = memchr(b'"', &payload[role_at..])?;
                item_kind("message", Some(&payload[role_at..role_at + role_end]))
            } else {
                item_kind(std::str::from_utf8(item).ok()?, None)
            }
        }
        b"event_msg" => {
            let payload = payload?;
            let event_end = memchr(b'"', payload)?;
            event_kind(&payload[..event_end])
        }
        _ => LineKind::Other,
    };
    Some(LineHead { timestamp_ms, kind })
}

#[derive(Deserialize)]
struct HeadFields {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    payload: Option<PayloadHead>,
}

#[derive(Deserialize)]
struct PayloadHead {
    #[serde(rename = "type")]
    kind: Option<String>,
    role: Option<String>,
}

fn parsed_line_head(line: &[u8]) -> Option<LineHead> {
    let fields: HeadFields = serde_json::from_slice(line).ok()?;
    let timestamp_ms = parse_rfc3339_millis(fields.timestamp.as_deref()?)?;
    let payload = fields.payload;
    let payload_kind = payload.as_ref().and_then(|payload| payload.kind.as_deref());
    let kind = match (fields.kind.as_deref(), payload_kind) {
        (Some("response_item"), Some(item)) => item_kind(
            item,
            payload
                .as_ref()
                .and_then(|payload| payload.role.as_deref())
                .map(str::as_bytes),
        ),
        (Some("event_msg"), Some(event)) => event_kind(event.as_bytes()),
        _ => LineKind::Other,
    };
    Some(LineHead { timestamp_ms, kind })
}

fn item_kind(item: &str, role: Option<&[u8]>) -> LineKind {
    let output = if item == "message" {
        role == Some(b"assistant".as_slice())
    } else {
        OUTPUT_ITEM_TYPES.contains(&item.as_bytes())
    };
    if output {
        LineKind::Output
    } else {
        LineKind::Other
    }
}

fn event_kind(event: &[u8]) -> LineKind {
    if event == b"item_completed" {
        LineKind::ItemCompleted
    } else {
        LineKind::Other
    }
}
