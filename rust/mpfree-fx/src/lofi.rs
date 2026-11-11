//! Lo-fi: worn tape and dusty vinyl, with one Amount knob over all of it,
//! so it can be a whisper of age or a broken machine.
//!
//! - Tape: wow (slow pitch drift) and flutter (fast waver) from a delay
//!   swept by two pairs of sines at unrelated rates, so the drift never
//!   sounds like a plain vibrato; and saturation, soft and a little
//!   asymmetric, which rounds peaks and adds warmth.
//! - Vinyl: crackle (clicks and the odd pop, random in size, length and
//!   place) and hiss. Both ride the music: they rise with the input and
//!   fade over about a second once it stops, so a stopped track is silent
//!   and a bypassed slot rests.
//! - Tone: a low cut, a high cut and width (age narrows the image).
//!
//! Amount scales everything: wow, flutter, saturation, crackle and noise
//! go to nothing at 0 %, the cuts open to 20 Hz and 20 kHz and the width
//! to full. The tape delay costs a fixed 2 ms of latency, reported for
//! compensation; the dry path is delayed to match, so Mix never combs.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::{Segs, Svf, SvfMode};
use crate::{db_to_gain, defaults, flush, lerp, soft, FixedDelay, Preset, Rng};

/// [`LoFi`] parameter ids.
pub mod ids {
    pub const AMOUNT: u16 = 0;
    pub const WOW: u16 = 1;
    pub const FLUTTER: u16 = 2;
    pub const SATURATION: u16 = 3;
    pub const CRACKLE: u16 = 4;
    pub const NOISE: u16 = 5;
    pub const LOW_CUT: u16 = 6;
    pub const HIGH_CUT: u16 = 7;
    pub const WIDTH: u16 = 8;
    pub const MIX: u16 = 9;
}

static INFO: NodeInfo = NodeInfo { id: "mpfree.fx.lofi", name: "Lo-fi", kind: Kind::Effect };
static PARAMS: [ParamInfo; 10] = [
    ParamInfo { id: 0, name: "Amount", group: "Lo-fi", min: 0.0, max: 100.0, default: 60.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 1, name: "Wow", group: "Tape", min: 0.0, max: 100.0, default: 40.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 2, name: "Flutter", group: "Tape", min: 0.0, max: 100.0, default: 30.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 3, name: "Saturation", group: "Tape", min: 0.0, max: 100.0, default: 40.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 4, name: "Crackle", group: "Vinyl", min: 0.0, max: 100.0, default: 30.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 5, name: "Noise", group: "Vinyl", min: 0.0, max: 100.0, default: 20.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 6, name: "Low cut", group: "Tone", min: 20.0, max: 500.0, default: 80.0, unit: Unit::Hz, curve: Curve::Log, readonly: false },
    ParamInfo { id: 7, name: "High cut", group: "Tone", min: 2_000.0, max: 20_000.0, default: 8_000.0, unit: Unit::Hz, curve: Curve::Log, readonly: false },
    ParamInfo { id: 8, name: "Width", group: "Tone", min: 0.0, max: 100.0, default: 70.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 9, name: "Mix", group: "Output", min: 0.0, max: 100.0, default: 100.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
];

/// Factory presets (Amount and Mix left at their defaults).
pub static PRESETS: &[Preset] = &[
    Preset { name: "Worn tape", values: &[(ids::WOW, 60.0), (ids::FLUTTER, 40.0), (ids::SATURATION, 60.0), (ids::CRACKLE, 0.0), (ids::NOISE, 25.0), (ids::LOW_CUT, 50.0), (ids::HIGH_CUT, 11_000.0), (ids::WIDTH, 85.0)] },
    Preset { name: "Dusty vinyl", values: &[(ids::WOW, 15.0), (ids::FLUTTER, 5.0), (ids::SATURATION, 20.0), (ids::CRACKLE, 75.0), (ids::NOISE, 30.0), (ids::LOW_CUT, 70.0), (ids::HIGH_CUT, 7_500.0), (ids::WIDTH, 60.0)] },
    Preset { name: "Cassette", values: &[(ids::WOW, 45.0), (ids::FLUTTER, 65.0), (ids::SATURATION, 50.0), (ids::CRACKLE, 0.0), (ids::NOISE, 45.0), (ids::LOW_CUT, 70.0), (ids::HIGH_CUT, 6_000.0), (ids::WIDTH, 55.0)] },
    Preset { name: "Old radio", values: &[(ids::WOW, 0.0), (ids::FLUTTER, 10.0), (ids::SATURATION, 85.0), (ids::CRACKLE, 35.0), (ids::NOISE, 40.0), (ids::LOW_CUT, 420.0), (ids::HIGH_CUT, 3_200.0), (ids::WIDTH, 0.0)] },
];

/// The tape delay's centre: the latency, and room for the widest wow.
const LATENCY_MS: f32 = 2.0;
/// Peak pitch deviation at full wow and full flutter.
const WOW_DEV: f32 = 0.004;
const FLUTTER_DEV: f32 = 0.0015;
/// Their rates in Hz: two sines each, at rates that never line up.
const WOW_HZ: [f32; 2] = [0.53, 0.89];
const FLUTTER_HZ: [f32; 2] = [7.1, 11.3];
/// Full saturation drives 18 dB into the shaper.
const SAT_DB: f32 = 18.0;
/// Clicks per second at full crackle.
const CLICKS_PER_S: f32 = 40.0;
/// Hiss at full noise, before the music envelope: about -45 dBFS RMS.
const HISS: f32 = 0.01;
/// The music envelope: attack and release time constants in seconds, and
/// the input level from which crackle and hiss play in full.
const ENV_ATTACK_S: f32 = 0.01;
const ENV_RELEASE_S: f32 = 0.35;
const ENV_FULL: f32 = 0.125;

/// A sine of `phase` cycles (polynomial, the same everywhere).
#[inline]
fn sine(phase: f32) -> f32 {
    let x = if phase < 0.5 { phase } else { phase - 1.0 };
    let y = 8.0 * x - 16.0 * x * x.abs();
    0.225 * (y * y.abs() - y) + y
}

#[inline]
fn hermite(x0: f32, x1: f32, x2: f32, x3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x2 - x0);
    let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
    let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
    ((c3 * t + c2) * t + c1) * t + x1
}

