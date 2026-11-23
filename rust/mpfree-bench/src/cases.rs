//! The workloads: each engine set up the way a busy song would use it, fed
//! block by block at the MPC's period. Setup allocates; the block function
//! must not.

use mpfree_audiotracks::host::COPY_SAMPLES;
use mpfree_audiotracks::{
    AudioTracks, Command as TrackCommand, Input as TrackInput, Io as TrackIo, MAX_TRACKS,
};
use mpfree_fx::ids;
use mpfree_fx::{
    Amp, AutoPan, AutoWah, CapturePoint, Chain, Character, Chorus, Cloud, Comb, Compressor, Crush,
    DeEsser, Delay, DjFilter, Drive, Ducker, Ensemble, Eq, Flanger, FreqShifter, Frozen, Fuzz,
    Gate, Harmonizer, Limiter, LoFi, MasterSafety, Mixer, MixerConfig, MultiFilter, MultiTap,
    MultibandCompressor, Perform, Phaser, PitchCorrect, PitchShifter, Plate, Print, PrintJob,
    PrintState, RingMod, Room, Rotary, Spring, StripId, Tail, TalkBox, TapeDelay, TransientShaper,
    Tremolo, Tuner, Vocoder,
};
use mpfree_grain::GrainNode;
use mpfree_lanes::{Config as LaneConfig, Job, Lane};
use mpfree_looper::{
    Command as LooperCommand, Edit as LooperEdit, Looper, Phase as LooperPhase, MAX_LOOPS,
};
use mpfree_node::{Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Timed, MAX_BLOCK};
use mpfree_play::{Ahead, Launch, Out, PlayView, Player, Sink, TrackArps};
use mpfree_prof::alloc;
use mpfree_prof::budget::MPC;
use mpfree_sampler::dsp::{Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event, GRAIN_BUDGET};
use mpfree_sampler::grain_share::RESERVE;
use mpfree_sampler::model::*;
use mpfree_seq::mpfree_param::{OnTrack, RelAddr};
use mpfree_seq::{Clip, Lock, Note, Project as Song, Quantise, Speed, Track};
use mpfree_synth::params::{
    DIST, DRIVE, OSC1_LEVEL, OSC1_POS, OSC1_WARP, OSC1_WARP_MODE, OSC1_WAVE, OSC2_LEVEL, OSC2_WAVE,
    POLYPHONY, UNISON, VOICE_MODE, WAVE_TABLE,
};
use mpfree_synth::wavetable::params as wt;
use mpfree_synth::{
    Analog, Bass, Choir, Drawbar, Drums, Fm, Keys, Pluck, Poly, Squelch, Strings, Wavetable, Wind,
};
use mpfree_track::{Instrument, Sum, TrackHost};
use mpfree_transport::time::{Swing, PPQ};
use mpfree_transport::{Block as TBlock, Command as T, Transport};

use mpfree_stream::coord::{Coordinator, Inline, Plan as StreamPlan};
use mpfree_stream::cursor::{Cursor as StreamCursor, Read};
use mpfree_stream::player::{Config as StreamConfig, Player as StreamPlayer};

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

