//! MIDI clock for MPFree: the bytes, not the ports.
//!
//! - [`ClockOut`] turns the transport into MIDI clock: a pulse (`F8`) every
//!   [`PULSE_TICKS`] ticks while it plays (24 per quarter note), Start
//!   (`FA`) from the top, Song Position (`F2`) and Continue (`FB`) from
//!   anywhere else, Stop (`FC`), and Song Position when it moves while
//!   stopped.
//! - [`ClockIn`] reads another device's clock: its tempo (averaged over the
//!   last beat of pulses), its position in ticks and whether it plays.
//!   What MPFree's transport does with that is the time engine's call.
//!
//! Ticks are the transport's: [`PPQ`] (960) per quarter note, so a pulse is
//! 40 ticks and a sixteenth (one Song Position step) is 240. Nothing here
//! opens a port or allocates, so both can run in the audio thread. Which
//! MIDI port the MPC uses, and whether it may, is decided elsewhere.
#![forbid(unsafe_code)]

/// Transport ticks per quarter note (the same as `mpfree_transport::PPQ`).
pub const PPQ: i64 = 960;
/// MIDI clock pulses per quarter note.
pub const PPQN: i64 = 24;
/// Ticks per pulse.
pub const PULSE_TICKS: i64 = PPQ / PPQN;
/// Ticks per Song Position step (a sixteenth note, 6 pulses).
pub const SIXTEENTH_TICKS: i64 = PPQ / 4;
/// The largest Song Position (14 bits of sixteenths).
pub const MAX_SONG_POSITION: u16 = 0x3FFF;

pub const CLOCK: u8 = 0xF8;
pub const START: u8 = 0xFA;
pub const CONTINUE: u8 = 0xFB;
pub const STOP: u8 = 0xFC;
pub const SONG_POSITION: u8 = 0xF2;

/// One MIDI message of one to three bytes, `offset` frames into the period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    pub offset: u32,
    bytes: [u8; 3],
    len: u8,
}

impl Message {
    pub const fn one(offset: u32, status: u8) -> Self {
        Self { offset, bytes: [status, 0, 0], len: 1 }
    }

    /// Song Position: `sixteenths` from the start, at most 16383.
    pub fn song_position(offset: u32, sixteenths: u16) -> Self {
        let s = sixteenths.min(MAX_SONG_POSITION);
        Self { offset, bytes: [SONG_POSITION, (s & 0x7F) as u8, (s >> 7) as u8], len: 3 }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }
}

/// Room for one period's clock messages.
pub const MAX_MESSAGES: usize = 32;

/// The messages of one period, in order. Make it once and reuse it.
#[derive(Clone, Debug)]
pub struct Messages {
    buf: [Message; MAX_MESSAGES],
    len: usize,
    /// Messages that did not fit, since it was made.
    pub dropped: u64,
}

impl Default for Messages {
    fn default() -> Self {
        Self::new()
    }
}

impl Messages {
    pub const fn new() -> Self {
        Self { buf: [Message::one(0, CLOCK); MAX_MESSAGES], len: 0, dropped: 0 }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn as_slice(&self) -> &[Message] {
        &self.buf[..self.len]
    }

    fn push(&mut self, m: Message) {
        if self.len < MAX_MESSAGES {
            self.buf[self.len] = m;
            self.len += 1;
        } else {
            self.dropped += 1;
        }
    }
}

/// The first multiple of `step` at or after `t`.
fn ceil_to(t: i64, step: i64) -> i64 {
    t.div_euclid(step) * step + if t.rem_euclid(step) == 0 { 0 } else { step }
}

/// The Song Position for a tick: the next sixteenth at or after it (the
/// first pulse after Continue plays that sixteenth), never below 0.
fn sixteenths(tick: i64) -> u16 {
    (ceil_to(tick.max(0), SIXTEENTH_TICKS) / SIXTEENTH_TICKS).min(i64::from(MAX_SONG_POSITION)) as u16
}

/// MIDI clock out of the transport.
///
/// Each period, in time order: call [`ClockOut::started`],
/// [`ClockOut::stopped`] and [`ClockOut::located`] for the transport's
/// marks, and [`ClockOut::pulse`] for every multiple of [`PULSE_TICKS`]
/// the period plays (`mpfree_transport::Span::lines(PULSE_TICKS)` gives
/// them with their offsets).
#[derive(Clone, Debug, Default)]
pub struct ClockOut {
    running: bool,
    /// The tick of the next pulse to send.
    next: i64,
    /// Started before the top (a count-in): Start goes out at tick 0.
    start_pending: bool,
}

impl ClockOut {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn running(&self) -> bool {
        self.running
    }

