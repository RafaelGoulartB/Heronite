//! System packages (.deb, .rpm, Arch) made by `scripts/packages.sh`. Each one
//! installs `share/heronite/package-format` beside `bin/`, holding its
//! format. Files owned by a package manager must not be replaced behind its
//! back: the app downloads the release's file of the same format, checks its
//! signature like the AppImage's, and hands it to the package manager
//! through `pkexec`.

use heronite_i18n::tr;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::install::InstallError;
use crate::release::ARCH;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Package {
    Deb,
    Rpm,
    Arch,
}

impl Package {
    /// The marker, relative to the prefix (`/usr`).
    pub const MARKER: &'static str = "share/heronite/package-format";

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "deb" => Some(Package::Deb),
            "rpm" => Some(Package::Rpm),
            "arch" => Some(Package::Arch),
            _ => None,
        }
    }

    /// The package `exe` (`<prefix>/bin/heronite-rs`) belongs to, if any.
    pub fn detect(exe: &Path) -> Option<Self> {
        let prefix = exe.parent()?.parent()?;
        Self::parse(&std::fs::read_to_string(prefix.join(Self::MARKER)).ok()?)
    }

    /// `.deb`, `.rpm`, `Arch`: "installed from a … package".
    pub fn label(self) -> &'static str {
        match self {
            Package::Deb => ".deb",
            Package::Rpm => ".rpm",
            Package::Arch => "Arch",
        }
    }

    /// The command a user types to install a downloaded `file` (an absolute
    /// path: apt and dnf need a `/` to read a file instead of a name).
    pub fn install_command(self, file: &str) -> String {
        match self {
            Package::Deb => format!("sudo apt install {file}"),
            Package::Rpm => format!("sudo dnf install {file}"),
            Package::Arch => format!("sudo pacman -U {file}"),
        }
    }

    /// End of this format's file name in a release (nfpm's default names:
    /// `heronite_0.2.0-1_amd64.deb`, `heronite-0.2.0-1.x86_64.rpm`,
    /// `heronite-0.2.0-1-x86_64.pkg.tar.zst`).
    pub fn file_suffix(self) -> String {
        match self {
            Package::Deb => format!("_{}.deb", if ARCH == "aarch64" { "arm64" } else { "amd64" }),
            Package::Rpm => format!(".{ARCH}.rpm"),
            Package::Arch => format!("-{ARCH}.pkg.tar.zst"),
        }
    }
}

/// Finds `name` in the folders of `path` (a `PATH` value).
fn which_in(name: &str, path: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path).map(|d| d.join(name)).find(|p| p.is_file())
}

/// Finds `name` on `PATH`.
fn which(name: &str) -> Option<PathBuf> {
    which_in(name, &std::env::var_os("PATH")?)
}

/// Exit status of [`ROOT_SCRIPT`] when the copy does not have the verified
/// hash. Package managers exit with 0..=8 and 100..=107, `pkexec` with
/// 126/127.
const EXIT_CHANGED: i32 = 71;

/// Where [`ROOT_SCRIPT`] makes its private directory.
const ROOT_TMP: &str = "/var/tmp";

