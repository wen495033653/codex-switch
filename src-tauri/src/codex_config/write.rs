use super::{
    io::{read_config_lines, write_config_lines},
    parse::{find_root_table_index, root_assignment},
};
use serde_json::Value;

fn remove_table_lines(table_name: &str) -> Result<Vec<String>, String> {
    let mut lines = read_config_lines()?;
    let header = format!("[{table_name}]");
    let Some(start) = lines.iter().position(|line| line.trim() == header) else {
        return Ok(lines);
    };
    let mut end = lines.len();
    for (index, line) in lines.iter().enumerate().skip(start + 1) {
        let normalized = line.trim();
        if normalized.starts_with('[') && normalized.ends_with(']') {
            end = index;
            break;
        }
    }
    lines.splice(start..end, std::iter::empty());
    while start < lines.len() && start > 0 && lines[start].is_empty() && lines[start - 1].is_empty()
    {
        lines.remove(start);
    }
    Ok(lines)
}

fn format_toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn format_toml_value(value: &Value) -> String {
    match value {
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::String(text) => format_toml_string(text),
        other => format_toml_string(&other.to_string()),
    }
}

pub(crate) fn set_table_config(table_name: &str, values: Vec<(&str, Value)>) -> Result<(), String> {
    let mut lines = remove_table_lines(table_name)?;
    let insert_at = find_root_table_index(&lines).unwrap_or(lines.len());
    let mut table_lines = Vec::new();
    if insert_at > 0
        && lines
            .get(insert_at - 1)
            .is_some_and(|line| !line.trim().is_empty())
    {
        table_lines.push(String::new());
    }
    table_lines.push(format!("[{table_name}]"));
    for (key, value) in values {
        table_lines.push(format!("{key} = {}", format_toml_value(&value)));
    }
    table_lines.push(String::new());
    lines.splice(insert_at..insert_at, table_lines);
    write_config_lines(&lines)
}

pub(crate) fn remove_table_config(table_name: &str) -> Result<(), String> {
    let lines = remove_table_lines(table_name)?;
    write_config_lines(&lines)
}

pub(crate) fn set_config_values(values: Vec<(&str, String)>) -> Result<(), String> {
    let lines = read_config_lines()?;
    let root_end = find_root_table_index(&lines).unwrap_or(lines.len());
    let mut pending: Vec<(String, String)> = values
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
    let mut next_lines = Vec::with_capacity(lines.len() + pending.len() + 2);

    for (index, line) in lines.iter().enumerate() {
        if index < root_end {
            if let Some((key, _value)) = root_assignment(line) {
                if let Some(pending_index) = pending
                    .iter()
                    .position(|(pending_key, _)| pending_key == &key)
                {
                    let (_pending_key, pending_value) = pending.remove(pending_index);
                    next_lines.push(format!("{key} = {}", format_toml_string(&pending_value)));
                    continue;
                }
            }
        }
        next_lines.push(line.clone());
    }

    if !pending.is_empty() {
        let mut insert_lines: Vec<String> = pending
            .into_iter()
            .map(|(key, value)| format!("{key} = {}", format_toml_string(&value)))
            .collect();
        let insert_at = root_end;
        if insert_at > 0
            && next_lines
                .get(insert_at - 1)
                .is_some_and(|line| !line.trim().is_empty())
        {
            insert_lines.insert(0, String::new());
        }
        if insert_at < next_lines.len()
            && !insert_lines.last().unwrap_or(&String::new()).is_empty()
            && next_lines
                .get(insert_at)
                .is_some_and(|line| !line.trim().is_empty())
        {
            insert_lines.push(String::new());
        }
        next_lines.splice(insert_at..insert_at, insert_lines);
    }
    write_config_lines(&next_lines)
}

pub(crate) fn remove_config_values(keys: &[&str]) -> Result<(), String> {
    let lines = read_config_lines()?;
    let root_end = find_root_table_index(&lines).unwrap_or(lines.len());
    let next_lines: Vec<String> = lines
        .into_iter()
        .enumerate()
        .filter_map(|(index, line)| {
            if index < root_end {
                if let Some((key, _value)) = root_assignment(&line) {
                    if keys.iter().any(|target| *target == key) {
                        return None;
                    }
                }
            }
            Some(line)
        })
        .collect();
    write_config_lines(&next_lines)
}
