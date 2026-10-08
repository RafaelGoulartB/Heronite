use super::*;
use std::sync::{mpsc, Arc};
use std::time::Duration;

fn write_png(path: &std::path::Path, w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) {
    let mut img = image::RgbaImage::new(w, h);
    for (x, y, p) in img.enumerate_pixels_mut() {
        *p = image::Rgba(f(x, y));
    }
    img.save(path).unwrap();
}

fn write_jpeg(path: &std::path::Path, w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) {
    let mut img = image::RgbImage::new(w, h);
    for (x, y, p) in img.enumerate_pixels_mut() {
        *p = image::Rgb(f(x, y));
    }
    let mut out = std::fs::File::create(path).unwrap();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95).encode_image(&img).unwrap();
}

fn opts(dir: &std::path::Path) -> Opts {
    let mut o = Opts::new(dir.join("images-cache"), Some(dir.join("thumbs")));
    o.allow_network = false;
    o
}

#[test]
fn bgra_layout_and_straight_alpha() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("red.png");
    // Opaque pure red (R=255,G=0,B=0) => BGRA bytes [0,0,255,255]
    write_png(&p, 40, 60, |_, _| [255, 0, 0, 255]);
    let mut o = opts(dir.path());
    o.thumbs_dir = None;
    let d = load_cover(&[ImageRef::File(p.clone())], 20, 30, &o).unwrap();
    assert_eq!((d.width, d.height, d.bgra.len(), d.bgra.capacity()), (20, 30, 20 * 30 * 4, 20 * 30 * 4));
    for px in d.bgra.chunks(4) {
        assert_eq!(px, [0, 0, 255, 255]);
    }
    // Semi-transparent green stays straight alpha: RGBA (0,200,0,100) -> BGRA [0,200,0,100]
    let p2 = dir.path().join("g.png");
    write_png(&p2, 8, 8, |_, _| [0, 200, 0, 100]);
    let d = load_cover(&[ImageRef::File(p2)], 4, 4, &o).unwrap();
    for px in d.bgra.chunks(4) {
        assert!(px[0] <= 1 && (px[1] as i32 - 200).abs() <= 1 && px[2] <= 1 && (px[3] as i32 - 100).abs() <= 1, "{px:?}");
    }
    // Wrapping for GPUI is zero-copy and keeps BGRA bytes.
    let d = load_cover(&[ImageRef::File(p)], 2, 3, &o).unwrap();
    let ptr = d.bgra.as_ptr();
    let buf = d.into_image_buffer();
    assert_eq!(buf.as_raw().as_ptr(), ptr);
    assert_eq!(buf.get_pixel(0, 0).0, [0, 0, 255, 255]);
}

#[test]
fn jpeg_cover_quadrants_survive_scaled_decode() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("q.jpg");
    // 800x1200: left half red, right half blue (aspect 2:3)
    write_jpeg(&p, 800, 1200, |x, _| if x < 400 { [255, 0, 0] } else { [0, 0, 255] });
    let mut o = opts(dir.path());
    o.thumbs_dir = None;
    for scaled in [true, false] {
        o.jpeg_scaled_decode = scaled;
        let d = load_cover(&[ImageRef::File(p.clone())], 100, 150, &o).unwrap();
        let at = |x: usize, y: usize| &d.bgra[(y * 100 + x) * 4..][..4];
        let l = at(10, 75);
        let r = at(90, 75);
        assert!(l[2] > 240 && l[0] < 20, "left should be red (BGRA): {l:?}");
        assert!(r[0] > 240 && r[2] < 20, "right should be blue (BGRA): {r:?}");
    }
    // Wide source into a square: central crop
    let p2 = dir.path().join("w.jpg");
    write_jpeg(&p2, 900, 300, |x, _| if x < 300 { [255, 0, 0] } else if x < 600 { [0, 255, 0] } else { [0, 0, 255] });
    let d = load_cover(&[ImageRef::File(p2)], 60, 60, &o).unwrap();
    for px in d.bgra.chunks(4) {
        assert!(px[1] > 235 && px[0] < 25 && px[2] < 25, "{px:?}");
    }
}

