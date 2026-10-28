//! `.desktop` shortcuts (Heroic `shortcuts/shortcuts/shortcuts.ts`, Linux
//! branch): `<Desktop>/<title>.desktop` and
//! `~/.local/share/applications/<title>.desktop`.
//!
//! A file with that name may be somebody else's (an app called like the game,
//! another game with the same sanitized title), so a file counts as ours only
//! when it carries the game's marker keys, or launches this game the way
//! earlier versions wrote it. [`add`] never overwrites a file that is not
//! ours: it takes the next free name. [`remove`] and [`exists`] only see ours.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::GameId;

/// Where shortcuts go. Built from the environment by [`Roots::detect`].
#[derive(Debug, Clone)]
pub struct Roots {
    /// XDG desktop dir (`None` when the system has none).
    pub desktop: Option<PathBuf>,
    /// `~/.local/share/applications`.
    pub applications: PathBuf,
}

impl Roots {
    /// `HERONITE_DESKTOP_DIR` / `HERONITE_APPLICATIONS_DIR` override the real
    /// folders (tests and manual runs against fixtures).
    pub fn detect() -> Self {
        let desktop = std::env::var_os("HERONITE_DESKTOP_DIR")
            .map(PathBuf::from)
            .or_else(dirs::desktop_dir)
            .or_else(|| dirs::home_dir().map(|h| h.join("Desktop")));
        let applications = std::env::var_os("HERONITE_APPLICATIONS_DIR")
            .map(PathBuf::from)
            .or_else(|| dirs::data_dir().map(|d| d.join("applications")))
            .unwrap_or_else(|| PathBuf::from("applications"));
        Self { desktop, applications }
    }
}

pub use heronite_data::text::sanitize_filename;

/// Longest file name a folder takes, in bytes.
const MAX_NAME: usize = 255;

/// Marker keys of the files this app writes: the game they launch.
const KEY_APP_NAME: &str = "X-Heronite-AppName";
const KEY_RUNNER: &str = "X-Heronite-Runner";

/// Names a game's shortcut may take in a folder, in the order [`add`] tries
/// them: the title, then the title with a `(Heronite)` tag for when the plain
/// name is another program's or another game's. A title with nothing usable
/// in it (`???`, `...`) falls back to the app name.
fn candidate_names(game: &GameId) -> Vec<String> {
    let usable = |s: &str| sanitize_filename(s).trim_start_matches('.').trim().to_string();
    let mut base = usable(&game.title);
    if base.is_empty() {
        base = usable(&game.app_name);
    }
    if base.is_empty() {
        base = "game".to_string();
    }
    let with = |tag: &str| {
        let tail = format!("{tag}.desktop");
        let mut stem = base.clone();
        while stem.len() + tail.len() > MAX_NAME {
            stem.pop();
        }
        format!("{stem}{tail}")
    };
    let mut names = vec![format!("{base}.desktop"), with(" (Heronite)")];
    names.extend((2..=20).map(|n| with(&format!(" (Heronite {n})"))));
    names
}

/// Value of `key` in the `[Desktop Entry]` group of `text`.
fn entry_value(text: &str, key: &str) -> Option<String> {
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry {
            if let Some(value) = line.strip_prefix(key).and_then(|rest| rest.strip_prefix('=')) {
                return Some(unescape_value(value));
            }
        }
    }
    None
}

