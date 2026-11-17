//! The second round's classes of the quality report: drive (Drive and
//! Character), crush, compressor, filters (the Multi and DJ filters) and
//! lo-fi. The plan's target ids are G, B, K, F and L; the targets and why
//! each was chosen are in `docs/dev/design/fx-quality.md`. The rows, the
//! report and the ratchet are the first round's (`super`); the measures
//! only these classes need live here.

use mpfree_fx::ids;
use mpfree_fx::mpfree_node::Node;

use super::{floor, lu_change, make, pink_stim, subjects, worst_zipper, Better, Report, Subject};
use crate::dut::{Change, Dut, NodeDut};
use crate::fft::power_spectrum;
use crate::harmonics::{self, N};
use crate::signals::{coherent, db_to_amp, level_steps, peak_of, pink, rms_of, sine};
use crate::wav::Stereo;
use crate::{dynamics, flutter, mixlaw, response};

// ---------------------------------------------------------------------------
// Shared measures.

fn db_pow(p: f64) -> f64 {
    10.0 * p.max(1e-30).log10()
}

fn db_amp(a: f64) -> f64 {
    20.0 * a.max(1e-15).log10()
}

/// A single engine (not a family) at its defaults, then at each of its
/// factory presets: `class/name` and `class/name[Preset]`.
fn engine_subjects(id: &'static str, class: &str, name: &str) -> (Subject, Vec<Subject>) {
    let s = |preset: Option<&'static mpfree_fx::Preset>| Subject {
        name: match preset {
            None => format!("{class}/{name}"),
            Some(p) => format!("{class}/{name}[{}]", p.name),
        },
        id,
        fixed: &[],
        preset,
    };
    let presets = mpfree_fx::presets(id).iter().map(|p| s(Some(p))).collect();
    (s(None), presets)
}

/// `s` with `values` set on top of its preset and fixed values.
fn make_with(s: &Subject, rate: u32, values: &[(u16, f64)]) -> NodeDut {
    let mut d = make(s, rate);
    for (id, v) in values {
        d.set(*id, *v);
    }
    d
}

/// The last word of a subject's name: its mode ("drive/soft" is "soft").
fn mode_of(s: &Subject) -> &str {
    let n = s.name.split('[').next().unwrap_or("");
    n.rsplit('/').next().unwrap_or("")
}

/// Worst alias of a `hz` sine at `level` dBFS: dB against the fundamental,
/// and dBFS.
fn alias(d: &mut dyn Dut, hz: f64, level: f64) -> (f64, f64) {
    let h = harmonics::measure(d, hz, level);
    (floor(h.alias_worst_db), floor(h.alias_worst_db + h.out_db))
}

/// The `N` analysed frames of a coherent sine near `hz` at `level` dBFS
/// through `d` (half a second to settle first), and its bin.
fn sine_out(d: &mut dyn Dut, hz: f64, level: f64) -> (Vec<f64>, usize) {
    let settle = d.rate() as usize / 2;
    let (stim, bin, settle) = harmonics::stimulus(hz, level, settle, d.rate());
    let lat = d.latency();
    let out = d.render(&stim, lat);
    (out.left[settle + lat..settle + lat + N].to_vec(), bin)
}

/// The amplitude of the `hz` component of `x` (a lock-in over whole
/// periods).
fn amplitude_at(x: &[f64], hz: f64, rate: u32) -> f64 {
    let per = rate as f64 / hz;
    let periods = (x.len() as f64 / per).floor().max(1.0);
    let n = ((periods * per).round() as usize).min(x.len());
    let w = 2.0 * std::f64::consts::PI * hz / rate as f64;
    let (mut c, mut s) = (0.0, 0.0);
    for (i, v) in x[x.len() - n..].iter().enumerate() {
        c += v * (w * i as f64).cos();
        s += v * (w * i as f64).sin();
    }
    2.0 * (c * c + s * s).sqrt() / n as f64
}

/// The gain of `d` at `hz` for a sine at `level` dBFS, dB, from the last
/// tenth of a second of 0.4 s.
fn gain_at(d: &mut dyn Dut, hz: f64, level: f64) -> f64 {
    let rate = d.rate();
    let n = (0.4 * rate as f64) as usize;
    let w = rate as usize / 10;
    let x = sine(hz, db_to_amp(level), n, rate);
    let lat = d.latency();
    let y = d.render(&Stereo::mono(x.clone(), rate), lat).left;
    db_amp(amplitude_at(&y[n + lat - w..n + lat], hz, rate) / amplitude_at(&x[n - w..], hz, rate))
}

/// Loudness change on `x`, LU, after `x` is rendered through `d`.
fn lu(d: &mut NodeDut, x: &Stereo) -> f64 {
    lu_change(d, x)
}

fn pink_at(rate: u32, rms_db: f64, secs: f64) -> Stereo {
    Stereo::mono(
        pink((secs * rate as f64) as usize, db_to_amp(rms_db), 7),
        rate,
    )
}

/// The largest second difference of `y[from..to]` against a clean sine's
/// at the local level (its RMS over two periods), dB: 0 for a clean sine at
/// any level, a click spikes. Levels under -80 dBFS count as -80.
fn rel_d2(y: &[f64], hz: f64, rate: u32, from: usize, to: usize) -> f64 {
    let w = 2.0 * std::f64::consts::PI * hz / rate as f64;
    let k = 2.0 * (1.0 - w.cos());
    let win = (2.0 * rate as f64 / hz) as usize;
    let mut sq = vec![0.0; y.len() + 1];
    for i in 0..y.len() {
        sq[i + 1] = sq[i] + y[i] * y[i];
    }
    let mut worst = 0.0f64;
    for i in from.max(win + 1)..to.min(y.len().saturating_sub(win + 1)) {
        let d2 = (y[i + 1] - 2.0 * y[i] + y[i - 1]).abs();
        let env =
            ((sq[i + win] - sq[i - win]) / (2 * win) as f64).sqrt() * std::f64::consts::SQRT_2;
        worst = worst.max(d2 / (k * env).max(1e-4 * k));
    }
    db_amp(worst)
}

/// The slope zipper: sweeps `id` from `lo` to `hi` and back, `ms` each way,
/// under a 997 Hz sine at -12 dBFS; the worst [`rel_d2`] while moving. For
/// linear effects whose ends attenuate the tone, where fxlab's usual
/// zipper (against the resting ends) reads high.
#[allow(clippy::too_many_arguments)]
fn slope_zipper(d: &mut NodeDut, id: u16, lo: f64, hi: f64, log: bool, ms: f64) -> f64 {
    let rate = d.rate();
    let hz = 997.0;
    let rest = (0.3 * rate as f64) as usize;
    let sweep = (ms / 1000.0 * rate as f64) as usize;
    let x = sine(hz, db_to_amp(-12.0), 3 * rest + 2 * sweep, rate);
    let at = |t: f64| {
        if log {
            (lo.ln() + t * (hi.ln() - lo.ln())).exp()
        } else {
            lo + t * (hi - lo)
        }
    };
    let steps = (sweep / 32).max(1);
    let mut ch = Vec::new();
    for k in 1..=steps {
        let t = k as f64 / steps as f64;
        ch.push(Change {
            frame: rest + k * sweep / steps - 1,
            id,
            value: at(t),
        });
        ch.push(Change {
            frame: 2 * rest + sweep + k * sweep / steps - 1,
            id,
            value: at(1.0 - t),
        });
    }
    ch.sort_by_key(|c| c.frame);
    d.set(id, lo);
    d.reset();
    let y = d.process(&Stereo::mono(x, rate), &ch).left;
    let g = rate as usize / 20;
    rel_d2(&y, hz, rate, rest, rest + sweep + g).max(rel_d2(
        &y,
        hz,
        rate,
        2 * rest + sweep,
        2 * rest + 2 * sweep + g,
    ))
}