/// What `pkexec` runs as root. The downloaded file sits in a folder the user
/// can write to, so anything of theirs could swap it after the signature was
/// checked. The script copies it into a directory only root can write to,
/// checks the copy against the SHA-256 of the bytes the signature covered, and
/// installs the copy. Everything variable comes as an argument, never as script
/// text: `$1` the downloaded file, `$2` its SHA-256, `$3` the folder for the
/// private directory, then the package manager and its options. The copy
/// stays unreadable to others until it is checked (the path may have been
/// swapped for a link to a file only root reads), and readable after it, so
/// apt's sandbox user can open it. The package manager runs with the usual
/// root umask, so the files it writes itself stay readable.
const ROOT_SCRIPT: &str = r#"set -eu
umask 077
file=$1 hash=$2 base=$3
shift 3
name=${file##*/}
dir=$(mktemp -d "$base/heronite-update.XXXXXX")
trap 'rm -rf "$dir"' EXIT
chmod 711 "$dir"
install -m 0600 -- "$file" "$dir/$name"
sum=$(sha256sum -- "$dir/$name")
if [ "${sum%% *}" != "$hash" ]; then
    echo "the update file changed after it was verified" >&2
    exit 71
fi
chmod 0644 "$dir/$name"
umask 022
"$@" "$dir/$name"
"#;

/// Whether `text` is a lowercase hex SHA-256.
fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl Package {
    /// The package manager and its options (run as root); `None` when the
    /// manager is not on `PATH`. Dependencies come from the configured
    /// repositories. The package file is appended by [`ROOT_SCRIPT`].
    fn manager(self) -> Option<Vec<OsString>> {
        self.manager_in(&std::env::var_os("PATH")?)
    }

    fn manager_in(self, path: &OsStr) -> Option<Vec<OsString>> {
        let which = |name: &str| which_in(name, path);
        let command = |tool: PathBuf, rest: &[&str]| {
            let mut v = vec![tool.into_os_string()];
            v.extend(rest.iter().map(OsString::from));
            v
        };
        match self {
            Package::Deb => which("apt-get").map(|t| command(t, &["install", "-y"])),
            Package::Rpm => which("dnf")
                .map(|t| command(t, &["install", "-y"]))
                .or_else(|| which("zypper").map(|t| command(t, &["--non-interactive", "install", "--allow-unsigned-rpm"]))),
            Package::Arch => which("pacman").map(|t| command(t, &["-U", "--noconfirm"])),
        }
    }

    /// The program and arguments `pkexec` runs to install `file`, whose bytes
    /// must have the SHA-256 `sha256` (lowercase hex): [`ROOT_SCRIPT`] with
    /// `file`, `sha256`, the temp folder and the package manager call as
    /// arguments. `None` when the manager is not on `PATH`.
    pub fn install_args(self, file: &Path, sha256: &str) -> Option<Vec<OsString>> {
        self.manager().map(|manager| root_args(file, sha256, Path::new(ROOT_TMP), manager))
    }

    /// Installs the package `file` that [`crate::install::download_signed`]
    /// verified, through `pkexec` (the desktop's password prompt). Blocks until
    /// the package manager is done. On failure the caller offers
    /// [`Package::install_command`].
    pub fn install(self, file: &Path) -> Result<(), InstallError> {
        let sha256 =
            crate::install::verified_sha256(file).ok_or_else(|| InstallError::Failed(tr!("The update file was not verified").into()))?;
        self.install_verified(file, &sha256)
    }

    /// [`Package::install`] for a `file` whose verified SHA-256 is `sha256`.
    pub fn install_verified(self, file: &Path, sha256: &str) -> Result<(), InstallError> {
        if !is_sha256(sha256) || file.file_name().is_none() {
            return Err(InstallError::Failed(tr!("The update file was not verified").into()));
        }
        let pkexec = which("pkexec").ok_or_else(|| InstallError::Failed(tr!("pkexec is not installed").into()))?;
        let args = self.install_args(file, sha256).ok_or_else(|| InstallError::Failed(tr!("No package manager found").into()))?;
        let out = Command::new(pkexec)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| InstallError::Failed(tr!("Could not run pkexec: {error}", error = e)))?;
        install_result(out.status.code(), &out.stdout, &out.stderr)
    }
}

/// `sh -c ROOT_SCRIPT sh <file> <sha256> <base> <manager...>`.
fn root_args(file: &Path, sha256: &str, base: &Path, manager: Vec<OsString>) -> Vec<OsString> {
    let mut v: Vec<OsString> =
        vec!["/bin/sh".into(), "-c".into(), ROOT_SCRIPT.into(), "sh".into(), file.into(), sha256.into(), base.into()];
    v.extend(manager);
    v
}

