//! The drum synth's parameters, its id layout and its default kit; its
//! factory kits are in `presets/drums.presets` ([`crate::factory::read`]).
//!
//! **The id layout** (Drums owns 0 to 1023):
//!
//! | Ids | What |
//! |---|---|
//! | 0 to 255 | Each pad's first fields at `pad * 16 + field` (field 0 to 13: Model to Choke), as since the engine was built, so kits and projects saved before load to the same settings |
//! | 300 to 399 | The kit's own settings ([`LEVEL`], [`BASE_NOTE`], [`VOICES`], the meter) |
//! | 512 to 1023 | Each pad's newer fields at `512 + pad * 32 + (field - 16)` (field 16 to 47) |
//! | 1024 to 2047 | Not Drums': each pad's effects and sends (`drum_fx.rs`), pad `p` at `1024 + p * 64 + field` |
//! | 2048 to 2079 | Not Drums': the kit's sends |
//!
//! [`pad_id`] takes a pad and a field from either range ([`field`]).
//!
//! **Voices.** The kit's [`VOICES`] setting is **Classic** (the default)
//! or **Current**. Classic plays the pads exactly as the engine did before
//! 19 Nov 2026 (`drum/classic.rs`), so a kit or a project saved before,
//! which has no Voices line, sounds as it did. Current plays the rebuilt
//! voices: each drum type with its own knobs ([`knobs`]), velocity and
//! accent to tone, a little variation from hit to hit, decays that match
//! their labels. Every factory kit sets Current.

use mpfree_node::{Curve, ParamInfo, Unit};

/// Pads in a kit.
pub const PADS: usize = 16;
/// Id stride between pads, for fields 0 to 15.
pub const STRIDE: u16 = 16;
/// Where the pads' newer fields start, and their stride.
pub const MORE_BASE: u16 = 512;
pub const MORE_STRIDE: u16 = 32;

/// A pad's parameters: give one to [`pad_id`] with the pad.
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
    /// The first fields' count (0 to 13).
    pub const COUNT: usize = 14;

    // The newer fields, at 512 + pad * 32 + (field - 16). Current voices
    // only: Classic leaves them be.
    /// How much a hit near full velocity jumps in level and bite.
    pub const ACCENT: u16 = 16;
    /// How much each hit differs from the last (level, pitch, tone).
    pub const VARIATION: u16 = 17;
    /// One-shot, or sounding only while the pad is held.
    pub const PLAYS: u16 = 18;
    /// Clap: how many handclaps, and the gap between them.
    pub const BURSTS: u16 = 19;
    pub const BURST_GAP: u16 = 20;
    /// Clap: the room's tail after the claps.
    pub const ROOM: u16 = 21;
    /// FM perc: the modulator's frequency against the carrier's.
    pub const RATIO: u16 = 22;
    /// Toms, congas, woodblock, bell: where the stick lands, centre to edge.
    pub const STRIKE: u16 = 23;
    /// Hats, cymbal: how far the six oscillators spread (50 is the 808's).
    pub const SPREAD: u16 = 24;
    /// Snare: how long the shell rings against the wires.
    pub const RING: u16 = 25;
    /// Shaker: how grainy (the beads), and how long it takes to swell.
    pub const GRAIN: u16 = 26;
    pub const ATTACK: u16 = 27;
    /// The newer fields' count (16 to 27).
    pub const MORE: usize = 12;
}

/// The id of `field` (0 to 13, or 16 to 27) on pad `pad` (0 to 15).
pub const fn pad_id(pad: usize, field: u16) -> u16 {
    if field < STRIDE {
        pad as u16 * STRIDE + field
    } else {
        MORE_BASE + pad as u16 * MORE_STRIDE + (field - STRIDE)
    }
}

/// The pad and field of a pad's parameter id, if it is one.
pub const fn pad_field(id: u16) -> Option<(usize, u16)> {
    if id < PADS as u16 * STRIDE {
        if id % STRIDE < field::COUNT as u16 {
            Some(((id / STRIDE) as usize, id % STRIDE))
        } else {
            None
        }
    } else if id >= MORE_BASE && id < MORE_BASE + PADS as u16 * MORE_STRIDE {
        let off = id - MORE_BASE;
        if off % MORE_STRIDE < field::MORE as u16 {
            Some(((off / MORE_STRIDE) as usize, STRIDE + off % MORE_STRIDE))
        } else {
            None
        }
    } else {
        None
    }
}

