//! What each MIDI effect does, as settings. The chain ([`crate::Chain`])
//! keeps what each note became, so a setting changed while a key is held
//! ends that key's notes where they went.

use mpfree_scale::{Scale, Snap};

/// The most notes one played note can become (a chord of seven and the
/// note played).
pub const MAX_MAP: usize = 8;

/// A length in time: milliseconds, or a note value in ticks at 960 a
/// quarter note (240 is a 1/16), which follows the tempo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Time {
    Ms(u16),
    Ticks(u32),
}

/// The note values an effect offers, in ticks, with their names.
pub const NOTE_VALUES: [(u32, &str); 11] = [
    (3840, "1 bar"),
    (1920, "1/2"),
    (960, "1/4"),
    (640, "1/4T"),
    (480, "1/8"),
    (320, "1/8T"),
    (360, "1/16."),
    (240, "1/16"),
    (160, "1/16T"),
    (120, "1/32"),
    (80, "1/32T"),
];

impl Time {
    /// Frames at `rate` and `bpm` (at least 1, so an off never meets its
    /// own on).
    pub fn frames(self, rate: u32, bpm: f64) -> u64 {
        let f = match self {
            Time::Ms(ms) => f64::from(ms) * f64::from(rate) / 1000.0,
            Time::Ticks(t) => {
                let bpm = if bpm > 0.0 { bpm } else { 120.0 };
                f64::from(t) * 60.0 * f64::from(rate) / (bpm * 960.0)
            }
        };
        (f.round() as u64).max(1)
    }

    /// "120 ms", "1/16".
    pub fn label(self) -> String {
        match self {
            Time::Ms(ms) => format!("{ms} ms"),
            Time::Ticks(t) => NOTE_VALUES
                .iter()
                .find(|(v, _)| *v == t)
                .map_or_else(|| format!("{t} ticks"), |(_, n)| (*n).to_string()),
        }
    }
}

/// A chord's notes above the note played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Octave,
    Fifth,
    Major,
    Minor,
    /// Dominant 7th.
    Seventh,
    Minor7,
    Major7,
    Sus2,
    Sus4,
    Add9,
    Dim,
    Aug,
    /// Stacked thirds in the chord's scale: 2 to 5 notes, the one played
    /// first.
    InKey(u8),
    /// Semitones above the note played: bit N is N + 1 semitones (up to
    /// two octaves).
    Custom(u32),
}

impl Shape {
    /// The shapes on the screen, in order (In key with 3 notes).
    pub const LIST: [Shape; 13] = [
        Shape::Octave,
        Shape::Fifth,
        Shape::Major,
        Shape::Minor,
        Shape::Seventh,
        Shape::Minor7,
        Shape::Major7,
        Shape::Sus2,
        Shape::Sus4,
        Shape::Add9,
        Shape::Dim,
        Shape::Aug,
        Shape::InKey(3),
    ];

    pub fn label(self) -> String {
        match self {
            Shape::Octave => "Octave".into(),
            Shape::Fifth => "Fifth".into(),
            Shape::Major => "Major".into(),
            Shape::Minor => "Minor".into(),
            Shape::Seventh => "7th".into(),
            Shape::Minor7 => "m7".into(),
            Shape::Major7 => "maj7".into(),
            Shape::Sus2 => "sus2".into(),
            Shape::Sus4 => "sus4".into(),
            Shape::Add9 => "add9".into(),
            Shape::Dim => "dim".into(),
            Shape::Aug => "aug".into(),
            Shape::InKey(n) => format!("In key · {n} notes"),
            Shape::Custom(_) => "Custom".into(),
        }
    }

