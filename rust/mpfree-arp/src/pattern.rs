//! Latch patterns: a rhythm and a melody shape that play over whatever notes
//! are held or latched.
//!
//! A pattern is a row of steps written as text, one token per step, so a
//! pattern fits in a settings file, a log line or a test:
//!
//! ```text
//! x . x x   1 3 2 3   1 2 3 1+ 2+ 3+   * . . *   x x:2 x x:3   x x%60 ? .
//! ```
//!
//! | Token | Step |
//! |---|---|
//! | `.` | rest |
//! | `_` | tie: the notes of the step before hold on through this one |
//! | `x` | the next note in the arpeggiator's order |
//! | `1`..`9` | the Nth held note, lowest first (wraps if fewer are held) |
//! | `*` | every held note at once (a chord) |
//! | `?` | a random held note |
//! | `l`, `h` | the lowest or highest held note |
//!
//! After a note token, in any order: `+` or `-` moves it an octave up or down
//! (up to three), `!` accents it, `~` holds it for the whole step, `'` makes
//! it short, `:N` plays it N times within the step (2..=8, a ratchet) and
//! `%N` plays it with an N% chance.

/// Longest pattern, in steps (four bars of sixteenths).
pub const MAX_STEPS: usize = 64;

/// Which held note a step plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// The next note in the arpeggiator's order (with its octaves).
    Next,
    /// The Nth held note, lowest first, 1-based; wraps if fewer are held.
    Index(u8),
    /// All held notes together.
    All,
    Random,
    Lowest,
    Highest,
}

/// Gate lengths that `~` and `'` select, in percent of the step.
pub const GATE_LEGATO: u8 = 100;
pub const GATE_SHORT: u8 = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Rest,
    Tie,
    Play {
        pick: Pick,
        /// Octaves up (positive) or down, -3..=3.
        octave: i8,
        accent: bool,
        /// Note length in percent of the step; 0 = the engine's gate setting.
        gate: u8,
        /// Hits within the step, 1..=8.
        ratchet: u8,
        /// Chance to play, 1..=100 percent.
        chance: u8,
    },
}

impl Step {
    pub const fn play(pick: Pick) -> Self {
        Step::Play { pick, octave: 0, accent: false, gate: 0, ratchet: 1, chance: 100 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based position of the token that is wrong.
    pub token: usize,
    pub reason: &'static str,
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "step {}: {}", self.token, self.reason)
    }
}

impl std::error::Error for ParseError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pattern {
    steps: [Step; MAX_STEPS],
    len: usize,
}

impl Default for Pattern {
    /// One step that plays the next note: a plain arpeggio.
    fn default() -> Self {
        let mut p = Self { steps: [Step::Rest; MAX_STEPS], len: 1 };
        p.steps[0] = Step::play(Pick::Next);
        p
    }
}

impl Pattern {
    pub fn steps(&self) -> &[Step] {
        &self.steps[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Builds a pattern from steps (at least one, at most [`MAX_STEPS`]).
    pub fn from_steps(steps: &[Step]) -> Option<Self> {
        if steps.is_empty() || steps.len() > MAX_STEPS {
            return None;
        }
        let mut p = Self { steps: [Step::Rest; MAX_STEPS], len: steps.len() };
        p.steps[..steps.len()].copy_from_slice(steps);
        Some(p)
    }

    /// Replaces one step, e.g. from a tap on the step grid.
    pub fn set_step(&mut self, index: usize, step: Step) -> bool {
        if index < self.len {
            self.steps[index] = step;
            true
        } else {
            false
        }
    }

    /// Reads the text form described in the module docs.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let mut p = Self { steps: [Step::Rest; MAX_STEPS], len: 0 };
        for (i, token) in text.split_whitespace().enumerate() {
            let at = |reason| ParseError { token: i + 1, reason };
            if p.len == MAX_STEPS {
                return Err(at("more than 64 steps"));
            }
            p.steps[p.len] = parse_step(token).map_err(at)?;
            p.len += 1;
        }
        if p.len == 0 {
            return Err(ParseError { token: 0, reason: "no steps" });
        }
        Ok(p)
    }

    /// Writes the text form; `parse` reads it back to the same pattern.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for (i, step) in self.steps().iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            write_step(&mut s, step);
        }
        s
    }
}

fn parse_step(token: &str) -> Result<Step, &'static str> {
    let b = token.as_bytes();
    let pick = match b[0] {
        b'.' | b'_' if b.len() > 1 => return Err("a rest or tie takes nothing after it"),
        b'.' => return Ok(Step::Rest),
        b'_' => return Ok(Step::Tie),
        b'x' => Pick::Next,
        b'1'..=b'9' => Pick::Index(b[0] - b'0'),
        b'*' => Pick::All,
        b'?' => Pick::Random,
        b'l' => Pick::Lowest,
        b'h' => Pick::Highest,
        _ => return Err("unknown step (use . _ x 1-9 * ? l h)"),
    };
    let (mut octave, mut accent, mut gate, mut ratchet, mut chance) = (0i8, false, 0u8, 1u8, 100u8);
    let mut i = 1;
    while i < b.len() {
        let c = b[i];
        i += 1;
        match c {
            b'+' | b'-' => {
                octave += if c == b'+' { 1 } else { -1 };
                if !(-3..=3).contains(&octave) {
                    return Err("octave shift is limited to three");
                }
            }
            b'!' => accent = true,
            b'~' => gate = GATE_LEGATO,
            b'\'' => gate = GATE_SHORT,
            b':' | b'%' => {
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                let n: u32 = token[start..i].parse().map_err(|_| "a number must follow : or %")?;
                if c == b':' {
                    if !(1..=8).contains(&n) {
                        return Err("ratchet must be 1 to 8");
                    }
                    ratchet = n as u8;
                } else {
                    if !(1..=100).contains(&n) {
                        return Err("chance must be 1 to 100");
                    }
                    chance = n as u8;
                }
            }
            _ => return Err("unknown mark after the note (use + - ! ~ ' :N %N)"),
        }
    }
    Ok(Step::Play { pick, octave, accent, gate, ratchet, chance })
}

