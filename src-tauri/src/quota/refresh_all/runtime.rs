use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

pub(crate) struct RefreshAllRuntime {
    status: Mutex<Value>,
}

fn default_refresh_all_status() -> Value {
    json!({
        "running": false,
        "total": 0,
        "completed": 0,
        "updated": 0,
        "failed": 0,
        "started_at": "",
        "finished_at": "",
        "message": "",
        "source": ""
    })
}

impl Default for RefreshAllRuntime {
    fn default() -> Self {
        Self {
            status: Mutex::new(default_refresh_all_status()),
        }
    }
}

pub(crate) fn get_refresh_all_status_value(runtime: &RefreshAllRuntime) -> Value {
    runtime
        .status
        .lock()
        .map(|status| status.clone())
        .unwrap_or_else(|_| default_refresh_all_status())
}

pub(super) fn set_refresh_all_status_value(runtime: &RefreshAllRuntime, status: Value) -> Value {
    if let Ok(mut current) = runtime.status.lock() {
        *current = status.clone();
    }
    status
}

pub(super) fn update_refresh_all_status_value<F>(runtime: &RefreshAllRuntime, update: F) -> Value
where
    F: FnOnce(Value) -> Value,
{
    if let Ok(mut current) = runtime.status.lock() {
        let next = update(current.clone());
        *current = next.clone();
        return next;
    }
    default_refresh_all_status()
}

pub(super) fn emit_refresh_all_status(app: &AppHandle, status: Value) {
    let _ = app.emit("refresh-all-status", json!({ "status": status }));
}
