use crate::paths::{codex_dir, codex_state_db_path};
use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

pub(super) const GLOBAL_STATE_FILE_NAME: &str = ".codex-global-state.json";

pub(super) fn state_db_path() -> Result<PathBuf, String> {
    codex_state_db_path()
}

pub(super) fn global_state_path() -> Result<PathBuf, String> {
    Ok(codex_dir()?.join(GLOBAL_STATE_FILE_NAME))
}

pub(super) fn provider_log_value(provider: &str) -> String {
    let provider = provider.trim();
    if provider.is_empty() {
        "(未设置)".to_string()
    } else {
        provider.to_string()
    }
}

pub(super) fn write_existing_file(
    path: &Path,
    content: &str,
    action: &str,
) -> Result<bool, String> {
    let mut file = match fs::OpenOptions::new().write(true).truncate(true).open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(format!("{action}失败 {}: {err}", path.display())),
    };
    file.write_all(content.as_bytes())
        .map_err(|err| format!("{action}失败 {}: {err}", path.display()))?;
    Ok(true)
}
