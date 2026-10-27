//! Battle.net: its games from the Agent's `product.db`, its sign-in from
//! `Battle.net.config`.

use std::path::Path;

use heronite_data::clients::drive_c;
use serde_json::Value;

use crate::files::{last_part, read_text, unix_path};
use crate::pbuf::{Fields, Value as Field};
use crate::winpath::join_ci;
use crate::{user_dirs, Account, ClientGame};

const PRODUCT_DB: &str = "ProgramData/Battle.net/Agent/product.db";
const CONFIG: &str = "AppData/Roaming/Battle.net/Battle.net.config";

/// Battle.net's own entries of the database: the client and its Agent.
const NOT_GAMES: [&str; 3] = ["agent", "bna", "battle.net"];

/// Games Battle.net knows by `uid`: `(uid, the code `launch` takes, title)`.
/// A game that is not here is started by its product code.
const KNOWN: &[(&str, &str, &str)] = &[
    ("wow", "WoW", "World of Warcraft"),
    ("wow_classic", "WoWC", "World of Warcraft Classic"),
    ("wow_classic_era", "WoW_wow_classic_era", "World of Warcraft Classic Era"),
    ("prometheus", "Pro", "Overwatch 2"),
    ("fenris", "Fen", "Diablo IV"),
    ("fen", "Fen", "Diablo IV"),
    ("diablo3", "D3", "Diablo III"),
    ("osi", "OSI", "Diablo II: Resurrected"),
    ("anbs", "ANBS", "Diablo Immortal"),
    ("d1", "D1", "Diablo"),
    ("hs_beta", "WTCG", "Hearthstone"),
    ("heroes", "Hero", "Heroes of the Storm"),
    ("s2", "S2", "StarCraft II"),
    ("s1", "S1", "StarCraft: Remastered"),
    ("w3", "W3", "Warcraft III: Reforged"),
    ("gryphon", "GRY", "Warcraft Rumble"),
    ("w1r", "W1R", "Warcraft: Orcs & Humans"),
    ("w2r", "W2R", "Warcraft II: Battle.net Edition"),
    ("rtro", "RTRO", "Blizzard Arcade Collection"),
    ("wlby", "WLBY", "Crash Bandicoot 4: It's About Time"),
    ("viper", "VIPR", "Call of Duty: Black Ops 4"),
    ("odin", "ODIN", "Call of Duty: Modern Warfare"),
    ("lazarus", "LAZR", "Call of Duty: Modern Warfare 2 Campaign Remastered"),
    ("zeus", "ZEUS", "Call of Duty: Black Ops Cold War"),
    ("fore", "FORE", "Call of Duty: Vanguard"),
    ("auks", "AUKS", "Call of Duty"),
    ("pinta", "PNTA", "Call of Duty: Modern Warfare III"),
    ("libra", "LBRA", "Tony Hawk's Pro Skater 3 + 4"),
    ("aris", "ARIS", "DOOM: The Dark Ages"),
    ("scorpio", "SCOR", "Sea of Thieves"),
    ("arkansas", "ARK", "The Outer Worlds 2"),
    ("aqua", "AQUA", "Avowed"),
];

/// A `ProductInstall` of the database.
#[derive(Debug, Default, PartialEq, Eq)]
struct Install {
    uid: String,
    product_code: String,
    /// `settings.install_path`, a Windows path with forward slashes.
    path: Option<String>,
    /// `cached_product_state.base_product_state.installed`, when it is there.
    installed: Option<bool>,
}

/// The `ProductInstall`s of `product.db` (field 1 of its `ProductDb`). What
/// cannot be read is left out.
fn installs(db: &[u8]) -> Vec<Install> {
    Fields::new(db)
        .filter_map(|(n, v)| match (n, v) {
            (1, Field::Bytes(b)) => Some(install(b)),
            _ => None,
        })
        .collect()
}

