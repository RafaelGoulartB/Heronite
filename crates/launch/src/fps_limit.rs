//! One FPS limit per game, applied by one limiter: MangoHud, DXVK /
//! VKD3D-Proton or gamescope. Two limiters at once fight each other (the
//! frame rate wobbles under the cap), so the launch only ever uses one.
//!
//! The user can pick the limiter or leave it on [`Method::Auto`]; a picked
//! limiter that cannot run on this launch falls back to the automatic one.
//!
//! Keys (`GamesConfig/<app>.json`, `config.json` -> `defaultSettings`):
//! [`KEY_LIMIT`], [`KEY_METHOD`], [`KEY_SMOOTH`], [`KEY_NO_FOCUS`]. Heroic
//! has three separate limiters (`enableDXVKFpsLimit` + `DXVKFpsCap`, the
//! `gamescope` object's `enableLimiter` + `fpsLimiter` + `fpsLimiterNoFocus`)
//! and Heronite added `heroniteMangohudFpsLimit`; they are read when the new
//! keys hold no limit.

use serde_json::{Map, Value};

use crate::probe::Probe;
use crate::settings::GameSettings;

/// Frames per second, digits; empty or `0` = no limit.
pub const KEY_LIMIT: &str = "heroniteFpsLimit";
/// [`Method::id`].
pub const KEY_METHOD: &str = "heroniteFpsLimitMethod";
/// MangoHud waits before presenting a frame (`fps_limit_method=early`):
/// smoother frame times for a little more latency.
pub const KEY_SMOOTH: &str = "heroniteFpsLimitSmooth";
/// gamescope's limit while the game is not focused (`-o`); empty = none.
pub const KEY_NO_FOCUS: &str = "heroniteFpsLimitNoFocus";

/// The settings the limit replaces; the panel clears them when it writes one.
pub const LEGACY_DXVK: (&str, &str) = ("enableDXVKFpsLimit", "DXVKFpsCap");
pub const LEGACY_MANGOHUD: &str = "heroniteMangohudFpsLimit";
/// Fields of the `gamescope` object.
pub const LEGACY_GAMESCOPE: [&str; 3] = ["enableLimiter", "fpsLimiter", "fpsLimiterNoFocus"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Method {
    #[default]
    Auto,
    MangoHud,
    /// `DXVK_FRAME_RATE` (DirectX 8-11) and `VKD3D_FRAME_RATE` (DirectX 12).
    Dxvk,
    /// `gamescope -r`: the game sees a monitor of that refresh rate.
    Gamescope,
}

impl Method {
    pub const ALL: [Method; 4] = [Method::Auto, Method::MangoHud, Method::Dxvk, Method::Gamescope];

    pub fn id(self) -> &'static str {
        match self {
            Method::Auto => "auto",
            Method::MangoHud => "mangohud",
            Method::Dxvk => "dxvk",
            Method::Gamescope => "gamescope",
        }
    }

    /// Unknown or missing = [`Method::Auto`].
    pub fn parse(id: &str) -> Method {
        Method::ALL.into_iter().find(|m| m.id() == id).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FpsLimit {
    /// `0` = no limit.
    pub limit: u32,
    pub method: Method,
    pub smooth: bool,
    /// `0` = none.
    pub no_focus: u32,
}

impl FpsLimit {
    /// The limit in `m` (merged settings or a Steam game's parsed line).
    pub fn from_map(m: &Map<String, Value>) -> Self {
        let mut s = FpsLimit {
            limit: number(m.get(KEY_LIMIT)),
            method: Method::parse(m.get(KEY_METHOD).and_then(Value::as_str).unwrap_or("")),
            smooth: m.get(KEY_SMOOTH) == Some(&Value::Bool(true)),
            no_focus: number(m.get(KEY_NO_FOCUS)),
        };
        if s.limit == 0 {
            if let Some((method, limit, no_focus)) = legacy(m) {
                (s.method, s.limit, s.no_focus) = (method, limit, no_focus);
            }
        }
        s
    }

    pub fn is_on(&self) -> bool {
        self.limit > 0
    }

    /// What limits the game with `tools`; `None` without a limit or when
    /// nothing can apply it.
    pub fn applied(&self, tools: &Tools) -> Option<Applied> {
        if !self.is_on() {
            return None;
        }
        let method = tools.resolve(self.method)?;
        Some(Applied { method, limit: self.limit, smooth: self.smooth, no_focus: self.no_focus })
    }
}

/// Heroic's and Heronite's earlier limiters, gamescope's first (it ran the
/// game in gamescope, which the user would notice losing).
fn legacy(m: &Map<String, Value>) -> Option<(Method, u32, u32)> {
    if let Some(Value::Object(g)) = m.get("gamescope") {
        let fps = number(g.get(LEGACY_GAMESCOPE[1]));
        if truthy(g.get(LEGACY_GAMESCOPE[0])) && fps > 0 {
            return Some((Method::Gamescope, fps, number(g.get(LEGACY_GAMESCOPE[2]))));
        }
    }
    let mangohud = number(m.get(LEGACY_MANGOHUD));
    if truthy(m.get("showMangohud")) && mangohud > 0 {
        return Some((Method::MangoHud, mangohud, 0));
    }
    let dxvk = number(m.get(LEGACY_DXVK.1));
    (truthy(m.get(LEGACY_DXVK.0)) && dxvk > 0).then_some((Method::Dxvk, dxvk, 0))
}

fn number(v: Option<&Value>) -> u32 {
    match v {
        Some(Value::Number(n)) => n.as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
        _ => false,
    }
}

/// The limit a launch applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    /// Never [`Method::Auto`].
    pub method: Method,
    pub limit: u32,
    pub smooth: bool,
    pub no_focus: u32,
}

impl Applied {
    pub fn by(&self, method: Method) -> bool {
        self.method == method
    }
}

/// What can limit one game's frame rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tools {
    /// gamescope runs the game anyway (its upscaling is on).
    pub gamescope_runs: bool,
    pub gamescope: bool,
    pub mangohud: bool,
    /// The game draws through DXVK or VKD3D-Proton (Wine / Proton).
    pub dxvk: bool,
}

