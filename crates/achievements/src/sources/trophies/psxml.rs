//! The trophy list of a PlayStation game: the `trophyconf` XML of its
//! trophy pack (RPCS3's `TROPCONF.SFM`, shadPS4's `TROP.XML`). shadPS4 also
//! writes each trophy's unlock (`unlockstate`, `timestamp`) into its copy.
//!
//! ```xml
//! <trophyconf version="1.1" platform="ps4">
//!   <npcommid>NPWR01234_00</npcommid>
//!   <title-name>The Game</title-name>
//!   <group id="001"><name>The DLC</name></group>
//!   <trophy id="000" hidden="no" ttype="P" pid="-1"><name>…</name><detail>…</detail></trophy>
//!   <trophy id="001" hidden="yes" ttype="B" pid="000" gid="001" unlockstate="true" timestamp="1700000000">…</trophy>
//! </trophyconf>
//! ```

use std::collections::HashSet;
use std::path::Path;

use chrono::Utc;
use serde_json::Map;

use crate::model::{Achievement, AchievementGroup, AchievementSet, Grade, Source};

/// More nodes than any trophy list has (one has at most a few hundred
/// trophies): a bound on what a damaged file costs.
const NODES_LIMIT: u32 = 200_000;
/// Elements nested deeper than this are not a trophy list (one nests three
/// deep); the parser recurses per level, so a damaged file must not reach it.
const MAX_DEPTH: usize = 64;
/// Comm ids taken from one file (a game names a handful).
const MAX_COMM_IDS: usize = 64;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrophyConf {
    pub npcommid: String,
    pub title: String,
    /// The DLC packs (`gid` → name), in the file's order.
    pub groups: Vec<(String, String)>,
    pub trophies: Vec<Trophy>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Trophy {
    pub id: u32,
    pub grade: Option<Grade>,
    pub hidden: bool,
    pub name: String,
    pub detail: String,
    pub gid: Option<String>,
    /// shadPS4's copies only.
    pub unlocked: bool,
    pub timestamp: Option<u64>,
}

/// pugixml's `as_bool`, which shadPS4 reads these with: true when the value
/// starts with `1`, `t` or `y` (any case).
fn truthy(v: Option<&str>) -> bool {
    matches!(v.and_then(|v| v.trim().chars().next()), Some('1' | 't' | 'T' | 'y' | 'Y'))
}

fn text_of(node: roxmltree::Node, tag: &str) -> String {
    node.children()
        .find(|c| c.has_tag_name(tag))
        .and_then(|c| c.text())
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

/// Where `pat` next starts in `b` from `from` (`b.len()` when it does not).
fn find(b: &[u8], from: usize, pat: &[u8]) -> usize {
    b.get(from..).and_then(|r| r.windows(pat.len()).position(|w| w == pat)).map_or(b.len(), |p| from + p)
}

/// How deep the elements of `text` nest, counted from its tags (`<a>`,
/// `</a>`, `<a/>`) as the parser sees them: comments, CDATA, processing
/// instructions and quoted attribute values are skipped whole, whatever `>`
/// or `</` they hold. Any other `<!` ends the count: the parser (without a
/// DTD) fails there, before reading what follows. One pass: linear.
fn nesting(text: &str) -> usize {
    let b = text.as_bytes();
    let (mut depth, mut max, mut i) = (0usize, 0usize, 0usize);
    while i < b.len() {
        let at = find(b, i, b"<");
        let rest = b.get(at..).unwrap_or_default();
        i = if rest.starts_with(b"<!--") {
            find(b, at + 4, b"-->").saturating_add(3)
        } else if rest.starts_with(b"<![CDATA[") {
            find(b, at + 9, b"]]>").saturating_add(3)
        } else if rest.starts_with(b"<?") {
            find(b, at + 2, b"?>").saturating_add(2)
        } else if rest.starts_with(b"<!") {
            break;
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            find(b, at, b">").saturating_add(1)
        } else if rest.is_empty() {
            b.len()
        } else {
            // A start tag: its end is the first `>` outside quotes.
            let (mut j, mut quote) = (at + 1, None);
            while let Some(&c) = b.get(j) {
                match quote {
                    Some(q) if c == q => quote = None,
                    Some(_) => {}
                    None if c == b'"' || c == b'\'' => quote = Some(c),
                    None if c == b'>' => break,
                    None => {}
                }
                j += 1;
            }
            if b.get(j.wrapping_sub(1)) != Some(&b'/') {
                depth += 1;
                max = max.max(depth);
            }
            j.saturating_add(1)
        };
    }
    max
}

/// `text` with its `<!DOCTYPE name>` blanked out. Trophy lists have only
/// that bare form; a declaration with a subset or an external id (where
/// entities could expand without bound) is refused.
fn without_doctype(text: &str) -> Result<std::borrow::Cow<'_, str>, String> {
    let Some(at) = text.find("<!DOCTYPE") else { return Ok(text.into()) };
    let end = text.get(at..).and_then(|r| r.find('>')).map(|e| at + e).ok_or("not a trophy list: a cut declaration")?;
    let decl = text.get(at..=end).unwrap_or_default();
    if decl.contains(['[', '\'', '"']) {
        return Err("not a trophy list: a document type with declarations".into());
    }
    let (before, after) = (text.get(..at).unwrap_or_default(), text.get(end + 1..).unwrap_or_default());
    Ok(format!("{before}{}{after}", " ".repeat(decl.len())).into())
}

pub fn parse(bytes: &[u8]) -> Result<TrophyConf, String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = String::from_utf8_lossy(bytes);
    // Packs pad the file after the root's end (shadPS4's `removePadding`).
    let text = text.trim_end_matches(|c: char| c == '\0' || c.is_whitespace());
    let text = without_doctype(text)?;
    // The parser recurses per level: a file nested too deep must not reach it.
    if nesting(&text) > MAX_DEPTH {
        return Err("not a trophy list: nested too deep".into());
    }
    let options = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: NODES_LIMIT, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(&text, options).map_err(|e| format!("not a trophy list: {e}"))?;
    let root = doc.root_element();
    if !root.has_tag_name("trophyconf") {
        return Err(format!("not a trophy list: <{}>", root.tag_name().name()));
    }
    let mut conf = TrophyConf { npcommid: text_of(root, "npcommid"), title: text_of(root, "title-name"), ..Default::default() };
    let mut seen = HashSet::new();
    let mut seen_groups = HashSet::new();
    for node in root.children().filter(|n| n.is_element()) {
        match node.tag_name().name() {
            "group" => {
                if let Some(id) = node.attribute("id").map(str::trim).filter(|id| !id.is_empty()) {
                    if seen_groups.insert(id.to_string()) {
                        conf.groups.push((id.to_string(), text_of(node, "name")));
                    }
                }
            }
            "trophy" => {
                let Some(id) = node.attribute("id").and_then(|id| id.trim().parse::<u32>().ok()) else { continue };
                if !seen.insert(id) {
                    continue;
                }
                conf.trophies.push(Trophy {
                    id,
                    grade: node.attribute("ttype").and_then(Grade::from_ttype),
                    hidden: truthy(node.attribute("hidden")),
                    name: text_of(node, "name"),
                    detail: text_of(node, "detail"),
                    gid: node.attribute("gid").map(str::trim).filter(|g| !g.is_empty()).map(str::to_string),
                    unlocked: truthy(node.attribute("unlockstate")),
                    timestamp: node.attribute("timestamp").and_then(|t| t.trim().parse::<u64>().ok()).filter(|t| *t > 0),
                });
            }
            _ => {}
        }
    }
    Ok(conf)
}

