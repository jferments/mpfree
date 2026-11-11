//! The mixer: inputs, pad channels, tracks, submixes, send returns and the
//! master, ending in Master safety. It covers the routing of the MPC 3.6
//! mixer (see `docs/dev/design/fx.md`, "Mixer").
//!
//! The signal runs in fixed stages, so nothing can feed back:
//!
//! 1. **Inputs** (up to [`MAX_INPUTS`]): live audio from the line input,
//!    the stereo pair or either side as mono ([`ids::SOURCE`]), heard
//!    through the strip's inserts when monitoring says so
//!    ([`ids::MONITOR`], [`ids::ARM`]).
//! 2. **Pad channels** (a pool of up to [`MAX_PADS`]): pads that need
//!    their own inserts, sends or output, rendered apart by the sampler.
//! 3. **Tracks** (up to [`MAX_TRACKS`]): the host adds each instrument
//!    into its track's input; inputs and pad channels can be routed in.
//! 4. **Submixes** ([`BUSES`]): sum what is routed to them. A submix can
//!    go on to a higher-numbered submix or the master.
//! 5. **Returns** (A to D): every strip of the stages above has four sends,
//!    each before or after its fader. Returns are solo-safe.
//! 6. **Master**: inserts and fader, then [`MasterSafety`], always last.
//!
//! Every strip has an insert [`Chain`] and a fader (volume, balance, mute,
//! solo), and every strip but a return and the master has an output
//! ([`ids::OUTPUT`]): the master, a submix, a track (inputs and pads only)
//! or nowhere.
//!
//! **Solo** is in place: soloing a strip keeps it, every strip it feeds
//! and every strip that feeds it; returns and the master are never cut,
//! and mute wins.
//!
//! **Sidechain keys** are set per insert slot ([`Mixer::set_key`]): the
//! key is another strip's signal after its inserts, before its fader and
//! mute (so a silent ghost kick still pumps the mix). A strip can be keyed
//! by any strip in an earlier stage, any strip in its own stage (the mixer
//! orders each stage so the key is ready, and refuses a loop), except that
//! a submix can only be keyed by a lower-numbered one. Keys cost no delay.
//!
//! **Latency is compensated:** effects with look-ahead or oversampling
//! report latency, and every path into a track, a submix, a return and the
//! master is delayed to line up with the slowest path that could reach it,
//! so routing changes and fader moves never move anything in time; only
//! adding, removing or swapping an effect with latency does, and then the
//! delays crossfade over 20 ms. Live inputs are the exception: they are
//! never held back to line up, so playing through the mixer stays as
//! immediate as possible (straight to the master, an input waits only for
//! its own inserts and Master safety; through a track or a submix, it
//! keeps that strip's place in time), and a recorder lines a take up
//! with [`Mixer::capture_latency`]. Compensation stops at a maximum set when the
//! mixer is built (50 ms by default); past it, [`MixerStats::misaligned`].
//!
//! **Click-free:** volume, pan, sends, mute, solo and monitoring glide (a
//! one-pole smoother of 4 ms); changing an output crossfades from the old
//! destination to the new one. An [`Edit`] (adding, removing, swapping or
//! moving an insert while the mixer plays) ducks that strip out over 10
//! ms, makes the change while it is silent, waits for any new latency to
//! fill, and brings it back over 10 ms; the rest of the mix carries on.
//!
//! **Capture points** ([`Mixer::set_capture`]) hand a recorder the signal
//! of any strip before its inserts, before its fader or after it, or the
//! master after Master safety: resampling, the looper, recording an input
//! dry or through its effects, bouncing and stems.
//!
//! **Sample-timed:** strip controls take [`Timed`] parameter events with
//! the ids in [`ids`] (automation, quantized mutes), and insert parameters
//! take them through [`Mixer::queue_insert`]. The output is the same,
//! sample for sample, for any block size, including mute, solo and output
//! changes that affect other strips. The exceptions are an [`Edit`], which
//! starts at the next block and so can land up to one block later, and
//! [`Mixer::set_key`], which takes effect from the next block.
//!
//! **Cheap when quiet:** a strip that has been silent for longer than its
//! compensation delays, with nothing gliding and no events, skips its
//! fader and routing work. A strip whose input nothing wrote this block
//! (no producer took it, nothing routed in) does not even scan it, so an
//! unused pad channel costs a few flag checks.
//!
//! **Profiling** ([`Mixer::set_profiling`], off by default): the mixer
//! times each block, each strip (inserts, fader and routing) and each
//! insert slot into `mpfree-prof` probes, for the Dev tools pages and
//! `examples/mixer_cost.rs`.
//!
//! **Real-time safe:** building the mixer, [`Mixer::chain_mut`] and
//! [`Insert::new`] allocate and belong on the control thread; everything
//! else, including [`Mixer::edit`], [`Mixer::set_key`] and
//! [`Mixer::process`], never allocates, locks or panics. What an edit takes
//! out comes back through [`Mixer::pop_retired`], to be dropped off the
//! audio thread.

use std::f32::consts::FRAC_PI_2;

use mpfree_node::{Ctx, Curve, NodeEvent, ParamInfo, Smooth, Timed, Unit};
use mpfree_prof::{Probe, Span};

use crate::chain::{Chain, Insert};
use crate::{db_to_gain, MasterSafety};

/// Submixes every mixer has.
pub const BUSES: usize = 8;
/// Send returns every mixer has, A to D.
pub const RETURNS: usize = 4;
/// The most input strips a mixer can be built with.
pub const MAX_INPUTS: usize = 8;
/// The largest pad channel pool a mixer can be built with.
pub const MAX_PADS: usize = 128;
/// The most tracks a mixer can be built with.
pub const MAX_TRACKS: usize = 64;
/// The most insert slots a strip can be built with.
pub const MAX_SLOTS: usize = 16;
/// Capture points every mixer has.
pub const CAPTURES: usize = 8;
/// Events that can be queued for one strip per block.
pub const MAX_STRIP_EVENTS: usize = 256;
/// A volume or send level at or below this is silent.
pub const SILENT_DB: f32 = -80.0;

const MAX_STRIPS: usize = MAX_INPUTS + MAX_PADS + MAX_TRACKS + BUSES + RETURNS + 1;
const MAX_EDITS: usize = 32;
const MAX_GLOBALS: usize = 1024;
const SMOOTH_MS: f32 = 4.0;
const DUCK_MS: f32 = 10.0;
const TAP_FADE_MS: f32 = 20.0;
/// Destinations a strip can be crossfading between at once.
const ROUTES: usize = 3;
const NONE: u16 = u16::MAX;
/// The fader and the routing run in pieces of at most this many frames, so
/// the compensation delay lines need only this much room past their delay.
const CHUNK: usize = 256;

/// Strip parameter ids, for [`Mixer::queue`], [`Mixer::set_now`] and
/// [`Mixer::param`]. Every strip lists the same parameters
/// ([`strip_params`]); [`param_applies`] says which a strip uses.
pub mod ids {
    /// dB, [`super::SILENT_DB`] to +6; at or below the bottom it is silent.
    pub const VOLUME: u16 = 0;
    /// -100 (left) to 100 (right) %, a balance control: the far side fades
    /// with a quarter-cosine and the near side stays at unity.
    pub const PAN: u16 = 1;
    pub const MUTE: u16 = 2;
    pub const SOLO: u16 = 3;
    /// Where the strip goes, one of [`output`].
    pub const OUTPUT: u16 = 4;
    /// The track an input or pad channel goes to when its output is
    /// [`output::TRACK`], 0 to [`super::MAX_TRACKS`] - 1.
    pub const OUTPUT_TRACK: u16 = 5;
    /// The send level to return `r` (0 to 3, A to D), dB.
    pub const fn send(r: usize) -> u16 {
        6 + r as u16
    }
    /// Where send `r` is taken: 0 after the fader, 1 before it.
    pub const fn send_pre(r: usize) -> u16 {
        10 + r as u16
    }
    /// What an input strip hears, one of [`source`].
    pub const SOURCE: u16 = 14;
    /// When an input strip is heard, one of [`monitor`].
    pub const MONITOR: u16 = 15;
    /// Whether the recorder an input feeds is armed (for
    /// [`monitor::AUTO`]).
    pub const ARM: u16 = 16;

    /// Values of [`OUTPUT`].
    pub mod output {
        pub const MASTER: f32 = 0.0;
        /// Submix `b`, 0 to [`crate::mixer::BUSES`] - 1.
        pub fn bus(b: usize) -> f32 {
            1.0 + b as f32
        }
        /// The track in [`super::OUTPUT_TRACK`] (inputs and pads only).
        pub const TRACK: f32 = 9.0;
        /// Nowhere: only the sends go on.
        pub const NONE: f32 = 10.0;
    }

    /// Values of [`SOURCE`].
    pub mod source {
        pub const OFF: f32 = 0.0;
        /// Inputs 1 and 2 as a stereo pair.
        pub const STEREO: f32 = 1.0;
        /// Input 1 alone, on both sides.
        pub const LEFT: f32 = 2.0;
        /// Input 2 alone, on both sides.
        pub const RIGHT: f32 = 3.0;
    }

    /// Values of [`MONITOR`].
    pub mod monitor {
        pub const OFF: f32 = 0.0;
        pub const ON: f32 = 1.0;
        /// Heard only while [`super::ARM`] is on.
        pub const AUTO: f32 = 2.0;
    }
}

const OUTPUTS: &[&str] = &[
    "Master", "Sub 1", "Sub 2", "Sub 3", "Sub 4", "Sub 5", "Sub 6", "Sub 7", "Sub 8", "Track",
    "None",
];
const TAPS: &[&str] = &["Post", "Pre"];
const SOURCES: &[&str] = &["Off", "In 1+2", "In 1", "In 2"];
const MONITORS: &[&str] = &["Off", "On", "Auto"];
const OUT_TRACK: u8 = 9;
const OUT_NONE: u8 = 10;

const fn send_param(r: usize, name: &'static str) -> ParamInfo {
    ParamInfo {
        id: ids::send(r),
        name,
        group: "Sends",
        min: SILENT_DB,
        max: 6.0,
        default: SILENT_DB,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    }
}

const fn tap_param(r: usize, name: &'static str) -> ParamInfo {
    ParamInfo {
        id: ids::send_pre(r),
        name,
        group: "Sends",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(TAPS),
        readonly: false,
    }
}

