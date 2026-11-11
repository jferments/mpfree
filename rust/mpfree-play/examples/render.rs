//! WORKSTATION ONLY: play a demo song through the transport, the player
//! and the real sampler engine, and write it to a WAV file, so playback can
//! be heard without the MPC. It never touches any device; it only writes
//! one WAV file.
//!
//! ```text
//! cargo run --release -p mpfree-play --example render -- [--period N] [--out FILE]
//! ```
//!
//! The song: 92 BPM, 1/16 swing at 56, a four-bar loop played twice.
//! Drums (a one-bar clip), hats at double speed with their own swing, a
//! two-bar bass line on gated pads (note-offs end the notes), and a
//! seven-sixteenths pluck clip that drifts against the bar (polymeter).
//! Every sound is synthesised here; no sample files are used.
//!
//! It prints two hashes. `events` covers every note the player sent (frame,
//! track, note, velocity) and is the same on every computer and for every
//! period size; the tool checks that itself at 1, 128 and 4096 frames.
//! `audio` covers the WAV bytes; it repeats on one computer but can differ
//! between computers (the sounds use the system's maths library).

use std::f32::consts::TAU;

use mpfree_play::{Event, Launch, Out, PlayView, Player, Sink};
use mpfree_sampler::dsp::Ahdsr;
use mpfree_sampler::engine::{Engine, EngineView, Event as SEvent};
use mpfree_sampler::model::{Layer, Pad, PlayMode, Project as SProject, Region, Source};
use mpfree_seq::{Clip, Note, Project, Speed, Track};
use mpfree_transport::time::Swing;
use mpfree_transport::{Block, Command, Transport};

const RATE: u32 = 44_100;
const BPM: f64 = 92.0;
const BAR: i64 = 3840;

fn main() {
    let mut period = 128u32;
    let mut out: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--period" => period = args.next().and_then(|v| v.parse().ok()).filter(|p| (1..=8192).contains(p)).unwrap_or_else(|| fail("--period takes 1 to 8192")),
            "--out" => out = Some(args.next().unwrap_or_else(|| fail("--out takes a file name"))),
            "-h" | "--help" => {
                println!("render [--period N] [--out FILE]   (default FILE: ~/mpfree-work/play-demo.wav)");
                return;
            }
            other => fail(&format!("unknown option {other}")),
        }
    }
    let out = out.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        format!("{home}/mpfree-work/play-demo.wav")
    });

    let view = song();
    let frames = (8.0 * 4.0 * 60.0 / BPM * f64::from(RATE)).ceil() as u64 + RATE as u64;

    // The events must not depend on the period size.
    let hashes: Vec<(u32, u64, usize)> = [1u32, 128, 4096].iter().map(|&p| {
        let mut log = EventLog::default();
        play(&view, frames, p, |_, _, block, player| player.process(&view, block, &mut log));
        (p, fnv(&log.bytes), log.notes)
    }).collect();
    let same = hashes.windows(2).all(|w| w[0].1 == w[1].1);

    // The audio, at the chosen period size.
    let mut engine = Engine::new(RATE, 32);
    engine.set_view(EngineView::build(&sounds(), RATE));
    let mut log = EventLog::default();
    let mut left = vec![0.0f32; 8192];
    let mut right = vec![0.0f32; 8192];
    let mut pcm = Vec::with_capacity(frames as usize * 4);
    play(&view, frames, period, |_, n, block, player| {
        let mut sink = SamplerSink { engine: &mut engine, log: &mut log };
        player.process(&view, block, &mut sink);
        let (l, r) = (&mut left[..n as usize], &mut right[..n as usize]);
        engine.render(l, r);
        for (a, b) in l.iter().zip(r.iter()) {
            for s in [a, b] {
                let v = ((s * 0.7).clamp(-1.0, 1.0) * 32767.0).round() as i16;
                pcm.extend_from_slice(&v.to_le_bytes());
            }
        }
    });
    let wav = wav(&pcm);
    if let Some(dir) = std::path::Path::new(&out).parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).unwrap_or_else(|e| fail(&format!("cannot make {}: {e}", dir.display())));
        }
    }
    std::fs::write(&out, &wav).unwrap_or_else(|e| fail(&format!("cannot write {out}: {e}")));

    println!("wrote {out}: {:.1} s, 44.1 kHz stereo 16-bit, period {period} frames", frames as f64 / f64::from(RATE));
    println!("notes played: {}  dropped by the sampler: {}", log.notes, engine.stats().dropped_events);
    println!("events {:016x}  (the same on every computer)", fnv(&log.bytes));
    println!("audio  {:016x}  (repeats on this computer)", fnv(&wav));
    for (p, h, n) in &hashes {
        println!("  events at period {p:>4}: {h:016x} ({n} notes)");
    }
    println!("same events for every period size: {}", if same && fnv(&log.bytes) == hashes[0].1 { "yes" } else { "NO" });
    if !same || fnv(&log.bytes) != hashes[0].1 {
        std::process::exit(1);
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("render: {msg}");
    std::process::exit(2);
}

