//! The delays', reverbs' and chorus's Mix curve (FX quality plan 8.1.1,
//! and round 3's decision 8.1 for the chorus). Every engine of the Delay
//! and Reverb families, and the Chorus, has it, after the controls it
//! had (so an effect saved before it loads at its default, Equal), with
//! the same three choices (so a mode change carries it). Under every curve
//! Mix 0 is the input bit for bit and Mix 100 % the effect alone; between
//! them each frame is the curve's dry gain times the input plus its wet
//! gain times the effect alone; and a change of curve glides from one
//! blend to the other instead of stepping.

use std::f64::consts::FRAC_PI_2;

use mpfree_fx::mpfree_node::{Ctx, Curve, Node, NodeEvent, Timed};
use mpfree_fx::{families, registry};

const RATE: u32 = 48_000;
const BLOCK: usize = 128;
const CURVES: [&str; 3] = ["Equal", "Dry first", "Linear"];

/// Every engine of the Delay and Reverb families, once, and the Chorus.
fn engines() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for f in families::FAMILIES
        .iter()
        .filter(|f| matches!(f.name, "Delay" | "Reverb"))
    {
        for m in f.modes {
            if !out.contains(&m.id) {
                out.push(m.id);
            }
        }
    }
    out.push("mpfree.fx.chorus");
    out
}

fn id_of(n: &dyn Node, name: &str) -> u16 {
    n.params()
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("{} has no {name}", n.info().id))
        .id
}

/// Half a second of noise at about -12 dBFS, then half a second of
/// silence, so the tails count too.
fn input() -> Vec<f32> {
    let mut s = 0x2545_f491_u32;
    (0..RATE as usize)
        .map(|i| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            if i < RATE as usize / 2 {
                (s >> 8) as f32 / (1u32 << 23) as f32 * 0.25 - 0.25
            } else {
                0.0
            }
        })
        .collect()
}

/// `x` on both sides through engine `id` set by `values` (by name) from a
/// reset, so every control starts where it is set; `change` is a control
/// (by name) set to a value at a frame on the way.
fn render(
    id: &str,
    values: &[(&str, f32)],
    change: Option<(usize, &str, f32)>,
    x: &[f32],
) -> (Vec<f32>, Vec<f32>) {
    let mut n = registry::make(id).unwrap();
    n.prepare(RATE, BLOCK);
    for &(name, v) in values {
        let p = id_of(n.as_ref(), name);
        n.set_param(p, v);
    }
    n.reset();
    let change = change.map(|(at, name, v)| (at, id_of(n.as_ref(), name), v));
    let (mut l, mut r) = (x.to_vec(), x.to_vec());
    let mut frame = 0usize;
    for (a, b) in l.chunks_mut(BLOCK).zip(r.chunks_mut(BLOCK)) {
        let mut events = Vec::new();
        if let Some((at, p, v)) = change {
            if (frame..frame + a.len()).contains(&at) {
                events.push(Timed::new(
                    (at - frame) as u32,
                    NodeEvent::Param { id: p, value: v },
                ));
            }
        }
        n.process(&Ctx::new(RATE, frame as u64), &events, a, b);
        frame += a.len();
    }
    (l, r)
}

/// The dry and wet gains curve `c` gives at Mix `m` (0 to 1).
fn gains(c: usize, m: f64) -> (f64, f64) {
    match c {
        0 => ((m * FRAC_PI_2).cos(), (m * FRAC_PI_2).sin()),
        1 => ((2.0 - 2.0 * m).min(1.0), (2.0 * m).min(1.0)),
        _ => (1.0 - m, m),
    }
}

/// Controls added after the Mix curve, appended in turn (engine, name).
const ADDED_AFTER: [(&str, &str); 1] = [("mpfree.fx.room", "Diffusion")];

