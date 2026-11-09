//! Heronite's games in Sunshine's application list, so Moonlight can start
//! them from a phone, a TV or another computer. Apollo, a Sunshine fork,
//! keeps the same web API.
//!
//! * [`api`]: Sunshine's web API on this computer (`https://localhost:47990`,
//!   basic auth, a self-signed certificate): list, save and delete
//!   applications.
//! * [`entry`]: what one of Heronite's applications holds, and the marker
//!   key (`heronite-id`) that tells them from the user's own. Sunshine keeps
//!   unknown keys of an application, so the marker survives its web UI.
//! * [`sync`]: brings the list in line with the games Heronite wants there,
//!   touching only marked entries.
//! * [`prefs`]: Settings → Integrations → Sunshine (`sunshine.json` in the
//!   app state), with the password sealed for this Windows user (DPAPI).
//! * [`install`]: whether Sunshine (or Apollo) looks installed, for the
//!   message when nothing answers.
//!
//! Every function is blocking: call them off the UI thread.
//!
//! The feature follows the idea of Irori's Sunshine integration
//! (github.com/irori-app/irori, MIT); the code is Heronite's own.

pub mod api;
pub mod entry;
pub mod install;
pub mod prefs;
pub mod sync;

pub use api::{Client, Error};
pub use entry::Entry;
pub use sync::{sync, Report};

#[cfg(test)]
mod tests;
