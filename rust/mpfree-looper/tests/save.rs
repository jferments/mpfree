//! Loops saved with a project and loaded back: a loop with two layers, an
//! edit and a mix is saved (its text and a WAV a layer), loaded into
//! another looper and plays the same, sample for sample, in time with the
//! song, from the same frame for any period size. Clear calls a load off,
//! and a load is refused where it cannot go.
//!
//! 22.05 kHz at 240 BPM in 4/4 makes a bar exactly 22 050 frames (1 s).

use mpfree_looper::save::{from_text, from_wav, to_text, wav, Saved};
use mpfree_looper::{
    Command as L, Edit, LayerCommand as Lc, LoopFile, Looper, Phase, FADE, LOAD_SPEED,
};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 22_050;
const BAR: usize = 22_050;

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
    fn new(period: usize, seconds: u32) -> Rig {
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
}

fn sine(f: u64) -> [f32; 2] {
    let x = 0.5 * (f as f64 * std::f64::consts::TAU / 50.0).sin() as f32;
    [x, 0.5 * x]
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

/// A one-bar loop from bar 1: the sine, then an overdub of noise, nudged
/// by three 16ths (a cut, so a seam fade), the overdub at half level
/// panned right, the loop at 0.8.
fn recorded() -> Rig {
    let mut r = Rig::new(128, 20);
    r.cmd(L::SetBars(Some(1)));
    r.cmd(L::Record);
    assert!(r.t.command(0, T::Play));
    r.run(2 * BAR as u64, &sine);
    assert_eq!(r.l.status().phase, Phase::Playing);
    r.cmd(L::Overdub);
    r.run(3 * BAR as u64, &noise);
    r.cmd(L::Overdub);
    r.run(3 * BAR as u64 + 4096, &silence);
    r.cmd(L::Edit(Edit::Nudge(3)));
    let id = r.l.layer_info(1).unwrap().id;
    assert!(r.l.layer_command(id, Lc::Level(0.5)));
    assert!(r.l.layer_command(id, Lc::Pan(0.3)));
    r.cmd(L::SetLevel(0.8));
    r.run(r.frame + 4096, &silence);
    let s = r.l.status();
    assert_eq!((s.phase, s.layers, s.frames), (Phase::Playing, 2, BAR));
    r
}

/// Loop 0 saved: its file with each layer's audio read a piece at a time.
fn saved(r: &Rig) -> LoopFile {
    let mut f = r.l.loop_file(0).expect("savable");
    let version = r.l.status().version;
    for (k, y) in f.layers.iter_mut().enumerate() {
        y.audio = vec![0.0; 2 * f.frames];
        for (i, piece) in y.audio.chunks_mut(2 * 1000).enumerate() {
            assert_eq!(r.l.read_layer(0, k, 1000 * i, piece), Some(version));
        }
    }
    f
}

/// Through the text and the WAV files, as a project keeps it.
fn through_files(f: LoopFile) -> LoopFile {
    let mut files = Vec::new();
    let mut s = Saved {
        rate: RATE,
        selected: 0,
        loops: vec![(3, f)],
    };
    for (k, y) in s.loops[0].1.layers.iter_mut().enumerate() {
        y.file = format!("layer {k}.wav");
        files.push(wav(&std::mem::take(&mut y.audio), RATE));
    }
    let mut back = from_text(&to_text(&s)).unwrap();
    assert_eq!(back.rate, RATE);
    let (n, mut f) = back.loops.remove(0);
    assert_eq!(n, 3);
    for (y, b) in f.layers.iter_mut().zip(&files) {
        let (audio, rate) = from_wav(b).unwrap();
        assert_eq!(rate, RATE);
        y.audio = audio;
    }
    f
}

/// A fresh looper playing from frame 0, `f` loaded into loop 3 there.
fn loaded(period: usize, f: LoopFile) -> Rig {
    let mut r = Rig::new(period, 20);
    assert!(r.t.command(0, T::Play));
    r.l.load(3, f).unwrap();
    r
}

#[test]
fn a_saved_loop_plays_the_same_once_loaded() {
    let mut a = recorded();
    let f = saved(&a);
    assert_eq!(f.layers.len(), 2);
    assert_eq!((f.layers[1].level, f.layers[1].pan), (0.5, 0.3));
    assert_eq!(f.level, 0.8);
    let f = through_files(f);
    let mut b = loaded(128, f);
    // Silent while it copies in, then in.
    let copy = (2 * BAR).div_ceil(LOAD_SPEED) as u64;
    b.run(copy / 2, &silence);
    let s = b.l.loop_status(3).unwrap();
    assert_eq!(s.phase, Phase::Empty);
    assert!(s.loading.is_some_and(|p| p > 0.3 && p < 0.7), "{s:?}");
    assert!(s.used >= 2 * BAR);
    assert!(b.out.iter().all(|&x| x == 0.0));
    b.run(copy + 128, &silence);
    let s = b.l.loop_status(3).unwrap();
    assert_eq!(
        (s.phase, s.layers, s.frames, s.loading),
        (Phase::Playing, 2, BAR, None)
    );
    assert_eq!(s.level, 0.8);
    let info = b.l.loop_layer_info(3, 1).unwrap();
    assert_eq!((info.level, info.pan), (0.5, 0.3));
    assert!(b.l.take_loaded(3).is_some());
    assert!(b.l.take_loaded(3).is_none());
    // The same as the loop it came from, frame for frame, a bar on.
    let end = a.frame + 2 * BAR as u64;
    a.run(end, &silence);
    b.run(end, &silence);
    let from = 2 * (end as usize - BAR);
    let worst = (from..2 * end as usize)
        .map(|i| (a.out[i] - b.out[i]).abs())
        .fold(0.0, f32::max);
    assert!(worst < 1e-5, "differs by {worst}");
    assert!(b.out[from..].iter().any(|&x| x.abs() > 0.1));
}

#[test]
fn a_load_comes_in_on_the_same_frame_for_any_period() {
    let f = saved(&recorded());
    let end = 3 * BAR as u64;
    let outs: Vec<Vec<f32>> = [1000, 128, 64, 1]
        .iter()
        .map(|&p| {
            let mut r = loaded(p, f.clone());
            if p == 1 {
                // A frame at a time is slow: the first part is the same.
                r.run(end / 8, &silence);
            } else {
                r.run(end, &silence);
            }
            r.out
        })
        .collect();
    for o in &outs[1..] {
        let n = o.len();
        assert_eq!(&outs[0][..n], &o[..], "the period changed the output");
    }
    let first = outs[0].iter().position(|&x| x != 0.0).unwrap() / 2;
    let copy = (2 * BAR).div_ceil(LOAD_SPEED);
    assert!(
        first >= copy && first < copy + FADE,
        "first sound at {first}"
    );
}

#[test]
fn clear_calls_a_load_off() {
    let f = saved(&recorded());
    let mut r = loaded(128, f);
    r.run(256, &silence);
    assert!(r.l.loop_status(3).unwrap().loading.is_some());
    assert!(r.l.take_loaded(3).is_none());
    assert!(r.l.select_loop(3));
    // Commands wait while it loads; Clear does not.
    r.cmd(L::Overdub);
    r.cmd(L::Clear);
    r.run(2048, &silence);
    let s = r.l.loop_status(3).unwrap();
    assert_eq!((s.phase, s.loading, s.used), (Phase::Empty, None, 0));
    assert_eq!(r.l.memory().used, 0);
    assert!(r.l.take_loaded(3).is_some());
    r.run(3 * BAR as u64, &silence);
    assert!(r.out.iter().all(|&x| x == 0.0));
    assert_eq!(r.l.loop_status(3).unwrap().phase, Phase::Empty);
}

#[test]
fn a_load_is_refused_where_it_cannot_go() {
    let mut a = recorded();
    let f = saved(&a);
    // Not into a loop that is not empty, or a loop past the last.
    let f = a.l.load(0, f).unwrap_err();
    let f = a.l.load(8, f).unwrap_err();
    // Not with a layer short of the loop's length.
    let mut short = f.clone();
    short.layers[1].audio.truncate(100);
    a.l.load(1, short).unwrap_err();
    // Not with no layers.
    let mut none = f.clone();
    none.layers.clear();
    a.l.load(1, none).unwrap_err();
    // Not past the memory: two 1 s layers into 1.5 s.
    let mut small = Rig::new(128, 1);
    let f = small.l.load(0, f).unwrap_err();
    // Not while the last load waits to be collected.
    let mut r = Rig::new(128, 20);
    r.l.load(0, f.clone()).unwrap();
    r.run(BAR as u64, &silence);
    r.cmd(L::Clear);
    r.run(BAR as u64 + 2 * FADE as u64 + 256, &silence);
    assert_eq!(r.l.status().phase, Phase::Empty);
    let f = r.l.load(0, f).unwrap_err();
    assert!(r.l.take_loaded(0).is_some());
    r.l.load(0, f).unwrap();
    // Nothing to save from an overdub in progress.
    a.cmd(L::Overdub);
    a.run(a.frame + 1024, &silence);
    assert!(a.l.loop_file(0).is_none());
    let mut buf = [1.0; 8];
    assert_eq!(a.l.read_layer(0, 0, 0, &mut buf), None);
    assert_eq!(buf, [1.0; 8]);
}
