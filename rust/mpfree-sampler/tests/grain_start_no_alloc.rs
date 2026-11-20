//! Sampler plan, phase 1 step 1: finding where a sound's grains start
//! allocates nothing, however long the sound. Before, a 25-minute take
//! asked for about 1.3 GB at once (two 8-byte numbers a frame, and a float
//! copy of a 16-bit sound), enough to get mpfree killed on the MPC's 2 GB
//! with no swap. A long region is searched in its loudness summary
//! ([`Region::loudest_grain_from`]); a short one where its frames are
//! ([`Region::loudest_grain`]).
//!
//! This file installs mpfree-prof's counting allocator (per thread).

use mpfree_analysis::PeakPyramid;
use mpfree_prof::alloc::{self, CountingAlloc};
use mpfree_sampler::model::*;

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const RATE: u32 = 44_100;

#[test]
fn finding_the_grain_start_allocates_nothing_for_a_long_take() {
    assert!(alloc::installed());
    // A 95-second 16-bit stereo take (the search's reads grow with the
    // length; what it allocates doesn't), loud at 61 s.
    let n = 1usize << 22;
    let mut data = Vec::with_capacity(2 * n);
    let mut seed = 0x2545_F491u32;
    for i in 0..n {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let noise = (seed as f32 / u32::MAX as f32 - 0.5) * 0.02;
        let t = i as f32 / RATE as f32;
        let loud = if (61.0..61.2).contains(&t) { 0.8 } else { 0.0 };
        let x = noise + loud * (t * 300.0 * std::f32::consts::TAU).sin();
        let side = |v: f32| ((v * 32_768.0).round() + 0.0) / 32_768.0;
        data.push(side(x));
        data.push(side(-x * 0.5));
    }
    let take = Source::new("take", RATE, 2, data).unwrap();
    assert!(take.data().is_16_bit());
    assert!(take.frames() > 4 * SCAN_FRAMES);
    let summary = PeakPyramid::build_from(take.data(), 2).unwrap();
    let mut whole = Region::whole(&take);
    let mut short = Region::whole(&take);
    short.start = (60.0 * RATE as f32) as u64;
    short.end = short.start + SCAN_FRAMES;
    for pick in [
        ChannelPick::Stereo,
        ChannelPick::MonoSum,
        ChannelPick::Left,
        ChannelPick::Right,
    ] {
        whole.channels = pick;
        short.channels = pick;
        let before = alloc::thread_count();
        let at = whole.loudest_grain_from(&take, 80.0, &summary);
        let near = short.loudest_grain(&take, 80.0);
        let calls = alloc::thread_count() - before;
        assert_eq!(calls, 0, "{pick:?}: {calls} allocations");
        let secs = f64::from(at.unwrap()) * take.frames() as f64 / f64::from(RATE);
        assert!((60.95..61.2).contains(&secs), "{pick:?}: {secs} s");
        let secs = 60.0 + f64::from(near) * SCAN_FRAMES as f64 / f64::from(RATE);
        assert!((60.95..61.2).contains(&secs), "{pick:?}: {secs} s");
    }
}
