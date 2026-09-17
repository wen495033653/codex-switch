use crate::{
    accounts::{
        find_store_account, profile_id_from_account, read_api_key_from_auth,
        read_api_key_from_provider_config,
    },
    json_file::write_json_file,
    json_util::string_field,
    model_instructions::{
        resolve_model_instructions_file, SETTING_KEY as MODEL_INSTRUCTIONS_ENABLED_SETTING_KEY,
    },
    paths::{app_data_dir, codex_dir},
    session_manager::migrate_legacy_codex_data_for_root,
    session_sync_diagnostics::log_session_sync_event,
    settings::{default_api_mode, read_settings_value},
    time_util::now_string,
};
use desktop_install::codex_app_executable;
use instance_config::{sync_instance_config, InstanceConfig};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::Duration as StdDuration,
};
use tauri::AppHandle;

mod desktop_install;
mod instance_config;

pub(crate) use desktop_install::{codex_desktop_cli_source_path, codex_desktop_support_status};

const CODEX_APP_INSTANCES_DIR: &str = "codex-app-instances";

const MULTI_OPEN_SUPPRESS_SOURCE: &str = "multi_open_target_channel";

struct CodexAppChannel {
    kind: &'static str,
    key: String,
    target_id: String,
    label: String,
    auth: Value,
    config: InstanceConfig,
}

struct CodexAppInstancePaths {
    root: PathBuf,
    codex_home: PathBuf,
    user_data_dir: PathBuf,
}

pub(crate) fn open_codex_app_instance(app: AppHandle, payload: Value) -> Result<Value, String> {
    if !cfg!(any(windows, target_os = "macos")) {
        return Err("Codex 多开目前仅支持 Windows 和 macOS".to_string());
    }

    let target_kind = string_field(&payload, "kind");
    let target_id = string_field(&payload, "id");
    if target_id.is_empty() {
        return Err("Codex 多开目标不能为空".to_string());
    }

    let settings = read_settings_value()?;
    let channel = match target_kind.as_str() {
        "account" => account_channel(&target_id)?,
        "api" => api_channel(&settings, &target_id)?,
        _ => return Err("Codex 多开目标类型无效".to_string()),
    };
    let executable = codex_app_executable()?;
    let paths = prepare_instance_paths(&app, &channel)?;
    let args = vec![format!(
        "--user-data-dir={}",
        paths.user_data_dir.to_string_lossy()
    )];
    let envs = vec![(
        "CODEX_HOME".to_string(),
        paths.codex_home.to_string_lossy().to_string(),
    )];

    log_session_sync_event(
        "codex_app_multi_open_start",
        json!({
            "kind": channel.kind,
            "channel": channel.label,
            "executable": executable,
            "instanceRoot": paths.root.to_string_lossy(),
            "codexHome": paths.codex_home.to_string_lossy(),
            "userDataDir": paths.user_data_dir.to_string_lossy()
        }),
    );

    super::codex_app_watcher::suppress_next_codex_app_open_handler(MULTI_OPEN_SUPPRESS_SOURCE);
    match super::codex_app_open::launch_codex_app_instance_for_current_settings_with_options(
        &executable,
        &args,
        &envs,
    ) {
        Ok(launch) if launch.launched => {
            trigger_instance_legacy_migration(paths.codex_home.clone());
            log_session_sync_event(
                "codex_app_multi_open_finish",
                json!({
                    "kind": channel.kind,
                    "channel": channel.label,
                    "instanceRoot": paths.root.to_string_lossy()
                }),
            );
            Ok(json!({
                "ok": true,
                "message": format!("已用{}打开 Codex", channel.label),
                "kind": channel.kind,
                "targetId": channel.target_id,
                "instanceKey": channel.key,
                "channel": channel.label,
                "instanceRoot": paths.root.to_string_lossy().to_string(),
                "codexHome": paths.codex_home.to_string_lossy().to_string(),
                "userDataDir": paths.user_data_dir.to_string_lossy().to_string()
            }))
        }
        Ok(_) => {
            super::codex_app_watcher::clear_suppressed_codex_app_open_handler(
                MULTI_OPEN_SUPPRESS_SOURCE,
            );
            Err("Codex 可执行路径不存在，无法多开".to_string())
        }
        Err(err) => {
            super::codex_app_watcher::clear_suppressed_codex_app_open_handler(
                MULTI_OPEN_SUPPRESS_SOURCE,
            );
            Err(err)
        }
    }
}

