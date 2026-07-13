use crate::{
    codex_app_server::{list_interactive_threads, CodexDesktopThread},
    codex_sessions::lock_codex_session_io,
    json_util::raw_string_field,
    paths::{
        app_data_dir, codex_dir, codex_state_db_path_for_root, legacy_codex_state_db_path_from_home,
    },
    time_util::{now_string, parse_rfc3339_seconds},
};
use rusqlite::{params, params_from_iter, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
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
const CODEX_DESKTOP_MIGRATION_VERSION: u32 = 2;
const CODEX_DESKTOP_MIGRATION_DIR: &str = "migrations";
const CODEX_DESKTOP_MIGRATION_FILE_PREFIX: &str = "codex-chatgpt-desktop-final-v2";
const CURRENT_STATE_MIN_SQLX_MIGRATION: i64 = 40;
const PREVIEW_MESSAGE_LIMIT_DEFAULT: usize = 80;
const PREVIEW_MESSAGE_LIMIT_MAX: usize = 200;
const PREVIEW_REVERSE_READ_BLOCK_BYTES: usize = 64 * 1024;
const PREVIEW_CANCELLED_ERROR: &str = "会话预览请求已取消";
const DELETED_SESSIONS_DIR: &str = "deleted-sessions";
const CURRENT_STATE_REQUIRED_COLUMNS: &[&str] = &[
    "id",
    "rollout_path",
    "title",
    "cwd",
    "archived",
    "updated_at",
    "updated_at_ms",
    "preview",
    "recency_at",
    "recency_at_ms",
    "history_mode",
];
static LATEST_PREVIEW_REQUEST_ID: AtomicU64 = AtomicU64::new(0);

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    #[serde(default = "default_deleted_session_state")]
    state: String,
}

#[derive(Debug, Clone, Serialize)]
struct ConversationMessage {
    role: String,
    text: String,
    timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<u64>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PreviewMessageSource {
    Event,
    Response,
}

impl PreviewMessageSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Event => "event",
            Self::Response => "response",
        }
    }
}

#[derive(Debug)]
struct PreviewMessagePage {
    messages: Vec<ConversationMessage>,
    source: PreviewMessageSource,
    next_before: Option<u64>,
    has_more: bool,
    file_size: u64,
}

#[derive(Debug)]
struct CurrentStateCatalog {
    conversations: Vec<ConversationItem>,
    warnings: Vec<String>,
}

