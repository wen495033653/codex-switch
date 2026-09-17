use super::*;

pub(crate) fn migrate_legacy_codex_data_for_current_home() -> Result<Value, String> {
    let root = codex_dir()?;
    migrate_legacy_codex_data_for_root(&root)
}

pub(crate) fn migrate_legacy_codex_data_for_root(root: &Path) -> Result<Value, String> {
    let marker = codex_desktop_migration_marker_path(root)?;
    if let Some(report) = read_completed_codex_desktop_migration(&marker)? {
        return Ok(report);
    }

    let legacy_state_db = legacy_codex_state_db_path_from_home(root);
    let current_state_db = codex_state_db_path_for_root(root)?;
    if normalized_path_identity(&legacy_state_db) == normalized_path_identity(&current_state_db) {
        return Err("新版 Codex 数据库路径不能与旧版 nested 数据库相同".to_string());
    }
    if !current_state_db.exists() {
        return Ok(json!({
            "ok": false,
            "completed": false,
            "migrationVersion": CODEX_DESKTOP_MIGRATION_VERSION,
            "action": "waiting_for_new_desktop_database",
            "message": "请先启动一次新版 ChatGPT Desktop，初始化新版 Codex 数据库后再迁移",
            "root": root.to_string_lossy(),
            "source": legacy_state_db.to_string_lossy(),
            "target": current_state_db.to_string_lossy()
        }));
    }

    let _io_guard = lock_codex_session_io("迁移旧版 Codex 数据")?;
    if let Some(report) = read_completed_codex_desktop_migration(&marker)? {
        return Ok(report);
    }
    let mut current_connection = Connection::open_with_flags(
        &current_state_db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| {
        format!(
            "打开新版 Codex state 数据库失败 {}: {err}",
            current_state_db.display()
        )
    })?;
    current_connection
        .busy_timeout(Duration::from_millis(5000))
        .map_err(|err| format!("配置新版 Codex state 数据库等待超时失败: {err}"))?;
    let Some(current_schema) = state_threads_schema(&current_connection)? else {
        return Ok(waiting_for_current_state_schema(
            root,
            &legacy_state_db,
            &current_state_db,
        ));
    };
    if CURRENT_STATE_REQUIRED_COLUMNS
        .iter()
        .any(|column| !current_schema.contains_key(*column))
        || !state_database_has_current_migrations(&current_connection)?
    {
        return Ok(waiting_for_current_state_schema(
            root,
            &legacy_state_db,
            &current_state_db,
        ));
    }

    if !legacy_state_db.exists() {
        validate_state_database_connection(&current_connection, &current_state_db)?;
        let report = json!({
            "ok": true,
            "completed": true,
            "migrationVersion": CODEX_DESKTOP_MIGRATION_VERSION,
            "action": "no_legacy_data",
            "root": root.to_string_lossy(),
            "target": current_state_db.to_string_lossy(),
            "completedAt": now_string()
        });
        write_codex_desktop_migration_marker(&marker, &report)?;
        return Ok(report);
    }

    let backup_path =
        backup_state_database_with_reason(&current_connection, "desktop-final-v2-migration")?;
    let inserted =
        merge_legacy_state_metadata(&mut current_connection, &legacy_state_db, &current_schema)?;
    let existing_thread_ids = read_state_thread_ids(&current_connection)?;
    drop(current_connection);

    let (missing_thread_metadata, rollout_errors, rollout_files, rollout_skipped_existing) =
        collect_thread_metadata_for_migration(root, &existing_thread_ids);
    let rollout_indexed = insert_missing_state_threads(root, &missing_thread_metadata)?;
    validate_state_database(&current_state_db)?;

    let report = json!({
        "ok": true,
        "completed": true,
        "migrationVersion": CODEX_DESKTOP_MIGRATION_VERSION,
        "action": "migrated_to_chatgpt_desktop",
        "root": root.to_string_lossy(),
        "source": legacy_state_db.to_string_lossy(),
        "target": current_state_db.to_string_lossy(),
        "backup": backup_path.to_string_lossy(),
        "inserted": inserted,
        "metadataOnlyRows": inserted.get("threads").copied().unwrap_or(0),
        "rolloutFiles": rollout_files,
        "rolloutRowsIndexed": rollout_indexed,
        "rolloutRowsSkippedExisting": rollout_skipped_existing,
        "rolloutErrors": rollout_errors,
        "completedAt": now_string()
    });
    write_codex_desktop_migration_marker(&marker, &report)?;
    Ok(report)
}

