//! Behaviour of the grain engine, checked against the signal.

use mpfree_grain::window::{value, TABLE};
use mpfree_grain::*;

const RATE: u32 = 48_000;

/// Renders `frames` in periods of `block`, starting at transport frame `t0`.
fn run<S: GrainSource>(v: &mut GrainVoice, src: &S, budget: &mut Budget, t0: u64, frames: usize, block: usize) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0; frames];
    let mut r = vec![0.0; frames];
    let mut at = 0;
    while at < frames {
        let n = block.min(frames - at);
        v.render(src, budget, t0 + at as u64, &mut l[at..at + n], &mut r[at..at + n]);
        at += n;
    }
    (l, r)
}

/// Start frames of bursts: the first non-zero frame after a silent one.
fn bursts(x: &[f32]) -> Vec<usize> {
    (0..x.len()).filter(|&i| x[i] != 0.0 && (i == 0 || x[i - 1] == 0.0)).collect()
}

fn ramp(frames: usize) -> Vec<f32> { (0..frames).map(|i| i as f32 * 1e-4).collect() }

#[test]
fn windows_are_zero_at_the_ends_and_shaped_right() {
    let shapes = [Window::Hann, Window::Tukey { taper: 0.3 }, Window::Trapezoid { ramp: 0.1 }, Window::Exponential { decay: 6.0 }];
    for s in shapes {
        let t = WindowTable::new(s);
        assert!(t.at(0.0).abs() < 1e-6, "{s:?} start");
        assert!(t.at(1.0).abs() < 1e-6, "{s:?} end");
        for i in 0..=200 {
            let x = i as f32 / 200.0;
            let w = t.at(x);
            assert!((0.0..=1.0 + 1e-6).contains(&w));
            // The table matches the formula to within interpolation error,
            // which is largest at the sharp corners (the 2 % attack).
            assert!((w - value(s, x)).abs() < 0.03, "{s:?} at {x}");
        }
    }
    let hann = WindowTable::new(Window::Hann);
    assert!((hann.at(0.5) - 1.0).abs() < 1e-6);
    for i in 0..TABLE {
        let x = i as f32 / TABLE as f32;
        assert!((hann.at(x) - hann.at(1.0 - x)).abs() < 1e-5, "symmetric");
    }
    // Tukey with a full taper is Hann; a short taper is flat in the middle.
    for i in 0..=100 {
        let x = i as f32 / 100.0;
        assert!((value(Window::Tukey { taper: 1.0 }, x) - value(Window::Hann, x)).abs() < 1e-5);
    }
    assert_eq!(value(Window::Tukey { taper: 0.2 }, 0.5), 1.0);
    assert_eq!(value(Window::Trapezoid { ramp: 0.25 }, 0.125), 0.5);
    // Exponential peaks early.
    let e = WindowTable::new(Window::Exponential { decay: 6.0 });
    assert!(value(Window::Exponential { decay: 6.0 }, 0.02) > 0.999 && e.at(0.03) > 0.9 && e.at(0.5) < 0.1);
}

#[test]
fn one_grain_is_the_source_times_the_window() {
    let data = ramp(48_000);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut v = GrainVoice::new(RATE, 1);
    v.set_params(Params { position: 0.25, size_ms: 10.0, density: 1.0, ..Params::default() });
    v.start();
    let mut b = Budget::new(256);
    let (l, r) = run(&mut v, &src, &mut b, 0, 2000, 128);
    assert_eq!(v.stats().spawned, 1);
    let size = 480;
    let w = WindowTable::new(Window::Hann);
    for k in 0..size {
        let want = (12_000 + k) as f32 * 1e-4 * w.at(k as f32 / (size - 1) as f32);
        assert!((l[k] - want).abs() < 1e-5, "frame {k}: {} vs {want}", l[k]);
        assert_eq!(l[k], r[k]);
    }
    assert!(l[size..].iter().all(|&x| x == 0.0));
    assert!(!v.is_active() || v.is_spawning());
}

