use super::*;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
};
use url::Url;

pub(crate) const CODEX_PLUGIN_DEBUG_PORT: u16 = 9229;
const CDP_CONNECT_TIMEOUT_MS: u64 = 12_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CodexCdpLaunchHooks {
    pub(crate) plugin_unlock: bool,
    pub(crate) codex_mobile_no_replace: bool,
}

struct CdpScript {
    name: &'static str,
    source: String,
}

struct CdpScriptBundle {
    scripts: Vec<CdpScript>,
}

const CODEX_PLUGIN_UNLOCK_SCRIPT: &str = r###"
(() => {
  const version = "6";
  if (window.__codexSwitchPluginUnlockController?.version === version) {
    window.__codexSwitchPluginUnlockPatch?.();
    return;
  }
  window.__codexSwitchPluginUnlockController?.stop?.();
  window.__codexSwitchPluginUnlockVersion = version;

  const controller = {
    version,
    observer: null,
    interval: null,
    timeout: null,
    stopped: false,
    stop() {
      this.stopped = true;
      this.observer?.disconnect?.();
      if (this.interval) clearInterval(this.interval);
      if (this.timeout) clearTimeout(this.timeout);
    },
  };
  window.__codexSwitchPluginUnlockController = controller;

  const modulePromises = new Map();
  const officialMarketplaceNames = new Set([
    "openai-bundled",
    "openai-curated-remote",
    "openai-primary-runtime",
  ]);

  function record(event, details = {}) {
    const item = { event, details, at: new Date().toISOString() };
    window.__codexSwitchPluginUnlockEvents = window.__codexSwitchPluginUnlockEvents || [];
    window.__codexSwitchPluginUnlockEvents.push(item);
    if (window.__codexSwitchPluginUnlockEvents.length > 40) {
      window.__codexSwitchPluginUnlockEvents.shift();
    }
  }

  function assetUrl(namePart) {
    const urls = [
      ...Array.from(document.querySelectorAll("script[src]") || []).map((script) => script.src),
      ...Array.from(document.querySelectorAll("link[href]") || []).map((link) => link.href),
      ...performance.getEntriesByType("resource").map((entry) => entry.name),
    ].filter(Boolean);
    return urls.find((url) => url.includes("/assets/") && url.includes(namePart) && url.split("?")[0].endsWith(".js")) || "";
  }

  async function loadCodexAppModule(namePart) {
    if (!modulePromises.has(namePart)) {
      const promise = Promise.resolve().then(async () => {
        const url = assetUrl(namePart);
        if (!url) throw new Error(`未找到 Codex App asset: ${namePart}`);
        return await import(url);
      }).catch((error) => {
        modulePromises.delete(namePart);
        throw error;
      });
      modulePromises.set(namePart, promise);
    }
    return await modulePromises.get(namePart);
  }

  function appServerRequestMethod(method, params) {
    if (method === "send-cli-request-for-host" && params?.method) return String(params.method);
    return String(method || "");
  }

  function directParams(value) {
    return value && typeof value === "object" ? value : null;
  }

  function isPluginListMethod(method) {
    return method === "plugin/list" || method === "plugin/installed";
  }

  function isPluginInstallMethod(method) {
    return method === "plugin/install";
  }

  function isPluginUninstallMethod(method) {
    return method === "plugin/uninstall";
  }

  function patchListPluginParams(value) {
    const params = directParams(value);
    if (!params) return value;
    if (isPluginListMethod(params.method) && directParams(params.params)) {
      return { ...params, params: patchListPluginParams(params.params) };
    }
    const hadMarketplaceKinds = Object.prototype.hasOwnProperty.call(params, "marketplaceKinds");
    if (!hadMarketplaceKinds) return params;
    const next = { ...params };
    delete next.marketplaceKinds;
    record("plugin_marketplace_request_expanded", {
      cwdCount: Array.isArray(next.cwds) ? next.cwds.length : 0,
    });
    return next;
  }

  function pluginMarketplaceAliasForName(name) {
    if (name === "openai-curated-remote") return "codex-switch-openai-curated-remote";
    if (name === "openai-primary-runtime") return "codex-switch-openai-primary-runtime";
    return "";
  }

  function restorePluginMarketplaceName(name) {
    if (name === "codex-switch-openai-curated-remote") return "openai-curated-remote";
    if (name === "codex-switch-openai-primary-runtime") return "openai-primary-runtime";
    return name;
  }

  function isOfficialMarketplaceName(name) {
    return officialMarketplaceNames.has(restorePluginMarketplaceName(name));
  }

  function displayNameForMarketplace(name, fallback) {
    if (name === "openai-bundled") return fallback || "OpenAI Bundled";
    if (name === "openai-curated-remote" || name === "codex-switch-openai-curated-remote") return "OpenAI Curated";
    if (name === "openai-primary-runtime" || name === "codex-switch-openai-primary-runtime") return "OpenAI Runtime";
    return fallback;
  }

  function patchMarketplaceObject(marketplace) {
    if (!marketplace || typeof marketplace !== "object" || marketplace.__codexSwitchPluginMarketplacePatched) return false;
    const originalName = String(marketplace.name || "");
    const alias = pluginMarketplaceAliasForName(originalName);
    if (alias) marketplace.name = alias;
    const displayName = displayNameForMarketplace(originalName, marketplace.displayName || marketplace.title || marketplace.label || marketplace.name);
    if (displayName) {
      marketplace.displayName = displayName;
      marketplace.title = displayName;
      marketplace.label = displayName;
      marketplace.interface = {
        ...(marketplace.interface && typeof marketplace.interface === "object" ? marketplace.interface : {}),
        displayName,
        name: displayName,
        title: displayName,
        label: displayName,
      };
    }
    marketplace.__codexSwitchPluginMarketplacePatched = true;
    return true;
  }

  function isBuildFlavorFilter(callback, sample) {
    if (!Array.isArray(sample) || sample.length === 0 || typeof callback !== "function") return false;
    let source = "";
    try {
      source = Function.prototype.toString.call(callback);
    } catch {
      return false;
    }
    if (!source.includes("!u(e.marketplaceName)||e.marketplaceName===r")) return false;
    if (!sample.some((plugin) => isOfficialMarketplaceName(plugin?.marketplaceName))) return false;
    return sample.some((plugin) => isOfficialMarketplaceName(plugin?.marketplaceName) && !callback(plugin));
  }

  function isMarketplaceHiddenFilter(callback, sample) {
    if (!Array.isArray(sample) || sample.length === 0 || typeof callback !== "function") return false;
    let source = "";
    try {
      source = Function.prototype.toString.call(callback);
    } catch {
      return false;
    }
    if (!source.includes("!t.includes(e.name)")) return false;
    if (!sample.some((marketplace) => isOfficialMarketplaceName(marketplace?.name))) return false;
    return sample.some((marketplace) => isOfficialMarketplaceName(marketplace?.name) && !callback(marketplace));
  }

  function installPluginMarketplaceFilterPatch() {
    if (Array.prototype.filter.__codexSwitchPluginMarketplacePatched === version) return;
    const originalFilter = Array.prototype.__codexSwitchPluginMarketplaceOriginalFilter || Array.prototype.filter;
    if (!Array.prototype.__codexSwitchPluginMarketplaceOriginalFilter) {
      Object.defineProperty(Array.prototype, "__codexSwitchPluginMarketplaceOriginalFilter", {
        value: originalFilter,
        configurable: true,
        writable: true,
      });
    }
    const patchedFilter = function codexSwitchPluginMarketplaceFilterPatch(callback, thisArg) {
      if (isBuildFlavorFilter(callback, this)) {
        record("plugin_build_flavor_filter_bypassed", { pluginCount: this.length });
        return Array.from(this);
      }
      if (isMarketplaceHiddenFilter(callback, this)) {
        record("plugin_marketplace_hidden_filter_bypassed", { marketplaceCount: this.length });
        return Array.from(this);
      }
      return originalFilter.call(this, callback, thisArg);
    };
    patchedFilter.__codexSwitchPluginMarketplacePatched = version;
    Array.prototype.filter = patchedFilter;
    record("plugin_marketplace_filter_patch_installed");
  }

  function restorePluginRequestParams(value, method = "") {
    const params = directParams(value);
    if (!params) return value;
    if (params.method && directParams(params.params)) {
      return { ...params, params: restorePluginRequestParams(params.params, String(params.method)) };
    }
    let next = params;
    if (Array.isArray(params.marketplaceKinds)) {
      next = { ...next, marketplaceKinds: Array.from(new Set(params.marketplaceKinds.map((kind) => {
        if (kind === "remote:codex-switch-openai-curated-remote") return "openai-curated-remote";
        if (kind === "remote:codex-switch-openai-primary-runtime") return "openai-primary-runtime";
        return restorePluginMarketplaceName(kind);
      }))) };
    }
    if (isPluginInstallMethod(method)) {
      next = next === params ? { ...params } : { ...next };
      if (next.remoteMarketplaceName) {
        next.remoteMarketplaceName = restorePluginMarketplaceName(next.remoteMarketplaceName);
      }
      if (typeof next.marketplacePath === "string" && next.marketplacePath.startsWith("remote:")) {
        const restoredName = restorePluginMarketplaceName(next.marketplacePath.slice("remote:".length));
        delete next.marketplacePath;
        next.remoteMarketplaceName = restoredName;
      }
    }
    return next;
  }

  function patchPluginMarketplaceResult(method, result) {
    if (!isPluginListMethod(method)) return result;
    try {
      let patchedCount = 0;
      if (Array.isArray(result?.marketplaces)) {
        result.marketplaces.forEach((marketplace) => {
          if (patchMarketplaceObject(marketplace)) patchedCount += 1;
        });
      }
      if (patchedCount > 0) {
        record("plugin_marketplace_response_expanded", { patchedCount });
      }
    } catch (error) {
      record("plugin_marketplace_response_patch_failed", {
        errorName: error?.name || "",
        errorMessage: error?.message || String(error),
      });
    }
    return result;
  }

  function patchRequestClient(client) {
    if (!client || typeof client.sendRequest !== "function") return false;
    if (client.__codexSwitchPluginMarketplacePatch === version) return true;
    const originalSendRequest = client.__codexSwitchPluginMarketplaceOriginalSendRequest || client.sendRequest.bind(client);
    client.__codexSwitchPluginMarketplaceOriginalSendRequest = originalSendRequest;
    client.sendRequest = async function codexSwitchPluginMarketplacePatchedSendRequest(method, params, options) {
      const requestMethod = appServerRequestMethod(String(method || ""), params);
      if (isPluginUninstallMethod(requestMethod)) {
        return await originalSendRequest(method, params, options);
      }
      const restoredParams = restorePluginRequestParams(params, requestMethod);
      const requestParams = isPluginListMethod(requestMethod)
        ? patchListPluginParams(restoredParams)
        : restoredParams;
      const result = await originalSendRequest(method, requestParams, options);
      return patchPluginMarketplaceResult(requestMethod, result);
    };
    client.__codexSwitchPluginMarketplacePatch = version;
    return true;
  }

  async function patchRequestClients() {
    try {
      const module = await loadCodexAppModule("app-server-manager-signals-");
      const candidates = Object.values(module).filter((value) => value && typeof value === "object");
      let patchedCount = 0;
      for (const candidate of candidates) {
        if (patchRequestClient(candidate)) patchedCount += 1;
        if (typeof candidate.sendRequest !== "function" && typeof candidate.get === "function") {
          try {
            if (patchRequestClient(candidate.get())) patchedCount += 1;
          } catch {
          }
        }
      }
      if (patchedCount > 0) {
        record("plugin_marketplace_request_patch_installed", {
          candidateCount: candidates.length,
          patchedCount,
        });
      } else {
        record("plugin_marketplace_request_patch_not_found", {
          exportCount: Object.keys(module || {}).length,
          candidateCount: candidates.length,
        });
      }
    } catch (error) {
      record("plugin_marketplace_request_patch_failed", {
        errorName: error?.name || "",
        errorMessage: error?.message || String(error),
      });
    }
  }

  let patchScheduled = false;
  function patch() {
    patchScheduled = false;
    if (controller.stopped) return;
    installPluginMarketplaceFilterPatch();
    void patchRequestClients();
  }

  function schedulePatch() {
    if (controller.stopped || patchScheduled) return;
    patchScheduled = true;
    controller.timeout = setTimeout(() => {
      controller.timeout = null;
      requestAnimationFrame(patch);
    }, 250);
  }

  window.__codexSwitchPluginUnlockPatch = patch;
  controller.observer = new MutationObserver(schedulePatch);
  controller.observer.observe(document.documentElement, {
    childList: true,
    subtree: true,
  });
  controller.interval = setInterval(schedulePatch, 5000);
  patch();
})();
"###;

