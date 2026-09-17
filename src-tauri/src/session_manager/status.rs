use super::*;

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
        let overwrite_backup_path = if status_move.target_path.exists()
            && conflict_strategy == ConflictStrategy::Overwrite
        {
            let backup_path =
                status_overwrite_backup_path(&status_move.target_path, &status_move.id);
            if let Err(err) = fs::rename(&status_move.target_path, &backup_path) {
                errors.push(format!("备份覆盖目标会话失败 {}: {err}", target_relative));
                continue;
            }
            Some(backup_path)
        } else {
            None
        };
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
            let mut error = err.to_string();
            if let Some(backup_path) = &overwrite_backup_path {
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
                        "；恢复原目标会话失败 {}: {restore_err}",
                        status_move.target_path.display()
                    ));
                }
            }
            errors.push(error);
            continue;
        }
        if let Some(backup_path) = &overwrite_backup_path {
            if let Some(id) = &status_move.overwritten_id {
                overwritten_ids.push(id.clone());
            }
            if let Err(err) = fs::remove_file(backup_path) {
                errors.push(format!("清理覆盖备份失败 {}: {err}", backup_path.display()));
            }
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
        let _ = delete_state_threads_for_sessions(&root, &overwritten_ids, &[]);
        let _ = remove_from_global_state(&root, &overwritten_ids, "status-overwrite");
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
            "state_backup_path": state_backup_path.map(|path| path.to_string_lossy().to_string()),
            "desktop_error": desktop_error,
            "conflicts": conflicts,
            "failed": errors.len(),
            "errors": errors
        }
    }))
}
