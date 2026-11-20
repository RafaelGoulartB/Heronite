//! "Get more sources": the gallery of the extensions the lists offer, and
//! the installer that every extension goes through before it is installed,
//! from a list or from a file. One dialog with two faces:
//!
//! - The gallery (`gallery.rs`): a search field and filters by kind (and
//!   "Updates N"), the entries on the left, the picked one's panel on the
//!   right (what it is allowed to do, what it fills, where it comes from,
//!   Install / Update). Its footer says which lists were read and when, and
//!   holds "Refresh", "Install from a file…", "Lists…" (the inline manager,
//!   `lists_manager.rs`) and Done.
//! - The installer (`confirm.rs`): what the package is and may do, built
//!   from `inspect` (or from the entry's files once downloaded and checked),
//!   where it comes from, and Cancel · Install (Update, the same or an
//!   older version: worded as such). Opened from the gallery it has a back
//!   arrow; from Settings → Metadata's "Install from a file…" it stands
//!   alone, and a package that does not read shows why in plain words.
//!
//! Installing announces the extension to the hub, which reads the rows
//! again; Settings → Metadata opens and scrolls to the new row. The work
//! runs off the UI thread (`actions.rs`). `HERONITE_DEBUG_EXTENSIONS`
//! makes the lists up (`fake.rs`); `HERONITE_DEBUG_DIALOG=sources[:<id>]`
//! opens the gallery and `extension-file:<path>` the installer.

mod actions;
mod confirm;
mod fake;
mod gallery;
mod lists_manager;
pub mod permissions;
mod state;
pub mod words;

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_metadata_ext::lists::{Origin, Prepared};
use heronite_metadata_ext::{ExtensionError, PackageInfo};

pub use state::{ExtensionsState, Offer, OfferState, Snapshot, SourcesRequest};

use super::metadata_wizard::{hub, MetadataHub};
use super::DialogEvent;
use crate::ui::dialog::{dialog, dialog_scrolled_with_back, DialogProps};
use crate::ui::fields::new_input;

/// The filters above the gallery's entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Filter {
    All,
    Kind(heronite_metadata::SourceKind),
    Updates,
}

/// Where the package being confirmed comes from.
enum Package {
    File(PathBuf),
    List { prepared: Arc<Prepared>, origin: Origin },
}

/// The installer's face.
struct Confirm {
    from: Package,
    /// Read (`None` while it is read, or when it could not be).
    info: Option<Box<PackageInfo>>,
    /// Why it cannot be read or installed.
    error: Option<ExtensionError>,
    installing: bool,
    /// Opened from the gallery: the back arrow returns to it.
    back: bool,
    /// Keep the values saved for its settings (asked when it comes from
    /// another author or connects to new hosts).
    keep_settings: bool,
}

enum Face {
    Gallery,
    Confirm(Box<Confirm>),
}

pub struct ExtensionsDialog {
    hub: Entity<MetadataHub>,
    face: Face,
    query: Entity<InputState>,
    filter: Filter,
    /// The id of the entry shown on the right.
    picked: Option<String>,
    /// "Lists…" is open.
    lists_open: bool,
    list_input: Entity<InputState>,
    list_error: Option<String>,
    /// The entry whose files are being downloaded and checked.
    preparing: Option<String>,
    /// The last entry that could not be installed, and why.
    entry_error: Option<(String, ExtensionError)>,
    scroll: ScrollHandle,
}

impl EventEmitter<DialogEvent> for ExtensionsDialog {}

impl ExtensionsDialog {
    fn build(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let hub = hub(cx);
        cx.observe(&hub, |_, _, cx| cx.notify()).detach();
        let query = new_input(window, cx, "", tr!("Search sources"));
        cx.subscribe_in(&query, window, |_, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        let list_input = new_input(window, cx, "", "https://");
        cx.subscribe_in(&list_input, window, |this: &mut Self, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { .. }) {
                this.add_list(window, cx);
            }
        })
        .detach();
        Self {
            hub,
            face: Face::Gallery,
            query,
            filter: Filter::All,
            picked: None,
            lists_open: false,
            list_input,
            list_error: None,
            preparing: None,
            entry_error: None,
            scroll: ScrollHandle::new(),
        }
    }

    /// The gallery, reading the lists from the network; `pick` shows that
    /// entry first.
    pub fn gallery(pick: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::build(window, cx);
        let fake = this.hub.read(cx).extensions.fake.clone();
        this.picked = pick.or_else(|| fake.as_ref().and_then(|f| f.pick()));
        if let Some(f) = &fake {
            this.lists_open = f.has("lists");
            if let Some(id) = this.picked.clone() {
                if f.has("busy") {
                    this.preparing = Some(id);
                } else if f.has("error") {
                    this.entry_error = Some((id, ExtensionError::Download("made up".into())));
                }
            }
        }
        this.hub.update(cx, |h, cx| h.read_extensions(true, cx));
        this
    }

    /// The installer of the package at `path` (a zip, or a folder).
    pub fn file(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::build(window, cx);
        this.open_file(path, false, cx);
        this
    }

    /// Esc goes back to the gallery from the installer, and closes "Lists…".
    pub fn on_escape(&mut self, cx: &mut Context<Self>) -> bool {
        match &self.face {
            Face::Confirm(c) if c.back && !c.installing => {
                self.face = Face::Gallery;
                cx.notify();
                true
            }
            Face::Gallery if self.lists_open => {
                self.lists_open = false;
                cx.notify();
                true
            }
            _ => false,
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(DialogEvent::Close);
    }
}

impl Render for ExtensionsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let area = crate::ui::content_size(window);
        let (vw, vh) = (f32::from(area.width), f32::from(area.height));
        let entity = cx.entity().downgrade();
        let on_close = move |_: &mut Window, cx: &mut App| {
            entity.update(cx, |_, cx| cx.emit(DialogEvent::Close)).ok();
        };
        match &self.face {
            Face::Gallery => {
                let body = self.render_gallery((vh * 0.62).clamp(320., 540.), cx);
                let footer = self.render_gallery_footer(cx);
                dialog(
                    DialogProps {
                        id: "extensions".into(),
                        title: tr!("Get more sources").into(),
                        width: 1020f32.min(vw * 0.96),
                        height: None,
                    },
                    body.into_any_element(),
                    Some(footer.into_any_element()),
                    on_close,
                )
            }
            Face::Confirm(c) => {
                let back = c.back;
                let title = self.confirm_title();
                let body = self.render_confirm(cx);
                let footer = self.render_confirm_footer(cx);
                let props = DialogProps { id: "extension-install".into(), title: title.into(), width: 640f32.min(vw * 0.96), height: None };
                if back {
                    let entity = cx.entity().downgrade();
                    dialog_scrolled_with_back(
                        props,
                        &self.scroll,
                        body.into_any_element(),
                        Some(footer.into_any_element()),
                        move |_, cx| {
                            entity
                                .update(cx, |d, cx| {
                                    d.on_escape(cx);
                                })
                                .ok();
                        },
                        on_close,
                    )
                } else {
                    crate::ui::dialog::dialog_scrolled(
                        props,
                        &self.scroll,
                        body.into_any_element(),
                        Some(footer.into_any_element()),
                        on_close,
                    )
                }
            }
        }
    }
}