/// The spike when control `id` jumps to `to` halfway through a second of a
/// sine at `hz`, -12 dBFS: the largest [`rel_d2`] around the change over
/// the larger of the steady signal's before and after it, dB, at least 0.
fn switch_click(d: &mut NodeDut, id: u16, to: f64, hz: f64) -> f64 {
    let rate = d.rate();
    let n = rate as usize;
    let x = sine(hz, db_to_amp(-12.0), n, rate);
    d.reset();
    let y = d
        .process(
            &Stereo::mono(x, rate),
            &[Change {
                frame: n / 2,
                id,
                value: to,
            }],
        )
        .left;
    let before = rel_d2(&y, hz, rate, n / 4, n / 2 - 200);
    let after = rel_d2(&y, hz, rate, n / 2 + 8000, 3 * n / 4);
    let at = rel_d2(&y, hz, rate, n / 2 - 100, n / 2 + 4000);
    (at - before.max(after)).max(0.0)
}

/// The largest difference between `d`'s output and its input (delayed by
/// its latency) on `x`, dBFS; -140 when they are the same. Nodes run in
/// 32-bit floats, so the input is compared as the node saw it.
fn null_dbfs(d: &mut NodeDut, x: &Stereo) -> f64 {
    let lat = d.latency();
    let y = d.render(x, lat);
    let f32_of = |v: f64| f64::from(v as f32);
    let mut worst = 0.0f64;
    for i in 0..x.len() {
        worst = worst
            .max((y.left[i + lat] - f32_of(x.left[i])).abs())
            .max((y.right[i + lat] - f32_of(x.right[i])).abs());
    }
    floor(db_amp(worst))
}

/// SMPTE IMD (60 Hz at -8 dBFS with 7 kHz at -20 dBFS: the sidebands at
/// ±60, ±120 and ±180 Hz against the 7 kHz line) and the CCIF difference
/// tone (19 and 20 kHz at -12 dBFS each: 1 kHz against one tone), dB.
fn imd(d: &mut NodeDut) -> (f64, f64) {
    let rate = d.rate();
    let settle = rate as usize / 2;
    let total = settle + N;
    let (b1, f1) = coherent(60.0, N, rate);
    let (b2, f2) = coherent(7000.0, N, rate);
    let x: Vec<f64> = sine(f1, db_to_amp(-8.0), total, rate)
        .iter()
        .zip(sine(f2, db_to_amp(-20.0), total, rate))
        .map(|(a, b)| a + b)
        .collect();
    let lat = d.latency();
    let y = d.render(&Stereo::mono(x, rate), lat).left;
    let p = power_spectrum(&y[settle + lat..settle + lat + N]);
    let side: f64 = (1..=3).map(|k| p[b2 + k * b1] + p[b2 - k * b1]).sum();
    let smpte = db_pow(side / p[b2]);
    let bin = |hz: f64| (hz * N as f64 / rate as f64).round() as usize;
    let (ba, bd) = (bin(19_000.0), bin(1000.0));
    let hz = |b: usize| b as f64 * rate as f64 / N as f64;
    let x: Vec<f64> = sine(hz(ba), db_to_amp(-12.0), total, rate)
        .iter()
        .zip(sine(hz(ba + bd), db_to_amp(-12.0), total, rate))
        .map(|(a, b)| a + b)
        .collect();
    let y = d.render(&Stereo::mono(x, rate), lat).left;
    let p = power_spectrum(&y[settle + lat..settle + lat + N]);
    (floor(smpte), floor(db_pow(p[bd] / p[ba])))
}

// ---------------------------------------------------------------------------
// Drive (plan-2 section 4).

const DRIVE: &str = "mpfree.fx.drive";

