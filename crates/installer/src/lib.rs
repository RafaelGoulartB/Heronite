//! Installs games from YAML (or JSON) install scripts: a list of files to
//! fetch, the steps that set the game up (run a setup program in a Wine
//! prefix, extract archives, write config files, install Winetricks verbs...)
//! and how the installed game starts. The result is an ordinary local game
//! (`sideload`) of the Collection, with its Wine settings, environment and
//! extra launch entries (the game's own actions).
//!
//! * [`script`] reads a script file into [`Script`]s (a file may hold several).
//! * [`check`] says before anything runs what the script needs and what this
//!   app cannot do (an unknown step, a missing tool, an unsupported runner).
//! * [`engine::install`] runs it on the calling thread, reporting through a
//!   [`Host`] (progress, questions to the user, cancellation).
//!
//! Every function here blocks; the app calls them from a worker thread.

pub mod archive;
pub mod check;
pub mod engine;
pub mod exec;
pub mod files;
pub mod finish;
pub mod fsops;
pub mod runners;
pub mod script;
pub mod steps;
pub mod tasks;
pub mod text;
pub mod vars;

pub use check::{check, Check, Issue};
pub use engine::{install, stages, Answer, Event, Failure, Host, Job, Outcome, Question, Resume};
pub use script::{parse, read_file, Script};
