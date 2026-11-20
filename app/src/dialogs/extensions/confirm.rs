//! The installer's face: what the package is (name, version, author, kind,
//! the games it is for, what it fills), what it is allowed to do, the
//! settings it asks for, where it comes from and what that guarantees,
//! then Cancel · Install. A newer version reads "Update <name> 1.0.0 →
//! 1.1.0"; the same or an older one says so before it goes ahead. One made
//! by someone else than the one installed, or that connects to new hosts,
//! says so, and asks whether what the user entered in its settings goes to
//! it (not by default when its author changed). A package that does not
//! read says why in plain words, with the engine's words under it for its
//! author.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_metadata_ext::lists::Origin;
use heronite_metadata_ext::{InstallAction, PackageInfo};

use super::permissions::{self, Asks};
use super::{words, Confirm, ExtensionsDialog, Face, Package};
use crate::icons::Icon;
use crate::pages::settings::metadata_labels as labels;
use crate::pages::{status_pill, tag_pill};
use crate::theme;
use crate::ui::controls::{action_button, button, toggle_row, ButtonKind};
use crate::ui::fields::{field_label, meta_text};
use crate::ui::icon;

/// The title for a package read.
pub(super) fn title_of(info: &PackageInfo) -> String {
    let name = info.name.clone();
    match (info.action, info.installed.clone()) {
        (InstallAction::Update, Some(from)) => tr!("Update {name} {from} \u{2192} {to}", name, from, to = info.version.clone()),
        (InstallAction::Reinstall, _) => tr!("Install {name} {version} again", name, version = info.version.clone()),
        (InstallAction::Downgrade, _) => tr!("Go back to {name} {version}", name, version = info.version.clone()),
        _ => tr!("Install {name}", name),
    }
}

/// What replacing the installed version does, for anything but a new install.
fn replacing(info: &PackageInfo) -> Option<String> {
    let installed = info.installed.clone().unwrap_or_default();
    if info.action != InstallAction::New && info.asks_about_settings() {
        // Whether its settings stay is the question under it.
        return Some(tr!("It replaces version {installed}. On or off and its place stay.", installed));
    }
    Some(match info.action {
        InstallAction::New => return None,
        InstallAction::Update => {
            tr!("It replaces version {installed}. What you set for it (on or off, its settings, its place) stays.", installed)
        }
        InstallAction::Reinstall => {
            tr!("Version {installed} is installed already: this puts its files back. What you set for it stays.", installed)
        }
        InstallAction::Downgrade => {
            tr!("Version {installed} is installed: this replaces it with an older one. What you set for it stays.", installed)
        }
    })
}

/// The primary button.
fn install_label(action: InstallAction) -> &'static str {
    match action {
        InstallAction::New => tr!("Install"),
        InstallAction::Update => tr!("Update"),
        InstallAction::Reinstall => tr!("Install again"),
        InstallAction::Downgrade => tr!("Install the older version"),
    }
}

fn origin_of(c: &Confirm) -> Origin {
    match &c.from {
        Package::File(_) => Origin::File,
        Package::List { origin, .. } => origin.clone(),
    }
}

fn trust(origin: &Origin) -> Div {
    let c = if words::cautious(origin) { theme::readable(theme::warning()) } else { theme::text_muted().into() };
    div()
        .flex()
        .items_start()
        .gap(px(10.))
        .p(px(12.))
        .rounded(px(12.))
        .bg(if words::cautious(origin) { Hsla { a: 0.08, ..theme::warning().into() } } else { theme::ink_a(0.04) })
        .child(div().pt(px(1.)).child(icon(if words::cautious(origin) { Icon::Warning } else { Icon::Check }, px(16.), c)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .min_w_0()
                .child(div().text_size(px(13.)).text_color(theme::text_primary()).child(words::origin(origin)))
                .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_muted()).child(words::guarantee(origin))),
        )
}

fn error_block(e: &heronite_metadata_ext::ExtensionError) -> Div {
    div().flex().items_start().gap(px(10.)).child(div().pt(px(1.)).child(icon(Icon::Warning, px(16.), theme::danger_text()))).child(
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .min_w_0()
            .child(div().text_size(px(13.5)).line_height(px(19.)).text_color(theme::danger_text()).child(words::error(e)))
            .children(words::detail(e).map(|d| div().text_size(px(11.5)).line_height(px(15.)).text_color(theme::text_dim()).child(d))),
    )
}

