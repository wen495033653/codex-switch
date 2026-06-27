use crate::{
    accounts::{
        find_store_account, profile_id_from_account, read_api_key_from_auth,
        read_api_key_from_provider_config,
    },
    api_config::API_PROVIDER_ID,
    json_file::write_json_file,
    json_util::string_field,
    paths::{app_data_dir, codex_dir},
    session_sync_diagnostics::log_session_sync_event,
    settings::{default_api_mode, read_settings_value},
    time_util::now_string,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

const CODEX_APP_INSTANCES_DIR: &str = "codex-app-instances";
const MULTI_OPEN_SUPPRESS_SOURCE: &str = "multi_open_target_channel";
const API_WIRE_RESPONSES: &str = "responses";
const WINDOWS_SANDBOX_MODE: &str = "elevated";
#[cfg(any(windows, test))]
const CODEX_APP_PACKAGE_FAMILY_SUFFIX: &str = "__2p2nqsd0c76g0";
#[cfg(windows)]
const CODEX_APP_PACKAGE_REGISTRY_KEY: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";

struct CodexAppChannel {
    kind: &'static str,
    key: String,
    target_id: String,
    label: String,
    auth: Value,
    config: InstanceConfig,
}

enum InstanceConfig {
    Subscription,
    Api { base_url: String },
}

struct CodexAppInstancePaths {
    root: PathBuf,
    codex_home: PathBuf,
    user_data_dir: PathBuf,
}

pub(crate) fn open_codex_app_instance(payload: Value) -> Result<Value, String> {
    if !cfg!(windows) {
        return Err("Codex app 多开目前仅支持 Windows".to_string());
    }

    let target_kind = string_field(&payload, "kind");
    let target_id = string_field(&payload, "id");
    if target_id.is_empty() {
        return Err("Codex app 多开目标不能为空".to_string());
    }

    let settings = read_settings_value()?;
    let channel = match target_kind.as_str() {
        "account" => account_channel(&target_id)?,
        "api" => api_channel(&settings, &target_id)?,
        _ => return Err("Codex app 多开目标类型无效".to_string()),
    };
    let executable = codex_app_executable()?;
    let paths = prepare_instance_paths(&channel)?;
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
            let hook_warning = launch.hook_warning.clone();
            let message = if hook_warning.is_some() {
                format!(
                    "已用{}打开 Codex app；hook 注入失败，增强功能可能未生效",
                    channel.label
                )
            } else {
                format!("已用{}打开 Codex app", channel.label)
            };
            log_session_sync_event(
                "codex_app_multi_open_finish",
                json!({
                    "kind": channel.kind,
                    "channel": channel.label,
                    "instanceRoot": paths.root.to_string_lossy(),
                    "hookWarning": hook_warning.clone()
                }),
            );
            Ok(json!({
                "ok": true,
                "message": message,
                "kind": channel.kind,
                "targetId": channel.target_id,
                "instanceKey": channel.key,
                "channel": channel.label,
                "instanceRoot": paths.root.to_string_lossy().to_string(),
                "codexHome": paths.codex_home.to_string_lossy().to_string(),
                "userDataDir": paths.user_data_dir.to_string_lossy().to_string(),
                "hookWarning": hook_warning
            }))
        }
        Ok(_) => {
            super::codex_app_watcher::clear_suppressed_codex_app_open_handler(
                MULTI_OPEN_SUPPRESS_SOURCE,
            );
            Err("Codex app 可执行路径不存在，无法多开".to_string())
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
    if !cfg!(windows) {
        return Err("Codex app 多开目前仅支持 Windows".to_string());
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
        return Err("独立 Codex app 实例不存在，请先打开一次".to_string());
    }

    let processes = super::codex_app_watcher::refresh_current_codex_app_processes()?;
    let pids = instance_pids_for_user_data_dir(&processes, &user_data_dir);
    if pids.is_empty() {
        return Err("独立 Codex app 窗口未运行，请重新打开一次".to_string());
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
        "message": "已打开 Codex app 窗口",
        "kind": target_kind,
        "targetId": target_id,
        "instanceRoot": root.to_string_lossy().to_string(),
        "codexHome": codex_home.to_string_lossy().to_string(),
        "userDataDir": user_data_dir.to_string_lossy().to_string()
    }))
}

