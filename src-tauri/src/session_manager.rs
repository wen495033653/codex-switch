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

mod backup;
mod delete;
mod deleted_records;
mod legacy_migration;
mod scan;
mod session_file;
mod state_db;
mod status;
#[cfg(test)]
mod tests;
mod transfer;
mod util;
mod zip;

use backup::*;
use delete::*;
use deleted_records::*;
pub(crate) use legacy_migration::*;
use scan::*;
use session_file::*;
use state_db::*;
use status::*;
use transfer::*;
use util::*;
use zip::*;

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
