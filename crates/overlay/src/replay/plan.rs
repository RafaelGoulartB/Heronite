//! What a recording costs and the command line that starts it. Pure, so
//! Settings can show the memory a choice takes before anything runs.
//!
//! The bitrate is constant (`-bm cbr`): the buffer's size then depends only
//! on the bitrate and its length, not on what the game shows.

use std::ffi::OsString;
use std::path::PathBuf;

use heronite_data::replay_prefs::{Audio, Codec, Options, Quality, Storage};

use super::info::Screen;

/// Keyframe interval (gpu-screen-recorder's default): a save starts at a
/// keyframe, so the buffer holds this much more.
const KEYINT: u32 = 2;
/// Opus, per audio track.
pub const AUDIO_KBPS: u32 = 128;
/// Private memory gpu-screen-recorder takes besides the buffer, measured on
/// an RTX 5070 (driver 615, gpu-screen-recorder 6.1.4) with the buffer on
/// disk. NVENC: its heap, CUDA's and the driver's pinned memory (about
/// 75 MiB); Vulkan has no CUDA. Without NVIDIA it is less (an estimate: not
/// measured). The sound (PipeWire, the Opus encoder) adds its share.
const PROCESS_BYTES_NVENC: u64 = 130 * MB;
const PROCESS_BYTES_VULKAN_NVIDIA: u64 = 90 * MB;
const PROCESS_BYTES_OTHER: u64 = 40 * MB;
const PROCESS_BYTES_AUDIO: u64 = 15 * MB;
/// Video memory: a base plus the frames the encoder keeps at the video's
/// size. Measured the same way (the card's used memory with and without the
/// recorder): NVENC takes 378 MiB for an H.264 video of 3440 × 1440,
/// 230 MiB for 1920 × 804, 261 MiB in HEVC and 302 MiB in AV1; Vulkan about
/// 200 MiB whatever the size and codec. VA-API keeps fewer frames (an
/// estimate: not measured).
const VRAM_BASE_NVIDIA: u64 = 160 * MB;
const VRAM_BASE_OTHER: u64 = 40 * MB;
const MB: u64 = 1024 * 1024;

fn vram_per_megapixel(codec: Codec, nvidia: bool, vulkan: bool) -> u64 {
    match (nvidia, vulkan, codec) {
        (true, true, _) => 8 * MB,
        (true, false, Codec::H264) => 44 * MB,
        (true, false, Codec::Hevc) => 20 * MB,
        (true, false, Codec::Av1) => 29 * MB,
        (false, ..) => 18 * MB,
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
pub fn video_size(opts: &Options, width: u32, height: u32) -> (u32, u32) {
    let even = |n: f64| ((n / 2.).round() as u32 * 2).max(2);
    match opts.resolution.limit() {
        Some((lw, lh)) if width > lw || height > lh => {
            let scale = (f64::from(lw) / f64::from(width)).min(f64::from(lh) / f64::from(height));
            (even(f64::from(width) * scale), even(f64::from(height) * scale))
        }
        _ => (even(f64::from(width)), even(f64::from(height))),
    }
}

/// Video bitrate in kbit/s, for a video of `width` × `height`.
pub fn video_kbps(opts: &Options, codec: Codec, width: u32, height: u32) -> u32 {
    let bits = f64::from(width) * f64::from(height) * f64::from(opts.fps) * bits_per_pixel(opts.quality) * codec_factor(codec);
    let kbps = (bits / 1000. / 500.).round() as u32 * 500;
    kbps.clamp(2_000, 150_000)
}

/// Audio tracks: the game's (or everything's) and the microphone's.
pub fn audio_tracks(opts: &Options) -> u32 {
    u32::from(opts.audio != Audio::Off) + u32::from(opts.microphone)
}

/// Seconds the buffer holds: the length saved and the panel's margin.
pub fn buffer_seconds(opts: &Options) -> u32 {
    opts.seconds + opts.panel_margin
}

/// How the video is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encoding {
    /// One the card encodes.
    pub codec: Codec,
    /// Through Vulkan video, else the card's usual encoder.
    pub vulkan: bool,
}

/// The cost of the options on a screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Estimate {
    /// Size of the video.
    pub width: u32,
    pub height: u32,
    pub video_kbps: u32,
    /// Memory while a game runs: the buffer (when kept in RAM) plus the
    /// recorder itself.
    pub ram_bytes: u64,
    /// The graphics card's memory while a game runs: it grows with the
    /// video's size, not its length.
    pub vram_bytes: u64,
    /// One saved replay on disk.
    pub file_bytes: u64,
    /// What the buffer writes to disk each second while a game runs (0 when
    /// it is in RAM).
    pub disk_bytes_per_sec: u64,
}

