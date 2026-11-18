//! A track's MIDI effects in order, run on one period's notes at a time.

use crate::fx::{Fx, LatchMode, StrumOrder, Time, MAX_MAP, STRUM_MOST, STRUM_TOGETHER_MS};
use crate::notes::{Ev, Kind, Notes, MAX_EVENTS};
use crate::track::TrackFx;

/// The most MIDI effects a track has.
pub const MAX_FX: usize = 8;

/// The period being played: where it starts on the frame count, how long it
/// is, and the sound's rate and tempo (for times in note values).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Period {
    pub start: u64,
    pub frames: u32,
    pub rate: u32,
    pub bpm: f64,
}

impl Period {
    pub fn end(&self) -> u64 {
        self.start + u64::from(self.frames)
    }
}

/// One place in the chain: its effect and whether it is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub fx: Fx,
    pub on: bool,
}

/// What follows a note's on, kept with what the note became.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Tail {
    None,
    /// Its off was put in with its on; the key's off ends nothing.
    Length {
        frames: u64,
    },
    /// Repeats were put in with the first on; the key's last off ends them.
    Echo {
        gap: u64,
        repeats: u8,
        pitch: i8,
    },
}

/// What one played note became in one stage, while its key is down.
#[derive(Clone, Copy, Debug)]
struct Held {
    /// Keys down on this note (a second on while held reuses the map).
    count: u8,
    /// Of those, the ones whose notes weren't put in (the list was full):
    /// their offs send nothing.
    silent: u8,
    len: u8,
    notes: [u8; MAX_MAP],
    /// Frames after the key each note starts (strum, humanize).
    delay: [u32; MAX_MAP],
    /// Bit i: note i takes the velocity played; the rest take `pct` of it.
    played: u8,
    pct: u8,
    /// Echo repeats' velocity, in percent of the one before.
    feedback: u8,
    /// Frames the key's off comes later (Randomize's Longer by).
    extra: u32,
    tail: Tail,
}

const IDLE: Held = Held {
    count: 0,
    silent: 0,
    len: 0,
    notes: [0; MAX_MAP],
    delay: [0; MAX_MAP],
    played: 0,
    pct: 100,
    feedback: 100,
    extra: 0,
    tail: Tail::None,
};

impl Held {
    fn one(note: u8) -> Held {
        let mut h = IDLE;
        h.len = 1;
        h.notes[0] = note;
        h.played = 1;
        h
    }
}

/// A small random number source, seeded, so a run can be played again.
#[derive(Clone, Copy, Debug)]
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// 0 to `n` inclusive.
    fn upto(&mut self, n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            self.next() % (n + 1)
        }
    }
}

struct Stage {
    slot: Option<Slot>,
    /// What each of the 128 notes coming in became, while its key is down.
    held: Box<[Held]>,
    /// Notes put in that leave in a later period (or later in this one).
    pending: Notes,
    /// Of each note number, how many this stage has sent on and not yet
    /// off (for [`Chain::cut`]).
    sounding: [u8; 128],
    /// A cut waiting for the period it falls in.
    cut: Option<u64>,
    /// Strum: where the chord being strummed started, and how many of its
    /// notes have come.
    group: Option<(u64, u8)>,
    /// Latch: the notes it holds on, the keys down while it latched, and
    /// how many keys that is.
    latched: [bool; 128],
    keys: [u8; 128],
    keys_down: u16,
}

impl Stage {
    fn new() -> Stage {
        Stage {
            slot: None,
            held: vec![IDLE; 128].into_boxed_slice(),
            pending: Notes::new(),
            sounding: [0; 128],
            cut: None,
            group: None,
            latched: [false; 128],
            keys: [0; 128],
            keys_down: 0,
        }
    }

    /// The effect at work on the notes: on, and not one of the CC
    /// effects (MIDI out applies those; the notes pass them untouched).
    fn active(&self) -> Option<Fx> {
        self.slot.filter(|s| s.on && !s.fx.is_cc()).map(|s| s.fx)
    }

