//! What Settings → Saves and a game's Saves tab show when Ludusavi is not
//! installed: what it is, where to download it and how to install it so
//! Heronite finds it, then "Look again".
//!
//! Heronite runs the `ludusavi` program itself (`$PATH`, `/usr/bin`,
//! `/usr/local/bin`, `~/.local/bin`). The Flatpak is not found there, and
//! its sandbox would not see the restore's temporary config, so the
//! download page's Linux build is the one to point at. On Windows it is the
//! `-win64.zip` (one `ludusavi.exe`), unpacked where Heronite looks, or
//! winget's or Scoop's package.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use crate::console::list::Row;
use crate::icons::Icon;
use crate::theme;
use crate::ui::controls::{action_button, card_box, help_text, ButtonKind};
use crate::ui::fields::meta_text;
use crate::ui::icon;

/// Ludusavi's releases: the Linux build is a `.tar.gz` with one `ludusavi` file.
pub const DOWNLOAD_URL: &str = "https://github.com/mtkennerly/ludusavi/releases/latest";

/// Opens the download page in the browser.
pub fn open_download() {
    if let Err(e) = heronite_platform::shell::open(DOWNLOAD_URL) {
        log::warn!("opening {DOWNLOAD_URL}: {e}");
    }
}

fn title() -> &'static str {
    tr!("Ludusavi is not installed")
}

fn why() -> &'static str {
    tr!("Heronite backs up and restores game saves with Ludusavi, a free program that knows where thousands of games keep their saves. Install it once and saves are backed up every time you close a game.")
}

fn steps() -> [&'static str; 3] {
    if cfg!(windows) {
        // `heronite_emulation::ludusavi` looks there, in winget's and Scoop's folders and on `PATH`.
        return [
            tr!("1. Download the Windows version (the file ending in -win64.zip)."),
            tr!("2. Extract ludusavi.exe into %LOCALAPPDATA%\\Programs\\Ludusavi, or install it with winget install mtkennerly.ludusavi or scoop install ludusavi."),
            tr!("3. Come back here and press Look again."),
        ];
    }
    [
        tr!("1. Download the Linux version (the file ending in -linux.tar.gz)."),
        tr!("2. Extract the ludusavi file into ~/.local/bin, or install the ludusavi package of your distribution."),
        tr!("3. Come back here and press Look again."),
    ]
}

fn flatpak_note() -> &'static str {
    if cfg!(windows) {
        return tr!("Installed somewhere else? Set the program in Backup location & format.");
    }
    tr!("The Flatpak version is not found: Heronite runs the ludusavi program itself. Installed somewhere else? Set the program in Backup location & format.")
}

/// The desktop card. `id` keeps the buttons apart when shown twice.
pub fn missing_card(id: &str, look_again: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Div {
    let mut steps_col = div().flex().flex_col().gap(px(4.));
    for s in steps() {
        steps_col = steps_col.child(div().text_size(px(13.)).line_height(px(18.)).text_color(theme::text_primary()).child(s));
    }
    card_box()
        .py(px(14.))
        .gap(px(10.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(icon(Icon::Warning, px(18.), theme::accent()))
                .child(div().text_size(px(15.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(title())),
        )
        .child(help_text(why()))
        .child(steps_col)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .mt(px(2.))
                .child(action_button(
                    SharedString::from(format!("{id}-download")),
                    ButtonKind::Primary,
                    tr!("Download Ludusavi"),
                    true,
                    |_, _, _| open_download(),
                ))
                .child(action_button(
                    SharedString::from(format!("{id}-look-again")),
                    ButtonKind::Glass,
                    tr!("Look again"),
                    true,
                    look_again,
                )),
        )
        .child(meta_text(flatpak_note()))
}

/// The same for console mode: `{prefix}-download` opens the page and
/// `{prefix}-look-again` is the caller's to handle.
pub fn missing_rows(prefix: &str) -> Vec<Row> {
    vec![
        Row::info(format!("{prefix}-missing"), title(), "").danger().desc(why()),
        Row::action(format!("{prefix}-download"), tr!("Download Ludusavi")).icon(Icon::OpenInNew).desc(steps().join(" ")),
        Row::action(format!("{prefix}-look-again"), tr!("Look again")).icon(Icon::Refresh).desc(flatpak_note()),
    ]
}
