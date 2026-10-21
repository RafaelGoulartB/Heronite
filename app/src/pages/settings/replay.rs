//! Settings → Replay: the replay buffer (off by default), its length, the
//! video and sound it keeps, and what that costs: the memory it takes while
//! a game runs, worked out from the screen, the frame rate and the bitrate
//! (`heronite_overlay::replay::estimate`). The options live in
//! `replay.json` (`heronite_data::replay_prefs`) and reach the running
//! service at once; Guide + X is the overlay's (`overlay::replay_shortcut_card`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::replay_prefs::{Audio, Codec, Quality, ReplayPrefs, Resolution, FPS, SECONDS};
use heronite_i18n::tr;
use heronite_overlay::replay::{estimate, Estimate, Program};

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
    prefs: ReplayPrefs,
    /// `None` while it is looked for.
    detected: Option<Detected>,
    /// The menu of the monitor is open.
    monitor_open: bool,
}

impl State {
    pub fn new(ctx: &PageCtx, cx: &mut Context<SettingsPage>) -> Self {
        // Its status (recording, unavailable) changes in the service.
        if let Some(service) = crate::services::replay::get(cx) {
            cx.observe(&service, |_, _, cx| cx.notify()).detach();
        }
        // A tiny file in the app's config folder.
        Self { prefs: ReplayPrefs::load(&ctx.app), detected: None, monitor_open: false }
    }
}

/// Looks for gpu-screen-recorder and what it can do when the section shows.
pub fn shown(cx: &mut Context<SettingsPage>) {
    cx.spawn(async move |this, cx| {
        let detected = cx.background_spawn(async { crate::services::replay::detect() }).await;
        this.update(cx, |d, cx| {
            d.replay.detected = Some(detected);
            cx.notify();
        })
        .ok();
    })
    .detach();
}

/// Changes the options, saves them (off the UI thread) and hands them to the
/// service.
fn update(d: &mut SettingsPage, cx: &mut Context<SettingsPage>, change: impl FnOnce(&mut ReplayPrefs)) {
    change(&mut d.replay.prefs);
    let prefs = d.replay.prefs.clone();
    let app = d.ctx.app.clone();
    let saved = prefs.clone();
    cx.background_spawn(async move {
        if let Err(e) = saved.save(&app) {
            log::warn!("saving the replay options: {e:#}");
        }
    })
    .detach();
    if let Some(service) = crate::services::replay::get(cx) {
        service.update(cx, |s, cx| s.set_prefs(prefs, cx));
    }
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

/// What the estimate is for: the monitor chosen, else the largest one (the
/// game may be on it), else a 1080p screen until they are known.
fn estimate_for(st: &State) -> (Estimate, bool, Option<String>) {
    let info = st.detected.as_ref().and_then(|d| d.info.as_ref().ok());
    let screens = info.map(|i| i.screens.as_slice()).unwrap_or_default();
    let screen = screens
        .iter()
        .find(|s| s.name == st.prefs.monitor)
        .or_else(|| screens.iter().max_by_key(|s| u64::from(s.width) * u64::from(s.height)));
    let (w, h) = screen.map_or(ASSUMED_SCREEN, |s| (s.width, s.height));
    let codec = info.and_then(|i| i.codec_for(st.prefs.codec)).unwrap_or(st.prefs.codec);
    let vendor = info.map(|i| i.vendor.as_str()).unwrap_or_default();
    (estimate(&st.prefs, codec, w, h, vendor), screen.is_some(), screen.map(|s| s.name.clone()))
}

fn estimate_card(st: &State) -> Div {
    let (e, known, name) = estimate_for(st);
    let p = &st.prefs;
    let codec = st.detected.as_ref().and_then(|d| d.info.as_ref().ok()).and_then(|i| i.codec_for(p.codec)).unwrap_or(p.codec);
    let video = tr!(
        "{w} × {h} at {fps} FPS, {mbps} Mbit/s {codec}",
        w = e.width,
        h = e.height,
        fps = p.fps,
        mbps = heronite_i18n::decimal(f64::from(e.video_kbps) / 1000., 1),
        codec = codec_label(codec)
    );
    let screen = match (known, name) {
        (true, Some(n)) => tr!("For {name}: {video}.", name = n, video = video),
        _ => tr!("For a 1920 × 1080 screen: {video}.", video = video),
    };
    let kept = tr!(
        "It keeps {kept} s: the {n} s saved and {margin} s for a save from the panel, whose time is cut off.",
        kept = heronite_overlay::replay::plan::buffer_seconds(p),
        n = p.seconds,
        margin = heronite_overlay::replay::plan::PANEL_MARGIN
    );
    let stats = div()
        .flex()
        .flex_wrap()
        .gap(px(32.))
        .child(stat(tr!("About {size}", size = format_size(e.ram_bytes)), tr!("Memory (RAM)"), tr!("Grows with the length saved.")))
        .child(stat(
            tr!("About {size}", size = format_size(e.vram_bytes)),
            tr!("Video memory (VRAM)"),
            tr!("Grows with the video's resolution."),
        ));
    let col = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .py(px(12.))
        .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(tr!("Memory while a game runs")))
        .child(stats.mb(px(4.)))
        .child(meta_text(screen))
        .child(meta_text(kept))
        .child(meta_text(tr!("Each replay takes about {size} on disk.", size = format_size(e.file_bytes))));
    card_box().child(col)
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
        (format!("{} \u{b7} {}", info.vendor.to_uppercase(), codecs.join(", ")), true)
    })
}

