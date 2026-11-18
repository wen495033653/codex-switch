use super::{
    catalog::{conversation_from_path, read_current_state_conversations},
    codex_home::{
        conversation_path_key, ensure_session_relative_path, is_compressed_rollout,
        normalize_relative_path, normalize_status, read_session_index, resolve_codex_root,
        resolve_rollout_file, status_from_relative_path, validate_codex_root,
        validate_session_file_path,
    },
    model::ManifestSession,
    rollout::{open_rollout, parse_session_file_for_list},
    state_db::{backup_state_database_file, thread_metadata_from_manifest, upsert_state_threads},
    util::{backup_stamp, sha256_bytes, sha256_reader},
    zip::{
        validate_zip_store_layout, write_zip_store, ZipArchiveLite, ZIP_V1_LIMIT_ERROR,
        ZIP_V1_MAX_BYTES,
    },
};
use crate::{
    app_log::log_event, codex_sessions::lock_codex_session_io, paths::codex_state_db_path_for_root,
    time_util::now_string,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, ErrorKind, Read, Write},
    path::{Path, PathBuf},
};
use tauri::AppHandle;
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};

const MANIFEST_FORMAT: &str = "codex-context-manager";

const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExportManifest {
    format: String,
    version: u32,
    exported_at: String,
    source_os: String,
    sessions: Vec<ManifestSession>,
}

impl ExportManifest {
    fn new(exported_at: String, sessions: Vec<ManifestSession>) -> Self {
        Self {
            format: MANIFEST_FORMAT.to_string(),
            version: MANIFEST_VERSION,
            exported_at,
            source_os: std::env::consts::OS.to_string(),
            sessions,
        }
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec_pretty(self).map_err(|err| format!("生成 manifest.json 失败: {err}"))
    }
}

