use crate::{
    codex_sessions::lock_codex_session_io,
    json_util::raw_string_field,
    paths::{app_data_dir, codex_dir},
    time_util::{now_string, parse_rfc3339_seconds},
};
use rusqlite::{params, params_from_iter, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read, Seek, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::AppHandle;
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};
use time::OffsetDateTime;

const MANIFEST_FORMAT: &str = "codex-context-manager";
const MANIFEST_VERSION: u32 = 1;
const ZIP_LOCAL_FILE_HEADER: u32 = 0x0403_4b50;
const ZIP_CENTRAL_DIRECTORY_HEADER: u32 = 0x0201_4b50;
const ZIP_END_OF_CENTRAL_DIRECTORY: u32 = 0x0605_4b50;
const ZIP_UTF8_FLAG: u16 = 1 << 11;
const SESSION_MANAGER_DATA_DIR: &str = "session-manager";
const DELETED_SESSIONS_DIR: &str = "deleted-sessions";

#[derive(Debug, Clone, Serialize)]
struct ConversationItem {
    id: String,
    title: String,
    updated_at: Option<String>,
    status: String,
    source_path: String,
    relative_path: String,
    size_bytes: u64,
    cwd: Option<String>,
    preview: Option<String>,
    sha256: Option<String>,
    parse_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeletedSessionRecord {
    delete_id: String,
    id: String,
    title: String,
    deleted_at: String,
    updated_at: Option<String>,
    original_status: String,
    original_relative_path: String,
    deleted_relative_path: String,
    root_path: String,
    size_bytes: u64,
    cwd: Option<String>,
    session_file: String,
}

#[derive(Debug, Clone, Serialize)]
struct ConversationMessage {
    role: String,
    text: String,
    timestamp: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct SessionSummary {
    id: Option<String>,
    title: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    cwd: Option<String>,
    model_provider: Option<String>,
    sandbox_policy: Option<String>,
    approval_mode: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
    first_user_message: Option<String>,
    preview: Option<String>,
    messages: Vec<ConversationMessage>,
    parse_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExportManifest {
    format: String,
    version: u32,
    exported_at: String,
    source_os: String,
    sessions: Vec<ManifestSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestSession {
    id: String,
    title: String,
    updated_at: Option<String>,
    status: String,
    relative_path: String,
    size_bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone)]
struct ImportCandidate {
    manifest: ManifestSession,
    data: Vec<u8>,
    target_path: PathBuf,
    action: ImportAction,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ImportAction {
    Import,
    SkipSame,
    Conflict,
    Error,
}

#[derive(Debug, Clone)]
struct ThreadMetadata {
    id: String,
    rollout_path: PathBuf,
    created_at: i64,
    updated_at: i64,
    source: String,
    model_provider: String,
    cwd: String,
    title: String,
    sandbox_policy: String,
    approval_mode: String,
    has_user_event: i64,
    archived: i64,
    archived_at: Option<i64>,
    first_user_message: String,
    model: Option<String>,
    reasoning_effort: Option<String>,
}

#[derive(Debug, Clone)]
struct DeleteCandidate {
    id: String,
    title: String,
    updated_at: Option<String>,
    source_path: PathBuf,
    relative_path: PathBuf,
}

#[derive(Debug, Clone)]
struct StatusMove {
    id: String,
    target_id: String,
    title: String,
    updated_at: Option<String>,
    source_path: PathBuf,
    target_path: PathBuf,
    rewrite_id: Option<(String, String)>,
    overwritten_id: Option<String>,
}

#[derive(Debug, Clone)]
struct RestoreCandidate {
    record: DeletedSessionRecord,
    record_dir: PathBuf,
    source_file: PathBuf,
    target_path: PathBuf,
    target_relative: String,
    target_id: String,
    rewrite_id: Option<(String, String)>,
    overwritten_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ConflictStrategy {
    Ask,
    Skip,
    Overwrite,
    ModifyId,
}

#[derive(Debug, Clone)]
struct IndexEntry {
    thread_name: Option<String>,
    updated_at: Option<String>,
}

fn blocking_task_error(action: &str, err: impl std::fmt::Display) -> String {
    let message = err.to_string();
    if message.contains("panicked") {
        format!("{action}任务异常，请重试")
    } else {
        format!("{action}任务异常: {message}")
    }
}

fn parse_conflict_strategy(value: Option<String>) -> Result<ConflictStrategy, String> {
    match value
        .as_deref()
        .unwrap_or("ask")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "ask" => Ok(ConflictStrategy::Ask),
        "skip" => Ok(ConflictStrategy::Skip),
        "overwrite" => Ok(ConflictStrategy::Overwrite),
        "modify_id" | "modify-id" | "modifyid" | "reassign_id" | "reassign-id" | "reassignid" => {
            Ok(ConflictStrategy::ModifyId)
        }
        other => Err(format!("不支持的冲突处理方式: {other}")),
    }
}

#[tauri::command]
pub(crate) async fn session_manager_scan(root: Option<String>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || scan_conversations_impl(root))
        .await
        .map_err(|err| blocking_task_error("扫描会话", err))?
}

#[tauri::command]
pub(crate) async fn session_manager_preview(
    root: String,
    relative_path: String,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || preview_conversation_impl(root, relative_path))
        .await
        .map_err(|err| blocking_task_error("读取预览", err))?
}

#[tauri::command]
pub(crate) async fn session_manager_preview_deleted(delete_id: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || preview_deleted_conversation_impl(delete_id))
        .await
        .map_err(|err| blocking_task_error("读取已删除预览", err))?
}

#[tauri::command]
pub(crate) fn session_manager_select_root(app: AppHandle) -> Result<Value, String> {
    let selected = app
        .dialog()
        .file()
        .set_title("选择 Codex 数据目录")
        .blocking_pick_folder()
        .ok_or_else(|| "选择目录已取消".to_string())?;
    let path = selected
        .into_path()
        .map_err(|err| format!("选择目录路径无效: {err}"))?;
    validate_codex_root(&path)?;
    Ok(json!({
        "ok": true,
        "path": path.to_string_lossy().to_string()
    }))
}

#[tauri::command]
pub(crate) fn session_manager_select_workdir(app: AppHandle) -> Result<Value, String> {
    let selected = app
        .dialog()
        .file()
        .set_title("选择工作目录")
        .blocking_pick_folder()
        .ok_or_else(|| "选择工作目录已取消".to_string())?;
    let path = selected
        .into_path()
        .map_err(|err| format!("选择工作目录路径无效: {err}"))?;
    Ok(json!({
        "ok": true,
        "path": path.to_string_lossy().to_string()
    }))
}

#[tauri::command]
pub(crate) async fn session_manager_export(
    app: AppHandle,
    root: String,
    relative_paths: Vec<String>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        export_conversations_impl(app, root, relative_paths)
    })
    .await
    .map_err(|err| blocking_task_error("导出会话", err))?
}

#[tauri::command]
pub(crate) async fn session_manager_import(app: AppHandle, root: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || import_conversations_impl(app, root))
        .await
        .map_err(|err| blocking_task_error("导入会话", err))?
}

#[tauri::command]
pub(crate) async fn session_manager_delete(
    root: String,
    relative_paths: Vec<String>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || delete_conversations_impl(root, relative_paths))
        .await
        .map_err(|err| blocking_task_error("删除会话", err))?
}

pub(crate) fn delete_codex_session_for_bridge(
    session_id: &str,
    title: &str,
) -> Result<Value, String> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err("未找到会话 ID".to_string());
    }

    let root = resolve_codex_root(None)?;
    validate_codex_root(&root)?;
    let relative_paths = find_session_relative_paths_by_id(&root, session_id)?;
    if relative_paths.is_empty() {
        let label = title.trim();
        return Err(if label.is_empty() {
            format!("未找到会话: {session_id}")
        } else {
            format!("未找到会话: {label}")
        });
    }

    delete_conversations_impl(root.to_string_lossy().to_string(), relative_paths)
}

#[tauri::command]
pub(crate) async fn session_manager_list_deleted() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(list_deleted_sessions_impl)
        .await
        .map_err(|err| blocking_task_error("读取已删除会话", err))?
}

#[tauri::command]
pub(crate) async fn session_manager_restore_deleted(
    root: String,
    delete_ids: Vec<String>,
    conflict_strategy: Option<String>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        restore_deleted_sessions_impl(root, delete_ids, conflict_strategy)
    })
    .await
    .map_err(|err| blocking_task_error("恢复会话", err))?
}

#[tauri::command]
pub(crate) async fn session_manager_purge_deleted(
    delete_ids: Vec<String>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || purge_deleted_sessions_impl(delete_ids))
        .await
        .map_err(|err| blocking_task_error("彻底删除会话", err))?
}

#[tauri::command]
pub(crate) async fn session_manager_set_status(
    root: String,
    relative_paths: Vec<String>,
    status: String,
    conflict_strategy: Option<String>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        set_conversation_status_impl(root, relative_paths, status, conflict_strategy)
    })
    .await
    .map_err(|err| blocking_task_error("切换会话状态", err))?
}

#[tauri::command]
pub(crate) async fn session_manager_update_cwd(
    root: String,
    relative_paths: Vec<String>,
    cwd: String,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        update_conversation_cwd_impl(root, relative_paths, cwd)
    })
    .await
    .map_err(|err| blocking_task_error("修改工作目录", err))?
}

fn scan_conversations_impl(root: Option<String>) -> Result<Value, String> {
    let root = resolve_codex_root(root.as_deref())?;
    validate_codex_root(&root)?;

    let mut warnings = Vec::new();
    let index = read_session_index(&root, &mut warnings);
    let mut errors = Vec::new();
    let mut conversations = Vec::new();

    let mut files = Vec::new();
    collect_conversation_files(&root.join("sessions"), "active", &mut files, &mut errors);
    collect_conversation_files(
        &root.join("archived_sessions"),
        "archived",
        &mut files,
        &mut errors,
    );

    for (status, path) in files {
        match conversation_from_path(&root, &path, &status, &index, false) {
            Ok(item) => conversations.push(item),
            Err(err) => errors.push(err),
        }
    }
    append_missing_state_thread_conversations(&root, &mut conversations, &mut warnings);
    warn_archived_session_index_entries(&conversations, &index, &mut warnings);

    conversations.sort_by(|a, b| {
        conversation_sort_key(b)
            .cmp(&conversation_sort_key(a))
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });

    Ok(json!({
        "ok": true,
        "root": root.to_string_lossy().to_string(),
        "conversations": conversations,
        "warnings": warnings,
        "errors": errors
    }))
}

fn preview_conversation_impl(root: String, relative_path: String) -> Result<Value, String> {
    let root = resolve_codex_root(Some(&root))?;
    validate_codex_root(&root)?;
    let relative = normalize_relative_path(&relative_path)?;
    ensure_session_relative_path(&relative)?;
    let path = root.join(&relative);
    if !path.exists() {
        return Err(format!("会话文件不存在: {}", relative.display()));
    }

    let mut warnings = Vec::new();
    let index = read_session_index(&root, &mut warnings);
    let status = status_from_relative_path(&relative)?;
    let item = conversation_from_path(&root, &path, &status, &index, false)?;
    let summary = parse_session_file(&path, true)?;

    Ok(json!({
        "ok": true,
        "conversation": item,
        "messages": summary.messages,
        "warnings": warnings,
        "parse_error": summary.parse_error
    }))
}

fn preview_deleted_conversation_impl(delete_id: String) -> Result<Value, String> {
    let record_dir = deleted_session_record_dir(&delete_id)?;
    preview_deleted_conversation_from_record_dir(&record_dir)
}

fn preview_deleted_conversation_from_record_dir(record_dir: &Path) -> Result<Value, String> {
    let record = read_deleted_session_record(record_dir)?;
    let session_file = record_dir.join(&record.session_file);
    if !session_file.exists() {
        return Err(format!("已删除会话备份文件缺失: {}", record.title));
    }
    let summary = parse_session_file(&session_file, true)?;
    let size_bytes = session_file
        .metadata()
        .map(|item| item.len())
        .unwrap_or(record.size_bytes);
    let title = if should_rebuild_deleted_title(&record.title) {
        deleted_title_from_summary(&summary)
    } else {
        record.title.clone()
    };
    let conversation = ConversationItem {
        id: record.id,
        title,
        updated_at: Some(record.deleted_at),
        status: "deleted".to_string(),
        source_path: session_file.to_string_lossy().to_string(),
        relative_path: record.original_relative_path,
        size_bytes,
        cwd: summary.cwd.clone().or(record.cwd),
        preview: summary.preview.clone(),
        sha256: None,
        parse_error: summary.parse_error.clone(),
    };

    Ok(json!({
        "ok": true,
        "conversation": conversation,
        "messages": summary.messages,
        "parse_error": summary.parse_error
    }))
}

fn export_conversations_impl(
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

    let mut warnings = Vec::new();
    let index = read_session_index(&root, &mut warnings);
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    let mut sessions = Vec::new();
    let mut errors = Vec::new();
    let mut total_size = 0u64;

    for relative_path in relative_paths {
        if !seen.insert(relative_path.clone()) {
            continue;
        }
        let relative = match normalize_relative_path(&relative_path) {
            Ok(relative) => relative,
            Err(err) => {
                errors.push(err);
                continue;
            }
        };
        if let Err(err) = ensure_session_relative_path(&relative) {
            errors.push(err);
            continue;
        }
        let path = root.join(&relative);
        let status = match status_from_relative_path(&relative) {
            Ok(status) => status,
            Err(err) => {
                errors.push(err);
                continue;
            }
        };
        match conversation_from_path(&root, &path, &status, &index, true) {
            Ok(item) => match fs::read(&path) {
                Ok(data) => {
                    let sha256 = item.sha256.clone().unwrap_or_else(|| sha256_bytes(&data));
                    total_size += item.size_bytes;
                    sessions.push(ManifestSession {
                        id: item.id,
                        title: item.title,
                        updated_at: item.updated_at,
                        status: item.status,
                        relative_path: item.relative_path.clone(),
                        size_bytes: item.size_bytes,
                        sha256,
                    });
                    entries.push((item.relative_path, data));
                }
                Err(err) => errors.push(format!("读取会话文件失败 {}: {err}", path.display())),
            },
            Err(err) => errors.push(err),
        }
    }

    if sessions.is_empty() {
        return Err(format!(
            "没有可导出的会话{}",
            if errors.is_empty() {
                String::new()
            } else {
                format!("：{}", errors.join("；"))
            }
        ));
    }

    let manifest = ExportManifest {
        format: MANIFEST_FORMAT.to_string(),
        version: MANIFEST_VERSION,
        exported_at: now_string(),
        source_os: std::env::consts::OS.to_string(),
        sessions,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|err| format!("生成 manifest.json 失败: {err}"))?;
    let mut zip_entries = vec![("manifest.json".to_string(), manifest_bytes)];
    zip_entries.extend(entries);
    write_zip_store(&export_path, &zip_entries)?;

    Ok(json!({
        "ok": true,
        "message": format!("导出完成（{} 个会话）", manifest.sessions.len()),
        "report": {
            "path": export_path.to_string_lossy().to_string(),
            "exported": manifest.sessions.len(),
            "total_size": total_size,
            "failed": errors.len(),
            "errors": errors,
            "warnings": warnings
        }
    }))
}

