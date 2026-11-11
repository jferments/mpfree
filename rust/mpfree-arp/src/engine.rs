//! The engine: notes in (pads, screen, MIDI), sample-timed notes out.
//!
//! It runs inside the audio thread. The app calls [`Engine::press`],
//! [`Engine::release`] and the setters between audio periods, then
//! [`Engine::process`] once per period, which returns the note-ons and
//! note-offs that fall inside that period with their frame offsets. Nothing
//! allocates, locks or does I/O after [`Engine::new`].
//!
//! On its own the engine free-runs: the first step falls on the press and
//! the tempo comes from [`Engine::set_tempo`]. With a transport, the host
//! calls [`Engine::sync`] with a [`Clock`] before every `process` call, and
//! while the transport plays the steps fall on its tick grid instead.

use crate::notes::{Held, LatchMode, NoteSet, Notes, MAX_NOTES};
use crate::pattern::{Pattern, Pick, Step};
use crate::scale::{Scale, Snap, CHROMATIC, MAJOR};

/// Clock resolution: ticks per quarter note.
pub const PPQ: u32 = 960;
/// Events one `process` call can return; more are dropped and counted.
/// As many as can be pending, so a panic's 128 note-offs and the notes
/// pressed right after it fit in one call.
pub const MAX_EVENTS: usize = MAX_PENDING;
/// Notes waiting to start or stop.
const MAX_PENDING: usize = 256;

/// Step length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rate {
    Quarter,
    Eighth,
    EighthTriplet,
    Sixteenth,
    SixteenthTriplet,
    ThirtySecond,
    ThirtySecondTriplet,
}

impl Rate {
    pub const ALL: [Rate; 7] = [
        Rate::Quarter,
        Rate::Eighth,
        Rate::EighthTriplet,
        Rate::Sixteenth,
        Rate::SixteenthTriplet,
        Rate::ThirtySecond,
        Rate::ThirtySecondTriplet,
    ];

    pub const fn ticks(self) -> u32 {
        match self {
            Rate::Quarter => PPQ,
            Rate::Eighth => PPQ / 2,
            Rate::EighthTriplet => PPQ / 3,
            Rate::Sixteenth => PPQ / 4,
            Rate::SixteenthTriplet => PPQ / 6,
            Rate::ThirtySecond => PPQ / 8,
            Rate::ThirtySecondTriplet => PPQ / 12,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Rate::Quarter => "1/4",
            Rate::Eighth => "1/8",
            Rate::EighthTriplet => "1/8T",
            Rate::Sixteenth => "1/16",
            Rate::SixteenthTriplet => "1/16T",
            Rate::ThirtySecond => "1/32",
            Rate::ThirtySecondTriplet => "1/32T",
        }
    }
}

/// The order the arpeggiator walks the notes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Up,
    Down,
    /// Up then down, without repeating the top and bottom notes.
    UpDown,
    /// Up then down, playing the top and bottom notes twice.
    UpDownInclusive,
    /// Outside in: highest, lowest, second highest, second lowest...
    Converge,
    /// Inside out: the reverse of Converge.
    Diverge,
    /// The order the notes were pressed.
    Played,
    Random,
    /// Random, but no note repeats until every note has played.
    RandomOther,
    /// One random order, kept until the held notes change.
    RandomOnce,
    /// All notes together on every step.
    Chord,
}

/// A chord built on each held note from the scale (stacked scale steps), so
/// one pad can play a whole chord for the arp to run over. Without a scale,
/// it is built from the major scale on the held note.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Chord {
    #[default]
    Off,
    /// 1 and 5.
    Power,
    /// 1 3 5.
    Triad,
    /// 1 2 5.
    Sus2,
    /// 1 4 5.
    Sus4,
    /// 1 3 5 7.
    Seventh,
    /// 1 3 5 7 9.
    Ninth,
}

impl Chord {
    pub const ALL: [Chord; 7] = [
        Chord::Off,
        Chord::Power,
        Chord::Triad,
        Chord::Sus2,
        Chord::Sus4,
        Chord::Seventh,
        Chord::Ninth,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Chord::Off => "Off",
            Chord::Power => "Power (1 5)",
            Chord::Triad => "Triad (1 3 5)",
            Chord::Sus2 => "Sus2 (1 2 5)",
            Chord::Sus4 => "Sus4 (1 4 5)",
            Chord::Seventh => "Seventh (1 3 5 7)",
            Chord::Ninth => "Ninth (1 3 5 7 9)",
        }
    }

    /// Scale steps above the held note.
    pub fn steps(self) -> &'static [i16] {
        match self {
            Chord::Off => &[0],
            Chord::Power => &[0, 4],
            Chord::Triad => &[0, 2, 4],
            Chord::Sus2 => &[0, 1, 4],
            Chord::Sus4 => &[0, 3, 4],
            Chord::Seventh => &[0, 2, 4, 6],
            Chord::Ninth => &[0, 2, 4, 6, 8],
        }
    }
}

/// Held notes taking turns instead of sounding together: with two or more
/// held, each step plays from one of them (grown into its chord when
/// [`Settings::chord`] is on), in the order they were pressed, for a beat or
/// a bar each. Holding Am, F, C and G then arpeggiates a progression.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Turns {
    #[default]
    Off,
    Beat,
    TwoBeats,
    Bar,
    TwoBars,
}

impl Turns {
    pub const ALL: [Turns; 5] = [
        Turns::Off,
        Turns::Beat,
        Turns::TwoBeats,
        Turns::Bar,
        Turns::TwoBars,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Turns::Off => "Off",
            Turns::Beat => "Every beat",
            Turns::TwoBeats => "Every 2 beats",
            Turns::Bar => "Every bar",
            Turns::TwoBars => "Every 2 bars",
        }
    }

    /// Quarter notes per turn (a bar is 4/4), 0 for Off.
    pub fn beats(self) -> u32 {
        match self {
            Turns::Off => 0,
            Turns::Beat => 1,
            Turns::TwoBeats => 2,
            Turns::Bar => 4,
            Turns::TwoBars => 8,
        }
    }
}

/// Steps each held note plays for while they take turns, at the current
/// rate (at least 1); 0 when they don't.
pub fn turn_steps(s: &Settings) -> u64 {
    match s.turns.beats() {
        0 => 0,
        b => (u64::from(b * PPQ) / u64::from(s.rate.ticks().max(1))).max(1),
    }
}

/// Which held note, by press order, step `step` plays from when held notes
/// take turns: None when they don't, in Direct mode, or with fewer than two
/// held.
pub fn turn_of(s: &Settings, held: usize, step: u64) -> Option<usize> {
    let len = turn_steps(s);
    (len > 0 && held >= 2 && s.mode != Mode::Direct).then(|| ((step / len) % held as u64) as usize)
}

/// The notes a step works from: the held notes, sorted, each grown into a
/// chord ([`Settings::chord`]), then, for the Arp and Pattern modes, every
/// scale note between the lowest and the highest ([`Settings::run`]; one
/// octave up from a single note). `list[..n]` in, the new count out, still
/// sorted. The screens use it too, to show what the steps will play.
pub fn step_notes(s: &Settings, list: &mut [Held; MAX_NOTES], n: usize) -> usize {
    let mut n = n.min(MAX_NOTES);
    if s.chord != Chord::Off {
        let mut out = [Held::default(); MAX_NOTES];
        let mut m = 0;
        for h in &list[..n] {
            let scale = s.scale.unwrap_or(Scale::new(h.note % 12, MAJOR));
            let Some(root) = scale.snapped(i16::from(h.note), s.snap) else {
                continue;
            };
            for &step in s.chord.steps() {
                let note = scale.walk(root, step);
                if m < MAX_NOTES
                    && (0..=127).contains(&note)
                    && !out[..m].iter().any(|x| i16::from(x.note) == note)
                {
                    out[m] = Held {
                        note: note as u8,
                        velocity: h.velocity,
                    };
                    m += 1;
                }
            }
        }
        out[..m].sort_unstable_by_key(|h| h.note);
        list[..m].copy_from_slice(&out[..m]);
        n = m;
    }
    if s.run && n > 0 && matches!(s.mode, Mode::Arp | Mode::Pattern) {
        let scale = s.scale.unwrap_or(Scale::new(0, CHROMATIC));
        let lo = i16::from(list[0].note);
        let hi = if n == 1 {
            lo + 11
        } else {
            i16::from(list[n - 1].note)
        };
        let mut out = [Held::default(); MAX_NOTES];
        let mut m = 0;
        let mut k = 0;
        while m < MAX_NOTES {
            let note = scale.nth(lo, k);
            if note > hi || note > 127 {
                break;
            }
            // Each run note takes the velocity of the held note at or below it.
            let velocity = list[..n]
                .iter()
                .rev()
                .find(|h| i16::from(h.note) <= note)
                .unwrap_or(&list[0])
                .velocity;
            out[m] = Held {
                note: note as u8,
                velocity,
            };
            m += 1;
            k += 1;
        }
        list[..m].copy_from_slice(&out[..m]);
        n = m;
    }
    n
}

impl Order {
    pub const ALL: [Order; 11] = [
        Order::Up,
        Order::Down,
        Order::UpDown,
        Order::UpDownInclusive,
        Order::Converge,
        Order::Diverge,
        Order::Played,
        Order::Random,
        Order::RandomOther,
        Order::RandomOnce,
        Order::Chord,
    ];

    /// Which of `total` places (the held notes times the octaves, lowest
    /// first; for As played, in press order) count `c` of the order plays.
    /// None for the random orders, which draw from the engine's generator.
    /// Chord plays every note; this gives the position as for Up.
    pub const fn place(self, total: usize, c: usize) -> Option<usize> {
        if total == 0 {
            return None;
        }
        Some(match self {
            Order::Up | Order::Played | Order::Chord => c % total,
            Order::Down => total - 1 - c % total,
            Order::UpDown if total == 1 => 0,
            Order::UpDown => {
                let p = c % (2 * total - 2);
                if p < total {
                    p
                } else {
                    2 * total - 2 - p
                }
            }
            Order::UpDownInclusive => {
                let p = c % (2 * total);
                if p < total {
                    p
                } else {
                    2 * total - 1 - p
                }
            }
            Order::Converge | Order::Diverge => {
                let k = if matches!(self, Order::Converge) {
                    c % total
                } else {
                    total - 1 - c % total
                };
                if k % 2 == 0 {
                    total - 1 - k / 2
                } else {
                    k / 2
                }
            }
            Order::Random | Order::RandomOther | Order::RandomOnce => return None,
        })
    }

