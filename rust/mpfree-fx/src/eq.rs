//! An eight-band parametric EQ with low and high cuts, built on the same
//! zero-delay-feedback state-variable filters as the Multi filter, so a
//! band swept or boosted under automation never zips or blows up.
//!
//! - Low cut and high cut: Off, 12 or 24 dB per octave (Butterworth). A
//!   slope change crossfades over about 20 ms.
//! - Eight bands, each a bell, a low shelf, a high shelf or a notch, with
//!   frequency, gain (±18 dB; a notch ignores it) and Q. A band at 0 dB is
//!   exactly flat. A shelf's Q stops at 2, where it rings a 5 dB bump past
//!   the corner at full gain. A type change glides the band's gain to 0 dB,
//!   where every type passes the input exactly, swaps, and glides back, so
//!   it never clicks.
//! - Each band's Use: Unused (not on the EQ page), On, or Bypassed. Only
//!   On is heard; leaving On glides the band flat the same way. Bands 1 to
//!   4 start On and bands 5 to 8 Unused, so a project saved with the
//!   four-band EQ sounds and looks as it did: bands 5 to 8 and the Use
//!   params took new ids after Output, and the old ids are unchanged.
//! - Output gain.
//!
//! [`Eq::response_db`] gives the curve the settings are heading to, exact
//! to the filters, for drawing on an EQ page.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::{Segs, Svf, SvfCoefs, SvfMode};
use crate::{db_to_gain, defaults};

/// [`Eq`] parameter ids. Each band has four in a row: `band_type(b)`,
/// `freq(b)`, `gain(b)` and `q(b)` for band `b` from 0 to 7, plus its
/// `band_use(b)`. Bands 1 to 4 keep the ids of the four-band EQ (4 to 19);
/// bands 5 to 8 follow Output (21 to 36), then the eight Use params.
pub mod ids {
    pub const LOW_CUT: u16 = 0;
    pub const LOW_SLOPE: u16 = 1;
    pub const HIGH_CUT: u16 = 2;
    pub const HIGH_SLOPE: u16 = 3;
    /// How many bands.
    pub const BANDS: usize = 8;
    /// The first id of band `b`'s row of four.
    const fn base(b: usize) -> u16 {
        if b < 4 {
            4 + 4 * b as u16
        } else {
            21 + 4 * (b as u16 - 4)
        }
    }
    pub const fn band_type(b: usize) -> u16 {
        base(b)
    }
    pub const fn freq(b: usize) -> u16 {
        base(b) + 1
    }
    pub const fn gain(b: usize) -> u16 {
        base(b) + 2
    }
    pub const fn q(b: usize) -> u16 {
        base(b) + 3
    }
    /// A band's Use: Unused (not on the page), On (heard) or Bypassed.
    pub const USE_UNUSED: f32 = 0.0;
    pub const USE_ON: f32 = 1.0;
    pub const USE_BYPASSED: f32 = 2.0;
    /// Band `b`'s Use: [`USE_UNUSED`], [`USE_ON`] or [`USE_BYPASSED`].
    pub const fn band_use(b: usize) -> u16 {
        37 + b as u16
    }
    pub const OUTPUT: u16 = 20;
    /// Solo band (named Listen until 16 Nov): Off, or band `n` (1 to 8)
    /// heard alone through a band-pass at its frequency and Q, to find
    /// what it touches. Not saved with a sound: pages turn it off when
    /// they close.
    pub const LISTEN: u16 = 45;
    /// Quality: Normal, or HQ 2x, where the bands, the cuts and Solo band run
    /// at twice the rate between IIR half-bands, so a bell keeps its
    /// analog shape right up to 20 kHz.
    pub const QUALITY: u16 = 46;
    pub const NORMAL: f32 = 0.0;
    pub const HQ: f32 = 1.0;
    /// The band an id belongs to and which of its params it is: 0 type,
    /// 1 freq, 2 gain, 3 Q, 4 Use.
    pub const fn band_of(id: u16) -> Option<(usize, u16)> {
        match id {
            4..=19 => Some(((id as usize - 4) / 4, (id - 4) % 4)),
            21..=36 => Some((4 + (id as usize - 21) / 4, (id - 21) % 4)),
            37..=44 => Some((id as usize - 37, 4)),
            _ => None,
        }
    }
}