fn install(message: &[u8]) -> Install {
    let mut out = Install::default();
    for (n, v) in Fields::new(message) {
        match (n, v) {
            (1, v) => out.uid = v.as_str().unwrap_or_default().to_string(),
            (2, v) => out.product_code = v.as_str().unwrap_or_default().to_string(),
            // UserSettings { install_path = 1 }
            (3, Field::Bytes(b)) => {
                out.path = Fields::new(b)
                    .find_map(|(n, v)| (n == 1).then(|| v.as_str().map(str::trim).filter(|p| !p.is_empty()).map(str::to_string)).flatten());
            }
            // CachedProductState { base_product_state = 1 { installed = 1 } }
            (4, Field::Bytes(b)) => {
                let base = Fields::new(b).find_map(|(n, v)| match (n, v) {
                    (1, Field::Bytes(base)) => Some(base),
                    _ => None,
                });
                out.installed = base.and_then(|base| {
                    Fields::new(base).find_map(|(n, v)| match (n, v) {
                        (1, Field::Varint(flag)) => Some(flag != 0),
                        _ => None,
                    })
                });
            }
            _ => {}
        }
    }
    out
}

/// The games Battle.net has installed in `prefix`.
pub fn games(prefix: &Path) -> Vec<ClientGame> {
    let Ok(db) = std::fs::read(join_ci(&drive_c(prefix), PRODUCT_DB)) else { return Vec::new() };
    let mut seen = std::collections::HashSet::new();
    installs(&db).into_iter().filter_map(|i| game(prefix, i)).filter(|g| seen.insert(g.id.to_lowercase())).collect()
}

fn game(prefix: &Path, install: Install) -> Option<ClientGame> {
    let Install { uid, product_code, path, installed } = install;
    let id = if uid.is_empty() { product_code.clone() } else { uid };
    if id.is_empty() || [&id, &product_code].iter().any(|n| NOT_GAMES.iter().any(|b| n.eq_ignore_ascii_case(b))) {
        return None;
    }
    // Not installed, or nothing says it is on disk (a product known to the
    // client that was never installed has no folder).
    if installed == Some(false) || (installed.is_none() && path.is_none()) {
        return None;
    }
    let known = |name: &str| KNOWN.iter().find(|(uid, ..)| uid.eq_ignore_ascii_case(name));
    let (code, title) = match known(&id).or_else(|| known(&product_code)) {
        Some((_, code, title)) => (code.to_string(), title.to_string()),
        None => {
            let code = if product_code.is_empty() { id.clone() } else { product_code.clone() };
            let title = path.as_deref().and_then(last_part).map_or_else(|| code.clone(), str::to_string);
            (code, title)
        }
    };
    let folder = path.as_deref().and_then(|p| unix_path(prefix, p));
    let launch = format!("--exec=\"launch {}\"", code.replace('"', ""));
    Some(ClientGame { id, title, folder, launch })
}

/// Whether Battle.net remembers an account (its email).
pub fn account(prefix: &Path) -> Account {
    let mut out = Account::Unknown;
    for user in user_dirs(prefix) {
        let Some(text) = read_text(&join_ci(&user, CONFIG)) else { continue };
        let emails = serde_json::from_str::<Value>(&text).ok().and_then(|v| saved_accounts(&v).map(str::to_string));
        match emails.as_deref().and_then(|e| e.split(',').map(str::trim).find(|e| !e.is_empty())) {
            Some(first) => return Account::SignedIn(Some(first.to_string())),
            None => out = Account::SignedOut,
        }
    }
    out
}

