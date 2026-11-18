//! The third round's class of the quality report: modulation (the Chorus,
//! Flanger and Phaser). The plan's target ids are M1 to M13; the targets
//! and why each was chosen are in `docs/dev/design/fx-quality.md`. The
//! rows, the report and the ratchet are the first round's (`super`).

use mpfree_fx::ids::{chorus as ch, flanger as fl, phaser as ph};
use mpfree_fx::mpfree_node::{Ctx, Curve, Node};
use mpfree_fx::{Chorus, Flanger, Phaser};

use super::{floor, lu_change, make, mono_sum, pink_stim, Better, Report, Subject};
use crate::dut::{Change, Dut, NodeDut};
use crate::fft::{hann_f64, power_spectrum, Fft};
use crate::signals::{db_to_amp, impulse, sine, white};
use crate::wav::Stereo;
use crate::{flutter, mixlaw};

const CHORUS: &str = "mpfree.fx.chorus";
const FLANGER: &str = "mpfree.fx.flanger";
const PHASER: &str = "mpfree.fx.phaser";

fn db_pow(p: f64) -> f64 {
    10.0 * p.max(1e-30).log10()
}

fn db_amp(a: f64) -> f64 {
    20.0 * a.max(1e-15).log10()
}

/// An engine at its defaults, then at each of its factory presets:
/// `modulation/name` and `modulation/name[Preset]`.
fn engine_subjects(id: &'static str, name: &str) -> Vec<Subject> {
    let s = |preset: Option<&'static mpfree_fx::Preset>| Subject {
        name: match preset {
            None => format!("modulation/{name}"),
            Some(p) => format!("modulation/{name}[{}]", p.name),
        },
        id,
        fixed: &[],
        preset,
    };
    let mut all = vec![s(None)];
    all.extend(mpfree_fx::presets(id).iter().map(|p| s(Some(p))));
    all
}

fn engine(id: &str, rate: u32, values: &[(u16, f64)]) -> NodeDut {
    let mut d = NodeDut::from_registry(id, rate).unwrap_or_else(|| panic!("{id} is registered"));
    for (k, v) in values {
        d.set(*k, *v);
    }
    d
}

fn mix_id(id: &str) -> u16 {
    match id {
        FLANGER => fl::MIX,
        PHASER => ph::MIX,
        _ => ch::MIX,
    }
}

