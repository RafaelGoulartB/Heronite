//! Settings → Cloud: what goes to the account (the library sync, backups,
//! game saves, screenshots), each Off, Manual (only "Send now") or
//! Automatic, with a dot for how much of it is up, what its mode does and
//! the space it takes, under the bar of that space (`space.rs`); and the row
//! the Backups, Saves and Screenshots sections show to link here.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::maintenance::human_size;
use heronite_i18n::{tr, trn};
use heronite_sync::cloud::Upload;
use heronite_sync::sent::Status;
use heronite_sync::usage::Kind;
use heronite_sync::{Mode, SyncPrefs};

use super::{account, persist, space, tile};
use crate::icons::Icon;
use crate::pages::settings::{Section, SettingsPage};
use crate::services::sync::{Job, LibrarySync};
use crate::theme;
use crate::ui::controls::{button_sm, card, card_box, row, segmented, sub_rows, toggle_row, ButtonKind};
use crate::ui::fields::meta_text;
use crate::ui::find::flash;
use crate::ui::icon;

pub(super) fn with_service(cx: &mut App, f: impl FnOnce(&mut LibrarySync, &mut Context<LibrarySync>)) {
    if let Some(svc) = crate::services::sync::get(cx) {
        svc.update(cx, f);
    }
}

/// One line of the card: the library or one of the uploads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Line {
    Library,
    Up(Upload),
}

impl Line {
    fn key(self) -> &'static str {
        match self {
            Line::Library => "library",
            Line::Up(u) => u.key(),
        }
    }

    fn mode(self, p: &SyncPrefs) -> Mode {
        match self {
            Line::Library => p.library,
            Line::Up(u) => u.mode(p),
        }
    }

    fn kind(self) -> Kind {
        match self {
            Line::Library => Kind::Library,
            Line::Up(Upload::Backups) => Kind::Backups,
            Line::Up(Upload::Saves) => Kind::Saves,
            Line::Up(Upload::Captures) => Kind::Captures,
        }
    }

    fn icon(self) -> Icon {
        match self {
            Line::Library => Icon::Collections,
            Line::Up(Upload::Backups) => Icon::History,
            Line::Up(Upload::Saves) => Icon::Save,
            Line::Up(Upload::Captures) => Icon::PhotoCamera,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Line::Library => tr!("Library between PCs"),
            Line::Up(Upload::Backups) => tr!("Heronite backups"),
            Line::Up(Upload::Saves) => tr!("Game saves (Ludusavi)"),
            Line::Up(Upload::Captures) => tr!("Screenshots"),
        }
    }

    /// What the mode does for this line.
    fn about(self, mode: Mode) -> &'static str {
        match (self, mode) {
            (Line::Library, Mode::Off) => tr!("Statuses, favourites, notes, play time and sessions, the same on every PC."),
            (Line::Library, Mode::Auto) => tr!("Syncs when Heronite starts, after each game and every 10 minutes."),
            (Line::Library, Mode::Manual) => tr!("Syncs only when you press Sync now."),
            (Line::Up(Upload::Backups), Mode::Off) => {
                tr!("Each backup as one zip, your artwork next to it once. Store sign-ins stay on this PC.")
            }
            (Line::Up(Upload::Backups), Mode::Auto) => tr!("Each backup goes up as it is made."),
            (Line::Up(Upload::Saves), Mode::Off) => tr!("Ludusavi's backups of your games, only what changed."),
            (Line::Up(Upload::Saves), Mode::Auto) => tr!("Ludusavi's backups go up after each game."),
            (Line::Up(Upload::Captures), Mode::Off) => {
                tr!("Once compressed, one file each, only the new ones. Deleting one here keeps it in the cloud.")
            }
            (Line::Up(Upload::Captures), Mode::Auto) => tr!("They go up once compressed, after each game and every hour."),
            (Line::Up(_), Mode::Manual) => tr!("Sent only when you press Send now."),
        }
    }
}

