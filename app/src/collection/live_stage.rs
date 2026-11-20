//! Animated backgrounds: with Settings → Appearance → "Animated backgrounds"
//! on, a background that is a video or an animated picture (the game's own,
//! "Your own picture", the one with no game selected) plays behind the
//! Collection. Off, or while it cannot play, its first frame shows still
//! like any picture (`heronite_images::Opts::video_still`).
//!
//! Kept cheap (about 6% of one core while it plays, measured): ffmpeg
//! (`heronite_media::stream`) decodes it 24 times a second into a raster of
//! at most 640 px wide (`heronite_images::stage_frame_size`) and gives it the
//! still stage's blur and colour itself (`gblur`, `eq`: several times faster
//! than on the frames); only the highlights' roll-off is done here
//! (`heronite_images::backdrop_frame_rolloff`), on ffmpeg's reading thread.
//! The GPU stretches the frame. Frames wait in one slot and only the newest becomes a
//! texture when the window draws, as the trailers' do. It stops while
//! Heronite is not the active window, a game runs, a trailer plays, the
//! stage is not drawn (console mode, Solid) or the window has not drawn for
//! a while (minimized, another workspace), and with Motion other than Full.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui_kit::*;
use heronite_images::{Decoded, ImageRef};
use heronite_media::stream::{Stream, StreamEvent, StreamSpec};
use smallvec::smallvec;

use super::CollectionView;

/// Frames a second: a background moves slowly, and each one redraws the window.
const FPS: u32 = 24;
/// Widest frame, even with no blur: a moving picture behind the panels is
/// not looked at closely (SteamGridDB's previews are 850 px wide), and each
/// pixel costs ffmpeg and the roll-off 24 times a second.
const MAX_W: u32 = 640;
/// A draw asked for this long ago that never came: the window is not shown.
const UNSEEN: Duration = Duration::from_millis(1500);

#[derive(Default)]
pub(super) struct LiveStage {
    /// The file each background plays, by the picture (its game and
    /// source, not its size or look) and its first file (`None`: a still
    /// picture), found off the UI thread once.
    known: HashMap<(String, Option<PathBuf>), Option<PathBuf>>,
    asking: HashSet<(String, Option<PathBuf>)>,
    /// Files ffmpeg could not play this session: they stay still.
    failed: HashSet<PathBuf>,
    /// Whether ffmpeg is there (asked off the UI thread once).
    pub(super) ffmpeg: Option<bool>,
    /// A background that moves is shown and ffmpeg is missing: the ffmpeg
    /// pill says so (`ffmpeg_pill.rs`). Asked again every frame.
    pub(super) wants_ffmpeg: bool,
    /// ffmpeg is being downloaded from the pill.
    pub(super) getting_ffmpeg: bool,
    pub(super) ffmpeg_error: Option<String>,
    playing: Option<Playing>,
    /// The stage was drawn since the last frame.
    seen: bool,
    /// When a frame first asked for a draw that has not happened yet.
    draw_asked: Option<Instant>,
    old_frames: Vec<Arc<RenderImage>>,
    generation: u64,
    /// Wakes the Collection when the window is active again (it stops while
    /// not); asked for by `wants_activation`.
    activation: Option<Subscription>,
    wants_activation: bool,
}

struct Playing {
    /// The file, the frame's size and its look: another one restarts it.
    key: String,
    stream: Stream,
    slot: Arc<Mutex<Option<Decoded>>>,
    frame: Option<Arc<RenderImage>>,
    generation: u64,
}

/// The look a frame gets: the still stage's (blur already scaled to the frame).
struct Look {
    blur: f32,
    saturation: f32,
    rolloff: f32,
}