/// `SavedAccountNames` of the config: under `Client`, or in a section named
/// by a hash.
fn saved_accounts(v: &Value) -> Option<&str> {
    let map = v.as_object()?;
    map.get("SavedAccountNames").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).or_else(|| map.values().find_map(saved_accounts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbuf::tests::Writer;

    pub(crate) fn product(uid: &str, code: &str, path: Option<&str>, installed: Option<bool>) -> Vec<u8> {
        let mut m = Writer::default().text(1, uid).text(2, code);
        if let Some(p) = path {
            m = m.bytes(3, &Writer::default().text(1, p).text(2, "us").0);
        }
        if let Some(flag) = installed {
            let base = Writer::default().num(1, flag.into()).num(2, flag.into()).text(7, "1.0.0").0;
            m = m.bytes(4, &Writer::default().bytes(1, &base).0);
        }
        Writer::default().bytes(1, &m.0).0
    }

    fn prefix_with_db(db: &[u8]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("drive_c").join(PRODUCT_DB);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, db).unwrap();
        dir
    }

    #[test]
    fn lists_the_installed_games_with_their_launch_codes() {
        let mut db = Vec::new();
        db.extend(product("battle.net", "bna", Some("C:/Program Files (x86)/Battle.net"), Some(true)));
        db.extend(product("agent", "agent", None, None));
        db.extend(product("WoW", "wow", Some("C:/Program Files (x86)/World of Warcraft"), Some(true)));
        db.extend(product("fenris", "fenris", Some("C:/Program Files (x86)/Diablo IV"), Some(true)));
        db.extend(product("diablo3", "d3", Some("C:/Games/Diablo III"), None));
        db.extend(product("prometheus", "prometheus", Some("C:/Program Files (x86)/Overwatch"), Some(false)));
        db.extend(product("newgame", "NGM", Some("C:/Program Files (x86)/The New Game"), Some(true)));
        db.extend(product("ghost", "GHO", None, None));
        // Fields this reader does not know are skipped.
        db.extend(Writer::default().text(2, "x").fixed64(3, 1).num(4, 1).0);
        let dir = prefix_with_db(&db);
        std::fs::create_dir_all(dir.path().join("drive_c/Program Files (x86)/World of Warcraft")).unwrap();

        let games = games(dir.path());
        let got: Vec<_> = games.iter().map(|g| (g.id.as_str(), g.title.as_str(), g.launch.as_str())).collect();
        assert_eq!(
            got,
            [
                ("WoW", "World of Warcraft", r#"--exec="launch WoW""#),
                ("fenris", "Diablo IV", r#"--exec="launch Fen""#),
                ("diablo3", "Diablo III", r#"--exec="launch D3""#),
                ("newgame", "The New Game", r#"--exec="launch NGM""#),
            ]
        );
        assert_eq!(games[0].folder.as_deref(), Some(dir.path().join("drive_c/Program Files (x86)/World of Warcraft").as_path()));
        // The launch text is one argument for the launch side.
        assert_eq!(shlex::split(&games[0].launch).unwrap(), [r#"--exec=launch WoW"#]);
    }

    #[test]
    fn a_missing_or_broken_database_has_no_games() {
        let dir = tempfile::tempdir().unwrap();
        assert!(games(dir.path()).is_empty());
        let broken = prefix_with_db(&[0x0A, 0xFF, 0x01]);
        assert!(games(broken.path()).is_empty());
        let junk = prefix_with_db(b"not a protobuf at all");
        assert!(games(junk.path()).is_empty());
    }

    fn prefix_with_config(text: Option<&str>) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("drive_c/users/steamuser");
        std::fs::create_dir_all(user.join("AppData/Roaming/Battle.net")).unwrap();
        std::fs::create_dir_all(dir.path().join("drive_c/users/Public")).unwrap();
        if let Some(text) = text {
            std::fs::write(user.join(CONFIG), text).unwrap();
        }
        dir
    }

    #[test]
    fn the_saved_account_says_who_is_signed_in() {
        let signed = prefix_with_config(Some(r#"{"Client": {"SavedAccountNames": "me@example.com, other@example.com"}}"#));
        assert_eq!(account(signed.path()), Account::SignedIn(Some("me@example.com".into())));
        let nested = prefix_with_config(Some(r#"{"Client": {"Hash": {"SavedAccountNames": "deep@example.com"}}, "Other": 1}"#));
        assert_eq!(account(nested.path()), Account::SignedIn(Some("deep@example.com".into())));
        let bom = prefix_with_config(Some("\u{feff}{\"Client\": {\"SavedAccountNames\": \"bom@example.com\"}}"));
        assert_eq!(account(bom.path()), Account::SignedIn(Some("bom@example.com".into())));
    }

    #[test]
    fn a_config_without_accounts_is_signed_out_and_no_config_is_unknown() {
        assert_eq!(account(prefix_with_config(Some(r#"{"Client": {"HardwareAcceleration": "false"}}"#)).path()), Account::SignedOut);
        assert_eq!(account(prefix_with_config(Some(r#"{"Client": {"SavedAccountNames": ""}}"#)).path()), Account::SignedOut);
        assert_eq!(account(prefix_with_config(None).path()), Account::Unknown);
        assert_eq!(account(prefix_with_config(Some("{ broken")).path()), Account::SignedOut);
        assert_eq!(account(tempfile::tempdir().unwrap().path()), Account::Unknown);
    }
}
