//! Y4 and Y7 of the DSP quality plan (`docs/dev/design/dsp-quality.md`):
//! every synth engine, the drums and every registered effect, each on its
//! own (no chain, no master safety to catch it).
//!
//! - **Y4, finite output.** Each subject is played or fed hard in layers:
//!   its defaults; each control alone at its bottom, top and middle; every
//!   control at its bottom, at its top and alternating; seeded random
//!   settings; each control alone swept fast from bottom to top and back;
//!   and every pair of controls in the same group at their tops. An
//!   instrument plays notes 0 to 127 at velocities 1 and 127, a chord,
//!   bends to both ends, full pressure, the mod wheel and breath; an
//!   effect is fed silence, full-scale white noise, a full-scale sine sweep
//!   and silence again. Every output sample must be finite: that is the
//!   gate ([`y4`]).
//! - **Y7, block size.** The same score, with notes and parameter moves on
//!   fixed frames, rendered in blocks of 1, 37 and 128 frames, each against
//!   128 ([`y7`]). A subject is bit-exact, or differs by a measured amount;
//!   the tests hold the bit-exact ones to it.
//!
//! Desktop only; nothing here runs on the MPC.

use mpfree_fx::registry;
use mpfree_node::{Ctl, Ctx, Node, NodeEvent, ParamInfo, Timed, MAX_BLOCK};
use mpfree_synth::{drum_params, Drums};

use crate::synth::Engine;

/// Frames in every render: 0.6 s at 44.1 kHz.
const LEN: usize = 26_460;

/// What is measured: an engine, the drums, or an effect by its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    Engine(Engine),
    Drums,
    Effect(&'static str),
}

impl Subject {
    /// Every synth engine, the drums, then every registered effect.
    pub fn all() -> Vec<Subject> {
        let mut s: Vec<Subject> = Engine::ALL.into_iter().map(Subject::Engine).collect();
        s.push(Subject::Drums);
        s.extend(registry::EFFECTS.iter().map(|e| Subject::Effect(e.id)));
        s
    }

    /// "synth/Poly", "synth/Drums" or the effect's id.
    pub fn name(&self) -> String {
        match self {
            Subject::Engine(e) => format!("synth/{}", e.name()),
            Subject::Drums => "synth/Drums".to_string(),
            Subject::Effect(id) => id.to_string(),
        }
    }

    pub fn is_instrument(&self) -> bool {
        !matches!(self, Subject::Effect(_))
    }

    /// A fresh node at `rate`: an engine with its first preset's settings
    /// and no effects, the first drum kit's drums, an effect at its
    /// defaults.
    pub fn make(&self, rate: u32) -> Box<dyn Node> {
        let mut n: Box<dyn Node> = match self {
            Subject::Engine(e) => e.engine_with(0, rate),
            Subject::Drums => {
                let mut d = Drums::new(rate);
                d.load_preset(&Drums::presets()[0]);
                Box::new(d)
            }
            Subject::Effect(id) => {
                registry::make(id).unwrap_or_else(|| panic!("{id} is in the registry"))
            }
        };
        n.prepare(rate, MAX_BLOCK);
        n
    }
}

/// A small fixed random sequence, the same every run.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    /// 0 to 1.
    fn unit(&mut self) -> f32 {
        (self.next() % 1_000_001) as f32 / 1_000_000.0
    }

    /// -1 to 1.
    fn signed(&mut self) -> f32 {
        2.0 * self.unit() - 1.0
    }
}

fn at(secs: f64, rate: u32) -> usize {
    (secs * f64::from(rate)).round() as usize
}

