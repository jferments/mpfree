//! The drum synth's parameters, its default kit and its factory kits.
//!
//! Each of the 16 pads has the same 14 parameters at ids `pad * 16 + field`
//! ([`pad_id`]), grouped as "Pad 1" to "Pad 16". Kit-wide parameters start
//! at 300.

use mpfree_node::{Curve, ParamInfo, Unit};

use crate::params::Preset;

/// Pads in a kit.
pub const PADS: usize = 16;
/// Id stride between pads.
pub const STRIDE: u16 = 16;

/// A pad's parameters: add to `pad * STRIDE` ([`pad_id`]).
pub mod field {
    pub const MODEL: u16 = 0;
    pub const TUNE: u16 = 1;
    pub const DECAY: u16 = 2;
    pub const TONE: u16 = 3;
    pub const SNAP: u16 = 4;
    pub const NOISE: u16 = 5;
    pub const SWEEP: u16 = 6;
    pub const SWEEP_TIME: u16 = 7;
    pub const DRIVE: u16 = 8;
    pub const LEVEL: u16 = 9;
    pub const PAN: u16 = 10;
    pub const VEL_TONE: u16 = 11;
    pub const VEL_LEVEL: u16 = 12;
    pub const CHOKE: u16 = 13;
    /// Fields per pad.
    pub const COUNT: usize = 14;
}

/// The id of `field` on pad `pad` (0 to 15).
pub const fn pad_id(pad: usize, field: u16) -> u16 {
    pad as u16 * STRIDE + field
}

/// The kit's level, in dB.
pub const LEVEL: u16 = 300;
/// The note pad 1 plays; pad n plays `BASE_NOTE + n - 1`.
pub const BASE_NOTE: u16 = 301;
/// Read-only: hits sounding now.
pub const METER_VOICES: u16 = 310;

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = 311;

/// The sound models, in the order of the Model choice.
pub const MODELS: &[&str] = &[
    "Kick",
    "Snare",
    "Clap",
    "Closed hat",
    "Open hat",
    "Tom",
    "Rim",
    "Cowbell",
    "Cymbal",
    "Zap",
    "FM perc",
];

/// Model indexes, for kits and tests.
pub mod model {
    pub const KICK: u8 = 0;
    pub const SNARE: u8 = 1;
    pub const CLAP: u8 = 2;
    pub const CLOSED_HAT: u8 = 3;
    pub const OPEN_HAT: u8 = 4;
    pub const TOM: u8 = 5;
    pub const RIM: u8 = 6;
    pub const COWBELL: u8 = 7;
    pub const CYMBAL: u8 = 8;
    pub const ZAP: u8 = 9;
    pub const FM_PERC: u8 = 10;
}

pub const CHOKES: &[&str] = &["Off", "1", "2", "3", "4"];

const GROUPS: [&str; PADS] = [
    "Pad 1", "Pad 2", "Pad 3", "Pad 4", "Pad 5", "Pad 6", "Pad 7", "Pad 8", "Pad 9", "Pad 10",
    "Pad 11", "Pad 12", "Pad 13", "Pad 14", "Pad 15", "Pad 16",
];

/// One pad's settings, in field order (Model .. Choke).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadDefaults(pub [f32; field::COUNT]);

/// `PadDefaults` in field order: model, tune, decay ms, tone, snap, noise,
/// sweep st, sweep ms, drive, level dB, pan, vel>tone, vel>level, choke.
#[allow(clippy::too_many_arguments)]
const fn pd(
    model: u8,
    tune: f32,
    decay: f32,
    tone: f32,
    snap: f32,
    noise: f32,
    sweep: f32,
    sweep_ms: f32,
    drive: f32,
    level: f32,
    pan: f32,
    choke: u8,
) -> PadDefaults {
    PadDefaults([
        model as f32,
        tune,
        decay,
        tone,
        snap,
        noise,
        sweep,
        sweep_ms,
        drive,
        level,
        pan,
        30.0,
        70.0,
        choke as f32,
    ])
}

use model::*;

