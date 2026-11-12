//! Prints and fits against a real transport: a loop recorded at one tempo
//! plays at another with its pitch kept once its print is in, stops
//! playing it when its layers change, joins up at its loop point, fits
//! itself to the tempo when overdubbed (undone and redone with a dip), and
//! gives the same output for any period size.
//!
//! 22.05 kHz at 240 BPM in 4/4 makes a bar exactly 22 050 frames (1 s); at
//! 200 BPM it is 26 460.

use mpfree_looper::print::{frames_at, print};
use mpfree_looper::{Command as L, LayerCommand as Lc, Looper, Phase, Print, Step, FADE};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: usize = 22_050;
const BAR_TICKS: i64 = 3_840;
const SLOW_BAR: usize = 26_460;

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

    /// Cycles a second of the left channel in `a..b` (rising zero
    /// crossings).
    fn pitch(&self, a: u64, b: u64) -> f64 {
        let ups = (a + 1..b)
            .filter(|&f| self.left(f - 1) < 0.0 && self.left(f) >= 0.0)
            .count();
        ups as f64 * f64::from(RATE) / (b - a) as f64
    }

    /// The biggest jump between neighbouring output frames in `a..b`.
    fn max_step(&self, a: u64, b: u64) -> f32 {
        (a + 1..b)
            .map(|f| {
                let (l0, l1) = (self.out[2 * (f - 1) as usize], self.out[2 * f as usize]);
                let (r0, r1) = (
                    self.out[2 * (f - 1) as usize + 1],
                    self.out[2 * f as usize + 1],
                );
                (l1 - l0).abs().max((r1 - r0).abs())
            })
            .fold(0.0, f32::max)
    }

    /// Bounce loop 0 for a print and make it for `bpm`.
    fn make_print(&mut self, bpm: f64) -> Print {
        let s = self.l.loop_status(0).unwrap();
        self.l
            .bounce_print(0, Vec::with_capacity(2 * s.frames))
            .unwrap();
        let to = self.frame + s.frames as u64;
        while self.l.bounce_progress().is_some_and(|p| p < 1.0) && self.frame < to {
            let f = self.frame + self.period as u64;
            self.run(f, &silence);
        }
        let b = self.l.take_bounce().expect("bounced");
        assert!(b.dry && b.complete);
        assert_eq!(b.version, s.version);
        print(&b, RATE, frames_at(BAR_TICKS, bpm, RATE)).unwrap()
    }
}

/// 441 Hz, 50 frames a cycle: a bar holds 441 whole cycles.
fn sine(f: u64) -> [f32; 2] {
    let x = 0.5 * (f as f64 * std::f64::consts::TAU / 50.0).sin() as f32;
    [x, x]
}

fn silence(_: u64) -> [f32; 2] {
    [0.0; 2]
}

fn noise(f: u64) -> [f32; 2] {
    let h = |s: u64| {
        let mut x = f.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(s);
        x ^= x >> 29;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 32;
        (x % 10_000) as f32 / 10_000.0 - 0.5
    };
    [0.2 * h(1), 0.2 * h(2)]
}

/// A one-bar loop of the sine from bar 1, then the tempo down to 200 BPM.
/// Returns the frame where the slower tempo has surely begun.
fn recorded(r: &mut Rig) -> u64 {
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    assert!(r.t.command(0, T::Play));
    r.run(BAR as u64 + 512, &sine);
    assert_eq!(r.l.status().phase, Phase::Playing);
    assert_eq!(r.l.status().frames, BAR);
    assert!(r.t.command(r.frame, T::SetTempo(200.0)));
    r.run(r.frame + 4096, &silence);
    r.frame
}

#[test]
fn a_print_plays_at_another_tempo_with_its_pitch_kept() {
    let mut r = Rig::new(128);
    let slow = recorded(&mut r);
    // Like tape until the print is in: 441 Hz at 200/240 of the speed.
    r.run(slow + 8192, &silence);
    let tape = r.pitch(slow, slow + 8192);
    assert!((tape - 367.5).abs() < 4.0, "tape {tape}");
    assert!(!r.l.status().printed);
    let p = r.make_print(200.0);
    assert_eq!(p.frames, SLOW_BAR);
    r.l.print(0, Some(p)).unwrap();
    let from = r.frame + 2 * FADE as u64;
    r.run(from + 2 * SLOW_BAR as u64, &silence);
    assert!(r.l.status().printed);
    let kept = r.pitch(from, from + SLOW_BAR as u64);
    assert!((kept - 441.0).abs() < 3.0, "print {kept}");
    // It loops every 26 460 frames, sample for sample, and joins up with
    // no click at its loop point.
    for f in from..from + 2000 {
        let (a, b) = (r.left(f), r.left(f + SLOW_BAR as u64));
        assert!((a - b).abs() < 1e-4, "{f}: {a} then {b}");
    }
    let step = r.max_step(from, from + 2 * SLOW_BAR as u64);
    assert!(step < 0.08, "biggest step {step}");
    // Taking it away fades back to tape; the old one waits to be dropped.
    r.l.print(0, None).unwrap();
    r.run(r.frame + 2 * FADE as u64 + 128, &silence);
    assert!(!r.l.status().printed);
    assert!(r.l.take_print(0).is_some());
}

