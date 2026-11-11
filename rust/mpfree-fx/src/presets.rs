//! Factory presets for every effect that has a sound to pick, in one place:
//! [`presets`] by node id. A preset holds the values that differ from the
//! node's defaults; the effect page keeps the Mix and Quality the effect
//! has (a preset sets the sound, not how much of it is heard or what it
//! costs). The DJ filter (one knob), Master safety (a safety stage) and
//! Perform FX (its own pad banks) have none.

use crate::ids::{
    chorus, comb, compressor as comp, crush, delay, drive, eq, filter, flanger, phaser,
};
use crate::Preset;

/// An effect's factory presets by its node id (empty for one without).
pub fn presets(id: &str) -> &'static [Preset] {
    match id {
        "mpfree.fx.eq" => EQ,
        "mpfree.fx.filter" => FILTER,
        "mpfree.fx.drive" => DRIVE,
        "mpfree.fx.comb" => COMB,
        "mpfree.fx.crush" => CRUSH,
        "mpfree.fx.delay" => DELAY,
        "mpfree.fx.chorus" => CHORUS,
        "mpfree.fx.flanger" => FLANGER,
        "mpfree.fx.phaser" => PHASER,
        "mpfree.fx.compressor" => COMPRESSOR,
        "mpfree.fx.lofi" => crate::lofi::PRESETS,
        "mpfree.fx.cloud" => crate::cloud::PRESETS,
        _ => &[],
    }
}

// EQ band types and cut slopes (band 1 is a low shelf, band 4 a high
// shelf and the others bells by default).
const BELL: f32 = 0.0;
const S12: f32 = 1.0;
const S24: f32 = 2.0;

/// EQ: starting points for the jobs a beat needs most.
pub static EQ: &[Preset] = &[
    Preset {
        name: "Kick weight",
        values: &[
            (eq::LOW_CUT, 30.0),
            (eq::LOW_SLOPE, S24),
            (eq::freq(0), 60.0),
            (eq::gain(0), 4.0),
            (eq::freq(1), 350.0),
            (eq::gain(1), -4.0),
            (eq::q(1), 1.2),
            (eq::freq(2), 3500.0),
            (eq::gain(2), 3.0),
            (eq::q(2), 1.4),
        ],
    },
    Preset {
        name: "Snare crack",
        values: &[
            (eq::LOW_CUT, 120.0),
            (eq::LOW_SLOPE, S12),
            (eq::band_type(0), BELL),
            (eq::freq(0), 200.0),
            (eq::gain(0), 3.0),
            (eq::q(0), 1.0),
            (eq::freq(1), 800.0),
            (eq::gain(1), -3.0),
            (eq::q(1), 1.5),
            (eq::freq(2), 5000.0),
            (eq::gain(2), 4.0),
            (eq::q(2), 1.0),
            (eq::freq(3), 10_000.0),
            (eq::gain(3), 2.0),
        ],
    },
    Preset {
        name: "Vocal clarity",
        values: &[
            (eq::LOW_CUT, 100.0),
            (eq::LOW_SLOPE, S24),
            (eq::freq(0), 200.0),
            (eq::gain(0), -2.0),
            (eq::freq(1), 300.0),
            (eq::gain(1), -3.0),
            (eq::q(1), 1.0),
            (eq::freq(2), 3000.0),
            (eq::gain(2), 3.0),
            (eq::q(2), 0.8),
            (eq::freq(3), 12_000.0),
            (eq::gain(3), 3.0),
        ],
    },
    Preset {
        name: "Thin for layering",
        values: &[
            (eq::LOW_CUT, 250.0),
            (eq::LOW_SLOPE, S24),
            (eq::freq(1), 500.0),
            (eq::gain(1), -2.0),
            (eq::q(1), 0.9),
            (eq::gain(3), -3.0),
        ],
    },
    Preset {
        name: "Warm and dark",
        values: &[
            (eq::HIGH_SLOPE, S12),
            (eq::freq(0), 120.0),
            (eq::gain(0), 2.0),
            (eq::freq(3), 6000.0),
            (eq::gain(3), -6.0),
        ],
    },
    Preset {
        name: "Telephone",
        values: &[
            (eq::LOW_CUT, 400.0),
            (eq::LOW_SLOPE, S24),
            (eq::HIGH_CUT, 3500.0),
            (eq::HIGH_SLOPE, S24),
            (eq::freq(1), 1500.0),
            (eq::gain(1), 6.0),
            (eq::q(1), 0.7),
            (eq::OUTPUT, -2.0),
        ],
    },
];

