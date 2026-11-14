//! The workloads: each engine set up the way a busy song would use it, fed
//! block by block at the MPC's period. Setup allocates; the block function
//! must not.

use mpfree_audiotracks::host::COPY_SAMPLES;
use mpfree_audiotracks::{
    AudioTracks, Command as TrackCommand, Input as TrackInput, Io as TrackIo, MAX_TRACKS,
};
use mpfree_fx::ids;
use mpfree_fx::{
    CapturePoint, Chain, Character, Chorus, Cloud, Comb, Compressor, Crush, Delay, DjFilter, Drive,
    Eq, Flanger, Frozen, LoFi, MasterSafety, Mixer, MixerConfig, MultiFilter, Perform, Phaser,
    Print, PrintJob, PrintState, StripId, Tail,
};
use mpfree_grain::GrainNode;
use mpfree_lanes::{Config as LaneConfig, Job, Lane};
use mpfree_looper::{
    Command as LooperCommand, Edit as LooperEdit, Looper, Phase as LooperPhase, MAX_LOOPS,
};
use mpfree_node::{Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Timed, MAX_BLOCK};
use mpfree_play::{Launch, Out, PlayView, Player, Sink, TrackArps};
use mpfree_prof::alloc;
use mpfree_prof::budget::MPC;
use mpfree_sampler::dsp::{Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;
use mpfree_seq::mpfree_param::{OnTrack, RelAddr};
use mpfree_seq::{Clip, Lock, Note, Project as Song, Quantise, Speed, Track};
use mpfree_synth::params::{POLYPHONY, UNISON, VOICE_MODE};
use mpfree_synth::wavetable::params::{self as wt, PRESETS as WT_PRESETS};
use mpfree_synth::{Drawbar, Drums, Keys, Pluck, Poly, Squelch, Wavetable, PRESETS};
use mpfree_track::{Instrument, Sum, TrackHost};
use mpfree_transport::time::{Swing, PPQ};
use mpfree_transport::{Block as TBlock, Command as T, Transport};

use std::time::{Duration, Instant};

pub type Block = Box<dyn FnMut(u64, &mut [f32], &mut [f32])>;

pub struct Case {
    pub name: String,
    /// The budget it counts against (see `mpfree_prof::budget`); "period"
    /// is the whole audio thread.
    pub module: &'static str,
    /// What it plays, for the report.
    pub what: &'static str,
    pub block: Block,
    /// Bytes the engine holds (allocated while it was built and not freed),
    /// when the counting allocator is installed; else 0.
    pub mem_bytes: i64,
}

/// A small deterministic noise source.
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 9) as f32 / (1u32 << 23) as f32 * 2.0 - 1.0
    }
}

/// Program material for effects: a 110 Hz tone under noise bursts that
/// start every half second and fall away, so dynamics, tails and silence
/// all happen.
fn input(rate: u32, frame0: u64, seed: &mut Lcg, l: &mut [f32], r: &mut [f32]) {
    let half = u64::from(rate / 2);
    for (i, (a, b)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
        let f = frame0 + i as u64;
        let t = (f % half) as f32 / rate as f32;
        let env = (-t * 12.0).exp();
        let tone = (f as f32 * 110.0 * std::f32::consts::TAU / rate as f32).sin() * 0.2;
        let n = seed.unit() * env * 0.5;
        *a = tone + n;
        *b = tone - 0.8 * n;
    }
}

/// One second of [`input`], made once and shared, so a case's timing is the
/// engine's alone: making it costs a `sin` and an `exp` per frame, about
/// 20 to 25 us a block on the MPC, which round 7 counted in every effect.
/// (The tone and the bursts both repeat exactly every second.)
pub struct Program {
    l: Vec<f32>,
    r: Vec<f32>,
}

thread_local! {
    static PROGRAM: std::cell::RefCell<Option<(u32, std::rc::Rc<Program>)>> = const { std::cell::RefCell::new(None) };
}

/// The shared program for `rate`, made on first use. [`all`] makes it before
/// it builds any case, so no case's memory includes it.
pub fn program(rate: u32) -> std::rc::Rc<Program> {
    PROGRAM.with(|p| {
        let mut p = p.borrow_mut();
        if let Some((r, prog)) = p.as_ref() {
            if *r == rate {
                return prog.clone();
            }
        }
        let n = rate as usize;
        let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
        input(rate, 0, &mut Lcg(11), &mut l, &mut r);
        let prog = std::rc::Rc::new(Program { l, r });
        *p = Some((rate, prog.clone()));
        prog
    })
}

impl Program {
    /// Copies the program from frame `frame0` (wrapping every second).
    fn play(&self, frame0: u64, l: &mut [f32], r: &mut [f32]) {
        let len = self.l.len();
        let mut at = (frame0 % len as u64) as usize;
        let mut done = 0;
        while done < l.len() {
            let k = (l.len() - done).min(len - at);
            l[done..done + k].copy_from_slice(&self.l[at..at + k]);
            r[done..done + k].copy_from_slice(&self.r[at..at + k]);
            done += k;
            at = (at + k) % len;
        }
    }
}

fn effect(name: &str, rate: u32, mut n: Box<dyn Node>, params: &[(u16, f32)]) -> Case {
    n.prepare(rate, MAX_BLOCK);
    for &(id, v) in params {
        n.set_param(id, v);
    }
    n.reset();
    let prog = program(rate);
    Case {
        name: name.to_string(),
        module: "fx",
        what: "tone and noise bursts every 0.5 s",
        block: Box::new(move |b, l, r| {
            let f0 = b * l.len() as u64;
            prog.play(f0, l, r);
            n.process(&Ctx::new(rate, f0), &[], l, r);
        }),
        mem_bytes: 0,
    }
}

/// Perform FX on a song: `pads` held from the start, or (with `cycle`) one
/// pad after another, a new one every half second, so entries, releases
/// and seams are all in the numbers.
fn perform(name: &str, rate: u32, pads: &'static [u8], cycle: bool) -> Case {
    let mut p = Perform::new();
    p.prepare(rate, MAX_BLOCK);
    p.reset();
    let prog = program(rate);
    let mut events: Vec<Timed> = Vec::with_capacity(2 * pads.len());
    let mut k = 0usize;
    Case {
        name: name.to_string(),
        module: "fx",
        what: if cycle {
            "tone and noise bursts every 0.5 s, a new pad every 0.5 s"
        } else {
            "tone and noise bursts every 0.5 s, pads held"
        },
        block: Box::new(move |b, l, r| {
            let len = l.len() as u64;
            let f0 = b * len;
            events.clear();
            if cycle {
                let half = u64::from(rate / 2);
                if f0 % half + len > half || f0 == 0 {
                    let at = if f0 == 0 {
                        0
                    } else {
                        (half - f0 % half) as u32 % len as u32
                    };
                    let note = pads[k % pads.len()];
                    events.push(Timed::new(at, NodeEvent::NoteOff { note }));
                    k += 1;
                    let note = pads[k % pads.len()];
                    events.push(Timed::new(
                        at,
                        NodeEvent::NoteOn {
                            note,
                            velocity: 100,
                        },
                    ));
                }
            } else if b == 0 {
                for &note in pads {
                    events.push(Timed::new(
                        0,
                        NodeEvent::NoteOn {
                            note,
                            velocity: 100,
                        },
                    ));
                }
            }
            prog.play(f0, l, r);
            let ctx = Ctx {
                bpm: 120.0,
                beats: f0 as f64 * 2.0 / f64::from(rate),
                playing: true,
                ..Ctx::new(rate, f0)
            };
            p.process(&ctx, &events, l, r);
        }),
        mem_bytes: 0,
    }
}

/// A synth playing `voices`-note chords that change every half second, so
/// note starts, releases and steals are all in the numbers.
fn synth(name: String, rate: u32, values: &[(u16, f32)], voices: usize) -> Case {
    chords(
        name,
        rate,
        Box::new(Poly::new(rate)),
        values,
        Some(POLYPHONY),
        voices,
    )
}

/// Any instrument node playing [`synth`]'s chords; `polyphony` is its voice
/// count parameter (None for an organ, which plays every key).
fn chords(
    name: String,
    rate: u32,
    s: Box<dyn Node>,
    values: &[(u16, f32)],
    polyphony: Option<u16>,
    voices: usize,
) -> Case {
    let mut c = Chords::new(rate, s, values, polyphony, voices);
    Case {
        name,
        module: "synth",
        what: "chords changing every 0.5 s",
        block: Box::new(move |b, l, r| c.block(b, l, r)),
        mem_bytes: 0,
    }
}

/// An instrument and the chords it plays (see [`chords`]): a block at a
/// time, on whichever thread holds it.
struct Chords {
    rate: u32,
    s: Box<dyn Node>,
    voices: usize,
    events: Vec<Timed>,
    held: Vec<u8>,
    seed: Lcg,
}

impl Chords {
    fn new(
        rate: u32,
        mut s: Box<dyn Node>,
        values: &[(u16, f32)],
        polyphony: Option<u16>,
        voices: usize,
    ) -> Self {
        s.prepare(rate, MAX_BLOCK);
        for &(id, v) in values {
            s.set_param(id, v);
        }
        if let Some(id) = polyphony {
            s.set_param(id, voices as f32);
        }
        Self {
            rate,
            s,
            voices,
            events: Vec::with_capacity(2 * voices),
            held: Vec::with_capacity(voices),
            seed: Lcg(3),
        }
    }

    fn block(&mut self, b: u64, l: &mut [f32], r: &mut [f32]) {
        let len = l.len() as u64;
        let f0 = b * len;
        let half = u64::from(self.rate / 2);
        self.events.clear();
        // A chord change falls in this block.
        if f0 % half + len > half || f0 == 0 {
            let at = if f0 == 0 {
                0
            } else {
                (half - f0 % half) as u32 % len as u32
            };
            for &note in self.held.iter() {
                self.events
                    .push(Timed::new(at, NodeEvent::NoteOff { note }));
            }
            self.held.clear();
            let root = 36 + (self.seed.next() >> 28) as u8;
            for k in 0..self.voices {
                let note = root + [0u8, 7, 12, 16, 19, 24, 28, 31][k % 8] + 12 * (k / 8) as u8;
                self.held.push(note);
                self.events.push(Timed::new(
                    at,
                    NodeEvent::NoteOn {
                        note,
                        velocity: 64 + (self.seed.next() >> 26) as u8,
                    },
                ));
            }
        }
        l.fill(0.0);
        r.fill(0.0);
        self.s.process(&Ctx::new(self.rate, f0), &self.events, l, r);
    }
}

/// A mono synth playing a sixteenth-note line at 125 BPM, every fourth
/// note accented (velocity 127) and every third sliding into the next, so
/// note starts, accents, slides and gate ends are all in the numbers.
fn line(name: String, rate: u32, s: Box<dyn Node>, values: &[(u16, f32)]) -> Case {
    let mut c = Line::new(rate, s, values);
    Case {
        name,
        module: "synth",
        what: "a sixteenth-note line with accents and slides",
        block: Box::new(move |b, l, r| c.block(b, l, r)),
        mem_bytes: 0,
    }
}

/// A mono instrument and the line it plays (see [`line`]).
struct Line {
    rate: u32,
    s: Box<dyn Node>,
    events: Vec<Timed>,
    held: Option<u8>,
    slide: bool,
    seed: Lcg,
}

impl Line {
    fn new(rate: u32, mut s: Box<dyn Node>, values: &[(u16, f32)]) -> Self {
        s.prepare(rate, MAX_BLOCK);
        for &(id, v) in values {
            s.set_param(id, v);
        }
        Self {
            rate,
            s,
            events: Vec::with_capacity(4),
            held: None,
            slide: false,
            seed: Lcg(5),
        }
    }

