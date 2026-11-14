//! Three classic reverbs, lighter than the Cloud reverb: [`Room`] (early
//! reflections into an eight-line feedback delay network), [`Plate`]
//! (Dattorro's figure-eight tank) and [`Spring`] (chirping stretched
//! all-pass chains in a feedback loop).
//!
//! All three read their delay lines with cubic interpolation where the
//! length moves (Size, modulation) and whole frames elsewhere, flush
//! denormals in every loop, and glide every control.

use mpfree_node::{Ctx, Kind, NodeInfo, ParamInfo, Smooth};

use crate::kit::{self, node_impl, param, Glide, Hz, Line, Linear, Log, Ms, Percent};
use crate::svf::{Svf, SvfMode};
use crate::{defaults, flush, lerp, Preset};

/// A Schroeder all-pass around a delay line, in Dattorro's form:
/// `w = x + g z`, `y = z - g w`, where `z` is the line's output.
#[derive(Clone, Debug, Default)]
struct Allpass {
    line: Line,
}

impl Allpass {
    fn new(frames: usize) -> Allpass {
        Allpass {
            line: Line::new(frames + 2),
        }
    }
    /// One frame through `d` whole frames of delay.
    #[inline]
    fn run(&mut self, x: f32, g: f32, d: usize) -> f32 {
        let z = self.line.tap(d);
        let w = flush(x + g * z);
        self.line.push(w);
        z - g * w
    }
    /// One frame through a fractional, moving delay of `d` frames (at
    /// least 2).
    #[inline]
    fn run_frac(&mut self, x: f32, g: f32, d: f32) -> f32 {
        let z = self.line.read(d - 1.0);
        let w = flush(x + g * z);
        self.line.push(w);
        z - g * w
    }
    fn clear(&mut self) {
        self.line.clear();
    }
}

/// A one-pole low-pass for damping: `y += (x - y) * k`.
#[derive(Clone, Copy, Debug, Default)]
struct Damp {
    y: f32,
}

impl Damp {
    #[inline]
    fn run(&mut self, x: f32, k: f32) -> f32 {
        self.y = flush(self.y + (x - self.y) * k);
        self.y
    }
}

/// The one-pole coefficient for a corner at `hz`.
fn pole(hz: f32, rate: u32) -> f32 {
    1.0 - (-std::f32::consts::TAU * hz / rate as f32).exp()
}

// ---------------------------------------------------------------- plate

/// [`Plate`] parameter ids.
pub mod plate {
    pub const PREDELAY: u16 = 0;
    pub const SIZE: u16 = 1;
    pub const DECAY: u16 = 2;
    pub const DAMPING: u16 = 3;
    pub const TONE: u16 = 4;
    pub const MOD: u16 = 5;
    pub const LOW_CUT: u16 = 6;
    pub const WIDTH: u16 = 7;
    pub const MIX: u16 = 8;
}

static PLATE_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.plate",
    name: "Plate reverb",
    kind: Kind::Effect,
};
static PLATE_PARAMS: [ParamInfo; 9] = [
    param(0, "Pre-delay", "Plate", 0.0, 250.0, 10.0, Ms, Linear),
    param(1, "Size", "Plate", 50.0, 150.0, 100.0, Percent, Linear),
    param(2, "Decay", "Plate", 200.0, 20_000.0, 2000.0, Ms, Log),
    param(3, "Damping", "Plate", 0.0, 100.0, 35.0, Percent, Linear),
    param(4, "Tone", "Plate", 1000.0, 20_000.0, 12_000.0, Hz, Log),
    param(5, "Mod", "Plate", 0.0, 100.0, 40.0, Percent, Linear),
    param(6, "Low cut", "Plate", 20.0, 1000.0, 80.0, Hz, Log),
    param(7, "Width", "Plate", 0.0, 100.0, 100.0, Percent, Linear),
    param(8, "Mix", "Plate", 0.0, 100.0, 25.0, Percent, Linear),
];

/// Plate factory presets.
pub static PLATE_PRESETS: &[Preset] = &[
    Preset {
        name: "Vocal plate",
        values: &[(plate::DECAY, 1800.0), (plate::PREDELAY, 30.0)],
    },
    Preset {
        name: "Snare plate",
        values: &[
            (plate::DECAY, 1200.0),
            (plate::SIZE, 80.0),
            (plate::DAMPING, 20.0),
        ],
    },
    Preset {
        name: "Dark plate",
        values: &[
            (plate::DECAY, 3500.0),
            (plate::DAMPING, 70.0),
            (plate::TONE, 4000.0),
        ],
    },
    Preset {
        name: "Endless plate",
        values: &[
            (plate::DECAY, 18000.0),
            (plate::SIZE, 140.0),
            (plate::MOD, 80.0),
        ],
    },
];

