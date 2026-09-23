use super::{
    backup::status_overwrite_backup_path,
    codex_home::{
        ensure_session_relative_path, extract_uuid_like, normalize_relative_path, normalize_status,
        path_to_slash, reassigned_relative_path, remove_empty_parent_dirs,
        remove_from_global_state, resolve_codex_root, status_from_relative_path,
        validate_codex_root, validate_session_file_path,
    },
    model::{parse_conflict_strategy, ConflictStrategy, SessionSummary, StatusMove},
    rollout::{
        conversation_title_from_summary, copy_session_with_new_id, new_session_id,
        parse_session_file_for_list,
    },
    state_db::apply_status_moves_to_state_db,
    util::dedupe_strings,
};
use crate::{
    codex_sessions::lock_codex_session_io, session_sync_diagnostics::log_session_sync_event,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};
use time::OffsetDateTime;

pub(super) fn set_conversation_status_impl(
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
        let summary = parse_session_file_for_list(&source_path).unwrap_or_default();
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
                        "title": conversation_title_from_summary(&summary)
                    }));
                    continue;
                }
                ConflictStrategy::Skip => {
                    skipped += 1;
                    continue;
                }
                ConflictStrategy::Overwrite => {
                    overwritten_id = parse_session_file_for_list(&target_path)
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

    // Phase 1: reversible file moves. A rewritten copy keeps its source and an overwritten target
    // keeps its backup until the state DB has committed, so a DB failure can put every file back.
    let mut applied_moves = Vec::new();
    for status_move in &moves {
        match apply_status_move_file(&root, status_move, conflict_strategy) {
            Ok(applied) => applied_moves.push(applied),
            Err(err) => errors.push(err),
        }
    }

    let completed_moves = applied_moves
        .iter()
        .map(|applied| applied.status_move.clone())
        .collect::<Vec<_>>();
    let active_ids: HashSet<&str> = completed_moves
        .iter()
        .map(|status_move| status_move.target_id.as_str())
        .collect();
    let mut overwritten_ids = applied_moves
        .iter()
        .filter(|applied| applied.overwrite_backup.is_some())
        .filter_map(|applied| applied.status_move.overwritten_id.clone())
        .filter(|id| !active_ids.contains(id.as_str()))
        .collect::<Vec<_>>();
    dedupe_strings(&mut overwritten_ids);

    // Phase 2: one state DB transaction for the whole batch.
    let state_backup_path = match apply_status_moves_to_state_db(
        &root,
        &completed_moves,
        &target_status,
        &overwritten_ids,
    ) {
        Ok(backup_path) => backup_path,
        Err(db_error) => {
            let mut rollback_errors = Vec::new();
            for applied in applied_moves.iter().rev() {
                if let Err(err) = rollback_status_move_file(applied) {
                    rollback_errors.push(err);
                }
            }
            log_session_sync_event(
                "session_manager_status_state_db_error",
                json!({
                    "root": root.to_string_lossy().to_string(),
                    "targetStatus": target_status,
                    "conflictStrategy": format!("{conflict_strategy:?}"),
                    "movedFiles": applied_moves.len(),
                    "overwrittenIds": overwritten_ids,
                    "error": db_error,
                    "rolledBack": applied_moves.len() - rollback_errors.len(),
                    "rollbackErrors": rollback_errors
                }),
            );
            let message = if rollback_errors.is_empty() {
                format!(
                    "切换会话状态失败：Codex state 数据库更新失败，已撤销 {} 个文件移动：{db_error}",
                    applied_moves.len()
                )
            } else {
                format!(
                    "切换会话状态失败：Codex state 数据库更新失败，{} 个文件未能撤销移动：{db_error}；{}",
                    rollback_errors.len(),
                    rollback_errors.join("；")
                )
            };
            errors.extend(rollback_errors);
            return Ok(json!({
                "ok": false,
                "message": message,
                "report": {
                    "changed": 0,
                    "skipped": skipped,
                    "state_backup_path": null,
                    "desktop_error": db_error,
                    "conflicts": conflicts,
                    "failed": errors.len(),
                    "errors": errors
                }
            }));
        }
    };

    // Phase 3: the DB points at the new files; drop what only existed for rollback.
    let mut cleanup_errors = Vec::new();
    for applied in &applied_moves {
        cleanup_errors.extend(finalize_status_move_file(&root, applied));
    }
    changed += applied_moves.len();
    if let Err(err) = remove_from_global_state(&root, &overwritten_ids, "status-overwrite") {
        cleanup_errors.push(format!("清理被覆盖会话的 global state 失败: {err}"));
    }
    if !cleanup_errors.is_empty() {
        log_session_sync_event(
            "session_manager_status_cleanup_error",
            json!({
                "root": root.to_string_lossy().to_string(),
                "targetStatus": target_status,
                "changed": changed,
                "overwrittenIds": overwritten_ids,
                "errors": cleanup_errors
            }),
        );
        errors.extend(cleanup_errors);
    }

    Ok(json!({
        "ok": errors.is_empty(),
        "message": if errors.is_empty() {
            format!("已切换 {} 个会话状态", changed)
        } else {
            format!("已切换 {} 个会话状态，{} 个失败：{}", changed, errors.len(), errors.join("；"))
        },
        "report": {
            "changed": changed,
            "skipped": skipped,
            "state_backup_path": state_backup_path.map(|path| path.to_string_lossy().to_string()),
            "desktop_error": null,
            "conflicts": conflicts,
            "failed": errors.len(),
            "errors": errors
        }
    }))
}

