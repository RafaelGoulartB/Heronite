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
    /// Heroic, Lutris, Bottles and Faugus (Heroic and Playnite on Windows): bringing
    /// their games in.
    Launchers,
    /// Sunshine: the games in its application list, for Moonlight.
    Integrations,
    /// What the games take on disk, and cleaning it up.
    Storage,
    /// Mods: the Nexus Mods account, the mod links, what applying mods needs.
    Mods,
    /// What every controller button does, where, and what can change
    /// (`controls`).
    Controls,
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
    /// The account the library sync, backups, game saves and screenshots go
    /// to (`heronite_sync`).
    Cloud,
    Logs,
    About,
}

impl Section {
    /// Left navigation: caption + sections (the caption is translated when drawn).
    pub const GROUPS: [(&'static str, &'static [Section]); 6] = [
        (n_!("App"), &[Section::General, Section::Appearance, Section::ConsoleMode]),
        (n_!("Games"), &[Section::GameDefaults, Section::Downloads, Section::Launchers, Section::Integrations, Section::Mods]),
        (n_!("Collection"), &[Section::Library, Section::Metadata, Section::Achievements]),
        (n_!("While playing"), &[Section::Controls, Section::Overlay, Section::Screenshots, Section::Replay]),
        (n_!("Data"), &[Section::Storage, Section::Saves, Section::Backups, Section::Cloud]),
        (n_!("System"), &[Section::Advanced, Section::Logs, Section::About]),
    ];

    /// Same order as [`Section::GROUPS`].
    pub const ALL: [Section; 22] = [
        Section::General,
        Section::Appearance,
        Section::ConsoleMode,
        Section::GameDefaults,
        Section::Downloads,
        Section::Launchers,
        Section::Integrations,
        Section::Mods,
        Section::Library,
        Section::Metadata,
        Section::Achievements,
        Section::Controls,
        Section::Overlay,
        Section::Screenshots,
        Section::Replay,
        Section::Storage,
        Section::Saves,
        Section::Backups,
        Section::Cloud,
        Section::Advanced,
        Section::Logs,
        Section::About,
    ];