pub(super) fn drive(r: &mut Report) {
    use ids::character as ch;
    use ids::drive as dr;
    let rate = r.rate;
    let (defaults, presets) = subjects("Drive", "drive");
    let p18 = pink_at(rate, -18.0, 4.0);
    let p6 = pink_at(rate, -6.0, 4.0);
    let hits = mixlaw::hits(4.0, rate);
    for s in &defaults {
        let name = s.name.as_str();
        let mode = mode_of(s);
        let is_drive = s.id == DRIVE;
        let hard = matches!(mode, "hard" | "fuzz" | "foldback" | "rectify");
        let rectify = mode == "rectify";
        let gid = if hard { "G2" } else { "G1" };

        // G1, G2: aliasing, in each quality.
        let qualities: &[(f64, &str)] = if is_drive {
            &[(0.0, "_eco"), (1.0, "_hq")]
        } else {
            &[(0.0, "")]
        };
        for (q, sfx) in qualities {
            let mut d = if is_drive {
                make_with(s, rate, &[(dr::QUALITY, *q)])
            } else {
                make(s, rate)
            };
            let target = match (is_drive, hard, *sfx) {
                (false, _, _) => matches!(mode, "console" | "tape").then_some(-65.0),
                (true, false, "_eco") => Some(-65.0),
                (true, false, _) => Some(-90.0),
                (true, true, "_eco") => Some(-50.0),
                (true, true, _) => Some(-60.0),
            };
            for (hz, level, tag) in [
                (2500.0, 0.0, "2k5_0dbfs"),
                (5000.0, 0.0, "5k_0dbfs"),
                (8000.0, 0.0, "8k_0dbfs"),
                (5000.0, -12.0, "5k_m12dbfs"),
            ] {
                let (rel, fs) = alias(&mut d, hz, level);
                let t = if tag == "5k_0dbfs" { target } else { None };
                // Rectify takes the fundamental away: read it in dBFS.
                let (metric, v) = if rectify {
                    (format!("alias_dbfs_{tag}{sfx}"), fs)
                } else {
                    (format!("alias_{tag}{sfx}"), rel)
                };
                r.put(gid, name, metric, Some(v), Better::Lower, t, 1.0);
            }
        }

        // G4, G5: loudness against the dry across Drive (Amount).
        let (knob, steps): (u16, &[f64]) = if is_drive {
            (dr::DRIVE, &[0.0, 12.0, 24.0, 36.0])
        } else {
            (ch::AMOUNT, &[0.0, 50.0, 100.0])
        };
        let tol_lu = if rectify { 3.0 } else { 1.0 };
        for v in steps {
            let mut d = make_with(s, rate, &[(knob, *v)]);
            let k = if is_drive { "drive" } else { "amount" };
            r.put(
                "G4",
                name,
                format!("lu_pink18_{k}{v}"),
                Some(lu(&mut d, &p18)),
                Better::Near(0.0),
                Some(tol_lu),
                0.3,
            );
            if *v > 0.0 && (*v == steps[1] || *v == steps[steps.len() - 1]) {
                r.put(
                    "G5",
                    name,
                    format!("lu_pink6_{k}{v}"),
                    Some(lu(&mut d, &p6)),
                    Better::Info,
                    None,
                    0.0,
                );
                r.put(
                    "G5",
                    name,
                    format!("lu_hits_{k}{v}"),
                    Some(lu(&mut d, &hits)),
                    Better::Info,
                    None,
                    0.0,
                );
            }
        }

        // G6: Bias, as the change it makes to a 1 kHz sine at -12 dBFS
        // (the difference between Bias 0 and Bias 100, against the output).
        if is_drive {
            let mut outs = Vec::new();
            for b in [0.0, 100.0] {
                let mut d = make_with(s, rate, &[(dr::BIAS, b)]);
                outs.push(sine_out(&mut d, 1000.0, -12.0).0);
            }
            let diff: f64 = outs[0]
                .iter()
                .zip(&outs[1])
                .map(|(a, b)| (a - b) * (a - b))
                .sum();
            let own: f64 = outs[0].iter().map(|a| a * a).sum();
            // Soft with Bias would be Tube: its knob is dimmed instead
            // (Jay, 17 Nov), so it has no target.
            let target = (mode != "soft").then_some(-20.0);
            r.put(
                "G6",
                name,
                "bias_change_db",
                Some(floor(db_pow(diff / own))),
                Better::Higher,
                target,
                1.0,
            );
        }

        // G8: clean when quiet; and Mix 50 % at no drive. Rectify folds
        // even a quiet sine, so it has no small-signal response.
        for (q, sfx) in qualities.iter().filter(|_| !rectify) {
            let mut d = if is_drive {
                make_with(s, rate, &[(dr::QUALITY, *q)])
            } else {
                make(s, rate)
            };
            // Tape's head bump and top loss are its colour: held, not
            // aimed at flat.
            r.put(
                "G8",
                name,
                format!("flat_m40dbfs{sfx}"),
                Some(flatness(&mut d)),
                Better::Lower,
                (mode != "tape").then_some(0.5),
                0.05,
            );
        }
        let mix = if is_drive {
            make_with(s, rate, &[(dr::DRIVE, 0.0), (dr::MIX, 50.0)])
        } else {
            make_with(s, rate, &[(ch::AMOUNT, 0.0), (ch::MIX, 50.0)])
        };
        let mut mix = mix;
        r.put(
            "G8",
            name,
            "flat_mix50",
            Some(flatness(&mut mix)),
            if rectify { Better::Info } else { Better::Lower },
            (!rectify).then_some(0.5),
            0.05,
        );

        // G10: the harmonic profile, touch and intermodulation (reported).
        let mut d = make(s, rate);
        let h = harmonics::measure(&mut d, 1000.0, -12.0);
        let hp = |k: usize| h.h[k - 2].map_or(0.0, |x| 10f64.powf(x / 10.0));
        let lo: f64 = (2..=5).map(hp).sum();
        let hi: f64 = (6..=9).map(hp).sum();
        let h2 = floor(h.h[0].unwrap_or(-300.0));
        let h3 = floor(h.h[1].unwrap_or(-300.0));
        r.put(
            "G10",
            name,
            "h2_minus_h3",
            Some(h2 - h3),
            Better::Info,
            None,
            0.0,
        );
        r.put(
            "G10",
            name,
            "high_order_share",
            Some(floor(db_pow(hi / lo.max(1e-30)))),
            Better::Info,
            None,
            0.0,
        );
        for level in [-36.0, -12.0, 0.0] {
            let t = harmonics::measure(&mut d, 1000.0, level).thd_db;
            r.put(
                "G10",
                name,
                format!("thd_1k_{}dbfs", level as i32),
                Some(floor(t)),
                Better::Info,
                None,
                0.0,
            );
        }
        let (smpte, ccif) = imd(&mut d);
        r.put(
            "G10",
            name,
            "imd_smpte",
            Some(smpte),
            Better::Info,
            None,
            0.0,
        );
        r.put("G10", name, "imd_ccif", Some(ccif), Better::Info, None, 0.0);
        r.put(
            "G12",
            name,
            "latency_frames",
            Some(d.latency() as f64),
            Better::Info,
            None,
            0.0,
        );

        // G11: zipper.
        r.put(
            "G11",
            name,
            "zipper_db",
            worst_zipper(s, rate),
            Better::Lower,
            Some(3.0),
            0.5,
        );
    }

    // G7: a Mode or Quality change under a 100 Hz sine.
    let soft = &defaults[0];
    for (from, to, metric) in [
        (0.0, 1.0, "click_soft_to_hard"),
        (0.0, 3.0, "click_soft_to_foldback"),
        (0.0, 4.0, "click_soft_to_fuzz"),
        (2.0, 5.0, "click_tube_to_rectify"),
    ] {
        let mut d = make_with(soft, rate, &[(dr::MODE, from)]);
        r.put(
            "G7",
            "drive/switch",
            metric,
            Some(switch_click(&mut d, dr::MODE, to, 100.0)),
            Better::Lower,
            Some(3.0),
            1.0,
        );
    }
    let mut d = make_with(soft, rate, &[(dr::QUALITY, 0.0)]);
    r.put(
        "G7",
        "drive/switch",
        "click_eco_to_hq",
        Some(switch_click(&mut d, dr::QUALITY, 1.0, 100.0)),
        Better::Lower,
        Some(3.0),
        1.0,
    );
    if let Some(console) = defaults.iter().find(|s| mode_of(s) == "console") {
        let mut d = make(console, rate);
        r.put(
            "G7",
            "drive/switch",
            "click_console_to_tape",
            Some(switch_click(&mut d, ch::MODE, ch::TAPE as f64, 100.0)),
            Better::Lower,
            Some(3.0),
            1.0,
        );
    }

    // G9: the low end of asymmetric curves on drum hits.
    let hits6 = mixlaw::hits(6.0, rate);
    for s in defaults
        .iter()
        .filter(|s| matches!(mode_of(s), "tube" | "fuzz"))
    {
        let mut d = make_with(s, rate, &[(dr::BIAS, 100.0), (dr::DRIVE, 24.0)]);
        let lat = d.latency();
        let out = d.render(&hits6, lat);
        let start = rate as usize + lat;
        let p = power_spectrum(&out.left[start..start + N]);
        let edge = (15.0 * N as f64 / rate as f64) as usize;
        let sub: f64 = p[1..=edge].iter().sum();
        let all: f64 = p[1..].iter().sum();
        r.put(
            "G9",
            &s.name,
            "subsonic_hits_bias100",
            Some(floor(db_pow(sub / all))),
            Better::Lower,
            Some(-20.0),
            1.0,
        );
        r.put(
            "G9",
            &s.name,
            "dc_hits_bias100_dbfs",
            Some(floor(db_pow(p[0]) + 6.0)),
            Better::Lower,
            Some(-100.0),
            3.0,
        );
    }

    // G3, G11: presets.
    for s in &presets {
        let mut d = make(s, rate);
        let (rel, fs) = alias(&mut d, 5000.0, 0.0);
        let (metric, v) = if mode_of(s) == "rectify" {
            ("alias_dbfs_5k_0dbfs", fs)
        } else {
            ("alias_5k_0dbfs", rel)
        };
        r.put("G3", &s.name, metric, Some(v), Better::Lower, None, 1.0);
        r.put(
            "G4",
            &s.name,
            "lu_pink18",
            Some(lu(&mut d, &p18)),
            Better::Info,
            None,
            0.0,
        );
        r.put(
            "G11",
            &s.name,
            "zipper_db",
            worst_zipper(s, rate),
            Better::Lower,
            Some(3.0),
            0.5,
        );
    }
}

/// The worst deviation of the response from its own 1 kHz gain, 20 Hz to
/// 20 kHz, for a sweep at -40 dBFS.
fn flatness(d: &mut NodeDut) -> f64 {
    let resp = response::measure(d, -40.0, 8192);
    let at1k = resp.at(1000.0).unwrap_or(0.0);
    resp.worst_error(20.0, 20_000.0, |_| at1k)
}

// ---------------------------------------------------------------------------
// Crush (plan-2 section 5).

const CRUSH: &str = "mpfree.fx.crush";

