use super::support::{provider_log_value, write_existing_file};
use crate::{json_util::raw_string_field, session_sync_diagnostics::log_session_sync_event};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

// TODO(verify): only the 50 most recently active rollouts are rewritten, while the state DB is synced in
// full. Two syncs with different "recent 50" windows leave rollouts whose first line still names the old
// provider (16 such files on 2026-09-17). Whether Codex reads that value when resuming is unknown; the
// trigger, pass criteria and where to continue are in docs/development/codex-restart.md ("待验证").
pub(super) const SESSION_SYNC_RECENT_ROLLOUT_LIMIT: usize = 50;

const SESSION_SYNC_TAIL_SAMPLE_BYTES: u64 = 128 * 1024;

#[derive(Debug)]
struct RolloutFileSyncOutcome {
    changed: bool,
    from_provider: Option<String>,
}

#[derive(Debug, Eq, PartialEq)]
struct RolloutFileCandidate {
    path: PathBuf,
    sort_key: String,
}

#[cfg(test)]
pub(super) fn sync_codex_session_rollouts_to_provider(
    sessions_dir: &Path,
    target_provider: &str,
) -> Result<usize, String> {
    let rollout_files =
        collect_recent_rollout_files(sessions_dir, SESSION_SYNC_RECENT_ROLLOUT_LIMIT)?;
    sync_rollout_files_to_provider(rollout_files, target_provider)
}

#[cfg(test)]
pub(super) fn sync_codex_session_rollout_dirs_to_provider(
    rollout_dirs: &[PathBuf],
    target_provider: &str,
    extra_rollout_paths: &[PathBuf],
) -> Result<usize, String> {
    sync_codex_session_rollout_dirs_to_provider_with_diagnostics(
        rollout_dirs,
        target_provider,
        extra_rollout_paths,
        None,
    )
}

pub(super) fn sync_codex_session_rollout_dirs_to_provider_with_diagnostics(
    rollout_dirs: &[PathBuf],
    target_provider: &str,
    extra_rollout_paths: &[PathBuf],
    trigger: Option<&str>,
) -> Result<usize, String> {
    sync_codex_session_rollout_dirs_to_provider_with_diagnostics_limit(
        rollout_dirs,
        target_provider,
        extra_rollout_paths,
        trigger,
        SESSION_SYNC_RECENT_ROLLOUT_LIMIT,
    )
}

pub(super) fn preview_codex_session_rollout_dirs_to_provider_with_diagnostics(
    rollout_dirs: &[PathBuf],
    target_provider: &str,
    extra_rollout_paths: &[PathBuf],
    trigger: Option<&str>,
) -> Result<usize, String> {
    let rollout_files = collect_recent_rollout_files_from_dirs(
        rollout_dirs,
        SESSION_SYNC_RECENT_ROLLOUT_LIMIT,
        extra_rollout_paths,
    )?;
    if let Some(trigger) = trigger {
        log_session_sync_event(
            "session_sync_preflight_rollout_selection",
            json!({
                "trigger": trigger,
                "targetProvider": target_provider,
                "selectedCount": rollout_files.len(),
                "extraRolloutCount": extra_rollout_paths.len()
            }),
        );
    }
    preview_rollout_files_to_provider_with_diagnostics(rollout_files, target_provider, trigger)
}

fn sync_codex_session_rollout_dirs_to_provider_with_diagnostics_limit(
    rollout_dirs: &[PathBuf],
    target_provider: &str,
    extra_rollout_paths: &[PathBuf],
    trigger: Option<&str>,
    limit: usize,
) -> Result<usize, String> {
    let rollout_files =
        collect_recent_rollout_files_from_dirs(rollout_dirs, limit, extra_rollout_paths)?;
    if let Some(trigger) = trigger {
        log_session_sync_event(
            "session_sync_rollout_selection",
            json!({
                "trigger": trigger,
                "targetProvider": target_provider,
                "selectedCount": rollout_files.len(),
                "extraRolloutCount": extra_rollout_paths.len()
            }),
        );
    }
    sync_rollout_files_to_provider_with_diagnostics(rollout_files, target_provider, trigger)
}

