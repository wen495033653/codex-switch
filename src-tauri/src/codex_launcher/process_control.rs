use super::shell::hide_command_window;
use crate::session_sync_diagnostics::log_session_sync_event;
use serde_json::json;
use std::process::Child;
use std::{
    collections::HashSet,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration as StdDuration, Instant},
};
use sysinfo::{Pid, System};

const PROCESS_KILL_TIMEOUT_MS: u64 = 10_000;
const PROCESS_TREE_EXIT_CONFIRM_MS: u64 = 5_000;
const LAUNCH_CONFIRM_MS: u64 = 1_500;

pub(crate) fn kill_process_tree(pid: u64) -> Result<bool, String> {
    let started = Instant::now();
    let mut termination_attempted = false;
    let result = if pid == 0 || u32::try_from(pid).is_err() {
        Err(format!("无效的进程 PID: {pid}"))
    } else if get_alive_pids(&[pid]).is_empty() {
        Ok(false)
    } else {
        check_process_kill_permission(pid).and_then(|()| {
            termination_attempted = true;
            kill_process_tree_impl(pid)
        })
    };
    log_session_sync_event(
        if result.is_err() {
            "codex_app_process_kill_error"
        } else {
            "codex_app_process_kill_finish"
        },
        json!({"pid": pid, "elapsedMs": started.elapsed().as_millis(), "timeoutMs": PROCESS_KILL_TIMEOUT_MS,
            "terminationAttempted": termination_attempted,
            "terminated": result.as_ref().ok(), "error": result.as_ref().err()}),
    );
    result
}

/// `processes` are `(pid, parent_pid)` pairs of one app. A root is a process whose parent is
/// not in the set.
pub(crate) fn root_pids(processes: &[(u64, u64)]) -> Vec<u64> {
    let all_pids = processes
        .iter()
        .map(|(pid, _)| *pid)
        .collect::<HashSet<_>>();
    let mut root_pids = processes
        .iter()
        .filter(|(_, parent_pid)| *parent_pid == 0 || !all_pids.contains(parent_pid))
        .map(|(pid, _)| *pid)
        .collect::<Vec<_>>();
    root_pids.sort_unstable();
    root_pids.dedup();
    root_pids
}

/// Ends the app by its roots only; each root's tree takes its helpers with it. Killing Electron
/// helpers one by one makes the main process respawn them before the root is reached.
pub(crate) fn kill_root_process_trees(processes: &[(u64, u64)]) -> Result<(), String> {
    kill_roots_after_preflight(
        &root_pids(processes),
        check_process_kill_permission,
        kill_process_tree,
    )
}

