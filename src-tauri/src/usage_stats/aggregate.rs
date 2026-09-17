use super::{
    db::{db_error, sql_i64_to_u64},
    model::{
        ScanWarnings, TokenUsage, UsageWindowStarts, OWNER_TYPE_API_PROFILE,
        OWNER_TYPE_SUBSCRIPTION,
    },
    pricing::{estimate_cost, PRICING_SOURCE, PRICING_UPDATED_AT},
};
use rusqlite::{params, Connection};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use time::{OffsetDateTime, UtcOffset};

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

pub(super) fn aggregate_usage(
    connection: &Connection,
    now_seconds: i64,
    warnings: &ScanWarnings,
) -> Result<Value, String> {
    let window_starts = usage_window_starts(now_seconds);
    let mut subscriptions: BTreeMap<String, OwnerUsage> = BTreeMap::new();
    let mut api_profiles: BTreeMap<String, OwnerUsage> = BTreeMap::new();

    let mut statement = connection
        .prepare(
            r#"
            SELECT owner_type,
                   owner_id,
                   model,
                   updated_at,
                   updated_at_seconds,
                   input_tokens,
                   cached_input_tokens,
                   output_tokens,
                   reasoning_output_tokens,
                   total_tokens,
                   COALESCE(events.today_input_tokens, 0),
                   COALESCE(events.today_cached_input_tokens, 0),
                   COALESCE(events.today_output_tokens, 0),
                   COALESCE(events.today_reasoning_output_tokens, 0),
                   COALESCE(events.today_total_tokens, 0),
                   COALESCE(events.days_7_input_tokens, 0),
                   COALESCE(events.days_7_cached_input_tokens, 0),
                   COALESCE(events.days_7_output_tokens, 0),
                   COALESCE(events.days_7_reasoning_output_tokens, 0),
                   COALESCE(events.days_7_total_tokens, 0),
                   COALESCE(events.days_30_input_tokens, 0),
                   COALESCE(events.days_30_cached_input_tokens, 0),
                   COALESCE(events.days_30_output_tokens, 0),
                   COALESCE(events.days_30_reasoning_output_tokens, 0),
                   COALESCE(events.days_30_total_tokens, 0),
                   model_context_window,
                   estimated_cost_usd,
                   priced,
                   COALESCE(pricing_context, ''),
                   COALESCE(unpriced_reason, '')
            FROM session_usage
            LEFT JOIN (
                SELECT source_path,
                       SUM(CASE WHEN timestamp_seconds >= ?1 THEN input_tokens ELSE 0 END)
                           AS today_input_tokens,
                       SUM(CASE WHEN timestamp_seconds >= ?1 THEN cached_input_tokens ELSE 0 END)
                           AS today_cached_input_tokens,
                       SUM(CASE WHEN timestamp_seconds >= ?1 THEN output_tokens ELSE 0 END)
                           AS today_output_tokens,
                       SUM(CASE WHEN timestamp_seconds >= ?1 THEN reasoning_output_tokens ELSE 0 END)
                           AS today_reasoning_output_tokens,
                       SUM(CASE WHEN timestamp_seconds >= ?1 THEN total_tokens ELSE 0 END)
                           AS today_total_tokens,
                       SUM(CASE WHEN timestamp_seconds >= ?2 THEN input_tokens ELSE 0 END)
                           AS days_7_input_tokens,
                       SUM(CASE WHEN timestamp_seconds >= ?2 THEN cached_input_tokens ELSE 0 END)
                           AS days_7_cached_input_tokens,
                       SUM(CASE WHEN timestamp_seconds >= ?2 THEN output_tokens ELSE 0 END)
                           AS days_7_output_tokens,
                       SUM(CASE WHEN timestamp_seconds >= ?2 THEN reasoning_output_tokens ELSE 0 END)
                           AS days_7_reasoning_output_tokens,
                       SUM(CASE WHEN timestamp_seconds >= ?2 THEN total_tokens ELSE 0 END)
                           AS days_7_total_tokens,
                       SUM(CASE WHEN timestamp_seconds >= ?3 THEN input_tokens ELSE 0 END)
                           AS days_30_input_tokens,
                       SUM(CASE WHEN timestamp_seconds >= ?3 THEN cached_input_tokens ELSE 0 END)
                           AS days_30_cached_input_tokens,
                       SUM(CASE WHEN timestamp_seconds >= ?3 THEN output_tokens ELSE 0 END)
                           AS days_30_output_tokens,
                       SUM(CASE WHEN timestamp_seconds >= ?3 THEN reasoning_output_tokens ELSE 0 END)
                           AS days_30_reasoning_output_tokens,
                       SUM(CASE WHEN timestamp_seconds >= ?3 THEN total_tokens ELSE 0 END)
                           AS days_30_total_tokens
                FROM session_token_events
                WHERE timestamp_seconds >= ?3
                GROUP BY source_path
            ) AS events ON events.source_path = session_usage.source_path
            "#,
        )
        .map_err(|err| db_error("读取 session token 统计失败", err))?;
    let rows = statement
        .query_map(
            params![
                window_starts.today,
                window_starts.days_7,
                window_starts.days_30
            ],
            |row| {
                Ok(UsageRow {
                    owner_type: row.get(0)?,
                    owner_id: row.get(1)?,
                    model: row.get(2)?,
                    updated_at: row.get(3)?,
                    updated_at_seconds: row.get(4)?,
                    input_tokens: row.get::<_, i64>(5).map(sql_i64_to_u64)?,
                    cached_input_tokens: row.get::<_, i64>(6).map(sql_i64_to_u64)?,
                    output_tokens: row.get::<_, i64>(7).map(sql_i64_to_u64)?,
                    reasoning_output_tokens: row.get::<_, i64>(8).map(sql_i64_to_u64)?,
                    total_tokens: row.get::<_, i64>(9).map(sql_i64_to_u64)?,
                    today_usage: token_usage_from_row(row, 10)?,
                    days_7_usage: token_usage_from_row(row, 15)?,
                    days_30_usage: token_usage_from_row(row, 20)?,
                    model_context_window: row
                        .get::<_, Option<i64>>(25)?
                        .and_then(|value| u64::try_from(value).ok()),
                    estimated_cost_usd: row.get(26)?,
                    priced: row.get::<_, i64>(27)? == 1,
                    pricing_context: row.get(28)?,
                    unpriced_reason: row.get(29)?,
                })
            },
        )
        .map_err(|err| db_error("读取 session token 统计失败", err))?;

    for row in rows {
        let row = row.map_err(|err| db_error("读取 session token 统计失败", err))?;
        let target = match row.owner_type.as_str() {
            OWNER_TYPE_SUBSCRIPTION => subscriptions.entry(row.owner_id.clone()).or_default(),
            OWNER_TYPE_API_PROFILE => api_profiles.entry(row.owner_id.clone()).or_default(),
            _ => continue,
        };
        apply_row_to_window(&mut target.all, &row);
        apply_row_to_model_window(&mut target.all_by_model, &row);
        if row.today_usage.has_tokens() {
            apply_window_usage_to_window(&mut target.today, &row, &row.today_usage);
            apply_window_usage_to_model_window(&mut target.today_by_model, &row, &row.today_usage);
        }
        if row.days_7_usage.has_tokens() {
            apply_window_usage_to_window(&mut target.days_7, &row, &row.days_7_usage);
            apply_window_usage_to_model_window(
                &mut target.days_7_by_model,
                &row,
                &row.days_7_usage,
            );
        }
        if row.days_30_usage.has_tokens() {
            apply_window_usage_to_window(&mut target.days_30, &row, &row.days_30_usage);
            apply_window_usage_to_model_window(
                &mut target.days_30_by_model,
                &row,
                &row.days_30_usage,
            );
        }
    }

    Ok(json!({
        "ok": true,
        "pricing_source": PRICING_SOURCE,
        "pricing_updated_at": PRICING_UPDATED_AT,
        "subscriptions": owner_usage_map_to_json(subscriptions),
        "api_profiles": owner_usage_map_to_json(api_profiles),
        "warnings": warnings_to_json(warnings)
    }))
}

