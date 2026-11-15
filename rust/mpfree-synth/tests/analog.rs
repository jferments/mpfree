//! Behaviour tests for the vintage analog poly (Analog) through the
//! mpfree-node interface.

use mpfree_node::{check_params, Ctx, Node, NodeEvent, Timed, Tuning, MAX_BLOCK};
use mpfree_synth::analog_params::*;
use mpfree_synth::Analog;

const RATE: u32 = 48_000;

fn render_at(
    s: &mut Analog,
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
    s: &mut Analog,
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
fn cents(a: f32, b: f32) -> f32 {
    1200.0 * (a / b).log2()
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
        c += 0.5;
    }
    guess * (best.0 / 1200.0).exp2()
}

/// A plain sawtooth through an open filter, no velocity, no spread.
fn plain() -> Analog {
    let mut s = Analog::new(RATE);
    s.set_param(CUTOFF, 18_000.0);
    s.set_param(KEY_TRACK, 0.0);
    s.set_param(VEL_AMP, 0.0);
    s.set_param(AGE, 0.0);
    s.reset();
    s
}

fn preset(name: &str) -> Analog {
    let mut s = Analog::new(RATE);
    let p = Analog::presets()
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no preset {name}"));
    s.load_preset(p);
    s
}

/// Steps the engine on by `frames` with no events, in 16-frame blocks.
fn run(s: &mut Analog, from: u64, frames: u64) {
    let mut l = [0.0f32; 16];
    let mut r = [0.0f32; 16];
    let mut f = from;
    while f < from + frames {
        s.process(&Ctx::new(RATE, f), &[], &mut l, &mut r);
        f += 16;
    }
}

#[test]
fn params_and_presets_are_valid() {
    let s = Analog::new(RATE);
    check_params(s.params()).unwrap();
    for p in s.params() {
        assert!((p.id as usize) < ID_SPACE, "{}", p.name);
        assert_eq!(s.param(p.id), p.default, "{}", p.name);
    }
    for preset in Analog::presets() {
        for &(id, v) in preset.values {
            let p = s
                .params()
                .iter()
                .find(|p| p.id == id)
                .unwrap_or_else(|| panic!("{}: no param {id}", preset.name));
            assert!(
                v >= p.min && v <= p.max,
                "{}: {} = {v}",
                preset.name,
                p.name
            );
        }
    }
    assert_eq!(Analog::presets()[0].name, "Init");
}

#[test]
fn silent_until_a_note_and_adds_into_the_buffers() {
    let mut s = Analog::new(RATE);
    let (l, r) = render(&mut s, 4800, &[], |_| 128);
    assert!(l.iter().chain(&r).all(|&v| v == 0.0));
    let mut l = vec![0.25f32; 4800];
    let mut r = vec![0.25f32; 4800];
    s.process(&Ctx::new(RATE, 0), &[], &mut l, &mut r);
    assert!(l.iter().chain(&r).all(|&v| v == 0.25));
    let (l, _) = render(&mut s, 9600, &[(0, on(60, 100))], |_| 128);
    assert!(peak(&l) > 0.05);
}

#[test]
fn plays_in_tune_from_low_to_high() {
    for note in [36u8, 48, 60, 72, 84] {
        let mut s = plain();
        let (l, _) = render(&mut s, 48_000, &[(0, on(note, 100))], |_| 128);
        let f = strongest_near(&l[12_000..], hz(f32::from(note)), 40.0, RATE);
        assert!(
            cents(f, hz(f32::from(note))).abs() < 2.0,
            "note {note}: {f} Hz"
        );
    }
}

