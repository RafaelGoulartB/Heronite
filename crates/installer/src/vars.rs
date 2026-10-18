//! Variables of a running script: `$GAMEDIR`, `${VERSION}`, the ids of the
//! script's files (`$setup`), the answers to its questions (`$INPUT_lang`),
//! the script's own `variables` and a few facts about this computer.
//!
//! Substitution follows the templates the scripts were written for (Python's
//! `string.Template.safe_substitute`): `$NAME` and `${NAME}` are replaced
//! when known and left alone otherwise (shell variables in a command stay
//! shell variables), `$$` is a literal `$`, and names are ASCII letters,
//! digits and `_`. Names with dashes (file ids such as `setup-file`) also
//! match, longest first. A value that is exactly a known name, without `$`,
//! is that variable (`executable: setup` is the file `setup`).

use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct Vars {
    values: HashMap<String, String>,
    /// Names containing `-`, longest first.
    dashed: Vec<String>,
}

impl Vars {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        if name.contains('-') && !self.dashed.contains(&name) {
            self.dashed.push(name.clone());
            self.dashed.sort_by_key(|n| std::cmp::Reverse(n.len()));
        }
        self.values.insert(name, value.into());
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    /// `template` with the known variables replaced.
    pub fn substitute(&self, template: &str) -> String {
        if let Some(v) = self.lookup(template) {
            return v.to_string();
        }
        let bytes = template.as_bytes();
        let mut out = String::with_capacity(template.len());
        let mut i = 0;
        let mut copied = 0;
        while i < bytes.len() {
            if bytes[i] != b'$' {
                i += 1;
                continue;
            }
            out.push_str(&template[copied..i]);
            let rest = &template[i + 1..];
            let (replacement, used) = self.expand_at(rest);
            match replacement {
                Some(text) => {
                    out.push_str(&text);
                    i += 1 + used;
                }
                None => {
                    out.push('$');
                    i += 1;
                }
            }
            copied = i;
        }
        out.push_str(&template[copied..]);
        out
    }

    /// What `$` followed by `rest` becomes and how many bytes of `rest` it
    /// used; `None` keeps the `$` as it is.
    fn expand_at(&self, rest: &str) -> (Option<String>, usize) {
        if rest.starts_with('$') {
            return (Some("$".into()), 1);
        }
        if let Some(inner) = rest.strip_prefix('{') {
            if let Some(end) = inner.find('}') {
                let name = &inner[..end];
                if let Some(v) = self.lookup(name) {
                    return (Some(v.to_string()), end + 2);
                }
            }
            return (None, 0);
        }
        for name in &self.dashed {
            if rest.starts_with(name.as_str()) && !continues_name(&rest[name.len()..]) {
                return (Some(self.values[name].clone()), name.len());
            }
        }
        let len = name_len(rest);
        if len == 0 {
            return (None, 0);
        }
        match self.lookup(&rest[..len]) {
            Some(v) => (Some(v.to_string()), len),
            None => (None, 0),
        }
    }

    fn lookup(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }
}

/// Length of the identifier at the start of `s` (`[_A-Za-z][_A-Za-z0-9]*`).
fn name_len(s: &str) -> usize {
    let b = s.as_bytes();
    if b.is_empty() || !(b[0] == b'_' || b[0].is_ascii_alphabetic()) {
        return 0;
    }
    b.iter().take_while(|c| **c == b'_' || c.is_ascii_alphanumeric()).count()
}

fn continues_name(s: &str) -> bool {
    s.as_bytes().first().is_some_and(|c| *c == b'_' || c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vars {
        let mut v = Vars::new();
        v.set("GAMEDIR", "/games/fl");
        v.set("VERSION", "26");
        v.set("setup", "/cache/SPFL26_setup.exe");
        v.set("setup-file", "/cache/other.exe");
        v
    }

    #[test]
    fn plain_and_braced_names() {
        let v = vars();
        assert_eq!(v.substitute("$GAMEDIR/drive_c"), "/games/fl/drive_c");
        assert_eq!(v.substitute("SP Football Life 20${VERSION}"), "SP Football Life 2026");
        assert_eq!(v.substitute("FL${VERSION} switcher.exe"), "FL26 switcher.exe");
        assert_eq!(v.substitute("$VERSIONS"), "$VERSIONS", "a longer name is another variable");
    }

    #[test]
    fn unknown_names_stay_for_the_shell() {
        let v = vars();
        assert_eq!(v.substitute("ratio=$(awk -v w=\"$W\")"), "ratio=$(awk -v w=\"$W\")");
        assert_eq!(v.substitute("${UNKNOWN} $"), "${UNKNOWN} $");
        assert_eq!(v.substitute("cost $$5"), "cost $5");
        assert_eq!(v.substitute("sed 's/\\1\\2/'"), "sed 's/\\1\\2/'");
    }

    #[test]
    fn file_ids_with_and_without_dollar() {
        let v = vars();
        assert_eq!(v.substitute("$setup"), "/cache/SPFL26_setup.exe");
        assert_eq!(v.substitute("setup"), "/cache/SPFL26_setup.exe");
        assert_eq!(v.substitute("$setup-file /S"), "/cache/other.exe /S");
        assert_eq!(v.substitute("$setup-filex"), "/cache/SPFL26_setup.exe-filex");
        assert_eq!(v.substitute("setup.exe"), "setup.exe");
    }

    #[test]
    fn multibyte_text_is_kept() {
        let v = vars();
        assert_eq!(v.substitute("Instalação em $GAMEDIR — ok ✓"), "Instalação em /games/fl — ok ✓");
    }
}
