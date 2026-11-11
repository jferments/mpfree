//! The audio-node interface shared by MPFree's instruments (`mpfree-synth`)
//! and effects (`mpfree-fx`).
//!
//! A [`Node`] is anything that runs on the audio thread on a stereo bus: a
//! synth, a filter, a delay, the master limiter. The host (the engine, a
//! chain, a test, an offline render) owns the buffers and calls
//! [`Node::process`] once per block with the events that fall in it.
//!
//! The rules every node keeps, so any node can go in any chain:
//!
//! - **Buffers.** Stereo `f32`, not interleaved, processed in place. An
//!   effect transforms what is in the buffers. An instrument **adds** to
//!   them, so several instruments can share a bus and an instrument can sit
//!   first in a chain on zeroed buffers.
//! - **Gain.** 1.0 is 0 dBFS. Buffers are unbounded `f32`: no node clamps
//!   or saturates unless that is its job (drive, crush, a clipper), so
//!   deliberate distortion stays possible. Instruments aim for about
//!   -12 dBFS peak for one full-velocity note at their defaults and fade
//!   every voice's end. Every output ends in a master node (DC block,
//!   headroom trim, a safety limiter that counts overs into a meter and
//!   never errors or mutes).
//! - **Real time.** [`Node::process`] never allocates, frees, locks, blocks
//!   or panics. Anything that needs memory gets it in [`Node::prepare`],
//!   which runs on the control thread.
//! - **Block size.** Events carry frame offsets inside the block. A node
//!   gives the same output, sample for sample, however the host cuts the
//!   same stream of frames and events into blocks of 1 to [`MAX_BLOCK`]
//!   frames. Work done at a control rate is aligned to the absolute frame
//!   ([`Ctx::frame`]), never to the block's start.
//! - **Parameters** are plain values in the units their [`ParamInfo`] gives.
//!   Each node smooths its own continuous parameters, so a jump never
//!   clicks. Values out of range are clamped, never refused.
//! - **NaN.** A node must not output non-finite samples. If one does, the
//!   host resets it ([`Node::reset`]) and counts it.
#![forbid(unsafe_code)]

/// The most frames a host passes in one call.
pub const MAX_BLOCK: usize = 8192;

/// Something that happens on an exact frame of a block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NodeEvent {
    /// A note starts. `note` is a MIDI note number (60 = middle C).
    NoteOn { note: u8, velocity: u8 },
    NoteOff { note: u8 },
    /// Pressure on a held note, 0 to 127 (polyphonic aftertouch).
    Pressure { note: u8, value: u8 },
    /// Pitch bend for every note, -1 to 1 (the node's bend range scales it).
    Bend { value: f32 },
    /// Ends every note with its release. Silences nothing abruptly.
    AllOff,
    /// Sets a parameter on this frame (automation, a parameter lock).
    Param { id: u16, value: f32 },
}

/// A [`NodeEvent`] at a frame offset inside the block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timed {
    pub offset: u32,
    pub ev: NodeEvent,
}

impl Timed {
    pub fn new(offset: u32, ev: NodeEvent) -> Timed { Timed { offset, ev } }
}

/// What the host knows about the block being processed.
#[derive(Clone, Copy, Debug)]
pub struct Ctx<'a> {
    /// Sample rate in Hz.
    pub rate: u32,
    /// The absolute frame of the block's first frame.
    pub frame: u64,
    /// Tempo in beats per minute.
    pub bpm: f64,
    /// Transport position in beats at the block's first frame.
    pub beats: f64,
    /// Whether the transport is playing.
    pub playing: bool,
    /// A sidechain input (for ducking), the same length as the block.
    pub sidechain: Option<(&'a [f32], &'a [f32])>,
}

impl<'a> Ctx<'a> {
    /// A context for offline work: 120 BPM, stopped, no sidechain.
    pub fn new(rate: u32, frame: u64) -> Ctx<'static> {
        Ctx { rate, frame, bpm: 120.0, beats: 0.0, playing: false, sidechain: None }
    }
}

/// Whether a node makes sound from notes or changes the sound it is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Instrument,
    Effect,
}

/// What a node is, for the host and the GUI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeInfo {
    /// A stable id for project files, such as `"mpfree.poly"`.
    pub id: &'static str,
    /// The name shown to the user.
    pub name: &'static str,
    pub kind: Kind,
}

/// The unit a parameter is shown in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    None,
    Hz,
    Db,
    Percent,
    Ms,
    Semitones,
    Cents,
    Octaves,
    Beats,
}