#[test]
fn same_output_for_any_block_size() {
    let setups: [&[(u16, f32)]; 4] = [
        &[],
        &[
            (SYNC, 1.0),
            (PM_ENV, 40.0),
            (OSC2_LEVEL, 50.0),
            (AGE, 100.0),
        ],
        &[
            (PM_OSC2, 60.0),
            (PM_CUTOFF, 1.0),
            (FILTER_TYPE, BP12),
            (RESONANCE, 80.0),
            (CHORUS, CHORUS_SLOW),
            (SUB_LEVEL, 50.0),
            (NOISE_LEVEL, 30.0),
        ],
        &[
            (VOICE_MODE, UNISON),
            (POLYPHONY, 4.0),
            (GLIDE, 100.0),
            (LFO_PITCH, 50.0),
            (LFO_DELAY, 300.0),
            (LFO_WAVE, 4.0),
            (CHORUS, CHORUS_VIBE),
            (RESONANCE, 100.0),
        ],
    ];
    let ev = [
        (0, on(48, 100)),
        (1000, on(55, 80)),
        (3333, on(64, 127)),
        (9000, set(CUTOFF, 900.0)),
        (12_345, off(55)),
        (20_000, on(67, 60)),
        (30_001, off(48)),
        (30_002, off(64)),
        (40_000, off(67)),
    ];
    for values in setups {
        let make = || {
            let mut s = Analog::new(RATE);
            for &(id, v) in values {
                s.set_param(id, v);
            }
            s.reset();
            s
        };
        let (a, ar) = render(&mut make(), 60_000, &ev, |_| 128);
        let sizes = [1usize, 7, 64, 333, 1024];
        let (b, br) = render(&mut make(), 60_000, &ev, |n| sizes[n % sizes.len()]);
        assert!(peak(&a) > 0.01);
        assert_eq!(a, b, "{values:?}");
        assert_eq!(ar, br, "{values:?}");
    }
}

#[test]
fn notes_take_the_cards_in_turn() {
    let mut s = plain();
    s.set_param(A_RELEASE, 5.0);
    let mut f = 0;
    let mut cards = Vec::new();
    for _ in 0..4 {
        s.process(
            &Ctx::new(RATE, f),
            &[Timed::new(0, on(60, 100))],
            &mut [0.0; 16],
            &mut [0.0; 16],
        );
        f += 16;
        cards.push((0..16).find(|&c| s.card_note(c) == Some(60)).unwrap());
        s.process(
            &Ctx::new(RATE, f),
            &[Timed::new(0, off(60))],
            &mut [0.0; 16],
            &mut [0.0; 16],
        );
        f += 16;
        run(&mut s, f, ms(400.0));
        f += ms(400.0) / 16 * 16;
        assert_eq!(s.active_voices(), 0);
    }
    assert_eq!(cards, vec![0, 1, 2, 3]);
    // A note struck again while it still sounds keeps its card.
    s.process(
        &Ctx::new(RATE, f),
        &[Timed::new(0, on(62, 100)), Timed::new(1, on(62, 100))],
        &mut [0.0; 16],
        &mut [0.0; 16],
    );
    assert_eq!(s.active_voices(), 1);
}

/// Each of eight cards' tuning (cents off its note) and cutoff, with
/// notes 60 to 67 held on them.
fn card_values(age: f32) -> Vec<(f32, f32)> {
    let mut s = plain();
    s.set_param(AGE, age);
    s.set_param(CUTOFF, 1000.0);
    s.set_param(POLYPHONY, 16.0);
    let ev: Vec<Timed> = (0..8)
        .map(|n| Timed::new(n, on(60 + n as u8, 100)))
        .collect();
    s.process(&Ctx::new(RATE, 0), &ev, &mut [0.0; 16], &mut [0.0; 16]);
    run(&mut s, 16, 64);
    (0..8)
        .map(|c| {
            let note = s.card_note(c).expect("a note on each card");
            (cents(s.card_hz(c), hz(f32::from(note))), s.card_cutoff(c))
        })
        .collect()
}

#[test]
fn age_makes_each_card_different() {
    let new = card_values(0.0);
    for &(t, c) in &new {
        assert!(t.abs() < 0.5, "{t}");
        assert!((c / new[0].1 - 1.0).abs() < 1e-3, "{c}");
    }
    let old = card_values(100.0);
    let spread = |f: &dyn Fn(&(f32, f32)) -> f32| {
        let v: Vec<f32> = old.iter().map(f).collect();
        v.iter().cloned().fold(f32::MIN, f32::max) - v.iter().cloned().fold(f32::MAX, f32::min)
    };
    let tune = spread(&|&(t, _)| t);
    let cut = spread(&|&(_, c)| 12.0 * (c / 1000.0).log2());
    assert!(tune > 3.0 && tune < 40.0, "tuning spread {tune} cents");
    assert!(cut > 0.3 && cut < 3.0, "cutoff spread {cut} semitones");
}

