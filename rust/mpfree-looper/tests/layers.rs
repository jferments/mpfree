//! Layers between loops ([`Looper::copy_layer`]): a copy onto a loop of
//! the same length, in time with the song; a move, which sounds the same
//! throughout and which Undo in either loop takes back in both; a layer
//! that makes a loop of its own; a loop recorded at another tempo; what
//! is refused, what waits and what calls a copy off; and the same output
//! for any period size.
//!
//! 22.05 kHz at 240 BPM in 4/4 makes a bar exactly 22 050 frames (1 s).

use mpfree_looper::{Command as L, LayerCommand as Lc, Looper, Phase, Step, COPY_SPEED, FADE};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: usize = 22_050;
const BAR_TICKS: i64 = 3_840;
/// The transport starts playing here, so bar lines are at START + k * BAR.
const START: u64 = 1_000;
/// Frames a copy of a one-bar layer takes, with room for its fade-in.
const COPY: u64 = (BAR / COPY_SPEED) as u64 + 4 * FADE as u64;

struct Rig {
    t: Transport,
    l: Looper,
    b: Block,
    period: usize,
    frame: u64,
    input: Vec<f32>,
    mix: Vec<f32>,
    out: Vec<f32>,
}

impl Rig {
    fn new(period: usize) -> Rig {
        Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            l: Looper::new(RATE, 20),
            b: Block::new(),
            period,
            frame: 0,
            input: vec![0.0; 2 * period],
            mix: vec![0.0; 2 * period],
            out: Vec::new(),
        }
    }

    fn cmd(&mut self, c: L) {
        assert!(self.l.command(c));
    }

    fn select(&mut self, n: usize) {
        assert!(self.l.select_loop(n));
    }

    fn copy(&mut self, from: usize, id: u8, to: usize, keep: bool) {
        assert!(self.l.copy_layer(from, id, to, keep));
    }

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

    fn wait(&mut self, frames: u64) {
        self.run(self.frame + frames, &dc(0.0));
    }

    fn out_at(&self, f: u64) -> [f32; 2] {
        [self.out[2 * f as usize], self.out[2 * f as usize + 1]]
    }

    fn status(&self, n: usize) -> mpfree_looper::Status {
        self.l.loop_status(n).unwrap()
    }

    fn ids(&self, n: usize) -> Vec<u8> {
        (0..)
            .map_while(|k| self.l.loop_layer_info(n, k))
            .map(|i| i.id)
            .collect()
    }

    fn done(&self, n: usize) -> Vec<Step> {
        self.l.loop_history(n).unwrap().done().to_vec()
    }

    fn redo(&self, n: usize) -> Vec<Step> {
        self.l.loop_history(n).unwrap().redo().to_vec()
    }

    /// Record `bars` bars of `input` into loop `n` from the next bar line
    /// after now, and run until its first pass ends.
    fn record(&mut self, n: usize, bars: u32, input: &dyn Fn(u64) -> [f32; 2]) {
        self.select(n);
        self.cmd(L::SetBars(Some(bars)));
        self.cmd(L::Record);
        while self.status(n).phase != Phase::Playing {
            self.run(self.frame + 64, input);
        }
        self.wait(2 * FADE as u64);
    }

    /// An overdub of `input` on loop `n` for exactly one pass: its
    /// fade-out falls on its fade-in, so a steady input makes a steady
    /// layer. (Overdub may wait for a fade: the pass counts from the
    /// period it starts in.)
    fn overdub(&mut self, n: usize, input: &dyn Fn(u64) -> [f32; 2]) {
        self.select(n);
        self.cmd(L::Overdub);
        let started = loop {
            self.run(self.frame + self.period as u64, input);
            if self.status(n).phase == Phase::Overdubbing {
                break self.frame - self.period as u64;
            }
        };
        let pass = self.status(n).frames as u64;
        self.run(started + pass, input);
        // The fade-out takes the input too.
        self.cmd(L::Overdub);
        self.run(self.frame + 2 * FADE as u64, input);
    }

    /// The biggest jump between neighbouring output frames in `a..b`.
    fn max_step(&self, a: u64, b: u64) -> f32 {
        (a + 1..b)
            .map(|f| {
                let (x, y) = (self.out_at(f), self.out_at(f - 1));
                (x[0] - y[0]).abs().max((x[1] - y[1]).abs())
            })
            .fold(0.0, f32::max)
    }
}

