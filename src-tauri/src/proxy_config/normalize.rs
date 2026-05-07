const VALID_PROXY_SCHEMES: [&str; 5] = [
    "http://",
    "https://",
    "socks4://",
    "socks5://",
    "socks5h://",
];

pub(crate) fn normalize_proxy_url(value: &str) -> Result<String, String> {
    let raw = value.trim();
    if raw.is_empty() {
        return Ok(String::new());
    }

    let proxy_url = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("http://{raw}")
    };
    let lower = proxy_url.to_ascii_lowercase();
    let valid_scheme = VALID_PROXY_SCHEMES
        .iter()
        .any(|scheme| lower.starts_with(scheme));
    if !valid_scheme
        || proxy_url
            .split("://")
            .nth(1)
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        return Err("代理地址格式无效，例如 127.0.0.1:10808".to_string());
    }
    Ok(proxy_url)
}

pub(crate) fn normalize_proxy_display_url(value: &str) -> String {
    let raw = value.trim();
    if raw.to_ascii_lowercase().starts_with("http://") {
        return raw["http://".len()..].trim().to_string();
    }
    raw.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_url_adds_http_scheme_for_host_port() {
        assert_eq!(
            normalize_proxy_url("127.0.0.1:10808").unwrap(),
            "http://127.0.0.1:10808"
        );
    }

    #[test]
    fn proxy_url_preserves_explicit_http_scheme() {
        assert_eq!(
            normalize_proxy_url("http://127.0.0.1:10808").unwrap(),
            "http://127.0.0.1:10808"
        );
    }

    #[test]
    fn proxy_display_hides_default_http_scheme() {
        assert_eq!(
            normalize_proxy_display_url("http://127.0.0.1:10808"),
            "127.0.0.1:10808"
        );
        assert_eq!(
            normalize_proxy_display_url("socks5://127.0.0.1:10808"),
            "socks5://127.0.0.1:10808"
        );
    }
}
