//! The battery of game controllers that XInput cannot tell, read the way
//! each maker's own software reads it. XInput only knows Xbox controllers,
//! with four coarse levels, and sees most other controllers in XInput mode as
//! wired Xbox 360 pads (no battery, or always "full"); Windows.Gaming.Input
//! makes up 1000 of 1000 mWh for those.
//!
//! Each driver knows some controllers by their USB vendor and product ids and
//! reads one through its HID collections ([`crate::hid`]): a battery field in
//! the input reports it sends anyway, or a read-only request on the maker's
//! own channel. No driver changes a controller's mode or settings.
//!
//! The drivers are ported from BatteryHub (`src/drivers/`,
//! <https://github.com/krisgfx9/Wireless-Device-Battery-Monitor>, GPL-3.0),
//! with the protocol references each one names.

mod beitong;
mod eightbitdo;
mod flydigi;
mod gamesir;
mod generic;
mod playstation;
mod sapjoy;
mod steam_controller;
mod switch_pro;
mod xbox;

pub use self::xbox::xbox;

use std::io;

use crate::hid::{self, Collection};

/// A battery reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    /// 0..=100.
    pub percent: u8,
    pub charging: bool,
}

/// A controller a driver knows, connected now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pad {
    pub vendor: u16,
    pub product: u16,
    /// Its model, as people know it ("DualSense", "GameSir G7 Pro 8K").
    pub model: &'static str,
    /// Its game input runs through XInput (an XInput slot names it too, see
    /// `gamepad::product`).
    pub xinput: bool,
    /// A receiver or adapter, which is there with no controller linked: it
    /// stands for one only while its driver reads a battery ([`RECEIVERS`]).
    pub receiver: bool,
}

/// Receivers and adapters that keep their ids with no controller linked
/// (the others change ids then, see `gamesir`): the DualShock 4 USB wireless
/// adapter, the Steam Controller's 2015 receiver and its 2026 puck, and
/// 8BitDo's 2.4 GHz receivers.
const RECEIVERS: &[(u16, u16)] = &[(0x054C, 0x0BA0), (0x28DE, 0x1142), (0x28DE, 0x1304), (0x2DC8, 0x6013), (0x2DC8, 0x3109)];

/// What a driver reads.
pub(crate) struct Driver {
    /// The model of the controller `vendor:product`, when the driver knows it.
    pub model: fn(vendor: u16, product: u16) -> Option<&'static str>,
    /// Reads the battery of the controller `vendor:product` (one the driver
    /// knows) through `collections`, every HID collection present: `None`
    /// when it tells none now (asleep, a receiver without its controller,
    /// charging without a percentage). Blocking, up to a couple of seconds.
    pub read: fn(vendor: u16, product: u16, collections: &[Collection]) -> io::Result<Option<Reading>>,
}

/// Every driver, the more specific first.
fn drivers() -> impl Iterator<Item = &'static Driver> {
    [
        gamesir::DRIVERS,
        sapjoy::DRIVERS,
        playstation::DRIVERS,
        switch_pro::DRIVERS,
        steam_controller::DRIVERS,
        eightbitdo::DRIVERS,
        flydigi::DRIVERS,
        beitong::DRIVERS,
    ]
    .into_iter()
    .flatten()
}

/// The model of the controller `vendor:product`, when a driver knows it.
pub fn model(vendor: u16, product: u16) -> Option<&'static str> {
    drivers().find_map(|d| (d.model)(vendor, product))
}

/// Whether a driver knows the controller `vendor:product`.
pub fn knows(vendor: u16, product: u16) -> bool {
    model(vendor, product).is_some()
}

/// Reads the battery of the controller `vendor:product`: by its driver, or,
/// for a controller no driver knows, by the battery usages of the HID
/// standard when it has them. `Ok(None)` when it tells none. With two
/// identical controllers, the first one Windows lists answers. Blocking.
pub fn read(vendor: u16, product: u16) -> io::Result<Option<Reading>> {
    let collections = hid::collections()?;
    match drivers().find(|d| (d.model)(vendor, product).is_some()) {
        Some(driver) => (driver.read)(vendor, product, &collections),
        None => generic::read(vendor, product, &collections),
    }
}

/// The controllers a driver knows that are connected now, each once.
/// Blocking (the HID collections are listed).
pub fn pads() -> io::Result<Vec<Pad>> {
    Ok(pads_in(&hid::collections()?))
}

fn pads_in(collections: &[Collection]) -> Vec<Pad> {
    let mut pads: Vec<Pad> = Vec::new();
    for c in collections {
        let Some(model) = model(c.vendor, c.product) else { continue };
        match pads.iter_mut().find(|p| p.vendor == c.vendor && p.product == c.product) {
            Some(pad) => pad.xinput |= c.is_xinput(),
            None => pads.push(Pad {
                vendor: c.vendor,
                product: c.product,
                model,
                xinput: c.is_xinput(),
                receiver: RECEIVERS.contains(&(c.vendor, c.product)),
            }),
        }
    }
    pads
}

/// `value` of `lo..=hi`, as a percentage.
pub(crate) fn map_to_100(value: i32, lo: i32, hi: i32) -> u8 {
    if hi <= lo {
        return 0;
    }
    ((value - lo) * 100 / (hi - lo)).clamp(0, 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_controllers_are_listed_once() {
        let collections = [
            Collection::new(0x3537, 0x10C6, 0x0001, 0x0005, (15, 0, 0)),
            Collection::new(0x3537, 0x10C6, 0xFFF0, 0x0040, (64, 64, 0)),
            Collection::new(0x1234, 0x5678, 0x0001, 0x0005, (15, 0, 0)),
        ];
        let pads = pads_in(&collections);
        assert_eq!(pads.len(), 1, "{pads:?}");
        assert_eq!((pads[0].vendor, pads[0].product), (0x3537, 0x10C6));
        assert!(!pads[0].receiver);
    }

    #[test]
    fn receivers_are_told_apart() {
        let pads = pads_in(&[Collection::new(0x054C, 0x0BA0, 0x0001, 0x0005, (64, 0, 0))]);
        assert!(pads[0].receiver, "{pads:?}");
        assert!(RECEIVERS.iter().all(|&(v, p)| knows(v, p)), "every receiver has a driver");
    }

    #[test]
    fn ranges_map_to_percentages() {
        assert_eq!(map_to_100(5, 0, 10), 50);
        assert_eq!(map_to_100(-3, 0, 10), 0);
        assert_eq!(map_to_100(12, 0, 10), 100);
        assert_eq!(map_to_100(1, 3, 3), 0);
    }
}
