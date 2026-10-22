//! What a recording costs and the command line that starts it. Pure, so
//! Settings can show the memory a choice takes before anything runs.
//!
//! The bitrate is constant (`-bm cbr`): the buffer's size then depends only
//! on the bitrate and its length, not on what the game shows.

use std::ffi::OsString;
use std::path::PathBuf;

use heronite_data::replay_prefs::{Audio, Codec, Quality, ReplayPrefs};

use super::info::Screen;

/// Seconds kept beyond the length saved: a save from the panel asks for the
/// time it was open too, then cuts it off the end (the panel is in the
/// picture).
pub const PANEL_MARGIN: u32 = 10;
/// Keyframe interval (gpu-screen-recorder's default): a save starts at a
/// keyframe, so the buffer holds this much more.
const KEYINT: u32 = 2;
/// Opus, per audio track.
pub const AUDIO_KBPS: u32 = 128;
/// Private memory gpu-screen-recorder takes besides the buffer, measured on
/// an RTX 5070 (driver 615, gpu-screen-recorder 6.1.4) with the buffer on
/// disk. NVENC: its heap, CUDA's and the driver's pinned memory (about
/// 75 MiB). Without NVIDIA it is less (an estimate: not measured). The sound
/// (PipeWire, the Opus encoder) adds its share.
const PROCESS_BYTES_NVENC: u64 = 130 * MB;
const PROCESS_BYTES_OTHER: u64 = 40 * MB;
const PROCESS_BYTES_AUDIO: u64 = 15 * MB;
/// Video memory: a base plus the frames the encoder keeps at the video's
/// size. Measured the same way (the card's used memory with and without the
/// recorder): NVENC takes 378 MiB for an H.264 video of 3440 × 1440,
/// 230 MiB for 1920 × 804, 261 MiB in HEVC and 302 MiB in AV1. VA-API keeps
/// fewer frames (an estimate: not measured).
const VRAM_BASE_NVIDIA: u64 = 160 * MB;
const VRAM_BASE_OTHER: u64 = 40 * MB;
const MB: u64 = 1024 * 1024;

fn vram_per_megapixel(codec: Codec, nvidia: bool) -> u64 {
    match (nvidia, codec) {
        (true, Codec::H264) => 44 * MB,
        (true, Codec::Hevc) => 20 * MB,
        (true, Codec::Av1) => 29 * MB,
        (false, _) => 18 * MB,
    }
}

/// Bits per pixel of each frame, for H.264.
fn bits_per_pixel(q: Quality) -> f64 {
    match q {
        Quality::Low => 0.05,
        Quality::Balanced => 0.08,
        Quality::High => 0.12,
    }
}

/// HEVC and AV1 look the same with fewer bits.
fn codec_factor(c: Codec) -> f64 {
    match c {
        Codec::H264 => 1.0,
        Codec::Hevc => 0.7,
        Codec::Av1 => 0.6,
    }
}

/// The video's size: the screen's, or scaled down (keeping its shape) to
/// fit the resolution chosen. Even sides, as the encoders want.
pub fn video_size(prefs: &ReplayPrefs, width: u32, height: u32) -> (u32, u32) {
    let even = |n: f64| ((n / 2.).round() as u32 * 2).max(2);
    match prefs.resolution.limit() {
        Some((lw, lh)) if width > lw || height > lh => {
            let scale = (f64::from(lw) / f64::from(width)).min(f64::from(lh) / f64::from(height));
            (even(f64::from(width) * scale), even(f64::from(height) * scale))
        }
        _ => (even(f64::from(width)), even(f64::from(height))),
    }
}

/// Video bitrate in kbit/s, for a video of `width` × `height`.
pub fn video_kbps(prefs: &ReplayPrefs, codec: Codec, width: u32, height: u32) -> u32 {
    let bits = f64::from(width) * f64::from(height) * f64::from(prefs.fps) * bits_per_pixel(prefs.quality) * codec_factor(codec);
    let kbps = (bits / 1000. / 500.).round() as u32 * 500;
    kbps.clamp(2_000, 150_000)
}

/// Audio tracks: the game's (or everything's) and the microphone's.
pub fn audio_tracks(prefs: &ReplayPrefs) -> u32 {
    u32::from(prefs.audio != Audio::Off) + u32::from(prefs.microphone)
}

/// Seconds the buffer holds.
pub fn buffer_seconds(prefs: &ReplayPrefs) -> u32 {
    prefs.seconds + PANEL_MARGIN
}

/// The cost of the options on a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Estimate {
    /// Size of the video.
    pub width: u32,
    pub height: u32,
    pub video_kbps: u32,
    /// Memory while a game runs: the buffer plus the recorder itself.
    pub ram_bytes: u64,
    /// The graphics card's memory while a game runs: it grows with the
    /// video's size, not its length.
    pub vram_bytes: u64,
    /// One saved replay on disk.
    pub file_bytes: u64,
}

