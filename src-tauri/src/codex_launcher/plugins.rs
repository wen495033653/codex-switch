use super::*;
use crate::{json_util::bool_field, session_manager, settings::read_settings_value};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde::Deserialize;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::{AtomicI64, Ordering},
};
use url::Url;

pub(crate) const CODEX_PLUGIN_DEBUG_PORT: u16 = 9229;
const CDP_CONNECT_TIMEOUT_MS: u64 = 12_000;
const CODEX_DELETE_BINDING_NAME: &str = "codexSwitchDeleteBridgeV1";
static CDP_BRIDGE_COMMAND_ID: AtomicI64 = AtomicI64::new(100);
const CODEX_PLUGIN_UNLOCK_SCRIPT: &str = r###"
(() => {
  const version = "4";
  if (window.__codexSwitchPluginUnlockController?.version === version) {
    window.__codexSwitchPluginUnlockScan?.();
    return;
  }
  window.__codexSwitchPluginUnlockController?.stop?.();
  window.__codexSwitchPluginUnlockVersion = version;

  const selectors = {
    disabledInstallButton: 'button:disabled.w-full.justify-center, [role="button"][aria-disabled="true"].cursor-not-allowed',
    pluginNavButton: 'button.h-token-nav-row.w-full',
    pluginSvgPath: 'svg path[d^="M7.94562 14.0277"]',
  };
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

  function reactFiberFrom(element) {
    const key = Object.keys(element || {}).find((name) => name.startsWith("__reactFiber"));
    return key ? element[key] : null;
  }

  function authContextValueFrom(element) {
    for (let fiber = reactFiberFrom(element); fiber; fiber = fiber.return) {
      for (const value of [fiber.memoizedProps?.value, fiber.pendingProps?.value]) {
        if (value && typeof value === "object" && typeof value.setAuthMethod === "function" && "authMethod" in value) {
          return value;
        }
      }
    }
    return null;
  }

  function spoofChatGPTAuthMethod(element) {
    const auth = authContextValueFrom(element);
    if (!auth || auth.authMethod === "chatgpt") return false;
    auth.setAuthMethod("chatgpt");
    return true;
  }

  function pluginEntryButton() {
    const byIcon = document.querySelector(`${selectors.pluginNavButton} ${selectors.pluginSvgPath}`)?.closest("button");
    if (byIcon) return byIcon;
    return Array.from(document.querySelectorAll(selectors.pluginNavButton))
      .find((button) => /^(插件|Plugins)(\s+-\s+.*)?$/i.test((button.textContent || "").trim())) || null;
  }

  function enablePluginEntry() {
    const button = pluginEntryButton();
    if (!button) return;
    spoofChatGPTAuthMethod(button);
    button.disabled = false;
    button.removeAttribute("disabled");
    button.style.display = "";
    button.querySelectorAll("*").forEach((node) => {
      node.style.display = "";
    });
    const reactPropsKey = Object.keys(button).find((key) => key.startsWith("__reactProps"));
    if (reactPropsKey) {
      button[reactPropsKey].disabled = false;
    }
    if (button.dataset.codexSwitchPluginEnabled !== "true") {
      button.dataset.codexSwitchPluginEnabled = "true";
      button.addEventListener("click", () => {
        spoofChatGPTAuthMethod(button);
      }, true);
    }
  }

  function unblockButtonElement(button) {
    button.disabled = false;
    button.removeAttribute("disabled");
    button.removeAttribute("aria-disabled");
    button.classList.remove("disabled", "opacity-50", "cursor-not-allowed", "pointer-events-none");
    button.style.pointerEvents = "auto";
    button.tabIndex = 0;
    const reactPropsKey = Object.keys(button).find((key) => key.startsWith("__reactProps"));
    if (reactPropsKey) {
      button[reactPropsKey].disabled = false;
      button[reactPropsKey]["aria-disabled"] = false;
    }
  }

  function unblockPluginInstallButtons() {
    Array.from(document.querySelectorAll(selectors.disabledInstallButton)).forEach((button) => {
      const text = (button.textContent || "").trim();
      if (!/^安装\s/.test(text) && !/^Install\s/.test(text)) return;
      unblockButtonElement(button);
    });
  }

  let scanScheduled = false;
  let lastScanAt = 0;
  function scan() {
    scanScheduled = false;
    lastScanAt = Date.now();
    if (controller.stopped) return;
    try {
      enablePluginEntry();
      unblockPluginInstallButtons();
    } catch (error) {
      window.__codexSwitchPluginUnlockErrors = window.__codexSwitchPluginUnlockErrors || [];
      window.__codexSwitchPluginUnlockErrors.push(String(error?.stack || error));
    }
  }

  function scheduleScan() {
    if (controller.stopped) return;
    if (scanScheduled) return;
    scanScheduled = true;
    const delay = Math.max(0, 600 - (Date.now() - lastScanAt));
    controller.timeout = setTimeout(() => {
      controller.timeout = null;
      requestAnimationFrame(scan);
    }, delay);
  }

  window.__codexSwitchPluginUnlockScan = scan;
  controller.observer = new MutationObserver(scheduleScan);
  controller.observer.observe(document.documentElement, {
    childList: true,
    subtree: true,
  });
  controller.interval = setInterval(scheduleScan, 8000);
  scan();
})();
"###;

