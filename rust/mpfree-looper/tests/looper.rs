//! The looper against a real transport: the first pass on the bar, sample
//! for sample playback, latency, the loop point, overdub layers with undo
//! and redo, fades, jumps, stop, count-in, free length, transport loops,
//! full memory, each layer's level, pan, mute, solo and delete, several
//! loops sharing one memory, one recording at a time, and the same output
//! for any period size.
//!
//! 22.05 kHz at 240 BPM in 4/4 makes a bar exactly 22 050 frames (1 s).

use mpfree_looper::{
    Command as L, LayerCommand as Lc, Looper, Phase, BLOCK, FADE, MAX_LAYERS, MAX_LOOPS,
};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: usize = 22_050;
const BAR_TICKS: i64 = 3_840;
/// The transport starts playing here, so bar lines are at START + k * BAR.
const START: u64 = 1_000;

/// The memory `layers` one-bar layers hold: whole blocks.
fn held(layers: usize) -> usize {
    layers * BAR.div_ceil(BLOCK) * BLOCK
}

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

    fn layer(&mut self, id: u8, c: Lc) {
        assert!(self.l.layer_command(id, c));
    }

    /// The loop the commands after this act on.
    fn select(&mut self, n: usize) {
        assert!(self.l.select_loop(n));
    }

    fn phase(&self, n: usize) -> Phase {
        self.l.loop_status(n).unwrap().phase
    }

    /// The layers' ids in play order.
    fn ids(&self) -> Vec<u8> {
        (0..)
            .map_while(|k| self.l.layer_info(k))
            .map(|i| i.id)
            .collect()
    }

    /// One overdub of `input` for `frames` frames, then its fade-out.
    fn overdub(&mut self, frames: u64, input: &dyn Fn(u64) -> [f32; 2]) {
        self.cmd(L::Overdub);
        self.run(self.frame + frames, input);
        self.cmd(L::Overdub);
        self.run(self.frame + 2 * FADE as u64, &dc(0.0));
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
            self.l
                .process(&self.b, &self.input[..2 * n], &mut self.mix[..2 * n]);
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
        (a + 1..b)
            .map(|f| {
                (self.left(f) - self.left(f - 1))
                    .abs()
                    .max((self.right(f) - self.right(f - 1)).abs())
            })
            .fold(0.0, f32::max)
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
    assert!(r.out[..2 * (START as usize + BAR)]
        .iter()
        .all(|&v| v == 0.0));
    r.run(START + 2 * BAR as u64, &|_| [0.0, 0.0]);
    let len = BAR as u64;
    for f in START + len + FADE as u64..START + 2 * len - FADE as u64 {
        let want = noise(f - len);
        assert!(
            (r.left(f) - want[0]).abs() < 1e-4,
            "frame {f}: {} vs {}",
            r.left(f),
            want[0]
        );
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
    let step = r.max_step(
        START + BAR as u64 + FADE as u64 + 1,
        START + 3 * BAR as u64 - 1,
    );
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
        assert!(
            (layer[2 * q] - 0.25).abs() < 1e-5,
            "position {q}: {}",
            layer[2 * q]
        );
    }
    for q in (5_000 + FADE + 1..BAR).chain(0..2_000) {
        assert!(
            (layer[2 * q] - 0.125).abs() < 1e-5,
            "position {q}: {}",
            layer[2 * q]
        );
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
    assert!(
        r.max_step(u, r.frame) < 0.125 / FADE as f32 * 1.5 + 1e-4,
        "undo clicked"
    );
    r.cmd(L::Redo);
    let u = r.frame;
    r.run(u + 4 * FADE as u64, &z);
    let s = r.l.status();
    assert_eq!((s.layers, s.redo), (2, 0));
    assert!(
        r.max_step(u, r.frame) < 0.25 / FADE as f32 * 1.5 + 1e-4,
        "redo clicked"
    );

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
    assert_eq!(layer.len(), 2 * BAR);
    // The waveform shows only what this overdub wrote, not the old audio.
    assert_eq!(
        (layer[2 * 2_000], layer[2 * 15_000 + 1]),
        (0.0, 0.0),
        "stale audio drawn"
    );
    assert!((layer[2 * 6_000] - 0.125).abs() < 1e-6);
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
    assert!(
        (r.left(g + 15_000) - 0.25).abs() < 1e-4,
        "stale audio heard"
    );
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
    assert_eq!(
        (s.phase, s.frames, s.ticks),
        (Phase::Playing, 2 * BAR, 2 * BAR_TICKS)
    );
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
    r.run(66_560, &input);
    r.layer(1, Lc::Pan(-0.7));
    r.run(71_680, &input);
    r.layer(0, Lc::Level(1.6));
    r.run(76_800, &input);
    r.layer(1, Lc::Delete);
    r.run(81_920, &input);
    r.cmd(L::Undo);
    r.run(92_160, &input);
    r.cmd(L::Redo);
    r.run(102_400, &input);
    r.cmd(L::Overdub);
    r.run(112_640, &input);
    r.layer(0, Lc::Solo(true));
    r.run(122_880, &input);
    r.layer(0, Lc::Solo(false));
    r.layer(2, Lc::Mute(true));
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
        let first = got
            .iter()
            .zip(&want)
            .position(|(a, b)| a.to_bits() != b.to_bits());
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
    let played = move |f: u64| {
        if f < end {
            [0.25, -0.25]
        } else {
            [0.125, -0.125]
        }
    };
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
    assert_eq!(
        (s.phase, s.layers, s.overdub_next),
        (Phase::Overdubbing, 2, false)
    );
    r.run(end + 8_000, &late);
    let layer = r.l.layer(1).unwrap();
    // It starts at the loop's first frame with a short fade, and holds
    // none of the first pass's input (that went to layer 1 only).
    assert!(layer[0].abs() < 0.125 / 32.0 + 1e-6, "{}", layer[0]);
    for q in mpfree_looper::HANDOFF_FADE..7_000 {
        assert!(
            (layer[2 * q] - 0.125).abs() < 1e-6,
            "position {q}: {}",
            layer[2 * q]
        );
    }
    let first = r.l.layer(0).unwrap();
    for q in (FADE..BAR - FADE).step_by(97) {
        assert!(
            (first[2 * q] - 0.25).abs() < 1e-6,
            "position {q}: {}",
            first[2 * q]
        );
    }
}

/// The output's left and right at `f`, against what is wanted.
fn near(r: &Rig, f: u64, want: [f32; 2]) -> bool {
    (r.left(f) - want[0]).abs() < 1e-4 && (r.right(f) - want[1]).abs() < 1e-4
}

#[test]
fn each_layer_has_its_own_level_pan_mute_and_solo() {
    let mut r = Rig::new(10, 128);
    let z = dc(0.0);
    record_one_bar(&mut r, &dc(0.25));
    // A whole-loop overdub of 0.125, so both layers sound everywhere.
    r.overdub(BAR as u64 + 2 * FADE as u64, &dc(0.125));
    assert_eq!(r.ids(), vec![0, 1]);
    let i = r.l.layer_info(1).unwrap();
    assert_eq!(
        (i.level, i.pan, i.muted, i.solo, i.audible, i.recording),
        (1.0, 0.0, false, false, true, false)
    );
    let settle = |r: &mut Rig| {
        let a = r.frame;
        r.run(a + 3 * FADE as u64, &z);
        let f = r.frame - 1;
        (a, f)
    };
    let (_, f) = settle(&mut r);
    assert!(near(&r, f, [0.375, -0.375]));
    // Mute the overdub: only the first pass, and no click on the way.
    r.layer(1, Lc::Mute(true));
    let (a, f) = settle(&mut r);
    assert!(near(&r, f, [0.25, -0.25]));
    assert!(
        r.max_step(a, f) <= 0.125 / FADE as f32 + 1e-5,
        "mute clicked"
    );
    assert!(!r.l.layer_info(1).unwrap().audible);
    // The first pass at level 2, panned hard right: its left side goes.
    r.layer(1, Lc::Mute(false));
    r.layer(0, Lc::Level(2.0));
    r.layer(0, Lc::Pan(1.0));
    let (a, f) = settle(&mut r);
    assert!(near(&r, f, [0.125, -0.625]));
    // Each gain moves one fade step a frame: the first pass's 0.25 and the
    // overdub's 0.125 coming back together.
    assert!(
        r.max_step(a, f) <= 0.375 / FADE as f32 + 1e-5,
        "level and pan clicked"
    );
    // Solo the overdub: only it plays; solo off brings the first pass back.
    r.layer(1, Lc::Solo(true));
    let (_, f) = settle(&mut r);
    assert!(near(&r, f, [0.125, -0.125]));
    assert!(!r.l.layer_info(0).unwrap().audible);
    r.layer(1, Lc::Solo(false));
    r.layer(0, Lc::Pan(f32::NAN));
    r.layer(0, Lc::Level(9.0));
    let (_, f) = settle(&mut r);
    assert!(near(&r, f, [0.125, -0.625]), "NaN is ignored, 9 is 2");
    // A layer that is not there is counted, not guessed.
    r.layer(7, Lc::Mute(true));
    r.run(r.frame + 128, &z);
    assert_eq!(r.l.stats().no_layer, 1);
}

#[test]
fn a_deleted_layer_comes_back_with_undo_in_its_place() {
    let mut r = Rig::new(10, 128);
    let z = dc(0.0);
    let whole = BAR as u64 + 2 * FADE as u64;
    record_one_bar(&mut r, &dc(0.25));
    r.overdub(whole, &dc(0.125));
    r.overdub(whole, &dc(0.0625));
    assert_eq!(r.ids(), vec![0, 1, 2]);
    r.run(r.frame + 3 * FADE as u64, &z);
    assert!(near(&r, r.frame - 1, [0.4375, -0.4375]));
    // Delete the middle layer: it fades out, the others keep their ids.
    r.layer(1, Lc::Delete);
    let a = r.frame;
    r.run(a + 3 * FADE as u64, &z);
    assert_eq!(r.ids(), vec![0, 2]);
    assert!(near(&r, r.frame - 1, [0.3125, -0.3125]));
    assert!(
        r.max_step(a, r.frame) <= 0.125 / FADE as f32 + 1e-5,
        "delete clicked"
    );
    let s = r.l.status();
    assert_eq!((s.layers, s.redo), (2, 0));
    assert_eq!(
        s.used,
        held(3),
        "Undo can bring it back, so it holds its memory"
    );
    // Undo brings it back in its place; Redo deletes it again.
    r.cmd(L::Undo);
    r.run(r.frame + 3 * FADE as u64, &z);
    assert_eq!(r.ids(), vec![0, 1, 2]);
    assert!(near(&r, r.frame - 1, [0.4375, -0.4375]));
    r.cmd(L::Redo);
    r.run(r.frame + 3 * FADE as u64, &z);
    assert_eq!(r.ids(), vec![0, 2]);
    // One history: the next Undo brings back the delete, the one after
    // takes the newest overdub away.
    r.cmd(L::Undo);
    r.cmd(L::Undo);
    r.run(r.frame + 6 * FADE as u64, &z);
    assert_eq!(r.ids(), vec![0, 1]);
    assert_eq!(r.l.status().redo, 2);
    // The first pass can go too, but the last layer stays.
    r.layer(0, Lc::Delete);
    r.run(r.frame + 3 * FADE as u64, &z);
    assert_eq!(r.ids(), vec![1]);
    r.layer(1, Lc::Delete);
    r.run(r.frame + 3 * FADE as u64, &z);
    assert_eq!(r.ids(), vec![1]);
    assert!(near(&r, r.frame - 1, [0.125, -0.125]));
}

#[test]
fn deleting_the_overdub_being_recorded_throws_it_away_like_undo() {
    let mut r = Rig::new(10, 128);
    let z = dc(0.0);
    record_one_bar(&mut r, &dc(0.25));
    r.cmd(L::Overdub);
    r.run(r.frame + 5_000, &dc(0.125));
    let rec = r.l.layer_info(1).unwrap();
    assert!(rec.recording);
    r.layer(rec.id, Lc::Delete);
    r.run(r.frame + 3 * FADE as u64, &z);
    let s = r.l.status();
    assert_eq!((s.phase, s.layers, s.redo), (Phase::Playing, 1, 1));
    r.cmd(L::Redo);
    r.run(r.frame + 3 * FADE as u64, &z);
    assert_eq!(r.ids(), vec![0, rec.id]);
}

#[test]
fn a_new_overdub_takes_the_memory_of_the_oldest_delete_when_full() {
    // 3 seconds: room for three one-second layers.
    let mut r = Rig::new(3, 128);
    let z = dc(0.0);
    record_one_bar(&mut r, &dc(0.25));
    r.overdub(2_000, &dc(0.1));
    r.overdub(2_000, &dc(0.1));
    assert_eq!(r.l.status().used, held(3));
    r.layer(1, Lc::Delete);
    r.run(r.frame + 3 * FADE as u64, &z);
    assert_eq!(r.l.status().used, held(3), "the delete can be undone");
    // Full: a new overdub takes the deleted layer's slot, and Undo can no
    // longer bring that layer back.
    r.overdub(2_000, &dc(0.2));
    assert_eq!(r.l.stats().refused, 0);
    assert_eq!(r.ids(), vec![0, 2, 1]);
    // The history up to that delete is gone: one Undo takes the new
    // overdub away, and there is nothing older to undo.
    r.cmd(L::Undo);
    r.cmd(L::Undo);
    r.run(r.frame + 6 * FADE as u64, &z);
    assert_eq!(r.ids(), vec![0, 2]);
    let s = r.l.status();
    assert_eq!(
        (s.redo, s.used),
        (1, held(3)),
        "Redo can bring the new overdub back"
    );
}

#[test]
fn several_loops_play_together_and_commands_go_to_the_selected_one() {
    let mut r = Rig::new(10, 128);
    let z = dc(0.0);
    record_one_bar(&mut r, &dc(0.25));
    // Loop 2: two bars of a sine, from the next bar line.
    r.select(1);
    r.cmd(L::SetBars(Some(2)));
    r.cmd(L::Record);
    r.run(r.frame + 128, &sine);
    assert_eq!(
        (r.l.selected(), r.phase(1), r.phase(0)),
        (1, Phase::Armed, Phase::Playing)
    );
    let from = START + 2 * BAR as u64;
    // Loop 2 at frame `f`: frame f - from of the pass, around the loop.
    let two = |f: u64| sine(from + (f - from) % (2 * BAR as u64));
    // Away from the loop point, where the end fades into the start.
    let clear_of_seam = |f: u64| ((f - from) % (2 * BAR as u64)) < 2 * (BAR - FADE) as u64;
    r.run(from + 2 * BAR as u64 + 1, &sine);
    let (a, b) = (r.l.loop_status(0).unwrap(), r.l.status());
    assert_eq!(
        (a.frames, b.frames, b.ticks, b.origin),
        (BAR, 2 * BAR, 2 * BAR_TICKS, 2 * BAR_TICKS)
    );
    assert_eq!((b.phase, b.layers), (Phase::Playing, 1));
    // Both sound, each in time with the song.
    let end = from + 4 * BAR as u64;
    r.run(end, &z);
    let both = |f: u64| {
        let s = two(f);
        [0.25 + s[0], -0.25 + s[1]]
    };
    for f in (from + 2 * BAR as u64 + FADE as u64..end - FADE as u64).step_by(11) {
        assert!(
            near(&r, f, both(f)),
            "frame {f}: {} vs {:?}",
            r.left(f),
            both(f)
        );
    }
    // Mute goes to the selected loop only.
    r.cmd(L::Mute(true));
    r.run(r.frame + 2 * FADE as u64, &z);
    assert!(r.l.status().muted && !r.l.loop_status(0).unwrap().muted);
    let f = r.frame - 1;
    assert!(near(&r, f, [0.25, -0.25]));
    // Back to loop 1: Clear forgets it; loop 2 plays on.
    r.cmd(L::Mute(false));
    r.select(0);
    r.cmd(L::Clear);
    r.run(r.frame + 3 * FADE as u64, &z);
    assert_eq!(
        (r.l.selected(), r.phase(0), r.phase(1)),
        (0, Phase::Empty, Phase::Playing)
    );
    for f in (r.frame - FADE as u64..r.frame).filter(|&f| clear_of_seam(f)) {
        assert!(near(&r, f, two(f)), "frame {f}");
    }
    // Past the last loop: counted, and the selection stays.
    r.select(MAX_LOOPS);
    r.run(r.frame + 128, &z);
    assert_eq!((r.l.selected(), r.l.stats().no_layer), (0, 1));
    assert!(r.l.loop_status(MAX_LOOPS).is_none() && r.l.loop_layer(MAX_LOOPS, 0).is_none());
    // Each loop's layers are its own.
    assert!(r.l.layer(0).is_none());
    let w = r.l.loop_layer(1, 0).unwrap();
    let q = 5_000;
    assert_eq!((w.len(), w[2 * q]), (4 * BAR, sine(from + q as u64)[0]));
}

#[test]
fn one_loop_records_at_a_time() {
    let mut r = Rig::new(10, 128);
    let period = 128u64;
    record_one_bar(&mut r, &dc(0.25));
    r.cmd(L::Overdub);
    r.run(r.frame + 2_000, &dc(0.1));
    assert_eq!(r.phase(0), Phase::Overdubbing);
    // Arming loop 2 punches loop 1's overdub out. (The overdub on loop 1
    // below waits for its tail to fade out.)
    r.select(1);
    r.cmd(L::Record);
    r.run(r.frame + period, &dc(0.1));
    assert_eq!((r.phase(0), r.phase(1)), (Phase::Playing, Phase::Armed));
    // An overdub on loop 1 disarms loop 2.
    r.select(0);
    r.cmd(L::Overdub);
    r.run(r.frame + period, &dc(0.1));
    assert_eq!(
        (r.phase(0), r.phase(1)),
        (Phase::Playing, Phase::Armed),
        "waiting for the tail"
    );
    r.run(r.frame + FADE as u64 + period, &dc(0.1));
    assert_eq!((r.phase(0), r.phase(1)), (Phase::Overdubbing, Phase::Empty));
    // Loop 3 records free length; loop 1's overdub punches out.
    r.select(2);
    r.cmd(L::SetBars(None));
    r.cmd(L::Record);
    let line = START + 2 * BAR as u64;
    r.run(line + 500, &noise);
    assert_eq!((r.phase(0), r.phase(2)), (Phase::Playing, Phase::Recording));
    assert_eq!(r.l.loop_status(0).unwrap().layers, 3);
    // Loop 4 arms: loop 3's pass ends on the next bar line, where loop 4's
    // starts.
    r.select(3);
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    r.run(line + BAR as u64, &noise);
    assert_eq!((r.phase(2), r.phase(3)), (Phase::Recording, Phase::Armed));
    assert!(r.l.loop_status(2).unwrap().ending);
    r.run(line + BAR as u64 + 1, &noise);
    assert_eq!((r.phase(2), r.phase(3)), (Phase::Playing, Phase::Recording));
    let (c, d) = (r.l.loop_status(2).unwrap(), r.l.loop_status(3).unwrap());
    assert_eq!(
        (c.frames, c.origin, d.origin),
        (BAR, 2 * BAR_TICKS, 3 * BAR_TICKS)
    );
    r.run(line + 2 * BAR as u64 + 1, &noise);
    assert_eq!(
        (r.phase(3), r.l.loop_status(3).unwrap().frames),
        (Phase::Playing, BAR)
    );
    // Each pass holds what was played in its own bar.
    let three = r.l.loop_layer(2, 0).unwrap();
    let four = r.l.loop_layer(3, 0).unwrap();
    for q in (FADE..BAR - FADE).step_by(97) {
        assert_eq!(three[2 * q], noise(line + q as u64)[0]);
        assert_eq!(four[2 * q], noise(line + (BAR + q) as u64)[0]);
    }
    assert_eq!(r.l.stats().aborted, 0);
}

#[test]
fn the_loops_share_one_memory_and_clear_gives_it_back() {
    // 3 seconds and a block per layer: room for three one-second layers.
    let mut r = Rig::new(3, 128);
    let z = dc(0.0);
    let capacity = ((3 * BAR).div_ceil(BLOCK) + MAX_LAYERS) * BLOCK;
    assert_eq!((r.l.memory().used, r.l.memory().capacity), (0, capacity));
    record_one_bar(&mut r, &dc(0.25));
    r.overdub(2_000, &dc(0.1));
    assert_eq!(r.l.memory().used, held(2));
    // Loop 2 takes the third second.
    r.select(1);
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    r.run(START + 4 * BAR as u64 + 1, &dc(0.125));
    assert_eq!(r.phase(1), Phase::Playing);
    let m = r.l.memory();
    assert_eq!(
        (m.used, r.l.loop_status(0).unwrap().used, r.l.status().used),
        (held(3), held(2), held(1))
    );
    assert_eq!(r.l.status().capacity, capacity);
    // Full: loop 2's overdub is refused.
    r.overdub(2_000, &dc(0.1));
    assert_eq!((r.l.stats().refused, r.l.status().layers), (2, 1));
    // Clearing loop 1 gives its memory back once it has faded out.
    r.select(0);
    r.cmd(L::Clear);
    r.run(r.frame + 128, &z);
    assert_eq!(r.l.memory().used, held(3), "still fading out");
    r.run(r.frame + 2 * FADE as u64, &z);
    assert_eq!((r.phase(0), r.l.memory().used), (Phase::Empty, held(1)));
    r.select(1);
    r.overdub(2_000, &dc(0.1));
    assert_eq!(
        (r.l.stats().refused, r.l.status().layers, r.l.memory().used),
        (2, 2, held(2))
    );
}

#[test]
fn a_first_pass_that_outgrows_the_memory_is_thrown_away() {
    let mut r = Rig::new(3, 128);
    record_one_bar(&mut r, &dc(0.25));
    // Free length on loop 2: what is left runs out during its third bar.
    r.select(1);
    r.cmd(L::SetBars(None));
    r.cmd(L::Record);
    r.run(START + 3 * BAR as u64 + 1, &noise);
    assert_eq!(r.phase(1), Phase::Recording);
    assert!(r.l.memory().used > held(1));
    r.run(START + 6 * BAR as u64, &noise);
    assert_eq!((r.phase(1), r.l.stats().aborted), (Phase::Empty, 1));
    assert_eq!((r.l.memory().used, r.phase(0)), (held(1), Phase::Playing));
    // Loop 1 played on throughout.
    let f = r.frame - 1;
    assert!(near(&r, f, [0.25, -0.25]));
}

fn loops_session(period: usize) -> Vec<f32> {
    let mut r = Rig::new(8, period);
    let input = |f: u64| {
        let n = noise(f);
        let s = sine(f);
        [0.3 * n[0] + s[0], 0.3 * n[1] + s[1]]
    };
    r.cmd(L::SetLatency(77));
    r.cmd(L::Record);
    r.at(START, T::Play);
    r.at(140_000, T::Locate(1_234));
    r.at(150_003, T::Stop);
    r.at(156_000, T::Play);
    // Loop 1: two bars, going straight into an overdub.
    r.run(25_600, &input);
    r.cmd(L::Overdub);
    // Loop 2, free length: loop 1's overdub punches out.
    r.run(51_200, &input);
    r.select(1);
    r.cmd(L::SetBars(None));
    r.cmd(L::Record);
    // Loop 3 arms: loop 2's pass ends on the next bar line.
    r.run(76_800, &input);
    r.select(2);
    r.cmd(L::SetLevel(0.5));
    r.cmd(L::Record);
    // An overdub on loop 2: loop 3's pass ends on the next bar line.
    r.run(92_160, &input);
    r.select(1);
    r.cmd(L::Overdub);
    r.run(102_400, &input);
    r.select(0);
    r.layer(1, Lc::Pan(0.8));
    r.run(112_640, &input);
    r.cmd(L::Clear);
    r.run(122_880, &input);
    r.select(1);
    r.cmd(L::Overdub);
    r.cmd(L::Undo);
    r.run(130_000, &input);
    r.select(2);
    r.cmd(L::Mute(true));
    r.run(180_000, &input);
    let phases: Vec<_> = (0..4).map(|n| r.phase(n)).collect();
    assert_eq!(
        phases,
        [Phase::Empty, Phase::Playing, Phase::Playing, Phase::Empty]
    );
    let (b, c) = (r.l.loop_status(1).unwrap(), r.l.loop_status(2).unwrap());
    assert_eq!(
        (b.frames, b.layers, b.redo, c.frames, c.muted),
        (BAR, 1, 1, BAR, true)
    );
    assert_eq!(r.l.stats(), Default::default());
    assert!(r.out.iter().any(|&v| v != 0.0));
    r.out
}

#[test]
fn several_loops_give_the_same_output_for_any_period_size() {
    let want = loops_session(1024);
    for period in [64, 128, 333, 512] {
        let got = loops_session(period);
        assert_eq!(got.len(), want.len());
        let first = got
            .iter()
            .zip(&want)
            .position(|(a, b)| a.to_bits() != b.to_bits());
        assert_eq!(first, None, "period {period} differs at sample {first:?}");
    }
}