fn import_conversations_impl(app: AppHandle, root: String) -> Result<Value, String> {
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

    let _io_guard = lock_codex_session_io("导入会话")?;
    let index_backup_path = if candidates.iter().any(|candidate| {
        matches!(
            candidate.action,
            ImportAction::Import | ImportAction::SkipSame
        )
    }) {
        backup_session_index(&root)?
    } else {
        None
    };
    let state_backup_path = if root.join("state_5.sqlite").exists()
        && candidates.iter().any(|candidate| {
            matches!(
                candidate.action,
                ImportAction::Import | ImportAction::SkipSame
            )
        }) {
        Some(backup_file(&root.join("state_5.sqlite"))?)
    } else {
        None
    };

    let mut imported = 0usize;
    for candidate in &candidates {
        if candidate.action != ImportAction::Import {
            continue;
        }
        if let Some(parent) = candidate.target_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("创建导入目录失败 {}: {err}", parent.display()))?;
        }
        fs::write(&candidate.target_path, &candidate.data).map_err(|err| {
            format!(
                "写入导入会话失败 {}: {err}",
                candidate.target_path.display()
            )
        })?;
        imported += 1;
    }

    let index_updates: Vec<ManifestSession> = candidates
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.action,
                ImportAction::Import | ImportAction::SkipSame
            )
        })
        .map(|candidate| candidate.manifest.clone())
        .collect();
    if !index_updates.is_empty() {
        update_session_index_from_manifest(&root, &index_updates)?;
    }

    let mut sqlite_updated = 0usize;
    let mut sqlite_error = None;
    if root.join("state_5.sqlite").exists() && !index_updates.is_empty() {
        let mut thread_metadata = Vec::new();
        for candidate in candidates.iter().filter(|candidate| {
            matches!(
                candidate.action,
                ImportAction::Import | ImportAction::SkipSame
            )
        }) {
            let summary = parse_session_file(&candidate.target_path, false).unwrap_or_default();
            thread_metadata.push(thread_metadata_from_manifest(
                &candidate.manifest,
                &candidate.target_path,
                &summary,
            ));
        }
        match upsert_state_threads(&root, &thread_metadata) {
            Ok(updated) => sqlite_updated = updated,
            Err(err) => sqlite_error = Some(err),
        }
    }

    Ok(json!({
        "ok": true,
        "message": format!("导入完成：{} 个导入，{} 个跳过，{} 个冲突", imported, skipped_count, conflicts.len()),
        "report": {
            "path": import_path.to_string_lossy().to_string(),
            "imported": imported,
            "skipped": skipped_count,
            "conflicts": conflicts,
            "errors": errors,
            "sqlite_updated": sqlite_updated,
            "sqlite_error": sqlite_error,
            "index_backup_path": index_backup_path.map(|path| path.to_string_lossy().to_string()),
            "state_backup_path": state_backup_path.map(|path| path.to_string_lossy().to_string())
        }
    }))
}

fn delete_conversations_impl(root: String, relative_paths: Vec<String>) -> Result<Value, String> {
    let root = resolve_codex_root(Some(&root))?;
    validate_codex_root(&root)?;
    if relative_paths.is_empty() {
        return Err("请先选择要删除的会话".to_string());
    }

    let _io_guard = lock_codex_session_io("删除会话")?;
    let deleted_at = now_string();
    let mut errors = Vec::new();
    let mut candidates = Vec::new();
    let mut deleted_records = Vec::new();
    let mut cleanup_ids = Vec::new();
    let mut warnings = Vec::new();
    let index = read_session_index(&root, &mut warnings);

    for relative_path in relative_paths {
        match normalize_relative_path(&relative_path).and_then(|relative| {
            ensure_session_relative_path(&relative)?;
            Ok(relative)
        }) {
            Ok(relative) => {
                let source_path = root.join(&relative);
                if !source_path.exists() {
                    if let Some(id) = extract_uuid_like(&relative_path) {
                        cleanup_ids.push(id);
                    }
                    continue;
                }

                if let Err(err) = validate_session_file_path(&root, &source_path) {
                    errors.push(err);
                    continue;
                }
                let summary = parse_session_file(&source_path, false).unwrap_or_default();
                let id = summary
                    .id
                    .clone()
                    .or_else(|| extract_uuid_like(&relative_path))
                    .unwrap_or_else(|| path_to_slash(&relative));
                let index_entry = index.get(&id);
                let title = index_entry
                    .and_then(|entry| entry.thread_name.clone())
                    .or_else(|| summary.title.clone())
                    .or_else(|| {
                        summary
                            .first_user_message
                            .clone()
                            .map(|text| truncate_text(&text, 48))
                    })
                    .unwrap_or_else(|| "未命名会话".to_string());
                let updated_at = index_entry
                    .and_then(|entry| entry.updated_at.clone())
                    .or_else(|| summary.updated_at.clone())
                    .or_else(|| {
                        source_path
                            .metadata()
                            .ok()
                            .and_then(|metadata| system_time_to_rfc3339(metadata.modified().ok()))
                    });
                candidates.push(DeleteCandidate {
                    id,
                    title,
                    updated_at,
                    source_path,
                    relative_path: relative,
                });
            }
            Err(err) => errors.push(err),
        }
    }

    let mut prepared_candidates = Vec::new();
    for candidate in candidates {
        if candidate.source_path.exists() {
            let original_status = status_from_relative_path(&candidate.relative_path)
                .unwrap_or_else(|_| "active".to_string());
            let summary = parse_session_file(&candidate.source_path, false).unwrap_or_default();
            match save_deleted_session_record(DeletedSessionRecordInput {
                root: &root,
                id: &candidate.id,
                session_path: &candidate.source_path,
                original_relative: &candidate.relative_path,
                deleted_relative: &candidate.relative_path,
                original_status: &original_status,
                summary: &summary,
                title: &candidate.title,
                updated_at: candidate.updated_at.as_deref(),
                deleted_at: &deleted_at,
            }) {
                Ok(record) => {
                    deleted_records.push(record);
                    prepared_candidates.push(candidate);
                }
                Err(err) => errors.push(err),
            }
        }
    }

    let mut deleted = 0usize;
    let mut file_delete_failed_ids = HashSet::new();
    let mut deleted_candidates = Vec::new();
    for candidate in prepared_candidates {
        if candidate.source_path.exists() {
            match fs::remove_file(&candidate.source_path) {
                Ok(_) => {
                    remove_empty_parent_dirs(&root, candidate.source_path.parent());
                    deleted += 1;
                    deleted_candidates.push(candidate);
                }
                Err(err) => {
                    file_delete_failed_ids.insert(candidate.id.clone());
                    errors.push(format!(
                        "删除失败 {}: {err}",
                        candidate.relative_path.display()
                    ));
                }
            }
        } else {
            deleted += 1;
            deleted_candidates.push(candidate);
        }
    }
    if !file_delete_failed_ids.is_empty() {
        let failed_records = deleted_records
            .iter()
            .filter(|record| file_delete_failed_ids.contains(&record.id))
            .cloned()
            .collect::<Vec<_>>();
        remove_deleted_session_records(&failed_records);
    }

    let state_rollout_paths = deleted_candidates
        .iter()
        .map(|candidate| candidate.source_path.clone())
        .collect::<Vec<_>>();
    let mut removed_ids = deleted_candidates
        .iter()
        .map(|candidate| candidate.id.clone())
        .chain(cleanup_ids)
        .collect::<Vec<_>>();
    removed_ids = expand_session_id_variants(&removed_ids);
    dedupe_strings(&mut removed_ids);

    let index_removed = match remove_session_index_ids(&root, &removed_ids) {
        Ok(removed) => removed,
        Err(err) => {
            remove_deleted_session_records(&deleted_records);
            return Err(err);
        }
    };
    let (state_delete, desktop_error) =
        match delete_state_threads_for_sessions(&root, &removed_ids, &state_rollout_paths) {
            Ok(report) => (report, None),
            Err(err) => (StateThreadDeleteReport::default(), Some(err)),
        };
    removed_ids.extend(state_delete.ids.iter().cloned());
    dedupe_strings(&mut removed_ids);
    let (global_state_backup_path, global_state_removed, global_state_error) =
        match remove_from_global_state(&root, &removed_ids, "delete") {
            Ok(Some(cleanup)) => (Some(cleanup.backup_path), cleanup.removed, None),
            Ok(None) => (None, 0, None),
            Err(err) => (None, 0, Some(err)),
        };

    let soft_deleted = deleted_records
        .len()
        .saturating_sub(file_delete_failed_ids.len());

    Ok(json!({
        "ok": errors.is_empty(),
        "message": if errors.is_empty() {
            format!("已删除 {} 个会话", deleted)
        } else {
            format!("已删除 {} 个会话，{} 个失败", deleted, errors.len())
        },
        "report": {
            "deleted": deleted,
            "index_removed": index_removed,
            "sqlite_deleted": state_delete.deleted,
            "state_backup_path": state_delete.backup_path.map(|path| path.to_string_lossy().to_string()),
            "desktop_error": desktop_error,
            "global_state_removed": global_state_removed,
            "global_state_backup_path": global_state_backup_path.map(|path| path.to_string_lossy().to_string()),
            "global_state_error": global_state_error,
            "soft_deleted": soft_deleted,
            "failed": errors.len(),
            "errors": errors,
            "warnings": warnings
        }
    }))
}

fn list_deleted_sessions_impl() -> Result<Value, String> {
    let mut records = read_deleted_session_records()?;
    for record in &mut records {
        if should_rebuild_deleted_title(&record.title) {
            let record_dir = deleted_session_record_dir(&record.delete_id)?;
            let session_file = record_dir.join(&record.session_file);
            if let Ok(summary) = parse_session_file(&session_file, false) {
                record.title = deleted_title_from_summary(&summary);
                record.updated_at = record.updated_at.clone().or(summary.updated_at);
            }
        }
    }
    records.sort_by(|a, b| {
        b.deleted_at
            .cmp(&a.deleted_at)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.delete_id.cmp(&b.delete_id))
    });
    Ok(json!({
        "ok": true,
        "deleted": records
    }))
}

fn restore_deleted_sessions_impl(
    root: String,
    delete_ids: Vec<String>,
    conflict_strategy: Option<String>,
) -> Result<Value, String> {
    let root = resolve_codex_root(Some(&root))?;
    validate_codex_root(&root)?;
    if delete_ids.is_empty() {
        return Err("请先选择要恢复的会话".to_string());
    }
    let conflict_strategy = parse_conflict_strategy(conflict_strategy)?;

    let _io_guard = lock_codex_session_io("恢复会话")?;
    let mut restored = 0usize;
    let mut skipped = 0usize;
    let mut errors = Vec::new();
    let mut conflicts = Vec::new();
    let mut candidates = Vec::new();
    let mut index_updates = Vec::new();
    let mut thread_updates = Vec::new();
    let mut overwritten_ids = Vec::new();

    for delete_id in delete_ids {
        match build_restore_deleted_candidate(&root, &delete_id, conflict_strategy) {
            Ok(Some(candidate)) => candidates.push(candidate),
            Ok(None) => skipped += 1,
            Err(err) => {
                if let Some(conflict) = err.strip_prefix("CONFLICT:") {
                    conflicts.push(json!({
                        "delete_id": delete_id,
                        "target": conflict
                    }));
                } else {
                    errors.push(err);
                }
            }
        }
    }

    if !conflicts.is_empty() && conflict_strategy == ConflictStrategy::Ask {
        return Ok(json!({
            "ok": true,
            "message": format!("发现 {} 个恢复冲突", conflicts.len()),
            "report": {
                "restored": 0,
                "skipped": 0,
                "conflict_action_required": true,
                "operation": "restore",
                "conflicts": conflicts,
                "failed": errors.len(),
                "errors": errors
            }
        }));
    }

    for candidate in candidates {
        if let Some(parent) = candidate.target_path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                errors.push(format!("创建恢复目录失败 {}: {err}", parent.display()));
                continue;
            }
        }
        if candidate.target_path.exists() && conflict_strategy == ConflictStrategy::Overwrite {
            if let Err(err) = fs::remove_file(&candidate.target_path) {
                errors.push(format!(
                    "覆盖恢复目标失败 {}: {err}",
                    candidate.target_path.display()
                ));
                continue;
            }
        }
        let restore_result = if let Some((old_id, new_id)) = &candidate.rewrite_id {
            copy_session_with_new_id(
                &candidate.source_file,
                &candidate.target_path,
                old_id,
                new_id,
            )
        } else {
            fs::copy(&candidate.source_file, &candidate.target_path)
                .map(|_| ())
                .map_err(|err| {
                    format!(
                        "恢复会话失败 {} -> {}: {err}",
                        candidate.source_file.display(),
                        candidate.target_path.display()
                    )
                })
        };
        match restore_result {
            Ok(()) => {
                if let Some(id) = &candidate.overwritten_id {
                    overwritten_ids.push(id.clone());
                }
                let summary = parse_session_file(&candidate.target_path, false).unwrap_or_default();
                let size_bytes = candidate
                    .target_path
                    .metadata()
                    .map(|item| item.len())
                    .unwrap_or(0);
                let sha256 = sha256_file(&candidate.target_path).unwrap_or_default();
                let manifest = ManifestSession {
                    id: candidate.target_id.clone(),
                    title: if should_rebuild_deleted_title(&candidate.record.title) {
                        deleted_title_from_summary(&summary)
                    } else {
                        candidate.record.title.clone()
                    },
                    updated_at: candidate.record.updated_at.clone(),
                    status: candidate.record.original_status.clone(),
                    relative_path: candidate.target_relative.clone(),
                    size_bytes,
                    sha256,
                };
                thread_updates.push(thread_metadata_from_manifest(
                    &manifest,
                    &candidate.target_path,
                    &summary,
                ));
                index_updates.push(manifest);
                let _ = fs::remove_dir_all(candidate.record_dir);
                restored += 1;
            }
            Err(err) => errors.push(err),
        }
    }

    if !index_updates.is_empty() {
        let mut sqlite_updated = 0usize;
        let mut sqlite_error = None;
        if !overwritten_ids.is_empty() {
            let active_ids: HashSet<&str> = index_updates
                .iter()
                .map(|session| session.id.as_str())
                .collect();
            overwritten_ids.retain(|id| !active_ids.contains(id.as_str()));
            dedupe_strings(&mut overwritten_ids);
            let _ = remove_session_index_ids(&root, &overwritten_ids);
            let _ = delete_state_threads_for_sessions(&root, &overwritten_ids, &[]);
            let _ = remove_from_global_state(&root, &overwritten_ids, "restore-overwrite");
        }
        update_session_index_from_manifest(&root, &index_updates)?;
        match upsert_state_threads(&root, &thread_updates) {
            Ok(updated) => sqlite_updated = updated,
            Err(err) => sqlite_error = Some(err),
        }

        Ok(json!({
            "ok": true,
            "message": format!("已恢复 {} 个会话", restored),
            "report": {
                "restored": restored,
                "skipped": skipped,
                "failed": errors.len(),
                "errors": errors,
                "sqlite_updated": sqlite_updated,
                "sqlite_error": sqlite_error
            }
        }))
    } else {
        Ok(json!({
            "ok": true,
            "message": format!("已恢复 {} 个会话", restored),
            "report": {
                "restored": restored,
                "skipped": skipped,
                "failed": errors.len(),
                "errors": errors,
                "sqlite_updated": 0,
                "sqlite_error": null
            }
        }))
    }
}