/// The kit every pad starts with (and the "Init Kit" preset): a full kit in
/// the MPC's usual places, kick on pad 1, snare 2, clap 3, hats 4 and 5
/// (choked together), toms 6 to 8, then percussion.
pub const INIT_KIT: [PadDefaults; PADS] = [
    pd(
        KICK, 0.0, 450.0, 30.0, 40.0, 0.0, 24.0, 45.0, 10.0, 0.0, 0.0, 0,
    ),
    pd(
        SNARE, 0.0, 220.0, 55.0, 50.0, 60.0, 7.0, 20.0, 0.0, 0.0, 0.0, 0,
    ),
    pd(
        CLAP, 0.0, 300.0, 50.0, 50.0, 100.0, 0.0, 10.0, 0.0, 0.0, 0.0, 0,
    ),
    pd(
        CLOSED_HAT, 0.0, 70.0, 60.0, 30.0, 30.0, 0.0, 10.0, 0.0, -2.0, 15.0, 1,
    ),
    pd(
        OPEN_HAT, 0.0, 450.0, 60.0, 30.0, 30.0, 0.0, 10.0, 0.0, -2.0, 15.0, 1,
    ),
    pd(
        TOM, -5.0, 450.0, 40.0, 30.0, 10.0, 10.0, 60.0, 0.0, 0.0, -30.0, 0,
    ),
    pd(
        TOM, 0.0, 400.0, 40.0, 30.0, 10.0, 10.0, 60.0, 0.0, 0.0, -10.0, 0,
    ),
    pd(
        TOM, 5.0, 350.0, 40.0, 30.0, 10.0, 10.0, 60.0, 0.0, 0.0, 10.0, 0,
    ),
    pd(
        RIM, 0.0, 60.0, 60.0, 50.0, 20.0, 0.0, 10.0, 0.0, 0.0, 0.0, 0,
    ),
    pd(
        COWBELL, 0.0, 350.0, 50.0, 30.0, 0.0, 0.0, 10.0, 0.0, -3.0, -20.0, 0,
    ),
    pd(
        CYMBAL, 0.0, 1800.0, 60.0, 30.0, 40.0, 0.0, 10.0, 0.0, -3.0, 25.0, 0,
    ),
    pd(
        ZAP, 0.0, 180.0, 50.0, 20.0, 0.0, 36.0, 25.0, 0.0, 0.0, 0.0, 0,
    ),
    pd(
        FM_PERC, 0.0, 250.0, 50.0, 40.0, 0.0, 0.0, 80.0, 0.0, 0.0, 20.0, 0,
    ),
    pd(
        KICK, -3.0, 1500.0, 20.0, 20.0, 0.0, 12.0, 60.0, 25.0, 0.0, 0.0, 0,
    ),
    pd(
        SNARE, 3.0, 140.0, 75.0, 70.0, 75.0, 5.0, 15.0, 0.0, 0.0, 0.0, 0,
    ),
    pd(
        CLOSED_HAT, 12.0, 120.0, 70.0, 10.0, 90.0, 0.0, 10.0, 0.0, -9.0, -25.0, 0,
    ),
];

const KIT: &[ParamInfo] = &[
    ParamInfo {
        id: LEVEL,
        name: "Level",
        group: "Kit",
        min: -60.0,
        max: 6.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: BASE_NOTE,
        name: "Pad 1 note",
        group: "Kit",
        min: 0.0,
        max: 112.0,
        default: 36.0,
        unit: Unit::None,
        curve: Curve::Stepped,
        readonly: false,
    },
    ParamInfo {
        id: METER_VOICES,
        name: "Hits on",
        group: "Kit",
        min: 0.0,
        max: 32.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Stepped,
        readonly: true,
    },
];

