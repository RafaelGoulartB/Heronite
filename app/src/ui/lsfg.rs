//! What the game settings show when frame generation is on but cannot run:
//! lsfg-vk is not installed, or Lossless Scaling (whose DLL lsfg-vk loads)
//! is missing or too old. Each case says what to install and offers
//! "Look again". See `heronite_launch::lsfg`.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::lsfg::{self, Install};

use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, card_box, help_text, ButtonKind};
use crate::ui::{icon, open_external};

/// Why frame generation cannot run, `None` when it can.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    Layer,
    /// lsfg-vk 1, where only lsfg-vk 2 works (Steam's launch options).
    OldLayer,
    App,
    /// Lossless Scaling without `lsfg-vk.dll` (installed before lsfg-vk 2).
    OldApp,
}

impl Missing {
    /// `uses_profile`: a `conf.toml` profile may name the DLL itself.
    pub fn of(install: &Install, uses_profile: bool) -> Option<Self> {
        match install {
            Install { layer: None, .. } => Some(Self::Layer),
            Install { dll: None, .. } if uses_profile => None,
            Install { dll: None, dll_outdated: true, .. } => Some(Self::OldApp),
            Install { dll: None, .. } => Some(Self::App),
            _ => None,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Layer => tr!("lsfg-vk is not installed"),
            Self::OldLayer => tr!("lsfg-vk needs an update"),
            Self::App => tr!("Lossless Scaling was not found"),
            Self::OldApp => tr!("Lossless Scaling needs an update"),
        }
    }

    /// One line next to the switch, which stays on but has no effect.
    pub fn status(self) -> &'static str {
        match self {
            Self::Layer => tr!("Not active: lsfg-vk is not installed"),
            Self::OldLayer => tr!("Not active: lsfg-vk needs an update"),
            Self::App => tr!("Not active: Lossless Scaling was not found"),
            Self::OldApp => tr!("Not active: Lossless Scaling needs an update"),
        }
    }

    pub fn why(self) -> &'static str {
        match self {
            Self::Layer => tr!("Frame generation runs through lsfg-vk, a free Vulkan layer that brings the frame generation of Lossless Scaling to Linux. It also needs Lossless Scaling, a Windows app sold on Steam."),
            Self::OldLayer => tr!("Steam games get frame generation through the settings of lsfg-vk 2. This computer has lsfg-vk 1."),
            Self::App => tr!("lsfg-vk is installed, but it loads the frame generation from Lossless Scaling, a Windows app sold on Steam. Install it in any Steam library; it never has to run."),
            Self::OldApp => tr!("This copy of Lossless Scaling has no lsfg-vk.dll, which this lsfg-vk needs. Update Lossless Scaling in Steam."),
        }
    }

    /// Numbered when there are several.
    pub fn steps(self) -> Vec<String> {
        let mut v = Vec::new();
        match self {
            Self::Layer => v.push(tr!("Install lsfg-vk: the lsfg-vk package of your distribution, or the guide on lsfg-vk.dev.")),
            Self::OldLayer => v.push(tr!("Update lsfg-vk to version 2 or newer (lsfg-vk.dev has the guide).")),
            Self::App | Self::OldApp => {}
        }
        if matches!(self, Self::Layer | Self::App) {
            v.push(tr!("Install Lossless Scaling from Steam. On Linux, turn on Steam Play for all other titles first (Steam → Settings → Compatibility)."));
        }
        v.push(tr!("Come back here and press Look again."));
        if v.len() == 1 {
            return v.into_iter().map(str::to_string).collect();
        }
        v.iter().enumerate().map(|(i, s)| format!("{}. {s}", i + 1)).collect()
    }
}

pub fn open_site() {
    open_external(lsfg::WEBSITE);
}

/// Lossless Scaling's store page, in the Steam client when it is installed.
pub fn open_store() {
    let steam = heronite_launch::Probe::which(&heronite_launch::SystemProbe, "steam").is_some();
    match steam {
        true => open_external(&format!("steam://store/{}", lsfg::STEAM_APP_ID)),
        false => open_external(&format!("https://store.steampowered.com/app/{}/", lsfg::STEAM_APP_ID)),
    }
}

/// `lsfg-vk-ui`, lsfg-vk's profile editor, when installed.
pub fn ui_program() -> Option<String> {
    heronite_launch::Probe::which(&heronite_launch::SystemProbe, "lsfg-vk-ui")
}

pub fn open_ui(program: &str) {
    if let Err(e) = std::process::Command::new(program)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        log::warn!("{program}: {e}");
    }
}

/// The desktop card. `id` keeps the buttons apart when shown twice.
pub fn missing_card(id: &str, missing: Missing, look_again: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Div {
    let mut steps_col = div().flex().flex_col().gap(px(4.));
    for s in missing.steps() {
        steps_col = steps_col.child(div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_primary()).child(s));
    }
    let mut buttons = div().flex().flex_wrap().items_center().gap(px(8.)).mt(px(2.));
    let layer = matches!(missing, Missing::Layer | Missing::OldLayer);
    if layer {
        buttons = buttons.child(action_button(
            SharedString::from(format!("{id}-site")),
            ButtonKind::Primary,
            tr!("Open lsfg-vk.dev"),
            true,
            |_, _, _| open_site(),
        ));
    }
    if missing != Missing::OldLayer {
        let kind = if layer { ButtonKind::Glass } else { ButtonKind::Primary };
        buttons = buttons.child(action_button(
            SharedString::from(format!("{id}-store")),
            kind,
            tr!("Lossless Scaling on Steam"),
            true,
            |_, _, _| open_store(),
        ));
    }
    buttons = buttons.child(action_button(
        SharedString::from(format!("{id}-look-again")),
        ButtonKind::Glass,
        tr!("Look again"),
        true,
        look_again,
    ));
    card_box()
        .py(px(14.))
        .gap(px(10.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(icon(Icon::Warning, px(18.), theme::accent()))
                .child(div().text_size(px(15.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(missing.title())),
        )
        .child(help_text(missing.why()))
        .child(steps_col)
        .child(buttons)
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's `test` attribute would replace the std one.
    use super::Missing;
    use heronite_launch::lsfg::{Install, Layer};

    #[test]
    fn what_is_missing() {
        let ready = Install { layer: Some(Layer::V2), dll: Some("/x/lsfg-vk.dll".into()), ..Default::default() };
        assert_eq!(Missing::of(&ready, false), None);
        assert_eq!(Missing::of(&Install::default(), true), Some(Missing::Layer));
        let no_app = Install { layer: Some(Layer::V2), ..Default::default() };
        assert_eq!(Missing::of(&no_app, false), Some(Missing::App));
        assert_eq!(Missing::of(&no_app, true), None, "a profile's conf.toml may name the DLL");
        let old = Install { dll_outdated: true, ..no_app };
        assert_eq!(Missing::of(&old, false), Some(Missing::OldApp));
    }
}
