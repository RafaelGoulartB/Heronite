//! Steam ROM Manager-compatible glob matching for ROM folders.
//!
//! Supports standard glob wildcards (`*`, `?`, `**`, `{a,b}`), bash/minimatch
//! extglobs (`@(a|b)`, `!(a|b)`, `+(a|b)`, `?(a|b)`, `*(a|b)`), character
//! classes (`[0-9]`, `[!a-z]`, `[abc]`, `[[]` / `[]]`), and the `${title}`
//! capture placeholder.
//!
//! Standard minimatch character classes are the default for `[...]`. As a
//! convenience for common ROM filenames, a non-negated bracket expression with
//! no `-` range is treated as literal text only when its content is `[!]`,
//! empty `[]`, 3+ all-uppercase ASCII letters (e.g. `[BASE]`, `[UPD]`, `[DLC]`,
//! `[USA]`), or contains a space or dot (e.g. `[UPD v1.2]`). Escape literal
//! brackets explicitly with `[[]` / `[]]` or `\[` / `\]` for anything else.

use std::collections::{HashMap, HashSet};

use heronite_i18n::{n_, tr};

const MAX_BRACE_EXPANSIONS: usize = 64;

/// A suggested glob pattern for a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub pattern: String,
}

/// What a [`RomGlob`] captured from a matching relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobMatch {
    /// Raw text captured by `${title}`, preserving its original case.
    pub title: Option<String>,
    /// True when `${title}` matched a directory component rather than the file name.
    pub title_from_dir: bool,
}