/// The outcome of the `pkexec` call from its exit status and output.
fn install_result(code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> Result<(), InstallError> {
    match code {
        Some(0) => Ok(()),
        // The password prompt was dismissed.
        Some(126) => Err(InstallError::Cancelled),
        Some(127) => Err(InstallError::Failed(tr!("Not authorized (is a polkit agent running?)").into())),
        Some(EXIT_CHANGED) => Err(InstallError::Failed(tr!("The update file changed after it was verified").into())),
        _ => {
            let text = String::from_utf8_lossy(stderr);
            let text = if text.trim().is_empty() { String::from_utf8_lossy(stdout) } else { text };
            let tail: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            let tail = tail[tail.len().saturating_sub(3)..].join(" ");
            Err(InstallError::Failed(tr!("The package manager failed: {tail}", tail)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Package;

    #[test]
    fn detects_the_marker_beside_bin() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("usr/bin/heronite-rs");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        assert_eq!(Package::detect(&exe), None);
        let marker = tmp.path().join("usr").join(Package::MARKER);
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, "rpm\n").unwrap();
        assert_eq!(Package::detect(&exe), Some(Package::Rpm));
        std::fs::write(&marker, "snap").unwrap();
        assert_eq!(Package::detect(&exe), None);
        assert_eq!(Package::Deb.install_command("/tmp/heronite_0.2.0_amd64.deb"), "sudo apt install /tmp/heronite_0.2.0_amd64.deb");
        if super::ARCH == "x86_64" {
            assert_eq!(Package::Deb.file_suffix(), "_amd64.deb");
            assert_eq!(Package::Arch.file_suffix(), "-x86_64.pkg.tar.zst");
        }
    }

    use super::{install_result, is_sha256, root_args, EXIT_CHANGED, ROOT_SCRIPT};
    use crate::install::InstallError;
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::{Command, Output};

    const BYTES: &[u8] = b"the signed package";

    /// A package manager that records its arguments and the bytes of the
    /// package file it was given, and where that file lives.
    fn fake_manager(dir: &Path) -> OsString {
        let tool = dir.join("apt-get");
        let log = dir.join("log");
        let script = format!(
            "#!/bin/sh\nfor a; do last=$a; done\necho \"$*\" > '{log}'\ncat \"$last\" > '{log}.bytes'\nstat -c %a \"$last\" > '{log}.mode'\n",
            log = log.display()
        );
        std::fs::write(&tool, script).unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        tool.into_os_string()
    }

    /// Runs the root-side script as the current user (no `pkexec`), with a
    /// private folder under `base`.
    fn run_script(file: &Path, sha256: &str, base: &Path, manager: Vec<OsString>) -> Output {
        let mut args = root_args(file, sha256, base, manager).into_iter();
        Command::new(args.next().unwrap()).args(args).output().unwrap()
    }

    fn sha256_of(bytes: &[u8]) -> String {
        use std::io::Write;
        let mut child = Command::new("sha256sum").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        let out = child.wait_with_output().unwrap();
        String::from_utf8(out.stdout).unwrap().split_whitespace().next().unwrap().to_owned()
    }

    #[test]
    fn the_root_call_passes_the_file_and_hash_as_arguments_not_script_text() {
        let file = Path::new("/home/u/.cache/heronite/updates/x'; touch /pwned; '.deb");
        let hash = "ab".repeat(32);
        let args = root_args(file, &hash, Path::new("/var/tmp"), vec!["/usr/bin/apt-get".into(), "install".into(), "-y".into()]);
        let args: Vec<&str> = args.iter().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(&args[..4], ["/bin/sh", "-c", ROOT_SCRIPT, "sh"]);
        assert_eq!(&args[4..], [file.to_str().unwrap(), hash.as_str(), "/var/tmp", "/usr/bin/apt-get", "install", "-y"]);
        assert!(!ROOT_SCRIPT.contains("/home") && !ROOT_SCRIPT.contains(&hash) && !ROOT_SCRIPT.contains(".deb"));
        assert!(ROOT_SCRIPT.contains(&format!("exit {EXIT_CHANGED}\n")));
        assert!(is_sha256(&hash) && !is_sha256("AB") && !is_sha256(&"AB".repeat(32)));
    }

    #[test]
    fn each_format_keeps_its_manager_options() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path();
        for tool in ["apt-get", "dnf", "zypper", "pacman"] {
            let path = bin.join(tool);
            std::fs::write(&path, "").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let tail = |p: super::Package| {
            let args = p.manager_in(bin.as_os_str()).unwrap();
            args.iter().map(|a| a.to_str().unwrap().replace(bin.to_str().unwrap(), "")).collect::<Vec<_>>()
        };
        let got = (tail(super::Package::Deb), tail(super::Package::Rpm), tail(super::Package::Arch));
        std::fs::remove_file(bin.join("dnf")).unwrap();
        let zypper = tail(super::Package::Rpm);
        assert_eq!(got.0, ["/apt-get", "install", "-y"]);
        assert_eq!(got.1, ["/dnf", "install", "-y"]);
        assert_eq!(got.2, ["/pacman", "-U", "--noconfirm"]);
        assert_eq!(zypper, ["/zypper", "--non-interactive", "install", "--allow-unsigned-rpm"]);
    }

    #[test]
    fn installs_the_checked_private_copy_not_the_downloaded_file() {
        let tmp = tempfile::tempdir().unwrap();
        let (downloads, base, bin) = (tmp.path().join("updates"), tmp.path().join("tmp"), tmp.path().join("bin"));
        for d in [&downloads, &base, &bin] {
            std::fs::create_dir(d).unwrap();
        }
        let file = downloads.join("heronite_1.0_amd64.deb");
        std::fs::write(&file, BYTES).unwrap();
        let tool = fake_manager(&bin);
        let out = run_script(&file, &sha256_of(BYTES), &base, vec![tool, "install".into(), "-y".into()]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let log = std::fs::read_to_string(bin.join("log")).unwrap();
        let installed = log.trim().strip_prefix("install -y ").unwrap();
        assert!(installed.starts_with(base.to_str().unwrap()) && installed.ends_with("/heronite_1.0_amd64.deb"), "{log}");
        assert_ne!(Path::new(installed), file);
        assert_eq!(std::fs::read(bin.join("log.bytes")).unwrap(), BYTES);
        assert_eq!(std::fs::read_to_string(bin.join("log.mode")).unwrap().trim(), "644");
        // The private folder is gone after the install.
        assert_eq!(std::fs::read_dir(&base).unwrap().count(), 0);
    }

    #[test]
    fn a_file_changed_after_the_check_installs_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let (downloads, base, bin) = (tmp.path().join("updates"), tmp.path().join("tmp"), tmp.path().join("bin"));
        for d in [&downloads, &base, &bin] {
            std::fs::create_dir(d).unwrap();
        }
        let file = downloads.join("heronite-1.0.x86_64.rpm");
        std::fs::write(&file, BYTES).unwrap();
        let verified = sha256_of(BYTES);
        // Swapped between the signature check and the install.
        std::fs::write(&file, b"another package").unwrap();
        let tool = fake_manager(&bin);
        let out = run_script(&file, &verified, &base, vec![tool, "install".into()]);
        assert_eq!(out.status.code(), Some(EXIT_CHANGED));
        assert!(!bin.join("log").exists(), "the package manager ran");
        assert_eq!(std::fs::read_dir(&base).unwrap().count(), 0);
        assert!(
            matches!(install_result(out.status.code(), &out.stdout, &out.stderr), Err(InstallError::Failed(m)) if m.contains("changed"))
        );
    }

    #[test]
    fn a_failing_manager_keeps_its_exit_status_and_cleans_up() {
        let tmp = tempfile::tempdir().unwrap();
        let (base, bin) = (tmp.path().join("tmp"), tmp.path().join("bin"));
        std::fs::create_dir(&base).unwrap();
        std::fs::create_dir(&bin).unwrap();
        let file = tmp.path().join("p.deb");
        std::fs::write(&file, BYTES).unwrap();
        let tool = bin.join("fail");
        std::fs::write(&tool, "#!/bin/sh\necho broken >&2\nexit 100\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = run_script(&file, &sha256_of(BYTES), &base, vec![tool.into_os_string()]);
        assert_eq!(out.status.code(), Some(100));
        assert_eq!(std::fs::read_dir(&base).unwrap().count(), 0);
    }

    #[test]
    fn a_file_without_a_verified_hash_is_refused() {
        let file = Path::new("/nonexistent/never-downloaded.deb");
        let refused = |r: Result<(), InstallError>| matches!(r, Err(InstallError::Failed(m)) if m == "The update file was not verified");
        assert!(refused(super::Package::Deb.install(file)));
        assert!(refused(super::Package::Deb.install_verified(file, "not a hash")));
    }

    #[test]
    fn exit_codes_map_to_cancel_and_translated_errors() {
        assert_eq!(install_result(Some(0), b"", b""), Ok(()));
        assert_eq!(install_result(Some(126), b"", b""), Err(InstallError::Cancelled));
        let text = |r: Result<(), InstallError>| match r {
            Err(InstallError::Failed(m)) => m,
            other => panic!("{other:?}"),
        };
        assert!(text(install_result(Some(127), b"", b"")).contains("Not authorized"));
        assert_eq!(text(install_result(Some(EXIT_CHANGED), b"", b"ignored")), "The update file changed after it was verified");
        assert_eq!(text(install_result(Some(100), b"", b"a\n\nb\nc\nd\n")), "The package manager failed: b c d");
        assert_eq!(text(install_result(None, b"out", b"  \n")), "The package manager failed: out");
    }
}
