//! An extension as a metadata source: [`ScriptSource`] runs its script for
//! a lookup, a search or a test; [`BrokenExtension`] stands for one that
//! could not be loaded, so Settings can show it and why.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use heronite_metadata::sources::options::option_values;
use heronite_metadata::sources::{OptionValue, SourceHealth, SourceTest};
use heronite_metadata::{
    Http, Lookup, MatchedBy, MetadataCandidate, MetadataSource, SourceContext, SourceDescriptor, SourceGame, SourceHit,
};
use parking_lot::Mutex;
use serde_json::{json, Map, Value};

use crate::engine::{session, CallError, Calls, HostEnv, Limit, Script};
use crate::limits::Limits;
use crate::manifest::{one_line, Manifest};
use crate::net::{scrub, HostRule, RateLimiter};
use crate::record::{self, GameKeys, Hit};
use crate::{store, ExtensionError};

/// A loaded extension.
pub struct ScriptSource {
    descriptor: SourceDescriptor,
    manifest: Manifest,
    script: Script,
    rate: Arc<RateLimiter>,
    limits: Limits,
    /// Hosts its pictures may come from: its hosts and image hosts.
    pictures: Vec<HostRule>,
    /// Its installed folder, where the failing state is kept, and the
    /// SHA-256 of the package it was loaded from.
    dir: Option<(PathBuf, String)>,
    in_a_row: AtomicU32,
    failing: Mutex<Option<String>>,
}

impl ScriptSource {
    /// Compiles the script of an extension. `dir`: its installed folder,
    /// whose record may say it was stopped after failing.
    pub fn new(manifest: Manifest, script: &str, limits: Limits, dir: Option<PathBuf>) -> Result<Self, ExtensionError> {
        let rate = Arc::new(RateLimiter::new(manifest.network.rate.interval()));
        let probe = HostEnv {
            id: manifest.id.clone(),
            http: Http::default(),
            hosts: Arc::new(manifest.network.hosts.clone()),
            rate: rate.clone(),
            limits: limits.clone(),
            settings: Arc::new(Map::new()),
            language: "en".into(),
        };
        let script = Script::compile(script, &probe)?;
        let pictures = manifest.network.hosts.iter().chain(&manifest.network.images).cloned().collect();
        let source = Self {
            descriptor: manifest.descriptor(),
            manifest,
            script,
            rate,
            limits,
            pictures,
            dir: None,
            in_a_row: AtomicU32::new(0),
            failing: Mutex::new(None),
        };
        Ok(match dir {
            Some(dir) => source.installed_in(dir),
            None => source,
        })
    }

    /// This source as the one installed in `dir`: its `install.json` read
    /// there (the package's SHA-256, whether it was stopped).
    pub(crate) fn installed_in(mut self, dir: PathBuf) -> Self {
        let record = store::read_record(&dir);
        *self.failing.get_mut() = record.as_ref().and_then(|r| r.failing.clone());
        self.dir = Some((dir, record.map(|r| r.sha256).unwrap_or_default()));
        self
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The `ctx` of a call: its own options' values, the interface language.
    fn env(&self, ctx: &SourceContext<'_>, language: Option<&str>) -> HostEnv {
        let settings: Map<String, Value> = option_values(ctx.paths, ctx.settings(), &self.descriptor)
            .into_iter()
            .map(|(o, v)| {
                let v = match v {
                    OptionValue::Text(s) => Value::String(s),
                    OptionValue::Switch(b) => Value::Bool(b),
                };
                (o.key, v)
            })
            .collect();
        HostEnv {
            id: self.descriptor.id.clone(),
            http: ctx.http.clone(),
            hosts: Arc::new(self.manifest.network.hosts.clone()),
            rate: self.rate.clone(),
            limits: self.limits.clone(),
            settings: Arc::new(settings),
            language: language.filter(|l| !l.is_empty()).unwrap_or("en").to_owned(),
        }
    }

    /// Whether a setting it cannot be asked without is empty.
    fn lacks_required(&self, ctx: &SourceContext<'_>) -> bool {
        option_values(ctx.paths, ctx.settings(), &self.descriptor).iter().any(|(o, v)| o.required && v.as_text().trim().is_empty())
    }

    /// Counts a failure towards stopping the extension; a success resets the count.
    fn account<T>(&self, what: &str, out: Result<T, CallError>) -> Result<T, CallError> {
        match &out {
            Ok(_) => self.in_a_row.store(0, Ordering::Relaxed),
            Err(e) if e.is_failure() => {
                log::warn!("Extension {} failed ({what}): {}", self.descriptor.id, one_line(&scrub(&e.to_string())));
                let n = self.in_a_row.fetch_add(1, Ordering::Relaxed) + 1;
                // A thread left running is not asked to run another.
                let stuck = matches!(e, CallError::Limit(Limit::Stuck, _));
                if stuck || n >= self.limits.failures_in_a_row {
                    let why = scrub(&format!("{what}: {e}"));
                    log::warn!("Extension {} failed {n} times in a row: not asked until switched on again", self.descriptor.id);
                    if let Some((dir, sha256)) = &self.dir {
                        store::set_failing(dir, sha256, Some(&why));
                    }
                    *self.failing.lock() = Some(why);
                }
            }
            Err(e) => {
                log::info!("Extension {} could not reach its service ({what}): {}", self.descriptor.id, one_line(&scrub(&e.to_string())))
            }
        }
        out
    }

    /// Runs `f` in a session of a call with `ctx`, and counts its failure.
    fn session<T: Send + 'static>(
        &self,
        ctx: &SourceContext<'_>,
        language: Option<&str>,
        what: &str,
        f: impl FnOnce(&mut dyn Calls) -> Result<T, CallError> + Send + 'static,
    ) -> Result<T, CallError> {
        let env = self.env(ctx, language);
        let out = session(&self.script, &env, f).and_then(|r| r);
        self.account(what, out)
    }

