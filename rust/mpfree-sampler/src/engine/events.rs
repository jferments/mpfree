//! Events: the queue that lands each on its frame, let-gos that are never
//! lost (they wait aside when the queue is full), the take's marks, and the
//! pads' live values and locks.

use super::*;

/// Events that can wait for the next render. A let-go is never refused:
/// past this it waits aside ([`Engine::schedule_for`]).
pub const MAX_EVENTS: usize = 256;

/// The places each kit has for let-gos the queue had no room for: one for
/// each pad number's let-go ([`Event::NoteOff`]), one for each key's
/// ([`Event::KeyOff`]) and one for its take's ([`Event::TakeOff`]). The
/// engine has one more, for all notes off.
pub(super) const LET_GO_PLACES: usize = 2 * 256 + 1;

/// A let-go place with nothing waiting in it.
pub(super) const NOT_DUE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    NoteOn {
        pad: u8,
        velocity: u8,
    },
    NoteOff {
        pad: u8,
    },
    /// Pressure on a held pad, 0 to 127 (polyphonic aftertouch). Moves the
    /// pad's pressure target on its playing voices, smoothed over about 5 ms.
    Pressure {
        pad: u8,
        value: u8,
    },
    /// A note through the keygroups: every pad whose [`Keys`] hold it
    /// plays, a semitone higher for each note above its root.
    Key {
        note: u8,
        velocity: u8,
    },
    KeyOff {
        note: u8,
    },
    /// Releases every voice (gate-style), e.g. on STOP.
    AllOff,
    /// Freezes the master effect (on) or lets it take in new audio again
    /// (off). Frozen, its grains keep playing the audio it holds while
    /// the pads carry on dry. Does nothing without a master effect.
    Freeze {
        on: bool,
    },
    /// Tape stop (`stop`) or start: every voice slows to a halt, or comes
    /// back up to speed, over `ms` milliseconds, as a tape machine's
    /// motor would; the sound fades out as it nears a halt. Stopped, new
    /// hits wait silent until the tape starts again.
    Tape {
        stop: bool,
        ms: u16,
    },
    /// Plays source frames `start..end` of layer `layer` of pad `pad` once,
    /// forward from its first frame, with the pad's settings (level, pan,
    /// tune, channels, envelopes, filter, pressure), as a pad made from
    /// that slice plays it: the Slice tool's audition of a slice. Pad
    /// [`TAKE_PAD`] is the take the view holds, with its settings. The
    /// pad's own playing voices fade out first; mute groups and chokes are
    /// left alone. `velocity` as for [`Event::NoteOn`]. The frames are
    /// clipped to the source; nothing plays if none are left.
    Audition {
        pad: u8,
        layer: u8,
        start: u32,
        end: u32,
        velocity: u8,
    },
    /// Plays the take the view holds (the one the Sample page shows, kept
    /// off the pads) at `velocity`, as a pad of one sound would: its own
    /// voices end first. Its voices carry [`TAKE_PAD`]; it has no round
    /// robin, Mono, mute group, chokes, live values or locks, which are
    /// the pads'. Nothing plays without a take.
    TakeOn {
        velocity: u8,
    },
    /// Lets the take go, as a pad let go does (a Gate or Loop take stops).
    TakeOff,
    /// Changes one of pad `pad`'s settings while it plays, as a knob, a
    /// p-lock or a modulation does: `value` replaces the pad's own until
    /// the next change (NaN gives the pad's own back) or a new view
    /// ([`Engine::set_view`]). New voices start with it; playing voices
    /// follow the fields [`PadField::follows`] says, gliding there. Values
    /// out of range are clamped to it ([`PadField::clamp`]).
    PadLive {
        pad: u8,
        field: PadField,
        value: f32,
    },
    /// A step's parameter lock on pad `pad` (the sequencer's
    /// `Event::Lock` on a pad's sound): `value` takes the place of the
    /// pad's own `field` until the next lock or its end (NaN), as the
    /// player sends them on their frames. Under a live value
    /// ([`Event::PadLive`], the scene crossfader), which still goes over
    /// it; a new view keeps it. New voices start with it and playing
    /// voices follow it as they follow a live value, so a lock with no
    /// note moves the sound still ringing. A locked Decay also makes the
    /// sound decay to silence (the pad's sustain at 0), as the Play page's
    /// Decay knob does, so it shortens a sound that plays out.
    PadLock {
        pad: u8,
        field: PadField,
        value: f32,
    },
    /// The XY pad's hand on pad `pad`'s `field` (a layout of yours plays it
    /// by Set): `value` plays in place of the pad's own, its live value and
    /// its lock, until NaN lets go. With `over` false (not recording) a
    /// step's lock still plays over it, as the mod engine's hand rule says.
    /// It stays through a new view; playing voices follow it as they
    /// follow a live value.
    PadHand {
        pad: u8,
        field: PadField,
        value: f32,
        over: bool,
    },
    /// Moves every pad at once by an offset on its own setting (or on its
    /// live one): see [`ProgramField`]. 0 or NaN is no offset. New voices
    /// start with it and playing voices follow every field. It stays when
    /// a new view comes in.
    ProgramLive {
        field: ProgramField,
        value: f32,
    },
    /// A keyboard's pitch wheel: the keygroups' notes (not pad hits or 16
    /// Levels) move by `cents` (-4800 to +4800) on top of the program's
    /// Tune, playing voices too, gliding there as they do for Tune. It
    /// stays until the next bend, through new views. A warped layer keeps
    /// its pitch.
    Bend {
        cents: f32,
    },
    /// Live chop on pad `pad`'s sound (`Some`), or off (`None`): while
    /// it is on and the pad's audition plays, [`Event::Mark`] notes where
    /// the audition is in its source ([`Engine::mark`]). Turned off, the
    /// audition stops (fading out); it ends by itself when the audition
    /// does ([`Engine::marking`]).
    LiveChop {
        pad: Option<u8>,
    },
    /// While live chop is on: a cut where the audition is now, to the
    /// frame. Nothing plays.
    Mark,
}

