use super::support::provider_log_value;
use crate::{app_log::log_event, json_util::raw_string_field};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{self, BufRead, BufReader, BufWriter, ErrorKind, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process,
    sync::{
        atomic::{AtomicU64, Ordering},
        LazyLock, Mutex, MutexGuard,
    },
    time::SystemTime,
};

const HEADER_READ_BUFFER_BYTES: usize = 64 * 1024;
const STREAM_COPY_BUFFER_BYTES: usize = 256 * 1024;
const MAX_HEADER_SESSION_META_LINES: usize = 8;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Caches the last header-only provider preflight result per rollout while its size and
/// modification time remain unchanged.
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

/// The sync preserves a rollout's modification time (and on the in-place fast path also its exact
/// byte length), so drop what the cache knows about the file whenever we rewrite it.
fn forget_cached_rollout(path: &Path) {
    preflight_cache().provider_checks.remove(path);
}

#[derive(Debug)]
struct RolloutFileSyncOutcome {
    changed: bool,
    from_provider: Option<String>,
}

struct HeaderLine {
    start_offset: u64,
    raw_segment: Vec<u8>,
    is_session_meta: bool,
}

struct RolloutHeader {
    lines: Vec<HeaderLine>,
    header_end_offset: u64,
}

struct InPlacePatch {
    offset: u64,
    original: Vec<u8>,
    replacement: Vec<u8>,
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

fn rollout_header_session_meta_summary(header: &RolloutHeader) -> Value {
    for entry in &header.lines {
        if !entry.is_session_meta {
            continue;
        }
        let Ok(segment) = std::str::from_utf8(&entry.raw_segment) else {
            continue;
        };
        let (line, _) = split_line_ending(segment);
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

/// Reads only the leading contiguous `session_meta` lines (and any leading blank lines) at the
/// start of a rollout file, stopping as soon as the first non-`session_meta` line is encountered.
/// Codex writes `session_meta` only at the head of a rollout (line 1, or lines 1-2 for forked
/// sessions), so the multi-hundred-MB body never needs to be read into memory.
fn read_rollout_header(path: &Path) -> Result<Option<(FileStamp, RolloutHeader)>, String> {
    let read_error =
        |err: io::Error| format!("读取 Codex session 文件失败 {}: {err}", path.display());
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(read_error(err)),
    };
    let metadata = file
        .metadata()
        .map_err(|err| format!("读取 Codex session 修改时间失败 {}: {err}", path.display()))?;
    let stamp = FileStamp {
        len: metadata.len(),
        modified: metadata
            .modified()
            .map_err(|err| format!("读取 Codex session 修改时间失败 {}: {err}", path.display()))?,
    };
    let header = read_rollout_header_from_file(path, file)?;
    Ok(Some((stamp, header)))
}

fn read_rollout_header_from_file(path: &Path, file: fs::File) -> Result<RolloutHeader, String> {
    let read_error =
        |err: io::Error| format!("读取 Codex session 文件失败 {}: {err}", path.display());
    let mut reader = BufReader::with_capacity(HEADER_READ_BUFFER_BYTES, file);
    let mut offset = 0u64;
    let mut lines = Vec::new();
    let mut session_meta_count = 0usize;

    while session_meta_count < MAX_HEADER_SESSION_META_LINES {
        let mut segment = Vec::new();
        let read = reader.read_until(b'\n', &mut segment).map_err(read_error)?;
        if read == 0 {
            break;
        }
        let is_blank = segment.iter().all(|byte| byte.is_ascii_whitespace());
        if is_blank {
            lines.push(HeaderLine {
                start_offset: offset,
                raw_segment: segment,
                is_session_meta: false,
            });
            offset += read as u64;
            continue;
        }
        if !is_session_meta_segment(&segment) {
            break;
        }
        session_meta_count += 1;
        lines.push(HeaderLine {
            start_offset: offset,
            raw_segment: segment,
            is_session_meta: true,
        });
        offset += read as u64;
    }

    Ok(RolloutHeader {
        lines,
        header_end_offset: offset,
    })
}

fn is_session_meta_segment(segment: &[u8]) -> bool {
    if memchr::memmem::find(segment, b"\"session_meta\"").is_none() {
        return false;
    }
    let Ok(text) = std::str::from_utf8(segment) else {
        return false;
    };
    let (line, _) = split_line_ending(text);
    serde_json::from_str::<Value>(line)
        .ok()
        .is_some_and(|event| event.get("type").and_then(Value::as_str) == Some("session_meta"))
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
    let Some((stamp, header)) = read_rollout_header(path)? else {
        forget_cached_rollout(path);
        return Ok(gone());
    };

    let mut any_changed = false;
    let mut in_place_patches: Option<Vec<InPlacePatch>> = Some(Vec::new());
    let mut rewritten_header = Vec::new();

    for entry in &header.lines {
        if !entry.is_session_meta {
            rewritten_header.extend_from_slice(&entry.raw_segment);
            continue;
        }
        let segment = std::str::from_utf8(&entry.raw_segment).map_err(|err| {
            format!(
                "读取 Codex session 文件失败 {}: 不是有效的 UTF-8: {err}",
                path.display()
            )
        })?;
        let (line, line_ending) = split_line_ending(segment);
        match update_rollout_provider_line(line, target_provider)? {
            Some(updated_line) => {
                any_changed = true;
                if let Some(patches) = in_place_patches.as_mut() {
                    if let Some(patch) = try_in_place_provider_patch(
                        entry.start_offset,
                        line.as_bytes(),
                        &updated_line,
                        target_provider,
                    ) {
                        patches.push(patch);
                    } else {
                        in_place_patches = None;
                    }
                }
                rewritten_header.extend_from_slice(updated_line.as_bytes());
                rewritten_header.extend_from_slice(line_ending.as_bytes());
            }
            None => {
                rewritten_header.extend_from_slice(&entry.raw_segment);
            }
        }
    }

    if !any_changed {
        return Ok(RolloutFileSyncOutcome {
            changed: false,
            from_provider: None,
        });
    }

    let before_meta = rollout_header_session_meta_summary(&header);
    let from_provider = rollout_meta_provider(&before_meta);

    let wrote = if let Some(patches) = in_place_patches {
        apply_in_place_patches(path, stamp, &patches)?
    } else {
        rewrite_rollout_header_streaming(path, stamp, header.header_end_offset, &rewritten_header)?
    };

    forget_cached_rollout(path);
    Ok(RolloutFileSyncOutcome {
        changed: wrote,
        from_provider: wrote.then_some(from_provider),
    })
}

fn apply_in_place_patches(
    path: &Path,
    stamp: FileStamp,
    patches: &[InPlacePatch],
) -> Result<bool, String> {
    let mut file = match fs::OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(format!(
                "打开 Codex session 文件失败 {}: {err}",
                path.display()
            ))
        }
    };

