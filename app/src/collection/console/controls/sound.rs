//! Console mode's UI sounds (`controls.sounds`): four short tones written
//! once as WAV files in the cache folder and played by whatever player the
//! system has (`pw-play`, `paplay`, `aplay`). A worker thread does all of it
//! (finding the player, writing the files, starting the process), so a press
//! never waits; the thread lives as long as the [`Player`] does.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use crate::console::Pad;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Sound {
    Move,
    Confirm,
    Back,
    Tab,
}

impl Sound {
    const ALL: [Sound; 4] = [Sound::Move, Sound::Confirm, Sound::Back, Sound::Tab];

    fn file(self) -> &'static str {
        match self {
            Sound::Move => "move.wav",
            Sound::Confirm => "confirm.wav",
            Sound::Back => "back.wav",
            Sound::Tab => "tab.wav",
        }
    }

    /// Tones as (Hz, ms), played one after the other, and the volume (0..=1).
    fn tones(self) -> (&'static [(f32, f32)], f32) {
        match self {
            Sound::Move => (&[(1150., 26.)], 0.22),
            Sound::Confirm => (&[(660., 45.), (990., 80.)], 0.3),
            Sound::Back => (&[(720., 45.), (480., 80.)], 0.3),
            Sound::Tab => (&[(880., 34.), (1180., 52.)], 0.26),
        }
    }

    /// The least time between two plays: a held direction repeats every
    /// ~85 ms, which would be a buzz.
    fn gap(self) -> Duration {
        match self {
            Sound::Move => Duration::from_millis(110),
            _ => Duration::from_millis(40),
        }
    }
}

/// What a press sounds like (`None`: silent, like the right stick).
pub(super) fn for_pad(pad: Pad) -> Option<Sound> {
    match pad {
        Pad::Up | Pad::Down | Pad::Left | Pad::Right => Some(Sound::Move),
        Pad::A | Pad::X | Pad::Y | Pad::Start | Pad::Select | Pad::LT | Pad::RT => Some(Sound::Confirm),
        Pad::B => Some(Sound::Back),
        Pad::LB | Pad::RB => Some(Sound::Tab),
        Pad::ScrollUp | Pad::ScrollDown => None,
    }
}

const RATE: u32 = 22_050;

/// A mono 16-bit WAV of sine tones with a soft start and a fading end.
fn wav(tones: &[(f32, f32)], volume: f32) -> Vec<u8> {
    let mut samples: Vec<i16> = Vec::new();
    for &(hz, ms) in tones {
        let n = (RATE as f32 * ms / 1000.) as usize;
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            let progress = i as f32 / n as f32;
            let attack = (t / 0.004).min(1.);
            let envelope = attack * (1. - progress).powi(2);
            let v = (t * hz * std::f32::consts::TAU).sin() * envelope * volume;
            samples.push((v * f32::from(i16::MAX)) as i16);
        }
    }
    let data = samples.len() as u32 * 2;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// The first player found on `path`, with the arguments that make it quiet
/// and play one file.
fn pick_player(path: &OsStr, exists: impl Fn(&Path) -> bool) -> Option<(PathBuf, &'static [&'static str])> {
    const PLAYERS: [(&str, &[&str]); 3] = [("pw-play", &[]), ("paplay", &[]), ("aplay", &["-q"])];
    PLAYERS.iter().find_map(|(name, args)| std::env::split_paths(path).map(|d| d.join(name)).find(|p| exists(p)).map(|p| (p, *args)))
}

/// Plays the sounds from a worker thread; dropping it stops the thread.
pub(super) struct Player {
    tx: Sender<Sound>,
    last: [Option<Instant>; 4],
}

impl Player {
    /// `dir` is where the WAV files are kept (written on first use).
    pub(super) fn new(dir: PathBuf) -> Self {
        let (tx, rx) = channel();
        if let Err(e) = std::thread::Builder::new().name("console-sounds".into()).spawn(move || work(&dir, rx)) {
            log::warn!("console sounds: {e}");
        }
        Self { tx, last: [None; 4] }
    }

    pub(super) fn play(&mut self, sound: Sound) {
        let now = Instant::now();
        let last = &mut self.last[sound as usize];
        if last.is_some_and(|t| now.duration_since(t) < sound.gap()) {
            return;
        }
        *last = Some(now);
        self.tx.send(sound).ok();
    }
}

fn work(dir: &Path, rx: Receiver<Sound>) {
    let Some((player, args)) = std::env::var_os("PATH").and_then(|p| pick_player(&p, |f| f.is_file())) else {
        log::info!("console sounds: no pw-play, paplay or aplay found");
        return;
    };
    if let Err(e) = write_files(dir) {
        log::warn!("console sounds: {e}");
        return;
    }
    let mut running: Vec<Child> = Vec::new();
    loop {
        // While a sound plays, wake up to reap it (no zombies left behind).
        let sound = if running.is_empty() {
            match rx.recv() {
                Ok(s) => Some(s),
                Err(_) => break,
            }
        } else {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(s) => Some(s),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        };
        running.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
        let Some(sound) = sound else { continue };
        // A burst never piles up processes.
        if running.len() >= 4 {
            continue;
        }
        match Command::new(&player)
            .args(args)
            .arg(dir.join(sound.file()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => running.push(child),
            Err(e) => log::debug!("console sounds: {e}"),
        }
    }
    for mut c in running {
        c.wait().ok();
    }
}

fn write_files(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for s in Sound::ALL {
        let file = dir.join(s.file());
        if !file.is_file() {
            let (tones, volume) = s.tones();
            std::fs::write(file, wav(tones, volume))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_has_a_valid_header_and_the_right_length() {
        let bytes = wav(&[(440., 100.)], 0.5);
        let samples = (RATE as f32 * 0.1) as usize;
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(bytes.len(), 44 + samples * 2);
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize, bytes.len() - 8);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize, samples * 2);
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), RATE);
    }

    #[test]
    fn wav_fades_out_and_stays_below_the_volume() {
        let bytes = wav(&[(440., 100.)], 0.5);
        let sample = |i: usize| i16::from_le_bytes([bytes[44 + i * 2], bytes[45 + i * 2]]);
        let n = (bytes.len() - 44) / 2;
        let peak = (0..n).map(|i| sample(i).unsigned_abs()).max().unwrap();
        assert!(peak > 2000 && f32::from(peak) <= 0.5 * f32::from(i16::MAX));
        assert!(sample(n - 1).unsigned_abs() < 50);
    }

    #[test]
    fn presses_map_to_sounds() {
        assert_eq!(for_pad(Pad::Down), Some(Sound::Move));
        assert_eq!(for_pad(Pad::A), Some(Sound::Confirm));
        assert_eq!(for_pad(Pad::B), Some(Sound::Back));
        assert_eq!(for_pad(Pad::RB), Some(Sound::Tab));
        assert_eq!(for_pad(Pad::ScrollDown), None);
    }

    #[test]
    fn the_first_player_on_the_path_wins() {
        let path = std::env::join_paths(["/a", "/b"]).unwrap();
        let has = |p: &Path| p == Path::new("/b/aplay") || p == Path::new("/a/paplay");
        assert_eq!(pick_player(&path, has), Some((PathBuf::from("/a/paplay"), &[][..])));
        assert_eq!(pick_player(&path, |p| p == Path::new("/b/aplay")), Some((PathBuf::from("/b/aplay"), &["-q"][..])));
        assert_eq!(pick_player(&path, |_| false), None);
    }
}