/// The reference rate of Dattorro's published lengths.
const DATTORRO_RATE: f32 = 29_761.0;
const PLATE_IN: [usize; 4] = [142, 107, 379, 277];
/// Tank: [modulated all-pass, delay, all-pass, delay] for each half.
const PLATE_TANK: [[usize; 4]; 2] = [[672, 4453, 1800, 3720], [908, 4217, 2656, 3163]];
/// Output taps: (half, element 1 delay / 2 all-pass / 3 delay, frames,
/// sign), left then right, from the paper's table.
const PLATE_TAPS: [[(usize, usize, usize, f32); 7]; 2] = [
    [
        (1, 1, 266, 1.0),
        (1, 1, 2974, 1.0),
        (1, 2, 1913, -1.0),
        (1, 3, 1996, 1.0),
        (0, 1, 1990, -1.0),
        (0, 2, 187, -1.0),
        (0, 3, 1066, -1.0),
    ],
    [
        (0, 1, 353, 1.0),
        (0, 1, 3627, 1.0),
        (0, 2, 1228, -1.0),
        (0, 3, 2673, 1.0),
        (1, 1, 2111, -1.0),
        (1, 2, 335, -1.0),
        (1, 3, 121, -1.0),
    ],
];
/// The tank's modulation: Dattorro's 16 frames at his rate, at about 1 Hz.
const PLATE_EXCURSION: f32 = 16.0;

/// A plate reverb after Jon Dattorro's 1997 design: four input diffusers,
/// then a figure-eight tank of two halves that feed each other, each a
/// modulated all-pass, a delay, damping and a second all-pass. The output
/// is taken from seven points per side, so the two sides are dense and
/// different. The modulation keeps the tank from ringing; Decay is the
/// time to fall 60 dB.
pub struct Plate {
    v: [f32; 9],
    rate: u32,
    /// Frames per Dattorro frame at this rate.
    scale: f32,
    pre: Line,
    bw: Damp,
    input: [Allpass; 4],
    mod_ap: [Allpass; 2],
    delay1: [Line; 2],
    ap2: [Allpass; 2],
    delay2: [Line; 2],
    damp: [Damp; 2],
    /// The value each half last sent to the other.
    feed: [f32; 2],
    lfo: f32,
    lfo_inc: f32,
    low_cut: Svf,
    size: Smooth,
    decay_g: Smooth,
    damping: Smooth,
    pre_frames: Smooth,
    modd: Smooth,
    width: Smooth,
    mix: Smooth,
    tone_k: f32,
}

impl Default for Plate {
    fn default() -> Self {
        Self::new()
    }
}

impl Plate {
    pub fn new() -> Plate {
        let v = defaults(&PLATE_PARAMS);
        let mut p = Plate {
            v,
            rate: 48_000,
            scale: 1.0,
            pre: Line::default(),
            bw: Damp::default(),
            input: Default::default(),
            mod_ap: Default::default(),
            delay1: Default::default(),
            ap2: Default::default(),
            delay2: Default::default(),
            damp: [Damp::default(); 2],
            feed: [0.0; 2],
            lfo: 0.0,
            lfo_inc: 0.0,
            low_cut: Svf::default(),
            size: Smooth::new(1.0),
            decay_g: Smooth::new(0.5),
            damping: Smooth::new(0.5),
            pre_frames: Smooth::new(0.0),
            modd: Smooth::new(0.0),
            width: Smooth::new(1.0),
            mix: Smooth::new(0.25),
            tone_k: 1.0,
        };
        p.setup(48_000);
        p
    }

    /// The per-pass decay gain for Decay seconds at the size now: each
    /// half of the loop passes two decay gains, so a full trip of
    /// `half` seconds multiplies by g squared.
    fn decay_gain(&self) -> f32 {
        let half: usize = PLATE_TANK[0].iter().sum();
        let secs = half as f32 / DATTORRO_RATE * (self.v[plate::SIZE as usize] / 100.0);
        let rt = self.v[plate::DECAY as usize] * 0.001;
        10f32.powf(-3.0 * secs / (2.0 * rt)).min(0.9995)
    }