/// How a control moves through a parameter's range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    Linear,
    /// Even steps in ratio (cutoff, time). Needs `min > 0`.
    Log,
    /// Whole numbers from `min` to `max`.
    Stepped,
    /// Named choices: the value is the index, `min` 0, `max` `len - 1`.
    Choice(&'static [&'static str]),
    /// Off (0) or on (1).
    Toggle,
}

/// A parameter's description, enough for a generic page of knobs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamInfo {
    /// Stable within the node: project files and automation keep it.
    pub id: u16,
    pub name: &'static str,
    /// The page section it belongs on, such as `"Filter"`.
    pub group: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: Unit,
    pub curve: Curve,
    /// A meter: the node writes it and [`Node::param`] reads the live value.
    /// Setting it does nothing.
    pub readonly: bool,
}

impl ParamInfo {
    /// Clamps to the range and, for stepped curves, rounds.
    pub fn clamp(&self, v: f32) -> f32 {
        let v = if v.is_finite() { v } else { self.default };
        let v = v.clamp(self.min, self.max);
        match self.curve {
            Curve::Stepped | Curve::Choice(_) => v.round(),
            Curve::Toggle => if v >= 0.5 { 1.0 } else { 0.0 },
            _ => v,
        }
    }

    /// The value at control position `t` (0 to 1), following the curve.
    pub fn from_unit(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let v = match self.curve {
            Curve::Log if self.min > 0.0 => self.min * (self.max / self.min).powf(t),
            _ => self.min + (self.max - self.min) * t,
        };
        self.clamp(v)
    }

    /// The control position (0 to 1) of a value, following the curve.
    pub fn to_unit(&self, v: f32) -> f32 {
        let v = self.clamp(v);
        if self.max <= self.min { return 0.0; }
        match self.curve {
            Curve::Log if self.min > 0.0 => (v / self.min).ln() / (self.max / self.min).ln(),
            _ => (v - self.min) / (self.max - self.min),
        }
    }

    /// Checks the description itself: a sane range and default.
    pub fn check(&self) -> Result<(), String> {
        let bad = |why: &str| Err(format!("param {} ({}): {why}", self.id, self.name));
        if !(self.min.is_finite() && self.max.is_finite() && self.default.is_finite()) { return bad("not finite"); }
        if self.min > self.max { return bad("min above max"); }
        if self.default < self.min || self.default > self.max { return bad("default out of range"); }
        match self.curve {
            Curve::Log if self.min <= 0.0 => return bad("log curve needs min > 0"),
            Curve::Choice(names) if names.is_empty() || self.min != 0.0 || self.max != (names.len() - 1) as f32 => return bad("choice range must be 0..len-1"),
            Curve::Toggle if self.min != 0.0 || self.max != 1.0 => return bad("toggle range must be 0..1"),
            _ => {}
        }
        Ok(())
    }
}

/// Checks a node's parameter list: each description, and unique ids.
pub fn check_params(params: &[ParamInfo]) -> Result<(), String> {
    for (i, p) in params.iter().enumerate() {
        p.check()?;
        if params[..i].iter().any(|q| q.id == p.id) { return Err(format!("param id {} used twice", p.id)); }
    }
    Ok(())
}

/// An instrument or effect on the audio thread. See the crate docs for the
/// rules.
pub trait Node: Send {
    fn info(&self) -> &'static NodeInfo;
    fn params(&self) -> &'static [ParamInfo];
    /// Sets a parameter (clamped). Takes effect from the next frame
    /// processed, smoothed where the parameter is continuous.
    fn set_param(&mut self, id: u16, value: f32);
    /// The current value (the target, not the smoothed value), or a meter's
    /// live value. NaN for an unknown id.
    fn param(&self, id: u16) -> f32;
    /// Called on the control thread before the first `process` and whenever
    /// the rate or the largest block changes. May allocate.
    fn prepare(&mut self, rate: u32, max_block: usize);
    /// Silences the node at once: ends every voice and clears every state.
    /// Keeps the parameters.
    fn reset(&mut self);
    /// Processes one block. `events` are sorted by offset and every offset
    /// is below the block length. `left` and `right` have the same length,
    /// at most the `max_block` given to [`Node::prepare`].
    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]);
    /// Frames the node keeps sounding after its input stops (a release, a
    /// reverb tail), so a host knows when it may stop calling it.
    fn tail_frames(&self) -> u32 { 0 }
    /// Frames of delay the node adds (look-ahead), for compensation.
    fn latency(&self) -> u32 { 0 }
}

/// A one-pole smoother for a continuous parameter, stepped once per frame.
#[derive(Clone, Copy, Debug)]
pub struct Smooth {
    pub value: f32,
    pub target: f32,
    coef: f32,
}

impl Smooth {
    /// Starts settled at `v`.
    pub fn new(v: f32) -> Smooth { Smooth { value: v, target: v, coef: 1.0 } }

