//! Behaviour tests for the tonewheel organ (Drawbar) through the
//! mpfree-node interface.

use mpfree_node::{check_params, Ctx, Node, NodeEvent, Timed, Tuning, MAX_BLOCK};
use mpfree_synth::drawbar::{bar_level, gear_hz, wheel_for, N_WHEELS};
use mpfree_synth::drawbar_params::*;
use mpfree_synth::Drawbar;

const RATE: u32 = 48_000;

fn render(
    s: &mut Drawbar,
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

fn on(note: u8) -> NodeEvent {
    NodeEvent::NoteOn {
        note,
        velocity: 100,
    }
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

fn hz(note: f32) -> f32 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}

/// The amplitude of the `hz` component of `x` (a Hann-windowed Goertzel).
fn tone(x: &[f32], hz: f32) -> f32 {
    let n = x.len();
    let w = std::f64::consts::TAU * f64::from(hz) / f64::from(RATE);
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, &v) in x.iter().enumerate() {
        let h = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
        re += f64::from(v) * h * (w * i as f64).cos();
        im += f64::from(v) * h * (w * i as f64).sin();
    }
    ((re * re + im * im).sqrt() * 4.0 / n as f64) as f32
}

/// The frequency of the strongest component within 30 cents of `guess`.
fn pitch_near(x: &[f32], guess: f32) -> f32 {
    let at = |c: f32| tone(x, guess * (c / 1200.0).exp2());
    let mut best = (-30.0f32, 0.0f32);
    let mut c = -30.0f32;
    while c <= 30.0 {
        let a = at(c);
        if a > best.1 {
            best = (c, a);
        }
        c += 0.25;
    }
    let (a, b, d) = (at(best.0 - 0.25), best.1, at(best.0 + 0.25));
    let off = 0.125 * (a - d) / (a - 2.0 * b + d);
    guess * ((best.0 + off) / 1200.0).exp2()
}

/// An organ with nothing on top of the wheels: no rotary, vibrato, drive,
/// leakage or click, the drawbars as given ("008000000").
fn bare(bars: &[u8; 9]) -> Drawbar {
    let mut s = Drawbar::new(RATE);
    for (i, &(id, _, _)) in DRAWBARS.iter().enumerate() {
        s.set_param(id, f32::from(bars[i] - b'0'));
    }
    s.set_param(ROTARY, 0.0);
    s.set_param(VIBRATO, 0.0);
    s.set_param(DRIVE, 0.0);
    s.set_param(LEAKAGE, 0.0);
    s.set_param(CLICK, 0.0);
    s.reset();
    s
}

fn preset(name: &str) -> Drawbar {
    let mut s = Drawbar::new(RATE);
    let p = Drawbar::presets()
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no preset {name}"));
    s.load_preset(p);
    s
}

#[test]
fn parameters_and_presets_are_valid() {
    check_params(PARAMS).unwrap();
    for p in PARAMS {
        assert!((p.id as usize) < ID_SPACE, "{} beyond the id space", p.name);
    }
    let mut names = std::collections::HashSet::new();
    for p in Drawbar::presets() {
        assert!(names.insert(p.name), "{} twice", p.name);
        for &(id, v) in p.values {
            let q = PARAMS
                .iter()
                .find(|q| q.id == id)
                .unwrap_or_else(|| panic!("{}: no param {id}", p.name));
            assert!(v >= q.min && v <= q.max, "{}: {} = {v}", p.name, q.name);
        }
    }
    let s = Drawbar::new(RATE);
    assert_eq!(s.info().id, "mpfree.drawbar");
    assert!(s.param(9999).is_nan());
}

#[test]
fn the_gears_give_the_published_pitches() {
    // The lowest and highest wheels, and A 440 exactly.
    assert!((gear_hz(0) - 32.692).abs() < 0.001, "{}", gear_hz(0));
    assert!(
        (gear_hz(N_WHEELS - 1) - 5924.57).abs() < 0.01,
        "{}",
        gear_hz(90)
    );
    assert_eq!(gear_hz(45), 440.0);
    // Within 0.7 cents of equal temperament below the top seven wheels,
    // within 2 cents (sharp) above.
    let mut worst = 0.0f64;
    for w in 0..N_WHEELS {
        let et = 440.0 * ((w as f64 + 24.0 - 69.0) / 12.0).exp2();
        let cents = 1200.0 * (gear_hz(w) / et).log2();
        if w < 84 {
            worst = worst.max(cents.abs());
        } else {
            assert!(cents > -0.5 && cents < 2.0, "wheel {}: {cents:+.2}", w + 1);
        }
    }
    // G#, 0.71 cents flat.
    assert!(worst < 0.71 && worst > 0.70, "worst {worst:.3} cents");
}

