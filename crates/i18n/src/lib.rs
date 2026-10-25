//! Interface language. The English text is the key: `tr!("Play")` is "Play"
//! in English and its entry in `locales/<lang>/*.json` in the other languages
//! (`pt-BR`, `es`; English again when there is none, so nothing ever shows up
//! empty).
//!
//! - `tr!("Play")` gives a `&'static str`;
//! - `tr!("{n} of {total} games", n = done, total)` fills named placeholders
//!   (`name` alone stands for `name = name`) and gives a `String`;
//! - `trn!(count, "{n} game", "{n} games")` picks the singular or the plural
//!   form, `{n}` being `count`;
//! - `trc!("verb", "Play")` is a key with a context, for the rare English text
//!   that needs two translations (`"verb|Play"` in the JSON);
//! - `n_!("Slate")` marks a literal translated later with [`tr()`] (constant
//!   tables), so `tests/coverage.rs` still sees it.
//!
//! `tests/coverage.rs` scans the workspace: every key needs a translation,
//! every translation a key, and placeholders must match.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    En,
    PtBr,
    Es,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::En, Lang::PtBr, Lang::Es];
    /// The languages with a `locales/` folder.
    pub const TRANSLATED: [Lang; 2] = [Lang::PtBr, Lang::Es];

    /// BCP 47 tag, also what the settings file stores.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::PtBr => "pt-BR",
            Lang::Es => "es",
        }
    }

    /// The name in the language itself (a picker shows it untranslated).
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::PtBr => "Português (Brasil)",
            Lang::Es => "Español",
        }
    }

    /// A language or locale code (`pt-BR`, `pt_BR.UTF-8`, `en_US`, `es_MX`...);
    /// any Portuguese is Brazilian Portuguese and any Spanish the neutral `es`,
    /// the only ones there are.
    pub fn parse(code: &str) -> Option<Lang> {
        let base = code.trim().split(['.', '@']).next().unwrap_or("");
        let lang = base.split(['_', '-']).next().unwrap_or("").to_ascii_lowercase();
        match lang.as_str() {
            "en" => Some(Lang::En),
            "pt" => Some(Lang::PtBr),
            "es" => Some(Lang::Es),
            _ => None,
        }
    }

    fn from_u8(v: u8) -> Lang {
        match v {
            1 => Lang::PtBr,
            2 => Lang::Es,
            _ => Lang::En,
        }
    }

    /// Writes weekdays and months in lower case in the middle of a sentence.
    pub fn lowercase_names(self) -> bool {
        self != Lang::En
    }

    /// Decimal separator (`1.5` / `1,5`).
    pub fn decimal_separator(self) -> char {
        match self {
            Lang::En => '.',
            Lang::PtBr | Lang::Es => ',',
        }
    }

    /// Uses a 24-hour clock.
    pub fn clock_24h(self) -> bool {
        self != Lang::En
    }
}

/// What the user picked: follow the system, or a fixed language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Preference {
    #[default]
    System,
    Fixed(Lang),
}

impl Preference {
    pub fn as_str(self) -> &'static str {
        match self {
            Preference::System => "system",
            Preference::Fixed(l) => l.code(),
        }
    }

    /// Lenient: anything unknown follows the system.
    pub fn parse(s: &str) -> Preference {
        match s {
            "system" | "" => Preference::System,
            other => Lang::parse(other).map(Preference::Fixed).unwrap_or_default(),
        }
    }

    pub fn resolve(self) -> Lang {
        match self {
            Preference::System => system_lang(),
            Preference::Fixed(l) => l,
        }
    }
}

/// The language of the session (English when it is not one we have).
pub fn system_lang() -> Lang {
    lang_from_env(|k| std::env::var(k).ok())
}