impl Event {
    /// Whether it lets go of something held (a pad, a key, the take, or
    /// all of them), which the queue never refuses.
    pub fn lets_go(&self) -> bool {
        matches!(
            self,
            Event::NoteOff { .. } | Event::KeyOff { .. } | Event::TakeOff | Event::AllOff
        )
    }

    /// Whether it starts a sound (a pad, a key, the take or an audition).
    pub fn is_hit(&self) -> bool {
        matches!(
            self,
            Event::NoteOn { .. }
                | Event::Key { .. }
                | Event::TakeOn { .. }
                | Event::Audition { .. }
        )
    }

    /// The place in its kit's [`LET_GO_PLACES`] a let-go waits in when the
    /// queue is full, or a hit's matching let-go does. None for all notes
    /// off (the engine's own place) and everything else.
    pub(super) fn let_go_place(&self) -> Option<usize> {
        match *self {
            Event::NoteOff { pad } | Event::NoteOn { pad, .. } => Some(usize::from(pad)),
            Event::KeyOff { note } | Event::Key { note, .. } => Some(256 + usize::from(note)),
            Event::TakeOff | Event::TakeOn { .. } => Some(512),
            _ => None,
        }
    }

    /// The let-go waiting in place `place` of a kit's [`LET_GO_PLACES`].
    pub(super) fn let_go_at(place: usize) -> Event {
        match place {
            0..=255 => Event::NoteOff { pad: place as u8 },
            256..=511 => Event::KeyOff {
                note: (place - 256) as u8,
            },
            _ => Event::TakeOff,
        }
    }

    /// What a pressure or live value sets (kind, pad, setting), where a
    /// newer value for the same takes its place when the queue is full.
    /// None for every other event.
    pub(super) fn value_key(&self) -> Option<(u8, u8, u8)> {
        match *self {
            Event::Pressure { pad, .. } => Some((0, pad, 0)),
            Event::PadLive { pad, field, .. } => Some((1, pad, field as u8)),
            Event::ProgramLive { field, .. } => Some((2, 0, field as u8)),
            Event::Bend { .. } => Some((3, 0, 0)),
            Event::PadHand { pad, field, .. } => Some((4, pad, field as u8)),
            _ => None,
        }
    }
}