#[test]
fn age_drifts_slowly() {
    let drift = |age: f32| {
        let mut s = plain();
        s.set_param(AGE, age);
        s.process(
            &Ctx::new(RATE, 0),
            &[Timed::new(0, on(60, 100))],
            &mut [0.0; 16],
            &mut [0.0; 16],
        );
        let mut pitch = Vec::new();
        let mut f = 16;
        for _ in 0..50 {
            run(&mut s, f, ms(100.0));
            f += ms(100.0) / 16 * 16;
            pitch.push(cents(s.card_hz(0), hz(60.0)));
        }
        let lo = pitch.iter().cloned().fold(f32::MAX, f32::min);
        let hi = pitch.iter().cloned().fold(f32::MIN, f32::max);
        let step = pitch
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        (hi - lo, step)
    };
    let (range, _) = drift(0.0);
    assert!(range < 0.01, "{range}");
    let (range, step) = drift(100.0);
    // Over 5 s it wanders a cent or more, but never jumps.
    assert!(range > 0.5 && range < 20.0, "range {range}");
    assert!(step < 1.5, "step {step}");
}

#[test]
fn the_four_pole_cuts_steeply_and_whistles_at_full_resonance() {
    // Harmonics an octave apart above the cutoff.
    let mut s = plain();
    s.set_param(CUTOFF, 500.0);
    let (l, _) = render(&mut s, 48_000, &[(0, on(36, 100))], |_| 128);
    let f = hz(36.0);
    let slope = db(tone(&l[12_000..], f * 16.0)) - db(tone(&l[12_000..], f * 32.0)) - 6.0;
    assert!(slope > 18.0 && slope < 32.0, "slope {slope} dB/oct");
    // Noise alone into the filter at Resonance 100: a tone at the cutoff.
    let mut s = plain();
    s.set_param(OSC1_LEVEL, 0.0);
    s.set_param(NOISE_LEVEL, 5.0);
    s.set_param(CUTOFF, 1000.0);
    s.set_param(RESONANCE, 100.0);
    let (l, _) = render(&mut s, 48_000, &[(0, on(60, 100))], |_| 128);
    let x = &l[24_000..];
    let f = strongest_near(x, 1000.0, 300.0, RATE);
    assert!((cents(f, 1000.0)).abs() < 100.0, "whistles at {f} Hz");
    assert!(db(tone(x, f)) - db(tone(x, f * 1.5)) > 30.0);
    // Resonance 0: no whistle.
    s.set_param(RESONANCE, 0.0);
    let (l, _) = render(&mut s, 48_000, &[(0, on(60, 100))], |_| 128);
    assert!(rms(&l[24_000..]) < rms(x) * 0.3);
}

#[test]
fn resonance_thins_the_bass_by_about_half() {
    let fund = |reso: f32| {
        let mut s = plain();
        s.set_param(CUTOFF, 3000.0);
        s.set_param(RESONANCE, reso);
        let (l, _) = render(&mut s, 48_000, &[(0, on(36, 100))], |_| 128);
        db(tone(&l[12_000..], hz(36.0)))
    };
    let loss = fund(0.0) - fund(90.0);
    // 1/(1 + k) is 14 dB at k = 4; half is made up.
    assert!(loss > 4.0 && loss < 10.0, "{loss}");
}

#[test]
fn the_two_pole_modes_shape_the_spectrum() {
    // Each mode's response at the 1st, 12th and 48th harmonic of a saw on
    // C2, against the saw's own 1/n.
    let response = |mode: f32| {
        let mut s = plain();
        s.set_param(FILTER_TYPE, mode);
        s.set_param(CUTOFF, 800.0);
        s.set_param(RESONANCE, 30.0);
        let (l, _) = render(&mut s, 48_000, &[(0, on(36, 100))], |_| 128);
        let f = hz(36.0);
        let x = &l[12_000..];
        [1.0f32, 12.0, 48.0].map(|n| db(tone(x, f * n)) + db(n))
    };
    let [lo, mid, hi] = response(LP12);
    assert!(lo > hi + 15.0 && mid > hi + 15.0, "LP {lo} {mid} {hi}");
    let [lo, mid, hi] = response(HP12);
    assert!(
        lo < mid - 20.0 && (hi - mid).abs() < 6.0,
        "HP {lo} {mid} {hi}"
    );
    let [lo, mid, hi] = response(BP12);
    assert!(mid > lo + 10.0 && mid > hi + 10.0, "BP {lo} {mid} {hi}");
    // The two-pole falls about half as fast as the four-pole.
    let slope = |mode: f32| {
        let mut s = plain();
        s.set_param(FILTER_TYPE, mode);
        s.set_param(CUTOFF, 400.0);
        let (l, _) = render(&mut s, 48_000, &[(0, on(36, 100))], |_| 128);
        let f = hz(36.0);
        db(tone(&l[12_000..], f * 16.0)) - db(tone(&l[12_000..], f * 32.0)) - 6.0
    };
    let s12 = slope(LP12);
    assert!(s12 > 8.0 && s12 < 16.0, "{s12}");
}

