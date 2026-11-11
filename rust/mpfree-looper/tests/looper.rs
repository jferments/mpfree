//! The looper against a real transport: the first pass on the bar, sample
//! for sample playback, latency, the loop point, overdub layers with undo
//! and redo, fades, jumps, stop, count-in, free length, transport loops,
//! full memory and the same output for any period size.
//!
//! 22.05 kHz at 240 BPM in 4/4 makes a bar exactly 22 050 frames (1 s).

use mpfree_looper::{Command as L, Looper, Phase, FADE};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: usize = 22_050;
const BAR_TICKS: i64 = 3_840;
/// The transport starts playing here, so bar lines are at START + k * BAR.
const START: u64 = 1_000;

struct Rig {
    t: Transport,
    l: Looper,
    b: Block,
    period: usize,
    frame: u64,
    input: Vec<f32>,
    mix: Vec<f32>,
    /// Every frame's output so far, interleaved stereo.
    out: Vec<f32>,
}

impl Rig {
    fn new(seconds: u32, period: usize) -> Rig {
        Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            l: Looper::new(RATE, seconds),
            b: Block::new(),
            period,
            frame: 0,
            input: vec![0.0; 2 * period],
            mix: vec![0.0; 2 * period],
            out: Vec::new(),
        }
    }

    fn at(&mut self, frame: u64, c: T) {
        assert!(self.t.command(frame, c));
    }

    fn cmd(&mut self, c: L) {
        assert!(self.l.command(c));
    }

    /// Run up to absolute frame `to`, feeding `input(frame)`.
    fn run(&mut self, to: u64, input: &dyn Fn(u64) -> [f32; 2]) {
        while self.frame < to {
            let n = (self.period as u64).min(to - self.frame) as usize;
            for i in 0..n {
                let x = input(self.frame + i as u64);
                self.input[2 * i] = x[0];
                self.input[2 * i + 1] = x[1];
            }
            self.mix[..2 * n].fill(0.0);
            assert!(self.t.process(n as u32, &mut self.b));
            self.l.process(&self.b, &self.input[..2 * n], &mut self.mix[..2 * n]);
            self.out.extend_from_slice(&self.mix[..2 * n]);
            self.frame += n as u64;
        }
    }

    fn left(&self, f: u64) -> f32 {
        self.out[2 * f as usize]
    }

    fn right(&self, f: u64) -> f32 {
        self.out[2 * f as usize + 1]
    }

    /// The biggest jump between neighbouring output frames in `a..b`.
    fn max_step(&self, a: u64, b: u64) -> f32 {
        (a + 1..b).map(|f| (self.left(f) - self.left(f - 1)).abs().max((self.right(f) - self.right(f - 1)).abs())).fold(0.0, f32::max)
    }
}

/// Deterministic noise, different on each channel.
fn noise(f: u64) -> [f32; 2] {
    let h = |s: u64| {
        let mut x = f.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(s);
        x ^= x >> 29;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 32;
        (x % 10_000) as f32 / 10_000.0 - 0.5
    };
    [h(1), h(2)]
}

fn sine(f: u64) -> [f32; 2] {
    let p = f as f64 * std::f64::consts::TAU / 1000.37;
    [(0.5 * p.sin()) as f32, (0.5 * p.cos()) as f32]
}

fn dc(v: f32) -> impl Fn(u64) -> [f32; 2] {
    move |_| [v, -v]
}

/// A one-bar loop of `input`, recorded from START; returns after the
/// first pass ends.
fn record_one_bar(r: &mut Rig, input: &dyn Fn(u64) -> [f32; 2]) {
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    r.at(START, T::Play);
    r.run(START + BAR as u64 / 2, input);
    assert_eq!(r.l.status().phase, Phase::Recording);
    r.run(START + BAR as u64 + 1, input);
    let s = r.l.status();
    assert_eq!(s.phase, Phase::Playing);
    assert_eq!(s.frames, BAR);
    assert_eq!(s.ticks, BAR_TICKS);
    assert_eq!(s.origin, 0);
    assert_eq!(s.layers, 1);
}

