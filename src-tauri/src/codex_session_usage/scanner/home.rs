use std::{env, path::PathBuf};

pub(crate) fn codex_home_dir() -> PathBuf {
    if let Some(value) = env::var_os("CODEX_HOME") {
        return PathBuf::from(value);
    }
    if let Some(value) = env::var_os("USERPROFILE") {
        return PathBuf::from(value).join(".codex");
    }
    if let Some(value) = env::var_os("HOME") {
        return PathBuf::from(value).join(".codex");
    }
    PathBuf::from(".codex")
}
