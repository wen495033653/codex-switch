mod request;
mod response;

use crate::accounts::exchange_oauth_code;
use std::{
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration as StdDuration, Instant},
};

use request::parse_http_url;
use response::send_http_response;

use super::{OAUTH_CALLBACK_PORT, OAUTH_CANCEL_MESSAGE};

const OAUTH_WAIT_TIMEOUT_MS: u64 = 180_000;

pub(super) fn wait_for_oauth_exchange(
    listener: TcpListener,
    state: &str,
    verifier: &str,
    canceled: Arc<AtomicBool>,
) -> Result<serde_json::Value, String> {
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("设置 OAuth server nonblocking 失败: {err}"))?;
    let deadline = Instant::now() + StdDuration::from_millis(OAUTH_WAIT_TIMEOUT_MS);

    while Instant::now() < deadline {
        if canceled.load(Ordering::SeqCst) {
            return Err(OAUTH_CANCEL_MESSAGE.to_string());
        }

        match listener.accept() {
            Ok((mut stream, _addr)) => {
                let url = match parse_http_url(&mut stream) {
                    Ok(value) => value,
                    Err(err) => {
                        send_http_response(&mut stream, 400, "登录失败", &err);
                        return Err(err);
                    }
                };

                if url.path() != "/auth/callback" {
                    send_http_response(&mut stream, 404, "Not Found", "Not Found");
                    continue;
                }

                let error = url
                    .query_pairs()
                    .find(|(key, _value)| key == "error")
                    .map(|(_key, value)| value.to_string())
                    .unwrap_or_default();
                if !error.is_empty() {
                    send_http_response(&mut stream, 400, "登录失败", &error);
                    return Err(error);
                }

                let callback_state = url
                    .query_pairs()
                    .find(|(key, _value)| key == "state")
                    .map(|(_key, value)| value.to_string())
                    .unwrap_or_default();
                if callback_state != state {
                    send_http_response(&mut stream, 400, "登录失败", "State 不匹配");
                    return Err("State mismatch".to_string());
                }

                let code = url
                    .query_pairs()
                    .find(|(key, _value)| key == "code")
                    .map(|(_key, value)| value.to_string())
                    .unwrap_or_default();
                if code.is_empty() {
                    send_http_response(&mut stream, 400, "登录失败", "缺少授权码");
                    return Err("Missing code".to_string());
                }

                if canceled.load(Ordering::SeqCst) {
                    send_http_response(&mut stream, 400, "登录已取消", OAUTH_CANCEL_MESSAGE);
                    return Err(OAUTH_CANCEL_MESSAGE.to_string());
                }

                match exchange_oauth_code(&code, OAUTH_CALLBACK_PORT, verifier) {
                    Ok(exchange) => {
                        if canceled.load(Ordering::SeqCst) {
                            send_http_response(
                                &mut stream,
                                400,
                                "登录已取消",
                                OAUTH_CANCEL_MESSAGE,
                            );
                            return Err(OAUTH_CANCEL_MESSAGE.to_string());
                        }

                        send_http_response(
                            &mut stream,
                            200,
                            "登录成功",
                            "可以关闭此窗口并回到 Codex Switch。",
                        );
                        return Ok(exchange);
                    }
                    Err(err) => {
                        send_http_response(&mut stream, 400, "登录失败", &err);
                        return Err(err);
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(StdDuration::from_millis(100));
            }
            Err(err) => return Err(format!("OAuth server 接收请求失败: {err}")),
        }
    }

    Err("OAuth 授权超时，请重试".to_string())
}
