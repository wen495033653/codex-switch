use std::{
    cmp::Reverse,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub(super) const SESSION_FILE_LIMIT: usize = 24;

pub(super) struct RecentRolloutFiles {
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
                eprintln!(
                    "读取 Codex session 文件修改时间失败 {}: {err}",
                    path.display()
                );
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
