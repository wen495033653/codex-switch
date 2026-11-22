use super::model::{EstimatedCost, ServiceTier, TokenPrices, TokenUsage, OWNER_TYPE_SUBSCRIPTION};

pub(super) const PRICING_SOURCE: &str = "https://developers.openai.com/api/docs/pricing";

/// The day the table below was copied from `PRICING_SOURCE`. Costs are fixed when a record is
/// stored, so a later table change leaves priced records as they are; records stored without a
/// price are priced once against each new table (`db::pending_reprice`).
pub(super) const PRICING_UPDATED_AT: &str = "2026-11-23";

/// "Short context: ≤272K input tokens, long context: >272K input tokens", judged per request.
pub(super) const LONG_CONTEXT_THRESHOLD_TOKENS: u64 = 272_000;

pub(super) const PRICE_LABEL_STANDARD_SHORT: &str = "standard_short_context";

pub(super) const PRICE_LABEL_STANDARD_LONG: &str = "standard_long_context";

pub(super) const PRICE_LABEL_FAST_SHORT: &str = "fast_short_context";

pub(super) const PRICE_LABEL_FAST_LONG: &str = "fast_long_context";

pub(super) const PRICE_LABEL_ULTRAFAST_SHORT: &str = "ultrafast_short_context";

pub(super) const PRICE_LABEL_ULTRAFAST_LONG: &str = "ultrafast_long_context";

pub(super) const PRICE_LABEL_AUTO_REVIEW_FREE: &str = "auto_review_free";

pub(super) const AUTO_REVIEW_MODEL: &str = "codex-auto-review";

/// Start of the UTC day after the waiver was announced on October 6, 2026. Do not assume
/// it applied before the announcement or at any particular time on the rollout day.
pub(super) const AUTO_REVIEW_FREE_SINCE: i64 = 1_791_331_200;

pub(super) const UNPRICED_MISSING_MODEL_PRICE: &str = "missing_model_price";

pub(super) const UNPRICED_MISSING_TIER_PRICE: &str = "missing_tier_price";

pub(super) const UNPRICED_MISSING_CACHED_INPUT_PRICE: &str = "missing_cached_input_price";

pub(super) const UNPRICED_MISSING_CACHE_WRITE_PRICE: &str = "missing_cache_write_price";

/// Stored while its model had no price, and not found when the rollouts were read again for a
/// table that prices the model: the service tier it ran on cannot be known.
pub(super) const UNPRICED_MISSING_SERVICE_TIER: &str = "missing_service_tier";

const PRICED_LABELS: &[&str] = &[
    PRICE_LABEL_STANDARD_SHORT,
    PRICE_LABEL_STANDARD_LONG,
    PRICE_LABEL_FAST_SHORT,
    PRICE_LABEL_FAST_LONG,
    PRICE_LABEL_ULTRAFAST_SHORT,
    PRICE_LABEL_ULTRAFAST_LONG,
    PRICE_LABEL_AUTO_REVIEW_FREE,
];

#[derive(Clone, Copy)]
struct TierPrices {
    short_context: TokenPrices,
    /// `None` for a model priced per context length means the page lists no long-context price
    /// for this tier.
    long_context: Option<TokenPrices>,
}

#[derive(Clone, Copy)]
struct ModelPrice {
    model: &'static str,
    /// Whether the page prices requests above the threshold separately for this model.
    long_context_priced: bool,
    standard: TierPrices,
    fast: TierPrices,
}

const fn prices(
    input: f64,
    cached_input: f64,
    cache_write: Option<f64>,
    output: f64,
) -> TokenPrices {
    TokenPrices {
        input,
        cached_input: Some(cached_input),
        cache_write,
        output,
    }
}

