use super::*;
use crate::session_sync_diagnostics::log_session_sync_event;
use std::process::Child;
use sysinfo::{Pid, System};

const PROCESS_KILL_TIMEOUT_MS: u64 = 10_000;
const LAUNCH_CONFIRM_MS: u64 = 1_500;

pub(crate) fn kill_process_tree(pid: u64) -> Result<bool, String> {
    let started = Instant::now();
    let result = if pid == 0 || u32::try_from(pid).is_err() {
        Err(format!("无效的进程 PID: {pid}"))
    } else if get_alive_pids(&[pid]).is_empty() {
        Ok(false)
    } else {
        kill_process_tree_impl(pid)
    };
    log_session_sync_event(
        if result.is_err() {
            "codex_app_process_kill_error"
        } else {
            "codex_app_process_kill_finish"
        },
        json!({"pid": pid, "elapsedMs": started.elapsed().as_millis(), "timeoutMs": PROCESS_KILL_TIMEOUT_MS,
            "terminated": result.as_ref().ok(), "error": result.as_ref().err()}),
    );
    result
}

#[cfg(windows)]
fn kill_process_tree_impl(pid: u64) -> Result<bool, String> {
    let mut command = Command::new("taskkill");
    command.args(["/F", "/T", "/PID", &pid.to_string()]);
    hide_command_window(&mut command);
    run_bounded_command(
        &mut command,
        StdDuration::from_millis(PROCESS_KILL_TIMEOUT_MS),
    )
    .map(|()| true)
    .map_err(|err| format!("taskkill /F /T /PID {pid}: {err}"))
}

// Drain both pipes concurrently, so a full pipe cannot turn the timeout into a deadlock.
// Pipe collection and timeout cleanup are themselves bounded, including inherited handles.
#[cfg(any(windows, test))]
fn run_bounded_command(command: &mut Command, timeout: StdDuration) -> Result<(), String> {
    use std::{io::Read, sync::mpsc};
    fn capture(stream: impl Read + Send + 'static) -> mpsc::Receiver<Result<String, String>> {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = {
                let mut stream = stream;
                stream.read_to_end(&mut bytes)
            };
            let text = match String::from_utf8(bytes.clone()) {
                Ok(text) => text,
                Err(_) => {
                    use base64::Engine as _;
                    format!(
                        "non-UTF8 output; rawBase64={}",
                        base64::engine::general_purpose::STANDARD.encode(&bytes)
                    )
                }
            };
            let result = result
                .map(|_| text.clone())
                .map_err(|err| format!("{err}; partial={text}"));
            // A timed-out caller may already have returned; sending is not part of command success.
            drop(tx.send(result));
        });
        rx
    }
    let started = Instant::now();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("创建结束进程命令失败: {err}"))?;
    let stdout = capture(child.stdout.take().expect("piped stdout"));
    let stderr = capture(child.stderr.take().expect("piped stderr"));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(err) => break Err(format!("查询命令状态失败: {err}")),
            Ok(None) if started.elapsed() >= timeout => {
                break Err(format!("命令超时 {}ms", timeout.as_millis()))
            }
            Ok(None) => thread::sleep(StdDuration::from_millis(25)),
        }
    };
    let cleanup = if status.is_err() {
        let killed = child.kill();
        let deadline = Instant::now() + StdDuration::from_secs(1);
        let reaped = loop {
            match child.try_wait() {
                Ok(Some(status)) => break format!("exit={status}"),
                Err(err) => break format!("waitError={err}"),
                Ok(None) if Instant::now() >= deadline => {
                    break "cleanup timeout 1000ms".to_string()
                }
                Ok(None) => thread::sleep(StdDuration::from_millis(25)),
            }
        };
        format!("kill={killed:?}; {reaped}")
    } else {
        String::new()
    };
    let stdout = stdout
        .recv_timeout(StdDuration::from_millis(500))
        .map_err(|err| format!("stdout 收集失败: {err}"))
        .and_then(|result| result);
    let stderr = stderr
        .recv_timeout(StdDuration::from_millis(500))
        .map_err(|err| format!("stderr 收集失败: {err}"))
        .and_then(|result| result);
    match (status, stdout, stderr) {
        (Ok(status), Ok(_), Ok(_)) if status.success() => Ok(()),
        (status, stdout, stderr) => {
            let exit_code = status.as_ref().ok().and_then(|status| status.code());
            Err(format!("status={status:?}; exitCode={exit_code:?}; elapsedMs={}; stdout={stdout:?}; stderr={stderr:?}; cleanup={cleanup}", started.elapsed().as_millis()))
        }
    }
}

