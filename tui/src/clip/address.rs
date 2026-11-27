//! Which network addresses a clip may connect to, and the pure URL check.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

use super::{ClipError, FetchPolicy};

/// True when `ip` is a public address a clip may connect to. Loopback,
/// private, link-local, shared, multicast, unspecified, and broadcast
/// addresses are refused. IPv6 forms that wrap an IPv4 address are judged
/// by the IPv4 address inside.
pub fn is_public_address(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: &Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0 // "this" network, includes 0.0.0.0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b)) // shared address space
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments
        || (a == 198 && (b == 18 || b == 19)) // benchmarking
        || a >= 224) // multicast, reserved, and broadcast
}

fn is_public_v6(ip: &Ipv6Addr) -> bool {
    let s = ip.segments();
    // Forms that carry an IPv4 address are judged by the address inside.
    let v4_of = |hi: u16, lo: u16| {
        let [a, b] = hi.to_be_bytes();
        let [c, d] = lo.to_be_bytes();
        Ipv4Addr::new(a, b, c, d)
    };
    let low96_zero = s[..6] == [0; 6];
    if (s[..5] == [0; 5] && s[5] == 0xffff) // ::ffff:a.b.c.d
        || (s[..6] == [0x64, 0xff9b, 0, 0, 0, 0]) // NAT64, 64:ff9b::/96
        || (low96_zero && s[6..] != [0, 0] && s[6..] != [0, 1])
    // ::a.b.c.d
    {
        return is_public_v4(&v4_of(s[6], s[7]));
    }
    if s[0] == 0x2002 {
        // 6to4: the IPv4 address is in segments 1 and 2.
        return is_public_v4(&v4_of(s[1], s[2]));
    }
    !(ip.octets() == [0; 16] // unspecified
        || s[..7] == [0; 7] && s[7] == 1 // loopback
        || (s[0] & 0xfe00) == 0xfc00 // unique local
        || (s[0] & 0xffc0) == 0xfe80 // link-local
        || (s[0] & 0xffc0) == 0xfec0 // site-local, deprecated
        || (s[0] == 0x64 && s[1] == 0xff9b && s[2] == 1) // local-use NAT64
        || (s[0] & 0xff00) == 0xff00) // multicast
}

/// Checks one URL against the policy. The fetch calls it for the first URL
/// and for every redirect target.
///
/// Host names pass here, even `localhost`. The resolver checks the addresses
/// they resolve to. IP literals are checked here, because they never reach
/// the resolver.
pub(crate) fn check_url(url: &Url, policy: &FetchPolicy) -> Result<(), ClipError> {
    match url.scheme() {
        "https" => {}
        "http" if policy.allow_http => {}
        _ => {
            // Never echo credentials into an error message.
            let mut shown = url.clone();
            let _ = shown.set_username("");
            let _ = shown.set_password(None);
            return Err(ClipError::NotHttps(shown.to_string()));
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ClipError::BadUrl(
            "the address contains a user name or password".to_string(),
        ));
    }
    let ip = match url.host() {
        None => {
            return Err(ClipError::BadUrl(format!("the address has no host: {url}")));
        }
        Some(Host::Domain(_)) => return Ok(()),
        Some(Host::Ipv4(v4)) => IpAddr::V4(v4),
        Some(Host::Ipv6(v6)) => IpAddr::V6(v6),
    };
    if policy.allow_private || is_public_address(&ip) {
        Ok(())
    } else {
        Err(ClipError::PrivateAddress(ip.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn refuses_non_public_addresses() {
        for s in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd12::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:7f00:1",
            "64:ff9b::a00:1",
            "64:ff9b::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "::127.0.0.1",
            "::10.0.0.1",
            "2002:7f00:1::",
            "2002:0a00:1::",
            "64:ff9b:1::1",
            "fec0::1",
            "192.0.0.1",
            "192.0.0.170",
            "198.18.0.1",
            "198.19.255.255",
        ] {
            assert!(!is_public_address(&ip(s)), "{s} should be refused");
        }
    }

    #[test]
    fn accepts_public_addresses() {
        for s in [
            "93.184.216.34",
            "8.8.8.8",
            "172.32.0.1",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2002:0808:0808::",
        ] {
            assert!(is_public_address(&ip(s)), "{s} should be allowed");
        }
    }

    fn check(s: &str, policy: &FetchPolicy) -> Result<(), ClipError> {
        check_url(&Url::parse(s).unwrap(), policy)
    }

    #[test]
    fn scheme_rules() {
        let strict = FetchPolicy::strict();
        assert!(matches!(
            check("http://example.com", &strict),
            Err(ClipError::NotHttps(_))
        ));
        let mut http = FetchPolicy::strict();
        http.allow_http = true;
        assert!(check("http://example.com", &http).is_ok());
        assert!(check("https://example.com", &strict).is_ok());
        for s in ["ftp://example.com", "file:///etc/passwd"] {
            assert!(
                matches!(check(s, &strict), Err(ClipError::NotHttps(_))),
                "{s}"
            );
            assert!(
                matches!(check(s, &http), Err(ClipError::NotHttps(_))),
                "{s}"
            );
        }
        for s in ["data:text/plain,hi", "javascript:alert(1)"] {
            assert!(
                matches!(check(s, &http), Err(ClipError::NotHttps(_))),
                "{s}"
            );
        }
    }

    #[test]
    fn private_literals_are_refused() {
        let strict = FetchPolicy::strict();
        for s in [
            "https://127.0.0.1/",
            "https://[::1]/",
            "https://10.0.0.1/",
            "https://169.254.169.254/latest/meta-data/",
            "https://[::ffff:127.0.0.1]/",
            "https://0.0.0.0/",
            "https://2130706433/",
            "https://0x7f.1/",
            "https://127.1/",
            "https://[::ffff:7f00:1]/",
            "https://127.0.0.1\\@example.com/",
        ] {
            assert!(
                matches!(check(s, &strict), Err(ClipError::PrivateAddress(_))),
                "{s}"
            );
        }
        let mut open = FetchPolicy::strict();
        open.allow_private = true;
        assert!(check("https://127.0.0.1/", &open).is_ok());
        assert!(check("https://[::1]/", &open).is_ok());
    }

    #[test]
    fn accepted_urls() {
        let strict = FetchPolicy::strict();
        assert!(check("https://93.184.216.34/", &strict).is_ok());
        // A host name passes here. The resolver checks what it resolves to.
        assert!(check("https://localhost/", &strict).is_ok());
        assert!(check("https://example.com:8443/", &strict).is_ok());
    }

    #[test]
    fn credentials_are_refused() {
        let strict = FetchPolicy::strict();
        assert!(matches!(
            check("https://user:pass@example.com/", &strict),
            Err(ClipError::BadUrl(_))
        ));
        assert!(matches!(
            check("https://user@example.com/", &strict),
            Err(ClipError::BadUrl(_))
        ));
    }

    #[test]
    fn not_https_error_hides_credentials() {
        let strict = FetchPolicy::strict();
        let err = check("ftp://user:pass@example.com/", &strict).unwrap_err();
        assert!(matches!(err, ClipError::NotHttps(_)));
        assert!(!err.to_string().contains("pass"), "{err}");
        assert!(!err.to_string().contains("user"), "{err}");
    }

    #[test]
    fn empty_host_never_parses() {
        // The `url` crate refuses an https URL with no host, so `check_url`
        // never sees one. Its no-host branch is a guard for other schemes.
        assert!(Url::parse("https:///").is_err());
    }
}