    /// How many counts the order takes before it repeats; None for the
    /// random orders.
    pub const fn cycle(self, total: usize) -> Option<usize> {
        if total == 0 {
            return None;
        }
        Some(match self {
            Order::UpDown if total == 1 => 1,
            Order::UpDown => 2 * total - 2,
            Order::UpDownInclusive => 2 * total,
            Order::Random | Order::RandomOther | Order::RandomOnce => return None,
            _ => total,
        })
    }

    pub const fn label(self) -> &'static str {
        match self {
            Order::Up => "Up",
            Order::Down => "Down",
            Order::UpDown => "Up-down",
            Order::UpDownInclusive => "Up-down incl.",
            Order::Converge => "Converge",
            Order::Diverge => "Diverge",
            Order::Played => "As played",
            Order::Random => "Random",
            Order::RandomOther => "Random, no repeats",
            Order::RandomOnce => "Random once",
            Order::Chord => "Chord",
        }
    }
}

/// Where the generator starts when notes arrive while the transport plays.
/// Without a playing transport it always starts on the press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncStart {
    /// The next step of the transport's grid.
    Step,
    /// The next beat (quarter note).
    Beat,
    /// The next bar line ([`Clock::bar_ticks`]).
    Bar,
}

impl SyncStart {
    pub const ALL: [SyncStart; 3] = [SyncStart::Step, SyncStart::Beat, SyncStart::Bar];

    pub const fn label(self) -> &'static str {
        match self {
            SyncStart::Step => "Step",
            SyncStart::Beat => "Beat",
            SyncStart::Bar => "Bar",
        }
    }
}

/// The transport's clock for one `process` call (one span: one tempo, one
/// play state, no jump inside it). Ticks are [`PPQ`] per quarter note and
/// can be negative (count-in). Tick `t` falls on frame
/// `anchor_frame + round((t - anchor_tick) * 60 * sample_rate / (bpm * PPQ))`,
/// where the anchor frame is `frames_since_anchor` frames before the start
/// of this call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    /// A whole tick that falls exactly on the anchor frame.
    pub anchor_tick: i64,
    /// Frames from the anchor to the start of this `process` call.
    pub frames_since_anchor: u64,
    pub bpm: f64,
    /// False: the engine free-runs as it does without a transport.
    pub playing: bool,
    /// A loop wrap or locate happened at the anchor: the next step is
    /// placed on the grid again from the new position.
    pub jumped: bool,
    /// Transport swing: (grid in ticks, 50..=75). None uses
    /// [`Settings::swing`] on the arp's own step grid.
    pub swing: Option<(u32, u8)>,
    /// Ticks per bar, for [`SyncStart::Bar`] (3840 in 4/4).
    pub bar_ticks: u32,
}

impl Clock {
    /// A playing 4/4 clock with no swing, anchored `frames_since_anchor`
    /// frames before this call.
    pub const fn playing(anchor_tick: i64, frames_since_anchor: u64, bpm: f64) -> Self {
        Self {
            anchor_tick,
            frames_since_anchor,
            bpm,
            playing: true,
            jumped: false,
            swing: None,
            bar_ticks: 4 * PPQ,
        }
    }
}

/// The last clock from [`Engine::sync`], in the engine's own frames.
#[derive(Clone, Copy, Debug)]
struct Synced {
    /// Engine frame of `anchor_tick` (a whole number, maybe negative).
    anchor_frame: f64,
    anchor_tick: i64,
    swing: Option<(u32, u8)>,
    bar_ticks: u32,
}

/// What the held notes do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Notes play as pressed (Latch still holds them).
    Direct,
    /// Note Repeat: every held note retriggers on every step.
    Repeat,
    /// Arpeggiator: one note per step, in [`Settings::order`].
    Arp,
    /// The latch pattern plays over the held notes.
    Pattern,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Velocity {
    /// The velocity (or latest pressure) the note was played with.
    Played,
    Fixed(u8),
}

/// Velocity that moves from where it starts towards `target` over `steps`
/// steps after the notes start, then stays there (like a decay to a target).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ramp {
    /// 1..=127.
    pub target: u8,
    /// Steps to reach the target; 0 turns the ramp off.
    pub steps: u16,
}

impl Ramp {
    pub const OFF: Ramp = Ramp {
        target: 127,
        steps: 0,
    };

    /// `velocity` after `step` steps of the ramp.
    pub fn apply(self, velocity: u8, step: u64) -> u8 {
        if self.steps == 0 {
            return velocity;
        }
        let k = (step as f64 / f64::from(self.steps)).min(1.0);
        let v = f64::from(velocity) + (f64::from(self.target) - f64::from(velocity)) * k;
        v.round().clamp(1.0, 127.0) as u8
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub mode: Mode,
    pub rate: Rate,
    pub order: Order,
    /// 1..=4 octaves for the arpeggiator's order.
    pub octaves: u8,
    /// Note length in percent of a step, 1..=100.
    pub gate: u8,
    /// 50 (straight) to 75: where every second step falls within its pair.
    pub swing: u8,
    pub velocity: Velocity,
    /// Velocity of accented pattern steps.
    pub accent: u8,
    /// Keeps generated notes in key, applied last. None for drum pads.
    pub scale: Option<Scale>,
    /// What happens to a note outside the scale: the scale note below
    /// (the default), above, the nearest, or not played.
    pub snap: Snap,
    /// Each held note plays a chord built from the scale.
    pub chord: Chord,
    /// Arp and Pattern modes: the steps run through every scale note
    /// between the lowest and the highest held note (one octave from a
    /// single note), in the order's order.
    pub run: bool,
    /// Held notes take turns, one chord at a time, instead of sounding
    /// together.
    pub turns: Turns,
    /// Spreads the notes of a chord step across 0..=100 % of the step,
    /// lowest first, like a strum.
    pub strum: u8,
    /// A new note restarts the order and pattern from their first step.
    pub retrigger: bool,
    /// Steps to play after the notes start, then stop; 0 = keep going.
    pub repeats: u16,
    /// Velocity ramp after the notes start (accents are left alone).
    pub ramp: Ramp,
    /// Where notes start while the transport plays.
    pub start: SyncStart,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Direct,
            rate: Rate::Sixteenth,
            order: Order::Up,
            octaves: 1,
            gate: 50,
            swing: 50,
            velocity: Velocity::Played,
            accent: 127,
            scale: None,
            snap: Snap::Down,
            chord: Chord::Off,
            run: false,
            turns: Turns::Off,
            strum: 0,
            retrigger: false,
            repeats: 0,
            ramp: Ramp::OFF,
            start: SyncStart::Step,
        }
    }
}

impl Settings {
    /// Brings every field into its range.
    pub fn clamped(mut self) -> Self {
        self.octaves = self.octaves.clamp(1, 4);
        self.gate = self.gate.clamp(1, 100);
        self.swing = self.swing.clamp(50, 75);
        self.accent = self.accent.clamp(1, 127);
        self.strum = self.strum.min(100);
        self.ramp.target = self.ramp.target.clamp(1, 127);
        if let Velocity::Fixed(v) = self.velocity {
            self.velocity = Velocity::Fixed(v.clamp(1, 127));
        }
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    On { note: u8, velocity: u8 },
    Off { note: u8 },
}

/// A note-on or note-off, `offset` frames into the period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub offset: u32,
    pub kind: EventKind,
}

/// The events of one period, in time order (offs before ons at the same frame).
pub struct Events {
    buf: [Event; MAX_EVENTS],
    len: usize,
    /// Events that did not fit, since the engine started.
    pub dropped: u64,
}

impl Default for Events {
    fn default() -> Self {
        Self::new()
    }
}

impl Events {
    pub const fn new() -> Self {
        Self {
            buf: [Event {
                offset: 0,
                kind: EventKind::Off { note: 0 },
            }; MAX_EVENTS],
            len: 0,
            dropped: 0,
        }
    }

    pub fn as_slice(&self) -> &[Event] {
        &self.buf[..self.len]
    }

    fn push(&mut self, e: Event) {
        if self.len < MAX_EVENTS {
            self.buf[self.len] = e;
            self.len += 1;
        } else {
            self.dropped += 1;
        }
    }
}

/// How one note of a step is played.
#[derive(Clone, Copy, Debug)]
struct Hit {
    /// Octaves added to the held note.
    octave: i16,
    /// Scale degrees (or semitones without a scale) added after the octave.
    degree: i16,
    /// Overrides the note's own velocity.
    velocity: Option<u8>,
    /// Accented: the velocity is used as it is, with no ramp or random.
    accent: bool,
    /// Random velocity offset from the `vel~N` lane.
    jitter: i16,
    ratchet: u8,
    gate: u8,
    /// Frames after the step's start (strum).
    delay: f64,
}

impl Hit {
    fn plain(gate: u8) -> Self {
        Self {
            octave: 0,
            degree: 0,
            velocity: None,
            accent: false,
            jitter: 0,
            ratchet: 1,
            gate,
            delay: 0.0,
        }
    }
}

const MAX_ORDER: usize = MAX_NOTES * 4;

#[derive(Clone, Copy, Debug)]
struct Pending {
    /// Absolute time in frames.
    at: f64,
    kind: EventKind,
}

pub struct Engine {
    settings: Settings,
    pattern: Pattern,
    sample_rate: f64,
    bpm: f64,
    notes: Notes,
    /// Frame at the start of the next period.
    now: u64,
    running: bool,
    /// Frame of the next step on the straight grid (swing is added on top).
    next_step: f64,
    step: u64,
    arp_counter: u64,
    queue: [Pending; MAX_PENDING],
    queued: usize,
    /// Extra length, in frames, for the last hit of the current step: the
    /// tie steps that follow it.
    tied: f64,
    rng: u32,
    /// Random no-repeat: which positions of the order have played.
    bag: u128,
    /// Random once: the order, and whether it still matches the notes.
    once: [u8; MAX_ORDER],
    once_len: usize,
    /// The nudge, in percent of a step, already drawn for step `.0`, so a
    /// random nudge is drawn once per step and not once per period.
    nudge: Option<(u64, i16)>,
    /// When the last step fired, so a nudge never puts a step before the
    /// one ahead of it.
    last_fire: f64,
    /// The transport, while it plays. None: free-running.
    sync: Option<Synced>,
    /// Tick of the next step on the straight grid, while synced.
    next_tick: i64,
    /// Notes that could not be queued, since the engine started.
    pub overflow: u64,
    /// Silenced by [`Engine::hush`]: the held and latched notes stay, but
    /// no steps play until [`Engine::resume`] or the next press.
    hushed: bool,
}

