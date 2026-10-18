//! System audio for the overlay through `pactl` (PulseAudio, or PipeWire's
//! pulse server, which every PipeWire desktop runs): the output volume, its
//! mute, the default output, the default microphone and its mute, the volume of the game's
//! own streams and of the other apps playing (a browser tab, a music
//! player). Every call is blocking (it runs `pactl`).

use std::process::Command;
use std::time::Duration;

use heronite_data::proc::Run;

use crate::procs::belongs_to;
use serde_json::Value;

/// An output device (or an input one: [`AudioState::inputs`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    /// pactl's sink (or source) name (what `set-default-sink` takes).
    pub name: String,
    /// Human name ("Arctis 7 Game").
    pub description: String,
}

/// An app playing sound now (not the game): its streams together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppAudio {
    /// `application.name` ("Google Chrome", "Spotify").
    pub name: String,
    /// `application.process.binary`, lowercase (`chrome`), when reported.
    pub binary: String,
    /// Of its first stream, in percent.
    pub volume: u16,
    /// Every stream of it is muted.
    pub muted: bool,
    /// Its streams (pactl sink-input indexes).
    pub streams: Vec<u32>,
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
    /// The microphones (input devices; not the outputs' monitors).
    pub inputs: Vec<Output>,
    /// Index of the default input in `inputs`.
    pub default_input: Option<usize>,
    /// Volume of the game's streams, when it plays any.
    pub game_volume: Option<u16>,
    pub game_muted: bool,
    /// Other apps playing, in the order they started.
    pub apps: Vec<AppAudio>,
}

fn pactl(args: &[&str]) -> Option<String> {
    let out = Run::new().timeout(Duration::from_secs(3)).run(Command::new("pactl").args(args)).ok()?;
    if !out.success() {
        log::debug!("pactl {}: {}", args.join(" "), out.stderr_text().trim());
        return None;
    }
    Some(out.stdout_text())
}

/// The current state; `None` when `pactl` is missing or no server answers.
/// `game_pid` is the game's process (group leader): its streams give
/// [`AudioState::game_volume`].
pub fn read(game_pid: Option<u32>) -> Option<AudioState> {
    let sinks = parse_sinks(&pactl(&["-f", "json", "list", "sinks"])?);
    let default = pactl(&["get-default-sink"]).map(|s| s.trim().to_string()).unwrap_or_default();
    let default_output = sinks.iter().position(|s| s.output.name == default);
    let (volume, muted) = default_output.map(|i| (sinks[i].volume, sinks[i].muted)).unwrap_or_default();
    let mic_muted = pactl(&["get-source-mute", "@DEFAULT_SOURCE@"]).and_then(|s| parse_mute(&s));
    let inputs = pactl(&["-f", "json", "list", "sources"]).map(|t| parse_sources(&t)).unwrap_or_default();
    let default_source = pactl(&["get-default-source"]).map(|s| s.trim().to_string()).unwrap_or_default();
    let default_input = inputs.iter().position(|s| s.name == default_source);
    let streams = pactl(&["-f", "json", "list", "sink-inputs"]).map(|t| parse_sink_inputs(&t)).unwrap_or_default();
    let is_game = |s: &SinkInput| game_pid.zip(s.pid).is_some_and(|(game, pid)| belongs_to(pid, game));
    let game = streams.iter().find(|s| is_game(s));
    let apps = group_apps(streams.iter().filter(|s| !is_game(s)));
    Some(AudioState {
        volume,
        muted,
        outputs: sinks.into_iter().map(|s| s.output).collect(),
        default_output,
        mic_muted,
        inputs,
        default_input,
        game_volume: game.as_ref().map(|s| s.volume),
        game_muted: game.is_some_and(|s| s.muted),
        apps,
    })
}

/// The apps playing now: streams not paused (corked) and not event sounds
/// (notifications), grouped by app name.
fn group_apps<'a>(inputs: impl Iterator<Item = &'a SinkInput>) -> Vec<AppAudio> {
    let mut apps: Vec<AppAudio> = Vec::new();
    for s in inputs.filter(|s| !s.corked && s.role != "event") {
        match apps.iter_mut().find(|a| a.name == s.name) {
            Some(a) => {
                a.streams.push(s.index);
                a.muted &= s.muted;
            }
            None => apps.push(AppAudio {
                name: s.name.clone(),
                binary: s.binary.clone(),
                volume: s.volume,
                muted: s.muted,
                streams: vec![s.index],
            }),
        }
    }
    apps
}

/// Sets the volume of an app's streams (and unmutes them).
pub fn set_streams_volume(streams: &[u32], percent: u16) {
    for index in streams {
        let index = index.to_string();
        pactl(&["set-sink-input-volume", &index, &format!("{percent}%")]);
        pactl(&["set-sink-input-mute", &index, "0"]);
    }
}

pub fn set_streams_muted(streams: &[u32], muted: bool) {
    for index in streams {
        pactl(&["set-sink-input-mute", &index.to_string(), if muted { "1" } else { "0" }]);
    }
}

