//! Nexus Mods: the account, `nxm://` links, the REST v1 API (needs the user's
//! personal API key) and the GraphQL v2 search (needs no key).

mod account;
mod api;
mod files;
mod nxm;
mod search;
mod update;

use crate::http::null_default;

pub use account::{forget_account, load_account, save_account, validate, Account, StoredAccount};
pub use api::{download_link, file_info, files, mod_info, rate_limits, NexusMod};
pub use files::{FileUpdate, NexusFile, NexusFiles};
pub use nxm::{is_nxm, parse_nxm, NxmError, NxmLink};
pub use search::{game_name, games_matching, search, SearchMod, SearchPage, SearchSort};
pub use update::{check_update, main_file};