fn write_step(s: &mut String, step: &Step) {
    use std::fmt::Write;
    match *step {
        Step::Rest => s.push('.'),
        Step::Tie => s.push('_'),
        Step::Play { pick, octave, accent, gate, ratchet, chance } => {
            match pick {
                Pick::Next => s.push('x'),
                Pick::Index(n) => {
                    let _ = write!(s, "{n}");
                }
                Pick::All => s.push('*'),
                Pick::Random => s.push('?'),
                Pick::Lowest => s.push('l'),
                Pick::Highest => s.push('h'),
            }
            let mark = if octave > 0 { '+' } else { '-' };
            for _ in 0..octave.unsigned_abs() {
                s.push(mark);
            }
            if accent {
                s.push('!');
            }
            match gate {
                GATE_LEGATO => s.push('~'),
                GATE_SHORT => s.push('\''),
                _ => {}
            }
            if ratchet != 1 {
                let _ = write!(s, ":{ratchet}");
            }
            if chance != 100 {
                let _ = write!(s, "%{chance}");
            }
        }
    }
}

/// MPFree's own starter patterns (original, not copied from any product).
pub const BUILT_IN: [(&str, &str); 8] = [
    ("Straight", "x"),
    ("Gallop", "x x . x"),
    ("Bounce", "1 3 2 3"),
    ("Climb", "1 2 3 1+ 2+ 3+ 1++ ."),
    ("Pulse", "x! x x' x x! x' x x'"),
    ("Stabs", "* . . * . . * ."),
    ("Rolls", "x x:2 x x:3"),
    ("Drift", "x x%60 ? x%30 . x _ ?"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_kind_of_step() {
        let p = Pattern::parse(" x . _ 3 * ? l h ").unwrap();
        use Pick::*;
        assert_eq!(
            p.steps(),
            [
                Step::play(Next),
                Step::Rest,
                Step::Tie,
                Step::play(Index(3)),
                Step::play(All),
                Step::play(Random),
                Step::play(Lowest),
                Step::play(Highest)
            ]
        );
    }

    #[test]
    fn reads_marks_in_any_order() {
        let p = Pattern::parse("2++!:3%40~ x-'").unwrap();
        assert_eq!(
            p.steps()[0],
            Step::Play { pick: Pick::Index(2), octave: 2, accent: true, gate: GATE_LEGATO, ratchet: 3, chance: 40 }
        );
        assert_eq!(
            p.steps()[1],
            Step::Play { pick: Pick::Next, octave: -1, accent: false, gate: GATE_SHORT, ratchet: 1, chance: 100 }
        );
    }

    #[test]
    fn says_which_step_is_wrong() {
        let cases = [
            ("", 0, "no steps"),
            ("x y", 2, "unknown step (use . _ x 1-9 * ? l h)"),
            ("x .!", 2, "a rest or tie takes nothing after it"),
            ("x++++", 1, "octave shift is limited to three"),
            ("x:9", 1, "ratchet must be 1 to 8"),
            ("x%0", 1, "chance must be 1 to 100"),
            ("x:", 1, "a number must follow : or %"),
            ("x x#", 2, "unknown mark after the note (use + - ! ~ ' :N %N)"),
        ];
        for (text, token, reason) in cases {
            assert_eq!(Pattern::parse(text), Err(ParseError { token, reason }), "{text:?}");
        }
        let long = vec!["x"; 65].join(" ");
        assert_eq!(Pattern::parse(&long).unwrap_err().token, 65);
    }

    #[test]
    fn text_round_trips_and_built_ins_parse() {
        for (name, text) in BUILT_IN {
            let p = Pattern::parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(p.to_text(), text, "{name}");
        }
        let p = Pattern::parse("2--!:3%40~ x' 9+++").unwrap();
        assert_eq!(Pattern::parse(&p.to_text()).unwrap(), p);
    }

    #[test]
    fn steps_can_be_edited() {
        let mut p = Pattern::parse("x x x x").unwrap();
        assert!(p.set_step(2, Step::Rest));
        assert!(!p.set_step(4, Step::Rest));
        assert_eq!(p.to_text(), "x x . x");
        assert!(Pattern::from_steps(&[]).is_none());
        assert_eq!(Pattern::from_steps(&[Step::Tie]).unwrap().len(), 1);
    }
}
