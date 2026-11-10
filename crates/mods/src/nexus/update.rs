//! Which file of a mod page replaces the installed one.

use super::{NexusFile, NexusFiles};
use crate::http::now_iso;
use crate::{ModEntry, ModSource, UpdateInfo};

/// The newer file of `entry`'s mod page, if any. Pure: `files` comes from
/// [`super::files`].
///
/// 1. The author's own chain (`file_updates`: old file replaced by new file)
///    followed from the installed file to its end.
/// 2. Otherwise, when the installed file left the page (gone, `OLD_VERSION`,
///    `ARCHIVED`) and exactly one newer `MAIN` file exists, that one. More
///    than one is ambiguous: no update is claimed.
pub fn check_update(files: &NexusFiles, entry: &ModEntry) -> Option<UpdateInfo> {
    let ModSource::Nexus { file_id: installed, .. } = &entry.source else { return None };
    let target = follow_chain(files, *installed).or_else(|| single_newer_main(files, *installed))?;
    let version = match files.get(target) {
        Some(f) => file_version(f),
        None => files.file_updates.iter().find(|u| u.new_file_id == target).map(|u| u.new_file_name.clone()).unwrap_or_default(),
    };
    Some(UpdateInfo { version, file_id: Some(target), checked_at: now_iso() })
}

fn follow_chain(files: &NexusFiles, from: u64) -> Option<u64> {
    let mut current = from;
    let mut seen = vec![from];
    while let Some(next) = files.file_updates.iter().filter(|u| u.old_file_id == current).max_by_key(|u| u.uploaded_timestamp) {
        if seen.contains(&next.new_file_id) {
            break;
        }
        current = next.new_file_id;
        seen.push(current);
    }
    (current != from).then_some(current)
}

fn single_newer_main(files: &NexusFiles, installed: u64) -> Option<u64> {
    let old = files.get(installed);
    let retired = old.is_none_or(|f| matches!(f.category().to_ascii_uppercase().as_str(), "OLD_VERSION" | "ARCHIVED" | ""));
    if !retired {
        return None;
    }
    let newer: Vec<&NexusFile> = files
        .files
        .iter()
        .filter(|f| f.is_main())
        .filter(|f| match old {
            Some(o) => f.uploaded_timestamp > o.uploaded_timestamp,
            None => f.file_id > installed,
        })
        .collect();
    match newer.as_slice() {
        [one] => Some(one.file_id),
        _ => None,
    }
}

fn file_version(f: &NexusFile) -> String {
    f.version.clone().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| f.name.clone())
}

