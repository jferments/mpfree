//! Behaviour tests for the string machine (Strings) through the
//! mpfree-node interface.

use mpfree_node::{check_params, Ctl, Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::strings_params::*;
use mpfree_synth::Strings;

const RATE: u32 = 48_000;

fn render_at(
    s: &mut Strings,
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

fn render(s: &mut Strings, frames: usize, events: &[(u64, NodeEvent)]) -> (Vec<f32>, Vec<f32>) {
    render_at(s, RATE, frames, events, |_| 128)
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
fn pedal(ctl: Ctl, down: bool) -> NodeEvent {
    NodeEvent::Control {
        ctl,
        value: if down { 1.0 } else { 0.0 },
    }
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
        let win = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
        let a = w * i as f64;
        re += f64::from(v) * win * a.cos();
        im -= f64::from(v) * win * a.sin();
    }
    (2.0 * (re * re + im * im).sqrt() / (n as f64 * 0.5)) as f32
}

/// The strongest component within `cents` of `guess`, Hz.
fn strongest_near(x: &[f32], guess: f32, cents: f32, rate: u32) -> f32 {
    let (mut best, mut at) = (0.0f32, guess);
    let steps = 400;
    for i in 0..=steps {
        let c = -cents + 2.0 * cents * i as f32 / steps as f32;
        let f = guess * (c / 1200.0).exp2();
        let a = tone_at(x, f, rate);
        if a > best {
            best = a;
            at = f;
        }
    }
    at
}

/// The spectral centroid of `x`, Hz (Goertzel on a third-octave grid).
fn centroid(x: &[f32], rate: u32) -> f32 {
    let (mut num, mut den) = (0.0f32, 0.0f32);
    let mut f = 50.0f32;
    while f < 12_000.0 {
        let a = tone_at(x, f, rate);
        num += f * a * a;
        den += a * a;
        f *= 2f32.powf(1.0 / 6.0);
    }
    num / den.max(1e-30)
}

/// The Viola alone, dry: one plain sawtooth bank through one register.
fn plain() -> Strings {
    plain_at(RATE)
}

fn plain_at(rate: u32) -> Strings {
    let mut s = Strings::new(rate);
    for (id, v) in [
        (VIOLIN, 0.0),
        (VIOLA, 100.0),
        (ENSEMBLE, 0.0),
        (NOISE, 0.0),
        (CRESCENDO, 2.0),
    ] {
        s.set_param(id, v);
    }
    s
}

fn preset(name: &str) -> Strings {
    let mut s = Strings::new(RATE);
    let p = Strings::presets()
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no preset {name}"));
    s.load_preset(p);
    s
}

#[test]
fn params_and_presets_are_valid() {
    let s = Strings::new(RATE);
    check_params(s.params()).unwrap();
    assert_eq!(s.info().id, "mpfree.strings");
    let names: Vec<_> = Strings::presets().iter().map(|p| p.name).collect();
    assert_eq!(names[0], "Init");
    for (i, a) in names.iter().enumerate() {
        assert!(!names[..i].contains(a), "{a} twice");
    }
    for p in Strings::presets() {
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
fn silent_until_a_key_and_adds_into_the_buffers() {
    let mut s = Strings::new(RATE);
    let (l, r) = render(&mut s, 4096, &[]);
    assert!(l.iter().chain(&r).all(|&x| x == 0.0));
    assert_eq!(s.sounding_voices(), 0);
    // What is in the buffers stays there, with the strings added on top.
    let mut s = Strings::new(RATE);
    let mut l = vec![0.25f32; 256];
    let mut r = vec![0.25f32; 256];
    s.process(&Ctx::new(RATE, 0), &[], &mut l, &mut r);
    assert!(l.iter().chain(&r).all(|&x| x == 0.25));
    let mut s = preset("Init");
    let (a, _) = render(&mut s, 9600, &[(0, on(60, 100))]);
    let mut s = preset("Init");
    let mut l = vec![0.25f32; 9600];
    let mut r = vec![0.25f32; 9600];
    for at in (0..9600).step_by(128) {
        let ev = if at == 0 {
            vec![Timed::new(0, on(60, 100))]
        } else {
            vec![]
        };
        s.process(
            &Ctx::new(RATE, at as u64),
            &ev,
            &mut l[at..at + 128],
            &mut r[at..at + 128],
        );
    }
    for (x, y) in a.iter().zip(&l) {
        assert!((x + 0.25 - y).abs() < 1e-6);
    }
}

#[test]
fn same_output_for_any_block_size() {
    let ev = vec![
        (100, on(48, 100)),
        (300, on(55, 80)),
        (ms(60.0), pedal(Ctl::Sustain, true)),
        (ms(80.0), off(48)),
        (ms(120.0), set(BRIGHTNESS, 80.0)),
        (ms(150.0), NodeEvent::Bend { value: 0.5 }),
        (ms(180.0), pedal(Ctl::Sustain, false)),
        (ms(200.0), set(SWELL, 1.0)),
        (ms(210.0), on(64, 127)),
        (
            ms(230.0),
            NodeEvent::Control {
                ctl: Ctl::ModWheel,
                value: 0.8,
            },
        ),
        (ms(260.0), off(55)),
        (ms(300.0), off(64)),
    ];
    let mut a = preset("Classic Strings");
    let mut b = preset("Classic Strings");
    let mut c = preset("Classic Strings");
    let x = render_at(&mut a, RATE, 24_000, &ev, |_| 128);
    let y = render_at(&mut b, RATE, 24_000, &ev, |n| [1, 7, 64, 3, 128, 33][n % 6]);
    let z = render_at(&mut c, RATE, 24_000, &ev, |_| 1);
    assert!(peak(&x.0) > 0.01);
    assert_eq!(x, y);
    assert_eq!(x, z);
}

#[test]
fn plays_in_tune_from_low_to_high() {
    for note in [36u8, 48, 60, 72, 84, 96] {
        let mut s = plain();
        let (l, _) = render(&mut s, 24_000, &[(0, on(note, 100))]);
        let f = hz(f32::from(note));
        let got = strongest_near(&l[4800..], f, 40.0, RATE);
        let cents = 1200.0 * (got / f).log2();
        assert!(cents.abs() < 1.0, "note {note}: {cents} cents");
    }
}

#[test]
fn in_tune_and_at_the_same_level_at_other_rates() {
    let level = |rate: u32| {
        let mut s = plain_at(rate);
        let (l, _) = render_at(&mut s, rate, rate as usize, &[(0, on(57, 100))], |_| 128);
        let half = rate as usize / 2;
        let f = strongest_near(&l[half..], 220.0, 40.0, rate);
        (db(rms(&l[half..])), 1200.0 * (f / 220.0).log2())
    };
    let (base, _) = level(48_000);
    for rate in [44_100u32, 96_000] {
        let (lv, cents) = level(rate);
        assert!(cents.abs() < 1.0, "{rate} Hz: {cents} cents");
        assert!((lv - base).abs() < 0.5, "{rate} Hz: {lv} dB against {base}");
    }
}

#[test]
fn octaves_never_beat() {
    // C3 and C4 held: the C4's 8' is the C3's 4', one oscillator, so the
    // level holds steady (no slow beating as between free oscillators).
    let mut s = plain();
    s.set_param(VIOLIN, 100.0);
    let (l, _) = render(&mut s, 96_000, &[(0, on(48, 100)), (0, on(60, 100))]);
    let blocks: Vec<f32> = l[24_000..].chunks(4_800).map(rms).collect();
    let (lo, hi) = blocks
        .iter()
        .fold((f32::MAX, 0.0f32), |(a, b), &x| (a.min(x), b.max(x)));
    assert!(db(hi) - db(lo) < 0.2, "{} dB of beating", db(hi) - db(lo));
}

#[test]
fn every_key_can_sound_at_once() {
    let mut s = preset("Init");
    let ev: Vec<_> = (36u8..=84).map(|n| (0u64, on(n, 100))).collect();
    let (l, r) = render(&mut s, 24_000, &ev);
    assert_eq!(s.keys_down(), 49);
    assert_eq!(s.keys_sounding(), 49);
    let pk = db(peak(&l).max(peak(&r)));
    assert!(pk < 0.0 && pk > -24.0, "{pk} dBFS");
}

#[test]
fn every_preset_sits_near_the_level_convention() {
    for p in Strings::presets() {
        let mut s = Strings::new(RATE);
        s.load_preset(p);
        let (l, r) = render(&mut s, 48_000, &[(0, on(60, 100))]);
        let pk = db(peak(&l).max(peak(&r)));
        assert!((-30.0..-6.0).contains(&pk), "{}: one key {pk} dBFS", p.name);
        let mut s = Strings::new(RATE);
        s.load_preset(p);
        let chord: Vec<_> = [48u8, 52, 55, 60, 64, 67, 72, 76]
            .iter()
            .map(|&n| (0u64, on(n, 127)))
            .collect();
        let (l, r) = render(&mut s, 96_000, &chord);
        let pk = db(peak(&l).max(peak(&r)));
        assert!(pk < -0.5, "{}: eight keys {pk} dBFS", p.name);
    }
}

#[test]
fn a_harder_key_is_louder_and_brighter() {
    let one = |velocity: u8| {
        let mut s = preset("Init");
        let (l, _) = render(&mut s, 48_000, &[(0, on(57, velocity))]);
        (db(rms(&l[24_000..])), centroid(&l[24_000..], RATE))
    };
    let (soft, soft_c) = one(30);
    let (hard, hard_c) = one(127);
    assert!(hard - soft > 8.0, "{soft} to {hard} dB");
    assert!(hard_c > soft_c * 1.1, "{soft_c} to {hard_c} Hz");
}

#[test]
fn the_sostenuto_holds_only_the_keys_down_when_it_went_down() {
    let mut s = plain();
    s.set_param(SUSTAIN, 50.0);
    let ev = [
        (0, on(48, 100)),
        (ms(20.0), pedal(Ctl::Sostenuto, true)),
        (ms(40.0), on(60, 100)),
        (ms(60.0), off(48)),
        (ms(60.0), off(60)),
    ];
    render(&mut s, ms(400.0) as usize, &ev);
    // C3 held by the pedal; C4 played after it went down, let go.
    assert_eq!(s.keys_down(), 1);
    assert_eq!(s.keys_sounding(), 1);
    render(&mut s, 128, &[(0, pedal(Ctl::Sostenuto, false))]);
    let (l, _) = render(&mut s, ms(400.0) as usize, &[]);
    assert!(peak(&l[l.len() - 4_800..]) < 1e-4);
    assert_eq!(s.keys_sounding(), 0);
}

#[test]
fn the_soft_pedal_is_quieter_and_darker() {
    let take = |soft: bool| {
        let mut s = preset("Init");
        let ev = [(0, pedal(Ctl::Soft, soft)), (0, on(57, 100))];
        let (l, _) = render(&mut s, 48_000, &ev);
        (db(rms(&l[24_000..])), centroid(&l[24_000..], RATE))
    };
    let (plain_db, plain_c) = take(false);
    let (soft_db, soft_c) = take(true);
    assert!(
        (plain_db - soft_db - 6.0).abs() < 1.5,
        "{plain_db} to {soft_db} dB"
    );
    assert!(soft_c < plain_c * 0.95, "{plain_c} to {soft_c} Hz");
}

#[test]
fn a_foot_controller_sets_the_volume() {
    let take = |value: f32| {
        let mut s = preset("Init");
        let ev = [
            (
                0,
                NodeEvent::Control {
                    ctl: Ctl::Foot,
                    value,
                },
            ),
            (0, on(57, 100)),
        ];
        let (l, _) = render(&mut s, 48_000, &ev);
        db(rms(&l[24_000..]))
    };
    let full = take(1.0);
    let half = take(0.5);
    assert!((full - half - 12.0).abs() < 1.0, "{full} to {half} dB");
}

#[test]
fn bend_moves_the_whole_bank() {
    let mut s = plain();
    let ev = [(0, on(57, 100)), (0, NodeEvent::Bend { value: 1.0 })];
    let (l, _) = render(&mut s, 24_000, &ev);
    let want = hz(59.0);
    let got = strongest_near(&l[4_800..], want, 40.0, RATE);
    assert!((1200.0 * (got / want).log2()).abs() < 1.0, "{got} Hz");
}

#[test]
fn a_track_at_rest_stops_running_and_starts_cleanly() {
    let mut s = preset("Init");
    render(&mut s, 4_800, &[(0, on(60, 100)), (2_400, off(60))]);
    let (l, r) = render(&mut s, RATE as usize * 4, &[]);
    assert_eq!(s.sounding_voices(), 0);
    let tail = RATE as usize;
    assert!(l[l.len() - tail..]
        .iter()
        .chain(&r[r.len() - tail..])
        .all(|&x| x == 0.0));
    // The next key sounds as a fresh one does.
    let (a, _) = render(&mut s, 24_000, &[(0, on(60, 100))]);
    let mut fresh = preset("Init");
    let (b, _) = render(&mut fresh, 24_000, &[(0, on(60, 100))]);
    let (la, lb) = (db(rms(&a[12_000..])), db(rms(&b[12_000..])));
    assert!((la - lb).abs() < 0.5, "{la} against {lb} dB");
}

#[test]
fn finite_at_the_extremes() {
    let mut s = Strings::new(RATE);
    for (id, v) in [
        (VIOLIN, 100.0),
        (VIOLA, 100.0),
        (CELLO, 100.0),
        (CONTRABASS, 100.0),
        (HORN, 100.0),
        (TRUMPET, 100.0),
        (BRIGHTNESS, 100.0),
        (BODY, 100.0),
        (NOISE, 100.0),
        (CRESCENDO, 2.0),
        (DEPTH, 200.0),
        (TRANSPOSE, 24.0),
        (OCTAVE, 1.0),
        (LEVEL, 6.0),
    ] {
        s.set_param(id, v);
    }
    let mut ev: Vec<_> = (0u8..128).map(|n| (0u64, on(n, 127))).collect();
    ev.push((ms(50.0), NodeEvent::Bend { value: 1.0 }));
    ev.push((ms(100.0), set(BRIGHTNESS, 0.0)));
    ev.push((ms(150.0), NodeEvent::AllOff));
    let (l, r) = render(&mut s, 24_000, &ev);
    assert!(l.iter().chain(&r).all(|x| x.is_finite() && x.abs() < 8.0));
}
