//! `[ids] pattern`: a product code an extension recognises in a game's title
//! or file name (`RJ01234567`), passed to its script as
//! `game.ids.<extension id>`. A hit the script found by that id is a sure
//! match, so the pattern is checked before anything runs: a loose one would
//! turn guesses into sure matches.
//!
//! The rules (see `docs/EXTENSIONS.md`):
//! - it is a word-bounded sequence: `\b` (or another word-boundary
//!   assertion) first and last, around everything else;
//! - it starts with a literal prefix of at least two ASCII letters (`RJ`,
//!   or a few such prefixes: `(?:RJ|BJ|VJ)`);
//! - what it matches is 4 to [`MAX_MATCH_LEN`] bytes long, so it has an
//!   upper bound (no `+`, `*` or `{n,}`).
//!
//! It is read in ASCII mode: `\b`, `\d` and `\w` are ASCII (so a code right
//! against Japanese text, `作品RJ123456`, is found), letter case is ignored
//! (`rj123456` is found, and passed as `RJ123456`), and what may match any
//! character (`.`, `[^…]`) is refused. In the texts, `_` also counts as a
//! space (`RJ123456_v1.02`).

use regex::Regex;
use regex_syntax::hir::literal::{ExtractKind, Extractor};
use regex_syntax::hir::{ErrorKind, Hir, HirKind, Look};

/// The longest text a pattern may match, in bytes.
pub const MAX_MATCH_LEN: usize = 64;
/// The shortest text a pattern may match, in bytes.
pub const MIN_MATCH_LEN: usize = 4;
/// The longest pattern, in characters.
pub const MAX_PATTERN_LEN: usize = 200;
/// Most alternatives a pattern's prefix may have (`RJ|BJ|VJ` is three).
const MAX_PREFIXES: usize = 16;

/// A checked id pattern.
#[derive(Debug, Clone)]
pub struct IdPattern {
    regex: Regex,
}

impl PartialEq for IdPattern {
    fn eq(&self, other: &Self) -> bool {
        self.regex.as_str() == other.regex.as_str()
    }
}

impl IdPattern {
    /// Checks `text` against the rules above; the error says which it breaks.
    pub fn parse(text: &str) -> Result<Self, String> {
        let shown = text.chars().take(60).collect::<String>();
        if text.is_empty() {
            return Err("ids.pattern is empty".into());
        }
        if text.chars().count() > MAX_PATTERN_LEN {
            return Err(format!("ids.pattern is longer than {MAX_PATTERN_LEN} characters"));
        }
        // As written (letter case kept), for the prefix rule; matched below
        // ignoring case.
        let hir = regex_syntax::ParserBuilder::new().unicode(false).build().parse(text).map_err(|e| match &e {
            regex_syntax::Error::Translate(t) if matches!(t.kind(), ErrorKind::InvalidUtf8) => {
                format!("ids.pattern \"{shown}\" may match any character (such as . or [^…]): name the characters, such as [0-9] or [A-Z]")
            }
            _ => format!("ids.pattern \"{shown}\" is not a regular expression: {}", first_line(&e.to_string())),
        })?;

        let body = word_bounded(&hir)
            .ok_or_else(|| format!("ids.pattern \"{shown}\" must start and end with a word boundary (\\b), around the whole pattern"))?;
        literal_prefix(body).map_err(|why| format!("ids.pattern \"{shown}\" {why}"))?;
        let props = hir.properties();
        match props.maximum_len() {
            Some(n) if n <= MAX_MATCH_LEN => {}
            _ => return Err(format!("ids.pattern \"{shown}\" may match more than {MAX_MATCH_LEN} bytes: give each repeat an upper bound")),
        }
        if props.minimum_len().unwrap_or(0) < MIN_MATCH_LEN {
            return Err(format!("ids.pattern \"{shown}\" may match fewer than {MIN_MATCH_LEN} bytes"));
        }
        let regex = regex::RegexBuilder::new(text)
            .unicode(false)
            .case_insensitive(true)
            .size_limit(1 << 20)
            .build()
            .map_err(|e| format!("ids.pattern \"{shown}\" is not a regular expression: {}", first_line(&e.to_string())))?;
        Ok(Self { regex })
    }

    /// The pattern as written.
    pub fn as_str(&self) -> &str {
        self.regex.as_str()
    }

    /// Checks `example` (`[ids] example`, a code the pattern is meant for):
    /// the pattern must find that code in it, the whole of it, as it would
    /// in a title. Gives the code as it is passed (upper case); the error
    /// says what was found instead.
    pub fn check_example(&self, example: &str) -> Result<String, String> {
        let example = example.trim();
        let pattern = self.as_str().chars().take(60).collect::<String>();
        if example.is_empty() {
            return Err("ids.example is empty".into());
        }
        if example.len() > MAX_MATCH_LEN {
            return Err(format!("ids.example is longer than {MAX_MATCH_LEN} bytes"));
        }
        let wanted = example.to_ascii_uppercase();
        match self.codes([example]).as_slice() {
            [code] if *code == wanted => Ok(wanted),
            [] => Err(format!("ids.pattern \"{pattern}\" does not find ids.example \"{example}\": fix the pattern or the example")),
            found => Err(format!(
                "ids.pattern \"{pattern}\" finds {} in ids.example \"{example}\", not the whole example: fix the pattern or the example",
                found.join(", ")
            )),
        }
    }

