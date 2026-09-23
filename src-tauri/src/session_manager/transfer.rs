use super::{
    backup::backup_file,
    catalog::{conversation_from_path, read_current_state_conversations},
    codex_home::{
        conversation_path_key, ensure_session_relative_path, normalize_relative_path,
        normalize_status, read_session_index, resolve_codex_root, status_from_relative_path,
        validate_codex_root,
    },
    model::ManifestSession,
    rollout::parse_session_file_for_list,
    state_db::{thread_metadata_from_manifest, upsert_state_threads},
    util::{backup_stamp, sha256_bytes, sha256_file},
    zip::{write_zip_store, ZipArchiveLite},
};
use crate::{
    codex_sessions::lock_codex_session_io, paths::codex_state_db_path_for_root,
    time_util::now_string,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
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

    let manifest = ExportManifest {
        format: MANIFEST_FORMAT.to_string(),
        version: MANIFEST_VERSION,
        exported_at: now_string(),
        source_os: std::env::consts::OS.to_string(),
        sessions: bundle.sessions,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|err| format!("生成 manifest.json 失败: {err}"))?;
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
    pub(super) sessions: Vec<ManifestSession>,
    pub(super) entries: Vec<(String, Vec<u8>)>,
    pub(super) errors: Vec<String>,
    pub(super) warnings: Vec<String>,
    pub(super) total_size: u64,
}

/// Reads the selected sessions under the session I/O lock. Each file is read exactly once and that
/// one buffer provides the zip entry, its SHA-256 and its size, so the manifest always matches
/// the stored bytes even while Codex keeps appending to the file.
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
        .map(|item| (conversation_path_key(Path::new(&item.source_path)), item))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut bundle = ExportBundle {
        sessions: Vec::new(),
        entries: Vec::new(),
        errors: Vec::new(),
        warnings,
        total_size: 0,
    };

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
        let path = root.join(&relative);
        let status = match status_from_relative_path(&relative) {
            Ok(status) => status,
            Err(err) => {
                bundle.errors.push(err);
                continue;
            }
        };
        let item = match state_by_path.get(&conversation_path_key(&path)) {
            Some(item) => item.clone(),
            None => match conversation_from_path(root, &path, &status, false, &session_index) {
                Ok(item) => item,
                Err(err) => {
                    bundle.errors.push(err);
                    continue;
                }
            },
        };
        let data = match fs::read(&path) {
            Ok(data) => data,
            Err(err) => {
                bundle
                    .errors
                    .push(format!("读取会话文件失败 {}: {err}", path.display()));
                continue;
            }
        };
        let size_bytes = data.len() as u64;
        bundle.total_size += size_bytes;
        bundle.sessions.push(ManifestSession {
            id: item.id,
            title: item.title,
            updated_at: item.updated_at,
            status: item.status,
            relative_path: item.relative_path.clone(),
            size_bytes,
            sha256: sha256_bytes(&data),
        });
        bundle.entries.push((item.relative_path, data));
    }
    Ok(bundle)
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

    let _io_guard = lock_codex_session_io("导入会话")?;
    let state_db = codex_state_db_path_for_root(&root)?;
    let state_backup_path = if state_db.exists()
        && candidates.iter().any(|candidate| {
            matches!(
                candidate.action,
                ImportAction::Import | ImportAction::SkipSame
            )
        }) {
        Some(backup_file(&state_db)?)
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

    let imported_sessions: Vec<ManifestSession> = candidates
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.action,
                ImportAction::Import | ImportAction::SkipSame
            )
        })
        .map(|candidate| candidate.manifest.clone())
        .collect();

    let mut sqlite_updated = 0usize;
    let mut sqlite_error = None;
    if state_db.exists() && !imported_sessions.is_empty() {
        let mut thread_metadata = Vec::new();
        for candidate in candidates.iter().filter(|candidate| {
            matches!(
                candidate.action,
                ImportAction::Import | ImportAction::SkipSame
            )
        }) {
            let summary = parse_session_file_for_list(&candidate.target_path).unwrap_or_default();
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
            "state_backup_path": state_backup_path.map(|path| path.to_string_lossy().to_string())
        }
    }))
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
