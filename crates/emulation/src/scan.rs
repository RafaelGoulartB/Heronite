//! ROM folder scanning (port of `emulation/scan.ts`): extensions, folder
//! indicators, playlists (.m3u/.cue/.gdi), merging related files and
//! subfolders, producing a [`RomScanPreview`].

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::Result;
use heronite_data::models::{LocalGameMeta, LocalGameRom, LocalMetaFile};
use heronite_data::text::locale_cmp;
use heronite_data::{json_store, DataPaths};
use heronite_i18n::tr;
use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};
use regex::Regex;

use crate::disc_id::read_sfo;
use crate::names::{name_without_disc, rom_disc_label, sanitize_rom_name, strip_ext, title_from_rom_path};
use crate::pathutil::dirname;
use crate::pathutil::{basename, ext_lower, is_absolute, join, normalize_key, resolve};
use crate::store::{self, resolve_template};
use crate::types::{RomScanArgs, RomScanItem, RomScanPreview};

const SKIP_DIRS: &[&str] =
    &["bios", "firmware", "saves", "save", "savestates", "states", "cheats", "sysdata", "sys", "system", ".git", "media", "manuals"];

fn skip_dir(name: &str) -> bool {
    SKIP_DIRS.contains(&name.to_lowercase().as_str())
}

/// `readdirSync` order: libuv sorts entries.
fn read_dir_sorted(dir: &str) -> Option<Vec<String>> {
    let mut names: Vec<String> =
        fs::read_dir(dir).ok()?.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    Some(names)
}

/// Whether an imported game was imported with this emulator/profile.
pub fn imported_with(meta: &LocalGameMeta, emulator_id: &str, profile_id: &str) -> bool {
    meta.emulator_id.as_deref() == Some(emulator_id)
        && meta.emulator_profile_id.as_deref().filter(|p| !p.is_empty()).is_none_or(|p| p == profile_id)
}

/// Whether an imported game's emulator no longer exists (`live` has the ids
/// of those that do). Its ROMs are offered again by the next scan, and
/// importing them re-links the game (playtime, status, notes kept) instead of
/// adding it twice: removing an emulator and adding it back loses nothing.
pub fn orphaned(meta: &LocalGameMeta, live: &HashSet<String>) -> bool {
    meta.emulator_id.as_deref().is_none_or(|id| !live.contains(id))
}

/// The ids of the emulators that exist now (see [`orphaned`]).
pub fn live_emulator_ids(paths: &DataPaths) -> Result<HashSet<String>> {
    Ok(store::list_user_emulators(paths)?.into_iter().map(|e| e.id).collect())
}

