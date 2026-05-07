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
pub(crate) fn switch_api_mode(runtime: State<'_, Arc<IdeRuntime>>) -> Result<Value, String> {
    switch_api_mode_impl(runtime)
}

#[tauri::command]
pub(crate) async fn import_accounts(app: AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || import_accounts_impl(app))
        .await
        .map_err(|err| format!("导入任务异常: {err}"))?
}

#[tauri::command]
pub(crate) async fn export_accounts(app: AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || export_accounts_impl(app))
        .await
        .map_err(|err| format!("导出任务异常: {err}"))?
}

#[tauri::command]
pub(crate) fn refresh_account(id: String) -> Result<Value, String> {
    refresh_account_impl(id)
}

#[tauri::command]
pub(crate) fn refresh_account_token(id: String) -> Result<Value, String> {
    refresh_account_token_impl(id)
}
