//! The sections of the Settings page: labels, navigation groups, palette
//! keywords and the `HERONITE_DEBUG_PAGE` names.

use heronite_i18n::{n_, tr};

use crate::dialogs::settings_panel::GlobalView;
use crate::icons::Icon;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    General,
    Appearance,
    Advanced,
    GameDefaults,
    Downloads,
    Library,
    /// The in-game overlay (Guide button).
    Overlay,
    Metadata,
    Screenshots,
    /// Ludusavi: game save backups and restoring them.
    Saves,
    /// The scheduled copy of the data folder.
    Backups,
    Logs,
    About,
}

impl Section {
    /// Left navigation: caption + sections (the caption is translated when drawn).
    pub const GROUPS: [(&'static str, &'static [Section]); 4] = [
        (n_!("App"), &[Section::General, Section::Appearance, Section::Advanced]),
        (n_!("Games"), &[Section::GameDefaults, Section::Downloads, Section::Library, Section::Overlay]),
        (n_!("Collection"), &[Section::Metadata, Section::Screenshots, Section::Saves, Section::Backups]),
        (n_!("Help"), &[Section::Logs, Section::About]),
    ];

    /// Same order as [`Section::GROUPS`].
    pub const ALL: [Section; 13] = [
        Section::General,
        Section::Appearance,
        Section::Advanced,
        Section::GameDefaults,
        Section::Downloads,
        Section::Library,
        Section::Overlay,
        Section::Metadata,
        Section::Screenshots,
        Section::Saves,
        Section::Backups,
        Section::Logs,
        Section::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::General => tr!("General"),
            Section::Appearance => tr!("Appearance"),
            Section::Advanced => tr!("Advanced"),
            Section::GameDefaults => tr!("Game defaults"),
            Section::Downloads => tr!("Downloads"),
            Section::Library => tr!("Library"),
            Section::Overlay => tr!("Overlay"),
            Section::Metadata => tr!("Metadata"),
            Section::Screenshots => tr!("Screenshots"),
            Section::Saves => tr!("Saves"),
            Section::Backups => tr!("Backups"),
            Section::Logs => tr!("Logs"),
            Section::About => tr!("About"),
        }
    }

    pub(super) fn icon(self) -> Icon {
        match self {
            Section::General => Icon::Settings,
            Section::Appearance => Icon::Visibility,
            Section::Advanced => Icon::Build,
            Section::GameDefaults => Icon::Tune,
            Section::Downloads => Icon::Download,
            Section::Library => Icon::GridView,
            Section::Overlay => Icon::SportsEsports,
            Section::Metadata => Icon::TravelExplore,
            Section::Screenshots => Icon::Fullscreen,
            Section::Saves => Icon::Save,
            Section::Backups => Icon::History,
            Section::Logs => Icon::Description,
            Section::About => Icon::Memory,
        }
    }

    /// Short description under the section title.
    pub(super) fn blurb(self) -> &'static str {
        match self {
            Section::General => tr!("Folders, tray and shortcuts, Wine and Proton lookup. Saved automatically."),
            Section::Appearance => tr!("Two independent looks: the artwork behind the Collection, and the background of menus, dialogs and pages (glass or solid)."),
            Section::Advanced => tr!("Logs, folders and cache."),
            Section::GameDefaults => tr!("Values new games start with. Settings of an installed game are changed from its own Settings."),
            Section::Downloads => tr!("Downloads and updates of Epic Games, GOG and Amazon Games games."),
            Section::Library => tr!("What the Collection shows: hidden games, favourites and recent games."),
            Section::Overlay => tr!("A panel the controller's Guide button opens over a game launched by Heronite: screenshots, volume, quitting the game."),
            Section::Metadata => tr!("Collection uses IGDB first, then Steam, store pages and Lutris to fill covers, descriptions and tags."),
            Section::Screenshots => tr!("Where your captures live, the in-game shortcut and the preview cache of the gallery."),
            Section::Saves => tr!("Game saves backed up with Ludusavi after you close a game, and the folders a game's Saves tab restores them from."),
            Section::Backups => tr!("A scheduled copy of the data folder."),
            Section::Logs => tr!("The logs of this app, the runners and every game you launched. Switch logging off in General (Disable logs) or per game (Enable verbose logs)."),
            Section::About => tr!("Updates, this app's data folders and the system it runs on."),
        }
    }

    /// Extra words the command palette matches.
    pub fn keywords(self) -> &'static str {
        match self {
            Section::General => "general folders tray shortcuts wine proton language theme geral pastas bandeja atalhos idioma tema",
            Section::Appearance => "appearance background blur dim darkening saturation cover aspect grey uninstalled artwork glass opacity transparent transparency solid interface panels menus dialogs colour tint collection aparencia plano de fundo desfoque escurecer saturacao capa cinza nao instalados arte vidro opacidade transparente transparencia solido paineis cor colecao",
            Section::Advanced => "advanced cache folders copy settings avancado pastas copiar configuracoes",
            Section::GameDefaults => "game defaults wine prefix gamescope launch options new games padroes dos jogos prefixo opcoes de inicializacao novos jogos",
            Section::Downloads => "downloads epic updates install atualizacoes instalar baixar",
            Section::Library => "library hidden favourites favorites recent biblioteca ocultos favoritos recentes",
            Section::Overlay => "overlay guide button controller gamepad in-game volume screenshot shortcut beta botao guia controle sobreposicao no jogo volume captura atalho",
            Section::Metadata => "metadata igdb twitch steam lutris covers fill download images priority metadados capas preencher baixar imagens prioridade",
            Section::Screenshots => "screenshots capture shortcut hotkey folder thumbnails cache gallery capturas de tela atalho pasta miniaturas galeria",
            Section::Saves => "saves ludusavi backup restore retention locked differential zip compression game list manifest salvamentos restaurar retencao travado diferencial compressao lista de jogos",
            Section::Backups => "backups backup data folder config copy schedule pasta de dados copia agendar agendamento",
            Section::Logs => "logs log viewer debug registros visualizador depurar",
            Section::About => "about version system info update upgrade new release changelog sobre versao sistema atualizacao atualizar novidades",
        }
    }

    pub(super) fn global_view(self) -> Option<GlobalView> {
        match self {
            Section::General => Some(GlobalView::General),
            Section::Downloads => Some(GlobalView::Downloads),
            Section::Advanced => Some(GlobalView::Advanced),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> Option<Section> {
        match s {
            "general" => Some(Section::General),
            "appearance" => Some(Section::Appearance),
            "advanced" => Some(Section::Advanced),
            "defaults" => Some(Section::GameDefaults),
            "downloads" => Some(Section::Downloads),
            "library" => Some(Section::Library),
            "overlay" => Some(Section::Overlay),
            "metadata" => Some(Section::Metadata),
            "screenshots" => Some(Section::Screenshots),
            "saves" | "ludusavi" => Some(Section::Saves),
            "backups" | "backup" => Some(Section::Backups),
            "logs" => Some(Section::Logs),
            "about" => Some(Section::About),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Section;

    #[test]
    fn sections_parse_for_debug_and_settings_page_is_a_debug_page() {
        for s in Section::ALL {
            assert!(!s.label().is_empty());
        }
        // The nav groups list every section once, in `ALL` order.
        let grouped: Vec<Section> = Section::GROUPS.iter().flat_map(|(_, s)| s.iter().copied()).collect();
        assert_eq!(grouped, Section::ALL);
        assert_eq!(Section::parse("backups"), Some(Section::Backups));
        assert_eq!(Section::parse("saves"), Some(Section::Saves));
        assert_eq!(Section::parse("metadata"), Some(Section::Metadata));
        assert_eq!(Section::parse("library"), Some(Section::Library));
        assert_eq!(Section::parse("defaults"), Some(Section::GameDefaults));
        assert_eq!(Section::parse("nope"), None);
        assert_eq!(crate::pages::Page::from_debug("settings"), Some(crate::pages::Page::Settings));
        assert_eq!(crate::pages::Page::from_debug("settings:about"), Some(crate::pages::Page::Settings));
        assert_eq!(crate::pages::Page::from_debug("settingsx"), None);
    }
}
