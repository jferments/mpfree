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
//!
//! Lanes follow the steps after a `|`, each with its own length, so they
//! loop against the steps (polymeter):
//!
//! ```text
//! x x . x | oct 0 0 1 | vel 127 70 90 | deg 0 2 4 | gate 30 80
//! ```
//!
//! `oct` adds octaves (-3..=3); `deg` moves the note by scale degrees, or
//! semitones without a scale (-24..=24); `vel` sets velocity (0 = as
//! played); `gate` sets the note length in percent (0 = default).

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

/// One lane of per-step values with its own length. A lane shorter or
/// longer than the steps loops on its own, so patterns can be polymetric.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lane<T: Copy> {
    values: [T; MAX_STEPS],
    len: usize,
}

impl<T: Copy + Default> Default for Lane<T> {
    fn default() -> Self {
        Self { values: [T::default(); MAX_STEPS], len: 0 }
    }
}

impl<T: Copy + Default> Lane<T> {
    /// A lane from up to [`MAX_STEPS`] values; an empty slice turns it off.
    pub fn new(values: &[T]) -> Option<Self> {
        if values.len() > MAX_STEPS {
            return None;
        }
        let mut l = Self::default();
        l.values[..values.len()].copy_from_slice(values);
        l.len = values.len();
        Some(l)
    }

    pub fn values(&self) -> &[T] {
        &self.values[..self.len]
    }

    pub fn is_off(&self) -> bool {
        self.len == 0
    }

    /// The value for the `step`th step played, or None if the lane is off.
    pub fn at(&self, step: u64) -> Option<T> {
        (self.len > 0).then(|| self.values[(step % self.len as u64) as usize])
    }
}

/// Optional lanes on top of the steps. Each loops at its own length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Lanes {
    /// Octaves added to the step's own octave, -3..=3.
    pub octave: Lane<i8>,
    /// Scale degrees (with a scale set) or semitones (without), -24..=24.
    pub degree: Lane<i8>,
    /// Velocity, 1..=127; 0 = as played. An accent still wins.
    pub velocity: Lane<u8>,
    /// Gate in percent, 1..=100; 0 = the step's or engine's gate.
    pub gate: Lane<u8>,
}

/// Lane names in the text form, in the order they are written.
const LANE_NAMES: [&str; 4] = ["oct", "deg", "vel", "gate"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pattern {
    steps: [Step; MAX_STEPS],
    len: usize,
    pub lanes: Lanes,
}

impl Default for Pattern {
    /// One step that plays the next note: a plain arpeggio.
    fn default() -> Self {
        let mut p = Self { steps: [Step::Rest; MAX_STEPS], len: 1, lanes: Lanes::default() };
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
        let mut p = Self { steps: [Step::Rest; MAX_STEPS], len: steps.len(), lanes: Lanes::default() };
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
        let mut parts = text.split('|');
        let mut p = Self { steps: [Step::Rest; MAX_STEPS], len: 0, lanes: Lanes::default() };
        let mut token = 0;
        for word in parts.next().unwrap_or("").split_whitespace() {
            token += 1;
            let t = token;
            let at = move |reason| ParseError { token: t, reason };
            if p.len == MAX_STEPS {
                return Err(at("more than 64 steps"));
            }
            p.steps[p.len] = parse_step(word).map_err(at)?;
            p.len += 1;
        }
        if p.len == 0 {
            return Err(ParseError { token: 0, reason: "no steps" });
        }
        for part in parts {
            let mut words = part.split_whitespace();
            token += 1;
            let t = token;
            let at = move |reason| ParseError { token: t, reason };
            let name = words.next().ok_or(at("an empty lane after |"))?;
            let (lo, hi) = match name {
                "oct" => (-3, 3),
                "deg" => (-24, 24),
                "vel" => (0, 127),
                "gate" => (0, 100),
                _ => return Err(at("unknown lane (use oct, deg, vel or gate)")),
            };
            let mut vals = [0i32; MAX_STEPS];
            let mut n = 0;
            for w in words {
                token += 1;
                let t = token;
            let at = move |reason| ParseError { token: t, reason };
                if n == MAX_STEPS {
                    return Err(at("more than 64 values in a lane"));
                }
                let v: i32 = w.parse().map_err(|_| at("a lane value must be a whole number"))?;
                if !(lo..=hi).contains(&v) {
                    return Err(at(match name {
                        "oct" => "octave must be -3 to 3",
                        "deg" => "degree must be -24 to 24",
                        "vel" => "velocity must be 0 to 127",
                        _ => "gate must be 0 to 100",
                    }));
                }
                vals[n] = v;
                n += 1;
            }
            if n == 0 {
                return Err(at("a lane needs at least one value"));
            }
            let v = &vals[..n];
            match name {
                "oct" => p.lanes.octave = lane_of(v, |x| x as i8),
                "deg" => p.lanes.degree = lane_of(v, |x| x as i8),
                "vel" => p.lanes.velocity = lane_of(v, |x| x as u8),
                _ => p.lanes.gate = lane_of(v, |x| x as u8),
            }
        }
        Ok(p)
    }

    /// The same pattern turned by `n` steps (positive = later). Lanes are
    /// left alone, as they loop on their own.
    pub fn rotated(&self, n: i32) -> Self {
        let mut p = *self;
        let len = self.len as i32;
        for i in 0..self.len {
            let from = (i as i32 - n).rem_euclid(len) as usize;
            p.steps[i] = self.steps[from];
        }
        p
    }

    /// A variation of this pattern. `amount` (0..=100) is the chance each
    /// step changes; `seed` picks which changes, so the same seed gives the
    /// same variation. The pattern itself is never changed, so going back
    /// to the original is always possible. Ties stay where they are, and at
    /// least one step still plays.
    pub fn mutated(&self, seed: u32, amount: u8) -> Self {
        let mut p = *self;
        let mut x = seed.max(1);
        let mut next = |n: u32| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x % n.max(1)
        };
        for i in 0..self.len {
            if next(100) >= u32::from(amount.min(100)) {
                continue;
            }
            p.steps[i] = match self.steps[i] {
                Step::Tie => Step::Tie,
                Step::Rest => Step::play(Pick::Next),
                Step::Play { pick, octave, accent, gate, ratchet, chance } => match next(6) {
                    0 => Step::Rest,
                    1 => Step::Play { pick, octave: (octave + if next(2) == 0 { 1 } else { -1 }).clamp(-3, 3), accent, gate, ratchet, chance },
                    2 => Step::Play { pick, octave, accent: !accent, gate, ratchet, chance },
                    3 => Step::Play { pick, octave, accent, gate, ratchet: if ratchet > 1 { 1 } else { 2 + next(2) as u8 }, chance },
                    4 => Step::Play { pick: Pick::Index(1 + next(4) as u8), octave, accent, gate, ratchet, chance },
                    _ => Step::Play { pick: Pick::Random, octave, accent, gate, ratchet, chance },
                },
            };
        }
        if !p.steps().iter().any(|s| matches!(s, Step::Play { .. })) {
            p.steps[0] = Step::play(Pick::Next);
        }
        p
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
        let l = &self.lanes;
        let lanes: [(&str, Vec<i32>); 4] = [
            (LANE_NAMES[0], l.octave.values().iter().map(|&v| i32::from(v)).collect()),
            (LANE_NAMES[1], l.degree.values().iter().map(|&v| i32::from(v)).collect()),
            (LANE_NAMES[2], l.velocity.values().iter().map(|&v| i32::from(v)).collect()),
            (LANE_NAMES[3], l.gate.values().iter().map(|&v| i32::from(v)).collect()),
        ];
        for (name, vals) in lanes {
            if !vals.is_empty() {
                s.push_str(" | ");
                s.push_str(name);
                for v in vals {
                    s.push(' ');
                    s.push_str(&v.to_string());
                }
            }
        }
        s
    }
}

