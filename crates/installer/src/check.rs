//! What a script needs and what stops it, known before anything runs: an
//! unsupported runner or step, a file that cannot be fetched, a program
//! missing on this computer. The install dialog shows the result and only
//! starts when nothing blocks.

use heronite_i18n::tr;

use crate::archive::{self, Format};
use crate::exec::which_any;
use crate::files::Source;
use crate::runners::{self, Runner};
use crate::script::Script;
use crate::steps::{Action, Task};
use crate::text::get_nonempty;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub text: String,
    /// The install cannot start.
    pub blocking: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Check {
    pub runner: Option<Runner>,
    pub issues: Vec<Issue>,
    /// Files the user picks: `(file id, what to pick)`.
    pub asks: Vec<(String, String)>,
    /// Files downloaded from the internet: `(name, address)`.
    pub downloads: Vec<(String, String)>,
    /// Programs and commands the script runs, as it wrote them.
    pub runs: Vec<String>,
    /// The script asks questions or for a disc while it runs.
    pub interactive: bool,
}

impl Check {
    pub fn blocked(&self) -> bool {
        self.issues.iter().any(|i| i.blocking)
    }

    pub fn needs_wine(&self) -> bool {
        self.runner == Some(Runner::Wine)
    }

    fn block(&mut self, text: impl Into<String>) {
        self.issues.push(Issue { text: text.into(), blocking: true });
    }

    fn warn(&mut self, text: impl Into<String>) {
        self.issues.push(Issue { text: text.into(), blocking: false });
    }
}

/// Checks `script` against this app and computer (`which`: finds a program
/// on `$PATH`).
pub fn check(script: &Script) -> Check {
    check_with(script, &|names| which_any(names).is_some())
}

/// [`check`] with the program lookup given (tests).
pub fn check_with(script: &Script, have: &dyn Fn(&[&str]) -> bool) -> Check {
    let mut c = Check::default();
    match runners::runner(&script.runner) {
        Ok(r) => c.runner = Some(r),
        Err(why) => c.block(why),
    }
    if let Some(base) = script.requires.as_ref().or(script.extends.as_ref()) {
        c.block(tr!("This script adds to another game ({game}) and cannot install on its own", game = base));
    }
    if let Some(r) = c.runner {
        if matches!(r, Runner::Wine | Runner::Linux) && get_nonempty(&script.game, "exe").is_none() {
            c.block(tr!("The script does not say which file starts the game"));
        }
        for group in r.tools() {
            if !have(group) {
                c.block(tr!("{program} is needed to play this game; install it with your package manager", program = group[0]));
            }
        }
    }
    for group in &script.require_binaries {
        let names: Vec<&str> = group.iter().map(String::as_str).collect();
        if !have(&names) {
            c.block(tr!("The script needs {program}; install it with your package manager", program = names.join(" / ")));
        }
    }
    for f in &script.files {
        match &f.source {
            Source::Ask(_) => c.asks.push((f.id.clone(), script.preview(&f.prompt()))),
            Source::Url(u) => c.downloads.push((f.file_name(), u.clone())),
            Source::Local(_) => {}
            Source::Unsupported(raw) => {
                c.block(tr!("The file \"{id}\" ({raw}) cannot be downloaded here", id = f.id.clone(), raw = raw.clone()))
            }
        }
    }
    let mut formats: Vec<Format> = Vec::new();
    for (i, step) in script.steps.iter().enumerate() {
        let n = i + 1;
        match &step.action {
            Action::Unsupported { name, reason } => {
                c.block(tr!("Step {n} ({name}): {reason}", n, name = name.clone(), reason = reason.clone()))
            }
            Action::Execute(e) => {
                c.runs.push(
                    e.command.clone().or_else(|| e.file.clone().map(|f| format!("{f} {}", e.args).trim().to_string())).unwrap_or_default(),
                );
                if e.terminal {
                    c.warn(tr!("Step {n} expects a terminal; its output goes to the install log instead", n));
                }
            }
            Action::Task(Task::WineExec { executable, args, .. }) => c.runs.push(format!("{executable} {args}").trim().to_string()),
            Action::Task(Task::DosExec { executable, .. }) => {
                c.runs.push(executable.clone());
                if !have(&["dosbox", "dosbox-staging", "dosbox-x"]) {
                    c.block(tr!("{program} is needed to play this game; install it with your package manager", program = "dosbox"));
                }
            }
            Action::Task(Task::Winetricks { .. } | Task::CreatePrefix { .. } | Task::Winecfg { .. }) if c.runner != Some(Runner::Wine) => {
                c.block(tr!("Step {n} needs Wine, but the script is not a Wine script", n));
            }
            Action::Extract(x) => formats.push(format_of(script, &x.file, x.format.as_deref())),
            Action::InstallOrExtract(f) | Action::ExtractOrRun { file: f, .. } => {
                c.runs.push(f.clone());
                let fmt = format_of(script, f, None);
                if fmt != Format::Exe {
                    formats.push(fmt);
                }
            }
            Action::InputMenu(_) | Action::InsertDisc { .. } => c.interactive = true,
            _ => {}
        }
    }
    c.runs = c.runs.iter().map(|r| script.preview(r)).collect();
    formats.sort_by_key(|f| *f as u8);
    formats.dedup();
    for f in formats {
        for group in f.tools() {
            if !group.is_empty() && !have(group) {
                c.block(tr!("{program} is needed to extract the game's files; install it with your package manager", program = group[0]));
            }
        }
    }
    c
}

