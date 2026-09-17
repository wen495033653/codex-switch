use super::{
    backup::backup_file_with_reason,
    util::{dedupe_strings, first_non_empty, non_empty},
};
use crate::{json_util::raw_string_field, paths::codex_dir};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, BufReader},
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone)]
pub(super) struct SessionIndexEntry {
    pub(super) thread_name: Option<String>,
    pub(super) updated_at: Option<String>,
}

pub(super) type SessionIndex = HashMap<String, SessionIndexEntry>;

pub(super) fn remove_from_global_state(
    root: &Path,
    ids: &[String],
    reason: &str,
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let path = root.join(".codex-global-state.json");
    if !path.exists() {
        return Ok(());
    }
    let id_set: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("读取 .codex-global-state.json 失败: {err}"))?;
    let mut value: Value = serde_json::from_str(&content)
        .map_err(|err| format!("解析 .codex-global-state.json 失败: {err}"))?;
    let removed = remove_matching_object_keys(&mut value, &id_set);
    if removed == 0 {
        return Ok(());
    }
    backup_file_with_reason(&path, reason)?;
    let mut output = serde_json::to_string_pretty(&value)
        .map_err(|err| format!("序列化 .codex-global-state.json 失败: {err}"))?;
    output.push('\n');
    fs::write(&path, output).map_err(|err| format!("写入 .codex-global-state.json 失败: {err}"))?;
    Ok(())
}

pub(super) fn remove_matching_object_keys(value: &mut Value, ids: &HashSet<&str>) -> usize {
    match value {
        Value::Object(map) => {
            let keys = map
                .keys()
                .filter(|key| ids.contains(key.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            let mut removed = 0usize;
            for key in keys {
                map.remove(&key);
                removed += 1;
            }
            for (key, value) in map.iter_mut() {
                if matches!(key.as_str(), "pinned-thread-ids" | "pinnedThreadIds") {
                    if let Value::Array(items) = value {
                        let before = items.len();
                        items.retain(|item| item.as_str().is_none_or(|id| !ids.contains(id)));
                        removed += before.saturating_sub(items.len());
                    }
                } else {
                    removed += remove_matching_object_keys(value, ids);
                }
            }
            removed
        }
        Value::Array(items) => items
            .iter_mut()
            .map(|item| remove_matching_object_keys(item, ids))
            .sum(),
        _ => 0,
    }
}

pub(super) fn resolve_codex_root(root: Option<&str>) -> Result<PathBuf, String> {
    let root = root.map(str::trim).filter(|value| !value.is_empty());
    let path = match root {
        Some(root) => PathBuf::from(root),
        None => codex_dir()?,
    };
    if !path.exists() {
        return Err(format!("Codex 数据目录不存在: {}", path.display()));
    }
    if !path.is_dir() {
        return Err(format!("Codex 数据目录不是文件夹: {}", path.display()));
    }
    Ok(path)
}

pub(super) fn session_id_variants(session_id: &str) -> Vec<String> {
    let raw = session_id.trim();
    let bare = raw.strip_prefix("local:").unwrap_or(raw);
    let mut variants = vec![raw.to_string(), bare.to_string()];
    if !bare.is_empty() {
        variants.push(format!("local:{bare}"));
    }
    dedupe_strings(&mut variants);
    variants
}

pub(super) fn session_index_entry<'a>(
    index: &'a SessionIndex,
    session_id: &str,
) -> Option<&'a SessionIndexEntry> {
    session_id_variants(session_id)
        .into_iter()
        .find_map(|variant| index.get(&variant))
}

pub(super) fn session_index_title(index: &SessionIndex, session_id: &str) -> Option<String> {
    session_index_entry(index, session_id).and_then(|entry| entry.thread_name.clone())
}

pub(super) fn validate_codex_root(root: &Path) -> Result<(), String> {
    let sessions = root.join("sessions");
    let archived = root.join("archived_sessions");
    if sessions.exists() || archived.exists() {
        Ok(())
    } else {
        Err(format!(
            "不是有效的 Codex 数据目录，缺少 sessions 或 archived_sessions: {}",
            root.display()
        ))
    }
}

