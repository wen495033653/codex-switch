use super::super::service::emit_update_status;
use serde_json::{json, Value};
use std::time::Instant;
use tauri::AppHandle;

pub(super) fn emit_download_started(app: &AppHandle, info: &Value) {
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

pub(super) struct DownloadProgress {
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