/// The base game's group (its trophies have no `gid`): "Base game". Not a
/// number, so no pack's own group has it.
const BASE_GROUP: &str = "base";

/// The set of a trophy list. `unlock` says whether a trophy is unlocked and
/// when; `icon` where its picture is (empty when there is none).
pub fn build(
    source: Source,
    conf: &TrophyConf,
    game_title: &str,
    unlock: impl Fn(&Trophy) -> (bool, Option<chrono::DateTime<Utc>>),
    icon: impl Fn(u32) -> Option<std::path::PathBuf>,
) -> AchievementSet {
    let grouped = !conf.groups.is_empty();
    let achievements = conf
        .trophies
        .iter()
        .map(|t| {
            let (unlocked, unlocked_at) = unlock(t);
            Achievement {
                id: t.id.to_string(),
                name: if t.name.is_empty() { format!("#{}", t.id) } else { t.name.clone() },
                description: t.detail.clone(),
                hidden: t.hidden,
                icon: icon(t.id).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
                unlocked,
                unlocked_at: unlocked_at.filter(|_| unlocked),
                grade: t.grade,
                group: grouped.then(|| t.gid.clone().unwrap_or_else(|| BASE_GROUP.to_string())),
                ..Default::default()
            }
        })
        .collect();
    let mut groups = Vec::new();
    if grouped {
        groups.push(AchievementGroup { id: BASE_GROUP.into(), order: Some(0), dlc_app_id: Some(0), ..Default::default() });
        for (i, (id, name)) in conf.groups.iter().enumerate() {
            let order = i64::try_from(i).map_or(i64::MAX, |i| i.saturating_add(1));
            groups.push(AchievementGroup { id: id.clone(), name: name.clone(), order: Some(order), ..Default::default() });
        }
    }
    let title = Some(conf.title.clone()).filter(|t| !t.is_empty() && !super::same_title(t, game_title));
    AchievementSet { source, external_id: conf.npcommid.clone(), title, fetched_at: Utc::now(), achievements, groups, unknown: Map::new() }
}

