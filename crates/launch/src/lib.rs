//! Launching games and tracking playtime.
//!
//! * [`plan`]: pure `LaunchPlan` building (Steam URI, native, Wine/Proton/umu,
//!   emulators, legendary).
//! * [`runtime`]: the [`runtime::Launcher`] that executes plans through a
//!   [`runtime::Spawner`], tracks the game and emits [`runtime::LaunchEvent`]s.
//! * [`record`]: session / playtime / recent-games writes, in Heronite's
//!   formats.
//! * [`steam`]: `steam://` URIs and installed-app detection.

pub mod actions;
pub mod anticheat_data;
pub mod companions;
pub mod diagnose;
pub mod dxvk;
pub mod emulators;
pub mod env;
pub mod first_launch;
pub mod fps_limit;
pub mod gog;
pub mod gpu;
pub mod install_state;
pub mod keep_awake;
pub mod known_fixes;
pub mod launch_option;
pub mod legendary;
pub mod legendary_install;
pub mod library;
pub mod live_fps;
pub mod log_reader;
pub mod log_writer;
pub mod lsfg;
pub mod native;
pub mod net;
pub mod nile;
pub mod plan;
pub mod playtime_watch;
pub mod prepare;
pub mod probe;
pub mod procs;
pub mod record;
pub mod runtime;
pub mod runtimes;
pub mod sandbox;
pub mod scripts;
pub mod settings;
pub mod shared_folders;
pub mod steam;
pub mod store_tools;
pub mod tools;
pub mod umu_ids;
pub mod vdf;
pub mod wake;
pub mod wine;
pub mod wine_tools;
pub mod winetricks;

pub use plan::{build_plan, build_plan_with_settings, Command, LaunchOptions, LaunchPlan, PlanContext, ProcessPlan, StopSpec, WineRuntime};
pub use probe::{Probe, SystemProbe};
pub use runtime::{
    failed_launch, ExitInfo, HandoffConfig, LaunchError, LaunchEvent, LaunchRequest, Launcher, LauncherConfig, ProcessHandle, Spawner,
    StopOptions, SystemSpawner,
};