const CODEX_DELETE_BUTTON_SCRIPT: &str = r###"
(() => {
  const version = "4";
  if (window.__codexSwitchDeleteButtonController?.version === version) {
    window.__codexSwitchDeleteButtonScan?.();
    return;
  }
  window.__codexSwitchDeleteButtonController?.stop?.();

  const selectors = {
    row: "[data-app-action-sidebar-thread-id]",
    archiveButton: 'button[aria-label="归档对话"], button[aria-label="Archive conversation"]',
    title: "[data-thread-title], .truncate.select-none, .truncate.text-base",
  };
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
  window.__codexSwitchDeleteButtonController = controller;

  function showToast(message) {
    document.querySelectorAll(".codex-switch-delete-toast").forEach((node) => node.remove());
    const toast = document.createElement("div");
    toast.className = "codex-switch-delete-toast";
    toast.textContent = message;
    Object.assign(toast.style, {
      position: "fixed",
      right: "18px",
      bottom: "18px",
      zIndex: "2147483647",
      padding: "9px 12px",
      borderRadius: "8px",
      background: "rgba(17, 24, 39, .92)",
      color: "#fff",
      fontSize: "13px",
      lineHeight: "18px",
      boxShadow: "0 8px 24px rgba(0,0,0,.22)",
      maxWidth: "320px",
      pointerEvents: "none",
    });
    document.body.appendChild(toast);
    setTimeout(() => toast.remove(), 4200);
  }

  function rowHref(row) {
    return row.getAttribute("href") || row.querySelector("a")?.getAttribute("href") || "";
  }

  function isCurrentSessionRow(row, ref) {
    if (row.getAttribute("aria-current") === "page" || row.getAttribute("aria-current") === "true") return true;
    const href = rowHref(row);
    if (href) {
      try {
        const url = new URL(href, window.location.href);
        if (url.href === window.location.href || url.pathname === window.location.pathname) return true;
      } catch {
        if (window.location.href.includes(href)) return true;
      }
    }
    return !!ref.session_id && window.location.href.includes(ref.session_id);
  }

  function sessionRefFromRow(row) {
    const href = rowHref(row);
    const idMatch = href.match(/(?:session|conversation|thread)[=/:-]([A-Za-z0-9_.-]+)/i) || href.match(/([A-Za-z0-9_-]{8,})$/);
    const sessionId = row.getAttribute("data-app-action-sidebar-thread-id") || (idMatch && idMatch[1]) || "";
    const titleNode = row.querySelector(selectors.title);
    const rawTitle = titleNode?.textContent || row.textContent || "Untitled session";
    const title = rawTitle.replace(/\s*(删除|归档|置顶|取消置顶)\s*$/g, "").trim().slice(0, 160);
    return { session_id: sessionId, title };
  }

  function trashIcon() {
    return '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 6h18"></path><path d="M8 6V4h8v2"></path><path d="M19 6l-1 14H6L5 6"></path><path d="M10 11v5"></path><path d="M14 11v5"></path></svg>';
  }

  function stopButtonEvent(event) {
    event.preventDefault();
    event.stopPropagation();
    event.stopImmediatePropagation?.();
  }

  function releaseFocus(row, button) {
    button.blur();
    if (row.contains(document.activeElement)) {
      document.activeElement.blur();
    }
  }

  function buildDeleteButton(archiveButton) {
    const button = archiveButton.cloneNode(false);
    button.type = "button";
    button.className = archiveButton.className;
    button.dataset.codexSwitchDeleteButton = "true";
    button.dataset.codexSwitchDeleteButtonVersion = version;
    button.disabled = false;
    button.removeAttribute("disabled");
    button.setAttribute("aria-label", "删除对话");
    button.title = "删除";
    button.innerHTML = trashIcon();
    button.style.display = "";
    button.style.pointerEvents = "auto";
    return button;
  }

  async function requestDelete(ref) {
    if (typeof window.__codexSwitchDeleteBridge !== "function") {
      throw new Error("删除桥接不可用，请重启 Codex app");
    }
    return await window.__codexSwitchDeleteBridge(ref);
  }

  async function deleteRow(row, button, ref, event) {
    stopButtonEvent(event);
    releaseFocus(row, button);
    if (button.dataset.codexSwitchDeleteBusy === "true") return;
    if (!ref.session_id) {
      showToast("删除失败：未找到会话 ID");
      return;
    }
    if (!window.confirm(`确定删除会话“${ref.title || ref.session_id}”？`)) return;
    button.disabled = true;
    button.dataset.codexSwitchDeleteBusy = "true";
    try {
      const result = await requestDelete(ref);
      if (result.status === "local_deleted") {
        const shouldReload = isCurrentSessionRow(row, ref);
        row.remove();
        showToast(result.message || "删除成功");
        if (shouldReload) window.location.reload();
      } else {
        showToast(result.message || "删除失败");
      }
    } catch (error) {
      showToast(error?.message || "删除失败");
    } finally {
      delete button.dataset.codexSwitchDeleteBusy;
      if (button.isConnected) button.disabled = false;
    }
  }

  function attachDeleteButton(row) {
    const archiveButton = row.querySelector(selectors.archiveButton);
    if (!archiveButton) return;
    const existing = row.querySelector('[data-codex-switch-delete-button="true"]');
    if (existing?.dataset.codexSwitchDeleteButtonVersion === version) return;
    existing?.remove();
    const ref = sessionRefFromRow(row);
    if (!ref.session_id) return;
    const button = buildDeleteButton(archiveButton);
    ["pointerdown", "mousedown", "mouseup", "touchstart"].forEach((eventName) => {
      button.addEventListener(eventName, stopButtonEvent, true);
    });
    const onActivate = (event) => deleteRow(row, button, ref, event);
    button.addEventListener("click", onActivate, true);
    if (archiveButton.parentElement) {
      archiveButton.parentElement.insertBefore(button, archiveButton);
    } else {
      archiveButton.before(button);
    }
  }

  let scanScheduled = false;
  function scan() {
    scanScheduled = false;
    if (controller.stopped) return;
    try {
      document.querySelectorAll(selectors.row).forEach(attachDeleteButton);
    } catch (error) {
      window.__codexSwitchDeleteButtonErrors = window.__codexSwitchDeleteButtonErrors || [];
      window.__codexSwitchDeleteButtonErrors.push(String(error?.stack || error));
    }
  }

  function scheduleScan() {
    if (controller.stopped || scanScheduled) return;
    scanScheduled = true;
    controller.timeout = setTimeout(() => {
      controller.timeout = null;
      requestAnimationFrame(scan);
    }, 200);
  }

  window.__codexSwitchDeleteButtonScan = scan;
  controller.observer = new MutationObserver(scheduleScan);
  controller.observer.observe(document.documentElement, {
    childList: true,
    subtree: true,
  });
  controller.interval = setInterval(scheduleScan, 5000);
  scan();
})();
"###;

