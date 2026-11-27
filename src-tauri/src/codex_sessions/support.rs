use crate::paths::{codex_dir, codex_state_db_path};
use std::path::PathBuf;

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