/// Run the transport and the player for `frames` frames in periods of
/// `period`, calling `each` after the transport fills each block.
fn play(view: &PlayView, frames: u64, period: u32, mut each: impl FnMut(&PlayView, u32, &Block, &mut Player)) {
    let mut t = Transport::new(RATE, BPM).expect("valid transport");
    t.command(0, Command::SetSwing(Swing::new(240, 56).expect("valid swing")));
    t.command(0, Command::SetLoop(Some((0, 4 * BAR))));
    t.command(0, Command::Play);
    // Stop just before the end so every note ends.
    t.command(frames - RATE as u64 / 2, Command::Stop);
    let mut player = Player::new();
    let mut block = Block::new();
    while t.frame() < frames {
        let n = period.min((frames - t.frame()) as u32);
        t.process(n, &mut block);
        each(view, n, &block, &mut player);
    }
}

#[derive(Default)]
struct EventLog {
    bytes: Vec<u8>,
    notes: usize,
}

impl EventLog {
    fn add(&mut self, o: &Out) {
        let (kind, note, vel) = match o.event {
            Event::NoteOn { note, velocity } => (1u8, note, velocity),
            Event::NoteOff { note } => (0, note, 0),
            Event::AllOff => (2, 0, 0),
        };
        if kind == 1 {
            self.notes += 1;
        }
        self.bytes.extend_from_slice(&o.frame.to_le_bytes());
        self.bytes.extend_from_slice(&o.track.to_le_bytes());
        self.bytes.extend_from_slice(&[kind, note, vel]);
    }
}

impl Sink for EventLog {
    fn send(&mut self, o: Out) {
        self.add(&o);
    }
}

/// Every track plays the sampler; a note number is a pad.
struct SamplerSink<'a> {
    engine: &'a mut Engine,
    log: &'a mut EventLog,
}

impl Sink for SamplerSink<'_> {
    fn send(&mut self, o: Out) {
        self.log.add(&o);
        let ev = match o.event {
            Event::NoteOn { note, velocity } => SEvent::NoteOn { pad: note, velocity },
            Event::NoteOff { note } => SEvent::NoteOff { pad: note },
            Event::AllOff => SEvent::AllOff,
        };
        self.engine.schedule(o.offset, ev);
    }
}

fn notes(list: &[(i64, i64, u8, u8)]) -> Vec<Note> {
    list.iter().map(|&(tick, len, note, velocity)| Note { tick, len, note, velocity }).collect()
}

/// The demo song: four tracks on the sampler's pads.
fn song() -> PlayView {
    const S: i64 = 240; // a 1/16
    let mut p = Project::new();
    let mut add = |name: &str, c: Clip| {
        let mut t = Track::new(name, 0);
        t.slots[0] = Some(c);
        p.tracks.push(t);
    };

    // Pads 0 kick, 1 snare.
    let mut drums = Clip::new("drums", BAR);
    drums.notes = notes(&[(0, S, 0, 120), (6 * S, S, 0, 90), (10 * S, S, 0, 110), (4 * S, S, 1, 115), (12 * S, S, 1, 118), (15 * S, S, 1, 50)]);
    add("drums", drums);

    // Pads 2 closed hat, 3 open hat: half a bar of 1/16s at double speed, so 1/32s.
    let mut hats = Clip::new("hats", BAR / 2);
    hats.speed = Speed::new(2, 1).expect("valid speed");
    hats.swing = Some(Swing::new(240, 62).expect("valid swing"));
    hats.notes = (0..8).map(|i| Note { tick: i * S, len: S / 2, note: if i == 6 { 3 } else { 2 }, velocity: [100, 50, 75, 45][i as usize % 4] }).collect();
    add("hats", hats);

    // Pads 4 to 8: bass on gated pads, so note lengths matter.
    let mut bass = Clip::new("bass", 2 * BAR);
    bass.notes = notes(&[(0, 3 * S, 4, 110), (3 * S, S, 4, 80), (6 * S, 2 * S, 6, 100), (10 * S, 4 * S, 5, 105), (16 * S, 3 * S, 4, 110), (20 * S, 2 * S, 7, 95), (24 * S, 6 * S, 8, 100), (30 * S, S, 7, 70)]);
    add("bass", bass);

    // Pads 9 to 12: a 7/16 pluck that drifts against the bar.
    let mut pluck = Clip::new("pluck", 7 * S);
    pluck.notes = notes(&[(0, S, 9, 90), (2 * S, S, 11, 70), (3 * S, S, 10, 80), (5 * S, 2 * S, 12, 85)]);
    add("pluck", pluck);

    let launches: Vec<Launch> = (0..4).map(|track| Launch { track, slot: 0, origin: if track == 3 { 2 * BAR } else { 0 } }).collect();
    PlayView::compile(&p, &launches).expect("valid song")
}