pub(crate) fn get_codex_app_instance_status() -> Result<Value, String> {
    if !cfg!(windows) {
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
        return Err("Codex app 多开目标不能为空".to_string());
    }
    match kind {
        "account" => Ok(format!("account-{}", safe_path_segment(target_id))),
        "api" => Ok(format!("api-{}", safe_path_segment(target_id))),
        _ => Err("Codex app 多开目标类型无效".to_string()),
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
            "读取 Codex app 多开实例目录失败 {}: {err}",
            instances_dir.display()
        )
    })?;
    let mut instances = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("读取 Codex app 多开实例条目失败: {err}"))?;
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
            "读取 Codex app 多开实例标记失败 {}: {err}",
            marker_path.display()
        )
    })?;
    serde_json::from_str(&raw).map_err(|err| {
        format!(
            "解析 Codex app 多开实例标记失败 {}: {err}",
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
        .replace('/', "\\")
        .trim_matches('"')
        .trim_end_matches('\\')
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
        return Err("独立 Codex app 窗口未运行，请重新打开一次".to_string());
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
        return Err("未找到独立 Codex app 的可见窗口".to_string());
    }

    unsafe {
        if IsIconic(search.hwnd) != 0 {
            ShowWindow(search.hwnd, SW_RESTORE);
        } else {
            ShowWindow(search.hwnd, SW_SHOW);
        }
        BringWindowToTop(search.hwnd);
        if SetForegroundWindow(search.hwnd) == 0 {
            return Err("独立 Codex app 窗口激活失败".to_string());
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn focus_instance_window(_pids: &[u64]) -> Result<(), String> {
    Err("Codex app 多开目前仅支持 Windows".to_string())
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

fn codex_app_executable() -> Result<String, String> {
    let mut candidates = Vec::new();
    extend_unique_paths(&mut candidates, running_codex_app_executables()?);
    extend_unique_paths(
        &mut candidates,
        installed_codex_app_desktop_executable_candidates(),
    );
    candidates
        .into_iter()
        .find(|path| Path::new(path).exists())
        .ok_or_else(|| "未找到 Codex app 桌面入口，请确认已安装 Codex app".to_string())
}

fn extend_unique_paths(paths: &mut Vec<String>, candidates: Vec<String>) {
    for candidate in candidates {
        if candidate.trim().is_empty() {
            continue;
        }
        if paths
            .iter()
            .any(|path| path.eq_ignore_ascii_case(candidate.trim()))
        {
            continue;
        }
        paths.push(candidate);
    }
}

fn running_codex_app_executables() -> Result<Vec<String>, String> {
    Ok(
        super::codex_app_watcher::refresh_current_codex_app_processes()?
            .iter()
            .map(|process| process.executable_path.trim().to_string())
            .filter(|path| !path.is_empty())
            .collect(),
    )
}

fn installed_codex_app_desktop_executable_candidates() -> Vec<String> {
    installed_codex_app_package_names()
        .into_iter()
        .map(|package_name| {
            PathBuf::from(r"C:\Program Files\WindowsApps")
                .join(package_name)
                .join("app")
                .join("Codex.exe")
                .to_string_lossy()
                .to_string()
        })
        .collect()
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

#[cfg(not(windows))]
fn installed_codex_app_package_names() -> Vec<String> {
    Vec::new()
}

#[cfg(any(windows, test))]
fn is_codex_app_package_name(name: &str) -> bool {
    name.starts_with("OpenAI.Codex_") && name.ends_with(CODEX_APP_PACKAGE_FAMILY_SUFFIX)
}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn prepare_instance_paths(channel: &CodexAppChannel) -> Result<CodexAppInstancePaths, String> {
    let root = app_data_dir()?
        .join(CODEX_APP_INSTANCES_DIR)
        .join(&channel.key);
    let codex_home = root.join("codex-home");
    let user_data_dir = root.join("user-data");

    fs::create_dir_all(&codex_home).map_err(|err| {
        format!(
            "创建 Codex app 多开 home 失败 {}: {err}",
            codex_home.display()
        )
    })?;
    fs::create_dir_all(&user_data_dir).map_err(|err| {
        format!(
            "创建 Codex app 多开 user-data 失败 {}: {err}",
            user_data_dir.display()
        )
    })?;
    sync_instance_codex_home(&codex_home, channel)?;
    write_instance_marker(&root, channel)?;

    Ok(CodexAppInstancePaths {
        root,
        codex_home,
        user_data_dir,
    })
}

fn sync_instance_codex_home(target_home: &Path, channel: &CodexAppChannel) -> Result<(), String> {
    let source_home = codex_dir()?;
    write_json_file(
        &target_home.join("auth.json"),
        "实例 auth.json",
        &channel.auth,
    )?;
    sync_instance_config(&target_home.join("config.toml"), &channel.config)?;

    copy_optional_file(&source_home.join(".env"), &target_home.join(".env"))?;
    copy_optional_file(
        &source_home.join("AGENTS.md"),
        &target_home.join("AGENTS.md"),
    )?;
    Ok(())
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
        "Codex app 多开实例标记",
        &marker,
    )
}

fn sync_instance_config(config_path: &Path, config: &InstanceConfig) -> Result<(), String> {
    let lines = read_instance_config_lines(config_path)?;
    let next_lines = merge_instance_config_lines(&lines, config);
    write_instance_config_lines(config_path, &next_lines)
}

fn read_instance_config_lines(config_path: &Path) -> Result<Vec<String>, String> {
    if !config_path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(config_path)
        .map_err(|err| format!("读取实例 config.toml 失败 {}: {err}", config_path.display()))?;
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    Ok(raw.lines().map(|line| line.to_string()).collect())
}

fn write_instance_config_lines(config_path: &Path, lines: &[String]) -> Result<(), String> {
    let mut raw = lines.join("\n");
    raw.push('\n');
    fs::write(config_path, raw)
        .map_err(|err| format!("写入实例 config.toml 失败 {}: {err}", config_path.display()))
}

fn merge_instance_config_lines(lines: &[String], config: &InstanceConfig) -> Vec<String> {
    let api_provider_table = format!("model_providers.{API_PROVIDER_ID}");
    let mut next_lines = match config {
        InstanceConfig::Subscription => {
            let lines = remove_root_config_entries(
                lines,
                &[
                    "model_provider",
                    "preferred_auth_method",
                    "forced_login_method",
                    "openai_base_url",
                ],
            );
            let lines = upsert_root_config_entries(
                &lines,
                vec![("cli_auth_credentials_store", toml_string("file"))],
            );
            remove_table_lines(&lines, &api_provider_table)
        }
        InstanceConfig::Api { base_url } => {
            let lines = remove_root_config_entries(
                lines,
                &[
                    "preferred_auth_method",
                    "forced_login_method",
                    "openai_base_url",
                ],
            );
            let lines = upsert_root_config_entries(
                &lines,
                vec![
                    ("model_provider", toml_string(API_PROVIDER_ID)),
                    ("cli_auth_credentials_store", toml_string("file")),
                ],
            );
            set_table_config_entries(
                &lines,
                &api_provider_table,
                vec![
                    ("name", toml_string(API_PROVIDER_ID)),
                    ("base_url", toml_string(base_url)),
                    ("wire_api", toml_string(API_WIRE_RESPONSES)),
                    ("supports_websockets", "false".to_string()),
                    ("requires_openai_auth", "true".to_string()),
                ],
            )
        }
    };

    next_lines = upsert_table_config_entries(
        &next_lines,
        "windows",
        vec![("sandbox", toml_string(WINDOWS_SANDBOX_MODE))],
    );
    normalize_blank_lines(&next_lines)
}

fn toml_string(value: &str) -> String {
    format_toml_string(value)
}

fn format_toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn find_root_table_index(lines: &[String]) -> Option<usize> {
    lines.iter().position(|line| {
        let normalized = line.trim();
        normalized.starts_with('[') && normalized.ends_with(']')
    })
}

fn root_assignment(line: &str) -> Option<(String, String)> {
    let normalized = line.trim();
    if normalized.is_empty() || normalized.starts_with('#') || normalized.starts_with('[') {
        return None;
    }
    let (key, value) = normalized.split_once('=')?;
    let key = key.trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
    {
        return None;
    }
    Some((key.to_string(), value.trim().to_string()))
}

fn table_bounds(lines: &[String], table_name: &str) -> Option<(usize, usize)> {
    let header = format!("[{table_name}]");
    let start = lines.iter().position(|line| line.trim() == header)?;
    let mut end = lines.len();
    for (index, line) in lines.iter().enumerate().skip(start + 1) {
        let normalized = line.trim();
        if normalized.starts_with('[') && normalized.ends_with(']') {
            end = index;
            break;
        }
    }
    Some((start, end))
}

fn remove_root_config_entries(lines: &[String], keys: &[&str]) -> Vec<String> {
    let root_end = find_root_table_index(lines).unwrap_or(lines.len());
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            if index < root_end {
                if let Some((key, _value)) = root_assignment(line) {
                    if keys.iter().any(|target| *target == key) {
                        return None;
                    }
                }
            }
            Some(line.clone())
        })
        .collect()
}

fn upsert_root_config_entries(lines: &[String], values: Vec<(&str, String)>) -> Vec<String> {
    let root_end = find_root_table_index(lines).unwrap_or(lines.len());
    let mut pending = values;
    let mut next_lines = Vec::with_capacity(lines.len() + pending.len() + 2);

    for (index, line) in lines.iter().enumerate() {
        if index < root_end {
            if let Some((key, _value)) = root_assignment(line) {
                if let Some(pending_index) = pending
                    .iter()
                    .position(|(pending_key, _)| *pending_key == key)
                {
                    let (_pending_key, pending_value) = pending.remove(pending_index);
                    next_lines.push(format!("{key} = {pending_value}"));
                    continue;
                }
            }
        }
        next_lines.push(line.clone());
    }

    if pending.is_empty() {
        return next_lines;
    }

    let insert_at = root_end;
    let mut insert_lines: Vec<String> = pending
        .into_iter()
        .map(|(key, value)| format!("{key} = {value}"))
        .collect();
    if insert_at > 0
        && next_lines
            .get(insert_at - 1)
            .is_some_and(|line| !line.trim().is_empty())
    {
        insert_lines.insert(0, String::new());
    }
    if insert_at < next_lines.len()
        && next_lines
            .get(insert_at)
            .is_some_and(|line| !line.trim().is_empty())
    {
        insert_lines.push(String::new());
    }
    next_lines.splice(insert_at..insert_at, insert_lines);
    next_lines
}

fn remove_table_lines(lines: &[String], table_name: &str) -> Vec<String> {
    let Some((start, end)) = table_bounds(lines, table_name) else {
        return lines.to_vec();
    };
    let mut next_lines = lines.to_vec();
    next_lines.splice(start..end, std::iter::empty());
    normalize_blank_lines(&next_lines)
}

fn set_table_config_entries(
    lines: &[String],
    table_name: &str,
    values: Vec<(&str, String)>,
) -> Vec<String> {
    let lines = remove_table_lines(lines, table_name);
    let insert_at = find_root_table_index(&lines).unwrap_or(lines.len());
    let mut table_lines = Vec::new();
    if insert_at > 0
        && lines
            .get(insert_at - 1)
            .is_some_and(|line| !line.trim().is_empty())
    {
        table_lines.push(String::new());
    }
    table_lines.push(format!("[{table_name}]"));
    table_lines.extend(
        values
            .into_iter()
            .map(|(key, value)| format!("{key} = {value}")),
    );
    if insert_at < lines.len()
        && lines
            .get(insert_at)
            .is_some_and(|line| !line.trim().is_empty())
    {
        table_lines.push(String::new());
    }
    let mut next_lines = lines;
    next_lines.splice(insert_at..insert_at, table_lines);
    next_lines
}

fn upsert_table_config_entries(
    lines: &[String],
    table_name: &str,
    values: Vec<(&str, String)>,
) -> Vec<String> {
    let Some((start, end)) = table_bounds(lines, table_name) else {
        return set_table_config_entries(lines, table_name, values);
    };
    let mut pending = values;
    let mut next_lines = Vec::with_capacity(lines.len() + pending.len());

    for (index, line) in lines.iter().enumerate() {
        if index > start && index < end {
            if let Some((key, _value)) = root_assignment(line) {
                if let Some(pending_index) = pending
                    .iter()
                    .position(|(pending_key, _)| *pending_key == key)
                {
                    let (_pending_key, pending_value) = pending.remove(pending_index);
                    next_lines.push(format!("{key} = {pending_value}"));
                    continue;
                }
            }
        }
        next_lines.push(line.clone());
    }

    if pending.is_empty() {
        return next_lines;
    }

    let mut insert_at = end;
    while insert_at > start + 1
        && next_lines
            .get(insert_at - 1)
            .is_some_and(|line| line.trim().is_empty())
    {
        insert_at -= 1;
    }
    let insert_lines: Vec<String> = pending
        .into_iter()
        .map(|(key, value)| format!("{key} = {value}"))
        .collect();
    next_lines.splice(insert_at..insert_at, insert_lines);
    next_lines
}

fn normalize_blank_lines(lines: &[String]) -> Vec<String> {
    let mut next_lines = Vec::with_capacity(lines.len());
    for line in lines {
        if line.trim().is_empty()
            && next_lines
                .last()
                .is_some_and(|previous: &String| previous.trim().is_empty())
        {
            continue;
        }
        next_lines.push(line.clone());
    }
    while next_lines.last().is_some_and(|line| line.trim().is_empty()) {
        next_lines.pop();
    }
    next_lines
}

fn copy_optional_file(source: &Path, target: &Path) -> Result<(), String> {
    if !source.exists() {
        return Ok(());
    }
    copy_file(source, target)
}

fn copy_file(source: &Path, target: &Path) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "创建 Codex app 多开文件目录失败 {}: {err}",
                parent.display()
            )
        })?;
    }
    fs::copy(source, target).map(|_| ()).map_err(|err| {
        format!(
            "复制 Codex app 多开文件失败 {} -> {}: {err}",
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

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    fn joined(lines: Vec<String>) -> String {
        lines.join("\n")
    }

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
    fn merge_api_config_preserves_app_tables_and_updates_provider() {
        let output = joined(merge_instance_config_lines(
            &lines(&[
                "model_provider = \"old\"",
                "cli_auth_credentials_store = \"keyring\"",
                "preferred_auth_method = \"chatgpt\"",
                "openai_base_url = \"https://old.example.com\"",
                "",
                "[model_providers.api]",
                "name = \"api\"",
                "base_url = \"https://old.example.com/v1\"",
                "wire_api = \"chat\"",
                "",
                "[plugins.\"browser@openai-bundled\"]",
                "enabled = true",
                "",
                "[features]",
                "js_repl = false",
            ]),
            &InstanceConfig::Api {
                base_url: "https://api.example.com/v1".to_string(),
            },
        ));

        assert!(output.contains("model_provider = \"api\""));
        assert!(output.contains("cli_auth_credentials_store = \"file\""));
        assert!(!output.contains("preferred_auth_method"));
        assert!(!output.contains("openai_base_url"));
        assert!(output.contains("[windows]\nsandbox = \"elevated\""));
        assert!(output.contains("[model_providers.api]"));
        assert!(output.contains("base_url = \"https://api.example.com/v1\""));
        assert!(output.contains("wire_api = \"responses\""));
        assert!(output.contains("requires_openai_auth = true"));
        assert!(output.contains("[plugins.\"browser@openai-bundled\"]\nenabled = true"));
        assert!(output.contains("[features]\njs_repl = false"));
    }

    #[test]
    fn merge_subscription_config_preserves_app_tables_and_removes_api_provider() {
        let output = joined(merge_instance_config_lines(
            &lines(&[
                "model_provider = \"api\"",
                "cli_auth_credentials_store = \"file\"",
                "",
                "[windows]",
                "sandbox_private_desktop = false",
                "",
                "[model_providers.api]",
                "name = \"api\"",
                "base_url = \"https://api.example.com/v1\"",
                "",
                "[mcp_servers.node_repl]",
                "command = 'node_repl.exe'",
            ]),
            &InstanceConfig::Subscription,
        ));

        assert!(!output.contains("model_provider = \"api\""));
        assert!(!output.contains("[model_providers.api]"));
        assert!(output.contains("cli_auth_credentials_store = \"file\""));
        assert!(
            output.contains("[windows]\nsandbox_private_desktop = false\nsandbox = \"elevated\"")
        );
        assert!(output.contains("[mcp_servers.node_repl]\ncommand = 'node_repl.exe'"));
    }

    #[test]
    fn merge_instance_config_updates_existing_windows_sandbox_only() {
        let output = merge_instance_config_lines(
            &lines(&[
                "cli_auth_credentials_store = \"file\"",
                "",
                "[windows]",
                "sandbox = \"unelevated\"",
                "sandbox_private_desktop = false",
                "",
                "[plugins.\"chrome@openai-bundled\"]",
                "enabled = true",
            ]),
            &InstanceConfig::Subscription,
        );

        assert_eq!(
            output,
            lines(&[
                "cli_auth_credentials_store = \"file\"",
                "",
                "[windows]",
                "sandbox = \"elevated\"",
                "sandbox_private_desktop = false",
                "",
                "[plugins.\"chrome@openai-bundled\"]",
                "enabled = true",
            ])
        );
    }

    #[test]
    fn merge_instance_config_creates_minimal_api_config() {
        let output = merge_instance_config_lines(
            &[],
            &InstanceConfig::Api {
                base_url: "https://api.example.com/v1".to_string(),
            },
        );

        assert_eq!(
            output,
            lines(&[
                "model_provider = \"api\"",
                "cli_auth_credentials_store = \"file\"",
                "",
                "[windows]",
                "sandbox = \"elevated\"",
                "",
                "[model_providers.api]",
                "name = \"api\"",
                "base_url = \"https://api.example.com/v1\"",
                "wire_api = \"responses\"",
                "supports_websockets = false",
                "requires_openai_auth = true",
            ])
        );
    }

    #[test]
    fn merge_instance_config_creates_minimal_subscription_config() {
        let output = merge_instance_config_lines(&[], &InstanceConfig::Subscription);

        assert_eq!(
            output,
            lines(&[
                "cli_auth_credentials_store = \"file\"",
                "",
                "[windows]",
                "sandbox = \"elevated\"",
            ])
        );
    }

    #[test]
    fn command_line_matches_instance_user_data_dir() {
        let user_data_dir = PathBuf::from(r"C:\Instances\codex-app-instances\api-main\user-data");

        assert!(command_line_matches_user_data_dir(
            r#""C:\Codex\Codex.exe" --user-data-dir="C:\Instances\codex-app-instances\api-main\user-data""#,
            &user_data_dir
        ));
        assert!(command_line_matches_user_data_dir(
            r#""C:\Codex\Codex.exe" --user-data-dir=C:/Instances/codex-app-instances/api-main/user-data"#,
            &user_data_dir
        ));
        assert!(!command_line_matches_user_data_dir(
            r#""C:\Codex\Codex.exe" --user-data-dir=C:\Instances\Codex\web\Codex"#,
            &user_data_dir
        ));
    }

    #[test]
    fn extend_unique_paths_preserves_first_candidate_priority() {
        let mut paths = vec![r"C:\Codex\app\Codex.exe".to_string()];

        extend_unique_paths(
            &mut paths,
            vec![
                r"c:\codex\app\codex.exe".to_string(),
                r"C:\CodexPreview\app\Codex.exe".to_string(),
            ],
        );

        assert_eq!(
            paths,
            vec![
                r"C:\Codex\app\Codex.exe".to_string(),
                r"C:\CodexPreview\app\Codex.exe".to_string(),
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

    #[test]
    fn format_toml_string_escapes_backslashes_and_quotes() {
        assert_eq!(format_toml_string("a\\b\"c"), "\"a\\\\b\\\"c\"");
    }
}
