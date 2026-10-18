//! System audio for the overlay through `pactl` (PulseAudio, or PipeWire's
//! pulse server, which every PipeWire desktop runs): the output volume, its
//! mute, the default output, the microphone mute and the volume of the
//! game's own streams. Every call is blocking (it runs `pactl`).

use std::process::Command;
use std::time::Duration;

use heronite_data::proc::Run;
use serde_json::Value;

/// An output device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    /// pactl's sink name (what `set-default-sink` takes).
    pub name: String,
    /// Human name ("Arctis 7 Game").
    pub description: String,
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
    /// Volume of the game's streams, when it plays any.
    pub game_volume: Option<u16>,
    pub game_muted: bool,
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
    let game = game_pid.and_then(|pid| game_streams(pid).into_iter().next());
    Some(AudioState {
        volume,
        muted,
        outputs: sinks.into_iter().map(|s| s.output).collect(),
        default_output,
        mic_muted,
        game_volume: game.as_ref().map(|s| s.volume),
        game_muted: game.is_some_and(|s| s.muted),
    })
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
    parse_sink_inputs(&text).into_iter().filter(|s| belongs_to(s.pid, game_pid)).collect()
}

fn belongs_to(pid: u32, game_pid: u32) -> bool {
    let mut current = pid;
    for _ in 0..64 {
        if current == game_pid {
            return true;
        }
        let Some((ppid, pgrp)) = proc_stat(current) else { return false };
        if pgrp == game_pid {
            return true;
        }
        if ppid <= 1 {
            return false;
        }
        current = ppid;
    }
    false
}

/// `(ppid, pgrp)` of a process.
fn proc_stat(pid: u32) -> Option<(u32, u32)> {
    parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// Fields after the command name, which may hold spaces and parentheses.
fn parse_stat(text: &str) -> Option<(u32, u32)> {
    let rest = &text[text.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace();
    let _state = fields.next()?;
    let ppid = fields.next()?.parse().ok()?;
    let pgrp = fields.next()?.parse().ok()?;
    Some((ppid, pgrp))
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

struct SinkInput {
    index: u32,
    pid: u32,
    volume: u16,
    muted: bool,
}

fn parse_sink_inputs(json: &str) -> Vec<SinkInput> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    items
        .iter()
        .filter_map(|s| {
            let index = u32::try_from(s.get("index")?.as_u64()?).ok()?;
            let pid = s.get("properties")?.get("application.process.id")?.as_str()?.parse().ok()?;
            Some(SinkInput {
                index,
                pid,
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
                      {"index":8,"properties":{}}]"#;
        let s = parse_sink_inputs(json);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].index, s[0].pid, s[0].volume, s[0].muted), (7, 4242, 85, true));
        assert_eq!(parse_mute("Mute: yes\n"), Some(true));
        assert_eq!(parse_mute("Mute: no"), Some(false));
        assert_eq!(parse_mute("Failure"), None);
    }

    #[test]
    fn steps_stay_in_range() {
        assert_eq!(stepped(70, 5), 75);
        assert_eq!(stepped(98, 5), 100);
        assert_eq!(stepped(3, -5), 0);
        assert_eq!(stepped(120, 5), 120, "a boosted volume is not raised");
        assert_eq!(stepped(120, -5), 115);
    }

    #[test]
    fn parses_proc_stat() {
        assert_eq!(parse_stat("1234 (Game (x64).exe) S 1200 1100 1100 0 -1"), Some((1200, 1100)));
        assert_eq!(parse_stat("garbage"), None);
    }
}
