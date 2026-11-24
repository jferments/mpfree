//! The looper's recording rule, from the owner's looper review of 23 Nov
//! (Finding A): **a sample is heard only from a region that was recorded;
//! moving a play position never makes unwritten audio valid.** The
//! looper's memory is a fixed pool of blocks used again and again, so a
//! block a layer is given still holds what the last layer to use it left
//! there. Here that is always poison: negative audio, where everything
//! the musician plays is positive (DC, so its fades stay positive too).
//! Random presses with the song changing tempo, locating, looping,
//! stopping and starting under the loops, the loops' own speed, edits,
//! latency, undo and redo, merges and first passes cut short must never
//! bring any of it back: not in what plays, not in any layer.
//!
//! 22.05 kHz at 240 BPM in 4/4 makes a bar exactly 22 050 frames (1 s).

use mpfree_looper::{Command as L, Edit as E, Looper, Overdubs, Phase, FADE, KEEP_AFTER};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: u64 = 22_050;
const BAR_TICKS: i64 = 3_840;
/// The transport starts playing here.
const START: u64 = 1_000;
/// What the stale blocks hold.
const POISON: f32 = -0.5;
/// The most latency the presses set.
const MAX_LATENCY: u64 = 3_000;
/// Anything heard below this is stale audio (what is played is DC of
/// 0.02 or more, and the playback's interpolation can dip a hair below 0
/// at a fade's foot, never this far).
const FLOOR: f32 = -0.02;

struct Rig {
    t: Transport,
    l: Looper,
    b: Block,
    period: usize,
    frame: u64,
    input: Vec<f32>,
    mix: Vec<f32>,
    /// The lowest sample heard since `watch`, and its frame.
    lowest: (f32, u64),
    /// Frames heard while watched.
    watched: u64,
    watching: bool,
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
            lowest: (0.0, 0),
            watched: 0,
            watching: false,
        }
    }

    fn at(&mut self, c: T) {
        assert!(self.t.command(self.frame, c));
    }

    fn cmd(&mut self, c: L) {
        assert!(self.l.command(c));
    }

    fn select(&mut self, n: usize) {
        assert!(self.l.select_loop(n));
    }

    /// Run for `frames`, feeding DC of `x` on both channels.
    fn run(&mut self, frames: u64, x: f32) {
        let to = self.frame + frames;
        while self.frame < to {
            let n = (self.period as u64).min(to - self.frame) as usize;
            self.input[..2 * n].fill(x);
            self.mix[..2 * n].fill(0.0);
            assert!(self.t.process(n as u32, &mut self.b));
            self.l
                .process(&self.b, &self.input[..2 * n], &mut self.mix[..2 * n]);
            if self.watching {
                for (i, v) in self.mix[..2 * n].iter().enumerate() {
                    if *v < self.lowest.0 {
                        self.lowest = (*v, self.frame + i as u64 / 2);
                    }
                }
                self.watched += n as u64;
            }
            self.frame += n as u64;
        }
    }

    /// From here on, nothing below [`FLOOR`] may be heard.
    fn watch(&mut self) {
        self.watching = true;
        self.lowest = (0.0, self.frame);
    }

    fn unwatch(&mut self) {
        self.watching = false;
    }

    /// Nothing stale heard, and none in any layer of loops 1 and 2.
    fn clean(&self, what: &str) {
        assert!(
            self.lowest.0 >= FLOOR,
            "{what}: {} heard on frame {}",
            self.lowest.0,
            self.lowest.1
        );
        for n in 0..2 {
            let layers = self.l.loop_status(n).unwrap().layers;
            for k in 0..layers {
                let v = self.l.loop_layer(n, k).unwrap();
                let mut p = vec![[0.0f32; 2]; v.len() / 2];
                v.read(0, &mut p);
                if let Some(q) = p.iter().position(|x| x[0] < FLOOR || x[1] < FLOOR) {
                    panic!("{what}: loop {} layer {k} holds {:?} at {q}", n + 1, p[q]);
                }
            }
        }
    }
}

/// A random number source that gives the same run every time.
struct Draw(u64);