/// One channel's tape delay.
#[derive(Clone, Debug, Default)]
struct Tape {
    buf: Vec<f32>,
    mask: usize,
    w: usize,
}

impl Tape {
    fn new(frames: usize) -> Tape {
        let n = (frames + 8).next_power_of_two();
        Tape { buf: vec![0.0; n], mask: n - 1, w: 0 }
    }
    /// Writes `x`, then reads `d` frames back (0 is `x` itself; `d` at
    /// least 1).
    #[inline]
    fn run(&mut self, x: f32, d: f32) -> f32 {
        self.buf[self.w] = x;
        let last = self.w;
        self.w = (self.w + 1) & self.mask;
        let d = d.clamp(1.0, (self.buf.len() - 4) as f32);
        let i = d as usize;
        let t = d - i as f32;
        let at = |k: usize| self.buf[last.wrapping_sub(k) & self.mask];
        hermite(at(i - 1), at(i), at(i + 1), at(i + 2), t)
    }
    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.w = 0;
    }
}

/// Crackle for one channel: a burst of noise with a fast decay per click.
#[derive(Clone, Copy, Debug)]
struct Crackle {
    rng: Rng,
    level: f32,
    decay: f32,
    /// A one-pole low-pass on the burst: big pops are duller.
    lp: f32,
    lp_a: f32,
}

impl Crackle {
    fn new(seed: u32) -> Crackle { Crackle { rng: Rng::new(seed), level: 0.0, decay: 0.0, lp: 0.0, lp_a: 1.0 } }
    #[inline]
    fn run(&mut self, chance: f32, size: f32, rate: f32) -> f32 {
        // A new click: mostly small, now and then a pop (size cubed).
        let roll = 0.5 * (self.rng.bipolar() + 1.0);
        if roll < chance {
            let u = 0.5 * (self.rng.bipolar() + 1.0);
            let big = u * u * u;
            self.level = size * (0.05 + 0.6 * big);
            // 0.1 ms for a tick, up to 1 ms for a pop.
            self.decay = (-1.0 / ((0.0001 + 0.0009 * big) * rate)).exp();
            self.lp_a = 1.0 - (-2.0 * std::f32::consts::PI * (9_000.0 - 7_000.0 * big) / rate).exp();
        }
        if self.level == 0.0 { return 0.0; }
        let n = self.rng.bipolar();
        let x = n * self.level;
        self.level = flush(self.level * self.decay);
        self.lp = flush(self.lp + self.lp_a * (x - self.lp));
        self.lp
    }
    fn reset(&mut self, seed: u32) { *self = Crackle::new(seed); }
}

/// Tape wear and vinyl dust with one Amount over all of it.
pub struct LoFi {
    v: [f32; 10],
    rate: u32,
    amount: Smooth,
    wow: Smooth,
    flutter: Smooth,
    sat: Smooth,
    crackle: Smooth,
    noise: Smooth,
    log_low: Smooth,
    log_high: Smooth,
    width: Smooth,
    mix: Smooth,
    /// Modulator phases (cycles) for the two wow and two flutter sines.
    phase: [f64; 4],
    inc: [f64; 4],
    tape: [Tape; 2],
    dry: [FixedDelay; 2],
    latency: u32,
    low: Svf,
    high: Svf,
    clicks: [Crackle; 2],
    hiss: Rng,
    hiss_lp: [f32; 2],
    hiss_a: f32,
    env: f32,
    env_up: f32,
    env_down: f32,
}

