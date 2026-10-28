use crate::{
    api_config::API_PROVIDER_ID,
    codex_config::{find_root_table_index, format_toml_string, root_assignment, table_bounds},
};
use std::{fs, path::Path};

const API_WIRE_RESPONSES: &str = "responses";

const WINDOWS_SANDBOX_MODE: &str = "elevated";

pub(super) enum InstanceConfig {
    Subscription,
    Api { base_url: String },
}

pub(super) fn sync_instance_config(
    config_path: &Path,
    config: &InstanceConfig,
    model_instructions_file: Option<&str>,
) -> Result<(), String> {
    let lines = read_instance_config_lines(config_path)?;
    let next_lines = merge_instance_config_lines(&lines, config, model_instructions_file);
    write_instance_config_lines(config_path, &next_lines)
}

fn read_instance_config_lines(config_path: &Path) -> Result<Vec<String>, String> {
    if !config_path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(config_path)
        .map_err(|err| format!("读取实例 config.toml 失败 {}: {err}", config_path.display()))?;
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    Ok(raw.lines().map(|line| line.to_string()).collect())
}

fn write_instance_config_lines(config_path: &Path, lines: &[String]) -> Result<(), String> {
    let mut raw = lines.join("\n");
    raw.push('\n');
    fs::write(config_path, raw)
        .map_err(|err| format!("写入实例 config.toml 失败 {}: {err}", config_path.display()))
}

fn merge_instance_config_lines(
    lines: &[String],
    config: &InstanceConfig,
    model_instructions_file: Option<&str>,
) -> Vec<String> {
    let api_provider_table = format!("model_providers.{API_PROVIDER_ID}");
    let mut next_lines = match config {
        InstanceConfig::Subscription => {
            let lines = remove_root_config_entries(
                lines,
                &[
                    "model_provider",
                    "preferred_auth_method",
                    "forced_login_method",
                    "openai_base_url",
                ],
            );
            let lines = upsert_root_config_entries(
                &lines,
                vec![("cli_auth_credentials_store", format_toml_string("file"))],
            );
            remove_table_lines(&lines, &api_provider_table)
        }
        InstanceConfig::Api { base_url } => {
            let lines = remove_root_config_entries(
                lines,
                &[
                    "preferred_auth_method",
                    "forced_login_method",
                    "openai_base_url",
                ],
            );
            let lines = upsert_root_config_entries(
                &lines,
                vec![
                    ("model_provider", format_toml_string(API_PROVIDER_ID)),
                    ("cli_auth_credentials_store", format_toml_string("file")),
                ],
            );
            set_table_config_entries(
                &lines,
                &api_provider_table,
                vec![
                    ("name", format_toml_string(API_PROVIDER_ID)),
                    ("base_url", format_toml_string(base_url)),
                    ("wire_api", format_toml_string(API_WIRE_RESPONSES)),
                    ("supports_websockets", "false".to_string()),
                    ("requires_openai_auth", "true".to_string()),
                ],
            )
        }
    };

    next_lines = if let Some(model_instructions_file) = model_instructions_file {
        upsert_root_config_entries(
            &next_lines,
            vec![(
                "model_instructions_file",
                format_toml_string(model_instructions_file),
            )],
        )
    } else {
        remove_root_config_entries(&next_lines, &["model_instructions_file"])
    };
    next_lines = upsert_table_config_entries(
        &next_lines,
        "windows",
        vec![("sandbox", format_toml_string(WINDOWS_SANDBOX_MODE))],
    );
    normalize_blank_lines(&next_lines)
}

