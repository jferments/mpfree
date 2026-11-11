//! Behaviour tests for the pluck synth through the mpfree-node interface.

use mpfree_node::{check_params, Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::pluck_params::*;
use mpfree_synth::Pluck;

const RATE: u32 = 48_000;

fn render(s: &mut Pluck, frames: usize, events: &[(u64, NodeEvent)], mut cut: impl FnMut(usize) -> usize) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let (mut at, mut n) = (0usize, 0usize);
    let mut buf = Vec::new();
    while at < frames {
        let len = cut(n).clamp(1, MAX_BLOCK).min(frames - at);
        n += 1;
        buf.clear();
        for &(f, ev) in events {
            if f >= at as u64 && f < (at + len) as u64 { buf.push(Timed::new((f - at as u64) as u32, ev)); }
        }
        s.process(&Ctx::new(RATE, at as u64), &buf, &mut l[at..at + len], &mut r[at..at + len]);
        at += len;
    }
    (l, r)
}

fn on(note: u8, velocity: u8) -> NodeEvent { NodeEvent::NoteOn { note, velocity } }
fn off(note: u8) -> NodeEvent { NodeEvent::NoteOff { note } }
fn peak(x: &[f32]) -> f32 { x.iter().fold(0.0, |a, &v| a.max(v.abs())) }
fn db(x: f32) -> f32 { 20.0 * x.max(1e-12).log10() }
fn rms(x: &[f32]) -> f32 { (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt() }

fn preset(name: &str) -> Pluck {
    let mut s = Pluck::new(RATE);
    let p = Pluck::presets().iter().find(|p| p.name == name).unwrap_or_else(|| panic!("no preset {name}"));
    s.load_preset(p);
    s
}

/// The fundamental by autocorrelation, refined between lags.
fn pitch(x: &[f32]) -> f32 {
    let ac = |lag: usize| x[..x.len() - lag].iter().zip(&x[lag..]).map(|(a, b)| a * b).sum::<f32>() / (x.len() - lag) as f32;
    let rate = RATE as f32;
    // The first lag that comes close to the best, so an octave below
    // (twice the lag) never wins.
    let lags: Vec<(usize, f32)> = ((rate / 5000.0) as usize..(rate / 25.0) as usize).map(|l| (l, ac(l))).collect();
    // Past the first dip below zero, so the near-zero lags don't count.
    let start = lags.iter().position(|x| x.1 < 0.0).unwrap_or(0);
    let lags = &lags[start..];
    let best = lags.iter().map(|x| x.1).fold(f32::MIN, f32::max);
    let mut at = lags.iter().find(|x| x.1 >= 0.9 * best).unwrap().0;
    while at + 1 < (rate / 25.0) as usize && ac(at + 1) > ac(at) { at += 1; }
    let (a, b, c) = (ac(at - 1), ac(at), ac(at + 1));
    rate / (at as f32 + 0.5 * (a - c) / (a - 2.0 * b + c))
}

fn hz(note: f32) -> f32 { 440.0 * ((note - 69.0) / 12.0).exp2() }

#[test]
fn parameters_and_presets_are_valid() {
    check_params(PARAMS).unwrap();
    for p in Pluck::presets() {
        for &(id, v) in p.values {
            let q = PARAMS.iter().find(|q| q.id == id).unwrap_or_else(|| panic!("{}: no param {id}", p.name));
            assert!(v >= q.min && v <= q.max, "{}: {} = {v}", p.name, q.name);
        }
    }
}

#[test]
fn strings_play_in_tune() {
    for name in ["Init", "Nylon", "Steel String", "Koto", "Muted Bass"] {
        for note in [36u8, 48, 60, 72, 84, 96] {
            let mut s = preset(name);
            let (l, _) = render(&mut s, 19_200, &[(0, on(note, 110))], |_| 128);
            let want = hz(f32::from(note) + s.param(TRANSPOSE));
            // At least eight periods.
            let len = (8.0 * RATE as f32 / want).max(4800.0) as usize;
            let got = pitch(&l[1200..1200 + len]);
            let cents = 1200.0 * (got / want).log2();
            // Within 5 cents (8 below 40 Hz, where a dark, damped string
            // is hard to measure in a short window).
            let tol = if want < 40.0 { 8.0 } else { 5.0 };
            assert!(cents.abs() < tol, "{name} note {note}: {got:.2} Hz for {want:.2} ({cents:+.1} cents)");
        }
    }
}

#[test]
fn transpose_fine_and_bend_move_the_pitch() {
    let measure = |s: &mut Pluck, ev: &[(u64, NodeEvent)]| { let (l, _) = render(s, 14_400, ev, |_| 128); pitch(&l[9600..14_400]) };
    let mut s = Pluck::new(RATE);
    s.set_param(TRANSPOSE, 7.0);
    s.set_param(FINE, 50.0);
    let got = measure(&mut s, &[(0, on(60, 100))]);
    assert!((1200.0 * (got / hz(67.5)).log2()).abs() < 5.0, "{got}");
    // A full bend up (2 semitones) while the note rings retunes it.
    let mut s = Pluck::new(RATE);
    let got = measure(&mut s, &[(0, on(60, 100)), (4800, NodeEvent::Bend { value: 1.0 })]);
    assert!((1200.0 * (got / hz(62.0)).log2()).abs() < 8.0, "bent to {got}");
}

#[test]
fn same_output_for_any_block_size() {
    let mut ev = vec![(0, on(48, 100)), (5, on(55, 90)), (7, on(60, 127)), (3_000, NodeEvent::Bend { value: 0.5 }),
                      (6_000, off(55)), (8_000, NodeEvent::Param { id: BRIGHTNESS, value: 20.0 }), (9_000, on(48, 60)),
                      (12_000, NodeEvent::Param { id: MODEL, value: 1.0 }), (12_500, on(72, 120)), (14_000, on(79, 80)),
                      (15_000, NodeEvent::Param { id: MATERIAL, value: 2.0 }), (16_000, on(64, 127)), (20_000, NodeEvent::AllOff)];
    for k in 0..20u64 { ev.push((22_000 + k * 97, on(40 + k as u8, 64 + k as u8))); }
    ev.sort_by_key(|e| e.0);
    let frames = 40_000;
    let (rl, rr) = render(&mut Pluck::new(RATE), frames, &ev, |_| 128);
    assert!(peak(&rl) > 0.01);
    for size in [1usize, 7, 16, 17, 64, 1000, 8192] {
        let (l, r) = render(&mut Pluck::new(RATE), frames, &ev, |_| size);
        assert!(l == rl && r == rr, "block size {size} differs");
    }
    let mut seed = 9u64;
    let (l, r) = render(&mut Pluck::new(RATE), frames, &ev, |_| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        1 + (seed >> 33) as usize % 700
    });
    assert!(l == rl && r == rr, "random block sizes differ");
}

