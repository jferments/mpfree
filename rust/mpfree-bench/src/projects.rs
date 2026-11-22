//! Step 0.6 of the DSP quality plan (`docs/dev/design/dsp-quality.md`):
//! fixed test projects, each played through the whole chain as a song
//! plays it. The synths sit in the track host and add into the mixer's
//! track strips; the sampler's pads go into the mixer's pad channels and
//! on into their tracks; then every insert, the sends, the returns, the
//! master chain and Master safety. Each project is 16 beats at 120 BPM
//! (8 s) and loops.
//!
//! - **Dense synths:** four synth tracks of chords and an arpeggio (Poly
//!   Supersaw, Wavetable Morph Pad, FM Glass Keys, Analog Warm Pad), EQ and
//!   compression on two, all sending to the reverb.
//! - **Distortion chain:** a lead through Drive, Amp, Fuzz and Character's
//!   Iron, into the delay; a Reese bass through Drive.
//! - **Granular drums:** kick, snare, hats and clap on pads, two granular
//!   pads over a texture, into a drum track with compression and the
//!   transient shaper.
//! - **Long stems:** four 8 s stems recorded at 48 kHz on one-shot pads
//!   (two of them tuned), each with an EQ, into a track with compression
//!   and the limiter.
//! - **Bass into the limiter:** a distorted 808 line, an acid line and a
//!   kick, with the master's compressor and the limiter 9 dB hot.
//! - **Fast automation:** Poly's cutoff and resonance moved every 64
//!   frames, a filter insert's cutoff every 128, a pan every 256 and a
//!   volume every 512, with tremolo and auto-pan on a sync-sweep line.
//! - **All at once:** the six together in one mixer.
//!
//! Each project keeps its tracks and pads in "all at once", so that one is
//! the sum of the others. Each is a bench case (named [`PROJECT`]...): the
//! timings time it and the instruction gate counts it per block, like any
//! case. Every part plays from the first beat, so the gate's 200 blocks
//! after the warm-up hear all of them. The test below renders each once
//! through and compares a fingerprint of it with `golden/projects.txt`.

use mpfree_fx::ids::{self, mixer as mx};
use mpfree_fx::{
    Amp, AutoPan, Character, Cloud, Compressor, Delay, Drive, Eq, Fuzz, Limiter, Mixer,
    MixerConfig, MultiFilter, StripId, TransientShaper, Tremolo,
};
use mpfree_node::{Ctx, NodeEvent, Timed, MAX_BLOCK};
use mpfree_sampler::engine::{Engine, EngineView, Event, PadOut};
use mpfree_sampler::model::{GrainParams, Layer, Pad, PlayMode, Project as Kit, Region, Source};
use mpfree_synth::params::{CUTOFF, RESONANCE};
use mpfree_track::{Instrument, TrackHost};
use std::sync::OnceLock;

use crate::cases::{keyboard_presets, Case};

/// What every project case's name starts with.
pub const PROJECT: &str = "Project: ";

/// A project's length: 16 beats at [`BPM`], then it loops.
pub const BEATS: f64 = 16.0;
const BPM: f64 = 120.0;

/// The projects, by the name each case takes after [`PROJECT`].
pub const ALL: [&str; 7] = [
    "dense synths",
    "distortion chain",
    "granular drums",
    "long stems",
    "bass into the limiter",
    "fast automation",
    "all at once",
];

/// The frames one pass of a project lasts at `rate`.
pub fn frames(rate: u32) -> u64 {
    beat_frame(BEATS, rate)
}

fn beat_frame(beat: f64, rate: u32) -> u64 {
    (beat * 60.0 / BPM * f64::from(rate)).round() as u64
}

/// One thing a project's score does at a frame.
#[derive(Clone, Copy, Debug)]
enum Ev {
    On {
        track: usize,
        note: u8,
        velocity: u8,
    },
    Off {
        track: usize,
        note: u8,
    },
    Param {
        track: usize,
        id: u16,
        value: f32,
    },
    Hit {
        pad: u8,
        velocity: u8,
    },
    PadOff {
        pad: u8,
    },
    Insert {
        strip: StripId,
        slot: usize,
        id: u16,
        value: f32,
    },
    Strip {
        strip: StripId,
        id: u16,
        value: f32,
    },
}

impl Ev {
    /// Endings sort before starts on the same frame.
    fn order(&self) -> u8 {
        match self {
            Ev::Off { .. } | Ev::PadOff { .. } => 0,
            _ => 1,
        }
    }
}

// ---------------------------------------------------------------------------
// The sounds: made once, at fixed rates (the drums and the texture at
// 44.1 kHz, the stems at 48 kHz), so every engine rate reads the same data.

const SOUND_RATE: u32 = 44_100;
const STEM_RATE: u32 = 48_000;

struct Sounds {
    kick: Source,
    snare: Source,
    hat: Source,
    clap: Source,
    texture: Source,
    stems: [Source; 4],
}

/// A small deterministic noise source.
struct Lcg(u32);

impl Lcg {
    fn unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 9) as f32 / (1u32 << 23) as f32 * 2.0 - 1.0
    }
}

/// A sine oscillator by rotation: no sine call a sample.
struct Osc {
    re: f64,
    im: f64,
    c: f64,
    s: f64,
}

impl Osc {
    fn new(hz: f64, rate: u32) -> Osc {
        let w = std::f64::consts::TAU * hz / f64::from(rate);
        Osc {
            re: 1.0,
            im: 0.0,
            c: w.cos(),
            s: w.sin(),
        }
    }
    fn next(&mut self) -> f32 {
        let (re, im) = (
            self.re * self.c - self.im * self.s,
            self.re * self.s + self.im * self.c,
        );
        // Keep the length at 1 against rounding.
        let k = 1.5 - 0.5 * (re * re + im * im);
        (self.re, self.im) = (re * k, im * k);
        self.im as f32
    }
}

