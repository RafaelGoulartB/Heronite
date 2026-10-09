//! The Details tab: the metadata form, its dirty check and saving.

use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::collection::collect_tag_options;
use heronite_data::models::{CollectionGameMetadata, LocalGameMeta};
use heronite_metadata::{DetailsPatch, DetailsResult};

use super::{block, Busy, GameSettingsDialog};
use crate::ui::controls::{card, help_text, caption};
use crate::ui::fields::{input_box, labeled, new_input, new_textarea, textarea_box, TagEvent, TagField};
use crate::dialogs::{DialogCtx, DialogEvent};

#[derive(Clone, PartialEq, Default)]
pub(super) struct Form {
    title: String,
    description: String,
    release_date: String,
    series: String,
    developers: Vec<String>,
    publishers: Vec<String>,
    genres: Vec<String>,
    themes: Vec<String>,
    game_modes: Vec<String>,
    platforms: Vec<String>,
    features: Vec<String>,
    score: String,
    websites: String,
    notes: String,
}

pub(super) fn form_from(title: &str, md: Option<&CollectionGameMetadata>, meta: Option<&LocalGameMeta>) -> Form {
    let md_str = |v: &Option<String>| v.clone().unwrap_or_default();
    Form {
        title: md.and_then(|m| m.title.as_deref()).map(str::trim).filter(|t| !t.is_empty()).unwrap_or(title).to_string(),
        description: md.map(|m| md_str(&m.description)).unwrap_or_default(),
        release_date: md.map(|m| md_str(&m.release_date)).unwrap_or_default(),
        series: md.map(|m| md_str(&m.series)).unwrap_or_default(),
        developers: md.map(|m| m.developers.clone()).unwrap_or_default(),
        publishers: md.map(|m| m.publishers.clone()).unwrap_or_default(),
        genres: md.map(|m| m.genres.clone()).unwrap_or_default(),
        themes: md.map(|m| m.themes.clone()).unwrap_or_default(),
        game_modes: md.map(|m| m.game_modes.clone()).unwrap_or_default(),
        platforms: md.map(|m| m.platforms.clone()).unwrap_or_default(),
        features: md.map(|m| m.features.clone()).unwrap_or_default(),
        score: md
            .and_then(|m| m.critic_score)
            .map(|n| if n.get().fract() == 0.0 { format!("{}", n.get() as i64) } else { n.get().to_string() })
            .unwrap_or_default(),
        websites: md.map(|m| m.websites.join(", ")).unwrap_or_default(),
        notes: md.and_then(|m| m.notes.clone()).filter(|n| !n.is_empty()).or_else(|| meta.and_then(|m| m.notes.clone())).unwrap_or_default(),
    }
}

