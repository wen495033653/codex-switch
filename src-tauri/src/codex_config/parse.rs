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

fn root_config_from_lines(lines: &[String]) -> Map<String, Value> {
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
    let Some(start) = lines.iter().position(|line| line.trim() == header) else {
        return Map::new();
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
    config
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
