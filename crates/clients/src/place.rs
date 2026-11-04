//! Where a client lives, and where its readers find its files there: a
//! Wine prefix of its own on Linux, this PC itself on Windows. The clients
//! keep the same files at the same places on both (relative to the C:
//! drive, to a user's folder, below `HKLM`), so the readers ask a [`Place`]
//! for them and never look at which one it is.

use std::path::{Path, PathBuf};

use heronite_data::clients::{drive_c, machine_drive, machine_user, Folders};

use crate::files;
use crate::reg::RegKey;
use crate::winpath::join_ci;

/// Where a client lives (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// The Wine prefix the client was set up in: its `drive_c`, the users
    /// of `drive_c/users` and its `system.reg`.
    Prefix(PathBuf),
    /// This PC, where the client runs as it is.
    Machine(Machine),
}

/// This PC's folders and registry, as the clients' paths name them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    drive: Folders,
    /// This user's folders; another user's are not readable.
    user: Option<Folders>,
    /// Where the keys of `HKLM` are read: `None` is `HKLM` itself; tests
    /// read the keys they wrote below this key of the current user.
    registry: Option<String>,
}

impl Machine {
    /// The folders and registry of the PC this runs on.
    pub fn this_pc() -> Self {
        Self { drive: machine_drive(|k| std::env::var(k).ok()), user: machine_user(), registry: None }
    }

    /// A PC whose folders are `drive` and `user`, and whose `HKLM` is the
    /// current user's key `registry` (it may be missing).
    #[cfg(test)]
    pub(crate) fn at(drive: Folders, user: Folders, registry: &str) -> Self {
        Self { drive, user: Some(user), registry: Some(registry.to_string()) }
    }
}

impl From<&Path> for Place {
    fn from(prefix: &Path) -> Self {
        Place::Prefix(prefix.to_path_buf())
    }
}

/// A Windows user's folder of a [`Place`], where the clients keep their
/// settings and sign-in.
pub(crate) enum User<'a> {
    /// `drive_c/users/<name>` of a prefix.
    InPrefix(PathBuf),
    Machine(&'a Folders),
}

impl User<'_> {
    /// `rel` (`AppData/Roaming/...`, parts separated by `/`) for this user.
    pub(crate) fn path(&self, rel: &str) -> PathBuf {
        match self {
            User::InPrefix(dir) => join_ci(dir, rel),
            User::Machine(folders) => folders.path(rel),
        }
    }
}

impl Place {
    /// This PC.
    pub fn machine() -> Self {
        Place::Machine(Machine::this_pc())
    }

    /// The prefix, when the client lives in one.
    pub fn prefix(&self) -> Option<&Path> {
        match self {
            Place::Prefix(p) => Some(p),
            Place::Machine(_) => None,
        }
    }

    /// `rel` (relative to the C: drive, parts separated by `/`: `ProgramData/...`).
    pub(crate) fn drive(&self, rel: &str) -> PathBuf {
        match self {
            Place::Prefix(prefix) => join_ci(&drive_c(prefix), rel),
            Place::Machine(m) => m.drive.path(rel),
        }
    }

    /// The Windows users whose folders the client may have written to: a
    /// prefix's (`steamuser` under Proton, the login name under Wine; not
    /// `Public`), this PC's current user.
    pub(crate) fn users(&self) -> Vec<User<'_>> {
        match self {
            Place::Prefix(prefix) => crate::user_dirs(prefix).into_iter().map(User::InPrefix).collect(),
            Place::Machine(m) => m.user.iter().map(User::Machine).collect(),
        }
    }

    /// The keys of `HKLM` under `under` (single backslashes, any case). A
    /// prefix gives every key below them (`system.reg`); this PC the keys
    /// themselves and the ones directly below, which is all the readers ask.
    pub(crate) fn machine_keys(&self, under: &[&str]) -> Vec<RegKey> {
        match self {
            Place::Prefix(prefix) => files::machine_keys(prefix, under),
            Place::Machine(m) => native_keys(m.registry.as_deref(), under),
        }
    }

    /// The folder (or file) of a Windows path the client wrote
    /// (`C:\Games\x`, `C:/Games/x/`): through the prefix's drives, or as it
    /// is on this PC. `None` for a path without a drive.
    pub(crate) fn local_path(&self, win: &str) -> Option<PathBuf> {
        match self {
            Place::Prefix(prefix) => files::unix_path(prefix, win),
            Place::Machine(_) => windows_path(win),
        }
    }
}

/// A Windows path as this PC's path: one kind of separator, no trailing
/// one, `.` and `..` folded. `None` without a drive letter (or `\\server`).
fn windows_path(win: &str) -> Option<PathBuf> {
    let win = win.trim().trim_matches('"').replace('/', "\\");
    let b = win.as_bytes();
    let root = if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        format!("{}\\", &win[..2])
    } else if win.starts_with("\\\\") {
        return Some(PathBuf::from(win.trim_end_matches('\\')));
    } else {
        return None;
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in win[2..].split('\\').filter(|p| !p.is_empty() && *p != ".") {
        if part == ".." {
            parts.pop();
        } else {
            parts.push(part);
        }
    }
    Some(parts.into_iter().fold(PathBuf::from(root), |p, part| p.join(part)))
}

