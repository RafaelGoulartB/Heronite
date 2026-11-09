//! Valve's binary KeyValues, the format of the stats files in
//! `appcache/stats`: a key is a type byte and a NUL-terminated name, then
//! the value; `0x08` ends a group.

/// Deeper than any real file; stops garbage from overflowing the stack.
const MAX_DEPTH: usize = 32;

const GROUP: u8 = 0x00;
const STRING: u8 = 0x01;
const INT32: u8 = 0x02;
const FLOAT: u8 = 0x03;
const POINTER: u8 = 0x04;
const WIDE_STRING: u8 = 0x05;
const COLOR: u8 = 0x06;
const UINT64: u8 = 0x07;
const END: u8 = 0x08;
const INT64: u8 = 0x0A;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Children in file order; keys may repeat.
    Group(Vec<(String, Value)>),
    Str(String),
    /// Every integer type; a `uint64` above `i64::MAX` wraps.
    Int(i64),
    Float(f32),
}

impl Value {
    /// The value of the first child called `key` (ASCII case-insensitive).
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.children().iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }

    pub fn children(&self) -> &[(String, Value)] {
        match self {
            Value::Group(children) => children,
            _ => &[],
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Integers and floats (truncated).
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Float(f) => Some(*f as i64),
            _ => None,
        }
    }
}

/// The top level of a file, as a group.
pub fn parse(bytes: &[u8]) -> Result<Value, String> {
    let mut reader = Reader { bytes, at: 0 };
    let children = reader.group(0, true)?;
    Ok(Value::Group(children))
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).filter(|end| *end <= self.bytes.len()).ok_or_else(|| format!("cut short at byte {}", self.at))?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn cstring(&mut self) -> Result<String, String> {
        let rest = &self.bytes[self.at..];
        let len = rest.iter().position(|b| *b == 0).ok_or_else(|| format!("unterminated string at byte {}", self.at))?;
        self.at += len + 1;
        Ok(String::from_utf8_lossy(&rest[..len]).into_owned())
    }

    /// UTF-16 units up to a NUL unit.
    fn wide_string(&mut self) -> Result<String, String> {
        let mut units = Vec::new();
        loop {
            let pair = self.take(2)?;
            let unit = u16::from_le_bytes([pair[0], pair[1]]);
            if unit == 0 {
                return Ok(String::from_utf16_lossy(&units));
            }
            units.push(unit);
        }
    }

    /// The children up to `END`; the top level may also stop at the end of
    /// the input.
    fn group(&mut self, depth: usize, top: bool) -> Result<Vec<(String, Value)>, String> {
        if depth > MAX_DEPTH {
            return Err("nested too deep".into());
        }
        let mut out = Vec::new();
        loop {
            if top && self.at >= self.bytes.len() {
                return Ok(out);
            }
            let kind = self.take(1)?[0];
            if kind == END {
                return Ok(out);
            }
            let key = self.cstring()?;
            let value = match kind {
                GROUP => Value::Group(self.group(depth + 1, false)?),
                STRING => Value::Str(self.cstring()?),
                INT32 | POINTER | COLOR => Value::Int(i32::from_le_bytes(self.take(4)?.try_into().unwrap_or_default()) as i64),
                FLOAT => Value::Float(f32::from_le_bytes(self.take(4)?.try_into().unwrap_or_default())),
                UINT64 | INT64 => Value::Int(i64::from_le_bytes(self.take(8)?.try_into().unwrap_or_default())),
                WIDE_STRING => Value::Str(self.wide_string()?),
                other => return Err(format!("unknown type {other:#04x} at byte {}", self.at - 1)),
            };
            out.push((key, value));
        }
    }
}

/// Builders for the byte fixtures of the tests.
#[cfg(test)]
pub mod build {
    pub fn group(key: &str, children: &[Vec<u8>]) -> Vec<u8> {
        let mut out = vec![0x00];
        out.extend_from_slice(key.as_bytes());
        out.push(0);
        for child in children {
            out.extend_from_slice(child);
        }
        out.push(0x08);
        out
    }

    pub fn string(key: &str, value: &str) -> Vec<u8> {
        let mut out = vec![0x01];
        out.extend_from_slice(key.as_bytes());
        out.push(0);
        out.extend_from_slice(value.as_bytes());
        out.push(0);
        out
    }

    pub fn int(key: &str, value: i32) -> Vec<u8> {
        let mut out = vec![0x02];
        out.extend_from_slice(key.as_bytes());
        out.push(0);
        out.extend_from_slice(&value.to_le_bytes());
        out
    }

    /// A whole file: the root group and the end byte a file adds.
    pub fn file(root: Vec<u8>) -> Vec<u8> {
        let mut out = root;
        out.push(0x08);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    #[test]
    fn reads_nested_groups_and_every_scalar() {
        let mut bytes = group("root", &[string("name", "Portal"), int("negative", -5), group("inner", &[int("n", 7)])]);
        bytes.extend_from_slice(&[0x03, b'f', 0, 0, 0, 0xC0, 0x3F]);
        bytes.extend_from_slice(&[0x07, b'u', 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        bytes.extend_from_slice(&[0x05, b'w', 0, b'h', 0, b'i', 0, 0, 0]);
        let doc = parse(&file(bytes)).unwrap();
        let root = doc.get("root").unwrap();
        assert_eq!(root.get("NAME").and_then(Value::as_str), Some("Portal"));
        assert_eq!(root.get("negative").and_then(Value::as_i64), Some(-5));
        assert_eq!(root.get("inner").and_then(|g| g.get("n")).and_then(Value::as_i64), Some(7));
        assert_eq!(doc.get("f"), Some(&Value::Float(1.5)));
        assert_eq!(doc.get("u"), Some(&Value::Int(1)));
        assert_eq!(doc.get("w").and_then(Value::as_str), Some("hi"));
    }

    #[test]
    fn a_file_without_the_final_end_byte_still_reads() {
        let doc = parse(&group("a", &[int("x", 1)])).unwrap();
        assert!(doc.get("a").is_some());
    }

    #[test]
    fn broken_files_are_errors() {
        let good = file(group("a", &[string("s", "text"), int("x", 1)]));
        for cut in 1..good.len() - 2 {
            let _ = parse(&good[..cut]);
        }
        assert!(parse(&good[..good.len() - 5]).is_err());
        assert!(parse(&[0x42, b'k', 0, 0, 0, 0, 0]).is_err());
        let deep: Vec<u8> = (0..64).flat_map(|_| [0x00, b'k', 0]).collect();
        assert!(parse(&deep).is_err());
    }
}