    fn busy(&self) -> bool {
        self.active().is_some()
            || !self.pending.is_empty()
            || self.held.iter().any(|h| h.count > 0)
            || self.latching()
    }

    /// Latch holds notes on, or keys it took are down.
    fn latching(&self) -> bool {
        self.keys_down > 0 || self.latched.iter().any(|&l| l)
    }

    /// Ons stop being put in while half the list waits, so the offs of
    /// notes already on always have room.
    fn room_for_ons(&self, n: usize) -> bool {
        self.pending.len() + n <= MAX_EVENTS / 2
    }

    /// What `note` becomes, at the key's first on.
    fn map(fx: Option<Fx>, p: &Period, note: u8, rng: &mut Rng) -> Held {
        let Some(fx) = fx else {
            return Held::one(note);
        };
        match fx {
            Fx::Transpose { semitones } => {
                let n = i16::from(note) + i16::from(semitones);
                if (0..=127).contains(&n) {
                    Held::one(n as u8)
                } else {
                    IDLE
                }
            }
            Fx::Scale { scale, snap } => match scale.snapped(i16::from(note), snap) {
                Some(n) if (0..=127).contains(&n) => Held::one(n as u8),
                _ => IDLE,
            },
            Fx::Chord(c) => {
                let (notes, len, at) = c.notes(note);
                let strum = if c.strum_ms == 0 {
                    0
                } else {
                    crate::fx::Time::Ms(c.strum_ms).frames(p.rate, p.bpm)
                };
                let mut h = IDLE;
                h.len = len as u8;
                h.notes = notes;
                for i in 0..len {
                    h.delay[i] = (strum * i as u64).min(u64::from(u32::MAX)) as u32;
                }
                h.played = at.map_or(0, |i| 1 << i);
                h.pct = c.velocity.clamp(1, 100);
                h
            }
            Fx::Velocity(_) => Held::one(note),
            Fx::Length { time } => {
                let mut h = Held::one(note);
                h.tail = Tail::Length {
                    frames: time.frames(p.rate, p.bpm),
                };
                h
            }
            Fx::Echo(e) => {
                let mut h = Held::one(note);
                h.feedback = e.feedback.clamp(1, 100);
                h.tail = Tail::Echo {
                    gap: e.time.frames(p.rate, p.bpm),
                    repeats: e.repeats.clamp(1, 16),
                    pitch: e.pitch.clamp(-12, 12),
                };
                h
            }
            Fx::Humanize { time_ms, .. } => {
                let mut h = Held::one(note);
                let most = u64::from(time_ms.min(100)) * u64::from(p.rate) / 1000;
                h.delay[0] = rng.upto(most as u32);
                h
            }
            Fx::Chance { percent } => {
                if rng.upto(99) < u32::from(percent.min(100)) {
                    Held::one(note)
                } else {
                    IDLE
                }
            }
            // Strum's delay depends on the notes before it ([`Stage::on`]);
            // Latch takes its keys before they get here ([`Stage::take`]).
            Fx::Strum(_) | Fx::Latch { .. } => Held::one(note),
            // MIDI out's: never active here.
            Fx::CcScale(_) | Fx::CcSmooth(_) => Held::one(note),
            Fx::Randomize(r) => {
                let mut to = note;
                if r.pitch > 0 && rng.upto(99) < u32::from(r.often.min(100)) {
                    let most = i16::from(r.pitch.min(24));
                    let m = i16::from(note) + rng.upto(2 * most as u32) as i16 - most;
                    if (0..=127).contains(&m) {
                        to = m as u8;
                    }
                }
                let mut h = Held::one(to);
                if r.longer_ms > 0 {
                    let most = Time::Ms(r.longer_ms.min(1000)).frames(p.rate, p.bpm);
                    h.extra = rng.upto(most.min(u64::from(u32::MAX)) as u32);
                }
                h
            }
        }
    }