fn preview_rollout_files_to_provider_with_diagnostics(
    rollout_files: Vec<PathBuf>,
    target_provider: &str,
    trigger: Option<&str>,
) -> Result<usize, String> {
    let mut updated = 0;
    let mut errors = Vec::new();

    for path in rollout_files {
        match rollout_file_provider_would_change(&path, target_provider) {
            Ok(true) => updated += 1,
            Ok(false) => {}
            Err(err) => {
                if let Some(trigger) = trigger {
                    log_session_sync_event(
                        "session_sync_preflight_rollout_file_error",
                        json!({
                            "trigger": trigger,
                            "targetProvider": target_provider,
                            "path": path.to_string_lossy().to_string(),
                            "error": err.clone()
                        }),
                    );
                }
                errors.push(err);
            }
        }
    }

    if errors.is_empty() {
        if let Some(trigger) = trigger {
            log_session_sync_event(
                "session_sync_preflight_rollout_batch_finish",
                json!({
                    "trigger": trigger,
                    "targetProvider": target_provider,
                    "updatedFiles": updated
                }),
            );
        }
        Ok(updated)
    } else {
        Err(format!(
            "预检查 Codex 会话失败，预计更新 {updated} 个文件，{} 个文件失败：{}",
            errors.len(),
            errors.join("；")
        ))
    }
}

#[cfg(test)]
fn sync_rollout_files_to_provider(
    rollout_files: Vec<PathBuf>,
    target_provider: &str,
) -> Result<usize, String> {
    sync_rollout_files_to_provider_with_diagnostics(rollout_files, target_provider, None)
}

fn sync_rollout_files_to_provider_with_diagnostics(
    rollout_files: Vec<PathBuf>,
    target_provider: &str,
    trigger: Option<&str>,
) -> Result<usize, String> {
    let mut updated = 0;
    let mut errors = Vec::new();
    let mut provider_change_counts = BTreeMap::new();

    for path in rollout_files {
        match sync_rollout_file_provider_with_diagnostics(&path, target_provider, trigger) {
            Ok(outcome) if outcome.changed => {
                updated += 1;
                if let Some(from_provider) = outcome.from_provider {
                    *provider_change_counts
                        .entry(from_provider)
                        .or_insert(0usize) += 1;
                }
            }
            Ok(_) => {}
            Err(err) => {
                if let Some(trigger) = trigger {
                    log_session_sync_event(
                        "session_sync_rollout_file_error",
                        json!({
                            "trigger": trigger,
                            "targetProvider": target_provider,
                            "path": path.to_string_lossy().to_string(),
                            "error": err.clone()
                        }),
                    );
                }
                errors.push(err);
            }
        }
    }

    if errors.is_empty() {
        if let Some(trigger) = trigger {
            log_session_sync_event(
                "session_sync_rollout_batch_finish",
                json!({
                    "trigger": trigger,
                    "targetProvider": target_provider,
                    "updatedFiles": updated,
                        "providerChanges": provider_change_counts_json(
                            &provider_change_counts,
                            target_provider
                        )
                }),
            );
        }
        Ok(updated)
    } else {
        Err(format!(
            "同步 Codex 会话失败，已更新 {updated} 个文件，{} 个文件失败：{}",
            errors.len(),
            errors.join("；")
        ))
    }
}

fn provider_change_counts_json(counts: &BTreeMap<String, usize>, target_provider: &str) -> Value {
    Value::Array(
        counts
            .iter()
            .map(|(from_provider, count)| {
                json!({
                    "fromProvider": from_provider,
                    "toProvider": target_provider,
                    "files": count
                })
            })
            .collect(),
    )
}

fn rollout_content_session_meta_summary(content: &str) -> Value {
    for line in content.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        let Some(payload) = event.get("payload").and_then(Value::as_object) else {
            return json!({
                "timestamp": raw_string_field(&event, "timestamp"),
                "payloadMissing": true
            });
        };
        return json!({
            "timestamp": raw_string_field(&event, "timestamp"),
            "id": payload.get("id").and_then(Value::as_str).unwrap_or(""),
            "cwd": payload.get("cwd").and_then(Value::as_str).unwrap_or(""),
            "modelProvider": payload.get("model_provider").and_then(Value::as_str).unwrap_or("")
        });
    }
    json!({ "missing": true })
}

fn rollout_meta_provider(meta: &Value) -> String {
    if meta
        .get("missing")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "(session_meta 缺失)".to_string()
    } else if meta
        .get("payloadMissing")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "(payload 缺失)".to_string()
    } else {
        provider_log_value(
            meta.get("modelProvider")
                .and_then(Value::as_str)
                .unwrap_or(""),
        )
    }
}

