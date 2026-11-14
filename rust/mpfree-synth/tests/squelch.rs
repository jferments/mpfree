//! Behaviour tests for the mono acid bass (Squelch) through the
//! mpfree-node interface.

use mpfree_node::{check_params, Ctx, Node, NodeEvent, Timed, Tuning, MAX_BLOCK};
use mpfree_synth::squelch::Ladder;
use mpfree_synth::squelch_params::*;
use mpfree_synth::Squelch;

const RATE: u32 = 48_000;

fn render_at(
    s: &mut Squelch,
    rate: u32,
    frames: usize,
    events: &[(u64, NodeEvent)],
    mut cut: impl FnMut(usize) -> usize,
) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let (mut at, mut n) = (0usize, 0usize);
    let mut buf = Vec::new();
    while at < frames {
        let len = cut(n).clamp(1, MAX_BLOCK).min(frames - at);
        n += 1;
        buf.clear();
        for &(f, ev) in events {
            if f >= at as u64 && f < (at + len) as u64 {
                buf.push(Timed::new((f - at as u64) as u32, ev));
            }
        }
        s.process(
            &Ctx::new(rate, at as u64),
            &buf,
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    (l, r)
}

fn render(
    s: &mut Squelch,
    frames: usize,
    events: &[(u64, NodeEvent)],
    cut: impl FnMut(usize) -> usize,
) -> (Vec<f32>, Vec<f32>) {
    render_at(s, RATE, frames, events, cut)
}

fn on(note: u8, velocity: u8) -> NodeEvent {
    NodeEvent::NoteOn { note, velocity }
}
fn off(note: u8) -> NodeEvent {
    NodeEvent::NoteOff { note }
}
fn set(id: u16, value: f32) -> NodeEvent {
    NodeEvent::Param { id, value }
}
fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |a, &v| a.max(v.abs()))
}
fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}
fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}
fn ms(t: f32) -> u64 {
    (t * 0.001 * RATE as f32) as u64
}
fn hz(note: f32) -> f32 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}

/// The amplitude of the `hz` component of `x` (a Hann-windowed Goertzel).
fn tone_at(x: &[f32], hz: f32, rate: u32) -> f32 {
    let n = x.len();
    let w = std::f64::consts::TAU * f64::from(hz) / f64::from(rate);
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, &v) in x.iter().enumerate() {
        let h = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
        re += f64::from(v) * h * (w * i as f64).cos();
        im += f64::from(v) * h * (w * i as f64).sin();
    }
    ((re * re + im * im).sqrt() * 4.0 / n as f64) as f32
}

fn tone(x: &[f32], hz: f32) -> f32 {
    tone_at(x, hz, RATE)
}

/// The frequency of the strongest component within `cents` of `guess`.
fn strongest_near(x: &[f32], guess: f32, cents: f32, rate: u32) -> f32 {
    let at = |c: f32| tone_at(x, guess * (c / 1200.0).exp2(), rate);
    let mut best = (-cents, 0.0f32);
    let mut c = -cents;
    while c <= cents {
        let a = at(c);
        if a > best.1 {
            best = (c, a);
        }
        c += 1.0;
    }
    guess * (best.0 / 1200.0).exp2()
}

/// A plain sawtooth through an open filter: no envelope, resonance,
/// accent or drive.
fn open() -> Squelch {
    let mut s = Squelch::new(RATE);
    s.set_param(CUTOFF, 5000.0);
    s.set_param(RESONANCE, 0.0);
    s.set_param(ENV_MOD, 0.0);
    s.set_param(ACCENT, 0.0);
    s.reset();
    s
}

fn preset(name: &str) -> Squelch {
    let mut s = Squelch::new(RATE);
    let p = Squelch::presets()
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no preset {name}"));
    s.load_preset(p);
    s
}

#[test]
fn params_and_presets_are_valid() {
    let s = Squelch::new(RATE);
    check_params(s.params()).unwrap();
    assert_eq!(s.info().id, "mpfree.squelch");
    let names: Vec<_> = Squelch::presets().iter().map(|p| p.name).collect();
    assert_eq!(names[0], "Init");
    for p in Squelch::presets() {
        for &(id, v) in p.values {
            let info = PARAMS
                .iter()
                .find(|i| i.id == id)
                .unwrap_or_else(|| panic!("{}: unknown id {id}", p.name));
            assert!(
                v >= info.min && v <= info.max,
                "{}: {} = {v} out of range",
                p.name,
                info.name
            );
        }
    }
}

