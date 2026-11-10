//! `source.toml`: what an extension is, read and checked before anything
//! runs, and turned into the [`SourceDescriptor`] every screen draws from.
//!
//! Unknown keys are refused where the app must understand every key to stay
//! safe (`[network]`, and `api` itself); anywhere else they are left out with
//! a warning ([`Manifest::warnings`]), so a manifest written for a later
//! minor contract still loads. See `docs/EXTENSIONS.md` for the reference.

use std::time::Duration;

use heronite_data::settings::ALL_FIELDS;
use heronite_metadata::sources::{
    is_reserved_id, AppliesTo, Asked, ExtensionInfo, GameKind, Needs, OptionChoice, OptionKind, OptionValue, SourceOption,
};
use heronite_metadata::{platforms, MatchedBy, SourceDescriptor, SourceKind};
use toml::{Table, Value};

use crate::net::HostRule;
use crate::ExtensionError;

/// The version of the contract (manifest and script API) this build speaks.
pub const API_VERSION: u32 = 1;

/// The picture fields: a pictures source provides only these.
const PICTURE_FIELDS: [&str; 4] = ["cover", "hero", "coverSquare", "coverWide"];
const MAX_HOSTS: usize = 16;
const MAX_SETTINGS: usize = 16;
const MAX_CHOICES: usize = 50;
/// The fastest rate a manifest may ask for.
const MAX_RATE_PER_SECOND: f64 = 10.0;

/// How often the host lets an extension send a request: `count` per `per`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    pub count: u32,
    pub per: Duration,
}

impl Rate {
    /// One request a second, when the manifest does not say.
    pub const DEFAULT: Rate = Rate { count: 1, per: Duration::from_secs(1) };

    /// The time between two requests.
    pub fn interval(&self) -> Duration {
        self.per / self.count.max(1)
    }

    fn parse(text: &str) -> Result<Self, String> {
        let bad = || format!("network.rate \"{text}\" is not a rate such as \"2/s\" or \"30/min\"");
        let (n, unit) = text.trim().split_once('/').ok_or_else(bad)?;
        let count: u32 = n.trim().parse().ok().filter(|n| *n > 0).ok_or_else(bad)?;
        let per = match unit.trim() {
            "s" | "sec" | "second" => Duration::from_secs(1),
            "m" | "min" | "minute" => Duration::from_secs(60),
            _ => return Err(bad()),
        };
        let rate = Rate { count, per };
        if count as f64 / per.as_secs_f64() > MAX_RATE_PER_SECOND {
            return Err(format!("network.rate \"{text}\" is faster than {MAX_RATE_PER_SECOND} requests a second"));
        }
        Ok(rate)
    }

    /// As written in a manifest (`2/s`, `30/min`).
    pub fn text(&self) -> String {
        if self.per == Duration::from_secs(60) {
            format!("{}/min", self.count)
        } else {
            format!("{}/s", self.count)
        }
    }
}

/// `[network]`: where the script may send requests and take pictures from.
#[derive(Debug, Clone, PartialEq)]
pub struct Network {
    /// Hosts the script may ask (`ctx.http`); their pictures are allowed too.
    pub hosts: Vec<HostRule>,
    /// Further hosts its pictures may come from (never asked by the script).
    pub images: Vec<HostRule>,
    pub rate: Rate,
}

/// A checked `source.toml`.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: semver::Version,
    pub author: String,
    pub homepage: Option<String>,
    pub kind: SourceKind,
    pub adult: bool,
    pub description: String,
    /// Field ids it fills (`ALL_FIELDS` entries), in the manifest's order.
    pub provides: Vec<String>,
    pub applies_to: AppliesTo,
    pub asked: Asked,
    pub game_kind: Option<GameKind>,
    pub network: Network,
    pub settings: Vec<SourceOption>,
    /// Keys left out and other things worth a line in the log.
    pub warnings: Vec<String>,
}

