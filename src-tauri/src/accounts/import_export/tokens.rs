use crate::json_util::string_field;
use serde_json::Value;
use std::collections::HashSet;

fn push_refresh_token_from_value(out: &mut Vec<String>, value: &Value) {
    if let Some(text) = value.as_str() {
        let token = text.trim();
        if !token.is_empty() {
            out.push(token.to_string());
        }
        return;
    }

    let token = string_field(value, "refresh_token");
    if !token.is_empty() {
        out.push(token);
    }
}

pub(crate) fn extract_refresh_tokens_from_data(data: &Value) -> Vec<String> {
    let mut raw_tokens = Vec::new();

    if let Some(items) = data.as_array() {
        for item in items {
            push_refresh_token_from_value(&mut raw_tokens, item);
        }
    } else if data.is_object() {
        if let Some(items) = data.get("refresh_tokens").and_then(Value::as_array) {
            for item in items {
                push_refresh_token_from_value(&mut raw_tokens, item);
            }
        }
        if let Some(items) = data.get("tokens").and_then(Value::as_array) {
            for item in items {
                push_refresh_token_from_value(&mut raw_tokens, item);
            }
        }
        if let Some(items) = data.get("accounts").and_then(Value::as_array) {
            for item in items {
                if let Some(tokens) = item.get("tokens") {
                    push_refresh_token_from_value(&mut raw_tokens, tokens);
                } else {
                    push_refresh_token_from_value(&mut raw_tokens, item);
                }
            }
        }
    }

    let mut seen = HashSet::new();
    raw_tokens
        .into_iter()
        .filter_map(|token| {
            let token = token.trim().to_string();
            if token.is_empty() || !seen.insert(token.clone()) {
                return None;
            }
            Some(token)
        })
        .collect()
}