/// The cuts live chop keeps for reading ([`Engine::mark`]).
pub const MAX_MARKS: usize = 64;

/// A pad setting [`Event::PadLive`] changes while the pad plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PadField {
    /// The pad's level, in dB (-120 to +24): replaces its own Level.
    Level = 0,
    /// The pad's pan, -1 (left) to +1 (right); each layer's own pan adds
    /// to it, as it does to the pad's own.
    Pan = 1,
    /// The pad's tune in cents (-4800 to +4800), for new voices. A warped
    /// layer keeps its pitch (its tune is in its stretched copy).
    Tune = 2,
    /// The filter's cutoff in Hz (10 to 22 000). A pad whose filter is off
    /// gets a 2-pole low-pass. It also takes the place of 16 Levels'
    /// cutoff.
    Cutoff = 3,
    /// The filter's resonance, 0 to 1 (only where there is a filter).
    Resonance = 4,
    /// The amp envelope's attack, decay and release in ms (0 to 60 000),
    /// for new voices.
    Attack = 5,
    Decay = 6,
    Release = 7,
    /// The slice a hit plays on a sliced pad, from 0 (R4): replaces the
    /// pad's own [`crate::model::Pad::slice`], rounded to a whole slice,
    /// for new voices.
    Slice = 8,
    /// The LFO's depth, -1 to 1 (sampler R5 part 4): playing voices follow
    /// it. It moves nothing on a pad whose LFO has no target.
    LfoDepth = 9,
    /// The pitch envelope's start in semitones (-24 to +24) and its decay
    /// in ms (1 to 10 000), for new voices.
    PitchEnv = 10,
    PitchDecay = 11,
}

impl PadField {
    pub const COUNT: usize = 12;
    pub const ALL: [PadField; PadField::COUNT] = [
        PadField::Level,
        PadField::Pan,
        PadField::Tune,
        PadField::Cutoff,
        PadField::Resonance,
        PadField::Attack,
        PadField::Decay,
        PadField::Release,
        PadField::Slice,
        PadField::LfoDepth,
        PadField::PitchEnv,
        PadField::PitchDecay,
    ];

    /// Whether playing voices follow it (Level, Pan, Cutoff, Resonance,
    /// LFO depth); the others reach new voices only.
    pub fn follows(self) -> bool {
        matches!(
            self,
            PadField::Level
                | PadField::Pan
                | PadField::Cutoff
                | PadField::Resonance
                | PadField::LfoDepth
        )
    }

    /// `x` within the field's range (NaN stays NaN).
    pub fn clamp(self, x: f32) -> f32 {
        let (lo, hi) = match self {
            PadField::Level => (-120.0, 24.0),
            PadField::Pan => (-1.0, 1.0),
            PadField::Tune => (-4800.0, 4800.0),
            PadField::Cutoff => (10.0, 22_000.0),
            PadField::Resonance => (0.0, 1.0),
            PadField::Attack | PadField::Decay | PadField::Release => (0.0, 60_000.0),
            PadField::Slice => (0.0, (crate::model::MAX_SLICES - 1) as f32),
            PadField::LfoDepth => (-1.0, 1.0),
            PadField::PitchEnv => (-PITCH_ENV_ST, PITCH_ENV_ST),
            PadField::PitchDecay => (*PITCH_DECAY_MS.start(), *PITCH_DECAY_MS.end()),
        };
        x.clamp(lo, hi)
    }
}

/// An offset on every pad's own setting, from [`Event::ProgramLive`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProgramField {
    /// dB added to every pad's level (-120 to +24).
    Level = 0,
    /// Cents added to every pad's tune (-4800 to +4800); playing voices
    /// glide there too. Warped layers keep their pitch.
    Tune = 1,
    /// Octaves the cutoff of every filter moves (-10 to +10). Below 0, a
    /// pad without a filter gets a 2-pole low-pass, so the whole kit
    /// darkens.
    Cutoff = 2,
}

impl ProgramField {
    pub const COUNT: usize = 3;
    pub const ALL: [ProgramField; ProgramField::COUNT] = [
        ProgramField::Level,
        ProgramField::Tune,
        ProgramField::Cutoff,
    ];