fn err(msg: impl Into<String>) -> ExtensionError {
    ExtensionError::Manifest(msg.into())
}

/// Reads one table's keys, noting the ones it does not know.
struct Keys<'a> {
    table: &'a Table,
    section: &'static str,
    known: &'static [&'static str],
}

impl<'a> Keys<'a> {
    fn new(table: &'a Table, section: &'static str, known: &'static [&'static str]) -> Self {
        Self { table, section, known }
    }

    /// `key` of this table, escaped (an unknown key is the author's text and
    /// goes to the log).
    fn name(&self, key: &str) -> String {
        if self.section.is_empty() {
            escaped(key)
        } else {
            format!("{}.{}", self.section, escaped(key))
        }
    }

    fn unknown(&self) -> Vec<String> {
        self.table.keys().filter(|k| !self.known.contains(&k.as_str())).map(|k| self.name(k)).collect()
    }

    fn get(&self, key: &str) -> Option<&'a Value> {
        self.table.get(key)
    }

    fn string(&self, key: &str) -> Result<Option<String>, ExtensionError> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.trim().to_owned())),
            Some(_) => Err(err(format!("{} must be text", self.name(key)))),
        }
    }

    /// Text of 1..=`max` characters without control characters.
    fn text(&self, key: &str, max: usize) -> Result<String, ExtensionError> {
        let value = self.string(key)?.ok_or_else(|| err(format!("{} is missing", self.name(key))))?;
        check_text(&self.name(key), &value, max)?;
        Ok(value)
    }

    fn optional_text(&self, key: &str, max: usize) -> Result<Option<String>, ExtensionError> {
        match self.string(key)? {
            Some(v) if !v.is_empty() => check_text(&self.name(key), &v, max).map(|_| Some(v)),
            _ => Ok(None),
        }
    }

    fn bool(&self, key: &str) -> Result<bool, ExtensionError> {
        match self.get(key) {
            None => Ok(false),
            Some(Value::Boolean(b)) => Ok(*b),
            Some(_) => Err(err(format!("{} must be true or false", self.name(key)))),
        }
    }

    fn strings(&self, key: &str) -> Result<Option<Vec<String>>, ExtensionError> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Array(items)) => items
                .iter()
                .map(|v| match v {
                    Value::String(s) => Ok(s.trim().to_owned()),
                    _ => Err(err(format!("{} must be a list of text", self.name(key)))),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
            Some(_) => Err(err(format!("{} must be a list of text", self.name(key)))),
        }
    }

    fn table(&self, key: &str) -> Result<Option<&'a Table>, ExtensionError> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Table(t)) => Ok(Some(t)),
            Some(_) => Err(err(format!("[{}] must be a table", self.name(key)))),
        }
    }
}

/// Whether `c` is an invisible Unicode format character (category Cf:
/// direction overrides and isolates, zero-width characters, soft hyphen,
/// tags...) or a line or paragraph separator: text that shows differently
/// from what it is, or breaks a line of the log.
pub fn is_format_char(c: char) -> bool {
    matches!(
        c as u32,
        0xad | 0x600..=0x605
            | 0x61c
            | 0x6dd
            | 0x70f
            | 0x890..=0x891
            | 0x8e2
            | 0x180e
            | 0x200b..=0x200f
            | 0x2028..=0x202e
            | 0x2060..=0x2064
            | 0x2066..=0x206f
            | 0xfeff
            | 0xfff9..=0xfffb
            | 0x110bd
            | 0x110cd
            | 0x13430..=0x1343f
            | 0x1bca0..=0x1bca3
            | 0x1d173..=0x1d17a
            | 0xe0001
            | 0xe0020..=0xe007f
    )
}

