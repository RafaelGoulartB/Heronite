//! Pure rules of the Add/Edit game dialog (no UI): title cleanup, art URLs,
//! platform detection and what the form needs before it can be saved.

use std::path::Path;

use heronite_data::sideload::SideloadPlatform;
use heronite_data::text::remove_special_characters;
use heronite_launch::native::{detect_executable_kind, ExecutableKind};
use heronite_launch::Probe;

/// Heroic's title field is limited to 40 characters.
pub const TITLE_MAX: usize = 40;

/// `handleTitle`: `removeSpecialcharacters`, then the 40 character limit.
pub fn clean_title(raw: &str) -> String {
    remove_special_characters(raw).chars().take(TITLE_MAX).collect()
}

/// `file://` URL of a local image the way Heroic stores it (`file://${path}`),
/// with the few characters that would break the URL percent-encoded.
pub fn file_url(path: &str) -> String {
    let mut out = String::from("file://");
    for c in path.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            c => out.push(c),
        }
    }
    out
}

/// What goes into `art_square` / `art_cover` for a text typed or picked in an
/// art field: URLs and `file:` URLs stay as they are, a plain absolute path
/// becomes a `file://` URL.
pub fn art_value(text: &str) -> String {
    let t = text.trim();
    if t.starts_with('/') {
        file_url(t)
    } else {
        t.to_string()
    }
}

/// Heroic stores `heroUrl || imageUrl` and `imageUrl || heroUrl`: one image
/// fills both slots when the other is empty. Returns `(art_cover, art_square)`.
pub fn art_pair(square: &str, hero: &str) -> (String, String) {
    let (square, hero) = (art_value(square), art_value(hero));
    let cover = if hero.is_empty() { square.clone() } else { hero.clone() };
    let sq = if square.is_empty() { hero } else { square };
    (cover, sq)
}

/// `handleGameUrl`: the browser URL always starts with `https://` (`http://`
/// addresses are kept as typed).
pub fn normalize_url(url: &str) -> String {
    let u = url.trim();
    if u.is_empty() || u.starts_with("https://") || u.starts_with("http://") {
        u.to_string()
    } else {
        format!("https://{u}")
    }
}

/// `${defaultWinePrefixDir}/${removeSpecialcharacters(title)}` of the Wine
/// selector (the shared prefix replaces it when that toggle is on).
pub fn suggested_prefix(prefix_dir: &str, title: &str, app_name: &str) -> String {
    let name = remove_special_characters(if title.trim().is_empty() { app_name } else { title.trim() });
    format!("{}/{}", prefix_dir.trim_end_matches('/'), name)
}

/// The platform the file itself says it needs (ELF / `#!` script / AppImage
/// -> Linux, MZ / `.exe` -> Windows); `None` when it cannot tell.
pub fn detect_platform(path: &str, probe: &dyn Probe) -> Option<SideloadPlatform> {
    match detect_executable_kind(path.trim(), probe) {
        ExecutableKind::Linux => Some(SideloadPlatform::Linux),
        ExecutableKind::Windows => Some(SideloadPlatform::Windows),
        ExecutableKind::Unknown => None,
    }
}

/// `runAs` to store so the chosen platform sticks: when it differs from what
/// detection says, the Collection's "Run as" override pins it; when it agrees
/// an existing override is reset to `auto`; otherwise nothing is touched.
pub fn run_as_for(chosen: SideloadPlatform, detected: Option<SideloadPlatform>, existing: Option<&str>) -> Option<String> {
    let name = |p: SideloadPlatform| match p {
        SideloadPlatform::Linux => Some("linux"),
        SideloadPlatform::Windows => Some("windows"),
        SideloadPlatform::Browser => None,
    };
    let chosen_name = name(chosen)?;
    match detected {
        Some(d) if d != chosen => Some(chosen_name.to_string()),
        Some(_) | None => match existing {
            Some(e) if e != "auto" && !e.is_empty() => Some("auto".to_string()),
            _ => None,
        },
    }
}

/// Why the form cannot be saved yet (empty when it can).
pub fn missing_field(title: &str, platform: SideloadPlatform, executable: &str, url: &str) -> Option<&'static str> {
    if title.trim().is_empty() {
        return Some("Add a title");
    }
    match platform {
        SideloadPlatform::Browser if url.trim().is_empty() => Some("Add the game URL"),
        SideloadPlatform::Linux | SideloadPlatform::Windows if executable.trim().is_empty() => Some("Select the executable"),
        _ => None,
    }
}

/// Parent folder shown as "Install folder" (Heroic's `folder_name`).
pub fn install_folder(executable: &str) -> Option<String> {
    let e = executable.trim();
    if e.is_empty() {
        return None;
    }
    Path::new(e).parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_string_lossy().into_owned())
}

pub fn platform_label(p: SideloadPlatform) -> &'static str {
    match p {
        SideloadPlatform::Linux => "Linux",
        SideloadPlatform::Windows => "Windows",
        SideloadPlatform::Browser => "Browser",
    }
}

