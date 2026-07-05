use crate::paths::{app_data_dir, ensure_parent_dir};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::{path::BaseDirectory, AppHandle, Manager};

pub(crate) const FILE_NAME: &str = "gpt5.5-unrestricted.md";
pub(crate) const CONFIG_KEY: &str = "model_instructions_file";
pub(crate) const SETTING_KEY: &str = "codex_model_instructions_enabled";

pub(crate) fn resolve_model_instructions_file(app: &AppHandle) -> Result<String, String> {
    let source = bundled_model_instructions_file(app)?;
    let target = user_model_instructions_file()?;
    copy_model_instructions_file(&source, &target)?;
    Ok(path_for_config(&target))
}

fn bundled_model_instructions_file(app: &AppHandle) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Ok(resource_path) = app.path().resolve(FILE_NAME, BaseDirectory::Resource) {
        push_unique_path(&mut candidates, resource_path);
    }

    if cfg!(debug_assertions) {
        push_unique_path(&mut candidates, dev_resource_path());
    }

    for source in &candidates {
        if source.exists() {
            return Ok(source.clone());
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

fn user_model_instructions_file() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join(FILE_NAME))
}

fn copy_model_instructions_file(source: &Path, target: &Path) -> Result<(), String> {
    if source == target {
        return Ok(());
    }
    ensure_parent_dir(target)?;
    fs::copy(source, target).map(|_| ()).map_err(|err| {
        format!(
            "同步模型指令文件失败 {} -> {}: {err}",
            source.display(),
            target.display()
        )
    })
}

fn path_for_config(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("//?/")
        .to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_for_config_uses_normal_slashes_without_extended_prefix() {
        assert_eq!(
            path_for_config(Path::new(
                r"\\?\C:\AppData\Roaming\codex-switch\gpt5.5-unrestricted.md"
            )),
            "C:/AppData/Roaming/codex-switch/gpt5.5-unrestricted.md"
        );
        assert_eq!(
            path_for_config(Path::new(
                r"C:\AppData\Roaming\codex-switch\gpt5.5-unrestricted.md"
            )),
            "C:/AppData/Roaming/codex-switch/gpt5.5-unrestricted.md"
        );
    }

    #[test]
    fn user_model_instructions_file_name_stays_at_app_data_root() {
        let target = PathBuf::from("codex-switch").join(FILE_NAME);

        assert_eq!(
            target,
            PathBuf::from("codex-switch").join("gpt5.5-unrestricted.md")
        );
    }
}
