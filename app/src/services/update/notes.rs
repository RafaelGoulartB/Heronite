//! Release notes as plain lines. The notes are git-cliff Markdown
//! (`cliff.toml`): `### ✨ Features` headings and `- **scope:** Message
//! ([abc1234](https://…/commit/…))` items. Only that subset is understood.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    Heading(String),
    Bullet(String),
    Text(String),
}

pub fn parse(markdown: &str) -> Vec<Note> {
    let mut out = Vec::new();
    for line in strip_comments(markdown).lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('#') {
            // The emoji of the group names are not in the app's fonts.
            let text = inline(line.trim_start_matches('#'));
            let text = text.trim_start_matches(|c: char| !c.is_alphanumeric()).to_string();
            if !text.is_empty() {
                out.push(Note::Heading(text));
            }
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            out.push(Note::Bullet(inline(item)));
        } else {
            out.push(Note::Text(inline(line)));
        }
    }
    out
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Links become their text, emphasis and code marks go, and a trailing
/// commit reference (`(abc1234)`) is dropped.
fn inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](").map(|i| open + i) else { break };
        let Some(end) = rest[close..].find(')').map(|i| close + i) else { break };
        out.push_str(&rest[..open]);
        out.push_str(&rest[open + 1..close]);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    let mut out = out.replace("**", "").replace('`', "");
    let trimmed = out.trim_end();
    if let Some(start) = trimmed.rfind(" (") {
        let inner = &trimmed[start + 2..];
        if let Some(hash) = inner.strip_suffix(')') {
            if (7..=40).contains(&hash.len()) && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                out.truncate(start);
            }
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::{parse, Note};

    #[test]
    fn reads_git_cliff_notes() {
        let md = "\n### <!-- 0 -->✨ Features\n\n- **collection:** Add a welcome screen ([013cdf0](https://github.com/x/y/commit/013cdf0abc))\n- Auto-fetch `legendary` ([c2d9a61](https://x))\n\n### 🐛 Bug fixes\n* Plain item\nSee the [README](https://x/readme).\n";
        assert_eq!(
            parse(md),
            vec![
                Note::Heading("Features".into()),
                Note::Bullet("collection: Add a welcome screen".into()),
                Note::Bullet("Auto-fetch legendary".into()),
                Note::Heading("Bug fixes".into()),
                Note::Bullet("Plain item".into()),
                Note::Text("See the README.".into()),
            ]
        );
        assert_eq!(parse(""), Vec::new());
        // A parenthesis that is not a commit stays.
        assert_eq!(parse("- Faster start (about 2x)"), vec![Note::Bullet("Faster start (about 2x)".into())]);
    }
}
