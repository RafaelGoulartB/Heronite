//! `HERONITE_*` environment switches: the `HERONITE_DEBUG_*` hooks open a
//! given state for screenshots and tests, the others turn background work off
//! in fixtures. Every read goes through here so they all agree on what "set"
//! means.

use std::str::FromStr;

/// The value of `name` when it is set and not empty.
pub fn value(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// `name` is set to anything but `""` or `"0"`.
pub fn flag(name: &str) -> bool {
    value(name).is_some_and(|v| v != "0")
}

/// `name` parsed as `T` (unset, empty or unparsable: `None`).
pub fn parse<T: FromStr>(name: &str) -> Option<T> {
    value(name)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{flag, parse, value};

    #[test]
    fn empty_and_zero_are_off() {
        // Unique names: tests run in parallel and share the environment.
        std::env::set_var("HERONITE_TEST_ENV_EMPTY", "");
        std::env::set_var("HERONITE_TEST_ENV_ZERO", "0");
        std::env::set_var("HERONITE_TEST_ENV_ON", "1");
        std::env::set_var("HERONITE_TEST_ENV_NUM", "42");
        assert_eq!(value("HERONITE_TEST_ENV_EMPTY"), None);
        assert!(!flag("HERONITE_TEST_ENV_EMPTY"));
        assert!(!flag("HERONITE_TEST_ENV_ZERO"));
        assert!(flag("HERONITE_TEST_ENV_ON"));
        assert!(!flag("HERONITE_TEST_ENV_UNSET"));
        assert_eq!(parse::<u64>("HERONITE_TEST_ENV_NUM"), Some(42));
        assert_eq!(parse::<u64>("HERONITE_TEST_ENV_ON"), Some(1));
        assert_eq!(parse::<u64>("HERONITE_TEST_ENV_ZERO"), Some(0));
        assert_eq!(parse::<u64>("HERONITE_TEST_ENV_UNSET"), None);
    }
}
