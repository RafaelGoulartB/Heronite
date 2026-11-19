//! SteamGridDB's style filter in the image searches (the Covers dialog and
//! Game settings → Search images): a pill per style its pictures have, with
//! how many, after "All". Only pictures with a style (SteamGridDB's) are
//! filtered; a style no picture has any more is no filter.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
use heronite_metadata::WebImage;

use crate::theme;

/// SteamGridDB's styles in the order its site lists them (grids, heroes,
/// then logos); a style it adds later comes after them.
const ORDER: [&str; 9] = ["alternate", "blurred", "white_logo", "material", "no_logo", "official", "white", "black", "custom"];

/// The name of a SteamGridDB style; one not known is shown as SteamGridDB names it.
pub fn label(style: &str) -> SharedString {
    match style {
        "alternate" => tr!("Alternate").into(),
        "blurred" => tr!("Blurred").into(),
        "white_logo" => tr!("White logo").into(),
        "no_logo" => tr!("No logo").into(),
        "material" => tr!("Material").into(),
        "official" => tr!("Official").into(),
        "white" => tr!("White").into(),
        "black" => tr!("Black").into(),
        "custom" => tr!("Custom").into(),
        other => other.replace('_', " ").into(),
    }
}

/// The styles `images` have and how many of each, in [`ORDER`].
pub fn counts(images: &[WebImage]) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for img in images.iter().filter(|i| !i.style.is_empty()) {
        match out.iter_mut().find(|(s, _)| *s == img.style) {
            Some((_, n)) => *n += 1,
            None => out.push((img.style.clone(), 1)),
        }
    }
    let rank = |s: &str| ORDER.iter().position(|o| *o == s).unwrap_or(ORDER.len());
    out.sort_by_key(|(s, _)| rank(s));
    out
}

/// `picked` while some of `images` have it (`None`: every style).
pub fn shown<'a>(images: &[WebImage], picked: Option<&'a str>) -> Option<&'a str> {
    picked.filter(|p| images.iter().any(|i| i.style == *p))
}

/// Whether `image` is listed with `shown` ([`shown`]) picked.
pub fn keeps(image: &WebImage, shown: Option<&str>) -> bool {
    shown.is_none_or(|s| image.style == s)
}

/// A pill of a filter row: lit when `on`.
pub fn pill(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .py(px(4.))
        .rounded_full()
        .border_1()
        .text_size(px(12.))
        .whitespace_nowrap()
        .cursor_pointer()
        .when(on, |d| d.bg(theme::accent_tint()).border_color(theme::accent_tint_border()).text_color(theme::text_primary()))
        .when(!on, |d| {
            d.border_color(theme::ink_a(0.08))
                .text_color(theme::text_muted())
                .hover(|s| s.bg(theme::ink_a(0.06)).text_color(theme::text_bright()))
        })
}

/// A count beside a pill's name, dimmed.
pub fn count(n: usize) -> Div {
    div().text_color(theme::text_dim()).child(n.to_string())
}

/// "Style", "All" and a pill per style of `images`; `pick` gets the style
/// clicked (`None`: All). `None` while they have fewer than two styles.
pub fn row<V: 'static>(
    id: &str,
    images: &[WebImage],
    picked: Option<&str>,
    cx: &mut Context<V>,
    pick: impl Fn(&mut V, Option<String>, &mut Context<V>) + Clone + 'static,
) -> Option<Div> {
    let styles = counts(images);
    if styles.len() < 2 {
        return None;
    }
    let shown = shown(images, picked);
    let total = images.len();
    let all = {
        let pick = pick.clone();
        pill(SharedString::from(format!("{id}-all")), shown.is_none())
            .child(tr!("All"))
            .child(count(total))
            .on_click(cx.listener(move |this, _, _, cx| pick(this, None, cx)))
    };
    let mut out = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .child(div().text_size(px(12.)).text_color(theme::text_dim()).mr(px(2.)).child(tr!("Style")))
        .child(all);
    for (style, n) in styles {
        let on = shown == Some(style.as_str());
        let pick = pick.clone();
        let name = label(&style);
        out = out.child(
            pill(SharedString::from(format!("{id}-{style}")), on)
                .child(name)
                .child(count(n))
                .on_click(cx.listener(move |this, _, _, cx| pick(this, Some(style.clone()), cx))),
        );
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    // Explicit imports: `use super::*` would bring in gpui's `test` macro and shadow `#[test]`.
    use super::{counts, keeps, label, shown};
    use gpui_kit::SharedString;
    use heronite_metadata::WebImage;

    fn img(style: &str) -> WebImage {
        WebImage { image_url: format!("u-{style}"), thumb_url: String::new(), width: 0, height: 0, style: style.into() }
    }

    #[test]
    fn styles_are_counted_in_steamgriddb_s_order() {
        let images = [img("material"), img("alternate"), img("future"), img("alternate"), img(""), img("white_logo")];
        let counts = counts(&images);
        let got: Vec<(&str, usize)> = counts.iter().map(|(s, n)| (s.as_str(), *n)).collect();
        assert_eq!(got, [("alternate", 2), ("white_logo", 1), ("material", 1), ("future", 1)]);
        let both = [img("no_logo"), img("material")];
        let order: Vec<String> = super::counts(&both).into_iter().map(|(s, _)| s).collect();
        assert_eq!(order, ["material", "no_logo"]);
    }

    #[test]
    fn a_style_no_picture_has_is_no_filter() {
        let images = [img("alternate"), img("material")];
        assert_eq!(shown(&images, Some("material")), Some("material"));
        assert_eq!(shown(&images, Some("blurred")), None);
        assert!(keeps(&images[0], None));
        assert!(!keeps(&images[0], Some("material")));
        assert_eq!(label("some_new"), SharedString::from("some new"));
    }
}
