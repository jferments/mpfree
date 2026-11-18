//! The [`Vocoder`]: a voice's spectrum laid on another sound.
//!
//! The voice (the modulator) is split into 4 to 40 bands spaced evenly in
//! pitch between Low and High; each band's level sets the level of the
//! same band of the carrier, so the carrier seems to speak. The carrier is
//! a saw that follows the voice's pitch (the robot voice, from a voice
//! strip alone), noise (a whisper), or the effect's input, with the voice
//! on the strip's sidechain key (a synth pad keyed by a voice strip).
//!
//! - **Bands** are 4th-order band-passes (two identical trapezoidal
//!   state-variable stages, each unity at its peak), all with the same Q,
//!   so they cross 3 dB down at Bandwidth 100 %. The carrier's bands are
//!   summed with alternating signs: a band's phase leads its neighbour's
//!   by about 180 degrees where they cross, so same-sign bands cut a notch
//!   10 to 40 dB deep between every pair, and alternating ones sum to
//!   within about 1.5 dB of flat (mpfree-claude
//!   `fx/quality/tools/vocoder_bank.py`).
//! - **Envelopes** follow each band's power, the band-pass squared plus a
//!   partner in quadrature with it (half the low-pass less the high-pass
//!   of the second stage, 90 degrees from the band-pass at every
//!   frequency and within 1 % of its size across the band). A tone in the
//!   band then reads a steady level instead of the rectified ripple at
//!   twice its pitch, so low bands don't buzz and nothing new lands two
//!   octaves up (FX quality V6's clicks).
//! - **Level**: each band's gain is the square root of the range in
//!   octaves times the bands' Q, so the output stays put as Bands, Low,
//!   High and Bandwidth move, and a voice at -18 dBFS RMS gives back about
//!   the carrier's loudness (V6).
//! - **Unvoiced**: the voice above 4 kHz is mixed in, so an "s" or a "t"
//!   is heard over a carrier with little there.
//! - **Gate**: below its threshold the output fades out, so a mic's hiss
//!   doesn't open the bands. Fully down it is off.
//!
//! Low, High, Formant, Bandwidth and Spread glide; their coefficients are
//! retuned every [`CTRL`] frames and ramp between, as the talk box's.
//! Carrier and Bands change the bands' layout: the output dips for a few
//! milliseconds while it changes. See `docs/dev/design/fx-voice.md`.

use std::f32::consts::{FRAC_PI_4, PI, SQRT_2};

use mpfree_node::{Ctx, Curve, Kind, NodeInfo, ParamInfo, Smooth, Unit};

use crate::kit::{
    self, meter, node_impl, param, Choice, Db, Glide, Hz, Linear, Log, Ms, Percent, NONE,
};
use crate::motion::saw;
use crate::vocal::{Tracker, VOICE_BANDS};
use crate::{defaults, fast_exp2, fast_log2, flush, lerp, Preset, Rng};

/// Frames between retunes.
const CTRL: usize = 16;
/// The most bands.
const MAX_BANDS: usize = 40;
/// Bands are run four at a time.
const W: usize = 4;
const CHUNKS: usize = MAX_BANDS / W;
type L = [f32; W];

/// [`Vocoder`] parameter ids.
pub mod ids {
    pub const CARRIER: u16 = 0;
    pub const PITCH: u16 = 1;
    pub const BANDS: u16 = 2;
    pub const LOW: u16 = 3;
    pub const HIGH: u16 = 4;
    pub const FORMANT: u16 = 5;
    pub const BANDWIDTH: u16 = 6;
    pub const ATTACK: u16 = 7;
    pub const RELEASE: u16 = 8;
    pub const GATE: u16 = 9;
    pub const UNVOICED: u16 = 10;
    pub const SPREAD: u16 = 11;
    pub const MIX: u16 = 12;
    /// The voice's level, a meter.
    pub const VOICE: u16 = 13;
}

/// The carriers: a saw that follows the voice's pitch, noise, or the
/// effect's input (the voice is then the sidechain key).
pub const CARRIERS: &[&str] = &["Saw", "Noise", "Input"];
const SAW: usize = 0;
const NOISE: usize = 1;
const INPUT: usize = 2;

/// The hint line's words for each carrier.
pub(crate) const CARRIER_HINTS: [&str; 3] = [
    "Saw: a saw that follows the voice's pitch (the voice is this track)",
    "Noise: a whisper (the voice is this track)",
    "Input: this track, voiced by its sidechain key",
];