#[cfg(not(windows))]
fn kill_process_tree_impl(pid: u64) -> Result<bool, String> {
    let system = System::new_all();
    let root_pid = Pid::from_u32(pid as u32);
    let mut tree = vec![root_pid];
    let mut known = std::collections::HashSet::from([root_pid]);
    loop {
        let mut added = false;
        for (candidate_pid, process) in system.processes() {
            if process
                .parent()
                .is_some_and(|parent| known.contains(&parent))
                && known.insert(*candidate_pid)
            {
                tree.push(*candidate_pid);
                added = true;
            }
        }
        if !added {
            break;
        }
    }

    let mut killed_any = false;
    for process in tree
        .into_iter()
        .rev()
        .filter_map(|tree_pid| system.process(tree_pid))
    {
        if !process.kill() {
            return Err(format!("结束进程失败 PID={}", process.pid()));
        }
        killed_any = true;
    }
    Ok(killed_any)
}

pub(crate) fn get_alive_pids(pids: &[u64]) -> Vec<u64> {
    let mut uniq: Vec<u64> = pids.iter().copied().filter(|pid| *pid > 0).collect();
    uniq.sort_unstable();
    uniq.dedup();
    if uniq.is_empty() {
        return Vec::new();
    }

    let system = System::new_all();
    uniq.into_iter()
        .filter(|pid| {
            u32::try_from(*pid)
                .ok()
                .map(Pid::from_u32)
                .is_some_and(|pid| system.process(pid).is_some())
        })
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
    spawn_executable(executable_path, args, envs).map(|_| true)
}