    /// Whether this system has what the section is about
    /// (`heronite_platform::features`).
    pub fn available(self) -> bool {
        use heronite_platform::{supported, Feature};
        match self {
            Section::Downloads => heronite_platform::features::any_store(),
            Section::Launchers => supported(Feature::LauncherImports),
            Section::Storage => supported(Feature::Storage),
            Section::Overlay => supported(Feature::Overlay),
            Section::Replay => supported(Feature::Replay),
            Section::Screenshots => supported(Feature::Capture),
            Section::Saves => supported(Feature::SaveBackups),
            Section::Mods => supported(Feature::Mods),
            // Its options are mostly Wine's.
            Section::GameDefaults => supported(Feature::Wine),
            _ => true,
        }
    }

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
            Section::Integrations => tr!("Integrations"),
            Section::Storage => tr!("Storage"),
            Section::Mods => tr!("Mods"),
            Section::Controls => tr!("Controller"),
            Section::Overlay => tr!("Overlay"),
            Section::Replay => tr!("Replay"),
            Section::Metadata => tr!("Metadata"),
            Section::Achievements => tr!("Achievements"),
            Section::Screenshots => tr!("Screenshots"),
            Section::Saves => tr!("Saves"),
            Section::Backups => tr!("Backups"),
            Section::Cloud => tr!("Cloud"),
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
            Section::Integrations => Icon::Tv,
            Section::Storage => Icon::FaHardDrive,
            Section::Mods => Icon::CallMerge,
            Section::Controls => Icon::SportsEsports,
            Section::Overlay => Icon::SportsEsports,
            Section::Replay => Icon::Videocam,
            Section::Metadata => Icon::TravelExplore,
            Section::Achievements => Icon::Trophy,
            Section::Screenshots => Icon::Fullscreen,
            Section::Saves => Icon::Save,
            Section::Backups => Icon::History,
            Section::Cloud => Icon::CloudSync,
            Section::Logs => Icon::Description,
            Section::About => Icon::Memory,
        }
    }

    /// Short description under the section title.
    pub(super) fn blurb(self) -> &'static str {
        match self {
            Section::General => tr!("Language, folders, the tray, shortcuts and where Wine and Proton are looked for."),
            Section::Appearance => tr!("Profiles, the theme and its colours, menus and panels, the picture behind the Collection, covers, text and layout."),
            Section::ConsoleMode => tr!("The controller-first TV mode: its themes, tabs and shelves, covers, background, interface and controls. Changes show at once; ☰ → Customize in console mode has the same options."),
            Section::Advanced => tr!("Logs, folders and cache."),
            Section::GameDefaults => tr!("What new games start with. To change a game you already have, open its own settings."),
            Section::Downloads => tr!("Downloads and updates of Epic Games, GOG and Amazon Games games."),
            Section::Library => tr!("What the Collection shows: hidden games, favourites and recent games."),
            Section::Launchers if cfg!(windows) => {
                tr!("Bring in the games of Heroic and Playnite. While on, a game added there shows up here too.")
            }
            Section::Launchers => tr!("Bring in the games of Heroic, Lutris, Bottles and Faugus. While on, a game added there shows up here too."),
            Section::Integrations => tr!("Sunshine keeps your games in its list, with their covers, so Moonlight can start them from a phone, a TV or another computer."),
            Section::Storage => tr!("What your games take on disk, and what can go."),
            Section::Mods => tr!("Your Nexus Mods account, the download buttons of the mod sites, and what Heronite needs to apply mods when a game starts. Each game's mods are managed from its own Mods."),
            Section::Controls => tr!("What every button does, where it works, and what you can change."),
            Section::Overlay => crate::overlay::labels::section_blurb(),
            Section::Replay => tr!("The last seconds of a game kept in memory and saved as a moment with Guide + X, and the memory that takes."),
            Section::Metadata => tr!("Where the details and pictures of your games come from. Sources are asked from the top of each list down; a field stays with the first one that has it."),
            Section::Achievements => tr!("Achievements from Steam, GOG, Epic Games and RetroAchievements. Heronite keeps its own copy of them, so your history stays even if a store or an account goes away."),
            Section::Screenshots => tr!("Where your captures live, the in-game shortcut and the preview cache of the gallery."),
            Section::Saves => tr!("Game saves backed up with Ludusavi after you close a game, and the folders a game's Saves tab restores them from."),
            Section::Backups => tr!("Scheduled backups of your Collection, play history and settings, and putting one back."),
            Section::Cloud => tr!("One account for the library synced between your PCs, your backups, game saves and screenshots: Google Drive, Dropbox, OneDrive, a server or a shared folder."),
            Section::Logs => tr!("The logs of this app, the runners and every game you launched. Logging is turned off in Advanced (Turn off logs) and made more detailed per game (Detailed logs)."),
            Section::About => tr!("Updates, help and feedback, this app's data folders and the system it runs on."),
        }
    }

    /// Extra words the command palette matches.
    pub fn keywords(self) -> &'static str {
        match self {
            Section::General => "general folders tray shortcuts wine proton language theme geral pastas bandeja atalhos idioma tema",
            Section::Appearance => "appearance background blur dim darkening saturation cover aspect grey uninstalled artwork glass opacity transparent transparency solid interface panels menus dialogs colour tint collection accent contrast zoom scale scaling size bigger smaller larger dpi hidpi 4k tv accessibility font density corners badges categories category tags ribbon strip bookmarks hover screenshot slideshow picture animations motion layout sections profile theme light dark mode system import export aparencia plano de fundo desfoque escurecer saturacao capa cinza nao instalados arte vidro opacidade transparente transparencia solido paineis cor colecao destaque contraste zoom escala tamanho maior menor ampliar aumentar diminuir acessibilidade fonte densidade cantos selos categorias categoria etiquetas fita faixa marcadores estrela animacoes movimento secoes perfil tema claro escuro modo sistema importar exportar imagem",
            Section::ConsoleMode => "console mode tv couch controller gamepad theme themes tabs shelves layout covers background font scale clock accent colours glyphs hints menu startup customize export import preset modo console televisao sofa controle tema temas abas prateleiras capas fundo fonte tamanho relogio destaque cores botoes dicas menu personalizar exportar importar perfil",
            Section::Advanced => "advanced cache folders copy settings avancado pastas copiar configuracoes",
            Section::GameDefaults => "game defaults wine proton version prefix dxvk nvapi esync fsync sync wayland wow64 gpu performance fps gamescope launch options new games padroes dos jogos versao prefixo desempenho opcoes de inicializacao novos jogos",
            Section::Downloads => "downloads epic updates install atualizacoes instalar baixar",
            Section::Library => "library hidden favourites favorites recent biblioteca ocultos favoritos recentes",
            Section::Launchers => "other launchers import heroic lutris bottles faugus playnite sync bring in outros launchers importar sincronizar trazer",
            Section::Integrations => "integrations sunshine moonlight stream streaming remote play tv phone couch integracoes transmitir transmissao jogar remotamente celular sofa",
            Section::Storage => "storage disk space free up clean cleanup size prefixes proton cache leftovers uninstall armazenamento disco espaco liberar limpar limpeza tamanho prefixos sobras desinstalar",
            Section::Mods => "mods nexus thunderstore nxm ror2mm api key account premium links fuse overlayfs fomod mod manager modding mods conta chave link modificacoes gerenciador",
            Section::Controls => "controls controller gamepad buttons shortcuts guide back start chord bindings console checks steam input conflict controles controle botoes atalhos conflito",
            Section::Overlay => "overlay guide button controller gamepad keyboard in-game volume screenshot shortcut botao guia controle teclado sobreposicao no jogo volume captura atalho",
            Section::Replay => "replay clip video record recording buffer last seconds guide gpu-screen-recorder memory ram vram vulkan nvenc vaapi encoder disk gravar gravacao clipe video ultimos segundos memoria codificador disco",
            Section::Achievements => "achievements trophies retroachievements retro emulator rom steam gog epic unlocked rare points hardcore web api key conquistas trofeus emulador desbloqueadas raras pontos chave",
            Section::Metadata => "metadata sources igdb twitch steam lutris steamgriddb howlongtobeat protondb tags covers backgrounds fill download update images order priority age ratings metadados fontes capas fundos preencher baixar atualizar imagens ordem prioridade classificacao etaria",
            Section::Screenshots => "screenshots capture shortcut hotkey folder thumbnails cache gallery capturas de tela atalho pasta miniaturas galeria",
            Section::Saves => "saves ludusavi backup restore retention locked differential zip compression game list manifest salvamentos restaurar retencao travado diferencial compressao lista de jogos",
            Section::Cloud => "cloud sync synchronize upload backup saves screenshots google drive dropbox onedrive box ftp smb webdav rclone syncthing dual boot windows linux other pc devices space restore sincronizar sincronizacao nuvem outro computador dispositivos enviar backup capturas espaco restaurar",
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
            "integrations" | "sunshine" => Some(Section::Integrations),
            "storage" | "disk" => Some(Section::Storage),
            "mods" | "nexus" => Some(Section::Mods),
            "controls" | "shortcuts" | "controller" => Some(Section::Controls),
            "overlay" => Some(Section::Overlay),
            "replay" => Some(Section::Replay),
            "metadata" => Some(Section::Metadata),
            "achievements" | "retroachievements" => Some(Section::Achievements),
            "screenshots" => Some(Section::Screenshots),
            "saves" | "ludusavi" => Some(Section::Saves),
            "backups" | "backup" => Some(Section::Backups),
            "cloud" | "sync" => Some(Section::Cloud),
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
        assert_eq!(Section::parse("mods"), Some(Section::Mods));
        assert_eq!(Section::parse("console-mode"), Some(Section::ConsoleMode));
        assert_eq!(Section::parse("sunshine"), Some(Section::Integrations));
        assert_eq!(Section::parse("controls"), Some(Section::Controls));
        assert_eq!(Section::parse("nope"), None);
        assert_eq!(crate::pages::Page::from_debug("settings"), Some(crate::pages::Page::Settings));
        assert_eq!(crate::pages::Page::from_debug("settings:about"), Some(crate::pages::Page::Settings));
        assert_eq!(crate::pages::Page::from_debug("settingsx"), None);
    }
}
