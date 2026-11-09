//! GOG achievements while the game runs: what Comet, GOG Galaxy's service
//! beside a GOG game (`heronite_launch::galaxy`), writes into its database
//! as the game unlocks them. GOG's servers get them a few seconds later at
//! best, so the live check reads the database instead.
//!
//! Comet keeps one SQLite database per game and account:
//! `<data>/gameplay/<client id>/<galaxy user id>/gameplay.db`, `<data>` being
//! `$XDG_DATA_HOME/comet` (`~/.local/share/comet`) on Linux and
//! `%LOCALAPPDATA%\comet` on Windows (its `paths.rs`). Its `achievement`
//! table has the game's achievements as GOG's API gives them, and their
//! unlock time (RFC 3339, or GOG's format) once unlocked.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use heronite_data::paths::DataPaths;
use rusqlite::{Connection, OpenFlags};

use super::gog::{self, parse_date};
use super::{Ctx, Fetch};
use crate::model::{Achievement, AchievementSet, Source};

/// Comet's data folder, as Comet finds it (`None` on other systems).
pub fn data_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("comet"));
    }
    if cfg!(target_os = "linux") {
        // Comet takes the variable as it is, even empty.
        return match std::env::var_os("XDG_DATA_HOME") {
            Some(d) => Some(PathBuf::from(d).join("comet")),
            None => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/comet")),
        };
    }
    None
}

/// The database of one game (by its Galaxy client id) and account.
pub fn database(data_dir: &Path, client_id: &str, user_id: &str) -> PathBuf {
    data_dir.join("gameplay").join(client_id).join(user_id).join("gameplay.db")
}

/// The database of the GOG game `product_id` for the signed-in account,
/// when the game is installed and has a client id. Reads two small files.
pub fn database_of(paths: &DataPaths, product_id: &str) -> Option<PathBuf> {
    let user = heronite_gog::user::account(paths)?.user_id;
    let install = heronite_gog::installed::get(paths, product_id)?;
    let client = heronite_gog::saves::client_id(paths, product_id, &install.install_path)?;
    Some(database(&data_dir()?, &client, &user))
}

/// The files whose change means an unlock: the database and its
/// write-ahead log (SQLite writes there first in WAL mode).
pub fn watched_files(db: &Path) -> Vec<PathBuf> {
    let mut wal = db.as_os_str().to_owned();
    wal.push("-wal");
    vec![db.to_path_buf(), PathBuf::from(wal)]
}

/// One row of the `achievement` table.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    id: i64,
    key: String,
    name: String,
    description: String,
    visible: bool,
    unlock_time: Option<String>,
    icon_locked: String,
    icon: String,
    rarity: Option<f64>,
}

impl Row {
    /// The id the GOG source gives it ([`gog::parse`]).
    fn achievement_id(&self) -> String {
        if self.key.is_empty() {
            self.id.to_string()
        } else {
            self.key.clone()
        }
    }

    fn unlocked(&self) -> bool {
        self.unlock_time.as_deref().is_some_and(|t| !t.is_empty())
    }

    fn achievement(&self) -> Achievement {
        Achievement {
            id: self.achievement_id(),
            name: self.name.clone(),
            description: self.description.clone(),
            hidden: !self.visible,
            icon: self.icon.clone(),
            icon_locked: self.icon_locked.clone(),
            unlocked: self.unlocked(),
            unlocked_at: self.unlock_time.as_deref().and_then(parse_date),
            rarity: self.rarity.map(|r| r as f32),
            ..Default::default()
        }
    }
}

/// The achievements in the database, read-only (Comet may be writing).
fn read(db: &Path) -> rusqlite::Result<Vec<Row>> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    conn.busy_timeout(Duration::from_secs(2))?;
    let mut stmt = conn.prepare(
        "SELECT id, key, name, description, visible_while_locked, unlock_time, image_url_locked, image_url_unlocked, rarity \
         FROM achievement ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Row {
            id: r.get(0)?,
            key: r.get(1)?,
            name: r.get(2)?,
            description: r.get(3)?,
            visible: r.get::<_, i64>(4)? != 0,
            unlock_time: r.get(5)?,
            icon_locked: r.get(6)?,
            icon: r.get(7)?,
            rarity: r.get(8)?,
        })
    })?;
    rows.collect()
}

/// The set a live read gives: the copy's (names in the interface language,
/// as GOG's API gave them) with the database's unlocks; the database's own
/// when there is no copy yet.
fn with_unlocks(product_id: &str, old: Option<&AchievementSet>, rows: &[Row]) -> AchievementSet {
    let Some(old) = old else {
        return AchievementSet {
            source: Source::Gog,
            external_id: product_id.into(),
            title: None,
            fetched_at: Utc::now(),
            achievements: rows.iter().map(Row::achievement).collect(),
            groups: Vec::new(),
            unknown: Default::default(),
        };
    };
    let mut set = old.clone();
    for row in rows {
        let id = row.achievement_id();
        match set.achievements.iter_mut().find(|a| a.id == id) {
            Some(a) if row.unlocked() && !a.unlocked => {
                a.unlocked = true;
                a.unlocked_at = row.unlock_time.as_deref().and_then(parse_date);
            }
            Some(_) => {}
            None => set.achievements.push(row.achievement()),
        }
    }
    set
}

