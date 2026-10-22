//! Installed tab notices about the game itself: what mods cannot do for it
//! (Baldur's Gate 3's `.pak` files, a Linux build, anti-cheat) and the mod
//! loaders that the game has to be started through.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::render_jobs::{tinted, Tone};
use super::{ModsDialog, View};
use crate::icons::Icon;
use crate::services::mods::{LoaderCard, LoaderStart};
use crate::theme;
use crate::ui::controls::{button_sm, ButtonKind};

/// A notice's text: an optional title over a muted body.
fn words(title: Option<SharedString>, text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(px(2.))
        .children(title.map(|t| div().text_size(px(14.)).font_weight(theme::W_500).text_color(theme::text_primary()).child(t)))
        .child(div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_muted()).child(text.into()))
}

/// Games on which online play with mods risks the account.
const BAN_RISK: [&str; 2] = ["eldenring", "darksouls3"];

impl ModsDialog {
    /// What mods cannot do for this game: short, one card each.
    pub(super) fn limit_notices(&self, v: &View) -> Vec<AnyElement> {
        let info = &v.g.info;
        let domain = info.nexus_domain.as_deref().or(info.known.and_then(|k| k.nexus_domain)).unwrap_or_default();
        let mut out = Vec::new();
        if domain == "baldursgate3" {
            out.push(
                tinted(Tone::Warn, Icon::Warning)
                    .child(words(
                        Some(tr!("Most mods for this game cannot be applied").into()),
                        tr!("Its .pak mods live in AppData/Larian Studios, outside the game's folder, so Heronite cannot apply them. Only Script Extender and native mods work."),
                    ))
                    .into_any_element(),
            );
        }
        if BAN_RISK.contains(&domain) {
            let text = if domain == "eldenring" {
                tr!("Playing online with mods can get your account banned. Use Seamless Coop, or play offline.")
            } else {
                tr!("Playing online with mods can get your account banned. Play offline.")
            };
            out.push(tinted(Tone::Warn, Icon::Warning).child(words(None, text)).into_any_element());
        } else if !info.anticheat.is_empty() {
            let names = info.anticheat.join(", ");
            out.push(
                tinted(Tone::Warn, Icon::Warning)
                    .child(words(None, tr!("This game uses {names}: playing online with mods can get your account banned.", names)))
                    .into_any_element(),
            );
        }
        if info.native && !v.g.list.mods.is_empty() {
            out.push(
                tinted(Tone::Info, Icon::InfoOutlined)
                    .child(words(
                        None,
                        tr!("This is the Linux build of the game: loaders that work through DLLs (BepInEx for Windows, MelonLoader, ASI loaders) do not apply to it."),
                    ))
                    .into_any_element(),
            );
        }
        out
    }

    /// One card for each loader the enabled mods bring.
    pub(super) fn loader_notices(&self, v: &View, cx: &mut Context<Self>) -> Vec<AnyElement> {
        v.g.loaders.iter().enumerate().map(|(i, l)| self.loader_card(l, i, cx)).collect()
    }

    fn loader_card(&self, l: &LoaderCard, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let toggle = |rel: String, on: bool, id: SharedString, label: SharedString, kind: ButtonKind, cx: &mut Context<Self>| {
            button_sm(id, kind, label).self_center().on_click(cx.listener(move |this, _, _, cx| {
                let key = this.key.clone();
                this.service.update(cx, |s, cx| s.start_through(&key, &rel, on, cx));
            }))
        };
        let arguments = l.needs_arguments.then(|| {
            div().mt(px(4.)).text_size(px(12.5)).line_height(px(17.)).text_color(theme::text_dim()).child(tr!(
                "It also needs its arguments (for ModEngine 2: -t er -c config_eldenring.toml): put them in the game's launcher arguments."
            ))
        });
        match &l.start {
            LoaderStart::On => tinted(Tone::Info, Icon::Check)
                .child(words(None, tr!("Starts through {file} \u{2713}", file = l.file.clone())))
                .child(toggle(l.rel.clone(), false, format!("mods-loader-off-{index}").into(), tr!("Undo").into(), ButtonKind::Ghost, cx))
                .into_any_element(),
            LoaderStart::Off => tinted(Tone::Warn, Icon::PlayArrow)
                .child(
                    words(
                        Some(tr!("{name} is installed: the game must start through {file}", name = l.name, file = l.file.clone()).into()),
                        tr!(
                            "{name} brings it, but the game starts through its own executable, so it does nothing yet.",
                            name = l.mod_name.clone()
                        ),
                    )
                    .children(arguments),
                )
                .child(toggle(
                    l.rel.clone(),
                    true,
                    format!("mods-loader-on-{index}").into(),
                    tr!("Start through {file}", file = l.file.clone()).into(),
                    ButtonKind::Glass,
                    cx,
                ))
                .into_any_element(),
            LoaderStart::Steam { line } => {
                let title = tr!("{name} is installed: the game must start through {file}", name = l.name, file = l.file.clone());
                let body: SharedString = match line {
                    Some(_) => tr!("Steam starts the game itself, so Heronite cannot change what it runs. Paste this line in the game's launch options in Steam:").into(),
                    None => tr!("Steam starts the game itself, so Heronite cannot change what it runs. Start the game through {file} instead.", file = l.file.clone()).into(),
                };
                let mut text = words(Some(title.into()), body).children(arguments);
                let mut card = tinted(Tone::Warn, Icon::Build);
                if let Some(line) = line.clone() {
                    let copy = line.clone();
                    text = text.child(
                        div()
                            .mt(px(6.))
                            .px(px(10.))
                            .py(px(7.))
                            .rounded(px(8.))
                            .bg(theme::white_a(0.06))
                            .border_1()
                            .border_color(theme::white_a(0.08))
                            .font_family(theme::FONT_MONO)
                            .text_size(px(12.5))
                            .line_height(px(17.))
                            .text_color(theme::text_primary())
                            .child(line),
                    );
                    card = card.child(text).child(
                        button_sm(format!("mods-loader-copy-{index}"), ButtonKind::Glass, tr!("Copy the line"))
                            .self_center()
                            .on_click(cx.listener(move |_, _, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone())))),
                    );
                } else {
                    card = card.child(text);
                }
                card.into_any_element()
            }
        }
    }
}