    fn apply(&mut self, id: u16, v: f32) {
        match id {
            plate::PREDELAY => self.pre_frames.target = v * 0.001 * self.rate as f32,
            plate::SIZE => {
                self.size.target = v / 100.0;
                self.decay_g.target = self.decay_gain();
            }
            plate::DECAY => self.decay_g.target = self.decay_gain(),
            plate::DAMPING => self.damping.target = 1.0 - 0.85 * v / 100.0,
            plate::TONE => self.tone_k = pole(v, self.rate).min(1.0),
            plate::MOD => self.modd.target = v / 100.0,
            plate::LOW_CUT => self
                .low_cut
                .set(SvfMode::HighPass, v, 0.707, 0.0, self.rate),
            plate::WIDTH => self.width.target = v / 100.0,
            plate::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }

    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let sc = self.scale;
        for i in 0..l.len() {
            let size = self.size.glide();
            let g = self.decay_g.glide();
            let damping = self.damping.glide();
            let modd = self.modd.glide();
            let pre = self.pre_frames.glide();
            let s = sc * size;
            let x = 0.5 * (l[i] + r[i]);
            self.pre.push(x);
            let mut d = if pre >= 2.0 {
                self.pre.read(pre - 1.0)
            } else {
                x
            };
            d = self.bw.run(d, self.tone_k);
            for (k, ap) in self.input.iter_mut().enumerate() {
                let gain = if k < 2 { 0.75 } else { 0.625 };
                d = ap.run(d, gain, ((PLATE_IN[k] as f32 * s) as usize).max(1));
            }
            self.lfo = kit::wrap(self.lfo + self.lfo_inc);
            let exc = PLATE_EXCURSION * sc * modd;
            let diff2 = (g + 0.15).clamp(0.25, 0.5);
            let mut out_half = [0.0f32; 2];
            for h in 0..2 {
                let len = &PLATE_TANK[h];
                let m = if h == 0 {
                    kit::sine(self.lfo)
                } else {
                    kit::cosine(self.lfo)
                };
                let into = d + self.feed[1 - h];
                let a = self.mod_ap[h].run_frac(
                    into,
                    -0.7,
                    (len[0] as f32 * s + exc * m).max(2.0 + exc),
                );
                let z1 = self.delay1[h].tap(((len[1] as f32 * s) as usize).max(1));
                self.delay1[h].push(a);
                let b = self.damp[h].run(z1, damping) * g;
                let c = self.ap2[h].run(b, diff2, ((len[2] as f32 * s) as usize).max(1));
                let z2 = self.delay2[h].tap(((len[3] as f32 * s) as usize).max(1));
                self.delay2[h].push(c);
                out_half[h] = z2 * g;
            }
            self.feed = out_half;
            let mut y = [0.0f32; 2];
            for (side, taps) in PLATE_TAPS.iter().enumerate() {
                for &(h, el, at, sign) in taps {
                    let at = ((at as f32 * s) as usize).max(1);
                    let v = match el {
                        1 => self.delay1[h].tap(at),
                        2 => self.ap2[h].line.tap(at),
                        _ => self.delay2[h].tap(at),
                    };
                    y[side] += sign * v;
                }
                y[side] = self.low_cut.run(side, 0.6 * y[side]);
            }
            let w = self.width.glide();
            let mid = 0.5 * (y[0] + y[1]);
            let (wl, wr) = (lerp(mid, y[0], w), lerp(mid, y[1], w));
            let mix = self.mix.glide();
            l[i] = lerp(l[i], wl, mix);
            r[i] = lerp(r[i], wr, mix);
        }
    }

    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.scale = rate as f32 / DATTORRO_RATE;
        // Room for Size 150 % and the excursion.
        let room = |n: usize| (n as f32 * self.scale * 1.5) as usize + 64;
        self.pre = Line::new((0.26 * rate as f32) as usize);
        self.input = PLATE_IN.map(|n| Allpass::new(room(n)));
        for h in 0..2 {
            self.mod_ap[h] = Allpass::new(room(PLATE_TANK[h][0]));
            self.delay1[h] = Line::new(room(PLATE_TANK[h][1]));
            self.ap2[h] = Allpass::new(room(PLATE_TANK[h][2]));
            self.delay2[h] = Line::new(room(PLATE_TANK[h][3]));
        }
        self.lfo_inc = 1.0 / rate as f32;
        for s in [
            &mut self.size,
            &mut self.decay_g,
            &mut self.damping,
            &mut self.modd,
            &mut self.width,
            &mut self.mix,
        ] {
            s.set_time(rate, 50.0);
        }
        self.pre_frames.set_time(rate, 100.0);
        for id in 0..PLATE_PARAMS.len() as u16 {
            self.apply(id, self.v[id as usize]);
        }
        self.clear();
    }

    fn clear(&mut self) {
        self.pre.clear();
        self.bw = Damp::default();
        for a in self
            .input
            .iter_mut()
            .chain(&mut self.mod_ap)
            .chain(&mut self.ap2)
        {
            a.clear();
        }
        for d in self.delay1.iter_mut().chain(&mut self.delay2) {
            d.clear();
        }
        self.damp = [Damp::default(); 2];
        self.feed = [0.0; 2];
        self.lfo = 0.0;
        self.low_cut.reset();
        for s in [
            &mut self.size,
            &mut self.decay_g,
            &mut self.damping,
            &mut self.pre_frames,
            &mut self.modd,
            &mut self.width,
            &mut self.mix,
        ] {
            s.snap(s.target);
        }
    }

    fn tail(&self) -> u32 {
        ((self.v[plate::DECAY as usize] * 0.0012 + 0.3) * self.rate as f32) as u32
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(Plate, PLATE_INFO, PLATE_PARAMS);

// ---------------------------------------------------------------- room

/// [`Room`] parameter ids.
pub mod room {
    pub const PREDELAY: u16 = 0;
    pub const SIZE: u16 = 1;
    pub const DECAY: u16 = 2;
    pub const DAMPING: u16 = 3;
    pub const EARLY: u16 = 4;
    pub const MOD: u16 = 5;
    pub const GATE: u16 = 6;
    pub const LOW_CUT: u16 = 7;
    pub const WIDTH: u16 = 8;
    pub const MIX: u16 = 9;
}

const LINES: usize = 8;
/// Line lengths in ms at Size 100 %, spread so no two share a factor.
const ROOM_MS: [f32; LINES] = [23.3, 29.1, 34.7, 38.9, 43.1, 47.9, 53.3, 59.7];
/// Early reflections: (ms at Size 100 %, gain, pan -1 left to 1 right).
const EARLY: [(f32, f32, f32); 8] = [
    (4.3, 0.84, -0.7),
    (7.9, 0.72, 0.6),
    (11.2, 0.66, -0.3),
    (14.8, 0.55, 0.9),
    (19.6, 0.47, -0.9),
    (24.1, 0.40, 0.2),
    (29.8, 0.33, 0.7),
    (36.5, 0.27, -0.5),
];
/// Each early tap's left and right gains (constant power is not needed:
/// the taps are summed, not panned one at a time).
const EARLY_PAN: [[f32; 2]; 8] = {
    let mut p = [[0.0; 2]; 8];
    let mut k = 0;
    while k < 8 {
        p[k] = [0.5 - 0.5 * EARLY[k].2, 0.5 + 0.5 * EARLY[k].2];
        k += 1;
    }
    p
};
/// The smallest Size, as a share of the lengths above.
const ROOM_MIN: f32 = 0.25;
const ROOM_MAX: f32 = 1.5;

static ROOM_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.room",
    name: "Room reverb",
    kind: Kind::Effect,
};
static ROOM_PARAMS: [ParamInfo; 10] = [
    param(0, "Pre-delay", "Room", 0.0, 100.0, 5.0, Ms, Linear),
    param(1, "Size", "Room", 0.0, 100.0, 45.0, Percent, Linear),
    param(2, "Decay", "Room", 100.0, 5000.0, 800.0, Ms, Log),
    param(3, "Damping", "Room", 0.0, 100.0, 50.0, Percent, Linear),
    param(4, "Early", "Room", 0.0, 100.0, 60.0, Percent, Linear),
    param(5, "Mod", "Room", 0.0, 100.0, 25.0, Percent, Linear),
    param(6, "Gate", "Room", 0.0, 600.0, 0.0, Ms, Linear),
    param(7, "Low cut", "Room", 20.0, 1000.0, 60.0, Hz, Log),
    param(8, "Width", "Room", 0.0, 100.0, 100.0, Percent, Linear),
    param(9, "Mix", "Room", 0.0, 100.0, 25.0, Percent, Linear),
];

