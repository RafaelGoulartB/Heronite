//! Collection screenshots backend (port of `src/local-library/screenshots/**`
//! of the Electron fork).
//!
//! * [`matching`] - game title -> captures sub-folder (exact port of `match.ts`).
//! * [`list`]     - [`list_screenshots`] (port of `index.ts`).
//! * [`thumbnails`] - 320x180 preview cache, compatible with Heronite's
//!   `local_library/screenshot-thumbnails` (port of `thumbnails.ts`).
//! * [`posters`] - a frame of each replay (ffmpeg) in that same cache.
//! * [`trash_screenshot`] - move an original to the freedesktop trash.
//! * [`capture`]  - global-shortcut screen capture (port of `capture.ts`).
//! * [`compress`] - lossless PNG -> WebP conversion of the captures (this app only).

pub mod capture;
pub mod compress;
pub mod list;
pub mod matching;
pub mod posters;
pub mod thumbnails;
mod trashing;

pub use list::{list_all_screenshots, list_screenshots, GameQuery, Screenshot, ScreenshotsResult, IMAGE_EXTENSIONS, REPLAYS_FOLDER};
pub use matching::{pick_screenshot_folder, score_screenshot_folder, FolderMatch};
pub use posters::{is_video, Poster};
pub use thumbnails::{CacheInfo, ThumbnailCache, THUMB_HEIGHT, THUMB_WIDTH};
pub use trashing::{trash_screenshot, trash_screenshot_in, TrashError};
