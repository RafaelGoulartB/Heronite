//! A refresh of one game: read every source, fold each answer into the
//! local copy, write it, and say what is new.

use chrono::Utc;

use crate::merge::merge;
use crate::model::{Achievement, GameAchievements, Source};
use crate::sources::{self, Ctx, Fetch};
use crate::store;
use crate::target::{order_of, SourceTarget};

/// Why a source gave nothing this time.
#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    NeedsAccount,
    Unmatched { hash: Option<String>, file: String },
    Failed(String),
}

#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub game: GameAchievements,
    /// Unlocked since the last refresh, by source.
    pub new_unlocks: Vec<(Source, Achievement)>,
    pub problems: Vec<(Source, Problem)>,
}

/// Reads every target of a game and updates its copy. Blocking.
pub fn refresh(ctx: &Ctx, runner: &str, app_name: &str, targets: &[SourceTarget]) -> Outcome {
    let mut game = store::read(ctx.paths, runner, app_name);
    let mut out = Outcome::default();
    let mut changed = false;
    for target in targets {
        let mut target = target.clone();
        if let SourceTarget::Retro(t) = &mut target {
            t.pinned_game_id = game.retro_game_id;
        }
        let source = target.source();
        match sources::fetch(ctx, &target) {
            Fetch::Set(fresh) => {
                let old = game.sets.iter().position(|s| s.source == source);
                let (merged, new) = merge(old.map(|i| &game.sets[i]), fresh, Utc::now());
                out.new_unlocks.extend(new.into_iter().map(|a| (source, a)));
                match old {
                    Some(i) => game.sets[i] = merged,
                    None => game.sets.push(merged),
                }
                changed = true;
            }
            Fetch::None => {}
            Fetch::NeedsAccount => out.problems.push((source, Problem::NeedsAccount)),
            Fetch::Unmatched { hash, file } => out.problems.push((source, Problem::Unmatched { hash, file })),
            Fetch::Failed(e) => {
                log::warn!("achievements of {runner}_{app_name} from {}: {e}", source.label());
                out.problems.push((source, Problem::Failed(e)));
            }
        }
    }
    if changed {
        if let Err(e) = store::write(ctx.paths, runner, app_name, &game, &order_of(targets)) {
            log::warn!("could not save the achievements of {runner}_{app_name}: {e:#}");
        }
    }
    out.game = game;
    out
}

/// Pins (or unpins) the RetroAchievements game of a ROM.
pub fn pin_retro_game(ctx: &Ctx, runner: &str, app_name: &str, game_id: Option<u32>, order: &[Source]) -> anyhow::Result<()> {
    let mut game = store::read(ctx.paths, runner, app_name);
    if game.retro_game_id == game_id {
        return Ok(());
    }
    game.retro_game_id = game_id;
    // The set of the other game no longer applies.
    game.sets.retain(|s| s.source != Source::RetroAchievements);
    store::write(ctx.paths, runner, app_name, &game, order)
}
