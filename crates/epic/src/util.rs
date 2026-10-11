//! Small helpers ported from Heroic's `backend/utils.ts`.

use std::sync::OnceLock;

use regex::Regex;

/// `getFileSize` (`filesize` with `base: 2`): IEC units, two decimals with
/// trailing zeros dropped (`25.87 GiB`, `1 GiB`, `512 B`).
pub fn file_size(bytes: f64) -> String {
    const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    if !bytes.is_finite() || bytes <= 0. {
        return "0 B".into();
    }
    let mut value = bytes;
    let mut unit = 0;
    while value >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    // `Number(value.toFixed(2))`; a rounding up to 1024 moves to the next unit.
    let mut rounded = (value * 100.).round() / 100.;
    if rounded >= 1024. && unit < UNITS.len() - 1 {
        rounded = ((rounded / 1024.) * 100.).round() / 100.;
        unit += 1;
    }
    let text = if unit == 0 { format!("{}", rounded as u64) } else { trim_decimals(rounded) };
    format!("{text} {}", UNITS[unit])
}

fn trim_decimals(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn special_chars() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"('[A-Za-z0-9_])|(\\[A-Za-z0-9_]{5})|(\\"(\\.|[^"])*")|[^()|0-9a-zA-Z\s]"#).expect("valid regex"))
}

/// `cleanTitle`.
pub fn clean_title(title: &str) -> String {
    let t = special_chars().replace_all(title, "").replace(' ', "-").replace('®', "").to_lowercase();
    t.split("--definitive").next().unwrap_or("").to_string()
}

/// `formatEpicStoreUrl`.
pub fn epic_store_url(title: &str) -> String {
    format!("https://www.epicgames.com/store/product/{}", clean_title(title))
}

/// `removeSpecialcharacters`.
pub fn remove_special_characters(text: &str) -> String {
    text.chars().filter(|c| !":|/*?<>\\&{}%$@`!™+'\"®".contains(*c)).collect()
}

/// JavaScript's `Date()` string, as `CacheStore` stores it in `__timestamp`.
pub fn js_date_now() -> String {
    chrono::Local::now().format("%a %b %d %Y %H:%M:%S GMT%z (Local Time)").to_string()
}

/// Parses what `js_date_now` (or Node's `Date()`) wrote into a unix time in
/// seconds. Lenient: only needs the fixed prefix and the numeric offset.
pub fn parse_js_date(s: &str) -> Option<i64> {
    let head = s.split(" (").next()?.trim();
    // "Wed Oct 07 2026 23:18:14 GMT-0300"
    chrono::DateTime::parse_from_str(head, "%a %b %d %Y %H:%M:%S GMT%z").ok().map(|d| d.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_sizes_match_filesize_js() {
        assert_eq!(file_size(0.), "0 B");
        assert_eq!(file_size(512.), "512 B");
        assert_eq!(file_size(1024.), "1 KiB");
        assert_eq!(file_size(1536.), "1.5 KiB");
        assert_eq!(file_size(45010292103.), "41.92 GiB");
        assert_eq!(file_size(27783660544.), "25.88 GiB");
        assert_eq!(file_size(1024. * 1024. * 1024.), "1 GiB");
    }

    #[test]
    fn titles_clean_like_heroic() {
        assert_eq!(clean_title("Control"), "control");
        assert_eq!(clean_title("Train Sim World® 7: Sand Patch Grade"), "train-sim-world-7-sand-patch-grade");
        assert_eq!(clean_title("Assassin's Creed"), "assassin-creed");
        assert_eq!(clean_title("Grand Theft Auto V (Premium)"), "grand-theft-auto-v-(premium)");
        assert_eq!(clean_title("Metro Exodus - Definitive Edition"), "metro-exodus");
        assert_eq!(epic_store_url("Control"), "https://www.epicgames.com/store/product/control");
    }

    #[test]
    fn js_dates_round_trip() {
        let d = "Wed Oct 07 2026 23:18:14 GMT-0300 (Brasilia Standard Time)";
        assert_eq!(parse_js_date(d), Some(1791425894));
        let now = js_date_now();
        assert!(parse_js_date(&now).is_some(), "{now}");
    }

    #[test]
    fn special_characters_removed() {
        assert_eq!(remove_special_characters("Metro: Exodus™ \"X\""), "Metro Exodus X");
    }
}
