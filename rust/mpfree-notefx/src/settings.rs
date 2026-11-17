//! Each effect's settings as a list a screen can show and step through:
//! a name, the section it sits in, its range, and how its value reads.
//! A setting's value is a whole number (a choice's index, or the number
//! itself), so a page, a Q-Link and the wheel all move it the same way.

use mpfree_scale::{Scale, SCALES};

use crate::fx::{
    Chord, Echo, Fx, LatchMode, Randomize, Shape, StrumOrder, Time, Velocity, Voicing, NOTE_VALUES,
};

/// How a setting is shown and changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One of these, by index: a few are buttons side by side, more are
    /// a value between - and +.
    Choice(&'static [&'static str]),
    /// A number from `min` to `max`, read with [`Fx::value_text`].
    Number { min: i32, max: i32 },
}

/// One setting of an effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setting {
    pub name: &'static str,
    /// The card it sits in on the effect's page.
    pub section: &'static str,
    pub kind: Kind,
}

impl Setting {
    pub fn min(&self) -> i32 {
        match self.kind {
            Kind::Choice(_) => 0,
            Kind::Number { min, .. } => min,
        }
    }

    pub fn max(&self) -> i32 {
        match self.kind {
            Kind::Choice(list) => list.len() as i32 - 1,
            Kind::Number { max, .. } => max,
        }
    }
}

const fn choice(
    name: &'static str,
    section: &'static str,
    list: &'static [&'static str],
) -> Setting {
    Setting {
        name,
        section,
        kind: Kind::Choice(list),
    }
}

const fn number(name: &'static str, section: &'static str, min: i32, max: i32) -> Setting {
    Setting {
        name,
        section,
        kind: Kind::Number { min, max },
    }
}

const ROOTS: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
const SCALE_NAMES: [&str; 49] = {
    let mut out = [""; 49];
    let mut i = 0;
    while i < 49 {
        out[i] = SCALES[i].name;
        i += 1;
    }
    out
};
const SNAPS: [&str; 4] = ["Down", "Up", "Nearest", "Skip"];
const SHAPES: [&str; 14] = [
    "Octave", "Fifth", "Major", "Minor", "7th", "m7", "maj7", "sus2", "sus4", "add9", "dim", "aug",
    "In key", "Custom",
];
const VOICINGS: [&str; 3] = ["Close", "Spread", "Drop 2"];
const TIME_IN: [&str; 2] = ["Note", "ms"];
const NOTE_NAMES: [&str; 11] = {
    let mut out = [""; 11];
    let mut i = 0;
    while i < 11 {
        out[i] = NOTE_VALUES[i].1;
        i += 1;
    }
    out
};
const VEL_MODE: [&str; 2] = ["Curve", "Fixed"];

const TRANSPOSE: [Setting; 1] = [number("Semitones", "Transpose", -48, 48)];
const SCALE: [Setting; 3] = [
    choice("Root", "Scale", &ROOTS),
    choice("Scale", "Scale", &SCALE_NAMES),
    choice("Outside it", "Notes outside", &SNAPS),
];
const CHORD: [Setting; 9] = [
    choice("Chord", "Chord", &SHAPES),
    choice("Voicing", "Voicing", &VOICINGS),
    number("Inversion", "Voicing", 0, 3),
    number("Keep below", "Voicing", -1, 127),
    number("Velocity", "Added notes", 1, 100),
    number("Strum", "Added notes", 0, 200),
    number("In key notes", "In key", 2, 5),
    choice("Key", "In key", &ROOTS),
    choice("Key scale", "In key", &SCALE_NAMES),
];
const VELOCITY: [Setting; 5] = [
    choice("Velocity", "Velocity", &VEL_MODE),
    number("Fixed", "Velocity", 1, 127),
    number("Curve", "Curve", -100, 100),
    number("Lowest", "Curve", 1, 127),
    number("Highest", "Curve", 1, 127),
];
const LENGTH: [Setting; 3] = [
    choice("Length in", "Length", &TIME_IN),
    choice("Note value", "Length", &NOTE_NAMES),
    number("Time", "Length", 1, 2000),
];
const ECHO: [Setting; 6] = [
    choice("Time in", "Time", &TIME_IN),
    choice("Note value", "Time", &NOTE_NAMES),
    number("Time ms", "Time", 1, 2000),
    number("Repeats", "Repeats", 1, 16),
    number("Feedback", "Repeats", 1, 100),
    number("Pitch", "Repeats", -12, 12),
];
const HUMANIZE: [Setting; 2] = [
    number("Late by", "Humanize", 0, 100),
    number("Velocity", "Humanize", 0, 64),
];
const CHANCE: [Setting; 1] = [number("Chance", "Chance", 0, 100)];
const ORDERS: [&str; 3] = ["Up", "Down", "As played"];
const STRUM: [Setting; 2] = [
    number("Time", "Strum", 1, 200),
    choice("Order", "Strum", &ORDERS),
];
const LATCH_MODES: [&str; 2] = ["Next chord", "Toggle"];
const LATCH: [Setting; 1] = [choice("Latch", "Latch", &LATCH_MODES)];
const RANDOMIZE: [Setting; 4] = [
    number("Pitch", "Pitch", 0, 24),
    number("How often", "Pitch", 0, 100),
    number("Velocity", "Velocity and length", 0, 64),
    number("Longer by", "Velocity and length", 0, 1000),
];