fn hz(note: f64) -> f64 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}

/// Interleaved stereo from a mono render, `width` of it spread by a short
/// delay on the right.
fn stereo(mono: &[f32], spread: usize) -> Vec<f32> {
    let mut v = Vec::with_capacity(mono.len() * 2);
    for i in 0..mono.len() {
        v.push(mono[i]);
        v.push(if i >= spread { mono[i - spread] } else { 0.0 });
    }
    v
}

fn source(name: &str, rate: u32, data: Vec<f32>) -> Source {
    Source::new(name, rate, 2, data).expect("a sound")
}

fn kick(rate: u32, secs: f64) -> Vec<f32> {
    let n = (secs * f64::from(rate)) as usize;
    let mut phase = 0.0f64;
    (0..n)
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            phase += std::f64::consts::TAU * (45.0 + 110.0 * (-t / 0.04).exp()) / f64::from(rate);
            (0.9 * phase.sin() * (-t / 0.25).exp()) as f32
        })
        .collect()
}

fn make_sounds() -> Sounds {
    let r = SOUND_RATE;
    let at = |secs: f64, rate: u32| (secs * f64::from(rate)) as usize;
    let mut noise = Lcg(7);
    let snare: Vec<f32> = {
        let mut tone = Osc::new(185.0, r);
        (0..at(0.3, r))
            .map(|i| {
                let t = i as f32 / r as f32;
                0.6 * noise.unit() * (-t / 0.07).exp() + 0.5 * tone.next() * (-t / 0.05).exp()
            })
            .collect()
    };
    let hat: Vec<f32> = {
        let mut last = 0.0;
        (0..at(0.09, r))
            .map(|i| {
                let t = i as f32 / r as f32;
                let x = noise.unit();
                let y = x - last;
                last = x;
                0.4 * y * (-t / 0.02).exp()
            })
            .collect()
    };
    let clap: Vec<f32> = (0..at(0.25, r))
        .map(|i| {
            let t = i as f32 / r as f32;
            let burst = [0.0f32, 0.011, 0.023]
                .iter()
                .map(|&b| {
                    if t >= b {
                        (-(t - b) / 0.006).exp()
                    } else {
                        0.0
                    }
                })
                .sum::<f32>()
                + 0.4 * (-t / 0.08).exp();
            0.5 * noise.unit() * burst
        })
        .collect();
    // Two seconds of chord tones under slow beating and a little noise.
    let texture: Vec<f32> = {
        let mut oscs: Vec<Osc> = [57.0, 60.0, 64.0, 67.0, 71.0, 76.0]
            .iter()
            .map(|&n| Osc::new(hz(n) * 1.003, r))
            .collect();
        let mut lfo = Osc::new(0.7, r);
        (0..at(2.0, r))
            .map(|_| {
                let a = 0.6 + 0.4 * lfo.next();
                0.12 * a * oscs.iter_mut().map(Osc::next).sum::<f32>() + 0.02 * noise.unit()
            })
            .collect()
    };
    // The stems: 8 s at 48 kHz, as a song's audio comes in.
    let s = STEM_RATE;
    let len = at(8.0, s);
    let roots = [45.0, 41.0, 48.0, 43.0];
    let bar = len / 4;
    let chords: Vec<f32> = {
        let mut out = vec![0.0f32; len];
        for (k, &root) in roots.iter().enumerate() {
            let third = if k == 0 { 3.0 } else { 4.0 };
            for (j, iv) in [12.0, 12.0 + third, 19.0, 24.0].iter().enumerate() {
                // Eight harmonics at 1/n: a soft saw.
                let mut hs: Vec<Osc> = (1..=8)
                    .map(|h| Osc::new(hz(root + iv) * f64::from(h) * (1.0 + 0.001 * j as f64), s))
                    .collect();
                for (i, o) in out[k * bar..(k + 1) * bar].iter_mut().enumerate() {
                    let env = (i as f32 / 2_000.0).min(1.0) * (1.0 - i as f32 / bar as f32).sqrt();
                    let v: f32 = hs
                        .iter_mut()
                        .enumerate()
                        .map(|(h, o)| o.next() / (h + 1) as f32)
                        .sum();
                    *o += 0.07 * env * v;
                }
            }
        }
        out
    };
    let beat_loop: Vec<f32> = {
        let mut out = vec![0.0f32; len];
        let k = kick(s, 0.4);
        let beat = len / 16;
        for b in 0..16 {
            for (i, &x) in k.iter().enumerate() {
                if let Some(o) = out.get_mut(b * beat + i) {
                    *o += x;
                }
            }
            for half in 0..2 {
                let at0 = b * beat + half * beat / 2;
                for i in 0..at(0.05, s) {
                    if let Some(o) = out.get_mut(at0 + i) {
                        *o += 0.25 * noise.unit() * (-(i as f32) / 400.0).exp();
                    }
                }
            }
        }
        out
    };
    let bass: Vec<f32> = {
        let mut out = vec![0.0f32; len];
        for (k, &root) in roots.iter().enumerate() {
            for q in 0..8 {
                let note = root - 12.0 + if q % 4 == 3 { 12.0 } else { 0.0 };
                let mut f = Osc::new(hz(note), s);
                let mut f2 = Osc::new(2.0 * hz(note), s);
                let mut f3 = Osc::new(3.0 * hz(note), s);
                let start = k * bar + q * bar / 8;
                for i in 0..bar / 8 {
                    let env = (i as f32 / 100.0).min(1.0) * (-(i as f32) / 9_000.0).exp();
                    out[start + i] = 0.5 * env * (f.next() + 0.4 * f2.next() + 0.2 * f3.next());
                }
            }
        }
        out
    };
    let voice: Vec<f32> = {
        // A sung line: a buzz of 12 harmonics, a vowel-like hump at
        // 700 Hz and 1.1 kHz, vibrato from the second beat of each note.
        let mut out = vec![0.0f32; len];
        let line = [69.0, 72.0, 76.0, 74.0, 72.0, 71.0, 69.0, 67.0];
        let note_len = len / line.len();
        let mut vib = Osc::new(5.5, s);
        for (k, &n) in line.iter().enumerate() {
            let f0 = hz(n);
            let mut phase = 0.0f64;
            for i in 0..note_len {
                let v = vib.next() as f64 * if i > note_len / 4 { 0.006 } else { 0.0 };
                phase += std::f64::consts::TAU * f0 * (1.0 + v) / f64::from(s);
                let mut x = 0.0f64;
                for h in 1..=12 {
                    let fh = f0 * h as f64;
                    let g = (-((fh - 700.0) / 300.0).powi(2)).exp()
                        + 0.6 * (-((fh - 1_100.0) / 400.0).powi(2)).exp()
                        + 0.05;
                    x += g * (phase * h as f64).sin();
                }
                let env = (i as f64 / 800.0).min(1.0) * (1.0 - i as f64 / note_len as f64).sqrt();
                out[k * note_len + i] = (0.15 * env * x) as f32;
            }
        }
        out
    };
    Sounds {
        kick: source("kick", r, stereo(&kick(r, 0.45), 0)),
        snare: source("snare", r, stereo(&snare, 11)),
        hat: source("hat", r, stereo(&hat, 7)),
        clap: source("clap", r, stereo(&clap, 23)),
        texture: source("texture", r, stereo(&texture, 300)),
        stems: [
            source("chords", s, stereo(&chords, 480)),
            source("beat", s, stereo(&beat_loop, 0)),
            source("bass", s, stereo(&bass, 0)),
            source("voice", s, stereo(&voice, 96)),
        ],
    }
}