fn dc(v: f32) -> impl Fn(u64) -> [f32; 2] {
    move |_| [v, -v]
}

/// A slow ramp, different on each channel: where each frame lands shows.
fn ramp(f: u64) -> [f32; 2] {
    let x = (f % 5_000) as f32 / 5_000.0 - 0.5;
    [x, 0.25 * x]
}

fn near(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5
}

/// Loop 1 (index 0): one bar of 0.25, with an overdub of 0.125 on top;
/// loop 2 (index 1): one bar of 0.0625. Playing from START.
fn two_loops(period: usize) -> Rig {
    let mut r = Rig::new(period);
    r.select(0);
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    assert!(r.t.command(START, T::Play));
    r.run(START + BAR as u64 + 2 * FADE as u64, &dc(0.25));
    assert_eq!(r.status(0).phase, Phase::Playing);
    r.overdub(0, &dc(0.125));
    r.record(1, 1, &dc(0.0625));
    r.wait(BAR as u64);
    r
}

#[test]
fn a_copy_goes_onto_a_loop_of_its_length_in_time_and_undo_takes_it_back() {
    let mut r = Rig::new(128);
    // Loop 1: two bars of a ramp from bar 0. Loop 2: two bars of 0.1 from
    // bar 3, so its start is a bar into loop 1's.
    r.select(0);
    r.cmd(L::SetBars(Some(2)));
    r.cmd(L::Record);
    assert!(r.t.command(START, T::Play));
    r.run(START + 2 * BAR as u64 + 2 * FADE as u64, &ramp);
    r.run(START + 2 * BAR as u64 + 300, &dc(0.0));
    r.record(1, 2, &dc(0.1));
    let (a, b) = (r.status(0), r.status(1));
    assert_eq!((a.origin, b.origin), (0, 3 * BAR_TICKS));
    assert_eq!(
        (a.ticks, b.ticks, b.frames),
        (2 * BAR_TICKS, 2 * BAR_TICKS, 2 * BAR)
    );
    // Loop 1 at frame f: what its first pass recorded at f, round the loop.
    let one = |f: u64| ramp(START + (f - START) % (2 * BAR as u64));
    r.copy(0, 0, 1, true);
    r.wait(64);
    assert!(r.status(1).copying.is_some());
    r.wait(2 * COPY);
    let s = r.status(1);
    assert_eq!((s.copying, s.layers), (None, 2));
    let k = r.ids(1)[1];
    assert_eq!(r.done(1), [Step::Added(k, 0)]);
    assert!(
        r.done(0).is_empty() && r.ids(0) == [0],
        "a copy leaves loop 1 as it was"
    );
    // Both loops hold the ramp now, in time with the song.
    let from = r.frame;
    r.wait(2 * BAR as u64);
    let seam = |f: u64| (f - START) % BAR as u64 > (BAR - 2 * FADE) as u64;
    let mut checked = 0;
    for f in (from..r.frame).step_by(13).filter(|&f| !seam(f)) {
        let x = one(f);
        let want = [2.0 * x[0] + 0.1, 2.0 * x[1] - 0.1];
        assert!(
            near(r.out_at(f), want),
            "frame {f}: {:?} vs {want:?}",
            r.out_at(f)
        );
        checked += 1;
    }
    assert!(checked > 1_000);
    // Undo takes the copy away; Redo brings it back.
    r.select(1);
    r.cmd(L::Undo);
    r.wait(4 * FADE as u64);
    assert_eq!((r.ids(1).len(), r.redo(1)), (1, vec![Step::Added(k, 0)]));
    let f = r.frame - 10;
    let x = one(f);
    assert!(seam(f) || near(r.out_at(f), [x[0] + 0.1, x[1] - 0.1]));
    r.cmd(L::Redo);
    r.wait(4 * FADE as u64);
    assert_eq!(r.ids(1), [0, k]);
}

