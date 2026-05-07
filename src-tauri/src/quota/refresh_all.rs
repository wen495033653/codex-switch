mod runtime;
mod targets;
mod worker;

pub(crate) use runtime::{get_refresh_all_status_value, RefreshAllRuntime};

use self::{
    runtime::{emit_refresh_all_status, set_refresh_all_status_value},
    targets::{has_due_background_quota_refresh, refresh_targets_from_store},
    worker::start_refresh_all_quotas_in_background,
};
use crate::{
    accounts::read_store_with_active_sync,
    json_util::{bool_field, value_u64_field},
    settings::{
        normalize_background_refresh_interval_minutes, read_settings_value,
        BACKGROUND_REFRESH_DEFAULT_INTERVAL_MINUTES,
    },
    time_util::now_string,
};
use serde_json::{json, Value};
use std::{sync::Arc, thread, time::Duration as StdDuration};
use tauri::AppHandle;

pub(crate) fn begin_refresh_all_quotas(
    app: AppHandle,
    runtime: Arc<RefreshAllRuntime>,
    source: &'static str,
) -> Result<Value, String> {
    let status = get_refresh_all_status_value(runtime.as_ref());
    let store = read_store_with_active_sync()?;
    if bool_field(&status, "running") {
        return Ok(json!({
            "ok": true,
            "message": "后台刷新仍在进行中",
            "started": false,
            "status": status,
            "store": store
        }));
    }

    let targets = refresh_targets_from_store(&store);
    let status = set_refresh_all_status_value(
        runtime.as_ref(),
        json!({
            "running": true,
            "total": targets.len(),
            "completed": 0,
            "updated": 0,
            "failed": 0,
            "started_at": now_string(),
            "finished_at": "",
            "message": if targets.is_empty() { "没有可刷新的账号" } else { "后台刷新中" },
            "source": source
        }),
    );
    emit_refresh_all_status(&app, status.clone());
    start_refresh_all_quotas_in_background(app, runtime, targets);

    Ok(json!({
        "ok": true,
        "message": "已开始后台刷新配额",
        "started": true,
        "status": status,
        "store": store
    }))
}

fn background_refresh_settings() -> Result<(bool, u64), String> {
    let settings = read_settings_value()?;
    let enabled = settings
        .get("background_refresh_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let interval_minutes = normalize_background_refresh_interval_minutes(value_u64_field(
        &settings,
        "background_refresh_interval_minutes",
    ));
    Ok((enabled, interval_minutes))
}

pub(crate) fn start_background_quota_auto_refresher(
    app: AppHandle,
    runtime: Arc<RefreshAllRuntime>,
) {
    thread::spawn(move || loop {
        let (enabled, interval_minutes) = match background_refresh_settings() {
            Ok(value) => value,
            Err(err) => {
                eprintln!("读取定时刷新全部账号设置失败: {err}");
                (true, BACKGROUND_REFRESH_DEFAULT_INTERVAL_MINUTES)
            }
        };
        if enabled {
            match read_store_with_active_sync() {
                Ok(store) => {
                    if has_due_background_quota_refresh(&store, interval_minutes) {
                        if let Err(err) =
                            begin_refresh_all_quotas(app.clone(), Arc::clone(&runtime), "auto")
                        {
                            eprintln!("定时刷新全部账号失败: {err}");
                        }
                    }
                }
                Err(err) => eprintln!("读取账号数据失败，已跳过定时刷新全部账号: {err}"),
            }
        }
        thread::sleep(StdDuration::from_secs(interval_minutes * 60));
    });
}