/// The keys at `under` and directly below them, from this PC's `HKLM` (or
/// the current user's `base`, in tests).
#[cfg(windows)]
fn native_keys(base: Option<&str>, under: &[&str]) -> Vec<RegKey> {
    use std::collections::BTreeMap;

    use heronite_platform::registry::{self, Hive, Value};

    use crate::reg::RegValue;

    let (hive, prefix) = match base {
        None => (Hive::LocalMachine, String::new()),
        Some(base) => (Hive::CurrentUser, format!("{base}\\")),
    };
    let read = |path: String| {
        let values = registry::values(hive, &format!("{prefix}{path}"))?;
        let values: BTreeMap<String, RegValue> = values
            .into_iter()
            .map(|(name, value)| {
                let value = match value {
                    Value::Str(s) => RegValue::Str(s),
                    Value::Dword(n) => RegValue::Dword(n),
                    Value::Other => RegValue::Other,
                };
                (name.to_lowercase(), value)
            })
            .collect();
        Some(RegKey { path, values })
    };
    let mut out = Vec::new();
    for path in under.iter().map(|u| u.trim_end_matches('\\')) {
        out.extend(read(path.to_string()));
        for sub in registry::subkeys(hive, &format!("{prefix}{path}")) {
            out.extend(read(format!("{path}\\{sub}")));
        }
    }
    out
}

/// No registry here.
#[cfg(not(windows))]
fn native_keys(_base: Option<&str>, _under: &[&str]) -> Vec<RegKey> {
    Vec::new()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A PC in `dir`: `drive` holds its C: drive (`Program Files` and the
    /// rest), `user` its user's folder; its `HKLM` is the current user's
    /// key `registry`.
    pub(crate) fn machine_in(dir: &Path, registry: &str) -> (Place, PathBuf, PathBuf) {
        let (drive, user) = (dir.join("drive"), dir.join("user"));
        let moved = ["Program Files (x86)", "Program Files", "ProgramData"];
        let drive_folders =
            Folders { root: drive.clone(), moved: moved.iter().map(|m| (m.to_string(), drive.join(m.replace(' ', "_")))).collect() };
        // OneDrive keeps the documents elsewhere.
        let user_folders = Folders { root: user.clone(), moved: vec![("Documents".into(), dir.join("OneDrive/Documents"))] };
        (Place::Machine(Machine::at(drive_folders, user_folders, registry)), drive, user)
    }

    #[test]
    fn windows_paths_are_this_pcs_as_they_are() {
        let at = |parts: &[&str]| parts.iter().fold(PathBuf::from("C:\\"), |p, part| p.join(part));
        let ubisoft = "C:/Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games/Far Cry 3/";
        assert_eq!(windows_path(ubisoft), Some(at(&["Program Files (x86)", "Ubisoft", "Ubisoft Game Launcher", "games", "Far Cry 3"])));
        assert_eq!(windows_path("\"C:\\Games\\.\\EA\\..\\Hades\""), Some(at(&["Games", "Hades"])));
        assert_eq!(windows_path("C:\\..\\x"), Some(at(&["x"])));
        assert_eq!(windows_path(r"\\nas\games\x\"), Some(PathBuf::from(r"\\nas\games\x")));
        assert_eq!(windows_path("games\\x"), None);
        assert_eq!(windows_path(""), None);
    }

    #[test]
    fn a_prefix_and_this_pc_name_the_same_files_their_way() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = Place::from(dir.path());
        assert_eq!(prefix.prefix(), Some(dir.path()));
        assert_eq!(prefix.drive("ProgramData/x"), dir.path().join("drive_c").join("ProgramData").join("x"));

        let (pc, drive, user) = machine_in(dir.path(), r"Software\HeroniteClientsTest\none");
        assert_eq!(pc.prefix(), None);
        assert_eq!(
            pc.drive("ProgramData/Battle.net/Agent/product.db"),
            drive.join("ProgramData").join("Battle.net").join("Agent").join("product.db")
        );
        assert_eq!(pc.drive("Games/x"), drive.join("Games").join("x"));
        let users = pc.users();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].path("AppData/Roaming/Battle.net"), user.join("AppData").join("Roaming").join("Battle.net"));
        assert_eq!(users[0].path("Documents/Rockstar Games"), dir.path().join("OneDrive/Documents").join("Rockstar Games"));
        assert!(pc.machine_keys(&["Software\\Wow6432Node\\Ubisoft\\Launcher\\Installs"]).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn this_pcs_keys_are_the_ones_asked_and_the_ones_below() {
        use heronite_platform::registry::{set_string, Hive};
        let base = format!(r"Software\HeroniteClientsTest\{}-keys", std::process::id());
        let installs = "Software\\Wow6432Node\\Ubisoft\\Launcher\\Installs";
        set_string(Hive::CurrentUser, &format!(r"{base}\{installs}"), "Own", "x").unwrap();
        set_string(Hive::CurrentUser, &format!(r"{base}\{installs}\635"), "InstallDir", "C:/Games/Far Cry 3/").unwrap();
        set_string(Hive::CurrentUser, &format!(r"{base}\{installs}\635\Deeper"), "Not", "read").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (pc, ..) = machine_in(dir.path(), &base);
        let keys = pc.machine_keys(&[installs, "Software\\Missing"]);
        let paths: Vec<String> = keys.iter().map(|k| k.path.clone()).collect();
        assert_eq!(paths, [installs.to_string(), format!("{installs}\\635")]);
        assert!(keys[1].is_child_of(installs));
        assert_eq!(keys[1].str("installdir"), Some("C:/Games/Far Cry 3/"));
        assert_eq!(pc.local_path(keys[1].str("InstallDir").unwrap()), Some(PathBuf::from(r"C:\Games\Far Cry 3")));
        heronite_platform::registry::delete_tree(Hive::CurrentUser, &base).unwrap();
    }
}
