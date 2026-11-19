//! The Wavetable synth's sounds as Poly sounds (Wavetable folded into
//! Poly, part B): every factory preset, converted, plays about as the
//! Wavetable synth played it: as loud, as bright and with the same shape
//! over time. Not to the bit: Poly's envelopes are the voice core's, its
//! filter output has the DC blocker under drive, and route 1 gives it the
//! mod wheel's vibrato (the wheel never moves here).

use mpfree_node::preset::Preset;
use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::params as pp;
use mpfree_synth::wavetable::params as wp;
use mpfree_synth::wavetable::to_poly::{self, node_to_poly, poly_id, preset_to_poly};
use mpfree_synth::{Poly, Wavetable};

const RATE: u32 = 48_000;
const BLOCK: usize = 128;

/// A chord (C3, E3, G3, C4) at full velocity, held 1.5 s, let go, and
/// 1.5 s of tail; a mono sound plays the notes one after another, legato.
/// Full velocity, since the velocity's curve in between is Poly's (as for
/// Poly's own sounds saved before 19 Nov: the softest hit is as soft, a
/// hit at 100 about 0.8 dB louder).
fn phrase(node: &mut dyn Node) -> Vec<f32> {
    let frames = RATE as usize * 3;
    let notes = [48u8, 52, 55, 60];
    let mut evs: Vec<(usize, NodeEvent)> = Vec::new();
    for (k, &n) in notes.iter().enumerate() {
        evs.push((
            k * 64,
            NodeEvent::NoteOn {
                note: n,
                velocity: 127,
            },
        ));
    }
    for &n in &notes {
        evs.push((RATE as usize * 3 / 2, NodeEvent::NoteOff { note: n }));
    }
    let mut out = Vec::with_capacity(frames);
    let (mut l, mut r) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
    let mut at = 0;
    while at < frames {
        let ev: Vec<Timed> = evs
            .iter()
            .filter(|(f, _)| (at..at + BLOCK).contains(f))
            .map(|&(f, e)| Timed::new((f - at) as u32, e))
            .collect();
        l.fill(0.0);
        r.fill(0.0);
        let ctx = Ctx {
            rate: RATE,
            frame: at as u64,
            bpm: 120.0,
            beats: at as f64 * 2.0 / f64::from(RATE),
            playing: true,
            sidechain: None,
        };
        node.process(&ctx, &ev, &mut l, &mut r);
        out.extend(l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)));
        at += BLOCK;
    }
    out
}

fn db(x: f64) -> f64 {
    10.0 * x.max(1e-20).log10()
}

/// Power over `x`, in dB.
fn power_db(x: &[f32]) -> f64 {
    db(x.iter().map(|&v| f64::from(v) * f64::from(v)).sum::<f64>() / x.len() as f64)
}

/// Spectral centroid of `x` in octaves above 20 Hz, from a DFT over 4096
/// frames at a time (Hann), summed.
fn centroid_oct(x: &[f32]) -> f64 {
    const N: usize = 4096;
    let mut p = vec![0.0f64; N / 2];
    for chunk in x.chunks_exact(N).step_by(2) {
        for (k, pk) in p.iter_mut().enumerate().skip(1) {
            let (mut re, mut im) = (0.0, 0.0);
            // A coarse DFT: every 4th bin is enough for a centroid.
            if k % 4 != 0 {
                continue;
            }
            for (i, &v) in chunk.iter().enumerate() {
                let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / N as f64).cos();
                let ph = std::f64::consts::TAU * (k * i) as f64 / N as f64;
                re += f64::from(v) * w * ph.cos();
                im += f64::from(v) * w * ph.sin();
            }
            *pk += re * re + im * im;
        }
    }
    let hz = |k: usize| k as f64 * f64::from(RATE) / N as f64;
    let (mut num, mut den) = (0.0, 0.0);
    for (k, &pk) in p.iter().enumerate().skip(1) {
        if hz(k) > 20.0 {
            num += pk * (hz(k) / 20.0).log2();
            den += pk;
        }
    }
    num / den.max(1e-30)
}

/// The level over time, in dB, 100 ms windows.
fn shape_db(x: &[f32]) -> Vec<f64> {
    x.chunks(RATE as usize / 10).map(power_db).collect()
}

fn wavetable_with(p: &mpfree_synth::Preset) -> Wavetable {
    let mut w = Wavetable::new(RATE);
    w.load_preset(p);
    w
}

fn poly_with(values: &[(u16, f32)]) -> Poly {
    let mut s = Poly::new(RATE);
    for &(id, v) in values {
        s.set_param(id, v);
    }
    s
}

