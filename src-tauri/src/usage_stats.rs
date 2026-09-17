use crate::{
    accounts::get_codex_state_value,
    json_util::{raw_string_field, string_field},
    paths::{app_data_dir, codex_dir, ensure_parent_dir},
    settings::read_settings_value,
    time_util::{now_string, parse_rfc3339_seconds},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};
use time::{OffsetDateTime, UtcOffset};

mod aggregate;
mod db;
mod parse;
mod pricing;
mod scan;
mod sources;
#[cfg(test)]
mod tests;

use aggregate::*;
use db::*;
use parse::*;
use pricing::*;
use scan::*;
use sources::*;

const OWNER_TYPE_SUBSCRIPTION: &str = "subscription";
const OWNER_TYPE_API_PROFILE: &str = "api_profile";
const PROVIDER_SUBSCRIPTION: &str = "openai";
const PROVIDER_API: &str = "api";
const CODEX_APP_INSTANCES_DIR: &str = "codex-app-instances";
const CODEX_APP_INSTANCE_MARKER_FILE: &str = "codex-switch-instance.json";
const META_STATS_STARTED_AT: &str = "stats_started_at";
const META_PRICING_UPDATED_AT: &str = "pricing_updated_at";
const PRICING_SOURCE: &str = "https://developers.openai.com/api/docs/pricing";
const PRICING_UPDATED_AT: &str = "2026-07-10";
const LONG_CONTEXT_THRESHOLD_TOKENS: u64 = 270_000;
const PRICING_CONTEXT_STANDARD_SHORT: &str = "standard_short_context";
const PRICING_CONTEXT_STANDARD_LONG: &str = "standard_long_context";
const UNPRICED_REASON_MISSING_MODEL_PRICE: &str = "missing_model_price";
const UNPRICED_REASON_MISSING_CACHED_INPUT_PRICE: &str = "missing_cached_input_price";
const SCAN_OUTCOME_INDEXED: &str = "indexed";
const SCAN_OUTCOME_IGNORED: &str = "ignored";
const SCAN_OUTCOME_DUPLICATE: &str = "duplicate";
const SCAN_OUTCOME_MISSING_ATTRIBUTION: &str = "missing_attribution";
const SCAN_OUTCOME_BEFORE_START: &str = "before_start";

static USAGE_STATS_SCAN_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Copy)]
struct TokenPrices {
    input_per_million: f64,
    cached_input_per_million: Option<f64>,
    output_per_million: f64,
}

#[derive(Clone, Copy)]
struct ModelPrice {
    model: &'static str,
    short_context: TokenPrices,
    long_context: Option<TokenPrices>,
    long_context_threshold: Option<u64>,
}

#[derive(Default)]
struct ScanWarnings {
    missing_attribution: u64,
    missing_price: u64,
    skipped_before_start: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SessionFileStamp {
    modified_nanos: i64,
    size: u64,
}

#[derive(Clone, Debug)]
struct SessionScanState {
    stamp: SessionFileStamp,
    scan_scope: String,
    session_id: String,
    outcome: String,
}

#[derive(Clone, Default)]
struct TokenUsage {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
}

impl TokenUsage {
    fn has_tokens(&self) -> bool {
        self.total_tokens > 0
    }

    fn add_assign(&mut self, other: &TokenUsage) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.cached_input_tokens = self
            .cached_input_tokens
            .saturating_add(other.cached_input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.reasoning_output_tokens = self
            .reasoning_output_tokens
            .saturating_add(other.reasoning_output_tokens);
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
    }

