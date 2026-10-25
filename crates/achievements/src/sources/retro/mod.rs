//! RetroAchievements: the ROM's hash names the game, the Web API gives its
//! achievements and the user's progress.

use super::{Ctx, Fetch};
use crate::target::RetroTarget;

pub fn fetch(ctx: &Ctx, _target: &RetroTarget) -> Fetch {
    if ctx.accounts.retro_achievements.is_none() {
        return Fetch::NeedsAccount;
    }
    Fetch::None
}
