//! Epic games managed by the EA app or Ubisoft Connect (`thirdPartyManagedApp`).
//! Port of Heroic's `legendary/thirdParty.ts` and `installEA` /
//! `installUbisoft`.
//!
//! legendary cannot download these games. On Linux Heroic's "install" only
//! fetches the store's installer into `tools/redist/legendary` and records
//! the game in `third-party-installed.json` (`[[appName, platform], ...]`,
//! `JSON.stringify` without spaces). The first launch then runs that
//! installer in the game's prefix (`heronite_launch::first_launch`) and
//! `legendary launch --origin|--ubisoft` opens the game through the store app.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use heronite_data::{json_store, temps, DataPaths};
use heronite_i18n::tr;
use serde_json::{json, Value};

use crate::cmd::Cancel;
use crate::progress::Progress;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Ea,
    Ubisoft,
}

impl Provider {
    /// `isEAManaged` / `isUbisoftManaged` from the `thirdPartyManagedApp` name.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "origin" | "the ea app" => Some(Self::Ea),
            "ubisoftconnect" => Some(Self::Ubisoft),
            _ => None,
        }
    }

    /// The name shown to the user (Heroic says "EA app" for Origin games).
    pub fn label(self) -> &'static str {
        match self {
            Self::Ea => "EA app",
            Self::Ubisoft => "Ubisoft Connect",
        }
    }

    /// File name in `tools/redist/legendary` (the first launch looks for it).
    pub fn installer_file(self) -> &'static str {
        match self {
            Self::Ea => "EAappInstaller.exe",
            Self::Ubisoft => "UbisoftConnectInstaller.exe",
        }
    }

    pub fn installer_url(self) -> &'static str {
        match self {
            Self::Ea => "https://origin-a.akamaihd.net/EA-Desktop-Client-Download/installer-releases/EAappInstaller.exe",
            Self::Ubisoft => "https://static3.cdn.ubi.com/orbit/launcher_installer/UbisoftConnectInstaller.exe",
        }
    }
}

/// The store app of a library entry: `None` for a normal Epic game,
/// `Some(Err(name))` for a store Heroic does not support.
pub fn managed_by(game_info: &Value) -> Option<Result<Provider, String>> {
    let name = game_info.get("thirdPartyManagedApp").and_then(Value::as_str).filter(|s| !s.is_empty())?;
    Some(Provider::from_name(name).ok_or_else(|| name.to_string()))
}

pub fn installed_file(data_paths: &DataPaths) -> PathBuf {
    data_paths.legendary_config_dir().join("third-party-installed.json")
}

pub fn redist_dir(data_paths: &DataPaths) -> PathBuf {
    data_paths.tools_dir().join("redist").join("legendary")
}

/// `[appName, platform]` pairs; a missing or broken file is empty (Heroic
/// only warns).
fn read_pairs(data_paths: &DataPaths) -> Vec<(String, String)> {
    let Ok(Value::Array(items)) = json_store::read_value(&installed_file(data_paths)) else { return Vec::new() };
    items
        .iter()
        .filter_map(|i| Some((i.get(0)?.as_str()?.to_string(), i.get(1).and_then(Value::as_str).unwrap_or_default().to_string())))
        .collect()
}

fn write_pairs(data_paths: &DataPaths, pairs: &[(String, String)]) -> Result<()> {
    let file = installed_file(data_paths);
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let value = Value::Array(pairs.iter().map(|(a, p)| json!([a, p])).collect());
    temps::write_atomic(&file, value.to_string().as_bytes(), None).with_context(|| format!("writing {}", file.display()))
}

pub fn is_installed(data_paths: &DataPaths, app_name: &str) -> bool {
    read_pairs(data_paths).iter().any(|(a, _)| a == app_name)
}

/// `addInstalledGame` (without Heroic's duplicate entry on a second install).
pub fn add_installed(data_paths: &DataPaths, app_name: &str, platform: &str) -> Result<()> {
    let mut pairs = read_pairs(data_paths);
    pairs.retain(|(a, _)| a != app_name);
    pairs.push((app_name.to_string(), platform.to_string()));
    write_pairs(data_paths, &pairs)
}

