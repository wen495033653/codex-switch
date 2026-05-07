use super::persistence::read_store_with_active_sync;
use crate::accounts::get_codex_state_value;
use serde_json::{Map, Value};

pub(crate) fn store_payload(message: Option<&str>) -> Result<Value, String> {
    let store = read_store_with_active_sync()?;
    Ok(store_payload_from_store(store, message))
}

pub(crate) fn store_payload_from_store(store: Value, message: Option<&str>) -> Value {
    let mut out = Map::new();
    out.insert("ok".to_string(), Value::Bool(true));
    if let Some(text) = message {
        out.insert("message".to_string(), Value::String(text.to_string()));
    }
    out.insert("codex_state".to_string(), get_codex_state_value());
    out.insert("store".to_string(), store);
    Value::Object(out)
}
