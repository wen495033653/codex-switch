use super::{
    model::{
        EstimatedCost, OwnerAttribution, RequestTiming, ServiceTier, TokenPrices, TokenUsage,
        UsageRecord, OWNER_TYPE_API_PROFILE, OWNER_TYPE_SUBSCRIPTION, PROVIDER_API,
        PROVIDER_SUBSCRIPTION,
    },
    pricing::{
        cost_parts, estimate_request_cost, estimate_summed_cost, has_model_price, prices_for_label,
        PRICING_UPDATED_AT, UNPRICED_MISSING_MODEL_PRICE, UNPRICED_MISSING_SERVICE_TIER,
    },
    records::RolloutCursor,
};
use crate::{
    app_log::log_event,
    paths::{app_data_dir, ensure_parent_dir},
    time_util::parse_rfc3339_seconds,
};
use rusqlite::{params, Connection, OptionalExtension, ToSql, Transaction, TransactionBehavior};
use serde_json::json;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// Records at or before this instant are never counted (RFC 3339). For a new database it is the
/// creation time; for one migrated from the per-session statistics it is the last moment those
/// statistics covered, so every response is counted by exactly one of the two.
pub(super) const META_RECORDS_COUNTED_AFTER: &str = "records_counted_after";

/// The price table (its `PRICING_UPDATED_AT`) the records stored without a price were last
/// checked against. Each new table is checked once; see `pending_reprice`.
const META_UNPRICED_CHECKED_FOR: &str = "unpriced_records_checked_for";

/// Set once the records stored before durations were kept got theirs from a re-read of the
/// rollouts.
const META_TIMINGS_FILLED: &str = "request_timings_filled";

/// Records migrated from the per-session statistics, not single API responses.
const LEGACY_RESPONSE_ID_PREFIX: &str = "legacy:";

const HOUR_SECONDS: i64 = 60 * 60;

const WINDOW_30_DAYS_SECONDS: i64 = 30 * 24 * HOUR_SECONDS;

pub(super) fn sql_i64_to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn u64_to_sql(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub(super) fn is_legacy_response_id(response_id: &str) -> bool {
    response_id.starts_with(LEGACY_RESPONSE_ID_PREFIX)
}

pub(super) fn usage_db_path() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join("usage-stats.sqlite"))
}

pub(super) fn db_error(context: &str, err: rusqlite::Error) -> String {
    format!("{context}: {err}")
}

pub(super) fn hour_start(timestamp_seconds: i64) -> i64 {
    timestamp_seconds - timestamp_seconds.rem_euclid(HOUR_SECONDS)
}

pub(super) fn open_usage_connection(path: &Path, now: &str) -> Result<Connection, String> {
    ensure_parent_dir(path)?;
    let mut connection =
        Connection::open(path).map_err(|err| db_error("打开 token 统计库失败", err))?;
    ensure_database(&mut connection, now)?;
    Ok(connection)
}

fn ensure_database(connection: &mut Connection, now: &str) -> Result<(), String> {
    // Immediate: two connections opening a new database at once must not both migrate it.
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|err| db_error("开启 token 统计库初始化事务失败", err))?;
    transaction
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS attribution (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                owner_type TEXT NOT NULL,
                owner_id TEXT NOT NULL,
                provider TEXT NOT NULL,
                started_at TEXT NOT NULL,
                started_at_seconds INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS attribution_provider_time_idx
                ON attribution(provider, started_at_seconds);
            CREATE TABLE IF NOT EXISTS usage_records (
                response_id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                owner_type TEXT NOT NULL,
                owner_id TEXT NOT NULL,
                model TEXT NOT NULL,
                timestamp_seconds INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL,
                cache_write_input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                reasoning_output_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL,
                price_label TEXT NOT NULL,
                estimated_cost_usd REAL,
                input_price REAL,
                cached_input_price REAL,
                cache_write_price REAL,
                output_price REAL,
                duration_ms INTEGER,
                first_token_ms INTEGER
            ) WITHOUT ROWID;
            CREATE INDEX IF NOT EXISTS usage_records_time_idx
                ON usage_records(timestamp_seconds);
            CREATE TABLE IF NOT EXISTS usage_hourly (
                hour_start INTEGER NOT NULL,
                owner_type TEXT NOT NULL,
                owner_id TEXT NOT NULL,
                model TEXT NOT NULL,
                thread_id TEXT NOT NULL,
                price_label TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL,
                cache_write_input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                reasoning_output_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL,
                estimated_cost_usd REAL NOT NULL,
                record_count INTEGER NOT NULL,
                last_used_seconds INTEGER NOT NULL,
                PRIMARY KEY(hour_start, owner_type, owner_id, model, thread_id, price_label)
            ) WITHOUT ROWID;
            CREATE TABLE IF NOT EXISTS usage_totals (
                owner_type TEXT NOT NULL,
                owner_id TEXT NOT NULL,
                model TEXT NOT NULL,
                thread_id TEXT NOT NULL,
                price_label TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL,
                cache_write_input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                reasoning_output_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL,
                estimated_cost_usd REAL NOT NULL,
                record_count INTEGER NOT NULL,
                last_used_seconds INTEGER NOT NULL,
                PRIMARY KEY(owner_type, owner_id, model, thread_id, price_label)
            ) WITHOUT ROWID;
            CREATE TABLE IF NOT EXISTS rollout_cursors (
                source_path TEXT PRIMARY KEY,
                modified_nanos INTEGER NOT NULL,
                file_size INTEGER NOT NULL,
                byte_offset INTEGER NOT NULL,
                resume_check BLOB NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                service_tier TEXT NOT NULL,
                request_started_ms INTEGER NOT NULL DEFAULT 0,
                output_started INTEGER NOT NULL DEFAULT 0
            );
            "#,
        )
        .map_err(|err| db_error("初始化 token 统计库失败", err))?;
    add_price_columns(&transaction)?;
    for (table, column, definition) in [
        ("usage_records", "duration_ms", "INTEGER"),
        ("usage_records", "first_token_ms", "INTEGER"),
        (
            "rollout_cursors",
            "request_started_ms",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "rollout_cursors",
            "output_started",
            "INTEGER NOT NULL DEFAULT 0",
        ),
    ] {
        add_column(&transaction, table, column, definition)?;
    }

    if optional_meta_value(&transaction, META_RECORDS_COUNTED_AFTER)?.is_none() {
        let counted_after = if table_exists(&transaction, "session_usage")? {
            migrate_session_statistics(&transaction, now)?
        } else {
            now.to_string()
        };
        set_meta_value(&transaction, META_RECORDS_COUNTED_AFTER, &counted_after)?;
    }
    transaction
        .commit()
        .map_err(|err| db_error("提交 token 统计库初始化失败", err))
}

