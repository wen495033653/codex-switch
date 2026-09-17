use super::{
    db::{db_error, sql_i64_to_u64},
    model::{EstimatedCost, TokenUsage},
};
use rusqlite::OptionalExtension;
use rusqlite::{params, Connection};

const META_PRICING_UPDATED_AT: &str = "pricing_updated_at";

pub(super) const PRICING_SOURCE: &str = "https://developers.openai.com/api/docs/pricing";

pub(super) const PRICING_UPDATED_AT: &str = "2026-07-10";

pub(super) const LONG_CONTEXT_THRESHOLD_TOKENS: u64 = 270_000;

pub(super) const PRICING_CONTEXT_STANDARD_SHORT: &str = "standard_short_context";

pub(super) const PRICING_CONTEXT_STANDARD_LONG: &str = "standard_long_context";

pub(super) const UNPRICED_REASON_MISSING_MODEL_PRICE: &str = "missing_model_price";

const UNPRICED_REASON_MISSING_CACHED_INPUT_PRICE: &str = "missing_cached_input_price";

const MODEL_PRICES: &[ModelPrice] = &[
    ModelPrice {
        model: "gpt-5.6-sol",
        short_context: TokenPrices {
            input_per_million: 5.0,
            cached_input_per_million: Some(0.5),
            output_per_million: 30.0,
        },
        long_context: None,
        long_context_threshold: None,
    },
    ModelPrice {
        model: "gpt-5.6-terra",
        short_context: TokenPrices {
            input_per_million: 2.5,
            cached_input_per_million: Some(0.25),
            output_per_million: 15.0,
        },
        long_context: None,
        long_context_threshold: None,
    },
    ModelPrice {
        model: "gpt-5.6-luna",
        short_context: TokenPrices {
            input_per_million: 1.0,
            cached_input_per_million: Some(0.1),
            output_per_million: 6.0,
        },
        long_context: None,
        long_context_threshold: None,
    },
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

pub(super) fn recompute_existing_costs_if_needed(connection: &Connection) -> Result<(), String> {
    let existing: Option<String> = connection
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [META_PRICING_UPDATED_AT],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| db_error("读取 token 定价版本失败", err))?;
    if existing.as_deref() == Some(PRICING_UPDATED_AT) {
        return Ok(());
    }
    recompute_existing_costs(connection)?;
    connection
        .execute(
            r#"
            INSERT INTO meta(key, value) VALUES(?1, ?2)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            "#,
            params![META_PRICING_UPDATED_AT, PRICING_UPDATED_AT],
        )
        .map_err(|err| db_error("写入 token 定价版本失败", err))?;
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

pub(super) fn count_unpriced_sessions(connection: &Connection) -> Result<u64, String> {
    connection
        .query_row(
            "SELECT COUNT(*) FROM session_usage WHERE priced = 0",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| u64::try_from(value).unwrap_or(0))
        .map_err(|err| db_error("读取未定价 session 数量失败", err))
}

pub(super) fn estimate_cost(
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
    let normalized = model
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");
    match normalized.as_str() {
        "gpt-5.6" => "gpt-5.6-sol".to_string(),
        _ => normalized,
    }
}

pub(super) fn per_million_cost(tokens: u64, price_per_million: f64) -> f64 {
    (tokens as f64 / 1_000_000.0) * price_per_million
}
