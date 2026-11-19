//! The choir's parameters and its vowel table; its factory presets are in
//! `presets/choir.presets` ([`crate::factory::read`]).

use mpfree_node::{Curve, ParamInfo, Unit};

macro_rules! ids {
    ($($name:ident = $v:expr),* $(,)?) => { $(pub const $name: u16 = $v;)* };
}

ids! {
    VOWEL = 0, SECTION = 1, SINGERS = 2, DETUNE = 3, BREATH = 4, BRIGHTNESS = 5, OCTAVE = 6,
    START = 10, START_TIME = 11, MORPH = 12, MORPH_RATE = 13,
    ATTACK = 20, DECAY = 21, SUSTAIN = 22, RELEASE = 23, VELOCITY = 24, VEL_TONE = 25,
    VIBRATO = 30, VIB_RATE = 31, VIB_DELAY = 32, DRIFT = 33,
    ENSEMBLE = 40, LINES = 41, SLOW_RATE = 42, FAST_RATE = 43, DEPTH = 44, WIDTH = 45,
    VOICES = 50, MODE = 51, GLIDE = 52, TRANSPOSE = 53, FINE = 54, BEND_RANGE = 55,
    PRESS_SWELL = 60, PRESS_VOWEL = 61,
    LEVEL = 70,
    METER_NOTES = 80,
}

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = 81;

/// The sections, low to high, then Mixed: each note sung by the section
/// whose range it sits in.
pub const SECTIONS: &[&str] = &["Bass", "Tenor", "Countertenor", "Alto", "Soprano", "Mixed"];
pub const BASS: f32 = 0.0;
pub const TENOR: f32 = 1.0;
pub const COUNTERTENOR: f32 = 2.0;
pub const ALTO: f32 = 3.0;
pub const SOPRANO: f32 = 4.0;
pub const MIXED: f32 = 5.0;

/// The vowels along the Vowel control, back of the mouth to front, so a
/// sweep moves the second formant one way: "oo" (as in boot), "oh", "ah"
/// (father), "eh" (bed) and "ee" (beet), at 0, 25, 50, 75 and 100 %.
pub const VOWELS: [&str; 5] = ["Ooh", "Oh", "Aah", "Eh", "Ee"];

/// How the notes are played: every note its own voice, or one voice
/// (Mono starts the swell again on each key, Legato only on a key after
/// silence).
pub const MODES: &[&str] = &["Poly", "Mono", "Legato"];
pub const POLY: f32 = 0.0;
pub const MONO: f32 = 1.0;
pub const LEGATO: f32 = 2.0;

/// The ensemble's lines.
pub const LINE_COUNTS: &[&str] = &["3", "4"];

/// One formant: frequency (Hz), level (dB against the first) and
/// bandwidth (Hz).
pub type Formant = (f32, f32, f32);