fn status_text(cx: &App) -> (String, bool) {
    let Some(service) = crate::services::replay::get(cx) else { return (String::new(), true) };
    match service.read(cx).status() {
        Status::Off => (tr!("Off").into(), true),
        Status::Idle => (tr!("Waiting for a game").into(), true),
        Status::Starting => (tr!("Starting…").into(), true),
        Status::Recording { monitor, .. } if monitor.is_empty() => (tr!("Recording").into(), true),
        Status::Recording { monitor, .. } => (tr!("Recording {monitor}", monitor = monitor.clone()), true),
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
    set: impl Fn(&mut ReplayPrefs, T) + 'static,
) -> Div {
    let page = cx.entity().downgrade();
    let set = std::rc::Rc::new(set);
    tabs(id, items, active, move |v, _, cx| {
        let set = set.clone();
        page.update(cx, |d, cx| update(d, cx, |p| set(p, v))).ok();
    })
}

fn monitor_select(d: &SettingsPage, cx: &mut Context<SettingsPage>) -> Option<Div> {
    let info = d.replay.detected.as_ref()?.info.as_ref().ok()?;
    let p = &d.replay.prefs;
    let mut names: Vec<String> = vec![String::new()];
    names.extend(info.screens.iter().map(|s| s.name.clone()));
    let label = |n: &str| -> String {
        match info.screens.iter().find(|s| s.name == n) {
            _ if n.is_empty() => tr!("The game's").into(),
            Some(s) => format!("{} ({} × {})", s.name, s.width, s.height),
            None => n.to_string(),
        }
    };
    let options: Vec<(SharedString, bool)> = names.iter().map(|n| (label(n).into(), *n == p.monitor)).collect();
    let picked = names.clone();
    Some(select_field(
        "replay-monitor",
        None,
        label(&p.monitor),
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
            update(d, cx, |p| p.monitor = name);
        },
    ))
}