    fn block(&mut self, b: u64, l: &mut [f32], r: &mut [f32]) {
        let len = l.len() as u64;
        let f0 = b * len;
        // A sixteenth at 125 BPM, and a gate 3.5 of its 6 ticks long.
        let step = u64::from(self.rate) * 3 / 25;
        let gate = step * 7 / 12;
        self.events.clear();
        let k = f0.div_ceil(step);
        let start = k * step;
        let j = if f0 >= gate {
            (f0 - gate).div_ceil(step)
        } else {
            0
        };
        let end = j * step + gate;
        if end < f0 + len && end < start && !self.slide {
            if let Some(note) = self.held.take() {
                self.events
                    .push(Timed::new((end - f0) as u32, NodeEvent::NoteOff { note }));
            }
        }
        if start < f0 + len {
            let at = (start - f0) as u32;
            let note = 36 + (self.seed.next() >> 28) as u8;
            let velocity = if k % 4 == 0 { 127 } else { 90 };
            self.events
                .push(Timed::new(at, NodeEvent::NoteOn { note, velocity }));
            // The note before slides into this one: let go after.
            if let Some(prev) = self.held.take() {
                if prev != note {
                    self.events
                        .push(Timed::new(at, NodeEvent::NoteOff { note: prev }));
                }
            }
            self.held = Some(note);
            self.slide = k % 3 == 2;
        }
        if end < f0 + len && end >= start && !self.slide {
            if let Some(note) = self.held.take() {
                self.events
                    .push(Timed::new((end - f0) as u32, NodeEvent::NoteOff { note }));
            }
        }
        l.fill(0.0);
        r.fill(0.0);
        self.s.process(&Ctx::new(self.rate, f0), &self.events, l, r);
    }
}

/// The drum synth with a kit, hit on a random pad every `every` blocks
/// with random velocities, so long hits overlap and choke groups work.
fn drums(name: &str, rate: u32, kit: &str, every: u64) -> Case {
    let mut d = Drums::new(rate);
    d.prepare(rate, MAX_BLOCK);
    let k = Drums::presets()
        .iter()
        .find(|k| k.name == kit)
        .unwrap_or_else(|| panic!("kit {kit}"));
    d.load_preset(k);
    let mut seed = Lcg(17);
    let mut events: Vec<Timed> = Vec::with_capacity(1);
    Case {
        name: name.to_string(),
        module: "synth",
        what: "random pads and velocities",
        block: Box::new(move |b, l, r| {
            events.clear();
            if b % every == 0 {
                let pad = (seed.next() >> 28) as u8 % 16;
                events.push(Timed::new(
                    0,
                    NodeEvent::NoteOn {
                        note: 36 + pad,
                        velocity: 40 + (seed.next() >> 25) as u8 % 88,
                    },
                ));
            }
            l.fill(0.0);
            r.fill(0.0);
            d.process(&Ctx::new(rate, b * l.len() as u64), &events, l, r);
        }),
        mem_bytes: 0,
    }
}

/// The pluck synth with a preset, `voices`-note chords changing every half
/// second (re-plucks, releases and steals all in the numbers).
fn pluck(preset: &str, rate: u32, voices: usize) -> Case {
    let mut s = Pluck::new(rate);
    s.prepare(rate, MAX_BLOCK);
    let p = Pluck::presets()
        .iter()
        .find(|p| p.name == preset)
        .unwrap_or_else(|| panic!("pluck preset {preset}"));
    s.load_preset(p);
    s.set_param(mpfree_synth::pluck_params::POLYPHONY, voices as f32);
    let mut events: Vec<Timed> = Vec::with_capacity(2 * voices);
    let mut held: Vec<u8> = Vec::with_capacity(voices);
    let mut seed = Lcg(5);
    Case {
        name: format!("Pluck {preset}, {voices} voices"),
        module: "synth",
        what: "chords changing every 0.5 s",
        block: Box::new(move |b, l, r| {
            let len = l.len() as u64;
            let f0 = b * len;
            let half = u64::from(rate / 2);
            events.clear();
            if f0 % half + len > half || f0 == 0 {
                let at = if f0 == 0 {
                    0
                } else {
                    (half - f0 % half) as u32 % len as u32
                };
                for &note in held.iter() {
                    events.push(Timed::new(at, NodeEvent::NoteOff { note }));
                }
                held.clear();
                let root = 40 + (seed.next() >> 28) as u8;
                for k in 0..voices {
                    let note = root + [0u8, 7, 12, 16, 19, 24, 28, 31][k % 8] + 12 * (k / 8) as u8;
                    held.push(note);
                    events.push(Timed::new(
                        at,
                        NodeEvent::NoteOn {
                            note,
                            velocity: 64 + (seed.next() >> 26) as u8,
                        },
                    ));
                }
            }
            l.fill(0.0);
            r.fill(0.0);
            s.process(&Ctx::new(rate, f0), &events, l, r);
        }),
        mem_bytes: 0,
    }
}

/// Which of the sampler case's pads are set. The hits are the same in
/// every case (a hit on an unset pad does nothing), so the two parts add
/// up to the whole.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SamplerPads {
    All,
    /// Pads 1 to 16: sample voices (Hermite read, envelope, filter).
    Samples,
    /// Pads 17 to 20: granular voices.
    Grains,
}

/// The sampler as the pad player uses it: 16 one-shot, gated and looping
/// pads with filters and two layers, four granular pads, 24 voices, a hit
/// every 4 blocks (about 11 a second), and optionally the master grain
/// effect.
fn sampler(name: &str, rate: u32, master: bool, which: SamplerPads) -> Case {
    let mut e = sampler_engine(rate, master, which, 24, |_| true);
    let mut hits = SamplerHits::new();
    Case {
        name: name.to_string(),
        module: "sampler",
        what: "a hit every 4 blocks over 20 pads, 24 voices",
        block: Box::new(move |b, l, r| {
            for ev in hits.at(b).into_iter().flatten() {
                e.apply(ev);
            }
            l.fill(0.0);
            r.fill(0.0);
            e.render(l, r);
        }),
        mem_bytes: 0,
    }
}

/// The [`sampler`] case's engine with `voices` voices and only the pads
/// `keep` keeps (pads 0 to 15 samples, 16 to 19 grains), so two engines can
/// split the pads between them.
fn sampler_engine(
    rate: u32,
    master: bool,
    which: SamplerPads,
    voices: usize,
    keep: impl Fn(usize) -> bool,
) -> Engine {
    let s = Source::new(
        "noise",
        rate,
        2,
        (0..rate as usize * 2)
            .map(|i| ((i * 7919) % 2000) as f32 / 1000.0 - 1.0)
            .collect(),
    )
    .expect("source");
    let mut p = Project::new();
    p.add_source(s.clone());
    for i in (0..16usize).filter(|&i| which != SamplerPads::Grains && keep(i)) {
        let mut r = Region::whole(&s);
        r.mode = [
            PlayMode::OneShot,
            PlayMode::Gate,
            PlayMode::Loop,
            PlayMode::OneShot,
        ][i % 4];
        r.sustain_loop = Some(Loop {
            start: 5000,
            end: 9000,
            crossfade: 200,
            mode: LoopMode::Forward,
        });
        r.tune_cents = (i as i32 - 8) * 50;
        let mut pad = Pad::new("p", Layer::new(r.clone()));
        pad.layers.push(Layer::new(r));
        pad.polyphony = 2;
        pad.filter = Filter {
            kind: FilterKind::LowPass,
            poles: 2,
            cutoff_hz: 2000.0,
            resonance: 0.3,
            env_octaves: 1.0,
        };
        p.set_pad(i, Some(pad), None).expect("pad");
    }
    for i in (16..20usize).filter(|&i| which != SamplerPads::Samples && keep(i)) {
        let mut r = Region::whole(&s);
        r.mode = PlayMode::Granular;
        r.grain = GrainParams {
            spray: 0.6,
            size_ms: 300.0,
            density: 200.0,
            pitch_spray: 5.0,
            spread: 1.0,
            scan: 1.0,
            ..GrainParams::default()
        };
        let mut pad = Pad::new("g", Layer::new(r));
        pad.polyphony = 2;
        p.set_pad(i, Some(pad), None).expect("pad");
    }
    let mut e = Engine::new(rate, voices);
    if master {
        e.set_master_fx(Engine::make_master_fx(rate));
        p.set_master(
            MasterFx {
                mix: 0.5,
                grain: GrainParams {
                    density: 200.0,
                    size_ms: 150.0,
                    spray: 0.3,
                    ..MasterFx::default().grain
                },
            },
            None,
        )
        .expect("master");
    }
    e.set_view(EngineView::build(&p, rate));
    e
}

/// The [`sampler`] case's hits: every 4 blocks, a note off on one pad and a
/// note on another, with a random velocity. A hit on a pad an engine
/// doesn't have does nothing, so engines that split the pads can all take
/// every hit.
struct SamplerHits(Lcg);

impl SamplerHits {
    fn new() -> Self {
        Self(Lcg(11))
    }

    fn at(&mut self, b: u64) -> Option<[Event; 2]> {
        if !b.is_multiple_of(4) {
            return None;
        }
        let pad = (self.0.next() >> 27) as u8 % 20;
        Some([
            Event::NoteOff {
                pad: (pad + 7) % 20,
            },
            Event::NoteOn {
                pad,
                velocity: 40 + (self.0.next() >> 25) as u8 % 88,
            },
        ])
    }
}

/// The inserts a [`mixer`] case carries: none, or a song's worth of
/// effects with the Cloud reverb at its Full quality (the worst case),
/// at Light, or at Eco (the reverb's default).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Inserts {
    None,
    Song,
    SongLight,
    SongEco,
    /// The song's inserts with the six tracks that have inserts frozen,
    /// the transport rolling, and the drum bus printing all the while.
    SongFrozen,
}

/// The mixer at its default size (2 inputs, 32 pad channels, 16 tracks, 8
/// submixes, 4 returns), fed `tracks` tracks and `pads` pad channels of
/// program material, with a typical song's routing: pads into the drum
/// track, drums and bass into submixes, sends to two returns, and the
/// inserts `with` names.
fn mixer(name: &str, rate: u32, tracks: usize, pads: usize, with: Inserts) -> Case {
    mixer_with(name, rate, tracks, pads, with, Second::None)
}

/// What a [`mixer_with`] case runs on a second core.
enum Second {
    None,
    /// Pad and track inserts partly on a lane (`Mixer::set_lane`).
    Tracks(LaneConfig),
    /// The returns a block late on a lane (`Mixer::set_pipelined_returns`).
    Returns(LaneConfig),
}