#[test]
fn silent_until_a_note_and_adds_into_the_buffers() {
    let mut s = Squelch::new(RATE);
    let (l, r) = render(&mut s, 4096, &[], |_| 128);
    assert!(l.iter().chain(&r).all(|&x| x == 0.0));
    assert_eq!(s.active_voices(), 0);
    let mut s = Squelch::new(RATE);
    let mut l = vec![0.25f32; 256];
    let mut r = vec![0.25f32; 256];
    s.process(&Ctx::new(RATE, 0), &[], &mut l, &mut r);
    assert!(l.iter().chain(&r).all(|&x| x == 0.25));
}

#[test]
fn plays_in_tune_from_low_to_high() {
    for note in [24u8, 36, 48, 60, 72] {
        let mut s = open();
        let (l, _) = render(&mut s, 24_000, &[(0, on(note, 90))], |_| 128);
        let f = hz(f32::from(note));
        let got = strongest_near(&l[4800..], f, 40.0, RATE);
        let cents = 1200.0 * (got / f).log2();
        assert!(cents.abs() < 2.0, "note {note}: {cents} cents");
    }
}

#[test]
fn same_output_for_any_block_size() {
    let ev = vec![
        (100, set(RESONANCE, 85.0)),
        (300, on(45, 120)),
        (ms(90.0), on(48, 90)),
        (ms(100.0), off(45)),
        (ms(170.0), off(48)),
        (ms(200.0), on(52, 127)),
        (ms(260.0), off(52)),
        (ms(300.0), on(40, 127)),
        (ms(330.0), NodeEvent::Bend { value: 0.5 }),
        (ms(400.0), off(40)),
    ];
    let mut a = preset("Acid Line");
    let mut b = preset("Acid Line");
    let mut c = preset("Acid Line");
    let x = render(&mut a, 24_000, &ev, |_| 128);
    let y = render(&mut b, 24_000, &ev, |n| [1, 7, 64, 3, 128, 33][n % 6]);
    let z = render(&mut c, 24_000, &ev, |_| 1);
    assert!(peak(&x.0) > 0.01);
    assert_eq!(x, y);
    assert_eq!(x, z);
}

/// The ladder's small-signal response against Stinchcombe's transfer
/// function, H(s) = 1 / (s⁴ + 6.727 s³ + 14.142 s² + 9.514 s + 1), at the
/// normalised frequency (which our cutoff puts 2^¼ below the resonance).
#[test]
fn the_ladder_matches_the_published_transfer_function() {
    let fs = 96_000.0f32;
    let fc = 1000.0f32;
    let g = (std::f32::consts::PI * fc / std::f32::consts::SQRT_2 / fs).tan();
    let w0 = fc / 2f32.powf(0.25); // the normalised 1 rad/s, in Hz
    for ratio in [0.25f32, 0.5, 1.0, 2.0, 4.0] {
        let f = w0 * ratio;
        let mut lad = Ladder::default();
        let n = 96_000;
        let mut out = vec![0.0f32; n];
        for (i, o) in out.iter_mut().enumerate() {
            let x = 1e-3 * (std::f32::consts::TAU * f * i as f32 / fs).sin();
            *o = lad.run(x, g, 0.0, 1.0);
        }
        let got = tone_at(&out[n / 2..], f, 96_000) / 1e-3;
        let s = num_complex(ratio);
        let want = 1.0 / s;
        let err = db(got) - db(want);
        assert!(
            err.abs() < 0.5,
            "at {ratio} x: {:.2} dB, want {:.2} dB",
            db(got),
            db(want)
        );
    }
}

/// |s⁴ + 6.727 s³ + 14.142 s² + 9.514 s + 1| at s = jw.
fn num_complex(w: f32) -> f32 {
    let re = w.powi(4) - 14.142 * w * w + 1.0;
    let im = -6.727 * w.powi(3) + 9.514 * w;
    (re * re + im * im).sqrt()
}