/// Standard and Fast mode tables of `PRICING_SOURCE`, USD per 1M tokens, with 30-minute cache writes.
const MODEL_PRICES: &[ModelPrice] = &[
    ModelPrice {
        model: "gpt-6.2-astra",
        long_context_priced: true,
        standard: TierPrices {
            short_context: prices(8.0, 0.4, Some(10.0), 40.0),
            long_context: Some(prices(16.0, 0.8, Some(20.0), 60.0)),
        },
        fast: TierPrices {
            short_context: prices(16.0, 0.8, Some(20.0), 80.0),
            long_context: Some(prices(32.0, 1.6, Some(40.0), 120.0)),
        },
    },
    ModelPrice {
        model: "gpt-6.2-sol",
        long_context_priced: true,
        standard: TierPrices {
            short_context: prices(2.0, 0.1, Some(2.5), 10.0),
            long_context: Some(prices(4.0, 0.2, Some(5.0), 15.0)),
        },
        fast: TierPrices {
            short_context: prices(4.0, 0.2, Some(5.0), 20.0),
            long_context: Some(prices(8.0, 0.4, Some(10.0), 30.0)),
        },
    },
    ModelPrice {
        model: "gpt-6-astra",
        long_context_priced: true,
        standard: TierPrices {
            short_context: prices(10.0, 1.0, Some(12.5), 50.0),
            long_context: Some(prices(20.0, 2.0, Some(25.0), 75.0)),
        },
        fast: TierPrices {
            short_context: prices(20.0, 2.0, Some(25.0), 100.0),
            long_context: Some(prices(40.0, 4.0, Some(50.0), 150.0)),
        },
    },
    ModelPrice {
        model: "gpt-6.1-sol",
        long_context_priced: true,
        standard: TierPrices {
            short_context: prices(2.0, 0.1, Some(2.5), 10.0),
            long_context: Some(prices(4.0, 0.2, Some(5.0), 15.0)),
        },
        fast: TierPrices {
            short_context: prices(4.0, 0.2, Some(5.0), 20.0),
            long_context: Some(prices(8.0, 0.4, Some(10.0), 30.0)),
        },
    },
    ModelPrice {
        model: "gpt-6.1-terra",
        long_context_priced: true,
        standard: TierPrices {
            short_context: prices(1.0, 0.05, Some(1.25), 5.0),
            long_context: Some(prices(2.0, 0.1, Some(2.5), 7.5)),
        },
        fast: TierPrices {
            short_context: prices(2.0, 0.1, Some(2.5), 10.0),
            long_context: Some(prices(4.0, 0.2, Some(5.0), 15.0)),
        },
    },
    ModelPrice {
        model: "gpt-6-sol",
        long_context_priced: true,
        standard: TierPrices {
            short_context: prices(2.0, 0.2, Some(2.5), 10.0),
            long_context: Some(prices(4.0, 0.4, Some(5.0), 15.0)),
        },
        fast: TierPrices {
            short_context: prices(4.0, 0.4, Some(5.0), 20.0),
            long_context: Some(prices(8.0, 0.8, Some(10.0), 30.0)),
        },
    },
    ModelPrice {
        model: "gpt-6-luna",
        long_context_priced: true,
        standard: TierPrices {
            short_context: prices(0.1, 0.01, Some(0.125), 0.5),
            long_context: Some(prices(0.2, 0.02, Some(0.25), 0.75)),
        },
        fast: TierPrices {
            short_context: prices(0.2, 0.02, Some(0.25), 1.0),
            long_context: Some(prices(0.4, 0.04, Some(0.5), 1.5)),
        },
    },
    ModelPrice {
        model: "gpt-5.6-sol",
        long_context_priced: true,
        standard: TierPrices {
            short_context: prices(4.0, 0.4, Some(5.0), 20.0),
            long_context: Some(prices(8.0, 0.8, Some(10.0), 30.0)),
        },
        fast: TierPrices {
            short_context: prices(8.0, 0.8, Some(10.0), 40.0),
            long_context: Some(prices(16.0, 1.6, Some(20.0), 60.0)),
        },
    },
    ModelPrice {
        model: "gpt-5.6-terra",
        long_context_priced: true,
        standard: TierPrices {
            short_context: prices(2.0, 0.2, Some(2.5), 12.0),
            long_context: Some(prices(4.0, 0.4, Some(5.0), 18.0)),
        },
        fast: TierPrices {
            short_context: prices(4.0, 0.4, Some(5.0), 24.0),
            long_context: Some(prices(8.0, 0.8, Some(10.0), 36.0)),
        },
    },
    ModelPrice {
        model: "gpt-5.6-luna",
        long_context_priced: true,
        standard: TierPrices {
            short_context: prices(0.2, 0.02, Some(0.25), 1.2),
            long_context: Some(prices(0.4, 0.04, Some(0.5), 1.8)),
        },
        fast: TierPrices {
            short_context: prices(0.4, 0.04, Some(0.5), 2.4),
            long_context: Some(prices(0.8, 0.08, Some(1.0), 3.6)),
        },
    },
    ModelPrice {
        model: "gpt-5.5",
        long_context_priced: true,
        standard: TierPrices {
            short_context: prices(5.0, 0.5, None, 30.0),
            long_context: Some(prices(10.0, 1.0, None, 45.0)),
        },
        fast: TierPrices {
            short_context: prices(12.5, 1.25, None, 75.0),
            long_context: None,
        },
    },
    ModelPrice {
        model: "gpt-5.4",
        long_context_priced: true,
        standard: TierPrices {
            short_context: prices(2.5, 0.25, None, 15.0),
            long_context: Some(prices(5.0, 0.5, None, 22.5)),
        },
        fast: TierPrices {
            short_context: prices(5.0, 0.5, None, 30.0),
            long_context: None,
        },
    },
    ModelPrice {
        model: "gpt-5.4-mini",
        long_context_priced: false,
        standard: TierPrices {
            short_context: prices(0.75, 0.075, None, 4.5),
            long_context: None,
        },
        fast: TierPrices {
            short_context: prices(1.5, 0.15, None, 9.0),
            long_context: None,
        },
    },
];

