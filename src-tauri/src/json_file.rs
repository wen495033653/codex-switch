use crate::paths::ensure_parent_dir;
use serde_json::Value;
use std::{fs, path::Path};

pub(crate) fn read_json_file(path: &Path, label: &str) -> Result<Value, String> {
    let raw = fs::read_to_string(path).map_err(|err| format!("读取 {label} 失败: {err}"))?;
    serde_json::from_str(&raw).map_err(|err| format!("解析 {label} 失败: {err}"))
}

pub(crate) fn write_json_file(path: &Path, label: &str, value: &Value) -> Result<(), String> {
    ensure_parent_dir(path)?;
    let raw =
        serde_json::to_string_pretty(value).map_err(|err| format!("序列化 {label} 失败: {err}"))?;
    fs::write(path, raw).map_err(|err| format!("写入 {label} 失败: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{env, time::SystemTime};

    fn unique_temp_path(name: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("codex-switch-json-file-{name}-{stamp}.json"))
    }

    #[test]
    fn json_file_round_trips_pretty_json() {
        let path = unique_temp_path("round-trip");
        let value = json!({ "name": "Codex Switch", "enabled": true });

        write_json_file(&path, "test.json", &value).unwrap();
        let parsed = read_json_file(&path, "test.json").unwrap();

        assert_eq!(parsed, value);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_json_file_reports_parse_error() {
        let path = unique_temp_path("parse-error");
        fs::write(&path, "{").unwrap();

        let err = read_json_file(&path, "broken.json").unwrap_err();

        assert!(err.starts_with("解析 broken.json 失败:"));
        fs::remove_file(path).unwrap();
    }
}