const SLOPES: &[&str] = &["Off", "12 dB", "24 dB"];
const TYPES: &[&str] = &["Bell", "Low shelf", "High shelf", "Notch"];
const USES: &[&str] = &["Unused", "On", "Bypassed"];
const LISTENS: &[&str] = &[
    "Off", "Band 1", "Band 2", "Band 3", "Band 4", "Band 5", "Band 6", "Band 7", "Band 8",
];
const QUALITIES: &[&str] = &["Normal", "HQ 2x"];
const N: usize = 47;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.eq",
    name: "EQ",
    kind: Kind::Effect,
};
static PARAMS: [ParamInfo; N] = [
    ParamInfo {
        id: 0,
        name: "Low cut",
        group: "Cuts",
        min: 20.0,
        max: 2_000.0,
        default: 80.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Low slope",
        group: "Cuts",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(SLOPES),
        readonly: false,
    },
    ParamInfo {
        id: 2,
        name: "High cut",
        group: "Cuts",
        min: 1_000.0,
        max: 20_000.0,
        default: 12_000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 3,
        name: "High slope",
        group: "Cuts",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(SLOPES),
        readonly: false,
    },
    ParamInfo {
        id: 4,
        name: "Type",
        group: "Band 1",
        min: 0.0,
        max: 3.0,
        default: 1.0,
        unit: Unit::None,
        curve: Curve::Choice(TYPES),
        readonly: false,
    },
    ParamInfo {
        id: 5,
        name: "Freq",
        group: "Band 1",
        min: 20.0,
        max: 20_000.0,
        default: 100.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 6,
        name: "Gain",
        group: "Band 1",
        min: -18.0,
        max: 18.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 7,
        name: "Q",
        group: "Band 1",
        min: 0.1,
        max: 18.0,
        default: 0.71,
        unit: Unit::None,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 8,
        name: "Type",
        group: "Band 2",
        min: 0.0,
        max: 3.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(TYPES),
        readonly: false,
    },
    ParamInfo {
        id: 9,
        name: "Freq",
        group: "Band 2",
        min: 20.0,
        max: 20_000.0,
        default: 400.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 10,
        name: "Gain",
        group: "Band 2",
        min: -18.0,
        max: 18.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 11,
        name: "Q",
        group: "Band 2",
        min: 0.1,
        max: 18.0,
        default: 0.71,
        unit: Unit::None,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 12,
        name: "Type",
        group: "Band 3",
        min: 0.0,
        max: 3.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(TYPES),
        readonly: false,
    },
    ParamInfo {
        id: 13,
        name: "Freq",
        group: "Band 3",
        min: 20.0,
        max: 20_000.0,
        default: 2_500.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 14,
        name: "Gain",
        group: "Band 3",
        min: -18.0,
        max: 18.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 15,
        name: "Q",
        group: "Band 3",
        min: 0.1,
        max: 18.0,
        default: 0.71,
        unit: Unit::None,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 16,
        name: "Type",
        group: "Band 4",
        min: 0.0,
        max: 3.0,
        default: 2.0,
        unit: Unit::None,
        curve: Curve::Choice(TYPES),
        readonly: false,
    },
    ParamInfo {
        id: 17,
        name: "Freq",
        group: "Band 4",
        min: 20.0,
        max: 20_000.0,
        default: 8_000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 18,
        name: "Gain",
        group: "Band 4",
        min: -18.0,
        max: 18.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 19,
        name: "Q",
        group: "Band 4",
        min: 0.1,
        max: 18.0,
        default: 0.71,
        unit: Unit::None,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 20,
        name: "Output",
        group: "Output",
        min: -18.0,
        max: 18.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 21,
        name: "Type",
        group: "Band 5",
        min: 0.0,
        max: 3.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(TYPES),
        readonly: false,
    },
    ParamInfo {
        id: 22,
        name: "Freq",
        group: "Band 5",
        min: 20.0,
        max: 20_000.0,
        default: 60.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 23,
        name: "Gain",
        group: "Band 5",
        min: -18.0,
        max: 18.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 24,
        name: "Q",
        group: "Band 5",
        min: 0.1,
        max: 18.0,
        default: 0.71,
        unit: Unit::None,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 25,
        name: "Type",
        group: "Band 6",
        min: 0.0,
        max: 3.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(TYPES),
        readonly: false,
    },
    ParamInfo {
        id: 26,
        name: "Freq",
        group: "Band 6",
        min: 20.0,
        max: 20_000.0,
        default: 250.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 27,
        name: "Gain",
        group: "Band 6",
        min: -18.0,
        max: 18.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 28,
        name: "Q",
        group: "Band 6",
        min: 0.1,
        max: 18.0,
        default: 0.71,
        unit: Unit::None,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 29,
        name: "Type",
        group: "Band 7",
        min: 0.0,
        max: 3.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(TYPES),
        readonly: false,
    },
    ParamInfo {
        id: 30,
        name: "Freq",
        group: "Band 7",
        min: 20.0,
        max: 20_000.0,
        default: 1_000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 31,
        name: "Gain",
        group: "Band 7",
        min: -18.0,
        max: 18.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 32,
        name: "Q",
        group: "Band 7",
        min: 0.1,
        max: 18.0,
        default: 0.71,
        unit: Unit::None,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 33,
        name: "Type",
        group: "Band 8",
        min: 0.0,
        max: 3.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(TYPES),
        readonly: false,
    },
    ParamInfo {
        id: 34,
        name: "Freq",
        group: "Band 8",
        min: 20.0,
        max: 20_000.0,
        default: 5_000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 35,
        name: "Gain",
        group: "Band 8",
        min: -18.0,
        max: 18.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 36,
        name: "Q",
        group: "Band 8",
        min: 0.1,
        max: 18.0,
        default: 0.71,
        unit: Unit::None,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 37,
        name: "Use",
        group: "Band 1",
        min: 0.0,
        max: 2.0,
        default: 1.0,
        unit: Unit::None,
        curve: Curve::Choice(USES),
        readonly: false,
    },
    ParamInfo {
        id: 38,
        name: "Use",
        group: "Band 2",
        min: 0.0,
        max: 2.0,
        default: 1.0,
        unit: Unit::None,
        curve: Curve::Choice(USES),
        readonly: false,
    },
    ParamInfo {
        id: 39,
        name: "Use",
        group: "Band 3",
        min: 0.0,
        max: 2.0,
        default: 1.0,
        unit: Unit::None,
        curve: Curve::Choice(USES),
        readonly: false,
    },
    ParamInfo {
        id: 40,
        name: "Use",
        group: "Band 4",
        min: 0.0,
        max: 2.0,
        default: 1.0,
        unit: Unit::None,
        curve: Curve::Choice(USES),
        readonly: false,
    },
    ParamInfo {
        id: 41,
        name: "Use",
        group: "Band 5",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(USES),
        readonly: false,
    },
    ParamInfo {
        id: 42,
        name: "Use",
        group: "Band 6",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(USES),
        readonly: false,
    },
    ParamInfo {
        id: 43,
        name: "Use",
        group: "Band 7",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(USES),
        readonly: false,
    },
    ParamInfo {
        id: 44,
        name: "Use",
        group: "Band 8",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(USES),
        readonly: false,
    },
    ParamInfo {
        id: ids::LISTEN,
        name: "Solo band",
        group: "Output",
        min: 0.0,
        max: 8.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(LISTENS),
        readonly: false,
    },
    ParamInfo {
        id: ids::QUALITY,
        name: "Quality",
        group: "Output",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(QUALITIES),
        readonly: false,
    },
];

/// Butterworth Qs: one 12 dB stage, or two stages for 24 dB.
const Q12: f32 = std::f32::consts::FRAC_1_SQRT_2;
const Q24: [f32; 2] = [0.541_196_1, 1.306_563];
/// The most Q a shelf takes: past this its resonant bump grows fast (a
/// +18 dB shelf at Q 18 peaks at +42 dB).
const SHELF_Q: f32 = 2.0;

/// The Q a band of type `kind` uses for the Q knob's `q`.
fn band_q(kind: usize, q: f32) -> f32 {
    if kind == 1 || kind == 2 {
        q.min(SHELF_Q)
    } else {
        q
    }
}

fn band_mode(kind: usize) -> SvfMode {
    match kind {
        1 => SvfMode::LowShelf,
        2 => SvfMode::HighShelf,
        3 => SvfMode::Notch,
        _ => SvfMode::Bell,
    }
}