/// Room factory presets.
pub static ROOM_PRESETS: &[Preset] = &[
    Preset {
        name: "Small booth",
        values: &[
            (room::SIZE, 15.0),
            (room::DECAY, 350.0),
            (room::EARLY, 80.0),
        ],
    },
    Preset {
        name: "Drum room",
        values: &[
            (room::SIZE, 50.0),
            (room::DECAY, 900.0),
            (room::DAMPING, 35.0),
        ],
    },
    Preset {
        name: "Big studio",
        values: &[
            (room::SIZE, 85.0),
            (room::DECAY, 1800.0),
            (room::PREDELAY, 15.0),
        ],
    },
    Preset {
        name: "Gated snare",
        values: &[
            (room::SIZE, 70.0),
            (room::DECAY, 2500.0),
            (room::GATE, 250.0),
            (room::DAMPING, 20.0),
        ],
    },
];

/// A room: eight early reflections panned around the listener, then an
/// eight-line feedback delay network mixed by a Householder matrix, with a
/// damping filter in each line set from Decay (the time to fall 60 dB at
/// low frequencies) and Damping (how much sooner the highs die). A slow
/// modulation on the lines keeps small rooms from sounding metallic. Gate
/// cuts the tail that many ms after the sound stops, the 1980s gated
/// snare.
pub struct Room {
    v: [f32; 10],
    rate: u32,
    pre: Line,
    early: Line,
    lines: [Line; LINES],
    /// Each line's loss filter: low-frequency gain and pole.
    g: [f32; LINES],
    a: [f32; LINES],
    state: [f32; LINES],
    phase: [f32; LINES],
    /// Each line's LFO step over 16 frames.
    lfo_step: [f32; LINES],
    /// Each line's length at the current size, its wobble and the
    /// wobble's step per frame (the LFO is worked out every 16 frames and
    /// followed in straight lines between).
    base: [f32; LINES],
    wob: [f32; LINES],
    dwob: [f32; LINES],
    /// The early taps (gain folded in) and pans for the size `placed`.
    taps: [kit::Frac; 8],
    placed: f32,
    pre_tap: kit::Frac,
    pre_placed: f32,
    /// Frames since reset, for the 16-frame LFO grid.
    n: u32,
    low_cut: Svf,
    size: Smooth,
    pre_frames: Smooth,
    early_gain: Smooth,
    modd: Smooth,
    width: Smooth,
    mix: Smooth,
    env: f32,
    env_fall: f32,
    held: u32,
    gate_g: f32,
    gate_step: f32,
}

