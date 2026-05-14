use crate::{json_util::raw_string_field, paths::codex_dir};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

const SESSION_SYNC_RECENT_ROLLOUT_LIMIT: usize = 50;
const SESSION_SYNC_TAIL_SAMPLE_BYTES: u64 = 128 * 1024;
const GLOBAL_STATE_FILE_NAME: &str = ".codex-global-state.json";

static SESSION_SYNC_IO_LOCK: Mutex<()> = Mutex::new(());

fn session_rollout_dir_paths() -> Result<Vec<PathBuf>, String> {
    let codex_dir = codex_dir()?;
    Ok(vec![
        codex_dir.join("sessions"),
        codex_dir.join("archived_sessions"),
    ])
}

fn state_db_path() -> Result<PathBuf, String> {
    Ok(codex_dir()?.join("state_5.sqlite"))
}

fn global_state_path() -> Result<PathBuf, String> {
    Ok(codex_dir()?.join(GLOBAL_STATE_FILE_NAME))
}

fn normalize_target_provider(target_provider: &str) -> Result<String, String> {
    let target_provider = target_provider.trim();
    if target_provider.is_empty() {
        return Err("当前 Codex provider 为空，无法同步会话".to_string());
    }
    Ok(target_provider.to_string())
}

pub(crate) fn sync_codex_sessions_to_provider_now(target_provider: &str) -> Result<usize, String> {
    let target_provider = normalize_target_provider(target_provider)?;
    let _guard = SESSION_SYNC_IO_LOCK
        .lock()
        .map_err(|_| "Codex 会话同步 I/O 状态已损坏".to_string())?;
    let mut updated = 0;
    let mut errors = Vec::new();

    match sync_codex_state_threads_to_provider_if_exists(&target_provider) {
        Ok(count) => updated += count,
        Err(err) => errors.push(err),
    }
    match sync_codex_session_rollouts_to_provider_if_exists(&target_provider) {
        Ok(count) => updated += count,
        Err(err) => errors.push(err),
    }

    if errors.is_empty() {
        Ok(updated)
    } else {
        Err(format!(
            "同步 Codex 会话失败，已更新 {updated} 项：{}",
            errors.join("；")
        ))
    }
}

fn sync_codex_session_rollouts_to_provider_if_exists(
    target_provider: &str,
) -> Result<usize, String> {
    sync_codex_session_rollout_dirs_to_provider(
        &session_rollout_dir_paths()?,
        target_provider,
        &pinned_thread_rollout_paths_if_exists()?,
    )
}

fn sync_codex_state_threads_to_provider_if_exists(target_provider: &str) -> Result<usize, String> {
    let state_db = state_db_path()?;
    sync_codex_state_threads_to_provider(&state_db, target_provider)
}

#[cfg(test)]
fn sync_codex_session_rollouts_to_provider(
    sessions_dir: &Path,
    target_provider: &str,
) -> Result<usize, String> {
    let rollout_files =
        collect_recent_rollout_files(sessions_dir, SESSION_SYNC_RECENT_ROLLOUT_LIMIT)?;
    sync_rollout_files_to_provider(rollout_files, target_provider)
}

fn sync_codex_session_rollout_dirs_to_provider(
    rollout_dirs: &[PathBuf],
    target_provider: &str,
    extra_rollout_paths: &[PathBuf],
) -> Result<usize, String> {
    let rollout_files = collect_recent_rollout_files_from_dirs(
        rollout_dirs,
        SESSION_SYNC_RECENT_ROLLOUT_LIMIT,
        extra_rollout_paths,
    )?;
    sync_rollout_files_to_provider(rollout_files, target_provider)
}

fn sync_rollout_files_to_provider(
    rollout_files: Vec<PathBuf>,
    target_provider: &str,
) -> Result<usize, String> {
    let mut updated = 0;
    let mut errors = Vec::new();

    for path in rollout_files {
        match sync_rollout_file_provider(&path, target_provider) {
            Ok(true) => updated += 1,
            Ok(false) => {}
            Err(err) => errors.push(err),
        }
    }

    if errors.is_empty() {
        Ok(updated)
    } else {
        Err(format!(
            "同步 Codex 会话失败，已更新 {updated} 个文件，{} 个文件失败：{}",
            errors.len(),
            errors.join("；")
        ))
    }
}

