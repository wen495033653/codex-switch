mod date_dirs;
mod home;
mod recent;
mod recursive;

use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

pub(super) use home::codex_home_dir;

pub(super) fn collect_recent_files(
    sessions_dir: &Path,
) -> Result<Vec<(SystemTime, PathBuf)>, String> {
    let mut files = recent::RecentRolloutFiles::new();
    date_dirs::collect_recent_files(sessions_dir, &mut files)?;
    if files.len() < recent::SESSION_FILE_LIMIT {
        recursive::collect_recent_files(sessions_dir, &mut files)?;
    }
    Ok(files.into_vec())
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