static PARAMS: [ParamInfo; 17] = [
    ParamInfo {
        id: ids::VOLUME,
        name: "Volume",
        group: "Fader",
        min: SILENT_DB,
        max: 6.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: ids::PAN,
        name: "Pan",
        group: "Fader",
        min: -100.0,
        max: 100.0,
        default: 0.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: ids::MUTE,
        name: "Mute",
        group: "Fader",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Toggle,
        readonly: false,
    },
    ParamInfo {
        id: ids::SOLO,
        name: "Solo",
        group: "Fader",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Toggle,
        readonly: false,
    },
    ParamInfo {
        id: ids::OUTPUT,
        name: "Output",
        group: "Output",
        min: 0.0,
        max: 10.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(OUTPUTS),
        readonly: false,
    },
    ParamInfo {
        id: ids::OUTPUT_TRACK,
        name: "Output track",
        group: "Output",
        min: 0.0,
        max: (MAX_TRACKS - 1) as f32,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Stepped,
        readonly: false,
    },
    send_param(0, "Send A"),
    send_param(1, "Send B"),
    send_param(2, "Send C"),
    send_param(3, "Send D"),
    tap_param(0, "Send A from"),
    tap_param(1, "Send B from"),
    tap_param(2, "Send C from"),
    tap_param(3, "Send D from"),
    ParamInfo {
        id: ids::SOURCE,
        name: "Source",
        group: "Input",
        min: 0.0,
        max: 3.0,
        default: 1.0,
        unit: Unit::None,
        curve: Curve::Choice(SOURCES),
        readonly: false,
    },
    ParamInfo {
        id: ids::MONITOR,
        name: "Monitor",
        group: "Input",
        min: 0.0,
        max: 2.0,
        default: 2.0,
        unit: Unit::None,
        curve: Curve::Choice(MONITORS),
        readonly: false,
    },
    ParamInfo {
        id: ids::ARM,
        name: "Armed",
        group: "Input",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Toggle,
        readonly: false,
    },
];

/// The parameters of a mixer strip, for a generic page (filter them with
/// [`param_applies`]).
pub fn strip_params() -> &'static [ParamInfo] {
    &PARAMS
}

/// Whether a strip of this kind uses a parameter: returns have no solo,
/// output or sends, the master only volume, pan and mute, only inputs and
/// pad channels go to a track, and only inputs have a source and
/// monitoring.
pub fn param_applies(s: StripId, id: u16) -> bool {
    let role = match s {
        StripId::Input(_) => Role::Input,
        StripId::Pad(_) => Role::Pad,
        StripId::Track(_) => Role::Track,
        StripId::Bus(_) => Role::Bus,
        StripId::Return(_) => Role::Return,
        StripId::Master => Role::Master,
    };
    match id {
        ids::VOLUME | ids::PAN | ids::MUTE => true,
        ids::SOLO | ids::OUTPUT | 6..=13 => role.has_sends(),
        ids::OUTPUT_TRACK => matches!(role, Role::Input | Role::Pad),
        ids::SOURCE | ids::MONITOR | ids::ARM => role == Role::Input,
        _ => false,
    }
}

/// Mute, solo and output change what other strips do, so they are
/// gathered across the mixer and replayed by every strip.
fn is_global(id: u16) -> bool {
    matches!(id, ids::MUTE | ids::SOLO | ids::OUTPUT | ids::OUTPUT_TRACK)
}

/// One strip of the mixer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StripId {
    Input(usize),
    Pad(usize),
    Track(usize),
    Bus(usize),
    Return(usize),
    Master,
}

/// Where a capture point takes its signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CapturePoint {
    /// The strip's input, before its inserts (an input recorded dry).
    BeforeInserts,
    /// After the inserts, before the fader and mute.
    BeforeFader,
    /// After the fader, mute and solo: what the strip sends on.
    AfterFader,
    /// The master only: after Master safety, what is heard.
    AfterSafety,
}

/// How a mixer is built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixerConfig {
    /// Input strips, 0 to [`MAX_INPUTS`].
    pub inputs: usize,
    /// Pad channels, 0 to [`MAX_PADS`].
    pub pads: usize,
    /// Tracks, 1 to [`MAX_TRACKS`].
    pub tracks: usize,
    /// Insert slots every strip has room for without allocating, 0 to
    /// [`MAX_SLOTS`].
    pub slots: usize,
    /// The most latency compensation any path gets, ms.
    pub max_latency_ms: f32,
}

impl Default for MixerConfig {
    fn default() -> Self {
        MixerConfig {
            inputs: 2,
            pads: 32,
            tracks: 16,
            slots: 8,
            max_latency_ms: 50.0,
        }
    }
}

/// A change to a strip's inserts while the mixer plays. See the module docs.
pub enum Edit {
    /// Puts `insert` in at slot `at` (0 to the number of slots). A refused
    /// insert (no room, or a slot id another slot has: see
    /// [`Chain::insert`]) comes back through [`Mixer::pop_retired`] and
    /// counts in [`MixerStats::bad_edits`], so a host loading a project
    /// looks there. Retired inserts keep their slot id: call
    /// [`Insert::without_uid`] before using one on another strip.
    Insert {
        strip: StripId,
        at: usize,
        insert: Insert,
    },
    /// Swaps the insert at `at` for `insert`; the old one is retired. A
    /// refused one comes back as for [`Edit::Insert`].
    Replace {
        strip: StripId,
        at: usize,
        insert: Insert,
    },
    /// Takes the insert at `at` out; it is retired.
    Remove { strip: StripId, at: usize },
    /// Moves the insert at `from` to `to`, with its state and its key.
    Move {
        strip: StripId,
        from: usize,
        to: usize,
    },
}

impl Edit {
    pub fn strip(&self) -> StripId {
        match self {
            Edit::Insert { strip, .. }
            | Edit::Replace { strip, .. }
            | Edit::Remove { strip, .. }
            | Edit::Move { strip, .. } => *strip,
        }
    }
}

/// Counters kept by a [`Mixer`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MixerStats {
    /// Strip or insert events dropped because a queue was full.
    pub dropped_events: u64,
    /// Blocks where a strip's input held a non-finite sample (it was
    /// silenced for that block).
    pub bad_input: u64,
    /// Edits that did not fit (a slot that is not there, a full chain, an
    /// insert prepared for another rate); their insert was retired.
    pub bad_edits: u64,
    /// Times an insert put out a non-finite sample and was reset.
    pub nan_resets: u64,
    /// Whether some path needs more latency compensation than the maximum.
    pub misaligned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Role {
    Input,
    Pad,
    Track,
    Bus,
    Return,
    Master,
}

impl Role {
    /// Strips with sends, solo and an output.
    fn has_sends(self) -> bool {
        self < Role::Return
    }
}

/// A set of strips.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Bits([u64; MAX_STRIPS.div_ceil(64)]);

impl Bits {
    #[inline]
    fn get(&self, k: usize) -> bool {
        (self.0[k / 64] >> (k % 64)) & 1 == 1
    }

    #[inline]
    fn set(&mut self, k: usize, on: bool) {
        let m = 1u64 << (k % 64);
        if on {
            self.0[k / 64] |= m
        } else {
            self.0[k / 64] &= !m
        }
    }

    fn any(&self) -> bool {
        self.0.iter().any(|&w| w != 0)
    }
}

/// Mute, solo and output for the whole mixer, as of one frame.
#[derive(Clone, Copy)]
struct Switches {
    mute: Bits,
    solo: Bits,
    out: [u8; MAX_STRIPS],
    out_track: [u8; MAX_STRIPS],
}

impl Switches {
    fn new() -> Switches {
        Switches {
            mute: Bits::default(),
            solo: Bits::default(),
            out: [0; MAX_STRIPS],
            out_track: [0; MAX_STRIPS],
        }
    }
}

/// Where each stage starts in the strip order.
#[derive(Clone, Copy, Debug)]
struct Topo {
    inputs: usize,
    pads: usize,
    tracks: usize,
}

impl Topo {
    fn pad(&self, p: usize) -> usize {
        self.inputs + p
    }
    fn track(&self, t: usize) -> usize {
        self.inputs + self.pads + t
    }
    fn bus(&self, b: usize) -> usize {
        self.inputs + self.pads + self.tracks + b
    }
    fn ret(&self, r: usize) -> usize {
        self.bus(BUSES) + r
    }
    fn master(&self) -> usize {
        self.ret(RETURNS)
    }
    fn len(&self) -> usize {
        self.master() + 1
    }

    fn order(&self, s: StripId) -> Option<usize> {
        match s {
            StripId::Input(i) if i < self.inputs => Some(i),
            StripId::Pad(p) if p < self.pads => Some(self.pad(p)),
            StripId::Track(t) if t < self.tracks => Some(self.track(t)),
            StripId::Bus(b) if b < BUSES => Some(self.bus(b)),
            StripId::Return(r) if r < RETURNS => Some(self.ret(r)),
            StripId::Master => Some(self.master()),
            _ => None,
        }
    }

    fn role(&self, k: usize) -> (Role, usize) {
        if k < self.inputs {
            (Role::Input, k)
        } else if k < self.track(0) {
            (Role::Pad, k - self.inputs)
        } else if k < self.bus(0) {
            (Role::Track, k - self.track(0))
        } else if k < self.ret(0) {
            (Role::Bus, k - self.bus(0))
        } else if k < self.master() {
            (Role::Return, k - self.ret(0))
        } else {
            (Role::Master, 0)
        }
    }

    fn id(&self, k: usize) -> StripId {
        match self.role(k) {
            (Role::Input, i) => StripId::Input(i),
            (Role::Pad, i) => StripId::Pad(i),
            (Role::Track, i) => StripId::Track(i),
            (Role::Bus, i) => StripId::Bus(i),
            (Role::Return, i) => StripId::Return(i),
            (Role::Master, _) => StripId::Master,
        }
    }

    /// The strips of each stage, and whether the mixer may order them by
    /// their keys (submixes always run in order, as they feed upward).
    fn stages(&self) -> [(usize, usize, bool); 6] {
        [
            (0, self.inputs, true),
            (self.pad(0), self.track(0), true),
            (self.track(0), self.bus(0), true),
            (self.bus(0), self.ret(0), false),
            (self.ret(0), self.master(), true),
            (self.master(), self.len(), false),
        ]
    }

    /// Where strip `k` goes with an output setting: always a later strip.
    fn resolve(&self, k: usize, out: u8, out_track: u8) -> Option<usize> {
        let (role, i) = self.role(k);
        if !role.has_sends() {
            return (role == Role::Return).then(|| self.master());
        }
        match out {
            OUT_NONE => None,
            1..=8 => {
                let b = out as usize - 1;
                Some(if role == Role::Bus && b <= i {
                    self.master()
                } else {
                    self.bus(b)
                })
            }
            OUT_TRACK
                if matches!(role, Role::Input | Role::Pad)
                    && (out_track as usize) < self.tracks =>
            {
                Some(self.track(out_track as usize))
            }
            _ => Some(self.master()),
        }
    }

