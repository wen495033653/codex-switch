use super::transport::CdpConnection;
use crate::app_log::log_event;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const LOCATOR: &str = include_str!("api_quota.js");
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(20);

struct Supervisor {
    websocket_url: String,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
    failure: Arc<Mutex<Option<String>>>,
}

fn supervisors() -> &'static Mutex<BTreeMap<u16, Supervisor>> {
    static SUPERVISORS: OnceLock<Mutex<BTreeMap<u16, Supervisor>>> = OnceLock::new();
    SUPERVISORS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(super) fn start(port: u16, websocket_url: &str) -> Result<(), String> {
    let mut active = supervisors()
        .lock()
        .map_err(|err| format!("API quota supervisor 锁失败: {err}"))?;
    if let Some(supervisor) = active
        .get(&port)
        .filter(|s| s.websocket_url == websocket_url)
    {
        if !supervisor.thread.is_finished() {
            return Ok(());
        }
        let failure = supervisor
            .failure
            .lock()
            .map_err(|err| format!("API quota supervisor 状态锁失败: {err}"))?
            .clone()
            .unwrap_or_else(|| "API quota supervisor 已停止".to_string());
        return Err(failure);
    }
    // A reopened desktop can reuse the same port with a new renderer target.
    // Retire that old target, rather than treating its disconnect as a failure
    // of the newly requested launch or keeping its dead entry forever.
    if let Some(previous) = active.remove(&port) {
        previous.stop.store(true, Ordering::Release);
        previous
            .thread
            .join()
            .map_err(|_| "API quota 旧 target supervisor 线程异常".to_string())?;
        let failure = previous
            .failure
            .lock()
            .map_err(|err| format!("API quota 旧 target 状态锁失败: {err}"))?
            .clone();
        log_event(
            "codex_api_quota_hook_stopped",
            json!({ "port": port, "reason": "renderer_target_changed", "previousError": failure }),
        );
    }

    let stop = Arc::new(AtomicBool::new(false));
    let failure = Arc::new(Mutex::new(None));
    let worker_stop = Arc::clone(&stop);
    let worker_failure = Arc::clone(&failure);
    let url = websocket_url.to_string();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name(format!("codex-api-quota-{port}"))
        .spawn(move || {
            let runtime = Runtime::connect(&url, &worker_stop);
            let result = match runtime {
                Ok(mut runtime) => {
                    log_event(
                        "codex_api_quota_hook_ready",
                        json!({ "port": port, "location": runtime.location }),
                    );
                    match ready_tx.send(Ok(())) {
                        Ok(()) => runtime.run(&worker_stop),
                        Err(err) => Err(format!("API quota readiness 接收端已关闭: {err}")),
                    }
                    .and_then(|()| runtime.remove())
                }
                Err(err) => {
                    let response = ready_tx.send(Err(err.clone()));
                    Err(match response {
                        Ok(()) => err,
                        Err(send_error) => format!("{err}; readiness={send_error}"),
                    })
                }
            };
            if let Err(error) = result {
                if let Ok(mut failure) = worker_failure.lock() {
                    *failure = Some(error.clone());
                }
                log_event(
                    "codex_api_quota_hook_error",
                    json!({ "port": port, "error": error, "stopping": worker_stop.load(Ordering::Acquire) }),
                );
            }
        })
        .map_err(|err| format!("启动 API quota supervisor 失败: {err}"))?;
    active.insert(
        port,
        Supervisor {
            websocket_url: websocket_url.to_string(),
            stop,
            thread: worker,
            failure,
        },
    );
    // Keep starts/stops serialized until the actual V8 breakpoint is ready.
    match ready_rx.recv_timeout(DISCOVERY_TIMEOUT + Duration::from_secs(15)) {
        Ok(Ok(())) => Ok(()),
        result => {
            let supervisor = active.remove(&port).expect("inserted supervisor");
            supervisor.stop.store(true, Ordering::Release);
            let join = supervisor.thread.join();
            if join.is_err() {
                return Err("API quota supervisor 线程异常".to_string());
            }
            match result {
                Ok(Err(error)) => Err(error),
                Err(error) => Err(format!("等待 API quota breakpoint 就绪失败: {error}")),
                Ok(Ok(())) => unreachable!(),
            }
        }
    }
}

pub(super) fn stop(port: u16) -> Result<(), String> {
    let supervisor = supervisors()
        .lock()
        .map_err(|err| format!("API quota supervisor 锁失败: {err}"))?
        .remove(&port);
    let Some(supervisor) = supervisor else {
        return Ok(());
    };
    supervisor.stop.store(true, Ordering::Release);
    supervisor
        .thread
        .join()
        .map_err(|_| "API quota supervisor 线程异常".to_string())?;
    let failure = supervisor
        .failure
        .lock()
        .map_err(|err| format!("API quota supervisor 状态锁失败: {err}"))?
        .clone();
    if let Some(error) = failure {
        return Err(error);
    }
    log_event("codex_api_quota_hook_stopped", json!({ "port": port }));
    Ok(())
}

struct Runtime {
    connection: CdpConnection,
    breakpoint_id: String,
    location: Value,
}

