//! The entries Heronite wants in Sunshine: the picked games (installed, or
//! the installed favourites) and, when asked, one that opens console mode.
//! Each starts through Heronite's protocol URL, like "Add to Steam"; covers
//! are rendered from the art cache as 600x800 PNG (Sunshine only shows PNG,
//! and Moonlight draws box art at 3:4).
//!
//! [`entries`] and [`remove_stale_covers`] are blocking (library files,
//! image decoding): off the UI thread. [`fingerprint`] only reads memory.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use heronite_data::cover_family::CoverFamily;
use heronite_data::{art, AppPaths, ArtContext, CollectionGame, DataPaths, Library};
use heronite_i18n::tr;
use heronite_sunshine::entry::{Entry, CONSOLE_ID};
use heronite_sunshine::prefs::{Games, Prefs};

use crate::system::cli::SCHEME;
use crate::system::shortcuts::{image_opts, own_exe, render};

/// Box art size: what Moonlight shows (3:4), sharp on a TV.
const COVER_W: u32 = 600;
const COVER_H: u32 = 800;

/// The games that go to Sunshine, in library order.
pub fn picked(lib: &Library, games: Games) -> impl Iterator<Item = &Arc<CollectionGame>> {
    lib.games.iter().filter(move |g| {
        g.installed
            && !g.hidden
            && g.game.install.is_dlc != Some(true)
            && g.launch_kind.as_deref() != Some("unavailable")
            && (games == Games::Installed || g.favourite)
    })
}

/// Changes when the entries would: the picked games, their titles, and the
/// options that shape an entry. Covers are not in it (a changed cover is
/// picked up by the next sync).
pub fn fingerprint(lib: &Library, prefs: &Prefs) -> u64 {
    let mut h = DefaultHasher::new();
    (prefs.games.as_str(), prefs.stop_on_quit, prefs.console_entry, &prefs.address, &prefs.username).hash(&mut h);
    heronite_i18n::current().hash(&mut h);
    for g in picked(lib, prefs.games) {
        (&g.key, &g.title).hash(&mut h);
    }
    h.finish()
}

fn quoted(s: &str) -> String {
    format!("\"{s}\"")
}

/// Percent-encodes a URL query value (the URL parser decodes it).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn url(action: &str, game: &CollectionGame) -> String {
    format!("{SCHEME}://{action}?appName={}&runner={}", encode(&game.app_name), encode(&game.runner))
}

/// The command that starts `game`: hidden window, as Steam shortcuts do.
/// `--handoff`: with Heronite not running, the instance starts detached
/// and this command exits at once, as it does when Heronite runs. Sunshine
/// then treats the launch as detached (`auto-detach`) and quitting the
/// stream ends nothing but what `undo` asks; otherwise Heronite and the game
/// would be in Sunshine's process group, ended with it whatever "Quitting
/// in Moonlight closes the game" says.
pub fn launch_cmd(exe: &Path, game: &CollectionGame) -> String {
    launch_line(exe, &url("launch", game))
}

fn launch_line(exe: &Path, url: &str) -> String {
    format!("{} --no-gui {} {}", quoted(&exe.to_string_lossy()), crate::system::cli::HANDOFF, quoted(url))
}

/// The command that closes `game` when the stream's application ends.
pub fn stop_cmd(exe: &Path, game: &CollectionGame) -> String {
    format!("{} {}", quoted(&exe.to_string_lossy()), quoted(&url("stop", game)))
}