    /// Which strips are heard (before their own monitoring).
    fn audible(&self, sw: &Switches) -> Bits {
        let n = self.len();
        let mut aud = Bits::default();
        if !sw.solo.any() {
            for k in 0..n {
                aud.set(k, !sw.mute.get(k));
            }
            return aud;
        }
        // Soloed or fed by a soloed strip: outputs always go to a later
        // strip, so one pass forward carries it all the way down.
        let mut up = sw.solo;
        for k in 0..n {
            if up.get(k) {
                if let Some(d) = self.resolve(k, sw.out[k], sw.out_track[k]) {
                    up.set(d, true);
                }
            }
        }
        // Soloed or feeding a soloed strip: one pass backward.
        let mut down = sw.solo;
        for k in (0..n).rev() {
            if let Some(d) = self.resolve(k, sw.out[k], sw.out_track[k]) {
                if down.get(d) {
                    down.set(k, true);
                }
            }
        }
        for k in 0..n {
            let safe = !self.role(k).0.has_sends();
            aud.set(k, !sw.mute.get(k) && (safe || up.get(k) || down.get(k)));
        }
        aud
    }

    fn apply(&self, sw: &mut Switches, k: usize, id: u16, value: f32) {
        let role = self.role(k).0;
        let v = match PARAMS.get(id as usize) {
            Some(p) => p.clamp(value),
            None => return,
        };
        match id {
            ids::MUTE => sw.mute.set(k, v >= 0.5),
            ids::SOLO if role.has_sends() => sw.solo.set(k, v >= 0.5),
            ids::OUTPUT if role.has_sends() => sw.out[k] = v as u8,
            ids::OUTPUT_TRACK if role.has_sends() => sw.out_track[k] = v as u8,
            _ => {}
        }
    }
}

/// A mute, solo or output change somewhere in the mixer this block.
#[derive(Clone, Copy)]
struct Global {
    offset: u32,
    strip: u16,
    seq: u16,
    id: u16,
    value: f32,
}

/// The mixer as of a global change: who is heard, and the changed strip's
/// output.
#[derive(Clone, Copy)]
struct Epoch {
    offset: u32,
    aud: Bits,
    strip: u16,
    out: u8,
    out_track: u8,
}

/// Every strip's compensation, worked out from the chains' latencies.
struct Plan {
    topo: Topo,
    /// Latency at each strip's input.
    arrive: Vec<u32>,
    /// Latency at each strip's output (its input plus its inserts).
    out: Vec<u32>,
    /// Each strip's insert latency, as last worked out.
    lats: Vec<u32>,
    max_comp: usize,
}

impl Plan {
    /// The delay on the path from strip `j` into strip `d`, uncapped.
    fn raw_delay(&self, j: usize, d: usize) -> usize {
        if self.topo.role(j).0 == Role::Input {
            0
        } else {
            self.arrive[d].saturating_sub(self.out[j]) as usize
        }
    }

    fn delay(&self, j: usize, d: usize) -> usize {
        self.raw_delay(j, d).min(self.max_comp)
    }
}

/// A stereo ring buffer for latency compensation, read at any delay up to
/// its size less a chunk.
struct Line {
    buf: [Vec<f32>; 2],
    mask: usize,
    w: usize,
}

impl Line {
    fn new(size: usize) -> Line {
        Line {
            buf: [vec![0.0; size], vec![0.0; size]],
            mask: size.saturating_sub(1),
            w: 0,
        }
    }

    fn write(&mut self, l: &[f32], r: &[f32]) {
        let n = l.len();
        let first = n.min(self.mask + 1 - self.w);
        for (b, x) in self.buf.iter_mut().zip([l, r]) {
            b[self.w..self.w + first].copy_from_slice(&x[..first]);
            b[..n - first].copy_from_slice(&x[first..]);
        }
    }

    /// The `n` frames of side `ch` that start `d` frames before the chunk
    /// just written, as up to two runs.
    #[inline]
    fn runs(&self, ch: usize, d: usize, n: usize) -> (&[f32], &[f32]) {
        let start = self.w.wrapping_sub(d) & self.mask;
        let first = n.min(self.mask + 1 - start);
        let b = &self.buf[ch];
        (&b[start..start + first], &b[..n - first])
    }

    /// Frame `i` of the chunk just written, `d` frames back.
    #[inline]
    fn at(&self, ch: usize, i: usize, d: usize) -> f32 {
        self.buf[ch][(self.w + i).wrapping_sub(d) & self.mask]
    }

    fn advance(&mut self, n: usize) {
        self.w = (self.w + n) & self.mask;
    }

    fn clear(&mut self) {
        for b in &mut self.buf {
            b.fill(0.0);
        }
    }
}

/// A compensation delay that crossfades from its old length to a new one.
#[derive(Clone, Copy)]
struct Tap {
    d: usize,
    from: usize,
    /// Crossfade position, 1 when settled.
    x: f32,
}

impl Tap {
    const ZERO: Tap = Tap {
        d: 0,
        from: 0,
        x: 1.0,
    };

    fn set(&mut self, d: usize, snap: bool) {
        if snap {
            *self = Tap { d, from: d, x: 1.0 };
        } else if d != self.d {
            // A change during a crossfade starts from the newer length.
            *self = Tap {
                d,
                from: self.d,
                x: 0.0,
            };
        }
    }

    fn fading(&self) -> bool {
        self.x < 1.0
    }

