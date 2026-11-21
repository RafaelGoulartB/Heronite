//! The first frame of a video as a picture, for the pictures that may be
//! videos (an animated background): wherever such a picture is drawn still
//! (the background with "Animated backgrounds" off, the glass, a preview),
//! it is this frame. Plugged into the image loader as
//! [`heronite_images::Opts::video_still`].

use std::path::Path;
use std::time::Duration;

use heronite_data::ffmpeg::{command, Tool};
use heronite_data::proc::Run;

/// A short or local video gives its first frame well within this.
const TIMEOUT: Duration = Duration::from_secs(15);

/// The first frame of the video at `path` as PNG bytes; `None` without
/// ffmpeg or when it cannot read the file.
pub fn video_still(path: &Path) -> Option<Vec<u8>> {
    heronite_data::ffmpeg::find(Tool::Ffmpeg)?;
    let mut cmd = command(Tool::Ffmpeg);
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-threads", "2", "-i"]).arg(path).args([
        "-map",
        "0:v:0",
        "-frames:v",
        "1",
        "-an",
        "-sn",
        "-f",
        "image2pipe",
        "-c:v",
        "png",
        "pipe:1",
    ]);
    match Run::new().timeout(TIMEOUT).run(&mut cmd) {
        Ok(out) if out.success() && out.stdout.starts_with(&[0x89, b'P', b'N', b'G']) => Some(out.stdout),
        Ok(out) => {
            log::warn!("no frame of {}: {}", path.display(), out.stderr_text().trim());
            None
        }
        Err(e) => {
            log::warn!("no frame of {}: {e}", path.display());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_video_s_first_frame_is_a_png() {
        if heronite_data::ffmpeg::find(Tool::Ffmpeg).is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.webm");
        let mut cmd = command(Tool::Ffmpeg);
        cmd.args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=64x36:rate=10:duration=1"])
            .args(["-c:v", "libvpx", "-b:v", "100k"])
            .arg(&path);
        if !Run::new().timeout(Duration::from_secs(60)).run(&mut cmd).is_ok_and(|o| o.success()) {
            return;
        }
        let png = video_still(&path).expect("a frame");
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!((img.width(), img.height()), (64, 36));
        assert!(video_still(&dir.path().join("missing.webm")).is_none());
    }
}