    /// Semitones above the note played, lowest first (In key needs the
    /// note and its scale: [`Chord::notes`]).
    fn intervals(self, out: &mut [i16; MAX_MAP]) -> usize {
        let list: &[i16] = match self {
            Shape::Octave => &[12],
            Shape::Fifth => &[7],
            Shape::Major => &[4, 7],
            Shape::Minor => &[3, 7],
            Shape::Seventh => &[4, 7, 10],
            Shape::Minor7 => &[3, 7, 10],
            Shape::Major7 => &[4, 7, 11],
            Shape::Sus2 => &[2, 7],
            Shape::Sus4 => &[5, 7],
            Shape::Add9 => &[4, 7, 14],
            Shape::Dim => &[3, 6],
            Shape::Aug => &[4, 8],
            Shape::InKey(_) => &[],
            Shape::Custom(mask) => {
                let mut n = 0;
                for i in 0..24 {
                    if mask & (1 << i) != 0 && n < MAX_MAP - 1 {
                        out[n] = i as i16 + 1;
                        n += 1;
                    }
                }
                return n;
            }
        };
        let n = list.len().min(MAX_MAP - 1);
        out[..n].copy_from_slice(&list[..n]);
        n
    }
}

/// How a chord's notes are spread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Voicing {
    /// As stacked.
    #[default]
    Close,
    /// Every second note up an octave.
    Spread,
    /// The second note from the top down an octave.
    Drop2,
}

impl Voicing {
    pub const ALL: [Voicing; 3] = [Voicing::Close, Voicing::Spread, Voicing::Drop2];

    pub fn label(self) -> &'static str {
        match self {
            Voicing::Close => "Close",
            Voicing::Spread => "Spread",
            Voicing::Drop2 => "Drop 2",
        }
    }
}

/// Chord: each note played becomes a chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub shape: Shape,
    pub voicing: Voicing,
    /// How many of the lowest notes go up an octave (0 to 3).
    pub inversion: u8,
    /// Notes above this go down an octave until they are at or below it;
    /// None keeps them where they are.
    pub keep_below: Option<u8>,
    /// The added notes' velocity, in percent of the note played.
    pub velocity: u8,
    /// Each note after the lowest starts this much after the one below.
    pub strum_ms: u16,
    /// The scale In key stacks its thirds in (C major when None).
    pub scale: Option<Scale>,
}

impl Default for Chord {
    fn default() -> Self {
        Chord {
            shape: Shape::Major,
            voicing: Voicing::Close,
            inversion: 0,
            keep_below: None,
            velocity: 100,
            strum_ms: 0,
            scale: None,
        }
    }
}

impl Chord {
    /// The chord for `played`, lowest first, with which one is the note
    /// played: (notes, how many, the note played's place). Notes outside
    /// 0 to 127 and doubled notes are left out.
    pub fn notes(&self, played: u8) -> ([u8; MAX_MAP], usize, Option<usize>) {
        let mut offs = [0i16; MAX_MAP];
        let mut n = 1;
        match self.shape {
            Shape::InKey(count) => {
                let scale = self.scale.unwrap_or(Scale::new(0, mpfree_scale::MAJOR));
                let root = scale.snap(i16::from(played));
                for k in 1..usize::from(count.clamp(2, 5)) {
                    offs[n] = scale.walk(root, 2 * k as i16) - i16::from(played);
                    n += 1;
                }
            }
            shape => {
                let mut ints = [0i16; MAX_MAP];
                let m = shape.intervals(&mut ints);
                offs[1..=m].copy_from_slice(&ints[..m]);
                n += m;
            }
        }
        // The note played is offs[0]; follow it through the voicing.
        let mut played_mark = [false; MAX_MAP];
        played_mark[0] = true;
        sort_with(&mut offs[..n], &mut played_mark[..n]);
        match self.voicing {
            Voicing::Close => {}
            Voicing::Spread => {
                for i in (1..n).step_by(2) {
                    offs[i] += 12;
                }
            }
            Voicing::Drop2 => {
                if n >= 3 {
                    offs[n - 2] -= 12;
                }
            }
        }
        sort_with(&mut offs[..n], &mut played_mark[..n]);
        for _ in 0..usize::from(self.inversion).min(n.saturating_sub(1)) {
            offs[0] += 12;
            sort_with(&mut offs[..n], &mut played_mark[..n]);
        }
        let mut out = [0u8; MAX_MAP];
        let mut len = 0;
        let mut at = None;
        let mut notes = [(0i16, false); MAX_MAP];
        for i in 0..n {
            let mut note = i16::from(played) + offs[i];
            if let Some(k) = self.keep_below {
                while note > i16::from(k) && note - 12 >= 0 {
                    note -= 12;
                }
            }
            notes[i] = (note, played_mark[i]);
        }
        notes[..n].sort_by_key(|(note, _)| *note);
        for &(note, is_played) in &notes[..n] {
            if !(0..=127).contains(&note) || out[..len].contains(&(note as u8)) {
                continue;
            }
            if is_played {
                at = Some(len);
            }
            out[len] = note as u8;
            len += 1;
        }
        (out, len, at)
    }
}

