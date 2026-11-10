//! Where an extension may send requests: the hosts of its manifest, checked
//! on every address the script builds (and on every redirect), and the rate
//! the host keeps for it.

use std::net::{IpAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use url::{Host, Url};

/// One `[network]` entry: `api.example.org` (https), `*.example.org` (its
/// subdomains, https), `http://old.example.org` (that host over http too).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRule {
    /// Lowercase, without the `*.`.
    pub host: String,
    /// Its subdomains rather than the host itself.
    pub subdomains: bool,
    /// Plain `http` is allowed besides `https`.
    pub http: bool,
}

impl HostRule {
    pub fn parse(text: &str) -> Result<Self, String> {
        let raw = text.trim();
        let (http, rest) = match raw.split_once("://") {
            Some(("http", rest)) => (true, rest),
            Some(("https", rest)) => (false, rest),
            Some(_) => return Err(format!("\"{raw}\": only http and https hosts")),
            None => (false, raw),
        };
        let (subdomains, host) = match rest.strip_prefix("*.") {
            Some(h) => (true, h),
            None => (false, rest),
        };
        let host = host.to_ascii_lowercase();
        if host.is_empty() || host.contains(['/', ':', '@', '?', '#', '*', ' ']) {
            return Err(format!("\"{raw}\" is not a host name (no path, port or credentials)"));
        }
        if host.parse::<IpAddr>().is_ok() || host.trim_matches(['[', ']']).parse::<IpAddr>().is_ok() {
            return Err(format!("\"{raw}\": an IP address is not allowed, name the host"));
        }
        let labels_ok = host
            .split('.')
            .all(|l| !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') && !l.starts_with('-'));
        if !labels_ok || !host.contains('.') {
            return Err(format!("\"{raw}\" is not a host name such as api.example.org"));
        }
        Ok(Self { host, subdomains, http })
    }

    fn matches(&self, host: &str) -> bool {
        if self.subdomains {
            host.strip_suffix(self.host.as_str()).is_some_and(|head| head.len() > 1 && head.ends_with('.'))
        } else {
            host == self.host
        }
    }

    /// As written in a manifest.
    pub fn text(&self) -> String {
        format!("{}{}{}", if self.http { "http://" } else { "" }, if self.subdomains { "*." } else { "" }, self.host)
    }
}

/// Why an address was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denied {
    NotAnAddress,
    /// Only `http` and `https`.
    Scheme,
    /// `user:password@` in the address.
    Credentials,
    /// A host given as an IP address.
    IpAddress,
    /// A port other than the scheme's.
    Port,
    /// Not in the manifest's hosts.
    Host(String),
    /// `http` to a host listed for `https` only.
    PlainHttp(String),
    /// The name leads to this computer or the local network.
    Private(String),
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnAddress => write!(f, "not an address"),
            Self::Scheme => write!(f, "only http(s) addresses"),
            Self::Credentials => write!(f, "addresses with credentials are not allowed"),
            Self::IpAddress => write!(f, "IP addresses are not allowed"),
            Self::Port => write!(f, "only the default port"),
            Self::Host(h) => write!(f, "{h} is not in the manifest's [network] hosts"),
            Self::PlainHttp(h) => write!(f, "{h} is allowed over https only"),
            Self::Private(h) => write!(f, "{h} leads to this computer or the local network"),
        }
    }
}

/// `text` as an address one of `rules` allows.
pub fn check(text: &str, rules: &[HostRule]) -> Result<Url, Denied> {
    let url = Url::parse(text).map_err(|_| Denied::NotAnAddress)?;
    let http = match url.scheme() {
        "https" => false,
        "http" => true,
        _ => return Err(Denied::Scheme),
    };
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Denied::Credentials);
    }
    let host = match url.host() {
        Some(Host::Domain(d)) => d.to_ascii_lowercase(),
        Some(Host::Ipv4(_) | Host::Ipv6(_)) => return Err(Denied::IpAddress),
        None => return Err(Denied::NotAnAddress),
    };
    if url.port().is_some() {
        return Err(Denied::Port);
    }
    let rule = rules.iter().find(|r| r.matches(&host)).ok_or_else(|| Denied::Host(host.clone()))?;
    if http && !rule.http {
        return Err(Denied::PlainHttp(host));
    }
    Ok(url)
}

/// Whether `addr` is this computer, the local network or otherwise not on
/// the internet.
fn is_private(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(a) => {
            a.is_loopback() || a.is_private() || a.is_link_local() || a.is_unspecified() || a.is_broadcast() || a.octets()[0] == 0
                // 100.64.0.0/10, carrier-grade NAT.
                || (a.octets()[0] == 100 && (a.octets()[1] & 0xc0) == 64)
        }
        IpAddr::V6(a) => {
            let s = a.segments();
            a.is_loopback()
                || a.is_unspecified()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || a.to_ipv4_mapped().is_some_and(|v4| is_private(IpAddr::V4(v4)))
        }
    }
}