/// While a band glides it is designed afresh (matched to the analog
/// curve, [`SvfCoefs::matched`]) every this many frames, and its filter
/// glides from one design to the next in between: a third of a
/// millisecond behind, for a sixteenth of the cost of a design a frame.
const DESIGN_EVERY: u32 = 16;

/// A notch: its type value. It has no gain; instead the band's output
/// crossfades from the input to the notch filter (`wet`), so it swaps type
/// and turns on and off as smoothly as a bell gliding to 0 dB.
const NOTCH: usize = 3;

/// One parametric band.
#[derive(Clone, Copy)]
struct Band {
    log_hz: Smooth,
    gain: Smooth,
    log_q: Smooth,
    /// The type playing and the type asked for; while they differ, or
    /// while the band is not On, the gain glides to 0 dB (`amt` to 0), then
    /// the type swaps.
    kind: usize,
    want: usize,
    on: bool,
    amt: f32,
    /// A notch's share of the output, 0 to 1 (the rest is the input).
    wet: f32,
    /// At 0 dB (a notch: at no share) the band is exactly the input: it
    /// rests, its state cleared.
    flat: bool,
    svf: Svf,
    /// The design playing, the next one, and the frames left to reach it.
    cur: SvfCoefs,
    to: SvfCoefs,
    left: u32,
}

impl Band {
    fn new(kind: usize, hz: f32, gain: f32, q: f32, on: bool) -> Band {
        Band {
            log_hz: Smooth::new(hz.log2()),
            gain: Smooth::new(gain),
            log_q: Smooth::new(q.log2()),
            kind,
            want: kind,
            on,
            amt: if on { 1.0 } else { 0.0 },
            wet: if on { 1.0 } else { 0.0 },
            flat: !on || (kind != NOTCH && gain == 0.0),
            svf: Svf::default(),
            cur: SvfCoefs::bilinear(SvfMode::Bell, 1000.0, 1.0, 0.0, 48_000),
            to: SvfCoefs::bilinear(SvfMode::Bell, 1000.0, 1.0, 0.0, 48_000),
            left: 0,
        }
    }
    /// Where `amt` rests: 1 while On, 0 while not.
    fn rest(&self) -> f32 {
        if self.on {
            1.0
        } else {
            0.0
        }
    }
    /// Sets the filter for this frame. `fresh` (after a reset or a rate
    /// change) takes the design for where the band is at once. `step` is
    /// the type-swap glide per frame.
    fn update(&mut self, rate: u32, fresh: bool, step: f32) {
        // An S-curve on the type-swap glide, so the gain eases in and out
        // instead of turning a corner.
        let ease = self.amt * self.amt * (3.0 - 2.0 * self.amt);
        let notch = self.kind == NOTCH;
        let db = if notch { 0.0 } else { self.gain.value * ease };
        self.wet = if notch { ease } else { 1.0 };
        let woke = self.flat || fresh;
        // Waking from rest it starts from silence, which only the gain (a
        // notch: its share), gliding up from nothing, lets through.
        self.flat = if notch { ease == 0.0 } else { db == 0.0 };
        if self.flat {
            self.svf.reset();
            self.left = 0;
            return;
        }
        // Waking (or fresh), it takes the design for where it is. Then, every
        // DESIGN_EVERY frames, a design for where it is now and a glide to
        // it; arriving, a last glide to the design for where it was heading
        // (the one the page draws), and it moves no more once it is there.
        if woke {
            self.cur = self.design(db, rate);
            self.left = 0;
        } else if self.left == 0 {
            // Designed for where the type-swap glide will be when the glide
            // between designs ends, so the filter keeps time with it and
            // is at 0 dB on the frame the band goes flat. (The smoothers
            // ease in, so the third of a millisecond behind does not show.)
            let ahead = if self.kind != self.want || !self.on {
                (self.amt - DESIGN_EVERY as f32 * step).max(0.0)
            } else {
                (self.amt + DESIGN_EVERY as f32 * step).min(1.0)
            };
            let db = if notch {
                0.0
            } else {
                self.gain.value * ahead * ahead * (3.0 - 2.0 * ahead)
            };
            let c = self.design(db, rate);
            if c != self.cur {
                self.to = c;
                self.left = DESIGN_EVERY;
            }
        }
        if self.left > 0 {
            self.left -= 1;
            self.cur = if self.left == 0 {
                self.to
            } else {
                self.cur.toward(self.to, 1.0 / (self.left + 1) as f32)
            };
        }
        self.svf.set_coefs(self.cur);
    }
    /// The analog-matched design for gain `db` and the band's type,
    /// frequency and Q as they are this frame.
    fn design(&self, db: f32, rate: u32) -> SvfCoefs {
        SvfCoefs::matched(
            band_mode(self.kind),
            self.log_hz.value.exp2(),
            band_q(self.kind, self.log_q.value.exp2()),
            db,
            rate,
        )
    }
    fn moving(&self) -> bool {
        self.left > 0
            || !(self.log_hz.settled()
                && self.gain.settled()
                && self.log_q.settled()
                && self.amt == self.rest()
                && self.kind == self.want)
    }
    /// One frame of glide: the smoothers, then the type swap.
    fn advance(&mut self, step: f32) {
        self.log_hz.next();
        self.gain.next();
        self.log_q.next();
        if self.kind != self.want || !self.on {
            self.amt = (self.amt - step).max(0.0);
            if self.amt <= 0.0 {
                // At 0 dB every type passes the input exactly, so the swap
                // is silent; whatever the old type left in the state fades
                // while the gain glides back up from nothing.
                self.amt = 0.0;
                self.kind = self.want;
            }
        } else if self.amt < 1.0 {
            self.amt = (self.amt + step).min(1.0);
        }
    }
    /// One sample of channel `ch` through the band (not called while flat).
    #[inline]
    fn run(&mut self, ch: usize, x: f32) -> f32 {
        let y = self.svf.run(ch, x);
        if self.kind == NOTCH {
            x + self.wet * (y - x)
        } else {
            y
        }
    }
    fn snap(&mut self) {
        self.log_hz.snap(self.log_hz.target);
        self.gain.snap(self.gain.target);
        self.log_q.snap(self.log_q.target);
        self.kind = self.want;
        self.amt = self.rest();
        self.svf.reset();
    }
}

/// A low or high cut: the output crossfades between the input and the 12
/// and 24 dB filters by weight. A filter at weight 0 rests with its state
/// cleared.
#[derive(Clone, Copy)]
struct Cut {
    mode: SvfMode,
    log_hz: Smooth,
    w12: Smooth,
    w24: Smooth,
    one: Svf,
    two: [Svf; 2],
}

