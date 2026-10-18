//! The `installer` steps of a script, read into typed [`Action`]s. Values
//! keep their `$VARIABLES`: they are substituted when the step runs, since
//! answers and the disc path only exist by then.
//!
//! A step is a one-key map (`- extract: {file: setup, dst: $GAMEDIR}`), the
//! key being the directive (`insert-disc` = `insert_disc`). A step that
//! cannot be read, or that this app does not do, becomes
//! [`Action::Unsupported`] and [`crate::check`] refuses the script before it
//! runs, instead of failing half way.

use heronite_i18n::tr;
use serde_json::{Map, Value};

use crate::text::{get_nonempty, get_str, pairs, scalar, truthy};

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub action: Action,
    /// The script's own words for the step ("Installing .NET...").
    pub description: Option<String>,
    /// An exit code accepted besides 0 (`return_code`).
    pub return_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Makes a file executable.
    Chmodx(String),
    Execute(Execute),
    Extract(Extract),
    InputMenu(InputMenu),
    /// Waits for a disc (or a mounted image) holding `requires`; sets `$DISC`.
    InsertDisc {
        requires: String,
        message: Option<String>,
    },
    Mkdir(String),
    /// Copies `src` (a file or a folder's contents) into the folder `dst`.
    Merge {
        src: String,
        dst: String,
        optional: bool,
        remove_source: bool,
    },
    /// Moves `src` into the folder `dst`.
    Move {
        src: String,
        dst: String,
        optional: bool,
    },
    /// Renames `src` to `dst`.
    Rename {
        src: String,
        dst: String,
    },
    /// Replaces the variables inside a text file.
    SubstituteVars(String),
    WriteFile {
        file: String,
        content: String,
        append: bool,
    },
    WriteJson {
        file: String,
        data: Value,
        merge: bool,
    },
    /// Sets keys of an INI file: `(section, [(key, value)])`.
    WriteConfig {
        file: String,
        data: Vec<(String, Vec<(String, String)>)>,
        merge: bool,
    },
    Task(Task),
    /// Wine only: runs the file when it is an `.exe`, else extracts it into
    /// `$GAMEDIR/drive_c/<game slug>`.
    InstallOrExtract(String),
    /// Extracts `file` into `dst` (default `$CACHE`); when that gives a
    /// setup program, runs it, else merges the files into `merge_dst`.
    ExtractOrRun {
        file: String,
        dst: Option<String>,
        merge_dst: Option<String>,
    },
    /// Read but not something this app can do: `(directive, why)`.
    Unsupported {
        name: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Execute {
    /// A program (file id or path)...
    pub file: Option<String>,
    /// ...or a shell command run by `bash -c`.
    pub command: Option<String>,
    pub args: String,
    pub working_dir: Option<String>,
    pub env: Vec<(String, String)>,
    /// Asked to run in a terminal; this app shows the output in the log.
    pub terminal: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Extract {
    pub file: String,
    pub dst: Option<String>,
    /// Forced format (`zip`, `7z`, `tgz`, `gog`, `innoextract`...).
    pub format: Option<String>,
    /// Keep a single top folder instead of merging its contents.
    pub nomerge: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct InputMenu {
    /// Answer goes to `$INPUT` and, with an id, `$INPUT_<id>`.
    pub id: Option<String>,
    pub description: String,
    /// `(value, label)` in the script's order.
    pub options: Vec<(String, String)>,
    pub preselect: Option<String>,
}

/// A runner task (`task: {name: ..}`).
#[derive(Debug, Clone, PartialEq)]
pub enum Task {
    CreatePrefix {
        prefix: Option<String>,
        arch: Option<String>,
        overrides: Vec<(String, String)>,
    },
    /// Runs a Windows program in the prefix and waits for it (and what it
    /// started) to end.
    WineExec {
        executable: String,
        args: String,
        prefix: Option<String>,
        working_dir: Option<String>,
        env: Vec<(String, String)>,
    },
    /// Installs Winetricks verbs (`apps`, space separated after
    /// substitution); empty opens the Winetricks window.
    Winetricks {
        apps: String,
        prefix: Option<String>,
        env: Vec<(String, String)>,
    },
    Winecfg {
        prefix: Option<String>,
    },
    Winekill {
        prefix: Option<String>,
    },
    /// `path` like `HKEY_CURRENT_USER\Software\Wine\Direct3D`; `value: None`
    /// deletes the value.
    SetRegedit {
        path: String,
        key: String,
        value: Option<String>,
        kind: String,
        prefix: Option<String>,
    },
    SetRegeditFile {
        filename: String,
        prefix: Option<String>,
    },
    DeleteRegistryKey {
        key: String,
        prefix: Option<String>,
    },
    /// Nothing to do: no disc is mounted inside Wine here.
    EjectDisc,
    /// Runs a DOS program with DOSBox.
    DosExec {
        executable: String,
        args: String,
        working_dir: Option<String>,
        config_file: Option<String>,
        exit: bool,
    },
}

/// Reads one entry of the `installer` list (`runner`: the script's).
pub fn parse(entry: &Value, runner: &str) -> Step {
    let (name, params) = match entry {
        Value::Object(m) if !m.is_empty() => {
            let (k, v) = m.iter().next().expect("not empty");
            (k.clone(), v.clone())
        }
        Value::String(s) => (s.clone(), Value::Object(Map::new())),
        _ => return unsupported("?", tr!("This step is not a directive")),
    };
    let name = name.replace('-', "_").trim_matches('_').to_string();
    let description = params.as_object().and_then(|m| get_nonempty(m, "description")).map(|d| d.trim().to_string());
    let return_code = params.as_object().and_then(|m| m.get("return_code")).and_then(scalar).and_then(|s| s.trim().parse().ok());
    let action = match read_action(&name, &params, runner) {
        Ok(a) => a,
        Err(reason) => Action::Unsupported { name: name.clone(), reason },
    };
    Step { action, description, return_code }
}

fn unsupported(name: &str, reason: &str) -> Step {
    Step { action: Action::Unsupported { name: name.into(), reason: reason.into() }, description: None, return_code: None }
}

fn need(map: &Map<String, Value>, key: &str, directive: &str) -> Result<String, String> {
    get_str(map, key).ok_or_else(|| tr!("The {param} parameter is required by {directive}", param = key, directive))
}

fn as_map<'a>(params: &'a Value, directive: &str) -> Result<&'a Map<String, Value>, String> {
    params.as_object().ok_or_else(|| tr!("The parameters of {directive} must be a map", directive))
}

/// A directive whose value is a path (`mkdir: $GAMEDIR/x`), or a map with it.
fn path_param(params: &Value, key: &str, directive: &str) -> Result<String, String> {
    match params {
        Value::Object(m) => need(m, key, directive),
        v => scalar(v).filter(|s| !s.trim().is_empty()).ok_or_else(|| tr!("{directive} needs a path", directive)),
    }
}

fn opt(map: &Map<String, Value>, key: &str) -> Option<String> {
    get_nonempty(map, key)
}

fn read_action(name: &str, params: &Value, runner: &str) -> Result<Action, String> {
    Ok(match name {
        "chmodx" => Action::Chmodx(path_param(params, "file", name)?),
        "mkdir" => Action::Mkdir(path_param(params, "path", name)?),
        "execute" => Action::Execute(read_execute(params)?),
        "extract" => {
            let m = as_map(params, name)?;
            let file = opt(m, "file").or_else(|| opt(m, "src")).ok_or_else(|| tr!("extract needs a file"))?;
            Action::Extract(Extract {
                file,
                dst: opt(m, "dst"),
                format: opt(m, "format").or_else(|| opt(m, "extractor")),
                nomerge: m.contains_key("nomerge"),
            })
        }
        "input_menu" => Action::InputMenu(read_menu(as_map(params, name)?)?),
        "insert_disc" => {
            let m = as_map(params, name)?;
            Action::InsertDisc { requires: need(m, "requires", name)?, message: opt(m, "message") }
        }
        "merge" | "copy" => {
            let m = as_map(params, name)?;
            Action::Merge {
                src: need(m, "src", name)?,
                dst: need(m, "dst", name)?,
                optional: truthy(m.get("optional")).unwrap_or(false),
                remove_source: truthy(m.get("remove_source")).unwrap_or(false),
            }
        }
        "move" => {
            let m = as_map(params, name)?;
            Action::Move { src: need(m, "src", name)?, dst: need(m, "dst", name)?, optional: truthy(m.get("optional")).unwrap_or(false) }
        }
        "rename" => {
            let m = as_map(params, name)?;
            Action::Rename { src: need(m, "src", name)?, dst: need(m, "dst", name)? }
        }
        "substitute_vars" => Action::SubstituteVars(path_param(params, "file", name)?),
        "write_file" => {
            let m = as_map(params, name)?;
            let mode = get_str(m, "mode").unwrap_or_else(|| "w".into());
            if !mode.starts_with('a') && !mode.starts_with('w') {
                return Err(tr!("Wrong value for write_file mode: '{mode}'", mode));
            }
            Action::WriteFile { file: need(m, "file", name)?, content: need(m, "content", name)?, append: mode.starts_with('a') }
        }
        "write_json" => {
            let m = as_map(params, name)?;
            let data = m
                .get("data")
                .cloned()
                .ok_or_else(|| tr!("The {param} parameter is required by {directive}", param = "data", directive = name))?;
            Action::WriteJson { file: need(m, "file", name)?, data, merge: truthy(m.get("merge")).unwrap_or(true) }
        }
        "write_config" => {
            let m = as_map(params, name)?;
            let data = match m.get("data") {
                Some(Value::Object(sections)) => {
                    sections.iter().map(|(section, keys)| (section.clone(), pairs(Some(keys)))).collect::<Vec<_>>()
                }
                _ => vec![(need(m, "section", name)?, vec![(need(m, "key", name)?, need(m, "value", name)?)])],
            };
            Action::WriteConfig { file: need(m, "file", name)?, data, merge: truthy(m.get("merge")).unwrap_or(true) }
        }
        "task" => Action::Task(read_task(as_map(params, name)?, runner)?),
        "install_or_extract" => {
            if runner != "wine" {
                return Err(tr!("install_or_extract only works with Wine scripts").into());
            }
            Action::InstallOrExtract(path_param(params, "file", name)?)
        }
        "extract_or_run" => {
            let m = as_map(params, name)?;
            Action::ExtractOrRun { file: need(m, "file", name)?, dst: opt(m, "dst"), merge_dst: opt(m, "merge_dst") }
        }
        "autosetup_gog_game" | "autosetup_zoom_platform" | "autosetup_amazon" | "gogdl_setup" => {
            return Err(tr!("Store setups are not supported; install the game from its store instead").into())
        }
        _ => return Err(tr!("Unknown directive").into()),
    })
}

fn read_execute(params: &Value) -> Result<Execute, String> {
    let m = match params {
        Value::String(command) => return Ok(Execute { command: Some(command.clone()), ..Default::default() }),
        Value::Object(m) => m,
        _ => return Err(tr!("No parameters supplied to execute").into()),
    };
    let file = opt(m, "file");
    let command = match m.get("command") {
        None => None,
        Some(Value::String(c)) => Some(c.clone()),
        Some(_) => return Err(tr!("The command parameter of execute must be text").into()),
    };
    match (&file, &command) {
        (Some(_), Some(_)) => return Err(tr!("execute takes a file or a command, not both").into()),
        (None, None) => return Err(tr!("execute needs a file or a command").into()),
        _ => {}
    }
    Ok(Execute {
        file,
        command,
        args: get_str(m, "args").unwrap_or_default(),
        working_dir: opt(m, "working_dir"),
        env: pairs(m.get("env")),
        terminal: truthy(m.get("terminal")).unwrap_or(false),
    })
}

fn read_menu(m: &Map<String, Value>) -> Result<InputMenu, String> {
    let mut options = Vec::new();
    match m.get("options") {
        Some(Value::Array(items)) => {
            for item in items {
                match item {
                    Value::Object(o) => options.extend(o.iter().filter_map(|(k, v)| scalar(v).map(|label| (k.clone(), label)))),
                    v => {
                        if let Some(s) = scalar(v) {
                            options.push((s.clone(), s));
                        }
                    }
                }
            }
        }
        Some(Value::Object(o)) => options.extend(o.iter().filter_map(|(k, v)| scalar(v).map(|label| (k.clone(), label)))),
        _ => {}
    }
    if options.is_empty() {
        return Err(tr!("input_menu has no options").into());
    }
    Ok(InputMenu {
        id: opt(m, "id"),
        description: get_str(m, "description").unwrap_or_default().trim().to_string(),
        options,
        preselect: opt(m, "preselect"),
    })
}

fn read_task(m: &Map<String, Value>, runner: &str) -> Result<Task, String> {
    let full = get_nonempty(m, "name").ok_or_else(|| tr!("A task needs a name"))?;
    let (task_runner, name) = match full.split_once('.') {
        Some((r, n)) => (r.to_ascii_lowercase(), n.to_string()),
        None => (runner.to_string(), full.clone()),
    };
    let name = name.replace('-', "_").trim_matches('_').to_string();
    let prefix = opt(m, "prefix");
    let env = pairs(m.get("env"));
    if task_runner == "dosbox" && name == "dosexec" {
        return Ok(Task::DosExec {
            executable: need(m, "executable", "dosexec")?,
            args: get_str(m, "args").unwrap_or_default(),
            working_dir: opt(m, "working_dir"),
            config_file: opt(m, "config_file"),
            exit: truthy(m.get("exit")).unwrap_or(true),
        });
    }
    if !task_runner.starts_with("wine") {
        return Err(tr!("The {runner} task {task} is not supported", runner = task_runner, task = name));
    }
    Ok(match name.as_str() {
        "create_prefix" => Task::CreatePrefix { prefix, arch: opt(m, "arch"), overrides: pairs(m.get("overrides")) },
        "wineexec" => Task::WineExec {
            executable: need(m, "executable", "wineexec")?,
            args: get_str(m, "args").unwrap_or_default(),
            prefix,
            working_dir: opt(m, "working_dir"),
            env,
        },
        "winetricks" => Task::Winetricks { apps: get_str(m, "app").unwrap_or_default(), prefix, env },
        "winecfg" => Task::Winecfg { prefix },
        "winekill" => Task::Winekill { prefix },
        "set_regedit" => Task::SetRegedit {
            path: need(m, "path", "set_regedit")?,
            key: need(m, "key", "set_regedit")?,
            value: match m.get("value") {
                None | Some(Value::Null) => None,
                Some(v) => Some(scalar(v).ok_or_else(|| tr!("The value of set_regedit must be text"))?),
            },
            kind: get_nonempty(m, "type").unwrap_or_else(|| "REG_SZ".into()),
            prefix,
        },
        "set_regedit_file" => Task::SetRegeditFile { filename: need(m, "filename", "set_regedit_file")?, prefix },
        "delete_registry_key" => Task::DeleteRegistryKey { key: need(m, "key", "delete_registry_key")?, prefix },
        "eject_disc" => Task::EjectDisc,
        _ => return Err(tr!("The {runner} task {task} is not supported", runner = "wine", task = name)),
    })
}

impl Step {
    /// What the step does, for the review list and the progress line: the
    /// script's description, else one made from the step.
    pub fn summary(&self) -> String {
        if let Some(d) = &self.description {
            return d.clone();
        }
        match &self.action {
            Action::Chmodx(f) => tr!("Make {file} executable", file = short(f)),
            Action::Execute(e) => match (&e.file, &e.command) {
                (Some(f), _) => tr!("Run {program}", program = short(f)),
                (None, Some(c)) => tr!("Run the command {command}", command = first_words(c)),
                _ => tr!("Run a program").into(),
            },
            Action::Extract(x) => tr!("Extract {file}", file = short(&x.file)),
            Action::InputMenu(m) if !m.description.is_empty() => m.description.clone(),
            Action::InputMenu(_) => tr!("Ask a question").into(),
            Action::InsertDisc { .. } => tr!("Ask for the game disc").into(),
            Action::Mkdir(d) => tr!("Create the folder {folder}", folder = short(d)),
            Action::Merge { src, dst, .. } => tr!("Copy {src} to {dst}", src = short(src), dst = short(dst)),
            Action::Move { src, dst, .. } => tr!("Move {src} to {dst}", src = short(src), dst = short(dst)),
            Action::Rename { src, dst } => tr!("Rename {src} to {dst}", src = short(src), dst = short(dst)),
            Action::SubstituteVars(f) => tr!("Fill in the paths of {file}", file = short(f)),
            Action::WriteFile { file, .. } | Action::WriteJson { file, .. } | Action::WriteConfig { file, .. } => {
                tr!("Write {file}", file = short(file))
            }
            Action::Task(t) => t.summary(),
            Action::InstallOrExtract(f) => tr!("Install or extract {file}", file = short(f)),
            Action::ExtractOrRun { file, .. } => tr!("Extract or run {file}", file = short(file)),
            Action::Unsupported { name, .. } => name.clone(),
        }
    }

    /// Whether the step runs programs or shell commands (shown before the
    /// install starts: the user decides whether to trust the script).
    pub fn runs_code(&self) -> bool {
        matches!(
            self.action,
            Action::Execute(_)
                | Action::InstallOrExtract(_)
                | Action::ExtractOrRun { .. }
                | Action::Task(Task::WineExec { .. } | Task::DosExec { .. })
        )
    }
}

impl Task {
    pub fn summary(&self) -> String {
        match self {
            Task::CreatePrefix { .. } => tr!("Create the Wine prefix").into(),
            Task::WineExec { executable, .. } => tr!("Run {program} with Wine", program = short(executable)),
            Task::Winetricks { apps, .. } if apps.trim().is_empty() => tr!("Open Winetricks").into(),
            Task::Winetricks { apps, .. } => tr!("Install {components} with Winetricks", components = apps.trim()),
            Task::Winecfg { .. } => tr!("Open the Wine configuration").into(),
            Task::Winekill { .. } => tr!("Stop the programs of the Wine prefix").into(),
            Task::SetRegedit { key, .. } => tr!("Set {key} in the Windows registry", key),
            Task::SetRegeditFile { filename, .. } => tr!("Import {file} into the Windows registry", file = short(filename)),
            Task::DeleteRegistryKey { key, .. } => tr!("Delete {key} from the Windows registry", key = short(key)),
            Task::EjectDisc => tr!("Eject the disc").into(),
            Task::DosExec { executable, .. } => tr!("Run {program} with DOSBox", program = short(executable)),
        }
    }
}

/// The last part of a path (`$GAMEDIR/a/setup.exe` -> `setup.exe`).
pub fn short(path: &str) -> String {
    let p = path.trim().trim_end_matches(['/', '\\']);
    p.rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or(p).to_string()
}

/// The first line of a command, cut to a few words.
fn first_words(command: &str) -> String {
    let line = command.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let mut out: String = line.chars().take(48).collect();
    if line.chars().count() > 48 || command.trim().lines().count() > 1 {
        out.push('\u{2026}');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn step(v: Value) -> Step {
        parse(&v, "wine")
    }

    #[test]
    fn wine_tasks() {
        let s = step(
            json!({"task": {"name": "winetricks", "app": "vcrun2022 unifont", "prefix": "$GAMEDIR", "env": {"PROTON_USE_XALIA": "0"}, "description": "Deps"}}),
        );
        assert_eq!(s.description.as_deref(), Some("Deps"));
        assert_eq!(
            s.action,
            Action::Task(Task::Winetricks {
                apps: "vcrun2022 unifont".into(),
                prefix: Some("$GAMEDIR".into()),
                env: vec![("PROTON_USE_XALIA".into(), "0".into())]
            })
        );
        let s = step(json!({"task": {"name": "wine.wineexec", "executable": "setup", "args": "/S", "return_code": 256}}));
        assert_eq!(s.return_code, Some(256));
        assert!(matches!(s.action, Action::Task(Task::WineExec { ref executable, ref args, .. }) if executable == "setup" && args == "/S"));
        let s =
            step(json!({"task": {"name": "set_regedit", "path": "HKEY_CURRENT_USER\\Software\\Wine", "key": "Version", "value": null}}));
        assert!(matches!(s.action, Action::Task(Task::SetRegedit { value: None, ref kind, .. }) if kind == "REG_SZ"));
        assert!(matches!(step(json!({"task": {"name": "install_cab_component"}})).action, Action::Unsupported { .. }));
        assert!(matches!(step(json!({"task": {"name": "steam.install_game"}})).action, Action::Unsupported { .. }));
    }

    #[test]
    fn file_directives() {
        assert_eq!(step(json!({"chmodx": "$GAMEDIR/start.sh"})).action, Action::Chmodx("$GAMEDIR/start.sh".into()));
        assert_eq!(step(json!({"mkdir": "$GAMEDIR/x"})).action, Action::Mkdir("$GAMEDIR/x".into()));
        let s = step(json!({"copy": {"src": "a", "dst": "b", "optional": true}}));
        assert_eq!(s.action, Action::Merge { src: "a".into(), dst: "b".into(), optional: true, remove_source: false });
        let s = step(json!({"extract": {"src": "archive", "nomerge": true, "format": "zip"}}));
        assert_eq!(s.action, Action::Extract(Extract { file: "archive".into(), dst: None, format: Some("zip".into()), nomerge: true }));
        let s = step(json!({"write_file": {"file": "a.txt", "content": "x", "mode": "a"}}));
        assert_eq!(s.action, Action::WriteFile { file: "a.txt".into(), content: "x".into(), append: true });
        assert!(matches!(step(json!({"write_file": {"file": "a", "content": "x", "mode": "r"}})).action, Action::Unsupported { .. }));
        let s = step(json!({"write_config": {"file": "a.ini", "section": "S", "key": "k", "value": 3}}));
        assert_eq!(
            s.action,
            Action::WriteConfig { file: "a.ini".into(), data: vec![("S".into(), vec![("k".into(), "3".into())])], merge: true }
        );
        let s = step(json!({"write_config": {"file": "a.ini", "merge": false, "data": {"S": {"a": 1, "b": "x"}}}}));
        assert!(matches!(s.action, Action::WriteConfig { merge: false, ref data, .. } if data[0].1.len() == 2));
    }

    #[test]
    fn execute_menu_and_disc() {
        let s = step(json!({"execute": {"command": "echo hi\nls", "env": {"A": 1}}}));
        let Action::Execute(e) = &s.action else { panic!() };
        assert_eq!((e.command.as_deref(), e.env.as_slice()), (Some("echo hi\nls"), &[("A".to_string(), "1".to_string())][..]));
        assert!(s.runs_code());
        assert_eq!(s.summary(), "Run the command echo hi\u{2026}");
        assert!(matches!(step(json!({"execute": {"command": "a", "file": "b"}})).action, Action::Unsupported { .. }));
        let s = step(
            json!({"input_menu": {"id": "LANG", "description": "Language", "options": [{"en": "English"}, {"fr": "French"}], "preselect": "en"}}),
        );
        let Action::InputMenu(m) = &s.action else { panic!() };
        assert_eq!(m.options, [("en".to_string(), "English".to_string()), ("fr".into(), "French".into())]);
        assert_eq!(s.summary(), "Language");
        let s = step(json!({"insert-disc": {"requires": "setup.exe"}}));
        assert!(matches!(s.action, Action::InsertDisc { ref requires, .. } if requires == "setup.exe"));
    }

    #[test]
    fn unknown_and_store_directives() {
        assert!(matches!(step(json!({"frobnicate": {}})).action, Action::Unsupported { ref name, .. } if name == "frobnicate"));
        assert!(matches!(step(json!({"autosetup_gog_game": "setup"})).action, Action::Unsupported { .. }));
        assert!(matches!(parse(&json!({"install_or_extract": "x"}), "linux").action, Action::Unsupported { .. }));
    }

    #[test]
    fn short_names() {
        assert_eq!(short("$GAMEDIR/drive_c/Game/setup.exe"), "setup.exe");
        assert_eq!(short("C:\\Game\\"), "Game");
        assert_eq!(short("setup"), "setup");
    }
}
