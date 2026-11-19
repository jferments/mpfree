//! The synths in the lab (synth library stage 2): the measure-only
//! baseline of every `mpfree-synth` engine and factory preset against the
//! targets of the research pages (`docs/dev/research/`), with a ratchet of
//! its own (`tests/synth.rs`, `quality/synth.tsv`) that keeps every number
//! from getting worse. Design and the list of measures:
//! `docs/dev/design/synth-quality.md`.
//!
//! - **Every preset** of the keyboard engines (L rows): a phrase's
//!   loudness, mono fold-down loss, stereo correlation, DC and tail; an
//!   8-note chord's peak; level and tone against velocity, with the
//!   square-law fit of Dannenberg's curve; pressure; level across the keys;
//!   a 6-note chord against the power sum of its notes; the attack.
//! - **Bass presets** (B rows): the start of a note, small-speaker
//!   audibility, the low end's correlation. **Keys and Pluck** (K rows):
//!   brightness against velocity, decay against the Decay label and its
//!   law across the keys, tremolo rate.
//! - **Each engine's plain patch**: tuning from C1 to C7, aliasing at C5
//!   to C7 (with drive where an engine has it).
//! - **Poly's voice**: glide, envelope times, the filter against its analog
//!   prototype and where its resonance peaks.
//! - **FM**: a pair's sidebands against the Bessel functions, feedback and
//!   the level across algorithms.
//! - **Drums**: each kit's pads, decay against the label, tone against
//!   velocity, level against the kick, a repeated hit.
//!
//! Cost per voice is the bench's (`mpfree-bench --afford`), not a ratchet
//! row. Desktop only; nothing here runs on the MPC.

use std::f64::consts::{PI, TAU};

use mpfree_instruments::sound::has_design;
use mpfree_instruments::{Design, Sound};
use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::{
    bass_params as bp, choir_params as chp, drum_params, Analog, Bass, Choir, Drawbar, Drums, Fm,
    Keys, Pluck, Poly, Preset, Squelch, Strings, Wavetable,
};

use crate::fft::{hann_f64, power_spectrum};
use crate::loudness;
use crate::quality::{Better, Row};
use crate::wav::Stereo;

/// Frames per block, as the MPC's period.
const BLOCK: usize = 128;

/// The ratchet file's header.
const HEADER: &str =
    "# The synths' quality ratchet: every number may stay or get better, never worse.\n\
# Written by `MPFREE_QUALITY_BLESS=1 cargo test --release --locked -p mpfree-fxlab --test synth`;\n\
# see docs/dev/design/synth-quality.md. key<TAB>value\n";

// ---------------------------------------------------------------------------
// The engines.

/// A keyboard engine of `mpfree-synth` (Drums are measured on their own).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Poly,
    Analog,
    Squelch,
    Fm,
    Wavetable,
    Keys,
    Drawbar,
    Pluck,
    Strings,
    Bass,
    Choir,
}

impl Engine {
    pub const ALL: [Engine; 11] = [
        Engine::Poly,
        Engine::Analog,
        Engine::Squelch,
        Engine::Fm,
        Engine::Wavetable,
        Engine::Keys,
        Engine::Drawbar,
        Engine::Pluck,
        Engine::Strings,
        Engine::Bass,
        Engine::Choir,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Engine::Poly => "Poly",
            Engine::Analog => "Analog",
            Engine::Squelch => "Squelch",
            Engine::Fm => "FM",
            Engine::Wavetable => "Wavetable",
            Engine::Keys => "Keys",
            Engine::Drawbar => "Drawbar",
            Engine::Pluck => "Pluck",
            Engine::Strings => "Strings",
            Engine::Bass => "Bass",
            Engine::Choir => "Choir",
        }
    }

    /// The factory presets' names, in order.
    pub fn presets(self) -> Vec<String> {
        fn names(p: &[Preset]) -> Vec<String> {
            p.iter().map(|p| p.name.to_string()).collect()
        }
        match self {
            Engine::Poly => names(Poly::presets()),
            Engine::Analog => names(Analog::presets()),
            Engine::Squelch => names(Squelch::presets()),
            Engine::Fm => Fm::presets().into_iter().map(|p| p.name).collect(),
            Engine::Wavetable => names(Wavetable::presets()),
            Engine::Keys => names(Keys::presets()),
            Engine::Drawbar => names(Drawbar::presets()),
            Engine::Pluck => names(Pluck::presets()),
            Engine::Strings => names(Strings::presets()),
            Engine::Bass => names(Bass::presets()),
            Engine::Choir => names(Choir::presets()),
        }
    }

    /// A fresh engine at `rate` with factory preset `preset` loaded: a
    /// sound with its own effects and macros where the preset brings them
    /// (`mpfree_instruments::Sound`), so the lab measures what the pads
    /// play.
    pub fn make(self, preset: usize, rate: u32) -> Box<dyn Node> {
        let node = self.engine_with(preset, rate);
        let Some(text) = self.text(preset).filter(|t| has_design(t)) else {
            return node;
        };
        let mut s = Sound::new(node, &Design::default()).expect("a plain sound builds");
        s.prepare_effects(rate, mpfree_node::MAX_BLOCK);
        s.load_text(text)
            .expect("every factory sound loads (mpfree-instruments' tests)");
        Box::new(s)
    }

    /// Factory preset `preset`'s text (None for FM, whose presets are
    /// built in code).
    fn text(self, preset: usize) -> Option<&'static str> {
        let p = match self {
            Engine::Poly => Poly::presets(),
            Engine::Analog => Analog::presets(),
            Engine::Squelch => Squelch::presets(),
            Engine::Fm => return None,
            Engine::Wavetable => Wavetable::presets(),
            Engine::Keys => Keys::presets(),
            Engine::Drawbar => Drawbar::presets(),
            Engine::Pluck => Pluck::presets(),
            Engine::Strings => Strings::presets(),
            Engine::Bass => Bass::presets(),
            Engine::Choir => Choir::presets(),
        };
        Some(p[preset].text)
    }

    /// A fresh engine at `rate` with factory preset `preset`'s engine
    /// settings loaded, and no effects.
    fn engine_with(self, preset: usize, rate: u32) -> Box<dyn Node> {
        match self {
            Engine::Poly => {
                let mut n = Poly::new(rate);
                n.load_preset(&Poly::presets()[preset]);
                Box::new(n)
            }
            Engine::Analog => {
                let mut n = Analog::new(rate);
                n.load_preset(&Analog::presets()[preset]);
                Box::new(n)
            }
            Engine::Squelch => {
                let mut n = Squelch::new(rate);
                n.load_preset(&Squelch::presets()[preset]);
                Box::new(n)
            }
            Engine::Fm => {
                let mut n = Fm::new(rate);
                n.load_preset(&Fm::presets()[preset]);
                Box::new(n)
            }
            Engine::Wavetable => {
                let mut n = Wavetable::new(rate);
                n.load_preset(&Wavetable::presets()[preset]);
                Box::new(n)
            }
            Engine::Keys => {
                let mut n = Keys::new(rate);
                n.load_preset(&Keys::presets()[preset]);
                Box::new(n)
            }
            Engine::Drawbar => {
                let mut n = Drawbar::new(rate);
                n.load_preset(&Drawbar::presets()[preset]);
                Box::new(n)
            }
            Engine::Pluck => {
                let mut n = Pluck::new(rate);
                n.load_preset(&Pluck::presets()[preset]);
                Box::new(n)
            }
            Engine::Strings => {
                let mut n = Strings::new(rate);
                n.load_preset(&Strings::presets()[preset]);
                Box::new(n)
            }
            Engine::Bass => {
                let mut n = Bass::new(rate);
                n.load_preset(&Bass::presets()[preset]);
                Box::new(n)
            }
            Engine::Choir => {
                let mut n = Choir::new(rate);
                n.load_preset(&Choir::presets()[preset]);
                Box::new(n)
            }
        }
    }

    /// The engine's plain patch: its Init preset with nothing that moves
    /// the pitch or blurs the harmonics (a second oscillator, unison,
    /// drift, a rotary speaker) and the filter open, for tuning and
    /// aliasing.
    pub fn plain(self, rate: u32) -> Box<dyn Node> {
        let mut n = self.make(0, rate);
        let set: &[(u16, f32)] = match self {
            // Osc 2 off, unison 1, filter open and still, no velocity.
            Engine::Poly | Engine::Wavetable => &[
                (15, 0.0),
                (17, 0.0),
                (31, 20_000.0),
                (32, 0.0),
                (34, 0.0),
                (35, 0.0),
                (36, 0.0),
                (52, 100.0),
                (54, 0.0),
            ],
            // Age 0, osc 2 off, filter open, no velocity.
            Engine::Analog => &[
                (64, 0.0),
                (11, 0.0),
                (21, 18_000.0),
                (22, 0.0),
                (23, 0.0),
                (24, 0.0),
                (38, 0.0),
            ],
            // Filter open, no envelope on it, a long amp decay.
            Engine::Squelch => &[(10, 5_000.0), (11, 0.0), (12, 0.0), (23, 8_000.0)],
            // Only the 8' drawbar; no rotary, click, leakage or drive.
            Engine::Drawbar => &[
                (0, 0.0),
                (1, 0.0),
                (2, 8.0),
                (40, 0.0),
                (30, 0.0),
                (31, 0.0),
                (32, 0.0),
            ],
            // The Viola (the 8' alone) with its filter an octave up, no
            // body, bow noise, velocity or ensemble.
            Engine::Strings => &[
                (0, 0.0),
                (1, 100.0),
                (10, 100.0),
                (11, 0.0),
                (12, 0.0),
                (23, 0.0),
                (30, 0.0),
            ],
            // The saw alone, the filter open and still, no drive or
            // velocity.
            Engine::Bass => &[
                (bp::SUB, 0.0),
                (bp::CUTOFF, 12_000.0),
                (bp::RESONANCE, 0.0),
                (bp::ENV_AMOUNT, 0.0),
                (bp::FILTER_DRIVE, 0.0),
                (bp::DRIVE, 0.0),
                (bp::VELOCITY, 0.0),
            ],
            // One singer, steady and dry: no detune, vibrato, wavering,
            // breath, ensemble or velocity colour.
            Engine::Choir => &CHOIR_PLAIN,
            Engine::Fm | Engine::Keys | Engine::Pluck => &[],
        };
        for &(id, v) in set {
            n.set_param(id, v);
        }
        n
    }

    /// Whether `node` plays one note at a time (a mono, legato or unison
    /// mode, or an engine that has no other).
    fn mono(self, node: &dyn Node) -> bool {
        if self == Engine::Squelch || self == Engine::Bass {
            return true;
        }
        node.params()
            .iter()
            .find(|p| p.group == "Voice" && p.name == "Mode")
            .is_some_and(|p| node.param(p.id) >= 0.5)
    }
}

/// The Choir's plain patch on its Init: one singer, steady and dry.
const CHOIR_PLAIN: [(u16, f32); 8] = [
    (chp::SINGERS, 1.0),
    (chp::DETUNE, 0.0),
    (chp::BREATH, 0.0),
    (chp::VIBRATO, 0.0),
    (chp::DRIFT, 0.0),
    (chp::ENSEMBLE, 0.0),
    (chp::VEL_TONE, 0.0),
    (chp::ATTACK, 5.0),
];

/// Whether a preset is a bass: its name says so, or the engine is the
/// acid bass line.
fn is_bass(engine: Engine, name: &str) -> bool {
    engine == Engine::Squelch || engine == Engine::Bass || name.contains("Bass")
}

// ---------------------------------------------------------------------------
// Playing.