pub const PLATFORMS: [SideloadPlatform; 3] = [SideloadPlatform::Linux, SideloadPlatform::Windows, SideloadPlatform::Browser];

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_launch::probe::FakeProbe;

    #[test]
    fn titles_lose_special_characters_and_are_capped() {
        assert_eq!(clean_title("Sonic: The Hedgehog™ 2?"), "Sonic The Hedgehog 2");
        assert_eq!(clean_title(&"a".repeat(60)).chars().count(), 40);
        assert_eq!(clean_title("Ação & Reação"), "Ação  Reação");
    }

    #[test]
    fn local_images_become_encoded_file_urls() {
        assert_eq!(file_url("/home/u/My Pics/a#b%c.png"), "file:///home/u/My%20Pics/a%23b%25c.png");
        assert_eq!(art_value("  /x/y.png "), "file:///x/y.png");
        assert_eq!(art_value("https://x/y.png"), "https://x/y.png");
        assert_eq!(art_value("file:///x/y.png"), "file:///x/y.png");
        assert_eq!(art_value("  "), "");
        // The app's own decoder reads it back.
        let url = file_url("/a b/c%d.png");
        let back = heronite_data::paths::percent_decode(url.strip_prefix("file://").unwrap());
        assert_eq!(back, "/a b/c%d.png");
    }

    #[test]
    fn one_image_fills_both_art_slots() {
        assert_eq!(art_pair("S", "H"), ("H".to_string(), "S".to_string()));
        assert_eq!(art_pair("S", ""), ("S".to_string(), "S".to_string()));
        assert_eq!(art_pair("", "H"), ("H".to_string(), "H".to_string()));
        assert_eq!(art_pair("", ""), (String::new(), String::new()));
    }

    #[test]
    fn urls_get_https() {
        assert_eq!(normalize_url("example.com/play"), "https://example.com/play");
        assert_eq!(normalize_url("https://a.b"), "https://a.b");
        assert_eq!(normalize_url("http://a.b"), "http://a.b");
        assert_eq!(normalize_url(" "), "");
    }

    #[test]
    fn prefix_suggestion_follows_the_wine_selector() {
        assert_eq!(suggested_prefix("/h/Games/Heroic/Prefixes/", "Half-Life: 2", "abc"), "/h/Games/Heroic/Prefixes/Half-Life 2");
        assert_eq!(suggested_prefix("/p", "", "abc"), "/p/abc");
    }

    #[test]
    fn platform_detection_and_run_as() {
        let probe = FakeProbe::new("/h")
            .with_file("/g/elf", b"\x7fELF\x02")
            .with_file("/g/pe", b"MZ\x90")
            .with_file("/g/blob", b"\x00\x01\x02\x03");
        assert_eq!(detect_platform("/g/elf", &probe), Some(SideloadPlatform::Linux));
        assert_eq!(detect_platform("/g/pe", &probe), Some(SideloadPlatform::Windows));
        assert_eq!(detect_platform("/g/blob", &probe), None);
        assert_eq!(detect_platform("/g/game.exe", &probe), Some(SideloadPlatform::Windows));
        assert_eq!(detect_platform("", &probe), None);

        use SideloadPlatform::*;
        // Same as detection: nothing to pin, an old override is reset.
        assert_eq!(run_as_for(Linux, Some(Linux), None), None);
        assert_eq!(run_as_for(Linux, Some(Linux), Some("windows")), Some("auto".into()));
        assert_eq!(run_as_for(Linux, Some(Linux), Some("auto")), None);
        // Different from detection: pin it.
        assert_eq!(run_as_for(Windows, Some(Linux), None), Some("windows".into()));
        assert_eq!(run_as_for(Linux, Some(Windows), Some("auto")), Some("linux".into()));
        // Undetectable file: the stored platform decides, nothing pinned.
        assert_eq!(run_as_for(Windows, None, None), None);
        // Web pages have no runAs.
        assert_eq!(run_as_for(Browser, None, Some("linux")), None);
    }

    #[test]
    fn form_needs_title_and_what_to_run() {
        use SideloadPlatform::*;
        assert_eq!(missing_field(" ", Linux, "/x", ""), Some("Add a title"));
        assert_eq!(missing_field("G", Windows, " ", ""), Some("Select the executable"));
        assert_eq!(missing_field("G", Browser, "", ""), Some("Add the game URL"));
        assert_eq!(missing_field("G", Browser, "", "x.com"), None);
        assert_eq!(missing_field("G", Linux, "/x", ""), None);
    }

    #[test]
    fn install_folder_is_the_executables_parent() {
        assert_eq!(install_folder("/g/My Game/game.exe").as_deref(), Some("/g/My Game"));
        assert_eq!(install_folder(""), None);
        assert_eq!(install_folder("game.exe"), None);
    }
}
