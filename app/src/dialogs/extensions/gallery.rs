//! The gallery's face: a search field and the filters, the entries the
//! lists offer on the left, the picked one on the right, and a footer that
//! says which lists were read and when. With nothing to show it says why:
//! the lists are being read, none could be read (the app's list answers
//! 404 until it is published), a list is empty, or nothing matches.

use std::time::SystemTime;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_metadata::SourceKind;
use heronite_metadata_ext::lists::ListCopy;

use super::permissions::{self, Asks};
use super::{words, ExtensionsDialog, Filter, Offer, OfferState};
use crate::icons::Icon;
use crate::pages::settings::metadata_labels as labels;
use crate::pages::{status_pill, tag_pill};
use crate::theme;
use crate::ui::controls::{action_button, button, icon_action_button, ButtonKind};
use crate::ui::fields::{field_label, meta_text, search_box};
use crate::ui::icon;

/// Width of the picked entry's panel.
const PANEL_W: f32 = 340.;

/// Whether `o` matches every word of `query` (name, author, id or
/// description) and `filter`.
pub(super) fn matches(o: &Offer, query: &str, filter: Filter) -> bool {
    let by_filter = match filter {
        Filter::All => true,
        Filter::Kind(k) => o.entry.kind == k,
        Filter::Updates => matches!(o.state, OfferState::Update(_)),
    };
    let e = &o.entry;
    let hay = format!("{} {} {} {}", e.name, e.author, e.id, e.description).to_lowercase();
    by_filter && query.to_lowercase().split_whitespace().all(|w| hay.contains(w))
}

/// The label of an entry's button, and whether it can be pressed.
fn action_label(state: &OfferState, version: &str) -> (String, bool) {
    match state {
        OfferState::New => (tr!("Install").into(), true),
        OfferState::Update(_) => (tr!("Update to {version}", version), true),
        OfferState::Installed => (tr!("Installed").into(), false),
        OfferState::Unsupported => (tr!("Needs a newer Heronite").into(), false),
    }
}

/// "From the Heronite list · read today" (and "could not be read now"
/// when the copy shown is an older one). `checkout`: the key of the app's
/// own list when it is this repository's ("From this copy of the
/// repository").
pub(super) fn source_line(copies: &[ListCopy], now: SystemTime, checkout: Option<&str>) -> Option<String> {
    let read: Vec<&ListCopy> = copies.iter().filter(|c| c.index.is_some()).collect();
    let oldest = read.iter().filter_map(|c| c.read_at).min()?;
    let from = match read.as_slice() {
        [one] => words::list_from(&one.url, one.builtin, checkout == Some(one.url.as_str())),
        many => tr!("From {n} lists", n = many.len()),
    };
    let mut parts = vec![from, words::read_ago(oldest, now)];
    if read.iter().any(|c| c.error.is_some()) {
        parts.push(tr!("could not be read now").into());
    }
    Some(parts.join(" \u{b7} "))
}

