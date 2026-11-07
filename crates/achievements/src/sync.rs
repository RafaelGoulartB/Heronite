//! A refresh of one game: read every source, fold each answer into the
//! local copy, write it, and say what is new.

use chrono::{DateTime, Utc};

use crate::merge::{merge, merge_live};
use crate::model::{Achievement, GameAchievements, LocalPin, Source};
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

/// A running game's session, for a read made during it.
#[derive(Debug, Clone, Copy)]
pub struct Session<'a> {
    /// The sources its live check reads: their reads use `merge_live`.
    pub sources: &'a [Source],
    pub since: DateTime<Utc>,
}

impl Session<'_> {
    fn covers(&self, source: Source) -> bool {
        self.sources.contains(&source)
    }
}

/// Reads every target of a game and updates its copy. Blocking.
pub fn refresh(ctx: &Ctx, runner: &str, app_name: &str, targets: &[SourceTarget]) -> Outcome {
    read_sources(ctx, runner, app_name, targets, None, false)
}

/// [`refresh`] of a game that runs (the overlay's panel asks): its live
/// sources are read as its live check reads them, so what the session
/// unlocked is new even on their first read.
pub fn refresh_in_session(ctx: &Ctx, runner: &str, app_name: &str, targets: &[SourceTarget], session: Session) -> Outcome {
    read_sources(ctx, runner, app_name, targets, Some(session), false)
}

/// The live check of a game that runs since `since`: reads only the
/// `sources` of its targets (the cheap, local ones), and reports as new
/// only what was unlocked in the session (`merge_live`). Failures are not
/// logged here: the caller decides when a source failing is worth saying
/// (a file read while the game writes it fails once, then reads).
pub fn refresh_live(
    ctx: &Ctx,
    runner: &str,
    app_name: &str,
    targets: &[SourceTarget],
    sources: &[Source],
    since: DateTime<Utc>,
) -> Outcome {
    read_sources(ctx, runner, app_name, targets, Some(Session { sources, since }), true)
}

