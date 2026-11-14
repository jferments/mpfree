//! Runs the bench's Keys, Drawbar, Squelch and Analog cases (and Poly
//! Init with 8 voices, for scale) on their own,
//! for counting their instructions as ARMv7 code under qemu, where the
//! whole bench (with Wavetable's start-up tables) would take hours.
//!
//! Each case plays what its `mpfree-bench` case plays (the same chords
//! changing every half second, or the same sixteenth-note line with
//! accents and slides, from the same seeds) at the MPC's period, 128
//! frames at 44.1 kHz: 50 blocks to warm up, then `INST_COST_BLOCKS`
//! (200 by default). `INST_COST_ONLY` keeps the case of that exact name.
//! It prints a hash of each case's output, so a change can be checked for
//! the same sound to the bit.
//!
//! Counting, as for the effects (docs/dev/design/fx.md): build for
//! `armv7-unknown-linux-gnueabihf`, run under
//! `qemu-arm -L /usr/arm-linux-gnueabihf -one-insn-per-tb -d exec,nochain`,
//! count the `Trace` lines at `INST_COST_BLOCKS=200` and at `0`, and divide
//! the difference by 200: instructions per block. `tools/inst_arm_cost.py`
//! does all of it.

use mpfree_node::{Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_synth::{analog_params, keys_params, params, Analog, Drawbar, Keys, Poly, Squelch};

const RATE: u32 = 44_100;
const FRAMES: usize = 128;
const WARM: u64 = 50;

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }
}

/// What a case plays.
enum Play {
    /// Chords of this many notes, changing every half second.
    Chords(usize),
    /// A sixteenth-note line at 125 BPM.
    Line,
}

struct Case {
    name: String,
    node: Box<dyn Node>,
    play: Play,
}

fn case(
    name: String,
    mut node: Box<dyn Node>,
    values: &[(u16, f32)],
    polyphony: Option<u16>,
    play: Play,
) -> Case {
    node.prepare(RATE, MAX_BLOCK);
    for &(id, v) in values {
        node.set_param(id, v);
    }
    if let (Some(id), Play::Chords(voices)) = (polyphony, &play) {
        node.set_param(id, *voices as f32);
    }
    Case { name, node, play }
}