#[test]
fn a_move_sounds_the_same_throughout_and_undo_in_either_loop_takes_it_back() {
    let mut r = two_loops(128);
    let all = [0.4375, -0.4375];
    let from = r.frame;
    assert!(near(r.out_at(from - 1), all));
    // The overdub moves from loop 1 to loop 2.
    r.copy(0, 1, 1, false);
    r.wait(2 * COPY);
    assert_eq!((r.ids(0), r.ids(1).len()), (vec![0], 2));
    let k = r.ids(1)[1];
    assert_eq!(r.done(0), [Step::Overdub(1), Step::Moved(1, 1)]);
    assert_eq!(r.done(1), [Step::Added(k, 0)]);
    // Loop 1 fades it out on the frame loop 2 fades it in: no dip.
    assert!(
        r.max_step(from, r.frame) < 1e-4,
        "{}",
        r.max_step(from, r.frame)
    );
    assert!(near(r.out_at(r.frame - 1), all));
    // Undo on loop 2 takes the move back in both loops.
    r.select(1);
    r.cmd(L::Undo);
    r.wait(4 * FADE as u64);
    assert_eq!((r.ids(0), r.ids(1)), (vec![0, 1], vec![0]));
    assert_eq!(
        (r.redo(0), r.redo(1)),
        (vec![Step::Moved(1, 1)], vec![Step::Added(k, 0)])
    );
    // Redo on loop 1 brings it back in both.
    r.select(0);
    r.cmd(L::Redo);
    r.wait(4 * FADE as u64);
    assert_eq!((r.ids(0), r.ids(1)), (vec![0], vec![0, k]));
    // Undo on loop 1 takes it back too.
    r.cmd(L::Undo);
    r.wait(4 * FADE as u64);
    assert_eq!((r.ids(0), r.ids(1)), (vec![0, 1], vec![0]));
    assert!(r.max_step(from, r.frame) < 1e-4);
    assert!(near(r.out_at(r.frame - 1), all));
    // Once one loop has moved on, the other's Undo is its own.
    r.cmd(L::Redo);
    r.wait(4 * FADE as u64);
    r.cmd(L::SetFadeOld(1.0));
    r.layer(0, Lc::Level(0.5));
    r.wait(4 * FADE as u64);
    r.overdub(0, &dc(0.0));
    r.select(1);
    r.cmd(L::Undo);
    r.wait(4 * FADE as u64);
    assert_eq!((r.ids(0).len(), r.ids(1)), (2, vec![0]));
}

impl Rig {
    fn layer(&mut self, id: u8, c: Lc) {
        assert!(self.l.layer_command(id, c));
    }
}

#[test]
fn a_layer_dropped_on_an_empty_loop_makes_a_loop_of_its_own_and_undo_clears_it() {
    let mut r = two_loops(128);
    let all = [0.4375, -0.4375];
    let from = r.frame;
    r.copy(0, 1, 2, false);
    r.wait(2 * COPY);
    let (a, c) = (r.status(0), r.status(2));
    assert_eq!(c.phase, Phase::Playing);
    assert_eq!((c.frames, c.ticks, c.origin), (a.frames, a.ticks, a.origin));
    assert_eq!((r.ids(0), r.ids(2)), (vec![0], vec![0]));
    assert_eq!(r.done(2), [Step::Added(0, 0)]);
    assert!(near(r.out_at(r.frame - 1), all));
    assert!(r.max_step(from, r.frame) < 1e-4);
    // Undo on the new loop clears it and puts the layer back in loop 1;
    // neither can be redone.
    r.select(2);
    r.cmd(L::Undo);
    r.wait(4 * FADE as u64);
    assert_eq!(r.status(2).phase, Phase::Empty);
    assert_eq!(r.ids(0), [0, 1]);
    assert!(r.redo(0).is_empty() && r.redo(2).is_empty());
    assert!(near(r.out_at(r.frame - 1), all));
    // A loop's only layer stays where it is: a copy.
    r.copy(1, 0, 3, false);
    r.wait(2 * COPY);
    assert_eq!((r.ids(1), r.ids(3)), (vec![0], vec![0]));
    assert!(r.done(1).is_empty());
    assert_eq!(r.done(3), [Step::Added(0, 1)]);
}

