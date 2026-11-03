//! Getting the bytes: secure links (signed CDN addresses for one product,
//! valid for a few hours), the address of a v2 chunk or of a v1 `main.bin`
//! under them, and decoding a chunk.

use std::io::Read;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use md5::{Digest, Md5};
use serde_json::{Map, Value};

use crate::api::{self, Client};
use crate::files::ChunkRef;

/// One CDN of a secure link.
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
    /// `{base_url}/...{path}`-like template; empty for a plain `url`.
    pub url_format: String,
    pub parameters: Map<String, Value>,
    pub url: String,
}

impl Endpoint {
    fn parse(v: &Value) -> Option<Self> {
        let parameters = v.get("parameters").and_then(Value::as_object).cloned().unwrap_or_default();
        let url_format = v.get("url_format").and_then(Value::as_str).unwrap_or_default().to_string();
        let url = v.get("url").and_then(Value::as_str).unwrap_or_default().to_string();
        (!url_format.is_empty() || !url.is_empty()).then_some(Self { url_format, parameters, url })
    }

    /// The address of `tail` (`/ab/cd/abcd…` or `/main.bin`) below the link:
    /// through the template when it has a `{path}`, else after the plain
    /// address (redistributables' links have no parameters).
    pub fn address(&self, tail: &str) -> String {
        if !self.url_format.contains("{path}") {
            let base = if self.url.is_empty() { &self.url_format } else { &self.url };
            return format!("{base}{tail}");
        }
        let mut params = self.parameters.clone();
        let path = params.get("path").and_then(Value::as_str).unwrap_or_default().to_string();
        params.insert("path".into(), Value::String(format!("{path}{tail}")));
        merge(&self.url_format, &params)
    }
}

/// `{key}` of `format` replaced by each parameter.
fn merge(format: &str, params: &Map<String, Value>) -> String {
    let mut out = format.to_string();
    for (k, v) in params {
        let text = match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        out = out.replace(&format!("{{{k}}}"), &text);
    }
    out
}

/// A secure link: its CDNs in GOG's order.
#[derive(Debug, Clone)]
pub struct SecureLink {
    pub endpoints: Vec<Endpoint>,
    pub fetched: Instant,
}

/// Links are asked for again after this, before GOG's tokens run out.
pub const LINK_LIFETIME: Duration = Duration::from_secs(60 * 50);

impl SecureLink {
    pub fn parse(doc: &Value) -> Result<Self> {
        let endpoints: Vec<Endpoint> =
            doc.get("urls").and_then(Value::as_array).map(|l| l.iter().filter_map(Endpoint::parse).collect()).unwrap_or_default();
        if endpoints.is_empty() {
            bail!("GOG sent no download address");
        }
        Ok(Self { endpoints, fetched: Instant::now() })
    }

    /// Whether it is old enough to ask for a new one.
    pub fn is_stale(&self) -> bool {
        self.fetched.elapsed() > LINK_LIFETIME
    }

    /// Every address of a v2 chunk, best first.
    pub fn chunk_urls(&self, compressed_md5: &str) -> Vec<String> {
        let tail = format!("/{}", api::galaxy_path(compressed_md5));
        self.endpoints.iter().map(|e| e.address(&tail)).collect()
    }

    /// Every address of a v1 product's `main.bin`.
    pub fn main_bin_urls(&self) -> Vec<String> {
        self.endpoints.iter().map(|e| e.address("/main.bin")).collect()
    }
}

impl Client {
    /// The secure link of `product`'s files: `generation` 2 (path `/`) or 1
    /// (path [`crate::v1::Meta::link_path`]). `root` reaches other trees
    /// (`/patches/store`).
    pub fn secure_link(&self, product: &str, path: &str, generation: u8, root: Option<&str>) -> Result<SecureLink> {
        let mut url = match generation {
            1 => format!("{}/products/{product}/secure_link?_version=2&type=depot&path={}", api::CONTENT_SYSTEM, api::encode(path, true)),
            _ => format!("{}/products/{product}/secure_link?_version=2&generation=2&path={}", api::CONTENT_SYSTEM, api::encode(path, true)),
        };
        if let Some(root) = root {
            url.push_str(&format!("&root={}", api::encode(root, true)));
        }
        SecureLink::parse(&self.get_json(&url).with_context(|| format!("the download address of {product}"))?)
    }

