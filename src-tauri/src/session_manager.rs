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

use crate::{app_log::log_event, blocking_task::run_blocking};
use serde_json::{json, Value};
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

/// The failures of single sessions come back in the report and are shown once; the log keeps
/// them (import records its own `session_manager_import_apply_error`).
fn log_item_errors(event: &str, root: Option<&str>, response: &Value) {
    let errors = response
        .pointer("/report/errors")
        .or_else(|| response.get("errors"))
        .and_then(Value::as_array)
        .filter(|errors| !errors.is_empty());
    if let Some(errors) = errors {
        log_event(event, json!({ "root": root, "errors": errors }));
    }
}

#[tauri::command]
pub(crate) async fn session_manager_scan(root: Option<String>) -> Result<Value, String> {
    run_blocking("扫描会话", move || scan_conversations_impl(root)).await
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
    run_blocking("读取预览", move || {
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
    run_blocking("读取已删除预览", move || {
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
}

#[tauri::command]
pub(crate) async fn session_manager_export(
    app: AppHandle,
    root: String,
    relative_paths: Vec<String>,
) -> Result<Value, String> {
    let log_root = root.clone();
    run_blocking("导出会话", move || {
        export_conversations_impl(app, root, relative_paths)
    })
    .await
    .inspect(|response| log_item_errors("session_manager_export_error", Some(&log_root), response))
}

#[tauri::command]
pub(crate) async fn session_manager_import(app: AppHandle, root: String) -> Result<Value, String> {
    run_blocking("导入会话", move || import_conversations_impl(app, root)).await
}

#[tauri::command]
pub(crate) async fn session_manager_delete(
    root: String,
    relative_paths: Vec<String>,
) -> Result<Value, String> {
    let log_root = root.clone();
    run_blocking("删除会话", move || {
        delete_conversations_impl(root, relative_paths)
    })
    .await
    .inspect(|response| log_item_errors("session_manager_delete_error", Some(&log_root), response))
}

#[tauri::command]
pub(crate) async fn session_manager_list_deleted() -> Result<Value, String> {
    run_blocking("读取已删除会话", list_deleted_sessions_impl).await
}

#[tauri::command]
pub(crate) async fn session_manager_restore_deleted(
    root: String,
    delete_ids: Vec<String>,
    conflict_strategy: Option<String>,
) -> Result<Value, String> {
    let log_root = root.clone();
    run_blocking("恢复会话", move || {
        restore_deleted_sessions_impl(root, delete_ids, conflict_strategy)
    })
    .await
    .inspect(|response| log_item_errors("session_manager_restore_error", Some(&log_root), response))
}

#[tauri::command]
pub(crate) async fn session_manager_purge_deleted(
    delete_ids: Vec<String>,
) -> Result<Value, String> {
    run_blocking("彻底删除会话", move || {
        purge_deleted_sessions_impl(delete_ids)
    })
    .await
    .inspect(|response| log_item_errors("session_manager_purge_error", None, response))
}

#[tauri::command]
pub(crate) async fn session_manager_set_status(
    root: String,
    relative_paths: Vec<String>,
    status: String,
    conflict_strategy: Option<String>,
) -> Result<Value, String> {
    let log_root = root.clone();
    run_blocking("切换会话状态", move || {
        set_conversation_status_impl(root, relative_paths, status, conflict_strategy)
    })
    .await
    .inspect(|response| log_item_errors("session_manager_status_error", Some(&log_root), response))
}