#[test]
fn the_first_pass_starts_on_the_bar_and_plays_back_sample_for_sample() {
    let mut r = Rig::new(10, 128);
    record_one_bar(&mut r, &noise);
    // Silent while armed and recording.
    assert!(r.out[..2 * (START as usize + BAR)].iter().all(|&v| v == 0.0));
    r.run(START + 2 * BAR as u64, &|_| [0.0, 0.0]);
    let len = BAR as u64;
    for f in START + len + FADE as u64..START + 2 * len - FADE as u64 {
        let want = noise(f - len);
        assert!((r.left(f) - want[0]).abs() < 1e-4, "frame {f}: {} vs {}", r.left(f), want[0]);
        assert!((r.right(f) - want[1]).abs() < 1e-4, "frame {f}");
    }
}

#[test]
fn input_latency_is_made_up_for() {
    const LAT: u64 = 300;
    let mut r = Rig::new(10, 128);
    r.cmd(L::SetLatency(LAT as u32));
    // The player plays `noise` in time; it reaches the input LAT late.
    let late = |f: u64| if f < LAT { [0.0, 0.0] } else { noise(f - LAT) };
    record_one_bar(&mut r, &late);
    r.run(START + 2 * BAR as u64 + LAT, &late);
    let len = BAR as u64;
    for f in START + len + FADE as u64..START + 2 * len - FADE as u64 {
        let want = noise(f - len);
        assert!((r.left(f) - want[0]).abs() < 1e-4, "frame {f}");
    }
    assert_eq!(r.l.status().latency, LAT as u32);
}

#[test]
fn the_loop_point_does_not_click() {
    let mut r = Rig::new(10, 128);
    record_one_bar(&mut r, &sine);
    // Through the loop point at START + 2 bars and on.
    r.run(START + 3 * BAR as u64, &|_| [0.0, 0.0]);
    let seam = START + 2 * BAR as u64;
    // The sine moves at most 0.5 * 2 pi / 1000.37 = 0.0031 per frame. A
    // raw splice here would jump about 0.13; the crossfade keeps it smooth.
    let step = r.max_step(START + BAR as u64 + FADE as u64 + 1, START + 3 * BAR as u64 - 1);
    assert!(step < 0.008, "biggest step {step}");
    assert!((r.left(seam) - sine(START)[0]).abs() < 1e-3);
}

/// Loop positions are frames since START, modulo the bar.
fn pos_frame(after: u64, pos: u64) -> u64 {
    let base = START + ((after - START) / BAR as u64 + 1) * BAR as u64;
    base + pos
}

#[test]
fn overdubs_stack_and_undo_and_redo_fade() {
    let mut r = Rig::new(10, 128);
    let a = dc(0.25);
    record_one_bar(&mut r, &a);
    // Overdub 0.125 from loop position 2000 for one loop and 3000 frames.
    let f1 = START + BAR as u64 + 2_000;
    let f2 = f1 + BAR as u64 + 3_000;
    r.run(f1, &a);
    r.cmd(L::Overdub);
    r.run(f2, &dc(0.125));
    assert_eq!(r.l.status().phase, Phase::Overdubbing);
    r.cmd(L::Overdub);
    let z = dc(0.0);
    r.run(f2 + 2 * FADE as u64, &z);
    let s = r.l.status();
    assert_eq!((s.phase, s.layers, s.redo), (Phase::Playing, 2, 0));
    let layer = r.l.layer(1).unwrap();
    for q in (2_000 + FADE + 1)..5_000 {
        assert!((layer[2 * q] - 0.25).abs() < 1e-5, "position {q}: {}", layer[2 * q]);
    }
    for q in (5_000 + FADE + 1..BAR).chain(0..2_000) {
        assert!((layer[2 * q] - 0.125).abs() < 1e-5, "position {q}: {}", layer[2 * q]);
    }
    // Played back: the first pass plus the layer.
    let g = pos_frame(r.frame, 0);
    r.run(g + BAR as u64, &z);
    assert!((r.left(g + 3_000) - 0.5).abs() < 1e-4);
    assert!((r.left(g + 10_000) - 0.375).abs() < 1e-4);
    assert!((r.right(g + 10_000) + 0.375).abs() < 1e-4);

    // Undo fades the layer out; Redo fades it back in.
    r.cmd(L::Undo);
    let u = r.frame;
    r.run(u + 4 * FADE as u64, &z);
    let s = r.l.status();
    assert_eq!((s.layers, s.redo), (1, 1));
    assert!((r.left(r.frame - 1) - 0.25).abs() < 1e-4);
    assert!(r.max_step(u, r.frame) < 0.125 / FADE as f32 * 1.5 + 1e-4, "undo clicked");
    r.cmd(L::Redo);
    let u = r.frame;
    r.run(u + 4 * FADE as u64, &z);
    let s = r.l.status();
    assert_eq!((s.layers, s.redo), (2, 0));
    assert!(r.max_step(u, r.frame) < 0.25 / FADE as f32 * 1.5 + 1e-4, "redo clicked");

    // Undo, then a new overdub: Redo has nothing left to bring back.
    r.cmd(L::Undo);
    r.cmd(L::Overdub);
    r.run(r.frame + 4 * FADE as u64, &z);
    let s = r.l.status();
    assert_eq!((s.phase, s.layers, s.redo), (Phase::Overdubbing, 2, 0));
    r.cmd(L::Redo);
    r.run(r.frame + 128, &z);
    assert_eq!(r.l.status().layers, 2);
}

