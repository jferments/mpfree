//! Behaviour tests for the electric piano (Keys) through the mpfree-node interface.

use mpfree_node::{check_params, Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::keys_params::*;
use mpfree_synth::Keys;

const RATE: u32 = 48_000;

fn render(
    s: &mut Keys,
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
            &Ctx::new(RATE, at as u64),
            &buf,
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    (l, r)
}

fn on(note: u8, velocity: u8) -> NodeEvent {
    NodeEvent::NoteOn { note, velocity }
}
fn off(note: u8) -> NodeEvent {
    NodeEvent::NoteOff { note }
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

fn preset(name: &str) -> Keys {
    let mut s = Keys::new(RATE);
    let p = Keys::presets()
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no preset {name}"));
    s.load_preset(p);
    s
}

fn hz(note: f32) -> f32 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}

/// The amplitude of the `hz` component of `x` (a Hann-windowed Goertzel).
fn tone(x: &[f32], hz: f32) -> f32 {
    let n = x.len();
    let w = std::f32::consts::TAU * hz / RATE as f32;
    let (mut re, mut im) = (0.0f32, 0.0f32);
    for (i, &v) in x.iter().enumerate() {
        let h = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos();
        re += v * h * (w * i as f32).cos();
        im += v * h * (w * i as f32).sin();
    }
    (re * re + im * im).sqrt() * 4.0 / n as f32
}

#[test]
fn parameters_and_presets_are_valid() {
    check_params(PARAMS).unwrap();
    let mut names = std::collections::HashSet::new();
    for p in Keys::presets() {
        assert!(names.insert(p.name), "{} twice", p.name);
        for &(id, v) in p.values {
            let q = PARAMS
                .iter()
                .find(|q| q.id == id)
                .unwrap_or_else(|| panic!("{}: no param {id}", p.name));
            assert!(v >= q.min && v <= q.max, "{}: {} = {v}", p.name, q.name);
        }
    }
}

/// The frequency of the strongest component within 60 cents of `guess`,
/// by a fine scan of [`tone`] and a parabola through the best three.
fn pitch_near(x: &[f32], guess: f32) -> f32 {
    let at = |c: f32| tone(x, guess * (c / 1200.0).exp2());
    let mut best = (-60.0f32, 0.0f32);
    let mut c = -60.0f32;
    while c <= 60.0 {
        let a = at(c);
        if a > best.1 {
            best = (c, a);
        }
        c += 0.5;
    }
    let (a, b, d) = (at(best.0 - 0.5), best.1, at(best.0 + 0.5));
    let off = 0.25 * (a - d) / (a - 2.0 * b + d);
    guess * ((best.0 + off) / 1200.0).exp2()
}

#[test]
fn both_models_play_in_tune() {
    for name in ["Init", "Barking Tines", "Bell Keys", "Reed", "Reed Bite"] {
        for note in [36u8, 48, 60, 72, 84, 96] {
            let mut s = preset(name);
            let (l, _) = render(&mut s, 28_800, &[(0, on(note, 110))], |_| 128);
            let want = hz(f32::from(note) + s.param(TRANSPOSE));
            let got = pitch_near(&l[4800..28_800], want);
            let cents = 1200.0 * (got / want).log2();
            assert!(
                cents.abs() < 1.0,
                "{name} note {note}: {got:.2} Hz for {want:.2} ({cents:+.1} cents)"
            );
        }
    }
}

#[test]
fn transpose_fine_and_bend_move_the_pitch() {
    let mut s = Keys::new(RATE);
    s.set_param(TRANSPOSE, 7.0);
    s.set_param(FINE, 50.0);
    s.set_param(BEND_RANGE, 12.0);
    let (l, _) = render(
        &mut s,
        24_000,
        &[(0, NodeEvent::Bend { value: -0.5 }), (0, on(60, 100))],
        |_| 64,
    );
    let want = hz(60.0 + 7.0 + 0.5 - 6.0);
    let got = pitch_near(&l[4800..24_000], want);
    let cents = 1200.0 * (got / want).log2();
    assert!(cents.abs() < 3.0, "{got:.2} Hz for {want:.2}");
}

#[test]
fn same_output_for_any_block_size() {
    let mut ev = vec![
        (0, on(48, 100)),
        (5, on(55, 90)),
        (7, on(60, 127)),
        (3_000, NodeEvent::Bend { value: 0.5 }),
        (6_000, off(55)),
        (
            7_000,
            NodeEvent::Param {
                id: TREMOLO,
                value: 80.0,
            },
        ),
        (
            8_000,
            NodeEvent::Param {
                id: TONE,
                value: -60.0,
            },
        ),
        (
            8_500,
            NodeEvent::Param {
                id: DRIVE,
                value: 60.0,
            },
        ),
        (9_000, on(48, 60)),
        (
            12_000,
            NodeEvent::Param {
                id: MODEL,
                value: 1.0,
            },
        ),
        (12_500, on(72, 120)),
        (
            13_000,
            NodeEvent::Param {
                id: TREM_MODE,
                value: 1.0,
            },
        ),
        (14_000, on(79, 80)),
        (
            15_000,
            NodeEvent::Param {
                id: BARK,
                value: 100.0,
            },
        ),
        (16_000, on(64, 127)),
        (20_000, NodeEvent::AllOff),
    ];
    for k in 0..20u64 {
        ev.push((22_000 + k * 97, on(40 + k as u8, 64 + k as u8)));
    }
    ev.sort_by_key(|e| e.0);
    let frames = 40_000;
    let (rl, rr) = render(&mut Keys::new(RATE), frames, &ev, |_| 128);
    assert!(peak(&rl) > 0.01);
    for size in [1usize, 7, 16, 17, 64, 1000, 8192] {
        let (l, r) = render(&mut Keys::new(RATE), frames, &ev, |_| size);
        assert!(l == rl && r == rr, "block size {size} differs");
    }
    let mut seed = 9u64;
    let (l, r) = render(&mut Keys::new(RATE), frames, &ev, |_| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        1 + (seed >> 33) as usize % 700
    });
    assert!(l == rl && r == rr, "random block sizes differ");
}

