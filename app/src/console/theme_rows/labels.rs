//! The names of the option values, English keys translated when drawn.

use heronite_data::console_theme::*;
use heronite_i18n::{n_, tr, tr_ctx};

/// The translated names of `all` (a `context|text` name is `text` in English).
pub(super) fn names<E: Copy>(all: &'static [E], name: fn(E) -> &'static str) -> Vec<&'static str> {
    all.iter()
        .map(|e| {
            let key = name(*e);
            match key.split_once('|') {
                Some((_, english)) => tr_ctx(key, english),
                None => tr(key),
            }
        })
        .collect()
}

pub(super) fn index<E: Copy + PartialEq>(all: &[E], v: E) -> usize {
    all.iter().position(|x| *x == v).unwrap_or(0)
}

macro_rules! label_fn {
    ($f:ident, $ty:ident { $($v:ident => $s:expr),+ $(,)? }) => {
        pub fn $f(v: $ty) -> &'static str {
            match v {
                $($ty::$v => $s),+
            }
        }
    };
}

label_fn!(layout_name, Layout {
    Shelves => n_!("Shelves"),
    Spotlight => n_!("Spotlight"),
    Cinema => n_!("Cinema"),
    Dashboard => n_!("Dashboard"),
    Grid => n_!("Grid"),
    Wheel => n_!("Wheel"),
    Carousel => n_!("Carousel"),
    Xbox => n_!("Series X"),
});
label_fn!(start_on_name, StartOn { First => n_!("First tab"), Last => n_!("Last tab used") });
label_fn!(a_button_name, AButton { Details => n_!("Opens the page"), Play => n_!("Plays") });
label_fn!(hero_title_name, HeroTitle { Text => n_!("Text"), Logo => n_!("Logo, if there is one") });
label_fn!(fact_style_name, FactStyle { Line => n_!("One line"), Chips => n_!("Chips and genres") });
label_fn!(hero_align_name, HeroAlign { Left => n_!("Left"), Center => n_!("Centred at the top") });
label_fn!(size3_name, Size3 { Small => n_!("Low"), Medium => n_!("Medium"), Large => n_!("High") });
label_fn!(tile_shape_name, TileShape { Portrait => n_!("Portrait"), Landscape => n_!("Landscape"), Square => n_!("shape|Square") });
label_fn!(tile_size_name, TileSize { Small => n_!("Small"), Medium => n_!("Medium"), Large => n_!("Large"), Huge => n_!("Extra large") });
label_fn!(tile_titles_name, TileTitles {
    Never => n_!("Never"),
    Focused => n_!("On the focused one"),
    Always => n_!("Always"),
    Bar => n_!("In a strip under the cover"),
});
label_fn!(badges_name, Badges { Off => n_!("None"), Running => n_!("Running"), All => n_!("All") });
label_fn!(corners_name, Corners { Square => n_!("Square"), Soft => n_!("Soft"), Round => n_!("Round") });
label_fn!(focus_name, FocusStyle { Ring => n_!("Ring"), Glow => n_!("Glow"), Lift => n_!("Lift"), Underline => n_!("Underline") });
label_fn!(background_name, BackgroundSource {
    Art => n_!("The game's art"),
    Cover => n_!("Blurred cover"),
    Screenshots => n_!("The game's screenshots"),
    GameColour => n_!("Colour of the game"),
    Solid => n_!("Solid colour"),
    Image => n_!("Your own picture"),
});
label_fn!(bg_motion_name, BackgroundMotion { Still => n_!("Still"), Drift => n_!("Slow drift") });
label_fn!(font_name, ConsoleFont {
    Heronite => n_!("Heronite"),
    Cabin => n_!("Cabin"),
    Rubik => n_!("Rubik"),
    System => n_!("System"),
    Mono => n_!("Monospace"),
});
label_fn!(tab_bar_name, TabBar {
    Top => n_!("Along the top"),
    Bottom => n_!("Along the bottom, with icons"),
    Title => n_!("Only the current one, centred"),
    Names => n_!("Names only, no buttons"),
});
label_fn!(spot_actions_name, SpotActions { Full => n_!("Play, status, favourite, more"), Play => n_!("Play and more") });
label_fn!(headings_name, Headings { Normal => n_!("As written"), Spaced => n_!("Spaced capitals") });
label_fn!(clock_name, Clock { H24 => n_!("24 h"), H12 => n_!("12 h"), Off => n_!("Hidden") });
label_fn!(motion_name, Motion { Full => n_!("Full"), Reduced => n_!("Reduced"), Off => n_!("Off") });
label_fn!(glyphs_name, Glyphs {
    Auto => n_!("Automatic (the controller's)"),
    Xbox => n_!("Xbox"),
    PlayStation => n_!("PlayStation"),
    Nintendo => n_!("Nintendo"),
    Deck => n_!("Steam Deck"),
    Keyboard => n_!("Keyboard"),
});
label_fn!(hints_name, Hints { Always => n_!("Always"), Auto => n_!("Fades by itself"), Off => n_!("Hidden") });
label_fn!(game_page_name, GamePage { Classic => n_!("Classic"), Hero => n_!("Hero"), Tabs => n_!("Tabs"), Sheet => n_!("Sheet") });
label_fn!(menu_style_name, MenuStyle { Sidebar => n_!("Sidebar"), Grid => n_!("Grid"), Dock => n_!("Dock"), Rail => n_!("Rail of icons") });
label_fn!(side_name, Side { Left => n_!("Left"), Right => n_!("Right") });
label_fn!(grid_panel_name, GridPanel { Title => n_!("Name above"), Right => n_!("Panel on the right") });
label_fn!(wheel_items_name, WheelItems { Name => n_!("Name"), Logo => n_!("Logo") });
label_fn!(open_in_name, OpenIn { Last => n_!("As I left it"), Console => n_!("In console mode"), Desktop => n_!("In desktop mode") });
label_fn!(auto_console_name, AutoConsole { Off => n_!("Nothing"), Ask => n_!("Ask"), On => n_!("Enter console mode") });
label_fn!(idle_name, Idle { Off => n_!("Nothing"), Attract => n_!("Show the games by themselves"), Dim => n_!("Dim the screen") });
