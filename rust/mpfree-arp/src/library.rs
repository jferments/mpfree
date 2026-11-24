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
//!
//! The owner's own scales go in the same file, after a `[scales]` line
//! (and `[patterns]` goes back to patterns). Each is semitones from `0`, or
//! degrees from `1`, with `/` and the notes coming down when they differ
//! (see `mpfree_scale::parse_steps`):
//!
//! ```text
//! [scales]
//! Bright minor   = 1 2 b3 4 5 6 7
//! Classical min  = 0 2 3 5 7 9 11 / 0 2 3 5 7 8 10
//! ```
//!
//! A custom scale may not take a built-in scale's name.
//!
//! Phrases ([`crate::phrase`]) go in the same kind of file, one to a line
//! among the patterns. A phrase line says what it is before a `:` on its
//! own (its kind and its scale first), so no heading is needed to tell the
//! two apart: a line with that `:`, or that starts with a kind or a scale,
//! is a phrase, and any other is a pattern.
//!
//! ```text
//! Night drive = bass minor : 1 1 1+ 1 1 1 1+ 1 1 1 1+ 1 1 1 1+ 1
//! Sus lift    = chords either 1/8 : 1,4,5 _ _ 1,4,5 _ _ 1,3,5 _
//! ```
//!
//! Patterns and phrases are named apart: a phrase may take a pattern's
//! name. `[phrases]` is read as `[patterns]` is (a heading for the reader).
//! MPFree's own phrases are such a file, [`STARTER_PHRASES`].

use crate::pattern::{ParseError, Pattern, BUILT_IN};
use crate::phrase::{Kind, Phrase, Written};

/// MPFree's starter phrases as a library file: 64 written for MPFree, 16
/// of each kind ([`Library::built_in`] reads them).
pub const STARTER_PHRASES: &str = include_str!("../phrases/starter.txt");

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
    phrases: Vec<(String, Phrase)>,
    /// The owner's scales: name, the notes going up and coming down.
    scales: Vec<(String, u16, u16)>,
}

impl Library {
    /// What MPFree ships: the ten starter patterns, then the rest of the
    /// library's groups ([`crate::groups::GROUPS`]) in order, and the 64
    /// starter phrases.
    pub fn built_in() -> Self {
        let mut l = Self::parse(STARTER_PHRASES).expect("the starter phrases parse");
        for (name, text) in BUILT_IN {
            l.entries.push((
                name.to_string(),
                Pattern::parse(text).expect("built-in patterns parse"),
            ));
        }
        for g in &crate::groups::GROUPS[1..] {
            for (name, text) in g.patterns {
                l.entries.push((
                    name.to_string(),
                    Pattern::parse(text).expect("built-in patterns parse"),
                ));
            }
        }
        l
    }

