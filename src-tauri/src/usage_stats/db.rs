use super::model::{
    EstimatedCost, OwnerAttribution, ParsedSession, TokenUsage, TokenUsageEvent,
    OWNER_TYPE_API_PROFILE, OWNER_TYPE_SUBSCRIPTION, PROVIDER_API, PROVIDER_SUBSCRIPTION,
};
use crate::{
    paths::{app_data_dir, ensure_parent_dir},
    time_util::parse_rfc3339_seconds,
};
use rusqlite::OptionalExtension;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

pub(super) const META_STATS_STARTED_AT: &str = "stats_started_at";

pub(super) const SCAN_OUTCOME_INDEXED: &str = "indexed";

pub(super) const SCAN_OUTCOME_IGNORED: &str = "ignored";

pub(super) const SCAN_OUTCOME_DUPLICATE: &str = "duplicate";

pub(super) const SCAN_OUTCOME_MISSING_ATTRIBUTION: &str = "missing_attribution";

pub(super) const SCAN_OUTCOME_BEFORE_START: &str = "before_start";

pub(super) fn sql_i64_to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

pub(super) fn usage_db_path() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join("usage-stats.sqlite"))
}

pub(super) fn db_error(context: &str, err: rusqlite::Error) -> String {
    format!("{context}: {err}")
}

pub(super) fn open_usage_connection(path: &Path, now: &str) -> Result<Connection, String> {
    ensure_parent_dir(path)?;
    let connection =
        Connection::open(path).map_err(|err| db_error("打开 token 统计库失败", err))?;
    ensure_database(&connection, now)?;
    Ok(connection)
}

fn ensure_database(connection: &Connection, now: &str) -> Result<(), String> {
    connection
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
            CREATE TABLE IF NOT EXISTS session_usage (
                session_id TEXT PRIMARY KEY,
                source_path TEXT NOT NULL,
                owner_type TEXT NOT NULL,
                owner_id TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                started_at TEXT NOT NULL,
                started_at_seconds INTEGER NOT NULL,
                updated_at TEXT NOT NULL,
                updated_at_seconds INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                reasoning_output_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL,
                model_context_window INTEGER,
                estimated_cost_usd REAL,
                priced INTEGER NOT NULL,
                pricing_context TEXT,
                unpriced_reason TEXT,
                last_scanned_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS session_usage_owner_idx
                ON session_usage(owner_type, owner_id, started_at_seconds);
            CREATE TABLE IF NOT EXISTS session_token_events (
                source_path TEXT NOT NULL,
                event_index INTEGER NOT NULL,
                timestamp_seconds INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                reasoning_output_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL,
                PRIMARY KEY(source_path, event_index)
            ) WITHOUT ROWID;
            CREATE INDEX IF NOT EXISTS session_token_events_time_idx
                ON session_token_events(timestamp_seconds);
            CREATE TABLE IF NOT EXISTS session_scan_state (
                source_path TEXT PRIMARY KEY,
                modified_nanos INTEGER NOT NULL,
                file_size INTEGER NOT NULL,
                scan_scope TEXT NOT NULL,
                session_id TEXT NOT NULL,
                outcome TEXT NOT NULL,
                last_scanned_at TEXT NOT NULL
            );
            "#,
        )
        .map_err(|err| db_error("初始化 token 统计库失败", err))?;
    ensure_session_usage_columns(connection)?;

    let existing: Option<String> = connection
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [META_STATS_STARTED_AT],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| db_error("读取 token 统计起始时间失败", err))?;
    if existing.is_none() {
        connection
            .execute(
                "INSERT INTO meta(key, value) VALUES(?1, ?2)",
                params![META_STATS_STARTED_AT, now],
            )
            .map_err(|err| db_error("写入 token 统计起始时间失败", err))?;
    }

    Ok(())
}

fn ensure_session_usage_columns(connection: &Connection) -> Result<(), String> {
    ensure_table_column(
        connection,
        "session_usage",
        "pricing_context",
        "pricing_context TEXT",
    )?;
    ensure_table_column(
        connection,
        "session_usage",
        "unpriced_reason",
        "unpriced_reason TEXT",
    )?;
    ensure_window_usage_columns(connection, "today")?;
    ensure_window_usage_columns(connection, "days_7")?;
    ensure_window_usage_columns(connection, "days_30")?;
    Ok(())
}

