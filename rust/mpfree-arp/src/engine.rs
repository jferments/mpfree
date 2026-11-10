//! The engine: notes in (pads, screen, MIDI), sample-timed notes out.
//!
//! It runs inside the audio thread. The app calls [`Engine::press`],
//! [`Engine::release`] and the setters between audio periods, then
//! [`Engine::process`] once per period, which returns the note-ons and
//! note-offs that fall inside that period with their frame offsets. Nothing
//! allocates, locks or does I/O after [`Engine::new`].

use crate::notes::{Held, LatchMode, NoteSet, Notes, MAX_NOTES};
use crate::pattern::{Pattern, Pick, Step};

/// Clock resolution: ticks per quarter note.
pub const PPQ: u32 = 960;
/// Events one `process` call can return; more are dropped and counted.
pub const MAX_EVENTS: usize = 128;
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
    /// All notes together on every step.
    Chord,
}

impl Order {
    pub const ALL: [Order; 9] = [
        Order::Up,
        Order::Down,
        Order::UpDown,
        Order::UpDownInclusive,
        Order::Converge,
        Order::Diverge,
        Order::Played,
        Order::Random,
        Order::Chord,
    ];

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
            Order::Chord => "Chord",
        }
    }
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
        Self { buf: [Event { offset: 0, kind: EventKind::Off { note: 0 } }; MAX_EVENTS], len: 0, dropped: 0 }
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
    /// Notes that could not be queued, since the engine started.
    pub overflow: u64,
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
            next_step: 0.0,
            step: 0,
            arp_counter: 0,
            queue: [Pending { at: 0.0, kind: EventKind::Off { note: 0 } }; MAX_PENDING],
            queued: 0,
            tied: 0.0,
            rng: 0x2545_f491,
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

    pub fn set_seed(&mut self, seed: u32) {
        self.rng = seed.max(1);
    }

    pub fn set_tempo(&mut self, bpm: f64) {
        self.bpm = bpm.clamp(20.0, 300.0);
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
        self.settings = settings;
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

    /// Stops everything at the start of the next period: all notes off,
    /// nothing held or latched.
    pub fn panic(&mut self) {
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
        if self.running {
            loop {
                let len = self.step_frames();
                let fire = self.next_step + self.swing_delay(len);
                if fire >= end as f64 {
                    break;
                }
                self.fire_step(fire, len);
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

    /// Every second step moves later by (swing - 50) / 50 of a step.
    fn swing_delay(&self, len: f64) -> f64 {
        if self.step % 2 == 1 {
            len * f64::from(self.settings.swing - 50) / 50.0
        } else {
            0.0
        }
    }

    fn after_change(&mut self, before: &NoteSet) {
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
                self.queue_at(at, EventKind::On { note: h.note, velocity: self.velocity_of(h.velocity) });
            }
        }
        self.update_running();
    }

    fn update_running(&mut self) {
        let want = self.settings.mode != Mode::Direct && !self.notes.active().is_empty();
        if want && !self.running {
            // Starts on the press, not on a bar line: there is no transport yet.
            self.running = true;
            self.next_step = self.now as f64;
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
        let n = self.notes.active().sorted(&mut sorted);
        if n == 0 {
            return;
        }
        let gate = self.settings.gate;
        self.tied = 0.0;
        match self.settings.mode {
            Mode::Direct => {}
            Mode::Repeat => {
                for h in &sorted[..n] {
                    self.hit(h.note, self.velocity_of(h.velocity), at, len, 1, gate);
                }
            }
            Mode::Arp => {
                self.play_next(&sorted[..n], 0, None, at, len, 1, gate);
            }
            Mode::Pattern => {
                let step = self.pattern.steps()[(self.step % self.pattern.len() as u64) as usize];
                match step {
                    // A tie was already added to the notes of the step before
                    // it, when they were scheduled.
                    Step::Rest | Step::Tie => {}
                    Step::Play { pick, octave, accent, gate: g, ratchet, chance } => {
                        self.tied = self.ties_after(self.step) as f64 * len;
                        if chance < 100 && self.random(100) >= usize::from(chance) {
                            return;
                        }
                        let gate = if g == 0 { gate } else { g };
                        let vel = if accent { Some(self.settings.accent) } else { None };
                        let shift = i16::from(octave) * 12;
                        match pick {
                            Pick::Next => self.play_next(&sorted[..n], shift, vel, at, len, ratchet, gate),
                            Pick::All => {
                                for h in &sorted[..n] {
                                    self.shifted_hit(*h, shift, vel, at, len, ratchet, gate);
                                }
                            }
                            Pick::Index(i) => {
                                let h = sorted[(usize::from(i.max(1)) - 1) % n];
                                self.shifted_hit(h, shift, vel, at, len, ratchet, gate);
                            }
                            Pick::Random => {
                                let h = sorted[self.random(n)];
                                self.shifted_hit(h, shift, vel, at, len, ratchet, gate);
                            }
                            Pick::Lowest => self.shifted_hit(sorted[0], shift, vel, at, len, ratchet, gate),
                            Pick::Highest => self.shifted_hit(sorted[n - 1], shift, vel, at, len, ratchet, gate),
                        }
                    }
                }
            }
        }
    }

    /// One step of the arpeggiator's order.
    #[allow(clippy::too_many_arguments)]
    fn play_next(&mut self, sorted: &[Held], shift: i16, vel: Option<u8>, at: f64, len: f64, ratchet: u8, gate: u8) {
        let n = sorted.len();
        let order = self.settings.order;
        if order == Order::Chord {
            for h in sorted {
                self.shifted_hit(*h, shift, vel, at, len, ratchet, gate);
            }
            return;
        }
        let mut played = [Held::default(); MAX_NOTES];
        let base: &[Held] = if order == Order::Played {
            played[..n].copy_from_slice(self.notes.active().as_slice());
            &played[..n]
        } else {
            sorted
        };
        let total = n * usize::from(self.settings.octaves);
        let c = self.arp_counter as usize;
        self.arp_counter += 1;
        let i = match order {
            Order::Up | Order::Played | Order::Chord => c % total,
            Order::Down => total - 1 - c % total,
            Order::UpDown if total == 1 => 0,
            Order::UpDown => {
                let p = c % (2 * total - 2);
                if p < total { p } else { 2 * total - 2 - p }
            }
            Order::UpDownInclusive => {
                let p = c % (2 * total);
                if p < total { p } else { 2 * total - 1 - p }
            }
            Order::Converge | Order::Diverge => {
                let k = if order == Order::Converge { c % total } else { total - 1 - c % total };
                if k % 2 == 0 { total - 1 - k / 2 } else { k / 2 }
            }
            Order::Random => self.random(total),
        };
        let h = base[i % n];
        self.shifted_hit(h, shift + 12 * (i / n) as i16, vel, at, len, ratchet, gate);
    }

    #[allow(clippy::too_many_arguments)]
    fn shifted_hit(&mut self, h: Held, shift: i16, vel: Option<u8>, at: f64, len: f64, ratchet: u8, gate: u8) {
        let note = i16::from(h.note) + shift;
        if (0..=127).contains(&note) {
            let v = vel.unwrap_or_else(|| self.velocity_of(h.velocity));
            self.hit(note as u8, v, at, len, ratchet, gate);
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
        (1..len).take_while(|k| steps[(i + k) % len] == Step::Tie).count()
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
            out.push(Event { offset, kind: p.kind });
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
            got.extend(out.as_slice().iter().map(|ev| (base + u64::from(ev.offset), ev.kind)));
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
        e.set_settings(Settings { mode, ..Settings::default() });
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
        assert_eq!(run(&mut e, 128, 128), [(0, EventKind::On { note: 5, velocity: 90 })]);
        e.release(5);
        assert_eq!(run(&mut e, 128, 128), [(128, EventKind::Off { note: 5 })]);
        e.set_latch(LatchMode::Replace);
        e.press(5, 90);
        e.release(5);
        assert_eq!(ons(&run(&mut e, 128, 128)), [(256, 5)]);
        e.press(6, 80); // a fresh press replaces the latched note
        let got = run(&mut e, 128, 128);
        assert_eq!(got, [(384, EventKind::Off { note: 5 }), (384, EventKind::On { note: 6, velocity: 80 })]);
        e.set_latch(LatchMode::Off); // 6 is still held, so it keeps sounding
        assert!(run(&mut e, 128, 128).is_empty());
    }

    #[test]
    fn note_repeat_retriggers_on_the_grid_with_the_gate() {
        let mut e = engine(Mode::Repeat);
        e.press(3, 100);
        let got = run(&mut e, STEP * 3, 128);
        assert_eq!(ons(&got), [(0, 3), (STEP, 3), (2 * STEP, 3)]);
        let offs: Vec<u64> = got.iter().filter(|(_, k)| matches!(k, EventKind::Off { .. })).map(|(t, _)| *t).collect();
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
        assert!(got.contains(&(STEP, EventKind::On { note: 3, velocity: 30 })));
    }

    #[test]
    fn timing_does_not_depend_on_the_period_size() {
        let mk = || {
            let mut e = engine(Mode::Pattern);
            e.set_pattern(Pattern::parse("x x:3 . x").unwrap());
            e.set_settings(Settings { mode: Mode::Pattern, swing: 62, rate: Rate::SixteenthTriplet, ..Settings::default() });
            e.press(1, 100);
            e.press(2, 100);
            e
        };
        let a = run(&mut mk(), 100_000, 128);
        assert!(a.len() > 40);
        for block in [1, 64, 127, 1000, 100_000] {
            assert_eq!(run(&mut mk(), 100_000, block), a, "block {block}");
        }
    }

    #[test]
    fn swing_moves_every_second_step_later() {
        let mut e = engine(Mode::Repeat);
        e.set_settings(Settings { mode: Mode::Repeat, swing: 66, ..Settings::default() });
        e.press(1, 100);
        let delay = STEP * 16 / 50; // (66 - 50) / 50 of a step
        assert_eq!(ons(&run(&mut e, STEP * 4, 128)), [(0, 1), (STEP + delay, 1), (2 * STEP, 1), (3 * STEP + delay, 1)]);
    }

    fn arp_notes(order: Order, octaves: u8, steps: u64) -> Vec<u8> {
        let mut e = Engine::new(SR, 120.0);
        e.set_settings(Settings { mode: Mode::Arp, order, octaves, ..Settings::default() });
        for n in [64, 60, 67] {
            e.press(n, 100);
        }
        ons(&run(&mut e, STEP * steps, 128)).into_iter().map(|(_, n)| n).collect()
    }

    #[test]
    fn arpeggiator_orders() {
        assert_eq!(arp_notes(Order::Up, 1, 4), [60, 64, 67, 60]);
        assert_eq!(arp_notes(Order::Down, 1, 4), [67, 64, 60, 67]);
        assert_eq!(arp_notes(Order::UpDown, 1, 6), [60, 64, 67, 64, 60, 64]);
        assert_eq!(arp_notes(Order::UpDownInclusive, 1, 7), [60, 64, 67, 67, 64, 60, 60]);
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
        e.set_settings(Settings { mode: Mode::Arp, octaves: 3, ..Settings::default() });
        e.press(110, 100);
        let n: Vec<u8> = ons(&run(&mut e, STEP * 3, 128)).into_iter().map(|(_, n)| n).collect();
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
                (0, EventKind::On { note: 60, velocity: 127 }),
                // gate 50% plus one tied step
                (STEP + STEP / 2, EventKind::Off { note: 60 }),
                (3 * STEP, EventKind::On { note: 64, velocity: 90 }),
                (3 * STEP + STEP / 4, EventKind::Off { note: 64 }),
                (3 * STEP + STEP / 2, EventKind::On { note: 64, velocity: 90 }),
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
                let on = got.iter().filter(|(_, k)| matches!(k, EventKind::On { note, .. } if *note == n)).count();
                let off = got.iter().filter(|(_, k)| *k == EventKind::Off { note: n }).count();
                assert!(off <= on && on <= off + 1, "{name}: note {n} has {on} ons and {off} offs");
            }
        }
    }

    #[test]
    fn changing_mode_stops_direct_notes_and_releasing_stops_the_generator() {
        let mut e = engine(Mode::Direct);
        e.press(9, 100);
        run(&mut e, 128, 128);
        e.set_settings(Settings { mode: Mode::Repeat, ..Settings::default() });
        let got = run(&mut e, 128, 128);
        assert_eq!(got[0], (128, EventKind::Off { note: 9 }));
        assert_eq!(got[1], (128, EventKind::On { note: 9, velocity: 100 }));
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
        assert!(got.iter().all(|(t, k)| *t == 128 && matches!(k, EventKind::Off { .. })));
        assert!(e.active().is_empty());
    }

    #[test]
    fn too_many_events_are_counted_not_lost_silently() {
        let mut e = engine(Mode::Repeat);
        e.set_settings(Settings { mode: Mode::Repeat, rate: Rate::ThirtySecondTriplet, ..Settings::default() });
        for n in 0..32 {
            e.press(n, 100);
        }
        let mut out = Events::new();
        e.process(48_000, &mut out);
        assert_eq!(out.as_slice().len(), MAX_EVENTS);
        assert!(out.dropped > 0 || e.overflow > 0);
    }

    #[test]
    fn settings_are_clamped() {
        let s = Settings { octaves: 9, gate: 0, swing: 99, accent: 0, velocity: Velocity::Fixed(0), ..Settings::default() }
            .clamped();
        assert_eq!((s.octaves, s.gate, s.swing, s.accent, s.velocity), (4, 1, 75, 1, Velocity::Fixed(1)));
    }
}
