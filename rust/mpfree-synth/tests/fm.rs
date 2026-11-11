//! Behaviour tests for the FM synth through the mpfree-node interface.

use mpfree_node::{check_params, Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::dx7;
use mpfree_synth::fm::{
    op_freq, peg_octaves, scale_keyboard, scale_level, scale_velocity, Route, ALGORITHMS,
};
use mpfree_synth::fm_params::*;
use mpfree_synth::Fm;

const RATE: u32 = 44_100;

/// Renders `frames` frames with `events` (absolute frames), cut into blocks
/// by `cut(i)`, and returns the left and right channels.
fn render(
    synth: &mut Fm,
    frames: usize,
    events: &[(u64, NodeEvent)],
    mut cut: impl FnMut(usize) -> usize,
) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let mut at = 0usize;
    let mut n = 0usize;
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
        let ctx = Ctx {
            rate: RATE,
            frame: at as u64,
            bpm: 120.0,
            beats: 0.0,
            playing: false,
            sidechain: None,
        };
        synth.process(&ctx, &buf, &mut l[at..at + len], &mut r[at..at + len]);
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
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-9).log10()
}

/// Rising zero crossings per second over `x`.
fn crossings_per_second(x: &[f32]) -> f32 {
    let ups = x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    ups as f32 * RATE as f32 / x.len() as f32
}

/// First-difference energy, a stand-in for high-frequency content.
fn brightness(x: &[f32]) -> f32 {
    let diff: f32 = x.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
    let all: f32 = x.iter().map(|v| v * v).sum();
    diff / all.max(1e-12)
}

/// The Init voice: operator 1 alone, a sine at the note.
fn init() -> Fm {
    let mut s = Fm::new(RATE);
    s.load_preset(&Fm::presets()[0]);
    s
}

fn preset(name: &str) -> Fm {
    let mut s = Fm::new(RATE);
    let p = Fm::presets()
        .into_iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no preset {name}"));
    s.load_preset(&p);
    s
}

/// A busy phrase: chords, algorithm and feedback changes, the pitch
/// envelope and LFO, pressure, bend, stealing, mono legato, AllOff.
fn busy_events() -> Vec<(u64, NodeEvent)> {
    let mut ev = vec![
        (
            0,
            NodeEvent::Param {
                id: LFO_PMD,
                value: 40.0,
            },
        ),
        (
            0,
            NodeEvent::Param {
                id: LFO_AMD,
                value: 30.0,
            },
        ),
        (
            0,
            NodeEvent::Param {
                id: op_id(1, op::AMS),
                value: 2.0,
            },
        ),
        (
            0,
            NodeEvent::Param {
                id: PEG_L1,
                value: 70.0,
            },
        ),
        (
            0,
            NodeEvent::Param {
                id: PEG_R1,
                value: 60.0,
            },
        ),
        (5, on(48, 100)),
        (5, on(55, 90)),
        (7, on(60, 127)),
        (
            3_001,
            NodeEvent::Pressure {
                note: 60,
                value: 90,
            },
        ),
        (
            5_000,
            NodeEvent::Param {
                id: ALGORITHM,
                value: 4.0,
            },
        ),
        (
            6_123,
            NodeEvent::Param {
                id: FEEDBACK,
                value: 7.0,
            },
        ),
        (9_000, NodeEvent::Bend { value: 0.5 }),
        (11_111, off(55)),
        (
            12_000,
            NodeEvent::Param {
                id: ALGORITHM,
                value: 22.0,
            },
        ),
    ];
    for k in 0..20u64 {
        ev.push((16_000 + k * 37, on(40 + k as u8, 64 + k as u8)));
    }
    ev.push((22_000, NodeEvent::AllOff));
    ev.push((
        26_000,
        NodeEvent::Param {
            id: VOICE_MODE,
            value: 1.0,
        },
    ));
    ev.push((26_500, on(50, 120)));
    ev.push((28_000, on(57, 120)));
    ev.push((30_000, off(57)));
    ev.push((32_000, off(50)));
    ev.sort_by_key(|e| e.0);
    ev
}