/// What is being sent, for "Uploading {what}…".
pub(super) fn upload_what(upload: Upload) -> &'static str {
    match upload {
        Upload::Backups => tr!("the backup"),
        Upload::Saves => tr!("the game saves"),
        Upload::Captures => tr!("the screenshots"),
    }
}

/// How much of the line is in the cloud (measured on this PC), as a dot
/// and a few words: green all sent, yellow something new, red nothing yet.
fn status_of(d: &SettingsPage, line: Line) -> Option<(Hsla, String)> {
    let usage = d.cloud.usage.as_ref()?;
    let kind = line.kind();
    let green: Hsla = theme::status_color("playing").into();
    let yellow: Hsla = theme::status_color("on-hold").into();
    let red: Hsla = theme::danger_text().into();
    Some(match usage.status.get(&kind).copied()? {
        Status::Done => (green, tr!("All sent").into()),
        Status::Empty => (theme::text_dim().into(), tr!("Nothing to send").into()),
        Status::Nothing => (red, tr!("Nothing sent yet").into()),
        Status::Behind { .. } if line == Line::Library => (yellow, tr!("Changes since the last sync").into()),
        Status::Behind { .. } if line == Line::Up(Upload::Backups) => {
            (yellow, tr!("The newest backup or new artwork is not sent yet").into())
        }
        Status::Behind { files, bytes } => {
            (yellow, trn!(files, "{n} file not sent yet ({size})", "{n} files not sent yet ({size})", size = human_size(bytes)))
        }
    })
}

/// How the line stands: its upload running or waiting for the account, how
/// much is sent, and when it last went (or why it failed). `None` while it
/// is off or nothing is known yet.
fn status_text(d: &SettingsPage, p: &SyncPrefs, line: Line, cx: &App) -> Option<String> {
    if !line.mode(p).is_on() {
        return None;
    }
    let status = status_of(d, line).map(|(_, text)| text);
    let Line::Up(upload) = line else {
        let synced = p.last_sync_at.as_deref().map(|at| {
            format!(
                "{} \u{b7} {}",
                tr!("Last sync: {last}", last = crate::pages::settings::format_time(at)),
                trn!(p.devices.len(), "{n} other PC", "{n} other PCs")
            )
        });
        return joined([status, synced]);
    };
    let svc = crate::services::sync::get(cx);
    let svc = svc.as_ref().map(|s| s.read(cx));
    if let Some(percent) = svc.and_then(|s| s.uploading()).filter(|u| u.job.upload() == upload).map(|u| u.percent) {
        return Some(match percent {
            Some(percent) => tr!("Uploading… {percent}%", percent),
            None => tr!("Uploading…").into(),
        });
    }
    if let Some(at) = svc.and_then(|s| s.retry_at(upload)).filter(|at| *at > std::time::SystemTime::now()) {
        let at: chrono::DateTime<chrono::Local> = at.into();
        return Some(tr!("The account is refusing requests for now: trying again at {time}.", time = at.format("%H:%M")));
    }
    let last = p.uploads.get(upload.key());
    if let Some(error) = last.and_then(|l| l.error.clone()) {
        return Some(tr!("The last upload failed: {error}", error));
    }
    let when = last.and_then(|l| l.at.as_deref()).map(|at| tr!("last upload {when}", when = crate::pages::settings::format_time(at)));
    joined([status, when])
}

/// The parts there are, joined by a dot (`None` when there is none).
fn joined(parts: [Option<String>; 2]) -> Option<String> {
    let parts: Vec<String> = parts.into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join(" \u{b7} "))
}

/// The upload "Send now" starts (`None`: nothing to send yet).
pub(super) fn job_of(d: &SettingsPage, upload: Upload) -> Option<Job> {
    match upload {
        Upload::Backups => d.settings.backup.last_backup_path.clone().map(|p| Job::Backup(p.into())),
        Upload::Saves => Some(Job::Saves),
        Upload::Captures => Some(Job::Captures),
    }
}

