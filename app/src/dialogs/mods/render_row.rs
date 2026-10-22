//! One mod of the load order: handle, switch, picture, name and pills, the
//! conflict list and the inline questions (rename, remove).

use std::collections::HashMap;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trn};
use heronite_images::ImageRef;
use heronite_mods::conflicts::ModConflicts;
use heronite_mods::{ModEntry, ModSource};

use super::{format, name_of, Confirm, ModsDialog, View};
use crate::dialogs::art_images::picture;
use crate::icons::Icon;
use crate::pages::{status_pill, tag_pill};
use crate::services::mods::GameMods;
use crate::theme;
use crate::ui::controls::{button_sm, toggle, ButtonKind};
use crate::ui::fields;
use crate::ui::icon;

/// A mod being dragged to another place of the list.
#[derive(Clone)]
struct DragMod {
    id: String,
    name: SharedString,
    from: usize,
}

struct DragChip(SharedString);

impl Render for DragChip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::ui::glass::glass_menu_surface(theme::R_ITEM)
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .text_size(px(13.5))
            .text_color(theme::text_primary())
            .child(icon(Icon::DragIndicator, px(16.), theme::text_dim()))
            .child(self.0.clone())
    }
}

/// For each mod, the names of the mods that list it as a dependency
/// (Thunderstore: `Namespace-Name`).
pub(super) fn needed_by(g: &GameMods) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for m in &g.list.mods {
        for dep in &m.depends_on {
            let owner = g.list.mods.iter().find(|o| match &o.source {
                ModSource::Thunderstore { namespace, name, .. } => format!("{namespace}-{name}").eq_ignore_ascii_case(dep),
                _ => false,
            });
            if let Some(o) = owner {
                out.entry(o.id.clone()).or_default().push(m.name.clone());
            }
        }
    }
    out
}