#[derive(Debug, Eq, PartialEq)]
struct RolloutFileCandidate {
    path: PathBuf,
    sort_key: String,
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

fn sync_rollout_file_provider(path: &Path, target_provider: &str) -> Result<bool, String> {
    let original_modified = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(|err| format!("读取 Codex session 修改时间失败 {}: {err}", path.display()))?;
    let content = fs::read_to_string(path)
        .map_err(|err| format!("读取 Codex session 文件失败 {}: {err}", path.display()))?;
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
        fs::write(path, updated_content)
            .map_err(|err| format!("写入 Codex session 文件失败 {}: {err}", path.display()))?;
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .and_then(|file| file.set_modified(original_modified))
            .map_err(|err| format!("恢复 Codex session 修改时间失败 {}: {err}", path.display()))?;
    }
    Ok(changed)
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

fn update_rollout_provider_line(
    line: &str,
    target_provider: &str,
) -> Result<Option<String>, String> {
    if line.trim().is_empty() {
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

fn pinned_thread_rollout_paths_if_exists() -> Result<Vec<PathBuf>, String> {
    pinned_thread_rollout_paths(&global_state_path()?, &state_db_path()?)
}

fn pinned_thread_rollout_paths(
    global_state: &Path,
    state_db: &Path,
) -> Result<Vec<PathBuf>, String> {
    if !global_state.exists() || !state_db.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(global_state).map_err(|err| {
        format!(
            "读取 Codex global state 失败 {}: {err}",
            global_state.display()
        )
    })?;
    let state: Value = serde_json::from_str(&content).map_err(|err| {
        format!(
            "解析 Codex global state 失败 {}: {err}",
            global_state.display()
        )
    })?;
    let pinned_thread_ids = state
        .get("pinned-thread-ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if pinned_thread_ids.is_empty() {
        return Ok(Vec::new());
    }

    let connection = Connection::open_with_flags(
        state_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| format!("打开 Codex state 数据库失败 {}: {err}", state_db.display()))?;
    let mut statement = connection
        .prepare("SELECT rollout_path FROM threads WHERE id = ?1")
        .map_err(|err| {
            format!(
                "读取 Codex pinned thread rollout 查询失败 {}: {err}",
                state_db.display()
            )
        })?;

    let mut paths = Vec::new();
    for thread_id in pinned_thread_ids {
        let thread_id = thread_id.as_str().unwrap_or("").trim();
        if thread_id.is_empty() {
            continue;
        }
        match statement.query_row([thread_id], |row| row.get::<_, String>(0)) {
            Ok(path) if !path.trim().is_empty() => paths.push(PathBuf::from(path)),
            Ok(_) | Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(err) => {
                return Err(format!(
                    "读取 Codex pinned thread rollout 失败 {thread_id}: {err}"
                ))
            }
        }
    }

    Ok(paths)
}

fn sync_codex_state_threads_to_provider(
    state_db: &Path,
    target_provider: &str,
) -> Result<usize, String> {
    if !state_db.exists() {
        return Ok(0);
    }
    let mut connection = Connection::open_with_flags(
        state_db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| format!("打开 Codex state 数据库失败 {}: {err}", state_db.display()))?;
    connection
        .busy_timeout(Duration::from_millis(3000))
        .map_err(|err| format!("配置 Codex state 数据库等待超时失败: {err}"))?;
    let transaction = connection
        .transaction()
        .map_err(|err| format!("开始 Codex state 会话同步事务失败: {err}"))?;
    let updated = transaction
        .execute(
            "UPDATE threads
             SET model_provider = ?1
             WHERE model_provider IS NULL
                OR model_provider <> ?1",
            [target_provider],
        )
        .map_err(|err| {
            format!(
                "更新 Codex state 会话 provider 失败 {}: {err}",
                state_db.display()
            )
        })?;
    transaction
        .commit()
        .map_err(|err| format!("保存 Codex state 会话同步结果失败: {err}"))?;
    Ok(updated)
}

#[cfg(test)]
mod tests;