impl Tools {
    /// Inside gamescope MangoHud is drawn by gamescope (`--mangoapp`), away
    /// from the game, and cannot hold its frames.
    pub fn can(&self, method: Method) -> bool {
        match method {
            Method::Auto => self.auto().is_some(),
            Method::MangoHud => self.mangohud && !self.gamescope_runs,
            Method::Dxvk => self.dxvk,
            Method::Gamescope => self.gamescope,
        }
    }

    /// gamescope when it runs anyway (adding another limiter would stack
    /// on its own pacing), else MangoHud (every graphics API, latency or
    /// smoothness), else DXVK (comes with Proton, DirectX only).
    pub fn auto(&self) -> Option<Method> {
        if self.gamescope_runs {
            Some(Method::Gamescope)
        } else if self.mangohud {
            Some(Method::MangoHud)
        } else if self.dxvk {
            Some(Method::Dxvk)
        } else {
            None
        }
    }

    /// `method`, or the automatic one when it cannot run.
    pub fn resolve(&self, method: Method) -> Option<Method> {
        match method {
            Method::Auto => self.auto(),
            m if self.can(m) => Some(m),
            _ => self.auto(),
        }
    }

    /// The tools of a launch from Heronite. On a Steam Deck the session's
    /// gamescope and MangoHud are Steam's, as in [`crate::env::build_wrappers`].
    pub fn detect(s: &GameSettings, probe: &dyn Probe) -> Tools {
        let deck = probe.env_var("SteamDeck").as_deref() == Some("1");
        let gamescope = !deck && probe.which("gamescope").is_some();
        let w = &s.wine_version;
        let wined3d = s.env_options.iter().any(|(k, v)| k == "PROTON_USE_WINED3D" && v == "1");
        Tools {
            gamescope_runs: gamescope && s.gamescope.enable_upscaling,
            gamescope,
            mangohud: !deck && probe.which("mangohud").is_some(),
            dxvk: uses_dxvk(&w.kind, &w.name, &w.bin, s.auto_install_dxvk, wined3d),
        }
    }
}

/// Whether a game run by this Wine build draws through DXVK / VKD3D-Proton:
/// Proton always (unless told to use WineD3D), plain Wine once Heronite
/// installed DXVK into the prefix.
pub fn uses_dxvk(kind: &str, name: &str, bin: &str, auto_install_dxvk: bool, wined3d: bool) -> bool {
    if wined3d || bin.contains("toolkit") || name.ends_with("-DXMT") {
        return false;
    }
    match kind {
        "proton" => true,
        "wine" => auto_install_dxvk,
        _ => false,
    }
}

/// `MANGOHUD_CONFIG` for a MangoHud limit. `existing`: the config the user
/// set, extended (it already says whether the config file is read); without
/// one `read_cfg` keeps the user's config file. `hud`: the overlay is shown;
/// off, MangoHud only limits (`no_display`, its toggle key still shows it).
pub fn mangohud_config(a: &Applied, hud: bool, existing: Option<&str>) -> String {
    let mut opts: Vec<String> = Vec::new();
    if !hud {
        opts.push("no_display".into());
    }
    opts.push(format!("fps_limit={}", a.limit));
    if a.smooth {
        opts.push("fps_limit_method=early".into());
    }
    let opts = opts.join(",");
    match existing.map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => format!("{},{opts}", c.trim_end_matches(',')),
        None => format!("read_cfg,{opts}"),
    }
}