fn frames(secs: f64, rate: u32) -> usize {
    (secs * f64::from(rate)).round() as usize
}

/// A score: notes `(note, velocity, start s, length s)`, sorted, a
/// note-off before a note-on on the same frame.
fn score(rate: u32, notes: &[(u8, u8, f64, f64)]) -> Vec<(usize, NodeEvent)> {
    let mut s = Vec::new();
    for &(note, velocity, at, len) in notes {
        s.push((frames(at, rate), NodeEvent::NoteOn { note, velocity }));
        s.push((frames(at + len, rate), NodeEvent::NoteOff { note }));
    }
    s.sort_by_key(|(f, e)| (*f, matches!(e, NodeEvent::NoteOn { .. })));
    s
}

/// Silent blocks played before every render, so parameters set just
/// before it (a preset, a patch) have finished gliding to their values,
/// as they have when a player picks a preset and then plays.
const SETTLE_BLOCKS: usize = 100;

/// `len` frames of `node` in blocks of [`BLOCK`] on absolute frames, each
/// event on its frame (`events` sorted by frame), after
/// [`SETTLE_BLOCKS`] silent blocks.
pub fn render(node: &mut dyn Node, rate: u32, len: usize, events: &[(usize, NodeEvent)]) -> Stereo {
    let (mut l, mut r) = (Vec::with_capacity(len), Vec::with_capacity(len));
    let (mut bl, mut br) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
    for b in 0..SETTLE_BLOCKS {
        bl.fill(0.0);
        br.fill(0.0);
        node.process(&Ctx::new(rate, (b * BLOCK) as u64), &[], &mut bl, &mut br);
    }
    let start = SETTLE_BLOCKS * BLOCK;
    let mut timed = Vec::new();
    let (mut at, mut next) = (0, 0);
    while at < len {
        let n = BLOCK.min(len - at);
        timed.clear();
        while next < events.len() && events[next].0 < at + n {
            let off = events[next].0.saturating_sub(at) as u32;
            timed.push(Timed::new(off, events[next].1));
            next += 1;
        }
        bl[..n].fill(0.0);
        br[..n].fill(0.0);
        let ctx = Ctx::new(rate, (start + at) as u64);
        node.process(&ctx, &timed, &mut bl[..n], &mut br[..n]);
        l.extend(bl[..n].iter().map(|&x| f64::from(x)));
        r.extend(br[..n].iter().map(|&x| f64::from(x)));
        at += n;
    }
    Stereo::new(l, r, rate)
}

/// One note held `held` seconds in a render `secs` long.
fn note(node: &mut dyn Node, rate: u32, key: u8, vel: u8, held: f64, secs: f64) -> Stereo {
    let s = score(rate, &[(key, vel, 0.0, held)]);
    render(node, rate, frames(secs, rate), &s)
}

/// The listening phrase: C, E and G a quarter second apart, each held to
/// 1.5 s, then a four-note chord from 2 s to 3.5 s; `base` is the C (60,
/// or 36 for a bass). Seven seconds, so a tail can ring out.
pub fn phrase_score(rate: u32, base: u8) -> (Vec<(usize, NodeEvent)>, usize, f64) {
    let notes = [
        (base, 100, 0.0, 1.5),
        (base + 4, 100, 0.25, 1.25),
        (base + 7, 100, 0.5, 1.0),
        (base - 12, 100, 2.0, 1.5),
        (base - 5, 100, 2.0, 1.5),
        (base, 100, 2.0, 1.5),
        (base + 4, 100, 2.0, 1.5),
    ];
    (score(rate, &notes), frames(7.0, rate), 3.5)
}

/// Preset `preset` of `engine` playing the listening phrase.
pub fn phrase(engine: Engine, preset: usize, rate: u32) -> Stereo {
    let name = &engine.presets()[preset];
    let base = if is_bass(engine, name) { 36 } else { 60 };
    let (s, len, _) = phrase_score(rate, base);
    render(engine.make(preset, rate).as_mut(), rate, len, &s)
}

/// The drum kits' names, in order.
pub fn kits() -> Vec<String> {
    Drums::presets()
        .iter()
        .map(|k| k.name.to_string())
        .collect()
}

/// Drum kit `kit` for listening: each pad in turn a third of a second
/// apart, then two bars of a plain beat (kick, snare, closed hat) at
/// 100 BPM.
pub fn kit_tour(kit: usize, rate: u32) -> Stereo {
    let mut d = Drums::new(rate);
    d.load_preset(&Drums::presets()[kit]);
    let base = d.param(drum_params::BASE_NOTE) as u8;
    let mut notes: Vec<(u8, u8, f64, f64)> = (0..16u8)
        .map(|p| (base + p, 110, f64::from(p) / 3.0, 0.05))
        .collect();
    let start = 16.0 / 3.0 + 0.5;
    let step = 60.0 / 100.0 / 4.0;
    for i in 0..32 {
        let t = start + f64::from(i) * step;
        if i % 8 == 0 || i % 16 == 10 {
            notes.push((base, 120, t, 0.05));
        }
        if i % 8 == 4 {
            notes.push((base + 1, 115, t, 0.05));
        }
        if i % 2 == 0 {
            notes.push((base + 3, if i % 4 == 0 { 100 } else { 70 }, t, 0.05));
        }
    }
    let len = frames(start + 32.0 * step + 1.5, rate);
    render(&mut d, rate, len, &score(rate, &notes))
}

/// A note's frequency in twelve-tone equal temperament, A4 = 440 Hz.
fn hz(note: u8) -> f64 {
    440.0 * ((f64::from(note) - 69.0) / 12.0).exp2()
}

// ---------------------------------------------------------------------------
// Analyses.

/// Levels far below hearing read as −140 dB, so numerical noise is never
/// held to account.
fn floor(db: f64) -> f64 {
    db.max(-140.0)
}

fn db20(a: f64) -> f64 {
    20.0 * a.max(1e-30).log10()
}

fn db10(p: f64) -> f64 {
    10.0 * p.max(1e-30).log10()
}

/// RMS levels (dB) of `x` over `win_ms` windows every `hop_ms`.
fn env_db(x: &[f64], rate: u32, win_ms: f64, hop_ms: f64) -> Vec<f64> {
    let w = frames(win_ms * 0.001, rate).max(1);
    let h = frames(hop_ms * 0.001, rate).max(1);
    let mut ps = Vec::with_capacity(x.len() + 1);
    ps.push(0.0);
    let mut acc = 0.0;
    for v in x {
        acc += v * v;
        ps.push(acc);
    }
    (0..)
        .map(|i| i * h)
        .take_while(|&i| i + w <= x.len())
        .map(|i| db10(((ps[i + w] - ps[i]) / w as f64).max(0.0)))
        .collect()
}

/// The loudest 20 ms of `x`, dB RMS.
fn level_db(x: &[f64], rate: u32) -> f64 {
    env_db(x, rate, 20.0, 5.0)
        .into_iter()
        .fold(f64::NEG_INFINITY, f64::max)
}

/// LUFS, or −120 for silence.
fn lufs(s: &Stereo) -> f64 {
    let v = loudness::integrated(s);
    if v.is_finite() {
        v.max(-120.0)
    } else {
        -120.0
    }
}

/// A 4-term Blackman-Harris window (sidelobes −92 dB).
fn blackman_harris(n: usize) -> Vec<f64> {
    let m = (n - 1) as f64;
    (0..n)
        .map(|i| {
            let t = TAU * i as f64 / m;
            0.35875 - 0.48829 * t.cos() + 0.14128 * (2.0 * t).cos() - 0.01168 * (3.0 * t).cos()
        })
        .collect()
}

fn windowed_power(x: &[f64], w: &[f64]) -> Vec<f64> {
    let v: Vec<f64> = x.iter().zip(w).map(|(a, b)| a * b).collect();
    power_spectrum(&v)
}

/// The spectral centroid (Hz) of `x` from 20 Hz to 20 kHz; `x.len()` a
/// power of two.
fn centroid(x: &[f64], rate: u32) -> f64 {
    let ps = windowed_power(x, &hann_f64(x.len()));
    let bin = f64::from(rate) / x.len() as f64;
    let (mut num, mut den) = (0.0, 0.0);
    for (k, p) in ps.iter().enumerate() {
        let f = k as f64 * bin;
        if (20.0..=20_000.0).contains(&f) {
            num += f * p;
            den += p;
        }
    }
    if den > 0.0 {
        num / den
    } else {
        0.0
    }
}

/// The window's complex response to `x` at `f` Hz, from frame 0 of `x`.
fn dft_at(x: &[f64], w: &[f64], f: f64, rate: u32) -> (f64, f64) {
    let dw = TAU * f / f64::from(rate);
    let (mut re, mut im) = (0.0, 0.0);
    for (i, (a, b)) in x.iter().zip(w).enumerate() {
        let ph = dw * i as f64;
        re += a * b * ph.cos();
        im -= a * b * ph.sin();
    }
    (re, im)
}

/// The amplitude of the partial at `f` in `x` (a window from frame 0).
fn amp_at(x: &[f64], w: &[f64], f: f64, rate: u32) -> f64 {
    let (re, im) = dft_at(x, w, f, rate);
    2.0 * re.hypot(im) / w.iter().sum::<f64>()
}