fn remove_root_config_entries(lines: &[String], keys: &[&str]) -> Vec<String> {
    let root_end = find_root_table_index(lines).unwrap_or(lines.len());
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            if index < root_end {
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

fn upsert_root_config_entries(lines: &[String], values: Vec<(&str, String)>) -> Vec<String> {
    let root_end = find_root_table_index(lines).unwrap_or(lines.len());
    let mut pending = values;
    let mut next_lines = Vec::with_capacity(lines.len() + pending.len() + 2);

    for (index, line) in lines.iter().enumerate() {
        if index < root_end {
            if let Some((key, _value)) = root_assignment(line) {
                if let Some(pending_index) = pending
                    .iter()
                    .position(|(pending_key, _)| *pending_key == key)
                {
                    let (_pending_key, pending_value) = pending.remove(pending_index);
                    next_lines.push(format!("{key} = {pending_value}"));
                    continue;
                }
            }
        }
        next_lines.push(line.clone());
    }

    if pending.is_empty() {
        return next_lines;
    }

    let mut insert_at = root_end;
    while insert_at > 0
        && next_lines
            .get(insert_at - 1)
            .is_some_and(|line| line.trim().is_empty())
    {
        insert_at -= 1;
    }
    let mut insert_lines: Vec<String> = pending
        .into_iter()
        .map(|(key, value)| format!("{key} = {value}"))
        .collect();
    if insert_at < next_lines.len()
        && next_lines
            .get(insert_at)
            .is_some_and(|line| !line.trim().is_empty())
    {
        insert_lines.push(String::new());
    }
    next_lines.splice(insert_at..insert_at, insert_lines);
    next_lines
}

fn remove_table_lines(lines: &[String], table_name: &str) -> Vec<String> {
    let Some((start, end)) = table_bounds(lines, &format!("[{table_name}]")) else {
        return lines.to_vec();
    };
    let mut next_lines = lines.to_vec();
    next_lines.splice(start..end, std::iter::empty());
    normalize_blank_lines(&next_lines)
}

fn set_table_config_entries(
    lines: &[String],
    table_name: &str,
    values: Vec<(&str, String)>,
) -> Vec<String> {
    let lines = remove_table_lines(lines, table_name);
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
    table_lines.extend(
        values
            .into_iter()
            .map(|(key, value)| format!("{key} = {value}")),
    );
    if insert_at < lines.len()
        && lines
            .get(insert_at)
            .is_some_and(|line| !line.trim().is_empty())
    {
        table_lines.push(String::new());
    }
    let mut next_lines = lines;
    next_lines.splice(insert_at..insert_at, table_lines);
    next_lines
}

fn upsert_table_config_entries(
    lines: &[String],
    table_name: &str,
    values: Vec<(&str, String)>,
) -> Vec<String> {
    let Some((start, end)) = table_bounds(lines, &format!("[{table_name}]")) else {
        return set_table_config_entries(lines, table_name, values);
    };
    let mut pending = values;
    let mut next_lines = Vec::with_capacity(lines.len() + pending.len());

    for (index, line) in lines.iter().enumerate() {
        if index > start && index < end {
            if let Some((key, _value)) = root_assignment(line) {
                if let Some(pending_index) = pending
                    .iter()
                    .position(|(pending_key, _)| *pending_key == key)
                {
                    let (_pending_key, pending_value) = pending.remove(pending_index);
                    next_lines.push(format!("{key} = {pending_value}"));
                    continue;
                }
            }
        }
        next_lines.push(line.clone());
    }

    if pending.is_empty() {
        return next_lines;
    }

    let mut insert_at = end;
    while insert_at > start + 1
        && next_lines
            .get(insert_at - 1)
            .is_some_and(|line| line.trim().is_empty())
    {
        insert_at -= 1;
    }
    let insert_lines: Vec<String> = pending
        .into_iter()
        .map(|(key, value)| format!("{key} = {value}"))
        .collect();
    next_lines.splice(insert_at..insert_at, insert_lines);
    next_lines
}

fn normalize_blank_lines(lines: &[String]) -> Vec<String> {
    let mut next_lines = Vec::with_capacity(lines.len());
    for line in lines {
        if line.trim().is_empty()
            && next_lines
                .last()
                .is_some_and(|previous: &String| previous.trim().is_empty())
        {
            continue;
        }
        next_lines.push(line.clone());
    }
    while next_lines.last().is_some_and(|line| line.trim().is_empty()) {
        next_lines.pop();
    }
    next_lines
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MODEL_INSTRUCTIONS_FILE: &str = "C:/CodexSwitch/gpt-unrestricted.md";

    const TEST_MODEL_INSTRUCTIONS_CONFIG_LINE: &str =
        "model_instructions_file = \"C:/CodexSwitch/gpt-unrestricted.md\"";

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    fn joined(lines: Vec<String>) -> String {
        lines.join("\n")
    }

    #[test]
    fn merge_api_config_preserves_app_tables_and_updates_provider() {
        let output = joined(merge_instance_config_lines(
            &lines(&[
                "model_provider = \"old\"",
                "cli_auth_credentials_store = \"keyring\"",
                "model_instructions_file = \"./old.md\"",
                "preferred_auth_method = \"chatgpt\"",
                "openai_base_url = \"https://old.example.com\"",
                "",
                "[model_providers.api]",
                "name = \"api\"",
                "base_url = \"https://old.example.com/v1\"",
                "wire_api = \"chat\"",
                "",
                "[plugins.\"browser@openai-bundled\"]",
                "enabled = true",
                "",
                "[features]",
                "js_repl = false",
            ]),
            &InstanceConfig::Api {
                base_url: "https://api.example.com/v1".to_string(),
            },
            Some(TEST_MODEL_INSTRUCTIONS_FILE),
        ));

        assert!(output.contains("model_provider = \"api\""));
        assert!(output.contains("cli_auth_credentials_store = \"file\""));
        assert!(output.contains(TEST_MODEL_INSTRUCTIONS_CONFIG_LINE));
        assert!(!output.contains("model_instructions_file = \"./old.md\""));
        assert!(!output.contains("preferred_auth_method"));
        assert!(!output.contains("openai_base_url"));
        assert!(output.contains("[windows]\nsandbox = \"elevated\""));
        assert!(output.contains("[model_providers.api]"));
        assert!(output.contains("base_url = \"https://api.example.com/v1\""));
        assert!(output.contains("wire_api = \"responses\""));
        assert!(output.contains("requires_openai_auth = true"));
        assert!(output.contains("[plugins.\"browser@openai-bundled\"]\nenabled = true"));
        assert!(output.contains("[features]\njs_repl = false"));
    }

    #[test]
    fn merge_subscription_config_preserves_app_tables_and_removes_api_provider() {
        let output = joined(merge_instance_config_lines(
            &lines(&[
                "model_provider = \"api\"",
                "cli_auth_credentials_store = \"file\"",
                "",
                "[windows]",
                "sandbox_private_desktop = false",
                "",
                "[model_providers.api]",
                "name = \"api\"",
                "base_url = \"https://api.example.com/v1\"",
                "",
                "[mcp_servers.node_repl]",
                "command = 'node_repl.exe'",
            ]),
            &InstanceConfig::Subscription,
            Some(TEST_MODEL_INSTRUCTIONS_FILE),
        ));

        assert!(!output.contains("model_provider = \"api\""));
        assert!(!output.contains("[model_providers.api]"));
        assert!(output.contains("cli_auth_credentials_store = \"file\""));
        assert!(output.contains(TEST_MODEL_INSTRUCTIONS_CONFIG_LINE));
        assert!(
            output.contains("[windows]\nsandbox_private_desktop = false\nsandbox = \"elevated\"")
        );
        assert!(output.contains("[mcp_servers.node_repl]\ncommand = 'node_repl.exe'"));
    }

    #[test]
    fn merge_instance_config_updates_existing_windows_sandbox_only() {
        let output = merge_instance_config_lines(
            &lines(&[
                "cli_auth_credentials_store = \"file\"",
                "",
                "[windows]",
                "sandbox = \"unelevated\"",
                "sandbox_private_desktop = false",
                "",
                "[plugins.\"chrome@openai-bundled\"]",
                "enabled = true",
            ]),
            &InstanceConfig::Subscription,
            Some(TEST_MODEL_INSTRUCTIONS_FILE),
        );

        assert_eq!(
            output,
            lines(&[
                "cli_auth_credentials_store = \"file\"",
                TEST_MODEL_INSTRUCTIONS_CONFIG_LINE,
                "",
                "[windows]",
                "sandbox = \"elevated\"",
                "sandbox_private_desktop = false",
                "",
                "[plugins.\"chrome@openai-bundled\"]",
                "enabled = true",
            ])
        );
    }

    #[test]
    fn merge_instance_config_creates_minimal_api_config() {
        let output = merge_instance_config_lines(
            &[],
            &InstanceConfig::Api {
                base_url: "https://api.example.com/v1".to_string(),
            },
            Some(TEST_MODEL_INSTRUCTIONS_FILE),
        );

        assert_eq!(
            output,
            lines(&[
                "model_provider = \"api\"",
                "cli_auth_credentials_store = \"file\"",
                TEST_MODEL_INSTRUCTIONS_CONFIG_LINE,
                "",
                "[windows]",
                "sandbox = \"elevated\"",
                "",
                "[model_providers.api]",
                "name = \"api\"",
                "base_url = \"https://api.example.com/v1\"",
                "wire_api = \"responses\"",
                "supports_websockets = false",
                "requires_openai_auth = true",
            ])
        );
    }

    #[test]
    fn merge_instance_config_creates_minimal_subscription_config() {
        let output = merge_instance_config_lines(
            &[],
            &InstanceConfig::Subscription,
            Some(TEST_MODEL_INSTRUCTIONS_FILE),
        );

        assert_eq!(
            output,
            lines(&[
                "cli_auth_credentials_store = \"file\"",
                TEST_MODEL_INSTRUCTIONS_CONFIG_LINE,
                "",
                "[windows]",
                "sandbox = \"elevated\"",
            ])
        );
    }

    #[test]
    fn merge_instance_config_removes_model_instructions_when_disabled() {
        let output = merge_instance_config_lines(
            &lines(&[
                "model_provider = \"api\"",
                "cli_auth_credentials_store = \"file\"",
                "model_instructions_file = \"./old.md\"",
                "",
                "[windows]",
                "sandbox = \"unelevated\"",
            ]),
            &InstanceConfig::Api {
                base_url: "https://api.example.com/v1".to_string(),
            },
            None,
        );

        let output = joined(output);
        assert!(!output.contains("model_instructions_file"));
        assert!(output.contains("model_provider = \"api\""));
        assert!(output.contains("[windows]\nsandbox = \"elevated\""));
    }

    #[test]
    fn format_toml_string_escapes_backslashes_and_quotes() {
        assert_eq!(format_toml_string("a\\b\"c"), "\"a\\\\b\\\"c\"");
    }
}