fn set_mode(d: &mut SettingsPage, line: Line, mode: Mode, cx: &mut Context<SettingsPage>) {
    persist(
        d,
        move |p| match line {
            Line::Library => p.library = mode,
            Line::Up(Upload::Backups) => p.backups = mode,
            Line::Up(Upload::Saves) => p.saves = mode,
            Line::Up(Upload::Captures) => p.captures = mode,
        },
        cx,
    );
    // Turned automatic: what is there already goes now.
    if mode == Mode::Auto {
        send_now(d, line, false, cx);
    }
}

fn send_now(d: &SettingsPage, line: Line, manual: bool, cx: &mut App) {
    match line {
        Line::Library => with_service(cx, |s, cx| s.sync(manual, cx)),
        Line::Up(upload) => {
            if let Some(job) = job_of(d, upload) {
                with_service(cx, |s, cx| s.upload(job, manual, cx));
            }
        }
    }
}

/// A line: its icon, name, how it stands, what its mode does, the space it
/// takes, "Send now" and its mode.
fn line_row(d: &SettingsPage, p: &SyncPrefs, line: Line, cx: &mut Context<SettingsPage>) -> AnyElement {
    let mode = line.mode(p);
    let ready = p.ready();
    let entity = cx.entity().downgrade();
    let modes = segmented(
        &format!("cloud-mode-{}", line.key()),
        &[(Mode::Off, tr!("Off")), (Mode::Manual, tr!("Manual")), (Mode::Auto, tr!("Automatic"))],
        mode,
        ready,
        move |m, _, cx| {
            entity.update(cx, |d, cx| set_mode(d, line, m, cx)).ok();
        },
    );
    let can_send = ready && mode.is_on() && (line == Line::Library || matches!(line, Line::Up(u) if job_of(d, u).is_some()));
    let label = if line == Line::Library { tr!("Sync now") } else { tr!("Send now") };
    let send = can_send.then(|| {
        button_sm(SharedString::from(format!("cloud-send-{}", line.key())), ButtonKind::Glass, label)
            .on_click(cx.listener(move |d, _, _, cx| send_now(d, line, true, cx)))
    });
    let size = d.cloud.usage.as_ref().map(|u| u.total(line.kind())).filter(|b| *b > 0).map(|b| {
        div()
            .w(px(72.))
            .text_right()
            .text_size(px(12.))
            .text_color(if mode.is_on() { theme::text_muted() } else { theme::text_dim() })
            .child(human_size(b))
    });
    let controls = div().flex().flex_none().items_center().gap(px(8.)).children(size).children(send).child(modes);
    let dot = mode.is_on().then(|| status_of(d, line)).flatten().map(|(color, _)| color);
    let status = status_text(d, p, line, cx).map(|text| {
        div()
            .flex()
            .items_start()
            .gap(px(6.))
            .children(dot.map(|c| div().flex_none().mt(px(4.)).size(px(8.)).rounded_full().bg(c)))
            .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_muted()).child(text))
    });
    let color = space::kind_color(line.kind());
    let text = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(1.))
        .child(div().text_size(px(14.)).text_color(theme::text_primary()).child(line.label()))
        .children(status)
        .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(line.about(mode)));
    div()
        .relative()
        .children(flash(line.label(), 8.))
        .flex()
        .items_center()
        .gap(px(12.))
        .py(px(11.))
        .child(tile(line.icon(), color.opacity(0.16), theme::readable(color)).when(!mode.is_on(), |t| t.opacity(0.5)))
        .child(text)
        .child(controls)
        .into_any_element()
}

/// `parent` with the rows that only matter because of it under its text
/// (past the tile).
fn with_sub_rows(parent: AnyElement, rows: Vec<AnyElement>) -> AnyElement {
    if rows.is_empty() {
        return parent;
    }
    div().flex().flex_col().child(parent).child(div().pl(px(42.)).child(sub_rows(rows))).into_any_element()
}