    fn candidate(&self, game: &SourceGame, hit: Option<&Hit>, id: &str, value: &Value, by: MatchedBy) -> Lookup<MetadataCandidate> {
        let adult_source = self.manifest.adult;
        let Some(fetched) = record::record(
            value,
            &game.app_name,
            &game.runner,
            &self.descriptor.provides,
            game.fields.as_deref(),
            &self.pictures,
            adult_source,
        ) else {
            return Lookup::NotFound;
        };
        // A title match holds only if the record is the game the hit named.
        let by = match (by, hit) {
            (MatchedBy::ExactTitle, Some(h)) if !h.is_named(fetched.metadata.title.as_deref()) => {
                if fetched.metadata.title.is_some() {
                    MatchedBy::FuzzyTitle
                } else {
                    by
                }
            }
            _ => by,
        };
        let mut c = MetadataCandidate::new(&self.descriptor.id, fetched.metadata, Some(by)).with_id(Some(id.to_owned()));
        if c.match_name.is_none() {
            c.match_name = hit.map(|h| h.title.clone());
        }
        if c.match_year.is_none() {
            c.match_year = hit.and_then(|h| h.year);
        }
        c.adult_pictures = fetched.adult;
        Lookup::Found(c)
    }
}

/// Whether `why`, the problem an extension's row shows, is a call left
/// running past its time ([`Limit::Stuck`]) rather than failures in a row.
pub fn is_stuck(why: &str) -> bool {
    why.contains(&CallError::Limit(Limit::Stuck, String::new()).to_string())
}

fn lookup_of<T>(out: Result<Lookup<T>, CallError>) -> Lookup<T> {
    match out {
        Ok(l) => l,
        Err(CallError::Unreachable(_)) => Lookup::Unreachable,
        // Other extensions' stuck threads kept it from starting: not asked.
        Err(CallError::Busy(_)) => Lookup::Skipped,
        Err(_) => Lookup::NotFound,
    }
}

/// The game as a script sees it (see `docs/EXTENSIONS.md`).
pub fn game_json(game: &SourceGame) -> Value {
    let mut ids = Map::new();
    if let Some(s) = game.steam_app_id.as_deref().filter(|s| !s.is_empty()) {
        ids.insert("steam".into(), json!(s));
    }
    if let Some(i) = game.igdb_id.filter(|i| *i > 0) {
        ids.insert("igdb".into(), json!(i.to_string()));
    }
    if let Some(s) = game.lutris_slug.as_deref().filter(|s| !s.is_empty()) {
        ids.insert("lutris".into(), json!(s));
    }
    let store = game.store.clone().or_else(|| {
        match game.runner.as_str() {
            "legendary" => Some("epic"),
            "gog" => Some("gog"),
            "nile" => Some("amazon"),
            _ => None,
        }
        .map(str::to_owned)
    });
    json!({
        "title": game.title,
        "platform": game.platform,
        "year": game.year,
        "fileName": game.file_name,
        "store": store,
        "ids": ids,
        "hashes": game.rom_hashes,
        "fields": game.fields,
    })
}

impl MetadataSource for ScriptSource {
    fn descriptor(&self) -> &SourceDescriptor {
        &self.descriptor
    }

