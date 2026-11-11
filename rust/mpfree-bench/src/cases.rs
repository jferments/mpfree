//! The workloads: each engine set up the way a busy song would use it, fed
//! block by block at the MPC's period. Setup allocates; the block function
//! must not.

use mpfree_fx::ids;
use mpfree_fx::{Chain, Chorus, Cloud, Compressor, Crush, Delay, DjFilter, Drive, Eq, Flanger, LoFi, MasterSafety, Mixer, MixerConfig, MultiFilter, Phaser, StripId};
use mpfree_grain::GrainNode;
use mpfree_node::{Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_sampler::dsp::{Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;
use mpfree_synth::params::{POLYPHONY, UNISON};
use mpfree_synth::{Poly, PRESETS};

pub type Block = Box<dyn FnMut(u64, &mut [f32], &mut [f32])>;

pub struct Case {
    pub name: String,
    /// The budget it counts against (see `mpfree_prof::budget`).
    pub module: &'static str,
    /// What it plays, for the report.
    pub what: &'static str,
    pub block: Block,
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

fn effect(name: &str, rate: u32, mut n: Box<dyn Node>, params: &[(u16, f32)]) -> Case {
    n.prepare(rate, MAX_BLOCK);
    for &(id, v) in params { n.set_param(id, v); }
    n.reset();
    let mut seed = Lcg(7);
    Case {
        name: name.to_string(),
        module: "fx",
        what: "tone and noise bursts every 0.5 s",
        block: Box::new(move |b, l, r| {
            let f0 = b * l.len() as u64;
            input(rate, f0, &mut seed, l, r);
            n.process(&Ctx::new(rate, f0), &[], l, r);
        }),
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
    }
}

/// The mixer at its default size (2 inputs, 32 pad channels, 16 tracks, 8
/// submixes, 4 returns), fed `tracks` tracks and `pads` pad channels of
/// program material, with a typical song's routing: pads into the drum
/// track, drums and bass into submixes, sends to two returns. With
/// `inserts`, the strips carry a song's worth of effects too.
fn mixer(name: &str, rate: u32, tracks: usize, pads: usize, inserts: bool) -> Case {
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
    }
}

/// Every case, at `rate`.
pub fn all(rate: u32) -> Vec<Case> {
    let mut v = Vec::new();
    for preset in PRESETS {
        v.push(synth(format!("Poly {}, 8 voices", preset.name), rate, preset.values, 8));
    }
    v.push(synth("Poly Init, 16 voices, unison 4".to_string(), rate, &[(UNISON, 4.0)], 16));
    v.push(sampler("Sampler, 20 pads", rate, false));
    v.push(sampler("Sampler, 20 pads + master grains", rate, true));
    v.push(effect("Multi filter (LP 12)", rate, Box::new(MultiFilter::new()), &[(ids::filter::CUTOFF, 800.0)]));
    v.push(effect("Multi filter (LP 24, drive)", rate, Box::new(MultiFilter::new()),
                  &[(ids::filter::CUTOFF, 800.0), (ids::filter::SLOPE, 1.0), (ids::filter::DRIVE, 12.0)]));
    v.push(effect("DJ filter", rate, Box::new(DjFilter::new()), &[(ids::dj_filter::POSITION, -40.0)]));
    v.push(effect("Drive (Eco 2x)", rate, Box::new(Drive::new()), &[]));
    v.push(effect("Drive (HQ 4x)", rate, Box::new(Drive::new()), &[(ids::drive::QUALITY, 1.0)]));
    v.push(effect("Crush", rate, Box::new(Crush::new()), &[]));
    v.push(effect("Delay", rate, Box::new(Delay::new()), &[]));
    v.push(effect("Compressor", rate, Box::new(Compressor::new()), &[]));
    v.push(effect("EQ (4 bands and both cuts)", rate, Box::new(Eq::new()), &[
        (ids::eq::LOW_SLOPE, 2.0), (ids::eq::HIGH_SLOPE, 2.0),
        (ids::eq::gain(0), 3.0), (ids::eq::gain(1), -4.0), (ids::eq::gain(2), 5.0), (ids::eq::gain(3), 2.0),
    ]));
    v.push(effect("Chorus (2 voices)", rate, Box::new(Chorus::new()), &[]));
    v.push(effect("Chorus (4 voices)", rate, Box::new(Chorus::new()), &[(ids::chorus::VOICES, 4.0)]));
    v.push(effect("Flanger", rate, Box::new(Flanger::new()), &[]));
    v.push(effect("Phaser (4 stages)", rate, Box::new(Phaser::new()), &[]));
    v.push(effect("Phaser (12 stages)", rate, Box::new(Phaser::new()), &[(ids::phaser::STAGES, 4.0)]));
    v.push(effect("Lo-fi", rate, Box::new(LoFi::new()), &[(ids::lofi::AMOUNT, 100.0)]));
    v.push(effect("Master safety", rate, Box::new(MasterSafety::new()), &[]));
    for (k, c) in ["Plate", "Room", "Hall", "Cloud", "Echoes"].iter().enumerate() {
        v.push(effect(&format!("Cloud reverb ({c})"), rate, Box::new(Cloud::new()),
                      &[(ids::cloud::CHARACTER, k as f32), (ids::cloud::MIX, 100.0)]));
    }
    v.push(effect("Grains FX (dense)", rate, Box::new(GrainNode::new(5)),
                  &[(mpfree_grain::node::param::MIX, 100.0), (mpfree_grain::node::param::DENSITY, 200.0),
                    (mpfree_grain::node::param::SPRAY, 50.0)]));
    v.push(mixer("Mixer, idle (63 strips, nothing playing)", rate, 0, 0, false));
    v.push(mixer("Mixer, 12 tracks and 8 pads, routing only", rate, 12, 8, false));
    v.push(mixer("Mixer, 16 tracks and 32 pads, routing only", rate, 16, 32, false));
    v.push(mixer("Mixer, 12 tracks and 8 pads, with a song's inserts", rate, 12, 8, true));
    let mut chain = Chain::new();
    chain.prepare(rate, MAX_BLOCK);
    chain.push(Box::new(Drive::new()));
    chain.push(Box::new(MultiFilter::new()));
    chain.push(Box::new(Delay::new()));
    chain.push(Box::new(Compressor::new()));
    chain.set_param(1, ids::filter::CUTOFF, 800.0);
    let mut seed = Lcg(5);
    v.push(Case {
        name: "Chain: drive, filter, delay, comp".to_string(),
        module: "fx",
        what: "one track's inserts",
        block: Box::new(move |b, l, r| {
            let f0 = b * l.len() as u64;
            input(rate, f0, &mut seed, l, r);
            chain.process(&Ctx::new(rate, f0), l, r);
        }),
    });
    v
}