    let metadata = file
        .metadata()
        .map_err(|err| format!("复核 Codex session 文件失败 {}: {err}", path.display()))?;
    ensure_metadata_unchanged(path, &metadata, stamp)?;

    let mut current = Vec::new();
    for patch in patches {
        current.resize(patch.original.len(), 0);
        file.seek(SeekFrom::Start(patch.offset))
            .and_then(|_| file.read_exact(&mut current))
            .map_err(|err| format!("复核 Codex session 文件失败 {}: {err}", path.display()))?;
        if current != patch.original {
            return Err(format!(
                "Codex session 文件在同步期间被写入（头部内容已变化），为避免覆盖新内容已跳过 {}",
                path.display()
            ));
        }
    }

    for patch in patches {
        file.seek(SeekFrom::Start(patch.offset))
            .and_then(|_| file.write_all(&patch.replacement))
            .map_err(|err| format!("写入 Codex session 文件失败 {}: {err}", path.display()))?;
    }
    file.flush()
        .map_err(|err| format!("刷新 Codex session 文件失败 {}: {err}", path.display()))?;
    file.set_modified(stamp.modified)
        .map_err(|err| format!("恢复 Codex session 修改时间失败 {}: {err}", path.display()))?;
    Ok(true)
}

fn rewrite_rollout_header_streaming(
    path: &Path,
    stamp: FileStamp,
    header_end_offset: u64,
    new_header_bytes: &[u8],
) -> Result<bool, String> {
    let temp = temp_rollout_path(path)?;
    let write_result = (|| -> Result<bool, String> {
        let mut source = match fs::File::open(path) {
            Ok(file) => file,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
            Err(err) => {
                return Err(format!(
                    "读取 Codex session 文件失败 {}: {err}",
                    path.display()
                ))
            }
        };
        let source_meta = source
            .metadata()
            .map_err(|err| format!("复核 Codex session 文件失败 {}: {err}", path.display()))?;
        ensure_metadata_unchanged(path, &source_meta, stamp)?;
        source
            .seek(SeekFrom::Start(header_end_offset))
            .map_err(|err| format!("定位 Codex session 正文失败 {}: {err}", path.display()))?;

        let temp_file = fs::File::create_new(&temp)
            .map_err(|err| format!("创建 Codex session 临时文件失败 {}: {err}", temp.display()))?;
        let mut reader = BufReader::with_capacity(STREAM_COPY_BUFFER_BYTES, source);
        let mut writer = BufWriter::with_capacity(STREAM_COPY_BUFFER_BYTES, temp_file);
        writer
            .write_all(new_header_bytes)
            .and_then(|()| io::copy(&mut reader, &mut writer).map(|_| ()))
            .and_then(|()| writer.flush())
            .map_err(|err| format!("写入 Codex session 文件失败 {}: {err}", path.display()))?;
        let temp_file = writer
            .into_inner()
            .map_err(|err| format!("刷新 Codex session 文件失败 {}: {err}", path.display()))?;
        temp_file
            .sync_all()
            .map_err(|err| format!("同步 Codex session 临时文件失败 {}: {err}", temp.display()))?;
        temp_file
            .set_modified(stamp.modified)
            .map_err(|err| format!("恢复 Codex session 修改时间失败 {}: {err}", path.display()))?;
        // Release both handles before replacing the target file (required on Windows).
        drop(temp_file);
        drop(reader);

        // Re-check right before replace so concurrent appends are never dropped and a rollout
        // moved away in the meantime is never recreated.
        match verify_rollout_present_and_unchanged(path, stamp)? {
            true => {}
            false => return Ok(false),
        }

        fs::rename(&temp, path)
            .map_err(|err| format!("替换 Codex session 文件失败 {}: {err}", path.display()))?;
        Ok(true)
    })();

    if !matches!(write_result, Ok(true)) {
        let _ = fs::remove_file(&temp);
    }
    write_result
}