const SEEDS: [u32; 3] = [0x51ED_270B, 0x2F6C_A3E1, 0x9B3C_57D4];

impl Default for LoFi {
    fn default() -> Self { Self::new() }
}

impl LoFi {
    pub fn new() -> LoFi {
        let v = defaults(&PARAMS);
        let mut f = LoFi {
            v,
            rate: 0,
            amount: Smooth::new(v[0] / 100.0),
            wow: Smooth::new(v[1] / 100.0),
            flutter: Smooth::new(v[2] / 100.0),
            sat: Smooth::new(v[3] / 100.0),
            crackle: Smooth::new(v[4] / 100.0),
            noise: Smooth::new(v[5] / 100.0),
            log_low: Smooth::new(v[6].log2()),
            log_high: Smooth::new(v[7].log2()),
            width: Smooth::new(v[8] / 100.0),
            mix: Smooth::new(v[9] / 100.0),
            phase: [0.0, 0.37, 0.0, 0.61],
            inc: [0.0; 4],
            tape: [Tape::default(), Tape::default()],
            dry: [FixedDelay::default(), FixedDelay::default()],
            latency: 0,
            low: Svf::default(),
            high: Svf::default(),
            clicks: [Crackle::new(SEEDS[0]), Crackle::new(SEEDS[1])],
            hiss: Rng::new(SEEDS[2]),
            hiss_lp: [0.0; 2],
            hiss_a: 1.0,
            env: 0.0,
            env_up: 1.0,
            env_down: 1.0,
        };
        f.prepare(48_000, mpfree_node::MAX_BLOCK);
        f.reset();
        f
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else { return };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        let target = match id {
            ids::LOW_CUT | ids::HIGH_CUT => v.log2(),
            _ => v / 100.0,
        };
        self.smooth(id).target = target;
    }

    fn smooth(&mut self, id: u16) -> &mut Smooth {
        match id {
            ids::AMOUNT => &mut self.amount,
            ids::WOW => &mut self.wow,
            ids::FLUTTER => &mut self.flutter,
            ids::SATURATION => &mut self.sat,
            ids::CRACKLE => &mut self.crackle,
            ids::NOISE => &mut self.noise,
            ids::LOW_CUT => &mut self.log_low,
            ids::HIGH_CUT => &mut self.log_high,
            ids::WIDTH => &mut self.width,
            _ => &mut self.mix,
        }
    }

    fn each_smooth(&mut self, f: &mut dyn FnMut(&mut Smooth)) {
        for s in [&mut self.amount, &mut self.wow, &mut self.flutter, &mut self.sat, &mut self.crackle, &mut self.noise, &mut self.log_low, &mut self.log_high, &mut self.width, &mut self.mix] { f(s); }
    }

