use super::{
    auth_file::{read_auth_value_if_exists, write_api_auth},
    store::profile_id_from_tokens_value,
};
use crate::{
    api_config::normalize_api_base_url,
    app_log::log_event_once,
    json_util::string_field,
    settings::{default_api_mode, read_settings_value},
};
use crate::{
    api_config::API_PROVIDER_ID,
    codex_config::{
        read_config_snapshot, remove_config_values, remove_table_config, set_config_values,
        set_table_config, ConfigSnapshot,
    },
    json_util::raw_string_field,
};
use serde_json::Value;
use serde_json::{json, Map};

const API_WIRE_RESPONSES: &str = "responses";

impl ApiModeProfile {
    pub(super) fn api_key_or_auth_file(&self) -> String {
        if self.api_key.is_empty() {
            let provider_key = read_api_key_from_provider_config().trim().to_string();
            if provider_key.is_empty() {
                read_api_key_from_auth()
            } else {
                provider_key
            }
        } else {
            self.api_key.clone()
        }
    }
}

pub(crate) fn set_subscription_mode() -> Result<(), String> {
    set_config_values(vec![("cli_auth_credentials_store", "file".to_string())])?;
    remove_config_values(&[
        "preferred_auth_method",
        "forced_login_method",
        "openai_base_url",
        "model_provider",
    ])?;
    remove_table_config(&format!("model_providers.{API_PROVIDER_ID}"))?;
    Ok(())
}

/// auth.json for read-only state checks. A missing file is the normal "not logged in" state. A
/// read or parse failure is logged with its reason and then read as empty, as before: the
/// callers (store payloads, `store-updated` events, mode checks) take a state value, not an
/// error.
fn read_auth_for_state() -> Value {
    match read_auth_value_if_exists() {
        Ok(Some(auth)) => auth,
        Ok(None) => json!({}),
        Err(err) => {
            log_codex_state_read_error("auth.json", "按空内容处理", err);
            json!({})
        }
    }
}

/// config.toml for read-only state checks; a failure is logged and read as an empty config.
fn read_config_for_state() -> Option<ConfigSnapshot> {
    match read_config_snapshot() {
        Ok(config) => Some(config),
        Err(err) => {
            log_codex_state_read_error("config.toml", "按空配置处理", err);
            None
        }
    }
}

/// The state is read on every store update, so one lasting cause is recorded once per run.
fn log_codex_state_read_error(file: &str, handling: &str, error: String) {
    log_event_once(
        "codex_state_read_error",
        json!({ "file": file, "handling": handling, "error": error }),
    );
}