fn temp_rollout_path(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} 没有上级目录", path.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("{} 不是文件路径", path.display()))?
        .to_string_lossy();
    let sequence = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(
        ".{file_name}.{}.{sequence}.sync-tmp",
        process::id()
    )))
}

fn ensure_metadata_unchanged(
    path: &Path,
    metadata: &fs::Metadata,
    stamp: FileStamp,
) -> Result<(), String> {
    let current_len = metadata.len();
    let current_modified = metadata.modified().ok();
    if current_len != stamp.len || current_modified != Some(stamp.modified) {
        return Err(format!(
            "Codex session 文件在同步期间被写入（读取时 {} 字节，写入前 {current_len} 字节），为避免丢失新内容已跳过 {}",
            stamp.len,
            path.display()
        ));
    }
    Ok(())
}

fn verify_rollout_present_and_unchanged(path: &Path, stamp: FileStamp) -> Result<bool, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(format!(
                "复核 Codex session 文件失败 {}: {err}",
                path.display()
            ))
        }
    };
    ensure_metadata_unchanged(path, &metadata, stamp)?;
    Ok(true)
}

#[cfg(test)]
fn ensure_rollout_unchanged_since_read(
    path: &Path,
    read_len: usize,
    read_modified: SystemTime,
) -> Result<(), String> {
    verify_rollout_present_and_unchanged(
        path,
        FileStamp {
            len: read_len as u64,
            modified: read_modified,
        },
    )
    .map(|_| ())
}

/// Attempts to build a byte-length-preserving patch for `payload.model_provider` inside a single
/// `session_meta` JSONL line. Padding with ASCII spaces after `:` keeps `"model_provider":"openai"`
/// and `"model_provider":   "api"` at identical byte length so the multi-hundred-MB body and all
/// byte offsets remain completely untouched.
fn try_in_place_provider_patch(
    line_start_offset: u64,
    raw_line: &[u8],
    expected_line: &str,
    target_provider: &str,
) -> Option<InPlacePatch> {
    let (slot_start, slot_end) = find_payload_model_provider_slot(raw_line)?;
    let encoded_target = serde_json::to_string(target_provider).ok()?;
    let slot_len = slot_end.checked_sub(slot_start)?;
    if encoded_target.len() > slot_len {
        return None;
    }

    let pad = slot_len - encoded_target.len();
    let mut replacement = Vec::with_capacity(slot_len);
    replacement.resize(pad, b' ');
    replacement.extend_from_slice(encoded_target.as_bytes());

    // Self-verify against the full serde_json rewrite before touching disk; this automatically
    // rejects lines with multiple `model_provider` fields or any unusual JSON edge case.
    let mut candidate = raw_line.to_vec();
    candidate[slot_start..slot_end].copy_from_slice(&replacement);
    let candidate_value: Value = serde_json::from_slice(&candidate).ok()?;
    let expected_value: Value = serde_json::from_str(expected_line).ok()?;
    if candidate_value != expected_value {
        return None;
    }

    Some(InPlacePatch {
        offset: line_start_offset + slot_start as u64,
        original: raw_line[slot_start..slot_end].to_vec(),
        replacement,
    })
}