/// The file "Install" picks on a mod page for premium accounts: the primary
/// `MAIN` file, else the newest `MAIN` one.
pub fn main_file(files: &NexusFiles) -> Option<&NexusFile> {
    let mains = || files.files.iter().filter(|f| f.is_main());
    mains().filter(|f| f.is_primary).max_by_key(|f| f.uploaded_timestamp).or_else(|| mains().max_by_key(|f| f.uploaded_timestamp))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(json: &str) -> NexusFiles {
        serde_json::from_str(json).unwrap()
    }

    fn entry(file_id: u64) -> ModEntry {
        ModEntry { source: ModSource::Nexus { domain: "g".into(), mod_id: 1, file_id }, ..Default::default() }
    }

    const SAMPLE: &str = r#"{
      "files": [
        {"id":[100,1],"uid":1,"file_id":100,"name":"Mod","version":"1.0","category_id":4,"category_name":"OLD_VERSION","is_primary":false,
         "size":1000,"file_name":"Mod-1-0.zip","uploaded_timestamp":1000,"uploaded_time":"x","mod_version":"1.0","external_virus_scan_url":null,
         "description":"","size_kb":1000,"size_in_bytes":1024000,"changelog_html":null,"content_preview_link":"x"},
        {"file_id":200,"name":"Mod","version":"1.1","category_id":1,"category_name":"MAIN","is_primary":true,"file_name":"Mod-1-1.zip",
         "uploaded_timestamp":2000,"size_kb":1100,"size_in_bytes":null,"description":null},
        {"file_id":300,"name":"Extras","version":"1.1","category_id":3,"category_name":"OPTIONAL","is_primary":false,"file_name":"x.zip",
         "uploaded_timestamp":2100},
        {"file_id":400,"name":"Gone","version":null,"category_id":null,"category_name":null,"is_primary":false,"file_name":null,"uploaded_timestamp":10}
      ],
      "file_updates": [
        {"old_file_id":100,"new_file_id":200,"old_file_name":"Mod-1-0.zip","new_file_name":"Mod-1-1.zip","uploaded_timestamp":2000,"uploaded_time":"x"}
      ]
    }"#;

    #[test]
    fn the_page_parses() {
        let p = page(SAMPLE);
        assert_eq!((p.files.len(), p.file_updates.len()), (4, 1));
        assert_eq!(p.files[1].size(), 1100 * 1024);
        assert_eq!(p.files[0].size(), 1_024_000);
        assert_eq!(p.files[3].file_name, "");
        assert!(p.get(200).unwrap().is_main());
    }

    #[test]
    fn the_authors_chain_wins() {
        let u = check_update(&page(SAMPLE), &entry(100)).unwrap();
        assert_eq!((u.version.as_str(), u.file_id), ("1.1", Some(200)));
        assert!(!u.checked_at.is_empty());
        assert_eq!(check_update(&page(SAMPLE), &entry(200)), None);
    }

    #[test]
    fn chains_are_followed_to_their_end_and_cycles_stop() {
        let p = page(
            r#"{"files":[{"file_id":3,"version":"3.0","category_name":"MAIN","uploaded_timestamp":30}],
                "file_updates":[{"old_file_id":1,"new_file_id":2,"uploaded_timestamp":20,"new_file_name":"two.zip"},
                                {"old_file_id":2,"new_file_id":3,"uploaded_timestamp":30,"new_file_name":"three.zip"},
                                {"old_file_id":3,"new_file_id":1,"uploaded_timestamp":40,"new_file_name":"one.zip"}]}"#,
        );
        assert_eq!(check_update(&p, &entry(1)).map(|u| u.file_id), Some(Some(3)));
        let missing =
            page(r#"{"files":[],"file_updates":[{"old_file_id":1,"new_file_id":2,"uploaded_timestamp":20,"new_file_name":"two.zip"}]}"#);
        assert_eq!(check_update(&missing, &entry(1)).map(|u| u.version), Some("two.zip".to_string()));
    }

    #[test]
    fn a_retired_file_moves_to_the_only_newer_main() {
        let p = page(
            r#"{"files":[{"file_id":1,"version":"1","category_name":"OLD_VERSION","uploaded_timestamp":10},
                         {"file_id":2,"version":"2","category_name":"MAIN","uploaded_timestamp":20}],"file_updates":[]}"#,
        );
        assert_eq!(check_update(&p, &entry(1)).map(|u| (u.version, u.file_id)), Some(("2".into(), Some(2))));
        // Two candidates: ambiguous.
        let two = page(
            r#"{"files":[{"file_id":1,"category_name":"ARCHIVED","uploaded_timestamp":10},
                         {"file_id":2,"category_name":"MAIN","uploaded_timestamp":20},
                         {"file_id":3,"category_name":"MAIN","uploaded_timestamp":30}]}"#,
        );
        assert_eq!(check_update(&two, &entry(1)), None);
        // The installed file removed from the page.
        let gone = page(r#"{"files":[{"file_id":5,"version":"5","category_name":"MAIN","uploaded_timestamp":50}]}"#);
        assert_eq!(check_update(&gone, &entry(1)).map(|u| u.file_id), Some(Some(5)));
        // Installed file still current, or an optional one: nothing to report.
        let current = page(
            r#"{"files":[{"file_id":1,"category_name":"MAIN","uploaded_timestamp":10},{"file_id":2,"category_name":"MAIN","uploaded_timestamp":20}]}"#,
        );
        assert_eq!(check_update(&current, &entry(1)), None);
        let optional = page(
            r#"{"files":[{"file_id":1,"category_name":"OPTIONAL","uploaded_timestamp":10},{"file_id":2,"category_name":"MAIN","uploaded_timestamp":20}]}"#,
        );
        assert_eq!(check_update(&optional, &entry(1)), None);
    }

    #[test]
    fn other_sources_have_no_nexus_updates() {
        assert_eq!(check_update(&page(SAMPLE), &ModEntry::default()), None);
    }

    #[test]
    fn main_file_prefers_the_primary_then_the_newest() {
        let p = page(SAMPLE);
        assert_eq!(main_file(&p).map(|f| f.file_id), Some(200));
        let none_primary = page(
            r#"{"files":[{"file_id":1,"category_name":"MAIN","uploaded_timestamp":10},{"file_id":2,"category_name":"MAIN","uploaded_timestamp":20},
                         {"file_id":3,"category_name":"OPTIONAL","uploaded_timestamp":90}]}"#,
        );
        assert_eq!(main_file(&none_primary).map(|f| f.file_id), Some(2));
        assert_eq!(main_file(&page(r#"{"files":[{"file_id":3,"category_name":"OPTIONAL"}]}"#)), None);
    }
}