fn sounds() -> &'static Sounds {
    static SOUNDS: OnceLock<Sounds> = OnceLock::new();
    SOUNDS.get_or_init(make_sounds)
}

// ---------------------------------------------------------------------------
// Building a project.

/// Where each project's parts go, so "all at once" can hold them all.
mod at {
    /// Tracks 0 to 3: dense synths.
    pub const DENSE: usize = 0;
    /// Tracks 4 and 5: the distortion chain's lead and bass.
    pub const LEAD: usize = 4;
    /// Tracks 6 and 7: the 808 and the acid line.
    pub const LOW: usize = 6;
    /// Tracks 8 and 9: fast automation.
    pub const AUTO: usize = 8;
    /// The drum track (granular drums), the kick's track (bass into the
    /// limiter) and the stems' track.
    pub const DRUMS: usize = 10;
    pub const KICK: usize = 11;
    pub const STEMS: usize = 12;
    /// Pads: the drums 0 to 5, the limiter project's kick 6, the stems 8
    /// to 11.
    pub const DRUM_PADS: usize = 0;
    pub const KICK_PAD: usize = 6;
    pub const STEM_PADS: usize = 8;
}

/// The returns: A the reverb, B the delay.
const REVERB: usize = 0;
const DELAY: usize = 1;

struct Build {
    rate: u32,
    host: TrackHost,
    mixer: Mixer,
    kit: Kit,
    pads: bool,
    returns: [bool; 2],
    master: bool,
    score: Vec<(u64, Ev)>,
}

/// Chord roots for the four bars: A, F, C, G.
const ROOTS: [u8; 4] = [45, 41, 48, 43];

/// The chord on bar `k`: minor on A, major on the rest.
fn third(k: usize) -> u8 {
    if k == 0 {
        3
    } else {
        4
    }
}

impl Build {
    fn new(rate: u32) -> Build {
        let max = MAX_BLOCK.min(1024);
        Build {
            rate,
            host: TrackHost::new(16, rate, max),
            mixer: Mixer::new(MixerConfig::default(), rate, max),
            kit: Kit::new(),
            pads: false,
            returns: [false; 2],
            master: false,
            score: Vec::new(),
        }
    }

    fn at(&mut self, beat: f64, e: Ev) {
        let f = beat_frame(beat, self.rate);
        assert!(f < frames(self.rate), "{e:?} past the end");
        self.score.push((f, e));
    }

    /// A factory preset on `track`, `voices` voices where it has a voice
    /// count.
    fn synth(&mut self, track: usize, preset: &str, voices: usize) {
        let presets = keyboard_presets(self.rate);
        let p = presets
            .iter()
            .find(|p| p.name == preset)
            .unwrap_or_else(|| panic!("no preset {preset}"));
        let max = MAX_BLOCK.min(1024);
        let mut inst = Instrument::new((p.make)(), self.rate, max);
        for &(id, v) in &p.values {
            inst.node_mut().set_param(id, v);
        }
        if let Some(id) = p.polyphony {
            inst.node_mut().set_param(id, voices as f32);
        }
        self.host.swap(track, Some(inst)).expect("a free slot");
        self.host.route(track, Some(track));
    }

    fn note(&mut self, track: usize, beat: f64, len: f64, note: u8, velocity: u8) {
        self.at(
            beat,
            Ev::On {
                track,
                note,
                velocity,
            },
        );
        self.at(beat + len, Ev::Off { track, note });
    }