fn token_usage_from_row(
    row: &rusqlite::Row<'_>,
    start_index: usize,
) -> rusqlite::Result<TokenUsage> {
    Ok(TokenUsage {
        input_tokens: row.get::<_, i64>(start_index).map(sql_i64_to_u64)?,
        cached_input_tokens: row.get::<_, i64>(start_index + 1).map(sql_i64_to_u64)?,
        output_tokens: row.get::<_, i64>(start_index + 2).map(sql_i64_to_u64)?,
        reasoning_output_tokens: row.get::<_, i64>(start_index + 3).map(sql_i64_to_u64)?,
        total_tokens: row.get::<_, i64>(start_index + 4).map(sql_i64_to_u64)?,
    })
}

pub(super) fn usage_window_starts(now_seconds: i64) -> UsageWindowStarts {
    UsageWindowStarts {
        today: today_start_seconds(now_seconds),
        days_7: now_seconds.saturating_sub(7 * 24 * 60 * 60),
        days_30: now_seconds.saturating_sub(30 * 24 * 60 * 60),
    }
}

fn today_start_seconds(now_seconds: i64) -> i64 {
    let Ok(now_utc) = OffsetDateTime::from_unix_timestamp(now_seconds) else {
        return now_seconds;
    };
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let local_now = now_utc.to_offset(offset);
    local_now
        .date()
        .midnight()
        .assume_offset(offset)
        .unix_timestamp()
}

