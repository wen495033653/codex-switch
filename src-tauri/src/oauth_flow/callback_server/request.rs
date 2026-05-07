use std::{io::Read, net::TcpStream, time::Duration as StdDuration};

pub(super) fn parse_http_url(stream: &mut TcpStream) -> Result<url::Url, String> {
    stream
        .set_read_timeout(Some(StdDuration::from_secs(5)))
        .map_err(|err| format!("设置 OAuth 请求读取超时失败: {err}"))?;

    let mut buffer = [0_u8; 8192];
    let size = stream
        .read(&mut buffer)
        .map_err(|err| format!("读取 OAuth 回调请求失败: {err}"))?;
    let request = String::from_utf8_lossy(&buffer[..size]);
    let first_line = request
        .lines()
        .next()
        .ok_or_else(|| "OAuth 回调请求为空".to_string())?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    if method != "GET" || target.is_empty() {
        return Err("OAuth 回调请求无效".to_string());
    }

    url::Url::parse(&format!("http://localhost{target}"))
        .map_err(|err| format!("OAuth 回调 URL 无效: {err}"))
}