    /// A pad of one region into `track`'s strip.
    fn pad(&mut self, pad: usize, track: usize, region: Region, polyphony: u8) {
        let mut p = Pad::new("pad", Layer::new(region));
        p.polyphony = polyphony;
        self.kit.set_pad(pad, Some(p), None).expect("a pad");
        let s = StripId::Pad(pad);
        self.mixer.set_now(s, mx::OUTPUT, mx::output::TRACK);
        self.mixer.set_now(s, mx::OUTPUT_TRACK, track as f32);
        self.pads = true;
    }

    fn sound(&mut self, s: &Source) -> Region {
        self.kit.add_source(s.clone());
        Region::whole(s)
    }

    fn hit(&mut self, pad: usize, beat: f64, velocity: u8) {
        self.at(
            beat,
            Ev::Hit {
                pad: pad as u8,
                velocity,
            },
        );
    }

    fn insert(&mut self, s: StripId, node: Box<dyn mpfree_node::Node>) -> usize {
        self.mixer.chain_mut(s).expect("a strip").push(node)
    }

    fn set(&mut self, s: StripId, slot: usize, id: u16, value: f32) {
        self.mixer
            .chain_mut(s)
            .expect("a strip")
            .set_param(slot, id, value);
    }

    fn send(&mut self, s: StripId, ret: usize, db: f32) {
        self.mixer.set_now(s, mx::send(ret), db);
        self.returns[ret] = true;
    }

    // -- The projects --------------------------------------------------

    fn dense_synths(&mut self) {
        let t = at::DENSE;
        self.synth(t, "Poly Supersaw", 8);
        self.synth(t + 1, "Wavetable Morph Pad", 4);
        self.synth(t + 2, "FM Glass Keys", 8);
        self.synth(t + 3, "Analog Warm Pad", 4);
        for (k, &root) in ROOTS.iter().enumerate() {
            let b = 4.0 * k as f64;
            let th = third(k);
            for iv in [12, 19, 24, 24 + th, 31] {
                self.note(t, b, 3.9, root + iv, 96);
            }
            if k % 2 == 0 {
                for iv in [24, 24 + th, 31, 34] {
                    self.note(t + 1, b, 7.8, root + iv, 80);
                }
            }
            let arp = [24, 31, 36, 36 + th, 43, 36, 31, 24 + th];
            for (j, &iv) in arp.iter().enumerate() {
                self.note(
                    t + 2,
                    b + 0.5 * j as f64,
                    0.45,
                    root + iv,
                    70 + (j as u8 * 7) % 30,
                );
            }
            for half in 0..2 {
                for iv in [12 + th, 19, 24] {
                    self.note(t + 3, b + 2.0 * half as f64, 1.9, root + iv, 72);
                }
            }
        }
        let s = StripId::Track(t);
        self.insert(s, Box::new(Eq::new()));
        self.insert(s, Box::new(Compressor::new()));
        self.insert(StripId::Track(t + 1), Box::new(Eq::new()));
        self.mixer.set_now(StripId::Track(t + 2), mx::PAN, -30.0);
        self.mixer.set_now(StripId::Track(t + 3), mx::PAN, 30.0);
        for k in 0..4 {
            self.mixer.set_now(StripId::Track(t + k), mx::VOLUME, -6.0);
            self.send(StripId::Track(t + k), REVERB, -14.0);
        }
    }

    fn distortion_chain(&mut self) {
        let (lead, bass) = (at::LEAD, at::LEAD + 1);
        self.synth(lead, "Poly Lead", 1);
        self.synth(bass, "Bass Reese", 1);
        let line = [69u8, 72, 74, 76, 74, 72, 79, 76];
        for (k, &root) in ROOTS.iter().enumerate() {
            let b = 4.0 * k as f64;
            let shift = (root as i32 - 45).rem_euclid(12) as u8 % 5;
            for (j, &n) in line.iter().enumerate() {
                self.note(lead, b + 0.5 * j as f64, 0.45, n + shift, 100);
            }
            for q in 0..8 {
                let n = root - 12 + if q % 4 == 3 { 12 } else { 0 };
                self.note(bass, b + 0.5 * q as f64, 0.4, n, 110);
            }
        }
        let s = StripId::Track(lead);
        let d = self.insert(s, Box::new(Drive::new()));
        self.set(s, d, ids::drive::DRIVE, 24.0);
        let a = self.insert(s, Box::new(Amp::new()));
        self.set(s, a, ids::amp::GAIN, 8.0);
        let f = self.insert(s, Box::new(Fuzz::new()));
        self.set(s, f, ids::fuzz::FUZZ, 70.0);
        let c = self.insert(s, Box::new(Character::new()));
        self.set(s, c, ids::character::MODE, ids::character::IRON);
        self.set(s, c, ids::character::AMOUNT, 80.0);
        self.mixer.set_now(s, mx::VOLUME, -12.0);
        self.send(s, DELAY, -12.0);
        let b = StripId::Track(bass);
        let d = self.insert(b, Box::new(Drive::new()));
        self.set(b, d, ids::drive::DRIVE, 18.0);
        self.mixer.set_now(b, mx::VOLUME, -8.0);
    }