#[test]
fn another_length_or_a_busy_loop_refuses_and_clear_or_undo_calls_a_copy_off() {
    let mut r = two_loops(128);
    // Loop 3: two bars.
    r.record(2, 2, &dc(0.01));
    let refused = r.l.stats().refused;
    r.copy(0, 1, 2, false);
    r.copy(0, 1, 0, false);
    r.copy(0, 9, 1, false);
    r.wait(2 * COPY);
    assert_eq!(r.l.stats().refused, refused + 2);
    assert_eq!(r.l.stats().no_layer, 1);
    assert_eq!((r.ids(0).len(), r.ids(2).len()), (2, 1));
    // While a layer copies out, its loop's commands wait for it.
    let used = r.l.memory().used;
    r.copy(0, 1, 1, false);
    r.select(0);
    r.layer(1, Lc::Level(0.5));
    r.wait(64);
    assert!(r.status(1).copying.is_some());
    assert_eq!(r.l.loop_layer_info(0, 1).unwrap().level, 1.0);
    // Clear on loop 1 calls it off: loop 2 takes nothing, and the memory
    // comes back once loop 1 has faded out.
    r.cmd(L::Clear);
    r.wait(4 * FADE as u64);
    assert_eq!(r.status(1).copying, None);
    assert_eq!((r.status(0).phase, r.ids(1)), (Phase::Empty, vec![0]));
    assert!(r.l.memory().used < used);
    // Undo on the loop a layer is copying into calls it off too.
    r.copy(1, 0, 3, true);
    r.select(3);
    r.cmd(L::Undo);
    r.wait(2 * COPY);
    assert_eq!(r.status(3).phase, Phase::Empty);
    assert_eq!(r.status(3).used, 0);
}

/// Loop 1 at 240 BPM, loop 2 one bar recorded at 200 BPM (more frames),
/// a layer of loop 1 moved onto loop 2, and Undo: the output.
fn tempo_session(period: usize) -> (Vec<f32>, Rig) {
    let mut r = Rig::new(period);
    r.select(0);
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    assert!(r.t.command(START, T::Play));
    r.run(START + BAR as u64 + 2 * FADE as u64, &ramp);
    r.overdub(0, &dc(0.125));
    assert!(r.t.command(r.frame, T::SetTempo(200.0)));
    r.record(1, 1, &dc(0.0625));
    r.copy(0, 0, 1, false);
    r.wait(3 * COPY);
    r.select(1);
    r.cmd(L::Undo);
    r.wait(4 * FADE as u64);
    (r.out.clone(), r)
}

#[test]
fn a_loop_recorded_at_another_tempo_takes_the_layer_at_its_own_length() {
    let (_, mut r) = tempo_session(128);
    let (a, b) = (r.status(0), r.status(1));
    assert_eq!(a.ticks, b.ticks);
    assert!(b.frames > a.frames + 1_000, "{} vs {}", b.frames, a.frames);
    // Redo puts the moved layer back on loop 2: read between loop 1's
    // frames, at loop 2's length.
    r.cmd(L::Redo);
    r.wait(4 * FADE as u64);
    let k = r.ids(1)[1];
    let ratio = a.frames as f64 / b.frames as f64;
    let shift =
        ((b.origin - a.origin).rem_euclid(a.ticks)) as f64 * a.frames as f64 / a.ticks as f64;
    // Loop 1's first pass is in its undo history now: read it as recorded.
    let src: Vec<[f32; 2]> = (0..a.frames).map(|q| ramp(START + q as u64)).collect();
    let new = r.l.loop_layer(1, 1).unwrap();
    assert_eq!(new.len(), 2 * b.frames);
    assert_eq!(r.l.loop_layer_info(1, 1).unwrap().id, k);
    for q in (0..b.frames).step_by(97) {
        let x = (q as f64 * ratio + shift).rem_euclid(a.frames as f64);
        let i = x as usize;
        let j = (i + 1) % a.frames;
        // Away from the first pass's seam and the ramp's jumps.
        if i + FADE >= a.frames || (START as usize + i) % 5_000 >= 4_990 {
            continue;
        }
        let t = (x - i as f64) as f32;
        let want = [
            src[i][0] + (src[j][0] - src[i][0]) * t,
            src[i][1] + (src[j][1] - src[i][1]) * t,
        ];
        let got = [new[2 * q], new[2 * q + 1]];
        assert!(near(got, want), "frame {q}: {got:?} vs {want:?}");
    }
}

#[test]
fn copies_give_the_same_output_for_any_period_size() {
    let (a, _) = tempo_session(128);
    for period in [1, 37, 512] {
        let (b, _) = tempo_session(period);
        assert_eq!(a.len(), b.len());
        assert!(a == b, "period {period}");
    }
}