/// Adds a column a database created by an earlier version lacks.
fn add_column(
    transaction: &Transaction<'_>,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let exists = transaction
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1)"),
            [column],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| db_error("读取 token 统计库结构失败", err))?;
    if exists == 1 {
        return Ok(());
    }
    transaction
        .execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))
        .map_err(|err| db_error(&format!("添加 token 统计库列 {table}.{column} 失败"), err))
}

fn table_exists(connection: &Connection, name: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [name],
            |row| row.get::<_, i64>(0),
        )
        .map(|found| found == 1)
        .map_err(|err| db_error("读取 token 统计库结构失败", err))
}

fn optional_meta_value(connection: &Connection, key: &str) -> Result<Option<String>, String> {
    connection
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|err| db_error("读取 token 统计元数据失败", err))
}

fn set_meta_value(connection: &Connection, key: &str, value: &str) -> Result<(), String> {
    connection
        .execute(
            r#"
            INSERT INTO meta(key, value) VALUES(?1, ?2)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            "#,
            params![key, value],
        )
        .map(|_| ())
        .map_err(|err| db_error("写入 token 统计元数据失败", err))
}

pub(super) fn records_counted_after(connection: &Connection) -> Result<String, String> {
    optional_meta_value(connection, META_RECORDS_COUNTED_AFTER)?
        .ok_or_else(|| "token 统计缺少记录起点".to_string())
}

pub(super) fn records_counted_after_seconds(connection: &Connection) -> Result<i64, String> {
    let value = records_counted_after(connection)?;
    parse_rfc3339_seconds(&value).ok_or_else(|| format!("token 统计记录起点无效: {value:?}"))
}

/// Whether this database was migrated from the per-session statistics of earlier versions, whose
/// all-time totals have no records behind them.
pub(super) fn has_session_statistics(connection: &Connection) -> Result<bool, String> {
    table_exists(connection, "session_usage")
}

/// Records stored before the prices were kept get them from their model and price label. All of
/// them were priced with the table as it still is (it did not change between 6.0.5, which
/// introduced per-response records, and these columns), and a record only gets prices that
/// reproduce its stored cost exactly, so none shows prices it was not charged at.
fn add_price_columns(transaction: &Transaction<'_>) -> Result<(), String> {
    let has_price_columns = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('usage_records') WHERE name = 'input_price')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| db_error("读取 token 记录表结构失败", err))?;
    if has_price_columns == 1 {
        return Ok(());
    }
    transaction
        .execute_batch(
            r#"
            ALTER TABLE usage_records ADD COLUMN input_price REAL;
            ALTER TABLE usage_records ADD COLUMN cached_input_price REAL;
            ALTER TABLE usage_records ADD COLUMN cache_write_price REAL;
            ALTER TABLE usage_records ADD COLUMN output_price REAL;
            "#,
        )
        .map_err(|err| db_error("添加 token 记录单价列失败", err))?;

    let priced = {
        let mut statement = transaction
            .prepare(
                r#"
                SELECT response_id, model, price_label, input_tokens, cached_input_tokens,
                       cache_write_input_tokens, output_tokens, estimated_cost_usd
                FROM usage_records WHERE estimated_cost_usd IS NOT NULL
                "#,
            )
            .map_err(|err| db_error("读取需要回填单价的 token 记录失败", err))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    TokenUsage {
                        input_tokens: row.get::<_, i64>(3).map(sql_i64_to_u64)?,
                        cached_input_tokens: row.get::<_, i64>(4).map(sql_i64_to_u64)?,
                        cache_write_input_tokens: row.get::<_, i64>(5).map(sql_i64_to_u64)?,
                        output_tokens: row.get::<_, i64>(6).map(sql_i64_to_u64)?,
                        reasoning_output_tokens: 0,
                        total_tokens: 0,
                    },
                    row.get::<_, f64>(7)?,
                ))
            })
            .map_err(|err| db_error("读取需要回填单价的 token 记录失败", err))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| db_error("读取需要回填单价的 token 记录失败", err))?
    };

    let mut filled = 0usize;
    let mut unmatched = Vec::new();
    for (response_id, model, label, usage, stored_cost) in priced {
        let prices = prices_for_label(&model, &label).filter(|prices| {
            cost_parts(&usage, prices).is_ok_and(|parts| parts.total() == stored_cost)
        });
        match prices {
            Some(prices) => {
                set_record_prices(transaction, &response_id, Some(&prices))?;
                filled += 1;
            }
            None => unmatched.push(json!({
                "responseId": response_id,
                "model": model,
                "priceLabel": label,
                "storedCostUsd": stored_cost
            })),
        }
    }
    log_event(
        "usage_stats_prices_backfilled",
        json!({ "filled": filled, "unmatched": unmatched.len() }),
    );
    if !unmatched.is_empty() {
        log_event(
            "usage_stats_price_backfill_error",
            json!({
                "unmatched": unmatched.len(),
                "examples": unmatched.iter().take(5).collect::<Vec<_>>(),
                "handling": "prices_left_empty_total_kept"
            }),
        );
    }
    Ok(())
}

