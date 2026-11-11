//! Turning desktop numbers into MPC numbers.
//!
//! A desktop core and a Cortex-A17 differ by a different factor for each
//! kind of work: plain float arithmetic, vectorisable loops, library maths
//! (`sinf`, `expf`, and `floorf`, which is a library call on ARMv7), reads
//! that miss the cache, and branchy control code. So the model is a small
//! set of reference kernels, one per kind, each about one 128-frame stereo
//! block of that work:
//!
//! 1. Run [`KERNELS`] on the device once per build setting (`mpfree-bench
//!    --calibrate` prints a calibration file).
//! 2. Every desktop run times the same kernels on the desktop, so the ratio
//!    `device / desktop` for each kind is known for this pair of machines.
//! 3. A desktop measurement times the ratios gives the estimate: the
//!    geometric mean as the figure, the lowest and highest ratio as its
//!    range.
//!
//! The estimate is a guide for code that has not been to the device yet. A
//! number measured on the device always wins, and every engine's numbers
//! are measured there in the test rounds.

use std::hint::black_box;
use std::time::Instant;

/// Frames per kernel call: one stereo 128-frame block.
const N: usize = 256;
/// The table the cache-miss kernel reads: 8 MiB, larger than any cache on
/// either machine.
const TABLE: usize = 2 << 20;

/// Memory the kernels work in, made once (it allocates).
pub struct Scratch {
    table: Vec<f32>,
    buf: Vec<f32>,
    srcs: Vec<f32>,
    seed: u32,
}

impl Default for Scratch {
    fn default() -> Scratch { Scratch::new() }
}

impl Scratch {
    pub fn new() -> Scratch {
        let mut s = 0x1234_5678u32;
        let mut noise = move || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 9) as f32 / (1u32 << 23) as f32 - 1.0
        };
        Scratch {
            table: (0..TABLE).map(|_| noise()).collect(),
            buf: (0..N).map(|_| noise()).collect(),
            srcs: (0..8 * N).map(|_| noise()).collect(),
            seed: 1,
        }
    }
}

/// One kind of work.
pub struct Kernel {
    pub name: &'static str,
    pub what: &'static str,
    pub run: fn(&mut Scratch) -> f32,
}

/// The reference kernels.
pub static KERNELS: &[Kernel] = &[
    Kernel { name: "biquad", what: "4 cascaded biquads, scalar float with a dependency chain (filters, EQ)", run: biquad },
    Kernel { name: "mix", what: "8 sources gained and summed (mixing, gains: vectorisable)", run: mix },
    Kernel { name: "libm", what: "sinf, expf and floorf per sample (oscillators, envelopes, curves)", run: libm },
    Kernel { name: "random_read", what: "interpolated reads at random places in 8 MiB (grains, delays, reverbs)", run: random_read },
    Kernel { name: "control", what: "integer search and branches over 64 slots (voice allocation, events)", run: control },
];

fn biquad(s: &mut Scratch) -> f32 {
    let (b0, b1, b2, a1, a2) = (0.2066f32, 0.4132, 0.2066, -0.3695, 0.1958);
    let mut z = [[0.0f32; 2]; 4];
    let mut acc = 0.0;
    for &x in &s.buf {
        let mut v = x;
        for st in z.iter_mut() {
            let y = b0 * v + st[0];
            st[0] = b1 * v - a1 * y + st[1];
            st[1] = b2 * v - a2 * y;
            v = y;
        }
        acc += v;
    }
    acc
}

fn mix(s: &mut Scratch) -> f32 {
    let gains = [0.9f32, 0.8, 0.7, 0.6, 0.5, 0.4, 0.3, 0.2];
    for x in s.buf.iter_mut() { *x *= 0.5; }
    for (src, g) in s.srcs.chunks_exact(N).zip(gains) {
        for (o, &x) in s.buf.iter_mut().zip(src) { *o += g * x; }
    }
    // Keep the buffer bounded across calls.
    for x in s.buf.iter_mut() { *x = x.clamp(-4.0, 4.0); }
    s.buf[N / 2]
}