// Multi filter types and slopes.
const LOW_PASS: f32 = 0.0;
const HIGH_PASS: f32 = 1.0;
const BAND_PASS: f32 = 2.0;
const NOTCH: f32 = 3.0;
const F_LOW_SHELF: f32 = 5.0;
const F24: f32 = 1.0;

/// Multi filter: the shapes people reach for on a loop or a bass.
pub static FILTER: &[Preset] = &[
    Preset {
        name: "Warm low-pass",
        values: &[
            (filter::TYPE, LOW_PASS),
            (filter::SLOPE, F24),
            (filter::CUTOFF, 2500.0),
            (filter::RESONANCE, 20.0),
        ],
    },
    Preset {
        name: "Acid squelch",
        values: &[
            (filter::TYPE, LOW_PASS),
            (filter::SLOPE, F24),
            (filter::CUTOFF, 900.0),
            (filter::RESONANCE, 80.0),
            (filter::DRIVE, 9.0),
        ],
    },
    Preset {
        name: "Thin high-pass",
        values: &[
            (filter::TYPE, HIGH_PASS),
            (filter::CUTOFF, 300.0),
            (filter::RESONANCE, 10.0),
        ],
    },
    Preset {
        name: "Radio band",
        values: &[
            (filter::TYPE, BAND_PASS),
            (filter::CUTOFF, 1500.0),
            (filter::RESONANCE, 30.0),
            (filter::DRIVE, 6.0),
        ],
    },
    Preset {
        name: "Hollow notch",
        values: &[
            (filter::TYPE, NOTCH),
            (filter::SLOPE, F24),
            (filter::CUTOFF, 1000.0),
            (filter::RESONANCE, 50.0),
        ],
    },
    Preset {
        name: "Sub lift",
        values: &[
            (filter::TYPE, F_LOW_SHELF),
            (filter::CUTOFF, 100.0),
            (filter::GAIN, 6.0),
        ],
    },
];

// Drive modes.
const SOFT: f32 = 0.0;
const HARD: f32 = 1.0;
const TUBE: f32 = 2.0;
const FOLDBACK: f32 = 3.0;
const FUZZ: f32 = 4.0;
const RECTIFY: f32 = 5.0;

/// Drive: from a little warmth to broken. Auto-gain stays on, so a preset
/// changes the grit, not the level.
pub static DRIVE: &[Preset] = &[
    Preset {
        name: "Warm tape",
        values: &[
            (drive::MODE, SOFT),
            (drive::DRIVE, 6.0),
            (drive::TONE, 12_000.0),
            (drive::BIAS, 10.0),
        ],
    },
    Preset {
        name: "Tube glow",
        values: &[
            (drive::MODE, TUBE),
            (drive::DRIVE, 12.0),
            (drive::TONE, 9000.0),
            (drive::BIAS, 40.0),
        ],
    },
    Preset {
        name: "Hard clip",
        values: &[
            (drive::MODE, HARD),
            (drive::DRIVE, 18.0),
            (drive::TONE, 16_000.0),
            (drive::BIAS, 0.0),
        ],
    },
    Preset {
        name: "Fuzz box",
        values: &[
            (drive::MODE, FUZZ),
            (drive::DRIVE, 30.0),
            (drive::TONE, 4500.0),
            (drive::BIAS, 30.0),
        ],
    },
    Preset {
        name: "Foldback grit",
        values: &[
            (drive::MODE, FOLDBACK),
            (drive::DRIVE, 20.0),
            (drive::TONE, 8000.0),
            (drive::BIAS, 0.0),
        ],
    },
    Preset {
        name: "Octave up",
        values: &[
            (drive::MODE, RECTIFY),
            (drive::DRIVE, 6.0),
            (drive::TONE, 7000.0),
            (drive::BIAS, 0.0),
        ],
    },
];

