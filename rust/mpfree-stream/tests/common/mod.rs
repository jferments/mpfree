//! Shared by the stream tests: sound files whose every sample says which
//! file and which frame it is, and a rig that runs a player and its
//! coordinator period by period in one thread.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use mpfree_stream::coord::{Coordinator, Inline, Plan};
use mpfree_stream::cursor::Cursor;
use mpfree_stream::player::{Config, Player};

/// A fresh folder for one test's files.
pub fn folder(name: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "mpfree-stream-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("temp folder");
    d
}

/// How a test file stores its samples.
#[derive(Clone, Copy, Debug)]
pub enum Store {
    Pcm16,
    Pcm24,
    Float,
}

/// The sample of file `marker` at `frame`, channel `ch`, as a whole number
/// below 2^23, never 0. Channel 0 holds the marker and the frame's low 16
/// bits; channel 1 the frame's high bits. A 24-bit or float file holds it
/// as `v / 2^23` exactly; a 16-bit file holds only `v % 32768`.
pub fn value(marker: u32, frame: u64, ch: usize) -> u32 {
    match ch {
        0 => (marker << 16) | (frame as u32 & 0xFFFF),
        _ => (frame >> 16) as u32 + 1,
    }
}

/// What a 24-bit or float file's sample plays as.
pub fn played(marker: u32, frame: u64, ch: usize) -> f32 {
    value(marker, frame, ch) as f32 / 8_388_608.0
}

/// What a 16-bit file's sample plays as.
pub fn played16(marker: u32, frame: u64, ch: usize) -> f32 {
    let v = (value(marker, frame, ch) % 32_767 + 1) as i16;
    f32::from(v) / 32_768.0
}

/// Read a sample of a 24-bit or float file back into (marker, frame low
/// bits) or (frame high bits).
pub fn unplay(x: f32) -> u32 {
    (x * 8_388_608.0) as u32
}

/// Write a WAV of `frames` frames.
pub fn wav(path: &Path, store: Store, channels: u16, frames: u64, marker: u32) {
    let (tag, bits): (u16, u16) = match store {
        Store::Pcm16 => (1, 16),
        Store::Pcm24 => (1, 24),
        Store::Float => (3, 32),
    };
    let align = u32::from(channels * bits / 8);
    let rate = 44_100u32;
    let mut data = Vec::with_capacity(frames as usize * align as usize);
    for f in 0..frames {
        for c in 0..usize::from(channels) {
            match store {
                Store::Pcm16 => {
                    let v = (value(marker, f, c) % 32_767 + 1) as i16;
                    data.extend_from_slice(&v.to_le_bytes());
                }
                Store::Pcm24 => {
                    let v = value(marker, f, c);
                    data.extend_from_slice(&v.to_le_bytes()[..3]);
                }
                Store::Float => {
                    data.extend_from_slice(&played(marker, f, c).to_le_bytes());
                }
            }
        }
    }
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&tag.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * align).to_le_bytes());
    fmt.extend_from_slice(&(align as u16).to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    let mut f = b"RIFF".to_vec();
    f.extend_from_slice(&(4 + 8 + fmt.len() as u32 + 8 + data.len() as u32).to_le_bytes());
    f.extend_from_slice(b"WAVE");
    f.extend_from_slice(b"fmt ");
    f.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    f.extend_from_slice(&fmt);
    f.extend_from_slice(b"data");
    f.extend_from_slice(&(data.len() as u32).to_le_bytes());
    f.extend_from_slice(&data);
    fs::write(path, f).expect("write wav");
}

/// A player and its coordinator, reading files inline.
pub struct Rig {
    pub p: Player,
    pub c: Coordinator,
}

impl Rig {
    pub fn new(cfg: Config, plan: Plan) -> Rig {
        let (p, rx, tx) = Player::new(cfg);
        let c = Coordinator::new(cfg, plan, rx, tx, Box::<Inline>::default());
        Rig { p, c }
    }

    /// One period of `out.len() / 2` frames for each cursor, then a
    /// coordinator round. Gives the frames missed.
    pub fn period(&mut self, cursors: &mut [&mut Cursor], outs: &mut [Vec<f32>]) -> u64 {
        self.p.begin();
        let mut missed = 0;
        let mut frames = 0;
        for (k, o) in cursors.iter_mut().zip(outs.iter_mut()) {
            missed += k.play(&mut self.p, o);
            frames = o.len() / 2;
        }
        self.p.end(frames as u32);
        self.c.step();
        missed
    }

    /// Hold the cursors where they are until everything they want ahead
    /// is ready (or `periods` pass).
    pub fn warm(&mut self, cursors: &mut [&mut Cursor], periods: usize) {
        let mut outs = vec![vec![0.0; 512]; cursors.len()];
        let mut quiet = 0;
        for _ in 0..periods {
            let at: Vec<(u64, f64)> = cursors
                .iter()
                .map(|k| (k.frame(), k.position() - k.frame() as f64))
                .collect();
            let before = (self.p.ready(), self.c.cached());
            self.period(cursors, &mut outs);
            for (k, (f, _)) in cursors.iter_mut().zip(at) {
                k.seek(f);
            }
            if (self.p.ready(), self.c.cached()) == before && self.p.ready() == self.c.cached() {
                quiet += 1;
                if quiet > 8 {
                    return;
                }
            } else {
                quiet = 0;
            }
        }
    }
}
