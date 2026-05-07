use super::io::read_config_lines;
use serde_json::{Map, Value};

pub(super) fn find_root_table_index(lines: &[String]) -> Option<usize> {
    lines.iter().position(|line| {
        let normalized = line.trim();
        normalized.starts_with('[') && normalized.ends_with(']')
    })
}

pub(super) fn root_assignment(line: &str) -> Option<(String, String)> {
    let normalized = line.trim();
    if normalized.is_empty() || normalized.starts_with('#') || normalized.starts_with('[') {
        return None;
    }
    let (key, value) = normalized.split_once('=')?;
    let key = key.trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
    {
        return None;
    }
    Some((key.to_string(), value.trim().to_string()))
}

fn parse_toml_value(raw_value: &str) -> Value {
    let raw = raw_value.trim();
    if raw == "true" {
        return Value::Bool(true);
    }
    if raw == "false" {
        return Value::Bool(false);
    }
    if raw.len() >= 2
        && ((raw.starts_with('"') && raw.ends_with('"'))
            || (raw.starts_with('\'') && raw.ends_with('\'')))
    {
        return Value::String(
            raw[1..raw.len() - 1]
                .replace("\\\"", "\"")
                .replace("\\\\", "\\"),
        );
    }
    Value::String(raw.to_string())
}

pub(crate) fn read_root_config() -> Result<Map<String, Value>, String> {
    let lines = read_config_lines()?;
    let end = find_root_table_index(&lines).unwrap_or(lines.len());
    let mut config = Map::new();
    for line in lines.iter().take(end) {
        if let Some((key, value)) = root_assignment(line) {
            config.insert(key, parse_toml_value(&value));
        }
    }
    Ok(config)
}

pub(crate) fn read_table_config(table_name: &str) -> Result<Map<String, Value>, String> {
    let lines = read_config_lines()?;
    let header = format!("[{table_name}]");
    let Some(start) = lines.iter().position(|line| line.trim() == header) else {
        return Ok(Map::new());
    };

    let mut config = Map::new();
    for line in lines.iter().skip(start + 1) {
        let normalized = line.trim();
        if normalized.starts_with('[') && normalized.ends_with(']') {
            break;
        }
        if let Some((key, value)) = root_assignment(line) {
            config.insert(key, parse_toml_value(&value));
        }
    }
    Ok(config)
}
