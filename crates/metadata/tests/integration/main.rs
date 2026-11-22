//! Every integration test of `heronite-metadata` in one test binary (one
//! link instead of one per file), each file a module run against a local
//! mock of the remote services (`common`).

#[path = "../common/mod.rs"]
mod common;

mod bulk_jobs;
mod cover_families;
mod launchbox;
mod libretro;
mod pipeline;
mod protondb;
mod review;
mod sources;
mod steam_link;
mod steam_tags;
mod store_media;
