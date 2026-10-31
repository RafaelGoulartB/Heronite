//! Plans and recording against a copy of real Heronite data.
//!
//! Point `HERONITE_FIXTURE` at a copy of the data folder (default
//! `/tmp/heronite-fixture`). Plan building only reads; the recording test
//! works on a temporary copy. Run with `-- --nocapture` to see the plans.

use std::path::{Path, PathBuf};

use heronite_data::models::{GameInfo, LocalGameMeta, LocalMetaFile, SideloadLibraryFile};
use heronite_data::{json_store, DataPaths};
use heronite_launch::library::{find_in_library, load_game};
use heronite_launch::record::{record_play, PlayRecord};
use heronite_launch::settings::GlobalSettings;
use heronite_launch::{build_plan, LaunchOptions, LaunchPlan, PlanContext, SystemProbe};

fn fixture() -> Option<PathBuf> {
    let root = std::env::var_os("HERONITE_FIXTURE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp/heronite-fixture"));
    if root.join("local_library/library.json").exists() {
        Some(root)
    } else {
        eprintln!("fixture {} not found; skipping", root.display());
        None
    }
}

/// The fixture has no `config.json` (Heroic's global settings); borrow the
/// real one, read-only, so Wine/Proton defaults resolve like on this machine.
fn context(root: &Path) -> PlanContext {
    let paths = DataPaths::at(root);
    let mut ctx = PlanContext::load(&paths);
    if ctx.global.defaults.is_empty() {
        ctx.global = GlobalSettings::load(&DataPaths::detect());
    }
    ctx
}

fn describe(plan: &LaunchPlan) -> String {
    match plan {
        LaunchPlan::SteamUri { uri, .. } => format!("steam      {uri}"),
        LaunchPlan::Unavailable(r) => format!("UNAVAILABLE {r}"),
        other => {
            let p = other.process().unwrap();
            let extra = match other {
                LaunchPlan::Wine { runtime, prefix, .. } => format!(" [{runtime:?}, prefix {prefix}]"),
                _ => String::new(),
            };
            format!("{:<10} {}{extra}", other.kind(), p.command.display())
        }
    }
}

struct Row {
    info: GameInfo,
    meta: Option<LocalGameMeta>,
    plan: LaunchPlan,
}

fn installed_plans(root: &Path) -> Vec<Row> {
    let ctx = context(root);
    let paths = DataPaths::at(root);
    let probe = SystemProbe;
    let metas: LocalMetaFile = json_store::read(&paths.local_meta()).unwrap();
    let sideload: SideloadLibraryFile = json_store::read(&paths.sideload_library()).unwrap();
    let mut rows = Vec::new();
    for info in sideload.games.into_iter().filter(|g| g.is_installed) {
        let meta = metas.games.get(&info.app_name).cloned();
        let plan = build_plan(&ctx, &probe, &info, meta.as_ref(), &LaunchOptions::default());
        rows.push(Row { info, meta, plan });
    }
    if let Some(info) = find_in_library(&paths, "legendary", "Calluna") {
        if info.is_installed {
            let plan = build_plan(&ctx, &probe, &info, None, &LaunchOptions::default());
            rows.push(Row { info, meta: None, plan });
        }
    }
    rows
}

#[test]
fn print_plans_for_installed_games() {
    let Some(root) = fixture() else { return };
    let rows = installed_plans(&root);
    assert!(!rows.is_empty());

    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for r in &rows {
        *counts.entry(r.plan.kind()).or_default() += 1;
    }
    println!("\n=== plans for {} installed games: {counts:?} ===", rows.len());
    // Steam games are many; show a few, and every other kind.
    let mut steam_shown = 0;
    for r in &rows {
        if matches!(r.plan, LaunchPlan::SteamUri { .. }) {
            steam_shown += 1;
            if steam_shown > 3 {
                continue;
            }
        }
        println!("{:<44} {:<32} {}", r.info.app_name, r.info.title.chars().take(30).collect::<String>(), describe(&r.plan));
        if let Some(p) = r.plan.process() {
            for n in &p.notes {
                println!("{:<44} note: {n}", "");
            }
            if !p.pre_commands.is_empty() {
                println!("{:<44} pre:  {}", "", p.pre_commands.iter().map(|c| c.display()).collect::<Vec<_>>().join(" && "));
            }
        }
    }
    println!();
}

