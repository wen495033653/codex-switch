use super::usage::{http_error_message, parse_endpoint_error};
use crate::{
    accounts::{OAUTH_AUTHORIZE_ENDPOINT, OAUTH_CLIENT_ID, OAUTH_SCOPE, OAUTH_TOKEN_ENDPOINT},
    json_util::string_field,
    time_util::now_string,
};
use base64::{engine::general_purpose, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration as StdDuration;
use time::{format_description::well_known::Rfc3339, Duration as TimeDuration, OffsetDateTime};

fn number_i64(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|item| i64::try_from(item).ok()))
            .or_else(|| number.as_f64().map(|item| item as i64)),
        Some(Value::String(text)) => text.parse::<i64>().ok(),
        _ => None,
    }
}

fn format_unix_timestamp(timestamp: i64) -> String {
    OffsetDateTime::from_unix_timestamp(timestamp)
        .ok()
        .and_then(|time| time.format(&Rfc3339).ok())
        .unwrap_or_default()
}

pub(crate) fn decode_jwt_payload(token: &str) -> Result<Value, String> {
    let payload = token
        .split('.')
        .nth(1)
        .ok_or_else(|| "id_token 格式无效".to_string())?;
    let bytes = general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| general_purpose::URL_SAFE.decode(payload))
        .map_err(|err| format!("id_token payload 解码失败: {err}"))?;
    serde_json::from_slice(&bytes).map_err(|err| format!("id_token payload 解析失败: {err}"))
}

fn account_id_from_claims(claims: &Value) -> Result<String, String> {
    let account_id = claims
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if account_id.is_empty() {
        return Err("JWT 缺少 chatgpt_account_id".to_string());
    }
    Ok(account_id)
}

fn resolve_auth_expires_at(claims: &Value, expires_in_seconds: Option<i64>) -> String {
    if let Some(expires_in) = expires_in_seconds.filter(|value| *value > 0) {
        return (OffsetDateTime::now_utc() + TimeDuration::seconds(expires_in))
            .format(&Rfc3339)
            .unwrap_or_default();
    }

    number_i64(claims.get("exp"))
        .filter(|value| *value > 0)
        .map(format_unix_timestamp)
        .unwrap_or_default()
}

fn token_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(StdDuration::from_secs(30))
        .build()
        .map_err(|err| format!("创建 HTTP client 失败: {err}"))
}

fn token_endpoint_error(prefix: &str, status: u16, text: &str, include_message: bool) -> String {
    let error = parse_endpoint_error(status, text);
    let mut lines = vec![prefix.to_string(), format!("HTTP {status}")];
    if !error.code.is_empty() {
        lines.push(format!("error.code: {}", error.code));
    }
    if !error.raw_message.is_empty() {
        lines.push(format!("error.message: {}", error.raw_message));
    } else if include_message {
        lines.push(error.message);
    }
    lines.join("\n")
}

/// Describes a rejected token request. A body that cannot be read still yields the status
/// lines (which `auth_error_is_login_expired` matches on) plus the read error.
fn token_endpoint_rejection(
    response: reqwest::blocking::Response,
    prefix: &str,
    include_message: bool,
) -> String {
    let status = response.status().as_u16();
    match response.text() {
        Ok(text) => token_endpoint_error(prefix, status, &text, include_message),
        Err(err) => format!(
            "{}\n读取错误响应正文失败: {}",
            token_endpoint_error(prefix, status, "", include_message),
            http_error_message(err)
        ),
    }
}

fn token_response_to_exchange(
    data: Value,
    fallback_refresh_token: Option<&str>,
) -> Result<Value, String> {
    let id_token = string_field(&data, "id_token");
    let access_token = string_field(&data, "access_token");
    let next_refresh_token = {
        let value = string_field(&data, "refresh_token");
        if value.is_empty() {
            fallback_refresh_token.unwrap_or("").to_string()
        } else {
            value
        }
    };
    if id_token.is_empty() {
        return Err("刷新结果缺少 id_token".to_string());
    }
    if access_token.is_empty() {
        return Err("刷新结果缺少 access_token".to_string());
    }
    if next_refresh_token.is_empty() {
        return Err("刷新结果缺少 refresh_token".to_string());
    }

    let claims = decode_jwt_payload(&id_token)?;
    let account_id = account_id_from_claims(&claims)?;
    let expires_at = resolve_auth_expires_at(&claims, number_i64(data.get("expires_in")));
    Ok(json!({
        "id_token": id_token,
        "access_token": access_token,
        "refresh_token": next_refresh_token,
        "account_id": account_id,
        "claims": claims,
        "expires_at": expires_at,
        "last_refresh_at": now_string()
    }))
}