#[test]
fn foldback_keeps_every_drawbar_on_a_wheel() {
    for note in 0..128 {
        for bar in 0..9 {
            assert!(wheel_for(note, bar) < N_WHEELS);
        }
    }
    // The 16' in the lowest manual octave doubles the 8'.
    for note in 36..48 {
        assert_eq!(wheel_for(note, 0), wheel_for(note, 2), "note {note}");
    }
    assert_eq!(wheel_for(48, 0), wheel_for(36, 2));
    // The top key's 1' (C 3 octaves up) folds down an octave; its 8' doesn't.
    assert_eq!(wheel_for(96, 8), wheel_for(84, 8));
    assert_eq!(wheel_for(96, 2), 72);
    // Drawbar steps are 3 dB apart.
    for pos in 1..8 {
        let step = db(bar_level(pos as f32 + 1.0)) - db(bar_level(pos as f32));
        assert!((step - 3.0).abs() < 1e-3);
    }
    assert_eq!(bar_level(0.0), 0.0);
    assert_eq!(bar_level(8.0), 1.0);
}

#[test]
fn each_drawbar_sounds_its_harmonic_in_tune() {
    for (bar, &(id, name, semis)) in DRAWBARS.iter().enumerate() {
        let mut s = bare(b"000000000");
        s.set_param(id, 8.0);
        s.set_param(WHEELS, 1.0);
        let note = 60u8;
        let (l, r) = render(&mut s, 24_000, &[(0, on(note))], |_| 128);
        assert_eq!(l, r, "{name}: mono without the rotary");
        let want = hz(f32::from(note) + semis as f32);
        let got = pitch_near(&l[4800..], want);
        let cents = 1200.0 * (got / want).log2();
        assert!(cents.abs() < 0.5, "{name}: {got:.2} for {want:.2} Hz");
        // Nothing else of note: the wheel's own tone dominates.
        let a = tone(&l[4800..], want);
        assert!(a > 0.02, "{name} (bar {bar}): {a}");
        let other = tone(&l[4800..], hz(f32::from(note) + semis as f32 + 5.0));
        assert!(other < a * 0.001, "{name}: stray {other} against {a}");
    }
}

#[test]
fn gears_and_equal_tuning() {
    // Gears: A 440 exactly, G# 0.69 cents flat.
    let mut s = bare(b"008000000");
    let (l, _) = render(&mut s, 24_000, &[(0, on(69)), (0, on(68))], |_| 128);
    let a = pitch_near(&l[4800..], 440.0);
    assert!((a - 440.0).abs() < 0.05, "{a}");
    let gs = pitch_near(&l[4800..], hz(68.0));
    let cents = 1200.0 * (gs / hz(68.0)).log2();
    assert!((cents + 0.69).abs() < 0.2, "G#: {cents:+.2} cents");
    // Both follow the project's tuning; the gears keep their offsets on it.
    let t = Tuning::equal(12, 440.0 * 1.01);
    for wheels in [0.0, 1.0] {
        let mut s = bare(b"008000000");
        s.set_param(WHEELS, wheels);
        s.set_tuning(&t);
        let (l, _) = render(&mut s, 24_000, &[(0, on(69)), (0, on(68))], |_| 128);
        let a = pitch_near(&l[4800..], 444.4);
        assert!((a / 444.4 - 1.0).abs() < 1e-4, "wheels {wheels}: {a}");
        let gs = pitch_near(&l[4800..], hz(68.0) * 1.01);
        let cents = 1200.0 * (gs / (hz(68.0) * 1.01)).log2();
        let want = if wheels == 0.0 { -0.71 } else { 0.0 };
        assert!(
            (cents - want).abs() < 0.2,
            "wheels {wheels}: G# {cents:+.2}"
        );
    }
}

#[test]
fn transpose_and_fine_move_the_wheels() {
    let mut s = bare(b"008000000");
    s.set_param(WHEELS, 1.0);
    s.set_param(TRANSPOSE, -12.0);
    s.set_param(FINE, 25.0);
    let (l, _) = render(&mut s, 24_000, &[(0, on(69))], |_| 128);
    let want = 220.0 * (25.0f32 / 1200.0).exp2();
    let got = pitch_near(&l[4800..], want);
    assert!((1200.0 * (got / want).log2()).abs() < 0.5, "{got}");
}