impl Cut {
    fn new(mode: SvfMode, hz: f32) -> Cut {
        Cut {
            mode,
            log_hz: Smooth::new(hz.log2()),
            w12: Smooth::new(0.0),
            w24: Smooth::new(0.0),
            one: Svf::default(),
            two: [Svf::default(); 2],
        }
    }
    fn slope(&mut self, s: usize) {
        self.w12.target = if s == 1 { 1.0 } else { 0.0 };
        self.w24.target = if s == 2 { 1.0 } else { 0.0 };
    }
    fn update(&mut self, rate: u32) {
        let hz = self.log_hz.value.exp2();
        self.one.set(self.mode, hz, Q12, 0.0, rate);
        self.two[0].set(self.mode, hz, Q24[0], 0.0, rate);
        self.two[1].set(self.mode, hz, Q24[1], 0.0, rate);
    }
    /// Steps the weights one frame and clears a filter that rests.
    #[inline]
    fn weights(&mut self) -> (f32, f32) {
        let (w12, w24) = (self.w12.next(), self.w24.next());
        if w12 == 0.0 {
            self.one.reset();
        }
        if w24 == 0.0 {
            self.two[0].reset();
            self.two[1].reset();
        }
        (w12, w24)
    }
    #[inline]
    fn run(&mut self, ch: usize, x: f32, w12: f32, w24: f32) -> f32 {
        let mut y = (1.0 - w12 - w24) * x;
        if w12 != 0.0 {
            y += w12 * self.one.run(ch, x);
        }
        if w24 != 0.0 {
            let half = self.two[0].run(ch, x);
            y += w24 * self.two[1].run(ch, half);
        }
        y
    }
    fn snap(&mut self) {
        self.log_hz.snap(self.log_hz.target);
        self.w12.snap(self.w12.target);
        self.w24.snap(self.w24.target);
        self.one.reset();
        self.two[0].reset();
        self.two[1].reset();
    }
}

/// The EQ at one rate: the cuts, the bands, Solo band and the output gain.
/// [`Eq`] runs one at the host's rate and, for Quality HQ 2x, one at twice
/// it.
struct Core {
    v: [f32; N],
    rate: u32,
    /// It runs the cuts; the HQ core leaves them to [`Eq`], at the base
    /// rate.
    cuts: bool,
    low: Cut,
    high: Cut,
    bands: [Band; ids::BANDS],
    out: Smooth,
    /// The type-swap glide per frame (10 ms each way).
    step: f32,
    /// Solo band: the band heard (its index while Solo band is on, kept while
    /// fading out), the band-pass, and its share of the output, 0 to 1.
    listen_band: usize,
    listen: Svf,
    /// The band-pass peaks at Q; this brings its peak to unity.
    listen_k: f32,
    listen_mix: Smooth,
}

