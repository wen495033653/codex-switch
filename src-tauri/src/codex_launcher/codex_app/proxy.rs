use super::{
    package::get_codex_package_info,
    process::{
        get_codex_desktop_processes, process_executable_starts_with, start_codex_desktop_app,
    },
    *,
};

pub(super) fn check_codex_proxy_impl(proxy_url: String) -> Result<Value, String> {
    let normalized = normalize_proxy_url(&proxy_url)?;
    if normalized.is_empty() {
        return Err("代理地址不能为空".to_string());
    }
    let display_url = normalize_proxy_display_url(&proxy_url);
    let endpoint = assert_proxy_ready(&normalized)?;
    Ok(json!({
        "ok": true,
        "message": "代理连接正常",
        "proxy_url": display_url,
        "host": endpoint.host,
        "port": endpoint.port
    }))
}

pub(super) fn launch_codex_with_proxy_impl(
    proxy_url: &str,
    openai_base_url: &str,
) -> Result<Value, String> {
    if !cfg!(target_os = "windows") {
        return Err("通过代理启动 Codex 仅支持 Windows".to_string());
    }

    fs::create_dir_all(launcher_dir()?).map_err(|err| format!("创建 launcher 目录失败: {err}"))?;
    let endpoint = assert_proxy_ready(proxy_url)?;
    let envs = build_proxy_environment(proxy_url, openai_base_url);
    let codex_info = get_codex_package_info()?;
    let existing = get_codex_desktop_processes()?;
    let executable_path = string_field(&codex_info, "ExecutablePath");
    let app_user_model_id = string_field(&codex_info, "AppUserModelId");
    let install_location = string_field(&codex_info, "InstallLocation");

    write_launcher_log(&format!(
        "prepare proxy={proxy_url} executable={executable_path} appUserModelId={app_user_model_id} existingCount={}",
        existing.len()
    ))?;
    write_launcher_log(&format!(
        "proxy env prepared proxy={proxy_url} openaiBaseUrl={}",
        openai_base_url.trim()
    ))?;

    let (current, fresh) = start_codex_desktop_app(&codex_info, &envs)?;
    let fresh_ids = fresh
        .iter()
        .filter_map(|process| value_u64_field(process, "ProcessId"))
        .map(|process_id| process_id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    write_launcher_log(&format!(
        "launch verification desktopProcessCount={} newProcessIds={}",
        current.len(),
        if fresh_ids.is_empty() {
            "none"
        } else {
            &fresh_ids
        }
    ))?;

    let proxy_connected = wait_codex_proxy_connection(&install_location, endpoint.port)?;
    write_launcher_log(&format!(
        "{} proxy verification port={} connected={proxy_connected}",
        if proxy_connected { "ok" } else { "warning" },
        endpoint.port
    ))?;

    Ok(json!({
        "proxy_url": proxy_url,
        "openai_base_url": openai_base_url.trim(),
        "proxy_connected": proxy_connected,
        "process_count": current.len(),
        "log_path": launcher_log_path()?.to_string_lossy()
    }))
}

pub(super) fn launch_codex_plain_impl() -> Result<Value, String> {
    if !cfg!(target_os = "windows") {
        return Err("启动 Codex 仅支持 Windows".to_string());
    }

    fs::create_dir_all(launcher_dir()?).map_err(|err| format!("创建 launcher 目录失败: {err}"))?;
    let codex_info = get_codex_package_info()?;
    let existing = get_codex_desktop_processes()?;
    let executable_path = string_field(&codex_info, "ExecutablePath");
    let app_user_model_id = string_field(&codex_info, "AppUserModelId");

    write_launcher_log(&format!(
        "prepare plain launch executable={executable_path} appUserModelId={app_user_model_id} existingCount={}",
        existing.len()
    ))?;

    let (current, fresh) = start_codex_desktop_app(&codex_info, &HashMap::new())?;
    let fresh_ids = fresh
        .iter()
        .filter_map(|process| value_u64_field(process, "ProcessId"))
        .map(|process_id| process_id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    write_launcher_log(&format!(
        "plain launch verification desktopProcessCount={} newProcessIds={}",
        current.len(),
        if fresh_ids.is_empty() {
            "none"
        } else {
            &fresh_ids
        }
    ))?;

    Ok(json!({
        "process_count": current.len(),
        "log_path": launcher_log_path()?.to_string_lossy()
    }))
}

pub(super) fn create_codex_proxy_desktop_shortcut_impl(proxy_url: &str) -> Result<Value, String> {
    if !cfg!(target_os = "windows") {
        return Err("创建桌面图标仅支持 Windows".to_string());
    }

    let normalized_proxy_url = normalize_proxy_url(proxy_url)?;
    let proxy_enabled = !normalized_proxy_url.is_empty();
    if proxy_enabled {
        assert_proxy_ready(&normalized_proxy_url)?;
    }
    let codex_info = get_codex_package_info()?;
    let shortcut_icon_path = codex_shortcut_icon_path(&codex_info)?;
    let launcher_path =
        std::env::current_exe().map_err(|err| format!("读取 Codex Switch 路径失败: {err}"))?;
    let shortcut_path = windows_desktop_dir()?.join(if proxy_enabled {
        "Codex 代理启动.lnk"
    } else {
        "Codex 启动.lnk"
    });

    fs::create_dir_all(launcher_dir()?).map_err(|err| format!("创建 launcher 目录失败: {err}"))?;
    let script = create_windows_shortcut_script(
        &shortcut_path,
        &launcher_path,
        &shortcut_arguments(proxy_enabled, &normalized_proxy_url),
        &shortcut_icon_path.to_string_lossy(),
        if proxy_enabled {
            "通过 Codex Switch 代理环境启动 Codex"
        } else {
            "启动 Codex"
        },
    );
    run_pwsh(&script)?;
    write_launcher_log(&format!(
        "desktop shortcut created shortcut={} launcher={} proxyEnabled={} proxy={}",
        shortcut_path.to_string_lossy(),
        launcher_path.to_string_lossy(),
        proxy_enabled,
        normalized_proxy_url
    ))?;

    Ok(json!({
        "proxy_enabled": proxy_enabled,
        "shortcut_path": shortcut_path.to_string_lossy(),
        "launcher_path": launcher_path.to_string_lossy(),
        "icon_path": shortcut_icon_path.to_string_lossy(),
        "proxy_url": normalized_proxy_url
    }))
}

fn codex_shortcut_icon_path(codex_info: &Value) -> Result<PathBuf, String> {
    let icon_path = string_field(codex_info, "IconPath");
    if icon_path.is_empty() {
        return Ok(PathBuf::from(string_field(codex_info, "ExecutablePath")));
    }

    let source_path = PathBuf::from(icon_path);
    if !source_path.exists() {
        return Ok(PathBuf::from(string_field(codex_info, "ExecutablePath")));
    }

    let target_path = launcher_dir()?.join("codex.ico");
    write_png_as_ico(&source_path, &target_path)?;
    Ok(target_path)
}

fn write_png_as_ico(source_path: &Path, target_path: &Path) -> Result<(), String> {
    let png = fs::read(source_path).map_err(|err| format!("读取 Codex 图标失败: {err}"))?;
    let (width, height) = png_dimensions(&png)?;
    let icon_width = if width >= 256 { 0 } else { width as u8 };
    let icon_height = if height >= 256 { 0 } else { height as u8 };
    let png_len =
        u32::try_from(png.len()).map_err(|_| "Codex 图标文件过大，无法写入 ico".to_string())?;

    let mut ico = Vec::with_capacity(22 + png.len());
    ico.extend_from_slice(&0u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.push(icon_width);
    ico.push(icon_height);
    ico.push(0);
    ico.push(0);
    ico.extend_from_slice(&1u16.to_le_bytes());
    ico.extend_from_slice(&32u16.to_le_bytes());
    ico.extend_from_slice(&png_len.to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes());
    ico.extend_from_slice(&png);

    fs::write(target_path, ico).map_err(|err| format!("写入 Codex 快捷方式图标失败: {err}"))
}

fn png_dimensions(png: &[u8]) -> Result<(u32, u32), String> {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if png.len() < 24 || &png[0..8] != PNG_SIGNATURE || &png[12..16] != b"IHDR" {
        return Err("Codex 图标不是有效 PNG 文件".to_string());
    }
    let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    if width == 0 || height == 0 {
        return Err("Codex 图标尺寸无效".to_string());
    }
    Ok((width, height))
}

fn windows_desktop_dir() -> Result<PathBuf, String> {
    let output = run_pwsh("[Environment]::GetFolderPath('DesktopDirectory')")?;
    let path = output.trim();
    if path.is_empty() {
        return Err("无法定位桌面目录".to_string());
    }
    Ok(PathBuf::from(path))
}

fn ps_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn windows_argument(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }

    let mut result = String::from("\"");
    let mut backslashes = 0usize;
    for ch in value.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                result.push_str(&"\\".repeat(backslashes * 2 + 1));
                result.push('"');
                backslashes = 0;
            }
            _ => {
                if backslashes > 0 {
                    result.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                }
                result.push(ch);
            }
        }
    }
    if backslashes > 0 {
        result.push_str(&"\\".repeat(backslashes * 2));
    }
    result.push('"');
    result
}

