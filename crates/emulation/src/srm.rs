//! Read-only Steam ROM Manager (`userConfigurations.json`) parser discovery
//! and import into Heronite's native emulators and ROM scanners.

use std::path::Path;

use anyhow::{Context, Result};
use heronite_data::DataPaths;
use heronite_i18n::tr;
use heronite_launch::emulators::{UserEmulator, UserEmulatorProfile};
use heronite_launch::split_args;
use serde_json::Value;

use crate::catalog::{emulator_definition, emulator_definitions, guess_definition_id};
use crate::detect::{DetectEnv, SystemEnv};
use crate::glob::RomGlob;
use crate::import::run_rom_scanners;
use crate::pathutil::{basename, join, join_all, normalize_key, unique};
use crate::setup::platform_of;
use crate::store::{insert_user_emulator, list_rom_scanners, list_user_emulators, replace_user_emulator, upsert_rom_scanner};
use crate::types::{RomScanner, UpsertRomScannerArgs};

/// One importable Glob parser read from Steam ROM Manager's `userConfigurations.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrmParserItem {
    pub title: String,
    pub definition_id: String,
    pub emulator_name: String,
    pub executable: String,
    /// `'native' | 'flatpak' | 'wine'`
    pub launch_kind: String,
    pub flatpak_id: Option<String>,
    pub arguments: String,
    pub rom_directory: String,
    pub rom_glob: String,
    pub platform_id: Option<String>,
    pub platform_name: Option<String>,
    pub already_configured: bool,
    pub selected: bool,
}

/// Summary of reading a Steam ROM Manager `userConfigurations.json` file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SrmScanResult {
    pub config_path: String,
    pub parsers: Vec<SrmParserItem>,
    pub disabled_count: usize,
    pub unsupported_count: usize,
    pub errors: Vec<String>,
}

/// Outcome of importing selected SRM parsers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SrmImportResult {
    pub emulators_added: usize,
    pub scanners_added: usize,
    pub games_imported: usize,
    pub errors: Vec<String>,
}

/// Standard locations of Steam ROM Manager's `userConfigurations.json`.
pub fn default_config_paths(env: &dyn DetectEnv) -> Vec<String> {
    let home = env.home();
    if cfg!(windows) {
        let appdata = env.var("APPDATA").unwrap_or_else(|| join_all(&home, &["AppData", "Roaming"]));
        return unique([Some(join_all(&appdata, &["steam-rom-manager", "userData", "userConfigurations.json"]))]);
    }
    let xdg_config = if heronite_platform::flatpak::active() { env.var("HOST_XDG_CONFIG_HOME") } else { env.var("XDG_CONFIG_HOME") }
        .unwrap_or_else(|| join(&home, ".config"));
    unique([
        Some(join_all(&xdg_config, &["steam-rom-manager", "userData", "userConfigurations.json"])),
        Some(join_all(
            &home,
            &[".var", "app", "com.steamgriddb.steam-rom-manager", "config", "steam-rom-manager", "userData", "userConfigurations.json"],
        )),
    ])
}

/// First existing `userConfigurations.json` on this machine.
pub fn find_config_file(env: &dyn DetectEnv) -> Option<String> {
    default_config_paths(env).into_iter().find(|p| env.exists(p))
}

/// Expands a leading `~` to the user's home directory.
pub fn expand_config_path(raw: &str, env: &dyn DetectEnv) -> String {
    let trimmed = raw.trim();
    if trimmed == "~" {
        return env.home();
    }
    if let Some(rest) = trimmed.strip_prefix("~/").or_else(|| trimmed.strip_prefix("~\\")) {
        return join(&env.home(), rest);
    }
    trimmed.to_string()
}

