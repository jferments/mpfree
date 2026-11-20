//! Capture against a real transport: the last bars heard become a loop,
//! ending on the bar line just passed (or, pressed in a bar's last beat,
//! the one coming; the last one if the song stops first), read `latency`
//! late, in time with the song; fewer bars when fewer were played since
//! the song started or the input changed; Undo or Capture again calls it
//! off; and the period size changes nothing.
//!
//! 22.05 kHz at 240 BPM in 4/4 makes a bar exactly 22 050 frames (1 s)
//! and a beat 5 512.5.

use mpfree_looper::{Command as L, Looper, Phase, FADE};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050;
const BAR_TICKS: i64 = 3_840;

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

    fn run(&mut self, to: u64) {
        while self.frame < to {
            let n = (self.period as u64).min(to - self.frame) as usize;
            for i in 0..n {
                let x = heard(self.frame + i as u64);
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

    /// Runs until loop 0 plays (at most `most` frames more).
    fn until_playing(&mut self, most: u64) {
        let to = self.frame + most;
        while self.l.status().phase != Phase::Playing {
            assert!(self.frame < to, "still {:?}", self.l.status());
            let f = self.frame + 64;
            self.run(f);
        }
    }

    /// Loop 0's layer, as recorded.
    fn layer(&self) -> Vec<[f32; 2]> {
        let v = self.l.loop_layer(0, 0).expect("a layer");
        let mut out = vec![[0.0; 2]; v.len() / 2];
        v.read(0, &mut out);
        out
    }
}

/// What the looper hears: a different value on every frame, so where a
/// frame came from shows.
fn heard(f: u64) -> [f32; 2] {
    let x = (f % 100_003) as f32 / 100_003.0 - 0.5;
    [x, -x]
}

/// Playing from bar 1 to `to` frames.
fn played(period: usize, to: u64) -> Rig {
    let mut r = Rig::new(period);
    assert!(r.t.command(0, T::Play));
    r.run(to);
    r
}

#[test]
fn capture_takes_the_last_bars_played_and_plays_them_in_time() {
    // Bar 4, beat 2: the bar line just passed is bar 4's.
    let mut r = played(64, 3 * BAR + 6_000);
    r.cmd(L::Capture(2));
    r.run(r.frame + 64);
    assert!(r.l.status().capturing.is_some(), "copying in");
    assert_eq!(r.l.status().phase, Phase::Empty, "silent until it is in");
    r.until_playing(BAR);
    let s = r.l.status();
    assert_eq!(s.capturing, None);
    assert_eq!(
        (s.frames, s.ticks, s.origin),
        (2 * BAR as usize, 2 * BAR_TICKS, BAR_TICKS)
    );
    assert_eq!(s.layers, 1);
    // Bars 2 and 3, frame for frame, up to the loop point's fade.
    let a = r.layer();
    for (j, x) in a[..a.len() - FADE].iter().enumerate() {
        assert_eq!(*x, heard(BAR + j as u64), "frame {j}");
    }
    // The loop point fades into what came just before bar 2.
    let last = a[a.len() - 1];
    let before = heard(BAR - 1);
    assert!(
        (last[0] - before[0]).abs() < 0.01,
        "{last:?} against {before:?}"
    );
    // Once it has faded in, it plays in time with the song: frame f of
    // the song plays bar 2 or 3's frame at the same place in the bars.
    let from = r.frame + FADE as u64;
    r.run(from + BAR);
    for f in from..from + BAR {
        let p = BAR + (f - BAR) % (2 * BAR);
        let want = a[(p - BAR) as usize];
        let got = [r.out[2 * f as usize], r.out[2 * f as usize + 1]];
        assert!(
            (got[0] - want[0]).abs() < 1e-6 && (got[1] - want[1]).abs() < 1e-6,
            "frame {f}"
        );
    }
    // Round the loop point (bar 6's line) with no click: what was heard
    // jumps by 0.44 between bar 3's end and bar 2's start, but the end
    // fades into what came before bar 2.
    let at = 5 * BAR;
    r.run(at + 1_000);
    let step = (at - 600..at + 600)
        .map(|f| (r.out[2 * f as usize] - r.out[2 * (f - 1) as usize]).abs())
        .fold(0.0, f32::max);
    assert!(step < 0.005, "a step of {step} at the loop point");
    // Clear (with Undo) takes it away, as any loop.
    r.cmd(L::Clear);
    r.run(r.frame + 2 * FADE as u64);
    assert_eq!(r.l.status().phase, Phase::Empty);
}

#[test]
fn pressed_in_a_bars_last_beat_it_waits_for_the_bar_line() {
    // Bar 3, beat 4.
    let mut r = played(64, 2 * BAR + 17_000);
    assert!(r.l.capture_waits());
    r.cmd(L::Capture(2));
    r.run(3 * BAR - 256);
    assert_eq!(r.l.status().capturing, Some(0.0), "waits for bar 4");
    r.until_playing(BAR);
    let s = r.l.status();
    assert_eq!(
        (s.ticks, s.origin),
        (2 * BAR_TICKS, BAR_TICKS),
        "bars 2 and 3"
    );
    assert_eq!(r.layer()[0], heard(BAR));
}

#[test]
fn waiting_for_its_bar_line_a_stop_takes_the_bars_up_to_the_last() {
    // Bar 3, beat 4: it waits for bar 4's line, but the song stops first.
    let mut r = played(64, 2 * BAR + 17_000);
    r.cmd(L::Capture(2));
    r.run(r.frame + 64);
    assert_eq!(r.l.status().capturing, Some(0.0), "waits for bar 4");
    assert!(r.t.command(0, T::Stop));
    r.until_playing(BAR);
    let s = r.l.status();
    assert_eq!((s.ticks, s.origin), (2 * BAR_TICKS, 0), "bars 1 and 2");
    assert_eq!(r.layer()[0], heard(0));
    assert_eq!(r.layer()[BAR as usize], heard(BAR));
}

#[test]
fn the_take_is_read_latency_late() {
    let mut r = played(64, 3 * BAR + 6_000);
    r.cmd(L::SetLatency(300));
    r.cmd(L::Capture(1));
    r.until_playing(BAR);
    let s = r.l.status();
    assert_eq!((s.frames, s.origin), (BAR as usize, 2 * BAR_TICKS), "bar 3");
    assert_eq!(r.layer()[0], heard(2 * BAR + 300));
    assert_eq!(r.layer()[100], heard(2 * BAR + 400));
}

#[test]
fn fewer_bars_when_fewer_were_played_and_none_before_a_whole_bar() {
    // Half a bar: no whole bar heard.
    let mut r = played(64, BAR / 2);
    r.cmd(L::Capture(2));
    r.run(r.frame + 128);
    let s = r.l.status();
    assert_eq!((s.phase, s.capturing), (Phase::Empty, None));
    assert_eq!(r.l.stats().refused, 1);
    assert!(r.l.capture_take(2).is_none());
    // A bar and a half: one bar, though four were asked for.
    r.run(BAR + BAR / 2);
    assert_eq!(r.l.capture_take(4).map(|t| t.bars), Some(1));
    r.cmd(L::Capture(4));
    r.until_playing(BAR);
    let s = r.l.status();
    assert_eq!((s.frames, s.origin), (BAR as usize, 0));
    // Three bars: two of the four (halving).
    let mut r = played(64, 3 * BAR + 100);
    assert_eq!(r.l.capture_take(4).map(|t| t.bars), Some(2));
    // A stop in between: the bars since PLAY only.
    assert!(r.t.command(0, T::Stop));
    r.run(r.frame + BAR);
    assert!(r.t.command(0, T::Play));
    r.run(r.frame + BAR);
    // Started again just after bar 4's line: bar 5's is the only one
    // since, so no whole bar.
    assert!(r.l.capture_take(1).is_none(), "not across the stop");
    r.run(r.frame + BAR);
    assert_eq!(r.l.capture_take(4).map(|t| t.bars), Some(1));
}

#[test]
fn a_new_input_forgets_what_was_heard_before() {
    let mut r = played(64, 3 * BAR + 6_000);
    assert_eq!(r.l.capture_take(2).map(|t| t.bars), Some(2));
    r.cmd(L::NewInput);
    r.run(r.frame + 64);
    assert!(r.l.capture_take(2).is_none());
    // Bars 5 and 6 played since: one whole bar after the change.
    r.run(5 * BAR + 100);
    let t = r.l.capture_take(2).expect("a bar");
    assert_eq!((t.bars, t.origin), (1, 4 * BAR_TICKS));
}

#[test]
fn undo_or_capture_again_calls_it_off() {
    // Waiting for its bar line: Undo.
    let mut r = played(64, 2 * BAR + 17_000);
    r.run(r.frame + 64);
    r.cmd(L::Capture(2));
    r.run(r.frame + 64);
    assert_eq!(r.l.status().capturing, Some(0.0));
    r.cmd(L::Undo);
    r.run(4 * BAR);
    let s = r.l.status();
    assert_eq!((s.phase, s.capturing), (Phase::Empty, None));
    // Copying in: Capture again; its memory goes back.
    r.cmd(L::Capture(2));
    r.run(r.frame + 128);
    assert!(r.l.status().capturing.is_some_and(|p| p > 0.0 && p < 1.0));
    assert!(r.l.status().used > 0);
    r.cmd(L::Capture(2));
    r.run(r.frame + BAR);
    let s = r.l.status();
    assert_eq!((s.phase, s.capturing, s.used), (Phase::Empty, None, 0));
    // Refused into a loop with sound.
    r.cmd(L::Capture(1));
    r.until_playing(BAR);
    let refused = r.l.stats().refused;
    r.cmd(L::Capture(1));
    r.run(r.frame + 128);
    assert_eq!(r.l.stats().refused, refused + 1);
    assert_eq!(r.l.status().capturing, None);
}

/// The output is the same frame for frame whatever the period size.
#[test]
fn the_period_size_changes_nothing() {
    let go = |period: usize| {
        let mut r = played(period, 2 * BAR + 17_000);
        r.cmd(L::Capture(2));
        r.run(3 * BAR + 6_000);
        assert!(r.l.select_loop(1));
        r.cmd(L::SetLatency(200));
        r.cmd(L::Capture(1));
        r.run(6 * BAR);
        r.out
    };
    let a = go(64);
    assert!(a.iter().any(|x| x.abs() > 0.1), "the captures play");
    for period in [1, 1000] {
        assert_eq!(go(period), a, "period {period}");
    }
}

#[test]
fn capture_to_takes_bars_ending_on_any_bar_line_still_kept() {
    // Bar 9, beat 2: bars 3 to 5 (ending on bar 6's line, tick 5 bars),
    // long gone from "the last bars", are still kept.
    let mut r = played(64, 8 * BAR + 6_000);
    let end = 5 * BAR_TICKS;
    let t = r.l.capture_take_to(end, 3).expect("kept");
    assert_eq!(
        (t.bars, t.origin, t.ticks),
        (3, 2 * BAR_TICKS, 3 * BAR_TICKS)
    );
    r.cmd(L::CaptureTo(end, 3));
    r.run(r.frame + 64);
    assert!(r.l.status().capturing.is_some(), "copying in");
    r.until_playing(2 * BAR);
    let s = r.l.status();
    assert_eq!(
        (s.frames, s.ticks, s.origin),
        (3 * BAR as usize, 3 * BAR_TICKS, 2 * BAR_TICKS)
    );
    let a = r.layer();
    for (j, x) in a[..a.len() - FADE].iter().enumerate() {
        assert_eq!(*x, heard(2 * BAR + j as u64), "frame {j}");
    }
    // A bar line never heard (or no longer kept) is refused.
    r.cmd(L::Clear);
    r.run(r.frame + 2 * FADE as u64);
    assert_eq!(r.l.capture_take_to(BAR_TICKS / 2, 1), None);
    let refused = r.l.stats().refused;
    r.cmd(L::CaptureTo(BAR_TICKS / 2, 1));
    r.run(r.frame + 64);
    assert_eq!(r.l.status().capturing, None);
    assert_eq!(r.l.stats().refused, refused + 1);
}