impl Default for Room {
    fn default() -> Self {
        Self::new()
    }
}

impl Room {
    pub fn new() -> Room {
        let v = defaults(&ROOM_PARAMS);
        let mut r = Room {
            v,
            rate: 48_000,
            pre: Line::default(),
            early: Line::default(),
            lines: Default::default(),
            g: [0.0; LINES],
            a: [0.0; LINES],
            state: [0.0; LINES],
            phase: [0.0; LINES],
            lfo_step: [0.0; LINES],
            base: [0.0; LINES],
            wob: [0.0; LINES],
            dwob: [0.0; LINES],
            taps: [kit::Frac::default(); 8],
            placed: f32::NAN,
            pre_tap: kit::Frac::default(),
            pre_placed: f32::NAN,
            n: 0,
            low_cut: Svf::default(),
            size: Smooth::new(1.0),
            pre_frames: Smooth::new(0.0),
            early_gain: Smooth::new(0.6),
            modd: Smooth::new(0.0),
            width: Smooth::new(1.0),
            mix: Smooth::new(0.25),
            env: 0.0,
            env_fall: 0.0,
            held: 0,
            gate_g: 1.0,
            gate_step: 0.0,
        };
        r.setup(48_000);
        r
    }

    fn size_of(v: f32) -> f32 {
        ROOM_MIN + (ROOM_MAX - ROOM_MIN) * v / 100.0
    }

    /// Each line's loss for Decay at low frequencies and a shorter decay
    /// for the highs (Jot's absorption filters): a one-pole whose gain at
    /// DC and at Nyquist give those two decay times for its length.
    fn set_loss(&mut self) {
        let rt = self.v[room::DECAY as usize] * 0.001;
        let rt_hi = rt * (1.0 - 0.9 * self.v[room::DAMPING as usize] / 100.0);
        let size = Self::size_of(self.v[room::SIZE as usize]);
        for k in 0..LINES {
            let secs = ROOM_MS[k] * 0.001 * size;
            let g = 10f32.powf(-3.0 * secs / rt);
            let g_hi = 10f32.powf(-3.0 * secs / rt_hi.max(0.02));
            let ratio = (g_hi / g).clamp(0.0, 1.0);
            self.g[k] = g;
            self.a[k] = (1.0 - ratio) / (1.0 + ratio);
        }
    }

