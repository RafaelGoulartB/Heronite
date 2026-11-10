//! Requests that may reach the public internet only: everything a metadata
//! extension causes to be fetched (its script's requests, the pictures and
//! thumbnails it names). An extension is written by someone the user does
//! not know; it must never make the app reach this computer or the local
//! network.
//!
//! - [`PublicResolver`] resolves a name and refuses it when any of its
//!   addresses is not a public one ([`is_public`]); the connection then goes
//!   to the addresses it checked, never to a second answer of the DNS.
//! - [`agent`] is an agent with that resolver and no proxy (a proxy would
//!   resolve the name itself).
//! - [`fetch`] downloads a picture through it, following redirects one by
//!   one, each checked again (scheme, credentials, addresses).
//!
//! A URL an extension gave carries [`EXTENSION_MARK`] as its fragment
//! ([`marked`]): it is saved that way in the records, so whatever later
//! downloads it (the images cache, a copy to disk) knows to come here.

use std::fmt;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;
use std::time::Duration;

use ureq::config::ConfigBuilder;
use ureq::http::Uri;
use ureq::typestate::AgentScope;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};
use ureq::{Agent, Error};
use url::Url;

/// The fragment that marks a URL an extension gave.
pub const EXTENSION_MARK: &str = "heronite-extension";

/// Redirects [`fetch`] follows, each checked like the first address.
const MAX_REDIRECTS: u32 = 5;

/// `url` with [`EXTENSION_MARK`] as its fragment (any fragment it had is
/// dropped: a server never sees one).
pub fn marked(url: &str) -> String {
    let base = url.split_once('#').map_or(url, |(b, _)| b);
    format!("{base}#{EXTENSION_MARK}")
}

/// Whether `url` came from an extension ([`marked`]).
pub fn is_extension_url(url: &str) -> bool {
    url.rsplit_once('#').is_some_and(|(_, f)| f == EXTENSION_MARK)
}

/// Whether `ip` is a public internet address: not this computer, a local or
/// private network, a shared (carrier-grade NAT) range, multicast,
/// documentation, benchmarking, a deprecated tunnel or a reserved range
/// (the IANA IPv4 and IPv6 special-purpose address registries). An IPv6
/// address that carries an IPv4 one (mapped, NAT64) is judged by it.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => is_public_v4(a),
        IpAddr::V6(a) => is_public_v6(a),
    }
}

fn is_public_v4(a: Ipv4Addr) -> bool {
    let [o0, o1, o2, _] = a.octets();
    let refused = o0 == 0 // 0.0.0.0/8, "this network"
        || o0 == 10 // 10.0.0.0/8, private
        || (o0 == 100 && (o1 & 0xc0) == 64) // 100.64.0.0/10, shared (CGNAT)
        || o0 == 127 // 127.0.0.0/8, loopback
        || (o0 == 169 && o1 == 254) // 169.254.0.0/16, link-local
        || (o0 == 172 && (o1 & 0xf0) == 16) // 172.16.0.0/12, private
        || (o0 == 192 && o1 == 0 && o2 == 0) // 192.0.0.0/24, IETF protocol assignments
        || (o0 == 192 && o1 == 0 && o2 == 2) // 192.0.2.0/24, documentation
        || (o0 == 192 && o1 == 88 && o2 == 99) // 192.88.99.0/24, 6to4 relay (deprecated)
        || (o0 == 192 && o1 == 168) // 192.168.0.0/16, private
        || (o0 == 198 && (o1 & 0xfe) == 18) // 198.18.0.0/15, benchmarking
        || (o0 == 198 && o1 == 51 && o2 == 100) // 198.51.100.0/24, documentation
        || (o0 == 203 && o1 == 0 && o2 == 113) // 203.0.113.0/24, documentation
        || o0 >= 224; // 224.0.0.0/4 multicast, 240.0.0.0/4 reserved, broadcast
    !refused
}

fn is_public_v6(a: Ipv6Addr) -> bool {
    let s = a.segments();
    // ::ffff:0:0/96, an IPv4 address mapped.
    if let Some(v4) = a.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    // 64:ff9b::/96, NAT64 to the IPv4 internet.
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        return is_public_v4(Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8));
    }
    // Only global unicast, 2000::/3, minus its special ranges. Refused
    // besides: ::/128, ::1, IPv4-compatible ::/96, 64:ff9b:1::/48, 100::/64,
    // unique-local fc00::/7, link-local fe80::/10, site-local fec0::/10,
    // multicast ff00::/8 and everything else outside 2000::/3.
    let global = (s[0] & 0xe000) == 0x2000;
    let special = s[0] == 0x2001 && s[1] < 0x0200 // 2001::/23, IETF protocol assignments (Teredo...)
        || (s[0] == 0x2001 && s[1] == 0x0db8) // 2001:db8::/32, documentation
        || s[0] == 0x2002 // 2002::/16, 6to4
        || (s[0] & 0xfff0) == 0x3ff0; // 3fff::/20, documentation
    global && !special
}