/// Sets the default output's volume (percent) and unmutes it.
pub fn set_volume(percent: u16) {
    pactl(&["set-sink-volume", "@DEFAULT_SINK@", &format!("{percent}%")]);
    pactl(&["set-sink-mute", "@DEFAULT_SINK@", "0"]);
}

/// Moves the default output's volume by `delta` percent, between 0 and 100
/// (a volume boosted above 100 is not raised further). Returns the new one.
pub fn step_volume(delta: i32) -> Option<u16> {
    let sinks = parse_sinks(&pactl(&["-f", "json", "list", "sinks"])?);
    let default = pactl(&["get-default-sink"])?.trim().to_string();
    let current = sinks.iter().find(|s| s.output.name == default)?.volume;
    let next = stepped(current, delta);
    set_volume(next);
    Some(next)
}

fn stepped(current: u16, delta: i32) -> u16 {
    let top = i32::from(current).max(100);
    (i32::from(current) + delta).clamp(0, top) as u16
}

pub fn set_muted(muted: bool) {
    pactl(&["set-sink-mute", "@DEFAULT_SINK@", if muted { "1" } else { "0" }]);
}

pub fn set_default_output(name: &str) {
    pactl(&["set-default-sink", name]);
}

/// Makes `name` (a source of [`AudioState::inputs`]) the microphone.
pub fn set_default_input(name: &str) {
    pactl(&["set-default-source", name]);
}

pub fn set_mic_muted(muted: bool) {
    pactl(&["set-source-mute", "@DEFAULT_SOURCE@", if muted { "1" } else { "0" }]);
}

/// Sets the volume of every stream the game plays (and unmutes them).
pub fn set_game_volume(game_pid: u32, percent: u16) {
    for s in game_streams(game_pid) {
        let index = s.index.to_string();
        pactl(&["set-sink-input-volume", &index, &format!("{percent}%")]);
        pactl(&["set-sink-input-mute", &index, "0"]);
    }
}

pub fn set_game_muted(game_pid: u32, muted: bool) {
    for s in game_streams(game_pid) {
        pactl(&["set-sink-input-mute", &s.index.to_string(), if muted { "1" } else { "0" }]);
    }
}

/// The game's playback streams. A stream belongs to the game when its
/// process is in the game's process group (the launcher starts each game as
/// its own group) or descends from it.
fn game_streams(game_pid: u32) -> Vec<SinkInput> {
    let Some(text) = pactl(&["-f", "json", "list", "sink-inputs"]) else { return Vec::new() };
    parse_sink_inputs(&text).into_iter().filter(|s| s.pid.is_some_and(|pid| belongs_to(pid, game_pid))).collect()
}

struct Sink {
    output: Output,
    volume: u16,
    muted: bool,
}