fn shortcut_arguments(proxy_enabled: bool, proxy_url: &str) -> String {
    let mut args = vec!["--codex-switch-launch-codex".to_string()];
    if proxy_enabled {
        args.push("--codex-switch-proxy-url".to_string());
        args.push(windows_argument(proxy_url));
    }
    args.join(" ")
}

fn create_windows_shortcut_script(
    shortcut_path: &Path,
    launcher_path: &Path,
    shortcut_arguments: &str,
    icon_path: &str,
    description: &str,
) -> String {
    format!(
        r#"
$ErrorActionPreference = "Stop"
$shortcutPath = {shortcut_path}
$launcherPath = {launcher_path}
$shortcutArguments = {shortcut_arguments}
$iconPath = {icon_path}
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = $launcherPath
$shortcut.Arguments = $shortcutArguments
$shortcut.WorkingDirectory = Split-Path -Parent $launcherPath
$shortcut.IconLocation = "$iconPath,0"
$shortcut.Description = {description}
$shortcut.Save()
"#,
        shortcut_path = ps_single_quote(&shortcut_path.to_string_lossy()),
        launcher_path = ps_single_quote(&launcher_path.to_string_lossy()),
        shortcut_arguments = ps_single_quote(shortcut_arguments),
        icon_path = ps_single_quote(icon_path),
        description = ps_single_quote(description)
    )
}

fn test_codex_proxy_connection(process_ids: &[u64], port: u16) -> Result<bool, String> {
    if process_ids.is_empty() {
        return Ok(false);
    }
    let process_id_list = process_ids
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    if process_id_list.is_empty() {
        return Ok(false);
    }

    let script = codex_proxy_connection(process_ids, port);
    let output = run_pwsh(&script)?;
    let result = parse_json_output(&output, json!({ "Connected": false }))?;
    Ok(bool_field(&result, "Connected"))
}

fn wait_codex_proxy_connection(install_location: &str, port: u16) -> Result<bool, String> {
    for _ in 0..27 {
        let process_ids: Vec<u64> = get_codex_desktop_processes()?
            .iter()
            .filter(|process| process_executable_starts_with(process, install_location))
            .filter_map(|process| value_u64_field(process, "ProcessId"))
            .collect();
        if test_codex_proxy_connection(&process_ids, port)? {
            return Ok(true);
        }
        thread::sleep(StdDuration::from_millis(300));
    }
    Ok(false)
}
