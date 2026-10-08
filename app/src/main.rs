mod assets;
mod collection;
mod icons;
mod images;
mod theme;
mod ui;

use std::time::Instant;

use gpui_kit::*;

fn main() {
    let started = Instant::now();
    env_logger_init();

    gpui_kit::application().with_assets(assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        assets::load_fonts(cx);

        let bounds = Bounds::centered(None, size(px(1600.), px(1000.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            app_id: Some("com.heronite.rs".into()),
            window_background: WindowBackgroundAppearance::Opaque,
            window_min_size: Some(size(px(640.), px(480.))),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            window.set_window_title("Heronite");
            cx.new(|cx| collection::CollectionView::new(started, window, cx))
        })
        .expect("failed to open window");
        cx.activate(true);
    });
}

fn env_logger_init() {
    // Minimal stderr logger; RUST_LOG=debug for more.
    struct L(log::LevelFilter);
    impl log::Log for L {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= self.0
        }
        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                eprintln!("[{} {}] {}", r.level(), r.target(), r.args());
            }
        }
        fn flush(&self) {}
    }
    let level = match std::env::var("RUST_LOG").as_deref() {
        Ok("debug") => log::LevelFilter::Debug,
        Ok("trace") => log::LevelFilter::Trace,
        _ => log::LevelFilter::Info,
    };
    let _ = log::set_boxed_logger(Box::new(L(level))).map(|_| log::set_max_level(level));
}
