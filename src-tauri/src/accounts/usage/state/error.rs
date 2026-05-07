use super::number::to_number;
use crate::{json_util::raw_string_field, time_util::now_string};
use serde_json::{json, Map, Value};

pub(crate) fn build_error_state(
    message: &str,
    code: &str,
    raw_message: &str,
    status: u16,
    path: &str,
) -> Value {
    let mut out = Map::new();
    out.insert("code".to_string(), Value::String(code.to_string()));
    out.insert("message".to_string(), Value::String(message.to_string()));
    out.insert("time".to_string(), Value::String(now_string()));
    if !raw_message.is_empty() && raw_message != message {
        out.insert(
            "raw_message".to_string(),
            Value::String(raw_message.to_string()),
        );
    }
    if status > 0 {
        out.insert("status".to_string(), json!(status));
    }
    if !path.is_empty() {
        out.insert("path".to_string(), Value::String(path.to_string()));
    }
    Value::Object(out)
}

pub(super) fn normalize_error_state(value: Option<&Value>) -> Value {
    let raw = value.unwrap_or(&Value::Null);
    if !raw.is_object() {
        return Value::Null;
    }

    let message = raw_string_field(raw, "message");
    let code = raw_string_field(raw, "code");
    let time = raw_string_field(raw, "time");
    let raw_message = raw_string_field(raw, "raw_message");
    let path = raw_string_field(raw, "path");
    let status = to_number(raw.get("status"));
    if message.is_empty()
        && code.is_empty()
        && raw_message.is_empty()
        && path.is_empty()
        && status <= 0.0
    {
        return Value::Null;
    }

    let mut out = Map::new();
    out.insert("code".to_string(), Value::String(code));
    out.insert("message".to_string(), Value::String(message));
    out.insert("time".to_string(), Value::String(time));
    if !raw_message.is_empty() {
        out.insert("raw_message".to_string(), Value::String(raw_message));
    }
    if !path.is_empty() {
        out.insert("path".to_string(), Value::String(path));
    }
    if status > 0.0 {
        out.insert("status".to_string(), json!(status));
    }
    Value::Object(out)
}