    /// The crossfade weight for each frame of a chunk, then moves on.
    fn weights(&mut self, step: f32, out: &mut [f32]) {
        for (i, w) in out.iter_mut().enumerate() {
            *w = (self.x + step * (i + 1) as f32).min(1.0);
        }
        self.x = (self.x + step * out.len() as f32).min(1.0);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Duck {
    Idle,
    /// Fading out for an edit.
    Out,
    /// Silent, waiting for its edits at the next block.
    Wait,
    /// Silent for this many more frames while new latency fills.
    Hold(u32),
    /// Fading back in.
    In,
}

/// One destination of a strip's output, gliding in or out.
#[derive(Clone, Copy)]
struct Route {
    dest: u16,
    g: Smooth,
    tap: Tap,
}

impl Route {
    fn idle(&self) -> bool {
        self.dest == NONE || (self.g.settled() && self.g.value == 0.0)
    }
}

struct Capture {
    src: Option<(u16, CapturePoint)>,
    buf: [Vec<f32>; 2],
}

struct Strip {
    role: Role,
    chain: Chain,
    buf: [Vec<f32>; 2],
    events: Vec<Timed>,
    vol_db: f32,
    pan: f32,
    send_db: [f32; RETURNS],
    send_pre: [bool; RETURNS],
    /// The settings above as gains, worked out when they change (not every
    /// block): volume times balance per side, and each send's level.
    vol_bal: [f32; 2],
    send_g: [f32; RETURNS],
    source: u8,
    monitor: u8,
    arm: bool,
    /// The output as this strip has played it so far (the switches hold
    /// the mixer's view).
    out: u8,
    out_track: u8,
    /// Volume times balance, per side.
    vp: [Smooth; 2],
    /// 1 when the strip is heard, 0 when muted, soloed out or not
    /// monitored.
    aud: Smooth,
    routes: [Route; ROUTES],
    /// Per return: the level taken before and after the fader.
    send: [[Smooth; 2]; RETURNS],
    duck: Duck,
    duck_g: f32,
    /// Before the fader, after mute (pre-fader sends).
    pre_line: Line,
    /// After the fader (outputs and post-fader sends).
    post_line: Line,
    tap_send: Tap,
    /// A track's own input, held back to line up with delayed pad channels.
    in_line: Line,
    tap_in: Tap,
    in_used: bool,
    peak: [f32; 2],
    peak_in: [f32; 2],
    /// Frames in a row the strip's signal after its inserts was silent.
    quiet: usize,
    /// The strip's whole block (inserts, fader, routing) while profiling.
    probe: Probe,
    /// The block in `buf` is known to be all zeros, so it needs no clearing.
    clean: bool,
    /// Something may have written `buf` this block (a producer took it, the
    /// line input, a route or send, the held-back input). Every buffer is
    /// all zeros at the start of a block, so an untouched one needs no
    /// scan.
    touched: bool,
}

impl Strip {
    fn new(role: Role, slots: usize, rate: u32, max_block: usize, line: usize) -> Strip {
        let mut chain = Chain::with_capacity(slots);
        chain.prepare(rate, max_block);
        let sm = |v: f32| {
            let mut s = Smooth::new(v);
            s.set_time(rate, SMOOTH_MS);
            s
        };
        let sends = role.has_sends();
        Strip {
            role,
            chain,
            buf: [vec![0.0; max_block], vec![0.0; max_block]],
            events: Vec::with_capacity(MAX_STRIP_EVENTS),
            vol_db: 0.0,
            pan: 0.0,
            send_db: [SILENT_DB; RETURNS],
            send_pre: [false; RETURNS],
            vol_bal: [1.0; 2],
            send_g: [0.0; RETURNS],
            source: ids::source::STEREO as u8,
            monitor: ids::monitor::AUTO as u8,
            arm: false,
            out: 0,
            out_track: 0,
            vp: [sm(1.0), sm(1.0)],
            aud: sm(1.0),
            routes: [Route {
                dest: NONE,
                g: sm(0.0),
                tap: Tap::ZERO,
            }; ROUTES],
            send: std::array::from_fn(|_| [sm(0.0), sm(0.0)]),
            duck: Duck::Idle,
            duck_g: 1.0,
            pre_line: Line::new(if sends { line } else { 0 }),
            post_line: Line::new(if role == Role::Master { 0 } else { line }),
            tap_send: Tap::ZERO,
            in_line: Line::new(if role == Role::Track { line } else { 0 }),
            tap_in: Tap::ZERO,
            in_used: false,
            peak: [0.0; 2],
            peak_in: [0.0; 2],
            quiet: 0,
            probe: Probe::new(u64::MAX),
            clean: false,
            touched: true,
        }
    }

    /// A strip's own (not global) parameter change.
    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        match id {
            ids::VOLUME | ids::PAN => {
                if id == ids::VOLUME {
                    self.vol_db = v
                } else {
                    self.pan = v
                }
                let g = level(self.vol_db);
                let (bl, br) = balance(self.pan * 0.01);
                self.vol_bal = [g * bl, g * br];
            }
            6..=9 => {
                let r = (id - 6) as usize;
                self.send_db[r] = v;
                self.send_g[r] = level(v);
            }
            10..=13 => self.send_pre[(id - 10) as usize] = v >= 0.5,
            ids::MONITOR => self.monitor = v as u8,
            ids::ARM => self.arm = v >= 0.5,
            _ => {}
        }
    }

    /// Points every smoother at where the settings say, given whether the
    /// mixer hears this strip, and picks up a new destination.
    fn targets(&mut self, plan: &Plan, k: usize, heard: bool) {
        let monitored = self.role != Role::Input
            || self.monitor == ids::monitor::ON as u8
            || (self.monitor == ids::monitor::AUTO as u8 && self.arm);
        self.aud.target = if heard && monitored { 1.0 } else { 0.0 };
        self.vp[0].target = self.vol_bal[0];
        self.vp[1].target = self.vol_bal[1];
        if self.role == Role::Master {
            return;
        }
        let want = plan
            .topo
            .resolve(k, self.out, self.out_track)
            .map_or(NONE, |d| d as u16);
        let mut have = false;
        for r in &mut self.routes {
            let on = want != NONE && r.dest == want;
            r.g.target = if on { 1.0 } else { 0.0 };
            have |= on;
        }
        if !have && want != NONE {
            let pick = self.routes.iter().position(Route::idle).unwrap_or_else(|| {
                // Three changes inside one glide: take over the quietest.
                let mut m = 0;
                for (i, r) in self.routes.iter().enumerate() {
                    if r.g.value < self.routes[m].g.value {
                        m = i;
                    }
                }
                m
            });
            let r = &mut self.routes[pick];
            r.dest = want;
            r.g.snap(0.0);
            r.g.target = 1.0;
            r.tap.set(plan.delay(k, want as usize), true);
        }
        if self.role.has_sends() {
            for r in 0..RETURNS {
                let g = self.send_g[r];
                let pre = self.send_pre[r];
                self.send[r][0].target = if pre { g } else { 0.0 };
                self.send[r][1].target = if pre { 0.0 } else { g };
            }
        }
    }

    fn snap(&mut self) {
        let snap = |s: &mut Smooth| s.snap(s.target);
        self.vp.iter_mut().for_each(snap);
        snap(&mut self.aud);
        self.send.iter_mut().flatten().for_each(snap);
        for r in &mut self.routes {
            snap(&mut r.g);
            if r.g.value == 0.0 {
                r.dest = NONE;
            }
        }
    }

    fn settled(&self) -> bool {
        self.vp.iter().all(Smooth::settled)
            && self.aud.settled()
            && self.send.iter().flatten().all(Smooth::settled)
            && self.routes.iter().all(|r| r.g.settled() && !r.tap.fading())
            && !self.tap_send.fading()
            && !self.tap_in.fading()
    }

    fn silent(&self) -> bool {
        matches!(self.duck, Duck::Wait | Duck::Hold(_))
    }
}

/// Steps an edit's duck by one frame and returns its gain.
#[inline]
fn duck_next(duck: &mut Duck, g: &mut f32, step: f32) -> f32 {
    match *duck {
        Duck::Idle | Duck::Wait => {}
        Duck::Out => {
            *g -= step;
            if *g <= 0.0 {
                *g = 0.0;
                *duck = Duck::Wait;
            }
        }
        Duck::Hold(0) => *duck = Duck::In,
        Duck::Hold(n) => *duck = Duck::Hold(n - 1),
        Duck::In => {
            *g += step;
            if *g >= 1.0 {
                *g = 1.0;
                *duck = Duck::Idle;
            }
        }
    }
    *g
}

/// Gain for a level in dB, with the bottom of the range silent.
fn level(db: f32) -> f32 {
    if db <= SILENT_DB {
        0.0
    } else {
        db_to_gain(db)
    }
}

/// Balance gains for a pan of -1 to 1: the far side follows a quarter
/// cosine down to nothing, the near side stays at unity.
fn balance(p: f32) -> (f32, f32) {
    let fade = |x: f32| {
        if x >= 1.0 {
            0.0
        } else {
            (FRAC_PI_2 * x).cos().max(0.0)
        }
    };
    if p >= 0.0 {
        (fade(p), 1.0)
    } else {
        (1.0, fade(-p))
    }
}

#[inline]
fn peak(p: &mut [f32; 2], l: &[f32], r: &[f32]) {
    p[0] = l.iter().fold(p[0], |m, v| m.max(v.abs()));
    p[1] = r.iter().fold(p[1], |m, v| m.max(v.abs()));
}

/// Per-chunk working buffers.
struct Scratch {
    post: [Vec<f32>; 2],
    pre: [Vec<f32>; 2],
    w_route: [Vec<f32>; ROUTES],
    w_send: Vec<f32>,
}

/// The mixer. See the module docs.
pub struct Mixer {
    rate: u32,
    max_block: usize,
    plan: Plan,
    strips: Vec<Strip>,
    /// Strips in processing order.
    order: Vec<u16>,
    sw: Switches,
    aud: Bits,
    globals: Vec<Global>,
    epochs: Vec<Epoch>,
    edits: Vec<Edit>,
    retired: Vec<Insert>,
    safety: MasterSafety,
    scratch: Scratch,
    hw: [Vec<f32>; 2],
    caps: Vec<Capture>,
    line_len: usize,
    tap_step: f32,
    duck_step: f32,
    latency: u32,
    started: bool,
    last_len: usize,
    stats: MixerStats,
    profiling: bool,
    probe: Probe,
}

impl Mixer {
    /// Builds a mixer for `rate` with blocks of up to `max_block` frames
    /// (control thread; allocates everything the mixer will ever use).
    pub fn new(cfg: MixerConfig, rate: u32, max_block: usize) -> Mixer {
        let rate = rate.max(1);
        let max_block = max_block.clamp(1, mpfree_node::MAX_BLOCK);
        let topo = Topo {
            inputs: cfg.inputs.min(MAX_INPUTS),
            pads: cfg.pads.min(MAX_PADS),
            tracks: cfg.tracks.clamp(1, MAX_TRACKS),
        };
        let slots = cfg.slots.min(MAX_SLOTS);
        let max_comp = (cfg.max_latency_ms.max(0.0) * 0.001 * rate as f32).round() as usize;
        let line_len = (max_comp + CHUNK + 1).next_power_of_two();
        let strips = (0..topo.len())
            .map(|k| Strip::new(topo.role(k).0, slots, rate, max_block, line_len))
            .collect();
        let mut safety = MasterSafety::new();
        mpfree_node::Node::prepare(&mut safety, rate, max_block);
        let block = || [vec![0.0; max_block], vec![0.0; max_block]];
        let mut m = Mixer {
            rate,
            max_block,
            plan: Plan {
                topo,
                arrive: vec![0; topo.len()],
                out: vec![0; topo.len()],
                lats: vec![0; topo.len()],
                max_comp,
            },
            strips,
            order: Vec::with_capacity(topo.len()),
            sw: Switches::new(),
            aud: Bits::default(),
            globals: Vec::with_capacity(MAX_GLOBALS),
            epochs: Vec::with_capacity(MAX_GLOBALS),
            edits: Vec::with_capacity(MAX_EDITS),
            retired: Vec::with_capacity(MAX_EDITS),
            safety,
            scratch: Scratch {
                post: [vec![0.0; CHUNK], vec![0.0; CHUNK]],
                pre: [vec![0.0; CHUNK], vec![0.0; CHUNK]],
                w_route: std::array::from_fn(|_| vec![0.0; CHUNK]),
                w_send: vec![0.0; CHUNK],
            },
            hw: block(),
            caps: (0..CAPTURES)
                .map(|_| Capture {
                    src: None,
                    buf: block(),
                })
                .collect(),
            line_len,
            tap_step: 1.0 / (TAP_FADE_MS * 0.001 * rate as f32),
            duck_step: 1.0 / (DUCK_MS * 0.001 * rate as f32),
            latency: 0,
            started: false,
            last_len: 0,
            stats: MixerStats::default(),
            profiling: false,
            probe: Probe::new(u64::MAX),
        };
        m.reorder();
        m.aud = topo.audible(&m.sw);
        m.align(true);
        m.retarget(true);
        m
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }
    pub fn max_block(&self) -> usize {
        self.max_block
    }
    pub fn inputs(&self) -> usize {
        self.plan.topo.inputs
    }
    pub fn pads(&self) -> usize {
        self.plan.topo.pads
    }
    pub fn tracks(&self) -> usize {
        self.plan.topo.tracks
    }

    /// Every strip, by stage: inputs, pads, tracks, submixes, returns,
    /// master.
    pub fn strips(&self) -> impl Iterator<Item = StripId> + '_ {
        (0..self.plan.topo.len()).map(|k| self.plan.topo.id(k))
    }

    /// A strip's insert chain.
    pub fn chain(&self, s: StripId) -> Option<&Chain> {
        self.plan.topo.order(s).map(|k| &self.strips[k].chain)
    }

    /// A strip's insert chain, to set up before playing (control thread:
    /// [`Chain::push`] allocates). While playing, use [`Mixer::edit`].
    pub fn chain_mut(&mut self, s: StripId) -> Option<&mut Chain> {
        self.plan.topo.order(s).map(|k| &mut self.strips[k].chain)
    }

    /// A track's or pad channel's input for the next block, both sides
    /// `max_block` long. Add the block's frames into the start; the mixer
    /// clears them after each block.
    pub fn strip_input(&mut self, s: StripId) -> Option<(&mut [f32], &mut [f32])> {
        if !matches!(s, StripId::Track(_) | StripId::Pad(_)) {
            return None;
        }
        let k = self.plan.topo.order(s)?;
        let st = &mut self.strips[k];
        st.touched = true;
        let [l, r] = &mut st.buf;
        Some((l.as_mut_slice(), r.as_mut_slice()))
    }

    /// A track's input for the next block (see [`Mixer::strip_input`]).
    pub fn track_input(&mut self, t: usize) -> Option<(&mut [f32], &mut [f32])> {
        self.strip_input(StripId::Track(t))
    }

    /// A pad channel's input for the next block (see [`Mixer::strip_input`]).
    pub fn pad_input(&mut self, p: usize) -> Option<(&mut [f32], &mut [f32])> {
        self.strip_input(StripId::Pad(p))
    }

    /// The line input (inputs 1 and 2) for the next block, both sides
    /// `max_block` long. Write the block's frames into the start; every
    /// input strip reads it through its [`ids::SOURCE`].
    pub fn hw_input(&mut self) -> (&mut [f32], &mut [f32]) {
        let [l, r] = &mut self.hw;
        (l.as_mut_slice(), r.as_mut_slice())
    }

