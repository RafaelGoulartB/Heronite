//! RetroAchievements: the ROM's hash names the game (the console lists of
//! the client API), then its achievements and the user's progress come
//! through the client API with the login's token, or through the Web API
//! when the user gave its key (which adds the times of past unlocks).

pub mod api;
pub mod connect;
pub mod consoles;
pub mod hash;
#[cfg(test)]
mod test_support;

use std::collections::HashMap;
use std::path::Path;

use super::{Ctx, Fetch};
use crate::accounts::RetroAccount;
use crate::target::RetroTarget;
use api::Listed;

pub use api::{search_games, validate_account, GameHit, Profile};

pub fn fetch(ctx: &Ctx, target: &RetroTarget) -> Fetch {
    fetch_from(ctx, api::BASE, target)
}

fn fetch_from(ctx: &Ctx, base: &str, target: &RetroTarget) -> Fetch {
    let Some(account) = ctx.accounts.retro_achievements.as_ref().filter(|a| a.usable()) else { return Fetch::NeedsAccount };
    let game_id = match target.pinned_game_id {
        Some(id) => id,
        None => match resolve(ctx, base, target) {
            Ok(Resolved::Game(id)) => id,
            Ok(Resolved::Unmatched { hash, file }) => return Fetch::Unmatched { hash, file },
            Err(e) => return Fetch::Failed(format!("{e:#}")),
        },
    };
    progress(ctx, base, account, game_id)
}

fn progress(ctx: &Ctx, base: &str, account: &RetroAccount, game_id: u32) -> Fetch {
    let read = if account.api_key.is_empty() {
        connect::game_progress(ctx.http, base, account, game_id)
    } else {
        api::game_progress(ctx, base, account, game_id)
    };
    match read {
        Ok(set) if set.achievements.is_empty() => Fetch::None,
        Ok(set) => Fetch::Set(set),
        Err(e) => Fetch::Failed(format!("{e:#}")),
    }
}

enum Resolved {
    Game(u32),
    Unmatched { hash: Option<String>, file: String },
}

/// The game lists read so far in one fetch, by console.
struct Lists<'a> {
    ctx: &'a Ctx<'a>,
    base: &'a str,
    loaded: HashMap<u32, Vec<Listed>>,
}

impl Lists<'_> {
    fn of(&mut self, console: u32) -> anyhow::Result<&[Listed]> {
        if !self.loaded.contains_key(&console) {
            let list = api::game_list_at(self.ctx, self.base, console, chrono::Utc::now())?;
            self.loaded.insert(console, list);
        }
        Ok(&self.loaded[&console])
    }

    fn by_hash(&mut self, console: u32, hash: &str) -> anyhow::Result<Option<u32>> {
        Ok(self.of(console)?.iter().find(|g| g.hashes.iter().any(|h| h == hash)).map(|g| g.id))
    }
}

/// The (console, hash) pairs to look up for a ROM: for the consoles the
/// Collection names, else for the ones its extension allows.
fn hashes_of(rom: &Path, consoles: &[u32]) -> Vec<hash::Candidate> {
    if consoles.is_empty() {
        return hash::candidates(rom);
    }
    consoles.iter().filter_map(|&console| Some(hash::Candidate { console, hash: hash::hash_file(console, rom)? })).collect()
}

fn resolve(ctx: &Ctx, base: &str, target: &RetroTarget) -> anyhow::Result<Resolved> {
    let consoles = consoles::for_target(target);
    let mut lists = Lists { ctx, base, loaded: HashMap::new() };
    let mut first_hash: Option<String> = None;
    for rom in &target.roms {
        for c in hashes_of(rom, &consoles) {
            if let Some(id) = lists.by_hash(c.console, &c.hash)? {
                return Ok(Resolved::Game(id));
            }
            first_hash.get_or_insert(c.hash);
        }
    }
    // No hash or none known (CHD, a patched ROM): the title, only when it is unambiguous.
    for &console in &consoles {
        if let Some(id) = exact_title(lists.of(console)?, &target.title) {
            return Ok(Resolved::Game(id));
        }
    }
    let file = target.roms.first().and_then(|r| r.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Resolved::Unmatched { hash: first_hash, file })
}