#[test]
fn undo_pressed_twice_removes_two_layers() {
    let mut r = Rig::new(10, 128);
    let z = dc(0.0);
    record_one_bar(&mut r, &dc(0.25));
    for _ in 0..2 {
        r.cmd(L::Overdub);
        r.run(r.frame + 3_000, &dc(0.1));
        r.cmd(L::Overdub);
        r.run(r.frame + 2 * FADE as u64, &z);
    }
    assert_eq!(r.l.status().layers, 3);
    // Both in the same period: the second waits for the first's fade.
    r.cmd(L::Undo);
    r.cmd(L::Undo);
    r.run(r.frame + 6 * FADE as u64, &z);
    let s = r.l.status();
    assert_eq!((s.layers, s.redo), (1, 2));
}

#[test]
fn punches_fade_and_old_audio_in_a_reused_slot_stays_silent() {
    let mut r = Rig::new(10, 128);
    let z = dc(0.0);
    record_one_bar(&mut r, &dc(0.25));
    // A loud full-loop overdub, then undone: its slot holds stale audio.
    r.cmd(L::Overdub);
    r.run(r.frame + BAR as u64 + 500, &dc(0.5));
    r.cmd(L::Overdub);
    r.run(r.frame + 2 * FADE as u64, &z);
    r.cmd(L::Undo);
    r.run(r.frame + 2 * FADE as u64, &z);
    assert_eq!(r.l.status().layers, 1);
    // A short overdub in the same slot.
    let p = pos_frame(r.frame, 4_000);
    r.run(p, &z);
    r.cmd(L::Overdub);
    r.run(p + 5_000, &dc(0.125));
    r.cmd(L::Overdub);
    // The player plays on while the tail fades out.
    r.run(p + 5_000 + 2 * FADE as u64, &dc(0.125));
    let layer = r.l.layer(1).unwrap();
    // Fade in and out over FADE frames: no step bigger than one fade step.
    let mut biggest = 0.0f32;
    for q in 4_001..4_000 + 5_000 + FADE {
        biggest = biggest.max((layer[2 * q] - layer[2 * q - 2]).abs());
    }
    assert!(biggest <= 0.125 / FADE as f32 + 1e-6, "step {biggest}");
    assert!(layer[2 * 4_000].abs() < 0.001);
    // A whole pass: the base everywhere, plus 0.125 only where written.
    let g = pos_frame(r.frame, 0);
    r.run(g + BAR as u64, &z);
    assert!((r.left(g + 2_000) - 0.25).abs() < 1e-4, "stale audio heard");
    assert!((r.left(g + 6_000) - 0.375).abs() < 1e-4);
    assert!((r.left(g + 15_000) - 0.25).abs() < 1e-4, "stale audio heard");
    assert!(r.max_step(g, g + BAR as u64) < 0.125 / FADE as f32 * 1.5 + 1e-4);
}

