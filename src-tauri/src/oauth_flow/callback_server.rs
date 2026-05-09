mod request;
mod response;

use crate::accounts::exchange_oauth_code;
use std::{
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, TryRecvError},
        Arc,
    },
    thread,
    time::{Duration as StdDuration, Instant},
};

use request::parse_http_url;
use response::send_http_response;

use super::{OAUTH_CALLBACK_PORT, OAUTH_CANCEL_MESSAGE};

const OAUTH_WAIT_TIMEOUT_MS: u64 = 300_000;

pub(super) fn wait_for_oauth_exchange(
    listener: TcpListener,
    state: &str,
    verifier: &str,
    canceled: Arc<AtomicBool>,
    manual_callbacks: Receiver<String>,
) -> Result<serde_json::Value, String> {
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("设置 OAuth server nonblocking 失败: {err}"))?;
    let deadline = Instant::now() + StdDuration::from_millis(OAUTH_WAIT_TIMEOUT_MS);

    while Instant::now() < deadline {
        if canceled.load(Ordering::SeqCst) {
            return Err(OAUTH_CANCEL_MESSAGE.to_string());
        }

        match manual_callbacks.try_recv() {
            Ok(callback_url) => {
                let url = parse_manual_callback_url(&callback_url)?;
                return exchange_oauth_callback_url(&url, state, verifier);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {}
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

                if !is_oauth_callback_path(&url) {
                    send_http_response(&mut stream, 404, "Not Found", "Not Found");
                    continue;
                }

                if canceled.load(Ordering::SeqCst) {
                    send_http_response(&mut stream, 400, "登录已取消", OAUTH_CANCEL_MESSAGE);
                    return Err(OAUTH_CANCEL_MESSAGE.to_string());
                }

                match exchange_oauth_callback_url(&url, state, verifier) {
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

pub(super) fn parse_manual_callback_url(input: &str) -> Result<url::Url, String> {
    let raw = input.trim().trim_matches('"').trim_matches('\'').trim();
    if raw.is_empty() {
        return Err("请输入回调 URL".to_string());
    }

    let mut candidates = Vec::new();
    candidates.push(raw.to_string());

    if !raw.contains("://") {
        if raw.starts_with("localhost") || raw.starts_with("127.0.0.1") || raw.starts_with("[::1]")
        {
            candidates.push(format!("http://{raw}"));
        }
        if raw.starts_with('/') {
            candidates.push(format!("http://localhost:{OAUTH_CALLBACK_PORT}{raw}"));
        }
        if raw.starts_with('?') || raw.starts_with('#') {
            candidates.push(format!(
                "http://localhost:{OAUTH_CALLBACK_PORT}/auth/callback{raw}"
            ));
        }
        if raw.contains('=') && !raw.starts_with('/') && !raw.starts_with('?') {
            candidates.push(format!(
                "http://localhost:{OAUTH_CALLBACK_PORT}/auth/callback?{raw}"
            ));
        }
    }

    let mut path_invalid = false;
    for candidate in candidates {
        if let Ok(url) = url::Url::parse(&candidate) {
            if matches!(url.scheme(), "http" | "https") {
                if is_oauth_callback_path(&url) {
                    return Ok(url);
                }
                path_invalid = true;
            }
        }
    }

    if path_invalid {
        Err("OAuth 回调 URL 路径无效".to_string())
    } else {
        Err("回调 URL 格式无效".to_string())
    }
}

fn exchange_oauth_callback_url(
    url: &url::Url,
    state: &str,
    verifier: &str,
) -> Result<serde_json::Value, String> {
    if !is_oauth_callback_path(url) {
        return Err("OAuth 回调 URL 路径无效".to_string());
    }

    let error = callback_param(url, "error");
    if !error.is_empty() {
        return Err(error);
    }

    let callback_state = callback_param(url, "state");
    if callback_state != state {
        return Err("State mismatch".to_string());
    }

    let code = callback_param(url, "code");
    if code.is_empty() {
        return Err("Missing code".to_string());
    }

    exchange_oauth_code(&code, OAUTH_CALLBACK_PORT, verifier)
}

fn is_oauth_callback_path(url: &url::Url) -> bool {
    url.path() == "/auth/callback"
}

fn callback_param(url: &url::Url, key: &str) -> String {
    if let Some(value) = url
        .query_pairs()
        .find(|(candidate, _value)| candidate.as_ref() == key)
        .map(|(_candidate, value)| value.to_string())
    {
        return value;
    }

    let Some(fragment) = url.fragment() else {
        return String::new();
    };
    let query_like = fragment
        .split_once('?')
        .map(|(_path, query)| query)
        .unwrap_or(fragment)
        .trim_start_matches('?');

    url::form_urlencoded::parse(query_like.as_bytes())
        .find(|(candidate, _value)| candidate.as_ref() == key)
        .map(|(_candidate, value)| value.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{callback_param, parse_manual_callback_url};

    #[test]
    fn parse_manual_callback_url_accepts_plain_query() {
        let url = parse_manual_callback_url("code=abc&state=xyz").unwrap();

        assert_eq!(url.path(), "/auth/callback");
        assert_eq!(callback_param(&url, "code"), "abc");
        assert_eq!(callback_param(&url, "state"), "xyz");
    }

    #[test]
    fn parse_manual_callback_url_accepts_localhost_without_scheme() {
        let url =
            parse_manual_callback_url("localhost:1455/auth/callback?code=abc&state=xyz").unwrap();

        assert_eq!(url.path(), "/auth/callback");
        assert_eq!(callback_param(&url, "code"), "abc");
        assert_eq!(callback_param(&url, "state"), "xyz");
    }

    #[test]
    fn callback_param_reads_fragment() {
        let url =
            parse_manual_callback_url("http://localhost:1455/auth/callback#code=abc&state=xyz")
                .unwrap();

        assert_eq!(callback_param(&url, "code"), "abc");
        assert_eq!(callback_param(&url, "state"), "xyz");
    }
}
