use crate::json_util::string_field;
#[cfg(target_os = "macos")]
use crate::paths::home_dir;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[cfg(windows)]
const WINDOWS_CODEX_DESKTOP_EXECUTABLE_NAME: &str = "ChatGPT.exe";

#[cfg(windows)]
const WINDOWS_LEGACY_CODEX_DESKTOP_EXECUTABLE_NAME: &str = "Codex.exe";

#[cfg(target_os = "macos")]
const MACOS_CODEX_DESKTOP_APP_NAMES: [&str; 2] = ["ChatGPT.app", "Codex.app"];

#[cfg(target_os = "macos")]
const MACOS_CODEX_DESKTOP_EXECUTABLE_NAME: &str = "ChatGPT";

#[cfg(target_os = "macos")]
const MACOS_LEGACY_CODEX_DESKTOP_APP_NAME: &str = "Codex.app";

#[cfg(target_os = "macos")]
const MACOS_LEGACY_CODEX_DESKTOP_EXECUTABLE_NAME: &str = "Codex";

#[cfg(any(windows, test))]
const CODEX_APP_PACKAGE_FAMILY_SUFFIX: &str = "__2p2nqsd0c76g0";

#[cfg(windows)]
const CODEX_APP_PACKAGE_REGISTRY_KEY: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";

pub(super) fn codex_app_executable() -> Result<String, String> {
    let mut candidates = Vec::new();
    extend_unique_paths(&mut candidates, running_codex_app_executables()?);
    extend_unique_paths(
        &mut candidates,
        installed_codex_app_desktop_executable_candidates(),
    );
    candidates
        .into_iter()
        .find(|path| Path::new(path).exists())
        .ok_or_else(|| {
            let status = codex_desktop_support_status();
            string_field(&status, "message")
        })
}

pub(crate) fn codex_desktop_cli_source_path() -> Result<PathBuf, String> {
    let mut desktop_candidates = running_codex_app_executables().unwrap_or_default();
    extend_unique_paths(
        &mut desktop_candidates,
        installed_codex_app_desktop_executable_candidates(),
    );

    let mut cli_candidates = Vec::new();
    for desktop_executable in desktop_candidates {
        let cli_path = codex_desktop_cli_path_for_host(Path::new(&desktop_executable));
        let cli_path = cli_path.to_string_lossy().to_string();
        if cli_candidates
            .iter()
            .any(|candidate: &String| executable_paths_equal(candidate, &cli_path))
        {
            continue;
        }
        cli_candidates.push(cli_path);
    }

    cli_candidates
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .ok_or_else(|| {
            let status = codex_desktop_support_status();
            let message = string_field(&status, "message");
            if message.is_empty() {
                "当前 Codex Desktop 安装缺少 resources/codex 可执行文件，请更新 Codex Desktop"
                    .to_string()
            } else {
                message
            }
        })
}

#[cfg(windows)]
fn codex_desktop_cli_path_for_host(desktop_executable: &Path) -> PathBuf {
    desktop_executable
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("resources")
        .join("codex.exe")
}

#[cfg(target_os = "macos")]
fn codex_desktop_cli_path_for_host(desktop_executable: &Path) -> PathBuf {
    desktop_executable
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new(""))
        .join("Resources")
        .join("codex")
}

#[cfg(not(any(windows, target_os = "macos")))]
fn codex_desktop_cli_path_for_host(_desktop_executable: &Path) -> PathBuf {
    PathBuf::new()
}

fn extend_unique_paths(paths: &mut Vec<String>, candidates: Vec<String>) {
    for candidate in candidates {
        if candidate.trim().is_empty() {
            continue;
        }
        if paths
            .iter()
            .any(|path| executable_paths_equal(path, candidate.trim()))
        {
            continue;
        }
        paths.push(candidate);
    }
}

fn executable_paths_equal(left: &str, right: &str) -> bool {
    if cfg!(windows) || (is_windows_style_path(left) && is_windows_style_path(right)) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn is_windows_style_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.contains('\\')
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/'))
}

fn running_codex_app_executables() -> Result<Vec<String>, String> {
    Ok(
        super::super::codex_app_watcher::refresh_current_codex_app_processes()?
            .iter()
            .map(|process| process.executable_path.trim().to_string())
            .filter(|path| !path.is_empty())
            .collect(),
    )
}