/// A mixer case, with some of its work on a second core as `second` says.
fn mixer_with(
    name: &str,
    rate: u32,
    tracks: usize,
    pads: usize,
    with: Inserts,
    second: Second,
) -> Case {
    let inserts = with != Inserts::None;
    use ids::mixer::{self as mx, output};
    let mut m = Mixer::new(MixerConfig::default(), rate, MAX_BLOCK.min(1024));
    for p in 0..pads {
        m.set_now(StripId::Pad(p), mx::OUTPUT, output::TRACK);
        m.set_now(StripId::Pad(p), mx::send(1), -12.0);
    }
    for t in 0..tracks {
        let s = StripId::Track(t);
        m.set_now(s, mx::OUTPUT, output::bus(t % 3));
        m.set_now(s, mx::PAN, (t as f32 * 37.0) % 200.0 - 100.0);
        m.set_now(s, mx::send(0), -18.0);
        m.set_now(s, mx::send_pre(1), 1.0);
        m.set_now(s, mx::send(1), -24.0);
    }
    m.set_now(StripId::Bus(0), mx::OUTPUT, output::bus(3));
    if inserts {
        for t in 0..tracks.min(6) {
            let c = m.chain_mut(StripId::Track(t)).unwrap();
            c.push(Box::new(Eq::new()));
            c.push(Box::new(Compressor::new()));
        }
        m.chain_mut(StripId::Bus(1))
            .unwrap()
            .push(Box::new(Drive::new()));
        m.chain_mut(StripId::Bus(3))
            .unwrap()
            .push(Box::new(Compressor::new()));
        m.chain_mut(StripId::Return(0))
            .unwrap()
            .push(Box::new(Cloud::new()));
        let quality = match with {
            Inserts::SongLight => 1.0,
            Inserts::SongEco => 2.0,
            _ => 0.0,
        };
        m.chain_mut(StripId::Return(0))
            .unwrap()
            .set_param(0, ids::cloud::QUALITY, quality);
        m.chain_mut(StripId::Return(1))
            .unwrap()
            .push(Box::new(Delay::new()));
        m.chain_mut(StripId::Master)
            .unwrap()
            .push(Box::new(Eq::new()));
        m.chain_mut(StripId::Track(1))
            .unwrap()
            .set_param(1, ids::compressor::SIDECHAIN, 1.0);
        m.set_key(StripId::Track(1), 1, Some(StripId::Track(0)));
    }
    match second {
        Second::None => {}
        Second::Tracks(cfg) => {
            m.set_lane(Some(cfg), Duration::from_nanos(10 * MPC.ns()));
        }
        Second::Returns(cfg) => {
            // A send reverb as it goes late: all wet, with a pre-delay to
            // take the block back from (otherwise it stays on time).
            let c = m.chain_mut(StripId::Return(0)).unwrap();
            c.set_param(0, ids::cloud::MIX, 100.0);
            c.set_param(0, ids::cloud::PRE_DELAY, 20.0);
            m.set_pipelined_returns(Some(cfg), Duration::from_nanos(10 * MPC.ns()));
        }
    }
    let frozen = with == Inserts::SongFrozen;
    // Two beats of each frozen track, at 120 BPM.
    let beat = rate as usize / 2;
    if frozen {
        for t in 0..tracks.min(6) {
            let (mut l, mut r) = (vec![0.0f32; 2 * beat], vec![0.0f32; 2 * beat]);
            input(
                rate,
                t as u64 * 977,
                &mut Lcg(31 + t as u32),
                &mut l,
                &mut r,
            );
            let take = Frozen {
                left: l,
                right: r,
                origin: 0.0,
                beats: 2.0,
                bpm: 120.0,
                rate,
            };
            assert!(m.freeze(StripId::Track(t), take).is_ok());
        }
        assert!(m.set_capture(0, Some((StripId::Bus(0), CapturePoint::BeforeFader))));
    }
    // Eight bars of the drum bus, taken again as each one ends.
    let mut print = frozen.then(|| Print::new(rate, 18.0));
    let job = |start: f64| PrintJob {
        start,
        beats: 32.0,
        steady: false,
        tail: Tail::Cut,
    };
    if let Some(p) = print.as_mut() {
        p.arm(job(0.0)).expect("arm");
    }
    // The sources' audio, rendered once so the timing is the mixer's alone
    // (the sampler and synth cases time making it); each source reads its
    // own offset.
    const TABLE: usize = 8192;
    let max = MAX_BLOCK.min(1024);
    let (mut tl, mut tr) = (vec![0.0f32; TABLE + max], vec![0.0f32; TABLE + max]);
    input(rate, 0, &mut Lcg(11), &mut tl, &mut tr);
    let at = |f: u64| (f % TABLE as u64) as usize;
    Case {
        name: name.to_string(),
        module: if inserts { "fx" } else { "mix" },
        what: if frozen {
            "the song's inserts with the 6 tracks that have them frozen, rolling, the drum bus printing"
        } else if inserts {
            "the routing below plus EQ and compressor on 6 tracks, drive and glue on submixes, reverb, delay, master EQ"
        } else {
            "a song's routing, no inserts (Master safety included)"
        },
        block: Box::new(move |b, out_l, out_r| {
            let n = out_l.len();
            let f0 = b * n as u64;
            for t in 0..tracks {
                let o = at(f0 + t as u64 * 977);
                let (il, ir) = m.track_input(t).unwrap();
                il[..n].copy_from_slice(&tl[o..o + n]);
                ir[..n].copy_from_slice(&tr[o..o + n]);
            }
            for p in 0..pads {
                let o = at(f0 + p as u64 * 1543);
                let (il, ir) = m.pad_input(p).unwrap();
                il[..n].copy_from_slice(&tl[o..o + n]);
                ir[..n].copy_from_slice(&tr[o..o + n]);
            }
            if !frozen {
                m.process(&Ctx::new(rate, f0), out_l, out_r);
                return;
            }
            let ctx = Ctx {
                bpm: 120.0,
                beats: f0 as f64 * 2.0 / rate as f64,
                playing: true,
                ..Ctx::new(rate, f0)
            };
            m.process(&ctx, out_l, out_r);
            if let Some(p) = print.as_mut() {
                p.process(&ctx, m.capture(0), m.capture_latency(0).unwrap_or(0));
                if p.state() == PrintState::Done {
                    let _ = p.arm(job(ctx.beats.ceil()));
                }
            }
        }),
        mem_bytes: 0,
    }
}

/// Every case, at `rate`.
pub fn all(rate: u32) -> Vec<Case> {
    // Made first, so no case's memory includes it.
    program(rate);
    let mut v = Vec::new();
    for preset in PRESETS {
        v.push(measured(|| {
            synth(
                format!("Poly {}, 8 voices", preset.name),
                rate,
                preset.values,
                8,
            )
        }));
    }
    v.push(measured(|| {
        synth(
            "Poly Init, 16 voices, unison 4".to_string(),
            rate,
            &[(UNISON, 4.0)],
            16,
        )
    }));
    v.push(measured(|| {
        drums("Drums Init Kit, a hit every 4 blocks", rate, "Init Kit", 4)
    }));
    v.push(measured(|| {
        drums("Drums Boom Kit, a hit every block", rate, "Boom Kit", 1)
    }));
    for (name, voices) in [
        ("Steel String", 8),
        ("Harp", 16),
        ("Marimba", 8),
        ("Bell", 16),
    ] {
        v.push(measured(|| pluck(name, rate, voices)));
    }
    // Keys: a tine and a reed preset, and the tremolo and drive on top.
    for (name, voices) in [("Init", 8), ("Init", 16), ("Reed", 16), ("Dusty Keys", 16)] {
        v.push(measured(|| {
            let p = Keys::presets()
                .iter()
                .find(|p| p.name == name)
                .unwrap_or_else(|| panic!("keys preset {name}"));
            chords(
                format!("Keys {name}, {voices} voices"),
                rate,
                Box::new(Keys::new(rate)),
                p.values,
                Some(mpfree_synth::keys_params::POLYPHONY),
                voices,
            )
        }));
    }
    // Drawbar: the default three drawbars, all nine with leakage, chorus
    // and the fast rotary, and the rotary off.
    for (name, keys) in [
        ("Init", 8),
        ("Full Organ", 8),
        ("Gospel", 16),
        ("Church", 8),
    ] {
        v.push(measured(|| {
            let p = Drawbar::presets()
                .iter()
                .find(|p| p.name == name)
                .unwrap_or_else(|| panic!("drawbar preset {name}"));
            chords(
                format!("Drawbar {name}, {keys} keys"),
                rate,
                Box::new(Drawbar::new(rate)),
                p.values,
                None,
                keys,
            )
        }));
    }
    // Squelch: the classic line, the square, and the drive on top.
    for name in ["Acid Line", "Square Acid", "Distorted"] {
        v.push(measured(|| {
            let p = Squelch::presets()
                .iter()
                .find(|p| p.name == name)
                .unwrap_or_else(|| panic!("squelch preset {name}"));
            line(
                format!("Squelch {name}, a line"),
                rate,
                Box::new(Squelch::new(rate)),
                p.values,
            )
        }));
    }
    // Wavetable: the first case also holds the factory tables (built once
    // per process, shared by every instance).
    for preset in WT_PRESETS {
        v.push(measured(|| {
            chords(
                format!("Wavetable {}, 8 voices", preset.name),
                rate,
                Box::new(Wavetable::new(rate)),
                preset.values,
                Some(wt::POLYPHONY),
                8,
            )
        }));
    }
    v.push(measured(|| {
        chords(
            "Wavetable Init, 8 voices, both osc, unison 4".to_string(),
            rate,
            Box::new(Wavetable::new(rate)),
            &[(wt::osc_id(1, wt::OSC_LEVEL), 60.0), (wt::UNISON, 4.0)],
            Some(wt::POLYPHONY),
            8,
        )
    }));
    v.push(measured(|| {
        chords(
            "Wavetable Init, 16 voices, both osc, unison 8".to_string(),
            rate,
            Box::new(Wavetable::new(rate)),
            &[(wt::osc_id(1, wt::OSC_LEVEL), 60.0), (wt::UNISON, 8.0)],
            Some(wt::POLYPHONY),
            16,
        )
    }));
    v.push(measured(|| {
        sampler("Sampler, 20 pads", rate, false, SamplerPads::All)
    }));
    v.push(measured(|| {
        sampler(
            "Sampler, 20 pads + master grains",
            rate,
            true,
            SamplerPads::All,
        )
    }));
    v.push(measured(|| {
        sampler(
            "Sampler, the 16 sample pads alone",
            rate,
            false,
            SamplerPads::Samples,
        )
    }));
    v.push(measured(|| {
        sampler(
            "Sampler, the 4 granular pads alone",
            rate,
            false,
            SamplerPads::Grains,
        )
    }));
    v.push(measured(|| {
        effect(
            "Multi filter (LP 12)",
            rate,
            Box::new(MultiFilter::new()),
            &[(ids::filter::CUTOFF, 800.0)],
        )
    }));
    v.push(measured(|| {
        effect(
            "Multi filter (LP 24, drive)",
            rate,
            Box::new(MultiFilter::new()),
            &[
                (ids::filter::CUTOFF, 800.0),
                (ids::filter::SLOPE, 1.0),
                (ids::filter::DRIVE, 12.0),
            ],
        )
    }));
    v.push(measured(|| {
        effect(
            "DJ filter",
            rate,
            Box::new(DjFilter::new()),
            &[(ids::dj_filter::POSITION, -40.0)],
        )
    }));
    v.push(measured(|| {
        effect("Drive (Eco 2x)", rate, Box::new(Drive::new()), &[])
    }));
    v.push(measured(|| {
        effect(
            "Drive (HQ 4x)",
            rate,
            Box::new(Drive::new()),
            &[(ids::drive::QUALITY, 1.0)],
        )
    }));
    v.push(measured(|| {
        effect("Crush", rate, Box::new(Crush::new()), &[])
    }));
    for (name, mode) in [
        ("Character (Console)", ids::character::CONSOLE),
        ("Character (Tape)", ids::character::TAPE),
        ("Character (Iron)", ids::character::IRON),
        ("Character (Soften)", ids::character::SOFTEN),
    ] {
        v.push(measured(|| {
            effect(
                name,
                rate,
                Box::new(Character::new()),
                &[(ids::character::MODE, mode), (ids::character::AMOUNT, 70.0)],
            )
        }));
    }
    v.push(measured(|| {
        effect("Delay", rate, Box::new(Delay::new()), &[])
    }));
    v.push(measured(|| {
        effect("Comb", rate, Box::new(Comb::new()), &[])
    }));
    v.push(measured(|| {
        effect("Compressor", rate, Box::new(Compressor::new()), &[])
    }));
    v.push(measured(|| {
        effect(
            "EQ (4 bands and both cuts)",
            rate,
            Box::new(Eq::new()),
            &[
                (ids::eq::LOW_SLOPE, 2.0),
                (ids::eq::HIGH_SLOPE, 2.0),
                (ids::eq::gain(0), 3.0),
                (ids::eq::gain(1), -4.0),
                (ids::eq::gain(2), 5.0),
                (ids::eq::gain(3), 2.0),
            ],
        )
    }));
    v.push(measured(|| {
        effect("Chorus (2 voices)", rate, Box::new(Chorus::new()), &[])
    }));
    v.push(measured(|| {
        effect(
            "Chorus (4 voices)",
            rate,
            Box::new(Chorus::new()),
            &[(ids::chorus::VOICES, 4.0)],
        )
    }));
    v.push(measured(|| {
        effect("Flanger", rate, Box::new(Flanger::new()), &[])
    }));
    v.push(measured(|| {
        effect("Phaser (4 stages)", rate, Box::new(Phaser::new()), &[])
    }));
    v.push(measured(|| {
        effect(
            "Phaser (12 stages)",
            rate,
            Box::new(Phaser::new()),
            &[(ids::phaser::STAGES, 4.0)],
        )
    }));
    v.push(measured(|| {
        effect(
            "Lo-fi",
            rate,
            Box::new(LoFi::new()),
            &[(ids::lofi::AMOUNT, 100.0)],
        )
    }));
    v.push(measured(|| {
        effect("Master safety", rate, Box::new(MasterSafety::new()), &[])
    }));
    // Perform FX: idle (no pad held), the default bank one pad at a time,
    // the heaviest stack (pitch drop with a gate, both sweeps, crush, the
    // riser and an echo throw) and the wash throw on its own.
    v.push(measured(|| perform("Perform FX, idle", rate, &[], false)));
    v.push(measured(|| {
        perform(
            "Perform FX, the 16 pads in turn",
            rate,
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
            true,
        )
    }));
    v.push(measured(|| {
        perform(
            "Perform FX, 7 pads held (pitch drop, gate, sweeps, crush, riser, echo)",
            rate,
            &[6, 7, 8, 9, 10, 11, 12],
            false,
        )
    }));
    // The wash throw (pad 16) held: its reverb fed and ringing all along.
    v.push(measured(|| {
        perform("Perform FX, a wash throw held", rate, &[15], false)
    }));
    // The reverb at Full (its cost before Eco, the worst case) and at
    // Eco, its default.
    for (k, c) in ["Plate", "Room", "Hall", "Cloud", "Echoes"]
        .iter()
        .enumerate()
    {
        v.push(measured(|| {
            effect(
                &format!("Cloud reverb ({c})"),
                rate,
                Box::new(Cloud::new()),
                &[
                    (ids::cloud::CHARACTER, k as f32),
                    (ids::cloud::MIX, 100.0),
                    (ids::cloud::QUALITY, 0.0),
                ],
            )
        }));
    }
    for (k, c) in ["Plate", "Room", "Hall", "Cloud", "Echoes"]
        .iter()
        .enumerate()
    {
        v.push(measured(|| {
            effect(
                &format!("Cloud reverb ({c}, Eco)"),
                rate,
                Box::new(Cloud::new()),
                &[
                    (ids::cloud::CHARACTER, k as f32),
                    (ids::cloud::MIX, 100.0),
                    (ids::cloud::QUALITY, 2.0),
                ],
            )
        }));
    }
    v.push(measured(|| {
        effect(
            "Grains FX (dense)",
            rate,
            Box::new(GrainNode::new(5)),
            &[
                (mpfree_grain::node::param::MIX, 100.0),
                (mpfree_grain::node::param::DENSITY, 200.0),
                (mpfree_grain::node::param::SPRAY, 50.0),
            ],
        )
    }));
    v.push(measured(|| {
        mixer(
            "Mixer, idle (63 strips, nothing playing)",
            rate,
            0,
            0,
            Inserts::None,
        )
    }));
    v.push(measured(|| {
        mixer(
            "Mixer, 12 tracks and 8 pads, routing only",
            rate,
            12,
            8,
            Inserts::None,
        )
    }));
    v.push(measured(|| {
        mixer(
            "Mixer, 16 tracks and 32 pads, routing only",
            rate,
            16,
            32,
            Inserts::None,
        )
    }));
    v.push(measured(|| {
        mixer(
            "Mixer, 12 tracks and 8 pads, with a song's inserts",
            rate,
            12,
            8,
            Inserts::Song,
        )
    }));
    v.push(measured(|| {
        mixer(
            "Mixer, 12 tracks and 8 pads, with a song's inserts, Light reverb",
            rate,
            12,
            8,
            Inserts::SongLight,
        )
    }));
    v.push(measured(|| {
        mixer(
            "Mixer, 12 tracks and 8 pads, with a song's inserts, Eco reverb",
            rate,
            12,
            8,
            Inserts::SongEco,
        )
    }));
    v.push(measured(|| {
        mixer(
            "Mixer, a song's inserts, 6 tracks frozen, printing a bus",
            rate,
            12,
            8,
            Inserts::SongFrozen,
        )
    }));
    v.push(measured(|| chain(rate)));
    v.push(measured(|| modulation(rate)));
    v.push(measured(|| moves(rate)));
    v.push(measured(|| track_host(rate, 4, 4)));
    v.push(measured(|| track_host_alone(rate, 16)));
    v.push(measured(|| time(rate, 8, false)));
    v.push(measured(|| time(rate, 8, true)));
    v.push(measured(|| live_arps(rate, 4)));
    v.push(measured(|| live_arps(rate, 16)));
    v.push(measured(|| looper(rate, 60)));
    v.push(measured(|| looper_loops(rate, 60, false)));
    v.push(measured(|| looper_loops(rate, 60, true)));
    v.push(measured(|| audio_tracks(rate, 0, false)));
    v.push(measured(|| audio_tracks(rate, 2, false)));
    v.push(measured(|| audio_tracks(rate, 0, true)));
    v.push(measured(|| song(rate)));
    v
}