    /// `x` within the field's range, NaN as 0 (no offset).
    pub fn clamp(self, x: f32) -> f32 {
        if x.is_nan() {
            return 0.0;
        }
        let (lo, hi) = match self {
            ProgramField::Level => (-120.0, 24.0),
            ProgramField::Tune => (-4800.0, 4800.0),
            ProgramField::Cutoff => (-10.0, 10.0),
        };
        x.clamp(lo, hi)
    }
}

/// The live settings: each pad's [`PadField`]s (NaN where the pad's own
/// applies), its step locks under them, the XY pad's hand over both, the
/// program's offsets, and the pitch wheel's bend on the keygroups.
#[derive(Clone, Copy, Debug)]
pub(super) struct Live {
    pub(super) pads: [[f32; PadField::COUNT]; PADS],
    pub(super) locks: [[f32; PadField::COUNT]; PADS],
    /// The XY pad's hand ([`Event::PadHand`]), and whether it holds over
    /// the locks.
    pub(super) hands: [[f32; PadField::COUNT]; PADS],
    pub(super) over: [[bool; PadField::COUNT]; PADS],
    pub(super) program: [f32; ProgramField::COUNT],
    /// Cents the keygroups' notes move by ([`Event::Bend`]).
    pub(super) bend: f32,
}

impl Live {
    pub(super) const OWN: [[f32; PadField::COUNT]; PADS] = [[f32::NAN; PadField::COUNT]; PADS];

    /// Nothing live, locked or held: every pad's own settings play.
    pub(super) const fn new() -> Live {
        Live {
            pads: Live::OWN,
            locks: Live::OWN,
            hands: Live::OWN,
            over: [[false; PadField::COUNT]; PADS],
            program: [0.0; ProgramField::COUNT],
            bend: 0.0,
        }
    }

    /// The program's offsets as a voice playing `note` hears them: a
    /// keygroup's note (below [`LEVEL_NOTE`]: not a pad hit, [`NO_NOTE`],
    /// or a 16 Levels voice) has the bend on its Tune.
    pub(super) fn program_for(&self, note: u8) -> [f32; ProgramField::COUNT] {
        let mut program = self.program;
        if note < LEVEL_NOTE {
            program[ProgramField::Tune as usize] += self.bend;
        }
        program
    }

    /// Pad `p`'s settings in place of its own: each field's XY hand (over
    /// a lock only while it holds over the locks), else its live value,
    /// else its lock (NaN where none is set).
    pub(super) fn row(&self, p: usize) -> [f32; PadField::COUNT] {
        let (Some(live), Some(locks), Some(hands), Some(over)) = (
            self.pads.get(p),
            self.locks.get(p),
            self.hands.get(p),
            self.over.get(p),
        ) else {
            return [f32::NAN; PadField::COUNT];
        };
        let mut out = *live;
        for (f, o) in out.iter_mut().enumerate() {
            let (hand, lock) = (hands[f], locks[f]);
            if !hand.is_nan() && (over[f] || lock.is_nan()) {
                *o = hand;
            } else if o.is_nan() {
                *o = lock;
            }
        }
        out
    }
}

impl Engine {
    /// Full Level (Some(127)), Half Level (Some(64)), or off (None).
    pub fn set_fixed_velocity(&mut self, v: Option<u8>) {
        self.fixed_velocity = v.map(|v| v.clamp(1, 127));
    }

    /// Queues an event `offset` frames into the next render (later renders if
    /// the offset is past its end). Events at the same offset keep their order.
    /// For kit 0; see [`Engine::schedule_for`] for a full queue.
    pub fn schedule(&mut self, offset: u32, ev: Event) -> bool {
        self.schedule_for(0, offset, ev)
    }

