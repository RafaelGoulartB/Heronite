//! Games the Amazon Games app installed (Windows): they become nile's
//! installed games without a new download.
//!
//! The app lists its installs in the SQLite database
//! `%LOCALAPPDATA%\Amazon Games\Data\Games\Sql\GameInstallInfo.sqlite`
//! (table `DbSet`: `Id` is the product id, `InstallDirectory`,
//! `ProductTitle`, `Installed`). The database is read from a copy, so the
//! app's files are never written, locked or moved.
//!
//! Each install of a game in the library and not yet installed with nile
//! goes through `nile import` ([`crate::ops::import`]), which checks the
//! files against the game's manifest and records the game in nile's
//! `installed.json`. The games brought in are remembered in
//! `nile_store/amazon_app.json` (`{"imported": {"<id>": "<folder>"}}`): their
//! files are the app's, so uninstalling one only takes it off the list and
//! moving one is refused ([`is_from_app`]).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use heronite_data::json_store;
use heronite_data::DataPaths;
use heronite_i18n::tr;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};

use crate::cmd::Nile;
use crate::{installed, library, ops};

const STORE: &str = "amazon_app";

/// One game the app has installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppGame {
    /// Product id (the `app_name` of the library).
    pub id: String,
    pub title: String,
    pub folder: String,
}

/// The app's database of installs on this PC (`None` where the app does not
/// exist or has none).
pub fn database() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(database_in(Path::new(&local))).filter(|db| db.is_file())
}

/// The database under a `%LOCALAPPDATA%` folder.
pub fn database_in(local_app_data: &Path) -> PathBuf {
    local_app_data.join("Amazon Games").join("Data").join("Games").join("Sql").join("GameInstallInfo.sqlite")
}

/// The installs the database lists as installed, read from a copy of it.
pub fn read_installs(db: &Path) -> Result<Vec<AppGame>> {
    let copy = Copy::of(db)?;
    let conn = Connection::open_with_flags(&copy.db, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .with_context(|| format!("opening {}", db.display()))?;
    let mut stmt = conn.prepare("SELECT * FROM DbSet").with_context(|| format!("reading {}", db.display()))?;
    let names: Vec<String> = stmt.column_names().into_iter().map(str::to_lowercase).collect();
    let col = |name: &str| names.iter().position(|n| n == name);
    let (Some(id), Some(folder)) = (col("id"), col("installdirectory")) else {
        anyhow::bail!("{}: the table of installs has no Id or InstallDirectory", db.display());
    };
    let (title, flag) = (col("producttitle"), col("installed"));
    let text = |row: &rusqlite::Row, i: usize| row.get::<_, Option<String>>(i).ok().flatten().unwrap_or_default().trim().to_string();
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        // A row without the flag counts as installed (older databases).
        let is_installed = flag.is_none_or(|i| row.get::<_, Option<i64>>(i).ok().flatten().unwrap_or(0) != 0);
        let game = AppGame {
            id: text(row, id),
            title: title.map(|i| text(row, i)).unwrap_or_default(),
            folder: text(row, folder).trim_end_matches(['/', '\\']).to_string(),
        };
        if is_installed && !game.id.is_empty() && !game.folder.is_empty() {
            out.push(game);
        }
    }
    Ok(out)
}

/// A private copy of the database (and its journal) in a temp folder,
/// removed when dropped.
struct Copy {
    dir: PathBuf,
    db: PathBuf,
}