#[test]
fn the_attack_bows_as_an_rc_toward_more_than_the_peak() {
    let mut s = plain();
    s.set_param(OSC1_WAVE, TRIANGLE);
    s.set_param(A_ATTACK, 200.0);
    s.set_param(A_DECAY, 20_000.0);
    let (l, _) = render(&mut s, ms(400.0) as usize, &[(0, on(72, 100))], |_| 128);
    let level = |t: f32| peak(&l[ms(t - 3.0) as usize..ms(t + 3.0) as usize]);
    let full = level(300.0);
    let half = level(100.0) / full;
    // 1.3 (1 - 0.23^0.5) = 0.675 halfway; a straight line would be 0.5.
    assert!((half - 0.675).abs() < 0.06, "halfway {half}");
    assert!(level(200.0) / full > 0.95);
}

#[test]
fn sync_locks_oscillator_1_to_oscillator_2() {
    let low = |sync: f32| {
        let mut s = plain();
        s.set_param(SYNC, sync);
        // Oscillator 2, unheard, a fourth and an octave under.
        s.set_param(OSC2_OCT, -1.0);
        s.set_param(OSC2_SEMI, -5.0);
        s.set_param(OSC2_FINE, 0.0);
        let (l, _) = render(&mut s, 48_000, &[(0, on(60, 100))], |_| 128);
        let x = &l[12_000..];
        db(tone(x, hz(43.0))) - db(tone(x, hz(60.0)))
    };
    assert!(low(0.0) < -50.0, "{}", low(0.0));
    assert!(low(1.0) > -20.0, "{}", low(1.0));
}

#[test]
fn poly_mod_from_oscillator_2_adds_sidebands() {
    let off_harmonics = |amount: f32| {
        let mut s = plain();
        s.set_param(OSC1_WAVE, TRIANGLE);
        s.set_param(OSC2_SEMI, 7.0);
        s.set_param(OSC2_FINE, 13.0);
        s.set_param(PM_OSC2, amount);
        let (l, _) = render(&mut s, 48_000, &[(0, on(60, 100))], |_| 128);
        let x = &l[12_000..];
        let f = hz(60.0);
        let total = rms(x);
        let harm: f32 = (1..30)
            .map(|n| tone(x, f * n as f32).powi(2) * 0.5)
            .sum::<f32>()
            .sqrt();
        (total * total - harm * harm).max(0.0).sqrt() / total
    };
    assert!(off_harmonics(0.0) < 0.05, "{}", off_harmonics(0.0));
    assert!(off_harmonics(60.0) > 0.3, "{}", off_harmonics(60.0));
}

#[test]
fn the_chorus_widens_without_much_more_level() {
    let play = |mode: f32| {
        let mut s = plain();
        s.set_param(CUTOFF, 3000.0);
        s.set_param(CHORUS, mode);
        s.reset();
        render(
            &mut s,
            96_000,
            &[(0, on(48, 100)), (0, on(55, 100)), (0, on(60, 100))],
            |_| 128,
        )
    };
    let (dl, dr) = play(CHORUS_OFF);
    assert_eq!(dl, dr);
    for mode in [CHORUS_SLOW, CHORUS_FAST, CHORUS_VIBE] {
        let (l, r) = play(mode);
        let side: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
        assert!(
            rms(&side[24_000..]) > rms(&l[24_000..]) * 0.05,
            "mode {mode}"
        );
        let d = db(rms(&l[24_000..])) - db(rms(&dl[24_000..]));
        assert!(d.abs() < 3.0, "mode {mode}: {d} dB");
    }
}

#[test]
fn letting_go_ends_in_exact_silence() {
    for mode in [CHORUS_OFF, CHORUS_SLOW] {
        let mut s = plain();
        s.set_param(CHORUS, mode);
        s.set_param(A_RELEASE, 100.0);
        let (l, r) = render(
            &mut s,
            48_000,
            &[(0, on(60, 127)), (ms(200.0), off(60))],
            |_| 128,
        );
        let after = ms(200.0 + 260.0) as usize;
        assert!(peak(&l[ms(150.0) as usize..ms(200.0) as usize]) > 0.05);
        assert!(
            l[after..].iter().chain(&r[after..]).all(|&v| v == 0.0),
            "mode {mode}"
        );
        assert_eq!(s.active_voices(), 0);
    }
}

