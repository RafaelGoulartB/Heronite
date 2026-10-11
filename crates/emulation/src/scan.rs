//! ROM folder scanning (port of `emulation/scan.ts`): extensions, folder
//! indicators, playlists (.m3u/.cue/.gdi), merging related files and
//! subfolders, producing a [`RomScanPreview`].

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::Result;
use heronite_data::models::{LocalGameRom, LocalMetaFile};
use heronite_data::text::locale_cmp;
use heronite_data::{json_store, DataPaths};
use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};
use regex::Regex;

use crate::names::{rom_disc_label, strip_ext, title_from_rom_path};
use crate::pathutil::{basename, ext_lower, join, normalize_key, resolve};
use crate::store::{self, resolve_template};
use crate::types::{RomScanArgs, RomScanItem, RomScanPreview};

const SKIP_DIRS: &[&str] = &[
    "bios", "firmware", "saves", "save", "savestates", "states", "cheats", "sysdata", "sys",
    "system", ".git", "media", "manuals",
];

fn skip_dir(name: &str) -> bool {
    SKIP_DIRS.contains(&name.to_lowercase().as_str())
}

/// `readdirSync` order: libuv sorts entries.
fn read_dir_sorted(dir: &str) -> Option<Vec<String>> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    Some(names)
}

/// The set of ROM paths already imported for this emulator/profile
/// (`importedRomPaths`), normalized with [`normalize_key`].
pub fn imported_rom_paths(
    metas: &LocalMetaFile,
    emulator_id: &str,
    profile_id: &str,
) -> HashSet<String> {
    let mut set = HashSet::new();
    for meta in metas.games.values() {
        if meta.emulator_id.as_deref() != Some(emulator_id) {
            continue;
        }
        if let Some(p) = meta.emulator_profile_id.as_deref().filter(|p| !p.is_empty()) {
            if p != profile_id {
                continue;
            }
        }
        for rom in meta.roms.iter().flatten() {
            set.insert(normalize_key(&rom.path));
        }
    }
    set
}

fn cue_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?i)FILE\s+"([^"]+)""#).unwrap())
}

fn quoted_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#""([^"]+)""#).unwrap())
}

/// `readPlaylistFiles`: files referenced by a .cue/.gdi/.m3u playlist.
pub fn read_playlist_files(file_path: &str) -> std::io::Result<Vec<String>> {
    let ext = ext_lower(file_path);
    let dir = crate::pathutil::dirname(file_path);
    let bytes = fs::read(file_path)?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(match ext.as_str() {
        "cue" => cue_re()
            .captures_iter(&text)
            .map(|c| join(&dir, &c[1]))
            .collect(),
        "gdi" => quoted_re()
            .captures_iter(&text)
            .map(|c| join(&dir, &c[1]))
            .collect(),
        _ => text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| if l.starts_with('/') { l.to_string() } else { join(&dir, l) })
            .collect(),
    })
}

fn walk_files(directory: &str, scan_subfolders: bool, acc: &mut Vec<String>, depth: usize) {
    let Some(entries) = read_dir_sorted(directory) else {
        return;
    };
    for name in entries {
        if name.starts_with('.') {
            continue;
        }
        let full = join(directory, &name);
        let Ok(meta) = fs::metadata(&full) else {
            continue;
        };
        if meta.is_dir() {
            if !scan_subfolders && depth > 0 {
                continue;
            }
            if skip_dir(&name) {
                continue;
            }
            walk_files(&full, scan_subfolders, acc, depth + 1);
            continue;
        }
        if meta.is_file() {
            acc.push(full);
        }
    }
}

fn walk_folders_with_indicator(
    directory: &str,
    indicator: &str,
    scan_subfolders: bool,
    acc: &mut Vec<String>,
    depth: usize,
) {
    let Some(entries) = read_dir_sorted(directory) else {
        return;
    };
    let wanted = indicator.to_lowercase();
    if let Some(file) = entries.iter().find(|n| n.to_lowercase() == wanted) {
        acc.push(join(directory, file));
        return;
    }
    if !scan_subfolders && depth > 0 {
        return;
    }
    for name in &entries {
        if name.starts_with('.') || skip_dir(name) {
            continue;
        }
        let full = join(directory, name);
        if fs::metadata(&full).map(|m| m.is_dir()).unwrap_or(false) {
            walk_folders_with_indicator(&full, indicator, scan_subfolders, acc, depth + 1);
        }
    }
}

