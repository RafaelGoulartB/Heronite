//! The LaunchBox Games Database's adapter: its lookups from the local
//! index ([`crate::launchbox`]), its box art region and shape, its download.

use std::sync::Arc;

use heronite_data::cover_family::{CoverFamilies, CoverFamily};
use heronite_data::paths::DataPaths;
use heronite_i18n::n_;

use super::{candidate, BOX_REGION};
use crate::launchbox::{index::LbGame, map as lb_map, LaunchBox};
use crate::sources::download::{DownloadProgress, DownloadState};
use crate::sources::{MetadataSource, SourceContext, SourceDescriptor, SourceGame};
use crate::types::{Lookup, MatchedBy, MetadataCandidate, SourceHit};

/// A region of the "Box art region" option, as a `tr!` key.
pub(super) fn region_label(region: &str) -> &'static str {
    match region {
        "North America" => n_!("North America"),
        "Europe" => n_!("Europe"),
        "Japan" => n_!("Japan"),
        "World" => n_!("World"),
        "United Kingdom" => n_!("United Kingdom"),
        "Germany" => n_!("Germany"),
        "France" => n_!("France"),
        "Spain" => n_!("Spain"),
        "Italy" => n_!("Italy"),
        "Brazil" => n_!("Brazil"),
        "Korea" => n_!("Korea"),
        "Australia" => n_!("Australia"),
        _ => n_!("Automatic"),
    }
}

/// The box art regions of LaunchBox's descriptor `d`, best first, from its
/// option and the interface language.
pub(crate) fn box_regions(d: &SourceDescriptor, ctx: &SourceContext<'_>) -> Vec<String> {
    let opt = d.options.iter().find(|o| o.key == BOX_REGION).expect("the LaunchBox region option");
    let chosen = crate::sources::options::option_value(ctx.paths, ctx.settings(), d, opt);
    lb_map::region_order(chosen.as_text(), heronite_i18n::current())
}

/// The LaunchBox Games Database, from its local index ([`launchbox`]).
pub(super) struct LaunchBoxSource {
    pub(super) d: &'static SourceDescriptor,
    pub(super) db: Arc<LaunchBox>,
}

impl LaunchBoxSource {
    fn regions(&self, ctx: &SourceContext<'_>) -> Vec<String> {
        box_regions(self.d, ctx)
    }

    /// The family of the box art of `lb`, read from its size only while
    /// this PC keeps the square or the wide family (the tall cover takes
    /// the box art whatever its shape): one small request per game, none
    /// for one read before.
    fn box_family(&self, ctx: &SourceContext<'_>, lb: &LbGame, regions: &[String]) -> Option<CoverFamily> {
        let front = lb_map::box_front(lb, regions)?;
        if !CoverFamilies::load(ctx.paths).kept().iter().any(|f| *f != CoverFamily::Tall) {
            return None;
        }
        let (w, h) = self.db.picture_size(ctx.http, &ctx.http.endpoints.launchbox_images, &front.file)?;
        Some(crate::picture_size::family_of(w, h))
    }

    fn candidate(&self, ctx: &SourceContext<'_>, game: &SourceGame, lb: &LbGame, by: MatchedBy) -> MetadataCandidate {
        let regions = self.regions(ctx);
        let family = self.box_family(ctx, lb, &regions);
        let md = lb_map::record(&game.app_name, &game.runner, lb, &regions, &ctx.http.endpoints.launchbox_images, family);
        candidate(self.d, by)(md).with_id(Some(lb.id.to_string()))
    }
}

impl MetadataSource for LaunchBoxSource {
    fn descriptor(&self) -> &SourceDescriptor {
        self.d
    }

    fn lookup(&self, ctx: &SourceContext<'_>, game: &SourceGame) -> Lookup<MetadataCandidate> {
        self.db.lookup(ctx.paths, game).map(|(lb, by)| self.candidate(ctx, game, &lb, by))
    }

    fn lookup_id(&self, ctx: &SourceContext<'_>, game: &SourceGame, id: &str) -> Lookup<MetadataCandidate> {
        self.db.by_id(ctx.paths, id).map(|lb| self.candidate(ctx, game, &lb, MatchedBy::SourceId))
    }

    fn search(&self, ctx: &SourceContext<'_>, query: &str) -> Lookup<Vec<SourceHit>> {
        let regions = self.regions(ctx);
        let base = &ctx.http.endpoints.launchbox_images;
        self.db.search(ctx.paths, query).map(|games| games.iter().map(|g| lb_map::hit(g, &regions, base)).collect())
    }

    fn download_state(&self, paths: &DataPaths) -> DownloadState {
        self.db.state(paths)
    }

    fn download(&self, ctx: &SourceContext<'_>, progress: &DownloadProgress) -> anyhow::Result<()> {
        self.db.download(ctx.paths, ctx.http, progress)
    }

    fn remove_download(&self, ctx: &SourceContext<'_>) -> anyhow::Result<()> {
        self.db.remove(ctx.paths)
    }
}