const CODEX_MOBILE_NO_REPLACE_SCRIPT: &str = r###"
(() => {
  const version = "1";
  if (window.__codexSwitchCodexMobileNoReplaceController?.version === version) {
    return;
  }
  window.__codexSwitchCodexMobileNoReplaceController?.stop?.();

  const originalReplaceState = history.replaceState;
  const originalPushState = history.pushState;
  const controller = {
    version,
    stopped: false,
    blocked: [],
    stop() {
      if (this.stopped) return;
      this.stopped = true;
      history.replaceState = originalReplaceState;
      history.pushState = originalPushState;
    },
  };
  window.__codexSwitchCodexMobileNoReplaceController = controller;

  function pathFrom(url) {
    if (url == null) return location.pathname;
    try {
      return new URL(String(url), location.href).pathname;
    } catch {
      return "";
    }
  }

  function shouldBlock(url) {
    return location.pathname.startsWith("/codex-mobile") && pathFrom(url) === "/login";
  }

  function recordBlocked(method, url) {
    const item = {
      method,
      url: url == null ? "" : String(url),
      from: location.pathname,
      at: new Date().toISOString(),
    };
    controller.blocked.push(item);
    window.__codexSwitchCodexMobileNoReplaceBlocked = item;
    window.dispatchEvent(new CustomEvent("codex-switch:codex-mobile-no-replace", { detail: item }));
  }

  history.replaceState = function codexSwitchReplaceState(state, title, url) {
    if (!controller.stopped && shouldBlock(url)) {
      recordBlocked("replaceState", url);
      return;
    }
    return originalReplaceState.apply(this, arguments);
  };

  history.pushState = function codexSwitchPushState(state, title, url) {
    if (!controller.stopped && shouldBlock(url)) {
      recordBlocked("pushState", url);
      return;
    }
    return originalPushState.apply(this, arguments);
  };
})();
"###;