/// Words of a title that carry its identity: no region or version tags, no
/// "the" (RA writes "Legend of Zelda, The").
fn title_key(title: &str) -> String {
    let mut depth = 0u32;
    let untagged: String = title
        .chars()
        .filter(|c| match c {
            '(' | '[' => {
                depth += 1;
                false
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                false
            }
            _ => depth == 0,
        })
        .collect();
    untagged
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !w.eq_ignore_ascii_case("the"))
        .collect::<Vec<_>>()
        .join("")
        .to_lowercase()
}

/// The one game whose title is the same as `title`, if exactly one is.
fn exact_title(list: &[Listed], title: &str) -> Option<u32> {
    let key = title_key(title);
    if key.is_empty() {
        return None;
    }
    let mut matches = list.iter().filter(|g| title_key(&g.title) == key);
    let first = matches.next()?;
    matches.next().is_none().then_some(first.id)
}

#[cfg(test)]
mod tests {
    use super::test_support::{serve, Env};
    use super::*;
    use crate::accounts::{Accounts, RetroAccount};
    use serde_json::{json, Value};

    /// MD5 of "abc", the body of the synthetic NES ROMs below.
    const ABC: &str = "900150983cd24fb0d6963f7d28e17f72";

    fn nes_rom(dir: &Path, name: &str) -> std::path::PathBuf {
        let mut rom = b"NES\x1a\x01\x01\0\0\0\0\0\0\0\0\0\0".to_vec();
        rom.extend_from_slice(b"abc");
        let file = dir.join(name);
        std::fs::write(&file, rom).unwrap();
        file
    }

    fn system_games(games: Value) -> String {
        json!({"Success": true, "Response": games}).to_string()
    }

    fn routes() -> Vec<(&'static str, u16, String)> {
        let list = system_games(json!([{"Title":"Other Game","ID":5,"NumAchievements":2,"SupportedHashes":["FFFF"]},
            {"Title":"Super Test, The","ID":6,"NumAchievements":1,"SupportedHashes":[ABC]}]));
        let game = json!({"ID":6,"Title":"Super Test, The","NumDistinctPlayers":10,"Achievements":{
            "1":{"ID":1,"NumAwarded":5,"Title":"First","Description":"d","Points":5,"BadgeName":"7","DisplayOrder":1,"type":"progression",
                 "DateEarned":"2024-05-06 07:08:09"}}});
        let patch = json!({"Success":true,"PatchData":{"ID":6,"Title":"Super Test, The","Achievements":[
            {"ID":1,"Title":"First","Description":"d","Flags":3,"Points":5,"BadgeName":"7","Type":"progression","Rarity":50.0},
            {"ID":2,"Title":"Second","Description":"d","Flags":3,"Points":10,"BadgeName":"8"}]}});
        vec![
            ("r=systemgames", 200, list),
            ("API_GetGameInfoAndUserProgress.php", 200, game.to_string()),
            ("r=patch", 200, patch.to_string()),
            ("h=0", 200, json!({"Success":true,"UserUnlocks":[1, 2],"GameID":6}).to_string()),
            ("h=1", 200, json!({"Success":true,"UserUnlocks":[2],"GameID":6}).to_string()),
        ]
    }

    fn with_token() -> Accounts {
        Accounts {
            retro_achievements: Some(RetroAccount { username: "me".into(), token: "TOKEN".into(), ..Default::default() }),
            ..Default::default()
        }
    }

    #[test]
    fn the_login_token_reads_the_progress_through_the_client_api() {
        let dir = tempfile::tempdir().unwrap();
        let rom = nes_rom(dir.path(), "Super Test (USA).nes");
        let server = serve(routes());
        let accounts = with_token();
        let env = Env::new(dir.path());
        let ctx = env.ctx(&accounts);
        let target = RetroTarget { roms: vec![rom], platform_id: Some("nintendo_nes".into()), ..Default::default() };
        let Fetch::Set(set) = fetch_from(&ctx, &server.base, &target) else { panic!("expected a set") };
        assert_eq!((set.external_id.as_str(), set.total(), set.unlocked()), ("6", 2, 2));
        let (first, second) = (&set.achievements[0], &set.achievements[1]);
        assert!(!first.hardcore && second.hardcore);
        assert_eq!((first.unlocked_at, first.rarity), (None, Some(50.0)));
        let requests = server.requests();
        assert!(requests.iter().all(|r| !r.contains("API_")), "{requests:?}");
        let patch = requests.iter().find(|r| r.contains("r=patch")).unwrap();
        assert!(patch.starts_with("POST ") && patch.contains("u=me") && patch.contains("t=TOKEN") && patch.contains("g=6"));
    }