#[test]
fn the_lowest_wheels_are_nearly_square() {
    // Note 24 sounds wheel 1: odd harmonics at 1/3 and 1/5.
    let mut s = bare(b"008000000");
    let (l, _) = render(&mut s, 48_000, &[(0, on(24))], |_| 128);
    let f = gear_hz(0) as f32;
    let (a1, a3, a5) = (
        tone(&l[4800..], f),
        tone(&l[4800..], 3.0 * f),
        tone(&l[4800..], 5.0 * f),
    );
    assert!((a3 / a1 - 1.0 / 3.0).abs() < 0.02, "{a1} {a3}");
    assert!((a5 / a1 - 0.2).abs() < 0.02, "{a1} {a5}");
    // A manual key's wheel is a sine.
    let mut s = bare(b"008000000");
    let (l, _) = render(&mut s, 48_000, &[(0, on(48))], |_| 128);
    let f = gear_hz(24) as f32;
    assert!(tone(&l[4800..], 3.0 * f) < tone(&l[4800..], f) * 1e-3);
}

#[test]
fn a_wheel_two_keys_share_sums_once() {
    // C4's 4' and C5's 8' are the same wheel: together they are one sine
    // at twice the level, not two beating ones.
    let mut s = bare(b"000800000");
    let (one, _) = render(&mut s, 24_000, &[(0, on(60))], |_| 128);
    let mut s = bare(b"008800000");
    let (both, _) = render(&mut s, 24_000, &[(0, on(60)), (0, on(72))], |_| 128);
    // Wheel 49 (C5): C4's 4' and C5's 8'.
    let f = gear_hz(48) as f32;
    let a1 = tone(&one[4800..], f);
    let a2 = tone(&both[4800..], f);
    assert!((a2 / a1 - 2.0).abs() < 0.01, "{a1} {a2}");
    // And steady: no beating between halves.
    let (h1, h2) = (rms(&both[4800..14_400]), rms(&both[14_400..24_000]));
    assert!((h1 / h2 - 1.0).abs() < 0.01);
}

#[test]
fn same_output_for_any_block_size() {
    let ev = vec![
        (0, on(48)),
        (5, on(55)),
        (7, on(60)),
        (3_001, set(VIBRATO, C3)),
        (6_000, off(55)),
        (7_000, set(SPEED, SPEED_FAST)),
        (8_000, set(TONE, -60.0)),
        (8_500, set(DRIVE, 60.0)),
        (9_000, on(48)),
        (9_100, off(48)),
        (12_000, set(PERC, 1.0)),
        (12_100, off(60)),
        (12_100, off(48)),
        (12_500, on(72)),
        (12_500, on(76)),
        (13_000, set(DB_1_35, 6.0)),
        (14_000, set(ROTARY, 0.0)),
        (15_000, set(ROTARY, 1.0)),
        (16_000, set(SPEED, STOP)),
        (17_000, set(LEAKAGE, 100.0)),
        (18_000, NodeEvent::AllOff),
        (19_000, on(36)),
    ];
    let mut a = Drawbar::new(RATE);
    let want = render(&mut a, 24_000, &ev, |_| 128);
    assert!(peak(&want.0) > 0.01);
    for cut in [1usize, 3, 16, 17, 64, 100] {
        let mut b = Drawbar::new(RATE);
        let got = render(&mut b, 24_000, &ev, |_| cut);
        assert!(got == want, "blocks of {cut} differ");
    }
    let mut seed = 7u32;
    let mut b = Drawbar::new(RATE);
    let got = render(&mut b, 24_000, &ev, |_| {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        1 + (seed >> 16) as usize % 200
    });
    assert!(got == want, "random blocks differ");
}

#[test]
fn adds_to_what_is_in_the_buffer() {
    let mut a = Drawbar::new(RATE);
    let mut b = Drawbar::new(RATE);
    let ev = [Timed::new(0, on(60))];
    let ctx = Ctx::new(RATE, 0);
    let (mut l1, mut r1) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    let (mut l2, mut r2) = (vec![0.25f32; 512], vec![-0.25f32; 512]);
    a.process(&ctx, &ev, &mut l1, &mut r1);
    b.process(&ctx, &ev, &mut l2, &mut r2);
    for i in 0..512 {
        assert!((l2[i] - 0.25 - l1[i]).abs() < 1e-6);
        assert!((r2[i] + 0.25 - r1[i]).abs() < 1e-6);
    }
}

/// The level of the harmonic `semis` above `note` across `x`.
fn harmonic(x: &[f32], note: u8, semis: i32) -> f32 {
    tone(x, gear_hz(wheel_for(i32::from(note) + semis, 2)) as f32)
}