#[test]
fn a_change_to_the_layers_stops_the_print() {
    let mut r = Rig::new(64);
    recorded(&mut r);
    let p = r.make_print(200.0);
    r.l.print(0, Some(p)).unwrap();
    r.run(r.frame + 4 * FADE as u64, &silence);
    assert!(r.l.status().printed);
    let v = r.l.status().version;
    // The loop's level and mute leave the layers as they are.
    r.cmd(L::SetLevel(0.5));
    r.run(r.frame + 64, &silence);
    assert_eq!(r.l.status().version, v);
    assert!(r.l.status().printed);
    // A layer's level does not.
    r.l.layer_command(0, Lc::Level(0.8));
    r.run(r.frame + 2 * FADE as u64 + 64, &silence);
    assert_ne!(r.l.status().version, v);
    assert!(!r.l.status().printed);
    // A print from before is no use; one made now plays.
    let p = r.make_print(200.0);
    r.l.print(0, Some(p)).unwrap();
    r.run(r.frame + 4 * FADE as u64, &silence);
    assert!(r.l.status().printed);
    assert!(
        r.l.take_print(0).is_some(),
        "the stale one waits to be dropped"
    );
}

/// The fit scenario at one period size: print in, Overdub (which fits the
/// loop, then records a bar of noise), then Undo, Undo (the fit), Redo.
fn fit_session(period: usize) -> (Rig, Vec<u64>) {
    let mut r = Rig::new(period);
    recorded(&mut r);
    // Commands go in on frames every period size here divides.
    let next = |f: u64| f.div_ceil(4096) * 4096;
    let p = r.make_print(200.0);
    let f = next(r.frame);
    r.run(f, &silence);
    r.l.print(0, Some(p)).unwrap();
    let mut marks = Vec::new();
    let f = next(r.frame + 2 * SLOW_BAR as u64);
    r.run(f, &silence);
    assert!(r.l.status().printed);
    marks.push(r.frame);
    r.cmd(L::Overdub);
    if period < 400 {
        let f = r.frame + period as u64;
        r.run(f, &silence);
        let s = r.l.status();
        assert!(s.fitting.is_some() && s.overdub_next, "{s:?}");
    }
    // The fit copies at 64x: done in about 414 frames.
    let f = next(r.frame + 1024);
    r.run(f, &silence);
    let s = r.l.status();
    assert_eq!(s.fitting, None);
    assert_eq!(s.phase, Phase::Overdubbing);
    assert_eq!(s.frames, SLOW_BAR);
    marks.push(r.frame);
    let f = next(r.frame + SLOW_BAR as u64);
    r.run(f, &noise);
    r.cmd(L::Overdub);
    let f = next(r.frame + 4 * FADE as u64);
    r.run(f, &silence);
    let h = r.l.history();
    assert!(
        matches!(h.done(), [Step::Fit(_), Step::Overdub(_)]),
        "{h:?}"
    );
    marks.push(r.frame);
    // Undo the overdub, then the fit: the loop dips and is back as it was.
    r.cmd(L::Undo);
    let f = next(r.frame + 4 * FADE as u64);
    r.run(f, &silence);
    r.cmd(L::Undo);
    let f = next(r.frame + 4 * FADE as u64);
    r.run(f, &silence);
    let s = r.l.status();
    assert_eq!(s.frames, BAR);
    assert_eq!(s.layers, 1);
    assert!(!s.printed, "the print was for the loop before the fit");
    marks.push(r.frame);
    r.cmd(L::Redo);
    let f = next(r.frame + 4 * FADE as u64);
    r.run(f, &silence);
    assert_eq!(r.l.status().frames, SLOW_BAR);
    marks.push(r.frame);
    (r, marks)
}

#[test]
fn overdub_over_a_print_fits_the_loop_first_and_undo_takes_the_fit_back() {
    let (mut r, m) = fit_session(128);
    // No click where the fitted layer takes over from the print (they are
    // the same audio) or where the overdub starts.
    let step = r.max_step(m[0], m[1]);
    assert!(step < 0.08, "biggest step {step}");
    // After Redo the fitted layer plays the print's 441 Hz again, sample
    // for sample (the bar of noise stays undone).
    r.run(r.frame + 8192, &silence);
    let kept = r.pitch(m[4], m[4] + 8192);
    assert!((kept - 441.0).abs() < 3.0, "fitted {kept}");
}

#[test]
fn prints_and_fits_give_the_same_output_for_any_period_size() {
    let (want, wm) = fit_session(1);
    for period in [64, 128, 1024] {
        let (got, gm) = fit_session(period);
        assert_eq!(gm, wm, "period {period}");
        assert_eq!(got.out.len(), want.out.len());
        for (i, (a, b)) in got.out.iter().zip(&want.out).enumerate() {
            assert_eq!(a.to_bits(), b.to_bits(), "period {period}, sample {i}");
        }
    }
}

#[test]
fn a_print_can_only_be_handed_in_once_the_last_one_took_over() {
    let mut r = Rig::new(128);
    recorded(&mut r);
    let p = r.make_print(200.0);
    let q = Print {
        audio: p.audio.clone(),
        frames: p.frames,
        ticks: p.ticks,
        origin: p.origin,
        version: p.version,
    };
    r.l.print(0, Some(p)).unwrap();
    // Still waiting for the next period: the second comes back.
    assert!(r.l.print(0, Some(q)).is_err());
    let bad = Print {
        audio: vec![0.0; 10],
        frames: 100,
        ticks: BAR_TICKS,
        origin: 0,
        version: 0,
    };
    r.run(r.frame + 128, &silence);
    assert!(r.l.print(0, Some(bad)).is_err(), "shorter than its frames");
    assert!(r.l.print(9, None).is_err());
}
