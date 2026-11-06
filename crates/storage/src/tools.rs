//! The stores' command-line tools where there is no Wine (Windows): the
//! copy of nile the app downloaded into `<data>/tools/bin`, and GOG's shared
//! redistributables. A copy the app does not run (the settings name another
//! one, or no store needs it any more, like legendary and gogdl since Epic
//! and GOG are native) is an old download.

use heronite_i18n::tr;
use heronite_launch::plan::PlanContext;
use heronite_launch::store_tools::{StoreTool, NILE};
use heronite_launch::SystemProbe;

use crate::{Ctx, Group, Item, Kind, State};

/// Each tool with the store it is for.
const TOOLS: [(StoreTool, &str); 1] = [(NILE, "Amazon Games")];

/// Tools the app downloaded once and no store runs any more.
const RETIRED: [(&str, &str); 2] = [("legendary", "Epic Games"), ("gogdl", "GOG")];

pub(crate) fn items(ctx: &Ctx) -> Vec<Item> {
    let plan = PlanContext::load(&ctx.data_paths);
    let mut out: Vec<Item> = TOOLS
        .iter()
        .filter_map(|(tool, store)| {
            let bin = tool.data_bin(&ctx.data_paths);
            if !bin.is_file() {
                return None;
            }
            let runs = tool.find(&plan, &SystemProbe).is_some_and(|found| std::path::Path::new(&found) == bin);
            let mut item = if runs {
                Item::new(Group::Tools, Kind::Runtime, State::Needed, tool.name, bin)
            } else {
                Item::new(Group::Downloads, Kind::Download, State::Unused, tool.name, bin)
            };
            item.source = Some(store.to_string());
            Some(item)
        })
        .collect();
    for (name, store) in RETIRED {
        let bin = ctx.data_paths.tools_dir().join("bin").join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        if bin.is_file() {
            let mut item = Item::new(Group::Downloads, Kind::Download, State::Unused, name, bin);
            item.source = Some(store.to_string());
            out.push(item);
        }
    }
    let redist = ctx.data_paths.gog_redist_dir();
    if redist.is_dir() {
        let mut item = Item::new(Group::Tools, Kind::Runtime, State::Needed, tr!("GOG redistributables"), redist);
        item.source = Some("GOG".into());
        out.push(item);
    }
    out
}
