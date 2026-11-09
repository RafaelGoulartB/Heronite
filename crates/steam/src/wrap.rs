//! The script that runs a Steam game's before/after scripts: Steam starts it
//! in front of the game (`heronite-steam-wrap --before a --after b --
//! %command%`), so they run however the game is opened (this app, the Steam
//! library, Big Picture).

use std::fs;
use std::io;
use std::path::PathBuf;

use heronite_data::DataPaths;

use crate::options::WRAP_NAME;

/// The variables match what the scripts get when this app launches a game
/// (`HEROIC_GAME_SCRIPT_TYPE` & co.); Steam adds `SteamAppId`.
const SCRIPT: &str = r#"#!/bin/sh
# Written by Heronite. Runs a Steam game's scripts around it:
#   heronite-steam-wrap [--before SCRIPT] [--after SCRIPT] -- COMMAND...
before=
after=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --before) before=$2; shift 2 ;;
    --after) after=$2; shift 2 ;;
    --) shift; break ;;
    *) break ;;
  esac
done
run_script() {
  HEROIC_GAME_SCRIPT_TYPE=$1 HERONITE_GAME_SCRIPT_TYPE=$1 HEROIC_GAME_RUNNER=steam HERONITE_GAME_RUNNER=steam "$2"
}
[ -n "$before" ] && run_script before "$before"
"$@"
status=$?
[ -n "$after" ] && run_script after "$after"
exit $status
"#;

/// `<data>/tools/bin/heronite-steam-wrap`.
pub fn path(paths: &DataPaths) -> PathBuf {
    paths.tools_dir().join("bin").join(WRAP_NAME)
}

/// Writes the script when it is missing or out of date.
pub fn install(paths: &DataPaths) -> io::Result<PathBuf> {
    let file = path(paths);
    if fs::read_to_string(&file).is_ok_and(|t| t == SCRIPT) {
        return Ok(file);
    }
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    heronite_data::temps::write_atomic(&file, SCRIPT.as_bytes(), Some(0o755))?;
    Ok(file)
}

// The scripts are shell scripts.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn runs_the_scripts_around_the_command_and_keeps_its_status() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path());
        let wrap = install(&paths).unwrap();
        let log = dir.path().join("log");
        let script = |name: &str| {
            let f = dir.path().join(name);
            fs::write(&f, format!("#!/bin/sh\necho \"{name} $HERONITE_GAME_SCRIPT_TYPE\" >> '{}'\n", log.display())).unwrap();
            Command::new("chmod").arg("+x").arg(&f).status().unwrap();
            f
        };
        let (before, after) = (script("b.sh"), script("a.sh"));
        let status = Command::new(&wrap)
            .args(["--before".as_ref(), before.as_os_str(), "--after".as_ref(), after.as_os_str(), "--".as_ref()])
            .args(["sh", "-c", &format!("echo game >> '{}'; exit 3", log.display())])
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(3));
        assert_eq!(fs::read_to_string(&log).unwrap(), "b.sh before\ngame\na.sh after\n");
        // Up to date: not rewritten.
        let modified = fs::metadata(&wrap).unwrap().modified().unwrap();
        install(&paths).unwrap();
        assert_eq!(fs::metadata(&wrap).unwrap().modified().unwrap(), modified);
    }
}