impl CollectionView {
    /// Every frame, before the layout: the newest frame becomes the one
    /// drawn (the one before goes from the GPU), and a background the stage
    /// no longer draws stops.
    pub(super) fn sync_live_stage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.live.draw_asked = None;
        self.live.wants_ffmpeg = false;
        if std::mem::take(&mut self.live.wants_activation) && self.live.activation.is_none() {
            self.live.activation = Some(cx.observe_window_activation(window, |_, _, cx| cx.notify()));
        }
        if let Some(p) = self.live.playing.as_mut() {
            let newest = p.slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(f) = newest {
                let image = Arc::new(RenderImage::new(smallvec![image::Frame::new(f.into_image_buffer())]));
                self.live.old_frames.extend(p.frame.replace(image));
            }
        }
        for old in self.live.old_frames.drain(..) {
            let _ = window.drop_image(old);
        }
        if !std::mem::take(&mut self.live.seen) {
            self.stop_live_stage();
        }
    }

    /// The moving stage for the picture `still_id` (`picture`: the same
    /// without its size and look) of `chain` drawn at `target` (physical px)
    /// with the stage's look, while it plays; `None` draws the still one.
    /// Starts or stops it as the settings and the window allow. Its frames
    /// keep the still's id: the stage swaps them in place, with no fade.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn live_stage(
        &mut self,
        still_id: &SharedString,
        picture: &str,
        chain: &[ImageRef],
        target: (u32, u32),
        (blur, saturation, rolloff): (f32, f32, f32),
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<(SharedString, ImageSource)> {
        self.live.seen = true;
        if !crate::theme::appearance().animated_backgrounds || !crate::theme::animations() {
            self.stop_live_stage();
            return None;
        }
        // A still picture (or one not known yet) stops what played before.
        let Some(path) = self.live_path(picture, chain, cx) else {
            self.stop_live_stage();
            return None;
        };
        let ffmpeg = self.live.ffmpeg.map(|f| f && !super::ffmpeg_pill::debug_no_ffmpeg());
        // It would move: the header says what is missing.
        self.live.wants_ffmpeg = ffmpeg == Some(false);
        let active = window.is_window_active() || crate::env::flag("HERONITE_DEBUG_LIVE_STAGE");
        if !active && self.live.activation.is_none() {
            self.live.wants_activation = true;
            cx.notify();
        }
        if !active || !self.playing.is_empty() || self.media_playing() || ffmpeg != Some(true) || self.live.failed.contains(&path) {
            self.stop_live_stage();
            return None;
        }
        let opts = heronite_images::Opts::new(PathBuf::new(), None);
        let (w, h, scale) = heronite_images::stage_frame_size(target.0, target.1, blur, MAX_W, &opts);
        // The picture names the file's version too (your own picture is
        // always copied to the same path); the frame's size changes in steps,
        // not with every pixel of a window being resized.
        let key = format!("{picture}|{}|{w}x{h}|{blur}|{saturation}|{rolloff}", path.display());
        if self.live.playing.as_ref().map(|p| &p.key) != Some(&key) {
            self.stop_live_stage();
            self.start_live_stage(key, path, (w, h), Look { blur: blur * scale, saturation, rolloff }, cx);
        }
        let frame = self.live.playing.as_ref()?.frame.clone()?;
        Some((still_id.clone(), ImageSource::Render(frame)))
    }

    /// The file `chain` plays (its first file, when it moves), once known;
    /// asks for it off the UI thread the first time.
    fn live_path(&mut self, picture: &str, chain: &[ImageRef], cx: &mut Context<Self>) -> Option<PathBuf> {
        let first = chain.iter().find_map(|c| match c {
            ImageRef::File(p) => Some(p.clone()),
            _ => None,
        });
        let id = (picture.to_owned(), first);
        if let Some(known) = self.live.known.get(&id) {
            return known.clone();
        }
        if self.live.asking.insert(id.clone()) {
            let chain = chain.to_vec();
            // Again while not found: it may have been installed since.
            let ask_ffmpeg = self.live.ffmpeg != Some(true);
            cx.spawn(async move |this, cx| {
                let (moving, ffmpeg) = cx
                    .background_spawn(async move {
                        let ffmpeg = ask_ffmpeg.then(heronite_media::stream::available);
                        (moving_file(&chain), ffmpeg)
                    })
                    .await;
                this.update(cx, |this, cx| {
                    this.live.asking.remove(&id);
                    if let Some(f) = ffmpeg {
                        this.live.ffmpeg = Some(f);
                    }
                    this.live.known.insert(id, moving);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        None
    }

    fn start_live_stage(&mut self, key: String, path: PathBuf, (width, height): (u32, u32), look: Look, cx: &mut Context<Self>) {
        self.live.generation += 1;
        let generation = self.live.generation;
        let slot: Arc<Mutex<Option<Decoded>>> = Arc::default();
        let (tx, mut rx) = unbounded::<Option<StreamEvent>>();
        let spec = StreamSpec {
            video: path.to_string_lossy().into_owned(),
            audio: None,
            start: 0.,
            width,
            height,
            fps: FPS,
            sound: false,
            volume: 0.,
            fill: true,
            looped: true,
            filter: Some(look_filter(&look)),
        };
        let frames = slot.clone();
        let mut count = 0u64;
        let stream = Stream::start(spec, move |ev| match ev {
            StreamEvent::Frame(mut f) => {
                heronite_images::backdrop_frame_rolloff(&mut f, look.rolloff);
                count += 1;
                let was_empty = frames.lock().unwrap_or_else(|e| e.into_inner()).replace(f).is_none();
                // Also once a second while nothing draws, so an unseen background stops.
                if was_empty || count.is_multiple_of(u64::from(FPS)) {
                    let _ = tx.unbounded_send(None);
                }
            }
            end => {
                let _ = tx.unbounded_send(Some(end));
            }
        });
        self.live.playing = Some(Playing { key, stream, slot, frame: None, generation });
        cx.spawn(async move |this, cx| {
            while let Some(ev) = rx.next().await {
                let alive = this
                    .update(cx, |this, cx| {
                        if this.live.playing.as_ref().is_none_or(|p| p.generation != generation) {
                            return false;
                        }
                        // A draw asked for this long ago never came.
                        let unseen = this.live.draw_asked.is_some_and(|t| t.elapsed() > UNSEEN);
                        this.live.draw_asked.get_or_insert_with(Instant::now);
                        match ev {
                            // A looped background never ends: it could not be played.
                            Some(StreamEvent::Ended | StreamEvent::Failed(_)) => {
                                if let Some(StreamEvent::Failed(why)) = &ev {
                                    log::warn!("animated background {}: {why}", path.display());
                                }
                                this.live.failed.insert(path.clone());
                                this.stop_live_stage();
                            }
                            _ if unseen => {
                                this.stop_live_stage();
                                return false;
                            }
                            _ => {}
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    /// Ends ffmpeg; the still picture shows again.
    fn stop_live_stage(&mut self) {
        if let Some(p) = self.live.playing.take() {
            p.stream.stop();
            self.live.old_frames.extend(p.frame);
        }
    }
}

/// ffmpeg's filters that give a frame the still stage's blur (`look.blur`,
/// already in the frame's pixels; none when too light to see) and colour.
fn look_filter(look: &Look) -> String {
    let saturation = format!("eq=saturation={:.3}", look.saturation.clamp(0., 3.));
    if look.blur < 0.3 {
        saturation
    } else {
        format!("gblur=sigma={:.2}:steps=2,{saturation}", look.blur)
    }
}

/// The file of `chain` that moves: its first file that is there, when that
/// one is a video or an animated picture (the image loader draws the first
/// one there is). Blocking (reads the file's first bytes).
fn moving_file(chain: &[ImageRef]) -> Option<PathBuf> {
    for c in chain {
        let path = match c {
            ImageRef::File(p) => p.clone(),
            ImageRef::Find { .. } => match c.local_path() {
                Some(p) => p,
                // Not there: the loader goes on to the next one too.
                None => continue,
            },
            // A picture from the web comes before any file after it.
            ImageRef::Url(_) => return None,
        };
        if path.is_file() {
            // An animated WebP stays still: ffmpeg loops it badly (one pass,
            // then it spins) and older ones cannot decode it at all.
            let motion = heronite_images::animation::file_motion(&path);
            return motion.filter(|m| *m != heronite_images::animation::Motion::WebP).map(|_| path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{look_filter, moving_file, Look};

    #[test]
    fn ffmpeg_gives_the_frames_the_stage_s_blur_and_colour() {
        let look = |blur| Look { blur, saturation: 0.94, rolloff: 0.5 };
        assert_eq!(look_filter(&look(1.9)), "gblur=sigma=1.90:steps=2,eq=saturation=0.940");
        assert_eq!(look_filter(&look(0.)), "eq=saturation=0.940", "no blur to see");
        assert_eq!(look_filter(&Look { saturation: 9., ..look(0.) }), "eq=saturation=3.000", "within eq's range");
    }
    use heronite_images::ImageRef;

    #[test]
    fn the_first_file_there_is_the_one_that_plays() {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("hero.webm");
        std::fs::write(&video, [0x1a, 0x45, 0xdf, 0xa3, 0, 0, 0, 0]).unwrap();
        let still = dir.path().join("hero.jpg");
        std::fs::write(&still, [0xff, 0xd8, 0xff, 0xe0, 0, 0, 0, 0]).unwrap();
        let missing = dir.path().join("gone.webm");
        let file = |p: &std::path::Path| ImageRef::File(p.to_path_buf());
        assert_eq!(moving_file(&[file(&missing), file(&video)]), Some(video.clone()));
        assert_eq!(moving_file(&[file(&still), file(&video)]), None, "a still picture first");
        assert_eq!(moving_file(&[ImageRef::Url("https://x/a.jpg".into()), file(&video)]), None);
        assert_eq!(moving_file(&[]), None);
        // An animated WebP (VP8X, animation flag) stays still.
        let webp = dir.path().join("hero.webp");
        std::fs::write(&webp, [b"RIFF\0\0\0\0WEBPVP8X".as_slice(), &[10, 0, 0, 0, 0x02, 0, 0, 0]].concat()).unwrap();
        assert_eq!(moving_file(&[file(&webp)]), None);
    }
}