fn check_process_kill_permission(pid: u64) -> Result<(), String> {
    #[cfg(windows)]
    {
        super::process_permissions::ensure_termination_elevation(u64::from(std::process::id()), pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        Ok(())
    }
}

fn kill_roots_after_preflight(
    roots: &[u64],
    mut preflight: impl FnMut(u64) -> Result<(), String>,
    mut terminate: impl FnMut(u64) -> Result<bool, String>,
) -> Result<(), String> {
    // Check every root before touching any tree, including when several Codex instances exist.
    for &pid in roots {
        if let Err(error) = preflight(pid) {
            log_session_sync_event(
                "codex_app_process_kill_error",
                json!({"pid": pid, "callerPid": std::process::id(),
                    "stage": "permission_preflight", "terminationAttempted": false,
                    "error": error}),
            );
            return Err(error);
        }
    }
    for &pid in roots {
        terminate(pid)?;
    }
    Ok(())
}

// Success means the tree captured before termination has exited. The termination result is only
// evidence: taskkill /T exits 128 ("not found") for tree members that exit on their own while it
// terminates their relatives (e.g. a `cmd /c` wrapper under Codex's app-server), and a kill can
// likewise fail for a process that has just exited, although the tree is gone.
fn kill_process_tree_impl(pid: u64) -> Result<bool, String> {
    let system = System::new_all();
    let tree_pids = process_tree_pids(&system, pid);
    let termination = terminate_process_tree(&system, pid, &tree_pids);
    let alive_pids = wait_for_pids_exit(&tree_pids, PROCESS_TREE_EXIT_CONFIRM_MS);
    process_tree_kill_outcome(pid, &tree_pids, termination, &alive_pids)
}

fn process_tree_kill_outcome(
    pid: u64,
    tree_pids: &[u64],
    termination: Result<(), String>,
    alive_pids: &[u64],
) -> Result<bool, String> {
    if !alive_pids.is_empty() {
        return Err(format!(
            "结束进程树 PID={pid} 后 {PROCESS_TREE_EXIT_CONFIRM_MS}ms 仍存活 PID: {alive_pids:?}; treePids={tree_pids:?}; termination={termination:?}"
        ));
    }
    if let Err(err) = termination {
        log_session_sync_event(
            "codex_app_process_kill_tree_exited",
            json!({"pid": pid, "treePids": tree_pids, "confirmMs": PROCESS_TREE_EXIT_CONFIRM_MS,
                "terminationError": err}),
        );
    }
    Ok(true)
}

#[cfg(windows)]
fn terminate_process_tree(_system: &System, pid: u64, _tree_pids: &[u64]) -> Result<(), String> {
    let mut command = Command::new("taskkill");
    command.args(["/F", "/T", "/PID", &pid.to_string()]);
    hide_command_window(&mut command);
    run_bounded_command(
        &mut command,
        StdDuration::from_millis(PROCESS_KILL_TIMEOUT_MS),
    )
    .map_err(|err| format!("taskkill /F /T /PID {pid}: {err}"))
}

#[cfg(not(windows))]
fn terminate_process_tree(system: &System, _pid: u64, tree_pids: &[u64]) -> Result<(), String> {
    let failed_pids = tree_pids
        .iter()
        .rev()
        .filter_map(|tree_pid| system.process(Pid::from_u32(*tree_pid as u32)))
        .filter(|process| !process.kill())
        .map(|process| process.pid().as_u32())
        .collect::<Vec<_>>();
    if failed_pids.is_empty() {
        Ok(())
    } else {
        Err(format!("结束进程失败 PID={failed_pids:?}"))
    }
}

fn process_tree_pids(system: &System, root_pid: u64) -> Vec<u64> {
    let mut tree = vec![root_pid];
    let mut known = HashSet::from([root_pid]);
    loop {
        let mut added = false;
        for (candidate_pid, process) in system.processes() {
            let candidate_pid = u64::from(candidate_pid.as_u32());
            if process
                .parent()
                .is_some_and(|parent| known.contains(&u64::from(parent.as_u32())))
                && known.insert(candidate_pid)
            {
                tree.push(candidate_pid);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    tree
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

    #[test]
    fn permission_preflight_failure_never_starts_any_root_termination() {
        let mut checked = Vec::new();
        let mut terminated = Vec::new();
        let error = kill_roots_after_preflight(
            &[41, 42],
            |pid| {
                checked.push(pid);
                if pid == 42 {
                    Err("callerElevated=false, targetElevated=true".to_string())
                } else {
                    Ok(())
                }
            },
            |pid| {
                terminated.push(pid);
                Ok(true)
            },
        )
        .unwrap_err();
        assert_eq!(checked, [41, 42]);
        assert!(terminated.is_empty());
        assert!(error.contains("targetElevated=true"));
        let entries = crate::session_sync_diagnostics::get_dev_log_entries();
        assert!(
            entries.as_array().unwrap().iter().any(|entry| {
                entry["details"]["event"] == "codex_app_process_kill_error"
                    && entry["details"]["details"]["pid"] == 42
                    && entry["details"]["details"]["terminationAttempted"] == false
                    && entry["details"]["details"]["stage"] == "permission_preflight"
            }),
            "{entries}"
        );
    }

    #[test]
    fn successful_permission_preflight_precedes_all_root_terminations() {
        use std::cell::RefCell;
        let calls = RefCell::new(Vec::new());
        kill_roots_after_preflight(
            &[41, 42],
            |pid| {
                calls.borrow_mut().push(("check", pid));
                Ok(())
            },
            |pid| {
                calls.borrow_mut().push(("terminate", pid));
                Ok(true)
            },
        )
        .unwrap();
        assert_eq!(
            *calls.borrow(),
            [
                ("check", 41),
                ("check", 42),
                ("terminate", 41),
                ("terminate", 42)
            ]
        );
    }

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
            "tree" => {
                let mut child = fixture_command("sleep")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                println!("child_pid={}", child.id());
                child.wait().unwrap();
            }
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

    #[cfg(windows)]
    #[test]
    fn taskkill_not_found_for_already_exited_tree_is_success_and_logged() {
        let mut child = fixture_command("zero")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child.wait().unwrap();
        // The open Child handle keeps this PID from being reused while taskkill looks it up.
        let pid = u64::from(child.id());

        assert!(kill_process_tree_impl(pid).unwrap());

        let entries = crate::session_sync_diagnostics::get_dev_log_entries();
        assert!(
            entries.as_array().unwrap().iter().any(|entry| {
                entry["details"]["event"] == "codex_app_process_kill_tree_exited"
                    && entry["details"]["details"]["pid"] == pid
                    && entry["details"]["details"]["terminationError"]
                        .as_str()
                        .is_some_and(|error| error.contains("exitCode=Some(128)"))
            }),
            "{entries}"
        );
    }

    #[test]
    fn process_tree_kill_outcome_is_decided_by_tree_exit_only() {
        let error =
            process_tree_kill_outcome(10, &[10, 11], Err("exitCode=Some(1)".to_string()), &[11])
                .unwrap_err();
        assert!(error.contains("仍存活 PID: [11]"), "{error}");
        assert!(error.contains("exitCode=Some(1)"), "{error}");

        let error = process_tree_kill_outcome(10, &[10, 11], Ok(()), &[11]).unwrap_err();
        assert!(error.contains("仍存活 PID: [11]"), "{error}");

        assert!(
            process_tree_kill_outcome(10, &[10], Err("exitCode=Some(128)".to_string()), &[])
                .unwrap()
        );
        assert!(process_tree_kill_outcome(10, &[10], Ok(()), &[]).unwrap());
    }

    #[test]
    fn root_tree_kill_ends_descendants_without_killing_them_separately() {
        use std::io::{BufRead, BufReader};
        let mut parent = fixture_command("tree")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let parent_pid = u64::from(parent.id());
        let child_pid = BufReader::new(parent.stdout.take().unwrap())
            .lines()
            .map_while(Result::ok)
            .find_map(|line| {
                line.strip_prefix("child_pid=")
                    .and_then(|pid| pid.parse::<u64>().ok())
            })
            .unwrap();
        let tree = [
            (parent_pid, u64::from(std::process::id())),
            (child_pid, parent_pid),
        ];
        assert_eq!(root_pids(&tree), vec![parent_pid]);

        kill_root_process_trees(&tree).unwrap();

        assert!(get_alive_pids(&[parent_pid, child_pid]).is_empty());
        assert!(parent.try_wait().unwrap().is_some());
        let entries = crate::session_sync_diagnostics::get_dev_log_entries();
        let killed_pids = entries
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| {
                entry["details"]["event"]
                    .as_str()
                    .is_some_and(|event| event.starts_with("codex_app_process_kill_"))
            })
            .filter_map(|entry| entry["details"]["details"]["pid"].as_u64())
            .collect::<Vec<_>>();
        assert!(killed_pids.contains(&parent_pid), "{entries}");
        assert!(!killed_pids.contains(&child_pid), "{entries}");
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
