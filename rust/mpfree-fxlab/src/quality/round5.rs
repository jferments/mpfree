//! The fifth round's classes of the quality report: movement (the shared
//! LFO, heard through the tremolo, then the tremolo, auto-pan, rotary and
//! auto wah, the plan's target ids W; the ring mod and the frequency
//! shifter, Z), pitch (the pitch shifter, pitch correct, the harmonizer and
//! the tuner, H) and voice (the talk box, V). The targets and why each was
//! chosen are in `docs/dev/design/fx-quality.md`. The rows, the report and
//! the ratchet are the first round's (`super`); the click measure is round
//! 3's (`modulation.rs`), the switch click round 2's (`round2.rs`), and the
//! measures only these classes need live here.

use mpfree_fx::ids::{
    autopan as ap, autowah as aw, freqshift as fs, harmonizer as hm, pitch as ps,
    pitch_correct as pc, ringmod as rm, rotary as ro, talkbox as tb, tremolo as tr, tuner as tu,
    vocoder as vc,
};
use mpfree_fx::mpfree_node::{Ctx, Curve, Node};

use super::modulation::hf_click;
use super::round2::{db_amp, db_pow, engine_subjects, make_with, switch_click};
use super::round4::{node_with, run as run_keyed, steady};
use super::{floor, lu_change, make, pink_stim, slug, Better, Report, Subject};
use crate::dut::{Change, Dut, NodeDut};
use crate::fft::{hann_f64, xcorr, Fft};
use crate::signals::{db_to_amp, impulse, pink, rms_of, sine, white};
use crate::wav::Stereo;
use crate::{flutter, loudness, mixlaw};

const TAU: f64 = std::f64::consts::TAU;

const TREMOLO: &str = "mpfree.fx.tremolo";
const AUTOPAN: &str = "mpfree.fx.autopan";
const ROTARY: &str = "mpfree.fx.rotary";
const RINGMOD: &str = "mpfree.fx.ringmod";
const FREQSHIFT: &str = "mpfree.fx.freqshift";
const AUTOWAH: &str = "mpfree.fx.autowah";
const PITCH: &str = "mpfree.fx.pitch";
const PITCH_CORRECT: &str = "mpfree.fx.pitch_correct";
const HARMONIZER: &str = "mpfree.fx.harmonizer";
const TUNER: &str = "mpfree.fx.tuner";
const TALKBOX: &str = "mpfree.fx.talkbox";
const VOCODER: &str = "mpfree.fx.vocoder";

/// The LFO's shapes, in the order of its Shape control.
const LFO_SHAPES: [&str; 6] = [
    "sine",
    "triangle",
    "square",
    "ramp-up",
    "ramp-down",
    "random",
];

/// The sync choice that is one bar.
const ONE_BAR: f64 = 11.0;

// ---------------------------------------------------------------------------
// Shared measures.

/// A subject that is a setting of an engine rather than a preset.
fn named(id: &'static str, name: &str) -> Subject {
    Subject {
        name: name.to_string(),
        id,
        fixed: &[],
        preset: None,
    }
}

fn mono(x: Vec<f64>, rate: u32) -> Stereo {
    Stereo::mono(x, rate)
}

fn cents(hz: f64, of: f64) -> f64 {
    1200.0 * (hz.max(1e-9) / of).log2()
}

fn worst(v: &[f64]) -> Option<f64> {
    v.iter().copied().reduce(f64::max)
}

fn min_max(v: &[f64]) -> (f64, f64) {
    v.iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| {
            (a.min(*x), b.max(*x))
        })
}

/// A band-limited sawtooth at `hz` (harmonics up to 20 kHz, or 0.45 of the
/// rate), `amp` times 0.55 times the sum of sin(k θ)/k, with a vibrato of
/// `vib_c` cents at `vib_hz` (0 for none). The harmonics come from the
/// Chebyshev recurrence, so long saws are cheap.
fn saw(hz: f64, amp: f64, n: usize, rate: u32, vib_c: f64, vib_hz: f64) -> Vec<f64> {
    let top = 20_000f64.min(0.45 * rate as f64);
    let k_max = ((top / (hz * 2f64.powf(vib_c.abs() / 1200.0))) as usize).max(1);
    let mut ph = 0.0f64;
    (0..n)
        .map(|i| {
            let t = i as f64 / rate as f64;
            let f = hz * 2f64.powf(vib_c * (TAU * vib_hz * t).sin() / 1200.0);
            ph = (ph + f / rate as f64).fract();
            let th = TAU * ph;
            let (s1, c) = (th.sin(), th.cos());
            let (mut a, mut b) = (0.0, s1);
            let mut sum = s1;
            for k in 2..=k_max {
                let s = 2.0 * c * b - a;
                sum += s / k as f64;
                a = b;
                b = s;
            }
            amp * 0.55 * sum
        })
        .collect()
}

/// A Hann-windowed power spectrum of the largest power-of-two prefix of
/// `x`, and its bin width in Hz.
fn spec(x: &[f64], rate: u32) -> (Vec<f64>, f64) {
    let mut n = 1usize;
    while n * 2 <= x.len() {
        n *= 2;
    }
    let w = hann_f64(n);
    let mut re: Vec<f64> = x[..n].iter().zip(&w).map(|(a, b)| a * b).collect();
    let mut im = vec![0.0; n];
    Fft::new(n).forward(&mut re, &mut im);
    let s = 1.0 / (n as f64 * n as f64);
    (
        (0..=n / 2)
            .map(|k| (re[k] * re[k] + im[k] * im[k]) * s)
            .collect(),
        rate as f64 / n as f64,
    )
}

/// The power of `d`'s impulse response at each bin of an `n`-point
/// transform (`|H|²`, unscaled), and the bin width in Hz.
fn response(d: &mut NodeDut, n: usize) -> (Vec<f64>, f64) {
    let rate = d.rate();
    let y = d.render(&mono(impulse(n, 1.0), rate), 0).left;
    let mut re = y[..n].to_vec();
    let mut im = vec![0.0; n];
    Fft::new(n).forward(&mut re, &mut im);
    (
        (0..=n / 2).map(|k| re[k] * re[k] + im[k] * im[k]).collect(),
        rate as f64 / n as f64,
    )
}

/// The strongest peak between `lo` and `hi` Hz, interpolated on the log
/// of the power: its frequency, and the power of its three bins.
fn peak_in(p: &[f64], bin: f64, lo: f64, hi: f64) -> (f64, f64) {
    let a = ((lo / bin) as usize).max(1);
    let b = ((hi / bin) as usize).min(p.len() - 2);
    let k = (a..=b).fold(a, |m, i| if p[i] > p[m] { i } else { m });
    let ln = |v: f64| v.max(1e-300).ln();
    let (ym, y0, yp) = (ln(p[k - 1]), ln(p[k]), ln(p[k + 1]));
    let den = ym - 2.0 * y0 + yp;
    let d = if den.abs() > 1e-12 {
        0.5 * (ym - yp) / den
    } else {
        0.0
    };
    ((k as f64 + d) * bin, p[k - 1] + p[k] + p[k + 1])
}

/// The power within `hz` ± `half` Hz.
fn band(p: &[f64], bin: f64, hz: f64, half: f64) -> f64 {
    let a = ((hz - half) / bin).floor().max(0.0) as usize;
    let b = (((hz + half) / bin).ceil() as usize).min(p.len() - 1);
    p[a..=b.max(a)].iter().sum()
}

/// `x` with everything outside `lo` to `hi` Hz removed (a brick wall, for
/// measuring only).
fn bandpass(x: &[f64], rate: u32, lo: f64, hi: f64) -> Vec<f64> {
    let m = x.len().next_power_of_two();
    let f = Fft::new(m);
    let mut re = x.to_vec();
    re.resize(m, 0.0);
    let mut im = vec![0.0; m];
    f.forward(&mut re, &mut im);
    let (a, b) = (
        (lo * m as f64 / rate as f64) as usize,
        (hi * m as f64 / rate as f64) as usize,
    );
    for k in 0..m {
        let kk = if k <= m / 2 { k } else { m - k };
        if kk < a || kk > b {
            re[k] = 0.0;
            im[k] = 0.0;
        }
    }
    f.inverse(&mut re, &mut im);
    re.truncate(x.len());
    re
}

/// The analytic signal of the part of `x` between `lo` and `hi` Hz: its
/// amplitude and its unwrapped phase, frame by frame.
fn analytic(x: &[f64], rate: u32, lo: f64, hi: f64) -> (Vec<f64>, Vec<f64>) {
    let n = x.len().next_power_of_two();
    let fft = Fft::new(n);
    let mut re = x.to_vec();
    re.resize(n, 0.0);
    let mut im = vec![0.0; n];
    fft.forward(&mut re, &mut im);
    let bin = |hz: f64| (hz * n as f64 / rate as f64).round() as usize;
    let (a, b) = (bin(lo).max(1), bin(hi).min(n / 2));
    for k in 0..n {
        let s = if k >= a && k <= b { 2.0 } else { 0.0 };
        re[k] *= s;
        im[k] *= s;
    }
    fft.inverse(&mut re, &mut im);
    let mut amp = Vec::with_capacity(x.len());
    let mut phase = Vec::with_capacity(x.len());
    let mut last = 0.0f64;
    let mut turns = 0.0;
    for i in 0..x.len() {
        amp.push(re[i].hypot(im[i]));
        let p = im[i].atan2(re[i]);
        if i > 0 {
            let d = p - last;
            if d > std::f64::consts::PI {
                turns -= TAU;
            } else if d < -std::f64::consts::PI {
                turns += TAU;
            }
        }
        last = p;
        phase.push(p + turns);
    }
    (amp, phase)
}

/// The level of `x` in windows of whole periods of `hz` (at least 5 ms),
/// dB, so a steady tone reads steady at any pitch.
fn level_db(x: &[f64], rate: u32, hz: f64) -> Vec<f64> {
    let per = rate as f64 / hz;
    let w = ((per * (0.005 * rate as f64 / per).ceil()).round() as usize).max(1);
    x.chunks(w)
        .filter(|c| c.len() == w)
        .map(|c| db_pow(c.iter().map(|v| v * v).sum::<f64>() / w as f64))
        .collect()
}