#[test]
fn unison_stacks_every_voice_on_the_note() {
    let mut s = plain();
    s.set_param(VOICE_MODE, UNISON);
    s.set_param(POLYPHONY, 4.0);
    s.set_param(UNI_DETUNE, 100.0);
    s.process(
        &Ctx::new(RATE, 0),
        &[Timed::new(0, on(60, 100))],
        &mut [0.0; 16],
        &mut [0.0; 16],
    );
    run(&mut s, 16, 64);
    let mut hzs: Vec<f32> = (0..4).map(|c| s.card_hz(c)).collect();
    assert!((0..4).all(|c| s.card_note(c) == Some(60)));
    assert_eq!(s.card_note(4), None);
    hzs.sort_by(f32::total_cmp);
    let spread = cents(hzs[3], hzs[0]);
    assert!((spread - 50.0).abs() < 1.0, "{spread}");
    // A second key takes all of them; letting it go returns to the first.
    s.process(
        &Ctx::new(RATE, 96),
        &[Timed::new(0, on(67, 100))],
        &mut [0.0; 16],
        &mut [0.0; 16],
    );
    assert!((0..4).all(|c| s.card_note(c) == Some(67)));
    s.process(
        &Ctx::new(RATE, 112),
        &[Timed::new(0, off(67))],
        &mut [0.0; 16],
        &mut [0.0; 16],
    );
    assert!((0..4).all(|c| s.card_note(c) == Some(60)));
}

#[test]
fn glide_moves_a_card_from_its_last_note() {
    let mut s = plain();
    s.set_param(GLIDE, 300.0);
    s.set_param(POLYPHONY, 1.0);
    s.process(
        &Ctx::new(RATE, 0),
        &[Timed::new(0, on(48, 100))],
        &mut [0.0; 16],
        &mut [0.0; 16],
    );
    run(&mut s, 16, ms(200.0));
    let mut f = 16 + ms(200.0) / 16 * 16;
    assert!((cents(s.card_hz(0), hz(48.0))).abs() < 1.0);
    s.process(
        &Ctx::new(RATE, f),
        &[Timed::new(0, on(60, 100))],
        &mut [0.0; 16],
        &mut [0.0; 16],
    );
    f += 16;
    run(&mut s, f, ms(100.0));
    f += ms(100.0) / 16 * 16;
    let mid = 12.0 * (s.card_hz(0) / hz(48.0)).log2();
    // Three time constants in 300 ms: a third of the way is 1 - e^-1.
    assert!((mid - 12.0 * (1.0 - (-1.0f32).exp())).abs() < 1.0, "{mid}");
    run(&mut s, f, ms(600.0));
    assert!((cents(s.card_hz(0), hz(60.0))).abs() < 5.0);
}

#[test]
fn the_lfo_delay_fades_the_vibrato_in() {
    let mut s = plain();
    s.set_param(LFO_PITCH, 100.0);
    s.set_param(LFO_RATE, 5.0);
    s.set_param(LFO_WAVE, 3.0);
    s.set_param(LFO_DELAY, 1000.0);
    s.process(
        &Ctx::new(RATE, 0),
        &[Timed::new(0, on(60, 100))],
        &mut [0.0; 16],
        &mut [0.0; 16],
    );
    let mut f = 16;
    let mut dev = |s: &mut Analog, until: u64| {
        let mut worst = 0.0f32;
        while f < until {
            run(s, f, 16);
            f += 16;
            worst = worst.max(cents(s.card_hz(0), hz(60.0)).abs());
        }
        worst
    };
    let early = dev(&mut s, ms(100.0));
    let late = dev(&mut s, ms(1500.0));
    assert!(early < 15.0, "{early}");
    assert!(late > 90.0, "{late}");
}