/// `vendor` as `--info` says it (`nvidia` takes more memory for CUDA).
pub fn estimate(opts: &Options, encoding: Encoding, screen_w: u32, screen_h: u32, vendor: &str) -> Estimate {
    let (width, height) = video_size(opts, screen_w, screen_h);
    let video_kbps = video_kbps(opts, encoding.codec, width, height);
    let bytes_per_sec = u64::from(video_kbps + audio_tracks(opts) * AUDIO_KBPS) * 125;
    let in_ram = opts.storage == Storage::Ram;
    // Each packet is kept with a little bookkeeping, and CBR overshoots a bit.
    let buffer = if in_ram { u64::from(buffer_seconds(opts) + KEYINT) * bytes_per_sec * 21 / 20 } else { 0 };
    let nvidia = vendor.eq_ignore_ascii_case("nvidia");
    let process = match (nvidia, encoding.vulkan) {
        (true, false) => PROCESS_BYTES_NVENC,
        (true, true) => PROCESS_BYTES_VULKAN_NVIDIA,
        (false, _) => PROCESS_BYTES_OTHER,
    };
    let audio = if audio_tracks(opts) > 0 { PROCESS_BYTES_AUDIO } else { 0 };
    let base = if nvidia { VRAM_BASE_NVIDIA } else { VRAM_BASE_OTHER };
    let vram_bytes = base + vram_per_megapixel(encoding.codec, nvidia, encoding.vulkan) * u64::from(width) * u64::from(height) / 1_000_000;
    Estimate {
        width,
        height,
        video_kbps,
        ram_bytes: buffer + process + audio,
        vram_bytes,
        file_bytes: u64::from(opts.seconds) * bytes_per_sec,
        disk_bytes_per_sec: if in_ram { 0 } else { bytes_per_sec },
    }
}

/// One recording, ready to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub screen: Screen,
    pub fps: u32,
    pub encoding: Encoding,
    pub video_kbps: u32,
    /// `-s`, when the video is smaller than the screen.
    pub scale_to: Option<(u32, u32)>,
    pub buffer_seconds: u32,
    pub storage: Storage,
    /// One `-a` per track (`app:a|app:b`, `default_output`, `default_input`).
    pub audio: Vec<String>,
    /// Where it writes the saved replays (they are moved away at once) and,
    /// on disk, the buffer.
    pub out_dir: PathBuf,
    pub socket: PathBuf,
}

impl Plan {
    /// `game_audio` is the `-a` of the game's sound (`None` = nothing yet).
    pub fn new(opts: &Options, screen: Screen, encoding: Encoding, game_audio: Option<String>, out_dir: PathBuf, socket: PathBuf) -> Self {
        let (w, h) = video_size(opts, screen.width, screen.height);
        let scaled = w < screen.width || h < screen.height;
        let mut audio = Vec::new();
        match opts.audio {
            Audio::Game => audio.extend(game_audio),
            Audio::All => audio.push("default_output".into()),
            Audio::Off => {}
        }
        if opts.microphone {
            audio.push("default_input".into());
        }
        Self {
            fps: opts.fps,
            encoding,
            video_kbps: video_kbps(opts, encoding.codec, w, h),
            scale_to: scaled.then_some((w, h)),
            buffer_seconds: buffer_seconds(opts),
            storage: opts.storage,
            audio,
            out_dir,
            socket,
            screen,
        }
    }

    /// `-k`: `hevc`, or `hevc_vulkan` through Vulkan video.
    pub fn codec_arg(&self) -> String {
        let codec = self.encoding.codec.as_str();
        if self.encoding.vulkan {
            format!("{codec}_vulkan")
        } else {
            codec.to_string()
        }
    }

