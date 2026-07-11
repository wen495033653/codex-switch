use std::{
    env, fs,
    path::{Path, PathBuf},
};

const APP_DATA_DIR_NAME: &str = "codex-switch";
const CODEX_SQLITE_DIR_NAME: &str = "sqlite";
const CODEX_STATE_DB_FILE_NAME: &str = "state_5.sqlite";

pub(crate) fn home_dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let home = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME"));
    #[cfg(not(windows))]
    let home = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"));

    home.map(PathBuf::from)
        .ok_or_else(|| "无法定位用户目录".to_string())
}

fn app_data_dir_named(dir_name: &str) -> Result<PathBuf, String> {
    if cfg!(target_os = "windows") {
        return env::var_os("APPDATA")
            .map(|base| PathBuf::from(base).join(dir_name))
            .ok_or_else(|| "APPDATA 环境变量不存在".to_string());
    }

    if cfg!(target_os = "macos") {
        return Ok(home_dir()?
            .join("Library")
            .join("Application Support")
            .join(dir_name));
    }

    if let Some(config_home) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(config_home).join(dir_name));
    }

    Ok(home_dir()?.join(".config").join(dir_name))
}

pub(crate) fn app_data_dir() -> Result<PathBuf, String> {
    app_data_dir_named(APP_DATA_DIR_NAME)
}

pub(crate) fn settings_path() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join("settings.json"))
}

pub(crate) fn accounts_path() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join("accounts.json"))
}

pub(crate) fn codex_dir() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".codex"))
}

pub(crate) fn codex_state_db_path() -> Result<PathBuf, String> {
    Ok(codex_state_db_path_from_home(&codex_dir()?))
}

pub(crate) fn codex_state_db_path_from_home(home: &Path) -> PathBuf {
    home.join(CODEX_STATE_DB_FILE_NAME)
}

pub(crate) fn codex_state_db_path_for_root(root: &Path) -> Result<PathBuf, String> {
    let global_home = codex_dir()?;
    if paths_equal(root, &global_home) {
        codex_state_db_path()
    } else {
        Ok(codex_state_db_path_from_home(root))
    }
}

pub(crate) fn legacy_codex_state_db_path_from_home(home: &Path) -> PathBuf {
    home.join(CODEX_SQLITE_DIR_NAME)
        .join(CODEX_STATE_DB_FILE_NAME)
}

pub(crate) fn codex_home_from_state_db_path(state_db: &Path) -> PathBuf {
    if let (Ok(global_state_db), Ok(global_home)) = (codex_state_db_path(), codex_dir()) {
        if paths_equal(state_db, &global_state_db) {
            return global_home;
        }
    }
    state_db
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf()
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

pub(crate) fn auth_path() -> Result<PathBuf, String> {
    Ok(codex_dir()?.join("auth.json"))
}

pub(crate) fn config_path() -> Result<PathBuf, String> {
    Ok(codex_dir()?.join("config.toml"))
}

pub(crate) fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建目录失败: {err}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_home(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("codex-switch-paths-{name}-{stamp}"))
    }

    #[test]
    fn codex_state_db_path_always_uses_current_root_path() {
        let home = temp_home("current-root");

        assert_eq!(
            codex_state_db_path_from_home(&home),
            home.join("state_5.sqlite")
        );
    }

    #[test]
    fn missing_codex_state_db_uses_current_root_path() {
        let home = temp_home("missing");

        assert_eq!(
            codex_state_db_path_from_home(&home),
            home.join("state_5.sqlite")
        );
    }

    #[test]
    fn codex_home_from_root_state_db_path_returns_parent() {
        let home = temp_home("home-from-state");
        let state_db = home.join("state_5.sqlite");

        assert_eq!(codex_home_from_state_db_path(&state_db), home);
    }
}
