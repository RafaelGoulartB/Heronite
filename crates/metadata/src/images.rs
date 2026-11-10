//! Local copies of provider images in `local_library/metadata-art/`: port of
//! `metadata/images.ts`.

use std::fs;
use std::path::{Path, PathBuf};

use heronite_data::paths::DataPaths;
use heronite_images::public;

use crate::http::Http;
use crate::types::ArtKind;
use crate::util::{folder_name, local_art_url, sniff_image_ext, ALLOWED_EXT};

const MAX_IMAGE_BYTES: u64 = 2 * 1024 * 1024;

/// `removeMetadataArt`.
pub fn remove_metadata_art(paths: &DataPaths, runner: &str, app_name: &str) {
    let dir = paths.metadata_art_dir().join(folder_name(runner, app_name));
    if !dir.exists() {
        return;
    }
    if let Err(e) = fs::remove_dir_all(&dir) {
        log::warn!("Could not remove metadata art {}: {e}", dir.display());
    }
}

fn dest_file(paths: &DataPaths, runner: &str, app_name: &str, kind: ArtKind, ext: &str) -> std::io::Result<PathBuf> {
    let dir = paths.metadata_art_dir().join(folder_name(runner, app_name));
    fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("{}.{ext}", kind.file_stem())))
}

pub fn is_remote_image(url: Option<&str>) -> bool {
    url.is_some_and(|u| {
        let l = u.get(..8).unwrap_or(u).to_ascii_lowercase();
        l.starts_with("http://") || l.starts_with("https://")
    })
}

/// `isLocalFile`.
pub fn is_local_file(url: Option<&str>) -> bool {
    url.is_some_and(|u| !u.is_empty() && !u.starts_with("http") && !u.starts_with("localart:"))
}

fn write_atomic(dest: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = dest.with_extension(format!("{}.part", dest.extension().and_then(|e| e.to_str()).unwrap_or("img")));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, dest)
}