/// Locates the `(slot_start, slot_end)` byte range of `payload.model_provider`'s JSON string value
/// (including any surrounding ASCII spaces/tabs between `:` and `,`/`}`) in a single JSON object.
fn find_payload_model_provider_slot(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut pos = skip_json_ws(bytes, 0);
    if bytes.get(pos) != Some(&b'{') {
        return None;
    }
    pos += 1;

    loop {
        pos = skip_json_ws(bytes, pos);
        match bytes.get(pos)? {
            b'}' => return None,
            b'"' => {}
            _ => return None,
        }
        let key_start = pos;
        let key_end = scan_json_string(bytes, pos)?;
        pos = skip_json_ws(bytes, key_end);
        if bytes.get(pos) != Some(&b':') {
            return None;
        }
        pos = skip_json_ws(bytes, pos + 1);

        if &bytes[key_start + 1..key_end - 1] == b"payload" {
            return find_model_provider_slot_in_object(bytes, pos);
        }

        pos = skip_json_value(bytes, pos)?;
        pos = skip_json_ws(bytes, pos);
        match bytes.get(pos)? {
            b',' => pos += 1,
            b'}' => return None,
            _ => return None,
        }
    }
}

fn find_model_provider_slot_in_object(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    if bytes.get(start) != Some(&b'{') {
        return None;
    }
    let mut pos = start + 1;

    loop {
        pos = skip_json_ws(bytes, pos);
        match bytes.get(pos)? {
            b'}' => return None,
            b'"' => {}
            _ => return None,
        }
        let key_start = pos;
        let key_end = scan_json_string(bytes, pos)?;
        pos = skip_json_ws(bytes, key_end);
        if bytes.get(pos) != Some(&b':') {
            return None;
        }
        let slot_start = pos + 1;
        pos = skip_json_ws(bytes, slot_start);

        if &bytes[key_start + 1..key_end - 1] == b"model_provider" {
            if bytes.get(pos) != Some(&b'"') {
                return None;
            }
            let val_end = scan_json_string(bytes, pos)?;
            let slot_end = skip_json_ws(bytes, val_end);
            if !matches!(bytes.get(slot_end), Some(b',' | b'}')) {
                return None;
            }
            return Some((slot_start, slot_end));
        }

        pos = skip_json_value(bytes, pos)?;
        pos = skip_json_ws(bytes, pos);
        match bytes.get(pos)? {
            b',' => pos += 1,
            b'}' => return None,
            _ => return None,
        }
    }
}

fn skip_json_ws(bytes: &[u8], mut pos: usize) -> usize {
    while let Some(&byte) = bytes.get(pos) {
        if matches!(byte, b' ' | b'\t' | b'\r' | b'\n') {
            pos += 1;
        } else {
            break;
        }
    }
    pos
}

fn scan_json_string(bytes: &[u8], start: usize) -> Option<usize> {
    if bytes.get(start) != Some(&b'"') {
        return None;
    }
    let mut pos = start + 1;
    while pos < bytes.len() {
        match bytes[pos] {
            b'\\' => {
                pos += 2;
                if pos > bytes.len() {
                    return None;
                }
            }
            b'"' => return Some(pos + 1),
            _ => pos += 1,
        }
    }
    None
}

fn skip_json_value(bytes: &[u8], start: usize) -> Option<usize> {
    let mut pos = skip_json_ws(bytes, start);
    match *bytes.get(pos)? {
        b'"' => scan_json_string(bytes, pos),
        b'{' | b'[' => {
            let mut depth = 0usize;
            while pos < bytes.len() {
                match bytes[pos] {
                    b'"' => pos = scan_json_string(bytes, pos)?,
                    b'{' | b'[' => {
                        depth += 1;
                        pos += 1;
                    }
                    b'}' | b']' => {
                        depth = depth.checked_sub(1)?;
                        pos += 1;
                        if depth == 0 {
                            return Some(pos);
                        }
                    }
                    _ => pos += 1,
                }
            }
            None
        }
        _ => {
            let begin = pos;
            while let Some(&byte) = bytes.get(pos) {
                if matches!(byte, b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n') {
                    break;
                }
                pos += 1;
            }
            (pos > begin).then_some(pos)
        }
    }
}

