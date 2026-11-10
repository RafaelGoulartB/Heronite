//! "What it is allowed to do": the same short list in the gallery's panel
//! and in the installer, built from what the list or the package says.
//! Each line is what the sandbox enforces (`docs/EXTENSIONS.md`, "What an
//! extension can and cannot do"), nothing more.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_metadata::sources::SourceOption;
use heronite_metadata_ext::index::IndexEntry;
use heronite_metadata_ext::PackageInfo;

use crate::icons::Icon;
use crate::theme;
use crate::ui::fields::field_label;
use crate::ui::icon;

/// What a source asks before it is installed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Asks {
    /// As a manifest writes them.
    pub hosts: Vec<String>,
    pub image_hosts: Vec<String>,
    pub adult: bool,
    /// The settings it asks for: `(label, required)`.
    pub settings: Vec<(String, bool)>,
}

impl Asks {
    pub fn of_entry(e: &IndexEntry) -> Self {
        Self {
            hosts: e.hosts.clone(),
            image_hosts: e.images.clone(),
            adult: e.adult,
            settings: e.settings.iter().map(|s| (s.label.clone(), s.required)).collect(),
        }
    }

    pub fn of_package(p: &PackageInfo) -> Self {
        Self {
            hosts: p.hosts.clone(),
            image_hosts: p.image_hosts.clone(),
            adult: p.adult,
            settings: p.settings.iter().map(|o: &SourceOption| (o.label.clone(), o.required)).collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Network,
    Reads,
    Walled,
    Adult,
    Settings,
}

/// One line of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub mark: Mark,
    pub text: String,
}

/// "a, b and c".
fn joined(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => tr!("{list} and {last}", list = rest.join(", "), last = last.clone()),
    }
}

/// The lines, in order: where it connects, what it reads, what it cannot
/// reach, adult pictures, the settings it asks for.
pub fn lines(a: &Asks) -> Vec<Line> {
    let mut out = Vec::new();
    let network = if a.hosts.is_empty() {
        tr!("Connects to nothing on the internet").to_string()
    } else {
        tr!("Connects to {hosts}, and nowhere else", hosts = joined(&a.hosts))
    };
    out.push(Line { mark: Mark::Network, text: network });
    if !a.image_hosts.is_empty() {
        out.push(Line { mark: Mark::Network, text: tr!("Takes pictures from {hosts} too", hosts = joined(&a.image_hosts)) });
    }
    out.push(Line {
        mark: Mark::Reads,
        text: tr!("Reads from your games: title, platform, year, file or install folder name (never a path), store and ids").into(),
    });
    out.push(Line { mark: Mark::Walled, text: tr!("No access to your files, your accounts or other sources' keys").into() });
    if a.adult {
        out.push(Line { mark: Mark::Adult, text: tr!("Its pictures are hidden until \u{201c}Allow adult covers\u{201d} is on").into() });
    }
    if !a.settings.is_empty() {
        let names: Vec<String> = a
            .settings
            .iter()
            .map(|(label, required)| if *required { tr!("{label} (needed)", label = label.clone()) } else { label.clone() })
            .collect();
        out.push(Line { mark: Mark::Settings, text: tr!("Asks you for: {settings}", settings = names.join(", ")) });
    }
    out
}

fn mark_icon(m: Mark) -> Icon {
    match m {
        Mark::Network => Icon::Language,
        Mark::Reads => Icon::Description,
        Mark::Walled => Icon::Lock,
        Mark::Adult => Icon::VisibilityOff,
        Mark::Settings => Icon::Settings,
    }
}

/// "What it is allowed to do" and its lines.
pub fn block(a: &Asks) -> Div {
    let mut col = div().flex().flex_col().gap(px(8.)).child(field_label(tr!("What it is allowed to do")));
    for l in lines(a) {
        let c = if l.mark == Mark::Adult { theme::readable(theme::warning()) } else { theme::text_muted().into() };
        col = col.child(
            div()
                .flex()
                .items_start()
                .gap(px(10.))
                .child(div().flex_none().pt(px(1.)).child(icon(mark_icon(l.mark), px(15.), c)))
                .child(div().flex_1().min_w_0().text_size(px(12.5)).line_height(px(17.)).text_color(theme::text_primary()).child(l.text)),
        );
    }
    col
}

#[cfg(test)]
mod tests {
    use super::{lines, Asks, Mark};

    #[test]
    fn the_list_says_what_the_sandbox_allows_and_nothing_else() {
        let a = Asks { hosts: vec!["*.wikipedia.org".into()], ..Default::default() };
        let l = lines(&a);
        assert_eq!(l.iter().map(|l| l.mark).collect::<Vec<_>>(), [Mark::Network, Mark::Reads, Mark::Walled]);
        assert_eq!(l[0].text, "Connects to *.wikipedia.org, and nowhere else");
        assert!(l[1].text.contains("file or install folder name (never a path)"), "a PC game's fileName is its install folder's name");

        let a = Asks {
            hosts: vec!["www.doujin.example.org".into(), "api.doujin.example.org".into(), "http://old.example.org".into()],
            image_hosts: vec!["img.doujin.example.org".into()],
            adult: true,
            settings: vec![("API key".into(), true), ("Region".into(), false)],
        };
        let l = lines(&a);
        assert_eq!(
            l.iter().map(|l| l.mark).collect::<Vec<_>>(),
            [Mark::Network, Mark::Network, Mark::Reads, Mark::Walled, Mark::Adult, Mark::Settings]
        );
        assert_eq!(l[0].text, "Connects to www.doujin.example.org, api.doujin.example.org and http://old.example.org, and nowhere else");
        assert_eq!(l[1].text, "Takes pictures from img.doujin.example.org too");
        assert_eq!(l[5].text, "Asks you for: API key (needed), Region");

        assert_eq!(lines(&Asks::default())[0].text, "Connects to nothing on the internet");
    }
}
