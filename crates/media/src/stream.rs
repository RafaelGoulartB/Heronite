//! Plays a video into frames for the UI: ffmpeg reads it (an HLS playlist
//! on the web or on this PC, a file) at its own pace (`-re`), scales each
//! picture into a fixed box and writes it as raw BGRA on its stdout; the
//! sound, when asked for, goes straight to the desktop's audio server
//! (PulseAudio, or PipeWire's pulse server) from the same process, so
//! picture and sound keep together. Used by the store trailers of the focus
//! panel: nothing is written to disk here.
//!
//! The frames come on the thread that reads ffmpeg's output, which also
//! started it (the process ends with that thread, `heronite_data::proc`).
//! [`Stream::stop`] (or dropping the [`Stream`]) ends ffmpeg at once.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use heronite_data::proc::{guard, spawn};
use heronite_images::Decoded;

/// What to play.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamSpec {
    /// The picture: an address or a path.
    pub video: String,
    /// The sound when it is apart from the picture (HLS audio playlist).
    pub audio: Option<String>,
    /// Where to start, in seconds.
    pub start: f64,
    /// Size of every frame; the picture is fitted inside, black bars around.
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Play the sound (only where [`sound_supported`]).
    pub sound: bool,
}

#[derive(Debug)]
pub enum StreamEvent {
    Frame(Decoded),
    /// The video played to its end.
    Ended,
    /// ffmpeg could not play it (missing, a network error...).
    Failed(String),
}

/// A video being played; ffmpeg is ended when it is dropped.
pub struct Stream {
    child: Arc<Mutex<Option<Child>>>,
    stopped: Arc<AtomicBool>,
}

impl Stream {
    /// Starts playing `spec`, calling `on` from a thread of its own for each
    /// frame, then once with [`StreamEvent::Ended`] or
    /// [`StreamEvent::Failed`] (never after [`stop`](Self::stop)).
    pub fn start(spec: StreamSpec, mut on: impl FnMut(StreamEvent) + Send + 'static) -> Stream {
        let child: Arc<Mutex<Option<Child>>> = Arc::default();
        let stopped = Arc::new(AtomicBool::new(false));
        let (slot, stop) = (child.clone(), stopped.clone());
        let spawned = std::thread::Builder::new().name("video-stream".into()).spawn(move || {
            let end = run(&spec, &slot, &stop, &mut on);
            if !stop.load(Ordering::SeqCst) {
                on(end);
            }
        });
        if let Err(e) = spawned {
            log::warn!("video stream thread: {e}");
        }
        Stream { child, stopped }
    }

    /// Ends ffmpeg; no event comes after this.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(mut c) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Whether ffmpeg can play sound here: Linux with an ffmpeg built with the
/// `pulse` output (asked once). Elsewhere trailers play without sound.
pub fn sound_supported() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    *FOUND.get_or_init(|| {
        if !cfg!(target_os = "linux") || !crate::posters::available() {
            return false;
        }
        let out = heronite_data::proc::Run::new()
            .timeout(std::time::Duration::from_secs(5))
            .run(Command::new("ffmpeg").args(["-hide_banner", "-devices"]));
        out.is_ok_and(|o| o.stdout_text().lines().any(|l| l.split_whitespace().nth(1) == Some("pulse") && l.trim_start().contains('E')))
    })
}

/// ffmpeg's arguments for `spec`.
fn args(spec: &StreamSpec) -> Vec<String> {
    let mut a: Vec<String> = ["-nostdin", "-hide_banner", "-loglevel", "error"].map(String::from).to_vec();
    // A local playlist names files and web addresses side by side.
    let input = |a: &mut Vec<String>, src: &str| {
        a.extend(["-protocol_whitelist", "file,http,https,tcp,tls,crypto", "-re"].map(String::from));
        if spec.start > 0. {
            a.extend(["-ss".into(), format!("{:.3}", spec.start)]);
        }
        a.extend(["-i".into(), src.to_owned()]);
    };
    input(&mut a, &spec.video);
    let sound = spec.sound && sound_supported();
    let audio_input = match (&spec.audio, sound) {
        (Some(audio), true) => {
            input(&mut a, audio);
            Some("1:a:0")
        }
        (None, true) => Some("0:a:0?"),
        _ => None,
    };
    let (w, h) = (spec.width, spec.height);
    a.extend(["-map".into(), "0:v:0".into(), "-an".into(), "-sn".into()]);
    a.extend([
        "-vf".into(),
        format!("fps={},scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:-1:-1:black", spec.fps.max(1)),
        "-pix_fmt".into(),
        "bgra".into(),
        "-f".into(),
        "rawvideo".into(),
        "pipe:1".into(),
    ]);
    if let Some(map) = audio_input {
        a.extend([
            "-map".into(),
            map.into(),
            "-vn".into(),
            "-f".into(),
            "pulse".into(),
            "-name".into(),
            "Heronite".into(),
            "Trailer".into(),
        ]);
    }
    a
}

