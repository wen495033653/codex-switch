use crate::paths::{codex_dir, ensure_parent_dir};
use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};
use tauri::{path::BaseDirectory, AppHandle, Manager};

pub(crate) const FILE_NAME: &str = "gpt-unrestricted.md";
pub(crate) const CONFIG_KEY: &str = "model_instructions_file";
pub(crate) const SETTING_KEY: &str = "codex_model_instructions_enabled";

pub(crate) fn resolve_model_instructions_file(app: &AppHandle) -> Result<String, String> {
    let target = user_model_instructions_file()?;
    initialize_model_instructions_file(&target, || bundled_model_instructions_file(app))?;
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
    Ok(model_instructions_file_in_codex_home(&codex_dir()?))
}

fn model_instructions_file_in_codex_home(codex_home: &Path) -> PathBuf {
    codex_home.join(FILE_NAME)
}

// 用户文件是权威来源；安装包资源只用于首次初始化，不能覆盖用户更新。
fn initialize_model_instructions_file(
    target: &Path,
    source: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<(), String> {
    match fs::metadata(target) {
        Ok(metadata) if metadata.is_file() => return Ok(()),
        Ok(_) => return Err(format!("模型指令路径不是文件: {}", target.display())),
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => {
            return Err(format!(
                "读取模型指令文件状态失败 {}: {err}",
                target.display()
            ))
        }
    }
    let source = source()?;
    let mut input = fs::File::open(&source)
        .map_err(|err| format!("读取模型指令资源失败 {}: {err}", source.display()))?;
    ensure_parent_dir(target)?;
    let mut output = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
    {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::AlreadyExists && target.is_file() => return Ok(()),
        Err(err) => return Err(format!("创建模型指令文件失败 {}: {err}", target.display())),
    };
    io::copy(&mut input, &mut output)
        .map(|_| ())
        .map_err(|err| {
            format!(
                "初始化模型指令文件失败 {} -> {}: {err}",
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
            path_for_config(Path::new(r"\\?\C:\CodexHome\gpt-unrestricted.md")),
            "C:/CodexHome/gpt-unrestricted.md"
        );
        assert_eq!(
            path_for_config(Path::new(r"C:\CodexHome\gpt-unrestricted.md")),
            "C:/CodexHome/gpt-unrestricted.md"
        );
    }

    #[test]
    fn user_model_instructions_file_stays_in_codex_home() {
        let codex_home = PathBuf::from(".codex");
        let target = model_instructions_file_in_codex_home(&codex_home);

        assert_eq!(target, codex_home.join("gpt-unrestricted.md"));
    }
    fn fixture() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "codex-switch-instructions-{}",
            crate::accounts::random_urlsafe(12)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn preserves_user_bytes_without_resolving_bundle() {
        let target = fixture().join(FILE_NAME);
        let content = "用户更新\r\nlocal instructions\r\n";
        fs::write(&target, content).unwrap();
        initialize_model_instructions_file(&target, || panic!("must not read bundle")).unwrap();
        assert_eq!(fs::read_to_string(target).unwrap(), content);
    }

    #[test]
    fn initializes_missing_file_once() {
        let dir = fixture();
        let source = dir.join("bundle.md");
        let target = dir.join("home").join(FILE_NAME);
        fs::write(&source, "bundled instructions").unwrap();
        initialize_model_instructions_file(&target, || Ok(source.clone())).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "bundled instructions");
        fs::write(&target, "user update").unwrap();
        initialize_model_instructions_file(&target, || Ok(source)).unwrap();
        assert_eq!(fs::read_to_string(target).unwrap(), "user update");
    }

    #[test]
    fn rejects_directory_and_missing_resource() {
        let dir = fixture();
        assert!(initialize_model_instructions_file(&dir, || unreachable!())
            .unwrap_err()
            .contains("不是文件"));
        let target = dir.join(FILE_NAME);
        assert!(
            initialize_model_instructions_file(&target, || Ok(dir.join("missing.md")))
                .unwrap_err()
                .contains("读取模型指令资源失败")
        );
        assert!(!target.exists());
    }
}
