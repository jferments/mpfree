//! A demo of audio tracks: a bass line recorded on track 1, then three
//! loop passes of a melody on track 2 (each played a little differently),
//! comped from the best bits of each, and the song played back.
//!
//! `cargo run --release -p mpfree-audiotracks --example demo [OUT.wav]`
//! writes the playback (16-bit stereo, 44.1 kHz) and prints a hash of it.

use mpfree_audiotracks::{AudioTracks, Command as A, Input};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 44_100;
const BPM: f64 = 100.0;
const PERIOD: usize = 128;

fn bar_frames() -> f64 {
    4.0 * 60.0 / BPM * f64::from(RATE)
}

/// A plucked note (a few decaying partials) at `hz`, `t` seconds in.
fn pluck(hz: f64, t: f64) -> f64 {
    if t < 0.0 {
        return 0.0;
    }
    let env = (-t * 3.5).exp() * (1.0 - (-t * 400.0).exp());
    let w = std::f64::consts::TAU * hz * t;
    env * (w.sin() + 0.4 * (2.0 * w).sin() * (-t * 6.0).exp() + 0.15 * (3.0 * w).sin())
}

fn midi_hz(n: f64) -> f64 {
    440.0 * 2f64.powf((n - 69.0) / 12.0)
}

/// What a player plays into the line input at absolute frame `g`, given
/// notes as (frame, midi note) pairs.
fn play(notes: &[(f64, f64)], g: u64, level: f64) -> f32 {
    let x = g as f64;
    let mut s = 0.0;
    for &(at, n) in notes {
        if at <= x && x < at + 2.0 * f64::from(RATE) {
            s += pluck(midi_hz(n), (x - at) / f64::from(RATE));
        }
    }
    (level * s) as f32
}

struct Rig {
    t: Transport,
    a: AudioTracks,
    b: Block,
    frame: u64,
    out: Vec<f32>,
    keep: bool,
}

impl Rig {
    fn run_to(&mut self, end: u64, input: &dyn Fn(u64) -> f32) {
        let mut inp = [0.0f32; 2 * PERIOD];
        let mut mix = [0.0f32; 2 * PERIOD];
        while self.frame < end {
            let n = PERIOD.min((end - self.frame) as usize);
            for j in 0..n {
                let s = input(self.frame + j as u64);
                inp[2 * j] = s;
                inp[2 * j + 1] = s;
            }
            mix[..2 * n].fill(0.0);
            self.t.process(n as u32, &mut self.b);
            let map = *self.t.tempo_map();
            self.a
                .process(&self.b, &map, &inp[..2 * n], &mut mix[..2 * n]);
            if self.keep {
                self.out.extend_from_slice(&mix[..2 * n]);
            }
            self.frame += n as u64;
        }
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "audiotracks-demo.wav".into());
    let bar = bar_frames();
    let mut r = Rig {
        t: Transport::new(RATE, BPM).unwrap(),
        a: AudioTracks::new(RATE, 120),
        b: Block::new(),
        frame: 0,
        out: Vec::new(),
        keep: false,
    };
    // Round 7: the MPC One's line input records on the left jack.
    for k in 0..2 {
        r.a.command(A::SetInput(k, Input::Left));
    }
    // Take 1: a bass line over 4 bars, on track 1.
    let start = 1000u64;
    let beat = bar / 4.0;
    let bass: Vec<(f64, f64)> = (0..16)
        .map(|i| {
            let root = [36.0, 36.0, 41.0, 43.0][i / 4];
            (
                start as f64 + i as f64 * beat,
                root + if i % 2 == 1 { 12.0 } else { 0.0 },
            )
        })
        .collect();
    r.a.command(A::Arm(0, true));
    r.t.command(start, T::Record);
    r.t.command(start + (4.0 * bar) as u64, T::Stop);
    let end = start + (4.0 * bar) as u64 + 2000;
    r.run_to(end, &|g| play(&bass, g, 0.35));
    r.a.command(A::Arm(0, false));
    // Takes 2-4: a melody looped over the 4 bars on track 2; each pass
    // has a different ending phrase (and the second pass a wrong note).
    let s2 = end + 1000;
    let tick_bar = 3840;
    r.t.command(s2, T::SetLoop(Some((0, 4 * tick_bar))));
    r.t.command(s2, T::Locate(0));
    r.a.command(A::Arm(1, true));
    r.t.command(s2, T::Record);
    let mut melody = Vec::new();
    for pass in 0..3 {
        let p0 = s2 as f64 + pass as f64 * 4.0 * bar;
        let line = match pass {
            0 => [72.0, 74.0, 76.0, 79.0, 77.0, 76.0, 74.0, 72.0],
            1 => [72.0, 74.0, 76.0, 78.0, 77.0, 76.0, 74.0, 71.0],
            _ => [72.0, 74.0, 76.0, 79.0, 81.0, 79.0, 77.0, 79.0],
        };
        for (i, n) in line.iter().enumerate() {
            melody.push((p0 + i as f64 * bar / 2.0, *n));
        }
    }
    let e2 = s2 + (12.0 * bar) as u64;
    r.t.command(e2, T::Stop);
    r.run_to(e2 + 2000, &|g| play(&melody, g, 0.25));
    r.a.command(A::Arm(1, false));
    // Comp: bars 1-2 from pass 1, bars 3-4 from pass 3 (pass 2 had the
    // wrong note).
    let takes: Vec<_> = r.a.takes().filter(|t| t.track == 1).collect();
    let pass = |p: u16| takes.iter().find(|t| t.pass == p).unwrap().id;
    r.a.command(A::Comp {
        take: pass(0),
        from: 0,
        to: 2 * tick_bar,
    });
    r.a.command(A::Comp {
        take: pass(2),
        from: 2 * tick_bar,
        to: 4 * tick_bar,
    });
    r.a.command(A::SetPan(1, 0.3));
    r.a.command(A::SetPan(0, -0.2));
    // Play the song back twice round the loop.
    let s3 = r.frame + 100;
    r.t.command(s3, T::PlayStart);
    r.run_to(s3, &|_| 0.0);
    r.keep = true;
    r.run_to(s3 + (8.0 * bar) as u64, &|_| 0.0);

    let peak = r.out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for s in &r.out {
        for b in s.to_le_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    let mut w = Vec::new();
    let frames = r.out.len() / 2;
    let bytes = (frames * 4) as u32;
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + bytes).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&bytes.to_le_bytes());
    for s in &r.out {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        w.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(&path, w).expect("write the WAV");
    println!(
        "{path}: {:.1} s, peak {:.1} dBFS, {} takes, track 2 regions {}, audio hash {h:016x}",
        frames as f64 / f64::from(RATE),
        20.0 * peak.max(1e-9).log10(),
        r.a.takes().count(),
        r.a.regions(1).len(),
    );
}
