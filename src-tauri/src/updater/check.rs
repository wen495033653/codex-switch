use super::{
    service::{dev_mode_update_result, emit_update_status, find_update, update_info},
    state::{clear_pending_update, store_pending_update, UpdateRuntime},
};
use crate::{
    json_util::{bool_field, raw_string_field, string_field},
    settings::{read_settings_value, update_settings_value},
};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, State};

pub(super) async fn check_update_impl(
    app: AppHandle,
    runtime: State<'_, Arc<UpdateRuntime>>,
    options: Option<Value>,
) -> Result<Value, String> {
    if cfg!(debug_assertions) {
        return Ok(dev_mode_update_result());
    }

    let opts = options.unwrap_or_else(|| json!({}));
    let manual = bool_field(&opts, "manual");
    emit_update_status(&app, json!({ "status": "checking" }));

    match find_update(&app).await {
        Ok(Some(update)) => {
            let info = update_info(&update);
            store_pending_update(runtime.inner().as_ref(), update, None);
            emit_update_status(
                &app,
                json!({
                    "status": "available",
                    "update": info
                }),
            );

            let settings = read_settings_value()?;
            let remote_version = string_field(&info, "version");
            let suppressed = !manual
                && !remote_version.is_empty()
                && string_field(&settings, "dismissed_update_version") == remote_version;

            Ok(json!({
                "ok": true,
                "message": format!("发现新版本 {remote_version}"),
                "has_update": true,
                "suppressed": suppressed,
                "current_version": env!("CARGO_PKG_VERSION"),
                "remote_version": remote_version,
                "release_name": string_field(&info, "release_name"),
                "release_notes": raw_string_field(&info, "release_notes"),
                "release_date": string_field(&info, "release_date")
            }))
        }
        Ok(None) => {
            clear_pending_update(runtime.inner().as_ref());
            emit_update_status(
                &app,
                json!({
                    "status": "not-available",
                    "update": {
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            );
            Ok(json!({
                "ok": true,
                "has_update": false,
                "current_version": env!("CARGO_PKG_VERSION"),
                "remote_version": env!("CARGO_PKG_VERSION"),
                "message": "当前已是最新版本"
            }))
        }
        Err(err) => {
            emit_update_status(
                &app,
                json!({
                    "status": "error",
                    "error": err
                }),
            );
            Err(err)
        }
    }
}

pub(super) fn dismiss_update_version_impl(version: String) -> Result<Value, String> {
    let value = version.trim();
    if value.is_empty() {
        return Err("版本号不能为空".to_string());
    }
    update_settings_value(&json!({ "dismissed_update_version": value }))?;
    Ok(json!({ "ok": true }))
}
