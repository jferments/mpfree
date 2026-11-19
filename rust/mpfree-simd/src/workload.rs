//! The kernels' bench workloads: the work an engine gives a kernel in a
//! busy song, built here once so the bench (`mpfree-bench`'s "Supersaw
//! saws" and "Choir formants" cases), the ARMv7 instruction count
//! (`examples/simd_cost.rs`, `tools/simd_arm_cost.py`) and the tests all
//! play the same thing.
//!
//! Building one allocates; playing a block doesn't.

use crate::{formants4_on, table_sum_on, Formants4, Path, FORMANTS};

/// Levels of the saw, half an octave of bandwidth apart, as
/// `mpfree_synth::osc` keeps them: the harmonics each level holds.
const LEVEL_HARMONICS: [usize; 17] = [
    512, 362, 256, 181, 128, 90, 64, 45, 32, 22, 16, 11, 8, 5, 4, 2, 1,
];
/// The saws of a supersaw.
pub const SAWS: usize = 7;
/// Frames per control step: Poly calls its oscillators once a step.
pub const STEP: usize = 16;

/// One level of a band-limited saw: its entries ([sample, step]) and the
/// shift that indexes them.
pub struct Level {
    pub table: Box<[[f32; 2]]>,
    pub shift: u32,
}

/// A band-limited saw (a rising ramp from -1 to 1) at every level, the
/// sizes `mpfree_synth::osc` uses: at least eight samples per cycle of the
/// top harmonic, 512 to 4096 entries.
pub fn saw_levels() -> Vec<Level> {
    LEVEL_HARMONICS
        .iter()
        .map(|&h| {
            let len = level_len(h);
            let sin: Vec<f64> = (0..len)
                .map(|n| (std::f64::consts::TAU * n as f64 / len as f64).sin())
                .collect();
            // 2p - 1 = -(2 / pi) sum sin(2 pi k p) / k.
            let x: Vec<f64> = (0..len)
                .map(|n| {
                    let s: f64 = (1..=h.min(len / 2 - 1))
                        .map(|k| sin[(k * n) % len] / k as f64)
                        .sum();
                    -2.0 / std::f64::consts::PI * s
                })
                .collect();
            let per = len as f64 / 4_294_967_296.0;
            Level {
                table: (0..len)
                    .map(|n| {
                        let (a, b) = (x[n], x[(n + 1) % len]);
                        [a as f32, ((b - a) * per) as f32]
                    })
                    .collect(),
                shift: 32 - len.trailing_zeros(),
            }
        })
        .collect()
}

/// A level's length for `h` harmonics: at least eight samples per cycle
/// of the top one, 512 to 4096.
fn level_len(h: usize) -> usize {
    (h * 8).next_power_of_two().clamp(512, 4096)
}

/// Plain ramps (not band-limited) with the sizes of [`saw_levels`], built in
/// a moment: for counting instructions under qemu, where only the sizes
/// matter and the band-limited build would take most of the run.
pub fn ramp_levels() -> Vec<Level> {
    LEVEL_HARMONICS
        .iter()
        .map(|&h| {
            let len = level_len(h);
            let per = len as f64 / 4_294_967_296.0;
            let at = |n: usize| 2.0 * (n % len) as f64 / len as f64 - 1.0;
            Level {
                table: (0..len)
                    .map(|n| [at(n) as f32, ((at(n + 1) - at(n)) * per) as f32])
                    .collect(),
                shift: 32 - len.trailing_zeros(),
            }
        })
        .collect()
}

/// The level whose harmonics all stay below half the rate for a phase
/// increment of `dt` cycles a frame.
pub fn level_for(dt: f64) -> usize {
    LEVEL_HARMONICS
        .iter()
        .position(|&h| h as f64 * dt <= 0.5)
        .unwrap_or(LEVEL_HARMONICS.len() - 1)
}

/// Each saw's offset from the note at full detune (Szabo's measurements,
/// as `mpfree_synth::osc`).
const OFFSETS: [f64; SAWS] = [
    -0.110_023_13,
    -0.062_884_39,
    -0.019_523_56,
    0.0,
    0.019_912_21,
    0.062_165_38,
    0.107_452_42,
];

struct Voice {
    level: usize,
    ph: [u32; SAWS],
    inc: [u32; SAWS],
    gain: [f32; SAWS],
    pan: (f32, f32),
}