#[test]
fn thumb_cache_hit_and_invalidation() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.png");
    write_png(&p, 40, 40, |x, _| [x as u8 * 6, 10, 20, 255]);
    let o = opts(dir.path());
    let chain = [ImageRef::File(p.clone())];
    let a = load_cover(&chain, 16, 16, &o).unwrap();
    let thumbs = std::fs::read_dir(dir.path().join("thumbs")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".hit")).count();
    assert_eq!(thumbs, 1);
    // Delete the source content but keep size/mtime? Instead: serve from cache
    // by checking it is bit-identical and no extra file is written.
    let b = load_cover(&chain, 16, 16, &o).unwrap();
    assert_eq!(a, b);
    // Different size => new entry.
    load_cover(&chain, 17, 16, &o).unwrap();
    // Modified source (new size) => new entry and new pixels.
    write_png(&p, 41, 40, |_, _| [200, 0, 0, 255]);
    let c = load_cover(&chain, 16, 16, &o).unwrap();
    assert_ne!(a, c);
    let thumbs = std::fs::read_dir(dir.path().join("thumbs")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".hit")).count();
    assert_eq!(thumbs, 3);
}

#[test]
fn chain_falls_through_missing_and_garbage() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("good.png");
    write_png(&good, 10, 10, |_, _| [1, 2, 3, 255]);
    let bad = dir.path().join("bad.png");
    std::fs::write(&bad, b"<html>404</html>").unwrap();
    let o = opts(dir.path());
    let chain = [
        ImageRef::File(dir.path().join("nope.png")),
        ImageRef::File(bad),
        ImageRef::Url("https://example.invalid/x.jpg".into()), // not cached, network off
        ImageRef::File(good),
    ];
    let d = load_cover(&chain, 5, 5, &o).unwrap();
    assert_eq!(&d.bgra[..4], [3, 2, 1, 255]);
    // all bad => AllFailed with one message per candidate
    let err = load_cover(&chain[..3], 5, 5, &o).unwrap_err();
    match err {
        ImageError::AllFailed(v) => assert_eq!(v.len(), 3),
        e => panic!("{e}"),
    }
    assert!(matches!(load_cover(&[], 5, 5, &o), Err(ImageError::NoCandidates)));
}

#[test]
fn url_chain_uses_shared_cache_file() {
    let dir = tempfile::tempdir().unwrap();
    let o = opts(dir.path());
    let url = "https://cdn.example/library_600x900.jpg";
    std::fs::create_dir_all(&o.images_cache_dir).unwrap();
    write_jpeg(&cache_path(&o.images_cache_dir, url), 60, 90, |_, _| [0, 255, 0]);
    let d = load_cover(&[ImageRef::Url(url.into())], 30, 45, &o).unwrap();
    assert!(d.bgra[0] < 8 && d.bgra[1] > 247 && d.bgra[2] < 8 && d.bgra[3] == 255, "{:?}", &d.bgra[..4]);
}

#[test]
fn stage_is_reduced_blurred_saturated() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("s.png");
    // Left half muted red, right half muted blue, hard edge in the middle.
    write_png(&p, 1600, 900, |x, _| if x < 800 { [160, 90, 90, 255] } else { [90, 90, 160, 255] });
    let mut o = opts(dir.path());
    o.thumbs_dir = None;
    let chain = [ImageRef::File(p)];
    let plain = load_stage(&chain, 1600, 900, 40.0, 1.0, &o).unwrap();
    // blur 40 => sigma_out 8 => s = 0.2 => 320x180
    assert_eq!((plain.width, plain.height), (320, 180));
    let at = |d: &Decoded, x: usize, y: usize| -> [u8; 4] { d.bgra[(y * d.width as usize + x) * 4..][..4].try_into().unwrap() };
    // Far from the edge colors are unchanged (BGRA order)
    assert_eq!(at(&plain, 5, 90), [90, 90, 160, 255]);
    assert_eq!(at(&plain, 315, 90), [160, 90, 90, 255]);
    // At the edge it's a smooth mix of both, and the edge is soft over ~sigma px
    let mid = at(&plain, 160, 90);
    assert!((mid[0] as i32 - 125).abs() <= 3 && (mid[2] as i32 - 125).abs() <= 3, "{mid:?}");
    let near = at(&plain, 160 - 16, 90); // 2 sigma left of the edge
    assert!(near[2] > 150 && near[2] < 160 && near[0] > 90 && near[0] < 100, "{near:?}");
    // Saturation boosts chroma but leaves alpha
    let sat = load_stage(&chain, 1600, 900, 40.0, 1.5, &o).unwrap();
    let px = at(&sat, 315, 90);
    assert!(px[0] > 160 && px[2] < 90 && px[3] == 255, "{px:?}");
    // Glass is a separate (stronger) variant
    let glass = load_panel_glass(&chain, 1600, 900, PANEL_GLASS_BLUR_PX, 1.0, &o).unwrap();
    // s = 8/20 = 0.4 => 640x360
    assert_eq!((glass.width, glass.height), (640, 360));
}