#[derive(Debug, Clone)]
pub(super) struct ImportCandidate {
    pub(super) manifest: ManifestSession,
    pub(super) data: Vec<u8>,
    pub(super) target_path: PathBuf,
    pub(super) action: ImportAction,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum ImportAction {
    Import,
    SkipSame,
    Conflict,
    Error,
}

pub(super) fn export_conversations_impl(
    app: AppHandle,
    root: String,
    relative_paths: Vec<String>,
) -> Result<Value, String> {
    let root = resolve_codex_root(Some(&root))?;
    validate_codex_root(&root)?;
    if relative_paths.is_empty() {
        return Err("请先选择要导出的会话".to_string());
    }

    let default_name = format!("codex_contexts_{}.codexctx.zip", backup_stamp());
    let selected = app
        .dialog()
        .file()
        .set_title("导出 Codex 会话")
        .set_file_name(default_name)
        .add_filter("Codex Context", &["codexctx.zip", "zip"])
        .blocking_save_file()
        .ok_or_else(|| "导出已取消".to_string())?;
    let export_path = selected
        .into_path()
        .map_err(|err| format!("导出文件路径无效: {err}"))?;

    let bundle = collect_export_bundle(&root, relative_paths)?;
    if bundle.sessions.is_empty() {
        return Err(format!(
            "没有可导出的会话{}",
            if bundle.errors.is_empty() {
                String::new()
            } else {
                format!("：{}", bundle.errors.join("；"))
            }
        ));
    }

    let manifest = ExportManifest::new(bundle.exported_at, bundle.sessions);
    let manifest_bytes = manifest.bytes()?;
    let mut zip_entries = vec![("manifest.json".to_string(), manifest_bytes)];
    zip_entries.extend(bundle.entries);
    write_zip_store(&export_path, &zip_entries)?;

    Ok(json!({
        "ok": true,
        "message": format!("导出完成（{} 个会话）", manifest.sessions.len()),
        "report": {
            "path": export_path.to_string_lossy().to_string(),
            "exported": manifest.sessions.len(),
            "total_size": bundle.total_size,
            "failed": bundle.errors.len(),
            "errors": bundle.errors,
            "warnings": bundle.warnings
        }
    }))
}

pub(super) struct ExportBundle {
    exported_at: String,
    pub(super) sessions: Vec<ManifestSession>,
    pub(super) entries: Vec<(String, Vec<u8>)>,
    pub(super) errors: Vec<String>,
    pub(super) warnings: Vec<String>,
    pub(super) total_size: u64,
}

struct ExportFile {
    path: PathBuf,
    session: ManifestSession,
}

/// One JSONL snapshot provides the ZIP entry, its SHA-256 and its size. Compressed rollouts
/// are decoded into that snapshot, never rewritten on disk.
pub(super) fn collect_export_bundle(
    root: &Path,
    relative_paths: Vec<String>,
) -> Result<ExportBundle, String> {
    let _io_guard = lock_codex_session_io("导出会话")?;
    let mut warnings = Vec::new();
    let session_index = read_session_index(root, &mut warnings);
    let catalog = read_current_state_conversations(root, &session_index)?;
    warnings.extend(catalog.warnings);
    let state_by_path = catalog
        .conversations
        .into_iter()
        .map(|item| (conversation_path_key(&root.join(&item.relative_path)), item))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut bundle = ExportBundle {
        exported_at: now_string(),
        sessions: Vec::new(),
        entries: Vec::new(),
        errors: Vec::new(),
        warnings,
        total_size: 0,
    };
    let mut selected = Vec::new();

    for relative_path in relative_paths {
        if !seen.insert(relative_path.clone()) {
            continue;
        }
        let relative = match normalize_relative_path(&relative_path).and_then(|relative| {
            ensure_session_relative_path(&relative)?;
            Ok(relative)
        }) {
            Ok(relative) => relative,
            Err(err) => {
                bundle.errors.push(err);
                continue;
            }
        };
        let logical_path = root.join(&relative);
        let (path, metadata) = match resolve_rollout_file(&logical_path) {
            Ok(file) => file,
            Err(err) => {
                bundle.errors.push(format!(
                    "读取会话文件失败 {}: {err}",
                    logical_path.display()
                ));
                continue;
            }
        };
        if let Err(err) = validate_session_file_path(root, &path) {
            bundle.errors.push(err);
            continue;
        }
        let lower_bound = match export_file_lower_bound(&path, metadata.len()) {
            Ok(len) => len,
            Err(err) => {
                bundle
                    .errors
                    .push(format!("读取会话大小失败 {}: {err}", path.display()));
                continue;
            }
        };
        let status = match status_from_relative_path(&relative) {
            Ok(status) => status,
            Err(err) => {
                bundle.errors.push(err);
                continue;
            }
        };
        let item = match state_by_path.get(&conversation_path_key(&logical_path)) {
            Some(item) => item.clone(),
            None => match conversation_from_path(root, &path, &status, &session_index) {
                Ok(item) => item,
                Err(err) => {
                    bundle.errors.push(err);
                    continue;
                }
            },
        };
        selected.push(ExportFile {
            path,
            session: ManifestSession {
                id: item.id,
                title: item.title,
                updated_at: item.updated_at,
                status: item.status,
                relative_path: item.relative_path,
                size_bytes: lower_bound,
                sha256: String::new(),
            },
        });
    }

    // Reject selections known to be oversized before allocating or decoding any payload.
    let layout = validate_zip_store_layout(selected.iter().map(export_file_layout))?;
    let prepared = prepare_export_files(
        selected,
        ZIP_V1_MAX_BYTES - layout.total_bytes(),
        &mut bundle.errors,
    )?;
    validate_export_layout(&prepared, &bundle.exported_at)?;

    for ExportFile { path, mut session } in prepared {
        let data = match read_exported_data(&path, session.size_bytes as usize) {
            Ok(data) => data,
            Err(err) => {
                bundle.errors.push(err);
                continue;
            }
        };
        session.sha256 = sha256_bytes(&data);
        bundle.total_size += session.size_bytes;
        bundle.entries.push((session.relative_path.clone(), data));
        bundle.sessions.push(session);
    }
    Ok(bundle)
}

fn read_exported_data(path: &Path, size: usize) -> Result<Vec<u8>, String> {
    let read_error = |err| format!("读取会话文件失败 {}: {err}", path.display());
    let mut reader = open_rollout(path).map_err(read_error)?;
    let mut data = vec![0; size];
    let changed = || format!("导出期间会话文件大小变化，请重新导出: {}", path.display());
    match reader.read_exact(&mut data) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::UnexpectedEof => return Err(changed()),
        Err(err) => return Err(read_error(err)),
    }
    if reader.read(&mut [0]).map_err(read_error)? != 0 {
        return Err(changed());
    }
    Ok(data)
}

