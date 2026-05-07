use crate::accounts::get_codex_state_value;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

pub(crate) fn emit_store_updated(app: &AppHandle, store: Value) {
    let _ = app.emit(
        "store-updated",
        json!({
            "store": store,
            "codex_state": get_codex_state_value()
        }),
    );
}