/// Sorts `offs` lowest first, keeping `mark` beside its note.
fn sort_with(offs: &mut [i16], mark: &mut [bool]) {
    for i in 1..offs.len() {
        let mut j = i;
        while j > 0 && offs[j - 1] > offs[j] {
            offs.swap(j - 1, j);
            mark.swap(j - 1, j);
            j -= 1;
        }
    }
}

/// Velocity: a fixed velocity, or the one played on a curve between a
/// lowest and a highest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Velocity {
    pub fixed: Option<u8>,
    /// -100 (soft notes softer) to 100 (soft notes louder); 0 is straight.
    pub curve: i8,
    pub min: u8,
    pub max: u8,
}

impl Default for Velocity {
    fn default() -> Self {
        Velocity {
            fixed: None,
            curve: 0,
            min: 1,
            max: 127,
        }
    }
}

impl Velocity {
    pub fn apply(&self, v: u8) -> u8 {
        if let Some(f) = self.fixed {
            return f.clamp(1, 127);
        }
        let (lo, hi) = (self.min.clamp(1, 127), self.max.clamp(1, 127));
        let (lo, hi) = (lo.min(hi), lo.max(hi));
        let x = f64::from(v.min(127)) / 127.0;
        let e = 2f64.powf(-f64::from(self.curve.clamp(-100, 100)) / 50.0);
        let y = x.powf(e);
        let out = f64::from(lo) + (f64::from(hi) - f64::from(lo)) * y;
        (out.round() as u8).clamp(lo, hi)
    }
}

/// Echo: each note played again after it, softer each time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Echo {
    pub time: Time,
    /// How many times it comes back (1 to 16).
    pub repeats: u8,
    /// Each repeat's velocity, in percent of the one before (1 to 100).
    pub feedback: u8,
    /// Semitones each repeat moves (-12 to 12).
    pub pitch: i8,
}

impl Default for Echo {
    fn default() -> Self {
        Echo {
            time: Time::Ticks(480),
            repeats: 3,
            feedback: 70,
            pitch: 0,
        }
    }
}

/// Which note of a strummed chord goes first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StrumOrder {
    /// Lowest first, as a guitar strummed down.
    #[default]
    Up,
    /// Highest first.
    Down,
    /// In the order they were played.
    Played,
}

impl StrumOrder {
    pub const ALL: [StrumOrder; 3] = [StrumOrder::Up, StrumOrder::Down, StrumOrder::Played];

    pub fn label(self) -> &'static str {
        match self {
            StrumOrder::Up => "Up",
            StrumOrder::Down => "Down",
            StrumOrder::Played => "As played",
        }
    }
}

/// Strum: notes that start together (within [`STRUM_TOGETHER_MS`] of the
/// first) start one after another instead, `time_ms` apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strum {
    /// Between one note and the next (1 to 200 ms).
    pub time_ms: u16,
    pub order: StrumOrder,
}

impl Default for Strum {
    fn default() -> Self {
        Strum {
            time_ms: 30,
            order: StrumOrder::Up,
        }
    }
}

/// Notes that start within this long of the first of them are one chord,
/// for [`Strum`].
pub const STRUM_TOGETHER_MS: u64 = 40;

/// The most notes one strum spreads; more start with the last.
pub const STRUM_MOST: usize = 16;

/// How Latch holds notes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LatchMode {
    /// The notes stay after the keys let go, until a key goes down with
    /// none held: then they end and the new ones play.
    #[default]
    NextChord,
    /// Each press turns its note on, and the next press of it off.
    Toggle,
}

impl LatchMode {
    pub const ALL: [LatchMode; 2] = [LatchMode::NextChord, LatchMode::Toggle];

    pub fn label(self) -> &'static str {
        match self {
            LatchMode::NextChord => "Next chord",
            LatchMode::Toggle => "Toggle",
        }
    }
}

