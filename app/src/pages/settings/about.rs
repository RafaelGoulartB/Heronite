//! Settings → About: version, data folders and system information.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::sysinfo::{self, SystemInfo};
use super::SettingsPage;
use crate::pages::PageCtx;
use crate::theme;
use crate::ui::controls::{action_button, caption, card, row, ButtonKind};
use crate::ui::fields::meta_text;

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
fn legendary_version(heroic: &heronite_data::HeroicPaths) -> Option<String> {
    let lg = heronite_epic::cmd::Legendary::locate(heroic).ok()?;
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
        let heroic = ctx.heroic.clone();
        let work = move || {
            let system = sysinfo::gather();
            let installs = heronite_wine::list_installations(&heroic);
            let proton = installs.iter().filter(|w| w.kind == "proton").count();
            (system, legendary_version(&heroic), installs.len(), proton)
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
            format!("Heronite (Rust) {}", env!("CARGO_PKG_VERSION")),
            format!("OS: {} (kernel {})", s.os, s.kernel),
            format!("CPU: {} ({} threads)", s.cpu, s.cores),
            format!("Memory: {}", sysinfo::human_memory(s.memory_kb)),
        ];
        for g in &s.gpus {
            out.push(format!("GPU: {} {} (driver {}{})", g.vendor, g.name, g.driver, g.driver_version.as_deref().map(|v| format!(" {v}")).unwrap_or_default()));
        }
        if !s.session.is_empty() {
            out.push(format!("Session: {}", s.session));
        }
        out.push(format!("Legendary: {}", self.legendary.as_deref().unwrap_or("not found")));
        out.push(format!("Config folder: {}", ctx.heroic.root.display()));
        out.join("\n")
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
            info_row("Version", format!("{} (Rust rewrite)", env!("CARGO_PKG_VERSION"))),
            info_row("Legendary", a.legendary.clone().unwrap_or_else(|| if a.loaded && a.wine_count.is_some() { "not found".into() } else { "...".into() })),
            info_row(
                "Wine / Proton found",
                match (a.wine_count, a.proton_count) {
                    (Some(w), Some(p)) => format!("{w} Wine, {p} Proton"),
                    _ => "...".into(),
                },
            ),
            info_row("Config folder", self.ctx.heroic.root.display().to_string()),
            info_row("App data folder", self.ctx.app.config.display().to_string()),
            info_row("Cache folder", self.ctx.app.cache.display().to_string()),
        ]);

        let mut col = div()
            .flex()
            .flex_col()
            .child(caption("Heronite"))
            .child(app)
            .child(div().mx(px(6.)).mt(px(8.)).child(meta_text("The config folder is shared with Heroic and Heronite, so both can be used during the transition.")))
            .child(caption("System"));
        if s.os.is_empty() {
            col = col.child(card([empty_line("Reading system information...")]));
        } else {
            let mut rows = vec![
                info_row("OS", s.os.clone()),
                info_row("Kernel", s.kernel.clone()),
                info_row("CPU", format!("{} ({} threads)", s.cpu, s.cores)),
                info_row("Memory", sysinfo::human_memory(s.memory_kb)),
            ];
            for (i, g) in s.gpus.iter().enumerate() {
                let driver = match &g.driver_version {
                    Some(v) => format!("{} {v}", g.driver),
                    None => g.driver.clone(),
                };
                rows.push(info_row(&if s.gpus.len() > 1 { format!("GPU {}", i + 1) } else { "GPU".to_string() }, format!("{} {}  (driver {driver})", g.vendor, g.name)));
            }
            if s.gpus.is_empty() {
                rows.push(info_row("GPU", "not detected"));
            }
            if !s.session.is_empty() {
                rows.push(info_row("Session", s.session.clone()));
            }
            let copied = a.copied;
            rows.push(
                row(
                    "Copy system information",
                    Some("For bug reports."),
                    action_button(
                        "about-copy",
                        ButtonKind::Glass,
                        if copied { "Copied to clipboard!" } else { "Copy to clipboard" },
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
