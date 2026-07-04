use std::path::{Path, PathBuf};
use tauri::{path::BaseDirectory, AppHandle, Manager};

pub(crate) const FILE_NAME: &str = "gpt5.5-unrestricted.md";
pub(crate) const CONFIG_KEY: &str = "model_instructions_file";
pub(crate) const SETTING_KEY: &str = "codex_model_instructions_enabled";

pub(crate) fn resolve_model_instructions_file(app: &AppHandle) -> Result<String, String> {
    let mut candidates = Vec::new();
    if let Ok(resource_path) = app.path().resolve(FILE_NAME, BaseDirectory::Resource) {
        push_unique_path(&mut candidates, resource_path);
    }

    if cfg!(debug_assertions) {
        push_unique_path(&mut candidates, dev_resource_path());
    }

    for source in &candidates {
        if source.exists() {
            return Ok(source.to_string_lossy().replace('\\', "/"));
        }
    }

    let checked = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "模型指令文件不存在，已检查 {checked}；请确认 resources/{FILE_NAME} 已随应用打包"
    ))
}

fn dev_resource_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .join("resources")
        .join(FILE_NAME)
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if paths.iter().any(|item| item == &path) {
        return;
    }
    paths.push(path);
}
