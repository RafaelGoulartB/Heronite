//! `HERONITE_DEBUG_EXTENSIONS=<state>[+<state>…]`: made-up lists of
//! extensions, read without the network, for captures and manual checks.
//! Nothing is downloaded; installing an entry fails as a download would.
//!
//! - `list` (or nothing else): the app's list, read now, with the two
//!   examples of `extensions/metadata/` and made-up sources (one adult, one made
//!   for a newer Heronite).
//! - `update`: the installed extensions count as installed from the app's
//!   list, which offers Wikipedia 1.1.0.
//! - `old`: the list could not be read now; the copy kept is 3 days old.
//! - `offline`: no copy and no network. `404`: the list's address answers
//!   404 (the app's list until it is published). `invalid`: the list does
//!   not read. `empty`: a list with no entry.
//! - `other`: a list the user added besides the app's.
//! - `lists`: the "Lists…" manager open. `pick:<id>`: that entry picked.
//! - `busy`, `error`: the picked entry installing, or its install failed.

use std::time::{Duration, SystemTime};

use heronite_metadata_ext::index::{self, IndexError};
use heronite_metadata_ext::lists::{ListCopy, ListError};
use heronite_metadata_ext::InstalledExtension;

/// The made-up list: the repository's two examples and made-up sources.
pub(super) const MADE_UP: &str = r#"{
  "index": 1,
  "name": "Heronite",
  "extensions": [
    {
      "id": "indie-shelf", "name": "Indie Shelf", "version": "1.4.2", "api": 1, "author": "okabe",
      "homepage": "https://indieshelf.example.org", "kind": "details",
      "description": "A large open catalogue of independent games: descriptions, genres, release dates and covers.",
      "fields": ["description", "releaseDate", "genres", "developers", "cover"],
      "hosts": ["api.indieshelf.example.org", "img.indieshelf.example.org"],
      "settings": [{ "label": "API key", "required": true }],
      "files": {
        "source.toml": { "url": "https://lists.example.org/indie-shelf/source.toml", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 900 },
        "source.js": { "url": "https://lists.example.org/indie-shelf/source.js", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 4000 }
      }
    },
    {
      "id": "retro-box-art", "name": "Retro Box Art", "version": "2.1.0", "api": 1, "author": "pixel-keeper",
      "kind": "pictures",
      "description": "Box art, square and wide covers for consoles and old computers, found by the ROM's name.",
      "fields": ["cover", "coverSquare", "coverWide"],
      "applies": { "to": "emulated" },
      "hosts": ["cdn.retroboxart.example.org"],
      "files": {
        "source.toml": { "url": "https://lists.example.org/retro-box-art/source.toml", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 700 },
        "source.js": { "url": "https://lists.example.org/retro-box-art/source.js", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 2500 }
      }
    },
    {
      "id": "critic-scores", "name": "Critic Scores", "version": "1.0.3", "api": 1, "author": "mira-dev",
      "kind": "extras",
      "description": "The critics' average score of PC and console releases.",
      "fields": ["criticScore"],
      "applies": { "to": "pc" },
      "hosts": ["api.criticscores.example.org"],
      "files": {
        "source.toml": { "url": "https://lists.example.org/critic-scores/source.toml", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 600 },
        "source.js": { "url": "https://lists.example.org/critic-scores/source.js", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 1800 }
      }
    },
    {
      "id": "doujin-finder", "name": "Doujin Finder", "version": "0.9.1", "api": 1, "author": "kissa",
      "homepage": "https://doujin.example.org", "kind": "details", "adult": true,
      "description": "Doujin games and visual novels, found by the code in the folder name. Reads the public product page.",
      "fields": ["description", "releaseDate", "genres", "cover", "hero"],
      "applies": { "to": "pc", "asked": "fallback", "game_kind": { "name": "Visual novels", "signals": ["Visual Novel"] } },
      "hosts": ["www.doujin.example.org"],
      "images": ["img.doujin.example.org"],
      "files": {
        "source.toml": { "url": "https://lists.example.org/doujin-finder/source.toml", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 800 },
        "source.js": { "url": "https://lists.example.org/doujin-finder/source.js", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 3000 }
      }
    },
    {
      "id": "wikipedia", "name": "Wikipedia", "version": "1.1.0", "api": 1, "author": "Heronite",
      "homepage": "https://www.wikipedia.org", "kind": "details",
      "description": "The first paragraphs of the game's Wikipedia article, in your language when there is one. Asked only when the other sources lack the game.",
      "fields": ["description", "releaseDate"],
      "applies": { "to": "all", "asked": "fallback" },
      "hosts": ["*.wikipedia.org"],
      "files": {
        "source.toml": { "url": "https://lists.example.org/wikipedia/source.toml", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 659 },
        "source.js": { "url": "https://lists.example.org/wikipedia/source.js", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 3199 }
      }
    },
    {
      "id": "vndb", "name": "VNDB", "version": "1.0.0", "api": 1, "author": "Heronite",
      "homepage": "https://vndb.org", "kind": "details",
      "description": "The Visual Novel Database (vndb.org), for visual novels and Japanese titles: found by the Japanese, romaji or English name, or by a VNDB id such as v17. Not affiliated with VNDB.",
      "fields": ["description", "releaseDate", "developers", "genres", "themes", "criticScore", "cover", "hero"],
      "applies": { "to": "all", "asked": "fallback", "game_kind": { "name": "Visual novels", "signals": ["Visual Novel"] } },
      "hosts": ["api.vndb.org"],
      "images": ["t.vndb.org"],
      "settings": [{ "label": "Title to show", "required": false }, { "label": "Use a screenshot as background", "required": false }],
      "files": {
        "source.toml": { "url": "https://lists.example.org/vndb/source.toml", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 1373 },
        "source.js": { "url": "https://lists.example.org/vndb/source.js", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 11129 }
      }
    },
    {
      "id": "future-db", "name": "Future DB", "version": "3.0.0", "api": 2, "author": "tomorrow",
      "kind": "details",
      "description": "A source written for a later version of the extension contract.",
      "fields": ["description"],
      "hosts": ["api.future.example.org"],
      "files": {
        "source.toml": { "url": "https://lists.example.org/future-db/source.toml", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 500 },
        "source.js": { "url": "https://lists.example.org/future-db/source.js", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 500 }
      }
    }
  ]
}"#;