    /// Reads a library file. Every bad line is reported, not just the first,
    /// so one pass fixes them all.
    pub fn parse(text: &str) -> Result<Self, Vec<LibraryError>> {
        let mut lib = Self::default();
        let mut errors = Vec::new();
        let mut in_scales = false;
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if trimmed.eq_ignore_ascii_case("[scales]")
                || trimmed.eq_ignore_ascii_case("[patterns]")
                || trimmed.eq_ignore_ascii_case("[phrases]")
            {
                in_scales = trimmed.eq_ignore_ascii_case("[scales]");
                continue;
            }
            let what = if in_scales { "scale" } else { "pattern" };
            let Some((name, body)) = trimmed.split_once('=') else {
                errors.push(LibraryError {
                    line,
                    reason: format!("expected: Name = {what}"),
                });
                continue;
            };
            let name = name.trim();
            let what = if !in_scales && is_phrase(body) {
                "phrase"
            } else {
                what
            };
            if let Err(reason) = check_name(name, what) {
                errors.push(LibraryError { line, reason });
                continue;
            }
            if in_scales {
                if let Err(reason) = lib.put_scale(name, body) {
                    errors.push(LibraryError { line, reason });
                }
                continue;
            }
            if is_phrase(body) {
                if lib.phrase(name).is_some() {
                    errors.push(LibraryError {
                        line,
                        reason: format!("\"{name}\" is already in the file"),
                    });
                    continue;
                }
                match Phrase::parse(body) {
                    Ok(p) => lib.phrases.push((name.to_string(), p)),
                    Err(e) if e.token == 0 => errors.push(LibraryError {
                        line,
                        reason: format!("\"{name}\": {}", e.reason),
                    }),
                    Err(e) => {
                        let word = nth_phrase_token(body, e.token).unwrap_or("");
                        errors.push(LibraryError {
                            line,
                            reason: format!(
                                "\"{name}\", token {} \"{word}\": {}",
                                e.token, e.reason
                            ),
                        });
                    }
                }
                continue;
            }
            if lib.get(name).is_some() {
                errors.push(LibraryError {
                    line,
                    reason: format!("\"{name}\" is already in the file"),
                });
                continue;
            }
            match Pattern::parse(body) {
                Ok(p) => lib.entries.push((name.to_string(), p)),
                Err(ParseError { token, reason }) => {
                    let word = nth_token(body, token).unwrap_or("");
                    errors.push(LibraryError {
                        line,
                        reason: format!("\"{name}\", token {token} \"{word}\": {reason}"),
                    });
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
        let width = self
            .entries
            .iter()
            .map(|(n, _)| n.chars().count())
            .max()
            .unwrap_or(0);
        let mut s = String::new();
        for (name, p) in &self.entries {
            let pad = width - name.chars().count();
            s.push_str(name);
            s.extend(std::iter::repeat_n(' ', pad));
            s.push_str(" = ");
            s.push_str(&p.to_text());
            s.push('\n');
        }
        if !self.phrases.is_empty() {
            if !s.is_empty() {
                s.push('\n');
            }
            let width = self
                .phrases
                .iter()
                .map(|(n, _)| n.chars().count())
                .max()
                .unwrap_or(0);
            for (name, p) in &self.phrases {
                s.push_str(name);
                s.extend(std::iter::repeat_n(' ', width - name.chars().count()));
                s.push_str(" = ");
                s.push_str(&p.to_text());
                s.push('\n');
            }
        }
        if !self.scales.is_empty() {
            let width = self
                .scales
                .iter()
                .map(|(n, ..)| n.chars().count())
                .max()
                .unwrap_or(0);
            s.push_str("\n[scales]\n");
            for (name, up, down) in &self.scales {
                s.push_str(name);
                s.extend(std::iter::repeat_n(' ', width - name.chars().count()));
                s.push_str(" = ");
                s.push_str(&mpfree_scale::scale_text(*up, *down));
                s.push('\n');
            }
        }
        s
    }

    /// The owner's scales: name, the notes going up and coming down.
    pub fn scales(&self) -> impl Iterator<Item = (&str, u16, u16)> {
        self.scales.iter().map(|(n, u, d)| (n.as_str(), *u, *d))
    }

    /// Adds a scale from its text (`0 2 3 5 7 9 11`, or degrees, with `/`
    /// and the notes coming down), or replaces the one with that name.
    pub fn put_scale(&mut self, name: &str, steps: &str) -> Result<(), String> {
        let name = name.trim();
        check_name(name, "scale")?;
        if mpfree_scale::by_name(name).is_some_and(|d| d.name.eq_ignore_ascii_case(name)) {
            return Err(format!(
                "\"{name}\" is a built-in scale; give yours another name"
            ));
        }
        let (up, down) =
            mpfree_scale::parse_steps(steps).map_err(|e| format!("scale \"{name}\": {e}"))?;
        match self
            .scales
            .iter_mut()
            .find(|(n, ..)| n.eq_ignore_ascii_case(name))
        {
            Some(entry) => *entry = (name.to_string(), up, down),
            None => self.scales.push((name.to_string(), up, down)),
        }
        Ok(())
    }

    /// How many patterns it has.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// It has no patterns (it may have phrases: [`Library::phrases`]).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Its phrases, in the file's order.
    pub fn phrases(&self) -> impl Iterator<Item = (&str, &Phrase)> {
        self.phrases.iter().map(|(n, p)| (n.as_str(), p))
    }

    /// Finds a phrase by name, ignoring case.
    pub fn phrase(&self, name: &str) -> Option<&Phrase> {
        self.phrases
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, p)| p)
    }