#[test]
fn same_output_for_any_block_size() {
    let ev = busy_events();
    let frames = 44_100;
    let start = || preset("Brass Section");
    let (rl, rr) = render(&mut start(), frames, &ev, |_| 128);
    assert!(peak(&rl) > 0.01);
    for size in [1usize, 7, 16, 17, 64, 1000, 8192] {
        let (l, r) = render(&mut start(), frames, &ev, |_| size);
        assert!(l == rl && r == rr, "block size {size} differs");
    }
    let mut seed = 777u64;
    let (l, r) = render(&mut start(), frames, &ev, |_| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        1 + (seed >> 33) as usize % 700
    });
    assert!(l == rl && r == rr, "random block sizes differ");
}

#[test]
fn plays_the_right_pitch() {
    let mut s = init();
    let (l, _) = render(&mut s, RATE as usize, &[(0, on(69, 127))], |_| 256);
    let hz = crossings_per_second(&l[RATE as usize / 2..]);
    assert!((hz - 440.0).abs() <= 2.0, "{hz} Hz, wanted 440");
}

#[test]
fn the_init_voice_peaks_near_minus_12_dbfs() {
    let mut s = init();
    let (l, r) = render(
        &mut s,
        RATE as usize,
        &[(0, on(60, 127)), (22_050, off(60))],
        |_| 128,
    );
    let d = db(peak(&l).max(peak(&r)));
    assert!((-16.0..=-8.0).contains(&d), "peak {d:.1} dBFS");
}

#[test]
fn instruments_add_to_the_buffer() {
    let mut s = init();
    let mut l = vec![0.5f32; 256];
    let mut r = vec![0.5f32; 256];
    s.process(&Ctx::new(RATE, 0), &[], &mut l, &mut r);
    assert!(l.iter().chain(&r).all(|&x| x == 0.5));
}

#[test]
fn voices_end_after_release_and_all_off() {
    let mut s = init();
    render(
        &mut s,
        4410,
        &[(0, on(60, 100)), (0, on(64, 100)), (100, off(60))],
        |_| 128,
    );
    assert_eq!(s.param(METER_VOICES), 1.0);
    render(&mut s, 4410, &[(0, NodeEvent::AllOff)], |_| 128);
    render(&mut s, 44_100, &[], |_| 128);
    assert_eq!(s.active_voices(), 0);
    let (l, _) = render(&mut s, 1000, &[], |_| 128);
    assert!(l.iter().all(|&x| x == 0.0));
}

#[test]
fn every_algorithm_has_the_dx7s_carriers_and_feeds_downward() {
    const CARRIERS: [u32; 32] = [
        2, 2, 2, 2, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 3, 3, 4, 4, 4, 5, 5, 3, 3, 3, 4, 4,
        5, 6,
    ];
    for (n, a) in ALGORITHMS.iter().enumerate() {
        let r = Route::new(n + 1);
        assert_eq!(r.carriers.count_ones(), CARRIERS[n], "algorithm {}", n + 1);
        // Operators run from 6 down to 1, so a modulator must be higher than what it modulates.
        assert!(
            a.edges
                .iter()
                .all(|&(from, to)| from > to && (1..=6).contains(&to)),
            "algorithm {}",
            n + 1
        );
        // Feedback goes through the last two samples, so a loop may run upward.
        let (f, t) = a.feedback;
        assert!(
            (1..=6).contains(&f) && (1..=6).contains(&t),
            "algorithm {} feedback",
            n + 1
        );
    }
    // The loops: algorithm 4 feeds 4 back to 6, algorithm 6 feeds 5 back to 6.
    assert_eq!(ALGORITHMS[3].feedback, (4, 6));
    assert_eq!(ALGORITHMS[5].feedback, (5, 6));
    assert_eq!(Route::new(0), Route::new(1));
    assert_eq!(Route::new(99), Route::new(32));
}