    /// Sets a strip parameter at once, with no glide: for setting the
    /// mixer up before it plays.
    pub fn set_now(&mut self, s: StripId, id: u16, value: f32) {
        let Some(k) = self.plan.topo.order(s) else {
            return;
        };
        let topo = self.plan.topo;
        if is_global(id) {
            topo.apply(&mut self.sw, k, id, value);
            self.aud = topo.audible(&self.sw);
            let st = &mut self.strips[k];
            st.out = self.sw.out[k];
            st.out_track = self.sw.out_track[k];
        } else if id == ids::SOURCE {
            self.strips[k].source = PARAMS[ids::SOURCE as usize].clamp(value) as u8;
        } else {
            self.strips[k].apply(id, value);
        }
        self.retarget(true);
    }

    /// Queues a sample-timed strip parameter event for the next block
    /// (offsets at or past its end apply right after it). False, and
    /// counted, when the strip's queue is full or the strip is not there.
    pub fn queue(&mut self, s: StripId, t: Timed) -> bool {
        let Some(k) = self.plan.topo.order(s) else {
            return false;
        };
        let q = &mut self.strips[k].events;
        if q.len() == q.capacity() {
            self.stats.dropped_events += 1;
            return false;
        }
        let at = q.partition_point(|e| e.offset <= t.offset);
        q.insert(at, t);
        true
    }

    /// Sets a strip parameter from the start of the next block (with a glide).
    pub fn set(&mut self, s: StripId, id: u16, value: f32) -> bool {
        self.queue(s, Timed::new(0, NodeEvent::Param { id, value }))
    }

    /// A strip parameter as it stands after the last block.
    pub fn param(&self, s: StripId, id: u16) -> f32 {
        let Some(k) = self.plan.topo.order(s) else {
            return f32::NAN;
        };
        let st = &self.strips[k];
        match id {
            ids::VOLUME => st.vol_db,
            ids::PAN => st.pan,
            ids::MUTE => self.sw.mute.get(k) as u8 as f32,
            ids::SOLO => self.sw.solo.get(k) as u8 as f32,
            ids::OUTPUT => self.sw.out[k] as f32,
            ids::OUTPUT_TRACK => self.sw.out_track[k] as f32,
            6..=9 => st.send_db[(id - 6) as usize],
            10..=13 => st.send_pre[(id - 10) as usize] as u8 as f32,
            ids::SOURCE => st.source as f32,
            ids::MONITOR => st.monitor as f32,
            ids::ARM => st.arm as u8 as f32,
            _ => f32::NAN,
        }
    }

    /// Where a strip's output goes now, if anywhere.
    pub fn destination(&self, s: StripId) -> Option<StripId> {
        let k = self.plan.topo.order(s)?;
        self.plan
            .topo
            .resolve(k, self.sw.out[k], self.sw.out_track[k])
            .map(|d| self.plan.topo.id(d))
    }

    /// Whether a strip is heard now: not muted, not left out by a solo,
    /// and (an input) monitored.
    pub fn is_audible(&self, s: StripId) -> bool {
        self.plan
            .topo
            .order(s)
            .is_some_and(|k| self.aud.get(k) && self.strips[k].aud.target > 0.0)
    }

    /// Queues a sample-timed event for an insert (see [`Chain::queue`]).
    pub fn queue_insert(&mut self, s: StripId, slot: usize, t: Timed) -> bool {
        let Some(k) = self.plan.topo.order(s) else {
            return false;
        };
        let ok = self.strips[k].chain.queue(slot, t);
        if !ok {
            self.stats.dropped_events += 1;
        }
        ok
    }

    /// Bypasses an insert or switches it back on (see [`Chain::set_on`]).
    pub fn set_on(&mut self, s: StripId, slot: usize, on: bool) {
        if let Some(k) = self.plan.topo.order(s) {
            self.strips[k].chain.set_on(slot, on);
        }
    }

    /// Keys the insert in `slot` of strip `s` (a compressor's sidechain)
    /// from strip `key`'s signal after its inserts, before its fader and
    /// mute, from the next block; the key moves with the insert. False,
    /// and nothing changes, if the slot is not there or the key is not
    /// allowed (see the module docs: a later stage, a higher submix, the
    /// strip itself, or a loop).
    pub fn set_key(&mut self, s: StripId, slot: usize, key: Option<StripId>) -> bool {
        let topo = self.plan.topo;
        let Some(k) = topo.order(s) else { return false };
        if slot >= self.strips[k].chain.len() {
            return false;
        }
        let tag = match key {
            None => None,
            Some(ks) => match topo.order(ks) {
                Some(j) if may_key(&topo, j, k) => Some(j as u16),
                _ => return false,
            },
        };
        let old = self.strips[k].chain.key(slot);
        self.strips[k].chain.set_key(slot, tag);
        if !self.reorder() {
            self.strips[k].chain.set_key(slot, old);
            self.reorder();
            return false;
        }
        true
    }

    /// The strip an insert is keyed from.
    pub fn key(&self, s: StripId, slot: usize) -> Option<StripId> {
        let k = self.plan.topo.order(s)?;
        self.strips[k]
            .chain
            .key(slot)
            .map(|j| self.plan.topo.id(j as usize))
    }

    /// Queues an edit to a strip's inserts. It is applied at the next
    /// block where the strip has ducked out (about 10 ms). Hands the edit
    /// back if the strip is not there or too many edits are waiting.
    // Handing the value back, to be dropped off the audio thread, is the point.
    #[allow(clippy::result_large_err)]
    pub fn edit(&mut self, e: Edit) -> Result<(), Edit> {
        if self.plan.topo.order(e.strip()).is_none() || self.edits.len() == self.edits.capacity() {
            return Err(e);
        }
        self.edits.push(e);
        Ok(())
    }

    /// Edits not applied yet.
    pub fn edits_pending(&self) -> usize {
        self.edits.len()
    }

    /// An insert an edit took out, to drop or reuse off the audio thread.
    pub fn pop_retired(&mut self) -> Option<Insert> {
        self.retired.pop()
    }

    /// Frames from a track's input to the output: the slowest compensated
    /// path plus the master's inserts and Master safety.
    pub fn latency(&self) -> u32 {
        self.latency
    }

    /// The loudest sample on each side of a strip's output since the last
    /// call (the master's is after Master safety), and starts again.
    pub fn take_peaks(&mut self, s: StripId) -> [f32; 2] {
        self.plan
            .topo
            .order(s)
            .map_or([0.0; 2], |k| std::mem::take(&mut self.strips[k].peak))
    }

    /// The loudest sample on each side of a strip after its inserts and
    /// before its fader (an input's level whether or not it is monitored)
    /// since the last call, and starts again.
    pub fn take_input_peaks(&mut self, s: StripId) -> [f32; 2] {
        self.plan
            .topo
            .order(s)
            .map_or([0.0; 2], |k| std::mem::take(&mut self.strips[k].peak_in))
    }

    /// Points capture `slot` (0 to [`CAPTURES`] - 1) at a strip, or frees
    /// it. Takes effect from the next block. False if the slot or strip is
    /// not there, or [`CapturePoint::AfterSafety`] is asked of a strip
    /// other than the master.
    pub fn set_capture(&mut self, slot: usize, src: Option<(StripId, CapturePoint)>) -> bool {
        let src = match src {
            None => None,
            Some((s, p)) => match self.plan.topo.order(s) {
                Some(k) if p != CapturePoint::AfterSafety || s == StripId::Master => {
                    Some((k as u16, p))
                }
                _ => return false,
            },
        };
        let Some(c) = self.caps.get_mut(slot) else {
            return false;
        };
        c.src = src;
        true
    }

    /// The last block a capture point took, or `None` when it is free.
    pub fn capture(&self, slot: usize) -> Option<(&[f32], &[f32])> {
        let c = self.caps.get(slot)?;
        c.src?;
        Some((&c.buf[0][..self.last_len], &c.buf[1][..self.last_len]))
    }

    /// Frames between a block's track inputs and the same moment at a
    /// capture point (for an input strip, between the line input and the
    /// point), so a recorder can line a take up with the transport.
    pub fn capture_latency(&self, slot: usize) -> Option<u32> {
        let (k, p) = self.caps.get(slot)?.src?;
        let k = k as usize;
        Some(match p {
            CapturePoint::BeforeInserts => self.plan.arrive[k],
            CapturePoint::BeforeFader | CapturePoint::AfterFader => self.plan.out[k],
            CapturePoint::AfterSafety => self.latency,
        })
    }

    /// Times the whole mixer, every strip and every insert per block (see
    /// [`Mixer::probe`], [`Mixer::strip_probe`] and [`Chain::probe`]), at
    /// two clock reads each. Off by default; the Dev pages switch it on.
    pub fn set_profiling(&mut self, on: bool) {
        self.profiling = on;
        for st in &mut self.strips {
            st.chain.set_profiling(on);
        }
    }

    pub fn profiling(&self) -> bool {
        self.profiling
    }

    /// The whole mixer's time per block, while profiling.
    pub fn probe(&self) -> &Probe {
        &self.probe
    }
    pub fn probe_mut(&mut self) -> &mut Probe {
        &mut self.probe
    }

    /// A strip's time per block (its inserts, fader and routing; a strip
    /// that skipped its work while quiet still records its scan), while
    /// profiling.
    pub fn strip_probe(&self, s: StripId) -> Option<&Probe> {
        self.plan.topo.order(s).map(|k| &self.strips[k].probe)
    }
    pub fn strip_probe_mut(&mut self, s: StripId) -> Option<&mut Probe> {
        self.plan.topo.order(s).map(|k| &mut self.strips[k].probe)
    }

    /// Master safety, for its meters and [`MasterSafety::stats`].
    pub fn safety(&self) -> &MasterSafety {
        &self.safety
    }

    /// Master safety, to set its mode, trim and ceiling.
    pub fn safety_mut(&mut self) -> &mut MasterSafety {
        &mut self.safety
    }

    pub fn stats(&self) -> MixerStats {
        let mut s = self.stats;
        s.nan_resets = self
            .strips
            .iter()
            .map(|st| st.chain.stats().nan_resets)
            .sum();
        s.dropped_events += self
            .strips
            .iter()
            .map(|st| st.chain.stats().dropped_events)
            .sum::<u64>();
        s
    }

    /// Clears every strip's delay lines, smoothers and inserts (a stop or
    /// a project load). Settings, switches, keys and pending edits stay.
    pub fn reset(&mut self) {
        for st in &mut self.strips {
            for n in 0..st.chain.len() {
                if let Some(node) = st.chain.node_mut(n) {
                    node.reset();
                }
            }
            st.pre_line.clear();
            st.post_line.clear();
            st.in_line.clear();
            st.peak = [0.0; 2];
            st.peak_in = [0.0; 2];
            st.quiet = 0;
        }
        mpfree_node::Node::reset(&mut self.safety);
        self.started = false;
        self.align(true);
        self.retarget(true);
    }

