use super::{
    model::{
        OwnerAttribution, TokenUsage, UsageRecord, OWNER_TYPE_API_PROFILE, OWNER_TYPE_SUBSCRIPTION,
        PROVIDER_API, PROVIDER_SUBSCRIPTION,
    },
    pricing::estimate_summed_cost,
    records::RolloutCursor,
};
use crate::{
    paths::{app_data_dir, ensure_parent_dir},
    time_util::parse_rfc3339_seconds,
};
use rusqlite::{params, Connection, OptionalExtension, ToSql, Transaction, TransactionBehavior};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// Records at or before this instant are never counted (RFC 3339). For a new database it is the
/// creation time; for one migrated from the per-session statistics it is the last moment those
/// statistics covered, so every response is counted by exactly one of the two.
pub(super) const META_RECORDS_COUNTED_AFTER: &str = "records_counted_after";

const HOUR_SECONDS: i64 = 60 * 60;

const WINDOW_30_DAYS_SECONDS: i64 = 30 * 24 * HOUR_SECONDS;

pub(super) fn sql_i64_to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn u64_to_sql(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
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
                estimated_cost_usd REAL
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
                service_tier TEXT NOT NULL
            );
            "#,
        )
        .map_err(|err| db_error("初始化 token 统计库失败", err))?;

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

pub(super) fn records_counted_after_seconds(connection: &Connection) -> Result<i64, String> {
    let value = optional_meta_value(connection, META_RECORDS_COUNTED_AFTER)?
        .ok_or_else(|| "token 统计缺少记录起点".to_string())?;
    parse_rfc3339_seconds(&value).ok_or_else(|| format!("token 统计记录起点无效: {value:?}"))
}

// TODO(verify): checked on a copy of the real database (docs/development/usage-stats.md, 验证记录
// 2026-09-23), not yet in an installed build. Trigger, checks, pass criteria and where to continue
// are in that document's TODO(verify).
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
                cost,
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
                    response_id: format!("legacy:{source_path}:{event_index}"),
                    thread_id: session.session_id.clone(),
                    timestamp_seconds,
                    owner: session.owner.clone(),
                    model: session.model.clone(),
                    usage,
                    cost,
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
                reasoning_output_tokens, total_tokens, price_label, estimated_cost_usd
            )
            VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
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
                record.cost.cost_usd
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
                   provider, model, service_tier
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
                provider, model, service_tier
            )
            VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(source_path) DO UPDATE SET
                modified_nanos = excluded.modified_nanos,
                file_size = excluded.file_size,
                byte_offset = excluded.byte_offset,
                resume_check = excluded.resume_check,
                provider = excluded.provider,
                model = excluded.model,
                service_tier = excluded.service_tier
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
                cursor.service_tier
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
