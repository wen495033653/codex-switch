use std::{
    env, fs,
    path::{Path, PathBuf},
};

const APP_DATA_DIR_NAME: &str = "codex-switch";

pub(crate) fn home_dir() -> Result<PathBuf, String> {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
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
