//! Launching games and tracking playtime, compatible with Heronite's data.
//!
//! * [`plan`]: pure `LaunchPlan` building (Steam URI, native, Wine/Proton/umu,
//!   emulators, legendary).
//! * [`runtime`]: the [`runtime::Launcher`] that executes plans through a
//!   [`runtime::Spawner`], tracks the game and emits [`runtime::LaunchEvent`]s.
//! * [`record`]: session / playtime / recent-games writes, byte-compatible
//!   with Heronite.
//! * [`steam`]: `steam://` URIs and installed-app detection.

pub mod emulators;
pub mod env;
pub mod install_state;
pub mod legendary;
pub mod library;
pub mod native;
pub mod plan;
pub mod playtime_watch;
pub mod probe;
pub mod procs;
pub mod record;
pub mod runtime;
pub mod settings;
pub mod steam;
pub mod vdf;
pub mod wine;

pub use plan::{
    build_plan, build_plan_with_settings, Command, LaunchOptions, LaunchPlan, PlanContext,
    ProcessPlan, StopSpec, WineRuntime,
};
pub use probe::{Probe, SystemProbe};
pub use runtime::{
    ExitInfo, LaunchError, LaunchEvent, LaunchRequest, Launcher, LauncherConfig, ProcessHandle,
    Spawner, StopOptions, SystemSpawner,
};
