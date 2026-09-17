use std::path::PathBuf;

pub(super) const OWNER_TYPE_SUBSCRIPTION: &str = "subscription";

pub(super) const OWNER_TYPE_API_PROFILE: &str = "api_profile";

pub(super) const PROVIDER_SUBSCRIPTION: &str = "openai";

pub(super) const PROVIDER_API: &str = "api";

#[derive(Default)]
pub(super) struct ScanWarnings {
    pub(super) missing_attribution: u64,
    pub(super) missing_price: u64,
    pub(super) skipped_before_start: u64,
}

#[derive(Clone, Default)]
pub(super) struct TokenUsage {
    pub(super) input_tokens: u64,
    pub(super) cached_input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) reasoning_output_tokens: u64,
    pub(super) total_tokens: u64,
}

impl TokenUsage {
    pub(super) fn has_tokens(&self) -> bool {
        self.total_tokens > 0
    }

    pub(super) fn add_assign(&mut self, other: &TokenUsage) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.cached_input_tokens = self
            .cached_input_tokens
            .saturating_add(other.cached_input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.reasoning_output_tokens = self
            .reasoning_output_tokens
            .saturating_add(other.reasoning_output_tokens);
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
    }

    pub(super) fn saturating_delta(&self, previous: &TokenUsage) -> TokenUsage {
        TokenUsage {
            input_tokens: self.input_tokens.saturating_sub(previous.input_tokens),
            cached_input_tokens: self
                .cached_input_tokens
                .saturating_sub(previous.cached_input_tokens),
            output_tokens: self.output_tokens.saturating_sub(previous.output_tokens),
            reasoning_output_tokens: self
                .reasoning_output_tokens
                .saturating_sub(previous.reasoning_output_tokens),
            total_tokens: self.total_tokens.saturating_sub(previous.total_tokens),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct UsageWindowStarts {
    pub(super) today: i64,
    pub(super) days_7: i64,
    pub(super) days_30: i64,
}

#[derive(Default)]
pub(super) struct TokenUsageWindows {
    pub(super) today: TokenUsage,
    pub(super) days_7: TokenUsage,
    pub(super) days_30: TokenUsage,
}

pub(super) struct TokenUsageEvent {
    pub(super) timestamp_seconds: i64,
    pub(super) usage: TokenUsage,
}

#[derive(Clone)]
pub(super) struct TimestampValue {
    pub(super) raw: String,
    pub(super) seconds: i64,
}

#[derive(Default)]
pub(super) struct ParsedSession {
    pub(super) session_id: String,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) started_at: Option<TimestampValue>,
    pub(super) updated_at: Option<TimestampValue>,
    pub(super) usage: Option<TokenUsage>,
    pub(super) model_context_window: Option<u64>,
    pub(super) previous_event_usage: Option<TokenUsage>,
    pub(super) window_usage: TokenUsageWindows,
    pub(super) token_events: Vec<TokenUsageEvent>,
}

#[derive(Clone)]
pub(super) struct OwnerAttribution {
    pub(super) owner_type: String,
    pub(super) owner_id: String,
}

pub(super) struct UsageScanSource {
    pub(super) codex_home: PathBuf,
    pub(super) attribution_override: Option<OwnerAttribution>,
}

pub(super) struct EstimatedCost {
    pub(super) cost_usd: Option<f64>,
    pub(super) priced: bool,
    pub(super) pricing_context: Option<&'static str>,
    pub(super) unpriced_reason: Option<&'static str>,
}
