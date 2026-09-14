mod client;
mod state;

pub(crate) use client::{get_subscription, get_usage, parse_endpoint_error};
pub(crate) use state::{
    build_error_state, error_state_is_auth_rejected, normalize_custom, set_auth_state,
    set_subscription_state, set_usage_result, set_usage_state,
};

/// Requests a user is waiting on: manual refresh and the sync right after an import.
pub(crate) const INTERACTIVE_REQUEST_TIMEOUT_MS: u64 = 10_000;
/// Requests made by background workers: refresh-all and batch imports.
pub(crate) const BACKGROUND_REQUEST_TIMEOUT_MS: u64 = 30_000;
