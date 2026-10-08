//! Steam store descriptions: a tiny HTML subset parser (what
//! `sanitizeSteamDescription` lets through and Steam actually uses) turned
//! into blocks the focus panel renders.

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading(Vec<Span>),
    Para(Vec<Span>),
    ListItem(Vec<Span>),
    Image { url: String, width: u32, height: u32 },
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let decoded = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            "trade" => Some('™'),
            "reg" => Some('®'),
            "copy" => Some('©'),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
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

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let key = format!("{name}=");
    let mut search = 0;
    while let Some(pos) = lower[search..].find(&key) {
        let at = search + pos;
        let ok_boundary = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let v = &tag[at + key.len()..];
        if ok_boundary {
            let value = if let Some(q) = v.strip_prefix('"') {
                q.split('"').next()
            } else if let Some(q) = v.strip_prefix('\'') {
                q.split('\'').next()
            } else {
                v.split(|c: char| c.is_whitespace() || c == '/' || c == '>').next()
            };
            return value.map(decode_entities);
        }
        search = at + key.len();
    }
    None
}

enum Ctx {
    Heading,
    Para,
    Item,
}

pub fn parse(html: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut spans: Vec<Span> = Vec::new();
    let mut ctx = Ctx::Para;
    let (mut bold, mut italic) = (0u32, 0u32);

    let flush = |blocks: &mut Vec<Block>, spans: &mut Vec<Span>, ctx: &Ctx| {
        // Collapse whitespace like HTML layout does.
        let mut merged: Vec<Span> = Vec::new();
        for s in spans.drain(..) {
            // `&nbsp;` is not collapsible: only ASCII whitespace collapses.
            let text: String =
                s.text.split(|c: char| c.is_ascii_whitespace()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
            let lead = s.text.starts_with(|c: char| c.is_ascii_whitespace());
            let trail = s.text.ends_with(|c: char| c.is_ascii_whitespace()) && !text.is_empty();
            let mut t = String::new();
            if lead && !merged.is_empty() {
                t.push(' ');
            }
            t.push_str(&text);
            if trail {
                t.push(' ');
            }
            if t.is_empty() {
                continue;
            }
            match merged.last_mut() {
                Some(last) if last.bold == s.bold && last.italic == s.italic => last.text.push_str(&t),
                _ => merged.push(Span { text: t, ..s }),
            }
        }
        if let Some(first) = merged.first_mut() {
            first.text = first.text.trim_start_matches(|c: char| c.is_ascii_whitespace()).to_string();
        }
        if let Some(last) = merged.last_mut() {
            last.text = last.text.trim_end_matches(|c: char| c.is_ascii_whitespace()).to_string();
        }
        merged.retain(|s| !s.text.is_empty());
        if merged.is_empty() {
            return;
        }
        blocks.push(match ctx {
            Ctx::Heading => Block::Heading(merged),
            Ctx::Para => Block::Para(merged),
            Ctx::Item => Block::ListItem(merged),
        });
    };

    let mut rest = html;
    while !rest.is_empty() {
        if let Some(lt) = rest.find('<') {
            if lt > 0 {
                spans.push(Span { text: decode_entities(&rest[..lt]), bold: bold > 0, italic: italic > 0 });
            }
            let Some(gt) = rest[lt..].find('>') else { break };
            let tag = &rest[lt + 1..lt + gt];
            rest = &rest[lt + gt + 1..];
            let closing = tag.starts_with('/');
            let name: String = tag
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            match (name.as_str(), closing) {
                ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => {
                    flush(&mut blocks, &mut spans, &ctx);
                    ctx = Ctx::Heading;
                }
                ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", true) => {
                    flush(&mut blocks, &mut spans, &ctx);
                    ctx = Ctx::Para;
                }
                ("li", false) => {
                    flush(&mut blocks, &mut spans, &ctx);
                    ctx = Ctx::Item;
                }
                ("li", true) | ("ul" | "ol", _) => {
                    flush(&mut blocks, &mut spans, &ctx);
                    ctx = Ctx::Para;
                }
                ("p" | "div", _) => {
                    if !matches!(ctx, Ctx::Item) {
                        flush(&mut blocks, &mut spans, &ctx);
                    }
                }
                ("br", _) => {
                    flush(&mut blocks, &mut spans, &ctx);
                }
                ("strong" | "b", false) => bold += 1,
                ("strong" | "b", true) => bold = bold.saturating_sub(1),
                ("em" | "i", false) => italic += 1,
                ("em" | "i", true) => italic = italic.saturating_sub(1),
                ("img", false) => {
                    if let Some(src) = attr(tag, "src") {
                        let url = if let Some(r) = src.strip_prefix("//") { format!("https://{r}") } else { src };
                        if url.starts_with("http") {
                            flush(&mut blocks, &mut spans, &ctx);
                            let width = attr(tag, "width").and_then(|v| v.parse().ok()).unwrap_or(0);
                            let height = attr(tag, "height").and_then(|v| v.parse().ok()).unwrap_or(0);
                            blocks.push(Block::Image { url, width, height });
                        }
                    }
                }
                _ => {}
            }
        } else {
            spans.push(Span { text: decode_entities(rest), bold: bold > 0, italic: italic > 0 });
            break;
        }
    }
    flush(&mut blocks, &mut spans, &ctx);
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_steam_markup() {
        let html = r#"<h2 class="bb_tag" >Become the Princess Knight</h2><p class="bb_paragraph" >As the <strong>lethal</strong> hunter&nbsp;Hornet &amp; co.</p><p></p><p><span><img class="bb_img" src="//cdn.x/a.avif?t=1" width=776 height=192 /></span></p><ul class="bb_ul"><li><p class="bb_paragraph" >Discover!</p></li><li>Two</li></ul>"#;
        let b = parse(html);
        assert_eq!(b[0], Block::Heading(vec![Span { text: "Become the Princess Knight".into(), bold: false, italic: false }]));
        match &b[1] {
            Block::Para(spans) => {
                assert_eq!(spans.len(), 3);
                assert!(spans[1].bold);
                assert_eq!(spans[2].text, " hunter\u{a0}Hornet & co.");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(b[2], Block::Image { url: "https://cdn.x/a.avif?t=1".into(), width: 776, height: 192 });
        assert!(matches!(&b[3], Block::ListItem(s) if s[0].text == "Discover!"));
        assert!(matches!(&b[4], Block::ListItem(s) if s[0].text == "Two"));
        assert_eq!(b.len(), 5);
    }
}