pub(super) fn read_state_thread_ids(connection: &Connection) -> Result<HashSet<String>, String> {
    let mut statement = connection
        .prepare("SELECT id FROM threads")
        .map_err(|err| format!("读取新版 Codex thread id 失败: {err}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|err| format!("查询新版 Codex thread id 失败: {err}"))?;
    rows.collect::<Result<HashSet<_>, _>>()
        .map_err(|err| format!("解析新版 Codex thread id 失败: {err}"))
}

pub(super) fn waiting_for_current_state_schema(root: &Path, source: &Path, target: &Path) -> Value {
    json!({
        "ok": false,
        "completed": false,
        "migrationVersion": CODEX_DESKTOP_MIGRATION_VERSION,
        "action": "waiting_for_new_desktop_schema",
        "message": "新版 Codex 数据库尚未完成初始化，请打开新版 ChatGPT Desktop 后重试",
        "root": root.to_string_lossy(),
        "source": source.to_string_lossy(),
        "target": target.to_string_lossy()
    })
}

pub(super) fn merge_legacy_state_metadata(
    current_connection: &mut Connection,
    legacy_state_db: &Path,
    current_schema: &HashMap<String, StateThreadColumn>,
) -> Result<HashMap<String, usize>, String> {
    current_connection
        .execute(
            "ATTACH DATABASE ?1 AS legacy",
            params![legacy_state_db.to_string_lossy()],
        )
        .map_err(|err| {
            format!(
                "挂载旧版 Codex state 数据库失败 {}: {err}",
                legacy_state_db.display()
            )
        })?;
    let result = (|| {
        let legacy_schema = state_threads_schema_for(&*current_connection, "legacy")?
            .ok_or_else(|| "旧版 Codex state 数据库缺少 threads 表".to_string())?;
        let mut insert_columns = legacy_schema
            .keys()
            .filter(|column| current_schema.contains_key(*column))
            .cloned()
            .collect::<Vec<_>>();
        insert_columns.sort();
        let mut select_expressions = insert_columns
            .iter()
            .map(|column| format!("legacy_thread.{}", quote_sqlite_identifier(column)))
            .collect::<Vec<_>>();

        for (column, expression) in [
            ("recency_at", "legacy_thread.updated_at".to_string()),
            (
                "recency_at_ms",
                if legacy_schema.contains_key("updated_at_ms") {
                    "COALESCE(legacy_thread.updated_at_ms, legacy_thread.updated_at * 1000)"
                        .to_string()
                } else {
                    "legacy_thread.updated_at * 1000".to_string()
                },
            ),
            ("history_mode", "'legacy'".to_string()),
        ] {
            if current_schema.contains_key(column)
                && !insert_columns.iter().any(|item| item == column)
            {
                insert_columns.push(column.to_string());
                select_expressions.push(expression);
            }
        }
        if !insert_columns.iter().any(|column| column == "id") {
            return Err("旧版 Codex state 数据库 threads 表缺少 id".to_string());
        }
        let columns = insert_columns
            .iter()
            .map(|column| quote_sqlite_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "INSERT OR IGNORE INTO main.threads ({columns}) SELECT {} FROM legacy.threads AS legacy_thread
             WHERE legacy_thread.id IN (SELECT id FROM codex_switch_migrated_thread_ids)",
            select_expressions.join(", ")
        );
        let transaction = current_connection
            .transaction()
            .map_err(|err| format!("开始旧版 Codex 数据迁移事务失败: {err}"))?;
        transaction
            .execute_batch(
                "CREATE TEMP TABLE IF NOT EXISTS codex_switch_migrated_thread_ids (id TEXT PRIMARY KEY);
                 DELETE FROM codex_switch_migrated_thread_ids;
                 INSERT OR IGNORE INTO codex_switch_migrated_thread_ids (id)
                 SELECT legacy_thread.id FROM legacy.threads AS legacy_thread
                 WHERE NOT EXISTS (
                   SELECT 1 FROM main.threads AS current_thread
                   WHERE current_thread.id = legacy_thread.id
                 );",
            )
            .map_err(|err| format!("准备旧版 Codex threads 迁移失败: {err}"))?;
        let mut inserted = HashMap::new();
        let thread_count = transaction
            .execute(&sql, [])
            .map_err(|err| format!("迁移旧版 Codex threads metadata 失败: {err}"))?;
        inserted.insert("threads".to_string(), thread_count);
        inserted.insert(
            "thread_spawn_edges".to_string(),
            merge_legacy_table_rows(
                &transaction,
                "thread_spawn_edges",
                Some("child_thread_id IN (SELECT id FROM codex_switch_migrated_thread_ids)"),
            )?,
        );
        inserted.insert(
            "thread_dynamic_tools".to_string(),
            merge_legacy_table_rows(
                &transaction,
                "thread_dynamic_tools",
                Some("thread_id IN (SELECT id FROM codex_switch_migrated_thread_ids)"),
            )?,
        );
        let (jobs, job_items) = merge_legacy_agent_jobs(&transaction)?;
        inserted.insert("agent_jobs".to_string(), jobs);
        inserted.insert("agent_job_items".to_string(), job_items);
        transaction
            .execute_batch("DROP TABLE codex_switch_migrated_thread_ids;")
            .map_err(|err| format!("清理旧版 Codex threads 迁移临时表失败: {err}"))?;
        transaction
            .commit()
            .map_err(|err| format!("保存旧版 Codex threads metadata 失败: {err}"))?;
        Ok(inserted)
    })();
    let detach_result = current_connection.execute_batch("DETACH DATABASE legacy");
    match (result, detach_result) {
        (Ok(inserted), Ok(())) => Ok(inserted),
        (Err(err), _) => Err(err),
        (Ok(_), Err(err)) => Err(format!("卸载旧版 Codex state 数据库失败: {err}")),
    }
}

pub(super) fn merge_legacy_table_rows(
    transaction: &rusqlite::Transaction<'_>,
    table: &str,
    where_clause: Option<&str>,
) -> Result<usize, String> {
    let Some(current_columns) = state_table_columns_for(transaction, "main", table)? else {
        return Ok(0);
    };
    let Some(legacy_columns) = state_table_columns_for(transaction, "legacy", table)? else {
        return Ok(0);
    };
    let current_set = current_columns
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let common_columns = legacy_columns
        .into_iter()
        .filter(|column| current_set.contains(column.as_str()))
        .collect::<Vec<_>>();
    if common_columns.is_empty() {
        return Ok(0);
    }
    let columns = common_columns
        .iter()
        .map(|column| quote_sqlite_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let where_sql = where_clause
        .map(|value| format!(" WHERE {value}"))
        .unwrap_or_default();
    let table_identifier = quote_sqlite_identifier(table);
    let sql = format!(
        "INSERT OR IGNORE INTO main.{table_identifier} ({columns}) SELECT {columns} FROM legacy.{table_identifier}{where_sql}"
    );
    transaction
        .execute(&sql, [])
        .map_err(|err| format!("迁移旧版 Codex 表 {table} 失败: {err}"))
}

pub(super) fn merge_legacy_agent_jobs(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(usize, usize), String> {
    if state_table_columns_for(transaction, "main", "agent_jobs")?.is_none()
        || state_table_columns_for(transaction, "legacy", "agent_jobs")?.is_none()
    {
        return Ok((0, 0));
    }
    transaction
        .execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS codex_switch_migrated_job_ids (id TEXT PRIMARY KEY);
             DELETE FROM codex_switch_migrated_job_ids;
             INSERT OR IGNORE INTO codex_switch_migrated_job_ids (id)
             SELECT legacy.id FROM legacy.agent_jobs AS legacy
             WHERE NOT EXISTS (SELECT 1 FROM main.agent_jobs AS current WHERE current.id = legacy.id);",
        )
        .map_err(|err| format!("准备旧版 Codex agent jobs 迁移失败: {err}"))?;
    let jobs = merge_legacy_table_rows(
        transaction,
        "agent_jobs",
        Some("id IN (SELECT id FROM codex_switch_migrated_job_ids)"),
    )?;
    let items = merge_legacy_table_rows(
        transaction,
        "agent_job_items",
        Some("job_id IN (SELECT id FROM codex_switch_migrated_job_ids)"),
    )?;
    transaction
        .execute_batch("DROP TABLE codex_switch_migrated_job_ids;")
        .map_err(|err| format!("清理旧版 Codex agent jobs 迁移临时表失败: {err}"))?;
    Ok((jobs, items))
}

pub(super) fn state_table_columns_for(
    connection: &Connection,
    schema: &str,
    table: &str,
) -> Result<Option<Vec<String>>, String> {
    let schema_identifier = quote_sqlite_identifier(schema);
    let exists = connection
        .query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM {schema_identifier}.sqlite_master WHERE type = 'table' AND name = ?1)"
            ),
            [table],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| format!("检查 Codex state 表 {schema}.{table} 失败: {err}"))?;
    if exists == 0 {
        return Ok(None);
    }
    let mut statement = connection
        .prepare(&format!(
            "PRAGMA {schema_identifier}.table_info({})",
            quote_sqlite_identifier(table)
        ))
        .map_err(|err| format!("读取 Codex state 表结构 {schema}.{table} 失败: {err}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| format!("查询 Codex state 表结构 {schema}.{table} 失败: {err}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map(Some)
        .map_err(|err| format!("解析 Codex state 表结构 {schema}.{table} 失败: {err}"))
}

pub(super) fn collect_thread_metadata_for_migration(
    root: &Path,
    existing_thread_ids: &HashSet<String>,
) -> (Vec<ThreadMetadata>, Vec<String>, usize, usize) {
    let mut warnings = Vec::new();
    let index = read_session_index(root, &mut warnings);
    let mut errors = warnings;
    let mut files = Vec::new();
    collect_conversation_files(&root.join("sessions"), "active", &mut files, &mut errors);
    collect_conversation_files(
        &root.join("archived_sessions"),
        "archived",
        &mut files,
        &mut errors,
    );
    let rollout_files = files.len();
    let mut skipped_existing = 0usize;
    let mut items = Vec::new();
    for (status, path) in files {
        if extract_uuid_like(&path.to_string_lossy()).is_some_and(|id| {
            session_id_variants(&id)
                .iter()
                .any(|variant| existing_thread_ids.contains(variant))
        }) {
            skipped_existing += 1;
            continue;
        }
        let summary = match parse_session_file_for_list(&path) {
            Ok(summary) => summary,
            Err(err) => {
                errors.push(err);
                continue;
            }
        };
        let Some(id) = summary
            .id
            .clone()
            .or_else(|| extract_uuid_like(&path.to_string_lossy()))
        else {
            errors.push(format!("迁移时无法识别会话 ID: {}", path.display()));
            continue;
        };
        if session_id_variants(&id)
            .iter()
            .any(|variant| existing_thread_ids.contains(variant))
        {
            skipped_existing += 1;
            continue;
        }
        let index_entry = session_index_entry(&index, &id);
        let title = index_entry
            .and_then(|entry| entry.thread_name.clone())
            .or_else(|| summary.title.clone())
            .or_else(|| summary.first_user_message.clone())
            .map(|value| truncate_text(&value, 80))
            .unwrap_or_else(|| "未命名会话".to_string());
        let relative_path = path
            .strip_prefix(root)
            .map(path_to_slash)
            .unwrap_or_else(|_| path.to_string_lossy().to_string());
        let session = ManifestSession {
            id,
            title,
            updated_at: index_entry
                .and_then(|entry| entry.updated_at.clone())
                .or_else(|| summary.updated_at.clone())
                .or_else(|| {
                    path.metadata()
                        .ok()
                        .and_then(|metadata| system_time_to_rfc3339(metadata.modified().ok()))
                }),
            status,
            relative_path,
            size_bytes: path.metadata().map(|metadata| metadata.len()).unwrap_or(0),
            sha256: String::new(),
        };
        items.push(thread_metadata_from_manifest(&session, &path, &summary));
    }
    (items, errors, rollout_files, skipped_existing)
}

pub(super) fn validate_state_database(path: &Path) -> Result<(), String> {
    let connection = Connection::open(path).map_err(|err| {
        format!(
            "打开迁移后的 Codex state 数据库失败 {}: {err}",
            path.display()
        )
    })?;
    validate_state_database_connection(&connection, path)
}

pub(super) fn validate_state_database_connection(
    connection: &Connection,
    path: &Path,
) -> Result<(), String> {
    let result: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|err| {
            format!(
                "校验迁移后的 Codex state 数据库失败 {}: {err}",
                path.display()
            )
        })?;
    if !result.eq_ignore_ascii_case("ok") {
        return Err(format!(
            "迁移后的 Codex state 数据库校验失败 {}: {result}",
            path.display()
        ));
    }
    let mut statement = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(|err| format!("检查 Codex state 外键失败 {}: {err}", path.display()))?;
    let mut rows = statement
        .query([])
        .map_err(|err| format!("查询 Codex state 外键失败 {}: {err}", path.display()))?;
    if rows
        .next()
        .map_err(|err| format!("读取 Codex state 外键检查失败 {}: {err}", path.display()))?
        .is_some()
    {
        return Err(format!(
            "迁移后的 Codex state 数据库存在外键异常: {}",
            path.display()
        ));
    }
    Ok(())
}

pub(super) fn state_database_has_current_migrations(
    connection: &Connection,
) -> Result<bool, String> {
    let has_table = connection
        .query_row(
            "SELECT EXISTS(
               SELECT 1 FROM sqlite_master
               WHERE type = 'table' AND name = '_sqlx_migrations'
             )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| format!("检查 Codex SQLx migration 表失败: {err}"))?;
    if has_table == 0 {
        return Ok(false);
    }
    let (max_version, failed): (i64, i64) = connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0),
                    COALESCE(SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END), 0)
             FROM _sqlx_migrations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|err| format!("读取 Codex SQLx migration 状态失败: {err}"))?;
    Ok(max_version >= CURRENT_STATE_MIN_SQLX_MIGRATION && failed == 0)
}

