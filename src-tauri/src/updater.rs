mod check;
mod download;
mod service;
mod state;

pub(crate) use state::UpdateRuntime;

use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, State};

#[tauri::command]
pub(crate) async fn check_update(
    app: AppHandle,
    runtime: State<'_, Arc<UpdateRuntime>>,
    options: Option<Value>,
) -> Result<Value, String> {
    check::check_update_impl(app, runtime, options).await
}

#[tauri::command]
pub(crate) async fn download_update(
    app: AppHandle,
    runtime: State<'_, Arc<UpdateRuntime>>,
) -> Result<Value, String> {
    download::download_update_impl(app, runtime).await
}

#[tauri::command]
pub(crate) fn install_update(runtime: State<'_, Arc<UpdateRuntime>>) -> Result<Value, String> {
    download::install_update_impl(runtime)
}

#[tauri::command]
pub(crate) fn dismiss_update_version(version: String) -> Result<Value, String> {
    check::dismiss_update_version_impl(version)
}
