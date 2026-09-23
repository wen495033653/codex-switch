mod aggregate;
mod db;
mod model;
mod parse;
mod pricing;
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
use std::{
    path::Path,
    sync::{Mutex, OnceLock},
};
use {
    aggregate::{aggregate_usage_cached, AggregateCache},
    db::{
        db_error, meta_value, open_usage_connection, record_attribution_at, usage_db_path,
        META_STATS_STARTED_AT,
    },
    model::{
        ScanWarnings, UsageScanSource, OWNER_TYPE_API_PROFILE, OWNER_TYPE_SUBSCRIPTION,
        PROVIDER_API, PROVIDER_SUBSCRIPTION,
    },
    pricing::{count_unpriced_sessions, recompute_existing_costs_if_needed},
    scan::{load_session_scan_states, scan_codex_sessions, write_session_scan_results},
    sources::default_usage_scan_sources,
};

// The scan lock also owns the aggregation cache, so both are only touched by one caller at a time.
static USAGE_STATS_SCAN_LOCK: OnceLock<Mutex<Option<AggregateCache>>> = OnceLock::new();

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
    let mut cache = USAGE_STATS_SCAN_LOCK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "token 统计扫描锁异常".to_string())?;
    let db_path = usage_db_path()?;
    let codex_home = codex_dir()?;
    let scan_sources = default_usage_scan_sources(&codex_home)?;
    usage_stats_get_for_scan_sources(&db_path, &scan_sources, &now_string(), &mut cache)
}

#[cfg(test)]
fn usage_stats_get_for_paths(
    db_path: &Path,
    codex_home: &Path,
    now: &str,
) -> Result<Value, String> {
    usage_stats_get_for_scan_sources(
        db_path,
        &[sources::main_usage_scan_source(codex_home)],
        now,
        &mut None,
    )
}

fn usage_stats_get_for_scan_sources(
    db_path: &Path,
    scan_sources: &[UsageScanSource],
    now: &str,
    cache: &mut Option<AggregateCache>,
) -> Result<Value, String> {
    let now_seconds =
        parse_rfc3339_seconds(now).ok_or_else(|| "token 统计当前时间无效".to_string())?;
    let mut connection = open_usage_connection(db_path, now)?;
    let stats_started_at = meta_value(&connection, META_STATS_STARTED_AT)?;
    let stats_started_at_seconds = parse_rfc3339_seconds(&stats_started_at)
        .ok_or_else(|| "token 统计起始时间无效".to_string())?;
    let mut warnings = ScanWarnings::default();
    // 先一次性载入全部 cursor，避免每个 session 都单独查询 SQLite。
    let scan_states = load_session_scan_states(&connection)?;
    // Files are read and parsed before any write, so the database stays unlocked while the
    // JSONL is read; `record_attribution` writes through its own connection meanwhile.
    let mut writes = Vec::new();
    let mut database_changed = false;
    for source in scan_sources {
        database_changed |= scan_codex_sessions(
            &connection,
            source,
            stats_started_at_seconds,
            &mut warnings,
            &scan_states,
            &mut writes,
        )?;
    }
    // One transaction for every write of this refresh, committed only after the summary was
    // built. Any failure rolls all of it back and returns the error: the next refresh then sees
    // the same files as changed and cannot reuse an `AggregateCache` that predates them.
    let transaction = connection
        .transaction()
        .map_err(|err| db_error("开启 token 统计写入事务失败", err))?;
    write_session_scan_results(&transaction, &writes, now)?;
    database_changed |= recompute_existing_costs_if_needed(&transaction)?;
    warnings.missing_price = count_unpriced_sessions(&transaction)?;
    let response = aggregate_usage_cached(
        &transaction,
        db_path,
        now_seconds,
        &warnings,
        database_changed,
        cache,
    )?;
    transaction
        .commit()
        .map_err(|err| db_error("提交 token 统计写入失败", err))?;
    Ok(response)
}