#[cfg(windows)]
fn installed_codex_app_desktop_executable_candidates() -> Vec<String> {
    installed_codex_app_package_names()
        .into_iter()
        .map(|package_name| {
            PathBuf::from(r"C:\Program Files\WindowsApps")
                .join(&package_name)
                .join("app")
                .join(WINDOWS_CODEX_DESKTOP_EXECUTABLE_NAME)
                .to_string_lossy()
                .to_string()
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn installed_codex_app_desktop_executable_candidates() -> Vec<String> {
    MACOS_CODEX_DESKTOP_APP_NAMES
        .iter()
        .flat_map(|app_name| {
            macos_app_executable_candidates(app_name, MACOS_CODEX_DESKTOP_EXECUTABLE_NAME)
        })
        .collect()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn installed_codex_app_desktop_executable_candidates() -> Vec<String> {
    Vec::new()
}

#[cfg(windows)]
fn installed_legacy_codex_app_desktop_executable_candidates() -> Vec<String> {
    let packages = installed_codex_app_package_names();
    installed_executable_candidates_for_packages(
        &packages,
        WINDOWS_LEGACY_CODEX_DESKTOP_EXECUTABLE_NAME,
    )
}

#[cfg(target_os = "macos")]
fn installed_legacy_codex_app_desktop_executable_candidates() -> Vec<String> {
    macos_app_executable_candidates(
        MACOS_LEGACY_CODEX_DESKTOP_APP_NAME,
        MACOS_LEGACY_CODEX_DESKTOP_EXECUTABLE_NAME,
    )
}

#[cfg(not(any(windows, target_os = "macos")))]
fn installed_legacy_codex_app_desktop_executable_candidates() -> Vec<String> {
    Vec::new()
}

pub(crate) fn codex_desktop_support_status() -> Value {
    if !cfg!(any(windows, target_os = "macos")) {
        return json!({
            "status": "unsupported",
            "supported": false,
            "requiresUpdate": false,
            "executable": Value::Null,
            "message": "当前系统暂不支持 ChatGPT Desktop 集成"
        });
    }
    let mut current = running_codex_app_executables().unwrap_or_default();
    extend_unique_paths(
        &mut current,
        installed_codex_app_desktop_executable_candidates(),
    );
    let legacy = installed_legacy_codex_app_desktop_executable_candidates();
    codex_desktop_support_status_from_candidates(&current, &legacy)
}

#[cfg(windows)]
fn installed_executable_candidates_for_packages(
    packages: &[String],
    executable_name: &str,
) -> Vec<String> {
    packages
        .iter()
        .map(|package_name| {
            PathBuf::from(r"C:\Program Files\WindowsApps")
                .join(package_name)
                .join("app")
                .join(executable_name)
                .to_string_lossy()
                .to_string()
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn macos_app_executable_candidates(app_name: &str, executable_name: &str) -> Vec<String> {
    let mut application_roots = Vec::new();
    if let Ok(home) = home_dir() {
        application_roots.push(home.join("Applications"));
    }
    application_roots.push(PathBuf::from("/Applications"));

    application_roots
        .into_iter()
        .map(|root| {
            root.join(app_name)
                .join("Contents")
                .join("MacOS")
                .join(executable_name)
                .to_string_lossy()
                .to_string()
        })
        .collect()
}

fn codex_desktop_support_status_from_candidates(current: &[String], legacy: &[String]) -> Value {
    if let Some(executable) = current.iter().find(|path| Path::new(path).exists()) {
        return json!({
            "status": "current",
            "supported": true,
            "requiresUpdate": false,
            "executable": executable
        });
    }
    if let Some(executable) = legacy.iter().find(|path| Path::new(path).exists()) {
        return json!({
            "status": "legacy",
            "supported": false,
            "requiresUpdate": true,
            "executable": executable,
            "message": "当前 Codex Desktop 版本过旧，请更新到集成 Codex 的新版 ChatGPT Desktop"
        });
    }
    json!({
        "status": "missing",
        "supported": false,
        "requiresUpdate": true,
        "executable": Value::Null,
        "message": "未找到新版 ChatGPT Desktop，请安装或更新后再使用 Codex 功能"
    })
}

#[cfg(windows)]
fn installed_codex_app_package_names() -> Vec<String> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS},
        System::Registry::{
            RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
        },
    };

    let mut key: HKEY = null_mut();
    let key_name = wide_null(CODEX_APP_PACKAGE_REGISTRY_KEY);
    let open_result =
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, key_name.as_ptr(), 0, KEY_READ, &mut key) };
    if open_result != ERROR_SUCCESS {
        return Vec::new();
    }

    let mut packages = Vec::new();
    let mut index = 0u32;
    loop {
        let mut name = vec![0u16; 512];
        let mut len = name.len() as u32;
        let result = unsafe {
            RegEnumKeyExW(
                key,
                index,
                name.as_mut_ptr(),
                &mut len,
                null(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if result == ERROR_NO_MORE_ITEMS {
            break;
        }
        if result == ERROR_SUCCESS {
            let package = String::from_utf16_lossy(&name[..len as usize]);
            if is_codex_app_package_name(&package) {
                packages.push(package);
            }
        } else if result != ERROR_MORE_DATA {
            break;
        }
        index += 1;
    }

    unsafe {
        RegCloseKey(key);
    }
    packages.sort_by(|left, right| right.cmp(left));
    packages
}

#[cfg(any(windows, test))]
fn is_codex_app_package_name(name: &str) -> bool {
    name.starts_with("OpenAI.Codex_") && name.ends_with(CODEX_APP_PACKAGE_FAMILY_SUFFIX)
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn extend_unique_paths_preserves_first_candidate_priority() {
        let mut paths = vec![r"C:\ChatGPT\app\ChatGPT.exe".to_string()];

        extend_unique_paths(
            &mut paths,
            vec![
                r"c:\chatgpt\app\chatgpt.exe".to_string(),
                r"C:\ChatGPTPreview\app\ChatGPT.exe".to_string(),
            ],
        );

        assert_eq!(
            paths,
            vec![
                r"C:\ChatGPT\app\ChatGPT.exe".to_string(),
                r"C:\ChatGPTPreview\app\ChatGPT.exe".to_string(),
            ]
        );
    }

    #[test]
    fn codex_app_package_name_detection_matches_appx_identity() {
        assert!(is_codex_app_package_name(
            "OpenAI.Codex_26.623.5175.0_x64__2p2nqsd0c76g0"
        ));
        assert!(!is_codex_app_package_name(
            "OpenAI.Codex_26.623.5175.0_x64__other"
        ));
        assert!(!is_codex_app_package_name(
            "Other.Codex_26.623.5175.0_x64__2p2nqsd0c76g0"
        ));
    }

    #[cfg(windows)]
    #[test]
    fn desktop_executable_candidates_only_include_chatgpt_host() {
        let package = "OpenAI.Codex_26.707.3748.0_x64__2p2nqsd0c76g0";
        let candidates = installed_executable_candidates_for_packages(
            &[package.to_string()],
            WINDOWS_CODEX_DESKTOP_EXECUTABLE_NAME,
        );

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].ends_with("ChatGPT.exe"));
    }

    #[cfg(windows)]
    #[test]
    fn desktop_cli_path_is_resolved_from_windows_resources_directory() {
        let host = Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
        );

        assert_eq!(
            codex_desktop_cli_path_for_host(host),
            PathBuf::from(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__2p2nqsd0c76g0\app\resources\codex.exe"
            )
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn desktop_executable_candidates_include_macos_chatgpt_app() {
        let candidates = macos_app_executable_candidates(
            MACOS_CODEX_DESKTOP_APP_NAMES[0],
            MACOS_CODEX_DESKTOP_EXECUTABLE_NAME,
        );

        assert!(candidates
            .iter()
            .any(|candidate| candidate.ends_with("/ChatGPT.app/Contents/MacOS/ChatGPT")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn desktop_cli_path_is_resolved_from_macos_resources_directory() {
        let host = Path::new("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT");

        assert_eq!(
            codex_desktop_cli_path_for_host(host),
            PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex")
        );
    }

    #[test]
    fn desktop_support_status_requires_current_chatgpt_host() {
        let root = std::env::temp_dir().join(format!(
            "codex-switch-desktop-status-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let current = root.join("ChatGPT.exe");
        let legacy = root.join("Codex.exe");
        fs::write(&legacy, []).unwrap();

        let legacy_status = codex_desktop_support_status_from_candidates(
            &[current.to_string_lossy().to_string()],
            &[legacy.to_string_lossy().to_string()],
        );
        assert_eq!(legacy_status["status"], "legacy");
        assert_eq!(legacy_status["requiresUpdate"], true);

        fs::write(&current, []).unwrap();
        let current_status = codex_desktop_support_status_from_candidates(
            &[current.to_string_lossy().to_string()],
            &[legacy.to_string_lossy().to_string()],
        );
        assert_eq!(current_status["status"], "current");
        assert_eq!(current_status["supported"], true);
        assert!(current_status.get("message").is_none());

        fs::remove_dir_all(root).unwrap();
    }
}