/// One engine measured at several voice counts, to find what a voice costs.
pub struct Scaling {
    pub name: String,
    /// The budget it counts against.
    pub module: &'static str,
    /// (voices, the case playing that many).
    pub points: Vec<(usize, Case)>,
}

/// Every Poly preset at 1 to 16 voices (chords changing every half second,
/// as in the synth cases), the sampler at 1 to 32 voices held, and the
/// sampler's grains at 16 to 256 alive.
pub fn scaling(rate: u32) -> Vec<Scaling> {
    let mut v: Vec<Scaling> = PRESETS
        .iter()
        .map(|p| {
            // Mono and legato presets play one voice whatever the polyphony.
            let mono = p.values.iter().any(|&(id, x)| id == VOICE_MODE && x != 0.0);
            let counts: &[usize] = if mono { &[1] } else { &[1, 2, 4, 8, 16] };
            Scaling {
                name: format!("Poly {}{}", p.name, if mono { " (mono)" } else { "" }),
                module: "synth",
                points: counts
                    .iter()
                    .map(|&n| (n, synth(format!("Poly {} x{n}", p.name), rate, p.values, n)))
                    .collect(),
            }
        })
        .collect();
    v.push(Scaling {
        name: "Sampler, looping pads held".to_string(),
        module: "sampler",
        points: [1usize, 4, 8, 16, 32]
            .iter()
            .map(|&n| (n, held_pads(rate, n)))
            .collect(),
    });
    v.push(Scaling {
        name: "Sampler grains (per grain)".to_string(),
        module: "sampler",
        points: [16usize, 32, 64, 128, 256]
            .iter()
            .map(|&n| (n, held_grains(rate, n)))
            .collect(),
    });
    v
}

/// Four granular pads held down, set so that about `grains` grains are alive
/// in every block in all: 100 ms grains at `grains / 4 * 10` a second per
/// pad. 256 is the engine's whole grain budget ([`mpfree_sampler::engine::GRAIN_BUDGET`]) and 64 a
/// pad is its per-voice cap, so the top point is the worst a grain burst can
/// cost. The other settings are the round 7 granular pads' (pitch spray,
/// spread, scan).
fn held_grains(rate: u32, grains: usize) -> Case {
    let mut e = grain_engine(rate, grains);
    Case {
        name: format!("Sampler grains x{grains}"),
        module: "sampler",
        what: "4 granular pads held, grains alive in all",
        block: Box::new(move |b, l, r| {
            if b == 0 {
                for pad in 0..4 {
                    e.apply(Event::NoteOn { pad, velocity: 100 });
                }
            }
            l.fill(0.0);
            r.fill(0.0);
            e.render(l, r);
        }),
        mem_bytes: 0,
    }
}

/// The engine behind [`held_grains`].
fn grain_engine(rate: u32, grains: usize) -> Engine {
    let s = Source::new(
        "noise",
        rate,
        2,
        (0..rate as usize * 2)
            .map(|i| ((i * 7919) % 2000) as f32 / 1000.0 - 1.0)
            .collect(),
    )
    .expect("source");
    let mut p = Project::new();
    p.add_source(s.clone());
    let density = grains as f32 / 4.0 * 10.0;
    for i in 0..4usize {
        let mut r = Region::whole(&s);
        r.mode = PlayMode::Granular;
        r.grain = GrainParams {
            spray: 0.6,
            size_ms: 100.0,
            density,
            pitch_spray: 5.0,
            spread: 1.0,
            scan: 1.0,
            ..GrainParams::default()
        };
        let mut pad = Pad::new("g", Layer::new(r));
        pad.polyphony = 1;
        p.set_pad(i, Some(pad), None).expect("pad");
    }
    let mut e = Engine::new(rate, 4);
    e.set_view(EngineView::build(&p, rate));
    e
}

/// The sampler with `n` looping, filtered, stereo pads held down, so exactly
/// `n` voices sound in every block.
fn held_pads(rate: u32, n: usize) -> Case {
    let s = Source::new(
        "noise",
        rate,
        2,
        (0..rate as usize * 2)
            .map(|i| ((i * 7919) % 2000) as f32 / 1000.0 - 1.0)
            .collect(),
    )
    .expect("source");
    let mut p = Project::new();
    p.add_source(s.clone());
    for i in 0..n {
        let mut r = Region::whole(&s);
        r.mode = PlayMode::Loop;
        r.sustain_loop = Some(Loop {
            start: 5000,
            end: 30000,
            crossfade: 200,
            mode: LoopMode::Forward,
        });
        r.tune_cents = (i as i32 % 24 - 12) * 50;
        let mut pad = Pad::new("p", Layer::new(r));
        pad.polyphony = 1;
        pad.filter = Filter {
            kind: FilterKind::LowPass,
            poles: 2,
            cutoff_hz: 2000.0,
            resonance: 0.3,
            env_octaves: 1.0,
        };
        p.set_pad(i, Some(pad), None).expect("pad");
    }
    let mut e = Engine::new(rate, n.max(1));
    e.set_view(EngineView::build(&p, rate));
    Case {
        name: format!("Sampler x{n} held"),
        module: "sampler",
        what: "looping pads held down",
        block: Box::new(move |b, l, r| {
            if b == 0 {
                for pad in 0..n {
                    e.apply(Event::NoteOn {
                        pad: pad as u8,
                        velocity: 100,
                    });
                }
            }
            l.fill(0.0);
            r.fill(0.0);
            e.render(l, r);
        }),
        mem_bytes: 0,
    }
}

/// A straight line through (voices, ns) by least squares: (fixed ns, ns
/// per voice).
pub fn fit(points: &[(usize, f64)]) -> (f64, f64) {
    if let [(1, ns)] = points {
        return (*ns, 0.0);
    }
    let n = points.len() as f64;
    let mx = points.iter().map(|p| p.0 as f64).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let sxy: f64 = points.iter().map(|p| (p.0 as f64 - mx) * (p.1 - my)).sum();
    let sxx: f64 = points.iter().map(|p| (p.0 as f64 - mx).powi(2)).sum();
    let b = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    (my - b * mx, b)
}

/// A busy song on one audio thread, against the whole thread's budget: the
/// sampler with the master grains, two synths (an 8-voice Supersaw and the
/// Sub Bass, a mono preset), the mixer with a song's inserts, the time engine and
/// the looper, each run every block in that order. Running them together
/// also shows what they cost each other in the caches.
fn song(rate: u32) -> Case {
    let preset = |name: &str| {
        PRESETS
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("preset {name}"))
            .values
    };
    let mut parts = [
        sampler("sampler", rate, true, SamplerPads::All),
        synth("synth 1".to_string(), rate, preset("Supersaw"), 8),
        synth("synth 2".to_string(), rate, preset("Sub Bass"), 4),
        mixer("mixer", rate, 12, 8, Inserts::Song),
        time(rate, 8, false),
        looper(rate, 60),
    ];
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = parts
        .iter()
        .map(|_| (vec![0.0f32; MAX_BLOCK], vec![0.0f32; MAX_BLOCK]))
        .collect();
    Case {
        name: "Song: sampler, 2 synths, mixer+FX, time, looper".to_string(),
        module: "period",
        what: "every engine of a busy song in one block",
        block: Box::new(move |b, l, r| {
            let n = l.len();
            for (part, (bl, br)) in parts.iter_mut().zip(bufs.iter_mut()) {
                (part.block)(b, &mut bl[..n], &mut br[..n]);
            }
            // The mixer's output (part 3) plus the loop is what is heard.
            for i in 0..n {
                l[i] = bufs[3].0[i] + bufs[5].0[i];
                r[i] = bufs[3].1[i] + bufs[5].1[i];
            }
        }),
        mem_bytes: 0,
    }
}

/// Builds a case and records the bytes it holds.
fn measured(f: impl FnOnce() -> Case) -> Case {
    let (mut c, bytes) = alloc::bytes_in(f);
    c.mem_bytes = bytes;
    c
}

fn chain(rate: u32) -> Case {
    let mut chain = Chain::new();
    chain.prepare(rate, MAX_BLOCK);
    chain.push(Box::new(Drive::new()));
    chain.push(Box::new(MultiFilter::new()));
    chain.push(Box::new(Delay::new()));
    chain.push(Box::new(Compressor::new()));
    chain.set_param(1, ids::filter::CUTOFF, 800.0);
    let prog = program(rate);
    Case {
        name: "Chain: drive, filter, delay, comp".to_string(),
        module: "fx",
        what: "one track's inserts",
        block: Box::new(move |b, l, r| {
            let f0 = b * l.len() as u64;
            prog.play(f0, l, r);
            chain.process(&Ctx::new(rate, f0), l, r);
        }),
        mem_bytes: 0,
    }
}

