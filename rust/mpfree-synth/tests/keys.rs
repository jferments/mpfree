//! Behaviour tests for the electric piano (Keys) through the mpfree-node interface.

use mpfree_node::{check_params, Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::keys::{clav_voicing, pickup_response, PICKUP_AT};
use mpfree_synth::keys_params::*;
use mpfree_synth::voice::velocity_gain;
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
fn every_model_plays_in_tune() {
    for name in [
        "Init",
        "Barking Tines",
        "Bell Keys",
        "Reed",
        "Reed Bite",
        "Clav",
        "Funk Clav",
        "Muted Clav",
        "Thin Clav",
    ] {
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
        (
            30_000,
            NodeEvent::Param {
                id: MODEL,
                value: CLAV,
            },
        ),
        (30_100, on(36, 127)),
        (30_200, on(60, 90)),
        (31_000, NodeEvent::Bend { value: -0.7 }),
        (
            32_000,
            NodeEvent::Param {
                id: PICKUP,
                value: OUT_OF_PHASE,
            },
        ),
        (33_000, off(60)),
        (
            34_000,
            NodeEvent::Param {
                id: MUTE,
                value: 60.0,
            },
        ),
        (35_000, off(36)),
        (36_000, on(72, 127)),
    ];
    for k in 0..20u64 {
        ev.push((22_000 + k * 97, on(40 + k as u8, 64 + k as u8)));
    }
    ev.sort_by_key(|e| e.0);
    let frames = 40_000;
    let (rl, rr) = render(&mut Keys::new(RATE), frames, &ev, |_| 128);
    assert!(peak(&rl[30_100..]) > 0.01, "the clav plays");
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
            // The clav's string pulses peak higher for the same loudness.
            let top = if s.param(MODEL) == CLAV { -3.0 } else { -6.0 };
            assert!(
                (-20.0..=top).contains(&pk),
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
        // The presets are matched by loudness, so a driven one, which
        // peaks little over its loudness, sits low here.
        assert!((-20.0..=0.0).contains(&pk), "{}: {pk:.1} dBFS", p.name);
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

/// The strongest component of `x` from `lo` to `hi` times `f0`, against
/// `f0`'s, dB.
fn band_db(x: &[f32], f0: f32, lo: f32, hi: f32) -> f32 {
    let mut best = 0.0f32;
    let mut r = lo;
    while r <= hi {
        best = best.max(tone(x, f0 * r));
        r += 0.01;
    }
    db(best) - db(tone(x, f0))
}

/// A tine's bending modes in the first 25 ms of `note` at `vel`, with the
/// pickup kept nearly straight (off centre, no bark) so all above the note
/// is the tine's own: the bell's level and the third mode's (the tick),
/// dB against the note.
fn tine_modes(note: u8, vel: u8, set: &[(u16, f32)]) -> (f32, f32) {
    let mut s = Keys::new(RATE);
    s.set_param(VOICING, 100.0);
    s.set_param(BARK, 0.0);
    s.set_param(KEY_NOISE, 0.0);
    for &(id, v) in set {
        s.set_param(id, v);
    }
    let (l, _) = render(&mut s, 1200, &[(0, on(note, vel))], |_| 128);
    let f = hz(f32::from(note));
    // The modes' ratios fall in the top octaves.
    let (bell, tick) = if note >= 84 {
        ((5.8, 7.0), (16.0, 19.0))
    } else {
        ((6.6, 8.0), (16.0, 21.0))
    };
    let t = if f * tick.1 < 0.45 * RATE as f32 {
        band_db(&l, f, tick.0, tick.1)
    } else {
        f32::NAN
    };
    (band_db(&l, f, bell.0, bell.1), t)
}

#[test]
fn a_harder_hammer_rings_the_bell() {
    // Up to 6 dB each way on the bell (here about 10 between the ends), and
    // 10 dB each way more on the tick above it (about 30).
    let soft = tine_modes(60, 127, &[(HARDNESS, 0.0)]);
    let hard = tine_modes(60, 127, &[(HARDNESS, 100.0)]);
    assert!(
        hard.0 > soft.0 + 7.0,
        "bell: hard {:.1} dB, soft {:.1} dB",
        hard.0,
        soft.0
    );
    assert!(
        hard.1 > soft.1 + 22.0,
        "tick: hard {:.1} dB, soft {:.1} dB",
        hard.1,
        soft.1
    );
}

#[test]
fn the_bell_sits_where_measured() {
    // The measured tine piano: the bell about 30 dB under the note on a
    // hard hit through the middle, a few dB less on a soft one, falling
    // away over C6.
    let c4 = tine_modes(60, 127, &[]).0;
    let soft = tine_modes(60, 30, &[]).0;
    let f2 = tine_modes(41, 127, &[]).0;
    let g6 = tine_modes(91, 127, &[]).0;
    assert!((-33.0..-26.0).contains(&c4), "C4 bell {c4:.1} dB");
    assert!((-34.0..-26.0).contains(&f2), "F2 bell {f2:.1} dB");
    assert!(
        (2.0..8.0).contains(&(c4 - soft)),
        "C4 bell: hard {c4:.1} dB, soft {soft:.1} dB"
    );
    assert!(g6 < c4 - 10.0, "G6 bell {g6:.1} dB against C4's {c4:.1}");
}

#[test]
fn bark_changes_the_colour_not_the_level() {
    // Middle C at full velocity, 50 to 300 ms in: its level, and its 2nd
    // harmonic against the note.
    let run = |bark: f32| {
        let mut s = Keys::new(RATE);
        s.set_param(BARK, bark);
        s.set_param(KEY_NOISE, 0.0);
        let (l, _) = render(&mut s, 14_400, &[(0, on(60, 127))], |_| 128);
        let x = &l[2400..14_400];
        let f = hz(60.0);
        (db(rms(x)), db(tone(x, 2.0 * f)) - db(tone(x, f)))
    };
    let (clean, barking) = (run(0.0), run(100.0));
    assert!(
        (clean.0 - barking.0).abs() < 3.0,
        "level: Bark 0 {:.1} dB, Bark 100 {:.1} dB",
        clean.0,
        barking.0
    );
    assert!(
        barking.1 > clean.1 + 30.0,
        "2nd harmonic: Bark 0 {:.1} dB, Bark 100 {:.1} dB",
        clean.1,
        barking.1
    );
}

#[test]
fn a_soft_tine_is_round_and_a_hard_one_growls() {
    // As measured: the softest hits nearly pure, the hardest with the
    // octave over the note.
    let soft = harmonic_db(&mut Keys::new(RATE), 30, 2.0);
    let hard = harmonic_db(&mut Keys::new(RATE), 127, 2.0);
    assert!(soft < -15.0, "soft: 2nd harmonic {soft:.1} dB");
    assert!(hard > 0.0, "hard: 2nd harmonic {hard:.1} dB");
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

/// The pitch, in semitones against `note`, of a clav note's last moments:
/// held half a second, then 10 to 50 ms after it is let go.
fn clav_drop(drop: f32, note: u8) -> f32 {
    let mut s = preset("Clav");
    s.set_param(DROP, drop);
    s.set_param(RELEASE, 400.0);
    let (l, _) = render(
        &mut s,
        27_000,
        &[(0, on(note, 100)), (24_000, off(note))],
        |_| 128,
    );
    let want = hz(f32::from(note) - drop);
    let got = pitch_near(&l[24_480..26_400], want);
    12.0 * (got / hz(f32::from(note))).log2()
}

#[test]
fn a_clav_note_drops_when_let_go() {
    for note in [36u8, 48, 60, 72] {
        let st = clav_drop(3.0, note);
        assert!((st + 3.0).abs() < 0.5, "note {note}: {st:+.2} semitones");
    }
    let st = clav_drop(0.0, 60);
    assert!(st.abs() < 0.1, "no drop: {st:+.2} semitones");
    let st = clav_drop(6.0, 48);
    assert!((st + 6.0).abs() < 0.5, "6: {st:+.2} semitones");
}

/// The level of partial `k` of a clav C2 against its neighbours', dB.
fn clav_notch(pickup: f32, k: f32) -> f32 {
    let mut s = preset("Clav");
    s.set_param(PICKUP, pickup);
    s.set_param(CLAV_TONE, BRILLIANT);
    s.set_param(KEY_NOISE, 0.0);
    let (l, _) = render(&mut s, 33_600, &[(0, on(36, 50))], |_| 128);
    let x = &l[4800..33_600];
    let f = hz(36.0);
    let at = |k: f32| db(tone(x, f * k));
    at(k) - 0.5 * (at(k - 1.0) + at(k + 1.0))
}

#[test]
fn the_clav_pickups_notch_where_they_sit() {
    // Each pickup hears nothing of the partials that have a node over it:
    // the neck pickup a fifth of the way along, every fifth; the bridge
    // pickup every fourteenth.
    let neck = clav_notch(NECK, 1.0 / PICKUP_AT[0]);
    let bridge = clav_notch(BRIDGE, (1.0 / PICKUP_AT[1]).round());
    assert!(neck < -15.0, "neck: 5th partial {neck:.1} dB");
    assert!(bridge < -15.0, "bridge: 14th partial {bridge:.1} dB");
    // Both pickups together fill in each other's notches.
    let both = clav_notch(BOTH, 5.0);
    assert!(both > -10.0, "both: 5th partial {both:.1} dB");
    // The picture on the Synth tab says the same.
    assert!(pickup_response(NECK, 5.0) < 1e-3);
    assert!(pickup_response(BRIDGE, 14.0) < 1e-3);
    assert!(pickup_response(BOTH, 5.0) > 0.1);
}

#[test]
fn out_of_phase_is_thin() {
    // Out of phase, what the pickups share cancels: the fundamental most.
    let low = |pickup: f32| {
        let mut s = preset("Clav");
        s.set_param(PICKUP, pickup);
        s.set_param(KEY_NOISE, 0.0);
        let (l, _) = render(&mut s, 14_400, &[(0, on(48, 80))], |_| 128);
        let x = &l[4800..14_400];
        let f = hz(48.0);
        let mid = (3..=5).map(|k| db(tone(x, f * k as f32))).sum::<f32>() / 3.0;
        db(tone(x, f)) - mid
    };
    let (both, oop) = (low(BOTH), low(OUT_OF_PHASE));
    assert!(
        oop < both - 3.0,
        "fundamental against the 3rd to 5th: both {both:.1} dB, out of phase {oop:.1} dB"
    );
}

#[test]
fn the_clav_switches_change_the_colour_not_the_level() {
    let level = |pickup: f32, tone: f32| {
        let mut s = preset("Clav");
        s.set_param(PICKUP, pickup);
        s.set_param(CLAV_TONE, tone);
        s.set_param(KEY_NOISE, 0.0);
        let (l, _) = render(&mut s, 24_000, &[(0, on(48, 80))], |_| 128);
        db(rms(&l[2400..24_000]))
    };
    let base = level(BOTH, MEDIUM);
    for p in [NECK, BRIDGE, OUT_OF_PHASE] {
        let d = level(p, MEDIUM) - base;
        assert!(d.abs() < 3.0, "pickup {p}: {d:+.1} dB");
    }
    for t in [SOFT, TREBLE, BRILLIANT] {
        let d = level(BOTH, t) - base;
        assert!(d.abs() < 3.0, "tone {t}: {d:+.1} dB");
    }
    // The voicing makes up what a switch takes away, and no more than 12 dB.
    for p in [NECK, BRIDGE, BOTH, OUT_OF_PHASE] {
        for t in [SOFT, MEDIUM, TREBLE, BRILLIANT] {
            for n in [24.0, 60.0, 96.0] {
                let g = clav_voicing(p, t, hz(n));
                assert!(
                    g.is_finite() && (0.1..=4.0).contains(&g),
                    "{p} {t} {n}: {g}"
                );
            }
        }
    }
}

#[test]
fn the_mute_shortens_and_darkens() {
    let run = |mute: f32| {
        let mut s = preset("Clav");
        s.set_param(MUTE, mute);
        s.set_param(KEY_NOISE, 0.0);
        let (l, _) = render(&mut s, 28_800, &[(0, on(48, 100))], |_| 128);
        let early = &l[2400..7200];
        let bright = db(tone(early, hz(48.0) * 6.0)) - db(tone(early, hz(48.0)));
        let ring = db(rms(&l[24_000..28_800])) - db(rms(early));
        (bright, ring)
    };
    let (open_b, open_r) = run(0.0);
    let (mute_b, mute_r) = run(80.0);
    assert!(
        mute_b < open_b - 6.0,
        "6th partial: open {open_b:.1}, muted {mute_b:.1}"
    );
    assert!(
        mute_r < open_r - 6.0,
        "ring: open {open_r:.1}, muted {mute_r:.1}"
    );
}

#[test]
fn velocity_follows_its_range() {
    // Level against velocity follows the voice core's square law for the
    // Velocity range, whatever the model; with no range every velocity
    // plays about as loud (only the tone changes).
    for model in [TINE, REED, CLAV] {
        let level = |range: f32, vel: u8| {
            let mut s = Keys::new(RATE);
            s.set_param(MODEL, model);
            s.set_param(VELOCITY, range);
            s.set_param(KEY_NOISE, 0.0);
            s.set_param(BARK, 0.0);
            s.set_param(VEL_TONE, 0.0);
            let (l, _) = render(&mut s, 9_600, &[(0, on(60, vel))], |_| 128);
            db(rms(&l[480..9_600]))
        };
        for range in [20.0, 40.0] {
            let got = level(range, 30) - level(range, 127);
            let want = db(velocity_gain(30, range));
            assert!(
                (got - want).abs() < 2.0,
                "model {model} range {range}: {got:.1} dB, law {want:.1}"
            );
        }
        let flat = level(0.0, 30) - level(0.0, 127);
        assert!(flat.abs() < 3.0, "model {model} no range: {flat:.1} dB");
    }
}