#[test]
fn percussion_is_single_trigger_and_polyphonic() {
    let mut s = bare(b"008000000");
    s.set_param(PERC, 1.0);
    s.set_param(PERC_HARM, THIRD);
    s.set_param(PERC_DECAY, FAST);
    // Detached: C4, let go, then E4: both get the twelfth.
    let (l, _) = render(
        &mut s,
        96_000,
        &[(0, on(60)), (24_000, off(60)), (48_000, on(64))],
        |_| 128,
    );
    let first = harmonic(&l[0..4800], 60, 19);
    let detached = harmonic(&l[48_000..52_800], 64, 19);
    assert!(first > 0.02, "{first}");
    assert!((detached / first - 1.0).abs() < 0.1, "{first} {detached}");
    // Legato: E4 while C4 is still held gets none once C4's has died.
    let mut s = bare(b"008000000");
    s.set_param(PERC, 1.0);
    s.set_param(PERC_DECAY, FAST);
    let (l, _) = render(&mut s, 120_000, &[(0, on(60)), (96_000, on(64))], |_| 128);
    let legato = harmonic(&l[96_000..100_800], 64, 19);
    assert!(legato < first * 1e-3, "{legato} against {first}");
    // A chord struck together all gets it.
    let mut s = bare(b"008000000");
    s.set_param(PERC, 1.0);
    let (l, _) = render(&mut s, 4800, &[(0, on(60)), (0, on(64))], |_| 128);
    let (c, e) = (harmonic(&l, 60, 19), harmonic(&l, 64, 19));
    assert!(c > 0.02 && (e / c - 1.0).abs() < 0.1, "{c} {e}");
}

#[test]
fn percussion_decays_fast_or_slow_and_soft_is_6_db_down() {
    let fall = |decay: f32, vol: f32| {
        let mut s = bare(b"008000000");
        s.set_param(PERC, 1.0);
        s.set_param(PERC_HARM, SECOND);
        s.set_param(PERC_DECAY, decay);
        s.set_param(PERC_VOL, vol);
        let (l, _) = render(&mut s, 48_000, &[(0, on(60))], |_| 128);
        let a = harmonic(&l[2400..7200], 60, 12);
        let b = harmonic(&l[26_400..31_200], 60, 12);
        (a, db(a) - db(b))
    };
    // 0.5 s apart: 30 dB for a 1 s fall to -60 dB, 7.5 dB for 4 s.
    let (normal, fast) = fall(FAST, 0.0);
    let (_, slow) = fall(SLOW, 0.0);
    let (soft, _) = fall(FAST, SOFT);
    assert!((fast - 30.0).abs() < 2.0, "fast {fast:.1} dB");
    assert!((slow - 7.5).abs() < 1.0, "slow {slow:.1} dB");
    let gap = db(normal) - db(soft);
    assert!((gap - 6.0).abs() < 0.3, "soft {gap:.1} dB down");
}

#[test]
fn percussion_takes_the_1_foot_and_normal_lowers_the_drawbars() {
    let sustain = |perc: f32, vol: f32| {
        let mut s = bare(b"008000008");
        s.set_param(PERC, perc);
        s.set_param(PERC_VOL, vol);
        let (l, _) = render(&mut s, 336_000, &[(0, on(48))], |_| 128);
        // After the percussion has died (slow: 4 s to -60 dB).
        let x = &l[288_000..];
        (harmonic(x, 48, 0), harmonic(x, 48, 36))
    };
    let (eight, one) = sustain(0.0, 0.0);
    assert!(one > 0.02 && (one / eight - 1.0).abs() < 0.05);
    let (eight_n, one_n) = sustain(1.0, 0.0);
    assert!(one_n < eight * 1e-3, "the 1' still sounds: {one_n}");
    assert!(
        (db(eight) - db(eight_n) - 3.0).abs() < 0.3,
        "{eight} {eight_n}"
    );
    let (eight_s, one_s) = sustain(1.0, SOFT);
    assert!(one_s < eight * 1e-3);
    assert!((eight_s / eight - 1.0).abs() < 0.01);
}

#[test]
fn leakage_adds_the_compartment_partner() {
    let partner = |leak: f32| {
        let mut s = bare(b"008000000");
        s.set_param(LEAKAGE, leak);
        let (l, _) = render(&mut s, 24_000, &[(0, on(48))], |_| 128);
        let main = tone(&l[4800..], gear_hz(24) as f32);
        let up = tone(&l[4800..], gear_hz(72) as f32);
        db(up) - db(main)
    };
    assert!(partner(0.0) < -100.0);
    assert!((partner(40.0) + 40.0).abs() < 1.0, "{}", partner(40.0));
    assert!((partner(100.0) + 26.0).abs() < 1.0, "{}", partner(100.0));
}