/// Where the time case's notes go after the per-track arps: counted (in
/// the pad player they become sampler and synth events, whose cost is in
/// those cases).
struct TimeSink {
    notes: u64,
}

impl Sink for TimeSink {
    fn send(&mut self, _: Out) {
        self.notes += 1;
    }
}

/// The time engine as a busy song uses it: the transport looping 4 bars at
/// 128 BPM, and `tracks` clips playing at once (16ths with 3-note chords,
/// one at 3:2 speed, one swung and quantised), through the per-track
/// arps as the pad player runs them (16 of them), the first track's clip
/// through its arp at 32nds over 2 octaves. Every step has two parameter locks: one of the
/// instrument's (a different one each step, so each ends at the next
/// note) and one of the strip's. With `trigs`, every note also has a trig
/// condition (A:B, PRE, NEI, FILL, 1ST and their opposites in turn),
/// every fourth step is a ratchet of 3, and FILL turns on and off every
/// 64 blocks.
fn time(rate: u32, tracks: usize, trigs: bool) -> Case {
    use mpfree_seq::Cond;
    const CONDS: [Cond; 9] = [
        Cond::Always,
        Cond::Every { a: 1, b: 2 },
        Cond::Pre,
        Cond::NotPre,
        Cond::Fill,
        Cond::NotFill,
        Cond::Nei,
        Cond::NotFirst,
        Cond::Every { a: 3, b: 4 },
    ];
    let ppq = i64::from(PPQ);
    let mut song = Song::new();
    let mut launches = Vec::new();
    for i in 0..tracks {
        let mut t = Track::new("t", i as u16);
        let mut c = Clip::new("c", 8 * ppq);
        c.notes = (0..32i64)
            .flat_map(|k| {
                (0..3i64).map(move |n| Note {
                    tick: k * ppq / 4,
                    len: ppq / 4 - 10,
                    note: (36 + 12 * (i as i64 % 3) + n * 4 + k % 5) as u8,
                    velocity: 100,
                    chance: 100,
                    cond: if trigs {
                        CONDS[(k * 3 + n) as usize % CONDS.len()]
                    } else {
                        Cond::Always
                    },
                    ratchet: if trigs && k % 4 == 3 { 3 } else { 1 },
                    touch: mpfree_seq::Touch::NONE,
                })
            })
            .collect();
        c.locks = (0..32i64)
            .flat_map(|k| {
                [
                    Lock::new(
                        k * ppq / 4,
                        None,
                        RelAddr::new(OnTrack::Inst, (k % 4) as u16),
                        0.5,
                    ),
                    Lock::new(
                        k * ppq / 4,
                        None,
                        RelAddr::new(OnTrack::Strip, 0),
                        (k % 8) as f32 / 8.0,
                    ),
                ]
            })
            .collect();
        if i == 1 {
            c.speed = Speed::new(3, 2).expect("speed");
        }
        if i == 2 {
            c.swing = Some(Swing::new(ppq as u32 / 4, 62).expect("swing"));
            c.quantise = Quantise::new(ppq as u32 / 4, 80);
        }
        t.slots[0] = Some(c);
        song.tracks.push(t);
        launches.push(Launch {
            track: i,
            slot: 0,
            origin: 0,
            start: 0,
        });
    }
    let view = PlayView::compile(&song, &launches).expect("view");
    let mut t = Transport::new(rate, 128.0).expect("transport");
    t.command(0, T::SetLoop(Some((0, 16 * ppq))));
    t.command(0, T::Play);
    let mut arps = Box::new(TrackArps::new(rate, 128.0));
    arps.engine(0)
        .expect("track 1's arp")
        .set_settings(mpfree_arp::Settings {
            mode: mpfree_arp::Mode::Arp,
            rate: mpfree_arp::Rate::ThirtySecond,
            octaves: 2,
            ..mpfree_arp::Settings::default()
        });
    arps.set_through(0, true);
    let mut sink = TimeSink { notes: 0 };
    let mut player = Player::new();
    let mut block = TBlock::new();
    Case {
        name: if trigs {
            format!("Time with trig conditions and ratchets, {tracks} clips, arp")
        } else {
            format!("Time: transport, {tracks} clips, arp")
        },
        module: "time",
        what: if trigs {
            "as the Time case, with a trig condition on every note and ratchets"
        } else {
            "16ths with chords and 2 locks a step on every track, looping 4 bars"
        },
        block: Box::new(move |b, l, r| {
            if trigs && b % 64 == 0 {
                player.set_fill(b % 128 == 0, 0);
            }
            t.process(l.len() as u32, &mut block);
            player.process(&view, &block, &mut arps.through_sink());
            arps.finish(&block, &mut sink);
            // Silent, except the notes so far in the first sample, so the
            // self-test can see that notes flowed.
            l.fill(0.0);
            r.fill(0.0);
            l[0] = sink.notes as f32;
        }),
        mem_bytes: 0,
    }
}

/// The pad player's per-track arps with `n` of them busy at once, as when
/// several tracks hold latched arps: each latched a 4-note chord (pressed
/// on the pads through its arp) and plays it Up over 2 octaves in 32nds,
/// following the transport (128 BPM, swung). The other arps idle. No clips:
/// the Time case measures those.
fn live_arps(rate: u32, n: usize) -> Case {
    let mut t = Transport::new(rate, 128.0).expect("transport");
    t.command(
        0,
        T::SetSwing(mpfree_transport::Swing {
            grid: PPQ / 4,
            amount: 58,
        }),
    );
    t.command(0, T::Play);
    let mut arps = Box::new(TrackArps::new(rate, 128.0));
    for track in 0..n {
        let e = arps.engine(track).expect("an arp");
        e.set_settings(mpfree_arp::Settings {
            mode: mpfree_arp::Mode::Arp,
            rate: mpfree_arp::Rate::ThirtySecond,
            octaves: 2,
            ..mpfree_arp::Settings::default()
        });
        e.set_latch(mpfree_arp::LatchMode::Replace);
    }
    let view = PlayView::compile(&Song::new(), &[]).expect("view");
    let mut sink = TimeSink { notes: 0 };
    let mut player = Player::new();
    let mut block = TBlock::new();
    Case {
        name: format!("Arps: {n} latched arps at 32nds"),
        module: "time",
        what: "the per-track arps, n of 16 playing latched 4-note chords",
        block: Box::new(move |b, l, r| {
            t.process(l.len() as u32, &mut block);
            if b == 0 {
                for track in 0..n {
                    for k in 0..4u8 {
                        let note = 36 + track as u8 * 2 + k * 4;
                        arps.pad(
                            track,
                            0,
                            mpfree_play::Event::NoteOn {
                                note,
                                velocity: 100,
                            },
                        );
                        arps.pad(track, 1, mpfree_play::Event::NoteOff { note });
                    }
                }
            }
            player.process(&view, &block, &mut arps.through_sink());
            arps.finish(&block, &mut sink);
            l.fill(0.0);
            r.fill(0.0);
            l[0] = sink.notes as f32;
        }),
        mem_bytes: 0,
    }
}

/// The audio looper with `seconds` of memory, recording a 2-bar first pass
/// from the input at 120 BPM, then overdubbing layer after layer, with an
/// undo now and then, so the first-pass writer, the overdub writer, the
/// loop-point crossfade and playback of every layer all run.
fn looper(rate: u32, seconds: u32) -> Case {
    let mut t = Transport::new(rate, 120.0).expect("transport");
    t.command(0, T::Play);
    let mut lp = Looper::new(rate, seconds);
    lp.command(LooperCommand::Record);
    let mut block = TBlock::new();
    let frames = MAX_BLOCK;
    let (mut line_in, mut out) = (vec![0.0f32; 2 * frames], vec![0.0f32; 2 * frames]);
    let prog = program(rate);
    // 2 bars at 120 BPM: 4 s. Overdub from 5 s, toggled every 4 s; undo
    // every 30 s once there are layers (so it never fills up).
    let block_of = |s: f64| (s * f64::from(rate) / 128.0) as u64;
    let (od_start, od_every, undo_every) = (block_of(5.0), block_of(4.0), block_of(30.0));
    Case {
        name: format!("Looper ({seconds} s), overdubbing"),
        module: "looper",
        what: "a 2-bar loop with layer after layer overdubbed",
        block: Box::new(move |b, l, r| {
            let n = l.len();
            if b >= od_start && (b - od_start) % od_every == 0 {
                lp.command(LooperCommand::Overdub);
            }
            if b > od_start && b % undo_every == 0 {
                lp.command(LooperCommand::Undo);
            }
            prog.play(b * n as u64, l, r);
            for (i, (a, c)) in l.iter().zip(r.iter()).enumerate() {
                input_frame(&mut line_in, i, *a, *c);
            }
            out[..2 * n].fill(0.0);
            t.process(n as u32, &mut block);
            lp.process(&block, &line_in[..2 * n], &mut out[..2 * n]);
            for (i, (a, c)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
                *a = out[2 * i];
                *c = out[2 * i + 1];
            }
        }),
        mem_bytes: 0,
    }
}

/// The looper with its transport and line input, run a period at a time.
struct LoopRig {
    t: Transport,
    lp: Looper,
    block: TBlock,
    rate: u32,
    frame: u64,
    seed: Lcg,
    line_in: Vec<f32>,
    out: Vec<f32>,
}

impl LoopRig {
    /// One period of `l.len()` frames: the input from `l` and `r` goes in,
    /// the loops come out in them.
    fn period(&mut self, l: &mut [f32], r: &mut [f32]) {
        let n = l.len();
        for (i, (a, c)) in l.iter().zip(r.iter()).enumerate() {
            input_frame(&mut self.line_in, i, *a, *c);
        }
        self.out[..2 * n].fill(0.0);
        self.t.process(n as u32, &mut self.block);
        self.lp
            .process(&self.block, &self.line_in[..2 * n], &mut self.out[..2 * n]);
        for (i, (a, c)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
            *a = self.out[2 * i];
            *c = self.out[2 * i + 1];
        }
        self.frame += n as u64;
    }

    /// Play `seconds` of program material in, 128 frames at a time.
    fn run(&mut self, seconds: f64) {
        let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
        let end = self.frame + (seconds * f64::from(self.rate)) as u64;
        while self.frame < end {
            input(self.rate, self.frame, &mut self.seed, &mut l, &mut r);
            self.period(&mut l, &mut r);
        }
    }
}

/// [`MAX_LOOPS`] one-bar loops (2 s at 120 BPM) of three layers each,
/// all playing. `edited`: every other loop is edited, a quarter reversed
/// and a quarter trimmed to three beats and nudged (two pieces and a cut
/// with its seam fade, the dearest to play).
fn loops_rig(rate: u32, seconds: u32, edited: bool) -> LoopRig {
    let mut g = LoopRig {
        t: Transport::new(rate, 120.0).expect("transport"),
        lp: Looper::new(rate, seconds),
        block: TBlock::new(),
        rate,
        frame: 0,
        seed: Lcg(29),
        line_in: vec![0.0; 2 * MAX_BLOCK],
        out: vec![0.0; 2 * MAX_BLOCK],
    };
    g.t.command(0, T::Play);
    for k in 0..MAX_LOOPS {
        // The pass from the next bar line, going straight into an overdub
        // for a bar, then one more overdub.
        let phase = |g: &LoopRig| g.lp.loop_status(k).map(|s| s.phase);
        g.lp.select_loop(k);
        g.lp.command(LooperCommand::SetBars(Some(1)));
        g.lp.command(LooperCommand::Record);
        while phase(&g) != Some(LooperPhase::Recording) {
            g.run(0.05);
        }
        g.lp.command(LooperCommand::Overdub);
        while phase(&g) != Some(LooperPhase::Overdubbing) {
            g.run(0.05);
        }
        g.run(2.0);
        g.lp.command(LooperCommand::Overdub);
        g.run(0.25);
        g.lp.command(LooperCommand::Overdub);
        g.run(2.0);
        g.lp.command(LooperCommand::Overdub);
        g.run(0.25);
    }
    if edited {
        let beat = i64::from(PPQ);
        for k in (1..MAX_LOOPS).step_by(2) {
            g.lp.select_loop(k);
            if k % 4 == 1 {
                g.lp.command(LooperCommand::Edit(LooperEdit::Reverse));
            } else {
                g.lp.command(LooperCommand::Edit(LooperEdit::Trim {
                    from: 0,
                    to: 3 * beat,
                }));
                g.run(0.05);
                g.lp.command(LooperCommand::Edit(LooperEdit::Nudge(1)));
            }
            g.run(0.05);
        }
    }
    g
}