#[test]
fn every_delay_and_reverb_has_the_mix_curve_after_its_older_controls() {
    let all = engines();
    assert_eq!(all.len(), 8, "{all:?}");
    for id in all {
        let n = registry::make(id).unwrap();
        let ps = n.params();
        let p = ps.iter().find(|p| p.name == "Mix curve").expect(id);
        assert_eq!(p.curve, Curve::Choice(&CURVES), "{id}");
        assert_eq!((p.default, p.min, p.max), (0.0, 0.0, 2.0), "{id}");
        assert_eq!(p.group, "Output", "{id}");
        for q in ps.iter().filter(|q| q.id > p.id) {
            assert!(
                ADDED_AFTER.contains(&(id, q.name)),
                "{id}: {} comes after the Mix curve",
                q.name
            );
        }
        assert!(ps.iter().any(|q| q.name == "Mix"), "{id}");
        assert!(mpfree_fx::page_keeps(p.name), "presets leave it alone");
        assert_eq!(n.param(p.id), 0.0, "{id}: starts at Equal");
    }
}

#[test]
fn mix_0_is_the_input_and_100_the_effect_alone_on_every_curve() {
    let x = input();
    for id in engines() {
        let (wl, wr) = render(id, &[("Mix", 100.0), ("Mix curve", 2.0)], None, &x);
        for c in 0..3 {
            let curve = c as f32;
            let (l, r) = render(id, &[("Mix", 0.0), ("Mix curve", curve)], None, &x);
            assert!(l == x && r == x, "{id} {}: Mix 0 is the input", CURVES[c]);
            let (l, r) = render(id, &[("Mix", 100.0), ("Mix curve", curve)], None, &x);
            assert!(
                l == wl && r == wr,
                "{id} {}: Mix 100 % is the effect",
                CURVES[c]
            );
        }
    }
}

#[test]
fn between_them_each_curve_blends_by_its_gains() {
    let x = input();
    for id in engines() {
        let (wl, wr) = render(id, &[("Mix", 100.0)], None, &x);
        for c in 0..3 {
            for mix in [10.0f32, 25.0, 50.0, 75.0] {
                let set = [("Mix", mix), ("Mix curve", c as f32)];
                let (l, r) = render(id, &set, None, &x);
                let (d, w) = gains(c, f64::from(mix) / 100.0);
                for (got, wet) in [(&l, &wl), (&r, &wr)] {
                    for i in 0..x.len() {
                        let (dry, e) = (f64::from(x[i]), f64::from(wet[i]));
                        let want = d * dry + w * e;
                        // The gains come from a polynomial sine (within
                        // 0.2 % of the true curve).
                        let err = (f64::from(got[i]) - want).abs();
                        assert!(
                            err <= 0.002 * (dry.abs() + e.abs()) + 1e-6,
                            "{id} {} at {mix} %: frame {i} {} against {want}",
                            CURVES[c],
                            got[i]
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn a_change_of_curve_glides() {
    let x = input();
    let at = RATE as usize / 4;
    for id in engines() {
        let set = |c: f32| [("Mix", 35.0), ("Mix curve", c)];
        let (old, _) = render(id, &set(0.0), None, &x);
        let (new, _) = render(id, &set(1.0), None, &x);
        let (got, _) = render(id, &set(0.0), Some((at, "Mix curve", 1.0)), &x);
        assert_eq!(got[..at], old[..at], "{id}: nothing moves before");
        // The first frames after the change are still near the old blend;
        // a quarter of a second later (five of the reverbs' 50 ms glide
        // times) it is within 1 % of the new one.
        let gap = |a: &[f32], b: &[f32]| -> f64 {
            a.iter()
                .zip(b)
                .map(|(p, q)| f64::from(p - q).abs())
                .sum::<f64>()
        };
        let whole = gap(&old[at..at + 16], &new[at..at + 16]);
        assert!(whole > 0.0, "{id}: the curves differ at 35 %");
        let moved = gap(&got[at..at + 16], &old[at..at + 16]);
        assert!(moved < 0.2 * whole, "{id}: {moved} of {whole} in 16 frames");
        let end = at + RATE as usize / 4;
        let left = gap(&got[end..end + 256], &new[end..end + 256]);
        let span = gap(&old[end..end + 256], &new[end..end + 256]);
        assert!(left <= 0.01 * span + 1e-6, "{id}: {left} of {span} left");
    }
}