/// RMS level in `ms` windows, dB.
fn env_db(x: &[f64], ms: f64, rate: u32) -> Vec<f64> {
    let w = ((ms / 1000.0) * rate as f64) as usize;
    x.chunks(w)
        .filter(|c| c.len() == w)
        .map(|c| db_pow(c.iter().map(|v| v * v).sum::<f64>() / w as f64))
        .collect()
}

fn swing(e: &[f64]) -> f64 {
    let (lo, hi) = min_max(e);
    hi - lo
}

/// The mean frequency of `y` from its upward zero crossings, each placed
/// between frames by linear interpolation.
fn crossing_hz(y: &[f64], rate: u32) -> f64 {
    let mut first = None;
    let mut last = 0.0;
    let mut count = 0usize;
    for i in 1..y.len() {
        if y[i - 1] < 0.0 && y[i] >= 0.0 {
            let t = (i - 1) as f64 + y[i - 1] / (y[i - 1] - y[i]);
            if first.is_none() {
                first = Some(t);
            }
            last = t;
            count += 1;
        }
    }
    match first {
        Some(t0) if count > 1 && last > t0 => (count - 1) as f64 * rate as f64 / (last - t0),
        _ => f64::NAN,
    }
}

/// How far `y` lags `x`, frames: where their cross-correlation peaks, from
/// 0 to `max`.
fn lag_of(x: &[f64], y: &[f64], max: usize) -> usize {
    let c = xcorr(y, x);
    let zero = x.len() - 1;
    (0..=max)
        .filter(|l| zero + l < c.len())
        .fold(0, |m, l| if c[zero + l] > c[zero + m] { l } else { m })
}

/// The instantaneous frequency of the tone near `near` in `y`, in cents
/// against `near` (the first and last tenth left out, as
/// `flutter::instantaneous_hz` does).
fn if_cents(y: &[f64], rate: u32, near: f64) -> Vec<f64> {
    flutter::instantaneous_hz(y, rate, near)
        .into_iter()
        .map(|f| cents(f, near))
        .collect()
}