/// Supersaw voices as Poly plays them: each voice's seven saws read one
/// saw level, one call a voice for each 16-frame control step, into a
/// step buffer added into the left and right outputs.
pub struct Supersaw {
    levels: Vec<Level>,
    voices: Vec<Voice>,
    step: [f32; STEP],
}

impl Supersaw {
    /// `voices` voices of a chord over three octaves from C2, at `rate`,
    /// with a middling detune and mix, every saw starting at its own
    /// phase.
    pub fn new(voices: usize, rate: u32) -> Supersaw {
        Supersaw::with_levels(voices, rate, saw_levels())
    }

    /// [`Supersaw::new`] reading `levels` (17 of them, as [`saw_levels`]).
    pub fn with_levels(voices: usize, rate: u32, levels: Vec<Level>) -> Supersaw {
        const NOTES: [u8; 8] = [36, 43, 48, 52, 55, 60, 64, 67];
        let mut seed = 0x1234_5678u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed
        };
        let detune = 0.25;
        let (mid, side) = (0.72f64, 0.36f64);
        let norm = 1.0 / (mid * mid + 6.0 * side * side).sqrt();
        let voices = (0..voices)
            .map(|v| {
                let note = NOTES[v % NOTES.len()] + 12 * (v / NOTES.len()) as u8;
                let hz = 440.0 * 2f64.powf((f64::from(note) - 69.0) / 12.0);
                let dt = hz / f64::from(rate);
                let top = 1.0 + OFFSETS[SAWS - 1] * detune;
                let gain = 0.25 / (voices as f64).sqrt();
                let x = v as f32 / voices.max(2) as f32;
                Voice {
                    level: level_for(dt * top),
                    ph: std::array::from_fn(|_| rnd()),
                    inc: std::array::from_fn(|k| {
                        (dt * (1.0 + OFFSETS[k] * detune) * 4_294_967_296.0) as u32
                    }),
                    gain: std::array::from_fn(|k| {
                        ((if k == 3 { mid } else { side }) * norm * gain) as f32
                    }),
                    pan: (1.0 - 0.5 * x, 0.5 + 0.5 * x),
                }
            })
            .collect();
        Supersaw {
            levels,
            voices,
            step: [0.0; STEP],
        }
    }

    /// Plays one block into `left` and `right` (written, not added) on
    /// `path`, in 16-frame steps. Returns the path the last call took.
    pub fn block(&mut self, path: Path, left: &mut [f32], right: &mut [f32]) -> Path {
        let mut took = path;
        left.fill(0.0);
        right.fill(0.0);
        let n = left.len().min(right.len());
        let mut at = 0;
        while at < n {
            let len = (n - at).min(STEP);
            for v in &mut self.voices {
                let lv = &self.levels[v.level];
                let step = &mut self.step[..len];
                took = table_sum_on::<false>(
                    path, &lv.table, lv.shift, step, &mut v.ph, &v.inc, &v.gain,
                );
                for ((l, r), &x) in left[at..at + len]
                    .iter_mut()
                    .zip(&mut right[at..at + len])
                    .zip(step.iter())
                {
                    *l += x * v.pan.0;
                    *r += x * v.pan.1;
                }
            }
            at += len;
        }
        took
    }
}

/// A vowel: each formant's centre (Hz), bandwidth (Hz) and level (dB).
struct Vowel([(f64, f64, f64); FORMANTS]);

/// A soprano's "ah" and an alto's "oh" (the usual published formant
/// tables): the two groups of the Choir workload.
const VOWELS: [Vowel; 2] = [
    Vowel([
        (800.0, 80.0, 0.0),
        (1150.0, 90.0, -6.0),
        (2900.0, 120.0, -32.0),
        (3900.0, 130.0, -20.0),
        (4950.0, 140.0, -50.0),
    ]),
    Vowel([
        (450.0, 70.0, 0.0),
        (800.0, 80.0, -9.0),
        (2830.0, 100.0, -16.0),
        (3500.0, 130.0, -28.0),
        (4950.0, 135.0, -55.0),
    ]),
];

/// One group of four voices: their filters, their buzz phases and pans.
struct Group {
    f: Formants4,
    ph: [f32; 4],
    dt: [f32; 4],
    pan: [(f32, f32); 4],
}

