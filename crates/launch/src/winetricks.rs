//! What the game settings' Winetricks page lists: the components the
//! winetricks script offers and the ones a prefix already has.
//!
//! * The catalog is read straight from the script (`<tools>/winetricks`): each
//!   verb is declared by a `w_metadata <name> <category>` block followed by
//!   `key="value"` lines (title, publisher, year, media). Nothing runs, so the
//!   list opens instantly and offline.
//! * winetricks appends every verb it applies to `winetricks.log` at the root
//!   of the prefix it worked on: the prefix itself with Wine or umu, `pfx/`
//!   inside it with plain Proton. Both are read.
//!
//! Installing goes through [`crate::wine_tools::winetricks_install_command`].

use std::path::Path;

/// The kinds of verbs the page offers (apps and benchmarks are left out).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    /// Runtime libraries: Visual C++, .NET, DirectX parts, codecs...
    Dlls,
    Fonts,
    /// Prefix tweaks: Windows version, renderer, virtual desktop...
    Settings,
}

impl Category {
    pub const ALL: [Category; 3] = [Category::Dlls, Category::Fonts, Category::Settings];

    fn parse(s: &str) -> Option<Self> {
        match s {
            "dlls" => Some(Self::Dlls),
            "fonts" => Some(Self::Fonts),
            "settings" => Some(Self::Settings),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verb {
    pub name: String,
    pub category: Category,
    pub title: String,
    pub publisher: String,
    pub year: String,
    /// The files must be fetched by hand first (`media="manual_download"`):
    /// a quiet install fails without them.
    pub manual: bool,
}

/// Reads the catalog of the script at `script` (empty when it is missing).
pub fn catalog(script: &Path) -> Vec<Verb> {
    std::fs::read_to_string(script).map(|text| parse_catalog(&text)).unwrap_or_default()
}

/// The `w_metadata` blocks of a winetricks script, in the script's order.
pub fn parse_catalog(script: &str) -> Vec<Verb> {
    let mut out = Vec::new();
    let mut lines = script.lines();
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix("w_metadata ") else { continue };
        let mut words = rest.split_whitespace();
        let (Some(name), Some(category)) = (words.next(), words.next()) else { continue };
        let Some(category) = Category::parse(category) else { continue };
        let mut verb =
            Verb { name: name.to_string(), category, title: String::new(), publisher: String::new(), year: String::new(), manual: false };
        // The block continues while lines end with a backslash.
        let mut more = line.trim_end().ends_with('\\');
        while more {
            let Some(next) = lines.next() else { break };
            let body = next.trim();
            more = body.ends_with('\\');
            let body = body.trim_end_matches('\\').trim();
            let Some((key, value)) = body.split_once('=') else { continue };
            let value = value.trim().trim_matches('"').to_string();
            match key.trim() {
                "title" => verb.title = value,
                "publisher" => verb.publisher = value,
                "year" => verb.year = value,
                "media" => verb.manual = value == "manual_download",
                _ => {}
            }
        }
        out.push(verb);
    }
    out
}

/// The verbs already applied to `prefix`, oldest first, each once.
pub fn installed(prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for log in [Path::new(prefix).join("winetricks.log"), Path::new(prefix).join("pfx").join("winetricks.log")] {
        let Ok(text) = std::fs::read_to_string(&log) else { continue };
        for verb in parse_log(&text) {
            if !out.contains(&verb) {
                out.push(verb);
            }
        }
    }
    out
}

/// One verb per line; flags (`-q`) and blank lines are not verbs.
fn parse_log(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('-')).map(str::to_string).collect()
}

/// Whether `verb` matches the folded search `tokens` (name, title or
/// publisher; every word must match).
pub fn matches(verb: &Verb, tokens: &[&str]) -> bool {
    if tokens.is_empty() {
        return true;
    }
    let hay = heronite_data::fuzzy::fold(&format!("{} {} {}", verb.name, verb.title, verb.publisher));
    tokens.iter().all(|t| hay.contains(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = r#"
w_metadata()
{
    something
}

w_metadata vcrun2022 dlls \
    title="Visual C++ 2015-2022 libraries (msvcp140.dll)" \
    publisher="Microsoft" \
    year="2022" \
    media="download" \
    file1="vc_redist.x86.exe"

load_vcrun2022()
{
    :
}

w_metadata corefonts fonts \
    title="MS Arial, Courier, Times fonts" \
    publisher="Microsoft" \
    year="2008" \
    media="download"

w_metadata steam apps \
    title="Steam Client"

w_metadata win10 settings \
    title="Set Windows version to Windows 10"

w_metadata dirac dlls \
    title="The Dirac directshow filter v1.0.2" \
    publisher="Dirac" \
    year="2009" \
    media="manual_download"
"#;

    #[test]
    fn reads_the_metadata_blocks() {
        let c = parse_catalog(SCRIPT);
        let names: Vec<&str> = c.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["vcrun2022", "corefonts", "win10", "dirac"], "apps are left out");
        let vc = &c[0];
        assert_eq!(vc.category, Category::Dlls);
        assert_eq!(vc.title, "Visual C++ 2015-2022 libraries (msvcp140.dll)");
        assert_eq!((vc.publisher.as_str(), vc.year.as_str(), vc.manual), ("Microsoft", "2022", false));
        assert_eq!(c[1].category, Category::Fonts);
        assert_eq!(c[2].category, Category::Settings);
        assert_eq!(c[2].publisher, "");
        assert!(c[3].manual);
    }

    #[test]
    fn installed_reads_both_logs_once() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().to_string_lossy().into_owned();
        assert!(installed(&prefix).is_empty());
        std::fs::write(dir.path().join("winetricks.log"), "vcrun2022\n-q\n\ncorefonts\n").unwrap();
        std::fs::create_dir_all(dir.path().join("pfx")).unwrap();
        std::fs::write(dir.path().join("pfx/winetricks.log"), "corefonts\nwin10\n").unwrap();
        assert_eq!(installed(&prefix), ["vcrun2022", "corefonts", "win10"]);
    }

    #[test]
    fn search_ignores_case_and_accents() {
        let c = parse_catalog(SCRIPT);
        let find = |q: &str| {
            let folded = heronite_data::fuzzy::fold(q);
            let tokens = heronite_data::fuzzy::tokens(&folded);
            c.iter().filter(|v| matches(v, &tokens)).map(|v| v.name.clone()).collect::<Vec<_>>()
        };
        assert_eq!(find("VCRUN"), ["vcrun2022"]);
        assert_eq!(find("microsoft fonts"), ["corefonts"]);
        assert_eq!(find("msvcp140"), ["vcrun2022"], "the title counts");
        assert_eq!(find("").len(), 4);
    }
}
