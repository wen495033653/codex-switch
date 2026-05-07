use super::*;

pub(super) fn get_codex_desktop_processes() -> Result<Vec<Value>, String> {
    let output = run_pwsh(GET_CODEX_DESKTOP_PROCESSES)?;
    Ok(json_as_array(parse_json_output(&output, json!([]))?))
}

pub(super) fn process_executable_starts_with(process: &Value, install_location: &str) -> bool {
    raw_string_field(process, "ExecutablePath")
        .to_ascii_lowercase()
        .starts_with(&install_location.to_ascii_lowercase())
}

pub(super) fn start_codex_desktop_app(
    codex_info: &Value,
    envs: &HashMap<String, String>,
) -> Result<(Vec<Value>, Vec<Value>), String> {
    let before_ids: HashSet<u64> = get_codex_desktop_processes()?
        .iter()
        .filter_map(|process| value_u64_field(process, "ProcessId"))
        .collect();
    let executable_path = string_field(codex_info, "ExecutablePath");
    let install_location = string_field(codex_info, "InstallLocation");
    if executable_path.is_empty() {
        return Err("Codex executable path is empty.".to_string());
    }

    let mut command = Command::new(&executable_path);
    if let Some(parent) = Path::new(&executable_path).parent() {
        command.current_dir(parent);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (name, value) in envs {
        command.env(name, value);
    }
    let _child = command
        .spawn()
        .map_err(|err| format!("启动 Codex app 失败: {err}"))?;
    write_launcher_log(&format!("launch requested executable={executable_path}"))?;

    for _ in 0..40 {
        let current: Vec<Value> = get_codex_desktop_processes()?
            .into_iter()
            .filter(|process| process_executable_starts_with(process, &install_location))
            .collect();
        if !current.is_empty() {
            let fresh = current
                .iter()
                .filter(|process| {
                    value_u64_field(process, "ProcessId")
                        .is_some_and(|process_id| !before_ids.contains(&process_id))
                })
                .cloned()
                .collect();
            return Ok((current, fresh));
        }
        thread::sleep(StdDuration::from_millis(300));
    }

    Err("Codex 启动请求已发送，但在 12 秒内没有发现桌面进程。".to_string())
}
