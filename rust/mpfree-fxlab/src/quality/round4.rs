//! The fourth round's classes of the quality report: the rest of the
//! dynamics. The Limiter and Master safety (the plan's target ids P), the
//! gate (N), the de-esser (S), the multiband compressor (X), the ducker
//! (U), the transient shaper (T) and the channel strip (C). The targets and
//! why each was chosen are in `docs/dev/design/fx-quality.md`. The rows,
//! the report and the ratchet are the first round's (`super`); the measures
//! shared with the second round's classes are in `round2.rs`, and the ones
//! only these classes need live here.

use mpfree_fx::ids;
use mpfree_fx::mpfree_node::{Ctx, Curve, Node, Timed, MAX_BLOCK};

use super::round2::{
    alias, db_amp, db_pow, engine_subjects, make_with, rel_d2, slope_zipper, switch_click,
};
use super::{floor, lu_change, make, Better, Report, Subject};
use crate::dut::{Change, Dut};
use crate::fft::Fft;
use crate::signals::{db_to_amp, level_steps, pink, rms_of, sine, white};
use crate::wav::Stereo;
use crate::{dynamics, harmonics, loudness, mixlaw, response};

// ---------------------------------------------------------------------------
// Shared measures.

/// The MPC's period: the frames each `run` block holds.
const BLOCK: usize = 128;

/// A subject that is a setting of an engine rather than a preset.
fn named(id: &'static str, name: &str, fixed: &'static [(u16, f32)]) -> Subject {
    Subject {
        name: name.to_string(),
        id,
        fixed,
        preset: None,
    }
}

/// Whether preset subject `s` sets control `id` to at least `v`.
fn preset_sets(s: &Subject, id: u16, v: f32) -> bool {
    s.preset
        .is_some_and(|p| p.values.iter().any(|(i, x)| *i == id && *x >= v))
}

/// A fresh node for `s` with `values` on top of its preset and fixed
/// values, for the measures that read a meter, feed a side-chain or move
/// the song position.
pub(super) fn node_with(s: &Subject, rate: u32, values: &[(u16, f64)]) -> Box<dyn Node> {
    let mut n =
        mpfree_fx::registry::make(s.id).unwrap_or_else(|| panic!("{} is in the registry", s.id));
    n.prepare(rate, MAX_BLOCK);
    if let Some(p) = s.preset {
        for (id, v) in p.values {
            n.set_param(*id, *v);
        }
    }
    for (id, v) in s.fixed {
        n.set_param(*id, *v);
    }
    for (id, v) in values {
        n.set_param(*id, *v as f32);
    }
    n.reset();
    n
}

/// The song position at 120 BPM with the transport playing from 0.
fn steady(rate: u32) -> impl Fn(usize) -> f64 {
    move |f| f as f64 / rate as f64 * 2.0
}

/// Runs `n` over `x` in 128-frame blocks at 120 BPM with the transport
/// playing and the song position `beats(frame)` at each block's start,
/// keyed by `side` when given. Returns the output and the reading of meter
/// `meter` after each block.
fn run(
    n: &mut dyn Node,
    x: &Stereo,
    side: Option<&Stereo>,
    beats: &dyn Fn(usize) -> f64,
    meter: Option<u16>,
) -> (Stereo, Vec<f32>) {
    let rate = x.rate;
    let len = x.len();
    let f32s = |v: &[f64]| v.iter().map(|s| *s as f32).collect::<Vec<f32>>();
    let (mut l, mut r) = (f32s(&x.left), f32s(&x.right));
    let (sl, sr) = side.map_or((Vec::new(), Vec::new()), |s| {
        (f32s(&s.left), f32s(&s.right))
    });
    let events: [Timed; 0] = [];
    let mut meters = Vec::new();
    let mut at = 0;
    while at < len {
        let m = BLOCK.min(len - at);
        let ctx = Ctx {
            rate,
            frame: at as u64,
            bpm: 120.0,
            beats: beats(at),
            playing: true,
            sidechain: side.map(|_| (&sl[at..at + m], &sr[at..at + m])),
        };
        n.process(&ctx, &events, &mut l[at..at + m], &mut r[at..at + m]);
        if let Some(id) = meter {
            meters.push(n.param(id));
        }
        at += m;
    }
    let back = |v: Vec<f32>| v.into_iter().map(f64::from).collect();
    (Stereo::new(back(l), back(r), rate), meters)
}

fn pink_rms(rate: u32, rms_db: f64, secs: f64) -> Stereo {
    Stereo::mono(
        pink((secs * rate as f64) as usize, db_to_amp(rms_db), 7),
        rate,
    )
}

fn scaled(x: &Stereo, db: f64) -> Stereo {
    let g = db_to_amp(db);
    Stereo::new(
        x.left.iter().map(|v| v * g).collect(),
        x.right.iter().map(|v| v * g).collect(),
        x.rate,
    )
}

/// A loud programme: pink at -14 dBFS RMS, drum hits (peaks about
/// +2.5 dBFS) and a 55 Hz bass at -12 dBFS, `secs` long.
fn programme(rate: u32, secs: f64) -> Stereo {
    let n = (secs * rate as f64) as usize;
    let p = pink(n, db_to_amp(-14.0), 11);
    let h = mixlaw::hits(secs, rate);
    let b = sine(55.0, db_to_amp(-12.0), n, rate);
    Stereo::mono((0..n).map(|i| p[i] + h.left[i] + b[i]).collect(), rate)
}

/// A sine at `hz` falling evenly in dB from `from` to `to` dBFS over
/// `secs`.
fn falling_tone(hz: f64, from: f64, to: f64, secs: f64, rate: u32) -> Vec<f64> {
    let n = (secs * rate as f64) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / n as f64;
            db_to_amp(from + (to - from) * t)
                * (std::f64::consts::TAU * hz * i as f64 / rate as f64).sin()
        })
        .collect()
}

/// Per half cycle of a sine `x` (`y` lined up with it): the input's peak
/// (dBFS) and the least and most gain (dB) over the frames above 30 % of
/// that peak.
fn half_cycle_ripple(x: &[f64], y: &[f64], hz: f64, rate: u32) -> Vec<(f64, f64, f64)> {
    let half = (rate as f64 / hz / 2.0) as usize;
    let mut out = Vec::new();
    let mut at = 0;
    while at + half < x.len().min(y.len()) {
        let m = x[at..at + half].iter().fold(0.0f64, |a, v| a.max(v.abs()));
        if m > 1e-9 {
            let (mut lo, mut hi) = (f64::MAX, f64::MIN);
            for i in at..at + half {
                if x[i].abs() > 0.3 * m {
                    let g = db_amp((y[i] / x[i]).abs());
                    lo = lo.min(g);
                    hi = hi.max(g);
                }
            }
            out.push((db_amp(m), lo, hi));
        }
        at += half;
    }
    out
}

/// Per half cycle of a sine `x` (`y` lined up with it): the frame of the
/// input's peak and the gain there (dB).
fn half_cycle_gains(x: &[f64], y: &[f64], hz: f64, rate: u32) -> Vec<(usize, f64)> {
    let half = (rate as f64 / hz / 2.0) as usize;
    let mut out = Vec::new();
    let mut at = 0;
    while at + half < x.len().min(y.len()) {
        let (mut k, mut m) = (at, 0.0f64);
        for (i, v) in x.iter().enumerate().skip(at).take(half) {
            if v.abs() > m {
                m = v.abs();
                k = i;
            }
        }
        if m > 1e-9 {
            out.push((k, db_amp((y[k] / x[k]).abs())));
        }
        at += half;
    }
    out
}