    /// The arguments of `gpu-screen-recorder`. The encoder is the graphics
    /// card's, with no fallback to the CPU; no cursor.
    pub fn args(&self) -> Vec<OsString> {
        let mut a: Vec<OsString> = Vec::new();
        let mut push = |k: &str, v: OsString| {
            a.push(k.into());
            a.push(v);
        };
        push("-w", self.screen.name.clone().into());
        push("-c", "mp4".into());
        push("-k", self.codec_arg().into());
        push("-bm", "cbr".into());
        push("-q", self.video_kbps.to_string().into());
        push("-f", self.fps.to_string().into());
        push("-fm", "vfr".into());
        push("-r", self.buffer_seconds.to_string().into());
        push("-replay-storage", self.storage.as_str().into());
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

    const H264: Encoding = Encoding { codec: Codec::H264, vulkan: false };

    fn screen(w: u32, h: u32) -> Screen {
        Screen { name: "DP-1".into(), width: w, height: h }
    }

    fn native() -> Options {
        Options { resolution: Resolution::Native, ..Default::default() }
    }

    #[test]
    fn the_video_fits_the_chosen_resolution() {
        let mut p = native();
        assert_eq!(video_size(&p, 3440, 1440), (3440, 1440));
        p.resolution = Resolution::P1080;
        assert_eq!(video_size(&p, 3440, 1440), (1920, 804), "ultrawide keeps its shape");
        assert_eq!(video_size(&p, 2560, 1440), (1920, 1080));
        assert_eq!(video_size(&p, 1920, 1080), (1920, 1080), "never scaled up");
        assert_eq!(video_size(&p, 1280, 800), (1280, 800));
    }

    #[test]
    fn bitrate_follows_pixels_frames_and_quality() {
        let mut p = native();
        assert_eq!(video_kbps(&p, Codec::H264, 1920, 1080), 10_000);
        assert_eq!(video_kbps(&p, Codec::H264, 2560, 1440), 17_500);
        assert_eq!(video_kbps(&p, Codec::Hevc, 2560, 1440), 12_500);
        p.fps = 30;
        p.quality = Quality::Low;
        assert_eq!(video_kbps(&p, Codec::H264, 1280, 720), 2_000, "never below the floor");
    }

    #[test]
    fn memory_grows_with_the_length_and_the_margin() {
        let p = Options { panel_margin: 10, ..native() };
        let thirty = estimate(&p, H264, 2560, 1440, "nvidia");
        let long = estimate(&Options { seconds: 60, ..p.clone() }, H264, 2560, 1440, "nvidia");
        assert!(long.ram_bytes > thirty.ram_bytes);
        // 30 s + 10 s of margin + 2 s of keyframe at 17.5 Mbit/s + 128 kbit/s.
        let buffer = 42 * (17_500 + 128) * 125 * 21 / 20;
        assert_eq!(thirty.ram_bytes, buffer + PROCESS_BYTES_NVENC + PROCESS_BYTES_AUDIO);
        assert_eq!(thirty.file_bytes, 30 * (17_500 + 128) * 125);
        assert_eq!(thirty.disk_bytes_per_sec, 0);
        assert_eq!(long.vram_bytes, thirty.vram_bytes, "the length is in RAM only");
        assert!(estimate(&p, H264, 2560, 1440, "amd").ram_bytes < thirty.ram_bytes);
        let no_margin = estimate(&Options { panel_margin: 0, ..p }, H264, 2560, 1440, "nvidia");
        assert_eq!(thirty.ram_bytes - no_margin.ram_bytes, 10 * (17_500 + 128) * 125 * 21 / 20);
    }

    #[test]
    fn a_buffer_on_disk_takes_no_memory() {
        let p = native();
        let ram = estimate(&p, H264, 2560, 1440, "nvidia");
        let disk = estimate(&Options { storage: Storage::Disk, ..p }, H264, 2560, 1440, "nvidia");
        assert_eq!(disk.ram_bytes, PROCESS_BYTES_NVENC + PROCESS_BYTES_AUDIO);
        assert_eq!(disk.disk_bytes_per_sec, (17_500 + 128) * 125);
        assert_eq!(disk.vram_bytes, ram.vram_bytes);
        assert_eq!(disk.file_bytes, ram.file_bytes);
    }

    #[test]
    fn video_memory_follows_the_size_the_codec_and_the_encoder() {
        let mib = |e: Estimate| e.vram_bytes / MB;
        let p = native();
        let at = |o: &Options, e: Encoding| mib(estimate(o, e, 3440, 1440, "nvidia"));
        // Measured on an RTX 5070: 378 MiB at 3440 × 1440, 230 MiB scaled to
        // 1920 × 804, 261 MiB in HEVC, about 200 MiB through Vulkan.
        assert!((365..390).contains(&at(&p, H264)));
        let p1080 = Options { resolution: Resolution::P1080, ..p.clone() };
        assert!((220..240).contains(&at(&p1080, H264)));
        assert!((250..270).contains(&at(&p, Encoding { codec: Codec::Hevc, vulkan: false })));
        assert!((190..210).contains(&at(&p, Encoding { codec: Codec::H264, vulkan: true })));
        let vulkan = estimate(&p, Encoding { codec: Codec::H264, vulkan: true }, 3440, 1440, "nvidia");
        assert_eq!(estimate(&p, H264, 3440, 1440, "nvidia").ram_bytes - vulkan.ram_bytes, 40 * MB, "no CUDA: 40 MiB less");
        assert!(mib(estimate(&p, H264, 3440, 1440, "amd")) < at(&p, H264));
    }

    #[test]
    fn arguments_record_the_screen_on_the_card() {
        let prefs = Options { resolution: Resolution::P1080, microphone: true, ..native() };
        let plan = Plan::new(
            &prefs,
            screen(2560, 1440),
            H264,
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
            &Options { audio: Audio::Off, storage: Storage::Disk, panel_margin: 0, ..native() },
            screen(1920, 1080),
            Encoding { codec: Codec::Hevc, vulkan: true },
            None,
            "/o".into(),
            "/s".into(),
        );
        let joined = quiet.args().into_iter().map(|a| a.into_string().unwrap()).collect::<Vec<_>>().join(" ");
        assert!(joined.contains("-k hevc_vulkan") && joined.contains("-r 30 -replay-storage disk"));
        assert!(!joined.contains("-a ") && !joined.contains("-s "));
        assert_eq!(app_audio(&[]), None);
    }
}
