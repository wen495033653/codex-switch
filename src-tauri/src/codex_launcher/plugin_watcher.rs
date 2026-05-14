use super::*;
use std::net::{SocketAddr, TcpStream};

const WATCHER_INTERVAL_MS: u64 = 3_000;
const TAKEOVER_GRACE_MS: u64 = 2_000;
const SUCCESS_COOLDOWN_MS: u64 = 15_000;
const FAILURE_BACKOFF_MS: u64 = 30_000;
const CDP_PROBE_TIMEOUT_MS: u64 = 500;
const RELAUNCH_DELAY_MS: u64 = 1_500;

#[derive(Clone, Debug, PartialEq, Eq)]
struct CodexProcess {
    pid: u64,
    executable_path: String,
}

pub(crate) fn start_codex_plugin_takeover_watcher() {
    if !cfg!(windows) {
        return;
    }

    thread::spawn(watch_codex_plugin_takeover);
}

fn watch_codex_plugin_takeover() {
    let mut candidate_pids = Vec::<u64>::new();
    let mut candidate_since: Option<Instant> = None;
    let mut cooldown_until: Option<Instant> = None;
    let mut backoff_until: Option<Instant> = None;

    loop {
        let now = Instant::now();

        if !codex_plugin_takeover_enabled().unwrap_or(false) {
            reset_candidate(&mut candidate_pids, &mut candidate_since);
            sleep_interval();
            continue;
        }

        if until_active(backoff_until, now) || until_active(cooldown_until, now) {
            reset_candidate(&mut candidate_pids, &mut candidate_since);
            sleep_interval();
            continue;
        }

        if plugin_cdp_port_listening() {
            reset_candidate(&mut candidate_pids, &mut candidate_since);
            sleep_interval();
            continue;
        }

        let processes = match running_codex_processes() {
            Ok(processes) => processes,
            Err(err) => {
                eprintln!("Codex app Plugin 自动接管检测失败: {err}");
                reset_candidate(&mut candidate_pids, &mut candidate_since);
                backoff_until = Some(now + StdDuration::from_millis(FAILURE_BACKOFF_MS));
                sleep_interval();
                continue;
            }
        };

        if processes.is_empty() {
            reset_candidate(&mut candidate_pids, &mut candidate_since);
            sleep_interval();
            continue;
        }

        let pids = processes
            .iter()
            .map(|process| process.pid)
            .collect::<Vec<_>>();
        if candidate_pids != pids {
            candidate_pids = pids;
            candidate_since = Some(now);
            sleep_interval();
            continue;
        }

        if candidate_since
            .map(|started| started.elapsed() < StdDuration::from_millis(TAKEOVER_GRACE_MS))
            .unwrap_or(true)
        {
            sleep_interval();
            continue;
        }

        if plugin_cdp_port_listening() {
            reset_candidate(&mut candidate_pids, &mut candidate_since);
            sleep_interval();
            continue;
        }

        match takeover_running_codex_processes(&processes) {
            Ok(restarted) if restarted > 0 => {
                reset_candidate(&mut candidate_pids, &mut candidate_since);
                cooldown_until =
                    Some(Instant::now() + StdDuration::from_millis(SUCCESS_COOLDOWN_MS));
            }
            Ok(_) => {
                reset_candidate(&mut candidate_pids, &mut candidate_since);
                backoff_until = Some(Instant::now() + StdDuration::from_millis(FAILURE_BACKOFF_MS));
            }
            Err(err) => {
                eprintln!("Codex app Plugin 自动接管失败: {err}");
                reset_candidate(&mut candidate_pids, &mut candidate_since);
                backoff_until = Some(Instant::now() + StdDuration::from_millis(FAILURE_BACKOFF_MS));
            }
        }

        sleep_interval();
    }
}

fn reset_candidate(candidate_pids: &mut Vec<u64>, candidate_since: &mut Option<Instant>) {
    candidate_pids.clear();
    *candidate_since = None;
}

fn sleep_interval() {
    thread::sleep(StdDuration::from_millis(WATCHER_INTERVAL_MS));
}

fn until_active(until: Option<Instant>, now: Instant) -> bool {
    until.is_some_and(|deadline| now < deadline)
}

fn plugin_cdp_port_listening() -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], CODEX_PLUGIN_DEBUG_PORT));
    TcpStream::connect_timeout(&addr, StdDuration::from_millis(CDP_PROBE_TIMEOUT_MS)).is_ok()
}

fn running_codex_processes() -> Result<Vec<CodexProcess>, String> {
    let snapshot = capture_open_ide_snapshot()?;
    let mut processes = snapshot
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| string_field(entry, "kind") == "codex")
        .filter_map(|entry| {
            let pid = value_u64_field(&entry, "pid")?;
            let executable_path = raw_string_field(&entry, "executablePath");
            if executable_path.trim().is_empty() {
                return None;
            }
            Some(CodexProcess {
                pid,
                executable_path,
            })
        })
        .collect::<Vec<_>>();
    processes.sort_by_key(|process| process.pid);
    processes.dedup_by_key(|process| process.pid);
    Ok(processes)
}

fn takeover_running_codex_processes(processes: &[CodexProcess]) -> Result<usize, String> {
    let pids = processes
        .iter()
        .map(|process| process.pid)
        .collect::<Vec<_>>();
    let mut executables = processes
        .iter()
        .map(|process| process.executable_path.clone())
        .filter(|path| !path.trim().is_empty())
        .collect::<Vec<_>>();
    executables.sort_by_key(|path| path.trim().to_ascii_lowercase());
    executables.dedup_by_key(|path| path.trim().to_ascii_lowercase());
    if executables.is_empty() {
        return Err("未检测到 Codex app 可执行路径".to_string());
    }

    for pid in &pids {
        let _ = kill_process_tree(*pid);
    }
    let alive = wait_for_pids_exit(&pids, 12_000);
    if !alive.is_empty() {
        return Err("Codex app 进程未能退出".to_string());
    }

    thread::sleep(StdDuration::from_millis(RELAUNCH_DELAY_MS));

    let mut restarted = 0usize;
    for executable in executables {
        launch_codex_with_plugins(Path::new(&executable))?;
        restarted += 1;
        thread::sleep(StdDuration::from_millis(120));
    }

    Ok(restarted)
}