    /// [`Engine::schedule`] for kit `kit`: its pads, its live settings. One
    /// queue holds every kit's events, in the order they come. Returns
    /// false for a kit out of range, which is not counted.
    ///
    /// The queue holds [`MAX_EVENTS`]. When it is full, a pressure or live
    /// value whose pad and setting already has one waiting merges with it
    /// (the newer value stays), and any event takes the place of a value
    /// that has a newer one for its pad and setting behind it; both are
    /// counted ([`Stats::merged_values`]). A let-go ([`Event::lets_go`]) is
    /// never refused: with no place to take, it waits aside, in a place of
    /// its own for each pad, key and take of each kit, and lands on its
    /// frame after the events queued for that frame, as it would from the
    /// queue (counted, [`Stats::late_let_gos`]). Two let-gos waiting in the
    /// same place land together, on the later one's frame, so a hit between
    /// them is let go of late, never left playing. A hit queued for the
    /// frame a let-go waiting aside lands on moves one frame later, so it
    /// still comes after it. Anything else is refused: false, counted
    /// ([`Stats::dropped_events`], hits also [`Stats::refused_hits`]).
    /// Never allocates.
    pub fn schedule_for(&mut self, kit: usize, offset: u32, ev: Event) -> bool {
        if kit >= self.views.len() {
            return false;
        }
        let mut offset = offset;
        if !self.late.is_empty() && ev.is_hit() {
            offset = self.after_late(kit, offset, ev);
        }
        let key = ev.value_key();
        if self.events.len() >= MAX_EVENTS {
            if let Some(key) = key {
                if self.merge_value(kit, key, offset, ev) {
                    return true;
                }
            }
            if !self.make_room() {
                if ev.lets_go() {
                    self.wait_late(kit, offset, ev);
                    return true;
                }
                self.stats.dropped_events += 1;
                if ev.is_hit() {
                    self.stats.refused_hits += 1;
                }
                return false;
            }
        }
        self.queue(kit, offset, ev);
        if key.is_some() {
            self.nothing_to_merge = false;
        }
        true
    }

    /// Puts an event in the queue, after those at the same offset.
    pub(super) fn queue(&mut self, kit: usize, offset: u32, ev: Event) {
        let i = self.events.partition_point(|(o, _, _)| *o <= offset);
        self.events.insert(i, (offset, kit as u8, ev));
    }

    /// Where kit `kit`'s hit `ev` at `offset` goes: one frame later if a
    /// let-go waiting aside for its pad, key or take (or all notes off)
    /// lands on that frame, as it came first.
    pub(super) fn after_late(&self, kit: usize, offset: u32, ev: Event) -> u32 {
        let own = ev
            .let_go_place()
            .map_or(NOT_DUE, |p| self.late_due[kit * LET_GO_PLACES + p]);
        let all = self.late_due[self.late_due.len() - 1];
        if own == offset || all == offset {
            offset.saturating_add(1).min(NOT_DUE - 1)
        } else {
            offset
        }
    }

    /// The full queue's newest value for kit `kit`'s `key` merges with the
    /// value `ev` at `offset`: the newer of the two stays. False if there is
    /// none waiting.
    pub(super) fn merge_value(
        &mut self,
        kit: usize,
        key: (u8, u8, u8),
        offset: u32,
        ev: Event,
    ) -> bool {
        let kit8 = kit as u8;
        let Some(j) = self
            .events
            .iter()
            .rposition(|&(_, k, e)| k == kit8 && e.value_key() == Some(key))
        else {
            return false;
        };
        if self.events[j].0 <= offset {
            self.events.remove(j);
            self.queue(kit, offset, ev);
        }
        self.stats.merged_values += 1;
        true
    }

    /// Makes room in the queue by dropping a pressure or live value that a
    /// newer one for the same pad and setting comes after (counted). False
    /// if there is none.
    pub(super) fn make_room(&mut self) -> bool {
        if self.nothing_to_merge {
            return false;
        }
        let n = self.events.len();
        for i in 0..n {
            let (_, kit, e) = self.events[i];
            let Some(key) = e.value_key() else {
                continue;
            };
            if self.events[i + 1..]
                .iter()
                .any(|&(_, k, e)| k == kit && e.value_key() == Some(key))
            {
                self.events.remove(i);
                self.stats.merged_values += 1;
                return true;
            }
        }
        self.nothing_to_merge = true;
        false
    }