impl Draw {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A whole number in `0..n`.
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// A number in `a..b`.
    fn within(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Some frames: `a` to `b` bars at 240 BPM.
    fn bars(&mut self, a: f64, b: f64) -> u64 {
        (self.within(a, b) * BAR as f64) as u64
    }
}

/// Loop 1's layers' ids, in play order.
fn ids(r: &Rig) -> Vec<u8> {
    (0..)
        .map_while(|k| r.l.layer_info(k))
        .map(|i| i.id)
        .collect()
}

/// Every free block of the memory poisoned: a long first pass of poison on
/// loop 4, cleared and forgotten.
fn poison_the_memory(r: &mut Rig, seconds: u32) {
    r.select(3);
    r.cmd(L::SetBars(None));
    r.cmd(L::Record);
    r.run(BAR + 10, POISON);
    r.run(BAR * u64::from(seconds) * 3 / 4, POISON);
    r.cmd(L::Record);
    r.run(2 * BAR, 0.0);
    assert_eq!(r.l.loop_status(3).unwrap().phase, Phase::Playing);
    r.cmd(L::Clear);
    r.run(4 * FADE as u64, 0.0);
    r.cmd(L::Forget);
    r.run(4 * FADE as u64, 0.0);
    assert_eq!(r.l.loop_status(3).unwrap().phase, Phase::Empty);
    r.select(0);
}

/// Loop 1 has sound to overdub: if it is empty (undone or never
/// recorded), a first pass of 1 or 2 bars, on the song playing.
fn loop_one_plays(r: &mut Rig, d: &mut Draw) {
    r.select(0);
    if r.l.loop_status(0).unwrap().phase != Phase::Empty {
        return;
    }
    r.cmd(L::SetBars(Some(1 + d.below(2) as u32)));
    r.cmd(L::Record);
    let end = r.frame + 40 * BAR;
    while r.l.loop_status(0).unwrap().phase != Phase::Playing && r.frame < end {
        r.run(BAR / 8, 0.25);
    }
    assert_eq!(
        r.l.loop_status(0).unwrap().phase,
        Phase::Playing,
        "a base for loop 1"
    );
}

/// Every writer done before poison comes in: loop 1's overdub ended (or
/// one waiting called off) and the tails written with what the musician
/// plays (a writer takes the input for `latency` frames after a press, as
/// it belongs before it, and a first pass keeps what it hears after its
/// end); loop 2 let go of or closed. Poison these took would be recorded,
/// not stale. True once they are all out of the way (a pass closing on a
/// slow bar line can take longer: the round is skipped).
fn settle(r: &mut Rig) -> bool {
    let one = r.l.loop_status(0).unwrap();
    if one.phase == Phase::Overdubbing || one.overdub_next {
        r.select(0);
        r.cmd(L::Overdub);
    }
    // Loop 2 waiting to record lets go; recording, it closes.
    if matches!(
        r.l.loop_status(1).unwrap().phase,
        Phase::Armed | Phase::Recording
    ) {
        r.select(1);
        r.cmd(L::Record);
        r.select(0);
    }
    r.run(2 * KEEP_AFTER as u64 + MAX_LATENCY + 8 * FADE as u64, 0.05);
    let one = r.l.loop_status(0).unwrap();
    let two = r.l.loop_status(1).unwrap().phase;
    one.phase == Phase::Playing
        && !one.overdub_next
        && !matches!(two, Phase::Armed | Phase::Recording)
}

/// The song playing.
fn song_plays(r: &mut Rig) {
    if !r.t.state().rolling() {
        r.at(T::Play);
        r.run(64, 0.0);
    }
}

/// Random rounds, each: an overdub of poison on loop 1, undone (its blocks
/// held for Redo), then an overdub that takes those blocks, and random
/// presses round it, every one heard and every layer read for poison.
fn rounds(seed: u64, rounds: u32, period: usize) -> (u64, u32, u32) {
    const SECONDS: u32 = 40;
    let mut d = Draw(seed);
    let mut r = Rig::new(SECONDS, period);
    r.at(T::Play);
    r.run(START, 0.0);
    poison_the_memory(&mut r, SECONDS);
    let (mut overdubs, mut cut_short, mut skipped) = (0, 0, 0);
    for round in 0..rounds {
        song_plays(&mut r);
        loop_one_plays(&mut r, &mut d);
        // Loop 1 keeps room for the round's layers.
        if r.l.status().layers > 10 {
            r.cmd(L::Merge);
            r.run(BAR, 0.0);
        }
        if !settle(&mut r) {
            skipped += 1;
            continue;
        }
        // Poison overdubbed and undone: every layer of it (the song's
        // loop going round under an overdub that has not gone round the
        // loop yet carries it on in a layer of its own).
        r.unwatch();
        let before = ids(&r);
        r.cmd(L::SetOverdubs(Overdubs::Add));
        r.cmd(L::Overdub);
        r.run(d.bars(0.3, 2.5), POISON);
        if r.l.status().phase == Phase::Overdubbing {
            r.cmd(L::Overdub);
        }
        r.run(4 * FADE as u64, 0.0);
        // (Refused, with the layers full, there is none to undo.)
        for _ in 0..8 {
            if ids(&r).iter().all(|i| before.contains(i)) {
                break;
            }
            r.cmd(L::Undo);
            r.run(4 * FADE as u64, 0.0);
        }
        assert_eq!(ids(&r), before, "the poison undone");
        let what = |w: &str| format!("seed {seed:#x}, round {round}: {w}");
        r.watch();
        // The overdub that takes the poisoned blocks, under one of the
        // moves that used to make it play what they held.
        match d.below(5) {
            0 => r.at(T::TempoNow(d.within(40.0, 400.0))),
            1 => r.at(T::Locate(d.below(16) as i64 * BAR_TICKS / 4)),
            2 => r.cmd(L::SetLatency(d.below(MAX_LATENCY) as u32)),
            3 => r.cmd(L::Edit(E::Speed(d.below(3) as i8 - 1))),
            _ => {}
        }
        r.run(64, 0.0);
        r.cmd(L::Overdub);
        r.run(d.bars(0.1, 1.5), d.within(0.02, 0.2) as f32);
        overdubs += 1;
        // Redo would bring the poison back only if the overdub left it
        // (refused, the layers full): then no Redo this round.
        let redo_poison = r.l.status().redo > 0;
        r.clean(&what("the overdub over poisoned blocks"));
        for press in 0..10 {
            let x = d.within(0.02, 0.2) as f32;
            let pick = d.below(20);
            let w = match pick {
                0 => {
                    r.at(T::TempoNow(d.within(40.0, 400.0)));
                    "tempo"
                }
                1 => {
                    r.at(T::Locate(d.below(32) as i64 * BAR_TICKS / 8));
                    "locate"
                }
                2 => {
                    let a = d.below(8) as i64 * BAR_TICKS / 4;
                    let l = (1 + d.below(8)) as i64 * BAR_TICKS / 4;
                    let on = d.below(3) > 0;
                    r.at(T::SetLoop(on.then_some((a, a + l))));
                    "the song's loop"
                }
                3 => {
                    r.at(T::Stop);
                    r.run(d.bars(0.0, 0.5), x);
                    r.at(T::Play);
                    "stop and play"
                }
                4..=6 => {
                    r.select(0);
                    r.cmd(L::Overdub);
                    overdubs += 1;
                    "overdub"
                }
                7 => {
                    let m = [Overdubs::Add, Overdubs::Replace, Overdubs::Once];
                    r.cmd(L::SetOverdubs(m[d.below(3) as usize]));
                    "overdubs"
                }
                8 => {
                    r.cmd(L::Undo);
                    "undo"
                }
                9 if !redo_poison => {
                    r.cmd(L::Redo);
                    "redo"
                }
                10 => {
                    r.cmd(L::SetLatency(d.below(MAX_LATENCY) as u32));
                    "latency"
                }
                11 => {
                    r.cmd(L::Edit(E::Speed(d.below(3) as i8 - 1)));
                    "speed"
                }
                12 => {
                    let e = [E::Half, E::Double, E::Reverse][d.below(3) as usize];
                    r.cmd(L::Edit(e));
                    "half, double or reverse"
                }
                13 => {
                    r.cmd(L::Edit(E::Nudge(d.below(9) as i32 - 4)));
                    "nudge"
                }
                14 => {
                    let f = d.below(2 * BAR) as i64 - BAR as i64;
                    let e = [E::Move(f), E::Resize(f), E::Turn(f)][d.below(3) as usize];
                    r.cmd(L::Edit(e));
                    "move, resize or turn"
                }
                15 => {
                    r.cmd(L::Merge);
                    "merge"
                }
                // A first pass on loop 2, cut short by STOP or closed;
                // or cleared and forgotten, its blocks freed.
                16 | 17 => {
                    r.select(1);
                    if r.l.loop_status(1).unwrap().phase == Phase::Empty {
                        r.cmd(L::SetBars([None, Some(1), Some(2)][d.below(3) as usize]));
                        r.cmd(L::Record);
                        r.run(d.bars(0.05, 2.5), x);
                        if d.below(2) == 0 {
                            r.at(T::Stop);
                            r.run(d.bars(0.0, 0.3), x);
                            r.at(T::Play);
                            cut_short += 1;
                        } else if r.l.loop_status(1).unwrap().phase == Phase::Recording {
                            r.cmd(L::Record);
                        }
                    } else {
                        r.cmd(L::Clear);
                        r.run(4 * FADE as u64, x);
                        if d.below(2) == 0 {
                            r.cmd(L::Forget);
                        }
                    }
                    r.select(0);
                    "loop 2's first pass"
                }
                _ => "nothing",
            };
            r.run(d.bars(0.05, 1.2), x);
            r.clean(&what(&format!("press {press}, {w}")));
        }
        // Loop 1's overdub ends; Once ends by itself.
        if r.l.status().phase == Phase::Overdubbing {
            r.cmd(L::Overdub);
        }
        r.run(4 * FADE as u64, 0.0);
        r.clean(&what("the round's end"));
    }
    assert!(skipped < rounds / 4, "{skipped} rounds of {rounds} skipped");
    (r.watched, overdubs, cut_short)
}

/// Random presses over poisoned memory, at three seeds and period sizes:
/// nothing stale is ever heard or kept. Each round's first overdub takes
/// the blocks a poison overdub, undone, has just let go of, under a
/// tempo change, a locate, a latency or a speed; the presses after it
/// mix overdubs (Add, Replace, Once) with the song's tempo changes,
/// locates, loop and stops, Undo and Redo, the edits, Merge and first
/// passes on another loop that STOP cuts short.
#[test]
fn nothing_unrecorded_is_ever_heard_whatever_moves_under_an_overdub() {
    let mut heard = 0;
    let (mut overdubs, mut cut_short) = (0, 0);
    for (seed, period) in [
        (0x000B_AD5E_ED0F_C0DE_u64, 128),
        (0x0005_EED0_FA11_7E57_u64, 97),
        (0xF00D_FACE_D15C_0123_u64, 256),
    ] {
        let (h, o, c) = rounds(seed, 120, period);
        heard += h;
        overdubs += o;
        cut_short += c;
    }
    // It ran what it says it ran.
    assert!(heard > 40_000_000, "{heard} frames heard");
    assert!(
        overdubs > 600 && cut_short > 60,
        "{overdubs} overdubs, {cut_short} cut short"
    );
}

/// A first pass closes on its bar line with its last `latency` frames
/// still to come in (its writer runs that far behind the playhead), and
/// STOP cuts one short the same way. Until they are written they are not
/// the loop's: read at once, the layer holds what was played, or silence
/// where it hasn't come in yet, never what its blocks held; and the song
/// jumped to the loop's last beat at once plays nothing stale.
#[test]
fn a_first_pass_holds_only_what_it_wrote_while_its_last_frames_come_in() {
    for stop in [false, true] {
        let mut r = Rig::new(20, 64);
        r.at(T::Play);
        r.run(START, 0.0);
        poison_the_memory(&mut r, 20);
        r.cmd(L::SetLatency(2_000));
        r.cmd(L::SetBars(None));
        r.cmd(L::Record);
        let end = r.frame + 4 * BAR;
        while r.l.status().phase != Phase::Recording && r.frame < end {
            r.run(16, 0.25);
        }
        r.run(BAR + BAR / 2, 0.25);
        if stop {
            r.at(T::Stop);
        } else {
            r.cmd(L::Record);
        }
        let end = r.frame + 2 * BAR;
        while r.l.status().phase != Phase::Playing && r.frame < end {
            r.run(16, 0.25);
        }
        let s = r.l.status();
        assert_eq!(
            (s.phase, s.ticks),
            (Phase::Playing, 2 * BAR_TICKS),
            "stop {stop}"
        );
        r.watch();
        r.clean(&format!("stop {stop}: at once"));
        // The song to the loop's last 1/32 note, at once.
        if stop {
            r.at(T::Play);
        }
        r.at(T::Locate(s.origin + s.ticks - BAR_TICKS / 32));
        r.run(BAR / 4, 0.25);
        r.clean(&format!("stop {stop}: its last beat"));
        r.run(3 * BAR, 0.0);
        r.clean(&format!("stop {stop}: round again"));
    }
}

/// An edit crossfades from the loop before it, through the layers it had
/// then. Overdub at another speed waits for a print; setting the speed
/// back is an edit, and the overdub opens at once, in its crossfade, in a
/// slot whose last layer (a reversed overdub on a shorter loop, cleared)
/// left its view in the shape faded from. The new layer is not read
/// through that view: it read the wrong frames, or (the new layer past
/// the view's end) failed on the arithmetic.
#[test]
fn an_overdub_opened_in_an_edits_crossfade_plays_only_what_it_wrote() {
    let mut r = Rig::new(20, 64);
    r.at(T::Play);
    r.run(START, 0.0);
    poison_the_memory(&mut r, 20);
    let first_pass = |r: &mut Rig, bars: u32| {
        r.cmd(L::SetBars(Some(bars)));
        r.cmd(L::Record);
        let end = r.frame + 2 * (bars as u64 + 1) * BAR;
        while r.l.status().phase != Phase::Playing && r.frame < end {
            r.run(64, 0.25);
        }
        assert_eq!(r.l.status().phase, Phase::Playing);
    };
    // A 1-bar loop with an overdub, reversed, then cleared: the overdub's
    // slot keeps its reversed 1-bar view.
    first_pass(&mut r, 1);
    r.cmd(L::SetOverdubs(Overdubs::Add));
    r.cmd(L::Overdub);
    r.run(BAR / 2, 0.1);
    r.cmd(L::Overdub);
    r.run(BAR / 4, 0.0);
    r.cmd(L::Edit(E::Reverse));
    r.run(4 * FADE as u64, 0.0);
    assert_eq!(r.l.status().layers, 2);
    r.cmd(L::Clear);
    r.run(4 * FADE as u64, 0.0);
    r.cmd(L::Forget);
    r.run(4 * FADE as u64, 0.0);
    // A 4-bar loop. At half speed Overdub waits for a print; the speed set
    // back with the song in the loop's last bar, it opens there at once.
    first_pass(&mut r, 4);
    let s = r.l.status();
    r.cmd(L::Edit(E::Speed(-1)));
    r.run(4 * FADE as u64, 0.0);
    r.cmd(L::Overdub);
    r.run(64, 0.0);
    assert!(r.l.status().overdub_next);
    // The song's place in the loop at its own speed (no locates here).
    let at = |r: &Rig| (r.frame as i64 * BAR_TICKS / BAR as i64 - s.origin).rem_euclid(s.ticks);
    while !(3 * BAR_TICKS + BAR_TICKS / 4..3 * BAR_TICKS + BAR_TICKS / 2).contains(&at(&r)) {
        r.run(64, 0.0);
    }
    r.watch();
    r.cmd(L::Edit(E::Speed(0)));
    r.run(64, 0.1);
    let s = r.l.status();
    assert_eq!((s.phase, s.layers, s.speed), (Phase::Overdubbing, 2, 0));
    r.run(BAR, 0.1);
    r.clean("the overdub opened in the crossfade");
}
