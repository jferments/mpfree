//! Behaviour tests for the wind engine (Wind) through the mpfree-node
//! interface.

use mpfree_node::{check_params, Ctl, Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::wind_params::*;
use mpfree_synth::Wind;

const RATE: u32 = 48_000;

/// Renders in blocks `cut` gives; returns left and right.
fn render_cut(
    s: &mut Wind,
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

/// One voice in the middle: the left channel (the right is the same).
fn render(s: &mut Wind, frames: usize, events: &[(u64, NodeEvent)]) -> Vec<f32> {
    let (l, r) = render_cut(s, frames, events, |_| 128);
    if s.param(VOICES) == 1.0 {
        assert_eq!(l, r, "one voice sits in the middle");
    }
    l
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
fn ctl(ctl: Ctl, value: f32) -> NodeEvent {
    NodeEvent::Control { ctl, value }
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
fn at(t: f32) -> usize {
    ms(t) as usize
}
fn hz(note: f32) -> f64 {
    440.0 * ((f64::from(note) - 69.0) / 12.0).exp2()
}
/// Brightness: the RMS of the slope against the RMS, in Hz (the spectral
/// centroid's cousin: a sine at f reads f).
fn bright(x: &[f32]) -> f32 {
    let d: Vec<f32> = x.windows(2).map(|w| w[1] - w[0]).collect();
    rms(&d) / rms(x).max(1e-12) * RATE as f32 / std::f32::consts::TAU
}

/// The pitch, Hz, from upward zero crossings between `from` and `to` of
/// `x` band-passed around `near` (two passes of a resonant band-pass,
/// which keep the fundamental of a periodic sound and its period).
fn pitch(x: &[f32], from: usize, to: usize, near: f64) -> f64 {
    let w = std::f64::consts::TAU * near / f64::from(RATE);
    let q = 2.0;
    let alpha = w.sin() / (2.0 * q);
    let (b0, b2, a0, a1, a2) = (alpha, -alpha, 1.0 + alpha, -2.0 * w.cos(), 1.0 - alpha);
    let mut y: Vec<f64> = x.iter().map(|&v| f64::from(v)).collect();
    for _ in 0..2 {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        for v in y.iter_mut() {
            let xn = *v;
            let yn = (b0 * xn + b2 * x2 - a1 * y1 - a2 * y2) / a0;
            x2 = x1;
            x1 = xn;
            y2 = y1;
            y1 = yn;
            *v = yn;
        }
    }
    let mut ups = Vec::new();
    for i in from.max(1)..to.min(y.len()) {
        if y[i - 1] < 0.0 && y[i] >= 0.0 {
            ups.push((i - 1) as f64 + y[i - 1] / (y[i - 1] - y[i]));
        }
    }
    let n = ups.len();
    assert!(n >= 3, "too few cycles");
    (n - 1) as f64 * f64::from(RATE) / (ups[n - 1] - ups[0])
}

fn cents(f: f64, want: f64) -> f64 {
    1200.0 * (f / want).log2()
}

fn preset(name: &str) -> Wind {
    let mut w = Wind::new(RATE);
    let p = Wind::presets()
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no preset {name}"));
    w.load_preset(p);
    w
}

/// A model with no vibrato, noise or growl: a steady tone.
fn steady(model: f32) -> Wind {
    let mut w = Wind::new(RATE);
    for (id, v) in [(MODEL, model), (VIB_DEPTH, 0.0), (NOISE, 0.0), (GROWL, 0.0)] {
        w.set_param(id, v);
    }
    w
}

const MODELS: [f32; 3] = [CLARINET, SAX, FLUTE];

#[test]
fn params_and_presets_are_valid() {
    let w = Wind::new(RATE);
    check_params(w.params()).unwrap();
    assert_eq!(w.info().id, "mpfree.wind");
    let names: Vec<_> = Wind::presets().iter().map(|p| p.name).collect();
    assert_eq!(names[0], "Init");
    // 18 engine presets and two sounds with their own effects (Hall Flute,
    // Smoky Sax).
    assert_eq!(names.len(), 20, "{names:?}");
    for (i, a) in names.iter().enumerate() {
        assert!(!names[..i].contains(a), "{a} twice");
    }
    for p in Wind::presets() {
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
            assert_ne!(id, LEVEL, "{} sets Level", p.name);
        }
    }
}

#[test]
fn silent_until_a_key_and_adds_into_the_buffers() {
    let mut w = Wind::new(RATE);
    let x = render(&mut w, 4096, &[]);
    assert!(x.iter().all(|&v| v == 0.0));
    assert_eq!(w.sounding_voices(), 0);
    let mut w = preset("Init");
    let alone = render(&mut w, 4800, &[(0, on(60, 100))]);
    let mut w = preset("Init");
    let mut l = vec![0.25f32; 4800];
    let mut r = vec![0.25f32; 4800];
    for at in (0..4800).step_by(96) {
        let ev = if at == 0 {
            vec![Timed::new(0, on(60, 100))]
        } else {
            vec![]
        };
        w.process(
            &Ctx::new(RATE, at as u64),
            &ev,
            &mut l[at..at + 96],
            &mut r[at..at + 96],
        );
    }
    for (a, x) in alone.iter().zip(&l) {
        assert!((a + 0.25 - x).abs() < 1e-6);
    }
    assert!(peak(&alone) > 0.01);
}

#[test]
fn the_same_output_for_any_block_size() {
    for name in [
        "Init",
        "Tenor Sax",
        "Concert Flute",
        "Growl Sax",
        "Reed Section",
        "Wind Controller Sax",
        "Pressure Swell",
    ] {
        let events = [
            (0, ctl(Ctl::Breath, 0.7)),
            (0, on(60, 100)),
            (
                ms(80.0),
                NodeEvent::Pressure {
                    note: 60,
                    value: 90,
                },
            ),
            (ms(100.0), on(64, 80)),
            (ms(120.0), ctl(Ctl::ModWheel, 0.5)),
            (ms(150.0), NodeEvent::Bend { value: 0.4 }),
            (ms(170.0), ctl(Ctl::Breath, 0.3)),
            (ms(190.0), set(TONE, 70.0)),
            (ms(200.0), off(64)),
            (ms(260.0), off(60)),
            (ms(300.0), on(55, 127)),
            (ms(310.0), on(67, 40)),
            (ms(450.0), off(55)),
            (ms(470.0), off(67)),
        ];
        let frames = ms(800.0) as usize;
        let mut a = preset(name);
        let x = render_cut(&mut a, frames, &events, |_| 128);
        assert!(peak(&x.0) > 1e-3, "{name} is silent");
        for cut in [1usize, 37, 64, 333] {
            let mut b = preset(name);
            let y = render_cut(
                &mut b,
                frames,
                &events,
                |n| if n % 2 == 0 { cut } else { 5 },
            );
            assert_eq!(x, y, "{name}, cut {cut}");
        }
    }
}

/// Each bore in tune from C2 to C6 at a soft, a middle and a full breath:
/// the nonlinear bore's pull on the pitch is measured and taken out.
#[test]
fn every_bore_is_in_tune_from_c2_to_c6() {
    for model in MODELS {
        for note in [36u8, 48, 60, 72, 84] {
            for vel in [30u8, 100, 127] {
                let mut w = steady(model);
                let x = render(&mut w, at(1500.0), &[(0, on(note, vel))]);
                let f = pitch(&x, at(500.0), at(1500.0), hz(f32::from(note)));
                let c = cents(f, hz(f32::from(note)));
                assert!(
                    c.abs() < 1.0,
                    "model {model}, note {note}, velocity {vel}: {c:+.2} cents"
                );
            }
        }
    }
}

/// Tone, Reed and Edge across their ranges keep the pitch within 3 cents
/// (the tables cover Tone and Edge; Reed is left, about a cent).
#[test]
fn tone_reed_and_edge_keep_the_pitch() {
    for model in MODELS {
        for (id, v) in [
            (TONE, 0.0),
            (TONE, 100.0),
            (REED, 0.0),
            (REED, 100.0),
            (EDGE, 0.0),
            (EDGE, 100.0),
        ] {
            let mut w = steady(model);
            w.set_param(id, v);
            let x = render(&mut w, at(1500.0), &[(0, on(60, 110))]);
            let f = pitch(&x, at(500.0), at(1500.0), hz(60.0));
            let c = cents(f, hz(60.0));
            assert!(
                c.abs() < 3.0,
                "model {model}, param {id} = {v}: {c:+.2} cents"
            );
        }
    }
}

/// A harder breath is louder on the voice core's square law over
/// Velocity's range, and brighter.
#[test]
fn a_harder_breath_is_louder_and_brighter() {
    for model in MODELS {
        let level = |vel: u8| {
            let mut w = steady(model);
            let x = render(&mut w, at(900.0), &[(0, on(60, vel))]);
            let seg = &x[at(500.0)..at(900.0)];
            (db(rms(seg)), bright(seg))
        };
        let (soft, soft_b) = level(20);
        let (mid, _) = level(64);
        let (hard, hard_b) = level(127);
        assert!(
            soft < mid && mid < hard,
            "model {model}: {soft} {mid} {hard}"
        );
        // The law: velocity 20 against 127 at 24 dB, Breath 85 and
        // Sustain 90 (breaths 0.12 and 0.77).
        let law = |b: f32| db(mpfree_synth::wind::breath_level(b, 24.0));
        let want = law(20.0 / 127.0 * 0.85 * 0.9) - law(0.85 * 0.9);
        assert!(
            ((soft - hard) - want).abs() < 1.0,
            "model {model}: {:.2} dB against the law's {want:.2}",
            soft - hard
        );
        assert!(
            hard_b > soft_b * 1.1,
            "model {model}: brightness {soft_b} to {hard_b}"
        );
    }
}

/// The level of a held note is within 3 dB from C2 to C6 on each bore.
#[test]
fn level_is_steady_across_the_keys() {
    for model in MODELS {
        let lv: Vec<f32> = [36u8, 48, 60, 72, 84]
            .iter()
            .map(|&n| {
                let mut w = steady(model);
                let x = render(&mut w, at(900.0), &[(0, on(n, 100))]);
                db(rms(&x[at(500.0)..at(900.0)]))
            })
            .collect();
        let hi = lv.iter().copied().fold(f32::MIN, f32::max);
        let lo = lv.iter().copied().fold(f32::MAX, f32::min);
        assert!(hi - lo < 3.0, "model {model}: {lv:?}");
        assert!(hi < -3.0 && lo > -30.0, "model {model}: {lv:?}");
    }
}

/// A note speaks within 60 ms of the key (within 6 dB of where it
/// settles), and starts from silence without a click.
#[test]
fn a_note_speaks_quickly_without_a_click() {
    for model in MODELS {
        let mut w = steady(model);
        let x = render(&mut w, at(800.0), &[(0, on(60, 100))]);
        let settled = rms(&x[at(500.0)..at(800.0)]);
        let win = at(5.0);
        let first = (0..x.len() - win)
            .step_by(win / 2)
            .find(|&i| rms(&x[i..i + win]) > settled * 0.5)
            .unwrap();
        assert!(first < at(60.0), "model {model}: {first} frames");
        // The first millisecond: no jump bigger than the settled level's.
        assert!(
            peak(&x[..at(1.0)]) < settled * 2.0,
            "model {model}: {} against {settled}",
            peak(&x[..at(1.0)])
        );
    }
}

/// One voice: a key over a held key slurs (the breath carries on, no new
/// attack) or, with Legato on Tongue, attacks again.
#[test]
fn a_slur_keeps_the_breath_and_a_tongue_attacks_again() {
    for (legato, rises) in [(SLUR, false), (TONGUED, true)] {
        let mut w = steady(CLARINET);
        w.set_param(DECAY, 50.0);
        w.set_param(SUSTAIN, 40.0);
        w.set_param(TONGUE, 0.0);
        w.set_param(LEGATO, legato);
        let x = render(
            &mut w,
            at(1200.0),
            &[(0, on(60, 110)), (ms(600.0), on(67, 110))],
        );
        let before = rms(&x[at(450.0)..at(590.0)]);
        let after = rms(&x[at(605.0)..at(650.0)]);
        if rises {
            assert!(after > before * 1.3, "tongued: {before} to {after}");
        } else {
            assert!(
                (db(after) - db(before)).abs() < 2.0,
                "slurred: {before} to {after}"
            );
        }
        let f = pitch(&x, at(800.0), at(1200.0), hz(67.0));
        assert!(cents(f, hz(67.0)).abs() < 1.0, "landed at {f}");
        assert_eq!(w.sounding_voices(), 1);
    }
}

#[test]
fn priority_picks_the_held_key() {
    for (prio, want) in [(0.0, 64u8), (1.0, 55), (2.0, 64)] {
        let mut w = steady(SAX);
        w.set_param(PRIORITY, prio);
        let x = render(
            &mut w,
            at(1200.0),
            &[
                (0, on(60, 100)),
                (ms(10.0), on(55, 100)),
                (ms(20.0), on(64, 100)),
            ],
        );
        let f = pitch(&x, at(600.0), at(1200.0), hz(f32::from(want)));
        assert!(
            cents(f, hz(f32::from(want))).abs() < 1.0,
            "priority {prio}: {f} Hz"
        );
    }
    // Last: letting go of the newest key goes back to the one before.
    let mut w = steady(SAX);
    let x = render(
        &mut w,
        at(1600.0),
        &[
            (0, on(60, 100)),
            (ms(10.0), on(64, 100)),
            (ms(600.0), off(64)),
        ],
    );
    let f = pitch(&x, at(1000.0), at(1600.0), hz(60.0));
    assert!(cents(f, hz(60.0)).abs() < 1.0, "back to {f} Hz");
}

/// Glide by time takes as long as it says; with Glide at 0 a slur still
/// moves over at least 10 ms, as fingers do.
#[test]
fn glide_moves_the_pitch_over_its_time() {
    let mut w = steady(FLUTE);
    w.set_param(GLIDE, 200.0);
    let x = render(
        &mut w,
        at(1500.0),
        &[(0, on(60, 100)), (ms(700.0), on(72, 100))],
    );
    // Half way through the glide: between the notes.
    let mid = pitch(&x, at(780.0), at(820.0), hz(66.0));
    assert!(mid > hz(61.0) && mid < hz(71.0), "{mid}");
    let end = pitch(&x, at(1100.0), at(1500.0), hz(72.0));
    assert!(cents(end, hz(72.0)).abs() < 1.0, "{end}");
}

/// Breath from a breath controller: no breath is silence while the key is
/// held; the controller swells the note; letting the breath go fades it
/// out smoothly, with no cut where the reed stops.
#[test]
fn a_breath_controller_plays_the_note() {
    for model in MODELS {
        let mut w = steady(model);
        w.set_param(BREATH_FROM, FROM_BREATH);
        w.set_param(ATTACK, 1.0);
        w.set_param(SUSTAIN, 100.0);
        let mut events = vec![(0, ctl(Ctl::Breath, 0.0)), (0, on(60, 100))];
        // Up over 300 ms from 200 ms, held, down over 300 ms from 1 s.
        for i in 0..=30u64 {
            let b = i as f32 / 30.0;
            events.push((ms(200.0) + i * ms(10.0), ctl(Ctl::Breath, b)));
            events.push((ms(1000.0) + i * ms(10.0), ctl(Ctl::Breath, 1.0 - b)));
        }
        let x = render(&mut w, at(1500.0), &events);
        assert!(
            peak(&x[..at(190.0)]) < 1e-6,
            "model {model}: no breath, no sound"
        );
        let full = rms(&x[at(600.0)..at(900.0)]);
        assert!(full > 0.05, "model {model}: {full}");
        // Fading down: every 10 ms window quieter than the one before
        // (within a little), none cut off.
        let wins: Vec<f32> = (0..30)
            .map(|i| rms(&x[at(1000.0 + 10.0 * i as f32)..at(1010.0 + 10.0 * i as f32)]))
            .collect();
        for i in 1..wins.len() {
            assert!(
                wins[i] < wins[i - 1] * 1.2 + 1e-4,
                "model {model}: window {i} {} after {}",
                wins[i],
                wins[i - 1]
            );
            if wins[i - 1] > full * 0.05 {
                assert!(
                    wins[i] > wins[i - 1] * 0.4,
                    "model {model}: cut off at window {i}: {} after {}",
                    wins[i],
                    wins[i - 1]
                );
            }
        }
        assert!(
            peak(&x[at(1320.0)..]) < full * 0.01,
            "model {model}: still sounding"
        );
    }
}

/// Pad pressure, on slot 1 (Breath), swells a note held at a soft breath.
#[test]
fn pressure_swells_the_note() {
    let mut w = preset("Pressure Swell");
    let x = render(
        &mut w,
        at(1500.0),
        &[
            (0, on(60, 100)),
            (
                ms(700.0),
                NodeEvent::Pressure {
                    note: 60,
                    value: 127,
                },
            ),
        ],
    );
    let before = rms(&x[at(500.0)..at(690.0)]);
    let after = rms(&x[at(1200.0)..at(1500.0)]);
    assert!(db(after) - db(before) > 3.0, "{before} to {after}");
    assert!(bright(&x[at(1200.0)..at(1500.0)]) > bright(&x[at(500.0)..at(690.0)]));
}

/// The mod wheel adds vibrato: the pitch moves.
#[test]
fn the_mod_wheel_adds_vibrato() {
    let wobble = |wheel: f32| {
        let mut w = steady(SAX);
        let x = render(
            &mut w,
            at(2000.0),
            &[(0, ctl(Ctl::ModWheel, wheel)), (0, on(60, 100))],
        );
        let ps: Vec<f64> = (0..20)
            .map(|i| {
                let a = at(1000.0 + 50.0 * i as f32);
                pitch(&x, a, a + at(50.0), hz(60.0))
            })
            .collect();
        let hi = ps.iter().copied().fold(f64::MIN, f64::max);
        let lo = ps.iter().copied().fold(f64::MAX, f64::min);
        cents(hi, lo)
    };
    let none = wobble(0.0);
    let full = wobble(1.0);
    assert!(none < 2.0, "{none}");
    assert!(full > 15.0, "{full}");
}

/// With voices, each note takes one; spread puts them left and right.
#[test]
fn voices_play_chords_and_spread() {
    let mut w = preset("Reed Section");
    let (l, r) = render_cut(
        &mut w,
        at(600.0),
        &[(0, on(48, 100)), (0, on(55, 100)), (0, on(64, 100))],
        |_| 128,
    );
    assert_eq!(w.sounding_voices(), 3);
    assert!(peak(&l) < 1.0 && peak(&r) < 1.0);
    assert_ne!(l, r);
    // A fourth past the limit of six is fine; a seventh steals.
    let mut w = preset("Reed Section");
    let notes: Vec<(u64, NodeEvent)> = (0..7u8).map(|i| (0, on(48 + 3 * i, 100))).collect();
    render_cut(&mut w, at(200.0), &notes, |_| 128);
    assert_eq!(w.sounding_voices(), 6);
}

/// A track at rest stops running: the release ends, the voice goes, and
/// the output is exact zeros.
#[test]
fn a_track_at_rest_stops_running() {
    for model in MODELS {
        let mut w = Wind::new(RATE);
        w.set_param(MODEL, model);
        let x = render(
            &mut w,
            at(1500.0),
            &[(0, on(60, 100)), (ms(300.0), off(60))],
        );
        assert!(peak(&x[..at(300.0)]) > 0.01);
        assert_eq!(w.sounding_voices(), 0, "model {model}");
        assert!(x[at(1000.0)..].iter().all(|&v| v == 0.0), "model {model}");
    }
}

/// Changing the bore while a note sounds fades it out; nothing blows up.
#[test]
fn a_new_bore_mid_note_fades_out() {
    let mut w = steady(CLARINET);
    let x = render(
        &mut w,
        at(800.0),
        &[(0, on(60, 100)), (ms(300.0), set(MODEL, FLUTE))],
    );
    assert!(x.iter().all(|v| v.is_finite() && v.abs() < 2.0));
    assert!(peak(&x[at(320.0)..]) < 1e-3, "{}", peak(&x[at(320.0)..]));
}

/// Every setting at its extremes, the lowest and highest keys, the
/// softest and hardest notes: finite and bounded.
#[test]
fn finite_at_the_extremes() {
    for model in MODELS {
        for hi in [false, true] {
            let mut w = Wind::new(RATE);
            for p in PARAMS {
                if !p.readonly && p.id != VOICES && p.id != MODEL {
                    w.set_param(p.id, if hi { p.max } else { p.min });
                }
            }
            w.set_param(MODEL, model);
            w.set_param(LEVEL, 0.0);
            w.set_param(BREATH_FROM, FROM_VELOCITY);
            let x = render(
                &mut w,
                at(1500.0),
                &[
                    (0, on(0, 1)),
                    (ms(300.0), off(0)),
                    (ms(300.0), on(127, 127)),
                    (ms(700.0), on(60, 127)),
                    (ms(900.0), NodeEvent::Bend { value: 1.0 }),
                    (
                        ms(950.0),
                        NodeEvent::Pressure {
                            note: 60,
                            value: 127,
                        },
                    ),
                ],
            );
            assert!(
                x.iter().all(|v| v.is_finite() && v.abs() < 4.0),
                "model {model}, {}: peak {}",
                if hi { "max" } else { "min" },
                peak(&x)
            );
        }
    }
}

/// A note held with no breath (a breath controller at 0) ends in exact
/// zeros, not denormals.
#[test]
fn a_note_at_no_breath_is_exact_silence() {
    for model in MODELS {
        let mut w = Wind::new(RATE);
        w.set_param(MODEL, model);
        w.set_param(BREATH_FROM, FROM_BREATH);
        let x = render(
            &mut w,
            at(2000.0),
            &[(0, ctl(Ctl::Breath, 0.0)), (0, on(60, 100))],
        );
        assert!(
            x[at(500.0)..].iter().all(|&v| v == 0.0),
            "model {model}: {}",
            peak(&x[at(500.0)..])
        );
    }
}

/// Breath from a controller that has not moved yet: the notes play from
/// their velocity, so the sound is not silent without one; once it moves,
/// it takes over; a reset forgets it.
#[test]
fn a_controller_takes_over_once_it_moves() {
    for from in [FROM_BREATH, FROM_WHEEL, FROM_FOOT] {
        let mut w = steady(SAX);
        w.set_param(BREATH_FROM, from);
        let c = [Ctl::Breath, Ctl::ModWheel, Ctl::Foot][from as usize - 1];
        let x = render(
            &mut w,
            at(3000.0),
            &[(0, on(60, 100)), (ms(700.0), ctl(c, 0.0))],
        );
        assert!(rms(&x[at(400.0)..at(690.0)]) > 0.02, "from {from}: silent");
        assert!(
            peak(&x[at(720.0)..]) < 1e-3,
            "from {from}: the controller at 0: {} {}",
            peak(&x[at(720.0)..]),
            peak(&x[at(400.0)..at(690.0)])
        );
        // Exact zeros once the DC blocker's tail is gone.
        assert!(
            peak(&x[at(2600.0)..]) == 0.0,
            "from {from}: {}",
            peak(&x[at(2600.0)..])
        );
        w.reset();
        let x = render(&mut w, at(600.0), &[(0, on(60, 100))]);
        assert!(rms(&x[at(400.0)..]) > 0.02, "from {from}: after a reset");
    }
}