#[test]
fn installed_games_get_sensible_plans() {
    let Some(root) = fixture() else { return };
    let rows = installed_plans(&root);
    let by = |name: &str| rows.iter().find(|r| r.info.app_name == name);

    // Every installed Steam game is a steam:// URI (or says Steam is missing).
    for r in rows.iter().filter(|r| r.meta.as_ref().map(|m| m.launch_kind == "steam-uri").unwrap_or(false)) {
        match &r.plan {
            LaunchPlan::SteamUri { app_id, uri, .. } => {
                assert_eq!(uri, &format!("steam://rungameid/{app_id}"))
            }
            LaunchPlan::Unavailable(reason) => assert!(reason.contains("Steam"), "{reason}"),
            other => panic!("{}: {other:?}", r.info.app_name),
        }
    }

    // PCSX2 AppImage games: emulator + the selected (or first) ROM.
    for r in rows.iter().filter(|r| r.meta.as_ref().map(|m| m.launch_kind == "emulator").unwrap_or(false)) {
        if !Path::new("/data/Games/Emulation/PS2").exists() {
            break;
        }
        let LaunchPlan::Emulator { process, rom, .. } = &r.plan else { panic!("{}: {:?}", r.info.app_name, r.plan) };
        assert!(process.command.program.ends_with("pcsx2-v2.8.2.AppImage"));
        assert_eq!(process.command.args.len(), 1);
        assert_eq!(Some(&process.command.args[0]), rom.as_ref());
        assert!(rom.as_ref().unwrap().ends_with(".iso"));
        assert_eq!(process.stop.kill_patterns, ["pcsx2-v2.8.2.AppImage"]);
    }

    // The Linux shell-script port runs natively.
    if let Some(r) = by("playnite_2fcb4999-cbe9-42f5-818c-df49003cfd5e") {
        if Path::new("/data/Games/Emulation/pc-ports/bb/Bloodborne-Ultrawide.sh").exists() {
            assert_eq!(r.plan.kind(), "native", "{:?}", r.plan);
        }
    }

    // Windows executables go through Wine/Proton, or say why they cannot.
    for name in ["playnite_424a7b2f-493b-4af6-b368-df784d909613", "playnite_96b47d77-e852-484b-998f-f6ded44b4872"] {
        if let Some(r) = by(name) {
            match &r.plan {
                LaunchPlan::Wine { process, prefix, .. } => {
                    assert!(process.command.args.iter().any(|a| a.ends_with(".exe")));
                    assert_eq!(prefix, "/home/rafa/Games/Heroic/Prefixes/shared");
                }
                LaunchPlan::Unavailable(reason) => assert!(reason.contains("Wine") || reason.contains("Executable"), "{reason}"),
                other => panic!("{name}: {other:?}"),
            }
        }
    }

    // Epic: legendary with the Heroic config dir.
    if let Some(r) = by("Calluna") {
        match &r.plan {
            LaunchPlan::Legendary { process, .. } => {
                assert_eq!(process.command.env["LEGENDARY_CONFIG_PATH"], root.join("legendaryConfig/legendary").to_string_lossy());
                assert_eq!(&process.command.args[..2], ["launch", "Calluna"]);
                assert!(process.command.args.contains(&"--no-wine".to_string()) || process.command.args.contains(&"--wine".to_string()));
                assert_eq!(process.command.env["WINEPREFIX"], "/home/rafa/Games/Heroic/Prefixes/Control");
            }
            LaunchPlan::Unavailable(reason) => println!("Calluna unavailable here: {reason}"),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn uninstalled_games_report_why() {
    let Some(root) = fixture() else { return };
    let ctx = context(&root);
    let paths = DataPaths::at(&root);
    // Playnite import without executable.
    let g = load_game(&paths, "playnite_febe19c4-f651-4f6a-8caa-e74151b2be8c").unwrap();
    let plan = build_plan(&ctx, &SystemProbe, &g.info, g.meta.as_ref(), &LaunchOptions::default());
    assert_eq!(plan, LaunchPlan::Unavailable("No executable is configured for this game".into()));
    // Playnite emulator import whose emulator was never configured.
    let g = load_game(&paths, "playnite_22400e79-95fd-4086-85ab-d3c54036af1c").unwrap();
    let plan = build_plan(&ctx, &SystemProbe, &g.info, g.meta.as_ref(), &LaunchOptions::default());
    assert!(plan.unavailable_reason().is_some(), "{plan:?}");
    // Epic games that are in the library but not installed.
    let epic: Vec<_> =
        serde_json::from_value::<Vec<GameInfo>>(json_store::read_value(&paths.legendary_library()).unwrap()["library"].clone())
            .unwrap()
            .into_iter()
            .filter(|g| !g.is_installed)
            .take(3)
            .collect();
    for g in epic {
        let plan = build_plan(&ctx, &SystemProbe, &g, None, &LaunchOptions::default());
        assert!(plan.unavailable_reason().unwrap().contains("not installed"), "{plan:?}");
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_type().unwrap().is_dir() {
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

#[test]
fn recording_into_a_copy_of_real_data_keeps_everything_else() {
    let Some(root) = fixture() else { return };
    let tmp = tempfile::tempdir().unwrap();
    for sub in ["store", "local_library"] {
        copy_dir(&root.join(sub), &tmp.path().join(sub));
    }
    let paths = DataPaths::at(tmp.path());
    let before_ts = json_store::read_value(&paths.timestamps()).unwrap();
    let before_sessions = json_store::read_value(&paths.sessions()).unwrap();
    let before_cfg = json_store::read_value(&paths.store_config()).unwrap();

    let start = chrono::Utc::now() - chrono::Duration::minutes(75);
    let end = start + chrono::Duration::minutes(75) - chrono::Duration::milliseconds(1);
    // steam_400 (Portal) has history in the fixture.
    let rec = PlayRecord { app_name: "steam_400".into(), title: "Portal".into(), started_at: start, ended_at: end };
    let out = record_play(&paths, &rec, 5).unwrap();
    assert!(out.session_recorded);

    let ts = json_store::read_value(&paths.timestamps()).unwrap();
    let old_total = before_ts["steam_400"]["totalPlayed"].as_i64().unwrap();
    assert_eq!(ts["steam_400"]["totalPlayed"].as_i64().unwrap(), old_total + 74);
    assert_eq!(ts["steam_400"]["firstPlayed"], before_ts["steam_400"]["firstPlayed"]);
    for (k, v) in before_ts.as_object().unwrap() {
        if k != "steam_400" {
            assert_eq!(&ts[k], v, "{k} must be untouched");
        }
    }

    let sessions = json_store::read_value(&paths.sessions()).unwrap();
    let old_len = before_sessions["sessions"]["steam_400"].as_array().map(|a| a.len()).unwrap_or(0);
    let new = sessions["sessions"]["steam_400"].as_array().unwrap();
    assert_eq!(new.len(), (old_len + 1).min(200));
    for (k, v) in before_sessions["sessions"].as_object().unwrap() {
        if k != "steam_400" {
            assert_eq!(&sessions["sessions"][k], v);
        }
    }

    let cfg = json_store::read_value(&paths.store_config()).unwrap();
    assert_eq!(cfg["games"]["recent"][0], serde_json::json!({"appName": "steam_400", "title": "Portal"}));
    let mut expected = before_cfg.clone();
    expected["games"]["recent"] = cfg["games"]["recent"].clone();
    assert_eq!(cfg, expected, "only games.recent changed in store/config.json");
    let before_recent: Vec<_> =
        before_cfg["games"]["recent"].as_array().unwrap().iter().filter(|g| g["appName"] != "steam_400").cloned().collect();
    assert_eq!(&cfg["games"]["recent"].as_array().unwrap()[1..], &before_recent[..]);
}