/// Resolves `url`'s host and refuses it when a name leads to a private
/// address (a listed name pointed at the local network). Best effort: the
/// request resolves the name again. `Ok(false)`: the name does not resolve.
pub fn public(url: &Url) -> Result<bool, Denied> {
    let Some(host) = url.host_str() else { return Err(Denied::NotAnAddress) };
    let port = url.port_or_known_default().unwrap_or(443);
    let Ok(addrs) = (host, port).to_socket_addrs() else { return Ok(false) };
    let addrs: Vec<IpAddr> = addrs.map(|a| a.ip()).collect();
    if addrs.iter().any(|a| is_private(*a)) {
        return Err(Denied::Private(host.to_owned()));
    }
    Ok(!addrs.is_empty())
}

/// The rate of one extension, shared by all its calls: requests are spaced
/// `interval` apart.
pub struct RateLimiter {
    interval: Duration,
    next: Mutex<Option<Instant>>,
}

impl RateLimiter {
    pub fn new(interval: Duration) -> Self {
        Self { interval, next: Mutex::new(None) }
    }

    /// Takes the next free slot and returns when it is. A caller that gives
    /// up still used it.
    pub fn slot(&self) -> Instant {
        let mut next = self.next.lock();
        let now = Instant::now();
        let at = next.map_or(now, |n| n.max(now));
        *next = Some(at + self.interval);
        at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(list: &[&str]) -> Vec<HostRule> {
        list.iter().map(|r| HostRule::parse(r).unwrap()).collect()
    }

    #[test]
    fn rules_are_host_names_only() {
        assert_eq!(HostRule::parse("API.example.org").unwrap().host, "api.example.org");
        let wild = HostRule::parse("*.wikipedia.org").unwrap();
        assert!(wild.subdomains && !wild.http && wild.text() == "*.wikipedia.org");
        assert!(HostRule::parse("http://old.example.org").unwrap().http);
        for bad in ["127.0.0.1", "[::1]", "example.org/path", "example.org:8080", "user@example.org", "ftp://x.org", "localhost", "", "*"] {
            assert!(HostRule::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_listed_hosts_over_https() {
        let r = rules(&["api.example.org", "*.wiki.org", "http://plain.example.org"]);
        assert!(check("https://api.example.org/v1?q=1", &r).is_ok());
        assert!(check("https://en.wiki.org/x", &r).is_ok());
        assert!(check("https://a.b.wiki.org/x", &r).is_ok());
        assert_eq!(check("https://wiki.org/x", &r), Err(Denied::Host("wiki.org".into())));
        assert_eq!(check("https://evilwiki.org/x", &r), Err(Denied::Host("evilwiki.org".into())));
        assert_eq!(check("https://api.example.org.evil.com/", &r), Err(Denied::Host("api.example.org.evil.com".into())));
        assert_eq!(check("http://api.example.org/", &r), Err(Denied::PlainHttp("api.example.org".into())));
        assert!(check("http://plain.example.org/", &r).is_ok());
        assert_eq!(check("https://user:pw@api.example.org/", &r), Err(Denied::Credentials));
        assert_eq!(check("https://api.example.org:8443/", &r), Err(Denied::Port));
        assert!(check("https://api.example.org:443/", &r).is_ok(), "the default port, written out");
        assert_eq!(check("https://127.0.0.1/", &r), Err(Denied::IpAddress));
        assert_eq!(check("https://0x7f.1/", &r), Err(Denied::IpAddress));
        assert_eq!(check("https://[::1]/", &r), Err(Denied::IpAddress));
        assert_eq!(check("file:///etc/passwd", &r), Err(Denied::Scheme));
        assert_eq!(check("data:text/plain,x", &r), Err(Denied::Scheme));
        assert_eq!(check("/relative", &r), Err(Denied::NotAnAddress));
    }

    #[test]
    fn a_name_that_leads_home_is_refused() {
        let url = Url::parse("https://localhost/").unwrap();
        assert!(matches!(public(&url), Err(Denied::Private(_))));
        for ip in ["10.0.0.1", "192.168.1.2", "172.16.0.1", "169.254.1.1", "100.64.0.1", "0.0.0.0", "::1", "fd00::1", "fe80::1"] {
            assert!(is_private(ip.parse().unwrap()), "{ip}");
        }
        assert!(!is_private("93.184.216.34".parse().unwrap()));
    }

    #[test]
    fn the_rate_spaces_requests() {
        let rate = RateLimiter::new(Duration::from_millis(100));
        let a = rate.slot();
        let b = rate.slot();
        let c = rate.slot();
        assert_eq!(b - a, Duration::from_millis(100));
        assert_eq!(c - b, Duration::from_millis(100));
    }
}
