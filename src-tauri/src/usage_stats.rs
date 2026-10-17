mod aggregate;
mod bills;
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
    app_log::{log_event, log_event_once},
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
    bills::{list_bills, BillsQuery},
    db::{
        count_unpriced_with_model_price, db_error, mark_service_tier_unknown,
        mark_unpriced_checked, open_usage_connection, pending_reprice, record_attribution_at,
        records_counted_after_seconds, usage_db_path,
    },
    model::{
        UsageScanSource, OWNER_TYPE_API_PROFILE, OWNER_TYPE_SUBSCRIPTION, PROVIDER_API,
        PROVIDER_SUBSCRIPTION,
    },
    pricing::PRICING_UPDATED_AT,
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

/// One page of the bill: the stored responses of a window, optionally of one owner and one
/// model, newest first. It reads what the refreshes stored; it does not scan the rollouts.
#[tauri::command]
pub(crate) async fn usage_bills_list(
    range: String,
    owner_type: Option<String>,
    owner_id: Option<String>,
    model: Option<String>,
    page: u32,
    page_size: u32,
) -> Result<Value, String> {
    // The bills page asks again after every statistics refresh (every 30 s while it shows).
    run_blocking("读取 token 账单", move || {
        usage_bills_list_impl(&range, owner_type, owner_id, model, page, page_size)
            .inspect_err(|err| log_event_once("usage_bills_list_error", json!({ "error": err })))
    })
    .await
}

fn usage_bills_list_impl(
    range: &str,
    owner_type: Option<String>,
    owner_id: Option<String>,
    model: Option<String>,
    page: u32,
    page_size: u32,
) -> Result<Value, String> {
    let query = BillsQuery::new(
        range,
        owner_type.as_deref(),
        owner_id.as_deref(),
        model,
        page,
        page_size,
    )?;
    let now = now_string();
    let now_seconds =
        parse_rfc3339_seconds(&now).ok_or_else(|| "token 统计当前时间无效".to_string())?;
    let connection = open_usage_connection(&usage_db_path()?, &now)?;
    list_bills(&connection, &query, now_seconds)
}

/// Stores what the rollouts gained since the last refresh. The session sync calls it before it
/// rewrites provider lines: a record whose provider is only in its rollout's `session_meta`
/// would otherwise be read after the rewrite and attributed to the mode being switched to.
pub(crate) fn store_new_usage_records() -> Result<(), String> {
    usage_stats_get_impl().map(|_| ())
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
    // After a price table change, the records stored without a price are checked once: the
    // rollouts holding the ones the table now prices are read again for the tier they ran on,
    // and the ones none of them holds are marked as having an unknown tier.
    let reprice = pending_reprice(&connection)?;
    let reread_since = reprice.as_ref().and_then(|check| check.reread_since);
    // Files are read before any write, so the database stays unlocked while they are read;
    // `record_attribution` writes through its own connection meanwhile.
    let scan = scan_sources(&connection, sources, counted_after_seconds, reread_since)?;
    // One transaction for every write of this refresh, committed only after the summary was
    // built. Any failure rolls all of it back and returns the error; the cursors then still
    // point before the new records, so the next refresh reads them again.
    let transaction = connection
        .transaction()
        .map_err(|err| db_error("开启 token 统计写入事务失败", err))?;
    let written = write_scan(&transaction, &scan)?;
    let recheck = match reprice {
        Some(_) => {
            let tier_unknown = mark_service_tier_unknown(&transaction)?;
            mark_unpriced_checked(&transaction)?;
            Some((tier_unknown, count_unpriced_with_model_price(&transaction)?))
        }
        None => None,
    };
    let response = aggregate_usage(&transaction, now_seconds)?;
    transaction
        .commit()
        .map_err(|err| db_error("提交 token 统计写入失败", err))?;
    if let Some((tier_unknown, still_unpriced)) = recheck {
        log_event(
            "usage_stats_unpriced_rechecked",
            json!({
                "pricingUpdatedAt": PRICING_UPDATED_AT,
                "rereadSince": reread_since,
                "rereadFiles": scan.reread_files,
                "repriced": written.repriced,
                "markedTierUnknown": tier_unknown,
                "stillUnpricedWithModelPrice": still_unpriced
            }),
        );
    }
    Ok(response)
}