impl Runtime {
    fn connect(websocket_url: &str, stop: &AtomicBool) -> Result<Self, String> {
        let mut connection = CdpConnection::connect(websocket_url)?;
        connection.command("Runtime.enable", json!({}))?;
        connection.command("Debugger.enable", json!({}))?;
        let started = Instant::now();
        while started.elapsed() < DISCOVERY_TIMEOUT && !stop.load(Ordering::Acquire) {
            // The response also drains scriptParsed events queued after Debugger.enable.
            evaluate(&mut connection, "0")?;
            for event in connection.take_events() {
                if event["method"] != "Debugger.scriptParsed" {
                    continue;
                }
                let params = &event["params"];
                let Some(url) = params["url"].as_str().filter(|url| {
                    url.starts_with("app://")
                        && url.contains("/assets/app-primary-")
                        && url.ends_with(".js")
                }) else {
                    continue;
                };
                let source = connection.command(
                    "Debugger.getScriptSource",
                    json!({ "scriptId": params["scriptId"] }),
                )?;
                let source = source["result"]["scriptSource"]
                    .as_str()
                    .ok_or_else(|| "API quota bundle scriptSource 缺失".to_string())?;
                let location = evaluate(
                    &mut connection,
                    &format!("{LOCATOR}({},{})", json!(source), json!(url)),
                )?;
                let possible = connection.command(
                    "Debugger.getPossibleBreakpoints",
                    json!({
                        "start": { "scriptId": params["scriptId"], "lineNumber": location["lineNumber"], "columnNumber": location["columnNumber"] },
                        "end": { "scriptId": params["scriptId"], "lineNumber": location["endLineNumber"], "columnNumber": location["endColumnNumber"] },
                        "restrictToFunction": true
                    }),
                )?;
                let actual = possible["result"]["locations"]
                    .as_array()
                    .and_then(|locations| locations.first())
                    .ok_or_else(|| format!("API quota guard 没有可执行 breakpoint: {location}"))?;
                let armed = connection.command(
                    "Debugger.setBreakpointByUrl",
                    json!({
                        "url": url,
                        "lineNumber": actual["lineNumber"],
                        "columnNumber": actual["columnNumber"],
                        "condition": location["condition"]
                    }),
                )?;
                let id = armed["result"]["breakpointId"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| "API quota breakpointId 缺失".to_string())?;
                if armed["result"]["locations"]
                    .as_array()
                    .is_none_or(Vec::is_empty)
                {
                    return Err(format!("API quota breakpoint 未绑定实际代码: {armed}"));
                }
                let mut report = location;
                report.as_object_mut().unwrap().remove("condition");
                report["actualLocations"] = armed["result"]["locations"].clone();
                return Ok(Self {
                    connection,
                    breakpoint_id: id.to_string(),
                    location: report,
                });
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err("API quota bundle discovery 超时或已取消；未找到 app-primary selector".to_string())
    }

    fn run(&mut self, stop: &AtomicBool) -> Result<(), String> {
        while !stop.load(Ordering::Acquire) {
            for _ in 0..10 {
                if stop.load(Ordering::Acquire) {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(100));
            }
            let error = evaluate(
                &mut self.connection,
                "globalThis.__codexSwitchApiQuotaState?.lastError ?? null",
            )?;
            self.connection.take_events();
            if let Some(error) = error.as_str() {
                return Err(format!("API quota condition 执行失败: {error}"));
            }
        }
        Ok(())
    }

    fn remove(&mut self) -> Result<(), String> {
        self.connection.command(
            "Debugger.removeBreakpoint",
            json!({ "breakpointId": self.breakpoint_id }),
        )?;
        evaluate(
            &mut self.connection,
            "delete globalThis.__codexSwitchApiQuotaState",
        )?;
        Ok(())
    }
}

fn evaluate(connection: &mut CdpConnection, expression: &str) -> Result<Value, String> {
    let response = connection.command(
        "Runtime.evaluate",
        json!({ "expression": expression, "awaitPromise": true, "returnByValue": true }),
    )?;
    if let Some(error) = response["result"].get("exceptionDetails") {
        return Err(format!("API quota Runtime.evaluate 异常: {error}"));
    }
    Ok(response["result"]["result"]["value"].clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the explicit isolated Node inspector fixture; never connects to Codex"]
    fn isolated_api_quota_supervisor() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("isolated fixture installs the same TLS provider as main");
        let url = std::env::var("CODEX_SWITCH_API_QUOTA_TEST_WS")
            .expect("run the opt-in scripts/test-api-quota.mjs CDP fixture");
        let control = std::env::var("CODEX_SWITCH_API_QUOTA_TEST_CONTROL")
            .expect("explicit isolated fixture control URL required");
        assert!(url.starts_with("ws://127.0.0.1:"));
        assert!(control.starts_with("http://127.0.0.1:"));
        let port = url::Url::parse(&url).unwrap().port().unwrap();
        start(port, &url).unwrap();
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        for (provider, reload, blocked) in [
            ("api", false, false),
            ("api", false, false),
            ("openai", false, true),
            ("api", false, false),
            ("api", true, false),
        ] {
            let probe: Value = client
                .get(format!(
                    "{control}/probe?provider={provider}&reload={reload}"
                ))
                .send()
                .unwrap()
                .json()
                .unwrap();
            assert_eq!(probe["blocked"], blocked, "{probe}");
            assert_eq!(probe["sharedAllowed"], false, "{probe}");
            assert!(probe["error"].is_null(), "{probe}");
            println!("API quota live fixture: {probe}");
        }
        stop(port).unwrap();
        let after: Value = client
            .get(format!("{control}/probe?provider=api&reload=false"))
            .send()
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(after["blocked"], true, "{after}");
        assert_eq!(after["sharedAllowed"], false, "{after}");
        println!("API quota after stop: {after}");
    }
}