fn apply_row_to_window(window: &mut UsageWindow, row: &UsageRow) {
    let usage = TokenUsage {
        input_tokens: row.input_tokens,
        cached_input_tokens: row.cached_input_tokens,
        output_tokens: row.output_tokens,
        reasoning_output_tokens: row.reasoning_output_tokens,
        total_tokens: row.total_tokens,
    };
    apply_tokens_to_window(window, row, &usage);
    if row.priced {
        window.estimated_cost_usd += row.estimated_cost_usd.unwrap_or(0.0);
        if !row.pricing_context.is_empty() {
            increment_count(&mut window.pricing_contexts, &row.pricing_context);
        }
    } else {
        window.has_unpriced = true;
        if !row.unpriced_reason.is_empty() {
            increment_count(&mut window.unpriced_reasons, &row.unpriced_reason);
        }
    }
}

fn apply_window_usage_to_window(window: &mut UsageWindow, row: &UsageRow, usage: &TokenUsage) {
    apply_tokens_to_window(window, row, usage);
    let estimated = estimate_cost(&row.model, usage, row.model_context_window);
    if estimated.priced {
        window.estimated_cost_usd += estimated.cost_usd.unwrap_or(0.0);
        if let Some(pricing_context) = estimated.pricing_context {
            increment_count(&mut window.pricing_contexts, pricing_context);
        }
    } else {
        window.has_unpriced = true;
        if let Some(unpriced_reason) = estimated.unpriced_reason {
            increment_count(&mut window.unpriced_reasons, unpriced_reason);
        }
    }
}

fn apply_tokens_to_window(window: &mut UsageWindow, row: &UsageRow, usage: &TokenUsage) {
    window.input_tokens = window.input_tokens.saturating_add(usage.input_tokens);
    window.cached_input_tokens = window
        .cached_input_tokens
        .saturating_add(usage.cached_input_tokens);
    window.output_tokens = window.output_tokens.saturating_add(usage.output_tokens);
    window.reasoning_output_tokens = window
        .reasoning_output_tokens
        .saturating_add(usage.reasoning_output_tokens);
    window.total_tokens = window.total_tokens.saturating_add(usage.total_tokens);
    window.session_count = window.session_count.saturating_add(1);
    if row.updated_at_seconds >= window.last_used_seconds {
        window.last_used_seconds = row.updated_at_seconds;
        window.last_used = row.updated_at.clone();
    }
}