/// The voice level that gives back the carrier's: -18 dBFS RMS, a sung
/// or spoken voice's usual level (V6). The internal carriers play at it.
const NOMINAL: f32 = 0.125_9;
/// The bands' gain: this times the square root of the range in octaves
/// times their Q, over [`NOMINAL`] and the square root of the bank's
/// summed power ([`sum_power`]). A band passes about 1.13 / Q octaves of
/// the voice (a 4th-order band-pass's noise bandwidth), so its envelope
/// goes as the square root of that, and the carrier comes through the
/// bank as its summed power has it. The theory's constant is 0.66; this
/// is fitted so the sung vowels at -18 dBFS on a saw come out within
/// 1.5 LU of the saw's loudness (V6).
const LEVEL: f32 = 0.607;
/// The bank's summed power: the mean over the range of the alternating
/// sum of the bands' responses squared (the carrier through the bank with
/// every band at unity), against Bandwidth, at log2 of its ratio from -2
/// (25 %) to 0.6 (150 %) in tenths. It hardly depends on the bands'
/// number or range (within 1 % from 8 to 40 bands); narrow bands leave
/// gaps and wide ones cancel where they overlap. From mpfree-claude
/// `fx/quality/tools/vocoder_bank.py`.
const SUM_POWER: [f32; 27] = [
    0.3286, 0.3580, 0.3911, 0.4284, 0.4702, 0.5170, 0.5692, 0.6267, 0.6895, 0.7566, 0.8271, 0.8990,
    0.9699, 1.0367, 1.0958, 1.1434, 1.1757, 1.1890, 1.1808, 1.1495, 1.0948, 1.0182, 0.9229, 0.8134,
    0.6954, 0.5750, 0.4586,
];
/// Each stage's Q is this over the bands' spacing (in the ratio's half
/// powers): the stages are 1.5 dB down halfway to the next band, so the
/// cascade crosses at 3 dB, where the alternating sum is flattest.
const CROSS: f32 = 0.643_6;
/// The narrowest range, in octaves (Low at 1 kHz and High at 1 kHz).
const MIN_SPAN: f32 = 1.0;
/// Where the voice's highs start (the Unvoiced path), Hz.
const UNVOICED_HZ: f32 = 4000.0;
/// How long Low, High, Formant, Bandwidth and Spread glide, in ms.
const GLIDE_MS: f32 = 20.0;
/// How long the output takes to dip out (and back) while the bands'
/// layout changes, in ms.
const DIP_MS: f32 = 6.0;
/// The gate opens at its threshold and closes 6 dB under it (powers).
const GATE_HYST: f32 = 0.25;

/// [`SUM_POWER`] at Bandwidth `b` (log2 of its ratio).
fn sum_power(b: f32) -> f32 {
    let x = ((b + 2.0) * 10.0).clamp(0.0, 26.0);
    let i = (x as usize).min(25);
    lerp(SUM_POWER[i], SUM_POWER[i + 1], x - i as f32)
}

/// `a1`, `a2` and `a3` of a trapezoidal state-variable stage peaking at
/// `hz` with damping `k` (1 / Q).
fn coefs(hz: f32, k: f32, rate: f32) -> [f32; 3] {
    let g = (PI * hz.clamp(10.0, 0.45 * rate) / rate).tan();
    let a1 = 1.0 / (1.0 + g * (g + k));
    let a2 = g * a1;
    [a1, a2, g * a2]
}

/// One trapezoidal state-variable stage: `(band, low)` for input `x`.
#[inline(always)]
fn stage(x: f32, a1: f32, a2: f32, a3: f32, s1: &mut f32, s2: &mut f32) -> (f32, f32) {
    let v3 = x - *s2;
    let v1 = a1 * *s1 + a2 * v3;
    let v2 = *s2 + a2 * *s1 + a3 * v3;
    *s1 = 2.0 * v1 - *s1;
    *s2 = 2.0 * v2 - *s2;
    (v1, v2)
}

/// A bank of up to [`MAX_BANDS`] 4th-order band-passes, four at a time:
/// each band's coefficients (both its stages share them), their steps a
/// frame while gliding, and the two stages' states.
#[derive(Clone, Copy, Debug, Default)]
struct Bank {
    a: [[L; CHUNKS]; 3],
    d: [[L; CHUNKS]; 3],
    s: [[L; CHUNKS]; 4],
}

impl Bank {
    fn set(&mut self, i: usize, c: [f32; 3]) {
        for (n, v) in c.into_iter().enumerate() {
            self.a[n][i / W][i % W] = v;
            self.d[n][i / W][i % W] = 0.0;
        }
    }
    fn aim(&mut self, i: usize, c: [f32; 3], per: f32) {
        for (n, v) in c.into_iter().enumerate() {
            self.d[n][i / W][i % W] = (v - self.a[n][i / W][i % W]) * per;
        }
    }
    fn hold(&mut self) {
        self.d = [[[0.0; W]; CHUNKS]; 3];
    }
    #[inline]
    fn advance(&mut self, chunks: usize) {
        for n in 0..3 {
            for c in 0..chunks {
                for j in 0..W {
                    self.a[n][c][j] += self.d[n][c][j];
                }
            }
        }
    }
    fn reset(&mut self) {
        self.s = [[[0.0; W]; CHUNKS]; 4];
    }
    fn flush(&mut self, chunks: usize) {
        for st in &mut self.s {
            for c in st.iter_mut().take(chunks) {
                for v in c {
                    *v = flush(*v);
                }
            }
        }
    }
}

