use crate::{
    accounts::get_codex_state_value,
    api_config::{API_PROVIDER_ID, OPENAI_PROVIDER_ID},
    json_util::raw_string_field,
    paths::codex_dir,
    settings::read_settings_value,
};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    thread,
    time::Duration,
};

const SESSION_SYNC_WATCH_INTERVAL_SECONDS: u64 = 60;
const SESSION_SYNC_FILE_PAUSE_MS: u64 = 15;

struct SessionSyncState {
    running: bool,
    pending_provider: Option<String>,
}

static SESSION_SYNC_STATE: Mutex<SessionSyncState> = Mutex::new(SessionSyncState {
    running: false,
    pending_provider: None,
});

fn current_session_provider() -> Result<String, String> {
    let state = get_codex_state_value();
    let model_provider = raw_string_field(&state, "model_provider");
    if !model_provider.is_empty() {
        return Ok(model_provider);
    }

    match raw_string_field(&state, "mode").as_str() {
        "api" => Ok(API_PROVIDER_ID.to_string()),
        "chatgpt" => Ok(OPENAI_PROVIDER_ID.to_string()),
        _ => Err("当前 Codex 模式未知，无法同步会话".to_string()),
    }
}

fn sessions_dir_path() -> Result<PathBuf, String> {
    Ok(codex_dir()?.join("sessions"))
}

fn state_db_path() -> Result<PathBuf, String> {
    Ok(codex_dir()?.join("state_5.sqlite"))
}

pub(crate) fn queue_codex_sessions_to_current_mode() -> Result<bool, String> {
    let target_provider = current_session_provider()?;
    queue_codex_sessions_to_provider(&target_provider)
}

pub(crate) fn queue_codex_sessions_to_provider(target_provider: &str) -> Result<bool, String> {
    queue_codex_sessions_to_provider_impl(target_provider, true)
}

pub(crate) fn sync_codex_session_index_then_queue_rollouts(
    target_provider: &str,
) -> Result<bool, String> {
    let target_provider = normalize_target_provider(target_provider)?;
    sync_codex_state_threads_to_provider_if_exists(&target_provider)?;
    queue_codex_sessions_to_provider_impl(&target_provider, true)
}

fn queue_codex_sessions_to_current_mode_if_idle() -> Result<bool, String> {
    let target_provider = current_session_provider()?;
    queue_codex_sessions_to_provider_impl(&target_provider, false)
}

fn queue_codex_sessions_to_provider_impl(
    target_provider: &str,
    replace_pending: bool,
) -> Result<bool, String> {
    let target_provider = normalize_target_provider(target_provider)?;
    let mut state = SESSION_SYNC_STATE
        .lock()
        .map_err(|_| "Codex 会话同步状态已损坏".to_string())?;

    if state.running {
        if replace_pending {
            state.pending_provider = Some(target_provider);
            return Ok(true);
        }
        return Ok(false);
    }

    state.running = true;
    state.pending_provider = Some(target_provider);
    thread::spawn(run_queued_session_sync_worker);
    Ok(true)
}

fn normalize_target_provider(target_provider: &str) -> Result<String, String> {
    let target_provider = target_provider.trim();
    if target_provider.is_empty() {
        return Err("当前 Codex provider 为空，无法同步会话".to_string());
    }
    Ok(target_provider.to_string())
}

fn sync_codex_sessions_to_provider_now(target_provider: &str) -> Result<usize, String> {
    let target_provider = normalize_target_provider(target_provider)?;
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
    let sessions_dir = sessions_dir_path()?;
    if !sessions_dir.exists() {
        return Ok(0);
    }
    sync_codex_session_rollouts_to_provider(&sessions_dir, target_provider)
}

fn sync_codex_state_threads_to_provider_if_exists(target_provider: &str) -> Result<usize, String> {
    let state_db = state_db_path()?;
    sync_codex_state_threads_to_provider(&state_db, target_provider)
}

fn run_queued_session_sync_worker() {
    let mut last_error = String::new();
    loop {
        let target_provider = match take_pending_session_sync_provider() {
            Ok(Some(target_provider)) => target_provider,
            Ok(None) => break,
            Err(err) => {
                eprintln!("Codex 会话同步失败: {err}");
                break;
            }
        };

        match sync_codex_sessions_to_provider_now(&target_provider) {
            Ok(_) => last_error.clear(),
            Err(err) if err != last_error => {
                eprintln!("Codex 会话同步失败: {err}");
                last_error = err;
            }
            Err(_) => {}
        }
    }
}

fn take_pending_session_sync_provider() -> Result<Option<String>, String> {
    let mut state = SESSION_SYNC_STATE
        .lock()
        .map_err(|_| "Codex 会话同步状态已损坏".to_string())?;
    if let Some(target_provider) = state.pending_provider.take() {
        return Ok(Some(target_provider));
    }
    state.running = false;
    Ok(None)
}

fn sync_codex_session_rollouts_to_provider(
    sessions_dir: &Path,
    target_provider: &str,
) -> Result<usize, String> {
    let mut updated = 0;
    let mut errors = Vec::new();
    sync_rollouts_from_dir(sessions_dir, target_provider, &mut updated, &mut errors)?;

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

fn sync_rollouts_from_dir(
    dir: &Path,
    target_provider: &str,
    updated: &mut usize,
    errors: &mut Vec<String>,
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
            sync_rollouts_from_dir(&path, target_provider, updated, errors)?;
        } else if file_type.is_file() && is_rollout_jsonl(&path) {
            match sync_rollout_file_provider(&path, target_provider) {
                Ok(true) => *updated += 1,
                Ok(false) => {}
                Err(err) => errors.push(err),
            }
            thread::sleep(Duration::from_millis(SESSION_SYNC_FILE_PAUSE_MS));
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

fn sync_rollout_file_provider(path: &Path, target_provider: &str) -> Result<bool, String> {
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

fn codex_session_sync_enabled() -> bool {
    read_settings_value()
        .ok()
        .and_then(|settings| {
            settings
                .get("codex_session_sync_enabled")
                .and_then(Value::as_bool)
        })
        .unwrap_or(true)
}

pub(crate) fn start_codex_session_sync_watcher() {
    thread::spawn(move || {
        let mut last_error = String::new();
        loop {
            if codex_session_sync_enabled() {
                match queue_codex_sessions_to_current_mode_if_idle() {
                    Ok(_) => last_error.clear(),
                    Err(err) if err != last_error => {
                        eprintln!("Codex 会话同步失败: {err}");
                        last_error = err;
                    }
                    Err(_) => {}
                }
            }
            thread::sleep(Duration::from_secs(SESSION_SYNC_WATCH_INTERVAL_SECONDS));
        }
    });
}

#[tauri::command]
pub(crate) fn sync_codex_sessions() -> Result<Value, String> {
    let queued = queue_codex_sessions_to_current_mode()?;
    let message = if queued {
        "会话同步已转入后台".to_string()
    } else {
        "会话同步正在后台进行".to_string()
    };
    Ok(json!({
        "ok": true,
        "queued": queued,
        "message": message
    }))
}

#[cfg(test)]
mod tests;
