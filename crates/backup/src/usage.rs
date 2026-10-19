//! What each part of a backup would take, and what backups leave out, for
//! Settings → Backups.

use std::collections::BTreeMap;
use std::path::Path;

use heronite_data::{AppPaths, DataPaths};

use crate::catalog::{self, Class, Left, Part, Side};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    /// Files and bytes of each part as they are now.
    pub parts: BTreeMap<Part, (usize, u64)>,
    /// Bytes left out, by reason (the app's caches folder counts as images).
    pub left: BTreeMap<Left, u64>,
}

impl Usage {
    pub fn part_bytes(&self, part: Part) -> u64 {
        self.parts.get(&part).map(|p| p.1).unwrap_or(0)
    }
}

/// Walks the data folder, the app state and the app caches. Takes a moment
/// with Wine builds installed (tens of thousands of files).
pub fn measure(data: &DataPaths, app: &AppPaths) -> Usage {
    let mut usage = Usage::default();
    for (side, root) in [(Side::Data, &data.root), (Side::Config, &app.config)] {
        for found in catalog::walk(root, side, true) {
            match found.class {
                Class::In(part) => {
                    let e = usage.parts.entry(part).or_default();
                    e.0 += 1;
                    e.1 += found.size;
                }
                Class::Out(left) => *usage.left.entry(left).or_default() += found.size,
            }
        }
    }
    *usage.left.entry(Left::Images).or_default() += tree_bytes(&app.cache);
    usage
}

fn tree_bytes(dir: &Path) -> u64 {
    catalog::walk(dir, Side::Data, true).iter().map(|f| f.size).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_by_part_and_reason() {
        let d = tempfile::tempdir().unwrap();
        let (data, app) = crate::snapshot::tests::fixture(d.path());
        std::fs::create_dir_all(app.cache.join("thumbs")).unwrap();
        std::fs::write(app.cache.join("thumbs/a.hit"), "12345").unwrap();
        let u = measure(&data, &app);
        assert_eq!(u.part_bytes(Part::Art), 3);
        assert_eq!(u.part_bytes(Part::Logins), 5);
        assert_eq!(u.part_bytes(Part::Settings), 2);
        assert_eq!(u.left[&Left::Tools], 1);
        // heroes/1.jpg + the thumb.
        assert_eq!(u.left[&Left::Images], 4 + 5);
        assert_eq!(u.left[&Left::Logs], 1);
    }
}