/// Choir voices as the Choir engine plays them: four voices a group, one
/// voice a lane, each a buzzy source (a plain saw) through its vowel's
/// five formant filters, one `formants4` call a group for each 16-frame
/// control step. The coefficients are set once here; the engine's own
/// control-rate work is the same on both paths and stays out of the
/// count.
pub struct Formants {
    groups: Vec<Group>,
    src: [[f32; 4]; STEP],
    out: [[f32; 4]; STEP],
}

impl Formants {
    /// `voices` voices (a multiple of four plays every lane; the rest of
    /// the last group is silent) of a chord from C3, at `rate`, half of
    /// the groups singing "ah" and half "oh".
    pub fn new(voices: usize, rate: u32) -> Formants {
        const NOTES: [u8; 8] = [48, 55, 60, 64, 67, 72, 76, 79];
        let fs = f64::from(rate);
        let gain = 0.5 / (voices.max(1) as f64).sqrt();
        let groups = (0..voices.div_ceil(4))
            .map(|gi| {
                let vowel = &VOWELS[gi % VOWELS.len()];
                let mut f = Formants4::default();
                for (k, &(fc, bw, db)) in vowel.0.iter().enumerate() {
                    // Simper's SVF: g = tan(pi fc / fs), k = 1 / Q.
                    let g = (std::f64::consts::PI * fc.min(0.45 * fs) / fs).tan();
                    let kk = bw / fc;
                    let a1 = 1.0 / (1.0 + g * (g + kk));
                    let a2 = g * a1;
                    let a3 = g * a2;
                    // The band-pass output peaks at 1 / k: scale it to the
                    // formant's level.
                    let level = 10f64.powf(db / 20.0) * kk;
                    for lane in 0..4 {
                        let on = gi * 4 + lane < voices;
                        f.a1[k][lane] = a1 as f32;
                        f.a2[k][lane] = a2 as f32;
                        f.a3[k][lane] = a3 as f32;
                        f.g[k][lane] = if on { (level * gain) as f32 } else { 0.0 };
                    }
                }
                let dt = std::array::from_fn(|lane| {
                    let v = gi * 4 + lane;
                    if v >= voices {
                        return 0.0;
                    }
                    let note = NOTES[v % NOTES.len()] + 12 * (v / NOTES.len()) as u8;
                    (440.0 * 2f64.powf((f64::from(note) - 69.0) / 12.0) / fs) as f32
                });
                let pan = std::array::from_fn(|lane| {
                    let x = (gi * 4 + lane) as f32 / voices.max(2) as f32;
                    (1.0 - 0.5 * x, 0.5 + 0.5 * x)
                });
                Group {
                    f,
                    ph: std::array::from_fn(|lane| lane as f32 * 0.23),
                    dt,
                    pan,
                }
            })
            .collect();
        Formants {
            groups,
            src: [[0.0; 4]; STEP],
            out: [[0.0; 4]; STEP],
        }
    }

    /// Plays one block into `left` and `right` (written, not added) on
    /// `path`, in 16-frame steps. Returns the path the last call took.
    pub fn block(&mut self, path: Path, left: &mut [f32], right: &mut [f32]) -> Path {
        let mut took = path;
        left.fill(0.0);
        right.fill(0.0);
        let n = left.len().min(right.len());
        let mut at = 0;
        while at < n {
            let len = (n - at).min(STEP);
            for g in &mut self.groups {
                for row in &mut self.src[..len] {
                    for lane in 0..4 {
                        let mut p = g.ph[lane] + g.dt[lane];
                        if p >= 1.0 {
                            p -= 1.0;
                        }
                        g.ph[lane] = p;
                        row[lane] = 2.0 * p - 1.0;
                    }
                }
                took = formants4_on(path, &self.src[..len], &mut self.out[..len], &mut g.f);
                for ((l, r), y) in left[at..at + len]
                    .iter_mut()
                    .zip(&mut right[at..at + len])
                    .zip(&self.out[..len])
                {
                    for lane in 0..4 {
                        *l += y[lane] * g.pan[lane].0;
                        *r += y[lane] * g.pan[lane].1;
                    }
                }
            }
            at += len;
        }
        took
    }
}