    fn granular_drums(&mut self) {
        let p = at::DRUM_PADS;
        let snd = sounds();
        for (k, s) in [&snd.kick, &snd.snare, &snd.hat, &snd.clap]
            .into_iter()
            .enumerate()
        {
            let r = self.sound(s);
            self.pad(p + k, at::DRUMS, r, 2);
        }
        for (k, (size, density, pitch)) in [(250.0, 60.0, 0.0), (80.0, 200.0, 7.0)]
            .into_iter()
            .enumerate()
        {
            let mut r = self.sound(&snd.texture);
            r.mode = PlayMode::Granular;
            r.grain = GrainParams {
                spray: 0.5,
                size_ms: size,
                density,
                pitch_spray: pitch,
                spread: 1.0,
                scan: 1.0,
                ..GrainParams::default()
            };
            self.pad(p + 4 + k, at::DRUMS, r, 2);
            self.send(StripId::Pad(p + 4 + k), REVERB, -10.0);
        }
        for bar in 0..4 {
            let b = 4.0 * bar as f64;
            for beat in 0..4 {
                self.hit(p, b + beat as f64, 120);
            }
            if bar % 2 == 1 {
                self.hit(p, b + 2.5, 90);
            }
            self.hit(p + 1, b + 1.0, 110);
            self.hit(p + 1, b + 3.0, 110);
            for s in 0..16 {
                self.hit(p + 2, b + 0.25 * s as f64, if s % 2 == 0 { 90 } else { 50 });
            }
        }
        self.hit(p + 3, 15.5, 100);
        self.hit(p + 4, 0.0, 100);
        self.at(7.5, Ev::PadOff { pad: (p + 4) as u8 });
        self.hit(p + 5, 8.0, 100);
        self.at(15.5, Ev::PadOff { pad: (p + 5) as u8 });
        let s = StripId::Track(at::DRUMS);
        self.insert(s, Box::new(Compressor::new()));
        self.insert(s, Box::new(TransientShaper::new()));
        self.send(s, REVERB, -20.0);
    }

    fn long_stems(&mut self) {
        let p = at::STEM_PADS;
        let snd = sounds();
        for (k, tune) in [0, 0, -200, 300].into_iter().enumerate() {
            let mut r = self.sound(&snd.stems[k]);
            r.tune_cents = tune;
            self.pad(p + k, at::STEMS, r, 1);
            self.insert(StripId::Pad(p + k), Box::new(Eq::new()));
            self.hit(p + k, 0.0, 100);
        }
        let s = StripId::Track(at::STEMS);
        self.insert(s, Box::new(Compressor::new()));
        let l = self.insert(s, Box::new(Limiter::new()));
        self.set(s, l, ids::limiter::GAIN, 6.0);
        self.mixer.set_now(s, mx::VOLUME, -6.0);
    }

    fn bass_into_the_limiter(&mut self) {
        let (b808, acid) = (at::LOW, at::LOW + 1);
        self.synth(b808, "Bass Distorted 808", 1);
        self.synth(acid, "Squelch Acid Line", 1);
        let r = self.sound(&sounds().kick);
        self.pad(at::KICK_PAD, at::KICK, r, 2);
        for (k, &root) in ROOTS.iter().enumerate() {
            let b = 4.0 * k as f64;
            for beat in 0..4 {
                self.note(b808, b + beat as f64, 0.9, root - 12, 120);
                self.hit(at::KICK_PAD, b + beat as f64, 127);
            }
            for s in 0..16 {
                let n = root + [0, 12, 0, 7, 0, 12, 10, 0][s % 8];
                let v = if s % 4 == 0 { 120 } else { 80 };
                self.note(acid, b + 0.25 * s as f64, 0.2, n, v);
            }
        }
        self.master = true;
    }

    fn fast_automation(&mut self) {
        let (pads, line) = (at::AUTO, at::AUTO + 1);
        self.synth(pads, "Poly Supersaw", 4);
        self.synth(line, "Wavetable Sync Sweep", 4);
        for (k, &root) in ROOTS.iter().enumerate() {
            let b = 4.0 * k as f64;
            for iv in [12, 19, 24, 24 + third(k)] {
                self.note(pads, b, 3.9, root + iv, 90);
            }
            for j in 0..8 {
                let n = root + 24 + [0, 7, 12, 7][j % 4];
                self.note(line, b + 0.5 * j as f64, 0.4, n, 90);
            }
        }
        let rate = self.rate;
        let end = frames(rate);
        let secs = |f: u64| f as f64 / f64::from(rate);
        let tau = std::f64::consts::TAU;
        let s = StripId::Track(pads);
        let filt = self.insert(s, Box::new(MultiFilter::new()));
        let l = StripId::Track(line);
        let trem = self.insert(l, Box::new(Tremolo::new()));
        self.set(l, trem, ids::tremolo::SYNC, 5.0);
        self.set(l, trem, ids::tremolo::DEPTH, 60.0);
        self.insert(l, Box::new(AutoPan::new()));
        self.send(l, DELAY, -14.0);
        let mut f = 0;
        while f < end {
            let t = secs(f);
            // Poly's cutoff, 200 Hz to 12 kHz and back every 2 s, and its
            // resonance, every 64 frames.
            let x = 0.5 - 0.5 * (tau * t / 2.0).cos();
            self.score.push((
                f,
                Ev::Param {
                    track: pads,
                    id: CUTOFF,
                    value: (200.0 * 60f64.powf(x)) as f32,
                },
            ));
            self.score.push((
                f,
                Ev::Param {
                    track: pads,
                    id: RESONANCE,
                    value: (30.0 + 25.0 * (tau * t / 3.0).sin()) as f32,
                },
            ));
            if f % 128 == 0 {
                let y = 0.5 - 0.5 * (tau * t).cos();
                self.score.push((
                    f,
                    Ev::Insert {
                        strip: s,
                        slot: filt,
                        id: ids::filter::CUTOFF,
                        value: (300.0 * 25f64.powf(y)) as f32,
                    },
                ));
            }
            if f % 256 == 0 {
                self.score.push((
                    f,
                    Ev::Strip {
                        strip: l,
                        id: mx::PAN,
                        value: (80.0 * (tau * t / 2.0).sin()) as f32,
                    },
                ));
            }
            if f % 512 == 0 {
                self.score.push((
                    f,
                    Ev::Strip {
                        strip: s,
                        id: mx::VOLUME,
                        value: (-12.0 + 6.0 * (1.0 - (tau * t / 4.0).cos())) as f32,
                    },
                ));
            }
            f += 64;
        }
    }