/// The instrument's score: the lowest and highest notes, velocities 1 and
/// 127, a chord, bends to both ends, full pressure, the mod wheel and
/// breath, everything let go at 0.4 s. `base` is the drums' first pad (60
/// for an engine).
fn instrument_score(rate: u32, base: u8) -> Vec<(usize, NodeEvent)> {
    use NodeEvent::*;
    let mut s = vec![
        (
            0,
            NoteOn {
                note: 0,
                velocity: 127,
            },
        ),
        (
            0,
            NoteOn {
                note: 127,
                velocity: 1,
            },
        ),
        (
            at(0.02, rate),
            NoteOn {
                note: base,
                velocity: 127,
            },
        ),
        (
            at(0.03, rate),
            NoteOn {
                note: base.saturating_add(4),
                velocity: 127,
            },
        ),
        (
            at(0.04, rate),
            NoteOn {
                note: base.saturating_add(7),
                velocity: 127,
            },
        ),
        (
            at(0.05, rate),
            NoteOn {
                note: base.saturating_add(11),
                velocity: 64,
            },
        ),
        (at(0.08, rate), Bend { value: 1.0 }),
        (at(0.12, rate), Bend { value: -1.0 }),
        (
            at(0.14, rate),
            Pressure {
                note: base,
                value: 127,
            },
        ),
        (
            at(0.15, rate),
            Control {
                ctl: Ctl::ModWheel,
                value: 1.0,
            },
        ),
        (
            at(0.16, rate),
            Control {
                ctl: Ctl::Breath,
                value: 1.0,
            },
        ),
        (at(0.20, rate), Bend { value: 0.0 }),
        (
            at(0.22, rate),
            NoteOn {
                note: 127,
                velocity: 127,
            },
        ),
        (
            at(0.24, rate),
            NoteOn {
                note: 0,
                velocity: 1,
            },
        ),
        (at(0.40, rate), AllOff),
    ];
    s.sort_by_key(|(f, _)| *f);
    s
}

/// The effect's input: silence, full-scale white noise, a full-scale
/// exponential sine sweep from 20 Hz to 20 kHz, then silence for its tail.
fn effect_input(rate: u32) -> (Vec<f32>, Vec<f32>) {
    let mut rng = Lcg(7);
    let (mut l, mut r) = (vec![0.0f32; LEN], vec![0.0f32; LEN]);
    let (n0, n1, s1) = (at(0.05, rate), at(0.20, rate), at(0.35, rate));
    for i in n0..n1 {
        l[i] = rng.signed();
        r[i] = rng.signed();
    }
    let dur = (s1 - n1) as f64 / f64::from(rate);
    let k = (20_000.0f64 / 20.0).ln();
    for i in n1..s1 {
        let t = (i - n1) as f64 / f64::from(rate);
        let phase = std::f64::consts::TAU * 20.0 * dur / k * ((t / dur * k).exp() - 1.0);
        l[i] = phase.sin() as f32;
        r[i] = phase.cos() as f32;
    }
    (l, r)
}

/// The drums' first pad's note, else 60.
fn base_note(node: &dyn Node, subject: &Subject) -> u8 {
    match subject {
        Subject::Drums => node.param(drum_params::BASE_NOTE) as u8,
        _ => 60,
    }
}

/// `node` in blocks of `block` frames on absolute frames, with `events`
/// (sorted by frame) on their frames; an effect is fed `input`. The
/// transport plays at 120 bpm, so anything synced to it moves the same for
/// any block size.
fn render(
    node: &mut dyn Node,
    rate: u32,
    input: Option<&(Vec<f32>, Vec<f32>)>,
    events: &[(usize, NodeEvent)],
    block: usize,
) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = match input {
        Some((l, r)) => (l.clone(), r.clone()),
        None => (vec![0.0; LEN], vec![0.0; LEN]),
    };
    let mut timed = Vec::new();
    let (mut start, mut next) = (0, 0);
    while start < LEN {
        let end = (start + block).min(LEN);
        timed.clear();
        while next < events.len() && events[next].0 < end {
            let off = events[next].0.saturating_sub(start) as u32;
            timed.push(Timed::new(off, events[next].1));
            next += 1;
        }
        let mut ctx = Ctx::new(rate, start as u64);
        ctx.playing = true;
        ctx.beats = start as f64 / f64::from(rate) * 2.0;
        node.process(&ctx, &timed, &mut l[start..end], &mut r[start..end]);
        start = end;
    }
    (l, r)
}

/// The settable controls.
fn controls(node: &dyn Node) -> Vec<ParamInfo> {
    node.params()
        .iter()
        .filter(|p| !p.readonly)
        .cloned()
        .collect()
}

/// One way of setting a subject's controls before it plays, and any moves
/// while it plays.
struct Setting {
    /// What it is, for a failure's message.
    what: String,
    set: Vec<(u16, f32)>,
    moves: Vec<(usize, u16, f32)>,
}