/// The transfer function |H| (dB) of `y` over `x` from Welch cross-spectra
/// (Hann, `n`-point segments, half overlap), at each bin's frequency.
fn transfer(x: &[f64], y: &[f64], n: usize, rate: u32) -> Vec<(f64, f64)> {
    let fft = Fft::new(n);
    let win: Vec<f64> = (0..n)
        .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos())
        .collect();
    let mut sxx = vec![0.0; n / 2];
    let mut sxy_re = vec![0.0; n / 2];
    let mut sxy_im = vec![0.0; n / 2];
    let mut at = 0;
    while at + n <= x.len().min(y.len()) {
        let (mut xr, mut xi) = (vec![0.0; n], vec![0.0; n]);
        let (mut yr, mut yi) = (vec![0.0; n], vec![0.0; n]);
        for i in 0..n {
            xr[i] = x[at + i] * win[i];
            yr[i] = y[at + i] * win[i];
        }
        fft.forward(&mut xr, &mut xi);
        fft.forward(&mut yr, &mut yi);
        for k in 0..n / 2 {
            sxx[k] += xr[k] * xr[k] + xi[k] * xi[k];
            sxy_re[k] += xr[k] * yr[k] + xi[k] * yi[k];
            sxy_im[k] += xr[k] * yi[k] - xi[k] * yr[k];
        }
        at += n / 2;
    }
    (1..n / 2)
        .map(|k| {
            let h = (sxy_re[k].powi(2) + sxy_im[k].powi(2)).sqrt() / sxx[k].max(1e-30);
            (k as f64 * rate as f64 / n as f64, db_amp(h))
        })
        .collect()
}

/// A transfer function smoothed to third octaves from 25 Hz: each band's
/// centre and mean dB.
fn third_octaves(h: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut f = 25.0f64;
    while f < 20_000.0 {
        let (lo, hi) = (f / 2f64.powf(1.0 / 6.0), f * 2f64.powf(1.0 / 6.0));
        let v: Vec<f64> = h
            .iter()
            .filter(|p| p.0 >= lo && p.0 < hi)
            .map(|p| p.1)
            .collect();
        if !v.is_empty() {
            out.push((f, v.iter().sum::<f64>() / v.len() as f64));
        }
        f *= 2f64.powf(1.0 / 3.0);
    }
    out
}

/// A control's name as a metric word: "Low/mid" is "low-mid".
fn word(name: &str) -> String {
    name.to_lowercase().replace([' ', '/'], "-")
}

/// The slope zipper (round 2's relative measure, 50 ms sweeps) of each of
/// `controls` on `s` with `values`: its id, and the span swept (its whole
/// range when `None`). Target 3 dB; a sweep under a clean tone reads 0, so
/// a control that leaves the output silent can't read better.
fn zippers(
    r: &mut Report,
    target: &'static str,
    s: &Subject,
    values: &[(u16, f64)],
    controls: &[(u16, Option<(f64, f64)>)],
) {
    let rate = r.rate;
    for (id, span) in controls {
        let mut d = make_with(s, rate, values);
        let p = d
            .params()
            .into_iter()
            .find(|p| p.id == *id)
            .unwrap_or_else(|| panic!("{} has control {id}", s.id));
        let (lo, hi) = span.unwrap_or((p.min as f64, p.max as f64));
        let log = matches!(p.curve, Curve::Log);
        let z = slope_zipper(&mut d, *id, lo, hi, log, 50.0).max(0.0);
        r.put(
            target,
            &s.name,
            format!("zipper_{}", word(p.name)),
            Some(z),
            Better::Lower,
            Some(3.0),
            0.5,
        );
    }
}

/// The largest minus the smallest of `v`.
fn spread(v: &[f64]) -> f64 {
    v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min)
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

/// The spread of latencies (frames) of `s` across `settings`.
fn latency_spread(s: &Subject, rate: u32, settings: &[&[(u16, f64)]]) -> f64 {
    let l: Vec<f64> = settings
        .iter()
        .map(|v| make_with(s, rate, v).latency() as f64)
        .collect();
    spread(&l)
}

// ---------------------------------------------------------------------------
// Limiter and Master safety (plan-4 section 4).

const LIMITER: &str = "mpfree.fx.limiter";
const MASTER: &str = "mpfree.fx.master_safety";

/// The worst sample peak and true peak over the ceiling (dB) of `s` on
/// the programme and on pink at -14 dBFS RMS, each pushed 0, 6 and 12 dB
/// by control `push`.
fn peaks_over(s: &Subject, rate: u32, push: u16, ceiling: u16) -> (f64, f64) {
    let secs = 5.0;
    let signals = [programme(rate, secs), pink_rms(rate, -14.0, secs)];
    let (mut sp, mut tp) = (f64::MIN, f64::MIN);
    for x in &signals {
        for db in [0.0, 6.0, 12.0] {
            let mut d = make_with(s, rate, &[(push, db)]);
            let ceil = d.node().param(ceiling) as f64;
            let lat = d.latency();
            let y = d.render(x, lat);
            let a = rate as usize;
            let body = y.slice(a + lat, x.len() - a);
            sp = sp.max(loudness::sample_peak(&body) - ceil);
            tp = tp.max(loudness::true_peak(&body) - ceil);
        }
    }
    (sp, tp)
}