/// Under the card: what Automatic never does and what never leaves a PC.
pub(super) fn stays_note() -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(6.))
        .mt(px(8.))
        .mx(px(6.))
        .child(div().flex_none().pt(px(1.)).child(icon(Icon::InfoOutlined, px(14.), theme::text_dim())))
        .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(tr!(
            "Automatic never sends during a game. Installed games, Wine prefixes, game settings and store sign-ins always stay on each PC."
        )))
}

fn library_row(d: &SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> AnyElement {
    let parent = line_row(d, p, Line::Library, cx);
    let play_only = p.play_only;
    let sub = p.library.is_on().then(|| {
        toggle_row(
            "cloud-play-only",
            tr!("This PC only sends play time and sessions"),
            Some(tr!(
                "For a PC you do not keep organised: what you change here (statuses, favourites, notes) stays here and gives way to your other PCs, while its play still reaches them."
            )),
            play_only,
        )
        .on_click(cx.listener(move |d, _, _, cx| persist(d, move |p| p.play_only = !play_only, cx)))
        .into_any_element()
    });
    with_sub_rows(parent, sub.into_iter().collect())
}

fn backups_row(d: &SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> AnyElement {
    let parent = line_row(d, p, Line::Up(Upload::Backups), cx);
    let mut subs = Vec::new();
    if d.settings.backup.folder.trim().is_empty() {
        subs.push(
            div().py(px(10.)).child(meta_text(tr!("Backups are off: choose a backup folder in Settings → Backups."))).into_any_element(),
        );
    }
    with_sub_rows(parent, subs)
}

fn captures_row(d: &SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> AnyElement {
    let parent = line_row(d, p, Line::Up(Upload::Captures), cx);
    let replays = p.replays;
    let sub = p.captures.is_on().then(|| {
        toggle_row("cloud-replays", tr!("Include replays (videos)"), Some(tr!("About 20 MB each: they fill the account quickly.")), replays)
            .on_click(cx.listener(move |d, _, _, cx| {
                persist(d, move |p| p.replays = !replays, cx);
                // The videos change what goes up.
                super::space::measure(d, cx);
            }))
            .into_any_element()
    });
    with_sub_rows(parent, sub.into_iter().collect())
}

/// The space it all takes, then the four lines, each with its mode (dimmed
/// until an account is connected).
pub(super) fn render(d: &mut SettingsPage, p: &SyncPrefs, cx: &mut Context<SettingsPage>) -> Div {
    let rows = vec![
        space::render(d, p, cx).into_any_element(),
        library_row(d, p, cx),
        backups_row(d, p, cx),
        line_row(d, p, Line::Up(Upload::Saves), cx),
        captures_row(d, p, cx),
    ];
    card(rows).when(!p.ready(), |c| c.opacity(0.55))
}

/// The Cloud row of Backups, Saves and Screenshots: whether `upload` goes to
/// the account, and the way to Settings → Cloud.
pub(in crate::pages::settings) fn link_row(d: &SettingsPage, upload: Upload, cx: &mut Context<SettingsPage>) -> Div {
    let status = match &d.cloud.prefs {
        Some(p) if p.ready() && upload.mode(p) == Mode::Auto => {
            tr!("Automatic \u{b7} {place}", place = account::provider_label(p.provider))
        }
        Some(p) if p.ready() && upload.mode(p) == Mode::Manual => tr!("Manual \u{b7} {place}", place = account::provider_label(p.provider)),
        Some(p) if p.ready() => tr!("Off").into(),
        _ => tr!("No account connected").into(),
    };
    let id = SharedString::from(format!("cloud-link-{}", upload.key()));
    card_box().child(row(
        tr!("Cloud"),
        Some(&status),
        button_sm(id, ButtonKind::Glass, tr!("Cloud settings"))
            .on_click(cx.listener(|d, _, window, cx| d.select(Section::Cloud, window, cx))),
    ))
}
