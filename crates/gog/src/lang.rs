//! GOG's language codes and the English names its installers expect.

/// `(code, English name)`.
const LANGUAGES: &[(&str, &str)] = &[
    ("af-ZA", "Afrikaans"),
    ("ar", "Arabic"),
    ("az-AZ", "Azeri"),
    ("be-BY", "Belarusian"),
    ("bn-BD", "Bengali"),
    ("bg-BG", "Bulgarian"),
    ("bs-BA", "Bosnian"),
    ("ca-ES", "Catalan"),
    ("cs-CZ", "Czech"),
    ("cy-GB", "Welsh"),
    ("da-DK", "Danish"),
    ("de-DE", "German"),
    ("dv-MV", "Divehi"),
    ("el-GR", "Greek"),
    ("en-GB", "British English"),
    ("en-US", "English"),
    ("es-ES", "Spanish"),
    ("es-MX", "Latin American Spanish"),
    ("et-EE", "Estonian"),
    ("eu-ES", "Basque"),
    ("fa-IR", "Persian"),
    ("fi-FI", "Finnish"),
    ("fo-FO", "Faroese"),
    ("fr-FR", "French"),
    ("gl-ES", "Galician"),
    ("gu-IN", "Gujarati"),
    ("he-IL", "Hebrew"),
    ("hi-IN", "Hindi"),
    ("hr-HR", "Croatian"),
    ("hu-HU", "Hungarian"),
    ("hy-AM", "Armenian"),
    ("id-ID", "Indonesian"),
    ("is-IS", "Icelandic"),
    ("it-IT", "Italian"),
    ("ja-JP", "Japanese"),
    ("jv-ID", "Javanese"),
    ("ka-GE", "Georgian"),
    ("kk-KZ", "Kazakh"),
    ("kn-IN", "Kannada"),
    ("ko-KR", "Korean"),
    ("kok-IN", "Konkani"),
    ("ky-KG", "Kyrgyz"),
    ("la", "Latin"),
    ("lt-LT", "Lithuanian"),
    ("lv-LV", "Latvian"),
    ("ml-IN", "Malayalam"),
    ("mi-NZ", "Maori"),
    ("mk-MK", "Macedonian"),
    ("mn-MN", "Mongolian"),
    ("mr-IN", "Marathi"),
    ("ms-MY", "Malay"),
    ("mt-MT", "Maltese"),
    ("nb-NO", "Norwegian"),
    ("nl-NL", "Dutch"),
    ("ns-ZA", "Northern Sotho"),
    ("pa-IN", "Punjabi"),
    ("pl-PL", "Polish"),
    ("ps-AR", "Pashto"),
    ("pt-BR", "Portuguese (Brazilian)"),
    ("pt-PT", "Portuguese"),
    ("ro-RO", "Romanian"),
    ("ru-RU", "Russian"),
    ("sa-IN", "Sanskrit"),
    ("sk-SK", "Slovak"),
    ("sl-SI", "Slovenian"),
    ("sq-AL", "Albanian"),
    ("sr-SP", "Serbian"),
    ("sv-SE", "Swedish"),
    ("sw-KE", "Kiswahili"),
    ("ta-IN", "Tamil"),
    ("te-IN", "Telugu"),
    ("th-TH", "Thai"),
    ("tl-PH", "Tagalog"),
    ("tn-ZA", "Setswana"),
    ("tr-TR", "Turkish"),
    ("tt-RU", "Tatar"),
    ("uk-UA", "Ukrainian"),
    ("ur-PK", "Urdu"),
    ("uz-UZ", "Uzbek"),
    ("vi-VN", "Vietnamese"),
    ("xh-ZA", "isiXhosa"),
    ("zh-Hans", "Chinese (Simplified)"),
    ("zh-Hant", "Chinese (Traditional)"),
    ("zu-ZA", "isiZulu"),
];

/// English name of a language code (`en-US` is "English"); an unknown code is
/// returned as it is.
pub fn english_name(code: &str) -> String {
    LANGUAGES.iter().find(|(c, _)| c.eq_ignore_ascii_case(code)).map_or_else(|| code.to_string(), |(_, name)| name.to_string())
}

/// The code of an English language name, any case (GOG Galaxy's `language`).
pub fn code_of(name: &str) -> Option<&'static str> {
    LANGUAGES.iter().find(|(_, n)| n.eq_ignore_ascii_case(name.trim())).map(|(c, _)| *c)
}

/// Whether `value` (a code or an English name, any case) is this language.
pub fn matches(code: &str, value: &str) -> bool {
    value.eq_ignore_ascii_case(code) || value.eq_ignore_ascii_case(&english_name(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_matching() {
        assert_eq!(english_name("en-US"), "English");
        assert_eq!(english_name("pt-BR"), "Portuguese (Brazilian)");
        assert_eq!(english_name("xx"), "xx");
        assert!(matches("en-US", "english"));
        assert!(matches("en-US", "EN-us"));
        assert!(!matches("en-US", "German"));
    }
}