fn libm(s: &mut Scratch) -> f32 {
    let mut acc = 0.0f32;
    for (i, &x) in s.buf.iter().enumerate() {
        let ph = x * 3.0 + i as f32 * 0.01;
        acc += ph.sin() + (-x.abs()).exp() + (ph * 7.3).floor();
    }
    acc
}

fn random_read(s: &mut Scratch) -> f32 {
    let mut acc = 0.0f32;
    let mut seed = s.seed;
    for _ in 0..N {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let i = (seed as usize >> 4) % (TABLE - 4);
        let t = (seed & 0xff) as f32 / 256.0;
        let (x0, x1, x2, x3) = (s.table[i], s.table[i + 1], s.table[i + 2], s.table[i + 3]);
        let c1 = 0.5 * (x2 - x0);
        let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
        let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
        acc += ((c3 * t + c2) * t + c1) * t + x1;
    }
    s.seed = seed;
    acc
}

fn control(s: &mut Scratch) -> f32 {
    let mut ages = [0u32; 64];
    let mut notes = [0u8; 64];
    let mut seed = s.seed ^ 0x9e37_79b9;
    let mut steals = 0u32;
    for step in 0..N as u32 {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let note = (seed >> 24) as u8 & 0x7f;
        // A free slot, or the slot playing this note, or the oldest.
        let mut pick = usize::MAX;
        let mut oldest = (u32::MAX, 0usize);
        for (k, (&a, &n)) in ages.iter().zip(&notes).enumerate() {
            if a == 0 || n == note { pick = k; break; }
            if a < oldest.0 { oldest = (a, k); }
        }
        if pick == usize::MAX { pick = oldest.1; steals += 1; }
        ages[pick] = step + 1;
        notes[pick] = note;
        if seed & 7 == 0 { ages[(seed as usize >> 8) & 63] = 0; }
    }
    s.seed = seed;
    steals as f32
}

/// Times one kernel: the median over `rounds` rounds of `calls` calls, in
/// ns per call. The median rejects a round that was preempted.
pub fn time_kernel(k: &Kernel, s: &mut Scratch, calls: u32, rounds: u32) -> f64 {
    let mut sink = 0.0f32;
    for _ in 0..calls.min(64) { sink += (k.run)(s); }
    let mut times: Vec<f64> = (0..rounds.max(1)).map(|_| {
        let t = Instant::now();
        for _ in 0..calls { sink += (k.run)(black_box(&mut *s)); }
        t.elapsed().as_nanos() as f64 / f64::from(calls.max(1))
    }).collect();
    black_box(sink);
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

/// Times every kernel on this machine: `(name, ns per call)`.
pub fn measure(calls: u32, rounds: u32) -> Vec<(String, f64)> {
    let mut s = Scratch::new();
    KERNELS.iter().map(|k| (k.name.to_string(), time_kernel(k, &mut s, calls, rounds))).collect()
}

/// Kernel timings from one machine and build, as a text file:
///
/// ```text
/// # mpfree-bench calibration
/// machine MPC One (RK3288, 1608 MHz), build armv7 +neon
/// biquad 812.5
/// mix 140.0
/// ```
///
/// `#` lines are comments; `machine` names where it was measured; every
/// other line is a kernel name and ns per call.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Calibration {
    pub machine: String,
    pub kernels: Vec<(String, f64)>,
}