/// How `b` plays against `a`: its level, its brightness and the most its
/// level over time strays from `a`'s past the level difference, in dB,
/// octaves and dB.
fn against(a: &mut dyn Node, b: &mut dyn Node) -> (f64, f64, f64) {
    let (a, b) = (phrase(a), phrase(b));
    let loud = power_db(&b) - power_db(&a);
    let bright = centroid_oct(&b) - centroid_oct(&a);
    // The shape over time where the sound is no more than 30 dB down
    // (under that, a detuned stack's beating moves a window by more than
    // any change in the envelopes).
    let (sa, sb) = (shape_db(&a), shape_db(&b));
    let top = sa.iter().cloned().fold(f64::MIN, f64::max);
    let shape = sa
        .iter()
        .zip(&sb)
        .filter(|(x, _)| **x > top - 30.0)
        .map(|(x, y)| (y - x - loud).abs())
        .fold(0.0, f64::max);
    (loud, bright, shape)
}

/// Each Wavetable factory preset against the Poly sound `poly` makes of
/// it: within 0.75 dB, 0.1 octave and 2 dB.
fn check_all(poly: impl Fn(&mpfree_synth::Preset, &Wavetable) -> Poly) {
    let mut report = Vec::new();
    let mut bad = Vec::new();
    for p in Wavetable::presets() {
        let mut w = wavetable_with(p);
        let mut s = poly(p, &w);
        let (loud, bright, shape) = against(&mut w, &mut s);
        report.push(format!(
            "{:<16} level {loud:+.2} dB, brightness {bright:+.3} oct, shape within {shape:.2} dB",
            p.name
        ));
        if loud.abs() > 0.75 || bright.abs() > 0.1 || shape > 2.0 {
            bad.push(p.name);
        }
    }
    println!("{}", report.join("\n"));
    assert!(bad.is_empty(), "{bad:?}\n{}", report.join("\n"));
}

#[test]
fn every_factory_preset_plays_about_as_it_did() {
    check_all(|p, w| {
        let c = node_to_poly(w);
        assert_eq!(c.short_db, 0.0, "{}", p.name);
        poly_with(&c.values)
    });
}

/// Poly's Table presets are the conversions, rounded for the file.
#[test]
fn polys_table_presets_play_as_the_wavetable_ones() {
    check_all(|p, _| {
        let name = if p.name == "Init" {
            "Table Init"
        } else {
            p.name
        };
        let q = Poly::presets()
            .iter()
            .find(|q| q.name == name)
            .unwrap_or_else(|| panic!("Poly has no {name}"));
        let mut s = Poly::new(RATE);
        s.load_preset(q);
        s
    });
}

#[test]
fn every_setting_lands_on_its_poly_twin() {
    // Each shared setting at a value off its default on both engines.
    let mut w = Wavetable::new(RATE);
    for &(wid, pid) in to_poly::SAME {
        let info = w.params().iter().find(|p| p.id == wid).unwrap();
        let v = if info.max > info.min + 2.0 {
            info.min + (info.max - info.min) * 0.37
        } else {
            info.max
        };
        let v = if info.unit == mpfree_node::Unit::None {
            v.round()
        } else {
            v
        };
        w.set_param(wid, v);
        let c = node_to_poly(&w);
        let got = c.values.iter().find(|&&(id, _)| id == pid).unwrap().1;
        let want = match pid {
            pp::AMP_A | pp::AMP_R | pp::FENV_A | pp::FENV_R | pp::VEL_AMP => {
                mpfree_synth::poly::upgrade(1, pid, w.param(wid))
            }
            pp::LEVEL => (w.param(wid) + to_poly::GAIN_DB).min(6.0),
            _ => w.param(wid),
        };
        assert!(
            (got - want).abs() <= 1e-4 * want.abs().max(1.0),
            "{} ({wid} -> {pid}): {got} != {want}",
            info.name
        );
    }
    let c = node_to_poly(&w);
    let get = |id: u16| c.values.iter().find(|&&(i, _)| i == id).unwrap().1;
    assert_eq!(get(pp::OSC1_WAVE), f32::from(pp::WAVE_TABLE));
    assert_eq!(get(pp::OSC2_WAVE), f32::from(pp::WAVE_TABLE));
    assert_eq!(get(pp::SUB_WAVE), 1.0);
}

#[test]
fn pressure_keeps_its_targets_by_name() {
    let mut w = Wavetable::new(RATE);
    // Slot 1 on Position, 2 on Cutoff, 3 on Pitch, 4 on Warp.
    for (n, t) in [(0, 1.0), (1, 3.0), (2, 8.0), (3, 2.0)] {
        w.set_param(wp::press_target(n), t);
        w.set_param(wp::press_depth(n), 25.0 * (n + 1) as f32);
    }
    let c = node_to_poly(&w);
    let get = |id: u16| c.values.iter().find(|&&(i, _)| i == id).unwrap().1;
    let name = |n: usize| {
        mpfree_synth::pressure::POLY_TARGETS
            [get(pp::PRESS_BASE + mpfree_node::pressure::field::target(n)) as usize]
    };
    assert_eq!(
        [name(0), name(1), name(2), name(3)],
        ["Position", "Cutoff", "Pitch", "Warp"]
    );
    for n in 0..4 {
        assert_eq!(
            get(pp::PRESS_BASE + mpfree_node::pressure::field::depth(n)),
            25.0 * (n + 1) as f32
        );
    }
}