impl Copy {
    fn of(db: &Path) -> Result<Self> {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("heronite-amazon-app-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let copy = Self { db: dir.join("installs.sqlite"), dir };
        std::fs::copy(db, &copy.db).with_context(|| format!("copying {}", db.display()))?;
        // Changes not yet written into the database itself.
        for ext in ["-wal", "-journal"] {
            let side = PathBuf::from(format!("{}{ext}", db.display()));
            if side.is_file() {
                std::fs::copy(&side, format!("{}{ext}", copy.db.display())).with_context(|| format!("copying {}", side.display()))?;
            }
        }
        Ok(copy)
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// What [`import_all`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Titles of the games now installed with nile.
    pub imported: Vec<String>,
    /// Title and nile's reason, for the games it would not take.
    pub failed: Vec<(String, String)>,
    /// Installs left aside: already installed with nile, not in the
    /// library, or their folder is gone.
    pub skipped: usize,
}

fn same_folder(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        let s = s.trim().trim_end_matches(['/', '\\']);
        if cfg!(windows) {
            s.replace('/', "\\").to_lowercase()
        } else {
            s.to_string()
        }
    };
    norm(a) == norm(b)
}

/// The installs of `db` that `nile import` can take: in the library, not
/// installed with nile yet, with their folder (and its `fuel.json`) there.
pub fn importable(paths: &DataPaths, db: &Path) -> Result<(Vec<AppGame>, usize)> {
    let all = read_installs(db)?;
    let total = all.len();
    let installed = installed::load(paths);
    let games: Vec<AppGame> = all
        .into_iter()
        .filter(|g| !installed.iter().any(|i| i.id == g.id))
        .filter(|g| library::game_info(paths, &g.id).is_some())
        .filter(|g| Path::new(&g.folder).join("fuel.json").is_file())
        .collect();
    let skipped = total - games.len();
    Ok((games, skipped))
}

/// Imports every game of [`importable`]. Blocking: nile checks every file of
/// each game, which takes a while for big games. `cancel` is asked between
/// games.
pub fn import_all(paths: &DataPaths, nile: &Nile, db: &Path, cancel: &dyn Fn() -> bool) -> Result<Report> {
    let (games, skipped) = importable(paths, db)?;
    let mut report = Report { skipped, ..Default::default() };
    for game in games {
        if cancel() {
            break;
        }
        let title = if game.title.is_empty() { game.id.clone() } else { game.title.clone() };
        log::info!("importing {} from the Amazon Games app ({})", game.id, game.folder);
        match ops::import(paths, nile, &game.id, &game.folder) {
            Ok(()) => {
                if let Err(err) = remember(paths, &game.id, &game.folder) {
                    log::warn!("remembering {} as the Amazon Games app's: {err:#}", game.id);
                }
                // The app ran the game's installers when it installed it.
                if let Err(err) = heronite_launch::native_setup::record(paths, "nile", &game.id, &game.folder) {
                    log::warn!("recording the setup of {}: {err:#}", game.id);
                }
                report.imported.push(title);
            }
            Err(err) => report.failed.push((title, format!("{err:#}"))),
        }
    }
    Ok(report)
}

fn remember(paths: &DataPaths, app_name: &str, folder: &str) -> Result<()> {
    json_store::update_value(&paths.nile_store(STORE), |doc| {
        let obj = doc.as_object_mut().expect("object");
        let imported = obj.entry("imported").or_insert_with(|| json!({}));
        if !imported.is_object() {
            *imported = json!({});
        }
        imported.as_object_mut().expect("object").insert(app_name.into(), Value::String(folder.into()));
        Ok(())
    })
}

/// The folder the game was imported from, when it came from the app.
fn imported_folder(paths: &DataPaths, app_name: &str) -> Option<String> {
    let file = paths.nile_store(STORE);
    if !file.exists() {
        return None;
    }
    let doc = json_store::read_value(&file).ok()?;
    doc.pointer(&format!("/imported/{}", app_name.replace('~', "~0").replace('/', "~1"))).and_then(Value::as_str).map(str::to_string)
}

/// Whether the game's files are the Amazon Games app's: it was brought in
/// from the app and nile still has it in that folder.
pub fn is_from_app(paths: &DataPaths, app_name: &str) -> bool {
    let Some(folder) = imported_folder(paths, app_name) else { return false };
    installed::get(paths, app_name).is_some_and(|g| same_folder(&g.path, &folder))
}

/// Forgets that the game came from the app (it is no longer installed with
/// nile).
pub(crate) fn forget(paths: &DataPaths, app_name: &str) {
    if imported_folder(paths, app_name).is_none() {
        return;
    }
    let res = json_store::update_value(&paths.nile_store(STORE), |doc| {
        if let Some(imported) = doc.get_mut("imported").and_then(Value::as_object_mut) {
            imported.shift_remove(app_name);
        }
        Ok(())
    });
    if let Err(err) = res {
        log::warn!("{}: {err:#}", paths.nile_store(STORE).display());
    }
}

/// What the user is told after an uninstall that left the app's files alone.
pub(crate) fn kept_files_note(folder: &str) -> String {
    tr!("The game's files belong to the Amazon Games app and were left in {folder}. Uninstall it there to delete them.", folder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::testing::{Env, Reply};

    const LIBRARY: &str = r#"[
        {"id": "e1", "product": {"id": "amzn1.adg.product.AAA", "title": "Cool Game"}},
        {"id": "e2", "product": {"id": "amzn1.adg.product.BBB", "title": "Other Game"}},
        {"id": "e3", "product": {"id": "amzn1.adg.product.CCC", "title": "Third"}}
    ]"#;

    /// A database like the app's, in `<root>/app`.
    fn app_database(env: &Env, rows: &[(&str, &str, &str, i64)]) -> PathBuf {
        let db = database_in(&env.root().join("app"));
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE DbSet (Id TEXT PRIMARY KEY, InstallDirectory TEXT, Installed INTEGER, ProductTitle TEXT, \
             ProductAsin TEXT, ProductSku TEXT, InstallDate TEXT, ManifestSignature TEXT);",
        )
        .unwrap();
        for (id, folder, title, installed) in rows {
            conn.execute(
                "INSERT INTO DbSet (Id, InstallDirectory, Installed, ProductTitle, ProductAsin) VALUES (?1, ?2, ?3, ?4, 'B0')",
                rusqlite::params![id, folder, installed, title],
            )
            .unwrap();
        }
        db
    }

