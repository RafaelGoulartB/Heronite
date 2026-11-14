//! Checks against GOG itself, with a real sign-in. They only run when asked:
//! `HERONITE_LIVE_DATA_DIR=<data folder> cargo test -p heronite-gog-api --
//! --ignored` (`HERONITE_LIVE_GOG_GAME` picks the game, an owned one). A
//! refreshed sign-in is written back to that folder's `auth.json`, as the app
//! does. Nothing is installed: chunks and files are only read into memory.

use std::path::PathBuf;

use heronite_gog_api::files::Source;
use heronite_gog_api::{auth, chunk, linux, plan, Client};

fn data_dir() -> Option<PathBuf> {
    std::env::var_os("HERONITE_LIVE_DATA_DIR").map(PathBuf::from)
}

fn game() -> String {
    std::env::var("HERONITE_LIVE_GOG_GAME").unwrap_or_else(|_| "1207658754".into())
}

fn client() -> Option<(Client, PathBuf)> {
    let dir = data_dir()?;
    let creds = auth::credentials(&dir.join("gog_store/auth.json")).expect("signed in to GOG");
    Some((Client::with_token(&creds.access_token), dir))
}

#[test]
#[ignore]
fn owned_games_and_install_info() {
    let Some((client, _)) = client() else { return };
    let owned = client.owned().unwrap();
    assert!(owned.contains(&game()), "{} is not owned", game());
    let info = heronite_gog_api::info::fetch(&client, &game(), "windows").unwrap();
    let mut shown = info.clone();
    shown.as_object_mut().unwrap().remove("builds");
    println!("windows info: {shown}");
    assert!(info["size"]["*"]["download_size"].as_u64().unwrap() > 0);
}

#[test]
#[ignore]
fn a_build_downloads_and_decodes() {
    let Some((client, dir)) = client() else { return };
    // `HERONITE_LIVE_GOG_BUILD` picks an older build (a v1 one, say).
    let build = std::env::var("HERONITE_LIVE_GOG_BUILD").ok();
    let request = plan::Request {
        game_id: &game(),
        os: "windows",
        language: "en-US",
        dlcs: None,
        branch: None,
        build_id: build.as_deref(),
        password: None,
        game_dir: None,
    };
    let prepared = plan::prepare(&client, &dir.join("gogdlConfig"), &request, plan::Mode::Install).unwrap();
    let (download, disk) = prepared.diff.sizes();
    println!(
        "generation {} build {} ({}): {} files, {download} bytes to download, {disk} on disk, folder {:?}, links {:?}",
        prepared.generation,
        prepared.build_id,
        prepared.version_name,
        prepared.diff.files.len(),
        prepared.folder_name,
        prepared.links.keys().collect::<Vec<_>>()
    );
    assert!(!prepared.diff.files.is_empty());
    let mut checked = 0;
    for f in prepared.diff.files.iter().filter(|f| f.size > 0 && f.size < 4 * 1024 * 1024).take(3) {
        match &f.source {
            Source::Chunks { product, parts } => {
                let link = &prepared.links[product];
                for p in parts {
                    let data = client.fetch_chunk(link, &p.chunk).unwrap();
                    assert_eq!(data.len() as u64, p.chunk.size);
                }
            }
            Source::Range { product, offset } => {
                let data = client.fetch_range(&prepared.links[product], *offset, f.size).unwrap();
                use md5::Digest;
                if let Some(heronite_gog_api::files::Checksum::Md5(want)) = &f.checksum {
                    assert_eq!(&hex::encode(md5::Md5::digest(&data)), want, "{}", f.path);
                }
            }
            _ => continue,
        }
        checked += 1;
        println!("fetched {} ({} bytes)", f.path, f.size);
    }
    assert!(checked > 0);
    // The links are only needed while they are fresh.
    assert!(prepared.links.values().all(|l: &chunk::SecureLink| !l.is_stale()));
}