fn to_scan_item(title: String, roms: Vec<LocalGameRom>, imported: &HashSet<String>) -> RomScanItem {
    let already = roms.iter().all(|r| imported.contains(&normalize_key(&r.path)));
    RomScanItem {
        title,
        roms,
        already_imported: already,
        import: !already,
    }
}

/// Scan options of `scanRomDirectory`.
#[derive(Debug, Clone, Copy)]
pub struct ScanOptions {
    pub scan_subfolders: bool,
    pub merge_related_files: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            scan_subfolders: true,
            merge_related_files: true,
        }
    }
}

/// `scanRomDirectory` with the already-imported set supplied by the caller.
pub fn scan_rom_directory(
    emulator: &UserEmulator,
    profile: &UserEmulatorProfile,
    directory: &str,
    options: ScanOptions,
    imported: &HashSet<String>,
) -> RomScanPreview {
    let directory = resolve(directory);
    let template = resolve_template(emulator, profile);
    let mut preview = RomScanPreview {
        scanner_id: None,
        emulator_id: emulator.id.clone(),
        emulator_name: emulator.name.clone(),
        profile_id: profile.id.clone(),
        profile_name: profile.name.clone(),
        platform_name: template.platform_name.clone(),
        directory: directory.clone(),
        games: vec![],
        errors: vec![],
    };
    if !Path::new(&directory).exists() {
        preview.errors.push(format!("Folder does not exist: {directory}"));
        return preview;
    }

    let extensions: HashSet<String> = template
        .extensions
        .iter()
        .map(|e| e.trim_start_matches('.').to_lowercase())
        .collect();
    let playlists: HashSet<String> = template
        .playlist_extensions
        .clone()
        .unwrap_or_else(|| vec!["cue".into(), "m3u".into(), "gdi".into()])
        .iter()
        .map(|e| e.to_lowercase())
        .collect();

    let mut files = Vec::new();
    match template.folder_indicator.as_deref() {
        Some(ind) => {
            walk_folders_with_indicator(&directory, ind, options.scan_subfolders, &mut files, 0)
        }
        None => walk_files(&directory, options.scan_subfolders, &mut files, 0),
    }

    let mut skip_children: HashSet<String> = HashSet::new();
    let mut candidates: Vec<(String, String)> = Vec::new();
    for file in files {
        let ext = ext_lower(&file);
        if playlists.contains(&ext) && extensions.contains(&ext) {
            match read_playlist_files(&file) {
                Ok(children) => {
                    for c in children {
                        skip_children.insert(normalize_key(&c));
                    }
                }
                Err(_) => preview.errors.push(format!("Could not read playlist {file}")),
            }
        }
        if !extensions.contains(&ext) && template.folder_indicator.is_none() {
            continue;
        }
        let title = title_from_rom_path(&basename(&file));
        candidates.push((file, title));
    }

    let filtered = candidates
        .into_iter()
        .filter(|(p, _)| !skip_children.contains(&normalize_key(p)));

    // Insertion-ordered groups.
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<LocalGameRom>> = HashMap::new();
    for (path, title) in filtered {
        let disc = rom_disc_label(&strip_ext(&basename(&path)));
        let rom = LocalGameRom {
            name: Some(disc.filter(|d| !d.is_empty()).unwrap_or_else(|| title.clone())),
            path: path.clone(),
            extra: Default::default(),
        };
        let key = if options.merge_related_files {
            title.to_lowercase()
        } else {
            path
        };
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_default().push(rom);
    }

    let mut games: Vec<RomScanItem> = order
        .into_iter()
        .map(|k| {
            let roms = groups.remove(&k).unwrap_or_default();
            let title = title_from_rom_path(&basename(&roms[0].path));
            to_scan_item(title, roms, imported)
        })
        .collect();
    games.sort_by(|a, b| locale_cmp(&a.title, &b.title));
    preview.games = games;
    preview
}

