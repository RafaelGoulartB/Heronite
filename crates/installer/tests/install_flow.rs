//! A whole install of a native Linux script in a temporary data folder:
//! files, steps, questions, and the game it becomes. Needs `tar` and `bash`.

// A Linux game's script: Linux only.
#![cfg(unix)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use heronite_data::DataPaths;
use heronite_installer::{install, Answer, Event, Host, Job, Question};
use serde_json::Value;

#[derive(Default)]
struct FakeHost {
    events: Mutex<Vec<Event>>,
    questions: Mutex<Vec<Question>>,
    cancel: AtomicBool,
}

impl Host for FakeHost {
    fn event(&self, event: Event) {
        self.events.lock().unwrap().push(event);
    }
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
    fn ask(&self, question: Question) -> Option<Answer> {
        self.questions.lock().unwrap().push(question.clone());
        match question {
            Question::Menu { preselect, options, .. } => Some(Answer::Choice(preselect.unwrap_or_else(|| options[0].0.clone()))),
            Question::Disc { .. } => None,
        }
    }
}

const SCRIPT: &str = r#"
name: Tiny Game
game_slug: tiny-game
version: Linux
runner: linux
script:
  variables:
    BUILD: 42
  files:
  - archive: $SCRIPTDIR/tiny.tar.gz
  - readme: N/A:Select the readme
  game:
    exe: start.sh
    args: --windowed
    launch_configs:
    - exe: tools/editor.sh
      name: Editor
  installer:
  - extract:
      file: archive
      dst: $GAMEDIR
  - input_menu:
      id: LANG
      description: Language
      options:
      - en: English
      - pt: Português
      preselect: pt
  - write_file:
      file: $GAMEDIR/lang.txt
      content: "lang=$INPUT_LANG build=${BUILD}"
  - execute:
      command: echo "hi $TINY_ENV" > "$GAMEDIR/out.txt"; echo done
  - move:
      src: readme
      dst: $GAMEDIR/docs
  - chmodx: $GAMEDIR/start.sh
  - write_config:
      file: $GAMEDIR/game.ini
      section: Video
      key: Width
      value: $RESOLUTION_WIDTH
  - write_json:
      file: $GAMEDIR/settings.json
      data: {a: 1}
  system:
    env:
      TINY_ENV: $GAMEDIR/x
    gamemode: true
"#;

fn have(program: &str) -> bool {
    Command::new("sh").arg("-c").arg(format!("command -v {program}")).output().map(|o| o.status.success()).unwrap_or(false)
}

struct Setup {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let src = root.join("src/Tiny-1.0");
    fs::create_dir_all(src.join("tools")).unwrap();
    fs::write(src.join("start.sh"), "#!/bin/sh\necho game\n").unwrap();
    fs::write(src.join("tools/editor.sh"), "#!/bin/sh\n").unwrap();
    fs::create_dir_all(root.join("scripts")).unwrap();
    let ok = Command::new("tar").arg("-czf").arg(root.join("scripts/tiny.tar.gz")).arg("-C").arg(root.join("src")).arg("Tiny-1.0").status();
    assert!(ok.unwrap().success());
    fs::write(root.join("README.txt"), "read me").unwrap();
    fs::create_dir_all(root.join("home")).unwrap();
    Setup { _dir: dir, root }
}

fn job(root: &Path, text: &str) -> Job {
    let script = heronite_installer::parse(text, Some(&root.join("scripts")), "x").unwrap().remove(0);
    let mut provided = HashMap::new();
    provided.insert("readme".to_string(), root.join("README.txt"));
    Job {
        title: script.title().to_string(),
        game_dir: root.join("games").join(script.folder_name()),
        script,
        app_name: "tinyApp".into(),
        wine_version: None,
        provided,
        resolution: Some((1920, 1080)),
        cache_root: root.join("cache"),
        paths: DataPaths::at(root.join("data")),
        home: root.join("home").to_string_lossy().into_owned(),
        resume: None,
        sandbox: Default::default(),
    }
}

