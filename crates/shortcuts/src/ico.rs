//! `.ico` files for Windows shortcuts, which take no other picture: one PNG
//! image inside the ICO container (Windows reads PNG icons since Vista and
//! scales a 256 px one down for the smaller sizes).

/// An `.ico` holding the PNG `png` as its only image. `None` when `png` is
/// not a PNG or is larger than an icon can be (256 px).
pub fn from_png(png: &[u8]) -> Option<Vec<u8>> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    // The IHDR chunk comes first: width and height after its length and type.
    if png.len() < 24 || !png.starts_with(SIGNATURE) || &png[12..16] != b"IHDR" {
        return None;
    }
    let dimension = |at: usize| u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]);
    let (width, height) = (dimension(16), dimension(20));
    if !(1..=256).contains(&width) || !(1..=256).contains(&height) {
        return None;
    }
    let size = u32::try_from(png.len()).ok()?;
    // 0 stands for 256 in the one-byte sizes.
    let byte = |n: u32| (n % 256) as u8;
    let mut out = Vec::with_capacity(22 + png.len());
    // ICONDIR: reserved, type 1 (icon), one image.
    out.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    // ICONDIRENTRY: width, height, no palette, reserved, 1 plane, 32 bits per
    // pixel, the image's size and where it starts.
    out.extend_from_slice(&[byte(width), byte(height), 0, 0, 1, 0, 32, 0]);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&22u32.to_le_bytes());
    out.extend_from_slice(png);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The signature and an IHDR chunk of a `w`x`h` RGBA image.
    fn png_header(w: u32, h: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&w.to_be_bytes());
        png.extend_from_slice(&h.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        png
    }

    #[test]
    fn wraps_the_png_after_a_one_entry_directory() {
        let png = png_header(256, 256);
        let ico = from_png(&png).unwrap();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(&ico[6..14], &[0, 0, 0, 0, 1, 0, 32, 0]);
        assert_eq!(u32::from_le_bytes(ico[14..18].try_into().unwrap()) as usize, png.len());
        assert_eq!(u32::from_le_bytes(ico[18..22].try_into().unwrap()), 22);
        assert_eq!(&ico[22..], &png[..]);
        assert_eq!(&from_png(&png_header(48, 32)).unwrap()[6..8], &[48, 32]);
    }

    #[test]
    fn refuses_what_is_no_icon() {
        assert!(from_png(b"GIF89a....................").is_none());
        assert!(from_png(&png_header(512, 512)).is_none());
        assert!(from_png(&png_header(0, 16)).is_none());
        assert!(from_png(&png_header(16, 16)[..20]).is_none());
    }
}
