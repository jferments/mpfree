//! DSP quality's filter measures (Y1a, Y1b, Y2a and Y2b of
//! `docs/dev/design/dsp-quality.md`, step 0.2, for step 2.2): every synth
//! and sampler filter on its own against the curve its page draws, and a
//! ratchet of their own (`tests/filters.rs`, `quality/filters.tsv`).
//!
//! - **The filters:** Poly's four (Wavetable runs the same,
//!   `mpfree_synth::dsp::Filter`), Analog's four, Squelch's and Bass's
//!   ladders, each through its engine's `FilterProbe` (the voice's own
//!   solve, coefficients and make-up, and for Squelch and Bass the inner
//!   rate and half-band), and every kind of the sampler's filter, played
//!   on a pad and divided by the same pad with its filter off. Each is
//!   held to the curve its page draws: `mpfree_synth_ui::visual::
//!   filter_gain`, `analog::filter_gain`, `squelch::response`,
//!   `bass::ladder_response` and `mpfree_sampler_ui::play_widgets::
//!   filter_db`. Resonance is a share of each panel's range.
//! - **Y1a, small signal:** the response to a -40 dBFS impulse, cutoff at
//!   5, 10 and 15 kHz, resonance 0, 0.5 and 0.9: the largest error below
//!   the cutoff where the drawn curve is within 24 dB of 0 dB (target 1
//!   dB), and from 18 to 20 kHz where it is within 30 dB (characterised,
//!   no target, kept by the ratchet).
//! - **Y1b, driven,** for the filters meant to saturate (Analog's,
//!   Squelch's, Bass's and the sampler's Ladder; the others are linear):
//!   sines at a quarter of the cutoff, half and the cutoff at -20, -10 and
//!   -3 dBFS against the same sines at -40 dBFS, how far the gain moves
//!   (reported, not kept: saturating is the filters' sound).
//! - **Y2a, resonance tracking:** at resonance 0.9, the measured
//!   response's peak against the drawn curve's, cutoff 100 Hz to 10 kHz
//!   (target 25 cents).
//! - **Y2b, self-oscillation,** for the filters meant to sing (Analog's
//!   LP 24, Bass's ladder, the sampler's Ladder): one -40 dBFS impulse at
//!   full resonance, then 10 seconds: the pitch over the last 0.74 s
//!   against the cutoff (target 10 cents) and how far the level moves
//!   from the fifth second to the last (target 1 dB).
//! - **Y1c, moving:** each filter moved the way its engine moves it, a
//!   control step at a time: Poly's coefficients and the sampler's held
//!   for a step, Analog's, Squelch's and Bass's cutoff in a straight line
//!   across it (the sampler's through [`PadMirror`], held to a real pad
//!   by a test). The moves: a fast filter envelope, an LFO, and the
//!   cutoff and the resonance each moved at audio rate. **Staying
//!   bounded:** fed noise, every frame and at the engine's step, every
//!   sample stays finite and the peak against the loudest the same noise
//!   gets with the settings held anywhere on the move (target 6 dB).
//!   **Stepping noise,** for the envelope and the LFO: fed a steady tone,
//!   the move at the engine's step against the same move every frame,
//!   octave band by octave band where the output is within 60 dB of its
//!   loudest band (target 3 dB). Every move repeats in a whole number of
//!   steps and the tone fits the window, so the spectra have no leakage,
//!   and a stepped move's lag doesn't count, only the noise it adds.

use std::f64::consts::TAU;

use mpfree_sampler::dsp::{
    vowel_formants, vowel_gain, vowel_resonance, Comb, Filter as PadFilter, FilterKind,
    Ladder as PadLadder, Svf, FORMANT_2,
};
use mpfree_synth::dsp::{Filter as SynthFilter, FilterCoefs, FilterMode};
use mpfree_synth::{analog, bass, poly, squelch};

use crate::fft::{power_spectrum, Fft};
use crate::quality::{Better, Row};
use crate::sampler;
use crate::signals::white;
use crate::synth::blackman_harris;

/// The small signal: -40 dBFS.
const SMALL: f64 = 0.01;
/// Y1a's cutoffs (Hz) and resonances (a share of the panel's range).
const CUTOFFS: [f32; 3] = [5_000.0, 10_000.0, 15_000.0];
const RESONANCES: [f32; 3] = [0.0, 0.5, 0.9];
/// Y1b's levels (dBFS peak).
const DRIVEN_DB: [f64; 3] = [-20.0, -10.0, -3.0];
/// Y2a's cutoffs and resonance.
const PEAK_CUTOFFS: [f32; 7] = [100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0];
const PEAK_RESONANCE: f32 = 0.9;
/// Y2b's cutoffs and how long each sings.
const SING_CUTOFFS: [f32; 4] = [110.0, 440.0, 1_760.0, 7_040.0];
const SING_SECS: usize = 10;
/// Y1a's window and Y2a's (frames at the host rate).
const IR_FRAMES: usize = 1 << 16;
const PEAK_FRAMES: usize = 1 << 17;
/// Where the pad's impulse sits in its sound: clear of the envelope's
/// start.
const PAD_LEAD: usize = 4_096;
/// Y1c: the resonances each move is measured at for staying bounded and
/// for stepping noise, the noise's peak and the tone's.
const BOUND_RESONANCES: [f32; 3] = [0.0, 0.9, 1.0];
const STEP_RESONANCES: [f32; 2] = [0.5, 0.9];
const BOUND_NOISE: f64 = 0.5;
const STEP_TONE: f64 = 0.1;
/// Y1c's stepping window and the frames run before it (host rate).
const STEP_FRAMES: usize = 1 << 16;
const STEP_SETTLE: usize = 1 << 14;
/// How often the sampler's voice sets its filter (frames;
/// `mpfree_sampler`'s private `FILTER_STEP`, held by
/// `the_pad_mirror_moves_as_a_pad_does`).
const PAD_STEP: usize = 16;

