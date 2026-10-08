//! Manual check: a legendary child must get SIGTERM when the app dies.
//! `cargo run -p heronite-epic --example pdeathsig -- <fake-legendary-script>`
//! then exits abruptly while the script runs; the script should record the TERM.
use std::time::Duration;

use heronite_epic::cmd::{Cancel, Legendary};

fn main() {
    let script = std::env::args().nth(1).expect("path of the fake legendary script");
    let lg = Legendary::with_bin(script, "/tmp/pdeathsig-cfg");
    std::thread::spawn(move || {
        let _ = lg.run_streaming(&["install"], &[], &Cancel::new(), |_| {});
    });
    std::thread::sleep(Duration::from_millis(800));
    std::process::exit(0);
}
