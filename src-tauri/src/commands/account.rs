use crate::{blocking_task::run_blocking, codex_launcher::IdeRuntime};
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, State};

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
pub(crate) async fn capture_current() -> Result<Value, String> {
    run_blocking("保存当前账号", capture_current_impl).await
}

#[tauri::command]
pub(crate) async fn import_refresh_token(app: AppHandle, token: String) -> Result<Value, String> {
    run_blocking("导入 refresh_token", move || {
        import_refresh_token_impl(app, token)
    })
    .await
}

#[tauri::command]
pub(crate) async fn delete_account(id: String) -> Result<Value, String> {
    run_blocking("删除账号", move || delete_account_impl(id)).await
}

#[tauri::command]
pub(crate) async fn switch_account(
    app: AppHandle,
    id: String,
    runtime: State<'_, Arc<IdeRuntime>>,
) -> Result<Value, String> {
    let runtime = Arc::clone(runtime.inner());
    run_blocking("切换账号", move || {
        switch_account_impl(app, id, &runtime)
    })
    .await
}

#[tauri::command]
pub(crate) async fn switch_api_mode(
    profile_id: Option<String>,
    runtime: State<'_, Arc<IdeRuntime>>,
) -> Result<Value, String> {
    let runtime = Arc::clone(runtime.inner());
    run_blocking("切换 API 模式", move || {
        switch_api_mode_impl(&runtime, profile_id)
    })
    .await
}

#[tauri::command]
pub(crate) async fn import_accounts(app: AppHandle) -> Result<Value, String> {
    run_blocking("导入", move || import_accounts_impl(app)).await
}

#[tauri::command]
pub(crate) async fn export_accounts(app: AppHandle) -> Result<Value, String> {
    run_blocking("导出", move || export_accounts_impl(app)).await
}

#[tauri::command]
pub(crate) async fn refresh_account(id: String) -> Result<Value, String> {
    run_blocking("刷新账号", move || refresh_account_impl(id)).await
}

#[tauri::command]
pub(crate) async fn refresh_account_token(id: String) -> Result<Value, String> {
    run_blocking("刷新 Refresh Token", move || {
        refresh_account_token_impl(id)
    })
    .await
}