impl ExtensionsDialog {
    fn render_filters(&self, offers: &[Offer], cx: &mut Context<Self>) -> Div {
        let updates = offers.iter().filter(|o| matches!(o.state, OfferState::Update(_))).count();
        let mut pills: Vec<(Filter, String)> = vec![(Filter::All, tr!("All {n}", n = offers.len()))];
        for k in [SourceKind::Details, SourceKind::Pictures, SourceKind::Extras] {
            pills.push((Filter::Kind(k), words::kind(k).to_string()));
        }
        if updates > 0 {
            pills.push((Filter::Updates, tr!("Updates {n}", n = updates)));
        }
        let mut row = div().flex().flex_wrap().items_center().gap(px(4.));
        for (i, (filter, label)) in pills.into_iter().enumerate() {
            let on = self.filter == filter;
            let update = filter == Filter::Updates;
            row = row.child(
                div()
                    .id(("ext-filter", i))
                    .px(px(11.))
                    .py(px(5.))
                    .rounded_full()
                    .text_size(px(12.5))
                    .cursor_pointer()
                    .when(on, |d| d.bg(theme::seg_active()).text_color(theme::text_bright()))
                    .when(!on && update, |d| d.bg(theme::accent_alpha(0.12)).text_color(theme::readable(theme::accent())))
                    .when(!on && !update, |d| {
                        d.text_color(theme::text_muted()).hover(|s| s.bg(theme::ink_a(0.06)).text_color(theme::text_primary()))
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.filter = filter;
                        cx.notify();
                    })),
            );
        }
        div().flex().items_center().gap(px(12.)).child(div().w(px(260.)).flex_none().child(search_box(&self.query))).child(row)
    }

    fn entry_button(&self, o: &Offer, small: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let busy = self.preparing.as_deref() == Some(o.entry.id.as_str());
        let (label, enabled) = action_label(&o.state, &o.entry.version.to_string());
        let label = if busy { tr!("Downloading\u{2026}").to_string() } else { label };
        let kind = if small { ButtonKind::Glass } else { ButtonKind::Primary };
        let offer = o.clone();
        let id = SharedString::from(format!("ext-{}-{}", if small { "row" } else { "go" }, o.entry.id));
        let go = cx.listener(move |this: &mut Self, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            this.picked = Some(offer.entry.id.clone());
            this.prepare(&offer, cx);
        });
        if small {
            action_button(id, kind, label, enabled && self.preparing.is_none(), go)
        } else {
            let ic = if matches!(o.state, OfferState::Update(_)) { Icon::Update } else { Icon::Download };
            icon_action_button(id, kind, ic, label, go).when(!enabled || self.preparing.is_some(), |b| b.opacity(0.45))
        }
    }

    fn render_entry(&self, o: &Offer, picked: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let e = &o.entry;
        let mut meta = format!("{} \u{b7} {}", e.author, e.version);
        // The list's host, unless it is the Heronite list or one on this computer.
        if matches!(words::list_origin(&o.list, o.builtin), heronite_metadata_ext::lists::Origin::List(_)) {
            meta = format!("{meta} \u{b7} {}", heronite_metadata_ext::lists::host_of(&o.list));
        }
        let id = e.id.clone();
        div()
            .id(SharedString::from(format!("ext-entry-{}", e.id)))
            .flex()
            .gap(px(14.))
            .p(px(14.))
            .rounded(px(14.))
            .border_1()
            .cursor_pointer()
            .when(picked, |d| d.bg(theme::accent_alpha(0.07)).border_color(theme::accent_alpha(0.45)))
            .when(!picked, |d| d.bg(theme::ink_a(0.03)).border_color(theme::ink_a(0.08)).hover(|s| s.bg(theme::ink_a(0.05))))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.picked = Some(id.clone());
                cx.notify();
            }))
            .child(labels::mark(&e.id, &e.name, 40.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(3.))
                    .child(div().text_size(px(15.)).text_color(theme::text_primary()).truncate().child(e.name.clone()))
                    .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(meta))
                    .child(
                        div()
                            .mt(px(3.))
                            .text_size(px(12.5))
                            .line_height(px(17.))
                            .text_color(theme::text_muted())
                            .line_clamp(2)
                            .child(e.description.clone()),
                    )
                    .child(
                        div()
                            .mt(px(5.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(tag_pill(words::kind(e.kind)))
                            .when(e.adult, |d| d.child(status_pill(tr!("Adult content"), theme::warning())))
                            .child(
                                div().min_w_0().truncate().text_size(px(12.)).text_color(theme::text_dim()).child(labels::gives(&e.fields)),
                            ),
                    ),
            )
            .child(div().flex_none().child(self.entry_button(o, true, cx)))
    }

    fn render_panel(&self, o: &Offer, cx: &mut Context<Self>) -> Div {
        let e = &o.entry;
        let origin = words::list_origin(&o.list, o.builtin);
        let audience = e.game_kind.as_ref().map_or_else(|| labels::audience_of(&e.applies_to), |k| labels::kind_name(k).to_string());
        let mut pills = div().flex().flex_wrap().gap(px(6.)).child(tag_pill(words::kind(e.kind))).child(tag_pill(audience));
        if e.adult {
            pills = pills.child(status_pill(tr!("Adult content"), theme::warning()));
        }
        if let OfferState::Update(have) = &o.state {
            pills = pills.child(status_pill(tr!("{have} installed", have = have.clone()), theme::accent()));
        }
        let mut fills = div().flex().flex_wrap().gap(px(6.));
        for f in &e.fields {
            fills = fills.child(tag_pill(labels::field_name(f)));
        }
        let trust_c = if words::cautious(&origin) { theme::readable(theme::warning()) } else { theme::text_muted().into() };
        let trust = div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(div().pt(px(1.)).child(icon(if words::cautious(&origin) { Icon::Warning } else { Icon::Check }, px(14.), trust_c)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .min_w_0()
                    .child(div().text_size(px(12.5)).text_color(theme::text_primary()).child(words::origin(&origin)))
                    .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_dim()).child(words::guarantee(&origin))),
            );
        let homepage = e.homepage.clone().map(|link| {
            let shown = link.clone();
            div()
                .id("ext-homepage")
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(px(12.5))
                .text_color(theme::readable(theme::accent()))
                .cursor_pointer()
                .hover(|s| s.opacity(0.8))
                .on_click(move |_, _, _| crate::ui::open_external(&link))
                .child(tr!("Homepage"))
                .child(icon(Icon::OpenInNew, px(13.), theme::readable(theme::accent())))
                .tooltip(crate::ui::tooltip(shown))
        });
        let error = self.entry_error.as_ref().filter(|(id, _)| *id == e.id).map(|(_, err)| {
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(12.5)).line_height(px(17.)).text_color(theme::danger_text()).child(words::error(err)))
                .children(words::detail(err).map(|d| div().text_size(px(11.5)).text_color(theme::text_dim()).child(d)))
        });
        let content = div()
            .id("ext-panel")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap(px(14.))
            .p(px(18.))
            .pb(px(10.))
            .overflow_y_scroll()
            .child(
                div().flex().items_center().gap(px(12.)).child(labels::mark(&e.id, &e.name, 48.)).child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(18.))
                                .font_weight(theme::W_BOLD)
                                .text_color(theme::text_primary())
                                .truncate()
                                .child(e.name.clone()),
                        )
                        .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(tr!(
                            "{author} \u{b7} version {version}",
                            author = e.author.clone(),
                            version = e.version.to_string()
                        ))),
                ),
            )
            .child(pills)
            .child(div().text_size(px(13.)).line_height(px(19.)).text_color(theme::text_primary()).child(e.description.clone()))
            .child(permissions::block(&Asks::of_entry(e)))
            .child(div().flex().flex_col().gap(px(6.)).child(field_label(tr!("Fills"))).child(fills));
        // The button stays in view however long the panel is.
        let foot = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(12.))
            .px(px(18.))
            .py(px(14.))
            .border_t_1()
            .border_color(theme::ink_a(0.08))
            .child(trust)
            .child(div().flex().items_center().justify_between().gap(px(10.)).child(self.entry_button(o, false, cx)).children(homepage))
            .children(error);
        div()
            .flex()
            .flex_col()
            .rounded(px(16.))
            .bg(theme::ink_a(0.03))
            .border_1()
            .border_color(theme::ink_a(0.08))
            .overflow_hidden()
            .child(content)
            .child(foot)
    }

    /// Why there is nothing to list, with what can be done.
    fn render_empty(&self, copies: &[ListCopy], reading: bool, cx: &mut Context<Self>) -> Div {
        let col = div().flex().flex_col().items_center().justify_center().gap(px(10.)).size_full().px(px(40.));
        let title = |t: String| div().text_size(px(16.)).text_color(theme::text_primary()).child(t);
        let none_read = copies.iter().all(|c| c.index.is_none());
        if reading && none_read {
            return col.child(meta_text(tr!("Reading the lists\u{2026}")));
        }
        if none_read {
            let error = copies.iter().find_map(|c| c.error.as_ref());
            let detail = error.and_then(words::list_detail);
            return col
                .child(icon(Icon::WifiOff, px(28.), theme::text_dim()))
                .child(title(tr!("Could not read the list").into()))
                .children(error.map(|e| meta_text(words::list_error(e))))
                .children(detail.map(|d| div().text_size(px(11.5)).text_color(theme::text_dim()).child(d)))
                .child(meta_text(tr!("You can still install a source from a file.")))
                .child(div().flex().gap(px(8.)).mt(px(6.)).child(
                    button("ext-retry", ButtonKind::Glass, tr!("Try again")).on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                ));
        }
        col.child(icon(Icon::Inbox, px(28.), theme::text_dim()))
            .child(title(tr!("No sources here yet").into()))
            .child(meta_text(tr!("The lists in use offer no source. Sources show here once someone adds them to a list.")))
    }

    pub(super) fn render_gallery(&mut self, height: f32, cx: &mut Context<Self>) -> Div {
        let (snapshot, reading, checkout) = {
            let ext = &self.hub.read(cx).extensions;
            (ext.snapshot.clone(), ext.reading, ext.checkout.clone())
        };
        let offers = snapshot.offers();
        let query = self.query.read(cx).value().to_string();
        let visible: Vec<&Offer> = offers.iter().filter(|o| matches(o, &query, self.filter)).collect();
        let picked = self.picked.as_deref().and_then(|id| visible.iter().find(|o| o.entry.id == id)).or(visible.first()).copied();
        let head = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(meta_text(tr!("Sources made by the community. Once installed, a source sits in your lists like any other.")))
            .children(snapshot.copies.iter().filter(|c| c.index.is_some() && c.error.is_some()).map(|c| {
                let e = c.error.as_ref().map(words::list_error).unwrap_or_default();
                let list = words::list_name(&c.url, c.builtin, checkout.as_deref() == Some(c.url.as_str()));
                div().text_size(px(12.)).text_color(theme::readable(theme::warning())).child(tr!(
                    "{list}: {why} Showing the copy read before.",
                    list,
                    why = e
                ))
            }));
        let mut body = div().flex().flex_col().gap(px(14.)).child(head);
        if self.lists_open {
            return body.child(div().h(px(height)).child(self.render_lists(&snapshot, cx)));
        }
        let columns = div().flex().gap(px(18.)).h(px(height));
        if offers.is_empty() {
            return body.child(columns.child(self.render_empty(&snapshot.copies, reading, cx)));
        }
        body = body.child(self.render_filters(&offers, cx));
        let mut list = div().id("ext-list").flex().flex_col().gap(px(10.)).flex_1().min_w_0().overflow_y_scroll().pr(px(4.));
        if visible.is_empty() {
            list = list.child(meta_text(tr!("No source matches.")).p(px(14.)));
        }
        for o in &visible {
            let on = picked.is_some_and(|p| p.entry.id == o.entry.id);
            list = list.child(self.render_entry(o, on, cx));
        }
        let panel = picked.map(|o| self.render_panel(o, cx).w(px(PANEL_W)).flex_none());
        body.child(columns.child(list).children(panel))
    }

    pub(super) fn render_gallery_footer(&mut self, cx: &mut Context<Self>) -> Div {
        let (copies, reading, checkout) = {
            let ext = &self.hub.read(cx).extensions;
            (ext.snapshot.copies.clone(), ext.reading, ext.checkout.clone())
        };
        let line = if reading {
            Some(tr!("Reading the lists\u{2026}").to_string())
        } else {
            source_line(&copies, SystemTime::now(), checkout.as_deref())
        };
        div()
            .flex()
            .flex_1()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(theme::text_dim()).children(line))
            .child(
                icon_action_button(
                    "ext-refresh",
                    ButtonKind::Ghost,
                    Icon::Refresh,
                    tr!("Refresh"),
                    cx.listener(|this, _, _, cx| this.refresh(cx)),
                )
                .when(reading, |b| b.opacity(0.45)),
            )
            .child(icon_action_button(
                "ext-file",
                ButtonKind::Ghost,
                Icon::FolderOpen,
                tr!("Install from a file\u{2026}"),
                cx.listener(|this, _, window, cx| this.pick_file(true, window, cx)),
            ))
            .child(icon_action_button(
                "ext-lists",
                ButtonKind::Ghost,
                Icon::Link,
                tr!("Lists\u{2026}"),
                cx.listener(|this, _, _, cx| {
                    this.lists_open = !this.lists_open;
                    cx.notify();
                }),
            ))
            .child(button("ext-done", ButtonKind::Glass, tr!("Done")).on_click(cx.listener(|this, _, _, cx| this.close(cx))))
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use heronite_metadata_ext::index;
    use heronite_metadata_ext::lists::{ListCopy, ListError, HERONITE_LIST};

    use super::{action_label, matches, source_line, Filter, Offer, OfferState, SourceKind};

    fn offers() -> Vec<Offer> {
        let index = index::parse(super::super::fake::MADE_UP.as_bytes()).unwrap();
        index
            .entries
            .into_iter()
            .map(|entry| {
                let state = match entry.id.as_str() {
                    "wikipedia" => OfferState::Update("1.0.0".into()),
                    "vndb" => OfferState::Installed,
                    _ => OfferState::New,
                };
                Offer { list: "https://own".into(), builtin: true, entry, state }
            })
            .collect()
    }

    fn ids(query: &str, filter: Filter) -> Vec<String> {
        offers().iter().filter(|o| matches(o, query, filter)).map(|o| o.entry.id.clone()).collect()
    }

    #[test]
    fn the_filters_and_the_search_narrow_the_entries() {
        assert_eq!(ids("", Filter::All).len(), offers().len());
        assert_eq!(ids("", Filter::Kind(SourceKind::Pictures)), ["retro-box-art"]);
        assert_eq!(ids("", Filter::Kind(SourceKind::Extras)), ["critic-scores"]);
        assert_eq!(ids("", Filter::Updates), ["wikipedia"]);
        assert_eq!(ids("WIKI", Filter::All), ["wikipedia"], "the name, any case");
        assert_eq!(ids("box-art", Filter::All), ["retro-box-art"], "the id");
        assert_eq!(ids("okabe", Filter::All), ["indie-shelf"], "the author");
        assert_eq!(ids("visual novels rj", Filter::All), Vec::<String>::new(), "every word must match");
        assert_eq!(ids("visual novels", Filter::Kind(SourceKind::Details)), ["doujin-finder", "vndb"]);
        assert_eq!(ids("pc", Filter::Kind(SourceKind::Extras)), ["critic-scores"]);
        assert_eq!(ids("wiki", Filter::Kind(SourceKind::Extras)), Vec::<String>::new());
    }

    #[test]
    fn the_footer_says_where_the_list_came_from_and_when() {
        let now = SystemTime::now();
        let index = Some(index::parse(br#"{"index": 1, "extensions": []}"#).unwrap());
        let own = ListCopy { url: HERONITE_LIST.into(), builtin: true, index: index.clone(), read_at: Some(now), error: None };
        assert_eq!(source_line(std::slice::from_ref(&own), now, None).as_deref(), Some("From the Heronite list \u{b7} read today"));
        let old = ListCopy {
            read_at: Some(now - Duration::from_secs(3 * 86_400)),
            error: Some(ListError::Unreachable("x".into())),
            ..own.clone()
        };
        assert_eq!(
            source_line(std::slice::from_ref(&old), now, None).as_deref(),
            Some("From the Heronite list \u{b7} read 3 days ago \u{b7} could not be read now")
        );
        let other = ListCopy { url: "https://lists.example.net/x.json".into(), builtin: false, index, read_at: Some(now), error: None };
        assert_eq!(source_line(&[own.clone(), other], now, None).as_deref(), Some("From 2 lists \u{b7} read today"));
        // The app's own list at another address (`HERONITE_EXTENSIONS_INDEX`).
        let elsewhere = ListCopy { url: "https://lists.example.net/own.json".into(), ..own.clone() };
        assert_eq!(source_line(std::slice::from_ref(&elsewhere), now, None).as_deref(), Some("From lists.example.net \u{b7} read today"));
        // A debug build's own list: this repository's, on this computer.
        let path = "/src/heronite/extensions/metadata/index.json";
        let checkout = ListCopy { url: path.into(), ..own.clone() };
        assert_eq!(
            source_line(std::slice::from_ref(&checkout), now, Some(path)).as_deref(),
            Some("From this copy of the repository \u{b7} read today")
        );
        assert_eq!(
            source_line(std::slice::from_ref(&checkout), now, None).as_deref(),
            Some("From a list on this computer \u{b7} read today")
        );
        let none = ListCopy { index: None, read_at: None, error: Some(ListError::Status(404)), ..own };
        assert_eq!(source_line(&[none], now, None), None);
    }

    #[test]
    fn the_button_says_what_it_does() {
        assert_eq!(action_label(&OfferState::New, "1.0.0"), ("Install".into(), true));
        assert_eq!(action_label(&OfferState::Update("1.0.0".into()), "1.1.0"), ("Update to 1.1.0".into(), true));
        assert!(!action_label(&OfferState::Installed, "1.0.0").1);
        assert!(!action_label(&OfferState::Unsupported, "1.0.0").1);
    }
}