pub(super) fn limiter(r: &mut Report) {
    use ids::limiter as li;
    use ids::master as mi;
    let rate = r.rate;
    let (lim, presets) = engine_subjects(LIMITER, "limiter", "limiter");
    let tp = named(
        LIMITER,
        "limiter/limiter-true-peak",
        &[(li::TRUE_PEAK, 1.0)],
    );
    let fixed = named(LIMITER, "limiter/limiter-auto-off", &[(li::AUTO, 0.0)]);
    let safe = named(MASTER, "limiter/master-safety", &[(mi::MODE, 0.0)]);
    let clip = named(MASTER, "limiter/master-soft-clip", &[(mi::MODE, 1.0)]);
    // (subject, push control, ceiling control, whether its true peaks
    // are caught: P2's target, P6's for the soft clip).
    let mut all: Vec<(&Subject, u16, u16, Option<&'static str>)> = vec![
        (&lim, li::GAIN, li::CEILING, None),
        (&tp, li::GAIN, li::CEILING, Some("P2")),
        (&fixed, li::GAIN, li::CEILING, None),
        (&safe, mi::TRIM, mi::CEILING, Some("P2")),
        (&clip, mi::TRIM, mi::CEILING, Some("P6")),
    ];
    for p in &presets {
        let caught = preset_sets(p, li::TRUE_PEAK, 0.5).then_some("P2");
        all.push((p, li::GAIN, li::CEILING, caught));
    }

    // P1, P2 (and P6's true peak): peaks against the ceiling.
    for (s, push, ceiling, caught) in &all {
        let (sp, tpk) = peaks_over(s, rate, *push, *ceiling);
        r.put(
            "P1",
            &s.name,
            "sample_peak_over_ceiling_db",
            Some(sp),
            Better::Lower,
            Some(0.01),
            0.01,
        );
        r.put(
            caught.unwrap_or("P2"),
            &s.name,
            "true_peak_over_ceiling_db",
            Some(tpk),
            Better::Lower,
            caught.map(|_| 0.1),
            0.05,
        );
    }

    // P3: bass distortion under 6 dB of limiting (12 dB reported).
    for (s, ceiling, target) in [
        (&lim, li::CEILING, true),
        (&tp, li::CEILING, false),
        (&fixed, li::CEILING, false),
        (&safe, mi::CEILING, true),
    ] {
        for hz in [50.0, 100.0] {
            for over in [6.0, 12.0] {
                let mut d = make(s, rate);
                let ceil = d.node().param(ceiling) as f64;
                let thd = floor(harmonics::measure(&mut d, hz, ceil + over).thd_db);
                let t = (target && over == 6.0).then_some(if hz < 75.0 { -40.0 } else { -50.0 });
                r.put(
                    "P3",
                    &s.name,
                    format!("thd_{hz}hz_{over}db_over"),
                    Some(thd),
                    Better::Lower,
                    t,
                    1.0,
                );
            }
        }
    }
    for p in &presets {
        let mut d = make(p, rate);
        let ceil = d.node().param(li::CEILING) as f64;
        let thd = floor(harmonics::measure(&mut d, 50.0, ceil + 6.0).thd_db);
        r.put(
            "P3",
            &p.name,
            "thd_50hz_6db_over",
            Some(thd),
            Better::Lower,
            None,
            1.0,
        );
    }

    // P3's price: the loudness the programme keeps when pushed 6 dB into
    // the ceiling (the push less what the limiting takes back), so the
    // hold's and the release's cost in punch is seen.
    for (s, push) in [
        (&lim, li::GAIN),
        (&tp, li::GAIN),
        (&fixed, li::GAIN),
        (&safe, mi::TRIM),
    ] {
        let mut d = make_with(s, rate, &[(push, 6.0)]);
        r.put(
            "P3",
            &s.name,
            "programme_lu_change_6db_push",
            Some(lu_change(&mut d, &programme(rate, 5.0))),
            Better::Info,
            None,
            0.0,
        );
    }

    // P4: Auto release comes back slower after a long push than a short
    // one; P5: with it off (and in Master safety) the release means its
    // label.
    for s in [&lim, &tp, &fixed] {
        let after = |loud: f64| {
            let mut d = make(s, rate);
            let ceil = d.node().param(li::CEILING) as f64;
            dynamics::release_after(&mut d, -20.0, ceil + 6.0, loud, 1.5).0
        };
        let (short, long) = (after(0.1), after(3.0));
        r.put(
            "P4",
            &s.name,
            "release_ms_after_100ms",
            short,
            Better::Info,
            None,
            0.0,
        );
        r.put(
            "P4",
            &s.name,
            "release_ms_after_3s",
            long,
            Better::Info,
            None,
            0.0,
        );
        let auto = !std::ptr::eq(s, &fixed);
        r.put(
            "P4",
            &s.name,
            "release_slowdown",
            short.zip(long).map(|(a, b)| b / a),
            if auto { Better::Higher } else { Better::Info },
            auto.then_some(2.0),
            0.2,
        );
    }
    for (s, release, ceiling) in [
        (&fixed, li::RELEASE, li::CEILING),
        (&safe, mi::RELEASE, mi::CEILING),
    ] {
        for rel in [20.0, 80.0, 300.0] {
            let mut d = make_with(s, rate, &[(release, rel)]);
            let ceil = d.node().param(ceiling) as f64;
            let t = dynamics::timing(&mut d, -20.0, ceil + 6.0, (8.0 * rel / 1000.0).max(0.5));
            r.put(
                "P5",
                &s.name,
                format!("release_ratio_{rel}ms"),
                t.release_ms.map(|v| v / rel),
                Better::Near(1.0),
                Some(0.15),
                0.03,
            );
        }
    }

    // P6: the soft clip's aliasing (the limiters' reported).
    for (s, ceiling, target) in [
        (&clip, mi::CEILING, true),
        (&safe, mi::CEILING, false),
        (&lim, li::CEILING, false),
        (&tp, li::CEILING, false),
    ] {
        for over in [6.0, 12.0] {
            let mut d = make(s, rate);
            let ceil = d.node().param(ceiling) as f64;
            let (db, _) = alias(&mut d, 5000.0, ceil + over);
            r.put(
                "P6",
                &s.name,
                format!("alias_5khz_{over}db_over"),
                Some(db),
                Better::Lower,
                (target && over == 6.0).then_some(-60.0),
                1.0,
            );
        }
    }

    // P7: the latency never moves.
    let lim_settings: [&[(u16, f64)]; 7] = [
        &[],
        &[(li::TRUE_PEAK, 1.0)],
        &[(li::AUTO, 0.0)],
        &[(li::RELEASE, 5.0)],
        &[(li::RELEASE, 1000.0)],
        &[(li::CEILING, -12.0)],
        &[(li::GAIN, 24.0)],
    ];
    let ms_settings: [&[(u16, f64)]; 5] = [
        &[(mi::MODE, 0.0)],
        &[(mi::MODE, 1.0)],
        &[(mi::RELEASE, 10.0)],
        &[(mi::CEILING, -6.0)],
        &[(mi::TRIM, 12.0)],
    ];
    for (s, settings) in [(&lim, &lim_settings[..]), (&safe, &ms_settings[..])] {
        r.put(
            "P7",
            &s.name,
            "latency_frames",
            Some(make(s, rate).latency() as f64),
            Better::Info,
            None,
            0.0,
        );
        r.put(
            "P7",
            &s.name,
            "latency_spread_frames",
            Some(latency_spread(s, rate, settings)),
            Better::Lower,
            Some(0.0),
            0.0,
        );
    }

    // P8: zipper.
    zippers(
        r,
        "P8",
        &lim,
        &[],
        &[(li::GAIN, None), (li::CEILING, None), (li::RELEASE, None)],
    );
    zippers(
        r,
        "P8",
        &safe,
        &[],
        &[(mi::TRIM, None), (mi::CEILING, None), (mi::RELEASE, None)],
    );
}

// ---------------------------------------------------------------------------
// Gate (plan-4 section 5).

const GATE: &str = "mpfree.fx.gate";

/// The energy of the first `ms` of each hit out against in (dB), latency
/// removed: 0 when the gate keeps the fronts.
fn onset_db(d: &mut dyn Dut, hits: &Stereo, ms: f64) -> f64 {
    let rate = hits.rate;
    let lat = d.latency();
    let y = d.render(hits, lat);
    let every = rate as usize / 2;
    let m = (ms / 1000.0 * rate as f64) as usize;
    let (mut ein, mut eout) = (0.0, 0.0);
    let mut at = 0;
    while at + every < hits.len() {
        ein += hits.left[at..at + m].iter().map(|v| v * v).sum::<f64>();
        eout += y.left[at + lat..at + lat + m]
            .iter()
            .map(|v| v * v)
            .sum::<f64>();
        at += every;
    }
    db_pow(eout / ein)
}

/// A tone at `hz` falling 20 dB over to 20 dB under the threshold in 3 s:
/// the worst gain ripple inside a half cycle while the input is 2 dB or
/// more over the threshold, and the input level (dBFS) where the gate has
/// shut (gain under -6 dB).
fn falls_through(d: &mut dyn Dut, thr: f64, hz: f64) -> (f64, Option<f64>) {
    let rate = d.rate();
    let x = falling_tone(hz, thr + 20.0, thr - 20.0, 3.0, rate);
    let lat = d.latency();
    let y = d.render(&Stereo::mono(x.clone(), rate), lat);
    let rip = half_cycle_ripple(&x, &y.left[lat..], hz, rate);
    let worst = rip
        .iter()
        .filter(|h| h.0 > thr + 2.0)
        .fold(0.0f64, |a, h| a.max(h.2 - h.1));
    (worst, rip.iter().find(|h| h.2 < -6.0).map(|h| h.0))
}

/// How long the gate takes to fade from open to its Range once a 1 kHz
/// tone drops from 20 dB over the threshold to 20 dB under (ms, from
/// -0.5 dB to within 1 dB of Range).
fn fade_ms(d: &mut dyn Dut, thr: f64, range: f64, release_ms: f64) -> Option<f64> {
    let rate = d.rate();
    let hz = 1000.0;
    let drop = rate as usize / 2;
    let n = drop + ((1.5 * release_ms / 1000.0 + 0.3) * rate as f64) as usize;
    let x: Vec<f64> = sine(hz, 1.0, n, rate)
        .iter()
        .enumerate()
        .map(|(i, v)| v * db_to_amp(if i < drop { thr + 20.0 } else { thr - 20.0 }))
        .collect();
    let lat = d.latency();
    let y = d.render(&Stereo::mono(x.clone(), rate), lat);
    let g = half_cycle_gains(&x, &y.left[lat..], hz, rate);
    let from = g.iter().find(|(k, gain)| *k >= drop && *gain < -0.5)?.0;
    let to = g
        .iter()
        .find(|(k, gain)| *k >= from && *gain < range + 1.0)?
        .0;
    Some((to - from) as f64 * 1000.0 / rate as f64)
}

