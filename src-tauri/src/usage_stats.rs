mod aggregate;
mod db;
mod model;
mod pricing;
mod records;
mod scan;
mod sources;
#[cfg(test)]
mod tests;

use crate::{
    accounts::get_codex_state_value,
    app_log::log_event_once,
    blocking_task::run_blocking,
    json_util::{raw_string_field, string_field},
    paths::codex_dir,
    settings::read_settings_value,
    time_util::{now_string, parse_rfc3339_seconds},
};
use serde_json::{json, Value};
use std::{path::Path, sync::Mutex};
use {
    aggregate::aggregate_usage,
    db::{
        db_error, open_usage_connection, record_attribution_at, records_counted_after_seconds,
        usage_db_path,
    },
    model::{
        UsageScanSource, OWNER_TYPE_API_PROFILE, OWNER_TYPE_SUBSCRIPTION, PROVIDER_API,
        PROVIDER_SUBSCRIPTION,
    },
    scan::{scan_sources, write_scan},
    sources::default_usage_scan_sources,
};

// One refresh at a time: two concurrent ones would read the same appended records.
static USAGE_STATS_SCAN_LOCK: Mutex<()> = Mutex::new(());

#[tauri::command]
pub(crate) async fn usage_stats_get() -> Result<Value, String> {
    // The page polls silently, and a failed refresh commits nothing, so the reason is logged here
    // or a refresh that keeps failing would go unnoticed. It polls every 30 s, so each distinct
    // failure is recorded once per run.
    run_blocking("读取 token 统计", || {
        usage_stats_get_impl()
            .inspect_err(|err| log_event_once("usage_stats_refresh_error", json!({ "error": err })))
    })
    .await
}

pub(crate) fn record_attribution(
    owner_type: &str,
    owner_id: &str,
    provider: &str,
) -> Result<(), String> {
    let started_at = now_string();
    let db_path = usage_db_path()?;
    record_attribution_at(&db_path, owner_type, owner_id, provider, &started_at)
}

pub(crate) fn record_current_attribution_if_available() -> Result<(), String> {
    let state = get_codex_state_value();
    let mode = raw_string_field(&state, "mode");
    if mode == "api" {
        let settings = read_settings_value()?;
        let owner_id = string_field(&settings, "active_api_profile_id");
        if owner_id.is_empty() {
            return Ok(());
        }
        return record_attribution(OWNER_TYPE_API_PROFILE, &owner_id, PROVIDER_API);
    }

    if mode == "chatgpt" {
        let owner_id = string_field(&state, "profile_id");
        if owner_id.is_empty() {
            return Ok(());
        }
        return record_attribution(OWNER_TYPE_SUBSCRIPTION, &owner_id, PROVIDER_SUBSCRIPTION);
    }

    Ok(())
}

fn usage_stats_get_impl() -> Result<Value, String> {
    let _guard = USAGE_STATS_SCAN_LOCK
        .lock()
        .map_err(|_| "token 统计扫描锁异常".to_string())?;
    let db_path = usage_db_path()?;
    let codex_home = codex_dir()?;
    let scan_sources = default_usage_scan_sources(&codex_home)?;
    usage_stats_get_for_scan_sources(&db_path, &scan_sources, &now_string())
}

fn usage_stats_get_for_scan_sources(
    db_path: &Path,
    sources: &[UsageScanSource],
    now: &str,
) -> Result<Value, String> {
    let now_seconds =
        parse_rfc3339_seconds(now).ok_or_else(|| "token 统计当前时间无效".to_string())?;
    let mut connection = open_usage_connection(db_path, now)?;
    let counted_after_seconds = records_counted_after_seconds(&connection)?;
    // Files are read before any write, so the database stays unlocked while they are read;
    // `record_attribution` writes through its own connection meanwhile.
    let scan = scan_sources(&connection, sources, counted_after_seconds)?;
    // One transaction for every write of this refresh, committed only after the summary was
    // built. Any failure rolls all of it back and returns the error; the cursors then still
    // point before the new records, so the next refresh reads them again.
    let transaction = connection
        .transaction()
        .map_err(|err| db_error("开启 token 统计写入事务失败", err))?;
    write_scan(&transaction, &scan)?;
    let response = aggregate_usage(&transaction, now_seconds)?;
    transaction
        .commit()
        .map_err(|err| db_error("提交 token 统计写入失败", err))?;
    Ok(response)
}