#[test]
fn pitch_and_reverse_change_the_read_speed() {
    let data = ramp(48_000);
    let src = Clip::new(&data, 1, RATE).unwrap();
    let w = WindowTable::new(Window::Hann);
    for (pitch, reverse, inc) in [(12.0, 0.0, 2.0f64), (-12.0, 0.0, 0.5), (0.0, 1.0, -1.0), (7.0, 1.0, -(7.0f64 / 12.0).exp2())] {
        let mut v = GrainVoice::new(RATE, 3);
        v.set_params(Params { position: 0.5, size_ms: 10.0, density: 1.0, pitch, reverse, ..Params::default() });
        v.start();
        let (l, _) = run(&mut v, &src, &mut Budget::new(8), 0, 600, 64);
        for k in 0..480 {
            let pos = 24_000.0 + k as f64 * inc;
            let want = (pos * 1e-4) as f32 * w.at(k as f32 / 479.0);
            assert!((l[k] - want).abs() < 2e-5, "pitch {pitch} rev {reverse} frame {k}: {} vs {want}", l[k]);
        }
    }
}

#[test]
fn the_source_rate_sets_the_pitch() {
    // A 24 kHz source played on a 48 kHz output reads half a frame per frame.
    let data = ramp(24_000);
    let src = Clip::new(&data, 1, 24_000).unwrap();
    let mut v = GrainVoice::new(RATE, 3);
    v.set_params(Params { position: 0.5, size_ms: 10.0, density: 1.0, ..Params::default() });
    v.start();
    let (l, _) = run(&mut v, &src, &mut Budget::new(8), 0, 480, 480);
    let w = WindowTable::new(Window::Hann);
    let want = (12_000.0 + 240.0 * 0.5) * 1e-4 * w.at(0.5 * 480.0 / 479.0);
    assert!((l[240] - want).abs() < 2e-5);
}

#[test]
fn same_seed_same_cloud_and_another_seed_differs() {
    let data: Vec<f32> = (0..96_000).map(|i| ((i as f32) * 0.01).sin() * 0.5).collect();
    let src = Clip::new(&data, 2, RATE).unwrap();
    let p = Params { spray: 0.8, size_ms: 40.0, density: 120.0, pitch_spray: 7.0, reverse: 0.3, spread: 1.0, scan: 0.5, ..Params::default() };
    let render = |seed| {
        let mut v = GrainVoice::new(RATE, seed);
        v.set_params(p);
        v.start();
        run(&mut v, &src, &mut Budget::new(256), 0, 48_000, 128)
    };
    let a = render(42);
    let b = render(42);
    assert!(a.0.iter().zip(&b.0).all(|(x, y)| x.to_bits() == y.to_bits()));
    assert!(a.1.iter().zip(&b.1).all(|(x, y)| x.to_bits() == y.to_bits()));
    assert_ne!(a.0, render(43).0);
    // Spread pans grains apart.
    assert_ne!(a.0, a.1);
}

#[test]
fn period_size_does_not_change_the_output() {
    let data: Vec<f32> = (0..48_000).map(|i| ((i * 31) % 97) as f32 / 97.0 - 0.5).collect();
    let src = Clip::new(&data, 1, RATE).unwrap();
    let p = Params { spray: 0.5, size_ms: 25.0, density: 90.0, pitch_spray: 3.0, scan: 1.0, ..Params::default() };
    let render = |block| {
        let mut v = GrainVoice::new(RATE, 9);
        v.set_params(p);
        v.start();
        run(&mut v, &src, &mut Budget::new(256), 0, 24_000, block).0
    };
    let a = render(128);
    for block in [1, 64, 333, 4096] {
        let b = render(block);
        let worst = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
        assert!(worst < 1e-4, "block {block}: {worst}");
    }
}

