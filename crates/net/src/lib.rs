//! One streaming download for every crate that fetches big files.
//!
//! A plain `read()` on a stalled TCP connection blocks forever, so a loop
//! that checks the cancel flag between reads never sees it. `download` reads
//! on a helper thread and waits for the data with a short tick: a cancel
//! returns within a tick and a connection that sends nothing for
//! `Options::stall_timeout` fails with `Error::Stalled`. The file is written
//! to `<dest>.part` and renamed onto `dest` only when it is complete and
//! checked, so a failed download never leaves a file that looks finished.

mod download;
mod error;

pub use download::{download, Hash, Options, Outcome};
pub use error::{redact_url, Error};
