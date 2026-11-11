//! A pattern library file: one named pattern per line, so patterns can be
//! written in any text editor, kept in git, shared, and loaded on the MPC.
//!
//! ```text
//! # My patterns. Blank lines and lines starting with # are ignored.
//! Skip     = 1 . 3 2 . 3 x:2 .
//! Lift     = 1 2 3 4 | oct 0 0 1 | vel 110 70
//! Shuffle  = x x' x x' | nudge 0 12 0 8 | vel~15
//! ```
//!
//! Names are 1 to 32 characters, unique ignoring case, and may not contain
//! `=` or start with `#`. Reading is not for the audio thread: it allocates.

use crate::pattern::{ParseError, Pattern, BUILT_IN};

/// Longest pattern name.
pub const MAX_NAME: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryError {
    /// 1-based line in the file.
    pub line: usize,
    pub reason: String,
}

impl core::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "line {}: {}", self.line, self.reason)
    }
}

impl std::error::Error for LibraryError {}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Library {
    entries: Vec<(String, Pattern)>,
}

impl Library {
    /// The ten starter patterns.
    pub fn built_in() -> Self {
        let mut l = Self::default();
        for (name, text) in BUILT_IN {
            l.entries.push((name.to_string(), Pattern::parse(text).expect("built-in patterns parse")));
        }
        l
    }

    /// Reads a library file. Every bad line is reported, not just the first,
    /// so one pass fixes them all.
    pub fn parse(text: &str) -> Result<Self, Vec<LibraryError>> {
        let mut lib = Self::default();
        let mut errors = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let Some((name, body)) = trimmed.split_once('=') else {
                errors.push(LibraryError { line, reason: "expected: Name = pattern".into() });
                continue;
            };
            let name = name.trim();
            if let Err(reason) = check_name(name) {
                errors.push(LibraryError { line, reason });
                continue;
            }
            if lib.get(name).is_some() {
                errors.push(LibraryError { line, reason: format!("\"{name}\" is already in the file") });
                continue;
            }
            match Pattern::parse(body) {
                Ok(p) => lib.entries.push((name.to_string(), p)),
                Err(ParseError { token, reason }) => {
                    let word = nth_token(body, token).unwrap_or("");
                    errors.push(LibraryError { line, reason: format!("\"{name}\", token {token} \"{word}\": {reason}") });
                }
            }
        }
        if errors.is_empty() {
            Ok(lib)
        } else {
            Err(errors)
        }
    }

    /// Writes the file form; `parse` reads it back to the same library.
    pub fn to_text(&self) -> String {
        let width = self.entries.iter().map(|(n, _)| n.chars().count()).max().unwrap_or(0);
        let mut s = String::new();
        for (name, p) in &self.entries {
            let pad = width - name.chars().count();
            s.push_str(name);
            s.extend(std::iter::repeat_n(' ', pad));
            s.push_str(" = ");
            s.push_str(&p.to_text());
            s.push('\n');
        }
        s
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Pattern)> {
        self.entries.iter().map(|(n, p)| (n.as_str(), p))
    }

    /// Finds a pattern by name, ignoring case.
    pub fn get(&self, name: &str) -> Option<&Pattern> {
        self.entries.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, p)| p)
    }

    /// Adds a pattern, or replaces the one with the same name (ignoring
    /// case, keeping its place in the list).
    pub fn put(&mut self, name: &str, pattern: Pattern) -> Result<(), String> {
        let name = name.trim();
        check_name(name)?;
        match self.entries.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            Some(entry) => *entry = (name.to_string(), pattern),
            None => self.entries.push((name.to_string(), pattern)),
        }
        Ok(())
    }

    /// Removes a pattern by name, ignoring case. Returns false if absent.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.entries.len() != before
    }
}

fn check_name(name: &str) -> Result<(), String> {
    let n = name.chars().count();
    if n == 0 {
        return Err("the pattern needs a name before =".into());
    }
    if n > MAX_NAME {
        return Err(format!("names are at most {MAX_NAME} characters"));
    }
    if name.contains('=') || name.starts_with('#') || name.chars().any(char::is_control) {
        return Err("names may not contain = or control characters, or start with #".into());
    }
    Ok(())
}

/// The `n`th (1-based) whitespace-separated token, counting `|` lane
/// names and values the same way `Pattern::parse` does.
fn nth_token(text: &str, n: usize) -> Option<&str> {
    text.split(|c: char| c.is_whitespace() || c == '|').filter(|w| !w.is_empty()).nth(n.checked_sub(1)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_a_file() {
        let text = "# mine\n\nSkip = 1 . 3 2 . 3 x:2 .\n  Lift=1 2 3 4 | oct 0 0 1 | vel 110 70  \nShuffle = x x' x x' | nudge 0 12 0 8 | vel~15\n";
        let lib = Library::parse(text).unwrap();
        assert_eq!(lib.len(), 3);
        assert_eq!(lib.get("lift").unwrap().to_text(), "1 2 3 4 | oct 0 0 1 | vel 110 70");
        let out = lib.to_text();
        assert_eq!(out.lines().next(), Some("Skip    = 1 . 3 2 . 3 x:2 ."));
        assert_eq!(Library::parse(&out).unwrap(), lib);
        assert_eq!(Library::built_in().len(), BUILT_IN.len());
        assert_eq!(Library::parse(&Library::built_in().to_text()).unwrap(), Library::built_in());
    }

    #[test]
    fn reports_every_bad_line() {
        let text = "A = x\nno equals sign\n = x\nA = x x\nB = x y x\nC = x | vel 200\nThis name is far too long to be a name = x\n";
        let errors = Library::parse(text).unwrap_err();
        let lines: Vec<(usize, &str)> = errors.iter().map(|e| (e.line, e.reason.as_str())).collect();
        assert_eq!(
            lines,
            [
                (2, "expected: Name = pattern"),
                (3, "the pattern needs a name before ="),
                (4, "\"A\" is already in the file"),
                (5, "\"B\", token 2 \"y\": unknown step (use . _ x 1-9 * ? l h)"),
                (6, "\"C\", token 3 \"200\": velocity must be 0 to 127"),
                (7, "names are at most 32 characters"),
            ]
        );
        assert_eq!(errors[0].to_string(), "line 2: expected: Name = pattern");
    }

    #[test]
    fn put_replaces_by_name_and_remove_deletes() {
        let mut lib = Library::built_in();
        let p = Pattern::parse("x . x").unwrap();
        lib.put("BOUNCE", p).unwrap();
        assert_eq!(lib.len(), BUILT_IN.len());
        assert_eq!(lib.iter().nth(2).unwrap(), ("BOUNCE", &p));
        lib.put("New one", p).unwrap();
        assert_eq!(lib.len(), BUILT_IN.len() + 1);
        assert!(lib.put("#hash", p).is_err());
        assert!(lib.remove("new ONE"));
        assert!(!lib.remove("new one"));
    }
}