fn unescape_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('s') => out.push(' '),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// The arguments of an `Exec` value (the reverse of [`quote_exec_arg`]).
fn split_exec(exec: &str) -> Vec<String> {
    let (mut args, mut cur, mut quoted, mut started) = (Vec::new(), String::new(), false, false);
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => cur.extend(chars.next()),
            c if c.is_whitespace() && !quoted => {
                if started {
                    args.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(cur);
    }
    args.into_iter().map(|a| a.replace("%%", "%")).collect()
}

/// Whether the file with this `text` is the shortcut of `game`: it has the
/// marker keys, or it is `<heronite> --launch <runner> <appName>` as earlier
/// versions wrote it.
fn is_shortcut_of(text: &str, game: &GameId) -> bool {
    if let Some(app_name) = entry_value(text, KEY_APP_NAME) {
        return app_name == game.app_name && entry_value(text, KEY_RUNNER).is_none_or(|runner| runner == game.runner);
    }
    let Some(exec) = entry_value(text, "Exec") else { return false };
    let args = split_exec(&exec);
    let [exe, flag, runner, app_name] = args.as_slice() else { return false };
    let program = exe.rsplit('/').next().unwrap_or(exe).to_ascii_lowercase();
    program.contains("heronite") && flag == "--launch" && *runner == game.runner && *app_name == game.app_name
}

fn is_ours(file: &Path, game: &GameId) -> bool {
    fs::read_to_string(file).is_ok_and(|text| is_shortcut_of(&text, game))
}

/// The game's shortcut in `dir`, if there is one.
fn find_in(dir: &Path, game: &GameId) -> Option<PathBuf> {
    candidate_names(game).into_iter().map(|n| dir.join(n)).find(|f| is_ours(f, game))
}

/// Where [`add`] writes in `dir`: the game's own file when there is one, else
/// the first name nobody has taken.
fn target_in(dir: &Path, game: &GameId) -> Result<PathBuf> {
    if let Some(file) = find_in(dir, game) {
        return Ok(file);
    }
    candidate_names(game)
        .into_iter()
        .map(|n| dir.join(n))
        .find(|f| f.symlink_metadata().is_err())
        .with_context(|| format!("no free shortcut name for {} in {}", game.title, dir.display()))
}

/// Quotes one `Exec` argument per the Desktop Entry spec.
pub fn quote_exec_arg(arg: &str) -> String {
    let needs_quotes = arg.is_empty()
        || arg.chars().any(|c| {
            matches!(c, ' ' | '\t' | '\n' | '"' | '\'' | '\\' | '>' | '<' | '~' | '|' | '&' | ';' | '$' | '*' | '?' | '#' | '(' | ')' | '`')
        });
    let arg = arg.replace('%', "%%");
    if !needs_quotes {
        return arg;
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    // The Exec key is a string value: backslashes are escaped once more.
    out.replace('\\', "\\\\")
}

fn escape_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
}

/// Content of the `.desktop` file. `exec` is the already-quoted command line.
pub fn desktop_file_text(game: &GameId, exec: &str, icon: Option<&str>) -> String {
    let icon_line = icon.map(|i| format!("Icon={}\n", escape_value(i))).unwrap_or_default();
    format!(
        "[Desktop Entry]\nName={}\nExec={exec}\nTerminal=false\nType=Application\n{icon_line}Categories=Game;\n{KEY_APP_NAME}={}\n{KEY_RUNNER}={}\n",
        escape_value(&game.title),
        escape_value(&game.app_name),
        escape_value(&game.runner)
    )
}

/// `<exe> --launch <runner> <appName>`, quoted for `Exec=`.
pub fn exec_line(exe: &Path, game: &GameId) -> String {
    format!("{} --launch {} {}", quote_exec_arg(&exe.to_string_lossy()), quote_exec_arg(&game.runner), quote_exec_arg(&game.app_name))
}

/// What [`add`] wrote.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Written {
    pub desktop: Option<PathBuf>,
    pub menu: Option<PathBuf>,
}

/// Adds the shortcuts: `desktop` and/or `menu` choose the targets
/// (`addDesktopShortcuts` / `addStartMenuShortcuts`, or both when asked from
/// the menu). The game's own files are overwritten (the Exec path may have
/// changed); a file of anybody else with the same name is left alone and the
/// shortcut takes another name.
pub fn add(roots: &Roots, game: &GameId, exe: &Path, icon: Option<&Path>, desktop: bool, menu: bool) -> Result<Written> {
    let text = desktop_file_text(game, &exec_line(exe, game), icon.map(|p| p.to_string_lossy()).as_deref());
    let mut written = Written::default();
    if desktop {
        if let Some(dir) = &roots.desktop {
            let file = target_in(dir, game)?;
            write_file(&file, &text, 0o755).with_context(|| format!("writing {}", file.display()))?;
            log::info!("shortcut saved on {}", file.display());
            written.desktop = Some(file);
        }
    }
    if menu {
        let file = target_in(&roots.applications, game)?;
        write_file(&file, &text, 0o755).with_context(|| format!("writing {}", file.display()))?;
        log::info!("shortcut saved on {}", file.display());
        written.menu = Some(file);
    }
    Ok(written)
}

fn write_file(path: &Path, text: &str, mode: u32) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    heronite_data::temps::write_atomic(path, text.as_bytes(), Some(mode))?;
    Ok(())
}

/// Removes the game's files when present; returns what was removed. Files
/// that are not the game's are left alone.
pub fn remove(roots: &Roots, game: &GameId) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    for dir in roots.desktop.iter().chain([&roots.applications]) {
        let Some(file) = find_in(dir, game) else { continue };
        match fs::remove_file(&file) {
            Ok(()) => {
                log::info!("shortcut removed: {}", file.display());
                removed.push(file);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("removing {}", file.display())),
        }
    }
    Ok(removed)
}

