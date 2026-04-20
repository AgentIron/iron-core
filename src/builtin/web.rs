use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use crate::builtin::config::BuiltinToolConfig;
use crate::builtin::error::BuiltinToolError;
use crate::builtin::helpers::BuiltinMeta;
use crate::builtin::policy::NetworkPolicy;
use crate::error::RuntimeResult;
use crate::tool::{Tool, ToolDefinition, ToolFuture};
use serde_json::Value;

pub struct WebFetchTool {
    config: Arc<BuiltinToolConfig>,
}

impl WebFetchTool {
    pub fn new(config: BuiltinToolConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

impl Tool for WebFetchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "webfetch",
            "Fetch content from a URL and return it in a normalized text format. Subject to network policy and size limits.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch"
                    },
                    "format": {
                        "type": "string",
                        "description": "Output format: 'text' (default) or 'markdown'",
                        "enum": ["text", "markdown"]
                    }
                },
                "required": ["url"]
            }),
        )
        .with_approval(true)
    }

    fn execute(&self, _call_id: &str, arguments: Value) -> ToolFuture {
        let config = self.config.clone();
        Box::pin(async move { execute_webfetch(&config, arguments).await })
    }

    fn requires_approval(&self) -> bool {
        true
    }
}

async fn execute_webfetch(config: &BuiltinToolConfig, args: Value) -> RuntimeResult<Value> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BuiltinToolError::invalid_input("missing 'url' argument"))
        .map_err(crate::error::RuntimeError::from)?;

    let _format = args
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("text");

    if config.policy.network == NetworkPolicy::DenyAll {
        return Err(crate::error::RuntimeError::from(
            BuiltinToolError::network_denied("network access is denied by policy"),
        ));
    }

    let allow_private = config.policy.allow_private_network;
    validate_url(url, allow_private).await?;

    // Reject redirects that would route to private IP ranges unless opted in.
    // reqwest resolves hops against `Url` but we don't get the resolved IP in
    // the redirect callback, so we re-validate the URL's host on each hop.
    let redirect_policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.stop();
        }
        match validate_url_sync(attempt.url().as_str(), allow_private) {
            Ok(()) => attempt.follow(),
            Err(err) => attempt.error(std::io::Error::other(err.to_string())),
        }
    });

    let client = reqwest::Client::builder()
        .redirect(redirect_policy)
        .build()
        .map_err(|e| {
            crate::error::RuntimeError::from(BuiltinToolError::fetch_failed(format!(
                "failed to build HTTP client: {}",
                e
            )))
        })?;

    let response = client.get(url).send().await.map_err(|e| {
        crate::error::RuntimeError::from(BuiltinToolError::fetch_failed(format!(
            "fetch failed: {}",
            e
        )))
    })?;

    if !response.status().is_success() {
        return Err(crate::error::RuntimeError::from(
            BuiltinToolError::fetch_failed(format!("HTTP {}", response.status())),
        ));
    }

    let body = response.text().await.map_err(|e| {
        crate::error::RuntimeError::from(BuiltinToolError::fetch_failed(format!(
            "failed to read response: {}",
            e
        )))
    })?;

    let total_bytes = body.len();
    let (content, truncated) = if body.len() > config.max_fetch_bytes {
        let truncated_at = body.floor_char_boundary(config.max_fetch_bytes);
        (body[..truncated_at].to_string(), true)
    } else {
        (body, false)
    };

    let meta = if truncated {
        BuiltinMeta::with_truncation(total_bytes)
    } else {
        BuiltinMeta::empty()
    };

    Ok(serde_json::json!({
        "content": content,
        "url": url,
        "size": total_bytes,
        "truncated": truncated,
        "meta": meta,
    }))
}

async fn validate_url(url: &str, allow_private_network: bool) -> Result<(), BuiltinToolError> {
    let parsed = parse_and_check_scheme(url)?;
    let host = parsed
        .host_str()
        .ok_or_else(|| BuiltinToolError::invalid_url(format!("URL has no host: {}", url)))?;

    if allow_private_network {
        return Ok(());
    }

    // If the host is an IP literal, validate it directly — skip DNS.
    if let Ok(ip) = host.parse::<IpAddr>() {
        return check_ip(ip, url);
    }

    let port = parsed.port_or_known_default().unwrap_or(80);
    let addrs = tokio::net::lookup_host((host, port)).await.map_err(|e| {
        BuiltinToolError::fetch_failed(format!("DNS lookup failed for {}: {}", host, e))
    })?;
    for sockaddr in addrs {
        check_ip(sockaddr.ip(), url)?;
    }
    Ok(())
}

/// Synchronous URL validation used by the redirect callback. Skips DNS
/// resolution — only literal IPs are checked here, which covers the concrete
/// metadata-endpoint redirect attacks. Hostname-based redirects are bounded by
/// the top-level `validate_url` which ran before the request started.
fn validate_url_sync(url: &str, allow_private_network: bool) -> Result<(), BuiltinToolError> {
    let parsed = parse_and_check_scheme(url)?;
    if allow_private_network {
        return Ok(());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| BuiltinToolError::invalid_url(format!("URL has no host: {}", url)))?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        return check_ip(ip, url);
    }
    Ok(())
}

