//! Splitting the arguments a user types for a game or a wrapper.
//!
//! A program takes its arguments the way its system writes them: a Linux
//! program from a shell-like line ([`split_args`]), a Windows program from a
//! command line ([`split_windows_args`]), where `\` is a path separator and
//! not an escape (single quotes still group, as they always did). [`split_args_for`] picks one by the program.

/// Splits `text` like a POSIX shell, and never gives up: a line the shell
/// would refuse (a quote that is never closed) still splits, with that quote
/// kept as a character, so `-name Rafa's -x "a b"` gives `-name`, `Rafa's`,
/// `-x`, `a b`.
pub fn split_args(text: &str) -> Vec<String> {
    shlex::split(text).unwrap_or_else(|| split_leniently(text))
}

/// [`split_args`] for a Linux program, [`split_windows_args`] for a Windows one.
pub fn split_args_for(windows: bool, text: &str) -> Vec<String> {
    if windows {
        split_windows_args(text)
    } else {
        split_args(text)
    }
}

/// The index of the quote closing the one before `from`: `quote` itself, or
/// for `"` the first one a backslash does not escape.
fn closing_quote(chars: &[char], from: usize, quote: char) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        match chars[i] {
            '\\' if quote == '"' => i += 1,
            c if c == quote => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// The shell's words where it can be read, the text as it is where it cannot:
/// a quote with no closing one is a character, and so is an apostrophe inside
/// a word ("Rafa's"), which no one means as a quote.
fn split_leniently(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let (mut out, mut word, mut in_word) = (Vec::new(), String::new(), false);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let inside_word = in_word && chars.get(i + 1).is_some_and(|n| n.is_alphanumeric()) && chars[i - 1].is_alphanumeric();
        match c {
            '\\' if i + 1 < chars.len() => {
                word.push(chars[i + 1]);
                i += 1;
            }
            '\'' | '"' if !(c == '\'' && inside_word) => match closing_quote(&chars, i + 1, c) {
                Some(end) => {
                    let body = &chars[i + 1..end];
                    if c == '"' {
                        unescape_double(body, &mut word);
                    } else {
                        word.extend(body);
                    }
                    i = end;
                }
                None => word.push(c),
            },
            c if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut word));
                    in_word = false;
                }
                i += 1;
                continue;
            }
            c => word.push(c),
        }
        in_word = true;
        i += 1;
    }
    if in_word {
        out.push(word);
    }
    out
}

/// The inside of a double-quoted word: `\"`, `\\`, `\$` and `` \` `` are the
/// character, any other backslash stays.
fn unescape_double(body: &[char], word: &mut String) {
    let mut chars = body.iter().copied().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some(&n)) if matches!(n, '"' | '\\' | '$' | '`') => {
                word.push(n);
                chars.next();
            }
            _ => word.push(c),
        }
    }
}

/// Splits the arguments of a Windows program the way its command line is read
/// (`CommandLineToArgvW`): spaces separate arguments, double quotes group
/// them, `\"` is a quote and other backslashes stay as they are (paths keep
/// their separators, and `\ ` does not escape the space). Single quotes
/// group too, as they did before the backslash was fixed, so a saved `-x 'a b'`
/// keeps working: they are dropped around the group, and nothing inside it
/// is special. An apostrophe in a word ("Rafa's") or with no closing one is
/// a character.
pub fn split_windows_args(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut word = String::new();
    let (mut in_word, mut quoted, mut single, mut backslashes) = (false, false, false, 0usize);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        if single {
            single = c != '\'';
            if single {
                word.push(c);
            }
            continue;
        }
        match c {
            '\\' => {
                backslashes += 1;
                in_word = true;
                continue;
            }
            '"' => {
                word.extend(std::iter::repeat_n('\\', backslashes / 2));
                if backslashes % 2 == 1 {
                    word.push('"');
                } else if quoted && chars.get(i) == Some(&'"') {
                    // `""` inside quotes is a quote.
                    i += 1;
                    word.push('"');
                } else {
                    quoted = !quoted;
                }
                in_word = true;
            }
            '\'' if !quoted && opens_single_quote(&chars, i - 1, in_word) => {
                word.extend(std::iter::repeat_n('\\', backslashes));
                single = true;
                in_word = true;
            }
            c if c.is_whitespace() && !quoted => {
                word.extend(std::iter::repeat_n('\\', backslashes));
                if in_word {
                    out.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                word.extend(std::iter::repeat_n('\\', backslashes));
                word.push(c);
                in_word = true;
            }
        }
        backslashes = 0;
    }
    word.extend(std::iter::repeat_n('\\', backslashes));
    if in_word {
        out.push(word);
    }
    out
}