fn export_file_layout(file: &ExportFile) -> (&str, u64) {
    (&file.session.relative_path, file.session.size_bytes)
}

fn export_file_lower_bound(path: &Path, file_size: u64) -> io::Result<u64> {
    if !is_compressed_rollout(path) {
        return Ok(file_size);
    }
    // Zstandard's largest frame header is 18 bytes. Its content size is only a lower bound:
    // it can be omitted, and a rollout can be a concatenation of more than one frame.
    let mut header = [0u8; 18];
    let needed = file_size.min(header.len() as u64) as usize;
    let header = &mut header[..needed];
    fs::File::open(path)?.read_exact(header)?;
    zstd::zstd_safe::get_frame_content_size(header)
        .map(|size| size.unwrap_or(0))
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err.to_string()))
}

fn prepare_export_files(
    files: Vec<ExportFile>,
    mut extra_budget: u64,
    errors: &mut Vec<String>,
) -> Result<Vec<ExportFile>, String> {
    let mut prepared = Vec::with_capacity(files.len());
    for mut file in files {
        if is_compressed_rollout(&file.path) {
            let bound = file.session.size_bytes;
            let budget = bound + extra_budget;
            let bytes = match open_rollout(&file.path)
                .and_then(|reader| io::copy(&mut reader.take(budget + 1), &mut io::sink()))
            {
                Ok(bytes) => bytes,
                Err(err) => {
                    errors.push(format!("读取会话文件失败 {}: {err}", file.path.display()));
                    extra_budget += bound;
                    continue;
                }
            };
            if bytes > budget {
                return Err(format!(
                    "{ZIP_V1_LIMIT_ERROR}；{} 至少有 {bytes} 字节，剩余容量 {budget} 字节",
                    file.path.display()
                ));
            }
            extra_budget = budget - bytes;
            file.session.size_bytes = bytes;
        }
        prepared.push(file);
    }
    Ok(prepared)
}

fn validate_export_layout(files: &[ExportFile], exported_at: &str) -> Result<(), String> {
    // Real SHA-256 values are 64 unescaped ASCII characters. Use equal-width placeholders only
    // to measure the manifest before buffering the data; the written manifest uses actual hashes.
    let sessions = files
        .iter()
        .map(|file| {
            let mut session = file.session.clone();
            session.sha256 = "0".repeat(64);
            session
        })
        .collect();
    let manifest_size = ExportManifest::new(exported_at.to_string(), sessions)
        .bytes()?
        .len() as u64;
    validate_zip_store_layout(
        std::iter::once(("manifest.json", manifest_size))
            .chain(files.iter().map(export_file_layout)),
    )?;
    Ok(())
}