fn run(spec: &StreamSpec, slot: &Mutex<Option<Child>>, stop: &AtomicBool, on: &mut dyn FnMut(StreamEvent)) -> StreamEvent {
    if !crate::posters::available() {
        return StreamEvent::Failed("ffmpeg is not installed".into());
    }
    let mut cmd = Command::new("ffmpeg");
    cmd.args(args(spec)).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    guard(&mut cmd);
    let mut child = match spawn(&mut cmd) {
        Ok(c) => c,
        Err(e) => return StreamEvent::Failed(format!("ffmpeg: {e}")),
    };
    let (Some(mut out), Some(mut err)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        return StreamEvent::Failed("ffmpeg: no output".into());
    };
    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
    if stop.load(Ordering::SeqCst) {
        // Stopped while it started: `stop` found the slot empty.
        if let Some(mut c) = slot.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        return StreamEvent::Ended;
    }
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = err.read_to_string(&mut text);
        text
    });
    let frame_len = spec.width as usize * spec.height as usize * 4;
    let mut frames = 0u64;
    loop {
        let mut bgra = vec![0u8; frame_len];
        if out.read_exact(&mut bgra).is_err() {
            break;
        }
        frames += 1;
        on(StreamEvent::Frame(Decoded { width: spec.width, height: spec.height, bgra }));
    }
    let status = slot.lock().unwrap_or_else(|e| e.into_inner()).take().and_then(|mut c| c.wait().ok());
    let errors = errors.join().unwrap_or_default();
    match status {
        _ if stop.load(Ordering::SeqCst) => StreamEvent::Ended,
        Some(s) if s.success() || frames > 0 => StreamEvent::Ended,
        _ => {
            let why = errors.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("ffmpeg stopped").trim().to_owned();
            log::warn!("video stream of {}: {errors}", spec.video);
            StreamEvent::Failed(why)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> StreamSpec {
        StreamSpec {
            video: "/v/video.m3u8".into(),
            audio: Some("/v/audio.m3u8".into()),
            start: 0.,
            width: 854,
            height: 480,
            fps: 30,
            sound: false,
        }
    }

    #[test]
    fn without_sound_only_the_picture_is_read() {
        let a = args(&spec());
        assert_eq!(a.iter().filter(|x| *x == "-i").count(), 1);
        assert!(!a.contains(&"pulse".to_string()));
        assert!(!a.contains(&"-ss".to_string()));
        assert!(a.iter().any(|x| x.starts_with("fps=30,scale=854:480:")));
        assert_eq!(a.last().map(String::as_str), Some("pipe:1"));
    }

    #[test]
    fn a_later_start_seeks_each_input() {
        let a = args(&StreamSpec { start: 12.5, ..spec() });
        let at = a.iter().position(|x| x == "-ss").unwrap();
        assert_eq!(a[at + 1], "12.500");
        assert!(at < a.iter().position(|x| x == "-i").unwrap(), "seeking the input, not decoding up to it");
    }

    #[test]
    fn a_test_pattern_plays_into_frames() {
        if !crate::posters::available() {
            return;
        }
        let spec = StreamSpec {
            video: "testsrc=duration=1:size=320x240:rate=10".into(),
            audio: None,
            start: 0.,
            width: 64,
            height: 36,
            fps: 10,
            sound: false,
        };
        // `lavfi` is a device, not a file: play it through a tiny wrapper.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("t.mp4");
        let made = Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-f", "lavfi", "-i", &spec.video, "-pix_fmt", "yuv420p"])
            .arg(&file)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let _s = Stream::start(StreamSpec { video: file.to_string_lossy().into_owned(), ..spec }, move |e| {
            let _ = tx.send(e);
        });
        let mut frames = 0;
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(20)).expect("an event") {
                StreamEvent::Frame(f) => {
                    assert_eq!((f.width, f.height, f.bgra.len()), (64, 36, 64 * 36 * 4));
                    frames += 1;
                }
                StreamEvent::Ended => break,
                StreamEvent::Failed(e) => panic!("{e}"),
            }
        }
        assert!((8..=12).contains(&frames), "{frames} frames for one second at 10 fps");
    }
}