/// Every loop playing at once ([`loops_rig`]), recorded before the timing
/// starts. Timed: an overdub every other 2 s, on one loop after another,
/// so the block mixes every layer of every loop and writes one more.
fn looper_loops(rate: u32, seconds: u32, edited: bool) -> Case {
    let mut g = loops_rig(rate, seconds, edited);
    let every = 2 * u64::from(rate);
    let mut next = g.frame + every;
    let mut turn = 0usize;
    Case {
        name: format!(
            "Looper ({seconds} s), {MAX_LOOPS} loops of 3 layers playing{}",
            if edited { ", half edited" } else { "" }
        ),
        module: "looper",
        what: "every loop playing, with an overdub on one loop after another",
        block: Box::new(move |_, l, r| {
            if g.frame >= next {
                next += every;
                if turn.is_multiple_of(2) {
                    g.lp.select_loop(turn / 2 % MAX_LOOPS);
                }
                g.lp.command(LooperCommand::Overdub);
                turn += 1;
            }
            let f = g.frame;
            input(rate, f, &mut g.seed, l, r);
            g.period(l, r);
        }),
        mem_bytes: 0,
    }
}

/// Audio tracks on the song timeline: every track with 8 bars recorded at
/// 120 BPM in three loop passes, comped so a region boundary (a crossfade
/// between two takes) falls on every beat, playing in a loop over the 8
/// bars, with `recording` of them recording the input again (punching in
/// and out every 2 bars) while the rest play.
///
/// `streaming`: the same with 60 s of memory (the app's default) streaming
/// from a disk (a map here, answering at once): each block runs the
/// planning and copies in at most `host::COPY_SAMPLES` samples of read
/// chunks, as the host does on the audio thread. The three passes in play
/// on every track are 24 streams at once, the worst case for the cache.
fn audio_tracks(rate: u32, recording: usize, streaming: bool) -> Case {
    const BARS: i64 = 8;
    let bar = 4 * i64::from(PPQ);
    let beat = i64::from(PPQ);
    let song_end = BARS * bar;
    let mut t = Transport::new(rate, 120.0).expect("transport");
    // 8 bars at 120 BPM: 16 s a pass; three passes on 8 mono tracks are
    // 384 s of mono, and the recording tracks need more on top.
    let mut a = AudioTracks::new(rate, if streaming { 60 } else { 300 });
    a.stream(streaming);
    let mut disk: std::collections::HashMap<(u16, u32), Vec<f32>> = Default::default();
    let mut reads: std::collections::VecDeque<(u16, u32, usize)> =
        std::collections::VecDeque::with_capacity(4096);
    // The disk thread's part: writes at once; read chunks copy in `limit`
    // samples a block (the host's part).
    let serve = |a: &mut AudioTracks,
                 disk: &mut std::collections::HashMap<(u16, u32), Vec<f32>>,
                 reads: &mut std::collections::VecDeque<(u16, u32, usize)>,
                 limit: usize| {
        while let Some(r) = a.take_io() {
            match r {
                TrackIo::Write { take, chunk } => {
                    if let Some(c) = a.chunk(take, chunk) {
                        disk.insert((take, chunk), c.to_vec());
                    }
                    a.written(take, chunk);
                }
                TrackIo::Read { take, chunk } => reads.push_back((take, chunk, 0)),
                TrackIo::Finish { .. } => {}
            }
        }
        let mut budget = limit;
        while budget > 0 {
            let Some((take, chunk, done)) = reads.pop_front() else {
                break;
            };
            let Some(d) = disk.get(&(take, chunk)) else {
                a.fill_failed(take, chunk);
                continue;
            };
            let k = budget.min(d.len() - done);
            let last = done + k == d.len();
            let kept = a.fill_part(take, chunk, done, &d[done..done + k], last);
            budget -= k;
            if kept && !last {
                reads.push_front((take, chunk, done + k));
            }
        }
    };
    let mut block = TBlock::new();
    let n = MAX_BLOCK;
    let (mut line_in, mut out) = (vec![0.0f32; 2 * n], vec![0.0f32; 2 * n]);
    let prog = program(rate);
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    let mut frame = 0u64;
    let mut step = |frame: &mut u64,
                    t: &mut Transport,
                    a: &mut AudioTracks,
                    line_in: &mut Vec<f32>,
                    out: &mut Vec<f32>| {
        prog.play(*frame, &mut l, &mut r);
        for (i, (x, y)) in l.iter().zip(r.iter()).enumerate() {
            input_frame(line_in, i, *x, *y);
        }
        out.fill(0.0);
        t.process(n as u32, &mut block);
        let map = *t.tempo_map();
        a.process(&block, &map, line_in, out);
        *frame += n as u64;
        serve(a, &mut disk, &mut reads, usize::MAX);
    };
    // Record three passes on every track at once.
    for k in 0..MAX_TRACKS {
        a.command(TrackCommand::SetInput(k, TrackInput::Left));
        a.command(TrackCommand::Arm(k, true));
    }
    t.command(0, T::SetLoop(Some((0, song_end))));
    t.command(0, T::Record);
    let pass = (16 * rate) as u64;
    while frame < 3 * pass + 2 * n as u64 {
        step(&mut frame, &mut t, &mut a, &mut line_in, &mut out);
    }
    t.command(frame, T::Stop);
    for k in 0..MAX_TRACKS {
        a.command(TrackCommand::Arm(k, false));
    }
    step(&mut frame, &mut t, &mut a, &mut line_in, &mut out);
    // Comp: beat i of track k plays pass (i + k) % 3.
    let takes: Vec<_> = a.takes().collect();
    for k in 0..MAX_TRACKS {
        for i in 0..BARS * 4 {
            let p = ((i as usize + k) % 3) as u16;
            if let Some(tk) = takes.iter().find(|x| x.track == k && x.pass == p) {
                a.command(TrackCommand::Comp {
                    take: tk.id,
                    from: i * beat,
                    to: (i + 1) * beat,
                });
            }
        }
        // Apply the queue (it holds 64) before the next track.
        step(&mut frame, &mut t, &mut a, &mut line_in, &mut out);
    }
    for k in 0..recording {
        a.command(TrackCommand::Arm(k, true));
    }
    a.command(TrackCommand::ClearHistory);
    t.command(frame, T::PlayStart);
    let punch_every = 4 * u64::from(rate); // 2 bars
    let mut next_punch = frame + punch_every;
    let mut tr = t;
    Case {
        name: format!(
            "Audio tracks: {} {} comped takes{}",
            MAX_TRACKS - recording,
            if streaming { "streaming" } else { "playing" },
            if recording > 0 {
                format!(", {recording} recording")
            } else {
                String::new()
            }
        ),
        module: "looper",
        what: "8-bar song loop, a crossfade between takes on every beat",
        block: Box::new(move |b, ol, or| {
            let n = ol.len();
            if recording > 0 && frame >= next_punch {
                next_punch += punch_every;
                tr.command(frame, T::Record);
            }
            prog.play(b * n as u64, ol, or);
            for (i, (x, y)) in ol.iter().zip(or.iter()).enumerate() {
                input_frame(&mut line_in, i, *x, *y);
            }
            out[..2 * n].fill(0.0);
            tr.process(n as u32, &mut block);
            let map = *tr.tempo_map();
            a.process(&block, &map, &line_in[..2 * n], &mut out[..2 * n]);
            frame += n as u64;
            if streaming {
                serve(&mut a, &mut disk, &mut reads, COPY_SAMPLES);
            }
            for (i, (x, y)) in ol.iter_mut().zip(or.iter_mut()).enumerate() {
                *x = out[2 * i];
                *y = out[2 * i + 1];
            }
        }),
        mem_bytes: 0,
    }
}

fn input_frame(buf: &mut [f32], i: usize, l: f32, r: f32) {
    buf[2 * i] = l;
    buf[2 * i + 1] = r;
}

/// A chord for track `t` at a change: `voices` notes from a root that
/// moves with the change.
fn chord(t: usize, change: u64, voices: usize) -> impl Iterator<Item = u8> {
    let root = 36 + ((change * 5 + t as u64 * 7) % 24) as u8;
    (0..voices).map(move |k| root + [0u8, 7, 12, 16, 19, 24, 28, 31][k % 8] + 12 * (k / 8) as u8)
}

/// The track host as a song uses it: `tracks` tracks of Poly (Init), each
/// a `voices`-note chord changing every half second (the tracks a beat
/// apart), into one sum. The Poly work is the same as the Poly cases'; the
/// rest is the host: queues, sorting, per-track sums, the clip guard.
fn track_host(rate: u32, tracks: usize, voices: usize) -> Case {
    let max = MAX_BLOCK.min(1024);
    let mut host = TrackHost::new(tracks, rate, max);
    for t in 0..tracks {
        let mut p = Poly::new(rate);
        p.set_param(POLYPHONY, voices as f32);
        host.swap(t, Some(Instrument::new(Box::new(p), rate, max)))
            .expect("a free slot");
        host.route(t, Some(t));
    }
    let mut held: Vec<Vec<u8>> = (0..tracks).map(|_| Vec::with_capacity(voices)).collect();
    Case {
        name: format!("Track host, {tracks} Poly tracks of {voices} voices"),
        module: "synth",
        what: "chords changing every 0.5 s on each track",
        block: Box::new(move |b, l, r| {
            let len = l.len() as u64;
            let f0 = b * len;
            let half = u64::from(rate / 2);
            for (t, notes) in held.iter_mut().enumerate() {
                // Track t changes t/tracks of a half second after track 0.
                let shift = half * t as u64 / tracks as u64;
                let g = f0 + half - shift;
                if g % half + len > half || b == 0 {
                    let at = if b == 0 {
                        0
                    } else {
                        ((half - g % half) % len) as u32
                    };
                    for &n in notes.iter() {
                        host.note_off(t, at, n);
                    }
                    notes.clear();
                    for n in chord(t, g / half, voices) {
                        notes.push(n);
                        host.note_on(t, at, n, 96);
                    }
                }
            }
            l.fill(0.0);
            r.fill(0.0);
            host.render(&Ctx::new(rate, f0), l.len(), &mut Sum { left: l, right: r });
        }),
        mem_bytes: 0,
    }
}

/// An instrument that makes no sound and costs nothing, to see the host's
/// own share.
struct Quiet;

static QUIET_INFO: NodeInfo = NodeInfo {
    id: "bench.quiet",
    name: "Quiet",
    kind: Kind::Instrument,
};

impl Node for Quiet {
    fn info(&self) -> &'static NodeInfo {
        &QUIET_INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &[]
    }
    fn set_param(&mut self, _: u16, _: f32) {}
    fn param(&self, _: u16) -> f32 {
        f32::NAN
    }
    fn prepare(&mut self, _: u32, _: usize) {}
    fn reset(&mut self) {}
    fn process(&mut self, _: &Ctx, events: &[Timed], _: &mut [f32], _: &mut [f32]) {
        std::hint::black_box(events);
    }
}

/// The host alone: `tracks` tracks of an instrument that does nothing, with
/// four notes on and off in every block on every track.
fn track_host_alone(rate: u32, tracks: usize) -> Case {
    let max = MAX_BLOCK.min(1024);
    let mut host = TrackHost::new(tracks, rate, max);
    for t in 0..tracks {
        host.swap(t, Some(Instrument::new(Box::new(Quiet), rate, max)))
            .expect("a free slot");
        host.route(t, Some(t));
    }
    Case {
        name: format!("Track host alone, {tracks} tracks, 8 events each per block"),
        module: "synth",
        what: "the host's own work: queues, sorting, sums, the clip guard",
        block: Box::new(move |b, l, r| {
            let len = l.len() as u32;
            for t in 0..tracks {
                for k in 0..4u32 {
                    let note = 40 + ((b as u32 + k * 7 + t as u32) % 48) as u8;
                    // Out of order on purpose: the host sorts.
                    host.note_on(t, (len - 1 - k * 13) % len, note, 100);
                    host.note_off(t, (k * 29 + 3) % len, note);
                }
            }
            l.fill(0.0);
            r.fill(0.0);
            host.render(
                &Ctx::new(rate, b * u64::from(len)),
                l.len(),
                &mut Sum { left: l, right: r },
            );
        }),
        mem_bytes: 0,
    }
}

