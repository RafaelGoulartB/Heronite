//! Console mode's UI sounds (`controls.sounds`): four short tones written as
//! WAV files in the cache folder and played by whatever player the system has
//! (`pw-play`, `paplay`, `aplay`). A worker thread does all of it (finding the
//! player, writing the files, starting the process), so a press never waits;
//! the thread lives as long as the [`Player`] does.
//!
//! The tones depend on the style and the volume (`controls.soundStyle`,
//! `controls.soundVolume`). The files are named after them and after
//! [`VERSION`], so a change never plays a stale file and one nobody asked for
//! is never written.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use heronite_data::console_theme::{ControlOptions, SoundStyle, SOUND_VOLUME};

use super::Pad;

/// Part of every file name: bumped when the tones change, so the old files
/// are not played (and are deleted).
const VERSION: u32 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sound {
    Move,
    Confirm,
    Back,
    Tab,
}

impl Sound {
    const ALL: [Sound; 4] = [Sound::Move, Sound::Confirm, Sound::Back, Sound::Tab];

    fn name(self) -> &'static str {
        match self {
            Sound::Move => "move",
            Sound::Confirm => "confirm",
            Sound::Back => "back",
            Sound::Tab => "tab",
        }
    }

    /// The tones as (Hz, ms), played one after the other, and the level
    /// (0..=1, at full volume).
    fn tones(self, style: SoundStyle) -> (&'static [(f32, f32)], f32) {
        match (style, self) {
            (SoundStyle::Classic, Sound::Move) => (&[(1150., 26.)], 0.22),
            (SoundStyle::Classic, Sound::Confirm) => (&[(660., 45.), (990., 80.)], 0.3),
            (SoundStyle::Classic, Sound::Back) => (&[(720., 45.), (480., 80.)], 0.3),
            (SoundStyle::Classic, Sound::Tab) => (&[(880., 34.), (1180., 52.)], 0.26),
            // Low and round: a tick that dips a little, then rising (G4 to D5)
            // and falling (A4 to E4) fifths for yes and back.
            (SoundStyle::Soft, Sound::Move) => (&[(520., 60.)], 0.26),
            (SoundStyle::Soft, Sound::Confirm) => (&[(392., 50.), (587., 110.)], 0.34),
            (SoundStyle::Soft, Sound::Back) => (&[(440., 50.), (330., 100.)], 0.34),
            (SoundStyle::Soft, Sound::Tab) => (&[(494., 38.), (587., 64.)], 0.3),
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
pub fn for_pad(pad: Pad) -> Option<Sound> {
    match pad {
        Pad::Up | Pad::Down | Pad::Left | Pad::Right => Some(Sound::Move),
        Pad::A | Pad::X | Pad::Y | Pad::Start | Pad::Select | Pad::LT | Pad::RT => Some(Sound::Confirm),
        Pad::B => Some(Sound::Back),
        Pad::LB | Pad::RB => Some(Sound::Tab),
        Pad::ScrollUp | Pad::ScrollDown => None,
    }
}

const RATE: u32 = 22_050;

/// How a tone is shaped.
struct Voice {
    /// Seconds to reach full level (a raised cosine).
    attack: f32,
    /// The fade-out is `(1 - progress)` to this power.
    decay: f32,
    /// Level of the octave above, which rounds the tone (0 = a pure sine).
    harmonic: f32,
    /// Where the pitch ends, as a part of where it starts.
    glide: f32,
}

impl Voice {
    fn of(style: SoundStyle) -> Self {
        match style {
            SoundStyle::Classic => Voice { attack: 0.004, decay: 2., harmonic: 0., glide: 1. },
            SoundStyle::Soft => Voice { attack: 0.008, decay: 2.5, harmonic: 0.3, glide: 0.97 },
        }
    }
}

/// What the sounds are played with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings {
    pub style: SoundStyle,
    /// Percent, [`SOUND_VOLUME`].
    pub volume: u8,
}

impl Settings {
    pub fn of(c: &ControlOptions) -> Self {
        Self { style: c.sound_style, volume: c.sound_volume.clamp(SOUND_VOLUME.0, SOUND_VOLUME.1) }
    }

    /// The factor on a sound's level: steeper than linear, as loudness is.
    fn gain(self) -> f32 {
        (f32::from(self.volume) / 100.).powf(1.5)
    }

    fn file(self, sound: Sound) -> String {
        format!("v{VERSION}-{}-{}-{}.wav", self.style.as_str(), self.volume, sound.name())
    }
}

/// A mono 16-bit WAV of the tones, none louder than `level` (0..=1).
fn wav(tones: &[(f32, f32)], level: f32, voice: &Voice) -> Vec<u8> {
    let mut samples: Vec<i16> = Vec::new();
    for &(hz, ms) in tones {
        let n = (RATE as f32 * ms / 1000.) as usize;
        let mut phase = 0f32;
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            let progress = i as f32 / n as f32;
            let attack = 0.5 - 0.5 * (std::f32::consts::PI * (t / voice.attack).min(1.)).cos();
            let envelope = attack * (1. - progress).powf(voice.decay);
            let now = hz * (1. + (voice.glide - 1.) * progress);
            phase += std::f32::consts::TAU * now / RATE as f32;
            let tone = (phase.sin() + voice.harmonic * (2. * phase).sin()) / (1. + voice.harmonic);
            samples.push((tone * envelope * level * f32::from(i16::MAX)) as i16);
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
pub struct Player {
    tx: Sender<(Sound, Settings)>,
    last: [Option<Instant>; 4],
}

impl Player {
    /// `dir` is where the WAV files are kept (written when first needed).
    pub fn new(dir: PathBuf) -> Self {
        let (tx, rx) = channel();
        if let Err(e) = std::thread::Builder::new().name("console-sounds".into()).spawn(move || work(&dir, rx)) {
            log::warn!("console sounds: {e}");
        }
        Self { tx, last: [None; 4] }
    }

    pub fn play(&mut self, sound: Sound, settings: Settings) {
        let now = Instant::now();
        let last = &mut self.last[sound as usize];
        if last.is_some_and(|t| now.duration_since(t) < sound.gap()) {
            return;
        }
        *last = Some(now);
        self.tx.send((sound, settings)).ok();
        *LAST_SENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(now);
    }
}

/// When any player last sent a sound, for [`preview`].
static LAST_SENT: Mutex<Option<Instant>> = Mutex::new(None);

/// A sound this recent comes from the press that made the change.
const PRESS_WINDOW: Duration = Duration::from_millis(60);

static PREVIEW: Mutex<Option<Player>> = Mutex::new(None);

/// Plays `sound` as `settings` make it, for Settings to let the user hear a
/// change. Console mode's own player already sounds for the press that made
/// it: then this stays silent (the next press has the new sound).
pub fn preview(dir: &Path, sound: Sound, settings: Settings) {
    let pressed = LAST_SENT.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| t.elapsed() < PRESS_WINDOW);
    if pressed {
        return;
    }
    let mut p = PREVIEW.lock().unwrap_or_else(|e| e.into_inner());
    p.get_or_insert_with(|| Player::new(dir.to_path_buf())).play(sound, settings);
}

fn work(dir: &Path, rx: Receiver<(Sound, Settings)>) {
    let Some((player, args)) = std::env::var_os("PATH").and_then(|p| pick_player(&p, |f| f.is_file())) else {
        log::info!("console sounds: no pw-play, paplay or aplay found");
        return;
    };
    if let Err(e) = remove_stale(dir) {
        log::warn!("console sounds: {e}");
    }
    let mut ready: Option<Settings> = None;
    let mut running: Vec<Child> = Vec::new();
    loop {
        // While a sound plays, wake up to reap it (no zombies left behind).
        let next = if running.is_empty() {
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
        let Some((sound, settings)) = next else { continue };
        // A burst never piles up processes.
        if running.len() >= 4 {
            continue;
        }
        if ready != Some(settings) {
            if let Err(e) = write_files(dir, settings) {
                log::warn!("console sounds: {e}");
                continue;
            }
            ready = Some(settings);
        }
        match Command::new(&player)
            .args(args)
            .arg(dir.join(settings.file(sound)))
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

/// Writes the four files of `settings` that are not there yet, or are not
/// whole (a crash can cut one short). Each goes to a temporary name first and
/// is renamed into place, so a player never reads half a file.
fn write_files(dir: &Path, settings: Settings) -> std::io::Result<()> {
    static TEMPS: AtomicU32 = AtomicU32::new(0);
    std::fs::create_dir_all(dir)?;
    for s in Sound::ALL {
        let file = dir.join(settings.file(s));
        let (tones, level) = s.tones(settings.style);
        let bytes = wav(tones, level * settings.gain(), &Voice::of(settings.style));
        if std::fs::metadata(&file).is_ok_and(|m| m.is_file() && m.len() == bytes.len() as u64) {
            continue;
        }
        let tmp = dir.join(format!("{}.{}-{}.tmp", settings.file(s), std::process::id(), TEMPS.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &file).inspect_err(|_| {
            std::fs::remove_file(&tmp).ok();
        })?;
    }
    Ok(())
}

/// Deletes the files of another [`VERSION`] (and of the first version, which
/// had no number in its names), and temporary files a crash left.
fn remove_stale(dir: &Path) -> std::io::Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(()) };
    let keep = format!("v{VERSION}-");
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".tmp") || (name.ends_with(".wav") && !name.starts_with(&keep)) {
            std::fs::remove_file(e.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classic() -> Voice {
        Voice::of(SoundStyle::Classic)
    }

    fn peak(bytes: &[u8]) -> u16 {
        bytes[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap()
    }

    fn make(style: SoundStyle, volume: u8, sound: Sound) -> Vec<u8> {
        let settings = Settings { style, volume };
        let (tones, level) = sound.tones(style);
        wav(tones, level * settings.gain(), &Voice::of(style))
    }

    #[test]
    fn wav_has_a_valid_header_and_the_right_length() {
        let bytes = wav(&[(440., 100.)], 0.5, &classic());
        let samples = (RATE as f32 * 0.1) as usize;
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(bytes.len(), 44 + samples * 2);
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize, bytes.len() - 8);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize, samples * 2);
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), RATE);
    }

    #[test]
    fn every_sound_fades_out_and_stays_below_its_level() {
        for style in SoundStyle::ALL {
            for sound in Sound::ALL {
                let (tones, level) = sound.tones(*style);
                let bytes = make(*style, 100, sound);
                let n = (bytes.len() - 44) / 2;
                let last = i16::from_le_bytes([bytes[44 + (n - 1) * 2], bytes[45 + (n - 1) * 2]]);
                let peak = f32::from(peak(&bytes));
                assert!(peak > 0.5 * level * f32::from(i16::MAX), "{style:?} {sound:?} is too quiet: {peak}");
                assert!(peak <= level * f32::from(i16::MAX), "{style:?} {sound:?} is above its level: {peak}");
                assert!(last.unsigned_abs() < 50, "{style:?} {sound:?} ends with a click");
                let first = i16::from_le_bytes([bytes[44], bytes[45]]);
                assert!(first.unsigned_abs() < 50, "{style:?} {sound:?} starts with a click");
                assert!(!tones.is_empty());
            }
        }
    }

    #[test]
    fn classic_keeps_the_first_tones() {
        assert_eq!(Sound::Move.tones(SoundStyle::Classic), (&[(1150., 26.)][..], 0.22));
        assert_eq!(Sound::Confirm.tones(SoundStyle::Classic), (&[(660., 45.), (990., 80.)][..], 0.3));
        assert_eq!(Sound::Back.tones(SoundStyle::Classic), (&[(720., 45.), (480., 80.)][..], 0.3));
        assert_eq!(Sound::Tab.tones(SoundStyle::Classic), (&[(880., 34.), (1180., 52.)][..], 0.26));
    }

    #[test]
    fn soft_is_lower_and_quieter_than_classic() {
        let top = |style| Sound::ALL.iter().flat_map(|s| s.tones(style).0.iter().map(|t| t.0)).fold(0f32, f32::max);
        assert!(top(SoundStyle::Soft) <= 600. && top(SoundStyle::Classic) > 1000.);
        for sound in Sound::ALL {
            assert!(sound.tones(SoundStyle::Soft).1 <= 0.35);
        }
        assert!(peak(&make(SoundStyle::Soft, 100, Sound::Move)) > 0);
    }

    #[test]
    fn the_volume_scales_the_sound() {
        let at = |v| peak(&make(SoundStyle::Soft, v, Sound::Confirm));
        let (low, mid, full) = (at(SOUND_VOLUME.0), at(50), at(SOUND_VOLUME.1));
        assert!(0 < low && low < mid && mid < full, "{low} {mid} {full}");
        assert!(f32::from(low) < 0.1 * f32::from(full));
    }

    #[test]
    fn files_are_named_after_the_style_the_volume_and_the_version() {
        let soft = Settings { style: SoundStyle::Soft, volume: 100 };
        let names = [
            soft.file(Sound::Move),
            Settings { volume: 60, ..soft }.file(Sound::Move),
            Settings { style: SoundStyle::Classic, ..soft }.file(Sound::Move),
            soft.file(Sound::Tab),
        ];
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
        assert!(names.iter().all(|n| n.starts_with(&format!("v{VERSION}-")) && n.ends_with(".wav")));
    }

    #[test]
    fn the_settings_come_from_the_options_and_stay_in_range() {
        let mut c = ControlOptions::default();
        assert_eq!(Settings::of(&c), Settings { style: SoundStyle::Soft, volume: 100 });
        c.sound_volume = 0;
        assert_eq!(Settings::of(&c).volume, SOUND_VOLUME.0);
        c.sound_volume = 250;
        assert_eq!(Settings::of(&c).volume, SOUND_VOLUME.1);
    }

    #[test]
    fn each_setting_gets_its_own_files_and_old_ones_are_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let soft = Settings { style: SoundStyle::Soft, volume: 70 };
        let classic = Settings { style: SoundStyle::Classic, volume: 70 };
        write_files(dir.path(), soft).unwrap();
        write_files(dir.path(), classic).unwrap();
        for s in Sound::ALL {
            let a = std::fs::read(dir.path().join(soft.file(s))).unwrap();
            let b = std::fs::read(dir.path().join(classic.file(s))).unwrap();
            assert_ne!(a, b);
        }
        // A file cut short is written again, whole.
        let cut = dir.path().join(soft.file(Sound::Move));
        let whole = std::fs::read(&cut).unwrap();
        std::fs::write(&cut, &whole[..whole.len() / 2]).unwrap();
        write_files(dir.path(), soft).unwrap();
        assert_eq!(std::fs::read(&cut).unwrap(), whole);
        // What the first version wrote, a file of another version, a temporary file.
        std::fs::write(dir.path().join("v1-soft-70-move.wav.1-0.tmp"), b"x").unwrap();
        std::fs::write(dir.path().join("move.wav"), b"x").unwrap();
        std::fs::write(dir.path().join("v0-soft-70-move.wav"), b"x").unwrap();
        remove_stale(dir.path()).unwrap();
        let mut left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left.len(), 8);
        assert!(left.iter().all(|n| n.starts_with(&format!("v{VERSION}-"))));
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
