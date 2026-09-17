use crate::{
    json_util::{bool_field, raw_string_field, string_field},
    settings::{read_settings_value, update_settings_value},
};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[tauri::command]
pub(crate) async fn check_update(
    app: AppHandle,
    runtime: State<'_, Arc<UpdateRuntime>>,
    options: Option<Value>,
) -> Result<Value, String> {
    check_update_impl(app, runtime, options).await
}

#[tauri::command]
pub(crate) async fn download_update(
    app: AppHandle,
    runtime: State<'_, Arc<UpdateRuntime>>,
) -> Result<Value, String> {
    download_update_impl(app, runtime).await
}

#[tauri::command]
pub(crate) fn install_update(runtime: State<'_, Arc<UpdateRuntime>>) -> Result<Value, String> {
    install_update_impl(runtime)
}

#[tauri::command]
pub(crate) fn dismiss_update_version(version: String) -> Result<Value, String> {
    dismiss_update_version_impl(version)
}

async fn check_update_impl(
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

fn dismiss_update_version_impl(version: String) -> Result<Value, String> {
    let value = version.trim();
    if value.is_empty() {
        return Err("版本号不能为空".to_string());
    }
    update_settings_value(&json!({ "dismissed_update_version": value }))?;
    Ok(json!({ "ok": true }))
}

async fn download_update_impl(
    app: AppHandle,
    runtime: State<'_, Arc<UpdateRuntime>>,
) -> Result<Value, String> {
    if cfg!(debug_assertions) {
        return Err(dev_mode_update_result()
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("开发模式不支持在线更新，请使用安装版测试")
            .to_string());
    }

    let pending = read_pending_update(runtime.inner().as_ref())?;
    let (update, existing_bytes) = match pending {
        Some(value) => value,
        None => {
            let update = find_update(&app)
                .await?
                .ok_or_else(|| "当前没有可下载的更新".to_string())?;
            (update, None)
        }
    };
    let info = update_info(&update);

    if existing_bytes.is_some() {
        emit_update_status(
            &app,
            json!({
                "status": "downloaded",
                "update": info
            }),
        );
        return Ok(json!({
            "ok": true,
            "message": "更新已下载完成",
            "downloaded": true,
            "update": info
        }));
    }

    emit_download_started(&app, &info);

    let update_for_download = update.clone();
    let mut progress = DownloadProgress::new(app.clone(), info.clone());
    let bytes = update_for_download
        .download(
            |chunk_length, content_length| {
                progress.emit_chunk(chunk_length, content_length);
            },
            || {},
        )
        .await
        .map_err(|err| {
            let message = format!("下载更新失败: {err}");
            emit_update_status(
                &app,
                json!({
                    "status": "error",
                    "error": message,
                    "update": info
                }),
            );
            message
        })?;

    store_pending_update(runtime.inner().as_ref(), update, Some(bytes));
    emit_update_status(
        &app,
        json!({
            "status": "downloaded",
            "update": info
        }),
    );

    Ok(json!({
        "ok": true,
        "message": "更新已下载完成",
        "downloaded": true,
        "update": info
    }))
}

fn install_update_impl(runtime: State<'_, Arc<UpdateRuntime>>) -> Result<Value, String> {
    let (update, bytes) = read_pending_update(runtime.inner().as_ref())?
        .and_then(|(update, bytes)| bytes.map(|value| (update, value)))
        .ok_or_else(|| "更新尚未下载完成".to_string())?;
    update
        .install(&bytes)
        .map_err(|err| format!("安装更新失败: {err}"))?;
    Ok(json!({
        "ok": true,
        "message": "正在重启安装"
    }))
}

fn emit_download_started(app: &AppHandle, info: &Value) {
    emit_update_status(
        app,
        json!({
            "status": "downloading",
            "progress": {
                "percent": 0,
                "transferred": 0,
                "total": 0,
                "bytes_per_second": 0
            },
            "update": info
        }),
    );
}

struct DownloadProgress {
    app: AppHandle,
    info: Value,
    start: Instant,
    transferred: u64,
}

impl DownloadProgress {
    pub(super) fn new(app: AppHandle, info: Value) -> Self {
        Self {
            app,
            info,
            start: Instant::now(),
            transferred: 0,
        }
    }

    pub(super) fn emit_chunk(&mut self, chunk_length: usize, content_length: Option<u64>) {
        self.transferred = self.transferred.saturating_add(chunk_length as u64);
        let percent = content_length
            .filter(|total| *total > 0)
            .map(|total| (self.transferred as f64 / total as f64) * 100.0)
            .unwrap_or(0.0);
        let elapsed = self.start.elapsed().as_secs_f64();
        let bytes_per_second = if elapsed > 0.0 {
            (self.transferred as f64 / elapsed).round() as u64
        } else {
            0
        };

        emit_update_status(
            &self.app,
            json!({
                "status": "downloading",
                "progress": {
                    "percent": percent,
                    "transferred": self.transferred,
                    "total": content_length.unwrap_or(0),
                    "bytes_per_second": bytes_per_second
                },
                "update": self.info
            }),
        );
    }
}

fn format_update_date(date: Option<OffsetDateTime>) -> String {
    date.and_then(|value| value.format(&Rfc3339).ok())
        .unwrap_or_default()
}

fn update_info(update: &Update) -> Value {
    json!({
        "version": update.version,
        "release_name": "",
        "release_notes": update.body.clone().unwrap_or_default(),
        "release_date": format_update_date(update.date),
        "download_url": update.download_url.to_string()
    })
}

fn emit_update_status(app: &AppHandle, payload: Value) {
    let _ = app.emit("update-status", payload);
}

async fn find_update(app: &AppHandle) -> Result<Option<Update>, String> {
    let updater = app
        .updater()
        .map_err(|err| format!("初始化更新器失败: {err}"))?;
    updater
        .check()
        .await
        .map_err(|err| format!("检查更新失败: {err}"))
}

fn dev_mode_update_result() -> Value {
    json!({
        "ok": true,
        "has_update": false,
        "current_version": env!("CARGO_PKG_VERSION"),
        "remote_version": env!("CARGO_PKG_VERSION"),
        "dev_mode": true,
        "message": "开发模式不支持在线更新，请使用安装版测试"
    })
}

struct PendingUpdate {
    update: Update,
    bytes: Option<Vec<u8>>,
}

type PendingUpdateData = Option<(Update, Option<Vec<u8>>)>;

#[derive(Default)]
pub(crate) struct UpdateRuntime {
    pending: Mutex<Option<PendingUpdate>>,
}

fn clear_pending_update(runtime: &UpdateRuntime) {
    if let Ok(mut pending) = runtime.pending.lock() {
        *pending = None;
    }
}

fn store_pending_update(runtime: &UpdateRuntime, update: Update, bytes: Option<Vec<u8>>) {
    if let Ok(mut pending) = runtime.pending.lock() {
        *pending = Some(PendingUpdate { update, bytes });
    }
}

fn read_pending_update(runtime: &UpdateRuntime) -> Result<PendingUpdateData, String> {
    runtime
        .pending
        .lock()
        .map_err(|_| "更新状态锁异常".to_string())
        .map(|pending| {
            pending
                .as_ref()
                .map(|item| (item.update.clone(), item.bytes.clone()))
        })
}
