//! `.desktop` shortcuts (Heroic `shortcuts/shortcuts/shortcuts.ts`, Linux
//! branch): `<Desktop>/<title>.desktop` and
//! `~/.local/share/applications/<title>.desktop`.

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

/// `(desktop file, application-menu file)` for a title.
pub fn shortcut_files(roots: &Roots, title: &str) -> (Option<PathBuf>, PathBuf) {
    let name = format!("{}.desktop", sanitize_filename(title));
    (roots.desktop.as_ref().map(|d| d.join(&name)), roots.applications.join(name))
}

/// Quotes one `Exec` argument per the Desktop Entry spec.
pub fn quote_exec_arg(arg: &str) -> String {
    let needs_quotes = arg.is_empty()
        || arg.chars().any(|c| {
            matches!(
                c,
                ' ' | '\t' | '\n' | '"' | '\'' | '\\' | '>' | '<' | '~' | '|' | '&' | ';' | '$' | '*' | '?' | '#' | '(' | ')' | '`'
            )
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
pub fn desktop_file_text(title: &str, exec: &str, icon: Option<&str>) -> String {
    let icon_line = icon.map(|i| format!("Icon={}\n", escape_value(i))).unwrap_or_default();
    format!(
        "[Desktop Entry]\nName={}\nExec={exec}\nTerminal=false\nType=Application\n{icon_line}Categories=Game;\n",
        escape_value(title)
    )
}

/// `<exe> --launch <runner> <appName>`, quoted for `Exec=`.
pub fn exec_line(exe: &Path, game: &GameId) -> String {
    format!(
        "{} --launch {} {}",
        quote_exec_arg(&exe.to_string_lossy()),
        quote_exec_arg(&game.runner),
        quote_exec_arg(&game.app_name)
    )
}

/// What [`add`] wrote.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Written {
    pub desktop: Option<PathBuf>,
    pub menu: Option<PathBuf>,
}

/// Adds the shortcuts: `desktop` and/or `menu` choose the targets
/// (`addDesktopShortcuts` / `addStartMenuShortcuts`, or both when asked from
/// the menu). Existing files are overwritten (the Exec path may have changed).
pub fn add(roots: &Roots, game: &GameId, exe: &Path, icon: Option<&Path>, desktop: bool, menu: bool) -> Result<Written> {
    let (desktop_file, menu_file) = shortcut_files(roots, &game.title);
    let text = desktop_file_text(&game.title, &exec_line(exe, game), icon.map(|p| p.to_string_lossy()).as_deref());
    let mut written = Written::default();
    if desktop {
        if let Some(file) = desktop_file {
            write_file(&file, &text, 0o755).with_context(|| format!("writing {}", file.display()))?;
            log::info!("shortcut saved on {}", file.display());
            written.desktop = Some(file);
        }
    }
    if menu {
        write_file(&menu_file, &text, 0o755).with_context(|| format!("writing {}", menu_file.display()))?;
        log::info!("shortcut saved on {}", menu_file.display());
        written.menu = Some(menu_file);
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

/// Removes both files when present; returns what was removed.
pub fn remove(roots: &Roots, title: &str) -> Result<Vec<PathBuf>> {
    let (desktop_file, menu_file) = shortcut_files(roots, title);
    let mut removed = Vec::new();
    for file in desktop_file.into_iter().chain([menu_file]) {
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

/// `shortcutsExists`: either file is there.
pub fn exists(roots: &Roots, title: &str) -> bool {
    let (desktop_file, menu_file) = shortcut_files(roots, title);
    desktop_file.is_some_and(|f| f.exists()) || menu_file.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(title: &str) -> GameId {
        GameId { runner: "legendary".into(), app_name: "Quail".into(), title: title.into() }
    }

    #[test]
    fn exec_quoting() {
        assert_eq!(quote_exec_arg("/usr/bin/heronite-rs"), "/usr/bin/heronite-rs");
        assert_eq!(quote_exec_arg("/opt/my app/x"), "\"/opt/my app/x\"");
        assert_eq!(quote_exec_arg("50%"), "50%%");
        // "a$b" -> "a\$b" (spec escape) -> backslash doubled at the string level.
        assert_eq!(quote_exec_arg("a$b"), "\"a\\\\$b\"");
    }

    #[test]
    fn desktop_text_has_exec_and_icon() {
        let exe = Path::new("/home/u/.local/opt/heronite-rs/heronite-rs");
        let g = game("Cool Game");
        let text = desktop_file_text(&g.title, &exec_line(exe, &g), Some("/icons/Quail.png"));
        assert_eq!(
            text,
            "[Desktop Entry]\nName=Cool Game\nExec=/home/u/.local/opt/heronite-rs/heronite-rs --launch legendary Quail\nTerminal=false\nType=Application\nIcon=/icons/Quail.png\nCategories=Game;\n"
        );
    }

    #[test]
    fn add_exists_remove_in_tempdirs() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots { desktop: Some(dir.path().join("Desktop")), applications: dir.path().join("apps") };
        let g = game("Half-Life: Alyx");
        let exe = Path::new("/bin/heronite-rs");
        assert!(!exists(&roots, &g.title));
        let w = add(&roots, &g, exe, None, true, true).unwrap();
        assert_eq!(w.desktop.as_deref(), Some(dir.path().join("Desktop/Half-Life Alyx.desktop").as_path()));
        assert_eq!(w.menu.as_deref(), Some(dir.path().join("apps/Half-Life Alyx.desktop").as_path()));
        assert!(exists(&roots, &g.title));
        let text = fs::read_to_string(w.menu.unwrap()).unwrap();
        assert!(text.contains("Name=Half-Life: Alyx\n"));
        assert!(text.contains("Exec=/bin/heronite-rs --launch legendary Quail\n"));
        assert!(!text.contains("Icon="));
        let removed = remove(&roots, &g.title).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(!exists(&roots, &g.title));
        // removing again is fine
        assert!(remove(&roots, &g.title).unwrap().is_empty());
    }

    #[test]
    fn only_requested_targets_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots { desktop: Some(dir.path().join("Desktop")), applications: dir.path().join("apps") };
        let g = game("Only Menu");
        let w = add(&roots, &g, Path::new("/bin/h"), None, false, true).unwrap();
        assert!(w.desktop.is_none() && w.menu.is_some());
        assert!(!dir.path().join("Desktop").exists());
    }
}