    fn retarget(&mut self, snap: bool) {
        let Mixer {
            plan, strips, aud, ..
        } = self;
        for (k, st) in strips.iter_mut().enumerate() {
            st.targets(plan, k, aud.get(k));
            if snap {
                st.snap();
            }
        }
    }

    /// Puts the strips in processing order: stage by stage, each stage
    /// ordered so every key runs before the strips it keys. False on a loop
    /// (the order is then incomplete).
    fn reorder(&mut self) -> bool {
        let Mixer {
            plan,
            strips,
            order,
            ..
        } = self;
        order.clear();
        let mut state = [0u8; MAX_STRIPS];
        for (lo, hi, free) in plan.topo.stages() {
            for k in lo..hi {
                if !free {
                    order.push(k as u16);
                } else if !visit(strips, k, lo, hi, &mut state, order) {
                    return false;
                }
            }
        }
        true
    }

    /// Works out every compensation delay from the chains' latencies (only
    /// when one changed, unless `force`).
    fn align(&mut self, force: bool) {
        let Mixer {
            plan,
            strips,
            safety,
            started,
            ..
        } = self;
        let mut changed = force;
        for (st, l) in strips.iter().zip(plan.lats.iter_mut()) {
            let now = st.chain.latency();
            changed |= now != *l;
            *l = now;
        }
        if !changed {
            return;
        }
        let topo = plan.topo;
        let lats = &plan.lats;
        let lat = |k: usize| lats[k];
        let n = topo.len();
        let span =
            |lo: usize, hi: usize, out: &[u32]| out[lo..hi].iter().copied().max().unwrap_or(0);
        for k in 0..topo.track(0) {
            plan.arrive[k] = 0;
            plan.out[k] = lat(k);
        }
        // Inputs are live and never counted; pads can feed a track.
        let at_track = span(topo.pad(0), topo.track(0), &plan.out);
        for k in topo.track(0)..topo.bus(0) {
            plan.arrive[k] = at_track;
            plan.out[k] = at_track + lat(k);
        }
        let mut at_bus = span(topo.pad(0), topo.bus(0), &plan.out);
        for k in topo.bus(0)..topo.ret(0) {
            plan.arrive[k] = at_bus;
            plan.out[k] = at_bus + lat(k);
            at_bus = at_bus.max(plan.out[k]);
        }
        let at_ret = span(topo.pad(0), topo.ret(0), &plan.out);
        for k in topo.ret(0)..topo.master() {
            plan.arrive[k] = at_ret;
            plan.out[k] = at_ret + lat(k);
        }
        let m = topo.master();
        plan.arrive[m] = span(topo.pad(0), m, &plan.out);
        plan.out[m] = plan.arrive[m] + lat(m);
        let mut misaligned = false;
        let ret0 = topo.ret(0);
        for (j, st) in strips.iter_mut().enumerate().take(n - 1) {
            let snap = !*started || st.silent();
            let mut fit = |raw: usize| {
                misaligned |= raw > plan.max_comp;
                raw.min(plan.max_comp)
            };
            for r in &mut st.routes {
                if r.dest != NONE {
                    r.tap.set(fit(plan.raw_delay(j, r.dest as usize)), snap);
                }
            }
            if st.role.has_sends() {
                st.tap_send.set(fit(plan.raw_delay(j, ret0)), snap);
            }
            if st.role == Role::Track {
                st.tap_in.set(fit(plan.arrive[j] as usize), snap);
            }
        }
        self.stats.misaligned = misaligned;
        self.latency = plan.out[m] + mpfree_node::Node::latency(safety);
    }

    /// Applies the edits of strips that have ducked out.
    fn apply_edits(&mut self) {
        let mut i = 0;
        while i < self.edits.len() {
            let Some(k) = self.plan.topo.order(self.edits[i].strip()) else {
                i += 1;
                continue;
            };
            if self.strips[k].duck != Duck::Wait {
                i += 1;
                continue;
            }
            if self.retired.len() == self.retired.capacity() {
                break;
            }
            let chain = &mut self.strips[k].chain;
            let (out, ok) = match self.edits.remove(i) {
                Edit::Insert { at, insert, .. } => match chain.insert(at, insert) {
                    Ok(()) => {
                        chain.set_key(at, None);
                        (None, true)
                    }
                    Err(ins) => (Some(ins), false),
                },
                Edit::Replace { at, insert, .. } => match chain.replace(at, insert) {
                    Ok(old) => {
                        chain.set_key(at, None);
                        (Some(old), true)
                    }
                    Err(ins) => (Some(ins), false),
                },
                Edit::Remove { at, .. } => {
                    let r = chain.remove(at);
                    let ok = r.is_some();
                    (r, ok)
                }
                Edit::Move { from, to, .. } => (None, chain.move_slot(from, to)),
            };
            if !ok {
                self.stats.bad_edits += 1;
            }
            if let Some(ins) = out {
                self.retired.push(ins);
            }
        }
        for k in 0..self.strips.len() {
            if self.strips[k].duck == Duck::Wait
                && !self
                    .edits
                    .iter()
                    .any(|e| self.plan.topo.order(e.strip()) == Some(k))
            {
                let st = &mut self.strips[k];
                st.duck = Duck::Hold(st.chain.latency());
            }
        }
    }

    /// Starts ducking the strips that have edits waiting.
    fn start_ducks(&mut self) {
        for e in &self.edits {
            let Some(k) = self.plan.topo.order(e.strip()) else {
                continue;
            };
            let st = &mut self.strips[k];
            st.duck = match st.duck {
                Duck::Idle | Duck::In => Duck::Out,
                Duck::Hold(_) => Duck::Wait,
                d => d,
            };
        }
    }

    /// Collects this block's mute, solo and output changes in order, and
    /// what the mixer hears after each.
    fn gather_globals(&mut self) -> Switches {
        self.globals.clear();
        self.epochs.clear();
        for (k, st) in self.strips.iter().enumerate() {
            for (q, e) in st.events.iter().enumerate() {
                if let NodeEvent::Param { id, value } = e.ev {
                    if !is_global(id) {
                        continue;
                    }
                    if self.globals.len() == self.globals.capacity() {
                        self.stats.dropped_events += 1;
                        continue;
                    }
                    self.globals.push(Global {
                        offset: e.offset,
                        strip: k as u16,
                        seq: q as u16,
                        id,
                        value,
                    });
                }
            }
        }
        self.globals
            .sort_unstable_by_key(|g| (g.offset, g.strip, g.seq));
        let topo = self.plan.topo;
        let mut sw = self.sw;
        for g in &self.globals {
            let k = g.strip as usize;
            topo.apply(&mut sw, k, g.id, g.value);
            self.epochs.push(Epoch {
                offset: g.offset,
                aud: topo.audible(&sw),
                strip: g.strip,
                out: sw.out[k],
                out_track: sw.out_track[k],
            });
        }
        sw
    }

    /// Mixes one block into `left` and `right` (overwriting them). The
    /// inputs given through [`Mixer::strip_input`] and
    /// [`Mixer::hw_input`] are read and then cleared.
    pub fn process(&mut self, ctx: &Ctx, left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len()).min(self.max_block);
        if len == 0 {
            return;
        }
        let whole = self.profiling.then(Span::start);
        self.apply_edits();
        self.align(false);
        self.start_ducks();
        let sw_end = self.gather_globals();
        let Mixer {
            plan,
            strips,
            order,
            aud,
            epochs,
            scratch,
            stats,
            caps,
            hw,
            safety,
            tap_step,
            duck_step,
            line_len,
            profiling,
            ..
        } = self;
        let profiling = *profiling;
        let (topo, tap_step, duck_step, line_len) = (plan.topo, *tap_step, *duck_step, *line_len);
        // With no capture point in use, nothing needs to look for one.
        let caps: &mut [Capture] = if caps.iter().any(|c| c.src.is_some()) {
            caps
        } else {
            &mut []
        };
        let base = Ctx {
            sidechain: None,
            ..*ctx
        };
        let (left, right) = (&mut left[..len], &mut right[..len]);
        for st in strips.iter_mut().take(topo.inputs) {
            fill_input(st, hw, len);
        }
        // A track's own input is held back before anything is routed in.
        for st in &mut strips[topo.track(0)..topo.bus(0)] {
            guard(st, len, stats);
            hold_input(st, len, tap_step, scratch);
        }
        for &k in order.iter() {
            let k = k as usize;
            let (lo, rest) = strips.split_at_mut(k);
            let Some((st, hi)) = rest.split_first_mut() else {
                break;
            };
            let span = profiling.then(Span::start);
            let fed = guard(st, len, stats);
            take(caps, k, CapturePoint::BeforeInserts, &st.buf, 0, len);
            {
                let key = |j: u16| {
                    let j = j as usize;
                    let s = if j < k {
                        lo.get(j)
                    } else {
                        hi.get(j.checked_sub(k + 1)?)
                    }?;
                    Some((&s.buf[0][..len], &s.buf[1][..len]))
                };
                let [bl, br] = &mut st.buf;
                st.chain
                    .process_keyed(&base, key, &mut bl[..len], &mut br[..len]);
            }
            take(caps, k, CapturePoint::BeforeFader, &st.buf, 0, len);
            if st.role == Role::Master {
                run_strip(
                    st,
                    k,
                    hi,
                    len,
                    epochs,
                    aud.get(k),
                    plan,
                    scratch,
                    caps,
                    tap_step,
                    duck_step,
                    Some((&mut *left, &mut *right)),
                );
                if let Some(span) = span {
                    st.probe.stop(span, len as u32);
                }
                continue;
            }
            // An empty chain leaves the block as it came in.
            let silent = if st.chain.is_empty() {
                !fed
            } else {
                !sounding(&st.buf[0][..len], &st.buf[1][..len])
            };
            st.clean = silent;
            if silent
                && st.quiet >= line_len
                && epochs.is_empty()
                && st.events.is_empty()
                && st.duck == Duck::Idle
                && st.settled()
            {
                // Nothing in, nothing in its delay lines, nothing moving:
                // the strip would only add zeros.
                take_zeros(caps, k, CapturePoint::AfterFader, len);
            } else {
                run_strip(
                    st,
                    k,
                    hi,
                    len,
                    epochs,
                    aud.get(k),
                    plan,
                    scratch,
                    caps,
                    tap_step,
                    duck_step,
                    None,
                );
            }
            st.quiet = if silent {
                st.quiet.saturating_add(len)
            } else {
                0
            };
            if let Some(span) = span {
                st.probe.stop(span, len as u32);
            }
        }
        mpfree_node::Node::process(safety, &base, &[], left, right);
        let m = topo.master();
        for c in caps.iter_mut() {
            if c.src == Some((m as u16, CapturePoint::AfterSafety)) {
                c.buf[0][..len].copy_from_slice(left);
                c.buf[1][..len].copy_from_slice(right);
            }
        }
        let master = &mut strips[m];
        peak(&mut master.peak, left, right);
        for st in strips.iter_mut() {
            st.touched = false;
            if !std::mem::take(&mut st.clean) {
                for b in &mut st.buf {
                    b[..len].fill(0.0);
                }
            }
        }
        for b in hw.iter_mut() {
            b[..len].fill(0.0);
        }
        self.sw = sw_end;
        self.aud = topo.audible(&sw_end);
        self.started = true;
        self.last_len = len;
        if let Some(span) = whole {
            self.probe.stop(span, len as u32);
        }
    }
}

