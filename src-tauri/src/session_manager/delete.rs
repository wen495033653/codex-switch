use super::*;

pub(super) fn delete_conversations_impl(
    root: String,
    relative_paths: Vec<String>,
) -> Result<Value, String> {
    let root = resolve_codex_root(Some(&root))?;
    validate_codex_root(&root)?;
    if relative_paths.is_empty() {
        return Err("请先选择要删除的会话".to_string());
    }
    let deleted_root = deleted_sessions_dir()?;
    let _io_guard = lock_codex_session_io("删除会话")?;
    delete_conversations_locked(&root, relative_paths, &deleted_root)
}

pub(super) fn delete_conversations_locked(
    root: &Path,
    relative_paths: Vec<String>,
    deleted_root: &Path,
) -> Result<Value, String> {
    let deleted_at = now_string();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let session_index = read_session_index(root, &mut warnings);
    let mut candidates = Vec::new();
    let mut seen_paths = HashSet::new();

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
        let relative_key = path_to_slash(&relative);
        if !seen_paths.insert(relative_key.clone()) {
            continue;
        }
        let source_path = root.join(&relative);
        if !source_path.exists() {
            errors.push(format!("会话文件不存在: {}", relative.display()));
            continue;
        }
        if let Err(err) = validate_session_file_path(root, &source_path) {
            errors.push(err);
            continue;
        }
        let summary = parse_session_file_for_list(&source_path).unwrap_or_default();
        let id = summary
            .id
            .clone()
            .or_else(|| extract_uuid_like(&relative_key))
            .unwrap_or_else(|| relative_key.clone());
        let title = session_index_title(&session_index, &id)
            .unwrap_or_else(|| conversation_title_from_summary(&summary));
        let updated_at = session_index_entry(&session_index, &id)
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
            summary,
        });
    }

    let mut deleted_records = Vec::new();
    let mut rollout_paths = Vec::new();
    for candidate in candidates {
        let original_status = match status_from_relative_path(&candidate.relative_path) {
            Ok(status) => status,
            Err(err) => {
                errors.push(err);
                continue;
            }
        };
        let mut record = match save_deleted_session_record(
            deleted_root,
            root,
            &candidate,
            &original_status,
            &deleted_at,
        ) {
            Ok(record) => record,
            Err(err) => {
                errors.push(err);
                continue;
            }
        };
        let record_dir = match deleted_session_record_dir_at(deleted_root, &record.delete_id) {
            Ok(path) => path,
            Err(err) => {
                errors.push(err);
                continue;
            }
        };
        let expected_sha = record.sha256.as_deref().unwrap_or_default();
        let source_sha = match sha256_file(&candidate.source_path) {
            Ok(value) => value,
            Err(err) => {
                errors.push(format!(
                    "删除前复核会话失败 {}: {err}",
                    candidate.relative_path.display()
                ));
                discard_uncommitted_deleted_record(&record_dir, &mut errors);
                continue;
            }
        };
        if source_sha != expected_sha {
            errors.push(format!(
                "删除前会话内容发生变化，已保留原文件: {}",
                candidate.relative_path.display()
            ));
            discard_uncommitted_deleted_record(&record_dir, &mut errors);
            continue;
        }
        match fs::remove_file(&candidate.source_path) {
            Ok(()) => {
                remove_empty_parent_dirs(root, candidate.source_path.parent());
                match mark_deleted_session_ready(&record_dir) {
                    Ok(()) => record.state = "ready".to_string(),
                    Err(err) => warnings.push(format!(
                        "会话已删除且回收站备份完整，但写入 ready 标记失败；仍可恢复: {err}"
                    )),
                }
                rollout_paths.push(candidate.source_path.clone());
                deleted_records.push(record);
            }
            Err(err) => {
                errors.push(format!(
                    "删除会话文件失败 {}: {err}",
                    candidate.relative_path.display()
                ));
                discard_uncommitted_deleted_record(&record_dir, &mut errors);
            }
        }
    }

    let delete_ids = deleted_records
        .iter()
        .map(|record| record.delete_id.clone())
        .collect::<Vec<_>>();
    let mut removed_ids = deleted_records
        .iter()
        .flat_map(|record| session_id_variants(&record.id))
        .collect::<Vec<_>>();
    dedupe_strings(&mut removed_ids);

    let desktop_error = if deleted_records.is_empty() {
        None
    } else {
        delete_state_threads_for_sessions(root, &removed_ids, &rollout_paths).err()
    };
    let global_state_error = if deleted_records.is_empty() {
        None
    } else {
        remove_from_global_state(root, &removed_ids, "delete").err()
    };
    if let Some(err) = &desktop_error {
        warnings.push(format!(
            "Codex Desktop state 清理失败，已删除会话仍保留在回收站: {err}"
        ));
    }
    if let Some(err) = &global_state_error {
        warnings.push(format!(
            "Codex global state 清理失败，已删除会话仍保留在回收站: {err}"
        ));
    }

    Ok(json!({
        "ok": errors.is_empty(),
        "message": if errors.is_empty() {
            format!("已删除 {} 个会话", delete_ids.len())
        } else {
            format!("已删除 {} 个会话，{} 个失败", delete_ids.len(), errors.len())
        },
        "delete_ids": delete_ids.clone(),
        "report": {
            "deleted": delete_ids.len(),
            "delete_ids": delete_ids,
            "soft_deleted": deleted_records.len(),
            "desktop_error": desktop_error,
            "global_state_error": global_state_error,
            "failed": errors.len(),
            "errors": errors,
            "warnings": warnings
        }
    }))
}

