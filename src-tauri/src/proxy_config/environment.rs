use std::collections::HashMap;

const PROXY_ENV_NAMES: [&str; 10] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "WS_PROXY",
    "WSS_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "ws_proxy",
    "wss_proxy",
];

pub(crate) fn build_proxy_environment(
    proxy_url: &str,
    openai_base_url: &str,
) -> HashMap<String, String> {
    let mut envs = HashMap::new();
    for name in PROXY_ENV_NAMES {
        envs.insert(name.to_string(), proxy_url.to_string());
    }
    if !openai_base_url.trim().is_empty() {
        envs.insert(
            "OPENAI_BASE_URL".to_string(),
            openai_base_url.trim().to_string(),
        );
    }
    envs
}
