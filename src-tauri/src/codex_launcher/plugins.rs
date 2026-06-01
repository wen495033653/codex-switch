use super::*;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use std::{
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
}

struct CdpScript {
    name: &'static str,
    source: &'static str,
}

const CODEX_PLUGIN_UNLOCK_SCRIPT: &str = r###"
(() => {
  const version = "3";
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

pub(crate) fn codex_processes_have_cdp_launch(processes: &[CodexProcess]) -> bool {
    processes
        .iter()
        .any(|process| command_line_has_cdp_launch(&process.command_line))
}

fn command_line_has_cdp_launch(command_line: &str) -> bool {
    let normalized = command_line.to_ascii_lowercase();
    normalized.contains("--remote-debugging-port")
        && normalized.contains("--remote-allow-origins=http://127.0.0.1:")
}

pub(crate) fn launch_codex_with_cdp_hooks(
    executable_path: &Path,
    hooks: CodexCdpLaunchHooks,
) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("Codex app hook 目前仅支持 Windows 重启入口".to_string());
    }
    if !executable_path.exists() {
        return Err(format!(
            "Codex app 路径不存在: {}",
            executable_path.display()
        ));
    }

    let scripts = cdp_scripts_for_hooks(hooks);
    if scripts.is_empty() {
        let mut command = Command::new(executable_path);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(parent) = executable_path.parent() {
            command.current_dir(parent);
        }
        sanitize_desktop_app_launch_env(&mut command);
        hide_command_window(&mut command);
        command
            .spawn()
            .map_err(|err| format!("启动 Codex app 失败: {err}"))?;
        return Ok(());
    }

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
    sanitize_desktop_app_launch_env(&mut command);
    hide_command_window(&mut command);

    let mut child = command
        .spawn()
        .map_err(|err| format!("启动 Codex app hook 模式失败: {err}"))?;
    if let Err(err) = wait_and_inject_cdp_scripts(debug_port, &scripts) {
        let _ = child.kill();
        return Err(err);
    }
    Ok(())
}

fn cdp_scripts_for_hooks(hooks: CodexCdpLaunchHooks) -> Vec<CdpScript> {
    let mut scripts = Vec::new();
    if hooks.plugin_unlock {
        scripts.push(CdpScript {
            name: "plugin_unlock",
            source: CODEX_PLUGIN_UNLOCK_SCRIPT,
        });
    }
    scripts
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

fn wait_and_inject_cdp_scripts(port: u16, scripts: &[CdpScript]) -> Result<(), String> {
    let started = Instant::now();
    let timeout = StdDuration::from_millis(CDP_CONNECT_TIMEOUT_MS);
    let mut last_error = String::new();

    while started.elapsed() < timeout {
        match inject_cdp_scripts(port, scripts) {
            Ok(()) => return Ok(()),
            Err(err) => last_error = err,
        }
        thread::sleep(StdDuration::from_millis(250));
    }

    Err(if last_error.is_empty() {
        "等待 Codex app CDP 端口超时".to_string()
    } else {
        format!("注入 Codex app hook 脚本失败: {last_error}")
    })
}

fn inject_cdp_scripts(port: u16, scripts: &[CdpScript]) -> Result<(), String> {
    let websocket_url = page_websocket_url(port)?;
    let mut ws = connect_websocket(&websocket_url)?;
    let mut command_id = 1;
    for script in scripts {
        send_cdp_command(
            &mut ws,
            command_id,
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": script.source }),
        )
        .map_err(|err| format!("注入 {} new-document 脚本失败: {err}", script.name))?;
        command_id += 1;
    }
    for script in scripts {
        send_cdp_command(
            &mut ws,
            command_id,
            "Runtime.evaluate",
            json!({
                "expression": script.source,
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
    fn select_loopback_port_zero_allocates_actual_port() {
        let port = select_loopback_port(0).expect("ephemeral port should be allocated");

        assert_ne!(port, 0);
    }
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