    /// Strum: how long after `at` a note that starts there waits, as the
    /// next of the chord being strummed (a new chord when the last began
    /// more than [`STRUM_TOGETHER_MS`] before).
    fn strum_delay(&mut self, p: &Period, at: u64, time_ms: u16) -> u32 {
        let together = STRUM_TOGETHER_MS * u64::from(p.rate) / 1000;
        let (start, count) = match self.group {
            Some((s, c)) if at >= s && at - s <= together => (s, c),
            _ => (at, 0),
        };
        self.group = Some((start, count.saturating_add(1)));
        let rank = u64::from(count).min(STRUM_MOST as u64 - 1);
        let gap = Time::Ms(time_ms.clamp(1, 200)).frames(p.rate, p.bpm);
        (start + rank * gap)
            .saturating_sub(at)
            .min(u64::from(u32::MAX)) as u32
    }

    fn on(&mut self, p: &Period, at: u64, note: u8, velocity: u8, rng: &mut Rng) {
        let fx = self.active();
        let n = usize::from(note & 0x7F);
        if self.held[n].count == u8::MAX {
            return;
        }
        let first = self.held[n].count == 0;
        if first {
            self.held[n] = Self::map(fx, p, note, rng);
            if let Some(Fx::Strum(st)) = fx {
                self.held[n].delay[0] = self.strum_delay(p, at, st.time_ms);
            }
        }
        self.held[n].count += 1;
        let h = self.held[n];
        if !self.room_for_ons(2 * usize::from(h.len)) {
            self.pending.dropped += 1;
            self.held[n].silent += 1;
            if let (true, Tail::Echo { gap, pitch, .. }) = (first, h.tail) {
                self.held[n].tail = Tail::Echo {
                    gap,
                    repeats: 0,
                    pitch,
                };
            }
            return;
        }
        let mut base_vel = velocity;
        for i in 0..usize::from(h.len) {
            let mut v = if h.played & (1 << i) != 0 {
                velocity
            } else {
                ((u32::from(velocity) * u32::from(h.pct) + 50) / 100).clamp(1, 127) as u8
            };
            v = match fx {
                Some(Fx::Velocity(c)) => c.apply(v),
                Some(Fx::Humanize { velocity: r, .. })
                | Some(Fx::Randomize(crate::fx::Randomize { velocity: r, .. })) => {
                    let r = u32::from(r.min(64));
                    let shift = rng.upto(2 * r) as i32 - r as i32;
                    (i32::from(v) + shift).clamp(1, 127) as u8
                }
                _ => v,
            };
            if i == 0 {
                base_vel = v;
            }
            let t = at + u64::from(h.delay[i]);
            self.pending.push(Ev::on(t, h.notes[i], v));
            if let Tail::Length { frames } = h.tail {
                self.pending.push(Ev::off(t + frames, h.notes[i]));
            }
        }
        if let (
            true,
            Tail::Echo {
                gap,
                repeats,
                pitch,
            },
        ) = (first, h.tail)
        {
            let mut v = f64::from(base_vel);
            let mut made = 0;
            for k in 1..=repeats {
                v *= f64::from(h.feedback) / 100.0;
                let vel = v.round() as u8;
                let rn = i16::from(note) + i16::from(pitch) * i16::from(k);
                if vel == 0 || !self.room_for_ons(2) {
                    break;
                }
                made = k;
                if (0..=127).contains(&rn) {
                    self.pending
                        .push(Ev::on(at + gap * u64::from(k), rn as u8, vel));
                }
            }
            self.held[n].tail = Tail::Echo {
                gap,
                repeats: made,
                pitch,
            };
        }
    }

