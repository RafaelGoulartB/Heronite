//! Building the index from the archive: `Metadata.xml` and `Files.xml` read
//! as streams straight out of the zip (never extracted, never loaded whole)
//! into a new SQLite file, in one transaction, then compacted. The caller
//! swaps it in only once it is complete and opens.

use std::collections::{HashMap, HashSet};
use std::io::{BufReader, Read, Write};
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use flate2::write::DeflateEncoder;
use flate2::Compression;
use rusqlite::{params, Connection, Transaction};

use super::index::{pack_file, Detail, ImageKind, BLOCK, SCHEMA, SCHEMA_AFTER, SCHEMA_VERSION};
use super::parse::{each_record, Record};
use crate::rom_names::{match_key, strip_extension};
use crate::sources::download::{Cancelled, DownloadProgress, Phase};
use crate::util::clip;

/// What a build read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Built {
    pub games: u64,
    pub names: u64,
    pub files: u64,
    pub images: u64,
    /// Records left out because their text does not read (a reference that
    /// is not closed, an impossible character).
    pub skipped: u64,
}

/// The archive itself does not read: not a zip, no `Metadata.xml`, its XML
/// cut short or not well formed, no games. Only another download mends
/// it; any other failure of a build (writing the index) keeps the archive.
#[derive(Debug)]
pub struct BadArchive;

impl std::fmt::Display for BadArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the LaunchBox archive does not read")
    }
}

/// An error of the index being written, told apart from the archive's.
#[derive(Debug)]
struct Written;

impl std::fmt::Display for Written {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "writing the LaunchBox index")
    }
}

/// Counts the bytes read through it, for the progress.
struct Counted<'a, R> {
    inner: R,
    read: u64,
    base: u64,
    total: u64,
    progress: &'a DownloadProgress,
}

impl<R: Read> Read for Counted<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.progress.cancelled() {
            return Err(std::io::Error::other(Cancelled));
        }
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        // Every ~4 MB is plenty for a bar.
        if self.read % (4 << 20) < n as u64 {
            self.progress.set(self.base + self.read, Some(self.total));
        }
        Ok(n)
    }
}

struct Writer<'t> {
    tx: Transaction<'t>,
    platforms: HashMap<String, i64>,
    regions: HashMap<String, i64>,
    /// Each game's platform, for its alternate names.
    game_platform: HashMap<i64, i64>,
    block: Vec<Detail>,
    seq: i64,
    /// The single pictures kept per game (the background, the square cover)
    /// and the box fronts per region.
    kept: HashSet<(i64, u8, String)>,
    built: Built,
}