fn set_record_prices(
    transaction: &Transaction<'_>,
    response_id: &str,
    prices: Option<&TokenPrices>,
) -> Result<(), String> {
    transaction
        .prepare_cached(
            r#"
            UPDATE usage_records
            SET input_price = ?2, cached_input_price = ?3, cache_write_price = ?4,
                output_price = ?5
            WHERE response_id = ?1
            "#,
        )
        .and_then(|mut statement| {
            statement.execute(params![
                response_id,
                prices.map(|prices| prices.input),
                prices.and_then(|prices| prices.cached_input),
                prices.and_then(|prices| prices.cache_write),
                prices.map(|prices| prices.output)
            ])
        })
        .map(|_| ())
        .map_err(|err| db_error("写入 token 记录单价失败", err))
}

/// A check of the records stored without a price against the current price table.
pub(super) struct RepriceCheck {
    /// Rollouts modified at or after this second are read again from the start: they hold the
    /// unpriced records whose model the table now prices, and only the rollout tells the tier
    /// each ran on. `None` when no record needs it.
    pub(super) reread_since: Option<i64>,
}

/// `None` once the records stored without a price were checked against this price table.
pub(super) fn pending_reprice(connection: &Connection) -> Result<Option<RepriceCheck>, String> {
    if optional_meta_value(connection, META_UNPRICED_CHECKED_FOR)?.as_deref()
        == Some(PRICING_UPDATED_AT)
    {
        return Ok(None);
    }
    let mut statement = connection
        .prepare(
            r#"
            SELECT model, MIN(timestamp_seconds) FROM usage_records
            WHERE estimated_cost_usd IS NULL AND response_id NOT LIKE 'legacy:%'
              AND price_label <> ?1
            GROUP BY model
            "#,
        )
        .map_err(|err| db_error("读取未定价 token 记录失败", err))?;
    let earliest = statement
        .query_map([UNPRICED_MISSING_SERVICE_TIER], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|err| db_error("读取未定价 token 记录失败", err))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| db_error("读取未定价 token 记录失败", err))?
        .into_iter()
        .filter(|(model, _)| has_model_price(model))
        .map(|(_, earliest)| earliest)
        .min();
    Ok(Some(RepriceCheck {
        reread_since: earliest,
    }))
}

pub(super) fn mark_unpriced_checked(transaction: &Transaction<'_>) -> Result<(), String> {
    set_meta_value(transaction, META_UNPRICED_CHECKED_FOR, PRICING_UPDATED_AT)
}

/// Until the records stored before durations were kept got them: the rollouts modified after
/// the records' start are read again once, from this second.
pub(super) fn pending_timing_fill(connection: &Connection) -> Result<Option<i64>, String> {
    if optional_meta_value(connection, META_TIMINGS_FILLED)?.is_some() {
        return Ok(None);
    }
    records_counted_after_seconds(connection).map(Some)
}

pub(super) fn mark_timings_filled(transaction: &Transaction<'_>, now: &str) -> Result<(), String> {
    set_meta_value(transaction, META_TIMINGS_FILLED, now)
}

/// A response read again gets the duration and first-token time its record lacks. Returns
/// whether the record changed.
pub(super) fn fill_record_timing(
    transaction: &Transaction<'_>,
    record: &UsageRecord,
) -> Result<bool, String> {
    if record.timing == RequestTiming::default() {
        return Ok(false);
    }
    transaction
        .prepare_cached(
            r#"
            UPDATE usage_records
            SET duration_ms = COALESCE(duration_ms, ?2),
                first_token_ms = COALESCE(first_token_ms, ?3)
            WHERE response_id = ?1
              AND ((duration_ms IS NULL AND ?2 IS NOT NULL)
                OR (first_token_ms IS NULL AND ?3 IS NOT NULL))
            "#,
        )
        .and_then(|mut statement| {
            statement.execute(params![
                record.response_id,
                record.timing.duration_ms,
                record.timing.first_token_ms
            ])
        })
        .map(|changed| changed == 1)
        .map_err(|err| db_error("写入 token 记录耗时失败", err))
}