/// Comb: rings, strings and robots. Key follow on means a note on the
/// effect's track sets the pitch.
pub static COMB: &[Preset] = &[
    Preset {
        name: "Metal ring",
        values: &[
            (comb::PITCH, 220.0),
            (comb::FEEDBACK, 92.0),
            (comb::DAMPING, 12_000.0),
            (comb::SPREAD, 8.0),
        ],
    },
    Preset {
        name: "Plucked string",
        values: &[
            (comb::PITCH, 220.0),
            (comb::FEEDBACK, 97.0),
            (comb::DAMPING, 4000.0),
            (comb::SPREAD, 3.0),
        ],
    },
    Preset {
        name: "Hollow tube",
        values: &[
            (comb::PITCH, 110.0),
            (comb::FEEDBACK, -80.0),
            (comb::DAMPING, 6000.0),
            (comb::SPREAD, 0.0),
        ],
    },
    Preset {
        name: "Robot voice",
        values: &[
            (comb::PITCH, 150.0),
            (comb::FEEDBACK, 70.0),
            (comb::DAMPING, 9000.0),
            (comb::SPREAD, 0.0),
            (comb::KEY_FOLLOW, 0.0),
        ],
    },
    Preset {
        name: "Wide shimmer",
        values: &[
            (comb::PITCH, 880.0),
            (comb::FEEDBACK, 85.0),
            (comb::DAMPING, 15_000.0),
            (comb::SPREAD, 25.0),
        ],
    },
];

/// Crush: classic sampler grit down to nothing left.
pub static CRUSH: &[Preset] = &[
    Preset {
        name: "12-bit classic",
        values: &[(crush::BITS, 12.0), (crush::RATE, 26_040.0)],
    },
    Preset {
        name: "Soft grit",
        values: &[
            (crush::BITS, 10.0),
            (crush::RATE, 32_000.0),
            (crush::DITHER, 1.0),
        ],
    },
    Preset {
        name: "Chip",
        values: &[(crush::BITS, 6.0), (crush::RATE, 22_050.0)],
    },
    Preset {
        name: "Phone line",
        values: &[
            (crush::BITS, 8.0),
            (crush::RATE, 8000.0),
            (crush::DITHER, 1.0),
        ],
    },
    Preset {
        name: "Destroyed",
        values: &[(crush::BITS, 3.0), (crush::RATE, 2000.0)],
    },
];

// Delay sync choices.
const D_FREE: f32 = 0.0;
const D_8T: f32 = 5.0;
const D_8: f32 = 6.0;
const D_8D: f32 = 7.0;
const D_4: f32 = 9.0;
const D_4D: f32 = 10.0;

