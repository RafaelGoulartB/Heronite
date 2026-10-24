//! The shapes of `files.json`: a mod's files and which file replaced which.

use serde::Deserialize;

use super::null_default;

/// One file of a mod page.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct NexusFile {
    pub file_id: u64,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    pub version: Option<String>,
    pub category_id: Option<i64>,
    /// `MAIN`, `UPDATE`, `OPTIONAL`, `OLD_VERSION`, `MISCELLANEOUS`,
    /// `ARCHIVED` or `None` (removed from the page).
    pub category_name: Option<String>,
    pub is_primary: bool,
    pub size_kb: Option<u64>,
    pub size_in_bytes: Option<u64>,
    #[serde(deserialize_with = "null_default")]
    pub file_name: String,
    /// Unix seconds.
    pub uploaded_timestamp: u64,
    pub description: Option<String>,
}

impl NexusFile {
    pub fn category(&self) -> &str {
        self.category_name.as_deref().unwrap_or("")
    }

    pub fn is_main(&self) -> bool {
        self.category().eq_ignore_ascii_case("MAIN")
    }

    /// Bytes, from whichever size the API sent.
    pub fn size(&self) -> u64 {
        self.size_in_bytes.filter(|b| *b > 0).or(self.size_kb.map(|k| k * 1024)).unwrap_or(0)
    }
}

/// "File `old_file_id` was replaced by `new_file_id`", as the author set it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct FileUpdate {
    pub old_file_id: u64,
    pub new_file_id: u64,
    pub old_file_name: String,
    pub new_file_name: String,
    pub uploaded_timestamp: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct NexusFiles {
    #[serde(deserialize_with = "null_default")]
    pub files: Vec<NexusFile>,
    #[serde(deserialize_with = "null_default")]
    pub file_updates: Vec<FileUpdate>,
}

impl NexusFiles {
    pub fn get(&self, file_id: u64) -> Option<&NexusFile> {
        self.files.iter().find(|f| f.file_id == file_id)
    }
}
