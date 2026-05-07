use crate::{
    api_config::API_PROVIDER_ID,
    codex_config::{
        read_root_config, read_table_config, remove_config_values, remove_table_config,
        set_config_values, set_table_config,
    },
    json_util::{raw_string_field, string_field},
    paths::auth_path,
    time_util::now_string,
};
use serde_json::{json, Map, Value};

const STORE_VERSION: i64 = 3;
const OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OAUTH_TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const OAUTH_AUTHORIZE_ENDPOINT: &str = "https://auth.openai.com/oauth/authorize";
const OAUTH_SCOPE: &str = "openid profile email offline_access";
const CHATGPT_USAGE_ENDPOINT: &str = "https://chatgpt.com/backend-api/wham/usage";

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

pub(crate) use account_builders::*;
pub(crate) use api_mode::*;
pub(crate) use auth_file::*;
pub(crate) use import_export::*;
pub(crate) use oauth_tokens::*;
pub(crate) use store::*;
pub(crate) use usage::*;
