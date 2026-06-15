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
    collections::{BTreeMap, HashSet},
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};
use time::{OffsetDateTime, UtcOffset};

const OWNER_TYPE_SUBSCRIPTION: &str = "subscription";
const OWNER_TYPE_API_PROFILE: &str = "api_profile";
const PROVIDER_SUBSCRIPTION: &str = "openai";
const PROVIDER_API: &str = "api";
const META_STATS_STARTED_AT: &str = "stats_started_at";
const PRICING_SOURCE: &str = "https://developers.openai.com/api/docs/pricing";
const PRICING_UPDATED_AT: &str = "2026-06-16";
const LONG_CONTEXT_THRESHOLD_TOKENS: u64 = 270_000;
const PRICING_CONTEXT_STANDARD_SHORT: &str = "standard_short_context";
const PRICING_CONTEXT_STANDARD_LONG: &str = "standard_long_context";
const UNPRICED_REASON_MISSING_MODEL_PRICE: &str = "missing_model_price";
const UNPRICED_REASON_MISSING_CACHED_INPUT_PRICE: &str = "missing_cached_input_price";

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

const MODEL_PRICES: &[ModelPrice] = &[
    ModelPrice {
        model: "gpt-5.5",
        short_context: TokenPrices {
            input_per_million: 5.0,
            cached_input_per_million: Some(0.5),
            output_per_million: 30.0,
        },
        long_context: Some(TokenPrices {
            input_per_million: 10.0,
            cached_input_per_million: Some(1.0),
            output_per_million: 45.0,
        }),
        long_context_threshold: Some(LONG_CONTEXT_THRESHOLD_TOKENS),
    },
    ModelPrice {
        model: "gpt-5.4",
        short_context: TokenPrices {
            input_per_million: 2.5,
            cached_input_per_million: Some(0.25),
            output_per_million: 15.0,
        },
        long_context: Some(TokenPrices {
            input_per_million: 5.0,
            cached_input_per_million: Some(0.5),
            output_per_million: 22.5,
        }),
        long_context_threshold: Some(LONG_CONTEXT_THRESHOLD_TOKENS),
    },
    ModelPrice {
        model: "gpt-5.4-mini",
        short_context: TokenPrices {
            input_per_million: 0.75,
            cached_input_per_million: Some(0.075),
            output_per_million: 4.5,
        },
        long_context: None,
        long_context_threshold: None,
    },
];

#[derive(Default)]
struct ScanWarnings {
    missing_attribution: u64,
    missing_price: u64,
    skipped_before_start: u64,
}

#[derive(Clone, Default)]
struct TokenUsage {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
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
}

struct OwnerAttribution {
    owner_type: String,
    owner_id: String,
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
    started_at_seconds: i64,
    updated_at: String,
    updated_at_seconds: i64,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
    total_tokens: u64,
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
    let db_path = usage_db_path()?;
    let codex_home = codex_dir()?;
    usage_stats_get_for_paths(&db_path, &codex_home, &now_string())
}

fn usage_db_path() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join("usage-stats.sqlite"))
}

fn db_error(context: &str, err: rusqlite::Error) -> String {
    format!("{context}: {err}")
}