/// Randomize: some notes move in pitch, every note's velocity a little,
/// and notes last a little longer, never the same twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Randomize {
    /// The most a note moves, either way (0 to 24 semitones).
    pub pitch: u8,
    /// How often a note moves, in percent.
    pub often: u8,
    /// The most a velocity moves, either way (0 to 64).
    pub velocity: u8,
    /// The most a note's end comes later (0 to 1000 ms).
    pub longer_ms: u16,
}

impl Default for Randomize {
    fn default() -> Self {
        Randomize {
            pitch: 5,
            often: 30,
            velocity: 0,
            longer_ms: 0,
        }
    }
}

/// The highest controller number the CC effects take: MIDI out sends CC
/// 0 to 119 for a track (120 to 127 are channel mode messages).
pub const CC_MOST: u8 = 119;

/// CC scale: a controller the track sends to MIDI out, fitted into
/// `low` to `high` (`low` above `high` turns it upside down). It leaves
/// the notes alone; MIDI out applies it to the values the track's lanes,
/// locks, LFOs, macros and learned knobs send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CcScale {
    /// The controller it takes (0 to [`CC_MOST`]), or every one.
    pub cc: Option<u8>,
    /// What 0 becomes.
    pub low: u8,
    /// What 127 becomes.
    pub high: u8,
}

impl Default for CcScale {
    fn default() -> Self {
        CcScale {
            cc: None,
            low: 20,
            high: 100,
        }
    }
}

impl CcScale {
    /// Whether it takes controller `cc`.
    pub fn takes(&self, cc: u8) -> bool {
        self.cc.is_none_or(|c| c == cc)
    }

    /// `value` (0 to 127) fitted into its range, to the nearest step.
    pub fn apply(&self, value: u8) -> u8 {
        let (lo, hi) = (i32::from(self.low), i32::from(self.high));
        let v = i32::from(value.min(127));
        let d = (hi - lo) * v;
        // Rounded half away from zero, either way up.
        let q = if d >= 0 {
            (d + 63) / 127
        } else {
            (d - 63) / 127
        };
        (lo + q).clamp(0, 127) as u8
    }
}

/// CC smooth: a controller the track sends to MIDI out glides to each new
/// value over `ms` instead of jumping, so a lane's steps sweep. A value
/// that comes with one of the track's notes jumps unless `on_note` says
/// glide, since some synths read the controller as the note starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CcSmooth {
    /// The controller it takes (0 to [`CC_MOST`]), or every one.
    pub cc: Option<u8>,
    /// How long a glide takes (1 to 2000 ms).
    pub ms: u16,
    /// A value that comes with a note glides too (else it jumps).
    pub glide_on_note: bool,
}

impl Default for CcSmooth {
    fn default() -> Self {
        CcSmooth {
            cc: None,
            ms: 100,
            glide_on_note: false,
        }
    }
}

impl CcSmooth {
    /// Whether it takes controller `cc`.
    pub fn takes(&self, cc: u8) -> bool {
        self.cc.is_none_or(|c| c == cc)
    }
}

/// "all CCs" or "CC 74".
fn cc_words(cc: Option<u8>) -> String {
    cc.map_or_else(|| "all CCs".to_string(), |c| format!("CC {c}"))
}

/// One MIDI effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fx {
    /// Every note moves by this many semitones (-48 to 48).
    Transpose {
        semitones: i8,
    },
    /// Notes outside the scale go into it (or aren't played, with
    /// [`Snap::Skip`]).
    Scale {
        scale: Scale,
        snap: Snap,
    },
    Chord(Chord),
    Velocity(Velocity),
    /// Every note lasts this long, however long it is held.
    Length {
        time: Time,
    },
    Echo(Echo),
    /// Each note a little late (up to `time_ms`) and its velocity a little
    /// off (up to `velocity` either way), never the same twice.
    Humanize {
        time_ms: u8,
        velocity: u8,
    },
    /// Each note plays with this chance, in percent.
    Chance {
        percent: u8,
    },
    Strum(Strum),
    /// Notes stay on after their keys let go.
    Latch {
        mode: LatchMode,
    },
    Randomize(Randomize),
    CcScale(CcScale),
    CcSmooth(CcSmooth),
}