#[test]
fn every_preset_sits_near_the_level_convention() {
    for p in Analog::presets() {
        let mut s = Analog::new(RATE);
        s.load_preset(p);
        let (l, _) = render(&mut s, 48_000, &[(0, on(60, 110))], |_| 128);
        let one = db(peak(&l));
        assert!(one > -22.0 && one < -6.0, "{}: one note {one}", p.name);
        let mut s = Analog::new(RATE);
        s.load_preset(p);
        let ev: Vec<_> = [48u8, 55, 60, 64, 67, 72]
            .iter()
            .map(|&n| (0, on(n, 127)))
            .collect();
        let (l, r) = render(&mut s, 96_000, &ev, |_| 128);
        let chord = db(peak(&l).max(peak(&r)));
        assert!(chord < 0.0, "{}: chord {chord}", p.name);
    }
}

#[test]
fn finite_at_the_extremes() {
    let mut s = Analog::new(RATE);
    for (id, v) in [
        (RESONANCE, 100.0),
        (DRIVE, 100.0),
        (PM_OSC2, 100.0),
        (PM_ENV, 100.0),
        (PM_PW, 1.0),
        (PM_CUTOFF, 1.0),
        (SYNC, 1.0),
        (OSC2_LEVEL, 100.0),
        (SUB_LEVEL, 100.0),
        (NOISE_LEVEL, 100.0),
        (ENV_AMT, 100.0),
        (LFO_CUTOFF, 100.0),
        (LFO_PW, 100.0),
        (AGE, 100.0),
        (CHORUS, CHORUS_FAST),
        (F_ATTACK, 1.0),
        (A_ATTACK, 1.0),
    ] {
        s.set_param(id, v);
    }
    for ft in [LP24, LP12, BP12, HP12] {
        s.set_param(FILTER_TYPE, ft);
        let ev: Vec<_> = [0u8, 24, 60, 96, 120, 127]
            .iter()
            .map(|&n| (0, on(n, 127)))
            .collect();
        let (l, r) = render(&mut s, 24_000, &ev, |_| 128);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()), "filter {ft}");
        assert!(peak(&l) < 8.0, "filter {ft}: {}", peak(&l));
        s.reset();
    }
}

#[test]
fn in_tune_and_at_the_same_level_at_other_rates() {
    let level = |rate: u32| {
        let mut s = Analog::new(rate);
        s.set_param(AGE, 0.0);
        s.set_param(CUTOFF, 2000.0);
        let frames = rate as usize;
        let (l, _) = render_at(&mut s, rate, frames, &[(0, on(57, 100))], |_| 128);
        let x = &l[frames / 4..];
        let f = strongest_near(x, 220.0, 30.0, rate);
        assert!((cents(f, 220.0)).abs() < 2.0, "{rate}: {f}");
        db(rms(x))
    };
    let base = level(48_000);
    for rate in [44_100, 96_000] {
        let d = level(rate) - base;
        assert!(d.abs() < 0.5, "{rate}: {d} dB");
    }
}

#[test]
fn follows_the_project_tuning() {
    let mut s = plain();
    let t = Tuning::equal(12, 442.0);
    s.set_tuning(&t);
    let (l, _) = render(&mut s, 48_000, &[(0, on(69, 100))], |_| 128);
    let f = strongest_near(&l[12_000..], 442.0, 20.0, RATE);
    assert!((cents(f, 442.0)).abs() < 1.5, "{f}");
}

#[test]
fn reset_stops_everything() {
    let mut s = preset("Warm Pad");
    render(&mut s, 9600, &[(0, on(60, 100)), (0, on(64, 100))], |_| 128);
    assert!(s.active_voices() > 0);
    s.reset();
    assert_eq!(s.active_voices(), 0);
    let (l, r) = render(&mut s, 4800, &[], |_| 128);
    assert!(l.iter().chain(&r).all(|&v| v == 0.0));
}

#[test]
#[ignore]
fn print_preset_levels() {
    for p in Analog::presets() {
        let mut s = Analog::new(RATE);
        s.load_preset(p);
        let (l, _) = render(&mut s, 48_000, &[(0, on(60, 110))], |_| 128);
        let mut c = Analog::new(RATE);
        c.load_preset(p);
        let ev: Vec<_> = [48u8, 55, 60, 64, 67, 72]
            .iter()
            .map(|&n| (0, on(n, 127)))
            .collect();
        let (cl, cr) = render(&mut c, 96_000, &ev, |_| 128);
        println!(
            "{:<15} note peak {:6.1} dB rms {:6.1} dB, chord peak {:6.1} dB",
            p.name,
            db(peak(&l)),
            db(rms(&l)),
            db(peak(&cl).max(peak(&cr)))
        );
    }
}
