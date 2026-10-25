//! Steam: the stats files the Steam client keeps on this PC.

use super::{Ctx, Fetch};

pub fn fetch(_ctx: &Ctx, _app_id: &str) -> Fetch {
    Fetch::None
}