pub(crate) fn show_codex_app_instance(payload: Value) -> Result<Value, String> {
    if !cfg!(any(windows, target_os = "macos")) {
        return Err("Codex 多开目前仅支持 Windows 和 macOS".to_string());
    }

    let target_kind = string_field(&payload, "kind");
    let target_id = string_field(&payload, "id");
    let instance_key = instance_key_for_target(&target_kind, &target_id)?;
    let root = app_data_dir()?
        .join(CODEX_APP_INSTANCES_DIR)
        .join(instance_key);
    let codex_home = root.join("codex-home");
    let user_data_dir = root.join("user-data");
    if !user_data_dir.exists() {
        return Err("独立 Codex 实例不存在，请先打开一次".to_string());
    }

    let processes = super::codex_app_watcher::refresh_current_codex_app_processes()?;
    let pids = instance_pids_for_user_data_dir(&processes, &user_data_dir);
    if pids.is_empty() {
        return Err("独立 Codex 窗口未运行，请重新打开一次".to_string());
    }
    focus_instance_window(&pids)?;
    log_session_sync_event(
        "codex_app_multi_open_show_window",
        json!({
            "kind": target_kind,
            "targetId": target_id,
            "instanceRoot": root.to_string_lossy(),
            "codexHome": codex_home.to_string_lossy(),
            "userDataDir": user_data_dir.to_string_lossy(),
            "pids": pids
        }),
    );
    Ok(json!({
        "ok": true,
        "message": "已打开 Codex 窗口",
        "kind": target_kind,
        "targetId": target_id,
        "instanceRoot": root.to_string_lossy().to_string(),
        "codexHome": codex_home.to_string_lossy().to_string(),
        "userDataDir": user_data_dir.to_string_lossy().to_string()
    }))
}

pub(crate) fn get_codex_app_instance_status() -> Result<Value, String> {
    if !cfg!(any(windows, target_os = "macos")) {
        return Ok(json!({
            "ok": true,
            "instances": []
        }));
    }

    let instances_dir = app_data_dir()?.join(CODEX_APP_INSTANCES_DIR);
    let processes = super::codex_app_watcher::refresh_current_codex_app_processes()?;
    let instances = read_codex_app_instance_statuses(&instances_dir, &processes)?;

    Ok(json!({
        "ok": true,
        "instances": instances
    }))
}

fn instance_key_for_target(kind: &str, target_id: &str) -> Result<String, String> {
    let target_id = target_id.trim();
    if target_id.is_empty() {
        return Err("Codex 多开目标不能为空".to_string());
    }
    match kind {
        "account" => Ok(format!("account-{}", safe_path_segment(target_id))),
        "api" => Ok(format!("api-{}", safe_path_segment(target_id))),
        _ => Err("Codex 多开目标类型无效".to_string()),
    }
}

fn read_codex_app_instance_statuses(
    instances_dir: &Path,
    processes: &[super::CodexProcess],
) -> Result<Vec<Value>, String> {
    if !instances_dir.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(instances_dir).map_err(|err| {
        format!(
            "读取 Codex 多开实例目录失败 {}: {err}",
            instances_dir.display()
        )
    })?;
    let mut instances = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("读取 Codex 多开实例条目失败: {err}"))?;
        let root = entry.path();
        if !root.is_dir() {
            continue;
        }
        let instance_key = root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        if !is_managed_instance_key(&instance_key) {
            continue;
        }

        let marker = read_instance_marker(&root).unwrap_or_else(|_| json!({}));
        let kind = first_non_empty(vec![
            string_field(&marker, "kind"),
            instance_kind_from_key(&instance_key).to_string(),
        ]);
        let channel = string_field(&marker, "channel");
        let target_id = string_field(&marker, "targetId");
        let user_data_dir = root.join("user-data");
        let codex_home = root.join("codex-home");
        let pids = instance_pids_for_user_data_dir(processes, &user_data_dir);
        let target_key = if target_id.is_empty() {
            String::new()
        } else {
            format!("{kind}:{target_id}")
        };

        instances.push(json!({
            "instanceKey": instance_key,
            "kind": kind,
            "targetId": target_id,
            "targetKey": target_key,
            "channel": channel,
            "running": !pids.is_empty(),
            "pids": pids,
            "instanceRoot": root.to_string_lossy().to_string(),
            "codexHome": codex_home.to_string_lossy().to_string(),
            "userDataDir": user_data_dir.to_string_lossy().to_string()
        }));
    }
    instances.sort_by_key(|instance| {
        (
            string_field(instance, "kind"),
            string_field(instance, "instanceKey"),
        )
    });
    Ok(instances)
}