fn open_usage_connection(path: &Path, now: &str) -> Result<Connection, String> {
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

fn record_attribution_at(
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

fn usage_stats_get_for_paths(
    db_path: &Path,
    codex_home: &Path,
    now: &str,
) -> Result<Value, String> {
    let now_seconds =
        parse_rfc3339_seconds(now).ok_or_else(|| "token 统计当前时间无效".to_string())?;
    let connection = open_usage_connection(db_path, now)?;
    let stats_started_at = meta_value(&connection, META_STATS_STARTED_AT)?;
    let stats_started_at_seconds = parse_rfc3339_seconds(&stats_started_at)
        .ok_or_else(|| "token 统计起始时间无效".to_string())?;
    let mut warnings = ScanWarnings::default();
    scan_codex_sessions(
        &connection,
        codex_home,
        stats_started_at_seconds,
        now,
        &mut warnings,
    )?;
    recompute_existing_costs(&connection)?;
    let response = aggregate_usage(&connection, now_seconds, &warnings)?;
    Ok(response)
}

fn meta_value(connection: &Connection, key: &str) -> Result<String, String> {
    connection
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .map_err(|err| db_error("读取 token 统计元数据失败", err))
}

fn scan_codex_sessions(
    connection: &Connection,
    codex_home: &Path,
    stats_started_at_seconds: i64,
    now: &str,
    warnings: &mut ScanWarnings,
) -> Result<(), String> {
    let files = collect_session_files(codex_home)?;
    let mut seen_session_ids = HashSet::new();
    for path in files {
        let parsed = match parse_session_file(&path) {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("{err}");
                continue;
            }
        };
        if parsed.session_id.is_empty() || parsed.usage.is_none() || parsed.started_at.is_none() {
            continue;
        }
        if !seen_session_ids.insert(parsed.session_id.clone()) {
            continue;
        }
        let started_at = parsed.started_at.as_ref().expect("checked above");
        if started_at.seconds < stats_started_at_seconds {
            warnings.skipped_before_start += 1;
            continue;
        }
        let Some(attribution) =
            find_owner_attribution(connection, &parsed.provider, started_at.seconds)?
        else {
            warnings.missing_attribution += 1;
            continue;
        };
        let usage = parsed.usage.as_ref().expect("checked above");
        let estimated = estimate_cost(&parsed.model, usage, parsed.model_context_window);
        if !estimated.priced {
            warnings.missing_price += 1;
        }
        upsert_session_usage(
            connection,
            &path,
            &parsed,
            usage,
            &attribution,
            &estimated,
            now,
        )?;
    }
    Ok(())
}

fn collect_session_files(codex_home: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    collect_jsonl_files_recursive(&codex_home.join("sessions"), &mut files)?;
    collect_jsonl_files_recursive(&codex_home.join("archived_sessions"), &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_jsonl_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(dir)
        .map_err(|err| format!("读取 Codex session 目录失败 {}: {err}", dir.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|err| format!("读取 Codex session 目录失败 {}: {err}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files_recursive(&path, files)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
    Ok(())
}

fn parse_session_file(path: &Path) -> Result<ParsedSession, String> {
    let file = fs::File::open(path)
        .map_err(|err| format!("读取 Codex session 文件失败 {}: {err}", path.display()))?;
    let reader = BufReader::new(file);
    let mut parsed = ParsedSession::default();
    for line in reader.lines() {
        let line =
            line.map_err(|err| format!("读取 Codex session 文件失败 {}: {err}", path.display()))?;
        parse_session_line(&line, &mut parsed);
    }
    Ok(parsed)
}

fn parse_session_line(line: &str, parsed: &mut ParsedSession) {
    if !line.contains("\"session_meta\"")
        && !line.contains("\"turn_context\"")
        && !line.contains("\"token_count\"")
    {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    match raw_string_field(&value, "type").as_str() {
        "session_meta" => parse_session_meta_line(&value, parsed),
        "turn_context" => parse_turn_context_line(&value, parsed),
        "event_msg" => parse_event_msg_line(&value, parsed),
        _ => {}
    }
}

fn parse_session_meta_line(value: &Value, parsed: &mut ParsedSession) {
    let payload = value.get("payload").unwrap_or(&Value::Null);
    let session_id = string_field(payload, "id");
    if !session_id.is_empty() {
        parsed.session_id = session_id;
    }
    let provider = string_field(payload, "model_provider");
    if !provider.is_empty() {
        parsed.provider = provider;
    }
    update_model_from_payload(payload, parsed);
    if let Some(timestamp) = timestamp_from_payload(value, payload) {
        parsed.started_at = Some(timestamp);
    }
}

fn parse_turn_context_line(value: &Value, parsed: &mut ParsedSession) {
    let payload = value.get("payload").unwrap_or(&Value::Null);
    update_model_from_payload(payload, parsed);
}

fn parse_event_msg_line(value: &Value, parsed: &mut ParsedSession) {
    let payload = value.get("payload").unwrap_or(&Value::Null);
    if string_field(payload, "type") != "token_count" {
        return;
    }
    let Some(info) = payload.get("info") else {
        return;
    };
    let Some(total_usage) = info.get("total_token_usage") else {
        return;
    };
    let usage = TokenUsage {
        input_tokens: u64_field(total_usage, "input_tokens"),
        cached_input_tokens: u64_field(total_usage, "cached_input_tokens"),
        output_tokens: u64_field(total_usage, "output_tokens"),
        reasoning_output_tokens: u64_field(total_usage, "reasoning_output_tokens"),
        total_tokens: u64_field(total_usage, "total_tokens"),
    };
    if usage.total_tokens == 0 {
        return;
    }
    let Some(timestamp) = timestamp_from_payload(value, payload) else {
        return;
    };
    let should_update = parsed
        .usage
        .as_ref()
        .map(|current| usage.total_tokens >= current.total_tokens)
        .unwrap_or(true);
    if should_update {
        parsed.usage = Some(usage);
        parsed.updated_at = Some(timestamp);
        parsed.model_context_window = optional_u64_field(info, "model_context_window");
    }
}

fn timestamp_from_payload(root: &Value, payload: &Value) -> Option<TimestampValue> {
    let raw = string_field(payload, "timestamp");
    let raw = if raw.is_empty() {
        string_field(root, "timestamp")
    } else {
        raw
    };
    parse_rfc3339_seconds(&raw).map(|seconds| TimestampValue { raw, seconds })
}

fn update_model_from_payload(payload: &Value, parsed: &mut ParsedSession) {
    for key in ["model", "model_slug", "selected_model", "current_model"] {
        let model = string_field(payload, key);
        if !model.is_empty() {
            parsed.model = model;
            return;
        }
    }
}

fn u64_field(value: &Value, key: &str) -> u64 {
    optional_u64_field(value, key).unwrap_or(0)
}

fn optional_u64_field(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(|raw| {
        raw.as_u64()
            .or_else(|| raw.as_i64().and_then(|number| u64::try_from(number).ok()))
            .or_else(|| {
                raw.as_str()
                    .and_then(|text| text.trim().parse::<u64>().ok())
            })
    })
}

fn find_owner_attribution(
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

fn upsert_session_usage(
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
                model_context_window,
                estimated_cost_usd,
                priced,
                pricing_context,
                unpriced_reason,
                last_scanned_at
            )
            VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
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
                model_context_window,
                estimated_cost_usd,
                if estimated.priced { 1 } else { 0 },
                estimated.pricing_context,
                estimated.unpriced_reason,
                now
            ],
        )
        .map_err(|err| db_error("写入 session token 统计失败", err))?;
    Ok(())
}

fn recompute_existing_costs(connection: &Connection) -> Result<(), String> {
    struct ExistingUsageRow {
        session_id: String,
        model: String,
        model_context_window: Option<u64>,
        usage: TokenUsage,
    }

    let mut statement = connection
        .prepare(
            r#"
            SELECT session_id,
                   model,
                   model_context_window,
                   input_tokens,
                   cached_input_tokens,
                   output_tokens,
                   reasoning_output_tokens,
                   total_tokens
            FROM session_usage
            "#,
        )
        .map_err(|err| db_error("读取 session token 费用失败", err))?;
    let rows = statement
        .query_map([], |row| {
            let raw_context_window: Option<i64> = row.get(2)?;
            Ok(ExistingUsageRow {
                session_id: row.get(0)?,
                model: row.get(1)?,
                model_context_window: raw_context_window
                    .and_then(|value| u64::try_from(value).ok()),
                usage: TokenUsage {
                    input_tokens: row.get::<_, i64>(3).map(sql_i64_to_u64)?,
                    cached_input_tokens: row.get::<_, i64>(4).map(sql_i64_to_u64)?,
                    output_tokens: row.get::<_, i64>(5).map(sql_i64_to_u64)?,
                    reasoning_output_tokens: row.get::<_, i64>(6).map(sql_i64_to_u64)?,
                    total_tokens: row.get::<_, i64>(7).map(sql_i64_to_u64)?,
                },
            })
        })
        .map_err(|err| db_error("读取 session token 费用失败", err))?;

    for row in rows {
        let row = row.map_err(|err| db_error("读取 session token 费用失败", err))?;
        let estimated = estimate_cost(&row.model, &row.usage, row.model_context_window);
        connection
            .execute(
                r#"
                UPDATE session_usage
                SET estimated_cost_usd = ?2,
                    priced = ?3,
                    pricing_context = ?4,
                    unpriced_reason = ?5
                WHERE session_id = ?1
                "#,
                params![
                    row.session_id,
                    estimated.cost_usd,
                    if estimated.priced { 1 } else { 0 },
                    estimated.pricing_context,
                    estimated.unpriced_reason
                ],
            )
            .map_err(|err| db_error("重新计算 session token 费用失败", err))?;
    }

    Ok(())
}

fn estimate_cost(
    model: &str,
    usage: &TokenUsage,
    model_context_window: Option<u64>,
) -> EstimatedCost {
    let normalized_model = normalize_model_id(model);
    let Some(price) = MODEL_PRICES
        .iter()
        .find(|price| price.model == normalized_model)
    else {
        return EstimatedCost {
            cost_usd: None,
            priced: false,
            pricing_context: None,
            unpriced_reason: Some(UNPRICED_REASON_MISSING_MODEL_PRICE),
        };
    };
    let (token_prices, pricing_context) = token_prices_for_context(price, model_context_window);

    let cached_input_tokens = usage.cached_input_tokens.min(usage.input_tokens);
    let non_cached_input_tokens = usage.input_tokens.saturating_sub(cached_input_tokens);
    let cached_cost = if cached_input_tokens == 0 {
        0.0
    } else if let Some(cached_input_per_million) = token_prices.cached_input_per_million {
        per_million_cost(cached_input_tokens, cached_input_per_million)
    } else {
        return EstimatedCost {
            cost_usd: None,
            priced: false,
            pricing_context: Some(pricing_context),
            unpriced_reason: Some(UNPRICED_REASON_MISSING_CACHED_INPUT_PRICE),
        };
    };
    let cost = per_million_cost(non_cached_input_tokens, token_prices.input_per_million)
        + cached_cost
        + per_million_cost(usage.output_tokens, token_prices.output_per_million);
    EstimatedCost {
        cost_usd: Some(cost),
        priced: true,
        pricing_context: Some(pricing_context),
        unpriced_reason: None,
    }
}

fn token_prices_for_context(
    price: &ModelPrice,
    model_context_window: Option<u64>,
) -> (&TokenPrices, &'static str) {
    if let (Some(threshold), Some(long_context)) =
        (price.long_context_threshold, price.long_context.as_ref())
    {
        if model_context_window.is_some_and(|window| window >= threshold) {
            return (long_context, PRICING_CONTEXT_STANDARD_LONG);
        }
    }
    (&price.short_context, PRICING_CONTEXT_STANDARD_SHORT)
}

fn normalize_model_id(model: &str) -> String {
    model
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

fn per_million_cost(tokens: u64, price_per_million: f64) -> f64 {
    (tokens as f64 / 1_000_000.0) * price_per_million
}

fn aggregate_usage(
    connection: &Connection,
    now_seconds: i64,
    warnings: &ScanWarnings,
) -> Result<Value, String> {
    let today_start = today_start_seconds(now_seconds);
    let days_7_start = now_seconds.saturating_sub(7 * 24 * 60 * 60);
    let days_30_start = now_seconds.saturating_sub(30 * 24 * 60 * 60);
    let mut subscriptions: BTreeMap<String, OwnerUsage> = BTreeMap::new();
    let mut api_profiles: BTreeMap<String, OwnerUsage> = BTreeMap::new();

    let mut statement = connection
        .prepare(
            r#"
            SELECT owner_type,
                   owner_id,
                   model,
                   started_at_seconds,
                   updated_at,
                   updated_at_seconds,
                   input_tokens,
                   cached_input_tokens,
                   output_tokens,
                   reasoning_output_tokens,
                   total_tokens,
                   estimated_cost_usd,
                   priced,
                   COALESCE(pricing_context, ''),
                   COALESCE(unpriced_reason, '')
            FROM session_usage
            "#,
        )
        .map_err(|err| db_error("读取 session token 统计失败", err))?;
    let rows = statement
        .query_map([], |row| {
            Ok(UsageRow {
                owner_type: row.get(0)?,
                owner_id: row.get(1)?,
                model: row.get(2)?,
                started_at_seconds: row.get(3)?,
                updated_at: row.get(4)?,
                updated_at_seconds: row.get(5)?,
                input_tokens: row.get::<_, i64>(6).map(sql_i64_to_u64)?,
                cached_input_tokens: row.get::<_, i64>(7).map(sql_i64_to_u64)?,
                output_tokens: row.get::<_, i64>(8).map(sql_i64_to_u64)?,
                reasoning_output_tokens: row.get::<_, i64>(9).map(sql_i64_to_u64)?,
                total_tokens: row.get::<_, i64>(10).map(sql_i64_to_u64)?,
                estimated_cost_usd: row.get(11)?,
                priced: row.get::<_, i64>(12)? == 1,
                pricing_context: row.get(13)?,
                unpriced_reason: row.get(14)?,
            })
        })
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
        if row.started_at_seconds >= today_start {
            apply_row_to_window(&mut target.today, &row);
            apply_row_to_model_window(&mut target.today_by_model, &row);
        }
        if row.started_at_seconds >= days_7_start {
            apply_row_to_window(&mut target.days_7, &row);
            apply_row_to_model_window(&mut target.days_7_by_model, &row);
        }
        if row.started_at_seconds >= days_30_start {
            apply_row_to_window(&mut target.days_30, &row);
            apply_row_to_model_window(&mut target.days_30_by_model, &row);
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

fn sql_i64_to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
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
    window.input_tokens = window.input_tokens.saturating_add(row.input_tokens);
    window.cached_input_tokens = window
        .cached_input_tokens
        .saturating_add(row.cached_input_tokens);
    window.output_tokens = window.output_tokens.saturating_add(row.output_tokens);
    window.reasoning_output_tokens = window
        .reasoning_output_tokens
        .saturating_add(row.reasoning_output_tokens);
    window.total_tokens = window.total_tokens.saturating_add(row.total_tokens);
    window.session_count = window.session_count.saturating_add(1);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_root(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("codex-switch-usage-stats-{name}-{stamp}"))
    }

    fn write_session(codex_home: &Path, day: &str, name: &str, lines: &[String]) -> PathBuf {
        let dir = codex_home
            .join("sessions")
            .join("2026")
            .join("06")
            .join(day);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.jsonl"));
        fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
        path
    }

    fn session_meta_line(
        session_id: &str,
        provider: &str,
        timestamp: &str,
        model: Option<&str>,
    ) -> String {
        let mut payload = json!({
            "id": session_id,
            "model_provider": provider,
            "timestamp": timestamp
        });
        if let Some(model) = model {
            payload["model"] = json!(model);
        }
        json!({
            "timestamp": timestamp,
            "type": "session_meta",
            "payload": payload
        })
        .to_string()
    }

    fn turn_context_line(timestamp: &str, model: &str) -> String {
        json!({
            "timestamp": timestamp,
            "type": "turn_context",
            "payload": {
                "model": model
            }
        })
        .to_string()
    }

    fn token_count_line(
        timestamp: &str,
        input: u64,
        cached: u64,
        output: u64,
        reasoning: u64,
        total: u64,
        context_window: u64,
    ) -> String {
        json!({
            "timestamp": timestamp,
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": input,
                        "cached_input_tokens": cached,
                        "output_tokens": output,
                        "reasoning_output_tokens": reasoning,
                        "total_tokens": total
                    },
                    "model_context_window": context_window
                }
            }
        })
        .to_string()
    }

    fn set_stats_started_at(db_path: &Path, value: &str) {
        let connection = open_usage_connection(db_path, value).unwrap();
        connection
            .execute(
                "UPDATE meta SET value = ?1 WHERE key = ?2",
                params![value, META_STATS_STARTED_AT],
            )
            .unwrap();
    }

    fn window_total(response: &Value, owner_map: &str, owner_id: &str, window: &str) -> u64 {
        response
            .get(owner_map)
            .and_then(|map| map.get(owner_id))
            .and_then(|owner| owner.get(window))
            .and_then(|window| window.get("total_tokens"))
            .and_then(Value::as_u64)
            .unwrap()
    }

    fn model_window<'a>(
        response: &'a Value,
        owner_map: &str,
        owner_id: &str,
        window: &str,
        model: &str,
    ) -> &'a Value {
        response
            .get(owner_map)
            .and_then(|map| map.get(owner_id))
            .and_then(|owner| owner.get(window))
            .and_then(|window| window.get("by_model"))
            .and_then(|by_model| by_model.get(model))
            .unwrap()
    }

    #[test]
    fn parses_total_token_usage_and_context_window() {
        let line = token_count_line("2026-06-15T01:00:00Z", 100, 25, 50, 20, 150, 258_400);
        let mut parsed = ParsedSession::default();

        parse_session_line(&line, &mut parsed);

        let usage = parsed.usage.unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.cached_input_tokens, 25);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.reasoning_output_tokens, 20);
        assert_eq!(usage.total_tokens, 150);
        assert_eq!(parsed.model_context_window, Some(258_400));
    }

    #[test]
    fn repeated_token_count_keeps_cumulative_max_once() {
        let root = temp_root("duplicate");
        let db_path = root.join("usage.sqlite");
        let codex_home = root.join("codex");
        record_attribution_at(
            &db_path,
            OWNER_TYPE_SUBSCRIPTION,
            "sub-a",
            PROVIDER_SUBSCRIPTION,
            "2026-06-15T00:00:00Z",
        )
        .unwrap();
        write_session(
            &codex_home,
            "15",
            "rollout-duplicate",
            &[
                session_meta_line(
                    "session-dup",
                    PROVIDER_SUBSCRIPTION,
                    "2026-06-15T01:00:00Z",
                    Some("gpt-5.5"),
                ),
                token_count_line("2026-06-15T01:01:00Z", 60, 10, 40, 5, 100, 258_400),
                token_count_line("2026-06-15T01:02:00Z", 90, 20, 60, 10, 150, 258_400),
            ],
        );

        let response =
            usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-15T03:00:00Z").unwrap();

        assert_eq!(
            window_total(&response, "subscriptions", "sub-a", "all"),
            150
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn skips_sessions_before_stats_started_at() {
        let root = temp_root("started-at");
        let db_path = root.join("usage.sqlite");
        let codex_home = root.join("codex");
        set_stats_started_at(&db_path, "2026-06-15T02:00:00Z");
        record_attribution_at(
            &db_path,
            OWNER_TYPE_SUBSCRIPTION,
            "sub-a",
            PROVIDER_SUBSCRIPTION,
            "2026-06-15T00:00:00Z",
        )
        .unwrap();
        write_session(
            &codex_home,
            "15",
            "rollout-old",
            &[
                session_meta_line(
                    "session-old",
                    PROVIDER_SUBSCRIPTION,
                    "2026-06-15T01:00:00Z",
                    Some("gpt-5.5"),
                ),
                token_count_line("2026-06-15T01:05:00Z", 100, 0, 20, 5, 120, 258_400),
            ],
        );

        let response =
            usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-15T03:00:00Z").unwrap();

        assert!(response
            .get("subscriptions")
            .and_then(Value::as_object)
            .unwrap()
            .is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_attribution_is_not_assigned_to_any_card() {
        let root = temp_root("missing-attribution");
        let db_path = root.join("usage.sqlite");
        let codex_home = root.join("codex");
        set_stats_started_at(&db_path, "2026-06-15T00:00:00Z");
        write_session(
            &codex_home,
            "15",
            "rollout-no-owner",
            &[
                session_meta_line(
                    "session-no-owner",
                    PROVIDER_SUBSCRIPTION,
                    "2026-06-15T01:00:00Z",
                    Some("gpt-5.5"),
                ),
                token_count_line("2026-06-15T01:05:00Z", 100, 0, 20, 5, 120, 258_400),
            ],
        );

        let response =
            usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-15T03:00:00Z").unwrap();

        assert!(response
            .get("subscriptions")
            .and_then(Value::as_object)
            .unwrap()
            .is_empty());
        assert!(!response
            .get("warnings")
            .and_then(Value::as_array)
            .unwrap()
            .is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn aggregates_subscription_and_api_profile_owners_separately() {
        let root = temp_root("owners");
        let db_path = root.join("usage.sqlite");
        let codex_home = root.join("codex");
        record_attribution_at(
            &db_path,
            OWNER_TYPE_SUBSCRIPTION,
            "sub-a",
            PROVIDER_SUBSCRIPTION,
            "2026-06-15T00:00:00Z",
        )
        .unwrap();
        record_attribution_at(
            &db_path,
            OWNER_TYPE_API_PROFILE,
            "api-a",
            PROVIDER_API,
            "2026-06-15T00:00:00Z",
        )
        .unwrap();
        write_session(
            &codex_home,
            "15",
            "rollout-sub",
            &[
                session_meta_line(
                    "session-sub",
                    PROVIDER_SUBSCRIPTION,
                    "2026-06-15T01:00:00Z",
                    None,
                ),
                turn_context_line("2026-06-15T01:00:30Z", "gpt-5.5"),
                token_count_line("2026-06-15T01:05:00Z", 100, 0, 20, 5, 120, 258_400),
            ],
        );
        write_session(
            &codex_home,
            "15",
            "rollout-api",
            &[
                session_meta_line("session-api", PROVIDER_API, "2026-06-15T02:00:00Z", None),
                turn_context_line("2026-06-15T02:00:30Z", "gpt-5.4 mini"),
                token_count_line("2026-06-15T02:05:00Z", 200, 50, 40, 10, 240, 128_000),
            ],
        );

        let response =
            usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-15T03:00:00Z").unwrap();

        assert_eq!(
            window_total(&response, "subscriptions", "sub-a", "all"),
            120
        );
        assert_eq!(window_total(&response, "api_profiles", "api-a", "all"), 240);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn aggregates_usage_by_model_inside_each_window() {
        let root = temp_root("by-model");
        let db_path = root.join("usage.sqlite");
        let codex_home = root.join("codex");
        record_attribution_at(
            &db_path,
            OWNER_TYPE_API_PROFILE,
            "api-a",
            PROVIDER_API,
            "2026-06-15T00:00:00Z",
        )
        .unwrap();
        write_session(
            &codex_home,
            "15",
            "rollout-model-a",
            &[
                session_meta_line(
                    "session-model-a",
                    PROVIDER_API,
                    "2026-06-15T01:00:00Z",
                    None,
                ),
                turn_context_line("2026-06-15T01:00:30Z", "gpt-5.5"),
                token_count_line("2026-06-15T01:05:00Z", 100, 0, 20, 5, 120, 258_400),
            ],
        );
        write_session(
            &codex_home,
            "15",
            "rollout-model-b",
            &[
                session_meta_line(
                    "session-model-b",
                    PROVIDER_API,
                    "2026-06-15T02:00:00Z",
                    None,
                ),
                turn_context_line("2026-06-15T02:00:30Z", "gpt-5.4 mini"),
                token_count_line("2026-06-15T02:05:00Z", 200, 50, 40, 10, 240, 128_000),
            ],
        );

        let response =
            usage_stats_get_for_paths(&db_path, &codex_home, "2026-06-15T03:00:00Z").unwrap();

        assert_eq!(window_total(&response, "api_profiles", "api-a", "all"), 360);
        assert_eq!(
            model_window(&response, "api_profiles", "api-a", "all", "gpt-5.5")
                .get("total_tokens")
                .and_then(Value::as_u64),
            Some(120)
        );
        assert_eq!(
            model_window(&response, "api_profiles", "api-a", "all", "gpt-5.4 mini")
                .get("total_tokens")
                .and_then(Value::as_u64),
            Some(240)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cost_formula_uses_cached_input_and_output_prices() {
        let usage = TokenUsage {
            input_tokens: 1_000_000,
            cached_input_tokens: 250_000,
            output_tokens: 100_000,
            reasoning_output_tokens: 25_000,
            total_tokens: 1_100_000,
        };

        let estimate = estimate_cost("gpt-5.4-mini", &usage, Some(128_000));

        assert!(estimate.priced);
        let expected = 0.75 * 0.75 + 0.25 * 0.075 + 0.1 * 4.5;
        assert!((estimate.cost_usd.unwrap() - expected).abs() < 0.000_001);
    }

    #[test]
    fn cumulative_input_above_threshold_still_uses_short_context_price() {
        let usage = TokenUsage {
            input_tokens: 753_341,
            cached_input_tokens: 661_376,
            output_tokens: 5_386,
            reasoning_output_tokens: 2_117,
            total_tokens: 758_727,
        };

        let estimate = estimate_cost("gpt-5.5", &usage, Some(258_400));

        assert!(estimate.priced);
        assert_eq!(
            estimate.pricing_context,
            Some(PRICING_CONTEXT_STANDARD_SHORT)
        );
        let expected = per_million_cost(91_965, 5.0)
            + per_million_cost(661_376, 0.5)
            + per_million_cost(5_386, 30.0);
        assert!((estimate.cost_usd.unwrap() - expected).abs() < 0.000_001);
    }

    #[test]
    fn long_context_uses_standard_long_context_prices() {
        let usage = TokenUsage {
            input_tokens: 300_000,
            cached_input_tokens: 100_000,
            output_tokens: 10_000,
            reasoning_output_tokens: 1_000,
            total_tokens: 310_000,
        };

        let estimate = estimate_cost("gpt-5.5", &usage, Some(LONG_CONTEXT_THRESHOLD_TOKENS));

        assert!(estimate.priced);
        assert_eq!(
            estimate.pricing_context,
            Some(PRICING_CONTEXT_STANDARD_LONG)
        );
        let expected = per_million_cost(200_000, 10.0)
            + per_million_cost(100_000, 1.0)
            + per_million_cost(10_000, 45.0);
        assert!((estimate.cost_usd.unwrap() - expected).abs() < 0.000_001);
    }

    #[test]
    fn missing_price_is_tokens_only() {
        let usage = TokenUsage {
            input_tokens: 100,
            cached_input_tokens: 0,
            output_tokens: 20,
            reasoning_output_tokens: 5,
            total_tokens: 120,
        };

        let estimate = estimate_cost("unknown-model", &usage, Some(128_000));

        assert!(!estimate.priced);
        assert!(estimate.cost_usd.is_none());
        assert_eq!(
            estimate.unpriced_reason,
            Some(UNPRICED_REASON_MISSING_MODEL_PRICE)
        );
    }
}