const CODEX_DELETE_BRIDGE_SCRIPT: &str = r###"
(() => {
  const bindingName = __CODEX_SWITCH_DELETE_BINDING_NAME__;
  window.__codexSwitchDeleteCallbacks = window.__codexSwitchDeleteCallbacks || new Map();
  window.__codexSwitchDeleteSeq = window.__codexSwitchDeleteSeq || 0;
  window.__codexSwitchDeleteResolve = (id, result) => {
    const callback = window.__codexSwitchDeleteCallbacks.get(id);
    if (!callback) return;
    window.__codexSwitchDeleteCallbacks.delete(id);
    callback.resolve(result);
  };
  window.__codexSwitchDeleteReject = (id, message) => {
    const callback = window.__codexSwitchDeleteCallbacks.get(id);
    if (!callback) return;
    window.__codexSwitchDeleteCallbacks.delete(id);
    callback.resolve({ status: "failed", message });
  };
  window.__codexSwitchDeleteBridge = (payload) => new Promise((resolve) => {
    const id = String(++window.__codexSwitchDeleteSeq);
    window.__codexSwitchDeleteCallbacks.set(id, { resolve });
    window[bindingName](JSON.stringify({ id, payload }));
  });
})();
"###;

#[derive(Deserialize)]
struct DeleteBridgeRequest {
    session_id: String,
    title: Option<String>,
}