pub(super) fn count_unpriced_with_model_price(connection: &Connection) -> Result<usize, String> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT model, COUNT(*) FROM usage_records
            WHERE estimated_cost_usd IS NULL AND response_id NOT LIKE 'legacy:%'
            GROUP BY model
            "#,
        )
        .map_err(|err| db_error("读取未定价 token 记录失败", err))?;
    let counts = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|err| db_error("读取未定价 token 记录失败", err))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| db_error("读取未定价 token 记录失败", err))?;
    Ok(counts
        .into_iter()
        .filter(|(model, _)| has_model_price(model))
        .map(|(_, count)| usize::try_from(count).unwrap_or(0))
        .sum())
}

/// Carries the per-session statistics of earlier versions into the per-response tables, once.
/// Their all-time totals become one `usage_totals` row per session, and their token events of
/// the last 30 days become records, so the day, 7 day and 30 day windows keep showing them
/// until they age out. Earlier versions kept no per-request sizes or service tiers, so these
/// are priced at standard, short-context prices. The old tables are left as they are.
/// Returns the instant the old statistics covered up to.
fn migrate_session_statistics(transaction: &Transaction<'_>, now: &str) -> Result<String, String> {
    let mut candidates = Vec::new();
    candidates.extend(optional_meta_value(transaction, "stats_started_at")?);
    if table_exists(transaction, "session_scan_state")? {
        candidates.extend(
            transaction
                .query_row(
                    "SELECT MAX(last_scanned_at) FROM session_scan_state",
                    [],
                    |row| row.get::<_, Option<String>>(0),
                )
                .map_err(|err| db_error("读取旧 token 统计扫描时间失败", err))?,
        );
    }
    let counted_after = candidates
        .into_iter()
        .filter_map(|value| parse_rfc3339_seconds(&value).map(|seconds| (seconds, value)))
        .max_by_key(|(seconds, _)| *seconds)
        .map(|(_, value)| value)
        .unwrap_or_else(|| now.to_string());
    let counted_after_seconds = parse_rfc3339_seconds(&counted_after)
        .ok_or_else(|| format!("旧 token 统计的截止时间无效: {counted_after:?}"))?;

    let sessions = read_legacy_sessions(transaction)?;
    for session in sessions.values() {
        let cost = estimate_summed_cost(&session.model, &session.usage);
        add_to_totals(
            transaction,
            &UsageRecord {
                response_id: String::new(),
                thread_id: session.session_id.clone(),
                timestamp_seconds: session.updated_at_seconds,
                owner: session.owner.clone(),
                model: session.model.clone(),
                usage: session.usage.clone(),
                service_tier: ServiceTier::Standard,
                cost,
                timing: RequestTiming::default(),
            },
        )?;
    }

    if table_exists(transaction, "session_token_events")? {
        let mut statement = transaction
            .prepare(
                r#"
                SELECT source_path, event_index, timestamp_seconds, input_tokens,
                       cached_input_tokens, output_tokens, reasoning_output_tokens, total_tokens
                FROM session_token_events
                WHERE timestamp_seconds >= ?1 AND timestamp_seconds <= ?2
                "#,
            )
            .map_err(|err| db_error("读取旧 token 事件失败", err))?;
        let events = statement
            .query_map(
                params![
                    counted_after_seconds - WINDOW_30_DAYS_SECONDS,
                    counted_after_seconds
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        TokenUsage {
                            input_tokens: row.get::<_, i64>(3).map(sql_i64_to_u64)?,
                            cached_input_tokens: row.get::<_, i64>(4).map(sql_i64_to_u64)?,
                            cache_write_input_tokens: 0,
                            output_tokens: row.get::<_, i64>(5).map(sql_i64_to_u64)?,
                            reasoning_output_tokens: row.get::<_, i64>(6).map(sql_i64_to_u64)?,
                            total_tokens: row.get::<_, i64>(7).map(sql_i64_to_u64)?,
                        },
                    ))
                },
            )
            .map_err(|err| db_error("读取旧 token 事件失败", err))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| db_error("读取旧 token 事件失败", err))?;
        for (source_path, event_index, timestamp_seconds, usage) in events {
            // The old windows only counted events of indexed sessions.
            let Some(session) = sessions.get(&source_path) else {
                continue;
            };
            let cost = estimate_summed_cost(&session.model, &usage);
            insert_record(
                transaction,
                &UsageRecord {
                    response_id: format!("{LEGACY_RESPONSE_ID_PREFIX}{source_path}:{event_index}"),
                    thread_id: session.session_id.clone(),
                    timestamp_seconds,
                    owner: session.owner.clone(),
                    model: session.model.clone(),
                    usage,
                    service_tier: ServiceTier::Standard,
                    cost,
                    timing: RequestTiming::default(),
                },
                false,
            )?;
        }
    }
    Ok(counted_after)
}

struct LegacySession {
    session_id: String,
    owner: OwnerAttribution,
    model: String,
    updated_at_seconds: i64,
    usage: TokenUsage,
}