/// A fixed 2nd-order high-pass stage (for the Unvoiced path).
#[derive(Clone, Copy, Debug, Default)]
struct HighPass {
    k: f32,
    a: [f32; 3],
    s1: f32,
    s2: f32,
}

impl HighPass {
    fn set(&mut self, hz: f32, q: f32, rate: u32) {
        self.k = 1.0 / q;
        self.a = coefs(hz, self.k, rate as f32);
    }
    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let (b, l) = stage(
            x,
            self.a[0],
            self.a[1],
            self.a[2],
            &mut self.s1,
            &mut self.s2,
        );
        x - self.k * b - l
    }
    fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

static VOCODER_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.vocoder",
    name: "Vocoder",
    kind: Kind::Effect,
};
static VOCODER_PARAMS: [ParamInfo; 14] = [
    param(
        0,
        "Carrier",
        "Carrier",
        0.0,
        2.0,
        0.0,
        NONE,
        Choice(CARRIERS),
    ),
    param(
        1,
        "Pitch",
        "Carrier",
        -24.0,
        24.0,
        0.0,
        Unit::Semitones,
        Curve::Stepped,
    ),
    param(2, "Bands", "Bands", 4.0, 40.0, 16.0, NONE, Curve::Stepped),
    param(3, "Low", "Bands", 50.0, 1000.0, 100.0, Hz, Log),
    param(4, "High", "Bands", 1000.0, 16000.0, 8000.0, Hz, Log),
    param(
        5,
        "Formant",
        "Bands",
        -12.0,
        12.0,
        0.0,
        Unit::Semitones,
        Linear,
    ),
    param(6, "Bandwidth", "Bands", 25.0, 150.0, 100.0, Percent, Log),
    param(7, "Attack", "Voice", 0.5, 100.0, 3.0, Ms, Log),
    param(8, "Release", "Voice", 5.0, 1000.0, 40.0, Ms, Log),
    param(9, "Gate", "Voice", -90.0, -20.0, -70.0, Db, Linear),
    param(10, "Unvoiced", "Voice", 0.0, 100.0, 70.0, Percent, Linear),
    param(11, "Spread", "Output", 0.0, 100.0, 0.0, Percent, Linear),
    param(12, "Mix", "Output", 0.0, 100.0, 100.0, Percent, Linear),
    meter(13, "Voice level", "Voice", -60.0, 0.0, Db),
];

/// Vocoder factory presets. All play from the strip's own sound (no
/// sidechain key needed).
pub static VOCODER_PRESETS: &[Preset] = &[
    Preset {
        name: "Robot voice",
        values: &[(ids::BANDS, 12.0), (ids::RELEASE, 30.0)],
    },
    Preset {
        name: "Deep robot",
        values: &[
            (ids::PITCH, -12.0),
            (ids::LOW, 50.0),
            (ids::FORMANT, -3.0),
            (ids::RELEASE, 50.0),
        ],
    },
    Preset {
        name: "Whisper",
        values: &[
            (ids::CARRIER, 1.0),
            (ids::BANDS, 24.0),
            (ids::RELEASE, 60.0),
            (ids::UNVOICED, 100.0),
        ],
    },
    Preset {
        name: "Vintage bands",
        values: &[
            (ids::BANDS, 8.0),
            (ids::LOW, 120.0),
            (ids::HIGH, 5000.0),
            (ids::BANDWIDTH, 140.0),
        ],
    },
    Preset {
        name: "Wide choir",
        values: &[
            (ids::BANDS, 24.0),
            (ids::RELEASE, 90.0),
            (ids::SPREAD, 80.0),
        ],
    },
];