/// gettext's order: `LANGUAGE` (a `:` list, the first language we have wins)
/// unless the locale is `C`, then `LC_ALL`, `LC_MESSAGES`, `LANG`.
pub fn lang_from_env(get: impl Fn(&str) -> Option<String>) -> Lang {
    let get = |k: &str| get(k).filter(|v| !v.trim().is_empty());
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].into_iter().find_map(get);
    let c_locale = locale.as_deref().is_some_and(|l| matches!(l.split('.').next(), Some("C" | "POSIX")));
    if !c_locale {
        if let Some(list) = get("LANGUAGE") {
            if let Some(l) = list.split(':').find_map(Lang::parse) {
                return l;
            }
        }
    }
    locale.as_deref().and_then(Lang::parse).unwrap_or(Lang::En)
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn current() -> Lang {
    Lang::from_u8(CURRENT.load(Ordering::Relaxed))
}

/// Switches the language of everything translated from now on (the caller
/// redraws).
pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

/// Every translation of `lang`, loaded the first time it is asked for
/// (`None` for English, the keys themselves).
fn translations(lang: Lang) -> Option<&'static HashMap<String, String>> {
    static PT_BR_MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    static ES_MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    let cell = match lang {
        Lang::En => return None,
        Lang::PtBr => &PT_BR_MAP,
        Lang::Es => &ES_MAP,
    };
    Some(cell.get_or_init(|| {
        let mut map = HashMap::new();
        for (_, text) in embedded_files(lang) {
            // `tests/coverage.rs` makes sure every file parses.
            if let Ok(file) = serde_json::from_str::<HashMap<String, String>>(text) {
                map.extend(file.into_iter().filter(|(_, v)| !v.is_empty()));
            }
        }
        map
    }))
}

/// The files embedded for `lang` (name, JSON text), for the tests.
pub fn embedded_files(lang: Lang) -> &'static [(&'static str, &'static str)] {
    match lang {
        Lang::En => &[],
        Lang::PtBr => PT_BR,
        Lang::Es => ES,
    }
}

/// `english` in the current language. Prefer the `tr!` macro, which the
/// coverage test can see; this is for text that went through `n_!`.
pub fn tr(english: &str) -> &str {
    match translations(current()) {
        None => english,
        Some(map) => map.get(english).map(String::as_str).unwrap_or(english),
    }
}

/// [`tr`] for a `context|text` key; English is `english`.
pub fn tr_ctx<'a>(key: &str, english: &'a str) -> &'a str {
    match translations(current()) {
        None => english,
        Some(map) => map.get(key).map(String::as_str).unwrap_or(english),
    }
}

/// Replaces `{name}` with the matching argument (`{{` and `}}` are braces).
/// Unknown names stay as they are. No format specs: format the value first.
pub fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        if tail.starts_with('{') {
            if let Some(end) = tail.find('}') {
                let name = &tail[1..end];
                if let Some((_, v)) = args.iter().find(|(k, _)| *k == name) {
                    use std::fmt::Write;
                    let _ = write!(out, "{v}");
                    rest = &tail[end + 1..];
                    continue;
                }
            }
        }
        out.push_str(&tail[..1]);
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

/// A count for [`trn!`]: one picks the singular.
pub trait Count {
    fn is_one(&self) -> bool;
}

macro_rules! count_ints {
    ($($t:ty),*) => {$(
        impl Count for $t {
            fn is_one(&self) -> bool {
                *self == 1
            }
        }
    )*};
}
count_ints!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);

impl Count for f32 {
    fn is_one(&self) -> bool {
        *self == 1.
    }
}

impl Count for f64 {
    fn is_one(&self) -> bool {
        *self == 1.
    }
}

impl<T: Count + ?Sized> Count for &T {
    fn is_one(&self) -> bool {
        (**self).is_one()
    }
}

/// Singular or plural (every language here says "1 juego" and "0 juegos").
pub fn plural<'a>(n: &dyn Count, one: &'a str, other: &'a str) -> &'a str {
    if n.is_one() {
        one
    } else {
        other
    }
}

/// `v` with `digits` decimals and the language's decimal separator
/// (`1.5` / `1,5`).
pub fn decimal(v: f64, digits: usize) -> String {
    let s = format!("{v:.digits$}");
    match current().decimal_separator() {
        '.' => s,
        sep => s.replace('.', &sep.to_string()),
    }
}