#[test]
fn stop_fades_out_and_play_fades_back_in() {
    let mut r = Rig::new(10, 128);
    record_one_bar(&mut r, &dc(0.25));
    let z = dc(0.0);
    let s = START + BAR as u64 + 7_777;
    r.at(s, T::Stop);
    r.run(s + 2_000, &z);
    assert!((r.left(s - 1) - 0.25).abs() < 1e-4);
    assert!(r.max_step(s - 10, s + 2_000) <= 0.25 / FADE as f32 + 1e-5);
    assert!((s + FADE as u64..s + 2_000).all(|f| r.left(f) == 0.0));
    let p = r.frame + 100;
    r.at(p, T::Play);
    r.run(p + 2_000, &z);
    assert!(r.max_step(p - 10, p + 2_000) <= 0.25 / FADE as f32 + 1e-5);
    assert!((r.left(p + 1_000) - 0.25).abs() < 1e-4);
}

#[test]
fn a_locate_crossfades_instead_of_clicking() {
    let mut r = Rig::new(10, 128);
    record_one_bar(&mut r, &sine);
    let z = dc(0.0);
    // From loop position 9000 to 8498.4 (tick 1480): half a sine period
    // away, the worst case for a splice (a jump of about 1.0).
    let l = START + BAR as u64 + 9_000;
    r.at(l, T::Locate(1_480));
    r.run(l + 3_000, &z);
    let step = r.max_step(l - 500, l + 3_000);
    assert!(step < 0.01, "biggest step {step}");
    // Then the new place, sample for sample (read between frames).
    let q = 1_480.0 * BAR as f64 / BAR_TICKS as f64;
    let at = (START as f64 + q + 1_000.0) * std::f64::consts::TAU / 1000.37;
    let want = (0.5 * at.sin()) as f32;
    let f = l + 1_000;
    assert!((r.left(f) - want).abs() < 1e-4, "{} vs {want}", r.left(f));
}

#[test]
fn nothing_plays_during_a_count_in() {
    let mut r = Rig::new(10, 128);
    record_one_bar(&mut r, &dc(0.25));
    let z = dc(0.0);
    let s = r.frame + 1_000;
    r.at(s, T::Stop);
    r.at(s + 2_000, T::SetCountIn(1));
    r.at(s + 2_000, T::Record);
    r.run(s + 2_000 + BAR as u64 - 10, &z);
    assert!((s + FADE as u64..r.frame).all(|f| r.left(f) == 0.0));
    r.run(s + 2_000 + 2 * BAR as u64, &z);
    assert!((r.left(r.frame - 1) - 0.25).abs() < 1e-4);
}

#[test]
fn free_length_ends_on_the_bar_after_record() {
    let mut r = Rig::new(10, 128);
    r.cmd(L::SetBars(None));
    r.cmd(L::Record);
    r.at(START, T::Play);
    r.run(START + BAR as u64 + BAR as u64 / 2, &noise);
    assert_eq!(r.l.status().phase, Phase::Recording);
    r.cmd(L::Record);
    r.run(START + BAR as u64 + BAR as u64 / 2 + 128, &noise);
    assert!(r.l.status().ending);
    r.run(START + 2 * BAR as u64 + 1, &noise);
    let s = r.l.status();
    assert_eq!((s.phase, s.frames, s.ticks), (Phase::Playing, 2 * BAR, 2 * BAR_TICKS));
}

#[test]
fn a_transport_loop_ends_the_first_pass_and_the_loop_follows_it() {
    let mut r = Rig::new(10, 128);
    r.at(0, T::SetLoop(Some((0, BAR_TICKS))));
    record_one_bar(&mut r, &noise);
    r.run(START + 3 * BAR as u64, &dc(0.0));
    let len = BAR as u64;
    for f in (START + 2 * len + FADE as u64..START + 3 * len - FADE as u64).step_by(7) {
        let want = noise(START + (f - START) % len);
        assert!((r.left(f) - want[0]).abs() < 1e-4, "frame {f}");
    }
}

