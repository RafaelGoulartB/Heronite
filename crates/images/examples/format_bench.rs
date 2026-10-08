//! Benchmarks candidate on-disk formats for the resized-thumb cache.
//! Reads covers from the (read-only) Heroic cache dir given as argv[1].
use std::time::Instant;

fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let n: usize = std::env::args().nth(2).map(|s| s.parse().unwrap()).unwrap_or(40);
    let (tw, th) = (250u32, 375u32);
    let mut samples: Vec<Vec<u8>> = Vec::new(); // BGRA
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        if samples.len() >= n { break; }
        let b = std::fs::read(e.path()).unwrap();
        if b.len() < 20_000 || &b[..2] != [0xff, 0xd8] { continue; }
        let Ok(img) = image::load_from_memory(&b) else { continue };
        if img.width() != 600 && img.width() != 300 { continue; }
        let img = img.resize_to_fill(tw, th, image::imageops::FilterType::Triangle).to_rgba8();
        samples.push(img.into_raw());
    }
    println!("{} samples of {}x{} ({} KB raw)", samples.len(), tw, th, samples[0].len() / 1024);
    let reps = 5;
    let t = |name: &str, enc: &dyn Fn(&[u8]) -> Vec<u8>, dec: &dyn Fn(&[u8]) -> Vec<u8>| {
        let encoded: Vec<Vec<u8>> = samples.iter().map(|s| enc(s)).collect();
        let size: usize = encoded.iter().map(|e| e.len()).sum::<usize>() / encoded.len();
        let mut best = f64::MAX;
        for _ in 0..reps {
            let s = Instant::now();
            for e in &encoded { std::hint::black_box(dec(e)); }
            best = best.min(s.elapsed().as_secs_f64() / encoded.len() as f64);
        }
        println!("{name:<12} avg size {:>6} KB  decode {:>7.1} us", size / 1024, best * 1e6);
    };
    let (w, h) = (tw, th);
    t("raw", &|s| s.to_vec(), &|e| e.to_vec());
    t("lz4", &|s| lz4_flex::block::compress(s), &|e| lz4_flex::block::decompress(e, (w * h * 4) as usize).unwrap());
    t("zstd1", &|s| zstd::encode_all(s, 1).unwrap(), &|e| zstd::decode_all(e).unwrap());
    t("qoi", &|s| qoi::encode_to_vec(s, w, h).unwrap(), &|e| qoi::decode_to_vec(e).unwrap().1);
    t("jpeg q90", &|s| {
        let mut out = Vec::new();
        let rgb: Vec<u8> = s.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90).encode(&rgb, w, h, image::ExtendedColorType::Rgb8).unwrap();
        out
    }, &|e| image::load_from_memory(e).unwrap().into_rgba8().into_raw());
}