    /// Kit `kit`'s let-go `ev` at `offset` waits aside, in its place: with
    /// one already there, the two land together on the later frame.
    pub(super) fn wait_late(&mut self, kit: usize, offset: u32, ev: Event) {
        let i = match ev.let_go_place() {
            Some(p) => kit * LET_GO_PLACES + p,
            None => self.late_due.len() - 1, // all notes off
        };
        let due = offset.min(NOT_DUE - 1);
        let d = &mut self.late_due[i];
        if *d == NOT_DUE {
            *d = due;
            self.late.push(i as u32);
        } else {
            *d = (*d).max(due);
        }
        self.stats.late_let_gos += 1;
    }

    /// Lands the let-gos waiting aside that are due by frame `at` of the
    /// render; returns the frame the next one is due on (`n` if none is due
    /// in this render).
    pub(super) fn land_late(&mut self, at: u32, n: u32) -> u32 {
        let mut next = n;
        let mut k = 0;
        while k < self.late.len() {
            let i = self.late[k] as usize;
            let due = self.late_due[i];
            if due > at {
                next = next.min(due);
                k += 1;
                continue;
            }
            self.late_due[i] = NOT_DUE;
            self.late.swap_remove(k);
            if i == self.late_due.len() - 1 {
                self.apply_for(0, Event::AllOff);
            } else {
                let ev = Event::let_go_at(i % LET_GO_PLACES);
                self.apply_for(i / LET_GO_PLACES, ev);
            }
        }
        next
    }

    /// How many let-gos wait aside now ([`Engine::schedule_for`]).
    pub fn late_waiting(&self) -> usize {
        self.late.len()
    }

    /// Applies an event now (before the next rendered frame). For kit 0.
    pub fn apply(&mut self, ev: Event) {
        self.apply_for(0, ev)
    }

    /// [`Engine::apply`] for kit `kit`. A kit out of range gets nothing.
    pub fn apply_for(&mut self, kit: usize, ev: Event) {
        if kit >= self.views.len() {
            return;
        }
        self.at = kit;
        match ev {
            // With the pads playing notes, a pad is the note it plays;
            // with 16 Levels, the one pad at its level.
            Event::NoteOn { pad, velocity } => match self.target(pad) {
                Target::Own(_) => self.hit(pad, velocity),
                Target::Key(note) => self.key(note, velocity),
                Target::Level { pad: p, by, k } => {
                    self.note_on(p, velocity, LEVEL_NOTE + k, self.level_vary(p, by, k))
                }
            },
            Event::NoteOff { pad } => match self.target(pad) {
                Target::Key(note) => self.key_off(note),
                t => self.release(|v| t.played(v)),
            },
            Event::Key { note, velocity } => self.key(note.min(127), velocity),
            Event::KeyOff { note } => self.key_off(note),
            Event::Pressure { pad, value } => {
                let t = self.target(pad);
                for (v, _) in self.pool.iter_mut() {
                    if v.active && usize::from(v.kit) == kit && t.played(v) && v.kill_left == 0 {
                        v.press_target = f32::from(value.min(127)) / 127.0;
                    }
                }
            }
            Event::AllOff => {
                self.mono_keys.fill(HeldKeys::NONE);
                for (v, g) in self.pool.iter_mut() {
                    if v.active {
                        v.held = false;
                        v.looping = false;
                        if let Some(a) = &mut v.amp {
                            a.release();
                        }
                        if v.granular {
                            g.stop();
                        }
                    }
                }
            }
            Event::Freeze { on } => {
                if let Some(fx) = &mut self.master {
                    fx.set_frozen(on);
                }
            }
            Event::Tape { stop, ms } => {
                self.tape_target = if stop { 0.0 } else { 1.0 };
                let frames = f32::from(ms) * self.rate / 1000.0;
                if frames < 1.0 {
                    self.tape = self.tape_target;
                    self.tape_rate = 0.0;
                } else {
                    self.tape_rate = 1.0 / frames;
                }
                self.tape_moved = true;
            }
            Event::Audition {
                pad,
                layer,
                start,
                end,
                velocity,
            } => self.audition(pad, layer, start, end, velocity),
            Event::TakeOn { velocity } => self.take_on(velocity),
            Event::TakeOff => self.release(|v| v.pad == TAKE_PAD),
            Event::PadLive { pad, field, value } => self.pad_live(pad, field, value),
            Event::PadLock { pad, field, value } => self.pad_lock(pad, field, value),
            Event::PadHand {
                pad,
                field,
                value,
                over,
            } => self.pad_hand(pad, field, value, over),
            Event::ProgramLive { field, value } => self.program_live(field, value),
            Event::Bend { cents } => self.bend(cents),
            Event::LiveChop { pad } => {
                if let (None, Some(was)) = (pad, self.chop_pad) {
                    let chop_kit = self.chop_kit;
                    for (v, _) in self.pool.iter_mut() {
                        if v.active && v.audition && v.kit == chop_kit && v.pad == was {
                            Self::kill(v);
                        }
                    }
                }
                self.chop_pad = pad;
                if pad.is_some() {
                    self.chop_kit = kit as u8;
                }
            }
            Event::Mark => {
                if let Some(at) = self.chop_pad.and_then(|p| self.audition_at(p)) {
                    self.marks[(self.marks_made % MAX_MARKS as u64) as usize] = at;
                    self.marks_made += 1;
                }
            }
        }
    }

