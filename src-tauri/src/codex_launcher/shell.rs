use super::*;

#[cfg(windows)]
pub(crate) fn hide_command_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000);
}

#[cfg(not(windows))]
pub(crate) fn hide_command_window(_command: &mut Command) {}

pub(crate) fn launcher_dir() -> Result<PathBuf, String> {
    Ok(codex_dir()?.join("launcher"))
}

pub(crate) fn launcher_log_path() -> Result<PathBuf, String> {
    Ok(launcher_dir()?.join("codex-proxy-launcher.log"))
}

pub(crate) fn write_launcher_log(message: &str) -> Result<(), String> {
    let dir = launcher_dir()?;
    fs::create_dir_all(&dir).map_err(|err| format!("创建 launcher 日志目录失败: {err}"))?;
    let path = launcher_log_path()?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| format!("打开 launcher 日志失败: {err}"))?;
    writeln!(file, "{} {message}", now_string())
        .map_err(|err| format!("写入 launcher 日志失败: {err}"))
}

pub(crate) fn run_pwsh(script: &str) -> Result<String, String> {
    let mut command = Command::new("pwsh.exe");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_command_window(&mut command);

    let output = command
        .output()
        .map_err(|err| format!("PowerShell 执行失败: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            "PowerShell 执行失败".to_string()
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(crate) fn parse_json_output(output: &str, fallback: Value) -> Result<Value, String> {
    let text = output.trim();
    if text.is_empty() {
        return Ok(fallback);
    }
    serde_json::from_str(text).map_err(|err| format!("解析 PowerShell JSON 输出失败: {err}"))
}

pub(crate) fn json_as_array(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items,
        Value::Null => Vec::new(),
        other => vec![other],
    }
}