impl ModsDialog {
    pub(super) fn mod_row(
        &mut self,
        v: &View,
        index: usize,
        count: usize,
        m: &ModEntry,
        needed: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _ = (window, count);
        let id = m.id.clone();
        let conflicts = v.g.conflicts.as_ref().and_then(|c| c.per_mod.get(&m.id)).filter(|_| m.enabled);
        let expanded = self.expanded.contains(&id);
        let name: SharedString = m.name.clone().into();
        let enabled = m.enabled;

        let mut row = div()
            .id(SharedString::from(format!("mods-row-{id}")))
            .flex()
            .flex_col()
            .rounded(px(theme::R_ITEM))
            .drag_over::<DragMod>(move |s, d, _, _| {
                // The mod lands where this row is: above it coming from below, below it coming from above.
                if d.from > index {
                    s.border_t_2().border_color(theme::accent())
                } else {
                    s.border_b_2().border_color(theme::accent())
                }
            })
            .on_drop::<DragMod>(cx.listener(move |this, d: &DragMod, _, cx| this.move_to(&d.id, index, cx)));

        if self.confirm == Some(Confirm::Remove(id.clone())) {
            let (rid, cid) = (id.clone(), id.clone());
            return row
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(10.))
                        .py(px(14.))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(13.5))
                                .line_height(px(19.))
                                .child(tr!("Remove {name}? Its files are deleted; the game is not touched.", name = m.name.clone())),
                        )
                        .child(button_sm(SharedString::from(format!("mods-rm-no-{cid}")), ButtonKind::Ghost, tr!("Cancel")).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.confirm = None;
                                cx.notify();
                            }),
                        ))
                        .child(
                            button_sm(SharedString::from(format!("mods-rm-yes-{rid}")), ButtonKind::Danger, tr!("Remove"))
                                .on_click(cx.listener(move |this, _, _, cx| this.remove_mod(&rid, cx))),
                        ),
                )
                .into_any_element();
        }

        let handle = {
            let drag = DragMod { id: id.clone(), name: name.clone(), from: index };
            div()
                .id(SharedString::from(format!("mods-drag-{id}")))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(px(20.))
                .h(px(40.))
                .rounded(px(6.))
                .cursor(CursorStyle::OpenHand)
                .hover(|s| s.bg(theme::white_a(0.06)))
                .tooltip(crate::ui::tooltip(tr!("Drag to change the order")))
                .on_drag(drag, |d, _, _, cx| cx.new(|_| DragChip(d.name.clone())))
                .child(icon(Icon::DragIndicator, px(18.), theme::text_dim()))
        };
        let sw = {
            let id = id.clone();
            toggle(SharedString::from(format!("mods-sw-{id}")), m.enabled, false)
                .on_click(cx.listener(move |this, _, _, cx| this.set_enabled(&id, !enabled, cx)))
        };
        let thumb = self.thumb(m, cx);

        let title: AnyElement = match &self.rename {
            Some(r) if r.id == id => div()
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                    if ev.keystroke.key == "escape" {
                        this.rename = None;
                        cx.stop_propagation();
                        cx.notify();
                    }
                }))
                .child(fields::input_box(&r.input, false))
                .into_any_element(),
            _ => div()
                .flex()
                .items_baseline()
                .gap(px(8.))
                .child(div().min_w_0().truncate().text_size(px(14.)).font_weight(theme::W_500).child(name.clone()))
                .children(m.version.clone().map(|ver| div().flex_none().text_size(px(12.)).text_color(theme::text_dim()).child(ver)))
                .into_any_element(),
        };

        let mut meta = div().flex().items_center().gap(px(7.)).overflow_hidden().text_size(px(12.5)).text_color(theme::text_muted());
        if let Some(author) = m.author.clone().filter(|a| !a.is_empty()) {
            meta = meta.child(div().min_w_0().max_w(px(150.)).truncate().child(author));
        }
        meta = meta.child(tag_pill(format::source_label(&m.source)));
        if let Some(c) = conflicts {
            meta = meta.children(self.conflict_badge(&id, c, expanded, cx));
        }
        if !needed.is_empty() {
            meta = meta
                .child(div().min_w_0().truncate().text_color(theme::text_dim()).child(tr!("Needed by {names}", names = needed.join(", "))));
        }

        let update = m.update.as_ref().map(|u| {
            let id = id.clone();
            let label = match &m.version {
                Some(from) => tr!("Update {from} \u{2192} {to}", from = from.clone(), to = u.version.clone()),
                None => tr!("Update to {to}", to = u.version.clone()),
            };
            div()
                .id(SharedString::from(format!("mods-update-{id}")))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(5.))
                .h(px(26.))
                .px(px(10.))
                .rounded_full()
                .bg(theme::accent_tint())
                .text_size(px(12.5))
                .text_color(theme::accent())
                .cursor_pointer()
                .hover(|s| s.bg(theme::accent_tint_border()))
                .child(icon(Icon::Update, px(14.), theme::accent()))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let key = this.key.clone();
                    this.service.update(cx, |s, cx| s.update(&key, &id, cx));
                }))
        });

        let more = {
            let id = id.clone();
            div()
                .id(SharedString::from(format!("mods-more-{id}")))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(32.))
                .rounded_full()
                .cursor_pointer()
                .bg(if self.menu.as_ref().is_some_and(|m| m.id == id) { theme::white_a(0.10) } else { transparent_black() })
                .hover(|s| s.bg(theme::white_a(0.10)))
                .child(icon(Icon::MoreHoriz, px(20.), theme::text_muted()))
                .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| this.open_menu(&id, ev.position(), cx)))
        };

        row = row.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(6.))
                .py(px(8.))
                .child(handle)
                .child(sw)
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap(px(12.))
                        .when(!m.enabled, |d| d.opacity(0.55))
                        .child(thumb)
                        .child(div().flex().flex_col().flex_1().min_w_0().gap(px(3.)).child(title).child(meta)),
                )
                .children(update)
                .child(more),
        );
        if expanded {
            if let Some(c) = conflicts {
                row = row.child(conflict_list(&v.g, c));
            }
        }
        row.into_any_element()
    }

    /// 44px picture of the mod, else its initial on the source's tint.
    fn thumb(&mut self, m: &ModEntry, cx: &mut Context<Self>) -> Div {
        let src =
            m.picture.as_deref().filter(|u| u.starts_with("http")).and_then(|u| {
                self.images.update(cx, |im, _| im.get(&format!("mod:{}", m.id), vec![ImageRef::Url(u.to_string())], 44., 44.))
            });
        match src {
            Some(src) => picture(Some(src)).size(px(44.)).flex_none().rounded(px(10.)),
            None => div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(44.))
                .rounded(px(10.))
                .bg(theme::white_a(0.07))
                .text_size(px(17.))
                .font_family(theme::font_heading())
                .text_color(theme::text_muted())
                .child(format::initial(&m.name)),
        }
    }

    /// "Overrides 2" (accent) or "Overridden by 1" (amber); click: the list.
    fn conflict_badge(&self, id: &str, c: &ModConflicts, expanded: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (text, color) = if c.fully_overridden {
            (tr!("Fully overridden").to_string(), theme::warning())
        } else if !c.overridden_by.is_empty() {
            (tr!("Overridden by {n}", n = c.overridden_by.len()), theme::warning())
        } else if !c.overrides.is_empty() {
            (tr!("Overrides {n}", n = c.overrides.len()), theme::accent())
        } else {
            return None;
        };
        let id = id.to_string();
        Some(
            div()
                .id(SharedString::from(format!("mods-conf-{id}")))
                .flex_none()
                .cursor_pointer()
                .opacity(if expanded { 1. } else { 0.92 })
                .hover(|s| s.opacity(1.))
                .child(status_pill(text, color))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.expanded.remove(&id) {
                        this.expanded.insert(id.clone());
                    }
                    cx.notify();
                }))
                .into_any_element(),
        )
    }
}

/// Which mods this one wins files from, and which win files from it.
fn conflict_list(g: &GameMods, c: &ModConflicts) -> Div {
    let section = |title: SharedString, color: Rgba, rows: &[(String, usize)]| {
        div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(div().text_size(px(12.)).font_weight(theme::W_500).text_color(color).child(title))
            .children(rows.iter().map(|(id, n)| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(10.))
                    .text_size(px(12.5))
                    .child(div().min_w_0().truncate().text_color(theme::text_primary()).child(name_of(g, id).to_string()))
                    .child(div().flex_none().text_color(theme::text_dim()).child(format::files(*n as u64)))
            }))
    };
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .mx(px(36.))
        .mb(px(8.))
        .px(px(14.))
        .py(px(10.))
        .rounded(px(12.))
        .bg(theme::white_a(0.04))
        .border_1()
        .border_color(theme::white_a(0.07));
    if !c.overrides.is_empty() {
        let n = c.overrides.len();
        col = col.child(section(trn!(n, "Wins files from {n} mod", "Wins files from {n} mods").into(), theme::accent(), &c.overrides));
    }
    if !c.overridden_by.is_empty() {
        let n = c.overridden_by.len();
        col = col.child(section(trn!(n, "Loses files to {n} mod", "Loses files to {n} mods").into(), theme::warning(), &c.overridden_by));
    }
    col
}