/// The mod engine with a busy performance set: 8 macros moving 8
/// parameters each, 16 matrix rows (8 LFOs, 2 followers listening to the
/// block's audio), the morph between two scenes swept by an LFO, and a
/// parameter lock every 16th note. Only the engine's own cost: what it
/// sends is counted, not applied (the nodes' cost is in their cases).
fn modulation(rate: u32) -> Case {
    use mpfree_mod::{
        MacroMap, MapCurve, ModEngine, ModEvent, ModTimed, Row, Source, DEFAULT_TICK,
    };
    use mpfree_node::{Curve, ParamInfo, Unit};
    use mpfree_param::{Owner, ParamAddr, Strip};
    const P: ParamInfo = ParamInfo {
        id: 0,
        name: "p",
        group: "g",
        min: 20.0,
        max: 20000.0,
        default: 1000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    };
    let mut e = ModEngine::new(rate, DEFAULT_TICK);
    let addr = |i: usize| {
        ParamAddr::new(
            Owner::Insert(Strip::Track((i / 16) as u8), (i % 4) as u16),
            (i % 16) as u16,
        )
    };
    for i in 0..80 {
        e.add_target(
            addr(i),
            ParamInfo {
                id: (i % 16) as u16,
                ..P
            },
            1000.0,
        )
        .expect("room");
    }
    for m in 0..8 {
        for k in 0..8 {
            e.map(
                m,
                MacroMap {
                    target: addr(m * 8 + k),
                    from: 0.1,
                    to: 0.9,
                    curve: MapCurve::S,
                },
            )
            .expect("map");
        }
    }
    for r in 0..16 {
        let source = if r < 14 {
            Source::Lfo((r % 8) as u8)
        } else {
            Source::Follower((r - 14) as u8)
        };
        e.add_row(Row {
            source,
            via: None,
            target: addr(64 + r),
            depth: 0.2,
        })
        .expect("row");
    }
    e.add_row(Row {
        source: Source::Lfo(7),
        via: None,
        target: ParamAddr::morph_pos(0),
        depth: 0.5,
    })
    .expect("row");
    e.capture_scene(0);
    for i in 0..80 {
        let _ = e.set_scene_value(addr(i), 1, Some(5000.0));
    }
    let mut seed = Lcg(9);
    let mut sent = 0u64;
    // A 16th note at 120 BPM.
    let step = u64::from(rate) / 8;
    let lock = addr(70);
    let mut events: Vec<ModTimed> = Vec::with_capacity(4);
    Case {
        name: "Mod: 8 macros x 8, 16 rows, morph, locks".to_string(),
        module: "fx",
        what: "macros, mod matrix and morph for a performance set",
        block: Box::new(move |b, l, r| {
            let len = l.len();
            let f0 = b * len as u64;
            input(rate, f0, &mut seed, l, r);
            events.clear();
            let next = f0.div_ceil(step) * step;
            if next < f0 + len as u64 {
                let n = next / step;
                let ev = if n % 2 == 0 {
                    ModEvent::Lock {
                        addr: lock,
                        value: 300.0 + (n % 7) as f32 * 100.0,
                    }
                } else {
                    ModEvent::Unlock { addr: lock }
                };
                events.push(ModTimed::new((next - f0) as u32, ev));
            }
            let ctx = Ctx {
                rate,
                frame: f0,
                bpm: 120.0,
                beats: f0 as f64 * 2.0 / rate as f64,
                playing: true,
                sidechain: None,
            };
            let follow = [Some((&l[..], &r[..])), Some((&r[..], &l[..])), None, None];
            e.process(&ctx, len, &events, &follow, &mut |_, _| sent += 1);
        }),
        mem_bytes: 0,
    }
}

/// FX moves on a busy clip: 16 lanes (the DJ filter, Perform pads' Holds,
/// knobs) of a 4-bar clip played period by period, a chase every bar, and
/// the always-on move log taking two knobs and an XY pad while it rolls.
fn moves(rate: u32) -> Case {
    use mpfree_moves::{Input, Kind, Map, MoveLog, Moves, Target};
    use mpfree_param::{Owner, ParamAddr, Strip};
    let len = 4 * 4 * PPQ;
    let addr = |p: u16| ParamAddr::new(Owner::Insert(Strip::Master, 1), p);
    let mut m = Moves::new();
    let mut seed = Lcg(77);
    for lane in 0..16u16 {
        let toggle = lane >= 12;
        let map = if toggle {
            Map::toggle()
        } else {
            Map::linear(-100.0, 100.0)
        };
        // Gestures of a knob turned every few beats, or pads pressed.
        let mut t = f64::from(lane) * 97.0;
        while t < f64::from(len) - 2000.0 {
            let n = 1 + (seed.next() % 40) as usize;
            let mut pts: Vec<(f64, f32, Kind)> = (0..n)
                .map(|k| {
                    let u = if toggle {
                        1.0
                    } else {
                        (seed.next() % 1000) as f32 / 1000.0
                    };
                    (t + k as f64 * 23.0, u, Kind::Ramp)
                })
                .collect();
            let end = t + n as f64 * 23.0 + 10.0;
            pts.push((end, 0.0, Kind::End));
            m.put(Target::Fixed(addr(lane)), map, &pts, len)
                .expect("put");
            t = end + 400.0 + f64::from(seed.next() % 1500);
        }
    }
    let mut log = MoveLog::new();
    let mut tp = Transport::new(rate, 120.0).expect("transport");
    let mut tb = TBlock::new();
    tp.command(0, T::Play);
    let mut tick = 0.0f64;
    let mut sent = 0u64;
    Case {
        name: "Moves: 16 lanes playing, the move log taking 3 controls".to_string(),
        module: "time",
        what: "a 4-bar clip's FX moves at 120 BPM, a chase each bar, knob and XY moves logged",
        block: Box::new(move |b, l, r| {
            let n = l.len() as u32;
            for (k, a) in [0u16, 1, 2].iter().enumerate() {
                if (b + k as u64).is_multiple_of(2) {
                    log.input(
                        (k as u32 * 37) % n,
                        Input::Move {
                            addr: addr(20 + a),
                            value: ((b % 200) as f32 - 100.0) * 0.9,
                            map: Map::linear(-100.0, 100.0),
                        },
                    );
                }
                if b % 300 == 299 {
                    log.input(n - 1, Input::End { addr: addr(20 + a) });
                }
            }
            tp.process(n, &mut tb);
            log.process(&tb);
            let step = f64::from(n) * 2.0 * f64::from(PPQ) / f64::from(rate);
            let next = tick + step;
            let lenf = f64::from(len);
            if next >= lenf {
                m.play(tick, lenf, |_, _, _| sent += 1);
                m.play(0.0, next - lenf, |_, _, _| sent += 1);
                tick = next - lenf;
            } else {
                m.play(tick, next, |_, _, _| sent += 1);
                tick = next;
            }
            if b % 690 == 0 {
                m.chase(tick, |_, _| sent += 1);
            }
            l[0] = sent as f32;
            r[0] = log.seq() as f32;
        }),
        mem_bytes: 0,
    }
}

/// The lane cases (`--lanes`): the same work on one core and split over
/// two with a lane (`mpfree-lanes`), so the gain and the handoff's cost
/// can be read side by side. `worker` pins each lane's worker to that
/// core. Each lane is closed when its case is dropped, before the next
/// case starts.
pub fn lanes(rate: u32, worker: Option<usize>) -> Vec<Case> {
    let config = |spin: bool| LaneConfig {
        name: "mpfree-bench-ln".into(),
        core: worker,
        // Spinning: longer than any gap between the bench's blocks, so the
        // worker never sleeps (the lowest handoff, a busy core).
        worker_spin: if spin {
            Duration::from_millis(20)
        } else {
            Duration::ZERO
        },
        ..LaneConfig::default()
    };
    let probe = Lane::new(|_b: u64| {}, config(false));
    let s = probe.setup();
    println!(
        "Lanes: {}; worker core: {}",
        match &s.threaded {
            Ok(()) => "a worker thread each".to_string(),
            Err(e) => format!("on this thread ({e})"),
        },
        match &s.pinned {
            None => "not pinned".to_string(),
            Some(Ok(c)) => format!("pinned to {c}"),
            Some(Err(e)) => format!("not pinned ({e})"),
        }
    );
    drop(probe);
    vec![
        handoff("Lane handoff, empty job, worker sleeps", config(false)),
        handoff("Lane handoff, empty job, worker spins", config(true)),
        measured(|| {
            sampler(
                "Sampler 24 voices, one engine, one core",
                rate,
                false,
                SamplerPads::All,
            )
        }),
        measured(|| {
            split_sampler(
                "Sampler as 2 x 12 voices, one core",
                rate,
                None,
                lane_wait(),
            )
        }),
        measured(|| {
            split_sampler(
                "Sampler as 2 x 12 voices, 2 cores, worker sleeps",
                rate,
                Some(config(false)),
                lane_wait(),
            )
        }),
        measured(|| {
            split_sampler(
                "Sampler as 2 x 12 voices, 2 cores, worker spins",
                rate,
                Some(config(true)),
                lane_wait(),
            )
        }),
        measured(|| native_split("Sampler 24 voices split natively, one core", rate, None)),
        measured(|| {
            native_split(
                "Sampler 24 voices split natively, 2 cores, worker sleeps",
                rate,
                Some(config(false)),
            )
        }),
        measured(|| {
            native_split(
                "Sampler 24 voices split natively, 2 cores, worker spins",
                rate,
                Some(config(true)),
            )
        }),
        measured(|| {
            beside(
                "Sampler 24 voices + Poly Init 8 voices, 1 core",
                rate,
                None,
                lane_wait(),
            )
        }),
        measured(|| {
            mixer_with(
                "Mixer, a song's inserts, one core",
                rate,
                12,
                8,
                Inserts::Song,
                Second::None,
            )
        }),
        measured(|| {
            mixer_with(
                "Mixer, a song's inserts, track inserts on 2 cores",
                rate,
                12,
                8,
                Inserts::Song,
                Second::Tracks(config(false)),
            )
        }),
        measured(|| {
            mixer_with(
                "Mixer, a song's inserts, returns a block late on 2 cores",
                rate,
                12,
                8,
                Inserts::Song,
                Second::Returns(config(false)),
            )
        }),
        measured(|| {
            mixer_with(
                "Mixer, a song's inserts, Eco reverb, one core",
                rate,
                12,
                8,
                Inserts::SongEco,
                Second::None,
            )
        }),
        measured(|| {
            mixer_with(
                "Mixer, a song's inserts, Eco reverb, returns a block late on 2 cores",
                rate,
                12,
                8,
                Inserts::SongEco,
                Second::Returns(config(false)),
            )
        }),
        measured(|| {
            beside(
                "Sampler 24 voices + Poly Init 8 voices, 2 cores",
                rate,
                Some(config(false)),
                lane_wait(),
            )
        }),
    ]
}

/// How long a lane case waits for its lane: ten periods, so the numbers
/// show the real time a split takes rather than a cut-off. A join that
/// misses it leaves the lane's part out of that block, and the lane's next
/// block too while it finishes, so the case stops matching its one-core
/// twin; the sound tests wait far longer for that reason.
fn lane_wait() -> Duration {
    Duration::from_nanos(10 * MPC.ns())
}

/// A lane with nothing to do: the handoff alone (post, wake, run, say
/// done, see it).
fn handoff(name: &str, config: LaneConfig) -> Case {
    let mut lane = Lane::new(|_b: u64| {}, config);
    Case {
        name: name.to_string(),
        module: "lanes",
        what: "an empty job posted and joined every block",
        block: Box::new(move |b, _l, _r| {
            if lane.post(b).is_ok() {
                let _ = lane.join(Instant::now() + lane_wait());
            }
        }),
        mem_bytes: 0,
    }
}

/// A part of a period's render that a lane runs: an engine and its own
/// output pair.
struct Part<E> {
    engine: E,
    l: Vec<f32>,
    r: Vec<f32>,
}

impl<E> Part<E> {
    fn new(engine: E) -> Self {
        let n = MPC.frames as usize;
        Self {
            engine,
            l: vec![0.0; n],
            r: vec![0.0; n],
        }
    }

    /// Adds the part's output into `l` and `r`.
    fn mix_into(&self, l: &mut [f32], r: &mut [f32]) {
        for (a, b) in l.iter_mut().zip(&self.l) {
            *a += b;
        }
        for (a, b) in r.iter_mut().zip(&self.r) {
            *a += b;
        }
    }
}