    fn off(&mut self, at: u64, note: u8) {
        let n = usize::from(note & 0x7F);
        let h = self.held[n];
        if h.count == 0 {
            // Not one it started (it was on before the effect came).
            self.pending.push(Ev::off(at, note));
            return;
        }
        self.held[n].count -= 1;
        let silent = h.silent > 0;
        if silent {
            self.held[n].silent -= 1;
        }
        if !silent && !matches!(h.tail, Tail::Length { .. }) {
            for i in 0..usize::from(h.len) {
                let t = at + u64::from(h.delay[i]) + u64::from(h.extra);
                self.pending.push(Ev::off(t, h.notes[i]));
            }
        }
        if h.count == 1 {
            if let Tail::Echo {
                gap,
                repeats,
                pitch,
            } = h.tail
            {
                for k in 1..=repeats {
                    let rn = i16::from(note) + i16::from(pitch) * i16::from(k);
                    if (0..=127).contains(&rn) {
                        self.pending
                            .push(Ev::off(at + gap * u64::from(k), rn as u8));
                    }
                }
            }
            self.held[n] = IDLE;
        }
    }

    fn run(&mut self, p: &Period, input: &[Ev], out: &mut Notes, rng: &mut Rng) {
        // Latch switched off, changed or taken out: its notes end, and the
        // keys still down from it end nothing.
        if !matches!(self.active(), Some(Fx::Latch { .. })) && self.latching() {
            self.unlatch(p.start);
        }
        // A cut from a period not run lands at this one's start.
        let cut = self.cut.filter(|&c| c < p.end()).map(|c| c.max(p.start));
        let split = cut.map_or(input.len(), |c| input.partition_point(|e| e.at < c));
        self.take_all(p, &input[..split], rng);
        if let Some(c) = cut {
            self.cut = None;
            self.cut_at(c);
        }
        self.take_all(p, &input[split..], rng);
        let first = out.len();
        self.pending.drain_before(p.end(), out);
        for e in &out.as_slice()[first..] {
            let s = &mut self.sounding[usize::from(e.kind.note() & 0x7F)];
            *s = match e.kind {
                Kind::On { .. } => s.saturating_add(1),
                Kind::Off { .. } => s.saturating_sub(1),
            };
        }
    }

    /// The notes in time order. Strum Up or Down takes notes that start on
    /// the same frame lowest (or highest) first, so a chord from the pads
    /// or a clip strums in pitch order.
    fn take_all(&mut self, p: &Period, input: &[Ev], rng: &mut Rng) {
        let order = match self.active() {
            Some(Fx::Strum(st)) if st.order != StrumOrder::Played => st.order,
            _ => {
                for e in input {
                    self.take(p, e, rng);
                }
                return;
            }
        };
        let mut i = 0;
        while i < input.len() {
            let at = input[i].at;
            let mut run = [(0u8, 0u8); STRUM_MOST];
            let mut len = 0;
            while let Some(Kind::On { note, velocity }) = input.get(i).map(|e| e.kind) {
                if input[i].at != at || len == STRUM_MOST {
                    break;
                }
                run[len] = (note, velocity);
                len += 1;
                i += 1;
            }
            if len == 0 {
                self.take(p, &input[i], rng);
                i += 1;
                continue;
            }
            let run = &mut run[..len];
            run.sort_unstable_by_key(|&(n, _)| n);
            if order == StrumOrder::Down {
                run.reverse();
            }
            for &(note, velocity) in run.iter() {
                self.on(p, at, note, velocity, rng);
            }
        }
    }

    fn take(&mut self, p: &Period, e: &Ev, rng: &mut Rng) {
        match (self.active(), e.kind) {
            (Some(Fx::Latch { mode }), Kind::On { note, velocity }) => {
                self.latch_on(e.at, note, velocity, mode)
            }
            (Some(Fx::Latch { .. }), Kind::Off { note }) => self.latch_off(e.at, note),
            (_, Kind::On { note, velocity }) => self.on(p, e.at, note, velocity, rng),
            (_, Kind::Off { note }) => self.off(e.at, note),
        }
    }