/// Keyed by `source_path`, which the old token events refer to.
fn read_legacy_sessions(
    transaction: &Transaction<'_>,
) -> Result<HashMap<String, LegacySession>, String> {
    let mut statement = transaction
        .prepare(
            r#"
            SELECT session_id, source_path, owner_type, owner_id, model, updated_at_seconds,
                   input_tokens, cached_input_tokens, output_tokens, reasoning_output_tokens,
                   total_tokens
            FROM session_usage
            "#,
        )
        .map_err(|err| db_error("读取旧 session token 统计失败", err))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                LegacySession {
                    session_id: row.get(0)?,
                    owner: OwnerAttribution {
                        owner_type: row.get(2)?,
                        owner_id: row.get(3)?,
                    },
                    model: row.get(4)?,
                    updated_at_seconds: row.get(5)?,
                    usage: TokenUsage {
                        input_tokens: row.get::<_, i64>(6).map(sql_i64_to_u64)?,
                        cached_input_tokens: row.get::<_, i64>(7).map(sql_i64_to_u64)?,
                        cache_write_input_tokens: 0,
                        output_tokens: row.get::<_, i64>(8).map(sql_i64_to_u64)?,
                        reasoning_output_tokens: row.get::<_, i64>(9).map(sql_i64_to_u64)?,
                        total_tokens: row.get::<_, i64>(10).map(sql_i64_to_u64)?,
                    },
                },
            ))
        })
        .map_err(|err| db_error("读取旧 session token 统计失败", err))?;
    rows.collect::<Result<HashMap<_, _>, _>>()
        .map_err(|err| db_error("读取旧 session token 统计失败", err))
}

pub(super) fn record_attribution_at(
    db_path: &Path,
    owner_type: &str,
    owner_id: &str,
    provider: &str,
    started_at: &str,
) -> Result<(), String> {
    let owner_type = owner_type.trim();
    let owner_id = owner_id.trim();
    let provider = provider.trim();
    if !matches!(owner_type, OWNER_TYPE_SUBSCRIPTION | OWNER_TYPE_API_PROFILE) {
        return Err("token 统计 owner_type 无效".to_string());
    }
    if owner_id.is_empty() {
        return Err("token 统计 owner_id 不能为空".to_string());
    }
    if !matches!(provider, PROVIDER_SUBSCRIPTION | PROVIDER_API) {
        return Err("token 统计 provider 无效".to_string());
    }
    let started_at_seconds =
        parse_rfc3339_seconds(started_at).ok_or_else(|| "token 统计归属时间无效".to_string())?;
    let connection = open_usage_connection(db_path, started_at)?;
    connection
        .execute(
            r#"
            INSERT INTO attribution(owner_type, owner_id, provider, started_at, started_at_seconds)
            VALUES(?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                owner_type,
                owner_id,
                provider,
                started_at,
                started_at_seconds
            ],
        )
        .map_err(|err| db_error("写入 token 统计归属失败", err))?;
    Ok(())
}

/// The owner Codex Switch had selected for `provider` at `timestamp_seconds`.
pub(super) fn find_owner_attribution(
    connection: &Connection,
    provider: &str,
    timestamp_seconds: i64,
) -> Result<Option<OwnerAttribution>, String> {
    if provider.trim().is_empty() {
        return Ok(None);
    }
    connection
        .prepare_cached(
            r#"
            SELECT owner_type, owner_id
            FROM attribution
            WHERE provider = ?1 AND started_at_seconds <= ?2
            ORDER BY started_at_seconds DESC, id DESC
            LIMIT 1
            "#,
        )
        .and_then(|mut statement| {
            statement
                .query_row(params![provider, timestamp_seconds], |row| {
                    Ok(OwnerAttribution {
                        owner_type: row.get(0)?,
                        owner_id: row.get(1)?,
                    })
                })
                .optional()
        })
        .map_err(|err| db_error("查询 token 统计归属失败", err))
}

/// Stores one response. Returns `false` when its `response_id` is already stored (the same
/// response copied into another rollout, or a file read again after a rewrite), in which case
/// nothing is added. `count_in_totals` is `false` only for migrated window events, whose
/// all-time share is already in their session's total.
pub(super) fn insert_record(
    transaction: &Transaction<'_>,
    record: &UsageRecord,
    count_in_totals: bool,
) -> Result<bool, String> {
    let inserted = transaction
        .prepare_cached(
            r#"
            INSERT OR IGNORE INTO usage_records(
                response_id, thread_id, owner_type, owner_id, model, timestamp_seconds,
                input_tokens, cached_input_tokens, cache_write_input_tokens, output_tokens,
                reasoning_output_tokens, total_tokens, price_label, estimated_cost_usd,
                input_price, cached_input_price, cache_write_price, output_price, duration_ms,
                first_token_ms
            )
            VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                   ?18, ?19, ?20)
            "#,
        )
        .and_then(|mut statement| {
            statement.execute(params![
                record.response_id,
                record.thread_id,
                record.owner.owner_type,
                record.owner.owner_id,
                record.model,
                record.timestamp_seconds,
                u64_to_sql(record.usage.input_tokens),
                u64_to_sql(record.usage.cached_input_tokens),
                u64_to_sql(record.usage.cache_write_input_tokens),
                u64_to_sql(record.usage.output_tokens),
                u64_to_sql(record.usage.reasoning_output_tokens),
                u64_to_sql(record.usage.total_tokens),
                record.cost.price_label,
                record.cost.cost_usd,
                record.cost.prices.map(|prices| prices.input),
                record.cost.prices.and_then(|prices| prices.cached_input),
                record.cost.prices.and_then(|prices| prices.cache_write),
                record.cost.prices.map(|prices| prices.output),
                record.timing.duration_ms,
                record.timing.first_token_ms
            ])
        })
        .map_err(|err| db_error("写入 token 用量记录失败", err))?;
    if inserted == 0 {
        return Ok(false);
    }
    add_to_hourly(transaction, record)?;
    if count_in_totals {
        add_to_totals(transaction, record)?;
    }
    Ok(true)
}

