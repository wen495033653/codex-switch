use crate::paths::{config_path, ensure_parent_dir};
use std::{fs, path::PathBuf};

pub(crate) fn ensure_config_file() -> Result<PathBuf, String> {
    let path = config_path()?;
    ensure_parent_dir(&path)?;
    if !path.exists() {
        fs::write(&path, "").map_err(|err| format!("创建 config.toml 失败: {err}"))?;
    }
    Ok(path)
}

pub(super) fn read_config_lines() -> Result<Vec<String>, String> {
    let path = ensure_config_file()?;
    let raw = fs::read_to_string(&path).map_err(|err| format!("读取 config.toml 失败: {err}"))?;
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    Ok(raw.lines().map(|line| line.to_string()).collect())
}

pub(super) fn write_config_lines(lines: &[String]) -> Result<(), String> {
    let path = ensure_config_file()?;
    let mut raw = lines.join("\n");
    raw.push('\n');
    fs::write(path, raw).map_err(|err| format!("写入 config.toml 失败: {err}"))
}