fn note_value_index(t: u32) -> i32 {
    NOTE_VALUES.iter().position(|(v, _)| *v == t).unwrap_or(7) as i32
}

fn scale_index(s: &Scale) -> i32 {
    SCALES
        .iter()
        .position(|d| d.mask == s.mask && d.down == s.down)
        .unwrap_or(1) as i32
}

fn shape_index(s: Shape) -> i32 {
    match s {
        Shape::InKey(_) => 12,
        Shape::Custom(_) => 13,
        s => Shape::LIST.iter().position(|x| *x == s).unwrap_or(2) as i32,
    }
}

/// A time's three settings: whether it is a note value, which, and the
/// milliseconds.
fn time_get(t: Time, k: usize) -> i32 {
    match (t, k) {
        (Time::Ticks(_), 0) => 0,
        (Time::Ms(_), 0) => 1,
        (Time::Ticks(v), 1) => note_value_index(v),
        (Time::Ms(_), 1) => 7,
        (Time::Ms(ms), _) => i32::from(ms),
        (Time::Ticks(_), _) => 250,
    }
}

fn time_set(t: Time, k: usize, v: i32) -> Time {
    match k {
        0 if v == 0 => match t {
            Time::Ticks(_) => t,
            Time::Ms(_) => Time::Ticks(NOTE_VALUES[7].0),
        },
        0 => match t {
            Time::Ms(_) => t,
            Time::Ticks(_) => Time::Ms(250),
        },
        1 => Time::Ticks(NOTE_VALUES[v.clamp(0, 10) as usize].0),
        _ => Time::Ms(v.clamp(1, 2000) as u16),
    }
}