/// The EQ as its bench case sets it, with every band's Freq moving: a new
/// target every 8 blocks, half an octave up and back, so the bands never
/// settle and each block pays for its designs (FX quality E11).
fn eq_gliding(rate: u32) -> Case {
    let mut n = Eq::new();
    n.prepare(rate, MAX_BLOCK);
    for (id, v) in [
        (ids::eq::LOW_SLOPE, 2.0),
        (ids::eq::HIGH_SLOPE, 2.0),
        (ids::eq::gain(0), 3.0),
        (ids::eq::gain(1), -4.0),
        (ids::eq::gain(2), 5.0),
        (ids::eq::gain(3), 2.0),
    ] {
        n.set_param(id, v);
    }
    n.reset();
    let base: [f32; 4] = std::array::from_fn(|k| n.param(ids::eq::freq(k)));
    let prog = program(rate);
    Case {
        name: "EQ (4 bands and both cuts), Freq gliding".to_string(),
        module: "fx",
        what: "tone and noise bursts every 0.5 s, every band's Freq moving",
        block: Box::new(move |b, l, r| {
            let f0 = b * l.len() as u64;
            if b % 8 == 0 {
                let up = if (b / 8) % 2 == 0 { 1.414 } else { 1.0 };
                for (k, f) in base.iter().enumerate() {
                    n.set_param(ids::eq::freq(k), f * up);
                }
            }
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

/// The drum synth with a factory kit, every pad hit at once each block:
/// 16 hits starting and the 16 before them fading, 32 sounding (D10).
fn drums_all(name: &str, rate: u32, kit: &str) -> Case {
    let mut d = Drums::new(rate);
    d.prepare(rate, MAX_BLOCK);
    let k = Drums::presets()
        .iter()
        .find(|k| k.name == kit)
        .unwrap_or_else(|| panic!("kit {kit}"));
    d.load_preset(k);
    let mut seed = Lcg(23);
    let mut events: Vec<Timed> = Vec::with_capacity(16);
    Case {
        name: name.to_string(),
        module: "synth",
        what: "every pad each block, 32 hits sounding",
        block: Box::new(move |b, l, r| {
            events.clear();
            for pad in 0..16u8 {
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

/// The drum synth's per-drum effects (`mpfree_synth::drum_fx`, D10): every
/// pad of the Init Kit ringing 2 s, one pad hit each block in turn so all
/// 16 sound at once (with the retriggers fading), each pad with `fx` (its
/// effects' fields and values) and, with `sends`, both kit sends.
fn drums_fx(name: &str, rate: u32, fx: &[(u16, f32)], sends: bool) -> Case {
    use mpfree_synth::drum_fx::{field as f, fx_id};
    use mpfree_synth::drum_params::{field as df, pad_id};
    let mut d = Drums::new(rate);
    d.prepare(rate, MAX_BLOCK);
    for pad in 0..16 {
        d.set_param(pad_id(pad, df::DECAY), 2000.0);
        for &(field, v) in fx {
            d.set_param(fx_id(pad, field), v);
        }
        if sends {
            d.set_param(fx_id(pad, f::DELAY_SEND), 20.0);
            d.set_param(fx_id(pad, f::REVERB_SEND), 30.0);
        }
    }
    let mut seed = Lcg(23);
    let mut events: Vec<Timed> = Vec::with_capacity(1);
    Case {
        name: name.to_string(),
        module: "synth",
        what: "16 pads ringing, one hit a block",
        block: Box::new(move |b, l, r| {
            events.clear();
            events.push(Timed::new(
                0,
                NodeEvent::NoteOn {
                    note: 36 + (b % 16) as u8,
                    velocity: 60 + (seed.next() >> 25) as u8 % 68,
                },
            ));
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

/// What a sampler case may trade away to keep going (sampler plan step
/// 10). Anything else it trades fails the bench run, as the case ends.
#[derive(Clone, Copy, Default)]
struct Allow {
    /// Voices taken over for new hits (a pad past its polyphony, a full
    /// pool): normal playing, which the status board never hears of.
    steals: bool,
    /// Grains thinned, faded or held back for the grain budget.
    grains: bool,
    /// A core late with its voices (lane cases on real threads).
    late: bool,
}

/// A bench case's sampler engine, checked as its case ends against what
/// it may trade ([`Allow`]). Its counts go to a status board as the app
/// sends them, and [`Board::unexpected`](mpfree_status::Board::unexpected)
/// names what was shown that the case doesn't allow: notes dropped and
/// voices given up always fail, grains thinned and a skipped part of the
/// sound unless allowed, and steals unless allowed. The check runs when
/// the case is dropped, after it is counted, so it costs the counts
/// nothing.
struct Watched {
    e: Engine,
    name: String,
    allow: Allow,
}

impl Watched {
    fn new(e: Engine, name: &str, allow: Allow) -> Self {
        Self {
            e,
            name: name.to_string(),
            allow,
        }
    }

    /// The trades the case made that it may not, with how many.
    fn unexpected(&self) -> Vec<String> {
        use mpfree_status::{kinds, Board, Report, SETTLE_ON};
        let st = self.e.stats();
        let a = self.allow;
        let mut board = Board::new();
        let t0 = std::time::Instant::now();
        for (kind, n) in [
            (&kinds::NOTES_DROPPED, st.dropped_events),
            (&kinds::VOICES_GIVEN_UP, st.culls),
            (&kinds::SOUND_SKIPPED, st.lane_late),
        ] {
            if n > 0 {
                let times = u32::try_from(n).unwrap_or(u32::MAX);
                board.apply(Report::happened(kind, times), t0);
            }
        }
        let thinned = [
            st.skipped_grains,
            st.crowded_frames,
            st.faded_grains,
            st.held_back_clouds,
        ];
        if thinned.iter().any(|&n| n > 0) {
            board.apply(Report::began(&kinds::GRAINS_THINNED), t0);
            board.tick(t0 + SETTLE_ON);
        }
        let mut allowed = Vec::new();
        if a.grains {
            allowed.push(&kinds::GRAINS_THINNED);
        }
        if a.late {
            allowed.push(&kinds::SOUND_SKIPPED);
        }
        let mut bad: Vec<String> = board
            .unexpected(&allowed)
            .into_iter()
            .map(|k| match k.id {
                "sound/grains-thinned" => format!("{} {thinned:?}", k.title),
                _ => {
                    let n = board
                        .raw()
                        .iter()
                        .find(|v| v.kind == k)
                        .map_or(0, |v| v.times);
                    format!("{}: {n}", k.title)
                }
            })
            .collect();
        if st.steals > 0 && !a.steals {
            bad.push(format!("Voices stolen: {}", st.steals));
        }
        bad
    }
}

impl std::ops::Deref for Watched {
    type Target = Engine;
    fn deref(&self) -> &Engine {
        &self.e
    }
}

impl std::ops::DerefMut for Watched {
    fn deref_mut(&mut self) -> &mut Engine {
        &mut self.e
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        let bad = self.unexpected();
        if !bad.is_empty() && !std::thread::panicking() {
            panic!(
                "{}: the sampler traded away what this case may not: {bad:?}",
                self.name
            );
        }
    }
}

/// Which of the sampler case's pads are set. The hits are the same in
/// every case (a hit on an unset pad does nothing), so the two parts add
/// up to the whole.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SamplerPads {
    All,
    /// Pads 1 to 16: sample voices (Hermite read, the three tuned more
    /// than two semitones up the clean band-limited read; envelope,
    /// filter).
    Samples,
    /// Pads 17 to 20: granular voices.
    Grains,
}

/// The sampler as the pad player uses it: 16 one-shot, gated and looping
/// pads with filters and two layers, four granular pads, 24 voices, a hit
/// every 4 blocks (about 11 a second), and optionally the master grain
/// effect.
fn sampler(name: &str, rate: u32, master: bool, which: SamplerPads) -> Case {
    let allow = Allow {
        steals: true,
        grains: true,
        ..Allow::default()
    };
    let mut e = Watched::new(
        sampler_engine(rate, master, which, 24, |_| true),
        name,
        allow,
    );
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
            drive_db: 0.0,
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

/// The pad player's drum kits as it plays them: 16 one-shot pads with the
/// kit pads' settings (no filter, the envelope open, at the sound's own
/// rate, 4 voices a pad), 2 s sounds, hit so that `voices` voices play
/// all the time (a hit every block, the next pad round, each voice ringing
/// until a hit on its pad pushes it out). Jay's round 4 kits, with Note
/// Repeat, kept the pool full like this. `sixteen`: the sounds are 16-bit
/// (a 16-bit WAV's), held and read as 16 bits.
fn drum_kit(name: &str, rate: u32, voices: usize, sixteen: bool) -> Case {
    let s = Source::new("drum", rate, 2, bench_sound(rate as usize * 2, sixteen)).expect("source");
    let mut p = Project::new();
    p.add_source(s.clone());
    for i in 0..16usize {
        let mut pad = Pad::new("kit", Layer::new(Region::whole(&s)));
        pad.polyphony = 4;
        pad.level_db = 3.0;
        p.set_pad(i, Some(pad), None).expect("pad");
    }
    let mut e = Engine::new(rate, voices);
    e.set_view(EngineView::build(&p, rate));
    // Each hit pushes out the oldest voice of its pad.
    let allow = Allow {
        steals: true,
        ..Allow::default()
    };
    let mut e = Watched::new(e, name, allow);
    let mut b0 = 0u64;
    Case {
        name: name.to_string(),
        module: "sampler",
        what: "16 kit pads, one-shot and open, every voice playing",
        block: Box::new(move |_b, l, r| {
            e.apply(Event::NoteOn {
                pad: (b0 % 16) as u8,
                velocity: 100,
            });
            b0 += 1;
            l.fill(0.0);
            r.fill(0.0);
            e.render(l, r);
        }),
        mem_bytes: 0,
    }
}

/// [`drum_kit`] on `kits` tracks at once, as a kit per track plays them:
/// one engine of `voices` voices with a copy of the kit on each of `kits`
/// kits, a hit every block on the next kit in turn (the next pad round on
/// each), so the pool stays full and steals across kits.
fn drum_kits(name: &str, rate: u32, voices: usize, kits: usize) -> Case {
    let s = Source::new("drum", rate, 2, bench_sound(rate as usize * 2, false)).expect("source");
    let mut p = Project::new();
    p.add_source(s.clone());
    for i in 0..16usize {
        let mut pad = Pad::new("kit", Layer::new(Region::whole(&s)));
        pad.polyphony = 4;
        pad.level_db = 3.0;
        p.set_pad(i, Some(pad), None).expect("pad");
    }
    let mut e = Engine::with_kits(rate, voices, kits);
    for k in 0..kits {
        e.set_kit_view(k, EngineView::build(&p, rate));
    }
    let allow = Allow {
        steals: true,
        ..Allow::default()
    };
    let mut e = Watched::new(e, name, allow);
    let mut b0 = 0u64;
    Case {
        name: name.to_string(),
        module: "sampler",
        what: "16 kit pads on each of several kits, one-shot and open, every voice playing",
        block: Box::new(move |_b, l, r| {
            let k = (b0 % kits as u64) as usize;
            e.apply_for(
                k,
                Event::NoteOn {
                    pad: ((b0 / kits as u64) % 16) as u8,
                    velocity: 100,
                },
            );
            b0 += 1;
            l.fill(0.0);
            r.fill(0.0);
            e.render(l, r);
        }),
        mem_bytes: 0,
    }
}

/// The bench's stereo sound, `frames` long: a fixed pattern, as 16-bit
/// values when `sixteen` (else values no 16 bits hold, so it stays floats).
fn bench_sound(frames: usize, sixteen: bool) -> Vec<f32> {
    (0..frames * 2)
        .map(|i| {
            let x = ((i * 7919) % 2000) as f32 / 1000.0 - 1.0;
            if sixteen {
                ((x * 32_767.0).round() + 0.0) / 32_768.0
            } else {
                x
            }
        })
        .collect()
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
    for preset in Poly::presets() {
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
    // The supersaw's saws alone, on the plain twin and (on an ARMv7 build
    // with NEON) the NEON kernel: the device round's A/B for mpfree-simd.
    v.push(measured(|| supersaw_saws(rate, mpfree_simd::Path::Plain)));
    if mpfree_simd::neon_available() {
        v.push(measured(|| supersaw_saws(rate, mpfree_simd::Path::Neon)));
    }
    // The Choir's formant filters alone, the same way.
    v.push(measured(|| choir_formants(rate, mpfree_simd::Path::Plain)));
    if mpfree_simd::neon_available() {
        v.push(measured(|| choir_formants(rate, mpfree_simd::Path::Neon)));
    }
    v.push(measured(|| {
        drums("Drums Init Kit, a hit every 4 blocks", rate, "Init Kit", 4)
    }));
    v.push(measured(|| {
        drums("Drums Boom Kit, a hit every block", rate, "Boom Kit", 1)
    }));
    v.push(measured(|| {
        drums_all("Drums Init Kit, every pad each block", rate, "Init Kit")
    }));
    {
        use mpfree_synth::drum_fx::field as f;
        let two = [(f::PUNCH, 40.0), (f::RATIO, 4.0), (f::THRESHOLD, -30.0)];
        let four = [
            (f::PUNCH, 40.0),
            (f::DISTORTION, 18.0),
            (f::LOW, 3.0),
            (f::MID, -4.0),
            (f::HIGH, 3.0),
            (f::RATIO, 4.0),
            (f::THRESHOLD, -30.0),
        ];
        v.push(measured(|| {
            drums_fx("Drums, 16 pads ringing, no effects", rate, &[], false)
        }));
        v.push(measured(|| {
            drums_fx(
                "Drums, 16 pads ringing, Punch and Compressor on each",
                rate,
                &two,
                false,
            )
        }));
        v.push(measured(|| {
            drums_fx(
                "Drums, 16 pads ringing, all four effects on each",
                rate,
                &four,
                false,
            )
        }));
        v.push(measured(|| {
            drums_fx(
                "Drums, 16 pads ringing, all four effects and both sends",
                rate,
                &four,
                true,
            )
        }));
    }
    for (name, voices) in [
        ("Steel String", 8),
        ("Harp", 16),
        ("Marimba", 8),
        ("Bell", 16),
    ] {
        v.push(measured(|| pluck(name, rate, voices)));
    }
    // Keys: a tine and a reed preset, the tremolo and drive on top, and
    // the clav (its string loop and the bus's tone switch and rotator).
    for (name, voices) in [
        ("Init", 8),
        ("Init", 16),
        ("Reed", 16),
        ("Dusty Keys", 16),
        ("Clav", 8),
        ("Clav", 16),
    ] {
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
    // A sound: the Keys engine with its own Ensemble and a macro moving a
    // setting of each (what the wrapper and its effect add to the engine).
    v.push(measured(|| {
        use mpfree_instruments::sound::FX_BASE;
        use mpfree_instruments::{Design, Macro, MacroMap, Sound};
        let design = Design {
            fx: vec!["mpfree.fx.ensemble".into()],
            macros: vec![Macro {
                name: "Space".into(),
                maps: vec![
                    MacroMap {
                        target: mpfree_synth::keys_params::TONE,
                        from: -20.0,
                        to: 20.0,
                        curve: mpfree_mod::MapCurve::Linear,
                    },
                    MacroMap {
                        target: FX_BASE + ids::ensemble::MIX,
                        from: 0.0,
                        to: 60.0,
                        curve: mpfree_mod::MapCurve::Linear,
                    },
                ],
            }],
        };
        let sound = Sound::new(Box::new(Keys::new(rate)), &design).expect("the sound");
        let space = mpfree_instruments::sound::MACRO_BASE;
        chords(
            "Sound Keys Init with Ensemble, 8 voices".to_string(),
            rate,
            Box::new(sound),
            &[(space, 50.0)],
            Some(mpfree_synth::keys_params::POLYPHONY),
            8,
        )
    }));
    // Drawbar: the default three drawbars, all nine with leakage, chorus
    // and the fast rotary, and the rotary off; then the combo model (its
    // divide-down bank renders only the notes heard): three footages with
    // both voices and the vibrato, and the reed on seven footages.
    for (name, keys) in [
        ("Init", 8),
        ("Full Organ", 8),
        ("Gospel", 16),
        ("Church", 8),
        ("Combo", 8),
        ("Combo Reed", 8),
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
    // Strings: the bank with one key, a chord and every key a big chord
    // can hold (it renders only the notes heard, so its cost grows with
    // them, S5), and the orchestra's five registers.
    for (name, keys) in [("Init", 1), ("Init", 6), ("Init", 49), ("Orchestra", 6)] {
        v.push(measured(|| {
            let p = Strings::presets()
                .iter()
                .find(|p| p.name == name)
                .unwrap_or_else(|| panic!("strings preset {name}"));
            chords(
                format!("Strings {name}, {keys} keys"),
                rate,
                Box::new(Strings::new(rate)),
                p.values,
                None,
                keys,
            )
        }));
    }
    // Choir: one note and a six-note chord of three singers each (O10),
    // and the big choir's eight notes of eight singers.
    for (name, keys) in [("Init", 1), ("Init", 6), ("Big Choir", 8)] {
        v.push(measured(|| {
            let p = Choir::presets()
                .iter()
                .find(|p| p.name == name)
                .unwrap_or_else(|| panic!("choir preset {name}"));
            chords(
                format!("Choir {name}, {keys} keys"),
                rate,
                Box::new(Choir::new(rate)),
                p.values,
                Some(mpfree_synth::choir_params::VOICES),
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
    // Bass: the synth bass through its ladder, the reese's pair over the
    // sub, and the 808 driven hard.
    for name in ["Init", "Reese", "Distorted 808"] {
        v.push(measured(|| {
            let p = Bass::presets()
                .iter()
                .find(|p| p.name == name)
                .unwrap_or_else(|| panic!("bass preset {name}"));
            line(
                format!("Bass {name}, a line"),
                rate,
                Box::new(Bass::new(rate)),
                p.values,
            )
        }));
    }
    // Wind: each bore playing a line (the clarinet's Init, the sax with its
    // growl and noise, the flute with vibrato), and the sax section's
    // chords on six voices.
    for name in ["Init", "Growl Sax", "Concert Flute"] {
        v.push(measured(|| {
            let p = Wind::presets()
                .iter()
                .find(|p| p.name == name)
                .unwrap_or_else(|| panic!("wind preset {name}"));
            line(
                format!("Wind {name}, a line"),
                rate,
                Box::new(Wind::new(rate)),
                p.values,
            )
        }));
    }
    v.push(measured(|| {
        let p = Wind::presets()
            .iter()
            .find(|p| p.name == "Reed Section")
            .expect("wind preset Reed Section");
        chords(
            "Wind Reed Section, 6 voices".to_string(),
            rate,
            Box::new(Wind::new(rate)),
            p.values,
            Some(mpfree_synth::wind_params::VOICES),
            6,
        )
    }));
    // Analog: the plain init, the pad with the chorus, and the brass on
    // the two-pole; Worn Cards adds Age's drift on every oscillator.
    for (name, voices) in [
        ("Init", 8),
        ("Warm Pad", 8),
        ("Poly Brass", 8),
        ("Worn Cards", 16),
    ] {
        v.push(measured(|| {
            let p = Analog::presets()
                .iter()
                .find(|p| p.name == name)
                .unwrap_or_else(|| panic!("analog preset {name}"));
            chords(
                format!("Analog {name}, {voices} voices"),
                rate,
                Box::new(Analog::new(rate)),
                p.values,
                Some(mpfree_synth::analog_params::POLYPHONY),
                voices,
            )
        }));
    }
    // Wavetable: the first case also holds the factory tables (built once
    // per process, shared by every instance).
    for preset in Wavetable::presets() {
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
    // Poly's Table wave, as the Wavetable cases above play it: both
    // oscillators on the Basic table.
    for (name, unison, voices) in [
        ("Poly Table, 8 voices, both osc, unison 4", 4.0, 8),
        ("Poly Table, 16 voices, both osc, unison 8", 8.0, 16),
    ] {
        v.push(measured(|| {
            synth(
                name.to_string(),
                rate,
                &[
                    (OSC1_WAVE, WAVE_TABLE as f32),
                    (OSC2_WAVE, WAVE_TABLE as f32),
                    (OSC1_LEVEL, 100.0),
                    (OSC2_LEVEL, 60.0),
                    (UNISON, unison),
                ],
                voices,
            )
        }));
    }
    // The voices the DSP quality plan's first limits are judged on (step
    // 0.4, docs/dev/design/dsp-quality.md): hard sync on Poly's Table wave
    // and on Wavetable (step 1.1, against "Poly Table Init" and "Wavetable
    // Init"), and Poly's per-voice drive and distortion (step 2.1, against
    // "Poly Init"), each at 8 voices.
    v.push(measured(|| {
        synth(
            "Poly Table, 8 voices, sync 50".to_string(),
            rate,
            &[
                (OSC1_WAVE, WAVE_TABLE as f32),
                (OSC1_POS, 100.0),
                (OSC1_WARP_MODE, 2.0),
                (OSC1_WARP, 50.0),
            ],
            8,
        )
    }));
    v.push(measured(|| {
        chords(
            "Wavetable Init, 8 voices, sync 50".to_string(),
            rate,
            Box::new(Wavetable::new(rate)),
            &[
                (wt::osc_id(0, wt::POSITION), 100.0),
                (wt::osc_id(0, wt::WARP_MODE), 2.0),
                (wt::osc_id(0, wt::WARP), 50.0),
            ],
            Some(wt::POLYPHONY),
            8,
        )
    }));
    for (name, id) in [("drive", DRIVE), ("distortion", DIST)] {
        v.push(measured(|| {
            synth(
                format!("Poly Init, 8 voices, {name} 24 dB"),
                rate,
                &[(id, 24.0)],
                8,
            )
        }));
    }
    // FM: a six-operator keys sound, a brass with feedback 7, and a lead
    // whose operators play the triangle, saw and square (the waves' loop).
    for (name, voices) in [("Glass Keys", 8), ("Brass Section", 16), ("Reed Lead", 8)] {
        let p = Fm::presets().iter().find(|p| p.name == name).expect(name);
        v.push(measured(|| {
            chords(
                format!("FM {name}, {voices} voices"),
                rate,
                Box::new(Fm::new(rate)),
                p.values,
                Some(mpfree_synth::fm_params::POLYPHONY),
                voices,
            )
        }));
    }
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
        drum_kit("Sampler, drum kit, 24 voices", rate, 24, false)
    }));
    v.push(measured(|| {
        drum_kit("Sampler, drum kit, 64 voices", rate, 64, false)
    }));
    v.push(measured(|| {
        drum_kits("Sampler, 4 drum kits, 64 voices", rate, 64, 4)
    }));
    v.push(measured(|| {
        drum_kit(
            "Sampler, drum kit, 24 voices, 16-bit sounds",
            rate,
            24,
            true,
        )
    }));
    v.push(measured(|| held_pads(rate, 16, false, false)));
    v.push(measured(|| held_pads(rate, 16, false, true)));
    v.push(measured(|| held_pads(rate, 16, true, false)));
    v.push(measured(|| held_pads(rate, 16, true, true)));
    v.push(measured(|| moving_pads(rate, 16)));
    v.push(measured(|| {
        new_filter_pads(rate, 16, FilterKind::Ladder, 12.0)
    }));
    v.push(measured(|| {
        new_filter_pads(rate, 16, FilterKind::Vowel, 0.0)
    }));
    v.push(measured(|| {
        new_filter_pads(rate, 16, FilterKind::Comb, 0.0)
    }));
    v.push(measured(|| {
        sampler(
            "Sampler, the 4 granular pads alone",
            rate,
            false,
            SamplerPads::Grains,
        )
    }));
    v.extend(grain_worst_cases(rate));
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
            "Multi filter (ladder)",
            rate,
            Box::new(MultiFilter::new()),
            &[
                (ids::filter::MODEL, 1.0),
                (ids::filter::CUTOFF, 800.0),
                (ids::filter::RESONANCE, 70.0),
            ],
        )
    }));
    v.push(measured(|| {
        effect(
            "Multi filter (LP 12, Hard)",
            rate,
            Box::new(MultiFilter::new()),
            &[
                (ids::filter::CUTOFF, 800.0),
                (ids::filter::RESONANCE, 70.0),
                (ids::filter::RES_MODE, 1.0),
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
    // The amp at its defaults (Crunch, Gain 5) and pushed (Lead, Gain 10).
    v.push(measured(|| {
        effect("Amp (Crunch)", rate, Box::new(Amp::new()), &[])
    }));
    v.push(measured(|| {
        effect(
            "Amp (Lead, Gain 10)",
            rate,
            Box::new(Amp::new()),
            &[(ids::amp::MODEL, 2.0), (ids::amp::GAIN, 10.0)],
        )
    }));
    v.push(measured(|| {
        effect("Fuzz", rate, Box::new(Fuzz::new()), &[])
    }));
    v.push(measured(|| {
        effect("Delay", rate, Box::new(Delay::new()), &[])
    }));
    v.push(measured(|| {
        effect("Tape delay", rate, Box::new(TapeDelay::new()), &[])
    }));
    v.push(measured(|| {
        effect("Multi-tap delay", rate, Box::new(MultiTap::new()), &[])
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
        effect(
            "EQ (4 bands and both cuts), HQ 2x",
            rate,
            Box::new(Eq::new()),
            &[
                (ids::eq::QUALITY, ids::eq::HQ),
                (ids::eq::LOW_SLOPE, 2.0),
                (ids::eq::HIGH_SLOPE, 2.0),
                (ids::eq::gain(0), 3.0),
                (ids::eq::gain(1), -4.0),
                (ids::eq::gain(2), 5.0),
                (ids::eq::gain(3), 2.0),
            ],
        )
    }));
    v.push(measured(|| eq_gliding(rate)));
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
        effect("Ensemble (3 lines)", rate, Box::new(Ensemble::new()), &[])
    }));
    v.push(measured(|| {
        effect(
            "Ensemble (4 lines)",
            rate,
            Box::new(Ensemble::new()),
            &[(ids::ensemble::LINES, 1.0)],
        )
    }));
    v.push(measured(|| {
        effect("Flanger", rate, Box::new(Flanger::new()), &[])
    }));
    v.push(measured(|| {
        effect(
            "Flanger, through zero",
            rate,
            Box::new(Flanger::new()),
            &[(ids::flanger::THROUGH_ZERO, 1.0)],
        )
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
    // The rest of the dynamics (FX quality round 4), each set to work on
    // the program: the limiter pushed 12 dB into its ceiling, the gate's
    // threshold over the tone so it opens on each burst and shuts after,
    // the ducker on the tempo.
    for (name, tp) in [("Limiter", 0.0), ("Limiter (true peak)", 1.0)] {
        v.push(measured(|| {
            effect(
                name,
                rate,
                Box::new(Limiter::new()),
                &[(ids::limiter::GAIN, 12.0), (ids::limiter::TRUE_PEAK, tp)],
            )
        }));
    }
    v.push(measured(|| {
        effect(
            "Gate",
            rate,
            Box::new(Gate::new()),
            &[(ids::gate::THRESHOLD, -10.0)],
        )
    }));
    v.push(measured(|| {
        effect("De-esser", rate, Box::new(DeEsser::new()), &[])
    }));
    v.push(measured(|| {
        effect(
            "Multiband compressor",
            rate,
            Box::new(MultibandCompressor::new()),
            &[],
        )
    }));
    v.push(measured(|| {
        effect(
            "Ducker (tempo)",
            rate,
            Box::new(Ducker::new()),
            &[(ids::ducker::SOURCE, 1.0)],
        )
    }));
    v.push(measured(|| {
        effect(
            "Transient shaper",
            rate,
            Box::new(TransientShaper::new()),
            &[],
        )
    }));
    // The movement, pitch and voice effects (FX quality round 5), each at
    // its defaults.
    v.push(measured(|| {
        effect("Tremolo", rate, Box::new(Tremolo::new()), &[])
    }));
    v.push(measured(|| {
        effect("Auto-pan", rate, Box::new(AutoPan::new()), &[])
    }));
    v.push(measured(|| {
        effect("Rotary", rate, Box::new(Rotary::new()), &[])
    }));
    v.push(measured(|| {
        effect("Ring mod", rate, Box::new(RingMod::new()), &[])
    }));
    v.push(measured(|| {
        effect("Frequency shifter", rate, Box::new(FreqShifter::new()), &[])
    }));
    v.push(measured(|| {
        effect("Auto wah", rate, Box::new(AutoWah::new()), &[])
    }));
    v.push(measured(|| {
        effect("Pitch shifter", rate, Box::new(PitchShifter::new()), &[])
    }));
    v.push(measured(|| {
        effect("Pitch correct", rate, Box::new(PitchCorrect::new()), &[])
    }));
    v.push(measured(|| {
        effect("Harmonizer", rate, Box::new(Harmonizer::new()), &[])
    }));
    v.push(measured(|| {
        effect("Tuner", rate, Box::new(Tuner::new()), &[])
    }));
    v.push(measured(|| {
        effect("Talk box", rate, Box::new(TalkBox::new()), &[])
    }));
    // The vocoder at its defaults: 16 bands, the saw following the
    // voice's pitch (the tracker runs).
    v.push(measured(|| {
        effect("Vocoder", rate, Box::new(Vocoder::new()), &[])
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
    // The Room, Plate and Spring at their defaults, all wet.
    v.push(measured(|| {
        effect(
            "Room reverb",
            rate,
            Box::new(Room::new()),
            &[(ids::room::MIX, 100.0)],
        )
    }));
    v.push(measured(|| {
        effect(
            "Plate reverb",
            rate,
            Box::new(Plate::new()),
            &[(ids::plate::MIX, 100.0)],
        )
    }));
    v.push(measured(|| {
        effect(
            "Spring reverb",
            rate,
            Box::new(Spring::new()),
            &[(ids::spring::MIX, 100.0)],
        )
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
    // The same with a band pass on each grain, sprayed (step C4).
    v.push(measured(|| {
        effect(
            "Grains FX (dense, filtered)",
            rate,
            Box::new(GrainNode::new(5)),
            &[
                (mpfree_grain::node::param::MIX, 100.0),
                (mpfree_grain::node::param::DENSITY, 200.0),
                (mpfree_grain::node::param::SPRAY, 50.0),
                (mpfree_grain::node::param::FILTER, 2.0),
                (mpfree_grain::node::param::RESONANCE, 50.0),
                (mpfree_grain::node::param::FILTER_SPRAY, 50.0),
            ],
        )
    }));
    // The same cloud in Tape and in Lo-fi (step C5).
    for (name, q) in [
        ("Grains FX (dense, tape)", 1.0),
        ("Grains FX (dense, lo-fi)", 2.0),
    ] {
        v.push(measured(|| {
            effect(
                name,
                rate,
                Box::new(GrainNode::new(5)),
                &[
                    (mpfree_grain::node::param::MIX, 100.0),
                    (mpfree_grain::node::param::DENSITY, 200.0),
                    (mpfree_grain::node::param::SPRAY, 50.0),
                    (mpfree_grain::node::param::QUALITY, q),
                ],
            )
        }));
    }
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
    v.push(measured(|| {
        pad_synths(
            "Pad synths: 16 Instrument pads of Poly, a hit each per 0.5 s",
            rate,
            16,
            vec![],
            Duration::ZERO,
        )
    }));
    v.push(measured(|| time(rate, 8, false)));
    v.push(measured(|| time(rate, 8, true)));
    v.push(measured(|| time_ahead(rate, 8, 4)));
    v.push(measured(|| time_record(rate, 8, 3)));
    v.push(measured(|| time_dense(rate)));
    v.push(measured(|| step_edit(StepTouch::Tap, false)));
    v.push(measured(|| step_edit(StepTouch::Tap, true)));
    v.push(measured(|| step_edit(StepTouch::Detent, false)));
    v.push(measured(|| step_edit(StepTouch::Detent, true)));
    v.push(measured(step_grid));
    v.push(arrange_compile());
    v.push(measured(arrange_step));
    v.push(measured(arrange_save));
    v.push(measured(song_view_frame));
    for what in [
        RollFrame::Built,
        RollFrame::BuiltPressure,
        RollFrame::Plays,
        RollFrame::Scrolled,
        RollFrame::Zoomed,
        RollFrame::Switched,
    ] {
        v.push(measured(|| roll_frame(what)));
    }
    v.push(measured(|| note_edit(NoteEditCase::VelocityAndUndo)));
    v.push(measured(|| note_edit(NoteEditCase::TransposeAll)));
    v.push(measured(|| live_arps(rate, 4)));
    v.push(measured(|| live_arps(rate, 16)));
    v.push(measured(|| looper(rate, 60)));
    v.push(measured(|| looper_loops(rate, 60, false)));
    v.push(measured(|| looper_loops(rate, 60, true)));
    v.push(measured(|| audio_tracks(rate, 0, false)));
    v.push(measured(|| audio_tracks(rate, 2, false)));
    v.push(measured(|| audio_tracks(rate, 2, true)));
    v.push(measured(|| audio_clips(rate)));
    v.push(measured(|| stream_voices(rate, 64, Read::Hermite, 1.5)));
    v.push(measured(|| stream_voices(rate, 64, Read::Sinc8, 1.5)));
    v.push(measured(|| stream_voices(rate, 64, Read::Sinc8, 3.0)));
    v.push(measured(|| stream_voices(rate, 64, Read::Sinc16, 1.5)));
    v.push(measured(|| stream_voices(rate, 16, Read::Sinc8, 1.0)));
    v.push(measured(|| song(rate)));
    // The DSP quality plan's fixed test projects (step 0.6), each through
    // the whole chain.
    for name in crate::projects::ALL {
        v.push(measured(|| crate::projects::case(name, rate)));
    }
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

/// One synth preset at 1 to 16 voices of `chords` (or keys held, for an
/// organ: `polyphony` None), or once on `line` when it plays one note at a
/// time.
fn preset_scaling(
    name: String,
    rate: u32,
    make: impl Fn() -> Box<dyn Node>,
    values: &[(u16, f32)],
    polyphony: Option<u16>,
    mono: bool,
) -> Scaling {
    if mono {
        return Scaling {
            name: format!("{name} (mono)"),
            module: "synth",
            points: vec![(1, line(format!("{name}, a line"), rate, make(), values))],
        };
    }
    Scaling {
        points: [1usize, 2, 4, 8, 16]
            .iter()
            .map(|&n| {
                let case = chords(format!("{name} x{n}"), rate, make(), values, polyphony, n);
                (n, case)
            })
            .collect(),
        name,
        module: "synth",
    }
}

/// One factory preset of a keyboard engine, ready to build: for
/// `--afford` and `--starts`.
pub struct Preset {
    /// The engine and the preset, such as "Keys Phase Tines".
    pub name: String,
    pub make: Box<dyn Fn() -> Box<dyn Node>>,
    pub values: Vec<(u16, f32)>,
    /// The engine's voice count parameter; None for an organ or a string
    /// machine (every key held sounds) and for a mono engine.
    pub polyphony: Option<u16>,
    /// It plays one note at a time.
    pub mono: bool,
}

impl Preset {
    fn new<N: Node + 'static>(
        name: String,
        rate: u32,
        make: fn(u32) -> N,
        values: &[(u16, f32)],
        polyphony: Option<u16>,
        mono: bool,
    ) -> Preset {
        Preset {
            name,
            make: Box::new(move || Box::new(make(rate))),
            values: values.to_vec(),
            polyphony,
            mono,
        }
    }
}

/// Every factory preset of Poly (a mono or legato one plays one voice).
fn poly_presets(rate: u32) -> Vec<Preset> {
    Poly::presets()
        .iter()
        .map(|p| {
            let mono = p.values.iter().any(|&(id, x)| id == VOICE_MODE && x != 0.0);
            Preset::new(
                format!("Poly {}", p.name),
                rate,
                Poly::new,
                p.values,
                Some(POLYPHONY),
                mono,
            )
        })
        .collect()
}

/// Every factory preset of every keyboard engine but Poly.
fn engine_presets(rate: u32) -> Vec<Preset> {
    use mpfree_synth::{analog_params, fm_params, keys_params, pluck_params, Fm};
    let mode = |values: &[(u16, f32)], id: u16| values.iter().any(|&(i, x)| i == id && x != 0.0);
    let mut v = Vec::new();
    for p in Analog::presets() {
        // Unison stacks every voice on one note: still a cost per voice.
        let name = format!("Analog {}", p.name);
        v.push(Preset::new(
            name,
            rate,
            Analog::new,
            p.values,
            Some(analog_params::POLYPHONY),
            false,
        ));
    }
    for p in Squelch::presets() {
        v.push(Preset::new(
            format!("Squelch {}", p.name),
            rate,
            Squelch::new,
            p.values,
            None,
            true,
        ));
    }
    for p in Bass::presets() {
        v.push(Preset::new(
            format!("Bass {}", p.name),
            rate,
            Bass::new,
            p.values,
            None,
            true,
        ));
    }
    for p in Wind::presets() {
        use mpfree_synth::wind_params::VOICES;
        let mono = !p.values.iter().any(|&(id, x)| id == VOICES && x > 1.0);
        v.push(Preset::new(
            format!("Wind {}", p.name),
            rate,
            Wind::new,
            p.values,
            Some(VOICES),
            mono,
        ));
    }
    for p in Fm::presets() {
        let mono = mode(p.values, fm_params::VOICE_MODE);
        let name = format!("FM {}", p.name);
        v.push(Preset::new(
            name,
            rate,
            Fm::new,
            p.values,
            Some(fm_params::POLYPHONY),
            mono,
        ));
    }
    for p in Wavetable::presets() {
        let mono = mode(p.values, wt::VOICE_MODE);
        let name = format!("Wavetable {}", p.name);
        v.push(Preset::new(
            name,
            rate,
            Wavetable::new,
            p.values,
            Some(wt::POLYPHONY),
            mono,
        ));
    }
    for p in Keys::presets() {
        let name = format!("Keys {}", p.name);
        v.push(Preset::new(
            name,
            rate,
            Keys::new,
            p.values,
            Some(keys_params::POLYPHONY),
            false,
        ));
    }
    for p in Drawbar::presets() {
        v.push(Preset::new(
            format!("Drawbar {}", p.name),
            rate,
            Drawbar::new,
            p.values,
            None,
            false,
        ));
    }
    for p in Strings::presets() {
        v.push(Preset::new(
            format!("Strings {}", p.name),
            rate,
            Strings::new,
            p.values,
            None,
            false,
        ));
    }
    for p in Pluck::presets() {
        let name = format!("Pluck {}", p.name);
        v.push(Preset::new(
            name,
            rate,
            Pluck::new,
            p.values,
            Some(pluck_params::POLYPHONY),
            false,
        ));
    }
    for p in Choir::presets() {
        let mono = mode(p.values, mpfree_synth::choir_params::MODE);
        v.push(Preset::new(
            format!("Choir {}", p.name),
            rate,
            Choir::new,
            p.values,
            Some(mpfree_synth::choir_params::VOICES),
            mono,
        ));
    }
    v
}

/// Every factory preset of every keyboard engine, Poly first.
pub fn keyboard_presets(rate: u32) -> Vec<Preset> {
    let mut v = poly_presets(rate);
    v.extend(engine_presets(rate));
    v
}

/// Notes started in one block by `--starts`: Poly's most voices.
pub const START_NOTES: usize = 16;

/// What starting [`START_NOTES`] notes in one block costs a preset
/// (`--starts`), each time from silence. Times in ns.
#[derive(Clone, Debug, Default)]
pub struct StartCost {
    /// The block the notes start in (the median over the rounds).
    pub start_ns: f64,
    /// The worst of those blocks.
    pub worst_start_ns: f64,
    /// The next three blocks, the notes sounding (the median of their
    /// means): what the notes cost once started.
    pub held_ns: f64,
    /// The block after those, where the chord changes: every note ends and
    /// as many start a fifth higher, so the new notes take voices still
    /// sounding their release, or steal them (the median).
    pub change_ns: f64,
}

/// [`StartCost`]: `rounds` times, the preset is silenced ([`Node::reset`],
/// which keeps its memory warm), plays two blocks of silence, then starts
/// [`START_NOTES`] notes on the first frame of a block (a chord over three
/// octaves, velocity 100); that block, the three after it and the chord
/// change after those are timed.
pub fn start_cost(p: &Preset, rate: u32, frames: usize, rounds: usize) -> StartCost {
    let mut s = (p.make)();
    s.prepare(rate, MAX_BLOCK);
    for &(id, v) in &p.values {
        s.set_param(id, v);
    }
    if let Some(id) = p.polyphony {
        s.set_param(id, START_NOTES as f32);
    }
    let chord = |root: u8| {
        (0..START_NOTES)
            .map(move |k| root + [0u8, 7, 12, 16, 19, 24, 28, 31][k % 8] + 12 * (k / 8) as u8)
    };
    let on = |note| {
        Timed::new(
            0,
            NodeEvent::NoteOn {
                note,
                velocity: 100,
            },
        )
    };
    let notes: Vec<Timed> = chord(48).map(on).collect();
    let change: Vec<Timed> = chord(48)
        .map(|note| Timed::new(0, NodeEvent::NoteOff { note }))
        .chain(chord(55).map(on))
        .collect();
    let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    let mut frame = 0u64;
    let mut block = |s: &mut Box<dyn Node>, events: &[Timed]| {
        l.fill(0.0);
        r.fill(0.0);
        let t = Instant::now();
        s.process(&Ctx::new(rate, frame), events, &mut l, &mut r);
        let ns = t.elapsed().as_nanos() as f64;
        std::hint::black_box((&l, &r));
        frame += frames as u64;
        ns
    };
    let (mut starts, mut held, mut changes) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..rounds {
        s.reset();
        for _ in 0..2 {
            block(&mut s, &[]);
        }
        starts.push(block(&mut s, &notes));
        held.push((0..3).map(|_| block(&mut s, &[])).sum::<f64>() / 3.0);
        changes.push(block(&mut s, &change));
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(0.0)
    };
    StartCost {
        worst_start_ns: starts.iter().copied().fold(0.0, f64::max),
        start_ns: median(&mut starts),
        held_ns: median(&mut held),
        change_ns: median(&mut changes),
    }
}

/// Every factory preset of every keyboard engine but Poly at 1 to 16
/// voices (keys held, for the organ), the acid line once (it is mono).
fn engine_scaling(rate: u32) -> Vec<Scaling> {
    engine_presets(rate)
        .into_iter()
        .map(|p| {
            let held = p.polyphony.is_none() && !p.mono;
            preset_scaling(
                format!("{}{}", p.name, if held { " (keys held)" } else { "" }),
                rate,
                &*p.make,
                &p.values,
                p.polyphony,
                p.mono,
            )
        })
        .collect()
}

/// Every synth preset of every keyboard engine at 1 to 16 voices (chords
/// changing every half second, as in the synth cases; a mono preset once,
/// on a line), the sampler at 1 to 32 voices held, and the sampler's
/// grains at 16 to 256 alive.
pub fn scaling(rate: u32) -> Vec<Scaling> {
    let mut v: Vec<Scaling> = Poly::presets()
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
    v.extend(engine_scaling(rate));
    v.push(Scaling {
        name: "Sampler, looping pads held".to_string(),
        module: "sampler",
        points: [1usize, 4, 8, 16, 32]
            .iter()
            .map(|&n| (n, held_pads(rate, n, false, false)))
            .collect(),
    });
    v.push(Scaling {
        name: "Sampler, pads held pitched up (clean read)".to_string(),
        module: "sampler",
        points: [1usize, 4, 8, 16, 32]
            .iter()
            .map(|&n| (n, held_pads(rate, n, true, false)))
            .collect(),
    });
    // A pair for the denormals check (#463): the same filtered pads, once
    // playing and once ringing out on silence. Before the sampler flushed
    // its own denormals, the second cost about 40 times the first.
    for ringing in [false, true] {
        v.push(Scaling {
            name: if ringing {
                "Sampler, filtered pads ringing on silence"
            } else {
                "Sampler, filtered pads playing"
            }
            .to_string(),
            module: "sampler",
            points: [1usize, 4, 8, 16, 32]
                .iter()
                .map(|&n| (n, filtered_pads(rate, n, ringing)))
                .collect(),
        });
    }
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
    let name = format!("Sampler grains x{grains}");
    // The top point asks for the whole budget, past the room sharing keeps
    // for a new hit, so it thins.
    let allow = Allow {
        grains: grains + RESERVE > GRAIN_BUDGET,
        ..Allow::default()
    };
    let mut e = Watched::new(grain_engine(rate, grains), &name, allow);
    Case {
        name,
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

/// What a grain worst case turns on (sampler plan step 13).
#[derive(Clone, Copy, Default)]
struct Worst {
    /// A band pass on every grain, its cutoff sprayed (`GrainFilter`).
    filters: bool,
    /// Pitched up 7 semitones, so every grain takes the clean
    /// (band-limited) read.
    clean: bool,
    /// Each grain's pitch sprayed over 12 semitones either way: grains at
    /// many speeds, those up past two semitones on the clean read.
    pitch_spray: bool,
    /// The master Grains FX over everything, dense.
    master: bool,
}

/// How often a grain worst case lets go of one of its pads and hits it
/// again, in blocks (about 0.3 s), so new hits, ringing tails and the room
/// made for them come round all the time.
const WORST_RETRIGGER: u64 = 100;

/// A grain worst case (sampler plan step 13): `pads` granular pads held
/// (two voices each, so a let-go rings out under the next hit), together
/// asking for more grains at once than the engine's budget, so the whole
/// budget plays and the sharing of step 9 works every block; every
/// [`WORST_RETRIGGER`] blocks the next pad is let go and hit again.
/// `w` turns on the costly grain features. Five pads: 100 ms grains, 640 a
/// second, each wanting the 64 a voice keeps. More pads: the 400 ms, 150 a
/// second clouds of the sharing tests.
fn grain_worst(name: &str, rate: u32, pads: usize, w: Worst, lane: Option<LaneConfig>) -> Case {
    let split = lane.is_some();
    let mut e = grain_worst_engine(name, rate, pads, w, lane);
    Case {
        name: name.to_string(),
        module: "sampler",
        what: "granular pads held over the grain budget, one re-hit every 0.3 s",
        block: Box::new(move |b, l, r| {
            grain_worst_hits(&mut e, b, pads);
            if split {
                e.render_until(l, r, Instant::now() + lane_wait());
            } else {
                l.fill(0.0);
                r.fill(0.0);
                e.render(l, r);
            }
        }),
        mem_bytes: 0,
    }
}

/// A [`grain_worst`] case's hits for block `b`: every pad at the start,
/// then the next pad let go and hit again every [`WORST_RETRIGGER`]
/// blocks.
fn grain_worst_hits(e: &mut Engine, b: u64, pads: usize) {
    if b == 0 {
        for pad in 0..pads {
            e.apply(Event::NoteOn {
                pad: pad as u8,
                velocity: 100,
            });
        }
    } else if b.is_multiple_of(WORST_RETRIGGER) {
        let pad = ((b / WORST_RETRIGGER) % pads as u64) as u8;
        e.apply(Event::NoteOff { pad });
        e.apply(Event::NoteOn { pad, velocity: 100 });
    }
}

/// The engine behind a [`grain_worst`] case, checked for its trades as it
/// ends.
fn grain_worst_engine(
    name: &str,
    rate: u32,
    pads: usize,
    w: Worst,
    lane: Option<LaneConfig>,
) -> Watched {
    let s = Source::new("worst", rate, 2, bench_sound(rate as usize * 4, false)).expect("source");
    if w.clean || w.pitch_spray {
        // Made now, so every grain finds them (a long sound's copies are
        // otherwise made on their own thread).
        s.make_octaves();
    }
    let mut p = Project::new();
    p.add_source(s.clone());
    for i in 0..pads {
        let mut r = Region::whole(&s);
        r.mode = PlayMode::Granular;
        let (size_ms, density) = if pads <= 5 {
            (100.0, 640.0)
        } else {
            (400.0, 150.0)
        };
        r.grain = GrainParams {
            position: i as f32 / pads as f32,
            spray: 0.5,
            size_ms,
            density,
            spread: 1.0,
            pitch: if w.clean { 7.0 } else { 0.0 },
            pitch_spray: if w.pitch_spray { 12.0 } else { 0.0 },
            filter: if w.filters {
                mpfree_sampler::model::GrainFilter {
                    kind: mpfree_sampler::model::GrainFilterKind::Band,
                    cutoff_hz: 1_500.0,
                    resonance: 0.5,
                    spray: 1.0,
                }
            } else {
                mpfree_sampler::model::GrainFilter::default()
            },
            ..GrainParams::default()
        };
        let mut pad = Pad::new("g", Layer::new(r));
        pad.polyphony = 2;
        p.set_pad(i, Some(pad), None).expect("pad");
    }
    let mut e = Engine::new(rate, 2 * pads);
    if w.master {
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
    // Crowded by design. Each pad's let-go rings out until its next hit
    // comes round, so the re-hit takes over no voice.
    let allow = Allow {
        grains: true,
        late: lane.as_ref().is_some_and(|c| c.threaded),
        ..Allow::default()
    };
    if let Some(config) = lane {
        e.split(config);
    }
    Watched::new(e, name, allow)
}

/// The grain worst cases on one core, for every bench run, the
/// instruction gate and the golden fingerprints (sampler plan step 13).
fn grain_worst_cases(rate: u32) -> Vec<Case> {
    let all = Worst {
        filters: true,
        clean: true,
        pitch_spray: true,
        master: true,
    };
    [
        ("Grains worst: 5 pads", Worst::default(), 5),
        (
            "Grains worst: 5 pads, grain filters",
            Worst {
                filters: true,
                ..Worst::default()
            },
            5,
        ),
        (
            "Grains worst: 5 pads, clean read",
            Worst {
                clean: true,
                ..Worst::default()
            },
            5,
        ),
        (
            "Grains worst: 5 pads, pitch spray",
            Worst {
                pitch_spray: true,
                ..Worst::default()
            },
            5,
        ),
        (
            "Grains worst: 5 pads + Grains FX",
            Worst {
                master: true,
                ..Worst::default()
            },
            5,
        ),
        ("Grains worst: 5 pads, everything", all, 5),
        ("Grains worst: 64 clouds sharing", Worst::default(), 64),
    ]
    .into_iter()
    .map(|(name, w, pads)| measured(|| grain_worst(name, rate, pads, w, None)))
    .collect()
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
/// `n` voices sound in every block. Tuned from an octave down to just under
/// an octave up, half of them at or below their pitch; `up` tunes them all
/// from 3 to 14.5 semitones up instead, so every voice takes the clean
/// (band-limited) read, the ones past an octave from the sound's octave
/// copy. `sixteen`: the sound is 16-bit, held and read as 16 bits.
fn held_pads(rate: u32, n: usize, up: bool, sixteen: bool) -> Case {
    let s = Source::new("noise", rate, 2, bench_sound(rate as usize, sixteen)).expect("source");
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
        r.tune_cents = if up {
            300 + (i as i32 % 24) * 50
        } else {
            (i as i32 % 24 - 12) * 50
        };
        let mut pad = Pad::new("p", Layer::new(r));
        pad.polyphony = 1;
        pad.filter = Filter {
            kind: FilterKind::LowPass,
            poles: 2,
            cutoff_hz: 2000.0,
            resonance: 0.3,
            env_octaves: 1.0,
            drive_db: 0.0,
        };
        p.set_pad(i, Some(pad), None).expect("pad");
    }
    if up {
        // Made now, so every voice finds them (a long sound's copies are
        // otherwise made on their own thread).
        s.make_octaves();
    }
    let mut e = Engine::new(rate, n.max(1));
    e.set_view(EngineView::build(&p, rate));
    let name = format!(
        "Sampler x{n} held{}{}",
        if up { " up" } else { "" },
        if sixteen { ", 16-bit sound" } else { "" }
    );
    let mut e = Watched::new(e, &name, Allow::default());
    Case {
        name,
        module: "sampler",
        what: if up {
            "looping pads held down, pitched up"
        } else {
            "looping pads held down"
        },
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

/// The sampler with `n` looping pads held down, as [`held_pads`], each
/// moving (sampler R5 part 4): half with an LFO on the pitch, half on the
/// cutoff (the dearer: the filter is set again every 16 frames), every one
/// with a pitch envelope that starts an octave up and comes back over 2 s,
/// so the blocks counted see it fall.
fn moving_pads(rate: u32, n: usize) -> Case {
    let s = Source::new("noise", rate, 2, bench_sound(rate as usize, false)).expect("source");
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
            drive_db: 0.0,
        };
        pad.lfo = PadLfo {
            shape: LfoShape::Triangle,
            rate_hz: 3.0 + i as f32 * 0.25,
            target: if i % 2 == 0 {
                LfoTarget::Pitch
            } else {
                LfoTarget::Cutoff
            },
            depth: 0.2,
            ..PadLfo::default()
        };
        pad.pitch_env = PitchEnv {
            semitones: 12.0,
            decay_ms: 2000.0,
        };
        p.set_pad(i, Some(pad), None).expect("pad");
    }
    let mut e = Engine::new(rate, n.max(1));
    e.set_view(EngineView::build(&p, rate));
    let name = format!("Sampler x{n} held, LFO and pitch envelope");
    let mut e = Watched::new(e, &name, Allow::default());
    Case {
        name,
        module: "sampler",
        what: "looping pads held down, each with an LFO and a pitch envelope",
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

/// The sampler with `n` looping pads held down, as [`held_pads`], through
/// one of sampler R5.6's filters with its envelope moving the cutoff (set
/// again every 16 frames): the Ladder (with `drive_db` of Drive, its curve
/// at twice the rate), the Vowel (two formants) or the Comb (its delay
/// gliding to each new pitch).
fn new_filter_pads(rate: u32, n: usize, kind: FilterKind, drive_db: f32) -> Case {
    let s = Source::new("noise", rate, 2, bench_sound(rate as usize, false)).expect("source");
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
            kind,
            poles: 4,
            cutoff_hz: 1000.0,
            resonance: 0.5,
            env_octaves: 2.0,
            drive_db,
        };
        pad.filter_env.decay_ms = 2000.0;
        pad.filter_env.sustain = 0.0;
        p.set_pad(i, Some(pad), None).expect("pad");
    }
    let mut e = Engine::new(rate, n.max(1));
    e.set_view(EngineView::build(&p, rate));
    let (name, what) = match kind {
        FilterKind::Ladder => (
            format!("Sampler x{n} held, driven ladder"),
            "looping pads held down through a ladder driven 12 dB, its envelope moving",
        ),
        FilterKind::Comb => (
            format!("Sampler x{n} held, comb"),
            "looping pads held down through a comb with feedback, its envelope moving its pitch",
        ),
        _ => (
            format!("Sampler x{n} held, vowel"),
            "looping pads held down through a vowel filter, its envelope moving",
        ),
    };
    let mut e = Watched::new(e, &name, Allow::default());
    Case {
        name,
        module: "sampler",
        what,
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

/// The sampler with `n` pads held down through a resonant 4-pole low-pass
/// at 1 kHz, each looping noise; `ringing` loops them over silence
/// instead, after the last 50 ms of the noise, so every filter rings out
/// on zeros: what costs most where a filter's state decays into denormals.
/// The pair is measured on one build, so it needs no other to compare with.
fn filtered_pads(rate: u32, n: usize, ringing: bool) -> Case {
    let len = rate as usize;
    let mut data = vec![0.0f32; len * 2 * 8];
    for (i, x) in data[..len * 2].iter_mut().enumerate() {
        *x = ((i * 7919) % 2000) as f32 / 1000.0 - 1.0;
    }
    let s = Source::new("noise then silence", rate, 2, data).expect("source");
    let mut p = Project::new();
    p.add_source(s.clone());
    let at = if ringing { len as u64 } else { 0 };
    for i in 0..n {
        let mut r = Region::whole(&s);
        r.mode = PlayMode::Loop;
        if ringing {
            r.start = at - 2205;
        }
        r.sustain_loop = Some(Loop {
            start: at + 5000,
            end: at + 30000,
            crossfade: 0,
            mode: LoopMode::Forward,
        });
        let mut pad = Pad::new("f", Layer::new(r));
        pad.polyphony = 1;
        pad.filter = Filter {
            kind: FilterKind::LowPass,
            poles: 4,
            cutoff_hz: 1000.0,
            resonance: 0.5,
            env_octaves: 0.0,
            drive_db: 0.0,
        };
        p.set_pad(i, Some(pad), None).expect("pad");
    }
    let mut e = Engine::new(rate, n.max(1));
    e.set_view(EngineView::build(&p, rate));
    let name = format!(
        "Sampler x{n} {}",
        if ringing { "ringing" } else { "filtered" }
    );
    let mut e = Watched::new(e, &name, Allow::default());
    Case {
        name,
        module: "sampler",
        what: if ringing {
            "filtered pads held, ringing out on silence"
        } else {
            "filtered pads held, playing"
        },
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
        Poly::presets()
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
/// Eight voices' supersaws without the rest of Poly: each voice's seven
/// saws read one band-limited saw level, once a voice per 16-frame step,
/// as Poly calls them (`mpfree_simd::workload::Supersaw`), on `path`. The
/// plain and NEON cases give the same samples.
fn supersaw_saws(rate: u32, path: mpfree_simd::Path) -> Case {
    let mut saws = mpfree_simd::workload::Supersaw::new(8, rate);
    Case {
        name: format!("Supersaw saws: 8 voices, {}", path.name()),
        module: "synth",
        what: "7 saws a voice from one table (mpfree-simd)",
        block: Box::new(move |_, l, r| {
            saws.block(path, l, r);
        }),
        mem_bytes: 0,
    }
}

/// Eight Choir voices' formants: two groups of four voices, each voice a
/// buzz through its vowel's five formant filters, once a group per
/// 16-frame step, as the Choir calls them
/// (`mpfree_simd::workload::Formants`), on `path`. The plain and NEON
/// cases give the same samples.
fn choir_formants(rate: u32, path: mpfree_simd::Path) -> Case {
    let mut voices = mpfree_simd::workload::Formants::new(8, rate);
    Case {
        name: format!("Choir formants: 8 voices, {}", path.name()),
        module: "synth",
        what: "5 formant filters a voice, 4 voices a vector (mpfree-simd)",
        block: Box::new(move |_, l, r| {
            voices.block(path, l, r);
        }),
        mem_bytes: 0,
    }
}

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

/// The Time cases' song: `tracks` clips of 16ths with 3-note chords and
/// two locks a step, one at 3:2 speed, one swung and quantised, with
/// `trigs` a trig condition on every note and a ratchet every fourth step.
fn time_song(tracks: usize, trigs: bool) -> (Song, Vec<Launch>) {
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
                    on: true,
                    ramp: 0,
                    join: mpfree_seq::Join::Attack,
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
    (song, launches)
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
    let ppq = i64::from(PPQ);
    let (song, launches) = time_song(tracks, trigs);
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

/// Tracks on MIDI gear slower than the sound's lead (sequencer S5's
/// look-ahead, `mpfree_play::Ahead`): the Time case's song without arps,
/// `ahead` of its `tracks` clips 4,000 frames ahead (100 ms of gear delay
/// less the 8.7 ms lead, at 44.1 kHz). Each block plays the real period,
/// checks its events against the log, then copies the transport and the
/// player and plays them on 4,000 frames to send what comes next.
fn time_ahead(rate: u32, tracks: usize, ahead: usize) -> Case {
    let ppq = i64::from(PPQ);
    let (song, launches) = time_song(tracks, false);
    let view = PlayView::compile(&song, &launches).expect("view");
    let mut t = Transport::new(rate, 128.0).expect("transport");
    t.command(0, T::SetLoop(Some((0, 16 * ppq))));
    t.command(0, T::Play);
    let mut player = Player::new();
    let mut block = TBlock::new();
    let mut look = Ahead::new(&t);
    let frames: Vec<u32> = (0..tracks)
        .map(|i| if i < ahead { 4_000 } else { 0 })
        .collect();
    look.set(&frames);
    let mut real = Box::new(RealPass {
        out: [Out {
            offset: 0,
            frame: 0,
            track: 0,
            dest: 0,
            event: mpfree_play::Event::AllOff,
            notes: false,
        }; REAL_PASS],
        n: 0,
    });
    let mut sent = 0u64;
    Case {
        name: format!("Time: {tracks} clips, {ahead} tracks 90 ms ahead"),
        module: "time",
        what: "the look-ahead for slow MIDI gear: a forecast copy each block",
        block: Box::new(move |_, l, r| {
            let n = l.len() as u32;
            t.process(n, &mut block);
            real.n = 0;
            player.process(&view, &block, &mut *real);
            for o in &real.out[..real.n] {
                if !look.sent_already(o) {
                    sent += 1;
                }
            }
            look.check(t.frame(), |_| sent += 1);
            look.forecast(&t, &player, &view, n, |_| {
                sent += 1;
                true
            });
            l.fill(0.0);
            r.fill(0.0);
            l[0] = sent as f32;
        }),
        mem_bytes: 0,
    }
}

/// Several armed tracks recording one take (SHIFT + a track's row): the
/// Time case's song without arps, the transport recording (a replace pass,
/// then overdub as each clip comes round), `armed` tracks armed (track 0
/// with its recorder, the others with notes-only ones,
/// `mpfree_play::Recorder::also`), and a keyboard on each playing a chord
/// of 3 a beat, held half a beat. What they record is handed over but not
/// put back in the view (the app's control half does that).
fn time_record(rate: u32, tracks: usize, armed: usize) -> Case {
    use mpfree_play::{Event, Recorder};
    let ppq = i64::from(PPQ);
    let (song, launches) = time_song(tracks, false);
    let mut view = PlayView::compile(&song, &launches).expect("view");
    view.set_armed(Some(0));
    view.set_also(((1u128 << armed) - 1) & !1);
    let mut t = Transport::new(rate, 128.0).expect("transport");
    t.command(0, T::SetLoop(Some((0, 16 * ppq))));
    t.command(0, T::Record);
    let mut player = Player::new();
    let mut block = TBlock::new();
    let mut sink = TimeSink { notes: 0 };
    let mut main = Box::new(Recorder::new());
    let mut others: Vec<Recorder> = (0..armed.saturating_sub(1)).map(Recorder::also).collect();
    // A beat at 128 BPM, in frames (even, so half a beat is whole).
    let beat = u64::from(rate) * 60 / 128 / 2 * 2;
    let mut taken = 0u64;
    Case {
        name: format!("Time: recording {armed} armed tracks, {tracks} clips"),
        module: "time",
        what: "one take over several armed tracks: a recorder each, a replace pass each",
        block: Box::new(move |_, l, r| {
            let n = l.len() as u32;
            let f0 = t.frame();
            t.process(n, &mut block);
            player.process(&view, &block, &mut sink);
            // Each track's keys: a chord on each beat, let go half a beat on.
            let half = beat / 2;
            let keys = |rec: &mut Recorder, base: u8| {
                let mut h = f0.div_ceil(half);
                while h * half < f0 + u64::from(n) {
                    let at = (h * half - f0) as u32;
                    let down = h % 2 == 0;
                    h += 1;
                    if down {
                        for k in 0..3 {
                            rec.input(
                                at,
                                Event::NoteOn {
                                    note: base + 4 * k,
                                    velocity: 100,
                                },
                            );
                        }
                    } else {
                        for k in 0..3 {
                            rec.input(at, Event::NoteOff { note: base + 4 * k });
                        }
                    }
                }
            };
            keys(&mut main, 36);
            for (i, o) in others.iter_mut().enumerate() {
                keys(o, 48 + 2 * i as u8);
            }
            let take = main.take();
            main.process(&view, &block);
            taken += main.taken().len() as u64;
            for o in &mut others {
                o.set_take(take);
                o.process(&view, &block);
                taken += o.taken().len() as u64;
            }
            l.fill(0.0);
            r.fill(0.0);
            l[0] = (sink.notes + taken) as f32;
        }),
        mem_bytes: 0,
    }
}

/// The module of the step-edit cases: the time engine's control half,
/// which builds and edits (and may allocate), not the audio path. Today it
/// runs on the audio thread too (time-threads.md, step 3), so its blocks
/// are what a touch costs there; it has no budget line of its own, and
/// `--check-allocs` leaves it out.
pub const CONTROL: &str = "control";

/// The module of the screen's cases (Arrange refactor 2.4): the screen
/// thread's work, not the audio path's. Like [`CONTROL`] it has no budget
/// line of its own, may allocate and makes no sound.
pub const SCREEN: &str = "screen";

/// The case is not the audio path's: the control half's or the screen's
/// (no sound, so no golden fingerprint, and `--check-allocs` leaves it
/// out).
pub fn off_the_audio_path(c: &Case) -> bool {
    c.module == CONTROL || c.module == SCREEN
}

/// What the big-song cases' names start with (Arrange refactor 2.1). Each
/// of their blocks compiles or saves a whole song, about a fifth of a
/// second a block on a desktop today, so they count and time far fewer
/// blocks than the rest ([`blocks_for`]).
pub const BIG_SONG: &str = "Arrange: ";

/// What the note editor's transpose of every note in a full clip is
/// called (piano roll plan 0C.3). Its edit takes every note out and puts
/// them back one at a time, in the clip and in the joined take, about a
/// second a block on a desktop today, so it counts and times as few
/// blocks as the big-song cases ([`blocks_for`]).
pub const BIG_EDIT: &str = "Note editor: 8,192 notes, all picked";

/// Whether `c` counts and times far fewer blocks than the rest: the
/// big-song cases ([`BIG_SONG`]) and the note editor's biggest edit
/// ([`BIG_EDIT`]).
pub fn big(c: &Case) -> bool {
    c.name.starts_with(BIG_SONG) || c.name.starts_with(BIG_EDIT)
}

/// Blocks to warm up for and then count, of `blocks` asked for: an engine
/// counts every one after 50; the control half's step cases and the
/// screen's cases a tenth of them after 5, since a step edit or a frame
/// costs far more a block than an engine and the same every block; the
/// big cases ([`big`]: the big song's and the note editor's biggest edit)
/// one in 200 (at least one) after one, since a block of theirs is
/// billions of instructions.
pub fn blocks_for(c: &Case, blocks: u64) -> (u64, u64) {
    if big(c) {
        (1, (blocks / 200).max(1))
    } else if off_the_audio_path(c) {
        (5, (blocks / 10).max(1))
    } else {
        (50, blocks)
    }
}

/// A step on the step cases' grid: a sixteenth (the app's `STEP_TICKS`).
const STEP: i64 = 240;
/// Rows of the step cases' clip.
const STEP_ROWS: u8 = 64;
/// Steps of the step cases' clip: 8 bars of sixteenths.
const STEP_COUNT: i64 = 128;
/// Room the app sets aside for recording and step edits in place in the
/// armed track's playing clip (`mpfree-app`'s `RECORD_ROOM`).
const STEP_ROOM: usize = 1_024;

/// What a step-edit case does each block (refactor B1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StepTouch {
    /// A tap on a step: its note comes out, and next block goes back in.
    Tap,
    /// A Q-Link turned on a held step: a detent each block, 64 a turn,
    /// one take a turn.
    Detent,
}

/// The step cases' song: one track with a full clip, 8,192 notes (the most
/// a clip holds), a note on every one of its 128 steps in each of its 64
/// rows, launched and armed.
fn step_song() -> (Song, Vec<Launch>) {
    let mut song = Song::new();
    let mut t = Track::new("t", 0);
    let mut c = Clip::new("c", STEP_COUNT * STEP);
    c.notes = (0..STEP_COUNT)
        .flat_map(|k| (0..STEP_ROWS).map(move |p| Note::new(k * STEP, STEP - 40, p, 100)))
        .collect();
    t.slots[0] = Some(c);
    song.tracks.push(t);
    let launch = Launch {
        track: 0,
        slot: 0,
        origin: 0,
        start: 0,
    };
    (song, vec![launch])
}

/// The view the app compiles after a step edit: the armed track's clip
/// with room to edit in place (`TimeControl::compile`).
fn step_view(song: &mut Song, launches: &[Launch]) -> PlayView {
    if let Some(Some(c)) = song.tracks[0].slots.get_mut(0) {
        c.notes.reserve(STEP_ROOM);
        c.locks.reserve(STEP_ROOM);
    }
    let mut v = PlayView::compile(song, launches).expect("view");
    v.set_armed(Some(0));
    v.make_room(STEP_ROOM, STEP_ROOM);
    v
}

/// A step edit on the Play page's pattern, a touch a block, on the full
/// clip of [`step_song`] while it plays (refactor B1), as
/// `TimeControl::step_action` makes it: the edit worked out on the grid
/// (`Grid::toggle`, `Grid::lock_steps`) and made to the clip and the takes
/// (`Takes::edit`, one undo). With `plays`, also what the audio half does
/// to play it: the edit made to its view in place when it fits
/// (`InPlace::fits`: a tap's one note change costs the clip's 8,192 notes,
/// a detent's two lock changes only its locks; before refactor B3 the
/// detent counted the notes too and compiled the view again). What the
/// screen and the step pads show is the grid rebuilt ([`step_grid`]).
fn step_edit(touch: StepTouch, plays: bool) -> Case {
    use mpfree_play::{InPlace, Takes, MAX_IN_PLACE};
    use mpfree_seq::steps::{lock_key_steps, Grid};
    let (mut song, launches) = step_song();
    let mut view = step_view(&mut song, &launches);
    let mut takes = Takes::default();
    let grid = Grid::new(STEP).expect("a sixteenth");
    let addr = RelAddr::new(OnTrack::Inst, 3);
    let mut changes = [InPlace::EraseNote(Note::new(0, 1, 0, 1)); MAX_IN_PLACE];
    let (mut in_place, mut compiled) = (0u64, 0u64);
    Case {
        name: format!(
            "Steps: {}, 8,192 notes: {}",
            match touch {
                StepTouch::Tap => "a tap",
                StepTouch::Detent => "a lock detent",
            },
            if plays { "until it plays" } else { "the edit" }
        ),
        module: CONTROL,
        what: match (touch, plays) {
            (StepTouch::Tap, false) => {
                "a step's note out, then back in: worked out and made to the clip and the takes"
            }
            (StepTouch::Tap, true) => "as the tap's edit, then made to the playing view in place",
            (StepTouch::Detent, false) => {
                "a held step's lock turned a detent: worked out and joined to the turn's take"
            }
            (StepTouch::Detent, true) => {
                "as the detent's edit, then made to the playing view in place"
            }
        },
        block: Box::new(move |b, l, r| {
            let clip = song.clip(0, 0).expect("the clip");
            let (edit, join) = match touch {
                // Each step of each row in turn, out on one block and back
                // in on the next.
                StepTouch::Tap => {
                    let k = b / 2;
                    let pitch = (k % u64::from(STEP_ROWS)) as u8;
                    let step = (k * 7 % STEP_COUNT as u64) as usize;
                    (grid.toggle(clip, pitch, step, 100), None)
                }
                StepTouch::Detent => {
                    let (sound, step) = (Some(5), 9usize);
                    let value = (b % 64) as f32 / 64.0;
                    (
                        grid.lock_steps(clip, sound, &[step], addr, Some(value)),
                        Some(lock_key_steps(sound, 1 << step, addr)),
                    )
                }
            };
            takes.edit(&mut song, 0, 0, &edit, join).expect("the edit");
            if plays {
                let (notes, locks) = song
                    .clip(0, 0)
                    .map_or((0, 0), |c| (c.notes.len(), c.locks.len()));
                let n = InPlace::count(&edit);
                if InPlace::fits(&edit, notes, locks) {
                    for (slot, c) in changes.iter_mut().zip(InPlace::all(&edit)) {
                        *slot = c;
                    }
                    assert!(view.edit_in_place(0, 0, &changes[..n]), "in place");
                    in_place += 1;
                } else {
                    view = step_view(&mut song, &launches);
                    compiled += 1;
                }
            }
            // What the clip and the view hold, and how the edits went.
            l.fill(0.0);
            r.fill(0.0);
            let notes = song.clip(0, 0).map_or(0, |c| c.notes.len() + c.locks.len());
            l[0] = notes as f32;
            r[0] = (view.notes() + view.locks()) as f32;
            l[1] = in_place as f32;
            r[1] = compiled as f32;
        }),
        mem_bytes: 0,
    }
}

/// The grid the Play page's pattern and the step pads show, built again
/// after an edit (`Grid::shown`, what `TimeControl::steps` hands the
/// screen), a block each, on the full clip of [`step_song`] (refactor B1).
/// With [`step_edit`], the third of a touch's three times: until the
/// screen and the pads show it.
fn step_grid() -> Case {
    use mpfree_seq::steps::Grid;
    let (song, _) = step_song();
    let grid = Grid::new(STEP).expect("a sixteenth");
    Case {
        name: "Steps: the grid rebuilt, 8,192 notes in 64 rows".to_string(),
        module: CONTROL,
        what: "every row's steps, lock counts and lock values, for the screen and the step pads",
        block: Box::new(move |_, l, r| {
            let clip = song.clip(0, 0).expect("the clip");
            let shown = grid.shown(clip, &mpfree_seq::steps::PAD_ROWS);
            l.fill(0.0);
            r.fill(0.0);
            l[0] = shown
                .rows
                .iter()
                .map(|(_, row)| row.iter().flatten().count())
                .sum::<usize>() as f32;
            r[0] = shown.rows.len() as f32;
        }),
        mem_bytes: 0,
    }
}

/// Tracks of the arrangement cases' song (Arrange refactor 2.1).
const ARR_TRACKS: usize = 16;
/// Blocks a track in the arrangement cases' song: 512 in all.
const ARR_BLOCKS: i64 = 32;
/// Notes in each of the arrangement cases' 32 clips.
const ARR_NOTES: i64 = 1_000;

/// The arrangement cases' big song (Arrange refactor 2.1): 16 tracks, each
/// with two 4-bar clips of 1,000 notes in rows 0 and 1 (32 clips), and a
/// song of 512 blocks, 32 a track, each 4 bars and each naming one of its
/// track's two clips, so every clip plays in 16 blocks.
fn arrange_song() -> Song {
    use mpfree_seq::Placement;
    let len = 16 * i64::from(PPQ);
    let mut song = Song::new();
    for t in 0..ARR_TRACKS {
        let mut tr = Track::new("t", t as u16);
        for row in 0..2usize {
            let mut c = Clip::new("c", len);
            c.notes = (0..ARR_NOTES)
                .map(|k| {
                    let pitch = 36 + (k * 7 + row as i64) % 48;
                    Note::new(k * len / ARR_NOTES, 8, pitch as u8, 100)
                })
                .collect();
            tr.slots[row] = Some(c);
        }
        song.tracks.push(tr);
    }
    song.give_clip_ids();
    for t in 0..ARR_TRACKS {
        for k in 0..ARR_BLOCKS {
            let b = Placement::of(&song.tracks, t, (k % 2) as usize, k * len, len);
            song.arrangement.place(b).expect("room for the block");
        }
    }
    song
}

/// The view of the big song in song mode, as `TimeControl::view` builds it.
fn arrange_view(song: &Song) -> PlayView {
    let mut v = PlayView::compile_cued(song, &[], &[]).expect("view");
    v.set_song(song, true).expect("song");
    v
}

/// The big song's view (Arrange refactor 2.1), a block each: compiled as the
/// control half compiles it after an edit in song mode, taken by the audio
/// half (`Recent::catch_up`, then the swap), and the view it played freed,
/// as the control half frees it when it comes back. Its bytes are the
/// view's alone. Phase 2.3 (one compiled copy per clip) is judged by it.
fn arrange_compile() -> Case {
    use mpfree_play::Recent;
    let song = arrange_song();
    let (view, bytes) = alloc::bytes_in(|| Box::new(arrange_view(&song)));
    let mut view = view;
    let mut recent = Box::new(Recent::new());
    Case {
        name: "Arrange: the big song's view, 512 blocks of 32 clips: compiled and swapped"
            .to_string(),
        module: CONTROL,
        what: "the song-mode view built, caught up and swapped in, and the old one freed",
        block: Box::new(move |_, l, r| {
            let mut next = Box::new(arrange_view(&song));
            let after = recent.last();
            recent.catch_up(&mut next, after).expect("caught up");
            let old = std::mem::replace(&mut view, next);
            drop(old);
            l.fill(0.0);
            r.fill(0.0);
            l[0] = view.song_notes() as f32;
        }),
        mem_bytes: bytes,
    }
}

/// A step tapped on one of the big song's clips in song mode (Arrange
/// refactor 2.1), a block each, as `TimeControl::step_action` makes it: the
/// edit worked out on the grid and made to the clip and the takes, then
/// made to the view in place, as the audio half makes it
/// (`PlayView::edit_in_place`), in the one copy all 16 of the clip's blocks
/// play (2.3; before it, the view compiled again).
fn arrange_step() -> Case {
    use mpfree_play::{InPlace, Takes};
    use mpfree_seq::steps::Grid;
    let mut song = arrange_song();
    let mut view = arrange_view(&song);
    // The room the control half makes on the armed track (RECORD_ROOM).
    view.make_room_on(0, 64, 64);
    let mut takes = Takes::default();
    let grid = Grid::new(STEP).expect("a sixteenth");
    Case {
        name: "Arrange: a step tap in song mode, 512 blocks of 32 clips: until it plays"
            .to_string(),
        module: CONTROL,
        what:
            "a step's note out, then back in, on a clip 16 blocks play, made to the view in place",
        block: Box::new(move |b, l, r| {
            let clip = song.clip(0, 0).expect("the clip");
            let k = b / 2;
            let step = (k * 7 % 64) as usize;
            let edit = grid.toggle(clip, 60, step, 100);
            takes.edit(&mut song, 0, 0, &edit, None).expect("the edit");
            let changes: Vec<InPlace> = InPlace::all(&edit).collect();
            assert!(view.edit_in_place(0, 0, &changes), "in place");
            l.fill(0.0);
            r.fill(0.0);
            l[0] = song.clip(0, 0).map_or(0, |c| c.notes.len()) as f32;
            r[0] = view.song_notes() as f32;
        }),
        mem_bytes: 0,
    }
}

/// The big song's clips part saved and opened again (Arrange refactor 2.1),
/// a block each: `mpfree_seq::save::to_text` and `from_text`, as a project
/// save and open do for the time engine's part.
fn arrange_save() -> Case {
    use mpfree_seq::save;
    let song = arrange_song();
    Case {
        name: "Arrange: the big song saved and opened, 512 blocks of 32 clips".to_string(),
        module: CONTROL,
        what: "the clips part written as text and read back",
        block: Box::new(move |_, l, r| {
            let text = save::to_text(&song);
            let back = save::from_text(&text).expect("it opens");
            l.fill(0.0);
            r.fill(0.0);
            l[0] = text.len() as f32;
            r[0] = back.arrangement.placements.len() as f32;
        }),
        mem_bytes: 0,
    }
}

/// The Song view of the longest song (Arrange refactor 2.4): 4,096 blocks
/// (`MAX_PLACEMENTS`: 16 tracks of 256 four-bar blocks, each naming one of
/// its track's two clips), a frame a block, as the screen draws one while
/// the song plays: the playhead moved (`ToUi::Transport`), the time page
/// updated and the page painted in software at the MPC's size. Every 8th
/// frame the view scrolls a bar on, as the wheel does, back to the start
/// past the song's end. Its bytes include the page and the fonts.
fn song_view_frame() -> Case {
    use mpfree_play::Takes;
    use mpfree_seq::Placement;
    use mpfree_time_ui::view::{self as tv, TimePage, TimeState, TransportInfo};
    use mpfree_ui::widgets::{Header, Label};
    use mpfree_ui::{Align, Fonts, SoftCanvas, Theme, Tone, Ui};
    let bar = 4 * i64::from(PPQ);
    let mut song = Song::new();
    for t in 0..ARR_TRACKS {
        let mut tr = Track::new(&format!("Track {}", t + 1), t as u16);
        tr.slots[0] = Some(Clip::new(&format!("Verse {}", t + 1), 4 * bar));
        tr.slots[1] = Some(Clip::new(&format!("Chorus {}", t + 1), 4 * bar));
        song.tracks.push(tr);
    }
    song.give_clip_ids();
    let per_track = (mpfree_seq::arrange::MAX_PLACEMENTS / ARR_TRACKS) as i64;
    for t in 0..ARR_TRACKS {
        for k in 0..per_track {
            let b = Placement::of(&song.tracks, t, (k % 2) as usize, k * 4 * bar, 4 * bar);
            song.arrangement.place(b).expect("room for the block");
        }
    }
    let playing = TransportInfo {
        state: mpfree_transport::State::Playing,
        ..TransportInfo::default()
    };
    let mut st = TimeState::from_engine(playing, &song, &[], Some(0), &Takes::default());
    st.song_view = true;
    st.song_on = true;
    let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
    ui.add("header", Header::new(&["Play", "Clips"]));
    ui.add("hint", Label::new("", Tone::Dim, Align::Center));
    let mut page = TimePage::add_to(&mut ui).expect("the time page");
    ui.show(tv::CLIPS_PAGE).expect("the clips page");
    page.update(&mut ui, &st);
    let mut canvas = SoftCanvas::new(1280, 800);
    ui.paint(&mut canvas);
    let end_bar = per_track * 4;
    Case {
        name: "Song view: 4,096 blocks, a frame while the song plays".to_string(),
        module: SCREEN,
        what: "the playhead moved, the page updated and painted; a bar's scroll every 8th frame",
        block: Box::new(move |b, l, r| {
            st.transport.position += i64::from(PPQ) / 8;
            if b % 8 == 7 {
                let back = if page.first_bar + 1 >= end_bar {
                    -end_bar
                } else {
                    1
                };
                page.scroll(&mut ui, back as i32);
            }
            page.update(&mut ui, &st);
            let drawn = ui.paint(&mut canvas);
            l.fill(0.0);
            r.fill(0.0);
            l[0] = drawn.len() as f32;
            r[0] = page.first_bar as f32;
        }),
        mem_bytes: 0,
    }
}

/// What a piano roll case does each frame (piano roll plan 0C.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RollFrame {
    /// A new copy of the clip each frame (the clip changed): the roll
    /// built again from its notes and drawn.
    Built,
    /// The same, every note with a run of pad pressure.
    BuiltPressure,
    /// The playhead moved while the clip plays: only it moves.
    Plays,
    /// Two bars shown, scrolled on a sixteenth each frame, as a finger on
    /// the ruler does.
    Scrolled,
    /// Zoomed a step each frame from a bar across the roll to the whole
    /// clip and back, as a pinch does.
    Zoomed,
    /// Two full clips shown in turn, a frame each: the Sequencer's clip
    /// switched, each keeping its own view.
    Switched,
}

/// What [`roll_page`] sets up: the screen, the time page, its state, a
/// canvas and the two clips' edit pictures.
type RollPage = (
    mpfree_ui::Ui,
    mpfree_time_ui::view::TimePage,
    mpfree_time_ui::view::TimeState,
    mpfree_ui::SoftCanvas,
    [Box<mpfree_time_ui::view::EditInfo>; 2],
);

/// The note editor's page on the full clip of [`step_song`] (8,192 notes,
/// 8 bars), as the Sequencer shows it: the transport playing, the editor
/// open with its drawer closed, painted once. With `pressure`, every note
/// has a run of pad pressure (as many points each as the clip holds,
/// [`mpfree_seq::MAX_TOUCH_POINTS`]); with `two`, slot 1 holds a copy.
fn roll_page(pressure: bool, two: bool) -> RollPage {
    use mpfree_play::Takes;
    use mpfree_seq::TouchPoint;
    use mpfree_time_ui::editor::Editor;
    use mpfree_time_ui::view::{self as tv, EditInfo, TimePage, TimeState, TransportInfo};
    use mpfree_ui::widgets::{Header, Label};
    use mpfree_ui::{Align, Fonts, SoftCanvas, Theme, Tone, Ui};
    use std::sync::Arc;
    let (mut song, launches) = step_song();
    if pressure {
        let c = song.tracks[0].slots[0].as_mut().expect("the clip");
        let each = (mpfree_seq::MAX_TOUCH_POINTS / c.notes.len()).clamp(1, 8);
        for k in 0..c.notes.len() {
            let len = c.notes[k].len.max(1) as u32;
            let run: Vec<TouchPoint> = (0..each)
                .map(|j| TouchPoint {
                    at: len * j as u32 / each as u32,
                    value: (40 + 80 * j / each) as u8,
                })
                .collect();
            let t = c.add_touch(&run).expect("room for the pressure");
            c.notes[k].touch = t;
        }
    }
    if two {
        let copy = song.tracks[0].slots[0].clone();
        song.tracks[0].slots[1] = copy;
    }
    let playing = TransportInfo {
        state: mpfree_transport::State::Playing,
        ..TransportInfo::default()
    };
    let st = TimeState::from_engine(playing, &song, &launches, Some(0), &Takes::default());
    let bar = 4 * i64::from(PPQ);
    let info = |slot: usize, rev: u64| {
        let clip = song.clip(0, slot).expect("the clip");
        Box::new(EditInfo {
            editor: Editor::new(0, slot, clip),
            clip: Arc::new(clip.clone()),
            rev,
            track_name: "Keys".into(),
            bar,
            answer: Default::default(),
        })
    };
    let infos = [info(0, 1), info(if two { 1 } else { 0 }, 2)];
    let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
    ui.add("header", Header::new(&["Play", "Sequencer"]));
    ui.add("hint", Label::new("", Tone::Dim, Align::Center));
    let mut page = TimePage::add_to(&mut ui).expect("the time page");
    ui.show(tv::CLIPS_PAGE).expect("the clips page");
    let mut st = st;
    st.edit = Some(infos[0].clone());
    let mut canvas = SoftCanvas::new(1280, 800);
    // The roll gets its size on the first paint, then its opening view.
    for _ in 0..2 {
        page.update(&mut ui, &st);
        ui.paint(&mut canvas);
    }
    (ui, page, st, canvas, infos)
}

/// The piano roll on a full clip of 8,192 notes, a frame a block, as the
/// screen draws it (piano roll plan 0C.3; see [`RollFrame`]): the page
/// updated and painted in software at the MPC's size.
fn roll_frame(what: RollFrame) -> Case {
    use mpfree_time_ui::editor::NoteGrid;
    use mpfree_time_ui::view as tv;
    use std::sync::Arc;
    let (mut ui, mut page, mut st, mut canvas, infos) = roll_page(
        what == RollFrame::BuiltPressure,
        what == RollFrame::Switched,
    );
    // The clip's second copy, for a new one each frame.
    let copies = [infos[0].clip.clone(), Arc::new((*infos[0].clip).clone())];
    let area = ui.rect_of(tv::NOTES).expect("the roll's area");
    let (length, bar) = (infos[0].clip.length, infos[0].bar);
    // Scrolled: two bars across; zoomed: a bar across to the whole clip.
    let roll = ui
        .get::<NoteGrid>(tv::NOTES)
        .map(|g| g.roll_rect(area))
        .expect("the roll");
    let roll_w = roll.w;
    let ppt_of = move |ticks: i64| f64::from(roll_w) / ticks as f64;
    if let Some(g) = ui.get_mut::<NoteGrid>(tv::NOTES) {
        if what == RollFrame::Scrolled {
            g.view = g
                .view
                .zoom_time(roll, length, 128, ppt_of(2 * bar), 0.0, roll.x);
        }
    }
    page.update(&mut ui, &st);
    ui.paint(&mut canvas);
    let (name, what_text) = match what {
        RollFrame::Built => (
            "Piano roll: 8,192 notes, built and drawn",
            "a new copy of the clip: the roll built again from its notes, updated and painted",
        ),
        RollFrame::BuiltPressure => (
            "Piano roll: 8,192 notes with pressure, built and drawn",
            "as built and drawn, every note with a run of pad pressure drawn along it",
        ),
        RollFrame::Plays => (
            "Piano roll: 8,192 notes, a frame while it plays",
            "the playhead moved: the roll's playhead set, the page painted",
        ),
        RollFrame::Scrolled => (
            "Piano roll: 8,192 notes, scrolled a sixteenth a frame",
            "two bars across, the view moved on a sixteenth, the page updated and painted",
        ),
        RollFrame::Zoomed => (
            "Piano roll: 8,192 notes, zoomed from a bar to the whole clip",
            "the view zoomed a step from a bar across to the whole clip and back, painted",
        ),
        RollFrame::Switched => (
            "Piano roll: two clips of 8,192 notes, switched each frame",
            "the other clip shown, its view kept: the roll built again and painted",
        ),
    };
    Case {
        name: name.to_string(),
        module: SCREEN,
        what: what_text,
        block: Box::new(move |b, l, r| {
            st.transport.position += i64::from(PPQ) / 8;
            match what {
                RollFrame::Built | RollFrame::BuiltPressure => {
                    if let Some(e) = st.edit.as_mut() {
                        e.clip = copies[(b % 2) as usize].clone();
                        e.rev += 1;
                    }
                }
                RollFrame::Plays => {}
                RollFrame::Scrolled => {
                    if let Some(g) = ui.get_mut::<NoteGrid>(tv::NOTES) {
                        let mut v = g.view;
                        v.start = (b as i64 * STEP) % (length - 2 * bar);
                        g.view = v.clamped(roll, length, 128);
                    }
                }
                RollFrame::Zoomed => {
                    // 16 steps each way, a bar across to the whole clip.
                    let k = (b % 32) as i64;
                    let k = if k < 16 { k } else { 31 - k };
                    let ticks = bar + (length - bar) * k / 15;
                    if let Some(g) = ui.get_mut::<NoteGrid>(tv::NOTES) {
                        let mid = (length / 2) as f64;
                        g.view = g.view.zoom_time(
                            roll,
                            length,
                            128,
                            ppt_of(ticks),
                            mid,
                            roll.x + roll.w / 2,
                        );
                    }
                }
                RollFrame::Switched => st.edit = Some(infos[(b % 2) as usize].clone()),
            }
            page.update(&mut ui, &st);
            let drawn = ui.paint(&mut canvas);
            l.fill(0.0);
            r.fill(0.0);
            let g = ui.get::<NoteGrid>(tv::NOTES);
            l[0] = drawn.len() as f32;
            r[0] = g.map_or(0, |g| g.notes.len()) as f32;
            r[1] = g.map_or(0.0, |g| g.view.ppt) as f32;
            l[1] = g.map_or(0, |g| g.view.start) as f32;
        }),
        mem_bytes: 0,
    }
}

/// What a note editor edit case does (piano roll plan 0C.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NoteEditCase {
    /// One picked note's velocity set (a velocity lane drag, one edit on
    /// the lift), then taken back by UNDO, a block each.
    VelocityAndUndo,
    /// Every note picked and transposed a semitone, up then down.
    TransposeAll,
}

/// A note editor edit on the full clip of [`step_song`] while it plays,
/// as `TimeControl::edit` makes it (piano roll plan 0C.3): the edit worked
/// out (`Editor::act`), made to the clip and the takes (`Takes::edit`, one
/// undo), the picks found again (`Editor::applied`), and the playing view
/// edited in place when it fits (`InPlace::fits`) or compiled again. On a
/// clip this full nothing fits: a velocity change is a note out and one
/// in, which costs twice the clip's 8,192 notes in place, more than
/// `IN_PLACE_WORK` (a step tap's one note change fits); a transpose of
/// every note is 16,384 changes; UNDO always compiles in the app.
fn note_edit(what: NoteEditCase) -> Case {
    use mpfree_play::{InPlace, Takes, MAX_IN_PLACE};
    use mpfree_time_ui::editor::{EditAction, Editor};
    let (mut song, launches) = step_song();
    let mut view = step_view(&mut song, &launches);
    let mut takes = Takes::default();
    let mut ed = Editor::new(0, 0, song.clip(0, 0).expect("the clip"));
    let mut changes = [InPlace::EraseNote(Note::new(0, 1, 0, 1)); MAX_IN_PLACE];
    let (mut in_place, mut compiled) = (0u64, 0u64);
    let (name, what_text) = match what {
        NoteEditCase::VelocityAndUndo => (
            "Note editor: 8,192 notes, a velocity change and its UNDO, until it plays",
            "a picked note's velocity set, then UNDO, a block each: the view compiled after each",
        ),
        NoteEditCase::TransposeAll => (
            "Note editor: 8,192 notes, all picked and transposed, until it plays",
            "every note moved a semitone, the picks found again, the view compiled",
        ),
    };
    Case {
        name: name.to_string(),
        module: CONTROL,
        what: what_text,
        block: Box::new(move |b, l, r| {
            let undo = what == NoteEditCase::VelocityAndUndo && b % 2 == 1;
            if undo {
                takes.undo(&mut song);
                view = step_view(&mut song, &launches);
                compiled += 1;
            } else {
                let clip = song.clip(0, 0).expect("the clip");
                let a = match what {
                    NoteEditCase::VelocityAndUndo => {
                        ed.sel = vec![(b as usize * 37) % clip.notes.len()];
                        EditAction::Velocity((b % 64) as f32 / 64.0)
                    }
                    NoteEditCase::TransposeAll => {
                        ed.sel = (0..clip.notes.len()).collect();
                        EditAction::Row {
                            row: 7,
                            delta: if b % 2 == 0 { 1 } else { -1 },
                        }
                    }
                };
                let done = ed.act(a, clip);
                assert!(InPlace::count(&done.edit) > 0, "an edit");
                takes
                    .edit(&mut song, 0, 0, &done.edit, done.join)
                    .expect("the edit");
                let clip = song.clip(0, 0).expect("the clip");
                ed.applied(clip, &done);
                let (notes, locks) = (clip.notes.len(), clip.locks.len());
                let n = InPlace::count(&done.edit);
                if InPlace::fits(&done.edit, notes, locks) {
                    for (slot, c) in changes.iter_mut().zip(InPlace::all(&done.edit)) {
                        *slot = c;
                    }
                    assert!(view.edit_in_place(0, 0, &changes[..n]), "in place");
                    in_place += 1;
                } else {
                    view = step_view(&mut song, &launches);
                    compiled += 1;
                }
            }
            l.fill(0.0);
            r.fill(0.0);
            l[0] = song.clip(0, 0).map_or(0, |c| c.notes.len()) as f32;
            r[0] = view.notes() as f32;
            l[1] = in_place as f32;
            r[1] = compiled as f32;
        }),
        mem_bytes: 0,
    }
}

/// The time engine at its densest (refactor B1): 16 tracks playing 2-bar
/// clips of 16ths, each step a 3-note chord ratcheted 4 times with 8
/// parameter locks (7 of the instrument's, one of the strip's, each step
/// a new value), the transport looping 4 bars at 128 BPM through the
/// per-track arps, none of them on, as the pad player runs them.
fn time_dense(rate: u32) -> Case {
    let ppq = i64::from(PPQ);
    let mut song = Song::new();
    let mut launches = Vec::new();
    for i in 0..16usize {
        let mut t = Track::new("t", i as u16);
        let mut c = Clip::new("c", 8 * ppq);
        c.notes = (0..32i64)
            .flat_map(|k| {
                (0..3i64).map(move |n| Note {
                    ratchet: 4,
                    ..Note::new(
                        k * ppq / 4,
                        ppq / 4 - 10,
                        (36 + 12 * (i as i64 % 3) + n * 4 + k % 5) as u8,
                        100,
                    )
                })
            })
            .collect();
        c.locks = (0..32i64)
            .flat_map(|k| {
                (0..8u16).map(move |p| {
                    let addr = if p < 7 {
                        RelAddr::new(OnTrack::Inst, p)
                    } else {
                        RelAddr::new(OnTrack::Strip, 0)
                    };
                    Lock::new(
                        k * ppq / 4,
                        None,
                        addr,
                        ((k + i64::from(p)) % 8) as f32 / 8.0,
                    )
                })
            })
            .collect();
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
    let mut sink = TimeSink { notes: 0 };
    let mut player = Player::new();
    let mut block = TBlock::new();
    Case {
        name: "Time: 16 tracks of ratchets, 8 locks a step".to_string(),
        module: "time",
        what: "16ths with 3-note chords ratcheted 4 times and 8 locks a step on 16 tracks",
        block: Box::new(move |_, l, r| {
            t.process(l.len() as u32, &mut block);
            player.process(&view, &block, &mut arps.through_sink());
            arps.finish(&block, &mut sink);
            l.fill(0.0);
            r.fill(0.0);
            l[0] = sink.notes as f32;
        }),
        mem_bytes: 0,
    }
}

/// Events one real period plays, as the app's sink keeps them.
const REAL_PASS: usize = 1_024;

struct RealPass {
    out: [Out; REAL_PASS],
    n: usize,
}

impl Sink for RealPass {
    fn send(&mut self, o: Out) {
        if let Some(slot) = self.out.get_mut(self.n) {
            *slot = o;
            self.n += 1;
        }
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
/// `files`: recording to files on (phase 3a): each take recorded is copied
/// out of the memory as it records, at most `host::COPY_SAMPLES` samples a
/// block, as the host does on the audio thread for the writer.
fn audio_tracks(rate: u32, recording: usize, files: bool) -> Case {
    const BARS: i64 = 8;
    let bar = 4 * i64::from(PPQ);
    let beat = i64::from(PPQ);
    let song_end = BARS * bar;
    let mut t = Transport::new(rate, 120.0).expect("transport");
    // 8 bars at 120 BPM: 16 s a pass; three passes on 8 mono tracks are
    // 384 s of mono, and the recording tracks need more on top.
    let mut a = AudioTracks::new(rate, 300);
    // The host's part: every take recording copied out, `limit` samples a
    // block, into a buffer made once.
    let mut copied = vec![0usize; mpfree_audiotracks::MAX_TAKES];
    let mut writing: Vec<u16> = Vec::with_capacity(mpfree_audiotracks::MAX_TAKES);
    let mut buf = vec![0.0f32; COPY_SAMPLES];
    let mut serve = move |a: &mut AudioTracks, limit: usize| {
        while let Some(r) = a.take_io() {
            if let TrackIo::Start { take } = r {
                writing.push(take);
            }
        }
        let mut budget = limit;
        writing.retain(|&id| {
            let Some(k) = a.take(id) else { return false };
            let room = (budget.min(buf.len()) / k.channels) * k.channels;
            let n = a
                .read_take(id, copied[id as usize], &mut buf[..room])
                .unwrap_or(0);
            copied[id as usize] += n;
            budget -= n * k.channels;
            let done = !k.recording && copied[id as usize] >= k.frames;
            if done {
                a.written(id);
            }
            !done
        });
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
    // Files from here on: the passes above were set up in memory.
    a.files(files);
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
            "playing",
            match (recording, files) {
                (0, _) => String::new(),
                (k, false) => format!(", {k} recording"),
                (k, true) => format!(", {k} recording to files"),
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
            if files {
                serve(&mut a, COPY_SAMPLES);
            }
            for (i, (x, y)) in ol.iter_mut().zip(or.iter_mut()).enumerate() {
                *x = out[2 * i];
                *y = out[2 * i + 1];
            }
        }),
        mem_bytes: 0,
    }
}

/// Audio clips (phase 2, step 2b): every track looping a 2-bar clip of two
/// regions (a comp boundary in the middle), and launching another on every
/// bar line (the tracks taking turns through 3 clips), so each bar every
/// track crossfades a launch over the old clip's fade out, as the
/// arranger's player says it.
fn audio_clips(rate: u32) -> Case {
    use mpfree_audiotracks::save::TakeFile;
    use mpfree_audiotracks::{Answer, Change, ClipPlay, ClipRegion, Plays, RecordingId};
    const CLIPS: usize = 3;
    let bar = 4 * i64::from(PPQ);
    let bar_frames = 2 * u64::from(rate); // 120 BPM
    let mut t = Transport::new(rate, 120.0).expect("transport");
    let mut a = AudioTracks::new(rate, 60);
    // Three recordings of 2 bars and a half, stereo, from the bench's
    // input.
    let prog = program(rate);
    let frames = (5 * rate / 2) as usize * 2;
    for k in 0..CLIPS as u16 {
        let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
        prog.play(u64::from(k) * 7 * u64::from(rate), &mut l, &mut r);
        let audio = l.iter().zip(&r).flat_map(|(x, y)| [*x, *y]).collect();
        let f = TakeFile {
            id: k,
            rec: RecordingId(1 + u128::from(k)),
            track: 0,
            group: k,
            pass: 0,
            bpm: 120.0,
            anchor: 0.0,
            frames,
            channels: 2,
            file: String::new(),
            path: None,
            audio,
        };
        assert!(a.load_take(f).is_ok(), "a take loads");
    }
    let regions: Vec<[ClipRegion; 2]> = (0..CLIPS as u16)
        .map(|k| {
            let r = ClipRegion {
                start: 0,
                end: bar,
                rec: RecordingId(1 + u128::from(k)),
                src: 0,
                phase: 0.0,
                gain: 1.0,
                fade_in: 0,
                fade_out: 0,
                into: 0,
                left: 0,
            };
            let later = i64::from(rate) / 3;
            [
                r,
                ClipRegion {
                    start: bar,
                    end: 2 * bar,
                    src: bar_frames as i64 + later,
                    ..r
                },
            ]
        })
        .collect();
    let mut block = TBlock::new();
    let n = MAX_BLOCK;
    let (line_in, mut out) = (vec![0.0f32; 2 * n], vec![0.0f32; 2 * n]);
    let mut frame = 0u64;
    while a.loading() > 0 {
        t.process(n as u32, &mut block);
        let map = *t.tempo_map();
        a.process(&block, &map, &line_in, &mut out);
        frame += n as u64;
    }
    t.command(frame, T::PlayStart);
    let start = frame;
    let mut tr = t;
    Case {
        name: format!("Audio tracks: {MAX_TRACKS} tracks of 2-bar clips launched every bar"),
        module: "looper",
        what: "a launch on every track on every bar line, crossfading",
        block: Box::new(move |_, ol, or| {
            let n = ol.len();
            // The clip track `k` plays in bar `b`, from its line.
            let plays = |k: usize, b: u64| {
                let c = (b as usize + k) % CLIPS;
                Plays::Clip(ClipPlay {
                    regions: &regions[c],
                    origin: (b as i64 * bar) as f64,
                    len: 2 * bar,
                    last_frame: 2 * bar_frames as i64,
                    gain: 1.0,
                    clip: c as u32 + 1,
                    stamp: 1,
                })
            };
            let since = frame.saturating_sub(start);
            let b0 = since.saturating_sub(1) / bar_frames;
            // A bar line in this block: every track launches there.
            let line = (b0 + 1) * bar_frames;
            let mut changes = [[Change {
                frame: 0,
                tick: 0.0,
                plays: Plays::Nothing,
                muted: false,
                ring: false,
            }; 1]; MAX_TRACKS];
            let mut answers = [Answer::default(); MAX_TRACKS];
            for (k, (ans, ch)) in answers.iter_mut().zip(changes.iter_mut()).enumerate() {
                let mut c = 0;
                if frame > start && (since..since + n as u64).contains(&line) {
                    ch[0] = Change {
                        frame: (line - since) as u32,
                        tick: ((b0 + 1) as i64 * bar) as f64,
                        plays: plays(k, b0 + 1),
                        muted: false,
                        ring: false,
                    };
                    c = 1;
                }
                *ans = Answer {
                    start: if frame > start {
                        plays(k, b0)
                    } else {
                        Plays::Song
                    },
                    muted: false,
                    changes: &ch[..c],
                    superseded: 0,
                };
            }
            out[..2 * n].fill(0.0);
            tr.process(n as u32, &mut block);
            let map = *tr.tempo_map();
            a.process_with(&block, &map, &line_in[..2 * n], &mut out[..2 * n], &answers);
            frame += n as u64;
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

/// The synth tracks cases' instruments: a busy song's, a factory preset
/// each from eight engines.
pub const SONG_TRACKS: [&str; 8] = [
    "Wavetable Morph Pad",
    "Poly Supersaw",
    "FM Glass Keys",
    "Analog Warm Pad",
    "Strings Hall Strings",
    "Keys Mellow Tines",
    "Drawbar Gospel",
    "Bass Amp Bass",
];

/// The tracks' own instruments as a song plays them (`--lanes`): the eight
/// of [`SONG_TRACKS`] in the track host, each a four-note chord (the bass
/// one note) changing every half second, the tracks a beat apart. With no
/// `lanes` they all render on this thread; with lanes the host splits them
/// over this thread and the lanes (`TrackHost::set_lanes`), placing them
/// by what they cost, and the sound is the same to the bit while no lane
/// is late. `wait` is how long a lane is waited for.
fn synth_tracks(name: &str, rate: u32, lanes: Vec<LaneConfig>, wait: Duration) -> Case {
    let max = MAX_BLOCK.min(1024);
    let presets = keyboard_presets(rate);
    let tracks = SONG_TRACKS.len();
    let mut host = TrackHost::new(tracks, rate, max);
    let mut voices = Vec::with_capacity(tracks);
    for (t, want) in SONG_TRACKS.iter().enumerate() {
        let p = presets
            .iter()
            .find(|p| p.name == *want)
            .expect("a factory preset");
        let mut inst = Instrument::new((p.make)(), rate, max);
        for &(id, v) in &p.values {
            inst.node_mut().set_param(id, v);
        }
        voices.push(if p.mono { 1 } else { 4 });
        host.swap(t, Some(inst)).expect("a free slot");
    }
    let split = !lanes.is_empty();
    if split {
        host.set_lanes(lanes);
    }
    let mut held: Vec<Vec<u8>> = voices.iter().map(|&v| Vec::with_capacity(v)).collect();
    Case {
        name: name.to_string(),
        module: "lanes",
        what: "8 synth tracks of 8 engines, chords changing every 0.5 s",
        block: Box::new(move |b, l, r| {
            let len = l.len() as u64;
            let f0 = b * len;
            let half = u64::from(rate / 2);
            for (t, notes) in held.iter_mut().enumerate() {
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
                    for n in chord(t, g / half, voices[t]) {
                        notes.push(n + 12);
                        host.note_on(t, at, n + 12, 96);
                    }
                }
            }
            l.fill(0.0);
            r.fill(0.0);
            if split {
                host.set_deadline(Some(Instant::now() + wait));
            }
            host.render(&Ctx::new(rate, f0), l.len(), &mut Sum { left: l, right: r });
            if split {
                host.settle(Instant::now() + wait);
            }
        }),
        mem_bytes: 0,
    }
}

/// Synth-voice pads with synths of their own (`mpfree-app`'s
/// `pad_synths.rs`, the redesign's 6b): `pads` copies of Poly as the
/// Synth tab starts it, in a host of twice as many slots (the rest for
/// ring-outs) as the app has it, each pad hit once every half second
/// (the pads a 1/16 of it apart) and let go after a quarter of it, so
/// every copy is playing or ringing out at once: the most the app lets
/// play this way (16). With `lanes` the host splits the copies over this
/// thread and the lanes by what they cost, as the app does on two or three
/// sound cores (`PadSynths::set_lanes`); `wait` is how long a lane is
/// waited for.
fn pad_synths(name: &str, rate: u32, pads: usize, lanes: Vec<LaneConfig>, wait: Duration) -> Case {
    let max = MAX_BLOCK.min(1024);
    let mut host = TrackHost::new(2 * pads, rate, max);
    for p in 0..pads {
        host.swap(
            p,
            Some(Instrument::new(Box::new(Poly::new(rate)), rate, max)),
        )
        .expect("a free slot");
        host.route(p, Some(p));
    }
    for s in pads..2 * pads {
        host.route(s, None);
    }
    let split = !lanes.is_empty();
    if split {
        host.set_lanes(lanes);
    }
    let half = u64::from(rate / 2);
    Case {
        name: name.to_string(),
        module: if split { "lanes" } else { "synth" },
        what: "each pad's own Poly playing and ringing out at once",
        block: Box::new(move |b, l, r| {
            let len = l.len() as u64;
            let f0 = b * len;
            for p in 0..pads {
                let note = 48 + (p as u8 * 5) % 24;
                let start = half * p as u64 / pads as u64;
                // The hit and the let-go, each at its frame in this block.
                for (at, on) in [(start, true), (start + half / 4, false)] {
                    // The frames from the block's start to the next one.
                    let to = (at % half + half - f0 % half) % half;
                    if to < len {
                        let k = to as u32;
                        if on {
                            host.note_on(p, k, note, 100);
                        } else {
                            host.note_off(p, k, note);
                        }
                    }
                }
            }
            l.fill(0.0);
            r.fill(0.0);
            if split {
                host.set_deadline(Some(Instant::now() + wait));
            }
            host.render(&Ctx::new(rate, f0), l.len(), &mut Sum { left: l, right: r });
            if split {
                host.settle(Instant::now() + wait);
            }
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
/// core, and `third` the second lane of the three-core case. Each lane is
/// closed when its case is dropped, before the next case starts.
pub fn lanes(rate: u32, worker: Option<usize>, third: Option<usize>) -> Vec<Case> {
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
            grain_worst(
                "Grains worst: 5 pads, everything, split, one core",
                rate,
                5,
                Worst {
                    filters: true,
                    clean: true,
                    pitch_spray: true,
                    master: true,
                },
                Some(LaneConfig {
                    name: "mpfree-bench-ln".into(),
                    threaded: false,
                    ..LaneConfig::default()
                }),
            )
        }),
        measured(|| {
            grain_worst(
                "Grains worst: 5 pads, everything, split, 2 cores",
                rate,
                5,
                Worst {
                    filters: true,
                    clean: true,
                    pitch_spray: true,
                    master: true,
                },
                Some(config(false)),
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
        measured(|| {
            synth_tracks(
                "Synth tracks: 8 instruments, one core",
                rate,
                vec![],
                lane_wait(),
            )
        }),
        measured(|| {
            synth_tracks(
                "Synth tracks: 8 instruments, 2 cores",
                rate,
                vec![config(false)],
                lane_wait(),
            )
        }),
        measured(|| {
            // A worker left unpinned would start on this thread's core.
            let second = LaneConfig {
                name: "mpfree-bench-ln2".into(),
                core: third,
                ..config(false)
            };
            synth_tracks(
                "Synth tracks: 8 instruments, 3 cores",
                rate,
                vec![config(false), second],
                lane_wait(),
            )
        }),
        measured(|| {
            pad_synths(
                "Pad synths: 16 Instrument pads of Poly, one core",
                rate,
                16,
                vec![],
                lane_wait(),
            )
        }),
        measured(|| {
            pad_synths(
                "Pad synths: 16 Instrument pads of Poly, 2 cores",
                rate,
                16,
                vec![config(false)],
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
    e: Watched,
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
    // As the sampler case, and a lane on its own core may be late.
    let allow = Allow {
        steals: true,
        grains: true,
        late: lane.is_some(),
    };
    let half = |even: bool| {
        let e = sampler_engine(rate, false, SamplerPads::All, 12, |p| (p % 2 == 0) == even);
        Watched::new(e, name, allow)
    };
    let mut here = half(true);
    let there = Part::new(SamplerPart {
        e: half(false),
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
    let allow = Allow {
        steals: true,
        grains: true,
        late: lane.is_some(),
    };
    let mut e = Watched::new(
        sampler_engine(rate, false, SamplerPads::All, 24, |_| true),
        name,
        allow,
    );
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
    let allow = Allow {
        steals: true,
        grains: true,
        ..Allow::default()
    };
    let mut here = Watched::new(
        sampler_engine(rate, false, SamplerPads::All, 24, |_| true),
        name,
        allow,
    );
    let poly = Part::new(Chords::new(
        rate,
        Box::new(Poly::new(rate)),
        Poly::presets()[0].values,
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

/// `n` voices reading streamed sounds through `mpfree-stream`'s cursors at
/// `speed`, with `read` (disk streaming's P3, step 3): what the sampler's
/// streamed voices cost the sound thread, for its voice budget. Each
/// voice reads its own source (sounds lent from memory, so the case
/// counts the reads and not the disk), looping. At speed 1 from a whole
/// frame a cursor copies: that is 16 stems at the engine's rate.
fn stream_voices(rate: u32, n: usize, read: Read, speed: f64) -> Case {
    let cfg = StreamConfig {
        slots: 64,
        ..StreamConfig::default()
    };
    let (mut p, rx, tx) = StreamPlayer::new(cfg);
    let plan = StreamPlan {
        rate,
        ..StreamPlan::default()
    };
    let mut c = Coordinator::new(cfg, plan, rx, tx, Box::<Inline>::default());
    c.new_project();
    let frames = rate as usize * 4;
    let data: std::sync::Arc<[f32]> = bench_sound(frames, false).into();
    let mut voices: Vec<StreamCursor> = (0..n)
        .map(|i| {
            c.open_memory(i as u16, 2, data.clone()).expect("opens");
            let mut k = StreamCursor::new(i as u16, i as u16);
            k.set_read(read);
            k.set_speed(speed);
            k.set_loop(Some((1_000, frames as u64 - 1_000)));
            k.seek(1_000 + i as u64 * 997);
            k
        })
        .collect();
    let mut out = vec![0.0f32; MAX_BLOCK * 2];
    let read_name = match read {
        Read::Hermite => "Hermite",
        Read::Sinc8 => "band-limited",
        Read::Sinc16 => "band-limited 16",
    };
    Case {
        name: format!("Stream voices x{n} at {speed}x, {read_name}"),
        module: "sampler",
        what: "streamed voices read from the cache, looping",
        block: Box::new(move |_, l, r| {
            let frames = l.len();
            p.begin();
            l.fill(0.0);
            r.fill(0.0);
            for k in &mut voices {
                let o = &mut out[..frames * 2];
                k.play(&mut p, o);
                for (j, f) in o.chunks_exact(2).enumerate() {
                    l[j] += f[0];
                    r[j] += f[1];
                }
            }
            p.end(frames as u32);
            // The coordinator is another thread's work: kept, not stepped
            // (the notes the cursors post wait, or are counted as dropped).
            let _ = &c;
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
        // Past the first placement by cost (half a second), on two lanes.
        same_blocks(
            synth_tracks("one", 44_100, vec![], wait),
            synth_tracks("three", 44_100, vec![LaneConfig::default(); 2], wait),
            600,
        );
        same_blocks(
            pad_synths("one", 44_100, 16, vec![], wait),
            pad_synths("two", 44_100, 16, vec![LaneConfig::default()], wait),
            600,
        );
    }

    #[test]
    fn the_lane_cases_cover_one_core_and_two() {
        let mut cases = lanes(44_100, None, None);
        assert_eq!(cases.len(), 23);
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
        for up in [false, true] {
            let mut c = held_pads(44_100, 4, up, false);
            let (l, _) = run(&mut c, 100);
            assert!(l.iter().any(|v| v.abs() > 1e-4));
        }
        // The filtered pair: one plays on, the other rings out on silence
        // to exact zero (no denormals left behind).
        let mut c = filtered_pads(44_100, 4, false);
        let (l, _) = run(&mut c, 400);
        assert!(l.iter().any(|v| v.abs() > 1e-4));
        let mut c = filtered_pads(44_100, 4, true);
        let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
        (c.block)(0, &mut l, &mut r);
        assert!(l.iter().any(|v| v.abs() > 1e-4), "the noise first");
        for b in 1..400 {
            (c.block)(b, &mut l, &mut r);
        }
        let most = l.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(l.iter().all(|v| *v == 0.0), "then exact silence: {most:e}");
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
    fn the_grain_worst_cases_keep_the_whole_budget_playing() {
        // Sampler plan step 13: each case asks for more grains than the
        // budget, so near the budget's worth plays every block, shared out,
        // and its re-hits sound.
        let every = Worst {
            filters: true,
            clean: true,
            pitch_spray: true,
            master: true,
        };
        for (w, pads) in [(Worst::default(), 5), (every, 5), (Worst::default(), 64)] {
            let mut e = grain_worst_engine("test", 44_100, pads, w, None);
            let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
            let (mut least, mut most, mut sum) = (usize::MAX, 0, 0);
            for b in 0..700u64 {
                grain_worst_hits(&mut e, b, pads);
                e.render(&mut l, &mut r);
                if b >= 200 {
                    least = least.min(e.live_grains());
                    most = most.max(e.live_grains());
                    sum += e.live_grains();
                }
            }
            // About 200 of the 256 play (step 9 keeps room for a new hit
            // and counts the grains about to end), dipping as a re-hit's
            // cloud builds while the one let go rings out.
            let budget = mpfree_sampler::engine::GRAIN_BUDGET;
            let mean = sum / 500;
            assert!(most + RESERVE <= budget, "{pads} pads: {most}");
            assert!(mean * 4 >= budget * 3, "{pads} pads: {mean} on average");
            assert!(least * 2 >= budget, "{pads} pads: as few as {least}");
            let st = e.stats();
            assert!(st.crowded_frames > 0 && st.hits >= 5, "{st:?}");
            assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        }
    }

    #[test]
    fn a_sampler_case_fails_on_a_trade_it_may_not_make() {
        // Sampler plan step 10: the whole-budget grain point thins its
        // grains; allowed, it ends quietly, and not allowed, it fails.
        let run = |allow| {
            let mut e = Watched::new(grain_engine(44_100, 256), "test", allow);
            for pad in 0..4 {
                e.apply(Event::NoteOn { pad, velocity: 100 });
            }
            let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
            for _ in 0..200 {
                e.render(&mut l, &mut r);
            }
            e
        };
        let allowed = run(Allow {
            grains: true,
            ..Allow::default()
        });
        assert!(allowed.unexpected().is_empty());
        drop(allowed);
        let e = run(Allow::default());
        assert!(
            e.unexpected()
                .iter()
                .any(|what| what.starts_with("Grains thinned")),
            "{:?}",
            e.unexpected()
        );
        let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(e)));
        assert!(ended.is_err(), "it ended without failing");
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
    fn a_tap_and_a_detent_go_in_place_on_a_full_clip() {
        // A tap: out, in, out, in: the view follows the clip, in place.
        let mut tap = measured(|| step_edit(StepTouch::Tap, true));
        let (l, r) = run(&mut tap, 4);
        assert_eq!((l[0], r[0], l[1], r[1]), (8_192.0, 8_192.0, 4.0, 0.0));
        let (l, r) = run(&mut measured(|| step_edit(StepTouch::Tap, true)), 3);
        assert_eq!((l[0], r[0]), (8_191.0, 8_191.0));
        // The tap's edit alone leaves the view as it was.
        let (l, r) = run(&mut measured(|| step_edit(StepTouch::Tap, false)), 3);
        assert_eq!((l[0], r[0], l[1], r[1]), (8_191.0, 8_192.0, 0.0, 0.0));
        // A detent goes in place too, as in the app: its lock changes cost
        // the clip's one lock, not its 8,192 notes (refactor B3).
        let (l, r) = run(&mut measured(|| step_edit(StepTouch::Detent, true)), 5);
        assert_eq!((l[0], r[0], l[1], r[1]), (8_193.0, 8_193.0, 5.0, 0.0));
        // One take a turn.
        let mut d = step_edit(StepTouch::Detent, false);
        run(&mut d, 64);
    }

    #[test]
    fn the_grid_case_shows_every_step_of_the_full_clip() {
        let (l, r) = run(&mut measured(step_grid), 1);
        // 64 rows of 128 steps, and the whole line's 128.
        assert_eq!((l[0], r[0]), (8_320.0, 65.0));
    }

    #[test]
    fn the_big_song_cases_compile_edit_and_save_the_whole_song() {
        // The view holds each clip's 1,000 notes once, however many of
        // the 512 blocks play it (2.3; before it, 512,000).
        let (l, _) = run(&mut arrange_compile(), 1);
        assert_eq!(l[0], 32_000.0);
        // A tap puts a note in the one copy all 16 of the clip's blocks
        // play; the next block takes it out again.
        let (l, r) = run(&mut measured(arrange_step), 1);
        assert_eq!((l[0], r[0]), (1_001.0, 32_001.0));
        let (l, r) = run(&mut measured(arrange_step), 2);
        assert_eq!((l[0], r[0]), (1_000.0, 32_000.0));
        let (l, r) = run(&mut measured(arrange_save), 1);
        assert!(l[0] > 0.0);
        assert_eq!(r[0], 512.0);
        // A block of theirs is billions of instructions: few are counted.
        let c = arrange_save();
        assert_eq!(blocks_for(&c, 200), (1, 1));
        assert_eq!(blocks_for(&step_grid(), 200), (5, 20));
    }

    #[test]
    fn the_song_view_case_draws_every_frame_and_scrolls() {
        // Each frame draws (the playhead moved); every 8th scrolls a bar.
        let mut c = measured(song_view_frame);
        assert_eq!(c.module, SCREEN);
        let (l, r) = run(&mut c, 7);
        assert!(l[0] > 0.0);
        assert_eq!(r[0], 0.0);
        let (l, r) = run(&mut c, 9);
        assert!(l[0] > 0.0);
        assert_eq!(r[0], 1.0);
        assert!(off_the_audio_path(&c));
        assert_eq!(blocks_for(&c, 200), (5, 20));
    }

    #[test]
    fn the_piano_roll_cases_draw_every_note_and_move_as_asked() {
        let bar = 4.0 * PPQ as f32;
        for what in [
            RollFrame::Built,
            RollFrame::BuiltPressure,
            RollFrame::Plays,
            RollFrame::Scrolled,
            RollFrame::Zoomed,
            RollFrame::Switched,
        ] {
            let mut c = measured(|| roll_frame(what));
            assert_eq!(c.module, SCREEN);
            assert!(off_the_audio_path(&c));
            assert_eq!(blocks_for(&c, 200), (5, 20));
            // (painted, notes in the roll, view start, view zoom) a block.
            let seen: Vec<_> = (0..4)
                .map(|b| {
                    let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
                    (c.block)(b, &mut l, &mut r);
                    (l[0], r[0], l[1], r[1])
                })
                .collect();
            assert!(
                seen.iter().all(|s| s.0 > 0.0 && s.1 == 8_192.0),
                "{what:?}: {seen:?}"
            );
            let starts: Vec<f32> = seen.iter().map(|s| s.2).collect();
            let zooms: Vec<f32> = seen.iter().map(|s| s.3).collect();
            match what {
                RollFrame::Scrolled => {
                    assert_eq!(starts, [0.0, 240.0, 480.0, 720.0], "a sixteenth a frame");
                    assert!(zooms.iter().all(|&z| z == zooms[0]));
                }
                RollFrame::Zoomed => {
                    // A bar across, then wider each frame.
                    assert!(zooms.windows(2).all(|w| w[1] < w[0]), "{zooms:?}");
                    let roll_w = zooms[0] * bar;
                    assert!(roll_w > 600.0, "{roll_w}");
                }
                _ => {
                    assert!(starts.iter().all(|&s| s == 0.0), "{what:?}: {starts:?}");
                    assert!(zooms.iter().all(|&z| z == zooms[0]), "{what:?}: {zooms:?}");
                }
            }
        }
    }

    #[test]
    fn the_note_edit_cases_compile_a_clip_this_full() {
        for what in [NoteEditCase::VelocityAndUndo, NoteEditCase::TransposeAll] {
            let mut c = measured(|| note_edit(what));
            assert_eq!(c.module, CONTROL);
            assert!(off_the_audio_path(&c));
            // The transpose of every note counts a block, as the big song.
            let big_one = what == NoteEditCase::TransposeAll;
            assert_eq!(big(&c), big_one);
            assert_eq!(blocks_for(&c, 200), if big_one { (1, 1) } else { (5, 20) });
            let (l, r) = run(&mut c, 4);
            // Every note still there and playing; none went in place.
            assert_eq!((l[0], r[0]), (8_192.0, 8_192.0), "{what:?}");
            assert_eq!((l[1], r[1]), (0.0, 4.0), "{what:?}: in place, compiled");
        }
    }

    #[test]
    fn the_dense_case_plays_every_ratchet() {
        let mut c = measured(|| time_dense(44_100));
        let (l, _) = run(&mut c, 1_000);
        // 2.9 s at 128 BPM is about 6 beats: 16 tracks x 4 steps x 3
        // notes x 4 ratchets, on and off, plus 8 locks a step.
        assert!(l[0] > 6_000.0, "{}", l[0]);
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