macro_rules! rollup_upsert_sql {
    ($table:literal, $hour_column:literal, $hour_value:literal) => {
        concat!(
            "INSERT INTO ", $table, "(", $hour_column,
            "owner_type, owner_id, model, thread_id, price_label, input_tokens,              cached_input_tokens, cache_write_input_tokens, output_tokens,              reasoning_output_tokens, total_tokens, estimated_cost_usd, record_count,              last_used_seconds)              VALUES(", $hour_value, "?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?)              ON CONFLICT(", $hour_column, "owner_type, owner_id, model, thread_id, price_label)              DO UPDATE SET              input_tokens = input_tokens + excluded.input_tokens,              cached_input_tokens = cached_input_tokens + excluded.cached_input_tokens,              cache_write_input_tokens = cache_write_input_tokens + excluded.cache_write_input_tokens,              output_tokens = output_tokens + excluded.output_tokens,              reasoning_output_tokens = reasoning_output_tokens + excluded.reasoning_output_tokens,              total_tokens = total_tokens + excluded.total_tokens,              estimated_cost_usd = estimated_cost_usd + excluded.estimated_cost_usd,              record_count = record_count + 1,              last_used_seconds = MAX(last_used_seconds, excluded.last_used_seconds)"
        )
    };
}

const HOURLY_UPSERT_SQL: &str = rollup_upsert_sql!("usage_hourly", "hour_start, ", "?, ");

const TOTALS_UPSERT_SQL: &str = rollup_upsert_sql!("usage_totals", "", "");

fn add_to_hourly(transaction: &Transaction<'_>, record: &UsageRecord) -> Result<(), String> {
    add_to_rollup(
        transaction,
        HOURLY_UPSERT_SQL,
        Some(hour_start(record.timestamp_seconds)),
        record,
    )
}

fn add_to_totals(transaction: &Transaction<'_>, record: &UsageRecord) -> Result<(), String> {
    add_to_rollup(transaction, TOTALS_UPSERT_SQL, None, record)
}

fn add_to_rollup(
    transaction: &Transaction<'_>,
    sql: &str,
    hour: Option<i64>,
    record: &UsageRecord,
) -> Result<(), String> {
    let usage = &record.usage;
    let tokens = [
        u64_to_sql(usage.input_tokens),
        u64_to_sql(usage.cached_input_tokens),
        u64_to_sql(usage.cache_write_input_tokens),
        u64_to_sql(usage.output_tokens),
        u64_to_sql(usage.reasoning_output_tokens),
        u64_to_sql(usage.total_tokens),
    ];
    let cost = record.cost.cost_usd.unwrap_or(0.0);
    let mut values: Vec<&dyn ToSql> = Vec::with_capacity(14);
    if let Some(hour) = hour.as_ref() {
        values.push(hour);
    }
    values.extend([
        &record.owner.owner_type as &dyn ToSql,
        &record.owner.owner_id,
        &record.model,
        &record.thread_id,
        &record.cost.price_label,
    ]);
    values.extend(tokens.iter().map(|value| value as &dyn ToSql));
    values.push(&cost);
    values.push(&record.timestamp_seconds);
    transaction
        .prepare_cached(sql)
        .and_then(|mut statement| statement.execute(values.as_slice()))
        .map(|_| ())
        .map_err(|err| db_error("写入 token 统计汇总失败", err))
}

const STORED_RECORD_COLUMNS: &str = "response_id, owner_type, owner_id, model, thread_id, \
     timestamp_seconds, input_tokens, cached_input_tokens, cache_write_input_tokens, \
     output_tokens, reasoning_output_tokens, total_tokens, price_label, estimated_cost_usd";

/// A stored record (`STORED_RECORD_COLUMNS`) with its price label and cost. The service tier is
/// not stored; the caller supplies the one it knows.
fn stored_record(
    row: &rusqlite::Row<'_>,
    service_tier: ServiceTier,
) -> rusqlite::Result<(UsageRecord, String, Option<f64>)> {
    Ok((
        UsageRecord {
            response_id: row.get(0)?,
            owner: OwnerAttribution {
                owner_type: row.get(1)?,
                owner_id: row.get(2)?,
            },
            model: row.get(3)?,
            thread_id: row.get(4)?,
            timestamp_seconds: row.get(5)?,
            usage: TokenUsage {
                input_tokens: row.get::<_, i64>(6).map(sql_i64_to_u64)?,
                cached_input_tokens: row.get::<_, i64>(7).map(sql_i64_to_u64)?,
                cache_write_input_tokens: row.get::<_, i64>(8).map(sql_i64_to_u64)?,
                output_tokens: row.get::<_, i64>(9).map(sql_i64_to_u64)?,
                reasoning_output_tokens: row.get::<_, i64>(10).map(sql_i64_to_u64)?,
                total_tokens: row.get::<_, i64>(11).map(sql_i64_to_u64)?,
            },
            service_tier,
            cost: EstimatedCost {
                cost_usd: None,
                price_label: "",
                prices: None,
            },
            timing: RequestTiming::default(),
        },
        row.get(12)?,
        row.get(13)?,
    ))
}

