use super::{
    catalog::{conversation_from_path, current_state_conversation_for_path},
    codex_home::{
        ensure_session_relative_path, normalize_relative_path, read_session_index,
        resolve_codex_root, status_from_relative_path, validate_codex_root,
    },
    model::ConversationMessage,
    rollout::readable_payload_text,
    util::non_empty,
};
use crate::json_util::raw_string_field;
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

const PREVIEW_MESSAGE_LIMIT_DEFAULT: usize = 80;

const PREVIEW_MESSAGE_LIMIT_MAX: usize = 200;

const PREVIEW_REVERSE_READ_BLOCK_BYTES: usize = 64 * 1024;

const PREVIEW_CANCELLED_ERROR: &str = "会话预览请求已取消";

static LATEST_PREVIEW_REQUEST_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum PreviewMessageSource {
    Event,
    Response,
}

impl PreviewMessageSource {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Event => "event",
            Self::Response => "response",
        }
    }
}

#[derive(Debug)]
pub(super) struct PreviewMessagePage {
    pub(super) messages: Vec<ConversationMessage>,
    pub(super) source: PreviewMessageSource,
    pub(super) next_before: Option<u64>,
    pub(super) has_more: bool,
    pub(super) file_size: u64,
}

pub(super) fn preview_conversation_impl(
    root: String,
    relative_path: String,
    before_cursor: Option<u64>,
    snapshot_size: Option<u64>,
    limit: Option<usize>,
    message_source: Option<String>,
    request_id: Option<u64>,
) -> Result<Value, String> {
    let root = resolve_codex_root(Some(&root))?;
    validate_codex_root(&root)?;
    let relative = normalize_relative_path(&relative_path)?;
    ensure_session_relative_path(&relative)?;
    let path = root.join(&relative);
    if !path.exists() {
        return Err(format!("会话文件不存在: {}", relative.display()));
    }

    let mut index_warnings = Vec::new();
    let session_index = read_session_index(&root, &mut index_warnings);
    let status = status_from_relative_path(&relative)?;
    // The file-based fallback parses the rollout; only run it (and only let it fail the preview)
    // when the state DB has no row for this path.
    let item = match current_state_conversation_for_path(&root, &path, &session_index)? {
        Some(item) => item,
        None => conversation_from_path(&root, &path, &status, false, &session_index)?,
    };
    let page = read_preview_message_page(
        &path,
        before_cursor,
        snapshot_size,
        limit,
        message_source.as_deref(),
        request_id,
    )?;

    Ok(json!({
        "ok": true,
        "conversation": item,
        "messages": page.messages,
        "message_page": {
            "source": page.source.as_str(),
            "next_before": page.next_before,
            "has_more": page.has_more,
            "file_size": page.file_size,
            "limit": normalize_preview_limit(limit)
        },
        "warnings": [],
        "parse_error": null
    }))
}

pub(super) fn begin_preview_request(request_id: Option<u64>) {
    if let Some(request_id) = request_id.filter(|value| *value > 0) {
        LATEST_PREVIEW_REQUEST_ID.store(request_id, Ordering::Release);
    }
}

fn preview_request_cancelled(request_id: Option<u64>) -> bool {
    request_id
        .filter(|value| *value > 0)
        .is_some_and(|request_id| LATEST_PREVIEW_REQUEST_ID.load(Ordering::Acquire) != request_id)
}

fn ensure_preview_request_current(request_id: Option<u64>) -> Result<(), String> {
    if preview_request_cancelled(request_id) {
        Err(PREVIEW_CANCELLED_ERROR.to_string())
    } else {
        Ok(())
    }
}

pub(super) fn normalize_preview_limit(limit: Option<usize>) -> usize {
    limit
        .unwrap_or(PREVIEW_MESSAGE_LIMIT_DEFAULT)
        .clamp(1, PREVIEW_MESSAGE_LIMIT_MAX)
}

fn parse_preview_message_source(
    source: Option<&str>,
) -> Result<Option<PreviewMessageSource>, String> {
    match source.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some("event") => Ok(Some(PreviewMessageSource::Event)),
        Some("response") => Ok(Some(PreviewMessageSource::Response)),
        Some(other) => Err(format!("不支持的会话消息来源: {other}")),
    }
}