    fn apply(&mut self, id: u16, v: f32) {
        match id {
            room::PREDELAY => self.pre_frames.target = v * 0.001 * self.rate as f32,
            room::SIZE => {
                self.size.target = Self::size_of(v);
                self.set_loss();
            }
            room::DECAY | room::DAMPING => self.set_loss(),
            room::EARLY => self.early_gain.target = v / 100.0,
            room::MOD => self.modd.target = v / 100.0,
            room::LOW_CUT => self
                .low_cut
                .set(SvfMode::HighPass, v, 0.707, 0.0, self.rate),
            room::WIDTH => self.width.target = v / 100.0,
            room::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }

    /// Places the early taps and the lines' lengths for a size (every
    /// frame while Size glides, once when it settles).
    fn place(&mut self, size: f32) {
        let ms = self.rate as f32 * 0.001;
        for (k, &(t, g, _)) in EARLY.iter().enumerate() {
            self.taps[k] = self.early.frac(t * ms * size, g);
        }
        for k in 0..LINES {
            self.base[k] = ROOM_MS[k] * ms * size;
        }
        self.placed = size;
    }

    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let ms = self.rate as f32 * 0.001;
        let gate_frames = (self.v[room::GATE as usize] * ms) as u32;
        let gate_on = gate_frames > 0;
        for i in 0..l.len() {
            let size = self.size.glide();
            if size != self.placed {
                self.place(size);
            }
            let pre = self.pre_frames.glide();
            if pre != self.pre_placed {
                self.pre_tap = self.pre.frac(pre - 1.0, 1.0);
                self.pre_placed = pre;
            }
            let early_gain = self.early_gain.glide();
            let modd = self.modd.glide();
            if self.n & 15 == 0 {
                for k in 0..LINES {
                    self.phase[k] = kit::wrap(self.phase[k] + self.lfo_step[k]);
                    let to = modd * 6.0 * kit::sine(self.phase[k]);
                    self.dwob[k] = (to - self.wob[k]) * (1.0 / 16.0);
                }
            }
            self.n = self.n.wrapping_add(1);
            let x = 0.5 * (l[i] + r[i]);
            self.pre.push(x);
            let d = if pre >= 2.0 {
                self.pre.read_at(&self.pre_tap)
            } else {
                x
            };
            // Early reflections, panned.
            self.early.push(d);
            let mut er = [0.0f32; 2];
            for (tap, pan) in self.taps.iter().zip(EARLY_PAN) {
                let v = self.early.read_at(tap);
                er[0] += v * pan[0];
                er[1] += v * pan[1];
            }
            // The network: read every line, mix, feed back.
            let mut out = [0.0f32; LINES];
            let mut sum = 0.0;
            for k in 0..LINES {
                self.wob[k] += self.dwob[k];
                let z = self.lines[k].read(self.base[k] + self.wob[k] - 1.0);
                // Loss: gain at DC g, less at the top.
                self.state[k] =
                    flush(self.g[k] * (1.0 - self.a[k]) * z + self.a[k] * self.state[k]);
                out[k] = self.state[k];
                sum += out[k];
            }
            let h = sum * (2.0 / LINES as f32);
            let input = d + 0.3 * (er[0] + er[1]);
            for k in 0..LINES {
                let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
                self.lines[k].push(out[k] - h + sign * 0.35 * input);
            }
            let mut y = [
                out[0] - out[2] + out[4] - out[6] + 0.5 * (out[1] + out[5]),
                out[1] - out[3] + out[5] - out[7] + 0.5 * (out[2] + out[6]),
            ];
            for (side, v) in y.iter_mut().enumerate() {
                *v = self.low_cut.run(side, 0.35 * *v + early_gain * er[side]);
            }
            // The gate, keyed by the dry input.
            if gate_on {
                let level = x.abs();
                self.env = if level > self.env {
                    level
                } else {
                    self.env * self.env_fall
                };
                if self.env > 0.01 {
                    self.held = gate_frames;
                    self.gate_g = 1.0;
                } else if self.held > 0 {
                    self.held -= 1;
                } else {
                    self.gate_g = (self.gate_g - self.gate_step).max(0.0);
                }
                y[0] *= self.gate_g;
                y[1] *= self.gate_g;
            }
            let w = self.width.glide();
            let mid = 0.5 * (y[0] + y[1]);
            let mix = self.mix.glide();
            l[i] = lerp(l[i], lerp(mid, y[0], w), mix);
            r[i] = lerp(r[i], lerp(mid, y[1], w), mix);
        }
    }

    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        let ms = rate as f32 * 0.001;
        self.pre = Line::new((101.0 * ms) as usize);
        self.early = Line::new((EARLY[7].0 * ROOM_MAX * ms) as usize + 8);
        for k in 0..LINES {
            self.lines[k] = Line::new((ROOM_MS[k] * ROOM_MAX * ms) as usize + 16);
        }
        for k in 0..LINES {
            self.lfo_step[k] = 16.0 * 0.37 / rate as f32 * (1.0 + 0.13 * k as f32);
        }
        self.env_fall = crate::db_to_gain(-60.0 / (0.03 * rate as f32));
        // The gate closes over 4 ms.
        self.gate_step = 1.0 / (4.0 * ms);
        for s in [
            &mut self.size,
            &mut self.early_gain,
            &mut self.modd,
            &mut self.width,
            &mut self.mix,
        ] {
            s.set_time(rate, 50.0);
        }
        self.pre_frames.set_time(rate, 100.0);
        for id in 0..ROOM_PARAMS.len() as u16 {
            self.apply(id, self.v[id as usize]);
        }
        self.clear();
    }

    fn clear(&mut self) {
        self.pre.clear();
        self.early.clear();
        for l in &mut self.lines {
            l.clear();
        }
        self.state = [0.0; LINES];
        self.phase = [0.0; LINES];
        for (k, p) in self.phase.iter_mut().enumerate() {
            *p = k as f32 / LINES as f32;
        }
        self.wob = [0.0; LINES];
        self.dwob = [0.0; LINES];
        self.n = 0;
        self.placed = f32::NAN;
        self.pre_placed = f32::NAN;
        self.low_cut.reset();
        self.env = 0.0;
        self.held = 0;
        self.gate_g = 1.0;
        for s in [
            &mut self.size,
            &mut self.pre_frames,
            &mut self.early_gain,
            &mut self.modd,
            &mut self.width,
            &mut self.mix,
        ] {
            s.snap(s.target);
        }
    }

    fn tail(&self) -> u32 {
        ((self.v[room::DECAY as usize] * 0.0012 + 0.2) * self.rate as f32) as u32
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(Room, ROOM_INFO, ROOM_PARAMS);

// ---------------------------------------------------------------- spring

/// [`Spring`] parameter ids.
pub mod spring {
    pub const LENGTH: u16 = 0;
    pub const DECAY: u16 = 1;
    pub const TENSION: u16 = 2;
    pub const DRIP: u16 = 3;
    pub const TONE: u16 = 4;
    pub const LOW_CUT: u16 = 5;
    pub const MIX: u16 = 6;
}

/// Stretched all-pass stages per spring (Parker's low chirp path, cut to
/// fit the MPC: 100 in the full model).
const STAGES: usize = 40;
/// The stretch: the chirp sits below rate / (2 K), about 4.8 kHz at 48 kHz.
const STRETCH: usize = 5;

static SPRING_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.spring",
    name: "Spring reverb",
    kind: Kind::Effect,
};
static SPRING_PARAMS: [ParamInfo; 7] = [
    param(0, "Length", "Spring", 20.0, 80.0, 40.0, Ms, Linear),
    param(1, "Decay", "Spring", 500.0, 8000.0, 2500.0, Ms, Log),
    param(2, "Tension", "Spring", 0.0, 100.0, 50.0, Percent, Linear),
    param(3, "Drip", "Spring", 0.0, 100.0, 50.0, Percent, Linear),
    param(4, "Tone", "Spring", 1000.0, 8000.0, 4000.0, Hz, Log),
    param(5, "Low cut", "Spring", 20.0, 1000.0, 150.0, Hz, Log),
    param(6, "Mix", "Spring", 0.0, 100.0, 30.0, Percent, Linear),
];