/// The frequency of the partial near `near` Hz in `x` from frame `from`:
/// the peak of a Blackman-Harris spectrum within ±60 cents, then refined
/// by the phase it turns through over a second. None when that peak is
/// more than 40 dB under the spectrum's loudest, or `x` is too short.
fn freq_near(x: &[f64], rate: u32, near: f64, from: usize) -> Option<f64> {
    const N: usize = 32_768;
    const W: usize = 8_192;
    let hop = rate as usize;
    if x.len() < from + (N).max(hop + W) {
        return None;
    }
    let ps = windowed_power(&x[from..from + N], &blackman_harris(N));
    let bin = f64::from(rate) / N as f64;
    let lo = ((near * (-0.05f64).exp2() / bin).floor() as usize).max(2);
    let hi = ((near * 0.05f64.exp2() / bin).ceil() as usize).min(N / 2 - 2);
    let k = (lo..=hi).max_by(|&a, &b| ps[a].total_cmp(&ps[b]))?;
    let top = ps.iter().skip(2).copied().fold(0.0, f64::max);
    if ps[k] < top * 1e-4 || ps[k] <= 0.0 {
        return None;
    }
    let (a, b, c) = (db10(ps[k - 1]), db10(ps[k]), db10(ps[k + 1]));
    let d = a - 2.0 * b + c;
    let off = if d.abs() > 1e-12 {
        (0.5 * (a - c) / d).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let coarse = (k as f64 + off) * bin;
    // The phase turned over `hop` frames at the coarse frequency.
    let w = blackman_harris(W);
    let (r1, i1) = dft_at(&x[from..from + W], &w, coarse, rate);
    let (r2, i2) = dft_at(&x[from + hop..from + hop + W], &w, coarse, rate);
    let turned = i2.atan2(r2) - i1.atan2(r1);
    let expect = TAU * coarse * hop as f64 / f64::from(rate);
    let mut d = (turned - expect) % TAU;
    if d > PI {
        d -= TAU;
    } else if d < -PI {
        d += TAU;
    }
    // The window's own phase is the same at both: `dft_at` measures from
    // each segment's start, so a steady partial at `coarse` turns by
    // `expect`, and the rest is the offset.
    Some(coarse + d * f64::from(rate) / (TAU * hop as f64))
}

/// Cents from `nominal` to `f`.
fn cents(f: f64, nominal: f64) -> f64 {
    1200.0 * (f / nominal).log2()
}

/// The loudest component of `x` (from `from`, 65,536 frames) that is not
/// a harmonic of `f0`, against the loudest harmonic, dB; 20 Hz to 20 kHz.
fn alias_db(x: &[f64], rate: u32, f0: f64, from: usize) -> f64 {
    const N: usize = 65_536;
    let ps = windowed_power(&x[from..from + N], &blackman_harris(N));
    let bin = f64::from(rate) / N as f64;
    let guard = 8.0;
    let (mut harm, mut other) = (0.0f64, 0.0f64);
    for (k, &p) in ps.iter().enumerate() {
        let f = k as f64 * bin;
        if !(20.0..=20_000.0).contains(&f) {
            continue;
        }
        let h = (f / f0).round().max(1.0);
        if ((f - h * f0) / bin).abs() <= guard {
            harm = harm.max(p);
        } else {
            other = other.max(p);
        }
    }
    floor(db10(other / harm.max(1e-300)))
}

/// Mono fold-down loss (dB): the loudness of `s` against `(L+R)/2` on
/// both sides.
fn mono_loss(s: &Stereo) -> f64 {
    let m = s.mid();
    let mono = Stereo::new(m.clone(), m, s.rate);
    lufs(s) - lufs(&mono)
}

fn correlation(l: &[f64], r: &[f64]) -> f64 {
    let (mut lr, mut ll, mut rr) = (0.0, 0.0, 0.0);
    for (a, b) in l.iter().zip(r) {
        lr += a * b;
        ll += a * a;
        rr += b * b;
    }
    if ll <= 0.0 || rr <= 0.0 {
        1.0
    } else {
        lr / (ll * rr).sqrt()
    }
}

/// Four one-pole low-passes at `fc`.
fn low_pass(x: &[f64], fc: f64, rate: u32) -> Vec<f64> {
    let a = 1.0 - (-TAU * fc / f64::from(rate)).exp();
    let mut s = [0.0; 4];
    x.iter()
        .map(|&v| {
            let mut u = v;
            for st in &mut s {
                *st += a * (u - *st);
                u = *st;
            }
            u
        })
        .collect()
}

/// Seconds from frame `from` until the output stays under −80 dBFS
/// (10 ms peaks); the rest of the render when it never does.
fn tail_secs(s: &Stereo, from: usize) -> f64 {
    let w = frames(0.01, s.rate);
    let mut last = from;
    let mut i = from;
    while i < s.len() {
        let end = (i + w).min(s.len());
        let peak = s.left[i..end]
            .iter()
            .chain(&s.right[i..end])
            .fold(0.0f64, |m, v| m.max(v.abs()));
        if db20(peak) > -80.0 {
            last = end;
        }
        i = end;
    }
    (last - from) as f64 / f64::from(s.rate)
}

/// Milliseconds from the start until the 5 ms level first comes within
/// 3 dB of its loudest.
fn attack_ms(x: &[f64], rate: u32) -> f64 {
    let e = env_db(x, rate, 5.0, 1.0);
    let top = e.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    e.iter().position(|&v| v >= top - 3.0).unwrap_or(0) as f64
}

/// Dannenberg's square-law velocity curve with a dynamic range of `range`
/// dB, as a level against velocity 127 (dB).
fn square_law_db(v: f64, range: f64) -> f64 {
    let r = 10f64.powf(range / 20.0);
    let b = 127.0 / (126.0 * r.sqrt()) - 1.0 / 126.0;
    let m = (1.0 - b) / 127.0;
    db20((m * v + b).powi(2))
}

/// The square-law range that best fits `levels` (velocity, dB against
/// velocity 127), and its worst error (dB).
fn square_law_fit(levels: &[(f64, f64)]) -> (f64, f64) {
    let mut best = (0.0, f64::INFINITY);
    for i in 0..=1600 {
        let range = f64::from(i) * 0.05;
        let err = levels
            .iter()
            .map(|&(v, l)| (l - square_law_db(v, range)).abs())
            .fold(0.0, f64::max);
        if err < best.1 {
            best = (range, err);
        }
    }
    best
}

/// The slope of the least-squares line through `pts`.
fn slope(pts: &[(f64, f64)]) -> f64 {
    let n = pts.len() as f64;
    let (sx, sy) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.0, a.1 + p.1));
    let (mx, my) = (sx / n, sy / n);
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for p in pts {
        sxy += (p.0 - mx) * (p.1 - my);
        sxx += (p.0 - mx) * (p.0 - mx);
    }
    if sxx > 0.0 {
        sxy / sxx
    } else {
        0.0
    }
}

/// `y` less its least-squares quadratic.
fn detrend2(y: &[f64]) -> Vec<f64> {
    let n = y.len();
    let xs: Vec<f64> = (0..n).map(|i| i as f64 / n as f64 - 0.5).collect();
    // Normal equations for a + b x + c x^2.
    let mut m = [[0.0f64; 3]; 3];
    let mut v = [0.0f64; 3];
    for (x, &t) in xs.iter().zip(y) {
        let p = [1.0, *x, x * x];
        for i in 0..3 {
            v[i] += p[i] * t;
            for j in 0..3 {
                m[i][j] += p[i] * p[j];
            }
        }
    }
    let c = solve3(m, v);
    xs.iter()
        .zip(y)
        .map(|(x, &t)| t - (c[0] + c[1] * x + c[2] * x * x))
        .collect()
}

/// Solves a 3x3 system by Cramer's rule.
fn solve3(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    let det = |a: [[f64; 3]; 3]| {
        a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
            - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
            + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0])
    };
    let d = det(m);
    if d.abs() < 1e-300 {
        return [0.0; 3];
    }
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let mut a = m;
        for i in 0..3 {
            a[i][k] = v[i];
        }
        *o = det(a) / d;
    }
    out
}

/// T60 (s) of a held note: the slope of its 10 ms level from where it
/// first falls 10 dB under its loudest until it is 50 dB under or the
/// render ends.
fn t60(x: &[f64], rate: u32) -> f64 {
    let hop = 0.002;
    let e = env_db(x, rate, 10.0, hop * 1000.0);
    let (peak, top) = e
        .iter()
        .copied()
        .enumerate()
        .fold(
            (0, f64::NEG_INFINITY),
            |a, (i, v)| if v > a.1 { (i, v) } else { a },
        );
    let start = (peak..e.len())
        .find(|&i| e[i] <= top - 10.0)
        .unwrap_or(peak);
    let pts: Vec<(f64, f64)> = e
        .iter()
        .enumerate()
        .skip(start)
        .take_while(|(_, &v)| v > top - 50.0)
        .map(|(i, &v)| (i as f64 * hop, v))
        .collect();
    if pts.len() < 10 {
        return 0.0;
    }
    let s = slope(&pts);
    if s < -1e-6 {
        (-60.0 / s).min(1000.0)
    } else {
        1000.0
    }
}

// ---------------------------------------------------------------------------
// Rows.

#[allow(clippy::too_many_arguments)]
fn put(
    rows: &mut Vec<Row>,
    id: &'static str,
    subject: &str,
    metric: &str,
    value: f64,
    better: Better,
    target: Option<f64>,
    tol: f64,
) {
    rows.push(Row {
        target_id: id,
        subject: format!("synth/{subject}"),
        metric: metric.to_string(),
        value,
        better,
        target,
        tol,
    });
}

/// Every number, in the order measured.
pub fn report(rate: u32) -> Vec<Row> {
    let mut rows = Vec::new();
    for e in Engine::ALL {
        for (i, name) in e.presets().iter().enumerate() {
            preset_rows(&mut rows, e, i, name, rate);
        }
    }
    tuning(&mut rows, rate);
    aliasing(&mut rows, rate);
    glide(&mut rows, rate);
    envelopes(&mut rows, rate);
    routes(&mut rows, rate);
    filters(&mut rows, rate);
    fm(&mut rows, rate);
    drums(&mut rows, rate);
    strings(&mut rows, rate);
    bass(&mut rows, rate);
    choir(&mut rows, rate);
    rows
}

/// The rows as text, one a line.
pub fn to_text(rows: &[Row]) -> String {
    let mut s = String::new();
    for r in rows {
        let target = match (r.target, r.met()) {
            (Some(t), Some(true)) => format!("target {t}: met"),
            (Some(t), _) => format!("target {t}: not yet"),
            (None, _) => String::new(),
        };
        s.push_str(&format!(
            "{:<4} {:<64} {:>10.3}  {target}\n",
            r.target_id,
            r.key(),
            r.value
        ));
    }
    s
}

/// How many rows have a target and how many meet it, by id.
pub fn summary(rows: &[Row]) -> String {
    let mut ids: Vec<&str> = rows.iter().map(|r| r.target_id).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut s = String::new();
    for id in ids {
        let of: Vec<&Row> = rows.iter().filter(|r| r.target_id == id).collect();
        let with: Vec<&&Row> = of.iter().filter(|r| r.met().is_some()).collect();
        let met = with.iter().filter(|r| r.met() == Some(true)).count();
        s.push_str(&format!(
            "{id:<4} {:>4} rows, {:>4} with a target, {:>4} met\n",
            of.len(),
            with.len(),
            met
        ));
    }
    s
}

/// The ratchet file for `rows`.
pub fn ratchet_text(rows: &[Row]) -> String {
    let mut lines: Vec<String> = rows
        .iter()
        .filter(|r| r.better != Better::Info)
        .map(|r| format!("{}\t{}", r.key(), num(r.value)))
        .collect();
    lines.sort();
    let mut s = String::from(HEADER);
    for l in lines {
        s.push_str(&l);
        s.push('\n');
    }
    s
}

/// What fails the ratchet: a row worse than recorded, a row not recorded
/// yet, a recorded row no longer measured.
pub fn ratchet_check(rows: &[Row], recorded: &[(String, f64)]) -> Vec<String> {
    let mut problems = Vec::new();
    for r in rows.iter().filter(|r| r.better != Better::Info) {
        let key = r.key();
        match recorded.iter().find(|(k, _)| *k == key) {
            Some((_, old)) if r.worse_than(*old) => problems.push(format!(
                "worse: {key} is {} (was {}, {:?} is better, tolerance {})",
                num(r.value),
                num(*old),
                r.better,
                r.tol
            )),
            Some(_) => {}
            None => problems.push(format!("not recorded: {key} = {}", num(r.value))),
        }
    }
    for (k, _) in recorded {
        if !rows.iter().any(|r| r.key() == *k) {
            problems.push(format!("no longer measured: {k}"));
        }
    }
    problems
}

fn num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Every preset.

/// Velocities for the level and tone curves.
const VELS: [u8; 5] = [20, 40, 64, 100, 127];