#[test]
fn the_ladder_oscillates_past_17_and_not_below() {
    let fs = 96_000.0f32;
    let fc = 1000.0f32;
    let g = (std::f32::consts::PI * fc / std::f32::consts::SQRT_2 / fs).tan();
    // The loop high-pass far below, so the ladder alone decides.
    let h = 1.0 / (1.0 + (std::f32::consts::PI * 5.0 / fs).tan());
    let ring = |k: f32| {
        let mut lad = Ladder::default();
        let mut out = vec![0.0f32; 96_000];
        for (i, o) in out.iter_mut().enumerate() {
            let x = if i < 10 { 0.1 } else { 0.0 };
            *o = lad.run(x, g, k, h);
        }
        out
    };
    let grows = ring(17.6);
    let dies = ring(16.0);
    // It settles where the diodes' compression has taken the 3 % excess.
    assert!(rms(&grows[86_000..]) > 0.005, "{}", rms(&grows[86_000..]));
    assert!(rms(&grows[86_000..]) < rms(&grows[48_000..58_000]) * 1.2);
    assert!(rms(&dies[86_000..]) < 1e-5, "{}", rms(&dies[86_000..]));
    let f = strongest_near(&grows[48_000..], fc, 150.0, 96_000);
    assert!((f / fc - 1.0).abs() < 0.06, "oscillates at {f} Hz");
}

#[test]
fn resonance_lifts_the_harmonic_at_the_cutoff_but_spares_the_bass() {
    // The harmonic nearest the cutoff against the fundamental, with and
    // without resonance. High in the filter it comes up by 30 dB; near
    // the bottom the loop's high-pass leaves the feedback little to do.
    let lift = |cutoff: f32| {
        let level = |reso: f32| {
            let mut s = Squelch::new(RATE);
            s.set_param(CUTOFF, cutoff);
            s.set_param(RESONANCE, reso);
            s.set_param(ENV_MOD, 0.0);
            s.set_param(ACCENT, 0.0);
            let (l, _) = render(&mut s, 48_000, &[(0, on(24, 90))], |_| 128);
            let f = hz(24.0);
            let n = (cutoff / f).round();
            db(tone(&l[24_000..], f * n)) - db(tone(&l[24_000..], f))
        };
        level(100.0) - level(0.0)
    };
    let high = lift(1200.0);
    let mid = lift(300.0);
    let low = lift(80.0);
    assert!(high > 25.0, "high {high}");
    assert!(mid > 15.0, "mid {mid}");
    assert!(low < 8.0, "low {low}");
}

#[test]
fn the_filter_cuts_steeply_above_the_cutoff() {
    // The saw's harmonics an octave apart above a low cutoff: four poles
    // and the saw's own 1/n make about 30 dB an octave.
    let mut s = Squelch::new(RATE);
    s.set_param(CUTOFF, 300.0);
    s.set_param(RESONANCE, 0.0);
    s.set_param(ENV_MOD, 0.0);
    s.set_param(ACCENT, 0.0);
    let (l, _) = render(&mut s, 48_000, &[(0, on(33, 90))], |_| 128);
    let f = hz(33.0);
    let a = tone(&l[24_000..], f * 16.0);
    let b = tone(&l[24_000..], f * 32.0);
    let fall = db(a) - db(b);
    assert!((24.0..36.0).contains(&fall), "{fall} dB an octave");
}

#[test]
fn the_envelope_sweeps_the_filter_down_over_decay() {
    let mut s = Squelch::new(RATE);
    s.set_param(CUTOFF, 200.0);
    s.set_param(ENV_MOD, 100.0);
    s.set_param(DECAY, 400.0);
    s.set_param(ACCENT, 0.0);
    let mut cut = Vec::new();
    let mut l = vec![0.0f32; 128];
    let mut r = vec![0.0f32; 128];
    for b in 0..200u64 {
        let ev = if b == 0 {
            vec![Timed::new(0, on(36, 90))]
        } else {
            vec![]
        };
        s.process(&Ctx::new(RATE, b * 128), &ev, &mut l, &mut r);
        cut.push(s.cutoff_hz());
    }
    // Up 4 octaves from a base a quarter of that (an octave) lower.
    let top = cut[1];
    assert!((top / 1600.0 - 1.0).abs() < 0.1, "top {top}");
    // At 400 ms (150 blocks) the envelope is at a tenth: 0.4 octave up.
    let at = cut[150];
    let want = 100.0 * 0.4f32.exp2();
    assert!((at / want - 1.0).abs() < 0.08, "{at} want {want}");
}