/// A trophy's picture in a trophy folder: `TROP000.PNG`, when it is there.
pub fn icon_in(dir: &Path, id: u32) -> Option<std::path::PathBuf> {
    let file = dir.join(format!("TROP{id:03}.PNG"));
    file.is_file().then_some(file)
}

/// The NP communication ids (`NPWR01234_00`) a game's files name: four
/// capital letters, five digits, `_`, two digits.
pub fn comm_ids(bytes: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in bytes.windows(12) {
        let ok = w.iter().take(4).all(u8::is_ascii_uppercase)
            && w.iter().skip(4).take(5).all(u8::is_ascii_digit)
            && w.get(9) == Some(&b'_')
            && w.iter().skip(10).all(u8::is_ascii_digit);
        if ok {
            let id = String::from_utf8_lossy(w).into_owned();
            if !out.contains(&id) {
                out.push(id);
                if out.len() >= MAX_COMM_IDS {
                    break;
                }
            }
        }
    }
    out
}

/// A folder name that is an NP communication id.
pub fn is_comm_id(name: &str) -> bool {
    name.len() == 12 && comm_ids(name.as_bytes()).len() == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONF: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE trophyconf>
<trophyconf version="1.1" platform="ps4" policy="large">
<npcommid>NPWR01234_00</npcommid>
<title-name>The   Game</title-name>
<group id="001"><name>The DLC</name><detail>More</detail></group>
<trophy id="000" hidden="no" ttype="P" pid="-1"><name>All of it</name><detail>Every trophy.</detail></trophy>
<trophy id="001" hidden="yes" ttype="B" pid="000" unlockstate="true" timestamp="1700000000"><name>First</name><detail>Start.</detail></trophy>
<trophy id="002" hidden="no" ttype="G" pid="-1" gid="001" unlockstate="false"><name>Extra</name></trophy>
<trophy id="001" ttype="S"><name>Again</name></trophy>
<trophy id="x" ttype="S"><name>Broken</name></trophy>
</trophyconf>"#;

    #[test]
    fn reads_the_trophies_groups_and_unlocks() {
        let c = parse(CONF.as_bytes()).unwrap();
        assert_eq!((c.npcommid.as_str(), c.title.as_str()), ("NPWR01234_00", "The Game"));
        assert_eq!(c.groups, [("001".to_string(), "The DLC".to_string())]);
        let ids: Vec<u32> = c.trophies.iter().map(|t| t.id).collect();
        assert_eq!(ids, [0, 1, 2], "a repeated or broken id is left out");
        let first = &c.trophies[1];
        assert!(first.hidden && first.unlocked);
        assert_eq!((first.grade, first.timestamp, first.name.as_str()), (Some(Grade::Bronze), Some(1_700_000_000), "First"));
        assert_eq!(c.trophies[2].gid.as_deref(), Some("001"));
        assert!(!c.trophies[2].unlocked);
    }

    #[test]
    fn builds_a_set_with_the_base_game_first() {
        let c = parse(CONF.as_bytes()).unwrap();
        let set = build(Source::ShadPs4, &c, "The Game", |t| (t.unlocked, None), |_| None);
        assert_eq!((set.total(), set.unlocked(), set.external_id.as_str()), (3, 1, "NPWR01234_00"));
        assert_eq!(set.title, None, "the same name as ours");
        let groups: Vec<_> = set.achievements.iter().map(|a| a.group.as_deref()).collect();
        assert_eq!(groups, [Some("base"), Some("base"), Some("001")]);
        assert_eq!(set.sections().len(), 2);
        assert_eq!(set.achievements[0].grade, Some(Grade::Platinum));
        let other = build(Source::ShadPs4, &c, "Another name", |_| (false, None), |_| None);
        assert_eq!(other.title.as_deref(), Some("The Game"));
        assert_eq!(other.unlocked(), 0);
    }

    #[test]
    fn padding_and_a_bom_are_fine_and_anything_else_fails() {
        let mut padded = b"\xEF\xBB\xBF".to_vec();
        padded.extend_from_slice(CONF.as_bytes());
        padded.extend_from_slice(&[0; 64]);
        assert_eq!(parse(&padded).unwrap().trophies.len(), 3);
        assert!(parse(b"").is_err());
        assert!(parse(b"<other/>").is_err());
        assert!(parse(&CONF.as_bytes()[..CONF.len() / 2]).is_err(), "half written");
        assert!(parse(&[0xFF, 0xFE, 0x00, 0x3C]).is_err());
    }

    #[test]
    fn entities_and_deep_nesting_are_refused() {
        let entities = CONF.replace("<!DOCTYPE trophyconf>", r#"<!DOCTYPE trophyconf [<!ENTITY a "aaaa">]>"#);
        assert!(parse(entities.as_bytes()).is_err());
        let deep = format!("<trophyconf>{}{}</trophyconf>", "<a>".repeat(100_000), "</a>".repeat(100_000));
        assert!(parse(deep.as_bytes()).is_err());
        assert_eq!(nesting(&without_doctype(CONF).unwrap()), 3);
        assert_eq!(nesting("<a><b/><!-- <c> --><?x?></a>"), 1);
        // A `>` or `</` where the parser sees none.
        for unit in [r#"<a x="/>">"#, "<a><!-- ></a>-->", "<a><![CDATA[></a>]]>", "<a><?p ></a>?>", "<a x='>'>"] {
            let text = format!("<trophyconf>{}</trophyconf>", unit.repeat(1000));
            assert!(nesting(&text) >= 1000, "{unit}");
            assert!(parse(text.as_bytes()).is_err(), "{unit}");
        }
        // Any document type but the bare one is refused, however it is written.
        let deep = format!("<trophyconf>{}", "<a>".repeat(1000));
        for decl in [
            "<!DOCTYPE trophyconf [ ] >",
            r#"<!DOCTYPE trophyconf SYSTEM "[">"#,
            r#"<!DOCTYPE trophyconf SYSTEM '><a x="'>"#,
            r#"<!DOCTYPE trophyconf [ <!-- ]> <a x=" --> ]>"#,
            "<!DOCTYPE trophyconf",
        ] {
            assert!(parse(format!("{decl}{deep}").as_bytes()).is_err(), "{decl}");
        }
        // Another `<!` ends the count where the parser fails.
        assert_eq!(nesting("<a><b><!x <c><d>"), 2);
        assert!(parse(format!("<trophyconf><!x>{deep}").as_bytes()).is_err());
        // Linear on a file of declarations.
        let many = "<!>".repeat(1 << 20);
        let started = std::time::Instant::now();
        assert!(parse(many.as_bytes()).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn any_cut_of_the_file_reads_or_fails_without_panicking() {
        let bytes = CONF.as_bytes();
        for end in 0..bytes.len() {
            let _ = parse(&bytes[..end]);
        }
        for m in crate::sources::trophies::mutants(bytes, 3000) {
            let _ = parse(&m);
            let _ = comm_ids(&m);
        }
    }

    #[test]
    fn finds_comm_ids_in_a_games_files() {
        let npbind = b"\x00\x00NPWR01234_00\x00\x00junkNPWR9_00 NPWR01234_00 CUSA00001_00";
        assert_eq!(comm_ids(npbind), ["NPWR01234_00", "CUSA00001_00"]);
        assert!(is_comm_id("NPWR01234_00"));
        assert!(!is_comm_id("NPWR01234_0"));
        assert!(!is_comm_id("npwr01234_00"));
        assert!(comm_ids(b"short").is_empty());
    }
}