fn apply_row_to_model_window(windows: &mut BTreeMap<String, UsageWindow>, row: &UsageRow) {
    let model = display_model_id(&row.model);
    let window = windows.entry(model).or_default();
    apply_row_to_window(window, row);
}

fn apply_window_usage_to_model_window(
    windows: &mut BTreeMap<String, UsageWindow>,
    row: &UsageRow,
    usage: &TokenUsage,
) {
    let model = display_model_id(&row.model);
    let window = windows.entry(model).or_default();
    apply_window_usage_to_window(window, row, usage);
}

fn increment_count(counts: &mut BTreeMap<String, u64>, key: &str) {
    *counts.entry(key.to_string()).or_insert(0) += 1;
}

fn display_model_id(model: &str) -> String {
    let model = model.trim();
    if model.is_empty() {
        "unknown".to_string()
    } else {
        model.to_string()
    }
}

fn owner_usage_map_to_json(source: BTreeMap<String, OwnerUsage>) -> Value {
    let mut output = Map::new();
    for (owner_id, usage) in source {
        output.insert(owner_id, owner_usage_to_json(&usage));
    }
    Value::Object(output)
}

fn owner_usage_to_json(usage: &OwnerUsage) -> Value {
    json!({
        "today": usage_window_to_json_with_models(&usage.today, &usage.today_by_model),
        "days_7": usage_window_to_json_with_models(&usage.days_7, &usage.days_7_by_model),
        "days_30": usage_window_to_json_with_models(&usage.days_30, &usage.days_30_by_model),
        "all": usage_window_to_json_with_models(&usage.all, &usage.all_by_model)
    })
}

fn usage_window_to_json(window: &UsageWindow) -> Value {
    let cost = if window.has_unpriced {
        Value::Null
    } else {
        json!(window.estimated_cost_usd)
    };
    json!({
        "input_tokens": window.input_tokens,
        "cached_input_tokens": window.cached_input_tokens,
        "output_tokens": window.output_tokens,
        "reasoning_output_tokens": window.reasoning_output_tokens,
        "total_tokens": window.total_tokens,
        "estimated_cost_usd": cost,
        "priced": !window.has_unpriced,
        "pricing_contexts": map_counts_to_json(&window.pricing_contexts),
        "unpriced_reasons": map_counts_to_json(&window.unpriced_reasons),
        "session_count": window.session_count,
        "last_used": window.last_used
    })
}

fn usage_window_to_json_with_models(
    window: &UsageWindow,
    by_model: &BTreeMap<String, UsageWindow>,
) -> Value {
    let mut output = usage_window_to_json(window);
    if let Value::Object(ref mut object) = output {
        object.insert("by_model".to_string(), usage_model_map_to_json(by_model));
    }
    output
}

fn usage_model_map_to_json(source: &BTreeMap<String, UsageWindow>) -> Value {
    let mut output = Map::new();
    for (model, window) in source {
        output.insert(model.clone(), usage_window_to_json(window));
    }
    Value::Object(output)
}

fn map_counts_to_json(source: &BTreeMap<String, u64>) -> Value {
    let mut output = Map::new();
    for (key, value) in source {
        output.insert(key.clone(), json!(value));
    }
    Value::Object(output)
}

fn warnings_to_json(warnings: &ScanWarnings) -> Vec<String> {
    let mut output = Vec::new();
    if warnings.missing_attribution > 0 {
        output.push(format!(
            "{} 个 session 缺少 Codex Switch 归属记录，未计入卡片",
            warnings.missing_attribution
        ));
    }
    if warnings.missing_price > 0 {
        output.push(format!(
            "{} 个 session 缺少可用价格，仅显示 tokens",
            warnings.missing_price
        ));
    }
    output
}