/// Spring factory presets.
pub static SPRING_PRESETS: &[Preset] = &[
    Preset {
        name: "Amp spring",
        values: &[(spring::DECAY, 2000.0), (spring::DRIP, 40.0)],
    },
    Preset {
        name: "Dub splash",
        values: &[
            (spring::DECAY, 4500.0),
            (spring::DRIP, 90.0),
            (spring::TENSION, 70.0),
        ],
    },
    Preset {
        name: "Short tank",
        values: &[
            (spring::LENGTH, 25.0),
            (spring::DECAY, 1000.0),
            (spring::TONE, 6000.0),
        ],
    },
];

/// One spring: the chirp chain, the round-trip delay and its loss.
#[derive(Clone, Debug)]
struct OneSpring {
    /// Each stage's last STRETCH inputs and outputs (fixed arrays, so
    /// the chain's loop has no bounds checks).
    xs: [[f32; STRETCH]; STAGES],
    ys: [[f32; STRETCH]; STAGES],
    at: usize,
    line: Line,
    lp: Svf,
    fb: f32,
}

impl Default for OneSpring {
    fn default() -> Self {
        OneSpring {
            xs: [[0.0; STRETCH]; STAGES],
            ys: [[0.0; STRETCH]; STAGES],
            at: 0,
            line: Line::default(),
            lp: Svf::default(),
            fb: 0.0,
        }
    }
}

impl OneSpring {
    fn new(frames: usize) -> OneSpring {
        OneSpring {
            line: Line::new(frames),
            ..OneSpring::default()
        }
    }

    /// The chirp: STAGES first-order all-passes in z^-K, each
    /// `y = a x + x[n-K] - a y[n-K]`, which delays lows more than highs
    /// below rate / 2K, the spring's falling "pew".
    #[inline]
    fn chirp(&mut self, mut x: f32, a: f32) -> f32 {
        let at = self.at.min(STRETCH - 1);
        for (xs, ys) in self.xs.iter_mut().zip(self.ys.iter_mut()) {
            let y = flush(a * (x - ys[at]) + xs[at]);
            xs[at] = x;
            ys[at] = y;
            x = y;
        }
        self.at = if at + 1 == STRETCH { 0 } else { at + 1 };
        x
    }