const fn pad_param(pad: usize, f: u16) -> ParamInfo {
    let d = INIT_KIT[pad].0[f as usize];
    let (name, min, max, unit, curve): (&'static str, f32, f32, Unit, Curve) = match f {
        field::MODEL => (
            "Model",
            0.0,
            (MODELS.len() - 1) as f32,
            Unit::None,
            Curve::Choice(MODELS),
        ),
        field::TUNE => ("Tune", -24.0, 24.0, Unit::Semitones, Curve::Linear),
        field::DECAY => ("Decay", 10.0, 5000.0, Unit::Ms, Curve::Log),
        field::TONE => ("Tone", 0.0, 100.0, Unit::Percent, Curve::Linear),
        field::SNAP => ("Snap", 0.0, 100.0, Unit::Percent, Curve::Linear),
        field::NOISE => ("Noise", 0.0, 100.0, Unit::Percent, Curve::Linear),
        field::SWEEP => ("Sweep", 0.0, 48.0, Unit::Semitones, Curve::Linear),
        field::SWEEP_TIME => ("Sweep time", 1.0, 1000.0, Unit::Ms, Curve::Log),
        field::DRIVE => ("Drive", 0.0, 100.0, Unit::Percent, Curve::Linear),
        field::LEVEL => ("Level", -60.0, 6.0, Unit::Db, Curve::Linear),
        field::PAN => ("Pan", -100.0, 100.0, Unit::Percent, Curve::Linear),
        field::VEL_TONE => ("Vel > tone", 0.0, 100.0, Unit::Percent, Curve::Linear),
        field::VEL_LEVEL => ("Vel > level", 0.0, 100.0, Unit::Percent, Curve::Linear),
        _ => (
            "Choke",
            0.0,
            (CHOKES.len() - 1) as f32,
            Unit::None,
            Curve::Choice(CHOKES),
        ),
    };
    ParamInfo {
        id: pad_id(pad, f),
        name,
        group: GROUPS[pad],
        min,
        max,
        default: d,
        unit,
        curve,
        readonly: false,
    }
}

const N_PARAMS: usize = PADS * field::COUNT + KIT.len();

const fn build() -> [ParamInfo; N_PARAMS] {
    let mut out = [KIT[0]; N_PARAMS];
    let mut i = 0;
    while i < PADS * field::COUNT {
        out[i] = pad_param(i / field::COUNT, (i % field::COUNT) as u16);
        i += 1;
    }
    let mut k = 0;
    while k < KIT.len() {
        out[PADS * field::COUNT + k] = KIT[k];
        k += 1;
    }
    out
}

/// Every parameter: Pad 1 to Pad 16, then Kit.
pub static PARAMS: [ParamInfo; N_PARAMS] = build();

use field as f;