/// The kit's level, in dB.
pub const LEVEL: u16 = 300;
/// The note pad 1 plays; pad n plays `BASE_NOTE + n - 1`.
pub const BASE_NOTE: u16 = 301;
/// Classic or Current voices (see the module docs).
pub const VOICES: u16 = 302;
/// Read-only: hits sounding now.
pub const METER_VOICES: u16 = 310;

/// The highest id plus one: values live in an array indexed by id.
pub const ID_SPACE: usize = 1024;

/// The [`VOICES`] choices.
pub const VOICE_SETS: &[&str] = &["Classic", "Current"];
/// [`VOICES`] at Current.
pub const CURRENT: f32 = 1.0;

/// The sound models, in the order of the Model choice. The first eleven
/// are the engine's first; their order and names never change, since
/// kits name them.
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
    "808 kick",
    "Conga",
    "Woodblock",
    "Bell",
    "Shaker",
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
    pub const KICK_808: u8 = 11;
    pub const CONGA: u8 = 12;
    pub const WOODBLOCK: u8 = 13;
    pub const BELL: u8 = 14;
    pub const SHAKER: u8 = 15;
    /// The models the first engine had (Classic plays these as it did).
    pub const FIRST: u8 = 11;
}

/// What kind of drum a model is: which knobs it shows, and how the lab and
/// the pad labels group it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Kick,
    Snare,
    Clap,
    Hat,
    Cymbal,
    /// Toms, congas, woodblock, bell, rim and cowbell.
    Perc,
    /// Zap and FM perc: sound-design hits.
    Fx,
    Shaker,
}

/// The kind of drum model `m` is.
pub const fn kind(m: u8) -> Kind {
    use model::*;
    match m {
        KICK | KICK_808 => Kind::Kick,
        SNARE => Kind::Snare,
        CLAP => Kind::Clap,
        CLOSED_HAT | OPEN_HAT => Kind::Hat,
        CYMBAL => Kind::Cymbal,
        ZAP | FM_PERC => Kind::Fx,
        SHAKER => Kind::Shaker,
        _ => Kind::Perc,
    }
}

/// A model's index from a Model value (out of range clamps to the last).
pub fn model_of(value: f32) -> u8 {
    (value.max(0.0) as usize).min(MODELS.len() - 1) as u8
}

pub const CHOKES: &[&str] = &["Off", "1", "2", "3", "4"];
/// The Plays choices.
pub const PLAY_MODES: &[&str] = &["One-shot", "While held"];

/// A pad's knobs that every Current drum shows besides its own: the
/// model, its mix and how it plays.
pub const SHARED: [u16; 9] = [
    field::MODEL,
    field::LEVEL,
    field::PAN,
    field::CHOKE,
    field::VEL_LEVEL,
    field::VEL_TONE,
    field::ACCENT,
    field::VARIATION,
    field::PLAYS,
];