    /// The transport starts playing from `tick`: Start from the top (at
    /// tick 0 when it starts before it, after a count-in), otherwise Song
    /// Position and Continue, with pulses from the next sixteenth.
    pub fn started(&mut self, offset: u32, tick: i64, out: &mut Messages) {
        self.running = true;
        if tick < 0 {
            self.start_pending = true;
            self.next = 0;
        } else if tick == 0 {
            self.start_pending = false;
            self.next = 0;
            out.push(Message::one(offset, START));
        } else {
            self.start_pending = false;
            let s = sixteenths(tick);
            self.next = i64::from(s) * SIXTEENTH_TICKS;
            out.push(Message::song_position(offset, s));
            out.push(Message::one(offset, CONTINUE));
        }
    }

    /// The transport stopped.
    pub fn stopped(&mut self, offset: u32, out: &mut Messages) {
        if self.running && !self.start_pending {
            out.push(Message::one(offset, STOP));
        }
        self.running = false;
        self.start_pending = false;
    }

    /// The position jumped to `tick` (a locate, or a loop wrapping). While
    /// playing: Stop, Song Position, Continue, so the other device follows
    /// the jump; while stopped, Song Position only.
    pub fn located(&mut self, offset: u32, tick: i64, out: &mut Messages) {
        if !self.running {
            out.push(Message::song_position(offset, sixteenths(tick)));
            return;
        }
        if tick < 0 {
            // Back into a count-in: Start again at the top.
            self.stopped(offset, out);
            self.started(offset, tick, out);
            return;
        }
        if !self.start_pending {
            out.push(Message::one(offset, STOP));
        }
        self.start_pending = false;
        let s = sixteenths(tick);
        self.next = i64::from(s) * SIXTEENTH_TICKS;
        out.push(Message::song_position(offset, s));
        out.push(Message::one(offset, CONTINUE));
    }

    /// A pulse line the transport plays: `tick` is a multiple of
    /// [`PULSE_TICKS`] at `offset`. Sends the pulse when it is due.
    pub fn pulse(&mut self, tick: i64, offset: u32, out: &mut Messages) {
        if !self.running || tick < self.next {
            return;
        }
        if self.start_pending {
            if tick != 0 {
                return;
            }
            self.start_pending = false;
            out.push(Message::one(offset, START));
        }
        out.push(Message::one(offset, CLOCK));
        self.next = tick + PULSE_TICKS;
    }
}

/// What a byte of incoming clock meant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InEvent {
    /// A pulse, at this position (ticks from the top).
    Pulse { tick: i64 },
    /// Start: play from the top at the next pulse.
    Started,
    /// Continue: play from the Song Position at the next pulse.
    Continued { tick: i64 },
    Stopped,
    /// A Song Position: where Continue will play from.
    Located { tick: i64 },
}

/// The other device's clock, as [`ClockIn`] reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InState {
    /// Averaged over up to the last beat of pulses; None until two pulses
    /// came, or after the pulses stop for [`ClockIn::LOST_SECONDS`].
    pub bpm: Option<f64>,
    pub playing: bool,
    /// The position of the last pulse, in ticks from the top (where the
    /// next one plays from, before the first pulse after Start or
    /// Continue).
    pub tick: i64,
    /// The frame the last pulse came at.
    pub pulse_frame: Option<u64>,
}

/// Reads MIDI clock from another device. Give it every incoming byte with
/// the frame it arrived at; other messages are skipped.
#[derive(Clone, Debug)]
pub struct ClockIn {
    rate: u32,
    /// Frames of the last pulses, oldest first (a ring of one beat).
    frames: [u64; PPQN as usize + 1],
    count: usize,
    head: usize,
    playing: bool,
    /// Ticks of the next pulse while playing (the Song Position after
    /// Continue, 0 after Start); the last pulse's otherwise.
    next_tick: i64,
    last_tick: i64,
    /// Song Position bytes still to come.
    sp: Option<(u8, Option<u8>)>,
}

impl ClockIn {
    /// Without a pulse for this long, the tempo is unknown again.
    pub const LOST_SECONDS: f64 = 0.5;

    pub fn new(rate: u32) -> Self {
        Self {
            rate: rate.max(1),
            frames: [0; PPQN as usize + 1],
            count: 0,
            head: 0,
            playing: false,
            next_tick: 0,
            last_tick: 0,
            sp: None,
        }
    }