fn spawn_executable(
    executable_path: &str,
    args: &[String],
    envs: &[(String, String)],
) -> Result<Child, String> {
    let path = PathBuf::from(executable_path);
    if !path.is_file() {
        return Err(format!("应用可执行文件不存在: {}", path.display()));
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
        .map_err(|err| format!("重新打开应用失败 {}: {err}", path.display()))
}

pub(crate) fn launch_codex_process_with_options(
    executable_path: &str,
    args: &[String],
    envs: &[(String, String)],
) -> Result<bool, String> {
    let started = Instant::now();
    let mut child = spawn_executable(executable_path, args, envs)?;
    let pid = child.id();
    let result = confirm_launched_process(&mut child, StdDuration::from_millis(LAUNCH_CONFIRM_MS));
    log_session_sync_event(
        if result.is_err() {
            "codex_app_launch_confirmation_error"
        } else {
            "codex_app_launch_confirmation_finish"
        },
        json!({"pid": pid, "executable": executable_path, "elapsedMs": started.elapsed().as_millis(),
            "confirmationMs": LAUNCH_CONFIRM_MS, "criterion": "spawned_process_still_running", "error": result.as_ref().err()}),
    );
    result
        .map(|()| true)
        .map_err(|err| format!("Codex 启动确认失败 {executable_path}: {err}"))
}

fn confirm_launched_process(child: &mut Child, confirmation: StdDuration) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(format!(
                    "新进程提前退出 PID={}，exitCode={:?}，status={status}，elapsedMs={}",
                    child.id(),
                    status.code(),
                    started.elapsed().as_millis()
                ))
            }
            Err(err) => return Err(format!("查询新进程失败 PID={}: {err}", child.id())),
            Ok(None) if started.elapsed() >= confirmation => return Ok(()),
            Ok(None) => thread::sleep(StdDuration::from_millis(25)),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_command(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "codex_launcher::process_control::tests::process_fixture",
                "--nocapture",
            ])
            .env("CODEX_SWITCH_PROCESS_FIXTURE", mode);
        hide_command_window(&mut command);
        command
    }

    #[test]
    fn process_fixture() {
        let Ok(mode) = std::env::var("CODEX_SWITCH_PROCESS_FIXTURE") else {
            return;
        };
        match mode.as_str() {
            "sleep" => thread::sleep(StdDuration::from_secs(10)),
            "error" => {
                println!("fixture stdout");
                eprintln!("fixture stderr");
                std::process::exit(23);
            }
            "large" => {
                println!("{}", "x".repeat(100_000));
                eprintln!("{}", "y".repeat(100_000));
            }
            "zero" => std::process::exit(0),
            _ => panic!("unexpected fixture mode: {mode}"),
        }
    }

    #[test]
    fn command_timeout_is_bounded_and_reports_cleanup() {
        let started = Instant::now();
        let error =
            run_bounded_command(&mut fixture_command("sleep"), StdDuration::from_millis(150))
                .unwrap_err();
        assert!(error.contains("命令超时 150ms"), "{error}");
        assert!(error.contains("exitCode=None"), "{error}");
        assert!(error.contains("kill=Ok(())"), "{error}");
        assert!(started.elapsed() < StdDuration::from_secs(4));
    }

    #[test]
    fn command_error_preserves_exit_status_and_both_streams() {
        let error = run_bounded_command(&mut fixture_command("error"), StdDuration::from_secs(5))
            .unwrap_err();
        assert!(error.contains("exitCode=Some(23)"), "{error}");
        assert!(error.contains("fixture stdout"), "{error}");
        assert!(error.contains("fixture stderr"), "{error}");
    }

    #[test]
    fn command_large_output_does_not_block_pipes() {
        run_bounded_command(&mut fixture_command("large"), StdDuration::from_secs(5)).unwrap();
    }

    #[test]
    fn launch_confirmation_rejects_both_successful_and_failed_early_exit() {
        for mode in ["zero", "error"] {
            let mut child = fixture_command(mode)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let error =
                confirm_launched_process(&mut child, StdDuration::from_millis(LAUNCH_CONFIRM_MS))
                    .unwrap_err();
            assert!(error.contains("提前退出"), "{error}");
            assert!(
                error.contains(if mode == "zero" {
                    "Some(0)"
                } else {
                    "Some(23)"
                }),
                "{error}"
            );
        }
    }

    #[test]
    fn actual_child_survives_confirmation_then_is_terminated() {
        let mut child = fixture_command("sleep")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let confirmed =
            confirm_launched_process(&mut child, StdDuration::from_millis(LAUNCH_CONFIRM_MS));
        let killed = kill_process_tree(u64::from(child.id()));
        confirmed.unwrap();
        assert!(killed.unwrap());
        assert!(wait_for_pids_exit(&[u64::from(child.id())], 2000).is_empty());
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn actual_launch_wrapper_reports_error_and_logs_exit_code() {
        let executable = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let args = vec![
            "--exact".into(),
            "codex_launcher::process_control::tests::process_fixture".into(),
            "--nocapture".into(),
        ];
        let envs = vec![("CODEX_SWITCH_PROCESS_FIXTURE".into(), "error".into())];
        let error = launch_codex_process_with_options(&executable, &args, &envs).unwrap_err();
        assert!(error.contains("Some(23)"), "{error}");
        let entries = crate::session_sync_diagnostics::get_dev_log_entries();
        assert!(
            entries.as_array().unwrap().iter().any(|entry| {
                entry["details"]["event"] == "codex_app_launch_confirmation_error"
                    && entry["details"]["details"]["error"]
                        .as_str()
                        .is_some_and(|error| error.contains("Some(23)"))
            }),
            "{entries}"
        );
    }

    #[test]
    fn kill_invalid_pid_is_returned_and_logged() {
        assert!(kill_process_tree(0).unwrap_err().contains("PID: 0"));
        let entries = crate::session_sync_diagnostics::get_dev_log_entries();
        assert!(entries.as_array().unwrap().iter().any(|entry| {
            entry["details"]["event"] == "codex_app_process_kill_error"
                && entry["details"]["details"]["pid"] == 0
        }));
    }
}
