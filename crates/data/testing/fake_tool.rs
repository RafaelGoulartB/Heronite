//! A stand-in for a command-line tool in tests (legendary, gogdl, nile...).
//! `heronite_data::testing` builds it once and copies it where a test wants
//! the tool; it then replays what the script next to it says
//! (`<its name>.fake`, written by `testing::FakeTool`). No script: it exits 0
//! (a downloaded tool's `--version` check).
//!
//! Script lines (values escaped: `\\`, `\n`, `\r`, `\t`; `${NAME}` is the
//! environment variable `NAME`):
//!
//! - `log <file>`: append each call there (its arguments joined by spaces),
//! - `env <NAME>`: and an `env NAME=<value>` line for this variable,
//! - `rule <arg>\t<arg>...`: the actions until the next rule run when every
//!   `<arg>` is one of the arguments (`x*`: one starts with `x`); the first
//!   rule that matches wins, a rule with no `<arg>` always matches,
//! - `out <text>` / `err <text>`: write to stdout / stderr,
//! - `write <file>\t<text>`: write a file (its folders created),
//! - `sleep <ms>`, `exit <code>`.

use std::io::Write;
use std::time::Duration;

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    expand(&out)
}

/// `${NAME}` becomes the variable's value (nothing when unset).
fn expand(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        let Some(len) = rest[start + 2..].find('}') else { break };
        out.push_str(&rest[..start]);
        out.push_str(&std::env::var(&rest[start + 2..start + 2 + len]).unwrap_or_default());
        rest = &rest[start + 3 + len..];
    }
    out.push_str(rest);
    out
}

fn matches(needles: &[String], args: &[String]) -> bool {
    needles.iter().all(|n| match n.strip_suffix('*') {
        Some(prefix) => args.iter().any(|a| a.starts_with(prefix)),
        None => args.iter().any(|a| a == n),
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Ok(exe) = std::env::current_exe() else { return };
    let Ok(script) = std::fs::read_to_string(exe.with_extension("fake")) else { return };
    let (mut log, mut env) = (None, Vec::new());
    // Rules: (needles, actions).
    let mut rules: Vec<(Vec<String>, Vec<(String, String)>)> = Vec::new();
    for line in script.lines() {
        let (verb, value) = line.split_once(' ').unwrap_or((line, ""));
        match verb {
            "log" => log = Some(unescape(value)),
            "env" => env.push(value.to_string()),
            "rule" => rules.push((value.split('\t').filter(|n| !n.is_empty()).map(unescape).collect(), Vec::new())),
            _ => {
                if let Some((_, actions)) = rules.last_mut() {
                    actions.push((verb.to_string(), value.to_string()));
                }
            }
        }
    }
    if let Some(log) = log {
        let mut entry = format!("{}\n", args.join(" "));
        for name in &env {
            entry.push_str(&format!("env {name}={}\n", std::env::var(name).unwrap_or_default()));
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
            let _ = f.write_all(entry.as_bytes());
        }
    }
    let Some((_, actions)) = rules.iter().find(|(needles, _)| matches(needles, &args)) else { return };
    for (verb, value) in actions {
        match verb.as_str() {
            "out" => {
                let mut out = std::io::stdout();
                let _ = out.write_all(unescape(value).as_bytes());
                let _ = out.flush();
            }
            "err" => {
                let mut err = std::io::stderr();
                let _ = err.write_all(unescape(value).as_bytes());
                let _ = err.flush();
            }
            "write" => {
                let (path, text) = value.split_once('\t').unwrap_or((value, ""));
                let path = std::path::PathBuf::from(unescape(path));
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(e) = std::fs::write(&path, unescape(text)) {
                    eprintln!("fake tool: writing {}: {e}", path.display());
                    std::process::exit(101);
                }
            }
            "sleep" => std::thread::sleep(Duration::from_millis(value.parse().unwrap_or(0))),
            "exit" => std::process::exit(value.parse().unwrap_or(1)),
            _ => {
                eprintln!("fake tool: unknown action {verb:?}");
                std::process::exit(101);
            }
        }
    }
}
