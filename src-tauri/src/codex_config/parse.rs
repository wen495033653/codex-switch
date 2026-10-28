use super::io::read_config_lines;
use serde_json::{Map, Value};

/// The `[table]` or `[[array]]` header on this line, without a trailing `# comment`.
pub(crate) fn table_header(line: &str) -> Option<&str> {
    let line = line.trim();
    if !line.starts_with('[') {
        return None;
    }
    let close = if line.starts_with("[[") { "]]" } else { "]" };
    let end = line.find(close)? + close.len();
    let rest = line[end..].trim_start();
    (rest.is_empty() || rest.starts_with('#')).then(|| &line[..end])
}

pub(crate) fn find_root_table_index(lines: &[String]) -> Option<usize> {
    lines.iter().position(|line| table_header(line).is_some())
}

pub(crate) fn root_assignment(line: &str) -> Option<(String, String)> {
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
    let mut quote = None;
    let mut escaped = false;
    let mut end = raw_value.len();
    for (index, ch) in raw_value.char_indices() {
        if escaped {
            escaped = false;
        } else if quote == Some('"') && ch == '\\' {
            escaped = true;
        } else if quote == Some(ch) {
            quote = None;
        } else if quote.is_none() {
            if ch == '#' {
                end = index;
                break;
            }
            if ch == '"' || ch == '\'' {
                quote = Some(ch);
            }
        }
    }
    let raw = raw_value[..end].trim();
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
        let contents = &raw[1..raw.len() - 1];
        return Value::String(if raw.starts_with('\'') {
            contents.to_string()
        } else {
            contents.replace("\\\"", "\"").replace("\\\\", "\\")
        });
    }
    Value::String(raw.to_string())
}

/// config.toml read once, for callers that need several values from it.
pub(crate) struct ConfigSnapshot {
    lines: Vec<String>,
}

impl ConfigSnapshot {
    pub(crate) fn root(&self) -> Map<String, Value> {
        root_config_from_lines(&self.lines)
    }

    pub(crate) fn table(&self, table_name: &str) -> Map<String, Value> {
        table_config_from_lines(&self.lines, table_name)
    }
}

pub(crate) fn read_config_snapshot() -> Result<ConfigSnapshot, String> {
    Ok(ConfigSnapshot {
        lines: read_config_lines()?,
    })
}

pub(crate) fn read_root_config() -> Result<Map<String, Value>, String> {
    Ok(root_config_from_lines(&read_config_lines()?))
}

pub(crate) fn read_table_config(table_name: &str) -> Result<Map<String, Value>, String> {
    Ok(table_config_from_lines(&read_config_lines()?, table_name))
}

pub(crate) fn root_config_from_lines(lines: &[String]) -> Map<String, Value> {
    let end = find_root_table_index(lines).unwrap_or(lines.len());
    let mut config = Map::new();
    for line in lines.iter().take(end) {
        if let Some((key, value)) = root_assignment(line) {
            config.insert(key, parse_toml_value(&value));
        }
    }
    config
}

fn table_config_from_lines(lines: &[String], table_name: &str) -> Map<String, Value> {
    let header = format!("[{table_name}]");
    let Some(start) = lines
        .iter()
        .position(|line| table_header(line) == Some(header.as_str()))
    else {
        return Map::new();
    };

    let mut config = Map::new();
    for line in lines.iter().skip(start + 1) {
        if table_header(line).is_some() {
            break;
        }
        if let Some((key, value)) = root_assignment(line) {
            config.insert(key, parse_toml_value(&value));
        }
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_headers_may_carry_a_trailing_comment() {
        assert_eq!(table_header("[features]"), Some("[features]"));
        assert_eq!(
            table_header("  [features] # experimental"),
            Some("[features]")
        );
        assert_eq!(table_header("[[profiles]]#x"), Some("[[profiles]]"));
        assert_eq!(table_header("[features] model = 1"), None);
        assert_eq!(table_header("model = \"[x]\""), None);
        assert_eq!(table_header("# [features]"), None);
    }

    #[test]
    fn snapshot_reads_root_values_and_one_table() {
        let snapshot = ConfigSnapshot {
            lines: [
                "model_provider = \"api\"",
                "# comment = 1",
                "openai_base_url = 'https://example.test/v1'",
                "[model_providers.api]",
                "base_url = \"https://example.test/v1\"",
                "supports_websockets = false",
                "[other]",
                "base_url = \"ignored\"",
            ]
            .map(str::to_string)
            .to_vec(),
        };

        let root = snapshot.root();
        assert_eq!(root.len(), 2);
        assert_eq!(root["model_provider"], "api");
        assert_eq!(root["openai_base_url"], "https://example.test/v1");
        let table = snapshot.table("model_providers.api");
        assert_eq!(table.len(), 2);
        assert_eq!(table["base_url"], "https://example.test/v1");
        assert_eq!(table["supports_websockets"], false);
        assert!(snapshot.table("model_providers.missing").is_empty());
    }

    #[test]
    fn catalog_paths_allow_trailing_comments_without_changing_quoted_hashes() {
        let lines = [
            r#"model_catalog_json = "custom#models.json" # catalog"#,
            r"literal_path = '\\server\share#catalog.json' # literal",
            r##"escaped_path = "folder\\custom\"#models.json" # escaped quote"##,
            "supports_websockets = false # disabled",
        ]
        .map(str::to_string);
        let root = root_config_from_lines(&lines);
        assert_eq!(root["model_catalog_json"], "custom#models.json");
        assert_eq!(root["literal_path"], r"\\server\share#catalog.json");
        assert_eq!(root["escaped_path"], "folder\\custom\"#models.json");
        assert_eq!(root["supports_websockets"], false);
    }
}
