//! Replaces a file in one step: the new content goes to a temporary file in the same
//! directory, is flushed to disk, and is then renamed over the target. A crash or a
//! concurrent reader sees either the old file or the new one, never a truncated one.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A symlinked target is written through to the file it points to, as `fs::write` did,
/// instead of replacing the link itself with a regular file.
fn resolve_write_target(path: &Path) -> Result<PathBuf, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(path)
            .map_err(|err| format!("解析符号链接 {} 失败: {err}", path.display())),
        _ => Ok(path.to_path_buf()),
    }
}

fn temp_path_for(target: &Path) -> Result<PathBuf, String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("{} 没有上级目录", target.display()))?;
    let file_name = target
        .file_name()
        .ok_or_else(|| format!("{} 不是文件路径", target.display()))?
        .to_string_lossy();
    let sequence = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(".{file_name}.{}.{sequence}.tmp", process::id())))
}

fn write_and_flush(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create_new(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

pub(crate) fn write_file_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let target = resolve_write_target(path)?;
    let temp = temp_path_for(&target)?;
    let step_error = match write_and_flush(&temp, bytes) {
        Ok(()) => match fs::rename(&temp, &target) {
            Ok(()) => return Ok(()),
            Err(err) => format!("替换 {} 失败: {err}", target.display()),
        },
        Err(err) => format!("写入临时文件 {} 失败: {err}", temp.display()),
    };
    match fs::remove_file(&temp) {
        Ok(()) => Err(step_error),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(step_error),
        Err(err) => Err(format!(
            "{step_error}；清理临时文件 {} 失败: {err}",
            temp.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!("codex-switch-atomic-{name}-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn replaces_existing_content_and_leaves_no_temp_file() {
        let dir = unique_dir("replace");
        let path = dir.join("accounts.json");
        fs::write(&path, "old content that is longer than the new one").unwrap();

        write_file_atomically(&path, b"new").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(entries(&dir), vec!["accounts.json"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn creates_missing_file() {
        let dir = unique_dir("create");
        let path = dir.join("settings.json");

        write_file_atomically(&path, b"{}").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "{}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_replace_keeps_original_and_cleans_temp_file() {
        let dir = unique_dir("failed");
        // A directory at the target path makes the final rename fail on every platform.
        let path = dir.join("target");
        fs::create_dir_all(path.join("occupied")).unwrap();

        let err = write_file_atomically(&path, b"new").unwrap_err();

        assert!(err.contains("替换"), "{err}");
        assert!(path.join("occupied").is_dir());
        assert_eq!(entries(&dir), vec!["target"]);
        fs::remove_dir_all(dir).unwrap();
    }
}