/// The limit in a `MANGOHUD_CONFIG` written by [`mangohud_config`] without
/// an `existing` config: `(limit, smooth, hud)`.
pub fn read_mangohud_config(config: &str) -> Option<(u32, bool, bool)> {
    let mut parts = config.split(',');
    if parts.next()? != "read_cfg" {
        return None;
    }
    let mut parts = parts.peekable();
    let hud = parts.next_if_eq(&"no_display").is_none();
    let limit = parts.next()?.strip_prefix("fps_limit=")?;
    if limit.is_empty() || !limit.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let smooth = parts.next_if_eq(&"fps_limit_method=early").is_some();
    if parts.next().is_some() {
        return None;
    }
    Some((limit.parse().ok().filter(|n| *n > 0)?, smooth, hud))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn limit(v: serde_json::Value) -> FpsLimit {
        FpsLimit::from_map(v.as_object().unwrap())
    }

    #[test]
    fn reads_the_new_keys() {
        let s = limit(json!({KEY_LIMIT: "60", KEY_METHOD: "dxvk", KEY_SMOOTH: true, KEY_NO_FOCUS: 15}));
        assert_eq!(s, FpsLimit { limit: 60, method: Method::Dxvk, smooth: true, no_focus: 15 });
        assert!(!limit(json!({KEY_LIMIT: "", KEY_METHOD: "mangohud"})).is_on());
        assert_eq!(limit(json!({KEY_LIMIT: "30", KEY_METHOD: "nope"})).method, Method::Auto);
    }

    #[test]
    fn reads_the_old_limiters_when_there_is_no_limit() {
        let gs = json!({"gamescope": {"enableLimiter": true, "fpsLimiter": "40", "fpsLimiterNoFocus": "10"}, "enableDXVKFpsLimit": true, "DXVKFpsCap": "60"});
        assert_eq!(limit(gs), FpsLimit { limit: 40, method: Method::Gamescope, smooth: false, no_focus: 10 });
        let mango = json!({"showMangohud": true, "heroniteMangohudFpsLimit": "72", "enableDXVKFpsLimit": true, "DXVKFpsCap": "60"});
        assert_eq!(limit(mango).method, Method::MangoHud);
        // MangoHud's limit only applied with the overlay on.
        let off = json!({"showMangohud": false, "heroniteMangohudFpsLimit": "72", "enableDXVKFpsLimit": true, "DXVKFpsCap": "60"});
        assert_eq!((limit(off.clone()).method, limit(off).limit), (Method::Dxvk, 60));
        assert!(!limit(json!({"enableDXVKFpsLimit": false, "DXVKFpsCap": "60"})).is_on());
        // A limit set with the new keys wins.
        let both = json!({KEY_LIMIT: "90", KEY_METHOD: "auto", "enableDXVKFpsLimit": true, "DXVKFpsCap": "60"});
        assert_eq!((limit(both.clone()).method, limit(both).limit), (Method::Auto, 90));
    }

    #[test]
    fn picks_one_limiter() {
        let all = Tools { gamescope_runs: false, gamescope: true, mangohud: true, dxvk: true };
        assert_eq!(all.auto(), Some(Method::MangoHud));
        assert_eq!(Tools { gamescope_runs: true, ..all }.auto(), Some(Method::Gamescope));
        assert_eq!(Tools { mangohud: false, ..all }.auto(), Some(Method::Dxvk));
        assert_eq!(Tools::default().auto(), None);
        // A pick that can run is kept; one that cannot falls back.
        assert_eq!(all.resolve(Method::Dxvk), Some(Method::Dxvk));
        assert_eq!(Tools { gamescope_runs: true, ..all }.resolve(Method::MangoHud), Some(Method::Gamescope));
        assert_eq!(Tools { dxvk: false, ..all }.resolve(Method::Dxvk), Some(Method::MangoHud));
        // gamescope can be picked even when only the limit would run it.
        assert_eq!(all.resolve(Method::Gamescope), Some(Method::Gamescope));
    }

    #[test]
    fn dxvk_runs_under_proton_and_installed_wine() {
        assert!(uses_dxvk("proton", "GE-Proton9", "/p/proton", false, false));
        assert!(!uses_dxvk("proton", "GE-Proton9", "/p/proton", false, true));
        assert!(!uses_dxvk("wine", "Wine", "/usr/bin/wine", false, false));
        assert!(uses_dxvk("wine", "Wine", "/usr/bin/wine", true, false));
        assert!(!uses_dxvk("crossover", "CrossOver", "/c/wine", true, false));
    }

    #[test]
    fn mangohud_config_round_trips() {
        let a = Applied { method: Method::MangoHud, limit: 60, smooth: false, no_focus: 0 };
        assert_eq!(mangohud_config(&a, true, None), "read_cfg,fps_limit=60");
        let smooth = Applied { smooth: true, ..a };
        assert_eq!(mangohud_config(&smooth, false, None), "read_cfg,no_display,fps_limit=60,fps_limit_method=early");
        assert_eq!(mangohud_config(&a, true, Some("cpu_temp,")), "cpu_temp,fps_limit=60");
        assert_eq!(read_mangohud_config("read_cfg,fps_limit=60"), Some((60, false, true)));
        assert_eq!(read_mangohud_config("read_cfg,no_display,fps_limit=60,fps_limit_method=early"), Some((60, true, false)));
        assert_eq!(read_mangohud_config("read_cfg,fps_limit=60,cpu_temp"), None);
        assert_eq!(read_mangohud_config("fps_limit=60"), None);
    }
}
