//! Behaviour tests for the Choir through the mpfree-node interface.

use mpfree_node::{check_params, Ctl, Ctx, Expr, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::choir::{bank_gain, FORMANTS, MAX_SINGERS};
use mpfree_synth::choir_params::*;
use mpfree_synth::Choir;

const RATE: u32 = 48_000;

fn render_at(
    c: &mut Choir,
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
        c.process(
            &Ctx::new(rate, at as u64),
            &buf,
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    (l, r)
}

/// Renders on from frame `from` (absolute), in 128-frame blocks.
fn more(c: &mut Choir, from: usize, frames: usize, events: &[(u64, NodeEvent)]) -> Vec<f32> {
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let mut at = 0;
    let mut buf = Vec::new();
    while at < frames {
        let len = 128.min(frames - at);
        buf.clear();
        for &(f, ev) in events {
            if f >= at as u64 && f < (at + len) as u64 {
                buf.push(Timed::new((f - at as u64) as u32, ev));
            }
        }
        c.process(
            &Ctx::new(RATE, (from + at) as u64),
            &buf,
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    l
}

fn render(c: &mut Choir, frames: usize, events: &[(u64, NodeEvent)]) -> (Vec<f32>, Vec<f32>) {
    render_at(c, RATE, frames, events, |_| 128)
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

/// The level as a loudness meter hears it, roughly: 4 dB more above
/// about 1.5 kHz (the weighting the vowel trims level by), dB.
fn loud(x: &[f32]) -> f32 {
    let c = 1.0 - (-std::f32::consts::TAU * 1_500.0 / RATE as f32).exp();
    let mut lp = 0.0f32;
    let y: Vec<f32> = x
        .iter()
        .map(|&v| {
            lp += c * (v - lp);
            lp + 10f32.powf(0.2) * (v - lp)
        })
        .collect();
    db(rms(&y))
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

/// The spectral centroid of `x` (power-weighted, 20 Hz to 20 kHz, the
/// lab's L5 measure) over its first power-of-two frames, Hz.
fn centroid(x: &[f32], rate: u32) -> f32 {
    let n = 1usize << (usize::BITS - 1 - x.len().leading_zeros());
    let fft = mpfree_analysis::fft::Fft::new(n);
    let w = mpfree_analysis::fft::hann(n);
    let mut re: Vec<f32> = x[..n].iter().zip(&w).map(|(a, b)| a * b).collect();
    let mut im = vec![0.0f32; n];
    fft.forward(&mut re, &mut im);
    let bin = rate as f32 / n as f32;
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for k in 1..n / 2 {
        let f = k as f32 * bin;
        if (20.0..=20_000.0).contains(&f) {
            let p = f64::from(re[k] * re[k] + im[k] * im[k]);
            num += f64::from(f) * p;
            den += p;
        }
    }
    (num / den.max(1e-30)) as f32
}

/// One singer on the note, dry, still: no detune, vibrato, drift or
/// breath, a quick swell, full brightness flat.
fn plain() -> Choir {
    plain_at(RATE)
}

fn plain_at(rate: u32) -> Choir {
    let mut c = Choir::new(rate);
    for (id, v) in [
        (SINGERS, 1.0),
        (DETUNE, 0.0),
        (VIBRATO, 0.0),
        (DRIFT, 0.0),
        (BREATH, 0.0),
        (ENSEMBLE, 0.0),
        (ATTACK, 5.0),
        (VEL_TONE, 0.0),
    ] {
        c.set_param(id, v);
    }
    c
}

fn preset(name: &str) -> Choir {
    let mut c = Choir::new(RATE);
    let p = Choir::presets()
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no preset {name}"));
    c.load_preset(p);
    c
}

/// The first busy slot.
fn slot(c: &Choir) -> usize {
    c.busy_slots().next().expect("a voice")
}

#[test]
fn params_and_presets_are_valid() {
    let c = Choir::new(RATE);
    check_params(c.params()).unwrap();
    assert_eq!(c.info().id, "mpfree.choir");
    assert_eq!(c.param(VOICES), 8.0, "8 voices by default");
    let names: Vec<_> = Choir::presets().iter().map(|p| p.name).collect();
    assert_eq!(names[0], "Init");
    for (i, a) in names.iter().enumerate() {
        assert!(!names[..i].contains(a), "{a} twice");
    }
    for p in Choir::presets() {
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
    let mut c = Choir::new(RATE);
    let (l, r) = render(&mut c, 4096, &[]);
    assert!(l.iter().chain(&r).all(|&x| x == 0.0));
    assert_eq!(c.sounding_voices(), 0);
    let mut c = preset("Init");
    let (a, _) = render(&mut c, 9600, &[(0, on(60, 100))]);
    assert!(peak(&a) > 0.01);
    let mut c = preset("Init");
    let mut l = vec![0.25f32; 9600];
    let mut r = vec![0.25f32; 9600];
    for at in (0..9600).step_by(128) {
        let ev = if at == 0 {
            vec![Timed::new(0, on(60, 100))]
        } else {
            vec![]
        };
        c.process(
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
        (ms(100.0), set(VOWEL, 80.0)),
        (ms(120.0), set(MORPH, 60.0)),
        (ms(150.0), NodeEvent::Bend { value: 0.5 }),
        (
            ms(170.0),
            NodeEvent::Pressure {
                note: 55,
                value: 90,
            },
        ),
        (ms(180.0), pedal(Ctl::Sustain, false)),
        (ms(200.0), set(SINGERS, 6.0)),
        (ms(210.0), on(64, 127)),
        (
            ms(230.0),
            NodeEvent::Control {
                ctl: Ctl::ModWheel,
                value: 0.8,
            },
        ),
        (
            ms(240.0),
            NodeEvent::Expression {
                note: 64,
                kind: Expr::Slide,
                value: 1.0,
            },
        ),
        (ms(260.0), off(55)),
        (ms(300.0), off(64)),
    ];
    let mut a = preset("Init");
    let mut b = preset("Init");
    let mut c = preset("Init");
    a.set_param(PRESS_VOWEL, 50.0);
    b.set_param(PRESS_VOWEL, 50.0);
    c.set_param(PRESS_VOWEL, 50.0);
    let x = render_at(&mut a, RATE, 24_000, &ev, |_| 128);
    let y = render_at(&mut b, RATE, 24_000, &ev, |n| [1, 7, 64, 3, 128, 33][n % 6]);
    let z = render_at(&mut c, RATE, 24_000, &ev, |_| 1);
    assert!(peak(&x.0) > 0.01);
    assert_eq!(x, y);
    assert_eq!(x, z);
}

#[test]
fn plays_in_tune_from_low_to_high() {
    for note in [36u8, 48, 60, 72, 84] {
        let mut c = plain();
        let (l, _) = render(&mut c, 24_000, &[(0, on(note, 100))]);
        let f = hz(f32::from(note));
        let got = strongest_near(&l[4800..], f, 40.0, RATE);
        let cents = 1200.0 * (got / f).log2();
        assert!(cents.abs() < 1.0, "note {note}: {cents} cents");
    }
}

#[test]
fn in_tune_and_at_the_same_level_at_other_rates() {
    let level = |rate: u32| {
        let mut c = plain_at(rate);
        let (l, _) = render_at(&mut c, rate, rate as usize, &[(0, on(57, 100))], |_| 128);
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
    // Breath adds the same at every rate.
    let breath = |rate: u32| {
        let take = |b: f32| {
            let mut c = plain_at(rate);
            c.set_param(BREATH, b);
            let (l, _) = render_at(&mut c, rate, rate as usize, &[(0, on(57, 100))], |_| 128);
            db(rms(&l[rate as usize / 2..]))
        };
        take(100.0) - take(0.0)
    };
    let base = breath(48_000);
    assert!(base > 2.0, "breath adds {base} dB");
    for rate in [44_100u32, 96_000] {
        let b = breath(rate);
        assert!(
            (b - base).abs() < 0.5,
            "{rate} Hz: breath adds {b} dB, {base} at 48 kHz"
        );
    }
}

/// O1: each formant's filter peaks where the vowel table says, within 5 %,
/// and is as wide as it says, within 20 %, for every section and vowel,
/// at 44.1 and 48 kHz.
#[test]
fn every_formant_sits_where_the_table_says() {
    for rate in [44_100u32, 48_000] {
        for section in 0..5 {
            for vowel in 0..5 {
                let mut c = plain_at(rate);
                c.set_param(SECTION, section as f32);
                c.set_param(VOWEL, 25.0 * vowel as f32);
                // C2: the note is far under every formant and its harmonics
                // close together, so the table is sung as it is.
                render_at(&mut c, rate, 512, &[(0, on(36, 100))], |_| 128);
                let s = slot(&c);
                let asked = c.voice_formants(s).unwrap();
                for k in 0..FORMANTS {
                    let (fc, _, bw) = TABLE[section][vowel][k];
                    assert!(
                        (asked[k].0 / fc - 1.0).abs() < 0.01
                            && (asked[k].2 / bw - 1.0).abs() < 0.01,
                        "section {section} vowel {vowel} formant {k}: asked {:?} for {:?}",
                        asked[k],
                        TABLE[section][vowel][k]
                    );
                    // The filter's own peak and its 3 dB points.
                    let g = |f: f32| c.formant_gain(s, k, f).unwrap();
                    let (mut top, mut at) = (0.0f32, fc);
                    let mut f = fc * 0.8;
                    while f < fc * 1.2 {
                        if g(f) > top {
                            top = g(f);
                            at = f;
                        }
                        f *= 1.0005;
                    }
                    let half = top * std::f32::consts::FRAC_1_SQRT_2;
                    let mut lo = at;
                    while g(lo) > half {
                        lo /= 1.0002;
                    }
                    let mut hi = at;
                    while g(hi) > half {
                        hi *= 1.0002;
                    }
                    let width = hi - lo;
                    let want_db = TABLE[section][vowel][k].1;
                    assert!(
                        (at / fc - 1.0).abs() < 0.05,
                        "{rate} Hz, section {section} vowel {vowel} formant {k}: peak {at} Hz for {fc}"
                    );
                    assert!(
                        (width / bw - 1.0).abs() < 0.2,
                        "{rate} Hz, section {section} vowel {vowel} formant {k}: {width} Hz wide for {bw}"
                    );
                    assert!(
                        (db(top) - want_db).abs() < 0.5,
                        "{rate} Hz, section {section} vowel {vowel} formant {k}: {} dB for {want_db}",
                        db(top)
                    );
                }
            }
        }
    }
}

/// The sound follows the formants: a low note's harmonics, heard through
/// the whole voice, have the levels the five formants together give them
/// (within 1.5 dB wherever that is within 30 dB of its loudest).
#[test]
fn the_voice_sounds_its_formants() {
    for (section, vowel) in [(BASS, 0.0), (ALTO, 50.0), (SOPRANO, 100.0), (TENOR, 75.0)] {
        let mut c = plain();
        c.set_param(SECTION, section);
        c.set_param(VOWEL, vowel);
        let (l, _) = render(&mut c, 48_000, &[(0, on(36, 127))]);
        let f = c.voice_formants(slot(&c)).unwrap();
        let x = &l[12_000..];
        let f0 = hz(36.0);
        let mut pairs = Vec::new();
        let mut k = 1.0;
        while f0 * k < 5_500.0 {
            let h = f0 * k;
            pairs.push((h, db(tone_at(x, h, RATE)), db(bank_gain(&f, h))));
            k += 1.0;
        }
        let loudest = pairs.iter().fold(f32::MIN, |m, p| m.max(p.2));
        let mut diffs: Vec<f32> = pairs
            .iter()
            .filter(|p| p.2 > loudest - 30.0)
            .map(|p| p.1 - p.2)
            .collect();
        diffs.sort_by(f32::total_cmp);
        let mid = diffs[diffs.len() / 2];
        for p in pairs.iter().filter(|p| p.2 > loudest - 30.0) {
            assert!(
                (p.1 - p.2 - mid).abs() < 1.5,
                "section {section} vowel {vowel}: {:.0} Hz at {:.1} dB, the formants say {:.1}",
                p.0,
                p.1 - mid,
                p.2
            );
        }
    }
}

/// High notes: the first formant rises to the note, as sopranos tune it
/// (Joliveau, Smith and Wolfe 2004), and the second keeps above it.
#[test]
fn a_high_note_raises_the_first_formant_to_it() {
    for note in [72u8, 79, 84, 88] {
        let mut c = plain();
        c.set_param(SECTION, SOPRANO);
        c.set_param(VOWEL, 0.0); // "Ooh": its first formant is lowest
        render(&mut c, 512, &[(0, on(note, 100))]);
        let f = c.voice_formants(slot(&c)).unwrap();
        let f0 = hz(f32::from(note));
        assert!(
            f[0].0 >= f0 * 0.999,
            "note {note}: F1 {} under {f0}",
            f[0].0
        );
        assert!(
            f[1].0 > f[0].0 * 1.1,
            "note {note}: F2 {} at F1 {}",
            f[1].0,
            f[0].0
        );
        assert!(f[0].2 >= f0 * 0.49, "note {note}: F1 {} Hz wide", f[0].2);
    }
}

/// The vowels are about as loud as each other, low and high, in every
/// section, and the keys about as loud as each other.
#[test]
fn every_vowel_and_key_about_as_loud() {
    for section in [BASS, TENOR, COUNTERTENOR, ALTO, SOPRANO, MIXED] {
        for note in [48u8, 60, 72] {
            let levels: Vec<f32> = (0..5)
                .map(|v| {
                    let mut c = plain();
                    c.set_param(SECTION, section);
                    c.set_param(VOWEL, 25.0 * v as f32);
                    let (l, _) = render(&mut c, 24_000, &[(0, on(note, 100))]);
                    loud(&l[9_600..])
                })
                .collect();
            let lo = levels.iter().cloned().fold(f32::MAX, f32::min);
            let hi = levels.iter().cloned().fold(f32::MIN, f32::max);
            assert!(
                hi - lo < 3.0,
                "section {section} note {note}: vowels {levels:?}"
            );
        }
    }
    let levels: Vec<f32> = (36u8..=84)
        .step_by(4)
        .map(|n| {
            let mut c = plain();
            c.set_param(SECTION, MIXED);
            let (l, _) = render(&mut c, 24_000, &[(0, on(n, 100))]);
            loud(&l[9_600..])
        })
        .collect();
    let lo = levels.iter().cloned().fold(f32::MAX, f32::min);
    let hi = levels.iter().cloned().fold(f32::MIN, f32::max);
    assert!(hi - lo < 3.0, "C2 to C6: {levels:?}");
}

/// The singers spread by Detune (standard deviation about the setting),
/// and with Detune, Vibrato and Drift at 0 every one sings the note.
#[test]
fn the_singers_spread_by_detune() {
    for n in [2.0f32, 3.0, 5.0, 8.0] {
        let mut c = plain();
        c.set_param(SINGERS, n);
        c.set_param(DETUNE, 20.0);
        render(&mut c, 4_800, &[(0, on(60, 100))]);
        let s = slot(&c);
        let cents: Vec<f32> = (0..n as usize)
            .map(|i| c.singer_cents(s, i).unwrap())
            .collect();
        let mean = cents.iter().sum::<f32>() / n;
        let sd = (cents.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / n).sqrt();
        assert!((10.0..32.0).contains(&sd), "{n} singers: {cents:?}");
        c.set_param(DETUNE, 0.0);
        render(&mut c, 256, &[]);
        for i in 0..n as usize {
            let x = c.singer_cents(s, i).unwrap();
            assert!(x.abs() < 0.01, "{n} singers: singer {i} {x} cents");
        }
    }
}

/// O8: each singer's vibrato and drift its own: the singers' pitch moves
/// are far from moving together.
#[test]
fn each_singer_wavers_on_its_own() {
    let mut c = Choir::new(RATE);
    c.set_param(SINGERS, MAX_SINGERS as f32);
    c.set_param(DETUNE, 0.0);
    c.set_param(VIBRATO, 30.0);
    c.set_param(VIB_DELAY, 0.0);
    render(&mut c, 9_600, &[(0, on(60, 100))]);
    let s = slot(&c);
    let mut tracks = vec![Vec::new(); MAX_SINGERS];
    let mut at = 9_600;
    for _ in 0..600 {
        more(&mut c, at, 128, &[]);
        at += 128;
        for (i, t) in tracks.iter_mut().enumerate() {
            t.push(c.singer_cents(s, i).unwrap());
        }
    }
    let corr = |a: &[f32], b: &[f32]| {
        let n = a.len() as f32;
        let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
        let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
        for (x, y) in a.iter().zip(b) {
            ab += (x - ma) * (y - mb);
            aa += (x - ma) * (x - ma);
            bb += (y - mb) * (y - mb);
        }
        ab / (aa * bb).sqrt()
    };
    let mut sum = 0.0;
    let mut pairs = 0;
    for i in 0..MAX_SINGERS {
        for j in i + 1..MAX_SINGERS {
            let r = corr(&tracks[i], &tracks[j]);
            assert!(r < 0.95, "singers {i} and {j} move together: {r}");
            sum += r;
            pairs += 1;
        }
    }
    let mean = sum / pairs as f32;
    assert!(mean.abs() < 0.3, "mean correlation {mean}");
}

#[test]
fn voices_limits_the_notes_and_steals_the_oldest() {
    let mut c = preset("Init");
    c.set_param(VOICES, 4.0);
    let ev: Vec<_> = (0..6)
        .map(|i| (ms(10.0) * i, on(60 + i as u8, 100)))
        .collect();
    render(&mut c, ms(200.0) as usize, &ev);
    assert_eq!(c.stolen(), 2);
    assert_eq!(c.active_voices(), 4);
    // Eight by default, sixteen at most.
    let mut c = preset("Init");
    let ev: Vec<_> = (0..20u8).map(|i| (0, on(40 + i, 100))).collect();
    render(&mut c, ms(100.0) as usize, &ev);
    assert_eq!(c.active_voices(), 8);
    c.set_param(VOICES, 16.0);
    let ev: Vec<_> = (0..20u8).map(|i| (0, on(70 + i, 100))).collect();
    more(&mut c, ms(100.0) as usize, ms(100.0) as usize, &ev);
    assert_eq!(c.active_voices(), 16);
}

#[test]
fn the_same_key_again_rings_on_in_its_release() {
    let mut c = preset("Init");
    render(&mut c, 4_800, &[(0, on(60, 100)), (2_400, on(60, 100))]);
    assert_eq!(c.active_voices(), 2);
    assert_eq!(c.stolen(), 0);
}

#[test]
fn the_sustain_pedal_holds_notes_until_it_lifts() {
    let mut c = plain();
    c.set_param(RELEASE, 50.0);
    let ev = [
        (0, pedal(Ctl::Sustain, true)),
        (0, on(60, 100)),
        (ms(50.0), off(60)),
    ];
    let (l, _) = render(&mut c, ms(600.0) as usize, &ev);
    assert!(rms(&l[l.len() - 2_400..]) > 1e-3, "held by the pedal");
    let l = more(
        &mut c,
        ms(600.0) as usize,
        ms(600.0) as usize,
        &[(0, pedal(Ctl::Sustain, false))],
    );
    assert!(peak(&l[l.len() - 2_400..]) < 1e-4);
}

#[test]
fn the_sostenuto_holds_only_the_keys_down_when_it_went_down() {
    let mut c = plain();
    c.set_param(RELEASE, 50.0);
    let ev = [
        (0, on(48, 100)),
        (ms(20.0), pedal(Ctl::Sostenuto, true)),
        (ms(40.0), on(60, 100)),
        (ms(60.0), off(48)),
        (ms(60.0), off(60)),
    ];
    render(&mut c, ms(600.0) as usize, &ev);
    assert_eq!(c.active_voices(), 1);
    more(
        &mut c,
        ms(600.0) as usize,
        ms(600.0) as usize,
        &[(0, pedal(Ctl::Sostenuto, false))],
    );
    assert_eq!(c.active_voices(), 0);
}

#[test]
fn mono_plays_one_note_and_goes_back_to_a_held_key() {
    let mut c = plain();
    c.set_param(MODE, 1.0);
    let ev = [
        (0, on(60, 100)),
        (ms(100.0), on(67, 100)),
        (ms(300.0), off(67)),
    ];
    let (l, _) = render(&mut c, ms(500.0) as usize, &ev);
    assert_eq!(c.active_voices(), 1);
    let up = strongest_near(
        &l[ms(150.0) as usize..ms(300.0) as usize],
        hz(67.0),
        40.0,
        RATE,
    );
    let back = strongest_near(&l[ms(350.0) as usize..], hz(60.0), 40.0, RATE);
    assert!((1200.0 * (up / hz(67.0)).log2()).abs() < 2.0, "{up} Hz");
    assert!((1200.0 * (back / hz(60.0)).log2()).abs() < 2.0, "{back} Hz");
}

#[test]
fn legato_moves_without_starting_the_swell_again() {
    // Mono starts the swell again on each new key; Legato carries on.
    let dip = |mode: f32| {
        let mut c = plain();
        c.set_param(MODE, mode);
        c.set_param(ATTACK, 300.0);
        let ev = [(0, on(60, 100)), (ms(600.0), on(62, 100))];
        let (l, _) = render(&mut c, ms(700.0) as usize, &ev);
        let before = rms(&l[ms(500.0) as usize..ms(600.0) as usize]);
        let after = rms(&l[ms(610.0) as usize..ms(640.0) as usize]);
        db(after) - db(before)
    };
    assert!(dip(LEGATO).abs() < 1.5, "legato: {} dB", dip(LEGATO));
    // Mono's retrigger starts from where the swell is, so there is no
    // drop either; but its envelope restarts its attack from there.
    assert!(dip(1.0) < 1.5);
}

#[test]
fn glide_slides_between_notes_in_mono() {
    let mut c = plain();
    c.set_param(MODE, LEGATO);
    c.set_param(GLIDE, 200.0);
    let ev = [(0, on(60, 100)), (ms(300.0), on(72, 100))];
    let (l, _) = render(&mut c, ms(900.0) as usize, &ev);
    // Halfway through the glide the pitch is between the notes; after it,
    // on the new one.
    let mid = strongest_near(
        &l[ms(330.0) as usize..ms(370.0) as usize],
        hz(66.0),
        600.0,
        RATE,
    );
    assert!(mid > hz(61.0) && mid < hz(71.0), "{mid} Hz");
    let end = strongest_near(&l[ms(700.0) as usize..], hz(72.0), 40.0, RATE);
    assert!((1200.0 * (end / hz(72.0)).log2()).abs() < 2.0, "{end} Hz");
}

#[test]
fn a_harder_key_is_louder_and_brighter() {
    // The level from velocity 20 to 127 is what Velocity says (V1: its
    // darker tone takes nothing off), and the tone at least 0.1 octave
    // brighter (V3, L5: the first 340 ms, as the lab reads it).
    let one = |velocity: u8| {
        let mut c = preset("Init");
        let (l, _) = render(&mut c, 48_000, &[(0, on(60, velocity))]);
        (db(rms(&l[24_000..])), centroid(&l[..16_384], RATE))
    };
    let (soft, soft_c) = one(20);
    let (hard, hard_c) = one(127);
    let range = 20.0f32;
    let r = 10f32.powf(range / 20.0);
    let b = 127.0 / (126.0 * r.sqrt()) - 1.0 / 126.0;
    let a = (1.0 - b) / 127.0 * 20.0 + b;
    let want = -db(a * a);
    assert!(
        (hard - soft - want).abs() < 1.0,
        "{soft} to {hard} dB, Velocity gives {want}"
    );
    assert!((hard_c / soft_c).log2() >= 0.1, "{soft_c} to {hard_c} Hz");
}

#[test]
fn the_vowel_changes_the_colour() {
    // "Aah" sits higher than "Ooh" (its first two formants are); "Ee" has
    // its second formant between 1.4 and 2.2 kHz, where "Ooh" has nothing.
    let take = |vowel: f32| {
        let mut c = plain();
        c.set_param(VOWEL, vowel);
        let (l, _) = render(&mut c, 48_000, &[(0, on(57, 100))]);
        let x = &l[16_384..16_384 + 16_384];
        let fft = mpfree_analysis::fft::Fft::new(x.len());
        let w = mpfree_analysis::fft::hann(x.len());
        let mut re: Vec<f32> = x.iter().zip(&w).map(|(a, b)| a * b).collect();
        let mut im = vec![0.0f32; x.len()];
        fft.forward(&mut re, &mut im);
        let bin = RATE as f32 / x.len() as f32;
        let (mut band, mut all) = (0.0f64, 0.0f64);
        for k in 1..x.len() / 2 {
            let p = f64::from(re[k] * re[k] + im[k] * im[k]);
            all += p;
            if (1_400.0..2_200.0).contains(&(k as f32 * bin)) {
                band += p;
            }
        }
        (centroid(x, RATE), 10.0 * (band / all).log10() as f32)
    };
    let ((u, u_band), (a, _), (_, i_band)) = (take(0.0), take(50.0), take(100.0));
    assert!(a > u * 1.3, "Ooh {u} Hz, Aah {a} Hz");
    assert!(
        i_band > u_band + 10.0,
        "1.4 to 2.2 kHz: Ooh {u_band} dB, Ee {i_band} dB"
    );
}

#[test]
fn a_start_vowel_opens_into_the_vowel() {
    let mut c = plain();
    c.set_param(VOWEL, 50.0);
    c.set_param(START, -100.0);
    c.set_param(START_TIME, 400.0);
    render(&mut c, 256, &[(0, on(57, 100))]);
    let s = slot(&c);
    let early = c.voice_formants(s).unwrap();
    more(&mut c, 256, ms(600.0) as usize, &[]);
    let late = c.voice_formants(s).unwrap();
    // "Ooh" opening to "Aah": the first formant rises.
    assert!(
        late[0].0 > early[0].0 * 1.5,
        "{:?} to {:?}",
        early[0],
        late[0]
    );
}

#[test]
fn pressure_swells_and_moves_the_vowel() {
    let take = |press: u8, id: u16| {
        let mut c = plain();
        c.set_param(id, 100.0);
        let ev = [
            (0, on(57, 100)),
            (
                ms(10.0),
                NodeEvent::Pressure {
                    note: 57,
                    value: press,
                },
            ),
        ];
        let (l, _) = render(&mut c, 48_000, &ev);
        (
            db(rms(&l[24_000..])),
            c.voice_formants(slot(&c)).unwrap()[0].0,
        )
    };
    let (none, _) = take(0, PRESS_SWELL);
    let (full, _) = take(127, PRESS_SWELL);
    assert!(full - none > 8.0, "{none} to {full} dB");
    let (_, f_none) = take(0, PRESS_VOWEL);
    let (_, f_full) = take(127, PRESS_VOWEL);
    assert!(
        (f_full / f_none - 1.0).abs() > 0.1,
        "{f_none} to {f_full} Hz"
    );
}

#[test]
fn the_soft_pedal_is_quieter_and_darker() {
    let take = |soft: bool| {
        let mut c = preset("Init");
        let ev = [(0, pedal(Ctl::Soft, soft)), (0, on(57, 100))];
        let (l, _) = render(&mut c, 48_000, &ev);
        (db(rms(&l[24_000..])), centroid(&l[16_384..], RATE))
    };
    let (plain_db, plain_c) = take(false);
    let (soft_db, soft_c) = take(true);
    assert!(
        (plain_db - soft_db - 6.0).abs() < 1.0,
        "{plain_db} to {soft_db} dB"
    );
    // A formant voice's centroid sits on its first two formants, so a
    // darker tilt moves it only a little.
    assert!(soft_c < plain_c * 0.98, "{plain_c} to {soft_c} Hz");
}

#[test]
fn a_foot_controller_sets_the_volume() {
    let take = |value: f32| {
        let mut c = preset("Init");
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
        let (l, _) = render(&mut c, 48_000, &ev);
        db(rms(&l[24_000..]))
    };
    let full = take(1.0);
    let half = take(0.5);
    assert!((full - half - 12.0).abs() < 1.0, "{full} to {half} dB");
}

#[test]
fn bend_and_slide_move_the_note() {
    let mut c = plain();
    let ev = [(0, on(57, 100)), (0, NodeEvent::Bend { value: 1.0 })];
    let (l, _) = render(&mut c, 24_000, &ev);
    let want = hz(59.0);
    let got = strongest_near(&l[4_800..], want, 40.0, RATE);
    assert!((1200.0 * (got / want).log2()).abs() < 1.0, "{got} Hz");
    let mut c = plain();
    let ev = [
        (0, on(57, 100)),
        (
            0,
            NodeEvent::Expression {
                note: 57,
                kind: Expr::Slide,
                value: -2.0,
            },
        ),
    ];
    let (l, _) = render(&mut c, 24_000, &ev);
    let want = hz(55.0);
    let got = strongest_near(&l[4_800..], want, 40.0, RATE);
    assert!((1200.0 * (got / want).log2()).abs() < 1.0, "{got} Hz");
}

#[test]
fn every_preset_sits_near_the_level_convention() {
    for p in Choir::presets() {
        let mut c = Choir::new(RATE);
        c.load_preset(p);
        let (l, r) = render(&mut c, 48_000, &[(0, on(60, 100))]);
        let pk = db(peak(&l).max(peak(&r)));
        assert!((-30.0..-6.0).contains(&pk), "{}: one key {pk} dBFS", p.name);
        let mut c = Choir::new(RATE);
        c.load_preset(p);
        let chord: Vec<_> = [48u8, 52, 55, 60, 64, 67, 72, 76]
            .iter()
            .map(|&n| (0u64, on(n, 127)))
            .collect();
        let (l, r) = render(&mut c, 96_000, &chord);
        let pk = db(peak(&l).max(peak(&r)));
        assert!(pk < -0.5, "{}: eight keys {pk} dBFS", p.name);
    }
}

#[test]
fn a_track_at_rest_stops_running_and_starts_cleanly() {
    let mut c = plain();
    c.set_param(ENSEMBLE, 50.0);
    render(&mut c, 4_800, &[(0, on(60, 100)), (2_400, off(60))]);
    let (l, r) = render(&mut c, RATE as usize * 4, &[]);
    assert_eq!(c.sounding_voices(), 0);
    let tail = RATE as usize;
    assert!(l[l.len() - tail..]
        .iter()
        .chain(&r[r.len() - tail..])
        .all(|&x| x == 0.0));
    // The next note sounds as a fresh one does (dry: the ensemble's sweep
    // is where it stopped, and a single steady singer hears where).
    c.set_param(ENSEMBLE, 0.0);
    let (a, _) = render(&mut c, 24_000, &[(0, on(60, 100))]);
    let mut fresh = plain();
    let (b, _) = render(&mut fresh, 24_000, &[(0, on(60, 100))]);
    let (la, lb) = (db(rms(&a[12_000..])), db(rms(&b[12_000..])));
    assert!((la - lb).abs() < 0.1, "{la} against {lb} dB");
}

#[test]
fn the_tail_ends_within_what_it_says() {
    let mut c = preset("Init");
    render(&mut c, 9_600, &[(0, on(60, 127)), (4_800, off(60))]);
    let tail = c.tail_frames() as usize;
    let (l, r) = render(&mut c, tail + 4_800, &[]);
    let end = &l[tail..];
    assert!(
        db(peak(end).max(peak(&r[tail..]))) < -80.0,
        "{} dBFS after the tail",
        db(peak(end))
    );
}

#[test]
fn finite_at_the_extremes() {
    let mut c = Choir::new(RATE);
    for (id, v) in [
        (SINGERS, 8.0),
        (DETUNE, 50.0),
        (BREATH, 100.0),
        (BRIGHTNESS, 100.0),
        (VIBRATO, 100.0),
        (DRIFT, 100.0),
        (MORPH, 100.0),
        (MORPH_RATE, 8.0),
        (ATTACK, 1.0),
        (DEPTH, 200.0),
        (VOICES, 16.0),
        (TRANSPOSE, 24.0),
        (OCTAVE, 2.0),
        (LEVEL, 6.0),
    ] {
        c.set_param(id, v);
    }
    let mut ev: Vec<_> = (0u8..128).map(|n| (0u64, on(n, 127))).collect();
    ev.push((ms(50.0), NodeEvent::Bend { value: 1.0 }));
    ev.push((ms(100.0), set(SECTION, MIXED)));
    ev.push((ms(120.0), set(BRIGHTNESS, 0.0)));
    ev.push((ms(150.0), NodeEvent::AllOff));
    let (l, r) = render(&mut c, 24_000, &ev);
    assert!(l.iter().chain(&r).all(|x| x.is_finite() && x.abs() < 8.0));
}