/// A knob as a drum shows it: the field and its label there.
pub type Knob = (u16, &'static str);

use field::*;

const KICK_KNOBS: &[Knob] = &[
    (TUNE, "Tune"),
    (DECAY, "Decay"),
    (SWEEP, "Sweep"),
    (SWEEP_TIME, "Sweep time"),
    (SNAP, "Click"),
    (TONE, "Tone"),
    (DRIVE, "Drive"),
];
const KICK_808_KNOBS: &[Knob] = &[
    (TUNE, "Tune"),
    (DECAY, "Decay"),
    (TONE, "Tone"),
    (SNAP, "Punch"),
    (SWEEP, "Sigh"),
    (SWEEP_TIME, "Sigh time"),
    (DRIVE, "Drive"),
];
const SNARE_KNOBS: &[Knob] = &[
    (TUNE, "Tune"),
    (DECAY, "Decay"),
    (TONE, "Tone"),
    (NOISE, "Snappy"),
    (RING, "Ring"),
    (SNAP, "Click"),
    (DRIVE, "Drive"),
];
const CLAP_KNOBS: &[Knob] = &[
    (TONE, "Tone"),
    (DECAY, "Decay"),
    (BURSTS, "Claps"),
    (BURST_GAP, "Gap"),
    (ROOM, "Room"),
    (SNAP, "Sharp"),
    (DRIVE, "Drive"),
];
const HAT_KNOBS: &[Knob] = &[
    (TUNE, "Tune"),
    (DECAY, "Decay"),
    (TONE, "Tone"),
    (NOISE, "Noise"),
    (SPREAD, "Spread"),
    (SNAP, "Click"),
    (DRIVE, "Drive"),
];
const COWBELL_KNOBS: &[Knob] = &[
    (TUNE, "Tune"),
    (DECAY, "Decay"),
    (TONE, "Tone"),
    (SNAP, "Click"),
    (NOISE, "Noise"),
    (DRIVE, "Drive"),
];
const TOM_KNOBS: &[Knob] = &[
    (TUNE, "Tune"),
    (DECAY, "Decay"),
    (TONE, "Tone"),
    (STRIKE, "Strike"),
    (SWEEP, "Bend"),
    (SWEEP_TIME, "Bend time"),
    (SNAP, "Click"),
];
const CONGA_KNOBS: &[Knob] = &[
    (TUNE, "Tune"),
    (DECAY, "Decay"),
    (TONE, "Tone"),
    (STRIKE, "Strike"),
    (SNAP, "Slap"),
    (SWEEP, "Bend"),
    (SWEEP_TIME, "Bend time"),
];
const WOOD_KNOBS: &[Knob] = &[
    (TUNE, "Tune"),
    (DECAY, "Decay"),
    (TONE, "Tone"),
    (STRIKE, "Strike"),
    (SNAP, "Click"),
];
const RIM_KNOBS: &[Knob] = &[
    (TUNE, "Tune"),
    (DECAY, "Decay"),
    (TONE, "Tone"),
    (SNAP, "Click"),
    (NOISE, "Noise"),
    (DRIVE, "Drive"),
];
const ZAP_KNOBS: &[Knob] = &[
    (TUNE, "Tune"),
    (DECAY, "Decay"),
    (SWEEP, "Sweep"),
    (SWEEP_TIME, "Sweep time"),
    (TONE, "Tone"),
    (NOISE, "Noise"),
    (DRIVE, "Drive"),
];
const FM_KNOBS: &[Knob] = &[
    (TUNE, "Tune"),
    (DECAY, "Decay"),
    (TONE, "Tone"),
    (RATIO, "Ratio"),
    (SWEEP, "Sweep"),
    (SWEEP_TIME, "Sweep time"),
    (NOISE, "Noise"),
];
const SHAKER_KNOBS: &[Knob] = &[
    (TUNE, "Tune"),
    (DECAY, "Decay"),
    (ATTACK, "Attack"),
    (TONE, "Tone"),
    (GRAIN, "Grain"),
];

/// The knobs a Current drum of model `m` shows besides [`SHARED`], in
/// order, with their labels there. The first four are its Q-Links unless
/// [`qlinks`] says otherwise.
pub const fn knobs(m: u8) -> &'static [Knob] {
    use model::*;
    match m {
        KICK => KICK_KNOBS,
        KICK_808 => KICK_808_KNOBS,
        SNARE => SNARE_KNOBS,
        CLAP => CLAP_KNOBS,
        CLOSED_HAT | OPEN_HAT | CYMBAL => HAT_KNOBS,
        COWBELL => COWBELL_KNOBS,
        TOM => TOM_KNOBS,
        CONGA => CONGA_KNOBS,
        WOODBLOCK | BELL => WOOD_KNOBS,
        RIM => RIM_KNOBS,
        ZAP => ZAP_KNOBS,
        FM_PERC => FM_KNOBS,
        _ => SHAKER_KNOBS,
    }
}