#[test]
fn every_preset_plays_at_a_sane_level_and_ends() {
    for p in Pluck::presets() {
        for note in [36u8, 60, 84] {
            let mut s = Pluck::new(RATE);
            s.load_preset(p);
            let (l, r) = render(&mut s, RATE as usize * 12, &[(0, on(note, 127)), (RATE as u64 / 2, off(note))], |_| 256);
            let pk = db(peak(&l).max(peak(&r)));
            assert!((-20.0..=-6.0).contains(&pk), "{} note {note}: {pk:.1} dBFS", p.name);
            assert_eq!(s.active_voices(), 0, "{} note {note} still sounding", p.name);
        }
    }
}

#[test]
fn letting_go_damps_the_note_over_release() {
    let ring = |release: f32| {
        let mut s = Pluck::new(RATE);
        s.set_param(RELEASE, release);
        let (l, _) = render(&mut s, RATE as usize, &[(0, on(60, 127)), (4800, off(60))], |_| 128);
        rms(&l[19_200..24_000])
    };
    let (short, long) = (ring(30.0), ring(5000.0));
    assert!(db(long) - db(short) > 40.0, "release 5 s {:.1} dB vs 30 ms {:.1} dB", db(long), db(short));
}

#[test]
fn a_ringing_note_is_replucked_in_its_own_voice() {
    let mut s = Pluck::new(RATE);
    let ev: Vec<_> = (0..10u64).map(|k| (k * 2400, on(60, 100))).collect();
    render(&mut s, 26_400, &ev, |_| 128);
    assert_eq!(s.param(METER_VOICES), 1.0);
}

#[test]
fn the_voice_limit_steals_without_clicks() {
    let mut s = Pluck::new(RATE);
    s.set_param(POLYPHONY, 4.0);
    let ev: Vec<_> = (0..12u64).map(|k| (k * 1200, on(48 + k as u8 * 2, 100))).collect();
    let (l, _) = render(&mut s, 19_200, &ev, |_| 128);
    assert!(s.param(METER_VOICES) <= 4.0);
    assert!(l.iter().all(|x| x.is_finite()));
}

#[test]
fn a_harder_mallet_is_brighter() {
    let bright = |hard: f32| {
        let mut s = preset("Marimba");
        s.set_param(HARDNESS, hard);
        s.set_param(VEL_TONE, 0.0);
        let (l, _) = render(&mut s, 9600, &[(0, on(48, 127))], |_| 128);
        // Share of the energy in the first difference: higher means brighter.
        let d: Vec<f32> = l.windows(2).map(|w| w[1] - w[0]).collect();
        rms(&d[..4800]) / rms(&l[..4800])
    };
    assert!(bright(100.0) > bright(0.0) * 1.5, "{} vs {}", bright(100.0), bright(0.0));
}

#[test]
fn mallets_ring_at_their_note() {
    for (name, note) in [("Marimba", 60u8), ("Vibes", 72), ("Kalimba", 72)] {
        let mut s = preset(name);
        let (l, _) = render(&mut s, 14_400, &[(0, on(note, 127))], |_| 128);
        let got = pitch(&l[4800..14_400]);
        let cents = 1200.0 * (got / hz(f32::from(note))).log2();
        assert!(cents.abs() < 5.0, "{name}: {got:.1} Hz ({cents:+.1} cents)");
    }
}

#[test]
fn instruments_add_to_the_buffer() {
    let mut s = Pluck::new(RATE);
    let mut l = vec![0.5f32; 64];
    let mut r = vec![0.5f32; 64];
    s.process(&Ctx::new(RATE, 0), &[], &mut l, &mut r);
    assert!(l.iter().chain(&r).all(|&x| x == 0.5));
}

#[test]
fn extreme_settings_stay_finite() {
    for model in [0.0, 1.0] {
        for p in PARAMS.iter().filter(|p| !p.readonly) {
            for v in [p.min, p.max, f32::NAN] {
                let mut s = Pluck::new(RATE);
                s.set_param(MODEL, model);
                s.set_param(p.id, v);
                let ev = [(0, on(0, 127)), (0, on(127, 127)), (0, on(60, 127)), (100, NodeEvent::Bend { value: f32::NAN }), (2000, off(0))];
                let (l, r) = render(&mut s, 6000, &ev, |_| 333);
                assert!(l.iter().chain(&r).all(|x| x.is_finite()), "model {model}: {} = {v}", p.name);
                assert!(peak(&l) < 4.0, "model {model}: {} = {v}: {}", p.name, peak(&l));
            }
        }
    }
}
