//! The XMP packet a converted WebP carries. It is the only record of the
//! conversion: a file that has it was made by us from a PNG and is never
//! processed again, and it remembers the original name and size (for the
//! "space freed" total and for finishing a conversion a crash interrupted).

/// XML namespace of our properties.
pub const NAMESPACE: &str = "https://heronite.app/ns/screenshots/1.0/";
/// `heronite:Compression` of a lossless conversion.
pub const LOSSLESS_WEBP: &str = "lossless-webp";

/// What the packet says about the conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// `heronite:Compression` (always [`LOSSLESS_WEBP`] for now).
    pub compression: String,
    /// File name of the PNG it replaced (`shot.png`).
    pub source_name: String,
    /// Size of that PNG in bytes.
    pub source_size: u64,
}

/// The XMP packet: our marker, the encoder, and the PNG's text chunks
/// (`key`, `value`) so metadata such as `Microsoft.GameDVR.Id` survives.
pub fn to_xmp(marker: &Marker, encoder: &str, png_text: &[(String, String)]) -> String {
    let mut text = String::new();
    if !png_text.is_empty() {
        text.push_str("\n   <heronite:PngText>\n    <rdf:Seq>\n");
        for (key, value) in png_text {
            text.push_str(&format!(
                "     <rdf:li rdf:parseType=\"Resource\"><heronite:Key>{}</heronite:Key><heronite:Value>{}</heronite:Value></rdf:li>\n",
                escape(key),
                escape(value)
            ));
        }
        text.push_str("    </rdf:Seq>\n   </heronite:PngText>\n  ");
    }
    format!(
        concat!(
            "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n",
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n",
            " <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n",
            "  <rdf:Description rdf:about=\"\" xmlns:heronite=\"{ns}\"\n",
            "   heronite:Compression=\"{compression}\"\n",
            "   heronite:SourceFormat=\"png\"\n",
            "   heronite:SourceName=\"{name}\"\n",
            "   heronite:SourceSize=\"{size}\"\n",
            "   heronite:Encoder=\"{encoder}\">{text}</rdf:Description>\n",
            " </rdf:RDF>\n",
            "</x:xmpmeta>\n",
            "<?xpacket end=\"r\"?>"
        ),
        ns = NAMESPACE,
        compression = escape(&marker.compression),
        name = escape(&marker.source_name),
        size = marker.source_size,
        encoder = escape(encoder),
        text = text,
    )
}

/// Our marker in an XMP packet, if it has one.
pub fn parse_xmp(xmp: &str) -> Option<Marker> {
    if !xmp.contains(NAMESPACE) {
        return None;
    }
    let compression = attribute(xmp, "heronite:Compression")?;
    let source_name = attribute(xmp, "heronite:SourceName")?;
    let source_size = attribute(xmp, "heronite:SourceSize")?.parse().ok()?;
    Some(Marker { compression, source_name, source_size })
}

/// Value of `name="..."` (we always write attributes with double quotes).
fn attribute(xml: &str, name: &str) -> Option<String> {
    let start = xml.find(&format!("{name}=\""))? + name.len() + 2;
    let len = xml[start..].find('"')?;
    Some(unescape(&xml[start..start + len]))
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 forbids most control characters, even escaped.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_with_awkward_names() {
        let marker = Marker {
            compression: LOSSLESS_WEBP.into(),
            source_name: "Baldur's Gate 3 \"<final>\" & co.png".into(),
            source_size: 5_321_234,
        };
        let text = vec![("Microsoft.GameDVR.Id".to_owned(), "fef4561c-61b4 <x>".to_owned())];
        let xmp = to_xmp(&marker, "libwebp 1.6.0", &text);
        assert!(xmp.contains("<heronite:Value>fef4561c-61b4 &lt;x&gt;</heronite:Value>"));
        assert_eq!(parse_xmp(&xmp), Some(marker));
    }

    #[test]
    fn foreign_packets_are_not_ours() {
        assert_eq!(parse_xmp("<x:xmpmeta><rdf:Description SourceSize=\"1\"/></x:xmpmeta>"), None);
        let other_size = to_xmp(&Marker { compression: LOSSLESS_WEBP.into(), source_name: "a.png".into(), source_size: 1 }, "x", &[])
            .replace("SourceSize=\"1\"", "SourceSize=\"one\"");
        assert_eq!(parse_xmp(&other_size), None);
    }
}
