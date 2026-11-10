//! The ReShade section once installed: the switch, version, preset, what
//! the preset misses, and the list of shader sources.

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::reshade::detect::{Api, Arch};
use heronite_launch::reshade::manifest::{Manifest, ADOPTED};
use heronite_launch::reshade::{Overview, SourceRow};

use super::render::{api_label, buttons, warn_line};
use super::render_setup::{about, link, title_row};
use super::{Confirm, SettingsPanel};
use crate::dialogs::settings_panel::parts::{help, hint, rows, Rows};
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, row, toggle, ButtonKind};
use crate::ui::fields::input_box;
use crate::ui::icon;

/// A small button for a row.
fn small(
    id: &'static str,
    kind: ButtonKind,
    label: &str,
    enabled: bool,
    cx: &mut Context<SettingsPanel>,
    on_click: impl Fn(&mut SettingsPanel, &mut Window, &mut Context<SettingsPanel>) + 'static,
) -> Stateful<Div> {
    action_button(id, kind, label.to_string(), enabled, cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
        .h(px(30.))
        .text_size(px(12.))
}

/// `6.10.0` is newer than `6.9.2`.
fn newer(latest: &str, current: &str) -> bool {
    let key = |v: &str| v.split('.').map(|p| p.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
    key(latest) > key(current)
}

/// What the list calls a source: a package's name, a folder's or a
/// repository's last part.
fn source_label(name: &str) -> String {
    if name == ADOPTED {
        return tr!("Shaders that were already there").to_string();
    }
    name.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(name).to_string()
}

impl SettingsPanel {
    /// One card: the switch, then the version, the preset and the shaders
    /// as rows (the shaders listed under "Manage"), then the links.
    pub(super) fn reshade_installed(&mut self, o: &Arc<Overview>, cx: &mut Context<Self>) -> Div {
        let Some(m) = o.manifest.clone() else { return div() };
        let mut card = self.reshade_main_card(o, &m, cx).card();
        let reveal = self.reshade.reveal.clone();
        let probe = canvas(
            move |bounds, window, _| {
                if reveal.replace(false) {
                    crate::ui::dialog::reveal_in_drawer(bounds, false);
                    window.on_next_frame(|window, _| window.refresh());
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();
        card = card.relative().child(probe);
        card
    }

    fn reshade_main_card(&mut self, o: &Arc<Overview>, m: &Manifest, cx: &mut Context<Self>) -> Rows {
        let busy = self.reshade.busy.is_some();
        let api = Api::from_key(&m.api);
        let arch = if m.arch == "x86" { Arch::X86 } else { Arch::X64 };
        let mut t = o.target.clone();
        if let Some(t) = &mut t {
            t.arch = arch;
        }
        let state = match m.enabled {
            true => tr!("Home opens its menu in the game."),
            false => tr!("Off: the game starts without it."),
        };
        let line = format!("{} · {} · {}", m.version, api_label(t.as_ref(), api), state);
        let on = m.enabled;
        let sw = toggle("reshade-on-sw", on, busy);
        let mut main = title_row("ReShade", about(line), sw).id("reshade-on");
        if !busy {
            main = main.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| this.switch_reshade(!on, cx)));
        }
        let mut card = rows().child(main);
        let mut notes: Vec<Div> = Vec::new();
        if o.risk.banned {
            notes.push(warn_line(tr!(
                "ReShade's own compatibility list says this game bans or blocks ReShade. Remove it before playing online."
            )));
        }
        if !o.risk.findings.is_empty() {
            let names: Vec<String> = o.risk.names().iter().map(|n| heronite_data::anticheat::short_name(n)).collect();
            notes
                .push(warn_line(&tr!("Anti-cheat: {names}. Online modes may block ReShade or ban the account.", names = names.join(", "))));
        }
        if m.adopted {
            notes.push(help(tr!("Installed by hand; Heronite manages it now.")));
        }
        if !o.sharing.is_empty() {
            notes.push(warn_line(&heronite_i18n::trn!(
                o.sharing.len(),
                "Another game uses the ReShade in this folder; removing it here keeps its files.",
                "{n} other games use the ReShade in this folder; removing it here keeps their files."
            )));
        }
        if !notes.is_empty() {
            card = card.child(notes.into_iter().fold(div().flex().flex_col().gap(px(4.)).py(px(12.)), |d, n| d.child(n)));
        }
        if let Some(latest) = o.latest.as_deref().filter(|l| newer(l, &m.version)) {
            let update = small("reshade-update", ButtonKind::Primary, tr!("Update"), !busy, cx, |this, _, cx| this.update_reshade(cx));
            card = card.child(row(
                tr!("Version"),
                Some(&tr!("{current} installed; {latest} is available", current = m.version, latest)),
                update,
            ));
        }
        let (preset, change) = match &m.preset {
            Some(p) => (p.clone(), tr!("Change")),
            None => (tr!("None: ReShade's default").to_string(), tr!("Choose…")),
        };
        let import =
            self.outline_button_dyn("reshade-import".into(), change, !busy, cx, |this, window, cx| this.pick_reshade_preset(window, cx));
        card = card.child(row(tr!("Preset"), Some(&preset), import.h(px(30.)).text_size(px(12.))));
        if let Some(missing) = o.preset_needs.as_ref().map(|n| n.missing.clone()).filter(|m| !m.is_empty()) {
            let text = tr!(
                "The preset uses effects that are not installed: {files}. Add the folder or the repository that has them.",
                files = missing.join(", ")
            );
            let folder = small("reshade-missing-folder", ButtonKind::Glass, tr!("Add folder…"), !busy, cx, |this, window, cx| {
                this.pick_reshade_folder(window, cx)
            });
            let repo = small("reshade-missing-repo", ButtonKind::Glass, tr!("Add repository…"), !busy, cx, |this, _, cx| {
                this.reshade.adding_repo = true;
                cx.notify();
            });
            // A box of its own, so its buttons do not read as links of the rows.
            let note = div()
                .flex()
                .items_start()
                .gap(px(10.))
                .child(icon(Icon::Warning, px(16.), theme::warning()).mt(px(1.)))
                .child(div().flex_1().min_w_0().text_size(px(12.)).line_height(px(17.)).text_color(theme::text_primary()).child(text));
            let callout = div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .p(px(12.))
                .rounded(px(theme::R_INPUT))
                .bg(theme::ink_a(0.04))
                .border_1()
                .border_color(theme::card_border())
                .child(note)
                .child(buttons().pl(px(26.)).child(folder).child(repo));
            card = card.child(div().pb(px(12.)).child(callout));
            if self.reshade.adding_repo {
                card = card.field(self.reshade_repo_field(busy, cx));
            }
        }
        let names: Vec<String> = o.sources.iter().map(|s| source_label(&s.name)).collect();
        let summary = if names.is_empty() { tr!("No shaders").to_string() } else { names.join(", ") };
        let manage = self.reshade.manage_shaders;
        let toggle_list =
            self.outline_button_dyn("reshade-manage".into(), if manage { tr!("Done") } else { tr!("Manage") }, true, cx, |this, _, cx| {
                this.reshade.manage_shaders = !this.reshade.manage_shaders;
                this.reshade.reveal.set(this.reshade.manage_shaders);
                cx.notify();
            });
        card = card.child(row(tr!("Shaders"), Some(&summary), toggle_list.h(px(30.)).text_size(px(12.))));
        if manage {
            card = card.child(div().pb(px(12.)).child(self.reshade_sources(o, m, cx).card()));
        }
        for r in self.reshade_mod_rows(o, m, cx) {
            card = card.child(r);
        }
        if !o.user_overrides.is_empty() {
            let text = tr!(
                "Your WINEDLLOVERRIDES already sets {dlls}. Heronite adds them by itself while ReShade is on, and nothing once it is off or removed.",
                dlls = o.user_overrides.join(", ")
            );
            let clean = self.outline_button("reshade-clean-overrides", tr!("Remove from my variables"), !busy, cx, |this, _, cx| {
                this.clean_reshade_overrides(cx)
            });
            card = card.child(div().flex().flex_col().gap(px(8.)).py(px(12.)).child(hint(&text)).child(buttons().child(clean)));
        }
        let links = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(16.))
            .py(px(12.))
            .child(link("reshade-open", tr!("Open folder")).on_click(cx.listener(|this, _, _, _| this.open_reshade_folder())))
            .child(link("reshade-update-shaders", tr!("Update shaders")).on_click(cx.listener(|this, _, _, cx| {
                if this.reshade.busy.is_none() {
                    this.update_reshade_shaders(cx)
                }
            })))
            .child(div().flex_1())
            .child(link("reshade-remove", tr!("Remove ReShade")).text_color(theme::danger_text()).on_click(cx.listener(
                |this, _, _, cx| {
                    if this.reshade.busy.is_none() {
                        this.reshade.confirm = Some(Confirm::Remove);
                        cx.notify();
                    }
                },
            )));
        card = card.child(links);
        if self.reshade.confirm == Some(Confirm::Remove) {
            card = card.child(self.reshade_confirm_card(
                tr!("Remove ReShade from this game?"),
                tr!("The DLL and the shaders Heronite installed are deleted. ReShade.ini, the preset and the log go to the trash. Files you changed stay."),
                tr!("Remove"),
                cx,
                |this, cx| this.remove_reshade(cx),
            ));
        }
        card
    }

    fn reshade_sources(&mut self, o: &Arc<Overview>, m: &Manifest, cx: &mut Context<Self>) -> Rows {
        let busy = self.reshade.busy.is_some();
        let mut r = rows();
        for s in &o.sources {
            r = r.child(self.reshade_source_row(s, busy, cx));
            if let Some(Confirm::RemoveSource { name, used }) = &self.reshade.confirm {
                if *name == s.name {
                    let preset = m.preset.clone().unwrap_or_default();
                    let text =
                        tr!("The preset {preset} uses {effects} from it; those effects will disappear.", preset, effects = used.join(", "));
                    let name = name.clone();
                    r = r.child(self.reshade_confirm_card(tr!("Remove these shaders?"), &text, tr!("Remove"), cx, move |this, cx| {
                        this.remove_reshade_source(name.clone(), cx)
                    }));
                }
            }
        }
        if o.sources.is_empty() {
            r = r.child(row(tr!("No shaders"), Some(tr!("Add a package, a folder or a repository.")), div()));
        }
        let missing: Vec<String> = o.packages.iter().filter(|p| !m.packages.contains(&p.name)).map(|p| p.name.clone()).collect();
        if !missing.is_empty() {
            let items = missing.iter().map(|n| (SharedString::from(n.clone()), false)).collect();
            let list = missing.clone();
            r = r.field(self.select_row(
                "reshade-add-package",
                Some(tr!("Add package")),
                tr!("Choose a package…").to_string(),
                items,
                busy,
                cx,
                move |this, i, _, cx| {
                    if let Some(n) = list.get(i) {
                        this.add_reshade_package(n.clone(), cx);
                    }
                },
            ));
        }
        let add = buttons()
            .child(self.outline_button("reshade-add-folder", tr!("Add folder…"), !busy, cx, |this, window, cx| {
                this.pick_reshade_folder(window, cx)
            }))
            .child(self.outline_button(
                "reshade-add-repo",
                tr!("Add repository…"),
                !busy && !self.reshade.adding_repo,
                cx,
                |this, _, cx| {
                    this.reshade.adding_repo = true;
                    cx.notify();
                },
            ));
        r = r.field_help(add, vec![help(tr!("Removed effects disappear when ReShade reloads (Reload in its menu, or the next start)."))]);
        let missing_effects = o.preset_needs.as_ref().is_some_and(|n| !n.missing.is_empty());
        if self.reshade.adding_repo && !missing_effects {
            r = r.field(self.reshade_repo_field(busy, cx));
        }
        r
    }

    /// The address of a repository to add, with Add and Cancel.
    fn reshade_repo_field(&self, busy: bool, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(input_box(&self.input("reshade.repo"), busy))
            .child(help(tr!("A GitHub repository (https://github.com/<user>/<repo>) or the address of a .zip. Its Shaders and Textures folders are copied.")))
            .child(
                buttons()
                    .child(action_button("reshade-repo-add", ButtonKind::Primary, tr!("Add"), !busy, cx.listener(|this, _, _, cx| this.add_reshade_repo(cx))))
                    .child(self.outline_button("reshade-repo-cancel", tr!("Cancel"), true, cx, |this, _, cx| {
                        this.reshade.adding_repo = false;
                        cx.notify();
                    })),
            )
    }

    fn reshade_source_row(&self, s: &SourceRow, busy: bool, cx: &mut Context<Self>) -> Div {
        let mut about = format!(
            "{} · {}",
            heronite_i18n::trn!(s.effects, "{n} effect", "{n} effects"),
            heronite_data::maintenance::human_size(s.bytes)
        );
        if !s.used.is_empty() {
            about.push_str(&format!(" · {}", tr!("used by the preset")));
        }
        if s.required {
            about.push_str(&format!(" · {}", tr!("Always installed")));
        }
        let name = s.name.clone();
        let remove = self.outline_button_dyn(
            SharedString::from(format!("reshade-src-{}", s.name)),
            tr!("Remove"),
            !busy && !s.required,
            cx,
            move |this, _, cx| this.ask_remove_source(name.clone(), cx),
        );
        row(source_label(&s.name), Some(&about), remove).text_color(theme::text_primary())
    }

    /// A question with Cancel (the default) and a destructive button.
    fn reshade_confirm_card(
        &self,
        title: &str,
        text: &str,
        go: &str,
        cx: &mut Context<Self>,
        on_go: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Div {
        let cancel = action_button(
            "reshade-confirm-cancel",
            ButtonKind::Primary,
            tr!("Cancel"),
            true,
            cx.listener(|this, _, _, cx| {
                this.reshade.confirm = None;
                cx.notify();
            }),
        );
        let go = action_button(
            "reshade-confirm-go",
            ButtonKind::Danger,
            go.to_string(),
            true,
            cx.listener(move |this, _, _, cx| on_go(this, cx)),
        );
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .py(px(12.))
            .child(div().text_size(px(14.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(title.to_string()))
            .child(div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_primary()).child(text.to_string()))
            .child(buttons().child(cancel).child(go))
    }

    /// [`outline_button`](Self::outline_button) with an id built at run time.
    fn outline_button_dyn(
        &self,
        id: SharedString,
        label: &str,
        enabled: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        action_button(id, ButtonKind::Glass, label.to_string(), enabled, cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions_compare_by_number() {
        assert!(super::newer("6.10.0", "6.9.2"));
        assert!(!super::newer("6.8.0", "6.8.0"));
    }
}
