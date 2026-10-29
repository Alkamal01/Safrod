//! Webhook URLs are supplied by API clients, so the server fetching them is a
//! classic SSRF vector. Destinations are checked when the endpoint is
//! registered and again, on the resolved address, at every delivery.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use reqwest::Url;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GuardError {
    #[error("invalid url")]
    Invalid,
    #[error("only https urls are allowed")]
    InsecureScheme,
    #[error("urls with credentials are not allowed")]
    Credentials,
    #[error("destination is not a public address")]
    NotPublic,
    #[error("hostname did not resolve")]
    Unresolvable,
}

/// Parses and checks a webhook URL. With `allow_private` (development only)
/// loopback and private destinations and plain http are permitted.
pub fn validate_url(raw: &str, allow_private: bool) -> Result<Url, GuardError> {
    let url = Url::parse(raw).map_err(|_| GuardError::Invalid)?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(GuardError::Credentials);
    }
    match url.scheme() {
        "https" => {}
        "http" if allow_private => {}
        "http" => return Err(GuardError::InsecureScheme),
        _ => return Err(GuardError::Invalid),
    }
    let host = url.host_str().ok_or(GuardError::Invalid)?;
    if !allow_private && host.eq_ignore_ascii_case("localhost") {
        return Err(GuardError::NotPublic);
    }
    if !allow_private {
        if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
            if !is_public_ip(ip) {
                return Err(GuardError::NotPublic);
            }
        }
    }
    Ok(url)
}

/// Resolves the host and returns an address that passed the public-address
/// check. The caller must connect to exactly this address; resolving again
/// at connect time would reopen a DNS rebinding window.
pub async fn resolve_checked(url: &Url, allow_private: bool) -> Result<SocketAddr, GuardError> {
    let host = url.host_str().ok_or(GuardError::Invalid)?;
    let port = url.port_or_known_default().ok_or(GuardError::Invalid)?;
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.trim_matches(['[', ']']), port))
        .await
        .map_err(|_| GuardError::Unresolvable)?
        .collect();
    if addrs.is_empty() {
        return Err(GuardError::Unresolvable);
    }
    // Every address must be acceptable, not just the first: a hostname that
    // also resolves to an internal address is rejected outright.
    if !allow_private && addrs.iter().any(|a| !is_public_ip(a.ip())) {
        return Err(GuardError::NotPublic);
    }
    Ok(addrs[0])
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        // 100.64.0.0/10 carrier-grade NAT
        || (a == 100 && (64..128).contains(&b))
        // 192.0.0.0/24 IETF protocol assignments
        || (a == 192 && b == 0 && c == 0)
        // 198.18.0.0/15 benchmarking
        || (a == 198 && (b == 18 || b == 19))
        // 240.0.0.0/4 reserved
        || a >= 240)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let first = ip.segments()[0];
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        // fc00::/7 unique local
        || (first & 0xfe00) == 0xfc00
        // fe80::/10 link-local
        || (first & 0xffc0) == 0xfe80
        // 2001:db8::/32 documentation
        || (first == 0x2001 && ip.segments()[1] == 0x0db8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_internal_destinations() {
        for url in [
            "https://127.0.0.1/hook",
            "https://localhost/hook",
            "https://10.0.0.5/hook",
            "https://192.168.1.1/hook",
            "https://172.16.0.1/hook",
            "https://169.254.169.254/latest/meta-data",
            "https://100.64.0.1/hook",
            "https://[::1]/hook",
            "https://[fd00::1]/hook",
            "https://[fe80::1]/hook",
            "https://[::ffff:127.0.0.1]/hook",
            "https://0.0.0.0/hook",
        ] {
            assert_eq!(
                validate_url(url, false),
                Err(GuardError::NotPublic),
                "{url}"
            );
        }
    }

    #[test]
    fn rejects_plain_http_credentials_and_other_schemes() {
        assert_eq!(
            validate_url("http://example.com/h", false),
            Err(GuardError::InsecureScheme)
        );
        assert_eq!(
            validate_url("https://user:pw@example.com/h", false),
            Err(GuardError::Credentials)
        );
        assert_eq!(
            validate_url("file:///etc/passwd", false),
            Err(GuardError::Invalid)
        );
        assert_eq!(
            validate_url("ftp://example.com/", true),
            Err(GuardError::Invalid)
        );
        assert_eq!(validate_url("not a url", false), Err(GuardError::Invalid));
    }

    #[test]
    fn accepts_public_https() {
        assert!(validate_url("https://example.com/hook", false).is_ok());
        assert!(validate_url("https://93.184.216.34/hook", false).is_ok());
    }

    #[test]
    fn development_mode_allows_private_and_http() {
        assert!(validate_url("http://127.0.0.1:3000/hook", true).is_ok());
        assert!(validate_url("http://merchant:3000/hook", true).is_ok());
    }

    #[tokio::test]
    async fn resolution_is_checked_for_loopback() {
        let url = Url::parse("https://localhost/hook").unwrap();
        assert_eq!(
            resolve_checked(&url, false).await,
            Err(GuardError::NotPublic)
        );
        assert!(resolve_checked(&url, true).await.is_ok());
    }
}
