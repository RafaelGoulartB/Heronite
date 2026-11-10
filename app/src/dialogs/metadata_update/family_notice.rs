//! The notice under a Cover aspect picker (the View options' and Settings →
//! Appearance's): after the user picks an aspect whose family some games
//! have no picture of, it says how many and offers to download them; when
//! the automatic download started (`fillCoverFamilies`), it says so instead,
//! with Stop. The hub keeps it, so both pickers show the same one; it goes
//! when dismissed, when the aspect goes back to a tall one, or when the
//! download it announced ends (the header pill reports how it went).
//!
//! The downloads by hand (this notice's, Settings → Metadata's, the dock's)
//! go through [`MetadataHub::download_family`].
//!
//! Debug: `HERONITE_DEBUG_ASPECT_NOTICE=offer[:wide]|started[:wide]` shows
//! the notice for the square (or wide) family over the Collection's real
//! counts; `started` needs `HERONITE_DEBUG_METADATA_JOB=running`, so the
//! download is made up (see `CollectionView::debug_metadata_job`).

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::cover_family::CoverFamily;
use heronite_data::settings::CoverAspect;
use heronite_i18n::tr;
use heronite_metadata::{BulkArgs, BulkItem, BulkState, JobKind};

use super::hub::{hub, HubGlobal, MetadataHub};
use crate::dialogs::covers::words;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{button_sm, ButtonKind};
use crate::ui::icon;

/// What the notice says (see the module docs).
#[derive(Clone, Debug)]
pub struct FamilyNotice {
    pub family: CoverFamily,
    /// The games without a picture of the family; `None` while counted.
    pub lacking: Option<usize>,
    /// A source that gives the family can be asked now.
    pub can_download: bool,
    /// The download of the family queued since the aspect was picked.
    pub job: Option<u64>,
}

/// Where the download a notice announced stands.
enum Download {
    Running { games: usize },
    Waiting { games: usize },
}

impl MetadataHub {
    /// A Cover aspect picker went from `before` to `after`: the notice about
    /// the new family, counted off the UI thread (nothing for a tall one).
    /// Call it before [`super::families::aspect_changed`], so the download
    /// that starts by itself is the notice's.
    pub fn aspect_picked(&mut self, before: CoverAspect, after: CoverAspect, cx: &mut Context<Self>) {
        let family = after.family();
        if family == before.family() {
            return;
        }
        self.notice = None;
        if family != CoverFamily::Tall {
            self.show_notice(family, cx);
        }
        cx.notify();
    }