    /// One byte that arrived at `frame`.
    pub fn byte(&mut self, frame: u64, b: u8) -> Option<InEvent> {
        match b {
            CLOCK => Some(self.clock(frame)),
            START => {
                self.playing = true;
                self.next_tick = 0;
                Some(InEvent::Started)
            }
            CONTINUE => {
                self.playing = true;
                Some(InEvent::Continued { tick: self.next_tick })
            }
            STOP => {
                self.playing = false;
                Some(InEvent::Stopped)
            }
            // Other real-time bytes can come in the middle of a message.
            0xF9 | 0xFD..=0xFF => None,
            SONG_POSITION => {
                self.sp = Some((0, None));
                None
            }
            // Any other status ends a Song Position that was coming.
            0x80..=0xF1 | 0xF3..=0xF7 => {
                self.sp = None;
                None
            }
            data => match self.sp {
                Some((0, None)) => {
                    self.sp = Some((1, Some(data)));
                    None
                }
                Some((_, Some(lsb))) => {
                    self.sp = None;
                    let tick = (i64::from(data) << 7 | i64::from(lsb)) * SIXTEENTH_TICKS;
                    self.next_tick = tick;
                    if !self.playing {
                        self.last_tick = tick;
                    }
                    Some(InEvent::Located { tick })
                }
                _ => None,
            },
        }
    }

    /// Every byte of `bytes`, all arrived at `frame`; `f` gets each event.
    pub fn bytes(&mut self, frame: u64, bytes: &[u8], mut f: impl FnMut(InEvent)) {
        for &b in bytes {
            if let Some(e) = self.byte(frame, b) {
                f(e);
            }
        }
    }

    fn clock(&mut self, frame: u64) -> InEvent {
        // A long gap starts the average again.
        if let Some(last) = self.last_frame() {
            if frame.saturating_sub(last) as f64 > Self::LOST_SECONDS * f64::from(self.rate) {
                self.count = 0;
            }
        }
        self.frames[self.head] = frame;
        self.head = (self.head + 1) % self.frames.len();
        self.count = (self.count + 1).min(self.frames.len());
        if self.playing {
            self.last_tick = self.next_tick;
            self.next_tick += PULSE_TICKS;
        }
        InEvent::Pulse { tick: self.last_tick }
    }

    fn last_frame(&self) -> Option<u64> {
        (self.count > 0).then(|| self.frames[(self.head + self.frames.len() - 1) % self.frames.len()])
    }