pub(crate) fn codex_processes_have_cdp_launch(processes: &[CodexProcess]) -> bool {
    processes
        .iter()
        .any(|process| command_line_has_cdp_launch(&process.command_line))
}

pub(crate) fn inject_codex_mobile_no_replace_hook(
    processes: &[CodexProcess],
) -> Result<usize, String> {
    inject_codex_cdp_hooks(
        processes,
        CodexCdpLaunchHooks {
            plugin_unlock: false,
            codex_mobile_no_replace: true,
        },
    )
}

pub(crate) fn inject_codex_cdp_hooks(
    processes: &[CodexProcess],
    hooks: CodexCdpLaunchHooks,
) -> Result<usize, String> {
    let scripts = cdp_scripts_for_hooks(hooks);
    inject_cdp_script_bundle(processes, &scripts)
}

fn inject_cdp_script_bundle(
    processes: &[CodexProcess],
    scripts: &CdpScriptBundle,
) -> Result<usize, String> {
    if scripts.scripts.is_empty() {
        return Ok(0);
    }
    let ports = processes
        .iter()
        .filter_map(|process| cdp_debug_port_from_command_line(&process.command_line))
        .collect::<BTreeSet<_>>();
    let mut injected = 0usize;
    for port in ports {
        inject_cdp_scripts(port, scripts)?;
        injected += 1;
    }
    Ok(injected)
}