impl Fx {
    /// Its settings, in the order its page shows them.
    pub fn settings(&self) -> &'static [Setting] {
        match self {
            Fx::Transpose { .. } => &TRANSPOSE,
            Fx::Scale { .. } => &SCALE,
            Fx::Chord(_) => &CHORD,
            Fx::Velocity(_) => &VELOCITY,
            Fx::Length { .. } => &LENGTH,
            Fx::Echo(_) => &ECHO,
            Fx::Humanize { .. } => &HUMANIZE,
            Fx::Chance { .. } => &CHANCE,
            Fx::Strum(_) => &STRUM,
            Fx::Latch { .. } => &LATCH,
            Fx::Randomize(_) => &RANDOMIZE,
        }
    }

    /// Whether setting `k` matters as the effect is set (a fixed velocity
    /// has no curve; a time in ms has no note value): the page shows only
    /// those.
    pub fn shows(&self, k: usize) -> bool {
        match self {
            Fx::Chord(c) => match k {
                6..=8 => matches!(c.shape, Shape::InKey(_)),
                _ => true,
            },
            Fx::Velocity(v) => match k {
                1 => v.fixed.is_some(),
                2..=4 => v.fixed.is_none(),
                _ => true,
            },
            Fx::Length { time } => match k {
                1 => matches!(time, Time::Ticks(_)),
                2 => matches!(time, Time::Ms(_)),
                _ => true,
            },
            Fx::Echo(e) => match k {
                1 => matches!(e.time, Time::Ticks(_)),
                2 => matches!(e.time, Time::Ms(_)),
                _ => true,
            },
            _ => k < self.settings().len(),
        }
    }

    /// The settings Q-Links 1 to 4 turn (fewer when it has fewer), the
    /// ones a player reaches for.
    pub fn qlinks(&self) -> &'static [usize] {
        match self {
            Fx::Transpose { .. } => &[0],
            Fx::Scale { .. } => &[0, 1, 2],
            Fx::Chord(_) => &[0, 1, 2, 4],
            Fx::Velocity(v) if v.fixed.is_some() => &[0, 1],
            Fx::Velocity(_) => &[2, 3, 4, 0],
            Fx::Length {
                time: Time::Ticks(_),
            } => &[1, 0],
            Fx::Length { .. } => &[2, 0],
            Fx::Echo(Echo {
                time: Time::Ticks(_),
                ..
            }) => &[1, 3, 4, 5],
            Fx::Echo(_) => &[2, 3, 4, 5],
            Fx::Humanize { .. } => &[0, 1],
            Fx::Chance { .. } => &[0],
            Fx::Strum(_) => &[0, 1],
            Fx::Latch { .. } => &[0],
            Fx::Randomize(_) => &[0, 1, 2, 3],
        }
    }

    /// Setting `k`'s value (0 past the last).
    pub fn get(&self, k: usize) -> i32 {
        match *self {
            Fx::Transpose { semitones } => i32::from(semitones),
            Fx::Scale { scale, snap } => match k {
                0 => i32::from(scale.root),
                1 => scale_index(&scale),
                _ => mpfree_scale::Snap::ALL
                    .iter()
                    .position(|s| *s == snap)
                    .unwrap_or(0) as i32,
            },
            Fx::Chord(c) => match k {
                0 => shape_index(c.shape),
                1 => Voicing::ALL
                    .iter()
                    .position(|v| *v == c.voicing)
                    .unwrap_or(0) as i32,
                2 => i32::from(c.inversion),
                3 => c.keep_below.map_or(-1, i32::from),
                4 => i32::from(c.velocity),
                5 => i32::from(c.strum_ms),
                6 => match c.shape {
                    Shape::InKey(n) => i32::from(n),
                    _ => 3,
                },
                7 => c.scale.map_or(0, |s| i32::from(s.root)),
                _ => c.scale.map_or(1, |s| scale_index(&s)),
            },
            Fx::Velocity(v) => match k {
                0 => i32::from(v.fixed.is_some()),
                1 => i32::from(v.fixed.unwrap_or(100)),
                2 => i32::from(v.curve),
                3 => i32::from(v.min),
                _ => i32::from(v.max),
            },
            Fx::Length { time } => time_get(time, k),
            Fx::Echo(e) => match k {
                0..=2 => time_get(e.time, k),
                3 => i32::from(e.repeats),
                4 => i32::from(e.feedback),
                _ => i32::from(e.pitch),
            },
            Fx::Humanize { time_ms, velocity } => {
                i32::from(if k == 0 { time_ms } else { velocity })
            }
            Fx::Chance { percent } => i32::from(percent),
            Fx::Strum(st) => match k {
                0 => i32::from(st.time_ms),
                _ => StrumOrder::ALL
                    .iter()
                    .position(|o| *o == st.order)
                    .unwrap_or(0) as i32,
            },
            Fx::Latch { mode } => {
                LatchMode::ALL.iter().position(|m| *m == mode).unwrap_or(0) as i32
            }
            Fx::Randomize(Randomize {
                pitch,
                often,
                velocity,
                longer_ms,
            }) => match k {
                0 => i32::from(pitch),
                1 => i32::from(often),
                2 => i32::from(velocity),
                _ => i32::from(longer_ms),
            },
        }
    }

    /// Sets setting `k` to `v`, kept in its range.
    pub fn set(&mut self, k: usize, v: i32) {
        let Some(s) = self.settings().get(k) else {
            return;
        };
        let v = v.clamp(s.min(), s.max());
        match self {
            Fx::Transpose { semitones } => *semitones = v as i8,
            Fx::Scale { scale, snap } => match k {
                0 => *scale = Scale::with_down(v as u8, scale.mask, scale.down),
                1 => *scale = Scale::of(scale.root, &SCALES[v as usize]),
                _ => *snap = mpfree_scale::Snap::ALL[v as usize],
            },
            Fx::Chord(c) => set_chord(c, k, v),
            Fx::Velocity(vel) => set_velocity(vel, k, v),
            Fx::Length { time } => *time = time_set(*time, k, v),
            Fx::Echo(e) => match k {
                0..=2 => e.time = time_set(e.time, k, v),
                3 => e.repeats = v as u8,
                4 => e.feedback = v as u8,
                _ => e.pitch = v as i8,
            },
            Fx::Humanize { time_ms, velocity } => {
                if k == 0 {
                    *time_ms = v as u8;
                } else {
                    *velocity = v as u8;
                }
            }
            Fx::Chance { percent } => *percent = v as u8,
            Fx::Strum(st) => match k {
                0 => st.time_ms = v as u16,
                _ => st.order = StrumOrder::ALL[v as usize],
            },
            Fx::Latch { mode } => *mode = LatchMode::ALL[v as usize],
            Fx::Randomize(r) => match k {
                0 => r.pitch = v as u8,
                1 => r.often = v as u8,
                2 => r.velocity = v as u8,
                _ => r.longer_ms = v as u16,
            },
        }
    }

    /// Moves setting `k` by `delta` steps.
    pub fn step(&mut self, k: usize, delta: i32) {
        let v = self.get(k);
        self.set(k, v.saturating_add(delta));
    }

    /// How setting `k`'s value reads ("m7", "+7 st", "90 %", "C5").
    pub fn value_text(&self, k: usize) -> String {
        let Some(s) = self.settings().get(k) else {
            return String::new();
        };
        let v = self.get(k);
        if let Kind::Choice(list) = s.kind {
            return list.get(v as usize).copied().unwrap_or("").to_string();
        }
        let signed = |unit: &str| format!("{v:+}{unit}");
        match (self, k) {
            (Fx::Transpose { .. }, _) => signed(" st"),
            (Fx::Chord(_), 3) if v < 0 => "Off".to_string(),
            (Fx::Chord(_), 3) => mpfree_scale::note_name(v as i16),
            (Fx::Chord(_), 4) | (Fx::Echo(_), 4) | (Fx::Chance { .. }, _) => format!("{v} %"),
            (Fx::Chord(_), 5) | (Fx::Length { .. }, 2) | (Fx::Echo(_), 2) => format!("{v} ms"),
            (Fx::Humanize { .. }, 0) | (Fx::Strum(_), 0) | (Fx::Randomize(_), 3) => {
                format!("{v} ms")
            }
            (Fx::Humanize { .. }, _) | (Fx::Randomize(_), 2) => format!("±{v}"),
            (Fx::Randomize(_), 0) => format!("±{v} st"),
            (Fx::Randomize(_), 1) => format!("{v} %"),
            (Fx::Velocity(_), 2) | (Fx::Echo(_), 5) => {
                if v == 0 {
                    "0".to_string()
                } else {
                    signed(if k == 5 { " st" } else { "" })
                }
            }
            _ => v.to_string(),
        }
    }
}

