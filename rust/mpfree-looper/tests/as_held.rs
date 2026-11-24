//! Loops kept as the looper holds them (format 2, audio tracks phase 4c):
//! each layer's audio as it was written ([`Looper::raw_layer`]), how the
//! layer plays it, and the loop's shape ([`Looper::loop_file_raw`]).
//! Saved that way, through the text and the WAV files, and opened in
//! another looper, a loop plays the same frames as before, and an edit
//! gives the same frames on both: a first pass opens with what it kept
//! around its loop, so Move, Resize and Turn work as before. A closed
//! layer's audio never changes: an overdub, a Replace, Fade old and a
//! Merge each write a layer of their own, and an edit changes only how
//! the layers play.
//!
//! 22.05 kHz at 240 BPM in 4/4 makes a bar exactly 22 050 frames (1 s).

use mpfree_looper::save::{from_text, from_wav, to_text, wav, Saved};
use mpfree_looper::{
    Command as L, Edit as E, LayerCommand as Lc, LoopFile, Looper, Overdubs, Phase, FADE,
    KEEP_AFTER, KEEP_BEFORE, LOAD_SPEED,
};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: usize = 22_050;
const BAR_TICKS: i64 = 3_840;
/// The song starts here in the looper that records: what was heard
/// before it is what the first pass keeps before its start.
const START: u64 = 20_000;

struct Rig {
    t: Transport,
    l: Looper,
    b: Block,
    /// The frame the song started on.
    start: u64,
    frame: u64,
    input: Vec<f32>,
    mix: Vec<f32>,
    out: Vec<f32>,
}

