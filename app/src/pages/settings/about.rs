//! Settings → About: updates (`updates.rs`); links to report a problem,
//! suggest an idea, ask and star the project on GitHub; version, data folders
//! and system information.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;

use super::sysinfo::{self, SystemInfo};
use super::SettingsPage;
use crate::icons::Icon;
use crate::pages::PageCtx;
use crate::theme;
use crate::ui::controls::{action_button, caption, card, icon_action_button, row, ButtonKind};

const REPO: &str = heronite_update::REPO_PAGE;

#[derive(Default)]
pub struct State {
    loaded: bool,
    system: SystemInfo,
    legendary: Option<String>,
    wine_count: Option<usize>,
    proton_count: Option<usize>,
    copied: bool,
}

/// `legendary --version` first line, from the bundled/alternative binary.
fn legendary_version(data_paths: &heronite_data::DataPaths) -> Option<String> {
    let lg = heronite_epic::cmd::Legendary::locate(data_paths).ok()?;
    let out = lg.run(&["--version"], &[]).ok()?;
    let line = out.stdout.lines().find(|l| !l.trim().is_empty())?.trim().to_string();
    Some(line)
}

impl State {
    pub fn load(&mut self, ctx: &PageCtx, cx: &mut Context<SettingsPage>) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let data_paths = ctx.data_paths.clone();
        let work = move || {
            let system = sysinfo::gather();
            let installs = heronite_wine::list_installations(&data_paths);
            let proton = installs.iter().filter(|w| w.kind == "proton").count();
            (system, legendary_version(&data_paths), installs.len(), proton)
        };
        // Debug captures only keep the first frames: gather in place there.
        if crate::env::value("HERONITE_DEBUG_PAGE").is_some() {
            self.apply(work());
            return;
        }
        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { work() }).await;
            this.update(cx, |this, cx| {
                this.about.apply(result);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn apply(&mut self, (system, legendary, wine_all, proton): (SystemInfo, Option<String>, usize, usize)) {
        self.system = system;
        self.legendary = legendary;
        self.wine_count = Some(wine_all - proton);
        self.proton_count = Some(proton);
    }

    /// The report copied to the clipboard.
    fn report(&self, ctx: &PageCtx) -> String {
        let s = &self.system;
        let mut out = vec![
            format!("Heronite {}", env!("CARGO_PKG_VERSION")),
            format!("OS: {} (kernel {})", s.os, s.kernel),
            format!("CPU: {} ({} threads)", s.cpu, s.cores),
            format!("Memory: {}", sysinfo::human_memory(s.memory_kb)),
        ];
        for g in &s.gpus {
            out.push(format!(
                "GPU: {} {} (driver {}{})",
                g.vendor,
                g.name,
                g.driver,
                g.driver_version.as_deref().map(|v| format!(" {v}")).unwrap_or_default()
            ));
        }
        if !s.session.is_empty() {
            out.push(format!("Session: {}", s.session));
        }
        out.push(format!("Legendary: {}", self.legendary.as_deref().unwrap_or("not found")));
        out.push(format!("Data folder: {}", ctx.data_paths.root.display()));
        out.join("\n")
    }
}

/// Where each feedback link goes on GitHub.
#[derive(Clone, Copy)]
enum Link {
    Problem,
    Idea,
    Question,
    Star,
}

impl Link {
    const ALL: [Link; 4] = [Link::Problem, Link::Idea, Link::Question, Link::Star];

    fn id(self) -> &'static str {
        match self {
            Link::Problem => "problem",
            Link::Idea => "idea",
            Link::Question => "question",
            Link::Star => "star",
        }
    }

    fn from_id(id: &str) -> Option<Link> {
        Link::ALL.into_iter().find(|l| l.id() == id)
    }

    fn label(self) -> &'static str {
        match self {
            Link::Problem => tr!("Report a problem"),
            Link::Idea => tr!("Suggest an idea"),
            Link::Question => tr!("Ask a question"),
            Link::Star => tr!("Like Heronite?"),
        }
    }

    fn description(self) -> &'static str {
        match self {
            Link::Problem => tr!("Opens a bug report on GitHub with this computer's information already filled in."),
            Link::Idea => tr!("A feature you miss, or something that could work better."),
            Link::Question => tr!("Get help with a setup, or talk with other players."),
            Link::Star => tr!("It is free and open source. A star on GitHub helps more people find it."),
        }
    }

    fn button(self) -> (Icon, &'static str) {
        match self {
            Link::Problem => (Icon::Flag, tr!("Report")),
            Link::Idea => (Icon::Lightbulb, tr!("Suggest")),
            Link::Question => (Icon::Notes, tr!("Ask")),
            Link::Star => (Icon::Star, tr!("Star on GitHub")),
        }
    }

    /// The bug report form gets the system information (the `system` field)
    /// once it is read.
    fn url(self, about: &State, ctx: &PageCtx) -> String {
        match self {
            Link::Problem if !about.system.os.is_empty() => format!(
                "{REPO}/issues/new?template=bug_report.yml&system={}",
                heronite_metadata::util::encode_uri_component(&about.report(ctx))
            ),
            Link::Problem => format!("{REPO}/issues/new?template=bug_report.yml"),
            Link::Idea => format!("{REPO}/issues/new?template=feature_request.yml"),
            Link::Question => format!("{REPO}/discussions/categories/q-a"),
            Link::Star => REPO.to_string(),
        }
    }
}