impl Fx {
    /// What `played` becomes in pitch, lowest first, for a drawing of what
    /// goes in and what comes out: empty when it isn't played (a Scale
    /// that skips it); the note itself for effects that keep pitches.
    pub fn pitches(&self, played: u8) -> Vec<u8> {
        match self {
            Fx::Transpose { semitones } => {
                let n = i16::from(played) + i16::from(*semitones);
                (0..=127)
                    .contains(&n)
                    .then_some(n as u8)
                    .into_iter()
                    .collect()
            }
            Fx::Scale { scale, snap } => scale
                .snapped(i16::from(played), *snap)
                .filter(|n| (0..=127).contains(n))
                .map(|n| n as u8)
                .into_iter()
                .collect(),
            Fx::Chord(c) => {
                let (notes, len, _) = c.notes(played);
                notes[..len].to_vec()
            }
            _ => vec![played],
        }
    }

    /// What it does, in a few words, for its page's header.
    pub fn does(&self) -> &'static str {
        match self {
            Fx::Transpose { .. } => "moves every note",
            Fx::Scale { .. } => "keeps notes in a scale",
            Fx::Chord(_) => "adds notes to each one played",
            Fx::Velocity(_) => "sets how hard each note plays",
            Fx::Length { .. } => "makes every note as long",
            Fx::Echo(_) => "plays each note again, softer",
            Fx::Humanize { .. } => "moves notes a little, never the same",
            Fx::Chance { .. } => "plays each note with a chance",
            Fx::Strum(_) => "plays a chord's notes one after another",
            Fx::Latch { .. } => "keeps notes on after the keys let go",
            Fx::Randomize(_) => "moves notes at random, never the same",
        }
    }
}

