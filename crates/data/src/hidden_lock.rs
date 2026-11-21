//! The PIN that guards "Show hidden games", kept in `AppPaths::hidden_lock()`.
//!
//! It is a privacy screen: hidden games stay out of Heronite's lists until the
//! PIN is typed. The games, their files and the data folder are not
//! encrypted. The PIN itself is never stored: the file holds a random salt and
//! a PBKDF2-HMAC-SHA256 hash of it, so reading the file does not give the PIN
//! away at a glance (a short PIN can still be guessed offline).

use std::time::Duration;

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};

use crate::json_store;
use crate::paths::AppPaths;

/// Digits a PIN may have.
pub const PIN_LEN: std::ops::RangeInclusive<usize> = 4..=8;

/// PBKDF2 rounds of a new PIN (~0.1 s on a laptop): slows guessing from the
/// file without making the dialog wait.
const ROUNDS: u32 = 200_000;
const SALT_LEN: usize = 16;
const HASH_LEN: usize = 32;

/// A saved PIN: its salt, rounds and hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    salt: Vec<u8>,
    rounds: u32,
    hash: Vec<u8>,
}

/// Whether `pin` can be a PIN: 4 to 8 digits, nothing else.
pub fn valid_pin(pin: &str) -> bool {
    PIN_LEN.contains(&pin.len()) && pin.bytes().all(|b| b.is_ascii_digit())
}

fn derive(pin: &str, salt: &[u8], rounds: u32) -> Vec<u8> {
    let mut out = vec![0u8; HASH_LEN];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(pin.as_bytes(), salt, rounds, &mut out);
    out
}

/// Compares without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Pin {
    /// Hashes a new PIN with a fresh salt. Slow (see [`ROUNDS`]): call it off
    /// the UI thread.
    pub fn new(pin: &str) -> Result<Self> {
        if !valid_pin(pin) {
            bail!("a PIN has 4 to 8 digits");
        }
        let mut salt = vec![0u8; SALT_LEN];
        getrandom::fill(&mut salt).map_err(|e| anyhow::anyhow!("no random bytes for the salt: {e}"))?;
        Ok(Self { hash: derive(pin, &salt, ROUNDS), salt, rounds: ROUNDS })
    }

    /// Whether `pin` is this PIN. Slow, like [`Pin::new`].
    pub fn matches(&self, pin: &str) -> bool {
        valid_pin(pin) && same(&derive(pin, &self.salt, self.rounds), &self.hash)
    }

    fn from_value(v: &Value) -> Option<Self> {
        let pin = v.get("pin")?;
        if pin.get("kdf")?.as_str()? != "pbkdf2-sha256" {
            return None;
        }
        let rounds = u32::try_from(pin.get("rounds")?.as_u64()?).ok().filter(|r| *r > 0)?;
        let salt = B64.decode(pin.get("salt")?.as_str()?).ok()?;
        let hash = B64.decode(pin.get("hash")?.as_str()?).ok().filter(|h| h.len() == HASH_LEN)?;
        Some(Self { salt, rounds, hash })
    }

    fn to_value(&self) -> Value {
        json!({ "pin": { "kdf": "pbkdf2-sha256", "rounds": self.rounds, "salt": B64.encode(&self.salt), "hash": B64.encode(&self.hash) } })
    }
}

/// The saved PIN; `None` when there is none (or the file cannot be read, so a
/// broken file never locks the user out).
pub fn load(paths: &AppPaths) -> Option<Pin> {
    let value = json_store::read_value(&paths.hidden_lock()).ok()?;
    Pin::from_value(&value)
}

/// Whether hidden games are behind a PIN.
pub fn is_set(paths: &AppPaths) -> bool {
    load(paths).is_some()
}

/// Saves `pin` (replacing any PIN) as its hash. Slow, like [`Pin::new`].
pub fn set(paths: &AppPaths, pin: &str) -> Result<()> {
    json_store::write(&paths.hidden_lock(), &Pin::new(pin)?.to_value()).context("saving the PIN")
}

/// Removes the PIN: hidden games show again with the switch alone.
pub fn clear(paths: &AppPaths) -> Result<()> {
    match std::fs::remove_file(paths.hidden_lock()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e).context("removing the PIN"),
        _ => Ok(()),
    }
}

/// Failed tries in a row before the PIN asks to wait.
const FREE_TRIES: u32 = 5;
/// The longest wait between two tries.
const MAX_WAIT: Duration = Duration::from_secs(300);

/// How long to wait before the next try after `failures` wrong PINs in a
/// row: nothing for the first 4, then 30 s, doubling up to 5 minutes.
pub fn wait_after(failures: u32) -> Duration {
    if failures < FREE_TRIES {
        return Duration::ZERO;
    }
    let doublings = (failures - FREE_TRIES).min(8);
    (Duration::from_secs(30) * 2u32.pow(doublings)).min(MAX_WAIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(dir: &tempfile::TempDir) -> AppPaths {
        AppPaths { config: dir.path().join("cfg"), cache: dir.path().join("cache") }
    }

    #[test]
    fn only_four_to_eight_digits_are_a_pin() {
        assert!(valid_pin("1234") && valid_pin("12345678"));
        assert!(!valid_pin("123") && !valid_pin("123456789"));
        assert!(!valid_pin("12a4") && !valid_pin(" 1234") && !valid_pin("１２３４"));
    }

    #[test]
    fn the_file_keeps_a_salted_hash_not_the_pin() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(&dir);
        assert!(!is_set(&p));
        set(&p, "2468").unwrap();
        let text = std::fs::read_to_string(p.hidden_lock()).unwrap();
        assert!(!text.contains("2468"));
        let pin = load(&p).unwrap();
        assert!(pin.matches("2468"));
        assert!(!pin.matches("2469") && !pin.matches("24680") && !pin.matches(""));
        // A new salt each time: the same PIN gives another hash.
        set(&p, "2468").unwrap();
        assert_ne!(load(&p).unwrap().hash, pin.hash);
        clear(&p).unwrap();
        assert!(!is_set(&p));
        clear(&p).unwrap();
    }

    #[test]
    fn a_broken_file_is_no_pin() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(&dir);
        std::fs::create_dir_all(&p.config).unwrap();
        for text in ["", "{", r#"{"pin":{}}"#, r#"{"pin":{"kdf":"md5","rounds":1,"salt":"","hash":""}}"#] {
            std::fs::write(p.hidden_lock(), text).unwrap();
            assert!(!is_set(&p), "{text}");
        }
        assert!(set(&p, "12").is_err());
    }

    #[test]
    fn waits_grow_after_five_wrong_pins() {
        assert_eq!(wait_after(0), Duration::ZERO);
        assert_eq!(wait_after(4), Duration::ZERO);
        assert_eq!(wait_after(5), Duration::from_secs(30));
        assert_eq!(wait_after(6), Duration::from_secs(60));
        assert_eq!(wait_after(8), Duration::from_secs(240));
        assert_eq!(wait_after(9), MAX_WAIT);
        assert_eq!(wait_after(u32::MAX), MAX_WAIT);
    }
}