/// A filter at one setting: `x`, at the filter's own rate (`os` samples a
/// frame), to its output at the host rate, at the input's level.
struct Set {
    os: usize,
    run: Run,
}

type Run = Box<dyn FnMut(&[f64]) -> Vec<f64>>;

type Make = Box<dyn Fn(f32, f32) -> Set>;
type Drawn = Box<dyn Fn(f32, f32, f64) -> f64>;

/// One control step of a moving filter: the cutoff (Hz) and resonance the
/// step moves to, and its input at the filter's own rate; its output at
/// the host rate.
type Step = Box<dyn FnMut(f32, f32, &[f64]) -> Vec<f64>>;
/// A moving filter at rest at a cutoff and resonance, with a control step
/// of so many host frames: its samples a frame and its step.
type Moving = Box<dyn Fn(f32, f32, usize) -> (usize, Step)>;

/// A filter type and how to set it up.
struct Subject {
    name: String,
    /// The filter at a cutoff (Hz) and resonance (0 to 1), at rest.
    make: Make,
    /// The drawn curve's gain (linear) at a cutoff, resonance and Hz.
    drawn: Drawn,
    /// It is meant to saturate (Y1b).
    saturates: bool,
    /// Its cutoff names a resonant peak (Y2a).
    peak: bool,
    /// It is meant to sing at full resonance (Y2b).
    sings: bool,
    /// The filter as its engine moves it (Y1c), and the engine's control
    /// step (frames).
    moving: Moving,
    ctrl: usize,
}

