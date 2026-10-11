//! Heroic's log line format and the kinds of game logs.

/// `LogLevel` of Heroic, widest name `WARNING` (+ 3 columns).
const LEVEL_WIDTH: usize = 7 + 3;
/// `MaxLogPrefixLength` of Heroic (`DownloadManager`) + 3 columns.
const PREFIX_WIDTH: usize = 15 + 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warning,
    Error,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warning => "WARNING",
            Level::Error => "ERROR",
        }
    }
}

/// `(hh:mm:ss) [LEVEL]:   [Prefix]:       message` without the newline.
/// An empty message gives an empty line (Heroic skips those entirely).
pub fn format_line(time: chrono::NaiveTime, level: Level, prefix: &str, message: &str) -> String {
    use chrono::Timelike;
    if message.is_empty() {
        return String::new();
    }
    let mut parts = vec![
        format!("({:02}:{:02}:{:02})", time.hour(), time.minute(), time.second()),
        format!("{:<LEVEL_WIDTH$}", format!("[{}]:", level.name())),
    ];
    if !prefix.is_empty() {
        parts.push(format!("{:<PREFIX_WIDTH$}", format!("[{prefix}]:")));
    }
    format!("{} {message}", parts.join(" "))
}

/// Which log of a game (Heroic's `GameLogType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GameLogType {
    Launch,
    Install,
    Import,
    Repair,
    Update,
    Setup,
}

impl GameLogType {
    pub const ALL: [GameLogType; 6] =
        [GameLogType::Launch, GameLogType::Install, GameLogType::Import, GameLogType::Repair, GameLogType::Update, GameLogType::Setup];

    pub fn file_stem(self) -> &'static str {
        match self {
            GameLogType::Launch => "launch",
            GameLogType::Install => "install",
            GameLogType::Import => "import",
            GameLogType::Repair => "repair",
            GameLogType::Update => "update",
            GameLogType::Setup => "setup",
        }
    }

    pub fn from_stem(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.file_stem() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            GameLogType::Launch => "Launch",
            GameLogType::Install => "Install",
            GameLogType::Import => "Import",
            GameLogType::Repair => "Repair",
            GameLogType::Update => "Update",
            GameLogType::Setup => "Setup",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    #[test]
    fn lines_are_formatted_like_heroic() {
        let t = NaiveTime::from_hms_opt(21, 4, 5).unwrap();
        assert_eq!(format_line(t, Level::Info, "", "Native? false"), "(21:04:05) [INFO]:    Native? false");
        assert_eq!(format_line(t, Level::Warning, "", "x"), "(21:04:05) [WARNING]: x");
        assert_eq!(format_line(t, Level::Error, "Backend", "boom"), "(21:04:05) [ERROR]:   [Backend]:         boom");
        assert_eq!(format_line(t, Level::Info, "", ""), "");
    }
}
