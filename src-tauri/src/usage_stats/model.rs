use std::path::PathBuf;
use time::{OffsetDateTime, UtcOffset};

pub(super) const OWNER_TYPE_SUBSCRIPTION: &str = "subscription";

pub(super) const OWNER_TYPE_API_PROFILE: &str = "api_profile";

pub(super) const PROVIDER_SUBSCRIPTION: &str = "openai";

pub(super) const PROVIDER_API: &str = "api";

/// Owner of records no attribution matched. They are stored so the warning can count them, but
/// never shown on a card.
pub(super) const OWNER_TYPE_UNATTRIBUTED: &str = "";

/// Token counts exactly as one Responses API `usage` reports them: `input_tokens` already
/// contains the cached and cache-write tokens, `output_tokens` already contains the reasoning
/// tokens, and `total_tokens` is input plus output.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct TokenUsage {
    pub(super) input_tokens: u64,
    pub(super) cached_input_tokens: u64,
    pub(super) cache_write_input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) reasoning_output_tokens: u64,
    pub(super) total_tokens: u64,
}

impl TokenUsage {
    pub(super) fn add(&mut self, other: &TokenUsage) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.cached_input_tokens = self
            .cached_input_tokens
            .saturating_add(other.cached_input_tokens);
        self.cache_write_input_tokens = self
            .cache_write_input_tokens
            .saturating_add(other.cache_write_input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.reasoning_output_tokens = self
            .reasoning_output_tokens
            .saturating_add(other.reasoning_output_tokens);
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct UsageWindowStarts {
    pub(super) today: i64,
    pub(super) days_7: i64,
    pub(super) days_30: i64,
}

impl UsageWindowStarts {
    /// "Today" starts at local midnight; 7 and 30 days count back from now.
    pub(super) fn at(now_seconds: i64) -> Self {
        Self {
            today: today_start_seconds(now_seconds),
            days_7: now_seconds.saturating_sub(7 * 24 * 60 * 60),
            days_30: now_seconds.saturating_sub(30 * 24 * 60 * 60),
        }
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

#[derive(Clone, Debug, PartialEq)]
pub(super) struct OwnerAttribution {
    pub(super) owner_type: String,
    pub(super) owner_id: String,
}

pub(super) struct UsageScanSource {
    pub(super) codex_home: PathBuf,
    pub(super) attribution_override: Option<OwnerAttribution>,
}

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

/// USD per 1M tokens. The cached-input and cache-write prices are `None` where the price table
/// has none; a request priced without one has no tokens of that kind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TokenPrices {
    pub(super) input: f64,
    pub(super) cached_input: Option<f64>,
    pub(super) cache_write: Option<f64>,
    pub(super) output: f64,
}

/// `price_label` is a pricing context when the cost is known, otherwise the unpriced reason.
/// `prices` are the ones `cost_usd` was computed with, stored with the record so a later change
/// of the price table does not change what an existing record shows.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct EstimatedCost {
    pub(super) cost_usd: Option<f64>,
    pub(super) price_label: &'static str,
    pub(super) prices: Option<TokenPrices>,
}

/// One Responses API response, ready to be stored.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct UsageRecord {
    pub(super) response_id: String,
    pub(super) thread_id: String,
    pub(super) timestamp_seconds: i64,
    pub(super) owner: OwnerAttribution,
    pub(super) model: String,
    pub(super) usage: TokenUsage,
    /// The tier the request ran on; not stored, but needed to price again a record stored
    /// while its model had no price.
    pub(super) service_tier: ServiceTier,
    pub(super) cost: EstimatedCost,
}

/// A summed group of records (one hourly bucket, one all-time total, or one record) as the
/// aggregation reads it.
pub(super) struct UsageItem {
    pub(super) owner_type: String,
    pub(super) owner_id: String,
    pub(super) model: String,
    pub(super) thread_id: String,
    pub(super) price_label: String,
    pub(super) usage: TokenUsage,
    pub(super) cost_usd: Option<f64>,
    pub(super) record_count: u64,
    pub(super) last_used_seconds: i64,
}
