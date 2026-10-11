//! Command palette commands of the Wine Manager (see `pages/commands.rs`):
//! refresh the release lists and install the newest build of a source.

use gpui_kit::*;
use heronite_wine::{Repository, WineVersionInfo};

use super::WineManagerPage;
use crate::icons::Icon;
use crate::pages::commands::{CommandCtx, PageCommand, PaletteCommands};

impl WineManagerPage {
    /// The newest installable build of `repo`.
    fn latest(&self, repo: Repository) -> Option<&WineVersionInfo> {
        self.releases.iter().filter(|r| r.kind == repo.kind() && r.downloadable()).max_by(|a, b| a.date.cmp(&b.date))
    }

    /// Runs the command kept while the release list was not read yet.
    pub(super) fn run_palette_pending(&mut self, cx: &mut Context<Self>) {
        if self.loaded && !self.releases.is_empty() {
            if let Some(id) = self.palette_pending.take() {
                self.run_command(&id, cx);
            }
        }
    }

    fn run_command(&mut self, id: &str, cx: &mut Context<Self>) {
        if id == "refresh" {
            self.manager.start_refresh(Repository::LINUX.to_vec());
        } else if let Some(repo) = id.strip_prefix("install:").and_then(|k| Repository::LINUX.into_iter().find(|r| r.kind() == k)) {
            if !self.loaded || self.releases.is_empty() {
                self.palette_pending = Some(id.to_string());
                return;
            }
            self.repo = repo;
            match self.latest(repo).cloned() {
                Some(r) if r.installed() => self.toast(format!("{} is installed already", r.version), false, cx),
                Some(r) => {
                    self.manager.start_install(r);
                }
                None => self.toast(format!("No {} release found: refresh the list", repo.kind()), true, cx),
            }
        }
        cx.notify();
    }
}

impl PaletteCommands for WineManagerPage {
    fn palette_commands(page: Option<&Self>, _ctx: &CommandCtx, _cx: &App) -> Vec<PageCommand> {
        let busy = page.is_some_and(|p| p.manager.refreshing());
        let mut v = vec![PageCommand::new("refresh", "Refresh Wine / Proton releases", "Fetch the newest builds", Icon::Refresh)
            .keywords("proton wine runner update list")
            .disabled(busy.then_some("Fetching releases already"))];
        for repo in Repository::LINUX {
            let kind = repo.kind();
            let mut c = PageCommand::new(format!("install:{kind}"), format!("Install the latest {kind}"), "Newest build of this source", Icon::Download)
                .keywords("proton wine runner download compatibility");
            if let Some(p) = page.filter(|p| p.loaded) {
                match p.latest(repo) {
                    Some(r) if r.installed() => c = c.disabled(Some(format!("{} is installed", r.version))),
                    Some(r) if p.job(&r.version).is_some() => c = c.disabled(Some(format!("Installing {}", r.version))),
                    Some(r) => c.subtitle = format!("{} \u{b7} {}", r.version, r.date),
                    None => {}
                }
            }
            v.push(c);
        }
        v
    }

    fn run_palette_command(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
        self.run_command(id, cx);
    }
}
