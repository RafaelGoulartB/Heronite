//! The sections of the Settings page: labels, navigation groups, palette
//! keywords and the `HERONITE_DEBUG_PAGE` names.

use heronite_i18n::{n_, tr};

use crate::dialogs::settings_panel::GlobalView;
use crate::icons::Icon;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    General,
    Appearance,
    /// Console mode's themes and options (also ☰ → Customize).
    ConsoleMode,
    Advanced,
    GameDefaults,
    Downloads,
    Library,
    /// Heroic, Lutris, Bottles and Faugus: bringing their games in.
    Launchers,
    /// What the games take on disk, and cleaning it up.
    Storage,
    /// The in-game overlay (Guide button).
    Overlay,
    /// The replay buffer: the last seconds of a game, saved on request.
    Replay,
    Metadata,
    /// The RetroAchievements account, the optional Steam Web API key and
    /// how achievements show.
    Achievements,
    Screenshots,
    /// Ludusavi: game save backups and restoring them.
    Saves,
    /// Backups of the data folder and the app settings, and restoring them.
    Backups,
    Logs,
    About,
}

impl Section {
    /// Left navigation: caption + sections (the caption is translated when drawn).
    pub const GROUPS: [(&'static str, &'static [Section]); 6] = [
        (n_!("App"), &[Section::General, Section::Appearance, Section::ConsoleMode]),
        (n_!("Games"), &[Section::GameDefaults, Section::Downloads, Section::Launchers]),
        (n_!("Collection"), &[Section::Library, Section::Metadata, Section::Achievements]),
        (n_!("While playing"), &[Section::Overlay, Section::Screenshots, Section::Replay]),
        (n_!("Data"), &[Section::Storage, Section::Saves, Section::Backups]),
        (n_!("System"), &[Section::Advanced, Section::Logs, Section::About]),
    ];