/// Visits strip `k` and, first, the keys inside its stage `lo..hi`.
fn visit(
    strips: &[Strip],
    k: usize,
    lo: usize,
    hi: usize,
    state: &mut [u8; MAX_STRIPS],
    order: &mut Vec<u16>,
) -> bool {
    match state[k] {
        2 => return true,
        1 => return false,
        _ => {}
    }
    state[k] = 1;
    let chain = &strips[k].chain;
    for slot in 0..chain.len() {
        if let Some(j) = chain.key(slot) {
            let j = j as usize;
            if (lo..hi).contains(&j) && !visit(strips, j, lo, hi, state, order) {
                return false;
            }
        }
    }
    state[k] = 2;
    order.push(k as u16);
    true
}

/// Whether strip `j` may key strip `k` (before looking for loops).
fn may_key(topo: &Topo, j: usize, k: usize) -> bool {
    let (rj, rk) = (topo.role(j).0, topo.role(k).0);
    j != k && (rj < rk || (rj == rk && (rj != Role::Bus || j < k)))
}

/// Silences a strip's input for the block if it holds a non-finite sample.
/// Returns whether anything is left (not all zeros).
fn guard(st: &mut Strip, len: usize, stats: &mut MixerStats) -> bool {
    if !st.touched {
        return false;
    }
    let [bl, br] = &mut st.buf;
    let (bl, br) = (&mut bl[..len], &mut br[..len]);
    let (bad, sound) = scan(bl, br);
    if bad {
        bl.fill(0.0);
        br.fill(0.0);
        stats.bad_input += 1;
        return false;
    }
    sound
}

/// Whether a block holds a non-finite sample, and whether it holds
/// anything but zeros, in one pass the compiler can vectorise (no early
/// exit, integer lanes).
#[inline]
fn scan(l: &[f32], r: &[f32]) -> (bool, bool) {
    // The largest magnitude's bits: infinities and NaNs sort above every
    // finite value, zeros below everything. With the sign bit cleared the
    // bits fit an i32, whose max every SIMD set has.
    let mut m = 0i32;
    for x in [l, r] {
        for &v in x {
            m = m.max((v.to_bits() & 0x7fff_ffff) as i32);
        }
    }
    (m >= 0x7f80_0000, m != 0)
}

/// Whether a block holds anything but zeros (or minus zeros).
#[inline]
fn sounding(l: &[f32], r: &[f32]) -> bool {
    let mut any = 0u32;
    for (&a, &b) in l.iter().zip(r) {
        any |= (a.to_bits() | b.to_bits()) & 0x7fff_ffff;
    }
    any != 0
}

/// Copies the line input into an input strip, following its source.
fn fill_input(st: &mut Strip, hw: &[Vec<f32>; 2], len: usize) {
    if st.source == ids::source::OFF as u8 && st.events.is_empty() {
        // Already silent: buffers start every block at zero.
        return;
    }
    st.touched = true;
    let mut at = 0;
    let mut ei = 0;
    while at < len {
        while let Some(e) = st.events.get(ei).filter(|e| e.offset as usize <= at) {
            if let NodeEvent::Param {
                id: ids::SOURCE,
                value,
            } = e.ev
            {
                st.source = PARAMS[ids::SOURCE as usize].clamp(value) as u8;
            }
            ei += 1;
        }
        let next = st.events[ei..]
            .iter()
            .find(|e| {
                matches!(
                    e.ev,
                    NodeEvent::Param {
                        id: ids::SOURCE,
                        ..
                    }
                )
            })
            .map_or(len, |e| (e.offset as usize).min(len));
        let (l, r) = (&hw[0][at..next], &hw[1][at..next]);
        let [bl, br] = &mut st.buf;
        let (bl, br) = (&mut bl[at..next], &mut br[at..next]);
        match st.source {
            1 => {
                bl.copy_from_slice(l);
                br.copy_from_slice(r);
            }
            2 => {
                bl.copy_from_slice(l);
                br.copy_from_slice(l);
            }
            3 => {
                bl.copy_from_slice(r);
                br.copy_from_slice(r);
            }
            _ => {
                bl.fill(0.0);
                br.fill(0.0);
            }
        }
        at = next;
    }
    for e in &st.events[ei..] {
        if let NodeEvent::Param {
            id: ids::SOURCE,
            value,
        } = e.ev
        {
            st.source = PARAMS[ids::SOURCE as usize].clamp(value) as u8;
        }
    }
}

/// Holds a track's own input back by its compensation delay, so it lines
/// up with pad channels that come through inserts with latency.
fn hold_input(st: &mut Strip, len: usize, tap_step: f32, sc: &mut Scratch) {
    if st.tap_in.d == 0 && !st.tap_in.fading() {
        st.in_used = false;
        return;
    }
    st.touched = true;
    if !st.in_used {
        // The line has not been kept up: start it from silence.
        st.in_line.clear();
        st.in_used = true;
    }
    let mut a = 0;
    while a < len {
        let b = (a + CHUNK).min(len);
        let n = b - a;
        let w = &mut sc.w_send[..n];
        let tap = &mut st.tap_in;
        let fading = tap.fading();
        if fading {
            tap.weights(tap_step, w);
        }
        let [bl, br] = &mut st.buf;
        st.in_line.write(&bl[a..b], &br[a..b]);
        let line = &st.in_line;
        for i in 0..n {
            let (l, r) = if fading {
                let x = w[i];
                (
                    line.at(0, i, tap.from) + (line.at(0, i, tap.d) - line.at(0, i, tap.from)) * x,
                    line.at(1, i, tap.from) + (line.at(1, i, tap.d) - line.at(1, i, tap.from)) * x,
                )
            } else {
                (line.at(0, i, tap.d), line.at(1, i, tap.d))
            };
            bl[a + i] = l;
            br[a + i] = r;
        }
        st.in_line.advance(n);
        a = b;
    }
}

/// Copies frames `a..a + n` of `src` into every capture point on strip `k`
/// at `point`.
fn take(
    caps: &mut [Capture],
    k: usize,
    point: CapturePoint,
    src: &[Vec<f32>; 2],
    a: usize,
    b: usize,
) {
    for c in caps.iter_mut() {
        if c.src == Some((k as u16, point)) {
            c.buf[0][a..b].copy_from_slice(&src[0][a..b]);
            c.buf[1][a..b].copy_from_slice(&src[1][a..b]);
        }
    }
}

fn take_zeros(caps: &mut [Capture], k: usize, point: CapturePoint, len: usize) {
    for c in caps.iter_mut() {
        if c.src == Some((k as u16, point)) {
            c.buf[0][..len].fill(0.0);
            c.buf[1][..len].fill(0.0);
        }
    }
}

/// Runs one strip's fader, outputs and sends over the block, in pieces
/// split at its own events, the mixer's mute, solo and output changes and
/// every [`CHUNK`] frames.
#[allow(clippy::too_many_arguments)]
fn run_strip(
    st: &mut Strip,
    k: usize,
    hi: &mut [Strip],
    len: usize,
    epochs: &[Epoch],
    heard: bool,
    plan: &Plan,
    sc: &mut Scratch,
    caps: &mut [Capture],
    tap_step: f32,
    duck_step: f32,
    mut out: Option<(&mut [f32], &mut [f32])>,
) {
    let mut heard = heard;
    st.targets(plan, k, heard);
    let (mut ei, mut gi, mut at) = (0usize, 0usize, 0usize);
    while at < len {
        let mut moved = false;
        while ei < st.events.len() && st.events[ei].offset as usize <= at {
            if let NodeEvent::Param { id, value } = st.events[ei].ev {
                if !is_global(id) {
                    st.apply(id, value);
                }
            }
            ei += 1;
            moved = true;
        }
        while gi < epochs.len() && epochs[gi].offset as usize <= at {
            let e = epochs[gi];
            heard = e.aud.get(k);
            if e.strip as usize == k {
                st.out = e.out;
                st.out_track = e.out_track;
            }
            gi += 1;
            moved = true;
        }
        if moved {
            st.targets(plan, k, heard);
        }
        let mut next = (at + CHUNK).min(len);
        if let Some(e) = st.events.get(ei) {
            next = next.min(e.offset as usize);
        }
        if let Some(g) = epochs.get(gi) {
            next = next.min(g.offset as usize);
        }
        let o = out
            .as_mut()
            .map(|(l, r)| (&mut l[at..next], &mut r[at..next]));
        run_chunk(st, k, hi, at, next, sc, caps, tap_step, duck_step, o);
        at = next;
    }
    for q in ei..st.events.len() {
        if let NodeEvent::Param { id, value } = st.events[q].ev {
            if !is_global(id) {
                st.apply(id, value);
            }
        }
    }
    for e in &epochs[gi..] {
        if e.strip as usize == k {
            st.out = e.out;
            st.out_track = e.out_track;
        }
    }
    st.events.clear();
}