/// A voice laid on a saw, noise or the strip's own sound.
pub struct Vocoder {
    v: [f32; 14],
    rate: u32,
    /// The voice's bands and the carrier's.
    ana: Bank,
    syn: Bank,
    /// The bands' damping (1 / Q, the same for every band of both banks)
    /// and its step a frame.
    k: f32,
    dk: f32,
    /// Each band's power envelope.
    env: [L; CHUNKS],
    /// Each band's output gain into the left and the right (sign, level,
    /// spread and the fade near the top), and their steps a frame.
    gl: [L; CHUNKS],
    gr: [L; CHUNKS],
    dgl: [L; CHUNKS],
    dgr: [L; CHUNKS],
    /// The power followers' coefficients (attack, release).
    att: f32,
    rel: f32,
    /// The bands and the carrier in use, and how many four-band chunks.
    n: usize,
    carrier: usize,
    chunks: usize,
    /// Low and High in log2 Hz, Formant in octaves, Bandwidth in log2 of
    /// its ratio, Spread 0 to 1: gliding a step a retune.
    low: Smooth,
    high: Smooth,
    shift: Smooth,
    width: Smooth,
    spread: Smooth,
    mix: Smooth,
    unvoiced: Smooth,
    ctrl: usize,
    /// The glides moved last retune (one more retune lands them).
    dirty: bool,
    ramping: bool,
    /// The dip while the layout changes: 1 open, 0 at the bottom.
    dip: f32,
    dip_step: f32,
    pending: bool,
    /// The saw: its phase, its step a frame and that step's step, and
    /// the voice's pitch in Hz (110 until one is found).
    phase: f32,
    dt: f32,
    ddt: f32,
    voice_hz: f32,
    pitch: Smooth,
    tracker: Tracker,
    rng: Rng,
    /// The voice's highs above 4 kHz.
    hp: [HighPass; 2],
    /// The gate: the voice's power, whether it is open, its gain, and
    /// their coefficients and thresholds.
    gate_env: f32,
    open: bool,
    gate: f32,
    gate_att: f32,
    gate_rel: f32,
    gate_up: f32,
    gate_down: f32,
    gate_on: f32,
    /// The voice level meter's power and its release.
    level: f32,
    level_rel: f32,
}

impl Default for Vocoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Vocoder {
    pub fn new() -> Vocoder {
        let v = defaults(&VOCODER_PARAMS);
        let mut t = Vocoder {
            v,
            rate: 48_000,
            ana: Bank::default(),
            syn: Bank::default(),
            k: 1.0,
            dk: 0.0,
            env: [[0.0; W]; CHUNKS],
            gl: [[0.0; W]; CHUNKS],
            gr: [[0.0; W]; CHUNKS],
            dgl: [[0.0; W]; CHUNKS],
            dgr: [[0.0; W]; CHUNKS],
            att: 1.0,
            rel: 1.0,
            n: 16,
            carrier: SAW,
            chunks: 4,
            low: Smooth::new(fast_log2(v[3])),
            high: Smooth::new(fast_log2(v[4])),
            shift: Smooth::new(v[5] / 12.0),
            width: Smooth::new(fast_log2(v[6] / 100.0)),
            spread: Smooth::new(v[11] / 100.0),
            mix: Smooth::new(v[12] / 100.0),
            unvoiced: Smooth::new(v[10] / 100.0),
            ctrl: 0,
            dirty: false,
            ramping: false,
            dip: 1.0,
            dip_step: 0.01,
            pending: false,
            phase: 0.0,
            dt: 0.0,
            ddt: 0.0,
            voice_hz: 110.0,
            pitch: Smooth::new(0.0),
            tracker: Tracker::new(&VOICE_BANDS),
            rng: Rng::new(0x5EED_70C0),
            hp: [HighPass::default(); 2],
            gate_env: 0.0,
            open: false,
            gate: 0.0,
            gate_att: 1.0,
            gate_rel: 1.0,
            gate_up: 1.0,
            gate_down: 1.0,
            gate_on: 0.0,
            level: 0.0,
            level_rel: 1.0,
        };
        t.setup(48_000);
        t.clear();
        t
    }

