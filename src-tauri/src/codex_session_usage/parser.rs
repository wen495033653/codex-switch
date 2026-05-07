mod event;
mod file;
mod normalize;

use crate::{json_util::string_field, time_util::parse_rfc3339_seconds};
use serde_json::Value;

pub(super) use file::usage_info_from_file;
pub(super) use normalize::newer_usage_info;

pub(crate) fn usage_info_fetched_at_seconds(usage_info: &Value) -> Option<i64> {
    parse_rfc3339_seconds(&string_field(usage_info, "fetched_at"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{env, fs, path::PathBuf};

    fn token_count_line(timestamp: &str, used_percent: f64) -> String {
        json!({
            "timestamp": timestamp,
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "rate_limits": {
                    "primary": {
                        "used_percent": used_percent,
                        "limit_window_seconds": 18_000,
                        "reset_at": 1_799_999_999
                    },
                    "secondary": {
                        "used_percent": 1.0,
                        "limit_window_seconds": 604_800,
                        "reset_at": 1_799_999_999
                    }
                }
            }
        })
        .to_string()
    }

    fn primary_used_percent(usage_info: &Value) -> f64 {
        usage_info
            .get("rate_limit")
            .and_then(|rate_limit| rate_limit.get("primary_window"))
            .and_then(|window| window.get("used_percent"))
            .and_then(Value::as_f64)
            .unwrap()
    }

    #[test]
    fn session_file_uses_newest_token_count_timestamp() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path: PathBuf = env::temp_dir().join(format!("codex-switch-session-{stamp}.jsonl"));
        let newer = token_count_line("2026-05-05T02:00:00Z", 42.0);
        let older = token_count_line("2026-05-05T01:00:00Z", 12.0);
        fs::write(&path, format!("{newer}\n{older}\n")).unwrap();

        let usage_info = usage_info_from_file(&path).unwrap().unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(
            string_field(&usage_info, "fetched_at"),
            "2026-05-05T02:00:00Z"
        );
        assert_eq!(primary_used_percent(&usage_info), 42.0);
    }

    #[test]
    fn session_usage_requires_valid_timestamp() {
        assert!(normalize::usage_info_from_line(&token_count_line("", 42.0)).is_none());
        assert!(normalize::usage_info_from_line(&token_count_line("not-a-date", 42.0)).is_none());
    }
}