pub(super) fn read_preview_message_page(
    path: &Path,
    before_cursor: Option<u64>,
    snapshot_size: Option<u64>,
    limit: Option<usize>,
    message_source: Option<&str>,
    request_id: Option<u64>,
) -> Result<PreviewMessagePage, String> {
    ensure_preview_request_current(request_id)?;
    let limit = normalize_preview_limit(limit);
    let source = parse_preview_message_source(message_source)?;
    if let Some(source) = source {
        return scan_preview_message_source(
            path,
            before_cursor,
            snapshot_size,
            limit,
            source,
            request_id,
        );
    }

    let event_page = scan_preview_message_source(
        path,
        before_cursor,
        snapshot_size,
        limit,
        PreviewMessageSource::Event,
        request_id,
    )?;
    if !event_page.messages.is_empty() {
        return Ok(event_page);
    }
    scan_preview_message_source(
        path,
        before_cursor,
        snapshot_size,
        limit,
        PreviewMessageSource::Response,
        request_id,
    )
}

fn scan_preview_message_source(
    path: &Path,
    before_cursor: Option<u64>,
    snapshot_size: Option<u64>,
    limit: usize,
    source: PreviewMessageSource,
    request_id: Option<u64>,
) -> Result<PreviewMessagePage, String> {
    let current_file_size = fs::metadata(path)
        .map_err(|err| format!("读取会话文件信息失败 {}: {err}", path.display()))?
        .len();
    if snapshot_size.is_some_and(|snapshot| current_file_size < snapshot) {
        return Err("会话文件已变化，请重新加载最新内容".to_string());
    }
    let file_size = snapshot_size.unwrap_or(current_file_size);
    let mut position = before_cursor.unwrap_or(file_size).min(file_size);
    let mut file = fs::File::open(path)
        .map_err(|err| format!("读取会话文件失败 {}: {err}", path.display()))?;
    let mut carry = Vec::new();
    let mut matches = Vec::<(u64, ConversationMessage)>::new();

    'scan: while position > 0 {
        ensure_preview_request_current(request_id)?;
        let start = position.saturating_sub(PREVIEW_REVERSE_READ_BLOCK_BYTES as u64);
        let block_len = usize::try_from(position - start)
            .map_err(|_| format!("会话文件分段长度无效: {}", path.display()))?;
        let mut block = vec![0u8; block_len];
        file.seek(SeekFrom::Start(start))
            .map_err(|err| format!("定位会话文件失败 {}: {err}", path.display()))?;
        file.read_exact(&mut block)
            .map_err(|err| format!("分段读取会话文件失败 {}: {err}", path.display()))?;
        block.extend_from_slice(&carry);

        let mut segment_end = block.len();
        for newline in (0..block.len())
            .rev()
            .filter(|index| block[*index] == b'\n')
        {
            let segment_start = newline + 1;
            if segment_start < segment_end {
                let line_offset = start.saturating_add(segment_start as u64);
                if let Some(message) = preview_message_from_jsonl_line(
                    &block[segment_start..segment_end],
                    source,
                    line_offset,
                ) {
                    matches.push((line_offset, message));
                    if matches.len() > limit {
                        break 'scan;
                    }
                }
            }
            segment_end = newline;
        }

        if start == 0 {
            if segment_end > 0 {
                if let Some(message) =
                    preview_message_from_jsonl_line(&block[..segment_end], source, 0)
                {
                    matches.push((0, message));
                }
            }
            position = 0;
        } else {
            carry.clear();
            carry.extend_from_slice(&block[..segment_end]);
            position = start;
        }
    }

    ensure_preview_request_current(request_id)?;
    let has_more = matches.len() > limit;
    if has_more {
        matches.truncate(limit);
    }
    let next_before = has_more
        .then(|| matches.last().map(|item| item.0))
        .flatten();
    let messages = matches
        .into_iter()
        .rev()
        .map(|(_, message)| message)
        .collect();
    Ok(PreviewMessagePage {
        messages,
        source,
        next_before,
        has_more,
        file_size,
    })
}

fn preview_message_from_jsonl_line(
    line: &[u8],
    source: PreviewMessageSource,
    offset: u64,
) -> Option<ConversationMessage> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let value = serde_json::from_slice::<Value>(line).ok()?;
    let event_type = raw_string_field(&value, "type");
    let payload = value.get("payload")?;
    let payload_type = raw_string_field(payload, "type");
    let (role, text) = match source {
        PreviewMessageSource::Event if event_type == "event_msg" => {
            let role = match payload_type.as_str() {
                "user_message" => "user",
                "agent_message" => "assistant",
                _ => return None,
            };
            (role, readable_payload_text(payload)?)
        }
        PreviewMessageSource::Response
            if event_type == "response_item" && payload_type == "message" =>
        {
            let role = raw_string_field(payload, "role");
            if role != "user" && role != "assistant" {
                return None;
            }
            let text = readable_payload_text(payload)?;
            (if role == "user" { "user" } else { "assistant" }, text)
        }
        _ => return None,
    };
    Some(ConversationMessage {
        role: role.to_string(),
        text,
        timestamp: non_empty(raw_string_field(&value, "timestamp")),
        offset: Some(offset),
    })
}
