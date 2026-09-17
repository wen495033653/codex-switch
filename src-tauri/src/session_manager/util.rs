use super::*;

pub(super) fn sqlite_string_literal(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

pub(super) fn quote_sqlite_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

pub(super) fn unique_sibling_path(path: &Path, base_name: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    for index in 0..1000 {
        let file_name = if index == 0 {
            base_name.to_string()
        } else {
            format!("{base_name}-{index:03}")
        };
        let candidate = parent.join(file_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{base_name}-overflow"))
}

pub(super) fn remove_empty_parent_dirs(root: &Path, parent: Option<&Path>) {
    let Ok(root) = root.canonicalize() else {
        return;
    };
    let protected = [root.join("sessions"), root.join("archived_sessions")];
    let mut current = parent.map(PathBuf::from);
    while let Some(dir) = current {
        let Ok(canonical) = dir.canonicalize() else {
            break;
        };
        if canonical == root || !canonical.starts_with(&root) || protected.contains(&canonical) {
            break;
        }
        match fs::remove_dir(&canonical) {
            Ok(()) => current = canonical.parent().map(PathBuf::from),
            Err(_) => break,
        }
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

pub(super) fn replace_exact_string_value(value: &mut Value, old_value: &str, new_value: &str) {
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

pub(super) fn split_line_ending(segment: &str) -> (&str, &str) {
    if let Some(line) = segment.strip_suffix("\r\n") {
        (line, "\r\n")
    } else if let Some(line) = segment.strip_suffix('\n') {
        (line, "\n")
    } else {
        (segment, "")
    }
}

pub(super) fn normalize_relative_path(value: &str) -> Result<PathBuf, String> {
    let raw = value.trim();
    if raw.is_empty() {
        return Err("会话相对路径为空".to_string());
    }
    if raw.contains('\\') {
        return Err(format!("会话路径不能包含反斜杠: {raw}"));
    }
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        return Err(format!("会话路径不能是绝对路径: {raw}"));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            _ => return Err(format!("会话路径不安全: {raw}")),
        }
    }
    Ok(normalized)
}

pub(super) fn ensure_session_relative_path(path: &Path) -> Result<(), String> {
    let mut components = path.components();
    let first = components
        .next()
        .and_then(|component| component.as_os_str().to_str())
        .unwrap_or("");
    if first != "sessions" && first != "archived_sessions" {
        return Err(format!(
            "会话路径必须位于 sessions 或 archived_sessions: {}",
            path.display()
        ));
    }
    if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
        return Err(format!("会话文件必须是 .jsonl: {}", path.display()));
    }
    Ok(())
}

pub(super) fn status_from_relative_path(path: &Path) -> Result<String, String> {
    let first = path
        .components()
        .next()
        .and_then(|component| component.as_os_str().to_str())
        .unwrap_or("");
    match first {
        "sessions" => Ok("active".to_string()),
        "archived_sessions" => Ok("archived".to_string()),
        _ => Err(format!("无法从路径判断会话状态: {}", path.display())),
    }
}

pub(super) fn normalize_status(status: &str) -> Result<String, String> {
    match status.trim().to_ascii_lowercase().as_str() {
        "active" => Ok("active".to_string()),
        "archived" => Ok("archived".to_string()),
        _ => Err(format!("不支持的会话状态: {status}")),
    }
}

pub(super) fn session_date_parts(
    summary: &SessionSummary,
    path: &Path,
) -> (String, String, String) {
    if let Some(file_name) = path.file_name().and_then(|value| value.to_str()) {
        if let Some(parts) = date_parts_from_rollout_filename(file_name) {
            return parts;
        }
    }
    let timestamp = summary
        .updated_at
        .as_deref()
        .or(summary.created_at.as_deref())
        .and_then(date_parts_from_timestamp);
    if let Some(parts) = timestamp {
        return parts;
    }
    let now = OffsetDateTime::now_utc();
    (
        format!("{:04}", now.year()),
        format!("{:02}", u8::from(now.month())),
        format!("{:02}", now.day()),
    )
}

pub(super) fn date_parts_from_timestamp(timestamp: &str) -> Option<(String, String, String)> {
    let date = timestamp.get(0..10)?;
    let mut parts = date.split('-');
    let year = parts.next()?;
    let month = parts.next()?;
    let day = parts.next()?;
    if year.len() == 4
        && month.len() == 2
        && day.len() == 2
        && [year, month, day]
            .iter()
            .all(|part| part.chars().all(|ch| ch.is_ascii_digit()))
    {
        Some((year.to_string(), month.to_string(), day.to_string()))
    } else {
        None
    }
}

pub(super) fn date_parts_from_rollout_filename(
    file_name: &str,
) -> Option<(String, String, String)> {
    let raw = file_name.strip_prefix("rollout-")?.get(0..10)?;
    date_parts_from_timestamp(raw)
}

pub(super) fn conversation_sort_key(item: &ConversationItem) -> i64 {
    item.updated_at
        .as_deref()
        .and_then(parse_rfc3339_seconds)
        .unwrap_or(0)
}

pub(super) fn is_jsonl_file(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
}

pub(super) fn set_first(target: &mut Option<String>, value: Option<String>) {
    if target.is_none() {
        *target = value;
    }
}

pub(super) fn first_non_empty(values: &[String]) -> Option<String> {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

pub(super) fn truncate_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.trim().chars();
    let mut result = String::new();
    for _ in 0..max_chars {
        let Some(ch) = chars.next() else {
            return result;
        };
        result.push(ch);
    }
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

pub(super) fn extract_uuid_like(value: &str) -> Option<String> {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() < 36 {
        return None;
    }
    for start in 0..=(chars.len() - 36) {
        let slice = &chars[start..start + 36];
        if [8, 13, 18, 23].iter().all(|index| slice[*index] == '-')
            && slice
                .iter()
                .enumerate()
                .all(|(index, ch)| [8, 13, 18, 23].contains(&index) || ch.is_ascii_hexdigit())
        {
            return Some(slice.iter().collect());
        }
    }
    None
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

pub(super) fn reassigned_relative_path(
    relative: &Path,
    old_id: &str,
    new_id: &str,
) -> Result<PathBuf, String> {
    let file_name = relative
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("会话文件名无效: {}", relative.display()))?;
    let new_file_name = if !old_id.is_empty() && file_name.contains(old_id) {
        file_name.replace(old_id, new_id)
    } else if let Some(stem) = file_name.strip_suffix(".jsonl") {
        format!("{stem}-{new_id}.jsonl")
    } else {
        format!("{file_name}-{new_id}")
    };
    Ok(relative
        .parent()
        .map(|parent| parent.join(&new_file_name))
        .unwrap_or_else(|| PathBuf::from(new_file_name)))
}

pub(super) fn path_to_slash(path: impl AsRef<Path>) -> String {
    path.as_ref()
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

pub(super) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|err| format!("读取文件失败 {}: {err}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let len = file
            .read(&mut buffer)
            .map_err(|err| format!("读取文件失败 {}: {err}", path.display()))?;
        if len == 0 {
            break;
        }
        hasher.update(&buffer[..len]);
    }
    Ok(hex_bytes(&hasher.finalize()))
}

pub(super) fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex_bytes(&hasher.finalize())
}

pub(super) fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    result
}

pub(super) fn system_time_to_rfc3339(time: Option<SystemTime>) -> Option<String> {
    time.map(OffsetDateTime::from).and_then(|time| {
        time.format(&time::format_description::well_known::Rfc3339)
            .ok()
    })
}

pub(super) fn timestamp_millis_to_rfc3339(milliseconds: i64) -> Option<String> {
    let nanoseconds = i128::from(milliseconds).checked_mul(1_000_000)?;
    OffsetDateTime::from_unix_timestamp_nanos(nanoseconds)
        .ok()
        .and_then(|time| {
            time.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
}

pub(super) fn timestamp_seconds_to_rfc3339(seconds: i64) -> Option<String> {
    OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|time| {
            time.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
}

pub(super) fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub(super) fn backup_stamp() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}