/// The four knobs the Q-Links turn on a Current drum of model `m`: the
/// ones players reach for on that drum.
pub const fn qlinks(m: u8) -> [u16; 4] {
    use model::*;
    match m {
        KICK => [TUNE, DECAY, SWEEP, SNAP],
        KICK_808 => [TUNE, DECAY, TONE, DRIVE],
        SNARE => [TUNE, DECAY, TONE, NOISE],
        CLAP => [TONE, DECAY, BURST_GAP, ROOM],
        CLOSED_HAT | OPEN_HAT => [TUNE, DECAY, TONE, NOISE],
        CYMBAL => [TUNE, DECAY, TONE, SPREAD],
        TOM | CONGA => [TUNE, DECAY, TONE, SWEEP],
        ZAP => [TUNE, DECAY, SWEEP, SWEEP_TIME],
        FM_PERC => [TUNE, DECAY, TONE, RATIO],
        SHAKER => [DECAY, ATTACK, TONE, GRAIN],
        _ => [TUNE, DECAY, TONE, SNAP],
    }
}

/// The Q-Links on a Classic kit's pad: the same four on every model, as
/// before.
pub const CLASSIC_QLINKS: [u16; 4] = [TUNE, DECAY, TONE, SNAP];

/// The label a Current drum of model `m` gives `field`, if it shows it.
pub fn label(m: u8, f: u16) -> Option<&'static str> {
    if let Some(&(_, l)) = knobs(m).iter().find(|(k, _)| *k == f) {
        return Some(l);
    }
    SHARED.contains(&f).then(|| info_name(f))
}

/// The fields pad `m`'s page shows, in order: a model the first engine
/// had, in a Classic kit, shows its first fields (Model to Choke) as it
/// always did; any other shows [`SHARED`] and its own [`knobs`].
pub fn page_fields(m: u8, current: bool) -> Vec<u16> {
    if !current && m < model::FIRST {
        (0..field::COUNT as u16).collect()
    } else {
        SHARED
            .iter()
            .copied()
            .chain(knobs(m).iter().map(|&(f, _)| f))
            .collect()
    }
}

/// The four fields the Q-Links turn on pad `m`'s page (see
/// [`page_fields`]).
pub fn page_qlinks(m: u8, current: bool) -> [u16; 4] {
    if !current && m < model::FIRST {
        CLASSIC_QLINKS
    } else {
        qlinks(m)
    }
}

/// Why `f` does nothing on pad `m`'s page, if it doesn't: a first-engine
/// field the Classic model leaves at nothing, or a shared field only the
/// Current voices play.
pub fn idle_reason(m: u8, current: bool, f: u16) -> Option<&'static str> {
    use field::*;
    if !current && m < model::FIRST {
        return (!classic_uses(m, f)).then_some("this drum doesn't use it");
    }
    if !current && matches!(f, ACCENT | VARIATION | PLAYS) {
        return Some("kit voices are Classic");
    }
    None
}

/// Whether a Classic kit's model `m` (one of the first eleven) uses
/// `field` (0 to 13): the first engine's models leave some at nothing (a
/// clap has no body to sweep). The newer fields are Current's only.
pub fn classic_uses(m: u8, f: u16) -> bool {
    use model::*;
    let no_body = matches!(m, CLAP | CLOSED_HAT | OPEN_HAT | CYMBAL | COWBELL);
    match f {
        TUNE | NOISE => m != CLAP,
        SWEEP | SWEEP_TIME => !no_body,
        _ => f < STRIDE,
    }
}

/// One pad's first fields (Model .. Choke), in field order. The newer
/// fields' defaults are the same on every pad ([`MORE_DEFAULTS`]).
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

/// The first fields' defaults: a full kit in the MPC's usual places, kick
/// on pad 1, snare 2, clap 3, hats 4 and 5 (choked together), toms 6 to 8,
/// then percussion. These are the first engine's Init kit, unchanged, so a
/// kit saved with only some of them set loads as it did; the factory Init
/// Kit sets its newer pads in `presets/drums.presets`.
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

