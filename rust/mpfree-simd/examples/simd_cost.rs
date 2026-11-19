//! Plays a kernel's bench workload for a number of blocks, for counting its
//! instructions as ARMv7 code under qemu (`tools/simd_arm_cost.py`).
//!
//! ```text
//! SIMD_COST_PATH=plain|neon SIMD_COST_BLOCKS=N simd_cost
//! ```
//!
//! After 50 warm-up blocks it plays N more (200 by default) of the supersaw
//! workload, 8 voices of 7 saws at 128 frames, on the chosen path, and
//! prints the path that ran and a hash of the last block's samples (the
//! same on both paths: the kernel matches its twin bit for bit). It reads
//! plain ramps the size of the band-limited levels: the count is the same,
//! and building those would take most of a traced run.

use mpfree_simd::workload::{ramp_levels, Supersaw};
use mpfree_simd::Path;

fn main() {
    let path = match std::env::var("SIMD_COST_PATH").as_deref() {
        Ok("neon") => Path::Neon,
        _ => Path::Plain,
    };
    let blocks: u32 = std::env::var("SIMD_COST_BLOCKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    let mut s = Supersaw::with_levels(8, 44_100, ramp_levels());
    let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
    let mut took = path;
    for _ in 0..50 + blocks {
        took = s.block(path, &mut l, &mut r);
    }
    let hash = l.iter().chain(&r).fold(0xcbf2_9ce4_8422_2325u64, |h, x| {
        (h ^ u64::from(x.to_bits())).wrapping_mul(0x100_0000_01b3)
    });
    println!("supersaw 8 voices {} {hash:016x}", took.name());
}