/// An integer with thousands separators (`12,345` / `12.345`).
pub fn grouped(v: i64) -> String {
    let sep = match current() {
        Lang::En => ',',
        Lang::PtBr | Lang::Es => '.',
    };
    let digits = v.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if v < 0 {
        out.push('-');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// Full month name, `month` 1-12 (clamped).
pub fn month_name(month: u32) -> &'static str {
    match month.clamp(1, 12) {
        1 => tr!("January"),
        2 => tr!("February"),
        3 => tr!("March"),
        4 => tr!("April"),
        5 => tr!("May"),
        6 => tr!("June"),
        7 => tr!("July"),
        8 => tr!("August"),
        9 => tr!("September"),
        10 => tr!("October"),
        11 => tr!("November"),
        _ => tr!("December"),
    }
}

/// Three-letter month, `month` 1-12 (clamped).
pub fn month_short(month: u32) -> &'static str {
    match month.clamp(1, 12) {
        1 => trc!("month", "Jan"),
        2 => trc!("month", "Feb"),
        3 => trc!("month", "Mar"),
        4 => trc!("month", "Apr"),
        5 => trc!("month", "May"),
        6 => trc!("month", "Jun"),
        7 => trc!("month", "Jul"),
        8 => trc!("month", "Aug"),
        9 => trc!("month", "Sep"),
        10 => trc!("month", "Oct"),
        11 => trc!("month", "Nov"),
        _ => trc!("month", "Dec"),
    }
}

/// Weekday name, 0 = Monday (wraps).
pub fn weekday_name(day: u32) -> &'static str {
    match day % 7 {
        0 => tr!("Monday"),
        1 => tr!("Tuesday"),
        2 => tr!("Wednesday"),
        3 => tr!("Thursday"),
        4 => tr!("Friday"),
        5 => tr!("Saturday"),
        _ => tr!("Sunday"),
    }
}

