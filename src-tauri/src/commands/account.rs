use super::*;

mod import_export;
mod mode;
mod refresh;

use import_export::{export_accounts_impl, import_accounts_impl};
use mode::{
    capture_current_impl, delete_account_impl, import_refresh_token_impl, switch_account_impl,
    switch_api_mode_impl,
};
use refresh::{refresh_account_impl, refresh_account_token_impl};

fn blocking_task_error(action: &str, err: impl std::fmt::Display) -> String {
    let message = err.to_string();
    if message.contains("panicked") {
        format!("{action}任务异常，请重试")
    } else {
        format!("{action}任务异常: {message}")
    }
}

#[tauri::command]
pub(crate) fn capture_current() -> Result<Value, String> {
    capture_current_impl()
}

#[tauri::command]
pub(crate) fn import_refresh_token(app: AppHandle, token: String) -> Result<Value, String> {
    import_refresh_token_impl(app, token)
}

#[tauri::command]
pub(crate) fn delete_account(id: String) -> Result<Value, String> {
    delete_account_impl(id)
}

#[tauri::command]
pub(crate) fn switch_account(
    app: AppHandle,
    id: String,
    runtime: State<'_, Arc<IdeRuntime>>,
) -> Result<Value, String> {
    switch_account_impl(app, id, runtime)
}

#[tauri::command]
pub(crate) fn switch_api_mode(
    profile_id: Option<String>,
    runtime: State<'_, Arc<IdeRuntime>>,
) -> Result<Value, String> {
    switch_api_mode_impl(runtime, profile_id)
}

#[tauri::command]
pub(crate) async fn import_accounts(app: AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || import_accounts_impl(app))
        .await
        .map_err(|err| blocking_task_error("导入", err))?
}

#[tauri::command]
pub(crate) async fn export_accounts(app: AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || export_accounts_impl(app))
        .await
        .map_err(|err| blocking_task_error("导出", err))?
}

#[tauri::command]
pub(crate) async fn refresh_account(id: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || refresh_account_impl(id))
        .await
        .map_err(|err| blocking_task_error("刷新账号", err))?
}

#[tauri::command]
pub(crate) async fn refresh_account_token(id: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || refresh_account_token_impl(id))
        .await
        .map_err(|err| blocking_task_error("刷新 Refresh Token", err))?
}