pub(super) fn gate(r: &mut Report) {
    use ids::gate as g;
    let rate = r.rate;
    let (main, presets) = engine_subjects(GATE, "gate", "gate");
    let name = main.name.clone();
    let name = name.as_str();

    // N1: a hit from silence keeps its first millisecond.
    let hits = mixlaw::hits(4.0, rate);
    for att in [0.1, 0.5, 1.0, 5.0] {
        let mut d = make_with(&main, rate, &[(g::ATTACK, att)]);
        let within = att <= 1.0;
        r.put(
            "N1",
            name,
            format!("onset_1ms_db_attack{att}ms"),
            Some(onset_db(&mut d, &hits, 1.0).abs()),
            if within { Better::Lower } else { Better::Info },
            within.then_some(0.5),
            0.1,
        );
    }
    for p in &presets {
        let mut d = make(p, rate);
        r.put(
            "N1",
            &p.name,
            "onset_1ms_db",
            Some(onset_db(&mut d, &hits, 1.0).abs()),
            Better::Lower,
            None,
            0.1,
        );
    }

    // N2, N3: a tone falling through the threshold: no chatter, and it
    // shuts at the threshold less the hysteresis.
    let thr = make(&main, rate).node().param(g::THRESHOLD) as f64;
    for hz in [40.0, 60.0, 100.0, 1000.0] {
        for hold in [0.0, 30.0] {
            let mut d = make_with(&main, rate, &[(g::HOLD, hold)]);
            let (ripple, shut) = falls_through(&mut d, thr, hz);
            r.put(
                "N2",
                name,
                format!("ripple_db_{hz}hz_hold{hold}ms"),
                Some(ripple),
                Better::Lower,
                Some(0.5),
                0.1,
            );
            r.put(
                "N3",
                name,
                format!("shut_error_db_{hz}hz_hold{hold}ms"),
                shut.map(|s| (s - (thr - 4.0)).abs()),
                Better::Lower,
                Some(1.0),
                0.2,
            );
        }
    }

    // N4: open to Range in Release (the fade is even in dB, so the part
    // from -0.5 dB to 1 dB short of Range takes that share of it).
    for rel in [50.0, 120.0, 500.0] {
        let mut d = make_with(&main, rate, &[(g::HOLD, 0.0), (g::RELEASE, rel)]);
        let range = d.node().param(g::RANGE) as f64;
        let share = (range.abs() - 1.5) / range.abs();
        r.put(
            "N4",
            name,
            format!("release_ratio_{rel}ms"),
            fade_ms(&mut d, thr, range, rel).map(|ms| ms / (rel * share)),
            Better::Near(1.0),
            Some(0.1),
            0.03,
        );
    }

    // N5: zipper, open (the threshold under the tone) and shut (Range sets
    // the level).
    zippers(
        r,
        "N5",
        &main,
        &[],
        &[
            (g::THRESHOLD, Some((-80.0, -24.0))),
            (g::ATTACK, None),
            (g::HOLD, None),
            (g::RELEASE, None),
            (g::KEY_HP, None),
            (g::KEY_LP, None),
        ],
    );
    zippers(r, "N5", &main, &[(g::THRESHOLD, 0.0)], &[(g::RANGE, None)]);

    // N5: and Range moved while the gate is shut takes effect: -80 to
    // -40 dB under a tone that keeps it shut, the gain against -40 dB
    // (floored at -128 dB, -140 dBFS out).
    let mut d = make_with(&main, rate, &[(g::THRESHOLD, 0.0), (g::RANGE, -80.0)]);
    let half = rate as usize / 2;
    let x = sine(997.0, db_to_amp(-12.0), 2 * half, rate);
    d.reset();
    let y = d
        .process(
            &Stereo::mono(x.clone(), rate),
            &[Change {
                frame: half,
                id: g::RANGE,
                value: -40.0,
            }],
        )
        .left;
    let tail = 2 * half - rate as usize / 10..2 * half;
    let level = db_amp(rms_of(&y[tail.clone()]) / rms_of(&x[tail])).max(-128.0);
    r.put(
        "N5",
        name,
        "range_change_while_shut_error_db",
        Some((level + 40.0).abs()),
        Better::Lower,
        Some(1.0),
        0.2,
    );

    // N6: the expander's curve: a steady tone 3 to 12 dB under the
    // threshold comes out Ratio less one dB down per dB under (40 Hz and
    // 1 kHz, Ratio 2 and 4).
    let n = rate as usize;
    let mut worst = 0.0f64;
    for hz in [40.0, 1000.0] {
        for ratio in [2.0, 4.0] {
            for under in [3.0, 6.0, 12.0] {
                let mut d = make_with(&main, rate, &[(g::MODE, 1.0), (g::RATIO, ratio)]);
                let x = sine(hz, db_to_amp(thr - under), n, rate);
                let lat = d.latency();
                let y = d.render(&Stereo::mono(x.clone(), rate), lat).left;
                let got = db_amp(rms_of(&y[n / 2 + lat..n + lat]) / rms_of(&x[n / 2..]));
                worst = worst.max((got + under * (ratio - 1.0)).abs());
            }
        }
    }
    r.put(
        "N6",
        name,
        "expander_curve_error_db",
        Some(worst),
        Better::Lower,
        Some(0.5),
        0.1,
    );
}

// ---------------------------------------------------------------------------
// De-esser (plan-4 section 6).

const DEESSER: &str = "mpfree.fx.deesser";

/// A vocal-like line: a 150 Hz buzz (harmonics 1/n up to 4 kHz) for
/// 400 ms, then an s (white noise high-passed twice at 4 kHz, 150 ms) over
/// the buzz 12 dB down, repeated; the peak at `peak_db`. With `left_only`
/// the s is on the left only. Also returns which frames are s.
fn vocal_line(rate: u32, secs: f64, peak_db: f64, left_only: bool) -> (Stereo, Vec<bool>) {
    let n = (secs * rate as f64) as usize;
    let noise = white(n, 1.0, 5);
    let a = (-std::f64::consts::TAU * 4000.0 / rate as f64).exp();
    let mut s = vec![0.0; n];
    let (mut x1, mut y1, mut x2, mut y2) = (0.0, 0.0, 0.0, 0.0);
    for i in 0..n {
        let h1 = a * (y1 + noise[i] - x1);
        x1 = noise[i];
        y1 = h1;
        let h2 = a * (y2 + h1 - x2);
        x2 = h1;
        y2 = h2;
        s[i] = h2;
    }
    let srms = (s.iter().map(|v| v * v).sum::<f64>() / n as f64).sqrt();
    let cycle = (0.55 * rate as f64) as usize;
    let voiced = (0.4 * rate as f64) as usize;
    let (mut l, mut r, mut is_s) = (vec![0.0; n], vec![0.0; n], vec![false; n]);
    for i in 0..n {
        let t = i as f64 / rate as f64;
        let mut buzz = 0.0;
        let mut k = 1;
        while 150.0 * k as f64 <= 4000.0 {
            buzz += (std::f64::consts::TAU * 150.0 * k as f64 * t).sin() / k as f64;
            k += 1;
        }
        is_s[i] = i % cycle >= voiced;
        let v = if is_s[i] { 0.25 * buzz } else { buzz } * 0.3;
        let sv = if is_s[i] { s[i] / srms * 0.25 } else { 0.0 };
        l[i] = v + sv;
        r[i] = v + if left_only { 0.0 } else { sv };
    }
    let peak = l.iter().chain(r.iter()).fold(0.0f64, |m, v| m.max(v.abs()));
    let g = db_to_amp(peak_db) / peak;
    for v in l.iter_mut().chain(r.iter_mut()) {
        *v *= g;
    }
    (Stereo::new(l, r, rate), is_s)
}

/// The de-esser's mean reduction (its meter, dB) on the vocal line during
/// the s, and between them (leaving out the 100 ms after each).
fn vocal_reduction(
    s: &Subject,
    rate: u32,
    detect: f64,
    peak_db: f64,
    left_only: bool,
) -> (f64, f64) {
    use ids::deesser as de;
    let (x, is_s) = vocal_line(rate, 4.4, peak_db, left_only);
    let mut n = node_with(s, rate, &[(de::DETECT, detect)]);
    let (_, m) = run(n.as_mut(), &x, None, &steady(rate), Some(de::REDUCTION));
    let settle = rate as usize / 10;
    let (mut ss, mut buzz) = (Vec::new(), Vec::new());
    for (b, v) in m.iter().enumerate() {
        let f = b * BLOCK + BLOCK / 2;
        if f < rate as usize / 2 || f >= is_s.len() {
            continue;
        }
        if is_s[f] {
            ss.push(*v as f64);
        } else if !is_s[f - settle] {
            buzz.push(*v as f64);
        }
    }
    (mean(&ss), mean(&buzz))
}

