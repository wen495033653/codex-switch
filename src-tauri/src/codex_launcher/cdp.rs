use super::{
    codex_app_watcher::CodexProcess,
    process_control::{launch_codex_process_with_options, sanitize_desktop_app_launch_env},
    shell::hide_command_window,
};
use serde_json::{json, Value};
use std::{collections::BTreeSet, net::TcpListener, path::Path};
use std::{
    process::{Command, Stdio},
    thread,
    time::{Duration as StdDuration, Instant},
};

mod api_quota;
mod transport;

use transport::CdpConnection;

pub(crate) const CODEX_CDP_DEBUG_PORT: u16 = 9229;
const CDP_CONNECT_TIMEOUT_MS: u64 = 12_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CodexCdpLaunchHooks {
    pub(crate) codex_mobile_no_replace: bool,
    pub(crate) api_quota: bool,
}

struct CdpScript {
    name: &'static str,
    source: String,
}

struct CdpScriptBundle {
    scripts: Vec<CdpScript>,
}

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

pub(crate) fn inject_codex_cdp_hooks(
    processes: &[CodexProcess],
    hooks: CodexCdpLaunchHooks,
) -> Result<usize, String> {
    let scripts = cdp_scripts_for_hooks(hooks);
    let ports = processes
        .iter()
        .filter_map(|process| cdp_debug_port_from_command_line(&process.command_line))
        .collect::<BTreeSet<_>>();
    let mut injected = 0usize;
    for port in ports {
        if !hooks.api_quota {
            api_quota::stop(port)?;
        }
        if !scripts.scripts.is_empty() {
            inject_cdp_scripts(port, &scripts)?;
        }
        if hooks.api_quota {
            api_quota::start(port, &page_websocket_url(port)?, false)?;
        }
        injected += 1;
    }
    Ok(injected)
}

pub(crate) fn stop_codex_api_quota_hooks() -> Result<usize, String> {
    api_quota::stop_all()
}

fn command_line_has_cdp_launch(command_line: &str) -> bool {
    cdp_debug_port_from_command_line(command_line).is_some()
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
    if !executable_path.exists() {
        return Err(format!("Codex 路径不存在: {}", executable_path.display()));
    }

    let debug_port = select_loopback_port(CODEX_CDP_DEBUG_PORT)?;
    let scripts = cdp_scripts_for_hooks(hooks);
    let mut launch_args = vec![
        format!("--remote-debugging-port={debug_port}"),
        "--remote-debugging-address=127.0.0.1".to_string(),
        format!("--remote-allow-origins=http://127.0.0.1:{debug_port}"),
    ];
    launch_args.extend(args.iter().cloned());
    // With nothing to inject, Codex only has to start with its debug port open, which the normal
    // launch confirms. Waiting for a page here would end a Codex that is slow to show one.
    if scripts.scripts.is_empty() && !hooks.api_quota {
        return launch_codex_process_with_options(
            &executable_path.to_string_lossy(),
            &launch_args,
            envs,
        );
    }
    let mut command = Command::new(executable_path);
    command
        .args(&launch_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
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
    let empty = CdpScriptBundle {
        scripts: Vec::new(),
    };
    let readiness =
        wait_and_inject_cdp_scripts(debug_port, if hooks.api_quota { &empty } else { &scripts })
            .and_then(|()| {
                if hooks.api_quota {
                    api_quota::start(debug_port, &page_websocket_url(debug_port)?, true)?;
                    if !scripts.scripts.is_empty() {
                        inject_cdp_scripts(debug_port, &scripts)?;
                    }
                }
                Ok(())
            });
    if let Err(err) = readiness {
        let quota_cleanup = api_quota::stop(debug_port);
        let cleanup = child.kill();
        return Err(format!(
            "{err}; quota cleanup={quota_cleanup:?}; child cleanup={cleanup:?}"
        ));
    }
    Ok(())
}

fn cdp_scripts_for_hooks(hooks: CodexCdpLaunchHooks) -> CdpScriptBundle {
    let mut scripts = Vec::new();
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
    let mut ws = CdpConnection::connect(&websocket_url)?;
    for script in &bundle.scripts {
        ws.command(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": &script.source }),
        )
        .map_err(|err| format!("注入 {} new-document 脚本失败: {err}", script.name))?;
    }
    for script in &bundle.scripts {
        let response = ws
            .command(
                "Runtime.evaluate",
                json!({
                    "expression": &script.source,
                    "awaitPromise": true,
                    "returnByValue": true,
                    "allowUnsafeEvalBlockedByCSP": true
                }),
            )
            .map_err(|err| format!("执行 {} 脚本失败: {err}", script.name))?;
        validate_cdp_script_result(script.name, &response)?;
    }
    Ok(())
}

fn validate_cdp_script_result(name: &str, response: &Value) -> Result<(), String> {
    let result = &response["result"];
    if let Some(exception) = result.get("exceptionDetails") {
        return Err(format!("执行 {name} 脚本异常: {exception}"));
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

    pages
        .iter()
        .find(|target| is_codex_desktop_page_target(target))
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
    if url.contains("avatar-overlay") {
        return false;
    }
    if url == "app://-/index.html" {
        return true;
    }
    let identity = format!(
        "{} {}",
        title.to_ascii_lowercase(),
        url.to_ascii_lowercase()
    );
    identity.contains("codex") || identity.contains("chatgpt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_has_cdp_launch_accepts_valid_debug_port() {
        assert!(command_line_has_cdp_launch(
            r#""C:\Codex\codex.exe" --remote-debugging-port=9229 --remote-allow-origins=http://127.0.0.1:9229"#
        ));
        assert!(command_line_has_cdp_launch(
            r#""C:\Codex\codex.exe" --remote-debugging-port=9229"#
        ));
        assert!(!command_line_has_cdp_launch(
            r#""C:\Codex\codex.exe" --remote-debugging-port=invalid"#
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
    fn cdp_target_excludes_avatar_overlay() {
        assert!(!is_codex_desktop_page_target(
            &json!({"type":"page","title":"Codex","url":"app://-/index.html?initialRoute=%2Favatar-overlay","webSocketDebuggerUrl":"ws://127.0.0.1:1/avatar"})
        ));
        assert!(is_codex_desktop_page_target(
            &json!({"type":"page","title":"Codex","url":"app://-/index.html","webSocketDebuggerUrl":"ws://127.0.0.1:1/main"})
        ));
    }

    #[test]
    fn bundle_contains_only_requested_mobile_hook() {
        assert!(cdp_scripts_for_hooks(CodexCdpLaunchHooks {
            codex_mobile_no_replace: false,
            api_quota: false,
        })
        .scripts
        .is_empty());
        let bundle = cdp_scripts_for_hooks(CodexCdpLaunchHooks {
            codex_mobile_no_replace: true,
            api_quota: false,
        });
        assert_eq!(bundle.scripts.len(), 1);
        assert_eq!(bundle.scripts[0].name, "codex_mobile_no_replace");
        assert!(!bundle.scripts[0].source.contains("plugin/list"));
    }
    #[test]
    fn cdp_script_exception_is_returned() {
        assert!(validate_cdp_script_result(
            "codex_mobile_no_replace",
            &json!({"result":{"exceptionDetails":{"text":"boom"}}})
        )
        .unwrap_err()
        .contains("boom"));
        assert!(validate_cdp_script_result(
            "codex_mobile_no_replace",
            &json!({"result":{"result":{"type":"undefined"}}})
        )
        .is_ok());
    }
}