/// `x` through a 4th-order Butterworth high-pass at `hz`.
fn highpass(x: &[f32], hz: f32) -> Vec<f32> {
    let mut out = x.to_vec();
    for q in [0.5412f32, 1.3066] {
        let w = std::f32::consts::TAU * hz / RATE as f32;
        let (sn, cs) = w.sin_cos();
        let alpha = sn / (2.0 * q);
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = (
            (1.0 + cs) / 2.0 / a0,
            -(1.0 + cs) / a0,
            (1.0 + cs) / 2.0 / a0,
        );
        let (a1, a2) = (-2.0 * cs / a0, (1.0 - alpha) / a0);
        let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for v in out.iter_mut() {
            let y = b0 * *v + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
            x2 = x1;
            x1 = *v;
            y2 = y1;
            y1 = y;
            *v = y;
        }
    }
    out
}

#[test]
fn click_makes_the_attack_sharper() {
    let attack = |click: f32| {
        let mut s = bare(b"888000000");
        s.set_param(CLICK, click);
        s.reset();
        let (l, _) = render(&mut s, 24_000, &[(960, on(60))], |_| 128);
        // What lies above 2 kHz in the first 5 ms (the wheels are all
        // below 400 Hz), against the steady note's level.
        let hp = highpass(&l, 2000.0);
        rms(&hp[960..1200]) / rms(&l[12_000..20_000])
    };
    let (none, half, full) = (attack(0.0), attack(50.0), attack(100.0));
    assert!(none < 0.005, "no click: {none}");
    assert!(half > none * 4.0, "{none} {half}");
    assert!(full > half * 1.5, "{half} {full}");
}

#[test]
fn letting_go_ends_in_exact_silence() {
    for name in ["Init", "Jazz Perc", "Gospel", "Rock Drive", "Church"] {
        let mut s = preset(name);
        let tail = s.tail_frames() as usize;
        let (l, r) = render(
            &mut s,
            96_000,
            &[
                (0, on(60)),
                (0, on(64)),
                (0, on(67)),
                (24_000, NodeEvent::AllOff),
            ],
            |_| 128,
        );
        assert_eq!(s.keys_sounding(), 0, "{name}");
        assert_eq!(s.active_voices(), 0, "{name}: still counted");
        let after = 24_000 + tail;
        assert!(
            peak(&l[after..]) < 1e-4 && peak(&r[after..]) < 1e-4,
            "{name}: {} after the tail",
            peak(&l[after..])
        );
        assert!(l[72_000..].iter().all(|&v| v == 0.0), "{name}: not silent");
    }
}

#[test]
fn the_voice_count_covers_the_tail() {
    let mut s = Drawbar::new(RATE);
    let tail = s.tail_frames() as usize;
    render(
        &mut s,
        4800,
        &[(0, on(60)), (0, on(64)), (2400, off(60))],
        |_| 128,
    );
    assert_eq!(s.active_voices(), 1);
    render(&mut s, 4800, &[(0, off(64))], |_| 128);
    // The contacts have opened, but the speaker still rings.
    assert_eq!(s.keys_sounding(), 0);
    assert_eq!(s.active_voices(), 1);
    render(&mut s, tail, &[], |_| 128);
    assert_eq!(s.active_voices(), 0);
}

#[test]
fn reset_silences_at_once() {
    let mut s = preset("Gospel");
    render(&mut s, 4800, &[(0, on(60)), (0, on(67))], |_| 128);
    s.reset();
    let (l, r) = render(&mut s, 4800, &[], |_| 128);
    assert!(l.iter().chain(&r).all(|&v| v == 0.0));
}

#[test]
fn rotary_off_or_spread_0_is_mono() {
    let mut s = Drawbar::new(RATE);
    s.set_param(ROTARY, 0.0);
    s.reset();
    let (l, r) = render(&mut s, 12_000, &[(0, on(60)), (0, on(79))], |_| 128);
    assert_eq!(l, r);
    let mut s = Drawbar::new(RATE);
    s.set_param(SPREAD, 0.0);
    let (l, r) = render(&mut s, 12_000, &[(0, on(60)), (0, on(79))], |_| 128);
    assert_eq!(l, r);
    let mut s = Drawbar::new(RATE);
    let (l, r) = render(&mut s, 12_000, &[(0, on(60)), (0, on(79))], |_| 128);
    assert!(l != r);
    let diff: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
    assert!(rms(&diff) > rms(&l) * 0.1, "spread 70 barely wide");
}