    /// The returns, the master chain and the sampler, then the case.
    fn finish(mut self, name: String) -> Case {
        if self.returns[REVERB] {
            let s = StripId::Return(REVERB);
            let c = self.insert(s, Box::new(Cloud::new()));
            self.set(s, c, ids::cloud::MIX, 100.0);
        }
        if self.returns[DELAY] {
            let s = StripId::Return(DELAY);
            let d = self.insert(s, Box::new(Delay::new()));
            self.set(s, d, ids::delay::MIX, 100.0);
        }
        if self.master {
            let s = StripId::Master;
            self.insert(s, Box::new(Compressor::new()));
            let l = self.insert(s, Box::new(Limiter::new()));
            self.set(s, l, ids::limiter::GAIN, 9.0);
        }
        let rate = self.rate;
        let mut sampler = self.pads.then(|| {
            let mut e = Engine::new(rate, 32);
            e.set_view(EngineView::build(&self.kit, rate));
            e
        });
        self.score.sort_by_key(|(f, e)| (*f, e.order()));
        let (mut host, mut mixer, score) = (self.host, self.mixer, self.score);
        let len = frames(rate);
        let mut next = 0usize;
        Case {
            name,
            module: "period",
            what: "a fixed project through the track host, the sampler's pads and the whole mixer",
            block: Box::new(move |b, l, r| {
                let n = l.len() as u64;
                let f0 = b * n;
                // The score loops: play what falls in this block, wrapping.
                let start = f0 % len;
                let mut play = |from: u64, to: u64, shift: u64, next: &mut usize| {
                    while *next < score.len() && score[*next].0 < to {
                        let (f, e) = score[*next];
                        *next += 1;
                        if f < from {
                            continue;
                        }
                        let off = (f + shift - start) as u32;
                        apply(e, off, &mut host, sampler.as_mut(), &mut mixer);
                    }
                };
                if start + n <= len {
                    play(start, start + n, 0, &mut next);
                } else {
                    play(start, len, 0, &mut next);
                    next = 0;
                    play(0, start + n - len, len, &mut next);
                }
                let ctx = Ctx {
                    bpm: BPM,
                    beats: f0 as f64 * BPM / 60.0 / f64::from(rate),
                    playing: true,
                    ..Ctx::new(rate, f0)
                };
                host.render(&ctx, l.len(), &mut mixer);
                if let Some(e) = sampler.as_mut() {
                    e.render_routed(l.len(), &mut Pads(&mut mixer));
                }
                mixer.process(&ctx, l, r);
            }),
            mem_bytes: 0,
        }
    }
}

/// One score event at `offset` frames into the block.
fn apply(e: Ev, offset: u32, host: &mut TrackHost, sampler: Option<&mut Engine>, m: &mut Mixer) {
    match e {
        Ev::On {
            track,
            note,
            velocity,
        } => {
            host.note_on(track, offset, note, velocity);
        }
        Ev::Off { track, note } => {
            host.note_off(track, offset, note);
        }
        Ev::Param { track, id, value } => {
            host.param(track, offset, id, value);
        }
        // The sampler takes its pads at the start of the block, as the
        // pad player hands them over.
        Ev::Hit { pad, velocity } => {
            if let Some(s) = sampler {
                s.apply(Event::NoteOn { pad, velocity });
            }
        }
        Ev::PadOff { pad } => {
            if let Some(s) = sampler {
                s.apply(Event::NoteOff { pad });
            }
        }
        Ev::Insert {
            strip,
            slot,
            id,
            value,
        } => {
            m.queue_insert(
                strip,
                slot,
                Timed::new(offset, NodeEvent::Param { id, value }),
            );
        }
        Ev::Strip { strip, id, value } => {
            m.queue(strip, Timed::new(offset, NodeEvent::Param { id, value }));
        }
    }
}

/// The sampler's pads into the mixer's pad channels.
struct Pads<'a>(&'a mut Mixer);

impl PadOut for Pads<'_> {
    fn pad(&mut self, _kit: usize, pad: usize) -> Option<(&mut [f32], &mut [f32])> {
        self.0.pad_input(pad)
    }
}

/// The project called `name` (one of [`ALL`]) as a bench case.
pub fn case(name: &str, rate: u32) -> Case {
    let mut b = Build::new(rate);
    let all = name == "all at once";
    if all || name == "dense synths" {
        b.dense_synths();
    }
    if all || name == "distortion chain" {
        b.distortion_chain();
    }
    if all || name == "granular drums" {
        b.granular_drums();
    }
    if all || name == "long stems" {
        b.long_stems();
    }
    if all || name == "bass into the limiter" {
        b.bass_into_the_limiter();
    }
    if all || name == "fast automation" {
        b.fast_automation();
    }
    assert!(ALL.contains(&name), "no project {name}");
    b.finish(format!("{PROJECT}{name}"))
}

#[cfg(test)]
mod tests {
    //! The full-project check: each project rendered once through (its 16
    //! beats, then two seconds of its tail as the loop starts again) and
    //! fingerprinted, against `golden/projects.txt`. A fingerprint is per
    //! half second and channel: the level and the octave bands from 63 Hz
    //! to 16 kHz, in dB. It is compared within 0.1 dB of level and 0.25 dB
    //! a band (bands more than 60 dB under the half second's loudest, or
    //! under -100 dBFS, are left out), so another libm passes while any
    //! change a listener could hear, and most a meter could see, fails.
    //! Each line also carries a hash of the render's bits, which says
    //! whether a change left the sound bit-identical (reported, not
    //! gated: another platform's libm changes the bits).
    //!
    //! When the sound changes on purpose, bless and say why in the PR:
    //!
    //!     MPFREE_GOLDEN_BLESS=1 cargo test --release -p mpfree-bench projects
    use super::*;
    use std::fmt::Write as _;

