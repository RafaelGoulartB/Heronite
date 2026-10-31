//! The stores' command-line tools where there is no Wine (Windows): the
//! copies of legendary, gogdl and nile the app downloaded into
//! `<data>/tools/bin`, and GOG's shared redistributables. A copy the app
//! does not run (the settings name another one) is an old download.

use heronite_i18n::tr;
use heronite_launch::plan::PlanContext;
use heronite_launch::store_tools::{StoreTool, GOGDL, LEGENDARY, NILE};
use heronite_launch::SystemProbe;

use crate::{Ctx, Group, Item, Kind, State};

/// Each tool with the store it is for.
const TOOLS: [(StoreTool, &str); 3] = [(LEGENDARY, "Epic Games"), (GOGDL, "GOG"), (NILE, "Amazon Games")];

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
    let redist = ctx.data_paths.gog_redist_dir();
    if redist.is_dir() {
        let mut item = Item::new(Group::Tools, Kind::Runtime, State::Needed, tr!("GOG redistributables"), redist);
        item.source = Some("GOG".into());
        out.push(item);
    }
    out
}
