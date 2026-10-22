//! `nxm://` and `ror2mm://` links: which Collection game a link is for, and
//! starting its download. A link names the site's game, not ours, so the
//! installed games are matched by what their lists are linked to, then by the
//! known-games table, then by title; when that does not give exactly one game
//! the user picks (and the choice is remembered in the game's links).

use std::collections::VecDeque;
use std::sync::Arc;

use gpui_kit::*;
use heronite_data::{DataPaths, Library};
use heronite_i18n::tr;
use heronite_mods::games::{self, normalize_title};
use heronite_mods::nexus::{self, NxmError, NxmLink};
use heronite_mods::thunderstore::{self, Ror2mmLink};
use heronite_mods::{links, store, GameLinks, ModsRoot};

use super::{LinkRequest, ModsEvent, ModsService};

/// A link that was understood.
#[derive(Clone, Debug)]
pub(super) enum Target {
    Nexus(NxmLink),
    Thunderstore(Ror2mmLink),
}

/// The site's game, as far as the link tells.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Site {
    pub nexus_domain: Option<String>,
    /// Thunderstore communities listing the package.
    pub communities: Vec<String>,
    /// The game's name on the site.
    pub names: Vec<String>,
}

/// An installed game, for matching.
pub(super) struct Candidate<'a> {
    pub title: &'a str,
    pub steam_app_id: Option<&'a str>,
    pub links: &'a GameLinks,
}