struct AppliedStatusMove {
    status_move: StatusMove,
    overwrite_backup: Option<PathBuf>,
}

fn apply_status_move_file(
    root: &Path,
    status_move: &StatusMove,
    conflict_strategy: ConflictStrategy,
) -> Result<AppliedStatusMove, String> {
    let target_relative = status_move
        .target_path
        .strip_prefix(root)
        .map(path_to_slash)
        .unwrap_or_else(|_| status_move.target_path.to_string_lossy().to_string());
    if let Some(parent) = status_move.target_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("创建目标目录失败 {}: {err}", parent.display()))?;
    }
    let overwrite_backup = if status_move.target_path.exists()
        && conflict_strategy == ConflictStrategy::Overwrite
    {
        let backup_path = status_overwrite_backup_path(&status_move.target_path, &status_move.id);
        fs::rename(&status_move.target_path, &backup_path)
            .map_err(|err| format!("备份覆盖目标会话失败 {}: {err}", target_relative))?;
        Some(backup_path)
    } else {
        None
    };
    if overwrite_backup.is_none() && status_move.target_path.exists() {
        // Planned as free under the same I/O lock; something else created it since. Never replace it.
        return Err(format!("目标会话已存在，未移动: {target_relative}"));
    }
    let move_result = if let Some((old_id, new_id)) = &status_move.rewrite_id {
        copy_session_with_new_id(
            &status_move.source_path,
            &status_move.target_path,
            old_id,
            new_id,
        )
    } else {
        fs::rename(&status_move.source_path, &status_move.target_path).map_err(|err| {
            format!(
                "移动会话失败 {} -> {}: {err}",
                status_move.source_path.display(),
                target_relative
            )
        })
    };
    if let Err(mut error) = move_result {
        // The target never held anything but our partial copy (or it was renamed to the backup).
        if status_move.rewrite_id.is_some() && status_move.target_path.exists() {
            if let Err(remove_err) = fs::remove_file(&status_move.target_path) {
                error.push_str(&format!(
                    "；清理未完成目标失败 {}: {remove_err}",
                    status_move.target_path.display()
                ));
            }
        }
        if let Some(backup_path) = &overwrite_backup {
            if status_move.target_path.exists() {
                if let Err(remove_err) = fs::remove_file(&status_move.target_path) {
                    error.push_str(&format!(
                        "；清理未完成目标失败 {}: {remove_err}",
                        status_move.target_path.display()
                    ));
                }
            }
            if let Err(restore_err) = fs::rename(backup_path, &status_move.target_path) {
                error.push_str(&format!(
                    "；恢复原目标会话失败 {}: {restore_err}（备份保留于 {}）",
                    status_move.target_path.display(),
                    backup_path.display()
                ));
            }
        }
        return Err(error);
    }
    Ok(AppliedStatusMove {
        status_move: status_move.clone(),
        overwrite_backup,
    })
}

fn rollback_status_move_file(applied: &AppliedStatusMove) -> Result<(), String> {
    let status_move = &applied.status_move;
    if status_move.rewrite_id.is_some() {
        fs::remove_file(&status_move.target_path).map_err(|err| {
            format!(
                "撤销移动失败，无法删除修改 ID 后的副本 {}: {err}",
                status_move.target_path.display()
            )
        })?;
    } else {
        if let Some(parent) = status_move.source_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!("撤销移动失败，无法创建原目录 {}: {err}", parent.display())
            })?;
        }
        fs::rename(&status_move.target_path, &status_move.source_path).map_err(|err| {
            format!(
                "撤销移动失败 {} -> {}: {err}",
                status_move.target_path.display(),
                status_move.source_path.display()
            )
        })?;
    }
    if let Some(backup_path) = &applied.overwrite_backup {
        fs::rename(backup_path, &status_move.target_path).map_err(|err| {
            format!(
                "撤销移动失败，无法恢复被覆盖的会话 {}: {err}（备份保留于 {}）",
                status_move.target_path.display(),
                backup_path.display()
            )
        })?;
    }
    Ok(())
}

fn finalize_status_move_file(root: &Path, applied: &AppliedStatusMove) -> Vec<String> {
    let status_move = &applied.status_move;
    let mut errors = Vec::new();
    if status_move.rewrite_id.is_some() {
        if let Err(err) = fs::remove_file(&status_move.source_path) {
            errors.push(format!(
                "删除原会话文件失败 {}: {err}",
                status_move.source_path.display()
            ));
        }
    }
    if let Some(backup_path) = &applied.overwrite_backup {
        if let Err(err) = fs::remove_file(backup_path) {
            errors.push(format!("清理覆盖备份失败 {}: {err}", backup_path.display()));
        }
    }
    remove_empty_parent_dirs(root, status_move.source_path.parent());
    errors
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
