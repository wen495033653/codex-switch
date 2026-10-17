use super::{
    io::update_config_lines,
    parse::{find_root_table_index, root_assignment, table_header},
};
use serde_json::Value;

const FEATURES_TABLE: &str = "features";
const REMOTE_CONTROL_CONFIG_KEY: &str = "remote_control";
const LEGACY_REMOTE_CONNECTIONS_CONFIG_KEY: &str = "remote_connections";
const REMOTE_CONTROL_CONFIG_KEYS: &[&str] = &[
    REMOTE_CONTROL_CONFIG_KEY,
    LEGACY_REMOTE_CONNECTIONS_CONFIG_KEY,
];

fn without_table(lines: &[String], table_name: &str) -> Vec<String> {
    let mut lines = lines.to_vec();
    let header = format!("[{table_name}]");
    let Some((start, end)) = table_bounds(&lines, &header) else {
        return lines;
    };

    lines.splice(start..end, std::iter::empty());
    while start < lines.len() && start > 0 && lines[start].is_empty() && lines[start - 1].is_empty()
    {
        lines.remove(start);
    }
    lines
}

/// `header` is the full `[table]` line. Returns the header index and the index of the next table
/// header (or the line count).
pub(crate) fn table_bounds(lines: &[String], header: &str) -> Option<(usize, usize)> {
    let start = lines
        .iter()
        .position(|line| table_header(line) == Some(header))?;
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, line)| table_header(line).is_some())
        .map_or(lines.len(), |(index, _)| index);
    Some((start, end))
}

pub(crate) fn format_toml_string(value: &str) -> String {
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

/// Replaces the table with `values`, placed before the first other table.
fn with_table(lines: &[String], table_name: &str, values: Vec<(&str, Value)>) -> Vec<String> {
    let mut lines = without_table(lines, table_name);
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
    lines
}

pub(crate) fn set_table_config(table_name: &str, values: Vec<(&str, Value)>) -> Result<(), String> {
    update_config_lines(|lines| with_table(lines, table_name, values)).map(|_| ())
}

pub(crate) fn remove_table_config(table_name: &str) -> Result<(), String> {
    update_config_lines(|lines| without_table(lines, table_name)).map(|_| ())
}

fn remove_table_config_entries(lines: &[String], table_name: &str, keys: &[&str]) -> Vec<String> {
    let header = format!("[{table_name}]");
    let Some((start, end)) = table_bounds(lines, &header) else {
        return lines.to_vec();
    };

    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            if index > start && index < end {
                if let Some((key, _value)) = root_assignment(line) {
                    if keys.iter().any(|target| *target == key) {
                        return None;
                    }
                }
            }
            Some(line.clone())
        })
        .collect()
}

fn remove_table_config_values(table_name: &str, keys: &[&str]) -> Result<bool, String> {
    update_config_lines(|lines| remove_table_config_entries(lines, table_name, keys))
}