/// `removeInstalledGame`; `false` when the game was not there (Heroic's
/// `splice(-1, 1)` would drop the last game instead).
pub fn remove_installed(data_paths: &DataPaths, app_name: &str) -> Result<bool> {
    let mut pairs = read_pairs(data_paths);
    let before = pairs.len();
    pairs.retain(|(a, _)| a != app_name);
    if pairs.len() == before {
        return Ok(false);
    }
    write_pairs(data_paths, &pairs)?;
    Ok(true)
}

/// The installer of `provider`, downloaded once (into `<file>.part`, renamed
/// when complete). `on_progress` gets the download progress.
pub fn ensure_installer(
    data_paths: &DataPaths,
    provider: Provider,
    cancel: &Cancel,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<PathBuf> {
    let dir = redist_dir(data_paths);
    let dest = dir.join(provider.installer_file());
    if dest.is_file() {
        return Ok(dest);
    }
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    log::info!("Getting the {} installer", provider.label());
    download_to(provider.installer_url(), &dest, cancel, on_progress)?;
    Ok(dest)
}

const REPORT_EVERY: Duration = Duration::from_millis(500);

/// Downloads `url` into `dest` (through `<dest>.part`, renamed when complete);
/// `on_progress` gets the progress about twice a second.
pub fn download_to(url: &str, dest: &Path, cancel: &Cancel, on_progress: &mut dyn FnMut(Progress)) -> Result<()> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .max_redirects(8)
        .user_agent("heronite-rs")
        .build()
        .into();
    let mut resp = agent.get(url).call().map_err(|e| anyhow!("{url}: {e}"))?;
    let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let mut body = resp.body_mut().with_config().limit(u64::MAX).reader();
    let part = PathBuf::from(format!("{}.part", dest.display()));
    let result = (|| -> Result<()> {
        let mut file = fs::File::create(&part).with_context(|| format!("creating {}", part.display()))?;
        let mut buf = vec![0u8; 256 * 1024];
        let (started, mut last_report, mut got) = (Instant::now(), Instant::now(), 0u64);
        loop {
            if cancel.is_requested() {
                bail!("cancelled");
            }
            let n = body.read(&mut buf).with_context(|| format!("downloading {url}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            got += n as u64;
            if last_report.elapsed() >= REPORT_EVERY {
                last_report = Instant::now();
                on_progress(progress(got, total, started.elapsed()));
            }
        }
        file.flush()?;
        if total > 0 && got < total {
            bail!(tr!("connection closed after {got} of {total} bytes", got, total));
        }
        on_progress(progress(got, total.max(got), started.elapsed()));
        Ok(())
    })();
    match result {
        Ok(()) => fs::rename(&part, dest).with_context(|| format!("renaming {}", part.display())),
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

fn progress(got: u64, total: u64, elapsed: Duration) -> Progress {
    const MIB: f64 = 1024. * 1024.;
    let secs = elapsed.as_secs_f64().max(0.001);
    let speed = got as f64 / secs;
    let eta = if total > got && speed > 0. {
        let left = ((total - got) as f64 / speed) as u64;
        format!("{:02}:{:02}:{:02}", left / 3600, left / 60 % 60, left % 60)
    } else {
        "00:00:00".into()
    };
    Progress {
        percent: if total > 0 { (got as f64 / total as f64 * 100.).min(100.) } else { 0. },
        downloaded_mib: got as f64 / MIB,
        eta,
        down_speed: speed / MIB,
        disk_speed: speed / MIB,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn data_paths() -> (tempfile::TempDir, DataPaths) {
        let dir = tempfile::tempdir().unwrap();
        let data_paths = DataPaths::at(dir.path());
        (dir, data_paths)
    }

    #[test]
    fn providers_follow_heroics_names() {
        assert_eq!(Provider::from_name("Origin"), Some(Provider::Ea));
        assert_eq!(Provider::from_name("The EA App"), Some(Provider::Ea));
        assert_eq!(Provider::from_name("UbisoftConnect"), Some(Provider::Ubisoft));
        assert_eq!(Provider::from_name("Battle.net"), None);
        assert_eq!(managed_by(&json!({"thirdPartyManagedApp": "UbisoftConnect"})), Some(Ok(Provider::Ubisoft)));
        assert_eq!(managed_by(&json!({"thirdPartyManagedApp": "Battle.net"})), Some(Err("Battle.net".into())));
        assert_eq!(managed_by(&json!({"thirdPartyManagedApp": ""})), None);
        assert_eq!(managed_by(&json!({"title": "Control"})), None);
    }

    #[test]
    fn installed_list_is_written_like_json_stringify() {
        let (_d, data_paths) = data_paths();
        assert!(!is_installed(&data_paths, "Anchovy"));
        add_installed(&data_paths, "Anchovy", "Windows").unwrap();
        add_installed(&data_paths, "MtMassive", "Windows").unwrap();
        add_installed(&data_paths, "Anchovy", "Windows").unwrap();
        assert_eq!(fs::read_to_string(installed_file(&data_paths)).unwrap(), r#"[["MtMassive","Windows"],["Anchovy","Windows"]]"#);
        assert!(is_installed(&data_paths, "Anchovy"));
        // The library sees them as installed.
        let games = crate::library::installed_games(&data_paths);
        assert_eq!(games["Anchovy"], json!({"app_name": "Anchovy", "platform": "Windows"}));

        assert!(!remove_installed(&data_paths, "Other").unwrap());
        assert!(remove_installed(&data_paths, "MtMassive").unwrap());
        assert_eq!(fs::read_to_string(installed_file(&data_paths)).unwrap(), r#"[["Anchovy","Windows"]]"#);
    }

    #[test]
    fn a_present_installer_is_not_downloaded_again() {
        let (_d, data_paths) = data_paths();
        fs::create_dir_all(redist_dir(&data_paths)).unwrap();
        fs::write(redist_dir(&data_paths).join("EAappInstaller.exe"), "MZ").unwrap();
        let path = ensure_installer(&data_paths, Provider::Ea, &Cancel::new(), &mut |_| panic!("no download")).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"MZ");
    }

    /// One HTTP response of `size` bytes from a local server.
    fn serve(size: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Some(Ok(mut s)) = listener.incoming().next() {
                let mut req = [0u8; 1024];
                let _ = s.read(&mut req);
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n");
                let _ = s.write_all(&vec![b'x'; size]);
            }
        });
        format!("http://{addr}/installer.exe")
    }

    #[test]
    fn downloads_report_progress_and_leave_no_part_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("UbisoftConnectInstaller.exe");
        let mut seen = Vec::new();
        download_to(&serve(1_000_000), &dest, &Cancel::new(), &mut |p| seen.push(p)).unwrap();
        assert_eq!(fs::metadata(&dest).unwrap().len(), 1_000_000);
        assert_eq!(seen.last().unwrap().percent, 100.);
        assert!(!dir.path().join("UbisoftConnectInstaller.exe.part").exists());
    }

    #[test]
    fn a_cancelled_download_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("EAappInstaller.exe");
        let cancel = Cancel::new();
        cancel.request();
        let err = download_to(&serve(10_000), &dest, &cancel, &mut |_| {}).unwrap_err();
        assert!(format!("{err:#}").contains("cancelled"));
        assert!(!dest.exists());
        assert!(!dir.path().join("EAappInstaller.exe.part").exists());
    }

    #[test]
    fn eta_and_speed() {
        let p = progress(50 * 1024 * 1024, 100 * 1024 * 1024, Duration::from_secs(10));
        assert_eq!(p.percent, 50.);
        assert_eq!(p.eta, "00:00:10");
        assert!((p.down_speed - 5.).abs() < 1e-9);
    }
}
