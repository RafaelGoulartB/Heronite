//! Command palette commands of the Wine Manager (see `pages/commands.rs`):
//! check the source on screen and install the newest build of a source.

use gpui_kit::*;
use heronite_i18n::tr;
use heronite_wine::{CheckMode, Repository, WineVersionInfo};

use super::WineManagerPage;
use crate::icons::Icon;
use crate::pages::commands::{CommandCtx, PageCommand, PaletteCommands};

impl WineManagerPage {
    /// The newest installable build of `repo`.
    fn latest(&self, repo: Repository) -> Option<&WineVersionInfo> {
        self.releases.iter().filter(|r| r.kind == repo.kind() && r.downloadable()).max_by(|a, b| a.date.cmp(&b.date))
    }

    /// Runs the command kept while the cache was read or its source checked.
    pub(super) fn run_palette_pending(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.palette_pending.clone() else { return };
        if !self.loaded || install_target(&id).is_some_and(|repo| self.manager.checking(repo)) {
            return;
        }
        self.palette_pending = None;
        self.run_command(&id, false, cx);
    }

    /// `first`: not a retry after the source was checked.
    fn run_command(&mut self, id: &str, first: bool, cx: &mut Context<Self>) {
        if id == "refresh" {
            self.check(self.repo, CheckMode::Manual);
        } else if let Some(repo) = install_target(id) {
            if !self.loaded {
                self.palette_pending = Some(id.to_string());
                return;
            }
            if self.repo != repo {
                self.select_source(repo);
            } else {
                self.check(repo, CheckMode::Auto);
            }
            match self.latest(repo).cloned() {
                Some(r) if r.installed() => self.toast(tr!("{version} is installed already", version = r.version), false, cx),
                Some(r) => {
                    self.manager.start_install(r);
                }
                None if first && self.manager.checking(repo) => self.palette_pending = Some(id.to_string()),
                None => self.toast(tr!("No {kind} release found: refresh the list", kind = repo.kind()), true, cx),
            }
        }
        cx.notify();
    }
}

impl PaletteCommands for WineManagerPage {
    fn palette_commands(page: Option<&Self>, _ctx: &CommandCtx, _cx: &App) -> Vec<PageCommand> {
        let busy = page.is_some_and(|p| p.checking());
        let mut v = vec![PageCommand::new(
            "refresh",
            tr!("Refresh Wine / Proton releases"),
            tr!("Check the source on screen for new builds"),
            Icon::Refresh,
        )
        .keywords(tr!("proton wine runner update list"))
        .disabled(busy.then_some(tr!("Fetching releases already")))];
        for &repo in Repository::offered() {
            let kind = repo.kind();
            let mut c = PageCommand::new(
                format!("install:{kind}"),
                tr!("Install the latest {kind}", kind),
                tr!("Newest build of this source"),
                Icon::Download,
            )
            .keywords(tr!("proton wine runner download compatibility"));
            if let Some(p) = page.filter(|p| p.loaded) {
                match p.latest(repo) {
                    Some(r) if r.installed() => c = c.disabled(Some(tr!("{version} is installed", version = r.version))),
                    Some(r) if p.job(&r.version).is_some() => c = c.disabled(Some(tr!("Installing {version}", version = r.version))),
                    Some(r) => c.subtitle = format!("{} \u{b7} {}", r.version, r.date),
                    None => {}
                }
            }
            v.push(c);
        }
        v
    }

    fn run_palette_command(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
        self.run_command(id, true, cx);
    }
}

/// The source of an `install:<type>` command.
fn install_target(id: &str) -> Option<Repository> {
    id.strip_prefix("install:").and_then(|k| Repository::offered().iter().copied().find(|r| r.kind() == k))
}