/// The cutoff every control step for `frames` after a note at `velocity`.
fn cutoffs(s: &mut Squelch, velocity: u8, frames: u64) -> Vec<f32> {
    let mut l = vec![0.0f32; 16];
    let mut r = vec![0.0f32; 16];
    (0..frames / 16)
        .map(|b| {
            let ev = if b == 0 {
                vec![Timed::new(0, on(36, velocity))]
            } else {
                vec![]
            };
            s.process(&Ctx::new(RATE, b * 16), &ev, &mut l, &mut r);
            s.cutoff_hz()
        })
        .collect()
}

#[test]
fn accented_notes_are_louder_and_sweep_the_filter() {
    let play = |velocity: u8| {
        let mut s = preset("Acid Line");
        let (l, _) = render(&mut s, 9600, &[(0, on(36, velocity))], |_| 128);
        l
    };
    let soft = play(90);
    let hard = play(127);
    assert!(
        db(peak(&hard)) > db(peak(&soft)) + 4.0,
        "{} vs {}",
        db(peak(&hard)),
        db(peak(&soft))
    );
    // At high resonance the accent is a smooth swell of the cutoff: 80 ms
    // in, it holds the filter higher even though the accented envelope
    // falls faster.
    let soft = cutoffs(&mut preset("Acid Line"), 90, ms(100.0));
    let hard = cutoffs(&mut preset("Acid Line"), 127, ms(100.0));
    let at = (ms(80.0) / 16) as usize;
    let up = (hard[at] / soft[at]).log2();
    assert!(up > 0.3, "{up} octaves");
    // Accent at decides.
    let mut s = preset("Acid Line");
    s.set_param(ACCENT_AT, 127.0);
    let (l, _) = render(&mut s, 9600, &[(0, on(36, 120))], |_| 128);
    assert!((db(peak(&l)) - db(peak(&play(90)))).abs() < 1.0);
}

#[test]
fn accents_close_together_build_up_at_high_resonance() {
    let peaks = |reso: f32| {
        let mut s = Squelch::new(RATE);
        s.set_param(RESONANCE, reso);
        s.set_param(ENV_MOD, 0.0);
        s.set_param(ACCENT, 100.0);
        let step = ms(125.0);
        let mut peaks = vec![0.0f32; 4];
        let mut l = vec![0.0f32; 16];
        let mut r = vec![0.0f32; 16];
        for b in 0..(4 * step / 16) {
            let f = b * 16;
            let mut ev = Vec::new();
            if f % step == 0 {
                ev.push(Timed::new(0, on(36, 127)));
            }
            if f % step == ms(70.0) / 16 * 16 {
                ev.push(Timed::new(0, off(36)));
            }
            s.process(&Ctx::new(RATE, f), &ev, &mut l, &mut r);
            let i = (f / step) as usize;
            peaks[i] = peaks[i].max(s.cutoff_hz());
        }
        peaks
    };
    let smooth = peaks(100.0);
    assert!(smooth[3] > smooth[0] * 1.15, "{smooth:?}");
    let sharp = peaks(0.0);
    assert!((sharp[3] / sharp[0] - 1.0).abs() < 0.05, "{sharp:?}");
    // With no resonance the sweep is the envelope itself: 2 octaves.
    assert!((sharp[0] / 400.0 - 4.0).abs() < 0.3, "{sharp:?}");
}

#[test]
fn a_note_played_while_another_is_held_slides_without_new_envelopes() {
    let mut s = preset("Acid Line");
    let mut l = vec![0.0f32; 16];
    let mut r = vec![0.0f32; 16];
    let mut pitch = Vec::new();
    let mut cut = Vec::new();
    for b in 0..(ms(400.0) / 16) {
        let f = b * 16;
        let mut ev = Vec::new();
        if f == 0 {
            ev.push(Timed::new(0, on(36, 90)));
        }
        if f == ms(200.0) / 16 * 16 {
            ev.push(Timed::new(0, on(48, 90)));
        }
        s.process(&Ctx::new(RATE, f), &ev, &mut l, &mut r);
        pitch.push(s.pitch());
        cut.push(s.cutoff_hz());
    }
    let at = |t: f32| (ms(t) / 16) as usize;
    assert_eq!(pitch[at(190.0)], 36.0);
    // 60 ms is three time constants: about 95 % there.
    let p = pitch[at(260.0)];
    assert!((47.0..48.0).contains(&p), "{p}");
    assert!(pitch[at(230.0)] > 37.0 && pitch[at(230.0)] < 46.0);
    assert!((pitch[at(390.0)] - 48.0).abs() < 1e-3);
    // The filter envelope carries on falling: no jump at the slide.
    assert!(cut[at(205.0)] <= cut[at(195.0)] * 1.001);
    // Letting go of the top key glides back to the one held.
    let ev = [Timed::new(0, off(48))];
    s.process(&Ctx::new(RATE, ms(400.0) / 16 * 16), &ev, &mut l, &mut r);
    for b in 1..(ms(200.0) / 16) {
        s.process(
            &Ctx::new(RATE, ms(400.0) / 16 * 16 + b * 16),
            &[],
            &mut l,
            &mut r,
        );
    }
    assert!((s.pitch() - 36.0).abs() < 1e-3);
}

