//! Zips: the ReShade installer (a PE with a zip appended at its end) and
//! shader packages (a repository's zip, whose layout varies).

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::net::{Cancel, CANCELLED};

fn open(path: &Path) -> Result<zip::ZipArchive<File>, String> {
    let f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // The default config finds an archive that starts after other data,
    // like the installer's PE.
    zip::ZipArchive::new(f).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes the entry named `name` (at the top of the archive) to `dest`.
pub fn extract_file(archive: &Path, name: &str, dest: &Path) -> Result<(), String> {
    let mut zip = open(archive)?;
    let mut entry = zip.by_name(name).map_err(|e| format!("{name} in {}: {e}", archive.display()))?;
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    heronite_data::temps::write_atomic(dest, &bytes, None).map_err(|e| format!("{}: {e}", dest.display()))
}

/// Which part of the game folder a package file goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Shaders,
    Textures,
}

const TEXTURE_EXTS: [&str; 7] = ["png", "jpg", "jpeg", "dds", "bmp", "tga", "hdr"];

fn ext(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// Where a package's shaders and textures are in its file list. ReShade's
/// installer copies the `Shaders` and `Textures` folders it finds; a repo
/// without them has its `.fx` files somewhere (the folder of the shallowest
/// one is used, and only effect and texture files are taken from it).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Layout {
    shaders: Vec<String>,
    /// `false`: a folder of loose files, not a `Shaders` folder.
    shaders_named: bool,
    textures: Option<Vec<String>>,
}

fn layout(files: &[Vec<String>]) -> Option<Layout> {
    let named = |want: &str| {
        files
            .iter()
            .filter_map(|f| f[..f.len() - 1].iter().position(|c| c.eq_ignore_ascii_case(want)).map(|i| f[..=i].to_vec()))
            .min_by_key(Vec::len)
    };
    let textures = named("Textures");
    if let Some(shaders) = named("Shaders") {
        return Some(Layout { shaders, shaders_named: true, textures });
    }
    let fx = files.iter().filter(|f| f.last().is_some_and(|n| matches!(ext(n).as_str(), "fx" | "fxh"))).min_by_key(|f| f.len())?;
    Some(Layout { shaders: fx[..fx.len() - 1].to_vec(), shaders_named: false, textures })
}

/// Calls `each` with the part, the path below it and the content of every
/// file a package installs. `deny`: effect file names left out.
pub fn package_files(
    archive: &Path,
    deny: &[String],
    cancel: Cancel,
    mut each: impl FnMut(Part, &[String], &mut dyn Read) -> Result<(), String>,
) -> Result<(), String> {
    let mut zip = open(archive)?;
    let mut files: Vec<(usize, Vec<String>)> = Vec::new();
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        // `enclosed_name` drops absolute paths and `..`.
        let Some(name) = entry.enclosed_name() else { continue };
        let parts: Vec<String> = name.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        if !parts.is_empty() {
            files.push((i, parts));
        }
    }
    let names: Vec<Vec<String>> = files.iter().map(|(_, p)| p.clone()).collect();
    let layout = layout(&names).ok_or_else(|| "no ReShade effects (.fx files) in this archive".to_string())?;
    for (i, parts) in &files {
        if cancel() {
            return Err(CANCELLED.into());
        }
        let file = parts.last().map(String::as_str).unwrap_or("");
        let under = |root: &[String]| parts.len() > root.len() && parts[..root.len()] == *root;
        let place = if layout.textures.as_deref().is_some_and(under) {
            Some((Part::Textures, layout.textures.as_ref().map_or(0, Vec::len)))
        } else if under(&layout.shaders) {
            match layout.shaders_named {
                true => Some((Part::Shaders, layout.shaders.len())),
                false if matches!(ext(file).as_str(), "fx" | "fxh") => Some((Part::Shaders, layout.shaders.len())),
                false if layout.textures.is_none() && TEXTURE_EXTS.contains(&ext(file).as_str()) => {
                    Some((Part::Textures, layout.shaders.len()))
                }
                false => None,
            }
        } else {
            None
        };
        let Some((part, skip)) = place else { continue };
        if part == Part::Shaders && deny.iter().any(|d| d.eq_ignore_ascii_case(file)) {
            continue;
        }
        let mut entry = zip.by_index(*i).map_err(|e| e.to_string())?;
        each(part, &parts[skip..], &mut entry)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// A zip of `files` (`(name, content)`), after `prefix` bytes.
    pub(crate) fn zip_bytes(prefix: &[u8], files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        out.write_all(prefix).unwrap();
        {
            let mut w = zip::ZipWriter::new(&mut out);
            for (name, content) in files {
                w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
                w.write_all(content).unwrap();
            }
            w.finish().unwrap();
        }
        out.into_inner()
    }

    fn names(files: &[&str]) -> Vec<Vec<String>> {
        files.iter().map(|f| f.split('/').map(str::to_string).collect()).collect()
    }

    #[test]
    fn finds_the_shaders_and_textures_folders() {
        let l = layout(&names(&["Sweet-master/README.md", "Sweet-master/Shaders/SweetFX/CAS.fx", "Sweet-master/Textures/a.png"])).unwrap();
        assert_eq!(l.shaders, ["Sweet-master", "Shaders"]);
        assert_eq!(l.textures.unwrap(), ["Sweet-master", "Textures"]);
        let loose = layout(&names(&["r-main/LICENSE", "r-main/src/deep/x.fx", "r-main/src/A.fx"])).unwrap();
        assert_eq!((loose.shaders, loose.shaders_named), (vec!["r-main".to_string(), "src".to_string()], false));
        assert!(layout(&names(&["r/readme.md"])).is_none());
    }

    #[test]
    fn reads_the_installer_and_the_package_files() {
        let dir = tempfile::tempdir().unwrap();
        let setup = dir.path().join("setup.exe");
        std::fs::write(&setup, zip_bytes(b"MZ fake installer", &[("ReShade64.dll", b"dll64"), ("ReShade32.dll", b"dll32")])).unwrap();
        extract_file(&setup, "ReShade64.dll", &dir.path().join("out/ReShade64.dll")).unwrap();
        assert_eq!(std::fs::read(dir.path().join("out/ReShade64.dll")).unwrap(), b"dll64");
        assert!(extract_file(&setup, "missing.dll", &dir.path().join("x")).is_err());

        let pkg = dir.path().join("p.zip");
        let files: [(&str, &[u8]); 5] = [
            ("p-main/Shaders/A.fx", b"a"),
            ("p-main/Shaders/Template.fx", b"t"),
            ("p-main/Shaders/sub/B.fxh", b"b"),
            ("p-main/Textures/t.png", b"png"),
            ("p-main/README.md", b"r"),
        ];
        std::fs::write(&pkg, zip_bytes(b"", &files)).unwrap();
        let mut got = Vec::new();
        package_files(&pkg, &["template.fx".into()], &|| false, |part, rel, r| {
            let mut s = String::new();
            r.read_to_string(&mut s).unwrap();
            got.push((part, rel.join("/"), s));
            Ok(())
        })
        .unwrap();
        assert_eq!(
            got,
            [
                (Part::Shaders, "A.fx".to_string(), "a".to_string()),
                (Part::Shaders, "sub/B.fxh".to_string(), "b".to_string()),
                (Part::Textures, "t.png".to_string(), "png".to_string()),
            ]
        );
        assert_eq!(package_files(&pkg, &[], &|| true, |_, _, _| Ok(())), Err(CANCELLED.to_string()));
    }
}