    fn apply(&mut self, id: u16, v: f32) {
        match id {
            ids::CARRIER | ids::BANDS => {
                if v as usize
                    != if id == ids::CARRIER {
                        self.carrier
                    } else {
                        self.n
                    }
                {
                    self.pending = true;
                }
            }
            ids::LOW => self.low.target = fast_log2(v),
            ids::HIGH => self.high.target = fast_log2(v),
            ids::FORMANT => self.shift.target = v / 12.0,
            ids::BANDWIDTH => self.width.target = fast_log2(v / 100.0),
            ids::ATTACK | ids::RELEASE => self.followers(),
            ids::GATE => self.gate_on = gate_power(v),
            ids::UNVOICED => self.unvoiced.target = v / 100.0,
            ids::SPREAD => self.spread.target = v / 100.0,
            ids::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }

    /// The power followers' coefficients: a power envelope's time constant
    /// is half its level's, so Attack and Release read as the level's.
    fn followers(&mut self) {
        self.att = kit::coef(0.5 * self.v[ids::ATTACK as usize], self.rate);
        self.rel = kit::coef(0.5 * self.v[ids::RELEASE as usize], self.rate);
    }

    fn begin(&mut self, _ctx: &Ctx) {}

    /// Sets every band where the controls are now: at once (`snap`), or
    /// ramping there over the next [`CTRL`] frames.
    fn aim(&mut self, snap: bool) {
        let rate = self.rate as f32;
        let n = self.n;
        let lo = self.low.value;
        let span = (self.high.value - lo).max(MIN_SPAN);
        let step = span / (n - 1) as f32;
        let half = 0.5 * fast_exp2(self.width.value) * step;
        let q = CROSS / (fast_exp2(half) - fast_exp2(-half));
        let k = 1.0 / q;
        let level = LEVEL * (span * q / sum_power(self.width.value)).sqrt() / NOMINAL;
        let up = fast_exp2(self.shift.value);
        let spread = self.spread.value;
        let top = 0.45 * rate;
        let noise = self.carrier == NOISE;
        let per = 1.0 / CTRL as f32;
        for i in 0..self.chunks * W {
            let (ca, cs, gl, gr) = if i < n {
                let f = fast_exp2(lo + step * i as f32);
                let fs = f * up;
                // Bands shifted near the top fade out rather than pile up.
                let fade = ((top - fs) / (0.1 * top)).clamp(0.0, 1.0);
                // White noise has as much in each hertz, so a band's share
                // grows with its frequency: tilted down, it plays as a
                // pink noise's (as much in each octave) at the nominal
                // level.
                let tilt = if noise {
                    (rate / (20.0 * std::f32::consts::LN_2 * fs.min(top))).sqrt()
                } else {
                    1.0
                };
                let (sign, pan) = if i % 2 == 0 {
                    (1.0, -spread)
                } else {
                    (-1.0, spread)
                };
                let amt = sign * level * fade * tilt * SQRT_2;
                let ang = (pan + 1.0) * FRAC_PI_4;
                (
                    coefs(f, k, rate),
                    coefs(fs, k, rate),
                    amt * ang.cos(),
                    amt * ang.sin(),
                )
            } else {
                let c = coefs(1000.0, k, rate);
                (c, c, 0.0, 0.0)
            };
            let (c, j) = (i / W, i % W);
            if snap {
                self.ana.set(i, ca);
                self.syn.set(i, cs);
                self.gl[c][j] = gl;
                self.gr[c][j] = gr;
                self.dgl[c][j] = 0.0;
                self.dgr[c][j] = 0.0;
            } else {
                self.ana.aim(i, ca, per);
                self.syn.aim(i, cs, per);
                self.dgl[c][j] = (gl - self.gl[c][j]) * per;
                self.dgr[c][j] = (gr - self.gr[c][j]) * per;
            }
        }
        if snap {
            self.k = k;
            self.dk = 0.0;
        } else {
            self.dk = (k - self.k) * per;
        }
    }

    /// Every [`CTRL`] frames: glide the controls a step and set the bands
    /// ramping to where they land; aim the saw at the voice's pitch.
    fn retune(&mut self) {
        self.low.glide();
        self.high.glide();
        self.shift.glide();
        self.width.glide();
        self.spread.glide();
        let still = self.low.settled()
            && self.high.settled()
            && self.shift.settled()
            && self.width.settled()
            && self.spread.settled();
        if !still || self.dirty {
            self.dirty = !still;
            self.aim(false);
            self.ramping = true;
        } else if self.ramping {
            self.ramping = false;
            self.ana.hold();
            self.syn.hold();
            self.dgl = [[0.0; W]; CHUNKS];
            self.dgr = [[0.0; W]; CHUNKS];
            self.dk = 0.0;
        }
        if self.carrier == SAW {
            self.pitch.target =
                fast_log2(self.voice_hz / self.rate as f32) + self.v[ids::PITCH as usize] / 12.0;
            self.pitch.glide();
            self.ddt = (fast_exp2(self.pitch.value) - self.dt) / CTRL as f32;
        }
    }

    /// Changes the bands' layout or the carrier, at the bottom of the dip.
    fn relayout(&mut self) {
        self.n = (self.v[ids::BANDS as usize] as usize).clamp(4, MAX_BANDS);
        self.carrier = (self.v[ids::CARRIER as usize] as usize).min(INPUT);
        self.chunks = self.n.div_ceil(W);
        self.ana.reset();
        self.syn.reset();
        self.env = [[0.0; W]; CHUNKS];
        self.tracker.reset();
        self.dirty = false;
        self.ramping = false;
        self.aim(true);
    }

    fn run(&mut self, ctx: &Ctx, from: usize, l: &mut [f32], r: &mut [f32]) {
        let len = l.len().min(r.len());
        let key = ctx
            .sidechain
            .filter(|(a, b)| a.len() >= from + len && b.len() >= from + len)
            .map(|(a, b)| (&a[from..from + len], &b[from..from + len]));
        let rate = self.rate as f32;
        for i in 0..len {
            let x = 0.5 * (l[i] + r[i]);
            // The voice (modulator) and the carrier.
            let (m, c) = match self.carrier {
                INPUT => (key.map_or(0.0, |(a, b)| 0.5 * (a[i] + b[i])), x),
                NOISE => (x, self.rng.bipolar() * NOMINAL * 3f32.sqrt()),
                _ => {
                    if self.tracker.push(x) && self.tracker.period > 0.0 {
                        self.voice_hz = rate / self.tracker.period;
                    }
                    self.dt += self.ddt;
                    self.phase += self.dt;
                    if self.phase >= 1.0 {
                        self.phase -= 1.0;
                    }
                    (x, saw(self.phase, self.dt) * NOMINAL * 3f32.sqrt())
                }
            };
            if self.ctrl == 0 {
                self.ctrl = CTRL;
                self.retune();
            }
            self.ctrl -= 1;
            if self.ramping {
                self.ana.advance(self.chunks);
                self.syn.advance(self.chunks);
                self.k += self.dk;
                for ch in 0..self.chunks {
                    for j in 0..W {
                        self.gl[ch][j] += self.dgl[ch][j];
                        self.gr[ch][j] += self.dgr[ch][j];
                    }
                }
            }
            // The bands.
            let k = self.k;
            let (att, rel) = (self.att, self.rel);
            // Four sums, one a lane, so the bands run four at a time
            // where the target has vectors (x86: 2.6 times fewer
            // instructions than one sum).
            let mut sl = [0.0f32; W];
            let mut sr = [0.0f32; W];
            for ch in 0..self.chunks {
                let a = &mut self.ana;
                let s = &mut self.syn;
                for j in 0..W {
                    let (a1, a2, a3) = (a.a[0][ch][j], a.a[1][ch][j], a.a[2][ch][j]);
                    let [s0, s1, s2, s3] = &mut a.s;
                    let (u1, _) = stage(m, a1, a2, a3, &mut s0[ch][j], &mut s1[ch][j]);
                    let o = k * u1;
                    let (w1, w2) = stage(o, a1, a2, a3, &mut s2[ch][j], &mut s3[ch][j]);
                    let bp = k * w1;
                    // Half the low-pass less the high-pass: in quadrature
                    // with the band-pass, about its size across the band.
                    let qd = 0.5 * k * (2.0 * w2 + bp - o);
                    let p = bp * bp + qd * qd;
                    let e0 = self.env[ch][j];
                    let e = e0 + (p - e0) * if p > e0 { att } else { rel };
                    self.env[ch][j] = e;
                    let (b1, b2, b3) = (s.a[0][ch][j], s.a[1][ch][j], s.a[2][ch][j]);
                    let [t0, t1, t2, t3] = &mut s.s;
                    let (z1, _) = stage(c, b1, b2, b3, &mut t0[ch][j], &mut t1[ch][j]);
                    let (y1, _) = stage(k * z1, b1, b2, b3, &mut t2[ch][j], &mut t3[ch][j]);
                    let y = k * y1 * e.sqrt();
                    sl[j] += y * self.gl[ch][j];
                    sr[j] += y * self.gr[ch][j];
                }
            }
            // The voice's highs, and the gate.
            let h = self.hp[0].run(m);
            let u = self.hp[1].run(h) * self.unvoiced.glide();
            let pw = m * m;
            let ge = self.gate_env;
            self.gate_env = ge
                + (pw - ge)
                    * if pw > ge {
                        self.gate_att
                    } else {
                        self.gate_rel
                    };
            if self.open {
                self.open = self.gate_env >= self.gate_on * GATE_HYST;
            } else {
                self.open = self.gate_env > self.gate_on;
            }
            let (to, k_g) = if self.open {
                (1.0, self.gate_up)
            } else {
                (0.0, self.gate_down)
            };
            self.gate += (to - self.gate) * k_g;
            self.level = if pw > self.level {
                pw
            } else {
                self.level * self.level_rel
            };
            // The dip while the layout changes.
            if self.pending {
                self.dip -= self.dip_step;
                if self.dip <= 0.0 {
                    self.dip = 0.0;
                    self.pending = false;
                    self.relayout();
                }
            } else if self.dip < 1.0 {
                self.dip = (self.dip + self.dip_step).min(1.0);
            }
            let d = self.dip;
            let wet = self.gate * d * d * (3.0 - 2.0 * d);
            let mix = self.mix.glide();
            let yl = (sl[0] + sl[1] + sl[2] + sl[3] + u) * wet;
            let yr = (sr[0] + sr[1] + sr[2] + sr[3] + u) * wet;
            l[i] = lerp(l[i], yl, mix);
            r[i] = lerp(r[i], yr, mix);
        }
        self.ana.flush(self.chunks);
        self.syn.flush(self.chunks);
        for c in self.env.iter_mut().take(self.chunks) {
            for v in c {
                *v = flush(*v);
            }
        }
        self.gate_env = flush(self.gate_env);
        self.level = flush(self.level);
        self.v[ids::VOICE as usize] = if self.level > 1e-6 {
            (3.010_3 * fast_log2(self.level)).clamp(-60.0, 0.0)
        } else {
            -60.0
        };
    }

    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        let retunes = (rate / CTRL as u32).max(1);
        for s in [
            &mut self.low,
            &mut self.high,
            &mut self.shift,
            &mut self.width,
            &mut self.spread,
        ] {
            s.set_time(retunes, GLIDE_MS);
        }
        self.pitch.set_time(retunes, 8.0);
        self.mix.set_time(rate, 20.0);
        self.unvoiced.set_time(rate, 20.0);
        self.followers();
        self.gate_att = kit::coef(1.0, rate);
        self.gate_rel = kit::coef(40.0, rate);
        self.gate_up = kit::coef(1.0, rate);
        self.gate_down = kit::coef(30.0, rate);
        self.gate_on = gate_power(self.v[ids::GATE as usize]);
        self.level_rel = (-1.0 / (0.3 * rate as f32)).exp();
        self.dip_step = 1.0 / (DIP_MS * 0.001 * rate as f32).max(1.0);
        // A 4th-order Butterworth high-pass: two stages.
        self.hp[0].set(UNVOICED_HZ, 0.541_2, rate);
        self.hp[1].set(UNVOICED_HZ, 1.306_6, rate);
        self.tracker.prepare(rate, 1);
    }