pub(super) fn import_conversations_impl(app: AppHandle, root: String) -> Result<Value, String> {
    let root = resolve_codex_root(Some(&root))?;
    validate_codex_root(&root)?;

    let selected = app
        .dialog()
        .file()
        .set_title("导入 Codex 会话")
        .add_filter("Codex Context", &["codexctx.zip", "zip"])
        .blocking_pick_file()
        .ok_or_else(|| "导入已取消".to_string())?;
    let import_path = selected
        .into_path()
        .map_err(|err| format!("导入文件路径无效: {err}"))?;

    let archive = ZipArchiveLite::open(&import_path)?;
    let manifest_bytes = archive.read_entry("manifest.json")?;
    let manifest: ExportManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|err| format!("manifest.json 格式无效: {err}"))?;
    validate_manifest(&manifest)?;

    let mut candidates = Vec::new();
    let mut errors = Vec::new();
    let mut conflicts = Vec::new();
    let mut active_count = 0usize;
    let mut archived_count = 0usize;

    for session in &manifest.sessions {
        if session.status == "archived" {
            archived_count += 1;
        } else {
            active_count += 1;
        }
        match build_import_candidate(&root, &archive, session) {
            Ok(candidate) => {
                if candidate.action == ImportAction::Conflict {
                    conflicts.push(json!({
                        "id": session.id,
                        "title": session.title,
                        "relative_path": session.relative_path
                    }));
                }
                candidates.push(candidate);
            }
            Err(err) => {
                errors.push(format!("{}: {err}", session.relative_path));
                candidates.push(ImportCandidate {
                    manifest: session.clone(),
                    data: Vec::new(),
                    target_path: root.join("invalid"),
                    action: ImportAction::Error,
                });
            }
        }
    }

    let importable_count = candidates
        .iter()
        .filter(|candidate| candidate.action == ImportAction::Import)
        .count();
    let skipped_count = candidates
        .iter()
        .filter(|candidate| candidate.action == ImportAction::SkipSame)
        .count();
    let choice = app
        .dialog()
        .message(format!(
            "来源文件：{}\n会话数量：{} 个\n进行中：{} 个，已归档：{} 个\n可导入：{} 个，重复跳过：{} 个，冲突：{} 个，错误：{} 个\n\n冲突文件不会被覆盖。",
            import_path.display(),
            manifest.sessions.len(),
            active_count,
            archived_count,
            importable_count,
            skipped_count,
            conflicts.len(),
            errors.len()
        ))
        .title("确认导入 Codex 会话")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "导入".to_string(),
            "取消".to_string(),
        ))
        .blocking_show_with_result();
    match choice {
        MessageDialogResult::Ok => {}
        MessageDialogResult::Custom(label) if label == "导入" => {}
        _ => return Err("导入已取消".to_string()),
    }

    let outcome = apply_import_candidates(&root, &candidates)?;
    conflicts.extend(outcome.conflicts);
    errors.extend(outcome.errors);
    let mut message = format!(
        "导入完成：{} 个导入，{} 个跳过，{} 个冲突",
        outcome.imported,
        outcome.skipped,
        conflicts.len()
    );
    if !errors.is_empty() {
        message.push_str(&format!("，{} 个失败", errors.len()));
    }
    if let Some(err) = &outcome.sqlite_error {
        message.push_str(&format!(
            "；Codex state 数据库未更新（会话文件已保留，重新导入同一文件可补写索引）：{err}"
        ));
    }

    Ok(json!({
        "ok": errors.is_empty() && outcome.sqlite_error.is_none(),
        "message": message,
        "report": {
            "path": import_path.to_string_lossy().to_string(),
            "imported": outcome.imported,
            "skipped": outcome.skipped,
            "conflicts": conflicts,
            "failed": errors.len(),
            "errors": errors,
            "sqlite_updated": outcome.sqlite_updated,
            "sqlite_error": outcome.sqlite_error,
            "state_backup_path": outcome
                .state_backup_path
                .map(|path| path.to_string_lossy().to_string())
        }
    }))
}

pub(super) struct ImportApplyOutcome {
    pub(super) imported: usize,
    pub(super) skipped: usize,
    pub(super) conflicts: Vec<Value>,
    pub(super) errors: Vec<String>,
    pub(super) sqlite_updated: usize,
    pub(super) sqlite_error: Option<String>,
    pub(super) state_backup_path: Option<PathBuf>,
}