pub(crate) fn exchange_oauth_code(code: &str, port: u16, verifier: &str) -> Result<Value, String> {
    let redirect_uri = format!("http://localhost:{port}/auth/callback");
    let client = token_client()?;
    let response = client
        .post(OAUTH_TOKEN_ENDPOINT)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", OAUTH_CLIENT_ID),
            ("code_verifier", verifier),
        ])
        .send()
        .map_err(|err| format!("OAuth Token 交换失败\n{}", http_error_message(err)))?;

    if !response.status().is_success() {
        return Err(token_endpoint_rejection(
            response,
            "OAuth Token 交换失败",
            false,
        ));
    }

    let data: Value = response
        .json()
        .map_err(|err| format!("解析 OAuth Token 响应失败: {}", http_error_message(err)))?;
    token_response_to_exchange(data, None)
}

pub(crate) fn exchange_refresh_token(refresh_token: &str) -> Result<Value, String> {
    let token = refresh_token.trim();
    if token.is_empty() {
        return Err("缺少 refreshToken".to_string());
    }

    let client = token_client()?;
    let response = client
        .post(OAUTH_TOKEN_ENDPOINT)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", token),
            ("client_id", OAUTH_CLIENT_ID),
        ])
        .send()
        .map_err(|err| format!("Refresh Token 刷新失败\n{}", http_error_message(err)))?;

    if !response.status().is_success() {
        return Err(token_endpoint_rejection(
            response,
            "Refresh Token 刷新失败",
            true,
        ));
    }

    let data: Value = response
        .json()
        .map_err(|err| format!("解析 Refresh Token 响应失败: {}", http_error_message(err)))?;
    token_response_to_exchange(data, Some(token))
}

pub(crate) fn random_urlsafe(bytes_len: usize) -> String {
    let mut bytes = vec![0_u8; bytes_len];
    rand::fill(&mut bytes);
    general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn generate_pkce() -> (String, String) {
    let verifier = random_urlsafe(32);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = general_purpose::URL_SAFE_NO_PAD.encode(digest);
    (verifier, challenge)
}

pub(crate) fn build_oauth_auth_url(
    port: u16,
    code_challenge: &str,
    state: &str,
) -> Result<String, String> {
    let redirect_uri = format!("http://localhost:{port}/auth/callback");
    let mut url = url::Url::parse(OAUTH_AUTHORIZE_ENDPOINT)
        .map_err(|err| format!("OAuth authorize endpoint 无效: {err}"))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", OAUTH_CLIENT_ID)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", OAUTH_SCOPE)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", state)
        .append_pair("originator", "codex_cli_rs");
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::auth_error_is_login_expired;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    /// Answers one request on 127.0.0.1 with `response` as raw bytes, then closes the socket.
    /// The client bypasses the system proxy, so nothing leaves the machine.
    fn local_response(response: &'static str) -> reqwest::blocking::Response {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/oauth/token", listener.local_addr().unwrap());
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            stream.write_all(response.as_bytes()).unwrap();
        });
        let _ = rustls::crypto::ring::default_provider().install_default();
        reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(StdDuration::from_secs(5))
            .build()
            .unwrap()
            .post(url)
            .send()
            .unwrap()
    }

    #[test]
    fn unreadable_rejection_body_keeps_the_status_lines() {
        // Content-Length promises 100 bytes, the socket closes after 8.
        let response = local_response(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{\"error\"",
        );

        let message = token_endpoint_rejection(response, "Refresh Token 刷新失败", true);

        assert!(
            message.starts_with(
                "Refresh Token 刷新失败\nHTTP 401\nAuthorization expired, please sign in again\n读取错误响应正文失败: "
            ),
            "{message}"
        );
        // Same classification as before the body read became explicit.
        assert!(auth_error_is_login_expired(&message));
    }

    #[test]
    fn readable_rejection_body_is_parsed() {
        let response = local_response(
            "HTTP/1.1 400 Bad Request\r\nContent-Length: 69\r\nConnection: close\r\n\r\n{\"error\":\"invalid_grant\",\"error_description\":\"refresh token reused!\"}",
        );

        let message = token_endpoint_rejection(response, "Refresh Token 刷新失败", true);

        assert_eq!(
            message,
            "Refresh Token 刷新失败\nHTTP 400\nerror.code: invalid_grant\nerror.message: refresh token reused!"
        );
    }
}