/// The vowel table: for each section (Bass, Tenor, Countertenor, Alto,
/// Soprano) and each vowel in [`VOWELS`]' order (u, o, a, e, i), the five
/// formants. From the Csound manual, Appendix D, "Formant Values"
/// (https://csound.com/docs/manual/MiscFormants.html), reordered.
pub const TABLE: [[[Formant; 5]; 5]; 5] = [
    // Bass
    [
        f(
            [350, 600, 2400, 2675, 2950],
            [0, -20, -32, -28, -36],
            [40, 80, 100, 120, 120],
        ),
        f(
            [400, 750, 2400, 2600, 2900],
            [0, -11, -21, -20, -40],
            [40, 80, 100, 120, 120],
        ),
        f(
            [600, 1040, 2250, 2450, 2750],
            [0, -7, -9, -9, -20],
            [60, 70, 110, 120, 130],
        ),
        f(
            [400, 1620, 2400, 2800, 3100],
            [0, -12, -9, -12, -18],
            [40, 80, 100, 120, 120],
        ),
        f(
            [250, 1750, 2600, 3050, 3340],
            [0, -30, -16, -22, -28],
            [60, 90, 100, 120, 120],
        ),
    ],
    // Tenor
    [
        f(
            [350, 600, 2700, 2900, 3300],
            [0, -20, -17, -14, -26],
            [40, 60, 100, 120, 120],
        ),
        f(
            [400, 800, 2600, 2800, 3000],
            [0, -10, -12, -12, -26],
            [70, 80, 100, 130, 135],
        ),
        f(
            [650, 1080, 2650, 2900, 3250],
            [0, -6, -7, -8, -22],
            [80, 90, 120, 130, 140],
        ),
        f(
            [400, 1700, 2600, 3200, 3580],
            [0, -14, -12, -14, -20],
            [70, 80, 100, 120, 120],
        ),
        f(
            [290, 1870, 2800, 3250, 3540],
            [0, -15, -18, -20, -30],
            [40, 90, 100, 120, 120],
        ),
    ],
    // Countertenor
    [
        f(
            [370, 630, 2750, 3000, 3400],
            [0, -20, -23, -30, -34],
            [40, 60, 100, 120, 120],
        ),
        f(
            [430, 820, 2700, 3000, 3300],
            [0, -10, -26, -22, -34],
            [40, 80, 100, 120, 120],
        ),
        f(
            [660, 1120, 2750, 3000, 3350],
            [0, -6, -23, -24, -38],
            [80, 90, 120, 130, 140],
        ),
        f(
            [440, 1800, 2700, 3000, 3300],
            [0, -14, -18, -20, -20],
            [70, 80, 100, 120, 120],
        ),
        f(
            [270, 1850, 2900, 3350, 3590],
            [0, -24, -24, -36, -36],
            [40, 90, 100, 120, 120],
        ),
    ],
    // Alto
    [
        f(
            [325, 700, 2530, 3500, 4950],
            [0, -12, -30, -40, -64],
            [50, 60, 170, 180, 200],
        ),
        f(
            [450, 800, 2830, 3500, 4950],
            [0, -9, -16, -28, -55],
            [70, 80, 100, 130, 135],
        ),
        f(
            [800, 1150, 2800, 3500, 4950],
            [0, -4, -20, -36, -60],
            [80, 90, 120, 130, 140],
        ),
        f(
            [400, 1600, 2700, 3300, 4950],
            [0, -24, -30, -35, -60],
            [60, 80, 120, 150, 200],
        ),
        f(
            [350, 1700, 2700, 3700, 4950],
            [0, -20, -30, -36, -60],
            [50, 100, 120, 150, 200],
        ),
    ],
    // Soprano
    [
        f(
            [325, 700, 2700, 3800, 4950],
            [0, -16, -35, -40, -60],
            [50, 60, 170, 180, 200],
        ),
        f(
            [450, 800, 2830, 3800, 4950],
            [0, -11, -22, -22, -50],
            [40, 80, 100, 120, 120],
        ),
        f(
            [800, 1150, 2900, 3900, 4950],
            [0, -6, -32, -20, -50],
            [80, 90, 120, 130, 140],
        ),
        f(
            [350, 2000, 2800, 3600, 4950],
            [0, -20, -15, -40, -56],
            [60, 100, 120, 150, 200],
        ),
        f(
            [270, 2140, 2950, 3900, 4950],
            [0, -12, -26, -26, -44],
            [60, 90, 100, 120, 120],
        ),
    ],
];

const fn f(hz: [i32; 5], db: [i32; 5], bw: [i32; 5]) -> [Formant; 5] {
    let mut out = [(0.0, 0.0, 0.0); 5];
    let mut k = 0;
    while k < 5 {
        out[k] = (hz[k] as f32, db[k] as f32, bw[k] as f32);
        k += 1;
    }
    out
}