    /// The clock as of `now` (a frame at or after the last byte).
    pub fn state(&self, now: u64) -> InState {
        let last = self.last_frame();
        let lost = last.is_none_or(|l| now.saturating_sub(l) as f64 > Self::LOST_SECONDS * f64::from(self.rate));
        let bpm = if self.count >= 2 && !lost {
            let n = self.frames.len();
            let first = self.frames[(self.head + n - self.count) % n];
            let per_pulse = (last.unwrap_or(first) - first) as f64 / (self.count - 1) as f64;
            (per_pulse > 0.0).then(|| 60.0 * f64::from(self.rate) / (per_pulse * PPQN as f64))
        } else {
            None
        };
        InState { bpm, playing: self.playing, tick: self.last_tick, pulse_frame: last }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;
    const PERIOD: u64 = 128;

    /// The frame of a tick at a steady tempo from tick `t0` at frame
    /// `f0`, rounded as the transport rounds.
    fn frame_of(f0: u64, t0: i64, bpm: f64, tick: i64) -> u64 {
        let f = (tick - t0) as f64 * 60.0 * f64::from(RATE) / (bpm * PPQ as f64);
        (f0 as i64 + f.round() as i64) as u64
    }

    /// Runs a transport that starts at frame `f0` from tick `t0`, for
    /// `periods`, and returns every message as (absolute frame, bytes).
    fn run(t0: i64, bpm: f64, periods: u64, jump: Option<(u64, i64)>) -> Vec<(u64, Vec<u8>)> {
        let mut c = ClockOut::new();
        let mut out = Messages::new();
        let mut got = Vec::new();
        let (mut f0, mut tick0) = (0u64, t0);
        for p in 0..periods {
            out.clear();
            let start = p * PERIOD;
            if p == 0 {
                c.started(0, t0, &mut out);
            }
            let mut from = start;
            if let Some((at, to)) = jump.filter(|&(at, _)| at >= start && at < start + PERIOD) {
                // Pulses before the jump, then the jump, then pulses after.
                for t in (ceil_to(tick_at(f0, tick0, bpm, from), PULSE_TICKS)..).step_by(PULSE_TICKS as usize) {
                    let f = frame_of(f0, tick0, bpm, t);
                    if f >= at {
                        break;
                    }
                    c.pulse(t, (f - start) as u32, &mut out);
                }
                c.located((at - start) as u32, to, &mut out);
                (f0, tick0, from) = (at, to, at);
            }
            for t in (ceil_to(tick_at(f0, tick0, bpm, from), PULSE_TICKS)..).step_by(PULSE_TICKS as usize) {
                let f = frame_of(f0, tick0, bpm, t);
                if f >= start + PERIOD {
                    break;
                }
                if f >= from {
                    c.pulse(t, (f - start) as u32, &mut out);
                }
            }
            got.extend(out.as_slice().iter().map(|m| (start + u64::from(m.offset), m.bytes().to_vec())));
        }
        got
    }

    fn tick_at(f0: u64, t0: i64, bpm: f64, frame: u64) -> i64 {
        t0 + ((frame - f0) as f64 * bpm * PPQ as f64 / (60.0 * f64::from(RATE))).floor() as i64
    }

    #[test]
    fn from_the_top_start_then_24_pulses_a_beat() {
        let got = run(0, 120.0, 2000, None);
        assert_eq!(got[0], (0, vec![START]));
        assert_eq!(got[1], (0, vec![CLOCK]));
        let pulses: Vec<u64> = got.iter().filter(|(_, b)| b == &[CLOCK]).map(|(f, _)| *f).collect();
        // 120 BPM: a beat is 22050 frames, a pulse 918.75.
        assert_eq!(pulses[24], 22_050);
        assert_eq!(pulses[48], 44_100);
        assert!(pulses.windows(2).all(|w| (918..=919).contains(&(w[1] - w[0]))));
    }

    #[test]
    fn from_elsewhere_song_position_and_continue_on_the_next_sixteenth() {
        // Started at tick 1000 (past the 4th sixteenth, 960): position 5.
        let got = run(1000, 120.0, 200, None);
        assert_eq!(got[0], (0, vec![SONG_POSITION, 5, 0]));
        assert_eq!(got[1], (0, vec![CONTINUE]));
        // The first pulse is tick 1200, 200 ticks in: 2296.875 frames.
        assert_eq!(got[2], (frame_of(0, 1000, 120.0, 1200), vec![CLOCK]));
        assert_eq!(Message::song_position(0, 200).bytes(), &[SONG_POSITION, 200 - 128, 1]);
        assert_eq!(Message::song_position(0, 60_000).bytes(), &[SONG_POSITION, 0x7F, 0x7F]);
    }

    #[test]
    fn a_count_in_sends_start_at_the_top() {
        // One bar of count-in at 120 BPM: tick -3840, frame 88200 is the top.
        let got = run(-3840, 120.0, 800, None);
        assert_eq!(got[0], (88_200, vec![START]));
        assert_eq!(got[1], (88_200, vec![CLOCK]));
        let mut c = ClockOut::new();
        let mut out = Messages::new();
        c.started(0, -960, &mut out);
        c.stopped(5, &mut out);
        assert_eq!(out.as_slice(), &[], "stopped in the count-in: nothing was started, nothing to stop");
    }

    #[test]
    fn a_loop_wrap_stops_positions_and_continues() {
        // Loop back to bar 1 (tick 0) at frame 50000 (the transport's wrap).
        let got = run(0, 120.0, 600, Some((50_000, 0)));
        let i = got.iter().position(|(f, _)| *f == 50_000).unwrap();
        assert_eq!(got[i..i + 4], [(50_000, vec![STOP]), (50_000, vec![SONG_POSITION, 0, 0]), (50_000, vec![CONTINUE]),
                                   (50_000, vec![CLOCK])]);
        assert!(got[..i].iter().all(|(f, _)| *f < 50_000));
    }

    #[test]
    fn stop_and_a_locate_while_stopped() {
        let mut c = ClockOut::new();
        let mut out = Messages::new();
        c.started(0, 0, &mut out);
        c.pulse(0, 0, &mut out);
        c.pulse(0, 1, &mut out);
        c.pulse(PULSE_TICKS, 900, &mut out);
        c.stopped(1000, &mut out);
        c.pulse(2 * PULSE_TICKS, 1800, &mut out);
        c.located(1900, 3840, &mut out);
        let want = [Message::one(0, START), Message::one(0, CLOCK), Message::one(900, CLOCK), Message::one(1000, STOP),
                    Message::song_position(1900, 16)];
        assert_eq!(out.as_slice(), &want, "a pulse sent once; none while stopped");
    }

    #[test]
    fn the_buffer_counts_what_did_not_fit() {
        let mut out = Messages::new();
        for i in 0..MAX_MESSAGES as u32 + 3 {
            out.push(Message::one(i, CLOCK));
        }
        assert_eq!((out.as_slice().len(), out.dropped), (MAX_MESSAGES, 3));
    }

    /// Clock bytes at `bpm` with up to `jitter` frames of noise per pulse.
    fn feed(c: &mut ClockIn, from: u64, pulses: u64, bpm: f64, jitter: u64) -> u64 {
        let per = 60.0 * f64::from(RATE) / (bpm * PPQN as f64);
        let mut seed = 12345u64;
        let mut f = from;
        for i in 0..pulses {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = ((seed >> 33) % (2 * jitter + 1)) as i64 - jitter as i64;
            f = (from as i64 + (i as f64 * per).round() as i64 + j).max(0) as u64;
            c.byte(f, CLOCK);
        }
        f
    }

    #[test]
    fn clock_in_reads_the_tempo_even_with_usb_jitter() {
        let mut c = ClockIn::new(RATE);
        assert_eq!(c.state(0).bpm, None);
        let last = feed(&mut c, 1000, 100, 128.0, 0);
        assert!((c.state(last).bpm.unwrap() - 128.0).abs() < 0.01);
        // About 1 ms of jitter on every pulse.
        let mut c = ClockIn::new(RATE);
        let last = feed(&mut c, 0, 200, 93.5, 44);
        assert!((c.state(last).bpm.unwrap() - 93.5).abs() < 0.5, "{:?}", c.state(last).bpm);
        // The pulses stop: the tempo is unknown again.
        assert_eq!(c.state(last + RATE as u64).bpm, None);
    }

    #[test]
    fn clock_in_follows_start_continue_stop_and_song_position() {
        let mut c = ClockIn::new(RATE);
        let mut ev = Vec::new();
        c.bytes(0, &[START], |e| ev.push(e));
        c.bytes(10, &[CLOCK], |e| ev.push(e));
        c.bytes(20, &[CLOCK], |e| ev.push(e));
        assert_eq!(ev, [InEvent::Started, InEvent::Pulse { tick: 0 }, InEvent::Pulse { tick: 40 }]);
        assert!(c.state(20).playing);
        ev.clear();
        // Stop, then a Song Position of 2 bars (32 sixteenths), split by a
        // real-time byte, then Continue.
        c.bytes(30, &[STOP, SONG_POSITION, 32, 0xFE, 0], |e| ev.push(e));
        assert_eq!(ev, [InEvent::Stopped, InEvent::Located { tick: 7680 }]);
        assert_eq!(c.state(30), InState { bpm: c.state(30).bpm, playing: false, tick: 7680, pulse_frame: Some(20) });
        ev.clear();
        c.bytes(40, &[CONTINUE, CLOCK, CLOCK], |e| ev.push(e));
        assert_eq!(ev, [InEvent::Continued { tick: 7680 }, InEvent::Pulse { tick: 7680 }, InEvent::Pulse { tick: 7720 }]);
        // Pulses while stopped keep the tempo but not the position.
        ev.clear();
        c.bytes(50, &[STOP, CLOCK], |e| ev.push(e));
        assert_eq!(ev, [InEvent::Stopped, InEvent::Pulse { tick: 7720 }]);
        // Notes and other messages are skipped; a status byte cancels a
        // Song Position half read.
        ev.clear();
        c.bytes(60, &[0x90, 60, 100, SONG_POSITION, 5, 0x80, 60, 0], |e| ev.push(e));
        assert_eq!(ev, []);
    }

    #[test]
    fn what_goes_out_comes_back_in() {
        let mut c = ClockIn::new(RATE);
        for (f, b) in run(1000, 140.0, 4000, Some((100_000, 0))) {
            c.bytes(f, &b, |_| {});
        }
        let s = c.state(4000 * PERIOD);
        assert!(s.playing);
        assert!((s.bpm.unwrap() - 140.0).abs() < 0.05, "{:?}", s.bpm);
        // After the wrap to 0 at frame 100000, the position follows the frames.
        let ticks = ((s.pulse_frame.unwrap() - 100_000) as f64 * 140.0 * PPQ as f64 / (60.0 * f64::from(RATE))).round() as i64;
        assert_eq!(s.tick, ticks.div_euclid(PULSE_TICKS) * PULSE_TICKS);
    }
}
