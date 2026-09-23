use crate::session_sync_diagnostics::log_session_sync_event_once;
use serde_json::json;
use std::{
    cmp::Reverse,
    env, fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// A directory or file the scan could not read is skipped. The scan runs every minute, so each
/// distinct failure is recorded once per run.
fn log_scan_error(error: String) {
    log_session_sync_event_once(
        "codex_session_usage_scan_error",
        json!({ "error": error, "handling": "skipped" }),
    );
}

pub(super) fn collect_recent_files(
    sessions_dir: &Path,
) -> Result<Vec<(SystemTime, PathBuf)>, String> {
    let mut files = RecentRolloutFiles::new();
    collect_recent_files_from_date_dirs(sessions_dir, &mut files)?;
    if files.len() < SESSION_FILE_LIMIT {
        collect_recent_files_recursive(sessions_dir, &mut files)?;
    }
    Ok(files.into_vec())
}

const SESSION_DATE_DIR_SCAN_LIMIT: usize = 7;

fn collect_recent_files_from_date_dirs(
    sessions_dir: &Path,
    files: &mut RecentRolloutFiles,
) -> Result<(), String> {
    let mut scanned_date_dirs = 0;
    for (_, year_dir) in read_child_dirs(sessions_dir, 4)? {
        let month_dirs = match read_child_dirs(&year_dir, 2) {
            Ok(month_dirs) => month_dirs,
            Err(err) => {
                log_scan_error(err);
                continue;
            }
        };
        for (_, month_dir) in month_dirs {
            let day_dirs = match read_child_dirs(&month_dir, 2) {
                Ok(day_dirs) => day_dirs,
                Err(err) => {
                    log_scan_error(err);
                    continue;
                }
            };
            for (_, day_dir) in day_dirs {
                if let Err(err) = collect_files_from_date_dir(&day_dir, files) {
                    log_scan_error(err);
                }
                scanned_date_dirs += 1;
                if scanned_date_dirs >= SESSION_DATE_DIR_SCAN_LIMIT {
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

fn is_fixed_width_digits(value: &str, width: usize) -> bool {
    value.len() == width && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn read_child_dirs(dir: &Path, name_width: usize) -> Result<Vec<(String, PathBuf)>, String> {
    let entries = fs::read_dir(dir)
        .map_err(|err| format!("读取 Codex sessions 目录失败 {}: {err}", dir.display()))?;
    let mut dirs = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                log_scan_error(format!("读取 Codex session 目录条目失败: {err}"));
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(err) => {
                log_scan_error(format!(
                    "读取 Codex session 目录类型失败 {}: {err}",
                    path.display()
                ));
                continue;
            }
        };
        if !file_type.is_dir() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if is_fixed_width_digits(name, name_width) {
            dirs.push((name.to_string(), path));
        }
    }
    dirs.sort_unstable_by(|left, right| right.0.cmp(&left.0));
    Ok(dirs)
}

fn collect_files_from_date_dir(
    date_dir: &Path,
    files: &mut RecentRolloutFiles,
) -> Result<(), String> {
    let entries = fs::read_dir(date_dir)
        .map_err(|err| format!("读取 Codex sessions 目录失败 {}: {err}", date_dir.display()))?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                log_scan_error(format!("读取 Codex session 条目失败: {err}"));
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(err) => {
                log_scan_error(format!(
                    "读取 Codex session 文件类型失败 {}: {err}",
                    path.display()
                ));
                continue;
            }
        };
        if file_type.is_file() {
            files.push_from_entry(&entry, &path);
        }
    }
    Ok(())
}

pub(super) fn codex_home_dir() -> PathBuf {
    if let Some(value) = env::var_os("CODEX_HOME") {
        return PathBuf::from(value);
    }
    #[cfg(windows)]
    let home = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME"));
    #[cfg(not(windows))]
    let home = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"));
    if let Some(value) = home {
        return PathBuf::from(value).join(".codex");
    }
    PathBuf::from(".codex")
}

const SESSION_FILE_LIMIT: usize = 24;

struct RecentRolloutFiles {
    files: Vec<(SystemTime, PathBuf)>,
}

impl RecentRolloutFiles {
    pub(super) fn new() -> Self {
        Self { files: Vec::new() }
    }

    pub(super) fn len(&self) -> usize {
        self.files.len()
    }

    pub(super) fn push_from_entry(&mut self, entry: &fs::DirEntry, path: &Path) {
        if !is_rollout_jsonl(path) {
            return;
        }
        if self.files.iter().any(|(_, existing)| existing == path) {
            return;
        }
        let modified = match entry.metadata().and_then(|metadata| metadata.modified()) {
            Ok(modified) => modified,
            Err(err) => {
                log_scan_error(format!(
                    "读取 Codex session 文件修改时间失败 {}: {err}",
                    path.display()
                ));
                return;
            }
        };
        self.files.push((modified, path.to_path_buf()));
        self.files
            .sort_unstable_by_key(|(modified, _)| Reverse(*modified));
        self.files.truncate(SESSION_FILE_LIMIT);
    }

    pub(super) fn into_vec(self) -> Vec<(SystemTime, PathBuf)> {
        self.files
    }
}

fn is_rollout_jsonl(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|file_name| file_name.starts_with("rollout-"))
        && path.extension().and_then(|value| value.to_str()) == Some("jsonl")
}

fn collect_recent_files_recursive(
    dir: &Path,
    files: &mut RecentRolloutFiles,
) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|err| format!("读取 Codex sessions 目录失败 {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                log_scan_error(format!("读取 Codex session 条目失败: {err}"));
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(err) => {
                log_scan_error(format!(
                    "读取 Codex session 文件类型失败 {}: {err}",
                    path.display()
                ));
                continue;
            }
        };
        if file_type.is_dir() {
            if let Err(err) = collect_recent_files_recursive(&path, files) {
                log_scan_error(err);
            }
            continue;
        }
        if file_type.is_file() {
            files.push_from_entry(&entry, &path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, fs};

    fn unique_temp_session_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("codex-switch-{name}-{stamp}"))
    }

    fn write_rollout_file(dir: &Path, name: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), "{}\n").unwrap();
    }

    #[test]
    fn session_file_scan_checks_more_than_the_latest_date_dir() {
        let sessions_dir = unique_temp_session_dir("sessions");
        let latest_dir = sessions_dir.join("2026").join("05").join("05");
        let older_dir = sessions_dir.join("2026").join("05").join("04");
        write_rollout_file(&latest_dir, "rollout-2026-05-05T02-00-00-newest-a.jsonl");
        write_rollout_file(&latest_dir, "rollout-2026-05-05T01-00-00-newest-b.jsonl");
        write_rollout_file(&latest_dir, "rollout-2026-05-05T00-00-00-newest-c.jsonl");
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_rollout_file(&older_dir, "rollout-2026-05-04T23-00-00-older.jsonl");

        let files = collect_recent_files(&sessions_dir).unwrap();
        let file_count = files.len();
        let includes_older_date_dir = files.iter().any(|(_, path)| path.starts_with(&older_dir));
        fs::remove_dir_all(&sessions_dir).unwrap();

        assert_eq!(file_count, 4);
        assert!(includes_older_date_dir);
    }

    #[test]
    fn session_file_scan_falls_back_when_date_dirs_are_missing() {
        let sessions_dir = unique_temp_session_dir("sessions-fallback");
        let fallback_dir = sessions_dir.join("latest");
        write_rollout_file(&fallback_dir, "rollout-fallback.jsonl");

        let files = collect_recent_files(&sessions_dir).unwrap();
        let file_count = files.len();
        let found_fallback_file = files
            .iter()
            .any(|(_, path)| path.ends_with("rollout-fallback.jsonl"));
        fs::remove_dir_all(&sessions_dir).unwrap();

        assert_eq!(file_count, 1);
        assert!(found_fallback_file);
    }
}
