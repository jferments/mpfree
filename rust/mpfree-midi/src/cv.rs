//! CV/gate: tracks' notes as control voltages for modular and analog gear.
//!
//! The MPC One has four 3.5 mm CV/Gate jacks. Each is TRS, so with a
//! breakout cable it carries two signals: [`CV_OUTS`] = 8 outputs (MPC 3.6
//! user guide: rear panel, item 13, and the specifications page).
//!
//! This is the musical half: which voltage each output should be at, and
//! when. How the MPC makes the voltage is in [`crate::cv_wire`]: Akai's
//! app sends 14-bit CC pairs to the controller (captured 2026-11-12). The
//! One's voltage range is still **unknown** (the guide does not give it
//! and no meter has been on a jack), so nothing here assumes a range; the
//! hardware layer will turn [`CvChange`]s into counts once it is
//! measured.
//!
//! Each output has a [`CvAssign`]: a track's pitch (1 V/oct or Hz/V, with
//! last, lowest or highest note priority), its gate, a trigger, velocity,
//! a CC, an LFO or an envelope ([`crate::cv_mod`]), a clock, run or reset,
//! or a fixed voltage. Pitch keeps its
//! voltage after the gate falls, so release tails stay in tune. A gate
//! that falls and rises on the same frame (a sequence's note ending as
//! the next starts) stays low for [`GATE_GAP_MS`], so the module's
//! envelope starts again. Every output has its own calibration
//! ([`Calibration`]) for tuning against a tuner.
//!
//! Changes come out as [`CvChange`] (frame offset, output, volts), only
//! when an output's voltage changes. Nothing allocates after `new`.

use crate::cv_mod::{self, Env, EnvState, Lfo, LfoState, SongSpan};
use crate::msg::Msg;