    /// Latch: a key down. With Next chord, the first key down after all
    /// were up ends the notes held; the note then stays on. With Toggle, a
    /// note held on ends instead.
    fn latch_on(&mut self, at: u64, note: u8, velocity: u8, mode: LatchMode) {
        let n = usize::from(note & 0x7F);
        if mode == LatchMode::NextChord && self.keys_down == 0 {
            self.release_latched(at);
        }
        if self.latched[n] {
            if mode == LatchMode::Toggle {
                self.pending.push(Ev::off(at, note & 0x7F));
                self.latched[n] = false;
            }
        } else if self.room_for_ons(2) {
            self.pending.push(Ev::on(at, note & 0x7F, velocity));
            self.latched[n] = true;
        } else {
            self.pending.dropped += 1;
        }
        self.keys[n] = self.keys[n].saturating_add(1);
        self.keys_down = self.keys_down.saturating_add(1);
    }

    /// Latch: a key let go. Its note stays on; a key that went down before
    /// Latch came ends as it would have.
    fn latch_off(&mut self, at: u64, note: u8) {
        let n = usize::from(note & 0x7F);
        if self.keys[n] == 0 {
            self.off(at, note);
            return;
        }
        self.keys[n] -= 1;
        self.keys_down = self.keys_down.saturating_sub(1);
    }

    /// Ends every note Latch holds on, at `at`.
    fn release_latched(&mut self, at: u64) {
        for n in 0..128 {
            if self.latched[n] {
                self.pending.push(Ev::off(at, n as u8));
                self.latched[n] = false;
            }
        }
    }

    /// Latch went away: its notes end at `at`, and the keys still down
    /// from it end nothing when they let go.
    fn unlatch(&mut self, at: u64) {
        self.release_latched(at);
        for n in 0..128 {
            let k = self.keys[n];
            if k > 0 {
                let h = &mut self.held[n];
                h.count = h.count.saturating_add(k);
                h.silent = h.silent.saturating_add(k);
                self.keys[n] = 0;
            }
        }
        self.keys_down = 0;
    }

    /// From frame `c` nothing waiting plays: every note this stage has on
    /// then ends at `c`, and the keys down end nothing more.
    fn cut_at(&mut self, c: u64) {
        let mut on = self.sounding.map(i32::from);
        let keep = self.pending.as_slice().partition_point(|e| e.at < c);
        for e in &self.pending.as_slice()[..keep] {
            on[usize::from(e.kind.note() & 0x7F)] += match e.kind {
                Kind::On { .. } => 1,
                Kind::Off { .. } => -1,
            };
        }
        self.pending.truncate(keep);
        for (n, &k) in on.iter().enumerate() {
            for _ in 0..k.max(0) {
                self.pending.push(Ev::off(c, n as u8));
            }
        }
        // Latch's notes ended with the rest; its keys end nothing.
        self.latched = [false; 128];
        self.group = None;
        for h in self.held.iter_mut().filter(|h| h.count > 0) {
            h.silent = h.count;
            if let Tail::Echo { gap, pitch, .. } = h.tail {
                h.tail = Tail::Echo {
                    gap,
                    repeats: 0,
                    pitch,
                };
            }
        }
    }

    fn reset(&mut self) {
        self.held.fill(IDLE);
        self.pending.clear();
        self.sounding = [0; 128];
        self.cut = None;
        self.group = None;
        self.latched = [false; 128];
        self.keys = [0; 128];
        self.keys_down = 0;
    }
}

/// A track's MIDI effects, top to bottom. Notes go in once a period and
/// come out changed, each at its frame. It never allocates once made, so
/// it can run in the sound loop; settings change between periods.
///
/// A key's notes end where they went: what each played note became is
/// kept until its key is let go, so a setting changed, an effect switched
/// off or taken out while a key is down still ends that key's notes.
/// A panic is [`Chain::reset`] and an all notes off after it.
pub struct Chain {
    stages: [Stage; MAX_FX],
    scratch: [Notes; 2],
    rng: Rng,
}