fn rollout_file_provider_would_change(path: &Path, target_provider: &str) -> Result<bool, String> {
    let read_error =
        |err: io::Error| format!("读取 Codex session 文件失败 {}: {err}", path.display());
    // A rollout Codex moved after it was listed (archiving the thread) has nothing to sync here;
    // the next listing checks it at its new path.
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            forget_cached_rollout(path);
            return Ok(false);
        }
        Err(err) => return Err(read_error(err)),
    };
    let stamp = FileStamp {
        len: metadata.len(),
        modified: metadata.modified().map_err(read_error)?,
    };
    if let Some(cached) = preflight_cache()
        .provider_checks
        .get(path)
        .filter(|check| check.target_provider == target_provider && check.stamp == stamp)
    {
        return Ok(cached.would_change);
    }

    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            forget_cached_rollout(path);
            return Ok(false);
        }
        Err(err) => return Err(read_error(err)),
    };
    let header = read_rollout_header_from_file(path, file)?;
    let mut would_change = false;
    for entry in &header.lines {
        if !entry.is_session_meta {
            continue;
        }
        let segment = std::str::from_utf8(&entry.raw_segment).map_err(|err| {
            format!(
                "读取 Codex session 文件失败 {}: 不是有效的 UTF-8: {err}",
                path.display()
            )
        })?;
        let (line, _) = split_line_ending(segment);
        if update_rollout_provider_line(line, target_provider)?.is_some() {
            would_change = true;
            break;
        }
    }

    preflight_cache().provider_checks.insert(
        path.to_path_buf(),
        ProviderCheck {
            target_provider: target_provider.to_string(),
            stamp,
            would_change,
        },
    );
    Ok(would_change)
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
    // the other lines skip the JSON parse.
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
mod patch_and_guard_tests {
    use super::{
        ensure_rollout_unchanged_since_read, preview_rollout_files_to_provider_with_diagnostics,
        rollout_file_provider_would_change, sync_rollout_file_provider,
    };
    use serde_json::{json, Value};
    use std::{
        env, fs,
        io::Write,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    fn unique_path(label: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!("codex-switch-rollout-patch-{label}-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        dir.join("rollout-test.jsonl")
    }

    #[test]
    fn rollout_appended_after_the_read_is_not_rewritten() {
        let path = unique_path("append-guard");
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
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_rollout_moved_away_after_listing_is_skipped_not_an_error() {
        let path = unique_path("moved-rollout");

        assert_eq!(rollout_file_provider_would_change(&path, "api"), Ok(false));
        assert_eq!(
            preview_rollout_files_to_provider_with_diagnostics(vec![path.clone()], "api", None),
            Ok(0)
        );
        let outcome = sync_rollout_file_provider(&path, "api").unwrap();
        assert!(!outcome.changed);
        assert!(!path.exists());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn in_place_patch_round_trip_keeps_exact_size_mtime_and_untouched_body() {
        let path = unique_path("in-place-round-trip");
        let meta = json!({
            "timestamp": "2026-11-27T00:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": "thread-1",
                "base_instructions": "mentions \"model_provider\":\"openai\" in text",
                "model_provider": "openai",
                "cwd": "E:\\Project\\ai"
            }
        });
        // Body intentionally contains non-UTF-8 bytes to prove in-place patch never reads or
        // parses the rollout body.
        let body = b"{\"type\":\"event_msg\",\"payload\":{\"raw\":\"\xff\xfe\"}}\n";
        let mut raw = format!("{meta}\n").into_bytes();
        let body_offset = raw.len();
        raw.extend_from_slice(body);
        fs::write(&path, &raw).unwrap();

        let original_mtime = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(original_mtime)
            .unwrap();

        let to_api = sync_rollout_file_provider(&path, "api").unwrap();
        assert!(to_api.changed);
        assert_eq!(to_api.from_provider.as_deref(), Some("openai"));

        let after_api_bytes = fs::read(&path).unwrap();
        assert_eq!(after_api_bytes.len(), raw.len());
        assert_eq!(&after_api_bytes[body_offset..], body);
        let after_api_meta: Value =
            serde_json::from_slice(&after_api_bytes[..body_offset - 1]).unwrap();
        assert_eq!(after_api_meta["payload"]["model_provider"], "api");
        assert_eq!(
            after_api_meta["payload"]["base_instructions"],
            "mentions \"model_provider\":\"openai\" in text"
        );
        let api_mtime = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(
            api_mtime
                .duration_since(original_mtime)
                .or_else(|_| original_mtime.duration_since(api_mtime))
                .unwrap()
                < Duration::from_secs(1)
        );
        assert_eq!(rollout_file_provider_would_change(&path, "api"), Ok(false));
        assert_eq!(
            rollout_file_provider_would_change(&path, "openai"),
            Ok(true)
        );

        // Switching back to "openai" reuses the exact 8-byte padded slot in place.
        let back_to_openai = sync_rollout_file_provider(&path, "openai").unwrap();
        assert!(back_to_openai.changed);
        assert_eq!(back_to_openai.from_provider.as_deref(), Some("api"));
        let back_bytes = fs::read(&path).unwrap();
        assert_eq!(back_bytes, raw);
        assert_eq!(
            rollout_file_provider_would_change(&path, "openai"),
            Ok(false)
        );

        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn forked_two_line_session_meta_patches_both_lines_in_place() {
        let path = unique_path("fork-two-lines");
        let line1 = json!({
            "type": "session_meta",
            "payload": {"id": "child", "forked_from_id": "parent", "model_provider": "openai"}
        });
        let line2 = json!({
            "type": "session_meta",
            "payload": {"id": "parent", "model_provider": "openai"}
        });
        let body = b"{\"type\":\"event_msg\",\"payload\":{}}\n";
        let mut raw = format!("{line1}\n{line2}\n").into_bytes();
        let body_offset = raw.len();
        raw.extend_from_slice(body);
        fs::write(&path, &raw).unwrap();

        let outcome = sync_rollout_file_provider(&path, "api").unwrap();
        assert!(outcome.changed);
        let after = fs::read(&path).unwrap();
        assert_eq!(after.len(), raw.len());
        assert_eq!(&after[body_offset..], body);

        let lines: Vec<&str> = std::str::from_utf8(&after[..body_offset])
            .unwrap()
            .lines()
            .collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            let parsed: Value = serde_json::from_str(line).unwrap();
            assert_eq!(parsed["payload"]["model_provider"], "api");
        }

        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn streaming_fallback_preserves_raw_body_mtime_and_subsequent_in_place_round_trip() {
        let path = unique_path("streaming-fallback");
        let line1 = json!({
            "type": "session_meta",
            "payload": {"id": "api-origin", "model_provider": "api"}
        });
        let body = b"{\"type\":\"event_msg\",\"payload\":\"\xff\xfe\"}\r\n";
        let mut raw = format!("{line1}\r\n").into_bytes();
        raw.extend_from_slice(body);
        fs::write(&path, &raw).unwrap();

        let original_mtime = UNIX_EPOCH + Duration::from_secs(1_700_000_500);
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(original_mtime)
            .unwrap();

        // "api" (5 bytes) -> "openai" (8 bytes) uses streaming rewrite once.
        let to_openai = sync_rollout_file_provider(&path, "openai").unwrap();
        assert!(to_openai.changed);
        assert_eq!(to_openai.from_provider.as_deref(), Some("api"));

        let after_openai = fs::read(&path).unwrap();
        assert!(after_openai.ends_with(body));
        let first_eol = after_openai.windows(2).position(|w| w == b"\r\n").unwrap();
        let meta: Value = serde_json::from_slice(&after_openai[..first_eol]).unwrap();
        assert_eq!(meta["payload"]["model_provider"], "openai");

        let entries: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["rollout-test.jsonl"]);

        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(
            mtime
                .duration_since(original_mtime)
                .or_else(|_| original_mtime.duration_since(mtime))
                .unwrap()
                < Duration::from_secs(1)
        );

        // Now that the slot holds "openai" (8 bytes), switching back to "api" is fixed-width in-place.
        let back_to_api = sync_rollout_file_provider(&path, "api").unwrap();
        assert!(back_to_api.changed);
        assert_eq!(fs::read(&path).unwrap().len(), after_openai.len());

        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
