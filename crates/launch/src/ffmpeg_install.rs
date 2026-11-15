//! Downloads ffmpeg (and ffprobe, for whoever asks for it) into
//! `<data>/tools/bin` when a trailer is played and neither the `PATH` nor
//! that folder has it (`heronite_data::ffmpeg` then finds it there), so
//! trailers play on a PC where nobody installed ffmpeg.
//!
//! Windows only: Linux systems have ffmpeg in their packages (with the
//! PulseAudio output the trailers' sound needs), and the static Linux
//! builds cannot resolve host names (static glibc), so they could not
//! stream a trailer anyway.
//!
//! The builds are the static ones the `ffmpeg-static` npm package
//! publishes (gyan.dev's "essentials" build on Windows: HLS, HTTPS through
//! GnuTLS), one gzipped program each, about 30 MB. A release is pinned with
//! the SHA-256 of each file (the ones GitHub shows for its assets): a file
//! that does not match is never installed. Like the store tools
//! ([`crate::store_tools`]), a download waits in `<name>.part.exe`, must run
//! (`-version`, without a console window) and only then takes its place.
//! A download that takes longer than [`DOWNLOAD_LIMIT`] (a connection that
//! stalled) gives up, so a play never waits on it forever.

use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use heronite_data::ffmpeg::{self, Tool};
use heronite_data::DataPaths;
use sha2::{Digest, Sha256};

use crate::legendary_install::part_file;
use crate::net::{Cancel, Net, CANCELLED};

/// The `ffmpeg-static` release (ffmpeg 6.1.1).
pub const VERSION: &str = "b6.1.1";
const REPO: &str = "eugeneware/ffmpeg-static";
/// What the download of each program costs, for the UI.
pub const DOWNLOAD_MB_EACH: u32 = 30;
/// Longest a program's download may take (30 MB in 20 minutes: 0.2 Mbit/s).
pub const DOWNLOAD_LIMIT: Duration = Duration::from_secs(20 * 60);

/// One gzipped program of the release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    pub tool: Tool,
    pub file: &'static str,
    /// SHA-256 of the gzipped file, lower-case hex.
    pub sha256: &'static str,
}

const WINDOWS_X64: [Asset; 2] = [
    Asset { tool: Tool::Ffmpeg, file: "ffmpeg-win32-x64.gz", sha256: "8883a3dffbd0a16cf4ef95206ea05283f78908dbfb118f73c83f4951dcc06d77" },
    Asset { tool: Tool::Ffprobe, file: "ffprobe-win32-x64.gz", sha256: "f309e6223ad89d2fe54bccd420a7709b66fd27540674e92309578ed491a43c8d" },
];

/// The builds for a system and CPU (`std::env::consts` names); `None`
/// where nothing is downloaded.
pub fn assets_for(os: &str, arch: &str) -> Option<&'static [Asset; 2]> {
    match (os, arch) {
        // No Arm build: Windows on Arm runs x64 programs.
        ("windows", "x86_64" | "aarch64") => Some(&WINDOWS_X64),
        _ => None,
    }
}

/// Whether ffmpeg is downloaded here when it is missing.
pub fn can_download() -> bool {
    assets_for(std::env::consts::OS, std::env::consts::ARCH).is_some()
}

pub fn download_url(file: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/{VERSION}/{file}")
}

/// Makes sure `tools` are there: each one neither on the `PATH` nor in
/// `<data>/tools/bin` is downloaded there, and [`ffmpeg::find`] looks again.
/// Blocking, network; one download at a time per process (a second call
/// waits, then finds them).
pub fn ensure(paths: &DataPaths, tools: &[Tool], net: &dyn Net, cancel: Cancel) -> Result<(), String> {
    static BUSY: Mutex<()> = Mutex::new(());
    let _guard = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    let path_var = std::env::var_os("PATH");
    let dir = ffmpeg::data_bin_dir(paths);
    let missing: Vec<Tool> = tools.iter().copied().filter(|t| ffmpeg::find_in(*t, path_var.clone(), &dir).is_none()).collect();
    if missing.is_empty() {
        // Put there some other way meanwhile (copied in, restored): found from now on.
        ffmpeg::forget();
        return Ok(());
    }
    let assets = assets_for(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        format!("ffmpeg is not downloaded on {} {}: install it from your system's packages", std::env::consts::OS, std::env::consts::ARCH)
    })?;
    if !net.is_online() {
        return Err("ffmpeg is not installed and there is no internet connection".into());
    }
    for asset in assets.iter().filter(|a| missing.contains(&a.tool)) {
        let target = ffmpeg::data_bin(paths, asset.tool);
        let installed = install(asset, &download_url(asset.file), &target, net, cancel);
        // What was installed before a failure is found from now on too.
        ffmpeg::forget();
        installed?;
        log::info!("{} ({} {VERSION}) downloaded to {}", asset.tool.name(), REPO, target.display());
    }
    Ok(())
}

