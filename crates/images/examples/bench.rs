//! Benchmark on real Heronite covers (read-only use of the Heroic image cache).
//!
//! cargo run --release -p heronite-images --example bench -- [cache_dir] [N] [--tw 250 --th 375] [--keep] [--threads 3]
//!
//! Each phase resets the kernel's peak-RSS counter first, so "peak" is the
//! high-water mark of that phase alone (baseline RSS is printed too).
use heronite_images::*;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

fn status(key: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|l| l.starts_with(key))
        .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        .unwrap_or(0)
}
fn reset_peak() {
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}
fn mb(kb: u64) -> f64 {
    kb as f64 / 1024.0
}
fn stats(name: &str, mut v: Vec<f64>) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    let avg = v.iter().sum::<f64>() / n as f64;
    println!("  {name:<34} n={n:<4} avg {avg:>8.3} ms  p50 {:>8.3}  p95 {:>8.3}  max {:>8.3}", v[n / 2], v[n * 95 / 100], v[n - 1]);
}
fn phase<T>(name: &str, f: impl FnOnce() -> T) -> T {
    reset_peak();
    let base = status("VmRSS:");
    let t = Instant::now();
    let r = f();
    println!("  [{name}] wall {:.0} ms | RSS before {:.1} MB, peak {:.1} MB, after {:.1} MB", t.elapsed().as_secs_f64() * 1e3, mb(base), mb(status("VmHWM:")), mb(status("VmRSS:")));
    r
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |args: &mut Vec<String>, name: &str, default: u32| -> u32 {
        if let Some(i) = args.iter().position(|a| a == name) {
            let v = args[i + 1].parse().unwrap();
            args.drain(i..i + 2);
            v
        } else {
            default
        }
    };
    let (tw, th) = (flag(&mut args, "--tw", 250), flag(&mut args, "--th", 375));
    let threads = flag(&mut args, "--threads", 3) as usize;
    let keep = if let Some(i) = args.iter().position(|a| a == "--keep") { args.remove(i); true } else { false };
    let cache: PathBuf = args.first().map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join(".local/share/heronite/images-cache"));
    let n: usize = args.get(1).map(|s| s.parse().unwrap()).unwrap_or(100);

    // Pick portrait JPEG covers and landscape heroes by header dims only.
    let mut covers = Vec::new();
    let mut heroes = Vec::new();
    for e in std::fs::read_dir(&cache).expect("cache dir").flatten() {
        if covers.len() >= n && heroes.len() >= 12 {
            break;
        }
        let Ok(b) = std::fs::read(e.path()) else { continue };
        if b.len() < 4096 { continue; }
        let Ok(r) = image::ImageReader::new(Cursor::new(&b)).with_guessed_format() else { continue };
        let Ok((w, h)) = r.into_dimensions() else { continue };
        if h > w && w >= 300 && covers.len() < n {
            covers.push((e.path(), w, h, b.len()));
        } else if w > 1500 && heroes.len() < 12 {
            heroes.push((e.path(), w, h, b.len()));
        }
    }
    println!("cache {} | {} covers (e.g. {}x{}, avg {} KB), {} heroes | target {tw}x{th}", cache.display(), covers.len(), covers[0].1, covers[0].2, covers.iter().map(|c| c.3).sum::<usize>() / covers.len() / 1024, heroes.len());
    let _ = std::fs::write("/proc/self/clear_refs", "5");
    println!("baseline RSS {:.1} MB", mb(status("VmRSS:")));

    let tmp = std::env::temp_dir().join(format!("heronite-images-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let mut o = Opts::new(cache.clone(), Some(tmp.join("thumbs")));
    o.allow_network = false; // never touch the network or write into the Heroic cache
    let chains: Vec<Vec<ImageRef>> = covers.iter().map(|c| vec![ImageRef::File(c.0.clone())]).collect();

    println!("\n== Cover {tw}x{th}: cold (decode+resize+write thumb), sequential ==");
    let mut kept = Vec::new();
    phase("cold", || {
        let mut ms = Vec::new();
        for c in &chains {
            let t = Instant::now();
            let d = load_cover(c, tw, th, &o).unwrap();
            ms.push(t.elapsed().as_secs_f64() * 1e3);
            if keep { kept.push(d); }
        }
        stats("cold load_cover", ms);
    });
    println!("  retained outputs: {} x {:.0} KB = {:.1} MB", kept.len(), (tw * th * 4) as f64 / 1024.0, (kept.len() as u64 * (tw * th * 4) as u64) as f64 / 1048576.0);
    drop(kept);

    println!("\n== Cover: warm (thumb cache hit), sequential ==");
    phase("warm", || {
        let mut ms = Vec::new();
        for c in &chains {
            let t = Instant::now();
            let d = load_cover(c, tw, th, &o).unwrap();
            ms.push(t.elapsed().as_secs_f64() * 1e3);
            std::hint::black_box(d);
        }
        stats("warm load_cover", ms);
    });
    let sz: u64 = std::fs::read_dir(tmp.join("thumbs")).unwrap().flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum();
    println!("  thumb cache: {:.1} MB on disk for {} covers (raw would be {:.1} MB)", sz as f64 / 1048576.0, chains.len(), (chains.len() as u32 * tw * th * 4) as f64 / 1048576.0);

    println!("\n== Cover: decode-path comparison (no thumb cache) ==");
    let mut o2 = o.clone();
    o2.thumbs_dir = None;
    for (name, scaled, f) in [("jpeg scaled + Lanczos3", true, Filter::Lanczos3), ("jpeg full    + Lanczos3", false, Filter::Lanczos3), ("jpeg scaled + Bilinear", true, Filter::Bilinear)] {
        o2.jpeg_scaled_decode = scaled;
        o2.filter = f;
        let mut ms = Vec::new();
        for c in &chains {
            let t = Instant::now();
            std::hint::black_box(load_cover(c, tw, th, &o2).unwrap());
            ms.push(t.elapsed().as_secs_f64() * 1e3);
        }
        stats(name, ms);
    }

    println!("\n== Loader: {threads} threads, cold, {} covers ==", chains.len());
    let _ = std::fs::remove_dir_all(tmp.join("thumbs"));
    phase("loader cold", || {
        let loader = Loader::new(LoaderConfig { threads, opts: o.clone() });
        let (tx, rx) = mpsc::channel();
        let t = Instant::now();
        for (i, c) in chains.iter().enumerate() {
            loader.submit_channel(Request::cover(c.clone(), tw, th, -(i as i32)), tx.clone());
        }
        for _ in 0..chains.len() {
            rx.recv().unwrap().1.unwrap();
        }
        let e = t.elapsed().as_secs_f64();
        println!("  {} covers in {:.0} ms = {:.2} ms/cover throughput", chains.len(), e * 1e3, e * 1e3 / chains.len() as f64);
    });

    if !heroes.is_empty() {
        println!("\n== Stage (hero {}x{} -> window), blur 40px sat 1.2 ==", heroes[0].1, heroes[0].2);
        let hchains: Vec<Vec<ImageRef>> = heroes.iter().map(|c| vec![ImageRef::File(c.0.clone())]).collect();
        for (ww, wh) in [(1920u32, 1080u32), (3840, 2160)] {
            phase(&format!("stage {ww}x{wh}"), || {
                let (mut cold, mut warm) = (Vec::new(), Vec::new());
                let mut dims = (0, 0);
                for c in &hchains {
                    let t = Instant::now();
                    let d = load_stage(c, ww, wh, 40.0, 1.2, &o).unwrap();
                    cold.push(t.elapsed().as_secs_f64() * 1e3);
                    dims = (d.width, d.height);
                    let t = Instant::now();
                    std::hint::black_box(load_stage(c, ww, wh, 40.0, 1.2, &o).unwrap());
                    warm.push(t.elapsed().as_secs_f64() * 1e3);
                }
                println!("  window {ww}x{wh} -> raster {}x{} ({:.2} MB)", dims.0, dims.1, (dims.0 * dims.1 * 4) as f64 / 1048576.0);
                stats("stage cold", cold);
                stats("stage warm", warm);
                let t = Instant::now();
                let g = load_panel_glass(&hchains[0], ww, wh, PANEL_GLASS_BLUR_PX, 1.2, &o).unwrap();
                println!("  glass cold ({}x{}): {:.2} ms", g.width, g.height, t.elapsed().as_secs_f64() * 1e3);
            });
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
