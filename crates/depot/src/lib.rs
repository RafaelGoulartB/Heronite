//! Downloads games into their install folder, whatever the store.
//!
//! A store's manifest says which files a build has and which pieces of data
//! (chunks) each file is made of. The store's crate turns it into a [`Plan`]:
//! the files to write, each a list of [`Part`]s, either bytes of a [`Chunk`]
//! the store serves or bytes of a file already installed (the old version,
//! during an update, so unchanged data is not downloaded again), plus the
//! files to remove and the folders to create. A [`Source`] fetches and
//! decodes chunks (the CDN, the HTTP ranges of an installer); this crate does
//! no network itself.
//!
//! * [`install`] writes a plan: chunks fetched by worker threads ahead of the
//!   writer, each held in memory until its last use within a budget, every
//!   file written beside its place and renamed when complete and checked,
//!   a resume journal so a cancelled run goes on where it stopped, and
//!   [`Progress`] a few times a second.
//! * [`verify`] checks installed files against their [`Hash`]es (repair).
//! * [`space_needed`] says how much free space a plan takes.
//!
//! Everything is blocking: call it from worker threads.

mod fetch;
mod hash;
mod install;
mod journal;
mod plan;
mod progress;
mod verify;

pub use hash::{Hash, Hasher};
pub use install::{install, space_needed, Options, Outcome, Source};
pub use plan::{check_path, folder_name, Chunk, FileTask, Part, Plan};
pub use progress::Progress;
pub use verify::{verify, Expected, Problem};

/// Threads for `asked` (`0`: twice the processors, 2 to 16).
pub(crate) fn workers(asked: usize) -> usize {
    if asked > 0 {
        return asked;
    }
    let cpus = std::thread::available_parallelism().map_or(4, |n| n.get());
    (cpus * 2).clamp(2, 16)
}

#[cfg(test)]
mod tests;