    #[test]
    fn a_rejected_token_is_a_failure_without_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let rom = nes_rom(dir.path(), "x.nes");
        let mut routes = routes();
        routes[2] = ("r=patch", 401, json!({"Success":false,"Error":"Invalid token","Code":"invalid_credentials"}).to_string());
        let server = serve(routes);
        let accounts = with_token();
        let env = Env::new(dir.path());
        let ctx = env.ctx(&accounts);
        let target = RetroTarget { roms: vec![rom], platform_id: Some("nintendo_nes".into()), ..Default::default() };
        let Fetch::Failed(e) = fetch_from(&ctx, &server.base, &target) else { panic!("expected a failure") };
        assert!(!e.contains("TOKEN"), "{e}");
    }

    #[test]
    fn needs_an_account() {
        let dir = tempfile::tempdir().unwrap();
        let (env, accounts) = (Env::new(dir.path()), Accounts::default());
        let ctx = env.ctx(&accounts);
        assert_eq!(fetch(&ctx, &RetroTarget::default()), Fetch::NeedsAccount);
    }

    #[test]
    fn a_rom_is_matched_by_its_hash_and_the_cache_spares_the_second_list_request() {
        let dir = tempfile::tempdir().unwrap();
        let rom = nes_rom(dir.path(), "Super Test (USA).nes");
        let server = serve(routes());
        let accounts = Accounts {
            retro_achievements: Some(RetroAccount { username: "me".into(), api_key: "SECRET".into(), ..Default::default() }),
            ..Default::default()
        };
        let env = Env::new(dir.path());
        let ctx = env.ctx(&accounts);
        let target =
            RetroTarget { roms: vec![rom], platform_id: Some("nintendo_nes".into()), title: "Super Test".into(), ..Default::default() };
        for _ in 0..2 {
            let Fetch::Set(set) = fetch_from(&ctx, &server.base, &target) else { panic!("expected a set") };
            assert_eq!((set.external_id.as_str(), set.title.as_deref(), set.total(), set.unlocked()), ("6", Some("Super Test, The"), 1, 1));
        }
        let requests = server.requests();
        assert_eq!(requests.iter().filter(|r| r.contains("r=systemgames")).count(), 1, "{requests:?}");
        assert!(requests[0].contains("s=7") && !requests[0].contains("SECRET"));
        let progress = requests.iter().find(|r| r.contains("API_GetGameInfoAndUserProgress.php")).unwrap();
        assert!(progress.contains("g=6") && progress.contains("u=me") && progress.contains("a=1"));
        assert!(requests.iter().all(|r| r.to_ascii_lowercase().contains("user-agent: heronite/")), "{requests:?}");
    }

    #[test]
    fn the_extension_names_the_console_when_the_collection_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let rom = nes_rom(dir.path(), "game.nes");
        let server = serve(routes());
        let accounts = Accounts {
            retro_achievements: Some(RetroAccount { username: "me".into(), api_key: "k".into(), ..Default::default() }),
            ..Default::default()
        };
        let env = Env::new(dir.path());
        let ctx = env.ctx(&accounts);
        let target = RetroTarget { roms: vec![rom], ..Default::default() };
        assert!(matches!(fetch_from(&ctx, &server.base, &target), Fetch::Set(_)));
    }

    #[test]
    fn an_unknown_rom_is_unmatched_with_its_hash_and_a_pinned_game_wins() {
        let dir = tempfile::tempdir().unwrap();
        let rom = nes_rom(dir.path(), "Mystery.nes");
        let mut routes = routes();
        routes[0].2 = system_games(json!([{"Title":"Other Game","ID":5,"NumAchievements":2,"SupportedHashes":["ffff"]}]));
        let server = serve(routes);
        let accounts = Accounts {
            retro_achievements: Some(RetroAccount { username: "me".into(), api_key: "k".into(), ..Default::default() }),
            ..Default::default()
        };
        let env = Env::new(dir.path());
        let ctx = env.ctx(&accounts);
        let mut target =
            RetroTarget { roms: vec![rom], platform_id: Some("nintendo_nes".into()), title: "Mystery".into(), ..Default::default() };
        assert_eq!(fetch_from(&ctx, &server.base, &target), Fetch::Unmatched { hash: Some(ABC.into()), file: "Mystery.nes".into() });
        target.pinned_game_id = Some(6);
        assert!(matches!(fetch_from(&ctx, &server.base, &target), Fetch::Set(_)));
    }

    #[test]
    fn an_unhashable_rom_falls_back_to_an_exact_title() {
        let dir = tempfile::tempdir().unwrap();
        let chd = dir.path().join("Super Test (USA).chd");
        std::fs::write(&chd, b"MComprHD").unwrap();
        let server = serve(routes());
        let accounts = Accounts {
            retro_achievements: Some(RetroAccount { username: "me".into(), api_key: "k".into(), ..Default::default() }),
            ..Default::default()
        };
        let env = Env::new(dir.path());
        let ctx = env.ctx(&accounts);
        let mut target = RetroTarget {
            roms: vec![chd],
            platform_id: Some("sony_playstation2".into()),
            title: "The Super Test".into(),
            ..Default::default()
        };
        assert!(matches!(fetch_from(&ctx, &server.base, &target), Fetch::Set(_)));
        target.title = "Super".into();
        assert!(matches!(fetch_from(&ctx, &server.base, &target), Fetch::Unmatched { hash: None, .. }));
    }

    #[test]
    fn a_site_error_is_a_failure_without_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let rom = nes_rom(dir.path(), "x.nes");
        let server = serve(vec![("r=systemgames", 200, system_games(json!([]))), ("API_GetGameInfoAndUserProgress.php", 401, "{}".into())]);
        let accounts = Accounts {
            retro_achievements: Some(RetroAccount { username: "me".into(), api_key: "SECRET".into(), ..Default::default() }),
            ..Default::default()
        };
        let env = Env::new(dir.path());
        let ctx = env.ctx(&accounts);
        let target =
            RetroTarget { roms: vec![rom], platform_id: Some("nintendo_nes".into()), pinned_game_id: Some(6), ..Default::default() };
        let Fetch::Failed(e) = fetch_from(&ctx, &server.base, &target) else { panic!("expected a failure") };
        assert!(e.contains("rejected") && !e.contains("SECRET"), "{e}");
    }

    #[test]
    fn title_keys() {
        assert_eq!(title_key("The Legend of Zelda: A Link to the Past (USA) [!]"), title_key("Legend of Zelda, The: A Link to the Past"));
        assert_ne!(title_key("Sonic"), title_key("Sonic 2"));
        assert_eq!(title_key("(USA)"), "");
    }

    #[test]
    fn search_finds_titles_by_words() {
        let dir = tempfile::tempdir().unwrap();
        let server = serve(routes());
        let accounts = Accounts {
            retro_achievements: Some(RetroAccount { username: "me".into(), api_key: "k".into(), ..Default::default() }),
            ..Default::default()
        };
        let env = Env::new(dir.path());
        let ctx = env.ctx(&accounts);
        // The base is the real one, so only the cache can answer: fill it through fetch first.
        let rom = nes_rom(dir.path(), "a.nes");
        let target = RetroTarget { roms: vec![rom], platform_id: Some("nintendo_nes".into()), ..Default::default() };
        assert!(matches!(fetch_from(&ctx, &server.base, &target), Fetch::Set(_)));
        let hits = search_games(&ctx, &[], "test super");
        assert_eq!(hits, [GameHit { id: 6, title: "Super Test, The".into(), console_id: 7, console: "NES".into() }]);
        assert!(search_games(&ctx, &[7], "zzz").is_empty());
        assert!(search_games(&ctx, &[7], "  ").is_empty());
    }
}