fn upsert_root_config_entries(lines: &[String], values: Vec<(String, Value)>) -> Vec<String> {
    let root_end = find_root_table_index(lines).unwrap_or(lines.len());
    let mut pending = values;
    let mut next_lines = Vec::with_capacity(lines.len() + pending.len() + 2);

    for (index, line) in lines.iter().enumerate() {
        if index < root_end {
            if let Some((key, _value)) = root_assignment(line) {
                if let Some(pending_index) = pending
                    .iter()
                    .position(|(pending_key, _)| pending_key == &key)
                {
                    let (_pending_key, pending_value) = pending.remove(pending_index);
                    next_lines.push(format!("{key} = {}", format_toml_value(&pending_value)));
                    continue;
                }
            }
        }
        next_lines.push(line.clone());
    }

    if !pending.is_empty() {
        let mut insert_lines: Vec<String> = pending
            .into_iter()
            .map(|(key, value)| format!("{key} = {}", format_toml_value(&value)))
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
    next_lines
}

fn set_config_entries(values: Vec<(String, Value)>) -> Result<(), String> {
    update_config_lines(|lines| upsert_root_config_entries(lines, values)).map(|_| ())
}

pub(crate) fn set_config_values(values: Vec<(&str, String)>) -> Result<(), String> {
    let values = values
        .into_iter()
        .map(|(key, value)| (key.to_string(), Value::String(value)))
        .collect();
    set_config_entries(values)
}

pub(crate) fn remove_remote_control_config() -> Result<bool, String> {
    remove_table_config_values(FEATURES_TABLE, REMOTE_CONTROL_CONFIG_KEYS)
}

fn without_root_values(lines: &[String], keys: &[&str]) -> Vec<String> {
    let root_end = find_root_table_index(lines).unwrap_or(lines.len());
    lines
        .iter()
        .enumerate()
        .filter(|(index, line)| {
            *index >= root_end
                || root_assignment(line).is_none_or(|(key, _value)| !keys.contains(&key.as_str()))
        })
        .map(|(_, line)| line.clone())
        .collect()
}

pub(crate) fn remove_config_values(keys: &[&str]) -> Result<(), String> {
    update_config_lines(|lines| without_root_values(lines, keys)).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn headers_with_a_trailing_comment_still_end_the_root_and_name_their_table() {
        let input = lines(&[
            "model = \"gpt-5\"",
            "[features] # experimental",
            "web_search_request = true",
            "[model_providers.api] # relay",
            "base_url = \"https://old.example/v1\"",
        ]);

        let rooted = upsert_root_config_entries(
            &input,
            vec![("model_provider".to_string(), Value::String("api".into()))],
        );
        let position = |target: &str| rooted.iter().position(|line| line == target).unwrap();
        assert!(position("model_provider = \"api\"") < position("[features] # experimental"));

        let replaced = with_table(
            &input,
            "model_providers.api",
            vec![("base_url", Value::String("https://new.example/v1".into()))],
        );
        assert_eq!(
            replaced
                .iter()
                .filter(|line| line.starts_with("[model_providers.api]"))
                .count(),
            1
        );
        assert!(replaced.contains(&"base_url = \"https://new.example/v1\"".to_string()));
        assert!(!replaced.contains(&"base_url = \"https://old.example/v1\"".to_string()));
    }

    #[test]
    fn writing_the_same_values_again_changes_no_line() {
        let start = lines(&["model = \"gpt-5\"", "", "[features]", "memories = true"]);
        let values = || {
            vec![
                ("name", Value::String("API".into())),
                ("base_url", Value::String("https://relay.example/v1".into())),
            ]
        };
        let once = with_table(&start, "model_providers.api", values());
        let twice = with_table(&once, "model_providers.api", values());
        assert_eq!(twice, once);

        let rooted = upsert_root_config_entries(
            &once,
            vec![("model_provider".to_string(), Value::String("api".into()))],
        );
        let rooted_again = upsert_root_config_entries(
            &rooted,
            vec![("model_provider".to_string(), Value::String("api".into()))],
        );
        assert_eq!(rooted_again, rooted);
        assert_eq!(without_root_values(&once, &["openai_base_url"]), once);
    }

    #[test]
    fn upsert_root_config_entries_writes_bool_without_quotes() {
        let output = upsert_root_config_entries(
            &lines(&["remote_control = false", "model_provider = \"api\""]),
            vec![(REMOTE_CONTROL_CONFIG_KEY.to_string(), Value::Bool(true))],
        );

        assert_eq!(
            output,
            lines(&["remote_control = true", "model_provider = \"api\""])
        );
    }

    #[test]
    fn upsert_root_config_entries_inserts_root_value_before_tables() {
        let output = upsert_root_config_entries(
            &lines(&[
                "model_provider = \"api\"",
                "",
                "[model_providers.api]",
                "name = \"API\"",
            ]),
            vec![(REMOTE_CONTROL_CONFIG_KEY.to_string(), Value::Bool(true))],
        );

        assert_eq!(
            output,
            lines(&[
                "model_provider = \"api\"",
                "",
                "remote_control = true",
                "",
                "[model_providers.api]",
                "name = \"API\"",
            ])
        );
    }

    #[test]
    fn upsert_root_config_entries_replaces_model_instructions_file() {
        let output = upsert_root_config_entries(
            &lines(&[
                "model_provider = \"api\"",
                "model_instructions_file = \"./old.md\"",
                "",
                "[features]",
                "remote_control = true",
            ]),
            vec![(
                "model_instructions_file".to_string(),
                Value::String("C:/CodexSwitch/gpt-unrestricted.md".to_string()),
            )],
        );

        assert_eq!(
            output,
            lines(&[
                "model_provider = \"api\"",
                "model_instructions_file = \"C:/CodexSwitch/gpt-unrestricted.md\"",
                "",
                "[features]",
                "remote_control = true",
            ])
        );
    }

    #[test]
    fn remove_table_config_entries_removes_remote_control_keys_only_from_features() {
        let output = remove_table_config_entries(
            &lines(&[
                "remote_connections = true",
                "",
                "[features]",
                "memories = true",
                "remote_control = true",
                "remote_connections = true",
                "",
                "[model_providers.api]",
                "remote_control = true",
            ]),
            FEATURES_TABLE,
            REMOTE_CONTROL_CONFIG_KEYS,
        );

        assert_eq!(
            output,
            lines(&[
                "remote_connections = true",
                "",
                "[features]",
                "memories = true",
                "",
                "[model_providers.api]",
                "remote_control = true",
            ])
        );
    }

    #[test]
    fn remove_table_config_entries_preserves_file_without_features_table() {
        let input = lines(&["model_provider = \"api\""]);
        let output =
            remove_table_config_entries(&input, FEATURES_TABLE, REMOTE_CONTROL_CONFIG_KEYS);

        assert_eq!(output, input);
    }
}