/// The newer fields' defaults, the same on every pad, in field order
/// (Accent .. Attack).
pub const MORE_DEFAULTS: [f32; field::MORE] = [
    50.0, // Accent
    50.0, // Variation
    0.0,  // Plays: one-shot
    4.0,  // Claps
    10.0, // Gap, ms
    35.0, // Room
    1.41, // Ratio
    30.0, // Strike
    50.0, // Spread: the 808's
    50.0, // Ring
    50.0, // Grain
    10.0, // Attack, ms
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
        id: VOICES,
        name: "Voices",
        group: "Kit",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(VOICE_SETS),
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

const GROUPS: [&str; PADS] = [
    "Pad 1", "Pad 2", "Pad 3", "Pad 4", "Pad 5", "Pad 6", "Pad 7", "Pad 8", "Pad 9", "Pad 10",
    "Pad 11", "Pad 12", "Pad 13", "Pad 14", "Pad 15", "Pad 16",
];

/// A pad field's name, range, unit and curve.
const fn shape(f: u16) -> (&'static str, f32, f32, Unit, Curve) {
    match f {
        MODEL => (
            "Model",
            0.0,
            (MODELS.len() - 1) as f32,
            Unit::None,
            Curve::Choice(MODELS),
        ),
        TUNE => ("Tune", -24.0, 24.0, Unit::Semitones, Curve::Linear),
        DECAY => ("Decay", 10.0, 5000.0, Unit::Ms, Curve::Log),
        TONE => ("Tone", 0.0, 100.0, Unit::Percent, Curve::Linear),
        SNAP => ("Snap", 0.0, 100.0, Unit::Percent, Curve::Linear),
        NOISE => ("Noise", 0.0, 100.0, Unit::Percent, Curve::Linear),
        SWEEP => ("Sweep", 0.0, 48.0, Unit::Semitones, Curve::Linear),
        SWEEP_TIME => ("Sweep time", 1.0, 1000.0, Unit::Ms, Curve::Log),
        DRIVE => ("Drive", 0.0, 100.0, Unit::Percent, Curve::Linear),
        field::LEVEL => ("Level", -60.0, 6.0, Unit::Db, Curve::Linear),
        PAN => ("Pan", -100.0, 100.0, Unit::Percent, Curve::Linear),
        VEL_TONE => ("Vel > tone", 0.0, 100.0, Unit::Percent, Curve::Linear),
        VEL_LEVEL => ("Vel > level", 0.0, 100.0, Unit::Percent, Curve::Linear),
        CHOKE => (
            "Choke",
            0.0,
            (CHOKES.len() - 1) as f32,
            Unit::None,
            Curve::Choice(CHOKES),
        ),
        ACCENT => ("Accent", 0.0, 100.0, Unit::Percent, Curve::Linear),
        VARIATION => ("Variation", 0.0, 100.0, Unit::Percent, Curve::Linear),
        PLAYS => (
            "Plays",
            0.0,
            (PLAY_MODES.len() - 1) as f32,
            Unit::None,
            Curve::Choice(PLAY_MODES),
        ),
        BURSTS => ("Bursts", 1.0, 8.0, Unit::None, Curve::Stepped),
        BURST_GAP => ("Burst gap", 2.0, 40.0, Unit::Ms, Curve::Linear),
        ROOM => ("Room", 0.0, 100.0, Unit::Percent, Curve::Linear),
        RATIO => ("Ratio", 0.5, 8.0, Unit::None, Curve::Log),
        STRIKE => ("Strike", 0.0, 100.0, Unit::Percent, Curve::Linear),
        SPREAD => ("Spread", 0.0, 100.0, Unit::Percent, Curve::Linear),
        RING => ("Ring", 0.0, 100.0, Unit::Percent, Curve::Linear),
        GRAIN => ("Grain", 0.0, 100.0, Unit::Percent, Curve::Linear),
        _ => ("Attack", 1.0, 60.0, Unit::Ms, Curve::Log),
    }
}

/// A pad field's own name (as files and MIDI Learn name it).
pub const fn info_name(f: u16) -> &'static str {
    shape(f).0
}

const fn default_of(pad: usize, f: u16) -> f32 {
    if f < STRIDE {
        INIT_KIT[pad].0[f as usize]
    } else {
        MORE_DEFAULTS[(f - STRIDE) as usize]
    }
}

const fn pad_param(pad: usize, f: u16) -> ParamInfo {
    let (name, min, max, unit, curve) = shape(f);
    ParamInfo {
        id: pad_id(pad, f),
        name,
        group: GROUPS[pad],
        min,
        max,
        default: default_of(pad, f),
        unit,
        curve,
        readonly: false,
    }
}