#[test]
fn every_preset_plays_at_a_sane_level_and_ends() {
    for p in Keys::presets() {
        for note in [36u8, 48, 60, 72, 84] {
            let mut s = Keys::new(RATE);
            s.load_preset(p);
            let (l, r) = render(
                &mut s,
                RATE as usize * 6,
                &[(0, on(note, 127)), (RATE as u64 / 2, off(note))],
                |_| 256,
            );
            let pk = db(peak(&l).max(peak(&r)));
            assert!(
                (-20.0..=-6.0).contains(&pk),
                "{} note {note}: {pk:.1} dBFS",
                p.name
            );
            assert_eq!(
                s.active_voices(),
                0,
                "{} note {note} still sounding",
                p.name
            );
        }
    }
}

#[test]
fn a_hard_chord_stays_under_full_scale() {
    for p in Keys::presets() {
        let mut s = Keys::new(RATE);
        s.load_preset(p);
        let ev: Vec<_> = [36u8, 48, 55, 60, 64, 67]
            .iter()
            .map(|&n| (0, on(n, 127)))
            .collect();
        let (l, r) = render(&mut s, 24_000, &ev, |_| 128);
        let pk = db(peak(&l).max(peak(&r)));
        assert!((-12.0..=0.0).contains(&pk), "{}: {pk:.1} dBFS", p.name);
        println!("chord {}: {pk:.1}", p.name);
    }
}

#[test]
fn a_held_note_rings_and_letting_go_damps_it() {
    let ring = |release: f32, hold: bool| {
        let mut s = Keys::new(RATE);
        s.set_param(RELEASE, release);
        s.set_param(KEY_NOISE, 0.0);
        let ev: &[(u64, NodeEvent)] = if hold {
            &[(0, on(60, 127))]
        } else {
            &[(0, on(60, 127)), (4800, off(60))]
        };
        let (l, _) = render(&mut s, RATE as usize, ev, |_| 128);
        rms(&l[19_200..24_000])
    };
    let held = ring(30.0, true);
    let (short, long) = (ring(30.0, false), ring(3000.0, false));
    assert!(db(held) > -40.0, "held {:.1} dB", db(held));
    assert!(
        db(long) - db(short) > 40.0,
        "release 3 s {:.1} dB vs 30 ms {:.1} dB",
        db(long),
        db(short)
    );
}

#[test]
fn a_ringing_note_is_struck_again_in_its_own_voice() {
    let mut s = Keys::new(RATE);
    let ev: Vec<_> = (0..10u64).map(|k| (k * 2400, on(60, 100))).collect();
    render(&mut s, 26_400, &ev, |_| 128);
    assert_eq!(s.param(METER_VOICES), 1.0);
}

#[test]
fn the_voice_limit_steals_without_clicks() {
    let mut s = Keys::new(RATE);
    s.set_param(POLYPHONY, 4.0);
    let ev: Vec<_> = (0..12u64)
        .map(|k| (k * 1200, on(48 + k as u8 * 2, 100)))
        .collect();
    let (l, _) = render(&mut s, 19_200, &ev, |_| 128);
    assert!(s.param(METER_VOICES) <= 4.0);
    assert!(l.iter().all(|x| x.is_finite()));
    // No step between frames bigger than the notes themselves make.
    let jump = l
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(jump < 0.1, "a jump of {jump}");
}