#[test]
fn a_detached_note_jumps_and_starts_the_envelopes_again() {
    let mut s = preset("Acid Line");
    let ev = [
        (0, on(36, 90)),
        (ms(100.0), off(36)),
        (ms(150.0), on(48, 90)),
    ];
    let mut l = vec![0.0f32; 16];
    let mut r = vec![0.0f32; 16];
    let mut cut = Vec::new();
    let mut pitch = Vec::new();
    for b in 0..(ms(200.0) / 16) {
        let f = b * 16;
        let e: Vec<_> = ev
            .iter()
            .filter(|(t, _)| *t >= f && *t < f + 16)
            .map(|&(t, e)| Timed::new((t - f) as u32, e))
            .collect();
        s.process(&Ctx::new(RATE, f), &e, &mut l, &mut r);
        cut.push(s.cutoff_hz());
        pitch.push(s.pitch());
    }
    let at = |t: f32| (ms(t) / 16) as usize;
    assert!((pitch[at(152.0)] - 48.0).abs() < 1e-3);
    assert!(cut[at(152.0)] > cut[at(148.0)] * 2.0);
}

#[test]
fn letting_go_ends_in_exact_silence() {
    let mut s = preset("Acid Line");
    let (l, r) = render(
        &mut s,
        24_000,
        &[(0, on(36, 127)), (ms(200.0), off(36))],
        |_| 128,
    );
    assert!(peak(&l[..9600]) > 0.05);
    // 8 ms to close, then the tails (the DC blocker's the longest).
    let after = ms(450.0) as usize;
    assert!(l[after..].iter().chain(&r[after..]).all(|&x| x == 0.0));
    assert_eq!(s.active_voices(), 0);
    assert_eq!(s.keys_held(), 0);
}

#[test]
fn every_preset_sits_near_the_level_convention() {
    for p in Squelch::presets() {
        let mut s = Squelch::new(RATE);
        s.load_preset(p);
        let (l, r) = render(&mut s, 24_000, &[(0, on(36, 90))], |_| 128);
        let pk = db(peak(&l).max(peak(&r)));
        assert!((-20.0..-6.0).contains(&pk), "{}: {pk} dBFS", p.name);
        let mut s = Squelch::new(RATE);
        s.load_preset(p);
        let ev: Vec<_> = (0..8u64)
            .flat_map(|i| {
                let n = [36u8, 48, 39, 36, 43, 36, 51, 46][i as usize];
                [
                    (i * ms(125.0), on(n, 127)),
                    (i * ms(125.0) + ms(80.0), off(n)),
                ]
            })
            .collect();
        let (l, r) = render(&mut s, 48_000, &ev, |_| 128);
        let pk = db(peak(&l).max(peak(&r)));
        assert!(pk < -1.0, "{} accented: {pk} dBFS", p.name);
    }
}

#[test]
fn finite_at_the_extremes() {
    let mut s = Squelch::new(RATE);
    for (id, v) in [
        (CUTOFF, 5000.0),
        (RESONANCE, 100.0),
        (ENV_MOD, 100.0),
        (ACCENT, 100.0),
        (DRIVE, 100.0),
        (KEY_TRACK, 100.0),
        (TRANSPOSE, 24.0),
    ] {
        s.set_param(id, v);
    }
    let ev = [
        (0, on(127, 127)),
        (ms(50.0), on(0, 127)),
        (ms(100.0), NodeEvent::Bend { value: 1.0 }),
        (ms(150.0), set(CUTOFF, 30.0)),
        (ms(200.0), NodeEvent::AllOff),
    ];
    let (l, r) = render(&mut s, 24_000, &ev, |_| 128);
    assert!(l.iter().chain(&r).all(|x| x.is_finite() && x.abs() < 4.0));
}

