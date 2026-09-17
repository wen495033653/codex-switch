mod backup;
mod catalog;
mod codex_home;
mod legacy_migration;
mod model;
mod preview;
mod rollout;
mod state_db;
mod status;
#[cfg(test)]
mod tests;
mod transfer;
mod trash;
mod trash_store;
mod util;
mod zip;

pub(crate) use legacy_migration::{
    migrate_legacy_codex_data_for_current_home, migrate_legacy_codex_data_for_root,
};

use serde_json::Value;
use tauri::AppHandle;
use {
    catalog::scan_conversations_impl,
    preview::{begin_preview_request, preview_conversation_impl},
    status::set_conversation_status_impl,
    transfer::{export_conversations_impl, import_conversations_impl},
    trash::{
        delete_conversations_impl, list_deleted_sessions_impl, preview_deleted_conversation_impl,
        purge_deleted_sessions_impl, restore_deleted_sessions_impl,
    },
};

fn blocking_task_error(action: &str, err: impl std::fmt::Display) -> String {
    let message = err.to_string();
    if message.contains("panicked") {
        format!("{action}任务异常，请重试")
    } else {
        format!("{action}任务异常: {message}")
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
