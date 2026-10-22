//! Settings → Replay: the replay buffer (off by default), its length, the
//! video and sound it keeps, and what that costs: the memory it takes while
//! a game runs, worked out from the screen, the frame rate and the bitrate
//! (`heronite_overlay::replay::estimate`), and how to spare it (the encoder,
//! the buffer on disk, the panel's margin). The options live in
//! `replay.json` (`heronite_data::replay_prefs`), are the same for every
//! game and reach the running service at once; a game can only have it on or
//! off on its own (Game settings → Performance). Guide + X is the overlay's
//! (`overlay::replay_shortcut_card`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::replay_prefs::{Audio, Codec, Encoder, Options, Quality, ReplayPrefs, Resolution, Storage, FPS, PANEL_MARGINS, SECONDS};
use heronite_i18n::tr;
use heronite_overlay::replay::plan::buffer_seconds;
use heronite_overlay::replay::{estimate, Encoding, Estimate, Info, Program};

use super::screenshots::format_size;
use super::{overlay, SettingsPage};
use crate::pages::PageCtx;
use crate::services::replay::{Detected, Status};
use crate::theme;
use crate::ui::controls::{caption, card, card_box, row, tabs, toggle_row};
use crate::ui::fields::{meta_text, select_field};

/// Screen the estimate assumes until the recorder says which ones there are.
const ASSUMED_SCREEN: (u32, u32) = (1920, 1080);

pub struct State {
    /// `None` until it is looked for: once per visit of Settings (it wakes
    /// the graphics card), or what the recorder found while a game runs.
    detected: Option<Detected>,
    looking: bool,
    /// The menu of the monitor is open.
    monitor_open: bool,
}

impl State {
    pub fn new(_ctx: &PageCtx, cx: &mut Context<SettingsPage>) -> Self {
        // Its options and status (recording, unavailable) change in the service.
        if let Some(service) = crate::services::replay::get(cx) {
            cx.observe(&service, |_, _, cx| cx.notify()).detach();
        }
        Self { detected: None, looking: false, monitor_open: false }
    }
}

/// Looks for gpu-screen-recorder and what it can do the first time the
/// section shows (what the recorder found, while a game runs): `--info`
/// opens the graphics card's encoders, so scrolling past the section or
/// coming back to it does not ask again.
pub fn shown(d: &mut SettingsPage, cx: &mut Context<SettingsPage>) {
    if d.replay.detected.is_some() || d.replay.looking {
        return;
    }
    if let Some(found) = crate::services::replay::cached_detected(cx) {
        d.replay.detected = Some(found);
        return;
    }
    d.replay.looking = true;
    cx.spawn(async move |this, cx| {
        let detected = cx.background_spawn(async { crate::services::replay::detect() }).await;
        this.update(cx, |d, cx| {
            d.replay.detected = Some(detected);
            d.replay.looking = false;
            cx.notify();
        })
        .ok();
    })
    .detach();
}

fn prefs(d: &SettingsPage, cx: &App) -> ReplayPrefs {
    crate::services::replay::prefs(cx, &d.ctx.app)
}

/// Changes the options (the service saves them and restarts the recorder).
fn update(d: &mut SettingsPage, cx: &mut Context<SettingsPage>, change: impl FnOnce(&mut ReplayPrefs)) {
    crate::services::replay::change(cx, &d.ctx.app, change);
    cx.notify();
}

fn seconds_label(s: u32) -> &'static str {
    match s {
        15 => "15 s",
        30 => "30 s",
        60 => "1 min",
        120 => "2 min",
        _ => "5 min",
    }
}

fn quality_label(q: Quality) -> &'static str {
    match q {
        Quality::Low => tr!("Light"),
        Quality::Balanced => tr!("Balanced"),
        Quality::High => tr!("High"),
    }
}

fn resolution_label(r: Resolution) -> &'static str {
    match r {
        Resolution::Native => tr!("The screen's"),
        Resolution::P1440 => "1440p",
        Resolution::P1080 => "1080p",
        Resolution::P720 => "720p",
    }
}

fn codec_label(c: Codec) -> &'static str {
    match c {
        Codec::H264 => "H.264",
        Codec::Hevc => "HEVC",
        Codec::Av1 => "AV1",
    }
}