#[test]
fn a_modulator_adds_harmonics_and_feedback_adds_more() {
    let tone = |set: &[(u16, f32)]| {
        let mut s = init();
        for &(id, v) in set {
            s.set_param(id, v);
        }
        let (l, _) = render(&mut s, 8820, &[(0, on(57, 127))], |_| 128);
        brightness(&l[4410..])
    };
    let sine = tone(&[]);
    let modulated = tone(&[(op_id(2, op::LEVEL), 80.0)]);
    assert!(modulated > 3.0 * sine, "sine {sine}, modulated {modulated}");
    // Algorithm 32: six carriers, feedback on 6.
    let six = [
        (ALGORITHM, 32.0),
        (op_id(1, op::LEVEL), 0.0),
        (op_id(6, op::LEVEL), 99.0),
    ];
    let plain = tone(&six);
    let fed = tone(&[six[0], six[1], six[2], (FEEDBACK, 7.0)]);
    assert!(fed > 3.0 * plain, "no feedback {plain}, feedback 7 {fed}");
}

#[test]
fn frequencies_follow_ratio_fixed_and_detune() {
    assert!((op_freq(69.0, false, 1, 0, 0) - 440.0).abs() < 0.01);
    assert!(
        (op_freq(69.0, false, 0, 0, 0) - 220.0).abs() < 0.01,
        "coarse 0 is half"
    );
    assert!(
        (op_freq(69.0, false, 2, 50, 0) - 1320.0).abs() < 0.1,
        "2 x 1.5"
    );
    assert!(
        (op_freq(30.0, true, 2, 0, 0) - 100.0).abs() < 0.01,
        "fixed: 10^2 Hz whatever the note"
    );
    assert!((op_freq(30.0, true, 1, 50, 0) - 31.62).abs() < 0.01);
    let up = op_freq(69.0, false, 1, 0, 7) / 440.0;
    assert!(
        up > 1.0 && up < 1.0 + 8.0 / 1200.0,
        "detune +7 is a few cents: {up}"
    );
    // Fixed frequency on the synth: the same pitch for two notes.
    for note in [40, 80] {
        let mut s = init();
        s.set_param(op_id(1, op::MODE), 1.0);
        s.set_param(op_id(1, op::COARSE), 2.0);
        s.set_param(op_id(1, op::FINE), 30.0);
        let (l, _) = render(&mut s, RATE as usize, &[(0, on(note, 127))], |_| 128);
        let hz = crossings_per_second(&l[RATE as usize / 2..]);
        assert!(
            (hz - 199.5).abs() <= 2.0,
            "note {note}: {hz} Hz, wanted 10^2.3"
        );
    }
}

/// Peak in each tenth of a second.
fn tenths(x: &[f32]) -> Vec<f32> {
    x.chunks(RATE as usize / 10).map(|c| db(peak(c))).collect()
}

#[test]
fn a_decay_falls_at_a_steady_rate_in_decibels() {
    let mut s = init();
    s.set_param(op_id(1, op::R2), 50.0);
    s.set_param(op_id(1, op::L2), 0.0);
    s.set_param(op_id(1, op::L3), 0.0);
    let (l, _) = render(&mut s, RATE as usize / 2, &[(0, on(60, 127))], |_| 128);
    let t = tenths(&l);
    let steps: Vec<f32> = t.windows(2).skip(1).map(|w| w[0] - w[1]).collect();
    assert!(steps.iter().all(|&d| d > 2.0), "{t:?}");
    let (lo, hi) = steps
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), &d| (a.min(d), b.max(d)));
    assert!(hi - lo < 1.0, "steps {steps:?}");
}