fn ensure_window_usage_columns(connection: &Connection, prefix: &str) -> Result<(), String> {
    for (name, definition) in [
        ("input_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("cached_input_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("output_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("reasoning_output_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("total_tokens", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        let column = format!("{prefix}_{name}");
        ensure_table_column(
            connection,
            "session_usage",
            &column,
            &format!("{column} {definition}"),
        )?;
    }
    Ok(())
}

fn ensure_table_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let pragma_sql = format!("PRAGMA table_info({table})");
    let mut statement = connection
        .prepare(&pragma_sql)
        .map_err(|err| db_error("读取 token 统计库结构失败", err))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| db_error("读取 token 统计库结构失败", err))?;
    for existing_column in columns {
        let existing_column =
            existing_column.map_err(|err| db_error("读取 token 统计库结构失败", err))?;
        if existing_column == column {
            return Ok(());
        }
    }
    let alter_sql = format!("ALTER TABLE {table} ADD COLUMN {definition}");
    connection
        .execute(&alter_sql, [])
        .map_err(|err| db_error("升级 token 统计库结构失败", err))?;
    Ok(())
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
    connection
        .execute(
            "DELETE FROM session_scan_state WHERE outcome = ?1",
            [SCAN_OUTCOME_MISSING_ATTRIBUTION],
        )
        .map_err(|err| db_error("刷新 token 统计扫描缓存失败", err))?;
    Ok(())
}

pub(super) fn meta_value(connection: &Connection, key: &str) -> Result<String, String> {
    connection
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .map_err(|err| db_error("读取 token 统计元数据失败", err))
}

pub(super) fn find_owner_attribution(
    connection: &Connection,
    provider: &str,
    session_started_at_seconds: i64,
) -> Result<Option<OwnerAttribution>, String> {
    if provider.trim().is_empty() {
        return Ok(None);
    }
    connection
        .query_row(
            r#"
            SELECT owner_type, owner_id
            FROM attribution
            WHERE provider = ?1 AND started_at_seconds <= ?2
            ORDER BY started_at_seconds DESC, id DESC
            LIMIT 1
            "#,
            params![provider, session_started_at_seconds],
            |row| {
                Ok(OwnerAttribution {
                    owner_type: row.get(0)?,
                    owner_id: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|err| db_error("查询 token 统计归属失败", err))
}

pub(super) fn upsert_session_usage(
    connection: &Connection,
    path: &Path,
    parsed: &ParsedSession,
    usage: &TokenUsage,
    attribution: &OwnerAttribution,
    estimated: &EstimatedCost,
    now: &str,
) -> Result<(), String> {
    let started_at = parsed.started_at.as_ref().expect("started_at checked");
    let updated_at = parsed.updated_at.as_ref().unwrap_or(started_at);
    let model_context_window = parsed
        .model_context_window
        .and_then(|value| i64::try_from(value).ok());
    let estimated_cost_usd = estimated.cost_usd;
    connection
        .execute(
            r#"
            INSERT INTO session_usage(
                session_id,
                source_path,
                owner_type,
                owner_id,
                provider,
                model,
                started_at,
                started_at_seconds,
                updated_at,
                updated_at_seconds,
                input_tokens,
                cached_input_tokens,
                output_tokens,
                reasoning_output_tokens,
                total_tokens,
                today_input_tokens,
                today_cached_input_tokens,
                today_output_tokens,
                today_reasoning_output_tokens,
                today_total_tokens,
                days_7_input_tokens,
                days_7_cached_input_tokens,
                days_7_output_tokens,
                days_7_reasoning_output_tokens,
                days_7_total_tokens,
                days_30_input_tokens,
                days_30_cached_input_tokens,
                days_30_output_tokens,
                days_30_reasoning_output_tokens,
                days_30_total_tokens,
                model_context_window,
                estimated_cost_usd,
                priced,
                pricing_context,
                unpriced_reason,
                last_scanned_at
            )
            VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36)
            ON CONFLICT(session_id) DO UPDATE SET
                source_path = excluded.source_path,
                owner_type = excluded.owner_type,
                owner_id = excluded.owner_id,
                provider = excluded.provider,
                model = excluded.model,
                started_at = excluded.started_at,
                started_at_seconds = excluded.started_at_seconds,
                updated_at = excluded.updated_at,
                updated_at_seconds = excluded.updated_at_seconds,
                input_tokens = excluded.input_tokens,
                cached_input_tokens = excluded.cached_input_tokens,
                output_tokens = excluded.output_tokens,
                reasoning_output_tokens = excluded.reasoning_output_tokens,
                total_tokens = excluded.total_tokens,
                today_input_tokens = excluded.today_input_tokens,
                today_cached_input_tokens = excluded.today_cached_input_tokens,
                today_output_tokens = excluded.today_output_tokens,
                today_reasoning_output_tokens = excluded.today_reasoning_output_tokens,
                today_total_tokens = excluded.today_total_tokens,
                days_7_input_tokens = excluded.days_7_input_tokens,
                days_7_cached_input_tokens = excluded.days_7_cached_input_tokens,
                days_7_output_tokens = excluded.days_7_output_tokens,
                days_7_reasoning_output_tokens = excluded.days_7_reasoning_output_tokens,
                days_7_total_tokens = excluded.days_7_total_tokens,
                days_30_input_tokens = excluded.days_30_input_tokens,
                days_30_cached_input_tokens = excluded.days_30_cached_input_tokens,
                days_30_output_tokens = excluded.days_30_output_tokens,
                days_30_reasoning_output_tokens = excluded.days_30_reasoning_output_tokens,
                days_30_total_tokens = excluded.days_30_total_tokens,
                model_context_window = excluded.model_context_window,
                estimated_cost_usd = excluded.estimated_cost_usd,
                priced = excluded.priced,
                pricing_context = excluded.pricing_context,
                unpriced_reason = excluded.unpriced_reason,
                last_scanned_at = excluded.last_scanned_at
            "#,
            params![
                parsed.session_id,
                path.to_string_lossy().to_string(),
                attribution.owner_type,
                attribution.owner_id,
                parsed.provider,
                parsed.model,
                started_at.raw,
                started_at.seconds,
                updated_at.raw,
                updated_at.seconds,
                i64::try_from(usage.input_tokens).unwrap_or(i64::MAX),
                i64::try_from(usage.cached_input_tokens).unwrap_or(i64::MAX),
                i64::try_from(usage.output_tokens).unwrap_or(i64::MAX),
                i64::try_from(usage.reasoning_output_tokens).unwrap_or(i64::MAX),
                i64::try_from(usage.total_tokens).unwrap_or(i64::MAX),
                i64::try_from(parsed.window_usage.today.input_tokens).unwrap_or(i64::MAX),
                i64::try_from(parsed.window_usage.today.cached_input_tokens).unwrap_or(i64::MAX),
                i64::try_from(parsed.window_usage.today.output_tokens).unwrap_or(i64::MAX),
                i64::try_from(parsed.window_usage.today.reasoning_output_tokens)
                    .unwrap_or(i64::MAX),
                i64::try_from(parsed.window_usage.today.total_tokens).unwrap_or(i64::MAX),
                i64::try_from(parsed.window_usage.days_7.input_tokens).unwrap_or(i64::MAX),
                i64::try_from(parsed.window_usage.days_7.cached_input_tokens).unwrap_or(i64::MAX),
                i64::try_from(parsed.window_usage.days_7.output_tokens).unwrap_or(i64::MAX),
                i64::try_from(parsed.window_usage.days_7.reasoning_output_tokens)
                    .unwrap_or(i64::MAX),
                i64::try_from(parsed.window_usage.days_7.total_tokens).unwrap_or(i64::MAX),
                i64::try_from(parsed.window_usage.days_30.input_tokens).unwrap_or(i64::MAX),
                i64::try_from(parsed.window_usage.days_30.cached_input_tokens).unwrap_or(i64::MAX),
                i64::try_from(parsed.window_usage.days_30.output_tokens).unwrap_or(i64::MAX),
                i64::try_from(parsed.window_usage.days_30.reasoning_output_tokens)
                    .unwrap_or(i64::MAX),
                i64::try_from(parsed.window_usage.days_30.total_tokens).unwrap_or(i64::MAX),
                model_context_window,
                estimated_cost_usd,
                if estimated.priced { 1 } else { 0 },
                estimated.pricing_context,
                estimated.unpriced_reason,
                now
            ],
        )
        .map_err(|err| db_error("写入 session token 统计失败", err))?;
    replace_session_token_events(connection, path, &parsed.token_events)?;
    Ok(())
}

fn replace_session_token_events(
    connection: &Connection,
    path: &Path,
    events: &[TokenUsageEvent],
) -> Result<(), String> {
    // token delta 很小，持久化后滚动窗口可直接从 SQLite 计算；文件未变化时
    // 无需为了 today / 7d / 30d 的时间边界重新读取 JSONL。
    let source_path = path.to_string_lossy().into_owned();
    let transaction = connection
        .unchecked_transaction()
        .map_err(|err| db_error("开启 session token 事件事务失败", err))?;
    transaction
        .execute(
            "DELETE FROM session_token_events WHERE source_path = ?1",
            [&source_path],
        )
        .map_err(|err| db_error("清理 session token 事件失败", err))?;
    {
        let mut statement = transaction
            .prepare_cached(
                r#"
                INSERT INTO session_token_events(
                    source_path,
                    event_index,
                    timestamp_seconds,
                    input_tokens,
                    cached_input_tokens,
                    output_tokens,
                    reasoning_output_tokens,
                    total_tokens
                )
                VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
            )
            .map_err(|err| db_error("准备 session token 事件写入失败", err))?;
        for (event_index, event) in events.iter().enumerate() {
            statement
                .execute(params![
                    source_path,
                    i64::try_from(event_index).unwrap_or(i64::MAX),
                    event.timestamp_seconds,
                    i64::try_from(event.usage.input_tokens).unwrap_or(i64::MAX),
                    i64::try_from(event.usage.cached_input_tokens).unwrap_or(i64::MAX),
                    i64::try_from(event.usage.output_tokens).unwrap_or(i64::MAX),
                    i64::try_from(event.usage.reasoning_output_tokens).unwrap_or(i64::MAX),
                    i64::try_from(event.usage.total_tokens).unwrap_or(i64::MAX)
                ])
                .map_err(|err| db_error("写入 session token 事件失败", err))?;
        }
    }
    transaction
        .commit()
        .map_err(|err| db_error("提交 session token 事件失败", err))?;
    Ok(())
}
