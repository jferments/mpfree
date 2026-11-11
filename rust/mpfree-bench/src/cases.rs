//! The workloads: each engine set up the way a busy song would use it, fed
//! block by block at the MPC's period. Setup allocates; the block function
//! must not.

use mpfree_fx::ids;
use mpfree_fx::{Chain, Chorus, Cloud, Compressor, Crush, Delay, DjFilter, Drive, Eq, Flanger, LoFi, MasterSafety, Mixer, MixerConfig, MultiFilter, Phaser, StripId};
use mpfree_grain::GrainNode;
use mpfree_looper::{Command as LooperCommand, Looper};
use mpfree_node::{Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_sampler::dsp::{Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;
use mpfree_synth::params::{POLYPHONY, UNISON, VOICE_MODE};
use mpfree_synth::{Poly, PRESETS};
use mpfree_play::{ArpHost, Launch, Out, PlayView, Player, Sink, ALL};
use mpfree_seq::{Clip, Note, Project as Song, Quantise, Speed, Track};
use mpfree_transport::time::{Swing, PPQ};
use mpfree_transport::{Block as TBlock, Command as T, Transport};
use mpfree_prof::alloc;

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
    fn unit(&mut self) -> f32 { (self.next() >> 9) as f32 / (1u32 << 23) as f32 * 2.0 - 1.0 }
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
            if *r == rate { return prog.clone(); }
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
    for &(id, v) in params { n.set_param(id, v); }
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

/// A synth playing `voices`-note chords that change every half second, so
/// note starts, releases and steals are all in the numbers.
fn synth(name: String, rate: u32, values: &[(u16, f32)], voices: usize) -> Case {
    let mut s = Poly::new(rate);
    s.prepare(rate, MAX_BLOCK);
    for &(id, v) in values { s.set_param(id, v); }
    s.set_param(POLYPHONY, voices as f32);
    let mut events: Vec<Timed> = Vec::with_capacity(2 * voices);
    let mut held: Vec<u8> = Vec::with_capacity(voices);
    let mut seed = Lcg(3);
    Case {
        name,
        module: "synth",
        what: "chords changing every 0.5 s",
        block: Box::new(move |b, l, r| {
            let len = l.len() as u64;
            let f0 = b * len;
            let half = u64::from(rate / 2);
            events.clear();
            // A chord change falls in this block.
            if f0 % half + len > half || f0 == 0 {
                let at = if f0 == 0 { 0 } else { (half - f0 % half) as u32 % len as u32 };
                for &note in held.iter() { events.push(Timed::new(at, NodeEvent::NoteOff { note })); }
                held.clear();
                let root = 36 + (seed.next() >> 28) as u8;
                for k in 0..voices {
                    let note = root + [0u8, 7, 12, 16, 19, 24, 28, 31][k % 8] + 12 * (k / 8) as u8;
                    held.push(note);
                    events.push(Timed::new(at, NodeEvent::NoteOn { note, velocity: 64 + (seed.next() >> 26) as u8 }));
                }
            }
            l.fill(0.0);
            r.fill(0.0);
            s.process(&Ctx::new(rate, f0), &events, l, r);
        }),
        mem_bytes: 0,
    }
}

/// The sampler as the pad player uses it: 16 one-shot, gated and looping
/// pads with filters and two layers, four granular pads, 24 voices, a hit
/// every 4 blocks (about 11 a second), and optionally the master grain
/// effect.
fn sampler(name: &str, rate: u32, master: bool) -> Case {
    let s = Source::new("noise", rate, 2, (0..rate as usize * 2).map(|i| ((i * 7919) % 2000) as f32 / 1000.0 - 1.0).collect())
        .expect("source");
    let mut p = Project::new();
    p.add_source(s.clone());
    for i in 0..16usize {
        let mut r = Region::whole(&s);
        r.mode = [PlayMode::OneShot, PlayMode::Gate, PlayMode::Loop, PlayMode::OneShot][i % 4];
        r.sustain_loop = Some(Loop { start: 5000, end: 9000, crossfade: 200, mode: LoopMode::Forward });
        r.tune_cents = (i as i32 - 8) * 50;
        let mut pad = Pad::new("p", Layer::new(r.clone()));
        pad.layers.push(Layer::new(r));
        pad.polyphony = 2;
        pad.filter = Filter { kind: FilterKind::LowPass, poles: 2, cutoff_hz: 2000.0, resonance: 0.3, env_octaves: 1.0 };
        p.set_pad(i, Some(pad), None).expect("pad");
    }
    for i in 16..20usize {
        let mut r = Region::whole(&s);
        r.mode = PlayMode::Granular;
        r.grain = GrainParams { spray: 0.6, size_ms: 300.0, density: 200.0, pitch_spray: 5.0, spread: 1.0, scan: 1.0, ..GrainParams::default() };
        let mut pad = Pad::new("g", Layer::new(r));
        pad.polyphony = 2;
        p.set_pad(i, Some(pad), None).expect("pad");
    }
    let mut e = Engine::new(rate, 24);
    if master {
        e.set_master_fx(Engine::make_master_fx(rate));
        p.set_master(MasterFx { mix: 0.5, grain: GrainParams { density: 200.0, size_ms: 150.0, spray: 0.3, ..MasterFx::default().grain } }, None)
            .expect("master");
    }
    e.set_view(EngineView::build(&p, rate));
    let mut seed = Lcg(11);
    Case {
        name: name.to_string(),
        module: "sampler",
        what: "a hit every 4 blocks over 20 pads, 24 voices",
        block: Box::new(move |b, l, r| {
            if b % 4 == 0 {
                let pad = (seed.next() >> 27) as u8 % 20;
                e.apply(Event::NoteOff { pad: (pad + 7) % 20 });
                e.apply(Event::NoteOn { pad, velocity: 40 + (seed.next() >> 25) as u8 % 88 });
            }
            l.fill(0.0);
            r.fill(0.0);
            e.render(l, r);
        }),
        mem_bytes: 0,
    }
}

/// The mixer at its default size (2 inputs, 32 pad channels, 16 tracks, 8
/// submixes, 4 returns), fed `tracks` tracks and `pads` pad channels of
/// program material, with a typical song's routing: pads into the drum
/// track, drums and bass into submixes, sends to two returns. With
/// [`Inserts::Song`], the strips carry a song's worth of effects too
/// (with the reverb at its Light quality: [`Inserts::SongLight`]).
/// The inserts a [`mixer`] case carries.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Inserts {
    None,
    Song,
    SongLight,
}

fn mixer(name: &str, rate: u32, tracks: usize, pads: usize, with: Inserts) -> Case {
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
        m.chain_mut(StripId::Bus(1)).unwrap().push(Box::new(Drive::new()));
        m.chain_mut(StripId::Bus(3)).unwrap().push(Box::new(Compressor::new()));
        m.chain_mut(StripId::Return(0)).unwrap().push(Box::new(Cloud::new()));
        if with == Inserts::SongLight {
            m.chain_mut(StripId::Return(0)).unwrap().set_param(0, ids::cloud::QUALITY, 1.0);
        }
        m.chain_mut(StripId::Return(1)).unwrap().push(Box::new(Delay::new()));
        m.chain_mut(StripId::Master).unwrap().push(Box::new(Eq::new()));
        m.chain_mut(StripId::Track(1)).unwrap().set_param(1, ids::compressor::SIDECHAIN, 1.0);
        m.set_key(StripId::Track(1), 1, Some(StripId::Track(0)));
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
        what: if inserts { "the routing below plus EQ and compressor on 6 tracks, drive and glue on submixes, reverb, delay, master EQ" } else { "a song's routing, no inserts (Master safety included)" },
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
            m.process(&Ctx::new(rate, f0), out_l, out_r);
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
        v.push(measured(|| synth(format!("Poly {}, 8 voices", preset.name), rate, preset.values, 8)));
    }
    v.push(measured(|| synth("Poly Init, 16 voices, unison 4".to_string(), rate, &[(UNISON, 4.0)], 16)));
    v.push(measured(|| sampler("Sampler, 20 pads", rate, false)));
    v.push(measured(|| sampler("Sampler, 20 pads + master grains", rate, true)));
    v.push(measured(|| effect("Multi filter (LP 12)", rate, Box::new(MultiFilter::new()), &[(ids::filter::CUTOFF, 800.0)])));
    v.push(measured(|| effect("Multi filter (LP 24, drive)", rate, Box::new(MultiFilter::new()),
                  &[(ids::filter::CUTOFF, 800.0), (ids::filter::SLOPE, 1.0), (ids::filter::DRIVE, 12.0)])));
    v.push(measured(|| effect("DJ filter", rate, Box::new(DjFilter::new()), &[(ids::dj_filter::POSITION, -40.0)])));
    v.push(measured(|| effect("Drive (Eco 2x)", rate, Box::new(Drive::new()), &[])));
    v.push(measured(|| effect("Drive (HQ 4x)", rate, Box::new(Drive::new()), &[(ids::drive::QUALITY, 1.0)])));
    v.push(measured(|| effect("Crush", rate, Box::new(Crush::new()), &[])));
    v.push(measured(|| effect("Delay", rate, Box::new(Delay::new()), &[])));
    v.push(measured(|| effect("Compressor", rate, Box::new(Compressor::new()), &[])));
    v.push(measured(|| effect("EQ (4 bands and both cuts)", rate, Box::new(Eq::new()), &[
        (ids::eq::LOW_SLOPE, 2.0), (ids::eq::HIGH_SLOPE, 2.0),
        (ids::eq::gain(0), 3.0), (ids::eq::gain(1), -4.0), (ids::eq::gain(2), 5.0), (ids::eq::gain(3), 2.0),
    ])));
    v.push(measured(|| effect("Chorus (2 voices)", rate, Box::new(Chorus::new()), &[])));
    v.push(measured(|| effect("Chorus (4 voices)", rate, Box::new(Chorus::new()), &[(ids::chorus::VOICES, 4.0)])));
    v.push(measured(|| effect("Flanger", rate, Box::new(Flanger::new()), &[])));
    v.push(measured(|| effect("Phaser (4 stages)", rate, Box::new(Phaser::new()), &[])));
    v.push(measured(|| effect("Phaser (12 stages)", rate, Box::new(Phaser::new()), &[(ids::phaser::STAGES, 4.0)])));
    v.push(measured(|| effect("Lo-fi", rate, Box::new(LoFi::new()), &[(ids::lofi::AMOUNT, 100.0)])));
    v.push(measured(|| effect("Master safety", rate, Box::new(MasterSafety::new()), &[])));
    for (k, c) in ["Plate", "Room", "Hall", "Cloud", "Echoes"].iter().enumerate() {
        v.push(measured(|| effect(&format!("Cloud reverb ({c})"), rate, Box::new(Cloud::new()),
                      &[(ids::cloud::CHARACTER, k as f32), (ids::cloud::MIX, 100.0)])));
    }
    v.push(measured(|| effect("Grains FX (dense)", rate, Box::new(GrainNode::new(5)),
                  &[(mpfree_grain::node::param::MIX, 100.0), (mpfree_grain::node::param::DENSITY, 200.0),
                    (mpfree_grain::node::param::SPRAY, 50.0)])));
    v.push(measured(|| mixer("Mixer, idle (63 strips, nothing playing)", rate, 0, 0, Inserts::None)));
    v.push(measured(|| mixer("Mixer, 12 tracks and 8 pads, routing only", rate, 12, 8, Inserts::None)));
    v.push(measured(|| mixer("Mixer, 16 tracks and 32 pads, routing only", rate, 16, 32, Inserts::None)));
    v.push(measured(|| mixer("Mixer, 12 tracks and 8 pads, with a song's inserts", rate, 12, 8, Inserts::Song)));
    v.push(measured(|| mixer("Mixer, 12 tracks and 8 pads, with a song's inserts, Light reverb", rate, 12, 8, Inserts::SongLight)));
    v.push(measured(|| chain(rate)));
    v.push(measured(|| time(rate, 8)));
    v.push(measured(|| looper(rate, 60)));
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
/// as in the synth cases) and the sampler at 1 to 32 voices held.
pub fn scaling(rate: u32) -> Vec<Scaling> {
    let mut v: Vec<Scaling> = PRESETS.iter().map(|p| {
        // Mono and legato presets play one voice whatever the polyphony.
        let mono = p.values.iter().any(|&(id, x)| id == VOICE_MODE && x != 0.0);
        let counts: &[usize] = if mono { &[1] } else { &[1, 2, 4, 8, 16] };
        Scaling {
            name: format!("Poly {}{}", p.name, if mono { " (mono)" } else { "" }),
            module: "synth",
            points: counts.iter().map(|&n| (n, synth(format!("Poly {} x{n}", p.name), rate, p.values, n))).collect(),
        }
    }).collect();
    v.push(Scaling {
        name: "Sampler, looping pads held".to_string(),
        module: "sampler",
        points: [1usize, 4, 8, 16, 32].iter().map(|&n| (n, held_pads(rate, n))).collect(),
    });
    v
}

/// The sampler with `n` looping, filtered, stereo pads held down, so exactly
/// `n` voices sound in every block.
fn held_pads(rate: u32, n: usize) -> Case {
    let s = Source::new("noise", rate, 2, (0..rate as usize * 2).map(|i| ((i * 7919) % 2000) as f32 / 1000.0 - 1.0).collect())
        .expect("source");
    let mut p = Project::new();
    p.add_source(s.clone());
    for i in 0..n {
        let mut r = Region::whole(&s);
        r.mode = PlayMode::Loop;
        r.sustain_loop = Some(Loop { start: 5000, end: 30000, crossfade: 200, mode: LoopMode::Forward });
        r.tune_cents = (i as i32 % 24 - 12) * 50;
        let mut pad = Pad::new("p", Layer::new(r));
        pad.polyphony = 1;
        pad.filter = Filter { kind: FilterKind::LowPass, poles: 2, cutoff_hz: 2000.0, resonance: 0.3, env_octaves: 1.0 };
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
                for pad in 0..n { e.apply(Event::NoteOn { pad: pad as u8, velocity: 100 }); }
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
    let preset = |name: &str| PRESETS.iter().find(|p| p.name == name).unwrap_or_else(|| panic!("preset {name}")).values;
    let mut parts = [
        sampler("sampler", rate, true),
        synth("synth 1".to_string(), rate, preset("Supersaw"), 8),
        synth("synth 2".to_string(), rate, preset("Sub Bass"), 4),
        mixer("mixer", rate, 12, 8, Inserts::Song),
        time(rate, 8),
        looper(rate, 60),
    ];
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = parts.iter().map(|_| (vec![0.0f32; MAX_BLOCK], vec![0.0f32; MAX_BLOCK])).collect();
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

/// Where the player's notes go in the time case: track 0 to the arp, the
/// rest counted (in the pad player they become sampler and synth events,
/// whose cost is in those cases).
struct TimeSink {
    arp: ArpHost,
    notes: u64,
}

impl Sink for TimeSink {
    fn send(&mut self, o: Out) {
        self.notes += 1;
        if o.dest == 0 || o.dest == ALL {
            self.arp.input(o.offset, o.event);
        }
    }
}

/// The time engine as a busy song uses it: the transport looping 4 bars at
/// 128 BPM, and `tracks` clips playing at once (16ths with 3-note chords,
/// one at 3:2 speed, one swung and quantised), the first through the arp
/// at 32nds over 2 octaves.
fn time(rate: u32, tracks: usize) -> Case {
    let ppq = i64::from(PPQ);
    let mut song = Song::new();
    let mut launches = Vec::new();
    for i in 0..tracks {
        let mut t = Track::new("t", i as u16);
        let mut c = Clip::new("c", 8 * ppq);
        c.notes = (0..32i64)
            .flat_map(|k| (0..3i64).map(move |n| Note { tick: k * ppq / 4, len: ppq / 4 - 10, note: (36 + 12 * (i as i64 % 3) + n * 4 + k % 5) as u8, velocity: 100 }))
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
        launches.push(Launch { track: i, slot: 0, origin: 0 });
    }
    let view = PlayView::compile(&song, &launches).expect("view");
    let mut t = Transport::new(rate, 128.0).expect("transport");
    t.command(0, T::SetLoop(Some((0, 16 * ppq))));
    t.command(0, T::Play);
    let mut engine = mpfree_arp::Engine::new(rate, 128.0);
    engine.set_settings(mpfree_arp::Settings { mode: mpfree_arp::Mode::Arp, rate: mpfree_arp::Rate::ThirtySecond, octaves: 2, ..mpfree_arp::Settings::default() });
    let mut sink = TimeSink { arp: ArpHost::new(engine), notes: 0 };
    let mut player = Player::new();
    let mut block = TBlock::new();
    let mut arp_notes = 0u64;
    Case {
        name: format!("Time: transport, {tracks} clips, arp"),
        module: "time",
        what: "16ths with chords on every track, looping 4 bars",
        block: Box::new(move |_, l, r| {
            t.process(l.len() as u32, &mut block);
            player.process(&view, &block, &mut sink);
            sink.arp.process(&block, &mut |_, _| arp_notes += 1);
            // Silent, except the notes so far in the first sample, so the
            // self-test can see that notes flowed.
            l.fill(0.0);
            r.fill(0.0);
            l[0] = (sink.notes + arp_notes) as f32;
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

fn input_frame(buf: &mut [f32], i: usize, l: f32, r: f32) {
    buf[2 * i] = l;
    buf[2 * i + 1] = r;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(c: &mut Case, blocks: u64) -> (Vec<f32>, Vec<f32>) {
        let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
        for b in 0..blocks {
            (c.block)(b, &mut l, &mut r);
        }
        (l, r)
    }

    #[test]
    fn the_time_case_plays_notes_through_the_arp() {
        let mut c = measured(|| time(44_100, 8));
        let (l, _) = run(&mut c, 3_000);
        // 8 s at 128 BPM is about 17 beats: 8 tracks x 3 notes x 4 per beat
        // as note-ons and offs, plus the arp's notes.
        assert!(l[0] > 1_000.0, "{}", l[0]);
        assert!(c.mem_bytes > 0 || !alloc::installed());
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
                energy += l.iter().chain(r.iter()).map(|v| f64::from(v.abs())).sum::<f64>();
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
        assert!(scaling(44_100).iter().all(|s| s.points.len() == 5 || (s.points.len() == 1 && s.name.ends_with("(mono)"))));
    }

    #[test]
    fn every_case_says_what_it_holds() {
        let cases = all(44_100);
        assert!(cases.iter().any(|c| c.module == "looper") && cases.iter().any(|c| c.module == "time"));
        if alloc::installed() {
            assert!(cases.iter().all(|c| c.mem_bytes > 0), "{:?}", cases.iter().map(|c| (&c.name, c.mem_bytes)).collect::<Vec<_>>());
        }
    }
}