/// Key / value row: label left, the value (selectable-looking, wraps) right.
fn info_row(label: &str, value: impl Into<SharedString>) -> AnyElement {
    row(label.to_string(), None, div().max_w(px(520.)).text_size(px(14.)).text_color(theme::text_muted()).text_right().child(value.into()))
        .into_any_element()
}

impl SettingsPage {
    pub(super) fn about_section(&mut self, cx: &mut Context<Self>) -> Div {
        let a = &self.about;
        let s = &a.system;
        let app = card([
            info_row(tr!("Version"), format!("Heronite {}", env!("CARGO_PKG_VERSION"))),
            info_row(
                "Legendary",
                a.legendary
                    .clone()
                    .unwrap_or_else(|| if a.loaded && a.wine_count.is_some() { tr!("not found").into() } else { "...".into() }),
            ),
            info_row(
                tr!("Wine / Proton found"),
                match (a.wine_count, a.proton_count) {
                    (Some(wine), Some(proton)) => tr!("{wine} Wine, {proton} Proton", wine, proton),
                    _ => "...".into(),
                },
            ),
            info_row(tr!("Data folder"), self.ctx.data_paths.root.display().to_string()),
            info_row(tr!("Settings folder"), self.ctx.app.config.display().to_string()),
            info_row(tr!("Cache folder"), self.ctx.app.cache.display().to_string()),
        ]);

        let feedback = card(Link::ALL.map(|link| {
            let (ic, label) = link.button();
            let kind = if matches!(link, Link::Star) { ButtonKind::Primary } else { ButtonKind::Glass };
            row(
                link.label(),
                Some(link.description()),
                icon_action_button(
                    SharedString::from(format!("about-{}", link.id())),
                    kind,
                    ic,
                    label,
                    cx.listener(move |this, _, _, cx| cx.open_url(&link.url(&this.about, &this.ctx))),
                ),
            )
            .into_any_element()
        }));

        let updates = self.updates_section(cx);
        let mut col = div()
            .flex()
            .flex_col()
            .children(updates)
            .child(caption(tr!("Help and feedback")))
            .child(feedback)
            .child(caption("Heronite"))
            .child(app)
            .child(caption(tr!("System")));
        if s.os.is_empty() {
            col = col.child(card([empty_line(tr!("Reading system information..."))]));
        } else {
            let mut rows = vec![
                info_row(tr!("OS"), s.os.clone()),
                info_row(tr!("Kernel"), s.kernel.clone()),
                info_row("CPU", tr!("{cpu} ({threads} threads)", cpu = s.cpu, threads = s.cores)),
                info_row(tr!("Memory"), sysinfo::memory_label(s.memory_kb)),
            ];
            for (i, g) in s.gpus.iter().enumerate() {
                let driver = match &g.driver_version {
                    Some(v) => format!("{} {v}", g.driver),
                    None => g.driver.clone(),
                };
                rows.push(info_row(
                    &if s.gpus.len() > 1 { format!("GPU {}", i + 1) } else { "GPU".to_string() },
                    tr!("{vendor} {name}  (driver {driver})", vendor = g.vendor, name = g.name, driver),
                ));
            }
            if s.gpus.is_empty() {
                rows.push(info_row("GPU", tr!("not detected")));
            }
            if !s.session.is_empty() {
                rows.push(info_row(tr!("Session"), s.session.clone()));
            }
            let copied = a.copied;
            rows.push(
                row(
                    tr!("Copy system information"),
                    Some(tr!("For bug reports.")),
                    action_button(
                        "about-copy",
                        ButtonKind::Glass,
                        if copied { tr!("Copied to clipboard!") } else { tr!("Copy to clipboard") },
                        true,
                        cx.listener(|this, _, _, cx| {
                            let text = this.about.report(&this.ctx);
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                            this.about.copied = true;
                            cx.notify();
                        }),
                    ),
                )
                .into_any_element(),
            );
            col = col.child(card(rows));
        }
        col
    }
}