pub fn render(d: &mut SettingsPage, _window: &mut Window, cx: &mut Context<SettingsPage>) -> AnyElement {
    let p = d.replay.prefs.clone();
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
            choice_tabs("replay-seconds", &seconds, p.seconds, cx, |p, v| p.seconds = v),
        )
        .into_any_element(),
    ]);

    let qualities: Vec<(Quality, &'static str)> = Quality::ALL.iter().map(|q| (*q, quality_label(*q))).collect();
    let rates: Vec<(u32, &'static str)> = FPS.iter().map(|f| (*f, fps_label(*f))).collect();
    let sizes: Vec<(Resolution, &'static str)> = Resolution::ALL.iter().map(|r| (*r, resolution_label(*r))).collect();
    let codecs: Vec<(Codec, &'static str)> = Codec::ALL.iter().map(|c| (*c, codec_label(*c))).collect();
    let info = d.replay.detected.as_ref().and_then(|d| d.info.as_ref().ok());
    let codec_desc = match info {
        Some(i) if !i.encodes(p.codec) => tr!("This graphics card cannot encode it: H.264 is used instead."),
        _ => tr!("H.264 plays everywhere. HEVC and AV1 look the same in less memory and space."),
    };
    let mut video_rows = vec![
        row(
            tr!("Quality"),
            Some(tr!("More bits for each frame: a sharper picture, more memory.")),
            choice_tabs("replay-quality", &qualities, p.quality, cx, |p, v| p.quality = v),
        )
        .into_any_element(),
        row(tr!("Frame rate"), None, choice_tabs("replay-fps", &rates, p.fps, cx, |p, v| p.fps = v)).into_any_element(),
        row(
            tr!("Resolution"),
            Some(tr!("A bigger screen is scaled down to it: less work for the graphics card, less RAM and less video memory.")),
            choice_tabs("replay-resolution", &sizes, p.resolution, cx, |p, v| p.resolution = v),
        )
        .into_any_element(),
        row(tr!("Format"), Some(codec_desc), choice_tabs("replay-codec", &codecs, p.codec, cx, |p, v| p.codec = v)).into_any_element(),
    ];
    if let Some(select) = monitor_select(d, cx) {
        video_rows.push(
            row(
                tr!("Monitor"),
                Some(tr!("The game's is the one its window is on (Hyprland), else the first one.")),
                div().w(px(220.)).child(select),
            )
            .into_any_element(),
        );
    }
    let video = card(video_rows);

    let audios: Vec<(Audio, &'static str)> = Audio::ALL.iter().map(|a| (*a, audio_label(*a))).collect();
    let audio_desc = match info {
        Some(i) if !i.app_audio && p.audio == Audio::Game => {
            tr!("This system cannot record one app alone (it needs PipeWire): everything the PC plays is recorded.")
        }
        _ => tr!("The game's: no voice chat, music or notifications."),
    };
    let sound = card([
        row(tr!("Sound"), Some(audio_desc), choice_tabs("replay-audio", &audios, p.audio, cx, |p, v| p.audio = v)).into_any_element(),
        toggle_row(
            "replay-mic",
            tr!("Microphone too"),
            Some(tr!("On a track of its own, so a video editor can lower or remove it.")),
            p.microphone,
        )
        .on_click(cx.listener(|d, _, _, cx| update(d, cx, |p| p.microphone = !p.microphone)))
        .into_any_element(),
    ]);

    let detected = d.replay.detected.as_ref();
    let (program, program_ok) = program_text(detected);
    let (now, now_ok) = status_text(cx);
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
            "It records with the graphics card's video encoder, a part of the card the game does not use, and the whole monitor straight from the display: the cost is mostly the memory above."
        )))
        .child(meta_text(tr!("What is on the monitor is in the replay, notifications and the overlay's notices included.")));

    let dim = |enabled: bool, el: Div| div().when(!enabled, |d| d.opacity(0.5)).child(el);
    div()
        .flex()
        .flex_col()
        .child(caption(tr!("Replay")))
        .child(div().flex().flex_col().gap(px(12.)).child(main).child(dim(p.enabled, estimate_card(&d.replay))))
        .child(caption(tr!("Controller")))
        .child(dim(p.enabled, overlay::replay_shortcut_card(d, cx)))
        .child(caption(tr!("Video")))
        .child(dim(p.enabled, video))
        .child(caption(tr!("Sound")))
        .child(dim(p.enabled, sound))
        .child(caption(tr!("Status")))
        .child(status)
        .child(caption(tr!("Good to know")))
        .child(notes)
        .into_any_element()
}

/// The RAM line for console mode.
fn estimate_line(st: &State) -> String {
    let (e, _, _) = estimate_for(st);
    tr!("About {ram} of RAM and {vram} of video memory while a game runs", ram = format_size(e.ram_bytes), vram = format_size(e.vram_bytes))
}

/// Console mode rows.
pub(super) fn console_rows(d: &SettingsPage, cx: &App) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    let p = &d.replay.prefs;
    let index = |len: usize, pos: Option<usize>| pos.unwrap_or(0).min(len.saturating_sub(1));
    let (program, _) = program_text(d.replay.detected.as_ref());
    let (now, _) = status_text(cx);
    let (e, _, _) = estimate_for(&d.replay);
    let mut rows = vec![
        Row::toggle("enabled", tr!("Keep a replay of the game"), p.enabled).desc(tr!("Guide + X saves the last seconds as a moment")),
        Row::choice(
            "seconds",
            tr!("Length saved"),
            SECONDS.iter().map(|s| SharedString::from(seconds_label(*s))).collect(),
            index(SECONDS.len(), SECONDS.iter().position(|s| *s == p.seconds)),
        )
        .desc(estimate_line(&d.replay)),
        Row::info("disk", tr!("Each replay on disk"), format_size(e.file_bytes)),
    ];
    rows.extend(overlay::console_replay_rows(d));
    rows.extend([
        Row::header(tr!("Video")),
        Row::choice(
            "quality",
            tr!("Quality"),
            Quality::ALL.iter().map(|q| SharedString::from(quality_label(*q))).collect(),
            index(Quality::ALL.len(), Quality::ALL.iter().position(|q| *q == p.quality)),
        ),
        Row::choice(
            "fps",
            tr!("Frame rate"),
            FPS.iter().map(|f| SharedString::from(fps_label(*f))).collect(),
            index(FPS.len(), FPS.iter().position(|f| *f == p.fps)),
        ),
        Row::choice(
            "resolution",
            tr!("Resolution"),
            Resolution::ALL.iter().map(|r| SharedString::from(resolution_label(*r))).collect(),
            index(Resolution::ALL.len(), Resolution::ALL.iter().position(|r| *r == p.resolution)),
        ),
        Row::choice(
            "codec",
            tr!("Format"),
            Codec::ALL.iter().map(|c| SharedString::from(codec_label(*c))).collect(),
            index(Codec::ALL.len(), Codec::ALL.iter().position(|c| *c == p.codec)),
        ),
        Row::header(tr!("Sound")),
        Row::choice(
            "audio",
            tr!("Sound"),
            Audio::ALL.iter().map(|a| SharedString::from(audio_label(*a))).collect(),
            index(Audio::ALL.len(), Audio::ALL.iter().position(|a| *a == p.audio)),
        ),
        Row::toggle("mic", tr!("Microphone too"), p.microphone),
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
    match outcome {
        Outcome::Changed(id, Value::Toggle(on)) => match id.as_ref() {
            "enabled" => update(d, cx, |p| p.enabled = on),
            "mic" => update(d, cx, |p| p.microphone = on),
            _ => {}
        },
        Outcome::Changed(id, Value::Choice { index, .. }) => match id.as_ref() {
            "seconds" => update(d, cx, |p| p.seconds = SECONDS.get(index).copied().unwrap_or(30)),
            "quality" => update(d, cx, |p| p.quality = Quality::ALL.get(index).copied().unwrap_or(Quality::Balanced)),
            "fps" => update(d, cx, |p| p.fps = FPS.get(index).copied().unwrap_or(60)),
            "resolution" => update(d, cx, |p| p.resolution = Resolution::ALL.get(index).copied().unwrap_or(Resolution::Native)),
            "codec" => update(d, cx, |p| p.codec = Codec::ALL.get(index).copied().unwrap_or(Codec::H264)),
            "audio" => update(d, cx, |p| p.audio = Audio::ALL.get(index).copied().unwrap_or(Audio::Game)),
            _ => {}
        },
        _ => {}
    }
}