/// The set of ROM paths already imported for this emulator/profile
/// (`importedRomPaths`), normalized with [`normalize_key`].
pub fn imported_rom_paths(metas: &LocalMetaFile, emulator_id: &str, profile_id: &str) -> HashSet<String> {
    let mut set = HashSet::new();
    for meta in metas.games.values() {
        if !imported_with(meta, emulator_id, profile_id) {
            continue;
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
        "cue" => cue_re().captures_iter(&text).map(|c| join(&dir, &c[1])).collect(),
        "gdi" => quoted_re().captures_iter(&text).map(|c| join(&dir, &c[1])).collect(),
        _ => text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| if is_absolute(l) { l.to_string() } else { join(&dir, l) })
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

/// Game folders (the ones holding `indicator`) and, next to them, the files
/// with a bootable image extension. A game folder is not entered.
fn walk_folders_with_indicator(
    directory: &str,
    indicator: &str,
    extensions: &HashSet<String>,
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
    for name in &entries {
        if name.starts_with('.') || skip_dir(name) {
            continue;
        }
        let full = join(directory, name);
        let Ok(meta) = fs::metadata(&full) else {
            continue;
        };
        if meta.is_file() {
            if extensions.contains(&ext_lower(name)) {
                acc.push(full);
            }
        } else if meta.is_dir() && (scan_subfolders || depth == 0) {
            walk_folders_with_indicator(&full, indicator, extensions, scan_subfolders, acc, depth + 1);
        }
    }
}

/// Folders that only hold a game's files, never name it.
const CONTAINER_DIRS: &[&str] =
    &["usrdir", "ps3_game", "ps3_extra", "sce_sys", "sce_module", "app", "ux0", "game", "games", "dev_hdd0", "dev_flash", "bin"];

/// Folders that are part of a game's layout, never a game's own folder: when
/// they are all there is between the indicator and the scan folder, the scan
/// folder is the game.
const STRUCTURAL_DIRS: &[&str] = &["usrdir", "ps3_game", "ps3_extra", "sce_sys", "sce_module"];

/// Packages the emulators install instead of booting (game, update and DLC
/// `.pkg` of RPCS3 and shadPS4, Vita3K's `.vpk`): never listed as games.
const INSTALLER_EXTENSIONS: &[&str] = &["pkg", "vpk"];

/// The entries of `dir` named `names` (without case), from one listing.
fn find_entries_ci(dir: &str, names: &[&str]) -> Vec<Option<String>> {
    let mut found = vec![None; names.len()];
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(i) = names.iter().position(|n| name.eq_ignore_ascii_case(n)) {
            found[i].get_or_insert(join(dir, &name));
        }
    }
    found
}

/// The first 64 KiB of a `PARAM.SFO`.
fn read_sfo_file(path: &str) -> Option<Vec<u8>> {
    let mut data = Vec::new();
    fs::File::open(path).ok()?.take(64 * 1024).read_to_end(&mut data).ok()?;
    Some(data)
}

/// The `TITLE` of the `PARAM.SFO` in `dir`, in the folder itself (Vita3K,
/// shadPS4: `sce_sys/param.sfo`) or in its `PS3_GAME` / `sce_sys` subfolder.
fn dir_sfo_title(dir: &str) -> Option<String> {
    let [sfo, ps3_game, sce_sys] = <[Option<String>; 3]>::try_from(find_entries_ci(dir, &["PARAM.SFO", "PS3_GAME", "sce_sys"])).ok()?;
    let nested = [ps3_game, sce_sys].into_iter().flatten().filter_map(|sub| find_entries_ci(&sub, &["PARAM.SFO"]).remove(0));
    for path in sfo.into_iter().chain(nested) {
        let title = read_sfo_file(&path).and_then(|data| read_sfo(&data).get("TITLE").cloned());
        let title = title.map(|t| t.replace(['\u{2122}', '\u{00AE}'], "").split_whitespace().collect::<Vec<_>>().join(" "));
        if let Some(title) = title.filter(|t| !t.is_empty()) {
            return Some(title);
        }
    }
    None
}

/// The folders from the one holding `indicator` up to `game`, the folder of
/// its game, never above it (a `PARAM.SFO` further up is not this game's).
fn folders_to_game(indicator: &str, game: &str) -> Vec<String> {
    let game_key = normalize_key(game);
    let mut dir = dirname(indicator);
    let mut dirs = vec![dir.clone()];
    while normalize_key(&dir) != game_key && dirname(&dir) != dir {
        dir = dirname(&dir);
        dirs.push(dir.clone());
    }
    dirs
}

/// The folder of the game that has `indicator`: the closest folder below
/// `root` that is not a container (`USRDIR`, `PS3_GAME`, `app`...), else the one
/// right below `root`. `root` itself when the indicator is directly in it or
/// only the folders of a game's layout (`USRDIR`, `PS3_GAME`) lie between: the
/// scan was pointed at the game.
fn game_folder(indicator: &str, root: &str) -> String {
    let root_key = normalize_key(root);
    let mut chain = vec![];
    let mut dir = dirname(indicator);
    while normalize_key(&dir) != root_key && dirname(&dir) != dir {
        chain.push(dir.clone());
        dir = dirname(&dir);
    }
    let named = |d: &String, names: &[&str]| names.contains(&basename(d).to_lowercase().as_str());
    if chain.iter().all(|d| named(d, STRUCTURAL_DIRS)) {
        return root.to_string();
    }
    chain.iter().find(|d| !named(d, CONTAINER_DIRS)).or(chain.last()).cloned().unwrap_or_else(|| root.to_string())
}

/// Title of the game that has the folder indicator file `path`: its
/// `PARAM.SFO`, else its folder (all `EBOOT.BIN`s are called alike).
fn indicator_title(path: &str, root: &str) -> String {
    let game = game_folder(path, root);
    // `USRDIR` holds the game's files, thousands of them, and never its SFO.
    let sfo = folders_to_game(path, &game).iter().filter(|d| !basename(d).eq_ignore_ascii_case("usrdir")).find_map(|d| dir_sfo_title(d));
    let name = basename(&game);
    sfo.unwrap_or_else(|| Some(sanitize_rom_name(&name)).filter(|t| !t.is_empty()).unwrap_or(name))
}

/// A file the scan may list as a game.
struct Candidate {
    path: String,
    title: String,
    /// The title when another game has the same one.
    distinct: String,
    /// The indicator file of a game folder (`EBOOT.BIN`), not a ROM file.
    game_folder: bool,
}

/// The files of one game.
struct Group {
    title: String,
    distinct: String,
    roms: Vec<LocalGameRom>,
}

fn to_scan_item(title: String, roms: Vec<LocalGameRom>, imported: &HashSet<String>) -> RomScanItem {
    let already = roms.iter().all(|r| imported.contains(&normalize_key(&r.path)));
    RomScanItem { title, roms, already_imported: already, import: !already }
}

/// Scan options of `scanRomDirectory`.
#[derive(Debug, Clone, Copy)]
pub struct ScanOptions {
    pub scan_subfolders: bool,
    pub merge_related_files: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self { scan_subfolders: true, merge_related_files: true }
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
        preview.errors.push(tr!("Folder does not exist: {directory}", directory = directory.as_str()));
        return preview;
    }

    let extensions: HashSet<String> = template.extensions.iter().map(|e| e.trim_start_matches('.').to_lowercase()).collect();
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
            let images: HashSet<String> = extensions.iter().filter(|e| !INSTALLER_EXTENSIONS.contains(&e.as_str())).cloned().collect();
            walk_folders_with_indicator(&directory, ind, &images, options.scan_subfolders, &mut files, 0)
        }
        None => walk_files(&directory, options.scan_subfolders, &mut files, 0),
    }
    let indicator = template.folder_indicator.as_deref().map(str::to_lowercase);

    let mut skip_children: HashSet<String> = HashSet::new();
    let mut candidates: Vec<Candidate> = Vec::new();
    for file in files {
        let ext = ext_lower(&file);
        if playlists.contains(&ext) && extensions.contains(&ext) {
            match read_playlist_files(&file) {
                Ok(children) => {
                    for c in children {
                        skip_children.insert(normalize_key(&c));
                    }
                }
                Err(_) => preview.errors.push(tr!("Could not read playlist {file}", file = file.as_str())),
            }
        }
        let is_game_folder = indicator.as_deref().is_some_and(|i| basename(&file).to_lowercase() == i);
        if is_game_folder {
            let title = indicator_title(&file, &directory);
            let folder = basename(&game_folder(&file, &directory));
            let distinct = if folder == title { title.clone() } else { format!("{title} ({folder})") };
            candidates.push(Candidate { path: file, title, distinct, game_folder: true });
        } else if extensions.contains(&ext) {
            let title = title_from_rom_path(&basename(&file));
            let distinct = name_without_disc(&basename(&file));
            candidates.push(Candidate { path: file, title, distinct, game_folder: false });
        }
    }

    // An image next to a game folder of the same name is that game, not another.
    let mut folders: HashMap<String, Vec<String>> = HashMap::new();
    for c in candidates.iter().filter(|c| c.game_folder) {
        folders.entry(c.title.to_lowercase()).or_default().push(c.path.clone());
    }
    let filtered = candidates.into_iter().filter(|c| {
        !skip_children.contains(&normalize_key(&c.path))
            && (c.game_folder
                || !folders.get(&c.title.to_lowercase()).is_some_and(|games| {
                    let dir = dirname(&c.path);
                    games.iter().any(|game| Path::new(game).starts_with(&dir))
                }))
    });

    // Insertion-ordered groups.
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Group> = HashMap::new();
    for Candidate { path, title, distinct, .. } in filtered {
        let disc = rom_disc_label(&strip_ext(&basename(&path)));
        let rom = LocalGameRom {
            name: Some(disc.filter(|d| !d.is_empty()).unwrap_or_else(|| title.clone())),
            path: path.clone(),
            extra: Default::default(),
        };
        // Discs of one game: same folder, same name but for the disc.
        let key = if options.merge_related_files {
            format!("{}|{}", normalize_key(&dirname(&path)), name_without_disc(&basename(&path)).to_lowercase())
        } else {
            path
        };
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_insert_with(|| Group { title, distinct, roms: vec![] }).roms.push(rom);
    }

    let groups: Vec<Group> = order.into_iter().filter_map(|k| groups.remove(&k)).collect();
    let mut games: Vec<RomScanItem> = groups
        .iter()
        .map(|g| {
            // Two versions of a game (regions, revisions) keep what tells them apart.
            let title = if groups.iter().filter(|o| o.title == g.title).count() > 1 { g.distinct.clone() } else { g.title.clone() };
            to_scan_item(title, g.roms.clone(), imported)
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
        return Ok(blank("", "", tr!("Emulator not found. Add it in Collection settings first.")));
    };
    let Some(profile) = emulator.profile(&args.profile_id).cloned() else {
        return Ok(blank(&emulator.name, "", tr!("Emulator profile not found.")));
    };
    let metas: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let imported = imported_rom_paths(&metas, &emulator.id, &profile.id);
    Ok(scan_rom_directory(
        &emulator,
        &profile,
        &args.directory,
        ScanOptions { scan_subfolders: args.scan_subfolders != Some(false), merge_related_files: args.merge_related_files != Some(false) },
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
    fn regions_and_folders_are_not_discs() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        create_dir_all(root.join("Other")).unwrap();
        write(root.join("Zelda (USA).iso"), "x").unwrap();
        write(root.join("Zelda (Europe).iso"), "x").unwrap();
        write(root.join("Other/Zelda (USA).iso"), "x").unwrap();
        write(root.join("Solo (USA).iso"), "x").unwrap();
        let (e, p) = emu("custom", "custom", "{ImagePath}", &["iso"]);
        let scan = scan_rom_directory(&e, &p, root.to_str().unwrap(), ScanOptions::default(), &HashSet::new());
        assert_eq!(titles(&scan), ["Solo", "Zelda (Europe)", "Zelda (USA)", "Zelda (USA)"]);
        assert!(scan.games.iter().all(|g| g.roms.len() == 1));
    }

    #[test]
    fn games_of_a_removed_emulator_are_orphans() {
        let meta = |emulator: Option<&str>| LocalGameMeta {
            emulator_id: emulator.map(str::to_string),
            emulator_profile_id: Some("p1".into()),
            ..Default::default()
        };
        let live: HashSet<String> = ["new".to_string()].into();
        assert!(imported_with(&meta(Some("new")), "new", "p1"));
        assert!(!imported_with(&meta(Some("new")), "new", "p2"));
        assert!(!orphaned(&meta(Some("new")), &live));
        assert!(orphaned(&meta(Some("removed")), &live) && orphaned(&meta(None), &live));
    }

    #[test]
    fn playlists_hide_their_tracks() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        write(
            root.join("Game (USA).cue"),
            "FILE \"Game (USA) (Track 1).bin\" BINARY\n  TRACK 01 MODE2/2352\nFILE \"Game (USA) (Track 2).bin\" BINARY\n",
        )
        .unwrap();
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
        assert_eq!(r.games[0].title, "God of War III");
        assert!(Path::new(&r.games[0].roms[0].path).ends_with("USRDIR/EBOOT.BIN"));
    }

    fn sfo(entries: &[(&str, &str)]) -> Vec<u8> {
        let (mut keys, mut vals, mut index) = (Vec::new(), Vec::new(), Vec::new());
        for (k, v) in entries {
            index.extend_from_slice(&(keys.len() as u16).to_le_bytes());
            index.extend_from_slice(&0x0204u16.to_le_bytes());
            index.extend_from_slice(&((v.len() + 1) as u32).to_le_bytes());
            index.extend_from_slice(&((v.len() + 1) as u32).to_le_bytes());
            index.extend_from_slice(&(vals.len() as u32).to_le_bytes());
            keys.extend_from_slice(k.as_bytes());
            keys.push(0);
            vals.extend_from_slice(v.as_bytes());
            vals.push(0);
        }
        let key_table = 20 + index.len();
        let mut out = b"\0PSF\x01\x01\0\0".to_vec();
        out.extend_from_slice(&(key_table as u32).to_le_bytes());
        out.extend_from_slice(&((key_table + keys.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        out.extend(index);
        out.extend(keys);
        out.extend(vals);
        out
    }

    fn indicator_profile(def: &str, profile: &str, ext: &[&str], indicator: &str) -> (UserEmulator, UserEmulatorProfile) {
        let (mut e, mut p) = emu(def, profile, "{ImagePath}", ext);
        p.folder_indicator = Some(indicator.into());
        e.profiles = vec![p.clone()];
        (e, p)
    }

    #[test]
    fn game_folders_are_named_by_their_sfo_or_their_folder() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        // Named by its PARAM.SFO (title with a line break and a mark).
        create_dir_all(root.join("BLES00001/PS3_GAME/USRDIR")).unwrap();
        write(root.join("BLES00001/PS3_GAME/USRDIR/EBOOT.BIN"), "x").unwrap();
        write(root.join("BLES00001/PS3_GAME/PARAM.SFO"), sfo(&[("TITLE", "Demon's Souls\u{2122}\n"), ("TITLE_ID", "BLES00001")])).unwrap();
        // No SFO: the folder that is not a container.
        create_dir_all(root.join("Journey/PS3_GAME/USRDIR")).unwrap();
        write(root.join("Journey/PS3_GAME/USRDIR/EBOOT.BIN"), "x").unwrap();
        let (e, p) = indicator_profile("rpcs3", "ps3", &["iso", "pkg"], "EBOOT.BIN");
        let r = scan_rom_directory(&e, &p, root.to_str().unwrap(), ScanOptions::default(), &HashSet::new());
        assert_eq!(titles(&r), ["Demon's Souls", "Journey"]);

        // Vita3K / shadPS4: sce_sys/param.sfo beside a lowercase eboot.bin.
        let v = tempfile::tempdir().unwrap();
        create_dir_all(v.path().join("ux0/app/PCSE00001/sce_sys")).unwrap();
        write(v.path().join("ux0/app/PCSE00001/eboot.bin"), "x").unwrap();
        write(v.path().join("ux0/app/PCSE00001/sce_sys/param.sfo"), sfo(&[("TITLE", "Persona 4 Golden")])).unwrap();
        create_dir_all(v.path().join("ux0/app/PCSE00002")).unwrap();
        write(v.path().join("ux0/app/PCSE00002/eboot.bin"), "x").unwrap();
        let (e, p) = indicator_profile("vita3k", "vita", &["vpk"], "eboot.bin");
        let r = scan_rom_directory(&e, &p, v.path().to_str().unwrap(), ScanOptions::default(), &HashSet::new());
        assert_eq!(titles(&r), ["PCSE00002", "Persona 4 Golden"]);
    }

    #[test]
    fn game_folders_list_images_and_dumps_beside_them() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        create_dir_all(root.join("God of War III/PS3_GAME/USRDIR")).unwrap();
        write(root.join("God of War III/PS3_GAME/USRDIR/EBOOT.BIN"), "x").unwrap();
        // The same game also as an image beside its folder: one game.
        write(root.join("God of War III.iso"), "x").unwrap();
        write(root.join("Uncharted (USA).iso"), "x").unwrap();
        // Installers (games, updates, DLC) are not games to boot.
        write(root.join("Heavy Rain.pkg"), "x").unwrap();
        write(root.join("God of War III DLC.PKG"), "x").unwrap();
        write(root.join("readme.txt"), "x").unwrap();
        let (e, p) = indicator_profile("rpcs3", "ps3", &["iso", "pkg"], "EBOOT.BIN");
        let r = scan_rom_directory(&e, &p, root.to_str().unwrap(), ScanOptions::default(), &HashSet::new());
        assert_eq!(titles(&r), ["God of War III", "Uncharted"]);
        assert!(r.games[0].roms[0].path.ends_with("EBOOT.BIN"));
        // Not recursive: the root files still show, deeper folders do not.
        create_dir_all(root.join("a/b")).unwrap();
        write(root.join("a/b/Deep.iso"), "x").unwrap();
        let flat = scan_rom_directory(
            &e,
            &p,
            root.to_str().unwrap(),
            ScanOptions { scan_subfolders: false, merge_related_files: true },
            &HashSet::new(),
        );
        assert_eq!(titles(&flat), ["God of War III", "Uncharted"]);
    }

    #[test]
    fn installer_packages_are_never_listed_beside_vita_and_ps4_games() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        create_dir_all(root.join("CUSA00001")).unwrap();
        write(root.join("CUSA00001/eboot.bin"), "x").unwrap();
        write(root.join("Update.pkg"), "x").unwrap();
        write(root.join("Game.vpk"), "x").unwrap();
        for (def, profile, ext, ind) in [("shadps4", "ps4", "pkg", "eboot.bin"), ("vita3k", "vita", "vpk", "eboot.bin")] {
            let (e, p) = indicator_profile(def, profile, &[ext], ind);
            let r = scan_rom_directory(&e, &p, root.to_str().unwrap(), ScanOptions::default(), &HashSet::new());
            assert_eq!(titles(&r), ["CUSA00001"], "{def}");
        }
    }

    #[test]
    fn a_param_sfo_above_the_game_folder_does_not_title_it() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        // The scan root, and a library folder, hold an SFO that is not any game's.
        write(root.join("PARAM.SFO"), sfo(&[("TITLE", "Not A Game")])).unwrap();
        create_dir_all(root.join("Library/Journey/USRDIR")).unwrap();
        write(root.join("Library/PARAM.SFO"), sfo(&[("TITLE", "Not A Game Either")])).unwrap();
        write(root.join("Library/Journey/USRDIR/EBOOT.BIN"), "x").unwrap();
        let (e, p) = indicator_profile("rpcs3", "ps3", &["iso"], "EBOOT.BIN");
        let r = scan_rom_directory(&e, &p, root.to_str().unwrap(), ScanOptions::default(), &HashSet::new());
        assert_eq!(titles(&r), ["Journey"]);
    }

    #[test]
    fn games_in_container_folders_are_not_named_after_the_scan_root() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("ps3");
        create_dir_all(root.join("Game")).unwrap();
        create_dir_all(root.join("Bin")).unwrap();
        write(root.join("Game/EBOOT.BIN"), "x").unwrap();
        write(root.join("Bin/EBOOT.BIN"), "x").unwrap();
        let (e, p) = indicator_profile("rpcs3", "ps3", &["iso"], "EBOOT.BIN");
        let r = scan_rom_directory(&e, &p, root.to_str().unwrap(), ScanOptions::default(), &HashSet::new());
        assert_eq!(titles(&r), ["Bin", "Game"]);

        // Pointed at the game itself, the folder is the game.
        let single = scan_rom_directory(&e, &p, root.join("Game").to_str().unwrap(), ScanOptions::default(), &HashSet::new());
        assert_eq!(titles(&single), ["Game"]);
    }

    #[test]
    fn a_scan_folder_that_is_the_game_is_titled_by_its_own_sfo() {
        let d = tempfile::tempdir().unwrap();
        let (e, p) = indicator_profile("rpcs3", "ps3", &["iso"], "EBOOT.BIN");
        let scan = |dir: &Path| scan_rom_directory(&e, &p, dir.to_str().unwrap(), ScanOptions::default(), &HashSet::new());

        // dev_hdd0/game/NPUB30001: the SFO beside USRDIR.
        let game = d.path().join("NPUB30001");
        create_dir_all(game.join("USRDIR")).unwrap();
        write(game.join("USRDIR/EBOOT.BIN"), "x").unwrap();
        write(game.join("PARAM.SFO"), sfo(&[("TITLE", "Flower")])).unwrap();
        assert_eq!(titles(&scan(&game)), ["Flower"]);

        // The PS3_GAME folder of a game.
        let ps3_game = d.path().join("God of War III/PS3_GAME");
        create_dir_all(ps3_game.join("USRDIR")).unwrap();
        write(ps3_game.join("USRDIR/EBOOT.BIN"), "x").unwrap();
        write(ps3_game.join("PARAM.SFO"), sfo(&[("TITLE", "God of War III")])).unwrap();
        assert_eq!(titles(&scan(&ps3_game)), ["God of War III"]);

        // The game folder, with PS3_GAME inside; no SFO: the folder's name.
        let bare = d.path().join("Journey");
        create_dir_all(bare.join("PS3_GAME/USRDIR")).unwrap();
        write(bare.join("PS3_GAME/USRDIR/EBOOT.BIN"), "x").unwrap();
        assert_eq!(titles(&scan(&bare)), ["Journey"]);
    }

    #[test]
    fn game_folders_with_one_title_keep_their_folder_apart() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        for id in ["BLUS00001", "BLES00001"] {
            create_dir_all(root.join(id).join("USRDIR")).unwrap();
            write(root.join(id).join("USRDIR/EBOOT.BIN"), "x").unwrap();
            write(root.join(id).join("PARAM.SFO"), sfo(&[("TITLE", "Same Game")])).unwrap();
        }
        let (e, p) = indicator_profile("rpcs3", "ps3", &["iso"], "EBOOT.BIN");
        let r = scan_rom_directory(&e, &p, root.to_str().unwrap(), ScanOptions::default(), &HashSet::new());
        assert_eq!(titles(&r), ["Same Game (BLES00001)", "Same Game (BLUS00001)"]);
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
        let gone = root.join("nonexistent").join("zzz");
        let missing = scan_rom_directory(&e, &p, gone.to_str().unwrap(), ScanOptions::default(), &imported);
        assert_eq!(missing.errors, [format!("Folder does not exist: {}", gone.display())]);
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
        assert_eq!(r.errors, [tr!("Emulator not found. Add it in Collection settings first.")]);
        assert_eq!(r.directory, "/r");
    }

    #[cfg(windows)]
    #[test]
    fn windows_folders_playlists_and_keys() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        create_dir_all(root.join("PS1")).unwrap();
        write(root.join(r"PS1\Multi (Disc 1).chd"), "x").unwrap();
        write(root.join(r"PS1\Multi (Disc 2).chd"), "x").unwrap();
        // A playlist written on Windows: CRLF, one track by its full path.
        let disc2 = root.join(r"PS1\Multi (Disc 2).chd");
        write(root.join(r"PS1\Multi.m3u"), format!("Multi (Disc 1).chd\r\n{}\r\n", disc2.display())).unwrap();
        write(root.join(r"PS1\Solo (USA).chd"), "x").unwrap();
        let (e, p) = emu("duckstation", "ps1", "", &["chd", "m3u"]);
        // The folder as typed: forward slashes, another case.
        let typed = format!("{}/ps1/", root.display());
        let mut imported = HashSet::new();
        imported.insert(normalize_key(&root.join(r"PS1\SOLO (USA).CHD").to_string_lossy()));
        let r = scan_rom_directory(&e, &p, &typed, ScanOptions::default(), &imported);
        assert_eq!(titles(&r), ["Multi", "Solo"]);
        assert_eq!(r.directory, root.join("ps1").to_string_lossy());
        assert_eq!(r.games[0].roms[0].path, root.join(r"ps1\Multi.m3u").to_string_lossy());
        assert!(r.games[1].already_imported, "paths compare without case on Windows");
    }
}
