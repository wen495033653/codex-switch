mod event;
mod file;
mod normalize;

use crate::{json_util::string_field, time_util::parse_rfc3339_seconds};
use serde_json::Value;

pub(super) use file::{fold_usage_info_from_file, FileUsageProgress};
pub(crate) use normalize::inherit_stored_usage_fields;
pub(super) use normalize::newer_usage_info;

pub(crate) fn usage_info_fetched_at_seconds(usage_info: &Value) -> Option<i64> {
    parse_rfc3339_seconds(&string_field(usage_info, "fetched_at"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{env, fs, path::PathBuf};

    fn token_count_line_with_reset_at(timestamp: &str, used_percent: f64, reset_at: i64) -> String {
        json!({
            "timestamp": timestamp,
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "rate_limits": {
                    "primary": {
                        "used_percent": used_percent,
                        "limit_window_seconds": 18_000,
                        "reset_at": reset_at
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

    fn token_count_line(timestamp: &str, used_percent: f64) -> String {
        token_count_line_with_reset_at(timestamp, used_percent, 1_799_999_999)
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

        let (_, usage_info) = fold_usage_info_from_file(&path, None).unwrap();
        let usage_info = usage_info.unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(
            string_field(&usage_info, "fetched_at"),
            "2026-05-05T02:00:00Z"
        );
        assert_eq!(primary_used_percent(&usage_info), 42.0);
    }

    fn unique_temp_rollout_path(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("codex-switch-{name}-{stamp}.jsonl"))
    }

    #[test]
    fn resumed_fold_reads_only_appended_lines_and_matches_a_full_read() {
        let path = unique_temp_rollout_path("session-resume");
        let first = token_count_line("2026-05-05T01:00:00Z", 10.0);
        fs::write(
            &path,
            format!(
                "{first}
"
            ),
        )
        .unwrap();
        let (progress, _) = fold_usage_info_from_file(&path, None).unwrap();

        // Overwrite the already folded line with same-length filler that keeps the bytes the
        // resume check compares. A resumed fold must not look at it again.
        let second = token_count_line("2026-05-05T02:00:00Z", 20.0);
        let filler = format!(
            "{}{}",
            " ".repeat(first.len() - 64),
            &first[first.len() - 64..]
        );
        fs::write(
            &path,
            format!(
                "{filler}
{second}
"
            ),
        )
        .unwrap();
        let (_, resumed) = fold_usage_info_from_file(&path, Some(progress)).unwrap();
        let (_, full) = fold_usage_info_from_file(&path, None).unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(primary_used_percent(resumed.as_ref().unwrap()), 20.0);
        assert_eq!(resumed, full);
    }

    #[test]
    fn rewritten_file_is_folded_from_the_start() {
        let path = unique_temp_rollout_path("session-rewrite");
        let stale = token_count_line("2026-05-05T09:00:00Z", 90.0);
        fs::write(
            &path,
            format!(
                "{stale}
"
            ),
        )
        .unwrap();
        let (progress, _) = fold_usage_info_from_file(&path, None).unwrap();

        let rewritten = token_count_line("2026-05-05T03:00:00Z", 30.0);
        let padding = json!({"type": "padding", "text": "x".repeat(stale.len())}).to_string();
        fs::write(
            &path,
            format!(
                "{rewritten}
{padding}
"
            ),
        )
        .unwrap();
        let (_, usage_info) = fold_usage_info_from_file(&path, Some(progress)).unwrap();
        fs::remove_file(&path).unwrap();

        // The stale 09:00 reading would win if the old progress had been kept.
        let usage_info = usage_info.unwrap();
        assert_eq!(
            string_field(&usage_info, "fetched_at"),
            "2026-05-05T03:00:00Z"
        );
        assert_eq!(primary_used_percent(&usage_info), 30.0);
    }

    #[test]
    fn half_written_line_is_read_again_once_complete() {
        let path = unique_temp_rollout_path("session-partial");
        let first = token_count_line("2026-05-05T01:00:00Z", 10.0);
        let second = token_count_line("2026-05-05T02:00:00Z", 20.0);
        let (head, tail) = second.split_at(second.len() / 2);
        fs::write(
            &path,
            format!(
                "{first}
{head}"
            ),
        )
        .unwrap();
        let (progress, partial) = fold_usage_info_from_file(&path, None).unwrap();

        fs::write(
            &path,
            format!(
                "{first}
{head}{tail}
"
            ),
        )
        .unwrap();
        let (_, complete) = fold_usage_info_from_file(&path, Some(progress)).unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(primary_used_percent(partial.as_ref().unwrap()), 10.0);
        assert_eq!(primary_used_percent(complete.as_ref().unwrap()), 20.0);
    }

    #[test]
    fn session_usage_keeps_plan_type_without_reset_credits() {
        let line = json!({
            "timestamp": "2026-09-13T17:14:32Z",
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "rate_limits": {
                    "limit_id": "codex",
                    "primary": {
                        "used_percent": 99.0,
                        "window_minutes": 10080,
                        "resets_at": 1_789_889_905
                    },
                    "secondary": null,
                    "plan_type": "pro"
                }
            }
        })
        .to_string();

        let usage_info = normalize::usage_info_from_line(&line).unwrap();

        assert_eq!(string_field(&usage_info, "plan_type"), "pro");
        assert!(usage_info.get("reset_credits").is_none());
        assert_eq!(primary_used_percent(&usage_info), 99.0);
    }

    fn session_usage_with_plan(plan_type: &str) -> Value {
        json!({
            "rate_limit": {
                "primary_window": {
                    "used_percent": 5.0,
                    "limit_window_seconds": 604800.0,
                    "reset_at": 1789889905.0
                },
                "secondary_window": null
            },
            "plan_type": plan_type,
            "fetched_at": "2026-09-14T04:00:00Z"
        })
    }

    #[test]
    fn session_usage_inherits_reset_credits_and_missing_plan_type() {
        let previous = json!({
            "plan_type": "pro",
            "reset_credits": { "available_count": 3, "applicable_available_count": 0 }
        });

        let merged = inherit_stored_usage_fields(&previous, session_usage_with_plan(""));

        assert_eq!(merged["plan_type"], json!("pro"));
        assert_eq!(
            merged["reset_credits"],
            json!({ "available_count": 3, "applicable_available_count": 0 })
        );
        assert_eq!(primary_used_percent(&merged), 5.0);
    }

    #[test]
    fn session_plan_type_wins_over_stored_plan_type() {
        let previous = json!({ "plan_type": "plus", "reset_credits": null });

        let merged = inherit_stored_usage_fields(&previous, session_usage_with_plan("pro"));

        assert_eq!(merged["plan_type"], json!("pro"));
        assert!(merged.get("reset_credits").is_none());
    }

    #[test]
    fn session_usage_requires_valid_timestamp() {
        assert!(normalize::usage_info_from_line(&token_count_line("", 42.0)).is_none());
        assert!(normalize::usage_info_from_line(&token_count_line("not-a-date", 42.0)).is_none());
    }

    #[test]
    fn session_usage_keeps_highest_primary_usage_for_same_reset_window() {
        let current =
            normalize::usage_info_from_line(&token_count_line("2026-05-05T01:00:00Z", 44.0))
                .unwrap();
        let candidate = normalize::usage_info_from_line(&token_count_line_with_reset_at(
            "2026-05-05T01:06:00Z",
            1.0,
            1_800_000_033,
        ))
        .unwrap();

        let usage_info = newer_usage_info(Some(current), candidate).unwrap();

        assert_eq!(
            string_field(&usage_info, "fetched_at"),
            "2026-05-05T01:06:00Z"
        );
        assert_eq!(primary_used_percent(&usage_info), 44.0);
    }

    #[test]
    fn session_usage_allows_lower_primary_usage_for_new_reset_window() {
        let current =
            normalize::usage_info_from_line(&token_count_line("2026-05-05T01:00:00Z", 44.0))
                .unwrap();
        let candidate = normalize::usage_info_from_line(&token_count_line_with_reset_at(
            "2026-05-05T06:01:00Z",
            1.0,
            1_800_018_000,
        ))
        .unwrap();

        let usage_info = newer_usage_info(Some(current), candidate).unwrap();

        assert_eq!(
            string_field(&usage_info, "fetched_at"),
            "2026-05-05T06:01:00Z"
        );
        assert_eq!(primary_used_percent(&usage_info), 1.0);
    }
}
