//! A take fitted to a new tempo: recorded at 240 BPM, played at 120 it
//! fills half its bar; stretched twice as long and swapped in, it fills
//! the bar again at the same pitch, and Undo goes back.

use mpfree_audiotracks::stretch::{fit_ratio, StretchJob};
use mpfree_audiotracks::{AudioTracks, Command as A};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050; // at 240 BPM
const BAR_TICKS: i64 = 3_840;
const START: u64 = 1_000;

fn tone(g: u64) -> f32 {
    (0.5 * (std::f64::consts::TAU * 441.0 * g as f64 / f64::from(RATE)).sin()) as f32
}

fn run(
    t: &mut Transport,
    a: &mut AudioTracks,
    b: &mut Block,
    from: u64,
    to: u64,
    live: bool,
    out: &mut Vec<f32>,
) {
    let mut f = from;
    let (mut inp, mut mix) = ([0.0f32; 512], [0.0f32; 512]);
    while f < to {
        let n = 256.min((to - f) as usize);
        for j in 0..n {
            let s = if live { tone(f + j as u64) } else { 0.0 };
            inp[2 * j] = s;
            inp[2 * j + 1] = s;
        }
        mix[..2 * n].fill(0.0);
        t.process(n as u32, b);
        let map = *t.tempo_map();
        a.process(b, &map, &inp[..2 * n], &mut mix[..2 * n]);
        out.extend_from_slice(&mix[..2 * n]);
        f += n as u64;
    }
}

/// RMS of the left channel over frames `from..to` of `out`.
fn rms(out: &[f32], from: u64, to: u64) -> f64 {
    let s: f64 = (from..to)
        .map(|g| f64::from(out[2 * g as usize]).powi(2))
        .sum();
    (s / (to - from) as f64).sqrt()
}

/// Zero crossings (rising) of the left channel over `from..to`.
fn crossings(out: &[f32], from: u64, to: u64) -> usize {
    (from + 1..to)
        .filter(|&g| out[2 * (g - 1) as usize] < 0.0 && out[2 * g as usize] >= 0.0)
        .count()
}

#[test]
fn a_take_fitted_to_a_slower_tempo_fills_its_bar_at_the_same_pitch() {
    let mut t = Transport::new(RATE, 240.0).unwrap();
    let mut a = AudioTracks::new(RATE, 20);
    let mut b = Block::new();
    let mut out = Vec::new();
    a.command(A::Arm(0, true));
    t.command(START, T::Record);
    t.command(START + BAR, T::Stop);
    let mut f = START + BAR + 500;
    run(&mut t, &mut a, &mut b, 0, f, true, &mut out);
    a.command(A::Arm(0, false));
    let take = a.take(0).unwrap();
    assert_eq!(take.bpm, 240.0);
    // Half the tempo: the bar is now two seconds long.
    t.command(f, T::SetTempo(120.0));
    let p = f + 10;
    t.command(p, T::PlayStart);
    run(
        &mut t,
        &mut a,
        &mut b,
        f,
        p + 2 * BAR + 100,
        false,
        &mut out,
    );
    f = p + 2 * BAR + 100;
    assert!(rms(&out, p + 1000, p + BAR - 1000) > 0.3);
    assert_eq!(rms(&out, p + BAR + 1000, p + 2 * BAR - 1000), 0.0);
    // Fit it, as a worker would.
    let ratio = fit_ratio(take.bpm, 120.0).unwrap();
    assert_eq!(ratio, 2.0);
    let mut audio = vec![0.0; take.frames * take.channels];
    assert_eq!(a.read_take(0, 0, &mut audio), Some(take.frames));
    let id = a.free_take_id().unwrap();
    let mut job = StretchJob::new(take, audio, RATE, ratio, 120.0, id).unwrap();
    while !job.run(1 << 14).finished {}
    let (file, swap) = job.finish().unwrap();
    assert!((file.frames as f64 - 2.0 * take.frames as f64).abs() < 2.0);
    assert!(a.load_take(file).is_ok());
    while a.loading() > 0 {
        run(&mut t, &mut a, &mut b, f, f + 256, false, &mut out);
        f += 256;
    }
    a.command(swap);
    t.command(f, T::Stop);
    run(&mut t, &mut a, &mut b, f, f + 10, false, &mut out);
    f += 10;
    assert_eq!(a.regions(0)[0].take, id);
    assert_eq!(a.regions(0)[0].end, BAR_TICKS);
    let q = f + 10;
    t.command(q, T::PlayStart);
    run(&mut t, &mut a, &mut b, f, q + 2 * BAR, false, &mut out);
    f = q + 2 * BAR;
    // Now the whole bar sounds, at 441 Hz still (about 441 rising zero
    // crossings a second).
    assert!(rms(&out, q + 1000, q + 2 * BAR - 1000) > 0.3);
    let c = crossings(&out, q + 2000, q + 2000 + BAR);
    assert!((436..=446).contains(&c), "{c}");
    // Undo plays the old take again.
    a.command(A::Undo);
    t.command(f, T::Stop);
    run(&mut t, &mut a, &mut b, f, f + 10, false, &mut out);
    assert_eq!(a.regions(0)[0].take, 0);
    // Swaps that make no sense are refused.
    let before = a.stats().refused;
    a.command(A::Swap {
        old: 0,
        new: 0,
        ratio: 2.0,
    });
    a.command(A::Swap {
        old: 0,
        new: 99,
        ratio: 2.0,
    });
    a.command(A::Swap {
        old: 0,
        new: id,
        ratio: f64::NAN,
    });
    run(&mut t, &mut a, &mut b, f + 10, f + 20, false, &mut out);
    assert_eq!(a.stats().refused, before + 3);
}
