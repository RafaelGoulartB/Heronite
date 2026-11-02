//! Settings → Replay: the replay buffer (off by default), its length, the
//! video and sound it keeps, and what that costs: the memory it takes while
//! a game runs, worked out from the screen, the frame rate and the bitrate
//! (`heronite_overlay::replay::estimate`), and how to spare it (the encoder,
//! the buffer on disk, the panel's margin). The options live in
//! `replay.json` (`heronite_data::replay_prefs`), are the same for every
//! game and reach the running service at once; a game can only have it on or
//! off on its own (Game settings → Performance). Guide + X is the overlay's
//! (`overlay::replay_shortcut_row`); the keyboard shortcut, none unless set,
//! is registered by the service while a game records.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::replay_prefs::{Audio, Codec, Encoder, Options, Quality, ReplayPrefs, Resolution, Storage, FPS, PANEL_MARGINS, SECONDS};
use heronite_i18n::tr;
use heronite_overlay::replay::plan::buffer_seconds;
use heronite_overlay::replay::{estimate, Encoding, Estimate, Info, Program};

use super::screenshots::format_size;
use super::{overlay, SettingsPage};
use crate::icons::Icon;
use crate::overlay::HotkeyState;
use crate::pages::PageCtx;
use crate::services::replay::{Detected, Status};
use crate::theme;
use crate::ui::controls::{
    button, caption, card, card_box, icon_button_sm, info_tip, row, tabs, toggle_row, toggle_row_info, with_sub_rows, ButtonKind, CAPTION_H,
};
use crate::ui::fields::{accelerator_from_key, hotkey_box, labeled, meta_text, select_field};
use crate::ui::icon;

/// Width of the column beside the options: the status and the cost.
const SIDE_W: f32 = 250.;

/// The section's width: the options and the column beside them.
pub const MAX_W: f32 = 1080.;

/// Screen the estimate assumes until the recorder says which ones there are.
const ASSUMED_SCREEN: (u32, u32) = (1920, 1080);

pub struct State {
    /// `None` until it is looked for: once per visit of Settings (it wakes
    /// the graphics card), or what the recorder found while a game runs.
    detected: Option<Detected>,
    looking: bool,
    /// The menu of the monitor is open.
    monitor_open: bool,
    /// Show advanced is open.
    advanced: bool,
    /// The keyboard shortcut field.
    hotkey: FocusHandle,
    /// The combination last pressed in it is another shortcut's.
    hotkey_taken: Option<String>,
}

impl State {
    pub fn new(_ctx: &PageCtx, cx: &mut Context<SettingsPage>) -> Self {
        // Its options and status (recording, unavailable) change in the service.
        if let Some(service) = crate::services::replay::get(cx) {
            cx.observe(&service, |_, _, cx| cx.notify()).detach();
        }
        Self { detected: None, looking: false, monitor_open: false, advanced: false, hotkey: cx.focus_handle(), hotkey_taken: None }
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

/// Under the main switch while gpu-screen-recorder is missing or too old:
/// what is wrong, where to get it, and Flathub.
fn missing_recorder(program: String) -> Div {
    div()
        .py(px(12.))
        .flex()
        .flex_col()
        .items_start()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(13.5))
                .text_color(theme::warning())
                .child(icon(Icon::Warning, px(16.), theme::warning()))
                .child(format!("gpu-screen-recorder: {program}")),
        )
        .child(meta_text(tr!(
            "It needs gpu-screen-recorder 6.0 or newer, from your distribution (Arch: pacman -S gpu-screen-recorder) or Flathub (com.dec05eba.gpu_screen_recorder)."
        )))
        .child(icon_button_sm("replay-flathub", ButtonKind::Glass, Icon::OpenInNew, tr!("Get it on Flathub"), true, |_, _, cx| {
            cx.open_url("https://flathub.org/apps/com.dec05eba.gpu_screen_recorder")
        }))
}

/// A card of the column beside the options: its title and `body`.
fn side_card(title: &'static str, body: Div) -> Div {
    card_box()
        .py(px(14.))
        .gap(px(12.))
        .child(div().text_size(px(12.)).font_weight(theme::W_BOLD).text_color(theme::text_muted()).child(title))
        .child(body)
}

