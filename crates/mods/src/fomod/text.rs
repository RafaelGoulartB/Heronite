//! Turns the bytes of an installer's XML into text `roxmltree` accepts.

use std::borrow::Cow;

/// Decodes UTF-16 (BOM, or told by its zero bytes), UTF-8 with or without a
/// BOM, and anything else as Windows-1252. The XML declaration's `encoding`
/// is not trusted: authors get it wrong and `roxmltree` ignores it.
pub(super) fn decode(bytes: &[u8]) -> String {
    let text = match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, false),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, true),
        [0xEF, 0xBB, 0xBF, rest @ ..] => utf8_or_1252(rest),
        _ => match sniff_utf16(bytes) {
            Some(big_endian) => utf16(bytes, big_endian),
            None => utf8_or_1252(bytes),
        },
    };
    trim_junk(&text)
}

/// A BOM-less UTF-16 file is ASCII text with a zero byte after (or before)
/// every character, so the zero bytes of the first characters tell the order.
fn sniff_utf16(bytes: &[u8]) -> Option<bool> {
    let head = &bytes[..bytes.len().min(64)];
    let pairs = head.len() / 2;
    if pairs < 2 {
        return None;
    }
    let zeros = |parity: usize| head.iter().enumerate().filter(|(i, b)| **b == 0 && i % 2 == parity).count();
    let (even, odd) = (zeros(0), zeros(1));
    if odd * 2 >= pairs && even == 0 {
        Some(false)
    } else if even * 2 >= pairs && odd == 0 {
        Some(true)
    } else {
        None
    }
}

fn utf16(bytes: &[u8], big_endian: bool) -> String {
    let units = bytes.as_chunks::<2>().0.iter().map(|c| if big_endian { u16::from_be_bytes(*c) } else { u16::from_le_bytes(*c) });
    char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect()
}

fn utf8_or_1252(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| windows_1252(b)).collect(),
    }
}

fn windows_1252(b: u8) -> char {
    // 0x80..=0x9F differ from Latin-1; the five undefined ones stay controls.
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•',
        '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9F => HIGH[(b - 0x80) as usize],
        _ => b as char,
    }
}

/// Drops a stray BOM character and whatever precedes the XML declaration (or
/// the root element when there is none).
fn trim_junk(text: &str) -> String {
    let text = text.trim_start_matches('\u{FEFF}');
    let start = text.find("<?xml").or_else(|| text.find('<')).unwrap_or(0);
    text[start..].to_string()
}

/// Escapes every `&` that does not start an entity: hand-written files write
/// "Fire & Ice" as is.
pub(super) fn escape_bare_ampersands(xml: &str) -> Cow<'_, str> {
    if !xml.contains('&') {
        return Cow::Borrowed(xml);
    }
    let mut out = String::with_capacity(xml.len() + 16);
    let mut rest = xml;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        out.push_str(if is_entity(&rest[i..]) { "&" } else { "&amp;" });
        rest = &rest[i + 1..];
    }
    out.push_str(rest);
    Cow::Owned(out)
}

fn is_entity(s: &str) -> bool {
    let Some(end) = s.find(';').filter(|&e| e <= 10) else { return false };
    let name = &s[1..end];
    match name {
        "amp" | "lt" | "gt" | "quot" | "apos" => true,
        _ => match name.strip_prefix('#') {
            Some(hex) if hex.starts_with(['x', 'X']) => hex.len() > 1 && hex[1..].chars().all(|c| c.is_ascii_hexdigit()),
            Some(dec) => !dec.is_empty() && dec.chars().all(|c| c.is_ascii_digit()),
            None => false,
        },
    }
}