/// Synthesised sounds on pads 0 to 12.
fn sounds() -> SProject {
    let rate = RATE as f32;
    let mut noise_state = 0x1234_5678u32;
    let mut noise = move || {
        noise_state ^= noise_state << 13;
        noise_state ^= noise_state >> 17;
        noise_state ^= noise_state << 5;
        noise_state as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let make = |secs: f32, f: &mut dyn FnMut(f32) -> f32| -> Vec<f32> {
        let n = (secs * rate) as usize;
        (0..n).flat_map(|i| {
            let s = f(i as f32 / rate);
            [s, s]
        }).collect()
    };
    let mut phase = 0.0f32;
    let kick = make(0.4, &mut |t| {
        let f = 45.0 + 90.0 * (-t * 30.0).exp();
        phase += TAU * f / rate;
        phase.sin() * (-t * 7.0).exp()
    });
    let snare = make(0.25, &mut |t| (0.5 * noise() + 0.5 * (TAU * 190.0 * t).sin()) * (-t * 18.0).exp());
    let mut last = 0.0f32;
    let mut hat = |len: f32, decay: f32| make(len, &mut |t| {
        let n = noise();
        let hp = n - last;
        last = n;
        0.5 * hp * (-t * decay).exp()
    });
    let closed = hat(0.06, 70.0);
    let open = hat(0.35, 9.0);
    let tone = |hz: f32, secs: f32, bright: f32| -> Vec<f32> {
        let n = (secs * rate) as usize;
        (0..n).flat_map(|i| {
            let t = i as f32 / rate;
            let s = (TAU * hz * t).sin() + bright * (TAU * 2.0 * hz * t).sin() * 0.5 + bright * (TAU * 3.0 * hz * t).sin() * 0.25;
            let s = s * 0.45 * (-t * 1.5).exp();
            [s, s]
        }).collect()
    };
    let mut p = SProject::new();
    let mut put = |pad: usize, name: &str, data: Vec<f32>, gate: bool| {
        let s = Source::new(name, RATE, 2, data).expect("valid source");
        p.add_source(s.clone());
        let mut r = Region::whole(&s);
        r.mode = if gate { PlayMode::Gate } else { PlayMode::OneShot };
        let mut pad_def = Pad::new(name, Layer::new(r));
        if gate {
            pad_def.amp = Ahdsr { attack_ms: 2.0, hold_ms: 0.0, decay_ms: 120.0, sustain: 0.7, release_ms: 60.0 };
        }
        if pad == 2 || pad == 3 {
            pad_def.mute_group = Some(1); // the closed hat stops the open one
        }
        p.set_pad(pad, Some(pad_def), None).expect("valid pad");
    };
    put(0, "kick", kick, false);
    put(1, "snare", snare, false);
    put(2, "closed hat", closed, false);
    put(3, "open hat", open, false);
    // A minor pentatonic bass: A1, C2, D2, E2, G2.
    for (i, hz) in [55.0, 65.41, 73.42, 82.41, 98.0].iter().enumerate() {
        put(4 + i, "bass", tone(*hz, 2.0, 0.8), true);
    }
    // Pluck: A4, C5, D5, E5.
    for (i, hz) in [440.0, 523.25, 587.33, 659.26].iter().enumerate() {
        put(9 + i, "pluck", tone(*hz, 0.5, 0.3).iter().enumerate().map(|(k, s)| s * 0.5 * (-(k as f32) / rate * 6.0).exp()).collect(), false);
    }
    p
}

fn wav(pcm: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(44 + pcm.len());
    let data = pcm.len() as u32;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&2u16.to_le_bytes()); // stereo
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    b.extend_from_slice(pcm);
    b
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}
