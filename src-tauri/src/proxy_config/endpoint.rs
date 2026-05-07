use std::{
    net::{TcpStream, ToSocketAddrs},
    time::Duration as StdDuration,
};

pub(crate) struct ProxyEndpoint {
    pub(crate) host: String,
    pub(crate) port: u16,
}

fn parse_proxy_endpoint(proxy_url: &str) -> Result<ProxyEndpoint, String> {
    let (scheme, rest) = proxy_url
        .split_once("://")
        .ok_or_else(|| "代理地址格式无效，例如 127.0.0.1:10808".to_string())?;
    let default_port = if scheme.eq_ignore_ascii_case("https") {
        443
    } else {
        80
    };
    let authority = rest
        .split('/')
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("")
        .trim();
    if authority.is_empty() {
        return Err("代理地址无效".to_string());
    }

    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        let (host, suffix) = stripped
            .split_once(']')
            .ok_or_else(|| "代理地址无效".to_string())?;
        let port = suffix
            .strip_prefix(':')
            .filter(|value| !value.is_empty())
            .map(|value| value.parse::<u16>().map_err(|_| "代理端口无效".to_string()))
            .transpose()?
            .unwrap_or(default_port);
        (host.to_string(), port)
    } else if let Some((host, port_text)) = authority.rsplit_once(':') {
        let port = port_text
            .parse::<u16>()
            .map_err(|_| "代理端口无效".to_string())?;
        (host.to_string(), port)
    } else {
        (authority.to_string(), default_port)
    };

    if host.trim().is_empty() || port == 0 {
        return Err("代理地址无效".to_string());
    }
    Ok(ProxyEndpoint { host, port })
}

fn test_tcp_port_listening(host: &str, port: u16, timeout_ms: u64) -> bool {
    let Ok(mut addresses) = (host, port).to_socket_addrs() else {
        return false;
    };
    let Some(address) = addresses.next() else {
        return false;
    };
    TcpStream::connect_timeout(&address, StdDuration::from_millis(timeout_ms)).is_ok()
}

pub(crate) fn assert_proxy_ready(proxy_url: &str) -> Result<ProxyEndpoint, String> {
    let endpoint = parse_proxy_endpoint(proxy_url)?;
    if !test_tcp_port_listening(&endpoint.host, endpoint.port, 800) {
        return Err(format!(
            "{}:{} 未监听。请先启动代理后再打开 Codex。",
            endpoint.host, endpoint.port
        ));
    }
    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_endpoint_accepts_auth_and_ipv6() {
        let auth_endpoint = parse_proxy_endpoint("http://user:pass@127.0.0.1:10808").unwrap();
        assert_eq!(auth_endpoint.host, "127.0.0.1");
        assert_eq!(auth_endpoint.port, 10808);

        let ipv6_endpoint = parse_proxy_endpoint("http://[::1]:10808").unwrap();
        assert_eq!(ipv6_endpoint.host, "::1");
        assert_eq!(ipv6_endpoint.port, 10808);
    }
}