fn sync_rollout_file_provider_with_diagnostics(
    path: &Path,
    target_provider: &str,
    _trigger: Option<&str>,
) -> Result<RolloutFileSyncOutcome, String> {
    let original_modified = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(|err| format!("读取 Codex session 修改时间失败 {}: {err}", path.display()))?;
    let content = fs::read_to_string(path)
        .map_err(|err| format!("读取 Codex session 文件失败 {}: {err}", path.display()))?;
    let before_meta = rollout_content_session_meta_summary(&content);
    let from_provider = rollout_meta_provider(&before_meta);
    let mut updated_content = String::with_capacity(content.len());
    let mut changed = false;

    for segment in content.split_inclusive('\n') {
        let (line, line_ending) = split_line_ending(segment);
        match update_rollout_provider_line(line, target_provider)? {
            Some(updated_line) => {
                updated_content.push_str(&updated_line);
                updated_content.push_str(line_ending);
                changed = true;
            }
            None => updated_content.push_str(segment),
        }
    }

    if changed {
        let wrote = write_existing_file(path, &updated_content, "写入 Codex session 文件")?;
        if wrote {
            fs::OpenOptions::new()
                .write(true)
                .open(path)
                .and_then(|file| file.set_modified(original_modified))
                .map_err(|err| {
                    format!("恢复 Codex session 修改时间失败 {}: {err}", path.display())
                })?;
        }
        changed = wrote;
    }

    Ok(RolloutFileSyncOutcome {
        changed,
        from_provider: changed.then_some(from_provider),
    })
}

fn rollout_file_provider_would_change(path: &Path, target_provider: &str) -> Result<bool, String> {
    let content = fs::read_to_string(path)
        .map_err(|err| format!("读取 Codex session 文件失败 {}: {err}", path.display()))?;

    for segment in content.split_inclusive('\n') {
        let (line, _line_ending) = split_line_ending(segment);
        if update_rollout_provider_line(line, target_provider)?.is_some() {
            return Ok(true);
        }
    }

    Ok(false)
}

#[cfg(test)]
fn collect_recent_rollout_files(dir: &Path, limit: usize) -> Result<Vec<PathBuf>, String> {
    collect_recent_rollout_files_from_dirs(&[dir.to_path_buf()], limit, &[])
}

fn collect_recent_rollout_files_from_dirs(
    dirs: &[PathBuf],
    limit: usize,
    extra_rollout_paths: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for dir in dirs {
        if dir.exists() {
            collect_rollout_file_candidates(dir, &mut files)?;
        }
    }
    files.sort_by(|a, b| {
        b.sort_key
            .cmp(&a.sort_key)
            .then_with(|| b.path.cmp(&a.path))
    });

    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    for candidate in files.into_iter().take(limit) {
        if seen.insert(candidate.path.clone()) {
            selected.push(candidate.path);
        }
    }

    for path in extra_rollout_paths {
        if path.is_file() && is_rollout_jsonl(path) && seen.insert(path.clone()) {
            selected.push(path.clone());
        }
    }

    Ok(selected)
}

fn collect_rollout_file_candidates(
    dir: &Path,
    files: &mut Vec<RolloutFileCandidate>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|err| format!("读取 Codex sessions 目录失败 {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("读取 Codex session 目录条目失败: {err}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("读取 Codex session 文件类型失败 {}: {err}", path.display()))?;
        if file_type.is_dir() {
            collect_rollout_file_candidates(&path, files)?;
        } else if file_type.is_file() && is_rollout_jsonl(&path) {
            let sort_key = rollout_activity_sort_key(&path);
            files.push(RolloutFileCandidate { path, sort_key });
        }
    }
    Ok(())
}

fn is_rollout_jsonl(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|file_name| file_name.starts_with("rollout-"))
        && path.extension().and_then(|value| value.to_str()) == Some("jsonl")
}

fn rollout_activity_sort_key(path: &Path) -> String {
    read_rollout_tail_timestamp(path)
        .or_else(|| rollout_filename_timestamp(path))
        .or_else(|| rollout_path_date(path))
        .unwrap_or_else(|| {
            path.metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(format_system_time_sort_key)
                .unwrap_or_else(|| "1970-01-01T00:00:00+00:00".to_string())
        })
}

