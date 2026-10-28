//! Installs a package file the way the app installs a package update
//! ([`Package::install_verified`]: `pkexec`, a root-owned copy checked
//! against the hash). `scripts/package-update-smoke.sh` runs it as root on
//! clean containers.
//! `install_package <deb|rpm|arch> <file> [--tamper]`; `--tamper` changes the
//! file after its hash was taken, like a process swapping it between the
//! signature check and the install. Prints `RESULT: Ok` or `RESULT: Err(..)`.
use std::io::Write;
use std::path::Path;

use heronite_update::Package;
use sha2::{Digest, Sha256};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(kind), Some(file)) = (args.get(1).and_then(|k| Package::parse(k)), args.get(2).map(Path::new)) else {
        eprintln!("usage: install_package <deb|rpm|arch> <file> [--tamper]");
        std::process::exit(2);
    };
    let bytes = std::fs::read(file).unwrap_or_else(|e| panic!("reading {}: {e}", file.display()));
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    if args.get(3).map(String::as_str) == Some("--tamper") {
        std::fs::OpenOptions::new().append(true).open(file).and_then(|mut f| f.write_all(b"tampered")).expect("changing the file");
    }
    match kind.install_verified(file, &sha256) {
        Ok(()) => println!("RESULT: Ok"),
        Err(e) => println!("RESULT: Err({e})"),
    }
}