/// `vendor` as `--info` says it (`nvidia` takes more memory for CUDA).
pub fn estimate(prefs: &ReplayPrefs, codec: Codec, screen_w: u32, screen_h: u32, vendor: &str) -> Estimate {
    let (width, height) = video_size(prefs, screen_w, screen_h);
    let video_kbps = video_kbps(prefs, codec, width, height);
    let bytes_per_sec = u64::from(video_kbps + audio_tracks(prefs) * AUDIO_KBPS) * 125;
    // Each packet is kept with a little bookkeeping, and CBR overshoots a bit.
    let buffer = u64::from(buffer_seconds(prefs) + KEYINT) * bytes_per_sec * 21 / 20;
    let nvidia = vendor.eq_ignore_ascii_case("nvidia");
    let audio = if audio_tracks(prefs) > 0 { PROCESS_BYTES_AUDIO } else { 0 };
    let process = (if nvidia { PROCESS_BYTES_NVENC } else { PROCESS_BYTES_OTHER }) + audio;
    let base = if nvidia { VRAM_BASE_NVIDIA } else { VRAM_BASE_OTHER };
    let vram_bytes = base + vram_per_megapixel(codec, nvidia) * u64::from(width) * u64::from(height) / 1_000_000;
    Estimate { width, height, video_kbps, ram_bytes: buffer + process, vram_bytes, file_bytes: u64::from(prefs.seconds) * bytes_per_sec }
}

/// One recording, ready to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub screen: Screen,
    pub fps: u32,
    pub codec: Codec,
    pub video_kbps: u32,
    /// `-s`, when the video is smaller than the screen.
    pub scale_to: Option<(u32, u32)>,
    pub buffer_seconds: u32,
    /// One `-a` per track (`app:a|app:b`, `default_output`, `default_input`).
    pub audio: Vec<String>,
    /// Where it writes the saved replays (they are moved away at once).
    pub out_dir: PathBuf,
    pub socket: PathBuf,
}

impl Plan {
    /// `codec` is one the card encodes; `game_audio` the `-a` of the game's
    /// sound (`None` = nothing yet).
    pub fn new(prefs: &ReplayPrefs, screen: Screen, codec: Codec, game_audio: Option<String>, out_dir: PathBuf, socket: PathBuf) -> Self {
        let (w, h) = video_size(prefs, screen.width, screen.height);
        let scaled = w < screen.width || h < screen.height;
        let mut audio = Vec::new();
        match prefs.audio {
            Audio::Game => audio.extend(game_audio),
            Audio::All => audio.push("default_output".into()),
            Audio::Off => {}
        }
        if prefs.microphone {
            audio.push("default_input".into());
        }
        Self {
            fps: prefs.fps,
            codec,
            video_kbps: video_kbps(prefs, codec, w, h),
            scale_to: scaled.then_some((w, h)),
            buffer_seconds: buffer_seconds(prefs),
            audio,
            out_dir,
            socket,
            screen,
        }
    }

    /// The arguments of `gpu-screen-recorder`. The encoder is the graphics
    /// card's, with no fallback to the CPU; no cursor; the buffer in memory.
    pub fn args(&self) -> Vec<OsString> {
        let mut a: Vec<OsString> = Vec::new();
        let mut push = |k: &str, v: OsString| {
            a.push(k.into());
            a.push(v);
        };
        push("-w", self.screen.name.clone().into());
        push("-c", "mp4".into());
        push("-k", self.codec.as_str().into());
        push("-bm", "cbr".into());
        push("-q", self.video_kbps.to_string().into());
        push("-f", self.fps.to_string().into());
        push("-fm", "vfr".into());
        push("-r", self.buffer_seconds.to_string().into());
        push("-replay-storage", "ram".into());
        push("-encoder", "gpu".into());
        push("-cursor", "no".into());
        push("-v", "no".into());
        if let Some((w, h)) = self.scale_to {
            push("-s", format!("{w}x{h}").into());
        }
        for source in &self.audio {
            push("-a", source.into());
        }
        if !self.audio.is_empty() {
            push("-ac", "opus".into());
            push("-ab", AUDIO_KBPS.to_string().into());
        }
        push("-o", self.out_dir.clone().into_os_string());
        push("-ipc", self.socket.clone().into_os_string());
        a
    }
}

/// `-a` for the game's streams (`app:a|app:b`); `None` without any.
pub fn app_audio(names: &[String]) -> Option<String> {
    // `|` joins sources: a name holding one cannot be asked for.
    let list: Vec<String> = names.iter().filter(|n| !n.is_empty() && !n.contains('|')).map(|n| format!("app:{n}")).collect();
    (!list.is_empty()).then(|| list.join("|"))
}

#[cfg(test)]
mod tests {
    use heronite_data::replay_prefs::Resolution;

    use super::*;

    fn screen(w: u32, h: u32) -> Screen {
        Screen { name: "DP-1".into(), width: w, height: h }
    }

