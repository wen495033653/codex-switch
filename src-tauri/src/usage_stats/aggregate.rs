//! Sums the stored records into the cards' windows.
//!
//! "All" is read from `usage_totals`. Today, 7 days and 30 days are exact to the second: whole
//! hours inside a window come from `usage_hourly`, and the part before the window's first whole
//! hour from the individual records. A refresh therefore reads about one bucket per active
//! session-hour of the last 30 days, not every record.

use super::{
    db::{db_error, hour_start, sql_i64_to_u64},
    model::{
        TokenUsage, UsageItem, UsageWindowStarts, OWNER_TYPE_API_PROFILE, OWNER_TYPE_SUBSCRIPTION,
        OWNER_TYPE_UNATTRIBUTED,
    },
    pricing::{is_priced_label, PRICING_SOURCE, PRICING_UPDATED_AT},
};
use rusqlite::{params, Connection, Row};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashSet};
use time::{format_description::well_known::Rfc3339, OffsetDateTime, UtcOffset};

const HOUR_SECONDS: i64 = 60 * 60;

#[derive(Default)]
struct UsageWindow {
    usage: TokenUsage,
    estimated_cost_usd: f64,
    has_unpriced: bool,
    pricing_contexts: BTreeMap<String, u64>,
    unpriced_reasons: BTreeMap<String, u64>,
    threads: HashSet<String>,
    last_used_seconds: i64,
}

impl UsageWindow {
    fn add(&mut self, item: &UsageItem) {
        self.usage.add(&item.usage);
        match item.cost_usd {
            Some(cost) => {
                self.estimated_cost_usd += cost;
                *self
                    .pricing_contexts
                    .entry(item.price_label.clone())
                    .or_insert(0) += item.record_count;
            }
            None => {
                self.has_unpriced = true;
                *self
                    .unpriced_reasons
                    .entry(item.price_label.clone())
                    .or_insert(0) += item.record_count;
            }
        }
        self.threads.insert(item.thread_id.clone());
        self.last_used_seconds = self.last_used_seconds.max(item.last_used_seconds);
    }
}

#[derive(Default)]
struct WindowUsage {
    total: UsageWindow,
    by_model: BTreeMap<String, UsageWindow>,
}

impl WindowUsage {
    fn add(&mut self, item: &UsageItem) {
        self.total.add(item);
        self.by_model
            .entry(display_model_id(&item.model))
            .or_default()
            .add(item);
    }
}

#[derive(Default)]
struct OwnerUsage {
    today: WindowUsage,
    days_7: WindowUsage,
    days_30: WindowUsage,
    all: WindowUsage,
}

#[derive(Clone, Copy)]
enum Window {
    Today,
    Days7,
    Days30,
}

#[derive(Default)]
struct Owners {
    subscriptions: BTreeMap<String, OwnerUsage>,
    api_profiles: BTreeMap<String, OwnerUsage>,
}

impl Owners {
    fn owner(&mut self, item: &UsageItem) -> Option<&mut OwnerUsage> {
        match item.owner_type.as_str() {
            OWNER_TYPE_SUBSCRIPTION => {
                Some(self.subscriptions.entry(item.owner_id.clone()).or_default())
            }
            OWNER_TYPE_API_PROFILE => {
                Some(self.api_profiles.entry(item.owner_id.clone()).or_default())
            }
            _ => None,
        }
    }

    fn add_to_window(&mut self, window: Window, item: &UsageItem) {
        if let Some(owner) = self.owner(item) {
            match window {
                Window::Today => owner.today.add(item),
                Window::Days7 => owner.days_7.add(item),
                Window::Days30 => owner.days_30.add(item),
            }
        }
    }
}

pub(super) fn aggregate_usage(connection: &Connection, now_seconds: i64) -> Result<Value, String> {
    let starts = usage_window_starts(now_seconds);
    let mut owners = Owners::default();
    let mut unattributed_threads = HashSet::new();
    let mut unpriced_threads = HashSet::new();

    for_each_item(
        connection,
        "SELECT owner_type, owner_id, model, thread_id, price_label, input_tokens, \
         cached_input_tokens, cache_write_input_tokens, output_tokens, reasoning_output_tokens, \
         total_tokens, estimated_cost_usd, record_count, last_used_seconds FROM usage_totals",
        params![],
        |item: UsageItem| {
            if item.owner_type == OWNER_TYPE_UNATTRIBUTED {
                unattributed_threads.insert(item.thread_id.clone());
            } else if let Some(owner) = owners.owner(&item) {
                if item.cost_usd.is_none() {
                    unpriced_threads.insert(item.thread_id.clone());
                }
                owner.all.add(&item);
            }
        },
    )?;

    let windows = [
        (Window::Days30, starts.days_30),
        (Window::Days7, starts.days_7),
        (Window::Today, starts.today),
    ];
    let first_whole_hour = |start: i64| {
        let hour = hour_start(start);
        if hour == start {
            hour
        } else {
            hour + HOUR_SECONDS
        }
    };
    for_each_item(
        connection,
        "SELECT owner_type, owner_id, model, thread_id, price_label, input_tokens, \
         cached_input_tokens, cache_write_input_tokens, output_tokens, reasoning_output_tokens, \
         total_tokens, estimated_cost_usd, record_count, last_used_seconds, hour_start \
         FROM usage_hourly WHERE hour_start >= ?1",
        params![first_whole_hour(starts.days_30)],
        |item: HourlyItem| {
            for (window, start) in windows {
                if item.hour_start >= first_whole_hour(start) {
                    owners.add_to_window(window, &item.item);
                }
            }
        },
    )?;
    for (window, start) in windows {
        for_each_item(
            connection,
            "SELECT owner_type, owner_id, model, thread_id, price_label, input_tokens, \
             cached_input_tokens, cache_write_input_tokens, output_tokens, \
             reasoning_output_tokens, total_tokens, estimated_cost_usd, 1, timestamp_seconds \
             FROM usage_records WHERE timestamp_seconds >= ?1 AND timestamp_seconds < ?2",
            params![start, first_whole_hour(start)],
            |item: UsageItem| owners.add_to_window(window, &item),
        )?;
    }

    Ok(json!({
        "ok": true,
        "pricing_source": PRICING_SOURCE,
        "pricing_updated_at": PRICING_UPDATED_AT,
        "subscriptions": owner_usage_map_to_json(owners.subscriptions),
        "api_profiles": owner_usage_map_to_json(owners.api_profiles),
        "warnings": warnings_to_json(unattributed_threads.len(), unpriced_threads.len())
    }))
}

