//! `export.ts`: the `heroic-playnite-export` v1 file.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use heronite_data::models::*;
use heronite_data::{json_store, status, HeroicPaths, JsNum};

use crate::store_ops::sorted_newest_first;
use crate::types::*;

const EXPORT_COMMENT: &str = "Copy this JSON to Windows. A Playnite extension can merge playtime, completion status, and sessions using playniteId or steamAppId.";

/// `resolveExportPath`.
pub fn resolve_export_path(target: &Path) -> PathBuf {
    let stamp = Utc::now().format("%Y-%m-%d").to_string();
    let default_name = format!("heroic-playnite-export-{stamp}.json");
    if target.is_dir() {
        return target.join(default_name);
    }
    if target.to_string_lossy().to_lowercase().ends_with(".json") {
        return target.to_path_buf();
    }
    target.join(default_name)
}

/// `exportPlayniteLibrary` without the file write.
pub fn build_export(paths: &HeroicPaths) -> Result<PlayniteExportFile> {
    let statuses = status::load_statuses(paths)?;
    let meta_file: LocalMetaFile = json_store::read(&paths.local_meta())?;
    let sessions: SessionsFile = json_store::read(&paths.sessions())?;
    let timestamps: TimestampFile = json_store::read(&paths.timestamps())?;

    let games = meta_file
        .games
        .values()
        .map(|meta| {
            let played = timestamps.get(&meta.app_name);
            let minutes = played.map(|p| p.total_played.get()).unwrap_or(0.0);
            let st = statuses
                .iter()
                .find(|s| Some(s.id.as_str()) == meta.completion_status_id.as_deref())
                .or_else(|| statuses.iter().find(|s| s.slug == "not-played"))
                .or_else(|| statuses.first());
            let non_empty = |s: Option<&String>| s.filter(|v| !v.is_empty()).cloned();
            PlayniteExportGame {
                playnite_id: meta.playnite_id.clone(),
                heroic_app_name: meta.app_name.clone(),
                steam_app_id: meta.steam_app_id.clone(),
                store_game_id: meta.store_game_id.clone(),
                title: meta.title.clone(),
                source: meta.source.clone(),
                playtime_minutes: JsNum(minutes),
                playtime_seconds: JsNum(minutes * 60.0),
                first_played: non_empty(played.and_then(|p| p.first_played.as_ref())),
                last_played: non_empty(played.and_then(|p| p.last_played.as_ref())),
                completion_status: PlayniteExportStatus {
                    id: st.map(|s| s.id.clone()).unwrap_or_else(|| "not-played".into()),
                    name: st.map(|s| s.name.clone()).unwrap_or_else(|| "Not Played".into()),
                    slug: st.map(|s| s.slug.clone()).unwrap_or_else(|| "not-played".into()),
                    playnite_id: st.and_then(|s| s.playnite_id.clone()),
                },
                last_playnite_completion_status_id: meta.playnite_completion_status_id.clone(),
                sessions: sorted_newest_first(sessions.sessions.get(&meta.app_name).map(Vec::as_slice).unwrap_or(&[])),
            }
        })
        .collect();

    Ok(PlayniteExportFile {
        format: "heroic-playnite-export".into(),
        version: 1,
        exported_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        comment: EXPORT_COMMENT.into(),
        statuses,
        games,
    })
}

/// Writes the export (`JSON.stringify(payload, null, 2) + '\n'`).
pub fn export_playnite_library(paths: &HeroicPaths, target: &Path) -> Result<PlayniteExportResult> {
    let payload = build_export(paths)?;
    let file_path = resolve_export_path(target);
    if let Some(dir) = file_path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut bytes = serde_json::to_vec_pretty(&payload)?;
    bytes.push(b'\n');
    fs::write(&file_path, bytes).with_context(|| format!("writing {}", file_path.display()))?;
    log::info!("Exported {} games for Playnite to {}", payload.games.len(), file_path.display());
    Ok(PlayniteExportResult { path: file_path.to_string_lossy().into_owned(), game_count: payload.games.len() })
}
