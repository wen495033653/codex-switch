use super::*;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
};
use url::Url;

pub(crate) const CODEX_PLUGIN_DEBUG_PORT: u16 = 9229;
const CDP_CONNECT_TIMEOUT_MS: u64 = 12_000;
const CODEX_PLUGIN_UNLOCK_SCRIPT: &str = r###"
(() => {
  const version = "1";
  if (window.__codexSwitchPluginUnlockVersion === version) {
    window.__codexSwitchPluginUnlockScan?.();
    return;
  }
  window.__codexSwitchPluginUnlockVersion = version;

  const selectors = {
    disabledInstallButton: 'button:disabled.w-full.justify-center, [role="button"][aria-disabled="true"].cursor-not-allowed',
    pluginNavButton: 'nav[role="navigation"] button.h-token-nav-row.w-full',
    pluginSvgPath: 'svg path[d^="M7.94562 14.0277"]',
  };

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

  function labelUnlockedPluginEntry(button) {
    const labelTextNode = Array.from(button.querySelectorAll("span, div")).reverse()
      .flatMap((node) => Array.from(node.childNodes))
      .find((node) => node.nodeType === 3 && /^(插件|Plugins)( - 已解锁| - Unlocked)?$/i.test((node.nodeValue || "").trim()));
    if (!labelTextNode) return;
    const current = (labelTextNode.nodeValue || "").trim();
    labelTextNode.nodeValue = /^Plugins/i.test(current) ? "Plugins - Unlocked" : "插件 - 已解锁";
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
    labelUnlockedPluginEntry(button);
    const reactPropsKey = Object.keys(button).find((key) => key.startsWith("__reactProps"));
    if (reactPropsKey) {
      button[reactPropsKey].disabled = false;
    }
    if (button.dataset.codexSwitchPluginEnabled === "true") return;
    button.dataset.codexSwitchPluginEnabled = "true";
    button.addEventListener("click", () => {
      spoofChatGPTAuthMethod(button);
    }, true);
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

  function labelForcedInstallButton(button) {
    const textNode = Array.from(button.childNodes).find((node) => {
      const text = (node.nodeValue || "").trim();
      return node.nodeType === 3 && (/^安装\s/.test(text) || /^Install\s/.test(text) || text === "强制安装");
    });
    if (textNode) textNode.nodeValue = "强制安装";
  }

  function unblockPluginInstallButtons() {
    Array.from(document.querySelectorAll(selectors.disabledInstallButton)).forEach((button) => {
      const text = (button.textContent || "").trim();
      if (!/^安装\s/.test(text) && !/^Install\s/.test(text) && text !== "强制安装") return;
      unblockButtonElement(button);
      labelForcedInstallButton(button);
    });
  }

  let scanScheduled = false;
  function scan() {
    scanScheduled = false;
    try {
      enablePluginEntry();
      unblockPluginInstallButtons();
    } catch (error) {
      window.__codexSwitchPluginUnlockErrors = window.__codexSwitchPluginUnlockErrors || [];
      window.__codexSwitchPluginUnlockErrors.push(String(error?.stack || error));
    }
  }

  function scheduleScan() {
    if (scanScheduled) return;
    scanScheduled = true;
    requestAnimationFrame(scan);
  }

  window.__codexSwitchPluginUnlockScan = scan;
  new MutationObserver(scheduleScan).observe(document.documentElement, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ["disabled", "aria-disabled", "class", "style"],
  });
  setInterval(scheduleScan, 1500);
  scan();
})();
"###;

pub(crate) fn should_launch_codex_with_plugins(path: &Path) -> Result<bool, String> {
    if !cfg!(windows) || !is_codex_app_executable(path) {
        return Ok(false);
    }
    codex_plugins_enabled()
}

