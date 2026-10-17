//! The bill: every stored response with its tokens and the prices it was charged at, filtered by
//! window, owner and model, newest first, one page at a time.

use super::{
    db::{
        db_error, has_session_statistics, is_legacy_response_id, records_counted_after,
        sql_i64_to_u64,
    },
    model::{
        TokenPrices, TokenUsage, UsageWindowStarts, OWNER_TYPE_API_PROFILE,
        OWNER_TYPE_SUBSCRIPTION, OWNER_TYPE_UNATTRIBUTED,
    },
    pricing::{cost_parts, split_input},
};
use crate::time_util::format_unix_seconds;
use rusqlite::{params_from_iter, types::Value as SqlValue, Connection, Row};
use serde_json::{json, Value};

/// The owner filter the page sends for records no attribution matched.
const OWNER_FILTER_UNATTRIBUTED: &str = "unattributed";

const MAX_PAGE_SIZE: u32 = 200;

const RECORD_COLUMNS: &str = "response_id, thread_id, owner_type, owner_id, model, \
     timestamp_seconds, input_tokens, cached_input_tokens, cache_write_input_tokens, \
     output_tokens, reasoning_output_tokens, total_tokens, price_label, estimated_cost_usd, \
     input_price, cached_input_price, cache_write_price, output_price";

#[derive(Clone, Copy, Debug, PartialEq)]
enum BillsRange {
    Today,
    Days7,
    Days30,
    All,
}