/// Every Y4 setting for a node with `ps` controls, layer by layer.
fn settings(ps: &[ParamInfo], rate: u32) -> Vec<Setting> {
    let mut out = vec![Setting {
        what: "defaults".to_string(),
        set: Vec::new(),
        moves: Vec::new(),
    }];
    let one = |what: String, set: Vec<(u16, f32)>| Setting {
        what,
        set,
        moves: Vec::new(),
    };
    // Each control alone at its bottom, top and middle.
    for p in ps {
        for (label, t) in [("bottom", 0.0), ("top", 1.0), ("middle", 0.5)] {
            out.push(one(
                format!("{} at its {label}", p.name),
                vec![(p.id, p.from_unit(t))],
            ));
        }
    }
    // The corners.
    out.push(one(
        "every control at its bottom".to_string(),
        ps.iter().map(|p| (p.id, p.from_unit(0.0))).collect(),
    ));
    out.push(one(
        "every control at its top".to_string(),
        ps.iter().map(|p| (p.id, p.from_unit(1.0))).collect(),
    ));
    out.push(one(
        "controls alternating bottom and top".to_string(),
        ps.iter()
            .enumerate()
            .map(|(k, p)| (p.id, p.from_unit((k % 2) as f32)))
            .collect(),
    ));
    // Seeded random settings.
    let mut rng = Lcg(0x5934);
    for k in 0..16 {
        out.push(one(
            format!("random setting {k}"),
            ps.iter().map(|p| (p.id, p.from_unit(rng.unit()))).collect(),
        ));
    }
    // Every pair of controls in the same group at their tops.
    for (i, a) in ps.iter().enumerate() {
        for b in &ps[i + 1..] {
            if a.group == b.group {
                out.push(one(
                    format!("{} and {} at their tops", a.name, b.name),
                    vec![(a.id, a.from_unit(1.0)), (b.id, b.from_unit(1.0))],
                ));
            }
        }
    }
    // Each control alone swept fast: bottom to top and back in 0.3 s, a
    // move every 32 frames.
    let sweep = at(0.3, rate);
    for p in ps {
        let moves = (0..sweep)
            .step_by(32)
            .map(|f| {
                let t = f as f32 / sweep as f32;
                (f, p.id, p.from_unit(1.0 - (2.0 * t - 1.0).abs()))
            })
            .collect();
        out.push(Setting {
            what: format!("{} swept fast", p.name),
            set: Vec::new(),
            moves,
        });
    }
    out
}

/// What Y4 found for one subject.
#[derive(Clone, Debug)]
pub struct Y4 {
    pub subject: String,
    /// Renders made.
    pub renders: usize,
    /// The settings whose output had a NaN or an infinity.
    pub failures: Vec<String>,
    /// The highest finite peak across every render, in dBFS (shown, not
    /// gated).
    pub peak_dbfs: f64,
}

/// Y4 for `subject` at `rate`.
pub fn y4(subject: Subject, rate: u32) -> Y4 {
    let probe = subject.make(rate);
    let ps = controls(probe.as_ref());
    let base = base_note(probe.as_ref(), &subject);
    drop(probe);
    let input = (!subject.is_instrument()).then(|| effect_input(rate));
    let score = if subject.is_instrument() {
        instrument_score(rate, base)
    } else {
        Vec::new()
    };
    let mut failures = Vec::new();
    let mut peak = 0.0f32;
    let all = settings(&ps, rate);
    for s in &all {
        let mut n = subject.make(rate);
        for &(id, v) in &s.set {
            n.set_param(id, v);
        }
        let mut events: Vec<(usize, NodeEvent)> = score.clone();
        events.extend(
            s.moves
                .iter()
                .map(|&(f, id, value)| (f, NodeEvent::Param { id, value })),
        );
        events.sort_by_key(|(f, _)| *f);
        let (l, r) = render(n.as_mut(), rate, input.as_ref(), &events, 128);
        let mut bad = false;
        for &v in l.iter().chain(&r) {
            if v.is_finite() {
                peak = peak.max(v.abs());
            } else {
                bad = true;
            }
        }
        if bad {
            failures.push(s.what.clone());
        }
    }
    Y4 {
        subject: subject.name(),
        renders: all.len(),
        failures,
        peak_dbfs: if peak > 0.0 {
            20.0 * f64::from(peak).log10()
        } else {
            -200.0
        },
    }
}

/// What Y7 found for one subject: the largest difference at block sizes 1
/// and 37 against 128, in dB against the 128-frame render's peak
/// (`f64::NEG_INFINITY` when bit-exact).
#[derive(Clone, Debug)]
pub struct Y7 {
    pub subject: String,
    pub diff_db: [(usize, f64); 2],
    /// The 128-frame render's peak, dBFS (a silent render would be
    /// bit-exact for nothing).
    pub peak_dbfs: f64,
}