/// March 1, 2027, 00:00 Pacific (UTC-08:00). GPT-6.2 switches from the promotional
/// 3x rates to the 6x list rates. Use the response timestamp even when repricing old records.
const ULTRAFAST_LIST_RATES_SINCE: i64 = 1_803_888_000;

/// `PRICING_SOURCE` Ultrafast tables, USD per 1M tokens, with 30-minute cache writes.
fn ultrafast_prices(model: &str, timestamp_seconds: i64) -> Option<TierPrices> {
    let promo = timestamp_seconds < ULTRAFAST_LIST_RATES_SINCE;
    Some(match model {
        "gpt-6.2-astra" if promo => TierPrices {
            short_context: prices(24.0, 1.2, Some(30.0), 120.0),
            long_context: Some(prices(48.0, 2.4, Some(60.0), 180.0)),
        },
        "gpt-6.2-sol" if promo => TierPrices {
            short_context: prices(6.0, 0.3, Some(7.5), 30.0),
            long_context: Some(prices(12.0, 0.6, Some(15.0), 45.0)),
        },
        "gpt-6.2-astra" => TierPrices {
            short_context: prices(48.0, 2.4, Some(60.0), 240.0),
            long_context: Some(prices(96.0, 4.8, Some(120.0), 360.0)),
        },
        "gpt-6.2-sol" | "gpt-6.1-sol" => TierPrices {
            short_context: prices(12.0, 0.6, Some(15.0), 60.0),
            long_context: Some(prices(24.0, 1.2, Some(30.0), 90.0)),
        },
        "gpt-6-astra" => TierPrices {
            short_context: prices(60.0, 6.0, Some(75.0), 300.0),
            long_context: Some(prices(120.0, 12.0, Some(150.0), 450.0)),
        },
        _ => return None,
    })
}

pub(super) fn is_priced_label(label: &str) -> bool {
    PRICED_LABELS.contains(&label)
}

/// Prices one API response: the tier it ran on and, outside a subscription, long context when
/// this request's input is above the threshold. A ChatGPT subscription has no long-context
/// surcharge or separate cache-write premium; responses no owner matched use API rates.
pub(super) fn estimate_request_cost(
    model: &str,
    usage: &TokenUsage,
    tier: ServiceTier,
    owner_type: &str,
    timestamp_seconds: i64,
) -> EstimatedCost {
    let subscription = owner_type == OWNER_TYPE_SUBSCRIPTION;
    if model == AUTO_REVIEW_MODEL && subscription && timestamp_seconds >= AUTO_REVIEW_FREE_SINCE {
        return auto_review_waived_cost();
    }
    let long_context = !subscription && usage.input_tokens > LONG_CONTEXT_THRESHOLD_TOKENS;
    price_tokens(
        model,
        usage,
        tier,
        long_context,
        subscription,
        timestamp_seconds,
    )
}

/// Auto-review requires a ChatGPT sign-in for the waiver and has no public API model price.
pub(super) fn auto_review_waived_cost() -> EstimatedCost {
    EstimatedCost {
        cost_usd: Some(0.0),
        price_label: PRICE_LABEL_AUTO_REVIEW_FREE,
        prices: Some(prices(0.0, 0.0, Some(0.0), 0.0)),
    }
}

/// Prices usage summed over several requests whose sizes are unknown (the statistics kept
/// before per-response records): standard processing, short context.
pub(super) fn estimate_summed_cost(model: &str, usage: &TokenUsage) -> EstimatedCost {
    price_tokens(model, usage, ServiceTier::Standard, false, false, 0)
}