/// Factory kits. Each starts from [`INIT_KIT`] and changes what it lists.
pub static KITS: &[Preset] = &[
    Preset {
        name: "Init Kit",
        values: &[],
    },
    Preset {
        // Long, low, round: 808-style booms, soft snare, sizzling hats.
        name: "Boom Kit",
        values: &[
            (pad_id(0, f::TUNE), -4.0),
            (pad_id(0, f::DECAY), 1800.0),
            (pad_id(0, f::SWEEP), 14.0),
            (pad_id(0, f::SWEEP_TIME), 70.0),
            (pad_id(0, f::SNAP), 15.0),
            (pad_id(0, f::DRIVE), 20.0),
            (pad_id(1, f::DECAY), 300.0),
            (pad_id(1, f::TONE), 40.0),
            (pad_id(1, f::NOISE), 45.0),
            (pad_id(2, f::DECAY), 450.0),
            (pad_id(2, f::TONE), 40.0),
            (pad_id(3, f::DECAY), 50.0),
            (pad_id(3, f::TONE), 75.0),
            (pad_id(4, f::DECAY), 700.0),
            (pad_id(4, f::TONE), 75.0),
            (pad_id(5, f::DECAY), 900.0),
            (pad_id(6, f::DECAY), 800.0),
            (pad_id(7, f::DECAY), 700.0),
            (pad_id(13, f::TUNE), -7.0),
            (pad_id(13, f::DECAY), 3000.0),
            (pad_id(13, f::DRIVE), 45.0),
        ],
    },
    Preset {
        // Short, punchy, bright: 909-style kick click, crisp snare, tight hats.
        name: "Tight Kit",
        values: &[
            (pad_id(0, f::TUNE), 2.0),
            (pad_id(0, f::DECAY), 280.0),
            (pad_id(0, f::SNAP), 75.0),
            (pad_id(0, f::SWEEP), 30.0),
            (pad_id(0, f::SWEEP_TIME), 25.0),
            (pad_id(0, f::DRIVE), 30.0),
            (pad_id(1, f::TUNE), 2.0),
            (pad_id(1, f::DECAY), 160.0),
            (pad_id(1, f::TONE), 75.0),
            (pad_id(1, f::SNAP), 80.0),
            (pad_id(1, f::NOISE), 70.0),
            (pad_id(2, f::DECAY), 220.0),
            (pad_id(2, f::TONE), 65.0),
            (pad_id(3, f::DECAY), 40.0),
            (pad_id(3, f::TONE), 85.0),
            (pad_id(4, f::DECAY), 280.0),
            (pad_id(4, f::TONE), 85.0),
            (pad_id(5, f::DECAY), 250.0),
            (pad_id(6, f::DECAY), 220.0),
            (pad_id(7, f::DECAY), 200.0),
            (pad_id(8, f::DECAY), 40.0),
            (pad_id(10, f::DECAY), 1200.0),
            (pad_id(10, f::TONE), 80.0),
        ],
    },
    Preset {
        // Grimy: everything driven, dark tops, a distorted boom on pad 14.
        name: "Grit Kit",
        values: &[
            (pad_id(0, f::DRIVE), 70.0),
            (pad_id(0, f::TONE), 60.0),
            (pad_id(0, f::DECAY), 600.0),
            (pad_id(1, f::DRIVE), 60.0),
            (pad_id(1, f::TONE), 35.0),
            (pad_id(1, f::NOISE), 80.0),
            (pad_id(2, f::DRIVE), 50.0),
            (pad_id(2, f::TONE), 30.0),
            (pad_id(3, f::TONE), 30.0),
            (pad_id(3, f::DRIVE), 40.0),
            (pad_id(4, f::TONE), 30.0),
            (pad_id(4, f::DRIVE), 40.0),
            (pad_id(5, f::DRIVE), 50.0),
            (pad_id(6, f::DRIVE), 50.0),
            (pad_id(7, f::DRIVE), 50.0),
            (pad_id(13, f::DRIVE), 90.0),
            (pad_id(13, f::DECAY), 2200.0),
            (pad_id(13, f::TONE), 70.0),
            (LEVEL, -3.0),
        ],
    },
    Preset {
        // Electro: zaps, FM blips and a snappy kick, toms swapped for zaps.
        name: "Electro Kit",
        values: &[
            (pad_id(0, f::SWEEP), 40.0),
            (pad_id(0, f::SWEEP_TIME), 18.0),
            (pad_id(0, f::DECAY), 380.0),
            (pad_id(0, f::SNAP), 60.0),
            (pad_id(1, f::MODEL), CLAP as f32),
            (pad_id(1, f::DECAY), 180.0),
            (pad_id(1, f::TONE), 70.0),
            (pad_id(5, f::MODEL), ZAP as f32),
            (pad_id(5, f::TUNE), -5.0),
            (pad_id(5, f::SWEEP), 30.0),
            (pad_id(5, f::SWEEP_TIME), 40.0),
            (pad_id(5, f::DECAY), 300.0),
            (pad_id(6, f::MODEL), ZAP as f32),
            (pad_id(6, f::SWEEP), 36.0),
            (pad_id(6, f::SWEEP_TIME), 30.0),
            (pad_id(6, f::DECAY), 250.0),
            (pad_id(7, f::MODEL), ZAP as f32),
            (pad_id(7, f::TUNE), 7.0),
            (pad_id(7, f::SWEEP), 42.0),
            (pad_id(7, f::SWEEP_TIME), 20.0),
            (pad_id(7, f::DECAY), 200.0),
            (pad_id(12, f::TONE), 80.0),
            (pad_id(12, f::TUNE), 12.0),
            (pad_id(12, f::DECAY), 160.0),
            (pad_id(14, f::MODEL), FM_PERC as f32),
            (pad_id(14, f::TONE), 35.0),
            (pad_id(14, f::TUNE), -12.0),
            (pad_id(14, f::DECAY), 350.0),
        ],
    },
];