#[test]
fn velocity_sensitivity_scales_the_level() {
    let level = |sens: f32, velocity: u8| {
        let mut s = init();
        s.set_param(op_id(1, op::VEL_SENS), sens);
        let (l, _) = render(&mut s, 4410, &[(0, on(60, velocity))], |_| 128);
        db(peak(&l))
    };
    assert!((level(0.0, 30) - level(0.0, 127)).abs() < 0.1);
    assert!(level(7.0, 127) - level(7.0, 30) > 15.0);
    assert!(level(3.0, 127) - level(3.0, 30) < level(7.0, 127) - level(7.0, 30));
    assert!(scale_velocity(127, 7) >= 0 && scale_velocity(1, 7) < -1000);
}

#[test]
fn keyboard_scaling_and_rate_scaling_follow_the_note() {
    // -LIN to the right of C3 at full depth: higher notes are quieter.
    assert_eq!(scale_keyboard(60, 39, 0, 99, 0, 0), 0);
    assert!(scale_keyboard(84, 39, 0, 99, 0, 0) < -20);
    assert!(scale_keyboard(84, 39, 0, 99, 0, 3) > 20, "+LIN raises");
    assert!(scale_keyboard(36, 39, 99, 0, 1, 0) < -5, "-EXP on the left");
    let level = |note: u8| {
        let mut s = init();
        s.set_param(op_id(1, op::BREAK), 39.0);
        s.set_param(op_id(1, op::R_DEPTH), 99.0);
        let (l, _) = render(&mut s, 4410, &[(0, on(note, 127))], |_| 128);
        db(peak(&l))
    };
    assert!(level(60) - level(84) > 6.0);
    // Rate scaling: a high note decays faster.
    let decay = |note: u8| {
        let mut s = init();
        s.set_param(op_id(1, op::R2), 40.0);
        s.set_param(op_id(1, op::L2), 0.0);
        s.set_param(op_id(1, op::L3), 0.0);
        s.set_param(op_id(1, op::RATE_SCALE), 7.0);
        let (l, _) = render(&mut s, RATE as usize / 2, &[(0, on(note, 127))], |_| 128);
        let t = tenths(&l);
        t[1] - t[4]
    };
    assert!(
        decay(96) > decay(36) + 6.0,
        "high {} dB, low {} dB",
        decay(96),
        decay(36)
    );
    assert_eq!(scale_level(99), 127);
    assert_eq!(scale_level(0), 0);
}

#[test]
fn the_pitch_envelope_starts_away_and_settles() {
    assert_eq!(peg_octaves(50), 0.0);
    assert!(peg_octaves(99) > 3.5 && peg_octaves(0) < -3.5);
    let mut s = init();
    // Starts an octave up (L4 is where it starts), falls to the note.
    s.set_param(PEG_L4, 82.0);
    s.set_param(PEG_L1, 50.0);
    s.set_param(PEG_R1, 70.0);
    let (l, _) = render(&mut s, RATE as usize, &[(0, on(57, 127))], |_| 128);
    let start = crossings_per_second(&l[..2205]);
    let end = crossings_per_second(&l[RATE as usize / 2..]);
    assert!((end - 220.0).abs() < 2.0, "settles on the note: {end}");
    assert!(start > 300.0, "starts above: {start}");
}

#[test]
fn polyphony_limit_steals_without_clicks() {
    let mut s = init();
    s.set_param(POLYPHONY, 4.0);
    s.set_param(op_id(1, op::R4), 30.0);
    let ev: Vec<_> = (0..10u64)
        .map(|k| (k * 441, on(50 + k as u8, 100)))
        .collect();
    let (l, _) = render(&mut s, 8820, &ev, |_| 64);
    assert!(s.param(METER_VOICES) <= 4.0);
    assert_eq!(s.dropped, 0);
    let jump = l
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(jump < 0.05, "largest step {jump}");
}

