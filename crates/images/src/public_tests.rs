use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};

use super::*;

#[test]
fn only_public_addresses_are_public() {
    let refused = [
        "0.0.0.0",
        "0.1.2.3",
        "10.1.2.3",
        "100.64.0.1",
        "100.127.255.254",
        "127.0.0.1",
        "127.8.9.10",
        "169.254.169.254",
        "172.16.0.1",
        "172.31.255.255",
        "192.0.0.8",
        "192.0.2.1",
        "192.88.99.1",
        "192.168.1.1",
        "198.18.0.1",
        "198.19.255.255",
        "198.51.100.7",
        "203.0.113.9",
        "224.0.0.1",
        "239.255.255.250",
        "240.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "::127.0.0.1",
        "::ffff:127.0.0.1",
        "::ffff:192.168.1.1",
        "64:ff9b::10.0.0.1",
        "64:ff9b::7f00:1",
        "64:ff9b:1::1",
        "100::1",
        "2001::1",
        "2001:0:4136:e378:8000:63bf:3fff:fdd2",
        "2001:db8::1",
        "2002:c0a8:101::1",
        "2002:0808:0808::1",
        "3fff::1",
        "fc00::1",
        "fd12:3456::1",
        "fe80::1",
        "fec0::1",
        "ff02::1",
        "5f00::1",
    ];
    for ip in refused {
        assert!(!is_public(ip.parse().unwrap()), "{ip} is not public");
    }
    let public = [
        "1.1.1.1",
        "8.8.8.8",
        "93.184.216.34",
        "100.63.255.255",
        "100.128.0.1",
        "172.15.255.255",
        "172.32.0.1",
        "192.0.1.1",
        "192.169.0.1",
        "198.17.255.255",
        "198.20.0.1",
        "223.255.255.255",
        "::ffff:8.8.8.8",
        "64:ff9b::808:808",
        "2001:200::1",
        "2001:4860:4860::8888",
        "2606:4700:4700::1111",
        "2a00:1450:4001::1",
    ];
    for ip in public {
        assert!(is_public(ip.parse().unwrap()), "{ip} is public");
    }
}

#[test]
fn a_mark_says_where_a_url_came_from() {
    assert_eq!(marked("https://a.example/c.jpg"), "https://a.example/c.jpg#heronite-extension");
    assert_eq!(marked("https://a.example/c.jpg#top"), "https://a.example/c.jpg#heronite-extension");
    assert!(is_extension_url(&marked("https://a.example/c.jpg?x=1")));
    assert!(!is_extension_url("https://a.example/c.jpg"));
    assert!(!is_extension_url("https://a.example/c.jpg#other"));
    assert_eq!(shown(&Url::parse("https://a.example/c.jpg?key=secret#heronite-extension").unwrap()), "https://a.example/c.jpg");
}

/// A server answering each path with a status, a `location` and a body.
fn serve(routes: Vec<(&'static str, u16, String)>) -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { break };
            let mut buf = [0u8; 2048];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
            let Some((_, status, to)) = routes.iter().find(|r| r.0 == path) else { continue };
            let head = format!("HTTP/1.1 {status} X\r\nLocation: {to}\r\nContent-Length: 4\r\nConnection: close\r\n\r\nGIF8");
            let _ = s.write_all(head.as_bytes());
        }
    });
    addr
}

/// Resolves `pictures.test` to the local server, as if it were public;
/// every other name goes through [`PublicResolver`].
#[derive(Debug)]
struct TestResolver(SocketAddr);

impl Resolver for TestResolver {
    fn resolve(&self, uri: &Uri, config: &ureq::config::Config, timeout: NextTimeout) -> Result<ResolvedSocketAddrs, Error> {
        if uri.host() == Some("pictures.test") {
            let mut out = self.empty();
            out.push(self.0);
            return Ok(out);
        }
        PublicResolver.resolve(uri, config, timeout)
    }
}

#[test]
fn a_redirect_is_checked_like_the_first_address() {
    let addr = serve(vec![
        ("/ok", 200, String::new()),
        ("/to-ok", 302, "/ok".into()),
        ("/to-loopback", 302, format!("http://127.0.0.1:{}/ok", 0)),
        ("/to-localhost", 302, "http://localhost/ok".into()),
        ("/to-router", 301, "http://192.168.1.1/cgi-bin/x".into()),
        ("/to-file", 302, "file:///etc/passwd".into()),
        ("/to-credentials", 302, "http://user:pw@pictures.test/ok".into()),
        ("/loop", 302, "/loop".into()),
    ]);
    let agent = Agent::with_parts(
        Agent::config_builder().http_status_as_error(false).max_redirects(0).proxy(None).build(),
        DefaultConnector::default(),
        TestResolver(addr),
    );
    let base = format!("http://pictures.test:{}", addr.port());
    let get = |path: &str| fetch_with(&agent, &format!("{base}{path}#heronite-extension"), 1024, Duration::from_secs(5));
    assert_eq!(get("/ok").unwrap(), b"GIF8");
    assert_eq!(get("/to-ok").unwrap(), b"GIF8");
    for (path, why) in [
        ("/to-loopback", "local network"),
        ("/to-localhost", "local network"),
        ("/to-router", "local network"),
        ("/to-file", "only http(s)"),
        ("/to-credentials", "credentials"),
        ("/loop", "redirects"),
    ] {
        let e = get(path).unwrap_err();
        assert!(e.contains(why), "{path}: {e}");
    }
    let e = fetch_with(&agent, &format!("{base}/missing?key=secret"), 1024, Duration::from_secs(5)).unwrap_err();
    assert!(!e.contains("secret"), "{e}");
    assert!(get("/ok").is_ok() && fetch_with(&agent, &format!("{base}/ok"), 2, Duration::from_secs(5)).is_err(), "over the size");
}

#[test]
fn a_name_or_an_address_of_this_computer_is_refused() {
    let addr = serve(vec![("/ok", 200, String::new())]);
    for url in [format!("http://127.0.0.1:{}/ok", addr.port()), format!("http://localhost:{}/ok", addr.port())] {
        let e = fetch(&marked(&url), 1024, Duration::from_secs(5)).unwrap_err();
        assert!(e.contains("local network"), "{url}: {e}");
    }
}