fn command_line_has_cdp_launch(command_line: &str) -> bool {
    let normalized = command_line.to_ascii_lowercase();
    normalized.contains("--remote-debugging-port")
        && normalized.contains("--remote-allow-origins=http://127.0.0.1:")
}

fn cdp_debug_port_from_command_line(command_line: &str) -> Option<u16> {
    let normalized = command_line.to_ascii_lowercase();
    let index = normalized.find("--remote-debugging-port")?;
    let rest = &command_line[index + "--remote-debugging-port".len()..];
    let rest = rest.trim_start();
    let value = if let Some(rest) = rest.strip_prefix('=') {
        rest.trim_start()
    } else {
        rest
    };
    let digits = value
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u16>().ok()
}

pub(crate) fn launch_codex_with_cdp_hooks(
    executable_path: &Path,
    hooks: CodexCdpLaunchHooks,
) -> Result<(), String> {
    launch_codex_with_cdp_hooks_with_options(executable_path, hooks, &[], &[])
}

pub(crate) fn launch_codex_with_cdp_hooks_with_options(
    executable_path: &Path,
    hooks: CodexCdpLaunchHooks,
    args: &[String],
    envs: &[(String, String)],
) -> Result<(), String> {
    launch_codex_with_cdp_hooks_with_options_and_failure_action(
        executable_path,
        hooks,
        args,
        envs,
        CdpHookFailureAction::KillProcess,
    )
    .map(|_| ())
}