#[test]
fn free_running_density_makes_the_right_number_of_grains() {
    let data = vec![0.5; 48_000];
    let src = Clip::new(&data, 1, RATE).unwrap();
    for density in [5.0, 50.0, 200.0] {
        let mut v = GrainVoice::new(RATE, 1);
        v.set_params(Params { size_ms: 5.0, density, ..Params::default() });
        v.start();
        run(&mut v, &src, &mut Budget::new(256), 0, 48_000, 128);
        let want = density as u64;
        assert!(v.stats().spawned.abs_diff(want) <= 1, "density {density}: {}", v.stats().spawned);
    }
}

#[test]
fn synced_grains_land_on_the_transport_grid() {
    let data = vec![0.5; 48_000];
    let src = Clip::new(&data, 1, RATE).unwrap();
    // 120 BPM at 48 kHz: 24000 frames a beat; 16 grains a beat = every 1500 frames.
    let sync = TempoSync { frames_per_beat: 24_000.0, grains_per_beat: Some(16.0), size_beats: Some(1.0 / 64.0) };
    let mut v = GrainVoice::new(RATE, 1);
    v.set_params(Params { sync: Some(sync), ..Params::default() });
    v.start();
    let t0 = 1_000_100;
    let (l, _) = run(&mut v, &src, &mut Budget::new(256), t0, 30_000, 128);
    // Hann starts at 0, so each burst is audible from its second frame.
    let starts: Vec<u64> = bursts(&l).iter().map(|&i| t0 + i as u64 - 1).collect();
    assert_eq!(starts.len(), 20);
    for s in &starts {
        assert_eq!(s % 1500, 0, "grain at {s}");
    }
    // Size synced to a 64th note: 375 frames.
    let first = bursts(&l)[0];
    assert!(l[first..first + 373].iter().all(|&x| x > 0.0));
    assert_eq!(l[first + 374], 0.0);
}

#[test]
fn the_budget_caps_grains_across_voices() {
    let data = vec![0.1; 48_000];
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut a = GrainVoice::new(RATE, 1);
    let mut b = GrainVoice::new(RATE, 2);
    let p = Params { size_ms: 200.0, density: 400.0, ..Params::default() };
    a.set_params(p);
    b.set_params(p);
    a.start();
    b.start();
    let mut budget = Budget::new(40);
    let mut l = vec![0.0; 128];
    let mut r = vec![0.0; 128];
    for _ in 0..200 {
        a.render(&src, &mut budget, 0, &mut l, &mut r);
        b.render(&src, &mut budget, 0, &mut l, &mut r);
        assert!(a.stats().active + b.stats().active <= 40);
        assert_eq!(budget.live(), a.stats().active + b.stats().active);
    }
    a.kill(&mut budget);
    assert_eq!(budget.live(), b.stats().active);
    b.stop();
    for _ in 0..100 {
        b.render(&src, &mut budget, 0, &mut l, &mut r);
    }
    assert_eq!(budget.live(), 0);
    assert!(budget.skipped > 0);
    assert_eq!(a.stats().skipped + b.stats().skipped, budget.skipped);
    // A voice on its own never exceeds its array.
    let mut c = GrainVoice::new(RATE, 3);
    c.set_params(Params { size_ms: 1000.0, density: 1000.0, ..Params::default() });
    c.start();
    run(&mut c, &src, &mut Budget::new(10_000), 0, 24_000, 128);
    assert_eq!(c.stats().peak_active, MAX_GRAINS);
    assert!(c.stats().skipped > 0);
}

