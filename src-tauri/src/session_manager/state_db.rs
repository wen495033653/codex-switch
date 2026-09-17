use super::*;

pub(super) fn thread_metadata_from_manifest(
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
        source: summary.source.clone().unwrap_or_else(|| "cli".to_string()),
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
        cli_version: summary.cli_version.clone().unwrap_or_default(),
        first_user_message: summary.first_user_message.clone().unwrap_or_default(),
        agent_nickname: summary.agent_nickname.clone(),
        agent_role: summary.agent_role.clone(),
        model: summary.model.clone(),
        reasoning_effort: summary.reasoning_effort.clone(),
        agent_path: summary.agent_path.clone(),
        thread_source: summary.thread_source.clone(),
        preview: summary.preview.clone().unwrap_or_default(),
        history_mode: summary
            .history_mode
            .clone()
            .unwrap_or_else(|| "legacy".to_string()),
        parent_thread_id: summary.parent_thread_id.clone(),
        dynamic_tools: summary.dynamic_tools.clone(),
    }
}

pub(super) fn upsert_state_threads(root: &Path, items: &[ThreadMetadata]) -> Result<usize, String> {
    write_state_threads(root, items, false)
}

pub(super) fn insert_missing_state_threads(
    root: &Path,
    items: &[ThreadMetadata],
) -> Result<usize, String> {
    write_state_threads(root, items, true)
}

pub(super) fn write_state_threads(
    root: &Path,
    items: &[ThreadMetadata],
    insert_only: bool,
) -> Result<usize, String> {
    if items.is_empty() {
        return Ok(0);
    }
    let state_db = codex_state_db_path_for_root(root)?;
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
    let update_clause = if insert_only || update_columns.is_empty() {
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
    let has_thread_spawn_edges = state_table_has_columns(
        &connection,
        "thread_spawn_edges",
        &["parent_thread_id", "child_thread_id", "status"],
    )?;
    let has_thread_dynamic_tools = state_table_has_columns(
        &connection,
        "thread_dynamic_tools",
        &[
            "thread_id",
            "position",
            "name",
            "description",
            "input_schema",
            "defer_loading",
            "namespace",
        ],
    )?;
    let transaction = connection
        .transaction()
        .map_err(|err| format!("开始 Codex state 索引事务失败: {err}"))?;
    let mut updated = 0usize;
    for item in items {
        let values = insert_columns
            .iter()
            .map(|column| thread_metadata_sql_value(item, column))
            .collect::<Vec<_>>();
        let affected = transaction
            .execute(&sql, params_from_iter(values.iter()))
            .map_err(|err| format!("更新 Codex Desktop threads 索引失败: {err}"))?;
        updated += affected;
        if !insert_only || affected > 0 {
            sync_thread_spawn_edge(&transaction, item, has_thread_spawn_edges)?;
            sync_thread_dynamic_tools(&transaction, item, has_thread_dynamic_tools)?;
        }
    }
    transaction
        .commit()
        .map_err(|err| format!("保存 Codex Desktop threads 索引失败: {err}"))?;
    Ok(updated)
}

pub(super) const THREAD_METADATA_COLUMNS: &[&str] = &[
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
    "agent_nickname",
    "agent_role",
    "memory_mode",
    "model",
    "reasoning_effort",
    "agent_path",
    "created_at_ms",
    "updated_at_ms",
    "thread_source",
    "preview",
    "recency_at",
    "recency_at_ms",
    "history_mode",
];

pub(super) const THREAD_METADATA_UPDATE_COLUMNS: &[&str] = &[
    "rollout_path",
    "source",
    "updated_at",
    "model_provider",
    "cwd",
    "title",
    "archived",
    "archived_at",
    "cli_version",
    "first_user_message",
    "agent_nickname",
    "agent_role",
    "model",
    "reasoning_effort",
    "agent_path",
    "updated_at_ms",
    "thread_source",
    "preview",
    "recency_at",
    "recency_at_ms",
    "history_mode",
];

#[derive(Debug)]
pub(super) struct StateThreadColumn {
    name: String,
    not_null: bool,
    default_value: Option<String>,
    primary_key: bool,
}

pub(super) fn thread_metadata_supported_column(column: &str) -> bool {
    THREAD_METADATA_COLUMNS.contains(&column)
}

pub(super) fn thread_metadata_sql_value(
    item: &ThreadMetadata,
    column: &str,
) -> rusqlite::types::Value {
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
        "cli_version" => SqlValue::Text(item.cli_version.clone()),
        "first_user_message" => SqlValue::Text(item.first_user_message.clone()),
        "agent_nickname" => item
            .agent_nickname
            .clone()
            .map(SqlValue::Text)
            .unwrap_or(SqlValue::Null),
        "agent_role" => item
            .agent_role
            .clone()
            .map(SqlValue::Text)
            .unwrap_or(SqlValue::Null),
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
        "agent_path" => item
            .agent_path
            .clone()
            .map(SqlValue::Text)
            .unwrap_or(SqlValue::Null),
        "created_at_ms" => SqlValue::Integer(item.created_at.saturating_mul(1000)),
        "updated_at_ms" => SqlValue::Integer(item.updated_at.saturating_mul(1000)),
        "thread_source" => item
            .thread_source
            .clone()
            .map(SqlValue::Text)
            .unwrap_or(SqlValue::Null),
        "preview" => SqlValue::Text(item.preview.clone()),
        "recency_at" => SqlValue::Integer(item.updated_at),
        "recency_at_ms" => SqlValue::Integer(item.updated_at.saturating_mul(1000)),
        "history_mode" => SqlValue::Text(item.history_mode.clone()),
        _ => SqlValue::Null,
    }
}

