//! GOG's own protocol, the work gogdl did for the app: signing in, the
//! builds and depot manifests of the content system (v1 and v2), secure
//! links and chunks. Everything is blocking: call it from worker threads.
//!
//! It keeps gogdl's files, so installs made by gogdl keep updating and the
//! rest of the app reads them as before:
//!
//! * [`auth`]: `gog_store/auth.json`, the tokens keyed by client id.
//! * [`stored`]: `gogdlConfig/heroic_gogdl/manifests/<id>` (the build of an
//!   installed game) and the support folder next to it.
//!
//! The rest reads GOG's answers into plain models that know nothing of how
//! the bytes are written: [`api`] (HTTP), [`builds`], [`v2`], [`v1`],
//! [`files`] (the files of a build and the difference between two) and
//! [`chunk`] (secure links, chunk addresses and decoding). [`language`] is
//! GOG's table of language codes.

pub mod api;
pub mod auth;
pub mod builds;
pub mod chunk;
pub mod files;
pub mod language;
pub mod stored;
pub mod v1;
pub mod v2;

pub use api::Client;
pub use auth::{Credentials, SignInExpired};
pub use files::{ChunkRef, Diff, FileEntry, Link, Root, Source};

/// Client id of the GOG Galaxy app: the account's tokens are kept under it.
pub const CLIENT_ID: &str = "46899977096215655";
/// Its secret (public: every Galaxy install has it).
pub const CLIENT_SECRET: &str = "9d85c43b1482497dbbce61f6e4aa173a433796eeae2ca8c5f6129f2dc4de46d9";