impl Calibration {
    pub fn parse(text: &str) -> Result<Calibration, String> {
        let mut c = Calibration::default();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            if let Some(m) = line.strip_prefix("machine ") { c.machine = m.trim().to_string(); continue; }
            let mut f = line.split_whitespace();
            let (Some(name), Some(ns), None) = (f.next(), f.next(), f.next()) else {
                return Err(format!("line {}: expected `kernel ns`", n + 1));
            };
            let ns: f64 = ns.parse().map_err(|_| format!("line {}: {ns} is not a number", n + 1))?;
            if !(ns.is_finite() && ns > 0.0) { return Err(format!("line {}: {ns} must be above 0", n + 1)); }
            c.kernels.push((name.to_string(), ns));
        }
        if c.kernels.is_empty() { return Err("no kernels".to_string()); }
        Ok(c)
    }

    pub fn to_text(&self) -> String {
        let mut s = String::from("# mpfree-bench calibration: reference kernel, ns per call\n");
        if !self.machine.is_empty() { s.push_str(&format!("machine {}\n", self.machine)); }
        for (k, ns) in &self.kernels { s.push_str(&format!("{k} {ns:.1}\n")); }
        s
    }
}

/// How much slower the device is than this machine, for each kind of work
/// both measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub ratios: Vec<(String, f64)>,
}

impl Model {
    /// Ratios `device / host` for kernels in both lists.
    pub fn new(host: &[(String, f64)], device: &Calibration) -> Result<Model, String> {
        let ratios: Vec<(String, f64)> = host.iter().filter_map(|(k, h)| {
            let d = device.kernels.iter().find(|(n, _)| n == k)?.1;
            (*h > 0.0).then(|| (k.clone(), d / h))
        }).collect();
        if ratios.is_empty() { return Err("the calibration shares no kernel with this machine".to_string()); }
        Ok(Model { ratios })
    }

    /// The geometric mean ratio, the lowest and the highest.
    pub fn factor(&self) -> (f64, f64, f64) {
        let n = self.ratios.len() as f64;
        let geo = (self.ratios.iter().map(|(_, r)| r.ln()).sum::<f64>() / n).exp();
        let lo = self.ratios.iter().map(|r| r.1).fold(f64::INFINITY, f64::min);
        let hi = self.ratios.iter().map(|r| r.1).fold(0.0, f64::max);
        (geo, lo, hi)
    }

    /// A desktop time scaled to the device: (estimate, low, high).
    pub fn estimate(&self, host_ns: f64) -> (f64, f64, f64) {
        let (g, lo, hi) = self.factor();
        (host_ns * g, host_ns * lo, host_ns * hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernels_run_and_give_finite_results() {
        let mut s = Scratch::new();
        for k in KERNELS {
            for _ in 0..3 { assert!((k.run)(&mut s).is_finite(), "{}", k.name); }
            assert!(time_kernel(k, &mut s, 4, 3) > 0.0);
        }
    }

    #[test]
    fn calibration_round_trips_and_rejects_junk() {
        let c = Calibration { machine: "MPC One".to_string(), kernels: vec![("biquad".into(), 800.0), ("mix".into(), 120.5)] };
        assert_eq!(Calibration::parse(&c.to_text()).unwrap(), c);
        assert!(Calibration::parse("biquad fast\n").is_err());
        assert!(Calibration::parse("biquad -1\n").is_err());
        assert!(Calibration::parse("# nothing\n").is_err());
        assert!(Calibration::parse("biquad 1 2\n").is_err());
    }

    #[test]
    fn the_model_scales_by_the_ratios() {
        let host = vec![("biquad".to_string(), 100.0), ("mix".to_string(), 10.0), ("other".to_string(), 5.0)];
        let dev = Calibration { machine: String::new(), kernels: vec![("biquad".into(), 400.0), ("mix".into(), 160.0)] };
        let m = Model::new(&host, &dev).unwrap();
        assert_eq!(m.ratios.len(), 2);
        let (g, lo, hi) = m.factor();
        assert!((g - 8.0).abs() < 1e-9 && lo == 4.0 && hi == 16.0);
        let (e, l, h) = m.estimate(50_000.0);
        assert!((e - 400_000.0).abs() < 1e-6 && l == 200_000.0 && h == 800_000.0);
        assert!(Model::new(&host, &Calibration { machine: String::new(), kernels: vec![("x".into(), 1.0)] }).is_err());
    }
}
