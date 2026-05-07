mod progress;

use super::{
    service::{dev_mode_update_result, emit_update_status, find_update, update_info},
    state::{read_pending_update, store_pending_update, UpdateRuntime},
};
use progress::{emit_download_started, DownloadProgress};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, State};

pub(super) async fn download_update_impl(
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

pub(super) fn install_update_impl(runtime: State<'_, Arc<UpdateRuntime>>) -> Result<Value, String> {
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