fn set_chord(c: &mut Chord, k: usize, v: i32) {
    match k {
        0 => {
            c.shape = match v {
                12 => Shape::InKey(match c.shape {
                    Shape::InKey(n) => n,
                    _ => 3,
                }),
                13 => match c.shape {
                    Shape::Custom(m) => Shape::Custom(m),
                    // A custom chord starts as the one it was.
                    other => Shape::Custom(custom_of(other)),
                },
                i => Shape::LIST[i as usize],
            }
        }
        1 => c.voicing = Voicing::ALL[v as usize],
        2 => c.inversion = v as u8,
        3 => c.keep_below = (v >= 0).then_some(v as u8),
        4 => c.velocity = v as u8,
        5 => c.strum_ms = v as u16,
        6 => {
            if let Shape::InKey(_) = c.shape {
                c.shape = Shape::InKey(v as u8);
            }
        }
        7 => {
            let s = c.scale.unwrap_or(Scale::new(0, mpfree_scale::MAJOR));
            c.scale = Some(Scale::with_down(v as u8, s.mask, s.down));
        }
        _ => {
            let root = c.scale.map_or(0, |s| s.root);
            c.scale = Some(Scale::of(root, &SCALES[v as usize]));
        }
    }
}

/// A shape's notes above the note played, as a custom chord's bits.
fn custom_of(s: Shape) -> u32 {
    let c = Chord {
        shape: s,
        voicing: Voicing::Close,
        inversion: 0,
        keep_below: None,
        velocity: 100,
        strum_ms: 0,
        scale: None,
    };
    let (notes, len, _) = c.notes(48);
    notes[..len]
        .iter()
        .filter(|&&n| n > 48 && n <= 72)
        .fold(0, |m, &n| m | 1 << (n - 49))
}

fn set_velocity(vel: &mut Velocity, k: usize, v: i32) {
    match k {
        0 => vel.fixed = (v == 1).then_some(vel.fixed.unwrap_or(100)),
        1 => vel.fixed = Some(v as u8),
        2 => vel.curve = v as i8,
        3 => vel.min = v as u8,
        _ => vel.max = v as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setting_reads_back_what_it_was_set_to() {
        for fx in Fx::all() {
            for (k, s) in fx.settings().iter().enumerate() {
                for v in [s.min(), (s.min() + s.max()) / 2, s.max()] {
                    let mut f = fx;
                    f.set(k, v);
                    if f.shows(k) {
                        assert_eq!(f.get(k), v, "{} {}", fx.name(), s.name);
                    }
                    assert!(!f.value_text(k).is_empty());
                    // Out of range stays in it.
                    f.set(k, s.max() + 5);
                    assert!(f.get(k) <= s.max());
                }
            }
            for &q in fx.qlinks() {
                assert!(q < fx.settings().len() && fx.shows(q), "{}", fx.name());
            }
        }
    }

    #[test]
    fn a_chord_steps_through_its_shapes_and_custom_keeps_its_notes() {
        let mut f = Fx::Chord(Chord {
            shape: Shape::Minor7,
            ..Chord::default()
        });
        assert_eq!(f.value_text(0), "m7");
        f.step(0, 1);
        assert_eq!(f.value_text(0), "maj7");
        f.set(0, 13);
        let Fx::Chord(c) = f else { unreachable!() };
        // maj7's notes above the root: 4, 7 and 11 semitones.
        assert_eq!(c.shape, Shape::Custom(1 << 3 | 1 << 6 | 1 << 10));
        assert_eq!(c.notes(48).1, 4);
        f.set(3, -1);
        assert_eq!(f.value_text(3), "Off");
        f.set(3, 72);
        assert_eq!(f.value_text(3), "C5");
        assert!(!f.shows(7));
        f.set(0, 12);
        assert!(f.shows(7));
    }

    #[test]
    fn times_switch_between_note_values_and_ms() {
        let mut f = Fx::Echo(Echo::default());
        assert_eq!(f.value_text(1), "1/8");
        assert!(f.shows(1) && !f.shows(2));
        f.set(0, 1);
        assert!(!f.shows(1) && f.shows(2));
        assert_eq!(f.value_text(2), "250 ms");
        f.step(2, 10);
        assert_eq!(f.get(2), 260);
        f.set(0, 0);
        assert_eq!(f.value_text(1), "1/16");
        let mut v = Fx::Velocity(Velocity::default());
        assert_eq!(v.qlinks(), &[2, 3, 4, 0]);
        v.set(0, 1);
        assert_eq!(v.value_text(1), "100");
        assert_eq!(v.qlinks(), &[0, 1]);
    }
}
