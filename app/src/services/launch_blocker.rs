//! Why a game cannot be launched right now. The Play button and the launches
//! that have no window (tray, `heronite://`, Steam shortcuts) share it.

use gpui_kit::*;
use heronite_data::CollectionGame;
use heronite_i18n::tr;

/// What stands between a store game and its launch.
#[derive(Debug, Clone, PartialEq)]
pub enum Blocker {
    /// In the download queue.
    Queued,
    /// An install, update, verification or move is running; its label.
    Busy(String),
    /// The install folder is gone.
    FolderMissing,
    /// An update is waiting (the game can still be played).
    Update,
}

impl Blocker {
    /// A download or an operation owns the game's files.
    pub fn is_busy(&self) -> bool {
        matches!(self, Blocker::Queued | Blocker::Busy(_))
    }

    /// What to tell the user when nobody can be asked (no window): `None` for
    /// an update, which does not stop a launch there.
    pub fn refusal(&self, title: &str) -> Option<String> {
        match self {
            Blocker::Queued => Some(tr!("{title} is waiting in the download queue", title)),
            Blocker::Busy(task) => Some(tr!("{title} is busy ({task})", title, task)),
            Blocker::FolderMissing => Some(tr!("The install folder of {title} is missing", title)),
            Blocker::Update => None,
        }
    }
}

/// The first thing that stops `game` from launching, in the order the Play
/// button deals with them. Reads the Epic service, which is not there in
/// one-shot mode (then nothing is downloading).
pub fn launch_blocker(cx: &App, game: &CollectionGame) -> Option<Blocker> {
    // Only the stores' games go through the download queue.
    if !matches!(game.runner.as_str(), "legendary" | "gog" | "nile") {
        return None;
    }
    let downloads = super::try_downloads(cx);
    if let Some(d) = downloads.as_ref().map(|d| d.read(cx)) {
        if d.queued(&game.app_name).is_some() {
            return Some(Blocker::Queued);
        }
        if let Some(busy) = d.busy_of(&game.app_name) {
            return Some(Blocker::Busy(busy.label.to_string()));
        }
    }
    if !game.installed {
        return None;
    }
    if game.install_folder_missing() {
        return Some(Blocker::FolderMissing);
    }
    downloads.is_some_and(|d| d.read(cx).has_update(&game.app_name)).then_some(Blocker::Update)
}

#[cfg(test)]
mod tests {
    use super::Blocker;

    #[test]
    fn downloads_block_a_launch_and_an_update_does_not() {
        assert!(Blocker::Queued.is_busy());
        assert!(Blocker::Busy("Verifying".into()).is_busy());
        assert!(!Blocker::FolderMissing.is_busy());
        assert!(!Blocker::Update.is_busy());
        assert_eq!(Blocker::Queued.refusal("Calluna").unwrap(), "Calluna is waiting in the download queue");
        assert_eq!(Blocker::Busy("Verifying".into()).refusal("Calluna").unwrap(), "Calluna is busy (Verifying)");
        assert_eq!(Blocker::FolderMissing.refusal("Calluna").unwrap(), "The install folder of Calluna is missing");
        assert_eq!(Blocker::Update.refusal("Calluna"), None);
    }
}
