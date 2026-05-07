use url::Url;

pub(crate) const DEFAULT_API_NAME: &str = "api";
pub(crate) const OPENAI_PROVIDER_ID: &str = "openai";
pub(crate) const API_PROVIDER_ID: &str = "api";
pub(crate) const LEGACY_API_PROVIDER_IDS: [&str; 3] =
    ["newapi", "codex-switcher-api", "codex-switch-api"];

pub(crate) fn api_provider_name(name: &str, _base_url: &str) -> String {
    let trimmed_name = name.trim();
    if !trimmed_name.is_empty() {
        return trimmed_name.to_string();
    }

    DEFAULT_API_NAME.to_string()
}

pub(crate) fn normalize_api_base_url(base_url: &str) -> Result<String, String> {
    let raw = base_url.trim();
    if raw.is_empty() {
        return Err("API Base URL 不能为空".to_string());
    }

    let value = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let mut url = Url::parse(&value).map_err(|err| format!("API Base URL 格式无效: {err}"))?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("API Base URL 仅支持 http 或 https".to_string()),
    }
    if url.host_str().unwrap_or("").trim().is_empty() {
        return Err("API Base URL 缺少 host".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("API Base URL 不能包含 query 或 fragment".to_string());
    }

    let path = url.path().trim_end_matches('/').to_string();
    if path.is_empty() {
        url.set_path("/v1");
    } else {
        url.set_path(&path);
    }
    Ok(url.to_string().trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_api_base_url_trims_trailing_slash() {
        assert_eq!(
            normalize_api_base_url(" https://api.example.com/v1/ ").unwrap(),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn normalize_api_base_url_adds_default_scheme_and_v1_path() {
        for value in [
            "https://gpt-pool.com/v1",
            "https://gpt-pool.com/v1/",
            "https://gpt-pool.com/",
            "https://gpt-pool.com",
            "gpt-pool.com",
        ] {
            assert_eq!(
                normalize_api_base_url(value).unwrap(),
                "https://gpt-pool.com/v1"
            );
        }
    }

    #[test]
    fn normalize_api_base_url_rejects_empty_value() {
        assert_eq!(
            normalize_api_base_url("  ").unwrap_err(),
            "API Base URL 不能为空"
        );
    }

    #[test]
    fn api_provider_name_uses_api_when_name_is_default() {
        assert_eq!(
            api_provider_name(DEFAULT_API_NAME, "https://api.example.com/v1"),
            DEFAULT_API_NAME
        );
        assert_eq!(
            api_provider_name("", "https://api.example.com/v1"),
            DEFAULT_API_NAME
        );
    }

    #[test]
    fn api_provider_name_preserves_custom_name() {
        assert_eq!(
            api_provider_name("Custom Provider", "https://api.example.com/v1"),
            "Custom Provider"
        );
    }
}