    /// Counts what `family` lacks over the Collection and shows the notice.
    pub fn show_notice(&mut self, family: CoverFamily, cx: &mut Context<Self>) {
        // A download of the family already queued is the one to tell about.
        let job = self.family_job(family);
        self.notice = Some(FamilyNotice { family, lacking: None, can_download: false, job });
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let lack = cx.background_spawn(async move { service.family_lack(family, None) }).await;
            this.update(cx, |hub, cx| {
                let Some(notice) = hub.notice.as_mut().filter(|n| n.family == family) else { return };
                match lack {
                    Some(lack) if !lack.games.is_empty() || notice.job.is_some() => {
                        notice.lacking = Some(lack.games.len());
                        notice.can_download = lack.can_download;
                    }
                    _ => hub.notice = None,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn dismiss_notice(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        cx.notify();
    }

    /// The running or waiting download of `family`, if any.
    fn family_job(&self, family: CoverFamily) -> Option<u64> {
        let kind = JobKind::Covers(family);
        if self.progress.kind == kind && self.progress.is_running() {
            return Some(self.progress.job);
        }
        self.progress.queued.iter().find(|q| q.kind == kind).map(|q| q.id)
    }

    /// A job was queued: the notice of its family announces it.
    pub(super) fn note_family_job(&mut self, kind: JobKind, id: u64) {
        if let (JobKind::Covers(family), Some(n)) = (kind, self.notice.as_mut()) {
            if n.family == family {
                n.job = Some(id);
            }
        }
    }

    /// A job ended: the notice that announced it has nothing more to say.
    pub(super) fn family_job_ended(&mut self, id: u64) {
        if self.notice.as_ref().is_some_and(|n| n.job == Some(id)) {
            self.notice = None;
        }
    }

    fn notice_download(&self) -> Option<Download> {
        let id = self.notice.as_ref()?.job?;
        if self.progress.job == id && self.progress.state != BulkState::Done {
            return Some(Download::Running { games: self.progress.total });
        }
        self.progress.queued.iter().find(|q| q.id == id).map(|q| Download::Waiting { games: q.games })
    }

    /// Downloads `family`'s covers for the games of `games` (`None`: the
    /// whole Collection) that lack one, whatever `fillCoverFamilies` says:
    /// the games are found off the UI thread, then the job is queued (the
    /// header pill follows it). Nothing when no game lacks one.
    pub fn download_family(&mut self, family: CoverFamily, games: Option<Vec<BulkItem>>, cx: &mut Context<Self>) {
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let lack = cx.background_spawn(async move { service.family_lack(family, games) }).await;
            let Some(lack) = lack.filter(|l| !l.games.is_empty()) else { return };
            this.update(cx, |hub, cx| hub.start(BulkArgs::download_covers(lack.games, family), cx)).ok();
        })
        .detach();
    }
}

/// The notice under a Cover aspect picker, when there is one; `id` keeps
/// the two pickers' buttons apart.
pub fn notice(id: &str, cx: &App) -> Option<Div> {
    if !cx.has_global::<HubGlobal>() {
        return None;
    }
    let h = hub(cx);
    let hub_ref = h.read(cx);
    let n = hub_ref.notice.clone()?;
    let family = n.family;
    let download = hub_ref.notice_download();
    let button = |name: &str, kind: ButtonKind, label: &'static str| button_sm(SharedString::from(format!("{id}-{name}")), kind, label);
    let (text, actions): (String, Vec<AnyElement>) = match download {
        Some(Download::Running { games }) => (
            words::downloading(family, games),
            vec![button("stop", ButtonKind::Glass, tr!("Stop"))
                .on_click(|_, _, cx| hub(cx).update(cx, |h, cx| h.cancel(cx)))
                .into_any_element()],
        ),
        Some(Download::Waiting { games }) => {
            (format!("{} {}", words::downloading(family, games), tr!("It starts after the update running now.")), Vec::new())
        }
        None => {
            let lacking = n.lacking?;
            let first = if n.can_download {
                button("download", ButtonKind::Primary, words::download(family))
                    .on_click(move |_, _, cx| {
                        hub(cx).update(cx, |h, cx| {
                            h.download_family(family, None, cx);
                            h.dismiss_notice(cx);
                        })
                    })
                    .into_any_element()
            } else {
                button("settings", ButtonKind::Glass, tr!("Open Settings \u{2192} Metadata"))
                    .on_click(|_, _, cx| hub(cx).update(cx, |h, cx| h.request_settings(cx)))
                    .into_any_element()
            };
            let text = if n.can_download {
                words::lacking(family, lacking)
            } else {
                format!("{} {}", words::lacking(family, lacking), words::no_source(family))
            };
            (text, vec![first])
        }
    };
    let dismiss = div()
        .id(SharedString::from(format!("{id}-dismiss")))
        .text_size(px(12.5))
        .text_color(theme::text_muted())
        .cursor_pointer()
        .hover(|s| s.text_color(theme::text_bright()))
        .child(if download.is_some() { tr!("Hide") } else { tr!("Keep the crop") })
        .on_click(|_, _, cx| hub(cx).update(cx, |h, cx| h.dismiss_notice(cx)));
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(12.))
            .rounded(px(theme::R_TILE))
            .bg(theme::card_fill())
            .border_1()
            .border_color(theme::card_border())
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(icon(Icon::Image, px(16.), theme::caution_text()).mt(px(1.)))
                    .child(div().flex_1().min_w_0().text_size(px(12.5)).line_height(px(17.)).text_color(theme::text_primary()).child(text)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(div().flex().gap(px(8.)).children(actions))
                    .child(dismiss),
            ),
    )
}
