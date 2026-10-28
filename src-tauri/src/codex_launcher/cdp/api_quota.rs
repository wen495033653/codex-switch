use super::transport::CdpConnection;
use crate::app_log::log_event;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    net::{Shutdown, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const LOCATOR: &str = include_str!("api_quota.js");
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(20);
const RPC_TIMEOUT: Duration = Duration::from_secs(5);
const REQUIRED: [&str; 2] = ["send_gate", "official_quota_banner"];

#[derive(Clone)]
enum Readiness {
    Starting,
    Ready,
    Failed(String),
    Stopped,
}

struct Control {
    stop: AtomicBool,
    socket: Mutex<Option<TcpStream>>,
    state: Mutex<Readiness>,
    changed: Condvar,
    deadline: Instant,
    progress: Arc<Mutex<String>>,
}

impl Control {
    fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            socket: Mutex::new(None),
            state: Mutex::new(Readiness::Starting),
            changed: Condvar::new(),
            deadline: Instant::now() + DISCOVERY_TIMEOUT,
            progress: Arc::new(Mutex::new("connecting".to_string())),
        }
    }

    fn state(&self, state: Readiness) {
        match self.state.lock() {
            Ok(mut current) => {
                *current = state;
                self.changed.notify_all();
            }
            Err(error) => log_event(
                "codex_api_quota_hook_error",
                json!({"stage":"readiness_state", "error":error.to_string()}),
            ),
        }
    }

    fn publish_ready(&self) -> Result<bool, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|error| format!("发布 API quota readiness 锁失败: {error}"))?;
        if self.stop.load(Ordering::Acquire) || !matches!(*state, Readiness::Starting) {
            return Ok(false);
        }
        *state = Readiness::Ready;
        self.changed.notify_all();
        Ok(true)
    }

    fn cancel(&self) -> Result<(), String> {
        self.stop.store(true, Ordering::Release);
        {
            let mut state = self
                .state
                .lock()
                .map_err(|error| format!("API quota cancellation readiness 锁失败: {error}"))?;
            if !matches!(*state, Readiness::Failed(_)) {
                *state = Readiness::Stopped;
            }
            self.changed.notify_all();
        }
        let socket = self
            .socket
            .lock()
            .map_err(|error| format!("API quota cancellation socket 锁失败: {error}"))?;
        if let Some(socket) = socket.as_ref() {
            if let Err(error) = socket.shutdown(Shutdown::Both) {
                if error.kind() != std::io::ErrorKind::NotConnected {
                    return Err(format!("停止 API quota CDP socket 失败: {error}"));
                }
            }
        }
        Ok(())
    }

    fn wait_ready(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|error| format!("API quota readiness 锁失败: {error}"))?;
        loop {
            match &*state {
                Readiness::Failed(error) => return Err(error.clone()),
                _ if self.stop.load(Ordering::Acquire) => {
                    return Err("API quota startup 已取消".to_string())
                }
                Readiness::Ready => return Ok(()),
                Readiness::Stopped => return Err("API quota startup 已取消".to_string()),
                Readiness::Starting => {}
            }
            let remaining = self
                .deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| {
                    format!(
                        "API quota startup deadline exceeded; {}",
                        self.progress
                            .lock()
                            .map(|progress| progress.clone())
                            .unwrap_or_else(|error| format!("progress lock error={error}"))
                    )
                })?;
            let (next, _) = self
                .changed
                .wait_timeout(state, remaining)
                .map_err(|error| format!("等待 API quota readiness 失败: {error}"))?;
            state = next;
        }
    }
}

struct Supervisor {
    websocket_url: String,
    control: Arc<Control>,
    thread: JoinHandle<()>,
}

fn supervisors() -> &'static Mutex<BTreeMap<u16, Supervisor>> {
    static SUPERVISORS: OnceLock<Mutex<BTreeMap<u16, Supervisor>>> = OnceLock::new();
    SUPERVISORS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(super) fn start(
    port: u16,
    websocket_url: &str,
    reload_owned_launch: bool,
) -> Result<(), String> {
    start_with_reload(port, websocket_url, reload_owned_launch, reload_page)
}