/// FNV-1a: the same in every build (`DefaultHasher` may change with Rust).
fn fnv64(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// A file name for a cover: the game and the art it is made from, so new
/// art gets a new file (and Sunshine sees the change). Stable across builds:
/// an update of Heronite does not rename (and so re-render and re-send)
/// every cover.
fn cover_name(key: &str, chain: &[heronite_data::ImageRef]) -> String {
    use heronite_data::ImageRef;
    let mut text = String::from(key);
    for image in chain {
        text.push('\0');
        match image {
            ImageRef::File(p) => {
                text.push_str("file:");
                text.push_str(&p.to_string_lossy());
            }
            ImageRef::Url(u) => {
                text.push_str("url:");
                text.push_str(u);
            }
            ImageRef::Fallback => text.push_str("fallback"),
        }
    }
    format!("{:016x}.png", fnv64(text.bytes()))
}

fn cover(dir: &Path, key: &str, chain: &[heronite_data::ImageRef], opts: &heronite_images::Opts) -> Option<PathBuf> {
    let path = dir.join(cover_name(key, chain));
    if path.is_file() {
        return Some(path);
    }
    render(chain, COVER_W, COVER_H, &path, true, opts)
}

/// Heronite's icon in the middle of a dark 600x800 card.
fn console_cover(dir: &Path) -> Option<PathBuf> {
    let path = dir.join("console.png");
    if path.is_file() {
        return Some(path);
    }
    let made = (|| -> Result<(), String> {
        let icon = image::load_from_memory(include_bytes!("../../../assets/app-icon/graphite.png")).map_err(|e| e.to_string())?;
        let icon = icon.resize(360, 360, image::imageops::FilterType::Lanczos3).to_rgba8();
        let mut card = image::RgbaImage::from_pixel(COVER_W, COVER_H, image::Rgba([18, 20, 26, 255]));
        let (x, y) = ((COVER_W - icon.width()) / 2, (COVER_H - icon.height()) / 2);
        image::imageops::overlay(&mut card, &icon, x.into(), y.into());
        let mut png = Vec::new();
        card.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        heronite_data::temps::write_atomic(&path, &png, None).map_err(|e| e.to_string())
    })();
    match made {
        Ok(()) => Some(path),
        Err(e) => {
            log::warn!("Sunshine console cover: {e}");
            None
        }
    }
}

/// Everything Heronite wants in Sunshine now.
pub fn entries(data_paths: &DataPaths, lib: &Library, prefs: &Prefs) -> Vec<Entry> {
    let exe = own_exe();
    let dir = AppPaths::for_data(data_paths).sunshine_covers_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("Sunshine covers {}: {e}", dir.display());
    }
    let shots = (!lib.settings.screenshots.folder.is_empty()).then(|| PathBuf::from(&lib.settings.screenshots.folder));
    let ctx = ArtContext::new(data_paths, shots.as_deref());
    let opts = image_opts(data_paths);
    let image = |p: Option<PathBuf>| p.map(|p| p.to_string_lossy().into_owned());
    let mut out: Vec<Entry> = picked(lib, prefs.games)
        .map(|g| Entry {
            id: g.key.clone(),
            name: g.title.clone(),
            cmd: launch_cmd(&exe, g),
            // Moonlight shows box art: the tall family.
            image_path: image(cover(&dir, &g.key, &art::family_cover(&ctx, g, CoverFamily::Tall).chain, &opts)),
            undo: prefs.stop_on_quit.then(|| stop_cmd(&exe, g)),
        })
        .collect();
    if prefs.console_entry {
        out.push(Entry {
            id: CONSOLE_ID.into(),
            name: tr!("Heronite console mode").into(),
            cmd: format!("{} --console --fullscreen", quoted(&exe.to_string_lossy())),
            image_path: image(console_cover(&dir)),
            undo: None,
        });
    }
    out
}

/// Deletes the covers no entry points at any more (after a sync went through).
pub fn remove_stale_covers(data_paths: &DataPaths, entries: &[Entry]) {
    let dir = AppPaths::for_data(data_paths).sunshine_covers_dir();
    let used: HashSet<PathBuf> = entries.iter().filter_map(|e| e.image_path.as_ref().map(PathBuf::from)).collect();
    let Ok(read) = std::fs::read_dir(&dir) else { return };
    for f in read.flatten() {
        let path = f.path();
        if path.extension().is_some_and(|e| e == "png") && !used.contains(&path) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::cli::{parse, UrlAction};

    #[test]
    fn query_values_are_encoded_and_read_back() {
        assert_eq!(encode("My Game&Co?=#%+"), "My%20Game%26Co%3F%3D%23%25%2B");
        assert_eq!(encode("abc-1_2.~"), "abc-1_2.~");
        let url = format!("{SCHEME}://launch?appName={}&runner=sideload", encode("A & B+C"));
        match crate::system::cli::parse_url(&url) {
            Some(UrlAction::Launch(r)) => assert_eq!(r.app_name, "A & B+C"),
            other => panic!("{other:?}"),
        }
        let stop = format!("{SCHEME}://stop?appName={}&runner=gog", encode("1207658930"));
        // The launch command, split as Sunshine's shell does: a hidden,
        // handed-off launch of that game.
        let cmd = launch_line(Path::new("/opt/My Apps/heronite-rs"), &format!("{SCHEME}://launch?appName={}&runner=gog", encode("A B")));
        let words: Vec<String> = cmd
            .split('"')
            .enumerate()
            .flat_map(|(i, part)| if i % 2 == 1 { vec![part.to_string()] } else { part.split_whitespace().map(str::to_string).collect() })
            .collect();
        assert_eq!(words[0], "/opt/My Apps/heronite-rs");
        let launch = parse(words.into_iter().skip(1));
        assert!(launch.handoff && launch.no_gui && launch.errors.is_empty());
        assert_eq!(launch.launch.map(|r| (r.app_name, r.runner)), Some(("A B".to_string(), Some("gog".to_string()))));
        let cli = parse([stop]);
        assert_eq!(cli.stop.map(|r| r.app_name).as_deref(), Some("1207658930"));
    }

    #[test]
    fn cover_names_change_with_the_art() {
        let a = vec![heronite_data::ImageRef::Url("https://x/1.jpg".into())];
        let b = vec![heronite_data::ImageRef::Url("https://x/2.jpg".into())];
        assert_eq!(cover_name("gog_1", &a), cover_name("gog_1", &a));
        assert_ne!(cover_name("gog_1", &a), cover_name("gog_1", &b));
        assert_ne!(cover_name("gog_1", &a), cover_name("gog_2", &a));
        assert!(cover_name("gog_1", &a).ends_with(".png"));
        // Fixed: the same name in every build of Heronite.
        assert_eq!(cover_name("gog_1", &a), format!("{:016x}.png", fnv64("gog_1\0url:https://x/1.jpg".bytes())));
        assert_eq!(fnv64("a".bytes()), 0xaf63_dc4c_8601_ec8c);
    }
}
