use super::util::backup_stamp;
#[cfg(not(test))]
use crate::paths::app_data_dir;
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const SESSION_MANAGER_DATA_DIR: &str = "session-manager";

#[cfg(not(test))]
pub(super) fn session_manager_data_dir() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join(SESSION_MANAGER_DATA_DIR))
}

// Unit tests take real state DB and global state backups; they must not write into the user's
// application data directory (same rule as the error log in app_log).
#[cfg(test)]
pub(super) fn session_manager_data_dir() -> Result<PathBuf, String> {
    Ok(std::env::temp_dir()
        .join("codex-switch-session-manager-tests")
        .join(SESSION_MANAGER_DATA_DIR))
}

pub(super) fn session_manager_backup_dir(reason: &str) -> Result<PathBuf, String> {
    let reason = sanitize_backup_reason(reason);
    Ok(session_manager_data_dir()?.join("backups").join(reason))
}

pub(super) fn unique_backup_id(id: &str) -> String {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{}-{}-{suffix}", backup_stamp(), sanitize_id_fragment(id))
}

pub(super) fn status_overwrite_backup_path(target: &Path, id: &str) -> PathBuf {
    let file_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("session.jsonl");
    target.with_file_name(format!(
        ".{file_name}.codex-switch-overwrite-{}",
        unique_backup_id(id)
    ))
}

pub(super) fn sanitize_id_fragment(id: &str) -> String {
    let value = id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .take(80)
        .collect::<String>();
    if value.is_empty() {
        "session".to_string()
    } else {
        value
    }
}

pub(super) fn sanitize_backup_reason(reason: &str) -> String {
    let value = reason
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .collect::<String>();
    if value.is_empty() {
        "general".to_string()
    } else {
        value
    }
}

/// A fresh, reserved (created empty) backup file for `file_name` in the data directory's
/// `backups/<reason>/`; the caller fills it. Names carry a second-resolution stamp, so the file is
/// claimed with `create_new`: a check-then-create lets two backups in the same second pick the
/// same name, and the second one then fails (VACUUM INTO) or overwrites the first (fs::write).
pub(super) fn reserve_reason_backup_file(file_name: &str, reason: &str) -> Result<PathBuf, String> {
    let reason = sanitize_backup_reason(reason);
    let base_name = format!(
        "{file_name}.bak.context-manager-{reason}-{}",
        backup_stamp()
    );
    let backup_dir = session_manager_backup_dir(&reason)?;
    fs::create_dir_all(&backup_dir)
        .map_err(|err| format!("创建备份目录失败 {}: {err}", backup_dir.display()))?;
    for index in 0..1000 {
        let candidate = if index == 0 {
            backup_dir.join(&base_name)
        } else {
            backup_dir.join(format!("{base_name}-{index:03}"))
        };
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(format!("创建备份文件失败 {}: {err}", candidate.display())),
        }
    }
    Err(format!(
        "备份文件名已用尽: {}",
        backup_dir.join(&base_name).display()
    ))
}

/// A reserved backup location for `path` (see `reserve_reason_backup_file`).
pub(super) fn reason_backup_path(path: &Path, reason: &str) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("备份文件名无效: {}", path.display()))?;
    reserve_reason_backup_file(file_name, reason)
}
