//! The local index of the LaunchBox Games Database: one SQLite file the
//! lookups query in place (nothing is loaded whole).
//!
//! - `games`: id (LaunchBox's DatabaseID), platform, name, release date,
//!   Steam app id, and the game's place among the `details` blocks.
//! - `details`: the long texts and numbers (overview, developer, publisher,
//!   genres, players, rating), 64 games per block, JSON deflated: the
//!   overviews are most of the database, and compress twice as well in
//!   blocks as one by one. A lookup inflates one block (about 40 KB).
//! - `names`: each game's name and alternate names as `rom_names::match_key`
//!   leaves them, by platform (`alt` = 1 for an alternate name).
//! - `files`: LaunchBox's own file name → title table (`Files.xml`), by
//!   platform: WHDLoad and MAME set names, and dump names it knows.
//! - `images`: the pictures the source uses (box fronts of every region,
//!   one fan art background, one square cover), regions by number
//!   (`regions`) and file names packed ([`pack_file`]): most are a UUID.

use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use flate2::read::DeflateDecoder;
use rusqlite::{params, params_from_iter, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Bumped when the schema changes: an index of another version is built again.
pub(super) const SCHEMA_VERSION: &str = "1";
/// Games per `details` block.
pub(super) const BLOCK: i64 = 64;

pub(super) const SCHEMA: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE platforms (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
CREATE TABLE games (id INTEGER PRIMARY KEY, platform INTEGER NOT NULL, name TEXT NOT NULL, released TEXT, steam INTEGER, seq INTEGER NOT NULL);
CREATE TABLE details (block INTEGER PRIMARY KEY, data BLOB NOT NULL);
CREATE TABLE names (platform INTEGER NOT NULL, key TEXT NOT NULL, game INTEGER NOT NULL, alt INTEGER NOT NULL, PRIMARY KEY (platform, key, game)) WITHOUT ROWID;
CREATE TABLE files (platform INTEGER NOT NULL, file TEXT NOT NULL, key TEXT NOT NULL, PRIMARY KEY (platform, file)) WITHOUT ROWID;
CREATE TABLE regions (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
CREATE TABLE images (game INTEGER NOT NULL, kind INTEGER NOT NULL, region INTEGER NOT NULL, file BLOB NOT NULL, PRIMARY KEY (game, kind, region, file)) WITHOUT ROWID;
";

/// Built after the rows are in (faster than keeping it up to date).
pub(super) const SCHEMA_AFTER: &str = "CREATE INDEX games_steam ON games (steam) WHERE steam IS NOT NULL;";

/// The picture types the source uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ImageKind {
    /// "Box - Front".
    BoxFront = 1,
    /// "Box - Front - Reconstructed": a box made up from scans, when there is no front.
    BoxFrontReconstructed = 2,
    /// "Fanart - Background": 16:9 art without text.
    Background = 3,
    /// "Square": a 1:1 cover.
    Square = 4,
}

impl ImageKind {
    pub(super) fn from_type(t: &str) -> Option<Self> {
        Some(match t {
            "Box - Front" => Self::BoxFront,
            "Box - Front - Reconstructed" => Self::BoxFrontReconstructed,
            "Fanart - Background" => Self::Background,
            "Square" => Self::Square,
            _ => return None,
        })
    }

    fn from_code(n: u8) -> Option<Self> {
        [Self::BoxFront, Self::BoxFrontReconstructed, Self::Background, Self::Square].into_iter().find(|k| *k as u8 == n)
    }
}

/// What a game's `details` block keeps of it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Detail {
    /// Clipped and on one line ([`crate::util::clip`]).
    #[serde(rename = "o", default, skip_serializing_if = "Option::is_none")]
    pub overview: Option<String>,
    #[serde(rename = "d", default, skip_serializing_if = "Option::is_none")]
    pub developer: Option<String>,
    #[serde(rename = "p", default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    /// `Action; Platform`.
    #[serde(rename = "g", default, skip_serializing_if = "Option::is_none")]
    pub genres: Option<String>,
    #[serde(rename = "m", default, skip_serializing_if = "Option::is_none")]
    pub max_players: Option<u32>,
    #[serde(rename = "c", default, skip_serializing_if = "std::ops::Not::not")]
    pub cooperative: bool,
    /// LaunchBox's community rating, 0..=5.
    #[serde(rename = "r", default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<f64>,
    #[serde(rename = "v", default, skip_serializing_if = "is_zero")]
    pub votes: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// Extensions of the picture files whose name is a UUID.
const EXTENSIONS: [&str; 4] = ["jpg", "png", "jpeg", "webp"];

/// The prefix of LaunchBox's newer picture names (`r2_<uuid>.jpg`).
const R2: &str = "r2_";
/// Set in the extension code of a name with [`R2`].
const R2_FLAG: u8 = 0x80;

/// A picture's file name in 17 bytes when it is `[r2_]<uuid>.<ext>` (an
/// extension code, then the UUID's bytes), else 0 and the name.
pub(super) fn pack_file(name: &str) -> Vec<u8> {
    let (flag, rest) = match name.strip_prefix(R2) {
        Some(rest) => (R2_FLAG, rest),
        None => (0, name),
    };
    let packed = rest.split_once('.').and_then(|(stem, ext)| {
        let code = (EXTENSIONS.iter().position(|e| *e == ext)? as u8 + 1) | flag;
        let hex: String = stem.chars().filter(|c| *c != '-').collect();
        let dashes = stem.char_indices().filter(|(_, c)| *c == '-').map(|(i, _)| i).collect::<Vec<_>>();
        if stem.len() != 36 || dashes != [8, 13, 18, 23] || hex.len() != 32 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return None;
        }
        let mut out = vec![code];
        out.extend((0..16).map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0)));
        Some(out)
    });
    packed.unwrap_or_else(|| std::iter::once(0).chain(name.bytes()).collect())
}

