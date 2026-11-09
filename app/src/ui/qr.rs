//! A QR code drawn with divs: one row per line of modules, each run of dark
//! modules one div. Always black on white with the standard quiet zone,
//! whatever the theme: phone cameras read that best.

use gpui_kit::prelude::*;
use gpui_kit::*;
use qrcodegen::{QrCode, QrCodeEcc};

/// The light margin around the code, in modules (the standard's minimum).
const QUIET_ZONE: usize = 4;

/// The runs of one line of modules: `(dark, length)`.
fn runs(qr: &QrCode, y: i32) -> Vec<(bool, usize)> {
    let mut out: Vec<(bool, usize)> = Vec::new();
    for x in 0..qr.size() {
        let dark = qr.get_module(x, y);
        match out.last_mut() {
            Some((d, n)) if *d == dark => *n += 1,
            _ => out.push((dark, 1)),
        }
    }
    out
}

/// The code of `text`, `module` pixels per module (a whole number keeps the
/// modules sharp). `None` when the text is too long for a QR code.
pub fn qr_code(text: &str, module: f32) -> Option<Div> {
    let qr = QrCode::encode_text(text, QrCodeEcc::Medium).ok()?;
    let side = (qr.size() as usize + 2 * QUIET_ZONE) as f32 * module;
    let black = rgb(0x000000);
    let lines = (0..qr.size()).map(|y| {
        div()
            .flex()
            .h(px(module))
            .children(runs(&qr, y).into_iter().map(|(dark, n)| div().h_full().w(px(n as f32 * module)).when(dark, |d| d.bg(black))))
    });
    Some(
        div()
            .flex_none()
            .size(px(side))
            .p(px(QUIET_ZONE as f32 * module))
            .rounded(px(10.))
            .bg(rgb(0xffffff))
            .child(div().flex().flex_col().children(lines)),
    )
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's own `test` attribute would shadow the standard one.
    use super::{runs, QrCode, QrCodeEcc};

    #[test]
    fn runs_cover_each_line() {
        let qr = QrCode::encode_text("https://s.team/q/1/8406597215213412810", QrCodeEcc::Medium).unwrap();
        for y in 0..qr.size() {
            let line = runs(&qr, y);
            assert_eq!(line.iter().map(|(_, n)| n).sum::<usize>(), qr.size() as usize);
            assert!(line.windows(2).all(|w| w[0].0 != w[1].0));
        }
        // The finder pattern's top edge: seven dark modules first.
        assert_eq!(runs(&qr, 0)[0], (true, 7));
    }
}
