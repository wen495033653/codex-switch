use super::{
    backup::{
        sanitize_id_fragment, session_manager_data_dir, status_overwrite_backup_path,
        unique_backup_id,
    },
    codex_home::{
        conversation_path_key, ensure_session_relative_path, extract_uuid_like,
        normalize_relative_path, normalize_status, path_to_slash, reassigned_relative_path,
        remove_from_global_state, resolve_codex_root, session_id_variants,
        status_from_relative_path, validate_codex_root, validate_session_file_path,
    },
    model::{ConflictStrategy, ManifestSession, SessionSummary},
    rollout::{
        conversation_title_from_summary, copy_session_with_new_id, new_session_id,
        parse_session_file_for_list,
    },
    state_db::{
        delete_state_threads_for_sessions, thread_metadata_from_manifest, upsert_state_threads,
    },
    util::{backup_stamp, sha256_file, unique_sibling_path},
};
use crate::session_sync_diagnostics::log_session_sync_event;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const DELETED_SESSIONS_DIR: &str = "deleted-sessions";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct DeletedSessionRecord {
    pub(super) delete_id: String,
    pub(super) id: String,
    pub(super) title: String,
    pub(super) deleted_at: String,
    pub(super) updated_at: Option<String>,
    pub(super) original_status: String,
    pub(super) original_relative_path: String,
    pub(super) deleted_relative_path: String,
    pub(super) root_path: String,
    pub(super) size_bytes: u64,
    pub(super) cwd: Option<String>,
    pub(super) session_file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) sha256: Option<String>,
    #[serde(default = "default_deleted_session_state")]
    pub(super) state: String,
}

#[derive(Debug, Clone)]
pub(super) struct DeleteCandidate {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) updated_at: Option<String>,
    pub(super) source_path: PathBuf,
    pub(super) relative_path: PathBuf,
    pub(super) summary: SessionSummary,
}

#[derive(Debug, Clone)]
pub(super) struct RestoreCandidate {
    pub(super) record: DeletedSessionRecord,
    pub(super) record_dir: PathBuf,
    pub(super) source_file: PathBuf,
    pub(super) root: PathBuf,
    pub(super) target_path: PathBuf,
    pub(super) target_relative: PathBuf,
    pub(super) target_id: String,
    pub(super) rewrite_id: Option<(String, String)>,
    pub(super) overwritten_id: Option<String>,
}

pub(super) fn save_deleted_session_record(
    deleted_root: &Path,
    root: &Path,
    candidate: &DeleteCandidate,
    original_status: &str,
    deleted_at: &str,
) -> Result<DeletedSessionRecord, String> {
    fs::create_dir_all(deleted_root)
        .map_err(|err| format!("创建已删除会话目录失败 {}: {err}", deleted_root.display()))?;
    let (delete_id, record_dir) = create_deleted_session_record_dir(deleted_root, &candidate.id)?;
    let result = (|| {
        let session_file = record_dir.join("session.jsonl");
        let temp_file = temporary_sibling_path(&session_file, "delete-copy")?;
        let sha256 = copy_file_verified(&candidate.source_path, &temp_file, None)?;
        fs::rename(&temp_file, &session_file).map_err(|err| {
            format!(
                "保存已删除会话备份失败 {} -> {}: {err}",
                temp_file.display(),
                session_file.display()
            )
        })?;
        let size_bytes = session_file
            .metadata()
            .map_err(|err| {
                format!(
                    "读取已删除会话备份信息失败 {}: {err}",
                    session_file.display()
                )
            })?
            .len();
        let record = DeletedSessionRecord {
            delete_id,
            id: candidate.id.clone(),
            title: if candidate.title.trim().is_empty() {
                "未命名会话".to_string()
            } else {
                candidate.title.clone()
            },
            deleted_at: deleted_at.to_string(),
            updated_at: candidate
                .updated_at
                .clone()
                .or_else(|| candidate.summary.updated_at.clone()),
            original_status: original_status.to_string(),
            original_relative_path: path_to_slash(&candidate.relative_path),
            deleted_relative_path: path_to_slash(&candidate.relative_path),
            root_path: root.to_string_lossy().to_string(),
            size_bytes,
            cwd: candidate.summary.cwd.clone(),
            session_file: "session.jsonl".to_string(),
            sha256: Some(sha256),
            state: "prepared".to_string(),
        };
        write_deleted_session_record(&record_dir, &record)?;
        let stored_record = read_deleted_session_record(&record_dir)?;
        validate_deleted_record_identity(&record.delete_id, &stored_record)?;
        verify_deleted_session_backup(&stored_record, &session_file)?;
        Ok(record)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&record_dir);
    }
    result
}

