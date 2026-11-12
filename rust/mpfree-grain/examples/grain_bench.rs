//! Worst-case cost of the grain engine on this machine: one voice with all
//! 64 grains playing, on a stereo clip and on a live buffer.
//! Run: cargo run --release -p mpfree-grain --example grain_bench
use std::time::Instant;

use mpfree_grain::*;

fn main() {
    let rate = 44_100u32;
    let data: Vec<f32> = (0..rate as usize * 20)
        .map(|i| ((i * 7919) % 2000) as f32 / 1000.0 - 1.0)
        .collect();
    let clip = Clip::new(&data, 2, rate).unwrap();
    let mut live = LiveBuffer::new(2, rate, rate as usize * 8).unwrap();
    live.write(&data[..rate as usize * 16]);
    let p = Params {
        spray: 0.5,
        size_ms: 1000.0,
        density: 400.0,
        pitch_spray: 7.0,
        reverse: 0.3,
        spread: 1.0,
        ..Params::default()
    };
    for (name, is_live) in [("clip", false), ("live", true)] {
        let mut v = GrainVoice::new(rate, 1);
        v.set_params(p);
        v.start();
        let mut budget = Budget::new(1000);
        let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
        let periods = 20_000;
        let mut grain_frames = 0u64;
        let t = Instant::now();
        for i in 0..periods {
            if is_live {
                v.render(&live, &mut budget, i * 128, &mut l, &mut r)
            } else {
                v.render(&clip, &mut budget, i * 128, &mut l, &mut r)
            }
            grain_frames += v.stats().active as u64 * 128;
        }
        let secs = t.elapsed().as_secs_f64();
        let audio = periods as f64 * 128.0 / f64::from(rate);
        println!("{name}: {:.1} M grain-frames/s, average {:.1} grains, {:.2} % of one core per voice at full load, per 128-frame period {:.1} us",
            grain_frames as f64 / secs / 1e6, grain_frames as f64 / (periods as f64 * 128.0), 100.0 * secs / audio, secs / periods as f64 * 1e6);
        std::hint::black_box(&l);
    }
}
