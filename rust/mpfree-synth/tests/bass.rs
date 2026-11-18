//! Behaviour tests for the bass engine (Bass) through the mpfree-node
//! interface.

use mpfree_node::{check_params, Ctl, Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::bass_params::*;
use mpfree_synth::Bass;

const RATE: u32 = 48_000;

fn render_cut(
    s: &mut Bass,
    frames: usize,
    events: &[(u64, NodeEvent)],
    mut cut: impl FnMut(usize) -> usize,
) -> Vec<f32> {
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
    assert_eq!(l, r, "the bass is mono");
    l
}

fn render(s: &mut Bass, frames: usize, events: &[(u64, NodeEvent)]) -> Vec<f32> {
    render_cut(s, frames, events, |_| 128)
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

/// The pitch, Hz, from upward zero crossings between `from` and `to`.
fn pitch(x: &[f32], from: usize, to: usize) -> f32 {
    let mut ups = Vec::new();
    for i in from.max(1)..to.min(x.len()) {
        if x[i - 1] < 0.0 && x[i] >= 0.0 {
            ups.push((i - 1) as f64 + f64::from(x[i - 1] / (x[i - 1] - x[i])));
        }
    }
    let n = ups.len();
    assert!(n >= 3, "too few cycles");
    ((n - 1) as f64 * f64::from(RATE) / (ups[n - 1] - ups[0])) as f32
}

/// The engine as a plain sine: no sub, no filter colour, no drive, no
/// glide, no velocity.
fn sine() -> Bass {
    let mut b = Bass::new(RATE);
    for (id, v) in [
        (WAVE, SINE),
        (SUB, 0.0),
        (CUTOFF, 12_000.0),
        (RESONANCE, 0.0),
        (ENV_AMOUNT, 0.0),
        (FILTER_DRIVE, 0.0),
        (DRIVE, 0.0),
        (VELOCITY, 0.0),
        (GLIDE, 0.0),
    ] {
        b.set_param(id, v);
    }
    b
}

fn preset(name: &str) -> Bass {
    let mut b = Bass::new(RATE);
    let p = Bass::presets()
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no preset {name}"));
    b.load_preset(p);
    b
}

#[test]
fn params_and_presets_are_valid() {
    let b = Bass::new(RATE);
    check_params(b.params()).unwrap();
    assert_eq!(b.info().id, "mpfree.bass");
    let names: Vec<_> = Bass::presets().iter().map(|p| p.name).collect();
    assert_eq!(names[0], "Init");
    assert_eq!(names.len(), 32);
    for (i, a) in names.iter().enumerate() {
        assert!(!names[..i].contains(a), "{a} twice");
    }
    for p in Bass::presets() {
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
    let mut b = Bass::new(RATE);
    let x = render(&mut b, 4096, &[]);
    assert!(x.iter().all(|&v| v == 0.0));
    assert_eq!(b.sounding_voices(), 0);
    let mut b = preset("Init");
    let alone = render(&mut b, 4800, &[(0, on(36, 100))]);
    let mut b = preset("Init");
    let mut l = vec![0.25f32; 4800];
    let mut r = vec![0.25f32; 4800];
    for at in (0..4800).step_by(96) {
        let ev = if at == 0 {
            vec![Timed::new(0, on(36, 100))]
        } else {
            vec![]
        };
        b.process(
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
    for name in ["Init", "808 Slide", "Reese", "Growl Press"] {
        let events = [
            (0, on(36, 100)),
            (
                ms(80.0),
                NodeEvent::Pressure {
                    note: 36,
                    value: 90,
                },
            ),
            (ms(100.0), on(43, 80)),
            (ms(150.0), NodeEvent::Bend { value: 0.4 }),
            (ms(190.0), set(CUTOFF, 400.0)),
            (ms(200.0), off(43)),
            (ms(260.0), off(36)),
            (ms(300.0), on(31, 127)),
            (ms(450.0), off(31)),
        ];
        let frames = ms(700.0) as usize;
        let mut a = preset(name);
        let x = render_cut(&mut a, frames, &events, |_| 128);
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

#[test]
fn in_tune_from_c0_to_c3_on_both_bodies() {
    for body in [SYNTH, B808] {
        for note in [12u8, 24, 36, 48] {
            let mut b = sine();
            b.set_param(BODY, body);
            b.set_param(HIGH_PASS, 10.0);
            let x = render(&mut b, RATE as usize * 2, &[(0, on(note, 100))]);
            let f = pitch(&x, RATE as usize / 10, x.len());
            let cents = 1200.0 * (f / hz(f32::from(note))).log2();
            assert!(cents.abs() < 1.0, "body {body}, note {note}: {cents} cents");
        }
    }
}

#[test]
fn the_drop_starts_high_and_lands_on_the_note() {
    let mut b = sine();
    b.set_param(BODY, B808);
    b.set_param(DROP, 12.0);
    b.set_param(DROP_TIME, 200.0);
    let x = render(&mut b, ms(500.0) as usize, &[(0, on(60, 100))]);
    let early = pitch(&x, 0, ms(12.0) as usize);
    let early_semis = 12.0 * (early / hz(60.0)).log2();
    assert!(early_semis > 9.0, "{early_semis}");
    let late = pitch(&x, ms(250.0) as usize, x.len());
    let cents = 1200.0 * (late / hz(60.0)).log2();
    assert!(cents.abs() < 1.0, "{cents}");
}

#[test]
fn a_note_starts_without_a_click() {
    for p in Bass::presets() {
        let mut b = Bass::new(RATE);
        b.load_preset(p);
        let x = render(&mut b, 256, &[(0, on(36, 127))]);
        let step = (0..4).fold(0.0f32, |m, k| {
            let prev = if k == 0 { 0.0 } else { x[k - 1] };
            m.max((x[k] - prev).abs())
        });
        assert!(db(step) < -40.0, "{}: {} dBFS", p.name, db(step));
    }
}

#[test]
fn priority_picks_the_held_key() {
    for (prio, want) in [(0.0, 43u8), (1.0, 36), (2.0, 43)] {
        let mut b = Bass::new(RATE);
        b.set_param(PRIORITY, prio);
        render(&mut b, 2048, &[(0, on(36, 100)), (512, on(43, 100))]);
        assert_eq!(b.sounding_note(), Some(want), "priority {prio}");
    }
    // Last: letting go of the top key goes back to the one still held.
    let mut b = Bass::new(RATE);
    render(
        &mut b,
        4096,
        &[(0, on(36, 100)), (512, on(43, 100)), (1024, off(43))],
    );
    assert_eq!(b.sounding_note(), Some(36));
    assert_eq!(b.keys_held(), 1);
}

#[test]
fn legato_glides_without_a_new_drop_and_retrigger_restarts() {
    // A legato note glides there and gets no new drop.
    let mut b = sine();
    b.set_param(BODY, B808);
    b.set_param(DROP, 12.0);
    b.set_param(DROP_TIME, 30.0);
    b.set_param(GLIDE, 100.0);
    let x = render(
        &mut b,
        ms(600.0) as usize,
        &[(0, on(48, 100)), (ms(200.0), on(50, 100))],
    );
    let just_after = pitch(&x, ms(200.0) as usize, ms(230.0) as usize);
    // No jump up an octave: under the new note, near the old one.
    assert!(just_after < hz(50.5), "{just_after}");
    let landed = pitch(&x, ms(450.0) as usize, x.len());
    assert!((1200.0 * (landed / hz(50.0)).log2()).abs() < 2.0);

    // With Retrigger the drop comes again on the legato note.
    let mut b = sine();
    b.set_param(BODY, B808);
    b.set_param(DROP, 12.0);
    b.set_param(DROP_TIME, 60.0);
    b.set_param(GLIDE, 0.0);
    b.set_param(RETRIGGER, 1.0);
    let x = render(
        &mut b,
        ms(400.0) as usize,
        &[(0, on(48, 100)), (ms(200.0), on(50, 100))],
    );
    let just_after = pitch(&x, ms(200.0) as usize, ms(215.0) as usize);
    assert!(just_after > hz(56.0), "{just_after}");
}

#[test]
fn glide_always_glides_from_the_last_note() {
    let mut b = sine();
    b.set_param(GLIDE, 200.0);
    b.set_param(GLIDE_ON, ALWAYS);
    let x = render(
        &mut b,
        ms(800.0) as usize,
        &[
            (0, on(36, 100)),
            (ms(200.0), off(36)),
            (ms(300.0), on(48, 100)),
        ],
    );
    let start = pitch(&x, ms(300.0) as usize, ms(340.0) as usize);
    assert!(start < hz(44.0), "{start}");
    // On Legato the detached note jumps.
    let mut b = sine();
    b.set_param(GLIDE, 200.0);
    let x = render(
        &mut b,
        ms(800.0) as usize,
        &[
            (0, on(36, 100)),
            (ms(200.0), off(36)),
            (ms(300.0), on(48, 100)),
        ],
    );
    // From 10 ms in, past the onset.
    let start = pitch(&x, ms(310.0) as usize, ms(360.0) as usize);
    assert!((1200.0 * (start / hz(48.0)).log2()).abs() < 5.0, "{start}");
}

#[test]
fn the_sub_and_fifth_stay_locked() {
    // A sine and its fifth: the sum repeats exactly every two cycles of
    // the note, so its period is the note an octave down.
    let mut b = sine();
    b.set_param(FIFTH, 100.0);
    b.set_param(SUB, 100.0);
    b.set_param(SUB_MODE, SUB_SINE1);
    let x = render(&mut b, RATE as usize, &[(0, on(48, 100))]);
    let period = RATE as f32 / hz(36.0);
    let (a, z) = (
        RATE as usize / 4,
        RATE as usize / 4 + 8 * period.round() as usize,
    );
    // The sum at one point and eight sub cycles later.
    let lag = (8.0 * period).round() as usize;
    let mut worst = 0.0f32;
    for i in a..z {
        worst = worst.max((x[i] - x[i + lag]).abs());
    }
    assert!(worst < 0.02 * peak(&x[a..z]), "{worst}");
}

#[test]
fn harder_notes_are_louder_and_brighter() {
    let mut soft = preset("Init");
    let s = render(&mut soft, 9600, &[(0, on(36, 40))]);
    let mut hard = preset("Init");
    let h = render(&mut hard, 9600, &[(0, on(36, 127))]);
    assert!(db(rms(&h[4800..])) - db(rms(&s[4800..])) > 6.0);
    // Brighter: more of the level above 400 Hz.
    let hp = |x: &[f32]| {
        let mut y = 0.0f32;
        let mut prev = 0.0f32;
        let a = (-std::f32::consts::TAU * 400.0 / RATE as f32).exp();
        x.iter()
            .map(|&v| {
                y = a * (y + v - prev);
                prev = v;
                y
            })
            .collect::<Vec<f32>>()
    };
    let share = |x: &[f32]| db(rms(&hp(x)[4800..])) - db(rms(&x[4800..]));
    assert!(share(&h) > share(&s) + 1.0, "{} {}", share(&h), share(&s));
}

#[test]
fn pressure_opens_the_filter() {
    let mut b = preset("Press Wah");
    let quiet = render(&mut b, 9600, &[(0, on(36, 100))]);
    let mut b = preset("Press Wah");
    let pressed = render(
        &mut b,
        9600,
        &[
            (0, on(36, 100)),
            (
                0,
                NodeEvent::Pressure {
                    note: 36,
                    value: 127,
                },
            ),
        ],
    );
    assert!(b.cutoff_hz() > 2.0 * 300.0, "{}", b.cutoff_hz());
    assert!(rms(&pressed[4800..]) != rms(&quiet[4800..]));
}

#[test]
fn the_mod_wheel_adds_vibrato() {
    let mut b = sine();
    let still = render(&mut b, RATE as usize, &[(0, on(48, 100))]);
    let mut b = sine();
    let wobbly = render(
        &mut b,
        RATE as usize,
        &[
            (0, on(48, 100)),
            (
                0,
                NodeEvent::Control {
                    ctl: Ctl::ModWheel,
                    value: 1.0,
                },
            ),
        ],
    );
    let spread = |x: &[f32]| {
        let w = (RATE / 20) as usize;
        let fs: Vec<f32> = (0..15)
            .map(|k| {
                pitch(
                    x,
                    RATE as usize / 4 + k * w / 2,
                    RATE as usize / 4 + k * w / 2 + w,
                )
            })
            .collect();
        let lo = fs.iter().cloned().fold(f32::MAX, f32::min);
        let hi = fs.iter().cloned().fold(0.0, f32::max);
        1200.0 * (hi / lo).log2()
    };
    assert!(spread(&still) < 1.0);
    assert!(spread(&wobbly) > 20.0, "{}", spread(&wobbly));
}

#[test]
fn a_track_at_rest_stops_running() {
    let mut b = preset("Init");
    let x = render(
        &mut b,
        ms(800.0) as usize,
        &[(0, on(36, 100)), (ms(100.0), off(36))],
    );
    assert!(peak(&x[..ms(100.0) as usize]) > 0.01);
    assert_eq!(b.sounding_voices(), 0);
    // The release (80 ms) and the quiet wait, then exact silence.
    assert!(x[ms(500.0) as usize..].iter().all(|&v| v == 0.0));
}

#[test]
fn level_is_steady_across_the_keys() {
    for name in ["Init", "808", "Reese"] {
        let level = |note: u8| {
            let mut b = preset(name);
            let x = render(&mut b, RATE as usize, &[(0, on(note, 100))]);
            db(rms(&x[RATE as usize / 4..]))
        };
        let (c1, c3) = (level(24), level(48));
        // Low notes come out a little higher, for the ear's sake, never
        // lower.
        assert!((-1.0..10.0).contains(&(c1 - c3)), "{name}: {c1} {c3}");
    }
}

#[test]
fn finite_at_the_extremes() {
    for p in PARAMS {
        for v in [p.min, p.max] {
            let mut b = Bass::new(RATE);
            b.set_param(RESONANCE, 100.0);
            b.set_param(FILTER_DRIVE, 100.0);
            b.set_param(DRIVE, 100.0);
            b.set_param(p.id, v);
            let x = render(
                &mut b,
                4800,
                &[
                    (0, on(0, 127)),
                    (1000, on(127, 1)),
                    (2000, NodeEvent::Bend { value: 1.0 }),
                    (3000, off(127)),
                ],
            );
            assert!(x.iter().all(|v| v.is_finite()), "{} = {v}", p.name);
            assert!(peak(&x) < 4.0, "{} = {v}: {}", p.name, peak(&x));
        }
    }
}