pub(super) fn list_deleted_sessions_impl() -> Result<Value, String> {
    let deleted_root = deleted_sessions_dir()?;
    list_deleted_sessions_from_dir(&deleted_root)
}

pub(super) fn list_deleted_sessions_from_dir(deleted_root: &Path) -> Result<Value, String> {
    let (mut records, errors) = read_deleted_session_records_from_dir(deleted_root)?;
    for record in &mut records {
        if should_rebuild_deleted_title(&record.title) {
            let record_dir = deleted_session_record_dir_at(deleted_root, &record.delete_id)?;
            if let Ok(session_file) = deleted_record_session_path(&record_dir, record) {
                if let Ok(summary) = parse_session_file_for_list(&session_file) {
                    record.title = conversation_title_from_summary(&summary);
                    record.updated_at = record.updated_at.clone().or(summary.updated_at);
                    record.cwd = record.cwd.clone().or(summary.cwd);
                }
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
        "ok": errors.is_empty(),
        "deleted": records,
        "errors": errors
    }))
}

pub(super) fn restore_deleted_sessions_impl(
    _root: String,
    delete_ids: Vec<String>,
    conflict_strategy: Option<String>,
) -> Result<Value, String> {
    if delete_ids.is_empty() {
        return Err("请先选择要恢复的会话".to_string());
    }
    let conflict_strategy = parse_conflict_strategy(conflict_strategy)?;
    let deleted_root = deleted_sessions_dir()?;
    let _io_guard = lock_codex_session_io("恢复会话")?;
    restore_deleted_sessions_locked(&deleted_root, delete_ids, conflict_strategy)
}

pub(super) fn restore_deleted_sessions_locked(
    deleted_root: &Path,
    delete_ids: Vec<String>,
    conflict_strategy: ConflictStrategy,
) -> Result<Value, String> {
    let mut candidates = Vec::new();
    let mut conflicts = Vec::new();
    let mut errors = Vec::new();
    let mut skipped = 0usize;
    let mut seen_delete_ids = HashSet::new();
    let mut reserved_targets = HashSet::new();

    for delete_id in delete_ids {
        if !seen_delete_ids.insert(delete_id.clone()) {
            continue;
        }
        let mut candidate =
            match build_restore_deleted_candidate(deleted_root, &delete_id, conflict_strategy) {
                Ok(Some(candidate)) => candidate,
                Ok(None) => {
                    skipped += 1;
                    continue;
                }
                Err(err) => {
                    if let Some(target) = err.strip_prefix("CONFLICT:") {
                        conflicts.push(json!({
                            "delete_id": delete_id,
                            "target": target
                        }));
                    } else {
                        errors.push(err);
                    }
                    continue;
                }
            };

        let mut target_key = conversation_path_key(&candidate.target_path);
        if reserved_targets.contains(&target_key) {
            match conflict_strategy {
                ConflictStrategy::Ask => {
                    conflicts.push(json!({
                        "delete_id": delete_id,
                        "target": path_to_slash(&candidate.target_relative)
                    }));
                    continue;
                }
                ConflictStrategy::Skip => {
                    skipped += 1;
                    continue;
                }
                ConflictStrategy::Overwrite => {
                    errors.push(format!(
                        "同一批恢复包含重复目标，已跳过以避免覆盖刚恢复的会话: {}",
                        candidate.target_path.display()
                    ));
                    continue;
                }
                ConflictStrategy::ModifyId => {
                    reassign_restore_candidate(&mut candidate, &reserved_targets)?;
                    target_key = conversation_path_key(&candidate.target_path);
                }
            }
        }
        reserved_targets.insert(target_key);
        candidates.push(candidate);
    }

    if !conflicts.is_empty() && conflict_strategy == ConflictStrategy::Ask {
        return Ok(json!({
            "ok": true,
            "message": format!("发现 {} 个恢复冲突", conflicts.len()),
            "report": {
                "restored": 0,
                "restored_delete_ids": [],
                "skipped": skipped,
                "conflict_action_required": true,
                "operation": "restore",
                "conflicts": conflicts,
                "failed": errors.len(),
                "errors": errors,
                "warnings": []
            }
        }));
    }

    let mut restored_delete_ids = Vec::new();
    let mut trash_retained = Vec::new();
    let mut sqlite_updated = 0usize;
    let mut warnings = Vec::new();
    for candidate in candidates {
        let delete_id = candidate.record.delete_id.clone();
        match restore_deleted_candidate(candidate, conflict_strategy) {
            Ok((updated, trash_removed, mut candidate_warnings)) => {
                sqlite_updated += updated;
                restored_delete_ids.push(delete_id.clone());
                if !trash_removed {
                    trash_retained.push(delete_id);
                }
                warnings.append(&mut candidate_warnings);
            }
            Err(err) => errors.push(err),
        }
    }

    Ok(json!({
        "ok": errors.is_empty(),
        "message": if errors.is_empty() {
            format!("已恢复 {} 个会话", restored_delete_ids.len())
        } else {
            format!("已恢复 {} 个会话，{} 个失败", restored_delete_ids.len(), errors.len())
        },
        "report": {
            "restored": restored_delete_ids.len(),
            "restored_delete_ids": restored_delete_ids,
            "trash_retained": trash_retained,
            "skipped": skipped,
            "conflict_action_required": false,
            "operation": "restore",
            "conflicts": conflicts,
            "failed": errors.len(),
            "errors": errors,
            "warnings": warnings,
            "sqlite_updated": sqlite_updated,
            "sqlite_error": null
        }
    }))
}

