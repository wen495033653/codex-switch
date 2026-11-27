use super::support::{provider_log_value, write_existing_file};
use crate::{app_log::log_event, json_util::raw_string_field};
use memchr::memmem::Finder;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex, MutexGuard},
    time::SystemTime,
};

const PREFLIGHT_RESUME_CHECK_BYTES: u64 = 64;

/// What earlier preflights learned about each rollout file. The watcher runs a preflight every
/// minute; without this it re-read every rollout each time. A provider check is reused while the
/// file keeps its size and modification time, and extended over the appended lines while the bytes
/// before its offset still match.
#[derive(Default)]
struct RolloutPreflightCache {
    provider_checks: HashMap<PathBuf, ProviderCheck>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: SystemTime,
}

#[derive(Clone)]
struct ProviderCheck {
    target_provider: String,
    stamp: FileStamp,
    /// Complete lines before this offset were checked.
    offset: u64,
    resume_check: Vec<u8>,
    would_change: bool,
}

static ROLLOUT_PREFLIGHT_CACHE: LazyLock<Mutex<RolloutPreflightCache>> =
    LazyLock::new(Default::default);

// Release builds abort on panic, so the lock can never be left poisoned there.
fn preflight_cache() -> MutexGuard<'static, RolloutPreflightCache> {
    ROLLOUT_PREFLIGHT_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The sync rewrites a file in place and then restores its modification time, so a same-length
/// rewrite would look unchanged. It drops what the cache knows about the file instead.
fn forget_cached_rollout(path: &Path) {
    preflight_cache().provider_checks.remove(path);
}

#[derive(Debug)]
struct RolloutFileSyncOutcome {
    changed: bool,
    from_provider: Option<String>,
}

