//! The little of HLS the store trailers need: the master playlist's
//! variants and audio track, a media playlist's segments, and the same
//! playlist with some segments read from files on this PC.
//!
//! Steam's trailers (`hls_h264`) are a master with one variant per height
//! (1080, 720, 480, 360) and a separate audio playlist; every playlist
//! names its segments relative to itself, with an `EXT-X-MAP` init segment.

use url::Url;

/// One picture size of a master playlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    /// Absolute address of its media playlist.
    pub uri: String,
    /// `RESOLUTION`; 0 when the master does not say.
    pub width: u32,
    pub height: u32,
    /// `BANDWIDTH`, bits per second.
    pub bandwidth: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Master {
    pub variants: Vec<Variant>,
    /// Absolute address of the default audio playlist (`EXT-X-MEDIA`), when
    /// the sound is not in the variants.
    pub audio: Option<String>,
}

impl Master {
    /// The tallest variant at most `max_height` tall, else the smallest.
    pub fn pick(&self, max_height: u32) -> Option<&Variant> {
        let fits = self.variants.iter().filter(|v| v.height <= max_height).max_by_key(|v| (v.height, v.bandwidth));
        fits.or_else(|| self.variants.iter().min_by_key(|v| (v.height, v.bandwidth)))
    }
}

/// A media playlist: its lines as they are, with the addresses made absolute.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Media {
    lines: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq)]
enum Line {
    Tag(String),
    /// `EXT-X-MAP:URI="…"`, the init segment.
    Map(String),
    Segment {
        uri: String,
        seconds: f64,
    },
}

impl Media {
    /// Length in seconds (the sum of the segments).
    pub fn seconds(&self) -> f64 {
        self.lines.iter().map(|l| if let Line::Segment { seconds, .. } = l { *seconds } else { 0. }).sum()
    }

    /// The init segment's address.
    pub fn map(&self) -> Option<&str> {
        self.lines.iter().find_map(|l| if let Line::Map(u) = l { Some(u.as_str()) } else { None })
    }

    /// The segments' addresses and lengths, in order.
    pub fn segments(&self) -> impl Iterator<Item = (&str, f64)> {
        self.lines.iter().filter_map(|l| if let Line::Segment { uri, seconds } = l { Some((uri.as_str(), *seconds)) } else { None })
    }

    /// The playlist with each address `local` has a file for replaced by
    /// that file (absolute paths, which ffmpeg reads among the addresses
    /// once `file` is in its protocol whitelist).
    pub fn with_local(&self, local: impl Fn(&str) -> Option<String>) -> String {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                Line::Tag(t) => out.push_str(t),
                Line::Map(u) => {
                    let u = local(u).unwrap_or_else(|| u.clone());
                    out.push_str(&format!("#EXT-X-MAP:URI=\"{u}\""));
                }
                Line::Segment { uri, seconds } => {
                    out.push_str(&format!("#EXTINF:{seconds},\n"));
                    out.push_str(&local(uri).unwrap_or_else(|| uri.clone()));
                }
            }
            out.push('\n');
        }
        out
    }
}

/// `uri` against the playlist at `base`.
fn absolute(base: &Url, uri: &str) -> Option<String> {
    base.join(uri.trim()).ok().map(String::from)
}

/// The value of `key` in an attribute list (`A=1,B="x,y"`).
fn attribute<'a>(list: &'a str, key: &str) -> Option<&'a str> {
    let mut rest = list;
    while !rest.is_empty() {
        let (name, after) = rest.split_once('=')?;
        let (value, next) = match after.strip_prefix('"') {
            Some(quoted) => {
                let end = quoted.find('"')?;
                (&quoted[..end], quoted[end + 1..].trim_start_matches(','))
            }
            None => match after.split_once(',') {
                Some((v, n)) => (v, n),
                None => (after, ""),
            },
        };
        if name.trim().eq_ignore_ascii_case(key) {
            return Some(value);
        }
        rest = next;
    }
    None
}

/// Reads a master playlist fetched from `url`.
pub fn parse_master(url: &str, text: &str) -> Master {
    let Ok(base) = Url::parse(url) else { return Master::default() };
    let mut master = Master::default();
    let mut pending: Option<(u32, u32, u64)> = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(attrs) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            let (w, h) = attribute(attrs, "RESOLUTION")
                .and_then(|r| r.split_once('x'))
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                .unwrap_or((0, 0));
            pending = Some((w, h, attribute(attrs, "BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(0)));
        } else if let Some(attrs) = line.strip_prefix("#EXT-X-MEDIA:") {
            let audio = attribute(attrs, "TYPE").is_some_and(|t| t.eq_ignore_ascii_case("AUDIO"));
            let default = attribute(attrs, "DEFAULT").is_none_or(|d| d.eq_ignore_ascii_case("YES"));
            if audio && (default || master.audio.is_none()) {
                if let Some(uri) = attribute(attrs, "URI").and_then(|u| absolute(&base, u)) {
                    master.audio = Some(uri);
                }
            }
        } else if !line.starts_with('#') {
            if let (Some((width, height, bandwidth)), Some(uri)) = (pending.take(), absolute(&base, line)) {
                master.variants.push(Variant { uri, width, height, bandwidth });
            }
        }
    }
    master
}

/// Reads a media playlist fetched from `url`.
pub fn parse_media(url: &str, text: &str) -> Media {
    let Ok(base) = Url::parse(url) else { return Media::default() };
    let mut lines = Vec::new();
    let mut seconds: Option<f64> = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            seconds = rest.split(',').next().and_then(|s| s.trim().parse().ok());
        } else if let Some(attrs) = line.strip_prefix("#EXT-X-MAP:") {
            match attribute(attrs, "URI").and_then(|u| absolute(&base, u)) {
                Some(uri) => lines.push(Line::Map(uri)),
                None => lines.push(Line::Tag(line.to_owned())),
            }
        } else if line.starts_with('#') {
            lines.push(Line::Tag(line.to_owned()));
        } else if let Some(uri) = absolute(&base, line) {
            lines.push(Line::Segment { uri, seconds: seconds.take().unwrap_or(0.) });
        }
    }
    Media { lines }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: &str = "#EXTM3U