/// A name that leads to an address that is not public.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotPublic(pub String);

impl fmt::Display for NotPublic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} leads to this computer or the local network", self.0)
    }
}

impl std::error::Error for NotPublic {}

/// Whether `e`, from an agent of [`agent`], is a refusal of [`PublicResolver`].
pub fn not_public(e: &Error) -> Option<NotPublic> {
    match e {
        Error::Io(io) => io.get_ref().and_then(|i| i.downcast_ref::<NotPublic>()).cloned(),
        _ => None,
    }
}

/// Resolves like the default resolver (within the request's time), then
/// refuses the name when any address is not public.
#[derive(Debug, Default)]
pub struct PublicResolver;

impl Resolver for PublicResolver {
    fn resolve(&self, uri: &Uri, config: &ureq::config::Config, timeout: NextTimeout) -> Result<ResolvedSocketAddrs, Error> {
        let addrs = DefaultResolver::default().resolve(uri, config, timeout)?;
        if addrs.iter().any(|a| !is_public(a.ip())) {
            let host = uri.host().unwrap_or("").to_owned();
            return Err(Error::Io(std::io::Error::new(std::io::ErrorKind::PermissionDenied, NotPublic(host))));
        }
        Ok(addrs)
    }
}

/// An agent of `config` that connects to public addresses only, never
/// through a proxy.
pub fn agent(config: ConfigBuilder<AgentScope>) -> Agent {
    Agent::with_parts(config.proxy(None).build(), DefaultConnector::default(), PublicResolver)
}

fn picture_agent() -> &'static Agent {
    static AGENT: OnceLock<Agent> = OnceLock::new();
    AGENT.get_or_init(|| agent(Agent::config_builder().http_status_as_error(false).max_redirects(0).user_agent("heronite-images/0.1")))
}

/// Downloads the picture at `url` (an extension's, [`marked`] or not) from
/// the public internet only: at most `max_bytes`, within `timeout` for
/// each request, following up to [`MAX_REDIRECTS`] redirects, each to an
/// http(s) address without credentials whose name is public. The error
/// names no query or fragment of the address.
pub fn fetch(url: &str, max_bytes: u64, timeout: Duration) -> Result<Vec<u8>, String> {
    fetch_with(picture_agent(), url, max_bytes, timeout)
}

fn fetch_with(agent: &Agent, url: &str, max_bytes: u64, timeout: Duration) -> Result<Vec<u8>, String> {
    let mut url = Url::parse(url).map_err(|_| "not an address".to_owned())?;
    for _ in 0..=MAX_REDIRECTS {
        url.set_fragment(None);
        if !matches!(url.scheme(), "http" | "https") {
            return Err(format!("{}: only http(s) addresses", shown(&url)));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(format!("{}: addresses with credentials are not allowed", shown(&url)));
        }
        let req = agent.get(url.as_str()).config().timeout_global(Some(timeout)).build();
        let mut resp = req.call().map_err(|e| match not_public(&e) {
            Some(n) => n.to_string(),
            None => format!("{}: {e}", shown(&url)),
        })?;
        let status = resp.status().as_u16();
        if (300..400).contains(&status) {
            let to =
                resp.headers().get("location").and_then(|v| v.to_str().ok()).ok_or_else(|| format!("{}: status {status}", shown(&url)))?;
            url = url.join(to).map_err(|_| format!("{}: a redirect to something that is not an address", shown(&url)))?;
            continue;
        }
        if !(200..300).contains(&status) {
            return Err(format!("{}: status {status}", shown(&url)));
        }
        let mut bytes = Vec::new();
        resp.body_mut().as_reader().take(max_bytes + 1).read_to_end(&mut bytes).map_err(|e| format!("{}: {e}", shown(&url)))?;
        if bytes.len() as u64 > max_bytes {
            return Err(format!("{}: larger than {max_bytes} bytes", shown(&url)));
        }
        return Ok(bytes);
    }
    Err(format!("{}: more than {MAX_REDIRECTS} redirects", shown(&url)))
}

/// `url` without its query and fragment, for messages (a query may hold a
/// key).
pub fn shown(url: &Url) -> String {
    let mut u = url.clone();
    u.set_query(None);
    u.set_fragment(None);
    u.to_string()
}

#[cfg(test)]
#[path = "public_tests.rs"]
mod tests;