#[allow(clippy::too_many_arguments)]
fn run_chunk(
    st: &mut Strip,
    k: usize,
    hi: &mut [Strip],
    a: usize,
    b: usize,
    sc: &mut Scratch,
    caps: &mut [Capture],
    tap_step: f32,
    duck_step: f32,
    out: Option<(&mut [f32], &mut [f32])>,
) {
    let n = b - a;
    let Scratch {
        post,
        pre,
        w_route,
        w_send,
    } = sc;
    let [post_l, post_r] = post;
    let [pre_l, pre_r] = pre;
    let (post_l, post_r, pre_l, pre_r) = (
        &mut post_l[..n],
        &mut post_r[..n],
        &mut pre_l[..n],
        &mut pre_r[..n],
    );
    let (in_l, in_r) = (&st.buf[0][a..b], &st.buf[1][a..b]);
    peak(&mut st.peak_in, in_l, in_r);
    // The fader: volume and balance, mute, solo and monitoring, and any
    // edit's duck.
    if st.duck == Duck::Idle && st.aud.settled() && st.vp[0].settled() && st.vp[1].settled() {
        let m = st.aud.value;
        let (gl, gr) = (st.vp[0].value * m, st.vp[1].value * m);
        for i in 0..n {
            pre_l[i] = in_l[i] * m;
            pre_r[i] = in_r[i] * m;
            post_l[i] = in_l[i] * gl;
            post_r[i] = in_r[i] * gr;
        }
    } else {
        for i in 0..n {
            let m = st.aud.next() * duck_next(&mut st.duck, &mut st.duck_g, duck_step);
            let (gl, gr) = (st.vp[0].next(), st.vp[1].next());
            pre_l[i] = in_l[i] * m;
            pre_r[i] = in_r[i] * m;
            post_l[i] = in_l[i] * gl * m;
            post_r[i] = in_r[i] * gr * m;
        }
    }
    for c in caps.iter_mut() {
        if c.src == Some((k as u16, CapturePoint::AfterFader)) {
            c.buf[0][a..b].copy_from_slice(post_l);
            c.buf[1][a..b].copy_from_slice(post_r);
        }
    }
    if let Some((ol, or)) = out {
        ol.copy_from_slice(post_l);
        or.copy_from_slice(post_r);
        return;
    }
    peak(&mut st.peak, post_l, post_r);
    let sends = st.role.has_sends();
    if sends {
        st.pre_line.write(pre_l, pre_r);
    }
    st.post_line.write(post_l, post_r);
    for (r, w) in st.routes.iter_mut().zip(w_route.iter_mut()) {
        if r.tap.fading() {
            r.tap.weights(tap_step, &mut w[..n]);
        }
    }
    let ws = &mut w_send[..n];
    if sends && st.tap_send.fading() {
        st.tap_send.weights(tap_step, ws);
    }
    for (r, w) in st.routes.iter_mut().zip(w_route.iter()) {
        if r.idle() {
            continue;
        }
        let dst = &mut hi[r.dest as usize - k - 1];
        dst.touched = true;
        add(
            &st.post_line,
            &r.tap,
            &w[..n],
            post_l,
            post_r,
            &mut r.g,
            &mut dst.buf,
            a,
        );
    }
    if sends {
        for r in 0..RETURNS {
            // Returns come after every strip with sends.
            let j = hi.len() - 1 - RETURNS + r;
            let [gp, gq] = &mut st.send[r];
            hi[j].touched |= !(gp.settled() && gp.value == 0.0 && gq.settled() && gq.value == 0.0);
            if !(gp.settled() && gp.value == 0.0) {
                add(
                    &st.pre_line,
                    &st.tap_send,
                    ws,
                    pre_l,
                    pre_r,
                    gp,
                    &mut hi[j].buf,
                    a,
                );
            }
            if !(gq.settled() && gq.value == 0.0) {
                add(
                    &st.post_line,
                    &st.tap_send,
                    ws,
                    post_l,
                    post_r,
                    gq,
                    &mut hi[j].buf,
                    a,
                );
            }
        }
        st.pre_line.advance(n);
    }
    st.post_line.advance(n);
}

/// Adds a line's signal at a tap, times a gliding gain, into `dst` from
/// frame `a`. `now_l` and `now_r` are the chunk just written, for the
/// common case of no delay.
#[allow(clippy::too_many_arguments)]
#[inline]
fn add(
    line: &Line,
    tap: &Tap,
    w: &[f32],
    now_l: &[f32],
    now_r: &[f32],
    g: &mut Smooth,
    dst: &mut [Vec<f32>; 2],
    a: usize,
) {
    let n = now_l.len();
    let [dl, dr] = dst;
    let (dl, dr) = (&mut dl[a..a + n], &mut dr[a..a + n]);
    if !tap.fading() && tap.d == 0 {
        if g.settled() {
            let v = g.value;
            for i in 0..n {
                dl[i] += now_l[i] * v;
                dr[i] += now_r[i] * v;
            }
        } else {
            for i in 0..n {
                let v = g.next();
                dl[i] += now_l[i] * v;
                dr[i] += now_r[i] * v;
            }
        }
    } else if !tap.fading() {
        let (l0, l1) = line.runs(0, tap.d, n);
        let (r0, r1) = line.runs(1, tap.d, n);
        let m = l0.len();
        if g.settled() {
            let v = g.value;
            for i in 0..m {
                dl[i] += l0[i] * v;
                dr[i] += r0[i] * v;
            }
            for i in m..n {
                dl[i] += l1[i - m] * v;
                dr[i] += r1[i - m] * v;
            }
        } else {
            for i in 0..n {
                let v = g.next();
                let (a, b) = if i < m {
                    (l0[i], r0[i])
                } else {
                    (l1[i - m], r1[i - m])
                };
                dl[i] += a * v;
                dr[i] += b * v;
            }
        }
    } else {
        for i in 0..n {
            let v = g.next();
            let x = w[i];
            let l = line.at(0, i, tap.from) + (line.at(0, i, tap.d) - line.at(0, i, tap.from)) * x;
            let r = line.at(1, i, tap.from) + (line.at(1, i, tap.d) - line.at(1, i, tap.from)) * x;
            dl[i] += l * v;
            dr[i] += r * v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topo() -> Topo {
        Topo {
            inputs: 2,
            pads: 4,
            tracks: 4,
        }
    }

    fn sw() -> Switches {
        Switches::new()
    }

    #[test]
    fn strip_params_are_valid() {
        mpfree_node::check_params(strip_params()).unwrap();
        for (i, p) in strip_params().iter().enumerate() {
            assert_eq!(p.id as usize, i);
        }
        assert!(MAX_STRIPS <= Bits::default().0.len() * 64);
    }

    #[test]
    fn params_apply_by_kind() {
        assert!(param_applies(StripId::Master, ids::VOLUME));
        assert!(!param_applies(StripId::Master, ids::SOLO));
        assert!(!param_applies(StripId::Return(0), ids::send(0)));
        assert!(param_applies(StripId::Bus(3), ids::send_pre(3)));
        assert!(param_applies(StripId::Pad(0), ids::OUTPUT_TRACK));
        assert!(!param_applies(StripId::Track(0), ids::OUTPUT_TRACK));
        assert!(param_applies(StripId::Input(1), ids::MONITOR));
        assert!(!param_applies(StripId::Track(1), ids::SOURCE));
    }

    #[test]
    fn balance_keeps_the_near_side_at_unity() {
        assert_eq!(balance(0.0), (1.0, 1.0));
        assert_eq!(balance(1.0), (0.0, 1.0));
        assert_eq!(balance(-1.0), (1.0, 0.0));
        let (l, r) = balance(0.5);
        assert!((l - 0.5f32.sqrt()).abs() < 1e-6 && r == 1.0);
    }

    #[test]
    fn outputs_only_go_forward() {
        let t = topo();
        // A pad to its track, an input to a submix, a track to nowhere.
        assert_eq!(t.resolve(t.pad(1), OUT_TRACK, 2), Some(t.track(2)));
        assert_eq!(t.resolve(t.pad(1), OUT_TRACK, 9), Some(t.master()));
        assert_eq!(t.resolve(0, 3, 0), Some(t.bus(2)));
        assert_eq!(t.resolve(t.track(0), OUT_NONE, 0), None);
        // A track cannot go to a track; a submix only to a higher one.
        assert_eq!(t.resolve(t.track(0), OUT_TRACK, 1), Some(t.master()));
        assert_eq!(t.resolve(t.bus(2), 4, 0), Some(t.bus(3)));
        assert_eq!(t.resolve(t.bus(2), 3, 0), Some(t.master()));
        assert_eq!(t.resolve(t.ret(1), 4, 0), Some(t.master()));
        assert_eq!(t.resolve(t.master(), 0, 0), None);
        for k in 0..t.len() {
            for out in 0..=10 {
                if let Some(d) = t.resolve(k, out, 1) {
                    assert!(d > k);
                }
            }
        }
    }

    #[test]
    fn solo_follows_the_routing_and_spares_returns() {
        let t = topo();
        let mut s = sw();
        s.out[t.track(1)] = 1; // tracks 1 and 2 to submix 1
        s.out[t.track(2)] = 1;
        s.out[t.bus(0)] = 2; // submix 1 to submix 2
        s.out[t.pad(0)] = OUT_TRACK; // pad 0 to track 3
        s.out_track[t.pad(0)] = 3;
        s.solo.set(t.track(0), true);
        let a = t.audible(&s);
        assert!(a.get(t.track(0)));
        assert!(!a.get(t.track(1)) && !a.get(t.bus(0)) && !a.get(t.pad(0)));
        assert!(a.get(t.ret(0)) && a.get(t.master()));
        // A track inside a submix brings the submix and the one it feeds,
        // not its neighbour.
        s.solo.set(t.track(1), true);
        let a = t.audible(&s);
        assert!(a.get(t.bus(0)) && a.get(t.bus(1)) && !a.get(t.track(2)));
        // Soloing a submix brings everything that feeds it, all the way up.
        s.solo = Bits::default();
        s.solo.set(t.bus(1), true);
        let a = t.audible(&s);
        assert!(a.get(t.bus(0)) && a.get(t.track(1)) && a.get(t.track(2)));
        assert!(!a.get(t.track(0)) && !a.get(t.track(3)));
        // Soloing a pad brings its track; soloing the track brings the pad.
        s.solo = Bits::default();
        s.solo.set(t.pad(0), true);
        let a = t.audible(&s);
        assert!(a.get(t.track(3)) && !a.get(t.track(0)));
        s.solo = Bits::default();
        s.solo.set(t.track(3), true);
        assert!(t.audible(&s).get(t.pad(0)));
        // Mute wins over solo.
        s.mute.set(t.pad(0), true);
        assert!(!t.audible(&s).get(t.pad(0)));
    }

    #[test]
    fn keys_follow_the_stage_rules() {
        let t = topo();
        assert!(may_key(&t, t.track(3), t.track(0)));
        assert!(may_key(&t, t.pad(0), t.track(0)));
        assert!(may_key(&t, 0, t.bus(0)));
        assert!(may_key(&t, t.bus(0), t.bus(1)));
        assert!(!may_key(&t, t.bus(1), t.bus(0)));
        assert!(!may_key(&t, t.track(0), t.pad(0)));
        assert!(!may_key(&t, t.track(0), t.track(0)));
        assert!(may_key(&t, t.ret(2), t.master()));
    }
}