#EXT-X-VERSION:7
#EXT-X-INDEPENDENT-SEGMENTS
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"Default\",AUTOSELECT=YES,DEFAULT=YES,URI=\"hls_264_4_audio.m3u8\"
#EXT-X-STREAM-INF:BANDWIDTH=5800000,CODECS=\"avc1.640029,mp4a.40.2\",RESOLUTION=1920x1080,FRAME-RATE=30,AUDIO=\"audio\"
hls_264_0_video.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2600000,CODECS=\"avc1.640029,mp4a.40.2\",RESOLUTION=1280x720,FRAME-RATE=30,AUDIO=\"audio\"
hls_264_1_video.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1400000,CODECS=\"avc1.640029,mp4a.40.2\",RESOLUTION=854x480,FRAME-RATE=30,AUDIO=\"audio\"
hls_264_2_video.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1000000,CODECS=\"avc1.640029,mp4a.40.2\",RESOLUTION=640x360,FRAME-RATE=30,AUDIO=\"audio\"
hls_264_3_video.m3u8
";

    const MEDIA: &str = "#EXTM3U
#EXT-X-VERSION:7
#EXT-X-TARGETDURATION:3
#EXT-X-MAP:URI=\"dash_h264/init-stream2.m4s\"
#EXTINF:3
dash_h264/chunk-stream2-00001.m4s
#EXTINF:3,
dash_h264/chunk-stream2-00002.m4s
#EXTINF:1.5
dash_h264/chunk-stream2-00003.m4s
#EXT-X-ENDLIST
";

    const BASE: &str = "https://video.example.com/store_trailers/1/2/abc/17/";

    #[test]
    fn the_master_gives_its_sizes_and_the_audio() {
        let m = parse_master(&format!("{BASE}hls_264_master.m3u8?t=1"), MASTER);
        assert_eq!(m.variants.len(), 4);
        assert_eq!(m.audio.as_deref(), Some(&*format!("{BASE}hls_264_4_audio.m3u8")));
        let v = m.pick(480).unwrap();
        assert_eq!((v.width, v.height), (854, 480));
        assert_eq!(v.uri, format!("{BASE}hls_264_2_video.m3u8"));
        assert_eq!(m.pick(200).unwrap().height, 360, "nothing small enough: the smallest");
        assert_eq!(m.pick(4000).unwrap().height, 1080);
    }

    #[test]
    fn attributes_keep_quoted_commas() {
        let list = "BANDWIDTH=1,CODECS=\"avc1.640029,mp4a.40.2\",RESOLUTION=640x360";
        assert_eq!(attribute(list, "CODECS"), Some("avc1.640029,mp4a.40.2"));
        assert_eq!(attribute(list, "RESOLUTION"), Some("640x360"));
        assert_eq!(attribute(list, "AUDIO"), None);
    }

    #[test]
    fn a_media_playlist_reads_its_segments() {
        let m = parse_media(&format!("{BASE}hls_264_2_video.m3u8"), MEDIA);
        assert_eq!(m.map(), Some(&*format!("{BASE}dash_h264/init-stream2.m4s")));
        let segs: Vec<_> = m.segments().collect();
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].0, format!("{BASE}dash_h264/chunk-stream2-00001.m4s"));
        assert_eq!(m.seconds(), 7.5);
    }

    #[test]
    fn local_files_replace_their_segments() {
        let m = parse_media(&format!("{BASE}v.m3u8"), MEDIA);
        let text = m.with_local(|u| u.ends_with("00001.m4s").then(|| "/cache/v-1.m4s".to_owned()));
        assert!(text.contains("\n/cache/v-1.m4s\n"));
        assert!(text.contains(&format!("\n{BASE}dash_h264/chunk-stream2-00002.m4s\n")));
        assert!(text.contains(&format!("#EXT-X-MAP:URI=\"{BASE}dash_h264/init-stream2.m4s\"")));
        assert!(text.trim_end().ends_with("#EXT-X-ENDLIST"));
        let back = parse_media(&format!("{BASE}v.m3u8"), &m.with_local(|_| None));
        assert_eq!(back, m, "without local files it reads as it was");
    }
}
