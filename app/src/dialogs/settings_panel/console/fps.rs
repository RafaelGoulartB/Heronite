//! Console rows of "FPS limit" (see `settings_panel::fps_limit`). The
//! numbers are picked from common values with ◀ ▶ (a value set on the
//! desktop stays in the list).

use std::rc::Rc;

use gpui_kit::*;
use heronite_i18n::tr;
use heronite_launch::fps_limit::Method;

use super::{item, Act, Item, SettingsPanel};
use crate::console::list::Row;
use crate::dialogs::settings_panel::fps_limit::method_name;

const LIMITS: [u32; 15] = [0, 30, 40, 45, 50, 60, 72, 75, 90, 100, 120, 144, 165, 180, 240];
const NO_FOCUS: [u32; 6] = [0, 5, 10, 15, 20, 30];

impl SettingsPanel {
    /// `steam`: a Steam game, whose rows follow the launch options' lock.
    pub(super) fn c_fps(&self, steam: bool) -> Vec<Item> {
        let enabled = !(steam && !self.steam_fields_editable());
        let f = self.fps();
        let mut v = vec![Self::c_header(tr!("FPS limit"))];
        let desc = tr!("The most frames per second the game draws");
        v.push(number_row("c:fps-limit", tr!("Maximum FPS"), desc, &LIMITS, f.limit, tr!("No limit"), enabled, |f, n| f.limit = n));
        if !f.is_on() {
            return v;
        }
        let effective = self.fps_effective(&f);
        let tools = self.fps_tools();
        let methods = self.fps_methods();
        let labels = methods
            .iter()
            .map(|m| {
                SharedString::from(match (m, self.fps_unavailable(*m, &tools)) {
                    (Method::Auto, _) => match tools.auto() {
                        Some(a) => format!("{} \u{b7} {}", tr!("Automatic"), method_name(a)),
                        None => tr!("Automatic").to_string(),
                    },
                    (m, Some(why)) => format!("{} \u{b7} {why}", method_name(*m)),
                    (m, None) => method_name(*m).to_string(),
                })
            })
            .collect();
        let index = methods.iter().position(|m| *m == f.method).unwrap_or(0);
        let notes = self.fps_notes(&f, effective);
        let mut row = Row::choice("c:fps-method", tr!("Limited by"), labels, index)
            .desc(notes.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join(" "))
            .enabled(enabled);
        if notes.iter().any(|(_, warn)| *warn) {
            row = row.danger();
        }
        v.push(item(
            row,
            Act::Choice(Rc::new(move |this, i, cx| {
                let method = methods[i];
                this.update_fps(cx, |f| f.method = method);
            })),
        ));
        match effective {
            Some(Method::MangoHud) => v.push(item(
                Row::choice(
                    "c:fps-pacing",
                    tr!("Frame pacing"),
                    vec![tr!("Lower latency").into(), tr!("Smoother").into()],
                    f.smooth as usize,
                )
                .desc(tr!("Lower latency: the game reacts sooner. Smoother: more even frame times, for a little more latency."))
                .enabled(enabled),
                Act::Choice(Rc::new(|this, i, cx| this.update_fps(cx, |f| f.smooth = i == 1))),
            )),
            Some(Method::Gamescope) => {
                let desc = tr!("A lower limit while the game is not focused, to save power");
                v.push(number_row(
                    "c:fps-no-focus",
                    tr!("In the background"),
                    desc,
                    &NO_FOCUS,
                    f.no_focus,
                    tr!("Same limit"),
                    enabled,
                    |f, n| f.no_focus = n,
                ));
            }
            _ => {}
        }
        v
    }
}

/// A ◀ ▶ choice of `values` (`0` shown as `zero`) plus `current` when it is
/// not one of them.
fn number_row(
    id: &'static str,
    label: &str,
    desc: &str,
    values: &[u32],
    current: u32,
    zero: &str,
    enabled: bool,
    set: fn(&mut heronite_launch::fps_limit::FpsLimit, u32),
) -> Item {
    let mut list = values.to_vec();
    if !list.contains(&current) {
        list.push(current);
        list.sort_unstable();
    }
    let labels = list.iter().map(|n| SharedString::from(if *n == 0 { zero.to_string() } else { n.to_string() })).collect();
    let index = list.iter().position(|n| *n == current).unwrap_or(0);
    item(
        Row::choice(id, label.to_string(), labels, index).desc(desc.to_string()).enabled(enabled),
        Act::Choice(Rc::new(move |this, i, cx| {
            let n = list[i];
            this.update_fps(cx, |f| set(f, n));
        })),
    )
}