/// `only_live`: read the session's sources alone (and say nothing of their
/// failures: the live check does).
fn read_sources(ctx: &Ctx, runner: &str, app_name: &str, targets: &[SourceTarget], session: Option<Session>, only_live: bool) -> Outcome {
    let mut game = store::read(ctx.paths, runner, app_name);
    let mut out = Outcome::default();
    let mut changed = false;
    for target in targets {
        let live = session.filter(|s| s.covers(target.source()));
        if only_live && live.is_none() {
            continue;
        }
        let mut target = target.clone();
        if let SourceTarget::Retro(t) = &mut target {
            t.pinned_game_id = game.retro_game_id;
        }
        if let SourceTarget::Local(t) = &mut target {
            t.pin = game.local.clone();
        }
        let source = target.source();
        match sources::fetch(ctx, &target) {
            Fetch::Set(fresh) => {
                let old = game.sets.iter().position(|s| s.source == source);
                let old_set = old.map(|i| &game.sets[i]);
                let (merged, new) = match live {
                    Some(s) => merge_live(old_set, fresh, Utc::now(), s.since),
                    None => merge(old_set, fresh, Utc::now()),
                };
                out.new_unlocks.extend(new.into_iter().map(|a| (source, a)));
                match old {
                    Some(i) => game.sets[i] = merged,
                    None => game.sets.push(merged),
                }
                game.empty_at.remove(&source);
                changed = true;
            }
            Fetch::None => {
                game.empty_at.insert(source, Utc::now());
                changed = true;
            }
            Fetch::NeedsAccount => out.problems.push((source, Problem::NeedsAccount)),
            Fetch::Unmatched { hash, file } => out.problems.push((source, Problem::Unmatched { hash, file })),
            Fetch::Failed(e) => {
                if !only_live {
                    log::warn!("achievements of {runner}_{app_name} from {}: {e}", source.label());
                }
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

/// Sets (or clears) the game-files choice of a game: a folder, an app id,
/// or off. Its set and its "none" mark go with the old choice, so the next
/// refresh reads the files again; `forget` drops the set even when the
/// choice stays (a fresh start).
pub fn pin_local(ctx: &Ctx, runner: &str, app_name: &str, pin: Option<LocalPin>, forget: bool, order: &[Source]) -> anyhow::Result<()> {
    let mut game = store::read(ctx.paths, runner, app_name);
    let pin = pin.filter(|p| *p != LocalPin::default());
    if game.local == pin && !forget {
        return Ok(());
    }
    game.local = pin;
    game.sets.retain(|s| s.source != Source::Local);
    game.empty_at.remove(&Source::Local);
    store::write(ctx.paths, runner, app_name, &game, order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::Accounts;
    use crate::target::LocalTarget;
    use heronite_data::paths::DataPaths;
    use heronite_metadata::Http;

    /// A game with an emulator's files (an app id no real saves folder has)
    /// whose saves say `A` was unlocked at `at`.
    fn game_with_unlock(dir: &std::path::Path, at: DateTime<Utc>) -> Vec<SourceTarget> {
        let game = dir.join("Game");
        let settings = game.join("steam_settings");
        std::fs::create_dir_all(game.join("saves/3999991")).unwrap();
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(settings.join("steam_appid.txt"), "3999991").unwrap();
        std::fs::write(settings.join("configs.user.ini"), "[user::saves]\nlocal_save_path=saves\n").unwrap();
        std::fs::write(
            settings.join("achievements.json"),
            br#"[{"name":"A","displayName":"Alpha","unlock_percentage":50},{"name":"B","displayName":"Beta","unlock_percentage":5}]"#,
        )
        .unwrap();
        let state = format!(r#"{{"A":{{"earned":true,"earned_time":{}}}}}"#, at.timestamp());
        std::fs::write(game.join("saves/3999991/achievements.json"), state).unwrap();
        vec![SourceTarget::Local(LocalTarget { app_name: "g".into(), folders: vec![game], ..Default::default() })]
    }

    fn run(dir: &std::path::Path, targets: &[SourceTarget], session: Option<Session>) -> Outcome {
        let paths = DataPaths::at(dir.join("data"));
        let (http, accounts) = (Http::default(), Accounts::default());
        let ctx = Ctx { paths: &paths, http: &http, accounts: &accounts, lang: "en" };
        match session {
            Some(s) => refresh_in_session(&ctx, "sideload", "g", targets, s),
            None => refresh(&ctx, "sideload", "g", targets),
        }
    }

    #[test]
    fn a_first_read_during_the_session_announces_its_unlock() {
        let now = Utc::now();
        let since = now - chrono::Duration::minutes(10);
        // Outside a session the first read is history.
        let dir = tempfile::tempdir().unwrap();
        let targets = game_with_unlock(dir.path(), now);
        assert!(run(dir.path(), &targets, None).new_unlocks.is_empty());
        // During one (the overlay's panel asked), what it unlocked is new...
        let dir = tempfile::tempdir().unwrap();
        let targets = game_with_unlock(dir.path(), now);
        let session = Session { sources: &[Source::Local], since };
        let out = run(dir.path(), &targets, Some(session));
        assert_eq!(out.new_unlocks.iter().map(|(_, a)| a.id.as_str()).collect::<Vec<_>>(), ["A"]);
        // ...once: the live check and the refresh after exit find nothing.
        assert!(run(dir.path(), &targets, Some(session)).new_unlocks.is_empty());
        assert!(run(dir.path(), &targets, None).new_unlocks.is_empty());
    }

    #[test]
    fn an_unlock_from_before_the_session_stays_history() {
        let now = Utc::now();
        let dir = tempfile::tempdir().unwrap();
        let targets = game_with_unlock(dir.path(), now - chrono::Duration::days(30));
        let out = run(dir.path(), &targets, Some(Session { sources: &[Source::Local], since: now }));
        assert!(out.new_unlocks.is_empty());
        assert!(out.game.sets[0].achievements.iter().any(|a| a.id == "A" && a.unlocked && a.noticed_at.is_none()));
    }
}
