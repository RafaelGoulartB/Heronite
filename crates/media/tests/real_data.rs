//! Read-only checks against the real captures folder and a copy of the real
//! Heroic data. Skipped (with a note) when those are not present.
//!
//! `HERONITE_FIXTURE` (default `/tmp/heronite-fixture`) is the Heroic data copy
//! and `HERONITE_CAPTURES` (default: the folder from its `settings.json`) the
//! captures folder. Nothing is ever written next to the originals.

use heronite_data::{HeroicPaths, Library};
use heronite_media::list::{list_screenshots, GameQuery};
use heronite_media::thumbnails::{cache_key, ThumbnailCache};
use std::path::{Path, PathBuf};

fn fixture() -> Option<HeroicPaths> {
    let root = std::env::var_os("HERONITE_FIXTURE").map(PathBuf::from).unwrap_or_else(|| "/tmp/heronite-fixture".into());
    root.is_dir().then(|| HeroicPaths::at(root))
}

fn captures(lib: &Library) -> Option<PathBuf> {
    let dir = std::env::var_os("HERONITE_CAPTURES")
        .map(PathBuf::from)
        .or_else(|| lib.settings.screenshots_folder().map(Path::to_path_buf))?;
    dir.is_dir().then_some(dir)
}

#[test]
fn fixture_games_against_the_real_captures_folder() {
    let Some(paths) = fixture() else { return eprintln!("no fixture; skipped") };
    let lib = Library::load(&paths).unwrap();
    let Some(dir) = captures(&lib) else { return eprintln!("no captures folder; skipped") };

    let folders = heronite_media::list::list_game_folders(&dir);
    println!("captures: {} game folders", folders.len());
    let mut matched = Vec::new();
    let mut claimed: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for game in &lib.games {
        let q = GameQuery::from_game(game);
        let r = list_screenshots(&dir, &q);
        if let Some(folder) = &r.matched_folder {
            claimed.entry(folder.clone()).or_default().push(game.title.clone());
            matched.push((game.title.clone(), folder.clone(), r.items.len()));
            // Ordering invariant: oldest first.
            assert!(r.items.windows(2).all(|w| w[0].taken_at <= w[1].taken_at));
            assert!(r.items.iter().all(|i| i.path.is_file() && i.id.starts_with(&format!("{folder}/"))));
        }
    }
    println!("games: {}  matched: {}", lib.games.len(), matched.len());
    for (title, folder, n) in &matched {
        println!("  {title:<48} -> {folder:<42} {n:>4} screenshots");
    }
    let unmatched: Vec<_> = folders.iter().filter(|f| !claimed.contains_key(*f)).collect();
    println!("folders with no game: {unmatched:?}");
    for (folder, games) in &claimed {
        if games.len() > 1 {
            println!("  folder {folder:?} claimed by several games: {games:?}");
        }
    }
    let total: usize = matched.iter().map(|m| m.2).sum();
    println!("total screenshots reachable: {total}");
    assert!(!matched.is_empty(), "at least one real game should match a folder");
}

/// The real Electron cache names must be derivable from their sidecars.
#[test]
fn cache_key_agrees_with_the_real_heronite_cache() {
    let dir = dirs_config().join("heroic/local_library/screenshot-thumbnails");
    let Ok(rd) = std::fs::read_dir(&dir) else { return eprintln!("no real thumbnail cache; skipped") };
    let (mut checked, mut bad) = (0, 0);
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".jpg.json") else { continue };
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let src = v["sourcePath"].as_str().unwrap();
        checked += 1;
        if cache_key(Path::new(src)) != stem {
            bad += 1;
        }
    }
    println!("cache keys checked: {checked}, mismatching: {bad}");
    assert!(checked == 0 || bad == 0, "{bad}/{checked} keys differ from Heronite's");
}

fn dirs_config() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".config"))
}

/// Our previews of real screenshots (written to a tempdir) look like Heronite's.
#[test]
fn previews_resemble_the_real_electron_ones() {
    let real_cache = dirs_config().join("heroic/local_library/screenshot-thumbnails");
    let Ok(rd) = std::fs::read_dir(&real_cache) else { return eprintln!("no real thumbnail cache; skipped") };
    let tmp = tempfile::tempdir().unwrap();
    let ours = ThumbnailCache::new(tmp.path().join("cache"), 50.0);
    let mut compared = 0;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".jpg.json") else { continue };
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(e.path()).unwrap()).unwrap();
        let src = PathBuf::from(v["sourcePath"].as_str().unwrap());
        if !src.is_file() {
            continue;
        }
        let theirs = image::open(real_cache.join(format!("{stem}.jpg"))).unwrap().to_rgb8();
        let mine = ours.thumbnail(&src).unwrap();
        assert_eq!((mine.width, mine.height), theirs.dimensions(), "{}", src.display());
        // Mean absolute difference per channel (JPEG q60 + different resampler).
        let mut sum = 0u64;
        for (i, p) in theirs.pixels().enumerate() {
            let m = &mine.bgra[i * 4..i * 4 + 3];
            sum += (p[0] as i32 - m[2] as i32).unsigned_abs() as u64
                + (p[1] as i32 - m[1] as i32).unsigned_abs() as u64
                + (p[2] as i32 - m[0] as i32).unsigned_abs() as u64;
        }
        let mad = sum as f64 / (theirs.width() * theirs.height() * 3) as f64;
        println!("{:<70} mean abs diff {mad:.2}", src.file_name().unwrap().to_string_lossy());
        assert!(mad < 12.0, "{}: {mad}", src.display());
        compared += 1;
        if compared >= 6 {
            break;
        }
    }
    println!("compared {compared}");
}