    #[test]
    fn the_video_fits_the_chosen_resolution() {
        let mut p = ReplayPrefs::default();
        assert_eq!(video_size(&p, 3440, 1440), (3440, 1440));
        p.resolution = Resolution::P1080;
        assert_eq!(video_size(&p, 3440, 1440), (1920, 804), "ultrawide keeps its shape");
        assert_eq!(video_size(&p, 2560, 1440), (1920, 1080));
        assert_eq!(video_size(&p, 1920, 1080), (1920, 1080), "never scaled up");
        assert_eq!(video_size(&p, 1280, 800), (1280, 800));
    }

    #[test]
    fn bitrate_follows_pixels_frames_and_quality() {
        let mut p = ReplayPrefs::default();
        assert_eq!(video_kbps(&p, Codec::H264, 1920, 1080), 10_000);
        assert_eq!(video_kbps(&p, Codec::H264, 2560, 1440), 17_500);
        assert_eq!(video_kbps(&p, Codec::Hevc, 2560, 1440), 12_500);
        p.fps = 30;
        p.quality = Quality::Low;
        assert_eq!(video_kbps(&p, Codec::H264, 1280, 720), 2_000, "never below the floor");
    }

    #[test]
    fn memory_grows_with_the_length() {
        let p = ReplayPrefs::default();
        let thirty = estimate(&p, Codec::H264, 2560, 1440, "nvidia");
        let long = estimate(&ReplayPrefs { seconds: 60, ..p.clone() }, Codec::H264, 2560, 1440, "nvidia");
        assert!(long.ram_bytes > thirty.ram_bytes);
        // 30 s + 10 s of margin + 2 s of keyframe at 17.5 Mbit/s + 128 kbit/s.
        let buffer = 42 * (17_500 + 128) * 125 * 21 / 20;
        assert_eq!(thirty.ram_bytes, buffer + PROCESS_BYTES_NVENC + PROCESS_BYTES_AUDIO);
        assert_eq!(thirty.file_bytes, 30 * (17_500 + 128) * 125);
        assert_eq!(long.vram_bytes, thirty.vram_bytes, "the length is in RAM only");
        assert!(estimate(&p, Codec::H264, 2560, 1440, "amd").ram_bytes < thirty.ram_bytes);
    }

    #[test]
    fn video_memory_follows_the_video_size_and_the_codec() {
        let mib = |e: Estimate| e.vram_bytes / MB;
        let p = ReplayPrefs::default();
        // Measured on an RTX 5070: 378 MiB at 3440 × 1440, 230 MiB scaled to
        // 1920 × 804, 261 MiB in HEVC.
        assert!((365..390).contains(&mib(estimate(&p, Codec::H264, 3440, 1440, "nvidia"))));
        let p1080 = ReplayPrefs { resolution: Resolution::P1080, ..p.clone() };
        assert!((220..240).contains(&mib(estimate(&p1080, Codec::H264, 3440, 1440, "nvidia"))));
        assert!((250..270).contains(&mib(estimate(&p, Codec::Hevc, 3440, 1440, "nvidia"))));
        assert!(estimate(&p, Codec::H264, 3440, 1440, "amd").vram_bytes < estimate(&p, Codec::H264, 3440, 1440, "nvidia").vram_bytes);
    }

    #[test]
    fn arguments_record_the_screen_on_the_card() {
        let prefs = ReplayPrefs { resolution: Resolution::P1080, microphone: true, ..Default::default() };
        let plan = Plan::new(
            &prefs,
            screen(2560, 1440),
            Codec::H264,
            app_audio(&["eldenring.exe".into(), "a|b".into()]),
            "/c/.replay".into(),
            "/run/x.sock".into(),
        );
        let args: Vec<String> = plan.args().into_iter().map(|a| a.into_string().unwrap()).collect();
        let joined = args.join(" ");
        assert!(joined.starts_with("-w DP-1 -c mp4 -k h264 -bm cbr -q 10000 -f 60 -fm vfr -r 40 -replay-storage ram -encoder gpu"));
        assert!(joined.contains("-s 1920x1080"));
        assert!(joined.contains("-a app:eldenring.exe -a default_input -ac opus -ab 128"));
        assert!(joined.ends_with("-o /c/.replay -ipc /run/x.sock"));
        assert!(!joined.contains("fallback-cpu"), "never the CPU");

        let quiet = Plan::new(
            &ReplayPrefs { audio: Audio::Off, ..Default::default() },
            screen(1920, 1080),
            Codec::Hevc,
            None,
            "/o".into(),
            "/s".into(),
        );
        let joined = quiet.args().into_iter().map(|a| a.into_string().unwrap()).collect::<Vec<_>>().join(" ");
        assert!(!joined.contains("-a ") && !joined.contains("-s "));
        assert_eq!(app_audio(&[]), None);
    }
}
