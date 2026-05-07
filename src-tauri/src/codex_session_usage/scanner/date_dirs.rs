use super::recent::RecentRolloutFiles;
use std::{
    fs,
    path::{Path, PathBuf},
};

const SESSION_DATE_DIR_SCAN_LIMIT: usize = 7;

pub(super) fn collect_recent_files(
    sessions_dir: &Path,
    files: &mut RecentRolloutFiles,
) -> Result<(), String> {
    let mut scanned_date_dirs = 0;
    for (_, year_dir) in read_child_dirs(sessions_dir, 4)? {
        let month_dirs = match read_child_dirs(&year_dir, 2) {
            Ok(month_dirs) => month_dirs,
            Err(err) => {
                eprintln!("{err}");
                continue;
            }
        };
        for (_, month_dir) in month_dirs {
            let day_dirs = match read_child_dirs(&month_dir, 2) {
                Ok(day_dirs) => day_dirs,
                Err(err) => {
                    eprintln!("{err}");
                    continue;
                }
            };
            for (_, day_dir) in day_dirs {
                if let Err(err) = collect_files_from_date_dir(&day_dir, files) {
                    eprintln!("{err}");
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
                eprintln!("读取 Codex session 目录条目失败: {err}");
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(err) => {
                eprintln!("读取 Codex session 目录类型失败 {}: {err}", path.display());
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
        if file_type.is_file() {
            files.push_from_entry(&entry, &path);
        }
    }
    Ok(())
}