pub(crate) fn launch_codex_with_optional_cdp_hooks_with_options(
    executable_path: &Path,
    hooks: CodexCdpLaunchHooks,
    args: &[String],
    envs: &[(String, String)],
) -> Result<Option<String>, String> {
    launch_codex_with_cdp_hooks_with_options_and_failure_action(
        executable_path,
        hooks,
        args,
        envs,
        CdpHookFailureAction::KeepProcess,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CdpHookFailureAction {
    KillProcess,
    KeepProcess,
}

fn launch_codex_with_cdp_hooks_with_options_and_failure_action(
    executable_path: &Path,
    hooks: CodexCdpLaunchHooks,
    args: &[String],
    envs: &[(String, String)],
    failure_action: CdpHookFailureAction,
) -> Result<Option<String>, String> {
    if !executable_path.exists() {
        return Err(format!("Codex 路径不存在: {}", executable_path.display()));
    }

    let debug_port = select_loopback_port(CODEX_PLUGIN_DEBUG_PORT)?;
    let scripts = cdp_scripts_for_hooks(hooks);
    let mut command = Command::new(executable_path);
    command
        .arg(format!("--remote-debugging-port={debug_port}"))
        .arg(format!(
            "--remote-allow-origins=http://127.0.0.1:{debug_port}"
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for arg in args {
        command.arg(arg);
    }
    for (name, value) in envs {
        command.env(name, value);
    }
    if let Some(parent) = executable_path.parent() {
        command.current_dir(parent);
    }
    sanitize_desktop_app_launch_env(&mut command);
    hide_command_window(&mut command);

    let mut child = command
        .spawn()
        .map_err(|err| format!("启动 Codex hook 模式失败: {err}"))?;
    if let Err(err) = wait_and_inject_cdp_scripts(debug_port, &scripts) {
        if failure_action == CdpHookFailureAction::KillProcess {
            let _ = child.kill();
            return Err(err);
        }
        return Ok(Some(err));
    }
    Ok(None)
}

fn cdp_scripts_for_hooks(hooks: CodexCdpLaunchHooks) -> CdpScriptBundle {
    let mut scripts = Vec::new();
    if hooks.plugin_unlock {
        scripts.push(CdpScript {
            name: "plugin_unlock",
            source: CODEX_PLUGIN_UNLOCK_SCRIPT.to_string(),
        });
    }
    if hooks.codex_mobile_no_replace {
        scripts.push(CdpScript {
            name: "codex_mobile_no_replace",
            source: CODEX_MOBILE_NO_REPLACE_SCRIPT.to_string(),
        });
    }
    CdpScriptBundle { scripts }
}

fn select_loopback_port(requested: u16) -> Result<u16, String> {
    if requested != 0 && TcpListener::bind(("127.0.0.1", requested)).is_ok() {
        return Ok(requested);
    }
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|err| format!("分配 CDP 端口失败: {err}"))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|err| format!("读取 CDP 端口失败: {err}"))
}

fn wait_and_inject_cdp_scripts(port: u16, bundle: &CdpScriptBundle) -> Result<(), String> {
    let started = Instant::now();
    let timeout = StdDuration::from_millis(CDP_CONNECT_TIMEOUT_MS);
    let mut last_error = String::new();

    while started.elapsed() < timeout {
        match inject_cdp_scripts(port, bundle) {
            Ok(()) => return Ok(()),
            Err(err) => last_error = err,
        }
        thread::sleep(StdDuration::from_millis(250));
    }

    Err(if last_error.is_empty() {
        "等待 Codex CDP 端口超时".to_string()
    } else {
        format!("注入 Codex hook 脚本失败: {last_error}")
    })
}

fn inject_cdp_scripts(port: u16, bundle: &CdpScriptBundle) -> Result<(), String> {
    let websocket_url = page_websocket_url(port)?;
    let mut ws = connect_websocket(&websocket_url)?;
    let mut command_id = 1;
    for script in &bundle.scripts {
        send_cdp_command(
            &mut ws,
            command_id,
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": &script.source }),
        )
        .map_err(|err| format!("注入 {} new-document 脚本失败: {err}", script.name))?;
        command_id += 1;
    }
    for script in &bundle.scripts {
        send_cdp_command(
            &mut ws,
            command_id,
            "Runtime.evaluate",
            json!({
                "expression": &script.source,
                "awaitPromise": false,
                "allowUnsafeEvalBlockedByCSP": true
            }),
        )
        .map_err(|err| format!("执行 {} 脚本失败: {err}", script.name))?;
        command_id += 1;
    }
    Ok(())
}

fn page_websocket_url(port: u16) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(StdDuration::from_millis(700))
        .no_proxy()
        .build()
        .map_err(|err| format!("创建 CDP HTTP client 失败: {err}"))?;
    let targets = client
        .get(format!("http://127.0.0.1:{port}/json/list"))
        .send()
        .map_err(|err| format!("连接 CDP target 列表失败: {err}"))?
        .json::<Value>()
        .map_err(|err| format!("解析 CDP target 列表失败: {err}"))?;
    let pages = targets
        .as_array()
        .ok_or_else(|| "CDP target 列表格式无效".to_string())?;

    let first_page = pages.iter().find(|target| {
        target.get("type").and_then(Value::as_str) == Some("page")
            && target
                .get("webSocketDebuggerUrl")
                .and_then(Value::as_str)
                .is_some()
    });
    let codex_page = pages
        .iter()
        .find(|target| is_codex_desktop_page_target(target));

    codex_page
        .or(first_page)
        .and_then(|target| target.get("webSocketDebuggerUrl").and_then(Value::as_str))
        .map(str::to_string)
        .ok_or_else(|| "未找到可注入的 Codex 页面".to_string())
}

fn is_codex_desktop_page_target(target: &Value) -> bool {
    if target.get("type").and_then(Value::as_str) != Some("page")
        || target
            .get("webSocketDebuggerUrl")
            .and_then(Value::as_str)
            .is_none()
    {
        return false;
    }
    let title = target.get("title").and_then(Value::as_str).unwrap_or("");
    let url = target.get("url").and_then(Value::as_str).unwrap_or("");
    let identity = format!(
        "{} {}",
        title.to_ascii_lowercase(),
        url.to_ascii_lowercase()
    );
    identity.contains("codex") || identity.contains("chatgpt")
}

fn connect_websocket(websocket_url: &str) -> Result<TcpStream, String> {
    let url = Url::parse(websocket_url).map_err(|err| format!("CDP WebSocket URL 无效: {err}"))?;
    if url.scheme() != "ws" {
        return Err("CDP WebSocket 仅支持 ws://".to_string());
    }
    let host = url
        .host_str()
        .ok_or_else(|| "CDP WebSocket URL 缺少 host".to_string())?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "CDP WebSocket URL 缺少端口".to_string())?;
    let mut path = url.path().to_string();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    if path.is_empty() {
        path.push('/');
    }

    let mut stream = TcpStream::connect((host, port))
        .map_err(|err| format!("连接 CDP WebSocket 失败: {err}"))?;
    stream
        .set_read_timeout(Some(StdDuration::from_secs(5)))
        .map_err(|err| format!("设置 CDP WebSocket read timeout 失败: {err}"))?;
    stream
        .set_write_timeout(Some(StdDuration::from_secs(5)))
        .map_err(|err| format!("设置 CDP WebSocket write timeout 失败: {err}"))?;

    let key = BASE64_STANDARD.encode([0u8; 16]);
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("发送 CDP WebSocket 握手失败: {err}"))?;

    let mut response = Vec::new();
    let mut buffer = [0u8; 512];
    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|err| format!("读取 CDP WebSocket 握手失败: {err}"))?;
        if read == 0 {
            return Err("CDP WebSocket 握手提前关闭".to_string());
        }
        response.extend_from_slice(&buffer[..read]);
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if response.len() > 8192 {
            return Err("CDP WebSocket 握手响应过大".to_string());
        }
    }
    let response_text = String::from_utf8_lossy(&response);
    if !response_text.starts_with("HTTP/1.1 101") && !response_text.starts_with("HTTP/1.0 101") {
        return Err("CDP WebSocket 握手未升级协议".to_string());
    }

    Ok(stream)
}