/// Signals on the four TRS jacks (tip and ring of each).
pub const CV_OUTS: usize = 8;
/// Keys a mono voice remembers for note priority.
pub const STACK: usize = 16;
/// How long a gate stays low when one note ends on the frame the next
/// starts. Long enough for an envelope to see it after USB's 1 ms frames
/// (a choice, not measured on a module yet).
pub const GATE_GAP_MS: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scale {
    /// The usual Eurorack and Moog scale: `volts` per octave (1.0).
    VoltsPerOctave(f32),
    /// Korg and Yamaha: voltage doubles each octave. `ref_volts` at
    /// `ref_note`.
    HzPerVolt { ref_volts: f32, ref_note: u8 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Priority {
    #[default]
    Last,
    Low,
    High,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum CvAssign {
    #[default]
    Off,
    /// A track's pitch. `zero_note` sounds at 0 V (1 V/oct); `bend_semis`
    /// is the pitch bend range.
    Pitch {
        track: u8,
        scale: Scale,
        zero_note: u8,
        bend_semis: u8,
        priority: Priority,
    },
    /// High (`volts`) while a key of the track is held. With
    /// `retrigger_ms` > 0, a new note while held drops it for that long
    /// so envelopes start again.
    Gate {
        track: u8,
        volts: f32,
        retrigger_ms: f32,
    },
    /// A pulse of `ms` on every note-on of the track (of `note` only, for
    /// a drum module's trigger input).
    Trigger {
        track: u8,
        note: Option<u8>,
        volts: f32,
        ms: f32,
    },
    /// The last note-on's velocity, 0 to `max_volts`.
    Velocity { track: u8, max_volts: f32 },
    /// The track's pressure, 0 to `max_volts`: its last channel pressure
    /// (a keyboard's, or the hardest key's, `crate::expr_out`) or a key's
    /// or pad's own pressure, whichever came last.
    Pressure { track: u8, max_volts: f32 },
    /// A CC of the track, `min_volts` at 0 to `max_volts` at 127.
    Cc {
        track: u8,
        cc: u8,
        min_volts: f32,
        max_volts: f32,
    },
    /// Sweeps on its own, or on the song ([`cv_mod`]).
    Lfo(Lfo),
    /// An envelope of a track's notes ([`cv_mod`]).
    Envelope(Env),
    /// A pulse of `ms` at each clock line ([`CvEngine::clock`]): the app
    /// calls it every `PPQ / ppqn` transport ticks.
    Clock { ppqn: u16, volts: f32, ms: f32 },
    /// High while the transport plays.
    Run { volts: f32 },
    /// A pulse of `ms` when the transport starts from the top.
    Reset { volts: f32, ms: f32 },
    /// Holds this voltage (the CV track's sliders).
    Fixed { volts: f32 },
}

/// Per output: what reaches the jack is `volts * gain + offset`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    pub gain: f32,
    pub offset: f32,
}

impl Default for Calibration {
    fn default() -> Self {
        Calibration {
            gain: 1.0,
            offset: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CvChange {
    pub offset: u32,
    pub out: u8,
    pub volts: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Voice {
    keys: [(u8, u8); STACK],
    len: usize,
    bend: i16,
}

impl Voice {
    fn press(&mut self, note: u8, vel: u8) {
        self.release(note);
        if self.len == STACK {
            self.keys.copy_within(1.., 0);
            self.len -= 1;
        }
        self.keys[self.len] = (note, vel);
        self.len += 1;
    }

    fn release(&mut self, note: u8) {
        if let Some(i) = self.keys[..self.len].iter().position(|k| k.0 == note) {
            self.keys.copy_within(i + 1..self.len, i);
            self.len -= 1;
        }
    }

    fn pick(&self, p: Priority) -> Option<(u8, u8)> {
        let k = &self.keys[..self.len];
        match p {
            Priority::Last => k.last().copied(),
            Priority::Low => k.iter().min_by_key(|k| k.0).copied(),
            Priority::High => k.iter().max_by_key(|k| k.0).copied(),
        }
    }
}

const TRACKS: usize = 16;
/// Notes that move an LFO or an envelope, kept until the period ends (past
/// this they act at once).
const MOD_EVENTS: usize = 64;
/// Parts of a period with their own tempo ([`CvEngine::song`]).
const SONG_SPANS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
enum ModKind {
    On(u8),
    Off,
    Restart,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ModEvent {
    offset: u32,
    out: u8,
    kind: ModKind,
}

const NO_EVENT: ModEvent = ModEvent {
    offset: 0,
    out: 0,
    kind: ModKind::Off,
};

pub struct CvEngine {
    rate: f32,
    assign: [CvAssign; CV_OUTS],
    cal: [Calibration; CV_OUTS],
    voices: [Voice; TRACKS],
    /// The voltage each output was last sent (before calibration).
    level: [f32; CV_OUTS],
    sent: [Option<f32>; CV_OUTS],
    /// The frame (from the period start) when a pulse or a retrigger gap
    /// ends, per output; and the level it ends at.
    until: [Option<(u64, f32)>; CV_OUTS],
    /// The frame (from the engine's start) each gate last fell on.
    fell: [Option<u64>; CV_OUTS],
    /// Frames from the engine's start to this period's start.
    base: u64,
    playing: bool,
    /// LFO and envelope outputs: where each is, the period's notes that
    /// move them (in the order they came), the song's tempo and position
    /// this period, and the last tempo (quarter notes a frame).
    lfo: [LfoState; CV_OUTS],
    env: [EnvState; CV_OUTS],
    events: [ModEvent; MOD_EVENTS],
    n_events: usize,
    spans: [SongSpan; SONG_SPANS],
    n_spans: usize,
    per_frame: f64,
    /// Frames between two workings-out ([`cv_mod::MOD_STEP_MS`]), and the
    /// next frame to work out.
    step: u64,
    grid: u64,
}

impl CvEngine {
    pub fn new(rate: u32) -> CvEngine {
        CvEngine {
            rate: rate as f32,
            assign: [CvAssign::Off; CV_OUTS],
            cal: [Calibration::default(); CV_OUTS],
            voices: [Voice::default(); TRACKS],
            level: [0.0; CV_OUTS],
            sent: [None; CV_OUTS],
            until: [None; CV_OUTS],
            fell: [None; CV_OUTS],
            base: 0,
            playing: false,
            lfo: [LfoState::default(); CV_OUTS],
            env: [EnvState::default(); CV_OUTS],
            events: [NO_EVENT; MOD_EVENTS],
            n_events: 0,
            spans: [SongSpan::default(); SONG_SPANS],
            n_spans: 0,
            // 120 BPM until the song says.
            per_frame: 2.0 / f64::from(rate.max(1)),
            step: ((cv_mod::MOD_STEP_MS * rate as f32 / 1000.0).round() as u64).max(1),
            grid: 0,
        }
    }

    pub fn assign(&self, out: usize) -> CvAssign {
        self.assign.get(out).copied().unwrap_or_default()
    }

    /// What an output does. Its voltage goes to 0 V at `offset`, or to the
    /// fixed voltage, or to where a held note puts it.
    pub fn set_assign(
        &mut self,
        out: usize,
        a: CvAssign,
        offset: u32,
        emit: &mut impl FnMut(CvChange),
    ) {
        if out >= CV_OUTS {
            return;
        }
        let now = self.base + u64::from(offset);
        // What moved on its own until now did so with its old job.
        self.mods_until(now, emit);
        let before = self.assign[out];
        self.assign[out] = a;
        self.until[out] = None;
        self.fell[out] = None;
        match (before, a) {
            // A setting turned: the LFO keeps its place.
            (CvAssign::Lfo(_), CvAssign::Lfo(_)) => return,
            // The same track's envelope keeps going.
            (CvAssign::Envelope(b), CvAssign::Envelope(e)) if b.track == e.track => return,
            (_, CvAssign::Lfo(_)) => {
                self.lfo[out] = LfoState { pos: 0.0, at: now };
                let v = self.mod_value(out, now);
                return self.set(out, v, offset, emit);
            }
            (_, CvAssign::Envelope(_)) => self.env[out] = EnvState::default(),
            _ => {}
        }
        let v = match a {
            CvAssign::Fixed { volts } => volts,
            CvAssign::Run { volts } if self.playing => volts,
            CvAssign::Pitch { track, .. } | CvAssign::Gate { track, .. }
                if usize::from(track) < TRACKS =>
            {
                self.set(out, 0.0, offset, emit);
                return self.track_changed(track, offset, false, emit);
            }
            _ => 0.0,
        };
        self.set(out, v, offset, emit);
    }

    pub fn calibration(&self, out: usize) -> Calibration {
        self.cal.get(out).copied().unwrap_or_default()
    }

    pub fn set_calibration(
        &mut self,
        out: usize,
        c: Calibration,
        offset: u32,
        emit: &mut impl FnMut(CvChange),
    ) {
        if out < CV_OUTS {
            self.cal[out] = c;
            self.sent[out] = None;
            self.set(out, self.level[out], offset, emit);
        }
    }

    /// Voltages now, after calibration.
    pub fn volts(&self) -> [f32; CV_OUTS] {
        std::array::from_fn(|o| self.level[o] * self.cal[o].gain + self.cal[o].offset)
    }

    fn set(&mut self, out: usize, v: f32, offset: u32, emit: &mut impl FnMut(CvChange)) {
        self.level[out] = v;
        let volts = v * self.cal[out].gain + self.cal[out].offset;
        if self.sent[out] != Some(volts) {
            self.sent[out] = Some(volts);
            emit(CvChange {
                offset,
                out: out as u8,
                volts,
            });
        }
    }

    fn pulse(
        &mut self,
        out: usize,
        volts: f32,
        ms: f32,
        offset: u32,
        emit: &mut impl FnMut(CvChange),
    ) {
        self.set(out, volts, offset, emit);
        self.until[out] = Some((self.base + u64::from(offset) + self.frames(ms), 0.0));
    }

    /// Pulses and gaps that end at or before `offset` end first, so a
    /// message later in the period finds them over.
    fn catch_up(&mut self, offset: u32, emit: &mut impl FnMut(CvChange)) {
        let now = self.base + u64::from(offset);
        for o in 0..CV_OUTS {
            if let Some((at, v)) = self.until[o] {
                if at <= now {
                    self.until[o] = None;
                    self.set(o, v, at.saturating_sub(self.base) as u32, emit);
                }
            }
        }
    }

    /// `ms` as frames, at least one.
    fn frames(&self, ms: f32) -> u64 {
        (ms.max(0.0) * self.rate / 1000.0).round().max(1.0) as u64
    }

    fn pitch_volts(scale: Scale, zero: u8, note: u8, bend: f32) -> f32 {
        match scale {
            Scale::VoltsPerOctave(k) => (f32::from(note) - f32::from(zero) + bend) / 12.0 * k,
            Scale::HzPerVolt {
                ref_volts,
                ref_note,
            } => ref_volts * ((f32::from(note) - f32::from(ref_note) + bend) / 12.0).exp2(),
        }
    }

    /// Pitch and gate outputs of `track` follow its held keys.
    /// `new_note`: a key went down while another was held (retrigger).
    fn track_changed(
        &mut self,
        track: u8,
        offset: u32,
        new_note: bool,
        emit: &mut impl FnMut(CvChange),
    ) {
        let voice = self.voices[usize::from(track)];
        for o in 0..CV_OUTS {
            match self.assign[o] {
                CvAssign::Pitch {
                    track: t,
                    scale,
                    zero_note,
                    bend_semis,
                    priority,
                } if t == track => {
                    if let Some((note, _)) = voice.pick(priority) {
                        let bend = f32::from(voice.bend) / 8192.0 * f32::from(bend_semis);
                        self.set(
                            o,
                            Self::pitch_volts(scale, zero_note, note, bend),
                            offset,
                            emit,
                        );
                    }
                }
                CvAssign::Gate {
                    track: t,
                    volts,
                    retrigger_ms,
                } if t == track => {
                    let now = self.base + u64::from(offset);
                    if voice.len == 0 {
                        self.until[o] = None;
                        if self.level[o] > 0.0 {
                            self.fell[o] = Some(now);
                        }
                        self.set(o, 0.0, offset, emit);
                    } else if new_note && retrigger_ms > 0.0 && self.level[o] > 0.0 {
                        self.set(o, 0.0, offset, emit);
                        self.until[o] = Some((now + self.frames(retrigger_ms), volts));
                    } else if self.until[o].is_some() {
                        // Low for a retrigger: it rises when that ends.
                    } else if self.fell[o] == Some(now) {
                        // Ended and started again on this frame.
                        let gap = self.frames(retrigger_ms.max(GATE_GAP_MS));
                        self.until[o] = Some((now + gap, volts));
                    } else {
                        self.set(o, volts, offset, emit);
                    }
                }
                _ => {}
            }
        }
    }

    /// A message from a track (its clip, the pads, the arp, a keyboard),
    /// `offset` frames into the period.
    pub fn track_msg(&mut self, track: u8, msg: Msg, offset: u32, emit: &mut impl FnMut(CvChange)) {
        let t = usize::from(track);
        if t >= TRACKS {
            return;
        }
        self.catch_up(offset, emit);
        match msg {
            Msg::NoteOn { note, vel, .. } => {
                let held = self.voices[t].len > 0;
                self.voices[t].press(note, vel);
                self.track_changed(track, offset, held, emit);
                for o in 0..CV_OUTS {
                    match self.assign[o] {
                        CvAssign::Envelope(e) if e.track == track => {
                            self.mod_event(o, ModKind::On(vel), offset, emit)
                        }
                        CvAssign::Lfo(l) if l.restart == Some(track) => {
                            self.mod_event(o, ModKind::Restart, offset, emit)
                        }
                        CvAssign::Trigger {
                            track: tt,
                            note: n,
                            volts,
                            ms,
                        } if tt == track && n.is_none_or(|n| n == note) => {
                            self.pulse(o, volts, ms, offset, emit)
                        }
                        CvAssign::Velocity {
                            track: tt,
                            max_volts,
                        } if tt == track => {
                            self.set(o, f32::from(vel) / 127.0 * max_volts, offset, emit)
                        }
                        _ => {}
                    }
                }
            }
            Msg::NoteOff { note, .. } => {
                self.voices[t].release(note);
                self.track_changed(track, offset, false, emit);
                if self.voices[t].len == 0 {
                    self.envelopes_off(track, offset, emit);
                }
            }
            Msg::PitchBend { value, .. } => {
                self.voices[t].bend = value;
                self.track_changed(track, offset, false, emit);
            }
            Msg::ChanPressure { value, .. } | Msg::PolyPressure { value, .. } => {
                for o in 0..CV_OUTS {
                    if let CvAssign::Pressure {
                        track: tt,
                        max_volts,
                    } = self.assign[o]
                    {
                        if tt == track {
                            let v = f32::from(value.min(127)) / 127.0 * max_volts;
                            self.set(o, v, offset, emit);
                        }
                    }
                }
            }
            Msg::Cc { cc, value, .. } => {
                for o in 0..CV_OUTS {
                    if let CvAssign::Cc {
                        track: tt,
                        cc: c,
                        min_volts,
                        max_volts,
                    } = self.assign[o]
                    {
                        if tt == track && c == cc {
                            let v = min_volts + (max_volts - min_volts) * f32::from(value) / 127.0;
                            self.set(o, v, offset, emit);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Every key of every track up (stop, panic): gates fall, pitch stays.
    pub fn release_all(&mut self, offset: u32, emit: &mut impl FnMut(CvChange)) {
        self.catch_up(offset, emit);
        for t in 0..TRACKS {
            if self.voices[t].len > 0 {
                self.voices[t].len = 0;
                self.track_changed(t as u8, offset, false, emit);
            }
            self.envelopes_off(t as u8, offset, emit);
        }
    }

    fn envelopes_off(&mut self, track: u8, offset: u32, emit: &mut impl FnMut(CvChange)) {
        for o in 0..CV_OUTS {
            if matches!(self.assign[o], CvAssign::Envelope(e) if e.track == track) {
                self.mod_event(o, ModKind::Off, offset, emit);
            }
        }
    }

    /// The song this period, for LFOs that follow the tempo: from `offset`
    /// for `len` frames, `beat` quarter notes in at `offset`, moving
    /// `per_frame` quarter notes a frame, `rolling` while it plays. The
    /// app calls it for each part of the period with its own tempo
    /// (`Block::spans`) before [`CvEngine::end_period`]; without it the
    /// last tempo holds and LFOs run free.
    pub fn song(&mut self, offset: u32, len: u32, beat: f64, per_frame: f64, rolling: bool) {
        if per_frame.is_finite() && per_frame > 0.0 {
            self.per_frame = per_frame;
        }
        if self.n_spans < SONG_SPANS {
            self.spans[self.n_spans] = SongSpan {
                offset,
                len,
                beat,
                per_frame: self.per_frame,
                rolling,
            };
            self.n_spans += 1;
        }
    }

    /// A note that moves an LFO or an envelope: kept until the period
    /// ends, so its frame falls between the right workings-out even when
    /// the period's clock (later frames) came first.
    fn mod_event(
        &mut self,
        out: usize,
        kind: ModKind,
        offset: u32,
        emit: &mut impl FnMut(CvChange),
    ) {
        let e = ModEvent {
            offset,
            out: out as u8,
            kind,
        };
        if self.n_events < MOD_EVENTS {
            self.events[self.n_events] = e;
            self.n_events += 1;
        } else {
            self.apply_event(e, emit);
        }
    }

    fn apply_event(&mut self, e: ModEvent, emit: &mut impl FnMut(CvChange)) {
        let o = usize::from(e.out);
        let f = self.base + u64::from(e.offset);
        let rate = self.rate;
        match (self.assign[o], e.kind) {
            (CvAssign::Envelope(env), ModKind::On(vel)) => self.env[o].on(f, vel, &env, rate),
            (CvAssign::Envelope(env), ModKind::Off) => self.env[o].off(f, &env, rate),
            (CvAssign::Lfo(_), ModKind::Restart) => {
                // Up to the note first, so a Random level is the cycle's.
                self.mod_value(o, f);
                self.lfo[o].restart(f);
            }
            _ => return,
        }
        let v = self.mod_value(o, f);
        self.set(o, v, e.offset, emit);
    }

    /// An LFO's or envelope's voltage at frame `f` of this period (or
    /// later).
    fn mod_value(&mut self, o: usize, f: u64) -> f32 {
        match self.assign[o] {
            CvAssign::Lfo(l) => {
                let offset = f.saturating_sub(self.base).min(u64::from(u32::MAX)) as u32;
                let span = self.spans[..self.n_spans].iter().find(|s| s.has(offset));
                let per_frame = span.map_or(self.per_frame, |s| s.per_frame);
                let song = span.filter(|s| s.rolling).map(|s| s.beat_at(offset));
                let x = self.lfo[o].value(f, &l, self.rate, per_frame, song, o as u64 + 1);
                cv_mod::lfo_volts(&l, x)
            }
            CvAssign::Envelope(e) => self.env[o].level(f, &e, self.rate) * e.volts,
            _ => self.level[o],
        }
    }

    /// LFO and envelope voltages up to frame `end` (not included): one
    /// every [`cv_mod::MOD_STEP_MS`] on the engine's grid, and the notes
    /// that move them, in frame order. Notes at `end` or later wait.
    fn mods_until(&mut self, end: u64, emit: &mut impl FnMut(CvChange)) {
        let moving = |a: &CvAssign| matches!(a, CvAssign::Lfo(_) | CvAssign::Envelope(_));
        let step = self.step;
        self.grid = self.grid.max(self.base.div_ceil(step) * step);
        if !self.assign.iter().any(moving) {
            self.n_events = 0;
            self.grid = self.grid.max(end.div_ceil(step) * step);
            return;
        }
        // In frame order, keeping the order they came in on one frame.
        let ev = &mut self.events[..self.n_events];
        for i in 1..ev.len() {
            let mut j = i;
            while j > 0 && ev[j - 1].offset > ev[j].offset {
                ev.swap(j - 1, j);
                j -= 1;
            }
        }
        let mut e = 0;
        loop {
            let next = (e < self.n_events).then(|| self.base + u64::from(self.events[e].offset));
            match next {
                Some(f) if f < end && (f <= self.grid || self.grid >= end) => {
                    let ev = self.events[e];
                    self.apply_event(ev, emit);
                    e += 1;
                }
                _ if self.grid < end => {
                    let g = self.grid;
                    for o in 0..CV_OUTS {
                        if moving(&self.assign[o]) {
                            let v = self.mod_value(o, g);
                            self.set(o, v, g.saturating_sub(self.base) as u32, emit);
                        }
                    }
                    self.grid += step;
                }
                _ => break,
            }
        }
        self.events.copy_within(e..self.n_events, 0);
        self.n_events -= e;
    }

    /// A clock line for outputs whose `ppqn` is `ppqn`.
    pub fn clock(&mut self, ppqn: u16, offset: u32, emit: &mut impl FnMut(CvChange)) {
        self.catch_up(offset, emit);
        for o in 0..CV_OUTS {
            if let CvAssign::Clock { ppqn: p, volts, ms } = self.assign[o] {
                if p == ppqn {
                    self.pulse(o, volts, ms, offset, emit);
                }
            }
        }
    }

    /// The transport started; `from_top`: from tick 0 (Reset pulses).
    pub fn started(&mut self, from_top: bool, offset: u32, emit: &mut impl FnMut(CvChange)) {
        self.catch_up(offset, emit);
        self.playing = true;
        for o in 0..CV_OUTS {
            match self.assign[o] {
                CvAssign::Run { volts } => self.set(o, volts, offset, emit),
                CvAssign::Reset { volts, ms } if from_top => self.pulse(o, volts, ms, offset, emit),
                _ => {}
            }
        }
    }

    pub fn stopped(&mut self, offset: u32, emit: &mut impl FnMut(CvChange)) {
        self.catch_up(offset, emit);
        self.playing = false;
        for o in 0..CV_OUTS {
            if let CvAssign::Run { .. } = self.assign[o] {
                self.set(o, 0.0, offset, emit);
            }
        }
    }

    /// Ends the period of `frames`: LFOs and envelopes are worked out
    /// across it, and pulses and retrigger gaps that end in it and were
    /// not caught up by a later message come out (after the period's other
    /// changes, with their own offsets; sort by offset if order matters).
    pub fn end_period(&mut self, frames: u32, emit: &mut impl FnMut(CvChange)) {
        let end = self.base + u64::from(frames);
        self.mods_until(end, emit);
        self.n_events = 0;
        self.n_spans = 0;
        for o in 0..CV_OUTS {
            if let Some((at, v)) = self.until[o] {
                if at < end {
                    self.until[o] = None;
                    let offset = at.saturating_sub(self.base) as u32;
                    self.set(o, v, offset, emit);
                }
            }
        }
        self.base = end;
    }
}