/// A compiled ROM glob pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomGlob {
    source: String,
    alternatives: Vec<Vec<Segment>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    GlobStar,
    Pattern(Vec<Atom>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtKind {
    /// `@(a|b)`: matches exactly one alternative.
    At,
    /// `!(a|b)`: matches any substring that none of the alternatives match.
    Not,
    /// `+(a|b)`: matches one or more concatenations of alternatives.
    Plus,
    /// `?(a|b)`: matches empty or one alternative.
    Question,
    /// `*(a|b)`: matches zero or more concatenations of alternatives.
    Star,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClassItem {
    Single(char),
    Range(char, char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Atom {
    Literal(Vec<char>),
    AnyChar,
    Star,
    Title,
    CharClass { negated: bool, items: Vec<ClassItem> },
    ExtGlob { kind: ExtKind, alts: Vec<Vec<Atom>> },
}

impl RomGlob {
    /// Parses a Steam ROM Manager-style glob pattern.
    pub fn parse(pattern: &str) -> Result<Self, String> {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            return Err(tr!("Glob pattern is empty.").into());
        }
        let expanded = expand_braces(trimmed)?;
        let mut alternatives = Vec::with_capacity(expanded.len());
        for raw in expanded {
            let segments = parse_path_segments(&raw)?;
            if segments.is_empty() {
                return Err(tr!("Glob pattern matches no path.").into());
            }
            alternatives.push(segments);
        }
        Ok(Self { source: trimmed.to_string(), alternatives })
    }

    /// Original trimmed pattern text.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// True when the pattern can match files inside subdirectories.
    pub fn needs_subfolders(&self) -> bool {
        self.alternatives.iter().any(|segs| segs.len() > 1 || segs.iter().any(|s| matches!(s, Segment::GlobStar)))
    }

    /// Matches a path relative to the ROM root (both `/` and `\` accepted).
    pub fn matches(&self, rel_path: &str) -> Option<GlobMatch> {
        let normalized = rel_path.replace('\\', "/");
        let trimmed = normalized.trim_start_matches("./").trim_start_matches('/');
        if trimmed.is_empty() {
            return None;
        }
        let parts: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return None;
        }
        let parts_chars: Vec<Vec<char>> = parts.iter().map(|p| p.chars().collect()).collect();
        let parts_lower: Vec<Vec<char>> = parts.iter().map(|p| p.chars().flat_map(|c| c.to_lowercase()).collect()).collect();
        // Keep exact char-length alignment when lowercasing is 1-to-1; fall back to
        // ASCII lowercase per char when Unicode casefolding expands a char so slice
        // indices stay valid on the original string.
        let lowered: Vec<Vec<char>> = parts_chars
            .iter()
            .zip(parts_lower)
            .map(|(orig, low)| if low.len() == orig.len() { low } else { orig.iter().map(|c| c.to_ascii_lowercase()).collect() })
            .collect();

        for segments in &self.alternatives {
            let mut seg_failed = HashSet::new();
            if let Some(m) = match_segments(segments, 0, &parts_chars, &lowered, 0, &mut seg_failed) {
                return Some(m);
            }
        }
        None
    }
}

/// Validates a glob pattern, returning `Ok(())` or an error message.
pub fn validate(pattern: &str) -> Result<(), String> {
    RomGlob::parse(pattern).map(|_| ())
}

/// Preset patterns offered in the UI for a profile's platform and extensions.
pub fn presets_for_profile(platform_id: Option<&str>, extensions: &[String]) -> Vec<GlobPreset> {
    let mut presets = Vec::new();
    if platform_id == Some("nintendo_switch") {
        presets.push(GlobPreset {
            id: "switch-subfolder-base",
            label: n_!("Switch subfolders ([BASE] NSP / XCI)"),
            pattern: "${title}/@(*[BASE]*.nsp|*.xci)".into(),
        });
        presets.push(GlobPreset {
            id: "switch-skip-updates",
            label: n_!("Switch (skip [UPD] / [DLC])"),
            pattern: "**/@(!(*[[]UPD*|*[[]DLC*).nsp|*.xci)".into(),
        });
    }
    let clean: Vec<String> = extensions.iter().map(|e| e.trim().trim_start_matches('.').to_lowercase()).filter(|e| !e.is_empty()).collect();
    let ext_glob = match clean.as_slice() {
        [] => "*".to_string(),
        [one] => format!("*.{one}"),
        many => format!("*.@({})", many.join("|")),
    };
    presets.push(GlobPreset { id: "subfolder-title", label: n_!("Subfolder name as title"), pattern: format!("${{title}}/{ext_glob}") });
    presets.push(GlobPreset {
        id: "recursive-title",
        label: n_!("Any subfolder"),
        pattern: format!(
            "**/${{title}}.{ext}",
            ext = match clean.as_slice() {
                [] => "*".to_string(),
                [one] => one.clone(),
                many => format!("@({})", many.join("|")),
            }
        ),
    });
    presets
}

/// Returns true when `c` is a glob metacharacter that `\` escapes as literal.
fn is_escapable(c: char) -> bool {
    matches!(c, '*' | '?' | '[' | ']' | '(' | ')' | '{' | '}' | '!' | '+' | '@' | '|' | '\\' | '$')
}

/// Expands brace expressions `{a,b}` (including nested ones). A `{...}` without
/// an unnested comma is kept literally (so `${title}` is never touched).
fn expand_braces(input: &str) -> Result<Vec<String>, String> {
    let mut results = vec![input.to_string()];
    loop {
        let mut expanded_any = false;
        let mut next = Vec::with_capacity(results.len());
        for s in results {
            match find_brace_group(&s)? {
                Some((start, end, parts)) => {
                    expanded_any = true;
                    let prefix = &s[..start];
                    let suffix = &s[end + 1..];
                    for part in parts {
                        if next.len() >= MAX_BRACE_EXPANSIONS {
                            return Err(tr!("Glob pattern has too many brace expansions.").into());
                        }
                        next.push(format!("{prefix}{part}{suffix}"));
                    }
                }
                None => next.push(s),
            }
        }
        results = next;
        if !expanded_any {
            break;
        }
    }
    Ok(results)
}

/// Finds the first `{a,b}` brace group containing at least one top-level comma.
fn find_brace_group(s: &str) -> Result<Option<(usize, usize, Vec<String>)>, String> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            let next = s[i + 1..].chars().next().unwrap();
            if is_escapable(next) {
                i += 1 + next.len_utf8();
                continue;
            }
        }
        // Skip `${...}` placeholders so they are never treated as brace groups.
        if bytes[i] == b'$' && bytes.get(i + 1) == Some(&b'{') {
            if let Some(rel) = s[i + 2..].find('}') {
                i = i + 2 + rel + 1;
                continue;
            }
            return Err(tr!("Unclosed placeholder in glob pattern.").into());
        }
        if bytes[i] == b'{' {
            let start = i;
            let mut depth = 1usize;
            let mut paren_depth = 0usize;
            let mut in_bracket = false;
            let mut part_start = i + 1;
            let mut parts = Vec::new();
            i += 1;
            while i < bytes.len() && depth > 0 {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    let next = s[i + 1..].chars().next().unwrap();
                    if is_escapable(next) {
                        i += 1 + next.len_utf8();
                        continue;
                    }
                }
                match bytes[i] {
                    b'[' if !in_bracket => {
                        in_bracket = true;
                        i += 1;
                    }
                    b']' if in_bracket => {
                        in_bracket = false;
                        i += 1;
                    }
                    b'(' if !in_bracket => {
                        paren_depth += 1;
                        i += 1;
                    }
                    b')' if !in_bracket && paren_depth > 0 => {
                        paren_depth -= 1;
                        i += 1;
                    }
                    b'$' if !in_bracket && bytes.get(i + 1) == Some(&b'{') => {
                        if let Some(rel) = s[i + 2..].find('}') {
                            i = i + 2 + rel + 1;
                        } else {
                            return Err(tr!("Unclosed placeholder in glob pattern.").into());
                        }
                    }
                    b'{' if !in_bracket => {
                        depth += 1;
                        i += 1;
                    }
                    b'}' if !in_bracket => {
                        depth -= 1;
                        if depth == 0 {
                            parts.push(s[part_start..i].to_string());
                            if parts.len() > 1 {
                                return Ok(Some((start, i, parts)));
                            }
                            break;
                        }
                        i += 1;
                    }
                    b',' if !in_bracket && depth == 1 && paren_depth == 0 => {
                        parts.push(s[part_start..i].to_string());
                        part_start = i + 1;
                        i += 1;
                    }
                    _ => {
                        let ch = s[i..].chars().next().unwrap();
                        i += ch.len_utf8();
                    }
                }
            }
            if depth > 0 {
                return Err(tr!("Unclosed brace in glob pattern.").into());
            }
            i += 1;
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        i += ch.len_utf8();
    }
    Ok(None)
}