fn purge_deleted_sessions_impl(delete_ids: Vec<String>) -> Result<Value, String> {
    if delete_ids.is_empty() {
        return Err("请先选择要彻底删除的会话".to_string());
    }
    let mut purged = 0usize;
    let mut errors = Vec::new();
    for delete_id in delete_ids {
        match deleted_session_record_dir(&delete_id).and_then(|dir| {
            if !dir.exists() {
                return Err(format!("已删除会话不存在: {delete_id}"));
            }
            fs::remove_dir_all(&dir).map_err(|err| format!("彻底删除失败 {}: {err}", dir.display()))
        }) {
            Ok(()) => purged += 1,
            Err(err) => errors.push(err),
        }
    }
    Ok(json!({
        "ok": true,
        "message": format!("已彻底删除 {} 个会话", purged),
        "report": {
            "purged": purged,
            "failed": errors.len(),
            "errors": errors
        }
    }))
}

struct DeletedSessionRecordInput<'a> {
    root: &'a Path,
    id: &'a str,
    session_path: &'a Path,
    original_relative: &'a Path,
    deleted_relative: &'a Path,
    original_status: &'a str,
    summary: &'a SessionSummary,
    title: &'a str,
    updated_at: Option<&'a str>,
    deleted_at: &'a str,
}

fn save_deleted_session_record(
    input: DeletedSessionRecordInput<'_>,
) -> Result<DeletedSessionRecord, String> {
    let delete_id = unique_delete_id(input.id);
    let record_dir = deleted_session_record_dir(&delete_id)?;
    fs::create_dir_all(&record_dir)
        .map_err(|err| format!("创建已删除会话目录失败 {}: {err}", record_dir.display()))?;
    let session_file = record_dir.join("session.jsonl");
    fs::copy(input.session_path, &session_file).map_err(|err| {
        format!(
            "备份已删除会话失败 {} -> {}: {err}",
            input.session_path.display(),
            session_file.display()
        )
    })?;
    let size_bytes = input
        .session_path
        .metadata()
        .map(|item| item.len())
        .unwrap_or(0);
    let record = DeletedSessionRecord {
        delete_id,
        id: input.id.to_string(),
        title: if input.title.trim().is_empty() {
            "未命名会话".to_string()
        } else {
            input.title.to_string()
        },
        deleted_at: input.deleted_at.to_string(),
        updated_at: input
            .updated_at
            .map(str::to_string)
            .or_else(|| input.summary.updated_at.clone()),
        original_status: input.original_status.to_string(),
        original_relative_path: path_to_slash(input.original_relative),
        deleted_relative_path: path_to_slash(input.deleted_relative),
        root_path: input.root.to_string_lossy().to_string(),
        size_bytes,
        cwd: input.summary.cwd.clone(),
        session_file: "session.jsonl".to_string(),
    };
    write_deleted_session_record(&record_dir, &record)?;
    Ok(record)
}

fn build_restore_deleted_candidate(
    root: &Path,
    delete_id: &str,
    conflict_strategy: ConflictStrategy,
) -> Result<Option<RestoreCandidate>, String> {
    let record_dir = deleted_session_record_dir(delete_id)?;
    let record = read_deleted_session_record(&record_dir)?;
    let relative = normalize_relative_path(&record.original_relative_path)?;
    ensure_session_relative_path(&relative)?;
    let session_file = record_dir.join(&record.session_file);
    if !session_file.exists() {
        return Err(format!("已删除会话备份文件缺失: {}", record.title));
    }
    let original_target_path = root.join(&relative);
    let mut target_path = original_target_path.clone();
    let mut target_relative = path_to_slash(&relative);
    let mut target_id = record.id.clone();
    let mut rewrite_id = None;
    let mut overwritten_id = None;

    if original_target_path.exists() {
        match conflict_strategy {
            ConflictStrategy::Ask => {
                return Err(format!("CONFLICT:{}", record.original_relative_path));
            }
            ConflictStrategy::Skip => return Ok(None),
            ConflictStrategy::Overwrite => {
                overwritten_id = parse_session_file(&original_target_path, false)
                    .ok()
                    .and_then(|summary| summary.id)
                    .or_else(|| extract_uuid_like(&record.original_relative_path));
            }
            ConflictStrategy::ModifyId => {
                let new_id = new_session_id(&record.id);
                let reassigned = reassigned_relative_path(&relative, &record.id, &new_id)?;
                target_path = root.join(&reassigned);
                target_relative = path_to_slash(&reassigned);
                target_id = new_id.clone();
                rewrite_id = Some((record.id.clone(), new_id));
            }
        }
    }

    while target_path.exists() && conflict_strategy == ConflictStrategy::ModifyId {
        let new_id = new_session_id(&target_id);
        let reassigned = reassigned_relative_path(&relative, &record.id, &new_id)?;
        target_path = root.join(&reassigned);
        target_relative = path_to_slash(&reassigned);
        target_id = new_id.clone();
        rewrite_id = Some((record.id.clone(), new_id));
    }

    Ok(Some(RestoreCandidate {
        record,
        record_dir,
        source_file: session_file,
        target_path,
        target_relative,
        target_id,
        rewrite_id,
        overwritten_id,
    }))
}

fn set_conversation_status_impl(
    root: String,
    relative_paths: Vec<String>,
    status: String,
    conflict_strategy: Option<String>,
) -> Result<Value, String> {
    let root = resolve_codex_root(Some(&root))?;
    validate_codex_root(&root)?;
    let target_status = normalize_status(&status)?;
    let conflict_strategy = parse_conflict_strategy(conflict_strategy)?;
    if relative_paths.is_empty() {
        return Err("请先选择要切换状态的会话".to_string());
    }

    let _io_guard = lock_codex_session_io("切换会话状态")?;
    let mut changed = 0usize;
    let mut skipped = 0usize;
    let mut errors = Vec::new();
    let mut conflicts = Vec::new();
    let mut moves = Vec::new();
    let mut status_index_updates = Vec::new();

    for relative_path in relative_paths {
        let relative = match normalize_relative_path(&relative_path).and_then(|relative| {
            ensure_session_relative_path(&relative)?;
            Ok(relative)
        }) {
            Ok(relative) => relative,
            Err(err) => {
                errors.push(err);
                continue;
            }
        };
        let current_status = match status_from_relative_path(&relative) {
            Ok(status) => status,
            Err(err) => {
                errors.push(err);
                continue;
            }
        };
        let source_path = root.join(&relative);
        if !source_path.exists() {
            errors.push(format!("会话文件不存在: {}", relative.display()));
            continue;
        }
        if let Err(err) = validate_session_file_path(&root, &source_path) {
            errors.push(err);
            continue;
        }
        let summary = parse_session_file(&source_path, false).unwrap_or_default();
        let id = summary
            .id
            .clone()
            .or_else(|| extract_uuid_like(&relative.to_string_lossy()))
            .unwrap_or_else(|| path_to_slash(&relative));
        let Some(file_name) = source_path.file_name().map(|value| value.to_owned()) else {
            errors.push(format!("会话文件名无效: {}", relative.display()));
            continue;
        };
        if current_status == target_status {
            status_index_updates.push(ManifestSession {
                id,
                title: deleted_title_from_summary(&summary),
                updated_at: summary.updated_at.clone(),
                status: target_status.clone(),
                relative_path: path_to_slash(&relative),
                size_bytes: source_path.metadata().map(|item| item.len()).unwrap_or(0),
                sha256: String::new(),
            });
            skipped += 1;
            continue;
        }
        let target_relative = if target_status == "archived" {
            PathBuf::from("archived_sessions").join(file_name)
        } else {
            let (year, month, day) = session_date_parts(&summary, &source_path);
            PathBuf::from("sessions")
                .join(year)
                .join(month)
                .join(day)
                .join(file_name)
        };
        let target_path = root.join(&target_relative);
        let mut target_id = id.clone();
        let mut final_target_path = target_path.clone();
        let mut rewrite_id = None;
        let mut overwritten_id = None;
        if target_path.exists() {
            match conflict_strategy {
                ConflictStrategy::Ask => {
                    conflicts.push(json!({
                        "relative_path": path_to_slash(&relative),
                        "target": path_to_slash(&target_relative),
                        "title": deleted_title_from_summary(&summary)
                    }));
                    continue;
                }
                ConflictStrategy::Skip => {
                    skipped += 1;
                    continue;
                }
                ConflictStrategy::Overwrite => {
                    overwritten_id = parse_session_file(&target_path, false)
                        .ok()
                        .and_then(|summary| summary.id)
                        .or_else(|| extract_uuid_like(&target_relative.to_string_lossy()));
                }
                ConflictStrategy::ModifyId => {
                    let new_id = new_session_id(&id);
                    let reassigned = reassigned_relative_path(&target_relative, &id, &new_id)?;
                    final_target_path = root.join(&reassigned);
                    while final_target_path.exists() {
                        let next_id = new_session_id(&new_id);
                        let next = reassigned_relative_path(&target_relative, &id, &next_id)?;
                        final_target_path = root.join(&next);
                        target_id = next_id.clone();
                        rewrite_id = Some((id.clone(), next_id));
                    }
                    if rewrite_id.is_none() {
                        target_id = new_id.clone();
                        rewrite_id = Some((id.clone(), new_id));
                    }
                }
            }
        }
        moves.push(StatusMove {
            id,
            target_id,
            title: deleted_title_from_summary(&summary),
            updated_at: summary.updated_at.clone(),
            source_path,
            target_path: final_target_path,
            rewrite_id,
            overwritten_id,
        });
    }

    if !conflicts.is_empty() && conflict_strategy == ConflictStrategy::Ask {
        return Ok(json!({
            "ok": true,
            "message": format!("发现 {} 个目标冲突", conflicts.len()),
            "report": {
                "changed": 0,
                "skipped": skipped,
                "conflict_action_required": true,
                "operation": "status",
                "status": target_status,
                "conflicts": conflicts,
                "failed": errors.len(),
                "errors": errors
            }
        }));
    }

    let mut completed_moves = Vec::new();
    let mut overwritten_ids = Vec::new();
    for status_move in &moves {
        let target_relative = status_move
            .target_path
            .strip_prefix(&root)
            .map(path_to_slash)
            .unwrap_or_else(|_| status_move.target_path.to_string_lossy().to_string());
        if let Some(parent) = status_move.target_path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                errors.push(format!("创建目标目录失败 {}: {err}", parent.display()));
                continue;
            }
        }
        if status_move.target_path.exists() && conflict_strategy == ConflictStrategy::Overwrite {
            if let Some(id) = &status_move.overwritten_id {
                overwritten_ids.push(id.clone());
            }
            if let Err(err) = fs::remove_file(&status_move.target_path) {
                errors.push(format!("覆盖目标会话失败 {}: {err}", target_relative));
                continue;
            }
        }
        let move_result = if let Some((old_id, new_id)) = &status_move.rewrite_id {
            copy_session_with_new_id(
                &status_move.source_path,
                &status_move.target_path,
                old_id,
                new_id,
            )
            .and_then(|()| {
                fs::remove_file(&status_move.source_path).map_err(|err| {
                    format!(
                        "删除原会话文件失败 {}: {err}",
                        status_move.source_path.display()
                    )
                })
            })
        } else {
            fs::rename(&status_move.source_path, &status_move.target_path).map_err(|err| {
                format!(
                    "移动会话失败 {} -> {}: {err}",
                    status_move.source_path.display(),
                    target_relative
                )
            })
        };
        if let Err(err) = move_result {
            errors.push(err.to_string());
            continue;
        }
        remove_empty_parent_dirs(&root, status_move.source_path.parent());
        completed_moves.push(status_move.clone());
        changed += 1;
    }

    if !overwritten_ids.is_empty() {
        let active_ids: HashSet<&str> = completed_moves
            .iter()
            .map(|status_move| status_move.target_id.as_str())
            .collect();
        overwritten_ids.retain(|id| !active_ids.contains(id.as_str()));
        dedupe_strings(&mut overwritten_ids);
        let _ = remove_session_index_ids(&root, &overwritten_ids);
        let _ = delete_state_threads_for_sessions(&root, &overwritten_ids, &[]);
        let _ = remove_from_global_state(&root, &overwritten_ids, "status-overwrite");
    }

    let reassign_moves = completed_moves
        .iter()
        .filter(|status_move| status_move.id != status_move.target_id)
        .cloned()
        .collect::<Vec<_>>();
    if !reassign_moves.is_empty() {
        let old_ids = reassign_moves
            .iter()
            .map(|status_move| status_move.id.clone())
            .collect::<Vec<_>>();
        let _ = remove_session_index_ids(&root, &old_ids);
    }

    status_index_updates.extend(completed_moves.iter().map(|status_move| {
        ManifestSession {
            id: status_move.target_id.clone(),
            title: status_move.title.clone(),
            updated_at: status_move.updated_at.clone(),
            status: target_status.clone(),
            relative_path: status_move
                .target_path
                .strip_prefix(&root)
                .map(path_to_slash)
                .unwrap_or_else(|_| status_move.target_path.to_string_lossy().to_string()),
            size_bytes: status_move
                .target_path
                .metadata()
                .map(|item| item.len())
                .unwrap_or(0),
            sha256: String::new(),
        }
    }));
    let (index_backup_path, index_error) = if status_index_updates.is_empty() {
        (None, None)
    } else {
        let backup_path = match backup_session_index(&root) {
            Ok(path) => path,
            Err(err) => {
                errors.push(err.clone());
                None
            }
        };
        let error = update_session_index_from_manifest(&root, &status_index_updates).err();
        (backup_path, error)
    };
    if let Some(err) = &index_error {
        errors.push(err.clone());
    }

    let (state_backup_path, desktop_error) =
        match update_state_thread_status(&root, &completed_moves, &target_status) {
            Ok(backup_path) => (backup_path, None),
            Err(err) => (None, Some(err)),
        };

    Ok(json!({
        "ok": true,
        "message": format!("已切换 {} 个会话状态", changed),
        "report": {
            "changed": changed,
            "skipped": skipped,
            "index_backup_path": index_backup_path.map(|path| path.to_string_lossy().to_string()),
            "index_error": index_error,
            "state_backup_path": state_backup_path.map(|path| path.to_string_lossy().to_string()),
            "desktop_error": desktop_error,
            "conflicts": conflicts,
            "failed": errors.len(),
            "errors": errors
        }
    }))
}