    /// The one code found in `texts` (a game's title and file name), in
    /// upper case. Two different codes are no sure id: none is returned.
    pub fn find<'a>(&self, texts: impl IntoIterator<Item = &'a str>) -> Option<String> {
        let mut codes = self.codes(texts);
        (codes.len() == 1).then(|| codes.remove(0))
    }

    /// Every different code found in `texts`, in upper case, in the order
    /// found. Each text is read as written and, when it has a `_`, with
    /// each `_` as a space: file names join words with it
    /// (`RJ123456_v1.02`).
    pub fn codes<'a>(&self, texts: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        let mut codes: Vec<String> = Vec::new();
        for text in texts {
            let spaced = text.contains('_').then(|| text.replace('_', " "));
            for t in std::iter::once(text).chain(spaced.as_deref()) {
                for m in self.regex.find_iter(t) {
                    let code = m.as_str().to_ascii_uppercase();
                    if !codes.contains(&code) {
                        codes.push(code);
                    }
                }
            }
        }
        codes
    }
}

fn first_line(text: &str) -> String {
    text.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("").trim().to_owned()
}

/// `h` without the groups around it.
fn peel(h: &Hir) -> &Hir {
    match h.kind() {
        HirKind::Capture(c) => peel(&c.sub),
        _ => h,
    }
}

fn is_look(h: &Hir, start: bool) -> bool {
    let HirKind::Look(look) = peel(h).kind() else { return false };
    match look {
        Look::WordAscii | Look::WordUnicode => true,
        Look::WordStartAscii | Look::WordStartUnicode | Look::WordStartHalfAscii | Look::WordStartHalfUnicode => start,
        Look::WordEndAscii | Look::WordEndUnicode | Look::WordEndHalfAscii | Look::WordEndHalfUnicode => !start,
        _ => false,
    }
}

/// What lies between the word boundaries that start and end the pattern,
/// when it has them.
fn word_bounded(hir: &Hir) -> Option<Hir> {
    let HirKind::Concat(items) = peel(hir).kind() else { return None };
    let [first, middle @ .., last] = items.as_slice() else { return None };
    (!middle.is_empty() && is_look(first, true) && is_look(last, false)).then(|| Hir::concat(middle.to_vec()))
}

