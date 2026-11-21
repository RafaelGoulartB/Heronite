//! Little-endian reading and writing of Epic's binary formats (manifests,
//! chunks), with Unreal's `FString` encoding.

use anyhow::{bail, Result};

/// A cursor over a byte slice that fails instead of panicking at the end.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Moves to `pos` (clamped to the end).
    pub fn seek(&mut self, pos: usize) {
        self.pos = pos.min(self.data.len());
    }

    pub fn rest(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let Some(end) = self.pos.checked_add(n).filter(|e| *e <= self.data.len()) else {
            bail!("unexpected end of data at {} (wanted {n} bytes of {})", self.pos, self.data.len());
        };
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.bytes(N)?.try_into().expect("length checked"))
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.array()?))
    }

    pub fn guid(&mut self) -> Result<[u32; 4]> {
        Ok([self.u32()?, self.u32()?, self.u32()?, self.u32()?])
    }

    /// An `FString`: a signed length counting the terminator, negative for
    /// UTF-16 (in characters), positive for 8-bit text, 0 for empty.
    pub fn fstring(&mut self) -> Result<String> {
        let len = self.i32()?;
        if len == 0 {
            return Ok(String::new());
        }
        if len < 0 {
            let chars = len.unsigned_abs() as usize;
            let raw = self.bytes(chars * 2)?;
            let units: Vec<u16> = raw[..raw.len() - 2].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            return Ok(String::from_utf16_lossy(&units));
        }
        let raw = self.bytes(len as usize)?;
        let text = &raw[..raw.len() - 1];
        // Epic writes ASCII; anything else is read as Latin-1 so no byte is lost.
        Ok(match std::str::from_utf8(text) {
            Ok(s) => s.to_string(),
            Err(_) => text.iter().map(|b| *b as char).collect(),
        })
    }
}

/// Appends little-endian values to a buffer.
#[derive(Default)]
pub(crate) struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn bytes(&mut self, v: &[u8]) {
        self.buf.extend_from_slice(v);
    }

    pub fn guid(&mut self, g: &[u32; 4]) {
        for v in g {
            self.u32(*v);
        }
    }

    /// Overwrites the `u32` at `at` (a size placeholder).
    pub fn patch_u32(&mut self, at: usize, v: u32) {
        self.buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// An `FString`: ASCII when it can be, else UTF-16.
    pub fn fstring(&mut self, s: &str) {
        if s.is_empty() {
            self.i32(0);
        } else if s.is_ascii() {
            self.i32(s.len() as i32 + 1);
            self.bytes(s.as_bytes());
            self.u8(0);
        } else {
            let units: Vec<u16> = s.encode_utf16().collect();
            self.i32(-(units.len() as i32 + 1));
            for u in units {
                self.bytes(&u.to_le_bytes());
            }
            self.bytes(&[0, 0]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fstrings_round_trip_in_both_encodings() {
        let mut w = Writer::default();
        for s in ["", "Game.exe", "Spiel für Sie ✓"] {
            w.fstring(s);
        }
        let mut r = Reader::new(&w.buf);
        assert_eq!(r.fstring().unwrap(), "");
        assert_eq!(r.fstring().unwrap(), "Game.exe");
        assert_eq!(r.fstring().unwrap(), "Spiel für Sie ✓");
        assert!(r.rest().is_empty());
    }

    #[test]
    fn reading_past_the_end_fails() {
        let mut r = Reader::new(&[1, 2]);
        assert!(r.u32().is_err());
        let mut r = Reader::new(&[10, 0, 0, 0, b'a']);
        assert!(r.fstring().is_err());
    }
}