#[test]
#[ignore]
fn a_linux_installer_is_read_without_downloading_it() {
    let Some((client, _)) = client() else { return };
    let product = client.product_downloads(&game()).unwrap();
    let installers = linux::linux_installers(&product);
    let Some(chosen) = linux::pick_installer(&installers, "en-US") else {
        println!("{} has no Linux installer", game());
        return;
    };
    let inst = client.open_installer(chosen, &game()).unwrap();
    let (download, disk) = inst.sizes();
    println!(
        "installer of {} bytes, archive at {}, {} entries, {download} / {disk} bytes",
        inst.size,
        inst.archive_start,
        inst.entries.len()
    );
    let gameinfo = inst.entries.iter().find(|e| e.name == "data/noarch/gameinfo").expect("gameinfo");
    let text = String::from_utf8(client.read_entry(&inst, gameinfo).unwrap()).unwrap();
    let info = linux::parse_gameinfo(&text);
    println!("gameinfo: {info:?}");
    assert!(!info.title.is_empty());
    // A bigger, deflated file streams through and matches its crc32.
    let big = inst
        .data_entries()
        .filter(|e| e.method == 8 && e.size > 1 << 20 && e.size < 16 << 20)
        .max_by_key(|e| e.size)
        .expect("a deflated file");
    let mut sink = std::io::sink();
    let mut read = 0;
    let written = linux::copy_entry(&client, &inst, big, &mut sink, &mut |n| read += n).unwrap();
    assert_eq!((written, read), (big.size, big.compressed_size));
    println!("streamed {} ({} -> {} bytes)", big.name, big.compressed_size, big.size);
    let info_doc = heronite_gog_api::info::fetch(&client, &game(), "linux").unwrap();
    println!("linux info: {info_doc}");
}

#[test]
#[ignore]
fn redistributables_and_cloud_saves_are_listed() {
    let Some((client, dir)) = client() else { return };
    let repo = client.repository().unwrap();
    let ids: Vec<String> = ["ISI", "MSVC2017"].iter().map(|s| s.to_string()).collect();
    let empty = tempfile::tempdir().unwrap();
    let (diff, installed) = heronite_gog_api::redist::shared_plan(&client, &repo, empty.path(), &ids).unwrap();
    println!("redist {installed:?}: {} files, {:?} bytes", diff.files.len(), diff.sizes());
    assert!(!diff.files.is_empty());
    let link = client.dependency_link().unwrap();
    let small = diff.files.iter().filter_map(|f| match &f.source {
        Source::Chunks { parts, .. } if f.size < 2 << 20 => parts.first(),
        _ => None,
    });
    for p in small.take(2) {
        assert_eq!(client.fetch_chunk(&link, &p.chunk).unwrap().len() as u64, p.chunk.size);
    }
    // Cloud saves: only listed, never changed.
    match client.game_client(&game(), "windows") {
        Ok(owner) => {
            let creds = auth::game_credentials(&dir.join("gog_store/auth.json"), &owner.client_id, &owner.client_secret).unwrap();
            let storage = heronite_gog_api::saves::Storage::new(&creds, &owner, "__default");
            // A game without cloud saves is refused (HTTP 400).
            match storage.list() {
                Ok(files) => println!("cloud saves: {} files", files.len()),
                Err(err) => println!("cloud saves refused: {err:#}"),
            }
        }
        Err(err) => println!("{} has no cloud saves: {err:#}", game()),
    }
}

#[test]
#[ignore]
fn an_update_from_an_older_build_takes_only_what_changed() {
    let Some((client, _)) = client() else { return };
    let builds = client.builds(&game(), "windows", None).unwrap();
    let Some(older) = builds.items.iter().filter(|b| b.branch.is_none()).nth(1) else {
        println!("{} has a single build", game());
        return;
    };
    let config = tempfile::tempdir().unwrap();
    let base = plan::Request {
        game_id: &game(),
        os: "windows",
        language: "en-US",
        dlcs: None,
        branch: None,
        build_id: Some(&older.build_id),
        password: None,
        game_dir: None,
    };
    let old = plan::prepare(&client, config.path(), &base, plan::Mode::Install).unwrap();
    heronite_gog_api::stored::write(config.path(), &game(), old.stored.as_ref().unwrap()).unwrap();
    let latest = plan::Request { build_id: None, ..base };
    let update = plan::prepare(&client, config.path(), &latest, plan::Mode::Update).unwrap();
    let reused = update
        .diff
        .files
        .iter()
        .filter_map(|f| match &f.source {
            Source::Chunks { parts, .. } => Some(parts.iter().filter(|p| p.old_offset.is_some()).count()),
            _ => None,
        })
        .sum::<usize>();
    println!(
        "{} ({}) -> {}: {} of {} files to write, {} deleted, {reused} chunks reused, {:?} bytes",
        older.build_id,
        old.generation,
        update.build_id,
        update.diff.files.len(),
        old.diff.files.len(),
        update.diff.deleted.len(),
        update.diff.sizes()
    );
    assert!(update.diff.files.len() <= old.diff.files.len() + 50);
    let repair = plan::prepare(&client, config.path(), &base, plan::Mode::Repair).unwrap();
    assert_eq!(repair.diff.files.len(), old.diff.files.len());
}