/// Every filter, at `rate`.
fn subjects(rate: u32) -> Vec<Subject> {
    let mut v = Vec::new();
    for (mode, index, label) in [
        (FilterMode::Lp24, 0.0f32, "LP 24"),
        (FilterMode::Lp12, 1.0, "LP 12"),
        (FilterMode::Bp12, 2.0, "BP 12"),
        (FilterMode::Hp12, 3.0, "HP 12"),
    ] {
        v.push(Subject {
            name: format!("Poly {label}"),
            make: Box::new(move |fc, r| {
                let c = FilterCoefs::new(fc, r, rate as f32);
                let mut f = SynthFilter::default();
                Set {
                    os: 1,
                    run: Box::new(move |x| {
                        x.iter()
                            .map(|&s| f64::from(f.process(mode, &c, s as f32)))
                            .collect()
                    }),
                }
            }),
            drawn: Box::new(move |fc, r, hz| {
                f64::from(mpfree_synth_ui::visual::filter_gain(
                    index, fc, r, hz as f32,
                ))
            }),
            saturates: false,
            peak: true,
            sings: false,
            // The voice works out its coefficients once a control step.
            moving: Box::new(move |_, _, _| {
                let mut f = SynthFilter::default();
                let step: Step = Box::new(move |fc, r, x| {
                    let c = FilterCoefs::new(fc, r, rate as f32);
                    let mut buf: Vec<f32> = x.iter().map(|&s| s as f32).collect();
                    f.run(mode, &c, &mut buf);
                    buf.into_iter().map(f64::from).collect()
                });
                (1, step)
            }),
            ctrl: poly::CTRL as usize,
        });
    }
    for (index, label) in [
        (0.0f32, "LP 24"),
        (1.0, "LP 12"),
        (2.0, "BP 12"),
        (3.0, "HP 12"),
    ] {
        v.push(Subject {
            name: format!("Analog {label}"),
            make: Box::new(move |fc, r| {
                let mut p = analog::FilterProbe::new(rate);
                p.set(index, fc, 100.0 * r, 0.0);
                Set {
                    os: 1,
                    run: Box::new(move |x| {
                        x.iter().map(|&s| f64::from(p.tick(s as f32))).collect()
                    }),
                }
            }),
            drawn: Box::new(move |fc, r, hz| {
                f64::from(analog::filter_gain(index, fc, 100.0 * r, hz as f32))
            }),
            saturates: true,
            peak: true,
            sings: index == 0.0,
            moving: Box::new(move |fc, r, _| {
                let mut p = analog::FilterProbe::new(rate);
                p.set(index, fc, 100.0 * r, 0.0);
                let step: Step = Box::new(move |fc, r, x| {
                    p.glide(fc, 100.0 * r, x.len() as u32);
                    x.iter().map(|&s| f64::from(p.tick(s as f32))).collect()
                });
                (1, step)
            }),
            ctrl: analog::CTRL as usize,
        });
    }
    v.push(Subject {
        name: "Squelch".into(),
        make: Box::new(move |fc, r| {
            let mut p = squelch::FilterProbe::new(rate);
            p.set(fc, 100.0 * r);
            let os = p.oversampling();
            Set {
                os,
                run: Box::new(move |x| frames(x, os, |f| p.frame(f))),
            }
        }),
        drawn: Box::new(|fc, r, hz| f64::from(squelch::response(fc, 100.0 * r, hz as f32))),
        saturates: true,
        peak: true,
        sings: false,
        moving: Box::new(move |fc, r, _| {
            let mut p = squelch::FilterProbe::new(rate);
            p.set(fc, 100.0 * r);
            let os = p.oversampling();
            let step: Step = Box::new(move |fc, r, x| {
                p.glide(fc, 100.0 * r, (x.len() / os) as u32);
                frames(x, os, |f| p.frame(f))
            });
            (os, step)
        }),
        ctrl: squelch::CTRL as usize,
    });
    v.push(Subject {
        name: "Bass".into(),
        make: Box::new(move |fc, r| {
            let mut p = bass::FilterProbe::new(rate);
            p.set(fc, 100.0 * r, 0.0);
            let os = p.oversampling();
            Set {
                os,
                run: Box::new(move |x| frames(x, os, |f| p.frame(f))),
            }
        }),
        drawn: Box::new(|fc, r, hz| f64::from(bass::ladder_response(fc, 100.0 * r, hz as f32))),
        saturates: true,
        peak: true,
        sings: true,
        moving: Box::new(move |fc, r, _| {
            let mut p = bass::FilterProbe::new(rate);
            p.set(fc, 100.0 * r, 0.0);
            let os = p.oversampling();
            let step: Step = Box::new(move |fc, r, x| {
                p.glide(fc, 100.0 * r, 0.0, (x.len() / os) as u32);
                frames(x, os, |f| p.frame(f))
            });
            (os, step)
        }),
        ctrl: bass::CTRL as usize,
    });
    let (delay, gain) = pad_path(rate);
    for (label, kind, poles) in [
        ("low-pass 2-pole", FilterKind::LowPass, 2u8),
        ("low-pass 4-pole", FilterKind::LowPass, 4),
        ("high-pass 2-pole", FilterKind::HighPass, 2),
        ("high-pass 4-pole", FilterKind::HighPass, 4),
        ("band-pass 2-pole", FilterKind::BandPass, 2),
        ("band-pass 4-pole", FilterKind::BandPass, 4),
        ("notch 2-pole", FilterKind::Notch, 2),
        ("notch 4-pole", FilterKind::Notch, 4),
        ("Ladder", FilterKind::Ladder, 4),
        ("Vowel", FilterKind::Vowel, 2),
        ("Comb", FilterKind::Comb, 2),
    ] {
        let filter = move |fc: f32, r: f32| PadFilter {
            kind,
            poles,
            cutoff_hz: fc,
            resonance: r,
            env_octaves: 0.0,
            drive_db: 0.0,
        };
        v.push(Subject {
            name: format!("sampler {label}"),
            make: Box::new(move |fc, r| {
                let f = filter(fc, r);
                Set {
                    os: 1,
                    run: Box::new(move |x| {
                        // The impulse or tone after a lead, so the pad's
                        // envelope is open; the pad's own gain and delay
                        // taken out.
                        let mut s = vec![0.0; PAD_LEAD];
                        s.extend_from_slice(x);
                        let y = sampler::through_pad(&s, Some(f), rate, s.len() + delay);
                        y[PAD_LEAD + delay..].iter().map(|v| v / gain).collect()
                    }),
                }
            }),
            drawn: Box::new(move |fc, r, hz| {
                let db = mpfree_sampler_ui::play_widgets::filter_db(&filter(fc, r), hz as f32);
                10f64.powf(f64::from(db) / 20.0)
            }),
            saturates: kind == FilterKind::Ladder,
            peak: matches!(
                kind,
                FilterKind::LowPass
                    | FilterKind::HighPass
                    | FilterKind::BandPass
                    | FilterKind::Ladder
            ),
            sings: kind == FilterKind::Ladder,
            moving: Box::new(move |fc, r, ctrl| {
                let mut m = PadMirror::new(filter(fc, r), rate, ctrl as u32);
                m.set(fc, r);
                let step: Step = Box::new(move |fc, r, x| {
                    m.set(fc, r);
                    x.iter().map(|&s| f64::from(m.process(s as f32))).collect()
                });
                (1, step)
            }),
            ctrl: PAD_STEP,
        });
    }
    v
}

/// The sampler's filter as a voice runs it (`mpfree_sampler::engine`'s
/// `retune` and `finish`, the left channel), so that it can be set every
/// frame as well as every [`PAD_STEP`] frames. Each set first flushes the
/// states too small to hear, as the voice's does.
struct PadMirror {
    f: PadFilter,
    rate: f32,
    /// Frames the comb's pitch glides over (the voice's step).
    glide: u32,
    svf: [Svf; 2],
    ladder: PadLadder,
    comb: Comb,
    vowel_gain: f32,
    /// The cutoff and resonance last set (the voice skips a set that
    /// changes nothing).
    last: Option<(u32, u32)>,
}

impl PadMirror {
    fn new(f: PadFilter, rate: u32, glide: u32) -> PadMirror {
        PadMirror {
            f,
            rate: rate as f32,
            glide,
            svf: [Svf::default(); 2],
            ladder: PadLadder::default(),
            comb: Comb::new(rate as f32),
            vowel_gain: 1.0,
            last: None,
        }
    }

