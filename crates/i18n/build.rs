//! Embeds every `locales/<lang>/*.json` (one folder per language, one file per
//! area of the app) so a new file needs no code change. `locales/pt-BR`
//! becomes `PT_BR`, `locales/es` becomes `ES`.

use std::fmt::Write;
use std::path::Path;

fn main() {
    let root = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("locales");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut langs: Vec<_> = std::fs::read_dir(&root).expect("locales").filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
    langs.sort();
    let mut out = String::new();
    for dir in &langs {
        println!("cargo:rerun-if-changed={}", dir.display());
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        files.sort();
        let name = dir.file_name().unwrap().to_string_lossy().to_ascii_uppercase().replace('-', "_");
        writeln!(out, "pub(crate) const {name}: &[(&str, &str)] = &[").unwrap();
        for f in &files {
            println!("cargo:rerun-if-changed={}", f.display());
            let file = f.file_name().unwrap().to_string_lossy();
            writeln!(out, "    ({file:?}, include_str!({:?})),", f.display().to_string()).unwrap();
        }
        out.push_str("];\n");
    }
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("locales.rs");
    std::fs::write(dest, out).unwrap();
}