fn update_conversation_cwd_impl(
    root: String,
    relative_paths: Vec<String>,
    cwd: String,
) -> Result<Value, String> {
    let root = resolve_codex_root(Some(&root))?;
    validate_codex_root(&root)?;
    let cwd = cwd.trim().to_string();
    if cwd.is_empty() {
        return Err("工作目录不能为空".to_string());
    }
    if relative_paths.is_empty() {
        return Err("请先选择要修改工作目录的会话".to_string());
    }

    let _io_guard = lock_codex_session_io("修改工作目录")?;
    let mut updated = 0usize;
    let mut errors = Vec::new();
    let mut cwd_updates = Vec::new();

    for relative_path in relative_paths {
        let relative = match normalize_relative_path(&relative_path).and_then(|relative| {
            ensure_session_relative_path(&relative)?;
            Ok(relative)
        }) {
            Ok(relative) => relative,
            Err(err) => {
                errors.push(err);
                continue;
            }
        };
        let path = root.join(&relative);
        if !path.exists() {
            errors.push(format!("会话文件不存在: {}", relative.display()));
            continue;
        }
        let summary = parse_session_file(&path, false).unwrap_or_default();
        match rewrite_session_cwd(&path, &cwd) {
            Ok(true) => {
                updated += 1;
                if let Some(id) = summary
                    .id
                    .or_else(|| extract_uuid_like(&relative.to_string_lossy()))
                {
                    cwd_updates.push((id, cwd.clone()));
                }
            }
            Ok(false) => {
                if let Some(id) = summary
                    .id
                    .or_else(|| extract_uuid_like(&relative.to_string_lossy()))
                {
                    cwd_updates.push((id, cwd.clone()));
                }
            }
            Err(err) => errors.push(err),
        }
    }

    if !cwd_updates.is_empty() {
        update_state_thread_cwds(&root, &cwd_updates)?;
    }

    Ok(json!({
        "ok": true,
        "message": format!("已修改 {} 个会话的工作目录", updated),
        "report": {
            "updated": updated,
            "failed": errors.len(),
            "errors": errors
        }
    }))
}