pub(crate) fn launch_codex_with_plugins(executable_path: &Path) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("Codex app 插件解锁目前仅支持 Windows 重启入口".to_string());
    }
    if !executable_path.exists() {
        return Err(format!(
            "Codex app 路径不存在: {}",
            executable_path.display()
        ));
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
    hide_command_window(&mut command);

    let mut child = command
        .spawn()
        .map_err(|err| format!("启动 Codex app 插件模式失败: {err}"))?;
    if let Err(err) = wait_and_inject_plugin_unlock(debug_port) {
        let _ = child.kill();
        return Err(err);
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn restart_codex_app_with_plugins() -> Result<Value, String> {
    if !cfg!(windows) {
        return Err("Codex app 插件解锁目前仅支持 Windows".to_string());
    }

    let snapshot = capture_open_ide_snapshot()?;
    let entries = snapshot
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| string_field(entry, "kind") == "codex")
        .collect::<Vec<_>>();

    if entries.is_empty() {
        return Ok(json!({
            "ok": true,
            "restarted": false,
            "message": "未检测到正在运行的 Codex app"
        }));
    }

    let pids = entries
        .iter()
        .filter_map(|entry| value_u64_field(entry, "pid"))
        .collect::<Vec<_>>();
    let mut executables = entries
        .iter()
        .map(|entry| raw_string_field(entry, "executablePath"))
        .filter(|path| !path.trim().is_empty())
        .collect::<Vec<_>>();
    executables.sort_by_key(|path| path.trim().to_ascii_lowercase());
    executables.dedup_by_key(|path| path.trim().to_ascii_lowercase());

    for pid in &pids {
        let _ = kill_process_tree(*pid);
    }
    let alive = wait_for_pids_exit(&pids, 12_000);
    if !alive.is_empty() {
        return Err("Codex app 进程未能退出，请手动关闭后重试".to_string());
    }

    let mut restarted = 0usize;
    for executable in executables {
        launch_codex_with_plugins(Path::new(&executable))?;
        restarted += 1;
        thread::sleep(StdDuration::from_millis(120));
    }

    Ok(json!({
        "ok": true,
        "restarted": restarted > 0,
        "restartedCount": restarted,
        "message": if restarted > 0 {
            "Codex app 插件模式已重启"
        } else {
            "未能重启 Codex app 插件模式"
        }
    }))
}

fn codex_plugins_enabled() -> Result<bool, String> {
    read_settings_value().map(|settings| bool_field(&settings, "codex_plugins_enabled"))
}

fn is_codex_app_executable(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let normalized = path
        .to_string_lossy()
        .to_ascii_lowercase()
        .replace('/', "\\");
    file_name == "codex.exe"
        && normalized.contains("\\openai.codex_")
        && normalized.contains("\\app\\codex.exe")
        && !normalized.contains("\\app\\resources\\codex.exe")
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

fn wait_and_inject_plugin_unlock(port: u16) -> Result<(), String> {
    let started = Instant::now();
    let timeout = StdDuration::from_millis(CDP_CONNECT_TIMEOUT_MS);
    let mut last_error = String::new();

    while started.elapsed() < timeout {
        match inject_plugin_unlock(port) {
            Ok(()) => return Ok(()),
            Err(err) => last_error = err,
        }
        thread::sleep(StdDuration::from_millis(250));
    }

    Err(if last_error.is_empty() {
        "等待 Codex app CDP 端口超时".to_string()
    } else {
        format!("注入 Codex app 插件解锁脚本失败: {last_error}")
    })
}

fn inject_plugin_unlock(port: u16) -> Result<(), String> {
    let websocket_url = page_websocket_url(port)?;
    let mut ws = connect_websocket(&websocket_url)?;
    send_cdp_command(
        &mut ws,
        1,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": CODEX_PLUGIN_UNLOCK_SCRIPT }),
    )?;
    send_cdp_command(
        &mut ws,
        2,
        "Runtime.evaluate",
        json!({
            "expression": CODEX_PLUGIN_UNLOCK_SCRIPT,
            "awaitPromise": false,
            "allowUnsafeEvalBlockedByCSP": true
        }),
    )?;
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
    fn detects_packaged_codex_app_executable() {
        assert!(is_codex_app_executable(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__abc\App\Codex.exe"
        )));
    }

    #[test]
    fn ignores_embedded_codex_resource_executable() {
        assert!(!is_codex_app_executable(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__abc\App\resources\codex.exe"
        )));
    }
}
