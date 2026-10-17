use crate::{
    atomic_file::write_file_atomically,
    paths::{config_path, ensure_parent_dir},
};
use std::{fs, io::ErrorKind, path::PathBuf};

/// Creates an empty config.toml when there is none, for opening it in an editor.
pub(crate) fn ensure_config_file() -> Result<PathBuf, String> {
    let path = config_path()?;
    ensure_parent_dir(&path)?;
    if !path.exists() {
        fs::write(&path, "").map_err(|err| format!("创建 config.toml 失败: {err}"))?;
    }
    Ok(path)
}

struct ConfigText {
    lines: Vec<String>,
    crlf: bool,
}

fn read_config_text() -> Result<ConfigText, String> {
    let path = config_path()?;
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        // No config.toml yet reads as an empty one; reading never creates it.
        Err(err) if err.kind() == ErrorKind::NotFound => String::new(),
        Err(err) => return Err(format!("读取 config.toml 失败: {err}")),
    };
    Ok(ConfigText {
        crlf: raw.contains("\r\n"),
        lines: raw.lines().map(str::to_string).collect(),
    })
}

pub(super) fn read_config_lines() -> Result<Vec<String>, String> {
    Ok(read_config_text()?.lines)
}

/// Applies `edit` to config.toml and writes the result only when a line changed, with the line
/// endings the file had. Codex and the user write this file too; leaving an unchanged file
/// alone keeps its bytes and modification time. Returns whether it was written.
pub(super) fn update_config_lines(
    edit: impl FnOnce(&[String]) -> Vec<String>,
) -> Result<bool, String> {
    let text = read_config_text()?;
    let next = edit(&text.lines);
    if next == text.lines {
        return Ok(false);
    }
    let path = config_path()?;
    ensure_parent_dir(&path)?;
    let line_ending = if text.crlf { "\r\n" } else { "\n" };
    let mut raw = next.join(line_ending);
    raw.push_str(line_ending);
    write_file_atomically(&path, raw.as_bytes())
        .map_err(|err| format!("写入 config.toml 失败: {err}"))?;
    Ok(true)
}
