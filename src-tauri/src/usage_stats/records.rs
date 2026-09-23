//! Reads the per-response usage Codex writes into rollout files.
//!
//! Every completed Responses API response leaves one `token_usage_record` line whose `usage` is
//! the API's `response.completed.usage` field by field, keyed by the API `response_id`. The
//! model, provider and service tier are not on that line: they are the ones in effect when it
//! was written, i.e. from the latest `turn_context`, `thread_settings_applied` or `session_meta`
//! line before it. (A compaction request is recorded before its turn's `turn_context`, so
//! "latest line before it" is the rule, not "same turn id".)

use super::model::TokenUsage;
use crate::{
    json_util::{raw_string_field, string_field},
    time_util::parse_rfc3339_seconds,
};
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
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RolloutRecord {
    pub(super) response_id: String,
    pub(super) thread_id: String,
    pub(super) timestamp_seconds: i64,
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

fn contains(line: &[u8], needle: &[u8]) -> bool {
    line.windows(needle.len()).any(|window| window == needle)
}

fn apply_line(
    line: &[u8],
    cursor: &mut RolloutCursor,
    records: &mut Vec<RolloutRecord>,
) -> Result<(), String> {
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
        "token_usage_record" => records.push(parse_record(&value, payload, cursor)?),
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
) -> Result<RolloutRecord, String> {
    let response_id = required_string(payload, "response_id")?;
    let thread_id = required_string(payload, "thread_id")?;
    let timestamp = raw_string_field(value, "timestamp");
    let timestamp_seconds = parse_rfc3339_seconds(&timestamp)
        .ok_or_else(|| format!("token_usage_record 时间无效: {timestamp:?}"))?;
    let usage = payload
        .get("usage")
        .filter(|usage| usage.is_object())
        .ok_or_else(|| "token_usage_record 缺少 usage".to_string())?;
    Ok(RolloutRecord {
        response_id,
        thread_id,
        timestamp_seconds,
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