impl Fx {
    /// The effects a track can add, with their settings at first.
    pub fn all() -> [Fx; 13] {
        [
            Fx::Scale {
                scale: Scale::new(0, mpfree_scale::MAJOR),
                snap: Snap::Down,
            },
            Fx::Chord(Chord::default()),
            Fx::Transpose { semitones: 12 },
            Fx::Velocity(Velocity::default()),
            Fx::Length {
                time: Time::Ticks(240),
            },
            Fx::Echo(Echo::default()),
            Fx::Humanize {
                time_ms: 6,
                velocity: 8,
            },
            Fx::Chance { percent: 75 },
            Fx::Strum(Strum::default()),
            Fx::Latch {
                mode: LatchMode::NextChord,
            },
            Fx::Randomize(Randomize::default()),
            Fx::CcScale(CcScale::default()),
            Fx::CcSmooth(CcSmooth::default()),
        ]
    }

    /// Whether it acts on the controllers the track sends to MIDI out
    /// (and so leaves the notes alone).
    pub fn is_cc(&self) -> bool {
        matches!(self, Fx::CcScale(_) | Fx::CcSmooth(_))
    }

    pub fn name(&self) -> &'static str {
        match self {
            Fx::Transpose { .. } => "Transpose",
            Fx::Scale { .. } => "Scale",
            Fx::Chord(_) => "Chord",
            Fx::Velocity(_) => "Velocity",
            Fx::Length { .. } => "Note length",
            Fx::Echo(_) => "Echo",
            Fx::Humanize { .. } => "Humanize",
            Fx::Chance { .. } => "Chance",
            Fx::Strum(_) => "Strum",
            Fx::Latch { .. } => "Latch",
            Fx::Randomize(_) => "Randomize",
            Fx::CcScale(_) => "CC scale",
            Fx::CcSmooth(_) => "CC smooth",
        }
    }

    /// One line on what it does now ("m7 · close", "C minor · down").
    pub fn summary(&self) -> String {
        match self {
            Fx::Transpose { semitones } => format!("{semitones:+} semitones"),
            Fx::Scale { scale, snap } => {
                let name = mpfree_scale::SCALES
                    .iter()
                    .find(|d| d.mask == scale.mask && d.down == scale.down)
                    .map_or("own scale", |d| d.name);
                format!(
                    "{} {name} · {}",
                    mpfree_scale::note_name(i16::from(scale.root) + 60)
                        .trim_end_matches(char::is_numeric),
                    snap.label().to_lowercase()
                )
            }
            Fx::Chord(c) => format!("{} · {}", c.shape.label(), c.voicing.label().to_lowercase()),
            Fx::Velocity(v) => match v.fixed {
                Some(f) => format!("fixed {f}"),
                None => format!("{} to {} · curve {:+}", v.min, v.max, v.curve),
            },
            Fx::Length { time } => time.label(),
            Fx::Echo(e) => format!("{} · {} repeats", e.time.label(), e.repeats),
            Fx::Humanize { time_ms, velocity } => {
                format!("up to {time_ms} ms late · ±{velocity} vel")
            }
            Fx::Chance { percent } => format!("{percent} %"),
            Fx::Strum(st) => format!("{} ms · {}", st.time_ms, st.order.label().to_lowercase()),
            Fx::Latch { mode } => mode.label().to_lowercase(),
            Fx::Randomize(r) => {
                let mut parts = Vec::new();
                if r.pitch > 0 && r.often > 0 {
                    parts.push(format!("±{} st on {} % of notes", r.pitch, r.often));
                }
                if r.velocity > 0 {
                    parts.push(format!("±{} vel", r.velocity));
                }
                if r.longer_ms > 0 {
                    parts.push(format!("up to {} ms longer", r.longer_ms));
                }
                if parts.is_empty() {
                    "nothing yet".to_string()
                } else {
                    parts.join(" · ")
                }
            }
            Fx::CcScale(c) => format!("{} · {} to {}", cc_words(c.cc), c.low, c.high),
            Fx::CcSmooth(c) => format!(
                "{} · {} ms{}",
                cc_words(c.cc),
                c.ms,
                if c.glide_on_note {
                    ""
                } else {
                    " · jumps on a note"
                }
            ),
        }
    }
}
