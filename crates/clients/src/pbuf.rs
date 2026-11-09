//! The little of protobuf the clients' files need: the fields of a message
//! by number, without a schema.

/// A field's value. Fixed-width fields (wire types 1 and 5) are skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Value<'a> {
    Varint(u64),
    /// A string, bytes or a nested message.
    Bytes(&'a [u8]),
}

impl<'a> Value<'a> {
    pub fn as_str(&self) -> Option<&'a str> {
        match self {
            Value::Bytes(b) => std::str::from_utf8(b).ok(),
            Value::Varint(_) => None,
        }
    }
}

/// The `(number, value)` fields of a message, in order. It stops at the
/// first byte that is not a field ([`Fields::is_malformed`]).
pub(crate) struct Fields<'a> {
    rest: &'a [u8],
    bad: bool,
}

impl<'a> Fields<'a> {
    pub fn new(message: &'a [u8]) -> Self {
        Self { rest: message, bad: false }
    }

    /// Whether the message ended in the middle of a field, or held bytes
    /// that are no field (a file that is not this message at all).
    pub fn is_malformed(&self) -> bool {
        self.bad
    }

    fn varint(&mut self) -> Option<u64> {
        let mut n: u64 = 0;
        for (i, b) in self.rest.iter().take(10).enumerate() {
            n |= u64::from(b & 0x7F) << (7 * i);
            if b & 0x80 == 0 {
                self.rest = &self.rest[i + 1..];
                return Some(n);
            }
        }
        None
    }

    fn take(&mut self, len: u64) -> Option<&'a [u8]> {
        let len = usize::try_from(len).ok().filter(|l| *l <= self.rest.len())?;
        let (head, tail) = self.rest.split_at(len);
        self.rest = tail;
        Some(head)
    }

    fn fail<T>(&mut self) -> Option<T> {
        self.bad = true;
        self.rest = &[];
        None
    }
}

impl<'a> Iterator for Fields<'a> {
    type Item = (u32, Value<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        while !self.rest.is_empty() {
            let Some(key) = self.varint() else { return self.fail() };
            let Some(number) = u32::try_from(key >> 3).ok().filter(|n| *n > 0) else { return self.fail() };
            let value = match key & 7 {
                0 => self.varint().map(Value::Varint),
                2 => self.varint().and_then(|len| self.take(len)).map(Value::Bytes),
                1 | 5 => {
                    if self.take(if key & 7 == 1 { 8 } else { 4 }).is_none() {
                        return self.fail();
                    }
                    continue;
                }
                _ => None,
            };
            return match value {
                Some(v) => Some((number, v)),
                None => self.fail(),
            };
        }
        None
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Writes messages for the tests of the files that hold them.
    #[derive(Default)]
    pub struct Writer(pub Vec<u8>);

    impl Writer {
        fn varint(&mut self, mut n: u64) {
            while n >= 0x80 {
                self.0.push((n as u8 & 0x7F) | 0x80);
                n >>= 7;
            }
            self.0.push(n as u8);
        }

        pub fn num(mut self, field: u32, n: u64) -> Self {
            self.varint(u64::from(field) << 3);
            self.varint(n);
            self
        }

        pub fn bytes(mut self, field: u32, b: &[u8]) -> Self {
            self.varint(u64::from(field) << 3 | 2);
            self.varint(b.len() as u64);
            self.0.extend_from_slice(b);
            self
        }

        pub fn text(self, field: u32, s: &str) -> Self {
            self.bytes(field, s.as_bytes())
        }

        pub fn fixed32(mut self, field: u32, n: u32) -> Self {
            self.varint(u64::from(field) << 3 | 5);
            self.0.extend_from_slice(&n.to_le_bytes());
            self
        }

        pub fn fixed64(mut self, field: u32, n: u64) -> Self {
            self.varint(u64::from(field) << 3 | 1);
            self.0.extend_from_slice(&n.to_le_bytes());
            self
        }
    }

    #[test]
    fn reads_varints_strings_and_skips_fixed_widths() {
        let msg = Writer::default().text(1, "wow").fixed32(2, 7).num(3, 300).fixed64(4, 9).bytes(5, &[1, 2]).0;
        let mut fields = Fields::new(&msg);
        let got: Vec<_> = fields.by_ref().collect();
        assert_eq!(got, [(1, Value::Bytes(b"wow")), (3, Value::Varint(300)), (5, Value::Bytes(&[1, 2]))],);
        assert!(!fields.is_malformed());
        assert_eq!(got[0].1.as_str(), Some("wow"));
        assert_eq!(got[1].1.as_str(), None);
    }

    #[test]
    fn stops_at_what_is_no_field() {
        // A length that runs past the end.
        let mut cut = Writer::default().text(1, "ok").0;
        cut.extend_from_slice(&[0x12, 0x09, b'x']);
        let mut fields = Fields::new(&cut);
        assert_eq!(fields.by_ref().count(), 1);
        assert!(fields.is_malformed());
        // Field zero and the group wire types are not fields.
        for bad in [&[0x00u8, 0x01][..], &[0x0B][..], &[0x80][..]] {
            let mut fields = Fields::new(bad);
            assert_eq!(fields.by_ref().count(), 0);
            assert!(fields.is_malformed(), "{bad:?}");
        }
        assert_eq!(Fields::new(&[]).count(), 0);
    }
}
