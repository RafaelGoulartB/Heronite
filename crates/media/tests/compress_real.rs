//! Lossless compression of real captures. `HERONITE_COMPRESS_SAMPLE` names a
//! folder of PNGs; each is copied to a temp folder and converted there (the
//! originals are only read). Skipped when the variable is not set.

use heronite_media::compress::{compress_file, usage, Outcome};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant, SystemTime};

#[test]
fn real_captures_convert_pixel_exact() {
    let Some(sample) = std::env::var_os("HERONITE_COMPRESS_SAMPLE").map(PathBuf::from) else {
        return eprintln!("HERONITE_COMPRESS_SAMPLE not set; skipped");
    };
    let work = tempfile::tempdir().unwrap();
    let game = work.path().join("Game");
    std::fs::create_dir_all(&game).unwrap();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&sample).unwrap().flatten().map(|e| e.path()).collect();
    files.retain(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")));
    files.sort();
    let (mut before, mut after, mut time) = (0u64, 0u64, Duration::ZERO);
    for src in &files {
        let copy = game.join(src.file_name().unwrap());
        std::fs::copy(src, &copy).unwrap();
        let old = SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options().write(true).open(&copy).unwrap().set_modified(old).unwrap();
        let t = Instant::now();
        let out = compress_file(&copy, &AtomicBool::new(false)).unwrap();
        time += t.elapsed();
        let Outcome::Converted { to, .. } = out else { panic!("{}: {out:?}", src.display()) };
        let a = image::open(src).unwrap().to_rgba8();
        let b = image::open(&to).unwrap().to_rgba8();
        assert!(a == b, "{} changed", src.display());
        before += std::fs::metadata(src).unwrap().len();
        after += std::fs::metadata(&to).unwrap().len();
    }
    let u = usage(work.path());
    assert_eq!(u.compressed_files as usize, files.len());
    assert_eq!(u.saved_bytes, before - after);
    println!(
        "{} files: {:.1} MB -> {:.1} MB ({:.1}% smaller), {:.2} s/file",
        files.len(),
        before as f64 / 1e6,
        after as f64 / 1e6,
        100.0 * (1.0 - after as f64 / before as f64),
        time.as_secs_f64() / files.len().max(1) as f64
    );
}