fn read_rollout_tail_timestamp(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let sample_len = len.min(SESSION_SYNC_TAIL_SAMPLE_BYTES);
    if len > sample_len {
        file.seek(SeekFrom::Start(len - sample_len)).ok()?;
    }
    let mut bytes = Vec::with_capacity(sample_len as usize);
    file.take(sample_len).read_to_end(&mut bytes).ok()?;
    if len > sample_len {
        if let Some(index) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes.drain(..=index);
        }
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut latest = None;
    for line in text.lines() {
        if let Some(timestamp) = rollout_line_timestamp(line) {
            if latest.as_ref().is_none_or(|value| timestamp > *value) {
                latest = Some(timestamp);
            }
        }
    }
    latest
}

fn rollout_line_timestamp(line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line).ok()?;
    let timestamp = raw_string_field(&value, "timestamp");
    let timestamp = timestamp.trim();
    if timestamp.is_empty() {
        None
    } else {
        Some(normalize_timestamp_sort_key(timestamp))
    }
}

fn normalize_timestamp_sort_key(timestamp: &str) -> String {
    timestamp.replace('Z', "+00:00")
}

fn format_system_time_sort_key(time: std::time::SystemTime) -> Option<String> {
    let datetime = ::time::OffsetDateTime::from(time);
    datetime
        .format(&::time::format_description::well_known::Rfc3339)
        .ok()
        .map(|timestamp| timestamp.replace('Z', "+00:00"))
}

fn rollout_filename_timestamp(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    let raw = file_name
        .strip_prefix("rollout-")?
        .strip_suffix(".jsonl")?
        .get(..19)?;
    Some(format!(
        "{}T{}:{}:{}",
        &raw[..10],
        &raw[11..13],
        &raw[14..16],
        &raw[17..19]
    ))
}

fn rollout_path_date(path: &Path) -> Option<String> {
    let parts: Vec<String> = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect();
    for window in parts.windows(3) {
        if window[0].len() == 4
            && window[1].len() == 2
            && window[2].len() == 2
            && window
                .iter()
                .all(|part| part.chars().all(|ch| ch.is_ascii_digit()))
        {
            return Some(format!(
                "{}-{}-{}T00:00:00",
                window[0], window[1], window[2]
            ));
        }
    }
    None
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

pub(super) fn update_rollout_provider_line(
    line: &str,
    target_provider: &str,
) -> Result<Option<String>, String> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    // A line can only change if some object in it has a `model_provider` key, or if it is a
    // session_meta event (which gains the key). Neither is possible without these literals, so
    // the other lines (almost all of a rollout) skip the JSON parse.
    if !line.contains("\"model_provider\"") && !line.contains("\"session_meta\"") {
        return Ok(None);
    }

    let mut event: Value = match serde_json::from_str(line) {
        Ok(event) => event,
        Err(_) => return Ok(None),
    };
    if event.get("type").and_then(Value::as_str) != Some("session_meta") {
        if update_model_provider_fields(&mut event, target_provider) {
            return serde_json::to_string(&event)
                .map(Some)
                .map_err(|err| format!("序列化 Codex session 元数据失败: {err}"));
        }
        return Ok(None);
    }

    let mut changed = update_model_provider_fields(&mut event, target_provider);
    let Some(payload) = event.get_mut("payload").and_then(Value::as_object_mut) else {
        return if changed {
            serde_json::to_string(&event)
                .map(Some)
                .map_err(|err| format!("序列化 Codex session 元数据失败: {err}"))
        } else {
            Ok(None)
        };
    };
    if !payload.contains_key("model_provider") {
        payload.insert(
            "model_provider".to_string(),
            Value::String(target_provider.to_string()),
        );
        changed = true;
    }

    if changed {
        serde_json::to_string(&event)
            .map(Some)
            .map_err(|err| format!("序列化 Codex session 元数据失败: {err}"))
    } else {
        Ok(None)
    }
}

fn update_model_provider_fields(value: &mut Value, target_provider: &str) -> bool {
    match value {
        Value::Object(map) => {
            let mut changed = false;
            if map.get("model_provider").and_then(Value::as_str) != Some(target_provider)
                && map.contains_key("model_provider")
            {
                map.insert(
                    "model_provider".to_string(),
                    Value::String(target_provider.to_string()),
                );
                changed = true;
            }
            for value in map.values_mut() {
                changed |= update_model_provider_fields(value, target_provider);
            }
            changed
        }
        Value::Array(items) => {
            let mut changed = false;
            for item in items {
                changed |= update_model_provider_fields(item, target_provider);
            }
            changed
        }
        _ => false,
    }
}