/// A response read again whose stored record has no price is priced now with the tier it ran
/// on, from the stored model and tokens, when the price table covers it; otherwise it gets the
/// reason the table gives now. Records migrated from the per-session statistics are left alone.
/// Returns whether the record was priced.
pub(super) fn reprice_stored_record(
    transaction: &Transaction<'_>,
    record: &UsageRecord,
) -> Result<bool, String> {
    if is_legacy_response_id(&record.response_id) {
        return Ok(false);
    }
    let stored = transaction
        .prepare_cached(&format!(
            "SELECT {STORED_RECORD_COLUMNS} FROM usage_records WHERE response_id = ?1"
        ))
        .and_then(|mut statement| {
            statement
                .query_row([&record.response_id], |row| {
                    stored_record(row, record.service_tier)
                })
                .optional()
        })
        .map_err(|err| db_error("读取需要改价的 token 记录失败", err))?;
    let Some((mut stored, stored_label, stored_cost)) = stored else {
        return Err(format!(
            "token 记录 {} 写入时被跳过却不存在",
            record.response_id
        ));
    };
    if stored_cost.is_some() {
        return Ok(false);
    }
    let cost = estimate_request_cost(
        &stored.model,
        &stored.usage,
        record.service_tier,
        &stored.owner.owner_type,
    );
    if cost.price_label == stored_label {
        return Ok(false);
    }
    stored.cost = cost;
    relabel_unpriced_record(transaction, &stored, &stored_label)?;
    Ok(stored.cost.cost_usd.is_some())
}

/// After the rollouts were read again for a new price table, the records still waiting for a
/// price the table now has were in none of them (their rollouts are gone, or could not be
/// read; the scan logged those). They are marked as having an unknown tier, so the reason shown
/// is true and a later table does not read everything again for them. Returns how many.
pub(super) fn mark_service_tier_unknown(transaction: &Transaction<'_>) -> Result<usize, String> {
    let candidates = {
        let mut statement = transaction
            .prepare(&format!(
                "SELECT {STORED_RECORD_COLUMNS} FROM usage_records \
                 WHERE estimated_cost_usd IS NULL AND price_label = ?1 \
                 AND response_id NOT LIKE 'legacy:%'"
            ))
            .map_err(|err| db_error("读取未定价 token 记录失败", err))?;
        let rows = statement
            .query_map([UNPRICED_MISSING_MODEL_PRICE], |row| {
                stored_record(row, ServiceTier::Other)
            })
            .map_err(|err| db_error("读取未定价 token 记录失败", err))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| db_error("读取未定价 token 记录失败", err))?;
        rows.into_iter()
            .filter(|(record, _, _)| has_model_price(&record.model))
            .collect::<Vec<_>>()
    };
    for (record, label, _) in &candidates {
        let record = UsageRecord {
            cost: EstimatedCost {
                cost_usd: None,
                price_label: UNPRICED_MISSING_SERVICE_TIER,
                prices: None,
            },
            ..record.clone()
        };
        relabel_unpriced_record(transaction, &record, label)?;
    }
    Ok(candidates.len())
}

/// Moves an unpriced record to the price label and cost it has now; its share of the hourly and
/// all-time rollups moves along.
fn relabel_unpriced_record(
    transaction: &Transaction<'_>,
    record: &UsageRecord,
    stored_label: &str,
) -> Result<(), String> {
    let hour = hour_start(record.timestamp_seconds);
    remove_from_rollup(
        transaction,
        "usage_hourly",
        Some(hour),
        record,
        stored_label,
    )?;
    remove_from_rollup(transaction, "usage_totals", None, record, stored_label)?;
    transaction
        .prepare_cached(
            "UPDATE usage_records SET price_label = ?2, estimated_cost_usd = ?3 WHERE response_id = ?1",
        )
        .and_then(|mut statement| {
            statement.execute(params![
                record.response_id,
                record.cost.price_label,
                record.cost.cost_usd
            ])
        })
        .map_err(|err| db_error("写入 token 记录改价失败", err))?;
    set_record_prices(
        transaction,
        &record.response_id,
        record.cost.prices.as_ref(),
    )?;
    add_to_hourly(transaction, record)?;
    add_to_totals(transaction, record)
}

