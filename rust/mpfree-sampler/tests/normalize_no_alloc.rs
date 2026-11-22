//! Sampler plan, phase 1 step 5: Normalize finds a long take's loudest
//! frame with no copy and nothing allocated, quickly. Before, it read a
//! 16-bit take through a float copy of all of it: about 530 MB at once for
//! a 25-minute stereo take, on the MPC's 2 GB with no swap, on the screen
//! thread. A long region is searched in its loudness summary
//! ([`Region::peak_from`]); a short one where its frames are
//! ([`Region::peak`]).
//!
//! The 25-minute timing (`--ignored`, in release) also writes down what
//! Put on audio track takes on the same take, so step 7, which moves that
//! work off the screen, can show it gone.
//!
//! This file installs mpfree-prof's counting allocator (per thread).

use mpfree_analysis::PeakPyramid;
use mpfree_prof::alloc::{self, CountingAlloc};
use mpfree_sampler::model::*;
use std::time::{Duration, Instant};

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const RATE: u32 = 44_100;

const PICKS: [ChannelPick; 4] = [
    ChannelPick::Stereo,
    ChannelPick::MonoSum,
    ChannelPick::Left,
    ChannelPick::Right,
];

/// A 16-bit stereo take of `n` frames: a soft wandering tone over noise,
/// the right side half the left with noise of its own, and a loud spot
/// at `loud` s.
fn take(n: usize, loud: f32) -> Source {
    let mut data = Vec::with_capacity(2 * n);
    let mut seed = 0x2545_F491u32;
    let mut noise = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32 / u32::MAX as f32 - 0.5) * 0.02
    };
    let side = |v: f32| ((v.clamp(-1.0, 0.99) * 32_768.0).round() + 0.0) / 32_768.0;
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        let swell = 0.2 + 0.1 * (t * 0.37).sin();
        let a = if (loud..loud + 0.2).contains(&t) {
            0.8
        } else {
            swell
        };
        let x = a * (t * 220.0 * std::f32::consts::TAU).sin() + noise();
        data.push(side(x));
        data.push(side(0.5 * x + noise()));
    }
    let s = Source::new("take", RATE, 2, data).unwrap();
    assert!(s.data().is_16_bit());
    s
}

#[test]
fn normalize_finds_a_long_takes_peak_with_no_copy() {
    assert!(alloc::installed());
    // 95 s (what the search reads grows with the length; what it
    // allocates doesn't), loud at 61 s.
    let s = take(1 << 22, 61.0);
    assert!(s.frames() > 4 * SCAN_FRAMES);
    let summary = PeakPyramid::build_from(s.data(), 2).unwrap();
    let mut r = Region::whole(&s);
    r.start = 1234;
    for pick in PICKS {
        r.channels = pick;
        let before = alloc::thread_count();
        let quick = r.peak_from(&s, &summary).unwrap();
        let read = r.peak(&s);
        let calls = alloc::thread_count() - before;
        assert_eq!(calls, 0, "{pick:?}: {calls} allocations");
        assert_eq!(quick.to_bits(), read.to_bits(), "{pick:?}");
        let low = if pick == ChannelPick::Left {
            0.75
        } else {
            0.35
        };
        assert!(quick > low, "{pick:?}: {quick}");
    }
}

/// The quickest of five runs of `f`.
fn best_of_5<T>(mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut best = Duration::MAX;
    let mut out = None;
    for _ in 0..5 {
        let t = Instant::now();
        let v = f();
        best = best.min(t.elapsed());
        out = Some(v);
    }
    (best, out.unwrap())
}

/// Run with `cargo test --release -p mpfree-sampler --test
/// normalize_no_alloc -- --ignored --nocapture`. The plan's test: on a
/// 25-minute take Normalize copies nothing and takes under 16 ms in
/// Stereo, Left and Right (Mono sum is timed too). Prints the times, and
/// Put on audio track's, which still copies the take as floats (and
/// converts it when its rate isn't the app's) on the screen thread.
#[test]
#[ignore]
fn print_normalize_and_put_on_audio_track_on_a_25_minute_take() {
    assert!(alloc::installed());
    let n = 25 * 60 * RATE as usize;
    let s = take(n, 1000.0);
    let summary = PeakPyramid::build_from(s.data(), 2).unwrap();
    let mut r = Region::whole(&s);
    println!(
        "25-minute 16-bit stereo take: {} frames, {} MB",
        s.frames(),
        s.data().bytes() / 1_000_000
    );
    for pick in PICKS {
        r.channels = pick;
        let before = alloc::thread_count();
        let (quick, peak) = best_of_5(|| r.peak_from(&s, &summary).unwrap());
        let calls = alloc::thread_count() - before;
        let (read, whole) = best_of_5(|| r.peak(&s));
        assert_eq!(calls, 0, "{pick:?}: {calls} allocations");
        assert_eq!(peak.to_bits(), whole.to_bits(), "{pick:?}");
        println!(
            "Normalize, {pick:?}: {:.3} ms from the summary (no copy); \
             {:.1} ms reading every frame",
            quick.as_secs_f64() * 1e3,
            read.as_secs_f64() * 1e3
        );
        if pick != ChannelPick::MonoSum {
            assert!(quick < Duration::from_millis(16), "{pick:?}: {quick:?}");
        }
    }
    // Put on audio track, as it is now: the take as floats, and converted
    // when the app runs at another rate.
    let ch = 2;
    let (copy, audio) = best_of_5(|| s.data().f32s(0..n * ch).unwrap().into_owned());
    println!(
        "Put on audio track, same rate: {:.0} ms (a {} MB float copy)",
        copy.as_secs_f64() * 1e3,
        audio.len() * 4 / 1_000_000
    );
    let t = Instant::now();
    let (x, _) = Source::new("take", RATE, 2, audio)
        .and_then(|x| x.at_rate(48_000))
        .unwrap();
    let converted = x.data().to_vec();
    println!(
        "Put on audio track, 44.1 kHz to 48 kHz: {:.0} ms more ({} MB out)",
        t.elapsed().as_secs_f64() * 1e3,
        converted.len() * 4 / 1_000_000
    );
}
