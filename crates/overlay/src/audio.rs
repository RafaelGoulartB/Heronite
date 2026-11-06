//! Blocking system and per-app audio controls. Call from a worker, not the UI.
//! Linux uses PulseAudio/PipeWire's `pactl`; Windows uses Core Audio.

#[cfg(not(windows))]
mod linux;
#[cfg(not(windows))]
pub use linux::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

/// A playback stream's identity, valid across snapshot enumeration changes.
#[cfg(windows)]
pub type StreamId = heronite_platform::audio::SessionId;
#[cfg(not(windows))]
pub type StreamId = u32;

/// An output device (or an input one: [`AudioState::inputs`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    /// PulseAudio sink/source name, or Windows endpoint ID.
    pub name: String,
    /// Human name ("Arctis 7 Game").
    pub description: String,
}

/// An app playing sound now (not the game): its streams together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppAudio {
    pub name: String,
    /// The process's executable name, lowercase, when reported.
    pub binary: String,
    /// Of its first stream, in percent.
    pub volume: u16,
    /// Every stream of it is muted.
    pub muted: bool,
    pub streams: Vec<StreamId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioState {
    /// Volume of the default output, in percent (above 100 when boosted on Linux).
    pub volume: u16,
    pub muted: bool,
    pub outputs: Vec<Output>,
    /// Index of the default output in `outputs`.
    pub default_output: Option<usize>,
    /// `None` when there is no microphone.
    pub mic_muted: Option<bool>,
    pub inputs: Vec<Output>,
    /// Index of the default input in `inputs`.
    pub default_input: Option<usize>,
    /// Volume of the game's streams, when it has any, silent ones included
    /// (a game in a menu or paused still has its row).
    pub game_volume: Option<u16>,
    pub game_muted: bool,
    /// Other apps playing, in backend enumeration order.
    pub apps: Vec<AppAudio>,
}

/// Windows exposes no documented API for changing the default endpoint.
pub const fn device_switching_supported() -> bool {
    !cfg!(windows)
}
