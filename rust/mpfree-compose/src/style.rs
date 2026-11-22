//! Styles made of parts (contract 11 of the design).
//!
//! Inside, a style is a mix of components: harmony, rhythm, bass, melody,
//! playing and development, plus its preference weights and drift. The
//! screen offers three named styles; because each is made of parts, a
//! later version can offer jazz harmony without the walking bass, or a
//! walking bass under a straight rhythm ([`Style::with`]).
//!
//! The numbers are the second prototype's, tuned by ear, with the review's
//! changes: drift is 0 in Minimal, and swing is a suggestion for the clip's
//! playback swing rather than something baked into the notes.

use mpfree_transport::time::{Swing, PPQ};

use crate::rules::Weights;

/// How the style reshapes a rhythm it keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Feel {
    /// Keep the strongest notes.
    Simplify,
    /// Push some beats a 1/16 early and chop long notes.
    Push,
    /// Notes on swung 1/8s, some beats anticipated.
    Swing8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HarmonyStyle {
    pub sevenths: bool,
    /// Weight of keeping the chord into the next slot.
    pub stay: f32,
    /// The fastest the chords may change, in 1/16 steps.
    pub window_steps: u8,
    /// Root motion weights by semitones up (0 = same root).
    pub motion: [f32; 12],
    /// Scale degrees (0 = I) chords may be built on.
    pub degrees: &'static [u8],
    /// A major V (or V7) in minor keys.
    pub borrowed_v: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RhythmStyle {
    pub feel: Feel,
    /// The playback swing the style suggests for a new clip, when no
    /// played groove sets the feel. Never baked into the notes.
    pub swing: Swing,
    /// Syncopation aim for new rhythms, 0 to 1.
    pub syncopation: f32,
    /// Notes per beat in new material.
    pub density: f32,
    /// Chance a beat is left silent in new material.
    pub rest: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BassLine {
    Offbeat,
    Sixteenths,
    Walking,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Comping {
    Pad,
    Stabs,
    Charleston,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DrumMap {
    FourOnTheFloor,
    BoomBap,
    Ride,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BassStyle {
    pub line: BassLine,
    pub comping: Comping,
    pub drums: DrumMap,
    /// Chord voicing register, lowest and highest note.
    pub voicing: (u8, u8),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MelodyStyle {
    /// Interval odds by size in scale steps, 0 to 7.
    pub intervals: [f32; 8],
    /// How often approach notes come in (0 never).
    pub chromatic: f32,
    /// Keep to this many pitch classes from what was played (0 any).
    pub pitch_set: u8,
    /// Cost of a note off the chord on strong and weak steps.
    pub off_chord: [f32; 2],
    /// Cost of colour tones (9ths, 13ths).
    pub colour: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayingStyle {
    /// Note length as a share of the gap to the next note.
    pub gate: f32,
    pub legato: bool,
    /// Velocity added per 1/16 position.
    pub accent: [i8; 16],
    pub velocity: u8,
    pub velocity_range: f32,
    /// Chance of a ghost note where one fits.
    pub ghosts: f32,
}

/// A plan shape: how tension and repetition run over a phrase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlanShape {
    /// A riff that loops, with small changes.
    Loop,
    /// Call and answer, two bars each.
    CallAndAnswer,
    /// Statement, repeat, development, close.
    Sentence,
    /// One rise to a peak about three quarters through.
    OneClimax,
    /// Several small rises.
    Waves,
    /// A slow build to the end.
    SlowBuild,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DevelopmentStyle {
    /// The plan shapes it picks from, by the seed.
    pub plans: &'static [PlanShape],
    /// Small changes per repeat (ornaments, a moved note).
    pub variation: f32,
}

/// A style: its components, its preference weights and its drift.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub name: &'static str,
    pub harmony: HarmonyStyle,
    pub rhythm: RhythmStyle,
    pub bass: BassStyle,
    pub melody: MelodyStyle,
    pub playing: PlayingStyle,
    pub development: DevelopmentStyle,
    pub weights: Weights,
    /// A slow drift shared by all parts, in ms (0: none). Off whenever a
    /// played groove has been read.
    pub drift_ms: f32,
}

/// A component of a style, for [`Style::with`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Component {
    Harmony(HarmonyStyle),
    Rhythm(RhythmStyle),
    Bass(BassStyle),
    Melody(MelodyStyle),
    Playing(PlayingStyle),
    Development(DevelopmentStyle),
}

impl Style {
    /// This style with one component taken from elsewhere.
    pub fn with(mut self, c: Component) -> Style {
        match c {
            Component::Harmony(h) => self.harmony = h,
            Component::Rhythm(r) => self.rhythm = r,
            Component::Bass(b) => self.bass = b,
            Component::Melody(m) => self.melody = m,
            Component::Playing(p) => self.playing = p,
            Component::Development(d) => self.development = d,
        }
        self
    }
}

const EIGHTH: u32 = PPQ / 2;
const SIXTEENTH: u32 = PPQ / 4;

const ENDINGS: (f32, f32, f32, f32) = (2.0, 3.0, 1.0, 2.0);

pub const MINIMAL: Style = Style {
    name: "Minimal",
    harmony: HarmonyStyle {
        sevenths: false,
        stay: 3.0,
        window_steps: 32,
        // Mostly stays; moves by a step or a third (i, bVI, bVII).
        motion: [0.0, 0.1, 0.6, 0.4, 0.5, 0.3, 0.05, 0.3, 0.6, 0.4, 0.8, 0.1],
        degrees: &[0, 3, 5, 6],
        borrowed_v: false,
    },
    rhythm: RhythmStyle {
        feel: Feel::Simplify,
        swing: Swing {
            grid: SIXTEENTH,
            amount: 52,
        },
        syncopation: 0.35,
        density: 1.0,
        rest: 0.25,
    },
    bass: BassStyle {
        line: BassLine::Offbeat,
        comping: Comping::Pad,
        drums: DrumMap::FourOnTheFloor,
        voicing: (55, 72),
    },
    melody: MelodyStyle {
        intervals: [1.6, 1.0, 0.6, 0.25, 0.25, 0.05, 0.03, 0.35],
        chromatic: 0.0,
        pitch_set: 4,
        off_chord: [1.6, 0.5],
        colour: 0.9,
    },
    playing: PlayingStyle {
        gate: 0.45,
        legato: false,
        accent: [10, -4, 4, -4, 6, -4, 4, -4, 8, -4, 4, -4, 6, -4, 4, -4],
        velocity: 92,
        velocity_range: 10.0,
        ghosts: 0.0,
    },
    development: DevelopmentStyle {
        plans: &[PlanShape::Loop, PlanShape::Waves, PlanShape::SlowBuild],
        variation: 0.15,
    },
    weights: Weights {
        leap: 0.6,
        chromatic: 4.0,
        ending_tune: ENDINGS.0,
        ending_chord: ENDINGS.1,
        ending_rhythm: ENDINGS.2,
        loop_return: ENDINGS.3,
        repeats: 0.0,
    },
    drift_ms: 0.0,
};

pub const GROOVE: Style = Style {
    name: "Groove",
    harmony: HarmonyStyle {
        sevenths: true,
        stay: 1.3,
        window_steps: 16,
        // Vamps: i to iv, i to bVII, and fourths.
        motion: [
            0.0, 0.05, 0.4, 0.5, 0.2, 1.0, 0.05, 0.6, 0.4, 0.3, 0.9, 0.05,
        ],
        degrees: &[0, 2, 3, 4, 5, 6],
        borrowed_v: false,
    },
    rhythm: RhythmStyle {
        feel: Feel::Push,
        swing: Swing {
            grid: SIXTEENTH,
            amount: 57,
        },
        syncopation: 0.6,
        density: 1.6,
        rest: 0.15,
    },
    bass: BassStyle {
        line: BassLine::Sixteenths,
        comping: Comping::Stabs,
        drums: DrumMap::BoomBap,
        voicing: (58, 76),
    },
    melody: MelodyStyle {
        intervals: [1.2, 0.9, 0.7, 0.35, 0.45, 0.1, 0.05, 0.5],
        chromatic: 0.15,
        pitch_set: 0,
        off_chord: [1.3, 0.35],
        colour: 0.45,
    },
    playing: PlayingStyle {
        gate: 0.6,
        legato: false,
        accent: [10, -8, 4, -6, 8, -8, 6, -4, 9, -8, 4, -6, 8, -8, 6, -4],
        velocity: 96,
        velocity_range: 14.0,
        ghosts: 0.35,
    },
    development: DevelopmentStyle {
        plans: &[
            PlanShape::CallAndAnswer,
            PlanShape::Waves,
            PlanShape::OneClimax,
        ],
        variation: 0.3,
    },
    weights: Weights {
        leap: 0.4,
        chromatic: 1.5,
        ending_tune: ENDINGS.0,
        ending_chord: ENDINGS.1,
        ending_rhythm: ENDINGS.2,
        loop_return: ENDINGS.3,
        repeats: 0.25,
    },
    drift_ms: 6.0,
};

pub const JAZZISH: Style = Style {
    name: "Jazz-ish",
    harmony: HarmonyStyle {
        sevenths: true,
        stay: 0.7,
        window_steps: 8,
        // Down a fifth above all: ii, V, I.
        motion: [
            0.0, 0.25, 0.3, 0.25, 0.15, 1.6, 0.35, 0.3, 0.2, 0.3, 0.3, 0.25,
        ],
        degrees: &[0, 1, 2, 3, 4, 5, 6],
        borrowed_v: true,
    },
    rhythm: RhythmStyle {
        feel: Feel::Swing8,
        swing: Swing {
            grid: EIGHTH,
            amount: 64,
        },
        syncopation: 0.45,
        density: 1.8,
        rest: 0.12,
    },
    bass: BassStyle {
        line: BassLine::Walking,
        comping: Comping::Charleston,
        drums: DrumMap::Ride,
        voicing: (52, 72),
    },
    melody: MelodyStyle {
        intervals: [0.35, 1.3, 0.9, 0.4, 0.3, 0.25, 0.1, 0.1],
        chromatic: 0.5,
        pitch_set: 0,
        off_chord: [1.0, 0.25],
        colour: 0.15,
    },
    playing: PlayingStyle {
        gate: 0.95,
        legato: true,
        accent: [4, -2, 9, -2, 3, -2, 10, -2, 4, -2, 9, -2, 3, -2, 10, -2],
        velocity: 88,
        velocity_range: 16.0,
        ghosts: 0.2,
    },
    development: DevelopmentStyle {
        plans: &[
            PlanShape::Sentence,
            PlanShape::OneClimax,
            PlanShape::SlowBuild,
        ],
        variation: 0.45,
    },
    weights: Weights {
        leap: 0.3,
        chromatic: 0.3,
        ending_tune: 1.5,
        ending_chord: ENDINGS.1,
        ending_rhythm: ENDINGS.2,
        loop_return: ENDINGS.3,
        repeats: 0.6,
    },
    drift_ms: 6.0,
};

/// The styles on screen, in order.
pub const STYLES: [Style; 3] = [MINIMAL, GROOVE, JAZZISH];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_styles_that_differ_in_every_component() {
        for (i, a) in STYLES.iter().enumerate() {
            for b in &STYLES[i + 1..] {
                assert_ne!(a.harmony, b.harmony);
                assert_ne!(a.rhythm, b.rhythm);
                assert_ne!(a.bass, b.bass);
                assert_ne!(a.melody, b.melody);
                assert_ne!(a.playing, b.playing);
                assert_ne!(a.development, b.development);
            }
            assert!(
                Swing::new(a.rhythm.swing.grid, a.rhythm.swing.amount).is_some(),
                "{}",
                a.name
            );
            assert!(a.development.plans.len() >= 3, "a few plan shapes each");
        }
    }

    #[test]
    fn minimal_has_no_drift() {
        let drifts: Vec<f32> = STYLES.iter().map(|s| s.drift_ms).collect();
        assert_eq!(drifts, vec![0.0, 6.0, 6.0]);
    }

    #[test]
    fn jazz_harmony_without_the_walking_bass() {
        let s = GROOVE.with(Component::Harmony(JAZZISH.harmony));
        assert!(s.harmony.borrowed_v);
        assert_eq!(s.bass.line, BassLine::Sixteenths);
        assert_eq!(s.rhythm, GROOVE.rhythm);
    }
}