/// GOG's achievements of `product_id` during a session: from Comet's
/// database, else (no database, or nothing in it yet) from GOG.
pub fn fetch(ctx: &Ctx, product_id: &str, old: Option<&AchievementSet>) -> Fetch {
    let rows = match database_of(ctx.paths, product_id).filter(|db| db.is_file()) {
        Some(db) => read(&db).unwrap_or_else(|e| {
            log::debug!("reading Comet's database {}: {e}", db.display());
            Vec::new()
        }),
        None => Vec::new(),
    };
    if rows.is_empty() {
        return gog::fetch(ctx, product_id);
    }
    Fetch::Set(with_unlocks(product_id, old, &rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database as Comet creates it (its `SETUP_QUERY`, the `achievement` table).
    fn comet_db(path: &Path, rows: &[(i64, &str, Option<&str>)]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS `achievement` (`id` INTEGER PRIMARY KEY NOT NULL,`key` TEXT UNIQUE NOT NULL,`name` TEXT NOT NULL,\
             `description` TEXT NOT NULL,`visible_while_locked` INTEGER CHECK ( visible_while_locked IN ( 0, 1 ) ) NOT NULL,`unlock_time` TEXT,\
             `image_url_locked` TEXT NOT NULL,`image_url_unlocked` TEXT NOT NULL,`changed` INTEGER CHECK ( changed IN ( 0, 1 ) ) NOT NULL, \
             rarity REAL NOT NULL, rarity_level_description TEXT NOT NULL, rarity_level_slug TEXT NOT NULL);",
        )
        .unwrap();
        for (id, key, time) in rows {
            conn.execute(
                "INSERT INTO achievement VALUES (?1, ?2, ?3, 'desc', 1, ?4, 'l', 'u', 0, 12.5, '', '')",
                rusqlite::params![id, key, format!("Name {id}"), time],
            )
            .unwrap();
        }
    }

    #[test]
    fn the_database_lives_where_comet_puts_it() {
        let db = database(Path::new("/d/comet"), "5007", "4242");
        assert_eq!(db, Path::new("/d/comet/gameplay/5007/4242/gameplay.db"));
        assert_eq!(watched_files(&db), [db.clone(), PathBuf::from("/d/comet/gameplay/5007/4242/gameplay.db-wal")]);
    }

    #[test]
    fn rows_become_gog_achievements() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("gameplay.db");
        comet_db(&db, &[(1, "WIN", Some("2026-11-09T12:00:00+00:00")), (2, "", None), (3, "OLD", Some("2026-01-02T03:04:05+0000"))]);
        let rows = read(&db).unwrap();
        let set = with_unlocks("42", None, &rows);
        assert_eq!((set.source, set.external_id.as_str()), (Source::Gog, "42"));
        let ids: Vec<&str> = set.achievements.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["WIN", "2", "OLD"], "the key, else the number, as GOG's API");
        let win = &set.achievements[0];
        assert!(win.unlocked && !win.hidden);
        assert_eq!(win.unlocked_at.unwrap().to_rfc3339(), "2026-11-09T12:00:00+00:00");
        assert_eq!(win.rarity, Some(12.5));
        assert!(!set.achievements[1].unlocked);
        assert_eq!(set.achievements[2].unlocked_at.unwrap().to_rfc3339(), "2026-01-02T03:04:05+00:00");
    }

    #[test]
    fn the_copy_keeps_its_names_and_takes_the_unlocks() {
        let old = AchievementSet {
            source: Source::Gog,
            external_id: "42".into(),
            title: None,
            fetched_at: Utc::now() - chrono::Duration::days(1),
            achievements: vec![
                Achievement { id: "WIN".into(), name: "Gewinnen".into(), ..Default::default() },
                Achievement {
                    id: "2".into(),
                    name: "Früh".into(),
                    unlocked: true,
                    unlocked_at: parse_date("2026-01-01T00:00:00+0000"),
                    ..Default::default()
                },
            ],
            groups: Vec::new(),
            unknown: Default::default(),
        };
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("gameplay.db");
        comet_db(&db, &[(1, "WIN", Some("2026-11-09T12:00:00+00:00")), (2, "", Some("2026-11-09T13:00:00+00:00")), (3, "NEW", None)]);
        let set = with_unlocks("42", Some(&old), &read(&db).unwrap());
        assert_eq!(set.fetched_at, old.fetched_at, "GOG is still asked after the session");
        assert_eq!(set.achievements[0].name, "Gewinnen");
        assert!(set.achievements[0].unlocked);
        assert_eq!(set.achievements[1].unlocked_at, old.achievements[1].unlocked_at, "an earlier unlock stays");
        assert_eq!(set.achievements[2].id, "NEW");
    }

    #[test]
    fn a_database_that_is_not_comets_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("gameplay.db");
        Connection::open(&db).unwrap().execute_batch("CREATE TABLE other (x INTEGER);").unwrap();
        assert!(read(&db).is_err());
        assert!(read(&dir.path().join("missing.db")).is_err());
    }
}
