use crate::quota::{begin_refresh_all_quotas, get_refresh_all_status_value, RefreshAllRuntime};
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

#[tauri::command]
pub(crate) fn refresh_all_quotas(
    app: AppHandle,
    runtime: State<'_, Arc<RefreshAllRuntime>>,
) -> Result<Value, String> {
    begin_refresh_all_quotas(app, Arc::clone(runtime.inner()), "manual")
}