#[derive(Deserialize)]
struct DeleteBridgeEnvelope {
    id: String,
    payload: DeleteBridgeRequest,
}

struct CodexEnhancementScript {
    source: String,
    delete_bridge_enabled: bool,
}

pub(crate) fn launch_codex_with_plugins(executable_path: &Path) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("Codex app 增强目前仅支持 Windows 重启入口".to_string());
    }
    if !executable_path.exists() {
        return Err(format!(
            "Codex app 路径不存在: {}",
            executable_path.display()
        ));
    }

    let enhancement = build_codex_enhancement_script()?;
    let debug_port = select_loopback_port(CODEX_PLUGIN_DEBUG_PORT)?;
    let mut command = Command::new(executable_path);
    command
        .arg(format!("--remote-debugging-port={debug_port}"))
        .arg(format!(
            "--remote-allow-origins=http://127.0.0.1:{debug_port}"
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(parent) = executable_path.parent() {
        command.current_dir(parent);
    }
    hide_command_window(&mut command);

    let mut child = command
        .spawn()
        .map_err(|err| format!("启动 Codex app 增强模式失败: {err}"))?;
    if let Err(err) = wait_and_inject_plugin_unlock(debug_port, enhancement) {
        let _ = child.kill();
        return Err(err);
    }
    Ok(())
}

fn build_codex_enhancement_script() -> Result<CodexEnhancementScript, String> {
    let settings = read_settings_value()?;
    let mut scripts = Vec::new();
    let mut delete_bridge_enabled = false;
    if bool_field(&settings, "codex_plugins_enabled") {
        scripts.push(CODEX_PLUGIN_UNLOCK_SCRIPT.to_string());
    }
    if bool_field(&settings, "codex_delete_button_enabled") {
        let binding_name = serde_json::to_string(CODEX_DELETE_BINDING_NAME)
            .map_err(|err| format!("序列化删除 binding 名称失败: {err}"))?;
        scripts.push(
            CODEX_DELETE_BRIDGE_SCRIPT
                .replace("__CODEX_SWITCH_DELETE_BINDING_NAME__", &binding_name),
        );
        scripts.push(
            CODEX_DELETE_BUTTON_SCRIPT
                .replace("__CODEX_SWITCH_DELETE_BINDING_NAME__", &binding_name),
        );
        delete_bridge_enabled = true;
    }
    if scripts.is_empty() {
        return Err("未启用 Codex app 增强功能".to_string());
    }
    Ok(CodexEnhancementScript {
        source: scripts.join("\n"),
        delete_bridge_enabled,
    })
}

fn select_loopback_port(requested: u16) -> Result<u16, String> {
    if TcpListener::bind(("127.0.0.1", requested)).is_ok() {
        return Ok(requested);
    }
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).map_err(|err| format!("分配 CDP 端口失败: {err}"))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|err| format!("读取 CDP 端口失败: {err}"))
}

fn wait_and_inject_plugin_unlock(
    port: u16,
    enhancement: CodexEnhancementScript,
) -> Result<(), String> {
    let started = Instant::now();
    let timeout = StdDuration::from_millis(CDP_CONNECT_TIMEOUT_MS);
    let mut last_error = String::new();

    while started.elapsed() < timeout {
        match inject_plugin_unlock(port, &enhancement) {
            Ok(()) => return Ok(()),
            Err(err) => last_error = err,
        }
        thread::sleep(StdDuration::from_millis(250));
    }

    Err(if last_error.is_empty() {
        "等待 Codex app CDP 端口超时".to_string()
    } else {
        format!("注入 Codex app 增强脚本失败: {last_error}")
    })
}

