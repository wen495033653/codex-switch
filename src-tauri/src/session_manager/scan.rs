use super::*;

pub(super) fn scan_conversations_impl(root: Option<String>) -> Result<Value, String> {
    let root = resolve_codex_root(root.as_deref())?;
    validate_codex_root(&root)?;
    let desktop_threads = list_interactive_threads(&root)
        .map_err(|err| format!("通过 Codex Desktop 查询会话失败: {err}"))?;
    let (mut conversations, warnings, errors) =
        conversations_from_desktop_threads(&root, desktop_threads);

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

pub(super) fn conversations_from_desktop_threads(
    root: &Path,
    desktop_threads: Vec<CodexDesktopThread>,
) -> (Vec<ConversationItem>, Vec<String>, Vec<String>) {
    let mut conversations = Vec::with_capacity(desktop_threads.len());
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut seen_paths = HashSet::new();

    for thread in desktop_threads {
        let id = thread.id.clone();
        let path_key = conversation_path_key(&thread.path);
        if seen_ids.contains(&id) || seen_paths.contains(&path_key) {
            warnings.push(format!("已忽略 Codex Desktop 返回的重复会话: {id}"));
            continue;
        }

        let relative = match relative_path_under_root(root, &thread.path) {
            Some(relative) => relative,
            None => {
                errors.push(format!(
                    "Codex Desktop 返回了当前数据目录外的会话路径 {}: {}",
                    id,
                    thread.path.display()
                ));
                continue;
            }
        };
        if let Err(err) = ensure_session_relative_path(&relative) {
            errors.push(format!("Codex Desktop 返回了无效会话路径 {id}: {err}"));
            continue;
        }
        let metadata = match thread.path.metadata() {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => {
                errors.push(format!(
                    "Codex Desktop 会话路径不是文件 {}: {}",
                    id,
                    thread.path.display()
                ));
                continue;
            }
            Err(err) => {
                errors.push(format!(
                    "读取 Codex Desktop 会话文件失败 {}: {}: {err}",
                    id,
                    thread.path.display()
                ));
                continue;
            }
        };

        seen_ids.insert(id.clone());
        seen_paths.insert(path_key);

        let preview = non_empty(thread.preview);
        let title = thread
            .name
            .and_then(non_empty)
            .or_else(|| preview.clone().map(|value| truncate_text(&value, 48)))
            .unwrap_or_else(|| id.clone());
        let updated_at = thread
            .recency_at
            .or(Some(thread.updated_at))
            .and_then(timestamp_seconds_to_rfc3339)
            .or_else(|| system_time_to_rfc3339(metadata.modified().ok()));

        conversations.push(ConversationItem {
            id,
            title,
            updated_at,
            status: if thread.archived {
                "archived".to_string()
            } else {
                "active".to_string()
            },
            source_path: thread.path.to_string_lossy().to_string(),
            relative_path: path_to_slash(&relative),
            size_bytes: metadata.len(),
            cwd: Some(thread.cwd.to_string_lossy().to_string()),
            preview,
            sha256: None,
            parse_error: None,
        });
    }

    (conversations, warnings, errors)
}

pub(super) fn resolve_codex_root(root: Option<&str>) -> Result<PathBuf, String> {
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

pub(super) fn session_id_variants(session_id: &str) -> Vec<String> {
    let raw = session_id.trim();
    let bare = raw.strip_prefix("local:").unwrap_or(raw);
    let mut variants = vec![raw.to_string(), bare.to_string()];
    if !bare.is_empty() {
        variants.push(format!("local:{bare}"));
    }
    dedupe_strings(&mut variants);
    variants
}

pub(super) fn session_index_entry<'a>(
    index: &'a SessionIndex,
    session_id: &str,
) -> Option<&'a SessionIndexEntry> {
    session_id_variants(session_id)
        .into_iter()
        .find_map(|variant| index.get(&variant))
}

pub(super) fn session_index_title(index: &SessionIndex, session_id: &str) -> Option<String> {
    session_index_entry(index, session_id).and_then(|entry| entry.thread_name.clone())
}