/// Delay: synced repeats for beats, and a slapback.
pub static DELAY: &[Preset] = &[
    Preset {
        name: "Slapback",
        values: &[
            (delay::SYNC, D_FREE),
            (delay::TIME, 110.0),
            (delay::FEEDBACK, 0.0),
            (delay::TONE, 6000.0),
            (delay::SATURATION, 30.0),
        ],
    },
    Preset {
        name: "Dotted ping-pong",
        values: &[
            (delay::SYNC, D_8D),
            (delay::FEEDBACK, 45.0),
            (delay::PING_PONG, 1.0),
        ],
    },
    Preset {
        name: "Triplet bounce",
        values: &[
            (delay::SYNC, D_8T),
            (delay::FEEDBACK, 35.0),
            (delay::PING_PONG, 1.0),
            (delay::TONE, 8000.0),
        ],
    },
    Preset {
        name: "Quarter repeats",
        values: &[
            (delay::SYNC, D_4),
            (delay::FEEDBACK, 30.0),
            (delay::TONE, 9000.0),
            (delay::SATURATION, 10.0),
        ],
    },
    Preset {
        name: "Dub echo",
        values: &[
            (delay::SYNC, D_4D),
            (delay::FEEDBACK, 70.0),
            (delay::TONE, 2500.0),
            (delay::SATURATION, 50.0),
        ],
    },
    Preset {
        name: "Tape runaway",
        values: &[
            (delay::SYNC, D_8),
            (delay::FEEDBACK, 95.0),
            (delay::TONE, 3500.0),
            (delay::SATURATION, 70.0),
        ],
    },
];

// LFO sync choices (chorus, flanger, phaser).
const L_4: f32 = 3.0;
const L_BAR: f32 = 5.0;
const L_2BARS: f32 = 6.0;

/// Chorus: from a touch of width to a big ensemble.
pub static CHORUS: &[Preset] = &[
    Preset {
        name: "Subtle widen",
        values: &[
            (chorus::RATE, 0.25),
            (chorus::DEPTH, 15.0),
            (chorus::DELAY, 8.0),
            (chorus::VOICES, 2.0),
            (chorus::TONE, 12_000.0),
        ],
    },
    Preset {
        name: "Lush ensemble",
        values: &[
            (chorus::RATE, 0.35),
            (chorus::DEPTH, 60.0),
            (chorus::DELAY, 18.0),
            (chorus::VOICES, 4.0),
            (chorus::TONE, 8000.0),
        ],
    },
    Preset {
        name: "Dimension",
        values: &[
            (chorus::RATE, 0.8),
            (chorus::DEPTH, 25.0),
            (chorus::DELAY, 6.0),
            (chorus::VOICES, 2.0),
        ],
    },
    Preset {
        name: "Fast wobble",
        values: &[
            (chorus::RATE, 5.0),
            (chorus::DEPTH, 50.0),
            (chorus::DELAY, 4.0),
            (chorus::VOICES, 1.0),
            (chorus::WIDTH, 0.0),
            (chorus::TONE, 12_000.0),
        ],
    },
    Preset {
        name: "Bar swirl",
        values: &[
            (chorus::SYNC, L_BAR),
            (chorus::DEPTH, 55.0),
            (chorus::DELAY, 14.0),
            (chorus::VOICES, 3.0),
        ],
    },
];

/// Flanger: jets, metal and slow synced sweeps.
pub static FLANGER: &[Preset] = &[
    Preset {
        name: "Gentle flange",
        values: &[
            (flanger::RATE, 0.2),
            (flanger::DEPTH, 50.0),
            (flanger::DELAY, 1.5),
            (flanger::FEEDBACK, 20.0),
        ],
    },
    Preset {
        name: "Jet sweep",
        values: &[
            (flanger::RATE, 0.1),
            (flanger::DEPTH, 80.0),
            (flanger::DELAY, 2.5),
            (flanger::FEEDBACK, 75.0),
        ],
    },
    Preset {
        name: "Metallic",
        values: &[
            (flanger::RATE, 0.3),
            (flanger::DEPTH, 40.0),
            (flanger::DELAY, 0.8),
            (flanger::FEEDBACK, 80.0),
        ],
    },
    Preset {
        name: "Hollow flange",
        values: &[
            (flanger::RATE, 0.25),
            (flanger::DEPTH, 70.0),
            (flanger::DELAY, 1.2),
            (flanger::FEEDBACK, -70.0),
        ],
    },
    Preset {
        name: "Two-bar sweep",
        values: &[
            (flanger::SYNC, L_2BARS),
            (flanger::DEPTH, 75.0),
            (flanger::DELAY, 2.0),
            (flanger::FEEDBACK, 60.0),
        ],
    },
];