/// Takes one unpriced record out of a rollup row (its cost there is 0) and drops the row once
/// it holds no record.
fn remove_from_rollup(
    transaction: &Transaction<'_>,
    table: &str,
    hour: Option<i64>,
    record: &UsageRecord,
    price_label: &str,
) -> Result<(), String> {
    let hour_key = if hour.is_some() {
        "hour_start = ? AND "
    } else {
        ""
    };
    let key = format!(
        "{hour_key}owner_type = ? AND owner_id = ? AND model = ? AND thread_id = ? AND price_label = ?"
    );
    let mut key_values: Vec<&dyn ToSql> = Vec::with_capacity(6);
    if let Some(hour) = hour.as_ref() {
        key_values.push(hour);
    }
    key_values.extend([
        &record.owner.owner_type as &dyn ToSql,
        &record.owner.owner_id,
        &record.model,
        &record.thread_id,
        &price_label,
    ]);
    let usage = &record.usage;
    let tokens = [
        u64_to_sql(usage.input_tokens),
        u64_to_sql(usage.cached_input_tokens),
        u64_to_sql(usage.cache_write_input_tokens),
        u64_to_sql(usage.output_tokens),
        u64_to_sql(usage.reasoning_output_tokens),
        u64_to_sql(usage.total_tokens),
    ];
    let mut values: Vec<&dyn ToSql> = tokens.iter().map(|value| value as &dyn ToSql).collect();
    values.extend(key_values.iter().copied());
    let changed = transaction
        .prepare_cached(&format!(
            "UPDATE {table} SET input_tokens = input_tokens - ?, \
             cached_input_tokens = cached_input_tokens - ?, \
             cache_write_input_tokens = cache_write_input_tokens - ?, \
             output_tokens = output_tokens - ?, \
             reasoning_output_tokens = reasoning_output_tokens - ?, \
             total_tokens = total_tokens - ?, record_count = record_count - 1 WHERE {key}"
        ))
        .and_then(|mut statement| statement.execute(values.as_slice()))
        .map_err(|err| db_error("移出 token 统计汇总失败", err))?;
    if changed != 1 {
        return Err(format!(
            "token 统计汇总 {table} 里没有记录 {} 所在的行（{price_label}）",
            record.response_id
        ));
    }
    transaction
        .prepare_cached(&format!(
            "DELETE FROM {table} WHERE {key} AND record_count <= 0"
        ))
        .and_then(|mut statement| statement.execute(key_values.as_slice()))
        .map(|_| ())
        .map_err(|err| db_error("清理 token 统计汇总失败", err))
}

/// The file stamp a cursor was stored with, to skip unchanged files without opening them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FileStamp {
    pub(super) modified_nanos: i64,
    pub(super) size: u64,
}

pub(super) fn load_cursors(
    connection: &Connection,
) -> Result<HashMap<String, (FileStamp, RolloutCursor)>, String> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT source_path, modified_nanos, file_size, byte_offset, resume_check,
                   provider, model, service_tier, request_started_ms, output_started
            FROM rollout_cursors
            "#,
        )
        .map_err(|err| db_error("读取 token 统计读取位置失败", err))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (
                    FileStamp {
                        modified_nanos: row.get(1)?,
                        size: row.get::<_, i64>(2).map(sql_i64_to_u64)?,
                    },
                    RolloutCursor {
                        offset: row.get::<_, i64>(3).map(sql_i64_to_u64)?,
                        resume_check: row.get(4)?,
                        provider: row.get(5)?,
                        model: row.get(6)?,
                        service_tier: row.get(7)?,
                        request_started_ms: row.get(8)?,
                        output_started: row.get::<_, i64>(9)? != 0,
                    },
                ),
            ))
        })
        .map_err(|err| db_error("读取 token 统计读取位置失败", err))?;
    rows.collect::<Result<HashMap<_, _>, _>>()
        .map_err(|err| db_error("读取 token 统计读取位置失败", err))
}

pub(super) fn save_cursor(
    transaction: &Transaction<'_>,
    source_path: &str,
    stamp: FileStamp,
    cursor: &RolloutCursor,
) -> Result<(), String> {
    transaction
        .prepare_cached(
            r#"
            INSERT INTO rollout_cursors(
                source_path, modified_nanos, file_size, byte_offset, resume_check,
                provider, model, service_tier, request_started_ms, output_started
            )
            VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(source_path) DO UPDATE SET
                modified_nanos = excluded.modified_nanos,
                file_size = excluded.file_size,
                byte_offset = excluded.byte_offset,
                resume_check = excluded.resume_check,
                provider = excluded.provider,
                model = excluded.model,
                service_tier = excluded.service_tier,
                request_started_ms = excluded.request_started_ms,
                output_started = excluded.output_started
            "#,
        )
        .and_then(|mut statement| {
            statement.execute(params![
                source_path,
                stamp.modified_nanos,
                u64_to_sql(stamp.size),
                u64_to_sql(cursor.offset),
                cursor.resume_check,
                cursor.provider,
                cursor.model,
                cursor.service_tier,
                cursor.request_started_ms,
                i64::from(cursor.output_started)
            ])
        })
        .map(|_| ())
        .map_err(|err| db_error("写入 token 统计读取位置失败", err))
}

pub(super) fn delete_cursor(
    transaction: &Transaction<'_>,
    source_path: &str,
) -> Result<(), String> {
    transaction
        .prepare_cached("DELETE FROM rollout_cursors WHERE source_path = ?1")
        .and_then(|mut statement| statement.execute([source_path]))
        .map(|_| ())
        .map_err(|err| db_error("清理 token 统计读取位置失败", err))
}