/// Translates SRM's `${...}` argument placeholders into Heronite's `{...}`
/// variables (case-insensitively), returning `(translated, unresolved_vars)`.
pub fn translate_srm_args_checked(raw: &str) -> (String, Vec<String>) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ("{ImagePath}".into(), vec![]);
    }
    let mut out = String::with_capacity(trimmed.len());
    let mut unresolved = Vec::new();
    let mut rest = trimmed;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let token = &after[..end];
        let lower = token.to_ascii_lowercase();
        let replacement: Option<&str> = match lower.as_str() {
            "filepath" => Some("{ImagePath}"),
            "filename" => Some("{ImageName}"),
            "filewithoutext" | "filenamenoext" => Some("{ImageNameNoExt}"),
            "filedir" | "dir" => Some("{ImageDir}"),
            "exedir" => Some("{EmulatorDir}"),
            "title" => Some("{Name}"),
            "/" => Some(std::path::MAIN_SEPARATOR_STR),
            _ if lower.starts_with("os:") => {
                let parts: Vec<&str> = token[3..].split('|').collect();
                match parts.as_slice() {
                    [win, mac, linux] => {
                        if cfg!(windows) {
                            Some(*win)
                        } else if cfg!(target_os = "macos") {
                            Some(*mac)
                        } else {
                            Some(*linux)
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(rep) = replacement {
            out.push_str(rep);
        } else {
            let full = format!("${{{token}}}");
            if !unresolved.contains(&full) {
                unresolved.push(full.clone());
            }
            out.push_str(&full);
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    (out, unresolved)
}

/// Translates SRM's `${...}` argument placeholders into Heronite's `{...}` variables.
pub fn translate_srm_args(raw: &str) -> String {
    translate_srm_args_checked(raw).0
}

/// Extracts `(launch_kind, executable, flatpak_id, arguments, unresolved_vars)`
/// from an SRM executable path and argument string.
fn parse_launch(raw_exe: &str, raw_args: &str) -> (String, String, Option<String>, String, Vec<String>) {
    let exe = raw_exe.trim();
    let (translated, unresolved) = translate_srm_args_checked(raw_args);
    if !cfg!(windows) && basename(exe).eq_ignore_ascii_case("flatpak") {
        // Common SRM Flatpak form: `run <app.id> [args...]`.
        let mut parts = translated.splitn(3, char::is_whitespace);
        if parts.next().is_some_and(|w| w == "run") {
            if let Some(app_id) = parts.next().filter(|id| id.contains('.') && !id.starts_with('-')) {
                let rest = parts.next().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("{ImagePath}");
                return ("flatpak".into(), "flatpak".into(), Some(app_id.to_string()), rest.to_string(), unresolved);
            }
        }
    }
    let launch_kind = if !cfg!(windows) && exe.to_lowercase().ends_with(".exe") { "wine" } else { "native" };
    (launch_kind.into(), exe.to_string(), None, translated, unresolved)
}

/// Strips SRM's `${...}` wrapper from `steamCategory` (e.g. `"${Nintendo Switch}"` -> `"Nintendo Switch"`).
fn clean_category(val: &Value) -> Option<String> {
    let raw = match val {
        Value::String(s) => s.as_str(),
        Value::Array(arr) => arr.first().and_then(Value::as_str)?,
        _ => return None,
    };
    let trimmed = raw.trim();
    let stripped = trimmed.strip_prefix("${").and_then(|s| s.strip_suffix('}')).unwrap_or(trimmed).trim();
    (!stripped.is_empty()).then(|| stripped.to_string())
}

/// Matches a free-form system label (category, folder name, or title) to a
/// catalog `(platform_id, platform_name)`.
fn guess_platform(definition_id: &str, category: Option<&str>, rom_dir: &str, title: &str) -> (Option<String>, Option<String>) {
    let by_platform_id = |pid: &str| -> Option<(String, String)> {
        emulator_definitions()
            .iter()
            .flat_map(|d| &d.profiles)
            .find(|p| p.platform_id == pid)
            .map(|p| (p.platform_id.clone(), p.platform_name.clone()))
    };

    if let Some(cat) = category {
        let lower = cat.to_lowercase();
        if let Some(p) = emulator_definitions()
            .iter()
            .flat_map(|d| &d.profiles)
            .find(|p| p.platform_name.to_lowercase() == lower || p.name.to_lowercase() == lower)
        {
            return (Some(p.platform_id.clone()), Some(p.platform_name.clone()));
        }
        let compact: String = lower.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        if let Some(pid) = platform_of(&compact).and_then(by_platform_id) {
            return (Some(pid.0), Some(pid.1));
        }
    }

    if let Some(pid) = platform_of(&basename(rom_dir)).and_then(by_platform_id) {
        return (Some(pid.0), Some(pid.1));
    }

    if let Some(def) = emulator_definition(definition_id) {
        if def.id != "custom" && def.profiles.len() == 1 {
            let p = &def.profiles[0];
            return (Some(p.platform_id.clone()), Some(p.platform_name.clone()));
        }
        let lower_title = title.to_lowercase();
        if let Some(p) = def
            .profiles
            .iter()
            .find(|p| lower_title.contains(&p.platform_name.to_lowercase()) || lower_title.contains(&p.name.to_lowercase()))
        {
            return (Some(p.platform_id.clone()), Some(p.platform_name.clone()));
        }
    }

    (Some("other".into()), category.map(str::to_string).or_else(|| Some("Other".into())))
}

/// Reads and parses a Steam ROM Manager `userConfigurations.json` file.
pub fn load_config(config_path: &str, existing_scanners: &[RomScanner]) -> Result<SrmScanResult> {
    let expanded = expand_config_path(config_path, &SystemEnv);
    let text = std::fs::read_to_string(&expanded).with_context(|| format!("reading {expanded}"))?;
    parse_config_json(&expanded, &text, existing_scanners)
}

/// Pure parser for `userConfigurations.json` contents.
pub fn parse_config_json(config_path: &str, text: &str, existing_scanners: &[RomScanner]) -> Result<SrmScanResult> {
    let doc: Value = serde_json::from_str(text).with_context(|| format!("parsing {config_path}"))?;
    let entries = doc
        .as_array()
        .or_else(|| doc.get("userConfigurations").and_then(Value::as_array))
        .context("expected a JSON array or { \"userConfigurations\": [...] }")?;

    let mut result = SrmScanResult { config_path: config_path.to_string(), ..Default::default() };

    for entry in entries {
        let Some(obj) = entry.as_object() else { continue };
        if obj.get("disabled").and_then(Value::as_bool).unwrap_or(false) {
            result.disabled_count += 1;
            continue;
        }
        let parser_type = obj.get("parserType").and_then(Value::as_str).unwrap_or("");
        if !parser_type.eq_ignore_ascii_case("glob") {
            result.unsupported_count += 1;
            continue;
        }
        let title = obj.get("configTitle").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let rom_directory = obj.get("romDirectory").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let glob = obj
            .get("parserInputs")
            .and_then(Value::as_object)
            .and_then(|inputs| inputs.get("glob"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let raw_exe = match obj.get("executable") {
            Some(Value::String(s)) => s.trim(),
            Some(Value::Object(m)) => m.get("path").and_then(Value::as_str).unwrap_or("").trim(),
            _ => "",
        };
        if rom_directory.is_empty() || glob.is_empty() || raw_exe.is_empty() {
            let label = if title.is_empty() { tr!("Unnamed parser") } else { title.as_str() };
            result.errors.push(tr!("{label}: skipped (missing ROM folder, glob, or executable)", label = label));
            continue;
        }
        if let Err(err) = RomGlob::parse(&glob) {
            let label = if title.is_empty() { rom_directory.as_str() } else { title.as_str() };
            result.errors.push(format!("{label}: {err}"));
            continue;
        }
        let raw_args = obj.get("executableArgs").and_then(Value::as_str).unwrap_or("");
        let (launch_kind, executable, flatpak_id, arguments, unresolved_vars) = parse_launch(raw_exe, raw_args);

        let def_from_fp = flatpak_id.as_deref().map(|f| guess_definition_id(Some(f))).filter(|d| d != "custom");
        let def_from_exe = Some(guess_definition_id(Some(&basename(raw_exe)))).filter(|d| d != "custom");
        let def_from_title = Some(guess_definition_id(Some(&title))).filter(|d| d != "custom");
        let definition_id = def_from_fp.or(def_from_exe).or(def_from_title).unwrap_or_else(|| "custom".into());

        let emulator_name = emulator_definition(&definition_id)
            .filter(|d| d.id != "custom")
            .map(|d| d.name.clone())
            .or_else(|| (!title.is_empty()).then(|| title.clone()))
            .unwrap_or_else(|| basename(raw_exe));

        let category = obj.get("steamCategory").and_then(clean_category);
        let (platform_id, platform_name) = guess_platform(&definition_id, category.as_deref(), &rom_directory, &title);

        let dir_key = normalize_key(&rom_directory);
        let already_configured = existing_scanners
            .iter()
            .any(|s| normalize_key(&s.directory) == dir_key && s.rom_glob.as_deref().map(str::trim) == Some(glob.as_str()));

        let display_title = if title.is_empty() { platform_name.clone().unwrap_or_else(|| emulator_name.clone()) } else { title };
        let has_unresolved = !unresolved_vars.is_empty();
        if has_unresolved {
            let vars = unresolved_vars.join(", ");
            result.errors.push(tr!(
                "{label}: unsupported Steam ROM Manager variable(s) in arguments ({vars})",
                label = display_title.as_str(),
                vars = vars.as_str()
            ));
        }
        result.parsers.push(SrmParserItem {
            title: display_title,
            definition_id,
            emulator_name,
            executable,
            launch_kind,
            flatpak_id,
            arguments,
            rom_directory,
            rom_glob: glob,
            platform_id,
            platform_name,
            already_configured,
            selected: !already_configured && !has_unresolved,
        });
    }

    Ok(result)
}

fn canonical(path: &str) -> String {
    std::fs::canonicalize(path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| path.to_owned())
}

fn matching_emulator<'a>(existing: &'a [UserEmulator], item: &SrmParserItem) -> Option<&'a UserEmulator> {
    if item.launch_kind == "flatpak" {
        return existing.iter().find(|e| e.launch_kind == "flatpak" && e.flatpak_id == item.flatpak_id);
    }
    let key = canonical(&item.executable);
    existing
        .iter()
        .filter(|e| e.launch_kind != "flatpak")
        .find(|e| e.executable == item.executable || (!e.executable.is_empty() && canonical(&e.executable) == key))
}

/// Imports the selected Steam ROM Manager parsers as Heronite emulators and ROM
/// scanners. When `scan_now` is true, each imported folder is scanned immediately.
pub fn import_srm_parsers(paths: &DataPaths, items: &[SrmParserItem], scan_now: bool) -> Result<SrmImportResult> {
    let mut result = SrmImportResult::default();
    let mut created_scanner_ids = Vec::new();

    for item in items.iter().filter(|i| i.selected) {
        let current = list_user_emulators(paths)?;
        let (emulator_id, profile_id) = match matching_emulator(&current, item).cloned() {
            Some(mut emu) => {
                // Reuse a profile with the same platform and effective arguments, or add one.
                let item_argv = split_args(&item.arguments);
                let found = emu.profiles.iter().find(|p| p.platform_id == item.platform_id && split_args(&p.arguments) == item_argv);
                let pid = if let Some(p) = found {
                    p.id.clone()
                } else {
                    let catalog_prof = emulator_definition(&item.definition_id)
                        .and_then(|d| d.profiles.iter().find(|p| Some(p.platform_id.as_str()) == item.platform_id.as_deref()));
                    let new_prof = UserEmulatorProfile {
                        id: crate::store::new_id(),
                        definition_profile_id: catalog_prof.map(|p| p.id.clone()),
                        name: item.platform_name.clone().unwrap_or_else(|| item.title.clone()),
                        platform_id: item.platform_id.clone(),
                        platform_name: item.platform_name.clone(),
                        arguments: item.arguments.clone(),
                        extensions: catalog_prof.map(|p| p.extensions.clone()).unwrap_or_default(),
                        core_path: None,
                        folder_indicator: catalog_prof.and_then(|p| p.folder_indicator.clone()),
                        rom_glob: Some(item.rom_glob.clone()),
                        playlist_extensions: catalog_prof.and_then(|p| p.playlist_extensions.clone()),
                        working_dir: catalog_prof.and_then(|p| p.working_dir.clone()),
                        extra: Default::default(),
                    };
                    let id = new_prof.id.clone();
                    emu.profiles.push(new_prof);
                    replace_user_emulator(paths, emu.clone())?;
                    id
                };
                (emu.id, pid)
            }
            None => {
                let catalog_prof = emulator_definition(&item.definition_id)
                    .and_then(|d| d.profiles.iter().find(|p| Some(p.platform_id.as_str()) == item.platform_id.as_deref()));
                let profile = UserEmulatorProfile {
                    id: crate::store::new_id(),
                    definition_profile_id: catalog_prof.map(|p| p.id.clone()),
                    name: item.platform_name.clone().unwrap_or_else(|| item.title.clone()),
                    platform_id: item.platform_id.clone(),
                    platform_name: item.platform_name.clone(),
                    arguments: item.arguments.clone(),
                    extensions: catalog_prof.map(|p| p.extensions.clone()).unwrap_or_default(),
                    core_path: None,
                    folder_indicator: catalog_prof.and_then(|p| p.folder_indicator.clone()),
                    rom_glob: Some(item.rom_glob.clone()),
                    playlist_extensions: catalog_prof.and_then(|p| p.playlist_extensions.clone()),
                    working_dir: catalog_prof.and_then(|p| p.working_dir.clone()),
                    extra: Default::default(),
                };
                let pid = profile.id.clone();
                let inserted = insert_user_emulator(
                    paths,
                    UserEmulator {
                        id: String::new(),
                        definition_id: item.definition_id.clone(),
                        name: item.emulator_name.clone(),
                        executable: item.executable.clone(),
                        install_dir: None,
                        launch_kind: item.launch_kind.clone(),
                        flatpak_id: item.flatpak_id.clone(),
                        playnite_id: None,
                        ludusavi_entry: None,
                        profiles: vec![profile],
                        extra: Default::default(),
                    },
                )?;
                result.emulators_added += 1;
                (inserted.id, pid)
            }
        };

        let existing_scanners = list_rom_scanners(paths)?;
        let existing_scanner = existing_scanners.iter().find(|s| {
            s.emulator_id == emulator_id && s.profile_id == profile_id && normalize_key(&s.directory) == normalize_key(&item.rom_directory)
        });
        let scanner = upsert_rom_scanner(
            paths,
            UpsertRomScannerArgs {
                id: existing_scanner.map(|s| s.id.clone()),
                name: if existing_scanner.is_some() { None } else { Some(item.title.clone()) },
                emulator_id,
                profile_id,
                directory: item.rom_directory.clone(),
                scan_subfolders: existing_scanner.is_none().then_some(true),
                merge_related_files: existing_scanner.is_none().then_some(true),
                auto_scan_on_refresh: existing_scanner.is_none().then_some(true),
                rom_glob: Some(item.rom_glob.clone()),
            },
        )?;
        if existing_scanner.is_none() {
            result.scanners_added += 1;
        }
        created_scanner_ids.push(scanner.id);
    }

    if scan_now {
        for sid in created_scanner_ids {
            match run_rom_scanners(paths, Some(&sid)) {
                Ok(report) => {
                    result.games_imported += report.imported;
                    result.errors.extend(report.errors);
                }
                Err(e) => result.errors.push(format!("{e:#}")),
            }
        }
    }

    Ok(result)
}

/// Checks whether a path looks like an existing file on disk.
pub fn config_exists(path: &str) -> bool {
    let expanded = expand_config_path(path, &SystemEnv);
    !expanded.is_empty() && Path::new(&expanded).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, write};

    #[test]
    fn parses_srm_array_and_translates_args_and_flatpak() {
        let json = r#"[
            {
                "parserType": "Glob",
                "configTitle": "Nintendo Switch - Eden",
                "steamCategory": "${Nintendo Switch}",
                "romDirectory": "/roms/switch",
                "executable": { "path": "/opt/Eden-Linux.AppImage" },
                "executableArgs": "-f -g \"${filePath}\"",
                "parserInputs": { "glob": "${title}/@(*[BASE]*.nsp|*.xci)" },
                "disabled": false
            },
            {
                "parserType": "Glob",
                "configTitle": "PS2 - PCSX2",
                "steamCategory": "${PlayStation 2}",
                "romDirectory": "/roms/ps2",
                "executable": { "path": "/usr/bin/flatpak" },
                "executableArgs": "run net.pcsx2.PCSX2 -fullscreen -- \"${filePath}\"",
                "parserInputs": { "glob": "${title}.@(iso|chd)" },
                "disabled": false
            },
            {
                "parserType": "Steam",
                "configTitle": "Steam Games",
                "disabled": false
            },
            {
                "parserType": "Glob",
                "configTitle": "Disabled Parser",
                "romDirectory": "/roms/old",
                "executable": { "path": "/bin/emu" },
                "parserInputs": { "glob": "*.rom" },
                "disabled": true
            }
        ]"#;

        let scan = parse_config_json("userConfigurations.json", json, &[]).unwrap();
        assert_eq!(scan.disabled_count, 1);
        assert_eq!(scan.unsupported_count, 1);
        assert!(scan.errors.is_empty());
        assert_eq!(scan.parsers.len(), 2);

        let eden = &scan.parsers[0];
        assert_eq!(eden.definition_id, "eden");
        assert_eq!(eden.emulator_name, "Eden");
        assert_eq!(eden.launch_kind, "native");
        assert_eq!(eden.arguments, "-f -g \"{ImagePath}\"");
        assert_eq!(eden.rom_glob, "${title}/@(*[BASE]*.nsp|*.xci)");
        assert_eq!(eden.platform_id.as_deref(), Some("nintendo_switch"));
        assert!(eden.selected && !eden.already_configured);

        let pcsx2 = &scan.parsers[1];
        assert_eq!(pcsx2.definition_id, "pcsx2");
        if !cfg!(windows) {
            assert_eq!(pcsx2.launch_kind, "flatpak");
            assert_eq!(pcsx2.flatpak_id.as_deref(), Some("net.pcsx2.PCSX2"));
            assert_eq!(pcsx2.arguments, "-fullscreen -- \"{ImagePath}\"");
        }
        assert_eq!(pcsx2.platform_id.as_deref(), Some("sony_playstation2"));
    }

    #[test]
    fn imports_srm_parsers_and_scans_roms_with_glob() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("data"));
        let roms = dir.path().join("roms/switch");
        create_dir_all(roms.join("Zelda TOTK")).unwrap();
        write(roms.join("Zelda TOTK/0100F2C0 [BASE].nsp"), "x").unwrap();
        write(roms.join("Zelda TOTK/0100F2C0 [UPD].nsp"), "x").unwrap();
        write(roms.join("Zelda TOTK/0100F2C0 [DLC].nsp"), "x").unwrap();
        create_dir_all(roms.join("Super Mario Wonder")).unwrap();
        write(roms.join("Super Mario Wonder/Wonder.xci"), "x").unwrap();

        let cfg = serde_json::json!([{
            "parserType": "Glob",
            "configTitle": "Nintendo Switch - Eden",
            "steamCategory": "${Nintendo Switch}",
            "romDirectory": roms.to_string_lossy(),
            "executable": { "path": "/opt/Eden.AppImage" },
            "executableArgs": "-f -g \"${filePath}\"",
            "parserInputs": { "glob": "${title}/@(*[BASE]*.nsp|*.xci)" },
            "disabled": false
        }]);
        let scan = parse_config_json("srm.json", &cfg.to_string(), &[]).unwrap();
        let report = import_srm_parsers(&paths, &scan.parsers, true).unwrap();
        assert_eq!(report.emulators_added, 1);
        assert_eq!(report.scanners_added, 1);
        assert_eq!(report.games_imported, 2);
        assert!(report.errors.is_empty());

        let scanners = list_rom_scanners(&paths).unwrap();
        assert_eq!(scanners.len(), 1);
        assert_eq!(scanners[0].rom_glob.as_deref(), Some("${title}/@(*[BASE]*.nsp|*.xci)"));

        // Re-reading marks the parser as already configured.
        let again = parse_config_json("srm.json", &cfg.to_string(), &scanners).unwrap();
        assert!(again.parsers[0].already_configured);
        assert!(!again.parsers[0].selected);

        // Customize scanner settings and re-import with a trailing slash: existing
        // name/toggles are preserved, no duplicate scanner is created, and profile
        // with equivalent argv (`-f -g {ImagePath}` vs `-f -g "{ImagePath}"`) is reused.
        let mut custom = scanners[0].clone();
        custom.name = "My Switch Shelf".into();
        custom.auto_scan_on_refresh = false;
        upsert_rom_scanner(
            &paths,
            UpsertRomScannerArgs {
                id: Some(custom.id.clone()),
                name: Some(custom.name.clone()),
                emulator_id: custom.emulator_id.clone(),
                profile_id: custom.profile_id.clone(),
                directory: custom.directory.clone(),
                scan_subfolders: Some(false),
                merge_related_files: Some(false),
                auto_scan_on_refresh: Some(false),
                rom_glob: custom.rom_glob.clone(),
            },
        )
        .unwrap();
        let mut emus = list_user_emulators(&paths).unwrap();
        emus[0].profiles[0].arguments = "-f -g {ImagePath}".into();
        replace_user_emulator(&paths, emus[0].clone()).unwrap();

        let mut reimport = scan.parsers.clone();
        reimport[0].rom_directory = format!("{}/", roms.to_string_lossy());
        reimport[0].selected = true;
        let second = import_srm_parsers(&paths, &reimport, false).unwrap();
        assert_eq!(second.emulators_added, 0);
        assert_eq!(second.scanners_added, 0);

        let emus_after = list_user_emulators(&paths).unwrap();
        assert_eq!(emus_after[0].profiles.len(), 1, "equivalent quoted args reuse existing profile");

        let scanners_after = list_rom_scanners(&paths).unwrap();
        assert_eq!(scanners_after.len(), 1, "trailing slash does not create duplicate scanner");
        assert_eq!(scanners_after[0].name, "My Switch Shelf");
        assert!(!scanners_after[0].auto_scan_on_refresh);
        assert!(!scanners_after[0].scan_subfolders);
        assert!(!scanners_after[0].merge_related_files);
    }

    #[test]
    fn translates_case_insensitive_os_sep_and_flags_unknown_vars_and_incomplete_parsers() {
        let (ok, unresolved) = translate_srm_args_checked(r#"--dir="${FileDir}${/}sub" --os=${OS:win|mac|linux} "${FILEPATH}""#);
        assert!(unresolved.is_empty());
        let expected_sep = std::path::MAIN_SEPARATOR_STR;
        let expected_os = if cfg!(windows) {
            "win"
        } else if cfg!(target_os = "macos") {
            "mac"
        } else {
            "linux"
        };
        assert_eq!(ok, format!(r#"--dir="{{ImageDir}}{expected_sep}sub" --os={expected_os} "{{ImagePath}}""#));

        let json = r#"[
            {
                "parserType": "Glob",
                "configTitle": "RetroArch Custom",
                "romDirectory": "/roms/snes",
                "executable": { "path": "/usr/bin/retroarch" },
                "executableArgs": "-L \"${steamDirGlobal}/cores/snes.so\" \"${filePath}\"",
                "parserInputs": { "glob": "${title}.sfc" },
                "disabled": false
            },
            {
                "parserType": "Glob",
                "configTitle": "Incomplete Parser",
                "romDirectory": "",
                "executable": { "path": "/usr/bin/eden" },
                "parserInputs": { "glob": "*.nsp" },
                "disabled": false
            }
        ]"#;
        let scan = parse_config_json("srm.json", json, &[]).unwrap();
        assert_eq!(scan.parsers.len(), 1);
        assert!(!scan.parsers[0].selected, "parser with unresolved SRM variable starts unselected");
        assert_eq!(scan.errors.len(), 2, "reports both unresolved variable and incomplete parser");
        assert!(scan.errors[0].contains("${steamDirGlobal}"));
        assert!(scan.errors[1].contains("Incomplete Parser"));
    }

    #[test]
    fn expands_tilde_in_config_path() {
        struct FakeEnv;
        impl DetectEnv for FakeEnv {
            fn home(&self) -> String {
                if cfg!(windows) {
                    r"C:\Users\gamer".into()
                } else {
                    "/home/gamer".into()
                }
            }
            fn var(&self, _: &str) -> Option<String> {
                None
            }
            fn which(&self, _: &str) -> Option<String> {
                None
            }
            fn flatpak_installed(&self, _: &str) -> bool {
                false
            }
            fn exists(&self, _: &str) -> bool {
                false
            }
            fn read_dir(&self, _: &str) -> Vec<String> {
                vec![]
            }
        }
        if cfg!(windows) {
            assert_eq!(
                expand_config_path(r"~\AppData\Roaming\steam-rom-manager\userData\userConfigurations.json", &FakeEnv),
                r"C:\Users\gamer\AppData\Roaming\steam-rom-manager\userData\userConfigurations.json"
            );
            assert_eq!(expand_config_path(r"D:\abs\path.json", &FakeEnv), r"D:\abs\path.json");
        } else {
            assert_eq!(
                expand_config_path("~/.config/steam-rom-manager/userData/userConfigurations.json", &FakeEnv),
                "/home/gamer/.config/steam-rom-manager/userData/userConfigurations.json"
            );
            assert_eq!(expand_config_path("/abs/path.json", &FakeEnv), "/abs/path.json");
        }
    }
}