/// Splits a brace-expanded pattern into path segments, respecting `(...)` and `[...]`.
fn parse_path_segments(raw: &str) -> Result<Vec<Segment>, String> {
    let mut raw_segments = Vec::new();
    let mut cur = String::new();
    let mut paren_depth = 0usize;
    let mut in_bracket = false;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                if is_escapable(next) {
                    cur.push(c);
                    cur.push(chars.next().unwrap());
                    continue;
                }
            }
            // Windows path separator when not escaping a glob metacharacter.
            if paren_depth == 0 && !in_bracket {
                if !cur.is_empty() && cur != "." {
                    raw_segments.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                continue;
            }
            cur.push('/');
            continue;
        }
        match c {
            '/' if paren_depth == 0 && !in_bracket => {
                if !cur.is_empty() && cur != "." {
                    raw_segments.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
            }
            '[' if !in_bracket => {
                in_bracket = true;
                cur.push(c);
            }
            ']' if in_bracket => {
                in_bracket = false;
                cur.push(c);
            }
            '(' if !in_bracket => {
                paren_depth += 1;
                cur.push(c);
            }
            ')' if !in_bracket && paren_depth > 0 => {
                paren_depth -= 1;
                cur.push(c);
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() && cur != "." {
        raw_segments.push(cur);
    }
    let mut segments = Vec::with_capacity(raw_segments.len());
    for seg in raw_segments {
        if seg == "**" {
            // Collapse consecutive `**` segments.
            if !matches!(segments.last(), Some(Segment::GlobStar)) {
                segments.push(Segment::GlobStar);
            }
        } else {
            let chars: Vec<char> = seg.chars().collect();
            let (atoms, pos) = parse_atoms(&chars, 0, false)?;
            if pos != chars.len() {
                return Err(tr!("Unexpected character '{ch}' in glob pattern.", ch = chars[pos]));
            }
            segments.push(Segment::Pattern(atoms));
        }
    }
    Ok(segments)
}

fn push_literal(atoms: &mut Vec<Atom>, ch: char) {
    let lower = ch.to_lowercase().next().unwrap_or(ch);
    if let Some(Atom::Literal(buf)) = atoms.last_mut() {
        buf.push(lower);
    } else {
        atoms.push(Atom::Literal(vec![lower]));
    }
}

/// Parses a sequence of segment atoms until end-of-slice or (when `in_extglob`)
/// a top-level `|` / `)`. Plain `(...)` inside an extglob are kept literally.
fn parse_atoms(chars: &[char], mut i: usize, in_extglob: bool) -> Result<(Vec<Atom>, usize), String> {
    let mut atoms = Vec::new();
    let mut plain_parens = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if in_extglob && plain_parens == 0 && (c == '|' || c == ')') {
            break;
        }
        if c == '\\' {
            if i + 1 < chars.len() {
                push_literal(&mut atoms, chars[i + 1]);
                i += 2;
                continue;
            }
            push_literal(&mut atoms, '\\');
            i += 1;
            continue;
        }
        // `${title}` placeholder (case-insensitive).
        if c == '$' && i + 1 < chars.len() && chars[i + 1] == '{' {
            let Some(rel_end) = chars[i + 2..].iter().position(|&ch| ch == '}') else {
                return Err(tr!("Unclosed placeholder in glob pattern.").into());
            };
            let name: String = chars[i + 2..i + 2 + rel_end].iter().collect();
            if name.eq_ignore_ascii_case("title") {
                atoms.push(Atom::Title);
            } else {
                let raw = format!("${{{name}}}");
                return Err(tr!("Unknown glob placeholder '{name}'. Use {expected}.", name = raw.as_str(), expected = "${title}"));
            }
            i = i + 2 + rel_end + 1;
            continue;
        }
        // Extglob prefix: @( | !( | +( | ?( | *(
        if let Some(kind) = extglob_kind(c) {
            if i + 1 < chars.len() && chars[i + 1] == '(' {
                let (alts, next_i) = parse_extglob(chars, i + 2)?;
                atoms.push(Atom::ExtGlob { kind, alts });
                i = next_i;
                continue;
            }
        }
        match c {
            '(' if in_extglob => {
                plain_parens += 1;
                push_literal(&mut atoms, c);
                i += 1;
            }
            ')' if in_extglob && plain_parens > 0 => {
                plain_parens -= 1;
                push_literal(&mut atoms, c);
                i += 1;
            }
            '*' => {
                // Collapse consecutive `*` atoms inside one segment.
                if !matches!(atoms.last(), Some(Atom::Star)) {
                    atoms.push(Atom::Star);
                }
                i += 1;
            }
            '?' => {
                atoms.push(Atom::AnyChar);
                i += 1;
            }
            '[' => {
                let (atom, next_i) = parse_bracket(chars, i)?;
                match atom {
                    Atom::Literal(lit) => {
                        for ch in lit {
                            push_literal(&mut atoms, ch);
                        }
                    }
                    other => atoms.push(other),
                }
                i = next_i;
            }
            _ => {
                push_literal(&mut atoms, c);
                i += 1;
            }
        }
    }
    Ok((atoms, i))
}

fn extglob_kind(c: char) -> Option<ExtKind> {
    match c {
        '@' => Some(ExtKind::At),
        '!' => Some(ExtKind::Not),
        '+' => Some(ExtKind::Plus),
        '?' => Some(ExtKind::Question),
        '*' => Some(ExtKind::Star),
        _ => None,
    }
}

fn parse_extglob(chars: &[char], mut i: usize) -> Result<(Vec<Vec<Atom>>, usize), String> {
    let mut alts = Vec::new();
    loop {
        let (alt, next_i) = parse_atoms(chars, i, true)?;
        alts.push(alt);
        if next_i >= chars.len() {
            return Err(tr!("Unclosed '(' in extglob pattern.").into());
        }
        match chars[next_i] {
            '|' => i = next_i + 1,
            ')' => return Ok((alts, next_i + 1)),
            other => return Err(tr!("Unexpected character '{ch}' in extglob pattern.", ch = other)),
        }
    }
}

/// Parses `[...]`, keeping standard minimatch character classes (`[0-9]`,
/// `[!a-z]`, `[nvz]`, `[abc]`) as the default.
///
/// Minimatch literal escapes (`[[]`, `[]]`) always produce literal `[` / `]`.
/// A non-negated bracket with no `-` range is treated as a literal ROM tag only
/// when it is empty (`[]`), GoodTools verified (`[!]`), 3+ all-uppercase ASCII
/// letters (`[BASE]`, `[UPD]`, `[DLC]`, `[USA]`), or contains a space or dot
/// (`[UPD v1.2]`).
fn parse_bracket(chars: &[char], start: usize) -> Result<(Atom, usize), String> {
    // Minimatch bracket escapes: `[[]` -> `[` and `[]]` -> `]`.
    if start + 2 < chars.len() && chars[start + 1] == '[' && chars[start + 2] == ']' {
        return Ok((Atom::Literal(vec!['[']), start + 3));
    }
    if start + 2 < chars.len() && chars[start + 1] == ']' && chars[start + 2] == ']' {
        return Ok((Atom::Literal(vec![']']), start + 3));
    }
    let Some(rel_end) = chars[start + 1..].iter().position(|&c| c == ']') else {
        return Err(tr!("Unclosed '[' in glob pattern.").into());
    };
    let end = start + 1 + rel_end;
    let inner = &chars[start + 1..end];
    if inner.is_empty() {
        return Ok((Atom::Literal(vec!['[', ']']), end + 1));
    }
    // GoodTools verified dump tag `[!]`.
    if inner == ['!'] {
        return Ok((Atom::Literal(vec!['[', '!', ']']), end + 1));
    }
    let negated = inner.len() >= 2 && matches!(inner[0], '!' | '^');
    let body = if negated { &inner[1..] } else { inner };
    let has_range = body.len() >= 3 && body[1..body.len() - 1].contains(&'-');
    let is_rom_tag = !negated
        && !has_range
        && ((inner.len() >= 3 && inner.iter().all(|c| c.is_ascii_uppercase())) || inner.iter().any(|c| *c == ' ' || *c == '.'));

    if is_rom_tag {
        let mut lit = Vec::with_capacity(inner.len() + 2);
        lit.push('[');
        for &c in inner {
            lit.push(c.to_lowercase().next().unwrap_or(c));
        }
        lit.push(']');
        return Ok((Atom::Literal(lit), end + 1));
    }

    let mut items = Vec::new();
    let mut k = 0;
    while k < body.len() {
        if k + 2 < body.len() && body[k + 1] == '-' {
            let a = body[k].to_lowercase().next().unwrap_or(body[k]);
            let b = body[k + 2].to_lowercase().next().unwrap_or(body[k + 2]);
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            items.push(ClassItem::Range(lo, hi));
            k += 3;
        } else {
            let ch = body[k].to_lowercase().next().unwrap_or(body[k]);
            items.push(ClassItem::Single(ch));
            k += 1;
        }
    }
    Ok((Atom::CharClass { negated, items }, end + 1))
}

type AltMemo = HashMap<(usize, usize, usize), Option<Option<String>>>;

fn match_segments(
    segments: &[Segment],
    si: usize,
    orig_parts: &[Vec<char>],
    lower_parts: &[Vec<char>],
    pi: usize,
    seg_failed: &mut HashSet<(usize, usize)>,
) -> Option<GlobMatch> {
    if si == segments.len() {
        return (pi == orig_parts.len()).then_some(GlobMatch { title: None, title_from_dir: false });
    }
    if seg_failed.contains(&(si, pi)) {
        return None;
    }
    let res = match &segments[si] {
        Segment::GlobStar => {
            // `**` matches 0 or more path segments.
            let mut found = None;
            for consume in 0..=(orig_parts.len() - pi) {
                if let Some(m) = match_segments(segments, si + 1, orig_parts, lower_parts, pi + consume, seg_failed) {
                    found = Some(m);
                    break;
                }
            }
            found
        }
        Segment::Pattern(atoms) => {
            if pi >= orig_parts.len() {
                None
            } else {
                let mut failed = HashSet::new();
                let mut alt_memo = HashMap::new();
                match match_atoms(atoms, 0, &orig_parts[pi], &lower_parts[pi], 0, orig_parts[pi].len(), &mut failed, &mut alt_memo) {
                    Some(title) => match match_segments(segments, si + 1, orig_parts, lower_parts, pi + 1, seg_failed) {
                        Some(rest) => {
                            let is_dir = pi + 1 < orig_parts.len();
                            Some(match (title, rest.title) {
                                (Some(t), _) => GlobMatch { title: Some(t), title_from_dir: is_dir },
                                (None, Some(t)) => GlobMatch { title: Some(t), title_from_dir: rest.title_from_dir },
                                (None, None) => GlobMatch { title: None, title_from_dir: false },
                            })
                        }
                        None => None,
                    },
                    None => None,
                }
            }
        }
    };
    if res.is_none() {
        seg_failed.insert((si, pi));
    }
    res
}

/// Matches `atoms[ai..]` against `orig[ci..limit]` (comparing against `lower[ci..limit]`).
/// Returns `Some(captured_title)` on match, `None` on failure.
#[allow(clippy::too_many_arguments)]
fn match_atoms(
    atoms: &[Atom],
    ai: usize,
    orig: &[char],
    lower: &[char],
    ci: usize,
    limit: usize,
    failed: &mut HashSet<(usize, usize)>,
    alt_memo: &mut AltMemo,
) -> Option<Option<String>> {
    if ai == atoms.len() {
        return (ci == limit).then_some(None);
    }
    if failed.contains(&(ai, ci)) {
        return None;
    }
    let result = match &atoms[ai] {
        Atom::Literal(lit) => {
            if ci + lit.len() <= limit && lower[ci..ci + lit.len()] == lit[..] {
                match_atoms(atoms, ai + 1, orig, lower, ci + lit.len(), limit, failed, alt_memo)
            } else {
                None
            }
        }
        Atom::AnyChar => {
            if ci < limit {
                match_atoms(atoms, ai + 1, orig, lower, ci + 1, limit, failed, alt_memo)
            } else {
                None
            }
        }
        Atom::CharClass { negated, items } => {
            if ci < limit && class_matches(lower[ci], *negated, items) {
                match_atoms(atoms, ai + 1, orig, lower, ci + 1, limit, failed, alt_memo)
            } else {
                None
            }
        }
        Atom::Star => {
            let mut found = None;
            for end in ci..=limit {
                if let Some(t) = match_atoms(atoms, ai + 1, orig, lower, end, limit, failed, alt_memo) {
                    found = Some(t);
                    break;
                }
            }
            found
        }
        Atom::Title => {
            // Fast path when `${title}` is the last (or only) atom in the segment.
            if ai + 1 == atoms.len() {
                if ci < limit {
                    let captured: String = orig[ci..limit].iter().collect();
                    Some(Some(captured))
                } else {
                    None
                }
            } else {
                // Non-greedy so `${title} (*).iso` captures the name before tags.
                let mut found = None;
                for end in (ci + 1)..=limit {
                    if let Some(rest_title) = match_atoms(atoms, ai + 1, orig, lower, end, limit, failed, alt_memo) {
                        let captured: String = orig[ci..end].iter().collect();
                        found = Some(Some(captured).or(rest_title));
                        break;
                    }
                }
                found
            }
        }
        Atom::ExtGlob { kind, alts } => match_extglob(*kind, alts, atoms, ai, orig, lower, ci, limit, failed, alt_memo),
    };
    if result.is_none() {
        failed.insert((ai, ci));
    }
    result
}

fn class_matches(ch: char, negated: bool, items: &[ClassItem]) -> bool {
    let hit = items.iter().any(|item| match *item {
        ClassItem::Single(c) => ch == c,
        ClassItem::Range(lo, hi) => ch >= lo && ch <= hi,
    });
    if negated {
        !hit
    } else {
        hit
    }
}

#[allow(clippy::too_many_arguments)]
fn match_extglob(
    kind: ExtKind,
    alts: &[Vec<Atom>],
    atoms: &[Atom],
    ai: usize,
    orig: &[char],
    lower: &[char],
    ci: usize,
    limit: usize,
    failed: &mut HashSet<(usize, usize)>,
    alt_memo: &mut AltMemo,
) -> Option<Option<String>> {
    match kind {
        ExtKind::At => {
            for end in ci..=limit {
                if let Some(rest_title) = match_atoms(atoms, ai + 1, orig, lower, end, limit, failed, alt_memo) {
                    if let Some(inner_title) = match_any_alt(alts, orig, lower, ci, end, alt_memo) {
                        return Some(inner_title.or(rest_title));
                    }
                }
            }
            None
        }
        ExtKind::Question => {
            if let Some(rest_title) = match_atoms(atoms, ai + 1, orig, lower, ci, limit, failed, alt_memo) {
                return Some(rest_title);
            }
            for end in (ci + 1)..=limit {
                if let Some(rest_title) = match_atoms(atoms, ai + 1, orig, lower, end, limit, failed, alt_memo) {
                    if let Some(inner_title) = match_any_alt(alts, orig, lower, ci, end, alt_memo) {
                        return Some(inner_title.or(rest_title));
                    }
                }
            }
            None
        }
        ExtKind::Not => {
            for end in ci..=limit {
                if let Some(rest_title) = match_atoms(atoms, ai + 1, orig, lower, end, limit, failed, alt_memo) {
                    if match_any_alt(alts, orig, lower, ci, end, alt_memo).is_none() {
                        return Some(rest_title);
                    }
                }
            }
            None
        }
        ExtKind::Star => {
            let mut visited = HashSet::new();
            match_repeat(alts, atoms, ai, orig, lower, ci, limit, 0, &mut visited, failed, alt_memo)
        }
        ExtKind::Plus => {
            let mut visited = HashSet::new();
            match_repeat(alts, atoms, ai, orig, lower, ci, limit, 1, &mut visited, failed, alt_memo)
        }
    }
}

fn match_any_alt(
    alts: &[Vec<Atom>],
    orig: &[char],
    lower: &[char],
    start: usize,
    end: usize,
    alt_memo: &mut AltMemo,
) -> Option<Option<String>> {
    let key = (alts.as_ptr() as usize, start, end);
    if let Some(cached) = alt_memo.get(&key) {
        return cached.clone();
    }
    let mut found = None;
    for alt in alts {
        let mut sub_failed = HashSet::new();
        if let Some(t) = match_atoms(alt, 0, orig, lower, start, end, &mut sub_failed, alt_memo) {
            found = Some(t);
            break;
        }
    }
    alt_memo.insert(key, found.clone());
    found
}

#[allow(clippy::too_many_arguments)]
fn match_repeat(
    alts: &[Vec<Atom>],
    atoms: &[Atom],
    ai: usize,
    orig: &[char],
    lower: &[char],
    pos: usize,
    limit: usize,
    min_reps: usize,
    visited: &mut HashSet<(usize, usize)>,
    failed: &mut HashSet<(usize, usize)>,
    alt_memo: &mut AltMemo,
) -> Option<Option<String>> {
    if !visited.insert((pos, min_reps)) {
        return None;
    }
    if min_reps == 0 {
        if let Some(rest_title) = match_atoms(atoms, ai + 1, orig, lower, pos, limit, failed, alt_memo) {
            return Some(rest_title);
        }
    } else if match_any_alt(alts, orig, lower, pos, pos, alt_memo).is_some() {
        // An empty alternative satisfies `+(...)` once without advancing.
        if let Some(rest_title) = match_atoms(atoms, ai + 1, orig, lower, pos, limit, failed, alt_memo) {
            return Some(rest_title);
        }
    }
    for end in (pos + 1)..=limit {
        if let Some(inner_title) = match_any_alt(alts, orig, lower, pos, end, alt_memo) {
            if let Some(rest_title) = match_repeat(alts, atoms, ai, orig, lower, end, limit, 0, visited, failed, alt_memo) {
                return Some(inner_title.or(rest_title));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_subfolder_base_or_xci_matches_and_excludes_updates_and_dlcs() {
        let g = RomGlob::parse("${title}/@(*[BASE]*.nsp|*.xci)").unwrap();
        assert!(g.needs_subfolders());

        let m = g.matches("Zelda Tears of the Kingdom/0100F2C0115B6000 [BASE].nsp").unwrap();
        assert_eq!(m.title.as_deref(), Some("Zelda Tears of the Kingdom"));
        assert!(m.title_from_dir);

        let m2 = g.matches("Super Mario Wonder/Super Mario Wonder.XCI").unwrap();
        assert_eq!(m2.title.as_deref(), Some("Super Mario Wonder"));
        assert!(m2.title_from_dir);

        assert!(g.matches("Zelda Tears of the Kingdom/0100F2C0115B6800 [UPD v1.2.1].nsp").is_none());
        assert!(g.matches("Zelda Tears of the Kingdom/0100F2C0115B6001 [DLC].nsp").is_none());
        assert!(g.matches("LooseGame [BASE].nsp").is_none());
        assert!(g.matches("A/B/TooDeep [BASE].nsp").is_none());
    }

    #[test]
    fn minimatch_escaped_brackets_and_backslash_brackets_work() {
        for pat in ["${title}/@(*[[]BASE[]]*.nsp|*.xci)", r"${title}/@(*\[BASE\]*.nsp|*.xci)"] {
            let g = RomGlob::parse(pat).unwrap();
            assert!(g.matches("Zelda/game [BASE].nsp").is_some(), "{pat}");
            assert!(g.matches("Zelda/game [UPD].nsp").is_none(), "{pat}");
        }
    }

    #[test]
    fn negated_extglob_filters_updates_and_dlcs_in_any_folder() {
        let g = RomGlob::parse("**/@(!(*[[]UPD*|*[[]DLC*).nsp|*.xci)").unwrap();
        assert!(g.matches("Mario.nsp").is_some());
        assert!(g.matches("Zelda/Zelda [BASE].nsp").is_some());
        assert!(g.matches("Zelda/Zelda.xci").is_some());
        assert!(g.matches("Zelda/Zelda [UPD v1.2].nsp").is_none());
        assert!(g.matches("Zelda/Zelda [UPD].nsp").is_none());
        assert!(g.matches("Zelda/Zelda [DLC].nsp").is_none());
        assert!(g.matches("notes.txt").is_none());

        let exact = RomGlob::parse("**/@(!(*[UPD]*|*[DLC]*).nsp|*.xci)").unwrap();
        assert!(exact.matches("Zelda/Zelda [UPD].nsp").is_none());
        assert!(exact.matches("Zelda/Zelda [BASE].nsp").is_some());
    }

    #[test]
    fn supports_globstar_braces_and_title_in_filename() {
        let g = RomGlob::parse("**/${title}.{iso,chd}").unwrap();
        let m = g.matches("RPGs/Final Fantasy X.chd").unwrap();
        assert_eq!(m.title.as_deref(), Some("Final Fantasy X"));
        assert!(!m.title_from_dir);

        let root = g.matches("Okami.ISO").unwrap();
        assert_eq!(root.title.as_deref(), Some("Okami"));
        assert!(g.matches("Okami.bin").is_none());
    }

    #[test]
    fn supports_cross_segment_braces_and_character_classes() {
        let g = RomGlob::parse("{usa,eu}/${title} (Disc [1-4]).[iI][sS][oO]").unwrap();
        let m = g.matches("usa/Metal Gear Solid (Disc 2).iso").unwrap();
        assert_eq!(m.title.as_deref(), Some("Metal Gear Solid"));
        assert!(g.matches("usa/Metal Gear Solid (Disc 5).iso").is_none());
        assert!(g.matches("jp/Metal Gear Solid (Disc 1).iso").is_none());

        // Standard minimatch character classes work as character classes by default.
        let n64 = RomGlob::parse("${title}.[nvz]64").unwrap();
        assert_eq!(n64.matches("Mario 64.z64").unwrap().title.as_deref(), Some("Mario 64"));
        assert_eq!(n64.matches("Mario 64.n64").unwrap().title.as_deref(), Some("Mario 64"));
        assert_eq!(n64.matches("Mario 64.v64").unwrap().title.as_deref(), Some("Mario 64"));
        assert!(n64.matches("Mario 64.a64").is_none());
        assert!(n64.matches("Mario 64.[nvz]64").is_none());

        let abc = RomGlob::parse("game_[abc].rom").unwrap();
        assert!(abc.matches("game_b.rom").is_some());
        assert!(abc.matches("game_[abc].rom").is_none());

        // ROM tags with spaces/dots stay literal.
        let upd = RomGlob::parse("*[UPD v1.2]*.nsp").unwrap();
        assert!(upd.matches("Zelda [UPD v1.2].nsp").is_some());
        assert!(upd.matches("Zelda U.nsp").is_none());
    }

    #[test]
    fn nested_extglob_and_long_filenames_stay_fast() {
        let g = RomGlob::parse("**/@(!(*[[]UPD*|*[[]DLC*).nsp|*.xci)").unwrap();
        let long_name = format!("a/b/c/{} [BASE].nsp", "VeryLongGameTitleSegment".repeat(6));
        assert!(g.matches(&long_name).is_some());
        let long_upd = format!("a/b/c/{} [UPD v1.4.2].nsp", "VeryLongGameTitleSegment".repeat(6));
        assert!(g.matches(&long_upd).is_none());

        let nested = RomGlob::parse("+(+(a|aa))b").unwrap();
        assert!(nested.matches(&format!("{}b", "a".repeat(30))).is_some());
        assert!(nested.matches(&format!("{}c", "a".repeat(30))).is_none());
    }

    #[test]
    fn supports_plus_question_and_star_extglobs() {
        let q = RomGlob::parse("${title}?( (Rev [1-9])).sfc").unwrap();
        assert_eq!(q.matches("Super Metroid.sfc").unwrap().title.as_deref(), Some("Super Metroid"));
        assert_eq!(q.matches("Super Metroid (Rev 2).sfc").unwrap().title.as_deref(), Some("Super Metroid"));

        let strict_q = RomGlob::parse("Super Metroid?( (Rev [1-9])).sfc").unwrap();
        assert!(strict_q.matches("Super Metroid.sfc").is_some());
        assert!(strict_q.matches("Super Metroid (Rev 2).sfc").is_some());
        assert!(strict_q.matches("Super Metroid (Rev A).sfc").is_none());

        let p = RomGlob::parse("+(a|b).rom").unwrap();
        assert!(p.matches("abba.rom").is_some());
        assert!(p.matches(".rom").is_none());

        let s = RomGlob::parse("*(a|b).rom").unwrap();
        assert!(s.matches(".rom").is_some());
        assert!(s.matches("baba.rom").is_some());
    }

    #[test]
    fn windows_backslashes_match_forward_slash_patterns() {
        let g = RomGlob::parse("${title}/@(*[BASE]*.nsp|*.xci)").unwrap();
        let m = g.matches(r"Metroid Dread\Metroid Dread [BASE].nsp").unwrap();
        assert_eq!(m.title.as_deref(), Some("Metroid Dread"));
        assert!(m.title_from_dir);
    }

    #[test]
    fn rejects_invalid_patterns_cleanly() {
        assert!(RomGlob::parse("").is_err());
        assert!(RomGlob::parse("${title}/@(*.nsp").is_err());
        assert!(RomGlob::parse("${title}/[abc").is_err());
        assert!(RomGlob::parse("${unknown}/*.nsp").is_err());
        assert!(RomGlob::parse("{a,b/*.nsp").is_err());
    }

    #[test]
    fn switch_presets_include_subfolder_base_and_skip_updates() {
        let presets = presets_for_profile(Some("nintendo_switch"), &["nsp".into(), "xci".into()]);
        assert_eq!(presets[0].pattern, "${title}/@(*[BASE]*.nsp|*.xci)");
        assert_eq!(presets[1].pattern, "**/@(!(*[[]UPD*|*[[]DLC*).nsp|*.xci)");
        for p in &presets {
            assert!(validate(&p.pattern).is_ok(), "{}", p.pattern);
        }
    }
}