    /// The link of redistributables (the same for every account).
    pub fn dependency_link(&self) -> Result<SecureLink> {
        let url = format!("{}/open_link?generation=2&_version=2&path=/dependencies/store/", api::CONTENT_SYSTEM);
        SecureLink::parse(&self.get_json(&url)?)
    }

    /// Downloads and decodes a v2 chunk, trying each CDN in turn.
    pub fn fetch_chunk(&self, link: &SecureLink, chunk: &ChunkRef) -> Result<Vec<u8>> {
        let mut last = None;
        for url in link.chunk_urls(&chunk.compressed_md5) {
            match self.get(&url, &[]).and_then(|r| {
                api::check(&r, &url)?;
                decode(&r.body, chunk)
            }) {
                Ok(data) => return Ok(data),
                Err(err) => last = Some(err),
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("no address for the chunk")))
    }

    /// `size` bytes of a v1 product's `main.bin` from `offset`.
    pub fn fetch_range(&self, link: &SecureLink, offset: u64, size: u64) -> Result<Vec<u8>> {
        let mut last = None;
        for url in link.main_bin_urls() {
            match self.get_range(&url, offset, size) {
                Ok(data) => return Ok(data),
                Err(err) => last = Some(err),
            }
        }
        Err(last.unwrap_or_else(|| anyhow!("no address for the file")))
    }
}

/// Inflates a chunk and checks it against its md5.
pub fn decode(compressed: &[u8], chunk: &ChunkRef) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(chunk.size as usize);
    flate2::read::ZlibDecoder::new(compressed)
        .read_to_end(&mut out)
        .with_context(|| format!("chunk {} is damaged", chunk.compressed_md5))?;
    if out.len() as u64 != chunk.size {
        bail!("chunk {} has {} bytes, not {}", chunk.compressed_md5, out.len(), chunk.size);
    }
    if !chunk.md5.is_empty() && !hex::encode(Md5::digest(&out)).eq_ignore_ascii_case(&chunk.md5) {
        bail!("chunk {} does not match its checksum", chunk.compressed_md5);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    #[test]
    fn chunk_addresses_follow_the_template() {
        let link = SecureLink::parse(&json!({"urls": [
            {"endpoint_name": "fastly", "url_format": "{base_url}/token=nva={expires_at}~hmac={hmac}{path}",
             "parameters": {"base_url": "https://cdn", "path": "/content-system/v2/store/42", "expires_at": 1700000000, "hmac": "h"}},
            {"endpoint_name": "plain", "url": "https://other/store/42"},
            {"endpoint_name": "redist", "url_format": "https://cdn/dependencies/store", "url": "https://cdn/dependencies/store", "parameters": {}}
        ]}))
        .unwrap();
        assert_eq!(
            link.chunk_urls("abcdef"),
            [
                "https://cdn/token=nva=1700000000~hmac=h/content-system/v2/store/42/ab/cd/abcdef",
                "https://other/store/42/ab/cd/abcdef",
                "https://cdn/dependencies/store/ab/cd/abcdef"
            ]
        );
        assert_eq!(link.main_bin_urls()[0], "https://cdn/token=nva=1700000000~hmac=h/content-system/v2/store/42/main.bin");
        assert!(!link.is_stale());
        assert!(SecureLink::parse(&json!({"urls": []})).is_err());
    }

    #[test]
    fn chunks_are_inflated_and_checked() {
        let data = b"some game bytes".repeat(20);
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&data).unwrap();
        let compressed = z.finish().unwrap();
        let mut chunk = ChunkRef {
            md5: hex::encode(Md5::digest(&data)),
            compressed_md5: "z".into(),
            size: data.len() as u64,
            compressed_size: compressed.len() as u64,
        };
        assert_eq!(decode(&compressed, &chunk).unwrap(), data);
        chunk.md5 = "0".repeat(32);
        assert!(decode(&compressed, &chunk).is_err());
        chunk.size += 1;
        assert!(decode(&compressed, &chunk).is_err());
        assert!(decode(b"junk", &chunk).is_err());
    }
}
