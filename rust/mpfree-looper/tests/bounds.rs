//! A first pass keeps what was heard around its loop ([`KEEP_BEFORE`]
//! frames before its start, [`KEEP_AFTER`] after its end), and the loop
//! is a region of that audio: its loop point is a seam it plays, never
//! written into the audio, and its boundaries move as undoable edits
//! (Move, Resize, Turn) within what it kept. Saving keeps the loop as it
//! sounds, as before; a loop opened from a save keeps nothing around it.
//!
//! 22.05 kHz at 240 BPM in 4/4 makes a bar exactly 22 050 frames (1 s).

use mpfree_looper::{
    Bounds, Command as L, Edit as E, LoopFile, Looper, Phase, Step, BLOCK, FADE, KEEP_AFTER,
    KEEP_BEFORE, LOAD_SPEED,
};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: usize = 22_050;
const BAR_TICKS: i64 = 3_840;
/// The song starts here, the first pass with it: what was heard before
/// it is what the pass keeps before its start.
const START: u64 = 20_000;
const KB: i64 = KEEP_BEFORE as i64;
const KA: i64 = KEEP_AFTER as i64;

struct Rig {
    t: Transport,
    l: Looper,
    b: Block,
    frame: u64,
    input: Vec<f32>,
    mix: Vec<f32>,
    out: Vec<f32>,
}