/// The name [`pack_file`] packed.
pub(super) fn unpack_file(bytes: &[u8]) -> Option<String> {
    let (&code, rest) = bytes.split_first()?;
    if code == 0 {
        return String::from_utf8(rest.to_vec()).ok();
    }
    let prefix = if code & R2_FLAG != 0 { R2 } else { "" };
    let ext = EXTENSIONS.get(usize::from(code & !R2_FLAG).checked_sub(1)?)?;
    let hex: String = rest.iter().map(|b| format!("{b:02x}")).collect();
    (hex.len() == 32).then(|| format!("{prefix}{}-{}-{}-{}-{}.{ext}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LbImage {
    pub kind: ImageKind,
    /// LaunchBox's region (`North America`, `World`...); `""` for none.
    pub region: String,
    /// The file name under the images address.
    pub file: String,
}

/// A game of the database with everything the source maps.
#[derive(Debug, Clone, PartialEq)]
pub struct LbGame {
    pub id: i64,
    /// LaunchBox's platform name.
    pub platform: String,
    pub name: String,
    /// `YYYY-MM-DD` or `YYYY`.
    pub released: Option<String>,
    pub steam_app_id: Option<i64>,
    pub detail: Detail,
    pub images: Vec<LbImage>,
}

/// A name of a game found in `names`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameHit {
    pub game: i64,
    pub platform: i64,
    /// An alternate name, not the game's own.
    pub alt: bool,
}

pub struct Index {
    conn: Connection,
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

impl Index {
    /// Opens an index read-only. Fails when it is not one of this schema.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .with_context(|| format!("cannot open {}", path.display()))?;
        let index = Self { conn };
        let version = index.meta("schema")?;
        anyhow::ensure!(version.as_deref() == Some(SCHEMA_VERSION), "the index is of another version ({version:?})");
        Ok(index)
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key = ?", [key], |r| r.get(0)).optional()?)
    }

    pub fn game_count(&self) -> Result<u64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM games", [], |r| r.get::<_, i64>(0))? as u64)
    }

    /// The ids of the platforms named `names`, in their order (unknown ones left out).
    pub fn platform_ids(&self, names: &[&str]) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare_cached("SELECT id FROM platforms WHERE name = ?")?;
        let mut out = Vec::new();
        for name in names {
            if let Some(id) = stmt.query_row([name], |r| r.get(0)).optional()? {
                out.push(id);
            }
        }
        Ok(out)
    }

    /// The games named `key` on `platforms`.
    pub fn by_key(&self, platforms: &[i64], key: &str) -> Result<Vec<NameHit>> {
        if platforms.is_empty() || key.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!("SELECT game, platform, alt FROM names WHERE key = ? AND platform IN ({})", placeholders(platforms.len()));
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let args = std::iter::once(rusqlite::types::Value::Text(key.to_owned())).chain(platforms.iter().map(|p| (*p).into()));
        let rows = stmt
            .query_map(params_from_iter(args), |r| Ok(NameHit { game: r.get(0)?, platform: r.get(1)?, alt: r.get::<_, i64>(2)? != 0 }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The games on `platforms` whose name starts with `key` and is the
    /// shortest such name (the closest to it), `key` itself left out.
    pub fn by_prefix(&self, platforms: &[i64], key: &str) -> Result<Vec<NameHit>> {
        if platforms.is_empty() || key.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT game, platform, alt, key FROM names WHERE key > ? AND key < ? AND platform IN ({}) ORDER BY length(key) LIMIT 50",
            placeholders(platforms.len())
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        // Past every key that starts with `key` (U+10FFFF sorts after any text).
        let upper = format!("{key}\u{10FFFF}");
        let args = [rusqlite::types::Value::Text(key.to_owned()), rusqlite::types::Value::Text(upper)]
            .into_iter()
            .chain(platforms.iter().map(|p| (*p).into()));
        let rows = stmt.query_map(params_from_iter(args), |r| {
            Ok((NameHit { game: r.get(0)?, platform: r.get(1)?, alt: r.get::<_, i64>(2)? != 0 }, r.get::<_, String>(3)?.chars().count()))
        })?;
        let rows: Vec<(NameHit, usize)> = rows.collect::<rusqlite::Result<_>>()?;
        let shortest = rows.first().map(|(_, n)| *n);
        Ok(rows.into_iter().filter(|(_, n)| Some(*n) == shortest).map(|(h, _)| h).collect())
    }

    /// The key LaunchBox's file table gives `file` (a file name without its
    /// extension, any case) on one of `platforms`.
    pub fn file_key(&self, platforms: &[i64], file: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare_cached("SELECT key FROM files WHERE platform = ? AND file = ?")?;
        let file = file.to_lowercase();
        for p in platforms {
            if let Some(key) = stmt.query_row(params![p, file], |r| r.get(0)).optional()? {
                return Ok(Some(key));
            }
        }
        Ok(None)
    }

    /// The games with Steam app id `id` on `platforms`.
    pub fn by_steam_id(&self, platforms: &[i64], id: i64) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare_cached("SELECT id, platform FROM games WHERE steam = ? ORDER BY id")?;
        let rows = stmt.query_map([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        Ok(rows.filter_map(|r| r.ok()).filter(|(_, p)| platforms.contains(p)).map(|(g, _)| g).collect())
    }

    /// Games of any platform with a name or alternate name that is `key`,
    /// at most `limit` (for a search by hand).
    pub fn named(&self, key: &str, limit: usize) -> Result<Vec<i64>> {
        // Every platform named, so the key's primary index is used.
        self.games_where("platform IN (SELECT id FROM platforms) AND key = ?1", "game", key, limit)
    }

    /// Games of any platform with a longer name that starts with `key`,
    /// shortest names first, at most `limit`.
    pub fn starting(&self, key: &str, limit: usize) -> Result<Vec<i64>> {
        if key.is_empty() {
            return Ok(Vec::new());
        }
        // Past every key that starts with `key` (U+10FFFF sorts after any text).
        let upper = format!("{key}\u{10FFFF}");
        let sql = "SELECT game FROM names WHERE platform IN (SELECT id FROM platforms) AND key > ?1 AND key < ?3 ORDER BY length(key), game LIMIT ?2";
        let mut stmt = self.conn.prepare_cached(sql)?;
        let rows = stmt.query_map(params![key, limit as i64, upper], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Games of any platform with a name that holds `key` past its start,
    /// shortest names first, at most `limit` (a scan of every name).
    pub fn containing(&self, key: &str, limit: usize) -> Result<Vec<i64>> {
        self.games_where("instr(key, ?1) > 1", "length(key), game", key, limit)
    }

    fn games_where(&self, filter: &str, order: &str, key: &str, limit: usize) -> Result<Vec<i64>> {
        if key.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!("SELECT game FROM names WHERE {filter} ORDER BY {order} LIMIT ?2");
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![key, limit as i64], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The names a game is known by (its own first).
    pub fn names_of(&self, game: i64) -> Result<Vec<(String, bool)>> {
        let mut stmt = self.conn.prepare_cached("SELECT key, alt FROM names WHERE game = ? ORDER BY alt")?;
        let rows = stmt.query_map([game], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One game with its details and pictures.
    pub fn game(&self, id: i64) -> Result<Option<LbGame>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT g.id, p.name, g.name, g.released, g.steam, g.seq FROM games g JOIN platforms p ON p.id = g.platform WHERE g.id = ?",
        )?;
        let row = stmt
            .query_row([id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, Option<i64>>(4)?, r.get::<_, i64>(5)?))
            })
            .optional()?;
        let Some((id, platform, name, released, steam, seq)) = row else { return Ok(None) };
        let detail = self.detail(seq)?;
        let mut stmt = self.conn.prepare_cached(
            "SELECT i.kind, coalesce(r.name, ''), i.file FROM images i LEFT JOIN regions r ON r.id = i.region WHERE i.game = ?",
        )?;
        let images = stmt
            .query_map([id], |r| Ok((r.get::<_, u8>(0)?, r.get::<_, String>(1)?, r.get::<_, Vec<u8>>(2)?)))?
            .filter_map(|r| r.ok())
            .filter_map(|(kind, region, file)| Some(LbImage { kind: ImageKind::from_code(kind)?, region, file: unpack_file(&file)? }))
            .collect();
        Ok(Some(LbGame { id, platform, name, released, steam_app_id: steam, detail, images }))
    }

    /// The details of the game at `seq`.
    fn detail(&self, seq: i64) -> Result<Detail> {
        let mut stmt = self.conn.prepare_cached("SELECT data FROM details WHERE block = ?")?;
        let Some(data) = stmt.query_row([seq / BLOCK], |r| r.get::<_, Vec<u8>>(0)).optional()? else { return Ok(Detail::default()) };
        let mut json = Vec::with_capacity(data.len() * 3);
        DeflateDecoder::new(data.as_slice()).read_to_end(&mut json)?;
        let mut block: Vec<Detail> = serde_json::from_slice(&json)?;
        let slot = (seq % BLOCK) as usize;
        Ok(if slot < block.len() { block.swap_remove(slot) } else { Detail::default() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_pack_and_unpack() {
        for name in
            ["0f840f55-28c2-4d6e-a946-1fd62417f3be.png", "r2_8efb1efd-08b6-41bf-9951-dfe0d4371f6f.jpg", "Super Mario-01.jpg", "a.gif"]
        {
            assert_eq!(unpack_file(&pack_file(name)).as_deref(), Some(name), "{name}");
        }
        assert_eq!(pack_file("0f840f55-28c2-4d6e-a946-1fd62417f3be.png").len(), 17);
        assert_eq!(pack_file("r2_8efb1efd-08b6-41bf-9951-dfe0d4371f6f.jpg").len(), 17);
        assert_eq!(pack_file("0F840F55-28C2-4D6E-A946-1FD62417F3BE.png")[0], 0, "upper case is kept as it is");
    }
}