fn read_instance_marker(root: &Path) -> Result<Value, String> {
    let marker_path = root.join("codex-switch-instance.json");
    if !marker_path.exists() {
        return Ok(json!({}));
    }
    let raw = fs::read_to_string(&marker_path).map_err(|err| {
        format!(
            "读取 Codex 多开实例标记失败 {}: {err}",
            marker_path.display()
        )
    })?;
    serde_json::from_str(&raw).map_err(|err| {
        format!(
            "解析 Codex 多开实例标记失败 {}: {err}",
            marker_path.display()
        )
    })
}

fn is_managed_instance_key(instance_key: &str) -> bool {
    instance_key.starts_with("account-") || instance_key.starts_with("api-")
}

fn instance_kind_from_key(instance_key: &str) -> &'static str {
    if instance_key.starts_with("api-") {
        "api"
    } else {
        "account"
    }
}

fn instance_pids_for_user_data_dir(
    processes: &[super::CodexProcess],
    user_data_dir: &Path,
) -> Vec<u64> {
    let mut pids = processes
        .iter()
        .filter(|process| command_line_matches_user_data_dir(&process.command_line, user_data_dir))
        .map(|process| process.pid)
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn command_line_matches_user_data_dir(command_line: &str, user_data_dir: &Path) -> bool {
    let target = normalize_command_path_fragment(&user_data_dir.to_string_lossy());
    if target.is_empty() {
        return false;
    }
    normalize_command_path_fragment(command_line).contains(&target)
}

fn normalize_command_path_fragment(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_matches('"')
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

#[cfg(windows)]
fn focus_instance_window(pids: &[u64]) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
    };

    struct WindowSearch {
        pids: std::collections::HashSet<u32>,
        hwnd: HWND,
    }

    unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> i32 {
        let search = &mut *(lparam as *mut WindowSearch);
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let mut window_pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut window_pid);
        if search.pids.contains(&window_pid) {
            search.hwnd = hwnd;
            return 0;
        }
        1
    }

    let pids = pids
        .iter()
        .filter_map(|pid| u32::try_from(*pid).ok())
        .collect::<std::collections::HashSet<_>>();
    if pids.is_empty() {
        return Err("独立 Codex 窗口未运行，请重新打开一次".to_string());
    }

    let mut search = WindowSearch {
        pids,
        hwnd: std::ptr::null_mut(),
    };
    unsafe {
        EnumWindows(
            Some(enum_windows_proc),
            &mut search as *mut WindowSearch as LPARAM,
        );
    }
    if search.hwnd.is_null() {
        return Err("未找到独立 Codex 的可见窗口".to_string());
    }

    unsafe {
        if IsIconic(search.hwnd) != 0 {
            ShowWindow(search.hwnd, SW_RESTORE);
        } else {
            ShowWindow(search.hwnd, SW_SHOW);
        }
        BringWindowToTop(search.hwnd);
        if SetForegroundWindow(search.hwnd) == 0 {
            return Err("独立 Codex 窗口激活失败".to_string());
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn focus_instance_window(pids: &[u64]) -> Result<(), String> {
    for pid in pids {
        let script = format!(
            "tell application \"System Events\" to set frontmost of first process whose unix id is {pid} to true"
        );
        let status = std::process::Command::new("osascript")
            .args(["-e", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if status.is_ok_and(|status| status.success()) {
            return Ok(());
        }
    }
    for app_name in ["ChatGPT", "Codex"] {
        let status = std::process::Command::new("open")
            .args(["-a", app_name])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if status.is_ok_and(|status| status.success()) {
            return Ok(());
        }
    }
    Err("未能激活独立 Codex 窗口".to_string())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn focus_instance_window(_pids: &[u64]) -> Result<(), String> {
    Err("当前系统不支持 Codex 多开".to_string())
}

fn account_channel(profile_id: &str) -> Result<CodexAppChannel, String> {
    let account = find_store_account(profile_id)?;
    let resolved_profile_id = profile_id_from_account(&account)?;
    let tokens = account
        .get("tokens")
        .cloned()
        .ok_or_else(|| "账号缺少 tokens".to_string())?;
    Ok(CodexAppChannel {
        kind: "account",
        key: format!("account-{}", safe_path_segment(&resolved_profile_id)),
        target_id: resolved_profile_id.clone(),
        label: format!("订阅账号 {}", compact_id(&resolved_profile_id)),
        auth: json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": Value::Null,
            "tokens": tokens,
            "last_refresh": now_string()
        }),
        config: InstanceConfig::Subscription,
    })
}

fn api_channel(settings: &Value, profile_id: &str) -> Result<CodexAppChannel, String> {
    let profile = find_api_profile(settings, profile_id)?;
    let resolved_profile_id = string_field(&profile, "id");
    let base_url = string_field(&profile, "base_url");
    if base_url.is_empty() {
        return Err("API Base URL 不能为空".to_string());
    }
    let api_key = api_key_for_profile(settings, &profile)?;
    if api_key.is_empty() {
        return Err("API Key 不能为空".to_string());
    }

    let display = first_non_empty(vec![
        string_field(&profile, "name"),
        resolved_profile_id.clone(),
        "default".to_string(),
    ]);
    Ok(CodexAppChannel {
        kind: "api",
        key: format!("api-{}", safe_path_segment(&resolved_profile_id)),
        target_id: resolved_profile_id,
        label: format!("API {display}"),
        auth: json!({
            "auth_mode": "apikey",
            "OPENAI_API_KEY": api_key
        }),
        config: InstanceConfig::Api { base_url },
    })
}

fn find_api_profile(settings: &Value, profile_id: &str) -> Result<Value, String> {
    if let Some(profile) = settings
        .get("api_profiles")
        .and_then(Value::as_array)
        .and_then(|profiles| {
            profiles
                .iter()
                .find(|profile| string_field(profile, "id") == profile_id)
        })
    {
        return Ok(profile.clone());
    }

    let active_profile = settings
        .get("api_mode")
        .cloned()
        .unwrap_or_else(default_api_mode);
    if string_field(&active_profile, "id") == profile_id {
        return Ok(active_profile);
    }

    Err("API 配置不存在".to_string())
}

fn api_key_for_profile(settings: &Value, profile: &Value) -> Result<String, String> {
    let api_key = string_field(profile, "api_key");
    if !api_key.is_empty() {
        return Ok(api_key);
    }

    let profile_id = string_field(profile, "id");
    if profile_id != string_field(settings, "active_api_profile_id") {
        return Err("该 API 配置没有保存 API Key".to_string());
    }

    let provider_key = read_api_key_from_provider_config();
    if !provider_key.trim().is_empty() {
        return Ok(provider_key.trim().to_string());
    }

    Ok(read_api_key_from_auth().trim().to_string())
}

fn prepare_instance_paths(
    app: &AppHandle,
    channel: &CodexAppChannel,
) -> Result<CodexAppInstancePaths, String> {
    let root = app_data_dir()?
        .join(CODEX_APP_INSTANCES_DIR)
        .join(&channel.key);
    let codex_home = root.join("codex-home");
    let user_data_dir = root.join("user-data");

    fs::create_dir_all(&codex_home)
        .map_err(|err| format!("创建 Codex 多开 home 失败 {}: {err}", codex_home.display()))?;
    fs::create_dir_all(&user_data_dir).map_err(|err| {
        format!(
            "创建 Codex 多开 user-data 失败 {}: {err}",
            user_data_dir.display()
        )
    })?;
    let migration_report = migrate_legacy_codex_data_for_root(&codex_home)?;
    log_session_sync_event("codex_app_instance_data_migration", migration_report);
    sync_instance_codex_home(app, &codex_home, channel)?;
    write_instance_marker(&root, channel)?;

    Ok(CodexAppInstancePaths {
        root,
        codex_home,
        user_data_dir,
    })
}

fn trigger_instance_legacy_migration(codex_home: PathBuf) {
    thread::spawn(move || {
        for delay in [2, 5] {
            thread::sleep(StdDuration::from_secs(delay));
            match migrate_legacy_codex_data_for_root(&codex_home) {
                Ok(report) => {
                    let completed = report.get("completed").and_then(Value::as_bool) == Some(true);
                    log_session_sync_event("codex_app_instance_data_migration", report);
                    if completed {
                        break;
                    }
                }
                Err(err) => {
                    log_session_sync_event(
                        "codex_app_instance_data_migration_error",
                        json!({
                            "codexHome": codex_home.to_string_lossy(),
                            "error": err
                        }),
                    );
                    break;
                }
            }
        }
    });
}

fn sync_instance_codex_home(
    app: &AppHandle,
    target_home: &Path,
    channel: &CodexAppChannel,
) -> Result<(), String> {
    let source_home = codex_dir()?;
    let settings = read_settings_value()?;
    let model_instructions_file = if model_instructions_enabled(&settings) {
        Some(resolve_model_instructions_file(app)?)
    } else {
        None
    };
    write_json_file(
        &target_home.join("auth.json"),
        "实例 auth.json",
        &channel.auth,
    )?;
    sync_instance_config(
        &target_home.join("config.toml"),
        &channel.config,
        model_instructions_file.as_deref(),
    )?;

    copy_optional_file(&source_home.join(".env"), &target_home.join(".env"))?;
    copy_optional_file(
        &source_home.join("AGENTS.md"),
        &target_home.join("AGENTS.md"),
    )?;
    Ok(())
}

fn model_instructions_enabled(settings: &Value) -> bool {
    settings
        .get(MODEL_INSTRUCTIONS_ENABLED_SETTING_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn write_instance_marker(root: &Path, channel: &CodexAppChannel) -> Result<(), String> {
    let marker = json!({
        "managedBy": "codex-switch",
        "updatedAt": now_string(),
        "kind": channel.kind,
        "targetId": channel.target_id,
        "instanceKey": channel.key,
        "channel": channel.label
    });
    write_json_file(
        &root.join("codex-switch-instance.json"),
        "Codex 多开实例标记",
        &marker,
    )
}

fn copy_optional_file(source: &Path, target: &Path) -> Result<(), String> {
    if !source.exists() {
        return Ok(());
    }
    copy_file(source, target)
}

fn copy_file(source: &Path, target: &Path) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("创建 Codex 多开文件目录失败 {}: {err}", parent.display()))?;
    }
    fs::copy(source, target).map(|_| ()).map_err(|err| {
        format!(
            "复制 Codex 多开文件失败 {} -> {}: {err}",
            source.display(),
            target.display()
        )
    })
}

fn first_non_empty(values: Vec<String>) -> String {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

fn compact_id(value: &str) -> String {
    let text = value.trim();
    if text.chars().count() <= 14 {
        return text.to_string();
    }
    let prefix: String = text.chars().take(8).collect();
    let suffix: String = text
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}...{suffix}")
}

fn safe_path_segment(value: &str) -> String {
    let source = value.trim();
    let mut output = String::new();
    let mut last_dash = false;
    for ch in source.chars() {
        let next = if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            ch.to_ascii_lowercase()
        } else {
            '-'
        };
        if next == '-' {
            if last_dash {
                continue;
            }
            last_dash = true;
        } else {
            last_dash = false;
        }
        output.push(next);
        if output.len() >= 80 {
            break;
        }
    }
    let trimmed = output.trim_matches('-').to_string();
    if trimmed.is_empty() {
        format!("channel-{}", stable_hex_hash(source))
    } else {
        trimmed
    }
}

fn stable_hex_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_path_segment_keeps_ascii_and_collapses_separators() {
        assert_eq!(safe_path_segment(" API/Profile: Main "), "api-profile-main");
        assert!(safe_path_segment("中文账号").starts_with("channel-"));
    }

    #[test]
    fn instance_key_for_target_prefixes_kind() {
        assert_eq!(
            instance_key_for_target("account", " API/Profile: Main ").unwrap(),
            "account-api-profile-main"
        );
        assert_eq!(
            instance_key_for_target("api", " API/Profile: Main ").unwrap(),
            "api-api-profile-main"
        );
        assert!(instance_key_for_target("other", "target").is_err());
    }

    #[test]
    fn compact_id_keeps_short_ids_and_masks_long_ids() {
        assert_eq!(compact_id("short-id"), "short-id");
        assert_eq!(compact_id("account-1234567890"), "account-...7890");
    }

    #[test]
    fn command_line_matches_instance_user_data_dir() {
        let user_data_dir = PathBuf::from(r"C:\Instances\codex-app-instances\api-main\user-data");

        assert!(command_line_matches_user_data_dir(
            r#""C:\ChatGPT\ChatGPT.exe" --user-data-dir="C:\Instances\codex-app-instances\api-main\user-data""#,
            &user_data_dir
        ));
        assert!(command_line_matches_user_data_dir(
            r#""C:\ChatGPT\ChatGPT.exe" --user-data-dir=C:/Instances/codex-app-instances/api-main/user-data"#,
            &user_data_dir
        ));
        assert!(!command_line_matches_user_data_dir(
            r#""C:\ChatGPT\ChatGPT.exe" --user-data-dir=C:\Instances\Codex\web\Codex"#,
            &user_data_dir
        ));
    }
}
