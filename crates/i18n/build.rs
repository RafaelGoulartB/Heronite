//! Embeds every `locales/pt-BR/*.json` (one file per area of the app) so a new
//! file needs no code change.

use std::fmt::Write;
use std::path::Path;

fn main() {
    let dir = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("locales/pt-BR");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("locales/pt-BR")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    let mut out = String::from("pub(crate) const PT_BR: &[(&str, &str)] = &[\n");
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let name = f.file_name().unwrap().to_string_lossy();
        writeln!(out, "    ({name:?}, include_str!({:?})),", f.display().to_string()).unwrap();
    }
    out.push_str("];\n");
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("locales.rs");
    std::fs::write(dest, out).unwrap();
}