impl BillsRange {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "today" => Ok(Self::Today),
            "days_7" => Ok(Self::Days7),
            "days_30" => Ok(Self::Days30),
            "all" => Ok(Self::All),
            _ => Err(format!("token 账单时间范围无效: {value:?}")),
        }
    }

    /// The same window starts as the cards.
    fn start(self, now_seconds: i64) -> Option<i64> {
        let starts = UsageWindowStarts::at(now_seconds);
        match self {
            Self::Today => Some(starts.today),
            Self::Days7 => Some(starts.days_7),
            Self::Days30 => Some(starts.days_30),
            Self::All => None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct BillsQuery {
    range: BillsRange,
    /// `(owner_type, owner_id)` as stored.
    owner: Option<(String, String)>,
    /// An empty model selects the records whose model was unknown.
    model: Option<String>,
    page: u32,
    page_size: u32,
}

impl BillsQuery {
    pub(super) fn new(
        range: &str,
        owner_type: Option<&str>,
        owner_id: Option<&str>,
        model: Option<String>,
        page: u32,
        page_size: u32,
    ) -> Result<Self, String> {
        let owner = match owner_type {
            None => None,
            Some(OWNER_FILTER_UNATTRIBUTED) => {
                Some((OWNER_TYPE_UNATTRIBUTED.to_string(), String::new()))
            }
            Some(kind @ (OWNER_TYPE_SUBSCRIPTION | OWNER_TYPE_API_PROFILE)) => {
                let owner_id = owner_id.map(str::trim).unwrap_or_default();
                if owner_id.is_empty() {
                    return Err("token 账单归属 ID 不能为空".to_string());
                }
                Some((kind.to_string(), owner_id.to_string()))
            }
            Some(other) => return Err(format!("token 账单归属类型无效: {other:?}")),
        };
        if page == 0 {
            return Err("token 账单页码从 1 开始".to_string());
        }
        if page_size == 0 || page_size > MAX_PAGE_SIZE {
            return Err(format!(
                "token 账单每页条数必须在 1 到 {MAX_PAGE_SIZE} 之间: {page_size}"
            ));
        }
        Ok(Self {
            range: BillsRange::parse(range)?,
            owner,
            model,
            page,
            page_size,
        })
    }
}

/// A `WHERE` clause and its values.
struct Filter {
    sql: String,
    values: Vec<SqlValue>,
}

fn filter(since: Option<i64>, owner: Option<&(String, String)>, model: Option<&str>) -> Filter {
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    if let Some(since) = since {
        clauses.push("timestamp_seconds >= ?");
        values.push(SqlValue::Integer(since));
    }
    if let Some((owner_type, owner_id)) = owner {
        clauses.push("owner_type = ? AND owner_id = ?");
        values.push(SqlValue::Text(owner_type.clone()));
        values.push(SqlValue::Text(owner_id.clone()));
    }
    if let Some(model) = model {
        clauses.push("model = ?");
        values.push(SqlValue::Text(model.to_string()));
    }
    let sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    Filter { sql, values }
}

pub(super) fn list_bills(
    connection: &Connection,
    query: &BillsQuery,
    now_seconds: i64,
) -> Result<Value, String> {
    let since = query.range.start(now_seconds);
    let selected = filter(since, query.owner.as_ref(), query.model.as_deref());
    let summary = read_summary(connection, &selected)?;
    // The owner list ignores the model filter so picking a model never hides the owners; the
    // model list follows the owner filter.
    let owners = read_owners(connection, &filter(since, None, None))?;
    let models = read_models(connection, &filter(since, query.owner.as_ref(), None))?;
    let records = read_page(connection, &selected, query.page, query.page_size)?;
    Ok(json!({
        "ok": true,
        "page": query.page,
        "page_size": query.page_size,
        "summary": summary,
        "owners": owners,
        "models": models,
        "records": records,
        "records_counted_after": records_counted_after(connection)?,
        "has_session_statistics": has_session_statistics(connection)?
    }))
}

fn read_summary(connection: &Connection, filter: &Filter) -> Result<Value, String> {
    let sql = format!(
        "SELECT COUNT(*), \
         COALESCE(SUM(input_tokens), 0), \
         COALESCE(SUM(MAX(input_tokens - cached_input_tokens - cache_write_input_tokens, 0)), 0), \
         COALESCE(SUM(cached_input_tokens), 0), \
         COALESCE(SUM(cache_write_input_tokens), 0), \
         COALESCE(SUM(output_tokens), 0), \
         COALESCE(SUM(reasoning_output_tokens), 0), \
         COALESCE(SUM(total_tokens), 0), \
         COALESCE(SUM(estimated_cost_usd), 0.0), \
         COALESCE(SUM(estimated_cost_usd IS NULL), 0) \
         FROM usage_records {}",
        filter.sql
    );
    connection
        .query_row(&sql, params_from_iter(filter.values.iter()), |row| {
            Ok(json!({
                "request_count": row.get::<_, i64>(0).map(sql_i64_to_u64)?,
                "input_tokens": row.get::<_, i64>(1).map(sql_i64_to_u64)?,
                "fresh_input_tokens": row.get::<_, i64>(2).map(sql_i64_to_u64)?,
                "cached_input_tokens": row.get::<_, i64>(3).map(sql_i64_to_u64)?,
                "cache_write_input_tokens": row.get::<_, i64>(4).map(sql_i64_to_u64)?,
                "output_tokens": row.get::<_, i64>(5).map(sql_i64_to_u64)?,
                "reasoning_output_tokens": row.get::<_, i64>(6).map(sql_i64_to_u64)?,
                "total_tokens": row.get::<_, i64>(7).map(sql_i64_to_u64)?,
                "estimated_cost_usd": row.get::<_, f64>(8)?,
                "unpriced_count": row.get::<_, i64>(9).map(sql_i64_to_u64)?
            }))
        })
        .map_err(|err| db_error("读取 token 账单汇总失败", err))
}

fn read_owners(connection: &Connection, filter: &Filter) -> Result<Vec<Value>, String> {
    let sql = format!(
        "SELECT owner_type, owner_id, COUNT(*) AS request_count FROM usage_records {} \
         GROUP BY owner_type, owner_id ORDER BY request_count DESC, owner_type, owner_id",
        filter.sql
    );
    collect_rows(connection, &sql, &filter.values, |row| {
        Ok(json!({
            "owner_type": row.get::<_, String>(0)?,
            "owner_id": row.get::<_, String>(1)?,
            "request_count": row.get::<_, i64>(2).map(sql_i64_to_u64)?
        }))
    })
    .map_err(|err| db_error("读取 token 账单归属失败", err))
}

fn read_models(connection: &Connection, filter: &Filter) -> Result<Vec<Value>, String> {
    let sql = format!(
        "SELECT model, COUNT(*) AS request_count FROM usage_records {} \
         GROUP BY model ORDER BY request_count DESC, model",
        filter.sql
    );
    collect_rows(connection, &sql, &filter.values, |row| {
        Ok(json!({
            "model": row.get::<_, String>(0)?,
            "request_count": row.get::<_, i64>(1).map(sql_i64_to_u64)?
        }))
    })
    .map_err(|err| db_error("读取 token 账单模型失败", err))
}

struct BillRow {
    response_id: String,
    thread_id: String,
    owner_type: String,
    owner_id: String,
    model: String,
    timestamp_seconds: i64,
    usage: TokenUsage,
    price_label: String,
    cost_usd: Option<f64>,
    prices: Option<TokenPrices>,
}

fn bill_row(row: &Row<'_>) -> rusqlite::Result<BillRow> {
    let input_price: Option<f64> = row.get(14)?;
    let output_price: Option<f64> = row.get(17)?;
    Ok(BillRow {
        response_id: row.get(0)?,
        thread_id: row.get(1)?,
        owner_type: row.get(2)?,
        owner_id: row.get(3)?,
        model: row.get(4)?,
        timestamp_seconds: row.get(5)?,
        usage: TokenUsage {
            input_tokens: row.get::<_, i64>(6).map(sql_i64_to_u64)?,
            cached_input_tokens: row.get::<_, i64>(7).map(sql_i64_to_u64)?,
            cache_write_input_tokens: row.get::<_, i64>(8).map(sql_i64_to_u64)?,
            output_tokens: row.get::<_, i64>(9).map(sql_i64_to_u64)?,
            reasoning_output_tokens: row.get::<_, i64>(10).map(sql_i64_to_u64)?,
            total_tokens: row.get::<_, i64>(11).map(sql_i64_to_u64)?,
        },
        price_label: row.get(12)?,
        cost_usd: row.get(13)?,
        prices: match (input_price, output_price) {
            (Some(input), Some(output)) => Some(TokenPrices {
                input,
                cached_input: row.get(15)?,
                cache_write: row.get(16)?,
                output,
            }),
            _ => None,
        },
    })
}

fn read_page(
    connection: &Connection,
    filter: &Filter,
    page: u32,
    page_size: u32,
) -> Result<Vec<Value>, String> {
    let sql = format!(
        "SELECT {RECORD_COLUMNS} FROM usage_records {} \
         ORDER BY timestamp_seconds DESC, response_id DESC LIMIT ? OFFSET ?",
        filter.sql
    );
    let mut values = filter.values.clone();
    values.push(SqlValue::Integer(i64::from(page_size)));
    values.push(SqlValue::Integer(
        i64::from(page - 1) * i64::from(page_size),
    ));
    let rows = collect_rows(connection, &sql, &values, bill_row)
        .map_err(|err| db_error("读取 token 账单失败", err))?;
    rows.iter().map(bill_json).collect()
}

fn bill_json(row: &BillRow) -> Result<Value, String> {
    let timestamp = format_unix_seconds(row.timestamp_seconds).ok_or_else(|| {
        format!(
            "token 账单记录 {} 的时间无效: {}",
            row.response_id, row.timestamp_seconds
        )
    })?;
    let split = split_input(&row.usage);
    Ok(json!({
        "response_id": row.response_id,
        "thread_id": row.thread_id,
        "timestamp": timestamp,
        "owner_type": row.owner_type,
        "owner_id": row.owner_id,
        "model": row.model,
        "price_label": row.price_label,
        "legacy": is_legacy_response_id(&row.response_id),
        "input_tokens": row.usage.input_tokens,
        "fresh_input_tokens": split.plain,
        "cached_input_tokens": row.usage.cached_input_tokens,
        "cache_write_input_tokens": row.usage.cache_write_input_tokens,
        "output_tokens": row.usage.output_tokens,
        "reasoning_output_tokens": row.usage.reasoning_output_tokens,
        "total_tokens": row.usage.total_tokens,
        "estimated_cost_usd": row.cost_usd,
        "charges": charges_json(row)?
    }))
}

/// Each kind of token with the price it was charged at. `null` for an unpriced record, and for a
/// priced one whose prices could not be recovered when they started to be stored.
fn charges_json(row: &BillRow) -> Result<Value, String> {
    let (Some(_), Some(prices)) = (row.cost_usd, row.prices.as_ref()) else {
        return Ok(Value::Null);
    };
    let parts = cost_parts(&row.usage, prices).map_err(|reason| {
        format!(
            "token 账单记录 {} 存储的单价不全: {reason}",
            row.response_id
        )
    })?;
    let split = split_input(&row.usage);
    Ok(json!([
        {
            "kind": "input",
            "tokens": split.plain,
            "price_per_million": prices.input,
            "cost_usd": parts.input
        },
        {
            "kind": "cached_input",
            "tokens": split.cached,
            "price_per_million": prices.cached_input,
            "cost_usd": parts.cached_input
        },
        {
            "kind": "cache_write",
            "tokens": split.cache_write,
            "price_per_million": prices.cache_write,
            "cost_usd": parts.cache_write
        },
        {
            "kind": "output",
            "tokens": row.usage.output_tokens,
            "price_per_million": prices.output,
            "cost_usd": parts.output
        }
    ]))
}

fn collect_rows<T>(
    connection: &Connection,
    sql: &str,
    values: &[SqlValue],
    map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
) -> rusqlite::Result<Vec<T>> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map(params_from_iter(values.iter()), map)?;
    rows.collect()
}