/// The left/right correlation of a signal's content above `hz`.
fn corr_above(l: &[f64], r: &[f64], hz: f64, rate: u32) -> f64 {
    let n = l.len().next_power_of_two();
    let fft = Fft::new(n);
    let hp = |x: &[f64]| {
        let mut re = x.to_vec();
        re.resize(n, 0.0);
        let mut im = vec![0.0; n];
        fft.forward(&mut re, &mut im);
        let cut = (hz * n as f64 / rate as f64) as usize;
        for k in 0..n {
            if k < cut || k > n - cut {
                re[k] = 0.0;
                im[k] = 0.0;
            }
        }
        fft.inverse(&mut re, &mut im);
        re.truncate(x.len());
        re
    };
    let (a, b) = (hp(l), hp(r));
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(&b) {
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    ab / (aa * bb).sqrt().max(1e-300)
}

/// The wet signal's left/right correlation above 300 Hz on mono pink.
fn width(d: &mut NodeDut, rate: u32) -> f64 {
    let x = pink_stim(rate, 4.0);
    let out = d.render(&x, 0);
    let s = rate as usize;
    corr_above(&out.left[s..], &out.right[s..], 300.0, rate)
}

/// The pitch of a 1 kHz tone at -12 dBFS through `d` (its left side), in
/// cents from its mean, per frame from the first half second on.
fn pitch_cents(d: &mut NodeDut, secs: f64) -> Vec<f64> {
    let rate = d.rate();
    let n = (secs * rate as f64) as usize;
    let out = d.render(
        &Stereo::mono(sine(1000.0, db_to_amp(-12.0), n, rate), rate),
        0,
    );
    let f = flutter::instantaneous_hz(&out.left, rate, 1000.0);
    let cents: Vec<f64> = f
        .iter()
        .map(|v| 1200.0 * (v.max(1e-3) / 1000.0).log2())
        .collect();
    let mean = cents.iter().sum::<f64>() / cents.len().max(1) as f64;
    cents.iter().map(|c| c - mean).collect()
}

/// The largest pitch bend, cents, of a 1 kHz tone through `d`: three LFO
/// cycles at `hz` (at least 3 s).
fn bend(d: &mut NodeDut, hz: f64) -> f64 {
    let secs = (3.0 / hz).clamp(3.0, 40.0) + 1.0;
    pitch_cents(d, secs)
        .iter()
        .fold(0.0f64, |m, c| m.max(c.abs()))
}

/// The fine magnitude response (dB per bin) of a held setting, from its
/// impulse response's left side.
fn fine(d: &mut NodeDut, n: usize) -> Vec<f64> {
    let out = d.render(&Stereo::mono(impulse(n, 1.0), d.rate()), 0);
    let mut re = out.left.clone();
    re.truncate(n);
    let mut im = vec![0.0; n];
    Fft::new(n).forward(&mut re, &mut im);
    (0..=n / 2)
        .map(|k| db_pow(re[k] * re[k] + im[k] * im[k]))
        .collect()
}

/// A delay in ms whose length in frames at `rate` is a whole number plus
/// `frac`, near `ms`.
fn frames_ms(ms: f64, frac: f64, rate: u32) -> f64 {
    ((ms * 0.001 * rate as f64).round() + frac) / rate as f64 * 1000.0
}

/// The worst difference, dB, between a held response and `drawn(hz)`
/// from 30 Hz to 18 kHz, where the drawn response is above -20 dB.
fn drawn_error(r: &[f64], rate: u32, drawn: impl Fn(f64) -> f64) -> f64 {
    let n = 2 * (r.len() - 1);
    let hz = |k: usize| k as f64 * rate as f64 / n as f64;
    let mut worst = 0.0f64;
    let mut k = (30.0 * n as f64 / rate as f64) as usize;
    while hz(k) < 18_000.0 {
        let want = drawn(hz(k));
        if want > -20.0 {
            worst = worst.max((want - r[k]).abs());
        }
        k += 3;
    }
    worst
}

/// The zipper and click measure for effects whose controls move pitch:
/// sweeps control `id` from `lo` to `hi` and back, 50 ms each way, under a
/// 997 Hz sine at -12 dBFS, and returns the loudest millisecond of the
/// output above 6 kHz while it moves, in dB against the tone. A swept
/// delay or LFO bends the tone's pitch and a resonance raises its level,
/// but neither puts energy two octaves up; a stepped control or a click
/// does.
pub(super) fn hf_click(d: &mut NodeDut, id: u16, lo: f64, hi: f64, log: bool) -> f64 {
    let rate = d.rate();
    let hz = 997.0;
    let rest = (0.3 * rate as f64) as usize;
    let sweep = (0.05 * rate as f64) as usize;
    let x = sine(hz, db_to_amp(-12.0), 3 * rest + 2 * sweep, rate);
    let at = |t: f64| {
        if log {
            (lo.ln() + t * (hi.ln() - lo.ln())).exp()
        } else {
            lo + t * (hi - lo)
        }
    };
    let steps = (sweep / 32).max(1);
    let mut changes = Vec::new();
    for k in 1..=steps {
        let t = k as f64 / steps as f64;
        changes.push(Change {
            frame: rest + k * sweep / steps - 1,
            id,
            value: at(t),
        });
        changes.push(Change {
            frame: 2 * rest + sweep + k * sweep / steps - 1,
            id,
            value: at(1.0 - t),
        });
    }
    changes.sort_by_key(|c| c.frame);
    d.set(id, lo);
    d.reset();
    let y = d.process(&Stereo::mono(x, rate), &changes).left;
    let n = y.len().next_power_of_two();
    let fft = Fft::new(n);
    let mut re = y.clone();
    re.resize(n, 0.0);
    let mut im = vec![0.0; n];
    fft.forward(&mut re, &mut im);
    let cut = (6000.0 * n as f64 / rate as f64) as usize;
    for k in 0..n {
        if k < cut || k > n - cut {
            re[k] = 0.0;
            im[k] = 0.0;
        }
    }
    fft.inverse(&mut re, &mut im);
    let ms = (rate / 1000) as usize;
    let g = rate as usize / 20;
    let loudest = |from: usize, to: usize| {
        let mut worst = 0.0f64;
        let mut i = from;
        while i + ms <= to {
            let e = re[i..i + ms].iter().map(|v| v * v).sum::<f64>() / ms as f64;
            worst = worst.max(e);
            i += ms / 2;
        }
        worst
    };
    let moving =
        loudest(rest, rest + sweep + g).max(loudest(2 * rest + sweep, 2 * rest + 2 * sweep + g));
    let tone = db_to_amp(-12.0).powi(2) / 2.0;
    floor(db_pow(moving / tone))
}

/// How far, in cycles (0 to 0.5), a bar-synced chorus's LFO is from the
/// song position after the transport loops back from 3.5 bars to the
/// start: the phase of its pitch wobble over the last bar before the wrap
/// against the phase over the second bar after it.
fn sync_error(rate: u32) -> f64 {
    let mut node = Chorus::new();
    node.prepare(rate, mpfree_fx::mpfree_node::MAX_BLOCK);
    for (k, v) in [
        (ch::VOICES, 1.0f32),
        (ch::SYNC, 5.0),
        (ch::MIX, 100.0),
        (ch::DEPTH, 100.0),
        (ch::DELAY, 20.0),
    ] {
        node.set_param(k, v);
    }
    node.reset();
    let bar = 2.0 * rate as f64; // 120 BPM
    let wrap = 14.0; // beats: 3.5 bars
    let n = (14.0 * rate as f64) as usize;
    let x = sine(1000.0, db_to_amp(-12.0), n, rate);
    let mut l: Vec<f32> = x.iter().map(|v| *v as f32).collect();
    let mut r = l.clone();
    let mut beats = 0.0f64;
    let mut frame = 0;
    // The song position (in bars, 0 to 1 within the bar) at each frame.
    let mut pos = Vec::with_capacity(n);
    while frame < n {
        let len = 128.min(n - frame);
        let ctx = Ctx {
            rate,
            frame: frame as u64,
            bpm: 120.0,
            beats,
            playing: true,
            sidechain: None,
        };
        node.process(
            &ctx,
            &[],
            &mut l[frame..frame + len],
            &mut r[frame..frame + len],
        );
        for i in 0..len {
            pos.push(((beats + i as f64 * 2.0 / rate as f64) / 4.0).fract());
        }
        beats += len as f64 * 2.0 / rate as f64;
        if beats >= wrap {
            beats -= wrap;
        }
        frame += len;
    }
    let y: Vec<f64> = l.iter().map(|v| *v as f64).collect();
    let f = flutter::instantaneous_hz(&y, rate, 1000.0);
    // instantaneous_hz smooths over 2.2 ms (w frames) and leaves out a
    // tenth at each end: its index i is about frame i + skip.
    let w = ((0.0022 * rate as f64) as usize).max(1);
    let skip = (n - w) / 10 + w / 2;
    // The phase of the wobble against the song position over one bar from
    // frame `a`: the angle of the pitch's correlation with the bar's sine.
    let phase = |a: usize| {
        let (mut s, mut c) = (0.0, 0.0);
        for i in a..a + bar as usize {
            let p = f[i - skip];
            let t = std::f64::consts::TAU * pos[i];
            s += p * t.sin();
            c += p * t.cos();
        }
        s.atan2(c) / std::f64::consts::TAU
    };
    let wrap_frame = (7.0 * rate as f64) as usize;
    let before = phase(wrap_frame - bar as usize);
    let after = phase(wrap_frame + bar as usize);
    let d = (after - before).rem_euclid(1.0);
    d.min(1.0 - d)
}

pub(super) fn modulation(r: &mut Report) {
    let rate = r.rate;
    let pink = pink_stim(rate, 6.0);
    let hits = mixlaw::hits(6.0, rate);
    let engines = [(CHORUS, "chorus"), (FLANGER, "flanger"), (PHASER, "phaser")];

    // Per engine and preset: M1 level, M4 mono fold-down, M13 zipper.
    for (id, name) in engines {
        for s in engine_subjects(id, name) {
            let sn = s.name.as_str();
            let mut d = make(&s, rate);
            r.put(
                "M1",
                sn,
                "insert_lu_pink",
                Some(lu_change(&mut d, &pink)),
                Better::Near(0.0),
                Some(1.0),
                0.2,
            );
            r.put(
                "M1",
                sn,
                "insert_lu_hits",
                Some(lu_change(&mut d, &hits)),
                Better::Near(0.0),
                Some(1.0),
                0.2,
            );
            r.put(
                "M4",
                sn,
                "mono_fold_lu",
                Some(mono_sum(&mut d, &pink)),
                Better::Higher,
                Some(-1.5),
                0.2,
            );
            let params = d.params();
            let mut worst = (String::new(), -140.0f64);
            for p in params.iter().filter(|p| {
                !p.readonly && matches!(p.curve, Curve::Linear | Curve::Log) && p.max > p.min
            }) {
                let mut z = make(&s, rate);
                let log = matches!(p.curve, Curve::Log);
                let v = hf_click(&mut z, p.id, p.min as f64, p.max as f64, log);
                if p.name == "Delay" {
                    // A fast Delay sweep bends the pitch, and round a
                    // feedback loop it bends it again every trip, as a
                    // flanger's Manual turned fast does: reported.
                    r.put(
                        "M13",
                        sn,
                        "click_db_over_6k_delay",
                        Some(v),
                        Better::Info,
                        None,
                        0.0,
                    );
                } else if v > worst.1 {
                    worst = (p.name.to_string(), v);
                }
            }
            r.put(
                "M13",
                sn,
                "click_db_over_6k",
                Some(worst.1),
                Better::Lower,
                Some(-60.0),
                3.0,
            );
            if !worst.0.is_empty() {
                r.notes
                    .push(format!("{sn}: the loudest click is {}", worst.0));
            }
        }
    }

    // M13: a change of LFO shape (M4), Sine to Triangle and back, at the
    // defaults: the two shapes crossfade, so it should click no more than
    // a knob does.
    for (id, name, shape) in [
        (CHORUS, "chorus", ch::SHAPE),
        (FLANGER, "flanger", fl::SHAPE),
        (PHASER, "phaser", ph::SHAPE),
    ] {
        let mut d = engine(id, rate, &[]);
        r.put(
            "M13",
            &format!("modulation/{name}"),
            "click_db_over_6k_shape",
            Some(hf_click(&mut d, shape, 0.0, 1.0, false)),
            Better::Lower,
            Some(-60.0),
            3.0,
        );
    }

    // M2: level across Feedback at the default Mix, against Feedback 0.
    for (id, name, fb_id) in [
        (FLANGER, "flanger", fl::FEEDBACK),
        (PHASER, "phaser", ph::FEEDBACK),
    ] {
        let sn = format!("modulation/{name}");
        let zero = lu_change(&mut engine(id, rate, &[(fb_id, 0.0)]), &pink);
        for fb in [-95.0, -70.0, 70.0, 95.0] {
            let v = lu_change(&mut engine(id, rate, &[(fb_id, fb)]), &pink) - zero;
            r.put(
                "M2",
                &sn,
                format!(
                    "feedback_lu_{}{}",
                    if fb < 0.0 { "m" } else { "p" },
                    fb.abs()
                ),
                Some(v),
                Better::Near(0.0),
                Some(3.0),
                0.3,
            );
        }
    }

    // M3: width of the wet signal at Width 100 %.
    for voices in 1..=4 {
        let mut d = engine(
            CHORUS,
            rate,
            &[(ch::VOICES, voices as f64), (ch::MIX, 100.0)],
        );
        r.put(
            "M3",
            "modulation/chorus",
            format!("wet_corr_300hz_{voices}v"),
            Some(width(&mut d, rate)),
            Better::Lower,
            Some(0.3),
            0.05,
        );
    }
    for (id, name) in [(FLANGER, "flanger"), (PHASER, "phaser")] {
        let mut d = engine(id, rate, &[(mix_id(id), 100.0)]);
        r.put(
            "M3",
            &format!("modulation/{name}"),
            "wet_corr_300hz",
            Some(width(&mut d, rate)),
            Better::Lower,
            Some(0.3),
            0.05,
        );
    }

    // M5: the chorus's deepest bend anywhere on its knobs; the flanger's
    // and phaser's reported.
    let sn = "modulation/chorus";
    for (tag, rate_hz, delay) in [
        ("r1_d40", 1.0, 40.0),
        ("r10_d40", 10.0, 40.0),
        ("r10_d2", 10.0, 2.0),
    ] {
        let mut d = engine(
            CHORUS,
            rate,
            &[
                (ch::VOICES, 1.0),
                (ch::MIX, 100.0),
                (ch::DEPTH, 100.0),
                (ch::RATE, rate_hz),
                (ch::DELAY, delay),
            ],
        );
        r.put(
            "M5",
            sn,
            format!("bend_cents_depth100_{tag}"),
            Some(bend(&mut d, rate_hz)),
            Better::Lower,
            Some(100.0),
            2.0,
        );
    }
    // The Triangle (M4) at the fastest and deepest: its bend is steady
    // between the corners, and no more than the sine's peak.
    let mut d = engine(
        CHORUS,
        rate,
        &[
            (ch::VOICES, 1.0),
            (ch::MIX, 100.0),
            (ch::DEPTH, 100.0),
            (ch::RATE, 10.0),
            (ch::DELAY, 40.0),
            (ch::SHAPE, 1.0),
        ],
    );
    r.put(
        "M5",
        sn,
        "bend_cents_depth100_r10_d40_triangle",
        Some(bend(&mut d, 10.0)),
        Better::Lower,
        Some(100.0),
        2.0,
    );
    let mut d = engine(
        FLANGER,
        rate,
        &[
            (fl::MIX, 100.0),
            (fl::FEEDBACK, 0.0),
            (fl::DEPTH, 100.0),
            (fl::RATE, 10.0),
            (fl::DELAY, 10.0),
        ],
    );
    r.put(
        "M5",
        "modulation/flanger",
        "bend_cents_depth100_r10_d10",
        Some(bend(&mut d, 10.0)),
        Better::Info,
        None,
        0.0,
    );

    // M6: the chorus's bend at Delay 5 and 40 ms, same Rate and Depth.
    let at_delay = |delay: f64| {
        let mut d = engine(
            CHORUS,
            rate,
            &[(ch::VOICES, 1.0), (ch::MIX, 100.0), (ch::DELAY, delay)],
        );
        bend(&mut d, 0.6)
    };
    let (short, long) = (at_delay(5.0), at_delay(40.0));
    r.put(
        "M6",
        sn,
        "bend_ratio_delay5_to_40",
        Some(short.max(long) / short.min(long).max(1e-6)),
        Better::Lower,
        Some(1.1),
        0.05,
    );

    // M7: held flatness at half a frame against a whole frame (the
    // chorus with Tone open), and the flanger's high notches.
    let n = 1 << 16;
    let held = |frac: f64| {
        let mut d = engine(
            CHORUS,
            rate,
            &[
                (ch::VOICES, 1.0),
                (ch::MIX, 100.0),
                (ch::DEPTH, 0.0),
                (ch::TONE, 20_000.0),
                (ch::DELAY, frames_ms(12.0, frac, rate)),
            ],
        );
        fine(&mut d, n)
    };
    let (whole, half) = (held(0.0), held(0.5));
    let top = (16_000.0 * n as f64 / rate as f64) as usize;
    let lo = (20.0 * n as f64 / rate as f64) as usize;
    let worst = (lo..=top).fold(0.0f64, |m, k| m.max((whole[k] - half[k]).abs()));
    r.put(
        "M7",
        sn,
        "half_frame_db_to_16k",
        Some(worst),
        Better::Lower,
        Some(0.5),
        0.1,
    );
    let mut d = engine(
        FLANGER,
        rate,
        &[
            (fl::DEPTH, 0.0),
            (fl::FEEDBACK, 0.0),
            (fl::MIX, 50.0),
            (fl::WIDTH, 0.0),
            (fl::DELAY, frames_ms(1.5, 0.5, rate)),
        ],
    );
    let resp = fine(&mut d, 1 << 17);
    let bin = |hz: f64| (hz * (1 << 17) as f64 / rate as f64) as usize;
    let notch = resp[bin(12_000.0)..bin(16_000.0)]
        .iter()
        .fold(f64::INFINITY, |m, v| m.min(*v));
    r.put(
        "M7",
        "modulation/flanger",
        "half_frame_notch_db_12k_16k",
        Some(floor(notch)),
        Better::Lower,
        Some(-30.0),
        1.0,
    );

    // M8: a 12 kHz tone through the moving read: energy beyond ±5 %.
    let mut d = engine(
        CHORUS,
        rate,
        &[(ch::VOICES, 1.0), (ch::MIX, 100.0), (ch::TONE, 20_000.0)],
    );
    let len = 1 << 17;
    let x = sine(12_000.0, db_to_amp(-12.0), len + rate as usize, rate);
    let out = d.render(&Stereo::mono(x, rate), 0);
    let seg: Vec<f64> = out.left[rate as usize..rate as usize + len]
        .iter()
        .zip(hann_f64(len))
        .map(|(a, w)| a * w)
        .collect();
    let p = power_spectrum(&seg);
    let b = |hz: f64| (hz * len as f64 / rate as f64) as usize;
    let inside: f64 = p[b(12_000.0 * 0.95)..=b(12_000.0 * 1.05)].iter().sum();
    let outside = p.iter().sum::<f64>() - inside;
    r.put(
        "M8",
        sn,
        "read_sidebands_db_12k",
        Some(floor(db_pow(outside / inside.max(1e-30)))),
        Better::Lower,
        Some(-70.0),
        1.0,
    );

    // M9: drawn against heard, held.
    let n = 1 << 17;
    // Through zero held at the crossing, heard wet (at Mix 50 it is
    // silence): the copy at the dry's wait, round its loop.
    for (frac, fb, tz) in [
        (0.0, 0.0, false),
        (0.5, 0.0, false),
        (0.5, 90.0, false),
        (0.5, -90.0, false),
        (0.0, 90.0, false),
        (0.0, 0.0, true),
        (0.0, 90.0, true),
    ] {
        let values = [
            (fl::DEPTH, 0.0),
            (fl::FEEDBACK, fb),
            (fl::MIX, if tz { 100.0 } else { 50.0 }),
            (fl::WIDTH, 0.0),
            (fl::DELAY, frames_ms(1.5, frac, rate)),
            (fl::THROUGH_ZERO, if tz { 1.0 } else { 0.0 }),
        ];
        let resp = fine(&mut engine(FLANGER, rate, &values), n);
        let mut node = Flanger::new();
        node.prepare(rate, mpfree_fx::mpfree_node::MAX_BLOCK);
        for (k, v) in values {
            node.set_param(k, v as f32);
        }
        let err = drawn_error(&resp, rate, |hz| node.response_db(hz as f32, 0.0) as f64);
        r.put(
            "M9",
            "modulation/flanger",
            format!(
                "drawn_err_db_{}_fb{}{}",
                if tz {
                    "tz"
                } else if frac == 0.0 {
                    "whole"
                } else {
                    "half"
                },
                if fb < 0.0 { "m" } else { "p" },
                fb.abs()
            ),
            Some(err),
            Better::Lower,
            Some(0.5),
            0.1,
        );
    }
    for (stages, center, fb) in [
        (1.0, 700.0, 0.0),
        (1.0, 700.0, 30.0),
        (4.0, 300.0, 0.0),
        (1.0, 700.0, -70.0),
        (1.0, 4000.0, 90.0),
    ] {
        let values = [
            (ph::DEPTH, 0.0),
            (ph::STAGES, stages),
            (ph::CENTER, center),
            (ph::FEEDBACK, fb),
            (ph::MIX, 50.0),
            (ph::WIDTH, 0.0),
        ];
        let resp = fine(&mut engine(PHASER, rate, &values), n);
        let mut node = Phaser::new();
        node.prepare(rate, mpfree_fx::mpfree_node::MAX_BLOCK);
        for (k, v) in values {
            node.set_param(k, v as f32);
        }
        let err = drawn_error(&resp, rate, |hz| node.response_db(hz as f32, 0.0) as f64);
        r.put(
            "M9",
            "modulation/phaser",
            format!(
                "drawn_err_db_st{stages}_c{center}_fb{}{}",
                if fb < 0.0 { "m" } else { "p" },
                fb.abs()
            ),
            Some(err),
            Better::Lower,
            Some(0.5),
            0.1,
        );
    }

    // M10: the flanger's longest swept delay, from its own drawing.
    let mut node = Flanger::new();
    node.prepare(rate, mpfree_fx::mpfree_node::MAX_BLOCK);
    node.set_param(fl::DELAY, 10.0);
    node.set_param(fl::DEPTH, 100.0);
    let longest = (0..=100).fold(0.0f32, |m, k| m.max(node.delay_ms(k as f32 / 100.0)));
    r.put(
        "M10",
        "modulation/flanger",
        "longest_delay_ms",
        Some(longest as f64),
        Better::Lower,
        Some(20.0),
        0.1,
    );

    // M11: a bar-synced LFO after a loop wrap.
    r.put(
        "M11",
        sn,
        "sync_error_cycles_after_wrap",
        Some(sync_error(rate)),
        Better::Lower,
        Some(0.01),
        0.005,
    );

    // M12: feedback at the extremes rings out within the reported tail.
    let noise = Stereo::new(
        white(4 * rate as usize, 0.5, 11),
        white(4 * rate as usize, 0.5, 12),
        rate,
    );
    // An engine, a name for the case, and its control values.
    type Case = (&'static str, &'static str, Vec<(u16, f64)>);
    for fb in [-95.0, 95.0] {
        let cases: [Case; 4] = [
            (
                "flanger",
                "slow_long",
                vec![(fl::RATE, 0.02), (fl::DEPTH, 100.0), (fl::DELAY, 10.0)],
            ),
            (
                "flanger",
                "fast_short",
                vec![(fl::RATE, 10.0), (fl::DEPTH, 100.0), (fl::DELAY, 0.1)],
            ),
            (
                "phaser",
                "slow_low",
                vec![(ph::RATE, 0.02), (ph::CENTER, 100.0), (ph::DEPTH, 100.0)],
            ),
            (
                "phaser",
                "fast_high",
                vec![(ph::RATE, 10.0), (ph::CENTER, 8000.0), (ph::DEPTH, 100.0)],
            ),
        ];
        for (name, tag, mut values) in cases {
            let id = if name == "flanger" { FLANGER } else { PHASER };
            let fb_id = if name == "flanger" {
                fl::FEEDBACK
            } else {
                ph::FEEDBACK
            };
            values.push((fb_id, fb));
            values.push((mix_id(id), 100.0));
            let mut d = engine(id, rate, &values);
            let tail = d.node().tail_frames() as usize;
            let out = d.render(&noise, tail);
            let from = noise.len() + tail * 4 / 5;
            let finite = out.left.iter().chain(&out.right).all(|v| v.is_finite());
            let rest = out.left[from..]
                .iter()
                .chain(&out.right[from..])
                .fold(0.0f64, |m, v| m.max(v.abs()));
            r.put(
                "M12",
                &format!("modulation/{name}"),
                format!(
                    "rest_dbfs_{tag}_fb{}{}",
                    if fb < 0.0 { "m" } else { "p" },
                    fb.abs()
                ),
                Some(if finite {
                    floor(db_amp(rest))
                } else {
                    f64::INFINITY
                }),
                Better::Lower,
                Some(-60.0),
                3.0,
            );
        }
    }
}
