//! `cargo run -p heronite-playnite --example dump -- <library path>`: prints
//! the parsed library as JSON lines (read-only), handy to diff against the TS
//! reader.
use heronite_playnite::load_playnite_library;
use serde_json::json;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("library path");
    let dump = load_playnite_library(&path)?;
    for g in &dump.games {
        let actions: Vec<_> = g
            .game_actions
            .iter()
            .map(|a| json!([a.action_type, a.name, a.path, a.arguments, a.is_play_action, a.emulator_id, a.emulator_profile_id]))
            .collect();
        println!(
            "{}",
            json!({
                "id": g.id, "name": g.name, "gameId": g.game_id, "pluginId": g.plugin_id,
                "installDirectory": g.install_directory, "playtime": g.playtime_seconds, "playCount": g.play_count,
                "lastActivity": g.last_activity, "added": g.added, "releaseDate": g.release_date,
                "developers": g.developers, "publishers": g.publishers, "genres": g.genres, "tags": g.tags,
                "features": g.features, "platforms": g.platforms, "series": g.series,
                "notes": g.notes.as_ref().map(|s| s.encode_utf16().count()), "description": g.description.as_ref().map(|s| s.encode_utf16().count()),
                "cover": g.cover_image, "bg": g.background_image, "status": g.completion_status_id,
                "actions": actions, "roms": g.roms.iter().map(|r| &r.path).collect::<Vec<_>>(),
            })
        );
    }
    for s in &dump.completion_statuses {
        println!("{}", json!({"status": s.id, "name": s.name}));
    }
    for e in &dump.emulators {
        println!("{}", json!({"emulator": e.id, "name": e.name, "installDir": e.install_dir, "profiles": e.profiles.len()}));
    }
    Ok(())
}