    const RATE: u32 = 44_100;
    const BLOCK: usize = 128;
    const SEG_S: f64 = 0.5;
    const FFT: usize = 4096;
    /// Octave band centres.
    const BANDS: [f64; 9] = [
        63.0, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0,
    ];
    const LEVEL_TOL_DB: f64 = 0.1;
    const BAND_TOL_DB: f64 = 0.25;
    const BAND_RANGE_DB: f64 = 60.0;
    const FLOOR_DB: f64 = -100.0;

    /// One half second of one channel: the level, then each band.
    type Seg = Vec<f64>;

    struct Print {
        segs: [Vec<Seg>; 2],
        hash: u64,
    }

    fn render(name: &str) -> (Vec<f32>, Vec<f32>) {
        let mut c = case(name, RATE);
        let total = frames(RATE) as usize + 2 * RATE as usize;
        let (mut left, mut right) = (Vec::with_capacity(total), Vec::with_capacity(total));
        let (mut l, mut r) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
        let mut b = 0u64;
        while left.len() < total {
            (c.block)(b, &mut l, &mut r);
            left.extend_from_slice(&l);
            right.extend_from_slice(&r);
            b += 1;
        }
        left.truncate(total);
        right.truncate(total);
        (left, right)
    }

    fn fft(re: &mut [f64], im: &mut [f64]) {
        let n = re.len();
        let mut j = 0;
        for i in 1..n {
            let mut bit = n >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j |= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let w = -std::f64::consts::TAU / len as f64;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let (c, s) = ((w * k as f64).cos(), (w * k as f64).sin());
                    let (a, b) = (start + k, start + k + len / 2);
                    let (xr, xi) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                    (re[b], im[b]) = (re[a] - xr, im[a] - xi);
                    (re[a], im[a]) = (re[a] + xr, im[a] + xi);
                }
            }
            len <<= 1;
        }
    }

    fn db(p: f64) -> f64 {
        (10.0 * p.max(1e-30).log10()).max(-200.0)
    }

    fn seg(x: &[f32]) -> Seg {
        let rms = x.iter().map(|&v| f64::from(v) * f64::from(v)).sum::<f64>() / x.len() as f64;
        let mut bands = [0.0f64; BANDS.len()];
        let hann: Vec<f64> = (0..FFT)
            .map(|i| 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / FFT as f64).cos())
            .collect();
        let windows = x.len() / FFT;
        for w in 0..windows {
            let mut re: Vec<f64> = x[w * FFT..(w + 1) * FFT]
                .iter()
                .zip(&hann)
                .map(|(&v, h)| f64::from(v) * h)
                .collect();
            let mut im = vec![0.0; FFT];
            fft(&mut re, &mut im);
            for k in 1..FFT / 2 {
                let f = k as f64 * f64::from(RATE) / FFT as f64;
                let p = re[k] * re[k] + im[k] * im[k];
                for (b, &c) in BANDS.iter().enumerate() {
                    if f >= c / 2f64.sqrt() && f < c * 2f64.sqrt() {
                        bands[b] += p;
                    }
                }
            }
        }
        // Scaled so a full-scale sine reads about 0 dB in its band.
        let scale = 4.0 / (FFT as f64 * 0.5).powi(2) / 2.0 / windows.max(1) as f64;
        let mut s = vec![db(rms)];
        s.extend(bands.iter().map(|&p| db(p * scale)));
        s
    }

    fn print(l: &[f32], r: &[f32]) -> Print {
        let n = (SEG_S * f64::from(RATE)) as usize;
        let segs = [l, r].map(|x| x.chunks_exact(n).map(seg).collect());
        // FNV-1a over both channels' bits.
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for v in l.iter().chain(r) {
            for byte in v.to_bits().to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
            }
        }
        Print { segs, hash }
    }

    fn line(name: &str, p: &Print) -> String {
        let mut s = format!("{name} | {:016x}", p.hash);
        for (ch, segs) in ["L", "R"].iter().zip(&p.segs) {
            write!(s, " | {ch}").unwrap();
            for g in segs {
                s.push(' ');
                let vals: Vec<String> = g.iter().map(|v| format!("{v:.2}")).collect();
                s.push_str(&vals.join("/"));
            }
        }
        s
    }

    fn parse(line: &str) -> Option<(String, Print)> {
        let mut parts = line.split(" | ");
        let name = parts.next()?.to_string();
        let hash = u64::from_str_radix(parts.next()?, 16).ok()?;
        let mut segs = [Vec::new(), Vec::new()];
        for (k, part) in parts.enumerate() {
            let mut toks = part.split_whitespace();
            toks.next()?;
            for t in toks {
                let g: Option<Seg> = t.split('/').map(|v| v.parse().ok()).collect();
                segs.get_mut(k)?.push(g?);
            }
        }
        Some((name, Print { segs, hash }))
    }

    fn differs(want: &Print, got: &Print) -> Option<String> {
        for (ch, w, g) in [
            ("left", &want.segs[0], &got.segs[0]),
            ("right", &want.segs[1], &got.segs[1]),
        ] {
            if w.len() != g.len() {
                return Some(format!("{ch}: {} half seconds, not {}", g.len(), w.len()));
            }
            for (i, (a, b)) in w.iter().zip(g).enumerate() {
                let t = i as f64 * SEG_S;
                let silent = a[0] < FLOOR_DB && b[0] < FLOOR_DB;
                if !silent && (a[0] - b[0]).abs() > LEVEL_TOL_DB {
                    return Some(format!(
                        "{ch} at {t:.1} s: level {:.2} dB, not {:.2}",
                        b[0], a[0]
                    ));
                }
                let top = a[1..].iter().cloned().fold(f64::MIN, f64::max);
                for (k, (&x, &y)) in a[1..].iter().zip(&b[1..]).enumerate() {
                    let heard = x > top - BAND_RANGE_DB && x > FLOOR_DB;
                    if heard && (x - y).abs() > BAND_TOL_DB {
                        return Some(format!(
                            "{ch} at {t:.1} s: the {} Hz band {y:.2} dB, not {x:.2}",
                            BANDS[k]
                        ));
                    }
                }
            }
        }
        None
    }

    fn reference_path() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("golden/projects.txt")
    }

    const HEADER: &str = "\
# mpfree-bench full-project fingerprints (DSP quality step 0.6): each project's
# 16 beats and 2 s more at 44.1 kHz, per half second and channel: level, then
# the octave bands 63 Hz to 16 kHz, in dB; after the name, a hash of the bits.
# Written by
#   MPFREE_GOLDEN_BLESS=1 cargo test --release -p mpfree-bench projects
# Compared within 0.1 dB of level and 0.25 dB a band (src/projects.rs).
";

    #[test]
    #[cfg_attr(
        debug_assertions,
        ignore = "release only: cargo test --release --locked -p mpfree-bench projects"
    )]
    fn projects_sound_as_approved() {
        let got: Vec<(String, Print)> = std::thread::scope(|s| {
            let handles: Vec<_> = ALL
                .iter()
                .map(|&name| {
                    s.spawn(move || {
                        let (l, r) = render(name);
                        for (i, v) in l.iter().chain(&r).enumerate() {
                            assert!(v.is_finite(), "{name}: sample {i} is not finite");
                        }
                        (name.to_string(), print(&l, &r))
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for (name, p) in &got {
            let loudest = p
                .segs
                .iter()
                .flatten()
                .map(|g| g[0])
                .fold(f64::MIN, f64::max);
            println!(
                "{name}: loudest half second {loudest:.1} dBFS RMS, bits {:016x}",
                p.hash
            );
            assert!(loudest > -40.0, "{name} is nearly silent");
        }
        if std::env::var_os("MPFREE_GOLDEN_BLESS").is_some() {
            let mut text = HEADER.to_string();
            for (name, p) in &got {
                text.push_str(&line(name, p));
                text.push('\n');
            }
            std::fs::write(reference_path(), text).unwrap();
            eprintln!("wrote {}", reference_path().display());
            return;
        }
        let text = std::fs::read_to_string(reference_path()).unwrap_or_else(|e| {
            panic!(
                "{}: {e} (bless with MPFREE_GOLDEN_BLESS=1)",
                reference_path().display()
            )
        });
        let want: Vec<(String, Print)> = text
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .map(|l| parse(l).unwrap_or_else(|| panic!("bad line in golden/projects.txt: {l}")))
            .collect();
        let mut problems = Vec::new();
        for (name, p) in &got {
            match want.iter().find(|(n, _)| n == name) {
                None => problems.push(format!("{name}: new project, no reference")),
                Some((_, w)) => {
                    let same = if w.hash == p.hash {
                        "bit-identical"
                    } else {
                        "not bit-identical"
                    };
                    println!("{name}: {same} to the approved render");
                    if let Some(why) = differs(w, p) {
                        problems.push(format!("{name}: {why}"));
                    }
                }
            }
        }
        assert!(
            problems.is_empty(),
            "{} projects sound different from golden/projects.txt:\n{}\n\
             If the change is meant, bless it and say why in the PR: \
             MPFREE_GOLDEN_BLESS=1 cargo test --release -p mpfree-bench projects",
            problems.len(),
            problems.join("\n")
        );
    }

    #[test]
    fn fingerprints_read_back_and_compare() {
        let p = Print {
            segs: [
                vec![vec![-12.0, -20.0, -30.0, -150.0]],
                vec![vec![-110.0, -120.0, -130.0, -140.0]],
            ],
            hash: 0xdead_beef,
        };
        let (name, back) = parse(&line("Project: test", &p)).unwrap();
        assert_eq!(name, "Project: test");
        assert_eq!(back.hash, p.hash);
        assert!(differs(&p, &back).is_none());
        let mut louder = Print {
            segs: back.segs.clone(),
            hash: 0,
        };
        louder.segs[0][0][0] += 0.2;
        assert!(differs(&p, &louder).unwrap().contains("level"));
        let mut band = Print {
            segs: p.segs.clone(),
            hash: 0,
        };
        band.segs[0][0][2] += 0.3;
        assert!(differs(&p, &band).unwrap().contains("125 Hz band"));
        // A band far under the loudest, or a silent half second, is not
        // compared.
        let mut quiet = Print {
            segs: p.segs.clone(),
            hash: 0,
        };
        quiet.segs[0][0][3] = -100.0;
        quiet.segs[1][0][0] = -105.0;
        assert!(differs(&p, &quiet).is_none());
    }

    #[test]
    fn every_project_builds_and_the_last_holds_the_rest() {
        let mut all = case("all at once", RATE);
        assert_eq!(all.name, "Project: all at once");
        let (mut l, mut r) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
        (all.block)(0, &mut l, &mut r);
        for name in ALL {
            let c = case(name, RATE);
            assert!(c.name.starts_with(PROJECT));
        }
    }
}