struct HourlyItem {
    item: UsageItem,
    hour_start: i64,
}

trait FromUsageRow: Sized {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self>;
}

impl FromUsageRow for UsageItem {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let price_label: String = row.get(4)?;
        let cost: Option<f64> = row.get(11)?;
        Ok(UsageItem {
            owner_type: row.get(0)?,
            owner_id: row.get(1)?,
            model: row.get(2)?,
            thread_id: row.get(3)?,
            usage: TokenUsage {
                input_tokens: row.get::<_, i64>(5).map(sql_i64_to_u64)?,
                cached_input_tokens: row.get::<_, i64>(6).map(sql_i64_to_u64)?,
                cache_write_input_tokens: row.get::<_, i64>(7).map(sql_i64_to_u64)?,
                output_tokens: row.get::<_, i64>(8).map(sql_i64_to_u64)?,
                reasoning_output_tokens: row.get::<_, i64>(9).map(sql_i64_to_u64)?,
                total_tokens: row.get::<_, i64>(10).map(sql_i64_to_u64)?,
            },
            // A rollup stores 0 for unpriced groups; the label says which case it is.
            cost_usd: if is_priced_label(&price_label) {
                Some(cost.unwrap_or(0.0))
            } else {
                None
            },
            price_label,
            record_count: row.get::<_, i64>(12).map(sql_i64_to_u64)?,
            last_used_seconds: row.get(13)?,
        })
    }
}

impl FromUsageRow for HourlyItem {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(HourlyItem {
            item: UsageItem::from_row(row)?,
            hour_start: row.get(14)?,
        })
    }
}

fn for_each_item<T: FromUsageRow>(
    connection: &Connection,
    sql: &str,
    values: &[&dyn rusqlite::ToSql],
    mut apply: impl FnMut(T),
) -> Result<(), String> {
    let mut statement = connection
        .prepare_cached(sql)
        .map_err(|err| db_error("读取 token 统计失败", err))?;
    let mut rows = statement
        .query(values)
        .map_err(|err| db_error("读取 token 统计失败", err))?;
    while let Some(row) = rows
        .next()
        .map_err(|err| db_error("读取 token 统计失败", err))?
    {
        apply(T::from_row(row).map_err(|err| db_error("读取 token 统计失败", err))?);
    }
    Ok(())
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
        output.insert(
            owner_id,
            json!({
                "today": window_usage_to_json(&usage.today),
                "days_7": window_usage_to_json(&usage.days_7),
                "days_30": window_usage_to_json(&usage.days_30),
                "all": window_usage_to_json(&usage.all)
            }),
        );
    }
    Value::Object(output)
}

fn window_usage_to_json(window: &WindowUsage) -> Value {
    let mut output = usage_window_to_json(&window.total);
    let by_model = window
        .by_model
        .iter()
        .map(|(model, usage)| (model.clone(), usage_window_to_json(usage)))
        .collect::<Map<_, _>>();
    if let Value::Object(ref mut object) = output {
        object.insert("by_model".to_string(), Value::Object(by_model));
    }
    output
}

fn usage_window_to_json(window: &UsageWindow) -> Value {
    let cost = if window.has_unpriced {
        Value::Null
    } else {
        json!(window.estimated_cost_usd)
    };
    json!({
        "input_tokens": window.usage.input_tokens,
        "cached_input_tokens": window.usage.cached_input_tokens,
        "output_tokens": window.usage.output_tokens,
        "reasoning_output_tokens": window.usage.reasoning_output_tokens,
        "total_tokens": window.usage.total_tokens,
        "estimated_cost_usd": cost,
        "priced": !window.has_unpriced,
        "pricing_contexts": window.pricing_contexts,
        "unpriced_reasons": window.unpriced_reasons,
        "session_count": window.threads.len(),
        "last_used": format_last_used(window.last_used_seconds)
    })
}

fn format_last_used(seconds: i64) -> String {
    if seconds <= 0 {
        return String::new();
    }
    OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|time| time.format(&Rfc3339).ok())
        .unwrap_or_default()
}

fn warnings_to_json(unattributed_sessions: usize, unpriced_sessions: usize) -> Vec<String> {
    let mut output = Vec::new();
    if unattributed_sessions > 0 {
        output.push(format!(
            "{unattributed_sessions} 个 session 缺少 Codex Switch 归属记录，未计入卡片"
        ));
    }
    if unpriced_sessions > 0 {
        output.push(format!(
            "{unpriced_sessions} 个 session 缺少可用价格，仅显示 tokens"
        ));
    }
    output
}
