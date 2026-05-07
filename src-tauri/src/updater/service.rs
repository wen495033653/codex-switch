use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

fn format_update_date(date: Option<OffsetDateTime>) -> String {
    date.and_then(|value| value.format(&Rfc3339).ok())
        .unwrap_or_default()
}

pub(super) fn update_info(update: &Update) -> Value {
    json!({
        "version": update.version,
        "release_name": "",
        "release_notes": update.body.clone().unwrap_or_default(),
        "release_date": format_update_date(update.date),
        "download_url": update.download_url.to_string()
    })
}

pub(super) fn emit_update_status(app: &AppHandle, payload: Value) {
    let _ = app.emit("update-status", payload);
}

pub(super) async fn find_update(app: &AppHandle) -> Result<Option<Update>, String> {
    let updater = app
        .updater()
        .map_err(|err| format!("初始化更新器失败: {err}"))?;
    updater
        .check()
        .await
        .map_err(|err| format!("检查更新失败: {err}"))
}

pub(super) fn dev_mode_update_result() -> Value {
    json!({
        "ok": true,
        "has_update": false,
        "current_version": env!("CARGO_PKG_VERSION"),
        "remote_version": env!("CARGO_PKG_VERSION"),
        "dev_mode": true,
        "message": "开发模式不支持在线更新，请使用安装版测试"
    })
}