fn price_tokens(
    model: &str,
    usage: &TokenUsage,
    tier: ServiceTier,
    long_context: bool,
    subscription: bool,
    timestamp_seconds: i64,
) -> EstimatedCost {
    let Some(price) = find_model_price(model) else {
        return unpriced(UNPRICED_MISSING_MODEL_PRICE);
    };
    let (tier_prices, short_label, long_label) = match tier {
        ServiceTier::Standard => (
            price.standard,
            PRICE_LABEL_STANDARD_SHORT,
            PRICE_LABEL_STANDARD_LONG,
        ),
        ServiceTier::Fast => (price.fast, PRICE_LABEL_FAST_SHORT, PRICE_LABEL_FAST_LONG),
        ServiceTier::Ultrafast => {
            let Some(tier_prices) = ultrafast_prices(price.model, timestamp_seconds) else {
                return unpriced(UNPRICED_MISSING_TIER_PRICE);
            };
            (
                tier_prices,
                PRICE_LABEL_ULTRAFAST_SHORT,
                PRICE_LABEL_ULTRAFAST_LONG,
            )
        }
        ServiceTier::Other => return unpriced(UNPRICED_MISSING_TIER_PRICE),
    };
    let (mut token_prices, label) = if long_context && price.long_context_priced {
        match tier_prices.long_context {
            Some(long_prices) => (long_prices, long_label),
            None => return unpriced(UNPRICED_MISSING_TIER_PRICE),
        }
    } else {
        (tier_prices.short_context, short_label)
    };
    if subscription {
        token_prices.cache_write = Some(token_prices.input);
    }

    match cost_parts(usage, &token_prices) {
        Ok(parts) => EstimatedCost {
            cost_usd: Some(parts.total()),
            price_label: label,
            prices: Some(token_prices),
        },
        Err(reason) => unpriced(reason),
    }
}

/// Prices for backfilling the schema that predated stored unit prices (and Ultrafast).
pub(super) fn prices_for_label(model: &str, label: &str) -> Option<TokenPrices> {
    let price = find_model_price(model)?;
    match label {
        PRICE_LABEL_STANDARD_SHORT => Some(price.standard.short_context),
        PRICE_LABEL_STANDARD_LONG => price.standard.long_context,
        PRICE_LABEL_FAST_SHORT => Some(price.fast.short_context),
        PRICE_LABEL_FAST_LONG => price.fast.long_context,
        _ => None,
    }
}

/// How a request's input tokens are billed: each one is plain input, cached input or a cache
/// write, never two of them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct InputSplit {
    pub(super) plain: u64,
    pub(super) cached: u64,
    pub(super) cache_write: u64,
}

pub(super) fn split_input(usage: &TokenUsage) -> InputSplit {
    let cached = usage.cached_input_tokens.min(usage.input_tokens);
    let cache_write = usage
        .cache_write_input_tokens
        .min(usage.input_tokens - cached);
    InputSplit {
        plain: usage.input_tokens - cached - cache_write,
        cached,
        cache_write,
    }
}

/// USD charged for each kind of token of one request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CostParts {
    pub(super) input: f64,
    pub(super) cached_input: f64,
    pub(super) cache_write: f64,
    pub(super) output: f64,
}

impl CostParts {
    pub(super) fn total(&self) -> f64 {
        self.input + self.cached_input + self.cache_write + self.output
    }
}

/// `Err` is the unpriced reason when the request has tokens of a kind `prices` has no price for.
pub(super) fn cost_parts(
    usage: &TokenUsage,
    prices: &TokenPrices,
) -> Result<CostParts, &'static str> {
    let split = split_input(usage);
    let cached_input = optional_cost(split.cached, prices.cached_input)
        .ok_or(UNPRICED_MISSING_CACHED_INPUT_PRICE)?;
    let cache_write = optional_cost(split.cache_write, prices.cache_write)
        .ok_or(UNPRICED_MISSING_CACHE_WRITE_PRICE)?;
    Ok(CostParts {
        input: per_million_cost(split.plain, prices.input),
        cached_input,
        cache_write,
        output: per_million_cost(usage.output_tokens, prices.output),
    })
}

fn unpriced(reason: &'static str) -> EstimatedCost {
    EstimatedCost {
        cost_usd: None,
        price_label: reason,
        prices: None,
    }
}

// No tokens of a kind need no price for it.
fn optional_cost(tokens: u64, price_per_million: Option<f64>) -> Option<f64> {
    if tokens == 0 {
        return Some(0.0);
    }
    price_per_million.map(|price| per_million_cost(tokens, price))
}

/// Whether the table prices this model in any tier.
pub(super) fn has_model_price(model: &str) -> bool {
    find_model_price(model).is_some()
}

fn find_model_price(model: &str) -> Option<&'static ModelPrice> {
    let normalized_model = normalize_model_id(model);
    MODEL_PRICES
        .iter()
        .find(|price| price.model == normalized_model)
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

fn per_million_cost(tokens: u64, price_per_million: f64) -> f64 {
    (tokens as f64 / 1_000_000.0) * price_per_million
}