pub(super) fn deesser(r: &mut Report) {
    use ids::deesser as de;
    let rate = r.rate;
    let (main, _) = engine_subjects(DEESSER, "deesser", "deesser");
    let split = named(DEESSER, "deesser/split", &[(de::MODE, 0.0)]);
    let wide = named(DEESSER, "deesser/wide", &[(de::MODE, 1.0)]);

    // S1, S2: the response at full reduction (Absolute, the threshold at
    // its lowest) on pink: Split mustn't lift the band under Freq, and
    // reaches Range above it.
    let x = pink_rms(rate, -12.0, 4.0);
    for (s, is_split) in [(&split, true), (&wide, false)] {
        for range in [10.0, 24.0] {
            for freq in [3000.0, 6000.0] {
                let mut d = make_with(
                    s,
                    rate,
                    &[
                        (de::DETECT, 1.0),
                        (de::THRESHOLD, -60.0),
                        (de::RANGE, range),
                        (de::FREQ, freq),
                    ],
                );
                let lat = d.latency();
                let y = d.render(&x, lat);
                let a = rate as usize;
                let h = third_octaves(&transfer(&x.left[a..], &y.left[a + lat..], 4096, rate));
                let lift = h
                    .iter()
                    .filter(|p| p.0 < 0.8 * freq)
                    .map(|p| p.1)
                    .fold(f64::MIN, f64::max);
                let reach = h
                    .iter()
                    .filter(|p| p.0 >= 1.5 * freq && p.0 <= 16_000.0)
                    .map(|p| (p.1 + range).abs())
                    .fold(0.0, f64::max);
                let tag = format!("range{range}_{freq}hz");
                r.put(
                    "S1",
                    &s.name,
                    format!("lift_db_{tag}"),
                    Some(lift),
                    Better::Lower,
                    is_split.then_some(0.2),
                    0.05,
                );
                r.put(
                    "S2",
                    &s.name,
                    format!("range_error_db_{tag}"),
                    Some(reach),
                    Better::Lower,
                    is_split.then_some(1.5),
                    0.1,
                );
            }
        }
    }

    // S3, S4, S5: the vocal line by level and side, in both detections.
    for (detect, tag) in [(0.0, "relative"), (1.0, "absolute")] {
        let subject = format!("deesser/{tag}");
        let mut s_means = Vec::new();
        for peak in [-6.0, -18.0, -30.0] {
            let (s_mean, voice) = vocal_reduction(&main, rate, detect, peak, false);
            s_means.push(s_mean);
            r.put(
                "S3",
                &subject,
                format!("s_reduction_db_{peak}dbfs"),
                Some(s_mean),
                Better::Info,
                None,
                0.0,
            );
            r.put(
                "S4",
                &subject,
                format!("voice_reduction_db_{peak}dbfs"),
                Some(voice),
                Better::Lower,
                Some(0.5),
                0.05,
            );
        }
        let relative = detect == 0.0;
        r.put(
            "S3",
            &subject,
            "s_reduction_spread_db",
            Some(spread(&s_means)),
            if relative {
                Better::Lower
            } else {
                Better::Info
            },
            relative.then_some(1.0),
            0.1,
        );
        let (one_side, _) = vocal_reduction(&main, rate, detect, -18.0, true);
        r.put(
            "S5",
            &subject,
            "one_side_error_db",
            Some((one_side - s_means[1]).abs()),
            Better::Lower,
            Some(1.0),
            0.1,
        );
    }

    // S6: zipper, at full reduction for Freq and Range.
    zippers(
        r,
        "S6",
        &main,
        &[(de::DETECT, 1.0), (de::THRESHOLD, -60.0)],
        &[(de::FREQ, None), (de::RANGE, None)],
    );
    zippers(
        r,
        "S6",
        &main,
        &[(de::DETECT, 1.0)],
        &[(de::THRESHOLD, None)],
    );
}

// ---------------------------------------------------------------------------
// Multiband compressor (plan-4 section 7).

const MULTIBAND: &str = "mpfree.fx.multiband";

/// The band compressors' knee, dB (`dynamics.rs`).
const MULTIBAND_KNEE: f64 = 6.0;

pub(super) fn multiband(r: &mut Report) {
    use ids::multiband as mb;
    let rate = r.rate;
    let (main, presets) = engine_subjects(MULTIBAND, "multiband", "multiband");
    let name = main.name.clone();
    let name = name.as_str();
    let unity: Vec<(u16, f64)> = (0..mb::BANDS).map(|b| (mb::ratio(b), 1.0)).collect();

    // X1, X2: flat with no compression, at any Mix.
    for mix in [100.0, 75.0, 50.0, 25.0] {
        let mut v = unity.clone();
        v.push((mb::MIX, mix));
        let mut d = make_with(&main, rate, &v);
        let worst = response::measure(&mut d, -20.0, 8192)
            .points
            .iter()
            .filter(|p| p.0 >= 20.0 && p.0 <= 20_000.0)
            .map(|p| p.1.abs())
            .fold(0.0, f64::max);
        if mix == 100.0 {
            r.put(
                "X1",
                name,
                "idle_error_db",
                Some(worst),
                Better::Lower,
                Some(0.1),
                0.02,
            );
        } else {
            r.put(
                "X2",
                name,
                format!("ratio1_error_db_mix{mix}"),
                Some(worst),
                Better::Lower,
                Some(0.5),
                0.1,
            );
        }
    }

    // X3: the mid band's static curve at 1 kHz against its formula: the
    // band soloed, its gain read against the same band at ratio 1 (so the
    // crossovers' own response doesn't count). The whole output against
    // the formula is reported: there the other bands' share of the tone
    // passes uncompressed.
    let mut d = make(&main, rate);
    let p = |id: u16| d.node().param(id) as f64;
    let (thr, ratio, makeup) = (
        p(mb::threshold(1)),
        p(mb::ratio(1)),
        p(mb::gain(1)) + p(mb::OUTPUT),
    );
    let drawn_gain = |i: f64| {
        let slope = 1.0 - 1.0 / ratio.max(1.0);
        let (over, h) = (i - thr, 0.5 * MULTIBAND_KNEE);
        let red = if over <= -h {
            0.0
        } else if over >= h {
            over * slope
        } else {
            slope * (over + h).powi(2) / (2.0 * MULTIBAND_KNEE)
        };
        makeup - red
    };
    let levels: Vec<f64> = (0..=10).map(|i| -40.0 + 4.0 * i as f64).collect();
    let whole = dynamics::static_curve(&mut d, &levels)
        .iter()
        .map(|(i, o)| (o - i - drawn_gain(*i)).abs())
        .fold(0.0, f64::max);
    let solo = [(mb::SOLO, 2.0)];
    let band = dynamics::static_curve(&mut make_with(&main, rate, &solo), &levels);
    let reference = dynamics::static_curve(
        &mut make_with(&main, rate, &[(mb::SOLO, 2.0), (mb::ratio(1), 1.0)]),
        &levels,
    );
    let worst = band
        .iter()
        .zip(&reference)
        .map(|((i, o), (_, o_ref))| (o - o_ref - drawn_gain(*i)).abs())
        .fold(0.0, f64::max);
    r.put(
        "X3",
        name,
        "curve_error_mid_1khz",
        Some(worst),
        Better::Lower,
        Some(0.25),
        0.05,
    );
    r.put(
        "X3",
        name,
        "curve_error_whole_output_1khz",
        Some(whole),
        Better::Info,
        None,
        0.0,
    );

    // X4, X5: attack and release against their labels (63 %), the mid
    // band soloed and its gain read against the same band at ratio 1, so
    // neither the crossovers' phase nor the step's spread into the other
    // bands counts.
    for (att, rel) in [(1.0f64, 60.0f64), (10.0, 150.0), (30.0, 500.0)] {
        let step = ((6.0 * rel / 1000.0).max(1.0) * rate as f64) as usize;
        let x = Stereo::mono(level_steps(1000.0, -40.0, -6.0, step, 3, rate), rate);
        let times = [(mb::ATTACK, att), (mb::RELEASE, rel), (mb::SOLO, 2.0)];
        let mut d = make_with(&main, rate, &times);
        let lat = d.latency();
        let y = d.render(&x, lat);
        let mut v = unity.clone();
        v.extend(times);
        let mut d = make_with(&main, rate, &v);
        let yr = d.render(&x, lat);
        let t = dynamics::timing_of(&yr.left[lat..], &y.left[lat..], step, rate);
        r.put(
            "X4",
            name,
            format!("attack_ratio_{att}ms"),
            t.attack_ms.map(|a| a / att),
            Better::Near(1.0),
            Some(0.15),
            0.05,
        );
        r.put(
            "X5",
            name,
            format!("release_ratio_{rel}ms"),
            t.release_ms.map(|a| a / rel),
            Better::Near(1.0),
            Some(0.10),
            0.03,
        );
        r.put(
            "X5",
            name,
            format!("release90_ratio_{rel}ms"),
            t.release90_ms.map(|a| a / rel),
            Better::Info,
            None,
            0.0,
        );
    }

    // X6: bass distortion 12 dB over the low band's threshold.
    for hz in [50.0, 100.0] {
        let mut d = make(&main, rate);
        let thr = d.node().param(mb::threshold(0)) as f64;
        let thd = floor(harmonics::measure(&mut d, hz, thr + 12.0).thd_db);
        r.put(
            "X6",
            name,
            format!("thd_{hz}hz_12db_over"),
            Some(thd),
            Better::Lower,
            (hz < 75.0).then_some(-50.0),
            1.0,
        );
    }
    for p in &presets {
        let mut d = make(p, rate);
        let thr = d.node().param(mb::threshold(0)) as f64;
        let thd = floor(harmonics::measure(&mut d, 50.0, (thr + 12.0).min(0.0)).thd_db);
        r.put(
            "X6",
            &p.name,
            "thd_50hz_12db_over",
            Some(thd),
            Better::Lower,
            None,
            1.0,
        );
    }

    // X7: the insert's loudness on pink at -18 dBFS: with Auto makeup on
    // it stays near the dry; off (as saved), reported.
    let pink18 = pink_rms(rate, -18.0, 6.0);
    for s in std::iter::once(&main).chain(&presets) {
        let mut d = make_with(s, rate, &[(mb::AUTO_MAKEUP, 1.0)]);
        r.put(
            "X7",
            &s.name,
            "lu_pink_auto",
            Some(lu_change(&mut d, &pink18)),
            Better::Near(0.0),
            Some(1.5),
            0.1,
        );
        let mut d = make_with(s, rate, &[(mb::AUTO_MAKEUP, 0.0)]);
        r.put(
            "X7",
            &s.name,
            "lu_pink",
            Some(lu_change(&mut d, &pink18)),
            Better::Info,
            None,
            0.0,
        );
    }

    // X8: zipper on every control.
    let mut controls: Vec<(u16, Option<(f64, f64)>)> = vec![(mb::LOW_X, None), (mb::HIGH_X, None)];
    for b in 0..mb::BANDS {
        controls.extend([
            (mb::threshold(b), None),
            (mb::ratio(b), None),
            (mb::gain(b), None),
        ]);
    }
    controls.extend([
        (mb::ATTACK, None),
        (mb::RELEASE, None),
        (mb::MIX, None),
        (mb::OUTPUT, None),
    ]);
    zippers(r, "X8", &main, &[], &controls);
}