impl Default for Chain {
    fn default() -> Self {
        Self::new()
    }
}

impl Chain {
    /// About 400 KB, on the heap: make it once, off the sound loop.
    pub fn new() -> Chain {
        Chain {
            stages: std::array::from_fn(|_| Stage::new()),
            scratch: [Notes::new(), Notes::new()],
            rng: Rng(0x2545_f491),
        }
    }

    /// The effect in place `i`, if any.
    pub fn slot(&self, i: usize) -> Option<Slot> {
        self.stages.get(i).and_then(|s| s.slot)
    }

    /// Puts an effect in place `i` (None takes it out). Notes its old
    /// effect started still end as they would have.
    pub fn set(&mut self, i: usize, slot: Option<Slot>) {
        if let Some(s) = self.stages.get_mut(i) {
            s.slot = slot;
        }
    }

    /// The effects in order, with their places.
    pub fn slots(&self) -> impl Iterator<Item = (usize, Slot)> + '_ {
        self.stages
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.slot.map(|sl| (i, sl)))
    }

    /// Something is on, waiting or held: the period has work to do.
    pub fn busy(&self) -> bool {
        self.stages.iter().any(Stage::busy)
    }

    /// Starts the random effects (Humanize, Chance) again from `seed`.
    pub fn reseed(&mut self, seed: u32) {
        self.rng = Rng(seed.max(1));
    }

    /// Notes dropped because a list was full, since it was made.
    pub fn dropped(&self) -> u64 {
        self.stages.iter().map(|s| s.pending.dropped).sum::<u64>()
            + self.scratch.iter().map(|s| s.dropped).sum::<u64>()
    }

    /// Forgets every note held and waiting (a panic: send all notes off
    /// after it).
    pub fn reset(&mut self) {
        for s in &mut self.stages {
            s.reset();
        }
    }

    /// From frame `at` (in this period or the next one run) nothing
    /// waiting in the chain plays: echoes, strummed and humanized notes
    /// not yet started are dropped, every note the chain has on ends at
    /// `at`, and the keys down then end nothing. Notes coming in after
    /// it play as usual. For a track muted while it plays.
    pub fn cut(&mut self, at: u64) {
        for s in &mut self.stages {
            s.cut = Some(at);
        }
    }

    /// Puts each of `fx`'s effects in its place, where it changed.
    pub fn apply(&mut self, fx: &TrackFx) {
        for (i, slot) in fx.slots.iter().enumerate() {
            if self.slot(i) != *slot {
                self.set(i, *slot);
            }
        }
    }

    /// Runs the period's notes (`input`, in time order, inside `p`)
    /// through every effect; what leaves in this period goes to `out`.
    pub fn process(&mut self, p: &Period, input: &[Ev], out: &mut Notes) {
        self.process_range(0, MAX_FX, p, input, out);
    }

    /// [`Chain::process`] through the effects in places `from` to `to`
    /// (not included): the ones before a track's arp, then after it.
    pub fn process_range(
        &mut self,
        from: usize,
        to: usize,
        p: &Period,
        input: &[Ev],
        out: &mut Notes,
    ) {
        let to = to.min(MAX_FX);
        let mut cur = 0;
        self.scratch[cur].clear();
        for e in input {
            self.scratch[cur].push(*e);
        }
        for i in from..to {
            let (a, b) = self.scratch.split_at_mut(1);
            let (src, dst) = if cur == 0 {
                (&a[0], &mut b[0])
            } else {
                (&b[0], &mut a[0])
            };
            dst.clear();
            self.stages[i].run(p, src.as_slice(), dst, &mut self.rng);
            cur = 1 - cur;
        }
        for e in self.scratch[cur].as_slice() {
            out.push(*e);
        }
    }
}
