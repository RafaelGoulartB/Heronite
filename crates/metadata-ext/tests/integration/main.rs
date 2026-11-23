//! Every integration test of `heronite-metadata-ext` in one test binary (one
//! link instead of one per file), each file a module. `stuck.rs` keeps a
//! binary of its own: the threads it leaves running count against a cap of
//! the whole process.

#[path = "../support/mod.rs"]
mod support;

mod dlsite;
mod emulated;
mod examples;
mod gather;
mod hostile;
mod id_patterns;
mod install;
mod lists;
mod local_lists;
mod sandbox;
mod vndb;