/// A 4-term Blackman-Harris window (sidelobes under -92 dB): the crush
/// rates that don't divide the session's put lines between bins, and a
/// plain spectrum's leakage would read as stray energy.
fn blackman_harris(n: usize) -> Vec<f64> {
    let a = [0.35875, 0.48829, 0.14128, 0.01168];
    (0..n)
        .map(|i| {
            let x = std::f64::consts::TAU * i as f64 / n as f64;
            a[0] - a[1] * x.cos() + a[2] * (2.0 * x).cos() - a[3] * (3.0 * x).cos()
        })
        .collect()
}

/// What a sample and hold at the crush rate R should make, against what
/// it does: a sine at -6 dBFS (3 kHz, or a quarter of R if that is lower)
/// through `d` (16 bits, the rate it is set to). The energy that is not
/// at k R ± f below the session's Nyquist (stray tones from the session's
/// frame grid, and images from above its Nyquist folded back; neither is
/// in the machine being imitated), dB against the tone, in all and its
/// loudest line.
fn off_grid(d: &mut NodeDut) -> (f64, f64) {
    let host = d.rate() as f64;
    let r_hz = d.node().param(ids::crush::RATE) as f64;
    let (y, bin) = sine_out(d, (r_hz / 4.0).min(3000.0), -6.0);
    let w = blackman_harris(y.len());
    let yw: Vec<f64> = y.iter().zip(&w).map(|(a, b)| a * b).collect();
    let p = power_spectrum(&yw);
    let df = host / N as f64;
    let f0 = bin as f64 * df;
    // The window spreads a line over about four bins either side.
    const SPREAD: usize = 5;
    let bins = p.len();
    let mut expected = vec![false; bins];
    let mut m = 0usize;
    while m as f64 * r_hz - f0 <= host / 2.0 {
        for g in [m as f64 * r_hz - f0, m as f64 * r_hz + f0] {
            let g = g.abs();
            if g <= host / 2.0 {
                let k = (g / df).round() as usize;
                for e in &mut expected[k.saturating_sub(SPREAD)..(k + SPREAD + 1).min(bins)] {
                    *e = true;
                }
            }
        }
        m += 1;
    }
    let tone: f64 = p[bin - SPREAD..=bin + SPREAD].iter().sum();
    let low = (20.0 / df) as usize;
    let (mut off, mut worst) = (0.0, 0.0f64);
    for (k, pk) in p.iter().enumerate().skip(low) {
        if !expected[k] {
            off += pk;
            worst = worst.max(*pk);
        }
    }
    (floor(db_pow(off / tone)), floor(db_pow(worst / p[bin])))
}

/// Noise and distortion against a full-scale sine's fundamental, dB.
fn sinad(y: &[f64], bin: usize, rate: u32) -> f64 {
    let p = power_spectrum(y);
    let low = (20.0 * N as f64 / rate as f64) as usize;
    let rest: f64 = p[low..].iter().sum::<f64>() - p[bin];
    db_pow(rest / p[bin])
}

/// The spike when Bits or Rate jumps, against the crushed signal's own
/// steps: under a 997 Hz sine at -12 dBFS the largest first difference
/// just after each change, over the larger of the steady signal's before
/// and after it, dB, worst of the changes, at least 0.
fn crush_steps(d: &mut NodeDut) -> f64 {
    use ids::crush as c;
    let rate = d.rate();
    let seg = rate as usize / 4;
    let plan: [(u16, f64); 8] = [
        (c::RATE, 4000.0),
        (c::RATE, 22_050.0),
        (c::RATE, 2000.0),
        (c::RATE, 11_025.0),
        (c::BITS, 4.0),
        (c::BITS, 12.0),
        (c::BITS, 2.0),
        (c::BITS, 8.0),
    ];
    let n = seg * (plan.len() + 1);
    let x = sine(997.0, db_to_amp(-12.0), n, rate);
    let ch: Vec<Change> = plan
        .iter()
        .enumerate()
        .map(|(i, (id, v))| Change {
            frame: (i + 1) * seg,
            id: *id,
            value: *v,
        })
        .collect();
    d.reset();
    let y = d.process(&Stereo::mono(x, rate), &ch).left;
    let max_d1 = |a: usize, b: usize| {
        y[a..b]
            .windows(2)
            .fold(0.0f64, |m, w| m.max((w[1] - w[0]).abs()))
    };
    let mut worst = f64::MIN;
    for i in 1..=plan.len() {
        let at = i * seg;
        let before = max_d1(at - seg / 2, at - 10);
        let after = max_d1(at + seg / 2, at + seg - 10);
        let spike = max_d1(at - 10, at + seg / 2);
        worst = worst.max(db_amp(spike / before.max(after).max(1e-12)));
    }
    worst.max(0.0)
}

pub(super) fn crush(r: &mut Report) {
    use ids::crush as c;
    let rate = r.rate;
    let (main, presets) = engine_subjects(CRUSH, "crush", "crush");
    let name = main.name.clone();
    let name = name.as_str();

    // B1: bits mean bits; every frame held once.
    for bits in [4.0, 8.0, 12.0, 16.0] {
        let mut d = make_with(&main, rate, &[(c::BITS, bits), (c::RATE, 48_000.0)]);
        let (y, bin) = sine_out(&mut d, 1000.0, -0.1);
        let theory = -(6.02 * bits + 1.76);
        r.put(
            "B1",
            name,
            format!("sinad_error_bits{bits}"),
            Some(sinad(&y, bin, rate) - theory),
            Better::Near(0.0),
            Some(1.0),
            0.3,
        );
    }

    // B2: dither.
    let quiet = |dither: f64| {
        let mut d = make_with(
            &main,
            rate,
            &[(c::BITS, 8.0), (c::RATE, 48_000.0), (c::DITHER, dither)],
        );
        let (y, bin) = sine_out(&mut d, 1000.0, -0.1);
        sinad(&y, bin, rate)
    };
    r.put(
        "B2",
        name,
        "dither_noise_rise_bits8",
        Some(quiet(1.0) - quiet(0.0)),
        Better::Near(5.0),
        Some(1.0),
        0.3,
    );
    let mut d = make_with(
        &main,
        rate,
        &[(c::BITS, 8.0), (c::RATE, 48_000.0), (c::DITHER, 1.0)],
    );
    r.put(
        "B2",
        name,
        "thd_m40dbfs_bits8_dither",
        Some(floor(harmonics::measure(&mut d, 1000.0, -40.0).thd_db)),
        Better::Lower,
        Some(-30.0),
        2.0,
    );

    // B3: exact timing, in this session and at 48 kHz.
    let mut hosts = vec![(rate, String::new())];
    if rate != 48_000 {
        hosts.push((48_000, "_at48k".to_string()));
    }
    for (host, sfx) in &hosts {
        for rh in [8000.0, 10_000.0, 11_025.0, 22_050.0, 26_040.0, 32_000.0] {
            let mut d = make_with(&main, *host, &[(c::BITS, 16.0), (c::RATE, rh)]);
            let (total, worst) = off_grid(&mut d);
            r.put(
                "B3",
                name,
                format!("off_grid_{rh}hz{sfx}"),
                Some(total),
                Better::Lower,
                Some(-45.0),
                1.0,
            );
            r.put(
                "B3",
                name,
                format!("off_grid_worst_{rh}hz{sfx}"),
                Some(worst),
                Better::Lower,
                None,
                1.0,
            );
        }
    }
    // B4: where R divides the session's rate, the first image against the
    // zero-order hold's sin(x)/x.
    let rh = rate as f64 / 4.0;
    let mut d = make_with(&main, rate, &[(c::BITS, 16.0), (c::RATE, rh)]);
    let (y, bin) = sine_out(&mut d, 3000.0, -6.0);
    let p = power_spectrum(&y);
    let f0 = bin as f64 * rate as f64 / N as f64;
    let ib = ((rh - f0) * N as f64 / rate as f64).round() as usize;
    let img: f64 = p[ib - 1..=ib + 1].iter().sum();
    let sinc = |f: f64| {
        let a = std::f64::consts::PI * f / rh;
        a.sin() / a
    };
    let theory = db_amp((sinc(rh - f0) / sinc(f0)).abs());
    r.put(
        "B4",
        name,
        "image_error_quarter_rate",
        Some(db_pow(img / p[bin]) - theory),
        Better::Near(0.0),
        Some(1.0),
        0.3,
    );

    // B6: quiet tails at 8 bits (mid-tread gates under half a step).
    let mut d = make_with(&main, rate, &[(c::BITS, 8.0), (c::RATE, 48_000.0)]);
    let (y, _) = sine_out(&mut d, 1000.0, -50.0);
    r.put(
        "B6",
        name,
        "m50dbfs_bits8_out_dbfs",
        Some(floor(db_amp(peak_of(&y)))),
        Better::Info,
        None,
        0.0,
    );

    // B8: Bits and Rate changes against the crushed signal's own steps.
    let mut d = make(&main, rate);
    r.put(
        "B8",
        name,
        "step_spike_db",
        Some(crush_steps(&mut d)),
        Better::Lower,
        Some(3.0),
        1.0,
    );

    // B10: silence in, silence out.
    let mut d = make(&main, rate);
    let silence = Stereo::mono(vec![0.0; rate as usize], rate);
    let out = d.render(&silence, 0);
    r.put(
        "B10",
        name,
        "silence_out_dbfs",
        Some(floor(db_amp(peak_of(&out.left).max(peak_of(&out.right))))),
        Better::Lower,
        Some(-120.0),
        1.0,
    );

    // B9 and the usual zipper (not valid for a staircase: reported).
    let p18 = pink_at(rate, -18.0, 4.0);
    for s in std::iter::once(&main).chain(&presets) {
        let mut d = make(s, rate);
        r.put(
            "B9",
            &s.name,
            "lu_pink18",
            Some(lu(&mut d, &p18)),
            Better::Near(0.0),
            Some(1.0),
            0.3,
        );
        if s.preset.is_some() {
            // The preset's rate alone: at 16 bits, so the steps' own noise
            // doesn't count as stray.
            let mut d = make_with(s, rate, &[(c::BITS, 16.0), (c::DITHER, 0.0)]);
            let (total, _) = off_grid(&mut d);
            r.put(
                "B3",
                &s.name,
                "off_grid_at_16bits",
                Some(total),
                Better::Lower,
                Some(-45.0),
                1.0,
            );
        }
        r.put(
            "B8",
            &s.name,
            "zipper_db",
            worst_zipper(s, rate),
            Better::Info,
            None,
            0.0,
        );
    }
}