/// What changed since the version installed (or removed with its settings
/// kept): another author, or one that cannot be told, new hosts.
fn change_lines(info: &PackageInfo) -> Vec<String> {
    let mut lines = Vec::new();
    if info.author_changed {
        lines.push(match info.installed_author.clone() {
            Some(author) => tr!("Made by someone other than the one installed ({author}).", author),
            None => tr!("The version installed no longer reads, so who made it is not known.").into(),
        });
    }
    if !info.new_hosts.is_empty() {
        lines.push(tr!("Connects to new hosts: {hosts}", hosts = info.new_hosts.join(", ")));
    }
    lines
}

/// [`change_lines`], calm but plain.
fn changes(info: &PackageInfo) -> Option<Div> {
    let lines = change_lines(info);
    if lines.is_empty() {
        return None;
    }
    let c = theme::readable(theme::warning());
    Some(div().flex().flex_col().gap(px(4.)).children(lines.into_iter().map(|t| {
        div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(div().pt(px(1.)).child(icon(Icon::Warning, px(14.), c)))
            .child(div().text_size(px(12.5)).line_height(px(17.)).text_color(theme::text_primary()).child(t))
    })))
}

fn package(info: &PackageInfo) -> Div {
    let d = &info.descriptor;
    let mut pills = div().flex().flex_wrap().gap(px(6.)).child(tag_pill(words::kind(info.kind))).child(tag_pill(labels::audience(d)));
    if info.adult {
        pills = pills.child(status_pill(tr!("Adult content"), theme::warning()));
    }
    let mut fills = div().flex().flex_wrap().gap(px(6.));
    for f in &info.provides {
        fills = fills.child(tag_pill(labels::field_name(f)));
    }
    let about = [Some(info.description.clone()), labels::when_asked(d)].into_iter().flatten();
    div()
        .flex()
        .flex_col()
        .gap(px(14.))
        .child(
            div().flex().items_center().gap(px(12.)).child(labels::mark(&info.id, &info.name, 48.)).child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(theme::W_BOLD)
                            .text_color(theme::text_primary())
                            .truncate()
                            .child(info.name.clone()),
                    )
                    .child(div().text_size(px(12.)).text_color(theme::text_dim()).truncate().child(tr!(
                        "{author} \u{b7} version {version}",
                        author = info.author.clone(),
                        version = info.version.clone()
                    ))),
            ),
        )
        .children(
            replacing(info).map(|t| div().text_size(px(12.5)).line_height(px(17.)).text_color(theme::readable(theme::accent())).child(t)),
        )
        .children(changes(info))
        .child(pills)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .children(about.map(|t| div().text_size(px(13.)).line_height(px(19.)).text_color(theme::text_primary()).child(t))),
        )
        .child(permissions::block(&Asks::of_package(info)))
        .child(div().flex().flex_col().gap(px(6.)).child(field_label(tr!("Fills"))).child(fills))
}

impl ExtensionsDialog {
    pub(super) fn confirm_title(&self) -> String {
        let Face::Confirm(c) = &self.face else { return String::new() };
        match (&c.info, &c.error) {
            (Some(info), _) => title_of(info),
            (None, Some(_)) => tr!("This source cannot be installed").into(),
            (None, None) => tr!("Install a source").into(),
        }
    }