#[test]
fn stage_and_glass_are_cached_separately() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("s.png");
    write_png(&p, 200, 100, |x, _| [x as u8, 50, 50, 255]);
    let o = opts(dir.path());
    let chain = [ImageRef::File(p)];
    let a = load_stage(&chain, 200, 100, 20.0, 1.2, &o).unwrap();
    let b = load_stage(&chain, 200, 100, 20.0, 1.2, &o).unwrap();
    assert_eq!(a, b);
    load_panel_glass(&chain, 200, 100, 20.0, 1.2, &o).unwrap();
    let n = std::fs::read_dir(dir.path().join("thumbs")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".hit")).count();
    assert_eq!(n, 2);
}

fn loader_with_png(dir: &std::path::Path, threads: usize) -> (Loader, ImageRef) {
    let p = dir.join("a.png");
    write_png(&p, 64, 64, |_, _| [1, 2, 3, 255]);
    let mut o = opts(dir);
    o.thumbs_dir = None;
    let mut cfg = LoaderConfig::new(o);
    cfg.threads = threads;
    (Loader::new(cfg), ImageRef::File(p))
}

#[test]
fn loader_dedups_in_flight_requests() {
    let dir = tempfile::tempdir().unwrap();
    let (l, img) = loader_with_png(dir.path(), 1);
    let (tx, rx) = mpsc::channel();
    // Block the single worker with a slow-ish first job so the rest queue up.
    let req = Request::cover(vec![img.clone()], 32, 32, 0);
    let k1 = l.submit_channel(req.clone(), tx.clone());
    let k2 = l.submit_channel(req.clone(), tx.clone());
    let k3 = l.submit_channel(req, tx);
    assert_eq!((k1, k2), (k2, k3));
    let mut got = 0;
    let mut ptrs = std::collections::HashSet::new();
    while let Ok((_, r)) = rx.recv_timeout(Duration::from_secs(5)) {
        got += 1;
        ptrs.insert(Arc::as_ptr(&r.unwrap()) as usize);
        if got == 3 {
            break;
        }
    }
    assert_eq!(got, 3);
    assert_eq!(ptrs.len(), 1, "all waiters share one decode");
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(l.pending(), 0);
}

#[test]
fn loader_runs_higher_priority_first_and_cancels() {
    let dir = tempfile::tempdir().unwrap();
    let (l, img) = loader_with_png(dir.path(), 1);
    // Occupy the single worker: a gate callback can't block the worker before
    // it picks, so instead queue many and check order of completion.
    let (tx, rx) = mpsc::channel();
    let mk = |w: u32, prio: i32| Request::cover(vec![img.clone()], w, 32, prio);
    // Stall the worker using a request whose callback sleeps.
    let (gate_tx, gate_rx) = mpsc::channel::<()>();
    l.submit(mk(31, 1000), move |_| {
        let _ = gate_rx.recv_timeout(Duration::from_secs(5));
    });
    std::thread::sleep(Duration::from_millis(100)); // worker now inside the callback
    let low = l.submit_channel(mk(10, 0), tx.clone());
    let mid = l.submit_channel(mk(11, 5), tx.clone());
    let high = l.submit_channel(mk(12, 9), tx.clone());
    let cancelled = l.submit_channel(mk(13, 7), tx.clone());
    assert!(l.cancel(cancelled));
    l.set_priority(low, 20); // bump the lowest to the front
    gate_tx.send(()).unwrap();
    let order: Vec<RequestKey> = (0..3).map(|_| rx.recv_timeout(Duration::from_secs(5)).unwrap().0).collect();
    assert_eq!(order, vec![low, high, mid]);
    assert!(rx.recv_timeout(Duration::from_millis(200)).is_err(), "cancelled request must not call back");
}

#[test]
fn loader_reports_errors_and_survives() {
    let dir = tempfile::tempdir().unwrap();
    let (l, img) = loader_with_png(dir.path(), 2);
    let (tx, rx) = mpsc::channel();
    l.submit_channel(Request::cover(vec![ImageRef::File(dir.path().join("missing"))], 8, 8, 0), tx.clone());
    assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap().1.is_err());
    l.submit_channel(Request::cover(vec![img], 8, 8, 0), tx);
    assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap().1.is_ok());
}