// Phaser stage choices (2, 4, 6, 8, 12).
const P4: f32 = 1.0;
const P6: f32 = 2.0;
const P8: f32 = 3.0;
const P12: f32 = 4.0;

/// Phaser: slow swirls, deep sweeps and a quick funk wobble.
pub static PHASER: &[Preset] = &[
    Preset {
        name: "Slow swirl",
        values: &[
            (phaser::RATE, 0.15),
            (phaser::DEPTH, 80.0),
            (phaser::CENTER, 800.0),
            (phaser::STAGES, P4),
            (phaser::FEEDBACK, 20.0),
        ],
    },
    Preset {
        name: "Deep phase",
        values: &[
            (phaser::RATE, 0.25),
            (phaser::DEPTH, 90.0),
            (phaser::CENTER, 600.0),
            (phaser::STAGES, P8),
            (phaser::FEEDBACK, 60.0),
        ],
    },
    Preset {
        name: "Funk wobble",
        values: &[
            (phaser::RATE, 2.5),
            (phaser::DEPTH, 60.0),
            (phaser::CENTER, 1200.0),
            (phaser::STAGES, P4),
            (phaser::FEEDBACK, 40.0),
        ],
    },
    Preset {
        name: "Liquid",
        values: &[
            (phaser::RATE, 0.1),
            (phaser::DEPTH, 85.0),
            (phaser::CENTER, 500.0),
            (phaser::STAGES, P12),
            (phaser::FEEDBACK, 70.0),
        ],
    },
    Preset {
        name: "Quarter pulse",
        values: &[
            (phaser::SYNC, L_4),
            (phaser::DEPTH, 70.0),
            (phaser::CENTER, 900.0),
            (phaser::STAGES, P6),
            (phaser::FEEDBACK, 50.0),
        ],
    },
];

/// Compressor: glue, punch, level and smash. Mix stays where it is, so
/// "Smash" under a lower Mix is parallel compression.
pub static COMPRESSOR: &[Preset] = &[
    Preset {
        name: "Gentle glue",
        values: &[
            (comp::THRESHOLD, -14.0),
            (comp::RATIO, 2.0),
            (comp::ATTACK, 30.0),
            (comp::RELEASE, 300.0),
            (comp::KNEE, 10.0),
            (comp::MAKEUP, 2.0),
        ],
    },
    Preset {
        name: "Drum punch",
        values: &[
            (comp::THRESHOLD, -20.0),
            (comp::RATIO, 4.0),
            (comp::ATTACK, 20.0),
            (comp::RELEASE, 100.0),
            (comp::KNEE, 4.0),
            (comp::MAKEUP, 4.0),
        ],
    },
    Preset {
        name: "Vocal level",
        values: &[
            (comp::THRESHOLD, -22.0),
            (comp::RATIO, 3.5),
            (comp::ATTACK, 5.0),
            (comp::RELEASE, 120.0),
            (comp::KNEE, 8.0),
            (comp::MAKEUP, 5.0),
        ],
    },
    Preset {
        name: "Smash",
        values: &[
            (comp::THRESHOLD, -35.0),
            (comp::RATIO, 12.0),
            (comp::ATTACK, 0.5),
            (comp::RELEASE, 60.0),
            (comp::KNEE, 2.0),
            (comp::MAKEUP, 12.0),
        ],
    },
    Preset {
        name: "Sidechain pump",
        values: &[
            (comp::THRESHOLD, -30.0),
            (comp::RATIO, 10.0),
            (comp::ATTACK, 1.0),
            (comp::RELEASE, 180.0),
            (comp::KNEE, 0.0),
            (comp::SIDECHAIN, 1.0),
        ],
    },
];
