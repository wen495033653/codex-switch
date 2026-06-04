use super::{
    runtime::{emit_refresh_all_status, update_refresh_all_status_value, RefreshAllRuntime},
    targets::RefreshTarget,
};
use crate::{
    accounts::{read_store_value, read_store_with_active_sync},
    events::emit_store_updated,
    json_util::value_u64_field,
    quota::usage_store::{get_usage_with_auth_retry, update_account_usage_result},
    time_util::now_string,
};
use serde_json::{json, Value};
use std::{sync::Arc, thread};
use tauri::AppHandle;

pub(super) fn start_refresh_all_quotas_in_background(
    app: AppHandle,
    runtime: Arc<RefreshAllRuntime>,
    targets: Vec<RefreshTarget>,
) {
    thread::spawn(move || {
        if targets.is_empty() {
            let status = update_refresh_all_status_value(runtime.as_ref(), |mut current| {
                current["running"] = Value::Bool(false);
                current["finished_at"] = Value::String(now_string());
                current["message"] = Value::String("没有可刷新的账号".to_string());
                current
            });
            emit_refresh_all_status(&app, status);
            return;
        }

        for target in targets {
            let usage_result = get_usage_with_auth_retry(
                &app,
                &target.profile_id,
                &target.account_id,
                &target.access_token,
                30_000,
            );
            let usage_ok = usage_result.is_ok();
            let store_result = update_account_usage_result(&target.profile_id, usage_result);
            let store_update_ok = store_result.is_ok();
            if let Ok(store) = store_result {
                emit_store_updated(&app, store);
            }

            let status = update_refresh_all_status_value(runtime.as_ref(), |mut current| {
                let completed = value_u64_field(&current, "completed").unwrap_or(0) + 1;
                let success = usage_ok && store_update_ok;
                let updated =
                    value_u64_field(&current, "updated").unwrap_or(0) + if success { 1 } else { 0 };
                let failed =
                    value_u64_field(&current, "failed").unwrap_or(0) + if success { 0 } else { 1 };
                let total = value_u64_field(&current, "total").unwrap_or(0);
                current["completed"] = json!(completed);
                current["updated"] = json!(updated);
                current["failed"] = json!(failed);
                current["message"] = Value::String(format!("后台刷新中（{completed}/{total}）"));
                current
            });
            emit_refresh_all_status(&app, status);
        }

        let _ = read_store_with_active_sync();
        if let Ok(store) = read_store_value() {
            emit_store_updated(&app, store);
        }
        let status = update_refresh_all_status_value(runtime.as_ref(), |mut current| {
            let updated = value_u64_field(&current, "updated").unwrap_or(0);
            let failed = value_u64_field(&current, "failed").unwrap_or(0);
            current["running"] = Value::Bool(false);
            current["finished_at"] = Value::String(now_string());
            current["message"] = Value::String(if failed > 0 {
                format!("已刷新 {updated} 个账号，{failed} 个失败")
            } else {
                format!("已刷新 {updated} 个账号")
            });
            current
        });
        emit_refresh_all_status(&app, status);
    });
}
