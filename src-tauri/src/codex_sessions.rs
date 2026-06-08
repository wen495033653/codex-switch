use crate::{
    accounts::{get_codex_state_value, restore_api_mode_if_selected},
    api_config::API_PROVIDER_ID,
    json_util::raw_string_field,
    paths::codex_dir,
    session_sync_diagnostics::log_session_sync_event,
};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{ErrorKind, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::Duration,
};

const SESSION_SYNC_RECENT_ROLLOUT_LIMIT: usize = 50;
const SESSION_SYNC_TAIL_SAMPLE_BYTES: u64 = 128 * 1024;
const GLOBAL_STATE_FILE_NAME: &str = ".codex-global-state.json";
const OPENAI_PROVIDER_ID: &str = "openai";

static CODEX_SESSION_IO_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Default)]
struct StateThreadSyncMetadata {
    user_event_thread_ids: HashSet<String>,
    cwd_by_thread_id: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default)]
struct StateThreadUpdateCounts {
    provider_rows: usize,
    user_event_rows: usize,
    cwd_rows: usize,
}

impl StateThreadUpdateCounts {
    fn total(self) -> usize {
        self.provider_rows + self.user_event_rows + self.cwd_rows
    }
}

#[derive(Debug, Default)]
struct RolloutThreadMetadata {
    cwd: Option<String>,
    has_user_event: bool,
}

pub(crate) fn lock_codex_session_io(action: &str) -> Result<MutexGuard<'static, ()>, String> {
    CODEX_SESSION_IO_LOCK
        .lock()
        .map_err(|_| format!("{action} I/O 状态已损坏"))
}

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