    /// Where live chop's kit's pad `pad`'s newest audition is in its
    /// source, if one plays.
    pub(super) fn audition_at(&self, pad: u8) -> Option<u64> {
        let mut best: Option<(u64, u64)> = None;
        for (v, g) in self.pool.iter() {
            if v.active && v.audition && v.kit == self.chop_kit && v.pad == pad && v.kill_left == 0
            {
                let at = self.source_frame(v, g);
                if best.is_none_or(|(age, _)| v.age > age) {
                    best = Some((v.age, at));
                }
            }
        }
        best.map(|(_, at)| at)
    }

    /// The pad live chop takes cuts from, while it is on and that pad's
    /// audition plays: hits mark cuts ([`Event::Mark`]) instead of playing
    /// then. `None` once the audition ends, even before live chop is
    /// turned off.
    pub fn marking(&self) -> Option<u8> {
        self.chop_pad.filter(|&p| self.audition_at(p).is_some())
    }

    /// The kit live chop was last turned on for ([`Event::LiveChop`]
    /// through [`Engine::apply_for`] or [`Engine::schedule_for`]).
    pub fn marking_kit(&self) -> usize {
        usize::from(self.chop_kit)
    }

    /// How many cuts live chop has made.
    pub fn marks_made(&self) -> u64 {
        self.marks_made
    }

    /// Cut `i` (from 0, in the order made), in source frames, while it is
    /// among the last [`MAX_MARKS`].
    pub fn mark(&self, i: u64) -> Option<u64> {
        (i < self.marks_made && self.marks_made - i <= MAX_MARKS as u64)
            .then(|| self.marks[(i % MAX_MARKS as u64) as usize])
    }

    /// [`Event::PadLive`]: the table, then the playing voices of the pad.
    pub(super) fn pad_live(&mut self, pad: u8, field: PadField, value: f32) {
        let Some(row) = self.lives[self.at].pads.get_mut(usize::from(pad)) else {
            return;
        };
        row[field as usize] = field.clamp(value);
        self.follow_pad(pad, field);
    }

    /// [`Event::PadLock`]: the table, then the playing voices of the pad.
    pub(super) fn pad_lock(&mut self, pad: u8, field: PadField, value: f32) {
        let Some(row) = self.lives[self.at].locks.get_mut(usize::from(pad)) else {
            return;
        };
        row[field as usize] = field.clamp(value);
        self.follow_pad(pad, field);
    }

    /// [`Event::PadHand`]: the table, then the playing voices of the pad.
    pub(super) fn pad_hand(&mut self, pad: u8, field: PadField, value: f32, over: bool) {
        let live = &mut self.lives[self.at];
        let (Some(row), Some(o)) = (
            live.hands.get_mut(usize::from(pad)),
            live.over.get_mut(usize::from(pad)),
        ) else {
            return;
        };
        row[field as usize] = if value.is_nan() {
            f32::NAN
        } else {
            field.clamp(value)
        };
        o[field as usize] = over;
        self.follow_pad(pad, field);
    }