fn encoder_label(e: Encoder) -> &'static str {
    match e {
        Encoder::Auto => tr!("Automatic"),
        Encoder::Vulkan => "Vulkan",
        Encoder::Native => "NVENC · VA-API",
    }
}

fn storage_label(s: Storage) -> &'static str {
    match s {
        Storage::Ram => "RAM",
        Storage::Disk => tr!("Disk"),
    }
}

fn margin_label(m: u32) -> &'static str {
    match m {
        0 => tr!("Off"),
        5 => "5 s",
        _ => "10 s",
    }
}

fn audio_label(a: Audio) -> &'static str {
    match a {
        Audio::Game => tr!("The game's"),
        Audio::All => tr!("Everything"),
        Audio::Off => tr!("None"),
    }
}

fn fps_label(f: u32) -> &'static str {
    if f == 30 {
        "30 FPS"
    } else {
        "60 FPS"
    }
}

/// What `--info` said, when it worked.
fn info_of(st: &State) -> Option<&Info> {
    st.detected.as_ref().and_then(|d| d.info.as_ref().ok())
}

/// How `o` is encoded here: the codec the card has, through Vulkan or not.
fn encoding_for(o: &Options, info: Option<&Info>) -> Encoding {
    let codec = info.and_then(|i| i.codec_for(o.codec)).unwrap_or(o.codec);
    let vulkan = match info {
        Some(i) => i.uses_vulkan(o.encoder, codec),
        None => o.encoder == Encoder::Vulkan,
    };
    Encoding { codec, vulkan }
}

/// The encoder's name: Vulkan, NVENC or VA-API.
fn encoder_name(vulkan: bool, info: Option<&Info>) -> &'static str {
    match (vulkan, info.map(Info::is_nvidia)) {
        (true, _) => "Vulkan",
        (false, Some(true)) => "NVENC",
        (false, Some(false)) => "VA-API",
        (false, None) => "NVENC · VA-API",
    }
}

/// The estimate on the monitor chosen, else the largest one (the game may
/// be on it), else a 1080p screen until they are known; with how it is
/// encoded and the screen's name, when known.
fn estimate_for(o: &Options, st: &State) -> (Estimate, Encoding, Option<String>) {
    let info = info_of(st);
    let screens = info.map(|i| i.screens.as_slice()).unwrap_or_default();
    let screen =
        screens.iter().find(|s| s.name == o.monitor).or_else(|| screens.iter().max_by_key(|s| u64::from(s.width) * u64::from(s.height)));
    let (w, h) = screen.map_or(ASSUMED_SCREEN, |s| (s.width, s.height));
    let encoding = encoding_for(o, info);
    let vendor = info.map(|i| i.vendor.as_str()).unwrap_or_default();
    (estimate(o, encoding, w, h, vendor), encoding, screen.map(|s| s.name.clone()))
}

fn estimate_card(o: &Options, st: &State) -> Div {
    let (e, encoding, name) = estimate_for(o, st);
    let video = tr!(
        "{w} × {h} at {fps} FPS, {mbps} Mbit/s {codec} through {encoder}",
        w = e.width,
        h = e.height,
        fps = o.fps,
        mbps = heronite_i18n::decimal(f64::from(e.video_kbps) / 1000., 1),
        codec = codec_label(encoding.codec),
        encoder = encoder_name(encoding.vulkan, info_of(st))
    );
    let screen = match name {
        Some(n) => tr!("For {name}: {video}.", name = n, video = video),
        None => tr!("For a 1920 × 1080 screen: {video}.", video = video),
    };
    let kept = if o.panel_margin > 0 {
        tr!(
            "It keeps {kept} s: the {n} s saved and {margin} s for a save from the panel, whose time is cut off.",
            kept = buffer_seconds(o),
            n = o.seconds,
            margin = o.panel_margin
        )
    } else {
        tr!("It keeps the {n} s saved.", n = o.seconds)
    };
    let ram_grows = match o.storage {
        Storage::Ram => tr!("Grows with the length saved."),
        Storage::Disk => tr!("The buffer is on disk: only the recorder's own."),
    };
    let stats = div()
        .flex()
        .flex_wrap()
        .gap(px(32.))
        .child(stat(tr!("About {size}", size = format_size(e.ram_bytes)), tr!("Memory (RAM)"), ram_grows))
        .child(stat(
            tr!("About {size}", size = format_size(e.vram_bytes)),
            tr!("Video memory (VRAM)"),
            tr!("Grows with the video's resolution."),
        ));
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .py(px(12.))
        .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(tr!("Memory while a game runs")))
        .child(stats.mb(px(4.)))
        .child(meta_text(screen))
        .child(meta_text(kept));
    if e.disk_bytes_per_sec > 0 {
        col = col.child(meta_text(tr!(
            "It writes about {size} an hour to disk while a game runs.",
            size = format_size(e.disk_bytes_per_sec * 3600)
        )));
    }
    card_box().child(col.child(meta_text(tr!("Each replay takes about {size} on disk.", size = format_size(e.file_bytes)))))
}

