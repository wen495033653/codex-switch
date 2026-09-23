use super::model::{EstimatedCost, TokenUsage};

pub(super) const PRICING_SOURCE: &str = "https://developers.openai.com/api/docs/pricing";

/// The day the table below was copied from `PRICING_SOURCE`. Costs are fixed when a record is
/// stored, so a later table change applies to new records only.
pub(super) const PRICING_UPDATED_AT: &str = "2026-09-23";

/// "Short context: ≤272K input tokens, long context: >272K input tokens", judged per request.
pub(super) const LONG_CONTEXT_THRESHOLD_TOKENS: u64 = 272_000;

pub(super) const PRICE_LABEL_STANDARD_SHORT: &str = "standard_short_context";

pub(super) const PRICE_LABEL_STANDARD_LONG: &str = "standard_long_context";

pub(super) const PRICE_LABEL_FAST_SHORT: &str = "fast_short_context";

pub(super) const PRICE_LABEL_FAST_LONG: &str = "fast_long_context";

pub(super) const UNPRICED_MISSING_MODEL_PRICE: &str = "missing_model_price";

pub(super) const UNPRICED_MISSING_TIER_PRICE: &str = "missing_tier_price";

pub(super) const UNPRICED_MISSING_CACHED_INPUT_PRICE: &str = "missing_cached_input_price";

pub(super) const UNPRICED_MISSING_CACHE_WRITE_PRICE: &str = "missing_cache_write_price";

const PRICED_LABELS: &[&str] = &[
    PRICE_LABEL_STANDARD_SHORT,
    PRICE_LABEL_STANDARD_LONG,
    PRICE_LABEL_FAST_SHORT,
    PRICE_LABEL_FAST_LONG,
];

/// The processing tier a request asked for, from Codex's `thread_settings.service_tier`.
/// `priority` is the former name of Fast mode; the API accepts both names.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ServiceTier {
    Standard,
    Fast,
    /// A tier this table has no prices for (for example `flex`).
    Other,
}

impl ServiceTier {
    pub(super) fn from_setting(value: &str) -> Self {
        match value.trim() {
            // Without a service_tier the request runs on standard processing.
            "" | "default" => Self::Standard,
            "priority" | "fast" => Self::Fast,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Copy)]
struct TokenPrices {
    input: f64,
    cached_input: Option<f64>,
    cache_write: Option<f64>,
    output: f64,
}

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

/// Standard and Fast mode tables of `PRICING_SOURCE`, USD per 1M tokens.
const MODEL_PRICES: &[ModelPrice] = &[
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

pub(super) fn is_priced_label(label: &str) -> bool {
    PRICED_LABELS.contains(&label)
}

/// Prices one API response: the tier it ran on, and long context when this request's input is
/// above the threshold.
pub(super) fn estimate_request_cost(
    model: &str,
    usage: &TokenUsage,
    tier: ServiceTier,
) -> EstimatedCost {
    price_tokens(
        model,
        usage,
        tier,
        usage.input_tokens > LONG_CONTEXT_THRESHOLD_TOKENS,
    )
}

/// Prices usage summed over several requests whose sizes are unknown (the statistics kept
/// before per-response records): standard processing, short context.
pub(super) fn estimate_summed_cost(model: &str, usage: &TokenUsage) -> EstimatedCost {
    price_tokens(model, usage, ServiceTier::Standard, false)
}

fn price_tokens(
    model: &str,
    usage: &TokenUsage,
    tier: ServiceTier,
    long_context: bool,
) -> EstimatedCost {
    let normalized_model = normalize_model_id(model);
    let Some(price) = MODEL_PRICES
        .iter()
        .find(|price| price.model == normalized_model)
    else {
        return unpriced(UNPRICED_MISSING_MODEL_PRICE);
    };
    let (tier_prices, short_label, long_label) = match tier {
        ServiceTier::Standard => (
            &price.standard,
            PRICE_LABEL_STANDARD_SHORT,
            PRICE_LABEL_STANDARD_LONG,
        ),
        ServiceTier::Fast => (&price.fast, PRICE_LABEL_FAST_SHORT, PRICE_LABEL_FAST_LONG),
        ServiceTier::Other => return unpriced(UNPRICED_MISSING_TIER_PRICE),
    };
    let (token_prices, label) = if long_context && price.long_context_priced {
        match tier_prices.long_context.as_ref() {
            Some(long_prices) => (long_prices, long_label),
            None => return unpriced(UNPRICED_MISSING_TIER_PRICE),
        }
    } else {
        (&tier_prices.short_context, short_label)
    };

    // Input tokens are either input, cached input or cache writes; the three do not add up.
    let cached_input_tokens = usage.cached_input_tokens.min(usage.input_tokens);
    let cache_write_input_tokens = usage
        .cache_write_input_tokens
        .min(usage.input_tokens - cached_input_tokens);
    let plain_input_tokens = usage.input_tokens - cached_input_tokens - cache_write_input_tokens;
    let Some(cached_cost) = optional_cost(cached_input_tokens, token_prices.cached_input) else {
        return unpriced(UNPRICED_MISSING_CACHED_INPUT_PRICE);
    };
    let Some(cache_write_cost) = optional_cost(cache_write_input_tokens, token_prices.cache_write)
    else {
        return unpriced(UNPRICED_MISSING_CACHE_WRITE_PRICE);
    };
    let cost = per_million_cost(plain_input_tokens, token_prices.input)
        + cached_cost
        + cache_write_cost
        + per_million_cost(usage.output_tokens, token_prices.output);
    EstimatedCost {
        cost_usd: Some(cost),
        price_label: label,
    }
}

fn unpriced(reason: &'static str) -> EstimatedCost {
    EstimatedCost {
        cost_usd: None,
        price_label: reason,
    }
}

// No tokens of a kind need no price for it.
fn optional_cost(tokens: u64, price_per_million: Option<f64>) -> Option<f64> {
    if tokens == 0 {
        return Some(0.0);
    }
    price_per_million.map(|price| per_million_cost(tokens, price))
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