// ---------------------------------------------------------------------------
// Compressor (plan-2 section 6).

const COMPRESSOR: &str = "mpfree.fx.compressor";

/// A compressor engine with `values`, for its drawn curve.
fn compressor_with(values: &[(u16, f64)]) -> mpfree_fx::Compressor {
    let mut c = mpfree_fx::Compressor::new();
    for (id, v) in values {
        c.set_param(*id, *v as f32);
    }
    c
}

pub(super) fn compressor(r: &mut Report) {
    use ids::compressor as k;
    let rate = r.rate;
    let (main, presets) = engine_subjects(COMPRESSOR, "compressor", "compressor");
    let name = main.name.clone();
    let name = name.as_str();

    // K1: the static curve against the drawn one.
    let levels: Vec<f64> = (0..=13).map(|i| -39.0 + 3.0 * i as f64).collect();
    for (metric, values) in [
        ("curve_error_peak", vec![]),
        ("curve_error_peak_knee0", vec![(k::KNEE, 0.0)]),
        ("curve_error_rms", vec![(k::DETECT, 1.0)]),
    ] {
        let c = compressor_with(&values);
        let mut d = make_with(&main, rate, &values);
        let pts = dynamics::static_curve(&mut d, &levels);
        let worst = pts
            .iter()
            .map(|(i, o)| (o - c.curve_db(*i as f32) as f64).abs())
            .fold(0.0, f64::max);
        r.put(
            "K1",
            name,
            metric,
            Some(worst),
            Better::Lower,
            Some(0.25),
            0.05,
        );
    }

    // K2, K3: attack and release against their labels (63 %).
    for (att, rel) in [(1.0, 50.0), (10.0, 150.0), (30.0, 500.0), (100.0, 2000.0)] {
        let mut d = make_with(&main, rate, &[(k::ATTACK, att), (k::RELEASE, rel)]);
        let t = dynamics::timing(&mut d, -40.0, 0.0, (rel * 8.0 / 1000.0).max(0.5));
        r.put(
            "K2",
            name,
            format!("attack_ratio_{att}ms"),
            t.attack_ms.map(|a| a / att),
            Better::Near(1.0),
            Some(0.15),
            0.05,
        );
        r.put(
            "K3",
            name,
            format!("release_ratio_{rel}ms"),
            t.release_ms.map(|a| a / rel),
            Better::Near(1.0),
            Some(0.10),
            0.03,
        );
    }
    let mut d = make_with(&main, rate, &[(k::ATTACK, 0.1), (k::RELEASE, 10.0)]);
    let t = dynamics::timing(&mut d, -40.0, 0.0, 0.5);
    r.put(
        "K2",
        name,
        "attack_ms_at_0.1ms",
        t.attack_ms,
        Better::Info,
        None,
        0.0,
    );

    // K4: bass distortion under about 10 dB of reduction.
    for (att, rel, detect, target) in [
        (10.0, 150.0, 0.0, true),
        (10.0, 150.0, 1.0, false),
        (1.0, 50.0, 0.0, false),
        (0.1, 10.0, 0.0, false),
    ] {
        let det = if detect > 0.0 { "rms" } else { "peak" };
        for hz in [50.0, 100.0] {
            let mut d = make_with(
                &main,
                rate,
                &[
                    (k::ATTACK, att),
                    (k::RELEASE, rel),
                    (k::DETECT, detect),
                    (k::THRESHOLD, -20.0),
                    (k::KNEE, 0.0),
                ],
            );
            let thd = floor(harmonics::measure(&mut d, hz, -7.0).thd_db);
            let t = target.then_some(if hz < 75.0 { -50.0 } else { -56.0 });
            r.put(
                "K4",
                name,
                format!("thd_{hz}hz_gr10_{det}_{att}ms_{rel}ms"),
                Some(thd),
                Better::Lower,
                t,
                1.0,
            );
        }
    }

    // K5: the meter against the measured gain.
    let mut d = make(&main, rate);
    let pts = dynamics::static_curve(&mut d, &[-6.0]);
    let shown = d.node().param(k::REDUCTION) as f64;
    r.put(
        "K5",
        name,
        "meter_error",
        Some((shown - (pts[0].1 + 6.0)).abs()),
        Better::Lower,
        Some(0.1),
        0.02,
    );

    // K6: stereo link: a loud left pulls the quiet right down as much.
    let mut d = make(&main, rate);
    let n = rate as usize;
    let l = sine(1000.0, 1.0, n, rate);
    let rr = sine(1000.0, 0.1, n, rate);
    let y = d.render(&Stereo::new(l.clone(), rr.clone(), rate), 0);
    let g = |a: &[f64], b: &[f64]| db_amp(rms_of(&a[n / 2..]) / rms_of(&b[n / 2..]));
    r.put(
        "K6",
        name,
        "link_error",
        Some((g(&y.left, &l) - g(&y.right, &rr)).abs()),
        Better::Lower,
        Some(0.1),
        0.02,
    );

    // K8: idle bit for bit.
    let p18 = pink_at(rate, -18.0, 4.0);
    let p6 = pink_at(rate, -6.0, 4.0);
    let mut d = make_with(&main, rate, &[(k::RATIO, 1.0)]);
    r.put(
        "K8",
        name,
        "null_ratio1_dbfs",
        Some(null_dbfs(&mut d, &p6)),
        Better::Lower,
        Some(-140.0),
        1.0,
    );
    let mut d = make(&main, rate);
    let quiet = Stereo::mono(sine(1000.0, db_to_amp(-40.0), rate as usize, rate), rate);
    r.put(
        "K8",
        name,
        "null_below_knee_dbfs",
        Some(null_dbfs(&mut d, &quiet)),
        Better::Lower,
        Some(-140.0),
        1.0,
    );
    let mut d = make_with(&main, rate, &[(k::MIX, 0.0)]);
    r.put(
        "K8",
        name,
        "null_mix0_dbfs",
        Some(null_dbfs(&mut d, &p6)),
        Better::Lower,
        Some(-140.0),
        1.0,
    );

    // K9: overshoot of a level step (reported).
    for att in [1.0, 10.0] {
        let mut d = make_with(&main, rate, &[(k::ATTACK, att)]);
        let step = rate as usize;
        let x = level_steps(1000.0, -40.0, 0.0, step, 2, rate);
        let y = d.render(&Stereo::mono(x, rate), 0).left;
        let first = peak_of(&y[step..step + rate as usize / 20]);
        let settled = peak_of(&y[2 * step - rate as usize / 10..2 * step]);
        r.put(
            "K9",
            name,
            format!("overshoot_db_attack{att}ms"),
            Some(db_amp(first / settled)),
            Better::Info,
            None,
            0.0,
        );
    }

    // K7 (reported until auto makeup exists), K10.
    for s in std::iter::once(&main).chain(&presets) {
        let mut d = make(s, rate);
        r.put(
            "K7",
            &s.name,
            "lu_pink18",
            Some(lu(&mut d, &p18)),
            Better::Info,
            None,
            0.0,
        );
        r.put(
            "K7",
            &s.name,
            "lu_pink6",
            Some(lu(&mut d, &p6)),
            Better::Info,
            None,
            0.0,
        );
        r.put(
            "K10",
            &s.name,
            "zipper_db",
            worst_zipper(s, rate),
            Better::Lower,
            Some(3.0),
            0.5,
        );
    }
}