    /// Adds a phrase, or replaces the one with the same name (ignoring
    /// case, keeping its place in the list).
    pub fn put_phrase(&mut self, name: &str, phrase: Phrase) -> Result<(), String> {
        let name = name.trim();
        check_name(name, "phrase")?;
        match self
            .phrases
            .iter_mut()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
        {
            Some(entry) => *entry = (name.to_string(), phrase),
            None => self.phrases.push((name.to_string(), phrase)),
        }
        Ok(())
    }

    /// Removes a phrase by name, ignoring case. Returns false if absent.
    pub fn remove_phrase(&mut self, name: &str) -> bool {
        let before = self.phrases.len();
        self.phrases.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        self.phrases.len() != before
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Pattern)> {
        self.entries.iter().map(|(n, p)| (n.as_str(), p))
    }

    /// Finds a pattern by name, ignoring case.
    pub fn get(&self, name: &str) -> Option<&Pattern> {
        self.entries
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, p)| p)
    }

    /// Adds a pattern, or replaces the one with the same name (ignoring
    /// case, keeping its place in the list).
    pub fn put(&mut self, name: &str, pattern: Pattern) -> Result<(), String> {
        let name = name.trim();
        check_name(name, "pattern")?;
        match self
            .entries
            .iter_mut()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
        {
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

/// A name fit for a line of the file; `what` is what it names (a
/// pattern, a phrase or a scale), for the message.
fn check_name(name: &str, what: &str) -> Result<(), String> {
    let n = name.chars().count();
    if n == 0 {
        return Err(format!("the {what} needs a name before ="));
    }
    if n > MAX_NAME {
        return Err(format!("names are at most {MAX_NAME} characters"));
    }
    if name.contains('=') || name.starts_with('#') || name.chars().any(char::is_control) {
        return Err("names may not contain = or control characters, or start with #".into());
    }
    Ok(())
}

/// Whether a line's text is a phrase: it has the `:` that ends a phrase's
/// heading on its own, or it starts with a kind or a scale. (A pattern has
/// colons only inside a step, as in `x:2`, and none of its steps is one of
/// those words.)
fn is_phrase(body: &str) -> bool {
    let mut words = body.split_whitespace();
    let first = words.next().unwrap_or("");
    let first = first.strip_suffix(':').unwrap_or(first);
    Kind::ALL.iter().any(|k| k.word() == first)
        || Written::ALL.iter().any(|w| w.word() == first)
        || body.split_whitespace().any(|w| w == ":")
}

/// The `n`th (1-based) token of a phrase's text as `Phrase::parse` counts
/// them: the heading's words, the steps, then each lane's name and values
/// (the `:` and the `|`s aren't counted).
fn nth_phrase_token(text: &str, n: usize) -> Option<&str> {
    let (head, body) = text.split_once(':')?;
    head.split_whitespace()
        .chain(
            body.split(|c: char| c.is_whitespace() || c == '|')
                .filter(|w| !w.is_empty()),
        )
        .nth(n.checked_sub(1)?)
}

/// The `n`th (1-based) whitespace-separated token, counting `|` lane
/// names and values the same way `Pattern::parse` does.
fn nth_token(text: &str, n: usize) -> Option<&str> {
    text.split(|c: char| c.is_whitespace() || c == '|')
        .filter(|w| !w.is_empty())
        .nth(n.checked_sub(1)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_owners_scales_go_in_the_same_file() {
        let text = "Skip = 1 . 3 2\n[scales]\nBright minor = 1 2 b3 4 5 6 7\nClassical = 0 2 3 5 7 9 11 / 0 2 3 5 7 8 10\n\
                    [patterns]\nLift = 1 2 3 4\n";
        let lib = Library::parse(text).unwrap();
        assert_eq!(lib.len(), 2, "patterns before and after the scales");
        let scales: Vec<_> = lib.scales().collect();
        assert_eq!(
            scales,
            [
                (
                    "Bright minor",
                    mpfree_scale::MELODIC_MINOR,
                    mpfree_scale::MELODIC_MINOR
                ),
                (
                    "Classical",
                    mpfree_scale::MELODIC_MINOR,
                    mpfree_scale::MINOR
                )
            ]
        );
        let again = Library::parse(&lib.to_text()).unwrap();
        assert_eq!(again, lib);
        assert!(lib.to_text().contains("[scales]\nBright minor = 0 2 3 5 7 9 11\nClassical    = 0 2 3 5 7 9 11 / 0 2 3 5 7 8 10\n"));
        let errors =
            Library::parse("[scales]\nDorian = 0 2 3\nOdd = 0 2 13\nFine = 0 4 7\n").unwrap_err();
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].line, 2);
        assert!(
            errors[0].reason.contains("built-in scale"),
            "{}",
            errors[0].reason
        );
        assert!(
            errors[1].reason.contains("\"13\" is not a semitone"),
            "{}",
            errors[1].reason
        );
        assert!(Library::default().to_text().is_empty());
    }

    /// Phrases sit among the patterns, told apart by what a phrase line
    /// says it is; the two are named apart; a file writes back as itself.
    #[test]
    fn phrases_go_in_the_same_file_as_patterns() {
        let text = "Skip = 1 . 3 2 . 3 x:2 .\n\
                    Night = bass minor 1/8 : 1 1 1+ 1 1 1 1+ 1\n\
                    Skip = riff either 1/8 : 1 3 5 _ . 5 3 .\n\
                    [phrases]\n\
                    Lift = chords 1/8: 1,3,5 . 1,3,5 . . 1,3,5 . .\n\
                    [scales]\nBright minor = 1 2 b3 4 5 6 7\n";
        let lib = Library::parse(text).unwrap();
        assert_eq!(lib.len(), 1, "one pattern");
        let names: Vec<&str> = lib.phrases().map(|(n, _)| n).collect();
        assert_eq!(names, ["Night", "Skip", "Lift"]);
        assert_eq!(lib.phrase("night").unwrap().kind, Kind::Bass);
        assert_eq!(lib.phrase("LIFT").unwrap().kind, Kind::Chords);
        assert!(lib.get("Skip").is_some() && lib.phrase("Skip").is_some());
        let out = lib.to_text();
        assert_eq!(
            out,
            "Skip = 1 . 3 2 . 3 x:2 .\n\n\
             Night = bass minor 1/8 : 1 1 1+ 1 1 1 1+ 1\n\
             Skip  = riff either 1/8 : 1 3 5 _ . 5 3 .\n\
             Lift  = chords either 1/8 : 1,3,5 . 1,3,5 . . 1,3,5 . .\n\n\
             [scales]\nBright minor = 0 2 3 5 7 9 11\n"
        );
        assert_eq!(Library::parse(&out).unwrap(), lib);

        // A bad phrase line says which word is wrong, counted as written.
        let errors = Library::parse(
            "A = bass minor : 1 . z 1\nA = bass : 1\nB = bass : 1\nB = bass : 1 2\n\
             C = organ either : 1\nD = bass minor : 1 | vel 200\nE = bass :\n = bass : 1\n",
        )
        .unwrap_err();
        let lines: Vec<(usize, &str)> =
            errors.iter().map(|e| (e.line, e.reason.as_str())).collect();
        assert_eq!(
            lines,
            [
                (
                    1,
                    "\"A\", token 5 \"z\": a note is a scale step 1 to 7, maybe with b or #, then + or -"
                ),
                (4, "\"B\" is already in the file"),
                (
                    5,
                    "\"C\", token 1 \"organ\": unknown word (a kind, a scale, a grid, a meter, swung or N bars)"
                ),
                (6, "\"D\", token 5 \"200\": velocity must be 0 to 127"),
                (7, "\"E\": no steps"),
                (8, "the phrase needs a name before ="),
            ]
        );

        let mut lib = Library::default();
        let p = Phrase::parse("line either : 1 3 5 3").unwrap();
        lib.put_phrase("Mine", p).unwrap();
        lib.put_phrase("MINE", Phrase::default()).unwrap();
        assert_eq!(lib.phrases().count(), 1);
        assert_eq!(lib.phrases().next().unwrap().0, "MINE");
        assert!(lib.put_phrase("", p).is_err());
        assert!(lib.remove_phrase("mine") && !lib.remove_phrase("mine"));
    }

    /// What MPFree ships: 64 phrases of its own, 16 of each kind, each 1,
    /// 2 or 4 bars, no two alike, each reading back as itself, with every
    /// feel, both meters and all three scales among them.
    #[test]
    fn the_starter_phrases_are_sixteen_of_each_kind() {
        use crate::phrase::Feel;
        let lib = Library::built_in();
        let all: Vec<(&str, &Phrase)> = lib.phrases().collect();
        assert_eq!(all.len(), 64);
        for kind in Kind::ALL {
            let n = all.iter().filter(|(_, p)| p.kind == kind).count();
            assert_eq!(n, 16, "{kind:?}");
        }
        for (i, (name, p)) in all.iter().enumerate() {
            assert!(matches!(p.bars, 1 | 2 | 4), "{name}: {} bars", p.bars);
            assert!(p.len() >= 2, "{name}");
            assert_eq!(
                Phrase::parse(&p.to_text()).as_ref(),
                Ok(*p),
                "{name}: {}",
                p.to_text()
            );
            // Every bar has a note or one held into it.
            let bar = p.bar_ticks();
            for b in 0..u32::from(p.bars) {
                let (from, to) = (b * bar, (b + 1) * bar);
                assert!(
                    p.notes()
                        .iter()
                        .any(|n| n.start < to && n.start + n.len > from),
                    "{name}: bar {} is empty",
                    b + 1
                );
            }
            // Within three octaves of the held note.
            for n in p.notes() {
                assert!((-24..=36).contains(&n.semis), "{name}: {}", n.semis);
            }
            for (other, q) in &all[i + 1..] {
                assert!(
                    p.notes() != q.notes() || p.ticks() != q.ticks(),
                    "{name} and {other} are the same"
                );
            }
        }
        for feel in Feel::ALL {
            assert!(all.iter().any(|(_, p)| p.feel() == feel), "{feel:?}");
        }
        for w in Written::ALL {
            assert!(all.iter().any(|(_, p)| p.written == w), "{w:?}");
        }
        assert!(all.iter().any(|(_, p)| p.beats == 3));
        // The patterns are still every group's, the ten starters first.
        assert_eq!(lib.len(), crate::groups::count());
    }

    #[test]
    fn reads_and_writes_a_file() {
        let text = "# mine\n\nSkip = 1 . 3 2 . 3 x:2 .\n  Lift=1 2 3 4 | oct 0 0 1 | vel 110 70  \nShuffle = x x' x x' | nudge 0 12 0 8 | vel~15\n";
        let lib = Library::parse(text).unwrap();
        assert_eq!(lib.len(), 3);
        assert_eq!(
            lib.get("lift").unwrap().to_text(),
            "1 2 3 4 | oct 0 0 1 | vel 110 70"
        );
        let out = lib.to_text();
        assert_eq!(out.lines().next(), Some("Skip    = 1 . 3 2 . 3 x:2 ."));
        assert_eq!(Library::parse(&out).unwrap(), lib);
        assert_eq!(Library::built_in().len(), crate::groups::count());
        assert_eq!(
            Library::parse(&Library::built_in().to_text()).unwrap(),
            Library::built_in()
        );
    }

    #[test]
    fn reports_every_bad_line() {
        let text = "A = x\nno equals sign\n = x\nA = x x\nB = x y x\nC = x | vel 200\nThis name is far too long to be a name = x\n";
        let errors = Library::parse(text).unwrap_err();
        let lines: Vec<(usize, &str)> =
            errors.iter().map(|e| (e.line, e.reason.as_str())).collect();
        assert_eq!(
            lines,
            [
                (2, "expected: Name = pattern"),
                (3, "the pattern needs a name before ="),
                (4, "\"A\" is already in the file"),
                (
                    5,
                    "\"B\", token 2 \"y\": unknown step (use . _ x 1-9 * ? l h)"
                ),
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
        assert_eq!(lib.len(), crate::groups::count());
        assert_eq!(lib.iter().nth(2).unwrap(), ("BOUNCE", &p));
        lib.put("New one", p).unwrap();
        assert_eq!(lib.len(), crate::groups::count() + 1);
        assert!(lib.put("#hash", p).is_err());
        assert!(lib.remove("new ONE"));
        assert!(!lib.remove("new one"));
    }
}