/// Writes the confirmed candidates under the session I/O lock. The classification shown in the
/// confirmation dialog was made without the lock, so it is re-checked here: new files are created
/// with `create_new` (a file that appeared meanwhile is a conflict, never overwritten) and
/// "same content" skips are re-hashed. One failed file does not stop the batch; every file that
/// is on disk afterwards is indexed in one upsert.
pub(super) fn apply_import_candidates(
    root: &Path,
    candidates: &[ImportCandidate],
) -> Result<ImportApplyOutcome, String> {
    let _io_guard = lock_codex_session_io("导入会话")?;
    let state_db = codex_state_db_path_for_root(root)?;
    let state_backup_path = if state_db.exists()
        && candidates.iter().any(|candidate| {
            matches!(
                candidate.action,
                ImportAction::Import | ImportAction::SkipSame
            )
        }) {
        Some(backup_state_database_file(&state_db, "import")?)
    } else {
        None
    };

    let mut outcome = ImportApplyOutcome {
        imported: 0,
        skipped: 0,
        conflicts: Vec::new(),
        errors: Vec::new(),
        sqlite_updated: 0,
        sqlite_error: None,
        state_backup_path,
    };
    let mut indexed = Vec::new();
    for candidate in candidates {
        match candidate.action {
            ImportAction::Import => match write_new_import_file(candidate) {
                Ok(()) => {
                    outcome.imported += 1;
                    indexed.push((candidate, candidate.target_path.clone()));
                }
                Err(ImportWriteError::AlreadyExists) => {
                    outcome.conflicts.push(import_conflict_json(
                        candidate,
                        "目标文件在确认导入后出现，未覆盖",
                    ));
                }
                Err(ImportWriteError::Failed(err)) => outcome
                    .errors
                    .push(format!("{}: {err}", candidate.manifest.relative_path)),
            },
            ImportAction::SkipSame => match existing_import_file(root, &candidate.target_path) {
                Ok(Some((path, sha))) if sha == candidate.manifest.sha256 => {
                    outcome.skipped += 1;
                    indexed.push((candidate, path));
                }
                Ok(_) => outcome.conflicts.push(import_conflict_json(
                    candidate,
                    "目标文件在确认导入后发生变化，未覆盖",
                )),
                Err(err) => outcome.errors.push(format!(
                    "{}: 复核已存在会话失败: {err}",
                    candidate.manifest.relative_path
                )),
            },
            ImportAction::Conflict | ImportAction::Error => {}
        }
    }

    if state_db.exists() && !indexed.is_empty() {
        let thread_metadata = indexed
            .iter()
            .map(|(candidate, path)| {
                let summary = parse_session_file_for_list(path).unwrap_or_default();
                thread_metadata_from_manifest(&candidate.manifest, path, &summary)
            })
            .collect::<Vec<_>>();
        match upsert_state_threads(root, &thread_metadata) {
            Ok(updated) => outcome.sqlite_updated = updated,
            Err(err) => outcome.sqlite_error = Some(err),
        }
    }

    if !outcome.errors.is_empty() || outcome.sqlite_error.is_some() {
        log_event(
            "session_manager_import_apply_error",
            json!({
                "root": root.to_string_lossy().to_string(),
                "candidates": candidates.len(),
                "imported": outcome.imported,
                "skipped": outcome.skipped,
                "lateConflicts": outcome.conflicts.len(),
                "indexedFiles": indexed.len(),
                "errors": outcome.errors,
                "sqliteError": outcome.sqlite_error
            }),
        );
    }
    Ok(outcome)
}

enum ImportWriteError {
    AlreadyExists,
    Failed(String),
}

fn write_new_import_file(candidate: &ImportCandidate) -> Result<(), ImportWriteError> {
    let path = &candidate.target_path;
    // Compression can create the sibling after the confirmation dialog. create_new also
    // protects the plain path if that one appears after this check.
    match resolve_rollout_file(path) {
        Ok(_) => return Err(ImportWriteError::AlreadyExists),
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => {
            return Err(ImportWriteError::Failed(format!(
                "复核导入目标失败 {}: {err}",
                path.display()
            )))
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            ImportWriteError::Failed(format!("创建导入目录失败 {}: {err}", parent.display()))
        })?;
    }
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {
            return Err(ImportWriteError::AlreadyExists)
        }
        Err(err) => {
            return Err(ImportWriteError::Failed(format!(
                "创建导入会话失败 {}: {err}",
                path.display()
            )))
        }
    };
    if let Err(err) = file.write_all(&candidate.data) {
        drop(file);
        let mut message = format!("写入导入会话失败 {}: {err}", path.display());
        // create_new guarantees this call created the file, so the partial copy is ours to remove.
        if let Err(remove_err) = fs::remove_file(path) {
            message.push_str(&format!("；清理未完成文件失败: {remove_err}"));
        }
        return Err(ImportWriteError::Failed(message));
    }
    Ok(())
}

fn import_conflict_json(candidate: &ImportCandidate, reason: &str) -> Value {
    json!({
        "id": candidate.manifest.id,
        "title": candidate.manifest.title,
        "relative_path": candidate.manifest.relative_path,
        "reason": reason
    })
}

fn existing_import_file(
    root: &Path,
    logical_path: &Path,
) -> Result<Option<(PathBuf, String)>, String> {
    let path = match resolve_rollout_file(logical_path) {
        Ok((path, _)) => path,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(format!(
                "读取会话文件失败 {}: {err}",
                logical_path.display()
            ))
        }
    };
    validate_session_file_path(root, &path)?;
    let sha = open_rollout(&path)
        .and_then(sha256_reader)
        .map_err(|err| format!("读取会话文件失败 {}: {err}", path.display()))?;
    Ok(Some((path, sha)))
}