fn parse_sinks(json: &str) -> Vec<Sink> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    items
        .iter()
        .filter_map(|s| {
            let name = s.get("name")?.as_str()?.to_string();
            let description = s.get("description").and_then(Value::as_str).unwrap_or(&name).to_string();
            Some(Sink {
                output: Output { name, description },
                volume: average_percent(s.get("volume")),
                muted: s.get("mute").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

/// The input devices, without the monitors of the outputs.
fn parse_sources(json: &str) -> Vec<Output> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    items
        .iter()
        .filter_map(|s| {
            let name = s.get("name")?.as_str()?.to_string();
            let class = s.get("properties").and_then(|p| p.get("device.class")).and_then(Value::as_str);
            if class == Some("monitor") || name.ends_with(".monitor") {
                return None;
            }
            let description = s.get("description").and_then(Value::as_str).unwrap_or(&name).to_string();
            Some(Output { name, description })
        })
        .collect()
}

struct SinkInput {
    index: u32,
    /// Not reported by native PipeWire streams.
    pid: Option<u32>,
    name: String,
    binary: String,
    /// `media.role` (`event` for notification sounds).
    role: String,
    /// Paused.
    corked: bool,
    volume: u16,
    muted: bool,
}

fn parse_sink_inputs(json: &str) -> Vec<SinkInput> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    items
        .iter()
        .filter_map(|s| {
            let index = u32::try_from(s.get("index")?.as_u64()?).ok()?;
            let props = s.get("properties")?;
            let prop = |key: &str| props.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
            let binary = prop("application.process.binary").unwrap_or_default().to_lowercase();
            let name = prop("application.name")
                .or(prop("application.process.binary"))
                .or(prop("node.name"))
                .or(prop("media.name"))
                .unwrap_or("?")
                .to_string();
            Some(SinkInput {
                index,
                pid: prop("application.process.id").and_then(|p| p.parse().ok()),
                name,
                binary,
                role: prop("media.role").unwrap_or_default().to_string(),
                corked: s.get("corked").and_then(Value::as_bool).unwrap_or(false),
                volume: average_percent(s.get("volume")),
                muted: s.get("mute").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

/// pactl's volume object: one entry per channel with `"value_percent": "70%"`.
fn average_percent(volume: Option<&Value>) -> u16 {
    let Some(Value::Object(channels)) = volume else { return 0 };
    let values: Vec<u32> =
        channels.values().filter_map(|c| c.get("value_percent")?.as_str()?.trim().trim_end_matches('%').trim().parse().ok()).collect();
    if values.is_empty() {
        return 0;
    }
    let sum: u32 = values.iter().sum();
    (sum as f32 / values.len() as f32).round() as u16
}

/// `Mute: yes`.
fn parse_mute(text: &str) -> Option<bool> {
    match text.trim().strip_prefix("Mute:")?.trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINKS: &str = r#"[
      {"index":55,"name":"alsa_output.usb-SteelSeries_Arctis_7-00.analog-stereo","description":"Arctis 7 Game","mute":false,
       "volume":{"front-left":{"value":45875,"value_percent":"70%","db":"-9.29 dB"},"front-right":{"value":45875,"value_percent":"71%","db":"-9.29 dB"}}},
      {"index":56,"name":"alsa_output.pci-0000_01_00.1.hdmi-stereo","mute":true,"volume":{"mono":{"value_percent":"100%"}}}
    ]"#;

    #[test]
    fn parses_sinks() {
        let s = parse_sinks(SINKS);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].output.description, "Arctis 7 Game");
        assert_eq!(s[0].volume, 71, "channels are averaged and rounded");
        assert!(!s[0].muted);
        assert_eq!(s[1].output.description, s[1].output.name, "no description: the name");
        assert!(s[1].muted);
        assert!(parse_sinks("not json").is_empty());
    }

    #[test]
    fn parses_sink_inputs_and_mute() {
        let json = r#"[{"index":7,"mute":true,"properties":{"application.process.id":"4242","application.name":"wine"},
                       "volume":{"front-left":{"value_percent":"85%"},"front-right":{"value_percent":"85%"}}},
                      {"index":8,"properties":{"node.name":"pw-play"}},
                      {"properties":{}}]"#;
        let s = parse_sink_inputs(json);
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].index, s[0].pid, s[0].volume, s[0].muted), (7, Some(4242), 85, true));
        assert_eq!((s[1].pid, s[1].name.as_str()), (None, "pw-play"), "native PipeWire streams have no pid");
        assert_eq!(parse_mute("Mute: yes\n"), Some(true));
        assert_eq!(parse_mute("Mute: no"), Some(false));
        assert_eq!(parse_mute("Failure"), None);
    }

    #[test]
    fn sources_leave_out_the_monitors() {
        let json = r#"[
          {"name":"alsa_input.usb-046d_C270.mono-fallback","description":"Webcam C270 Mono","properties":{"device.class":"sound"}},
          {"name":"alsa_output.pci-0000_0d_00.6.analog-stereo.monitor","description":"Monitor of Ryzen","properties":{"device.class":"monitor"}},
          {"name":"alsa_input.pci-0000_0d_00.6.analog-stereo","properties":{}}
        ]"#;
        let s = parse_sources(json);
        assert_eq!(
            s.iter().map(|o| o.description.as_str()).collect::<Vec<_>>(),
            ["Webcam C270 Mono", "alsa_input.pci-0000_0d_00.6.analog-stereo"]
        );
    }

    #[test]
    fn groups_the_apps_playing() {
        let json = r#"[
          {"index":1,"corked":false,"mute":false,"properties":{"application.name":"Google Chrome","application.process.binary":"Chrome"},
           "volume":{"mono":{"value_percent":"80%"}}},
          {"index":2,"corked":false,"mute":true,"properties":{"application.name":"Spotify"},"volume":{"mono":{"value_percent":"50%"}}},
          {"index":3,"corked":false,"mute":false,"properties":{"application.name":"Google Chrome"},"volume":{"mono":{"value_percent":"100%"}}},
          {"index":4,"corked":true,"properties":{"application.name":"mpv"}},
          {"index":5,"corked":false,"properties":{"application.name":"Discord","media.role":"event"}}
        ]"#;
        let inputs = parse_sink_inputs(json);
        let apps = group_apps(inputs.iter());
        assert_eq!(apps.len(), 2, "paused streams and event sounds are left out");
        assert_eq!((apps[0].name.as_str(), apps[0].binary.as_str(), apps[0].volume), ("Google Chrome", "chrome", 80));
        assert_eq!(apps[0].streams, vec![1, 3], "one row per app");
        assert!(!apps[0].muted);
        assert!(apps[1].muted);
    }

    #[test]
    fn steps_stay_in_range() {
        assert_eq!(stepped(70, 5), 75);
        assert_eq!(stepped(98, 5), 100);
        assert_eq!(stepped(3, -5), 0);
        assert_eq!(stepped(120, 5), 120, "a boosted volume is not raised");
        assert_eq!(stepped(120, -5), 115);
    }
}