/// Whether every text `body` matches starts with one of a few literal
/// prefixes of two ASCII letters or more.
fn literal_prefix(body: Hir) -> Result<(), String> {
    const NEEDS: &str = "must start with a literal prefix of two letters or more (such as RJ, or (?:RJ|BJ|VJ))";
    let mut extractor = Extractor::new();
    extractor.kind(ExtractKind::Prefix).limit_class(4).limit_total(64);
    let seq = extractor.extract(&body);
    let Some(literals) = seq.literals() else { return Err(NEEDS.into()) };
    if literals.is_empty() || literals.len() > MAX_PREFIXES {
        return Err(NEEDS.into());
    }
    let lettered = |bytes: &[u8]| bytes.len() >= 2 && bytes[..2].iter().all(u8::is_ascii_alphabetic);
    if literals.iter().all(|l| lettered(l.as_bytes())) {
        Ok(())
    } else {
        Err(NEEDS.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(pattern: &str) -> String {
        match IdPattern::parse(pattern) {
            Ok(_) => panic!("accepted {pattern}"),
            Err(e) => e,
        }
    }

    #[test]
    fn product_codes_are_accepted() {
        for p in [
            r"\bRJ[0-9]{6,8}\b",
            r"\b(?:RJ|BJ|VJ)[0-9]{6,8}\b",
            r"\b(RJ|BJ|VJ)\d{6,8}\b",
            r"(?-u:\b)[RBV]J[0-9]{6,8}(?-u:\b)",
            r"\b{start}CUSA[0-9]{5}\b{end}",
            r"\bSLUS-[0-9]{5}\b",
            r"(?i)\bCUSA[0-9]{5}\b",
            r"\bSK[0-9]{6}\b",
            r"\bRJ\w{6,8}\b",
        ] {
            assert!(IdPattern::parse(p).is_ok(), "{p}: {:?}", IdPattern::parse(p).err());
        }
    }

    #[test]
    fn loose_patterns_are_refused_with_the_rule_they_break() {
        assert!(refused(r"RJ[0-9]{6,8}").contains("word boundary"));
        assert!(refused(r"\bRJ[0-9]{6,8}").contains("word boundary"));
        assert!(refused(r"RJ[0-9]{6,8}\b").contains("word boundary"));
        assert!(refused(r"\bRJ[0-9]{6}\b|\bBJ[0-9]{6}\b").contains("word boundary"), "each branch bounded is not the whole");
        assert!(refused(r"\b[0-9]{6,8}\b").contains("literal prefix"));
        assert!(refused(r"\b[A-Z]{2}[0-9]{6}\b").contains("literal prefix"));
        assert!(refused(r"\bR[0-9]{6}\b").contains("literal prefix"));
        assert!(refused(r"\b(?:RJ|[0-9]{2})[0-9]{6}\b").contains("literal prefix"));
        assert!(refused(r"\b.{2}[0-9]{6}\b").contains("any character"));
        assert!(refused(r"\bRJ.{2,15}\b").contains("any character"), "a wildcard after the prefix");
        assert!(refused(r"\bRJ[^ ]{6,8}\b").contains("any character"));
        assert!(refused(r"\b12[0-9]{6}\b").contains("literal prefix"));
        assert!(refused(r"\bRJ[0-9]+\b").contains("upper bound"));
        assert!(refused(r"\bRJ[0-9]{6,}\b").contains("upper bound"));
        assert!(refused(r"\bRJ[0-9]{70}\b").contains("upper bound"));
        assert!(refused(r"\bRJ\b").contains("fewer than"));
        assert!(refused(r"\bRJ[0-9{6}\b").contains("not a regular expression"));
        assert!(refused("").contains("empty"));
        assert!(refused(&format!(r"\bRJ{}\b", "[0-9]?".repeat(60))).contains("longer than"));
    }

    #[test]
    fn one_code_in_the_title_or_file_name_is_found() {
        let p = IdPattern::parse(r"\b(?:RJ|BJ|VJ)[0-9]{6,8}\b").unwrap();
        assert_eq!(p.find(["My Game [RJ01234567]", "x"]), Some("RJ01234567".into()));
        assert_eq!(p.find(["My Game", "RJ123456 My Game"]), Some("RJ123456".into()));
        assert_eq!(p.find(["RJ123456", "RJ123456 v1.02"]), Some("RJ123456".into()), "the same code twice is one");
        assert_eq!(p.find(["RJ123456 RJ123456 Remake"]), Some("RJ123456".into()));
        assert_eq!(p.find(["RJ123456 and VJ654321"]), None, "two codes: no sure id");
        assert_eq!(p.find(["RJ123456", "RJ654321"]), None);
        assert_eq!(p.find(["XRJ123456", "RJ1234567890"]), None, "inside a longer word");
        assert_eq!(p.find(["My Game"]), None);
    }

    #[test]
    fn ascii_boundaries_any_case_and_underscores() {
        let p = IdPattern::parse(r"\b(?:RJ|BJ|VJ)[0-9]{6,8}\b").unwrap();
        assert_eq!(p.find(["rj123456 my game"]), Some("RJ123456".into()), "any case, passed in upper case");
        assert_eq!(p.find(["RJ123456", "rj123456"]), Some("RJ123456".into()), "the same code in two cases is one");
        assert_eq!(p.find(["作品RJ123456"]), Some("RJ123456".into()), "right against Japanese text");
        assert_eq!(p.find(["【RJ123456】"]), Some("RJ123456".into()));
        assert_eq!(p.find(["RJ123456_GameName"]), Some("RJ123456".into()), "_ joins words in a file name");
        assert_eq!(p.find(["RJ01234567_v1.02"]), Some("RJ01234567".into()));
        assert_eq!(p.find(["My_Game_rj123456"]), Some("RJ123456".into()));
        assert_eq!(p.find(["RJ123456_VJ654321"]), None, "two codes joined by _");
        assert_eq!(p.find(["RJ123456Z"]), None, "a letter still joins");
        let serial = IdPattern::parse(r"\bSLUS_[0-9]{3}\.[0-9]{2}\b").unwrap();
        assert_eq!(serial.find(["SLUS_012.34"]), Some("SLUS_012.34".into()), "a pattern with _ still matches it");
    }

    #[test]
    fn an_example_must_be_found_whole() {
        let p = IdPattern::parse(r"\b(?:RJ|BJ|VJ)[0-9]{6,8}\b").unwrap();
        assert_eq!(p.check_example("RJ01234567"), Ok("RJ01234567".into()));
        assert_eq!(p.check_example(" rj123456 "), Ok("RJ123456".into()), "any case, passed in upper case");
        assert!(p.check_example("SK123456").unwrap_err().contains("does not find ids.example \"SK123456\""));
        assert!(p.check_example("RJ123").unwrap_err().contains("does not find"), "too short for the pattern");
        assert!(p.check_example("RJ123456 VJ654321").unwrap_err().contains("not the whole example"), "two codes");
        assert!(p.check_example("My Game RJ123456").unwrap_err().contains("finds RJ123456"), "more than the code");
        assert!(p.check_example("").unwrap_err().contains("empty"));
        assert!(p.check_example(&"RJ".repeat(40)).unwrap_err().contains("longer than"));
        let serial = IdPattern::parse(r"\bSLUS_[0-9]{3}\.[0-9]{2}\b").unwrap();
        assert_eq!(serial.check_example("slus_012.34"), Ok("SLUS_012.34".into()));
    }
}
