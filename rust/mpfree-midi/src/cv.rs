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
//! a CC, a clock, run or reset, or a fixed voltage. Pitch keeps its
//! voltage after the gate falls, so release tails stay in tune. Every
//! output has its own calibration ([`Calibration`]) for tuning against a
//! tuner.
//!
//! Changes come out as [`CvChange`] (frame offset, output, volts), only
//! when an output's voltage changes. Nothing allocates after `new`.

use crate::msg::Msg;

/// Signals on the four TRS jacks (tip and ring of each).
pub const CV_OUTS: usize = 8;
/// Keys a mono voice remembers for note priority.
pub const STACK: usize = 16;

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
    /// A CC of the track, `min_volts` at 0 to `max_volts` at 127.
    Cc {
        track: u8,
        cc: u8,
        min_volts: f32,
        max_volts: f32,
    },
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
    /// Frames from the engine's start to this period's start.
    base: u64,
    playing: bool,
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
            base: 0,
            playing: false,
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
        self.assign[out] = a;
        self.until[out] = None;
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
        let len = (ms.max(0.0) * self.rate / 1000.0).round().max(1.0) as u64;
        self.until[out] = Some((self.base + u64::from(offset) + len, 0.0));
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
                    if voice.len == 0 {
                        self.until[o] = None;
                        self.set(o, 0.0, offset, emit);
                    } else if new_note && retrigger_ms > 0.0 && self.level[o] > 0.0 {
                        self.set(o, 0.0, offset, emit);
                        let len = (retrigger_ms * self.rate / 1000.0).round().max(1.0) as u64;
                        self.until[o] = Some((self.base + u64::from(offset) + len, volts));
                    } else if self.until[o].is_none() {
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
        match msg {
            Msg::NoteOn { note, vel, .. } => {
                let held = self.voices[t].len > 0;
                self.voices[t].press(note, vel);
                self.track_changed(track, offset, held, emit);
                for o in 0..CV_OUTS {
                    match self.assign[o] {
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
            }
            Msg::PitchBend { value, .. } => {
                self.voices[t].bend = value;
                self.track_changed(track, offset, false, emit);
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
        for t in 0..TRACKS {
            if self.voices[t].len > 0 {
                self.voices[t].len = 0;
                self.track_changed(t as u8, offset, false, emit);
            }
        }
    }

    /// A clock line for outputs whose `ppqn` is `ppqn`.
    pub fn clock(&mut self, ppqn: u16, offset: u32, emit: &mut impl FnMut(CvChange)) {
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
        self.playing = false;
        for o in 0..CV_OUTS {
            if let CvAssign::Run { .. } = self.assign[o] {
                self.set(o, 0.0, offset, emit);
            }
        }
    }

    /// Ends the period of `frames`: pulses and retrigger gaps that end in
    /// it come out (after the period's other changes, with their own
    /// offsets; sort by offset if order matters).
    pub fn end_period(&mut self, frames: u32, emit: &mut impl FnMut(CvChange)) {
        let end = self.base + u64::from(frames);
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
