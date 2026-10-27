//! Tag scanning for the XML the EA app writes (`installerdata.xml`). It
//! finds elements by name wherever they are; nothing here validates.

/// An element found by [`elements`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Element<'a> {
    /// The start tag after the name (` locale="en_US"`), trimmed.
    pub attrs: &'a str,
    /// What is between the tags, as written.
    pub inner: &'a str,
}

impl Element<'_> {
    /// The value of attribute `name`.
    pub fn attr(&self, name: &str) -> Option<String> {
        let lower = self.attrs.to_ascii_lowercase();
        let name = name.to_ascii_lowercase();
        let mut from = 0;
        while let Some(at) = lower[from..].find(&name).map(|i| i + from) {
            from = at + name.len();
            let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
            let rest = self.attrs[from..].trim_start();
            let Some(rest) = rest.strip_prefix('=').map(str::trim_start) else { continue };
            let quote = rest.chars().next().filter(|q| *q == '"' || *q == '\'');
            if let (true, Some(q)) = (before_ok, quote) {
                let value = &rest[1..];
                return value.find(q).map(|end| unescape(&value[..end]));
            }
        }
        None
    }

    /// The text inside, trimmed and unescaped; `None` when empty.
    pub fn text(&self) -> Option<String> {
        let inner = self.inner.trim();
        let inner = inner.strip_prefix("<![CDATA[").and_then(|c| c.strip_suffix("]]>")).unwrap_or(inner);
        let text = unescape(inner.trim());
        (!text.is_empty()).then_some(text)
    }
}

/// Every `<name ...>...</name>` of `xml`, in order (the name matched without
/// regard to case; `<contentIDs>` is not `<contentID>`).
pub(crate) fn elements<'a>(xml: &'a str, name: &str) -> Vec<Element<'a>> {
    // ASCII lowercasing keeps every byte offset.
    let lower = xml.to_ascii_lowercase();
    let (open, close) = (format!("<{}", name.to_ascii_lowercase()), format!("</{}>", name.to_ascii_lowercase()));
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = lower[at..].find(&open).map(|i| i + at) {
        let after = i + open.len();
        at = after;
        if !matches!(lower.as_bytes().get(after), Some(b'>' | b'/' | b' ' | b'\t' | b'\r' | b'\n')) {
            continue;
        }
        let Some(gt) = lower[after..].find('>').map(|g| g + after) else { break };
        let attrs = xml[after..gt].trim_end_matches('/').trim();
        if lower.as_bytes()[gt - 1] == b'/' {
            out.push(Element { attrs, inner: "" });
            at = gt + 1;
            continue;
        }
        let Some(end) = lower[gt + 1..].find(&close).map(|e| e + gt + 1) else { break };
        out.push(Element { attrs, inner: &xml[gt + 1..end] });
        at = end + close.len();
    }
    out
}

/// `&amp;`, `&lt;`, `&gt;`, `&quot;`, `&apos;` and numeric references.
pub(crate) fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let c = match &rest[1..end] {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                num => {
                    let code = match num.strip_prefix("#x").or_else(|| num.strip_prefix("#X")) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => num.strip_prefix('#')?.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((c, end + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_elements_by_whole_name_with_their_attributes() {
        let xml = r#"<game><contentIDs><contentID>71788</contentID><ContentId> 2 </ContentId></contentIDs>
            <gameTitles><gameTitle locale="de_DE">Spiel</gameTitle><gameTitle locale='en_US'>Fifa &amp; Co &#233;</gameTitle><gameTitle/></gameTitles></game>"#;
        let ids: Vec<_> = elements(xml, "contentID").iter().filter_map(Element::text).collect();
        assert_eq!(ids, ["71788", "2"]);
        let titles = elements(xml, "gameTitle");
        assert_eq!(titles.len(), 3);
        assert_eq!(titles[0].attr("locale").as_deref(), Some("de_DE"));
        assert_eq!(titles[1].attr("LOCALE").as_deref(), Some("en_US"));
        assert_eq!(titles[1].text().as_deref(), Some("Fifa & Co é"));
        assert_eq!((titles[2].attr("locale"), titles[2].text()), (None, None));
    }

    #[test]
    fn an_attribute_is_matched_whole() {
        let xml = r#"<localeInfo sublocale="x" locale = "en_US"><title><![CDATA[A <b> game]]></title></localeInfo>"#;
        let info = elements(xml, "localeInfo")[0];
        assert_eq!(info.attr("locale").as_deref(), Some("en_US"));
        assert_eq!(elements(info.inner, "title")[0].text().as_deref(), Some("A <b> game"));
    }

    #[test]
    fn cut_xml_gives_what_was_complete() {
        assert_eq!(elements("<a>1</a><a>2", "a").len(), 1);
        assert!(elements("<a", "a").is_empty());
        assert_eq!(unescape("&unknown; &#xZZ; a&b"), "&unknown; &#xZZ; a&b");
    }
}