impl Core {
    fn new() -> Core {
        let v = defaults(&PARAMS);
        let band = |b: usize| {
            Band::new(
                v[ids::band_type(b) as usize] as usize,
                v[ids::freq(b) as usize],
                v[ids::gain(b) as usize],
                v[ids::q(b) as usize],
                v[ids::band_use(b) as usize] == ids::USE_ON,
            )
        };
        let mut eq = Core {
            v,
            rate: 48_000,
            cuts: true,
            low: Cut::new(SvfMode::HighPass, v[0]),
            high: Cut::new(SvfMode::LowPass, v[2]),
            bands: std::array::from_fn(band),
            out: Smooth::new(db_to_gain(v[20])),
            step: 0.0,
            listen_band: 0,
            listen: Svf::default(),
            listen_k: 1.0,
            listen_mix: Smooth::new(0.0),
        };
        eq.low.slope(v[1] as usize);
        eq.high.slope(v[3] as usize);
        eq.prepare(48_000);
        eq.reset();
        eq
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            ids::LOW_CUT => self.low.log_hz.target = v.log2(),
            ids::LOW_SLOPE if self.cuts => self.low.slope(v as usize),
            ids::HIGH_CUT => self.high.log_hz.target = v.log2(),
            ids::HIGH_SLOPE if self.cuts => self.high.slope(v as usize),
            ids::LOW_SLOPE | ids::HIGH_SLOPE => {}
            ids::OUTPUT => self.out.target = db_to_gain(v),
            ids::LISTEN => {
                let n = v as usize;
                if n > 0 {
                    self.listen_band = n - 1;
                }
                self.listen_mix.target = if n > 0 { 1.0 } else { 0.0 };
            }
            _ => {
                let Some((b, what)) = ids::band_of(id) else {
                    return;
                };
                let b = &mut self.bands[b];
                match what {
                    0 => b.want = v as usize,
                    1 => b.log_hz.target = v.log2(),
                    2 => b.gain.target = v,
                    3 => b.log_q.target = v.log2(),
                    _ => b.on = v == ids::USE_ON,
                }
            }
        }
    }

    fn update_all(&mut self) {
        self.low.update(self.rate);
        self.high.update(self.rate);
        for b in &mut self.bands {
            b.update(self.rate, true, self.step);
        }
    }

    /// Nothing glides: every smoother has reached its target and no band
    /// is changing type, so a frame needs no per-sample upkeep.
    fn still(&self) -> bool {
        let cut = |c: &Cut| c.log_hz.settled() && c.w12.settled() && c.w24.settled();
        cut(&self.low)
            && cut(&self.high)
            && self.out.settled()
            && self.bands.iter().all(|b| !b.moving())
            && self.listen_mix.settled()
            && self.listen_mix.value == 0.0
    }

    /// Points Solo band's band-pass at its band's frequency and Q as they
    /// are this frame.
    fn aim_listen(&mut self) {
        let b = &self.bands[self.listen_band];
        let hz = b.log_hz.value.exp2();
        let q = b.log_q.value.exp2();
        self.listen.set(SvfMode::BandPass, hz, q, 0.0, self.rate);
        self.listen_k = 1.0 / q;
    }

    /// [`Eq::run`] while [`Eq::still`]: the same output, sample for sample,
    /// without stepping settled smoothers (which would return their targets)
    /// or clearing filters that rest (which are already clear). With the
    /// cuts off, every band flat and the output at 0 dB, the EQ is exactly
    /// the input and the block is left as it is.
    fn run_still(&mut self, l: &mut [f32], r: &mut [f32]) {
        let (lo12, lo24) = (self.low.w12.value, self.low.w24.value);
        let (hi12, hi24) = (self.high.w12.value, self.high.w24.value);
        let g = self.out.value;
        if lo12 == 0.0
            && lo24 == 0.0
            && hi12 == 0.0
            && hi24 == 0.0
            && g == 1.0
            && self.bands.iter().all(|b| b.flat)
        {
            return;
        }
        // The bands that shape the sound, found once a block: the rest
        // rest, and nothing changes while still.
        // Filters in series commute, so the notches (which crossfade) run
        // after the rest, each list without a branch per sample.
        let (mut plain, mut notch) = ([0usize; ids::BANDS], [0usize; ids::BANDS]);
        let (mut np, mut nn) = (0, 0);
        for (k, b) in self.bands.iter().enumerate() {
            if b.flat {
                continue;
            }
            if b.kind == NOTCH {
                notch[nn] = k;
                nn += 1;
            } else {
                plain[np] = k;
                np += 1;
            }
        }
        let (plain, notch) = (&plain[..np], &notch[..nn]);
        for i in 0..l.len() {
            let mut y = [l[i], r[i]];
            for (ch, x) in y.iter_mut().enumerate() {
                let mut s = self.low.run(ch, *x, lo12, lo24);
                s = self.high.run(ch, s, hi12, hi24);
                for &k in plain {
                    s = self.bands[k].svf.run(ch, s);
                }
                for &k in notch {
                    s = self.bands[k].run(ch, s);
                }
                *x = g * s;
            }
            l[i] = y[0];
            r[i] = y[1];
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.still() {
            self.run_still(l, r)
        } else {
            self.run_gliding(l, r)
        }
    }

    /// The general frame loop: smoothers, type swaps and filter updates
    /// per sample, then the filters.
    fn run_gliding(&mut self, l: &mut [f32], r: &mut [f32]) {
        let rate = self.rate;
        for i in 0..l.len() {
            for cut in [&mut self.low, &mut self.high] {
                if !cut.log_hz.settled() {
                    cut.log_hz.next();
                    cut.update(rate);
                }
            }
            for b in &mut self.bands {
                if b.moving() {
                    b.advance(self.step);
                    b.update(rate, false, self.step);
                }
            }
            let (lo12, lo24) = self.low.weights();
            let (hi12, hi24) = self.high.weights();
            let g = self.out.next();
            let m = self.listen_mix.next();
            if m > 0.0 {
                self.aim_listen();
            } else {
                self.listen.reset();
            }
            let mut y = [l[i], r[i]];
            for (ch, x) in y.iter_mut().enumerate() {
                let dry = *x;
                let mut s = self.low.run(ch, dry, lo12, lo24);
                s = self.high.run(ch, s, hi12, hi24);
                // As the still loop: the notches after the rest.
                for b in &mut self.bands {
                    if !b.flat && b.kind != NOTCH {
                        s = b.run(ch, s);
                    }
                }
                for b in &mut self.bands {
                    if !b.flat && b.kind == NOTCH {
                        s = b.run(ch, s);
                    }
                }
                s *= g;
                if m > 0.0 {
                    s += m * (self.listen_k * self.listen.run(ch, dry) - s);
                }
                *x = s;
            }
            l[i] = y[0];
            r[i] = y[1];
        }
    }

    /// The gain band `b` is heading to, in dB: 0 unless it is On.
    fn target_db(&self, b: usize) -> f32 {
        if self.v[ids::band_use(b) as usize] != ids::USE_ON {
            return 0.0;
        }
        self.v[ids::gain(b) as usize]
    }

    /// Band `b` changes the sound once it gets where it is heading: it is
    /// On, and a notch or away from 0 dB.
    fn shapes(&self, b: usize) -> bool {
        self.v[ids::band_use(b) as usize] == ids::USE_ON
            && (self.v[ids::band_type(b) as usize] as usize == NOTCH
                || self.v[ids::gain(b) as usize] != 0.0)
    }

    /// The filters the EQ is heading to, at this core's rate, in series
    /// (the first `n`; the output gain is apart).
    fn stages(&self) -> ([Svf; 4 + ids::BANDS], usize) {
        let mut stages = [Svf::default(); 4 + ids::BANDS];
        let mut n = if self.cuts {
            self.cut_stages(&mut stages)
        } else {
            0
        };
        for b in 0..ids::BANDS {
            let gain = self.target_db(b);
            if self.shapes(b) {
                let kind = self.v[ids::band_type(b) as usize] as usize;
                stages[n].set_coefs(SvfCoefs::matched(
                    band_mode(kind),
                    self.v[ids::freq(b) as usize],
                    band_q(kind, self.v[ids::q(b) as usize]),
                    gain,
                    self.rate,
                ));
                n += 1;
            }
        }
        (stages, n)
    }

    /// The cuts the EQ is heading to, at this core's rate, written from
    /// the start of `stages`; how many.
    fn cut_stages(&self, stages: &mut [Svf]) -> usize {
        let mut n = 0;
        for (cut, f, s) in [
            (&self.low, self.v[0], self.v[1] as usize),
            (&self.high, self.v[2], self.v[3] as usize),
        ] {
            if s == 1 {
                stages[n].set(cut.mode, f, Q12, 0.0, self.rate);
                n += 1;
            } else if s == 2 {
                for q in Q24 {
                    stages[n].set(cut.mode, f, q, 0.0, self.rate);
                    n += 1;
                }
            }
        }
        n
    }

    /// See [`Eq::response_db`].
    fn response_db(&self, hz: &[f32], out: &mut [f32]) {
        let (stages, n) = self.stages();
        for (o, &f) in out.iter_mut().zip(hz) {
            let mut db = f64::from(self.v[ids::OUTPUT as usize]);
            for s in &stages[..n] {
                let (re, im) = s.response(f, self.rate);
                db += 10.0 * (re * re + im * im).max(1e-30).log10();
            }
            *o = db as f32;
        }
    }

    fn prepare(&mut self, rate: u32) {
        self.rate = rate.max(1);
        for cut in [&mut self.low, &mut self.high] {
            cut.log_hz.set_time(self.rate, 20.0);
            cut.w12.set_time(self.rate, 20.0);
            cut.w24.set_time(self.rate, 20.0);
        }
        for b in &mut self.bands {
            b.log_hz.set_time(self.rate, 20.0);
            b.gain.set_time(self.rate, 20.0);
            b.log_q.set_time(self.rate, 20.0);
        }
        self.out.set_time(self.rate, 20.0);
        self.listen_mix.set_time(self.rate, 20.0);
        self.step = 1.0 / (0.01 * self.rate as f32);
        self.update_all();
    }

    fn reset(&mut self) {
        self.low.snap();
        self.high.snap();
        for b in &mut self.bands {
            b.snap();
        }
        self.out.snap(self.out.target);
        self.listen_mix.snap(self.listen_mix.target);
        self.listen.reset();
        self.update_all();
    }

    /// How long the settings ring, seconds.
    fn tail_secs(&self) -> f32 {
        // A resonance rings for about Q / (pi f) seconds per 27 dB; allow 4x.
        let ring = |q: f32, hz: f32| 4.0 * q / (std::f32::consts::PI * hz.max(20.0));
        let mut secs = 0.0f32;
        if self.v[1] > 0.0 {
            secs = secs.max(ring(Q24[1], self.v[0]));
        }
        if self.v[3] > 0.0 {
            secs = secs.max(ring(Q24[1], self.v[2]));
        }
        for b in 0..ids::BANDS {
            if self.shapes(b) {
                let kind = self.v[ids::band_type(b) as usize] as usize;
                secs = secs.max(ring(
                    band_q(kind, self.v[ids::q(b) as usize]),
                    self.v[ids::freq(b) as usize],
                ));
            }
        }
        secs
    }
}