/// How much the level of `x` swings, in dB: its power through two
/// one-poles at 20 Hz, over all but the first 0.2 s.
fn swing(x: &[f32]) -> f32 {
    let a = 1.0 - (-std::f32::consts::TAU * 20.0 / RATE as f32).exp();
    let (mut p1, mut p2) = (0.0f32, 0.0f32);
    let (mut lo, mut hi) = (f32::MAX, 0.0f32);
    for (i, &v) in x.iter().enumerate() {
        p1 += (v * v - p1) * a;
        p2 += (p1 - p2) * a;
        if i >= 9600 {
            lo = lo.min(p2);
            hi = hi.max(p2);
        }
    }
    10.0 * hi.log10() - 10.0 * lo.max(1e-24).log10()
}

#[test]
fn the_rotary_moves_the_sound() {
    // A high note (horn) and a low one (drum) at Fast: the level swings
    // as each rotor faces the microphone and away; without the rotary it
    // is steady.
    for note in [84u8, 43] {
        let mut s = bare(b"008000000");
        s.set_param(ROTARY, 1.0);
        s.set_param(SPEED, SPEED_FAST);
        s.reset();
        let (l, _) = render(&mut s, 48_000, &[(0, on(note))], |_| 128);
        let moving = swing(&l);
        assert!(moving > 2.0, "note {note}: {moving:.2} dB");
        let mut s = bare(b"008000000");
        let (l, _) = render(&mut s, 48_000, &[(0, on(note))], |_| 128);
        let still = swing(&l);
        assert!(still < 0.1, "note {note} without the rotary: {still:.2} dB");
    }
}

#[test]
fn the_horn_shifts_the_pitch_both_ways() {
    // Doppler: a 2 kHz wheel at Fast moves up to 2.5 % either way (the
    // horn's mouth at 8.5 m/s), so its energy spreads into sidebands at the
    // horn's rate (7.06 turns a second) either side; without the rotary it
    // stays a single line.
    let spread = |rotary: f32| {
        let mut s = bare(b"008000000");
        s.set_param(ROTARY, rotary);
        s.set_param(SPEED, SPEED_FAST);
        s.set_param(SPREAD, 0.0);
        s.reset();
        let note = 95u8;
        let f = gear_hz(wheel_for(i32::from(note), 2)) as f32;
        let (l, _) = render(&mut s, 96_000, &[(0, on(note))], |_| 128);
        let x = &l[48_000..];
        let horn = 423.36 / 60.0;
        let line = tone(x, f) / (rms(x) * std::f32::consts::SQRT_2);
        (
            line,
            tone(x, f - horn),
            tone(x, f + horn),
            tone(x, f * 1.04),
        )
    };
    let (still, _, _, _) = spread(0.0);
    assert!(still > 0.98, "{still}");
    let (line, lo, hi, far) = spread(1.0);
    assert!(line < 0.7, "the centre keeps {line}");
    assert!(
        lo > 0.0 && (db(lo) - db(hi)).abs() < 6.0,
        "sidebands {lo} {hi}"
    );
    // Nothing beyond the Doppler shift's reach.
    assert!(far < lo * 0.1, "{far} beyond {lo}");
}

#[test]
fn the_rotors_change_speed_with_their_own_inertia() {
    let mut s = Drawbar::new(RATE);
    let slow = s.rotor_speeds();
    assert!((slow.0 - 40.32 / 60.0).abs() < 1e-3 && (slow.1 - 36.0 / 60.0).abs() < 1e-3);
    render(&mut s, 48_000, &[(0, set(SPEED, SPEED_FAST))], |_| 128);
    let (horn, drum) = s.rotor_speeds();
    // One second in: the horn is there, the drum about a fifth of the way.
    assert!(horn > 0.99 * 423.36 / 60.0, "horn {horn}");
    let part = (drum - 0.6) / (357.3 / 60.0 - 0.6);
    assert!(
        (part - (1.0 - (-1.0f32 / 4.127).exp())).abs() < 0.02,
        "drum {part}"
    );
    render(&mut s, 48_000 * 10, &[(0, set(SPEED, STOP))], |_| 128);
    assert_eq!(s.rotor_speeds(), (0.0, 0.0));
}