/// Text of 1..=`max` characters, on one line (the `description` may have
/// line breaks), without control or format characters.
pub(crate) fn check_text(name: &str, value: &str, max: usize) -> Result<(), ExtensionError> {
    if value.is_empty() {
        return Err(err(format!("{name} is empty")));
    }
    if value.chars().count() > max {
        return Err(err(format!("{name} is longer than {max} characters")));
    }
    let paragraph = name == "description";
    if value.chars().any(|c| c.is_control() && !(paragraph && c == '\n')) {
        return Err(err(format!("{name} has control characters or line breaks")));
    }
    if value.chars().any(is_format_char) {
        return Err(err(format!("{name} has invisible formatting characters")));
    }
    Ok(())
}

/// `text` written by the author, as it may appear in the log: escaped, so
/// it stays on one line and shows what it is.
pub(crate) fn escaped(text: &str) -> String {
    one_line(&text.chars().take(100).collect::<String>())
}

/// `text` (an author's or a script's) on one line of the log: line breaks,
/// other control and format characters written as escapes.
pub fn one_line(text: &str) -> String {
    text.chars()
        .flat_map(|c| -> Box<dyn Iterator<Item = char>> {
            if c.is_control() || is_format_char(c) {
                Box::new(c.escape_debug())
            } else {
                Box::new(std::iter::once(c))
            }
        })
        .collect()
}