    /// Same order as [`Section::GROUPS`].
    pub const ALL: [Section; 18] = [
        Section::General,
        Section::Appearance,
        Section::ConsoleMode,
        Section::GameDefaults,
        Section::Downloads,
        Section::Launchers,
        Section::Library,
        Section::Metadata,
        Section::Achievements,
        Section::Overlay,
        Section::Screenshots,
        Section::Replay,
        Section::Storage,
        Section::Saves,
        Section::Backups,
        Section::Advanced,
        Section::Logs,
        Section::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::General => tr!("General"),
            Section::Appearance => tr!("Appearance"),
            Section::ConsoleMode => tr!("Console mode"),
            Section::Advanced => tr!("Advanced"),
            Section::GameDefaults => tr!("Game defaults"),
            Section::Downloads => tr!("Downloads"),
            Section::Library => tr!("Library"),
            Section::Launchers => tr!("Other launchers"),
            Section::Storage => tr!("Storage"),
            Section::Overlay => tr!("Overlay"),
            Section::Replay => tr!("Replay"),
            Section::Metadata => tr!("Metadata"),
            Section::Achievements => tr!("Achievements"),
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
            Section::ConsoleMode => Icon::Tv,
            Section::Advanced => Icon::Build,
            Section::GameDefaults => Icon::Tune,
            Section::Downloads => Icon::Download,
            Section::Library => Icon::GridView,
            Section::Launchers => Icon::CallMerge,
            Section::Storage => Icon::FaHardDrive,
            Section::Overlay => Icon::SportsEsports,
            Section::Replay => Icon::Videocam,
            Section::Metadata => Icon::TravelExplore,
            Section::Achievements => Icon::Trophy,
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
            Section::Appearance => tr!("Profiles, the theme and its colours, menus and panels, the picture behind the Collection, covers, text and layout."),
            Section::ConsoleMode => tr!("The controller-first TV mode: its themes, tabs and shelves, covers, background, interface and controls. Changes show at once; ☰ → Customize in console mode has the same options."),
            Section::Advanced => tr!("Logs, folders and cache."),
            Section::GameDefaults => tr!("Values new games start with. Settings of an installed game are changed from its own Settings."),
            Section::Downloads => tr!("Downloads and updates of Epic Games, GOG and Amazon Games games."),
            Section::Library => tr!("What the Collection shows: hidden games, favourites and recent games."),
            Section::Launchers => tr!("Bring in the games of Heroic, Lutris, Bottles and Faugus. While on, a game added there shows up here too."),
            Section::Storage => tr!("What your games take on disk, and what can go."),
            Section::Overlay => tr!("A panel the controller's Guide button or a keyboard shortcut opens over a game launched by Heronite: screenshots, volume, quitting the game."),
            Section::Replay => tr!("The last seconds of a game kept in memory and saved as a moment with Guide + X, and the memory that takes."),
            Section::Metadata => tr!("Collection uses IGDB first, then Steam and store pages to fill covers, descriptions and tags."),
            Section::Achievements => tr!("Achievements from Steam, GOG, Epic Games and RetroAchievements. Heronite keeps its own copy of them, so your history stays even if a store or an account goes away."),
            Section::Screenshots => tr!("Where your captures live, the in-game shortcut and the preview cache of the gallery."),
            Section::Saves => tr!("Game saves backed up with Ludusavi after you close a game, and the folders a game's Saves tab restores them from."),
            Section::Backups => tr!("Scheduled backups of your Collection, play history and settings, and putting one back."),
            Section::Logs => tr!("The logs of this app, the runners and every game you launched. Switch logging off in General (Disable logs) or per game (Enable verbose logs)."),
            Section::About => tr!("Updates, this app's data folders and the system it runs on."),
        }
    }

    /// Extra words the command palette matches.
    pub fn keywords(self) -> &'static str {
        match self {
            Section::General => "general folders tray shortcuts wine proton language theme geral pastas bandeja atalhos idioma tema",
            Section::Appearance => "appearance background blur dim darkening saturation cover aspect grey uninstalled artwork glass opacity transparent transparency solid interface panels menus dialogs colour tint collection accent contrast font density corners badges hover screenshot slideshow picture animations motion layout sections profile theme light dark mode system import export aparencia plano de fundo desfoque escurecer saturacao capa cinza nao instalados arte vidro opacidade transparente transparencia solido paineis cor colecao destaque contraste fonte densidade cantos selos estrela animacoes movimento secoes perfil tema claro escuro modo sistema importar exportar imagem",
            Section::ConsoleMode => "console mode tv couch controller gamepad theme themes tabs shelves layout covers background font scale clock accent colours glyphs hints menu startup customize export import preset modo console televisao sofa controle tema temas abas prateleiras capas fundo fonte tamanho relogio destaque cores botoes dicas menu personalizar exportar importar perfil",
            Section::Advanced => "advanced cache folders copy settings avancado pastas copiar configuracoes",
            Section::GameDefaults => "game defaults wine prefix gamescope launch options new games padroes dos jogos prefixo opcoes de inicializacao novos jogos",
            Section::Downloads => "downloads epic updates install atualizacoes instalar baixar",
            Section::Library => "library hidden favourites favorites recent biblioteca ocultos favoritos recentes",
            Section::Launchers => "other launchers import heroic lutris bottles faugus sync bring in outros launchers importar sincronizar trazer",
            Section::Storage => "storage disk space free up clean cleanup size prefixes proton cache leftovers uninstall armazenamento disco espaco liberar limpar limpeza tamanho prefixos sobras desinstalar",
            Section::Overlay => "overlay guide button controller gamepad keyboard in-game volume screenshot shortcut beta botao guia controle teclado sobreposicao no jogo volume captura atalho",
            Section::Replay => "replay clip video record recording buffer last seconds guide gpu-screen-recorder memory ram vram vulkan nvenc vaapi encoder disk gravar gravacao clipe video ultimos segundos memoria codificador disco",
            Section::Achievements => "achievements trophies retroachievements retro emulator rom steam gog epic unlocked rare points hardcore web api key conquistas trofeus emulador desbloqueadas raras pontos chave",
            Section::Metadata => "metadata igdb twitch steam lutris covers fill download images priority metadados capas preencher baixar imagens prioridade",
            Section::Screenshots => "screenshots capture shortcut hotkey folder thumbnails cache gallery capturas de tela atalho pasta miniaturas galeria",
            Section::Saves => "saves ludusavi backup restore retention locked differential zip compression game list manifest salvamentos restaurar retencao travado diferencial compressao lista de jogos",
            Section::Backups => "backups backup restore data folder config copy schedule retention sign-ins artwork pasta de dados copia agendar agendamento restaurar retencao logins arte",
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
            "console-mode" | "console" => Some(Section::ConsoleMode),
            "advanced" => Some(Section::Advanced),
            "defaults" => Some(Section::GameDefaults),
            "downloads" => Some(Section::Downloads),
            "library" => Some(Section::Library),
            "launchers" | "import" => Some(Section::Launchers),
            "storage" | "disk" => Some(Section::Storage),
            "overlay" => Some(Section::Overlay),
            "replay" => Some(Section::Replay),
            "metadata" => Some(Section::Metadata),
            "achievements" | "retroachievements" => Some(Section::Achievements),
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
        assert_eq!(Section::parse("storage"), Some(Section::Storage));
        assert_eq!(Section::parse("defaults"), Some(Section::GameDefaults));
        assert_eq!(Section::parse("replay"), Some(Section::Replay));
        assert_eq!(Section::parse("console-mode"), Some(Section::ConsoleMode));
        assert_eq!(Section::parse("nope"), None);
        assert_eq!(crate::pages::Page::from_debug("settings"), Some(crate::pages::Page::Settings));
        assert_eq!(crate::pages::Page::from_debug("settings:about"), Some(crate::pages::Page::Settings));
        assert_eq!(crate::pages::Page::from_debug("settingsx"), None);
    }
}
