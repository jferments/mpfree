//! Times the synths on this computer: how long one 128-frame block takes
//! with 1, 4, 8 and 16 voices sounding, as a share of the block's real time
//! at 48 kHz. Poly runs with one and four unison stacks; FM runs its two
//! heaviest presets (Brass Section: algorithm 22 with three carriers and
//! the pitch envelope; Glass Keys: algorithm 5 with feedback).
//!
//! `cargo run --release -p mpfree-synth --example bench`. The MPC One's
//! Cortex-A17 cores are several times slower than a desktop core, so run
//! it there (cross-built) before trusting the numbers for the device.

use std::time::Instant;

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::params::*;
use mpfree_synth::{fm_params, Fm, Poly};

fn main() {
    let rate = 48_000u32;
    let block = 128usize;
    let blocks = 3_000u64;
    println!("Poly, {block}-frame blocks at {rate} Hz ({:.2} ms each), {blocks} blocks per row", block as f64 * 1000.0 / f64::from(rate));
    for unison in [1.0f32, 4.0] {
        for voices in [1usize, 4, 8, 16] {
            let mut s = Poly::new(rate);
            s.set_param(UNISON, unison);
            s.set_param(POLYPHONY, 16.0);
            s.set_param(LFO_CUTOFF, 1.0);
            s.set_param(DRIVE, 6.0);
            let per = time_blocks(&mut s, rate, block, blocks, voices);
            let budget = block as f64 / f64::from(rate);
            println!("  unison {unison:.0}, {voices:>2} voices: {:>7.1} us per block, {:>5.2} % of real time", per * 1e6, per / budget * 100.0);
        }
    }
    println!("FM, same blocks");
    let presets = Fm::presets();
    for name in ["Brass Section", "Glass Keys"] {
        let preset = presets.iter().find(|p| p.name == name).expect("an FM preset");
        for voices in [1usize, 4, 8, 16] {
            let mut s = Fm::new(rate);
            s.load_preset(preset);
            s.set_param(fm_params::POLYPHONY, 16.0);
            let per = time_blocks(&mut s, rate, block, blocks, voices);
            let budget = block as f64 / f64::from(rate);
            println!("  {:<14} {voices:>2} voices: {:>7.1} us per block, {:>5.2} % of real time", format!("{name},"), per * 1e6, per / budget * 100.0);
        }
    }
}

/// Holds `voices` notes and returns the mean seconds per block.
fn time_blocks(s: &mut dyn Node, rate: u32, block: usize, blocks: u64, voices: usize) -> f64 {
    let mut l = vec![0.0f32; block];
    let mut r = vec![0.0f32; block];
    let notes: Vec<Timed> = (0..voices).map(|i| Timed::new(0, NodeEvent::NoteOn { note: 36 + 3 * i as u8, velocity: 100 })).collect();
    s.process(&Ctx::new(rate, 0), &notes, &mut l, &mut r);
    let t = Instant::now();
    for b in 1..=blocks {
        l.fill(0.0);
        r.fill(0.0);
        s.process(&Ctx::new(rate, b * block as u64), &[], &mut l, &mut r);
    }
    std::hint::black_box(&l);
    t.elapsed().as_secs_f64() / blocks as f64
}