/// `materializeMetadataImage`: turns a candidate image into what is stored in
/// `metadata.json`.
///
/// - `localart://` URLs are kept;
/// - a local file path is copied to `metadata-art/<runner_app>/<kind>.<ext>`
///   ([`ArtKind::file_stem`]: `cover`, `hero`, `cover-square`, `cover-wide`)
///   and becomes a `localart://metadata-art/...` URL (`None` if missing or not
///   an image);
/// - a remote URL is downloaded the same way when `download` is set (max
///   2 MB), and kept as the URL when not or when the download fails.
pub fn materialize_metadata_image(
    paths: &DataPaths,
    http: &Http,
    runner: &str,
    app_name: &str,
    kind: ArtKind,
    source: Option<&str>,
    download: bool,
) -> Option<String> {
    let source = source.map(str::trim).filter(|s| !s.is_empty())?;
    if source.starts_with("localart:") {
        return Some(source.to_owned());
    }
    let folder = folder_name(runner, app_name);

    if is_local_file(Some(source)) {
        let src = Path::new(source);
        if !src.exists() {
            return None;
        }
        let ext = src.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_else(|| "jpg".into());
        if !ALLOWED_EXT.contains(&ext.as_str()) {
            return None;
        }
        let ext = if ext == "jpeg" { "jpg".to_owned() } else { ext };
        let dest = dest_file(paths, runner, app_name, kind, &ext).ok()?;
        // Through a new file, never into the old one: a bulk job's Undo keeps
        // the old picture as a hard link to it.
        write_atomic(&dest, &fs::read(src).ok()?).ok()?;
        return Some(local_art_url("metadata-art", &[&folder, &format!("{}.{ext}", kind.file_stem())]));
    }

    if !download || !is_remote_image(Some(source)) {
        return Some(source.to_owned());
    }

    let fetched = (|| -> anyhow::Result<Option<String>> {
        // A picture an extension named: public addresses only, each redirect
        // checked again.
        let body = if public::is_extension_url(source) {
            public::fetch(source, MAX_IMAGE_BYTES, std::time::Duration::from_secs(20)).map_err(anyhow::Error::msg)?
        } else {
            let resp = http.get(source).timeout_secs(20).max_bytes(MAX_IMAGE_BYTES).send()?;
            if resp.status != 200 {
                anyhow::bail!("HTTP {}", resp.status);
            }
            resp.body
        };
        let ext = sniff_image_ext(&body, false).unwrap_or("jpg");
        if !ALLOWED_EXT.contains(&ext) || body.len() < 32 {
            return Ok(None);
        }
        let dest = dest_file(paths, runner, app_name, kind, ext)?;
        write_atomic(&dest, &body)?;
        Ok(Some(local_art_url("metadata-art", &[&folder, &format!("{}.{ext}", kind.file_stem())])))
    })();
    match fetched {
        Ok(Some(url)) => Some(url),
        Ok(None) => Some(source.to_owned()),
        Err(e) => {
            let shown = source.split(['?', '#']).next().unwrap_or(source);
            log::warn!("Metadata image download skipped for {shown}: {e}");
            Some(source.to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_files_are_copied_and_remote_urls_kept() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("cfg"));
        let http = Http::default();
        let src = dir.path().join("a.JPEG");
        fs::write(&src, b"fake").unwrap();
        let url = materialize_metadata_image(&paths, &http, "sideload", "steam_1", ArtKind::Cover, src.to_str(), false).unwrap();
        assert_eq!(url, "localart://metadata-art/sideload_steam_1/cover.jpg");
        assert!(paths.metadata_art_dir().join("sideload_steam_1/cover.jpg").exists());
        // Missing / unsupported local files vanish.
        assert!(materialize_metadata_image(&paths, &http, "sideload", "x", ArtKind::Hero, Some("/nope.png"), false).is_none());
        let txt = dir.path().join("a.txt");
        fs::write(&txt, b"x").unwrap();
        assert!(materialize_metadata_image(&paths, &http, "sideload", "x", ArtKind::Hero, txt.to_str(), false).is_none());
        // Remote URL without download stays a URL; localart stays.
        assert_eq!(
            materialize_metadata_image(&paths, &http, "sideload", "x", ArtKind::Hero, Some(" https://a/b.jpg "), false).as_deref(),
            Some("https://a/b.jpg")
        );
        assert_eq!(
            materialize_metadata_image(&paths, &http, "sideload", "x", ArtKind::Hero, Some("localart://heroes/1.jpg"), true).as_deref(),
            Some("localart://heroes/1.jpg")
        );
        assert!(materialize_metadata_image(&paths, &http, "sideload", "x", ArtKind::Hero, None, true).is_none());
        // Each cover family has its own file next to the tall one.
        let sq = materialize_metadata_image(&paths, &http, "sideload", "steam_1", ArtKind::CoverSquare, src.to_str(), false).unwrap();
        assert_eq!(sq, "localart://metadata-art/sideload_steam_1/cover-square.jpg");
        let wide = materialize_metadata_image(&paths, &http, "sideload", "steam_1", ArtKind::CoverWide, src.to_str(), false).unwrap();
        assert_eq!(wide, "localart://metadata-art/sideload_steam_1/cover-wide.jpg");
        assert!(paths.metadata_art_dir().join("sideload_steam_1/cover.jpg").exists());
        remove_metadata_art(&paths, "sideload", "steam_1");
        assert!(!paths.metadata_art_dir().join("sideload_steam_1").exists());
    }

    #[test]
    fn an_extensions_picture_is_never_copied_from_this_computer() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for mut s in listener.incoming().flatten() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let mut body = b"\x89PNG\r\n\x1a\n".to_vec();
                body.resize(64, 0);
                let head =
                    format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().join("cfg"));
        let http = Http::default();
        let url = format!("{base}/c.png");
        // A built-in source's picture there is copied as before.
        let copied = materialize_metadata_image(&paths, &http, "sideload", "a", ArtKind::Cover, Some(&url), true).unwrap();
        assert!(copied.starts_with("localart://"), "{copied}");
        // An extension's is not fetched: it stays the address it was.
        let marked = public::marked(&url);
        let kept = materialize_metadata_image(&paths, &http, "sideload", "b", ArtKind::Cover, Some(&marked), true).unwrap();
        assert_eq!(kept, marked);
        assert!(!paths.metadata_art_dir().join("sideload_b").exists());
    }
}
