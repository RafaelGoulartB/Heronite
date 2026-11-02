//! What the Linux and Windows trays share: the menu's rows and what they ask
//! for, what a click on the icon asks for, the icon's pictures.

use heronite_data::DataPaths;
use heronite_i18n::tr;
use image::RgbaImage;

use super::TrayCmd;
use crate::system::settings::recent_games;

const ICON_DARK: &[u8] = include_bytes!("../../../assets/tray/icon-dark.png");
const ICON_LIGHT: &[u8] = include_bytes!("../../../assets/tray/icon-light.png");

/// One row of the menu, before it becomes an item of the system's tray.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Recent { app_name: String, title: String },
    Separator,
    ShowHide { window_open: bool },
    Quit,
}

/// Heroic's order: recent games, separator, Show, Quit.
pub fn menu_rows(recent: &[(String, String)], window_open: bool) -> Vec<Row> {
    let mut rows: Vec<Row> = recent.iter().map(|(a, t)| Row::Recent { app_name: a.clone(), title: t.clone() }).collect();
    if !rows.is_empty() {
        rows.push(Row::Separator);
    }
    rows.push(Row::ShowHide { window_open });
    rows.push(Row::Quit);
    rows
}

/// The rows of a menu that opens now, with the recent games on disk.
pub fn current_rows(data_paths: &DataPaths, max_recent: usize, window_open: bool) -> Vec<Row> {
    menu_rows(&recent_games(data_paths, max_recent), window_open)
}

/// A click on the icon: hide or show when closing the window keeps the app,
/// else show.
pub fn click_cmd(exit_to_tray: bool) -> TrayCmd {
    if exit_to_tray {
        TrayCmd::Toggle
    } else {
        TrayCmd::Show
    }
}

/// The label of the Show/Hide row and what it asks for. It says what the
/// click does: "Hide" only makes sense when closing the window keeps the app
/// alive.
pub fn show_hide(window_open: bool, exit_to_tray: bool) -> (&'static str, TrayCmd) {
    if window_open && exit_to_tray {
        (tr!("Hide"), TrayCmd::Toggle)
    } else {
        (tr!("Show"), TrayCmd::Show)
    }
}

/// The icon (64 px) at each of `sizes` (px, square); empty if it cannot be
/// decoded.
pub fn icon_images(dark: bool, sizes: &[u32]) -> Vec<RgbaImage> {
    let bytes = if dark { ICON_DARK } else { ICON_LIGHT };
    let Ok(img) = image::load_from_memory(bytes) else { return Vec::new() };
    let rgba = img.to_rgba8();
    sizes
        .iter()
        .map(|&size| {
            if rgba.width() == size {
                rgba.clone()
            } else {
                image::imageops::resize(&rgba, size, size, image::imageops::FilterType::Lanczos3)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_follow_heroic_order() {
        let recent = vec![("a".to_string(), "A".to_string()), ("b".to_string(), "B".to_string())];
        assert_eq!(
            menu_rows(&recent, false),
            vec![
                Row::Recent { app_name: "a".into(), title: "A".into() },
                Row::Recent { app_name: "b".into(), title: "B".into() },
                Row::Separator,
                Row::ShowHide { window_open: false },
                Row::Quit
            ]
        );
        // No recent games: no separator at the top.
        assert_eq!(menu_rows(&[], true), vec![Row::ShowHide { window_open: true }, Row::Quit]);
    }

    #[test]
    fn show_hide_says_what_the_click_does() {
        assert_eq!(show_hide(true, true), ("Hide", TrayCmd::Toggle));
        // Closing would quit: the row only shows the window.
        assert_eq!(show_hide(true, false), ("Show", TrayCmd::Show));
        assert_eq!(show_hide(false, true), ("Show", TrayCmd::Show));
        assert_eq!(click_cmd(true), TrayCmd::Toggle);
        assert_eq!(click_cmd(false), TrayCmd::Show);
    }

    #[test]
    fn icons_come_at_each_size() {
        for dark in [true, false] {
            let images = icon_images(dark, &[16, 64]);
            assert_eq!(images.iter().map(|i| (i.width(), i.height())).collect::<Vec<_>>(), vec![(16, 16), (64, 64)]);
            // Something is drawn (not fully transparent).
            assert!(images[1].pixels().any(|p| p[3] > 0));
        }
    }
}