pub(super) fn codex_desktop_migration_marker_path(root: &Path) -> Result<PathBuf, String> {
    let identity = normalized_path_identity(root);
    let digest = Sha256::digest(identity.as_bytes());
    let key = &hex_bytes(&digest)[..16];
    Ok(app_data_dir()?
        .join(CODEX_DESKTOP_MIGRATION_DIR)
        .join(format!("{CODEX_DESKTOP_MIGRATION_FILE_PREFIX}-{key}.json")))
}

pub(super) fn read_completed_codex_desktop_migration(path: &Path) -> Result<Option<Value>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)
        .map_err(|err| format!("读取 Codex 数据迁移标记失败 {}: {err}", path.display()))?;
    let Ok(value) = serde_json::from_str::<Value>(&content) else {
        return Ok(None);
    };
    let completed = value.get("completed").and_then(Value::as_bool) == Some(true);
    let version = value.get("migrationVersion").and_then(Value::as_u64);
    if completed && version == Some(u64::from(CODEX_DESKTOP_MIGRATION_VERSION)) {
        Ok(Some(value))
    } else {
        Ok(None)
    }
}

pub(super) fn write_codex_desktop_migration_marker(
    path: &Path,
    report: &Value,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("创建 Codex 数据迁移目录失败 {}: {err}", parent.display()))?;
    }
    let temp_path = path.with_extension("tmp");
    let mut content = serde_json::to_string_pretty(report)
        .map_err(|err| format!("序列化 Codex 数据迁移标记失败: {err}"))?;
    content.push('\n');
    fs::write(&temp_path, content)
        .map_err(|err| format!("写入 Codex 数据迁移标记失败 {}: {err}", temp_path.display()))?;
    if path.exists() {
        fs::remove_file(path)
            .map_err(|err| format!("替换 Codex 数据迁移标记失败 {}: {err}", path.display()))?;
    }
    fs::rename(&temp_path, path)
        .map_err(|err| format!("保存 Codex 数据迁移标记失败 {}: {err}", path.display()))
}