// ---------------------------------------------------------------------------
// Ducker (plan-4 section 8).

const DUCKER: &str = "mpfree.fx.ducker";

/// A tempo duck against its drawing: the worst error (dB, a quarter of a
/// millisecond either way allowed) between frames `from` and `to`, and the
/// worst distance from full depth from each beat until its hold ends.
struct TempoCheck {
    worst: f64,
    beat: f64,
}

/// Runs `s` with `values` in tempo mode over a 200 Hz tone at -12 dBFS
/// with the song position `beats(frame)`, and checks it against its own
/// drawing at that position (frames where the tone is under half its peak
/// left out).
fn tempo_check(
    s: &Subject,
    rate: u32,
    values: &[(u16, f64)],
    beats: &dyn Fn(usize) -> f64,
    from: usize,
    to: usize,
) -> TempoCheck {
    use ids::ducker as du;
    let mut n = node_with(s, rate, values);
    let depth = n.param(du::DEPTH) as f64;
    let hold = n.param(du::HOLD) as f64;
    let rate_beats = mpfree_fx::DUCKER_RATE_BEATS[n.param(du::RATE) as usize];
    let cycle_ms = rate_beats * 500.0;
    let mut drawing = mpfree_fx::Ducker::new();
    for p in n.params() {
        if !p.readonly {
            drawing.set_param(p.id, n.param(p.id));
        }
    }
    // Twenty points a millisecond, so a quarter of a millisecond either
    // way spans ten of them and the steepest attack (0.1 ms) still has two.
    let per_ms = 20.0;
    let drawn = drawing.curve(cycle_ms as f32, (cycle_ms * per_ms) as usize + 1);
    let near = (0.25 * per_ms) as usize;
    // In cosine phase, so the beats (every 100 cycles) land on its peaks.
    let amp = db_to_amp(-12.0);
    let w = std::f64::consts::TAU * 200.0 / rate as f64;
    let x: Vec<f64> = (0..to).map(|i| amp * (w * i as f64).cos()).collect();
    let (y, _) = run(
        n.as_mut(),
        &Stereo::mono(x.clone(), rate),
        None,
        beats,
        None,
    );
    let (mut worst, mut beat) = (0.0f64, 0.0f64);
    for i in from..to {
        if x[i].abs() < 0.5 * amp {
            continue;
        }
        let g = db_amp((y.left[i] / x[i]).abs());
        let ms = beats(i).rem_euclid(rate_beats) / rate_beats * cycle_ms;
        // The distance from the gains the drawing passes through within a
        // quarter of a millisecond either way (the step wrapping round).
        let k = (ms * per_ms).round() as usize;
        let last = drawn.len() - 1;
        let (lo, hi) = (k as isize - near as isize..=(k + near) as isize)
            .map(|j| -depth * drawn[j.rem_euclid(last as isize) as usize] as f64)
            .fold((f64::MAX, f64::MIN), |(lo, hi), d| (lo.min(d), hi.max(d)));
        let err = (lo - g).max(g - hi).max(0.0);
        worst = worst.max(err);
        if ms <= hold.max(0.25) {
            beat = beat.max((g + depth).abs());
        }
    }
    TempoCheck { worst, beat }
}

