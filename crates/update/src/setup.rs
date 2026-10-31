//! A copy installed on Windows by Heronite's setup (Inno Setup, per user,
//! `%LOCALAPPDATA%\Programs\Heronite` by default). The setup writes
//! [`Installer::MARKER`] next to the program, holding `windows-installer`.
//!
//! Its files belong to the setup (the uninstaller lists them), so an update
//! runs the release's setup instead of replacing them: the app downloads
//! `Heronite-Setup-<arch>.exe` and checks its signature like any other
//! download ([`crate::install::download_signed`]), then starts it as it
//! quits ([`Installer::run_setup`]). The setup waits for this process to
//! exit (`/WAITPID`), replaces the files silently and, after a restart,
//! starts the new version (`/RELAUNCH=1`).

use heronite_i18n::tr;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::install::{download_signed, InstallError, Progress};
use crate::release::{Release, WindowsAsset};

/// This copy was installed by the Windows setup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installer {
    /// This program, read at start.
    pub exe: PathBuf,
}

impl Installer {
    /// The marker the setup writes next to the program.
    pub const MARKER: &'static str = "package-format";
    /// Its content.
    pub const FORMAT: &'static str = "windows-installer";

    /// The installed copy `exe` belongs to, if the setup installed it.
    pub fn detect(exe: &Path) -> Option<Self> {
        let marker = exe.parent()?.join(Self::MARKER);
        let text = std::fs::read_to_string(marker).ok()?;
        (text.trim() == Self::FORMAT).then(|| Self { exe: exe.to_path_buf() })
    }

    /// Downloads the setup of `release` into `dir` and checks it against
    /// `public_key` ([`crate::PUBLIC_KEY`]). Returns the file, ready for
    /// [`Installer::run_setup`].
    pub fn download(
        release: &Release,
        dir: &Path,
        public_key: &str,
        cancel: &AtomicBool,
        on_progress: &mut dyn FnMut(Progress),
    ) -> Result<PathBuf, InstallError> {
        let kind = WindowsAsset::Setup;
        let asset = release.windows_asset(kind).ok_or_else(|| InstallError::Failed(tr!("This release has no Windows setup").into()))?;
        let signature = release.windows_signature(kind).ok_or_else(|| InstallError::Failed(tr!("This release is not signed").into()))?;
        download_signed(asset, signature, &release.version, dir, public_key, cancel, on_progress)
    }

    /// The setup's switches for an unattended update: no window nor
    /// question, no reboot, no Cancel; wait for `pid` (this app) to exit;
    /// start the new version after it when `relaunch`; write its log to
    /// `log`.
    pub fn setup_args(pid: u32, relaunch: bool, log: Option<&Path>) -> Vec<OsString> {
        let mut args: Vec<OsString> =
            ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/NOCANCEL"].into_iter().map(OsString::from).collect();
        args.push(format!("/WAITPID={pid}").into());
        if relaunch {
            args.push("/RELAUNCH=1".into());
        }
        if let Some(log) = log {
            let mut arg = OsString::from("/LOG=");
            arg.push(log.as_os_str());
            args.push(arg);
        }
        args
    }

    /// Starts the downloaded `setup`, detached and out of this app's job, to
    /// install the update once this process has exited. Returns its pid.
    pub fn run_setup(setup: &Path, relaunch: bool, log: Option<&Path>) -> io::Result<u32> {
        crate::relaunch::launch(setup, &Self::setup_args(std::process::id(), relaunch, log))
    }
}

#[cfg(test)]
mod tests {
    use super::Installer;
    use crate::install::tests::{blob, serve, SIG_014, TEST_KEY};
    use crate::release::{Asset, Release, WindowsAsset};
    use std::path::Path;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    fn release(base: &str, assets: &[&str]) -> Release {
        Release {
            version: "0.1.4".into(),
            name: String::new(),
            notes: String::new(),
            page_url: String::new(),
            published_at: String::new(),
            appimage: None,
            signature: None,
            packages: Vec::new(),
            windows: assets.iter().map(|n| Asset { name: n.to_string(), url: format!("{base}/{n}"), size: 0 }).collect(),
        }
    }