/// Whether the apostrophe at `at` opens a quoted group: it has a closing one
/// after it and is not the apostrophe inside a word ("Rafa's").
fn opens_single_quote(chars: &[char], at: usize, in_word: bool) -> bool {
    let inside_word = in_word && at > 0 && chars[at - 1].is_alphanumeric() && chars.get(at + 1).is_some_and(|c| c.is_alphanumeric());
    !inside_word && chars[at + 1..].contains(&'\'')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_command_lines_split_like_windows_programs_do() {
        assert_eq!(split_windows_args(r#"-a "C:\Program Files\G" b"#), ["-a", r"C:\Program Files\G", "b"]);
        assert_eq!(split_windows_args(r#"/path=C:\x\ "q\"uote" "" end\"#), [r"/path=C:\x\", r#"q"uote"#, "", r"end\"]);
        assert_eq!(split_windows_args(r#""a\\" "x""y""#), [r"a\", r#"x"y"#]);
        assert!(split_windows_args("   ").is_empty());
    }

    #[test]
    fn single_quotes_still_group_in_a_windows_line() {
        assert_eq!(split_windows_args("-x 'a b' -y"), ["-x", "a b", "-y"]);
        assert_eq!(split_windows_args(r#"-p 'C:\My Games\g' "d e""#), ["-p", r"C:\My Games\g", "d e"]);
        // Nothing is special inside them, and not inside double quotes either.
        assert_eq!(split_windows_args(r#"'a "b" c' "it's""#), [r#"a "b" c"#, "it's"]);
        // A lone apostrophe, or one inside a word, is a character.
        assert_eq!(split_windows_args("-name Rafa's -x 'a b'"), ["-name", "Rafa's", "-x", "a b"]);
        assert_eq!(split_windows_args("-name Rafa's -x \"a b\""), ["-name", "Rafa's", "-x", "a b"]);
        assert_eq!(split_windows_args("it's"), ["it's"]);
        assert_eq!(split_windows_args("-a 'b c"), ["-a", "'b", "c"]);
        // The one change from the shell's rules: a backslash does not escape the space.
        assert_eq!(split_windows_args(r"--path /home/u/My\ Games"), ["--path", r"/home/u/My\", "Games"]);
    }

    #[test]
    fn a_shell_line_splits_like_a_shell() {
        assert_eq!(split_args(r#"-a "b c" 'd e' f\ g"#), ["-a", "b c", "d e", "f g"]);
        assert!(split_args("  ").is_empty());
    }

    #[test]
    fn an_unclosed_quote_is_a_character_not_the_end_of_the_splitting() {
        assert_eq!(split_args(r#"-name Rafa's -x "a b""#), ["-name", "Rafa's", "-x", "a b"]);
        // Balanced quotes elsewhere in the line still group.
        assert_eq!(split_args(r#"-name Rafa's -title 'My Game'"#), ["-name", "Rafa's", "-title", "My Game"]);
        assert_eq!(split_args(r#"--say "it's" he"llo"#), ["--say", "it's", "he\"llo"]);
        assert_eq!(split_args("-a 'b c"), ["-a", "'b", "c"]);
        assert_eq!(split_args(r#"-a "b \"c\" d" it's"#), ["-a", r#"b "c" d"#, "it's"]);
    }

    #[test]
    fn the_program_picks_the_syntax() {
        let line = r#"-userdir C:\Users\me\saves -name "Rafa Goulart""#;
        assert_eq!(split_args_for(true, line), ["-userdir", r"C:\Users\me\saves", "-name", "Rafa Goulart"]);
        // A shell reads the backslashes as escapes.
        assert_eq!(split_args_for(false, line)[1], "C:Usersmesaves");
        assert_eq!(split_args_for(false, "-x 'a b'"), ["-x", "a b"]);
    }
}