impl Rig {
    fn new(seconds: u32, start: u64) -> Rig {
        let mut r = Rig {
            t: Transport::new(RATE, 240.0).unwrap(),
            l: Looper::new(RATE, seconds),
            b: Block::new(),
            start,
            frame: 0,
            input: vec![0.0; 256],
            mix: vec![0.0; 256],
            out: Vec::new(),
        };
        assert!(r.t.command(start, T::Play));
        r
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

    /// Frames since the song started.
    fn song(&self) -> u64 {
        self.frame - self.start
    }

    /// An edit, run until its crossfade is over; refused is a failure.
    fn edit(&mut self, e: E) {
        let refused = self.l.stats().refused;
        self.cmd(L::Edit(e));
        self.run(self.frame + 4 * FADE as u64, &silence);
        assert_eq!(self.l.stats().refused, refused, "{e:?} refused");
    }

    /// An overdub of `input` (by frames since the song started) for
    /// `frames`, then its punch-out run out.
    fn overdub(&mut self, frames: u64, input: &dyn Fn(u64) -> [f32; 2]) {
        self.cmd(L::Overdub);
        let start = self.start;
        self.run(self.frame + frames, &|f| input(f - start));
        self.cmd(L::Overdub);
        self.run(self.frame + 4_096, &silence);
        assert_eq!(self.l.status().phase, Phase::Playing);
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

fn tone(f: u64) -> [f32; 2] {
    let x = 0.3 * (f as f64 * std::f64::consts::TAU / 73.0).sin() as f32;
    [x, -0.5 * x]
}

fn other(f: u64) -> [f32; 2] {
    let x = 0.2 * (f as f64 * std::f64::consts::TAU / 31.0).sin() as f32;
    [0.5 * x, x]
}

fn silence(_: u64) -> [f32; 2] {
    [0.0; 2]
}

/// A one-bar first pass from bar 1 at START, with what it kept before
/// and after its loop all in.
fn recorded() -> Rig {
    let mut r = Rig::new(20, START);
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    r.run(START + BAR as u64 + 1, &noise);
    let s = r.l.status();
    assert_eq!((s.phase, s.frames, s.layers), (Phase::Playing, BAR, 1));
    r.run(START + (BAR + KEEP_AFTER) as u64 + 1_000, &noise);
    r
}

/// The first pass, an overdub on top for half a bar, a Replace over a
/// quarter, and the overdub turned down and panned.
fn layered() -> Rig {
    let mut r = recorded();
    r.overdub(BAR as u64 / 2, &tone);
    r.cmd(L::SetOverdubs(Overdubs::Replace));
    r.overdub(BAR as u64 / 4, &other);
    r.cmd(L::SetOverdubs(Overdubs::Add));
    assert_eq!(r.l.status().layers, 3);
    assert!(r.l.layer_info(2).unwrap().replace);
    let id = r.l.layer_info(1).unwrap().id;
    assert!(r.l.layer_command(id, Lc::Level(0.6)));
    assert!(r.l.layer_command(id, Lc::Pan(-0.4)));
    // (They apply at the next period.)
    r.run(r.frame + 4_096, &silence);
    r
}

/// As layered, with the Replace muted: what it covered plays again.
fn replace_muted() -> Rig {
    let mut r = layered();
    let id = r.l.layer_info(2).unwrap().id;
    assert!(r.l.layer_command(id, Lc::Mute(true)));
    r.run(r.frame + 4_096, &silence);
    r
}

/// Loop 0 as the looper holds it, read a piece at a time, then through
/// the text and the WAV files, as a project keeps it.
fn held(r: &Rig) -> LoopFile {
    let mut f = r.l.loop_file_raw(0).expect("savable");
    let version = r.l.status().version;
    for (k, y) in f.layers.iter_mut().enumerate() {
        let size = y.raw.as_ref().expect("kept as held").size;
        y.audio = vec![0.0; 2 * size];
        for (i, piece) in y.audio.chunks_mut(2 * 1000).enumerate() {
            assert_eq!(r.l.raw_layer(0, k, 1000 * i, piece), Some(version));
        }
    }
    let mut files = Vec::new();
    let mut s = Saved {
        rate: RATE,
        selected: 0,
        loops: vec![(0, f)],
        free_clock: None,
    };
    for (k, y) in s.loops[0].1.layers.iter_mut().enumerate() {
        y.file = format!("layer {k}.wav");
        files.push(wav(&std::mem::take(&mut y.audio), RATE));
    }
    let text = to_text(&s);
    assert!(text.starts_with("mpfree-looper 2\n"), "{text}");
    let mut back = from_text(&text).unwrap();
    assert_eq!(to_text(&back), text, "the text reads back as written");
    let (_, mut f) = back.loops.remove(0);
    for (y, b) in f.layers.iter_mut().zip(&files) {
        let (audio, rate) = from_wav(b).unwrap();
        assert_eq!(rate, RATE);
        y.audio = audio;
    }
    f
}

/// A fresh looper, its song from frame 0, `f` loaded into loop 0 and
/// all in.
fn opened(f: LoopFile) -> Rig {
    let mut b = Rig::new(20, 0);
    let total: usize = f.layers.iter().map(|y| y.audio.len() / 2).sum();
    b.l.load(0, f).unwrap();
    b.run(total.div_ceil(LOAD_SPEED) as u64 + 1_000, &silence);
    assert_eq!(b.l.status().phase, Phase::Playing);
    b
}

/// Both at the same place in their songs.
fn sync(a: &mut Rig, b: &mut Rig) {
    let t = a.song().max(b.song());
    a.run(a.start + t, &silence);
    b.run(b.start + t, &silence);
}

/// The same edit on both, at the same place in their songs.
fn both(a: &mut Rig, b: &mut Rig, e: E) {
    sync(a, b);
    a.edit(e);
    b.edit(e);
}

/// Both play on for two bars, the second the same to 0.00001, and not
/// silent.
fn same(a: &mut Rig, b: &mut Rig, what: &str) {
    let t = a.song().max(b.song()) + 2 * BAR as u64;
    a.run(a.start + t, &silence);
    b.run(b.start + t, &silence);
    let (mut worst, mut loud) = (0.0f32, 0.0f32);
    for s in t - BAR as u64..t {
        let (i, j) = ((a.start + s) as usize, (b.start + s) as usize);
        for c in 0..2 {
            worst = worst.max((a.out[2 * i + c] - b.out[2 * j + c]).abs());
            loud = loud.max(b.out[2 * j + c].abs());
        }
    }
    assert!(worst < 1e-5, "{what}: differs by {worst}");
    assert!(loud > 0.05, "{what}: silent");
}

#[test]
fn a_first_pass_opens_with_what_it_kept_around_it_and_moves_the_same() {
    let mut a = recorded();
    let bounds = a.l.status().bounds.expect("a first pass alone");
    let f = held(&a);
    // Its audio as written: what it kept before and after its loop.
    let r = f.layers[0].raw.as_ref().unwrap();
    assert_eq!(r.valid, (0, bounds.kept as usize));
    assert!(r.size >= KEEP_BEFORE + BAR + KEEP_AFTER);
    let shape = f.raw.unwrap();
    assert_eq!(shape.region, Some((KEEP_BEFORE, KEEP_BEFORE)));
    let mut b = opened(f);
    assert_eq!(b.l.status().bounds, Some(bounds));
    assert_eq!(b.l.memory().used, a.l.memory().used);
    same(&mut a, &mut b, "opened");
    for e in [
        E::Move(700),
        E::Turn(-3_000),
        E::Resize(-2_205),
        E::Move(-(KEEP_BEFORE as i64) / 2),
        E::Resize(1_000),
    ] {
        both(&mut a, &mut b, e);
        let s = b.l.status().bounds;
        assert!(s.is_some(), "{e:?}");
        assert_eq!(s, a.l.status().bounds, "{e:?}");
        same(&mut a, &mut b, &format!("{e:?}"));
    }
    // Moved and resized, it opens again as it is.
    let mut c = opened(held(&a));
    assert_eq!(c.l.status().bounds, a.l.status().bounds);
    same(&mut a, &mut c, "opened again");
}

#[test]
fn a_layered_loop_opens_as_it_was_and_each_edit_gives_the_same_frames() {
    let mut a = layered();
    let mut b = opened(held(&a));
    same(&mut a, &mut b, "opened");
    for e in [
        E::Reverse,
        E::Nudge(3),
        E::Half,
        E::Double,
        E::Trim {
            from: BAR_TICKS / 4,
            to: BAR_TICKS * 3 / 4,
        },
        E::SeamFade(600),
        E::Speed(1),
        E::Speed(0),
        E::Reverse,
    ] {
        both(&mut a, &mut b, e);
        same(&mut a, &mut b, &format!("{e:?}"));
        // Opened again after each edit, it plays the same.
        let mut c = opened(held(&a));
        same(&mut a, &mut c, &format!("opened after {e:?}"));
    }
    // An overdub on the reopened loop lands as on the one it came from.
    sync(&mut a, &mut b);
    a.overdub(BAR as u64 / 3, &tone);
    b.overdub(BAR as u64 / 3, &tone);
    same(&mut a, &mut b, "overdubbed");
}

#[test]
fn a_muted_replace_opens_muted_and_what_it_covered_plays() {
    let mut a = replace_muted();
    let f = held(&a);
    assert!(f.layers[2].muted && f.layers[2].raw.as_ref().unwrap().replace.is_some());
    let mut b = opened(f);
    same(&mut a, &mut b, "opened");
    // Heard again, it covers as it did.
    sync(&mut a, &mut b);
    for r in [&mut a, &mut b] {
        let id = r.l.layer_info(2).unwrap().id;
        assert!(r.l.layer_command(id, Lc::Mute(false)));
    }
    same(&mut a, &mut b, "heard again");
}

#[test]
fn with_its_first_pass_deleted_a_loop_opens_without_its_bounds() {
    let mut a = recorded();
    a.overdub(BAR as u64 / 2, &tone);
    let id = a.l.layer_info(0).unwrap().id;
    assert!(a.l.layer_command(id, Lc::Delete));
    a.run(a.frame + 4_096, &silence);
    assert_eq!(a.l.status().layers, 1);
    let f = held(&a);
    let shape = f.raw.unwrap();
    assert_eq!((shape.region, shape.captured), (None, (0, 0)));
    let mut b = opened(f);
    assert_eq!(b.l.status().bounds, None);
    same(&mut a, &mut b, "opened");
}

#[test]
fn a_loop_kept_as_held_that_does_not_hold_together_is_refused() {
    let a = layered();
    let f = held(&a);
    let mut l = Looper::new(RATE, 20);
    // A view that doesn't play the loop's length.
    let mut g = f.clone();
    g.layers[1].raw.as_mut().unwrap().pieces[0].1 -= 1;
    assert!(l.load(0, g).is_err());
    // Frames written past its audio.
    let mut g = f.clone();
    let r = g.layers[1].raw.as_mut().unwrap();
    r.valid = (r.size, 1);
    assert!(l.load(0, g).is_err());
    // A region past the first layer's audio.
    let mut g = f.clone();
    let size = g.layers[0].raw.as_ref().unwrap().size;
    g.raw.as_mut().unwrap().region = Some((size + 1, 0));
    assert!(l.load(0, g).is_err());
    // Only some of it kept as held.
    let mut g = f.clone();
    g.layers[2].raw = None;
    assert!(l.load(0, g).is_err());
    // Audio shorter than its view's size.
    let mut g = f.clone();
    let n = g.layers[0].audio.len();
    g.layers[0].audio.truncate(n - 2);
    assert!(l.load(0, g).is_err());
    // As it was, it loads.
    assert!(l.load(0, f).is_ok());
}

/// Layer `k` of loop 0's audio as written.
fn raw(r: &Rig, k: usize) -> Vec<f32> {
    let f = r.l.loop_file_raw(0).expect("savable");
    let size = f.layers[k].raw.as_ref().unwrap().size;
    let mut out = vec![0.0; 2 * size];
    r.l.raw_layer(0, k, 0, &mut out).unwrap();
    out
}

#[test]
fn a_closed_layers_audio_never_changes() {
    let mut r = recorded();
    let first = raw(&r, 0);
    r.overdub(BAR as u64 / 2, &tone);
    let over = raw(&r, 1);
    let unchanged = |r: &Rig, was: &[Vec<f32>], what: &str| {
        for (k, w) in was.iter().enumerate() {
            assert!(raw(r, k) == *w, "layer {k} changed by {what}");
        }
    };
    unchanged(&r, &[first.clone(), over.clone()], "the overdub");
    // Edits change how the layers play, not their audio.
    for e in [
        E::Reverse,
        E::Nudge(3),
        E::Half,
        E::Double,
        E::Trim {
            from: BAR_TICKS / 4,
            to: BAR_TICKS * 3 / 4,
        },
        E::SeamFade(600),
        E::Speed(1),
        E::Speed(0),
    ] {
        r.edit(e);
        unchanged(&r, &[first.clone(), over.clone()], &format!("{e:?}"));
    }
    // A Replace writes a layer of its own.
    r.cmd(L::SetOverdubs(Overdubs::Replace));
    r.overdub(BAR as u64 / 4, &other);
    r.cmd(L::SetOverdubs(Overdubs::Add));
    let replace = raw(&r, 2);
    let was = [first, over, replace];
    unchanged(&r, &was, "the Replace");
    // Fade old turns down only the overdub's own earlier passes.
    r.cmd(L::SetFadeOld(0.5));
    let len = r.l.status().frames as u64;
    r.overdub(len * 3 / 2, &tone);
    assert_eq!(r.l.status().layers, 4);
    unchanged(&r, &was, "Fade old");
    // A Merge writes the layers heard as one new layer; Undo brings them
    // back as they were.
    r.cmd(L::Merge);
    r.run(r.frame + 2 * BAR as u64, &silence);
    assert_eq!(r.l.status().layers, 1);
    r.cmd(L::Undo);
    r.run(r.frame + 4 * FADE as u64, &silence);
    assert_eq!(r.l.status().layers, 4);
    unchanged(&r, &was, "the Merge and its Undo");
}

/// Each layer's number ([`mpfree_looper::LayerInfo::born`]): a new one for
/// each layer made, the same through edits, mutes, undo and redo, and a
/// new one for a layer loaded or merged; closed once all of its audio is
/// written, and a first pass only once what it keeps after its loop is
/// in. A merged layer names the layers it was made from (4c.5).
#[test]
fn each_layer_keeps_a_number_of_its_own() {
    let mut r = Rig::new(20, START);
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    r.run(START + BAR as u64 + 1, &noise);
    let first = r.l.layer_info(0).unwrap();
    assert!(!first.closed, "still keeping what it hears after its end");
    r.run(START + (BAR + KEEP_AFTER) as u64 + 1_000, &noise);
    let first = r.l.layer_info(0).unwrap();
    assert!(first.closed && first.born > 0);
    r.cmd(L::Overdub);
    r.run(r.frame + 4_096, &tone);
    let over = r.l.layer_info(1).unwrap();
    assert!(over.recording && !over.closed && over.born > first.born);
    r.cmd(L::Overdub);
    r.run(r.frame + 4_096, &silence);
    let born = |r: &Rig| -> Vec<u64> {
        (0..r.l.status().layers)
            .map(|k| r.l.layer_info(k).unwrap().born)
            .collect()
    };
    let both = vec![first.born, over.born];
    assert_eq!(born(&r), both);
    assert!(r.l.layer_info(1).unwrap().closed);
    // Edits, a mute, Undo and Redo keep the numbers.
    r.edit(E::Reverse);
    assert!(r.l.layer_command(over.id, Lc::Mute(true)));
    r.run(r.frame + 4_096, &silence);
    r.cmd(L::Undo);
    r.run(r.frame + 4_096, &silence);
    r.cmd(L::Undo);
    r.run(r.frame + 4_096, &silence);
    assert_eq!(born(&r), vec![first.born]);
    r.cmd(L::Redo);
    r.run(r.frame + 4_096, &silence);
    assert_eq!(born(&r), both);
    // A merge makes a layer of its own.
    r.cmd(L::Merge);
    r.run(r.frame + 2 * BAR as u64, &silence);
    let merged = born(&r);
    assert_eq!(merged.len(), 1);
    assert!(merged[0] > over.born);
    // It names the layers it was made from, in play order (4c.5); a
    // layer recorded names none.
    let from = |r: &Rig, k: usize| -> Vec<u64> {
        r.l.layer_from(0, k)
            .unwrap()
            .into_iter()
            .take_while(|&b| b != 0)
            .collect()
    };
    assert_eq!(from(&r, 0), both);
    // A loop loaded in has new ones, never one given before.
    r.cmd(L::Undo);
    r.run(r.frame + 4_096, &silence);
    assert_eq!((from(&r, 0), from(&r, 1)), (vec![], vec![]));
    r.cmd(L::Redo);
    r.run(r.frame + 4_096, &silence);
    assert_eq!(from(&r, 0), both, "redone, the same");
    r.cmd(L::Undo);
    r.run(r.frame + 4_096, &silence);
    let f = held(&r);
    let mut b = Rig::new(20, 0);
    b.l.load(0, f).unwrap();
    b.run(BAR as u64, &silence);
    let opened = born(&b);
    assert_eq!(opened.len(), 2);
    assert!(opened[0] != opened[1] && opened.iter().all(|&x| x > 0));
    assert!((0..2).all(|k| b.l.layer_info(k).unwrap().closed));
}

/// Phase 4c.3: a closed layer reads as it was written by its number
/// whatever the loop is doing (here overdubbing another layer, when the
/// loop can't be saved), never while it is still being written, and not
/// once the loop no longer lists it.
#[test]
fn a_closed_layer_reads_by_its_number_while_the_loop_goes_on() {
    let mut r = layered();
    let (first, k0) = (r.l.layer_info(0).unwrap(), raw(&r, 0));
    let (second, k1) = (r.l.layer_info(1).unwrap(), raw(&r, 1));
    r.cmd(L::Overdub);
    r.run(r.frame + 4_096, &tone);
    let open = r.l.layer_info(3).unwrap();
    assert!(open.recording && !open.closed);
    let mut two = [0.0f32; 2];
    assert_eq!(r.l.raw_layer(0, 0, 0, &mut two), None, "not savable now");
    let mut out = vec![1.0; k0.len()];
    assert_eq!(r.l.raw_of(0, first.born, 0, &mut out), Some(k0.len() / 2));
    assert_eq!(out, k0);
    // A piece from the middle, as the writer reads it.
    let mut piece = vec![1.0; 2_000];
    assert_eq!(
        r.l.raw_of(0, second.born, 300, &mut piece),
        Some(k1.len() / 2)
    );
    assert_eq!(piece, k1[600..2_600]);
    assert_eq!(
        r.l.raw_of(0, open.born, 0, &mut two),
        None,
        "still being written"
    );
    assert_eq!(r.l.raw_of(0, 0, 0, &mut two), None);
    assert_eq!(
        r.l.raw_of(1, first.born, 0, &mut two),
        None,
        "another loop's"
    );
    // Closed, it reads; undone, the loop no longer lists it.
    r.cmd(L::Overdub);
    r.run(r.frame + 4_096, &silence);
    assert!(r.l.layer_info(3).unwrap().closed);
    let k3 = raw(&r, 3);
    let mut out = vec![1.0; k3.len()];
    assert_eq!(r.l.raw_of(0, open.born, 0, &mut out), Some(k3.len() / 2));
    assert_eq!(out, k3);
    r.cmd(L::Undo);
    r.run(r.frame + 4_096, &silence);
    assert_eq!(r.l.raw_of(0, open.born, 0, &mut two), None);
    assert!(r.l.raw_of(0, first.born, 0, &mut two).is_some());
}
