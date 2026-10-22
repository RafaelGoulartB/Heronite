//! What gpu-screen-recorder can do on this PC, from `--info`: sections
//! (`section=<name>`) of `key|value` lines or bare words.
//!
//! ```text
//! section=system_info
//! display_server|wayland
//! supports_app_audio|yes
//! gsr_version|6.1.4
//! section=gpu_info
//! vendor|nvidia
//! section=video_codecs
//! h264
//! h264_software
//! hevc_vulkan
//! section=capture_options
//! DP-1|3440x1440
//! region
//! ```

use heronite_data::replay_prefs::{Codec, Encoder};

/// The first version with the `-ipc` socket, which saves a chosen length and
/// says where the file went.
pub const MIN_VERSION: (u32, u32, u32) = (6, 0, 0);

/// A monitor it can record, in physical pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    /// Connector name (`DP-1`), what `-w` takes.
    pub name: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Info {
    /// `None` when it did not say.
    pub version: Option<(u32, u32, u32)>,
    /// `nvidia`, `amd`, `intel`...
    pub vendor: String,
    /// It can record one app's sound (PipeWire).
    pub app_audio: bool,
    /// Video codecs; `*_software` ones are encoded by the CPU, `*_vulkan`
    /// ones through Vulkan video.
    pub codecs: Vec<String>,
    pub screens: Vec<Screen>,
}

impl Info {
    pub fn parse(text: &str) -> Self {
        let mut info = Info::default();
        let mut section = "";
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if let Some(name) = line.strip_prefix("section=") {
                section = name;
                continue;
            }
            let (key, value) = line.split_once('|').unwrap_or((line, ""));
            match section {
                "system_info" => match key {
                    "gsr_version" => info.version = parse_version(value),
                    "supports_app_audio" => info.app_audio = value == "yes",
                    _ => {}
                },
                "gpu_info" if key == "vendor" => info.vendor = value.to_string(),
                "video_codecs" => info.codecs.push(key.to_string()),
                // Monitors only: not the region, the portal nor cameras (`/dev/video0`).
                "capture_options" if !key.starts_with('/') => {
                    if let Some((w, h)) = value.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))) {
                        info.screens.push(Screen { name: key.to_string(), width: w, height: h });
                    }
                }
                _ => {}
            }
        }
        info
    }

    /// Whether this version can be driven (it has the `-ipc` socket).
    pub fn recent_enough(&self) -> bool {
        self.version.is_some_and(|v| v >= MIN_VERSION)
    }

    /// Whether the graphics card encodes `codec` (never the CPU: that would
    /// take time from the game).
    pub fn encodes(&self, codec: Codec) -> bool {
        self.codecs.iter().any(|c| c == codec.as_str())
    }

    /// `codec` when the card encodes it, else the first one it does.
    pub fn codec_for(&self, wanted: Codec) -> Option<Codec> {
        std::iter::once(wanted).chain(Codec::ALL.iter().copied()).find(|c| self.encodes(*c))
    }

    /// Whether the card also encodes `codec` through Vulkan video.
    pub fn encodes_vulkan(&self, codec: Codec) -> bool {
        self.codecs.iter().any(|c| c.strip_suffix("_vulkan") == Some(codec.as_str()))
    }

    pub fn is_nvidia(&self) -> bool {
        self.vendor.eq_ignore_ascii_case("nvidia")
    }

    /// Whether `encoder` records `codec` through Vulkan here: asked for, or
    /// `auto` on NVIDIA (no CUDA: about half the video memory and less RAM,
    /// measured on an RTX 5070), and offered by the card.
    pub fn uses_vulkan(&self, encoder: Encoder, codec: Codec) -> bool {
        let wanted = match encoder {
            Encoder::Vulkan => true,
            Encoder::Auto => self.is_nvidia(),
            Encoder::Native => false,
        };
        wanted && self.encodes_vulkan(codec)
    }

    pub fn version_text(&self) -> String {
        self.version.map(|(a, b, c)| format!("{a}.{b}.{c}")).unwrap_or_default()
    }
}

fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text.trim().split(['.', '-']).map(|p| p.parse::<u32>().ok());
    let major = parts.next()??;
    let minor = parts.next().flatten().unwrap_or(0);
    let patch = parts.next().flatten().unwrap_or(0);
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = "section=system_info\ndisplay_server|wayland\nsupports_app_audio|yes\nis_steam_deck|no\ngsr_version|6.1.4\n\
section=gpu_info\nvendor|nvidia\ncard_path|/dev/dri/card1\nsection=video_codecs\nh264\nh264_software\nhevc\nav1_10bit\nhevc_vulkan\n\
section=image_formats\njpeg\nsection=containers\nmp4\nsection=capture_options\nDP-1|3440x1440\nHDMI-A-1|1920x1080\nregion\n\
/dev/video0|1280x720@10hz|yuyv\nportal\n";

    #[test]
    fn reads_the_sections() {
        let info = Info::parse(INFO);
        assert_eq!(info.version, Some((6, 1, 4)));
        assert!(info.recent_enough());
        assert!(info.app_audio);
        assert_eq!(info.vendor, "nvidia");
        assert_eq!(info.codecs, ["h264", "h264_software", "hevc", "av1_10bit", "hevc_vulkan"]);
        assert_eq!(
            info.screens,
            [Screen { name: "DP-1".into(), width: 3440, height: 1440 }, Screen { name: "HDMI-A-1".into(), width: 1920, height: 1080 }]
        );
        assert!(info.encodes(Codec::Hevc));
        assert!(!info.encodes(Codec::Av1), "only the 10-bit AV1 is there");
        assert_eq!(info.codec_for(Codec::Av1), Some(Codec::H264));
        assert_eq!(info.version_text(), "6.1.4");
    }

    #[test]
    fn vulkan_is_used_when_asked_or_on_nvidia() {
        let info = Info::parse(INFO);
        assert!(info.encodes_vulkan(Codec::Hevc) && !info.encodes_vulkan(Codec::H264));
        assert!(info.uses_vulkan(Encoder::Auto, Codec::Hevc), "NVIDIA: Vulkan saves the CUDA context");
        assert!(!info.uses_vulkan(Encoder::Auto, Codec::H264), "not offered for H.264 here");
        assert!(!info.uses_vulkan(Encoder::Native, Codec::Hevc));
        let amd = Info { vendor: "amd".into(), ..info };
        assert!(!amd.uses_vulkan(Encoder::Auto, Codec::Hevc), "VA-API has no CUDA to save");
        assert!(amd.uses_vulkan(Encoder::Vulkan, Codec::Hevc));
    }

    #[test]
    fn old_or_silent_versions_are_not_enough() {
        assert!(!Info::parse("section=system_info\ngsr_version|5.13.8\n").recent_enough());
        assert!(!Info::parse("").recent_enough());
        assert_eq!(parse_version("6"), Some((6, 0, 0)));
        assert_eq!(parse_version("6.0.0-1"), Some((6, 0, 0)));
        assert_eq!(parse_version("x"), None);
        assert_eq!(Info::parse("section=video_codecs\nh264_software\n").codec_for(Codec::H264), None);
    }
}