fn preset_rows(rows: &mut Vec<Row>, e: Engine, i: usize, name: &str, rate: u32) {
    let subject = format!("{}[{name}]", e.name());
    let make = || e.make(i, rate);
    let bass = is_bass(e, name);
    let mono = e.mono(make().as_ref());

    // L1, L3, L4, L8, L9: the phrase.
    let base = if bass { 36 } else { 60 };
    let (s, len, last_off) = phrase_score(rate, base);
    let ph = render(make().as_mut(), rate, len, &s);
    put(
        rows,
        "L1",
        &subject,
        "phrase_lufs",
        lufs(&ph),
        Better::Info,
        None,
        0.0,
    );
    let dc = {
        let n = ph.len() as f64;
        let l = ph.left.iter().sum::<f64>() / n;
        let r = ph.right.iter().sum::<f64>() / n;
        floor(db20(l.abs().max(r.abs())))
    };
    put(
        rows,
        "L3",
        &subject,
        "dc_dbfs",
        dc,
        Better::Lower,
        Some(-60.0),
        1.0,
    );
    let tail = tail_secs(&ph, frames(last_off, rate));
    put(
        rows,
        "L4",
        &subject,
        "tail_s",
        tail,
        Better::Info,
        None,
        0.0,
    );
    put(
        rows,
        "L8",
        &subject,
        "mono_loss_db",
        mono_loss(&ph),
        Better::Lower,
        Some(3.0),
        0.2,
    );
    put(
        rows,
        "L9",
        &subject,
        "correlation",
        correlation(&ph.left, &ph.right),
        Better::Info,
        None,
        0.0,
    );

    // L2: eight notes at full velocity.
    let chord8: Vec<(u8, u8, f64, f64)> = [0u8, 4, 7, 11, 12, 16, 19, 23]
        .iter()
        .map(|&k| (base - 12 + k, 127, 0.0, 1.0))
        .collect();
    let c8 = render(
        make().as_mut(),
        rate,
        frames(2.0, rate),
        &score(rate, &chord8),
    );
    put(
        rows,
        "L2",
        &subject,
        "chord8_peak_dbfs",
        loudness::sample_peak(&c8).max(-140.0),
        Better::Lower,
        Some(0.0),
        0.5,
    );

    // L5, L6, L13: one note at each velocity.
    let key = base;
    let by_vel: Vec<(u8, Stereo)> = VELS
        .iter()
        .map(|&v| (v, note(make().as_mut(), rate, key, v, 1.0, 1.5)))
        .collect();
    let lv: Vec<(f64, f64)> = by_vel
        .iter()
        .map(|(v, s)| (f64::from(*v), level_db(&s.mid(), rate)))
        .collect();
    let top = lv.last().map_or(0.0, |p| p.1);
    let rel: Vec<(f64, f64)> = lv.iter().map(|&(v, l)| (v, l - top)).collect();
    put(
        rows,
        "L6",
        &subject,
        "vel_range_db",
        top - lv[0].1,
        Better::Info,
        None,
        0.0,
    );
    let (_, fit_err) = square_law_fit(&rel);
    put(
        rows,
        "L6",
        &subject,
        "vel_curve_err_db",
        fit_err,
        Better::Lower,
        Some(2.0),
        0.2,
    );
    let tone = |s: &Stereo| centroid(&s.mid()[..16_384], rate);
    let (c_lo, c_hi) = (tone(&by_vel[0].1), tone(&by_vel[4].1));
    let vel_tone = if c_lo > 0.0 && c_hi > 0.0 {
        (c_hi / c_lo).log2()
    } else {
        0.0
    };
    // An organ's keys don't feel velocity: shown, no target.
    let target = (e != Engine::Drawbar).then_some(0.1);
    put(
        rows,
        "L5",
        &subject,
        "vel_tone_oct",
        vel_tone,
        Better::Higher,
        target,
        0.03,
    );
    let at100 = &by_vel[3].1;
    put(
        rows,
        "L13",
        &subject,
        "attack_ms",
        attack_ms(&at100.mid(), rate),
        Better::Info,
        None,
        0.0,
    );

    // L7: full pressure from 0.3 s against none, over 0.8 to 1.0 s.
    let mut ev = score(rate, &[(key, 100, 0.0, 1.0)]);
    ev.push((
        frames(0.3, rate),
        NodeEvent::Pressure {
            note: key,
            value: 127,
        },
    ));
    ev.sort_by_key(|(f, _)| *f);
    let pr = render(make().as_mut(), rate, frames(1.5, rate), &ev);
    let (a, b) = (frames(0.8, rate), frames(0.8, rate) + 8_192);
    let seg = |s: &Stereo| s.mid()[a..b].to_vec();
    let lvl = |x: &[f64]| db20((x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64).sqrt());
    let (p0, p1) = (seg(at100), seg(&pr));
    put(
        rows,
        "L7",
        &subject,
        "pressure_db",
        lvl(&p1) - lvl(&p0),
        Better::Info,
        None,
        0.0,
    );
    let (q0, q1) = (centroid(&p0, rate), centroid(&p1, rate));
    let p_oct = if q0 > 0.0 && q1 > 0.0 {
        (q1 / q0).log2()
    } else {
        0.0
    };
    put(
        rows,
        "L7",
        &subject,
        "pressure_oct",
        p_oct,
        Better::Info,
        None,
        0.0,
    );

    // L11: loudness across the keys (B6 for a bass, K6 for keys).
    let keys: &[u8] = if bass {
        &[24, 36, 48]
    } else if matches!(e, Engine::Keys | Engine::Pluck) {
        &[24, 36, 48, 60, 72, 84, 96]
    } else {
        &[36, 48, 60, 72, 84]
    };
    let mut singles: Vec<(u8, Stereo)> = Vec::new();
    let mut single = |k: u8| -> Stereo {
        if let Some((_, s)) = singles.iter().find(|(n, _)| *n == k) {
            return s.clone();
        }
        let s = note(make().as_mut(), rate, k, 100, 1.5, 2.0);
        singles.push((k, s.clone()));
        s
    };
    let louds: Vec<f64> = keys.iter().map(|&k| lufs(&single(k))).collect();
    let spread = louds.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        - louds.iter().copied().fold(f64::INFINITY, f64::min);
    put(
        rows,
        "L11",
        &subject,
        "key_spread_db",
        spread,
        Better::Lower,
        Some(3.0),
        0.3,
    );

    // L12: a six-note chord against the power sum of its notes.
    if !mono {
        let ks = [48u8, 52, 55, 60, 64, 67];
        let chord: Vec<(u8, u8, f64, f64)> = ks.iter().map(|&k| (k, 100, 0.0, 1.5)).collect();
        let c6 = render(
            make().as_mut(),
            rate,
            frames(2.0, rate),
            &score(rate, &chord),
        );
        let sum: f64 = ks
            .iter()
            .map(|&k| 10f64.powf(lufs(&single(k)) / 10.0))
            .sum();
        put(
            rows,
            "L12",
            &subject,
            "chord6_vs_power_db",
            lufs(&c6) - db10(sum),
            Better::Near(0.0),
            Some(6.0),
            0.3,
        );
    }

    if bass {
        bass_rows(rows, &subject, &mut single, &ph, rate);
    }
    if matches!(e, Engine::Keys | Engine::Pluck) {
        // The K rows measure the engine's own voicing (decay against its
        // label, the release drop, the pickups, the tremolo), so they play
        // the engine alone: a sound's echo or reverb lengthens a decay by
        // design.
        keys_rows(rows, e, &subject, || e.engine_with(i, rate), &by_vel, rate);
    }
}

fn bass_rows(
    rows: &mut Vec<Row>,
    subject: &str,
    single: &mut impl FnMut(u8) -> Stereo,
    ph: &Stereo,
    rate: u32,
) {
    // B4: the largest step in the first four frames of C2.
    let c2 = single(36).mid();
    let step = (0..4).fold(0.0f64, |m, k| {
        let prev = if k == 0 { 0.0 } else { c2[k - 1] };
        m.max((c2[k] - prev).abs())
    });
    put(
        rows,
        "B4",
        subject,
        "start_step_dbfs",
        floor(db20(step)),
        Better::Lower,
        Some(-40.0),
        1.0,
    );
    // B5: 100 to 400 Hz against 20 to 100 Hz, the worse of C1 and C2.
    let band = |x: &[f64]| {
        let ps = windowed_power(
            &x[frames(0.1, rate)..frames(0.1, rate) + 32_768],
            &hann_f64(32_768),
        );
        let bin = f64::from(rate) / 32_768.0;
        let sum = |lo: f64, hi: f64| {
            ps.iter()
                .enumerate()
                .filter(|(k, _)| (lo..hi).contains(&(*k as f64 * bin)))
                .map(|(_, p)| p)
                .sum::<f64>()
        };
        db10(sum(100.0, 400.0)) - db10(sum(20.0, 100.0))
    };
    let small = band(&single(24).mid()).min(band(&c2));
    put(
        rows,
        "B5",
        subject,
        "small_speaker_db",
        small,
        Better::Higher,
        Some(-12.0),
        0.3,
    );
    // B7: the phrase's correlation under 120 Hz.
    let (l, r) = (
        low_pass(&ph.left, 120.0, rate),
        low_pass(&ph.right, 120.0, rate),
    );
    put(
        rows,
        "B7",
        subject,
        "low_correlation",
        correlation(&l, &r),
        Better::Higher,
        Some(0.95),
        0.01,
    );
}

