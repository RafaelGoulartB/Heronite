//! Release versions (`v0.1.4`, `0.2.0-rc.1`): enough of semver to order the
//! tags of this app's releases.

use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// Text after `-` (`rc.1`); empty for a final release.
    pub pre: String,
}

impl Version {
    /// Parses `X.Y.Z[-pre][+build]`, with or without a leading `v`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
        let text = text.split('+').next().unwrap_or(text);
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) if !pre.is_empty() => (core, pre),
            Some(_) => return None,
            None => (text, ""),
        };
        let mut parts = core.split('.');
        let mut num = || parts.next().filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))?.parse::<u64>().ok();
        let (major, minor, patch) = (num()?, num()?, num()?);
        if parts.next().is_some() {
            return None;
        }
        Some(Self { major, minor, patch, pre: pre.to_owned() })
    }

    /// The version this binary was built as.
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).expect("the package version is semver")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre)?;
        }
        Ok(())
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)).then_with(|| cmp_pre(&self.pre, &other.pre))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Semver precedence of pre-release parts: a final release is above its
/// pre-releases; numeric identifiers compare as numbers and below text ones.
fn cmp_pre(a: &str, b: &str) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    let mut left = a.split('.');
    let mut right = b.split('.');
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Version;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn parses_tags() {
        assert_eq!(v("v0.1.4"), Version { major: 0, minor: 1, patch: 4, pre: String::new() });
        assert_eq!(v("1.2.3-rc.1+abc").pre, "rc.1");
        assert_eq!(v(" 10.0.0 ").to_string(), "10.0.0");
        for bad in ["", "v", "1.2", "1.2.3.4", "1.2.x", "1.2.3-", "a1.2.3", "1..3"] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn orders_like_semver() {
        let ordered = [
            "0.1.3",
            "0.1.4-alpha",
            "0.1.4-alpha.1",
            "0.1.4-alpha.beta",
            "0.1.4-beta.2",
            "0.1.4-beta.11",
            "0.1.4",
            "0.1.10",
            "0.2.0",
            "1.0.0",
        ];
        for pair in ordered.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
        }
        assert_eq!(v("v1.0.0"), v("1.0.0+build"));
    }
}