impl Writer<'_> {
    fn platform(&mut self, name: &str) -> Result<i64> {
        if let Some(id) = self.platforms.get(name) {
            return Ok(*id);
        }
        let id = self.platforms.len() as i64 + 1;
        self.tx.execute("INSERT INTO platforms (id, name) VALUES (?, ?)", params![id, name])?;
        self.platforms.insert(name.to_owned(), id);
        Ok(id)
    }

    /// The number of a region; 0 for none.
    fn region(&mut self, name: &str) -> Result<i64> {
        if name.is_empty() {
            return Ok(0);
        }
        if let Some(id) = self.regions.get(name) {
            return Ok(*id);
        }
        let id = self.regions.len() as i64 + 1;
        self.tx.execute("INSERT INTO regions (id, name) VALUES (?, ?)", params![id, name])?;
        self.regions.insert(name.to_owned(), id);
        Ok(id)
    }

    fn name(&mut self, platform: i64, name: &str, game: i64, alt: bool) -> Result<()> {
        let key = match_key(name);
        if key.is_empty() {
            return Ok(());
        }
        let mut stmt = self.tx.prepare_cached("INSERT OR IGNORE INTO names (platform, key, game, alt) VALUES (?, ?, ?, ?)")?;
        self.built.names += stmt.execute(params![platform, key, game, alt as i64])? as u64;
        Ok(())
    }

    fn game(&mut self, r: &Record) -> Result<()> {
        let (Some(id), Some(name), Some(platform)) =
            (r.text("DatabaseID").and_then(|s| s.parse::<i64>().ok()), r.text("Name"), r.text("Platform"))
        else {
            return Ok(());
        };
        let platform = self.platform(platform)?;
        let released = r
            .text("ReleaseDate")
            .map(|d| d.chars().take(10).collect::<String>())
            .filter(|d| d.len() == 10)
            .or_else(|| r.text("ReleaseYear").filter(|y| y.len() == 4).map(str::to_owned));
        let steam = r.text("SteamAppId").and_then(|s| s.parse::<i64>().ok()).filter(|n| *n > 0);
        let seq = self.seq;
        let inserted = self
            .tx
            .prepare_cached("INSERT OR IGNORE INTO games (id, platform, name, released, steam, seq) VALUES (?, ?, ?, ?, ?, ?)")?
            .execute(params![id, platform, name, released, steam, seq])?;
        if inserted == 0 {
            return Ok(());
        }
        self.seq += 1;
        self.built.games += 1;
        self.game_platform.insert(id, platform);
        self.name(platform, name, id, false)?;
        self.block.push(Detail {
            overview: clip(r.text("Overview")),
            developer: r.text("Developer").map(str::to_owned),
            publisher: r.text("Publisher").map(str::to_owned),
            genres: r.text("Genres").map(str::to_owned),
            max_players: r.text("MaxPlayers").and_then(|s| s.parse().ok()),
            cooperative: r.text("Cooperative") == Some("true"),
            rating: r.text("CommunityRating").and_then(|s| s.parse().ok()).filter(|r: &f64| (0.0..=5.0).contains(r)),
            votes: r.text("CommunityRatingCount").and_then(|s| s.parse().ok()).unwrap_or(0),
        });
        if self.block.len() as i64 == BLOCK {
            self.flush_block()?;
        }
        Ok(())
    }

    fn flush_block(&mut self) -> Result<()> {
        if self.block.is_empty() {
            return Ok(());
        }
        let number = (self.seq - 1) / BLOCK;
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::best());
        enc.write_all(&serde_json::to_vec(&self.block)?)?;
        let data = enc.finish()?;
        self.tx.prepare_cached("INSERT INTO details (block, data) VALUES (?, ?)")?.execute(params![number, data])?;
        self.block.clear();
        Ok(())
    }

    fn alternate(&mut self, r: &Record) -> Result<()> {
        let (Some(game), Some(name)) = (r.text("DatabaseID").and_then(|s| s.parse::<i64>().ok()), r.text("AlternateName")) else {
            return Ok(());
        };
        match self.game_platform.get(&game) {
            Some(platform) => self.name(*platform, name, game, true),
            None => Ok(()),
        }
    }

    fn image(&mut self, r: &Record) -> Result<()> {
        let (Some(game), Some(kind), Some(file)) =
            (r.text("DatabaseID").and_then(|s| s.parse::<i64>().ok()), r.text("Type").and_then(ImageKind::from_type), r.text("FileName"))
        else {
            return Ok(());
        };
        if !self.game_platform.contains_key(&game) {
            return Ok(());
        }
        let region = r.text("Region").unwrap_or("");
        // Box fronts: one per region; the others: one per game.
        let slot = match kind {
            ImageKind::BoxFront | ImageKind::BoxFrontReconstructed => region.to_owned(),
            ImageKind::Background | ImageKind::Square => String::new(),
        };
        if !self.kept.insert((game, kind as u8, slot)) {
            return Ok(());
        }
        let region = self.region(region)?;
        let mut stmt = self.tx.prepare_cached("INSERT OR IGNORE INTO images (game, kind, region, file) VALUES (?, ?, ?, ?)")?;
        self.built.images += stmt.execute(params![game, kind as u8, region, pack_file(file)])? as u64;
        Ok(())
    }

    fn file(&mut self, r: &Record) -> Result<()> {
        let (Some(platform), Some(file), Some(name)) = (r.text("Platform"), r.text("FileName"), r.text("GameName")) else { return Ok(()) };
        let key = match_key(name);
        if key.is_empty() {
            return Ok(());
        }
        let platform = self.platform(platform)?;
        let file = strip_extension(file).to_lowercase();
        let mut stmt = self.tx.prepare_cached("INSERT OR IGNORE INTO files (platform, file, key) VALUES (?, ?, ?)")?;
        self.built.files += stmt.execute(params![platform, file, key])? as u64;
        Ok(())
    }
}

