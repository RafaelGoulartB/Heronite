//! Short texts of the manager: sizes, counts, the status line, job stages.

use heronite_i18n::{decimal, grouped, tr, trn};
use heronite_mods::ModSource;

use crate::services::mods::{GameMods, Job, JobStage};

/// `1.2 MB`, `640 KB`, `12 B` (powers of 1024).
pub(super) fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024. && i < UNITS.len() - 1 {
        v /= 1024.;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{} {}", decimal(v, 1), UNITS[i])
    }
}

/// `12,345` or `1.2M` for the big download counts of the sites.
pub(super) fn compact(n: u64) -> String {
    match n {
        0..=9_999 => grouped(n as i64),
        10_000..=999_999 => format!("{}k", decimal(n as f64 / 1_000., 0)),
        _ => format!("{}M", decimal(n as f64 / 1_000_000., 1)),
    }
}

pub(super) fn files(n: u64) -> String {
    trn!(n, "{n} file", "{n} files")
}

pub(super) fn source_label(source: &ModSource) -> &'static str {
    match source {
        ModSource::File => tr!("File"),
        ModSource::Nexus { .. } => tr!("Nexus Mods"),
        ModSource::Thunderstore { .. } => tr!("Thunderstore"),
    }
}

/// "12 mods · 10 on · applied when you play from Heronite".
pub(super) fn status_line(g: &GameMods) -> String {
    let total = g.list.mods.len();
    if total == 0 {
        return tr!("No mods yet").to_string();
    }
    let on = g.list.enabled().count();
    let count = trn!(total, "{n} mod", "{n} mods");
    if !g.list.active {
        tr!("{count} · off · the game starts without them", count)
    } else {
        tr!("{count} · {on} on · applied when you play from Heronite", count, on)
    }
}

/// What a job is doing, in a few words.
pub(super) fn stage_text(job: &Job) -> String {
    match job.stage {
        JobStage::Waiting => tr!("Waiting…").to_string(),
        JobStage::Tool => tr!("Getting fuse-overlayfs…").to_string(),
        JobStage::Downloading => match (job.progress.total, job.progress.done) {
            (0, _) => tr!("Downloading…").to_string(),
            (total, done) => tr!("Downloading · {done} of {total}", done = size(done), total = size(total)),
        },
        JobStage::Extracting => tr!("Unpacking…").to_string(),
        JobStage::NeedsChoices => tr!("Waiting for your choices").to_string(),
        JobStage::Installing => tr!("Installing…").to_string(),
        JobStage::Done if job.dependencies > 0 => {
            let n = job.dependencies;
            tr!("Installed · {deps}", deps = trn!(n, "+{n} dependency", "+{n} dependencies"))
        }
        JobStage::Done => tr!("Installed").to_string(),
        JobStage::Failed => job.error.clone().unwrap_or_else(|| tr!("The install failed").to_string()),
        JobStage::Cancelled => tr!("Cancelled").to_string(),
    }
}

/// A mod's initial for its thumbnail when it has no picture.
pub(super) fn initial(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into())
}

pub(super) fn is_active(stage: JobStage) -> bool {
    !matches!(stage, JobStage::Done | JobStage::Failed | JobStage::Cancelled)
}
