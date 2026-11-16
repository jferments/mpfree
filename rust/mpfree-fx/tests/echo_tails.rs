//! The delays' tails (FX quality plan, D12). What each reports covers its
//! echoes down to -60 dB, so a bypassed slot or a resting chain never cuts
//! one off; and a loop that gains on each pass rings on until its Feedback
//! comes down.

use mpfree_fx::ids::{multitap as mt, tape};
use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_fx::{presets, registry};

const RATE: u32 = 48_000;
const BLOCK: usize = 128;
const IDS: [&str; 3] = [
    "mpfree.fx.delay",
    "mpfree.fx.tape_delay",
    "mpfree.fx.multitap",
];

/// The engine at its defaults, then once at each factory preset.
fn setups(id: &str) -> Vec<(&'static str, Box<dyn Node>)> {
    let mut out = vec![("defaults", registry::make(id).unwrap())];
    for p in presets(id) {
        let mut n = registry::make(id).unwrap();
        for &(k, v) in p.values {
            n.set_param(k, v);
        }
        out.push((p.name, n));
    }
    for (_, n) in &mut out {
        n.prepare(RATE, BLOCK);
        n.reset();
    }
    out
}

/// A quarter of a second of noise at about -18 dB RMS, then `quiet`
/// frames of silence; the mid channel out.
fn render(n: &mut dyn Node, quiet: usize) -> Vec<f32> {
    let burst = RATE as usize / 4;
    let mut s = 0x1234_5678_u32;
    let x: Vec<f32> = (0..burst + quiet)
        .map(|i| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            if i < burst {
                ((s >> 8) as f32 / (1u32 << 23) as f32 - 1.0) * 0.217
            } else {
                0.0
            }
        })
        .collect();
    let (mut l, mut r) = (x.clone(), x);
    let mut frame = 0u64;
    for (a, b) in l.chunks_mut(BLOCK).zip(r.chunks_mut(BLOCK)) {
        n.process(&Ctx::new(RATE, frame), &[], a, b);
        frame += a.len() as u64;
    }
    l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect()
}

fn rms_db(x: &[f32]) -> f64 {
    let p = x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len() as f64;
    10.0 * p.max(1e-20).log10()
}

#[test]
fn every_tail_covers_its_echoes_down_to_60_db() {
    let cap = 30 * RATE as usize;
    for id in IDS {
        for (name, mut n) in setups(id) {
            let tail = n.tail_frames() as usize;
            if tail >= cap {
                // Rings on, or at the 30 s most: nothing to cut.
                continue;
            }
            let w = RATE as usize / 10;
            let out = render(n.as_mut(), tail);
            let burst = RATE as usize / 4;
            let input = rms_db(&out[..burst]).max(-18.5);
            let end = burst + tail;
            let cut = rms_db(&out[end - w..end]) - input;
            assert!(
                cut <= -60.0,
                "{id} {name}: {cut:.1} dB at its {tail}-frame tail"
            );
        }
    }
}

#[test]
fn a_loop_that_gains_rings_on_until_feedback_comes_down() {
    let mut t = registry::make("mpfree.fx.tape_delay").unwrap();
    t.prepare(RATE, BLOCK);
    let finite = t.tail_frames();
    assert!(finite < 30 * RATE, "defaults: {finite}");
    // Over 100 % the loop grows: it rings on.
    t.set_param(tape::FEEDBACK, 120.0);
    assert_eq!(t.tail_frames(), u32::MAX);
    // So does the preset that runs away by design; Dub echo, at 80 %,
    // dies away (its saturation adds no gain).
    for (name, n) in setups("mpfree.fx.tape_delay") {
        match name {
            "Runaway" => assert_eq!(n.tail_frames(), u32::MAX, "{name}"),
            "Dub echo" => assert!(n.tail_frames() < 30 * RATE, "{name}"),
            _ => {}
        }
    }
    // And back down, it dies away again.
    t.process(
        &Ctx::new(RATE, 0),
        &[Timed::new(
            0,
            NodeEvent::Param {
                id: tape::FEEDBACK,
                value: 45.0,
            },
        )],
        &mut [0.0; BLOCK],
        &mut [0.0; BLOCK],
    );
    assert_eq!(t.tail_frames(), finite);
    // The Multi-tap's Feedback stops short of 100 %: it always dies away.
    let mut m = registry::make("mpfree.fx.multitap").unwrap();
    m.prepare(RATE, BLOCK);
    m.set_param(mt::FEEDBACK, 95.0);
    assert!(m.tail_frames() <= 30 * RATE);
}