#[test]
fn grains_stay_inside_the_region() {
    // 1.0 inside the region, 1000 outside: any read outside would show.
    let mut data = vec![1000.0f32; 20_000];
    for x in &mut data[5_000..9_000] {
        *x = 1.0;
    }
    let src = Clip::new(&data, 1, RATE).unwrap().with_range(5_000, 9_000);
    let mut v = GrainVoice::new(RATE, 5);
    // Grains far longer than the region, sprayed everywhere, pitched up and backwards.
    v.set_params(Params { spray: 1.0, size_ms: 300.0, density: 30.0, pitch_spray: 12.0, reverse: 0.5, position: 0.9, scan: 2.0, ..Params::default() });
    v.start();
    let (l, _) = run(&mut v, &src, &mut Budget::new(256), 0, 48_000, 128);
    assert!(v.stats().spawned > 20);
    let peak = l.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    assert!(peak < 2.0 * MAX_GRAINS as f32, "read outside the region: {peak}");
    assert!(peak > 0.1);
    // With one sparse grain at a time, each burst is a whole window: nothing cut short.
    let mut v = GrainVoice::new(RATE, 6);
    v.set_params(Params { spray: 1.0, size_ms: 20.0, density: 10.0, reverse: 0.5, ..Params::default() });
    v.start();
    let (l, _) = run(&mut v, &src, &mut Budget::new(256), 0, 48_000, 128);
    let w = WindowTable::new(Window::Hann);
    let full: f32 = (0..960).map(|k| w.at(k as f32 / 959.0)).sum();
    let got: f32 = l.iter().sum();
    let n = v.stats().spawned as f32;
    assert!((got - n * full).abs() < 0.01 * n * full, "{got} vs {}", n * full);
}

#[test]
fn scan_moves_the_playhead_at_the_source_speed() {
    let data = vec![0.0; 48_000];
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut v = GrainVoice::new(RATE, 1);
    v.set_params(Params { position: 0.1, scan: 1.0, ..Params::default() });
    v.start();
    run(&mut v, &src, &mut Budget::new(256), 0, 12_000, 128);
    assert!((v.playhead() - 0.35).abs() < 1e-4, "{}", v.playhead());
    v.set_params(Params { position: 0.1, scan: -0.5, ..Params::default() });
    run(&mut v, &src, &mut Budget::new(256), 0, 24_000, 128);
    assert!((v.playhead() - 0.1).abs() < 1e-4, "{}", v.playhead());
    // It wraps round the span.
    v.set_params(Params { position: 0.1, scan: -1.0, ..Params::default() });
    run(&mut v, &src, &mut Budget::new(256), 0, 9_600, 128);
    assert!((v.playhead() - 0.9).abs() < 1e-4, "{}", v.playhead());
}

#[test]
fn stop_lets_grains_finish_then_goes_quiet() {
    let data = vec![0.5; 48_000];
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut v = GrainVoice::new(RATE, 1);
    v.set_params(Params { size_ms: 50.0, density: 100.0, ..Params::default() });
    v.start();
    let mut b = Budget::new(256);
    run(&mut v, &src, &mut b, 0, 4800, 128);
    let playing = v.stats().active;
    assert!(playing >= 4);
    v.stop();
    let (l, _) = run(&mut v, &src, &mut b, 0, 4800, 128);
    assert!(l[0] > 0.0, "grains keep playing after stop");
    assert!(!v.is_active());
    assert!(l[2400..].iter().all(|&x| x == 0.0));
    let mut views = [GrainView::default(); MAX_GRAINS];
    assert_eq!(v.grains(&mut views), 0);
}

#[test]
fn the_ui_can_see_the_cloud() {
    let data = vec![0.5; 48_000];
    let src = Clip::new(&data, 1, RATE).unwrap();
    let mut v = GrainVoice::new(RATE, 1);
    v.set_params(Params { position: 0.6, size_ms: 100.0, density: 50.0, ..Params::default() });
    v.start();
    run(&mut v, &src, &mut Budget::new(256), 0, 4800, 128);
    let mut views = [GrainView::default(); MAX_GRAINS];
    let n = v.grains(&mut views);
    assert_eq!(n, v.stats().active);
    assert!(n >= 4);
    for g in &views[..n] {
        assert!((0.6..=0.7).contains(&g.position), "{g:?}");
        assert!(g.level > 0.0 && !g.reverse);
    }
}