/// One number of the estimate, what it is and what makes it grow.
fn stat(value: String, label: &'static str, grows: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(div().text_size(px(22.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(value))
        .child(div().text_size(px(13.)).text_color(theme::text_primary()).child(label))
        .child(meta_text(grows))
}

fn program_text(d: Option<&Detected>) -> (String, bool) {
    let Some(d) = d else { return (tr!("Looking for it…").into(), true) };
    match (&d.program, &d.info) {
        (None, _) => (tr!("Not installed").into(), false),
        (Some(_), Err(e)) => (e.clone(), false),
        (Some(p), Ok(info)) => {
            let how = match p {
                Program::Native(_) => tr!("installed on the system"),
                Program::Flatpak => tr!("Flatpak"),
            };
            if info.recent_enough() {
                (tr!("Version {v}, {how}", v = info.version_text(), how = how), true)
            } else {
                (tr!("Version {v} is too old: 6.0 or newer is needed", v = info.version_text()), false)
            }
        }
    }
}

fn encoder_text(d: Option<&Detected>) -> Option<(String, bool)> {
    let info = d?.info.as_ref().ok()?;
    let codecs: Vec<&str> = Codec::ALL.iter().filter(|c| info.encodes(**c)).map(|c| codec_label(*c)).collect();
    Some(if codecs.is_empty() {
        (tr!("None: the graphics card has no video encoder it can use").into(), false)
    } else {
        let vulkan: Vec<&str> = Codec::ALL.iter().filter(|c| info.encodes_vulkan(**c)).map(|c| codec_label(*c)).collect();
        let mut text = format!("{} \u{b7} {}", info.vendor.to_uppercase(), codecs.join(", "));
        if !vulkan.is_empty() {
            text.push_str(&format!(" \u{b7} Vulkan {}", vulkan.join(", ")));
        }
        (text, true)
    })
}

fn status_text(st: &State, cx: &App) -> (String, bool) {
    let Some(service) = crate::services::replay::get(cx) else { return (String::new(), true) };
    match service.read(cx).status() {
        Status::Off => (tr!("Off").into(), true),
        Status::Idle => (tr!("Waiting for a game").into(), true),
        Status::OffForGame => (tr!("Off for the game running").into(), true),
        Status::OffForSession => (tr!("Off until the game ends (overlay)").into(), true),
        Status::Starting => (tr!("Starting…").into(), true),
        Status::Recording { monitor, codec, vulkan } => {
            let encoder = encoder_name(*vulkan, info_of(st));
            (tr!("Recording {monitor} ({codec} through {encoder})", monitor = monitor.clone(), codec = codec_label(*codec), encoder), true)
        }
        Status::Unavailable(why) => (why.clone(), false),
    }
}

fn status_line(text: String, ok: bool) -> Div {
    div().text_size(px(13.)).text_color(if ok { theme::text_primary() } else { theme::danger_text() }).max_w(px(420.)).child(text)
}