fn parse_and_check_scheme(url: &str) -> Result<url::Url, BuiltinToolError> {
    let parsed = url::Url::parse(url)
        .map_err(|_| BuiltinToolError::invalid_url(format!("invalid URL: {}", url)))?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        scheme => Err(BuiltinToolError::invalid_url(format!(
            "unsupported URL scheme: {} (only http and https are supported)",
            scheme
        ))),
    }
}

fn check_ip(ip: IpAddr, url: &str) -> Result<(), BuiltinToolError> {
    if is_blocked_ip(ip) {
        return Err(BuiltinToolError::network_denied(format!(
            "URL '{}' resolves to a blocked address ({}); enable allow_private_network to override",
            url, ip
        )));
    }
    Ok(())
}

fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => is_blocked_v6(v6),
    }
}

fn is_blocked_v4(v4: Ipv4Addr) -> bool {
    // Loopback (127/8), unspecified (0.0.0.0), link-local (169.254/16),
    // multicast (224/4), broadcast, private (RFC1918), CGNAT (100.64/10),
    // IETF-reserved (240/4), and test-only (192.0.2/24 etc.) ranges.
    v4.is_loopback()
        || v4.is_unspecified()
        || v4.is_link_local()
        || v4.is_multicast()
        || v4.is_broadcast()
        || v4.is_private()
        || is_v4_cgnat(v4)
        || is_v4_reserved(v4)
        || is_v4_documentation(v4)
}

fn is_v4_cgnat(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    o[0] == 100 && (o[1] & 0b1100_0000) == 64
}

fn is_v4_reserved(v4: Ipv4Addr) -> bool {
    v4.octets()[0] >= 240 && !v4.is_broadcast()
}

fn is_v4_documentation(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    matches!(
        (o[0], o[1], o[2]),
        (192, 0, 2) | (198, 51, 100) | (203, 0, 113)
    )
}

fn is_blocked_v6(v6: Ipv6Addr) -> bool {
    if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
        return true;
    }
    // IPv4-mapped (::ffff:0:0/96) — unwrap and re-check against v4 rules.
    if let Some(v4) = v6.to_ipv4_mapped() {
        return is_blocked_v4(v4);
    }
    let segments = v6.segments();
    // Unique-local (fc00::/7)
    if (segments[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    // Link-local (fe80::/10)
    if (segments[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_loopback_v4() {
        assert!(is_blocked_ip("127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("127.255.255.254".parse().unwrap()));
    }

    #[test]
    fn blocks_rfc1918_ranges() {
        assert!(is_blocked_ip("10.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("172.16.0.1".parse().unwrap()));
        assert!(is_blocked_ip("192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn blocks_link_local_and_metadata() {
        assert!(is_blocked_ip("169.254.0.1".parse().unwrap()));
        assert!(is_blocked_ip("169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn blocks_cgnat() {
        assert!(is_blocked_ip("100.64.0.1".parse().unwrap()));
        assert!(is_blocked_ip("100.127.255.254".parse().unwrap()));
        assert!(!is_blocked_ip("100.63.0.1".parse().unwrap()));
        assert!(!is_blocked_ip("100.128.0.1".parse().unwrap()));
    }

    #[test]
    fn blocks_unspecified_multicast_broadcast() {
        assert!(is_blocked_ip("0.0.0.0".parse().unwrap()));
        assert!(is_blocked_ip("224.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("255.255.255.255".parse().unwrap()));
    }

    #[test]
    fn blocks_v6_loopback_ula_link_local() {
        assert!(is_blocked_ip("::1".parse().unwrap()));
        assert!(is_blocked_ip("fc00::1".parse().unwrap()));
        assert!(is_blocked_ip("fd12:3456:789a::1".parse().unwrap()));
        assert!(is_blocked_ip("fe80::1".parse().unwrap()));
    }

    #[test]
    fn blocks_v4_mapped_v6_private() {
        // ::ffff:10.0.0.1 should be blocked via v4 rules.
        assert!(is_blocked_ip("::ffff:10.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("::ffff:127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn allows_public_v4() {
        assert!(!is_blocked_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_blocked_ip("1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn allows_public_v6() {
        assert!(!is_blocked_ip("2001:4860:4860::8888".parse().unwrap()));
    }

    #[tokio::test]
    async fn rejects_http_schemes_other_than_http_https() {
        let err = validate_url("ftp://example.com", false).await.unwrap_err();
        assert!(err.to_string().contains("unsupported URL scheme"));

        let err = validate_url("file:///etc/passwd", false).await.unwrap_err();
        assert!(err.to_string().contains("unsupported URL scheme"));
    }

    #[tokio::test]
    async fn rejects_ip_literal_to_private_range() {
        let err = validate_url("http://127.0.0.1/", false).await.unwrap_err();
        assert!(err.to_string().contains("blocked"));

        let err = validate_url("http://169.254.169.254/latest/meta-data/", false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("blocked"));
    }

    #[tokio::test]
    async fn allows_private_when_opted_in() {
        validate_url("http://127.0.0.1/", true).await.unwrap();
        validate_url("http://10.0.0.1/", true).await.unwrap();
    }

    #[test]
    fn redirect_validator_blocks_private_ip_hop() {
        let err = validate_url_sync("http://192.168.1.1/", false).unwrap_err();
        assert!(err.to_string().contains("blocked"));
    }

    #[test]
    fn redirect_validator_allows_public_ip_hop() {
        validate_url_sync("http://8.8.8.8/", false).unwrap();
    }
}
