//! Settings → Storage in console mode: the disks, the "Free up space"
//! choices, then a row per thing with its size. A on a game uninstalls it (its
//! own dialog asks); on anything else that can go, A arms the row and a
//! second A removes it.

use gpui_kit::*;
use heronite_data::maintenance::human_size;
use heronite_i18n::tr;
use heronite_storage::{Group, Item, Kind, State as Use, Sweep};

use super::{group_label, item_meta, remove, size_text, sweep_label, State};
use crate::console::list::{Outcome, Row, Value};
use crate::icons::Icon;
use crate::pages::settings::SettingsPage;
use crate::pages::PageEvent;

const SWEEP: &str = "sweep";

fn item_icon(item: &Item) -> Icon {
    match item.kind {
        Kind::Game { .. } => Icon::SportsEsports,
        Kind::Prefix { .. } | Kind::Build { .. } => Icon::WineBar,
        Kind::Runtime => Icon::Build,
        Kind::SteamPrefix { .. } | Kind::SteamShaders { .. } => Icon::SportsEsports,
        Kind::Cache(_) => Icon::Refresh,
        Kind::Leftover => Icon::Folder,
        Kind::Download => Icon::Download,
    }
}

fn sorted<'a>(st: &State, mut items: Vec<&'a Item>) -> Vec<&'a Item> {
    items.sort_by_key(|i| (i.state != Use::Unused, std::cmp::Reverse(st.own(i).unwrap_or(0))));
    items
}

pub fn console_rows(d: &SettingsPage) -> Vec<Row> {
    let st = &d.storage;
    let Some(inv) = st.inv.as_ref() else { return vec![Row::info("loading", tr!("Looking at your disks\u{2026}"), "").enabled(false)] };
    let mut rows = vec![Row::header(tr!("Disks"))];
    for (i, disk) in inv.disks.iter().enumerate() {
        let parts: Vec<String> = Group::ALL
            .iter()
            .filter_map(|g| {
                let bytes: u64 = st.items().filter(|it| it.group == *g && it.disk == Some(i)).filter_map(|it| st.own(it)).sum();
                (bytes > 0).then(|| format!("{} {}", group_label(*g), human_size(bytes)))
            })
            .collect();
        rows.push(
            Row::info(
                SharedString::from(format!("disk:{i}")),
                disk.name(),
                tr!("{free} free of {total}", free = human_size(disk.free), total = human_size(disk.total)),
            )
            .icon(Icon::FaHardDrive)
            .desc(parts.join(" \u{b7} ")),
        );
    }

    rows.push(Row::header(tr!("Free up space")));
    for s in Sweep::ALL {
        let (items, bytes) = st.sweep_parts(s);
        if !items.is_empty() {
            rows.push(Row::toggle(SharedString::from(format!("sweep:{s:?}")), sweep_label(s), st.sweeps.contains(&s)).desc(format!(
                "{} \u{b7} {}",
                items.len(),
                human_size(bytes)
            )));
        }
    }
    let ticked = st.ticked_bytes();
    let armed = st.armed.as_deref() == Some(SWEEP);
    let mut free = Row::action(
        SWEEP,
        if armed { tr!("Press A again to free {size}", size = human_size(ticked)) } else { tr!("Free {size}", size = human_size(ticked)) },
    )
    .icon(Icon::DeleteOutline)
    .enabled(ticked > 0 && !st.busy());
    if armed {
        free = free.danger();
    }
    rows.push(free);
    if let Some((done, total)) = st.walking.filter(|(_, t)| *t > 0) {
        rows.push(Row::info("walking", tr!("Measuring {done} of {total}", done, total), "").enabled(false));
    }

    for group in Group::ALL {
        let mut items = st.group_items(group);
        if items.is_empty() {
            continue;
        }
        items = if group == Group::Games {
            items.sort_by_key(|i| std::cmp::Reverse(i.size.unwrap_or(0)));
            items
        } else {
            sorted(st, items)
        };
        rows.push(Row::header(format!("{} \u{b7} {}", group_label(group), human_size(st.group_bytes(group)))));
        for item in items {
            let id = SharedString::from(format!("item:{}", item.id));
            let armed = st.armed.as_ref() == Some(&id);
            let desc = if armed {
                if item.goes_to_trash() && cfg!(windows) {
                    tr!("Press A again to move it to the Recycle Bin").into()
                } else if item.goes_to_trash() {
                    tr!("Press A again to move it to the trash").into()
                } else {
                    tr!("Press A again to delete it").into()
                }
            } else if item.state == Use::Unused {
                format!("{} \u{b7} {}", tr!("Unused"), item_meta(st, item))
            } else {
                item_meta(st, item)
            };
            let can_act = !st.busy() && (item.removable() || matches!(item.kind, Kind::Game { .. }));
            let mut row = Row::info(id, item.title.clone(), size_text(st.own(item))).icon(item_icon(item)).desc(desc).enabled(can_act);
            if armed {
                row = row.danger();
            }
            rows.push(row);
        }
    }
    rows
}

pub fn console_apply(d: &mut SettingsPage, outcome: Outcome, cx: &mut Context<SettingsPage>) {
    match outcome {
        Outcome::Changed(id, Value::Toggle(on)) => {
            if let Some(s) = Sweep::ALL.into_iter().find(|s| id.as_ref() == format!("sweep:{s:?}")) {
                let st = &mut d.storage;
                if on {
                    st.sweeps.insert(s);
                } else {
                    st.sweeps.remove(&s);
                }
                st.armed = None;
            }
        }
        Outcome::Activate(id) if id.as_ref() == SWEEP => {
            if d.storage.armed.as_ref() == Some(&id) {
                let st = &d.storage;
                let ids = st.items().filter(|i| i.sweep().is_some_and(|s| st.sweeps.contains(&s))).map(|i| i.id.clone()).collect();
                remove(d, ids, cx);
            } else {
                d.storage.armed = Some(id);
            }
        }
        Outcome::Activate(id) => {
            let Some(item) = id.strip_prefix("item:").and_then(|i| d.storage.item(i)).cloned() else {
                d.storage.armed = None;
                return;
            };
            if d.storage.busy() {
                return;
            }
            match &item.kind {
                Kind::Game { key } => {
                    d.storage.armed = None;
                    cx.emit(PageEvent::Uninstall(key.clone()));
                }
                _ if !item.removable() => d.storage.armed = None,
                _ if d.storage.armed.as_ref() == Some(&id) => remove(d, vec![item.id.clone()], cx),
                _ => d.storage.armed = Some(id),
            }
        }
        _ => {}
    }
    cx.notify();
}
