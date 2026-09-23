use std::path::PathBuf;

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

#[derive(Clone, Debug, PartialEq)]
pub(super) struct OwnerAttribution {
    pub(super) owner_type: String,
    pub(super) owner_id: String,
}

pub(super) struct UsageScanSource {
    pub(super) codex_home: PathBuf,
    pub(super) attribution_override: Option<OwnerAttribution>,
}

/// `price_label` is a pricing context when the cost is known, otherwise the unpriced reason.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct EstimatedCost {
    pub(super) cost_usd: Option<f64>,
    pub(super) price_label: &'static str,
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