impl Rig {
    fn new(seconds: u32) -> Rig {
        Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            l: Looper::new(RATE, seconds),
            b: Block::new(),
            frame: 0,
            input: vec![0.0; 256],
            mix: vec![0.0; 256],
            out: Vec::new(),
        }
    }

    fn cmd(&mut self, c: L) {
        assert!(self.l.command(c));
    }

    fn run(&mut self, to: u64, input: &dyn Fn(u64) -> [f32; 2]) {
        while self.frame < to {
            let n = 128.min(to - self.frame) as usize;
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

    /// Run a bar more of `noise` (so the input goes on), then check the
    /// bar after that against `want(p)`, p being the loop frame (the loop
    /// is `len` frames from bar 1 at START); None where it doesn't check.
    fn check(&mut self, len: u64, want: &dyn Fn(u64) -> Option<[f32; 2]>) -> usize {
        let from = self.frame + BAR as u64;
        self.run(from + BAR as u64, &noise);
        let mut checked = 0;
        for f in from..from + BAR as u64 {
            let p = (f - START) % len;
            if let Some(w) = want(p) {
                let y = [self.out[2 * f as usize], self.out[2 * f as usize + 1]];
                assert!(
                    (y[0] - w[0]).abs() < 1e-4 && (y[1] - w[1]).abs() < 1e-4,
                    "frame {f} (loop frame {p}): {y:?} against {w:?}"
                );
                checked += 1;
            }
        }
        checked
    }

    fn bounds(&self) -> Option<Bounds> {
        self.l.status().bounds
    }

    fn refused(&self) -> u64 {
        self.l.stats().refused
    }

    /// An edit, run until its crossfade is over.
    fn edit(&mut self, e: E) {
        self.cmd(L::Edit(e));
        self.run(self.frame + 4 * FADE as u64, &noise);
    }
}

/// Deterministic noise, different on each channel; heard all along.
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

/// What was heard at loop frame `a` of the pass (negative: before it).
fn heard(a: i64) -> [f32; 2] {
    noise((START as i64 + a) as u64)
}

/// A one-bar first pass from START, the input going on after it until
/// what the pass keeps after its end is in.
fn recorded(seconds: u32) -> Rig {
    let mut r = Rig::new(seconds);
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    assert!(r.t.command(START, T::Play));
    r.run(START + BAR as u64 + 1, &noise);
    let s = r.l.status();
    assert_eq!((s.phase, s.frames, s.layers), (Phase::Playing, BAR, 1));
    r.run(START + (BAR + KEEP_AFTER) as u64 + 1_000, &noise);
    r
}

#[test]
fn the_first_pass_keeps_what_was_heard_around_its_loop() {
    let mut r = Rig::new(10);
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    assert!(r.t.command(START, T::Play));
    r.run(START + BAR as u64 + 1_000, &noise);
    // Just after the end: what it keeps after its end is coming in, and
    // its memory is taken already.
    let b = r.bounds().unwrap();
    assert_eq!(
        (b.src, b.start, b.captured),
        (KB, KB, (KB, KB + BAR as i64))
    );
    assert!(b.kept > KB + BAR as i64 && b.kept < KB + BAR as i64 + KA);
    let all = (KEEP_BEFORE + BAR + KEEP_AFTER).div_ceil(BLOCK) * BLOCK;
    assert_eq!(r.l.memory().used, all);
    r.run(START + (BAR + KEEP_AFTER) as u64 + 1_000, &noise);
    assert_eq!(r.bounds().unwrap().kept, KB + BAR as i64 + KA);
    assert_eq!(r.l.memory().used, all);
    assert_eq!(r.l.stats(), Default::default());
}

#[test]
fn the_loop_point_is_a_seam_the_loop_plays_not_written_in() {
    let mut r = recorded(10);
    // The audio is as heard, to the end: nothing was written at the loop
    // point.
    let w = r.l.layer(0).unwrap();
    for p in [0, BAR / 2, BAR - FADE, BAR - 1] {
        assert_eq!(
            (w[2 * p], w[2 * p + 1]),
            (heard(p as i64)[0], heard(p as i64)[1])
        );
    }
    // It plays as heard, but for the seam: over its last FADE frames the
    // end fades into what was heard just before the start.
    let bar = BAR as u64;
    let edge = bar - FADE as u64;
    let n = r.check(bar, &|p| {
        if p == bar - 1 {
            // Read between the last frame and the first.
            return None;
        }
        Some(if p < edge {
            heard(p as i64)
        } else {
            let k = (p - edge) as i64;
            let w = (k as f32 + 0.5) / FADE as f32;
            let (a, b) = (heard(p as i64), heard(k - FADE as i64));
            [a[0] * (1.0 - w) + b[0] * w, a[1] * (1.0 - w) + b[1] * w]
        })
    });
    assert_eq!(n, BAR - 1);
    // So the first loop's seam can be set like any other: a straight cut,
    // or a long fade into what came before the start.
    r.edit(E::SeamFade(0));
    r.check(bar, &|p| (p < bar - 1).then(|| heard(p as i64)));
    r.edit(E::SeamFade(4_000));
    r.check(bar, &|p| {
        let edge = bar - 4_000;
        (p < bar - 1).then(|| {
            if p < edge {
                heard(p as i64)
            } else {
                let k = (p - edge) as i64;
                let w = (k as f32 + 0.5) / 4_000.0;
                let (a, b) = (heard(p as i64), heard(k - 4_000));
                [a[0] * (1.0 - w) + b[0] * w, a[1] * (1.0 - w) + b[1] * w]
            }
        })
    });
    assert_eq!(r.refused(), 0);
}

#[test]
fn move_resize_and_turn_are_edits_with_undo() {
    let mut r = recorded(10);
    let bar = BAR as u64;
    let as_recorded = r.bounds().unwrap();
    // Move: both boundaries later; the end is what was heard after the
    // pass, as played.
    r.edit(E::Move(500));
    let b = r.bounds().unwrap();
    assert_eq!(
        (b.src, b.start, b.captured),
        (KB + 500, KB + 500, as_recorded.captured)
    );
    assert_eq!(r.l.status().frames, BAR);
    assert_eq!(r.l.history().done(), &[Step::Edit(E::Move(500))]);
    let n = r.check(bar, &|p| {
        (p < bar - FADE as u64 - 1).then(|| heard(500 + p as i64))
    });
    assert!(n > 21_000);
    // Undo: as recorded.
    r.cmd(L::Undo);
    r.run(r.frame + 4 * FADE as u64, &noise);
    assert_eq!(r.bounds(), Some(as_recorded));
    r.check(bar, &|p| {
        (p < bar - FADE as u64 - 1).then(|| heard(p as i64))
    });
    // Turn: bar 1 falls 1 000 frames in; the first 1 000 play at the end,
    // leading into it, as a pickup does.
    r.edit(E::Turn(1_000));
    let b = r.bounds().unwrap();
    assert_eq!((b.src, b.start), (KB + 1_000, KB));
    assert_eq!(r.l.status().lead, 1_000);
    let cut = bar - 1_000;
    r.check(bar, &|p| {
        let plain = (1..cut - FADE as u64 - 1).contains(&p) || (cut + 1..bar - 1).contains(&p);
        plain.then(|| heard(((p + 1_000) % bar) as i64))
    });
    r.cmd(L::Undo);
    r.run(r.frame + 4 * FADE as u64, &noise);
    // Resize: the end earlier, a tenth of a bar shorter; the bars stay, so
    // it plays faster until the tempo goes with it.
    r.edit(E::Resize(-2_205));
    let s = r.l.status();
    assert_eq!((s.frames, s.ticks), (BAR - 2_205, BAR_TICKS));
    assert_eq!(s.bounds.map(|b| (b.src, b.start)), Some((KB, KB)));
    r.cmd(L::Undo);
    r.run(r.frame + 4 * FADE as u64, &noise);
    assert_eq!(r.l.status().frames, BAR);
    assert_eq!(r.refused(), 0);
    // Refused: outside what was kept, shorter than a 1/16 note, or no
    // change.
    for e in [
        E::Move(-KB - 1),
        E::Move(KA + 1),
        E::Resize(KA + 1),
        E::Resize(-(BAR as i64) + 1_000),
        E::Move(0),
        E::Turn(BAR as i64),
    ] {
        let before = r.refused();
        r.edit(e);
        assert_eq!(r.refused(), before + 1, "{e:?}");
    }
    // The furthest it can go.
    r.edit(E::Move(-KB));
    r.edit(E::Move(KB + KA));
    assert_eq!(r.bounds().unwrap().start, KB + KA);
}

#[test]
fn the_bounds_move_only_while_the_first_pass_plays_alone() {
    let mut r = recorded(10);
    // An overdub: a second layer, so no bounds.
    r.cmd(L::Overdub);
    r.run(r.frame + 3_000, &noise);
    r.cmd(L::Overdub);
    r.run(r.frame + 4 * FADE as u64, &noise);
    assert_eq!((r.l.status().layers, r.bounds()), (2, None));
    let before = r.refused();
    r.edit(E::Move(100));
    assert_eq!(r.refused(), before + 1);
    // Undo takes the overdub away: they move again.
    r.cmd(L::Undo);
    r.run(r.frame + 4 * FADE as u64, &noise);
    assert!(r.bounds().is_some());
    // Another edit changes what the layer plays: none, until it is undone.
    r.edit(E::Half);
    assert_eq!(r.bounds(), None);
    r.cmd(L::Undo);
    r.run(r.frame + 4 * FADE as u64, &noise);
    assert!(r.bounds().is_some());
    // A seam fade or a speed leaves them.
    r.edit(E::SeamFade(1_000));
    r.edit(E::Speed(-1));
    assert!(r.bounds().is_some());
}

#[test]
fn stop_keeps_nothing_after_it() {
    let mut r = Rig::new(10);
    r.cmd(L::SetBars(None));
    r.cmd(L::Record);
    assert!(r.t.command(START, T::Play));
    let stop = START + BAR as u64 + 5_000;
    assert!(r.t.command(stop, T::Stop));
    r.run(stop + (KEEP_AFTER + 2_000) as u64, &noise);
    let s = r.l.status();
    assert_eq!((s.frames, s.cut), (2 * BAR, Some(BAR + 5_000)));
    let b = s.bounds.unwrap();
    assert_eq!(
        (b.captured, b.kept),
        ((KB, KB + 2 * BAR as i64), KB + BAR as i64 + 5_000)
    );
    assert_eq!(r.l.stats().kept_short, 0);
}

#[test]
fn with_no_memory_for_what_follows_the_loop_keeps_nothing_after_it() {
    // 1 second: the bar and what came before it, but not what follows.
    let mut r = recorded(1);
    let b = r.bounds().unwrap();
    assert_eq!(b.kept, KB + BAR as i64);
    assert_eq!((r.l.stats().kept_short, r.l.stats().aborted), (1, 0));
    // Its end can only move earlier.
    let before = r.refused();
    r.edit(E::Resize(100));
    r.edit(E::Resize(-100));
    assert_eq!(r.refused(), before + 1);
    assert_eq!(r.l.status().frames, BAR - 100);
    // It plays as heard.
    r.cmd(L::Undo);
    let bar = BAR as u64;
    r.check(bar, &|p| {
        (p < bar - FADE as u64 - 1).then(|| heard(p as i64))
    });
}

/// Loop 0 as a project saves it: its file with each layer's audio.
fn saved(r: &Rig) -> LoopFile {
    let mut f = r.l.loop_file(0).expect("savable");
    for (k, y) in f.layers.iter_mut().enumerate() {
        y.audio = vec![0.0; 2 * f.frames];
        assert!(r.l.read_layer(0, k, 0, &mut y.audio).is_some());
    }
    f
}

#[test]
fn saving_keeps_the_loop_as_it_sounds_and_a_reopened_loop_keeps_nothing_around_it() {
    let mut a = recorded(10);
    // Saved as before: the audio as heard, and over its last FADE frames
    // the loop point's fade into what was heard just before the start.
    let f = saved(&a);
    let x = &f.layers[0].audio;
    for p in 0..BAR {
        let want = if p < BAR - FADE {
            heard(p as i64)
        } else {
            let k = (p - (BAR - FADE)) as i64;
            let w = (k as f32 + 0.5) / FADE as f32;
            let (u, v) = (heard(p as i64), heard(k - FADE as i64));
            [u[0] * (1.0 - w) + v[0] * w, u[1] * (1.0 - w) + v[1] * w]
        };
        assert!(
            (x[2 * p] - want[0]).abs() < 1e-6 && (x[2 * p + 1] - want[1]).abs() < 1e-6,
            "frame {p}"
        );
    }
    // Moved and turned in what it kept, then saved and opened again.
    a.edit(E::Move(700));
    a.edit(E::Turn(-3_000));
    let f = saved(&a);
    let mut b = Rig::new(10);
    assert!(b.t.command(0, T::Play));
    b.l.load(0, f).unwrap();
    let copy = BAR.div_ceil(LOAD_SPEED) as u64;
    b.run(copy + 1_000, &|_| [0.0; 2]);
    let s = b.l.status();
    assert_eq!((s.phase, s.frames, s.bounds), (Phase::Playing, BAR, None));
    assert_eq!(
        s.used,
        BAR.div_ceil(BLOCK) * BLOCK,
        "nothing kept around it"
    );
    // It sounds as it did: the same bar of the song plays the same.
    let end = a.frame + 3 * BAR as u64;
    a.run(end, &|_| [0.0; 2]);
    b.run(end - START, &|_| [0.0; 2]);
    let worst = (end - BAR as u64..end)
        .flat_map(|f| {
            let g = (f - START) as usize;
            [
                (a.out[2 * f as usize] - b.out[2 * g]).abs(),
                (a.out[2 * f as usize + 1] - b.out[2 * g + 1]).abs(),
            ]
        })
        .fold(0.0, f32::max);
    assert!(worst < 1e-5, "differs by {worst}");
    assert!(b.out.iter().any(|&v| v.abs() > 0.1));
}
