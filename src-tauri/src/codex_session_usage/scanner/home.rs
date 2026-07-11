use std::{env, path::PathBuf};

pub(crate) fn codex_home_dir() -> PathBuf {
    if let Some(value) = env::var_os("CODEX_HOME") {
        return PathBuf::from(value);
    }
    #[cfg(windows)]
    let home = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME"));
    #[cfg(not(windows))]
    let home = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"));
    if let Some(value) = home {
        return PathBuf::from(value).join(".codex");
    }
    PathBuf::from(".codex")
}