    fn lookup(&self, ctx: &SourceContext<'_>, game: &SourceGame) -> Lookup<MetadataCandidate> {
        if self.lacks_required(ctx) {
            return Lookup::Skipped;
        }
        let input = game_json(game);
        let (pictures, title, year) = (self.pictures.clone(), game.title.clone(), game.year);
        let out = self.session(ctx, game.language.as_deref(), "find", move |calls| {
            let found = calls.call("find", Some(input.clone()))?;
            let ids = input["ids"].as_object().cloned().unwrap_or_default();
            let hashes = input["hashes"].as_object().cloned().unwrap_or_default();
            let keys = GameKeys { title: &title, year, ids: &ids, hashes: &hashes };
            let Some(best) = record::hits(&found, &keys, &pictures).into_iter().next() else {
                return match calls.unreachable() {
                    Some(why) => Err(CallError::Unreachable(why)),
                    None => Ok(None),
                };
            };
            let value = calls.call("fetch", Some(json!(best.id)))?;
            Ok(Some((best, value)))
        });
        lookup_of(out.map(|found| match found {
            Some((hit, value)) => self.candidate(game, Some(&hit), &hit.id, &value, hit.matched_by),
            None => Lookup::NotFound,
        }))
    }

    fn lookup_id(&self, ctx: &SourceContext<'_>, game: &SourceGame, id: &str) -> Lookup<MetadataCandidate> {
        let id = id.trim();
        if id.is_empty() || self.lacks_required(ctx) {
            return Lookup::Skipped;
        }
        let arg = json!(id);
        let out = self.session(ctx, game.language.as_deref(), "fetch", move |calls| calls.call("fetch", Some(arg)));
        lookup_of(out.map(|value| self.candidate(game, None, id, &value, MatchedBy::SourceId)))
    }

    fn search(&self, ctx: &SourceContext<'_>, query: &str) -> Lookup<Vec<SourceHit>> {
        if self.lacks_required(ctx) {
            return Lookup::Skipped;
        }
        let input = json!({ "title": query, "ids": {}, "hashes": {} });
        let out = self.session(ctx, None, "search", move |calls| calls.call("find", Some(input)));
        let (empty_ids, empty_hashes) = (Map::new(), Map::new());
        let keys = GameKeys { title: query, year: None, ids: &empty_ids, hashes: &empty_hashes };
        lookup_of(out.map(|value| {
            let hits: Vec<SourceHit> = record::hits(&value, &keys, &self.pictures)
                .into_iter()
                .map(|h| SourceHit {
                    source: self.descriptor.id.clone(),
                    id: Some(h.id),
                    title: Some(h.title),
                    year: h.year,
                    platform: h.platform,
                    thumb_url: h.thumb,
                    matched_by: None,
                    alt_title: h.alt_title,
                    thumb_adult: self.manifest.adult || h.thumb_adult,
                })
                .collect();
            Lookup::Found(hits)
        }))
    }

    fn has_test(&self) -> bool {
        self.script.exports("test")
    }

    /// `test(ctx)`: `true` (or nothing) is ok, `false` failed, a string is
    /// what failed.
    fn test(&self, ctx: &SourceContext<'_>) -> Option<SourceTest> {
        if !self.has_test() {
            return None;
        }
        let out = self.session(ctx, None, "test", |calls| calls.call("test", None));
        Some(match out {
            Ok(Value::Bool(true) | Value::Null) => SourceTest { ok: true, error: None },
            Ok(Value::String(s)) if !s.trim().is_empty() => SourceTest { ok: false, error: Some(scrub(&s).chars().take(300).collect()) },
            Ok(_) => SourceTest { ok: false, error: None },
            Err(e) => SourceTest { ok: false, error: Some(scrub(&e.to_string())) },
        })
    }

    fn health(&self) -> SourceHealth {
        match self.failing.lock().clone() {
            Some(why) => SourceHealth::Failing(why),
            None => SourceHealth::Ok,
        }
    }

    fn reset_health(&self) {
        self.in_a_row.store(0, Ordering::Relaxed);
        if self.failing.lock().take().is_some() {
            if let Some((dir, sha256)) = &self.dir {
                store::set_failing(dir, sha256, None);
            }
        }
    }
}

/// An installed extension that could not be loaded: listed (with what the
/// manifest said, when it could be read) so the user sees it and why;
/// never asked.
pub struct BrokenExtension {
    descriptor: SourceDescriptor,
    why: String,
}

impl BrokenExtension {
    /// `descriptor`: the manifest's, else a stand-in named by its folder.
    pub fn new(id: &str, descriptor: Option<SourceDescriptor>, why: &ExtensionError) -> Self {
        let descriptor = descriptor.unwrap_or_else(|| {
            serde_json::from_value(
                json!({ "id": id, "name": id, "kind": "details", "network": true, "extension": {"version": "", "author": ""} }),
            )
            .expect("a plain descriptor")
        });
        Self { descriptor, why: why.to_string() }
    }
}

impl MetadataSource for BrokenExtension {
    fn descriptor(&self) -> &SourceDescriptor {
        &self.descriptor
    }

    fn lookup(&self, _ctx: &SourceContext<'_>, _game: &SourceGame) -> Lookup<MetadataCandidate> {
        Lookup::Skipped
    }

    fn health(&self) -> SourceHealth {
        SourceHealth::LoadFailed(self.why.clone())
    }
}
