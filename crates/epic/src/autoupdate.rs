//! Automatic game updates (`autoUpdateGames`): which of the updates found may
//! be queued right now. Port of Heroic's `autoUpdate` (skip games that ignore
//! updates), plus the conditions Heroic does not look at: a game being played
//! (or being verified or moved) is updated after it exits, nothing is
//! downloaded offline or on a metered connection.
//!
//! Everything here is pure except the two probes at the bottom.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// One game with an update available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub app_name: String,
    /// `ignoreGameUpdates` in the game's settings.
    pub ignored: bool,
    /// Already in the download queue (or downloading).
    pub queued: bool,
    /// Being played right now.
    pub playing: bool,
    /// A verify, move or other operation is running on it.
    pub busy: bool,
}

/// The conditions of the moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Gates {
    /// `autoUpdateGames`.
    pub enabled: bool,
    pub offline: bool,
    pub metered: bool,
}

/// Why nothing is queued at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Block {
    Disabled,
    Offline,
    Metered,
}

impl Block {
    pub fn describe(self) -> &'static str {
        match self {
            Block::Disabled => "automatic updates are off",
            Block::Offline => "offline",
            Block::Metered => "the connection is metered",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// Queue these updates now.
    pub queue: Vec<String>,
    /// Being played or busy: queue them when the game exits / the operation ends.
    pub deferred: Vec<String>,
    /// Games that ignore updates.
    pub ignored: Vec<String>,
    pub blocked: Option<Block>,
}

/// Decides what to do with the updates found.
pub fn plan(gates: &Gates, candidates: &[Candidate]) -> Plan {
    let blocked = if !gates.enabled {
        Some(Block::Disabled)
    } else if gates.offline {
        Some(Block::Offline)
    } else if gates.metered {
        Some(Block::Metered)
    } else {
        None
    };
    let mut plan = Plan { blocked, ..Default::default() };
    if plan.blocked.is_some() {
        return plan;
    }
    for c in candidates {
        if c.ignored {
            plan.ignored.push(c.app_name.clone());
        } else if c.queued {
            // Nothing to do: it is on its way.
        } else if c.playing || c.busy {
            plan.deferred.push(c.app_name.clone());
        } else {
            plan.queue.push(c.app_name.clone());
        }
    }
    plan
}

/// `NMMetered` values 1 (yes) and 3 (guess yes) as `busctl` prints them
/// (`u 1`). Anything else, or garbage, is "not metered".
pub fn parse_metered(busctl_output: &str) -> bool {
    matches!(busctl_output.split_whitespace().nth(1), Some("1" | "3"))
}

/// Whether NetworkManager calls the current connection metered. Without
/// NetworkManager (or `busctl`) it is not. Blocking, a few ms.
pub fn connection_metered() -> bool {
    let Ok(mut child) = Command::new("busctl")
        .args([
            "--system",
            "get-property",
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
            "Metered",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        use std::io::Read;
        let _ = stdout.read_to_string(&mut out);
    }
    parse_metered(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(app: &str) -> Candidate {
        Candidate { app_name: app.into(), ignored: false, queued: false, playing: false, busy: false }
    }

    fn open() -> Gates {
        Gates { enabled: true, ..Default::default() }
    }

    #[test]
    fn queues_plain_updates() {
        let p = plan(&open(), &[c("a"), c("b")]);
        assert_eq!(p.queue, ["a", "b"]);
        assert_eq!(p.blocked, None);
    }

    #[test]
    fn ignored_queued_and_playing_games_are_not_queued() {
        let list = [
            Candidate { ignored: true, ..c("ignored") },
            Candidate { queued: true, ..c("queued") },
            Candidate { playing: true, ..c("playing") },
            c("free"),
        ];
        let p = plan(&open(), &list);
        assert_eq!(p.queue, ["free"]);
        assert_eq!(p.deferred, ["playing"]);
        assert_eq!(p.ignored, ["ignored"]);
    }

    #[test]
    fn a_game_with_an_operation_running_waits_like_a_game_being_played() {
        let p =
            plan(&open(), &[Candidate { busy: true, ..c("verifying") }, Candidate { busy: true, queued: true, ..c("queued") }, c("free")]);
        assert_eq!(p.queue, ["free"]);
        assert_eq!(p.deferred, ["verifying"]);
    }

    #[test]
    fn an_ignored_game_that_is_playing_stays_ignored() {
        let p = plan(&open(), &[Candidate { ignored: true, playing: true, ..c("g") }]);
        assert!(p.queue.is_empty() && p.deferred.is_empty());
        assert_eq!(p.ignored, ["g"]);
    }

    #[test]
    fn gates_block_everything_in_priority_order() {
        let list = [c("a")];
        for (gates, why) in [
            (Gates::default(), Block::Disabled),
            (Gates { enabled: true, offline: true, metered: true }, Block::Offline),
            (Gates { enabled: true, metered: true, ..Default::default() }, Block::Metered),
        ] {
            let p = plan(&gates, &list);
            assert_eq!(p.blocked, Some(why));
            assert!(p.queue.is_empty() && p.deferred.is_empty());
        }
    }

    #[test]
    fn metered_values() {
        assert!(parse_metered("u 1\n"));
        assert!(parse_metered("u 3\n"));
        assert!(!parse_metered("u 0\n"));
        assert!(!parse_metered("u 2\n"));
        assert!(!parse_metered("u 4\n"));
        assert!(!parse_metered(""));
        assert!(!parse_metered("Failed to get property"));
    }
}