/// Picks among `items` with tabs that save at once.
fn choice_tabs<T: Copy + PartialEq + 'static>(
    id: &str,
    items: &[(T, &'static str)],
    active: T,
    cx: &mut Context<SettingsPage>,
    set: impl Fn(&mut Options, T) + 'static,
) -> Div {
    let page = cx.entity().downgrade();
    let set = std::rc::Rc::new(set);
    tabs(id, items, active, move |v, _, cx| {
        let set = set.clone();
        page.update(cx, |d, cx| update(d, cx, |p| set(&mut p.options, v))).ok();
    })
}

fn monitor_select(d: &SettingsPage, o: &Options, cx: &mut Context<SettingsPage>) -> Option<Div> {
    let info = info_of(&d.replay)?;
    let mut names: Vec<String> = vec![String::new()];
    names.extend(info.screens.iter().map(|s| s.name.clone()));
    let label = |n: &str| -> String {
        match info.screens.iter().find(|s| s.name == n) {
            _ if n.is_empty() => tr!("The game's").into(),
            Some(s) => format!("{} ({} × {})", s.name, s.width, s.height),
            None => n.to_string(),
        }
    };
    let options: Vec<(SharedString, bool)> = names.iter().map(|n| (label(n).into(), *n == o.monitor)).collect();
    let picked = names.clone();
    Some(select_field(
        "replay-monitor",
        None,
        label(&o.monitor),
        false,
        d.replay.monitor_open,
        options,
        cx,
        |d: &mut SettingsPage, _, cx| {
            d.replay.monitor_open = !d.replay.monitor_open;
            cx.notify();
        },
        move |d: &mut SettingsPage, i, _, cx| {
            d.replay.monitor_open = false;
            let name = picked.get(i).cloned().unwrap_or_default();
            update(d, cx, |p| p.options.monitor = name);
        },
    ))
}

/// Quality, frame rate, resolution, format and monitor.
fn video_card(d: &SettingsPage, o: &Options, cx: &mut Context<SettingsPage>) -> Div {
    let qualities: Vec<(Quality, &'static str)> = Quality::ALL.iter().map(|q| (*q, quality_label(*q))).collect();
    let rates: Vec<(u32, &'static str)> = FPS.iter().map(|f| (*f, fps_label(*f))).collect();
    let sizes: Vec<(Resolution, &'static str)> = Resolution::ALL.iter().map(|r| (*r, resolution_label(*r))).collect();
    let codecs: Vec<(Codec, &'static str)> = Codec::ALL.iter().map(|c| (*c, codec_label(*c))).collect();
    let codec_desc = match info_of(&d.replay) {
        Some(i) if !i.encodes(o.codec) => tr!("This graphics card cannot encode it: H.264 is used instead."),
        _ => tr!("H.264 plays everywhere. HEVC and AV1 look the same in less memory and space."),
    };
    let mut rows = vec![
        row(
            tr!("Quality"),
            Some(tr!("More bits for each frame: a sharper picture, more memory.")),
            choice_tabs("replay-quality", &qualities, o.quality, cx, |o, v| o.quality = v),
        )
        .into_any_element(),
        row(tr!("Frame rate"), None, choice_tabs("replay-fps", &rates, o.fps, cx, |o, v| o.fps = v)).into_any_element(),
        row(
            tr!("Resolution"),
            Some(tr!("A bigger screen is scaled down to it: less work for the graphics card, less RAM and less video memory.")),
            choice_tabs("replay-resolution", &sizes, o.resolution, cx, |o, v| o.resolution = v),
        )
        .into_any_element(),
        row(tr!("Format"), Some(codec_desc), choice_tabs("replay-codec", &codecs, o.codec, cx, |o, v| o.codec = v)).into_any_element(),
    ];
    if let Some(select) = monitor_select(d, o, cx) {
        rows.push(
            row(
                tr!("Monitor"),
                Some(tr!("The game's is the one its window is on (Hyprland), else the first one.")),
                div().w(px(220.)).child(select),
            )
            .into_any_element(),
        );
    }
    card(rows)
}

/// The encoder, where the buffer is kept and the panel's margin: what
/// spares memory and video memory.
fn memory_card(d: &SettingsPage, o: &Options, cx: &mut Context<SettingsPage>) -> Div {
    let info = info_of(&d.replay);
    let (e, encoding, _) = estimate_for(o, &d.replay);
    let encoders: Vec<(Encoder, &'static str)> = Encoder::ALL.iter().map(|x| (*x, encoder_label(*x))).collect();
    let encoder_desc = match (o.encoder, info) {
        (Encoder::Native, _) => tr!("NVENC on NVIDIA, VA-API on AMD and Intel: the most tested."),
        (Encoder::Vulkan, Some(i)) if !i.encodes_vulkan(encoding.codec) => {
            tr!("This graphics card has no Vulkan encoder for this format: its usual one records.")
        }
        (Encoder::Vulkan, _) => tr!(
            "About half the video memory of NVENC on NVIDIA. Experimental in gpu-screen-recorder: if it cannot start, the usual encoder records."
        ),
        (Encoder::Auto, _) => tr!("Vulkan on NVIDIA when the card offers it (about half the video memory), else NVENC or VA-API."),
    };
    let storages: Vec<(Storage, &'static str)> = Storage::ALL.iter().map(|s| (*s, storage_label(*s))).collect();
    let storage_desc: String = match o.storage {
        Storage::Ram => tr!("The buffer stays in memory: the RAM above grows with the length.").into(),
        Storage::Disk => tr!(
            "No memory for the buffer, but about {size} written each hour to the disk of the screenshots folder.",
            size = format_size(e.disk_bytes_per_sec * 3600)
        ),
    };
    let margins: Vec<(u32, &'static str)> = PANEL_MARGINS.iter().map(|m| (*m, margin_label(*m))).collect();
    card([
        row(tr!("Encoder"), Some(encoder_desc), choice_tabs("replay-encoder", &encoders, o.encoder, cx, |o, v| o.encoder = v))
            .into_any_element(),
        row(tr!("Buffer"), Some(&storage_desc), choice_tabs("replay-storage", &storages, o.storage, cx, |o, v| o.storage = v))
            .into_any_element(),
        row(
            tr!("Room for the panel"),
            Some(tr!(
                "A save from the overlay's panel cuts the time it was open off the end: this much more is kept for it. Off: such a replay is shorter by that time."
            )),
            choice_tabs("replay-margin", &margins, o.panel_margin, cx, |o, v| o.panel_margin = v),
        )
        .into_any_element(),
    ])
}

/// What it hears: the game's sound (or everything) and the microphone.
fn sound_card(d: &SettingsPage, o: &Options, cx: &mut Context<SettingsPage>) -> Div {
    let audios: Vec<(Audio, &'static str)> = Audio::ALL.iter().map(|a| (*a, audio_label(*a))).collect();
    let audio_desc = match info_of(&d.replay) {
        Some(i) if !i.app_audio && o.audio == Audio::Game => {
            tr!("This system cannot record one app alone (it needs PipeWire): everything the PC plays is recorded.")
        }
        _ => tr!("The game's: no voice chat, music or notifications."),
    };
    card([
        row(tr!("Sound"), Some(audio_desc), choice_tabs("replay-audio", &audios, o.audio, cx, |o, v| o.audio = v)).into_any_element(),
        toggle_row(
            "replay-mic",
            tr!("Microphone too"),
            Some(tr!("On a track of its own, so a video editor can lower or remove it.")),
            o.microphone,
        )
        .on_click(cx.listener(|d, _, _, cx| update(d, cx, |p| p.options.microphone = !p.options.microphone)))
        .into_any_element(),
    ])
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let p = prefs(d, cx);
    let o = &p.options;
    let seconds: Vec<(u32, &'static str)> = SECONDS.iter().map(|s| (*s, seconds_label(*s))).collect();
    let main = card([
        toggle_row(
            "replay-enabled",
            tr!("Keep a replay of the game"),
            Some(tr!(
                "While a game opened by Heronite runs, its last seconds stay in memory. Guide + X or the overlay's \"Save replay\" saves them in the game's screenshots folder, under replays, and marks them as a moment in its journal."
            )),
            p.enabled,
        )
        .on_click(cx.listener(|d, _, _, cx| update(d, cx, |p| p.enabled = !p.enabled)))
        .into_any_element(),
        row(
            tr!("Length saved"),
            Some(tr!("A longer replay takes more memory while you play.")),
            choice_tabs("replay-seconds", &seconds, o.seconds, cx, |o, v| o.seconds = v),
        )
        .into_any_element(),
    ]);
    let per_game = match p.games.len() {
        0 => tr!("A game can have it on or off on its own: Game settings → Performance.").to_string(),
        n => heronite_i18n::trn!(
            n,
            "{n} game has it on or off on its own (Game settings → Performance).",
            "{n} games have it on or off on its own (Game settings → Performance)."
        ),
    };
    let video = video_card(d, o, cx);
    let memory = memory_card(d, o, cx);
    let sound = sound_card(d, o, cx);

    let detected = d.replay.detected.as_ref();
    let (program, program_ok) = program_text(detected);
    let (now, now_ok) = status_text(&d.replay, cx);
    let has_folder = !d.settings.screenshots.folder.trim().is_empty();
    let mut status_rows = vec![row("gpu-screen-recorder", None, status_line(program, program_ok)).into_any_element()];
    if let Some((text, ok)) = encoder_text(detected) {
        status_rows.push(row(tr!("Video encoder"), None, status_line(text, ok)).into_any_element());
    }
    status_rows.push(
        row(
            tr!("Saved in"),
            None,
            status_line(
                if has_folder {
                    tr!("{folder}/<game>/replays", folder = d.settings.screenshots.folder.trim().trim_end_matches('/'))
                } else {
                    tr!("Choose a screenshots folder in Settings → Screenshots").into()
                },
                has_folder,
            ),
        )
        .into_any_element(),
    );
    status_rows.push(row(tr!("Now"), None, status_line(now, now_ok)).into_any_element());
    let status = card(status_rows);

    let notes = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .px(px(6.))
        .child(meta_text(tr!(
            "It needs gpu-screen-recorder 6.0 or newer, from your distribution (Arch: pacman -S gpu-screen-recorder) or Flathub (com.dec05eba.gpu_screen_recorder)."
        )))
        .child(meta_text(tr!(
            "The graphics card encodes the video with a part of its own, and the whole monitor is captured straight from the display. The game still shares the card's memory with it: that is the video memory above."
        )))
        .child(meta_text(tr!("What is on the monitor is in the replay, notifications and the overlay's notices included.")));

    let dim = |enabled: bool, el: Div| div().when(!enabled, |d| d.opacity(0.5)).child(el);
    let on = p.any_enabled();
    div()
        .flex()
        .flex_col()
        .child(caption(tr!("Replay")))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(main)
                .child(dim(on, estimate_card(o, &d.replay)))
                .child(div().px(px(6.)).child(meta_text(per_game))),
        )
        .child(caption(tr!("Controller")))
        .child(dim(on, overlay::replay_shortcut_card(d, cx)))
        .child(caption(tr!("Video")))
        .child(dim(on, video))
        .child(caption(tr!("Memory")))
        .child(dim(on, memory))
        .child(caption(tr!("Sound")))
        .child(dim(on, sound))
        .child(caption(tr!("Status")))
        .child(status)
        .child(caption(tr!("Good to know")))
        .child(notes)
        .into_any_element()
}

/// The RAM line for console mode.
fn estimate_line(o: &Options, st: &State) -> String {
    let (e, _, _) = estimate_for(o, st);
    tr!("About {ram} of RAM and {vram} of video memory while a game runs", ram = format_size(e.ram_bytes), vram = format_size(e.vram_bytes))
}

/// Console mode rows.
pub(super) fn console_rows(d: &SettingsPage, cx: &App) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    let p = prefs(d, cx);
    let o = &p.options;
    let st = &d.replay;
    let (program, _) = program_text(st.detected.as_ref());
    let (now, _) = status_text(st, cx);
    let (e, _, _) = estimate_for(o, st);
    let index = |len: usize, pos: Option<usize>| pos.unwrap_or(0).min(len.saturating_sub(1));
    let choice = |id: &'static str, label: &'static str, names: Vec<&'static str>, pos: Option<usize>| {
        let len = names.len();
        Row::choice(id, label, names.into_iter().map(SharedString::from).collect(), index(len, pos))
    };
    let mut rows = vec![
        Row::toggle("enabled", tr!("Keep a replay of the game"), p.enabled).desc(tr!("Guide + X saves the last seconds as a moment")),
        choice(
            "seconds",
            tr!("Length saved"),
            SECONDS.iter().map(|s| seconds_label(*s)).collect(),
            SECONDS.iter().position(|s| *s == o.seconds),
        )
        .desc(estimate_line(o, st)),
        Row::info("disk", tr!("Each replay on disk"), format_size(e.file_bytes)),
    ];
    rows.extend(overlay::console_replay_rows(d));
    rows.extend([
        Row::header(tr!("Video")),
        choice(
            "quality",
            tr!("Quality"),
            Quality::ALL.iter().map(|q| quality_label(*q)).collect(),
            Quality::ALL.iter().position(|q| *q == o.quality),
        ),
        choice("fps", tr!("Frame rate"), FPS.iter().map(|f| fps_label(*f)).collect(), FPS.iter().position(|f| *f == o.fps)),
        choice(
            "resolution",
            tr!("Resolution"),
            Resolution::ALL.iter().map(|r| resolution_label(*r)).collect(),
            Resolution::ALL.iter().position(|r| *r == o.resolution),
        ),
        choice("codec", tr!("Format"), Codec::ALL.iter().map(|c| codec_label(*c)).collect(), Codec::ALL.iter().position(|c| *c == o.codec)),
        Row::header(tr!("Memory")),
        choice(
            "encoder",
            tr!("Encoder"),
            Encoder::ALL.iter().map(|x| encoder_label(*x)).collect(),
            Encoder::ALL.iter().position(|x| *x == o.encoder),
        ),
        choice(
            "storage",
            tr!("Buffer"),
            Storage::ALL.iter().map(|s| storage_label(*s)).collect(),
            Storage::ALL.iter().position(|s| *s == o.storage),
        ),
        choice(
            "margin",
            tr!("Room for the panel"),
            PANEL_MARGINS.iter().map(|m| margin_label(*m)).collect(),
            PANEL_MARGINS.iter().position(|m| *m == o.panel_margin),
        ),
        Row::header(tr!("Sound")),
        choice("audio", tr!("Sound"), Audio::ALL.iter().map(|a| audio_label(*a)).collect(), Audio::ALL.iter().position(|a| *a == o.audio)),
        Row::toggle("mic", tr!("Microphone too"), o.microphone),
        Row::header(tr!("Status")),
        Row::info("program", "gpu-screen-recorder", program),
        Row::info("now", tr!("Now"), now),
    ]);
    rows
}

pub(super) fn console_apply(d: &mut SettingsPage, outcome: crate::console::list::Outcome, cx: &mut Context<SettingsPage>) {
    use crate::console::list::{Outcome, Value};
    if overlay::console_apply_replay(d, &outcome, cx) {
        return;
    }
    let set =
        |d: &mut SettingsPage, cx: &mut Context<SettingsPage>, change: &dyn Fn(&mut Options)| update(d, cx, |p| change(&mut p.options));
    match outcome {
        Outcome::Changed(id, Value::Toggle(on)) => match id.as_ref() {
            "enabled" => update(d, cx, |p| p.enabled = on),
            "mic" => set(d, cx, &|o| o.microphone = on),
            _ => {}
        },
        Outcome::Changed(id, Value::Choice { index: i, .. }) => match id.as_ref() {
            "seconds" => set(d, cx, &|o| o.seconds = SECONDS.get(i).copied().unwrap_or(30)),
            "quality" => set(d, cx, &|o| o.quality = Quality::ALL.get(i).copied().unwrap_or(Quality::Balanced)),
            "fps" => set(d, cx, &|o| o.fps = FPS.get(i).copied().unwrap_or(60)),
            "resolution" => set(d, cx, &|o| o.resolution = Resolution::ALL.get(i).copied().unwrap_or(Resolution::P1440)),
            "codec" => set(d, cx, &|o| o.codec = Codec::ALL.get(i).copied().unwrap_or(Codec::Hevc)),
            "encoder" => set(d, cx, &|o| o.encoder = Encoder::ALL.get(i).copied().unwrap_or(Encoder::Auto)),
            "storage" => set(d, cx, &|o| o.storage = Storage::ALL.get(i).copied().unwrap_or(Storage::Ram)),
            "margin" => set(d, cx, &|o| o.panel_margin = PANEL_MARGINS.get(i).copied().unwrap_or(5)),
            "audio" => set(d, cx, &|o| o.audio = Audio::ALL.get(i).copied().unwrap_or(Audio::Game)),
            _ => {}
        },
        _ => {}
    }
}
