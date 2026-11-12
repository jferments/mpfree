//! Every factory sound of every engine saved as preset text and loaded
//! into a fresh engine plays the same to the bit, and the text reads like
//! the Synth tab.

use mpfree_instruments::preset::{engine_of, Preset};
use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::{Drums, Fm, Pluck, Poly, Wavetable};

const RATE: u32 = 48_000;
const BLOCK: usize = 128;

/// An engine and its factory sounds, each loaded into a fresh engine.
struct Engine {
    name: &'static str,
    sounds: fn() -> usize,
    fresh: fn() -> Box<dyn Node>,
    factory: fn(usize) -> Box<dyn Node>,
}

const ENGINES: [Engine; 5] = [
    Engine {
        name: "Poly",
        sounds: || Poly::presets().len(),
        fresh: || Box::new(Poly::new(RATE)),
        factory: |i| {
            let mut s = Poly::new(RATE);
            s.load_preset(&Poly::presets()[i]);
            Box::new(s)
        },
    },
    Engine {
        name: "FM",
        sounds: || Fm::presets().len(),
        fresh: || Box::new(Fm::new(RATE)),
        factory: |i| {
            let mut s = Fm::new(RATE);
            s.load_preset(&Fm::presets()[i]);
            Box::new(s)
        },
    },
    Engine {
        name: "Wavetable",
        sounds: || Wavetable::presets().len(),
        fresh: || Box::new(Wavetable::new(RATE)),
        factory: |i| {
            let mut s = Wavetable::new(RATE);
            s.load_preset(&Wavetable::presets()[i]);
            Box::new(s)
        },
    },
    Engine {
        name: "Pluck",
        sounds: || Pluck::presets().len(),
        fresh: || Box::new(Pluck::new(RATE)),
        factory: |i| {
            let mut s = Pluck::new(RATE);
            s.load_preset(&Pluck::presets()[i]);
            Box::new(s)
        },
    },
    Engine {
        name: "Drums",
        sounds: || Drums::presets().len(),
        fresh: || Box::new(Drums::new(RATE)),
        factory: |i| {
            let mut s = Drums::new(RATE);
            s.load_preset(&Drums::presets()[i]);
            Box::new(s)
        },
    },
];

/// A short phrase: a chord, a high note over it, all released, with pad
/// notes for Drums (36 up).
fn phrase(node: &mut dyn Node) -> Vec<f32> {
    let frames = RATE as usize * 3 / 4;
    let ev = |f: usize, e: NodeEvent| (f, e);
    let on = |note: u8| NodeEvent::NoteOn {
        note,
        velocity: 100,
    };
    let off = |note: u8| NodeEvent::NoteOff { note };
    let events = [
        ev(0, on(48)),
        ev(0, on(36)),
        ev(64, on(55)),
        ev(64, on(38)),
        ev(9_000, on(67)),
        ev(9_000, on(42)),
        ev(20_000, off(48)),
        ev(20_000, off(36)),
        ev(20_000, off(38)),
        ev(20_000, off(42)),
        ev(20_100, off(55)),
        ev(24_000, off(67)),
    ];
    let mut out = Vec::with_capacity(frames * 2);
    let mut l = [0.0f32; BLOCK];
    let mut r = [0.0f32; BLOCK];
    let mut buf = Vec::new();
    let mut at = 0;
    while at < frames {
        buf.clear();
        for &(f, e) in &events {
            if f >= at && f < at + BLOCK {
                buf.push(Timed::new((f - at) as u32, e));
            }
        }
        l.fill(0.0);
        r.fill(0.0);
        node.process(&Ctx::new(RATE, at as u64), &buf, &mut l, &mut r);
        out.extend_from_slice(&l);
        out.extend_from_slice(&r);
        at += BLOCK;
    }
    out
}

#[test]
fn every_factory_sound_saves_and_loads_to_the_bit() {
    let mut checked = 0;
    for e in &ENGINES {
        for i in 0..(e.sounds)() {
            let mut a = (e.factory)(i);
            let saved = Preset::capture(a.as_ref(), "Saved", &["test"]);
            let text = saved.to_text(a.params());
            assert_eq!(engine_of(&text), Some(a.info().id));
            let mut b = (e.fresh)();
            let loaded = Preset::parse(&text, b.info(), b.params()).unwrap();
            assert_eq!(loaded, saved, "{} sound {i}", e.name);
            loaded.apply(b.as_mut()).unwrap();
            for p in a.params().iter().filter(|p| !p.readonly) {
                assert_eq!(
                    a.param(p.id).to_bits(),
                    b.param(p.id).to_bits(),
                    "{} sound {i}: {}",
                    e.name,
                    p.name
                );
            }
            let (pa, pb) = (phrase(a.as_mut()), phrase(b.as_mut()));
            assert!(
                pa.iter().any(|x| *x != 0.0),
                "{} sound {i} is silent",
                e.name
            );
            assert!(pa == pb, "{} sound {i} plays differently", e.name);
            checked += 1;
        }
    }
    eprintln!("{checked} factory sounds checked");
    assert!(checked > 40, "only {checked} sounds");
}

