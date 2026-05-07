use super::normalize::{newer_usage_info, usage_info_from_line};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
};

pub(crate) fn usage_info_from_file(path: &Path) -> Result<Option<Value>, String> {
    let file = fs::File::open(path)
        .map_err(|err| format!("读取 Codex session 文件失败 {}: {err}", path.display()))?;
    let reader = BufReader::new(file);
    let mut latest = None;
    for line in reader.lines() {
        let line =
            line.map_err(|err| format!("读取 Codex session 文件失败 {}: {err}", path.display()))?;
        if let Some(usage_info) = usage_info_from_line(&line) {
            latest = newer_usage_info(latest, usage_info);
        }
    }
    Ok(latest)
}
