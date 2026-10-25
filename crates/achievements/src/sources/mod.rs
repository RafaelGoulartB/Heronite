//! One module per source. Each turns what its store (or this PC) says into
//! an [`AchievementSet`]. Blocking (files, network): call from a worker
//! thread.

pub mod epic;
pub mod gog;
pub mod retro;
pub mod steam;

use heronite_data::paths::DataPaths;
use heronite_metadata::Http;

use crate::accounts::Accounts;
use crate::model::AchievementSet;
use crate::target::SourceTarget;

/// What every source may need.
pub struct Ctx<'a> {
    pub paths: &'a DataPaths,
    pub http: &'a Http,
    pub accounts: &'a Accounts,
    /// The interface language (`pt-BR`, `es`, `en`), for the sources that
    /// translate names and descriptions.
    pub lang: &'a str,
}

/// The answer of a source about one game.
#[derive(Debug, Clone, PartialEq)]
pub enum Fetch {
    Set(AchievementSet),
    /// The game has no achievements there.
    None,
    /// The source needs an account the user has not given (a store sign-in,
    /// the RetroAchievements account).
    NeedsAccount,
    /// A ROM no RetroAchievements set knows; `hash` is the one computed.
    Unmatched {
        hash: Option<String>,
        file: String,
    },
    /// It could not be read now (offline, an expired sign-in, a server
    /// error); the local copy stays as it is.
    Failed(String),
}

pub fn fetch(ctx: &Ctx, target: &SourceTarget) -> Fetch {
    match target {
        SourceTarget::Steam { app_id } => steam::fetch(ctx, app_id),
        SourceTarget::Gog { product_id } => gog::fetch(ctx, product_id),
        SourceTarget::Epic { app_name } => epic::fetch(ctx, app_name),
        SourceTarget::Retro(t) => retro::fetch(ctx, t),
    }
}
