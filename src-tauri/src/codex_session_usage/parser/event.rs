use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct SessionEvent {
    pub(super) timestamp: String,
    #[serde(rename = "type")]
    pub(super) event_type: String,
    pub(super) payload: Option<SessionPayload>,
}

#[derive(Deserialize)]
pub(super) struct SessionPayload {
    #[serde(rename = "type")]
    pub(super) payload_type: String,
    pub(super) rate_limits: Option<SessionRateLimits>,
}

#[derive(Deserialize)]
pub(super) struct SessionRateLimits {
    pub(super) primary: Option<SessionRateLimitWindow>,
    pub(super) secondary: Option<SessionRateLimitWindow>,
}

#[derive(Deserialize)]
pub(super) struct SessionRateLimitWindow {
    pub(super) used_percent: Option<FlexibleNumber>,
    pub(super) limit_window_seconds: Option<FlexibleNumber>,
    pub(super) window_minutes: Option<FlexibleNumber>,
    pub(super) reset_at: Option<FlexibleNumber>,
    pub(super) resets_at: Option<FlexibleNumber>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum FlexibleNumber {
    Number(f64),
    String(String),
}

impl FlexibleNumber {
    pub(super) fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(*value),
            Self::String(value) => value.parse::<f64>().ok(),
        }
    }
}