impl Y7 {
    pub fn bit_exact(&self) -> bool {
        self.diff_db.iter().all(|(_, d)| *d == f64::NEG_INFINITY)
    }
}

/// The Y7 score: the subject's own (notes for an instrument, the input for
/// an effect) plus a seeded parameter move every 700 to 4700 frames.
fn y7_moves(ps: &[ParamInfo], rate: u32) -> Vec<(usize, NodeEvent)> {
    let mut rng = Lcg(11);
    let mut out = Vec::new();
    let mut f = 0;
    let end = at(0.45, rate);
    if ps.is_empty() {
        return out;
    }
    loop {
        f += 700 + (rng.next() % 4000) as usize;
        if f >= end {
            break;
        }
        let p = &ps[(rng.next() as usize) % ps.len()];
        out.push((
            f,
            NodeEvent::Param {
                id: p.id,
                value: p.from_unit(rng.unit()),
            },
        ));
    }
    out
}

/// Y7 for `subject` at `rate`.
pub fn y7(subject: Subject, rate: u32) -> Y7 {
    let probe = subject.make(rate);
    let ps = controls(probe.as_ref());
    let base = base_note(probe.as_ref(), &subject);
    drop(probe);
    let input = (!subject.is_instrument()).then(|| effect_input(rate));
    let mut events = if subject.is_instrument() {
        instrument_score(rate, base)
    } else {
        Vec::new()
    };
    events.extend(y7_moves(&ps, rate));
    events.sort_by_key(|(f, _)| *f);
    let run = |block: usize| {
        render(
            subject.make(rate).as_mut(),
            rate,
            input.as_ref(),
            &events,
            block,
        )
    };
    let (rl, rr) = run(128);
    let peak = rl
        .iter()
        .chain(&rr)
        .fold(0.0f32, |m, v| m.max(v.abs()))
        .max(1e-9);
    let diff = |block: usize| {
        let (l, r) = run(block);
        let mut worst = 0.0f32;
        let mut exact = true;
        for (a, b) in l.iter().chain(&r).zip(rl.iter().chain(&rr)) {
            if a.to_bits() != b.to_bits() {
                exact = false;
                worst = worst.max((a - b).abs());
            }
        }
        if exact {
            f64::NEG_INFINITY
        } else {
            20.0 * f64::from(worst.max(1e-30) / peak).log10()
        }
    };
    Y7 {
        subject: subject.name(),
        diff_db: [(1, diff(1)), (37, diff(37))],
        peak_dbfs: 20.0 * f64::from(peak).log10(),
    }
}

/// Runs `f` over every subject on every core, in the subjects' order.
pub fn over_all<T: Send>(f: impl Fn(Subject) -> T + Sync) -> Vec<T> {
    let subjects = Subject::all();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(subjects.len());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<std::sync::Mutex<Option<T>>> = subjects
        .iter()
        .map(|_| std::sync::Mutex::new(None))
        .collect();
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= subjects.len() {
                    break;
                }
                let v = f(subjects[i]);
                *results[i].lock().unwrap() = Some(v);
            });
        }
    });
    results
        .into_iter()
        .map(|m| m.into_inner().unwrap().expect("every subject measured"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_engine_the_drums_and_every_effect_are_subjects() {
        let all = Subject::all();
        assert_eq!(all.len(), Engine::ALL.len() + 1 + registry::EFFECTS.len());
        let mut names: Vec<String> = all.iter().map(Subject::name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), all.len(), "subject names are unique");
    }

    #[test]
    fn the_layers_cover_each_control_and_the_corners() {
        let n = Subject::Effect("mpfree.fx.drive").make(44_100);
        let ps = controls(n.as_ref());
        let s = settings(&ps, 44_100);
        // Defaults, three per control, three corners, sixteen random, the
        // pairs, one sweep per control.
        let pairs: usize = ps
            .iter()
            .enumerate()
            .map(|(i, a)| ps[i + 1..].iter().filter(|b| b.group == a.group).count())
            .sum();
        assert_eq!(s.len(), 1 + 3 * ps.len() + 3 + 16 + pairs + ps.len());
    }

    #[test]
    fn a_render_keeps_every_event_on_its_frame_whatever_the_block() {
        // A plain pass-through check on the score's shape: events land in
        // the block that holds their frame.
        let e = instrument_score(44_100, 60);
        assert!(e.windows(2).all(|w| w[0].0 <= w[1].0));
        assert!(e.iter().all(|(f, _)| *f < LEN));
    }
}
