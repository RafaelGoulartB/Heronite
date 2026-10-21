//! Options of the replay buffer (the last seconds of a game kept in memory
//! by gpu-screen-recorder and saved on request), kept in
//! `AppPaths::replay_prefs()`: they belong to this machine, not to the
//! library.
//!
//! - `enabled`: off until the user turns it on.
//! - `seconds`: what a save keeps, one of [`SECONDS`] (30 unless changed).
//! - `quality`: `low`, `balanced` or `high`, the bitrate per pixel.
//! - `fps`: one of [`FPS`].
//! - `resolution`: `native`, `1440p`, `1080p` or `720p`, the largest video.
//! - `codec`: `h264`, `hevc` or `av1`.
//! - `audio`: `game` (only the game's sound), `all` (what the PC plays) or
//!   `none`.
//! - `microphone`: the default microphone on a track of its own.
//! - `monitor`: the connector recorded (`DP-1`); empty = the game's.

use anyhow::Result;
use serde_json::{json, Value};

use crate::json_store;
use crate::paths::AppPaths;

/// Lengths offered in Settings, in seconds.
pub const SECONDS: [u32; 5] = [15, 30, 60, 120, 300];
/// Frame rates offered in Settings.
pub const FPS: [u32; 2] = [30, 60];

/// A small enum stored as a string.
macro_rules! named {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($(#[$vm])* $v),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$v),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$v => $s),+ }
            }

            pub fn parse(value: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|v| v.as_str() == value)
            }
        }
    };
}

named!(
    /// Bits spent per pixel of each frame.
    Quality { Low = "low", Balanced = "balanced", High = "high" }
);

named!(
    /// The largest video recorded: a bigger screen is scaled down to fit.
    Resolution { Native = "native", P1440 = "1440p", P1080 = "1080p", P720 = "720p" }
);

named!(
    /// H.264 plays everywhere; HEVC and AV1 give the same picture in fewer bytes.
    Codec { H264 = "h264", Hevc = "hevc", Av1 = "av1" }
);

named!(
    /// What the replay hears.
    Audio {
        /// Only the game's streams: no chat, music or notifications.
        Game = "game",
        /// Everything the default output plays.
        All = "all",
        Off = "none",
    }
);

impl Resolution {
    /// The box the video must fit in (`None` = the screen's own size).
    pub fn limit(self) -> Option<(u32, u32)> {
        match self {
            Resolution::Native => None,
            Resolution::P1440 => Some((2560, 1440)),
            Resolution::P1080 => Some((1920, 1080)),
            Resolution::P720 => Some((1280, 720)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayPrefs {
    pub enabled: bool,
    /// One of [`SECONDS`].
    pub seconds: u32,
    pub quality: Quality,
    /// One of [`FPS`].
    pub fps: u32,
    pub resolution: Resolution,
    pub codec: Codec,
    pub audio: Audio,
    pub microphone: bool,
    /// Connector name; empty = the monitor the game is on.
    pub monitor: String,
}

impl Default for ReplayPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            seconds: 30,
            quality: Quality::Balanced,
            fps: 60,
            resolution: Resolution::Native,
            codec: Codec::H264,
            audio: Audio::Game,
            microphone: false,
            monitor: String::new(),
        }
    }
}

impl ReplayPrefs {
    /// Loads the options; a missing or invalid file (or key) gives the defaults.
    pub fn load(paths: &AppPaths) -> Self {
        let value = json_store::read_value(&paths.replay_prefs()).unwrap_or(Value::Null);
        let d = Self::default();
        let text = |key: &str| value.get(key).and_then(Value::as_str);
        let number = |key: &str, allowed: &[u32], default: u32| {
            value.get(key).and_then(Value::as_u64).and_then(|n| allowed.iter().copied().find(|a| u64::from(*a) == n)).unwrap_or(default)
        };
        Self {
            enabled: value.get("enabled").and_then(Value::as_bool).unwrap_or(d.enabled),
            seconds: number("seconds", &SECONDS, d.seconds),
            quality: text("quality").and_then(Quality::parse).unwrap_or(d.quality),
            fps: number("fps", &FPS, d.fps),
            resolution: text("resolution").and_then(Resolution::parse).unwrap_or(d.resolution),
            codec: text("codec").and_then(Codec::parse).unwrap_or(d.codec),
            audio: text("audio").and_then(Audio::parse).unwrap_or(d.audio),
            microphone: value.get("microphone").and_then(Value::as_bool).unwrap_or(d.microphone),
            monitor: text("monitor").map(str::trim).unwrap_or_default().to_string(),
        }
    }

    /// Writes the options atomically, keeping keys this struct does not model.
    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        json_store::update_value(&paths.replay_prefs(), |doc| {
            if !doc.is_object() {
                *doc = json!({});
            }
            doc["enabled"] = json!(self.enabled);
            doc["seconds"] = json!(self.seconds);
            doc["quality"] = json!(self.quality.as_str());
            doc["fps"] = json!(self.fps);
            doc["resolution"] = json!(self.resolution.as_str());
            doc["codec"] = json!(self.codec.as_str());
            doc["audio"] = json!(self.audio.as_str());
            doc["microphone"] = json!(self.microphone);
            doc["monitor"] = json!(self.monitor);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_and_lenient_load() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_data(&crate::DataPaths::at(dir.path().join("heroic")));
        let d = ReplayPrefs::load(&paths);
        assert_eq!(d, ReplayPrefs::default());
        assert!(!d.enabled, "the replay is off until the user turns it on");
        assert_eq!(d.seconds, 30);

        std::fs::create_dir_all(paths.replay_prefs().parent().unwrap()).unwrap();
        std::fs::write(
            paths.replay_prefs(),
            r#"{"enabled": "yes", "seconds": 45, "fps": 144, "quality": "max", "codec": "vp9", "audio": "mic", "other": 1}"#,
        )
        .unwrap();
        assert_eq!(ReplayPrefs::load(&paths), ReplayPrefs::default());

        let p = ReplayPrefs {
            enabled: true,
            seconds: 120,
            quality: Quality::High,
            fps: 30,
            resolution: Resolution::P1080,
            codec: Codec::Av1,
            audio: Audio::All,
            microphone: true,
            monitor: "DP-1".into(),
        };
        p.save(&paths).unwrap();
        assert_eq!(ReplayPrefs::load(&paths), p);
        assert_eq!(json_store::read_value(&paths.replay_prefs()).unwrap()["other"], 1);
    }

    #[test]
    fn names_parse_back() {
        for q in Quality::ALL {
            assert_eq!(Quality::parse(q.as_str()), Some(*q));
        }
        for r in Resolution::ALL {
            assert_eq!(Resolution::parse(r.as_str()), Some(*r));
        }
        assert_eq!(Resolution::P1080.limit(), Some((1920, 1080)));
        assert_eq!(Resolution::Native.limit(), None);
        assert_eq!(Codec::parse("hevc"), Some(Codec::Hevc));
        assert_eq!(Audio::parse("none"), Some(Audio::Off));
    }
}
