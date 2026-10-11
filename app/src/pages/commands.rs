//! Commands a page offers to the command palette (Ctrl K): "Pause
//! downloads", "Install the latest GE-Proton", "Sign in to Epic Games"...
//!
//! Each page implements [`PaletteCommands`] in its own `palette.rs`, with
//! access to its private state. The Collection lists the commands every time
//! the palette opens, also for pages that were not shown yet (`page` is
//! `None` then: list from the services and the [`CommandCtx`], so no page is
//! created just to be listed), and runs the chosen one on the page, creating
//! it first when needed.

use gpui_kit::*;
use heronite_data::settings::EffectiveSettings;

use crate::icons::Icon;

#[derive(Clone, Debug, PartialEq)]
pub struct PageCommand {
    /// Handed back to [`PaletteCommands::run_palette_command`]; stable, as
    /// the palette keeps it in its history.
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub icon: Icon,
    /// Extra search words.
    pub keywords: &'static str,
    /// Why it cannot run now: listed dimmed with this text instead of hidden,
    /// so the command can still be found.
    pub disabled: Option<String>,
    /// Shows the page when it runs (otherwise it runs where the user is).
    pub navigate: bool,
}

impl PageCommand {
    pub fn new(id: impl Into<String>, title: impl Into<String>, subtitle: impl Into<String>, icon: Icon) -> Self {
        Self { id: id.into(), title: title.into(), subtitle: subtitle.into(), icon, keywords: "", disabled: None, navigate: true }
    }

    pub fn keywords(mut self, keywords: &'static str) -> Self {
        self.keywords = keywords;
        self
    }

    pub fn disabled(mut self, reason: Option<impl Into<String>>) -> Self {
        self.disabled = reason.map(Into::into);
        self
    }

    /// Runs without leaving the current page.
    pub fn stay(mut self) -> Self {
        self.navigate = false;
        self
    }
}

/// What the Collection knows that the commands of a page not created yet
/// may need.
pub struct CommandCtx<'a> {
    /// `local_library/settings.json` as the Collection loaded it.
    pub settings: Option<&'a EffectiveSettings>,
}

pub trait PaletteCommands: Sized + 'static {
    /// The commands offered now; `page` is `None` while the page was never
    /// shown.
    fn palette_commands(page: Option<&Self>, ctx: &CommandCtx, cx: &App) -> Vec<PageCommand>;

    /// Runs `id` (one of the listed commands). A page that has not loaded
    /// its data yet keeps the command and runs it once it has.
    fn run_palette_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>);

    /// A dialog or question of the page is open: global shortcuts and the
    /// palette wait for it.
    fn page_modal(&self) -> bool {
        false
    }
}