    fn clear(&mut self) {
        self.xs = [[0.0; STRETCH]; STAGES];
        self.ys = [[0.0; STRETCH]; STAGES];
        self.at = 0;
        self.line.clear();
        self.lp.reset();
        self.fb = 0.0;
    }
}

/// A spring tank: two springs of slightly different lengths (one per
/// side). Each passes the sound through a long chain of stretched
/// all-passes, which smears it into the spring's chirp, then round a
/// delay of Length with a loss for Decay, so each return chirps again.
/// Drip feeds more of each chirp back into the spring, for the dub splash
/// a tap on the tank gives. After Välimäki, Parker and Abel's parametric
/// spring model, with the chain cut to 40 stages for the MPC.
pub struct Spring {
    v: [f32; 7],
    rate: u32,
    springs: [OneSpring; 2],
    in_hp: Svf,
    length: Smooth,
    gain: f32,
    a: f32,
    drip: Smooth,
    mix: Smooth,
}

impl Default for Spring {
    fn default() -> Self {
        Self::new()
    }
}

impl Spring {
    pub fn new() -> Spring {
        let v = defaults(&SPRING_PARAMS);
        let mut s = Spring {
            v,
            rate: 48_000,
            springs: Default::default(),
            in_hp: Svf::default(),
            length: Smooth::new(v[0]),
            gain: 0.5,
            a: -0.6,
            drip: Smooth::new(0.5),
            mix: Smooth::new(0.3),
        };
        s.setup(48_000);
        s
    }

    fn set_gain(&mut self) {
        let secs = self.v[spring::LENGTH as usize] * 0.001;
        self.gain = 10f32
            .powf(-3.0 * secs / (self.v[spring::DECAY as usize] * 0.001))
            .min(0.98);
    }

    fn apply(&mut self, id: u16, v: f32) {
        match id {
            spring::LENGTH => {
                self.length.target = v;
                self.set_gain();
            }
            spring::DECAY => self.set_gain(),
            // Negative, so lows are delayed more than highs below the
            // corner (a positive coefficient chirps the other way).
            spring::TENSION => self.a = -(0.45 + 0.3 * v / 100.0),
            spring::DRIP => self.drip.target = v / 100.0,
            spring::TONE => {
                for s in &mut self.springs {
                    s.lp.set(SvfMode::LowPass, v, 0.6, 0.0, self.rate);
                }
            }
            spring::LOW_CUT => self.in_hp.set(SvfMode::HighPass, v, 0.707, 0.0, self.rate),
            spring::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }

    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let ms = self.rate as f32 * 0.001;
        for i in 0..l.len() {
            let len = self.length.glide();
            let drip = self.drip.glide();
            let x = self.in_hp.run(0, 0.5 * (l[i] + r[i]));
            let mut y = [0.0f32; 2];
            for (k, s) in self.springs.iter_mut().enumerate() {
                // The second spring is 13 % longer, so the sides differ.
                let frames = len * ms * (1.0 + 0.13 * k as f32);
                let back = s.line.read(frames - 1.0);
                let into = x + s.fb;
                let c = s.chirp(into, self.a);
                let c = s.lp.run(0, c);
                s.line.push(c);
                // What comes back round, chirped again, plus Drip's extra
                // ring of the fresh chirp.
                s.fb = flush(self.gain * (back + drip * 0.25 * (c - back)));
                y[k] = back + 0.3 * c;
            }
            let mix = self.mix.glide();
            l[i] = lerp(l[i], y[0], mix);
            r[i] = lerp(r[i], y[1], mix);
        }
    }

    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        let frames = (80.0 * 1.13 * rate as f32 * 0.001) as usize + 8;
        self.springs = [OneSpring::new(frames), OneSpring::new(frames)];
        self.length.set_time(rate, 200.0);
        self.drip.set_time(rate, 30.0);
        self.mix.set_time(rate, 20.0);
        for id in 0..SPRING_PARAMS.len() as u16 {
            self.apply(id, self.v[id as usize]);
        }
        self.clear();
    }

    fn clear(&mut self) {
        for s in &mut self.springs {
            s.clear();
        }
        self.in_hp.reset();
        self.length.snap(self.length.target);
        self.drip.snap(self.drip.target);
        self.mix.snap(self.mix.target);
    }

    fn tail(&self) -> u32 {
        ((self.v[spring::DECAY as usize] * 0.0012 + 0.2) * self.rate as f32) as u32
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(Spring, SPRING_INFO, SPRING_PARAMS);