#[test]
fn stopping_or_jumping_during_the_first_pass_throws_it_away() {
    let mut r = Rig::new(10, 128);
    r.cmd(L::Record);
    r.at(START, T::Play);
    r.at(START + 5_000, T::Stop);
    r.run(START + 6_000, &noise);
    assert_eq!(r.l.status().phase, Phase::Empty);
    assert_eq!(r.l.stats().aborted, 1);
    // Back to bar 1, then a locate in the middle of the pass.
    r.at(START + 6_500, T::StopAndRewind);
    r.cmd(L::Record);
    r.at(START + 7_000, T::Play);
    r.at(START + 9_000, T::Locate(5_000));
    r.run(START + 8_000, &noise);
    assert_eq!(r.l.status().phase, Phase::Recording);
    r.run(START + 10_000, &noise);
    assert_eq!(r.l.status().phase, Phase::Empty);
    assert_eq!(r.l.stats().aborted, 2);
}

#[test]
fn overdubs_are_refused_when_the_memory_is_full() {
    // 3 seconds: the first pass and two overdubs of a one-second loop.
    let mut r = Rig::new(3, 128);
    let z = dc(0.0);
    record_one_bar(&mut r, &dc(0.25));
    for _ in 0..3 {
        r.cmd(L::Overdub);
        r.run(r.frame + 2_000, &dc(0.1));
        r.cmd(L::Overdub);
        r.run(r.frame + 2 * FADE as u64, &z);
    }
    let s = r.l.status();
    assert_eq!((s.layers, s.phase), (3, Phase::Playing));
    // Both presses of the third round were refused.
    assert_eq!(r.l.stats().refused, 2);
}

#[test]
fn overdub_pressed_while_stopped_waits_for_play() {
    let mut r = Rig::new(10, 128);
    let z = dc(0.0);
    record_one_bar(&mut r, &dc(0.25));
    let s = r.frame + 500;
    r.at(s, T::Stop);
    r.run(s + 1_000, &z);
    r.cmd(L::Overdub);
    r.run(s + 3_000, &dc(0.5));
    assert_eq!(r.l.status().phase, Phase::Overdubbing);
    // Pressed again before Play: nothing was written, no layer is left.
    r.cmd(L::Overdub);
    r.run(s + 3_500, &z);
    let st = r.l.status();
    assert_eq!((st.phase, st.layers, st.redo), (Phase::Playing, 1, 0));
    // Armed again, then Play: it records from the start of playback.
    r.cmd(L::Overdub);
    r.run(s + 4_000, &z);
    let p = s + 4_000;
    r.at(p, T::Play);
    r.run(p + 2_000, &dc(0.125));
    // Stopping ends the overdub with a fade.
    r.at(p + 2_000, T::Stop);
    r.run(p + 3_000, &z);
    let st = r.l.status();
    assert_eq!((st.phase, st.layers), (Phase::Playing, 2));
}

#[test]
fn clear_then_record_arms_again() {
    let mut r = Rig::new(10, 128);
    record_one_bar(&mut r, &dc(0.25));
    r.cmd(L::Clear);
    r.cmd(L::Record);
    let c = r.frame;
    r.run(c + 4 * FADE as u64, &dc(0.0));
    assert!(r.max_step(c, r.frame) <= 0.25 / FADE as f32 + 1e-5);
    let s = r.l.status();
    assert_eq!((s.phase, s.layers, s.frames), (Phase::Armed, 0, 0));
    assert!(r.l.layer(0).is_none());
}