#[test]
fn mono_plays_legato_and_returns_to_the_held_key() {
    let mut s = preset("Rubber Bass");
    assert_eq!(s.param(VOICE_MODE), 1.0);
    render(&mut s, 4410, &[(0, on(36, 100))], |_| 128);
    let (l, _) = render(&mut s, 4410, &[(0, on(43, 100))], |_| 128);
    assert_eq!(s.active_voices(), 1);
    let first = crossings_per_second(&l[2205..]);
    render(&mut s, 441, &[(0, off(43))], |_| 128);
    assert_eq!(s.param(METER_VOICES), 1.0, "back to the held key");
    let (l, _) = render(&mut s, 4410, &[], |_| 128);
    assert!(crossings_per_second(&l) < first, "a lower pitch again");
    render(&mut s, 3 * RATE as usize, &[(0, off(36))], |_| 128);
    assert_eq!(s.active_voices(), 0);
}

#[test]
fn extreme_settings_stay_finite() {
    for p in PARAMS.iter() {
        for v in [p.min, p.max] {
            let mut s = preset("Brass Section");
            s.set_param(p.id, v);
            let (l, r) = render(
                &mut s,
                4000,
                &[(0, on(20, 127)), (0, on(120, 127)), (2000, off(20))],
                |_| 333,
            );
            assert!(
                l.iter().chain(&r).all(|x| x.is_finite()),
                "{} = {v}",
                p.name
            );
            assert!(peak(&l) < 4.0, "{} = {v}: peak {}", p.name, peak(&l));
        }
    }
    let mut s = init();
    for id in [FEEDBACK, LFO_PMD, LFO_AMD, LFO_PMS, LFO_SPEED, LEVEL] {
        s.set_param(id, f32::MAX);
    }
    // Every pressure slot at full depth on each target in turn.
    for t in 0..8 {
        for n in 0..4 {
            s.set_param(press_target(n), t as f32);
            s.set_param(press_depth(n), 100.0);
        }
        s.set_param(PRESS_HOLDOFF, 0.0);
        let ev = [
            (0, on(0, 127)),
            (0, on(127, 127)),
            (
                10,
                NodeEvent::Pressure {
                    note: 127,
                    value: 127,
                },
            ),
            (
                10,
                NodeEvent::Pressure {
                    note: 0,
                    value: 127,
                },
            ),
        ];
        let (l, _) = render(&mut s, 8_000, &ev, |_| 128);
        assert!(l.iter().all(|x| x.is_finite()), "target {t}");
        s.reset();
    }
    for n in 1..=6 {
        s.set_param(op_id(n, op::LEVEL), 99.0);
        s.set_param(op_id(n, op::AMS), 3.0);
    }
    let ev = [
        (0, on(0, 127)),
        (0, on(127, 127)),
        (
            10,
            NodeEvent::Pressure {
                note: 127,
                value: 127,
            },
        ),
        (100, NodeEvent::Bend { value: f32::NAN }),
    ];
    let (l, _) = render(&mut s, 44_100, &ev, |_| 128);
    assert!(l.iter().all(|x| x.is_finite()));
}

#[test]
fn parameters_and_presets_are_valid() {
    let s = Fm::new(RATE);
    check_params(s.params()).unwrap();
    assert!(s.params().iter().all(|p| (p.id as usize) < ID_SPACE));
    for preset in Fm::presets() {
        for &(id, v) in &preset.values {
            let p = info(id).unwrap_or_else(|| panic!("{}: unknown id {id}", preset.name));
            assert_eq!(
                p.clamp(v),
                v,
                "{}: {} = {v} out of range",
                preset.name,
                p.name
            );
        }
    }
    assert!(s.param(9999).is_nan());
    assert_eq!(s.info().id, "mpfree.fm");
}