#[derive(Debug, Default, Clone)]
struct SessionSummary {
    id: Option<String>,
    title: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    cwd: Option<String>,
    source: Option<String>,
    thread_source: Option<String>,
    model_provider: Option<String>,
    sandbox_policy: Option<String>,
    approval_mode: Option<String>,
    cli_version: Option<String>,
    agent_nickname: Option<String>,
    agent_role: Option<String>,
    agent_path: Option<String>,
    history_mode: Option<String>,
    parent_thread_id: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
    first_user_message: Option<String>,
    preview: Option<String>,
    dynamic_tools: Vec<ThreadDynamicToolMetadata>,
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
    cli_version: String,
    first_user_message: String,
    agent_nickname: Option<String>,
    agent_role: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
    agent_path: Option<String>,
    thread_source: Option<String>,
    preview: String,
    history_mode: String,
    parent_thread_id: Option<String>,
    dynamic_tools: Vec<ThreadDynamicToolMetadata>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ThreadDynamicToolMetadata {
    name: String,
    description: String,
    input_schema: String,
    defer_loading: bool,
    namespace: Option<String>,
}

#[derive(Debug, Clone)]
struct StatusMove {
    id: String,
    target_id: String,
    source_path: PathBuf,
    target_path: PathBuf,
    rewrite_id: Option<(String, String)>,
    overwritten_id: Option<String>,
}

#[derive(Debug, Clone)]
struct DeleteCandidate {
    id: String,
    title: String,
    updated_at: Option<String>,
    source_path: PathBuf,
    relative_path: PathBuf,
    summary: SessionSummary,
}

#[derive(Debug, Clone)]
struct RestoreCandidate {
    record: DeletedSessionRecord,
    record_dir: PathBuf,
    source_file: PathBuf,
    root: PathBuf,
    target_path: PathBuf,
    target_relative: PathBuf,
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
struct SessionIndexEntry {
    thread_name: Option<String>,
    updated_at: Option<String>,
}

type SessionIndex = HashMap<String, SessionIndexEntry>;

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
    before_cursor: Option<u64>,
    snapshot_size: Option<u64>,
    limit: Option<usize>,
    message_source: Option<String>,
    request_id: Option<u64>,
) -> Result<Value, String> {
    begin_preview_request(request_id);
    tauri::async_runtime::spawn_blocking(move || {
        preview_conversation_impl(
            root,
            relative_path,
            before_cursor,
            snapshot_size,
            limit,
            message_source,
            request_id,
        )
    })
    .await
    .map_err(|err| blocking_task_error("读取预览", err))?
}

#[tauri::command]
pub(crate) async fn session_manager_preview_deleted(
    delete_id: String,
    before_cursor: Option<u64>,
    snapshot_size: Option<u64>,
    limit: Option<usize>,
    message_source: Option<String>,
    request_id: Option<u64>,
) -> Result<Value, String> {
    begin_preview_request(request_id);
    tauri::async_runtime::spawn_blocking(move || {
        preview_deleted_conversation_impl(
            delete_id,
            before_cursor,
            snapshot_size,
            limit,
            message_source,
            request_id,
        )
    })
    .await
    .map_err(|err| blocking_task_error("读取已删除预览", err))?
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

fn read_state_thread_ids(connection: &Connection) -> Result<HashSet<String>, String> {
    let mut statement = connection
        .prepare("SELECT id FROM threads")
        .map_err(|err| format!("读取新版 Codex thread id 失败: {err}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|err| format!("查询新版 Codex thread id 失败: {err}"))?;
    rows.collect::<Result<HashSet<_>, _>>()
        .map_err(|err| format!("解析新版 Codex thread id 失败: {err}"))
}

fn waiting_for_current_state_schema(root: &Path, source: &Path, target: &Path) -> Value {
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

fn merge_legacy_state_metadata(
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

fn merge_legacy_table_rows(
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

fn merge_legacy_agent_jobs(
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

fn state_table_columns_for(
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

fn collect_thread_metadata_for_migration(
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

fn validate_state_database(path: &Path) -> Result<(), String> {
    let connection = Connection::open(path).map_err(|err| {
        format!(
            "打开迁移后的 Codex state 数据库失败 {}: {err}",
            path.display()
        )
    })?;
    validate_state_database_connection(&connection, path)
}

fn validate_state_database_connection(connection: &Connection, path: &Path) -> Result<(), String> {
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

fn state_database_has_current_migrations(connection: &Connection) -> Result<bool, String> {
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

fn codex_desktop_migration_marker_path(root: &Path) -> Result<PathBuf, String> {
    let identity = normalized_path_identity(root);
    let digest = Sha256::digest(identity.as_bytes());
    let key = &hex_bytes(&digest)[..16];
    Ok(app_data_dir()?
        .join(CODEX_DESKTOP_MIGRATION_DIR)
        .join(format!("{CODEX_DESKTOP_MIGRATION_FILE_PREFIX}-{key}.json")))
}

fn read_completed_codex_desktop_migration(path: &Path) -> Result<Option<Value>, String> {
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

fn write_codex_desktop_migration_marker(path: &Path, report: &Value) -> Result<(), String> {
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

fn scan_conversations_impl(root: Option<String>) -> Result<Value, String> {
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

fn conversations_from_desktop_threads(
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

fn preview_conversation_impl(
    root: String,
    relative_path: String,
    before_cursor: Option<u64>,
    snapshot_size: Option<u64>,
    limit: Option<usize>,
    message_source: Option<String>,
    request_id: Option<u64>,
) -> Result<Value, String> {
    let root = resolve_codex_root(Some(&root))?;
    validate_codex_root(&root)?;
    let relative = normalize_relative_path(&relative_path)?;
    ensure_session_relative_path(&relative)?;
    let path = root.join(&relative);
    if !path.exists() {
        return Err(format!("会话文件不存在: {}", relative.display()));
    }

    let mut index_warnings = Vec::new();
    let session_index = read_session_index(&root, &mut index_warnings);
    let status = status_from_relative_path(&relative)?;
    let item = current_state_conversation_for_path(&root, &path, &session_index)?.unwrap_or(
        conversation_from_path(&root, &path, &status, false, &session_index)?,
    );
    let page = read_preview_message_page(
        &path,
        before_cursor,
        snapshot_size,
        limit,
        message_source.as_deref(),
        request_id,
    )?;

    Ok(json!({
        "ok": true,
        "conversation": item,
        "messages": page.messages,
        "message_page": {
            "source": page.source.as_str(),
            "next_before": page.next_before,
            "has_more": page.has_more,
            "file_size": page.file_size,
            "limit": normalize_preview_limit(limit)
        },
        "warnings": [],
        "parse_error": null
    }))
}

fn preview_deleted_conversation_impl(
    delete_id: String,
    before_cursor: Option<u64>,
    snapshot_size: Option<u64>,
    limit: Option<usize>,
    message_source: Option<String>,
    request_id: Option<u64>,
) -> Result<Value, String> {
    let deleted_root = deleted_sessions_dir()?;
    preview_deleted_conversation_from_dir(
        &deleted_root,
        &delete_id,
        before_cursor,
        snapshot_size,
        limit,
        message_source.as_deref(),
        request_id,
    )
}

fn preview_deleted_conversation_from_dir(
    deleted_root: &Path,
    delete_id: &str,
    before_cursor: Option<u64>,
    snapshot_size: Option<u64>,
    limit: Option<usize>,
    message_source: Option<&str>,
    request_id: Option<u64>,
) -> Result<Value, String> {
    let record_dir = deleted_session_record_dir_at(deleted_root, delete_id)?;
    let record = read_deleted_session_record(&record_dir)?;
    validate_deleted_record_identity(delete_id, &record)?;
    let record = recover_deleted_session_record_state(&record_dir, record)?
        .ok_or_else(|| "删除操作尚未完成，原会话文件仍然存在".to_string())?;
    let session_file = deleted_record_session_path(&record_dir, &record)?;
    if !session_file.exists() {
        return Err(format!("已删除会话备份文件缺失: {}", record.title));
    }
    verify_deleted_session_backup(&record, &session_file)?;
    let summary = parse_session_file_for_list(&session_file).unwrap_or_default();
    let size_bytes = session_file
        .metadata()
        .map(|item| item.len())
        .unwrap_or(record.size_bytes);
    let title = if should_rebuild_deleted_title(&record.title) {
        conversation_title_from_summary(&summary)
    } else {
        record.title.clone()
    };
    let conversation = ConversationItem {
        id: record.id.clone(),
        title,
        updated_at: record
            .updated_at
            .clone()
            .or(Some(record.deleted_at.clone())),
        status: "deleted".to_string(),
        source_path: session_file.to_string_lossy().to_string(),
        relative_path: record.original_relative_path.clone(),
        size_bytes,
        cwd: summary.cwd.clone().or(record.cwd.clone()),
        preview: summary.preview.clone(),
        sha256: record.sha256.clone(),
        parse_error: summary.parse_error.clone(),
    };
    let page = read_preview_message_page(
        &session_file,
        before_cursor,
        snapshot_size,
        limit,
        message_source,
        request_id,
    )?;

    Ok(json!({
        "ok": true,
        "conversation": conversation,
        "messages": page.messages,
        "message_page": {
            "source": page.source.as_str(),
            "next_before": page.next_before,
            "has_more": page.has_more,
            "file_size": page.file_size,
            "limit": normalize_preview_limit(limit)
        },
        "warnings": [],
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
    let session_index = read_session_index(&root, &mut warnings);
    let catalog = read_current_state_conversations(&root, &session_index)?;
    warnings.extend(catalog.warnings);
    let state_by_path = catalog
        .conversations
        .into_iter()
        .map(|item| (conversation_path_key(Path::new(&item.source_path)), item))
        .collect::<HashMap<_, _>>();
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
        let item = state_by_path
            .get(&conversation_path_key(&path))
            .cloned()
            .map(|mut item| {
                item.sha256 = sha256_file(&path).ok();
                item
            })
            .map(Ok)
            .unwrap_or_else(|| conversation_from_path(&root, &path, &status, true, &session_index));
        match item {
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

fn delete_conversations_impl(root: String, relative_paths: Vec<String>) -> Result<Value, String> {
    let root = resolve_codex_root(Some(&root))?;
    validate_codex_root(&root)?;
    if relative_paths.is_empty() {
        return Err("请先选择要删除的会话".to_string());
    }
    let deleted_root = deleted_sessions_dir()?;
    let _io_guard = lock_codex_session_io("删除会话")?;
    delete_conversations_locked(&root, relative_paths, &deleted_root)
}

fn delete_conversations_locked(
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

fn list_deleted_sessions_impl() -> Result<Value, String> {
    let deleted_root = deleted_sessions_dir()?;
    list_deleted_sessions_from_dir(&deleted_root)
}

fn list_deleted_sessions_from_dir(deleted_root: &Path) -> Result<Value, String> {
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

fn restore_deleted_sessions_impl(
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

fn restore_deleted_sessions_locked(
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

fn purge_deleted_sessions_impl(delete_ids: Vec<String>) -> Result<Value, String> {
    if delete_ids.is_empty() {
        return Err("请先选择要彻底删除的会话".to_string());
    }
    let deleted_root = deleted_sessions_dir()?;
    let _io_guard = lock_codex_session_io("彻底删除会话")?;
    purge_deleted_sessions_locked(&deleted_root, delete_ids)
}

fn purge_deleted_sessions_locked(
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

fn session_index_entry<'a>(
    index: &'a SessionIndex,
    session_id: &str,
) -> Option<&'a SessionIndexEntry> {
    session_id_variants(session_id)
        .into_iter()
        .find_map(|variant| index.get(&variant))
}

fn session_index_title(index: &SessionIndex, session_id: &str) -> Option<String> {
    session_index_entry(index, session_id).and_then(|entry| entry.thread_name.clone())
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

fn read_session_index(root: &Path, warnings: &mut Vec<String>) -> SessionIndex {
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

fn read_current_state_conversations(
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

fn current_state_conversation_for_path(
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

fn resolve_state_rollout_path(root: &Path, rollout_path: &str) -> Option<(PathBuf, PathBuf)> {
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

fn relative_path_under_root(root: &Path, path: &Path) -> Option<PathBuf> {
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

fn conversation_path_key(path: &Path) -> String {
    normalized_path_identity(path)
}

fn normalized_path_identity(path: &Path) -> String {
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

fn parse_session_file_for_list(path: &Path) -> Result<SessionSummary, String> {
    parse_session_file_with_limit(path, false, Some(240))
}

#[cfg(test)]
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
            set_first(&mut summary.source, session_source(payload));
            set_first(
                &mut summary.thread_source,
                non_empty(raw_string_field(payload, "thread_source")),
            );
            set_first(
                &mut summary.model_provider,
                non_empty(raw_string_field(payload, "model_provider")),
            );
            set_first(
                &mut summary.cli_version,
                non_empty(raw_string_field(payload, "cli_version")),
            );
            set_first(
                &mut summary.agent_nickname,
                non_empty(raw_string_field(payload, "agent_nickname")),
            );
            set_first(
                &mut summary.agent_role,
                non_empty(raw_string_field(payload, "agent_role")),
            );
            set_first(
                &mut summary.agent_path,
                non_empty(raw_string_field(payload, "agent_path")),
            );
            set_first(
                &mut summary.history_mode,
                non_empty(raw_string_field(payload, "history_mode")),
            );
            set_first(
                &mut summary.parent_thread_id,
                session_parent_thread_id(payload),
            );
            if summary.dynamic_tools.is_empty() {
                summary.dynamic_tools = session_dynamic_tools(payload);
            }
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

fn begin_preview_request(request_id: Option<u64>) {
    if let Some(request_id) = request_id.filter(|value| *value > 0) {
        LATEST_PREVIEW_REQUEST_ID.store(request_id, Ordering::Release);
    }
}

fn preview_request_cancelled(request_id: Option<u64>) -> bool {
    request_id
        .filter(|value| *value > 0)
        .is_some_and(|request_id| LATEST_PREVIEW_REQUEST_ID.load(Ordering::Acquire) != request_id)
}

fn ensure_preview_request_current(request_id: Option<u64>) -> Result<(), String> {
    if preview_request_cancelled(request_id) {
        Err(PREVIEW_CANCELLED_ERROR.to_string())
    } else {
        Ok(())
    }
}

fn normalize_preview_limit(limit: Option<usize>) -> usize {
    limit
        .unwrap_or(PREVIEW_MESSAGE_LIMIT_DEFAULT)
        .clamp(1, PREVIEW_MESSAGE_LIMIT_MAX)
}

fn parse_preview_message_source(
    source: Option<&str>,
) -> Result<Option<PreviewMessageSource>, String> {
    match source.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some("event") => Ok(Some(PreviewMessageSource::Event)),
        Some("response") => Ok(Some(PreviewMessageSource::Response)),
        Some(other) => Err(format!("不支持的会话消息来源: {other}")),
    }
}

fn read_preview_message_page(
    path: &Path,
    before_cursor: Option<u64>,
    snapshot_size: Option<u64>,
    limit: Option<usize>,
    message_source: Option<&str>,
    request_id: Option<u64>,
) -> Result<PreviewMessagePage, String> {
    ensure_preview_request_current(request_id)?;
    let limit = normalize_preview_limit(limit);
    let source = parse_preview_message_source(message_source)?;
    if let Some(source) = source {
        return scan_preview_message_source(
            path,
            before_cursor,
            snapshot_size,
            limit,
            source,
            request_id,
        );
    }

    let event_page = scan_preview_message_source(
        path,
        before_cursor,
        snapshot_size,
        limit,
        PreviewMessageSource::Event,
        request_id,
    )?;
    if !event_page.messages.is_empty() {
        return Ok(event_page);
    }
    scan_preview_message_source(
        path,
        before_cursor,
        snapshot_size,
        limit,
        PreviewMessageSource::Response,
        request_id,
    )
}

fn scan_preview_message_source(
    path: &Path,
    before_cursor: Option<u64>,
    snapshot_size: Option<u64>,
    limit: usize,
    source: PreviewMessageSource,
    request_id: Option<u64>,
) -> Result<PreviewMessagePage, String> {
    let current_file_size = fs::metadata(path)
        .map_err(|err| format!("读取会话文件信息失败 {}: {err}", path.display()))?
        .len();
    if snapshot_size.is_some_and(|snapshot| current_file_size < snapshot) {
        return Err("会话文件已变化，请重新加载最新内容".to_string());
    }
    let file_size = snapshot_size.unwrap_or(current_file_size);
    let mut position = before_cursor.unwrap_or(file_size).min(file_size);
    let mut file = fs::File::open(path)
        .map_err(|err| format!("读取会话文件失败 {}: {err}", path.display()))?;
    let mut carry = Vec::new();
    let mut matches = Vec::<(u64, ConversationMessage)>::new();

    'scan: while position > 0 {
        ensure_preview_request_current(request_id)?;
        let start = position.saturating_sub(PREVIEW_REVERSE_READ_BLOCK_BYTES as u64);
        let block_len = usize::try_from(position - start)
            .map_err(|_| format!("会话文件分段长度无效: {}", path.display()))?;
        let mut block = vec![0u8; block_len];
        file.seek(SeekFrom::Start(start))
            .map_err(|err| format!("定位会话文件失败 {}: {err}", path.display()))?;
        file.read_exact(&mut block)
            .map_err(|err| format!("分段读取会话文件失败 {}: {err}", path.display()))?;
        block.extend_from_slice(&carry);

        let mut segment_end = block.len();
        for newline in (0..block.len())
            .rev()
            .filter(|index| block[*index] == b'\n')
        {
            let segment_start = newline + 1;
            if segment_start < segment_end {
                let line_offset = start.saturating_add(segment_start as u64);
                if let Some(message) = preview_message_from_jsonl_line(
                    &block[segment_start..segment_end],
                    source,
                    line_offset,
                ) {
                    matches.push((line_offset, message));
                    if matches.len() > limit {
                        break 'scan;
                    }
                }
            }
            segment_end = newline;
        }

        if start == 0 {
            if segment_end > 0 {
                if let Some(message) =
                    preview_message_from_jsonl_line(&block[..segment_end], source, 0)
                {
                    matches.push((0, message));
                }
            }
            position = 0;
        } else {
            carry.clear();
            carry.extend_from_slice(&block[..segment_end]);
            position = start;
        }
    }

    ensure_preview_request_current(request_id)?;
    let has_more = matches.len() > limit;
    if has_more {
        matches.truncate(limit);
    }
    let next_before = has_more
        .then(|| matches.last().map(|item| item.0))
        .flatten();
    let messages = matches
        .into_iter()
        .rev()
        .map(|(_, message)| message)
        .collect();
    Ok(PreviewMessagePage {
        messages,
        source,
        next_before,
        has_more,
        file_size,
    })
}

fn preview_message_from_jsonl_line(
    line: &[u8],
    source: PreviewMessageSource,
    offset: u64,
) -> Option<ConversationMessage> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let value = serde_json::from_slice::<Value>(line).ok()?;
    let event_type = raw_string_field(&value, "type");
    let payload = value.get("payload")?;
    let payload_type = raw_string_field(payload, "type");
    let (role, text) = match source {
        PreviewMessageSource::Event if event_type == "event_msg" => {
            let role = match payload_type.as_str() {
                "user_message" => "user",
                "agent_message" => "assistant",
                _ => return None,
            };
            (role, readable_payload_text(payload)?)
        }
        PreviewMessageSource::Response
            if event_type == "response_item" && payload_type == "message" =>
        {
            let role = raw_string_field(payload, "role");
            if role != "user" && role != "assistant" {
                return None;
            }
            let text = readable_payload_text(payload)?;
            (if role == "user" { "user" } else { "assistant" }, text)
        }
        _ => return None,
    };
    Some(ConversationMessage {
        role: role.to_string(),
        text,
        timestamp: non_empty(raw_string_field(&value, "timestamp")),
        offset: Some(offset),
    })
}

fn session_source(payload: &Value) -> Option<String> {
    let source = payload.get("source")?;
    if let Some(value) = source.as_str().map(str::to_string).and_then(non_empty) {
        return Some(value);
    }
    if source.is_object() || source.is_array() {
        return serde_json::to_string(source).ok();
    }
    None
}

fn session_parent_thread_id(payload: &Value) -> Option<String> {
    first_non_empty(&[
        raw_string_field(payload, "parent_thread_id"),
        raw_string_field(payload, "forked_from_id"),
        payload
            .pointer("/source/subagent/thread_spawn/parent_thread_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    ])
}

fn session_dynamic_tools(payload: &Value) -> Vec<ThreadDynamicToolMetadata> {
    let Some(items) = payload.get("dynamic_tools").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut tools = Vec::new();
    for item in items {
        let item_type = raw_string_field(item, "type");
        if item_type == "namespace" {
            let namespace = non_empty(raw_string_field(item, "name"));
            for tool in item
                .get("tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(tool) = dynamic_tool_metadata(tool, namespace.clone()) {
                    tools.push(tool);
                }
            }
        } else if let Some(tool) = dynamic_tool_metadata(item, None) {
            tools.push(tool);
        }
    }
    tools
}

fn dynamic_tool_metadata(
    value: &Value,
    namespace: Option<String>,
) -> Option<ThreadDynamicToolMetadata> {
    if raw_string_field(value, "type") != "function" {
        return None;
    }
    let name = non_empty(raw_string_field(value, "name"))?;
    let input_schema = value
        .get("inputSchema")
        .or_else(|| value.get("input_schema"))
        .and_then(|schema| serde_json::to_string(schema).ok())
        .unwrap_or_else(|| "{}".to_string());
    Some(ThreadDynamicToolMetadata {
        name,
        description: raw_string_field(value, "description"),
        input_schema,
        defer_loading: value
            .get("deferLoading")
            .or_else(|| value.get("defer_loading"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        namespace,
    })
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
            offset: None,
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

fn remove_from_global_state(root: &Path, ids: &[String], reason: &str) -> Result<(), String> {
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

fn conversation_title_from_summary(summary: &SessionSummary) -> String {
    summary
        .title
        .clone()
        .or_else(|| summary.first_user_message.clone())
        .map(|value| truncate_text(&value, 80))
        .unwrap_or_else(|| "未命名会话".to_string())
}

fn save_deleted_session_record(
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

fn discard_uncommitted_deleted_record(record_dir: &Path, errors: &mut Vec<String>) {
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

fn read_deleted_session_records_from_dir(
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

fn read_deleted_session_record(record_dir: &Path) -> Result<DeletedSessionRecord, String> {
    let path = record_dir.join("metadata.json");
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("读取已删除会话元数据失败 {}: {err}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|err| format!("解析已删除会话元数据失败 {}: {err}", path.display()))
}

fn default_deleted_session_state() -> String {
    "ready".to_string()
}

fn mark_deleted_session_ready(record_dir: &Path) -> Result<(), String> {
    let marker = record_dir.join("ready");
    write_new_file_atomically(&marker, b"ready\n", "delete-ready")
}

fn recover_deleted_session_record_state(
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

fn validate_deleted_record_identity(
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

fn deleted_record_session_path(
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

fn verify_deleted_session_backup(
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

fn build_restore_deleted_candidate(
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

fn reassign_restore_candidate(
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

fn restore_deleted_candidate(
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

fn should_rebuild_deleted_title(title: &str) -> bool {
    title.trim().is_empty() || title == "未命名会话"
}

fn deleted_sessions_dir() -> Result<PathBuf, String> {
    Ok(session_manager_data_dir()?.join(DELETED_SESSIONS_DIR))
}

fn deleted_session_record_dir_at(deleted_root: &Path, delete_id: &str) -> Result<PathBuf, String> {
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

fn session_manager_data_dir() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join(SESSION_MANAGER_DATA_DIR))
}

fn session_manager_backup_dir(reason: &str) -> Result<PathBuf, String> {
    let reason = sanitize_backup_reason(reason);
    Ok(session_manager_data_dir()?.join("backups").join(reason))
}

fn unique_backup_id(id: &str) -> String {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{}-{}-{suffix}", backup_stamp(), sanitize_id_fragment(id))
}

fn status_overwrite_backup_path(target: &Path, id: &str) -> PathBuf {
    let file_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("session.jsonl");
    target.with_file_name(format!(
        ".{file_name}.codex-switch-overwrite-{}",
        unique_backup_id(id)
    ))
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

fn quote_sqlite_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
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

fn upsert_state_threads(root: &Path, items: &[ThreadMetadata]) -> Result<usize, String> {
    write_state_threads(root, items, false)
}

fn insert_missing_state_threads(root: &Path, items: &[ThreadMetadata]) -> Result<usize, String> {
    write_state_threads(root, items, true)
}

fn write_state_threads(
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

const THREAD_METADATA_UPDATE_COLUMNS: &[&str] = &[
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

fn sync_thread_spawn_edge(
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

fn sync_thread_dynamic_tools(
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

fn update_state_thread_status(
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

fn delete_state_threads_for_sessions(
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
    state_threads_schema_for(connection, "main")
}

fn state_threads_schema_for(
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

fn timestamp_millis_to_rfc3339(milliseconds: i64) -> Option<String> {
    let nanoseconds = i128::from(milliseconds).checked_mul(1_000_000)?;
    OffsetDateTime::from_unix_timestamp_nanos(nanoseconds)
        .ok()
        .and_then(|time| {
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
            cli_version: "0.144.1".to_string(),
            first_user_message: "hello".to_string(),
            agent_nickname: None,
            agent_role: None,
            model: Some("gpt-5.2".to_string()),
            reasoning_effort: Some("high".to_string()),
            agent_path: None,
            thread_source: Some("user".to_string()),
            preview: "hello".to_string(),
            history_mode: "legacy".to_string(),
            parent_thread_id: None,
            dynamic_tools: Vec::new(),
        }
    }

    fn create_current_state_db(root: &Path) -> Connection {
        fs::create_dir_all(root).unwrap();
        let connection = Connection::open(root.join("state_5.sqlite")).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE _sqlx_migrations (
                    version INTEGER PRIMARY KEY,
                    success INTEGER NOT NULL
                );
                INSERT INTO _sqlx_migrations (version, success) VALUES (40, 1);
                CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT NOT NULL,
                    title TEXT NOT NULL DEFAULT '',
                    cwd TEXT NOT NULL DEFAULT '',
                    archived INTEGER NOT NULL DEFAULT 0,
                    archived_at INTEGER,
                    updated_at INTEGER NOT NULL DEFAULT 0,
                    updated_at_ms INTEGER NOT NULL DEFAULT 0,
                    preview TEXT NOT NULL DEFAULT '',
                    recency_at INTEGER NOT NULL DEFAULT 0,
                    recency_at_ms INTEGER NOT NULL DEFAULT 0,
                    history_mode TEXT NOT NULL DEFAULT 'legacy'
                );
                "#,
            )
            .unwrap();
        connection
    }

    fn write_test_session(root: &Path, relative: &Path, id: &str, label: &str) -> PathBuf {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                "{}\n{}\n{}\n",
                json!({
                    "timestamp": "2026-07-13T00:00:00Z",
                    "type": "session_meta",
                    "payload": {
                        "id": id,
                        "cwd": "C:\\work",
                        "originator": "codex-switch-test"
                    }
                }),
                json!({
                    "timestamp": "2026-07-13T00:00:01Z",
                    "type": "event_msg",
                    "payload": {"type": "user_message", "message": label}
                }),
                json!({
                    "timestamp": "2026-07-13T00:00:02Z",
                    "type": "event_msg",
                    "payload": {"type": "agent_message", "message": format!("reply-{label}")}
                })
            ),
        )
        .unwrap();
        path
    }

    fn delete_test_session(root: &Path, deleted_root: &Path, relative: &Path) -> (Value, String) {
        let result =
            delete_conversations_locked(root, vec![path_to_slash(relative)], deleted_root).unwrap();
        assert_eq!(result["report"]["deleted"], 1, "{result}");
        let delete_id = result["delete_ids"][0].as_str().unwrap().to_string();
        (result, delete_id)
    }

    #[test]
    fn soft_delete_preview_and_restore_round_trip() {
        let base = temp_path("delete-restore-round-trip");
        let root = base.join("codex");
        let deleted_root = base.join("deleted-sessions");
        let relative = PathBuf::from("sessions/2026/07/13/rollout-delete-restore.jsonl");
        let session_id = "019f0000-0000-7000-8000-000000000001";
        let source = write_test_session(&root, &relative, session_id, "delete me");
        let original = fs::read(&source).unwrap();

        let (delete_result, delete_id) = delete_test_session(&root, &deleted_root, &relative);
        assert_eq!(delete_result["report"]["deleted"], 1);
        assert!(!source.exists());
        let record_dir = deleted_root.join(&delete_id);
        let record = read_deleted_session_record(&record_dir).unwrap();
        assert_eq!(record.root_path, root.to_string_lossy());
        assert_eq!(
            record.sha256.as_deref(),
            Some(sha256_bytes(&original).as_str())
        );

        let preview = preview_deleted_conversation_from_dir(
            &deleted_root,
            &delete_id,
            None,
            None,
            Some(1),
            None,
            None,
        )
        .unwrap();
        assert_eq!(preview["conversation"]["status"], "deleted");
        assert_eq!(preview["messages"].as_array().unwrap().len(), 1);
        assert_eq!(preview["message_page"]["has_more"], true);

        let restore = restore_deleted_sessions_locked(
            &deleted_root,
            vec![delete_id.clone()],
            ConflictStrategy::Ask,
        )
        .unwrap();
        assert_eq!(restore["report"]["restored"], 1);
        assert_eq!(fs::read(&source).unwrap(), original);
        assert!(!record_dir.exists());

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn purge_removes_persistent_trash_record() {
        let base = temp_path("delete-purge");
        let root = base.join("codex");
        let deleted_root = base.join("deleted-sessions");
        let relative = PathBuf::from("sessions/2026/07/13/rollout-delete-purge.jsonl");
        write_test_session(
            &root,
            &relative,
            "019f0000-0000-7000-8000-000000000002",
            "purge me",
        );
        let (_, delete_id) = delete_test_session(&root, &deleted_root, &relative);
        let record_dir = deleted_root.join(&delete_id);
        assert!(record_dir.exists());

        let result = purge_deleted_sessions_locked(&deleted_root, vec![delete_id.clone()]).unwrap();
        assert_eq!(result["report"]["purged"], 1);
        assert_eq!(result["report"]["purged_delete_ids"][0], delete_id);
        assert!(!record_dir.exists());

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn delete_cleanup_failure_keeps_verified_trash() {
        let base = temp_path("delete-cleanup-failure");
        let root = base.join("codex");
        let deleted_root = base.join("deleted-sessions");
        let relative = PathBuf::from("sessions/2026/07/13/rollout-cleanup-failure.jsonl");
        let source = write_test_session(
            &root,
            &relative,
            "019f0000-0000-7000-8000-000000000003",
            "cleanup failure",
        );
        fs::write(root.join("state_5.sqlite"), b"not sqlite").unwrap();
        fs::write(root.join(".codex-global-state.json"), b"not json").unwrap();

        let (result, delete_id) = delete_test_session(&root, &deleted_root, &relative);
        assert!(!source.exists());
        assert!(deleted_root.join(&delete_id).join("session.jsonl").exists());
        assert!(result["report"]["desktop_error"].is_string());
        assert!(result["report"]["global_state_error"].is_string());

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn prepared_record_is_hidden_until_original_file_is_gone() {
        let base = temp_path("delete-prepared-recovery");
        let root = base.join("codex");
        let deleted_root = base.join("deleted-sessions");
        let relative = PathBuf::from("sessions/2026/07/13/rollout-prepared-recovery.jsonl");
        let source = write_test_session(
            &root,
            &relative,
            "019f0000-0000-7000-8000-000000000006",
            "prepared recovery",
        );
        let (_, delete_id) = delete_test_session(&root, &deleted_root, &relative);
        let record_dir = deleted_root.join(&delete_id);
        fs::remove_file(record_dir.join("ready")).unwrap();
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"original still exists\n").unwrap();

        let hidden = list_deleted_sessions_from_dir(&deleted_root).unwrap();
        assert!(hidden["deleted"].as_array().unwrap().is_empty());
        fs::remove_file(&source).unwrap();

        let recovered = list_deleted_sessions_from_dir(&deleted_root).unwrap();
        assert_eq!(recovered["deleted"].as_array().unwrap().len(), 1);
        assert_eq!(recovered["deleted"][0]["state"], "ready");

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn restore_db_failure_rolls_back_overwrite_and_keeps_trash() {
        let base = temp_path("restore-db-failure-rollback");
        let root = base.join("codex");
        let deleted_root = base.join("deleted-sessions");
        let relative = PathBuf::from("sessions/2026/07/13/rollout-restore-rollback.jsonl");
        let source = write_test_session(
            &root,
            &relative,
            "019f0000-0000-7000-8000-000000000004",
            "trashed version",
        );
        let (_, delete_id) = delete_test_session(&root, &deleted_root, &relative);
        let replacement = b"existing target must survive\n".to_vec();
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, &replacement).unwrap();
        fs::write(root.join("state_5.sqlite"), b"not sqlite").unwrap();

        let result = restore_deleted_sessions_locked(
            &deleted_root,
            vec![delete_id.clone()],
            ConflictStrategy::Overwrite,
        )
        .unwrap();
        assert_eq!(result["report"]["restored"], 0);
        assert_eq!(result["report"]["failed"], 1);
        assert_eq!(fs::read(&source).unwrap(), replacement);
        assert!(deleted_root.join(&delete_id).join("session.jsonl").exists());

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn restore_conflict_supports_ask_skip_and_modify_id() {
        let base = temp_path("restore-conflict-strategies");
        let root = base.join("codex");
        let deleted_root = base.join("deleted-sessions");
        let relative = PathBuf::from("sessions/2026/07/13/rollout-restore-conflict.jsonl");
        let source = write_test_session(
            &root,
            &relative,
            "019f0000-0000-7000-8000-000000000005",
            "trashed conflict",
        );
        let (_, delete_id) = delete_test_session(&root, &deleted_root, &relative);
        let existing = b"existing target\n".to_vec();
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, &existing).unwrap();

        let ask = restore_deleted_sessions_locked(
            &deleted_root,
            vec![delete_id.clone()],
            ConflictStrategy::Ask,
        )
        .unwrap();
        assert_eq!(ask["report"]["conflict_action_required"], true);
        assert_eq!(fs::read(&source).unwrap(), existing);

        let skip = restore_deleted_sessions_locked(
            &deleted_root,
            vec![delete_id.clone()],
            ConflictStrategy::Skip,
        )
        .unwrap();
        assert_eq!(skip["report"]["skipped"], 1);
        assert!(deleted_root.join(&delete_id).exists());

        let modified = restore_deleted_sessions_locked(
            &deleted_root,
            vec![delete_id.clone()],
            ConflictStrategy::ModifyId,
        )
        .unwrap();
        assert_eq!(modified["report"]["restored"], 1);
        assert_eq!(fs::read(&source).unwrap(), existing);
        assert!(!deleted_root.join(&delete_id).exists());
        let mut files = Vec::new();
        let mut collect_errors = Vec::new();
        collect_conversation_files(
            &root.join("sessions"),
            "active",
            &mut files,
            &mut collect_errors,
        );
        assert!(collect_errors.is_empty());
        assert_eq!(files.len(), 2);
        let restored = files
            .iter()
            .map(|(_, path)| path)
            .find(|path| **path != source)
            .unwrap();
        let restored_summary = parse_session_file_for_list(restored).unwrap();
        assert_ne!(
            restored_summary.id.as_deref(),
            Some("019f0000-0000-7000-8000-000000000005")
        );

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn relative_path_rejects_traversal() {
        assert!(normalize_relative_path("../sessions/a.jsonl").is_err());
        assert!(normalize_relative_path("sessions/2026/05/01/a.jsonl").is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn path_identity_normalizes_windows_extended_prefix_and_case() {
        assert_eq!(
            normalized_path_identity(Path::new(r"\\?\C:\Profiles\Example\.codex")),
            normalized_path_identity(Path::new(r"c:\profiles\example\.codex"))
        );
        assert_eq!(
            normalized_path_identity(Path::new(r"\\?\UNC\server\share\folder")),
            normalized_path_identity(Path::new(r"\\server\share\folder"))
        );
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
    fn parser_preserves_current_thread_metadata_and_dynamic_tools() {
        let path = temp_path("parse-current-metadata.jsonl");
        fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-07-10T08:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"child-thread\",\"cwd\":\"C:\\\\work\",\"model_provider\":\"openai\",\"cli_version\":\"0.144.1\",\"source\":{\"subagent\":{\"thread_spawn\":{\"parent_thread_id\":\"parent-thread\"}}},\"thread_source\":\"subagent\",\"parent_thread_id\":\"parent-thread\",\"forked_from_id\":\"parent-thread\",\"agent_nickname\":\"Curie\",\"agent_role\":\"explorer\",\"agent_path\":\"/root/audit\",\"history_mode\":\"legacy\",\"dynamic_tools\":[{\"type\":\"namespace\",\"name\":\"codex_app\",\"tools\":[{\"type\":\"function\",\"name\":\"read_thread\",\"description\":\"Read a thread\",\"inputSchema\":{\"type\":\"object\"},\"deferLoading\":true}]}]}}\n",
                "{\"timestamp\":\"2026-07-10T08:00:01Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"hello current schema\"}}\n"
            ),
        )
        .unwrap();

        let summary = parse_session_file(&path, true).unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(summary.thread_source.as_deref(), Some("subagent"));
        assert_eq!(summary.parent_thread_id.as_deref(), Some("parent-thread"));
        assert_eq!(summary.agent_nickname.as_deref(), Some("Curie"));
        assert_eq!(summary.agent_role.as_deref(), Some("explorer"));
        assert_eq!(summary.agent_path.as_deref(), Some("/root/audit"));
        assert_eq!(summary.history_mode.as_deref(), Some("legacy"));
        assert_eq!(summary.preview.as_deref(), Some("hello current schema"));
        assert_eq!(summary.dynamic_tools.len(), 1);
        assert_eq!(summary.dynamic_tools[0].name, "read_thread");
        assert_eq!(
            summary.dynamic_tools[0].namespace.as_deref(),
            Some("codex_app")
        );
        assert!(summary.dynamic_tools[0].defer_loading);
        assert!(summary
            .source
            .as_deref()
            .is_some_and(|source| source.contains("parent-thread")));
    }

    #[test]
    fn preview_reads_recent_messages_in_pages_without_duplicates() {
        let path = temp_path("preview-pages.jsonl");
        let content = (0..10)
            .map(|index| {
                json!({
                    "timestamp": format!("2026-07-10T08:00:{index:02}Z"),
                    "type": "event_msg",
                    "payload": {
                        "type": if index % 2 == 0 { "user_message" } else { "agent_message" },
                        "message": format!("message-{index}")
                    }
                })
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, format!("{content}\n")).unwrap();

        let latest = read_preview_message_page(&path, None, None, Some(3), None, None).unwrap();
        assert_eq!(
            latest
                .messages
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            vec!["message-7", "message-8", "message-9"]
        );
        assert!(latest.has_more);
        assert_eq!(latest.source, PreviewMessageSource::Event);

        let earlier = read_preview_message_page(
            &path,
            latest.next_before,
            Some(latest.file_size),
            Some(3),
            Some(latest.source.as_str()),
            None,
        )
        .unwrap();
        assert_eq!(
            earlier
                .messages
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            vec!["message-4", "message-5", "message-6"]
        );
        let latest_offsets = latest
            .messages
            .iter()
            .filter_map(|message| message.offset)
            .collect::<HashSet<_>>();
        assert!(earlier
            .messages
            .iter()
            .filter_map(|message| message.offset)
            .all(|offset| !latest_offsets.contains(&offset)));

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn preview_uses_response_items_when_event_messages_are_absent() {
        let path = temp_path("preview-response-fallback.jsonl");
        fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-07-10T08:00:00Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"text\":\"fallback-user\"}]}}\n",
                "{\"timestamp\":\"2026-07-10T08:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"text\":\"fallback-assistant\"}]}}\n"
            ),
        )
        .unwrap();

        let page = read_preview_message_page(&path, None, None, Some(10), None, None).unwrap();
        assert_eq!(page.source, PreviewMessageSource::Response);
        assert_eq!(page.messages.len(), 2);
        assert_eq!(page.messages[0].text, "fallback-user");
        assert_eq!(page.messages[1].text, "fallback-assistant");

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn preview_snapshot_allows_append_and_rejects_truncate() {
        let path = temp_path("preview-snapshot.jsonl");
        let initial = (0..5)
            .map(|index| {
                json!({
                    "timestamp": format!("2026-07-10T08:00:{index:02}Z"),
                    "type": "event_msg",
                    "payload": {"type": "user_message", "message": format!("m{index}")}
                })
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, format!("{initial}\n")).unwrap();
        let latest = read_preview_message_page(&path, None, None, Some(2), None, None).unwrap();

        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-07-10T08:01:00Z",
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "appended"}
            })
        )
        .unwrap();
        drop(file);
        let earlier = read_preview_message_page(
            &path,
            latest.next_before,
            Some(latest.file_size),
            Some(2),
            Some(latest.source.as_str()),
            None,
        )
        .unwrap();
        assert_eq!(earlier.file_size, latest.file_size);
        assert!(earlier
            .messages
            .iter()
            .all(|message| message.text != "appended"));

        fs::write(&path, "{}\n").unwrap();
        let stale = read_preview_message_page(
            &path,
            latest.next_before,
            Some(latest.file_size),
            Some(2),
            Some(latest.source.as_str()),
            None,
        )
        .unwrap_err();
        assert!(stale.contains("会话文件已变化"));

        fs::remove_file(path).unwrap();
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
    fn status_change_updates_current_state_without_rewriting_session_index() {
        let root = temp_path("status-current-state");
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
        let connection = create_current_state_db(&root);
        connection
            .execute(
                "INSERT INTO threads
                 (id, rollout_path, title, updated_at, updated_at_ms, preview, recency_at, recency_at_ms)
                 VALUES (?1, ?2, '新版标题', 1, 1000, 'preview', 1, 1000)",
                params![session_id, active_path.to_string_lossy()],
            )
            .unwrap();
        drop(connection);
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
        assert!(archived_path.exists());
        let archived_row: (i64, String) = Connection::open(root.join("state_5.sqlite"))
            .unwrap()
            .query_row(
                "SELECT archived, rollout_path FROM threads WHERE id = ?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(archived_row.0, 1);
        assert_eq!(
            conversation_path_key(Path::new(&archived_row.1)),
            conversation_path_key(&archived_path)
        );

        set_conversation_status_impl(
            root.to_string_lossy().to_string(),
            vec![path_to_slash(&archived_relative)],
            "active".to_string(),
            None,
        )
        .unwrap();
        let active_row: (i64, String) = Connection::open(root.join("state_5.sqlite"))
            .unwrap()
            .query_row(
                "SELECT archived, rollout_path FROM threads WHERE id = ?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let session_index = fs::read_to_string(root.join("session_index.jsonl")).unwrap();

        fs::remove_dir_all(&root).unwrap();

        assert_eq!(active_row.0, 0);
        assert_eq!(
            conversation_path_key(Path::new(&active_row.1)),
            conversation_path_key(&active_path)
        );
        assert!(session_index.contains(session_id));
        assert!(session_index.contains("测试归档索引"));
    }

    #[test]
    fn scan_catalog_uses_only_codex_desktop_thread_list() {
        let root = temp_path("scan-desktop-thread-list");
        let session_id = "019e20f9-34b7-7a82-a95b-fe461de8983a";
        let file_name = "rollout-2026-05-13T18-54-23-019e20f9-34b7-7a82-a95b-fe461de8983a.jsonl";
        let archived_path = root.join("archived_sessions").join(file_name);
        fs::create_dir_all(archived_path.parent().unwrap()).unwrap();
        fs::write(&archived_path, b"{}\n").unwrap();
        let unreturned_path = root.join("sessions").join("subagent.jsonl");
        fs::create_dir_all(unreturned_path.parent().unwrap()).unwrap();
        fs::write(&unreturned_path, b"{}\n").unwrap();

        let desktop_threads = vec![CodexDesktopThread {
            id: session_id.to_string(),
            name: Some("Codex Desktop 标题".to_string()),
            preview: "Desktop 预览".to_string(),
            cwd: root.join("workspace"),
            path: archived_path.clone(),
            updated_at: 10,
            recency_at: Some(20),
            archived: true,
        }];
        let (conversations, warnings, errors) =
            conversations_from_desktop_threads(&root, desktop_threads);

        fs::remove_dir_all(&root).unwrap();

        assert!(warnings.is_empty());
        assert!(errors.is_empty());
        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].id, session_id);
        assert_eq!(conversations[0].title, "Codex Desktop 标题");
        assert_eq!(conversations[0].preview.as_deref(), Some("Desktop 预览"));
        assert_eq!(conversations[0].status, "archived");
        assert_eq!(
            conversations[0].source_path,
            archived_path.to_string_lossy()
        );
    }

    #[test]
    fn session_index_uses_latest_title_across_local_id_variants() {
        let root = temp_path("session-index-title");
        let session_id = "019e20f9-34b7-7a82-a95b-fe461de8983a";
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("session_index.jsonl"),
            format!(
                "{}\n{}\n{}\n",
                json!({
                    "id": session_id,
                    "thread_name": "初始标题",
                    "updated_at": "2026-05-13T10:54:27.000Z"
                }),
                json!({
                    "id": format!("local:{session_id}"),
                    "thread_name": "Codex 最新标题",
                    "updated_at": "2026-05-13T10:55:27.000Z"
                }),
                json!({
                    "id": session_id,
                    "thread_name": "",
                    "updated_at": "2026-05-13T10:56:27.000Z"
                })
            ),
        )
        .unwrap();

        let mut warnings = Vec::new();
        let index = read_session_index(&root, &mut warnings);
        fs::remove_dir_all(&root).unwrap();

        assert!(warnings.is_empty());
        assert_eq!(
            session_index_title(&index, session_id).as_deref(),
            Some("Codex 最新标题")
        );
        assert_eq!(
            session_index_title(&index, &format!("local:{session_id}")).as_deref(),
            Some("Codex 最新标题")
        );
    }

    #[test]
    fn preview_uses_codex_session_index_title() {
        let root = temp_path("preview-session-index-title");
        let session_id = "019e20f9-34b7-7a82-a95b-fe461de8983a";
        let relative_path = PathBuf::from("sessions").join("rollout-preview-title.jsonl");
        let path = root.join(&relative_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                "{}\n{}\n",
                json!({
                    "timestamp": "2026-05-13T10:54:26.757Z",
                    "type": "session_meta",
                    "payload": {"id": session_id, "cwd": "C:\\work"}
                }),
                json!({
                    "timestamp": "2026-05-13T10:54:27.000Z",
                    "type": "event_msg",
                    "payload": {"type": "user_message", "message": "原始长消息"}
                })
            ),
        )
        .unwrap();
        let connection = create_current_state_db(&root);
        connection
            .execute(
                "INSERT INTO threads
                 (id, rollout_path, title, updated_at, updated_at_ms, preview, recency_at, recency_at_ms)
                 VALUES (?1, ?2, '数据库长标题', 1, 1000, '原始长消息', 1, 1000)",
                params![session_id, path.to_string_lossy()],
            )
            .unwrap();
        drop(connection);
        fs::write(
            root.join("session_index.jsonl"),
            format!(
                "{}\n",
                json!({
                    "id": session_id,
                    "thread_name": "Codex 原生标题",
                    "updated_at": "2026-05-13T10:54:27.000Z"
                })
            ),
        )
        .unwrap();

        let result = preview_conversation_impl(
            root.to_string_lossy().to_string(),
            path_to_slash(&relative_path),
            None,
            None,
            Some(10),
            None,
            None,
        )
        .unwrap();
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(
            result["conversation"]["title"],
            Value::String("Codex 原生标题".to_string())
        );
    }

    #[test]
    fn current_state_title_falls_back_when_session_index_has_no_match() {
        let root = temp_path("current-state-title-fallback");
        let session_id = "019e20f9-34b7-7a82-a95b-fe461de8983a";
        let path = root.join("sessions").join("rollout-title-fallback.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{}\n").unwrap();
        let connection = create_current_state_db(&root);
        connection
            .execute(
                "INSERT INTO threads
                 (id, rollout_path, title, updated_at, updated_at_ms, preview, recency_at, recency_at_ms)
                 VALUES (?1, ?2, '数据库标题', 1, 1000, '预览', 1, 1000)",
                params![session_id, path.to_string_lossy()],
            )
            .unwrap();
        drop(connection);

        let catalog = read_current_state_conversations(&root, &SessionIndex::new()).unwrap();
        fs::remove_dir_all(&root).unwrap();

        assert_eq!(catalog.conversations.len(), 1);
        assert_eq!(catalog.conversations[0].title, "数据库标题");
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
    fn upsert_state_threads_ignores_legacy_nested_state_db() {
        let root = temp_path("upsert-current-state-db");
        let current_db = root.join("state_5.sqlite");
        let legacy_db = root.join("sqlite").join("state_5.sqlite");
        for state_db in [&current_db, &legacy_db] {
            fs::create_dir_all(state_db.parent().unwrap()).unwrap();
            let connection = Connection::open(state_db).unwrap();
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
        }

        let item = sample_thread_metadata(root.join("sessions/rollout-thread-1.jsonl"));
        let updated = upsert_state_threads(&root, &[item]).unwrap();
        let current = Connection::open(&current_db).unwrap();
        let legacy = Connection::open(&legacy_db).unwrap();
        let current_count: i64 = current
            .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
            .unwrap();
        let legacy_count: i64 = legacy
            .query_row("SELECT COUNT(*) FROM threads", [], |row| row.get(0))
            .unwrap();

        assert_eq!(updated, 1);
        assert_eq!(current_count, 1);
        assert_eq!(legacy_count, 0);

        drop(current);
        drop(legacy);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn upsert_state_threads_writes_current_schema_relationships() {
        let root = temp_path("upsert-current-schema");
        fs::create_dir_all(&root).unwrap();
        let state_db = root.join("state_5.sqlite");
        let connection = Connection::open(&state_db).unwrap();
        connection
            .execute_batch(
                r#"
                PRAGMA foreign_keys = ON;
                CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT NOT NULL,
                    source TEXT NOT NULL,
                    updated_at INTEGER NOT NULL,
                    title TEXT NOT NULL,
                    cli_version TEXT NOT NULL DEFAULT '',
                    agent_nickname TEXT,
                    agent_role TEXT,
                    agent_path TEXT,
                    thread_source TEXT,
                    preview TEXT NOT NULL DEFAULT '',
                    recency_at INTEGER NOT NULL DEFAULT 0,
                    recency_at_ms INTEGER NOT NULL DEFAULT 0,
                    history_mode TEXT NOT NULL DEFAULT 'legacy'
                );
                CREATE TABLE thread_spawn_edges (
                    parent_thread_id TEXT NOT NULL,
                    child_thread_id TEXT NOT NULL PRIMARY KEY,
                    status TEXT NOT NULL
                );
                CREATE TABLE thread_dynamic_tools (
                    thread_id TEXT NOT NULL,
                    position INTEGER NOT NULL,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL,
                    input_schema TEXT NOT NULL,
                    defer_loading INTEGER NOT NULL DEFAULT 0,
                    namespace TEXT,
                    PRIMARY KEY(thread_id, position),
                    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
                );
                "#,
            )
            .unwrap();
        drop(connection);

        let mut item = sample_thread_metadata(root.join("sessions/rollout-thread-1.jsonl"));
        item.source =
            r#"{"subagent":{"thread_spawn":{"parent_thread_id":"parent-thread"}}}"#.to_string();
        item.agent_nickname = Some("Curie".to_string());
        item.agent_role = Some("explorer".to_string());
        item.agent_path = Some("/root/audit".to_string());
        item.thread_source = Some("subagent".to_string());
        item.preview = "hello current schema".to_string();
        item.parent_thread_id = Some("parent-thread".to_string());
        item.dynamic_tools = vec![ThreadDynamicToolMetadata {
            name: "read_thread".to_string(),
            description: "Read a thread".to_string(),
            input_schema: r#"{"type":"object"}"#.to_string(),
            defer_loading: true,
            namespace: Some("codex_app".to_string()),
        }];

        assert_eq!(upsert_state_threads(&root, &[item]).unwrap(), 1);
        let connection = Connection::open(&state_db).unwrap();
        let thread = connection
            .query_row(
                "SELECT source, cli_version, agent_nickname, agent_role, agent_path,
                        thread_source, preview, recency_at, recency_at_ms, history_mode
                 FROM threads WHERE id = 'thread-1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, String>(9)?,
                    ))
                },
            )
            .unwrap();
        let edge = connection
            .query_row(
                "SELECT parent_thread_id, status FROM thread_spawn_edges WHERE child_thread_id = 'thread-1'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        let tool = connection
            .query_row(
                "SELECT name, description, input_schema, defer_loading, namespace
                 FROM thread_dynamic_tools WHERE thread_id = 'thread-1' AND position = 0",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .unwrap();

        assert!(thread.0.contains("parent-thread"));
        assert_eq!(thread.1, "0.144.1");
        assert_eq!(thread.2.as_deref(), Some("Curie"));
        assert_eq!(thread.3.as_deref(), Some("explorer"));
        assert_eq!(thread.4.as_deref(), Some("/root/audit"));
        assert_eq!(thread.5.as_deref(), Some("subagent"));
        assert_eq!(thread.6, "hello current schema");
        assert_eq!(thread.7, 1_700_000_120);
        assert_eq!(thread.8, 1_700_000_120_000);
        assert_eq!(thread.9, "legacy");
        assert_eq!(edge, ("parent-thread".to_string(), "closed".to_string()));
        assert_eq!(tool.0, "read_thread");
        assert_eq!(tool.1, "Read a thread");
        assert_eq!(tool.2, r#"{"type":"object"}"#);
        assert_eq!(tool.3, 1);
        assert_eq!(tool.4.as_deref(), Some("codex_app"));

        drop(connection);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn insert_missing_state_threads_never_overwrites_current_rows_or_relationships() {
        let root = temp_path("insert-missing-current-wins");
        fs::create_dir_all(&root).unwrap();
        let state_db = root.join("state_5.sqlite");
        let connection = Connection::open(&state_db).unwrap();
        connection
            .execute_batch(
                r#"
                CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT NOT NULL,
                    title TEXT NOT NULL DEFAULT '',
                    updated_at INTEGER NOT NULL DEFAULT 0,
                    preview TEXT NOT NULL DEFAULT '',
                    recency_at INTEGER NOT NULL DEFAULT 0,
                    recency_at_ms INTEGER NOT NULL DEFAULT 0,
                    history_mode TEXT NOT NULL DEFAULT 'legacy'
                );
                CREATE TABLE thread_spawn_edges (
                    parent_thread_id TEXT NOT NULL,
                    child_thread_id TEXT NOT NULL PRIMARY KEY,
                    status TEXT NOT NULL
                );
                CREATE TABLE thread_dynamic_tools (
                    thread_id TEXT NOT NULL,
                    position INTEGER NOT NULL,
                    name TEXT NOT NULL,
                    description TEXT NOT NULL,
                    input_schema TEXT NOT NULL,
                    defer_loading INTEGER NOT NULL DEFAULT 0,
                    namespace TEXT,
                    PRIMARY KEY(thread_id, position)
                );
                INSERT INTO threads
                  (id, rollout_path, title, updated_at, preview, recency_at, recency_at_ms, history_mode)
                VALUES
                  ('thread-1', 'sessions/current.jsonl', 'Current title', 99, 'Current preview', 99, 99000, 'full');
                INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status)
                VALUES ('current-parent', 'thread-1', 'ready');
                INSERT INTO thread_dynamic_tools
                  (thread_id, position, name, description, input_schema, defer_loading, namespace)
                VALUES ('thread-1', 0, 'current-tool', 'current', '{}', 0, NULL);
                "#,
            )
            .unwrap();
        drop(connection);

        let mut item = sample_thread_metadata(root.join("sessions/replacement.jsonl"));
        item.title = "Replacement title".to_string();
        item.preview = "Replacement preview".to_string();
        item.parent_thread_id = Some("replacement-parent".to_string());
        item.dynamic_tools = vec![ThreadDynamicToolMetadata {
            name: "replacement-tool".to_string(),
            description: "replacement".to_string(),
            input_schema: "{}".to_string(),
            defer_loading: true,
            namespace: None,
        }];

        assert_eq!(insert_missing_state_threads(&root, &[item]).unwrap(), 0);
        let connection = Connection::open(&state_db).unwrap();
        let thread: (String, String, i64, String) = connection
            .query_row(
                "SELECT rollout_path, title, updated_at, preview FROM threads WHERE id = 'thread-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        let edge: String = connection
            .query_row(
                "SELECT parent_thread_id FROM thread_spawn_edges WHERE child_thread_id = 'thread-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let tool: String = connection
            .query_row(
                "SELECT name FROM thread_dynamic_tools WHERE thread_id = 'thread-1' AND position = 0",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(thread.0, "sessions/current.jsonl");
        assert_eq!(thread.1, "Current title");
        assert_eq!(thread.2, 99);
        assert_eq!(thread.3, "Current preview");
        assert_eq!(edge, "current-parent");
        assert_eq!(tool, "current-tool");

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

        delete_state_threads_for_sessions(&root, &["t1".to_string()], &[]).unwrap();
        let connection = Connection::open(&state_db).unwrap();

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

        delete_state_threads_for_sessions(&root, &["t1".to_string()], &[rollout_path]).unwrap();
        let connection = Connection::open(&state_db).unwrap();

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

    #[test]
    fn global_state_cleanup_removes_pinned_thread_ids() {
        let mut value = json!({
            "pinned-thread-ids": ["keep", "remove"],
            "nested": {
                "pinnedThreadIds": ["remove", "keep"],
                "remove": { "title": "old" },
                "keep": { "title": "current" }
            }
        });
        let ids = HashSet::from(["remove"]);

        let removed = remove_matching_object_keys(&mut value, &ids);

        assert_eq!(removed, 3);
        assert_eq!(value["pinned-thread-ids"], json!(["keep"]));
        assert_eq!(value["nested"]["pinnedThreadIds"], json!(["keep"]));
        assert!(value["nested"].get("remove").is_none());
        assert!(value["nested"].get("keep").is_some());
    }

    #[test]
    fn legacy_state_metadata_migration_preserves_current_rows_and_backfills_recency() {
        let root = temp_path("legacy-state-migration");
        fs::create_dir_all(root.join("sqlite")).unwrap();
        let current_path = root.join("state_5.sqlite");
        let legacy_path = root.join("sqlite").join("state_5.sqlite");
        let current_schema = r#"
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                preview TEXT NOT NULL DEFAULT '',
                recency_at INTEGER NOT NULL DEFAULT 0,
                recency_at_ms INTEGER NOT NULL DEFAULT 0,
                history_mode TEXT NOT NULL DEFAULT 'legacy'
            );
            CREATE TABLE thread_spawn_edges (
                parent_thread_id TEXT NOT NULL,
                child_thread_id TEXT PRIMARY KEY,
                status TEXT NOT NULL
            );
            CREATE TABLE thread_dynamic_tools (
                thread_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                name TEXT NOT NULL,
                PRIMARY KEY(thread_id, position)
            );
            CREATE TABLE agent_jobs (id TEXT PRIMARY KEY, name TEXT NOT NULL);
            CREATE TABLE agent_job_items (
                job_id TEXT NOT NULL,
                item_id TEXT NOT NULL,
                row_index INTEGER NOT NULL,
                PRIMARY KEY(job_id, item_id)
            );
        "#;
        let legacy_schema = r#"
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                preview TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE thread_spawn_edges (
                parent_thread_id TEXT NOT NULL,
                child_thread_id TEXT PRIMARY KEY,
                status TEXT NOT NULL
            );
            CREATE TABLE thread_dynamic_tools (
                thread_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                name TEXT NOT NULL,
                PRIMARY KEY(thread_id, position)
            );
            CREATE TABLE agent_jobs (id TEXT PRIMARY KEY, name TEXT NOT NULL);
            CREATE TABLE agent_job_items (
                job_id TEXT NOT NULL,
                item_id TEXT NOT NULL,
                row_index INTEGER NOT NULL,
                PRIMARY KEY(job_id, item_id)
            );
        "#;
        let current = Connection::open(&current_path).unwrap();
        current.execute_batch(current_schema).unwrap();
        current
            .execute(
                "INSERT INTO threads (id, rollout_path, created_at, updated_at, title, recency_at, recency_at_ms)
                 VALUES ('same', 'sessions/current.jsonl', 1, 2, 'Current', 2, 2000)",
                [],
            )
            .unwrap();
        current
            .execute_batch(
                "INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status)
                 VALUES ('current-parent', 'same', 'current');
                 INSERT INTO thread_dynamic_tools (thread_id, position, name)
                 VALUES ('same', 0, 'current-tool');",
            )
            .unwrap();
        drop(current);
        let legacy = Connection::open(&legacy_path).unwrap();
        legacy.execute_batch(legacy_schema).unwrap();
        legacy
            .execute_batch(
                "INSERT INTO threads (id, rollout_path, created_at, updated_at, title)
                 VALUES ('same', 'sessions/legacy-same.jsonl', 1, 3, 'Legacy');
                 INSERT INTO threads (id, rollout_path, created_at, updated_at, title)
                 VALUES ('old', 'sessions/old.jsonl', 10, 20, 'Old');
                 INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status)
                 VALUES ('same', 'old', 'ready');
                 INSERT INTO thread_dynamic_tools (thread_id, position, name)
                 VALUES ('old', 0, 'tool');
                 INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status)
                 VALUES ('legacy-parent', 'same', 'legacy');
                 INSERT INTO thread_dynamic_tools (thread_id, position, name)
                 VALUES ('same', 0, 'legacy-tool');
                 INSERT INTO agent_jobs (id, name) VALUES ('job-old', 'Old job');
                 INSERT INTO agent_job_items (job_id, item_id, row_index)
                 VALUES ('job-old', 'item-1', 0);",
            )
            .unwrap();
        drop(legacy);

        let mut current = Connection::open(&current_path).unwrap();
        let schema = state_threads_schema(&current).unwrap().unwrap();
        let inserted = merge_legacy_state_metadata(&mut current, &legacy_path, &schema).unwrap();
        let same_title: String = current
            .query_row("SELECT title FROM threads WHERE id = 'same'", [], |row| {
                row.get(0)
            })
            .unwrap();
        let old_row: (i64, i64, String) = current
            .query_row(
                "SELECT recency_at, recency_at_ms, history_mode FROM threads WHERE id = 'old'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        let same_edge: (String, String) = current
            .query_row(
                "SELECT parent_thread_id, status FROM thread_spawn_edges WHERE child_thread_id = 'same'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let same_tool: String = current
            .query_row(
                "SELECT name FROM thread_dynamic_tools WHERE thread_id = 'same' AND position = 0",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(inserted.get("threads"), Some(&1));
        assert_eq!(inserted.get("thread_spawn_edges"), Some(&1));
        assert_eq!(inserted.get("thread_dynamic_tools"), Some(&1));
        assert_eq!(inserted.get("agent_jobs"), Some(&1));
        assert_eq!(inserted.get("agent_job_items"), Some(&1));
        assert_eq!(same_title, "Current");
        assert_eq!(old_row, (20, 20_000, "legacy".to_string()));
        assert_eq!(
            same_edge,
            ("current-parent".to_string(), "current".to_string())
        );
        assert_eq!(same_tool, "current-tool");

        drop(current);
        fs::remove_dir_all(root).unwrap();
    }
}