#[test]
fn in_tune_and_at_the_same_level_at_other_rates() {
    for rate in [44_100u32, 96_000] {
        let mut s = Squelch::new(rate);
        let frames = rate as usize;
        let (l, _) = render_at(&mut s, rate, frames, &[(0, on(48, 90))], |_| 128);
        let f = hz(48.0);
        let got = strongest_near(&l[frames / 4..], f, 40.0, rate);
        assert!((1200.0 * (got / f).log2()).abs() < 2.0, "{rate}: {got}");
        let mut b = Squelch::new(RATE);
        let (m, _) = render(&mut b, 48_000, &[(0, on(48, 90))], |_| 128);
        let d = db(rms(&l[frames / 4..])) - db(rms(&m[12_000..]));
        assert!(d.abs() < 1.0, "{rate}: {d} dB from 48 kHz");
    }
}

#[test]
fn follows_the_project_tuning() {
    let mut s = open();
    let t = Tuning::equal(12, 432.0);
    s.set_tuning(&t);
    let (l, _) = render(&mut s, 24_000, &[(0, on(57, 90))], |_| 128);
    let got = strongest_near(&l[4800..], 216.0, 40.0, RATE);
    assert!((1200.0 * (got / 216.0).log2()).abs() < 2.0, "{got}");
}

#[test]
fn reset_stops_everything() {
    let mut s = preset("Acid Line");
    render(&mut s, 4800, &[(0, on(36, 127))], |_| 128);
    s.reset();
    assert_eq!(s.active_voices(), 0);
    let (l, _) = render(&mut s, 4800, &[], |_| 128);
    assert!(l.iter().all(|&x| x == 0.0));
}

#[test]
fn drive_adds_harmonics_at_about_the_same_level() {
    let play = |drive: f32| {
        let mut s = Squelch::new(RATE);
        s.set_param(CUTOFF, 300.0);
        s.set_param(ENV_MOD, 0.0);
        s.set_param(RESONANCE, 0.0);
        s.set_param(DRIVE, drive);
        let (l, _) = render(&mut s, 24_000, &[(0, on(36, 90))], |_| 128);
        l[12_000..].to_vec()
    };
    let clean = play(0.0);
    let hot = play(80.0);
    let f = hz(36.0);
    // Harmonics 6 to 25 (0.4 to 1.6 kHz), above the cutoff: the filter
    // leaves little there, the drive makes more.
    let band = |x: &[f32]| {
        let e: f32 = (6..=25).map(|n| tone(x, f * n as f32).powi(2)).sum();
        db(e.sqrt() / tone(x, f))
    };
    assert!(
        band(&hot) > band(&clean) + 6.0,
        "{} {}",
        band(&hot),
        band(&clean)
    );
    // About the same peak; denser, so louder on average.
    let d = db(peak(&hot)) - db(peak(&clean));
    assert!(d.abs() < 6.0, "peak {d} dB");
    let d = db(rms(&hot)) - db(rms(&clean));
    assert!(d.abs() < 9.0, "rms {d} dB");
}

#[test]
fn the_square_has_uneven_halves_and_a_sharp_reset() {
    let mut s = open();
    s.set_param(WAVE, SQUARE);
    s.reset();
    let (l, _) = render(&mut s, 24_000, &[(0, on(36, 90))], |_| 128);
    let x = &l[12_000..];
    let f = hz(36.0);
    // Even harmonics come through (a 50 % square has none).
    let even = tone(x, 2.0 * f) / tone(x, f);
    assert!(even > 0.05 && even < 0.5, "{even}");
}

#[test]
#[ignore = "prints levels; run with --ignored --nocapture"]
fn print_preset_levels() {
    for p in Squelch::presets() {
        for vel in [90u8, 127] {
            let mut s = Squelch::new(RATE);
            s.load_preset(p);
            let (l, _) = render(&mut s, 24_000, &[(0, on(36, vel))], |_| 128);
            println!(
                "{:<12} vel {vel:>3} peak {:6.1} dB  rms(0.5 s) {:6.1} dB",
                p.name,
                db(peak(&l)),
                db(rms(&l[..24_000]))
            );
        }
    }
}