fn read_json(p: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn a_linux_script_installs_and_becomes_a_local_game() {
    if !have("tar") || !have("bash") {
        return;
    }
    let s = setup();
    let job = job(&s.root, SCRIPT);
    let check = heronite_installer::check(&job.script);
    assert!(!check.blocked(), "{:?}", check.issues);
    let host = FakeHost::default();
    let out = install(&job, &host).unwrap_or_else(|f| panic!("{}", f.message));
    assert_eq!((out.app_name.as_str(), out.title.as_str()), ("tinyApp", "Tiny Game"));

    let g = &job.game_dir;
    assert_eq!(fs::read_to_string(g.join("lang.txt")).unwrap(), "lang=pt build=42");
    assert_eq!(fs::read_to_string(g.join("out.txt")).unwrap().trim(), format!("hi {}/x", g.display()));
    assert_eq!(fs::read_to_string(g.join("docs")).unwrap(), "read me", "moved to the path given");
    assert!(s.root.join("README.txt").exists(), "a file the user picked is copied, not moved");
    assert!(fs::read_to_string(g.join("game.ini")).unwrap().contains("Width=1920"));
    assert_eq!(read_json(&g.join("settings.json"))["a"], 1);
    use std::os::unix::fs::PermissionsExt;
    assert!(fs::metadata(g.join("start.sh")).unwrap().permissions().mode() & 0o111 != 0);
    assert!(!job.cache_dir().exists(), "the downloads folder goes away");

    let lib = read_json(&job.paths.sideload_library());
    let entry = &lib["games"][0];
    assert_eq!(entry["app_name"], "tinyApp");
    assert_eq!(entry["install"]["executable"], g.join("start.sh").to_string_lossy().as_ref());
    assert_eq!(entry["install"]["platform"], "linux");
    assert_eq!(entry["install"]["install_path"], g.to_string_lossy().as_ref());

    let cfg = read_json(&job.paths.games_config_dir().join("tinyApp.json"));
    let c = &cfg["tinyApp"];
    assert_eq!(c["launcherArgs"], "--windowed");
    assert_eq!(c["useGameMode"], true);
    assert_eq!(c["enviromentOptions"][0]["key"], "TINY_ENV");
    assert_eq!(c["enviromentOptions"][0]["value"], format!("{}/x", g.display()));

    let actions = heronite_data::game_actions::load(&job.paths);
    let list = &actions["sideload_tinyApp"];
    assert_eq!(list[0].name, "Editor");
    assert_eq!(list[0].target, g.join("tools/editor.sh").to_string_lossy());

    let events = host.events.lock().unwrap();
    let stages: Vec<usize> = events.iter().filter_map(|e| if let Event::Stage(i) = e { Some(*i) } else { None }).collect();
    assert_eq!(stages, (0..=job.script.steps.len() + 1).collect::<Vec<_>>());
    assert!(events.iter().any(|e| matches!(e, Event::Log(l) if l == "done")), "program output reaches the log");
    assert_eq!(host.questions.lock().unwrap().len(), 1);
}

#[test]
fn a_failing_step_stops_with_its_name() {
    if !have("bash") {
        return;
    }
    let s = setup();
    let text = "name: Broken\nrunner: linux\nscript:\n  game: {exe: a.sh}\n  installer:\n  - execute:\n      command: exit 3\n      description: Patching\n  - execute: {command: exit 2, return_code: 2}\n";
    let job = job(&s.root, text);
    let host = FakeHost::default();
    let f = install(&job, &host).unwrap_err();
    assert!(f.message.starts_with("Patching: "), "{}", f.message);
    assert!(f.message.contains('3'), "{}", f.message);
    assert!(!f.cancelled && f.created_dir);
    heronite_installer::engine::remove_game_dir(&job).unwrap();
    assert!(!job.game_dir.exists());

    let accepted =
        "name: Fine\nrunner: linux\nscript:\n  game: {exe: a.sh}\n  installer:\n  - execute: {command: exit 2, return_code: 2}\n";
    let job = self::job(&s.root, accepted);
    install(&job, &FakeHost::default()).unwrap_or_else(|f| panic!("{}", f.message));
}

#[test]
fn a_cancelled_install_says_so() {
    let s = setup();
    let job = job(&s.root, SCRIPT);
    let host = FakeHost::default();
    host.cancel.store(true, Ordering::SeqCst);
    let f = install(&job, &host).unwrap_err();
    assert!(f.cancelled);
}

#[test]
fn a_failed_install_resumes_at_its_failed_step() {
    if !have("bash") {
        return;
    }
    let s = setup();
    let text = r#"
name: Resumable
runner: linux
script:
  game: {exe: run.sh}
  installer:
  - write_file: {file: $GAMEDIR/count.txt, content: "x", mode: a}
  - input_menu: {id: SIDE, description: Side, options: [{l: Left}, {r: Right}], preselect: r}
  - execute: {command: "test -f $GAMEDIR/ready", description: Waiting for ready}
  - write_file: {file: $GAMEDIR/side.txt, content: $INPUT_SIDE}
"#;
    let mut job = job(&s.root, text);
    let host = FakeHost::default();
    let f = install(&job, &host).unwrap_err();
    assert_eq!(f.step, Some(2));
    assert!(f.created_dir);
    assert_eq!(f.answers, [("INPUT".to_string(), "r".to_string()), ("INPUT_SIDE".to_string(), "r".to_string())]);

    fs::write(job.game_dir.join("ready"), "").unwrap();
    job.resume = Some(heronite_installer::Resume { from: 2, answers: f.answers.clone(), created_dir: f.created_dir });
    let again = FakeHost::default();
    install(&job, &again).unwrap_or_else(|f| panic!("{}", f.message));
    assert_eq!(fs::read_to_string(job.game_dir.join("count.txt")).unwrap(), "x", "steps before the failed one do not run again");
    assert_eq!(fs::read_to_string(job.game_dir.join("side.txt")).unwrap(), "r", "earlier answers are kept");
    assert!(again.questions.lock().unwrap().is_empty());
}