fn send_cdp_command(
    stream: &mut TcpStream,
    id: i64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let payload = json!({
        "id": id,
        "method": method,
        "params": params
    });
    send_ws_text(stream, &payload.to_string())?;

    loop {
        let message = read_ws_text(stream)?;
        let value = serde_json::from_str::<Value>(&message)
            .map_err(|err| format!("解析 CDP 响应失败: {err}"))?;
        if value.get("id").and_then(Value::as_i64) != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(format!("CDP {method} 失败: {error}"));
        }
        return Ok(value);
    }
}

fn send_ws_text(stream: &mut TcpStream, text: &str) -> Result<(), String> {
    send_ws_frame(stream, 0x1, text.as_bytes())
}

fn send_ws_frame(stream: &mut TcpStream, opcode: u8, payload: &[u8]) -> Result<(), String> {
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x80 | opcode);
    let len = payload.len();
    if len < 126 {
        frame.push(0x80 | len as u8);
    } else if len <= u16::MAX as usize {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(0x80 | 127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }

    let mask = [0x13u8, 0x37, 0x5a, 0xc0];
    frame.extend_from_slice(&mask);
    for (index, byte) in payload.iter().enumerate() {
        frame.push(byte ^ mask[index % 4]);
    }

    stream
        .write_all(&frame)
        .map_err(|err| format!("发送 CDP WebSocket frame 失败: {err}"))
}