/// Downloads `asset` from `url`, checks its SHA-256, unpacks it next to
/// `target`, checks that it runs, then moves it into place: a broken or
/// interrupted download never becomes the program.
pub fn install(asset: &Asset, url: &str, target: &Path, net: &dyn Net, cancel: Cancel) -> Result<(), String> {
    let name = asset.tool.name();
    let dir = target.parent().ok_or_else(|| format!("{name} has no folder"))?;
    fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let gz = dir.join(format!("{name}.part.gz"));
    let part = part_file(target);
    let clean = || {
        let _ = fs::remove_file(&gz);
        let _ = fs::remove_file(&part);
    };
    clean();
    let fail = |msg: String| {
        clean();
        msg
    };
    net.download_within(url, &gz, cancel, DOWNLOAD_LIMIT)
        .map_err(|e| fail(if e == CANCELLED { e } else { format!("downloading {name} failed: {e}") }))?;
    let sum = sha256_of(&gz).map_err(|e| fail(format!("reading the downloaded {name}: {e}")))?;
    if sum != asset.sha256 {
        return Err(fail(format!("the downloaded {name} is not the expected file (SHA-256 {sum})")));
    }
    if cancel() {
        return Err(fail(CANCELLED.into()));
    }
    gunzip(&gz, &part).map_err(|e| fail(format!("unpacking {name}: {e}")))?;
    let _ = fs::remove_file(&gz);
    heronite_platform::fs::make_executable(&part).map_err(|e| fail(format!("making {name} executable: {e}")))?;
    if !runs(&part) {
        return Err(fail(format!("the downloaded {name} does not run")));
    }
    heronite_platform::fs::rename_replacing(&part, target).map_err(|e| fail(format!("installing {name}: {e}")))
}