/// The archive format of a step's file: forced, else from the file's name
/// (a file id's download name, or the path), else 7-Zip's catch-all.
fn format_of(script: &Script, file: &str, forced: Option<&str>) -> Format {
    if let Some(f) = forced.and_then(archive::named) {
        return f;
    }
    let id = file.trim().trim_start_matches('$');
    let name = script.files.iter().find(|f| f.id == id).map(|f| f.file_name()).unwrap_or_else(|| file.to_string());
    archive::by_name(&name).unwrap_or(Format::SevenZip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::parse;

    fn script(text: &str) -> Script {
        parse(text, None, "x").unwrap().remove(0)
    }

    const OK: &str = r#"
name: Game
runner: wine
script:
  files:
  - setup: N/A:Select the setup
  - patch: https://example.com/patch.7z
  game:
    exe: $GAMEDIR/drive_c/game.exe
  installer:
  - task: {name: create_prefix}
  - task: {name: wineexec, executable: setup}
  - extract: {file: patch, dst: $GAMEDIR}
  - input_menu: {description: Language, options: [{en: English}]}
"#;

    #[test]
    fn a_wine_script_with_everything_available() {
        let c = check_with(&script(OK), &|_| true);
        assert!(!c.blocked(), "{:?}", c.issues);
        assert!(c.needs_wine());
        assert_eq!(c.asks, [("setup".to_string(), "Select the setup".to_string())]);
        assert_eq!(c.downloads, [("patch.7z".to_string(), "https://example.com/patch.7z".to_string())]);
        assert_eq!(c.runs, ["setup"]);
        assert!(c.interactive);
    }

    #[test]
    fn missing_archivers_block() {
        let c = check_with(&script(OK), &|names| !names.contains(&"7z"));
        assert!(c.blocked());
        assert!(c.issues.iter().any(|i| i.text.contains("7z")), "{:?}", c.issues);
    }

    #[test]
    fn unsupported_things_block() {
        let s = script("name: G\nrunner: steam\nscript:\n  installer:\n  - frobnicate: {}\n");
        let c = check_with(&s, &|_| true);
        assert_eq!(c.issues.iter().filter(|i| i.blocking).count(), 2, "{:?}", c.issues);
        let s = script("name: G\nrunner: wine\nscript:\n  requires: base-game\n  game: {exe: a.exe}\n");
        assert!(check_with(&s, &|_| true).blocked());
        let s = script("name: G\nrunner: linux\nscript:\n  game: {}\n");
        assert!(check_with(&s, &|_| true).blocked(), "no entry point");
        let s = script("name: G\nrunner: dosbox\nscript:\n  game: {main_file: a.conf}\n");
        assert!(check_with(&s, &|n| !n.contains(&"dosbox")).blocked(), "no DOSBox");
    }
}