fn resolve_codex_root(root: Option<&str>) -> Result<PathBuf, String> {
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

fn find_session_relative_paths_by_id(root: &Path, session_id: &str) -> Result<Vec<String>, String> {
    let variants = session_id_variants(session_id);
    let mut relative_paths = Vec::new();
    let state_db = root.join("state_5.sqlite");
    if state_db.exists() {
        let connection = Connection::open_with_flags(
            &state_db,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|err| format!("打开 Codex state 数据库失败 {}: {err}", state_db.display()))?;
        connection
            .busy_timeout(Duration::from_millis(3000))
            .map_err(|err| format!("配置 Codex state 数据库等待超时失败: {err}"))?;
        if state_threads_has_columns(&connection, &["id", "rollout_path"])? {
            for variant in &variants {
                let mut statement = connection
                    .prepare("SELECT rollout_path FROM threads WHERE id = ?1")
                    .map_err(|err| format!("查询 Codex Desktop threads 索引失败: {err}"))?;
                let rows = statement
                    .query_map([variant], |row| row.get::<_, String>(0))
                    .map_err(|err| format!("查询 Codex Desktop threads 索引失败: {err}"))?;
                for row in rows {
                    let rollout_path =
                        row.map_err(|err| format!("读取 Codex Desktop rollout_path 失败: {err}"))?;
                    if let Some(relative) = rollout_path_to_relative(root, &rollout_path) {
                        relative_paths.push(relative);
                    }
                }
            }
        }
    }

    if relative_paths.is_empty() {
        let variant_set: HashSet<&str> = variants.iter().map(String::as_str).collect();
        let mut files = Vec::new();
        let mut errors = Vec::new();
        collect_conversation_files(&root.join("sessions"), "active", &mut files, &mut errors);
        collect_conversation_files(
            &root.join("archived_sessions"),
            "archived",
            &mut files,
            &mut errors,
        );
        for (_status, path) in files {
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let path_id = extract_uuid_like(&path_to_slash(relative));
            let summary_id = parse_session_file(&path, true)
                .ok()
                .and_then(|summary| summary.id);
            let matches = summary_id
                .as_deref()
                .is_some_and(|id| variant_set.contains(id))
                || path_id
                    .as_deref()
                    .is_some_and(|id| variant_set.contains(id));
            if matches {
                relative_paths.push(path_to_slash(relative));
            }
        }
    }

    dedupe_strings(&mut relative_paths);
    Ok(relative_paths)
}

fn session_id_variants(session_id: &str) -> Vec<String> {
    let raw = session_id.trim();
    let bare = raw.strip_prefix("local:").unwrap_or(raw);
    let mut variants = vec![raw.to_string(), bare.to_string()];
    if !bare.is_empty() {
        variants.push(format!("local:{bare}"));
    }
    dedupe_strings(&mut variants);
    variants
}

fn expand_session_id_variants(ids: &[String]) -> Vec<String> {
    let mut variants = ids
        .iter()
        .flat_map(|id| session_id_variants(id))
        .collect::<Vec<_>>();
    dedupe_strings(&mut variants);
    variants
}

fn rollout_path_to_relative(root: &Path, rollout_path: &str) -> Option<String> {
    let path = PathBuf::from(rollout_path);
    let relative = if path.is_absolute() {
        path.strip_prefix(root).ok()?.to_path_buf()
    } else {
        path
    };
    ensure_session_relative_path(&relative).ok()?;
    Some(path_to_slash(relative))
}

fn validate_codex_root(root: &Path) -> Result<(), String> {
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

fn validate_session_file_path(root: &Path, path: &Path) -> Result<(), String> {
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

fn read_session_index(root: &Path, warnings: &mut Vec<String>) -> HashMap<String, IndexEntry> {
    let path = root.join("session_index.jsonl");
    let mut map = HashMap::new();
    if !path.exists() {
        warnings.push("session_index.jsonl 不存在，已从会话文件推断标题和更新时间".to_string());
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
        map.insert(
            id,
            IndexEntry {
                thread_name,
                updated_at,
            },
        );
    }
    map
}

fn collect_conversation_files(
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

fn conversation_from_path(
    root: &Path,
    path: &Path,
    status: &str,
    index: &HashMap<String, IndexEntry>,
    include_sha: bool,
) -> Result<ConversationItem, String> {
    let metadata = fs::metadata(path)
        .map_err(|err| format!("读取会话文件信息失败 {}: {err}", path.display()))?;
    let summary = parse_session_file_for_list(path).unwrap_or_else(|err| SessionSummary {
        parse_error: Some(err),
        ..SessionSummary::default()
    });
    let relative_path = path
        .strip_prefix(root)
        .map(path_to_slash)
        .unwrap_or_else(|_| path.to_string_lossy().to_string());
    let id = summary
        .id
        .clone()
        .or_else(|| extract_uuid_like(&relative_path))
        .unwrap_or_else(|| relative_path.clone());
    let index_entry = index.get(&id);
    let title = index_entry
        .and_then(|entry| entry.thread_name.clone())
        .or_else(|| summary.title.clone())
        .or_else(|| {
            summary
                .first_user_message
                .clone()
                .map(|text| truncate_text(&text, 48))
        })
        .unwrap_or_else(|| "未命名会话".to_string());
    let updated_at = index_entry
        .and_then(|entry| entry.updated_at.clone())
        .or_else(|| summary.updated_at.clone())
        .or_else(|| system_time_to_rfc3339(metadata.modified().ok()));
    let sha256 = if include_sha {
        Some(sha256_file(path)?)
    } else {
        None
    };

    Ok(ConversationItem {
        id,
        title,
        updated_at,
        status: status.to_string(),
        source_path: path.to_string_lossy().to_string(),
        relative_path,
        size_bytes: metadata.len(),
        cwd: summary.cwd,
        preview: summary.preview,
        sha256,
        parse_error: summary.parse_error,
    })
}

fn append_missing_state_thread_conversations(
    root: &Path,
    conversations: &mut Vec<ConversationItem>,
    warnings: &mut Vec<String>,
) {
    let state_db = root.join("state_5.sqlite");
    if !state_db.exists() {
        return;
    }

    let seen_ids: HashSet<String> = conversations.iter().map(|item| item.id.clone()).collect();
    let Ok(connection) = Connection::open_with_flags(
        &state_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        warnings.push("读取 Codex Desktop 会话索引失败，已跳过残留项检查".to_string());
        return;
    };
    let Ok(mut statement) = connection.prepare(
        "SELECT id, rollout_path, updated_at, title, cwd, archived
         FROM threads
         ORDER BY updated_at DESC",
    ) else {
        return;
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, i64>(5)?,
        ))
    }) else {
        return;
    };

    let mut missing_count = 0usize;
    for row in rows.flatten() {
        let (id, rollout_path, updated_at, title, cwd, archived) = row;
        if seen_ids.contains(&id) {
            continue;
        }
        let path = PathBuf::from(&rollout_path);
        if !path.is_absolute() || !path.starts_with(root) || path.exists() {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative_path = path_to_slash(relative);
        if ensure_session_relative_path(relative).is_err() {
            continue;
        }
        conversations.push(ConversationItem {
            id,
            title: if title.trim().is_empty() {
                "缺失的会话文件".to_string()
            } else {
                title
            },
            updated_at: timestamp_seconds_to_rfc3339(updated_at),
            status: if archived == 0 {
                "active".to_string()
            } else {
                "archived".to_string()
            },
            source_path: rollout_path,
            relative_path,
            size_bytes: 0,
            cwd: non_empty(cwd),
            preview: None,
            sha256: None,
            parse_error: Some("会话文件已不存在，仅保留 Codex Desktop 索引".to_string()),
        });
        missing_count += 1;
    }
    if missing_count > 0 {
        warnings.push(format!(
            "发现 {missing_count} 条 Codex Desktop 残留索引，可选中后删除清理"
        ));
    }
}

fn warn_archived_session_index_entries(
    conversations: &[ConversationItem],
    index: &HashMap<String, IndexEntry>,
    warnings: &mut Vec<String>,
) {
    let archived_count = conversations
        .iter()
        .filter(|item| item.status == "archived")
        .filter(|item| index.contains_key(&item.id))
        .count();
    if archived_count > 0 {
        warnings.push(format!(
            "发现 {archived_count} 条已归档会话的 Codex 索引残留，可选中后删除清理"
        ));
    }
}

fn parse_session_file_for_list(path: &Path) -> Result<SessionSummary, String> {
    parse_session_file_with_limit(path, false, Some(240))
}

fn parse_session_file(path: &Path, include_messages: bool) -> Result<SessionSummary, String> {
    parse_session_file_with_limit(path, include_messages, None)
}

fn parse_session_file_with_limit(
    path: &Path,
    include_messages: bool,
    max_lines: Option<usize>,
) -> Result<SessionSummary, String> {
    let file = fs::File::open(path)
        .map_err(|err| format!("读取会话文件失败 {}: {err}", path.display()))?;
    let reader = BufReader::new(file);
    let mut summary = SessionSummary::default();
    let mut valid_lines = 0usize;
    let mut event_messages = Vec::new();
    let mut fallback_messages = Vec::new();

    for (line_index, line) in reader.lines().enumerate() {
        if max_lines.is_some_and(|limit| line_index >= limit) {
            break;
        }
        let line = line.map_err(|err| format!("读取会话文件失败 {}: {err}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        valid_lines += 1;
        let timestamp = non_empty(raw_string_field(&value, "timestamp"));
        update_summary_times(&mut summary, timestamp.as_deref());
        let event_type = raw_string_field(&value, "type");
        let payload = value.get("payload").unwrap_or(&Value::Null);

        if event_type == "session_meta" {
            set_first(&mut summary.id, non_empty(raw_string_field(payload, "id")));
            set_first(
                &mut summary.cwd,
                non_empty(raw_string_field(payload, "cwd")),
            );
            set_first(
                &mut summary.model_provider,
                non_empty(raw_string_field(payload, "model_provider")),
            );
            continue;
        }

        if event_type == "turn_context" {
            set_first(
                &mut summary.cwd,
                non_empty(raw_string_field(payload, "cwd")),
            );
            continue;
        }

        let payload_type = raw_string_field(payload, "type");
        if event_type == "event_msg" {
            if payload_type == "task_started" {
                set_first(
                    &mut summary.cwd,
                    non_empty(raw_string_field(payload, "cwd")),
                );
                set_first(
                    &mut summary.model,
                    non_empty(raw_string_field(payload, "model")),
                );
                set_first(
                    &mut summary.reasoning_effort,
                    non_empty(raw_string_field(payload, "effort")),
                );
                set_first(
                    &mut summary.approval_mode,
                    non_empty(raw_string_field(payload, "approval_policy")),
                );
                if summary.sandbox_policy.is_none() {
                    if let Some(policy) = payload.get("sandbox_policy") {
                        summary.sandbox_policy = serde_json::to_string(policy).ok();
                    }
                }
            } else if payload_type == "thread_name_updated" {
                set_first(
                    &mut summary.title,
                    first_non_empty(&[
                        raw_string_field(payload, "thread_name"),
                        raw_string_field(payload, "title"),
                        raw_string_field(payload, "name"),
                    ]),
                );
            } else if payload_type == "user_message" {
                if let Some(text) = readable_payload_text(payload) {
                    push_readable_message(
                        &mut event_messages,
                        "user",
                        text,
                        timestamp.clone(),
                        include_messages,
                    );
                }
            } else if payload_type == "agent_message" {
                if let Some(text) = readable_payload_text(payload) {
                    push_readable_message(
                        &mut event_messages,
                        "assistant",
                        text,
                        timestamp.clone(),
                        include_messages,
                    );
                }
            }
        } else if event_type == "response_item" && payload_type == "message" {
            let role = raw_string_field(payload, "role");
            if role == "user" || role == "assistant" {
                if let Some(text) = readable_payload_text(payload) {
                    push_readable_message(
                        &mut fallback_messages,
                        &role,
                        text,
                        timestamp.clone(),
                        include_messages,
                    );
                }
            }
        }
    }

    if valid_lines == 0 {
        summary.parse_error = Some("没有识别到有效 JSONL 事件".to_string());
    }

    let chosen = if event_messages.is_empty() {
        fallback_messages
    } else {
        event_messages
    };
    for message in &chosen {
        if message.role == "user" && summary.first_user_message.is_none() {
            summary.first_user_message = Some(message.text.clone());
        }
        if summary.preview.is_none() {
            summary.preview = Some(truncate_text(&message.text, 120));
        }
    }
    summary.messages = chosen;
    Ok(summary)
}

fn readable_payload_text(payload: &Value) -> Option<String> {
    let message = raw_string_field(payload, "message");
    if !message.trim().is_empty() {
        return Some(message.trim().to_string());
    }

    if let Some(text_elements) = payload.get("text_elements").and_then(Value::as_array) {
        let text = text_elements
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            return Some(text);
        }
    }

    match payload.get("content") {
        Some(Value::String(text)) if !text.trim().is_empty() => Some(text.trim().to_string()),
        Some(Value::Array(items)) => {
            let text = items
                .iter()
                .filter_map(|item| {
                    item.get("text")
                        .and_then(Value::as_str)
                        .or_else(|| item.as_str())
                })
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        _ => None,
    }
}

fn push_readable_message(
    messages: &mut Vec<ConversationMessage>,
    role: &str,
    text: String,
    timestamp: Option<String>,
    include_messages: bool,
) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    if include_messages || messages.is_empty() {
        messages.push(ConversationMessage {
            role: role.to_string(),
            text,
            timestamp,
        });
    }
}

fn update_summary_times(summary: &mut SessionSummary, timestamp: Option<&str>) {
    let Some(timestamp) = timestamp else {
        return;
    };
    if parse_rfc3339_seconds(timestamp).is_none() {
        return;
    }
    if summary.created_at.is_none() {
        summary.created_at = Some(timestamp.to_string());
    }
    if summary
        .updated_at
        .as_deref()
        .and_then(parse_rfc3339_seconds)
        .is_none_or(|current| parse_rfc3339_seconds(timestamp).unwrap_or(current) >= current)
    {
        summary.updated_at = Some(timestamp.to_string());
    }
}

fn build_import_candidate(
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
    let action = if target_path.exists() {
        let current_sha = sha256_file(&target_path)?;
        if current_sha == session.sha256 {
            ImportAction::SkipSame
        } else {
            ImportAction::Conflict
        }
    } else {
        ImportAction::Import
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

fn update_session_index_from_manifest(
    root: &Path,
    sessions: &[ManifestSession],
) -> Result<(), String> {
    let path = root.join("session_index.jsonl");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建索引目录失败: {err}"))?;
    }
    let mut update_ids = HashSet::new();
    for session in sessions {
        update_ids.insert(session.id.clone());
    }
    let mut lines = Vec::new();
    if path.exists() {
        let content = fs::read_to_string(&path)
            .map_err(|err| format!("读取 session_index.jsonl 失败: {err}"))?;
        for line in content.lines() {
            let keep = serde_json::from_str::<Value>(line)
                .ok()
                .map(|value| raw_string_field(&value, "id"))
                .filter(|id| !id.is_empty())
                .is_none_or(|id| !update_ids.contains(&id));
            if keep {
                lines.push(line.to_string());
            }
        }
    }
    for session in sessions {
        if session.status != "active" {
            continue;
        }
        lines.push(
            json!({
                "id": session.id,
                "thread_name": session.title,
                "updated_at": session.updated_at
            })
            .to_string(),
        );
    }
    fs::write(&path, format!("{}\n", lines.join("\n")))
        .map_err(|err| format!("写入 session_index.jsonl 失败: {err}"))
}

fn remove_session_index_ids(root: &Path, ids: &[String]) -> Result<usize, String> {
    remove_session_index_ids_with_reason(root, ids, "delete")
}

fn remove_session_index_ids_with_reason(
    root: &Path,
    ids: &[String],
    reason: &str,
) -> Result<usize, String> {
    let path = root.join("session_index.jsonl");
    if ids.is_empty() || !path.exists() {
        return Ok(0);
    }
    let id_set: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let content =
        fs::read_to_string(&path).map_err(|err| format!("读取 session_index.jsonl 失败: {err}"))?;
    let mut lines = Vec::new();
    let mut removed = 0usize;
    for line in content.lines() {
        let remove = serde_json::from_str::<Value>(line)
            .ok()
            .map(|value| raw_string_field(&value, "id"))
            .is_some_and(|id| id_set.contains(id.as_str()));
        if !remove {
            lines.push(line.to_string());
        } else {
            removed += 1;
        }
    }
    if removed == 0 {
        return Ok(0);
    }
    let _backup_path = backup_file_with_reason(&path, reason)?;
    fs::write(
        &path,
        if lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", lines.join("\n"))
        },
    )
    .map_err(|err| format!("写入 session_index.jsonl 失败: {err}"))?;
    Ok(removed)
}

#[derive(Debug)]
struct GlobalStateCleanup {
    backup_path: PathBuf,
    removed: usize,
}

fn remove_from_global_state(
    root: &Path,
    ids: &[String],
    reason: &str,
) -> Result<Option<GlobalStateCleanup>, String> {
    if ids.is_empty() {
        return Ok(None);
    }
    let path = root.join(".codex-global-state.json");
    if !path.exists() {
        return Ok(None);
    }
    let id_set: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("读取 .codex-global-state.json 失败: {err}"))?;
    let mut value: Value = serde_json::from_str(&content)
        .map_err(|err| format!("解析 .codex-global-state.json 失败: {err}"))?;
    let removed = remove_matching_object_keys(&mut value, &id_set);
    if removed == 0 {
        return Ok(None);
    }
    let backup_path = backup_file_with_reason(&path, reason)?;
    let mut output = serde_json::to_string_pretty(&value)
        .map_err(|err| format!("序列化 .codex-global-state.json 失败: {err}"))?;
    output.push('\n');
    fs::write(&path, output).map_err(|err| format!("写入 .codex-global-state.json 失败: {err}"))?;
    Ok(Some(GlobalStateCleanup {
        backup_path,
        removed,
    }))
}

fn remove_matching_object_keys(value: &mut Value, ids: &HashSet<&str>) -> usize {
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
            for value in map.values_mut() {
                removed += remove_matching_object_keys(value, ids);
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

fn read_deleted_session_records() -> Result<Vec<DeletedSessionRecord>, String> {
    let dir = deleted_sessions_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(&dir)
        .map_err(|err| format!("读取已删除会话目录失败 {}: {err}", dir.display()))?;
    let mut records = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        if let Ok(record) = read_deleted_session_record(&entry.path()) {
            records.push(record);
        }
    }
    Ok(records)
}

fn read_deleted_session_record(record_dir: &Path) -> Result<DeletedSessionRecord, String> {
    let path = record_dir.join("metadata.json");
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("读取已删除会话元数据失败 {}: {err}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|err| format!("解析已删除会话元数据失败 {}: {err}", path.display()))
}

fn write_deleted_session_record(
    record_dir: &Path,
    record: &DeletedSessionRecord,
) -> Result<(), String> {
    let path = record_dir.join("metadata.json");
    let mut content = serde_json::to_string_pretty(record)
        .map_err(|err| format!("序列化已删除会话元数据失败: {err}"))?;
    content.push('\n');
    fs::write(&path, content)
        .map_err(|err| format!("写入已删除会话元数据失败 {}: {err}", path.display()))
}

fn remove_deleted_session_records(records: &[DeletedSessionRecord]) {
    for record in records {
        if let Ok(dir) = deleted_session_record_dir(&record.delete_id) {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

fn should_rebuild_deleted_title(title: &str) -> bool {
    title.trim().is_empty() || title == "未命名会话"
}

fn deleted_title_from_summary(summary: &SessionSummary) -> String {
    summary
        .title
        .clone()
        .or_else(|| summary.first_user_message.clone())
        .map(|value| truncate_text(&value, 80))
        .unwrap_or_else(|| "未命名会话".to_string())
}

fn deleted_sessions_dir() -> Result<PathBuf, String> {
    Ok(session_manager_data_dir()?.join(DELETED_SESSIONS_DIR))
}

fn deleted_session_record_dir(delete_id: &str) -> Result<PathBuf, String> {
    validate_delete_id(delete_id)?;
    Ok(deleted_sessions_dir()?.join(delete_id))
}

fn session_manager_data_dir() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join(SESSION_MANAGER_DATA_DIR))
}

fn session_manager_backup_dir(reason: &str) -> Result<PathBuf, String> {
    let reason = sanitize_backup_reason(reason);
    Ok(session_manager_data_dir()?.join("backups").join(reason))
}

fn unique_delete_id(id: &str) -> String {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{}-{}-{suffix}", backup_stamp(), sanitize_id_fragment(id))
}

fn sanitize_id_fragment(id: &str) -> String {
    let value = id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .take(80)
        .collect::<String>();
    if value.is_empty() {
        "session".to_string()
    } else {
        value
    }
}

fn sanitize_backup_reason(reason: &str) -> String {
    let value = reason
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .collect::<String>();
    if value.is_empty() {
        "general".to_string()
    } else {
        value
    }
}

fn dedupe_strings(items: &mut Vec<String>) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.clone()));
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

fn backup_session_index(root: &Path) -> Result<Option<PathBuf>, String> {
    let path = root.join("session_index.jsonl");
    if path.exists() {
        backup_file(&path).map(Some)
    } else {
        Ok(None)
    }
}

fn backup_file(path: &Path) -> Result<PathBuf, String> {
    backup_file_with_reason(path, "")
}

fn backup_file_with_reason(path: &Path, reason: &str) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("备份文件名无效: {}", path.display()))?;
    let reason = reason.trim();
    let backup = if reason.is_empty() {
        let base_name = format!("{file_name}.bak.context-manager-{}", backup_stamp());
        unique_sibling_path(path, &base_name)
    } else {
        let reason = sanitize_backup_reason(reason);
        let base_name = format!(
            "{file_name}.bak.context-manager-{reason}-{}",
            backup_stamp()
        );
        let backup_dir = session_manager_backup_dir(&reason)?;
        fs::create_dir_all(&backup_dir)
            .map_err(|err| format!("创建备份目录失败 {}: {err}", backup_dir.display()))?;
        unique_sibling_path(&backup_dir.join(&base_name), &base_name)
    };
    fs::copy(path, &backup).map_err(|err| {
        format!(
            "备份文件失败 {} -> {}: {err}",
            path.display(),
            backup.display()
        )
    })?;
    Ok(backup)
}

fn backup_state_database_for_delete(
    connection: &Connection,
    _root: &Path,
) -> Result<PathBuf, String> {
    backup_state_database_with_reason(connection, "delete")
}

fn backup_state_database_for_status(
    connection: &Connection,
    _root: &Path,
) -> Result<PathBuf, String> {
    backup_state_database_with_reason(connection, "status")
}

fn backup_state_database_with_reason(
    connection: &Connection,
    reason: &str,
) -> Result<PathBuf, String> {
    let reason = sanitize_backup_reason(reason);
    let backup_dir = session_manager_backup_dir(&reason)?;
    fs::create_dir_all(&backup_dir)
        .map_err(|err| format!("创建备份目录失败 {}: {err}", backup_dir.display()))?;
    let base_name = format!(
        "state_5.sqlite.bak.context-manager-{reason}-{}",
        backup_stamp()
    );
    let backup = unique_sibling_path(&backup_dir.join(&base_name), &base_name);
    let backup_literal = sqlite_string_literal(&backup);
    connection
        .execute_batch(&format!("VACUUM main INTO {backup_literal};"))
        .map_err(|err| format!("备份 state_5.sqlite 失败 {}: {err}", backup.display()))?;
    Ok(backup)
}

fn sqlite_string_literal(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

fn unique_sibling_path(path: &Path, base_name: &str) -> PathBuf {
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

fn remove_empty_parent_dirs(root: &Path, parent: Option<&Path>) {
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

fn thread_metadata_from_manifest(
    session: &ManifestSession,
    target_path: &Path,
    summary: &SessionSummary,
) -> ThreadMetadata {
    let updated_at = session
        .updated_at
        .as_deref()
        .and_then(parse_rfc3339_seconds)
        .or_else(|| {
            summary
                .updated_at
                .as_deref()
                .and_then(parse_rfc3339_seconds)
        })
        .unwrap_or_else(now_unix_seconds);
    let created_at = summary
        .created_at
        .as_deref()
        .and_then(parse_rfc3339_seconds)
        .unwrap_or(updated_at);
    ThreadMetadata {
        id: session.id.clone(),
        rollout_path: target_path.to_path_buf(),
        created_at,
        updated_at,
        source: "cli".to_string(),
        model_provider: summary
            .model_provider
            .clone()
            .unwrap_or_else(|| "openai".to_string()),
        cwd: summary.cwd.clone().unwrap_or_default(),
        title: if session.title.trim().is_empty() {
            "未命名会话".to_string()
        } else {
            session.title.clone()
        },
        sandbox_policy: summary
            .sandbox_policy
            .clone()
            .unwrap_or_else(|| "{\"type\":\"workspace-write\"}".to_string()),
        approval_mode: summary
            .approval_mode
            .clone()
            .unwrap_or_else(|| "on-request".to_string()),
        has_user_event: i64::from(summary.first_user_message.is_some()),
        archived: i64::from(session.status == "archived"),
        archived_at: if session.status == "archived" {
            Some(updated_at)
        } else {
            None
        },
        first_user_message: summary.first_user_message.clone().unwrap_or_default(),
        model: summary.model.clone(),
        reasoning_effort: summary.reasoning_effort.clone(),
    }
}

fn upsert_state_threads(root: &Path, items: &[ThreadMetadata]) -> Result<usize, String> {
    if items.is_empty() {
        return Ok(0);
    }
    let state_db = root.join("state_5.sqlite");
    if !state_db.exists() {
        return Ok(0);
    }
    let mut connection = Connection::open_with_flags(
        &state_db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| format!("打开 Codex state 数据库失败 {}: {err}", state_db.display()))?;
    connection
        .busy_timeout(Duration::from_millis(3000))
        .map_err(|err| format!("配置 Codex state 数据库等待超时失败: {err}"))?;
    let Some(schema) = state_threads_schema(&connection)? else {
        return Ok(0);
    };
    let available_columns = schema.keys().cloned().collect::<HashSet<_>>();
    if !available_columns.contains("id") || !available_columns.contains("rollout_path") {
        return Ok(0);
    }
    let unsupported_required_columns = schema
        .values()
        .filter(|column| {
            column.not_null
                && !column.primary_key
                && column.default_value.is_none()
                && !thread_metadata_supported_column(&column.name)
        })
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    if !unsupported_required_columns.is_empty() {
        return Ok(0);
    }

    let insert_columns = THREAD_METADATA_COLUMNS
        .iter()
        .filter(|column| available_columns.contains(**column))
        .copied()
        .collect::<Vec<_>>();
    if !insert_columns.contains(&"id") || !insert_columns.contains(&"rollout_path") {
        return Ok(0);
    }
    let placeholders = (1..=insert_columns.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let update_columns = THREAD_METADATA_UPDATE_COLUMNS
        .iter()
        .filter(|column| insert_columns.contains(column))
        .copied()
        .collect::<Vec<_>>();
    let update_clause = if update_columns.is_empty() {
        "DO NOTHING".to_string()
    } else {
        format!(
            "DO UPDATE SET {}",
            update_columns
                .iter()
                .map(|column| format!("{column} = excluded.{column}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let sql = format!(
        "INSERT INTO threads ({}) VALUES ({}) ON CONFLICT(id) {}",
        insert_columns.join(", "),
        placeholders,
        update_clause
    );
    let transaction = connection
        .transaction()
        .map_err(|err| format!("开始 Codex state 索引事务失败: {err}"))?;
    let mut updated = 0usize;
    for item in items {
        let values = insert_columns
            .iter()
            .map(|column| thread_metadata_sql_value(item, column))
            .collect::<Vec<_>>();
        updated += transaction
            .execute(&sql, params_from_iter(values.iter()))
            .map_err(|err| format!("更新 Codex Desktop threads 索引失败: {err}"))?;
    }
    transaction
        .commit()
        .map_err(|err| format!("保存 Codex Desktop threads 索引失败: {err}"))?;
    Ok(updated)
}

const THREAD_METADATA_COLUMNS: &[&str] = &[
    "id",
    "rollout_path",
    "created_at",
    "updated_at",
    "source",
    "model_provider",
    "cwd",
    "title",
    "sandbox_policy",
    "approval_mode",
    "tokens_used",
    "has_user_event",
    "archived",
    "archived_at",
    "cli_version",
    "first_user_message",
    "memory_mode",
    "model",
    "reasoning_effort",
    "created_at_ms",
    "updated_at_ms",
    "thread_source",
];

const THREAD_METADATA_UPDATE_COLUMNS: &[&str] = &[
    "rollout_path",
    "updated_at",
    "model_provider",
    "cwd",
    "title",
    "archived",
    "archived_at",
    "first_user_message",
    "model",
    "reasoning_effort",
    "updated_at_ms",
    "thread_source",
];

#[derive(Debug)]
struct StateThreadColumn {
    name: String,
    not_null: bool,
    default_value: Option<String>,
    primary_key: bool,
}

fn thread_metadata_supported_column(column: &str) -> bool {
    THREAD_METADATA_COLUMNS.contains(&column)
}

fn thread_metadata_sql_value(item: &ThreadMetadata, column: &str) -> rusqlite::types::Value {
    use rusqlite::types::Value as SqlValue;

    match column {
        "id" => SqlValue::Text(item.id.clone()),
        "rollout_path" => SqlValue::Text(item.rollout_path.to_string_lossy().to_string()),
        "created_at" => SqlValue::Integer(item.created_at),
        "updated_at" => SqlValue::Integer(item.updated_at),
        "source" => SqlValue::Text(item.source.clone()),
        "model_provider" => SqlValue::Text(item.model_provider.clone()),
        "cwd" => SqlValue::Text(item.cwd.clone()),
        "title" => SqlValue::Text(item.title.clone()),
        "sandbox_policy" => SqlValue::Text(item.sandbox_policy.clone()),
        "approval_mode" => SqlValue::Text(item.approval_mode.clone()),
        "tokens_used" => SqlValue::Integer(0),
        "has_user_event" => SqlValue::Integer(item.has_user_event),
        "archived" => SqlValue::Integer(item.archived),
        "archived_at" => item
            .archived_at
            .map(SqlValue::Integer)
            .unwrap_or(SqlValue::Null),
        "cli_version" => SqlValue::Text(String::new()),
        "first_user_message" => SqlValue::Text(item.first_user_message.clone()),
        "memory_mode" => SqlValue::Text("enabled".to_string()),
        "model" => item
            .model
            .clone()
            .map(SqlValue::Text)
            .unwrap_or(SqlValue::Null),
        "reasoning_effort" => item
            .reasoning_effort
            .clone()
            .map(SqlValue::Text)
            .unwrap_or(SqlValue::Null),
        "created_at_ms" => SqlValue::Integer(item.created_at.saturating_mul(1000)),
        "updated_at_ms" => SqlValue::Integer(item.updated_at.saturating_mul(1000)),
        "thread_source" => SqlValue::Text("local".to_string()),
        _ => SqlValue::Null,
    }
}

#[derive(Debug, Default)]
struct StateThreadDeleteReport {
    deleted: usize,
    ids: Vec<String>,
    backup_path: Option<PathBuf>,
}

fn update_state_thread_status(
    root: &Path,
    moves: &[StatusMove],
    target_status: &str,
) -> Result<Option<PathBuf>, String> {
    if moves.is_empty() {
        return Ok(None);
    }
    let state_db = root.join("state_5.sqlite");
    if !state_db.exists() {
        return Ok(None);
    }
    let mut connection = Connection::open_with_flags(
        &state_db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| format!("打开 Codex state 数据库失败 {}: {err}", state_db.display()))?;
    connection
        .busy_timeout(Duration::from_millis(3000))
        .map_err(|err| format!("配置 Codex state 数据库等待超时失败: {err}"))?;

    if !state_threads_has_columns(
        &connection,
        &["id", "archived", "archived_at", "rollout_path"],
    )? {
        return Ok(None);
    }

    let backup_path = backup_state_database_for_status(&connection, root)?;
    let archived = target_status == "archived";
    let archived_value = i64::from(archived);
    let archived_at = archived.then(now_unix_seconds);
    let transaction = connection
        .transaction()
        .map_err(|err| format!("开始 Codex state 状态更新事务失败: {err}"))?;
    for status_move in moves {
        let rollout_path = status_move.target_path.to_string_lossy().to_string();
        transaction
            .execute(
                "UPDATE threads SET id = ?1, archived = ?2, archived_at = ?3, rollout_path = ?4 WHERE id = ?5",
                params![
                    status_move.target_id,
                    archived_value,
                    archived_at,
                    rollout_path,
                    status_move.id
                ],
            )
            .map_err(|err| format!("更新 Codex Desktop threads 状态失败: {err}"))?;
    }
    transaction
        .commit()
        .map_err(|err| format!("保存 Codex Desktop threads 状态失败: {err}"))?;
    let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    Ok(Some(backup_path))
}

fn delete_state_threads_for_sessions(
    root: &Path,
    ids: &[String],
    rollout_paths: &[PathBuf],
) -> Result<StateThreadDeleteReport, String> {
    let state_db = root.join("state_5.sqlite");
    if !state_db.exists() {
        return Ok(StateThreadDeleteReport::default());
    }
    let mut connection = Connection::open_with_flags(
        &state_db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| format!("打开 Codex state 数据库失败 {}: {err}", state_db.display()))?;
    connection
        .busy_timeout(Duration::from_millis(3000))
        .map_err(|err| format!("配置 Codex state 数据库等待超时失败: {err}"))?;

    let mut delete_ids: HashSet<String> = ids.iter().cloned().collect();
    if !state_threads_has_columns(&connection, &["id"])? {
        return Ok(StateThreadDeleteReport::default());
    }
    for path in rollout_paths {
        for path_text in rollout_path_lookup_values(root, path) {
            let mut statement = connection
                .prepare("SELECT id FROM threads WHERE rollout_path = ?1")
                .map_err(|err| format!("查询 Codex Desktop threads 索引失败: {err}"))?;
            let rows = statement
                .query_map([path_text], |row| row.get::<_, String>(0))
                .map_err(|err| format!("查询 Codex Desktop threads 索引失败: {err}"))?;
            for id in rows {
                delete_ids
                    .insert(id.map_err(|err| format!("读取 Codex Desktop thread id 失败: {err}"))?);
            }
        }
    }

    if delete_ids.is_empty() {
        return Ok(StateThreadDeleteReport::default());
    }

    let has_thread_dynamic_tools =
        state_table_has_columns(&connection, "thread_dynamic_tools", &["thread_id"])?;
    let has_thread_goals = state_table_has_columns(&connection, "thread_goals", &["thread_id"])?;
    let has_thread_spawn_edges = state_table_has_columns(
        &connection,
        "thread_spawn_edges",
        &["parent_thread_id", "child_thread_id"],
    )?;
    let has_stage1_outputs =
        state_table_has_columns(&connection, "stage1_outputs", &["thread_id"])?;
    let has_agent_job_items =
        state_table_has_columns(&connection, "agent_job_items", &["assigned_thread_id"])?;

    let backup_path = backup_state_database_for_delete(&connection, root)?;
    let transaction = connection
        .transaction()
        .map_err(|err| format!("开始 Codex state 删除事务失败: {err}"))?;
    let mut deleted = 0usize;
    let mut ids: Vec<String> = delete_ids.into_iter().collect();
    ids.sort();
    for id in &ids {
        if has_thread_dynamic_tools {
            transaction
                .execute(
                    "DELETE FROM thread_dynamic_tools WHERE thread_id = ?1",
                    [id],
                )
                .map_err(|err| format!("删除 Codex Desktop thread_dynamic_tools 失败: {err}"))?;
        }
        if has_thread_goals {
            transaction
                .execute("DELETE FROM thread_goals WHERE thread_id = ?1", [id])
                .map_err(|err| format!("删除 Codex Desktop thread_goals 失败: {err}"))?;
        }
        if has_thread_spawn_edges {
            transaction
                .execute(
                    "DELETE FROM thread_spawn_edges WHERE parent_thread_id = ?1 OR child_thread_id = ?1",
                    [id],
                )
                .map_err(|err| format!("删除 Codex Desktop thread_spawn_edges 失败: {err}"))?;
        }
        if has_stage1_outputs {
            transaction
                .execute("DELETE FROM stage1_outputs WHERE thread_id = ?1", [id])
                .map_err(|err| format!("删除 Codex Desktop stage1_outputs 失败: {err}"))?;
        }
        if has_agent_job_items {
            transaction
                .execute(
                    "UPDATE agent_job_items SET assigned_thread_id = NULL WHERE assigned_thread_id = ?1",
                    [id],
                )
                .map_err(|err| format!("清理 Codex Desktop agent_job_items 失败: {err}"))?;
        }
        deleted += transaction
            .execute("DELETE FROM threads WHERE id = ?1", [id])
            .map_err(|err| format!("删除 Codex Desktop threads 索引失败: {err}"))?;
    }
    transaction
        .commit()
        .map_err(|err| format!("保存 Codex Desktop threads 删除结果失败: {err}"))?;
    let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    Ok(StateThreadDeleteReport {
        deleted,
        ids,
        backup_path: Some(backup_path),
    })
}

fn rollout_path_lookup_values(root: &Path, path: &Path) -> Vec<String> {
    let mut values = Vec::new();
    values.push(path.to_string_lossy().to_string());
    values.push(path_to_slash(path));
    if let Ok(canonical) = path.canonicalize() {
        values.push(canonical.to_string_lossy().to_string());
        values.push(path_to_slash(&canonical));
    }
    if let Ok(relative) = path.strip_prefix(root) {
        values.push(relative.to_string_lossy().to_string());
        values.push(path_to_slash(relative));
    }
    dedupe_strings(&mut values);
    values
}

fn state_threads_schema(
    connection: &Connection,
) -> Result<Option<HashMap<String, StateThreadColumn>>, String> {
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'threads')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| format!("检查 Codex Desktop threads 表失败: {err}"))?;
    if exists == 0 {
        return Ok(None);
    }

    let mut statement = connection
        .prepare("PRAGMA table_info(threads)")
        .map_err(|err| format!("读取 Codex Desktop threads 表结构失败: {err}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(StateThreadColumn {
                name: row.get::<_, String>(1)?,
                not_null: row.get::<_, i64>(3)? != 0,
                default_value: row.get::<_, Option<String>>(4)?,
                primary_key: row.get::<_, i64>(5)? != 0,
            })
        })
        .map_err(|err| format!("读取 Codex Desktop threads 表结构失败: {err}"))?;
    let mut columns = HashMap::new();
    for row in rows {
        let column = row.map_err(|err| format!("读取 Codex Desktop threads 列失败: {err}"))?;
        columns.insert(column.name.clone(), column);
    }
    Ok(Some(columns))
}

fn state_threads_has_columns(connection: &Connection, required: &[&str]) -> Result<bool, String> {
    let Some(columns) = state_threads_schema(connection)? else {
        return Ok(false);
    };
    Ok(required.iter().all(|column| columns.contains_key(*column)))
}

fn state_table_has_columns(
    connection: &Connection,
    table: &str,
    required: &[&str],
) -> Result<bool, String> {
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| format!("检查 Codex Desktop {table} 表失败: {err}"))?;
    if exists == 0 {
        return Ok(false);
    }

    let mut statement = connection
        .prepare(&format!(
            "PRAGMA table_info(\"{}\")",
            table.replace('"', "\"\"")
        ))
        .map_err(|err| format!("读取 Codex Desktop {table} 表结构失败: {err}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| format!("读取 Codex Desktop {table} 表结构失败: {err}"))?;
    let mut columns = HashSet::new();
    for row in rows {
        columns.insert(row.map_err(|err| format!("读取 Codex Desktop {table} 列失败: {err}"))?);
    }
    Ok(required.iter().all(|column| columns.contains(*column)))
}

fn update_state_thread_cwds(root: &Path, items: &[(String, String)]) -> Result<usize, String> {
    let state_db = root.join("state_5.sqlite");
    if items.is_empty() || !state_db.exists() {
        return Ok(0);
    }
    let mut connection = Connection::open_with_flags(
        &state_db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| format!("打开 Codex state 数据库失败 {}: {err}", state_db.display()))?;
    connection
        .busy_timeout(Duration::from_millis(3000))
        .map_err(|err| format!("配置 Codex state 数据库等待超时失败: {err}"))?;
    let transaction = connection
        .transaction()
        .map_err(|err| format!("开始 Codex state cwd 更新事务失败: {err}"))?;
    let mut updated = 0usize;
    for (id, cwd) in items {
        updated += transaction
            .execute(
                "UPDATE threads SET cwd = ?1 WHERE id = ?2",
                params![cwd, id],
            )
            .map_err(|err| format!("更新 Codex Desktop threads.cwd 失败: {err}"))?;
    }
    transaction
        .commit()
        .map_err(|err| format!("保存 Codex Desktop threads.cwd 失败: {err}"))?;
    Ok(updated)
}

fn rewrite_session_cwd(path: &Path, cwd: &str) -> Result<bool, String> {
    let content = fs::read_to_string(path)
        .map_err(|err| format!("读取会话文件失败 {}: {err}", path.display()))?;
    let mut output = String::with_capacity(content.len());
    let mut changed = false;
    for segment in content.split_inclusive('\n') {
        let (line, line_ending) = split_line_ending(segment);
        match update_cwd_line(line, cwd)? {
            Some(updated_line) => {
                output.push_str(&updated_line);
                output.push_str(line_ending);
                changed = true;
            }
            None => output.push_str(segment),
        }
    }
    if !content.ends_with('\n') {
        let last_line = content
            .rsplit_once('\n')
            .map(|(_, line)| line)
            .unwrap_or(content.as_str());
        if !last_line.is_empty() && !output.ends_with(last_line) {
            // split_inclusive already handled this branch. This is a guard for future edits.
        }
    }
    if changed {
        fs::write(path, output)
            .map_err(|err| format!("写入会话文件失败 {}: {err}", path.display()))?;
    }
    Ok(changed)
}

fn copy_session_with_new_id(
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

fn rewrite_session_id_content(content: &str, old_id: &str, new_id: &str) -> Result<String, String> {
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

fn replace_exact_string_value(value: &mut Value, old_value: &str, new_value: &str) {
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

fn update_cwd_line(line: &str, cwd: &str) -> Result<Option<String>, String> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    let mut value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let event_type = raw_string_field(&value, "type");
    let should_update = event_type == "session_meta" || event_type == "turn_context" || {
        event_type == "event_msg"
            && value
                .get("payload")
                .map(|payload| raw_string_field(payload, "type") == "task_started")
                .unwrap_or(false)
    };
    if !should_update {
        return Ok(None);
    }
    let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) else {
        return Ok(None);
    };
    if payload.get("cwd").and_then(Value::as_str) == Some(cwd) {
        return Ok(None);
    }
    payload.insert("cwd".to_string(), Value::String(cwd.to_string()));
    serde_json::to_string(&value)
        .map(Some)
        .map_err(|err| format!("序列化会话 cwd 更新失败: {err}"))
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

fn normalize_relative_path(value: &str) -> Result<PathBuf, String> {
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

fn ensure_session_relative_path(path: &Path) -> Result<(), String> {
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

fn status_from_relative_path(path: &Path) -> Result<String, String> {
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

fn normalize_status(status: &str) -> Result<String, String> {
    match status.trim().to_ascii_lowercase().as_str() {
        "active" => Ok("active".to_string()),
        "archived" => Ok("archived".to_string()),
        _ => Err(format!("不支持的会话状态: {status}")),
    }
}

fn session_date_parts(summary: &SessionSummary, path: &Path) -> (String, String, String) {
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

fn date_parts_from_timestamp(timestamp: &str) -> Option<(String, String, String)> {
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

fn date_parts_from_rollout_filename(file_name: &str) -> Option<(String, String, String)> {
    let raw = file_name.strip_prefix("rollout-")?.get(0..10)?;
    date_parts_from_timestamp(raw)
}

fn conversation_sort_key(item: &ConversationItem) -> i64 {
    item.updated_at
        .as_deref()
        .and_then(parse_rfc3339_seconds)
        .unwrap_or(0)
}

fn is_jsonl_file(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
}

fn set_first(target: &mut Option<String>, value: Option<String>) {
    if target.is_none() {
        *target = value;
    }
}

fn first_non_empty(values: &[String]) -> Option<String> {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn truncate_text(text: &str, max_chars: usize) -> String {
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

fn extract_uuid_like(value: &str) -> Option<String> {
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

fn new_session_id(seed: &str) -> String {
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

fn reassigned_relative_path(
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

fn path_to_slash(path: impl AsRef<Path>) -> String {
    path.as_ref()
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

fn sha256_file(path: &Path) -> Result<String, String> {
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

fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex_bytes(&hasher.finalize())
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    result
}

fn system_time_to_rfc3339(time: Option<SystemTime>) -> Option<String> {
    time.map(OffsetDateTime::from).and_then(|time| {
        time.format(&time::format_description::well_known::Rfc3339)
            .ok()
    })
}

fn timestamp_seconds_to_rfc3339(seconds: i64) -> Option<String> {
    OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|time| {
            time.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn backup_stamp() -> String {
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

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

struct ZipCentralEntry {
    name: String,
    crc: u32,
    size: u32,
    offset: u32,
}

fn write_zip_store(path: &Path, entries: &[(String, Vec<u8>)]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("创建导出目录失败 {}: {err}", parent.display()))?;
    }
    let mut file = fs::File::create(path)
        .map_err(|err| format!("创建 zip 文件失败 {}: {err}", path.display()))?;
    let mut central_entries = Vec::new();
    for (name, data) in entries {
        let name_bytes = name.as_bytes();
        if name_bytes.len() > u16::MAX as usize {
            return Err(format!("zip 条目路径过长: {name}"));
        }
        if data.len() > u32::MAX as usize {
            return Err(format!("zip 条目过大: {name}"));
        }
        let offset = file
            .stream_position()
            .map_err(|err| format!("读取 zip 写入位置失败: {err}"))?;
        if offset > u32::MAX as u64 {
            return Err("zip 文件过大，V1 不支持 Zip64".to_string());
        }
        let crc = crc32(data);
        write_u32(&mut file, ZIP_LOCAL_FILE_HEADER)?;
        write_u16(&mut file, 20)?;
        write_u16(&mut file, ZIP_UTF8_FLAG)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 33)?;
        write_u32(&mut file, crc)?;
        write_u32(&mut file, data.len() as u32)?;
        write_u32(&mut file, data.len() as u32)?;
        write_u16(&mut file, name_bytes.len() as u16)?;
        write_u16(&mut file, 0)?;
        file.write_all(name_bytes)
            .map_err(|err| format!("写入 zip 条目名失败: {err}"))?;
        file.write_all(data)
            .map_err(|err| format!("写入 zip 条目失败: {err}"))?;
        central_entries.push(ZipCentralEntry {
            name: name.clone(),
            crc,
            size: data.len() as u32,
            offset: offset as u32,
        });
    }
    let central_start = file
        .stream_position()
        .map_err(|err| format!("读取 zip central directory 位置失败: {err}"))?;
    if central_start > u32::MAX as u64 {
        return Err("zip 文件过大，V1 不支持 Zip64".to_string());
    }
    for entry in &central_entries {
        let name_bytes = entry.name.as_bytes();
        write_u32(&mut file, ZIP_CENTRAL_DIRECTORY_HEADER)?;
        write_u16(&mut file, 20)?;
        write_u16(&mut file, 20)?;
        write_u16(&mut file, ZIP_UTF8_FLAG)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 33)?;
        write_u32(&mut file, entry.crc)?;
        write_u32(&mut file, entry.size)?;
        write_u32(&mut file, entry.size)?;
        write_u16(&mut file, name_bytes.len() as u16)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 0)?;
        write_u32(&mut file, 0)?;
        write_u32(&mut file, entry.offset)?;
        file.write_all(name_bytes)
            .map_err(|err| format!("写入 zip central directory 失败: {err}"))?;
    }
    let central_end = file
        .stream_position()
        .map_err(|err| format!("读取 zip central directory 大小失败: {err}"))?;
    let central_size = central_end - central_start;
    if central_size > u32::MAX as u64 || central_entries.len() > u16::MAX as usize {
        return Err("zip 文件过大，V1 不支持 Zip64".to_string());
    }
    write_u32(&mut file, ZIP_END_OF_CENTRAL_DIRECTORY)?;
    write_u16(&mut file, 0)?;
    write_u16(&mut file, 0)?;
    write_u16(&mut file, central_entries.len() as u16)?;
    write_u16(&mut file, central_entries.len() as u16)?;
    write_u32(&mut file, central_size as u32)?;
    write_u32(&mut file, central_start as u32)?;
    write_u16(&mut file, 0)?;
    Ok(())
}

fn write_u16(file: &mut fs::File, value: u16) -> Result<(), String> {
    file.write_all(&value.to_le_bytes())
        .map_err(|err| format!("写入 zip 失败: {err}"))
}

fn write_u32(file: &mut fs::File, value: u32) -> Result<(), String> {
    file.write_all(&value.to_le_bytes())
        .map_err(|err| format!("写入 zip 失败: {err}"))
}

#[derive(Debug, Clone)]
struct ZipArchiveLite {
    data: Vec<u8>,
    entries: HashMap<String, ZipReadEntry>,
}

#[derive(Debug, Clone)]
struct ZipReadEntry {
    method: u16,
    crc: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    local_header_offset: u32,
}

impl ZipArchiveLite {
    fn open(path: &Path) -> Result<Self, String> {
        let data =
            fs::read(path).map_err(|err| format!("读取导入 zip 失败 {}: {err}", path.display()))?;
        Self::from_bytes(data)
    }

    fn from_bytes(data: Vec<u8>) -> Result<Self, String> {
        let eocd = find_eocd(&data).ok_or_else(|| "未找到 zip central directory".to_string())?;
        let disk = read_u16_at(&data, eocd + 4)?;
        let central_disk = read_u16_at(&data, eocd + 6)?;
        if disk != 0 || central_disk != 0 {
            return Err("不支持分卷 zip".to_string());
        }
        let entry_count = read_u16_at(&data, eocd + 10)? as usize;
        let central_size = read_u32_at(&data, eocd + 12)? as usize;
        let central_offset = read_u32_at(&data, eocd + 16)? as usize;
        if central_offset + central_size > data.len() {
            return Err("zip central directory 越界".to_string());
        }

        let mut entries = HashMap::new();
        let mut cursor = central_offset;
        for _ in 0..entry_count {
            if read_u32_at(&data, cursor)? != ZIP_CENTRAL_DIRECTORY_HEADER {
                return Err("zip central directory 结构无效".to_string());
            }
            let flags = read_u16_at(&data, cursor + 8)?;
            let method = read_u16_at(&data, cursor + 10)?;
            let crc = read_u32_at(&data, cursor + 16)?;
            let compressed_size = read_u32_at(&data, cursor + 20)?;
            let uncompressed_size = read_u32_at(&data, cursor + 24)?;
            let name_len = read_u16_at(&data, cursor + 28)? as usize;
            let extra_len = read_u16_at(&data, cursor + 30)? as usize;
            let comment_len = read_u16_at(&data, cursor + 32)? as usize;
            let local_header_offset = read_u32_at(&data, cursor + 42)?;
            let name_start = cursor + 46;
            let name_end = name_start + name_len;
            if name_end > data.len() {
                return Err("zip 条目名越界".to_string());
            }
            let name = if flags & ZIP_UTF8_FLAG != 0 {
                String::from_utf8(data[name_start..name_end].to_vec())
                    .map_err(|_| "zip 条目名不是 UTF-8".to_string())?
            } else {
                String::from_utf8_lossy(&data[name_start..name_end]).to_string()
            };
            if name != "manifest.json" {
                let relative = normalize_relative_path(&name)?;
                ensure_session_relative_path(&relative)?;
            }
            entries.insert(
                name,
                ZipReadEntry {
                    method,
                    crc,
                    compressed_size,
                    uncompressed_size,
                    local_header_offset,
                },
            );
            cursor = name_end + extra_len + comment_len;
            if cursor > data.len() {
                return Err("zip central directory 条目越界".to_string());
            }
        }
        Ok(Self { data, entries })
    }

    fn read_entry(&self, name: &str) -> Result<Vec<u8>, String> {
        let entry = self
            .entries
            .get(name)
            .ok_or_else(|| format!("zip 中缺少文件: {name}"))?;
        if entry.method != 0 {
            return Err(format!("zip 文件 {name} 使用了不支持的压缩方式"));
        }
        if entry.compressed_size != entry.uncompressed_size {
            return Err(format!("zip 文件 {name} 大小信息不一致"));
        }
        let offset = entry.local_header_offset as usize;
        if read_u32_at(&self.data, offset)? != ZIP_LOCAL_FILE_HEADER {
            return Err(format!("zip 文件 {name} 的本地头无效"));
        }
        let name_len = read_u16_at(&self.data, offset + 26)? as usize;
        let extra_len = read_u16_at(&self.data, offset + 28)? as usize;
        let start = offset + 30 + name_len + extra_len;
        let end = start + entry.uncompressed_size as usize;
        if end > self.data.len() {
            return Err(format!("zip 文件 {name} 数据越界"));
        }
        let data = self.data[start..end].to_vec();
        if crc32(&data) != entry.crc {
            return Err(format!("zip 文件 {name} CRC 校验失败"));
        }
        Ok(data)
    }
}

fn find_eocd(data: &[u8]) -> Option<usize> {
    if data.len() < 22 {
        return None;
    }
    let min = data.len().saturating_sub(65_557);
    (min..=data.len() - 22)
        .rev()
        .find(|index| read_u32_at(data, *index).ok() == Some(ZIP_END_OF_CENTRAL_DIRECTORY))
}

fn read_u16_at(data: &[u8], offset: usize) -> Result<u16, String> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| "zip 数据越界".to_string())?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32_at(data: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| "zip 数据越界".to_string())?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn temp_path(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("codex-switch-session-manager-{name}-{stamp}"))
    }

    fn sample_thread_metadata(rollout_path: PathBuf) -> ThreadMetadata {
        ThreadMetadata {
            id: "thread-1".to_string(),
            rollout_path,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_120,
            source: "codex".to_string(),
            model_provider: "openai".to_string(),
            cwd: "C:\\work".to_string(),
            title: "Imported thread".to_string(),
            sandbox_policy: "workspace-write".to_string(),
            approval_mode: "on-request".to_string(),
            has_user_event: 1,
            archived: 0,
            archived_at: None,
            first_user_message: "hello".to_string(),
            model: Some("gpt-5.2".to_string()),
            reasoning_effort: Some("high".to_string()),
        }
    }

    #[test]
    fn relative_path_rejects_traversal() {
        assert!(normalize_relative_path("../sessions/a.jsonl").is_err());
        assert!(normalize_relative_path("sessions/2026/05/01/a.jsonl").is_ok());
    }

    #[test]
    fn parser_prefers_event_messages_over_response_items() {
        let path = temp_path("parse.jsonl");
        fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-05-01T00:00:00Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"fallback\"}]}}\n",
                "{\"timestamp\":\"2026-05-01T00:00:01Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"hello\"}}\n",
                "{\"timestamp\":\"2026-05-01T00:00:02Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"world\"}}\n"
            ),
        )
        .unwrap();

        let summary = parse_session_file(&path, true).unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(summary.messages.len(), 2);
        assert_eq!(summary.messages[0].role, "user");
        assert_eq!(summary.messages[0].text, "hello");
        assert_eq!(summary.messages[1].role, "assistant");
    }

    #[test]
    fn preview_deleted_session_reads_backup_messages() {
        let record_dir = temp_path("deleted-preview");
        fs::create_dir_all(&record_dir).unwrap();
        let record = DeletedSessionRecord {
            delete_id: "delete-id".to_string(),
            id: "019e20f9-34b7-7a82-a95b-fe461de8983a".to_string(),
            title: "已删除预览".to_string(),
            deleted_at: "2026-05-13T10:55:00Z".to_string(),
            updated_at: Some("2026-05-13T10:54:00Z".to_string()),
            original_status: "active".to_string(),
            original_relative_path: "sessions/2026/05/13/rollout-2026-05-13T10-54-00-test.jsonl"
                .to_string(),
            deleted_relative_path: "sessions/2026/05/13/rollout-2026-05-13T10-54-00-test.jsonl"
                .to_string(),
            root_path: "C:\\Users\\yuhon\\.codex".to_string(),
            size_bytes: 0,
            cwd: Some("C:\\work".to_string()),
            session_file: "session.jsonl".to_string(),
        };
        write_deleted_session_record(&record_dir, &record).unwrap();
        fs::write(
            record_dir.join("session.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-05-13T10:54:01Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"019e20f9-34b7-7a82-a95b-fe461de8983a\",\"cwd\":\"C:\\\\work\"}}\n",
                "{\"timestamp\":\"2026-05-13T10:54:02Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"hi\"}}\n",
                "{\"timestamp\":\"2026-05-13T10:54:03Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"hello\"}}\n"
            ),
        )
        .unwrap();

        let preview = preview_deleted_conversation_from_record_dir(&record_dir).unwrap();
        let messages = preview
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        fs::remove_dir_all(&record_dir).unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["text"], "hi");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["text"], "hello");
    }

    #[test]
    fn zip_store_round_trip_reads_entries() {
        let path = temp_path("archive.zip");
        write_zip_store(
            &path,
            &[
                ("manifest.json".to_string(), br#"{"ok":true}"#.to_vec()),
                (
                    "sessions/2026/05/01/rollout-test.jsonl".to_string(),
                    b"{}\n".to_vec(),
                ),
            ],
        )
        .unwrap();

        let archive = ZipArchiveLite::open(&path).unwrap();
        let manifest = archive.read_entry("manifest.json").unwrap();
        let session = archive
            .read_entry("sessions/2026/05/01/rollout-test.jsonl")
            .unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(manifest, br#"{"ok":true}"#);
        assert_eq!(session, b"{}\n");
    }

    #[test]
    fn status_change_syncs_session_index_for_codex_sidebar() {
        let root = temp_path("status-index");
        let session_id = "019e20f9-34b7-7a82-a95b-fe461de8983a";
        let file_name = "rollout-2026-05-13T18-54-23-019e20f9-34b7-7a82-a95b-fe461de8983a.jsonl";
        let active_relative = PathBuf::from("sessions")
            .join("2026")
            .join("05")
            .join("13")
            .join(file_name);
        let active_path = root.join(&active_relative);
        fs::create_dir_all(active_path.parent().unwrap()).unwrap();
        fs::write(
            &active_path,
            format!(
                "{}\n{}\n",
                json!({
                    "timestamp": "2026-05-13T10:54:26.757Z",
                    "type": "session_meta",
                    "payload": {
                        "id": session_id,
                        "cwd": "C:\\Users\\yuhon\\Documents\\Codex\\hello"
                    }
                }),
                json!({
                    "timestamp": "2026-05-13T10:54:27.000Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "thread_name_updated",
                        "thread_name": "测试归档索引"
                    }
                })
            ),
        )
        .unwrap();
        fs::write(
            root.join("session_index.jsonl"),
            format!(
                "{}\n",
                json!({
                    "id": session_id,
                    "thread_name": "测试归档索引",
                    "updated_at": "2026-05-13T10:54:27.000Z"
                })
            ),
        )
        .unwrap();

        set_conversation_status_impl(
            root.to_string_lossy().to_string(),
            vec![path_to_slash(&active_relative)],
            "archived".to_string(),
            None,
        )
        .unwrap();
        let archived_relative = PathBuf::from("archived_sessions").join(file_name);
        let archived_path = root.join(&archived_relative);
        let archived_index = fs::read_to_string(root.join("session_index.jsonl")).unwrap();
        assert!(archived_path.exists());
        assert!(!archived_index.contains(session_id));

        fs::write(
            root.join("session_index.jsonl"),
            format!(
                "{}\n",
                json!({
                    "id": session_id,
                    "thread_name": "旧版残留索引",
                    "updated_at": "2026-05-13T10:54:27.000Z"
                })
            ),
        )
        .unwrap();
        set_conversation_status_impl(
            root.to_string_lossy().to_string(),
            vec![path_to_slash(&archived_relative)],
            "archived".to_string(),
            None,
        )
        .unwrap();
        let repaired_index = fs::read_to_string(root.join("session_index.jsonl")).unwrap();
        assert!(!repaired_index.contains(session_id));

        set_conversation_status_impl(
            root.to_string_lossy().to_string(),
            vec![path_to_slash(&archived_relative)],
            "active".to_string(),
            None,
        )
        .unwrap();
        let active_index = fs::read_to_string(root.join("session_index.jsonl")).unwrap();

        fs::remove_dir_all(&root).unwrap();

        assert!(active_index.contains(session_id));
        assert!(active_index.contains("测试归档索引"));
    }

    #[test]
    fn scan_reports_archived_session_index_residue_without_repairing() {
        let root = temp_path("scan-repair-index");
        let session_id = "019e20f9-34b7-7a82-a95b-fe461de8983a";
        let file_name = "rollout-2026-05-13T18-54-23-019e20f9-34b7-7a82-a95b-fe461de8983a.jsonl";
        let archived_path = root.join("archived_sessions").join(file_name);
        fs::create_dir_all(archived_path.parent().unwrap()).unwrap();
        fs::write(
            &archived_path,
            format!(
                "{}\n",
                json!({
                    "timestamp": "2026-05-13T10:54:26.757Z",
                    "type": "session_meta",
                    "payload": {
                        "id": session_id,
                        "cwd": "C:\\Users\\yuhon\\Documents\\Codex\\hello"
                    }
                })
            ),
        )
        .unwrap();
        fs::write(
            root.join("session_index.jsonl"),
            format!(
                "{}\n{}\n",
                json!({
                    "id": session_id,
                    "thread_name": "旧版残留索引",
                    "updated_at": "2026-05-13T10:54:27.000Z"
                }),
                json!({
                    "id": "active-session",
                    "thread_name": "进行中会话",
                    "updated_at": "2026-05-13T10:55:27.000Z"
                })
            ),
        )
        .unwrap();

        let result = scan_conversations_impl(Some(root.to_string_lossy().to_string())).unwrap();
        let scanned_index = fs::read_to_string(root.join("session_index.jsonl")).unwrap();
        let warnings = result
            .get("warnings")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        fs::remove_dir_all(&root).unwrap();

        assert!(scanned_index.contains(session_id));
        assert!(scanned_index.contains("active-session"));
        assert!(warnings.iter().any(|warning| warning
            .as_str()
            .is_some_and(|text| text.contains("发现 1 条已归档会话的 Codex 索引残留"))));
    }

    #[test]
    fn upsert_state_threads_supports_older_thread_schema() {
        let root = temp_path("upsert-old-schema");
        fs::create_dir_all(&root).unwrap();
        let state_db = root.join("state_5.sqlite");
        let connection = Connection::open(&state_db).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    title TEXT,
                    updated_at INTEGER
                );
                "#,
            )
            .unwrap();
        drop(connection);

        let item = sample_thread_metadata(root.join("sessions/rollout-thread-1.jsonl"));
        let updated = upsert_state_threads(&root, &[item]).unwrap();
        let connection = Connection::open(&state_db).unwrap();
        let row = connection
            .query_row(
                "SELECT rollout_path, title, updated_at FROM threads WHERE id = 'thread-1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(updated, 1);
        assert!(row.0.ends_with("sessions/rollout-thread-1.jsonl"));
        assert_eq!(row.1, "Imported thread");
        assert_eq!(row.2, 1_700_000_120);

        drop(connection);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn upsert_state_threads_skips_unsupported_required_schema() {
        let root = temp_path("upsert-required-schema");
        fs::create_dir_all(&root).unwrap();
        let state_db = root.join("state_5.sqlite");
        let connection = Connection::open(&state_db).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    unsupported TEXT NOT NULL
                );
                "#,
            )
            .unwrap();
        drop(connection);

        let item = sample_thread_metadata(root.join("sessions/rollout-thread-1.jsonl"));
        let updated = upsert_state_threads(&root, &[item]).unwrap();
        let connection = Connection::open(&state_db).unwrap();
        let count = connection
            .query_row("SELECT COUNT(*) FROM threads", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();

        assert_eq!(updated, 0);
        assert_eq!(count, 0);

        drop(connection);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn delete_state_threads_removes_related_rows() {
        let root = temp_path("delete-state-related");
        fs::create_dir_all(&root).unwrap();
        let state_db = root.join("state_5.sqlite");
        let connection = Connection::open(&state_db).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT);
                CREATE TABLE thread_dynamic_tools (thread_id TEXT NOT NULL, tool_name TEXT NOT NULL);
                CREATE TABLE thread_goals (thread_id TEXT NOT NULL, goal TEXT NOT NULL);
                CREATE TABLE thread_spawn_edges (parent_thread_id TEXT NOT NULL, child_thread_id TEXT NOT NULL);
                CREATE TABLE stage1_outputs (thread_id TEXT NOT NULL, output TEXT NOT NULL);
                CREATE TABLE agent_job_items (id TEXT PRIMARY KEY, assigned_thread_id TEXT);
                INSERT INTO threads (id, rollout_path, title) VALUES ('t1', 'sessions/rollout-t1.jsonl', 'Thread');
                INSERT INTO thread_dynamic_tools (thread_id, tool_name) VALUES ('t1', 'Read');
                INSERT INTO thread_goals (thread_id, goal) VALUES ('t1', 'goal');
                INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id) VALUES ('t1', 'child');
                INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id) VALUES ('parent', 't1');
                INSERT INTO stage1_outputs (thread_id, output) VALUES ('t1', 'cached');
                INSERT INTO agent_job_items (id, assigned_thread_id) VALUES ('job1', 't1');
                "#,
            )
            .unwrap();
        drop(connection);

        let report = delete_state_threads_for_sessions(&root, &["t1".to_string()], &[]).unwrap();
        let connection = Connection::open(&state_db).unwrap();

        assert_eq!(report.deleted, 1);
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM threads WHERE id = 't1'", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM thread_spawn_edges WHERE parent_thread_id = 't1' OR child_thread_id = 't1'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT assigned_thread_id FROM agent_job_items WHERE id = 'job1'",
                    [],
                    |row| { row.get::<_, Option<String>>(0) }
                )
                .unwrap(),
            None
        );

        drop(connection);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn delete_state_threads_resolves_thread_id_from_rollout_path() {
        let root = temp_path("delete-state-rollout-path");
        let rollout_relative = PathBuf::from("sessions/2026/05/15/rollout-t1.jsonl");
        let rollout_path = root.join(&rollout_relative);
        fs::create_dir_all(rollout_path.parent().unwrap()).unwrap();
        fs::write(&rollout_path, "{}\n").unwrap();
        let state_db = root.join("state_5.sqlite");
        let connection = Connection::open(&state_db).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT);
                "#,
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, title) VALUES (?1, ?2, 'Thread')",
                ("local:t1", path_to_slash(&rollout_relative)),
            )
            .unwrap();
        drop(connection);

        let report =
            delete_state_threads_for_sessions(&root, &["t1".to_string()], &[rollout_path]).unwrap();
        let connection = Connection::open(&state_db).unwrap();

        assert_eq!(report.deleted, 1);
        assert!(report.ids.contains(&"local:t1".to_string()));
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM threads WHERE id = 'local:t1'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );

        drop(connection);
        fs::remove_dir_all(&root).unwrap();
    }
}