/// The half-bands round the HQ 2x core: four first-order all-pass
/// sections each way, flat to 20 kHz at 44.1 kHz (within 0.002 dB) and
/// 36 dB down at the image of 20 kHz. With a straight line inside, up then
/// down is one all-pass, about 1.6 frames late at low frequencies.
const HQ_SECTIONS: usize = 4;
const HQ_TBW: f64 = 0.06;
/// Frames the HQ core takes at a time (it gets twice as many).
const HQ_CHUNK: usize = 64;
/// Frames the half-bands ring for, to about -120 dB.
const HQ_TAIL: u32 = 128;

/// One first-order all-pass section, `y = a (x - y1) + x1`, its state
/// `[x1, y1]`.
#[inline(always)]
fn ap1(a: f32, s: &mut [f32; 2], x: f32) -> f32 {
    let y = crate::flush(a * (x - s[1]) + s[0]);
    *s = [x, y];
    y
}

/// One channel's half-bands round the HQ core: [`crate::oversample::IirUp2`]
/// and [`crate::oversample::IirDown2`] of four sections, unrolled.
/// Coefficients 0 and 2 make the even branch, 1 and 3 the odd one.
#[derive(Clone, Copy, Default)]
struct Halves {
    up: [[f32; 2]; HQ_SECTIONS],
    down: [[f32; 2]; HQ_SECTIONS],
}

impl Halves {
    /// One frame in, two out (in time order).
    #[inline(always)]
    fn up(&mut self, c: &[f32; HQ_SECTIONS], x: f32) -> [f32; 2] {
        let s = &mut self.up;
        let e = ap1(c[0], &mut s[0], x);
        let e = ap1(c[2], &mut s[2], e);
        let o = ap1(c[1], &mut s[1], x);
        let o = ap1(c[3], &mut s[3], o);
        [e, o]
    }
    /// Two frames in (in time order), one out.
    #[inline(always)]
    fn down(&mut self, c: &[f32; HQ_SECTIONS], a: f32, b: f32) -> f32 {
        let s = &mut self.down;
        let e = ap1(c[0], &mut s[0], b);
        let e = ap1(c[2], &mut s[2], e);
        let o = ap1(c[1], &mut s[1], a);
        let o = ap1(c[3], &mut s[3], o);
        0.5 * (e + o)
    }
}

/// An eight-band parametric EQ with low and high cuts and an output gain.
/// At Quality Normal it is one [`Core`] at the host's rate; at HQ 2x the
/// cuts, then the rest at twice the rate between IIR half-bands. A change of Quality
/// wakes the other core, lets it settle unheard, then crossfades the two
/// over 20 ms.
pub struct Eq {
    rate: u32,
    normal: Core,
    /// The bands, Solo band and the output gain at twice the rate; the cuts
    /// stay at the base rate (`low`, `high`, ahead of the half-bands):
    /// they have no curve to keep near Nyquist, and it saves a third.
    hq: Core,
    low: Cut,
    high: Cut,
    /// The HQ core's share of the output, 0 to 1.
    hq_mix: Smooth,
    /// Frames the core just woken still plays unheard, its filters
    /// settling on the input, before the crossfade starts.
    warm: u32,
    /// The half-bands' coefficients and each channel's sections.
    hb: [f32; HQ_SECTIONS],
    halves: [Halves; 2],
}

impl Default for Eq {
    fn default() -> Self {
        Self::new()
    }
}

impl Eq {
    pub fn new() -> Eq {
        let v = defaults(&PARAMS);
        let mut hq = Core::new();
        hq.cuts = false;
        hq.low.slope(0);
        hq.high.slope(0);
        let mut low = Cut::new(SvfMode::HighPass, v[ids::LOW_CUT as usize]);
        let mut high = Cut::new(SvfMode::LowPass, v[ids::HIGH_CUT as usize]);
        low.slope(v[ids::LOW_SLOPE as usize] as usize);
        high.slope(v[ids::HIGH_SLOPE as usize] as usize);
        let mut eq = Eq {
            rate: 48_000,
            normal: Core::new(),
            hq,
            low,
            high,
            hq_mix: Smooth::new(0.0),
            warm: 0,
            hb: {
                let c = crate::oversample::iir_halfband(HQ_SECTIONS, HQ_TBW);
                std::array::from_fn(|k| c[k])
            },
            halves: [Halves::default(); 2],
        };
        eq.prepare(48_000, mpfree_node::MAX_BLOCK);
        eq.reset();
        eq
    }

    fn apply(&mut self, id: u16, value: f32) {
        self.normal.apply(id, value);
        self.hq.apply(id, value);
        let v = self.normal.v.get(id as usize).copied().unwrap_or(0.0);
        match id {
            ids::LOW_CUT => self.low.log_hz.target = v.log2(),
            ids::LOW_SLOPE => self.low.slope(v as usize),
            ids::HIGH_CUT => self.high.log_hz.target = v.log2(),
            ids::HIGH_SLOPE => self.high.slope(v as usize),
            _ => {}
        }
        if id == ids::QUALITY {
            let to = if self.hq() { 1.0 } else { 0.0 };
            // A core that has been idle starts afresh, where the settings
            // are, and plays unheard until its filters have settled on the
            // input (as long as they ring, up to 100 ms); then the two
            // crossfade. While they crossfade both run.
            if self.hq_mix.settled() && self.warm == 0 && self.hq_mix.value != to {
                let ring = (self.normal.tail_secs() * self.rate as f32).ceil() as u32;
                self.warm = (ring + HQ_TAIL).min(self.rate / 10);
                if to == 1.0 {
                    self.hq.reset();
                    self.snap_cuts();
                    self.halves = [Halves::default(); 2];
                } else {
                    self.normal.reset();
                }
            }
            self.hq_mix.target = to;
        }
    }

