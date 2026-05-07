use super::recent::RecentRolloutFiles;
use std::{fs, path::Path};

pub(super) fn collect_recent_files(
    dir: &Path,
    files: &mut RecentRolloutFiles,
) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|err| format!("读取 Codex sessions 目录失败 {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                eprintln!("读取 Codex session 条目失败: {err}");
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(err) => {
                eprintln!("读取 Codex session 文件类型失败 {}: {err}", path.display());
                continue;
            }
        };
        if file_type.is_dir() {
            if let Err(err) = collect_recent_files(&path, files) {
                eprintln!("{err}");
            }
            continue;
        }
        if file_type.is_file() {
            files.push_from_entry(&entry, &path);
        }
    }
    Ok(())
}