fn empty_line(text: &str) -> AnyElement {
    div().py(px(14.)).text_size(px(13.)).text_color(theme::text_dim()).child(text.to_string()).into_any_element()
}

/// Console mode rows: the same facts, read-only, and the copy action.
pub(super) fn console_rows(p: &SettingsPage, cx: &App) -> Vec<crate::console::list::Row> {
    use crate::console::list::Row;
    let a = &p.about;
    let s = &a.system;
    let mut rows = super::updates::console_rows(cx);
    rows.push(Row::header(tr!("Help and feedback")));
    rows.extend(Link::ALL.map(|link| Row::action(link.id(), link.label()).desc(link.description())));
    rows.extend([
        Row::header("Heronite"),
        Row::info("i:version", tr!("Version"), format!("Heronite {}", env!("CARGO_PKG_VERSION"))),
        Row::info("i:legendary", "Legendary", a.legendary.clone().unwrap_or_else(|| "\u{2026}".into())),
        Row::info(
            "i:wine",
            tr!("Wine / Proton found"),
            match (a.wine_count, a.proton_count) {
                (Some(wine), Some(proton)) => tr!("{wine} Wine, {proton} Proton", wine, proton),
                _ => "\u{2026}".into(),
            },
        ),
        Row::info("i:config", tr!("Data folder"), p.ctx.data_paths.root.display().to_string()),
        Row::header(tr!("System")),
    ]);
    if s.os.is_empty() {
        rows.push(Row::info("i:reading", tr!("Reading system information\u{2026}"), ""));
    } else {
        rows.push(Row::info("i:os", tr!("OS"), s.os.clone()));
        rows.push(Row::info("i:kernel", tr!("Kernel"), s.kernel.clone()));
        rows.push(Row::info("i:cpu", "CPU", tr!("{cpu} ({threads} threads)", cpu = s.cpu, threads = s.cores)));
        rows.push(Row::info("i:mem", tr!("Memory"), sysinfo::memory_label(s.memory_kb)));
        for (i, g) in s.gpus.iter().enumerate() {
            rows.push(Row::info(SharedString::from(format!("i:gpu{i}")), "GPU", format!("{} {}", g.vendor, g.name)));
        }
        if !s.session.is_empty() {
            rows.push(Row::info("i:session", tr!("Session"), s.session.clone()));
        }
        rows.push(
            Row::action("copy", if a.copied { tr!("Copied to the clipboard") } else { tr!("Copy the system information") })
                .desc(tr!("For bug reports")),
        );
    }
    rows
}

pub(super) fn console_apply(p: &mut SettingsPage, outcome: crate::console::list::Outcome, cx: &mut Context<SettingsPage>) {
    if super::updates::console_apply(&outcome, cx) {
        return;
    }
    if let crate::console::list::Outcome::Activate(id) = outcome {
        if let Some(link) = Link::from_id(&id) {
            cx.open_url(&link.url(&p.about, &p.ctx));
        } else if id.as_ref() == "copy" {
            let text = p.about.report(&p.ctx);
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            p.about.copied = true;
            cx.notify();
        }
    }
}
