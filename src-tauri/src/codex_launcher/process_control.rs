use super::*;

pub(crate) fn kill_process_tree(pid: u64) -> bool {
    if !cfg!(windows) || pid == 0 {
        return false;
    }
    let mut command = Command::new("taskkill");
    command
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_command_window(&mut command);
    command.status().is_ok_and(|status| status.success())
}

pub(crate) fn get_alive_pids(pids: &[u64]) -> Vec<u64> {
    let mut uniq: Vec<u64> = pids.iter().copied().filter(|pid| *pid > 0).collect();
    uniq.sort_unstable();
    uniq.dedup();
    if uniq.is_empty() || !cfg!(windows) {
        return Vec::new();
    }

    let script = alive_pids(&uniq);

    run_pwsh(&script)
        .ok()
        .and_then(|output| parse_json_output(&output, json!([])).ok())
        .map(json_as_array)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_u64())
        .collect()
}

pub(crate) fn wait_for_pids_exit(pids: &[u64], timeout_ms: u64) -> Vec<u64> {
    let start = Instant::now();
    let timeout = StdDuration::from_millis(timeout_ms);
    let mut alive = get_alive_pids(pids);

    while !alive.is_empty() && start.elapsed() < timeout {
        thread::sleep(StdDuration::from_millis(250));
        alive = get_alive_pids(&alive);
    }

    alive
}

pub(crate) fn launch_executable_with_options(
    executable_path: &str,
    args: &[String],
    envs: &[(String, String)],
) -> Result<bool, String> {
    let path = PathBuf::from(executable_path);
    if !path.exists() {
        return Ok(false);
    }

    let mut command = Command::new(&path);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for arg in args {
        command.arg(arg);
    }
    for (name, value) in envs {
        command.env(name, value);
    }
    if let Some(parent) = path.parent() {
        command.current_dir(parent);
    }
    sanitize_desktop_app_launch_env(&mut command);
    hide_command_window(&mut command);

    command
        .spawn()
        .map(|_| true)
        .map_err(|err| format!("重新打开应用失败 {}: {err}", path.display()))
}

pub(crate) fn relaunch_executable(executable_path: &str) -> Result<bool, String> {
    launch_executable_with_options(executable_path, &[], &[])
}

pub(crate) fn sanitize_desktop_app_launch_env(command: &mut Command) {
    // Codex Switch can be launched from Codex/VS Code, where Electron helper
    // processes set this. Packaged desktop apps must start as Electron apps,
    // not as Node entrypoints.
    command.env_remove("ELECTRON_RUN_AS_NODE");
}

pub(crate) fn relaunch_executable_with_retry(executable_path: &str) -> Result<bool, String> {
    let mut last_error = None;
    for _ in 0..2 {
        match relaunch_executable(executable_path) {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(err) => last_error = Some(err),
        }
        thread::sleep(StdDuration::from_millis(300));
    }
    if let Some(err) = last_error {
        Err(err)
    } else {
        Ok(false)
    }
}
