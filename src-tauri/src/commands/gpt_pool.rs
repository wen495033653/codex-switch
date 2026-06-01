use super::*;
use rand::{distr::Alphanumeric, RngExt};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};

const GPT_POOL_TOKEN_NAME: &str = "codex";
const GPT_POOL_AUTOCONFIG_TIMEOUT: Duration = Duration::from_secs(300);

#[tauri::command]
pub(crate) async fn configure_gpt_pool_api() -> Result<Value, String> {
    let token = random_callback_token();
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|err| format!("启动 GPT Pool 本地回调失败: {err}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("配置 GPT Pool 本地回调失败: {err}"))?;
    let callback_url = format!(
        "http://127.0.0.1:{}/gpt-pool-autoconfig/{token}",
        listener
            .local_addr()
            .map_err(|err| format!("读取 GPT Pool 本地回调地址失败: {err}"))?
            .port()
    );

    open_gpt_pool_autoconfig_browser(&callback_url, &token)?;
    let wait_token = token.clone();
    let payload = tauri::async_runtime::spawn_blocking(move || {
        wait_for_gpt_pool_callback(listener, &wait_token)
    })
    .await
    .map_err(|err| format!("等待 GPT Pool 自动配置任务失败: {err}"))??;

    let api_key = payload
        .get("api_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "GPT Pool 未返回 API Key".to_string())?;
    let base_url = payload
        .get("base_url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "GPT Pool 未返回 API Base URL".to_string())?;
    let created = payload
        .get("created")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let patch = json!({
        "api_mode": {
            "name": "GPT Pool",
            "base_url": base_url,
            "api_key": api_key
        }
    });
    let settings = apply_codex_proxy_env_state_to_settings(update_settings_value(&patch)?)?;
    apply_complete_api_mode_profile_if_active(&settings)?;

    Ok(json!({
        "ok": true,
        "message": if created {
            "已创建并配置 GPT Pool API Key"
        } else {
            "已配置 GPT Pool API Key"
        },
        "created": created,
        "settings": settings
    }))
}

fn open_gpt_pool_autoconfig_browser(callback_url: &str, state: &str) -> Result<(), String> {
    let mut url = url::Url::parse("https://gpt-pool.com/console/token")
        .map_err(|err| format!("解析 GPT Pool 地址失败: {err}"))?;
    url.query_pairs_mut()
        .append_pair("codex_switch_callback", callback_url)
        .append_pair("codex_switch_state", state)
        .append_pair("codex_switch_token_name", GPT_POOL_TOKEN_NAME);
    open::that(url.as_str()).map_err(|err| format!("打开默认浏览器失败: {err}"))?;
    Ok(())
}

fn wait_for_gpt_pool_callback(listener: TcpListener, token: &str) -> Result<Value, String> {
    let deadline = Instant::now() + GPT_POOL_AUTOCONFIG_TIMEOUT;
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _addr)) => {
                if let Some(payload) = handle_callback_stream(&mut stream, token)? {
                    if payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                        return Ok(payload);
                    }
                    let message = payload
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("GPT Pool 自动配置失败");
                    return Err(message.to_string());
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(150));
            }
            Err(err) => return Err(format!("接收 GPT Pool 本地回调失败: {err}")),
        }
    }

    Err("等待 GPT Pool 登录或授权超时".to_string())
}

fn handle_callback_stream(stream: &mut TcpStream, token: &str) -> Result<Option<Value>, String> {
    let mut buffer = vec![0_u8; 64 * 1024];
    let read = stream
        .read(&mut buffer)
        .map_err(|err| format!("读取 GPT Pool 本地回调失败: {err}"))?;
    let request = String::from_utf8_lossy(&buffer[..read]);

    if request.starts_with("OPTIONS ") {
        write_http_response(stream, 204, "")?;
        return Ok(None);
    }

    let path = format!("/gpt-pool-autoconfig/{token}");
    if !request.starts_with(&format!("POST {path} ")) {
        write_http_response(stream, 404, "")?;
        return Ok(None);
    }
    let origin = header_value(&request, "origin").unwrap_or_default();
    if origin != "https://gpt-pool.com" {
        write_http_response(stream, 403, "{\"ok\":false}")?;
        return Err("GPT Pool 回调来源无效".to_string());
    }

    let body = request
        .split_once("\r\n\r\n")
        .map(|(_headers, body)| body)
        .unwrap_or("");
    let payload: Value =
        serde_json::from_str(body).map_err(|err| format!("解析 GPT Pool 本地回调失败: {err}"))?;
    if payload.get("state").and_then(Value::as_str) != Some(token) {
        write_http_response(stream, 400, "{\"ok\":false}")?;
        return Err("GPT Pool 回调 state 无效".to_string());
    }
    write_http_response(stream, 200, "{\"ok\":true}")?;
    Ok(Some(payload))
}

fn header_value(request: &str, name: &str) -> Option<String> {
    let needle = format!("{name}:");
    request.lines().find_map(|line| {
        if line.len() <= needle.len() || !line[..needle.len()].eq_ignore_ascii_case(&needle) {
            return None;
        }
        Some(line[needle.len()..].trim().to_string())
    })
}

fn write_http_response(stream: &mut TcpStream, status: u16, body: &str) -> Result<(), String> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Access-Control-Allow-Origin: https://gpt-pool.com\r\n\
         Access-Control-Allow-Methods: POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: content-type\r\n\
         Access-Control-Allow-Private-Network: true\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n\
         {body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|err| format!("写入 GPT Pool 本地回调响应失败: {err}"))
}

fn random_callback_token() -> String {
    let mut rng = rand::rng();
    (&mut rng)
        .sample_iter(Alphanumeric)
        .take(24)
        .map(char::from)
        .collect()
}