/// Fields per pad, both ranges.
const PER_PAD: usize = field::COUNT + field::MORE;
const N_PARAMS: usize = PADS * PER_PAD + KIT.len();

/// The field at place `k` of a pad's list (the first fields, then the
/// newer ones).
const fn field_at(k: usize) -> u16 {
    if k < field::COUNT {
        k as u16
    } else {
        STRIDE + (k - field::COUNT) as u16
    }
}

const fn build() -> [ParamInfo; N_PARAMS] {
    let mut out = [KIT[0]; N_PARAMS];
    let mut i = 0;
    while i < PADS * PER_PAD {
        out[i] = pad_param(i / PER_PAD, field_at(i % PER_PAD));
        i += 1;
    }
    let mut k = 0;
    while k < KIT.len() {
        out[PADS * PER_PAD + k] = KIT[k];
        k += 1;
    }
    out
}

/// Every parameter: Pad 1 to Pad 16 (each its first fields, then the
/// newer ones), then Kit.
pub static PARAMS: [ParamInfo; N_PARAMS] = build();

/// Each id's place in [`PARAMS`], or `u16::MAX` for an id Drums lacks.
pub static INDEX: [u16; ID_SPACE] = {
    let mut t = [u16::MAX; ID_SPACE];
    let mut i = 0;
    while i < N_PARAMS {
        t[PARAMS[i].id as usize] = i as u16;
        i += 1;
    }
    t
};

/// Drums' parameter with id `id`, if it has one.
pub fn param_info(id: u16) -> Option<&'static ParamInfo> {
    let i = *INDEX.get(usize::from(id))?;
    PARAMS.get(usize::from(i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_id_is_in_its_range_and_found() {
        let mut seen = std::collections::HashSet::new();
        for (i, p) in PARAMS.iter().enumerate() {
            assert!(usize::from(p.id) < ID_SPACE, "{}", p.id);
            assert!(seen.insert(p.id), "{} twice", p.id);
            assert_eq!(usize::from(INDEX[usize::from(p.id)]), i);
        }
        for pad in 0..PADS {
            for k in 0..PER_PAD {
                let f = field_at(k);
                let id = pad_id(pad, f);
                assert_eq!(pad_field(id), Some((pad, f)), "{pad} {f}");
                assert_eq!(param_info(id).unwrap().group, GROUPS[pad]);
            }
        }
        // The first fields kept their ids.
        assert_eq!(pad_id(3, field::DECAY), 50);
        assert_eq!(pad_id(15, field::CHOKE), 253);
        assert_eq!(pad_id(0, field::ACCENT), 512);
        assert_eq!(pad_id(15, field::ATTACK), 512 + 15 * 32 + 11);
        // Below the drum effects' range.
        assert!(PARAMS.iter().all(|p| p.id < 1024));
        assert_eq!(pad_field(LEVEL), None);
        assert_eq!(pad_field(14), None);
    }

    #[test]
    fn every_pad_field_has_its_own_name() {
        // The first fields' names as the first engine gave them: files
        // and MIDI Learn name them so.
        let first = [
            "Model", "Tune", "Decay", "Tone", "Snap", "Noise", "Sweep", "Sweep time", "Drive",
            "Level", "Pan", "Vel > tone", "Vel > level", "Choke",
        ];
        for (f, want) in first.iter().enumerate() {
            assert_eq!(info_name(f as u16), *want);
        }
        let mut names = std::collections::HashSet::new();
        for k in 0..PER_PAD {
            assert!(names.insert(info_name(field_at(k))), "{}", info_name(field_at(k)));
        }
    }

    #[test]
    fn every_current_drum_fits_a_page() {
        for m in 0..MODELS.len() as u8 {
            let n = SHARED.len() + knobs(m).len();
            assert!(n <= 16, "{}: {n}", MODELS[m as usize]);
            for q in qlinks(m) {
                assert!(knobs(m).iter().any(|(f, _)| *f == q), "{} {q}", MODELS[m as usize]);
            }
            for (f, _) in knobs(m) {
                assert!(!SHARED.contains(f));
            }
        }
    }
}