pub(super) fn sync_thread_spawn_edge(
    transaction: &rusqlite::Transaction<'_>,
    item: &ThreadMetadata,
    enabled: bool,
) -> Result<(), String> {
    if !enabled {
        return Ok(());
    }
    transaction
        .execute(
            "DELETE FROM thread_spawn_edges WHERE child_thread_id = ?1",
            [&item.id],
        )
        .map_err(|err| format!("清理 Codex thread parent 索引失败: {err}"))?;
    let Some(parent_thread_id) = item.parent_thread_id.as_deref().and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    }) else {
        return Ok(());
    };
    transaction
        .execute(
            "INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status)
             VALUES (?1, ?2, 'closed')
             ON CONFLICT(child_thread_id) DO UPDATE SET
               parent_thread_id = excluded.parent_thread_id,
               status = excluded.status",
            params![parent_thread_id, item.id],
        )
        .map_err(|err| format!("更新 Codex thread parent 索引失败: {err}"))?;
    Ok(())
}

pub(super) fn sync_thread_dynamic_tools(
    transaction: &rusqlite::Transaction<'_>,
    item: &ThreadMetadata,
    enabled: bool,
) -> Result<(), String> {
    if !enabled {
        return Ok(());
    }
    transaction
        .execute(
            "DELETE FROM thread_dynamic_tools WHERE thread_id = ?1",
            [&item.id],
        )
        .map_err(|err| format!("清理 Codex thread dynamic tools 失败: {err}"))?;
    for (position, tool) in item.dynamic_tools.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO thread_dynamic_tools
                 (thread_id, position, name, description, input_schema, defer_loading, namespace)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    item.id,
                    position as i64,
                    tool.name,
                    tool.description,
                    tool.input_schema,
                    i64::from(tool.defer_loading),
                    tool.namespace
                ],
            )
            .map_err(|err| format!("更新 Codex thread dynamic tools 失败: {err}"))?;
    }
    Ok(())
}

pub(super) fn update_state_thread_status(
    root: &Path,
    moves: &[StatusMove],
    target_status: &str,
) -> Result<Option<PathBuf>, String> {
    if moves.is_empty() {
        return Ok(None);
    }
    let state_db = codex_state_db_path_for_root(root)?;
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

pub(super) fn delete_state_threads_for_sessions(
    root: &Path,
    ids: &[String],
    rollout_paths: &[PathBuf],
) -> Result<(), String> {
    let state_db = codex_state_db_path_for_root(root)?;
    if !state_db.exists() {
        return Ok(());
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
        return Ok(());
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
        return Ok(());
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

    backup_state_database_for_delete(&connection, root)?;
    let transaction = connection
        .transaction()
        .map_err(|err| format!("开始 Codex state 删除事务失败: {err}"))?;
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
        transaction
            .execute("DELETE FROM threads WHERE id = ?1", [id])
            .map_err(|err| format!("删除 Codex Desktop threads 索引失败: {err}"))?;
    }
    transaction
        .commit()
        .map_err(|err| format!("保存 Codex Desktop threads 删除结果失败: {err}"))?;
    let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    Ok(())
}

pub(super) fn rollout_path_lookup_values(root: &Path, path: &Path) -> Vec<String> {
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

pub(super) fn state_threads_schema(
    connection: &Connection,
) -> Result<Option<HashMap<String, StateThreadColumn>>, String> {
    state_threads_schema_for(connection, "main")
}

pub(super) fn state_threads_schema_for(
    connection: &Connection,
    schema: &str,
) -> Result<Option<HashMap<String, StateThreadColumn>>, String> {
    let schema_identifier = quote_sqlite_identifier(schema);
    let exists = connection
        .query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM {schema_identifier}.sqlite_master WHERE type = 'table' AND name = 'threads')"
            ),
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| format!("检查 Codex Desktop threads 表失败: {err}"))?;
    if exists == 0 {
        return Ok(None);
    }

    let mut statement = connection
        .prepare(&format!("PRAGMA {schema_identifier}.table_info(threads)"))
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

pub(super) fn state_threads_has_columns(
    connection: &Connection,
    required: &[&str],
) -> Result<bool, String> {
    let Some(columns) = state_threads_schema(connection)? else {
        return Ok(false);
    };
    Ok(required.iter().all(|column| columns.contains_key(*column)))
}

pub(super) fn state_table_has_columns(
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
