//! Does the configured server URL point at this machine?
//!
//! The app only updates a server it can restart, and only a server on this machine can be
//! restarted. A remote server, or a name that merely resolves to this machine through a proxy,
//! counts as remote. Only a loopback address counts as local.

use std::net::{IpAddr, Ipv6Addr};

/// Whether `url` names a server on this machine: `localhost`, or a loopback address
/// (`127.0.0.0/8`, `::1`). Anything else, or anything that does not parse, is not local.
pub fn is_local_url(url: &str) -> bool {
    let Some(host) = host_of(url.trim()) else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback(),
        Ok(IpAddr::V6(ip)) => ip == Ipv6Addr::LOCALHOST,
        Err(_) => false,
    }
}

/// The host part of an `http://` or `https://` URL, without brackets, user info, or port.
fn host_of(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let host = if let Some(inner) = authority.strip_prefix('[') {
        inner.split_once(']')?.0
    } else {
        authority.split(':').next()?
    };
    (!host.is_empty()).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_addresses_and_localhost_are_local() {
        for url in [
            "http://localhost:7573",
            "http://LOCALHOST",
            "https://localhost/",
            "http://127.0.0.1:7573",
            "http://127.1.2.3:80/x",
            "http://[::1]:7573",
            "  http://127.0.0.1:7573  ",
        ] {
            assert!(is_local_url(url), "{url}");
        }
    }

    #[test]
    fn other_hosts_are_not_local() {
        for url in [
            "http://192.168.1.10:7573",
            "http://kimun.example.com",
            "http://10.0.0.1",
            "http://0.0.0.0:7573",
            "http://localhost.example.com",
            "http://127.0.0.1.example.com",
            "http://[::2]:7573",
        ] {
            assert!(!is_local_url(url), "{url}");
        }
    }

    #[test]
    fn a_local_name_in_user_info_or_path_does_not_make_a_remote_host_local() {
        assert!(!is_local_url("http://localhost@evil.example.com/"));
        assert!(!is_local_url("http://evil.example.com/localhost"));
        assert!(!is_local_url("http://evil.example.com?host=127.0.0.1"));
    }

    #[test]
    fn text_that_is_not_an_http_url_is_not_local() {
        for url in [
            "",
            "localhost",
            "127.0.0.1:7573",
            "ftp://localhost",
            "http://",
            "http://:7573",
        ] {
            assert!(!is_local_url(url), "{url:?}");
        }
    }
}