    /// Quality is HQ 2x.
    fn hq(&self) -> bool {
        self.normal.v[ids::QUALITY as usize] >= 0.5
    }

    /// The HQ path's cuts where they are heading, their state cleared.
    fn snap_cuts(&mut self) {
        for cut in [&mut self.low, &mut self.high] {
            cut.snap();
            cut.update(self.rate);
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.warm == 0 && self.hq_mix.settled() && self.hq_mix.value == 0.0 {
            self.normal.run(l, r);
            return;
        }
        let mut at = 0;
        while at < l.len() {
            let end = (at + HQ_CHUNK).min(l.len());
            self.run_hq(&mut l[at..end], &mut r[at..end]);
            at = end;
        }
    }

    /// Up to [`HQ_CHUNK`] frames through the HQ core, crossfaded with the
    /// normal one while Quality changes.
    fn run_hq(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n = l.len();
        let (mut ul, mut ur) = ([0.0f32; 2 * HQ_CHUNK], [0.0f32; 2 * HQ_CHUNK]);
        let still = |c: &Cut| c.log_hz.settled() && c.w12.settled() && c.w24.settled();
        if still(&self.low) && still(&self.high) {
            // As the core's still loop: nothing to step, and a cut that is
            // off is skipped.
            let (lo, hi) = (&mut self.low, &mut self.high);
            let (lo12, lo24) = (lo.w12.value, lo.w24.value);
            let (hi12, hi24) = (hi.w12.value, hi.w24.value);
            let (lo_on, hi_on) = (lo12 + lo24 != 0.0, hi12 + hi24 != 0.0);
            for i in 0..n {
                let mut x = [l[i], r[i]];
                for (ch, s) in x.iter_mut().enumerate() {
                    if lo_on {
                        *s = lo.run(ch, *s, lo12, lo24);
                    }
                    if hi_on {
                        *s = hi.run(ch, *s, hi12, hi24);
                    }
                }
                [ul[2 * i], ul[2 * i + 1]] = self.halves[0].up(&self.hb, x[0]);
                [ur[2 * i], ur[2 * i + 1]] = self.halves[1].up(&self.hb, x[1]);
            }
        } else {
            let rate = self.rate;
            for i in 0..n {
                for cut in [&mut self.low, &mut self.high] {
                    if !cut.log_hz.settled() {
                        cut.log_hz.next();
                        cut.update(rate);
                    }
                }
                let (lo12, lo24) = self.low.weights();
                let (hi12, hi24) = self.high.weights();
                let mut x = [l[i], r[i]];
                for (ch, s) in x.iter_mut().enumerate() {
                    *s = self.low.run(ch, *s, lo12, lo24);
                    *s = self.high.run(ch, *s, hi12, hi24);
                }
                [ul[2 * i], ul[2 * i + 1]] = self.halves[0].up(&self.hb, x[0]);
                [ur[2 * i], ur[2 * i + 1]] = self.halves[1].up(&self.hb, x[1]);
            }
        }
        self.hq.run(&mut ul[..2 * n], &mut ur[..2 * n]);
        if self.warm == 0 && self.hq_mix.settled() && self.hq_mix.value == 1.0 {
            for i in 0..n {
                l[i] = self.halves[0].down(&self.hb, ul[2 * i], ul[2 * i + 1]);
                r[i] = self.halves[1].down(&self.hb, ur[2 * i], ur[2 * i + 1]);
            }
            return;
        }
        let (mut nl, mut nr) = ([0.0f32; HQ_CHUNK], [0.0f32; HQ_CHUNK]);
        nl[..n].copy_from_slice(l);
        nr[..n].copy_from_slice(r);
        self.normal.run(&mut nl[..n], &mut nr[..n]);
        for i in 0..n {
            let m = if self.warm > 0 {
                self.warm -= 1;
                self.hq_mix.value
            } else {
                self.hq_mix.next()
            };
            let hl = self.halves[0].down(&self.hb, ul[2 * i], ul[2 * i + 1]);
            let hr = self.halves[1].down(&self.hb, ur[2 * i], ur[2 * i + 1]);
            l[i] = crate::lerp(nl[i], hl, m);
            r[i] = crate::lerp(nr[i], hr, m);
        }
    }

    /// The response in dB at each frequency of `hz` (written to `out`),
    /// for the settings the EQ is heading to: what an EQ page draws. Exact
    /// to the filters, cuts and output gain included, and at HQ 2x to the
    /// half-bands round them.
    pub fn response_db(&self, hz: &[f32], out: &mut [f32]) {
        if !self.hq() {
            return self.normal.response_db(hz, out);
        }
        // The cuts at the base rate, then the rest: an up-sampler, a
        // filter and a down-sampler by the same factor make one filter at
        // the base rate. With the half-bands' even
        // branch `e` and odd branch `o` (all-passes at the base rate) and
        // the fast filter's polyphase parts `h0` and `h1`, it is
        // `h0 e o + h1 (e^2 + z^-1 o^2) / 2`; with a straight line inside,
        // `e o`.
        let (stages, n) = self.hq.stages();
        let mut cuts = [Svf::default(); 4];
        let nc = self.normal.cut_stages(&mut cuts);
        let fast = self.hq.rate;
        let base = f64::from(self.rate);
        let c = self.hb;
        let at = |f: f64| {
            stages[..n]
                .iter()
                .fold((1.0, 0.0), |h, s| cmul(h, s.response(f as f32, fast)))
        };
        for (o, &f) in out.iter_mut().zip(hz) {
            let f = f64::from(f);
            let w = std::f64::consts::TAU * f / base;
            let zi = (w.cos(), -w.sin());
            let (mut e, mut od) = ((1.0, 0.0), (1.0, 0.0));
            for (k, &a) in c.iter().enumerate() {
                let a = f64::from(a);
                // (a + z^-1) / (1 + a z^-1)
                let s = cdiv((a + zi.0, zi.1), (1.0 + a * zi.0, a * zi.1));
                if k % 2 == 0 {
                    e = cmul(e, s);
                } else {
                    od = cmul(od, s);
                }
            }
            // The fast filter at z2 = e^{jw/2}, and at -z2 (f + base, the
            // mirror of base - f).
            let hp = at(f);
            let hm = {
                let (re, im) = at(base - f);
                (re, -im)
            };
            let h0 = (0.5 * (hp.0 + hm.0), 0.5 * (hp.1 + hm.1));
            let z2 = ((0.5 * w).cos(), (0.5 * w).sin());
            let h1 = cmul((0.5 * (hp.0 - hm.0), 0.5 * (hp.1 - hm.1)), z2);
            let ee = cmul(e, e);
            let oo = cmul(zi, cmul(od, od));
            let a = cmul(h0, cmul(e, od));
            let b = cmul(h1, (ee.0 + oo.0, ee.1 + oo.1));
            let (re, im) = (a.0 + 0.5 * b.0, a.1 + 0.5 * b.1);
            let mut db = f64::from(self.normal.v[ids::OUTPUT as usize])
                + 10.0 * (re * re + im * im).max(1e-30).log10();
            for s in &cuts[..nc] {
                let (re, im) = s.response(f as f32, self.rate);
                db += 10.0 * (re * re + im * im).max(1e-30).log10();
            }
            *o = db as f32;
        }
    }
}

fn cmul(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}

fn cdiv(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let d = b.0 * b.0 + b.1 * b.1;
    ((a.0 * b.0 + a.1 * b.1) / d, (a.1 * b.0 - a.0 * b.1) / d)
}

impl Node for Eq {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        self.apply(id, value);
    }
    fn param(&self, id: u16) -> f32 {
        self.normal.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(1);
        self.normal.prepare(self.rate);
        self.hq.prepare(2 * self.rate);
        self.hq_mix.set_time(self.rate, 20.0);
        for cut in [&mut self.low, &mut self.high] {
            cut.log_hz.set_time(self.rate, 20.0);
            cut.w12.set_time(self.rate, 20.0);
            cut.w24.set_time(self.rate, 20.0);
            cut.update(self.rate);
        }
    }
    fn reset(&mut self) {
        self.normal.reset();
        self.hq.reset();
        self.snap_cuts();
        self.hq_mix.snap(self.hq_mix.target);
        self.warm = 0;
        self.halves = [Halves::default(); 2];
    }
    fn process(&mut self, _ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| {
            if let NodeEvent::Param { id, value } = *ev {
                self.apply(id, value)
            }
        }) {
            self.run(&mut left[from..to], &mut right[from..to]);
        }
    }
    fn rest_frames(&self) -> u32 {
        // What it holds is its tail: once rung out it is as good as reset.
        self.tail_frames()
    }
    fn tail_frames(&self) -> u32 {
        let ring = (self.normal.tail_secs() * self.rate as f32).ceil() as u32;
        if self.hq() || !self.hq_mix.settled() || self.warm > 0 {
            ring + HQ_TAIL
        } else {
            ring
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut rng = crate::Rng::new(seed);
        (0..n).map(|_| 0.5 * rng.bipolar()).collect()
    }

    /// The unrolled half-bands are the oversampler's IIR pair.
    #[test]
    fn the_halves_are_the_iir_pair() {
        use crate::oversample::{IirDown2, IirUp2};
        let eq = Eq::new();
        let mut h = Halves::default();
        let (mut up, mut down) = (
            IirUp2::new(HQ_SECTIONS, HQ_TBW),
            IirDown2::new(HQ_SECTIONS, HQ_TBW),
        );
        for (i, &x) in noise(4000, 9).iter().enumerate() {
            let a = h.up(&eq.hb, x);
            let b = up.run(x);
            assert!(
                (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6,
                "{i}"
            );
            let (y, z) = (
                h.down(&eq.hb, a[0], a[1] * 0.5),
                down.run([b[0], b[1] * 0.5]),
            );
            assert!((y - z).abs() < 1e-6, "{i}: {y} {z}");
        }
    }

    /// Once nothing glides, the lean loop gives the general loop's output
    /// bit for bit, and an EQ that does nothing leaves the block as it is.
    #[test]
    fn the_still_loop_matches_the_general_one() {
        let settings: [&[(u16, f32)]; 5] = [
            &[],
            &[(1, 1.0), (3, 2.0)],
            &[
                (1, 2.0),
                (ids::gain(0), 6.0),
                (ids::gain(2), -4.5),
                (ids::q(2), 3.0),
                (ids::OUTPUT, -2.0),
            ],
            &[
                (ids::band_type(3), 1.0),
                (ids::gain(3), 3.0),
                (ids::freq(1), 700.0),
                (ids::gain(1), 9.0),
            ],
            &[
                (ids::band_use(5), ids::USE_ON),
                (ids::band_type(5), 3.0),
                (ids::freq(5), 3000.0),
                (ids::q(5), 6.0),
                (ids::band_use(6), ids::USE_ON),
                (ids::gain(6), -5.0),
                (ids::band_use(0), ids::USE_BYPASSED),
                (ids::gain(0), 9.0),
            ],
        ];
        for (n, set) in settings.iter().enumerate() {
            let mut a = Core::new();
            let mut b = Core::new();
            for e in [&mut a, &mut b] {
                e.prepare(48_000);
                for &(id, v) in *set {
                    e.apply(id, v);
                }
                // Settle every glide and type swap.
                let (mut l, mut r) = (noise(48_000, 3), noise(48_000, 4));
                e.run(&mut l, &mut r);
                assert!(e.still(), "setting {n} settles");
            }
            let (mut la, mut ra) = (noise(4096, 5), noise(4096, 6));
            let (mut lb, mut rb) = (la.clone(), ra.clone());
            let (l0, r0) = (la.clone(), ra.clone());
            a.run_still(&mut la, &mut ra);
            b.run_gliding(&mut lb, &mut rb);
            assert!(
                la.iter()
                    .zip(&lb)
                    .chain(ra.iter().zip(&rb))
                    .all(|(x, y)| x.to_bits() == y.to_bits()),
                "setting {n}"
            );
            if n == 0 {
                assert_eq!(
                    (la, ra),
                    (l0, r0),
                    "the default EQ passes the input untouched"
                );
            }
        }
    }
}
