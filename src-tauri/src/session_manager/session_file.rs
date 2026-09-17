use super::*;

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
    let item = current_state_conversation_for_path(&root, &path, &session_index)?.unwrap_or(
        conversation_from_path(&root, &path, &status, false, &session_index)?,
    );
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

pub(super) fn preview_deleted_conversation_impl(
    delete_id: String,
    before_cursor: Option<u64>,
    snapshot_size: Option<u64>,
    limit: Option<usize>,
    message_source: Option<String>,
    request_id: Option<u64>,
) -> Result<Value, String> {
    let deleted_root = deleted_sessions_dir()?;
    preview_deleted_conversation_from_dir(
        &deleted_root,
        &delete_id,
        before_cursor,
        snapshot_size,
        limit,
        message_source.as_deref(),
        request_id,
    )
}

pub(super) fn preview_deleted_conversation_from_dir(
    deleted_root: &Path,
    delete_id: &str,
    before_cursor: Option<u64>,
    snapshot_size: Option<u64>,
    limit: Option<usize>,
    message_source: Option<&str>,
    request_id: Option<u64>,
) -> Result<Value, String> {
    let record_dir = deleted_session_record_dir_at(deleted_root, delete_id)?;
    let record = read_deleted_session_record(&record_dir)?;
    validate_deleted_record_identity(delete_id, &record)?;
    let record = recover_deleted_session_record_state(&record_dir, record)?
        .ok_or_else(|| "删除操作尚未完成，原会话文件仍然存在".to_string())?;
    let session_file = deleted_record_session_path(&record_dir, &record)?;
    if !session_file.exists() {
        return Err(format!("已删除会话备份文件缺失: {}", record.title));
    }
    verify_deleted_session_backup(&record, &session_file)?;
    let summary = parse_session_file_for_list(&session_file).unwrap_or_default();
    let size_bytes = session_file
        .metadata()
        .map(|item| item.len())
        .unwrap_or(record.size_bytes);
    let title = if should_rebuild_deleted_title(&record.title) {
        conversation_title_from_summary(&summary)
    } else {
        record.title.clone()
    };
    let conversation = ConversationItem {
        id: record.id.clone(),
        title,
        updated_at: record
            .updated_at
            .clone()
            .or(Some(record.deleted_at.clone())),
        status: "deleted".to_string(),
        source_path: session_file.to_string_lossy().to_string(),
        relative_path: record.original_relative_path.clone(),
        size_bytes,
        cwd: summary.cwd.clone().or(record.cwd.clone()),
        preview: summary.preview.clone(),
        sha256: record.sha256.clone(),
        parse_error: summary.parse_error.clone(),
    };
    let page = read_preview_message_page(
        &session_file,
        before_cursor,
        snapshot_size,
        limit,
        message_source,
        request_id,
    )?;

    Ok(json!({
        "ok": true,
        "conversation": conversation,
        "messages": page.messages,
        "message_page": {
            "source": page.source.as_str(),
            "next_before": page.next_before,
            "has_more": page.has_more,
            "file_size": page.file_size,
            "limit": normalize_preview_limit(limit)
        },
        "warnings": [],
        "parse_error": summary.parse_error
    }))
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

pub(super) fn parse_session_file_with_limit(
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

pub(super) fn begin_preview_request(request_id: Option<u64>) {
    if let Some(request_id) = request_id.filter(|value| *value > 0) {
        LATEST_PREVIEW_REQUEST_ID.store(request_id, Ordering::Release);
    }
}

pub(super) fn preview_request_cancelled(request_id: Option<u64>) -> bool {
    request_id
        .filter(|value| *value > 0)
        .is_some_and(|request_id| LATEST_PREVIEW_REQUEST_ID.load(Ordering::Acquire) != request_id)
}

pub(super) fn ensure_preview_request_current(request_id: Option<u64>) -> Result<(), String> {
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

pub(super) fn parse_preview_message_source(
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

pub(super) fn scan_preview_message_source(
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

pub(super) fn preview_message_from_jsonl_line(
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

pub(super) fn session_source(payload: &Value) -> Option<String> {
    let source = payload.get("source")?;
    if let Some(value) = source.as_str().map(str::to_string).and_then(non_empty) {
        return Some(value);
    }
    if source.is_object() || source.is_array() {
        return serde_json::to_string(source).ok();
    }
    None
}

pub(super) fn session_parent_thread_id(payload: &Value) -> Option<String> {
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

pub(super) fn session_dynamic_tools(payload: &Value) -> Vec<ThreadDynamicToolMetadata> {
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

pub(super) fn dynamic_tool_metadata(
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

pub(super) fn push_readable_message(
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

pub(super) fn update_summary_times(summary: &mut SessionSummary, timestamp: Option<&str>) {
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