fn sha256_of(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn gunzip(from: &Path, to: &Path) -> io::Result<()> {
    let mut input = flate2::read::GzDecoder::new(io::BufReader::new(fs::File::open(from)?));
    let mut output = io::BufWriter::new(fs::File::create(to)?);
    io::copy(&mut input, &mut output)?;
    io::Write::flush(&mut output)?;
    Ok(())
}

/// Whether `bin -version` exits well. It runs as a helper: no console
/// window on Windows (`heronite_data::proc`).
fn runs(bin: &Path) -> bool {
    let mut cmd = Command::new(bin);
    cmd.arg("-version");
    let run = heronite_data::proc::Run::new().timeout(Duration::from_secs(60)).grace(Duration::from_secs(2));
    run.run(&mut cmd).is_ok_and(|out| out.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeNet;
    use std::io::Write;

    use crate::store_tools::tests::fake_bin;

    fn gz(bytes: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    fn sha(bytes: &[u8]) -> &'static str {
        let hex: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
        Box::leak(hex.into_boxed_str())
    }

    const URL: &str = "https://example.invalid/ffmpeg.gz";

    #[test]
    fn a_good_download_is_unpacked_into_place() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let body = gz(&fake_bin(true));
        let asset = Asset { tool: Tool::Ffmpeg, file: "ffmpeg.gz", sha256: sha(&body) };
        let target = ffmpeg::data_bin(&paths, Tool::Ffmpeg);
        let net = FakeNet::online().with_file(URL, body);
        install(&asset, URL, &target, &net, &|| false).unwrap();
        assert_eq!(fs::read(&target).unwrap(), fake_bin(true));
        let left: Vec<_> = fs::read_dir(target.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, [target.file_name().unwrap().to_owned()], "nothing else is left behind");
    }

    #[test]
    fn a_file_with_another_checksum_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let asset = Asset { tool: Tool::Ffprobe, file: "ffprobe.gz", sha256: sha(b"something else") };
        let target = ffmpeg::data_bin(&paths, Tool::Ffprobe);
        let net = FakeNet::online().with_file(URL, gz(&fake_bin(true)));
        let err = install(&asset, URL, &target, &net, &|| false).unwrap_err();
        assert!(err.contains("not the expected file"), "{err}");
        assert!(!target.exists());
        assert_eq!(fs::read_dir(target.parent().unwrap()).unwrap().count(), 0);
    }

    #[test]
    fn a_program_that_does_not_run_is_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let body = gz(&fake_bin(false));
        let asset = Asset { tool: Tool::Ffmpeg, file: "ffmpeg.gz", sha256: sha(&body) };
        let target = ffmpeg::data_bin(&paths, Tool::Ffmpeg);
        let err = install(&asset, URL, &target, &FakeNet::online().with_file(URL, body), &|| false).unwrap_err();
        assert!(err.contains("does not run"), "{err}");
        assert!(!target.exists());
        assert!(!part_file(&target).exists());
    }

    #[test]
    fn a_cancelled_download_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let body = gz(&fake_bin(true));
        let asset = Asset { tool: Tool::Ffmpeg, file: "ffmpeg.gz", sha256: sha(&body) };
        let target = ffmpeg::data_bin(&paths, Tool::Ffmpeg);
        let err = install(&asset, URL, &target, &FakeNet::online().with_file(URL, body), &|| true).unwrap_err();
        assert_eq!(err, CANCELLED);
        assert!(!target.exists());
    }

    #[test]
    fn programs_already_there_are_not_downloaded_again() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        for tool in Tool::ALL {
            let p = ffmpeg::data_bin(&paths, tool);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b"").unwrap();
        }
        let net = FakeNet::offline();
        ensure(&paths, &Tool::ALL, &net, &|| false).unwrap();
        assert!(net.requests().is_empty());
    }

    #[test]
    fn only_the_programs_asked_for_are_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        let Some(assets) = assets_for(std::env::consts::OS, std::env::consts::ARCH) else {
            let err = ensure(&paths, &[Tool::Ffmpeg], &FakeNet::online(), &|| false).unwrap_err();
            assert!(err.contains("not downloaded on"), "{err}");
            return;
        };
        // Not the release's bytes: refused, after asking for ffmpeg only.
        let url = download_url(assets[0].file);
        let net = FakeNet::online().with_file(&url, gz(&fake_bin(true)));
        let err = ensure(&paths, &[Tool::Ffmpeg], &net, &|| false).unwrap_err();
        assert!(err.contains("not the expected file"), "{err}");
        assert_eq!(net.requests(), [url]);
        assert!(!ffmpeg::data_bin(&paths, Tool::Ffmpeg).exists());
    }

    #[test]
    fn only_windows_downloads_it() {
        assert_eq!(assets_for("windows", "x86_64"), Some(&WINDOWS_X64));
        assert_eq!(assets_for("windows", "aarch64"), Some(&WINDOWS_X64), "x64 under emulation");
        assert_eq!(assets_for("linux", "x86_64"), None);
        assert_eq!(
            download_url(WINDOWS_X64[0].file),
            "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1/ffmpeg-win32-x64.gz"
        );
        for a in WINDOWS_X64 {
            assert_eq!(a.sha256.len(), 64);
            assert!(a.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        }
    }

    /// The real release, downloaded, checked and run on this system:
    /// `cargo test -p heronite-launch --lib ffmpeg_install -- --ignored`.
    #[test]
    #[ignore = "downloads about 60 MB"]
    fn the_real_release_installs_and_runs_here() {
        let Some(assets) = assets_for(std::env::consts::OS, std::env::consts::ARCH) else { return };
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("heronite"));
        for asset in assets {
            let target = ffmpeg::data_bin(&paths, asset.tool);
            install(asset, &download_url(asset.file), &target, &crate::net::SystemNet, &|| false).unwrap();
            assert!(runs(&target));
        }
    }
}