fn lane_of<T: Copy + Default>(v: &[i32], f: impl Fn(i32) -> T) -> Lane<T> {
    let mut l = Lane::default();
    for (i, &x) in v.iter().enumerate() {
        l.values[i] = f(x);
    }
    l.len = v.len();
    l
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
pub const BUILT_IN: [(&str, &str); 10] = [
    ("Straight", "x"),
    ("Gallop", "x x . x"),
    ("Bounce", "1 3 2 3"),
    ("Climb", "1 2 3 1+ 2+ 3+ 1++ ."),
    ("Pulse", "x! x x' x x! x' x x'"),
    ("Stabs", "* . . * . . * ."),
    ("Rolls", "x x:2 x x:3"),
    ("Drift", "x x%60 ? x%30 . x _ ?"),
    ("Three over four", "x x x x | oct 0 1 0 | vel 120 70 90 70 80"),
    ("Climbing line", "1 1 1 1 1 1 1 1 | deg 0 2 4 5 7 4 2 | gate 40 40 80"),
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
    fn reads_lanes_with_their_own_lengths() {
        let p = Pattern::parse("x . x | oct 0 1 | vel 0 100 50 20 | deg -2 7 | gate 30").unwrap();
        assert_eq!(p.len(), 3);
        assert_eq!(p.lanes.octave.values(), [0, 1]);
        assert_eq!(p.lanes.velocity.values(), [0, 100, 50, 20]);
        assert_eq!(p.lanes.degree.values(), [-2, 7]);
        assert_eq!(p.lanes.gate.values(), [30]);
        assert_eq!(p.lanes.octave.at(5), Some(1));
        assert_eq!(Pattern::parse("x").unwrap().lanes.octave.at(3), None);
        assert_eq!(Pattern::parse(&p.to_text()).unwrap(), p);
        let errors = [
            ("x | foo 1", 2, "unknown lane (use oct, deg, vel or gate)"),
            ("x |", 2, "an empty lane after |"),
            ("x | oct", 2, "a lane needs at least one value"),
            ("x x | oct 0 4", 5, "octave must be -3 to 3"),
            ("x | vel 1 x", 4, "a lane value must be a whole number"),
            ("x | gate 101", 3, "gate must be 0 to 100"),
        ];
        for (text, token, reason) in errors {
            assert_eq!(Pattern::parse(text), Err(ParseError { token, reason }), "{text:?}");
        }
    }

    #[test]
    fn rotate_and_mutate_keep_the_original() {
        let p = Pattern::parse("1 2 . _").unwrap();
        assert_eq!(p.rotated(1).to_text(), "_ 1 2 .");
        assert_eq!(p.rotated(-1).to_text(), "2 . _ 1");
        assert_eq!(p.rotated(4), p);
        let base = Pattern::parse("x x x x x x x x . . _ x").unwrap();
        assert_eq!(base.mutated(7, 0), base);
        let m = base.mutated(7, 60);
        assert_ne!(m, base);
        assert_eq!(base.mutated(7, 60), m, "same seed, same variation");
        assert_eq!(m.steps()[10], Step::Tie);
        assert_eq!(m.len(), base.len());
        for seed in 1..200 {
            let m = Pattern::parse(". x .").unwrap().mutated(seed, 100);
            assert!(m.steps().iter().any(|s| matches!(s, Step::Play { .. })));
        }
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
