//! The few EDID fields that identify a monitor: vendor, product, serial and
//! name. Windows hands over the vendor and product directly; the serial is
//! only in the raw EDID the driver stores in the registry.

use crate::model::real_serial;

/// The three-letter PNP vendor id (`DEL`, `GSM`, `SAM`) from the EDID's
/// manufacturer field, as the big-endian 16-bit value of bytes 8-9.
pub fn pnp_vendor(be: u16) -> String {
    let letter = |shift: u16| {
        let v = (be >> shift) & 0x1f;
        if (1..=26).contains(&v) {
            (b'A' + v as u8 - 1) as char
        } else {
            '?'
        }
    };
    [letter(10), letter(5), letter(0)].into_iter().collect()
}

/// [`pnp_vendor`] of the manufacturer id as `DISPLAYCONFIG_TARGET_DEVICE_NAME`
/// holds it: the EDID bytes read as a little-endian number.
pub fn pnp_vendor_from_windows(raw: u16) -> String {
    pnp_vendor(raw.swap_bytes())
}

/// What a raw EDID block says about the monitor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EdidInfo {
    pub vendor: String,
    /// The product code, four hex digits as Windows writes it (`A1D0`).
    pub product: String,
    /// The serial-number descriptor, else the 32-bit serial number field.
    pub serial: Option<String>,
    /// The monitor-name descriptor (`DELL U2723QE`).
    pub name: Option<String>,
}

const HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];

/// Reads the base block of `edid`; `None` when it is not an EDID.
pub fn parse(edid: &[u8]) -> Option<EdidInfo> {
    if edid.len() < 128 || edid[..8] != HEADER {
        return None;
    }
    let vendor = pnp_vendor(u16::from_be_bytes([edid[8], edid[9]]));
    let product = format!("{:04X}", u16::from_le_bytes([edid[10], edid[11]]));
    let number = u32::from_le_bytes([edid[12], edid[13], edid[14], edid[15]]);
    let mut serial = None;
    let mut name = None;
    // Four 18-byte descriptors; a display descriptor starts with three zeros
    // and its tag, and its text ends at a line feed.
    for d in edid[54..126].as_chunks::<18>().0 {
        if d[..3] != [0, 0, 0] {
            continue;
        }
        let text = || {
            let raw = &d[5..18];
            let end = raw.iter().position(|&b| b == 0x0a).unwrap_or(raw.len());
            String::from_utf8_lossy(&raw[..end]).trim().to_string()
        };
        match d[3] {
            0xff => serial = real_serial(&text()),
            0xfc => name = Some(text()).filter(|n| !n.is_empty()),
            _ => {}
        }
    }
    let serial = serial.or_else(|| real_serial(&format!("{number:08X}")));
    Some(EdidInfo { vendor, product, serial, name })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A base EDID block with the given fields (checksum not filled in: the
    /// parser does not need it).
    pub(crate) fn edid(vendor: &str, product: u16, number: u32, serial: Option<&str>, name: Option<&str>) -> Vec<u8> {
        let mut e = vec![0u8; 128];
        e[..8].copy_from_slice(&HEADER);
        let v: Vec<u16> = vendor.bytes().map(|b| (b - b'A' + 1) as u16).collect();
        e[8..10].copy_from_slice(&((v[0] << 10) | (v[1] << 5) | v[2]).to_be_bytes());
        e[10..12].copy_from_slice(&product.to_le_bytes());
        e[12..16].copy_from_slice(&number.to_le_bytes());
        let mut slot = 54;
        for (tag, text) in [(0xffu8, serial), (0xfc, name)] {
            let Some(text) = text else { continue };
            e[slot + 3] = tag;
            let mut field = [0x20u8; 13];
            field[..text.len()].copy_from_slice(text.as_bytes());
            if text.len() < 13 {
                field[text.len()] = 0x0a;
            }
            e[slot + 5..slot + 18].copy_from_slice(&field);
            slot += 18;
        }
        // A detailed timing descriptor in a free slot (not a display descriptor).
        e[slot] = 0x01;
        e[slot + 1] = 0x1d;
        e
    }

    #[test]
    fn decodes_pnp_vendors() {
        assert_eq!(pnp_vendor(0x10ac), "DEL");
        assert_eq!(pnp_vendor(0x1e6d), "GSM");
        // Windows' DISPLAYCONFIG field holds the bytes the other way round.
        assert_eq!(pnp_vendor_from_windows(0xac10), "DEL");
        assert_eq!(pnp_vendor(0), "???");
    }

    #[test]
    fn reads_vendor_product_serial_and_name() {
        let info = parse(&edid("DEL", 0xa1d0, 0x4c4c3130, Some("5CD2140XYZ"), Some("DELL U2723QE"))).unwrap();
        assert_eq!(info.vendor, "DEL");
        assert_eq!(info.product, "A1D0");
        assert_eq!(info.serial.as_deref(), Some("5CD2140XYZ"));
        assert_eq!(info.name.as_deref(), Some("DELL U2723QE"));
    }

    #[test]
    fn falls_back_to_the_serial_number_field_and_skips_placeholders() {
        let numbered = parse(&edid("GBT", 0x273e, 0x0001_e240, None, Some("M27U"))).unwrap();
        assert_eq!(numbered.serial.as_deref(), Some("0001E240"));
        // Neither a descriptor nor a number: no serial (a TV, a cheap panel).
        let none = parse(&edid("GSM", 0x5b9b, 0, None, None)).unwrap();
        assert_eq!(none.serial, None);
        assert_eq!(none.name, None);
        // The number LG and Samsung panels share, behind a `0` descriptor.
        let placeholder = parse(&edid("GSM", 0x5b9b, 0x0101_0101, Some("0"), None)).unwrap();
        assert_eq!(placeholder.serial, None);
    }

    #[test]
    fn rejects_what_is_not_an_edid() {
        assert_eq!(parse(&[0u8; 128]), None);
        assert_eq!(parse(&HEADER), None);
    }
}