#[test]
fn odd_values_save_and_load_to_the_bit() {
    // Every setting at an awkward value, as a knob leaves it.
    let mut seed = 0x2545_f491_u32;
    for e in &ENGINES {
        let mut a = (e.fresh)();
        for p in a.params().to_vec() {
            if p.readonly {
                continue;
            }
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let t = (seed >> 8) as f32 / (1u32 << 24) as f32;
            a.set_param(p.id, p.min + (p.max - p.min) * t);
        }
        let text = Preset::capture(a.as_ref(), "Odd", &[]).to_text(a.params());
        let mut b = (e.fresh)();
        Preset::parse(&text, b.info(), b.params())
            .unwrap()
            .apply(b.as_mut())
            .unwrap();
        for p in a.params().iter().filter(|p| !p.readonly) {
            assert_eq!(
                a.param(p.id).to_bits(),
                b.param(p.id).to_bits(),
                "{}: {} {} became {}",
                e.name,
                p.name,
                a.param(p.id),
                b.param(p.id)
            );
        }
        let bits = |x: Vec<f32>| x.into_iter().map(f32::to_bits).collect::<Vec<_>>();
        assert!(
            bits(phrase(a.as_mut())) == bits(phrase(b.as_mut())),
            "{} plays differently",
            e.name
        );
    }
}

#[test]
fn native_engines_name_every_setting() {
    for e in &ENGINES {
        let node = (e.fresh)();
        let text = Preset::capture(node.as_ref(), "Init", &[]).to_text(node.params());
        let id_keys: Vec<&str> = text.lines().filter(|l| l.starts_with("id.")).collect();
        assert!(id_keys.is_empty(), "{}: {id_keys:?}", e.name);
        let settings = text
            .lines()
            .filter(|l| l.contains(" = ") && l.contains('.') && !l.starts_with('#'))
            .filter(|l| !l.starts_with("engine"))
            .count();
        let writable = node.params().iter().filter(|p| !p.readonly).count();
        assert_eq!(settings, writable, "{}", e.name);
    }
    let poly = Poly::new(RATE);
    let text = Preset::capture(&poly, "Init", &[]).to_text(poly.params());
    for line in ["filter.cutoff = ", "amp-env.attack = ", "osc-1.wave = Saw"] {
        assert!(text.contains(line), "Poly has no {line:?} line:\n{text}");
    }
}

#[test]
fn a_setting_the_file_lacks_takes_its_default() {
    // A preset saved before a setting existed: Poly's cutoff line removed.
    let mut a = Poly::new(RATE);
    a.load_preset(&Poly::presets()[3]);
    let text: String = Preset::capture(&a, "Old", &[])
        .to_text(a.params())
        .lines()
        .filter(|l| !l.starts_with("filter.cutoff"))
        .map(|l| format!("{l}\n"))
        .collect();
    let cutoff = mpfree_synth::params::CUTOFF;
    let mut b = Poly::new(RATE);
    b.set_param(cutoff, 123.0);
    let p = Preset::parse(&text, b.info(), b.params()).unwrap();
    p.apply(&mut b).unwrap();
    let default = b.params().iter().find(|p| p.id == cutoff).unwrap().default;
    assert_eq!(b.param(cutoff), default);
}

#[test]
fn a_preset_for_another_engine_is_refused() {
    let poly = Poly::new(RATE);
    let text = Preset::capture(&poly, "Pad", &[]).to_text(poly.params());
    let mut fm = Fm::new(RATE);
    assert!(Preset::parse(&text, fm.info(), fm.params()).is_err());
    let p = Preset::parse(&text, poly.info(), poly.params()).unwrap();
    let before: Vec<f32> = fm.params().iter().map(|q| fm.param(q.id)).collect();
    assert!(p.apply(&mut fm).is_err());
    let after: Vec<f32> = fm.params().iter().map(|q| fm.param(q.id)).collect();
    assert_eq!(before, after, "a refused preset changes nothing");
}