#[test]
fn mute_and_level_fade_and_scale() {
    let mut r = Rig::new(10, 128);
    record_one_bar(&mut r, &dc(0.25));
    let z = dc(0.0);
    r.cmd(L::Mute(true));
    let m = r.frame;
    r.run(m + 2 * FADE as u64, &z);
    assert_eq!(r.left(r.frame - 1), 0.0);
    assert!(r.max_step(m, r.frame) <= 0.25 / FADE as f32 + 1e-5);
    r.cmd(L::Mute(false));
    r.cmd(L::SetLevel(0.5));
    r.run(r.frame + 2 * FADE as u64, &z);
    assert!((r.left(r.frame - 1) - 0.125).abs() < 1e-4);
    r.cmd(L::SetLevel(f32::NAN));
    r.run(r.frame + 128, &z);
    assert_eq!(r.l.status().level, 0.5);
}

/// The whole session at one period size.
fn session(period: usize) -> Vec<f32> {
    let mut r = Rig::new(10, period);
    let input = |f: u64| {
        let n = noise(f);
        let s = sine(f);
        [0.3 * n[0] + s[0], 0.3 * n[1] + s[1]]
    };
    r.cmd(L::SetLatency(77));
    r.cmd(L::Record);
    r.at(START, T::Play);
    r.at(30_000, T::SetLoop(Some((0, 2 * BAR_TICKS))));
    r.at(70_001, T::Locate(999));
    r.at(90_000, T::SetTempo(200.0));
    r.at(120_003, T::Stop);
    r.at(125_000, T::Play);
    r.run(25_600, &input);
    r.cmd(L::Overdub);
    r.run(51_200, &input);
    r.cmd(L::Overdub);
    r.run(61_440, &input);
    r.cmd(L::Overdub);
    r.run(81_920, &input);
    r.cmd(L::Undo);
    r.run(92_160, &input);
    r.cmd(L::Redo);
    r.run(102_400, &input);
    r.cmd(L::Overdub);
    r.run(130_048, &input);
    r.cmd(L::Clear);
    r.cmd(L::Record);
    r.run(150_528, &input);
    assert!(r.out.iter().any(|&v| v != 0.0));
    r.out
}

#[test]
fn every_period_size_gives_the_same_output() {
    let want = session(1024);
    for period in [64, 128, 256, 512] {
        let got = session(period);
        assert_eq!(got.len(), want.len());
        let first = got.iter().zip(&want).position(|(a, b)| a.to_bits() != b.to_bits());
        assert_eq!(first, None, "period {period} differs at sample {first:?}");
    }
}

#[test]
fn overdub_pressed_during_the_first_pass_takes_over_where_it_ends() {
    const LAT: u64 = 300;
    let mut r = Rig::new(10, 128);
    r.cmd(L::SetLatency(LAT as u32));
    // The player plays 0.25 for the first pass, then 0.125; the input
    // hears it LAT late.
    let end = START + BAR as u64;
    let played = move |f: u64| if f < end { [0.25, -0.25] } else { [0.125, -0.125] };
    let late = move |f: u64| if f < LAT { [0.0, 0.0] } else { played(f - LAT) };
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    r.at(START, T::Play);
    r.run(START + 10_000, &late);
    r.cmd(L::Overdub);
    r.cmd(L::Overdub);
    r.run(START + 10_128, &late);
    assert!(!r.l.status().overdub_next, "a second press cancels");
    r.cmd(L::Overdub);
    r.run(START + 10_256, &late);
    assert!(r.l.status().overdub_next);
    r.run(end + 1, &late);
    let s = r.l.status();
    assert_eq!((s.phase, s.layers, s.overdub_next), (Phase::Overdubbing, 2, false));
    r.run(end + 8_000, &late);
    let layer = r.l.layer(1).unwrap();
    // It starts at the loop's first frame with a short fade, and holds
    // none of the first pass's input (that went to layer 1 only).
    assert!(layer[0].abs() < 0.125 / 32.0 + 1e-6, "{}", layer[0]);
    for q in mpfree_looper::HANDOFF_FADE..7_000 {
        assert!((layer[2 * q] - 0.125).abs() < 1e-6, "position {q}: {}", layer[2 * q]);
    }
    let first = r.l.layer(0).unwrap();
    for q in (FADE..BAR - FADE).step_by(97) {
        assert!((first[2 * q] - 0.25).abs() < 1e-6, "position {q}: {}", first[2 * q]);
    }
}
