use super::claims::{
    account_id_from_claims, decode_jwt_payload, number_i64, resolve_auth_expires_at,
};
use crate::{
    accounts::{parse_endpoint_error, OAUTH_CLIENT_ID, OAUTH_TOKEN_ENDPOINT},
    json_util::string_field,
    time_util::now_string,
};
use serde_json::{json, Value};
use std::time::Duration as StdDuration;

fn token_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(StdDuration::from_secs(30))
        .build()
        .map_err(|err| format!("创建 HTTP client 失败: {err}"))
}

fn token_endpoint_error(prefix: &str, status: u16, text: &str, include_message: bool) -> String {
    let (message, code, raw_message) = parse_endpoint_error(status, text);
    let mut lines = vec![prefix.to_string(), format!("HTTP {status}")];
    if !code.is_empty() {
        lines.push(format!("error.code: {code}"));
    }
    if !raw_message.is_empty() {
        lines.push(format!("error.message: {raw_message}"));
    } else if include_message {
        lines.push(message);
    }
    lines.join("\n")
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
        .map_err(|err| format!("OAuth Token 交换失败\n{err}"))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().unwrap_or_default();
        return Err(token_endpoint_error(
            "OAuth Token 交换失败",
            status.as_u16(),
            &text,
            false,
        ));
    }

    let data: Value = response
        .json()
        .map_err(|err| format!("解析 OAuth Token 响应失败: {err}"))?;
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
        .map_err(|err| format!("Refresh Token 刷新失败\n{}", err))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().unwrap_or_default();
        return Err(token_endpoint_error(
            "Refresh Token 刷新失败",
            status.as_u16(),
            &text,
            true,
        ));
    }

    let data: Value = response
        .json()
        .map_err(|err| format!("解析 Refresh Token 响应失败: {err}"))?;
    token_response_to_exchange(data, Some(token))
}