#[test]
fn every_preset_plays_at_a_sane_level_and_ends() {
    for preset in Fm::presets() {
        let mut s = Fm::new(RATE);
        s.load_preset(&preset);
        let ev = [
            (0, on(48, 110)),
            (0, on(55, 110)),
            (0, on(60, 110)),
            (33_075, off(48)),
            (33_075, off(55)),
            (33_075, off(60)),
        ];
        let (l, r) = render(&mut s, 10 * RATE as usize, &ev, |_| 512);
        let d = db(peak(&l).max(peak(&r)));
        println!("{}: chord peak {d:.1} dBFS", preset.name);
        assert!(
            (-30.0..=-1.0).contains(&d),
            "{}: chord peak {d:.1} dBFS",
            preset.name
        );
        assert_eq!(s.active_voices(), 0, "{}: voices left", preset.name);
        let tail = peak(&l[l.len() - 441..]);
        assert!(tail == 0.0, "{}: tail {tail}", preset.name);
    }
}

#[test]
fn a_dx7_bank_loads_and_plays() {
    let voices: Vec<dx7::Dx7Voice> = Fm::presets()
        .iter()
        .map(dx7::Dx7Voice::from_preset)
        .collect();
    let bytes = dx7::bank(&voices);
    assert_eq!(bytes.len(), 4104);
    let back = dx7::parse(&bytes).unwrap();
    assert_eq!(back.len(), 32, "a bank always holds 32 voices");
    for (k, v) in back.iter().take(voices.len()).enumerate() {
        let mut s = Fm::new(RATE);
        s.load_preset(&v.preset());
        let (l, _) = render(&mut s, 8820, &[(0, on(60, 100))], |_| 128);
        assert!(
            peak(&l) > 0.01 && l.iter().all(|x| x.is_finite()),
            "voice {k} ({})",
            v.name
        );
    }
}

#[test]
fn the_drawn_envelope_matches_the_engine() {
    use mpfree_synth::fm::env_stages;
    // Init: one carrier, no keyboard or velocity scaling. Decay at rate 40
    // from full to silence, held down.
    let rates = [99u8, 40, 99, 99];
    let levels = [99u8, 0, 0, 0];
    let stages = env_stages(&rates, &levels);
    let decay = stages[1].last().unwrap().0;
    assert!(
        stages[0].last().unwrap().0 < 0.01,
        "a rate 99 attack is near instant"
    );
    assert!((stages[0].last().unwrap().1 - 1.0).abs() < 1e-4 && stages[1].last().unwrap().1 == 0.0);
    // Equal levels take no time.
    assert_eq!(stages[2].len(), 1);
    // The engine's output falls 60 dB in two thirds of the drawn decay (the
    // envelope is linear in dB, 90 dB from full to its floor).
    let mut s = Fm::new(RATE);
    s.load_preset(&Fm::presets()[0]);
    for (k, f) in [op::R1, op::R2, op::R3, op::R4].into_iter().enumerate() {
        s.set_param(op_id(1, f), f32::from(rates[k]));
    }
    for (k, f) in [op::L1, op::L2, op::L3, op::L4].into_iter().enumerate() {
        s.set_param(op_id(1, f), f32::from(levels[k]));
    }
    let frames = ((decay * 1.2 + 0.2) * RATE as f32) as usize;
    let (l, _) = render(&mut s, frames, &[(0, on(60, 127))], |_| 256);
    let win = RATE as usize / 100;
    let rms: Vec<f32> = l
        .chunks(win)
        .map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt())
        .collect();
    let top = rms.iter().copied().fold(0.0, f32::max);
    let at = rms
        .iter()
        .position(|&r| r < top * 1e-3)
        .expect("it decays 60 dB") as f32
        * 0.01;
    let want = decay * 60.0 / (6.02 * (3840.0 - 16.0) / 256.0);
    println!("drawn decay {decay:.3} s; -60 dB at {at:.3} s, want {want:.3} s");
    assert!((at / want - 1.0).abs() < 0.1, "{at} vs {want}");
}
