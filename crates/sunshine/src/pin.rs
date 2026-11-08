//! Trust on first use for a Sunshine on another computer. Sunshine makes
//! its own self-signed certificate, so no authority can vouch for it:
//! instead the SHA-256 of the certificate seen first is kept with the
//! settings, and a different one is refused during the TLS handshake
//! itself (no window between a check and the request). The user can accept
//! a new certificate in Settings.
//!
//! On this computer (`localhost`, `127.0.0.1`, `::1`) the certificate is
//! not checked: what answers there is already on this machine.
//!
//! The connector plugs into ureq's connector chain (TCP, then this TLS
//! layer); handshake signatures are still checked with rustls' own
//! algorithms, only the certificate's identity is the pin.

use std::io::{Read, Write};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned};
use sha2::{Digest, Sha256};
use ureq::unversioned::transport::{Buffers, ConnectionDetails, Connector, Either, LazyBuffers, NextTimeout, Transport, TransportAdapter};

/// `localhost`, `127.0.0.1` (any `127.x`), `::1`, with or without brackets.
pub fn is_loopback(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost") || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// The host of `https://host:port/path` (brackets kept off IPv6).
pub fn host_of(address: &str) -> &str {
    let rest = address.split_once("://").map_or(address, |(_, r)| r);
    let authority = rest.split('/').next().unwrap_or("");
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    if let Some(v6) = authority.strip_prefix('[') {
        return v6.split(']').next().unwrap_or("");
    }
    authority.split(':').next().unwrap_or("")
}

/// `AB:CD:..` of the SHA-256 of a certificate (as browsers show it).
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

/// What the handshake saw, for the caller after a request.
#[derive(Debug, Default)]
pub struct Seen {
    /// The server certificate's fingerprint, once a handshake got that far.
    pub fingerprint: Option<String>,
    /// It differed from the pin (the handshake was refused).
    pub changed: bool,
}

#[derive(Debug)]
struct PinVerifier {
    /// `None`: first use, any certificate is taken (and reported).
    pin: Option<String>,
    seen: Arc<Mutex<Seen>>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = fingerprint(end_entity.as_ref());
        let changed = self.pin.as_ref().is_some_and(|p| !p.eq_ignore_ascii_case(&fp));
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Seen { fingerprint: Some(fp), changed };
        }
        if changed {
            return Err(rustls::Error::General("Sunshine's certificate changed".into()));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// TLS for ureq whose only trust is the pin.
#[derive(Debug)]
pub struct PinnedConnector {
    config: Arc<ClientConfig>,
}

impl PinnedConnector {
    /// `pin`: the fingerprint kept from the first use, `None` for a first use.
    /// What the handshake sees goes to `seen`.
    pub fn new(pin: Option<String>, seen: Arc<Mutex<Seen>>) -> Self {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let algorithms = provider.signature_verification_algorithms;
        let config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("ring supports the default TLS versions")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinVerifier { pin, seen, algorithms }))
            .with_no_client_auth();
        Self { config: Arc::new(config) }
    }
}

impl<In: Transport> Connector<In> for PinnedConnector {
    type Out = Either<In, PinnedTransport>;

    fn connect(&self, details: &ConnectionDetails, chained: Option<In>) -> Result<Option<Self::Out>, ureq::Error> {
        let Some(transport) = chained else { return Ok(None) };
        if !details.needs_tls() || transport.is_tls() {
            return Ok(Some(Either::A(transport)));
        }
        let host = details.uri.host().unwrap_or_default().trim_start_matches('[').trim_end_matches(']').to_string();
        let name = ServerName::try_from(host).map_err(|_| ureq::Error::Tls("not a host name"))?;
        let mut conn = ClientConnection::new(self.config.clone(), name).map_err(|e| ureq::Error::Other(Box::new(e)))?;
        let mut sock = TransportAdapter::new(transport.boxed());
        sock.set_timeout(details.timeout);
        conn.complete_io(&mut sock)?;
        let buffers = LazyBuffers::new(details.config.input_buffer_size(), details.config.output_buffer_size());
        Ok(Some(Either::B(PinnedTransport { buffers, stream: StreamOwned { conn, sock } })))
    }
}

pub struct PinnedTransport {
    buffers: LazyBuffers,
    stream: StreamOwned<ClientConnection, TransportAdapter>,
}

impl std::fmt::Debug for PinnedTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PinnedTransport")
    }
}

impl Transport for PinnedTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.stream.get_mut().set_timeout(timeout);
        let output = &self.buffers.output()[..amount];
        self.stream.write_all(output)?;
        Ok(())
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.stream.get_mut().set_timeout(timeout);
        let input = self.buffers.input_append_buf();
        let amount = self.stream.read(input)?;
        self.buffers.input_appended(amount);
        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        self.stream.get_mut().get_mut().is_open()
    }

    fn is_tls(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_hosts_and_hosts_of_addresses() {
        for h in ["localhost", "LOCALHOST", "127.0.0.1", "127.1.2.3", "::1", "[::1]"] {
            assert!(is_loopback(h), "{h}");
        }
        for h in ["pc.lan", "192.168.1.20", "10.0.0.1", "::2", "localhost.evil.com", ""] {
            assert!(!is_loopback(h), "{h}");
        }
        assert_eq!(host_of("https://localhost:47990"), "localhost");
        assert_eq!(host_of("https://[::1]:47990/x"), "::1");
        assert_eq!(host_of("https://192.168.1.20"), "192.168.1.20");
        assert_eq!(host_of("https://user@pc.lan:47990"), "pc.lan");
    }

    #[test]
    fn fingerprints_look_like_a_browsers() {
        let fp = fingerprint(b"abc");
        assert_eq!(fp.len(), 32 * 3 - 1);
        assert!(fp.starts_with("BA:78:16:BF"));
    }
}