/// Half the sampler's pads, with this block's hits.
struct SamplerPart {
    e: Engine,
    hits: Option<[Event; 2]>,
}

impl Job for Part<SamplerPart> {
    fn run(&mut self, _block: u64) {
        let p = &mut self.engine;
        for ev in p.hits.take().into_iter().flatten() {
            p.e.apply(ev);
        }
        self.l.fill(0.0);
        self.r.fill(0.0);
        p.e.render(&mut self.l, &mut self.r);
    }
}

impl Job for Part<Chords> {
    fn run(&mut self, block: u64) {
        self.engine.block(block, &mut self.l, &mut self.r);
    }
}

/// The [`sampler`] case split in two by pad (even pads here, odd pads on
/// the lane), 12 voices each, every hit to both. `lane` None: the lane runs
/// on this thread, which is the same work on one core. Each block waits up
/// to `wait` for the lane.
fn split_sampler(name: &str, rate: u32, lane: Option<LaneConfig>, wait: Duration) -> Case {
    let mut here = sampler_engine(rate, false, SamplerPads::All, 12, |p| p % 2 == 0);
    let there = Part::new(SamplerPart {
        e: sampler_engine(rate, false, SamplerPads::All, 12, |p| p % 2 == 1),
        hits: None,
    });
    let mut lane = match lane {
        Some(c) => Lane::new(there, c),
        None => Lane::inline(there),
    };
    let mut hits = SamplerHits::new();
    Case {
        name: name.to_string(),
        module: "sampler",
        what: "the sampler case's pads split over two engines of 12 voices",
        block: Box::new(move |b, l, r| {
            let ev = hits.at(b);
            let posted = match lane.job() {
                Some(j) => {
                    j.engine.hits = ev;
                    lane.post(b).is_ok()
                }
                None => false,
            };
            for e in ev.into_iter().flatten() {
                here.apply(e);
            }
            l.fill(0.0);
            r.fill(0.0);
            here.render(l, r);
            if posted {
                if let Ok(j) = lane.join(Instant::now() + wait) {
                    j.mix_into(l, r);
                }
            }
        }),
        mem_bytes: 0,
    }
}

/// The [`sampler`] case with the engine's own split (`Engine::split`):
/// one engine of 24 voices, the second 12 rendered on a lane, every hit,
/// choke and steal still seeing all 24. `lane` None: the lane runs on this
/// thread, which is the same work on one core.
fn native_split(name: &str, rate: u32, lane: Option<LaneConfig>) -> Case {
    let mut e = sampler_engine(rate, false, SamplerPads::All, 24, |_| true);
    let config = lane.unwrap_or(LaneConfig {
        name: "mpfree-bench-ln".into(),
        threaded: false,
        ..LaneConfig::default()
    });
    e.split(config);
    let mut hits = SamplerHits::new();
    Case {
        name: name.to_string(),
        module: "sampler",
        what: "the sampler case, its voices split over two cores by the engine",
        block: Box::new(move |b, l, r| {
            for ev in hits.at(b).into_iter().flatten() {
                e.apply(ev);
            }
            e.render_until(l, r, Instant::now() + lane_wait());
        }),
        mem_bytes: 0,
    }
}

/// The [`sampler`] case and Poly Init playing 8-voice chords: both here
/// (`lane` None), or Poly on a lane beside the sampler. Each block waits up
/// to `wait` for the lane.
fn beside(name: &str, rate: u32, lane: Option<LaneConfig>, wait: Duration) -> Case {
    let mut here = sampler_engine(rate, false, SamplerPads::All, 24, |_| true);
    let poly = Part::new(Chords::new(
        rate,
        Box::new(Poly::new(rate)),
        PRESETS[0].values,
        Some(POLYPHONY),
        8,
    ));
    let mut lane = match lane {
        Some(c) => Lane::new(poly, c),
        None => Lane::inline(poly),
    };
    let mut hits = SamplerHits::new();
    Case {
        name: name.to_string(),
        module: "lanes",
        what: "the sampler case here, Poly Init chords on a lane",
        block: Box::new(move |b, l, r| {
            let posted = lane.post(b).is_ok();
            for e in hits.at(b).into_iter().flatten() {
                here.apply(e);
            }
            l.fill(0.0);
            r.fill(0.0);
            here.render(l, r);
            if posted {
                if let Ok(j) = lane.join(Instant::now() + wait) {
                    j.mix_into(l, r);
                }
            }
        }),
        mem_bytes: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_moves_case_plays_and_logs() {
        let mut c = measured(|| moves(44_100));
        let (l, r) = run(&mut c, 2_000);
        // 2000 periods is about 5.8 s: 16 lanes sending every 8 ticks
        // while they move, and the log about 3 moves a period.
        assert!(l[0] > 5_000.0, "{}", l[0]);
        assert!(r[0] > 2_000.0, "{}", r[0]);
    }

    #[test]
    fn the_frozen_mixer_case_prints_and_plays() {
        let mut c = measured(|| mixer("frozen", 44_100, 12, 8, Inserts::SongFrozen));
        let (l, _) = run(&mut c, 400);
        assert!(l.iter().any(|v| v.abs() > 1e-4), "it plays");
    }

    /// Every block's output from two cases, side by side.
    fn same_blocks(mut a: Case, mut b: Case, blocks: u64) {
        let (mut la, mut ra) = (vec![0.0f32; 128], vec![0.0f32; 128]);
        let (mut lb, mut rb) = (vec![0.0f32; 128], vec![0.0f32; 128]);
        let mut loud = false;
        for k in 0..blocks {
            (a.block)(k, &mut la, &mut ra);
            (b.block)(k, &mut lb, &mut rb);
            assert_eq!((&la, &ra), (&lb, &rb), "block {k}");
            loud |= la.iter().any(|v| v.abs() > 1e-4);
        }
        assert!(loud, "the cases made no sound");
    }

    #[test]
    fn a_split_on_two_cores_sounds_exactly_like_the_split_on_one() {
        let lane = || Some(LaneConfig::default());
        let tracks = || Second::Tracks(LaneConfig::default());
        // The bench's ten periods (29 ms) are too short for a debug build on
        // a busy test runner, where the worker can wait that long for a
        // core; a late block would leave the lane's part out and fail the
        // comparison for a reason that is not the sound. So wait as long as
        // it takes, within reason.
        let wait = Duration::from_secs(60);
        same_blocks(
            split_sampler("one", 44_100, None, wait),
            split_sampler("two", 44_100, lane(), wait),
            600,
        );
        same_blocks(
            beside("one", 44_100, None, wait),
            beside("two", 44_100, lane(), wait),
            600,
        );
        same_blocks(
            native_split("one", 44_100, None),
            native_split("two", 44_100, lane()),
            600,
        );
        same_blocks(
            mixer_with("one", 44_100, 12, 8, Inserts::Song, Second::None),
            mixer_with("two", 44_100, 12, 8, Inserts::Song, tracks()),
            600,
        );
    }

    #[test]
    fn the_lane_cases_cover_one_core_and_two() {
        let mut cases = lanes(44_100, None);
        assert_eq!(cases.len(), 16);
        assert!(cases.iter().any(|c| c.name.contains("one core")));
        assert!(cases.iter().any(|c| c.name.contains("2 cores")));
        for c in &mut cases {
            run(c, 20);
        }
    }

    fn run(c: &mut Case, blocks: u64) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
        for b in 0..blocks {
            (c.block)(b, &mut l, &mut r);
        }
        (l, r)
    }

    #[test]
    fn the_time_case_plays_notes_through_the_arp() {
        let mut c = measured(|| time(44_100, 8, false));
        let (l, _) = run(&mut c, 3_000);
        // 8 s at 128 BPM is about 17 beats: 8 tracks x 3 notes x 4 per beat
        // as note-ons and offs, plus the arp's notes.
        assert!(l[0] > 1_000.0, "{}", l[0]);
        assert!(c.mem_bytes > 0 || !alloc::installed());
        // With trig conditions some notes are left out, and ratchets add hits.
        let mut t = measured(|| time(44_100, 8, true));
        let (lt, _) = run(&mut t, 3_000);
        assert!(lt[0] > 500.0 && lt[0] != l[0], "{} against {}", lt[0], l[0]);
    }

    #[test]
    fn the_live_arps_case_plays_every_busy_arp() {
        let mut four = measured(|| live_arps(44_100, 4));
        let (l4, _) = run(&mut four, 1_000);
        let mut all = measured(|| live_arps(44_100, 16));
        let (l16, _) = run(&mut all, 1_000);
        // About 3 s at 128 BPM: some 50 32nds, an on and an off each.
        assert!(l4[0] > 300.0, "{}", l4[0]);
        assert!(
            (l16[0] / l4[0] - 4.0).abs() < 0.2,
            "{} against {}",
            l16[0],
            l4[0]
        );
    }

    #[test]
    fn the_looper_case_plays_its_loop_back() {
        let mut c = measured(|| looper(44_100, 20));
        // Past the first pass (4 s) and into the overdubs.
        let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
        let mut energy = 0.0f64;
        for b in 0..2_500u64 {
            (c.block)(b, &mut l, &mut r);
            if b > 1_600 {
                energy += l
                    .iter()
                    .chain(r.iter())
                    .map(|v| f64::from(v.abs()))
                    .sum::<f64>();
            }
        }
        assert!(energy > 100.0, "{energy}");
        if alloc::installed() {
            // 20 s of stereo f32.
            assert!(c.mem_bytes >= 20 * 44_100 * 2 * 4, "{}", c.mem_bytes);
        }
    }

    #[test]
    fn a_line_fits_and_held_pads_sound() {
        let (a, b) = fit(&[(1, 12.0), (2, 14.0), (4, 18.0), (8, 26.0)]);
        assert!((a - 10.0).abs() < 1e-9 && (b - 2.0).abs() < 1e-9, "{a} {b}");
        let mut c = held_pads(44_100, 4);
        let (l, _) = run(&mut c, 100);
        assert!(l.iter().any(|v| v.abs() > 1e-4));
        assert!(scaling(44_100)
            .iter()
            .all(|s| s.points.len() == 5 || (s.points.len() == 1 && s.name.ends_with("(mono)"))));
    }

    #[test]
    fn the_loops_case_has_every_loop_playing() {
        let mut c = measured(|| looper_loops(44_100, 60, true));
        let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
        let mut energy = 0.0f64;
        for b in 0..1_500u64 {
            (c.block)(b, &mut l, &mut r);
            energy += l
                .iter()
                .chain(r.iter())
                .map(|v| f64::from(v.abs()))
                .sum::<f64>();
        }
        assert!(energy > 1_000.0, "{energy}");
        let g = loops_rig(44_100, 60, true);
        for k in 0..MAX_LOOPS {
            let s = g.lp.loop_status(k).unwrap();
            // Trimmed to three beats of the bar.
            let frames = if k % 4 == 3 { 66_150 } else { 88_200 };
            assert_eq!(
                (s.phase, s.layers, s.frames),
                (LooperPhase::Playing, 3, frames),
                "loop {k}"
            );
        }
        let st = g.lp.stats();
        assert_eq!((st.refused, st.aborted, st.dropped_commands), (0, 0, 0));
    }

    #[test]
    fn the_grain_points_hold_about_that_many_grains() {
        for n in [16usize, 64, 256] {
            let mut e = grain_engine(44_100, n);
            for pad in 0..4 {
                e.apply(Event::NoteOn { pad, velocity: 100 });
            }
            let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
            let mut live = 0usize;
            for b in 0..700 {
                e.render(&mut l, &mut r);
                if b >= 350 {
                    live += e.live_grains();
                }
            }
            let mean = live as f64 / 350.0;
            assert!(
                (mean - n as f64).abs() <= n as f64 * 0.25,
                "{n} grains asked, {mean:.1} alive"
            );
        }
    }

    #[test]
    fn every_case_says_what_it_holds() {
        let cases = all(44_100);
        assert!(
            cases.iter().any(|c| c.module == "looper") && cases.iter().any(|c| c.module == "time")
        );
        if alloc::installed() {
            assert!(
                cases.iter().all(|c| c.mem_bytes > 0),
                "{:?}",
                cases
                    .iter()
                    .map(|c| (&c.name, c.mem_bytes))
                    .collect::<Vec<_>>()
            );
        }
    }
}