/// Three-letter weekday, 0 = Monday (wraps).
pub fn weekday_short(day: u32) -> &'static str {
    match day % 7 {
        0 => trc!("weekday", "Mon"),
        1 => trc!("weekday", "Tue"),
        2 => trc!("weekday", "Wed"),
        3 => trc!("weekday", "Thu"),
        4 => trc!("weekday", "Fri"),
        5 => trc!("weekday", "Sat"),
        _ => trc!("weekday", "Sun"),
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! __arg {
    ($k:ident) => {
        $k
    };
    ($k:ident, $v:expr) => {
        $v
    };
}

/// See the crate docs.
#[macro_export]
macro_rules! tr {
    ($s:literal) => {
        $crate::tr($s)
    };
    ($s:literal, $($k:ident $(= $v:expr)?),+ $(,)?) => {
        $crate::fill($crate::tr($s), &[$((stringify!($k), &$crate::__arg!($k $(, $v)?) as &dyn ::std::fmt::Display)),+])
    };
}

/// See the crate docs.
#[macro_export]
macro_rules! trn {
    ($n:expr, $one:literal, $other:literal $(, $k:ident $(= $v:expr)?)* $(,)?) => {{
        let n = $n;
        $crate::fill(
            $crate::plural(&n, $crate::tr($one), $crate::tr($other)),
            &[("n", &n as &dyn ::std::fmt::Display) $(, (stringify!($k), &$crate::__arg!($k $(, $v)?) as &dyn ::std::fmt::Display))*],
        )
    }};
}

/// See the crate docs.
#[macro_export]
macro_rules! trc {
    ($ctx:literal, $s:literal) => {
        $crate::tr_ctx(concat!($ctx, "|", $s), $s)
    };
    ($ctx:literal, $s:literal, $($k:ident $(= $v:expr)?),+ $(,)?) => {
        $crate::fill($crate::tr_ctx(concat!($ctx, "|", $s), $s), &[$((stringify!($k), &$crate::__arg!($k $(, $v)?) as &dyn ::std::fmt::Display)),+])
    };
}

/// See the crate docs.
#[macro_export]
macro_rules! n_ {
    ($s:literal) => {
        $s
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codes_and_locales() {
        assert_eq!(Lang::parse("pt_BR.UTF-8"), Some(Lang::PtBr));
        assert_eq!(Lang::parse("pt-PT"), Some(Lang::PtBr));
        assert_eq!(Lang::parse("PT"), Some(Lang::PtBr));
        assert_eq!(Lang::parse("en_US.UTF-8@euro"), Some(Lang::En));
        assert_eq!(Lang::parse("es_MX.UTF-8"), Some(Lang::Es));
        assert_eq!(Lang::parse("es-ES"), Some(Lang::Es));
        assert_eq!(Lang::parse("fr_FR"), None);
        assert_eq!(Lang::parse("C"), None);
        assert_eq!(Preference::parse("pt-BR"), Preference::Fixed(Lang::PtBr));
        assert_eq!(Preference::parse("de"), Preference::System);
        assert_eq!(Preference::parse("es"), Preference::Fixed(Lang::Es));
        for p in [Preference::System, Preference::Fixed(Lang::En), Preference::Fixed(Lang::PtBr), Preference::Fixed(Lang::Es)] {
            assert_eq!(Preference::parse(p.as_str()), p);
        }
    }

    #[test]
    fn system_language_follows_gettext_order() {
        let env =
            |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        assert_eq!(lang_from_env(env(&[])), Lang::En);
        assert_eq!(lang_from_env(env(&[("LANG", "pt_BR.UTF-8")])), Lang::PtBr);
        assert_eq!(lang_from_env(env(&[("LANG", "pt_BR.UTF-8"), ("LC_MESSAGES", "en_US.UTF-8")])), Lang::En);
        assert_eq!(lang_from_env(env(&[("LANG", "en_US.UTF-8"), ("LC_ALL", "pt_BR.UTF-8")])), Lang::PtBr);
        assert_eq!(lang_from_env(env(&[("LANG", "en_US.UTF-8"), ("LANGUAGE", "fr:pt_BR:en")])), Lang::PtBr);
        assert_eq!(lang_from_env(env(&[("LANG", "C.UTF-8"), ("LANGUAGE", "pt_BR")])), Lang::En);
        assert_eq!(lang_from_env(env(&[("LANG", "de_DE.UTF-8")])), Lang::En);
        assert_eq!(lang_from_env(env(&[("LANG", "es_AR.UTF-8")])), Lang::Es);
        assert_eq!(lang_from_env(env(&[("LANG", "en_US.UTF-8"), ("LANGUAGE", "fr:es:en")])), Lang::Es);
        assert_eq!(lang_from_env(env(&[("LANG", ""), ("LC_ALL", "")])), Lang::En);
    }

    #[test]
    fn fill_replaces_named_placeholders() {
        assert_eq!(fill("{a} of {b}", &[("a", &1), ("b", &"two")]), "1 of two");
        assert_eq!(fill("{{a}} {a} {missing}", &[("a", &3)]), "{a} 3 {missing}");
        assert_eq!(fill("odd { brace } }", &[]), "odd { brace } }");
        assert_eq!(fill("ünï {a}çõ", &[("a", &"ã")]), "ünï ãçõ");
    }

    #[test]
    fn macros_fall_back_to_english() {
        // Only this test switches the language (tests share the process).
        set(Lang::En);
        let total = 3;
        assert_eq!(tr!("Some text no file has"), "Some text no file has");
        assert_eq!(tr!("{n} of {total}", n = 2, total), "2 of 3");
        assert_eq!(trn!(1usize, "{n} game", "{n} games"), "1 game");
        assert_eq!(trn!(&4u32, "{n} game", "{n} games in {where}", where = "x"), "4 games in x");
        assert_eq!(trc!("ctx", "Plain"), "Plain");
        assert_eq!(decimal(1.25, 1), "1.2");
        assert_eq!(grouped(-1234567), "-1,234,567");
        set(Lang::PtBr);
        assert_eq!(tr!("Some text no file has"), "Some text no file has");
        assert_eq!(decimal(1.5, 1), "1,5");
        assert_eq!(grouped(12345), "12.345");
        assert_eq!(grouped(999), "999");
        assert_eq!(current(), Lang::PtBr);
        set(Lang::Es);
        assert_eq!(current(), Lang::Es);
        assert_eq!(decimal(2.5, 1), "2,5");
        assert_eq!(grouped(12345), "12.345");
        set(Lang::En);
    }
}