/// The pitch of the tone near `near` in `y`, in cents against `near`, for
/// the vocal effects: as [`if_cents`] (the first and last tenth left out,
/// smoothed over 2.2 ms, so [`if_start`] holds), but the tone is taken
/// through a band with soft edges, flat from 0.85 to 1.15 of `near` and
/// down to nothing at 0.7 and 1.3, from a transform twice the length. The
/// brick-wall band rings for the whole of a one-second cut: a pure saw at
/// 70 Hz read a 12 Hz wobble of 3 cents (2.9 cents RMS), more than the
/// 2-cent target, and its floor falls only as the note rises.
fn tone_cents(y: &[f64], rate: u32, near: f64) -> Vec<f64> {
    let n = crate::fft::pow2_at_least(2 * y.len());
    let fft = Fft::new(n);
    let mut re = vec![0.0; n];
    let mut im = vec![0.0; n];
    re[..y.len()].copy_from_slice(y);
    fft.forward(&mut re, &mut im);
    for k in 0..n {
        let x = if k <= n / 2 {
            (k as f64 * rate as f64 / n as f64 / near - 1.0).abs()
        } else {
            f64::MAX
        };
        let g = if x <= 0.15 {
            2.0
        } else if x < 0.3 {
            1.0 + (std::f64::consts::PI * (x - 0.15) / 0.15).cos()
        } else {
            0.0
        };
        re[k] *= g;
        im[k] *= g;
    }
    fft.inverse(&mut re, &mut im);
    let k = rate as f64 / TAU;
    let f: Vec<f64> = (1..y.len())
        .map(|i| {
            let d = im[i].atan2(re[i]) - im[i - 1].atan2(re[i - 1]);
            (d + 3.0 * std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI
        })
        .map(|d| d * k)
        .collect();
    let w = ((0.0022 * rate as f64) as usize).max(1);
    let smooth: Vec<f64> = f
        .windows(w)
        .map(|v| v.iter().sum::<f64>() / w as f64)
        .collect();
    let cut = smooth.len() / 10;
    smooth[cut..smooth.len() - cut]
        .iter()
        .map(|f| cents(*f, near))
        .collect()
}

/// The frame of `y` that `flutter::instantaneous_hz(y)`'s first value is
/// centred on.
fn if_start(len: usize, rate: u32) -> usize {
    let w = ((0.0022 * rate as f64) as usize).max(1);
    (len - w) / 10 + w / 2 + 1
}

/// Runs `n` over `x` in 128-frame blocks at 120 BPM with the transport
/// playing from the song position `beats(frame)` at each block's start,
/// and calls `watch` with the node and the frames done after each block.
fn drive(
    n: &mut dyn Node,
    x: &Stereo,
    beats: &dyn Fn(usize) -> f64,
    watch: &mut dyn FnMut(&dyn Node, usize),
) -> Stereo {
    let rate = x.rate;
    let mut l: Vec<f32> = x.left.iter().map(|v| *v as f32).collect();
    let mut r: Vec<f32> = x.right.iter().map(|v| *v as f32).collect();
    let mut at = 0;
    while at < l.len() {
        let m = 128.min(l.len() - at);
        let ctx = Ctx {
            rate,
            frame: at as u64,
            bpm: 120.0,
            beats: beats(at),
            playing: true,
            sidechain: None,
        };
        n.process(&ctx, &[], &mut l[at..at + m], &mut r[at..at + m]);
        at += m;
        watch(&*n, at);
    }
    Stereo::new(
        l.into_iter().map(f64::from).collect(),
        r.into_iter().map(f64::from).collect(),
        rate,
    )
}

/// Round 3's click measure (M13) for every control of `s` except `skip`,
/// with `values` set on top: the rows `click_db_<control>`, under target
/// `id` unless `ids` names another for that control.
fn clicks(
    r: &mut Report,
    s: &Subject,
    values: &[(u16, f64)],
    skip: &[u16],
    id: &'static str,
    ids: &[(u16, &'static str)],
) {
    let rate = r.rate;
    let params = make(s, rate).params();
    for p in params
        .iter()
        .filter(|p| !p.readonly && p.max > p.min && !skip.contains(&p.id))
    {
        let mut d = make_with(s, rate, values);
        let log = matches!(p.curve, Curve::Log);
        let v = hf_click(&mut d, p.id, p.min as f64, p.max as f64, log);
        let target = ids.iter().find(|(c, _)| *c == p.id).map_or(id, |(_, t)| *t);
        r.put(
            target,
            &s.name,
            format!("click_db_{}", slug(p.name)),
            Some(v),
            Better::Lower,
            Some(-60.0),
            3.0,
        );
    }
}

/// The loudest millisecond above 6 kHz while a 997 Hz tone at -12 dBFS
/// plays through `d` for `secs` (the first and last tenth of a second left
/// out), dB against the tone: what an LFO's resets and edges put there.
fn steady_hf(d: &mut NodeDut, secs: f64) -> f64 {
    let rate = d.rate();
    let n = (secs * rate as f64) as usize;
    let x = sine(997.0, db_to_amp(-12.0), n, rate);
    let y = d.render(&mono(x, rate), 0).left;
    let hp = bandpass(&y, rate, 6000.0, rate as f64 / 2.0);
    let ms = rate as usize / 1000;
    let edge = rate as usize / 10;
    let mut loudest = 0.0f64;
    let mut i = edge;
    while i + ms <= n - edge {
        loudest = loudest.max(hp[i..i + ms].iter().map(|v| v * v).sum::<f64>() / ms as f64);
        i += ms / 2;
    }
    floor(db_pow(loudest / (db_to_amp(-12.0).powi(2) / 2.0)))
}

/// The tremolo's gain frame by frame (Volume mode, Width 0) from a steady
/// input of 0.5: the LFO as heard.
fn gain_of(s: &Subject, rate: u32, values: &[(u16, f64)], secs: f64) -> Vec<f64> {
    let n = (secs * rate as f64) as usize;
    let mut d = make_with(s, rate, values);
    d.render(&mono(vec![0.5; n], rate), 0)
        .left
        .iter()
        .map(|x| x * 2.0)
        .collect()
}

/// A square's 10 to 90 % rise on `g` (whose range is 0 to 1), ms: from
/// the last frame under 0.1 to the first at or over 0.9 of the first
/// rising edge.
fn rise_ms(g: &[f64], rate: u32) -> Option<f64> {
    let mut i = 0;
    while i + 1 < g.len() && !(g[i] < 0.1 && g[i + 1] >= 0.1) {
        i += 1;
    }
    let from = i;
    while i < g.len() && g[i] < 0.9 {
        i += 1;
    }
    (i < g.len()).then(|| (i - from) as f64 * 1000.0 / rate as f64)
}

/// How far, in cycles (0 to 0.5), a bar-synced LFO is from the song
/// position after the transport loops back from 3.5 bars to the start:
/// the phase of the left channel's gain (a steady input of 0.5) against
/// the song position over the last bar before the wrap, against the same
/// over the bar that starts 50 ms after it.
fn sync_error(s: &Subject, rate: u32, values: &[(u16, f64)]) -> f64 {
    let n = (9.5 * rate as f64) as usize;
    let song = move |f: usize| (f as f64 / rate as f64 * 2.0) % 14.0;
    let mut node = node_with(s, rate, values);
    let y = drive(
        node.as_mut(),
        &mono(vec![0.5; n], rate),
        &song,
        &mut |_, _| {},
    )
    .left;
    let phase = |from: usize| {
        let (mut c, mut q) = (0.0, 0.0);
        for (i, v) in y.iter().enumerate().skip(from).take(2 * rate as usize) {
            let a = TAU * song(i) / 4.0;
            c += v * a.cos();
            q += v * a.sin();
        }
        q.atan2(c)
    };
    let sec = rate as usize;
    let e = ((phase(7 * sec + sec / 20) - phase(5 * sec)) / TAU).rem_euclid(1.0);
    e.min(1.0 - e)
}

/// A tone at `hz`, `db` dBFS, through `d`: the power more than 30 Hz from
/// every harmonic, against the tone's, dB.
fn alias_db(d: &mut NodeDut, hz: f64, db: f64) -> f64 {
    let rate = d.rate();
    let n = 1 << 16;
    let settle = rate as usize / 2;
    let y = d
        .render(&mono(sine(hz, db_to_amp(db), n + settle, rate), rate), 0)
        .left;
    let (p, bin) = spec(&y[settle..], rate);
    let want: Vec<f64> = (1..40)
        .map(|k| k as f64 * hz)
        .filter(|f| *f < rate as f64 / 2.0)
        .collect();
    let off: f64 = (0..p.len())
        .filter(|k| {
            let f = *k as f64 * bin;
            f >= 20.0 && want.iter().all(|w| (f - w).abs() >= 30.0)
        })
        .map(|k| p[k])
        .sum();
    floor(db_pow(off.max(1e-30) / band(&p, bin, hz, 30.0)))
}

/// The share of `p`'s power more than `tol` (a fraction) from every
/// harmonic of `f0`, against the whole, dB.
fn off_harmonics_db(p: &[f64], bin: f64, f0: f64, tol: f64) -> f64 {
    let mut inside = 0.0;
    for (k, v) in p.iter().enumerate() {
        let f = k as f64 * bin;
        let h = (f / f0).round();
        if h >= 1.0 && (f - h * f0).abs() < h * f0 * tol {
            inside += v;
        }
    }
    let total: f64 = p.iter().sum();
    floor(db_pow((total - inside).max(1e-30) / total))
}

// ---------------------------------------------------------------------------
// Movement (plan-5 sections 4 to 8).

pub(super) fn movement(r: &mut Report) {
    lfo(r);
    tremolo(r);
    autopan(r);
    rotary(r);
    autowah(r);
    ringmod(r);
    freqshift(r);
}

/// The shared LFO (W1 to W3), heard through the tremolo at Depth 100.
fn lfo(r: &mut Report) {
    let s = named(TREMOLO, "movement/lfo");
    // At the report's rate and at 48 kHz: where a phase lands near a wrap
    // depends on the rate.
    let mut rates = vec![(r.rate, String::new())];
    if r.rate != 48_000 {
        rates.push((48_000, "_48k".to_string()));
    }
    for (k, shape) in LFO_SHAPES.iter().enumerate() {
        for hz in [0.5, 4.0, 20.0] {
            for (rate, at) in &rates {
                let rate = *rate;
                let from = rate as usize / 2;
                let v = [(tr::RATE, hz), (tr::SHAPE, k as f64), (tr::DEPTH, 100.0)];
                let g = gain_of(&s, rate, &v, 3.0);
                let jump = g[from..]
                    .windows(2)
                    .map(|w| (w[1] - w[0]).abs())
                    .fold(0.0, f64::max);
                r.put(
                    "W1",
                    &s.name,
                    format!("jump_{shape}_{hz}hz{at}"),
                    Some(jump),
                    Better::Lower,
                    Some(0.05),
                    0.005,
                );
                let mut d = make_with(&s, rate, &v);
                r.put(
                    "W1",
                    &s.name,
                    format!("click_db_{shape}_{hz}hz{at}"),
                    Some(steady_hf(&mut d, 3.0)),
                    Better::Lower,
                    Some(-60.0),
                    3.0,
                );
                if *shape != "square" || !at.is_empty() {
                    continue;
                }
                r.put(
                    "W2",
                    &s.name,
                    format!("edge_ms_{hz}hz"),
                    rise_ms(&g[from..], rate),
                    Better::Near(2.5),
                    Some(1.5),
                    0.05,
                );
            }
        }
    }
    let rate = r.rate;
    for (id, name, sync, depth) in [
        (TREMOLO, "tremolo", tr::SYNC, tr::DEPTH),
        (AUTOPAN, "autopan", ap::SYNC, ap::DEPTH),
    ] {
        let e = sync_error(
            &named(id, &s.name),
            rate,
            &[(sync, ONE_BAR), (depth, 100.0)],
        );
        r.put(
            "W3",
            &s.name,
            format!("sync_error_cycles_{name}"),
            Some(e),
            Better::Lower,
            Some(0.01),
            0.005,
        );
    }
}

/// The tremolo (W4 to W7).
fn tremolo(r: &mut Report) {
    let rate = r.rate;
    let (s, _) = engine_subjects(TREMOLO, "movement", "tremolo");
    let from = rate as usize / 2;
    for depth in [25.0, 50.0, 75.0] {
        let g = gain_of(&s, rate, &[(tr::DEPTH, depth)], 2.0);
        let (lo, hi) = min_max(&g[from..]);
        let want = db_amp(1.0 / (1.0 - depth / 100.0));
        r.put(
            "W5",
            &s.name,
            format!("depth_error_db_{depth}"),
            Some((db_amp(hi / lo) - want).abs()),
            Better::Lower,
            Some(0.2),
            0.05,
        );
    }
    let g = gain_of(&s, rate, &[(tr::DEPTH, 100.0)], 2.0);
    let (lo, hi) = min_max(&g[from..]);
    r.put(
        "W5",
        &s.name,
        "trough_db_depth100",
        Some(db_amp(lo.abs() / hi).max(-80.0)),
        Better::Lower,
        Some(-60.0),
        3.0,
    );
    // Harmonic mode: a 200 Hz and a 3 kHz tone at the same level.
    let n = 3 * rate as usize;
    let x: Vec<f64> = sine(200.0, 0.2, n, rate)
        .iter()
        .zip(sine(3000.0, 0.2, n, rate))
        .map(|(a, b)| a + b)
        .collect();
    let mut d = make_with(&s, rate, &[(tr::DEPTH, 100.0), (tr::MODE, 1.0)]);
    let y = d.render(&mono(x, rate), 0).left;
    let y = &y[rate as usize..];
    let inner = |e: Vec<f64>| swing(&e[1..e.len() - 1]);
    let whole = inner(env_db(y, 10.0, rate));
    let high = inner(env_db(&bandpass(y, rate, 2000.0, 4000.0), 10.0, rate));
    r.put(
        "W6",
        &s.name,
        "harmonic_whole_swing_db",
        Some(whole),
        Better::Lower,
        Some(4.5),
        0.2,
    );
    r.put(
        "W6",
        &s.name,
        "harmonic_3khz_swing_db",
        Some(high),
        Better::Higher,
        Some(20.0),
        0.5,
    );
    clicks(r, &s, &[], &[], "W4", &[(tr::MODE, "W7")]);
}

/// The auto-pan (W4, W8).
fn autopan(r: &mut Report) {
    let rate = r.rate;
    let (s, _) = engine_subjects(AUTOPAN, "movement", "autopan");
    let n = 6 * rate as usize;
    let p = pink(n, 0.1, 41);
    let mut d = make_with(
        &s,
        rate,
        &[(ap::SYNC, 0.0), (ap::RATE, 1.0), (ap::DEPTH, 100.0)],
    );
    let y = d.render(&mono(p.clone(), rate), 0);
    let w = rate as usize / 50;
    let from = rate as usize;
    let rel: Vec<f64> = (from..n - w)
        .step_by(w)
        .map(|i| {
            let out: f64 = (i..i + w)
                .map(|k| y.left[k] * y.left[k] + y.right[k] * y.right[k])
                .sum();
            let inp: f64 = (i..i + w).map(|k| 2.0 * p[k] * p[k]).sum();
            db_pow(out / inp)
        })
        .collect();
    r.put(
        "W8",
        &s.name,
        "power_swing_db",
        Some(swing(&rel)),
        Better::Lower,
        Some(0.25),
        0.05,
    );
    clicks(r, &s, &[], &[], "W4", &[]);
}

/// The rotary's test tones: the drum's 200 Hz and the horn's 1.5, 3 and
/// 6 kHz, each -18 dBFS.
fn rotary_tones(rate: u32, secs: f64, tones: &[f64]) -> Vec<f64> {
    let n = (secs * rate as f64) as usize;
    let mut x = vec![0.0; n];
    for hz in tones {
        for (a, b) in x.iter_mut().zip(sine(*hz, db_to_amp(-18.0), n, rate)) {
            *a += b;
        }
    }
    x
}

/// A rotor as one microphone hears it through the tone at `hz` in `y`:
/// the phase of its level's swing (one turn a rotation, unwrapped), frame
/// by frame.
fn rotor_phase(y: &[f64], rate: u32, hz: f64) -> Vec<f64> {
    let (amp, _) = analytic(y, rate, hz * 0.75, hz * 1.25);
    let mean = amp.iter().sum::<f64>() / amp.len() as f64;
    let env: Vec<f64> = amp.iter().map(|a| a - mean).collect();
    analytic(&env, rate, 0.3, 12.0).1
}

/// A rotor's speed over `from..to` (frames), rotations a second.
fn rotor_hz(ph: &[f64], rate: u32, from: usize, to: usize) -> f64 {
    (ph[to] - ph[from]) / TAU * rate as f64 / (to - from) as f64
}

/// The rotary (W9 to W14, and its clicks under W10).
fn rotary(r: &mut Report) {
    let rate = r.rate;
    let (s, presets) = engine_subjects(ROTARY, "movement", "rotary");
    let sec = rate as usize;
    // W9, W12, W14: steady at Fast and at Slow.
    for (speed, name, secs) in [(2.0, "fast", 5.0), (1.0, "slow", 10.0)] {
        let x = rotary_tones(rate, secs, &[200.0, 1500.0, 3000.0, 6000.0]);
        let mut d = make_with(&s, rate, &[(ro::SPEED, speed)]);
        let y = d.render(&mono(x, rate), 0);
        let (from, to) = (sec, y.len() - sec / 2);
        let horn = rotor_phase(&y.left, rate, 3000.0);
        let drum = rotor_phase(&y.left, rate, 200.0);
        let (horn_want, drum_want) = if speed == 2.0 {
            (6.7, 5.7)
        } else {
            (0.8, 0.67)
        };
        r.put(
            "W9",
            &s.name,
            format!("horn_hz_{name}"),
            Some(rotor_hz(&horn, rate, from, to)),
            Better::Near(horn_want),
            Some(0.02 * horn_want),
            0.01,
        );
        r.put(
            "W9",
            &s.name,
            format!("drum_hz_{name}"),
            Some(rotor_hz(&drum, rate, from, to)),
            Better::Near(drum_want),
            Some(0.02 * drum_want),
            0.01,
        );
        if speed != 2.0 {
            continue;
        }
        // The Doppler, from the tones' pitch.
        for (hz, part, want, within) in [(3000.0, "horn", 37.5, 7.5), (200.0, "drum", 7.5, 2.5)] {
            let c = if_cents(&y.left, rate, hz);
            let skip = (sec / 2).min(c.len());
            let c = &c[skip..];
            let mean = c.iter().sum::<f64>() / c.len() as f64;
            let peak = c.iter().fold(0.0f64, |m, v| m.max((v - mean).abs()));
            r.put(
                "W9",
                &s.name,
                format!("{part}_doppler_cents"),
                Some(peak),
                Better::Near(want),
                Some(within),
                0.5,
            );
        }
        // W12: the horn's level swing at 6 kHz against at 1.5 kHz.
        let swing_at = |hz: f64| {
            let (amp, _) = analytic(&y.left, rate, hz * 0.75, hz * 1.25);
            let (lo, hi) = min_max(&amp[from..to]);
            db_amp(hi / lo.max(1e-12))
        };
        r.put(
            "W12",
            &s.name,
            "beam_db",
            Some(swing_at(6000.0) - swing_at(1500.0)),
            Better::Higher,
            Some(4.0),
            0.3,
        );
        // W14: which way each rotor turns, from which microphone hears it
        // first.
        let lead = |hz: f64| {
            let (l, r) = (
                rotor_phase(&y.left, rate, hz),
                rotor_phase(&y.right, rate, hz),
            );
            let d: f64 = (from..to)
                .map(|i| {
                    (l[i] - r[i] + std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI
                })
                .sum();
            d.signum()
        };
        let opposite = if lead(3000.0) != lead(200.0) {
            1.0
        } else {
            0.0
        };
        r.put(
            "W14",
            &s.name,
            "opposite_rotation",
            Some(opposite),
            Better::Higher,
            Some(1.0),
            0.0,
        );
    }
    // W10: Slow to Fast at 2 s, back to Slow at 10 s.
    let (up_at, down_at, end) = (2 * sec, 10 * sec, 18 * sec);
    let x = rotary_tones(rate, 18.0, &[200.0, 3000.0]);
    let mut d = make_with(&s, rate, &[(ro::SPEED, 1.0)]);
    d.reset();
    let changes = [
        Change {
            frame: up_at,
            id: ro::SPEED,
            value: 2.0,
        },
        Change {
            frame: down_at,
            id: ro::SPEED,
            value: 1.0,
        },
    ];
    let y = d.process(&mono(x, rate), &changes).left;
    for (hz, part, slow, fast) in [(3000.0, "horn", 0.8, 6.7), (200.0, "drum", 0.67, 5.7)] {
        let ph = rotor_phase(&y, rate, hz);
        let w = rate as usize / 50;
        let speed = |i: usize| rotor_hz(&ph, rate, i - w, i + w);
        // From `at`, the time until `ok` holds for good (to `until`), s.
        let settle = |at: usize, until: usize, ok: &dyn Fn(f64) -> bool| {
            let mut last_bad = at;
            for i in (at..until).step_by(w) {
                if !ok(speed(i)) {
                    last_bad = i;
                }
            }
            (last_bad - at) as f64 / rate as f64
        };
        let up = settle(up_at, down_at - w, &|v| v >= 0.9 * fast);
        let down = settle(down_at, end - sec / 2, &|v| v <= slow + 0.1 * (fast - slow));
        let (want, within) = if part == "horn" {
            (1.2, 0.4)
        } else {
            (4.75, 1.25)
        };
        r.put(
            "W10",
            &s.name,
            format!("ramp_up_s_{part}"),
            Some(up),
            Better::Near(want),
            Some(within),
            0.05,
        );
        r.put(
            "W10",
            &s.name,
            format!("ramp_down_over_up_{part}"),
            Some(down / up.max(1e-3)),
            Better::Higher,
            Some(1.0),
            0.05,
        );
    }
    // W11: the level, at Slow and Fast on pink and hits, and each preset.
    let pinks = pink_stim(rate, 6.0);
    let hits = mixlaw::hits(6.0, rate);
    for (speed, name) in [(1.0, "slow"), (2.0, "fast")] {
        for (x, what) in [(&pinks, "pink"), (&hits, "hits")] {
            let mut d = make_with(&s, rate, &[(ro::SPEED, speed)]);
            r.put(
                "W11",
                &s.name,
                format!("lu_{what}_{name}"),
                Some(lu_change(&mut d, x)),
                Better::Near(0.0),
                Some(1.5),
                0.1,
            );
        }
    }
    for p in &presets {
        let mut d = make(p, rate);
        r.put(
            "W11",
            &p.name,
            "lu_pink",
            Some(lu_change(&mut d, &pinks)),
            Better::Near(0.0),
            Some(1.5),
            0.1,
        );
    }
    // W13: the drive's fold-back, stopped.
    for (drive, target) in [(50.0, -60.0), (100.0, -50.0)] {
        let mut d = make_with(&s, rate, &[(ro::SPEED, 0.0), (ro::DRIVE, drive)]);
        r.put(
            "W13",
            &s.name,
            format!("alias_db_5khz_drive{drive}"),
            Some(alias_db(&mut d, 5000.0, -6.0)),
            Better::Lower,
            Some(target),
            1.0,
        );
    }
    clicks(r, &s, &[], &[ro::DRIVE], "W10", &[]);
}

/// The auto wah (W15 to W18).
fn autowah(r: &mut Report) {
    let rate = r.rate;
    let (s, presets) = engine_subjects(AUTOWAH, "movement", "autowah");
    // W15: the Pedal's peak against Low·(High/Low)^Center.
    for res in [0.0, 60.0, 100.0] {
        let mut errs = Vec::new();
        for c in [0.0, 50.0, 100.0] {
            let mut d = make_with(
                &s,
                rate,
                &[
                    (aw::SOURCE, 2.0),
                    (aw::CENTER, c),
                    (aw::RESONANCE, res),
                    (aw::FILTER, 0.0),
                    (aw::MIX, 100.0),
                ],
            );
            let (p, bin) = response(&mut d, 1 << 16);
            let (hz, _) = peak_in(&p, bin, 150.0, 8000.0);
            let want = 300.0 * (2500.0f64 / 300.0).powf(c / 100.0);
            errs.push((hz / want - 1.0).abs() * 100.0);
        }
        r.put(
            "W15",
            &s.name,
            format!("peak_error_pct_res{res}"),
            worst(&errs),
            Better::Lower,
            Some(3.0),
            0.2,
        );
    }
    // W16: the level at the defaults and each preset.
    let pinks = pink_stim(rate, 5.0);
    for p in std::iter::once(&s).chain(&presets) {
        let mut d = make(p, rate);
        r.put(
            "W16",
            &p.name,
            "lu_pink",
            Some(lu_change(&mut d, &pinks)),
            Better::Near(0.0),
            Some(2.0),
            0.1,
        );
    }
    // W17: the level across Resonance, the Pedal at the middle.
    for (filter, name) in [(0.0, "wah"), (1.0, "band-pass")] {
        let lu_at = |res: f64| {
            let mut d = make_with(
                &s,
                rate,
                &[
                    (aw::SOURCE, 2.0),
                    (aw::CENTER, 50.0),
                    (aw::RESONANCE, res),
                    (aw::FILTER, filter),
                    (aw::MIX, 100.0),
                ],
            );
            lu_change(&mut d, &pinks)
        };
        let at60 = lu_at(60.0);
        for res in [0.0, 30.0, 100.0] {
            r.put(
                "W17",
                &s.name,
                format!("lu_vs_res60_{name}_res{res}"),
                Some(lu_at(res) - at60),
                Better::Near(0.0),
                Some(3.0),
                0.1,
            );
        }
    }
    clicks(r, &s, &[], &[], "W18", &[]);
}

/// The ring mod's products of a `fi` Hz sine with a `fc` Hz carrier of
/// wave `wave` (Mix 100, no sweep): the power off every true product
/// (input ± each harmonic the wave has, below Nyquist) against the
/// products', dB.
fn ring_spurs(s: &Subject, rate: u32, wave: f64, fc: f64, fi: f64) -> f64 {
    let mut d = make_with(
        s,
        rate,
        &[
            (rm::FREQ, fc),
            (rm::WAVE, wave),
            (rm::SWEEP, 0.0),
            (rm::MIX, 100.0),
        ],
    );
    let settle = rate as usize / 2;
    let y = d
        .render(&mono(sine(fi, 0.25, (1 << 16) + settle, rate), rate), 0)
        .left;
    let (p, bin) = spec(&y[settle..], rate);
    let ny = rate as f64 / 2.0;
    let mut want = Vec::new();
    for k in 1..400 {
        if wave == 0.0 && k > 1 {
            break;
        }
        if wave != 0.0 && k % 2 == 0 {
            continue;
        }
        for f in [fi + k as f64 * fc, (fi - k as f64 * fc).abs()] {
            if f < ny {
                want.push(f);
            }
        }
    }
    let total: f64 = p.iter().sum();
    let inside: f64 = (0..p.len())
        .filter(|k| want.iter().any(|w| (*k as f64 * bin - w).abs() < 30.0))
        .map(|k| p[k])
        .sum();
    floor(db_pow((total - inside).max(1e-30) / inside))
}

/// The ring mod (Z2 to Z5).
fn ringmod(r: &mut Report) {
    let rate = r.rate;
    let (s, _) = engine_subjects(RINGMOD, "movement", "ringmod");
    r.put(
        "Z2",
        &s.name,
        "spurs_db_sine",
        Some(ring_spurs(&s, rate, 0.0, 3000.0, 7000.0)),
        Better::Lower,
        Some(-80.0),
        1.0,
    );
    for (wave, name) in [(1.0, "triangle"), (2.0, "square")] {
        r.put(
            "Z3",
            &s.name,
            format!("alias_db_{name}"),
            Some(ring_spurs(&s, rate, wave, 440.0, 1000.0)),
            Better::Lower,
            Some(-60.0),
            1.0,
        );
    }
    let top = (0.95 * rate as f64 / 2.0).round();
    r.put(
        "Z3",
        &s.name,
        "alias_db_sine_top_octave",
        Some(ring_spurs(&s, rate, 0.0, 5000.0, top)),
        Better::Info,
        None,
        0.0,
    );
    // Z4: the three waves' loudness on pink.
    let pinks = pink_stim(rate, 5.0);
    let lus: Vec<f64> = (0..3)
        .map(|w| {
            let mut d = make_with(
                &s,
                rate,
                &[
                    (rm::FREQ, 500.0),
                    (rm::WAVE, w as f64),
                    (rm::SWEEP, 0.0),
                    (rm::MIX, 100.0),
                ],
            );
            lu_change(&mut d, &pinks)
        })
        .collect();
    r.put(
        "Z4",
        &s.name,
        "wave_spread_lu",
        Some(swing(&lus)),
        Better::Lower,
        Some(1.5),
        0.1,
    );
    // Z5: Wave's products pass 6 kHz by design, so its switch is round 2's
    // switch click instead; Freq and Sweep move the products past 6 kHz.
    let names = ["sine", "triangle", "square"];
    for (from, to) in [(0usize, 1usize), (1, 2), (2, 0)] {
        let mut d = make_with(&s, rate, &[(rm::WAVE, from as f64)]);
        r.put(
            "Z5",
            &s.name,
            format!("wave_click_db_{}-to-{}", names[from], names[to]),
            Some(switch_click(&mut d, rm::WAVE, to as f64, 997.0)),
            Better::Lower,
            Some(3.0),
            1.0,
        );
    }
    clicks(r, &s, &[], &[rm::FREQ, rm::WAVE, rm::SWEEP], "Z5", &[]);
}

/// The frequency shifter (Z1, Z2, Z5, Z6).
fn freqshift(r: &mut Report) {
    let (s, _) = engine_subjects(FREQSHIFT, "movement", "freqshift");
    for (at, tag) in [(44_100u32, "44k1"), (48_000, "48k")] {
        for shift in [100.0f64, 1000.0] {
            let (mut image, mut spurs) = (Vec::new(), Vec::new());
            for fi in [
                40.0, 60.0, 100.0, 200.0, 1000.0, 5000.0, 10_000.0, 15_000.0, 18_000.0,
            ] {
                let down = (fi - shift).abs();
                if down < 20.0 {
                    continue;
                }
                let mut d = make_with(
                    &s,
                    at,
                    &[(fs::SHIFT, shift), (fs::MODE, 0.0), (fs::MIX, 100.0)],
                );
                let settle = at as usize / 2;
                let y = d
                    .render(&mono(sine(fi, 0.25, (1 << 16) + settle, at), at), 0)
                    .left;
                let (p, bin) = spec(&y[settle..], at);
                let w = band(&p, bin, fi + shift, 6.0);
                let im = band(&p, bin, down, 6.0);
                image.push(floor(db_pow(im / w)));
                // Spurs: everything else, beyond ±30 Hz of the two (as the
                // ring mod's), since ±6 Hz is inside the Hann window's own
                // skirt (-67 to -80 dB).
                let (w30, im30) = (band(&p, bin, fi + shift, 30.0), band(&p, bin, down, 30.0));
                let total: f64 = p.iter().sum();
                spurs.push(floor(db_pow((total - w30 - im30).max(1e-30) / w)));
            }
            r.put(
                "Z1",
                &s.name,
                format!("image_db_{tag}_shift{shift}"),
                worst(&image),
                Better::Lower,
                Some(-60.0),
                1.0,
            );
            r.put(
                "Z2",
                &s.name,
                format!("spurs_db_{tag}_shift{shift}"),
                worst(&spurs),
                Better::Lower,
                Some(-80.0),
                1.0,
            );
        }
    }
    let rate = r.rate;
    // Z6: 95 % feedback on 4 s of white noise, then 3 s of silence.
    let sec = rate as usize;
    let x = white(4 * sec, 0.1, 72);
    let (mut peaks, mut tails) = (Vec::new(), Vec::new());
    let mut finite = true;
    for shift in [0.01, 5.0, 100.0, 2000.0] {
        let mut d = make_with(
            &s,
            rate,
            &[(fs::SHIFT, shift), (fs::FEEDBACK, 95.0), (fs::MIX, 100.0)],
        );
        let y = d.render(&mono(x.clone(), rate), 3 * sec).left;
        finite &= y.iter().all(|v| v.is_finite());
        peaks.push(db_amp(y.iter().fold(0.0f64, |m, v| m.max(v.abs()))));
        let playing = rms_of(&y[2 * sec..4 * sec]);
        tails.push(floor(db_amp(rms_of(&y[6 * sec..7 * sec]) / playing)));
    }
    r.put(
        "Z6",
        &s.name,
        "feedback95_ringout_db",
        worst(&tails).filter(|_| finite),
        Better::Lower,
        Some(-60.0),
        3.0,
    );
    r.put(
        "Z6",
        &s.name,
        "feedback95_peak_dbfs",
        worst(&peaks).filter(|_| finite),
        Better::Info,
        None,
        0.0,
    );
    clicks(r, &s, &[], &[fs::SHIFT, fs::FEEDBACK], "Z5", &[]);
}

// ---------------------------------------------------------------------------
// Pitch (plan-5 sections 9 and 10).

pub(super) fn pitch(r: &mut Report) {
    shifter(r);
    correct(r);
    harmonizer(r);
    tuner(r);
}

fn semis_tag(semis: f64) -> String {
    if semis >= 0.0 {
        format!("up{semis}")
    } else {
        format!("down{}", -semis)
    }
}

/// The pitch shifter (H1 to H5).
fn shifter(r: &mut Report) {
    let rate = r.rate;
    let (s, _) = engine_subjects(PITCH, "pitch", "pitch-shifter");
    let settle = rate as usize / 2;
    // H1: 13 sines, 110 Hz to 1.76 kHz in half octaves, at four shifts.
    for semis in [12.0, 7.0, -5.0, -12.0] {
        let tag = semis_tag(semis);
        let (mut offs, mut errs, mut swings) = (Vec::new(), Vec::new(), Vec::new());
        for k in 0..13 {
            let f = 110.0 * 2f64.powf(k as f64 / 2.0 + 0.037);
            let mut d = make_with(&s, rate, &[(ps::PITCH, semis), (ps::MIX, 100.0)]);
            let x = sine(f, 0.25, (1 << 16) + settle, rate);
            let y = d.render(&mono(x, rate), 0).left;
            let y = &y[settle..];
            let want = f * 2f64.powf(semis / 12.0);
            let (p, bin) = spec(y, rate);
            let inside = band(&p, bin, want, want * 0.02);
            let total: f64 = p.iter().sum();
            offs.push(floor(db_pow((total - inside).max(1e-30) / inside)));
            errs.push(cents(crossing_hz(y, rate), want).abs());
            swings.push(swing(&level_db(y, rate, want)));
        }
        let mut sorted = offs.clone();
        sorted.sort_by(f64::total_cmp);
        r.put(
            "H1",
            &s.name,
            format!("off_target_db_{tag}"),
            worst(&offs),
            Better::Lower,
            Some(-30.0),
            1.0,
        );
        r.put(
            "H1",
            &s.name,
            format!("off_target_db_median_{tag}"),
            Some(sorted[sorted.len() / 2]),
            Better::Info,
            None,
            0.0,
        );
        r.put(
            "H1",
            &s.name,
            format!("pitch_error_cents_{tag}"),
            worst(&errs),
            Better::Lower,
            Some(5.0),
            0.5,
        );
        r.put(
            "H1",
            &s.name,
            format!("level_swing_db_{tag}"),
            worst(&swings),
            Better::Lower,
            Some(3.0),
            0.3,
        );
    }
    // H2: a chord of saws (A3, C#4, E4) up an octave.
    let n = (1 << 17) + settle;
    let mut x = vec![0.0; n];
    for f in [220.0, 277.18, 329.63] {
        for (a, b) in x.iter_mut().zip(saw(f, 0.08, n, rate, 0.0, 0.0)) {
            *a += b;
        }
    }
    let mut d = make_with(&s, rate, &[(ps::PITCH, 12.0), (ps::MIX, 100.0)]);
    let y = d.render(&mono(x, rate), 0).left;
    let (p, bin) = spec(&y[settle..], rate);
    let mut want = Vec::new();
    for f in [440.0f64, 554.36, 659.26] {
        let mut k = 1.0;
        while k * f < 20_000f64.min(0.45 * rate as f64) {
            want.push(k * f);
            k += 1.0;
        }
    }
    let inside: f64 = (0..p.len())
        .filter(|k| {
            let f = *k as f64 * bin;
            want.iter().any(|w| (f - w).abs() < w * 0.015)
        })
        .map(|k| p[k])
        .sum();
    let total: f64 = p.iter().sum();
    r.put(
        "H2",
        &s.name,
        "chord_off_target_db",
        Some(floor(db_pow((total - inside).max(1e-30) / inside))),
        Better::Lower,
        Some(-15.0),
        1.0,
    );
    // H3: one click at +12: its copies over -12 dB, and their spread.
    let mut d = make_with(&s, rate, &[(ps::PITCH, 12.0), (ps::MIX, 100.0)]);
    let mut x = vec![0.0; rate as usize];
    x[rate as usize / 3] = 1.0;
    let y: Vec<f64> = d
        .render(&mono(x, rate), 0)
        .left
        .iter()
        .map(|v| v.abs())
        .collect();
    let loudest = y.iter().fold(0.0f64, |m, v| m.max(*v));
    let gap = rate as usize / 1000;
    let mut copies: Vec<usize> = Vec::new();
    for i in gap..y.len() - gap {
        if y[i] >= 0.25 * loudest
            && y[i - gap..i + gap].iter().all(|v| *v <= y[i])
            && copies.last().is_none_or(|l| i - l > gap)
        {
            copies.push(i);
        }
    }
    let spread = match (copies.first(), copies.last()) {
        (Some(a), Some(b)) => (b - a) as f64 * 1000.0 / rate as f64,
        _ => 0.0,
    };
    r.put(
        "H3",
        &s.name,
        "impulse_copies_up12",
        Some(copies.len() as f64),
        Better::Info,
        None,
        0.0,
    );
    r.put(
        "H3",
        &s.name,
        "impulse_spread_ms_up12",
        Some(spread),
        Better::Info,
        None,
        0.0,
    );
    // H4: the delay at 0 semitones, and what it reports.
    let mut d = make_with(&s, rate, &[(ps::PITCH, 0.0), (ps::MIX, 100.0)]);
    let x = pink(2 * rate as usize, 0.1, 5);
    let y = d.render(&mono(x.clone(), rate), 0).left;
    let lag = lag_of(&x, &y, rate as usize / 5);
    r.put(
        "H4",
        &s.name,
        "delay_ms",
        Some(lag as f64 * 1000.0 / rate as f64),
        Better::Lower,
        Some(15.0),
        0.5,
    );
    r.put(
        "H4",
        &s.name,
        "reported_latency_frames",
        Some(d.latency() as f64),
        Better::Near(0.0),
        Some(0.0),
        0.0,
    );
    let mut d = make_with(&s, rate, &[(ps::PITCH, 0.0), (ps::MIX, 50.0)]);
    let (p, bin) = response(&mut d, 1 << 16);
    let audio: Vec<f64> = (0..p.len())
        .filter(|k| (100.0..10_000.0).contains(&(*k as f64 * bin)))
        .map(|k| db_pow(p[k]))
        .collect();
    r.put(
        "H4",
        &s.name,
        "comb_db_mix50",
        Some(swing(&audio)),
        Better::Info,
        None,
        0.0,
    );
    // H5: the highs a semitone up on pink.
    let x = pink(4 * rate as usize, 0.1, 91);
    let mut d = make_with(&s, rate, &[(ps::PITCH, 1.0), (ps::MIX, 100.0)]);
    let y = d.render(&mono(x.clone(), rate), 0).left;
    let tilt = |v: &[f64]| {
        let (p, bin) = spec(v, rate);
        db_pow((band(&p, bin, 14_000.0, 2000.0) / 4000.0) / (band(&p, bin, 1500.0, 500.0) / 1000.0))
    };
    let sec = rate as usize;
    r.put(
        "H5",
        &s.name,
        "hf_tilt_error_db_up1",
        Some(tilt(&y[sec..]) - tilt(&x[sec..])),
        Better::Near(0.0),
        Some(0.5),
        0.1,
    );
    clicks(r, &s, &[], &[ps::FEEDBACK], "H13", &[]);
}

/// The nearest note of C major to `hz`, Hz (A4 = 440).
fn c_major(hz: f64) -> f64 {
    let m = 69.0 + 12.0 * (hz / 440.0).log2();
    let lo = m.floor() as i32 - 2;
    let best = (lo..lo + 5)
        .filter(|k| [0, 2, 4, 5, 7, 9, 11].contains(&k.rem_euclid(12)))
        .min_by(|a, b| (*a as f64 - m).abs().total_cmp(&(*b as f64 - m).abs()))
        .unwrap_or(69);
    440.0 * 2f64.powf((best - 69) as f64 / 12.0)
}

/// The ranges and the pitches (Hz) each listens for.
const RANGES: [(&str, f64, f64); 3] = [
    ("low", 60.0, 600.0),
    ("mid", 90.0, 900.0),
    ("high", 150.0, 1500.0),
];

/// Pitch correct (H6 to H9, H11 to H13).
fn correct(r: &mut Report) {
    let rate = r.rate;
    let (s, _) = engine_subjects(PITCH_CORRECT, "pitch", "pitch-correct");
    let sec = rate as usize;
    // Key C major, Speed 0, Amount 100.
    let base = |extra: &[(u16, f64)]| {
        let mut v = vec![
            (pc::KEY, 0.0),
            (pc::SCALE, 1.0),
            (pc::SPEED, 0.0),
            (pc::AMOUNT, 100.0),
            (pc::MIX, 100.0),
        ];
        v.extend_from_slice(extra);
        v
    };
    // H6: steady saws in each range's span.
    for (k, (name, lo, hi)) in RANGES.iter().enumerate() {
        let (mut errs, mut wobbles) = (Vec::new(), Vec::new());
        for f in [70.0, 104.0, 150.0, 230.0, 450.0, 700.0, 1200.0] {
            if f < *lo || f > *hi {
                continue;
            }
            let want = c_major(f);
            let mut d = make_with(&s, rate, &base(&[(pc::RANGE, k as f64)]));
            let y = d
                .render(&mono(saw(f, 0.2, 2 * sec, rate, 0.0, 0.0), rate), 0)
                .left;
            let c = tone_cents(&y[sec..], rate, want);
            let mean = c.iter().sum::<f64>() / c.len() as f64;
            let sd = (c.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / c.len() as f64).sqrt();
            errs.push(mean.abs());
            wobbles.push(sd);
        }
        r.put(
            "H6",
            &s.name,
            format!("accuracy_cents_{name}"),
            worst(&errs),
            Better::Lower,
            Some(3.0),
            0.3,
        );
        r.put(
            "H6",
            &s.name,
            format!("wobble_cents_{name}"),
            worst(&wobbles),
            Better::Lower,
            Some(2.0),
            0.3,
        );
    }
    // H7: a ±40-cent vibrato at 5.5 Hz on A4.
    let vib = saw(440.0, 0.2, 4 * sec, rate, 40.0, 5.5);
    let vibrato = |y: &[f64]| {
        let c = tone_cents(&y[sec..], rate, 440.0);
        let w = rate as usize / 100;
        let sm: Vec<f64> = c
            .windows(w)
            .step_by(w / 2)
            .map(|v| v.iter().sum::<f64>() / w as f64)
            .collect();
        let mean = sm.iter().sum::<f64>() / sm.len() as f64;
        sm.iter().fold(0.0f64, |m, v| m.max((v - mean).abs()))
    };
    let input = vibrato(&vib);
    let mut d = make_with(&s, rate, &base(&[]));
    let left = vibrato(&d.render(&mono(vib.clone(), rate), 0).left);
    r.put(
        "H7",
        &s.name,
        "vibrato_left_cents_speed0",
        Some(left),
        Better::Lower,
        Some(5.0),
        0.5,
    );
    let mut d = make_with(&s, rate, &base(&[(pc::SPEED, 100.0)]));
    let kept = vibrato(&d.render(&mono(vib, rate), 0).left) / input;
    r.put(
        "H7",
        &s.name,
        "vibrato_kept_speed100",
        Some(kept),
        Better::Higher,
        Some(0.9),
        0.02,
    );
    // H8: a saw at A4 + 30 cents from silence; how long until the output's
    // pitch (over 10 ms) stays within 5 cents of A4, after the latency.
    for speed in [0.0, 20.0, 50.0, 100.0, 200.0] {
        let mut d = make_with(&s, rate, &base(&[(pc::SPEED, speed)]));
        let lat = d.latency();
        let onset = sec / 2;
        let mut x = vec![0.0; onset];
        x.extend(saw(447.7, 0.2, 2 * sec, rate, 0.0, 0.0));
        let y = d.render(&mono(x, rate), 0).left;
        let w = rate as usize / 100;
        let c: Vec<f64> = tone_cents(&y, rate, 440.0)
            .windows(w)
            .map(|v| v.iter().sum::<f64>() / w as f64)
            .collect();
        let first = if_start(y.len(), rate) + w / 2;
        let last_bad = (0..c.len()).rev().find(|i| c[*i].abs() > 5.0);
        let ms = last_bad.map(|i| (first + i + 1) as f64 - onset as f64 - lat as f64);
        r.put(
            "H8",
            &s.name,
            format!("settle_ms_speed{speed}"),
            ms.map(|f| f * 1000.0 / rate as f64),
            Better::Lower,
            Some(1.8 * speed + 25.0),
            3.0,
        );
    }
    // H9: an in-tune saw against the input's own; a saw 45 cents sharp.
    let n = (1 << 17) + sec;
    let x = saw(220.0, 0.2, n, rate, 0.0, 0.0);
    let mut d = make_with(&s, rate, &base(&[]));
    let y = d.render(&mono(x.clone(), rate), 0).left;
    let (p, bin) = spec(&y[sec..], rate);
    let (pi, _) = spec(&x[sec..], rate);
    r.put(
        "H9",
        &s.name,
        "in_tune_off_harmonic_over_input_db",
        Some(off_harmonics_db(&p, bin, 220.0, 0.01) - off_harmonics_db(&pi, bin, 220.0, 0.01)),
        Better::Lower,
        Some(3.0),
        0.5,
    );
    let mut d = make_with(&s, rate, &base(&[]));
    let y = d
        .render(&mono(saw(451.6, 0.2, n, rate, 0.0, 0.0), rate), 0)
        .left;
    let (p, bin) = spec(&y[sec..], rate);
    r.put(
        "H9",
        &s.name,
        "correcting_off_harmonic_db",
        Some(off_harmonics_db(&p, bin, 440.0, 0.01)),
        Better::Lower,
        Some(-50.0),
        1.0,
    );
    // H11: noise passes; the note meter on four kinds of tone.
    let x = white(4 * sec, 0.1, 3);
    let mut d = make_with(&s, rate, &base(&[]));
    let y = d.render(&mono(x.clone(), rate), 0).left;
    r.put(
        "H11",
        &s.name,
        "noise_change_db",
        Some(db_amp(rms_of(&y[sec..]) / rms_of(&x[sec..]))),
        Better::Near(0.0),
        Some(0.5),
        0.1,
    );
    let names: &[&str] = match make(&s, rate).params()[pc::NOTE as usize].curve {
        Curve::Choice(n) => n,
        _ => &[],
    };
    let mut wrong = 0;
    for kind in 0..4 {
        for (hz, note) in [(110.0, "A2"), (220.0, "A3"), (440.0, "A4")] {
            let x: Vec<f64> = match kind {
                0 => sine(hz, 0.25, sec, rate),
                1 => saw(hz, 0.2, sec, rate, 0.0, 0.0),
                _ => (0..sec)
                    .map(|i| {
                        let t = i as f64 / rate as f64;
                        let ks: Vec<usize> = if kind == 2 {
                            (1..40).step_by(2).collect()
                        } else {
                            (2..12).collect()
                        };
                        ks.iter()
                            .filter(|k| **k as f64 * hz < 20_000.0)
                            .map(|k| (TAU * *k as f64 * hz * t).sin() / *k as f64)
                            .sum::<f64>()
                            * 0.2
                    })
                    .collect(),
            };
            let mut node = node_with(&s, rate, &base(&[(pc::RANGE, 1.0)]));
            drive(node.as_mut(), &mono(x, rate), &|_| 0.0, &mut |_, _| {});
            let read = names.get(node.param(pc::NOTE) as usize).copied();
            if read != Some(note) {
                wrong += 1;
            }
        }
    }
    r.put(
        "H11",
        &s.name,
        "wrong_notes",
        Some(wrong as f64),
        Better::Lower,
        Some(0.0),
        0.0,
    );
    // H12: the latency each range reports against the delay of pink
    // (which passes uncorrected).
    let x = pink(2 * sec, 0.1, 5);
    for (k, (name, _, _)) in RANGES.iter().enumerate() {
        let mut d = make_with(&s, rate, &base(&[(pc::RANGE, k as f64)]));
        let y = d.render(&mono(x.clone(), rate), 0).left;
        let lag = lag_of(&x, &y, sec / 10);
        r.put(
            "H12",
            &s.name,
            format!("latency_error_frames_{name}"),
            Some((lag as f64 - d.latency() as f64).abs()),
            Better::Lower,
            Some(1.0),
            0.0,
        );
    }
    clicks(r, &s, &[], &[], "H13", &[]);
}

/// The harmonizer's voices, by their Voice choice: (index, row name,
/// the voice's note from A4 in C major, Hz).
const VOICES: [(f64, &str, f64); 3] = [
    (7.0, "3rd-up", 523.25),
    (11.0, "octave-up", 880.0),
    (5.0, "3rd-down", 349.23),
];

/// A sung vowel's stand-in: a pulse train at `f0` (harmonics falling
/// 6 dB an octave) through three formant resonances.
fn vowel(f0: f64, formants: [f64; 3], n: usize, rate: u32) -> Vec<f64> {
    let bw = [80.0, 100.0, 140.0];
    let lv = [0.0, -6.0, -14.0];
    let mut amps = Vec::new();
    let mut k = 1.0;
    while k * f0 < 8000.0 {
        let f = k * f0;
        let a: f64 = (0..3)
            .map(|j| {
                let r = 1.0 / (1.0 + ((f - formants[j]) / (bw[j] / 2.0)).powi(2)).sqrt();
                r * db_to_amp(lv[j])
            })
            .sum();
        amps.push((f, a / k));
        k += 1.0;
    }
    (0..n)
        .map(|i| {
            let t = i as f64 / rate as f64;
            0.15 * amps
                .iter()
                .map(|(f, a)| a * (TAU * f * t).sin())
                .sum::<f64>()
        })
        .collect()
}

/// The spectral envelope's peak between `lo` and `hi` Hz, read from the
/// harmonics of `f0` in `p`: the loudest harmonic's frequency.
fn formant_peak(p: &[f64], bin: f64, f0: f64, lo: f64, hi: f64) -> f64 {
    let mut best = (0.0, f64::NEG_INFINITY);
    let mut k = (lo / f0).ceil();
    while k * f0 <= hi {
        let v = band(p, bin, k * f0, f0 * 0.25);
        if v > best.1 {
            best = (k * f0, v);
        }
        k += 1.0;
    }
    best.0
}

/// The harmonizer (H6, H9, H10, H12, H13).
fn harmonizer(r: &mut Report) {
    let rate = r.rate;
    let (s, _) = engine_subjects(HARMONIZER, "pitch", "harmonizer");
    let sec = rate as usize;
    let n = (1 << 17) + sec;
    let x = saw(440.0, 0.2, n, rate, 0.0, 0.0);
    // Voice 1 alone on an A4 saw, C major: its pitch and its roughness.
    for (voice, name, want) in VOICES {
        let mut d = make_with(
            &s,
            rate,
            &[
                (hm::KEY, 0.0),
                (hm::SCALE, 1.0),
                (hm::VOICE1, voice),
                (hm::VOICE2, 0.0),
                (hm::LEVEL1, 100.0),
                (hm::DRY, 0.0),
                (hm::PAN1, 0.0),
            ],
        );
        let y = d.render(&mono(x.clone(), rate), 0).left;
        let (p, bin) = spec(&y[sec..], rate);
        let (hz, _) = peak_in(&p, bin, want * 0.97, want * 1.03);
        r.put(
            "H6",
            &s.name,
            format!("interval_error_cents_{name}"),
            Some(cents(hz, want).abs()),
            Better::Lower,
            Some(3.0),
            0.3,
        );
        r.put(
            "H9",
            &s.name,
            format!("voice_off_harmonic_db_{name}"),
            Some(off_harmonics_db(&p, bin, want, 0.01)),
            Better::Lower,
            Some(-40.0),
            1.0,
        );
    }
    // H10: the formants of an "AH" sung at 130 Hz, in and out.
    let x = vowel(130.0, [730.0, 1090.0, 2440.0], n, rate);
    let (pi, bin) = spec(&x[sec..], rate);
    let f_in = [
        formant_peak(&pi, bin, 130.0, 400.0, 950.0),
        formant_peak(&pi, bin, 130.0, 950.0, 1700.0),
    ];
    for (voice, name, semis) in [
        (11.0, "octave-up", 12.0),
        (9.0, "5th-up", 7.0),
        (1.0, "octave-down", -12.0),
    ] {
        let mut d = make_with(
            &s,
            rate,
            &[
                (hm::KEY, 0.0),
                (hm::SCALE, 0.0),
                (hm::VOICE1, voice),
                (hm::VOICE2, 0.0),
                (hm::LEVEL1, 100.0),
                (hm::DRY, 0.0),
                (hm::PAN1, 0.0),
            ],
        );
        let y = d.render(&mono(x.clone(), rate), 0).left;
        let (p, _) = spec(&y[sec..], rate);
        let f0 = 130.0 * 2f64.powf(semis / 12.0);
        let out = [
            formant_peak(&p, bin, f0, 400.0, 950.0),
            formant_peak(&p, bin, f0, 950.0, 1700.0),
        ];
        for (k, f) in ["f1", "f2"].iter().enumerate() {
            r.put(
                "H10",
                &s.name,
                format!("formant_{f}_error_harmonics_{name}"),
                Some((out[k] - f_in[k]).abs() / f0),
                Better::Lower,
                Some(1.0),
                0.01,
            );
        }
    }
    // H12: the dry alone, against what it reports.
    let x = pink(2 * sec, 0.1, 5);
    let mut d = make_with(&s, rate, &[(hm::LEVEL1, 0.0), (hm::LEVEL2, 0.0)]);
    let y = d.render(&mono(x.clone(), rate), 0).left;
    let lag = lag_of(&x, &y, sec / 10);
    r.put(
        "H12",
        &s.name,
        "latency_error_frames",
        Some((lag as f64 - d.latency() as f64).abs()),
        Better::Lower,
        Some(1.0),
        0.0,
    );
    clicks(r, &s, &[], &[], "H13", &[]);
}

/// The tuner (H14): a tone 13 cents sharp of each note from A0 to G6, as
/// a sine, a saw, and a saw with white noise 30 dB down.
fn tuner(r: &mut Report) {
    let rate = r.rate;
    let (s, _) = engine_subjects(TUNER, "pitch", "tuner");
    let sec = rate as usize;
    let n = (1.5 * rate as f64) as usize;
    for (kind, name) in [(0, "sine"), (1, "saw"), (2, "saw-noise")] {
        let (mut errs, mut times, mut jitters) = (Vec::new(), Vec::new(), Vec::new());
        for note in [
            27.5, 30.87, 41.2, 55.0, 82.41, 110.0, 196.0, 329.63, 440.0, 880.0, 1318.5, 1567.98,
        ] {
            let hz = note * 2f64.powf(13.0 / 1200.0);
            let mut x = match kind {
                0 => sine(hz, 0.25, n, rate),
                _ => saw(hz, 0.2, n, rate, 0.0, 0.0),
            };
            if kind == 2 {
                for (a, b) in x.iter_mut().zip(white(n, 0.2 * 0.03 / 1.3, 9)) {
                    *a += b;
                }
            }
            let mut node = node_with(&s, rate, &[]);
            let mut reads = Vec::new();
            drive(node.as_mut(), &mono(x, rate), &|_| 0.0, &mut |n, at| {
                reads.push((at, n.param(tu::CENTS) as f64, n.param(tu::HZ) as f64));
            });
            let settled = reads
                .iter()
                .find(|(_, c, h)| *h > 0.0 && (c - 13.0).abs() <= 2.0 && cents(*h, hz).abs() < 50.0)
                .map(|(at, _, _)| *at as f64 * 1000.0 / rate as f64);
            times.push(settled.unwrap_or(1500.0));
            if let Some((_, _, h)) = reads.iter().find(|(at, _, _)| *at >= sec) {
                errs.push(cents(*h, hz).abs());
            }
            let tail: Vec<f64> = reads
                .iter()
                .filter(|(at, _, _)| *at >= sec)
                .map(|(_, _, h)| cents(*h, hz))
                .collect();
            let mean = tail.iter().sum::<f64>() / tail.len() as f64;
            jitters.push(
                (tail.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / tail.len() as f64).sqrt(),
            );
        }
        r.put(
            "H14",
            &s.name,
            format!("error_cents_{name}"),
            worst(&errs),
            Better::Lower,
            Some(0.5),
            0.05,
        );
        r.put(
            "H14",
            &s.name,
            format!("settle_ms_{name}"),
            worst(&times),
            Better::Lower,
            Some(100.0),
            5.0,
        );
        r.put(
            "H14",
            &s.name,
            format!("jitter_cents_{name}"),
            worst(&jitters),
            Better::Lower,
            Some(1.0),
            0.1,
        );
    }
    clicks(r, &s, &[], &[], "H13", &[]);
}

// ---------------------------------------------------------------------------
// Voice (plan-5 section 11).

/// Peterson and Barney's vowels (1952, men): the talk box's Vowel choices
/// in order, their formants (Hz), and the levels of F2 and F3 under F1
/// (dB) from their table II.
const VOWELS: [(&str, [f64; 3], [f64; 2]); 9] = [
    ("oo", [300.0, 870.0, 2240.0], [-16.0, -40.0]),
    ("ou", [440.0, 1020.0, 2240.0], [-11.0, -33.0]),
    ("aw", [570.0, 840.0, 2410.0], [-7.0, -34.0]),
    ("ah", [730.0, 1090.0, 2440.0], [-4.0, -27.0]),
    ("uh", [640.0, 1190.0, 2390.0], [-9.0, -26.0]),
    ("ae", [660.0, 1720.0, 2410.0], [-11.0, -21.0]),
    ("eh", [530.0, 1840.0, 2480.0], [-15.0, -22.0]),
    ("ih", [390.0, 1990.0, 2550.0], [-20.0, -24.0]),
    ("ee", [270.0, 2290.0, 3010.0], [-20.0, -24.0]),
];

pub(super) fn voice(r: &mut Report) {
    talkbox(r);
    vocoder(r);
}

/// The talk box (V1 to V5).
fn talkbox(r: &mut Report) {
    let rate = r.rate;
    let (s, presets) = engine_subjects(TALKBOX, "voice", "talkbox");
    let sec = rate as usize;
    let saws = mono(saw(110.0, 0.2, (1 << 17) + sec, rate, 0.0, 0.0), rate);
    let mut saw_lus = Vec::new();
    for (i, (name, f, pb)) in VOWELS.iter().enumerate() {
        let v = [
            (tb::VOWEL, i as f64),
            (tb::ENV_DEPTH, 0.0),
            (tb::LFO_DEPTH, 0.0),
            (tb::MIX, 100.0),
        ];
        let mut d = make_with(&s, rate, &v);
        let (p, bin) = response(&mut d, 1 << 16);
        let db: Vec<f64> = p.iter().map(|v| db_pow(*v)).collect();
        let peak = |lo: f64, hi: f64| {
            let (a, b) = ((lo / bin) as usize, (hi / bin) as usize);
            let k = (a..=b).fold(a, |m, k| if db[k] > db[m] { k } else { m });
            (k as f64 * bin, db[k])
        };
        let (f1, _) = peak(f[0] * 0.8, f[0] * 1.2);
        let (f2, l2) = peak(f[1] * 0.85, f[1] * 1.15);
        let (f3, _) = peak(f[2] * 0.9, f[2] * 1.1);
        let err = [f1, f2, f3]
            .iter()
            .zip(f)
            .map(|(got, want)| (got / want - 1.0).abs() * 100.0)
            .fold(0.0, f64::max);
        r.put(
            "V1",
            &s.name,
            format!("formant_error_pct_{name}"),
            Some(err),
            Better::Lower,
            Some(3.0),
            0.2,
        );
        // V2: the dip between F1 and F2 under F2, against an all-pole
        // cascade's with the same formants and bandwidths.
        let dip = db[(f1 / bin) as usize..=(f2 / bin) as usize]
            .iter()
            .fold(f64::INFINITY, |m, v| m.min(*v))
            - l2;
        let cascade = |hz: f64| -> f64 {
            [80.0, 100.0, 140.0]
                .iter()
                .zip(f)
                .map(|(bw, fj)| {
                    db_pow(fj.powi(4) / ((fj * fj - hz * hz).powi(2) + (bw * hz).powi(2)))
                })
                .sum()
        };
        let mut low = f64::INFINITY;
        let mut hz = f[0];
        while hz < f[1] {
            low = low.min(cascade(hz));
            hz += 5.0;
        }
        r.put(
            "V2",
            &s.name,
            format!("dip_excess_db_{name}"),
            Some(low - cascade(f[1]) - dip),
            Better::Lower,
            Some(3.0),
            0.3,
        );
        // V3: the harmonics nearest each formant on a 110 Hz saw.
        let y = d.render(&saws, 0);
        let (p, bn) = spec(&y.left[sec..], rate);
        let h = |hz: f64| db_pow(band(&p, bn, (hz / 110.0).round() * 110.0, 30.0));
        let (s1, s2, s3) = (h(f1), h(f2), h(f3));
        for (k, (got, want)) in [(s2 - s1, pb[0]), (s3 - s1, pb[1])].iter().enumerate() {
            r.put(
                "V3",
                &s.name,
                format!("f{}_level_error_db_{name}", k + 2),
                Some((got - want).abs()),
                Better::Lower,
                Some(4.0),
                0.3,
            );
        }
        saw_lus.push(lu_change(&mut d, &saws));
    }
    // V4: the level across the vowels, and on pink at the defaults and
    // each preset.
    r.put(
        "V4",
        &s.name,
        "vowel_spread_lu",
        Some(swing(&saw_lus)),
        Better::Lower,
        Some(4.0),
        0.1,
    );
    let pinks = pink_stim(rate, 5.0);
    for p in std::iter::once(&s).chain(&presets) {
        let mut d = make(p, rate);
        r.put(
            "V4",
            &p.name,
            "lu_pink",
            Some(lu_change(&mut d, &pinks)),
            Better::Near(0.0),
            Some(2.0),
            0.1,
        );
    }
    clicks(r, &s, &[], &[], "V5", &[]);
}

/// `x` scaled to `db` dBFS RMS.
fn at_rms(x: Vec<f64>, db: f64) -> Vec<f64> {
    let g = db_to_amp(db) / rms_of(&x).max(1e-12);
    x.into_iter().map(|v| v * g).collect()
}

/// The vocoder (V6). Keyed, its input (the carrier) is a 110 Hz saw and
/// its key (the voice) a sung vowel at 220 Hz, both at -18 dBFS RMS, a
/// voice's usual level.
fn vocoder(r: &mut Report) {
    let rate = r.rate;
    let (s, presets) = engine_subjects(VOCODER, "voice", "vocoder");
    let n = 4 * rate as usize;
    let sec = rate as usize;
    let lat = make(&s, rate).latency() as f64;
    r.put(
        "V6",
        &s.name,
        "latency_frames",
        Some(lat),
        Better::Lower,
        Some(0.0),
        0.0,
    );
    let input = [(vc::CARRIER, 2.0)];
    let carrier = mono(at_rms(saw(110.0, 1.0, n, rate, 0.0, 0.0), -18.0), rate);
    let lu_carrier = loudness::integrated(&carrier.slice(sec, n - sec));
    let (pc, _) = spec(&carrier.left[sec..], rate);
    // The bands' spacing at the defaults, in octaves.
    let spacing = (8000.0f64 / 100.0).log2() / 15.0;
    for (name, f, _) in VOWELS.iter() {
        let voice = mono(at_rms(vowel(220.0, *f, n, rate), -18.0), rate);
        let mut node = node_with(&s, rate, &input);
        let (y, _) = run_keyed(node.as_mut(), &carrier, Some(&voice), &steady(rate), None);
        r.put(
            "V6",
            &s.name,
            format!("lu_carrier_{name}"),
            Some(loudness::integrated(&y.slice(sec, n - sec)) - lu_carrier),
            Better::Near(0.0),
            Some(2.0),
            0.1,
        );
        // Near each of F1 and F2 (between it and its neighbours, no more
        // than 1.5 bands either side), on the 110 Hz grid: where the
        // voice is loudest, and where the output's gain over the carrier
        // peaks; how far apart, in bands.
        let (py, bin) = spec(&y.left[sec..], rate);
        let (pv, _) = spec(&voice.left[sec..], rate);
        let gain = |hz: f64| band(&py, bin, hz, 30.0) / band(&pc, bin, hz, 30.0).max(1e-30);
        let loud = |hz: f64| band(&pv, bin, hz, 30.0);
        let reach = 2f64.powf(1.5 * spacing);
        for j in 0..2 {
            let lo = if j == 0 {
                f[0] / reach
            } else {
                (f[0] * f[1]).sqrt().max(f[1] / reach)
            };
            let hi = (f[j] * reach).min((f[j] * f[j + 1]).sqrt());
            let peak = |of: &dyn Fn(f64) -> f64| {
                let mut best = (f[j], f64::NEG_INFINITY);
                let mut k = (lo / 110.0).ceil();
                while k * 110.0 <= hi {
                    let g = of(k * 110.0);
                    if g > best.1 {
                        best = (k * 110.0, g);
                    }
                    k += 1.0;
                }
                best.0
            };
            r.put(
                "V6",
                &s.name,
                format!("f{}_error_bands_{name}", j + 1),
                Some((peak(&gain) / peak(&loud)).log2().abs() / spacing),
                Better::Lower,
                Some(1.0),
                0.1,
            );
        }
    }
    // The defaults and the presets on pink at the strip (the voice): the
    // saw speaks it at about its level.
    let pinks = pink_stim(rate, 5.0);
    for p in std::iter::once(&s).chain(&presets) {
        let mut d = make(p, rate);
        r.put(
            "V6",
            &p.name,
            "lu_pink",
            Some(lu_change(&mut d, &pinks)),
            Better::Near(0.0),
            Some(2.0),
            0.1,
        );
    }
    // An "s": white noise above 4 kHz at -18 dBFS RMS, as the voice on
    // the strip (the saw) and on the key (the 110 Hz saw through the
    // input): its level out against in.
    let ess = mono(
        at_rms(bandpass(&white(n, 1.0, 11), rate, 4000.0, 20_000.0), -18.0),
        rate,
    );
    let ess_rms = rms_of(&ess.left[sec..]);
    let mut d = make(&s, rate);
    let y = d.render(&ess, 0);
    r.put(
        "V6",
        &s.name,
        "s_level_db",
        Some(floor(db_amp(rms_of(&y.left[sec..]) / ess_rms))),
        Better::Higher,
        Some(-6.0),
        0.2,
    );
    let mut node = node_with(&s, rate, &input);
    let (y, _) = run_keyed(node.as_mut(), &carrier, Some(&ess), &steady(rate), None);
    r.put(
        "V6",
        &s.name,
        "s_level_db_keyed",
        Some(floor(db_amp(rms_of(&y.left[sec..]) / ess_rms))),
        Better::Higher,
        Some(-6.0),
        0.2,
    );
    // Clicks: the 997 Hz tone as the voice and the carrier both (keyed by
    // the input), every control but Carrier (which changes what is heard).
    let params = make(&s, rate).params();
    for p in params
        .iter()
        .filter(|p| !p.readonly && p.max > p.min && p.id != vc::CARRIER)
    {
        let mut d = make_with(&s, rate, &input).keyed_by_input();
        let log = matches!(p.curve, Curve::Log);
        let v = hf_click(&mut d, p.id, p.min as f64, p.max as f64, log);
        r.put(
            "V6",
            &s.name,
            format!("click_db_{}", slug(p.name)),
            Some(v),
            Better::Lower,
            Some(-60.0),
            3.0,
        );
    }
}