pub(super) fn validate_session_file_path(root: &Path, path: &Path) -> Result<(), String> {
    let root = root
        .canonicalize()
        .map_err(|err| format!("读取 Codex 数据目录失败 {}: {err}", root.display()))?;
    let path = path
        .canonicalize()
        .map_err(|err| format!("读取会话文件失败 {}: {err}", path.display()))?;

    if !path.starts_with(&root) {
        return Err(format!(
            "拒绝处理 Codex 数据目录外的文件: {}",
            path.display()
        ));
    }
    if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
        return Err(format!("拒绝处理非 jsonl 会话文件: {}", path.display()));
    }
    let sessions = root.join("sessions");
    let archived_sessions = root.join("archived_sessions");
    if !path.starts_with(&sessions) && !path.starts_with(&archived_sessions) {
        return Err(format!("拒绝处理非会话目录中的文件: {}", path.display()));
    }
    Ok(())
}

pub(super) fn read_session_index(root: &Path, warnings: &mut Vec<String>) -> SessionIndex {
    let path = root.join("session_index.jsonl");
    let mut map = HashMap::new();
    if !path.exists() {
        warnings
            .push("session_index.jsonl 不存在，已使用其他会话元数据推断标题和更新时间".to_string());
        return map;
    }
    let file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(err) => {
            warnings.push(format!("读取 session_index.jsonl 失败: {err}"));
            return map;
        }
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = raw_string_field(&value, "id");
        if id.is_empty() {
            continue;
        }
        let thread_name = first_non_empty(&[
            raw_string_field(&value, "thread_name"),
            raw_string_field(&value, "title"),
        ]);
        let updated_at = non_empty(raw_string_field(&value, "updated_at"));
        let previous = session_index_entry(&map, &id).cloned();
        let entry = SessionIndexEntry {
            thread_name: thread_name.or_else(|| {
                previous
                    .as_ref()
                    .and_then(|entry| entry.thread_name.clone())
            }),
            updated_at: updated_at
                .or_else(|| previous.as_ref().and_then(|entry| entry.updated_at.clone())),
        };
        for variant in session_id_variants(&id) {
            map.insert(variant, entry.clone());
        }
    }
    map
}

pub(super) fn collect_conversation_files(
    dir: &Path,
    status: &str,
    files: &mut Vec<(String, PathBuf)>,
    errors: &mut Vec<String>,
) {
    if !dir.exists() {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            errors.push(format!("读取目录失败 {}: {err}", dir.display()));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                errors.push(format!("读取目录条目失败 {}: {err}", dir.display()));
                continue;
            }
        };
        let path = entry.path();
        match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => {
                collect_conversation_files(&path, status, files, errors);
            }
            Ok(file_type) if file_type.is_file() && is_jsonl_file(&path) => {
                files.push((status.to_string(), path));
            }
            Ok(_) => {}
            Err(err) => errors.push(format!("读取文件类型失败 {}: {err}", path.display())),
        }
    }
}

pub(super) fn relative_path_under_root(root: &Path, path: &Path) -> Option<PathBuf> {
    if let Ok(relative) = path.strip_prefix(root) {
        return Some(relative.to_path_buf());
    }
    if path.exists() {
        let canonical_root = root.canonicalize().ok()?;
        let canonical_path = path.canonicalize().ok()?;
        if let Ok(relative) = canonical_path.strip_prefix(canonical_root) {
            return Some(relative.to_path_buf());
        }
    }
    if cfg!(windows) {
        let root_text = path_to_slash(root).trim_end_matches('/').to_string();
        let path_text = path_to_slash(path);
        if path_text.len() > root_text.len()
            && path_text[..root_text.len()].eq_ignore_ascii_case(&root_text)
            && path_text.as_bytes().get(root_text.len()) == Some(&b'/')
        {
            return normalize_relative_path(&path_text[root_text.len() + 1..]).ok();
        }
    }
    None
}

pub(super) fn conversation_path_key(path: &Path) -> String {
    normalized_path_identity(path)
}

pub(super) fn normalized_path_identity(path: &Path) -> String {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut value = resolved.to_string_lossy().replace('\\', "/");
    if let Some(rest) = value.strip_prefix("//?/UNC/") {
        value = format!("//{rest}");
    } else if let Some(rest) = value.strip_prefix("//?/") {
        value = rest.to_string();
    }
    while value.len() > 1 && value.ends_with('/') {
        value.pop();
    }
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
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

fn is_jsonl_file(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
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