    /// The setup is checked like the AppImage (the test blob is signed for
    /// 0.1.4 with the first test key).
    #[test]
    fn downloads_the_signed_setup_and_refuses_a_bad_or_missing_one() {
        let tmp = tempfile::tempdir().unwrap();
        let setup = WindowsAsset::Setup.file_name();
        let sig = format!("{setup}.minisig");
        let data = blob();
        let mut tampered = data.clone();
        tampered[100] ^= 1;
        let base = serve(
            vec![
                (format!("/{setup}"), data.clone()),
                (format!("/{sig}"), SIG_014.into()),
                (format!("/bad/{setup}"), tampered),
                (format!("/bad/{sig}"), SIG_014.into()),
            ],
            0,
        );
        let no = AtomicBool::new(false);
        let dir = tmp.path().join("updates");
        let file = Installer::download(&release(&base, &[&setup, &sig]), &dir, TEST_KEY, &no, &mut |_| {}).unwrap();
        assert_eq!(file, dir.join(&setup));
        assert_eq!(std::fs::read(&file).unwrap(), data);
        let bad = format!("{base}/bad");
        let err = Installer::download(&release(&bad, &[&setup, &sig]), &tmp.path().join("b"), TEST_KEY, &no, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("does not match its signature"), "{err}");
        assert!(!tmp.path().join("b").join(&setup).exists());
        let err = Installer::download(&release(&base, &[&setup]), &dir, TEST_KEY, &no, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("not signed"), "{err}");
        let err = Installer::download(&release(&base, &[]), &dir, TEST_KEY, &no, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("no Windows setup"), "{err}");
    }

    #[test]
    fn detects_the_marker_next_to_the_program() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("heronite-rs.exe");
        assert_eq!(Installer::detect(&exe), None);
        std::fs::write(tmp.path().join(Installer::MARKER), "deb\n").unwrap();
        assert_eq!(Installer::detect(&exe), None);
        std::fs::write(tmp.path().join(Installer::MARKER), "windows-installer\r\n").unwrap();
        assert_eq!(Installer::detect(&exe), Some(Installer { exe: exe.clone() }));
    }

    #[test]
    fn the_setup_command_waits_for_the_app_and_relaunches_on_request() {
        let args = |relaunch, log: Option<&Path>| -> Vec<String> {
            Installer::setup_args(4242, relaunch, log).into_iter().map(|a| a.into_string().unwrap()).collect()
        };
        assert_eq!(args(true, None), ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/NOCANCEL", "/WAITPID=4242", "/RELAUNCH=1"]);
        let quiet = args(false, Some(Path::new(r"C:\Users\A B\setup.log")));
        assert_eq!(quiet[4..], ["/WAITPID=4242", r"/LOG=C:\Users\A B\setup.log"]);
    }

    /// A stand-in setup (a batch file) started detached writes the command
    /// line it got.
    #[test]
    fn runs_the_setup_detached_with_its_switches() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("args.txt");
        let stub = tmp.path().join("setup stub.cmd");
        std::fs::write(&stub, format!("@echo %*> \"{}.tmp\"\r\n@move /y \"{0}.tmp\" \"{0}\" >nul\r\n", out.display())).unwrap();
        let log = tmp.path().join("setup log.txt");
        let pid = Installer::run_setup(&stub, true, Some(&log)).unwrap();
        assert!(pid > 0);
        for _ in 0..500 {
            if out.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // Without the quotes: a batch file gets `=` quoted too, a program
        // only the argument with a space.
        let line = std::fs::read_to_string(&out).unwrap().replace('"', "");
        let expected = format!(
            "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /NOCANCEL /WAITPID={} /RELAUNCH=1 /LOG={}",
            std::process::id(),
            log.display()
        );
        assert_eq!(line.trim(), expected);
    }
}