/// The level of the `k`th harmonic of middle C against the fundamental's,
/// in dB, 100 to 300 ms into a note at `vel`.
fn harmonic_db(s: &mut Keys, vel: u8, k: f32) -> f32 {
    let (l, _) = render(s, 14_400, &[(0, on(60, vel))], |_| 128);
    let x = &l[4800..14_400];
    let f = hz(60.0);
    db(tone(x, f * k)) - db(tone(x, f))
}

#[test]
fn a_hard_hit_barks() {
    let make = || {
        let mut s = Keys::new(RATE);
        s.set_param(VOICING, 30.0);
        s.set_param(BARK, 90.0);
        s
    };
    let soft = harmonic_db(&mut make(), 30, 2.0);
    let hard = harmonic_db(&mut make(), 127, 2.0);
    assert!(
        hard > soft + 6.0,
        "2nd harmonic: hard {hard:.1} dB, soft {soft:.1} dB"
    );
}

#[test]
fn voicing_moves_from_the_octave_to_a_pure_tone() {
    // Centred on the pickup, the octave above the note is stronger than
    // the note; far off centre, the note is nearly pure.
    let second = |voicing: f32| {
        let mut s = Keys::new(RATE);
        s.set_param(VOICING, voicing);
        s.set_param(BARK, 30.0);
        harmonic_db(&mut s, 100, 2.0)
    };
    let (centred, off) = (second(0.0), second(100.0));
    assert!(centred > 3.0, "centred: 2nd harmonic {centred:.1} dB");
    assert!(off < -25.0, "off centre: 2nd harmonic {off:.1} dB");
}

#[test]
fn a_harder_hammer_rings_the_bell() {
    let bell = |hard: f32| {
        let mut s = Keys::new(RATE);
        s.set_param(HARDNESS, hard);
        s.set_param(VEL_TONE, 0.0);
        s.set_param(VOICING, 100.0);
        s.set_param(BARK, 0.0);
        s.set_param(KEY_NOISE, 0.0);
        let (l, _) = render(&mut s, 2400, &[(0, on(60, 127))], |_| 128);
        let x = &l[..1200];
        db(tone(x, hz(60.0) * 7.11)) - db(tone(x, hz(60.0)))
    };
    let (soft, hard) = (bell(0.0), bell(100.0));
    assert!(
        hard > soft + 12.0,
        "bell: hard {hard:.1} dB, soft {soft:.1} dB"
    );
}

#[test]
fn the_tremolo_pans_or_pulses() {
    let run = |mode: f32| {
        let mut s = Keys::new(RATE);
        s.set_param(TREMOLO, 100.0);
        s.set_param(TREM_RATE, 4.0);
        s.set_param(TREM_MODE, mode);
        s.set_param(DECAY, 20000.0);
        render(&mut s, RATE as usize / 2, &[(0, on(48, 100))], |_| 128)
    };
    // Pan: a quarter cycle in (62.5 ms) the sound sits right, at 3/4 left.
    let (l, r) = run(0.0);
    let at = |x: &[f32], ms: usize| rms(&x[ms * 48 - 240..ms * 48 + 240]);
    assert!(at(&r, 62) > at(&l, 62) * 4.0);
    assert!(at(&l, 187) > at(&r, 187) * 4.0);
    // Amp: both sides together, quiet at the top of the cycle.
    let (l, r) = run(1.0);
    assert_eq!(l, r);
    assert!(at(&l, 187) > at(&l, 62) * 4.0);
}

#[test]
fn instruments_add_to_the_buffer() {
    let mut s = Keys::new(RATE);
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
                let mut s = Keys::new(RATE);
                s.set_param(MODEL, model);
                s.set_param(p.id, v);
                let ev = [
                    (0, on(0, 127)),
                    (0, on(127, 127)),
                    (0, on(60, 127)),
                    (100, NodeEvent::Bend { value: f32::NAN }),
                    (2000, off(0)),
                ];
                let (l, r) = render(&mut s, 6000, &ev, |_| 333);
                assert!(
                    l.iter().chain(&r).all(|x| x.is_finite()),
                    "model {model}: {} = {v}",
                    p.name
                );
                assert!(
                    peak(&l).max(peak(&r)) < 2.0,
                    "model {model}: {} = {v}",
                    p.name
                );
            }
        }
    }
}