// ---------------------------------------------------------------------------
// Filters (plan-2 section 7).

/// A Multi filter engine with `values`, for its drawn response.
fn multi_with(values: &[(u16, f64)], rate: u32) -> mpfree_fx::MultiFilter {
    let mut f = mpfree_fx::MultiFilter::new();
    f.prepare(rate, mpfree_fx::mpfree_node::MAX_BLOCK);
    for (id, v) in values {
        f.set_param(*id, *v as f32);
    }
    f.reset();
    f
}

/// The loudest sample of pink at -18 dBFS RMS through `d` while control
/// `id` moves from `from` to `to` (log) over four seconds, dBFS.
fn swept_pink_peak(d: &mut NodeDut, id: u16, from: f64, to: f64, log: bool) -> f64 {
    let rate = d.rate();
    let n = 4 * rate as usize;
    let x = pink(n, db_to_amp(-18.0), 11);
    let ch: Vec<Change> = (0..400)
        .map(|i| {
            let t = i as f64 / 399.0;
            Change {
                frame: i * n / 400,
                id,
                value: if log {
                    from * (to / from).powf(t)
                } else {
                    from + t * (to - from)
                },
            }
        })
        .collect();
    d.set(id, from);
    d.reset();
    let y = d.process(&Stereo::mono(x, rate), &ch);
    db_amp(peak_of(&y.left).max(peak_of(&y.right)))
}

