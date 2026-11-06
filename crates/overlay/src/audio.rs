//! Blocking system and per-app audio controls. Call from a worker, not the UI.
//! Uses PulseAudio/PipeWire's `pactl`.

mod linux;
pub use linux::*;

/// A playback stream's identity, valid across snapshot enumeration changes.
pub type StreamId = u32;

/// An output device (or an input one: [`AudioState::inputs`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    /// PulseAudio sink/source name.
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
    /// Volume of the default output, in percent (above 100 when boosted).
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
    /// Volume of the game's streams, when it plays any.
    pub game_volume: Option<u16>,
    pub game_muted: bool,
    /// Other apps playing, in backend enumeration order.
    pub apps: Vec<AppAudio>,
}