fn trimmed_string(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

/// The `[model_providers.<model_provider>]` table of the selected provider, empty without one.
fn selected_provider_config(
    config: &ConfigSnapshot,
    root_config: &Map<String, Value>,
) -> Map<String, Value> {
    match root_config.get("model_provider").and_then(Value::as_str) {
        Some(model_provider) if !model_provider.is_empty() => {
            config.table(&format!("model_providers.{model_provider}"))
        }
        _ => Map::new(),
    }
}

pub(crate) fn read_api_key_from_auth() -> String {
    trimmed_string(read_auth_for_state().get("OPENAI_API_KEY"))
}

pub(crate) fn read_api_key_from_provider_config() -> String {
    let Some(config) = read_config_for_state() else {
        return String::new();
    };
    let provider_config = selected_provider_config(&config, &config.root());
    trimmed_string(provider_config.get("experimental_bearer_token"))
}

fn api_mode_provider_config(profile: &ApiModeProfile) -> Vec<(&'static str, Value)> {
    vec![
        ("name", Value::String(API_PROVIDER_ID.to_string())),
        ("base_url", Value::String(profile.base_url.clone())),
        ("wire_api", Value::String(API_WIRE_RESPONSES.to_string())),
        ("supports_websockets", Value::Bool(false)),
        ("requires_openai_auth", Value::Bool(true)),
    ]
}

pub(crate) fn set_api_mode(profile: &Value) -> Result<(), String> {
    let profile = ApiModeProfile::from_value(profile)?;
    let api_key = profile.api_key_or_auth_file().trim().to_string();
    write_api_auth(&api_key)?;
    set_config_values(vec![
        ("model_provider", API_PROVIDER_ID.to_string()),
        ("cli_auth_credentials_store", "file".to_string()),
    ])?;
    remove_config_values(&[
        "preferred_auth_method",
        "forced_login_method",
        "openai_base_url",
    ])?;
    set_table_config(
        &format!("model_providers.{API_PROVIDER_ID}"),
        api_mode_provider_config(&profile),
    )?;
    Ok(())
}

pub(crate) fn restore_api_mode_if_selected() -> Result<bool, String> {
    let settings = read_settings_value()?;
    if raw_string_field(&settings, "codex_active_mode") != "api" {
        return Ok(false);
    }

    let profile = settings
        .get("api_mode")
        .cloned()
        .unwrap_or_else(default_api_mode);
    if string_field(&profile, "base_url").is_empty() {
        return Ok(false);
    }

    let state = get_codex_state_value();
    if raw_string_field(&state, "mode") == "api"
        && raw_string_field(&state, "model_provider") == API_PROVIDER_ID
    {
        return Ok(false);
    }

    set_api_mode(&profile)?;
    Ok(true)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ApiModeProfile {
    pub(super) base_url: String,
    pub(super) api_key: String,
}

impl ApiModeProfile {
    pub(super) fn from_value(value: &Value) -> Result<Self, String> {
        Ok(Self {
            base_url: normalize_api_base_url(&string_field(value, "base_url"))?,
            api_key: string_field(value, "api_key"),
        })
    }
}

/// Reads auth.json and config.toml once each. It runs on every `store-updated` event, so the
/// previous two reads of auth.json and up to four of config.toml per call added up.
pub(crate) fn get_codex_state_value() -> Value {
    let auth = read_auth_for_state();
    let (root_config, provider_config) = match read_config_for_state() {
        Some(config) => {
            let root_config = config.root();
            let provider_config = selected_provider_config(&config, &root_config);
            (root_config, provider_config)
        }
        None => (Map::new(), Map::new()),
    };
    codex_state_from(&auth, &root_config, &provider_config)
}

fn codex_state_from(
    auth: &Value,
    root_config: &Map<String, Value>,
    provider_config: &Map<String, Value>,
) -> Value {
    let auth_mode = raw_string_field(auth, "auth_mode");
    let preferred_auth_method = root_config
        .get("preferred_auth_method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let forced_login_method = root_config
        .get("forced_login_method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let model_provider = root_config
        .get("model_provider")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let provider_name = provider_config
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let provider_base_url = provider_config
        .get("base_url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let wire_api = provider_config
        .get("wire_api")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let supports_websockets = provider_config
        .get("supports_websockets")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let openai_base_url = root_config
        .get("openai_base_url")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(&provider_base_url)
        .to_string();
    let api_key_present = !trimmed_string(auth.get("OPENAI_API_KEY")).is_empty()
        || !trimmed_string(provider_config.get("experimental_bearer_token")).is_empty();
    let account_id = auth
        .get("tokens")
        .and_then(|tokens| tokens.get("account_id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let profile_id = profile_id_from_tokens_value(auth.get("tokens")).unwrap_or_default();

    let api_provider_ready = model_provider == API_PROVIDER_ID && !provider_base_url.is_empty();
    let api_credentials_ready = auth_mode == "apikey"
        || api_key_present
        || preferred_auth_method == "api"
        || forced_login_method == "api";

    let mode = if api_credentials_ready && api_provider_ready {
        "api"
    } else if auth_mode == "chatgpt" || !account_id.is_empty() {
        "chatgpt"
    } else {
        "unknown"
    };

    json!({
        "mode": mode,
        "auth_mode": auth_mode,
        "preferred_auth_method": preferred_auth_method,
        "forced_login_method": forced_login_method,
        "model_provider": model_provider,
        "provider_name": provider_name,
        "wire_api": wire_api,
        "supports_websockets": supports_websockets,
        "openai_base_url": openai_base_url,
        "api_key_present": api_key_present,
        "api_provider_ready": api_provider_ready,
        "account_id": account_id,
        "profile_id": profile_id
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    #[test]
    fn api_mode_provider_config_uses_responses_wire_api_and_openai_auth() {
        let profile = ApiModeProfile {
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: "sk-test".to_string(),
        };

        let config: Map<String, Value> = api_mode_provider_config(&profile)
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect();

        assert_eq!(config.get("name").and_then(Value::as_str), Some("api"));
        assert_eq!(
            config.get("wire_api").and_then(Value::as_str),
            Some(API_WIRE_RESPONSES)
        );
        assert_eq!(
            config.get("supports_websockets").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            config.get("requires_openai_auth").and_then(Value::as_bool),
            Some(true)
        );
    }

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn codex_state_reports_api_mode_from_auth_key_and_provider_table() {
        let state = codex_state_from(
            &json!({ "auth_mode": "apikey", "OPENAI_API_KEY": " placeholder-key " }),
            &map(json!({ "model_provider": "api" })),
            &map(json!({
                "name": "api",
                "base_url": "https://example.test/v1",
                "wire_api": "responses",
                "supports_websockets": false
            })),
        );

        assert_eq!(
            state,
            json!({
                "mode": "api",
                "auth_mode": "apikey",
                "preferred_auth_method": "",
                "forced_login_method": "",
                "model_provider": "api",
                "provider_name": "api",
                "wire_api": "responses",
                "supports_websockets": false,
                "openai_base_url": "https://example.test/v1",
                "api_key_present": true,
                "api_provider_ready": true,
                "account_id": "",
                "profile_id": ""
            })
        );
    }

    #[test]
    fn codex_state_counts_a_provider_bearer_token_as_api_key() {
        let state = codex_state_from(
            &json!({}),
            &map(json!({
                "model_provider": "api",
                "openai_base_url": "https://override.test/v1"
            })),
            &map(json!({
                "base_url": "https://example.test/v1",
                "experimental_bearer_token": "placeholder-token"
            })),
        );

        assert_eq!(state["mode"], "api");
        assert_eq!(state["api_key_present"], true);
        assert_eq!(state["openai_base_url"], "https://override.test/v1");
    }

    #[test]
    fn codex_state_reports_chatgpt_and_unknown_modes() {
        let chatgpt = codex_state_from(
            &json!({ "auth_mode": "chatgpt", "tokens": { "account_id": "account-fixture" } }),
            &Map::new(),
            &Map::new(),
        );
        assert_eq!(chatgpt["mode"], "chatgpt");
        assert_eq!(chatgpt["account_id"], "account-fixture");
        assert_eq!(chatgpt["api_key_present"], false);

        // A blank key does not count, and a provider without base_url is not ready.
        let unknown = codex_state_from(
            &json!({ "OPENAI_API_KEY": "  " }),
            &map(json!({ "model_provider": "api" })),
            &Map::new(),
        );
        assert_eq!(unknown["mode"], "unknown");
        assert_eq!(unknown["api_key_present"], false);
        assert_eq!(unknown["api_provider_ready"], false);
    }
}
