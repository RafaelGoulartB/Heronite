//! `header.collection__header`: 42px, three columns (left tools, centred
//! search + filters, right tools).

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::icons::Icon;
use crate::theme;
use crate::ui::{icon, tool_button};

pub fn header() -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(theme::HEADER_H)
        .px(px(12.))
        .gap(px(8.))
        .child(
            div()
                .flex()
                .flex_1()
                .items_center()
                .min_w(px(32.))
                .child(tool_button("hdr-sidebar", Icon::Menu, false))
                .child(tool_button("hdr-install-filter", Icon::FaBorderAll, false)),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .child(search_box())
                .child(tool_button("hdr-facets", Icon::FilterList, false))
                .child(tool_button("hdr-sort", Icon::SortByAlpha, false))
                .child(tool_button("hdr-status", Icon::Flag, false)),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_end()
                .min_w(px(32.))
                .child(tool_button("hdr-refresh", Icon::Refresh, false))
                .child(tool_button("hdr-playnite", Icon::SyncAlt, false))
                .child(tool_button("hdr-emulated", Icon::SportsEsports, false))
                .child(tool_button("hdr-settings", Icon::Settings, false)),
        )
}

/// 20rem × 32px search with an underline only.
fn search_box() -> Div {
    div()
        .relative()
        .flex()
        .items_center()
        .w(px(320.))
        .h(px(32.))
        .border_b_1()
        .border_color(theme::white_a(0.18))
        .child(
            div()
                .absolute()
                .left(px(4.))
                .child(icon(Icon::FaSearch, px(12.8), theme::white_a(0.42))),
        )
        .child(
            div()
                .flex_1()
                .text_center()
                .text_size(px(16.))
                .text_color(theme::white_a(0.38))
                .child("Search for Games"),
        )
}