/// Builds the index at `out` (replaced if there) from the archive at
/// `archive`, reporting the bytes of XML read to `progress`
/// ([`Phase::Preparing`]). Fails on an archive that is not a zip, lacks
/// `Metadata.xml`, or is cut short ([`BadArchive`]); `out` is then
/// incomplete and the caller drops it. A record whose text does not read is
/// left out and counted ([`Built::skipped`]).
pub fn build(archive: &Path, out: &Path, progress: &DownloadProgress) -> Result<Built> {
    let file = std::fs::File::open(archive).with_context(|| format!("cannot open {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file)).context("the LaunchBox archive is not a zip file").context(BadArchive)?;
    let size = |zip: &mut zip::ZipArchive<_>, name: &str| zip.by_name(name).map(|f| f.size()).unwrap_or(0);
    let metadata_size = size(&mut zip, "Metadata.xml");
    if metadata_size == 0 {
        return Err(anyhow!("the LaunchBox archive has no Metadata.xml").context(BadArchive));
    }
    let total = metadata_size + size(&mut zip, "Files.xml");
    progress.set_phase(Phase::Preparing);
    progress.set(0, Some(total));

    let _ = std::fs::remove_file(out);
    let mut conn = Connection::open(out).with_context(|| format!("cannot create {}", out.display()))?;
    // A file being built is thrown away when anything fails: no journal.
    conn.execute_batch("PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF; PRAGMA page_size = 4096;")?;
    conn.execute_batch(SCHEMA)?;
    let built = {
        let tx = conn.transaction()?;
        let mut w = Writer {
            tx,
            platforms: HashMap::new(),
            regions: HashMap::new(),
            game_platform: HashMap::new(),
            block: Vec::with_capacity(BLOCK as usize),
            seq: 0,
            kept: HashSet::new(),
            built: Built::default(),
        };
        let stream =
            |zip: &mut zip::ZipArchive<BufReader<std::fs::File>>, name: &str, base: u64, f: &mut dyn FnMut(&Record) -> Result<()>| {
                let entry = zip.by_name(name).context(BadArchive)?;
                let counted = Counted { inner: entry, read: 0, base, total, progress };
                each_record(BufReader::with_capacity(256 * 1024, counted), |r| f(r).context(Written)).map_err(|e| {
                    // The reader stops with an I/O error once cancelled.
                    if progress.cancelled() {
                        anyhow!(Cancelled)
                    } else if e.is::<Written>() {
                        e
                    } else {
                        e.context(format!("reading {name}")).context(BadArchive)
                    }
                })
            };
        let mut skipped = 0;
        skipped += stream(&mut zip, "Metadata.xml", 0, &mut |r| match r.kind.as_str() {
            "Game" => w.game(r),
            "GameAlternateName" => w.alternate(r),
            "GameImage" => w.image(r),
            _ => Ok(()),
        })?;
        w.flush_block()?;
        if zip.by_name("Files.xml").is_ok() {
            skipped += stream(&mut zip, "Files.xml", metadata_size, &mut |r| if r.kind == "File" { w.file(r) } else { Ok(()) })?;
        }
        if w.built.games == 0 {
            return Err(anyhow!("the LaunchBox archive has no games").context(BadArchive));
        }
        if skipped > 0 {
            log::warn!("LaunchBox archive: {skipped} records whose text does not read were left out");
        }
        w.built.skipped = skipped;
        let Writer { tx, built, .. } = w;
        tx.execute("INSERT INTO meta (key, value) VALUES ('schema', ?)", [SCHEMA_VERSION])?;
        tx.execute("INSERT INTO meta (key, value) VALUES ('games', ?)", [built.games.to_string()])?;
        tx.commit()?;
        built
    };
    conn.execute_batch(SCHEMA_AFTER)?;
    // Rows went in out of key order: compacting halves the file.
    conn.execute_batch("VACUUM;")?;
    progress.set(total, Some(total));
    Ok(built)
}