    fn clear(&mut self) {
        self.low.snap(self.low.target);
        self.high.snap(self.high.target);
        self.shift.snap(self.shift.target);
        self.width.snap(self.width.target);
        self.spread.snap(self.spread.target);
        self.mix.snap(self.mix.target);
        self.unvoiced.snap(self.unvoiced.target);
        self.ctrl = 0;
        self.pending = false;
        self.dip = 1.0;
        self.voice_hz = 110.0;
        self.pitch
            .snap(fast_log2(self.voice_hz / self.rate as f32) + self.v[ids::PITCH as usize] / 12.0);
        self.dt = fast_exp2(self.pitch.value);
        self.ddt = 0.0;
        self.phase = 0.0;
        self.rng = Rng::new(0x5EED_70C0);
        for h in &mut self.hp {
            h.reset();
        }
        self.gate_env = 0.0;
        self.open = false;
        self.gate = 0.0;
        self.level = 0.0;
        self.v[ids::VOICE as usize] = -60.0;
        self.relayout();
    }

    fn tail(&self) -> u32 {
        // The envelopes' release to under -60 dB, and the narrowest
        // bands' ringing.
        ((7.0 * self.v[ids::RELEASE as usize] + 200.0) * 0.001 * self.rate as f32) as u32
    }

    fn lat(&self) -> u32 {
        0
    }
}