/// One cost of the replay and how big it is next to the most it can take
/// (`max`, the same option at its largest), as a bar.
fn cost_bar(label: &'static str, bytes: u64, max: u64) -> Div {
    let share = if max == 0 { 0. } else { (bytes as f32 / max as f32).clamp(0.03, 1.) };
    div()
        .flex()
        .flex_col()
        .gap(px(5.))
        .child(
            div()
                .flex()
                .justify_between()
                .gap(px(8.))
                .text_size(px(12.5))
                .child(div().text_color(theme::text_muted()).child(label))
                .child(div().font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(format_size(bytes))),
        )
        .child(
            div()
                .h(px(6.))
                .w_full()
                .rounded_full()
                .bg(theme::ink_a(0.08))
                .child(div().h_full().w(relative(share)).rounded_full().bg(theme::accent())),
        )
}

/// What the replay costs while a game runs: its RAM (against the longest
/// length), its video memory (against the screen's resolution) and each
/// replay on disk (against the longest length), with how it was worked out
/// in the (i).
fn cost_card(o: &Options, st: &State) -> Div {
    let (e, encoding, name) = estimate_for(o, st);
    let longest = Options { seconds: SECONDS[SECONDS.len() - 1], ..o.clone() };
    let (e_long, _, _) = estimate_for(&longest, st);
    let (e_native, _, _) = estimate_for(&Options { resolution: Resolution::Native, ..o.clone() }, st);
    let video = tr!(
        "{w} × {h} at {fps} FPS, {mbps} Mbit/s {codec} through {encoder}",
        w = e.width,
        h = e.height,
        fps = o.fps,
        mbps = heronite_i18n::decimal(f64::from(e.video_kbps) / 1000., 1),
        codec = codec_label(encoding.codec),
        encoder = encoder_name(encoding.vulkan, info_of(st))
    );
    let mut how = vec![
        match name {
            Some(n) => tr!("For {name}: {video}.", name = n, video = video),
            None => tr!("For a 1920 × 1080 screen: {video}.", video = video),
        },
        if o.panel_margin > 0 {
            tr!(
                "It keeps {kept} s: the {n} s saved and {margin} s for a save from the panel, whose time is cut off.",
                kept = buffer_seconds(o),
                n = o.seconds,
                margin = o.panel_margin
            )
        } else {
            tr!("It keeps the {n} s saved.", n = o.seconds)
        },
    ];
    if e.disk_bytes_per_sec > 0 {
        how.push(tr!("It writes about {size} an hour to disk while a game runs.", size = format_size(e.disk_bytes_per_sec * 3600)));
    }
    how.push(
        tr!(
            "The graphics card encodes the video with a part of its own, and the whole monitor is captured straight from the display. The game still shares the card's memory with it: that is the video memory above."
        )
        .to_string(),
    );
    let body = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(cost_bar(tr!("Memory (RAM)"), e.ram_bytes, e_long.ram_bytes.max(e.ram_bytes)))
        .child(cost_bar(tr!("Video memory (VRAM)"), e.vram_bytes, e_native.vram_bytes.max(e.vram_bytes)))
        .child(cost_bar(tr!("Each replay on disk"), e.file_bytes, e_long.file_bytes.max(e.file_bytes)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(info_tip("replay-cost-info", &how.join("\n\n")))
                .child(meta_text(tr!("How it is worked out"))),
        );
    side_card(tr!("While a game runs"), body)
}

/// One line of the status card: a check (or a warning), what it is and how
/// it is.
fn status_item(ok: bool, title: &'static str, value: String, tip: Option<(&'static str, &'static str)>) -> Div {
    let mark = if ok { icon(Icon::Check, px(15.), theme::success()) } else { icon(Icon::Warning, px(15.), theme::warning()) };
    div().flex().items_start().gap(px(8.)).child(div().mt(px(1.)).child(mark)).child(
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_size(px(13.))
                    .text_color(theme::text_primary())
                    .child(title)
                    .children(tip.map(|(id, text)| info_tip(id, text))),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .line_height(px(17.))
                    .text_color(if ok { theme::text_dim() } else { theme::warning() })
                    .child(value),
            ),
    )
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

/// What most people change: the picture's quality, the sound and the microphone.
fn picture_sound_card(d: &SettingsPage, o: &Options, cx: &mut Context<SettingsPage>) -> Div {
    let qualities: Vec<(Quality, &'static str)> = Quality::ALL.iter().map(|q| (*q, quality_label(*q))).collect();
    let audios: Vec<(Audio, &'static str)> = Audio::ALL.iter().map(|a| (*a, audio_label(*a))).collect();
    let audio_desc = match info_of(&d.replay) {
        Some(i) if !i.app_audio && o.audio == Audio::Game => {
            tr!("This system cannot record one app alone (it needs PipeWire): everything the PC plays is recorded.")
        }
        _ => tr!("The game's: no voice chat, music or notifications."),
    };
    card([
        row(
            tr!("Quality"),
            Some(tr!("More bits for each frame: a sharper picture, more memory.")),
            choice_tabs("replay-quality", &qualities, o.quality, cx, |o, v| o.quality = v),
        )
        .into_any_element(),
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

/// The rest, under Show advanced: frame rate, resolution, format and
/// monitor, then what spares memory and video memory (the encoder, where the
/// buffer is kept, the panel's margin).
fn advanced_card(d: &SettingsPage, o: &Options, cx: &mut Context<SettingsPage>) -> Div {
    let info = info_of(&d.replay);
    let rates: Vec<(u32, &'static str)> = FPS.iter().map(|f| (*f, fps_label(*f))).collect();
    let sizes: Vec<(Resolution, &'static str)> = Resolution::ALL.iter().map(|r| (*r, resolution_label(*r))).collect();
    let codecs: Vec<(Codec, &'static str)> = Codec::ALL.iter().map(|c| (*c, codec_label(*c))).collect();
    let codec_desc = match info {
        Some(i) if !i.encodes(o.codec) => tr!("This graphics card cannot encode it: H.264 is used instead."),
        _ => tr!("H.264 plays everywhere. HEVC and AV1 look the same in less memory and space."),
    };
    let mut rows = vec![
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
        Storage::Ram => tr!("The buffer stays in memory: its RAM grows with the length.").into(),
        Storage::Disk => tr!(
            "No memory for the buffer, but about {size} written each hour to the disk of the screenshots folder.",
            size = format_size(e.disk_bytes_per_sec * 3600)
        ),
    };
    let margins: Vec<(u32, &'static str)> = PANEL_MARGINS.iter().map(|m| (*m, margin_label(*m))).collect();
    rows.extend([
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
    ]);
    card(rows)
}

/// The settings [`advanced_card`] holds (`search::INDEX` labels).
const ADVANCED: [&str; 7] = ["Frame rate", "Resolution", "Format", "Monitor", "Encoder", "Buffer", "Room for the panel"];

/// The search jumped to `label`: open Show advanced when it is in there.
pub fn reveal(d: &mut SettingsPage, label: &str) {
    if ADVANCED.contains(&label) {
        d.replay.advanced = true;
    }
}

/// "Show advanced", and [`advanced_card`] under it once open.
fn advanced_part(d: &SettingsPage, o: &Options, cx: &mut Context<SettingsPage>) -> Div {
    let open = d.replay.advanced;
    let toggle = div()
        .id("replay-advanced")
        .flex()
        .items_center()
        .gap(px(6.))
        .mt(px(16.))
        .mx(px(6.))
        .mb(px(8.))
        .cursor_pointer()
        .text_size(px(13.))
        .text_color(theme::text_muted())
        .hover(|s| s.text_color(theme::text_primary()))
        .child(icon(if open { Icon::ExpandMore } else { Icon::ChevronRight }, px(16.), theme::text_muted()))
        .child(if open { tr!("Hide advanced") } else { tr!("Show advanced") })
        .on_click(cx.listener(|d, _, _, cx| {
            d.replay.advanced = !d.replay.advanced;
            d.replay.monitor_open = false;
            cx.notify();
        }));
    div().flex().flex_col().child(toggle).when(open, |c| c.child(advanced_card(d, o, cx)))
}

/// `CommandOrControl` as the keyboard says it.
fn shown_accelerator(a: &str) -> String {
    a.replace("CommandOrControl", "Ctrl")
}

/// What else uses `accelerator` (the overlay's or the screenshot's shortcut).
fn taken_by(d: &SettingsPage, accelerator: &str) -> Option<String> {
    let o = &d.overlay.prefs;
    let s = &d.settings.screenshots;
    if o.enabled && o.keyboard_shortcut && o.keyboard_accelerator.eq_ignore_ascii_case(accelerator) {
        Some(tr!("{key} already opens the overlay.", key = shown_accelerator(accelerator)))
    } else if s.capture_enabled && s.capture_accelerator.eq_ignore_ascii_case(accelerator) {
        Some(tr!("{key} already takes a screenshot.", key = shown_accelerator(accelerator)))
    } else {
        None
    }
}

/// The keyboard shortcut that saves the replay: a field (none unless set;
/// Backspace or Delete clears it) and whether it is registered.
fn keyboard_row(d: &SettingsPage, p: &ReplayPrefs, window: &Window, cx: &mut Context<SettingsPage>) -> Div {
    let accelerator = p.keyboard_accelerator.trim();
    let focused = d.replay.hotkey.is_focused(window);
    let text = match (accelerator.is_empty(), focused) {
        (false, _) => shown_accelerator(accelerator),
        (true, true) => tr!("Press the keys…").into(),
        (true, false) => tr!("None").into(),
    };
    let field = hotkey_box(
        div().id("replay-hotkey").track_focus(&d.replay.hotkey).cursor_pointer().on_key_down(cx.listener(|d, ev: &KeyDownEvent, _, cx| {
            cx.stop_propagation();
            let k = &ev.keystroke;
            let plain = !(k.modifiers.control || k.modifiers.alt || k.modifiers.shift || k.modifiers.platform);
            let accelerator = if plain && matches!(k.key.as_str(), "backspace" | "delete") {
                String::new()
            } else {
                let Some(a) = accelerator_from_key(ev) else { return };
                a
            };
            d.replay.hotkey_taken = taken_by(d, &accelerator);
            if d.replay.hotkey_taken.is_none() {
                update(d, cx, |p| p.keyboard_accelerator = accelerator);
            }
            cx.notify();
        })),
        text,
        focused,
    )
    .when(accelerator.is_empty() && !focused, |b| b.text_color(theme::text_dim()));
    let clear = (!accelerator.is_empty()).then(|| {
        button("replay-hotkey-clear", ButtonKind::Glass, tr!("Remove")).on_click(cx.listener(|d, _, _, cx| {
            d.replay.hotkey_taken = None;
            update(d, cx, |p| p.keyboard_accelerator.clear());
        }))
    });
    let status: Option<(String, bool)> = match &d.replay.hotkey_taken {
        Some(why) => Some((why.clone(), true)),
        None if accelerator.is_empty() => None,
        None => {
            let state = crate::services::replay::get(cx).map(|s| s.read(cx).hotkey_state().clone()).unwrap_or_default();
            Some(match state {
                HotkeyState::Idle => (tr!("It is registered while a game records, so the keys stay the game's otherwise.").into(), false),
                HotkeyState::Registering => (tr!("Registering the shortcut…").into(), false),
                HotkeyState::Ready(_) => (tr!("Shortcut registered and ready.").into(), false),
                HotkeyState::Failed(e) => (e, true),
            })
        }
    };
    div()
        .py(px(12.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(labeled(
            tr!("Keyboard shortcut"),
            div().flex().items_center().gap(px(8.)).child(div().flex_1().child(field)).children(clear),
        ))
        .child(meta_text(tr!(
            "Saves the replay like Guide + X, overlay or not. Click the field and press the key combination; Backspace removes it."
        )))
        .children(status.map(|(text, error)| {
            div().text_size(px(12.)).text_color(if error { theme::danger_text() } else { theme::success() }).child(text)
        }))
}

pub fn render(d: &mut SettingsPage, window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let p = prefs(d, cx);
    let o = &p.options;
    let seconds: Vec<(u32, &'static str)> = SECONDS.iter().map(|s| (*s, seconds_label(*s))).collect();
    let mut main_rows = vec![with_sub_rows(
        toggle_row_info(
            "replay-enabled",
            tr!("Keep a replay of the game"),
            tr!("The last seconds stay in memory; save them when something good happens."),
            &format!(
                "{} {}",
                tr!(
                    "While a game opened by Heronite runs, its last seconds stay in memory. Guide + X or the overlay's \"Save replay\" saves them in the game's screenshots folder, under replays, and marks them as a moment in its journal."
                ),
                tr!("What is on the monitor is in the replay, notifications and the overlay's notices included.")
            ),
            p.enabled,
        )
        .on_click(cx.listener(|d, _, _, cx| update(d, cx, |p| p.enabled = !p.enabled)))
        .into_any_element(),
        vec![row(
            tr!("Length saved"),
            Some(tr!("A longer replay takes more memory while you play.")),
            choice_tabs("replay-seconds", &seconds, o.seconds, cx, |o, v| o.seconds = v),
        )
        .into_any_element()],
    )];
    // Missing or too old (not while it is still looked for).
    let (program, program_ok) = program_text(d.replay.detected.as_ref());
    if !program_ok {
        main_rows.push(missing_recorder(program).into_any_element());
    }
    let main = card(main_rows);
    let per_game = match p.games.len() {
        0 => tr!("A game can have it on or off on its own: Game settings → Performance.").to_string(),
        n => heronite_i18n::trn!(
            n,
            "{n} game has it on or off on its own (Game settings → Performance).",
            "{n} games have it on or off on its own (Game settings → Performance)."
        ),
    };
    // Both ways to save it: Guide + X and the keyboard shortcut.
    let saving = card([overlay::replay_shortcut_row(d, cx), keyboard_row(d, &p, window, cx).into_any_element()]);
    let picture_sound = picture_sound_card(d, o, cx);
    let advanced = advanced_part(d, o, cx);

    let detected = d.replay.detected.as_ref();
    let (program, program_ok) = program_text(detected);
    let (now, now_ok) = status_text(&d.replay, cx);
    let has_folder = !d.settings.screenshots.folder.trim().is_empty();
    let mut status = div().flex().flex_col().gap(px(10.)).child(status_item(
        program_ok,
        "gpu-screen-recorder",
        program,
        Some((
            "replay-program-info",
            tr!(
                "It needs gpu-screen-recorder 6.0 or newer, from your distribution (Arch: pacman -S gpu-screen-recorder) or Flathub (com.dec05eba.gpu_screen_recorder)."
            ),
        )),
    ));
    if let Some((text, ok)) = encoder_text(detected) {
        status = status.child(status_item(ok, tr!("Video encoder"), text, None));
    }
    let saved_in = if has_folder {
        tr!("{folder}/<game>/replays", folder = d.settings.screenshots.folder.trim().trim_end_matches('/'))
    } else {
        tr!("Choose a screenshots folder in Settings → Screenshots").into()
    };
    let status = status.child(status_item(has_folder, tr!("Saved in"), saved_in, None)).child(status_item(now_ok, tr!("Now"), now, None));
    let side = div()
        .flex()
        .flex_col()
        .flex_none()
        .w(px(SIDE_W))
        .gap(px(12.))
        // Level with the first card, under its caption.
        .mt(px(CAPTION_H + 20.))
        .child(side_card(tr!("Status"), status))
        .child(div().when(!p.any_enabled(), |d| d.opacity(0.5)).child(cost_card(o, &d.replay)));

    let dim = |enabled: bool, el: Div| div().when(!enabled, |d| d.opacity(0.5)).child(el);
    let on = p.any_enabled();
    let column = div()
        .flex()
        .flex_col()
        .child(caption(tr!("Replay")))
        .child(main)
        .child(caption(tr!("Saving it")))
        .child(dim(on, saving))
        .child(caption(tr!("Picture and sound")))
        .child(dim(on, picture_sound))
        .child(dim(on, advanced))
        .child(div().mt(px(14.)).px(px(6.)).child(meta_text(per_game)));
    div().flex().items_start().gap(px(18.)).child(column.flex_1().min_w_0()).child(side).into_any_element()
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