/// Where each section's range sits (MIDI note of its middle), for Mixed:
/// a note at or under the bass's is sung by the basses, one between two
/// sections' middles by a blend of the two, and so on up to the
/// sopranos. The countertenors sit out of Mixed.
pub const MIXED_MIDDLES: [(f32, f32); 4] =
    [(BASS, 50.0), (TENOR, 57.0), (ALTO, 64.0), (SOPRANO, 72.0)];

#[allow(clippy::too_many_arguments)]
const fn p(
    id: u16,
    name: &'static str,
    group: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: Unit,
    curve: Curve,
) -> ParamInfo {
    ParamInfo {
        id,
        name,
        group,
        min,
        max,
        default,
        unit,
        curve,
        readonly: false,
    }
}

const fn pct(id: u16, name: &'static str, group: &'static str, default: f32) -> ParamInfo {
    p(
        id,
        name,
        group,
        0.0,
        100.0,
        default,
        Unit::Percent,
        Curve::Linear,
    )
}

use Curve::{Choice, Linear, Log, Stepped};

pub static PARAMS: &[ParamInfo] = &[
    // Where along the vowels: Ooh (0), Oh, Aah (50), Eh, Ee (100), and
    // every blend between.
    pct(VOWEL, "Vowel", "Choir", 50.0),
    p(
        SECTION,
        "Section",
        "Choir",
        0.0,
        5.0,
        ALTO,
        Unit::None,
        Choice(SECTIONS),
    ),
    // Singers on each note, each with their own pitch, vibrato and
    // wavering.
    p(
        SINGERS,
        "Singers",
        "Choir",
        1.0,
        8.0,
        3.0,
        Unit::None,
        Stepped,
    ),
    // How far apart the singers' pitches are: the spread of their tuning.
    p(
        DETUNE,
        "Detune",
        "Choir",
        0.0,
        50.0,
        10.0,
        Unit::Cents,
        Linear,
    ),
    // Breath in the voice, through the same vowel.
    pct(BREATH, "Breath", "Choir", 12.0),
    // Darker (0) or brighter (100) than the vowel as sung (50).
    pct(BRIGHTNESS, "Brightness", "Choir", 50.0),
    p(
        OCTAVE,
        "Octave",
        "Choir",
        -2.0,
        2.0,
        0.0,
        Unit::Octaves,
        Stepped,
    ),
    // Each note starts this far along the vowels from Vowel and moves to
    // it over Start time ("oo" opening to "ah").
    p(
        START,
        "Start",
        "Vowel move",
        -100.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(
        START_TIME,
        "Start time",
        "Vowel move",
        10.0,
        4_000.0,
        400.0,
        Unit::Ms,
        Log,
    ),
    // A slow swing along the vowels, either side of Vowel.
    pct(MORPH, "Morph", "Vowel move", 0.0),
    p(
        MORPH_RATE,
        "Morph rate",
        "Vowel move",
        0.02,
        8.0,
        0.2,
        Unit::Hz,
        Log,
    ),
    p(
        ATTACK,
        "Attack",
        "Swell",
        1.0,
        8_000.0,
        250.0,
        Unit::Ms,
        Log,
    ),
    p(DECAY, "Decay", "Swell", 5.0, 8_000.0, 600.0, Unit::Ms, Log),
    pct(SUSTAIN, "Sustain", "Swell", 100.0),
    p(
        RELEASE,
        "Release",
        "Swell",
        10.0,
        12_000.0,
        900.0,
        Unit::Ms,
        Log,
    ),
    // How much quieter the softest note is than the hardest.
    p(
        VELOCITY,
        "Velocity",
        "Swell",
        0.0,
        40.0,
        20.0,
        Unit::Db,
        Linear,
    ),
    // How much darker a soft note is, as a singer singing softly.
    pct(VEL_TONE, "Vel > tone", "Swell", 60.0),
    // Each singer's vibrato, cents each way.
    p(
        VIBRATO,
        "Vibrato",
        "Vibrato",
        0.0,
        100.0,
        14.0,
        Unit::Cents,
        Linear,
    ),
    p(
        VIB_RATE,
        "Vib rate",
        "Vibrato",
        3.0,
        8.0,
        5.2,
        Unit::Hz,
        Linear,
    ),
    // How long after a note starts the vibrato comes in.
    p(
        VIB_DELAY,
        "Vib delay",
        "Vibrato",
        0.0,
        2_000.0,
        350.0,
        Unit::Ms,
        Linear,
    ),
    // The singers' small, uneven wavering in pitch: fast flutter and a
    // slow wander, each singer their own.
    pct(DRIFT, "Drift", "Vibrato", 40.0),
    // The built-in ensemble: dry (0) to all ensemble (100).
    pct(ENSEMBLE, "Ensemble", "Ensemble", 75.0),
    p(
        LINES,
        "Lines",
        "Ensemble",
        0.0,
        1.0,
        0.0,
        Unit::None,
        Choice(LINE_COUNTS),
    ),
    p(
        SLOW_RATE,
        "Slow rate",
        "Ensemble",
        0.1,
        3.0,
        0.5,
        Unit::Hz,
        Log,
    ),
    p(
        FAST_RATE,
        "Fast rate",
        "Ensemble",
        2.0,
        12.0,
        5.5,
        Unit::Hz,
        Log,
    ),
    // Both sweeps' depth against the ensemble's own (12 and 5 cents).
    p(
        DEPTH,
        "Depth",
        "Ensemble",
        0.0,
        200.0,
        60.0,
        Unit::Percent,
        Linear,
    ),
    pct(WIDTH, "Width", "Ensemble", 85.0),
    p(
        VOICES,
        "Voices",
        "Voice",
        1.0,
        16.0,
        8.0,
        Unit::None,
        Stepped,
    ),
    p(
        MODE,
        "Mode",
        "Voice",
        0.0,
        2.0,
        POLY,
        Unit::None,
        Choice(MODES),
    ),
    // The pitch slides from note to note in Mono and Legato.
    p(GLIDE, "Glide", "Voice", 0.0, 2_000.0, 0.0, Unit::Ms, Linear),
    p(
        TRANSPOSE,
        "Transpose",
        "Voice",
        -24.0,
        24.0,
        0.0,
        Unit::Semitones,
        Stepped,
    ),
    p(
        FINE,
        "Fine",
        "Voice",
        -100.0,
        100.0,
        0.0,
        Unit::Cents,
        Linear,
    ),
    p(
        BEND_RANGE,
        "Bend range",
        "Voice",
        0.0,
        24.0,
        2.0,
        Unit::Semitones,
        Stepped,
    ),
    // Pressing a pad swells the note: up to 12 dB under the sound's level
    // with no pressure, its level at full.
    pct(PRESS_SWELL, "Press > swell", "Pressure", 0.0),
    // Pressing a pad moves the vowel, up to two vowels either way.
    p(
        PRESS_VOWEL,
        "Press > vowel",
        "Pressure",
        -100.0,
        100.0,
        0.0,
        Unit::Percent,
        Linear,
    ),
    p(LEVEL, "Level", "Output", -36.0, 6.0, 0.0, Unit::Db, Linear),
    ParamInfo {
        id: METER_NOTES,
        name: "Notes sounding",
        group: "Output",
        min: 0.0,
        max: 20.0,
        default: 0.0,
        unit: Unit::None,
        curve: Stepped,
        readonly: true,
    },
];

/// The vowel at position `v` (0 to 100) in words: "Aah", or "Aah > Eh"
/// between two.
pub fn vowel_name(v: f32) -> String {
    let x = (v.clamp(0.0, 100.0) / 25.0).clamp(0.0, 4.0);
    let i = x.floor().min(3.0) as usize;
    let t = x - i as f32;
    if t < 0.15 {
        VOWELS[i].to_string()
    } else if t > 0.85 {
        VOWELS[i + 1].to_string()
    } else {
        format!("{} > {}", VOWELS[i], VOWELS[i + 1])
    }
}