/// The list a user added, in the `other` state.
const OTHER: &str = "https://lists.example.net/heronite/index.json";
const OTHER_LIST: &str = r#"{
  "index": 1,
  "extensions": [
    {
      "id": "speedrun-times", "name": "Speedrun Times", "version": "0.3.0", "api": 1, "author": "runner42",
      "kind": "extras",
      "description": "The fastest times of a game's main categories.",
      "fields": ["features"],
      "hosts": ["api.speedruns.example.net"],
      "files": {
        "source.toml": { "url": "https://lists.example.net/heronite/speedrun-times/source.toml", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 500 },
        "source.js": { "url": "https://lists.example.net/heronite/speedrun-times/source.js", "sha256": "0000000000000000000000000000000000000000000000000000000000000000", "size": 1500 }
      }
    }
  ]
}"#;

#[derive(Clone, Debug, Default)]
pub struct Fake {
    states: Vec<String>,
}

impl Fake {
    pub fn from_env() -> Option<Self> {
        let value = crate::env::value("HERONITE_DEBUG_EXTENSIONS")?;
        Some(Self { states: value.split('+').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect() })
    }

    pub fn has(&self, state: &str) -> bool {
        self.states.iter().any(|s| s == state)
    }

    /// `pick:<id>`.
    pub fn pick(&self) -> Option<String> {
        self.states.iter().find_map(|s| s.strip_prefix("pick:").map(str::to_owned))
    }

    pub fn in_use(&self, builtin: &str) -> Vec<(String, bool)> {
        let mut out = vec![(builtin.to_owned(), true)];
        if self.has("other") {
            out.push((OTHER.to_owned(), false));
        }
        out
    }

    /// The installed extensions as the state makes them.
    pub fn adjust(&self, installed: &mut [InstalledExtension], builtin: &str) {
        if self.has("update") {
            for e in installed.iter_mut() {
                e.list = Some(builtin.to_owned());
            }
        }
    }

    pub fn copies(&self, builtin: &str) -> Vec<ListCopy> {
        let now = SystemTime::now();
        let read = |json: &str| index::parse(json.as_bytes()).expect("the made-up list reads");
        let mut own = ListCopy { url: builtin.to_owned(), builtin: true, index: Some(read(MADE_UP)), read_at: Some(now), error: None };
        if self.has("old") {
            own.read_at = Some(now - Duration::from_secs(3 * 86_400));
            own.error = Some(ListError::Unreachable("made up".into()));
        } else if self.has("offline") {
            own = ListCopy { index: None, read_at: None, error: Some(ListError::Unreachable("made up".into())), ..own };
        } else if self.has("404") {
            own = ListCopy { index: None, read_at: None, error: Some(ListError::Status(404)), ..own };
        } else if self.has("invalid") {
            let why = IndexError::Invalid("wikipedia: files.source.js.url \"http://example.org/source.js\" is not an https address".into());
            own = ListCopy { index: None, read_at: None, error: Some(ListError::Invalid(why)), ..own };
        } else if self.has("empty") {
            own.index = Some(read(r#"{"index": 1, "name": "Heronite", "extensions": []}"#));
        }
        let mut out = vec![own];
        if self.has("other") {
            out.push(ListCopy { url: OTHER.to_owned(), builtin: false, index: Some(read(OTHER_LIST)), read_at: Some(now), error: None });
        }
        out
    }
}