#[cfg(test)]
pub(super) fn sync_codex_session_rollouts_to_provider(
    sessions_dir: &Path,
    target_provider: &str,
) -> Result<usize, String> {
    let rollout_files = collect_rollout_files_from_dirs(&[sessions_dir.to_path_buf()], &[])?;
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
    let rollout_files = collect_rollout_files_from_dirs(rollout_dirs, extra_rollout_paths)?;
    if let Some(trigger) = trigger {
        log_event(
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

pub(super) fn preview_codex_session_rollout_dirs_to_provider_with_diagnostics(
    rollout_dirs: &[PathBuf],
    target_provider: &str,
    extra_rollout_paths: &[PathBuf],
    trigger: Option<&str>,
) -> Result<usize, String> {
    let rollout_files = collect_rollout_files_from_dirs(rollout_dirs, extra_rollout_paths)?;
    if let Some(trigger) = trigger {
        log_event(
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
                    log_event(
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
            log_event(
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
        match sync_rollout_file_provider(&path, target_provider) {
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
                    log_event(
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
            log_event(
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

fn sync_rollout_file_provider(
    path: &Path,
    target_provider: &str,
) -> Result<RolloutFileSyncOutcome, String> {
    // Codex moves a rollout when it archives the thread; one listed a moment ago may be gone.
    // It is synced at its new path, never recreated here.
    let gone = || RolloutFileSyncOutcome {
        changed: false,
        from_provider: None,
    };
    let original_modified = match fs::metadata(path).and_then(|metadata| metadata.modified()) {
        Ok(modified) => modified,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(gone()),
        Err(err) => {
            return Err(format!(
                "读取 Codex session 修改时间失败 {}: {err}",
                path.display()
            ))
        }
    };
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(gone()),
        Err(err) => {
            return Err(format!(
                "读取 Codex session 文件失败 {}: {err}",
                path.display()
            ))
        }
    };
    let mut updated_content: Option<String> = None;
    let mut processed_bytes = 0usize;

    for segment in content.split_inclusive('\n') {
        let (line, line_ending) = split_line_ending(segment);
        match update_rollout_provider_line(line, target_provider)? {
            Some(updated_line) => {
                let buf = updated_content.get_or_insert_with(|| {
                    let mut s = String::with_capacity(content.len());
                    s.push_str(&content[..processed_bytes]);
                    s
                });
                buf.push_str(&updated_line);
                buf.push_str(line_ending);
            }
            None => {
                if let Some(buf) = updated_content.as_mut() {
                    buf.push_str(segment);
                }
            }
        }
        processed_bytes += segment.len();
    }

    let mut changed = false;
    if let Some(updated_content) = updated_content {
        let before_meta = rollout_content_session_meta_summary(&content);
        let from_provider = rollout_meta_provider(&before_meta);
        // The rewrite truncates the file in place. When Codex is still running (a close that
        // failed, or reopening editors) it may have appended since the read; writing now would
        // drop those lines, so the file is left alone and reported instead. This narrows the
        // window to the write itself; it does not close it.
        ensure_rollout_unchanged_since_read(path, content.len(), original_modified)?;
        let wrote = write_existing_file(path, &updated_content, "写入 Codex session 文件")?;
        forget_cached_rollout(path);
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
        return Ok(RolloutFileSyncOutcome {
            changed,
            from_provider: changed.then_some(from_provider),
        });
    }

    Ok(RolloutFileSyncOutcome {
        changed,
        from_provider: None,
    })
}

fn ensure_rollout_unchanged_since_read(
    path: &Path,
    read_len: usize,
    read_modified: std::time::SystemTime,
) -> Result<(), String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        // A file removed since the read is handled by write_existing_file (never recreated).
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(format!(
                "复核 Codex session 文件失败 {}: {err}",
                path.display()
            ))
        }
    };
    let current_len = metadata.len();
    let current_modified = metadata.modified().ok();
    if current_len != read_len as u64 || current_modified != Some(read_modified) {
        return Err(format!(
            "Codex session 文件在同步期间被写入（读取时 {read_len} 字节，写入前 {current_len} 字节），为避免丢失新内容已跳过 {}",
            path.display()
        ));
    }
    Ok(())
}

fn rollout_file_provider_would_change(path: &Path, target_provider: &str) -> Result<bool, String> {
    let read_error =
        |err: std::io::Error| format!("读取 Codex session 文件失败 {}: {err}", path.display());
    // A rollout Codex moved after it was listed (archiving the thread) has nothing to sync here;
    // the next listing checks it at its new path.
    let opened = fs::metadata(path).and_then(|metadata| Ok((metadata, fs::File::open(path)?)));
    let (metadata, mut file) = match opened {
        Ok(opened) => opened,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            forget_cached_rollout(path);
            return Ok(false);
        }
        Err(err) => return Err(read_error(err)),
    };
    let stamp = FileStamp {
        len: metadata.len(),
        modified: metadata.modified().map_err(read_error)?,
    };
    let previous = preflight_cache()
        .provider_checks
        .get(path)
        .filter(|check| check.target_provider == target_provider)
        .cloned();
    if let Some(previous) = previous.as_ref() {
        if previous.stamp == stamp {
            return Ok(previous.would_change);
        }
    }

    let resumed = match previous {
        Some(previous)
            if resumes_at(&mut file, previous.offset, &previous.resume_check)
                .map_err(read_error)? =>
        {
            Some(previous)
        }
        _ => None,
    };
    // Each line decides on its own, so a file that already needs a change still does after more
    // lines were appended.
    let (offset, would_change) = match resumed {
        Some(previous) if previous.would_change => (previous.offset, true),
        Some(previous) => check_provider_lines(path, &mut file, previous.offset, target_provider)?,
        None => check_provider_lines(path, &mut file, 0, target_provider)?,
    };
    let check_len = offset.min(PREFLIGHT_RESUME_CHECK_BYTES);
    let mut resume_check = vec![0; check_len as usize];
    file.seek(SeekFrom::Start(offset - check_len))
        .and_then(|_| file.read_exact(&mut resume_check))
        .map_err(read_error)?;
    preflight_cache().provider_checks.insert(
        path.to_path_buf(),
        ProviderCheck {
            target_provider: target_provider.to_string(),
            stamp,
            offset,
            resume_check,
            would_change,
        },
    );
    Ok(would_change)
}

fn resumes_at(file: &mut fs::File, offset: u64, resume_check: &[u8]) -> std::io::Result<bool> {
    let check_len = resume_check.len() as u64;
    if offset < check_len || file.metadata()?.len() < offset {
        return Ok(false);
    }
    let mut current = vec![0; resume_check.len()];
    file.seek(SeekFrom::Start(offset - check_len))?;
    file.read_exact(&mut current)?;
    Ok(current == resume_check)
}

/// Checks the complete lines from `offset` on and stops at the first one that would change.
/// Returns the offset after the last line checked. A trailing line without a newline is still
/// being written and is checked once it is complete.
fn check_provider_lines(
    path: &Path,
    file: &mut fs::File,
    offset: u64,
    target_provider: &str,
) -> Result<(u64, bool), String> {
    let read_error =
        |err: std::io::Error| format!("读取 Codex session 文件失败 {}: {err}", path.display());
    file.seek(SeekFrom::Start(offset)).map_err(read_error)?;
    let mut reader = BufReader::new(&mut *file);
    let mut offset = offset;
    let mut segment = Vec::new();
    let provider_finder = Finder::new(b"\"model_provider\"");
    let meta_finder = Finder::new(b"\"session_meta\"");
    loop {
        segment.clear();
        let read = reader.read_until(b'\n', &mut segment).map_err(read_error)?;
        if read == 0 || segment.last() != Some(&b'\n') {
            return Ok((offset, false));
        }
        offset += read as u64;
        if provider_finder.find(&segment).is_none() && meta_finder.find(&segment).is_none() {
            continue;
        }
        let segment = std::str::from_utf8(&segment).map_err(|err| {
            format!(
                "读取 Codex session 文件失败 {}: 不是有效的 UTF-8: {err}",
                path.display()
            )
        })?;
        let (line, _line_ending) = split_line_ending(segment);
        if update_rollout_provider_line(line, target_provider)?.is_some() {
            return Ok((offset, true));
        }
    }
}

fn collect_rollout_files_from_dirs(
    dirs: &[PathBuf],
    extra_rollout_paths: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for dir in dirs {
        if dir.exists() {
            collect_rollout_file_paths(dir, &mut files)?;
        }
    }
    forget_missing_rollouts(dirs, &files);
    files.sort();

    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    for path in files {
        if seen.insert(path.clone()) {
            selected.push(path);
        }
    }

    for path in extra_rollout_paths {
        if path.is_file() && is_rollout_jsonl(path) && seen.insert(path.clone()) {
            selected.push(path.clone());
        }
    }

    Ok(selected)
}

fn collect_rollout_file_paths(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|err| format!("读取 Codex sessions 目录失败 {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("读取 Codex session 目录条目失败: {err}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("读取 Codex session 文件类型失败 {}: {err}", path.display()))?;
        if file_type.is_dir() {
            collect_rollout_file_paths(&path, files)?;
        } else if file_type.is_file() && is_rollout_jsonl(&path) {
            files.push(path);
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

/// Drops cache entries of files under `dirs` that the walk no longer found (deleted, moved to
/// the archive, or moved to the trash).
fn forget_missing_rollouts(dirs: &[PathBuf], found: &[PathBuf]) {
    let found = found.iter().map(PathBuf::as_path).collect::<HashSet<_>>();
    let stale = |path: &PathBuf| {
        !found.contains(path.as_path()) && dirs.iter().any(|dir| path.starts_with(dir))
    };
    preflight_cache()
        .provider_checks
        .retain(|path, _| !stale(path));
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

#[cfg(test)]
mod append_guard_tests {
    use super::{
        ensure_rollout_unchanged_since_read, preview_rollout_files_to_provider_with_diagnostics,
        rollout_file_provider_would_change, sync_rollout_file_provider,
    };
    use std::{
        env, fs,
        io::Write,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn rollout_appended_after_the_read_is_not_rewritten() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("codex-switch-append-guard-{stamp}.jsonl"));
        fs::write(&path, "{\"type\":\"session_meta\"}\n").unwrap();
        let metadata = fs::metadata(&path).unwrap();
        let read_len = metadata.len() as usize;
        let read_modified = metadata.modified().unwrap();

        ensure_rollout_unchanged_since_read(&path, read_len, read_modified).unwrap();

        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"type\":\"event_msg\"}\n")
            .unwrap();
        let err = ensure_rollout_unchanged_since_read(&path, read_len, read_modified).unwrap_err();
        assert!(err.contains("同步期间被写入"), "{err}");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_rollout_moved_away_after_listing_is_skipped_not_an_error() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("codex-switch-moved-rollout-{stamp}.jsonl"));

        assert_eq!(rollout_file_provider_would_change(&path, "api"), Ok(false));
        assert_eq!(
            preview_rollout_files_to_provider_with_diagnostics(vec![path.clone()], "api", None),
            Ok(0)
        );
        let outcome = sync_rollout_file_provider(&path, "api").unwrap();
        assert!(!outcome.changed);
        assert!(!path.exists());
    }
}
