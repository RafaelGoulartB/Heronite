//! `Date#toISOString` timestamps (UTC, milliseconds), as Heronite writes them.

use chrono::{DateTime, SecondsFormat, Utc};

pub(crate) fn iso_millis(t: impl Into<DateTime<Utc>>) -> String {
    t.into().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn iso_now() -> String {
    iso_millis(Utc::now())
}
