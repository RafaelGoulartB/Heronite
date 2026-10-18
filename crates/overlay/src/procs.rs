//! The game's processes, from `/proc`.

/// Whether `pid` is the game, in its process group (the launcher starts each
/// game as its own group) or descends from it.
pub(crate) fn belongs_to(pid: u32, game_pid: u32) -> bool {
    let mut current = pid;
    for _ in 0..64 {
        if current == game_pid {
            return true;
        }
        let Some((ppid, pgrp)) = proc_stat(current) else { return false };
        if pgrp == game_pid {
            return true;
        }
        if ppid <= 1 {
            return false;
        }
        current = ppid;
    }
    false
}

/// `(ppid, pgrp)` of a process.
fn proc_stat(pid: u32) -> Option<(u32, u32)> {
    parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// Fields after the command name, which may hold spaces and parentheses.
fn parse_stat(text: &str) -> Option<(u32, u32)> {
    let rest = &text[text.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace();
    let _state = fields.next()?;
    let ppid = fields.next()?.parse().ok()?;
    let pgrp = fields.next()?.parse().ok()?;
    Some((ppid, pgrp))
}

/// The game's processes (group, descendants), every live pid
/// that [`belongs_to`] the game.
pub(crate) fn game_processes(game_pid: u32) -> Vec<u32> {
    let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
    dir.flatten().filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok()).filter(|p| belongs_to(*p, game_pid)).collect()
}

/// One variable of a process's environment.
pub(crate) fn env_of(pid: u32, key: &str) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    environ_value(&raw, key)
}

fn environ_value(raw: &[u8], key: &str) -> Option<String> {
    raw.split(|b| *b == 0).find_map(|entry| {
        let entry = std::str::from_utf8(entry).ok()?;
        entry.strip_prefix(key)?.strip_prefix('=').map(str::to_string)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_stat_and_environ() {
        assert_eq!(parse_stat("1234 (Game (x64).exe) S 1200 1100 1100 0 -1"), Some((1200, 1100)));
        assert_eq!(parse_stat("garbage"), None);
        let env = b"PATH=/usr/bin\0DISPLAY=:1\0DISPLAY_X=2\0";
        assert_eq!(environ_value(env, "DISPLAY").as_deref(), Some(":1"));
        assert_eq!(environ_value(env, "HOME"), None);
    }
}