    /// The cut filters for the current Amount and corners: at Amount 0
    /// they open to 20 Hz and 20 kHz.
    fn filters(&mut self) {
        let a = self.amount.value;
        let low = lerp(20f32.log2(), self.log_low.value, a).exp2();
        let high = lerp(20_000f32.log2(), self.log_high.value, a).exp2();
        self.low.set(SvfMode::HighPass, low, std::f32::consts::FRAC_1_SQRT_2, 0.0, self.rate);
        self.high.set(SvfMode::LowPass, high, std::f32::consts::FRAC_1_SQRT_2, 0.0, self.rate);
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let fs = self.rate as f32;
        let centre = self.latency as f32;
        // Delay swing for a peak pitch deviation `dev` at `hz`: dev * fs / (2 pi hz).
        let tau = 2.0 * std::f32::consts::PI;
        let wow_frames = WOW_DEV * fs / (tau * 0.5 * (WOW_HZ[0] + WOW_HZ[1]));
        let flutter_frames = FLUTTER_DEV * fs / (tau * 0.5 * (FLUTTER_HZ[0] + FLUTTER_HZ[1]));
        for i in 0..l.len() {
            if !(self.amount.settled() && self.log_low.settled() && self.log_high.settled()) {
                self.amount.next();
                self.log_low.next();
                self.log_high.next();
                self.filters();
            }
            let a = self.amount.value;
            let wow = a * self.wow.next();
            let flutter = a * self.flutter.next();
            let sat = a * self.sat.next();
            let crackle = a * self.crackle.next();
            let noise = a * self.noise.next();
            let width = 1.0 - a * (1.0 - self.width.next());
            let mix = self.mix.next();
            for (p, inc) in self.phase.iter_mut().zip(&self.inc) {
                *p += inc;
                if *p >= 1.0 { *p -= 1.0; }
            }
            let w = 0.6 * sine(self.phase[0] as f32) + 0.4 * sine(self.phase[1] as f32);
            let fl = 0.7 * sine(self.phase[2] as f32) + 0.3 * sine(self.phase[3] as f32);
            let d = centre + wow * wow_frames * w + flutter * flutter_frames * fl;
            let x = [l[i], r[i]];
            // The music envelope that crackle and hiss ride.
            let level = x[0].abs().max(x[1].abs());
            let k = if level > self.env { self.env_up } else { self.env_down };
            self.env = flush(self.env + k * (level - self.env));
            let ride = (self.env / ENV_FULL).min(1.0);
            let chance = crackle * CLICKS_PER_S / fs;
            let drive = db_to_gain(SAT_DB * sat);
            let bias = 0.15 * sat;
            let mut y = [0.0f32; 2];
            for ch in 0..2 {
                let mut s = self.tape[ch].run(x[ch], d);
                if sat > 0.0 {
                    // Asymmetric, so it adds even harmonics; the bias's own
                    // offset is taken back out, and the low cut removes the
                    // rest of the DC it leaves on a changing signal.
                    s = (soft(drive * s + bias) - soft(bias)) / drive;
                }
                s = self.low.run(ch, s);
                s = self.high.run(ch, s);
                if ride > 0.0 {
                    let h = self.hiss.bipolar();
                    self.hiss_lp[ch] = flush(self.hiss_lp[ch] + self.hiss_a * (h - self.hiss_lp[ch]));
                    s += ride * (noise * HISS * self.hiss_lp[ch] + self.clicks[ch].run(chance, crackle, fs));
                }
                y[ch] = s;
            }
            // Width: narrow the side signal.
            let mid = 0.5 * (y[0] + y[1]);
            let side = 0.5 * (y[0] - y[1]) * width;
            let dry = [self.dry[0].run(x[0]), self.dry[1].run(x[1])];
            l[i] = lerp(dry[0], mid + side, mix);
            r[i] = lerp(dry[1], mid - side, mix);
        }
    }
}

impl Node for LoFi {
    fn info(&self) -> &'static NodeInfo { &INFO }
    fn params(&self) -> &'static [ParamInfo] { &PARAMS }
    fn set_param(&mut self, id: u16, value: f32) { self.apply(id, value); }
    fn param(&self, id: u16) -> f32 { self.v.get(id as usize).copied().unwrap_or(f32::NAN) }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        let rate = rate.max(1);
        let fs = rate as f32;
        if rate != self.rate {
            self.latency = (LATENCY_MS * 0.001 * fs).round() as u32;
            let frames = 2 * self.latency as usize + 8;
            self.tape = [Tape::new(frames), Tape::new(frames)];
            self.dry = [FixedDelay::new(self.latency as usize), FixedDelay::new(self.latency as usize)];
        }
        self.rate = rate;
        self.each_smooth(&mut |s| s.set_time(rate, 30.0));
        for (inc, hz) in self.inc.iter_mut().zip(WOW_HZ.iter().chain(&FLUTTER_HZ)) { *inc = f64::from(*hz) / f64::from(rate); }
        self.hiss_a = 1.0 - (-2.0 * std::f32::consts::PI * 6_000.0f32.min(0.45 * fs) / fs).exp();
        self.env_up = 1.0 - (-1.0 / (ENV_ATTACK_S * fs)).exp();
        self.env_down = 1.0 - (-1.0 / (ENV_RELEASE_S * fs)).exp();
        self.filters();
    }
    fn reset(&mut self) {
        self.each_smooth(&mut |s| s.snap(s.target));
        for t in &mut self.tape { t.clear(); }
        for d in &mut self.dry { d.reset(); }
        self.low.reset();
        self.high.reset();
        self.clicks[0].reset(SEEDS[0]);
        self.clicks[1].reset(SEEDS[1]);
        self.hiss = Rng::new(SEEDS[2]);
        self.hiss_lp = [0.0; 2];
        self.env = 0.0;
        self.phase = [0.0, 0.37, 0.0, 0.61];
        self.filters();
    }
    fn process(&mut self, _ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| if let NodeEvent::Param { id, value } = *ev { self.apply(id, value) }) {
            self.run(&mut left[from..to], &mut right[from..to]);
        }
    }
    fn latency(&self) -> u32 { self.latency }
    fn tail_frames(&self) -> u32 {
        let fs = self.rate as f32;
        // The tape delay and the filters' settling; while crackle or hiss
        // is on, the music envelope's release down 80 dB (from full).
        let mut secs = 0.05;
        if self.v[4] > 0.0 || self.v[5] > 0.0 { secs += ENV_RELEASE_S * (1e4f32 / ENV_FULL).ln(); }
        self.latency + (secs * fs) as u32
    }
}