#[test]
fn a_loud_sound_says_how_far_its_level_falls_short() {
    let mut w = Wavetable::new(RATE);
    w.set_param(wp::LEVEL, 6.0);
    let c = node_to_poly(&w);
    assert!(
        (c.short_db - to_poly::GAIN_DB).abs() < 1e-3,
        "{}",
        c.short_db
    );
    w.set_param(wp::LEVEL, 0.0);
    assert_eq!(node_to_poly(&w).short_db, 0.0);
}

#[test]
fn a_saved_wavetable_preset_becomes_a_poly_preset() {
    let mut w = Wavetable::new(RATE);
    w.load_preset(&Wavetable::presets()[1]);
    let mut saved = Preset::capture(&w, "My Morph", &["pad"]);
    saved.extra.push((
        "qset".into(),
        format!("{} {} 999 - Tables", wp::POSITION, wp::CUTOFF),
    ));
    saved.extra.push(("macro".into(), "kept as it is".into()));
    let p = preset_to_poly(&saved);
    assert_eq!(p.engine, "mpfree.poly");
    assert_eq!(p.name, "My Morph");
    assert_eq!(p.tags, ["pad"]);
    assert_eq!(
        p.extra[0].1,
        format!("{} {} - - Tables", pp::OSC1_POS, pp::CUTOFF)
    );
    assert_eq!(p.extra[1].1, "kept as it is");
    assert_eq!(p.values, node_to_poly(&w).values);
    // It loads in Poly and reads back the same.
    let poly = Poly::new(RATE);
    let text = p.to_text(poly.params());
    let back = Preset::parse(&text, poly.info(), poly.params()).unwrap();
    assert_eq!(back.values, p.values);
    // Another engine's preset comes back as it is.
    let poly = Preset::capture(&Poly::new(RATE), "Plain", &[]);
    assert_eq!(preset_to_poly(&poly), poly);
    assert_eq!(poly_id(wp::OSC_B + wp::OSC_LEVEL), Some(pp::OSC2_LEVEL));
}

#[test]
fn a_sounds_effects_and_macros_come_along() {
    use mpfree_node::preset::{key_of, VERSION};
    let w = Wavetable::new(RATE);
    let wk = |id: u16| key_of(w.params(), id).unwrap();
    let pk = |id: u16| key_of(pp::PARAMS, id).unwrap();
    // Every setting of the Wavetable synth's has a place in Poly.
    assert!(w
        .params()
        .iter()
        .all(|p| p.readonly || poly_id(p.id).is_some()));
    let mut saved = Preset::capture(&w, "Mine", &[]);
    // An effect's setting and a macro's position (ids no engine has).
    saved.values.push((2_000, 0.25));
    saved.values.push((3_000, 0.5));
    saved.extra = vec![
        ("fx.1".into(), "mpfree.chorus".into()),
        ("macro.1".into(), "Bright".into()),
        (
            "macro.1.map".into(),
            format!("{} 10 80 exp", wk(wp::POSITION)),
        ),
        ("macro.1.map".into(), format!("{} 0 6", wk(wp::LEVEL))),
        ("macro.1.map".into(), format!("{} 0 100", wk(wp::VEL_AMP))),
        ("macro.1.map".into(), "fx-1.mix 0 1".into()),
    ];
    let p = preset_to_poly(&saved);
    assert_eq!(p.format, VERSION);
    assert!(p.values.contains(&(2_000, 0.25)) && p.values.contains(&(3_000, 0.5)));
    let maps: Vec<&str> = p
        .extra
        .iter()
        .filter(|(k, _)| k == "macro.1.map")
        .map(|(_, v)| v.as_str())
        .collect();
    let vel = mpfree_synth::poly::upgrade(1, pp::VEL_AMP, 100.0);
    assert_eq!(
        maps,
        [
            format!("{} 10 80 exp", pk(pp::OSC1_POS)),
            format!(
                "{} {} {}",
                pk(pp::LEVEL),
                to_poly::GAIN_DB,
                6.0 + to_poly::GAIN_DB
            ),
            format!("{} 0 {vel}", pk(pp::VEL_AMP)),
            "fx-1.mix 0 1".to_string(),
        ]
    );
    assert_eq!(&p.extra[..2], &saved.extra[..2]);
}