fn keys_rows(
    rows: &mut Vec<Row>,
    e: Engine,
    subject: &str,
    make: impl Fn() -> Box<dyn Node>,
    by_vel: &[(u8, Stereo)],
    rate: u32,
) {
    let node = make();
    let clav = e == Engine::Keys && node.param(0).round() == 2.0;
    // K1: the second harmonic against the first at C4, velocity 40 to 127,
    // on the tine and reed: the pickup's growl. The clav has no such
    // growl (a harder hit brightens the whole string), so its velocity
    // shows in L5.
    if e == Engine::Keys && !clav {
        let w = blackman_harris(8_192);
        let from = frames(0.05, rate);
        let h2 = |s: &Stereo| {
            let x = &s.mid()[from..from + 8_192];
            db20(amp_at(x, &w, 2.0 * hz(60), rate)) - db20(amp_at(x, &w, hz(60), rate))
        };
        put(
            rows,
            "K1",
            subject,
            "h2_rise_db",
            h2(&by_vel[4].1) - h2(&by_vel[1].1),
            Better::Higher,
            Some(12.0),
            0.3,
        );
    }
    // K3: decay against the Decay label at C4, and its law across C2 to C6.
    // The clav's Mute shortens it by up to 85 %, as its label says.
    let mut label = f64::from(node.param(1)) * 0.001;
    if clav {
        label *= 1.0 - 0.85 * f64::from(node.param(5)) / 100.0;
    }
    let law = if e == Engine::Keys { -0.75 } else { -0.5 };
    let mut pts = Vec::new();
    let mut at_c4 = 0.0;
    for k in [36u8, 48, 60, 72, 84] {
        let s = note(make().as_mut(), rate, k, 100, 6.0, 6.0);
        let t = t60(&s.mid(), rate);
        if k == 60 {
            at_c4 = t;
        }
        pts.push((hz(k).ln(), t.max(1e-3).ln()));
    }
    put(
        rows,
        "K3",
        subject,
        "decay_c4_err_pct",
        (at_c4 - label) / label * 100.0,
        Better::Near(0.0),
        Some(20.0),
        1.0,
    );
    put(
        rows,
        "K3",
        subject,
        "decay_slope",
        slope(&pts),
        Better::Near(law),
        Some(0.15),
        0.02,
    );
    // K4: the clav's release drop, on clav presets: C5 held half a
    // second, then let go with Release at 400 ms so it rings long enough
    // to read; its pitch 10 to 50 ms later against Release drop.
    if clav {
        let label = f64::from(node.param(6));
        let mut n = make();
        n.set_param(2, 400.0);
        let s = note(n.as_mut(), rate, 72, 100, 0.5, 0.6);
        let (from, len) = (frames(0.51, rate), frames(0.04, rate));
        let x = &s.mid()[from..from + len];
        let w = blackman_harris(len);
        let want = hz(72) * 2f64.powf(-label / 12.0);
        let at = |c: f64| amp_at(x, &w, want * 2f64.powf(c / 1200.0), rate);
        let best = (-100..=100)
            .map(f64::from)
            .max_by(|&a, &b| at(a).total_cmp(&at(b)))
            .unwrap_or(0.0);
        let (a, b, c) = (at(best - 1.0), at(best), at(best + 1.0));
        let d = a - 2.0 * b + c;
        let off = if d.abs() > 1e-30 {
            (0.5 * (a - c) / d).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        put(
            rows,
            "K4",
            subject,
            "release_drop_err_st",
            -(best + off) / 100.0,
            Better::Near(0.0),
            Some(0.5),
            0.05,
        );
    }
    // K7: the clav's pickup notches, on clav presets with one pickup: the
    // partial over that pickup's node (the 5th for the neck, the 14th for
    // the bridge) against its neighbours, C2.
    if clav && node.param(3).round() <= 1.0 {
        let k = if node.param(3).round() == 0.0 {
            5.0
        } else {
            14.0
        };
        let s = note(make().as_mut(), rate, 36, 50, 1.0, 1.0);
        let n = 16_384;
        let from = frames(0.1, rate);
        let x = &s.mid()[from..from + n];
        let w = blackman_harris(n);
        let lvl = |k: f64| db20(amp_at(x, &w, k * hz(36), rate));
        put(
            rows,
            "K7",
            subject,
            "pickup_notch_db",
            lvl(k) - 0.5 * (lvl(k - 1.0) + lvl(k + 1.0)),
            Better::Lower,
            Some(-15.0),
            0.5,
        );
    }
    // K9: tremolo rate, when the preset has tremolo.
    if e == Engine::Keys && node.param(20) > 0.0 {
        let rate_label = f64::from(node.param(21));
        let s = note(make().as_mut(), rate, 60, 100, 6.0, 6.0);
        // The 5 ms level from 0.3 s while it is within 40 dB of its
        // loudest, less a quadratic for the decay.
        let env = env_db(&s.left[frames(0.3, rate)..], rate, 5.0, 5.0);
        let top = env.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let live = env.iter().take_while(|&&v| v > top - 40.0).count();
        let n = if live >= 256 {
            1 << (usize::BITS - 1 - live.leading_zeros())
        } else {
            256
        };
        let n = n.min(1024).min(env.len());
        let flat = detrend2(&env[..n]);
        let ps = windowed_power(&flat, &hann_f64(n));
        let bin = 200.0 / n as f64;
        let k = (2..n / 2)
            .filter(|&k| (0.5..15.0).contains(&(k as f64 * bin)))
            .max_by(|&a, &b| ps[a].total_cmp(&ps[b]))
            .unwrap_or(2);
        let (a, b, c) = (db10(ps[k - 1]), db10(ps[k]), db10(ps[k + 1]));
        let d = a - 2.0 * b + c;
        let off = if d.abs() > 1e-12 {
            (0.5 * (a - c) / d).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        let measured = (k as f64 + off) * bin;
        put(
            rows,
            "K9",
            subject,
            "trem_rate_err_pct",
            (measured - rate_label) / rate_label * 100.0,
            Better::Near(0.0),
            Some(5.0),
            0.5,
        );
    }
}

// ---------------------------------------------------------------------------
// Each engine's plain patch.

fn tuning(rows: &mut Vec<Row>, rate: u32) {
    for e in Engine::ALL {
        let mut worst = (0.0f64, 0u8);
        for k in [24u8, 36, 60, 84, 96] {
            let s = note(e.plain(rate).as_mut(), rate, k, 100, 2.0, 2.0);
            let c = freq_near(&s.mid(), rate, hz(k), frames(0.4, rate))
                .map_or(100.0, |f| cents(f, hz(k)));
            if c.abs() > worst.0.abs() {
                worst = (c, k);
            }
        }
        let subject = format!("tune[{}]", e.name());
        // A tonewheel organ is tuned by its gears: shown, no target.
        let target = (e != Engine::Drawbar).then_some(1.0);
        put(
            rows,
            "T1",
            &subject,
            "worst_cents",
            worst.0.abs(),
            Better::Lower,
            target,
            0.05,
        );
        put(
            rows,
            "T1",
            &subject,
            "worst_note",
            f64::from(worst.1),
            Better::Info,
            None,
            0.0,
        );
    }
}

/// An aliasing case: a name, the engine and what to set on its plain
/// patch.
type AliasCase = (&'static str, Engine, Vec<(u16, f32)>);

/// The aliasing cases.
fn alias_cases() -> Vec<AliasCase> {
    vec![
        ("Poly saw", Engine::Poly, vec![]),
        ("Poly pulse", Engine::Poly, vec![(0, 1.0)]),
        ("Poly triangle", Engine::Poly, vec![(0, 2.0)]),
        ("Poly saw, drive 24 dB", Engine::Poly, vec![(33, 24.0)]),
        ("Poly saw, distortion 24 dB", Engine::Poly, vec![(37, 24.0)]),
        ("Analog saw", Engine::Analog, vec![]),
        ("Analog saw, drive 100", Engine::Analog, vec![(26, 100.0)]),
        ("Squelch saw", Engine::Squelch, vec![]),
        ("Squelch square", Engine::Squelch, vec![(0, 1.0)]),
        ("Squelch saw, drive 100", Engine::Squelch, vec![(30, 100.0)]),
        ("Bass saw", Engine::Bass, vec![]),
        ("Bass pulse", Engine::Bass, vec![(bp::WAVE, bp::PULSE)]),
        (
            "Bass saw, drive 100",
            Engine::Bass,
            vec![(bp::DRIVE, 100.0)],
        ),
        ("Wavetable Init", Engine::Wavetable, vec![]),
        (
            "Wavetable position 100",
            Engine::Wavetable,
            vec![(1, 100.0)],
        ),
        ("Strings viola", Engine::Strings, vec![]),
        ("Choir aah", Engine::Choir, vec![(chp::VOWEL, 50.0)]),
        ("Choir ee", Engine::Choir, vec![(chp::VOWEL, 100.0)]),
        // The Violin's 4' over its 8'.
        (
            "Strings violin",
            Engine::Strings,
            vec![(0, 100.0), (1, 0.0)],
        ),
        // Algorithm 32, operator 6 alone with full feedback.
        (
            "FM feedback 7",
            Engine::Fm,
            vec![(0, 32.0), (1, 7.0), (40, 0.0), (200, 99.0)],
        ),
    ]
}

fn aliasing(rows: &mut Vec<Row>, rate: u32) {
    for (what, e, set) in alias_cases() {
        for (k, label) in [(72u8, "C5"), (84, "C6"), (96, "C7")] {
            let mut n = e.plain(rate);
            for &(id, v) in &set {
                n.set_param(id, v);
            }
            let s = note(n.as_mut(), rate, k, 100, 2.0, 2.0);
            let x = s.mid();
            let from = frames(0.3, rate);
            let f0 = freq_near(&x, rate, hz(k), from).unwrap_or(hz(k));
            put(
                rows,
                "A1",
                &format!("alias[{what}, {label}]"),
                "worst_alias_db",
                alias_db(&x, rate, f0, from),
                Better::Lower,
                Some(-60.0),
                1.0,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Poly's voice.

/// Poly's plain patch on a sine.
fn poly_sine(rate: u32) -> Box<dyn Node> {
    let mut n = Engine::Poly.plain(rate);
    n.set_param(0, 3.0);
    n
}

/// The pitch (Hz) at each upward zero crossing of `x` after the first,
/// with its time (s).
fn pitch_track(x: &[f64], rate: u32) -> Vec<(f64, f64)> {
    let mut ups = Vec::new();
    for i in 1..x.len() {
        if x[i - 1] < 0.0 && x[i] >= 0.0 {
            let t = (i - 1) as f64 + x[i - 1] / (x[i - 1] - x[i]);
            ups.push(t / f64::from(rate));
        }
    }
    ups.windows(2)
        .map(|w| (0.5 * (w[0] + w[1]), 1.0 / (w[1] - w[0])))
        .collect()
}

fn glide(rows: &mut Vec<Row>, rate: u32) {
    // B2: legato C3 to C4 with a 200 ms glide.
    let mut n = poly_sine(rate);
    n.set_param(70, 2.0);
    n.set_param(72, 200.0);
    let s = score(rate, &[(48, 100, 0.0, 1.5), (60, 100, 0.5, 1.0)]);
    let out = render(n.as_mut(), rate, frames(1.5, rate), &s);
    let track = pitch_track(&out.mid(), rate);
    let target = hz(60);
    let land: Vec<f64> = track
        .iter()
        .filter(|(t, _)| (0.9..1.1).contains(t))
        .map(|(_, f)| cents(*f, target))
        .collect();
    let land = land.iter().sum::<f64>() / land.len().max(1) as f64;
    let arrived = track
        .iter()
        .rposition(|(t, f)| *t < 1.4 && cents(*f, target).abs() > 12.0)
        .map_or(0.5, |i| track[i].0);
    let subject = "glide[Poly legato, 200 ms]";
    put(
        rows,
        "B2",
        subject,
        "land_cents",
        land.abs(),
        Better::Lower,
        Some(2.0),
        0.2,
    );
    put(
        rows,
        "B2",
        subject,
        "time_err_pct",
        ((arrived - 0.5) - 0.2) / 0.2 * 100.0,
        Better::Near(0.0),
        Some(10.0),
        1.0,
    );
}

/// The amplitude of a sine at `f` in `x`, a least-squares fit over two
/// periods around every `hop` frames: (time s, amplitude).
fn sine_amp(x: &[f64], f: f64, rate: u32, hop: usize) -> Vec<(f64, f64)> {
    let w = (2.0 * f64::from(rate) / f).round() as usize;
    let dw = TAU * f / f64::from(rate);
    (0..)
        .map(|i| i * hop)
        .take_while(|&i| i + w <= x.len())
        .map(|i| {
            let (mut cc, mut ss, mut cs, mut xc, mut xs) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for (j, v) in x[i..i + w].iter().enumerate() {
                let (s, c) = (dw * (i + j) as f64).sin_cos();
                cc += c * c;
                ss += s * s;
                cs += c * s;
                xc += v * c;
                xs += v * s;
            }
            let det = cc * ss - cs * cs;
            let a = (xc * ss - xs * cs) / det;
            let b = (xs * cc - xc * cs) / det;
            (((i + w / 2) as f64) / f64::from(rate), a.hypot(b))
        })
        .collect()
}

fn envelopes(rows: &mut Vec<Row>, rate: u32) {
    // P6: amp envelope times against their labels, on a sine at C6. Since
    // Poly moved onto the voice core (preset format 2) a time means what
    // it says: the attack reaches full level in its time (99 % here, which
    // it reaches 2 % early), the decay covers 99 % of the way to the
    // sustain, the release falls 60 dB.
    let key = 84;
    let run = |set: &[(u16, f32)], held: f64| {
        let mut n = poly_sine(rate);
        n.set_param(50, 0.5);
        n.set_param(53, 1.0);
        for &(id, v) in set {
            n.set_param(id, v);
        }
        let s = note(n.as_mut(), rate, key, 100, held, held + 0.6);
        let x = s.mid();
        let f = freq_near(&x, rate, hz(key), 0).unwrap_or(hz(key));
        sine_amp(&x, f, rate, 22)
    };
    let mean = |a: &[(f64, f64)], lo: f64, hi: f64| {
        let v: Vec<f64> = a
            .iter()
            .filter(|(t, _)| (lo..hi).contains(t))
            .map(|p| p.1)
            .collect();
        v.iter().sum::<f64>() / v.len().max(1) as f64
    };
    let err = |measured: f64, label_ms: f64| (measured * 1000.0 - label_ms) / label_ms * 100.0;

    // Attack 100 ms to full sustain.
    let a = run(&[(50, 100.0), (52, 100.0)], 1.0);
    let full = mean(&a, 0.6, 0.9);
    let t = a.iter().find(|p| p.1 >= 0.99 * full).map_or(1.0, |p| p.0);
    put(
        rows,
        "P6",
        "env[Poly amp, attack 100 ms]",
        "time_err_pct",
        err(t, 100.0),
        Better::Near(0.0),
        Some(10.0),
        1.0,
    );
    // Decay 200 ms to half.
    let d = run(&[(51, 200.0), (52, 50.0)], 1.0);
    let held = mean(&d, 0.8, 0.95);
    let peak = d.iter().map(|p| p.1).fold(0.0, f64::max);
    let t = d
        .iter()
        .find(|p| p.1 <= held + 0.01 * (peak - held))
        .map_or(1.0, |p| p.0);
    put(
        rows,
        "P6",
        "env[Poly amp, decay 200 ms]",
        "time_err_pct",
        err(t, 200.0),
        Better::Near(0.0),
        Some(10.0),
        1.0,
    );
    // Release 300 ms from full: the time to fall 60 dB.
    let r = run(&[(52, 100.0), (53, 300.0)], 0.5);
    let held = mean(&r, 0.3, 0.45);
    let t = r
        .iter()
        .find(|p| p.0 > 0.5 && p.1 <= 0.001 * held)
        .map_or(1.1, |p| p.0);
    put(
        rows,
        "P6",
        "env[Poly amp, release 300 ms]",
        "time_err_pct",
        err(t - 0.5, 300.0),
        Better::Near(0.0),
        Some(10.0),
        1.0,
    );
}

fn routes(rows: &mut Vec<Row>, rate: u32) {
    // P12: a play route reaches as far as its depth says (the reach of a
    // pressure slot's on the same target). With the mod wheel at the top,
    // Pitch at 100 % is an octave up, and the default route, the wheel on
    // Vibrato at 50 %, is half a semitone each way at Vib rate.
    use mpfree_node::Ctl;
    use mpfree_synth::params::{route, route_id, target};
    let key = 69;
    let wheel = |set: &[(u16, f32)]| {
        let mut n = poly_sine(rate);
        for &(id, v) in set {
            n.set_param(id, v);
        }
        let mut s = vec![(
            0,
            NodeEvent::Control {
                ctl: Ctl::ModWheel,
                value: 1.0,
            },
        )];
        // Long enough for the frequency read (two windows a second apart).
        s.extend(score(rate, &[(key, 100, 0.0, 2.5)]));
        render(n.as_mut(), rate, frames(2.5, rate), &s).mid()
    };
    let x = wheel(&[
        (route_id(0, route::TARGET), f32::from(target::PITCH)),
        (route_id(0, route::DEPTH), 100.0),
    ]);
    let f = freq_near(&x, rate, hz(key + 12), frames(0.4, rate)).unwrap_or(hz(key));
    put(
        rows,
        "P12",
        "route[Poly, wheel to pitch 100 %]",
        "pitch_err_cents",
        cents(f, hz(key + 12)).abs(),
        Better::Lower,
        Some(1.0),
        0.1,
    );
    let x = wheel(&[]);
    let semis: Vec<f64> = pitch_track(&x, rate)
        .iter()
        .filter(|(t, _)| (0.5..1.4).contains(t))
        .map(|(_, f)| cents(*f, hz(key)) / 100.0)
        .collect();
    let hi = semis.iter().copied().fold(f64::MIN, f64::max);
    let lo = semis.iter().copied().fold(f64::MAX, f64::min);
    put(
        rows,
        "P12",
        "route[Poly, wheel to vibrato 50 %]",
        "depth_err_pct",
        ((hi - lo) / 2.0 - 0.5) / 0.5 * 100.0,
        Better::Near(0.0),
        Some(5.0),
        1.0,
    );
}

fn filters(rows: &mut Vec<Row>, rate: u32) {
    // B9 and P5: the filter's response from a saw's harmonics at C1,
    // against the same saw with the filter open, and its analog
    // prototype at zero resonance: LP24 `1 / (1 + s)^4`, LP12 `1 / (1 + s)^2`.
    let saw = |mode: f32, cutoff: f32| {
        let mut n = Engine::Poly.plain(rate);
        n.set_param(30, mode);
        n.set_param(31, cutoff);
        note(n.as_mut(), rate, 24, 100, 2.0, 2.0).mid()
    };
    let from = frames(0.4, rate);
    let open = saw(0.0, 20_000.0);
    let f0 = freq_near(&open, rate, hz(24), from).unwrap_or(hz(24));
    let w = blackman_harris(32_768);
    for (mode, label, order) in [(0.0f32, "LP24", 4.0), (1.0, "LP12", 2.0)] {
        let fc = 1_000.0;
        let x = saw(mode, fc as f32);
        let proto = |f: f64| -10.0 * order * (1.0 + (f / fc).powi(2)).log10();
        let mut pts = Vec::new();
        let mut k = 1.0;
        while k * f0 < 20_000.0 && proto(k * f0) > -30.0 {
            let f = k * f0;
            let a = amp_at(&x[from..from + 32_768], &w, f, rate);
            let b = amp_at(&open[from..from + 32_768], &w, f, rate);
            pts.push((f, db20(a) - db20(b)));
            k += 1.0;
        }
        let low: Vec<f64> = pts
            .iter()
            .filter(|(f, _)| *f < fc / 4.0)
            .map(|(f, g)| g - proto(*f))
            .collect();
        let gain = low.iter().sum::<f64>() / low.len().max(1) as f64;
        let worst = pts
            .iter()
            .filter(|(f, _)| *f >= 50.0)
            .map(|(f, g)| (g - gain - proto(*f)).abs())
            .fold(0.0, f64::max);
        let subject = format!("filter[Poly {label}, 1 kHz]");
        put(
            rows,
            "B9",
            &subject,
            "response_err_db",
            worst,
            Better::Lower,
            Some(1.0),
            0.1,
        );
        // Where it is 3 dB per pole down, against the label.
        let down = -10.0 * order * 2f64.log10();
        let cross = pts.windows(2).find_map(|p| {
            let (a, b) = (p[0].1 - gain, p[1].1 - gain);
            (a >= down && b < down).then(|| {
                let t = (a - down) / (a - b);
                (p[0].0.ln() + t * (p[1].0.ln() - p[0].0.ln())).exp()
            })
        });
        put(
            rows,
            "P5",
            &subject,
            "cutoff_oct",
            cross.map_or(1.0, |f| (f / fc).log2()),
            Better::Near(0.0),
            Some(0.1),
            0.01,
        );
    }
    // B9: where full resonance peaks, on noise, against the cutoff.
    let mut worst = 0.0f64;
    for fc in [250.0f64, 1_000.0, 4_000.0] {
        let mut n = Engine::Poly.plain(rate);
        for (id, v) in [(3u16, 0.0f32), (17, 100.0), (31, fc as f32), (32, 100.0)] {
            n.set_param(id, v);
        }
        let x = note(n.as_mut(), rate, 60, 100, 4.0, 4.0).mid();
        const N: usize = 16_384;
        let win = hann_f64(N);
        let mut avg = vec![0.0; N / 2 + 1];
        let mut at = frames(0.5, rate);
        while at + N <= x.len() {
            for (a, p) in avg.iter_mut().zip(windowed_power(&x[at..at + N], &win)) {
                *a += p;
            }
            at += N / 2;
        }
        let bin = f64::from(rate) / N as f64;
        let lo = (fc / 1.5 / bin) as usize;
        let hi = (fc * 1.5 / bin) as usize;
        let k = (lo..=hi)
            .max_by(|&a, &b| avg[a].total_cmp(&avg[b]))
            .unwrap_or(lo);
        let (a, b, c) = (db10(avg[k - 1]), db10(avg[k]), db10(avg[k + 1]));
        let d = a - 2.0 * b + c;
        let off = if d.abs() > 1e-12 {
            (0.5 * (a - c) / d).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        worst = worst.max(cents((k as f64 + off) * bin, fc).abs());
    }
    put(
        rows,
        "B9",
        "filter[Poly LP24, full resonance]",
        "peak_worst_cents",
        worst,
        Better::Lower,
        Some(25.0),
        1.0,
    );
}

// ---------------------------------------------------------------------------
// FM.

/// The Bessel function of the first kind, order `n`, by its series.
fn bessel_j(n: u32, x: f64) -> f64 {
    let half = x / 2.0;
    let mut term = (1..=n).fold(1.0, |t, k| t * half / f64::from(k));
    let mut sum = term;
    for k in 1..100u32 {
        term *= -half * half / (f64::from(k) * f64::from(k + n));
        sum += term;
        if term.abs() < 1e-18 {
            break;
        }
    }
    sum
}

fn fm(rows: &mut Vec<Row>, rate: u32) {
    // F1: operator 2 at ratio 4 into operator 1 (algorithm 1), at C4:
    // the partials at 1 ± 4n against |Jn(I)|, I fitted.
    let key = 60;
    let f0 = hz(key);
    let w = blackman_harris(32_768);
    let from = frames(0.3, rate);
    for level in [60.0f32, 75.0, 90.0] {
        let mut n = Engine::Fm.plain(rate);
        n.set_param(74, 4.0);
        n.set_param(72, level);
        let x = note(n.as_mut(), rate, key, 100, 1.5, 1.5).mid();
        let seg = &x[from..from + 32_768];
        // (order, frequency) for each partial under 18 kHz.
        let mut parts = Vec::new();
        for order in 0u32..40 {
            let up = (1.0 + 4.0 * f64::from(order)) * f0;
            if up < 18_000.0 {
                parts.push((order, up));
            }
            if order > 0 {
                let down = (4.0 * f64::from(order) - 1.0) * f0;
                if down < 18_000.0 {
                    parts.push((order, down));
                }
            }
        }
        let amps: Vec<f64> = parts
            .iter()
            .map(|&(_, f)| amp_at(seg, &w, f, rate))
            .collect();
        let total = amps.iter().map(|a| a * a).sum::<f64>().sqrt().max(1e-30);
        let norm: Vec<f64> = amps.iter().map(|a| a / total).collect();
        let mut best = (0.0, f64::INFINITY);
        for i in 0..=15_000 {
            let index = f64::from(i) * 0.001;
            let e: f64 = parts
                .iter()
                .zip(&norm)
                .map(|(&(o, _), a)| (a - bessel_j(o, index).abs()).powi(2))
                .sum();
            if e < best.1 {
                best = (index, e);
            }
        }
        let index = best.0;
        let worst = parts
            .iter()
            .zip(&norm)
            .filter(|(&(o, _), _)| bessel_j(o, index).abs() >= 0.01)
            .map(|(&(o, _), a)| (db20(*a) - db20(bessel_j(o, index).abs())).abs())
            .fold(0.0, f64::max);
        let subject = format!("fm[pair 1:4, level {level}]");
        put(
            rows,
            "F1",
            &subject,
            "bessel_err_db",
            worst,
            Better::Lower,
            Some(0.5),
            0.1,
        );
        put(
            rows,
            "F1",
            &subject,
            "index",
            index,
            Better::Info,
            None,
            0.0,
        );
    }

    // F5: operator 6 alone (algorithm 32) at feedback 7 against 0.
    let fb = |amount: f32| {
        let mut n = Engine::Fm.plain(rate);
        for (id, v) in [(0u16, 32.0f32), (1, amount), (40, 0.0), (200, 99.0)] {
            n.set_param(id, v);
        }
        lufs(&note(n.as_mut(), rate, key, 100, 1.0, 1.5))
    };
    put(
        rows,
        "F5",
        "fm[feedback 7 against 0]",
        "level_db",
        fb(7.0) - fb(0.0),
        Better::Near(0.0),
        Some(3.0),
        0.2,
    );

    // F7: one preset under every algorithm.
    let names = Engine::Fm.presets();
    let glass = names.iter().position(|n| n == "Glass Keys").unwrap_or(0);
    let mut louds = Vec::new();
    for algo in 1..=32 {
        let mut n = Engine::Fm.make(glass, rate);
        n.set_param(0, algo as f32);
        louds.push(lufs(&note(n.as_mut(), rate, key, 100, 1.0, 1.5)));
    }
    let spread = louds.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        - louds.iter().copied().fold(f64::INFINITY, f64::min);
    put(
        rows,
        "F7",
        &format!("fm[{}, algorithms 1 to 32]", names[glass]),
        "level_spread_db",
        spread,
        Better::Lower,
        Some(3.0),
        0.3,
    );
}

// ---------------------------------------------------------------------------
// Drums.

fn drums(rows: &mut Vec<Row>, rate: u32) {
    use drum_params::{field, pad_id, PADS};
    for (ki, kit) in Drums::presets().iter().enumerate() {
        let make = || {
            let mut d = Drums::new(rate);
            d.load_preset(&Drums::presets()[ki]);
            d
        };
        let probe = make();
        let base = probe.param(drum_params::BASE_NOTE) as u8;
        let mut kick_level = None;
        for pad in 0..PADS {
            let model = probe.pad_name(pad);
            let label_ms = f64::from(probe.param(pad_id(pad, field::DECAY)));
            let secs = (label_ms * 0.0013 + 0.4).min(7.0);
            let key = base + pad as u8;
            let hit = |vel: u8| {
                let mut d = make();
                note(&mut d, rate, key, vel, 0.05, secs).mid()
            };
            let loud = hit(127);
            let soft = hit(40);
            let subject = format!("Drums[{}, pad {} {model}]", kit.name, pad + 1);

            // D5: time to −60 dB under the loudest 5 ms against the label.
            let e = env_db(&loud, rate, 5.0, 1.0);
            let top = e.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let last = e.iter().rposition(|&v| v > top - 60.0).unwrap_or(0) as f64;
            put(
                rows,
                "D5",
                &subject,
                "decay_err_pct",
                (last - label_ms) / label_ms * 100.0,
                Better::Near(0.0),
                Some(10.0),
                1.0,
            );
            // D3: tone against velocity, over the first 93 ms.
            let (c_lo, c_hi) = (
                centroid(&soft[..4_096], rate),
                centroid(&loud[..4_096], rate),
            );
            let oct = if c_lo > 0.0 && c_hi > 0.0 {
                (c_hi / c_lo).log2()
            } else {
                0.0
            };
            put(
                rows,
                "D3",
                &subject,
                "vel_tone_oct",
                oct,
                Better::Higher,
                Some(0.1),
                0.03,
            );
            // D7: level against the kit's first kick.
            let lv = level_db(&loud, rate);
            if kick_level.is_none() && model == "Kick" {
                kick_level = Some(lv);
            }
            if let Some(k) = kick_level {
                put(
                    rows,
                    "D7",
                    &subject,
                    "level_vs_kick_db",
                    lv - k,
                    Better::Info,
                    None,
                    0.0,
                );
            }
            // D4: the same hit twice, the second's level against the first.
            let mut d = make();
            let gap = frames(secs, rate);
            let ev = vec![
                (
                    0,
                    NodeEvent::NoteOn {
                        note: key,
                        velocity: 127,
                    },
                ),
                (frames(0.05, rate), NodeEvent::NoteOff { note: key }),
                (
                    gap,
                    NodeEvent::NoteOn {
                        note: key,
                        velocity: 127,
                    },
                ),
                (gap + frames(0.05, rate), NodeEvent::NoteOff { note: key }),
            ];
            let two = render(&mut d, rate, 2 * gap, &ev).mid();
            put(
                rows,
                "D4",
                &subject,
                "repeat_db",
                level_db(&two[gap..], rate) - level_db(&two[..gap], rate),
                Better::Info,
                None,
                0.0,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Strings.

/// S7: the swell against its labels on the plain patch at C6, per key and
/// shared: Crescendo from silence to full level, Sustain 60 dB down from
/// full.
fn strings(rows: &mut Vec<Row>, rate: u32) {
    let key = 84;
    let err = |measured: f64, label_ms: f64| (measured * 1000.0 - label_ms) / label_ms * 100.0;
    for (mode, shared) in [("per key", 0.0), ("shared", 1.0)] {
        let run = |cresc: f32, sus: f32, held: f64| {
            let mut n = Engine::Strings.plain(rate);
            n.set_param(20, cresc);
            n.set_param(21, sus);
            n.set_param(22, shared);
            let tail = f64::from(sus) * 0.0016 + 0.2;
            let s = note(n.as_mut(), rate, key, 127, held, held + tail);
            let x = s.mid();
            let f = freq_near(&x, rate, hz(key), 0).unwrap_or(hz(key));
            sine_amp(&x, f, rate, 22)
        };
        let mean = |a: &[(f64, f64)], lo: f64, hi: f64| {
            let v: Vec<f64> = a
                .iter()
                .filter(|(t, _)| (lo..hi).contains(t))
                .map(|p| p.1)
                .collect();
            v.iter().sum::<f64>() / v.len().max(1) as f64
        };
        for cresc in [400.0f32, 1_500.0] {
            let held = f64::from(cresc) / 1000.0 + 1.0;
            let a = run(cresc, 1_000.0, held);
            let full = mean(&a, held - 0.5, held - 0.1);
            let t = a.iter().find(|p| p.1 >= 0.99 * full).map_or(held, |p| p.0);
            put(
                rows,
                "S7",
                &format!("swell[Strings {mode}, crescendo {cresc} ms]"),
                "time_err_pct",
                err(t, f64::from(cresc)),
                Better::Near(0.0),
                Some(10.0),
                1.0,
            );
        }
        for sus in [300.0f32, 1_500.0] {
            let held = 0.5;
            let r = run(5.0, sus, held);
            let full = mean(&r, 0.3, 0.45);
            let t = r
                .iter()
                .find(|p| p.0 > held && p.1 <= 0.001 * full)
                .map_or(held + 2.0 * f64::from(sus) / 1000.0, |p| p.0);
            put(
                rows,
                "S7",
                &format!("swell[Strings {mode}, sustain {sus} ms]"),
                "time_err_pct",
                err(t - held, f64::from(sus)),
                Better::Near(0.0),
                Some(10.0),
                1.0,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Bass.

/// B1: held notes in tune 100 ms in, C0 to C3, the synth body (its plain
/// saw) and the 808 (its preset, the drop and punch over by then). B3: the
/// 808's drop against its curve and its time. V7: glide by rate.
fn bass(rows: &mut Vec<Row>, rate: u32) {
    let b808 = Bass::presets()
        .iter()
        .position(|p| p.name == "808")
        .unwrap_or(0);
    for (what, make) in [
        (
            "synth",
            Box::new(|| Engine::Bass.plain(rate)) as Box<dyn Fn() -> Box<dyn Node>>,
        ),
        ("808", Box::new(move || Engine::Bass.make(b808, rate))),
    ] {
        let mut worst = (0.0f64, 0u8);
        for k in [12u8, 24, 36, 48] {
            let s = note(make().as_mut(), rate, k, 100, 2.5, 2.5);
            let c = freq_near(&s.mid(), rate, hz(k), frames(0.1, rate))
                .map_or(100.0, |f| cents(f, hz(k)));
            if c.abs() > worst.0.abs() {
                worst = (c, k);
            }
        }
        let subject = format!("tune[Bass {what}, C0 to C3]");
        put(
            rows,
            "B1",
            &subject,
            "worst_cents",
            worst.0.abs(),
            Better::Lower,
            Some(3.0),
            0.1,
        );
    }

    // B3: the 808 at C4 with its drop alone, 12 semitones over 200 ms
    // (long enough for each cycle to see the curve), no drive. Each
    // cycle's pitch is the mean of the curve over that cycle.
    let (key, drop, drop_ms) = (60u8, 12.0f64, 200.0f64);
    let mut n = Engine::Bass.make(b808, rate);
    n.set_param(bp::PUNCH, 0.0);
    n.set_param(bp::DRIVE, 0.0);
    n.set_param(bp::DROP, drop as f32);
    n.set_param(bp::DROP_TIME, drop_ms as f32);
    let s = note(n.as_mut(), rate, key, 100, 0.6, 0.6);
    let track = pitch_track(&s.mid(), rate);
    let t_drop = drop_ms / 1000.0;
    let semis = |f: f64| 12.0 * (f / hz(key)).log2();
    let curve = |t: f64| {
        if (0.0..t_drop).contains(&t) {
            drop * (1.0 - t / t_drop).powi(3)
        } else {
            0.0
        }
    };
    // The curve's mean pitch over the cycle centred on `t` at `f`.
    let expect = |t: f64, f: f64| {
        let (a, b) = (t - 0.5 / f, t + 0.5 / f);
        let n = 64;
        let mean = (0..n)
            .map(|i| (curve(a + (b - a) * (i as f64 + 0.5) / n as f64) / 12.0).exp2())
            .sum::<f64>()
            / n as f64;
        12.0 * mean.log2()
    };
    let ratios: Vec<f64> = track
        .iter()
        .filter(|(t, _)| *t < 0.5 * t_drop)
        .map(|(t, f)| semis(*f) / expect(*t, *f))
        .collect();
    let amount = ratios.iter().sum::<f64>() / ratios.len().max(1) as f64;
    // When the pitch comes within 17 cents of the note, against when the
    // curve does.
    let near = 0.17f64;
    let reached = track
        .iter()
        .find(|(_, f)| semis(*f) < near)
        .map_or(0.6, |p| p.0);
    let due = track
        .iter()
        .find(|(t, f)| expect(*t, *f) < near)
        .map_or(t_drop, |p| p.0);
    let subject = "drop[Bass 808, C4, 12 semitones over 200 ms]";
    put(
        rows,
        "B3",
        subject,
        "amount_err_pct",
        (amount - 1.0) * 100.0,
        Better::Near(0.0),
        Some(5.0),
        1.0,
    );
    put(
        rows,
        "B3",
        subject,
        "time_err_pct",
        (reached - due) / due * 100.0,
        Better::Near(0.0),
        Some(5.0),
        1.0,
    );
    let settled: Vec<f64> = track
        .iter()
        .filter(|(t, _)| (0.3..0.55).contains(t))
        .map(|(_, f)| cents(*f, hz(key)))
        .collect();
    put(
        rows,
        "B3",
        subject,
        "settle_cents",
        (settled.iter().sum::<f64>() / settled.len().max(1) as f64).abs(),
        Better::Lower,
        Some(3.0),
        0.1,
    );

    // V7: a sine, legato C2 to C3, gliding at an octave in 300 ms.
    let mut n = Engine::Bass.plain(rate);
    n.set_param(bp::WAVE, bp::SINE);
    n.set_param(bp::GLIDE_MODE, 1.0);
    n.set_param(bp::GLIDE, 300.0);
    let s = score(rate, &[(36, 100, 0.0, 1.5), (48, 100, 0.5, 1.0)]);
    let out = render(n.as_mut(), rate, frames(1.5, rate), &s);
    let track = pitch_track(&out.mid(), rate);
    let target = hz(48);
    let land: Vec<f64> = track
        .iter()
        .filter(|(t, _)| (1.0..1.3).contains(t))
        .map(|(_, f)| cents(*f, target))
        .collect();
    let land = land.iter().sum::<f64>() / land.len().max(1) as f64;
    let arrived = track
        .iter()
        .rposition(|(t, f)| *t < 1.4 && cents(*f, target).abs() > 12.0)
        .map_or(0.5, |i| track[i].0);
    let subject = "glide[Bass rate, an octave in 300 ms]";
    put(
        rows,
        "V7",
        subject,
        "land_cents",
        land.abs(),
        Better::Lower,
        Some(2.0),
        0.2,
    );
    // Within 12 cents of an octave's 1,200 is 99 % of the way.
    put(
        rows,
        "V7",
        subject,
        "time_err_pct",
        ((arrived - 0.5) - 0.3 * 0.99) / (0.3 * 0.99) * 100.0,
        Better::Near(0.0),
        Some(10.0),
        1.0,
    );
}

// ---------------------------------------------------------------------------
// Choir.

/// O1: each section's formants, as its filters run, against the vowel
/// table at C2 (the note far under every formant, so the table is sung as
/// it is): the worst peak and width errors over its five vowels, and the
/// worst level error. O2: the vowel swept from "ooh" to "ee" and back by
/// the knob and by Morph, the loudest sound off the note's harmonics
/// against the loudest harmonic. O8: the singers' wavering against each
/// other, and the stereo correlation of a chord on the Init sound.
fn choir(rows: &mut Vec<Row>, rate: u32) {
    use mpfree_synth::choir::FORMANTS;

    let plain = || {
        let mut c = Choir::new(rate);
        for &(id, v) in &CHOIR_PLAIN {
            c.set_param(id, v);
        }
        c
    };
    for (section, name) in chp::SECTIONS.iter().take(5).enumerate() {
        let (mut peak, mut width, mut level) = (0.0f64, 0.0f64, 0.0f64);
        for vowel in 0..5 {
            let mut c = plain();
            c.set_param(chp::SECTION, section as f32);
            c.set_param(chp::VOWEL, 25.0 * vowel as f32);
            render(&mut c, rate, 512, &score(rate, &[(36, 100, 0.0, 1.0)]));
            let Some(s) = c.busy_slots().next() else {
                continue;
            };
            for k in 0..FORMANTS {
                let (fc, db, bw) = chp::TABLE[section][vowel][k];
                let (fc, bw) = (f64::from(fc), f64::from(bw));
                let g = |f: f64| f64::from(c.formant_gain(s, k, f as f32).unwrap_or(0.0));
                let (mut top, mut at) = (0.0f64, fc);
                let mut f = fc * 0.8;
                while f < fc * 1.2 {
                    if g(f) > top {
                        top = g(f);
                        at = f;
                    }
                    f *= 1.0005;
                }
                let half = top * std::f64::consts::FRAC_1_SQRT_2;
                let (mut lo, mut hi) = (at, at);
                while g(lo) > half && lo > 1.0 {
                    lo /= 1.0002;
                }
                while g(hi) > half && hi < f64::from(rate) / 2.0 {
                    hi *= 1.0002;
                }
                peak = peak.max((at / fc - 1.0).abs() * 100.0);
                width = width.max(((hi - lo) / bw - 1.0).abs() * 100.0);
                level = level.max((db20(top) - f64::from(db)).abs());
            }
        }
        let subject = format!("formants[Choir {name}]");
        put(
            rows,
            "O1",
            &subject,
            "peak_err_pct",
            peak,
            Better::Lower,
            Some(5.0),
            0.2,
        );
        put(
            rows,
            "O1",
            &subject,
            "width_err_pct",
            width,
            Better::Lower,
            Some(20.0),
            1.0,
        );
        put(
            rows,
            "O1",
            &subject,
            "level_err_db",
            level,
            Better::Lower,
            None,
            0.05,
        );
    }

    // O2: the vowel moving from "ooh" to "ee" and back over three
    // seconds, smoothly, by the knob (a change every 32 frames) and by
    // Morph as Vowel Pad sets it, and at rest on "aah"; the note tuned so
    // the formants' control steps (every 16 frames) would sound halfway
    // between its harmonics (and the knob's, every 32, a quarter of the
    // way), near B4 so the gaps are wide: the loudest sound in the middle
    // 60 % of any gap against the loudest harmonic.
    const N: usize = 131_072;
    let from = frames(0.3, rate);
    let ctrl = f64::from(rate) / 16.0;
    let f0 = ctrl / ((ctrl / hz(71) - 0.5).round() + 0.5);
    let key = 71u8;
    let fine = cents(f0, hz(key));
    let mut morph_case = |what: &str, set: &[(u16, f32)], sweep: bool| {
        let mut c = plain();
        c.set_param(chp::SECTION, chp::MIXED);
        c.set_param(chp::FINE, fine as f32);
        for &(id, v) in set {
            c.set_param(id, v);
        }
        let mut ev = score(rate, &[(key, 100, 0.0, 4.0)]);
        if sweep {
            for i in (0..N).step_by(32) {
                let t = i as f64 / N as f64;
                let v = 50.0 - 50.0 * (TAU * t).cos();
                ev.push((
                    from + i,
                    NodeEvent::Param {
                        id: chp::VOWEL,
                        value: v as f32,
                    },
                ));
            }
            ev.sort_by_key(|(f, _)| *f);
        }
        let s = render(&mut c, rate, from + N, &ev);
        let x = s.mid();
        let ps = windowed_power(&x[from..from + N], &blackman_harris(N));
        let bin = f64::from(rate) / N as f64;
        let (mut harm, mut gap) = (0.0f64, 0.0f64);
        for (k, &p) in ps.iter().enumerate() {
            let f = k as f64 * bin;
            if !(20.0..=20_000.0).contains(&f) {
                continue;
            }
            let off = (f / f0 - (f / f0).round()).abs();
            if off * f0 <= 8.0 * bin {
                harm = harm.max(p);
            } else if off >= 0.2 {
                gap = gap.max(p);
            }
        }
        put(
            rows,
            "O2",
            &format!("morph[Choir {what}, B4]"),
            "step_db",
            floor(db10(gap / harm.max(1e-300))),
            Better::Lower,
            Some(-60.0),
            1.0,
        );
    };
    morph_case("at rest", &[(chp::VOWEL, 50.0)], false);
    morph_case("vowel knob ooh to ee and back", &[(chp::VOWEL, 0.0)], true);
    morph_case(
        "Morph 60 at 0.15 Hz",
        &[
            (chp::VOWEL, 50.0),
            (chp::MORPH, 60.0),
            (chp::MORPH_RATE, 0.15),
        ],
        false,
    );

    // O8: eight singers on one note with vibrato and drift and no detune,
    // each singer's pitch in cents every 128 frames for 2 s: the mean of
    // the pairs' correlations, size alone.
    let mut c = Choir::new(rate);
    c.set_param(chp::SINGERS, 8.0);
    c.set_param(chp::DETUNE, 0.0);
    c.set_param(chp::VIBRATO, 30.0);
    c.set_param(chp::VIB_DELAY, 0.0);
    let ev = score(rate, &[(60, 100, 0.0, 10.0)]);
    let warm = frames(0.2, rate) / BLOCK * BLOCK;
    render(&mut c, rate, warm, &ev);
    let slot = c.busy_slots().next();
    let blocks = frames(2.0, rate) / BLOCK;
    let mut tracks: Vec<Vec<f64>> = (0..8).map(|_| Vec::with_capacity(blocks)).collect();
    let (mut bl, mut br) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
    for b in 0..blocks {
        let at = (SETTLE_BLOCKS * BLOCK + warm + b * BLOCK) as u64;
        c.process(&Ctx::new(rate, at), &[], &mut bl, &mut br);
        for (i, t) in tracks.iter_mut().enumerate() {
            let v = slot.and_then(|s| c.singer_cents(s, i)).unwrap_or(0.0);
            t.push(f64::from(v));
        }
    }
    let corr = |a: &[f64], b: &[f64]| {
        let n = a.len() as f64;
        let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
        let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
        for (x, y) in a.iter().zip(b) {
            ab += (x - ma) * (y - mb);
            aa += (x - ma) * (x - ma);
            bb += (y - mb) * (y - mb);
        }
        if aa <= 0.0 || bb <= 0.0 {
            1.0
        } else {
            ab / (aa * bb).sqrt()
        }
    };
    let (mut sum, mut pairs) = (0.0, 0);
    for i in 0..8 {
        for j in i + 1..8 {
            sum += corr(&tracks[i], &tracks[j]).abs();
            pairs += 1;
        }
    }
    put(
        rows,
        "O8",
        "singers[Choir, 8 singers, vibrato 30 cents]",
        "mean_abs_corr",
        sum / f64::from(pairs),
        Better::Lower,
        Some(0.5),
        0.05,
    );
    // The Init sound (Ensemble 75 %) holding C major across two octaves.
    let chord: Vec<(u8, u8, f64, f64)> = [48u8, 55, 60, 64, 67]
        .iter()
        .map(|&k| (k, 100, 0.0, 3.0))
        .collect();
    let s = render(
        Engine::Choir.make(0, rate).as_mut(),
        rate,
        frames(3.0, rate),
        &score(rate, &chord),
    );
    let a = frames(1.0, rate);
    put(
        rows,
        "O8",
        "ensemble[Choir Init, C major chord]",
        "correlation",
        correlation(&s.left[a..], &s.right[a..]),
        Better::Near(0.4),
        Some(0.2),
        0.05,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bessel_matches_known_values() {
        assert!((bessel_j(0, 1.0) - 0.765_197_686_6).abs() < 1e-9);
        assert!((bessel_j(1, 1.0) - 0.440_050_585_7).abs() < 1e-9);
        assert!((bessel_j(2, 5.0) - 0.046_565_116_3).abs() < 1e-9);
        // Every order's power sums to 1.
        let x = 3.7;
        let s = bessel_j(0, x).powi(2) + 2.0 * (1..40).map(|n| bessel_j(n, x).powi(2)).sum::<f64>();
        assert!((s - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_square_law_spans_its_range() {
        assert!(square_law_db(127.0, 30.0).abs() < 1e-9);
        assert!((square_law_db(1.0, 30.0) + 30.0).abs() < 1e-9);
        let pts: Vec<(f64, f64)> = [20.0, 40.0, 64.0, 100.0, 127.0]
            .iter()
            .map(|&v| (v, square_law_db(v, 36.0)))
            .collect();
        let (range, err) = square_law_fit(&pts);
        assert!((range - 36.0).abs() < 0.06 && err < 0.05, "{range} {err}");
    }

    #[test]
    fn a_sine_reads_its_frequency_to_a_hundredth_of_a_cent() {
        let rate = 44_100;
        for f in [32.703, 261.63, 2093.0] {
            let f_true = f * 1.0007;
            let x: Vec<f64> = (0..frames(2.0, rate))
                .map(|i| (TAU * f_true * i as f64 / f64::from(rate)).sin() * 0.5)
                .collect();
            let got = freq_near(&x, rate, f, 4_000).expect("a partial");
            assert!(
                cents(got, f_true).abs() < 0.01,
                "{f}: {got} against {f_true}"
            );
        }
    }

    #[test]
    fn a_band_limited_saw_has_no_alias_and_a_naive_one_does() {
        let rate = 44_100;
        let f = 2093.0;
        let n = frames(2.0, rate);
        let blit: Vec<f64> = (0..n)
            .map(|i| {
                let t = TAU * f * i as f64 / f64::from(rate);
                (1..=9)
                    .map(|k| (k as f64 * t).sin() / k as f64)
                    .sum::<f64>()
                    * 0.3
            })
            .collect();
        assert!(alias_db(&blit, rate, f, 4_000) < -85.0);
        let naive: Vec<f64> = (0..n)
            .map(|i| ((f * i as f64 / f64::from(rate)).fract() * 2.0 - 1.0) * 0.3)
            .collect();
        assert!(alias_db(&naive, rate, f, 4_000) > -40.0);
    }
}