/// The gate's threshold as a power, or 0 (off) fully down.
fn gate_power(db: f32) -> f32 {
    if db <= -90.0 {
        0.0
    } else {
        fast_exp2(db * 0.332_192_8)
    }
}

node_impl!(Vocoder, VOCODER_INFO, VOCODER_PARAMS);

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::Node;

    const RATE: u32 = 48_000;

    fn make(values: &[(u16, f32)]) -> Vocoder {
        let mut v = Vocoder::new();
        v.prepare(RATE, 512);
        for &(id, x) in values {
            v.set_param(id, x);
        }
        v.reset();
        v
    }

    /// Runs `input` through `fx` with `key` as its sidechain, in 128-frame
    /// blocks; the left side out.
    fn run(fx: &mut Vocoder, input: &[f32], key: Option<&[f32]>) -> (Vec<f32>, Vec<f32>) {
        let mut l = input.to_vec();
        let mut r = input.to_vec();
        let mut at = 0;
        while at < l.len() {
            let end = (at + 128).min(l.len());
            let ctx = Ctx {
                rate: RATE,
                frame: at as u64,
                bpm: 120.0,
                beats: 0.0,
                playing: true,
                sidechain: key.map(|k| (&k[at..end], &k[at..end])),
            };
            fx.process(&ctx, &[], &mut l[at..end], &mut r[at..end]);
            at = end;
        }
        (l, r)
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn noise(n: usize, amp: f32, seed: u32) -> Vec<f32> {
        let mut g = Rng::new(seed);
        (0..n).map(|_| g.bipolar() * amp * 3f32.sqrt()).collect()
    }

    fn sine(hz: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin())
            .collect()
    }

    /// The output's level near `hz` (a one-bin DFT over the last half).
    fn level_at(x: &[f32], hz: f32) -> f32 {
        let h = &x[x.len() / 2..];
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, v) in h.iter().enumerate() {
            let p = std::f64::consts::TAU * hz as f64 * i as f64 / RATE as f64;
            re += *v as f64 * p.cos();
            im += *v as f64 * p.sin();
        }
        (2.0 * (re * re + im * im).sqrt() / h.len() as f64) as f32
    }

    #[test]
    fn a_keyed_tone_comes_out_of_the_carrier_at_its_band() {
        // Noise carrier through the input, a 1 kHz tone on the key: the
        // output is the noise's band around 1 kHz, not 4 kHz.
        let n = RATE as usize;
        let mut fx = make(&[(ids::CARRIER, 2.0)]);
        let carrier = noise(n, 0.126, 3);
        let key = sine(1000.0, 0.178, n);
        let (y, _) = run(&mut fx, &carrier, Some(&key));
        let near: f32 = [900.0, 1000.0, 1100.0]
            .iter()
            .map(|&f| level_at(&y, f))
            .sum();
        let far: f32 = [3800.0, 4000.0, 4200.0]
            .iter()
            .map(|&f| level_at(&y, f))
            .sum();
        assert!(near > 10.0 * far, "near {near} far {far}");
    }

    #[test]
    fn formant_moves_the_band_up() {
        let n = RATE as usize;
        let carrier = noise(n, 0.126, 4);
        let key = sine(1000.0, 0.178, n);
        let mut fx = make(&[(ids::CARRIER, 2.0), (ids::FORMANT, 12.0)]);
        let (y, _) = run(&mut fx, &carrier, Some(&key));
        let up: f32 = [1900.0, 2000.0, 2100.0]
            .iter()
            .map(|&f| level_at(&y, f))
            .sum();
        let at: f32 = [900.0, 1000.0, 1100.0]
            .iter()
            .map(|&f| level_at(&y, f))
            .sum();
        assert!(up > 4.0 * at, "2 kHz {up} 1 kHz {at}");
    }

    #[test]
    fn with_no_key_the_input_carrier_is_silent_and_the_meter_says_so() {
        let n = RATE as usize / 2;
        let mut fx = make(&[(ids::CARRIER, 2.0)]);
        let (y, _) = run(&mut fx, &noise(n, 0.126, 5), None);
        assert!(rms(&y[n / 2..]) < 1e-4, "{}", rms(&y[n / 2..]));
        assert_eq!(fx.param(ids::VOICE), -60.0);
    }

    #[test]
    fn the_saw_speaks_at_about_the_voices_level() {
        // A voice-like noise at -18 dBFS RMS on the strip, the saw carrier:
        // the output within 6 dB of it, whatever the bands' layout.
        let n = RATE as usize;
        let voice = noise(n, 0.126, 6);
        let mut levels = Vec::new();
        for vals in [
            vec![],
            vec![(ids::BANDS, 4.0)],
            vec![(ids::BANDS, 40.0)],
            vec![(ids::BANDWIDTH, 40.0)],
            vec![(ids::LOW, 400.0), (ids::HIGH, 3000.0)],
        ] {
            let mut fx = make(&vals);
            let (y, _) = run(&mut fx, &voice, None);
            levels.push(20.0 * (rms(&y[n / 2..]) / 0.126).log10());
        }
        let (lo, hi) = levels
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
        assert!(lo > -8.0 && hi < 6.0, "{levels:?}");
        assert!(hi - lo < 6.0, "{levels:?}");
    }

    #[test]
    fn the_gate_shuts_a_quiet_voice_out() {
        let n = RATE as usize / 2;
        let mut fx = make(&[(ids::GATE, -40.0), (ids::CARRIER, 1.0)]);
        let (y, _) = run(&mut fx, &noise(n, 0.002, 7), None);
        assert!(rms(&y[n / 2..]) < 1e-5, "{}", rms(&y[n / 2..]));
        let mut fx = make(&[(ids::GATE, -90.0), (ids::CARRIER, 1.0)]);
        let (y, _) = run(&mut fx, &noise(n, 0.002, 7), None);
        assert!(rms(&y[n / 2..]) > 1e-4, "{}", rms(&y[n / 2..]));
    }

    #[test]
    fn spread_puts_the_bands_apart() {
        let n = RATE as usize;
        let voice = noise(n, 0.126, 8);
        let mut fx = make(&[(ids::CARRIER, 1.0), (ids::SPREAD, 100.0)]);
        let (l, r) = run(&mut fx, &voice, None);
        let d: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
        assert!(rms(&d[n / 2..]) > 0.5 * rms(&l[n / 2..]));
        let mut fx = make(&[(ids::CARRIER, 1.0)]);
        let (l, r) = run(&mut fx, &voice, None);
        assert_eq!(l, r);
    }

    #[test]
    fn changing_the_bands_dips_and_comes_back() {
        let n = RATE as usize;
        let voice = noise(n, 0.126, 9);
        let mut fx = make(&[(ids::CARRIER, 1.0)]);
        let mut l = voice.clone();
        let mut r = voice.clone();
        let ctx = Ctx::new(RATE, 0);
        let half = n / 2;
        fx.process(&ctx, &[], &mut l[..half], &mut r[..half]);
        fx.set_param(ids::BANDS, 24.0);
        fx.process(&ctx, &[], &mut l[half..], &mut r[half..]);
        assert_eq!(fx.n, 24);
        assert!(l.iter().all(|v| v.is_finite()));
        let before = rms(&l[half - 4800..half]);
        let after = rms(&l[n - 4800..]);
        assert!(
            (20.0 * (after / before).log10()).abs() < 3.0,
            "{before} {after}"
        );
    }
}