fn read_ws_text(stream: &mut TcpStream) -> Result<String, String> {
    loop {
        let mut header = [0u8; 2];
        stream
            .read_exact(&mut header)
            .map_err(|err| format!("读取 CDP WebSocket frame 失败: {err}"))?;
        let opcode = header[0] & 0x0f;
        let masked = header[1] & 0x80 != 0;
        let mut len = u64::from(header[1] & 0x7f);
        if len == 126 {
            let mut buffer = [0u8; 2];
            stream
                .read_exact(&mut buffer)
                .map_err(|err| format!("读取 CDP WebSocket frame 长度失败: {err}"))?;
            len = u64::from(u16::from_be_bytes(buffer));
        } else if len == 127 {
            let mut buffer = [0u8; 8];
            stream
                .read_exact(&mut buffer)
                .map_err(|err| format!("读取 CDP WebSocket frame 长度失败: {err}"))?;
            len = u64::from_be_bytes(buffer);
        }

        let mut mask = [0u8; 4];
        if masked {
            stream
                .read_exact(&mut mask)
                .map_err(|err| format!("读取 CDP WebSocket mask 失败: {err}"))?;
        }
        if len > 16 * 1024 * 1024 {
            return Err("CDP WebSocket frame 过大".to_string());
        }
        let mut payload = vec![0u8; len as usize];
        stream
            .read_exact(&mut payload)
            .map_err(|err| format!("读取 CDP WebSocket payload 失败: {err}"))?;
        if masked {
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % 4];
            }
        }

        match opcode {
            0x1 => {
                return String::from_utf8(payload)
                    .map_err(|err| format!("CDP WebSocket 文本不是 UTF-8: {err}"));
            }
            0x8 => return Err("CDP WebSocket 已关闭".to_string()),
            0x9 => {
                send_ws_frame(stream, 0xA, &payload)?;
            }
            0xA => {}
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_has_cdp_launch_requires_cdp_launch_args() {
        assert!(command_line_has_cdp_launch(
            r#""C:\Codex\codex.exe" --remote-debugging-port=9229 --remote-allow-origins=http://127.0.0.1:9229"#
        ));
        assert!(!command_line_has_cdp_launch(
            r#""C:\Codex\codex.exe" --remote-debugging-port=9229"#
        ));
        assert!(!command_line_has_cdp_launch(r#""C:\Codex\codex.exe""#));
    }

    #[test]
    fn cdp_debug_port_parses_equals_and_space_forms() {
        assert_eq!(
            cdp_debug_port_from_command_line(
                r#""C:\Codex\codex.exe" --remote-debugging-port=9229 --remote-allow-origins=http://127.0.0.1:9229"#
            ),
            Some(9229)
        );
        assert_eq!(
            cdp_debug_port_from_command_line(
                r#""C:\Codex\codex.exe" --remote-debugging-port 9333 --remote-allow-origins=http://127.0.0.1:9333"#
            ),
            Some(9333)
        );
        assert_eq!(
            cdp_debug_port_from_command_line(r#""C:\Codex\codex.exe""#),
            None
        );
    }

    #[test]
    fn select_loopback_port_zero_allocates_actual_port() {
        let port = select_loopback_port(0).expect("ephemeral port should be allocated");

        assert_ne!(port, 0);
    }

    #[test]
    fn codex_page_target_accepts_chatgpt_hosted_page() {
        assert!(is_codex_desktop_page_target(&json!({
            "type": "page",
            "title": "ChatGPT",
            "url": "app://codex/index.html",
            "webSocketDebuggerUrl": "ws://127.0.0.1:9229/devtools/page/1"
        })));
        assert!(!is_codex_desktop_page_target(&json!({
            "type": "page",
            "title": "Settings",
            "url": "app://settings/index.html",
            "webSocketDebuggerUrl": "ws://127.0.0.1:9229/devtools/page/2"
        })));
    }

    #[test]
    fn plugin_unlock_script_supports_current_app_server_rpc_names() {
        assert!(CODEX_PLUGIN_UNLOCK_SCRIPT.contains("plugin/list"));
        assert!(CODEX_PLUGIN_UNLOCK_SCRIPT.contains("plugin/installed"));
        assert!(CODEX_PLUGIN_UNLOCK_SCRIPT.contains("plugin/install"));
        assert!(CODEX_PLUGIN_UNLOCK_SCRIPT.contains("plugin/uninstall"));
        assert!(CODEX_PLUGIN_UNLOCK_SCRIPT.contains("openai-curated-remote"));
        assert!(CODEX_PLUGIN_UNLOCK_SCRIPT.contains("codex-switch-openai-curated-remote"));
        assert!(!CODEX_PLUGIN_UNLOCK_SCRIPT.contains("list-plugins"));
        assert!(!CODEX_PLUGIN_UNLOCK_SCRIPT.contains("install-plugin"));
        assert!(!CODEX_PLUGIN_UNLOCK_SCRIPT.contains("uninstall-plugin"));
    }
}
