//! GOG's languages: the code the content system and gogdl's files use
//! (`en-US`), the English name v1 builds use (`English`) and the short codes
//! older data still has (`en`). The table is gogdl's.

/// One language of [`LANGUAGES`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Language {
    pub code: &'static str,
    pub name: &'static str,
    /// Older codes that mean the same.
    pub old_codes: &'static [&'static str],
}

impl Language {
    /// Whether `value` names this language (code, name or an old code, in
    /// any case).
    pub fn matches(&self, value: &str) -> bool {
        value.eq_ignore_ascii_case(self.code)
            || value.eq_ignore_ascii_case(self.name)
            || self.old_codes.iter().any(|c| value.eq_ignore_ascii_case(c))
    }
}

/// The language `value` names; `None` for `*` (every language) and unknown
/// values.
pub fn parse(value: &str) -> Option<Language> {
    if value == "*" {
        return None;
    }
    LANGUAGES.iter().find(|l| l.matches(value)).copied()
}

/// The code of `value` when it is a known language, else `value` itself
/// (`*` stays `*`).
pub fn code_of(value: &str) -> String {
    parse(value).map(|l| l.code.to_string()).unwrap_or_else(|| value.to_string())
}

const fn lang(code: &'static str, name: &'static str, old_codes: &'static [&'static str]) -> Language {
    Language { code, name, old_codes }
}

pub const LANGUAGES: &[Language] = &[
    lang("af-ZA", "Afrikaans", &[]),
    lang("ar", "Arabic", &[]),
    lang("az-AZ", "Azeri", &[]),
    lang("be-BY", "Belarusian", &["be"]),
    lang("bn-BD", "Bengali", &["bn_BD"]),
    lang("bg-BG", "Bulgarian", &["bg", "bl"]),
    lang("bs-BA", "Bosnian", &[]),
    lang("ca-ES", "Catalan", &["ca"]),
    lang("cs-CZ", "Czech", &["cz"]),
    lang("cy-GB", "Welsh", &[]),
    lang("da-DK", "Danish", &["da"]),
    lang("de-DE", "German", &["de"]),
    lang("dv-MV", "Divehi", &[]),
    lang("el-GR", "Greek", &["gk", "el-GK"]),
    lang("en-GB", "British English", &["en_GB"]),
    lang("en-US", "English", &["en"]),
    lang("es-ES", "Spanish", &["es"]),
    lang("es-MX", "Latin American Spanish", &["es_mx"]),
    lang("et-EE", "Estonian", &["et"]),
    lang("eu-ES", "Basque", &[]),
    lang("fa-IR", "Persian", &["fa"]),
    lang("fi-FI", "Finnish", &["fi"]),
    lang("fo-FO", "Faroese", &[]),
    lang("fr-FR", "French", &["fr"]),
    lang("gl-ES", "Galician", &[]),
    lang("gu-IN", "Gujarati", &["gu"]),
    lang("he-IL", "Hebrew", &["he"]),
    lang("hi-IN", "Hindi", &["hi"]),
    lang("hr-HR", "Croatian", &[]),
    lang("hu-HU", "Hungarian", &["hu"]),
    lang("hy-AM", "Armenian", &[]),
    lang("id-ID", "Indonesian", &[]),
    lang("is-IS", "Icelandic", &["is"]),
    lang("it-IT", "Italian", &["it"]),
    lang("ja-JP", "Japanese", &["jp"]),
    lang("jv-ID", "Javanese", &["jv"]),
    lang("ka-GE", "Georgian", &[]),
    lang("kk-KZ", "Kazakh", &[]),
    lang("kn-IN", "Kannada", &[]),
    lang("ko-KR", "Korean", &["ko"]),
    lang("kok-IN", "Konkani", &[]),
    lang("ky-KG", "Kyrgyz", &[]),
    lang("la", "Latin", &[]),
    lang("lt-LT", "Lithuanian", &[]),
    lang("lv-LV", "Latvian", &[]),
    lang("ml-IN", "Malayalam", &["ml"]),
    lang("mi-NZ", "Maori", &[]),
    lang("mk-MK", "Macedonian", &[]),
    lang("mn-MN", "Mongolian", &[]),
    lang("mr-IN", "Marathi", &["mr"]),
    lang("ms-MY", "Malay", &[]),
    lang("mt-MT", "Maltese", &[]),
    lang("nb-NO", "Norwegian", &["no"]),
    lang("nl-NL", "Dutch", &["nl"]),
    lang("ns-ZA", "Northern Sotho", &[]),
    lang("pa-IN", "Punjabi", &[]),
    lang("pl-PL", "Polish", &["pl"]),
    lang("ps-AR", "Pashto", &[]),
    lang("pt-BR", "Portuguese (Brazilian)", &["br"]),
    lang("pt-PT", "Portuguese", &["pt"]),
    lang("ro-RO", "Romanian", &["ro"]),
    lang("ru-RU", "Russian", &["ru"]),
    lang("sa-IN", "Sanskrit", &[]),
    lang("sk-SK", "Slovak", &["sk"]),
    lang("sl-SI", "Slovenian", &[]),
    lang("sq-AL", "Albanian", &[]),
    lang("sr-SP", "Serbian", &["sb"]),
    lang("sv-SE", "Swedish", &["sv"]),
    lang("sw-KE", "Kiswahili", &[]),
    lang("ta-IN", "Tamil", &["ta_IN"]),
    lang("te-IN", "Telugu", &["te"]),
    lang("th-TH", "Thai", &["th"]),
    lang("tl-PH", "Tagalog", &[]),
    lang("tn-ZA", "Setswana", &[]),
    lang("tr-TR", "Turkish", &["tr"]),
    lang("tt-RU", "Tatar", &[]),
    lang("uk-UA", "Ukrainian", &["uk"]),
    lang("ur-PK", "Urdu", &["ur_PK"]),
    lang("uz-UZ", "Uzbek", &[]),
    lang("vi-VN", "Vietnamese", &["vi"]),
    lang("xh-ZA", "isiXhosa", &[]),
    lang("zh-Hans", "Chinese (Simplified)", &["zh_Hans", "zh", "cn"]),
    lang("zh-Hant", "Chinese (Traditional)", &["zh_Hant"]),
    lang("zu-ZA", "isiZulu", &[]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_names_and_old_codes_all_match() {
        assert_eq!(parse("en-US").unwrap().code, "en-US");
        assert_eq!(parse("English").unwrap().code, "en-US");
        assert_eq!(parse("en").unwrap().code, "en-US");
        assert_eq!(parse("BR").unwrap().code, "pt-BR");
        assert_eq!(parse("zh").unwrap().code, "zh-Hans");
        assert!(parse("*").is_none());
        assert!(parse("Klingon").is_none());
        assert_eq!(code_of("German"), "de-DE");
        assert_eq!(code_of("*"), "*");
        assert_eq!(LANGUAGES.len(), 84);
    }
}