pub(super) fn ducker(r: &mut Report) {
    use ids::ducker as du;
    let rate = r.rate;
    let (main, presets) = engine_subjects(DUCKER, "ducker", "ducker");
    let tempo = named(DUCKER, "ducker/tempo", &[(du::SOURCE, 1.0)]);
    // The dotted eighth and the quarter-note triplet (appended rates 6
    // and 7).
    let dotted = named(
        DUCKER,
        "ducker/tempo-dotted8",
        &[(du::SOURCE, 1.0), (du::RATE, 6.0)],
    );
    let triplet = named(
        DUCKER,
        "ducker/tempo-triplet4",
        &[(du::SOURCE, 1.0), (du::RATE, 7.0)],
    );
    let tempo_subjects: Vec<&Subject> = [&tempo, &dotted, &triplet]
        .into_iter()
        .chain(presets.iter().filter(|p| preset_sets(p, du::SOURCE, 0.5)))
        .collect();

    // U1, U2: the tempo duck down on the beat, and as drawn, over two
    // bars after the first step.
    for s in &tempo_subjects {
        let n = node_with(s, rate, &[]);
        let rate_beats = mpfree_fx::DUCKER_RATE_BEATS[n.param(du::RATE) as usize];
        let from = (rate_beats * 0.5 * rate as f64) as usize;
        let c = tempo_check(s, rate, &[], &steady(rate), from, from + 4 * rate as usize);
        r.put(
            "U1",
            &s.name,
            "beat_error_db",
            Some(c.beat),
            Better::Lower,
            Some(1.0),
            0.1,
        );
        r.put(
            "U2",
            &s.name,
            "drawing_error_db",
            Some(c.worst),
            Better::Lower,
            Some(1.0),
            0.1,
        );
    }

    // U3: the side-chain duck starts within a millisecond of the key
    // crossing the threshold.
    let hits = mixlaw::hits(4.0, rate);
    let amp = db_to_amp(-12.0);
    let x = sine(200.0, amp, hits.len(), rate);
    let xs = Stereo::mono(x.clone(), rate);
    for att in [2.0, 0.1] {
        let mut n = node_with(&main, rate, &[(du::ATTACK, att)]);
        let key = db_to_amp(n.param(du::THRESHOLD) as f64);
        let (y, _) = run(n.as_mut(), &xs, Some(&hits), &steady(rate), None);
        let every = rate as usize / 2;
        let mut delays = Vec::new();
        let mut at = every;
        while at + every < hits.len() {
            let Some(onset) = (at..at + every).find(|i| hits.left[*i].abs() > key) else {
                at += every;
                continue;
            };
            let ducked = (onset.saturating_sub(200)..at + every / 2)
                .find(|i| x[*i].abs() >= 0.5 * amp && db_amp((y.left[*i] / x[*i]).abs()) < -1.0);
            if let Some(i) = ducked {
                delays.push((i as f64 - onset as f64) * 1000.0 / rate as f64);
            }
            at += every;
        }
        r.put(
            "U3",
            &main.name,
            format!("response_ms_attack{att}ms"),
            (!delays.is_empty()).then(|| mean(&delays)),
            Better::Lower,
            Some(1.0),
            0.1,
        );
    }

    // U3: a drum key through Key LP at 120 Hz: the hats alone don't duck
    // the tone, and each kick still takes it down by Depth.
    {
        let n0 = 4 * rate as usize;
        let every = rate as usize / 2;
        let hiss = white(n0 + 1, 1.0, 9);
        let (mut hats, mut kicks) = (vec![0.0; n0], vec![0.0; n0]);
        for at in (every / 4..n0).step_by(every / 2) {
            for i in 0..(0.03 * rate as f64) as usize {
                if at + i < n0 {
                    let env = (-(i as f64) / (0.004 * rate as f64)).exp();
                    // The noise's first difference: a bright hat at about
                    // -6 dBFS.
                    hats[at + i] = 0.25 * (hiss[at + i + 1] - hiss[at + i]) * env;
                }
            }
        }
        let w = std::f64::consts::TAU * 55.0 / rate as f64;
        for at in (every / 2..n0).step_by(every) {
            for i in 0..(0.25 * rate as f64) as usize {
                if at + i < n0 {
                    let env = (-(i as f64) / (0.08 * rate as f64)).exp();
                    kicks[at + i] = 0.5 * (w * i as f64).sin() * env;
                }
            }
        }
        let tone = sine(200.0, amp, n0, rate);
        let lp = [(du::KEY_LP, 120.0)];
        let gains = |key: &[f64]| {
            let mut n = node_with(&main, rate, &lp);
            let (y, _) = run(
                n.as_mut(),
                &Stereo::mono(tone.clone(), rate),
                Some(&Stereo::mono(key.to_vec(), rate)),
                &steady(rate),
                None,
            );
            let depth = n.param(du::DEPTH) as f64;
            let g: Vec<f64> = (0..n0)
                .map(|i| {
                    if tone[i].abs() >= 0.5 * amp {
                        db_amp((y.left[i] / tone[i]).abs())
                    } else {
                        0.0
                    }
                })
                .collect();
            (g, depth)
        };
        let (g, _) = gains(&hats);
        let most = g.iter().fold(0.0f64, |a, v| a.max(-v));
        r.put(
            "U3",
            &main.name,
            "hats_duck_db_key_lp120",
            Some(most),
            Better::Lower,
            Some(1.0),
            0.1,
        );
        let both: Vec<f64> = (0..n0).map(|i| hats[i] + kicks[i]).collect();
        let (g, depth) = gains(&both);
        let worst = (every / 2..n0)
            .step_by(every)
            .filter(|at| at + every / 4 < n0)
            .map(|at| {
                let low = g[at..at + (0.03 * rate as f64) as usize]
                    .iter()
                    .fold(0.0f64, |a, v| a.min(*v));
                (low + depth).abs()
            })
            .fold(0.0f64, f64::max);
        r.put(
            "U3",
            &main.name,
            "kick_depth_error_db_key_lp120",
            Some(worst),
            Better::Lower,
            Some(1.0),
            0.1,
        );
    }

    // U4: clicks on a 60 Hz tone ducked by the beat, by attack (reported:
    // the shortest attacks click by choice). The worst of eight starting
    // phases of the tone, so where the duck's corners fall on its wave
    // (at a zero crossing they hide) doesn't decide the number.
    for att in [0.1, 0.5, 2.0, 10.0] {
        let n0 = 2 * rate as usize;
        let c = rate as usize / 2;
        let w = std::f64::consts::TAU * 60.0 / rate as f64;
        let worst = (0..8)
            .map(|k| {
                let ph = std::f64::consts::TAU * k as f64 / 8.0;
                let x: Vec<f64> = (0..n0).map(|i| amp * (w * i as f64 + ph).sin()).collect();
                let mut n = node_with(&tempo, rate, &[(du::ATTACK, att)]);
                let (y, _) = run(
                    n.as_mut(),
                    &Stereo::mono(x, rate),
                    None,
                    &steady(rate),
                    None,
                );
                rel_d2(&y.left, 60.0, rate, c - 3000, c + 3000)
            })
            .fold(f64::MIN, f64::max);
        r.put(
            "U4",
            &tempo.name,
            format!("click_db_attack{att}ms"),
            Some(worst),
            Better::Info,
            None,
            0.0,
        );
    }

    // U5: in step with the song position 50 ms after it jumps (to a point
    // inside a step) or a loop wraps (to the start).
    let jump = 690 * BLOCK;
    for (tag, to_beats) in [("jump", 0.37), ("wrap", 0.0)] {
        let beats = move |f: usize| {
            let rate = rate as f64;
            if f < jump {
                f as f64 / rate * 2.0
            } else {
                to_beats + (f - jump) as f64 / rate * 2.0
            }
        };
        let from = jump + rate as usize / 20;
        let c = tempo_check(&tempo, rate, &[], &beats, from, jump + rate as usize);
        r.put(
            "U5",
            &tempo.name,
            format!("drawing_error_db_after_{tag}"),
            Some(c.worst),
            Better::Lower,
            Some(1.0),
            0.1,
        );
    }

    // U6: zipper, while a long tempo duck holds and then releases.
    zippers(
        r,
        "U6",
        &tempo,
        &[
            (du::RATE, 4.0),
            (du::ATTACK, 2.0),
            (du::HOLD, 500.0),
            (du::RELEASE, 2000.0),
        ],
        &[
            (du::DEPTH, None),
            (du::SHAPE, None),
            (du::MIX, None),
            (du::RELEASE, Some((500.0, 2000.0))),
        ],
    );
}

// ---------------------------------------------------------------------------
// Transient shaper (plan-4 section 9).

const TRANSIENT: &str = "mpfree.fx.transient";

/// The mean gain of each hit's front (the peak of its first 5 ms, out
/// against in, dB), latency removed.
fn front_gain(d: &mut dyn Dut, hits: &Stereo) -> f64 {
    let rate = hits.rate;
    let lat = d.latency();
    let y = d.render(hits, lat);
    let every = rate as usize / 2;
    let m = (0.005 * rate as f64) as usize;
    let mut g = Vec::new();
    let mut at = 0;
    while at + every < hits.len() {
        let pin = hits.left[at..at + m]
            .iter()
            .fold(0.0f64, |a, v| a.max(v.abs()));
        let pout = y.left[at + lat..at + lat + m]
            .iter()
            .fold(0.0f64, |a, v| a.max(v.abs()));
        g.push(db_amp(pout / pin));
        at += every;
    }
    mean(&g)
}

