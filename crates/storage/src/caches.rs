//! Caches that rebuild themselves: this app's resized and downloaded images, store info and
//! install script downloads, winetricks' downloads, the GPU drivers' compiled
//! shaders and Steam's shader caches of the installed games. Only what
//! exists is listed.

use std::path::PathBuf;

use heronite_data::maintenance;
use heronite_i18n::tr;

use crate::{CacheKind, Ctx, Group, Item, Kind, State};

pub(crate) fn items(ctx: &Ctx, steam_shaders: Vec<PathBuf>) -> Vec<Item> {
    let cache = ctx.home.join(".cache");
    let thumbs = ctx.app.thumbs_dir();
    let store_info: Vec<PathBuf> =
        maintenance::cache_targets(&ctx.data_paths, &ctx.app).into_iter().filter(|p| *p != thumbs && p.exists()).collect();
    let gpu: Vec<PathBuf> = ["mesa_shader_cache", "mesa_shader_cache_db", "radv_builtin_shaders", "nvidia/GLCache", "nvidia/ComputeCache"]
        .iter()
        .map(|d| cache.join(d))
        .filter(|p| p.is_dir())
        .collect();
    let all: [(CacheKind, String, Vec<PathBuf>); 7] = [
        (CacheKind::Images, tr!("Image previews").into(), vec![thumbs.clone()]),
        (CacheKind::Downloads, tr!("Downloaded artwork").into(), vec![ctx.data_paths.images_cache_dir()]),
        (CacheKind::StoreInfo, tr!("Store game info").into(), store_info),
        (CacheKind::InstallScripts, tr!("Install script downloads").into(), vec![ctx.app.cache.join("install-scripts")]),
        (CacheKind::Winetricks, tr!("Winetricks downloads").into(), vec![cache.join("winetricks")]),
        (CacheKind::GpuShaders, tr!("Graphics driver shaders").into(), gpu),
        (CacheKind::SteamShaders, tr!("Steam shader caches").into(), steam_shaders),
    ];
    all.into_iter()
        .filter_map(|(kind, title, paths)| {
            let paths: Vec<PathBuf> = paths.into_iter().filter(|p| p.exists()).collect();
            let first = paths.first()?.clone();
            // The folder to open: one cache folder, else their parent.
            let folder = if paths.len() == 1 && first.is_dir() { first.clone() } else { first.parent()?.to_path_buf() };
            let mut item = Item::new(Group::Caches, Kind::Cache(kind), State::Cache, title, first);
            item.id = format!("cache:{kind:?}");
            item.folder = folder;
            item.paths = paths;
            Some(item)
        })
        .collect()
}