/// Whether `id` is lowercase letters and digits in groups joined by single
/// dashes, 2 to 40 characters long.
pub fn valid_id(id: &str) -> bool {
    (2..=40).contains(&id.len())
        && id.split('-').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

pub(crate) fn http_url(name: &str, value: &str) -> Result<(), ExtensionError> {
    match url::Url::parse(value) {
        Ok(u) if matches!(u.scheme(), "http" | "https") && u.host_str().is_some() => Ok(()),
        _ => Err(err(format!("{name} \"{value}\" is not an http(s) address"))),
    }
}

impl Manifest {
    /// Parses and checks `source.toml`.
    pub fn parse(text: &str) -> Result<Self, ExtensionError> {
        let table: Table = text.parse().map_err(|e: toml::de::Error| err(format!("source.toml is not valid TOML: {}", e.message())))?;
        // The contract version first: a newer one may change everything else.
        let api = match table.get("api") {
            None => return Err(err("api is missing (this build reads api = 1)")),
            Some(Value::Integer(n)) => *n,
            Some(_) => return Err(err("api must be a whole number")),
        };
        if api > API_VERSION as i64 {
            return Err(ExtensionError::NewerApi(u32::try_from(api).unwrap_or(u32::MAX)));
        }
        if api < 1 {
            return Err(err(format!("api = {api} is not a contract version")));
        }
        let top = Keys::new(
            &table,
            "",
            &[
                "id",
                "name",
                "version",
                "api",
                "author",
                "homepage",
                "kind",
                "adult",
                "description",
                "provides",
                "applies",
                "network",
                "settings",
            ],
        );
        let mut warnings: Vec<String> = top.unknown().into_iter().map(|k| format!("unknown key {k} left out")).collect();

        let id = top.string("id")?.ok_or_else(|| err("id is missing"))?;
        if !valid_id(&id) {
            return Err(err(format!("id \"{id}\" must be 2 to 40 lowercase letters, digits and single dashes")));
        }
        if is_reserved_id(&id) {
            return Err(ExtensionError::IdTaken(id));
        }
        let name = top.text("name", 40)?;
        let version_text = top.string("version")?.ok_or_else(|| err("version is missing"))?;
        let version = semver::Version::parse(&version_text)
            .map_err(|e| err(format!("version \"{version_text}\" is not a semantic version such as 1.0.0: {e}")))?;
        let author = top.text("author", 80)?;
        let homepage = top.optional_text("homepage", 300)?;
        if let Some(h) = &homepage {
            http_url("homepage", h)?;
        }
        let kind = match top.string("kind")?.as_deref() {
            Some("details") => SourceKind::Details,
            Some("pictures") => SourceKind::Pictures,
            Some("extras") => SourceKind::Extras,
            Some(other) => return Err(err(format!("kind \"{other}\" must be details, pictures or extras"))),
            None => return Err(err("kind is missing")),
        };
        let adult = top.bool("adult")?;
        let description = top.text("description", 300)?;

        let provides = provides(&top, kind, &mut warnings)?;
        let (applies_to, asked, game_kind) = applies(&top, &mut warnings)?;
        let network = network(&top)?;
        let settings = settings(&table, &mut warnings)?;
        Ok(Self {
            id,
            name,
            version,
            author,
            homepage,
            kind,
            adult,
            description,
            provides,
            applies_to,
            asked,
            game_kind,
            network,
            settings,
            warnings,
        })
    }

    /// The descriptor the registry lists it with.
    pub fn descriptor(&self) -> SourceDescriptor {
        SourceDescriptor {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: self.kind,
            provides: self.provides.clone(),
            applies_to: self.applies_to.clone(),
            finds_by: vec![MatchedBy::SourceId, MatchedBy::SteamId, MatchedBy::ExactTitle, MatchedBy::FuzzyTitle],
            needs: if self.settings.iter().any(|s| s.required) { Needs::ApiKey } else { Needs::Nothing },
            network: !self.network.hosts.is_empty(),
            hidden: false,
            manual_only: false,
            searchable: true,
            only_on: Vec::new(),
            options: self.settings.clone(),
            asked: self.asked,
            game_kind: self.game_kind.clone(),
            about: Some(self.description.clone()),
            // Never a database to download, an audience an option widens,
            // or a place ahead of the sources already in a field's order.
            download: None,
            also_for: None,
            leads_for: None,
            extension: Some(ExtensionInfo {
                version: self.version.to_string(),
                author: self.author.clone(),
                homepage: self.homepage.clone(),
                adult: self.adult,
            }),
        }
    }
}

fn provides(top: &Keys<'_>, kind: SourceKind, warnings: &mut Vec<String>) -> Result<Vec<String>, ExtensionError> {
    let table = top.table("provides")?.ok_or_else(|| err("[provides] is missing"))?;
    let keys = Keys::new(table, "provides", &["fields"]);
    warnings.extend(keys.unknown().into_iter().map(|k| format!("unknown key {k} left out")));
    let fields = keys.strings("fields")?.ok_or_else(|| err("provides.fields is missing"))?;
    let mut out: Vec<String> = Vec::new();
    for f in fields {
        if !ALL_FIELDS.contains(&f.as_str()) {
            warnings.push(format!("provides.fields: unknown field \"{}\" left out", escaped(&f)));
        } else if kind == SourceKind::Pictures && !PICTURE_FIELDS.contains(&f.as_str()) {
            return Err(err(format!("a pictures source provides pictures only, not \"{f}\"")));
        } else if !out.contains(&f) {
            out.push(f);
        }
    }
    if out.is_empty() {
        return Err(err("provides.fields names no field this app knows"));
    }
    Ok(out)
}

fn applies(top: &Keys<'_>, warnings: &mut Vec<String>) -> Result<(AppliesTo, Asked, Option<GameKind>), ExtensionError> {
    applies_of(top.table("applies")?, warnings)
}

/// The `[applies]` table read as the manifest reads it (a list of
/// extensions copies it as is: `index.rs`).
pub(crate) fn applies_of(
    table: Option<&Table>,
    warnings: &mut Vec<String>,
) -> Result<(AppliesTo, Asked, Option<GameKind>), ExtensionError> {
    let Some(table) = table else { return Ok((AppliesTo::All, Asked::Always, None)) };
    let keys = Keys::new(table, "applies", &["to", "platforms", "stores", "asked", "game_kind"]);
    warnings.extend(keys.unknown().into_iter().map(|k| format!("unknown key {k} left out")));
    let to = keys.string("to")?;
    let platforms = keys.strings("platforms")?;
    let stores = keys.strings("stores")?;
    if [to.is_some(), platforms.is_some(), stores.is_some()].iter().filter(|b| **b).count() > 1 {
        return Err(err("[applies] takes one of to, platforms and stores"));
    }
    let non_empty = |name: &str, list: Vec<String>| {
        let list: Vec<String> = list.into_iter().filter(|s| !s.is_empty()).collect();
        if list.is_empty() {
            Err(err(format!("applies.{name} is empty")))
        } else {
            Ok(list)
        }
    };
    let applies_to = match (to.as_deref(), platforms, stores) {
        (Some("all") | None, None, None) => AppliesTo::All,
        (Some("emulated"), ..) => AppliesTo::Emulated,
        (Some("pc"), ..) => AppliesTo::Pc,
        (Some("steam"), ..) => AppliesTo::HasSteamId,
        (Some(other), ..) => return Err(err(format!("applies.to \"{other}\" must be all, emulated, pc or steam"))),
        (None, Some(p), _) => AppliesTo::Platforms(platform_ids(non_empty("platforms", p)?, warnings)?),
        (None, None, Some(s)) => AppliesTo::Stores(non_empty("stores", s)?),
    };
    let asked = match keys.string("asked")?.as_deref() {
        None | Some("always") => Asked::Always,
        Some("fallback") => Asked::AsFallback,
        Some(other) => return Err(err(format!("applies.asked \"{other}\" must be always or fallback"))),
    };
    let game_kind = match keys.table("game_kind")? {
        None => None,
        Some(t) => {
            let k = Keys::new(t, "applies.game_kind", &["name", "signals"]);
            warnings.extend(k.unknown().into_iter().map(|key| format!("unknown key {key} left out")));
            let name = k.text("name", 40)?;
            let signals: Vec<String> = k.strings("signals")?.unwrap_or_default().into_iter().filter(|s| !s.is_empty()).collect();
            Some(GameKind { name, signals })
        }
    };
    Ok((applies_to, asked, game_kind))
}

/// The app's platform ids ([`platforms::by_name`]: an id, a name or an
/// alias such as `snes`) of `names`, which is what an emulated game's
/// `platform` is compared with. A name the app does not know is left out.
fn platform_ids(names: Vec<String>, warnings: &mut Vec<String>) -> Result<Vec<String>, ExtensionError> {
    let mut ids: Vec<String> = Vec::new();
    for name in names {
        match platforms::by_name(&name) {
            Some(p) if !ids.iter().any(|id| id == p.id) => ids.push(p.id.to_owned()),
            Some(_) => {}
            None => warnings.push(format!("applies.platforms: unknown platform \"{}\" left out", escaped(&name))),
        }
    }
    if ids.is_empty() {
        return Err(err("applies.platforms names no platform this app knows"));
    }
    Ok(ids)
}

fn network(top: &Keys<'_>) -> Result<Network, ExtensionError> {
    let Some(table) = top.table("network")? else {
        return Ok(Network { hosts: Vec::new(), images: Vec::new(), rate: Rate::DEFAULT });
    };
    let keys = Keys::new(table, "network", &["hosts", "images", "rate"]);
    if let Some(k) = keys.unknown().first() {
        return Err(err(format!("unknown key {k}: [network] must be understood in full")));
    }
    let rules = |name: &str| -> Result<Vec<HostRule>, ExtensionError> {
        let list = keys.strings(name)?.unwrap_or_default();
        if list.len() > MAX_HOSTS {
            return Err(err(format!("network.{name} lists more than {MAX_HOSTS} hosts")));
        }
        list.iter().map(|h| HostRule::parse(h).map_err(|e| err(format!("network.{name}: {e}")))).collect()
    };
    let rate = match keys.string("rate")? {
        Some(text) => Rate::parse(&text).map_err(err)?,
        None => Rate::DEFAULT,
    };
    Ok(Network { hosts: rules("hosts")?, images: rules("images")?, rate })
}

fn settings(table: &Table, warnings: &mut Vec<String>) -> Result<Vec<SourceOption>, ExtensionError> {
    let list = match table.get("settings") {
        None => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(err("settings must be [[settings]] tables")),
    };
    if list.len() > MAX_SETTINGS {
        return Err(err(format!("more than {MAX_SETTINGS} settings")));
    }
    let mut out: Vec<SourceOption> = Vec::new();
    for (i, item) in list.iter().enumerate() {
        let Value::Table(t) = item else { return Err(err("settings must be [[settings]] tables")) };
        let keys = Keys::new(t, "settings", &["key", "label", "type", "options", "default", "help", "link", "required"]);
        warnings.extend(keys.unknown().into_iter().map(|k| format!("unknown key {k} (setting {}) left out", i + 1)));
        let key = keys.string("key")?.ok_or_else(|| err(format!("setting {} has no key", i + 1)))?;
        let valid_key = key.len() <= 32
            && key.starts_with(|c: char| c.is_ascii_alphabetic())
            && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid_key {
            return Err(err(format!("setting key \"{key}\" must be letters, digits and _ (32 at most), starting with a letter")));
        }
        if out.iter().any(|o| o.key == key) {
            return Err(err(format!("setting key \"{key}\" is used twice")));
        }
        let label = keys.text("label", 60)?;
        let kind = match keys.string("type")?.as_deref() {
            Some("text") => OptionKind::Text,
            Some("secret") => OptionKind::Secret,
            Some("select") => OptionKind::Select,
            Some("switch") => OptionKind::Switch,
            Some(other) => return Err(err(format!("setting {key}: type \"{other}\" must be text, secret, select or switch"))),
            None => return Err(err(format!("setting {key} has no type"))),
        };
        let choices = choices(&key, t.get("options"))?;
        if kind == OptionKind::Select && choices.is_empty() {
            return Err(err(format!("setting {key} is a select without options")));
        }
        let default = match (t.get("default"), kind) {
            (None, _) => None,
            (Some(Value::Boolean(b)), OptionKind::Switch) => Some(OptionValue::Switch(*b)),
            (Some(Value::String(s)), OptionKind::Select) if choices.iter().any(|c| c.value == *s) => Some(OptionValue::Text(s.clone())),
            (Some(Value::String(s)), OptionKind::Text | OptionKind::Secret) => Some(OptionValue::Text(s.clone())),
            (Some(_), _) => return Err(err(format!("setting {key}: the default does not fit its type"))),
        };
        let help = keys.optional_text("help", 200)?;
        let link = keys.optional_text("link", 300)?;
        if let Some(l) = &link {
            http_url(&format!("setting {key}: link"), l)?;
        }
        out.push(SourceOption { key, label, kind, choices, default, help, link, required: keys.bool("required")? });
    }
    Ok(out)
}

/// `options = ["a", "b"]` or `options = [{ value = "a", label = "A" }]`.
fn choices(key: &str, value: Option<&Value>) -> Result<Vec<OptionChoice>, ExtensionError> {
    let Some(value) = value else { return Ok(Vec::new()) };
    let Value::Array(items) = value else { return Err(err(format!("setting {key}: options must be a list"))) };
    if items.len() > MAX_CHOICES {
        return Err(err(format!("setting {key} has more than {MAX_CHOICES} options")));
    }
    let mut out: Vec<OptionChoice> = Vec::new();
    for item in items {
        let choice = match item {
            Value::String(s) => OptionChoice { value: s.clone(), label: s.clone() },
            Value::Table(t) => {
                let k = Keys::new(t, "settings.options", &["value", "label"]);
                let value = k.text("value", 100)?;
                let label = k.optional_text("label", 60)?.unwrap_or_else(|| value.clone());
                OptionChoice { value, label }
            }
            _ => return Err(err(format!("setting {key}: an option must be text or {{ value, label }}"))),
        };
        if out.iter().any(|c| c.value == choice.value) {
            return Err(err(format!("setting {key}: option \"{}\" is listed twice", choice.value)));
        }
        out.push(choice);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