pub(super) fn build_import_candidate(
    root: &Path,
    archive: &ZipArchiveLite,
    session: &ManifestSession,
) -> Result<ImportCandidate, String> {
    let relative = normalize_relative_path(&session.relative_path)?;
    ensure_session_relative_path(&relative)?;
    if status_from_relative_path(&relative)? != session.status {
        return Err("manifest status 与 relative_path 不一致".to_string());
    }
    let data = archive.read_entry(&session.relative_path)?;
    let actual_sha = sha256_bytes(&data);
    if actual_sha != session.sha256 {
        return Err("sha256 校验失败".to_string());
    }
    if session.size_bytes != data.len() as u64 {
        return Err("文件大小与 manifest 不一致".to_string());
    }
    let target_path = root.join(relative);
    let _io_guard = lock_codex_session_io("检查会话导入冲突")?;
    let action = match existing_import_file(root, &target_path)? {
        Some((_, sha)) if sha == session.sha256 => ImportAction::SkipSame,
        Some(_) => ImportAction::Conflict,
        None => ImportAction::Import,
    };
    Ok(ImportCandidate {
        manifest: session.clone(),
        data,
        target_path,
        action,
    })
}

fn validate_manifest(manifest: &ExportManifest) -> Result<(), String> {
    if manifest.format != MANIFEST_FORMAT {
        return Err("manifest format 不受支持".to_string());
    }
    if manifest.version != MANIFEST_VERSION {
        return Err(format!("manifest version 不受支持: {}", manifest.version));
    }
    let mut seen = HashSet::new();
    for session in &manifest.sessions {
        if session.id.trim().is_empty() {
            return Err("manifest 中存在空会话 ID".to_string());
        }
        if !seen.insert(session.relative_path.clone()) {
            return Err(format!(
                "manifest 中存在重复路径: {}",
                session.relative_path
            ));
        }
        let status = normalize_status(&session.status)?;
        let relative = normalize_relative_path(&session.relative_path)?;
        ensure_session_relative_path(&relative)?;
        if status_from_relative_path(&relative)? != status {
            return Err(format!(
                "manifest 状态与路径不一致: {}",
                session.relative_path
            ));
        }
        if session.sha256.len() != 64 || !session.sha256.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(format!("manifest sha256 无效: {}", session.relative_path));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_manager::backup::unique_backup_id;

    fn prepared_file(path: PathBuf, bytes: u64) -> ExportFile {
        ExportFile {
            path,
            session: ManifestSession {
                id: "fixture".to_string(),
                title: "fixture".to_string(),
                updated_at: None,
                status: "active".to_string(),
                relative_path: "sessions/rollout-fixture.jsonl".to_string(),
                size_bytes: bytes,
                sha256: String::new(),
            },
        }
    }

    #[test]
    fn export_size_preflight_counts_every_compressed_frame() {
        let root = std::env::temp_dir().join(unique_backup_id("zip-preflight"));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("rollout-fixture.jsonl.zst");
        let mut compressed = zstd::bulk::compress(b"first", 0).unwrap();
        compressed.extend(zstd::bulk::compress(b"second", 0).unwrap());
        fs::write(&path, &compressed).unwrap();
        let lower_bound = export_file_lower_bound(&path, compressed.len() as u64).unwrap();

        assert_eq!(lower_bound, 5);
        let mut errors = Vec::new();
        let result = prepare_export_files(
            vec![prepared_file(path.clone(), lower_bound)],
            0,
            &mut errors,
        );
        assert!(result.err().unwrap().starts_with(ZIP_V1_LIMIT_ERROR));
        let files = prepare_export_files(
            vec![prepared_file(path.clone(), lower_bound)],
            6,
            &mut errors,
        )
        .unwrap();
        assert!(errors.is_empty());
        assert_eq!(files[0].session.size_bytes, 11);
        assert_eq!(read_exported_data(&path, 11).unwrap(), b"firstsecond");
        assert!(read_exported_data(&path, 10)
            .unwrap_err()
            .contains("大小变化"));
        assert!(read_exported_data(&path, 12)
            .unwrap_err()
            .contains("大小变化"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_manifest_counts_toward_the_limit_before_buffering_payloads() {
        let entry = prepared_file(PathBuf::new(), ZIP_V1_MAX_BYTES - 160);
        validate_zip_store_layout([export_file_layout(&entry)]).unwrap();
        let err = validate_export_layout(&[entry], "2026-11-18T00:00:00Z").unwrap_err();
        assert!(err.starts_with(ZIP_V1_LIMIT_ERROR), "{err}");
    }
}