fn inject_plugin_unlock(port: u16, enhancement: &CodexEnhancementScript) -> Result<(), String> {
    let websocket_url = page_websocket_url(port)?;
    let mut ws = connect_websocket(&websocket_url)?;
    if enhancement.delete_bridge_enabled {
        install_delete_binding(&mut ws)?;
    }
    send_cdp_command(
        &mut ws,
        1,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": enhancement.source }),
    )?;
    send_cdp_command(
        &mut ws,
        2,
        "Runtime.evaluate",
        json!({
            "expression": enhancement.source,
            "awaitPromise": false,
            "allowUnsafeEvalBlockedByCSP": true
        }),
    )?;
    if enhancement.delete_bridge_enabled {
        ws.set_read_timeout(None)
            .map_err(|err| format!("设置 Codex 删除 binding read timeout 失败: {err}"))?;
        thread::spawn(move || run_delete_binding_loop(ws));
    }
    Ok(())
}

fn install_delete_binding(ws: &mut TcpStream) -> Result<(), String> {
    send_cdp_command(ws, 10, "Runtime.enable", json!({}))?;
    let _ = send_cdp_command(
        ws,
        11,
        "Runtime.removeBinding",
        json!({ "name": CODEX_DELETE_BINDING_NAME }),
    );
    send_cdp_command(
        ws,
        12,
        "Runtime.addBinding",
        json!({ "name": CODEX_DELETE_BINDING_NAME }),
    )?;
    Ok(())
}

fn run_delete_binding_loop(mut ws: TcpStream) {
    loop {
        let message = match read_ws_text(&mut ws) {
            Ok(message) => message,
            Err(err) => {
                eprintln!("Codex 删除 binding 已断开: {err}");
                return;
            }
        };
        let value = match serde_json::from_str::<Value>(&message) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("method").and_then(Value::as_str) != Some("Runtime.bindingCalled") {
            continue;
        }
        let payload = value
            .get("params")
            .and_then(|params| params.get("payload"))
            .and_then(Value::as_str)
            .unwrap_or("{}");
        match serde_json::from_str::<DeleteBridgeEnvelope>(payload) {
            Ok(envelope) => {
                let result = handle_delete_bridge_request(envelope.payload);
                let _ = resolve_delete_binding(&mut ws, &envelope.id, &result);
            }
            Err(err) => {
                eprintln!("解析 Codex 删除 binding 请求失败: {err}");
            }
        }
    }
}

fn handle_delete_bridge_request(payload: DeleteBridgeRequest) -> Value {
    let title = payload.title.unwrap_or_default();
    match session_manager::delete_codex_session_for_bridge(&payload.session_id, &title) {
        Ok(value) => json!({
            "status": "local_deleted",
            "session_id": payload.session_id,
            "message": value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("删除成功"),
            "report": value.get("report").cloned().unwrap_or(Value::Null)
        }),
        Err(err) => json!({
            "status": "failed",
            "session_id": payload.session_id,
            "message": err
        }),
    }
}

fn resolve_delete_binding(
    ws: &mut TcpStream,
    request_id: &str,
    result: &Value,
) -> Result<(), String> {
    let request_id = serde_json::to_string(request_id)
        .map_err(|err| format!("序列化 Codex 删除 binding 请求 ID 失败: {err}"))?;
    let expression = format!(
        "window.__codexSwitchDeleteResolve({request_id}, {})",
        result
    );
    let id = CDP_BRIDGE_COMMAND_ID.fetch_add(1, Ordering::Relaxed);
    let payload = json!({
        "id": id,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "awaitPromise": false,
            "allowUnsafeEvalBlockedByCSP": true
        }
    });
    send_ws_text(ws, &payload.to_string())
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
    let codex_page = pages.iter().find(|target| {
        if target.get("type").and_then(Value::as_str) != Some("page") {
            return false;
        }
        let title = target.get("title").and_then(Value::as_str).unwrap_or("");
        let url = target.get("url").and_then(Value::as_str).unwrap_or("");
        (title.to_ascii_lowercase() + " " + &url.to_ascii_lowercase()).contains("codex")
            && target
                .get("webSocketDebuggerUrl")
                .and_then(Value::as_str)
                .is_some()
    });

    codex_page
        .or(first_page)
        .and_then(|target| target.get("webSocketDebuggerUrl").and_then(Value::as_str))
        .map(str::to_string)
        .ok_or_else(|| "未找到可注入的 Codex 页面".to_string())
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