    /// Sets the time constant: about `ms` milliseconds to cover 63 % of a jump.
    pub fn set_time(&mut self, rate: u32, ms: f32) {
        let n = (ms * 0.001 * rate as f32).max(1.0);
        self.coef = 1.0 - (-1.0 / n).exp();
    }

    /// Jumps straight to `v` (on reset, or a value with no zipper risk).
    pub fn snap(&mut self, v: f32) { self.value = v; self.target = v; }

    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f32 {
        let next = self.value + (self.target - self.value) * self.coef;
        // Snap once the step is lost in rounding or the gap is tiny.
        self.value = if next == self.value || (self.target - next).abs() < 1e-6 { self.target } else { next };
        self.value
    }

    pub fn settled(&self) -> bool { self.value == self.target }
}

/// Splits a block at its events and calls `run(from, to)` for each piece and
/// `apply(event)` for each event, in order. The usual shape of a node's
/// `process`, so events land on their exact frame.
pub fn split<E: FnMut(&NodeEvent), R: FnMut(usize, usize)>(len: usize, events: &[Timed], mut apply: E, mut run: R) {
    let mut at = 0usize;
    let mut i = 0usize;
    while at < len {
        while i < events.len() && (events[i].offset as usize) <= at {
            apply(&events[i].ev);
            i += 1;
        }
        let next = events.get(i).map_or(len, |e| (e.offset as usize).min(len));
        if next > at { run(at, next); }
        at = next;
    }
    // Events at or past the end (a host error) still apply, so no note hangs.
    for e in &events[i..] { apply(&e.ev); }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: ParamInfo = ParamInfo { id: 1, name: "Cutoff", group: "Filter", min: 20.0, max: 20000.0, default: 1000.0, unit: Unit::Hz, curve: Curve::Log, readonly: false };

    #[test]
    fn log_curve_round_trips() {
        assert!((P.from_unit(0.0) - 20.0).abs() < 1e-3);
        assert!((P.from_unit(1.0) - 20000.0).abs() < 1.0);
        assert!((P.from_unit(0.5) - 632.46).abs() < 0.1);
        for v in [20.0, 100.0, 1000.0, 15000.0] {
            assert!((P.from_unit(P.to_unit(v)) - v).abs() / v < 1e-4);
        }
    }

    #[test]
    fn clamp_steps_and_choices() {
        let s = ParamInfo { curve: Curve::Stepped, min: -2.0, max: 2.0, default: 0.0, ..P };
        assert_eq!(s.clamp(1.4), 1.0);
        assert_eq!(s.clamp(9.0), 2.0);
        assert_eq!(s.clamp(f32::NAN), 0.0);
        let c = ParamInfo { curve: Curve::Choice(&["a", "b", "c"]), min: 0.0, max: 2.0, default: 0.0, ..P };
        assert!(c.check().is_ok());
        assert_eq!(c.clamp(1.6), 2.0);
        let t = ParamInfo { curve: Curve::Toggle, min: 0.0, max: 1.0, default: 0.0, ..P };
        assert_eq!(t.clamp(0.7), 1.0);
    }

    #[test]
    fn check_finds_bad_descriptions() {
        assert!(P.check().is_ok());
        assert!(ParamInfo { default: 5.0, ..P }.check().is_err());
        assert!(ParamInfo { min: 0.0, ..P }.check().is_err());
        assert!(check_params(&[P, P]).is_err());
        assert!(ParamInfo { curve: Curve::Choice(&["a"]), min: 0.0, max: 2.0, default: 0.0, ..P }.check().is_err());
    }

    #[test]
    fn split_lands_events_on_their_frames() {
        let ev = [Timed::new(0, NodeEvent::AllOff), Timed::new(3, NodeEvent::NoteOff { note: 1 }), Timed::new(3, NodeEvent::NoteOff { note: 2 }), Timed::new(7, NodeEvent::AllOff)];
        let mut log = Vec::new();
        let log_cell = std::cell::RefCell::new(&mut log);
        split(10, &ev, |e| log_cell.borrow_mut().push(format!("{e:?}")), |a, b| log_cell.borrow_mut().push(format!("{a}..{b}")));
        assert_eq!(log, ["AllOff", "0..3", "NoteOff { note: 1 }", "NoteOff { note: 2 }", "3..7", "AllOff", "7..10"]);
    }

    #[test]
    fn smooth_settles() {
        let mut s = Smooth::new(0.0);
        s.set_time(48_000, 5.0);
        s.target = 1.0;
        for _ in 0..48_000 { s.next(); }
        assert!(s.settled());
        assert_eq!(s.value, 1.0);
    }
}