fn create_deleted_session_record_dir(
    deleted_root: &Path,
    session_id: &str,
) -> Result<(String, PathBuf), String> {
    for _ in 0..16 {
        let delete_id = unique_delete_id(session_id);
        let record_dir = deleted_session_record_dir_at(deleted_root, &delete_id)?;
        match fs::create_dir(&record_dir) {
            Ok(()) => return Ok((delete_id, record_dir)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(format!(
                    "创建已删除会话记录目录失败 {}: {err}",
                    record_dir.display()
                ));
            }
        }
    }
    Err("生成已删除会话记录 ID 失败，请重试".to_string())
}

pub(super) fn discard_uncommitted_deleted_record(record_dir: &Path, errors: &mut Vec<String>) {
    if record_dir.exists() {
        if let Err(err) = fs::remove_dir_all(record_dir) {
            errors.push(format!(
                "清理未完成的已删除会话备份失败 {}: {err}",
                record_dir.display()
            ));
        }
    }
}

fn copy_file_verified(
    source: &Path,
    target: &Path,
    expected_sha256: Option<&str>,
) -> Result<String, String> {
    fs::copy(source, target).map_err(|err| {
        format!(
            "复制会话文件失败 {} -> {}: {err}",
            source.display(),
            target.display()
        )
    })?;
    sync_file_contents(target)?;
    let source_sha = sha256_file(source)?;
    let target_sha = sha256_file(target)?;
    if source_sha != target_sha {
        let _ = fs::remove_file(target);
        return Err(format!(
            "会话备份 SHA-256 校验失败 {} -> {}",
            source.display(),
            target.display()
        ));
    }
    if expected_sha256.is_some_and(|expected| expected != target_sha) {
        let _ = fs::remove_file(target);
        return Err(format!(
            "已删除会话备份 SHA-256 不匹配: {}",
            source.display()
        ));
    }
    Ok(target_sha)
}

fn sync_file_contents(path: &Path) -> Result<(), String> {
    fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|err| format!("同步文件失败 {}: {err}", path.display()))
}

fn write_deleted_session_record(
    record_dir: &Path,
    record: &DeletedSessionRecord,
) -> Result<(), String> {
    let path = record_dir.join("metadata.json");
    let mut content = serde_json::to_vec_pretty(record)
        .map_err(|err| format!("序列化已删除会话元数据失败: {err}"))?;
    content.push(b'\n');
    write_new_file_atomically(&path, &content, "delete-metadata")
}

