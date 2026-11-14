//! What sleep on silence saves (chain.rs): a mixer of 64 tracks, each
//! with an EQ and a compressor, where 8 tracks play at a time, in turns,
//! as in a song. Runs the same song with the inserts allowed to sleep and
//! kept awake, alternately, and with no inserts at all, and prints the
//! mixer's time per 128-frame block for each. Run with `--release`; desktop numbers.

use mpfree_fx::ids::mixer::{self as mx, output};
use mpfree_fx::mpfree_node::Ctx;
use mpfree_fx::{Compressor, Eq, Mixer, MixerConfig, StripId};
use std::time::Instant;

const RATE: u32 = 44_100;
const BLOCK: usize = 128;
const BLOCKS: u64 = 8_000;
const TRACKS: usize = 64;

fn song(sleep: bool, inserts: bool) -> f64 {
    let cfg = MixerConfig {
        tracks: TRACKS,
        ..MixerConfig::default()
    };
    let mut m = Mixer::new(cfg, RATE, 1024);
    for t in 0..TRACKS {
        let s = StripId::Track(t);
        m.set_now(s, mx::OUTPUT, output::MASTER);
        let c = m.chain_mut(s).unwrap();
        if inserts {
            c.push(Box::new(Eq::new()));
            c.push(Box::new(Compressor::new()));
        }
        c.set_sleep(sleep);
    }
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut seed = 1u32;
    let start = Instant::now();
    for b in 0..BLOCKS {
        // Tracks 8k..8k+8 play in turn k, about 3 s each.
        let turn = (b / 1000) as usize % (TRACKS / 8);
        for t in turn * 8..turn * 8 + 8 {
            let (il, ir) = m.track_input(t).unwrap();
            for i in 0..BLOCK {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                il[i] = (seed >> 9) as f32 / (1u32 << 23) as f32 * 0.4 - 0.2;
                ir[i] = il[i];
            }
        }
        m.process(&Ctx::new(RATE, b * BLOCK as u64), &mut l, &mut r);
    }
    start.elapsed().as_secs_f64() / BLOCKS as f64 * 1e6
}

fn main() {
    let period_us = BLOCK as f64 / RATE as f64 * 1e6;
    let (mut awake, mut asleep, mut none) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..3 {
        awake.push(song(false, true));
        asleep.push(song(true, true));
        none.push(song(true, false));
    }
    let best = |v: &[f64]| v.iter().cloned().fold(f64::MAX, f64::min);
    for (what, v) in [
        ("kept awake", &awake),
        ("may sleep", &asleep),
        ("no inserts", &none),
    ] {
        println!(
            "{what}: {:.1} us a block ({:.1} % of a period), best of 3",
            best(v),
            best(v) / period_us * 100.0
        );
    }
}