    fn set(&mut self, fc: f32, resonance: f32) {
        let four = self.f.poles >= 4;
        match self.f.kind {
            FilterKind::Ladder => self.ladder.flush(),
            FilterKind::Comb => {}
            FilterKind::Vowel => self.svf.iter_mut().for_each(Svf::flush),
            _ => {
                self.svf[0].flush();
                if four {
                    self.svf[1].flush();
                }
            }
        }
        let key = (fc.to_bits(), resonance.to_bits());
        if self.last == Some(key) {
            return;
        }
        self.last = Some(key);
        let rate = self.rate;
        match self.f.kind {
            FilterKind::Ladder => self.ladder.set(fc, resonance, self.f.drive_db, rate),
            FilterKind::Comb => self.comb.set(fc, resonance, rate, self.glide),
            FilterKind::Vowel => {
                let (f1, f2) = vowel_formants(fc);
                let res = vowel_resonance(resonance);
                self.svf[0].set(f1, res, rate);
                self.svf[1].set(f2, res, rate);
                self.vowel_gain = vowel_gain(resonance);
            }
            _ => {
                self.svf[0].set(fc, resonance, rate);
                if four {
                    self.svf[1].set(fc, resonance * 0.5, rate);
                }
            }
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let kind = self.f.kind;
        match kind {
            FilterKind::Ladder => self.ladder.process(0, x),
            FilterKind::Comb => self.comb.process(x, 0.0).0,
            FilterKind::Vowel => {
                let [a, b] = &mut self.svf;
                self.vowel_gain * (a.process(0, x, kind) + FORMANT_2 * b.process(0, x, kind))
            }
            _ => {
                let y = self.svf[0].process(0, x, kind);
                if self.f.poles >= 4 {
                    self.svf[1].process(0, y, kind)
                } else {
                    y
                }
            }
        }
    }
}

/// `x` a frame of `os` samples at a time through `frame`.
fn frames(x: &[f64], os: usize, mut frame: impl FnMut(&[f32]) -> f32) -> Vec<f64> {
    let mut buf = vec![0.0f32; os];
    x.chunks(os)
        .map(|c| {
            for (b, &s) in buf.iter_mut().zip(c) {
                *b = s as f32;
            }
            f64::from(frame(&buf))
        })
        .collect()
}

/// The pad with its filter off: the frames from a sound's start to where
/// it comes out, and its gain (the read at pitch is a plain copy).
fn pad_path(rate: u32) -> (usize, f64) {
    let mut x = vec![0.0; 2 * PAD_LEAD];
    x[PAD_LEAD] = SMALL;
    let y = sampler::through_pad(&x, None, rate, x.len());
    let at = (0..y.len())
        .max_by(|&a, &b| y[a].abs().total_cmp(&y[b].abs()))
        .unwrap_or(PAD_LEAD);
    (at - PAD_LEAD, y[at] / SMALL)
}

/// The response to a -40 dBFS impulse, `n` frames at the host rate (a
/// power of two), as gain per FFT bin: (re, im).
fn spectrum(s: &mut Set, n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut x = vec![0.0; n * s.os];
    x[0] = SMALL;
    let y = (s.run)(&x);
    // Down from the inner rate, the band below half the host rate comes
    // out at 1/os of its gain.
    let scale = s.os as f64 / SMALL;
    let mut re: Vec<f64> = y.iter().take(n).map(|v| v * scale).collect();
    re.resize(n, 0.0);
    let mut im = vec![0.0; n];
    Fft::new(n).forward(&mut re, &mut im);
    (re, im)
}

fn db20(a: f64) -> f64 {
    20.0 * a.max(1e-30).log10()
}

/// The log grid Y1a reads: 48 points an octave from 20 Hz to 20 kHz, as
/// FFT bins of `n` at `rate` (each once).
fn grid(n: usize, rate: u32) -> Vec<usize> {
    let mut bins: Vec<usize> = (0..)
        .map(|i| 20.0 * 2f64.powf(f64::from(i) / 48.0))
        .take_while(|&f| f <= 20_000.0)
        .map(|f| (f * n as f64 / f64::from(rate)).round() as usize)
        .collect();
    bins.dedup();
    bins
}

/// The gain of `x` at `f` Hz (a windowed single-bin transform).
fn amp_at(x: &[f64], f: f64, rate: u32) -> f64 {
    let w = blackman_harris(x.len());
    let (mut re, mut im) = (0.0, 0.0);
    for (i, (&s, &wi)) in x.iter().zip(&w).enumerate() {
        let ph = TAU * f * i as f64 / f64::from(rate);
        re += s * wi * ph.cos();
        im -= s * wi * ph.sin();
    }
    2.0 * re.hypot(im) / w.iter().sum::<f64>()
}

/// The gain at `f` Hz for a sine of peak `amp`: 0.1 s to settle, 0.2 s
/// read.
fn tone_gain(s: &mut Set, f: f64, amp: f64, rate: u32) -> f64 {
    let host = (0.3 * f64::from(rate)) as usize;
    let fs = f64::from(rate) * s.os as f64;
    let x: Vec<f64> = (0..host * s.os)
        .map(|i| amp * (TAU * f * i as f64 / fs).sin())
        .collect();
    let y = (s.run)(&x);
    let from = host / 3;
    amp_at(&y[from..host.min(y.len())], f, rate) / amp
}

/// The drawn curve's peak between a quarter and four times `fc` (below
/// half of `rate`), if it has one inside.
fn drawn_peak(drawn: &dyn Fn(f64) -> f64, fc: f64, rate: u32) -> Option<f64> {
    let (lo, hi) = (fc / 4.0, (fc * 4.0).min(0.49 * f64::from(rate)));
    let steps = (1200.0 * (hi / lo).log2()).ceil() as usize;
    let at = |i: usize| lo * 2f64.powf(i as f64 / 1200.0);
    let best = (0..=steps).max_by(|&a, &b| drawn(at(a)).total_cmp(&drawn(at(b))))?;
    if best == 0 || best == steps {
        return None;
    }
    let (a, b, c) = (
        db20(drawn(at(best - 1))),
        db20(drawn(at(best))),
        db20(drawn(at(best + 1))),
    );
    Some(at(best) * 2f64.powf(parabola(a, b, c) / 1200.0))
}

/// Where a parabola through three equally spaced points peaks, in steps
/// from the middle one.
fn parabola(a: f64, b: f64, c: f64) -> f64 {
    let d = a - 2.0 * b + c;
    if d.abs() > 1e-12 {
        (0.5 * (a - c) / d).clamp(-0.5, 0.5)
    } else {
        0.0
    }
}

/// The loudest bin of `db` (per bin of `n` at `rate`) between `lo` and
/// `hi` Hz, refined between bins.
fn peak_hz(db: &[f64], n: usize, rate: u32, lo: f64, hi: f64) -> f64 {
    let hz = f64::from(rate) / n as f64;
    let (a, b) = (
        (lo / hz).floor().max(1.0) as usize,
        ((hi / hz).ceil() as usize).min(db.len() - 2),
    );
    let k = (a..=b)
        .max_by(|&x, &y| db[x].total_cmp(&db[y]))
        .unwrap_or(a);
    (k as f64 + parabola(db[k - 1], db[k], db[k + 1])) * hz
}

fn cents(f: f64, to: f64) -> f64 {
    1200.0 * (f / to).log2()
}

#[allow(clippy::too_many_arguments)]
fn put(
    rows: &mut Vec<Row>,
    id: &'static str,
    subject: String,
    metric: &str,
    value: f64,
    better: Better,
    target: Option<f64>,
    tol: f64,
) {
    rows.push(Row {
        target_id: id,
        subject: format!("filters/{subject}"),
        metric: metric.to_string(),
        value,
        better,
        target,
        tol,
    });
}

fn khz(fc: f32) -> String {
    let k = f64::from(fc) / 1000.0;
    if k >= 1.0 {
        format!("{k} kHz")
    } else {
        format!("{fc} Hz")
    }
}

/// Every filter measure at `rate`.
pub fn report(rate: u32) -> Vec<Row> {
    let mut rows = Vec::new();
    for s in subjects(rate) {
        small_signal(&mut rows, &s, rate);
        if s.saturates {
            driven(&mut rows, &s, rate);
        }
        if s.peak {
            tracking(&mut rows, &s, rate);
        }
        if s.sings {
            singing(&mut rows, &s, rate);
        }
        bounded(&mut rows, &s, rate);
        stepping(&mut rows, &s, rate);
    }
    rows
}

/// Y1a.
fn small_signal(rows: &mut Vec<Row>, s: &Subject, rate: u32) {
    let hz = f64::from(rate) / IR_FRAMES as f64;
    let bins = grid(IR_FRAMES, rate);
    for fc in CUTOFFS {
        for r in RESONANCES {
            let (re, im) = spectrum(&mut (s.make)(fc, r), IR_FRAMES);
            let (mut below, mut top) = (0.0f64, None::<f64>);
            for &k in &bins {
                let f = k as f64 * hz;
                let want = db20((s.drawn)(fc, r, f));
                let err = (db20(re[k].hypot(im[k])) - want).abs();
                if f <= f64::from(fc) && want >= -24.0 {
                    below = below.max(err);
                }
                if (18_000.0..=20_000.0).contains(&f) && want >= -30.0 {
                    top = Some(top.unwrap_or(0.0).max(err));
                }
            }
            let subject = format!("{}[{}, resonance {r}]", s.name, khz(fc));
            put(
                rows,
                "Y1a",
                subject.clone(),
                "below_cutoff_err_db",
                below,
                Better::Lower,
                Some(1.0),
                0.05,
            );
            if let Some(t) = top {
                put(
                    rows,
                    "Y1a",
                    subject,
                    "err_18_20k_db",
                    t,
                    Better::Lower,
                    None,
                    0.05,
                );
            }
        }
    }
}

/// Y1b.
fn driven(rows: &mut Vec<Row>, s: &Subject, rate: u32) {
    for fc in CUTOFFS {
        for r in RESONANCES {
            let freqs = [0.25, 0.5, 1.0].map(|m| m * f64::from(fc));
            let quiet: Vec<f64> = freqs
                .iter()
                .map(|&f| tone_gain(&mut (s.make)(fc, r), f, SMALL, rate))
                .collect();
            for level in DRIVEN_DB {
                let amp = 10f64.powf(level / 20.0);
                let change = freqs
                    .iter()
                    .zip(&quiet)
                    .map(|(&f, &q)| db20(tone_gain(&mut (s.make)(fc, r), f, amp, rate)) - db20(q))
                    .fold(0.0f64, |a, c| if c.abs() > a.abs() { c } else { a });
                put(
                    rows,
                    "Y1b",
                    format!("{}[{}, resonance {r}]", s.name, khz(fc)),
                    &format!("gain_change_at_{}dbfs_db", level as i32),
                    change,
                    Better::Info,
                    None,
                    0.0,
                );
            }
        }
    }
}

/// Y2a.
fn tracking(rows: &mut Vec<Row>, s: &Subject, rate: u32) {
    let mut worst = None::<f64>;
    for fc in PEAK_CUTOFFS {
        let drawn = |f: f64| (s.drawn)(fc, PEAK_RESONANCE, f);
        let Some(want) = drawn_peak(&drawn, f64::from(fc), rate) else {
            continue;
        };
        let (re, im) = spectrum(&mut (s.make)(fc, PEAK_RESONANCE), PEAK_FRAMES);
        let db: Vec<f64> = re
            .iter()
            .zip(&im)
            .take(PEAK_FRAMES / 2)
            .map(|(a, b)| db20(a.hypot(*b)))
            .collect();
        let got = peak_hz(&db, PEAK_FRAMES, rate, want / 2.0, want * 2.0);
        let c = cents(got, want);
        put(
            rows,
            "Y2a",
            format!("{}[{}, resonance {PEAK_RESONANCE}]", s.name, khz(fc)),
            "peak_cents",
            c,
            Better::Info,
            None,
            0.0,
        );
        if worst.is_none_or(|w| c.abs() > w.abs()) {
            worst = Some(c);
        }
    }
    if let Some(w) = worst {
        put(
            rows,
            "Y2a",
            format!("{}[100 Hz to 10 kHz, resonance {PEAK_RESONANCE}]", s.name),
            "peak_worst_cents",
            w,
            Better::Near(0.0),
            Some(25.0),
            0.5,
        );
    }
}

/// Y2b.
fn singing(rows: &mut Vec<Row>, s: &Subject, rate: u32) {
    const N: usize = 1 << 15;
    let second = rate as usize;
    let (mut worst_c, mut worst_g) = (None::<f64>, None::<f64>);
    for fc in SING_CUTOFFS {
        // Four equal stages turn half a cycle at the cutoff, whatever the
        // feedback, so that is where a ladder sings (the drawn curve's
        // peak sits a little above it once the feedback passes 4).
        let want = f64::from(fc);
        let mut set = (s.make)(fc, 1.0);
        let mut x = vec![0.0; SING_SECS * second * set.os];
        x[0] = SMALL;
        let y = (set.run)(&x);
        let rms = |i: usize| {
            let w = &y[i * second..((i + 1) * second).min(y.len())];
            (w.iter().map(|v| v * v).sum::<f64>() / w.len().max(1) as f64).sqrt()
        };
        let last = rms(SING_SECS - 1);
        let growth = (db20(last) - db20(rms(SING_SECS / 2 - 1))).max(-140.0);
        let c = if last > 1e-6 {
            let tail = &y[y.len() - N..];
            let w = blackman_harris(N);
            let mut re: Vec<f64> = tail.iter().zip(&w).map(|(a, b)| a * b).collect();
            let mut im = vec![0.0; N];
            Fft::new(N).forward(&mut re, &mut im);
            let db: Vec<f64> = re
                .iter()
                .zip(&im)
                .take(N / 2)
                .map(|(a, b)| db20(a.hypot(*b)))
                .collect();
            cents(peak_hz(&db, N, rate, want / 2.0, want * 2.0), want)
        } else {
            // Silent: no pitch to read.
            1200.0
        };
        let subject = format!("{}[{}, resonance 1]", s.name, khz(fc));
        put(
            rows,
            "Y2b",
            subject.clone(),
            "sing_cents",
            c,
            Better::Info,
            None,
            0.0,
        );
        put(
            rows,
            "Y2b",
            subject.clone(),
            "sing_growth_db",
            growth,
            Better::Info,
            None,
            0.0,
        );
        put(
            rows,
            "Y2b",
            subject,
            "sing_level_db",
            db20(last * 2f64.sqrt()),
            Better::Info,
            None,
            0.0,
        );
        if worst_c.is_none_or(|w| c.abs() > w.abs()) {
            worst_c = Some(c);
        }
        if worst_g.is_none_or(|w| growth.abs() > w.abs()) {
            worst_g = Some(growth);
        }
    }
    let subject = format!("{}[110 Hz to 7 kHz, resonance 1]", s.name);
    if let Some(c) = worst_c {
        put(
            rows,
            "Y2b",
            subject.clone(),
            "sing_worst_cents",
            c,
            Better::Near(0.0),
            Some(10.0),
            0.5,
        );
    }
    if let Some(g) = worst_g {
        put(
            rows,
            "Y2b",
            subject,
            "sing_growth_worst_db",
            g,
            Better::Near(0.0),
            Some(1.0),
            0.1,
        );
    }
}

/// One of Y1c's moves: a cycle of `period` host frames (a power of two
/// and a whole number of control steps, so a stepped filter steps the
/// same way every cycle), and the cutoff (Hz) and resonance at a phase of
/// it (0 to 1) for a resonance setting.
struct Move {
    name: &'static str,
    period: usize,
    at: fn(f64, f32) -> (f32, f32),
    /// It moves the resonance itself (measured once, not at each of
    /// [`BOUND_RESONANCES`]).
    own_resonance: bool,
    /// A move an engine makes at its control rate, measured for stepping
    /// noise too.
    stepped: bool,
}

/// A fast filter envelope taking the cutoff between 100 Hz and 12.8 kHz:
/// up in a straight line over the first 32nd of the cycle (1.5 ms at
/// 44.1 kHz), then down by e^-9 over the rest (a 5 ms time constant).
fn envelope_at(p: f64, r: f32) -> (f32, f32) {
    const ATTACK: f64 = 1.0 / 32.0;
    let e = if p < ATTACK {
        p / ATTACK
    } else {
        (-9.0 * (p - ATTACK) / (1.0 - ATTACK)).exp()
    };
    ((100.0 * (7.0 * e).exp2()) as f32, r)
}

/// An LFO: two octaves either side of 1 kHz.
fn lfo_at(p: f64, r: f32) -> (f32, f32) {
    ((1_000.0 * (2.0 * (TAU * p).sin()).exp2()) as f32, r)
}

/// The cutoff at audio rate: three octaves either side of 1 kHz.
fn audio_cutoff_at(p: f64, r: f32) -> (f32, f32) {
    ((1_000.0 * (3.0 * (TAU * p).sin()).exp2()) as f32, r)
}

/// The resonance at audio rate, 0 to 1, at 1 kHz.
fn audio_resonance_at(p: f64, _: f32) -> (f32, f32) {
    (1_000.0, (0.5 - 0.5 * (TAU * p).cos()) as f32)
}

const MOVES: [Move; 4] = [
    Move {
        name: "envelope",
        period: 2_048,
        at: envelope_at,
        own_resonance: false,
        stepped: true,
    },
    Move {
        name: "LFO",
        period: 8_192,
        at: lfo_at,
        own_resonance: false,
        stepped: true,
    },
    Move {
        name: "audio-rate cutoff",
        period: 64,
        at: audio_cutoff_at,
        own_resonance: false,
        stepped: false,
    },
    Move {
        name: "audio-rate resonance",
        period: 64,
        at: audio_resonance_at,
        own_resonance: true,
        stepped: false,
    },
];

/// The filter's samples a frame.
fn os_of(s: &Subject) -> usize {
    (s.moving)(1_000.0, 0.0, 1).0
}

/// `x` (at the filter's own rate) through `s`'s filter moved with a
/// control step of `ctrl` frames, each step toward `at(frame)` (cutoff,
/// resonance) for the frame it starts at, as the engines work out their
/// targets.
fn run_moving(s: &Subject, ctrl: usize, x: &[f64], at: impl Fn(usize) -> (f32, f32)) -> Vec<f64> {
    let (c0, r0) = at(0);
    let (os, mut step) = (s.moving)(c0, r0, ctrl);
    let frames = x.len() / os;
    let mut y = Vec::with_capacity(frames);
    let mut n = 0;
    while n < frames {
        let len = ctrl.min(frames - n);
        let (c, r) = at(n);
        y.extend(step(c, r, &x[n * os..(n + len) * os]));
        n += len;
    }
    y
}

/// The phase of host frame `n` in a move's cycle.
fn phase(m: &Move, n: usize) -> f64 {
    (n % m.period) as f64 / m.period as f64
}

fn peak(y: &[f64]) -> Option<f64> {
    y.iter()
        .try_fold(0.0f64, |p, &v| v.is_finite().then(|| p.max(v.abs())))
}

/// Y1c, staying bounded.
fn bounded(rows: &mut Vec<Row>, s: &Subject, rate: u32) {
    let frames = rate as usize / 2;
    let os = os_of(s);
    let noise = white(frames * os, BOUND_NOISE, 0x59_1c);
    for m in &MOVES {
        let resonances: &[f32] = if m.own_resonance {
            &[0.0]
        } else {
            &BOUND_RESONANCES
        };
        let mut worst = None::<f64>;
        for &r in resonances {
            let steps: &[usize] = if m.stepped { &[1, s.ctrl] } else { &[1] };
            let moved = steps
                .iter()
                .map(|&ctrl| peak(&run_moving(s, ctrl, &noise, |n| (m.at)(phase(m, n), r))))
                .try_fold(0.0f64, |a, p| p.map(|p| a.max(p)));
            // The same noise with the settings held at 16 places on the
            // move, half as long each.
            let held = (0..16)
                .map(|i| {
                    let set = (m.at)(i as f64 / 16.0, r);
                    peak(&run_moving(s, s.ctrl, &noise[..noise.len() / 2], |_| set))
                })
                .try_fold(0.0f64, |a, p| p.map(|p| a.max(p)));
            let over = match (moved, held) {
                (Some(a), Some(b)) => db20(a) - db20(b),
                // Not finite.
                _ => 999.0,
            };
            worst = Some(worst.map_or(over, |w: f64| w.max(over)));
        }
        if let Some(w) = worst {
            put(
                rows,
                "Y1c",
                format!("{}[{}]", s.name, m.name),
                "over_held_peak_db",
                w,
                Better::Lower,
                Some(6.0),
                0.1,
            );
        }
    }
}

/// Octave band powers (centres 62.5 Hz to 16 kHz) of `x`, which holds a
/// whole number of cycles of everything in it (no window).
fn octave_bands(x: &[f64], rate: u32) -> [f64; 9] {
    let ps = power_spectrum(x);
    let bin = f64::from(rate) / x.len() as f64;
    let mut bands = [0.0; 9];
    for (k, &p) in ps.iter().enumerate().skip(1) {
        let b = ((k as f64 * bin / 62.5).log2() + 0.5).floor();
        if (0.0..9.0).contains(&b) {
            bands[b as usize] += p;
        }
    }
    bands
}

/// Y1c, stepping noise.
fn stepping(rows: &mut Vec<Row>, s: &Subject, rate: u32) {
    let os = os_of(s);
    let frames = STEP_SETTLE + STEP_FRAMES;
    // A tone near 1 kHz that fits the window exactly.
    let k = (1_000.0 * STEP_FRAMES as f64 / f64::from(rate)).round();
    let fs = f64::from(rate) * os as f64;
    let f0 = k * f64::from(rate) / STEP_FRAMES as f64;
    let tone: Vec<f64> = (0..frames * os)
        .map(|i| STEP_TONE * (TAU * f0 * i as f64 / fs).sin())
        .collect();
    for m in MOVES.iter().filter(|m| m.stepped) {
        let mut worst = 0.0f64;
        for r in STEP_RESONANCES {
            let at = |n| (m.at)(phase(m, n), r);
            let stepped = run_moving(s, s.ctrl, &tone, at);
            let smooth = run_moving(s, 1, &tone, at);
            let a = octave_bands(&stepped[STEP_SETTLE..], rate);
            let b = octave_bands(&smooth[STEP_SETTLE..], rate);
            // Each band against the smooth move's, where the stepped one
            // is within 60 dB of the smooth one's loudest band; a smooth
            // band quieter than that counts as that.
            let floor = b.iter().cloned().fold(0.0, f64::max) * 1e-6;
            let over = a
                .iter()
                .zip(&b)
                .filter(|(&p, _)| p > floor || !p.is_finite())
                .map(|(&p, &q)| {
                    if p.is_finite() {
                        10.0 * (p / q.max(floor)).log10()
                    } else {
                        999.0
                    }
                })
                .fold(0.0, f64::max);
            worst = worst.max(over);
        }
        put(
            rows,
            "Y1c",
            format!("{}[{}]", s.name, m.name),
            "step_noise_db",
            worst,
            Better::Lower,
            Some(3.0),
            0.1,
        );
    }
}

/// The ratchet file's header.
const HEADER: &str =
    "# The filter measures' ratchet (DSP quality Y1a, Y1c, Y2a, Y2b): every number may stay or get better, never worse.\n\
# Written by `MPFREE_QUALITY_BLESS=1 cargo test --release --locked -p mpfree-fxlab --test filters`;\n\
# see docs/dev/design/dsp-quality.md. key<TAB>value\n";

/// The ratchet file for `rows`.
pub fn ratchet_text(rows: &[Row]) -> String {
    let body = sampler::ratchet_text(rows);
    // The same lines under this file's own header.
    let lines: String = body
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| format!("{l}\n"))
        .collect();
    format!("{HEADER}{lines}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pad_path_is_a_plain_copy() {
        let (delay, gain) = pad_path(44_100);
        assert!(gain > 0.1 && gain <= 1.0, "gain {gain}");
        assert!(delay < 256, "delay {delay}");
    }