    /// Whether what the user entered in its settings goes to this package.
    fn keep_settings_choice(&self, c: &Confirm, cx: &mut Context<Self>) -> Div {
        div().child(
            toggle_row(
                "ext-keep-settings",
                tr!("Keep what you entered in its settings, keys included"),
                Some(tr!("Only if you trust this package with them: it can send them to the hosts it connects to.")),
                c.keep_settings,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if let Face::Confirm(c) = &mut this.face {
                    c.keep_settings = !c.keep_settings;
                }
                cx.notify();
            })),
        )
    }

    pub(super) fn render_confirm(&self, cx: &mut Context<Self>) -> Div {
        let Face::Confirm(c) = &self.face else { return div() };
        let mut col = div().flex().flex_col().gap(px(16.)).pb(px(4.));
        match &c.info {
            None if c.error.is_none() => return col.child(meta_text(tr!("Reading the package\u{2026}"))),
            None => {
                return col
                    .children(c.error.as_ref().map(error_block))
                    .child(meta_text(tr!("Nothing was installed. Choose another file, or ask the source's author for one that works.")));
            }
            Some(info) => {
                col = col.child(package(info));
                if info.asks_about_settings() {
                    col = col.child(self.keep_settings_choice(c, cx));
                }
                col = col.child(trust(&origin_of(c)));
                if !info.warnings.is_empty() {
                    col = col.child(meta_text(tr!("Its source.toml has keys this Heronite does not know; they are left out.")));
                }
                col = col
                    .child(div().text_size(px(11.)).text_color(theme::text_dim()).child(tr!("SHA-256 {sha}", sha = info.sha256.clone())));
            }
        }
        col.children(c.error.as_ref().map(error_block))
    }

    pub(super) fn render_confirm_footer(&self, cx: &mut Context<Self>) -> Div {
        let Face::Confirm(c) = &self.face else { return div() };
        let back = c.back;
        let cancel = button("ext-cancel", ButtonKind::Glass, if c.info.is_some() { tr!("Cancel") } else { tr!("Close") }).on_click(
            cx.listener(move |this, _, _, cx| {
                if back {
                    this.on_escape(cx);
                } else {
                    this.close(cx);
                }
            }),
        );
        let row = div().flex().flex_1().items_center().justify_end().gap(px(8.));
        match &c.info {
            Some(info) => {
                let label = if c.installing { tr!("Installing\u{2026}") } else { install_label(info.action) };
                row.child(cancel).child(action_button(
                    "ext-install",
                    ButtonKind::Primary,
                    label,
                    !c.installing,
                    cx.listener(|this, _, _, cx| this.install(cx)),
                ))
            }
            None => {
                let again = matches!(c.from, Package::File(_)) && c.error.is_some();
                row.when(again, |r| {
                    r.child(
                        button("ext-other-file", ButtonKind::Ghost, tr!("Choose another file\u{2026}"))
                            .on_click(cx.listener(move |this, _, window, cx| this.pick_file(back, window, cx))),
                    )
                })
                .child(cancel)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{install_label, replacing, title_of, InstallAction, PackageInfo};

    fn info(action: InstallAction, installed: Option<&str>) -> PackageInfo {
        let pkg = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extensions/metadata/wikipedia");
        let svc = heronite_metadata::MetadataService::new(heronite_data::paths::DataPaths::at(tempfile::tempdir().unwrap().path()));
        use heronite_metadata_ext::ExtensionsExt;
        let mut info = svc.inspect_extension(&pkg).unwrap();
        info.action = action;
        info.installed = installed.map(Into::into);
        info
    }

    #[test]
    fn replacing_one_says_from_which_version() {
        assert_eq!(title_of(&info(InstallAction::New, None)), "Install Wikipedia");
        let mut i = info(InstallAction::Update, Some("0.9.0"));
        assert_eq!(title_of(&i), "Update Wikipedia 0.9.0 \u{2192} 1.0.0");
        assert!(replacing(&i).unwrap().contains("0.9.0"));
        i.action = InstallAction::Reinstall;
        assert_eq!(title_of(&i), "Install Wikipedia 1.0.0 again");
        assert_eq!(install_label(i.action), "Install again");
        i.action = InstallAction::Downgrade;
        assert_eq!(title_of(&i), "Go back to Wikipedia 1.0.0");
        assert!(replacing(&info(InstallAction::New, None)).is_none());
    }

    #[test]
    fn another_author_or_new_hosts_are_said() {
        let mut i = info(InstallAction::Update, Some("0.9.0"));
        assert!(super::changes(&i).is_none());
        i.author_changed = true;
        i.installed_author = Some("Alice".into());
        i.new_hosts = vec!["collect.evil.example".into()];
        assert!(super::changes(&i).is_some());
        // Its settings are the question under it, not promised to stay.
        i.saved_settings = true;
        assert!(i.asks_about_settings() && !i.keeps_settings_by_default());
        assert!(!replacing(&i).unwrap().contains("settings"));
    }

    #[test]
    fn an_installed_version_that_no_longer_reads_is_said() {
        let mut i = info(InstallAction::New, None);
        i.author_changed = true;
        i.installed_author = None;
        i.saved_settings = true;
        assert_eq!(super::change_lines(&i), ["The version installed no longer reads, so who made it is not known."]);
        assert!(i.asks_about_settings() && !i.keeps_settings_by_default());
        i.installed_author = Some("Alice".into());
        assert_eq!(super::change_lines(&i), ["Made by someone other than the one installed (Alice)."]);
    }
}
