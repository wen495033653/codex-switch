use serde_json::Value;

const STORE_VERSION: i64 = 3;
const OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OAUTH_TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const OAUTH_AUTHORIZE_ENDPOINT: &str = "https://auth.openai.com/oauth/authorize";
const OAUTH_SCOPE: &str = "openid profile email offline_access";

pub(crate) struct ImportTokenResult {
    pub(crate) account: Option<Value>,
    pub(crate) usage_ok: bool,
}

mod account_builders;
mod api_mode;
mod auth_file;
mod import_export;
mod oauth_tokens;
mod store;
mod usage;

pub(crate) use account_builders::{
    account_from_exchange, account_from_exchange_preserve_usage, account_from_exchange_syncing,
};
pub(crate) use api_mode::{
    get_codex_state_value, read_api_key_from_auth, read_api_key_from_provider_config,
    restore_api_mode_if_selected, set_api_mode, set_subscription_mode,
};
pub(crate) use auth_file::{auth_to_account, read_auth_value, write_account_auth};
pub(crate) use import_export::{
    build_export_account_item, extract_refresh_tokens_from_data,
    import_accounts_from_refresh_tokens, local_date_for_filename,
};
#[cfg(test)]
pub(crate) use oauth_tokens::decode_jwt_payload;
pub(crate) use oauth_tokens::{
    build_oauth_auth_url, exchange_oauth_code, exchange_refresh_token, generate_pkce,
    random_urlsafe,
};
pub(crate) use store::{
    access_token_from_account, account_id_from_account, account_with_custom, add_account_to_store,
    auth_error_is_login_expired, find_store_account, lookup_store_account, mark_account_auth_error,
    mark_store_account_used, normalize_tokens, profile_id_from_account,
    profile_id_from_tokens_value, read_store_value, read_store_with_active_sync,
    refresh_token_from_account, remove_store_account, sort_accounts_by_last_used, store_payload,
    store_payload_from_store, sync_auth_file_if_active, write_store_value,
};
pub(crate) use usage::{
    build_error_state, error_state_is_auth_rejected, get_subscription, get_usage, normalize_custom,
    set_auth_state, set_subscription_state, set_usage_result, BACKGROUND_REQUEST_TIMEOUT_MS,
    INTERACTIVE_REQUEST_TIMEOUT_MS,
};