#[test]
fn the_scanner_bends_the_pitch_by_its_depth() {
    // A 1.5 kHz wheel through V1 and V3: the delay swept across 0.40 or
    // 0.85 ms at 6.9 Hz moves the pitch by about ±(2 × delay × rate).
    let depth = |mode: f32| {
        let mut s = bare(b"008000000");
        s.set_param(VIBRATO, mode);
        let note = 90u8;
        let f = gear_hz(wheel_for(i32::from(note), 2)) as f32;
        let (l, _) = render(&mut s, 48_000, &[(0, on(note))], |_| 128);
        // The pitch in 5 ms windows, by zero crossings.
        let x = &l[4800..];
        let mut cross = Vec::new();
        for i in 1..x.len() {
            if x[i - 1] < 0.0 && x[i] >= 0.0 {
                cross.push(i as f32 - x[i] / (x[i] - x[i - 1]));
            }
        }
        let mut lo = f32::MAX;
        let mut hi = 0.0f32;
        for w in cross.windows(9) {
            let p = RATE as f32 * 8.0 / (w[8] - w[0]);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        (1200.0 * (hi / f).log2(), 1200.0 * (lo / f).log2())
    };
    let (up0, down0) = depth(0.0);
    assert!(up0 < 0.5 && down0 > -0.5, "{up0} {down0}");
    let (up1, down1) = depth(V1);
    let (up3, down3) = depth(V3);
    // ±(2 × 0.36 ms × 6.9 Hz) = ±8.6 cents for V1, ±(2 × 0.80 × 6.9) = ±19 for V3.
    assert!(
        up1 > 4.0 && up1 < 14.0 && down1 < -4.0 && down1 > -14.0,
        "V1 {up1} {down1}"
    );
    assert!(
        up3 > 12.0 && up3 < 30.0 && down3 < -12.0 && down3 > -30.0,
        "V3 {up3} {down3}"
    );
}

#[test]
fn chorus_mixes_the_dry_sound_back() {
    // Chorus is vibrato plus the dry sound: C3 = 0.6 × (dry + V3).
    let play = |mode: f32| {
        let mut s = bare(b"008000000");
        s.set_param(VIBRATO, mode);
        render(&mut s, 24_000, &[(0, on(72)), (0, on(79))], |_| 128).0
    };
    let (dry, vib, cho) = (play(0.0), play(V3), play(C3));
    let diff: Vec<f32> = (0..dry.len())
        .map(|i| cho[i] - 0.6 * (dry[i] + vib[i]))
        .collect();
    assert!(
        rms(&diff) < rms(&cho) * 0.02,
        "{} against {}",
        rms(&diff),
        rms(&cho)
    );
    // And vibrato alone is not the dry sound.
    let wet: Vec<f32> = (0..dry.len()).map(|i| vib[i] - dry[i]).collect();
    assert!(rms(&wet) > rms(&dry) * 0.3);
}

#[test]
fn a_preset_starts_at_its_own_level() {
    // Loading a preset while nothing sounds jumps to its level and tone;
    // while a key sounds, they glide.
    let mut a = Drawbar::new(RATE);
    a.set_param(LEVEL, -20.0);
    let (l, _) = render(&mut a, 2400, &[(0, on(60))], |_| 128);
    let mut b = Drawbar::new(RATE);
    let (m, _) = render(&mut b, 2400, &[(0, on(60))], |_| 128);
    for i in 0..2400 {
        assert!((l[i] - m[i] * 0.1).abs() < 1e-6, "frame {i}");
    }
}

#[test]
fn drive_adds_harmonics_and_0_is_clean() {
    let second = |drive: f32| {
        let mut s = bare(b"008000000");
        s.set_param(DRIVE, drive);
        let (l, _) = render(&mut s, 24_000, &[(0, on(57))], |_| 128);
        let f = gear_hz(33) as f32;
        db(tone(&l[4800..], 2.0 * f)) - db(tone(&l[4800..], f))
    };
    assert!(second(0.0) < -50.0, "{}", second(0.0));
    assert!(second(70.0) > -30.0, "{}", second(70.0));
}

#[test]
fn tone_tilts_the_spectrum() {
    let tilt = |t: f32| {
        let mut s = bare(b"008000008");
        s.set_param(TONE, t);
        // C5's 8' (523 Hz) against its 1' (4.2 kHz).
        let (l, _) = render(&mut s, 24_000, &[(0, on(72))], |_| 128);
        let x = &l[4800..];
        db(harmonic(x, 72, 36)) - db(harmonic(x, 72, 0))
    };
    let (dark, flat, bright) = (tilt(-100.0), tilt(0.0), tilt(100.0));
    assert!(flat.abs() < 0.5, "{flat}");
    assert!(
        bright > flat + 4.0 && dark < flat - 6.0,
        "{dark} {flat} {bright}"
    );
}

#[test]
fn chords_stay_under_full_scale_and_presets_sit_together() {
    let chord: Vec<(u64, NodeEvent)> = [48, 55, 60, 64, 67, 72]
        .iter()
        .map(|&n| (0, on(n)))
        .collect();
    let mut init = 0.0;
    for p in Drawbar::presets() {
        // A six-note chord peaks under -1 dBFS.
        let mut s = preset(p.name);
        let (l, r) = render(&mut s, 96_000, &chord, |_| 128);
        let pk = db(peak(&l).max(peak(&r)));
        assert!(pk < -1.0, "{}: chord peaks at {pk:.1} dBFS", p.name);
        // One held key sits within 5 dB of Init's (once any percussion has gone).
        let mut s = preset(p.name);
        let (l, r) = render(&mut s, 96_000, &[(0, on(60))], |_| 128);
        let level = db(rms(&l[48_000..]).max(rms(&r[48_000..])));
        if p.name == "Init" {
            init = level;
            let pk = db(peak(&l).max(peak(&r)));
            assert!(
                pk > -17.0 && pk < -13.0,
                "Init: one key peaks at {pk:.1} dBFS"
            );
        }
        assert!(
            (level - init).abs() < 5.0,
            "{}: {level:.1} dB against {init:.1}",
            p.name
        );
    }
}

#[test]
fn every_setting_stays_finite() {
    let mut seed = 3u32;
    let mut rnd = move || {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        seed >> 8
    };
    for round in 0..6 {
        let mut s = Drawbar::new(RATE);
        for p in PARAMS {
            let v = match round {
                0 => p.min,
                1 => p.max,
                _ => p.min + (p.max - p.min) * (rnd() % 1000) as f32 / 1000.0,
            };
            s.set_param(p.id, v);
        }
        let mut ev = Vec::new();
        for i in 0..40u64 {
            let note = (rnd() % 128) as u8;
            ev.push((i * 600, on(note)));
            if rnd() % 3 == 0 {
                ev.push((i * 600 + 300, off(note)));
            }
        }
        let (l, r) = render(&mut s, 48_000, &ev, |_| 128);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()), "round {round}");
        // Forty keys at full drawbars, +6 dB, panned hard: loud, but bounded.
        assert!(peak(&l).max(peak(&r)) < 20.0, "round {round}: {}", peak(&l));
    }
}