fn write_new_file_atomically(path: &Path, content: &[u8], label: &str) -> Result<(), String> {
    if path.exists() {
        return Err(format!("目标文件已存在，拒绝覆盖: {}", path.display()));
    }
    let temp_path = temporary_sibling_path(path, label)?;
    let result = (|| {
        let mut file = fs::File::create(&temp_path)
            .map_err(|err| format!("创建临时文件失败 {}: {err}", temp_path.display()))?;
        file.write_all(content)
            .map_err(|err| format!("写入临时文件失败 {}: {err}", temp_path.display()))?;
        file.sync_all()
            .map_err(|err| format!("同步临时文件失败 {}: {err}", temp_path.display()))?;
        drop(file);
        fs::rename(&temp_path, path).map_err(|err| {
            format!(
                "原子保存文件失败 {} -> {}: {err}",
                temp_path.display(),
                path.display()
            )
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn temporary_sibling_path(path: &Path, label: &str) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("临时文件名无效: {}", path.display()))?;
    let base_name = format!(
        ".{file_name}.codex-switch-{label}-{}",
        unique_backup_id("temp")
    );
    Ok(unique_sibling_path(path, &base_name))
}

pub(super) fn read_deleted_session_records_from_dir(
    deleted_root: &Path,
) -> Result<(Vec<DeletedSessionRecord>, Vec<String>), String> {
    if !deleted_root.exists() {
        return Ok((Vec::new(), Vec::new()));
    }
    let entries = fs::read_dir(deleted_root)
        .map_err(|err| format!("读取已删除会话目录失败 {}: {err}", deleted_root.display()))?;
    let mut records = Vec::new();
    let mut errors = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                errors.push(format!("读取已删除会话目录项失败: {err}"));
                continue;
            }
        };
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(err) => {
                errors.push(format!("读取已删除会话目录项类型失败: {err}"));
                continue;
            }
        };
        if !file_type.is_dir() {
            continue;
        }
        let delete_id = entry.file_name().to_string_lossy().to_string();
        match read_deleted_session_record(&entry.path()).and_then(|record| {
            validate_deleted_record_identity(&delete_id, &record)?;
            let Some(record) = recover_deleted_session_record_state(&entry.path(), record)? else {
                return Ok(None);
            };
            let session_file = deleted_record_session_path(&entry.path(), &record)?;
            verify_deleted_session_backup(&record, &session_file)?;
            Ok(Some(record))
        }) {
            Ok(Some(record)) => records.push(record),
            Ok(None) => {}
            Err(err) => errors.push(err),
        }
    }
    Ok((records, errors))
}