    /// A game folder as the app leaves it.
    fn game_folder(env: &Env, name: &str) -> String {
        let dir = env.root().join("Amazon Games").join("Library").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fuel.json"), r#"{"Main": {"Command": "Game.exe"}}"#).unwrap();
        std::fs::write(dir.join("Game.exe"), "exe").unwrap();
        dir.to_string_lossy().into_owned()
    }

    fn bytes_of(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| (e.path(), std::fs::read(e.path()).unwrap())).collect();
        out.sort();
        out
    }

    #[test]
    fn the_database_is_where_the_app_keeps_it() {
        let db = database_in(Path::new(r"C:\Users\ana\AppData\Local"));
        assert!(db.ends_with(Path::new("Amazon Games/Data/Games/Sql/GameInstallInfo.sqlite")), "{db:?}");
        if !cfg!(windows) {
            assert_eq!(database(), None);
        }
    }

    #[test]
    fn installs_are_read_without_touching_the_database() {
        let env = Env::new();
        let db = app_database(
            &env,
            &[
                ("amzn1.adg.product.AAA", r"C:\Amazon Games\Library\Cool Game\", "Cool Game", 1),
                ("amzn1.adg.product.BBB", r"D:\Old", "Other Game", 0),
                ("", r"C:\x", "No id", 1),
            ],
        );
        let before = bytes_of(db.parent().unwrap());
        let games = read_installs(&db).unwrap();
        assert_eq!(
            games,
            [AppGame {
                id: "amzn1.adg.product.AAA".into(),
                title: "Cool Game".into(),
                folder: r"C:\Amazon Games\Library\Cool Game".into()
            }]
        );
        assert_eq!(bytes_of(db.parent().unwrap()), before, "the app's files are left as they were");
        assert!(read_installs(&env.root().join("missing.sqlite")).is_err());
    }

    #[test]
    fn owned_games_are_imported_with_nile_and_remembered_as_the_apps() {
        let env = Env::new();
        env.write_nile_file("library.json", LIBRARY);
        env.write_nile_file("installed.json", r#"[{"id": "amzn1.adg.product.CCC", "version": "1", "path": "/g/Third"}]"#);
        let (cool, other) = (game_folder(&env, "Cool Game"), game_folder(&env, "Other Game"));
        let gone = env.root().join("gone").to_string_lossy().into_owned();
        let db = app_database(
            &env,
            &[
                ("amzn1.adg.product.AAA", &cool, "Cool Game", 1),
                ("amzn1.adg.product.BBB", &other, "Other Game", 1),
                // Installed with nile already; not in the library; folder gone.
                ("amzn1.adg.product.CCC", &game_folder(&env, "Third"), "Third", 1),
                ("amzn1.adg.product.ZZZ", &game_folder(&env, "Not Owned"), "Not Owned", 1),
                ("amzn1.adg.product.BBB2", &gone, "Gone", 1),
            ],
        );
        let entry = |id: &str, path: &str| json!({"id": id, "version": "v1", "path": path});
        let written = json!([entry("amzn1.adg.product.CCC", "/g/Third"), entry("amzn1.adg.product.AAA", &cool)]).to_string();
        let tool = env
            .fake_tool()
            .on(&["import", "amzn1.adg.product.AAA"], Reply::new().write_file("${NILE_CONFIG_PATH}/nile/installed.json", &written))
            .on(&["import", "amzn1.adg.product.BBB"], Reply::new().err("ERROR [IMPORT]:\t There are missing files. Failed import."));
        let report = import_all(&env.paths, &env.nile_of(&tool), &db, &|| false).unwrap();
        assert_eq!(report.imported, ["Cool Game"]);
        assert_eq!(report.failed, [("Other Game".to_string(), "There are missing files. Failed import.".to_string())]);
        assert_eq!(report.skipped, 3);
        assert_eq!(
            tool.calls(),
            [format!("import --path {cool} amzn1.adg.product.AAA"), format!("import --path {other} amzn1.adg.product.BBB")]
        );
        assert_eq!(library::game_info(&env.paths, "amzn1.adg.product.AAA").unwrap()["is_installed"], true);
        assert!(is_from_app(&env.paths, "amzn1.adg.product.AAA"));
        assert!(!is_from_app(&env.paths, "amzn1.adg.product.BBB"));
        assert!(!is_from_app(&env.paths, "amzn1.adg.product.CCC"));
        // Nothing left to import the next time.
        let again = import_all(&env.paths, &env.nile_of(&tool), &db, &|| false).unwrap();
        assert_eq!(again.imported.len() + again.failed.len(), 1, "only the failed one is tried again");
    }

    #[test]
    fn uninstalling_a_game_of_the_app_keeps_its_files() {
        let env = Env::new();
        env.write_nile_file("library.json", LIBRARY);
        let cool = game_folder(&env, "Cool Game");
        env.write_nile_file("installed.json", &json!([{"id": "amzn1.adg.product.AAA", "version": "1", "path": cool}]).to_string());
        remember(&env.paths, "amzn1.adg.product.AAA", &cool).unwrap();
        let tool = env.fake_tool().otherwise(Reply::new().err("nile must not be asked to delete the files").exit(1));
        let note = ops::uninstall(&env.paths, &env.nile_of(&tool), "amzn1.adg.product.AAA").unwrap().expect("the user is told");
        assert!(note.contains("Amazon Games app") && note.contains(&cool), "{note}");
        assert!(tool.calls().is_empty(), "{:?}", tool.calls());
        assert!(Path::new(&cool).join("Game.exe").is_file());
        assert!(installed::get(&env.paths, "amzn1.adg.product.AAA").is_none());
        assert!(!is_from_app(&env.paths, "amzn1.adg.product.AAA"));
        assert!(imported_folder(&env.paths, "amzn1.adg.product.AAA").is_none());
    }

    #[test]
    fn a_game_of_the_app_is_not_moved() {
        let env = Env::new();
        env.write_nile_file("library.json", LIBRARY);
        let cool = game_folder(&env, "Cool Game");
        env.write_nile_file("installed.json", &json!([{"id": "amzn1.adg.product.AAA", "version": "1", "path": cool}]).to_string());
        remember(&env.paths, "amzn1.adg.product.AAA", &cool).unwrap();
        let to = env.root().join("elsewhere").to_string_lossy().into_owned();
        let err = ops::move_install(&env.paths, "amzn1.adg.product.AAA", &to, &crate::cmd::Cancel::new(), &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("Amazon Games app"), "{err}");
        assert!(Path::new(&cool).join("Game.exe").is_file());
    }
}