    #[test]
    fn the_pad_mirror_moves_as_a_pad_does() {
        use mpfree_sampler::dsp::{Ahdsr, Envelope};
        let rate = 44_100;
        let (delay, gain) = pad_path(rate);
        // A filter envelope sweeping five octaves down in 40 ms over noise.
        let env = Ahdsr {
            attack_ms: 0.0,
            hold_ms: 0.0,
            decay_ms: 40.0,
            sustain: 0.0,
            release_ms: 5.0,
        };
        let x = white(8_192, 0.25, 7);
        for (kind, poles) in [
            (FilterKind::LowPass, 2u8),
            (FilterKind::LowPass, 4),
            (FilterKind::HighPass, 2),
            (FilterKind::HighPass, 4),
            (FilterKind::BandPass, 2),
            (FilterKind::BandPass, 4),
            (FilterKind::Notch, 2),
            (FilterKind::Notch, 4),
            (FilterKind::Ladder, 4),
            (FilterKind::Vowel, 2),
            (FilterKind::Comb, 2),
        ] {
            let f = PadFilter {
                kind,
                poles,
                cutoff_hz: 200.0,
                resonance: 0.7,
                env_octaves: 5.0,
                drive_db: 0.0,
            };
            let pad = sampler::through_shaped_pad(&x, rate, x.len() + delay, |p| {
                p.filter = f;
                p.filter_env = env;
            });
            let mut m = PadMirror::new(f, rate, PAD_STEP as u32);
            let mut e = Envelope::start(&env, rate as f32);
            let mut worst = 0.0f64;
            for (n, &s) in x.iter().enumerate() {
                let level = e.next();
                if n % PAD_STEP == 0 {
                    m.set(f.cutoff_hz * (f.env_octaves * level).exp2(), f.resonance);
                }
                let y = f64::from(m.process((s * gain) as f32));
                worst = worst.max((y - pad[n + delay]).abs());
            }
            // The same to the rounding of f32.
            assert!(worst < 1e-6, "{kind:?} {poles}-pole: {worst}");
        }
    }

    #[test]
    fn a_wire_reads_flat_and_its_peak_where_drawn() {
        // A plain gain: 0 dB at every bin.
        let mut wire = Set {
            os: 1,
            run: Box::new(|x| x.to_vec()),
        };
        let (re, im) = spectrum(&mut wire, 1 << 12);
        assert!(re
            .iter()
            .zip(&im)
            .all(|(a, b)| (a.hypot(*b) - 1.0).abs() < 1e-12));
        // The drawn peak of a resonant two-pole sits where the formula says.
        let d = 0.2f64;
        let lp = |f: f64| {
            let w = f / 1000.0;
            1.0 / ((1.0 - w * w).powi(2) + (d * w).powi(2)).sqrt()
        };
        let want = 1000.0 * (1.0 - d * d / 2.0).sqrt();
        let got = drawn_peak(&lp, 1000.0, 44_100).expect("a peak");
        assert!(cents(got, want).abs() < 0.5, "{got} against {want}");
    }
}