#[test]
fn the_meter_counts_keys_held() {
    let mut s = Drawbar::new(RATE);
    render(
        &mut s,
        4800,
        &[(0, on(60)), (0, on(64)), (100, off(60))],
        |_| 128,
    );
    assert_eq!(s.param(METER_KEYS), 1.0);
    s.set_param(METER_KEYS, 50.0);
    assert_eq!(s.param(METER_KEYS), 1.0);
}

#[test]
fn other_rates_play_in_tune() {
    for rate in [44_100u32, 96_000] {
        let mut s = Drawbar::new(rate);
        for (i, &(id, _, _)) in DRAWBARS.iter().enumerate() {
            s.set_param(id, if i == 2 { 8.0 } else { 0.0 });
        }
        s.set_param(ROTARY, 0.0);
        s.reset();
        let n = rate as usize / 2;
        let mut l = vec![0.0f32; n];
        let mut r = vec![0.0f32; n];
        let mut at = 0;
        while at < n {
            let len = 128.min(n - at);
            let ev = if at == 0 {
                vec![Timed::new(0, on(69))]
            } else {
                vec![]
            };
            s.process(
                &Ctx::new(rate, at as u64),
                &ev,
                &mut l[at..at + len],
                &mut r[at..at + len],
            );
            at += len;
        }
        // Count cycles over the last 0.4 s.
        let x = &l[n / 5..];
        let mut cross = Vec::new();
        for i in 1..x.len() {
            if x[i - 1] < 0.0 && x[i] >= 0.0 {
                cross.push(i as f32 - x[i] / (x[i] - x[i - 1]));
            }
        }
        let cycles = (cross.len() - 1) as f32;
        let got = rate as f32 * cycles / (cross[cross.len() - 1] - cross[0]);
        assert!((got - 440.0).abs() < 0.1, "{rate}: {got}");
    }
}

#[test]
#[ignore]
fn print_preset_levels() {
    for chord in [&[60u8][..], &[48, 55, 60, 64, 67, 72][..]] {
        let ev: Vec<(u64, NodeEvent)> = chord.iter().map(|&n| (0, on(n))).collect();
        for p in Drawbar::presets() {
            let mut s = preset(p.name);
            let (l, r) = render(&mut s, 96_000, &ev, |_| 128);
            println!(
                "{:>2} keys {:<12} peak {:6.1} dB  rms {:6.1} dB",
                chord.len(),
                p.name,
                db(peak(&l).max(peak(&r))),
                db(rms(&l[48_000..]).max(rms(&r[48_000..])))
            );
        }
    }
}