pub(super) fn transient(r: &mut Report) {
    use ids::transient as tr;
    let rate = r.rate;
    let (main, presets) = engine_subjects(TRANSIENT, "transient", "transient");
    let name = main.name.clone();
    let name = name.as_str();

    // T1: the same front gain at any level.
    let hits = mixlaw::hits(4.0, rate);
    for (tag, attack, target) in [
        ("attack50", 50.0, true),
        ("attack100", 100.0, true),
        ("attack-100", -100.0, false),
    ] {
        let fronts: Vec<f64> = [0.0, -12.0, -24.0, -36.0]
            .iter()
            .map(|lvl| {
                let mut d = make_with(&main, rate, &[(tr::ATTACK, attack)]);
                front_gain(&mut d, &scaled(&hits, *lvl))
            })
            .collect();
        r.put(
            "T1",
            name,
            format!("front_spread_db_{tag}"),
            Some(spread(&fronts)),
            Better::Lower,
            target.then_some(1.0),
            0.1,
        );
        for (lvl, f) in [("loud", fronts[0]), ("quiet", fronts[3])] {
            r.put(
                "T1",
                name,
                format!("front_gain_db_{tag}_{lvl}"),
                Some(f),
                Better::Info,
                None,
                0.0,
            );
        }
    }

    // T2: steady tones untouched.
    for (tag, values) in [
        ("attack100", [(tr::ATTACK, 100.0), (tr::SUSTAIN, 0.0)]),
        ("sustain100", [(tr::ATTACK, 0.0), (tr::SUSTAIN, 100.0)]),
        ("sustain-100", [(tr::ATTACK, 0.0), (tr::SUSTAIN, -100.0)]),
    ] {
        let mut worst = 0.0f64;
        for hz in [30.0, 40.0, 100.0, 1000.0] {
            let n = 2 * rate as usize;
            let x = sine(hz, db_to_amp(-12.0), n, rate);
            let mut d = make_with(&main, rate, &values);
            let lat = d.latency();
            let y = d.render(&Stereo::mono(x.clone(), rate), lat);
            let a = rate as usize;
            let g: Vec<f64> = half_cycle_gains(&x[a..], &y.left[a + lat..], hz, rate)
                .iter()
                .map(|p| p.1)
                .collect();
            worst = worst.max(spread(&g));
        }
        r.put(
            "T2",
            name,
            format!("ripple_db_{tag}"),
            Some(worst),
            Better::Lower,
            Some(0.5),
            0.05,
        );
    }

    // T3: an insert at the defaults and each preset: hits' peaks and
    // loudness (reported; decision 11.4).
    let hits6 = mixlaw::hits(6.0, rate);
    for s in std::iter::once(&main).chain(&presets) {
        let mut d = make(s, rate);
        let lat = d.latency();
        let y = d.render(&hits6, lat);
        let peak = loudness::sample_peak(&y) - loudness::sample_peak(&hits6);
        r.put(
            "T3",
            &s.name,
            "hits_peak_change_db",
            Some(peak),
            Better::Info,
            None,
            0.0,
        );
        r.put(
            "T3",
            &s.name,
            "hits_lu",
            Some(lu_change(&mut d, &hits6)),
            Better::Info,
            None,
            0.0,
        );
    }

    // T4: zipper.
    zippers(
        r,
        "T4",
        &main,
        &[],
        &[
            (tr::ATTACK, None),
            (tr::SUSTAIN, None),
            (tr::SPEED, None),
            (tr::OUTPUT, None),
            (tr::MIX, None),
        ],
    );
}

// ---------------------------------------------------------------------------
// Channel strip (plan-4 section 9).

const STRIP: &str = "mpfree.fx.channel_strip";

pub(super) fn strip(r: &mut Report) {
    use ids::channel_strip as cs;
    use ids::compressor as k;
    let rate = r.rate;
    let (main, presets) = engine_subjects(STRIP, "strip", "strip");
    let name = main.name.clone();
    let name = name.as_str();

    // C1: its gate alone (as N1 to N3) ...
    let gate_only = [(cs::GATE, 1.0), (cs::COMP, 0.0), (cs::EQ, 0.0)];
    let hits = mixlaw::hits(4.0, rate);
    let mut d = make_with(&main, rate, &gate_only);
    r.put(
        "C1",
        name,
        "gate_onset_1ms_db",
        Some(onset_db(&mut d, &hits, 1.0).abs()),
        Better::Lower,
        Some(0.5),
        0.1,
    );
    let thr = d.node().param(cs::GATE_THRESHOLD) as f64;
    for hz in [40.0, 1000.0] {
        let mut d = make_with(&main, rate, &gate_only);
        let (ripple, shut) = falls_through(&mut d, thr, hz);
        r.put(
            "C1",
            name,
            format!("gate_ripple_db_{hz}hz"),
            Some(ripple),
            Better::Lower,
            Some(0.5),
            0.1,
        );
        r.put(
            "C1",
            name,
            format!("gate_shut_error_db_{hz}hz"),
            shut.map(|s| (s - (thr - 4.0)).abs()),
            Better::Lower,
            Some(1.0),
            0.2,
        );
    }

    // ... and its compressor alone (as K1, K2, K3 and K4), against the
    // compressor engine set as the strip sets it.
    let comp_only = [(cs::GATE, 0.0), (cs::COMP, 1.0), (cs::EQ, 0.0)];
    let mut d = make_with(&main, rate, &comp_only);
    let mut c = mpfree_fx::Compressor::new();
    for (from, to) in [
        (cs::COMP_THRESHOLD, k::THRESHOLD),
        (cs::RATIO, k::RATIO),
        (cs::COMP_ATTACK, k::ATTACK),
        (cs::COMP_RELEASE, k::RELEASE),
        (cs::MAKEUP, k::MAKEUP),
    ] {
        c.set_param(to, d.node().param(from));
    }
    let levels: Vec<f64> = (0..=13).map(|i| -39.0 + 3.0 * i as f64).collect();
    let worst = dynamics::static_curve(&mut d, &levels)
        .iter()
        .map(|(i, o)| (o - c.curve_db(*i as f32) as f64).abs())
        .fold(0.0, f64::max);
    r.put(
        "C1",
        name,
        "comp_curve_error",
        Some(worst),
        Better::Lower,
        Some(0.25),
        0.05,
    );
    let (att, rel) = (
        d.node().param(cs::COMP_ATTACK) as f64,
        d.node().param(cs::COMP_RELEASE) as f64,
    );
    let t = dynamics::timing(&mut d, -40.0, 0.0, (rel * 8.0 / 1000.0).max(0.5));
    r.put(
        "C1",
        name,
        "comp_attack_ratio",
        t.attack_ms.map(|a| a / att),
        Better::Near(1.0),
        Some(0.15),
        0.05,
    );
    r.put(
        "C1",
        name,
        "comp_release_ratio",
        t.release_ms.map(|a| a / rel),
        Better::Near(1.0),
        Some(0.10),
        0.03,
    );
    let mut v = comp_only.to_vec();
    v.push((cs::COMP_THRESHOLD, -20.0));
    let mut d = make_with(&main, rate, &v);
    let thd = floor(harmonics::measure(&mut d, 50.0, -7.0).thd_db);
    r.put(
        "C1",
        name,
        "comp_thd_50hz_13db_over",
        Some(thd),
        Better::Lower,
        Some(-50.0),
        1.0,
    );

    // C2: the insert's loudness on pink at -18 dBFS: with Auto makeup on
    // it stays near the dry; off (as saved before it was on the page),
    // reported.
    let pink18 = pink_rms(rate, -18.0, 6.0);
    for s in std::iter::once(&main).chain(&presets) {
        let mut d = make_with(s, rate, &[(cs::AUTO_MAKEUP, 1.0)]);
        r.put(
            "C2",
            &s.name,
            "lu_pink_auto",
            Some(lu_change(&mut d, &pink18)),
            Better::Near(0.0),
            Some(1.5),
            0.1,
        );
        let mut d = make_with(s, rate, &[(cs::AUTO_MAKEUP, 0.0)]);
        r.put(
            "C2",
            &s.name,
            "lu_pink",
            Some(lu_change(&mut d, &pink18)),
            Better::Info,
            None,
            0.0,
        );
    }

    // C3: a stage switched while a tone plays doesn't click.
    let eq_on = [(cs::MID_FREQ, 220.0), (cs::MID_GAIN, 6.0)];
    for (tag, values, id, to) in [
        ("gate_on", &[][..], cs::GATE, 1.0),
        ("comp_off", &[][..], cs::COMP, 0.0),
        ("eq_off", &eq_on[..], cs::EQ, 0.0),
        ("order", &eq_on[..], cs::ORDER, 1.0),
    ] {
        let mut d = make_with(&main, rate, values);
        r.put(
            "C3",
            name,
            format!("switch_click_db_{tag}"),
            Some(switch_click(&mut d, id, to, 220.0)),
            Better::Lower,
            Some(3.0),
            0.5,
        );
    }

    // C4: the latency is the same whatever is on.
    let mut settings: Vec<Vec<(u16, f64)>> = Vec::new();
    for bits in 0..8 {
        let on = |b: usize| f64::from(u8::from(bits & (1 << b) != 0));
        settings.push(vec![(cs::GATE, on(0)), (cs::COMP, on(1)), (cs::EQ, on(2))]);
    }
    settings.push(vec![(cs::ORDER, 1.0)]);
    let refs: Vec<&[(u16, f64)]> = settings.iter().map(|v| v.as_slice()).collect();
    r.put(
        "C4",
        name,
        "latency_frames",
        Some(make(&main, rate).latency() as f64),
        Better::Info,
        None,
        0.0,
    );
    r.put(
        "C4",
        name,
        "latency_spread_frames",
        Some(latency_spread(&main, rate, &refs)),
        Better::Lower,
        Some(0.0),
        0.0,
    );
}