pub(super) fn read_deleted_session_record(
    record_dir: &Path,
) -> Result<DeletedSessionRecord, String> {
    let path = record_dir.join("metadata.json");
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("读取已删除会话元数据失败 {}: {err}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|err| format!("解析已删除会话元数据失败 {}: {err}", path.display()))
}

fn default_deleted_session_state() -> String {
    "ready".to_string()
}

pub(super) fn mark_deleted_session_ready(record_dir: &Path) -> Result<(), String> {
    let marker = record_dir.join("ready");
    write_new_file_atomically(&marker, b"ready\n", "delete-ready")
}

pub(super) fn recover_deleted_session_record_state(
    record_dir: &Path,
    mut record: DeletedSessionRecord,
) -> Result<Option<DeletedSessionRecord>, String> {
    match record.state.trim().to_ascii_lowercase().as_str() {
        "" | "ready" => {
            record.state = "ready".to_string();
            Ok(Some(record))
        }
        "prepared" => {
            if record_dir.join("ready").exists() {
                record.state = "ready".to_string();
                return Ok(Some(record));
            }
            let original_path = deleted_record_original_path(&record)?;
            if original_path.exists() {
                return Ok(None);
            }
            record.state = "ready".to_string();
            Ok(Some(record))
        }
        other => Err(format!(
            "已删除会话记录状态无效 {}: {other}",
            record.delete_id
        )),
    }
}

fn deleted_record_original_path(record: &DeletedSessionRecord) -> Result<PathBuf, String> {
    let root = record.root_path.trim();
    if root.is_empty() {
        return Err(format!("已删除会话缺少原 Codex 数据目录: {}", record.title));
    }
    let relative = normalize_relative_path(&record.original_relative_path)?;
    ensure_session_relative_path(&relative)?;
    Ok(PathBuf::from(root).join(relative))
}

pub(super) fn validate_deleted_record_identity(
    requested_delete_id: &str,
    record: &DeletedSessionRecord,
) -> Result<(), String> {
    validate_delete_id(requested_delete_id)?;
    validate_delete_id(&record.delete_id)?;
    if record.delete_id != requested_delete_id {
        return Err(format!(
            "已删除会话记录 ID 不一致: {requested_delete_id} != {}",
            record.delete_id
        ));
    }
    Ok(())
}

pub(super) fn deleted_record_session_path(
    record_dir: &Path,
    record: &DeletedSessionRecord,
) -> Result<PathBuf, String> {
    let session_file = record.session_file.trim();
    if session_file.is_empty() {
        return Err(format!("已删除会话备份文件名为空: {}", record.delete_id));
    }
    let relative = Path::new(session_file);
    let mut components = relative.components();
    let only_component =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if !only_component
        || relative
            .extension()
            .and_then(|value| value.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("jsonl"))
    {
        return Err(format!("已删除会话备份文件名无效: {}", record.session_file));
    }
    Ok(record_dir.join(relative))
}

pub(super) fn verify_deleted_session_backup(
    record: &DeletedSessionRecord,
    session_file: &Path,
) -> Result<(), String> {
    let metadata = session_file.metadata().map_err(|err| {
        format!(
            "读取已删除会话备份信息失败 {}: {err}",
            session_file.display()
        )
    })?;
    if metadata.len() != record.size_bytes {
        return Err(format!("已删除会话备份大小不匹配: {}", record.delete_id));
    }
    if let Some(expected) = record.sha256.as_deref() {
        let actual = sha256_file(session_file)?;
        if actual != expected {
            return Err(format!(
                "已删除会话备份 SHA-256 不匹配: {}",
                record.delete_id
            ));
        }
    }
    Ok(())
}

pub(super) fn build_restore_deleted_candidate(
    deleted_root: &Path,
    delete_id: &str,
    conflict_strategy: ConflictStrategy,
) -> Result<Option<RestoreCandidate>, String> {
    let record_dir = deleted_session_record_dir_at(deleted_root, delete_id)?;
    let record = read_deleted_session_record(&record_dir)?;
    validate_deleted_record_identity(delete_id, &record)?;
    let record = recover_deleted_session_record_state(&record_dir, record)?
        .ok_or_else(|| "删除操作尚未完成，原会话文件仍然存在".to_string())?;
    let source_file = deleted_record_session_path(&record_dir, &record)?;
    verify_deleted_session_backup(&record, &source_file)?;
    let root_path = record.root_path.trim();
    if root_path.is_empty() {
        return Err(format!("已删除会话缺少原 Codex 数据目录: {}", record.title));
    }
    let root = resolve_codex_root(Some(root_path))?;
    validate_codex_root(&root)?;
    let relative = normalize_relative_path(&record.original_relative_path)?;
    ensure_session_relative_path(&relative)?;
    let original_status = normalize_status(&record.original_status)?;
    if status_from_relative_path(&relative)? != original_status {
        return Err(format!("已删除会话状态与原路径不一致: {}", record.title));
    }
    let original_target_path = root.join(&relative);
    let mut target_path = original_target_path.clone();
    let mut target_relative = relative.clone();
    let mut target_id = record.id.clone();
    let mut rewrite_id = None;
    let mut overwritten_id = None;

    if original_target_path.exists() {
        validate_session_file_path(&root, &original_target_path)?;
        match conflict_strategy {
            ConflictStrategy::Ask => {
                return Err(format!("CONFLICT:{}", record.original_relative_path));
            }
            ConflictStrategy::Skip => return Ok(None),
            ConflictStrategy::Overwrite => {
                overwritten_id = parse_session_file_for_list(&original_target_path)
                    .ok()
                    .and_then(|summary| summary.id)
                    .or_else(|| extract_uuid_like(&record.original_relative_path));
            }
            ConflictStrategy::ModifyId => {
                let mut new_id = new_session_id(&record.id);
                loop {
                    let reassigned = reassigned_relative_path(&relative, &record.id, &new_id)?;
                    let reassigned_path = root.join(&reassigned);
                    if !reassigned_path.exists() {
                        target_path = reassigned_path;
                        target_relative = reassigned;
                        target_id = new_id.clone();
                        rewrite_id = Some((record.id.clone(), new_id));
                        break;
                    }
                    new_id = new_session_id(&new_id);
                }
            }
        }
    }

    Ok(Some(RestoreCandidate {
        record,
        record_dir,
        source_file,
        root,
        target_path,
        target_relative,
        target_id,
        rewrite_id,
        overwritten_id,
    }))
}

pub(super) fn reassign_restore_candidate(
    candidate: &mut RestoreCandidate,
    reserved_targets: &HashSet<String>,
) -> Result<(), String> {
    let original_relative = normalize_relative_path(&candidate.record.original_relative_path)?;
    let mut new_id = new_session_id(&candidate.target_id);
    loop {
        let reassigned =
            reassigned_relative_path(&original_relative, &candidate.record.id, &new_id)?;
        let target_path = candidate.root.join(&reassigned);
        if !target_path.exists() && !reserved_targets.contains(&conversation_path_key(&target_path))
        {
            candidate.target_path = target_path;
            candidate.target_relative = reassigned;
            candidate.target_id = new_id.clone();
            candidate.rewrite_id = Some((candidate.record.id.clone(), new_id));
            candidate.overwritten_id = None;
            return Ok(());
        }
        new_id = new_session_id(&new_id);
    }
}

pub(super) fn restore_deleted_candidate(
    candidate: RestoreCandidate,
    conflict_strategy: ConflictStrategy,
) -> Result<(usize, bool, Vec<String>), String> {
    verify_deleted_session_backup(&candidate.record, &candidate.source_file)?;
    let parent = candidate
        .target_path
        .parent()
        .ok_or_else(|| format!("恢复目标目录无效: {}", candidate.target_path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|err| format!("创建恢复目录失败 {}: {err}", parent.display()))?;
    if candidate.target_path.exists() && conflict_strategy != ConflictStrategy::Overwrite {
        return Err(format!(
            "恢复目标在操作期间出现冲突，请重新选择处理方式: {}",
            candidate.target_path.display()
        ));
    }

    let temp_path = temporary_sibling_path(&candidate.target_path, "restore")?;
    if let Err(err) = prepare_restored_temp_file(&candidate, &temp_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }
    let overwrite_backup = if candidate.target_path.exists() {
        let backup = status_overwrite_backup_path(&candidate.target_path, &candidate.target_id);
        if let Err(err) = fs::rename(&candidate.target_path, &backup) {
            let _ = fs::remove_file(&temp_path);
            return Err(format!(
                "备份恢复覆盖目标失败 {}: {err}",
                candidate.target_path.display()
            ));
        }
        Some(backup)
    } else {
        None
    };

    if let Err(err) = fs::rename(&temp_path, &candidate.target_path) {
        let mut message = format!(
            "恢复会话失败 {} -> {}: {err}",
            candidate.source_file.display(),
            candidate.target_path.display()
        );
        append_restore_rollback_error(
            &mut message,
            &candidate.target_path,
            overwrite_backup.as_deref(),
        );
        let _ = fs::remove_file(&temp_path);
        return Err(message);
    }

    let summary = match parse_session_file_for_list(&candidate.target_path) {
        Ok(summary) => summary,
        Err(err) => {
            let mut message = format!("解析恢复后的会话失败: {err}");
            append_restore_rollback_error(
                &mut message,
                &candidate.target_path,
                overwrite_backup.as_deref(),
            );
            return Err(message);
        }
    };
    if summary
        .id
        .as_deref()
        .is_some_and(|id| id != candidate.target_id)
    {
        let mut message = format!("恢复后的会话 ID 不匹配: 期望 {}", candidate.target_id);
        append_restore_rollback_error(
            &mut message,
            &candidate.target_path,
            overwrite_backup.as_deref(),
        );
        return Err(message);
    }
    let manifest = ManifestSession {
        id: candidate.target_id.clone(),
        title: if should_rebuild_deleted_title(&candidate.record.title) {
            conversation_title_from_summary(&summary)
        } else {
            candidate.record.title.clone()
        },
        updated_at: candidate
            .record
            .updated_at
            .clone()
            .or_else(|| summary.updated_at.clone()),
        status: candidate.record.original_status.clone(),
        relative_path: path_to_slash(&candidate.target_relative),
        size_bytes: candidate
            .target_path
            .metadata()
            .map(|metadata| metadata.len())
            .unwrap_or(0),
        sha256: sha256_file(&candidate.target_path).unwrap_or_default(),
    };
    let thread_metadata =
        thread_metadata_from_manifest(&manifest, &candidate.target_path, &summary);
    let sqlite_updated = match upsert_state_threads(&candidate.root, &[thread_metadata]) {
        Ok(updated) => updated,
        Err(err) => {
            let mut message = format!("恢复 Codex Desktop state 失败: {err}");
            append_restore_rollback_error(
                &mut message,
                &candidate.target_path,
                overwrite_backup.as_deref(),
            );
            log_session_sync_event(
                "session_manager_restore_state_db_error",
                json!({
                    "deleteId": candidate.record.delete_id,
                    "root": candidate.root.to_string_lossy().to_string(),
                    "target": candidate.target_path.to_string_lossy().to_string(),
                    "trashRetained": candidate.record_dir.exists(),
                    "error": message
                }),
            );
            return Err(message);
        }
    };

    let mut warnings = Vec::new();
    if let Some(overwritten_id) = candidate
        .overwritten_id
        .as_deref()
        .filter(|id| *id != candidate.target_id)
    {
        let overwritten_ids = session_id_variants(overwritten_id);
        if let Err(err) = delete_state_threads_for_sessions(&candidate.root, &overwritten_ids, &[])
        {
            warnings.push(format!("清理被覆盖会话的 Desktop state 失败: {err}"));
        }
        if let Err(err) =
            remove_from_global_state(&candidate.root, &overwritten_ids, "restore-overwrite")
        {
            warnings.push(format!("清理被覆盖会话的 global state 失败: {err}"));
        }
    }

    if let Some(backup) = overwrite_backup {
        if let Err(err) = fs::remove_file(&backup) {
            warnings.push(format!("清理恢复覆盖备份失败 {}: {err}", backup.display()));
        }
    }
    let trash_removed = match fs::remove_dir_all(&candidate.record_dir) {
        Ok(()) => true,
        Err(err) => {
            warnings.push(format!(
                "恢复已完成，但清理回收站记录失败 {}: {err}",
                candidate.record_dir.display()
            ));
            false
        }
    };
    Ok((sqlite_updated, trash_removed, warnings))
}

fn prepare_restored_temp_file(
    candidate: &RestoreCandidate,
    temp_path: &Path,
) -> Result<(), String> {
    if let Some((old_id, new_id)) = &candidate.rewrite_id {
        let source_sha_before = sha256_file(&candidate.source_file)?;
        copy_session_with_new_id(&candidate.source_file, temp_path, old_id, new_id)?;
        sync_file_contents(temp_path)?;
        let source_sha_after = sha256_file(&candidate.source_file)?;
        if source_sha_before != source_sha_after {
            let _ = fs::remove_file(temp_path);
            return Err(format!(
                "恢复期间已删除会话备份发生变化: {}",
                candidate.record.delete_id
            ));
        }
        let summary = parse_session_file_for_list(temp_path)?;
        if summary.id.as_deref() != Some(new_id.as_str()) {
            let _ = fs::remove_file(temp_path);
            return Err(format!("修改恢复会话 ID 失败: {}", candidate.record.title));
        }
        Ok(())
    } else {
        copy_file_verified(
            &candidate.source_file,
            temp_path,
            candidate.record.sha256.as_deref(),
        )
        .map(|_| ())
    }
}

fn append_restore_rollback_error(
    message: &mut String,
    target_path: &Path,
    overwrite_backup: Option<&Path>,
) {
    if target_path.exists() {
        if let Err(err) = fs::remove_file(target_path) {
            message.push_str(&format!(
                "；清理未完成恢复目标失败 {}: {err}",
                target_path.display()
            ));
            return;
        }
    }
    if let Some(backup) = overwrite_backup {
        if let Err(err) = fs::rename(backup, target_path) {
            message.push_str(&format!(
                "；回滚原恢复目标失败 {}: {err}（备份保留于 {}）",
                target_path.display(),
                backup.display()
            ));
        }
    }
}

pub(super) fn should_rebuild_deleted_title(title: &str) -> bool {
    title.trim().is_empty() || title == "未命名会话"
}

pub(super) fn deleted_sessions_dir() -> Result<PathBuf, String> {
    Ok(session_manager_data_dir()?.join(DELETED_SESSIONS_DIR))
}

pub(super) fn deleted_session_record_dir_at(
    deleted_root: &Path,
    delete_id: &str,
) -> Result<PathBuf, String> {
    validate_delete_id(delete_id)?;
    Ok(deleted_root.join(delete_id))
}

fn validate_delete_id(delete_id: &str) -> Result<(), String> {
    if delete_id.trim().is_empty()
        || !delete_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        return Err("已删除会话 ID 无效".to_string());
    }
    Ok(())
}

fn unique_delete_id(id: &str) -> String {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{}-{}-{suffix}", backup_stamp(), sanitize_id_fragment(id))
}
