//! Self-update of the AppImage and of the Windows copies.
//!
//! The release workflow publishes, for every tag, `Heronite-<arch>.AppImage`
//! (and on Windows `Heronite-Setup-<arch>.exe` and
//! `Heronite-Portable-<arch>.zip`) and `<file>.minisig`, a minisign signature made with the key whose public
//! half is [`PUBLIC_KEY`]. Its trusted comment carries `version:<X.Y.Z>`, so
//! an older signed build cannot be served as a newer release.
//!
//! - [`release`]: asks GitHub for the latest release (conditional request with
//!   the last ETag) and picks the assets for this architecture.
//! - [`install`]: finds the running AppImage (`$APPIMAGE`), downloads the new
//!   one next to it while checking the signature, and renames it over the old
//!   file. The running app keeps reading the old inode, so it is never
//!   disturbed; the new version starts with the next launch.
//! - [`relaunch`]: starts the new AppImage, detached, as the app quits.
//! - [`package`]: .deb / .rpm / Arch installs, which are only told about new
//!   versions (their package manager owns the files).
//! - [`setup`] (Windows): a copy installed by the setup downloads the new
//!   setup and runs it, silently, as the app quits.
//! - [`portable`] (Windows): a portable copy downloads the new zip and swaps
//!   its program files in, keeping the replaced ones as `.old` until the
//!   next start.
//! - [`prefs`]: the user's choices and the last check (`update.json`).
//!
//! Every function blocks; the app calls them off the UI thread.

pub mod install;
pub mod package;
#[cfg(windows)]
pub mod portable;
pub mod prefs;
pub mod relaunch;
pub mod release;
#[cfg(windows)]
pub mod setup;
pub mod version;

pub use install::{InstallError, Progress, Target, Unsupported};
pub use package::Package;
#[cfg(windows)]
pub use portable::Portable;
pub use prefs::Prefs;
pub use release::{Asset, Fetched, Release, WindowsAsset};
#[cfg(windows)]
pub use setup::Installer;
pub use version::Version;

/// GitHub API endpoint of the latest (non-draft, non-prerelease) release.
pub const LATEST_API: &str = "https://api.github.com/repos/RafaelGoulartB/Heronite/releases/latest";

/// The project on GitHub (a star, issues, Discussions).
pub const REPO_PAGE: &str = "https://github.com/RafaelGoulartB/Heronite";

/// The releases page, for people who update by hand.
pub const RELEASES_PAGE: &str = "https://github.com/RafaelGoulartB/Heronite/releases/latest";

/// Public half of the release signing key (minisign, ed25519). The secret
/// half lives only in the repository secret `MINISIGN_SECRET_KEY`.
pub const PUBLIC_KEY: &str = "RWSE2jZR7ZDsOydCTZOZivDlm7+0a3fALyFTGewUjbVbWGb6V4A2CALD";

fn agent() -> &'static ureq::Agent {
    use std::sync::OnceLock;
    use std::time::Duration;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .max_redirects(8)
            .user_agent(concat!("heronite-rs/", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    })
}

/// A readable error for a failed request (GitHub's rate limit named as such).
fn http_error(e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::StatusCode(c @ (403 | 429)) => {
            anyhow::anyhow!(heronite_i18n::tr!("GitHub rate limit reached (HTTP {status}), try again later", status = c))
        }
        ureq::Error::StatusCode(c) => anyhow::anyhow!("HTTP {c}"),
        e => anyhow::anyhow!("{e}"),
    }
}
