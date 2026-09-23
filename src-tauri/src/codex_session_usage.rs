mod parser;
mod scanner;

use crate::app_log::log_event_once;
use parser::FileUsageProgress;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

pub(crate) use parser::{inherit_stored_usage_fields, usage_info_fetched_at_seconds};

struct CachedFileUsage {
    modified: Option<SystemTime>,
    size: u64,
    progress: FileUsageProgress,
    latest: Option<Value>,
}

type FileUsageCache = HashMap<PathBuf, CachedFileUsage>;

// The active-quota refresher calls latest_usage_info every minute. Without this cache every call
// read all recent rollout files from the first byte (about 147 MB per minute on a busy home).
// Confirmed in the installed v6.0.5 on 2026-09-23 (docs/development/usage-stats.md, "安装版
// v6.0.5"): no recurring ~147 MB read, and the active account's quota kept updating.
static FILE_USAGE_CACHE: OnceLock<Mutex<FileUsageCache>> = OnceLock::new();

pub(crate) fn latest_usage_info() -> Result<Option<Value>, String> {
    let sessions_dir = scanner::codex_home_dir().join("sessions");
    if !sessions_dir.exists() {
        return Ok(None);
    }
    let mut cache = FILE_USAGE_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "Codex session 用量缓存锁异常".to_string())?;
    latest_usage_info_in(&sessions_dir, &mut cache)
}

fn latest_usage_info_in(
    sessions_dir: &Path,
    cache: &mut FileUsageCache,
) -> Result<Option<Value>, String> {
    let files = scanner::collect_recent_files(sessions_dir)?;

    let mut latest = None;
    for (_, path) in &files {
        match file_usage_info(path, cache) {
            Ok(Some(usage_info)) => {
                latest = parser::newer_usage_info(latest, usage_info);
            }
            Ok(None) => {}
            // One unreadable rollout must not hide the quota in the others; it runs every minute,
            // so each distinct failure is recorded once per run.
            Err(err) => log_event_once(
                "codex_session_usage_file_error",
                json!({ "path": path.to_string_lossy(), "error": err, "handling": "skipped" }),
            ),
        }
    }
    cache.retain(|cached_path, _| files.iter().any(|(_, path)| path == cached_path));
    Ok(latest)
}

// An unchanged file is answered from the cache without opening it; a changed one is folded
// from where the previous read stopped.
fn file_usage_info(path: &Path, cache: &mut FileUsageCache) -> Result<Option<Value>, String> {
    let metadata = fs::metadata(path).map_err(|err| {
        format!(
            "读取 Codex session 文件元数据失败 {}: {err}",
            path.display()
        )
    })?;
    let modified = metadata.modified().ok();
    let size = metadata.len();
    let cached = cache.get(path);
    if let Some(cached) = cached {
        if modified.is_some() && cached.modified == modified && cached.size == size {
            return Ok(cached.latest.clone());
        }
    }

    let previous = cached.map(|cached| cached.progress.clone());
    let (progress, latest) = parser::fold_usage_info_from_file(path, previous)?;
    cache.insert(
        path.to_path_buf(),
        CachedFileUsage {
            modified,
            size,
            progress,
            latest: latest.clone(),
        },
    );
    Ok(latest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{env, io::Write};

    fn unique_temp_sessions_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("codex-switch-{name}-{stamp}"))
    }

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

    fn fetched_at(usage_info: &Option<Value>) -> String {
        crate::json_util::string_field(usage_info.as_ref().unwrap(), "fetched_at")
    }

    #[test]
    fn unchanged_files_are_answered_from_the_cache_and_appends_are_picked_up() {
        let sessions_dir = unique_temp_sessions_dir("usage-cache");
        let day_dir = sessions_dir.join("2026").join("05").join("05");
        fs::create_dir_all(&day_dir).unwrap();
        let path = day_dir.join("rollout-2026-05-05T01-00-00-a.jsonl");
        fs::write(
            &path,
            format!("{}\n", token_count_line("2026-05-05T01:00:00Z", 10.0)),
        )
        .unwrap();
        let mut cache = FileUsageCache::new();

        let first = latest_usage_info_in(&sessions_dir, &mut cache).unwrap();
        // Poison the cached answer: an unchanged file must be served from it, not re-read.
        cache.get_mut(&path).unwrap().latest = Some(json!({"fetched_at": "cached"}));
        let cached = latest_usage_info_in(&sessions_dir, &mut cache).unwrap();

        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{}", token_count_line("2026-05-05T02:00:00Z", 20.0)).unwrap();
        drop(file);
        let appended = latest_usage_info_in(&sessions_dir, &mut cache).unwrap();

        fs::remove_file(&path).unwrap();
        let removed = latest_usage_info_in(&sessions_dir, &mut cache).unwrap();
        fs::remove_dir_all(&sessions_dir).unwrap();

        assert_eq!(fetched_at(&first), "2026-05-05T01:00:00Z");
        assert_eq!(fetched_at(&cached), "cached");
        assert_eq!(fetched_at(&appended), "2026-05-05T02:00:00Z");
        assert!(removed.is_none());
        assert!(cache.is_empty());
    }
}