/// Indexes of the candidates the site's game is: the games whose lists are
/// linked to it, else those the known-games table says so about, else those
/// with its name. The first group that is not empty.
pub(super) fn matching(site: &Site, games: &[Candidate]) -> Vec<usize> {
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let domain = |d: Option<&str>| d.zip(site.nexus_domain.as_deref()).is_some_and(|(a, b)| same(a, b));
    let community = |c: Option<&str>| c.is_some_and(|c| site.communities.iter().any(|s| same(s, c)));
    let linked = |g: &Candidate| domain(g.links.nexus_domain.as_deref()) || community(g.links.thunderstore.as_deref());
    let known = |g: &Candidate| games::find(g.steam_app_id, g.title).is_some_and(|k| domain(k.nexus_domain) || community(k.thunderstore));
    let named = |g: &Candidate| {
        let title = normalize_title(g.title);
        !title.is_empty() && site.names.iter().any(|n| normalize_title(n) == title)
    };
    let tiers: [&dyn Fn(&Candidate) -> bool; 3] = [&linked, &known, &named];
    for tier in tiers {
        let found: Vec<usize> = games.iter().enumerate().filter(|(_, g)| tier(g)).map(|(i, _)| i).collect();
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

/// What to put in the chosen game's links, so the next link finds it at once.
#[derive(Clone, Debug, Default)]
pub(super) struct Remember {
    pub nexus_domain: Option<String>,
    pub thunderstore: Option<String>,
}

struct Pending {
    request: LinkRequest,
    target: Target,
    remember: Remember,
    library: Arc<Library>,
}

#[derive(Default)]
pub(super) struct Links {
    /// Links read but not matched yet.
    waiting: Vec<Target>,
    matching: bool,
    /// Links that need a pick, the first on screen (`link_request`).
    pending: VecDeque<Pending>,
}

/// A link matched against the installed games.
struct Resolved {
    target: Target,
    site: Site,
    /// Keys of the games it can be, best first.
    matches: Vec<String>,
    remember: Remember,
    error: Option<String>,
}

fn read(link: &str) -> Result<Target, String> {
    if nexus::is_nxm(link) {
        return nexus::parse_nxm(link).map(Target::Nexus).map_err(|e: NxmError| e.to_string());
    }
    if thunderstore::is_ror2mm(link) {
        return thunderstore::parse_ror2mm(link).map(Target::Thunderstore);
    }
    Err(tr!("This is not a link to a mod").to_string())
}

/// Matches the links with the installed games. Blocking: reads the library
/// and asks the sites.
fn resolve(paths: &DataPaths, root: &ModsRoot, targets: Vec<Target>) -> Result<(Arc<Library>, Vec<Resolved>), String> {
    let library = Library::load_with(paths, false).map_err(|e| tr!("Could not read the Collection: {error}", error = format!("{e:#}")))?;
    let library = Arc::new(library);
    let installed: Vec<_> = library.games.iter().filter(|g| g.installed && g.launch_kind.as_deref() != Some("emulator")).collect();
    let with_lists: Vec<String> = store::game_keys(root);
    let own_links: Vec<GameLinks> = installed
        .iter()
        .map(|g| if with_lists.contains(&g.key) { store::load(&root.game(&g.key)).links } else { GameLinks::default() })
        .collect();
    let candidates: Vec<Candidate> = installed
        .iter()
        .zip(&own_links)
        .map(|(g, l)| Candidate { title: &g.title, steam_app_id: g.steam_app_id.as_deref(), links: l })
        .collect();
    let cache = root.cache_dir();
    let mut out = Vec::new();
    for target in targets {
        let (site, remember, error) = match &target {
            Target::Nexus(link) => {
                let name = links::nexus_game_name(&cache, &link.domain);
                let site = Site { nexus_domain: Some(link.domain.clone()), communities: Vec::new(), names: name.into_iter().collect() };
                (site, Remember { nexus_domain: Some(link.domain.clone()), thunderstore: None }, None)
            }
            Target::Thunderstore(link) => match thunderstore::package(&link.namespace, &link.name) {
                Ok(package) => {
                    let all = thunderstore::communities(&cache).unwrap_or_default();
                    let names = package
                        .communities
                        .iter()
                        .filter_map(|c| all.iter().find(|a| a.identifier.eq_ignore_ascii_case(c)).map(|a| a.name.clone()))
                        .collect();
                    let remember = Remember { nexus_domain: None, thunderstore: package.communities.first().cloned() };
                    (Site { nexus_domain: None, communities: package.communities, names }, remember, None)
                }
                Err(e) => (Site::default(), Remember::default(), Some(e)),
            },
        };
        let matches = matching(&site, &candidates).into_iter().map(|i| installed[i].key.clone()).collect();
        out.push(Resolved { target, site, matches, remember, error });
    }
    Ok((library, out))
}

fn link_text(target: &Target) -> String {
    match target {
        Target::Nexus(l) => format!("nxm://{}/mods/{}/files/{}", l.domain, l.mod_id, l.file_id),
        Target::Thunderstore(l) => format!("ror2mm://{}/{}", l.namespace, l.name),
    }
}

impl ModsService {
    pub(super) fn receive_link(&mut self, link: String, cx: &mut Context<Self>) {
        match read(&link) {
            Ok(target) => {
                self.links.waiting.push(target);
                self.match_links(cx);
            }
            Err(message) => self.notice("", message, true, cx),
        }
    }

    /// Matches the waiting links, one batch at a time (the library is read once for it).
    fn match_links(&mut self, cx: &mut Context<Self>) {
        if self.links.matching || self.links.waiting.is_empty() {
            return;
        }
        self.links.matching = true;
        let targets = std::mem::take(&mut self.links.waiting);
        let (paths, root) = (self.data_paths.clone(), self.root.clone());
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { resolve(&paths, &root, targets) }).await;
            this.update(cx, |s, cx| {
                s.links.matching = false;
                match result {
                    Ok((library, resolved)) => {
                        for r in resolved {
                            s.route(r, library.clone(), cx);
                        }
                    }
                    Err(message) => s.notice("", message, true, cx),
                }
                s.match_links(cx);
            })
            .ok();
        })
        .detach();
    }

    /// One game: starts the download. Otherwise the user picks.
    fn route(&mut self, resolved: Resolved, library: Arc<Library>, cx: &mut Context<Self>) {
        let Resolved { target, site, matches, remember, error } = resolved;
        if let Some(message) = error {
            self.notice("", message, true, cx);
            return;
        }
        if let [key] = matches.as_slice() {
            self.begin(&target, key, &library, &remember, cx);
            return;
        }
        let id = self.next_job_id();
        let request = LinkRequest { id, link: link_text(&target), site_game: site.names.first().cloned(), candidates: matches };
        self.links.pending.push_back(Pending { request, target, remember, library });
        self.show_next_request(cx);
    }

    fn show_next_request(&mut self, cx: &mut Context<Self>) {
        if self.link_request.is_none() {
            self.link_request = self.links.pending.front().map(|p| p.request.clone());
            if self.link_request.is_some() {
                cx.emit(ModsEvent::PickGame);
            }
        }
    }

    pub(super) fn pick_link_game(&mut self, id: u64, game_key: Option<String>, cx: &mut Context<Self>) {
        let Some(at) = self.links.pending.iter().position(|p| p.request.id == id) else { return };
        let Some(pending) = self.links.pending.remove(at) else { return };
        if self.link_request.as_ref().is_some_and(|r| r.id == id) {
            self.link_request = None;
        }
        if let Some(key) = game_key {
            self.begin(&pending.target, &key, &pending.library, &pending.remember, cx);
        }
        cx.emit(ModsEvent::PickGame);
        self.show_next_request(cx);
    }

    /// Starts the job for the link in `game_key`, shows its manager and
    /// remembers the game's site links.
    fn begin(&mut self, target: &Target, game_key: &str, library: &Library, remember: &Remember, cx: &mut Context<Self>) {
        let Some(game) = library.get(game_key) else {
            self.notice(game_key, tr!("The game of this link is not in the Collection"), true, cx);
            return;
        };
        self.sources.insert(game_key.to_string(), game.clone());
        self.remember_links(game_key, remember.clone(), cx);
        match target {
            Target::Nexus(link) => self.start_nexus_link(game_key, link.clone(), cx),
            Target::Thunderstore(link) => {
                self.start_thunderstore(game_key, &link.namespace, &link.name, link.version.clone(), false, cx);
            }
        }
        cx.emit(ModsEvent::OpenManager { game_key: game_key.to_string() });
    }

    /// Puts the links in the game's list where it has none yet.
    fn remember_links(&mut self, game_key: &str, remember: Remember, cx: &mut Context<Self>) {
        if remember.nexus_domain.is_none() && remember.thunderstore.is_none() {
            return;
        }
        let shared = self.shared_for(game_key);
        let (root, key, title) = (self.root.clone(), game_key.to_string(), self.game_title(game_key));
        let dirs = root.game(game_key);
        cx.spawn(async move |this, cx| {
            let changed = cx
                .background_spawn(async move {
                    shared.ensure_loaded(&root, &key, &title);
                    let (changed, _) = shared.edit(|l| {
                        let before = l.links.clone();
                        l.links.nexus_domain = l.links.nexus_domain.take().or(remember.nexus_domain);
                        l.links.thunderstore = l.links.thunderstore.take().or(remember.thunderstore);
                        l.links != before
                    });
                    if changed {
                        if let Err(e) = shared.save(&dirs) {
                            log::warn!("mods: could not keep the links of {key}: {e}");
                        }
                    }
                    (changed, key)
                })
                .await;
            this.update(cx, |s, cx| {
                if changed.0 {
                    s.refresh_from_shared(&changed.1, cx);
                }
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{matching, read, Candidate, Site, Target};
    use heronite_mods::GameLinks;

    fn none() -> GameLinks {
        GameLinks::default()
    }

    fn cands<'a>(rows: &'a [(&'a str, Option<&'a str>, GameLinks)]) -> Vec<Candidate<'a>> {
        rows.iter().map(|(t, id, l)| Candidate { title: t, steam_app_id: *id, links: l }).collect()
    }

    fn nexus(domain: &str, name: &str) -> Site {
        Site { nexus_domain: Some(domain.into()), communities: Vec::new(), names: vec![name.into()] }
    }

    #[test]
    fn links_are_read_by_scheme() {
        assert!(matches!(read("nxm://skyrimspecialedition/mods/12604/files/35407?key=a&expires=1"), Ok(Target::Nexus(_))));
        assert!(matches!(read("ror2mm://v1/install/thunderstore.io/BepInEx/BepInExPack/5.4.2100/"), Ok(Target::Thunderstore(_))));
        assert!(read("https://example.org").is_err());
        assert!(read("nxm://skyrimspecialedition/collections/abc/revisions/3").is_err());
    }

    #[test]
    fn a_games_own_links_come_first() {
        let linked = GameLinks { nexus_domain: Some("SkyrimSpecialEdition".into()), ..none() };
        let rows = [("My Skyrim", None, linked), ("The Elder Scrolls V: Skyrim Special Edition", Some("489830"), none())];
        assert_eq!(matching(&nexus("skyrimspecialedition", "Skyrim Special Edition"), &cands(&rows)), vec![0]);
    }

    #[test]
    fn the_table_then_the_title_find_the_game() {
        let rows = [("Fallout 4", Some("377160"), none()), ("Skyrim Special Edition", Some("489830"), none())];
        assert_eq!(matching(&nexus("skyrimspecialedition", ""), &cands(&rows)), vec![1]);
        let indie = [("Some Indie Game", None, none()), ("Other", None, none())];
        assert_eq!(matching(&nexus("someindiegame", "Some Indie Game"), &cands(&indie)), vec![0]);
        assert!(matching(&nexus("nothing", "Nothing Here"), &cands(&indie)).is_empty());
    }

    #[test]
    fn two_games_that_fit_both_come_back() {
        let rows = [("Fallout 4", Some("377160"), none()), ("Fallout 4 GOTY", None, none())];
        assert_eq!(matching(&nexus("fallout4", "Fallout 4"), &cands(&rows)), vec![0, 1]);
    }

    #[test]
    fn thunderstore_packages_match_by_community() {
        let ts = GameLinks { thunderstore: Some("risk-of-rain-2".into()), ..none() };
        let rows = [("Risk of Rain 2", None, ts), ("Lethal Company", Some("1966720"), none())];
        let site = Site { nexus_domain: None, communities: vec!["lethal-company".into()], names: vec!["Lethal Company".into()] };
        assert_eq!(matching(&site, &cands(&rows)), vec![1]);
        let both = Site { communities: vec!["risk-of-rain-2".into(), "lethal-company".into()], ..site };
        assert_eq!(matching(&both, &cands(&rows)), vec![0]);
    }
}
