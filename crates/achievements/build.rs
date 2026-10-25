//! Compiles the vendored rcheevos `rc_hash` (C) and our two wrappers.

fn main() {
    let root = "vendor/rcheevos";
    let sources = [
        "src/rc_compat.c",
        "src/rhash/aes.c",
        "src/rhash/cdreader.c",
        "src/rhash/hash.c",
        "src/rhash/hash_disc.c",
        "src/rhash/hash_encrypted.c",
        "src/rhash/hash_rom.c",
        "src/rhash/hash_zip.c",
        "src/rhash/md5.c",
    ];
    let mut build = cc::Build::new();
    // Third-party C: its warnings are not ours to fix.
    build.warnings(false).include(format!("{root}/include")).include(format!("{root}/src"));
    for s in sources {
        build.file(format!("{root}/{s}"));
    }
    build.file("native/heronite_rc.c");
    build.compile("heronite_rc");
    println!("cargo:rerun-if-changed={root}");
    println!("cargo:rerun-if-changed=native");
}
