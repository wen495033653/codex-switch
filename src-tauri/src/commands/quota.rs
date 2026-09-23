use crate::{
    blocking_task::run_blocking,
    quota::{begin_refresh_all_quotas, get_refresh_all_status_value, RefreshAllRuntime},
};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, State};

#[tauri::command]
pub(crate) fn get_refresh_all_status(runtime: State<'_, Arc<RefreshAllRuntime>>) -> Value {
    json!({
        "ok": true,
        "status": get_refresh_all_status_value(runtime.inner().as_ref())
    })
}

/// Reads accounts.json, auth.json and config.toml before the background pass starts, so it
/// runs on the blocking pool rather than the main thread.
#[tauri::command]
pub(crate) async fn refresh_all_quotas(
    app: AppHandle,
    runtime: State<'_, Arc<RefreshAllRuntime>>,
) -> Result<Value, String> {
    let runtime = Arc::clone(runtime.inner());
    run_blocking("刷新全部配额", move || {
        begin_refresh_all_quotas(app, runtime, "manual")
    })
    .await
}