/// `shortcutsExists`: either of the game's files is there.
pub fn exists(roots: &Roots, game: &GameId) -> bool {
    roots.desktop.iter().chain([&roots.applications]).any(|dir| find_in(dir, game).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(title: &str) -> GameId {
        GameId { runner: "legendary".into(), app_name: "Quail".into(), title: title.into() }
    }

    fn other(title: &str, app_name: &str) -> GameId {
        GameId { runner: "gog".into(), app_name: app_name.into(), title: title.into() }
    }

    fn roots(dir: &Path) -> Roots {
        Roots { desktop: Some(dir.join("Desktop")), applications: dir.join("apps") }
    }

    const EXE: &str = "/bin/heronite-rs";

    #[test]
    fn exec_quoting() {
        assert_eq!(quote_exec_arg("/usr/bin/heronite-rs"), "/usr/bin/heronite-rs");
        assert_eq!(quote_exec_arg("/opt/my app/x"), "\"/opt/my app/x\"");
        assert_eq!(quote_exec_arg("50%"), "50%%");
        // "a$b" -> "a\$b" (spec escape) -> backslash doubled at the string level.
        assert_eq!(quote_exec_arg("a$b"), "\"a\\\\$b\"");
    }

    #[test]
    fn desktop_text_has_exec_icon_and_marker() {
        let exe = Path::new("/home/u/.local/opt/heronite-rs/heronite-rs");
        let g = game("Cool Game");
        let text = desktop_file_text(&g, &exec_line(exe, &g), Some("/icons/Quail.png"));
        assert_eq!(
            text,
            "[Desktop Entry]\nName=Cool Game\nExec=/home/u/.local/opt/heronite-rs/heronite-rs --launch legendary Quail\nTerminal=false\nType=Application\nIcon=/icons/Quail.png\nCategories=Game;\nX-Heronite-AppName=Quail\nX-Heronite-Runner=legendary\n"
        );
        assert!(is_shortcut_of(&text, &g));
    }

    #[test]
    fn add_exists_remove_in_tempdirs() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        let g = game("Half-Life: Alyx");
        let exe = Path::new(EXE);
        assert!(!exists(&roots, &g));
        let w = add(&roots, &g, exe, None, true, true).unwrap();
        assert_eq!(w.desktop.as_deref(), Some(dir.path().join("Desktop/Half-Life Alyx.desktop").as_path()));
        assert_eq!(w.menu.as_deref(), Some(dir.path().join("apps/Half-Life Alyx.desktop").as_path()));
        assert!(exists(&roots, &g));
        let text = fs::read_to_string(w.menu.unwrap()).unwrap();
        assert!(text.contains("Name=Half-Life: Alyx\n"));
        assert!(text.contains("Exec=/bin/heronite-rs --launch legendary Quail\n"));
        assert!(text.contains("X-Heronite-AppName=Quail\n"));
        assert!(!text.contains("Icon="));
        let removed = remove(&roots, &g).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(!exists(&roots, &g));
        // removing again is fine
        assert!(remove(&roots, &g).unwrap().is_empty());
    }

    #[test]
    fn only_requested_targets_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        let w = add(&roots, &game("Only Menu"), Path::new("/bin/h"), None, false, true).unwrap();
        assert!(w.desktop.is_none() && w.menu.is_some());
        assert!(!dir.path().join("Desktop").exists());
    }

    #[test]
    fn adding_again_overwrites_its_own_file() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        let g = game("Cool Game");
        add(&roots, &g, Path::new("/old/heronite-rs"), None, true, true).unwrap();
        let w = add(&roots, &g, Path::new("/new/heronite-rs"), None, true, true).unwrap();
        let file = dir.path().join("apps/Cool Game.desktop");
        assert_eq!(w.menu.as_deref(), Some(file.as_path()));
        assert!(fs::read_to_string(&file).unwrap().contains("Exec=/new/heronite-rs"));
        assert_eq!(fs::read_dir(dir.path().join("apps")).unwrap().count(), 1);
    }

    #[test]
    fn a_foreign_file_with_the_name_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        fs::create_dir_all(&roots.applications).unwrap();
        let theirs = roots.applications.join("Firefox.desktop");
        let their_text = "[Desktop Entry]\nName=Firefox\nExec=firefox %u\nType=Application\n";
        fs::write(&theirs, their_text).unwrap();
        let g = game("Firefox");
        // Not ours: neither seen nor removed.
        assert!(!exists(&roots, &g));
        assert!(remove(&roots, &g).unwrap().is_empty());
        let w = add(&roots, &g, Path::new(EXE), None, false, true).unwrap();
        assert_eq!(fs::read_to_string(&theirs).unwrap(), their_text);
        let ours = roots.applications.join("Firefox (Heronite).desktop");
        assert_eq!(w.menu.as_deref(), Some(ours.as_path()));
        assert!(exists(&roots, &g));
        // Adding again reuses the tagged file; removing takes only that one.
        assert_eq!(add(&roots, &g, Path::new(EXE), None, false, true).unwrap().menu.as_deref(), Some(ours.as_path()));
        assert_eq!(remove(&roots, &g).unwrap(), vec![ours.clone()]);
        assert!(!ours.exists());
        assert_eq!(fs::read_to_string(&theirs).unwrap(), their_text);
    }

    #[test]
    fn titles_that_sanitize_the_same_keep_both_shortcuts() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        let (a, b, c) = (game("Doom: Eternal"), other("Doom? Eternal", "doom2"), other("Doom* Eternal", "doom3"));
        for g in [&a, &b, &c] {
            add(&roots, g, Path::new(EXE), None, true, true).unwrap();
        }
        for g in [&a, &b, &c] {
            assert!(exists(&roots, g));
        }
        let names = |d: &str| {
            let mut v: Vec<_> =
                fs::read_dir(dir.path().join(d)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            v.sort();
            v
        };
        assert_eq!(names("apps"), ["Doom Eternal (Heronite 2).desktop", "Doom Eternal (Heronite).desktop", "Doom Eternal.desktop"]);
        // Removing one leaves the others.
        assert_eq!(remove(&roots, &b).unwrap().len(), 2);
        assert!(exists(&roots, &a) && exists(&roots, &c) && !exists(&roots, &b));
        // Its name is free again, and the next one takes the first free name.
        assert_eq!(remove(&roots, &a).unwrap().len(), 2);
        let w = add(&roots, &b, Path::new(EXE), None, false, true).unwrap();
        assert_eq!(w.menu.unwrap().file_name().unwrap(), "Doom Eternal.desktop");
    }

    #[test]
    fn a_title_with_nothing_usable_falls_back_to_the_app_name() {
        for title in ["???", "...", "", " . "] {
            let dir = tempfile::tempdir().unwrap();
            let roots = roots(dir.path());
            let g = game(title);
            let w = add(&roots, &g, Path::new(EXE), None, true, true).unwrap();
            assert_eq!(w.menu.as_deref(), Some(dir.path().join("apps/Quail.desktop").as_path()), "{title:?}");
            assert!(exists(&roots, &g));
        }
        let hidden = game(".hidden");
        assert_eq!(candidate_names(&hidden)[0], "hidden.desktop");
        let nothing = GameId { runner: "r".into(), app_name: "..".into(), title: "??".into() };
        assert_eq!(candidate_names(&nothing)[0], "game.desktop");
    }

    #[test]
    fn files_written_by_earlier_versions_are_still_ours() {
        let dir = tempfile::tempdir().unwrap();
        let roots = roots(dir.path());
        fs::create_dir_all(&roots.applications).unwrap();
        let old = "[Desktop Entry]\nName=Cool Game\nExec=/home/u/.local/opt/heronite-rs/heronite-rs --launch legendary Quail\nTerminal=false\nType=Application\nCategories=Game;\n";
        let file = roots.applications.join("Cool Game.desktop");
        fs::write(&file, old).unwrap();
        let g = game("Cool Game");
        assert!(exists(&roots, &g));
        // Another game, or another program with a similar Exec, does not match.
        assert!(!exists(&roots, &GameId { app_name: "Other".into(), ..g.clone() }));
        assert!(!is_shortcut_of(&old.replace("heronite-rs --launch", "heroic --launch"), &g));
        assert!(!is_shortcut_of(&old.replace("--launch legendary Quail", "--launch legendary Quail --extra"), &g));
        // Quoted paths (AppImage in a folder with spaces) match too.
        let quoted = desktop_file_text(&g, &exec_line(Path::new("/opt/My Apps/Heronite.AppImage"), &g), None);
        let legacy = quoted.lines().filter(|l| !l.starts_with("X-Heronite")).collect::<Vec<_>>().join("\n");
        assert!(legacy.contains("\"/opt/My Apps/Heronite.AppImage\""));
        assert!(is_shortcut_of(&legacy, &g));
        assert_eq!(remove(&roots, &g).unwrap(), vec![file]);
    }

    #[test]
    fn exec_arguments_survive_a_round_trip() {
        for arg in ["/usr/bin/x", "/opt/my app/x", "a$b c", "50%", "q\"uote", "back\\slash", "(paren)"] {
            let value = format!("Exec={}", quote_exec_arg(arg));
            let text = format!("[Desktop Entry]\n{value}\n");
            let exec = entry_value(&text, "Exec").unwrap();
            assert_eq!(split_exec(&exec), [arg], "{arg}");
        }
    }

    #[test]
    fn the_marker_names_the_game_it_launches() {
        let g = game("Cool Game");
        let text = desktop_file_text(&g, "x", None);
        assert!(!is_shortcut_of(&text, &other("Cool Game", "Quail")), "same app name under another runner");
        assert!(!is_shortcut_of(&text, &GameId { app_name: "Other".into(), ..g }));
    }
}