/// `previewRomScan`: looks up the emulator/profile and what is already
/// imported on disk.
pub fn preview_rom_scan(paths: &DataPaths, args: &RomScanArgs) -> Result<RomScanPreview> {
    let blank = |emulator_name: &str, profile_name: &str, error: &str| RomScanPreview {
        scanner_id: None,
        emulator_id: args.emulator_id.clone(),
        emulator_name: emulator_name.into(),
        profile_id: args.profile_id.clone(),
        profile_name: profile_name.into(),
        platform_name: None,
        directory: args.directory.clone(),
        games: vec![],
        errors: vec![error.into()],
    };
    let Some(emulator) = store::get_user_emulator(paths, &args.emulator_id)? else {
        return Ok(blank("", "", "Emulator not found. Add it in Collection settings first."));
    };
    let Some(profile) = emulator.profile(&args.profile_id).cloned() else {
        return Ok(blank(&emulator.name, "", "Emulator profile not found."));
    };
    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let imported = imported_rom_paths(&metas, &emulator.id, &profile.id);
    Ok(scan_rom_directory(
        &emulator,
        &profile,
        &args.directory,
        ScanOptions {
            scan_subfolders: args.scan_subfolders != Some(false),
            merge_related_files: args.merge_related_files != Some(false),
        },
        &imported,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, write};

    fn emu(def: &str, profile_def: &str, args: &str, ext: &[&str]) -> (UserEmulator, UserEmulatorProfile) {
        let profile = UserEmulatorProfile {
            id: "p1".into(),
            definition_profile_id: Some(profile_def.into()),
            name: "Profile".into(),
            arguments: args.into(),
            extensions: ext.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        let e = UserEmulator {
            id: "e1".into(),
            definition_id: def.into(),
            name: "Emu".into(),
            executable: "/x/emu".into(),
            launch_kind: "native".into(),
            profiles: vec![profile.clone()],
            ..Default::default()
        };
        (e, profile)
    }

    fn titles(p: &RomScanPreview) -> Vec<String> {
        p.games.iter().map(|g| g.title.clone()).collect()
    }

    #[test]
    fn scans_extensions_subfolders_and_skips_dirs() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        create_dir_all(root.join("Sub/Deep")).unwrap();
        create_dir_all(root.join("bios")).unwrap();
        create_dir_all(root.join(".hidden")).unwrap();
        write(root.join("Zelda (USA).iso"), "x").unwrap();
        write(root.join("notes.txt"), "x").unwrap();
        write(root.join("Sub/Mario (EU).ISO"), "x").unwrap();
        write(root.join("Sub/Deep/Deeper.iso"), "x").unwrap();
        write(root.join("bios/scph.iso"), "x").unwrap();
        write(root.join(".hidden/h.iso"), "x").unwrap();
        let (e, p) = emu("custom", "custom", "{ImagePath}", &["iso"]);
        let all = scan_rom_directory(&e, &p, root.to_str().unwrap(), ScanOptions::default(), &HashSet::new());
        assert_eq!(titles(&all), ["Deeper", "Mario", "Zelda"]);
        assert!(all.errors.is_empty());
        // Non recursive still looks one level into subfolders (as Heronite).
        let flat = scan_rom_directory(
            &e,
            &p,
            root.to_str().unwrap(),
            ScanOptions { scan_subfolders: false, merge_related_files: true },
            &HashSet::new(),
        );
        assert_eq!(titles(&flat), ["Mario", "Zelda"]);
    }

    #[test]
    fn merges_discs_and_respects_merge_flag() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        write(root.join("FF7 (USA) (Disc 1).iso"), "x").unwrap();
        write(root.join("FF7 (USA) (Disc 2).iso"), "x").unwrap();
        write(root.join("Other.iso"), "x").unwrap();
        let (e, p) = emu("custom", "custom", "{ImagePath}", &["iso"]);
        let merged = scan_rom_directory(&e, &p, root.to_str().unwrap(), ScanOptions::default(), &HashSet::new());
        assert_eq!(titles(&merged), ["FF7", "Other"]);
        let ff7 = &merged.games[0];
        assert_eq!(ff7.roms.len(), 2);
        assert_eq!(ff7.roms[0].name.as_deref(), Some("Disc 1"));
        assert_eq!(ff7.roms[1].name.as_deref(), Some("Disc 2"));
        let split = scan_rom_directory(
            &e,
            &p,
            root.to_str().unwrap(),
            ScanOptions { scan_subfolders: true, merge_related_files: false },
            &HashSet::new(),
        );
        assert_eq!(split.games.len(), 3);
    }

    #[test]
    fn playlists_hide_their_tracks() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        write(root.join("Game (USA).cue"), "FILE \"Game (USA) (Track 1).bin\" BINARY\n  TRACK 01 MODE2/2352\nFILE \"Game (USA) (Track 2).bin\" BINARY\n").unwrap();
        write(root.join("Game (USA) (Track 1).bin"), "x").unwrap();
        write(root.join("Game (USA) (Track 2).bin"), "x").unwrap();
        write(root.join("Multi.m3u"), "# comment\nMulti (Disc 1).chd\nMulti (Disc 2).chd\n").unwrap();
        write(root.join("Multi (Disc 1).chd"), "x").unwrap();
        write(root.join("Multi (Disc 2).chd"), "x").unwrap();
        let (e, p) = emu("duckstation", "ps1", "", &["cue", "bin", "chd", "m3u"]);
        let r = scan_rom_directory(&e, &p, root.to_str().unwrap(), ScanOptions::default(), &HashSet::new());
        assert_eq!(titles(&r), ["Game", "Multi"]);
        assert_eq!(r.games[0].roms.len(), 1);
        assert!(r.games[0].roms[0].path.ends_with(".cue"));
        assert!(r.games[1].roms[0].path.ends_with("Multi.m3u"));
        assert_eq!(r.platform_name.as_deref(), Some("PlayStation"));
    }

    #[test]
    fn folder_indicator_finds_game_folders() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        create_dir_all(root.join("God of War III/PS3_GAME/USRDIR")).unwrap();
        write(root.join("God of War III/PS3_GAME/USRDIR/EBOOT.BIN"), "x").unwrap();
        create_dir_all(root.join("Nope")).unwrap();
        write(root.join("Nope/readme.txt"), "x").unwrap();
        let (mut e, mut p) = emu("rpcs3", "ps3", "{ImagePath}", &["iso", "pkg"]);
        p.folder_indicator = Some("EBOOT.BIN".into());
        e.profiles = vec![p.clone()];
        let r = scan_rom_directory(&e, &p, root.to_str().unwrap(), ScanOptions::default(), &HashSet::new());
        assert_eq!(r.games.len(), 1);
        assert_eq!(r.games[0].title, "EBOOT");
        assert!(r.games[0].roms[0].path.ends_with("USRDIR/EBOOT.BIN"));
    }

    #[test]
    fn marks_already_imported_and_reports_missing_folder() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        write(root.join("A.iso"), "x").unwrap();
        write(root.join("B.iso"), "x").unwrap();
        let (e, p) = emu("custom", "custom", "{ImagePath}", &["iso"]);
        let mut imported = HashSet::new();
        imported.insert(normalize_key(root.join("A.iso").to_str().unwrap()));
        let r = scan_rom_directory(&e, &p, root.to_str().unwrap(), ScanOptions::default(), &imported);
        assert!(r.games[0].already_imported && !r.games[0].import);
        assert!(!r.games[1].already_imported && r.games[1].import);
        let missing = scan_rom_directory(&e, &p, "/nonexistent/zzz", ScanOptions::default(), &imported);
        assert_eq!(missing.errors, ["Folder does not exist: /nonexistent/zzz"]);
    }

    #[test]
    fn preview_without_emulator_has_error() {
        let d = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(d.path());
        let r = preview_rom_scan(
            &paths,
            &RomScanArgs { emulator_id: "x".into(), profile_id: "y".into(), directory: "/r".into(), ..Default::default() },
        )
        .unwrap();
        assert_eq!(r.errors, ["Emulator not found. Add it in Collection settings first."]);
        assert_eq!(r.directory, "/r");
    }
}