pub(crate) fn current_session_provider() -> Result<String, String> {
    restore_api_mode_if_selected()?;
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

fn normalize_target_provider(target_provider: &str) -> Result<String, String> {
    let target_provider = target_provider.trim();
    if target_provider.is_empty() {
        return Err("当前 Codex provider 为空，无法同步会话".to_string());
    }
    Ok(target_provider.to_string())
}

#[allow(dead_code)]
pub(crate) fn sync_codex_sessions_to_current_mode_now() -> Result<usize, String> {
    sync_codex_sessions_to_current_mode_now_from("current_mode")
}

pub(crate) fn sync_codex_sessions_to_current_mode_now_from(trigger: &str) -> Result<usize, String> {
    log_session_sync_event(
        "session_sync_current_mode_resolve_start",
        json!({ "trigger": trigger }),
    );
    let target_provider = current_session_provider()?;
    log_session_sync_event(
        "session_sync_current_mode_resolved",
        json!({
            "trigger": trigger,
            "targetProvider": target_provider
        }),
    );
    sync_codex_sessions_to_provider_now_from(&target_provider, trigger)
}

pub(crate) fn preview_codex_sessions_to_current_mode_now_from(
    trigger: &str,
) -> Result<usize, String> {
    log_session_sync_event(
        "session_sync_preflight_current_mode_resolve_start",
        json!({ "trigger": trigger }),
    );
    let target_provider = current_session_provider()?;
    log_session_sync_event(
        "session_sync_preflight_current_mode_resolved",
        json!({
            "trigger": trigger,
            "targetProvider": target_provider
        }),
    );
    preview_codex_sessions_to_provider_now_from(&target_provider, trigger)
}

#[allow(dead_code)]
pub(crate) fn sync_codex_sessions_to_provider_now(target_provider: &str) -> Result<usize, String> {
    sync_codex_sessions_to_provider_now_from(target_provider, "explicit_provider")
}

pub(crate) fn preview_codex_sessions_to_provider_now_from(
    target_provider: &str,
    trigger: &str,
) -> Result<usize, String> {
    let target_provider = normalize_target_provider(target_provider)?;
    log_session_sync_event(
        "session_sync_preflight_start",
        json!({
            "trigger": trigger,
            "targetProvider": target_provider
        }),
    );
    let _guard = lock_codex_session_io("Codex 会话同步")?;
    let mut updated = 0;
    let mut state_db_updated = 0;
    let mut rollout_files_updated = 0;
    let mut global_state_updated = 0;
    let mut errors = Vec::new();

    match preview_codex_state_threads_to_provider_if_exists(&target_provider, trigger) {
        Ok(count) => {
            state_db_updated = count;
            updated += count;
        }
        Err(err) => errors.push(err),
    }
    match preview_codex_session_rollouts_to_provider_if_exists(&target_provider, trigger) {
        Ok(count) => {
            rollout_files_updated = count;
            updated += count;
        }
        Err(err) => errors.push(err),
    }
    match preview_codex_global_state_workspace_roots_if_exists(trigger) {
        Ok(count) => {
            global_state_updated = count;
            updated += count;
        }
        Err(err) => errors.push(err),
    }

    if errors.is_empty() {
        log_session_sync_event(
            "session_sync_preflight_finish",
            json!({
                "trigger": trigger,
                "targetProvider": target_provider,
                "stateDbUpdated": state_db_updated,
                "rolloutFilesUpdated": rollout_files_updated,
                "globalStateUpdated": global_state_updated,
                "updated": updated
            }),
        );
        Ok(updated)
    } else {
        log_session_sync_event(
            "session_sync_preflight_error",
            json!({
                "trigger": trigger,
                "targetProvider": target_provider,
                "stateDbUpdated": state_db_updated,
                "rolloutFilesUpdated": rollout_files_updated,
                "globalStateUpdated": global_state_updated,
                "updated": updated,
                "errors": errors.clone()
            }),
        );
        Err(format!(
            "预检查 Codex 会话同步失败，预计更新 {updated} 项：{}",
            errors.join("；")
        ))
    }
}

pub(crate) fn sync_codex_sessions_to_provider_now_from(
    target_provider: &str,
    trigger: &str,
) -> Result<usize, String> {
    let target_provider = normalize_target_provider(target_provider)?;
    log_session_sync_event(
        "session_sync_start",
        json!({
            "trigger": trigger,
            "targetProvider": target_provider
        }),
    );
    let _guard = lock_codex_session_io("Codex 会话同步")?;
    let mut updated = 0;
    let mut state_db_updated = 0;
    let mut rollout_files_updated = 0;
    let mut global_state_updated = 0;
    let mut errors = Vec::new();

    match sync_codex_state_threads_to_provider_if_exists(&target_provider, trigger) {
        Ok(count) => {
            state_db_updated = count;
            updated += count;
        }
        Err(err) => errors.push(err),
    }
    match sync_codex_session_rollouts_to_provider_if_exists(&target_provider, trigger) {
        Ok(count) => {
            rollout_files_updated = count;
            updated += count;
        }
        Err(err) => errors.push(err),
    }
    match sync_codex_global_state_workspace_roots_if_exists(trigger) {
        Ok(count) => {
            global_state_updated = count;
            updated += count;
        }
        Err(err) => errors.push(err),
    }

    if errors.is_empty() {
        log_session_sync_event(
            "session_sync_finish",
            json!({
                "trigger": trigger,
                "targetProvider": target_provider,
                "stateDbUpdated": state_db_updated,
                "rolloutFilesUpdated": rollout_files_updated,
                "globalStateUpdated": global_state_updated,
                "updated": updated
            }),
        );
        Ok(updated)
    } else {
        log_session_sync_event(
            "session_sync_error",
            json!({
                "trigger": trigger,
                "targetProvider": target_provider,
                "stateDbUpdated": state_db_updated,
                "rolloutFilesUpdated": rollout_files_updated,
                "globalStateUpdated": global_state_updated,
                "updated": updated,
                "errors": errors.clone()
            }),
        );
        Err(format!(
            "同步 Codex 会话失败，已更新 {updated} 项：{}",
            errors.join("；")
        ))
    }
}

fn preview_codex_session_rollouts_to_provider_if_exists(
    target_provider: &str,
    trigger: &str,
) -> Result<usize, String> {
    preview_codex_session_rollout_dirs_to_provider_with_diagnostics(
        &session_rollout_dir_paths()?,
        target_provider,
        &pinned_thread_rollout_paths_if_exists()?,
        Some(trigger),
    )
}

fn sync_codex_session_rollouts_to_provider_if_exists(
    target_provider: &str,
    trigger: &str,
) -> Result<usize, String> {
    sync_codex_session_rollout_dirs_to_provider_with_diagnostics(
        &session_rollout_dir_paths()?,
        target_provider,
        &pinned_thread_rollout_paths_if_exists()?,
        Some(trigger),
    )
}

fn sync_codex_state_threads_to_provider_if_exists(
    target_provider: &str,
    trigger: &str,
) -> Result<usize, String> {
    let state_db = state_db_path()?;
    sync_codex_state_threads_to_provider_with_diagnostics(&state_db, target_provider, Some(trigger))
}

fn preview_codex_state_threads_to_provider_if_exists(
    target_provider: &str,
    trigger: &str,
) -> Result<usize, String> {
    let state_db = state_db_path()?;
    preview_codex_state_threads_to_provider_with_diagnostics(
        &state_db,
        target_provider,
        Some(trigger),
    )
}

fn sync_codex_global_state_workspace_roots_if_exists(trigger: &str) -> Result<usize, String> {
    let path = global_state_path()?;
    sync_global_state_workspace_roots_with_diagnostics(&path, Some(trigger))
}

fn preview_codex_global_state_workspace_roots_if_exists(trigger: &str) -> Result<usize, String> {
    let path = global_state_path()?;
    preview_global_state_workspace_roots_with_diagnostics(&path, Some(trigger))
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

#[cfg(test)]
fn sync_codex_session_rollout_dirs_to_provider(
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

fn sync_codex_session_rollout_dirs_to_provider_with_diagnostics(
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

fn preview_codex_session_rollout_dirs_to_provider_with_diagnostics(
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

pub(crate) fn collect_recent_codex_rollout_files_for_remote_control(
    rollout_dirs: &[PathBuf],
    limit: usize,
) -> Result<Vec<PathBuf>, String> {
    collect_recent_rollout_files_from_dirs(rollout_dirs, limit, &[])
}

pub(crate) fn sync_codex_rollout_files_to_provider_for_remote_control(
    rollout_files: Vec<PathBuf>,
    target_provider: &str,
) -> Result<usize, String> {
    sync_rollout_files_to_provider_with_diagnostics(rollout_files, target_provider, None)
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

#[derive(Debug)]
struct RolloutFileSyncOutcome {
    changed: bool,
    from_provider: Option<String>,
}

fn provider_log_value(provider: &str) -> String {
    let provider = provider.trim();
    if provider.is_empty() {
        "(未设置)".to_string()
    } else {
        provider.to_string()
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

fn write_existing_file(path: &Path, content: &str, action: &str) -> Result<bool, String> {
    let mut file = match fs::OpenOptions::new().write(true).truncate(true).open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(format!("{action}失败 {}: {err}", path.display())),
    };
    file.write_all(content.as_bytes())
        .map_err(|err| format!("{action}失败 {}: {err}", path.display()))?;
    Ok(true)
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

fn state_threads_columns(connection: &Connection) -> Result<Option<HashSet<String>>, String> {
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'threads')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| format!("检查 Codex state threads 表失败: {err}"))?;
    if exists == 0 {
        return Ok(None);
    }

    let mut statement = connection
        .prepare("PRAGMA table_info(threads)")
        .map_err(|err| format!("读取 Codex state threads 表结构失败: {err}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| format!("读取 Codex state threads 表结构失败: {err}"))?;
    let mut columns = HashSet::new();
    for row in rows {
        columns.insert(row.map_err(|err| format!("读取 Codex state threads 列失败: {err}"))?);
    }
    Ok(Some(columns))
}

fn collect_state_thread_sync_metadata(
    connection: &Connection,
    state_db: &Path,
    columns: &HashSet<String>,
) -> Result<StateThreadSyncMetadata, String> {
    let wants_user_event = columns.contains("has_user_event");
    let wants_cwd = columns.contains("cwd");
    if !wants_user_event && !wants_cwd {
        return Ok(StateThreadSyncMetadata::default());
    }
    if !columns.contains("id") || !columns.contains("rollout_path") {
        return Ok(StateThreadSyncMetadata::default());
    }

    let mut statement = connection
        .prepare(
            "SELECT id, rollout_path
             FROM threads
             WHERE COALESCE(rollout_path, '') <> ''",
        )
        .map_err(|err| format!("查询 Codex state 会话 rollout 路径失败: {err}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|err| format!("读取 Codex state 会话 rollout 路径失败: {err}"))?;

    let mut metadata = StateThreadSyncMetadata::default();
    let root = state_db.parent().unwrap_or_else(|| Path::new(""));
    for row in rows {
        let (thread_id, rollout_path) =
            row.map_err(|err| format!("读取 Codex state 会话 rollout 路径失败: {err}"))?;
        let thread_id = thread_id.trim();
        let Some(rollout_path) = rollout_path else {
            continue;
        };
        if thread_id.is_empty() || rollout_path.trim().is_empty() {
            continue;
        }
        let rollout_path = state_thread_rollout_path(root, &rollout_path);
        let Some(rollout_metadata) = rollout_thread_metadata(&rollout_path)? else {
            continue;
        };
        if wants_user_event && rollout_metadata.has_user_event {
            metadata.user_event_thread_ids.insert(thread_id.to_string());
        }
        if wants_cwd {
            if let Some(cwd) = rollout_metadata.cwd {
                metadata.cwd_by_thread_id.insert(thread_id.to_string(), cwd);
            }
        }
    }
    Ok(metadata)
}

fn state_thread_rollout_path(root: &Path, raw_path: &str) -> PathBuf {
    let path = PathBuf::from(raw_path.trim());
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn rollout_thread_metadata(path: &Path) -> Result<Option<RolloutThreadMetadata>, String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) if is_locked_io_error(&err) => return Ok(None),
        Err(err) => {
            return Err(format!(
                "读取 Codex state 会话 rollout 元数据失败 {}: {err}",
                path.display()
            ))
        }
    };
    let mut metadata = RolloutThreadMetadata {
        cwd: None,
        has_user_event: content.contains("\"user_message\"") || content.contains("\"user_input\""),
    };

    for line in content.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        let Some(payload) = event.get("payload").and_then(Value::as_object) else {
            continue;
        };
        metadata.cwd = payload
            .get("cwd")
            .and_then(Value::as_str)
            .and_then(to_desktop_workspace_path);
        break;
    }

    Ok(Some(metadata))
}

fn is_locked_io_error(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::PermissionDenied)
        || matches!(error.raw_os_error(), Some(32 | 33))
}

fn to_desktop_workspace_path(value: &str) -> Option<String> {
    let stripped = value.trim();
    if stripped.is_empty() {
        return None;
    }
    let lower = stripped.to_ascii_lowercase();
    if lower.starts_with(r"\\?\unc\") {
        return Some(format!(r"\\{}", stripped[8..].replace('/', "\\")));
    }
    if stripped.starts_with(r"\\?\") {
        return Some(stripped[4..].replace('\\', "/"));
    }
    Some(stripped.to_string())
}

#[cfg(test)]
fn sync_codex_state_threads_to_provider(
    state_db: &Path,
    target_provider: &str,
) -> Result<usize, String> {
    sync_codex_state_threads_to_provider_with_diagnostics(state_db, target_provider, None)
}

fn sync_codex_state_threads_to_provider_with_diagnostics(
    state_db: &Path,
    target_provider: &str,
    trigger: Option<&str>,
) -> Result<usize, String> {
    if !state_db.exists() {
        if let Some(trigger) = trigger {
            log_session_sync_event(
                "session_sync_state_db_missing",
                json!({
                    "trigger": trigger,
                    "targetProvider": target_provider,
                    "stateDb": state_db.to_string_lossy().to_string()
                }),
            );
        }
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
    let Some(columns) = state_threads_columns(&connection)? else {
        if let Some(trigger) = trigger {
            log_session_sync_event(
                "session_sync_state_db_threads_missing",
                json!({
                    "trigger": trigger,
                    "targetProvider": target_provider,
                    "stateDb": state_db.to_string_lossy().to_string()
                }),
            );
        }
        return Ok(0);
    };
    if !columns.contains("model_provider") {
        if let Some(trigger) = trigger {
            log_session_sync_event(
                "session_sync_state_db_threads_unsupported",
                json!({
                    "trigger": trigger,
                    "targetProvider": target_provider,
                    "stateDb": state_db.to_string_lossy().to_string(),
                    "missingColumn": "model_provider"
                }),
            );
        }
        return Ok(0);
    }
    let thread_metadata = collect_state_thread_sync_metadata(&connection, state_db, &columns)?;
    let before_summary = trigger
        .map(|_| query_state_db_summary(&connection, target_provider))
        .transpose();
    let transaction = connection
        .transaction()
        .map_err(|err| format!("开始 Codex state 会话同步事务失败: {err}"))?;
    let mut counts = StateThreadUpdateCounts::default();
    counts.provider_rows = transaction
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
    if columns.contains("has_user_event") {
        for thread_id in &thread_metadata.user_event_thread_ids {
            counts.user_event_rows += transaction
                .execute(
                    "UPDATE threads
                     SET has_user_event = 1
                     WHERE id = ?1
                        AND COALESCE(has_user_event, 0) <> 1",
                    [thread_id],
                )
                .map_err(|err| {
                    format!(
                        "更新 Codex state 会话 user event 状态失败 {}: {err}",
                        state_db.display()
                    )
                })?;
        }
    }
    if columns.contains("cwd") {
        for (thread_id, cwd) in &thread_metadata.cwd_by_thread_id {
            counts.cwd_rows += transaction
                .execute(
                    "UPDATE threads
                     SET cwd = ?1
                     WHERE id = ?2
                        AND COALESCE(cwd, '') <> ?1",
                    (cwd, thread_id),
                )
                .map_err(|err| {
                    format!(
                        "更新 Codex state 会话 cwd 失败 {}: {err}",
                        state_db.display()
                    )
                })?;
        }
    }
    transaction
        .commit()
        .map_err(|err| format!("保存 Codex state 会话同步结果失败: {err}"))?;
    let updated = counts.total();
    log_state_db_update_summary(state_db, target_provider, trigger, counts, before_summary);
    Ok(updated)
}

fn preview_codex_state_threads_to_provider_with_diagnostics(
    state_db: &Path,
    target_provider: &str,
    trigger: Option<&str>,
) -> Result<usize, String> {
    if !state_db.exists() {
        if let Some(trigger) = trigger {
            log_session_sync_event(
                "session_sync_preflight_state_db_missing",
                json!({
                    "trigger": trigger,
                    "targetProvider": target_provider,
                    "stateDb": state_db.to_string_lossy().to_string()
                }),
            );
        }
        return Ok(0);
    }
    let connection = Connection::open_with_flags(
        state_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| format!("打开 Codex state 数据库失败 {}: {err}", state_db.display()))?;
    connection
        .busy_timeout(Duration::from_millis(3000))
        .map_err(|err| format!("配置 Codex state 数据库等待超时失败: {err}"))?;
    let Some(columns) = state_threads_columns(&connection)? else {
        if let Some(trigger) = trigger {
            log_session_sync_event(
                "session_sync_preflight_state_db_threads_missing",
                json!({
                    "trigger": trigger,
                    "targetProvider": target_provider,
                    "stateDb": state_db.to_string_lossy().to_string()
                }),
            );
        }
        return Ok(0);
    };
    if !columns.contains("model_provider") {
        if let Some(trigger) = trigger {
            log_session_sync_event(
                "session_sync_preflight_state_db_threads_unsupported",
                json!({
                    "trigger": trigger,
                    "targetProvider": target_provider,
                    "stateDb": state_db.to_string_lossy().to_string(),
                    "missingColumn": "model_provider"
                }),
            );
        }
        return Ok(0);
    }
    let thread_metadata = collect_state_thread_sync_metadata(&connection, state_db, &columns)?;
    let mut counts = StateThreadUpdateCounts::default();
    counts.provider_rows = connection
        .query_row(
            "SELECT COUNT(*) FROM threads
             WHERE model_provider IS NULL
                OR model_provider <> ?1",
            [target_provider],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| {
            format!(
                "统计 Codex state 会话 provider 待同步数量失败 {}: {err}",
                state_db.display()
            )
        })? as usize;
    if columns.contains("has_user_event") {
        for thread_id in &thread_metadata.user_event_thread_ids {
            counts.user_event_rows += connection
                .query_row(
                    "SELECT COUNT(*) FROM threads
                     WHERE id = ?1
                        AND COALESCE(has_user_event, 0) <> 1",
                    [thread_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|err| {
                    format!(
                        "统计 Codex state 会话 user event 待同步数量失败 {}: {err}",
                        state_db.display()
                    )
                })? as usize;
        }
    }
    if columns.contains("cwd") {
        for (thread_id, cwd) in &thread_metadata.cwd_by_thread_id {
            counts.cwd_rows += connection
                .query_row(
                    "SELECT COUNT(*) FROM threads
                     WHERE id = ?1
                        AND COALESCE(cwd, '') <> ?2",
                    (thread_id, cwd),
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|err| {
                    format!(
                        "统计 Codex state 会话 cwd 待同步数量失败 {}: {err}",
                        state_db.display()
                    )
                })? as usize;
        }
    }
    let updated = counts.total();
    if let Some(trigger) = trigger {
        log_session_sync_event(
            "session_sync_preflight_state_db_summary",
            json!({
                "trigger": trigger,
                "targetProvider": target_provider,
                "stateDb": state_db.to_string_lossy().to_string(),
                "providerRowsUpdated": counts.provider_rows,
                "userEventRowsUpdated": counts.user_event_rows,
                "cwdRowsUpdated": counts.cwd_rows,
                "updated": updated
            }),
        );
    }
    Ok(updated)
}

fn log_state_db_update_summary(
    state_db: &Path,
    target_provider: &str,
    trigger: Option<&str>,
    counts: StateThreadUpdateCounts,
    summary: Result<Option<Value>, String>,
) {
    let Some(trigger) = trigger else {
        return;
    };
    let updated = counts.total();
    let summary = match summary {
        Ok(Some(summary)) => summary,
        Ok(None) if updated == 0 => return,
        Ok(None) => json!({}),
        Err(err) => json!({ "summaryError": err }),
    };
    if updated == 0 && summary.get("summaryError").is_none() {
        return;
    }
    log_session_sync_event(
        "session_sync_state_db_summary",
        json!({
            "trigger": trigger,
            "targetProvider": target_provider,
            "stateDb": state_db.to_string_lossy().to_string(),
            "providerRowsUpdated": counts.provider_rows,
            "userEventRowsUpdated": counts.user_event_rows,
            "cwdRowsUpdated": counts.cwd_rows,
            "updated": updated,
            "summary": summary
        }),
    );
}

fn query_state_db_summary(connection: &Connection, target_provider: &str) -> Result<Value, String> {
    let total_threads = connection
        .query_row("SELECT COUNT(*) FROM threads", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|err| format!("统计 Codex state threads 总数失败: {err}"))?;
    let target_threads = connection
        .query_row(
            "SELECT COUNT(*) FROM threads WHERE model_provider = ?1",
            [target_provider],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| format!("统计 Codex state target provider 数量失败: {err}"))?;
    let would_update = connection
        .query_row(
            "SELECT COUNT(*) FROM threads
             WHERE model_provider IS NULL
                OR model_provider <> ?1",
            [target_provider],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| format!("统计 Codex state 待同步数量失败: {err}"))?;
    let mut provider_counts_statement = connection
        .prepare(
            "SELECT COALESCE(model_provider, ''), COUNT(*)
             FROM threads
             GROUP BY COALESCE(model_provider, '')
             ORDER BY COUNT(*) DESC, COALESCE(model_provider, '')",
        )
        .map_err(|err| format!("统计 Codex state provider 分布失败: {err}"))?;
    let provider_counts = provider_counts_statement
        .query_map([], |row| {
            let provider = row.get::<_, String>(0)?;
            let threads = row.get::<_, i64>(1)?;
            Ok(json!({
                "provider": provider_log_value(&provider),
                "threads": threads
            }))
        })
        .map_err(|err| format!("读取 Codex state provider 分布失败: {err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("读取 Codex state provider 分布失败: {err}"))?;
    let mut provider_changes_statement = connection
        .prepare(
            "SELECT COALESCE(model_provider, ''), COUNT(*)
             FROM threads
             WHERE model_provider IS NULL
                OR model_provider <> ?1
             GROUP BY COALESCE(model_provider, '')
             ORDER BY COUNT(*) DESC, COALESCE(model_provider, '')",
        )
        .map_err(|err| format!("统计 Codex state provider 变更失败: {err}"))?;
    let provider_changes = provider_changes_statement
        .query_map([target_provider], |row| {
            let from_provider = row.get::<_, String>(0)?;
            let threads = row.get::<_, i64>(1)?;
            Ok(json!({
                "fromProvider": provider_log_value(&from_provider),
                "toProvider": target_provider,
                "threads": threads
            }))
        })
        .map_err(|err| format!("读取 Codex state provider 变更失败: {err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("读取 Codex state provider 变更失败: {err}"))?;
    Ok(json!({
        "totalThreads": total_threads,
        "targetProviderThreads": target_threads,
        "wouldUpdateThreads": would_update,
        "providerCounts": provider_counts,
        "providerChanges": provider_changes
    }))
}

#[cfg(test)]
fn sync_global_state_workspace_roots(path: &Path) -> Result<usize, String> {
    sync_global_state_workspace_roots_with_diagnostics(path, None)
}

#[cfg(test)]
fn preview_global_state_workspace_roots(path: &Path) -> Result<usize, String> {
    preview_global_state_workspace_roots_with_diagnostics(path, None)
}

fn preview_global_state_workspace_roots_with_diagnostics(
    path: &Path,
    trigger: Option<&str>,
) -> Result<usize, String> {
    let state = load_global_state(path)?;
    let next = normalized_global_state_workspace_roots(&state);
    let updated = global_state_update_count(&state, &next);
    if let Some(trigger) = trigger {
        log_session_sync_event(
            "session_sync_preflight_global_state_summary",
            json!({
                "trigger": trigger,
                "globalState": path.to_string_lossy().to_string(),
                "updated": updated
            }),
        );
    }
    Ok(updated)
}

fn sync_global_state_workspace_roots_with_diagnostics(
    path: &Path,
    trigger: Option<&str>,
) -> Result<usize, String> {
    if !path.exists() {
        if let Some(trigger) = trigger {
            log_session_sync_event(
                "session_sync_global_state_missing",
                json!({
                    "trigger": trigger,
                    "globalState": path.to_string_lossy().to_string()
                }),
            );
        }
        return Ok(0);
    }

    let original_content = fs::read_to_string(path)
        .map_err(|err| format!("读取 Codex global state 失败 {}: {err}", path.display()))?;
    let mut state = parse_global_state(&original_content, path)?;
    let next = normalized_global_state_workspace_roots(&state);
    let updated = global_state_update_count(&state, &next);
    if updated > 0 {
        for (key, value) in next {
            state.insert(key, value);
        }
        if let Some(parent) = path.parent() {
            fs::write(
                parent.join(format!("{GLOBAL_STATE_FILE_NAME}.bak")),
                &original_content,
            )
            .map_err(|err| {
                format!(
                    "备份 Codex global state 失败 {}: {err}",
                    parent
                        .join(format!("{GLOBAL_STATE_FILE_NAME}.bak"))
                        .display()
                )
            })?;
        }
        let mut output = serde_json::to_string_pretty(&Value::Object(state))
            .map_err(|err| format!("序列化 Codex global state 失败: {err}"))?;
        output.push('\n');
        write_existing_file(path, &output, "写入 Codex global state")?;
    }
    if let Some(trigger) = trigger {
        log_session_sync_event(
            "session_sync_global_state_summary",
            json!({
                "trigger": trigger,
                "globalState": path.to_string_lossy().to_string(),
                "updated": updated
            }),
        );
    }
    Ok(updated)
}

fn load_global_state(path: &Path) -> Result<Map<String, Value>, String> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|err| format!("读取 Codex global state 失败 {}: {err}", path.display()))?;
    parse_global_state(&content, path)
}

fn parse_global_state(content: &str, path: &Path) -> Result<Map<String, Value>, String> {
    let value: Value = serde_json::from_str(content)
        .map_err(|err| format!("解析 Codex global state 失败 {}: {err}", path.display()))?;
    Ok(value.as_object().cloned().unwrap_or_default())
}

fn normalized_global_state_workspace_roots(state: &Map<String, Value>) -> Map<String, Value> {
    let mut next = Map::new();
    if let Some(value) = state.get("electron-saved-workspace-roots") {
        next.insert(
            "electron-saved-workspace-roots".to_string(),
            json!(dedupe_paths(path_array(value))),
        );
    }
    if let Some(value) = state.get("project-order") {
        next.insert(
            "project-order".to_string(),
            json!(dedupe_paths(path_array(value))),
        );
    }
    if let Some(value) = state.get("active-workspace-roots") {
        let normalized = dedupe_paths(path_array(value));
        let next_value = if value.is_array() {
            json!(normalized)
        } else if let Some(first) = normalized.first() {
            json!(first)
        } else {
            value.clone()
        };
        next.insert("active-workspace-roots".to_string(), next_value);
    }
    if let Some(value) = state
        .get("electron-workspace-root-labels")
        .and_then(Value::as_object)
    {
        let mut labels = Map::new();
        for (key, item) in value {
            labels.insert(
                to_desktop_workspace_path(key).unwrap_or_else(|| key.clone()),
                item.clone(),
            );
        }
        next.insert(
            "electron-workspace-root-labels".to_string(),
            Value::Object(labels),
        );
    }
    if let Some(open_targets) = state
        .get("open-in-target-preferences")
        .and_then(Value::as_object)
    {
        let mut next_open_targets = open_targets.clone();
        if let Some(per_path) =
            copy_resolved_object_keys(open_targets.get("perPath").and_then(Value::as_object))
        {
            next_open_targets.insert("perPath".to_string(), Value::Object(per_path));
        }
        next.insert(
            "open-in-target-preferences".to_string(),
            Value::Object(next_open_targets),
        );
    }
    next
}

fn copy_resolved_object_keys(value: Option<&Map<String, Value>>) -> Option<Map<String, Value>> {
    let value = value?;
    let mut next = Map::new();
    for (key, item) in value {
        next.insert(
            to_desktop_workspace_path(key).unwrap_or_else(|| key.clone()),
            item.clone(),
        );
    }
    Some(next)
}

fn global_state_update_count(state: &Map<String, Value>, next: &Map<String, Value>) -> usize {
    next.iter()
        .filter(|(key, value)| state.get(*key) != Some(*value))
        .count()
}

fn path_array(value: &Value) -> Vec<String> {
    if let Some(items) = value.as_array() {
        items
            .iter()
            .filter_map(Value::as_str)
            .filter(|item| !item.trim().is_empty())
            .map(ToString::to_string)
            .collect()
    } else if let Some(value) = value.as_str().filter(|item| !item.trim().is_empty()) {
        vec![value.to_string()]
    } else {
        Vec::new()
    }
}

fn dedupe_paths(paths: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for path in paths {
        let normalized = to_desktop_workspace_path(&path).unwrap_or(path);
        if seen.insert(normalized.to_ascii_lowercase()) {
            result.push(normalized);
        }
    }
    result
}

#[cfg(test)]
mod tests;