fn values(presets: &'static [mpfree_synth::Preset], name: &str) -> &'static [(u16, f32)] {
    presets
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no preset {name}"))
        .values
}

/// The cases, in the bench's order and with its names, after Poly Init
/// with 8 voices for scale (the synth the device rounds measured).
fn cases() -> Vec<(String, Box<dyn Fn() -> Case>)> {
    let mut v: Vec<(String, Box<dyn Fn() -> Case>)> = Vec::new();
    v.push((
        "Poly Init, 8 voices".to_string(),
        Box::new(|| {
            case(
                "Poly Init, 8 voices".to_string(),
                Box::new(Poly::new(RATE)),
                &[],
                Some(params::POLYPHONY),
                Play::Chords(8),
            )
        }),
    ));
    for (name, voices) in [("Init", 8), ("Init", 16), ("Reed", 16), ("Dusty Keys", 16)] {
        let label = format!("Keys {name}, {voices} voices");
        let l = label.clone();
        v.push((
            label,
            Box::new(move || {
                case(
                    l.clone(),
                    Box::new(Keys::new(RATE)),
                    values(Keys::presets(), name),
                    Some(keys_params::POLYPHONY),
                    Play::Chords(voices),
                )
            }),
        ));
    }
    for (name, keys) in [
        ("Init", 8),
        ("Full Organ", 8),
        ("Gospel", 16),
        ("Church", 8),
    ] {
        let label = format!("Drawbar {name}, {keys} keys");
        let l = label.clone();
        v.push((
            label,
            Box::new(move || {
                case(
                    l.clone(),
                    Box::new(Drawbar::new(RATE)),
                    values(Drawbar::presets(), name),
                    None,
                    Play::Chords(keys),
                )
            }),
        ));
    }
    for name in ["Acid Line", "Square Acid", "Distorted"] {
        let label = format!("Squelch {name}, a line");
        let l = label.clone();
        v.push((
            label,
            Box::new(move || {
                case(
                    l.clone(),
                    Box::new(Squelch::new(RATE)),
                    values(Squelch::presets(), name),
                    None,
                    Play::Line,
                )
            }),
        ));
    }
    for (name, voices) in [
        ("Init", 8),
        ("Warm Pad", 8),
        ("Poly Brass", 8),
        ("Worn Cards", 16),
    ] {
        let label = format!("Analog {name}, {voices} voices");
        let l = label.clone();
        v.push((
            label,
            Box::new(move || {
                case(
                    l.clone(),
                    Box::new(Analog::new(RATE)),
                    values(Analog::presets(), name),
                    Some(analog_params::POLYPHONY),
                    Play::Chords(voices),
                )
            }),
        ));
    }
    v
}

/// The bench's chords (`Chords::block` in mpfree-bench's cases.rs).
fn chord_events(
    b: u64,
    voices: usize,
    held: &mut Vec<u8>,
    seed: &mut Lcg,
    events: &mut Vec<Timed>,
) {
    let len = FRAMES as u64;
    let f0 = b * len;
    let half = u64::from(RATE / 2);
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
        let root = 36 + (seed.next() >> 28) as u8;
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
}

/// The bench's line (`Line::block` in mpfree-bench's cases.rs).
fn line_events(
    b: u64,
    held: &mut Option<u8>,
    slide: &mut bool,
    seed: &mut Lcg,
    events: &mut Vec<Timed>,
) {
    let len = FRAMES as u64;
    let f0 = b * len;
    let step = u64::from(RATE) * 3 / 25;
    let gate = step * 7 / 12;
    let k = f0.div_ceil(step);
    let start = k * step;
    let j = if f0 >= gate {
        (f0 - gate).div_ceil(step)
    } else {
        0
    };
    let end = j * step + gate;
    if end < f0 + len && end < start && !*slide {
        if let Some(note) = held.take() {
            events.push(Timed::new((end - f0) as u32, NodeEvent::NoteOff { note }));
        }
    }
    if start < f0 + len {
        let at = (start - f0) as u32;
        let note = 36 + (seed.next() >> 28) as u8;
        let velocity = if k % 4 == 0 { 127 } else { 90 };
        events.push(Timed::new(at, NodeEvent::NoteOn { note, velocity }));
        if let Some(prev) = held.take() {
            if prev != note {
                events.push(Timed::new(at, NodeEvent::NoteOff { note: prev }));
            }
        }
        *held = Some(note);
        *slide = k % 3 == 2;
    }
    if end < f0 + len && end >= start && !*slide {
        if let Some(note) = held.take() {
            events.push(Timed::new((end - f0) as u32, NodeEvent::NoteOff { note }));
        }
    }
}

fn run(c: &mut Case, blocks: u64) -> u64 {
    let (mut l, mut r) = (vec![0.0f32; FRAMES], vec![0.0f32; FRAMES]);
    let mut events = Vec::with_capacity(64);
    let mut held = Vec::with_capacity(32);
    let mut mono = None;
    let mut slide = false;
    let mut seed = Lcg(match c.play {
        Play::Chords(_) => 3,
        Play::Line => 5,
    });
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in 0..WARM + blocks {
        events.clear();
        match c.play {
            Play::Chords(voices) => chord_events(b, voices, &mut held, &mut seed, &mut events),
            Play::Line => line_events(b, &mut mono, &mut slide, &mut seed, &mut events),
        }
        l.fill(0.0);
        r.fill(0.0);
        c.node
            .process(&Ctx::new(RATE, b * FRAMES as u64), &events, &mut l, &mut r);
        for k in 0..FRAMES {
            h = (h ^ u64::from(l[k].to_bits())).wrapping_mul(0x100_0000_01b3);
            h = (h ^ u64::from(r[k].to_bits())).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

fn main() {
    let only = std::env::var("INST_COST_ONLY").ok();
    let blocks: u64 = std::env::var("INST_COST_BLOCKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    for (name, make) in cases() {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        let mut c = make();
        let h = run(&mut c, blocks);
        println!("{:<28} {h:016x}", c.name);
    }
}
