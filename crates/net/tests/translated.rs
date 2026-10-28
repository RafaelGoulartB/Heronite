//! The error texts follow the interface language. Its own test binary: the
//! language is a process-wide switch.

use std::time::Duration;

use heronite_i18n::Lang;
use heronite_net::Error;

fn errors() -> Vec<Error> {
    vec![
        Error::Cancelled,
        Error::Stalled(Duration::from_secs(30)),
        Error::Unreachable { url: "https://host/p".into(), detail: "connection refused".into() },
        Error::RateLimited { retry_after: Some(Duration::from_secs(7)) },
        Error::RateLimited { retry_after: None },
        Error::Truncated { got: 10, total: 100 },
    ]
}

#[test]
fn every_error_has_a_translation_in_every_language() {
    let english: Vec<String> = errors().iter().map(|e| e.to_string()).collect();
    assert_eq!(english[1], "the download stalled: no data for 30 s");
    assert_eq!(english[3], "rate limit reached, try again in 7 s");
    for lang in Lang::TRANSLATED {
        heronite_i18n::set(lang);
        let translated: Vec<String> = errors().iter().map(|e| e.to_string()).collect();
        for (en, tr) in english.iter().zip(&translated) {
            assert_ne!(en, tr, "{lang:?}: {en:?} is not translated");
        }
        assert!(translated[1].contains("30"), "{lang:?}: {}", translated[1]);
        assert!(translated[2].contains("https://host/p") && translated[2].contains("connection refused"), "{lang:?}: {}", translated[2]);
        assert!(translated[3].contains('7'), "{lang:?}: {}", translated[3]);
        assert!(translated[5].contains("10") && translated[5].contains("100"), "{lang:?}: {}", translated[5]);
    }
    heronite_i18n::set(Lang::En);
}
