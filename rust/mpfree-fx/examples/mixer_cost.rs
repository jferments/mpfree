//! Where the mixer's time goes: runs a mixer with profiling on and prints
//! the whole mixer's time per block and each strip's, slowest first. Run
//! with `--release`. Desktop numbers; `mpfree-bench` has the MPC model.

use mpfree_fx::ids::mixer::{self as mx, output};
use mpfree_fx::mpfree_node::Ctx;
use mpfree_fx::{Mixer, MixerConfig, StripId};

const RATE: u32 = 44_100;
const BLOCK: usize = 128;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let tracks: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(12);
    let pads: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(8);
    let mut m = Mixer::new(MixerConfig::default(), RATE, 1024);
    for p in 0..pads {
        m.set_now(StripId::Pad(p), mx::OUTPUT, output::TRACK);
        m.set_now(StripId::Pad(p), mx::send(1), -12.0);
    }
    for t in 0..tracks {
        m.set_now(StripId::Track(t), mx::OUTPUT, output::bus(t % 3));
        m.set_now(StripId::Track(t), mx::send(0), -18.0);
    }
    m.set_profiling(true);
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut seed = 1u32;
    for b in 0..20_000u64 {
        for t in 0..tracks {
            let (il, ir) = m.track_input(t).unwrap();
            for i in 0..BLOCK {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                il[i] = (seed >> 9) as f32 / (1u32 << 23) as f32 * 0.2 - 0.1;
                ir[i] = il[i];
            }
        }
        for p in 0..pads {
            let (il, ir) = m.pad_input(p).unwrap();
            il[..BLOCK].fill(0.1);
            ir[..BLOCK].fill(0.1);
        }
        m.process(&Ctx::new(RATE, b * BLOCK as u64), &mut l, &mut r);
    }
    let whole = m.probe().lifetime();
    println!("{tracks} tracks, {pads} pads: whole mixer mean {:.2} us, p99 {:.2} us per {BLOCK}-frame block", whole.mean_ns / 1e3, whole.p99_ns as f64 / 1e3);
    let mut rows: Vec<(StripId, f64)> = m.strips().map(|s| (s, m.strip_probe(s).unwrap().lifetime().mean_ns / 1e3)).collect();
    let sum: f64 = rows.iter().map(|r| r.1).sum();
    println!("  sum of strips {sum:.2} us, outside the strips {:.2} us", whole.mean_ns / 1e3 - sum);
    rows.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (s, us) in rows.iter().take(12) {
        println!("  {s:?}: {us:.3} us");
    }
}