    /// Pad `pad`'s playing voices go where its settings now say, if they
    /// follow `field`.
    pub(super) fn follow_pad(&mut self, pad: u8, field: PadField) {
        if field.follows() {
            let at = self.at;
            let (live, row) = (&self.lives[at], self.lives[at].row(usize::from(pad)));
            let t = &self.tunings[at];
            for (v, _) in self.pool.iter_mut() {
                if v.active && usize::from(v.kit) == at && v.pad == pad {
                    retarget(v, &row, &live.program_of(v, t));
                }
            }
        }
    }

    /// [`Event::ProgramLive`]: the offset, then every playing voice.
    pub(super) fn program_live(&mut self, field: ProgramField, value: f32) {
        let at = self.at;
        self.lives[at].program[field as usize] = field.clamp(value);
        let (live, t) = (&self.lives[at], &self.tunings[at]);
        for (v, _) in self.pool.iter_mut() {
            // The pads' voices and the take's (no live values of its own).
            let p = usize::from(v.pad);
            if v.active && usize::from(v.kit) == at && (p < PADS || v.pad == TAKE_PAD) {
                retarget(v, &live.row(p), &live.program_of(v, t));
            }
        }
    }

    /// [`Event::Bend`]: the bend, then every keygroup voice of the kit.
    pub(super) fn bend(&mut self, cents: f32) {
        let at = self.at;
        let cents = if cents.is_nan() {
            0.0
        } else {
            cents.clamp(-4800.0, 4800.0)
        };
        if self.lives[at].bend == cents {
            return;
        }
        self.lives[at].bend = cents;
        let (live, t) = (&self.lives[at], &self.tunings[at]);
        for (v, _) in self.pool.iter_mut() {
            let p = usize::from(v.pad);
            if v.active && usize::from(v.kit) == at && p < PADS && v.note < LEVEL_NOTE {
                retarget(v, &live.row(p), &live.program_of(v, t));
            }
        }
    }

    /// The pitch wheel's bend on kit `kit`'s keygroups, in cents (0 out of
    /// range).
    pub fn kit_bend(&self, kit: usize) -> f32 {
        self.lives.get(kit).map_or(0.0, |l| l.bend)
    }

    /// Pad `pad`'s live `field`, or `None` where the pad's own applies.
    /// Kit 0's.
    pub fn pad_live_value(&self, pad: u8, field: PadField) -> Option<f32> {
        self.kit_pad_live_value(0, pad, field)
    }

    /// [`Engine::pad_live_value`] of kit `kit`'s pad.
    pub fn kit_pad_live_value(&self, kit: usize, pad: u8, field: PadField) -> Option<f32> {
        let x = self.lives.get(kit)?.pads.get(usize::from(pad))?[field as usize];
        (!x.is_nan()).then_some(x)
    }

    /// Pad `pad`'s step lock on `field` ([`Event::PadLock`]), if one holds.
    /// Kit 0's.
    pub fn pad_lock_value(&self, pad: u8, field: PadField) -> Option<f32> {
        self.kit_pad_lock_value(0, pad, field)
    }

    /// [`Engine::pad_lock_value`] of kit `kit`'s pad.
    pub fn kit_pad_lock_value(&self, kit: usize, pad: u8, field: PadField) -> Option<f32> {
        let x = self.lives.get(kit)?.locks.get(usize::from(pad))?[field as usize];
        (!x.is_nan()).then_some(x)
    }

    /// The XY pad's hand on kit `kit`'s pad `pad`'s `field`
    /// ([`Event::PadHand`]), if it holds one.
    pub fn kit_pad_hand_value(&self, kit: usize, pad: u8, field: PadField) -> Option<f32> {
        let x = self.lives.get(kit)?.hands.get(usize::from(pad))?[field as usize];
        (!x.is_nan()).then_some(x)
    }

    /// The program's offset on `field` (0 when none). Kit 0's.
    pub fn program_live_value(&self, field: ProgramField) -> f32 {
        self.kit_program_live_value(0, field)
    }

    /// [`Engine::program_live_value`] of kit `kit` (0 out of range).
    pub fn kit_program_live_value(&self, kit: usize, field: ProgramField) -> f32 {
        self.lives
            .get(kit)
            .map_or(0.0, |l| l.program[field as usize])
    }
}