pub(super) fn validate_codex_root(root: &Path) -> Result<(), String> {
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

pub(super) fn validate_session_file_path(root: &Path, path: &Path) -> Result<(), String> {
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

pub(super) fn read_session_index(root: &Path, warnings: &mut Vec<String>) -> SessionIndex {
    let path = root.join("session_index.jsonl");
    let mut map = HashMap::new();
    if !path.exists() {
        warnings
            .push("session_index.jsonl 不存在，已使用其他会话元数据推断标题和更新时间".to_string());
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
        let previous = session_index_entry(&map, &id).cloned();
        let entry = SessionIndexEntry {
            thread_name: thread_name.or_else(|| {
                previous
                    .as_ref()
                    .and_then(|entry| entry.thread_name.clone())
            }),
            updated_at: updated_at
                .or_else(|| previous.as_ref().and_then(|entry| entry.updated_at.clone())),
        };
        for variant in session_id_variants(&id) {
            map.insert(variant, entry.clone());
        }
    }
    map
}

pub(super) fn collect_conversation_files(
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

pub(super) fn conversation_from_path(
    root: &Path,
    path: &Path,
    status: &str,
    include_sha: bool,
    session_index: &SessionIndex,
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
    let title = session_index_title(session_index, &id)
        .or_else(|| {
            summary.title.clone().or_else(|| {
                summary
                    .first_user_message
                    .clone()
                    .map(|text| truncate_text(&text, 48))
            })
        })
        .unwrap_or_else(|| "未命名会话".to_string());
    let updated_at = summary
        .updated_at
        .clone()
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

pub(super) fn read_current_state_conversations(
    root: &Path,
    session_index: &SessionIndex,
) -> Result<CurrentStateCatalog, String> {
    let state_db = codex_state_db_path_for_root(root)?;
    if !state_db.exists() {
        return Err(format!(
            "未检测到新版 Codex 数据库 {}，请先启动新版 ChatGPT Desktop 完成初始化",
            state_db.display()
        ));
    }
    let connection = Connection::open_with_flags(
        &state_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| {
        format!(
            "打开新版 Codex state 数据库失败 {}: {err}",
            state_db.display()
        )
    })?;
    connection
        .busy_timeout(Duration::from_millis(3000))
        .map_err(|err| format!("配置新版 Codex state 数据库等待超时失败: {err}"))?;
    let Some(schema) = state_threads_schema(&connection)? else {
        return Err("新版 Codex 数据库缺少 threads 表，请更新 ChatGPT Desktop".to_string());
    };
    if CURRENT_STATE_REQUIRED_COLUMNS
        .iter()
        .any(|column| !schema.contains_key(*column))
        || !state_database_has_current_migrations(&connection)?
    {
        return Err("ChatGPT Desktop 会话数据库结构过旧，请更新到最新版本".to_string());
    }

    let mut statement = connection
        .prepare(
            "SELECT id,
                    rollout_path,
                    COALESCE(title, ''),
                    COALESCE(preview, ''),
                    COALESCE(cwd, ''),
                    COALESCE(archived, 0),
                    CASE
                      WHEN COALESCE(recency_at_ms, 0) > 0 THEN recency_at_ms
                      WHEN COALESCE(updated_at_ms, 0) > 0 THEN updated_at_ms
                      WHEN COALESCE(recency_at, 0) > 0 THEN recency_at * 1000
                      ELSE COALESCE(updated_at, 0) * 1000
                    END AS effective_updated_at_ms
             FROM threads
             WHERE rollout_path IS NOT NULL AND TRIM(rollout_path) <> ''
             ORDER BY recency_at_ms DESC, id DESC",
        )
        .map_err(|err| format!("读取新版 Codex threads 目录失败: {err}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })
        .map_err(|err| format!("查询新版 Codex threads 目录失败: {err}"))?;

    let mut conversations = Vec::new();
    let mut indexed_paths = HashSet::new();
    let mut invalid_paths = 0usize;
    let mut duplicate_paths = 0usize;
    for row in rows {
        let (id, rollout_path, title, preview, cwd, archived, updated_at_ms) =
            row.map_err(|err| format!("解析新版 Codex thread 失败: {err}"))?;
        let Some((path, relative)) = resolve_state_rollout_path(root, &rollout_path) else {
            invalid_paths += 1;
            continue;
        };
        let Ok(metadata) = path.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let path_key = conversation_path_key(&path);
        if !indexed_paths.insert(path_key) {
            duplicate_paths += 1;
            continue;
        }
        let relative_path = path_to_slash(&relative);
        let preview = non_empty(preview);
        conversations.push(ConversationItem {
            id: id.clone(),
            title: session_index_title(session_index, &id)
                .or_else(|| non_empty(title))
                .or_else(|| preview.clone().map(|value| truncate_text(&value, 48)))
                .unwrap_or_else(|| id.clone()),
            updated_at: timestamp_millis_to_rfc3339(updated_at_ms)
                .or_else(|| system_time_to_rfc3339(metadata.modified().ok())),
            status: if archived == 0 {
                "active".to_string()
            } else {
                "archived".to_string()
            },
            source_path: path.to_string_lossy().to_string(),
            relative_path,
            size_bytes: metadata.len(),
            cwd: non_empty(cwd),
            preview,
            sha256: None,
            parse_error: None,
        });
    }

    let mut warnings = Vec::new();
    if invalid_paths > 0 {
        warnings.push(format!(
            "已忽略 {invalid_paths} 条不属于当前 Codex 数据目录的新版索引"
        ));
    }
    if duplicate_paths > 0 {
        warnings.push(format!("已忽略 {duplicate_paths} 条重复的新版会话索引"));
    }
    Ok(CurrentStateCatalog {
        conversations,
        warnings,
    })
}

pub(super) fn current_state_conversation_for_path(
    root: &Path,
    path: &Path,
    session_index: &SessionIndex,
) -> Result<Option<ConversationItem>, String> {
    let target = conversation_path_key(path);
    let catalog = read_current_state_conversations(root, session_index)?;
    Ok(catalog
        .conversations
        .into_iter()
        .find(|item| conversation_path_key(Path::new(&item.source_path)) == target))
}

pub(super) fn resolve_state_rollout_path(
    root: &Path,
    rollout_path: &str,
) -> Option<(PathBuf, PathBuf)> {
    let raw = rollout_path.trim();
    if raw.is_empty() {
        return None;
    }
    let raw = raw.strip_prefix(r"\\?\").unwrap_or(raw);
    let candidate = PathBuf::from(raw);
    let (path, relative) = if candidate.is_absolute() {
        let relative = relative_path_under_root(root, &candidate)?;
        (candidate, relative)
    } else {
        let normalized = normalize_relative_path(&path_to_slash(&candidate)).ok()?;
        (root.join(&normalized), normalized)
    };
    ensure_session_relative_path(&relative).ok()?;
    Some((path, relative))
}

pub(super) fn relative_path_under_root(root: &Path, path: &Path) -> Option<PathBuf> {
    if let Ok(relative) = path.strip_prefix(root) {
        return Some(relative.to_path_buf());
    }
    if path.exists() {
        let canonical_root = root.canonicalize().ok()?;
        let canonical_path = path.canonicalize().ok()?;
        if let Ok(relative) = canonical_path.strip_prefix(canonical_root) {
            return Some(relative.to_path_buf());
        }
    }
    if cfg!(windows) {
        let root_text = path_to_slash(root).trim_end_matches('/').to_string();
        let path_text = path_to_slash(path);
        if path_text.len() > root_text.len()
            && path_text[..root_text.len()].eq_ignore_ascii_case(&root_text)
            && path_text.as_bytes().get(root_text.len()) == Some(&b'/')
        {
            return normalize_relative_path(&path_text[root_text.len() + 1..]).ok();
        }
    }
    None
}

pub(super) fn conversation_path_key(path: &Path) -> String {
    normalized_path_identity(path)
}

pub(super) fn normalized_path_identity(path: &Path) -> String {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut value = resolved.to_string_lossy().replace('\\', "/");
    if let Some(rest) = value.strip_prefix("//?/UNC/") {
        value = format!("//{rest}");
    } else if let Some(rest) = value.strip_prefix("//?/") {
        value = rest.to_string();
    }
    while value.len() > 1 && value.ends_with('/') {
        value.pop();
    }
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}