#[test]
fn scale_quantise_picks_the_nearest_allowed_note() {
    let major = 0b1010_1011_0101;
    assert_eq!(quantise(1.0, major), 0.0); // tie goes down
    assert_eq!(quantise(1.4, major), 2.0);
    assert_eq!(quantise(6.2, major), 7.0);
    assert_eq!(quantise(5.8, major), 5.0);
    assert_eq!(quantise(-1.2, major), -1.0); // B below
    assert_eq!(quantise(13.0, major), 12.0);
    assert_eq!(quantise(3.3, 0), 3.3);
    assert_eq!(quantise(4.0, 1), 0.0); // octaves only
    assert_eq!(quantise(7.0, 1), 12.0);
}

#[test]
fn live_buffer_keeps_the_recent_past_and_freezes() {
    let mut lb = LiveBuffer::new(2, RATE, 1000).unwrap();
    assert_eq!(lb.span(), (0, 0));
    let frames: Vec<f32> = (0..1500).flat_map(|i| [i as f32, -(i as f32)]).collect();
    lb.write(&frames[..600]); // 300 frames
    assert_eq!(lb.span(), (0, 300));
    lb.write(&frames[600..]); // 1500 frames in total
    assert_eq!(lb.written(), 1500);
    // 1000 held (500..1500), the oldest 250 reserved while writing.
    assert_eq!(lb.span(), (750, 1500));
    assert_eq!(lb.sample(0, 499), 0.0);
    assert_eq!(lb.sample(0, 500), 500.0);
    assert_eq!(lb.sample(1, 1499), -1499.0);
    assert_eq!(lb.sample(0, 1500), 0.0);
    lb.set_frozen(true);
    assert_eq!(lb.span(), (500, 1500));
    lb.write(&[9.0, 9.0]);
    lb.write_planar(&[9.0], &[9.0]);
    assert_eq!(lb.written(), 1500);
    assert_eq!(lb.sample(0, 1000), 1000.0);
    lb.set_frozen(false);
    lb.write_planar(&[7.0, 8.0], &[-7.0, -8.0]);
    assert_eq!((lb.sample(0, 1500), lb.sample(1, 1501)), (7.0, -8.0));
    // Mono stores the average of planar input.
    let mut m = LiveBuffer::new(1, RATE, 10).unwrap();
    m.write_planar(&[1.0], &[0.0]);
    assert_eq!(m.sample(5, 0), 0.5);
}

#[test]
fn granular_effect_on_a_live_signal_survives_freeze() {
    // A 440 Hz sine arrives in periods; the effect reads it back as grains.
    let mut lb = LiveBuffer::new(1, RATE, RATE as usize * 4).unwrap();
    let mut v = GrainVoice::new(RATE, 11);
    v.set_params(Params { position: 0.8, spray: 0.3, size_ms: 60.0, density: 40.0, ..Params::default() });
    v.start();
    let mut budget = Budget::new(256);
    let mut out = vec![0.0f32; 128];
    let mut out_r = vec![0.0f32; 128];
    let mut t = 0u64;
    let mut energy = [0.0f64; 3];
    for period in 0..1500 {
        let sig: Vec<f32> = (0..128).map(|k| ((t + k) as f32 * 440.0 * std::f32::consts::TAU / RATE as f32).sin()).collect();
        let phase = match period { 0..=499 => 0, 500..=999 => 1, _ => 2 };
        if period == 500 { lb.set_frozen(true); }
        if period == 1000 { lb.set_frozen(false); }
        // Silence the input while frozen: the effect must keep sounding anyway.
        if phase == 1 { lb.write(&vec![0.0; 128]); } else { lb.write(&sig); }
        out.fill(0.0);
        out_r.fill(0.0);
        v.render(&lb, &mut budget, t, &mut out, &mut out_r);
        if period % 500 > 100 {
            energy[phase] += out.iter().map(|&x| f64::from(x * x)).sum::<f64>();
        }
        t += 128;
    }
    assert!(energy[0] > 1.0, "{energy:?}");
    // Frozen: the input is silent but the cloud plays what was held.
    assert!(energy[1] > 0.5 * energy[0], "{energy:?}");
    // Unfrozen, the buffer fills with live sine again.
    assert!(energy[2] > 0.5 * energy[0], "{energy:?}");
    assert!(out.iter().all(|x| x.abs() < 4.0));
}