fn split_list(value: &str) -> Vec<String> {
    value.split([',', ';', '\n']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

pub(super) const TAG_FIELDS: [(&str, &str, bool); 8] = [
    ("series", "Series", false),
    ("developers", "Developers", true),
    ("publishers", "Publishers", true),
    ("genres", "Genres", true),
    ("themes", "Themes", true),
    ("gameModes", "Game modes", true),
    ("platforms", "Platforms", true),
    ("features", "Features", true),
];

pub(super) struct Details {
    pub(super) name: Entity<InputState>,
    pub(super) description: Entity<TextareaState>,
    pub(super) release: Entity<InputState>,
    pub(super) score: Entity<InputState>,
    pub(super) websites: Entity<InputState>,
    pub(super) notes: Entity<TextareaState>,
    /// In [`TAG_FIELDS`] order.
    pub(super) tags: Vec<Entity<TagField>>,
}

impl GameSettingsDialog {
    pub(super) fn build_details(ctx: &DialogCtx, initial: &Form, window: &mut Window, cx: &mut Context<Self>) -> Details {
        let options = collect_tag_options(&ctx.lib.games);
        let tags: Vec<Entity<TagField>> = TAG_FIELDS
            .iter()
            .enumerate()
            .map(|(i, (id, label, multiple))| {
                let values = match *id {
                    "series" => vec![initial.series.clone()],
                    "developers" => initial.developers.clone(),
                    "publishers" => initial.publishers.clone(),
                    "genres" => initial.genres.clone(),
                    "themes" => initial.themes.clone(),
                    "gameModes" => initial.game_modes.clone(),
                    "platforms" => initial.platforms.clone(),
                    _ => initial.features.clone(),
                };
                let opts = options.get(i).map(|(_, v)| v.clone()).unwrap_or_default();
                let field = cx.new(|cx| TagField::new(&format!("gs-{id}"), label, *multiple, opts, values, window, cx));
                cx.subscribe(&field, |_, _, _: &TagEvent, cx| cx.notify()).detach();
                field
            })
            .collect();
        let details = Details {
            name: new_input(window, cx, &initial.title, ""),
            description: new_textarea(window, cx, &initial.description, 6),
            release: new_input(window, cx, &initial.release_date, ""),
            score: new_input(window, cx, &initial.score, ""),
            websites: new_input(window, cx, &initial.websites, ""),
            notes: new_textarea(window, cx, &initial.notes, 4),
            tags,
        };
        for input in [&details.name, &details.release, &details.score, &details.websites] {
            cx.subscribe_in(input, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            })
            .detach();
        }
        for area in [&details.description, &details.notes] {
            cx.subscribe_in(area, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            })
            .detach();
        }
        details
    }

    pub(super) fn read_form(&self, cx: &App) -> Form {
        let d = &self.details;
        let tag = |i: usize| d.tags[i].read(cx).values().to_vec();
        Form {
            title: d.name.read(cx).value().to_string(),
            description: d.description.read(cx).value().to_string(),
            release_date: d.release.read(cx).value().to_string(),
            series: d.tags[0].read(cx).value(),
            developers: tag(1),
            publishers: tag(2),
            genres: tag(3),
            themes: tag(4),
            game_modes: tag(5),
            platforms: tag(6),
            features: tag(7),
            score: d.score.read(cx).value().to_string(),
            websites: d.websites.read(cx).value().to_string(),
            notes: d.notes.read(cx).value().to_string(),
        }
    }

    pub(super) fn save_details(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let (form, old) = (self.read_form(cx), self.initial.clone());
        let mut patch = DetailsPatch { app_name: self.game.app_name.clone(), runner: self.runner(), ..Default::default() };
        if form.title != old.title {
            patch.title = Some(form.title.clone());
        }
        if form.notes != old.notes {
            patch.notes = Some(form.notes.clone());
        }
        if form.description != old.description {
            patch.description = Some(form.description.clone());
        }
        if form.release_date != old.release_date {
            patch.release_date = Some(form.release_date.clone());
        }
        if form.series != old.series {
            patch.series = Some(form.series.clone());
        }
        macro_rules! list {
            ($f:ident) => {
                if form.$f != old.$f {
                    patch.$f = Some(form.$f.clone());
                }
            };
        }
        list!(developers);
        list!(publishers);
        list!(genres);
        list!(themes);
        list!(game_modes);
        list!(platforms);
        list!(features);
        if form.websites != old.websites {
            patch.websites = Some(split_list(&form.websites));
        }
        if form.score != old.score {
            let t = form.score.trim();
            if t.is_empty() {
                patch.critic_score = Some(None);
            } else {
                match t.parse::<f64>() {
                    Ok(n) if n.is_finite() => patch.critic_score = Some(Some(n.round())),
                    _ => {
                        self.error = "Score must be a number, or empty.".into();
                        cx.notify();
                        return;
                    }
                }
            }
        }
        self.busy = Some(Busy::Details);
        self.error.clear();
        let service = self.service.clone();
        let title = self.title.clone();
        cx.spawn(async move |this, cx| {
            let res: anyhow::Result<DetailsResult> = cx.background_spawn(async move { service.update_game_details(&patch) }).await;
            this.update(cx, |this, cx| {
                this.busy = None;
                match res {
                    Ok(r) => {
                        let meta = r.meta.clone().or_else(|| this.game.meta.as_deref().cloned());
                        this.initial = form_from(&title, Some(&r.metadata), meta.as_ref());
                        cx.emit(DialogEvent::Changed);
                    }
                    Err(err) => this.error = format!("{err:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn details_tab(&mut self) -> Div {
        let d = &self.details;
        let general = card([
            block(labeled("Name", input_box(&d.name, false))),
            block(labeled("Description", textarea_box(&d.description, 6))),
            block(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(div().flex_1().child(labeled("Release date", input_box(&d.release, false))))
                    .child(div().flex_1().child(labeled("Score", input_box(&d.score, false)))),
            ),
        ]);
        let tags = card(d.tags.iter().map(|t| block(t.clone())));
        let more = card([block(labeled("Websites", input_box(&d.websites, false))), block(labeled("Notes", textarea_box(&d.notes, 4)))]);
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                help_text(
                    "These fields stay in Collection. Fetching metadata later will not overwrite values you edit here, unless you choose to overwrite.",
                )
                .px(px(6.)),
            )
            .child(caption("General"))
            .child(general)
            .child(caption("Tags"))
            .child(help_text("Pick existing tags or type a new one and press Enter to create it.").px(px(6.)))
            .child(tags)
            .child(caption("Links and notes"))
            .child(more)
    }
}