pub(super) fn purge_deleted_sessions_impl(delete_ids: Vec<String>) -> Result<Value, String> {
    if delete_ids.is_empty() {
        return Err("请先选择要彻底删除的会话".to_string());
    }
    let deleted_root = deleted_sessions_dir()?;
    let _io_guard = lock_codex_session_io("彻底删除会话")?;
    purge_deleted_sessions_locked(&deleted_root, delete_ids)
}

pub(super) fn purge_deleted_sessions_locked(
    deleted_root: &Path,
    delete_ids: Vec<String>,
) -> Result<Value, String> {
    let mut purged_delete_ids = Vec::new();
    let mut errors = Vec::new();
    let mut seen = HashSet::new();
    for delete_id in delete_ids {
        if !seen.insert(delete_id.clone()) {
            continue;
        }
        let result = deleted_session_record_dir_at(deleted_root, &delete_id).and_then(|dir| {
            if !dir.exists() {
                return Err(format!("已删除会话不存在: {delete_id}"));
            }
            fs::remove_dir_all(&dir).map_err(|err| format!("彻底删除失败 {}: {err}", dir.display()))
        });
        match result {
            Ok(()) => purged_delete_ids.push(delete_id),
            Err(err) => errors.push(err),
        }
    }
    Ok(json!({
        "ok": errors.is_empty(),
        "message": if errors.is_empty() {
            format!("已彻底删除 {} 个会话", purged_delete_ids.len())
        } else {
            format!("已彻底删除 {} 个会话，{} 个失败", purged_delete_ids.len(), errors.len())
        },
        "report": {
            "purged": purged_delete_ids.len(),
            "purged_delete_ids": purged_delete_ids,
            "failed": errors.len(),
            "errors": errors
        }
    }))
}

pub(super) fn remove_from_global_state(
    root: &Path,
    ids: &[String],
    reason: &str,
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let path = root.join(".codex-global-state.json");
    if !path.exists() {
        return Ok(());
    }
    let id_set: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("读取 .codex-global-state.json 失败: {err}"))?;
    let mut value: Value = serde_json::from_str(&content)
        .map_err(|err| format!("解析 .codex-global-state.json 失败: {err}"))?;
    let removed = remove_matching_object_keys(&mut value, &id_set);
    if removed == 0 {
        return Ok(());
    }
    backup_file_with_reason(&path, reason)?;
    let mut output = serde_json::to_string_pretty(&value)
        .map_err(|err| format!("序列化 .codex-global-state.json 失败: {err}"))?;
    output.push('\n');
    fs::write(&path, output).map_err(|err| format!("写入 .codex-global-state.json 失败: {err}"))?;
    Ok(())
}

pub(super) fn remove_matching_object_keys(value: &mut Value, ids: &HashSet<&str>) -> usize {
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
            for (key, value) in map.iter_mut() {
                if matches!(key.as_str(), "pinned-thread-ids" | "pinnedThreadIds") {
                    if let Value::Array(items) = value {
                        let before = items.len();
                        items.retain(|item| item.as_str().is_none_or(|id| !ids.contains(id)));
                        removed += before.saturating_sub(items.len());
                    }
                } else {
                    removed += remove_matching_object_keys(value, ids);
                }
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

pub(super) fn conversation_title_from_summary(summary: &SessionSummary) -> String {
    summary
        .title
        .clone()
        .or_else(|| summary.first_user_message.clone())
        .map(|value| truncate_text(&value, 80))
        .unwrap_or_else(|| "未命名会话".to_string())
}