    fn saturating_delta(&self, previous: &TokenUsage) -> TokenUsage {
        TokenUsage {
            input_tokens: self.input_tokens.saturating_sub(previous.input_tokens),
            cached_input_tokens: self
                .cached_input_tokens
                .saturating_sub(previous.cached_input_tokens),
            output_tokens: self.output_tokens.saturating_sub(previous.output_tokens),
            reasoning_output_tokens: self
                .reasoning_output_tokens
                .saturating_sub(previous.reasoning_output_tokens),
            total_tokens: self.total_tokens.saturating_sub(previous.total_tokens),
        }
    }
}

#[derive(Clone, Copy)]
struct UsageWindowStarts {
    today: i64,
    days_7: i64,
    days_30: i64,
}

#[derive(Default)]
struct TokenUsageWindows {
    today: TokenUsage,
    days_7: TokenUsage,
    days_30: TokenUsage,
}

struct TokenUsageEvent {
    timestamp_seconds: i64,
    usage: TokenUsage,
}

#[derive(Clone)]
struct TimestampValue {
    raw: String,
    seconds: i64,
}

#[derive(Default)]
struct ParsedSession {
    session_id: String,
    provider: String,
    model: String,
    started_at: Option<TimestampValue>,
    updated_at: Option<TimestampValue>,
    usage: Option<TokenUsage>,
    model_context_window: Option<u64>,
    previous_event_usage: Option<TokenUsage>,
    window_usage: TokenUsageWindows,
    token_events: Vec<TokenUsageEvent>,
}

#[derive(Clone)]
struct OwnerAttribution {
    owner_type: String,
    owner_id: String,
}

struct UsageScanSource {
    codex_home: PathBuf,
    attribution_override: Option<OwnerAttribution>,
}

struct EstimatedCost {
    cost_usd: Option<f64>,
    priced: bool,
    pricing_context: Option<&'static str>,
    unpriced_reason: Option<&'static str>,
}

#[derive(Default)]
struct UsageWindow {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
    session_count: u64,
    estimated_cost_usd: f64,
    has_unpriced: bool,
    pricing_contexts: BTreeMap<String, u64>,
    unpriced_reasons: BTreeMap<String, u64>,
    last_used: String,
    last_used_seconds: i64,
}

#[derive(Default)]
struct OwnerUsage {
    today: UsageWindow,
    today_by_model: BTreeMap<String, UsageWindow>,
    days_7: UsageWindow,
    days_7_by_model: BTreeMap<String, UsageWindow>,
    days_30: UsageWindow,
    days_30_by_model: BTreeMap<String, UsageWindow>,
    all: UsageWindow,
    all_by_model: BTreeMap<String, UsageWindow>,
}

struct UsageRow {
    owner_type: String,
    owner_id: String,
    model: String,
    updated_at: String,
    updated_at_seconds: i64,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
    today_usage: TokenUsage,
    days_7_usage: TokenUsage,
    days_30_usage: TokenUsage,
    model_context_window: Option<u64>,
    estimated_cost_usd: Option<f64>,
    priced: bool,
    pricing_context: String,
    unpriced_reason: String,
}

#[tauri::command]
pub(crate) async fn usage_stats_get() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(usage_stats_get_impl)
        .await
        .map_err(|err| format!("读取 token 统计任务异常: {err}"))?
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
    let _scan_guard = USAGE_STATS_SCAN_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "token 统计扫描锁异常".to_string())?;
    let db_path = usage_db_path()?;
    let codex_home = codex_dir()?;
    let scan_sources = default_usage_scan_sources(&codex_home)?;
    usage_stats_get_for_scan_sources(&db_path, &scan_sources, &now_string())
}

#[cfg(test)]
fn usage_stats_get_for_paths(
    db_path: &Path,
    codex_home: &Path,
    now: &str,
) -> Result<Value, String> {
    usage_stats_get_for_scan_sources(db_path, &[main_usage_scan_source(codex_home)], now)
}

fn usage_stats_get_for_scan_sources(
    db_path: &Path,
    scan_sources: &[UsageScanSource],
    now: &str,
) -> Result<Value, String> {
    let now_seconds =
        parse_rfc3339_seconds(now).ok_or_else(|| "token 统计当前时间无效".to_string())?;
    let window_starts = usage_window_starts(now_seconds);
    let connection = open_usage_connection(db_path, now)?;
    let stats_started_at = meta_value(&connection, META_STATS_STARTED_AT)?;
    let stats_started_at_seconds = parse_rfc3339_seconds(&stats_started_at)
        .ok_or_else(|| "token 统计起始时间无效".to_string())?;
    let mut warnings = ScanWarnings::default();
    // 先一次性载入全部 cursor，避免每个 session 都单独查询 SQLite。
    let mut scan_states = load_session_scan_states(&connection)?;
    for source in scan_sources {
        scan_codex_sessions(
            &connection,
            source,
            &window_starts,
            stats_started_at_seconds,
            now,
            &mut warnings,
            &mut scan_states,
        )?;
    }
    recompute_existing_costs_if_needed(&connection)?;
    warnings.missing_price = count_unpriced_sessions(&connection)?;
    let response = aggregate_usage(&connection, now_seconds, &warnings)?;
    Ok(response)
}