pub(super) fn filter(r: &mut Report) {
    use ids::dj_filter as dj;
    use ids::filter as m;
    let rate = r.rate;
    let (defaults, presets) = subjects("Filter", "filter");
    let multi = defaults
        .iter()
        .find(|s| mode_of(s) == "multi")
        .expect("the Filter family has Multi");
    let djs = defaults
        .iter()
        .find(|s| mode_of(s) == "dj")
        .expect("the Filter family has DJ");
    let mname = multi.name.as_str();

    for (slope, sl) in [(0.0, "12"), (1.0, "24")] {
        // F1: the drawn -3 dB point against the cutoff.
        let mut worst_pct = 0.0f64;
        for hz in [100.0, 1000.0, 5000.0, 10_000.0, 16_000.0] {
            let f = multi_with(&[(m::SLOPE, slope), (m::CUTOFF, hz)], rate);
            let (mut lo, mut hi) = (hz * 0.25, (hz * 4.0).min(0.49 * rate as f64));
            for _ in 0..60 {
                let mid = (lo * hi).sqrt();
                if (f.response_db(mid as f32) as f64) > -3.0103 {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            worst_pct = worst_pct.max((lo / hz - 1.0).abs() * 100.0);
        }
        r.put(
            "F1",
            mname,
            format!("cutoff_error_pct_{sl}db"),
            Some(worst_pct),
            Better::Lower,
            Some(1.0),
            0.05,
        );

        // F1: heard against drawn, a resonant low-pass and a high-pass.
        for (ty, res, tag) in [(0.0, 50.0, "lp_res50"), (1.0, 0.0, "hp_res0")] {
            let values = [
                (m::TYPE, ty),
                (m::SLOPE, slope),
                (m::CUTOFF, 1000.0),
                (m::RESONANCE, res),
            ];
            let f = multi_with(&values, rate);
            let mut d = make_with(multi, rate, &values);
            let worst = [100.0, 300.0, 700.0, 1000.0, 1400.0, 3000.0, 8000.0]
                .iter()
                .map(|hz| (gain_at(&mut d, *hz, -24.0) - f.response_db(*hz as f32) as f64).abs())
                .fold(0.0, f64::max);
            r.put(
                "F1",
                mname,
                format!("heard_vs_drawn_{tag}_{sl}db"),
                Some(worst),
                Better::Lower,
                Some(0.1),
                0.02,
            );
        }

        // F2: the passband (100 Hz under a 1 kHz low-pass) by Resonance;
        // F4: a quiet sine at the cutoff still gets the peak.
        for res in [0.0, 50.0, 100.0] {
            let mut d = make_with(
                multi,
                rate,
                &[(m::SLOPE, slope), (m::CUTOFF, 1000.0), (m::RESONANCE, res)],
            );
            r.put(
                "F2",
                mname,
                format!("passband_res{res}_{sl}db"),
                Some(gain_at(&mut d, 100.0, -24.0)),
                Better::Near(0.0),
                Some(0.5),
                0.05,
            );
        }
        let mut d = make_with(
            multi,
            rate,
            &[
                (m::SLOPE, slope),
                (m::CUTOFF, 1000.0),
                (m::RESONANCE, 100.0),
            ],
        );
        r.put(
            "F4",
            mname,
            format!("peak_gain_m40dbfs_res100_{sl}db"),
            Some(gain_at(&mut d, 1000.0, -40.0)),
            Better::Higher,
            Some(28.0),
            0.5,
        );

        // F3: pink at a mix level through a swept, resonant low-pass.
        for res in [75.0, 100.0] {
            let mut d = make_with(multi, rate, &[(m::SLOPE, slope), (m::RESONANCE, res)]);
            r.put(
                "F3",
                mname,
                format!("swept_pink_peak_res{res}_{sl}db"),
                Some(swept_pink_peak(&mut d, m::CUTOFF, 100.0, 10_000.0, true)),
                Better::Lower,
                (res == 100.0).then_some(-1.0),
                0.3,
            );
        }

        // F6: cutoff sweeps against the tone's own slope.
        for (ty, tn) in [(0.0, "lp"), (1.0, "hp"), (2.0, "bp")] {
            for res in [0.0, 50.0] {
                let mut d = make_with(
                    multi,
                    rate,
                    &[(m::TYPE, ty), (m::SLOPE, slope), (m::RESONANCE, res)],
                );
                let z = slope_zipper(&mut d, m::CUTOFF, 20.0, 20_000.0, true, 50.0);
                r.put(
                    "F6",
                    mname,
                    format!("sweep50ms_{tn}{sl}_res{res}"),
                    Some(z),
                    if res == 0.0 {
                        Better::Lower
                    } else {
                        Better::Info
                    },
                    (res == 0.0).then_some(3.0),
                    0.5,
                );
            }
        }
    }

    // F5: type and slope changes.
    let mut d = make_with(multi, rate, &[(m::CUTOFF, 1000.0), (m::SLOPE, 1.0)]);
    r.put(
        "F5",
        mname,
        "click_lp_to_hp_24db",
        Some(switch_click(&mut d, m::TYPE, 1.0, 100.0)),
        Better::Lower,
        Some(3.0),
        1.0,
    );
    let mut d = make_with(multi, rate, &[(m::CUTOFF, 300.0), (m::SLOPE, 0.0)]);
    r.put(
        "F5",
        mname,
        "click_12_to_24db",
        Some(switch_click(&mut d, m::SLOPE, 1.0, 1000.0)),
        Better::Lower,
        Some(3.0),
        1.0,
    );

    // F7: the drive.
    for (drv, target) in [(12.0, Some(-65.0)), (24.0, None)] {
        let mut d = make_with(multi, rate, &[(m::DRIVE, drv)]);
        r.put(
            "F7",
            mname,
            format!("drive{drv}_alias_5k_0dbfs"),
            Some(alias(&mut d, 5000.0, 0.0).0),
            Better::Lower,
            target,
            1.0,
        );
    }

    // F8: the DJ filter.
    let dname = djs.name.as_str();
    let pink6 = pink_at(rate, -6.0, 2.0);
    let mut d = make_with(djs, rate, &[(dj::POSITION, 0.0)]);
    r.put(
        "F8",
        dname,
        "null_middle_dbfs",
        Some(null_dbfs(&mut d, &pink6)),
        Better::Lower,
        Some(-140.0),
        1.0,
    );
    for res in [0.0, 50.0] {
        let mut d = make_with(djs, rate, &[(dj::RESONANCE, res)]);
        let z = slope_zipper(&mut d, dj::POSITION, -100.0, 100.0, false, 50.0);
        r.put(
            "F8",
            dname,
            format!("position_sweep50ms_res{res}"),
            Some(z),
            if res == 0.0 {
                Better::Lower
            } else {
                Better::Info
            },
            (res == 0.0).then_some(3.0),
            0.5,
        );
    }
    let corner = |pos: f64| {
        let mut f = mpfree_fx::DjFilter::new();
        f.prepare(rate, mpfree_fx::mpfree_node::MAX_BLOCK);
        f.set_param(dj::POSITION, pos as f32);
        f.reset();
        f.corner().map(|c| c.1 as f64)
    };
    r.put(
        "F8",
        dname,
        "lowpass_corner_hz_at_left",
        corner(-100.0),
        Better::Lower,
        Some(100.0),
        1.0,
    );
    r.put(
        "F8",
        dname,
        "highpass_corner_hz_at_right",
        corner(100.0),
        Better::Higher,
        Some(5000.0),
        10.0,
    );
    let mut d = make_with(djs, rate, &[(dj::RESONANCE, 100.0)]);
    r.put(
        "F8",
        dname,
        "swept_pink_peak_res100",
        Some(swept_pink_peak(&mut d, dj::POSITION, -1.0, -100.0, false)),
        Better::Lower,
        Some(-1.0),
        0.3,
    );

    // F10: fxlab's usual zipper (reported: it reads high where the ends
    // attenuate the tone; F6 and F8 are the measures that fit).
    for s in defaults.iter().chain(&presets) {
        r.put(
            "F10",
            &s.name,
            "zipper_db",
            worst_zipper(s, rate),
            Better::Info,
            None,
            0.0,
        );
    }
}

// ---------------------------------------------------------------------------
// Lo-fi (plan-2 section 8).

const LOFI: &str = "mpfree.fx.lofi";

/// The share of the strongest line (with its neighbours) in the spectrum
/// of the pitch track of a 3.15 kHz tone through `d` over ten seconds: 1
/// for a single sine of wow, small for irregular wow.
fn wow_line_share(d: &mut NodeDut) -> Option<f64> {
    let rate = d.rate();
    let n = 10 * rate as usize;
    let x = sine(3150.0, db_to_amp(-12.0), n, rate);
    let y = d.render(&Stereo::mono(x, rate), 0).left;
    let f = flutter::instantaneous_hz(&y[rate as usize / 2..], rate, 3150.0);
    if f.len() < 1024 {
        return None;
    }
    let mean = f.iter().sum::<f64>() / f.len() as f64;
    let len = f.len().next_power_of_two() / 2;
    let dev: Vec<f64> = f[..len].iter().map(|v| v - mean).collect();
    let p = power_spectrum(&dev);
    let total: f64 = p[1..].iter().sum();
    let best = (2..p.len() - 1)
        .map(|k| p[k - 1] + p[k] + p[k + 1])
        .fold(0.0, f64::max);
    (total > 0.0).then(|| best / total)
}

pub(super) fn lofi(r: &mut Report) {
    use ids::lofi as l;
    let rate = r.rate;
    let (main, presets) = engine_subjects(LOFI, "lofi", "lofi");
    let name = main.name.clone();
    let name = name.as_str();
    let quiet = [(l::CRACKLE, 0.0), (l::NOISE, 0.0), (l::SATURATION, 0.0)];
    let with = |extra: &[(u16, f64)]| {
        let mut v = quiet.to_vec();
        v.extend_from_slice(extra);
        v
    };

    // L1, L2, L3: wow and flutter.
    for (metric, values, better, target) in [
        ("wf_peak_pct", with(&[]), Better::Near(0.116), Some(0.023)),
        (
            "wow_peak_pct_top",
            with(&[(l::AMOUNT, 100.0), (l::WOW, 100.0), (l::FLUTTER, 0.0)]),
            Better::Higher,
            Some(2.0),
        ),
        (
            "flutter_peak_pct_top",
            with(&[(l::AMOUNT, 100.0), (l::WOW, 0.0), (l::FLUTTER, 100.0)]),
            Better::Higher,
            Some(0.5),
        ),
        (
            "wf_peak_pct_amount0",
            with(&[(l::AMOUNT, 0.0)]),
            Better::Lower,
            Some(0.001),
        ),
    ] {
        let mut d = make_with(&main, rate, &values);
        let f = flutter::measure(&mut d, 10.0);
        let id = if metric.contains("flutter") {
            "L2"
        } else {
            "L1"
        };
        r.put(
            id,
            name,
            metric,
            f.as_ref().map(|f| f.peak_pct),
            better,
            target,
            0.01,
        );
        r.put(
            "L3",
            name,
            format!("mean_pitch_error_pct_{metric}"),
            f.as_ref()
                .map(|f| ((f.mean_hz - 3150.0) / 3150.0 * 100.0).abs()),
            Better::Lower,
            Some(0.01),
            0.002,
        );
    }
    for s in &presets {
        let mut v: Vec<(u16, f64)> = Vec::new();
        v.extend_from_slice(&quiet);
        let mut d = make_with(s, rate, &v);
        let f = flutter::measure(&mut d, 10.0);
        r.put(
            "L1",
            &s.name,
            "wf_peak_pct",
            f.as_ref().map(|f| f.peak_pct),
            Better::Info,
            None,
            0.0,
        );
        r.put(
            "L3",
            &s.name,
            "mean_pitch_error_pct",
            f.as_ref()
                .map(|f| ((f.mean_hz - 3150.0) / 3150.0 * 100.0).abs()),
            Better::Lower,
            Some(0.01),
            0.002,
        );
    }

    // L4: how regular the wow is.
    let mut d = make_with(
        &main,
        rate,
        &with(&[(l::AMOUNT, 100.0), (l::WOW, 100.0), (l::FLUTTER, 0.0)]),
    );
    r.put(
        "L4",
        name,
        "wow_line_share",
        wow_line_share(&mut d),
        Better::Lower,
        Some(0.5),
        0.05,
    );

    // L5, L6: saturation.
    for sat in [40.0, 100.0] {
        let mut d = make_with(
            &main,
            rate,
            &[
                (l::AMOUNT, 100.0),
                (l::SATURATION, sat),
                (l::WOW, 0.0),
                (l::FLUTTER, 0.0),
                (l::NOISE, 0.0),
                (l::CRACKLE, 0.0),
                (l::LOW_CUT, 20.0),
                (l::HIGH_CUT, 20_000.0),
            ],
        );
        r.put(
            "L5",
            name,
            format!("sat{sat}_alias_5k_0dbfs"),
            Some(alias(&mut d, 5000.0, 0.0).0),
            Better::Lower,
            (sat == 40.0).then_some(-65.0),
            1.0,
        );
        let h = harmonics::measure(&mut d, 1000.0, -12.0);
        r.put(
            "L6",
            name,
            format!("sat{sat}_h3_minus_h2"),
            Some(floor(h.h[1].unwrap_or(-300.0)) - floor(h.h[0].unwrap_or(-300.0))),
            Better::Info,
            None,
            0.0,
        );
    }

    // L7: noise at the defaults, with music and in the silence after it.
    let mut d = make_with(
        &main,
        rate,
        &[(l::WOW, 0.0), (l::FLUTTER, 0.0), (l::SATURATION, 0.0)],
    );
    let h = harmonics::measure(&mut d, 1000.0, -18.0);
    r.put(
        "L7",
        name,
        "noise_m18dbfs_tone_db",
        Some(floor(h.other_total_db)),
        Better::Info,
        None,
        0.0,
    );
    let n = 2 * rate as usize;
    let mut x = sine(1000.0, db_to_amp(-18.0), n, rate);
    x.resize(2 * n, 0.0);
    let y = d.render(&Stereo::mono(x, rate), 0).left;
    r.put(
        "L7",
        name,
        "noise_in_silence_dbfs",
        Some(floor(db_amp(rms_of(&y[n + rate as usize / 2..])))),
        Better::Info,
        None,
        0.0,
    );

    // L8: the drawn cuts against the heard (wow and the rest off).
    for (lo, hi) in [(80.0, 8000.0), (420.0, 3200.0)] {
        let values = [
            (l::AMOUNT, 100.0),
            (l::SATURATION, 0.0),
            (l::WOW, 0.0),
            (l::FLUTTER, 0.0),
            (l::NOISE, 0.0),
            (l::CRACKLE, 0.0),
            (l::LOW_CUT, lo),
            (l::HIGH_CUT, hi),
        ];
        let mut f = mpfree_fx::LoFi::new();
        f.prepare(rate, mpfree_fx::mpfree_node::MAX_BLOCK);
        for (id, v) in values {
            f.set_param(id, v as f32);
        }
        f.reset();
        let mut d = make_with(&main, rate, &values);
        let worst = [40.0, 80.0, 160.0, 420.0, 1000.0, 3200.0, 8000.0, 12_000.0]
            .iter()
            .map(|hz| (gain_at(&mut d, *hz, -24.0) - f.cuts_db(*hz as f32) as f64).abs())
            .fold(0.0, f64::max);
        r.put(
            "L8",
            name,
            format!("cuts_heard_vs_drawn_{lo}_{hi}"),
            Some(worst),
            Better::Lower,
            Some(0.5),
            0.05,
        );
    }

    // L10: Amount 0 is the input, late by the latency.
    let mut d = make_with(&main, rate, &[(l::AMOUNT, 0.0)]);
    let pink6 = pink_at(rate, -6.0, 2.0);
    r.put(
        "L10",
        name,
        "null_amount0_dbfs",
        Some(null_dbfs(&mut d, &pink6)),
        Better::Lower,
        Some(-140.0),
        1.0,
    );
    r.put(
        "L13",
        name,
        "latency_frames",
        Some(d.latency() as f64),
        Better::Info,
        None,
        0.0,
    );

    // L9, L11: level and zipper, defaults and presets.
    let p18 = pink_stim(rate, 4.0);
    for s in std::iter::once(&main).chain(&presets) {
        let mut d = make(s, rate);
        let radio = s.name.contains("Old radio");
        r.put(
            "L9",
            &s.name,
            "lu_pink18",
            Some(lu(&mut d, &p18)),
            Better::Near(0.0),
            (!radio).then_some(1.0),
            0.3,
        );
        r.put(
            "L11",
            &s.name,
            "zipper_db",
            worst_zipper(s, rate),
            Better::Lower,
            Some(3.0),
            0.5,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dut::Wire;

    const RATE: u32 = 44_100;

    #[test]
    fn the_lock_in_reads_a_sines_amplitude() {
        let x = sine(997.0, 0.3, RATE as usize / 5, RATE);
        assert!((amplitude_at(&x, 997.0, RATE) - 0.3).abs() < 1e-3);
        assert!(amplitude_at(&x, 1500.0, RATE) < 0.01);
        let mut w = Wire { rate: RATE };
        assert!(gain_at(&mut w, 440.0, -12.0).abs() < 0.01);
    }

    #[test]
    fn the_slope_measure_is_zero_for_a_clean_sine_and_spikes_on_a_click() {
        let mut x = sine(997.0, 0.25, RATE as usize / 2, RATE);
        let n = x.len();
        assert!(rel_d2(&x, 997.0, RATE, 0, n).abs() < 0.5);
        // A quieter stretch is still clean: the measure follows the level.
        for v in &mut x[n / 2..] {
            *v *= 0.01;
        }
        assert!(rel_d2(&x, 997.0, RATE, n / 2 + 2000, n).abs() < 0.5);
        x[n / 4] += 0.05;
        assert!(rel_d2(&x, 997.0, RATE, 0, n / 2) > 20.0);
    }

    #[test]
    fn the_window_is_a_four_term_blackman_harris() {
        let w = blackman_harris(1024);
        assert!(w[0].abs() < 1e-4, "{}", w[0]);
        assert!((w[512] - 1.0).abs() < 1e-9);
        let mean = w.iter().sum::<f64>() / w.len() as f64;
        assert!((mean - 0.35875).abs() < 1e-9);
    }

    #[test]
    fn a_rate_that_divides_the_sessions_has_nothing_off_grid() {
        use ids::crush as c;
        let mut d = NodeDut::from_registry(CRUSH, RATE).unwrap();
        d.set(c::BITS, 16.0);
        d.set(c::RATE, RATE as f64 / 4.0);
        assert!(off_grid(&mut d).0 < -80.0);
        // A rate that doesn't puts stray tones in today's crusher.
        d.set(c::RATE, 8000.0);
        assert!(off_grid(&mut d).0 > -40.0);
    }
}