impl Engine {
    pub fn new(sample_rate: u32, bpm: f64) -> Self {
        Self {
            settings: Settings::default(),
            pattern: Pattern::default(),
            sample_rate: f64::from(sample_rate.max(1)),
            bpm: bpm.clamp(20.0, 300.0),
            notes: Notes::new(),
            now: 0,
            running: false,
            hushed: false,
            next_step: 0.0,
            step: 0,
            arp_counter: 0,
            queue: [Pending {
                at: 0.0,
                kind: EventKind::Off { note: 0 },
            }; MAX_PENDING],
            queued: 0,
            tied: 0.0,
            rng: 0x2545_f491,
            bag: 0,
            once: [0; MAX_ORDER],
            once_len: 0,
            nudge: None,
            last_fire: f64::MIN,
            sync: None,
            next_tick: 0,
            overflow: 0,
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    pub fn pattern(&self) -> &Pattern {
        &self.pattern
    }

    pub fn bpm(&self) -> f64 {
        self.bpm
    }

    /// The notes the generators play (held plus latched), for the screen.
    pub fn active(&self) -> &NoteSet {
        self.notes.active()
    }

    pub fn latch(&self) -> LatchMode {
        self.notes.mode()
    }

    /// Steps played since the notes last started, for a step cursor.
    pub fn step(&self) -> u64 {
        self.step
    }

    /// Where the generator is now, in steps since the notes started,
    /// including the fraction of the current step (swing not counted).
    /// Free-time recording uses it to place pad presses. 0 when stopped,
    /// below 0 while a synced start waits for its beat or bar.
    pub fn position(&self) -> f64 {
        if !self.running {
            return 0.0;
        }
        self.step as f64 - (self.next_step - self.now as f64) / self.step_frames()
    }

    pub fn set_seed(&mut self, seed: u32) {
        self.rng = seed.max(1);
    }

    /// The free-running tempo. While a transport plays, [`Engine::sync`]
    /// sets the tempo instead.
    pub fn set_tempo(&mut self, bpm: f64) {
        self.bpm = bpm.clamp(20.0, 300.0);
    }

    /// True while the steps follow a playing transport.
    pub fn synced(&self) -> bool {
        self.sync.is_some()
    }

    /// Follows the transport. Call it before every [`Engine::process`]
    /// call, with the clock of the span that call covers. While
    /// `clock.playing` is false the engine free-runs as if never synced,
    /// carrying on from where the transport left it.
    pub fn sync(&mut self, clock: &Clock) {
        if !clock.playing {
            self.sync = None;
            return;
        }
        let was = self.sync.is_some();
        self.bpm = clock.bpm.clamp(20.0, 300.0);
        self.sync = Some(Synced {
            anchor_frame: self.now as f64 - clock.frames_since_anchor as f64,
            anchor_tick: clock.anchor_tick,
            swing: clock
                .swing
                .map(|(grid, amount)| (grid.max(1), amount.clamp(50, 75))),
            bar_ticks: clock.bar_ticks.max(1),
        });
        if self.running {
            if !was || clock.jumped {
                self.next_tick = self.grid_after(i64::from(self.settings.rate.ticks()));
            }
            self.next_step = self.frame_of(self.next_tick as f64);
        }
    }

    pub fn set_pattern(&mut self, pattern: Pattern) {
        self.pattern = pattern;
    }

    /// Changing the mode stops notes that were scheduled but not started.
    pub fn set_settings(&mut self, settings: Settings) {
        let settings = settings.clamped();
        if settings.mode != self.settings.mode {
            self.drop_pending_ons();
            if self.settings.mode == Mode::Direct {
                // Notes held in Direct mode are sounding: stop them; the
                // generator takes over from the next step.
                let mut list = [Held::default(); MAX_NOTES];
                let n = self.notes.active().sorted(&mut list);
                for h in &list[..n] {
                    self.queue_at(self.now as f64, EventKind::Off { note: h.note });
                }
            }
            self.running = false;
        }
        let regrid = settings.rate != self.settings.rate;
        if settings.octaves != self.settings.octaves || settings.order != self.settings.order {
            self.once_len = 0;
            self.bag = 0;
        }
        self.settings = settings;
        if regrid && self.running && self.sync.is_some() {
            self.next_tick = self.grid_after(i64::from(settings.rate.ticks()));
            self.next_step = self.frame_of(self.next_tick as f64);
        }
        self.update_running();
    }

    pub fn set_latch(&mut self, mode: LatchMode) {
        let before = self.notes.active().clone();
        self.notes.set_mode(mode);
        self.after_change(&before);
    }

    /// Drops the latched notes, keeping what is still held.
    pub fn clear_latched(&mut self) {
        let before = self.notes.active().clone();
        self.notes.clear_latched();
        self.after_change(&before);
    }

    pub fn press(&mut self, note: u8, velocity: u8) {
        self.hushed = false;
        let before = self.notes.active().clone();
        self.notes.press(note, velocity.max(1));
        self.after_change(&before);
    }

    pub fn release(&mut self, note: u8) {
        let before = self.notes.active().clone();
        self.notes.release(note);
        self.after_change(&before);
    }

    pub fn pressure(&mut self, note: u8, value: u8) {
        self.notes.pressure(note, value.max(1));
    }

    /// Silences the arp at the start of the next period (all notes off) but
    /// keeps what is held and latched, so [`Engine::resume`] (or the next
    /// press) carries on with the same notes. Decision 5 of the design doc:
    /// STOP silences a latched arp and keeps the latch.
    pub fn hush(&mut self) {
        self.hushed = true;
        self.queued = 0;
        self.running = false;
        let at = self.now as f64;
        for n in 0..128u8 {
            self.queue_at(at, EventKind::Off { note: n });
        }
    }

    /// Plays again after [`Engine::hush`]; nothing happens if not hushed.
    pub fn resume(&mut self) {
        if self.hushed {
            self.hushed = false;
            self.update_running();
        }
    }

    /// Silenced by [`Engine::hush`] and not resumed yet.
    pub fn hushed(&self) -> bool {
        self.hushed
    }

    /// Stops everything at the start of the next period: all notes off,
    /// nothing held or latched.
    pub fn panic(&mut self) {
        self.hushed = false;
        self.queued = 0;
        self.notes.clear_all();
        self.running = false;
        let at = self.now as f64;
        for n in 0..128u8 {
            // One per note is more than MAX_PENDING allows only if 128 > 256; it isn't.
            self.queue_at(at, EventKind::Off { note: n });
        }
    }

    /// Runs one period of `frames` frames and returns its events in `out`.
    pub fn process(&mut self, frames: u32, out: &mut Events) {
        out.len = 0;
        let end = self.now + u64::from(frames);
        if self.running && self.sync.is_some() {
            let ticks = i64::from(self.settings.rate.ticks());
            loop {
                let len = self.step_frames();
                let nudge = len * f64::from(self.nudge_percent()) / 100.0;
                let swung = self.swung(self.next_tick, ticks);
                let fire = (self.frame_of(swung) + nudge).max(self.last_fire);
                if fire >= end as f64 {
                    break;
                }
                self.fire_step(fire, len);
                self.last_fire = fire;
                self.step += 1;
                self.next_tick += ticks;
            }
            self.next_step = self.frame_of(self.next_tick as f64);
        } else if self.running {
            loop {
                let len = self.step_frames();
                let nudge = len * f64::from(self.nudge_percent()) / 100.0;
                let fire = (self.next_step + self.swing_delay(len) + nudge).max(self.last_fire);
                if fire >= end as f64 {
                    break;
                }
                self.fire_step(fire, len);
                self.last_fire = fire;
                self.step += 1;
                self.next_step += len;
            }
        }
        self.drain(end, out);
        self.now = end;
    }

    fn step_frames(&self) -> f64 {
        f64::from(self.settings.rate.ticks()) / f64::from(PPQ) * 60.0 / self.bpm * self.sample_rate
    }

    /// Frames per tick at the current tempo.
    fn frames_per_tick(&self) -> f64 {
        60.0 * self.sample_rate / (self.bpm * f64::from(PPQ))
    }

    /// The engine frame of tick `t`, rounded as the transport rounds it.
    /// Only called while synced.
    fn frame_of(&self, t: f64) -> f64 {
        let Some(s) = self.sync else {
            return self.next_step;
        };
        s.anchor_frame + ((t - s.anchor_tick as f64) * self.frames_per_tick()).round()
    }

    /// The first multiple of `unit` ticks that falls on or after the
    /// current frame. Only called while synced.
    fn grid_after(&self, unit: i64) -> i64 {
        let Some(s) = self.sync else {
            return self.next_tick;
        };
        let unit = unit.max(1);
        let now = self.now as f64;
        let tick = s.anchor_tick as f64 + (now - s.anchor_frame) / self.frames_per_tick();
        let mut t = (tick.floor() as i64).div_euclid(unit) * unit;
        while self.frame_of(t as f64) < now {
            t += unit;
        }
        while self.frame_of((t - unit) as f64) >= now {
            t -= unit;
        }
        t
    }

    /// Tick `t` moved by swing: every second cell of the swing grid starts
    /// late by (swing - 50) / 50 of a cell, and the points inside each pair
    /// of cells are stretched or squeezed to match. Without transport swing
    /// the grid is the arp's own step, as when free-running.
    fn swung(&self, t: i64, step_ticks: i64) -> f64 {
        let (grid, amount) = match self.sync.and_then(|s| s.swing) {
            Some((grid, amount)) => (i64::from(grid), amount),
            None => (step_ticks, self.settings.swing),
        };
        let s = f64::from(amount.saturating_sub(50)) / 50.0;
        if s == 0.0 {
            return t as f64;
        }
        let pair = 2 * grid;
        let base = t.div_euclid(pair) * pair;
        let p = (t - base) as f64;
        let g = grid as f64;
        let moved = if p < g {
            p * (1.0 + s)
        } else {
            g * (1.0 + s) + (p - g) * (1.0 - s)
        };
        base as f64 + moved
    }

    /// The nudge lane's value for the coming step, plus its random part,
    /// drawn once per step. Only patterns have lanes.
    fn nudge_percent(&mut self) -> i16 {
        let lane = self.pattern.lanes.nudge;
        if self.settings.mode != Mode::Pattern || lane.is_off() {
            return 0;
        }
        if let Some((step, n)) = self.nudge {
            if step == self.step {
                return n;
            }
        }
        let n = (i16::from(lane.at(self.step).unwrap_or(0)) + self.jitter(lane.random()))
            .clamp(-50, 50);
        self.nudge = Some((self.step, n));
        n
    }

    /// A random whole number from -amount to amount (0 draws nothing).
    fn jitter(&mut self, amount: u8) -> i16 {
        if amount == 0 {
            0
        } else {
            self.random(2 * usize::from(amount) + 1) as i16 - i16::from(amount)
        }
    }

    /// Every second step moves later by (swing - 50) / 50 of a step.
    fn swing_delay(&self, len: f64) -> f64 {
        if self.step % 2 == 1 {
            len * f64::from(self.settings.swing - 50) / 50.0
        } else {
            0.0
        }
    }

    fn after_change(&mut self, before: &NoteSet) {
        let added = self
            .notes
            .active()
            .as_slice()
            .iter()
            .any(|h| !before.contains(h.note));
        let removed = before
            .as_slice()
            .iter()
            .any(|h| !self.notes.active().contains(h.note));
        if added || removed {
            self.once_len = 0;
            self.bag = 0;
        }
        if added && self.running && self.settings.retrigger {
            self.step = 0;
            self.arp_counter = 0;
            self.nudge = None;
        }
        if self.settings.mode == Mode::Direct {
            let at = self.now as f64;
            for h in before.as_slice() {
                if !self.notes.active().contains(h.note) {
                    self.queue_at(at, EventKind::Off { note: h.note });
                }
            }
            let mut added = [Held::default(); MAX_NOTES];
            let mut n = 0;
            for h in self.notes.active().as_slice() {
                if !before.contains(h.note) {
                    added[n] = *h;
                    n += 1;
                }
            }
            for h in &added[..n] {
                self.queue_at(
                    at,
                    EventKind::On {
                        note: h.note,
                        velocity: self.velocity_of(h.velocity),
                    },
                );
            }
        }
        self.update_running();
    }

    fn update_running(&mut self) {
        let want =
            !self.hushed && self.settings.mode != Mode::Direct && !self.notes.active().is_empty();
        if want && !self.running {
            // Free-running, the first step is on the press. With a playing
            // transport it waits for the next step, beat or bar.
            self.running = true;
            if let Some(s) = self.sync {
                let unit = match self.settings.start {
                    SyncStart::Step => self.settings.rate.ticks(),
                    SyncStart::Beat => PPQ,
                    SyncStart::Bar => s.bar_ticks,
                };
                self.next_tick = self.grid_after(i64::from(unit));
                self.next_step = self.frame_of(self.next_tick as f64);
            } else {
                self.next_step = self.now as f64;
            }
            self.last_fire = f64::MIN;
            self.nudge = None;
            self.step = 0;
            self.arp_counter = 0;
        } else if !want && self.running {
            self.running = false;
            self.drop_pending_ons();
        }
    }

    fn velocity_of(&self, played: u8) -> u8 {
        match self.settings.velocity {
            Velocity::Played => played,
            Velocity::Fixed(v) => v,
        }
    }

    fn random(&mut self, n: usize) -> usize {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as usize) % n.max(1)
    }

    fn fire_step(&mut self, at: f64, len: f64) {
        let mut sorted = [Held::default(); MAX_NOTES];
        let active = self.notes.active();
        let n = match turn_of(&self.settings, active.len(), self.step) {
            Some(k) => {
                sorted[0] = active.as_slice()[k];
                // Each turn starts the order again, from its first note.
                if self.step.checked_rem(turn_steps(&self.settings)) == Some(0) {
                    self.arp_counter = 0;
                    self.bag = 0;
                    self.once_len = 0;
                }
                1
            }
            None => active.sorted(&mut sorted),
        };
        let n = step_notes(&self.settings, &mut sorted, n);
        if n == 0 {
            return;
        }
        if self.settings.repeats > 0 && self.step >= u64::from(self.settings.repeats) {
            return;
        }
        let gate = self.settings.gate;
        self.tied = 0.0;
        match self.settings.mode {
            Mode::Direct => {}
            Mode::Repeat => self.chord(&sorted[..n], Hit::plain(gate), at, len),
            Mode::Arp => self.play_next(&sorted[..n], Hit::plain(gate), at, len),
            Mode::Pattern => {
                let step = self.pattern.steps()[(self.step % self.pattern.len() as u64) as usize];
                // A tie was already added to the notes of the step before it,
                // when they were scheduled.
                let Step::Play {
                    pick,
                    octave,
                    accent,
                    gate: g,
                    ratchet,
                    chance,
                } = step
                else {
                    return;
                };
                self.tied = self.ties_after(self.step) as f64 * len;
                if chance < 100 && self.random(100) >= usize::from(chance) {
                    return;
                }
                let lanes = self.pattern.lanes;
                let lane_vel = lanes.velocity.at(self.step).filter(|&v| v > 0);
                let lane_gate = lanes.gate.at(self.step).filter(|&v| v > 0);
                let gate = if g != 0 { g } else { lane_gate.unwrap_or(gate) };
                let hit = Hit {
                    octave: i16::from(octave)
                        + i16::from(lanes.octave.at(self.step).unwrap_or(0))
                        + self.jitter(lanes.octave.random()),
                    degree: i16::from(lanes.degree.at(self.step).unwrap_or(0))
                        + self.jitter(lanes.degree.random()),
                    velocity: if accent {
                        Some(self.settings.accent)
                    } else {
                        lane_vel
                    },
                    accent,
                    jitter: self.jitter(lanes.velocity.random()),
                    ratchet,
                    gate: (i16::from(gate) + self.jitter(lanes.gate.random())).clamp(1, 100) as u8,
                    delay: 0.0,
                };
                match pick {
                    Pick::Next => self.play_next(&sorted[..n], hit, at, len),
                    Pick::All => self.chord(&sorted[..n], hit, at, len),
                    Pick::Index(i) => {
                        self.play(sorted[(usize::from(i.max(1)) - 1) % n], hit, at, len)
                    }
                    Pick::Random => {
                        let h = sorted[self.random(n)];
                        self.play(h, hit, at, len)
                    }
                    Pick::Lowest => self.play(sorted[0], hit, at, len),
                    Pick::Highest => self.play(sorted[n - 1], hit, at, len),
                }
            }
        }
    }

    /// Every note at once, strummed if the strum setting asks for it.
    fn chord(&mut self, notes: &[Held], hit: Hit, at: f64, len: f64) {
        let spread = len * f64::from(self.settings.strum) / 100.0 / notes.len() as f64;
        for (k, h) in notes.iter().enumerate() {
            self.play(
                *h,
                Hit {
                    delay: hit.delay + spread * k as f64,
                    ..hit
                },
                at,
                len,
            );
        }
    }

    /// One step of the arpeggiator's order.
    fn play_next(&mut self, sorted: &[Held], hit: Hit, at: f64, len: f64) {
        let n = sorted.len();
        let order = self.settings.order;
        if order == Order::Chord {
            self.chord(sorted, hit, at, len);
            return;
        }
        let mut played = [Held::default(); MAX_NOTES];
        // A chord or a run has no press order: As played goes up.
        let grown = self.settings.chord != Chord::Off || self.settings.run;
        let base: &[Held] = if order == Order::Played && !grown && n == self.notes.active().len() {
            played[..n].copy_from_slice(self.notes.active().as_slice());
            &played[..n]
        } else {
            sorted
        };
        let total = (n * usize::from(self.settings.octaves)).min(MAX_ORDER);
        let c = self.arp_counter as usize;
        self.arp_counter += 1;
        let i = match order.place(total, c) {
            Some(i) => i,
            None if order == Order::Random => self.random(total),
            None if order == Order::RandomOther => {
                let full = if total == 128 {
                    u128::MAX
                } else {
                    (1u128 << total) - 1
                };
                if self.bag & full == full {
                    self.bag = 0;
                }
                let free = total - (self.bag & full).count_ones() as usize;
                let mut r = self.random(free);
                let mut pick = 0;
                for p in 0..total {
                    if self.bag & (1 << p) == 0 {
                        if r == 0 {
                            pick = p;
                            break;
                        }
                        r -= 1;
                    }
                }
                self.bag |= 1 << pick;
                pick
            }
            None => {
                // Random once.
                if self.once_len != total {
                    for p in 0..total {
                        self.once[p] = p as u8;
                    }
                    for p in (1..total).rev() {
                        let q = self.random(p + 1);
                        self.once.swap(p, q);
                    }
                    self.once_len = total;
                }
                usize::from(self.once[c % total])
            }
        };
        let h = base[i % n];
        self.play(
            h,
            Hit {
                octave: hit.octave + (i / n) as i16,
                ..hit
            },
            at,
            len,
        );
    }

    /// Works out the pitch and queues the note; notes outside 0..=127, and
    /// notes the scale skips, are not played.
    fn play(&mut self, h: Held, hit: Hit, at: f64, len: f64) {
        let note = i16::from(h.note) + 12 * hit.octave;
        let note = match self.settings.scale {
            Some(scale) => match scale.fit(note, hit.degree, self.settings.snap) {
                Some(n) => n,
                None => return,
            },
            None => note + hit.degree,
        };
        if (0..=127).contains(&note) {
            let mut v = hit.velocity.unwrap_or_else(|| self.velocity_of(h.velocity));
            if !hit.accent {
                v = self.settings.ramp.apply(v, self.step);
                v = (i16::from(v) + hit.jitter).clamp(1, 127) as u8;
            }
            self.hit(
                note as u8,
                v,
                at + hit.delay,
                len - hit.delay,
                hit.ratchet,
                hit.gate,
            );
        }
    }

    /// Queues `ratchet` hits of a note across one step.
    fn hit(&mut self, note: u8, velocity: u8, at: f64, len: f64, ratchet: u8, gate: u8) {
        let r = ratchet.max(1);
        let sub = len / f64::from(r);
        let on_for = (sub * f64::from(gate) / 100.0).max(1.0);
        for k in 0..r {
            let start = at + sub * f64::from(k);
            let extra = if k + 1 == r { self.tied } else { 0.0 };
            self.queue_at(start, EventKind::On { note, velocity });
            self.queue_at(start + on_for + extra, EventKind::Off { note });
        }
    }

    /// How many tie steps follow pattern step `step` (wrapping, but never
    /// back onto the step itself).
    fn ties_after(&self, step: u64) -> usize {
        let steps = self.pattern.steps();
        let len = steps.len();
        let i = (step % len as u64) as usize;
        (1..len)
            .take_while(|k| steps[(i + k) % len] == Step::Tie)
            .count()
    }

    fn queue_at(&mut self, at: f64, kind: EventKind) {
        if self.queued < MAX_PENDING {
            self.queue[self.queued] = Pending { at, kind };
            self.queued += 1;
        } else {
            self.overflow += 1;
        }
    }

    fn drop_pending_ons(&mut self) {
        let mut i = 0;
        while i < self.queued {
            if matches!(self.queue[i].kind, EventKind::On { .. }) {
                self.queued -= 1;
                self.queue[i] = self.queue[self.queued];
            } else {
                i += 1;
            }
        }
    }

    /// Moves every pending event before `end` into `out`, in time order.
    fn drain(&mut self, end: u64, out: &mut Events) {
        let start = self.now as f64;
        loop {
            let mut best: Option<usize> = None;
            for i in 0..self.queued {
                let p = &self.queue[i];
                if p.at >= end as f64 {
                    continue;
                }
                let earlier = match best {
                    None => true,
                    Some(b) => {
                        let q = &self.queue[b];
                        let is_on = |k: &EventKind| matches!(k, EventKind::On { .. });
                        (p.at, is_on(&p.kind)) < (q.at, is_on(&q.kind))
                    }
                };
                if earlier {
                    best = Some(i);
                }
            }
            let Some(b) = best else { break };
            let p = self.queue[b];
            self.queued -= 1;
            self.queue[b] = self.queue[self.queued];
            let offset = (p.at - start).max(0.0).floor() as u32;
            out.push(Event {
                offset,
                kind: p.kind,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::BUILT_IN;

    const SR: u32 = 48_000;
    /// A sixteenth at 120 bpm and 48 kHz.
    const STEP: u64 = 6_000;

    /// Absolute (frame, event) list from running `frames` frames in blocks.
    fn run(e: &mut Engine, frames: u64, block: u32) -> Vec<(u64, EventKind)> {
        let mut out = Events::new();
        let mut got = Vec::new();
        let mut done = 0;
        while done < frames {
            let n = block.min((frames - done) as u32);
            let base = e.now;
            e.process(n, &mut out);
            got.extend(
                out.as_slice()
                    .iter()
                    .map(|ev| (base + u64::from(ev.offset), ev.kind)),
            );
            done += u64::from(n);
        }
        got
    }

    fn ons(events: &[(u64, EventKind)]) -> Vec<(u64, u8)> {
        events
            .iter()
            .filter_map(|(t, k)| match k {
                EventKind::On { note, .. } => Some((*t, *note)),
                _ => None,
            })
            .collect()
    }

    fn engine(mode: Mode) -> Engine {
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode,
            ..Settings::default()
        });
        e
    }

    #[test]
    fn rates_in_ticks() {
        let t: Vec<u32> = Rate::ALL.iter().map(|r| r.ticks()).collect();
        assert_eq!(t, [960, 480, 320, 240, 160, 120, 80]);
        let e = Engine::new(SR, 120.0);
        assert_eq!(e.step_frames(), STEP as f64);
    }

    #[test]
    fn direct_mode_passes_notes_through_and_latch_holds_them() {
        let mut e = engine(Mode::Direct);
        e.press(5, 90);
        assert_eq!(
            run(&mut e, 128, 128),
            [(
                0,
                EventKind::On {
                    note: 5,
                    velocity: 90
                }
            )]
        );
        e.release(5);
        assert_eq!(run(&mut e, 128, 128), [(128, EventKind::Off { note: 5 })]);
        e.set_latch(LatchMode::Replace);
        e.press(5, 90);
        e.release(5);
        assert_eq!(ons(&run(&mut e, 128, 128)), [(256, 5)]);
        e.press(6, 80); // a fresh press replaces the latched note
        let got = run(&mut e, 128, 128);
        assert_eq!(
            got,
            [
                (384, EventKind::Off { note: 5 }),
                (
                    384,
                    EventKind::On {
                        note: 6,
                        velocity: 80
                    }
                )
            ]
        );
        e.set_latch(LatchMode::Off); // 6 is still held, so it keeps sounding
        assert!(run(&mut e, 128, 128).is_empty());
    }

    #[test]
    fn hush_silences_a_latched_arp_and_keeps_the_latch() {
        let mut e = engine(Mode::Repeat);
        e.set_latch(LatchMode::Replace);
        e.press(3, 100);
        e.release(3);
        assert_eq!(
            ons(&run(&mut e, STEP * 2, 128)).len(),
            2,
            "latched, it repeats"
        );
        e.hush();
        let got = run(&mut e, STEP * 4, 128);
        assert!(ons(&got).is_empty(), "hushed, nothing plays");
        assert!(
            got.iter().any(|(_, k)| *k == EventKind::Off { note: 3 }),
            "its note is ended"
        );
        assert!(e.hushed() && e.active().contains(3), "the latch is kept");
        e.resume();
        assert!(!e.hushed());
        assert!(
            !ons(&run(&mut e, STEP * 2, 128)).is_empty(),
            "resumed, it repeats again"
        );
        // A press resumes too; panic clears everything.
        e.hush();
        e.press(5, 100);
        assert!(!e.hushed());
        e.hush();
        e.panic();
        assert!(!e.hushed() && e.active().is_empty());
    }

    #[test]
    fn note_repeat_retriggers_on_the_grid_with_the_gate() {
        let mut e = engine(Mode::Repeat);
        e.press(3, 100);
        let got = run(&mut e, STEP * 3, 128);
        assert_eq!(ons(&got), [(0, 3), (STEP, 3), (2 * STEP, 3)]);
        let offs: Vec<u64> = got
            .iter()
            .filter(|(_, k)| matches!(k, EventKind::Off { .. }))
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(offs, [STEP / 2, STEP + STEP / 2, 2 * STEP + STEP / 2]);
        e.release(3);
        let after = run(&mut e, STEP * 2, 128);
        assert!(ons(&after).is_empty());
    }

    #[test]
    fn repeat_follows_pad_pressure() {
        let mut e = engine(Mode::Repeat);
        e.press(3, 100);
        run(&mut e, STEP, 128);
        e.pressure(3, 30);
        let got = run(&mut e, STEP, 128);
        assert!(got.contains(&(
            STEP,
            EventKind::On {
                note: 3,
                velocity: 30
            }
        )));
    }

    #[test]
    fn timing_does_not_depend_on_the_period_size() {
        for text in [
            "x x:3 . x",
            "x x:3 . x | nudge~20 -10 10 | vel~30 | gate~40",
        ] {
            let mk = || {
                let mut e = engine(Mode::Pattern);
                e.set_pattern(Pattern::parse(text).unwrap());
                e.set_settings(Settings {
                    mode: Mode::Pattern,
                    swing: 62,
                    rate: Rate::SixteenthTriplet,
                    ..Settings::default()
                });
                e.press(1, 100);
                e.press(2, 100);
                e
            };
            let a = run(&mut mk(), 100_000, 128);
            assert!(a.len() > 40);
            for block in [1, 64, 127, 1000, 100_000] {
                assert_eq!(run(&mut mk(), 100_000, block), a, "{text:?}, block {block}");
            }
        }
    }

    #[test]
    fn swing_moves_every_second_step_later() {
        let mut e = engine(Mode::Repeat);
        e.set_settings(Settings {
            mode: Mode::Repeat,
            swing: 66,
            ..Settings::default()
        });
        e.press(1, 100);
        let delay = STEP * 16 / 50; // (66 - 50) / 50 of a step
        assert_eq!(
            ons(&run(&mut e, STEP * 4, 128)),
            [
                (0, 1),
                (STEP + delay, 1),
                (2 * STEP, 1),
                (3 * STEP + delay, 1)
            ]
        );
    }

    #[test]
    fn order_places_repeat_after_their_cycle() {
        for order in Order::ALL {
            for total in 1..=12 {
                match order.cycle(total) {
                    Some(n) => {
                        for c in 0..n {
                            let p = order.place(total, c).unwrap();
                            assert!(p < total, "{order:?} {total} {c}");
                            assert_eq!(order.place(total, c + n), Some(p), "{order:?} {total} {c}");
                        }
                    }
                    None => assert_eq!(order.place(total, 0), None, "{order:?}"),
                }
            }
            assert_eq!(order.place(0, 0), None);
        }
    }

    fn arp_notes(order: Order, octaves: u8, steps: u64) -> Vec<u8> {
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode: Mode::Arp,
            order,
            octaves,
            ..Settings::default()
        });
        for n in [64, 60, 67] {
            e.press(n, 100);
        }
        ons(&run(&mut e, STEP * steps, 128))
            .into_iter()
            .map(|(_, n)| n)
            .collect()
    }

    #[test]
    fn arpeggiator_orders() {
        assert_eq!(arp_notes(Order::Up, 1, 4), [60, 64, 67, 60]);
        assert_eq!(arp_notes(Order::Down, 1, 4), [67, 64, 60, 67]);
        assert_eq!(arp_notes(Order::UpDown, 1, 6), [60, 64, 67, 64, 60, 64]);
        assert_eq!(
            arp_notes(Order::UpDownInclusive, 1, 7),
            [60, 64, 67, 67, 64, 60, 60]
        );
        assert_eq!(arp_notes(Order::Played, 1, 4), [64, 60, 67, 64]);
        assert_eq!(arp_notes(Order::Converge, 1, 4), [67, 60, 64, 67]);
        assert_eq!(arp_notes(Order::Diverge, 1, 4), [64, 60, 67, 64]);
        assert_eq!(arp_notes(Order::Converge, 2, 6), [79, 60, 76, 64, 72, 67]);
        assert_eq!(arp_notes(Order::Up, 2, 7), [60, 64, 67, 72, 76, 79, 60]);
        assert_eq!(arp_notes(Order::Chord, 1, 2), [60, 64, 67, 60, 64, 67]);
        let r = arp_notes(Order::Random, 2, 64);
        assert!(r.iter().all(|n| [60, 64, 67, 72, 76, 79].contains(n)));
        assert!(r.contains(&60) && r.contains(&79));
        assert_eq!(arp_notes(Order::Random, 2, 64), r, "same seed, same notes");
    }

    #[test]
    fn notes_out_of_range_are_skipped() {
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode: Mode::Arp,
            octaves: 3,
            ..Settings::default()
        });
        e.press(110, 100);
        let n: Vec<u8> = ons(&run(&mut e, STEP * 3, 128))
            .into_iter()
            .map(|(_, n)| n)
            .collect();
        assert_eq!(n, [110, 122]); // 134 is past 127: that step is silent
    }

    #[test]
    fn latch_keeps_the_arpeggio_going_after_release() {
        let mut e = engine(Mode::Arp);
        e.set_latch(LatchMode::Replace);
        e.press(60, 100);
        e.press(64, 100);
        e.release(60);
        e.release(64);
        assert_eq!(ons(&run(&mut e, STEP * 3, 128)).len(), 3);
        e.clear_latched();
        let got = run(&mut e, STEP * 3, 128);
        assert!(ons(&got).is_empty(), "{got:?}");
    }

    #[test]
    fn patterns_rest_tie_ratchet_and_accent() {
        let mut e = engine(Mode::Pattern);
        e.set_pattern(Pattern::parse("1! _ . 2:2").unwrap());
        e.press(60, 90);
        e.press(64, 90);
        let got = run(&mut e, STEP * 4, 128);
        assert_eq!(
            got,
            [
                (
                    0,
                    EventKind::On {
                        note: 60,
                        velocity: 127
                    }
                ),
                // gate 50% plus one tied step
                (STEP + STEP / 2, EventKind::Off { note: 60 }),
                (
                    3 * STEP,
                    EventKind::On {
                        note: 64,
                        velocity: 90
                    }
                ),
                (3 * STEP + STEP / 4, EventKind::Off { note: 64 }),
                (
                    3 * STEP + STEP / 2,
                    EventKind::On {
                        note: 64,
                        velocity: 90
                    }
                ),
                (3 * STEP + 3 * STEP / 4, EventKind::Off { note: 64 }),
            ]
        );
    }

    #[test]
    fn every_built_in_pattern_plays() {
        for (name, text) in BUILT_IN {
            let mut e = engine(Mode::Pattern);
            e.set_pattern(Pattern::parse(text).unwrap());
            for n in [60, 63, 67] {
                e.press(n, 100);
            }
            let got = run(&mut e, STEP * 16, 128);
            assert!(!ons(&got).is_empty(), "{name}");
            for n in 0..=127u8 {
                let on = got
                    .iter()
                    .filter(|(_, k)| matches!(k, EventKind::On { note, .. } if *note == n))
                    .count();
                let off = got
                    .iter()
                    .filter(|(_, k)| *k == EventKind::Off { note: n })
                    .count();
                assert!(
                    off <= on && on <= off + 1,
                    "{name}: note {n} has {on} ons and {off} offs"
                );
            }
        }
    }

    #[test]
    fn changing_mode_stops_direct_notes_and_releasing_stops_the_generator() {
        let mut e = engine(Mode::Direct);
        e.press(9, 100);
        run(&mut e, 128, 128);
        e.set_settings(Settings {
            mode: Mode::Repeat,
            ..Settings::default()
        });
        let got = run(&mut e, 128, 128);
        assert_eq!(got[0], (128, EventKind::Off { note: 9 }));
        assert_eq!(
            got[1],
            (
                128,
                EventKind::On {
                    note: 9,
                    velocity: 100
                }
            )
        );
        e.release(9);
        let got = run(&mut e, STEP * 2, 128);
        assert_eq!(got, [(128 + STEP / 2, EventKind::Off { note: 9 })]);
    }

    #[test]
    fn panic_stops_everything() {
        let mut e = engine(Mode::Repeat);
        e.set_latch(LatchMode::Replace);
        e.press(1, 100);
        run(&mut e, 128, 128);
        e.panic();
        let got = run(&mut e, STEP * 2, 128);
        assert_eq!(got.len(), 128);
        assert!(got
            .iter()
            .all(|(t, k)| *t == 128 && matches!(k, EventKind::Off { .. })));
        assert!(e.active().is_empty());
    }

    #[test]
    fn too_many_events_are_counted_not_lost_silently() {
        let mut e = engine(Mode::Repeat);
        e.set_settings(Settings {
            mode: Mode::Repeat,
            rate: Rate::ThirtySecondTriplet,
            ..Settings::default()
        });
        for n in 0..32 {
            e.press(n, 100);
        }
        let mut out = Events::new();
        e.process(48_000, &mut out);
        assert_eq!(out.as_slice().len(), MAX_EVENTS);
        assert!(out.dropped > 0 || e.overflow > 0);
    }

    fn notes_of(e: &mut Engine, steps: u64) -> Vec<u8> {
        ons(&run(e, STEP * steps, 128))
            .into_iter()
            .map(|(_, n)| n)
            .collect()
    }

    #[test]
    fn random_without_repeats_plays_every_note_once_per_round() {
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode: Mode::Arp,
            order: Order::RandomOther,
            octaves: 2,
            ..Settings::default()
        });
        for n in [60, 64, 67] {
            e.press(n, 100);
        }
        let got = notes_of(&mut e, 18);
        for round in got.chunks(6) {
            let mut r = round.to_vec();
            r.sort();
            assert_eq!(r, [60, 64, 67, 72, 76, 79], "{got:?}");
        }
    }

    #[test]
    fn random_once_repeats_one_order_until_the_notes_change() {
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode: Mode::Arp,
            order: Order::RandomOnce,
            ..Settings::default()
        });
        e.set_seed(99);
        for n in [60, 62, 64, 65, 67] {
            e.press(n, 100);
        }
        let got = notes_of(&mut e, 15);
        assert_eq!(got[..5], got[5..10]);
        assert_eq!(got[..5], got[10..]);
        let mut sorted = got[..5].to_vec();
        sorted.sort();
        assert_eq!(sorted, [60, 62, 64, 65, 67]);
    }

    #[test]
    fn lanes_loop_at_their_own_length() {
        let mut e = engine(Mode::Pattern);
        e.set_pattern(Pattern::parse("1 1 1 1 | oct 0 1 0 | vel 100 50").unwrap());
        e.press(60, 90);
        let got = run(&mut e, STEP * 6, 128);
        let on: Vec<(u8, u8)> = got
            .iter()
            .filter_map(|(_, k)| match k {
                EventKind::On { note, velocity } => Some((*note, *velocity)),
                _ => None,
            })
            .collect();
        assert_eq!(
            on,
            [
                (60, 100),
                (72, 50),
                (60, 100),
                (60, 50),
                (72, 100),
                (60, 50)
            ]
        );
    }

    #[test]
    fn scales_apply_after_offsets() {
        let mut e = engine(Mode::Pattern);
        e.set_settings(Settings {
            mode: Mode::Pattern,
            scale: Some(Scale::new(0, crate::scale::MAJOR)),
            ..Settings::default()
        });
        e.set_pattern(Pattern::parse("1 1 1 1 | deg 0 2 4 7").unwrap());
        e.press(61, 100); // C#: snaps to C
        assert_eq!(notes_of(&mut e, 4), [60, 64, 67, 72]);
        let mut e = engine(Mode::Pattern);
        e.set_pattern(Pattern::parse("1 1 | deg 0 3").unwrap());
        e.press(61, 100); // no scale: semitones
        assert_eq!(notes_of(&mut e, 2), [61, 64]);
    }

    #[test]
    fn one_pad_plays_a_chord_from_the_scale_for_the_arp() {
        let arp = |chord: Chord, scale: Option<Scale>, held: &[u8], steps: u64| {
            let mut e = engine(Mode::Arp);
            e.set_settings(Settings {
                mode: Mode::Arp,
                scale,
                chord,
                ..Settings::default()
            });
            for &n in held {
                e.press(n, 100);
            }
            notes_of(&mut e, steps)
        };
        let c_major = Some(Scale::new(0, crate::scale::MAJOR));
        // D in C major: D F A (a minor triad, from the scale).
        assert_eq!(arp(Chord::Triad, c_major, &[62], 4), [62, 65, 69, 62]);
        assert_eq!(
            arp(Chord::Seventh, c_major, &[67], 4),
            [67, 71, 74, 77],
            "G7 in C"
        );
        assert_eq!(arp(Chord::Sus4, c_major, &[60], 3), [60, 65, 67]);
        // Without a scale: built from the major scale on the held note.
        assert_eq!(arp(Chord::Triad, None, &[62], 3), [62, 66, 69]);
        // Two pads: both chords, shared notes once, lowest first.
        assert_eq!(arp(Chord::Power, c_major, &[60, 64], 4), [60, 64, 67, 71]);
        // Repeat mode plays the whole chord on every step.
        let mut e = engine(Mode::Repeat);
        e.set_settings(Settings {
            mode: Mode::Repeat,
            scale: c_major,
            chord: Chord::Triad,
            ..Settings::default()
        });
        e.press(57, 100);
        assert_eq!(notes_of(&mut e, 1), [57, 60, 64], "A minor in C major");
    }

    #[test]
    fn held_chords_take_turns_for_a_progression() {
        let a_minor = Some(Scale::new(9, crate::scale::MINOR));
        let mut e = engine(Mode::Arp);
        e.set_settings(Settings {
            mode: Mode::Arp,
            scale: a_minor,
            chord: Chord::Triad,
            turns: Turns::Beat,
            ..Settings::default()
        });
        // Pressed Am, F, C, G: a beat (4 sixteenths) of each, in that order,
        // each starting again from its root.
        for n in [57, 53, 60, 55] {
            e.press(n, 100);
        }
        assert_eq!(
            notes_of(&mut e, 20),
            [57, 60, 64, 57, 53, 57, 60, 53, 60, 64, 67, 60, 55, 59, 62, 55, 57, 60, 64, 57]
        );
        // Without a chord, the held notes themselves take turns, even As
        // played.
        let mut e = engine(Mode::Arp);
        e.set_settings(Settings {
            mode: Mode::Arp,
            order: Order::Played,
            turns: Turns::Beat,
            octaves: 2,
            ..Settings::default()
        });
        for n in [64, 60] {
            e.press(n, 100);
        }
        assert_eq!(notes_of(&mut e, 8), [64, 76, 64, 76, 60, 72, 60, 72]);
        // Repeat mode: the whole chord on each step, a chord per turn.
        let mut e = engine(Mode::Repeat);
        e.set_settings(Settings {
            mode: Mode::Repeat,
            scale: a_minor,
            chord: Chord::Triad,
            turns: Turns::Beat,
            ..Settings::default()
        });
        for n in [57, 53] {
            e.press(n, 100);
        }
        let got = notes_of(&mut e, 5);
        assert_eq!(
            (&got[..3], &got[12..15]),
            (&[57, 60, 64][..], &[53, 57, 60][..])
        );
        // The length of a turn follows the rate; one note held, or Direct
        // mode, has no turns.
        let s = Settings {
            mode: Mode::Arp,
            turns: Turns::Bar,
            rate: Rate::Eighth,
            ..Settings::default()
        };
        assert_eq!(turn_steps(&s), 8);
        assert_eq!(
            turn_steps(&Settings {
                turns: Turns::Off,
                ..s
            }),
            0
        );
        assert_eq!(turn_of(&s, 3, 17), Some(2));
        assert_eq!(turn_of(&s, 1, 17), None);
        assert_eq!(
            turn_of(
                &Settings {
                    mode: Mode::Direct,
                    ..s
                },
                3,
                17
            ),
            None
        );
    }

    #[test]
    fn a_scale_run_fills_in_between_the_held_notes() {
        let run = |order: Order, scale: Option<Scale>, held: &[u8], steps: u64| {
            let mut e = engine(Mode::Arp);
            e.set_settings(Settings {
                mode: Mode::Arp,
                order,
                scale,
                run: true,
                ..Settings::default()
            });
            for &n in held {
                e.press(n, 100);
            }
            notes_of(&mut e, steps)
        };
        let a_minor_pent = Some(Scale::new(9, crate::scale::PENTATONIC_MINOR));
        // From A3 to A4 held: the pentatonic notes between, up.
        assert_eq!(
            run(Order::Up, a_minor_pent, &[57, 69], 7),
            [57, 60, 62, 64, 67, 69, 57]
        );
        assert_eq!(run(Order::Down, a_minor_pent, &[57, 69], 3), [69, 67, 64]);
        // One note: an octave of the scale from it.
        let d_dorian = Some(Scale::new(2, crate::scale::DORIAN));
        assert_eq!(
            run(Order::Up, d_dorian, &[62], 8),
            [62, 64, 65, 67, 69, 71, 72, 62]
        );
        // No scale: every semitone (on drum pads, every pad between).
        assert_eq!(run(Order::Up, None, &[0, 3], 5), [0, 1, 2, 3, 0]);
        // Repeat mode is left alone: it plays the held notes.
        let mut e = engine(Mode::Repeat);
        e.set_settings(Settings {
            mode: Mode::Repeat,
            run: true,
            ..Settings::default()
        });
        e.press(0, 100);
        e.press(3, 100);
        assert_eq!(notes_of(&mut e, 1), [0, 3]);
        // A chord, then a run over it: one pad, the scale from root to seventh.
        let mut s = [Held::default(); MAX_NOTES];
        s[0] = Held {
            note: 60,
            velocity: 90,
        };
        let set = Settings {
            mode: Mode::Arp,
            scale: Some(Scale::new(0, crate::scale::MAJOR)),
            chord: Chord::Seventh,
            run: true,
            ..Settings::default()
        };
        let n = step_notes(&set, &mut s, 1);
        assert_eq!(
            s[..n].iter().map(|h| h.note).collect::<Vec<_>>(),
            [60, 62, 64, 65, 67, 69, 71]
        );
        assert!(s[..n].iter().all(|h| h.velocity == 90));
    }

    #[test]
    fn out_of_key_notes_go_up_to_the_nearest_or_are_skipped() {
        let run_with = |snap: Snap, held: &[u8]| {
            let mut e = engine(Mode::Arp);
            e.set_settings(Settings {
                mode: Mode::Arp,
                scale: Some(Scale::new(0, crate::scale::MAJOR)),
                snap,
                ..Settings::default()
            });
            for &n in held {
                e.press(n, 100);
            }
            notes_of(&mut e, 4)
        };
        // C, D# and G held in C major, played up.
        assert_eq!(run_with(Snap::Down, &[60, 63, 67]), [60, 62, 67, 60]);
        assert_eq!(run_with(Snap::Up, &[60, 63, 67]), [60, 64, 67, 60]);
        assert_eq!(
            run_with(Snap::Nearest, &[60, 63, 66, 67]),
            [60, 62, 65, 67],
            "D# and F# are halfway: down"
        );
        // Skip: D# is not played, so its step is silent.
        let mut e = engine(Mode::Arp);
        e.set_settings(Settings {
            mode: Mode::Arp,
            scale: Some(Scale::new(0, crate::scale::MAJOR)),
            snap: Snap::Skip,
            ..Settings::default()
        });
        for n in [60, 63, 67] {
            e.press(n, 100);
        }
        let got: Vec<u8> = ons(&run(&mut e, 3 * STEP, 128))
            .into_iter()
            .map(|(_, n)| n)
            .collect();
        assert_eq!(got, [60, 67]);
    }

    #[test]
    fn strum_spreads_a_chord_across_the_step() {
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode: Mode::Arp,
            order: Order::Chord,
            strum: 60,
            ..Settings::default()
        });
        for n in [60, 64, 67] {
            e.press(n, 100);
        }
        let got = ons(&run(&mut e, STEP, 128));
        let gap = STEP * 60 / 100 / 3;
        assert_eq!(got, [(0, 60), (gap, 64), (2 * gap, 67)]);
    }

    #[test]
    fn retrigger_restarts_the_order_and_repeats_stop_after_n_steps() {
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode: Mode::Arp,
            retrigger: true,
            ..Settings::default()
        });
        e.press(60, 100);
        e.press(64, 100);
        e.press(67, 100);
        assert_eq!(notes_of(&mut e, 2), [60, 64]);
        e.press(55, 100); // restarts from the lowest
        assert_eq!(notes_of(&mut e, 2), [55, 60]);

        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode: Mode::Arp,
            repeats: 3,
            ..Settings::default()
        });
        e.press(60, 100);
        e.press(64, 100);
        assert_eq!(notes_of(&mut e, 6), [60, 64, 60]);
    }

    #[test]
    fn settings_are_clamped() {
        let s = Settings {
            octaves: 9,
            gate: 0,
            swing: 99,
            accent: 0,
            velocity: Velocity::Fixed(0),
            strum: 200,
            ..Settings::default()
        }
        .clamped();
        assert_eq!(s.strum, 100);
        assert_eq!(
            (s.octaves, s.gate, s.swing, s.accent, s.velocity),
            (4, 1, 75, 1, Velocity::Fixed(1))
        );
    }
    #[test]
    fn nudge_moves_single_steps_and_never_reorders_them() {
        let mut e = engine(Mode::Pattern);
        e.set_pattern(Pattern::parse("x x x x x | nudge 0 20 0 -10").unwrap());
        e.press(60, 100);
        let n = STEP / 10;
        let got: Vec<u64> = ons(&run(&mut e, STEP * 5, 128))
            .iter()
            .map(|&(t, _)| t)
            .collect();
        assert_eq!(got, [0, STEP + 2 * n, 2 * STEP, 3 * STEP - n, 4 * STEP]);

        // Swing 75 and a late nudge on step 1, an early one on step 2: step 2
        // waits for step 1 instead of jumping ahead of it.
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode: Mode::Pattern,
            swing: 75,
            ..Settings::default()
        });
        e.set_pattern(Pattern::parse("x x x | nudge 0 50 -50").unwrap());
        e.press(60, 100);
        let got: Vec<u64> = ons(&run(&mut e, STEP * 3, 128))
            .iter()
            .map(|&(t, _)| t)
            .collect();
        assert_eq!(got, [0, 2 * STEP, 2 * STEP]);

        // Other modes have no lanes, so no nudge.
        let mut e = engine(Mode::Repeat);
        e.set_pattern(Pattern::parse("x | nudge 30").unwrap());
        e.press(60, 100);
        assert_eq!(ons(&run(&mut e, STEP * 2, 128)), [(0, 60), (STEP, 60)]);
    }

    fn velocities(e: &mut Engine, frames: u64) -> Vec<u8> {
        run(e, frames, 128)
            .iter()
            .filter_map(|(_, k)| match k {
                EventKind::On { velocity, .. } => Some(*velocity),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn lane_random_amounts_stay_in_range_and_repeat_with_the_seed() {
        let mk = |seed| {
            let mut e = engine(Mode::Pattern);
            e.set_seed(seed);
            e.set_pattern(Pattern::parse("x x x x! | vel~20 80 | oct~1 | gate~30 50").unwrap());
            e.press(60, 100);
            e
        };
        let a = run(&mut mk(3), STEP * 64, 128);
        assert_eq!(run(&mut mk(3), STEP * 64, 128), a, "same seed, same notes");
        assert_ne!(run(&mut mk(4), STEP * 64, 128), a, "another seed varies");
        let mut seen_vel = std::collections::BTreeSet::new();
        let mut seen_note = std::collections::BTreeSet::new();
        let mut on_at = [0u64; 128];
        for (i, (t, k)) in a.iter().enumerate() {
            match *k {
                EventKind::On { note, velocity } => {
                    seen_note.insert(note);
                    on_at[usize::from(note)] = *t;
                    if (i / 2) % 4 == 3 {
                        assert_eq!(velocity, 127, "accents are left alone");
                    } else {
                        assert!((60..=100).contains(&velocity), "{velocity}");
                        seen_vel.insert(velocity);
                    }
                }
                EventKind::Off { note } => {
                    let len = *t - on_at[usize::from(note)];
                    assert!(
                        (STEP / 5..=STEP * 4 / 5).contains(&len),
                        "gate 50 +/- 30: {len}"
                    );
                }
            }
        }
        assert!(seen_vel.len() > 10, "{seen_vel:?}");
        assert_eq!(seen_note.into_iter().collect::<Vec<_>>(), [48, 60, 72]);

        // A plain pattern draws no random numbers, so old patterns sound the same.
        let mut e = engine(Mode::Pattern);
        e.set_pattern(Pattern::parse("x ?").unwrap());
        e.press(60, 100);
        e.press(64, 100);
        let mut f = engine(Mode::Pattern);
        f.set_pattern(Pattern::parse("x ? | vel 0 | nudge 0").unwrap());
        f.press(60, 100);
        f.press(64, 100);
        assert_eq!(run(&mut e, STEP * 32, 128), run(&mut f, STEP * 32, 128));
    }

    #[test]
    fn velocity_ramps_to_its_target_and_stays() {
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode: Mode::Repeat,
            velocity: Velocity::Fixed(100),
            ramp: Ramp {
                target: 20,
                steps: 4,
            },
            ..Settings::default()
        });
        e.press(60, 90);
        assert_eq!(velocities(&mut e, STEP * 6), [100, 80, 60, 40, 20, 20]);
        assert_eq!(Ramp::OFF.apply(55, 1000), 55);
        assert_eq!(
            Ramp {
                target: 127,
                steps: 2
            }
            .apply(27, 1),
            77
        );
        let s = Settings {
            ramp: Ramp {
                target: 0,
                steps: 8,
            },
            ..Settings::default()
        }
        .clamped();
        assert_eq!(s.ramp.target, 1);
    }
    #[test]
    fn position_counts_steps_since_the_start() {
        let mut e = engine(Mode::Repeat);
        assert_eq!(e.position(), 0.0);
        e.press(60, 100);
        run(&mut e, STEP * 5 / 2, 100);
        assert_eq!(e.position(), 2.5);
        run(&mut e, STEP / 2, 100);
        assert_eq!(e.position(), 3.0);
        e.release(60);
        assert_eq!(e.position(), 0.0);
    }

    /// A stretch of transport time: from engine frame `from`, tick
    /// `tick` is on frame `from` and the tempo is `bpm`.
    #[derive(Clone, Copy)]
    struct Span {
        from: u64,
        tick: i64,
        bpm: f64,
        playing: bool,
        jumped: bool,
        swing: Option<(u32, u8)>,
    }

    fn span(from: u64, tick: i64, bpm: f64) -> Span {
        Span {
            from,
            tick,
            bpm,
            playing: true,
            jumped: false,
            swing: None,
        }
    }

    /// Runs like a host behind a transport: periods of `block` frames are
    /// cut where a span starts, and each piece gets its own `sync`.
    /// `press` events are applied at their frames (which must be period
    /// boundaries in every block size used).
    fn run_synced(
        e: &mut Engine,
        spans: &[Span],
        press: &[(u64, u8)],
        frames: u64,
        block: u32,
    ) -> Vec<(u64, EventKind)> {
        let mut out = Events::new();
        let mut got = Vec::new();
        while e.now < frames {
            let now = e.now;
            for &(at, note) in press {
                if at == now {
                    e.press(note, 100);
                }
            }
            let s = *spans.iter().rev().find(|s| s.from <= now).unwrap();
            let mut n = u64::from(block).min(frames - now);
            if let Some(next) = spans.iter().find(|s| s.from > now) {
                n = n.min(next.from - now);
            }
            if let Some(&(at, _)) = press.iter().find(|(at, _)| *at > now) {
                n = n.min(at - now);
            }
            e.sync(&Clock {
                anchor_tick: s.tick,
                frames_since_anchor: now - s.from,
                bpm: s.bpm,
                playing: s.playing,
                jumped: s.jumped && now == s.from,
                swing: s.swing,
                bar_ticks: 4 * PPQ,
            });
            e.process(n as u32, &mut out);
            got.extend(
                out.as_slice()
                    .iter()
                    .map(|ev| (now + u64::from(ev.offset), ev.kind)),
            );
        }
        got
    }

    #[test]
    fn synced_steps_wait_for_the_transport_grid() {
        // The transport is at tick 0 on frame 0, 120 bpm (a 1/16 is STEP).
        let spans = [span(0, 0, 120.0)];
        let mut e = engine(Mode::Repeat);
        let got = run_synced(&mut e, &spans, &[(1000, 1)], STEP * 4, 128);
        assert!(e.synced());
        assert_eq!(ons(&got), [(STEP, 1), (2 * STEP, 1), (3 * STEP, 1)]);
    }

    #[test]
    fn synced_start_can_wait_for_a_beat_or_a_bar() {
        for (start, first) in [(SyncStart::Beat, 4 * STEP), (SyncStart::Bar, 16 * STEP)] {
            let mut e = Engine::new(SR, 120.0);
            e.set_settings(Settings {
                mode: Mode::Repeat,
                start,
                ..Settings::default()
            });
            let got = run_synced(
                &mut e,
                &[span(0, 0, 120.0)],
                &[(1000, 1)],
                17 * STEP + 1,
                128,
            );
            assert_eq!(ons(&got)[0], (first, 1), "{start:?}");
            assert!(e.position() > 0.0);
        }
        // A press exactly on the line starts there.
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode: Mode::Repeat,
            start: SyncStart::Bar,
            ..Settings::default()
        });
        let got = run_synced(
            &mut e,
            &[span(0, 0, 120.0)],
            &[(16 * STEP, 1)],
            17 * STEP,
            128,
        );
        assert_eq!(ons(&got), [(16 * STEP, 1)]);
    }

    #[test]
    fn synced_steps_follow_tempo_changes_and_count_ins() {
        // Count-in from tick -960, then 120 bpm until tick 960 (frame 48000),
        // then 60 bpm: a 1/16 takes twice as long.
        let spans = [span(0, -960, 120.0), span(48_000, 960, 60.0)];
        let mut e = engine(Mode::Repeat);
        let got = run_synced(&mut e, &spans, &[(0, 1)], 48_000 + 3 * 12_000, 128);
        let t: Vec<u64> = ons(&got).iter().map(|(t, _)| *t).collect();
        let mut want: Vec<u64> = (0..8).map(|k| k * STEP).collect();
        want.extend((0..3).map(|k| 48_000 + k * 12_000));
        assert_eq!(t, want);
    }

    #[test]
    fn synced_timing_does_not_depend_on_the_period_size() {
        let mut spans = [
            span(0, 100, 120.0),
            span(30_011, 1920, 97.0),
            span(70_000, 0, 97.0),
        ];
        spans[1].swing = Some((240, 60));
        spans[2].jumped = true;
        let mk = || {
            let mut e = engine(Mode::Pattern);
            e.set_pattern(
                Pattern::parse("x x:3 . x | nudge~20 -10 10 | vel~30 | gate~40").unwrap(),
            );
            e.set_settings(Settings {
                mode: Mode::Pattern,
                swing: 62,
                rate: Rate::SixteenthTriplet,
                ..Settings::default()
            });
            e
        };
        let press = [(0, 1), (0, 2)];
        let a = run_synced(&mut mk(), &spans, &press, 100_000, 128);
        assert!(a.len() > 40);
        for block in [1, 64, 127, 1000, 100_000] {
            assert_eq!(
                run_synced(&mut mk(), &spans, &press, 100_000, block),
                a,
                "block {block}"
            );
        }
    }

    #[test]
    fn a_jump_puts_the_next_step_back_on_the_grid() {
        // Loop back to tick 0 at frame 10_000, between two steps.
        let mut spans = [span(0, 0, 120.0), span(10_000, 0, 120.0)];
        spans[1].jumped = true;
        let mut e = engine(Mode::Repeat);
        let got = run_synced(&mut e, &spans, &[(0, 1)], 10_000 + 2 * STEP, 128);
        assert_eq!(
            ons(&got),
            [(0, 1), (STEP, 1), (10_000, 1), (10_000 + STEP, 1)]
        );
    }

    #[test]
    fn transport_swing_bends_the_grid() {
        // 1/16 swing at 66 with the arp at 1/32: the odd 1/16 lines move
        // 16/50 of a 1/16 later, and the 1/32s between them are spread out.
        let mut s = span(0, 0, 120.0);
        s.swing = Some((240, 66));
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode: Mode::Repeat,
            rate: Rate::ThirtySecond,
            ..Settings::default()
        });
        let got = run_synced(&mut e, &[s], &[(0, 1)], 2 * STEP, 128);
        let t: Vec<u64> = ons(&got).iter().map(|(t, _)| *t).collect();
        let delay = STEP * 16 / 50;
        assert_eq!(
            t,
            [
                0,
                (STEP / 2) * 66 / 50,
                STEP + delay,
                STEP + delay + (STEP / 2) * 34 / 50
            ]
        );
        // Without transport swing, the arp's own swing works on the transport
        // grid as it does free-running.
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings {
            mode: Mode::Repeat,
            swing: 66,
            ..Settings::default()
        });
        let got = run_synced(&mut e, &[span(0, 0, 120.0)], &[(0, 1)], 4 * STEP, 128);
        assert_eq!(
            ons(&got),
            [
                (0, 1),
                (STEP + delay, 1),
                (2 * STEP, 1),
                (3 * STEP + delay, 1)
            ]
        );
    }

    #[test]
    fn stopping_the_transport_goes_back_to_free_running() {
        let mut stop = span(2 * STEP + 100, 0, 120.0);
        stop.playing = false;
        let mut e = engine(Mode::Repeat);
        let got = run_synced(&mut e, &[span(0, 0, 120.0), stop], &[(0, 1)], 5 * STEP, 128);
        assert!(!e.synced());
        let t: Vec<u64> = ons(&got).iter().map(|(t, _)| *t).collect();
        assert_eq!(t, [0, STEP, 2 * STEP, 3 * STEP, 4 * STEP]);
    }

    #[test]
    fn changing_the_rate_while_synced_snaps_to_the_new_grid() {
        let mut e = engine(Mode::Repeat);
        let spans = [span(0, 0, 120.0)];
        let got = run_synced(&mut e, &spans, &[(0, 1)], STEP + 1000, 128);
        assert_eq!(ons(&got), [(0, 1), (STEP, 1)]);
        e.set_settings(Settings {
            rate: Rate::Quarter,
            ..e.settings()
        });
        let mut out = Events::new();
        let mut got = Vec::new();
        while e.now < 9 * STEP {
            let now = e.now;
            e.sync(&Clock::playing(0, now, 120.0));
            e.process(128, &mut out);
            got.extend(
                out.as_slice()
                    .iter()
                    .map(|ev| (now + u64::from(ev.offset), ev.kind)),
            );
        }
        assert_eq!(ons(&got), [(4 * STEP, 1), (8 * STEP, 1)]);
    }
}