fn main_frame_page(runtime: &mut Runtime, deadline: Instant) -> Result<String, String> {
    let response = runtime.command("Page.getFrameTree", json!({}), deadline)?;
    response["result"]["frameTree"]["frame"]["id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "API quota Page.getFrameTree 主 frameId 缺失".to_string())
}

fn reload_page(runtime: &mut Runtime, deadline: Instant) -> Result<(), String> {
    runtime.command("Page.reload", json!({"ignoreCache":false}), deadline)?;
    Ok(())
}

fn start_with_reload(
    port: u16,
    websocket_url: &str,
    reload_owned_launch: bool,
    reload: fn(&mut Runtime, Instant) -> Result<(), String>,
) -> Result<(), String> {
    start_with_lifecycle(
        port,
        websocket_url,
        reload_owned_launch,
        reload,
        main_frame_page,
    )
}

fn start_with_lifecycle(
    port: u16,
    websocket_url: &str,
    reload_owned_launch: bool,
    reload: fn(&mut Runtime, Instant) -> Result<(), String>,
    main_frame: fn(&mut Runtime, Instant) -> Result<String, String>,
) -> Result<(), String> {
    let mut active = supervisors()
        .lock()
        .map_err(|error| format!("API quota supervisor 锁失败: {error}"))?;
    if let Some(supervisor) = active
        .get(&port)
        .filter(|s| s.websocket_url == websocket_url)
    {
        let control = Arc::clone(&supervisor.control);
        drop(active);
        return control.wait_ready();
    }
    if !reload_owned_launch {
        return Err("[REMOTE_QUOTA_RESTART_REQUIRED] 远控额度兼容首次生效需要通过 Codex Switch 重启 Codex；未刷新当前工作窗口".to_string());
    }
    if let Some(previous) = active.remove(&port) {
        previous.control.cancel()?;
        previous
            .thread
            .join()
            .map_err(|_| "API quota 旧 target 线程异常".to_string())?;
        log_event(
            "codex_api_quota_hook_stopped",
            json!({"port":port,"reason":"renderer_target_changed"}),
        );
    }
    let control = Arc::new(Control::new());
    let worker_control = Arc::clone(&control);
    let url = websocket_url.to_string();
    let worker = thread::Builder::new().name(format!("codex-api-quota-{port}")).spawn(move || {
        let result = Runtime::connect(&url, &worker_control, reload, main_frame).and_then(|mut runtime| {
            if !worker_control.publish_ready()? { return Ok(()); }
            log_event("codex_api_quota_hook_ready", json!({"port":port,"phase":"send_gate_attached","bannerAttached":runtime.seen.contains("official_quota_banner"),"locations":runtime.reports()}));
            runtime.run(&worker_control)
        });
        if worker_control.stop.load(Ordering::Acquire) {
            worker_control.state(Readiness::Stopped);
        } else if let Err(error) = result {
            worker_control.state(Readiness::Failed(error.clone()));
            log_event("codex_api_quota_hook_error", json!({"port":port,"error":error}));
        }
    }).map_err(|error| format!("启动 API quota supervisor 失败: {error}"))?;
    active.insert(
        port,
        Supervisor {
            websocket_url: websocket_url.to_string(),
            control: Arc::clone(&control),
            thread: worker,
        },
    );
    drop(active);
    // Stop/reconcile can now cancel an in-flight RPC rather than waiting behind this lock.
    let result = control.wait_ready();
    if let Err(error) = &result {
        if let Err(cleanup) = control.cancel() {
            return Err(format!("{error}; cancellation={cleanup}"));
        }
    }
    result
}

pub(super) fn stop(port: u16) -> Result<(), String> {
    let supervisor = supervisors()
        .lock()
        .map_err(|error| format!("API quota supervisor 锁失败: {error}"))?
        .remove(&port);
    if let Some(supervisor) = supervisor {
        supervisor.control.cancel()?;
        supervisor
            .thread
            .join()
            .map_err(|_| "API quota supervisor 线程异常".to_string())?;
        log_event("codex_api_quota_hook_stopped", json!({"port":port}));
    }
    Ok(())
}

pub(super) fn stop_all() -> Result<usize, String> {
    let active = std::mem::take(
        &mut *supervisors()
            .lock()
            .map_err(|error| format!("API quota supervisor 锁失败: {error}"))?,
    );
    let count = active.len();
    for supervisor in active.values() {
        supervisor.control.cancel()?;
    }
    for (port, supervisor) in active {
        supervisor
            .thread
            .join()
            .map_err(|_| "API quota supervisor 线程异常".to_string())?;
        log_event("codex_api_quota_hook_stopped", json!({"port":port}));
    }
    Ok(count)
}

struct Armed {
    id: String,
    location: Value,
}
struct Runtime {
    connection: CdpConnection,
    armed: BTreeMap<String, Armed>,
    seen: BTreeSet<String>,
    reload_deadline: Option<Instant>,
    parsed_scripts: usize,
    preloaded_banners: BTreeSet<String>,
    main_frame: String,
    default_context: Option<i64>,
    retired_context: Option<i64>,
    context_events: BTreeMap<&'static str, usize>,
    pending_events: VecDeque<Value>,
    progress: Arc<Mutex<String>>,
}

impl Runtime {
    fn connect(
        websocket_url: &str,
        control: &Control,
        reload: fn(&mut Runtime, Instant) -> Result<(), String>,
        main_frame: fn(&mut Runtime, Instant) -> Result<String, String>,
    ) -> Result<Self, String> {
        let connection =
            CdpConnection::connect_until(websocket_url, rpc_deadline(control.deadline))?;
        *control
            .socket
            .lock()
            .map_err(|error| format!("API quota cancellation socket 锁失败: {error}"))? =
            Some(connection.interrupt_handle()?);
        let mut runtime = Self {
            connection,
            armed: BTreeMap::new(),
            seen: BTreeSet::new(),
            reload_deadline: Some(control.deadline),
            parsed_scripts: 0,
            preloaded_banners: BTreeSet::new(),
            main_frame: String::new(),
            default_context: None,
            retired_context: None,
            context_events: BTreeMap::new(),
            pending_events: VecDeque::new(),
            progress: Arc::clone(&control.progress),
        };
        runtime.command("Runtime.enable", json!({}), control.deadline)?;
        runtime.main_frame = main_frame(&mut runtime, control.deadline)?;
        runtime.wait_default_context(control.deadline)?;
        runtime.command("Debugger.enable", json!({}), control.deadline)?;
        let mut reloaded = false;
        while !control.stop.load(Ordering::Acquire) {
            runtime.evaluate("0", control.deadline)?;
            runtime.process_events(control.deadline)?;
            if runtime.ready() {
                if !reloaded {
                    if !runtime.armed.contains_key("official_quota_banner") {
                        return Err("API quota banner 静态依赖未预挂；未刷新窗口".to_string());
                    }
                    runtime.seen.clear();
                    runtime.retired_context = runtime.default_context.take();
                    runtime.pending_events.clear();
                    reload(&mut runtime, control.deadline)?;
                    runtime.wait_default_context(control.deadline)?;
                    reloaded = true;
                } else {
                    runtime.reload_deadline = None;
                    return Ok(runtime);
                }
            }
            if Instant::now() >= control.deadline {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err(runtime.not_ready_error("startup"))
    }

    fn command(&mut self, method: &str, params: Value, deadline: Instant) -> Result<Value, String> {
        self.connection
            .command_until(method, params, rpc_deadline(deadline))
    }

    fn evaluate(&mut self, expression: &str, deadline: Instant) -> Result<Value, String> {
        let mut context = self.wait_default_context(deadline)?;
        loop {
            let response = match self.command("Runtime.evaluate", json!({"expression":expression,"contextId":context,"awaitPromise":true,"returnByValue":true}), deadline) {
                Ok(response) => response,
                Err(error) => {
                    let protocol = self.connection.take_protocol_error();
                    let mut requested_context_destroyed = false;
                    for event in self.connection.take_events() {
                        requested_context_destroyed |= (event["method"] == "Runtime.executionContextDestroyed" && event["params"]["executionContextId"].as_i64() == Some(context))
                            || (event["method"] == "Runtime.executionContextsCleared" && self.default_context == Some(context));
                        self.accept_event(event);
                    }
                    let context_invalid = protocol.as_ref().is_some_and(|protocol| protocol["code"] == -32000 && protocol["message"].as_str().is_some_and(|message|
                        message == "Cannot find context with specified id" || message == "Cannot find default execution context" || message.starts_with("Execution context was destroyed")));
                    if !requested_context_destroyed || !context_invalid { return Err(error); }
                    let replacement = self.wait_default_context(deadline).map_err(|wait| format!("{error}; navigation_wait={wait}"))?;
                    log_event("codex_api_quota_context_replaced",json!({"oldContext":context,"newContext":replacement,"protocolError":protocol,"error":error}));
                    context = replacement;
                    continue;
                }
            };
            if let Some(error) = response["result"].get("exceptionDetails") {
                return Err(format!("API quota Runtime.evaluate 异常: {error}"));
            }
            return Ok(response["result"]["result"]["value"].clone());
        }
    }

    fn ready(&self) -> bool {
        self.seen.contains("send_gate")
            && self
                .armed
                .get("send_gate")
                .is_some_and(|item| item.location["pending"] == false)
    }
    fn reports(&self) -> Vec<&Value> {
        self.armed.values().map(|item| &item.location).collect()
    }
    fn not_ready_error(&self, stage: &str) -> String {
        let missing: Vec<_> = REQUIRED
            .iter()
            .filter(|kind| !self.seen.contains(**kind))
            .collect();
        format!(
            "API quota {stage} 未就绪: parsedScripts={} missing={missing:?} defaultContextReady={} contextEvents={:?}",
            self.parsed_scripts, self.default_context.is_some(), self.context_events
        )
    }

    fn accept_event(&mut self, event: Value) {
        let method = event["method"].as_str().unwrap_or("");
        match method {
            "Runtime.executionContextsCleared" => {
                *self.context_events.entry("cleared").or_default() += 1;
                self.default_context = None;
                self.pending_events.clear();
                self.seen.clear();
                self.reload_deadline
                    .get_or_insert_with(|| Instant::now() + DISCOVERY_TIMEOUT);
                for armed in self.armed.values_mut() {
                    armed.location["pending"] = json!(true);
                    armed.location["actualLocations"] = json!([]);
                }
            }
            "Runtime.executionContextDestroyed" => {
                *self.context_events.entry("destroyed").or_default() += 1;
                if self.default_context.is_some()
                    && event["params"]["executionContextId"].as_i64() == self.default_context
                {
                    self.default_context = None;
                    self.reload_deadline
                        .get_or_insert_with(|| Instant::now() + DISCOVERY_TIMEOUT);
                }
            }
            "Runtime.executionContextCreated" => {
                *self.context_events.entry("created").or_default() += 1;
                let context = &event["params"]["context"];
                if context["auxData"]["isDefault"] == true {
                    if context["auxData"]["frameId"].as_str() == Some(self.main_frame.as_str()) {
                        self.default_context = context["id"].as_i64();
                        *self
                            .context_events
                            .entry("main_default_created")
                            .or_default() += 1;
                    } else {
                        *self
                            .context_events
                            .entry("foreign_default_created")
                            .or_default() += 1;
                    }
                }
            }
            _ => self.pending_events.push_back(event),
        }
    }

    fn ingest_events(&mut self) {
        for event in self.connection.take_events() {
            self.accept_event(event);
        }
    }

    fn wait_default_context(&mut self, deadline: Instant) -> Result<i64, String> {
        self.ingest_events();
        let deadline = self.reload_deadline.unwrap_or(deadline).min(deadline);
        while self.default_context.is_none() {
            match self.progress.lock() {
                Ok(mut progress) => *progress = self.not_ready_error("await_main_default_context"),
                Err(error) => return Err(format!("API quota context progress 锁失败: {error}")),
            }
            let event = self
                .connection
                .receive_event_until(deadline)
                .map_err(|error| {
                    format!(
                        "{}; {error}",
                        self.not_ready_error("await_main_default_context")
                    )
                })?;
            self.accept_event(event);
        }
        Ok(self.default_context.expect("checked default context"))
    }

    fn process_events(&mut self, deadline: Instant) -> Result<(), String> {
        loop {
            self.ingest_events();
            let Some(event) = self.pending_events.pop_front() else {
                return Ok(());
            };
            if event["method"] == "Debugger.breakpointResolved" {
                for armed in self.armed.values_mut().filter(|item| {
                    item.id == event["params"]["breakpointId"].as_str().unwrap_or("")
                }) {
                    armed.location["pending"] = json!(false);
                    armed.location["actualLocations"] = json!([event["params"]["location"]]);
                }
            }
            if event["method"] != "Debugger.scriptParsed" {
                continue;
            }
            self.parsed_scripts += 1;
            let params = &event["params"];
            let Some(url) = params["url"].as_str().filter(|url| {
                url.starts_with("app://")
                    && url.ends_with(".js")
                    && (url.contains("/assets/app-primary-")
                        || url.contains("/assets/composer-content-"))
            }) else {
                continue;
            };
            let source = self.command(
                "Debugger.getScriptSource",
                json!({"scriptId":params["scriptId"]}),
                deadline,
            )?;
            let source = source["result"]["scriptSource"]
                .as_str()
                .ok_or_else(|| "API quota bundle scriptSource 缺失".to_string())?;
            if url.contains("/assets/composer-content-")
                && !(source.contains("isUsageBannerEnabled:")
                    && source.contains("rateLimitWarningThreshold:"))
            {
                continue;
            }
            let location = self.evaluate(
                &format!("{LOCATOR}({},{})", json!(source), json!(url)),
                deadline,
            )?;
            if location["kind"] == "send_gate" {
                let urls = location["bannerUrls"]
                    .as_array()
                    .ok_or_else(|| "API quota bannerUrls 缺失".to_string())?;
                if urls.is_empty() {
                    return Err("API quota primary 静态依赖没有 composer-content JS".to_string());
                }
                for url in urls {
                    let url = url
                        .as_str()
                        .ok_or_else(|| "API quota banner URL 格式错误".to_string())?;
                    if self.preloaded_banners.contains(url) {
                        continue;
                    }
                    let text = self.evaluate(&format!("(async()=>{{const response=await fetch({},{{signal:AbortSignal.timeout(5000)}});if(!response.ok)throw Error(`quota local asset HTTP ${{response.status}}: ${{response.url}}`);return await response.text();}})()", json!(url)), deadline)?;
                    let text = text
                        .as_str()
                        .ok_or_else(|| "API quota 静态资源文本缺失".to_string())?;
                    if text.contains("isUsageBannerEnabled:")
                        && text.contains("rateLimitWarningThreshold:")
                    {
                        let pending = self.evaluate(
                            &format!("{LOCATOR}({},{})", json!(text), json!(url)),
                            deadline,
                        )?;
                        self.arm(pending, None, deadline)?;
                    }
                    self.preloaded_banners.insert(url.to_string());
                }
            }
            let kind = location["kind"]
                .as_str()
                .ok_or_else(|| "API quota locator kind 缺失".to_string())?
                .to_string();
            self.arm(location, Some(params["scriptId"].clone()), deadline)?;
            let banner_new = kind == "official_quota_banner" && !self.seen.contains(&kind);
            self.seen.insert(kind);
            if banner_new {
                log_event(
                    "codex_api_quota_hook_ready",
                    json!({"phase":"banner_attached","locations":self.reports()}),
                );
            }
        }
    }

    fn arm(
        &mut self,
        location: Value,
        script_id: Option<Value>,
        deadline: Instant,
    ) -> Result<(), String> {
        let kind = location["kind"]
            .as_str()
            .ok_or_else(|| "API quota locator kind 缺失".to_string())?
            .to_string();
        if self.armed.get(&kind).is_some_and(|item| {
            item.location["url"] == location["url"]
                && item.location["lineNumber"] == location["lineNumber"]
                && item.location["columnNumber"] == location["columnNumber"]
        }) {
            return Ok(());
        }
        if let Some(old) = self.armed.remove(&kind) {
            self.command(
                "Debugger.removeBreakpoint",
                json!({"breakpointId":old.id}),
                deadline,
            )?;
        }
        let mut line = location["lineNumber"].clone();
        let mut column = location["columnNumber"].clone();
        if let Some(script_id) = script_id {
            let possible = self.command("Debugger.getPossibleBreakpoints", json!({
                "start":{"scriptId":script_id,"lineNumber":line,"columnNumber":column},
                "end":{"scriptId":script_id,"lineNumber":location["endLineNumber"],"columnNumber":location["endColumnNumber"]},"restrictToFunction":true
            }), deadline)?;
            let actual = possible["result"]["locations"]
                .as_array()
                .and_then(|locations| locations.first())
                .ok_or_else(|| format!("API quota guard 没有可执行 breakpoint: {location}"))?;
            line = actual["lineNumber"].clone();
            column = actual["columnNumber"].clone();
        }
        let armed = self.command("Debugger.setBreakpointByUrl", json!({"url":location["url"],"lineNumber":line,"columnNumber":column,"condition":location["condition"]}), deadline)?;
        let id = armed["result"]["breakpointId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "API quota breakpointId 缺失".to_string())?;
        let pending = armed["result"]["locations"]
            .as_array()
            .is_none_or(Vec::is_empty);
        if kind == "send_gate" && pending {
            return Err(format!("API quota send breakpoint 未绑定实际代码: {armed}"));
        }
        let mut report = location;
        report.as_object_mut().unwrap().remove("condition");
        report["actualLocations"] = armed["result"]["locations"].clone();
        report["pending"] = json!(pending);
        self.armed.insert(
            kind,
            Armed {
                id: id.to_string(),
                location: report,
            },
        );
        Ok(())
    }

    fn run(&mut self, control: &Control) -> Result<(), String> {
        let mut next_probe = Instant::now();
        while !control.stop.load(Ordering::Acquire) {
            let message_deadline = self
                .reload_deadline
                .unwrap_or_else(|| Instant::now() + RPC_TIMEOUT);
            if let Some(event) = self.connection.poll_event_until(
                Instant::now() + Duration::from_millis(100),
                message_deadline,
            )? {
                self.accept_event(event);
            }
            self.ingest_events();
            let deadline = self
                .reload_deadline
                .unwrap_or_else(|| Instant::now() + RPC_TIMEOUT);
            self.wait_default_context(deadline)?;
            self.process_events(deadline)?;
            if Instant::now() >= next_probe {
                let error = self.evaluate(
                    "globalThis.__codexSwitchApiQuotaState?.lastError ?? null",
                    deadline,
                )?;
                if let Some(error) = error.as_str() {
                    return Err(format!("API quota condition 执行失败: {error}"));
                }
                self.process_events(deadline)?;
                next_probe = Instant::now() + Duration::from_secs(1);
            }
            if let Some(deadline) = self.reload_deadline {
                if self.ready() {
                    self.reload_deadline = None;
                } else if Instant::now() >= deadline {
                    return Err(self.not_ready_error("reload"));
                }
            }
        }
        Ok(())
    }
}

fn rpc_deadline(deadline: Instant) -> Instant {
    deadline.min(Instant::now() + RPC_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::super::transport::read_fixture_text;
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
    };

    fn fixture_send(reader: &mut BufReader<TcpStream>, value: Value) {
        let text = value.to_string();
        let mut frame = vec![0x81];
        if text.len() < 126 {
            frame.push(text.len() as u8);
        } else {
            frame.push(126);
            frame.extend_from_slice(&(text.len() as u16).to_be_bytes());
        }
        frame.extend_from_slice(text.as_bytes());
        reader.get_mut().write_all(&frame).unwrap();
    }

    fn fixture_command(reader: &mut BufReader<TcpStream>) -> Value {
        serde_json::from_str(
            &read_fixture_text(reader, Instant::now() + Duration::from_secs(2)).unwrap(),
        )
        .unwrap()
    }

    fn fixture_quiet(reader: &mut BufReader<TcpStream>, duration: Duration) {
        assert!(
            reader.buffer().is_empty(),
            "no early Runtime.evaluate bytes may be buffered"
        );
        reader.get_mut().set_read_timeout(Some(duration)).unwrap();
        match reader.get_ref().peek(&mut [0u8; 1]) {
            Ok(0) => {} // A deadline test may close its socket after the expected error.
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) => {}
            other => panic!("RPC sent before the new main default context existed: {other:?}"),
        }
    }

    fn context_event(id: i64, frame: &str) -> Value {
        json!({"method":"Runtime.executionContextCreated","params":{"context":{"id":id,"origin":"app://-","name":"","auxData":{"isDefault":true,"type":"default","frameId":frame}}}})
    }

    fn context_protocol_case(case: &'static str) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let control = Arc::new(Control::new());
        let worker_control = Arc::clone(&control);
        let (go_tx, go_rx) = mpsc::channel();
        let (sent_tx, sent_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            reader.get_mut().write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n").unwrap();
            let frame = fixture_command(&mut reader);
            assert_eq!(frame["method"], "Page.getFrameTree");
            fixture_send(
                &mut reader,
                json!({"id":frame["id"],"result":{"frameTree":{"frame":{"id":"main-frame"}}}}),
            );
            fixture_send(&mut reader, context_event(11, "main-frame"));
            go_rx.recv().unwrap();
            if case == "inflight_context" || case == "unrelated_rpc_error" {
                let old = fixture_command(&mut reader);
                assert_eq!(old["method"], "Runtime.evaluate");
                assert_eq!(old["params"]["contextId"], 11);
                fixture_send(
                    &mut reader,
                    json!({"method":"Runtime.executionContextDestroyed","params":{"executionContextId":11}}),
                );
                fixture_send(
                    &mut reader,
                    json!({"method":"Runtime.executionContextsCleared","params":{}}),
                );
                fixture_send(&mut reader, context_event(22, "main-frame"));
                let message = if case == "inflight_context" {
                    "Cannot find context with specified id"
                } else {
                    "fixture arbitrary RPC error"
                };
                fixture_send(
                    &mut reader,
                    json!({"id":old["id"],"error":{"code":-32000,"message":message}}),
                );
                if case == "unrelated_rpc_error" {
                    fixture_quiet(&mut reader, Duration::from_millis(50));
                    return;
                }
                let resumed = fixture_command(&mut reader);
                assert_eq!(resumed["method"], "Runtime.evaluate");
                assert_eq!(resumed["params"]["contextId"], 22);
                fixture_send(
                    &mut reader,
                    json!({"id":resumed["id"],"result":{"result":{"type":"boolean","value":true}}}),
                );
                return;
            }
            if case != "normal_refresh" {
                let reload = fixture_command(&mut reader);
                assert_eq!(reload["method"], "Page.reload");
                // A successful reload response does not imply an execution context exists.
                fixture_send(&mut reader, json!({"id":reload["id"],"result":{}}));
            }
            fixture_send(
                &mut reader,
                json!({"method":"Runtime.executionContextDestroyed","params":{"executionContextId":11}}),
            );
            fixture_send(
                &mut reader,
                json!({"method":"Runtime.executionContextsCleared","params":{}}),
            );
            fixture_send(&mut reader, context_event(99, "iframe-frame"));
            sent_tx.send(()).unwrap();
            fixture_quiet(
                &mut reader,
                Duration::from_millis(if case == "missing_context" { 250 } else { 70 }),
            );
            if case == "missing_context" {
                return;
            }
            fixture_send(&mut reader, context_event(22, "main-frame"));
            let evaluate = fixture_command(&mut reader);
            assert_eq!(evaluate["method"], "Runtime.evaluate");
            assert_eq!(
                evaluate["params"]["contextId"], 22,
                "iframe context or retired main context must never be used"
            );
            fixture_send(
                &mut reader,
                json!({"id":evaluate["id"],"result":{"result":{"type":"boolean","value":true}}}),
            );
            if case == "normal_refresh" {
                worker_control.stop.store(true, Ordering::Release);
            }
        });
        let deadline = Instant::now() + Duration::from_millis(200);
        let connection =
            CdpConnection::connect(&format!("ws://127.0.0.1:{port}/context-fixture")).unwrap();
        let mut runtime = Runtime {
            connection,
            armed: BTreeMap::new(),
            seen: BTreeSet::new(),
            reload_deadline: Some(deadline),
            parsed_scripts: 0,
            preloaded_banners: BTreeSet::new(),
            main_frame: String::new(),
            default_context: None,
            retired_context: None,
            context_events: BTreeMap::new(),
            pending_events: VecDeque::new(),
            progress: Arc::clone(&control.progress),
        };
        runtime.main_frame = main_frame_page(&mut runtime, deadline).unwrap();
        assert_eq!(runtime.wait_default_context(deadline).unwrap(), 11);
        go_tx.send(()).unwrap();
        if case == "inflight_context" || case == "unrelated_rpc_error" {
            let result = runtime.evaluate("true", deadline);
            if case == "inflight_context" {
                assert_eq!(result.unwrap(), true);
                assert_eq!(runtime.default_context, Some(22));
            } else {
                let error = result.unwrap_err();
                assert!(error.contains("fixture arbitrary RPC error"), "{error}");
                assert!(error.contains("-32000"), "{error}");
            }
            drop(runtime);
            worker.join().unwrap();
            return;
        }
        if case != "normal_refresh" {
            runtime.retired_context = runtime.default_context.take();
            reload_page(&mut runtime, deadline).unwrap();
        }
        sent_rx.recv().unwrap();
        if case == "missing_context" {
            let started = Instant::now();
            let error = runtime.wait_default_context(deadline).unwrap_err();
            assert!(error.contains("await_main_default_context"), "{error}");
            for evidence in ["cleared", "destroyed", "foreign_default_created"] {
                assert!(error.contains(evidence), "{error}");
            }
            assert!(started.elapsed() < Duration::from_millis(600), "{error}");
            println!("missing main default context fixture: {error}");
        } else if case == "normal_refresh" {
            runtime.run(&control).unwrap();
            assert_eq!(runtime.default_context, Some(22));
        } else {
            assert_eq!(runtime.wait_default_context(deadline).unwrap(), 22);
            assert_eq!(runtime.evaluate("true", deadline).unwrap(), true);
        }
        assert_eq!(runtime.context_events["foreign_default_created"], 1);
        drop(runtime);
        worker.join().unwrap();
    }

    #[test]
    fn owned_reload_waits_for_new_main_default_context_without_early_evaluate() {
        context_protocol_case("owned_reload");
    }

    #[test]
    fn normal_refresh_uses_the_same_context_gate_before_health_evaluate() {
        context_protocol_case("normal_refresh");
    }

    #[test]
    fn missing_main_context_keeps_deadline_and_lifecycle_evidence() {
        context_protocol_case("missing_context");
    }

    #[test]
    fn cancelled_startup_cannot_publish_or_return_ready() {
        let control = Arc::new(Control::new());
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let worker_control = Arc::clone(&control);
        let worker_barrier = Arc::clone(&barrier);
        let worker = thread::spawn(move || {
            worker_barrier.wait();
            worker_control.publish_ready().unwrap()
        });
        control.cancel().unwrap();
        barrier.wait();
        assert!(!worker.join().unwrap());
        assert!(matches!(*control.state.lock().unwrap(), Readiness::Stopped));
        assert!(control.wait_ready().unwrap_err().contains("取消"));
        let ready = Control::new();
        assert!(ready.publish_ready().unwrap());
        ready.cancel().unwrap();
        assert!(ready.wait_ready().unwrap_err().contains("取消"));
        let failed = Control::new();
        failed.state(Readiness::Failed("fixture original failure".into()));
        failed.cancel().unwrap();
        assert_eq!(failed.wait_ready().unwrap_err(), "fixture original failure");
    }

    #[test]
    fn inflight_evaluate_context_loss_follows_confirmed_navigation_to_new_context() {
        context_protocol_case("inflight_context");
    }

    #[test]
    fn navigation_does_not_retry_an_unrelated_rpc_error() {
        context_protocol_case("unrelated_rpc_error");
    }

    // Node has V8 Debugger but not Page.reload. This explicit test adapter recompiles
    // the two app:// fixture scripts; production always sends the real Page.reload.
    fn reload_fixture(runtime: &mut Runtime, deadline: Instant) -> Result<(), String> {
        let context = runtime
            .retired_context
            .expect("fixture owned reload retires its current main context");
        runtime.command("Runtime.evaluate", json!({"expression":"globalThis.fixtureReload()","contextId":context,"awaitPromise":true,"returnByValue":true}), deadline)?;
        let parsed = runtime.connection.take_events();
        runtime.connection.queue_fixture_events([
            json!({"method":"Runtime.executionContextDestroyed","params":{"executionContextId":context}}),
            json!({"method":"Runtime.executionContextsCleared","params":{}}),
            json!({"method":"Runtime.executionContextCreated","params":{"context":{"id":context,"auxData":{"isDefault":true,"type":"default","frameId":"fixture-main"}}}}),
        ]);
        runtime.connection.queue_fixture_events(parsed);
        Ok(())
    }

    fn frame_fixture(runtime: &mut Runtime, _deadline: Instant) -> Result<String, String> {
        let mut events = runtime.connection.take_events();
        let mut count = 0;
        for event in &mut events {
            if event["method"] == "Runtime.executionContextCreated" {
                count += 1;
                event["params"]["context"]["auxData"] =
                    json!({"isDefault":true,"type":"default","frameId":"fixture-main"});
            }
        }
        assert_eq!(
            count, 1,
            "the explicit Node fixture has exactly one main execution context"
        );
        runtime.connection.queue_fixture_events(events);
        Ok("fixture-main".to_string())
    }

    fn probe(client: &reqwest::blocking::Client, control: &str, mode: &str, reload: bool) -> Value {
        client
            .get(format!("{control}/probe?mode={mode}&reload={reload}"))
            .send()
            .unwrap()
            .json()
            .unwrap()
    }

    #[test]
    #[ignore = "requires the explicit isolated Node inspector fixture; never connects to Codex"]
    fn isolated_api_quota_supervisor() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("same TLS provider as main");
        let url =
            std::env::var("CODEX_SWITCH_API_QUOTA_TEST_WS").expect("explicit fixture WS required");
        let control = std::env::var("CODEX_SWITCH_API_QUOTA_TEST_CONTROL")
            .expect("explicit fixture control required");
        let case = std::env::var("CODEX_SWITCH_API_QUOTA_TEST_CASE")
            .expect("explicit fixture case required");
        assert!(url.starts_with("ws://127.0.0.1:"));
        assert!(control.starts_with("http://127.0.0.1:"));
        let port = url::Url::parse(&url).unwrap().port().unwrap();
        let error = start(port, &url, false).unwrap_err();
        assert!(error.contains("REMOTE_QUOTA_RESTART_REQUIRED"), "{error}");
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let warm = probe(&client, &control, "mixed", false);
        assert_eq!(
            warm["blocked"], true,
            "warm derived cache must initially block: {warm}"
        );
        assert_eq!(warm["reloads"], 0, "warm attach must not reload: {warm}");
        if case == "cancel_startup" {
            let thread_url = url.clone();
            let worker = thread::spawn(move || {
                start_with_lifecycle(port, &thread_url, true, reload_fixture, frame_fixture)
            });
            let started = Instant::now();
            while !supervisors().lock().unwrap().contains_key(&port)
                && started.elapsed() < Duration::from_secs(1)
            {
                thread::sleep(Duration::from_millis(5));
            }
            thread::sleep(Duration::from_millis(150));
            assert_eq!(stop_all().unwrap(), 1);
            let error = worker.join().unwrap().unwrap_err();
            assert!(error.contains("取消"), "{error}");
            assert!(started.elapsed() < Duration::from_secs(2));
            println!(
                "API quota cancellation fixture: elapsedMs={} error={error}",
                started.elapsed().as_millis()
            );
            return;
        }
        let started = Instant::now();
        let result = start_with_lifecycle(port, &url, true, reload_fixture, frame_fixture);
        if case == "missing_selector" {
            let error = result.unwrap_err();
            assert!(
                error.contains("rate_limit.allowed guard missing"),
                "{error}"
            );
            assert!(started.elapsed() < Duration::from_secs(2), "{error}");
            assert_eq!(probe(&client, &control, "mixed", false)["reloads"], 0);
            stop_all().unwrap();
            println!(
                "API quota missing selector fixture: elapsedMs={} error={error}",
                started.elapsed().as_millis()
            );
            return;
        }
        result.unwrap();
        start(port, &url, false).unwrap(); // Already ready: no second reload.
        let mut cold = probe(&client, &control, "mixed", false);
        assert_eq!(
            cold["coldBlocked"], false,
            "first post-reload cached read must be unlocked: {cold}"
        );
        assert_eq!(cold["reloads"], 1, "{cold}");
        if case == "lazy_banner" {
            for _ in 0..30 {
                if !cold["firstBanner"].is_null() {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
                cold = probe(&client, &control, "mixed", false);
            }
        }
        assert_eq!(
            cold["firstBanner"], false,
            "pending breakpoint must hide the very first lazy render: {cold}"
        );
        println!("API quota first-render fixture: {cold}");
        for (mode, reload, blocked) in [
            ("mixed", false, false),
            ("mixed", false, false),
            ("pure_api", false, true),
            ("subscription", false, true),
            ("mixed", false, false),
            ("mixed", true, false),
        ] {
            let current = probe(&client, &control, mode, reload);
            assert_eq!(current["blocked"], blocked, "{current}");
            // The lazy module has already been proved on its first render above;
            // a second artificial reload can intentionally defer it again.
            if !current["bannerVisible"].is_null() {
                assert_eq!(current["bannerVisible"], blocked, "{current}");
            }
            assert_eq!(current["sharedAllowed"], false, "{current}");
            assert_eq!(current["sharedBanner"], "usage_limit", "{current}");
            assert!(current["error"].is_null(), "{current}");
            println!("API quota live fixture: {current}");
        }
        assert_eq!(stop_all().unwrap(), 1);
        let after = probe(&client, &control, "pure_api", false);
        assert_eq!(after["blocked"], true, "{after}");
        assert_eq!(after["sharedAllowed"], false, "{after}");
        println!("API quota after stop: {after}");
    }
}
