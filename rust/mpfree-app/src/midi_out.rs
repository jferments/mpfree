//! MIDI out: a track whose "Plays on" is the 5-pin MIDI Out jack plays
//! there instead of on the pads. Step M3 of docs/dev/design/midi-io.md.
//!
//! **State-changing on the MPC.** The bytes go to the controller's cable 2
//! ("MIDI Port"), which is the MIDI Out jack (the owner's read-only captures
//! of 2026-11-12, docs/dev/hardware/controller-protocol.md). Nothing is
//! stored on the MPC. Only note, pressure, bank and program messages are
//! ever sent, never SysEx. The app opens the port only when started with
//! `mpfree app --midi-out`, until a device round has checked it.
//!
//! - The sound loop calls [`MidiOut::sync`] once a period before the notes:
//!   it takes the project's track outputs when they change, and sends each
//!   routed track's bank and program when it is set and when the song
//!   starts. [`MidiOut::send`] takes each note of a track that plays out,
//!   and [`MidiOut::period_end`] hands the period's messages on. None of
//!   them allocates, waits or writes to the port.
//! - A writer thread gets each message with the moment its frame is heard
//!   (the period's start plus the sound's lead plus its offset), sleeps
//!   until then and writes it. A message that finds the writer's queue full
//!   is dropped and counted.
//! - `OutputRouter` remembers every note that is on, so a route change, a
//!   stop or a panic ends exactly those notes, and closing the port ends
//!   them before it lets go.
//! - A track set to a port this build can't send to (a USB device: not yet)
//!   or set while MIDI out is off plays on the pads, and the I/O view says
//!   so on its row.
//! - **Clock out** ([`MidiOut::clock`]): with the jacks' "Clock out" set to
//!   While playing or Always, the transport's period (`TimeHost::block`)
//!   becomes MIDI clock through `mpfree_midi_clock::ClockOut`: Start, or
//!   Song Position and Continue, where playback starts; a pulse (F8) every
//!   40 ticks (24 a quarter note) while it plays, following every tempo
//!   change; Stop, Song Position and Continue on a locate or a loop's jump;
//!   Stop when it stops. A count-in sends nothing until playback starts.
//!   Always also sends pulses at the tempo while stopped.

use std::io::{self, Write};
use std::sync::TryLockError;
use std::time::{Duration, Instant};

use mpfree_controller::mpc_one::PAD_NOTES;
use mpfree_midi::msg::Msg;
use mpfree_midi::output::{DinPacer, OutQueue, OutRoute, OutputRouter, Timed, MAX_TRACKS};
use mpfree_midi::port::DIN;
use mpfree_midi::setup::{ClockOut, TrackOut, DIN_NAME};
use mpfree_midi_clock::{ClockOut as Pulses, Messages, PULSE_TICKS};
use mpfree_midi_ui::{IoModel, Shared};
use mpfree_play::{Event, Out};
use mpfree_transport::{Block, Mark, Marker, Span, State};

use crate::port_writer::{Wire, Writer, STALL};

/// Notes one pad can start (a synth pad's chord); more are not sent.
const CHORD: usize = 8;
const PADS: usize = 16;

/// What an event's note number is.
#[derive(Clone, Copy, Debug)]
pub enum NoteIs<'a> {
    /// A MIDI note (the arps' notes while a synth's pad layout plays
    /// notes).
    Note,
    /// A pad, 0 to 15, that plays these notes: a synth pad's notes, or the
    /// MPC's own note for the pad with a kit ([`kit_notes`]). A pad's
    /// note-off and pressure go to the notes its note-on sent.
    Pad(&'a [u8]),
}

/// A kit pad's note on MIDI Out: the MPC's own pad notes (pad 1 is C#2,
/// note 37), as Akai's MPC sends them.
pub fn kit_notes(pad: usize) -> &'static [u8] {
    PAD_NOTES.get(pad).map_or(&[], std::slice::from_ref)
}

/// A track's output as it was taken from the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Applied {
    channel: u8,
    program: Option<u8>,
    bank: Option<u16>,
}

impl Applied {
    /// Only the 5-pin jacks for now; a USB device's output comes later.
    fn of(o: &TrackOut) -> Option<Applied> {
        (o.port == DIN_NAME).then_some(Applied {
            channel: o.channel.min(15),
            program: o.program.map(|p| p.min(127)),
            bank: o.bank.map(|b| b.min(0x3FFF)),
        })
    }
}

/// The notes a pad's note-on sent.
#[derive(Clone, Copy, Debug, Default)]
struct Chord {
    notes: [u8; CHORD],
    len: u8,
}

impl Chord {
    fn notes(&self) -> &[u8] {
        &self.notes[..usize::from(self.len)]
    }
}

/// What each track's pads' note-ons sent, so a pad's note-off and
/// pressure go to the same notes even if its layout changed in between.
/// MIDI Out and CV out each keep one.
#[derive(Clone, Debug)]
pub struct Chords([[Chord; PADS]; MAX_TRACKS]);

impl Default for Chords {
    fn default() -> Self {
        Self::new()
    }
}

impl Chords {
    pub fn new() -> Chords {
        Chords([[Chord::default(); PADS]; MAX_TRACKS])
    }

    /// The note messages (on channel 0) one note event of track `t` makes,
    /// in order, to `f`. A pad hit again ends its earlier notes first, as
    /// the synth does. Velocity 0 sounds as 1, so a note-on is never read
    /// as a note-off. Other events make none.
    pub fn messages(&mut self, t: usize, event: Event, is: NoteIs, mut f: impl FnMut(Msg)) {
        let Some(pads) = self.0.get_mut(t) else {
            return;
        };
        let off = |note| Msg::NoteOff {
            ch: 0,
            note,
            vel: 64,
        };
        match (event, is) {
            (Event::NoteOn { note, velocity }, NoteIs::Note) => f(Msg::NoteOn {
                ch: 0,
                note: note & 0x7F,
                vel: velocity.clamp(1, 127),
            }),
            (Event::NoteOn { note, velocity }, NoteIs::Pad(notes)) => {
                let Some(c) = pads.get_mut(usize::from(note)) else {
                    return;
                };
                for &n in std::mem::take(c).notes() {
                    f(off(n));
                }
                for &n in notes.iter().filter(|&&n| n < 128).take(CHORD) {
                    c.notes[usize::from(c.len)] = n;
                    c.len += 1;
                }
                for &n in c.notes() {
                    f(Msg::NoteOn {
                        ch: 0,
                        note: n,
                        vel: velocity.clamp(1, 127),
                    });
                }
            }
            (Event::NoteOff { note }, NoteIs::Note) => f(off(note & 0x7F)),
            (Event::NoteOff { note }, NoteIs::Pad(_)) => {
                if let Some(c) = pads.get_mut(usize::from(note)).map(std::mem::take) {
                    for &n in c.notes() {
                        f(off(n));
                    }
                }
            }
            (Event::Pressure { note, value }, NoteIs::Note) => f(Msg::PolyPressure {
                ch: 0,
                note: note & 0x7F,
                value,
            }),
            (Event::Pressure { note, value }, NoteIs::Pad(_)) => {
                if let Some(c) = pads.get(usize::from(note)) {
                    for &n in c.notes() {
                        f(Msg::PolyPressure {
                            ch: 0,
                            note: n,
                            value,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    /// Note-offs for every pad of track `t` that is on.
    pub fn end_track(&mut self, t: usize, mut f: impl FnMut(Msg)) {
        let Some(pads) = self.0.get_mut(t) else {
            return;
        };
        for c in pads.iter_mut() {
            for &n in std::mem::take(c).notes() {
                f(Msg::NoteOff {
                    ch: 0,
                    note: n,
                    vel: 64,
                });
            }
        }
    }

    pub fn clear(&mut self) {
        *self = Chords::new();
    }
}

/// Tracks to the MIDI Out jack, from the sound loop.
pub struct MidiOut {
    router: OutputRouter,
    /// The jacks' messages this period (only the 5-pin jacks for now).
    queue: [OutQueue; 1],
    pacer: DinPacer,
    rate: u32,
    lead: Duration,
    writer: Option<Writer>,
    tracks: [Option<Applied>; MAX_TRACKS],
    /// Bank and program to send before this period's notes.
    program_due: [bool; MAX_TRACKS],
    chords: Chords,
    applied_gen: Option<u64>,
    shown_on: Option<bool>,
    rolling: bool,
    /// Periods whose settings update was skipped because the screen held
    /// the model.
    busy: u64,
    /// Messages dropped because the writer's queue was full.
    full: u64,
    /// Messages handed to the writer.
    queued: u64,
    /// Why the port stopped, once the log has said so.
    lost: bool,
    /// The jacks' "Clock out" setting, and the clock it makes.
    clock_mode: ClockOut,
    pulses: Pulses,
    clock_msgs: Messages,
    /// Always, while stopped: frames from this period's start to the next
    /// pulse.
    free_next: f64,
}

impl MidiOut {
    /// Closed: no track is taken until [`MidiOut::open`]. `lead` is how long
    /// after a period is made its sound is heard (the sound card's buffer).
    pub fn new(rate: u32, lead: Duration) -> MidiOut {
        let mut pacer = DinPacer::new(rate);
        // The MPC's controller sends the jack's bytes; MPFree hands it whole
        // messages over USB, so the wire time is estimated without running
        // status.
        pacer.running_status = false;
        MidiOut {
            router: OutputRouter::new(),
            queue: [OutQueue::new()],
            pacer,
            rate: rate.max(1),
            lead,
            writer: None,
            tracks: [None; MAX_TRACKS],
            program_due: [false; MAX_TRACKS],
            chords: Chords::new(),
            applied_gen: None,
            shown_on: None,
            rolling: false,
            busy: 0,
            full: 0,
            queued: 0,
            lost: false,
            clock_mode: ClockOut::Off,
            pulses: Pulses::new(),
            clock_msgs: Messages::new(),
            free_next: 0.0,
        }
    }

    /// Starts the writer thread on the MIDI Out port (`w`). `started` is
    /// the app's start, for the log's times.
    pub fn open<W: Write + Send + 'static>(&mut self, w: W, started: Instant) -> io::Result<()> {
        self.writer = Some(Writer::start(w, "midi-out", "MIDI Out", describe, started)?);
        self.lost = false;
        Ok(())
    }

    #[cfg(test)]
    pub fn is_open(&self) -> bool {
        self.writer.is_some()
    }

    /// Whether `track`'s events go to MIDI Out (and not to the pads).
    pub fn takes(&self, track: u16) -> bool {
        self.writer.is_some()
            && self
                .tracks
                .get(usize::from(track))
                .is_some_and(Option::is_some)
    }

    /// Once a period, before the notes: takes the project's track outputs
    /// when they changed (a changed route first ends the notes on at the
    /// old one) and tells the I/O view whether MIDI out works. Never waits
    /// for the screen. On the song's start each routed track's bank and
    /// program go out ahead of its notes.
    pub fn sync(&mut self, model: &Shared, rolling: bool) {
        match model.try_lock() {
            Ok(mut m) => self.apply(&mut m),
            Err(TryLockError::Poisoned(e)) => self.apply(&mut e.into_inner()),
            Err(TryLockError::WouldBlock) => self.busy += 1,
        }
        if rolling && !self.rolling {
            for t in 0..MAX_TRACKS {
                self.program_due[t] |= self.tracks[t].is_some();
            }
        }
        self.rolling = rolling;
        for t in 0..MAX_TRACKS {
            if std::mem::take(&mut self.program_due[t]) {
                self.program(t);
            }
        }
    }

    fn apply(&mut self, m: &mut IoModel) {
        let on = self.writer.is_some();
        if self.shown_on != Some(on) {
            m.live.built.notes_out = on;
            m.live.built.clock_out = on;
            self.shown_on = Some(on);
        }
        if self.applied_gen == Some(m.gen) {
            return;
        }
        self.applied_gen = Some(m.gen);
        // Found in place: `MidiSetup::port` would make a copy.
        self.clock_mode = m
            .setup
            .ports
            .iter()
            .find(|p| p.name == DIN_NAME)
            .map_or(ClockOut::default(), |p| p.clock_out);
        for t in 0..MAX_TRACKS {
            let want = m.setup.tracks[t].as_ref().and_then(Applied::of);
            if want == self.tracks[t] {
                continue;
            }
            let channel = want.map(|a| a.channel);
            if channel != self.tracks[t].map(|a| a.channel) {
                // Another channel, or none: what is on ends where it
                // started. A new program alone leaves held notes alone.
                self.end_chords(t, 0);
                let route = channel.map(|channel| OutRoute { port: DIN, channel });
                self.router.set_route(t as u8, route, 0, &mut self.queue);
            }
            self.tracks[t] = want;
            self.program_due[t] = want.is_some_and(|a| a.program.is_some() || a.bank.is_some());
        }
    }

    /// Bank select (CC 0, then CC 32) and program change, for what is set.
    fn program(&mut self, t: usize) {
        let Some(a) = self.tracks[t] else { return };
        if let Some(b) = a.bank {
            for (cc, value) in [(0, (b >> 7) as u8), (32, (b & 0x7F) as u8)] {
                self.msg(t, Msg::Cc { ch: 0, cc, value }, 0);
            }
        }
        if let Some(program) = a.program {
            self.msg(t, Msg::Program { ch: 0, program }, 0);
        }
    }

    /// One message from track `t`; the router puts it on the track's
    /// channel.
    fn msg(&mut self, t: usize, m: Msg, offset: u32) {
        self.router.send(t as u8, m, offset, &mut self.queue);
    }

    /// One of the player's events. True when MIDI Out took it, so the pads
    /// don't play it. An `AllOff` (a panic) ends every note on MIDI Out and
    /// is left to the pads too; locks and lanes are left to the mixer.
    pub fn send(&mut self, o: &Out, is: NoteIs) -> bool {
        if o.event == Event::AllOff {
            self.all_off(o.offset);
            return false;
        }
        if !self.takes(o.track) {
            return false;
        }
        if matches!(
            o.event,
            Event::Lock { .. } | Event::Lane { .. } | Event::Unlock { .. }
        ) {
            return false;
        }
        let t = usize::from(o.track);
        let (router, queue) = (&mut self.router, &mut self.queue);
        self.chords.messages(t, o.event, is, |m| {
            router.send(t as u8, m, o.offset, queue);
        });
        true
    }

    fn end_chords(&mut self, t: usize, offset: u32) {
        let (router, queue) = (&mut self.router, &mut self.queue);
        self.chords
            .end_track(t, |m| router.send(t as u8, m, offset, queue));
    }

    /// Ends every note on MIDI Out (a panic, a stop), at `offset`: only the
    /// notes that are on, never 2048 blind note-offs.
    pub fn all_off(&mut self, offset: u32) {
        self.chords.clear();
        self.router.all_off(offset, &mut self.queue);
    }

    /// The transport's period as MIDI clock on the jacks, by their "Clock
    /// out" setting. Call it once a period after `TimeHost::advance`.
    pub fn clock(&mut self, block: &Block) {
        if self.writer.is_none() {
            return;
        }
        self.clock_msgs.clear();
        if self.clock_mode == ClockOut::Off {
            // Turned off while playing: the other device stops too.
            if self.pulses.running() {
                self.pulses.stopped(0, &mut self.clock_msgs);
            }
        } else {
            let markers = block.markers();
            let mut next = 0;
            for span in block.spans() {
                // A marker on a span's first frame happened just before it.
                while next < markers.len() && markers[next].offset <= span.offset {
                    self.mark(&markers[next]);
                    next += 1;
                }
                self.clock_span(span);
            }
            for m in &markers[next..] {
                self.mark(m);
            }
            self.free_next = (self.free_next - f64::from(block.frames)).max(0.0);
        }
        for i in 0..self.clock_msgs.as_slice().len() {
            let m = self.clock_msgs.as_slice()[i];
            self.queue[0].push(Timed::raw(m.offset, m.bytes()));
        }
    }

    fn mark(&mut self, m: &Marker) {
        let out = &mut self.clock_msgs;
        match m.mark {
            Mark::Started { tick } => self.pulses.started(m.offset, tick, out),
            Mark::Stopped { .. } => {
                self.pulses.stopped(m.offset, out);
                // Always: pulses go on from here at the tempo.
                self.free_next = f64::from(m.offset);
            }
            Mark::Located { to, .. } => self.pulses.located(m.offset, to, out),
            Mark::Looped { start, .. } => self.pulses.located(m.offset, start, out),
            // Back to bar 1 while stopped: the other device's position too.
            Mark::Rewound => self.pulses.located(m.offset, 0, out),
            _ => {}
        }
    }

    fn clock_span(&mut self, span: &Span) {
        match span.state {
            State::Playing | State::Recording => {
                // Clock turned on (or the port opened) while playing: the
                // other device joins from the next sixteenth.
                if !self.pulses.running() {
                    let tick = span.ticks().start;
                    self.pulses.started(span.offset, tick, &mut self.clock_msgs);
                }
                for (tick, offset) in span.lines(PULSE_TICKS) {
                    self.pulses.pulse(tick, offset, &mut self.clock_msgs);
                }
            }
            // Nothing until playback starts, when Started comes.
            State::CountIn => {}
            State::Stopped if self.clock_mode == ClockOut::Always && !self.pulses.running() => {
                let every = 60.0 * f64::from(span.rate) / (span.bpm.max(1.0) * 24.0);
                let (start, end) = (f64::from(span.offset), f64::from(span.offset + span.len));
                self.free_next = self.free_next.max(start);
                while self.free_next < end {
                    let offset = self.free_next as u32;
                    self.queue[0].push(Timed::realtime(offset, mpfree_midi_clock::CLOCK));
                    self.free_next += every;
                }
            }
            State::Stopped => {}
        }
    }

    /// After the notes: the period's messages, in frame order, go to the
    /// writer, each due when its frame is heard. `t0` is when the period
    /// was made (the loop's start); the sound is heard `lead` later.
    pub fn period_end(&mut self, frames: u32, t0: Instant) {
        let q = &mut self.queue[0];
        // The clock went in first (`clock` runs before the notes), in the
        // order it must leave.
        q.sort_by_frame();
        let at = t0 + self.lead;
        let rate = f64::from(self.rate);
        let writer = self.writer.as_ref();
        let (mut queued, mut full) = (0u64, 0u64);
        self.pacer.pace(q, frames, |m, _| {
            let Some(w) = writer else { return };
            let due = at + Duration::from_secs_f64(f64::from(m.offset) / rate);
            if w.send(Wire::new(due, m.bytes())) {
                queued += 1;
            } else {
                full += 1;
            }
        });
        q.clear();
        self.queued += queued;
        self.full += full;
    }

    /// The writer's log lines (the first messages sent, failures), and
    /// whether the port stopped. A port that stopped is let go: its tracks
    /// play on the pads again and the I/O view says MIDI out is off.
    pub fn tick(&mut self, log: &mut dyn FnMut(String)) {
        let Some(w) = &self.writer else { return };
        // `gone` first, then the lines: a port that fails between the two
        // still has its failure's line read before the writer goes.
        let gone = w.gone();
        w.log_lines(log);
        if gone {
            self.lost = true;
            log("MIDI Out stopped: tracks set to play on MIDI play on the pads again".to_string());
            self.writer = None;
            // Nothing can reach the port now; forget what was on.
            self.all_off(0);
            self.queue[0].clear();
        }
    }

    /// Ends the notes that are on (and the clock, with Stop), lets the
    /// writer send them and stops it,
    /// waiting at most a second. True when everything queued was written
    /// or there was nothing to close.
    pub fn close(&mut self) -> bool {
        if self.writer.is_none() {
            return true;
        }
        self.all_off(0);
        // The other device stops with us.
        if self.pulses.running() {
            self.clock_msgs.clear();
            self.pulses.stopped(0, &mut self.clock_msgs);
            for i in 0..self.clock_msgs.as_slice().len() {
                let m = self.clock_msgs.as_slice()[i];
                self.queue[0].push(Timed::raw(m.offset, m.bytes()));
            }
        }
        self.period_end(0, Instant::now() - self.lead);
        let done = self.writer.take().is_none_or(Writer::finish);
        self.shown_on = None;
        done
    }

    /// The run's counts, for the log at exit.
    pub fn summary(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(w) = &self.writer {
            lines.push(format!(
                "MIDI Out: {} messages written, {} dropped (writer's queue full), {} dropped \
                 (port took nothing for {} ms); the latest left {:.1} ms after its frame",
                w.written(),
                self.full,
                w.stalled(),
                STALL.as_millis(),
                w.max_late_ms(),
            ));
        } else if self.queued > 0 || self.lost {
            lines.push(format!(
                "MIDI Out: {} messages queued before the port stopped",
                self.queued
            ));
        }
        if self.queued > 0 {
            let fpb = self.pacer.frames_per_byte();
            lines.push(format!(
                "MIDI Out: the 5-pin wire (31250 bit/s, {:.2} ms a byte) would leave a message \
                 at most {:.1} ms after its frame",
                fpb * 1000.0 / f64::from(self.rate),
                self.pacer.stats.max_late * 1000.0 / f64::from(self.rate),
            ));
        }
        if self.router.dropped + self.queue[0].dropped > 0 {
            lines.push(format!(
                "MIDI Out: {} messages dropped (more than {} in one period)",
                self.router.dropped + self.queue[0].dropped,
                mpfree_midi::output::QUEUE
            ));
        }
        if self.busy > 0 {
            lines.push(format!(
                "MIDI Out: {} periods kept the old settings while the screen held them",
                self.busy
            ));
        }
        lines
    }
}

impl Drop for MidiOut {
    fn drop(&mut self) {
        self.close();
    }
}

/// "note on, ch 10, C#2, velocity 100", as the log shows a message.
fn describe(b: &[u8]) -> String {
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let Some(m) = Msg::from_bytes(at(0), at(1), at(2)) else {
        return "not a channel message".to_string();
    };
    let ch = m.channel() + 1;
    let name = mpfree_midi_ui::note_name;
    match m {
        Msg::NoteOn { note, vel, .. } => {
            format!("note on, ch {ch}, {}, velocity {vel}", name(note))
        }
        Msg::NoteOff { note, .. } => format!("note off, ch {ch}, {}", name(note)),
        Msg::PolyPressure { note, value, .. } => {
            format!("pressure, ch {ch}, {}, {value}", name(note))
        }
        Msg::Cc { cc, value, .. } => format!("CC {cc}, ch {ch}, value {value}"),
        Msg::Program { program, .. } => format!("program {}, ch {ch}", u16::from(program) + 1),
        Msg::ChanPressure { value, .. } => format!("channel pressure, ch {ch}, {value}"),
        Msg::PitchBend { value, .. } => format!("pitch bend, ch {ch}, {value}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_midi::setup::MidiSetup;
    use mpfree_play::ALL;
    use std::sync::{Arc, Mutex};
    use std::thread;

    const RATE: u32 = 44_100;
    const PERIOD: u32 = 128;

    /// The port: every write with when it came.
    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Writes>>);

    /// Each write, with when it came.
    type Writes = Vec<(Instant, Vec<u8>)>;

    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().push((Instant::now(), b.to_vec()));
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Sink {
        fn bytes(&self) -> Vec<u8> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .flat_map(|(_, b)| b.clone())
                .collect()
        }
    }

    /// A port that fails, as an unplugged one does.
    struct Broken;

    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from_raw_os_error(19)) // ENODEV
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn model(tracks: &[(usize, TrackOut)]) -> Shared {
        let mut m = IoModel {
            setup: MidiSetup::default(),
            ..IoModel::default()
        };
        for (t, o) in tracks {
            m.setup.tracks[*t] = Some(o.clone());
        }
        m.gen = 1;
        Arc::new(Mutex::new(m))
    }

    fn open(lead: Duration) -> (MidiOut, Sink) {
        let mut mo = MidiOut::new(RATE, lead);
        let sink = Sink::default();
        mo.open(sink.clone(), Instant::now()).unwrap();
        (mo, sink)
    }

    fn ev(track: u16, offset: u32, event: Event) -> Out {
        Out {
            offset,
            frame: u64::from(offset),
            track,
            dest: Default::default(),
            event,
        }
    }

    fn on(note: u8, velocity: u8) -> Event {
        Event::NoteOn { note, velocity }
    }

    /// A kit track on the jacks, channel 10: its pads go out as the MPC's
    /// pad notes and the pads don't play them; another track isn't taken.
    #[test]
    fn a_kit_track_plays_the_mpcs_pad_notes_on_its_channel() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(1, TrackOut::new(DIN_NAME, 9))]);
        mo.sync(&m, false);
        assert!(m.lock().unwrap().live.built.notes_out);
        assert!(mo.takes(1));
        assert!(!mo.takes(0));
        let hit = ev(1, 10, on(0, 100));
        assert!(mo.send(&hit, NoteIs::Pad(kit_notes(0))));
        let other = ev(0, 10, on(0, 100));
        assert!(!mo.send(&other, NoteIs::Pad(kit_notes(0))));
        let off = ev(1, 90, Event::NoteOff { note: 0 });
        assert!(mo.send(&off, NoteIs::Pad(kit_notes(0))));
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        // Pad 1 is C#2, note 37 (0x25), on channel 10.
        assert_eq!(sink.bytes(), [0x99, 0x25, 100, 0x89, 0x25, 64]);
    }

    /// While MIDI out is off, a routed track plays on the pads and the I/O
    /// view is told MIDI out is off.
    #[test]
    fn closed_takes_nothing_and_says_so() {
        let mut mo = MidiOut::new(RATE, Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        m.lock().unwrap().live.built.notes_out = true;
        mo.sync(&m, false);
        assert!(!m.lock().unwrap().live.built.notes_out);
        assert!(!mo.takes(0));
        assert!(!mo.send(&ev(0, 0, on(0, 100)), NoteIs::Pad(kit_notes(0))));
    }

    /// A track set to a USB device plays on the pads: this build sends only
    /// to the 5-pin jacks.
    #[test]
    fn a_usb_port_is_not_taken_yet() {
        let (mut mo, _sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new("Digitone", 0))]);
        mo.sync(&m, false);
        assert!(!mo.takes(0));
    }

    /// A synth pad plays its chord, and its note-off ends the same notes
    /// even when the layout changed in between; pressure reaches each.
    #[test]
    fn a_pad_chord_ends_the_notes_it_started() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        mo.sync(&m, false);
        mo.send(&ev(0, 0, on(3, 90)), NoteIs::Pad(&[60, 64, 67]));
        mo.send(
            &ev(0, 5, Event::Pressure { note: 3, value: 40 }),
            NoteIs::Pad(&[]),
        );
        mo.send(&ev(0, 10, Event::NoteOff { note: 3 }), NoteIs::Pad(&[61]));
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        assert_eq!(
            sink.bytes(),
            [
                0x90, 60, 90, 0x90, 64, 90, 0x90, 67, 90, // the chord
                0xA0, 60, 40, 0xA0, 64, 40, 0xA0, 67, 40, // pressure
                0x80, 60, 64, 0x80, 64, 64, 0x80, 67, 64, // its end
            ]
        );
    }

    /// The arps' notes with a synth layout are MIDI notes; velocity 0
    /// still sounds (as 1), so a note-on is never read as a note-off.
    #[test]
    fn midi_notes_go_as_they_are() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(2, TrackOut::new(DIN_NAME, 1))]);
        mo.sync(&m, false);
        mo.send(&ev(2, 0, on(72, 0)), NoteIs::Note);
        mo.send(&ev(2, 3, Event::NoteOff { note: 72 }), NoteIs::Note);
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        assert_eq!(sink.bytes(), [0x91, 72, 1, 0x81, 72, 64]);
    }

    /// A changed channel ends the notes on at the old one first; notes
    /// later go to the new one.
    #[test]
    fn a_route_change_ends_the_old_notes_there() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        mo.sync(&m, true);
        mo.send(&ev(0, 0, on(1, 100)), NoteIs::Pad(kit_notes(1)));
        mo.period_end(PERIOD, Instant::now());
        {
            let mut g = m.lock().unwrap();
            g.setup.tracks[0] = Some(TrackOut::new(DIN_NAME, 4));
            g.gen += 1;
        }
        mo.sync(&m, true);
        mo.send(&ev(0, 0, on(1, 100)), NoteIs::Pad(kit_notes(1)));
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        assert_eq!(
            sink.bytes(),
            [
                0x90, 0x24, 100, // channel 1
                0x80, 0x24, 64, // ended there by the change
                0x94, 0x24, 100, // channel 5
                0x84, 0x24, 64, // ended by close
            ]
        );
    }

    /// Bank and program go out when set, before the notes, and again when
    /// the song starts; not again while it plays.
    #[test]
    fn bank_and_program_go_when_set_and_at_each_start() {
        let (mut mo, sink) = open(Duration::ZERO);
        let mut o = TrackOut::new(DIN_NAME, 2);
        o.program = Some(4);
        o.bank = Some(129); // MSB 1, LSB 1
        let m = model(&[(0, o)]);
        mo.sync(&m, false);
        mo.period_end(PERIOD, Instant::now());
        let program = [0xB2, 0, 1, 0xB2, 32, 1, 0xC2, 4];
        // The song starts: again, ahead of the first note.
        mo.sync(&m, true);
        mo.send(&ev(0, 0, on(0, 100)), NoteIs::Pad(kit_notes(0)));
        mo.period_end(PERIOD, Instant::now());
        // Still playing: not again.
        mo.sync(&m, true);
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        let mut want = program.to_vec();
        want.extend(program);
        want.extend([0x92, 0x25, 100, 0x82, 0x25, 64]);
        assert_eq!(sink.bytes(), want);
    }

    /// A panic ends exactly the notes that are on, is left to the pads too
    /// (false), and nothing is left on for close to end.
    #[test]
    fn all_off_ends_only_what_is_on() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[
            (0, TrackOut::new(DIN_NAME, 0)),
            (1, TrackOut::new(DIN_NAME, 1)),
        ]);
        mo.sync(&m, true);
        mo.send(&ev(0, 0, on(0, 100)), NoteIs::Pad(kit_notes(0)));
        mo.send(&ev(1, 0, on(64, 100)), NoteIs::Note);
        assert!(!mo.send(&ev(ALL, 50, Event::AllOff), NoteIs::Note));
        mo.period_end(PERIOD, Instant::now());
        assert!(mo.close());
        let b = sink.bytes();
        assert_eq!(b.len(), 12, "{b:02X?}");
        assert_eq!(&b[6..], [0x80, 0x25, 64, 0x81, 64, 64]);
    }

    /// Locks and lanes stay with the mixer.
    #[test]
    fn locks_are_not_taken() {
        let (mut mo, _sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        mo.sync(&m, true);
        let lock = Event::Unlock {
            addr: mpfree_play::mpfree_param::ParamAddr::new(
                mpfree_play::mpfree_param::Owner::Inst(0),
                5,
            ),
        };
        assert!(!mo.send(&ev(0, 0, lock), NoteIs::Note));
    }

    /// Each message leaves when its frame is heard: the period's start,
    /// plus the lead, plus its offset; never before.
    #[test]
    fn messages_leave_when_their_frame_is_heard() {
        let lead = Duration::from_millis(30);
        let (mut mo, sink) = open(lead);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        mo.sync(&m, true);
        let t0 = Instant::now();
        // 4410 frames is 100 ms.
        mo.send(&ev(0, 4410, on(0, 100)), NoteIs::Pad(kit_notes(0)));
        mo.send(&ev(0, 0, on(1, 100)), NoteIs::Pad(kit_notes(1)));
        mo.period_end(8820, t0);
        thread::sleep(Duration::from_millis(250));
        let writes = sink.0.lock().unwrap().clone();
        assert_eq!(writes.len(), 2);
        // In frame order, each no earlier than due.
        assert_eq!(writes[0].1, [0x90, 0x24, 100]);
        assert_eq!(writes[1].1, [0x90, 0x25, 100]);
        assert!(writes[0].0 >= t0 + lead);
        assert!(writes[1].0 >= t0 + lead + Duration::from_millis(100));
        assert!(mo.close());
    }

    /// A port that fails stops the writer; the loop says so, lets the port
    /// go, and the track plays on the pads again.
    #[test]
    fn a_failed_port_is_let_go_and_said() {
        let mut mo = MidiOut::new(RATE, Duration::ZERO);
        mo.open(Broken, Instant::now()).unwrap();
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        mo.sync(&m, true);
        assert!(mo.send(&ev(0, 0, on(0, 100)), NoteIs::Pad(kit_notes(0))));
        mo.period_end(PERIOD, Instant::now());
        let mut lines = Vec::new();
        let give_up = Instant::now() + Duration::from_secs(2);
        while mo.is_open() && Instant::now() < give_up {
            mo.tick(&mut |l| lines.push(l));
            thread::sleep(Duration::from_millis(2));
        }
        assert!(!mo.is_open());
        assert!(
            lines[0].starts_with("MIDI Out: writing 90 25 64 failed"),
            "{lines:?}"
        );
        assert!(lines.last().unwrap().starts_with("MIDI Out stopped"));
        assert!(!mo.takes(0));
        mo.sync(&m, true);
        assert!(!m.lock().unwrap().live.built.notes_out);
    }

    /// The screen holding the model never makes the loop wait.
    #[test]
    fn a_busy_model_is_skipped_and_counted() {
        let (mut mo, _sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 0))]);
        let held = m.lock().unwrap();
        mo.sync(&m, false);
        assert!(!mo.takes(0));
        drop(held);
        mo.sync(&m, false);
        assert!(mo.takes(0));
        assert!(mo.summary().iter().any(|l| l.contains("1 periods kept")));
    }

    /// The log lists the first messages sent, named.
    #[test]
    fn the_log_names_what_was_sent() {
        let (mut mo, _sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 9))]);
        mo.sync(&m, true);
        mo.send(&ev(0, 0, on(0, 100)), NoteIs::Pad(kit_notes(0)));
        mo.period_end(PERIOD, Instant::now());
        let mut lines = Vec::new();
        let give_up = Instant::now() + Duration::from_secs(2);
        while lines.is_empty() && Instant::now() < give_up {
            mo.tick(&mut |l| lines.push(l));
            thread::sleep(Duration::from_millis(2));
        }
        assert!(
            lines[0].ends_with("MIDI Out: 99 25 64 = note on, ch 10, C#2, velocity 100"),
            "{lines:?}"
        );
        assert!(mo.close());
    }

    /// The jacks with "Clock out" set, a transport, and its period.
    struct ClockRig {
        mo: MidiOut,
        sink: Sink,
        m: Shared,
        tr: mpfree_transport::Transport,
        block: Block,
    }

    impl ClockRig {
        fn new(mode: ClockOut) -> ClockRig {
            let (mut mo, sink) = open(Duration::ZERO);
            let m = model(&[]);
            m.lock()
                .unwrap()
                .setup
                .set_port(DIN_NAME, |p| p.clock_out = mode);
            mo.sync(&m, false);
            ClockRig {
                mo,
                sink,
                m,
                tr: mpfree_transport::Transport::new(RATE, 120.0).unwrap(),
                block: Block::new(),
            }
        }

        fn command(&mut self, c: mpfree_transport::Command) {
            self.tr.command(self.tr.frame(), c);
        }

        /// `seconds` of periods; returns the bytes they sent.
        fn run(&mut self, seconds: f64) -> Vec<u8> {
            let periods = (seconds * f64::from(RATE) / f64::from(PERIOD)).round() as u32;
            for _ in 0..periods {
                self.tr.process(PERIOD, &mut self.block);
                self.mo.sync(&self.m, self.tr.state().rolling());
                self.mo.clock(&self.block);
                self.mo.period_end(PERIOD, Instant::now());
            }
            self.flush()
        }

        /// What the writer has written since the last call.
        fn flush(&mut self) -> Vec<u8> {
            let give_up = Instant::now() + Duration::from_secs(2);
            while self
                .mo
                .writer
                .as_ref()
                .is_some_and(|w| w.written() < self.mo.queued)
                && Instant::now() < give_up
            {
                thread::sleep(Duration::from_millis(1));
            }
            let b = self.sink.bytes();
            self.sink.0.lock().unwrap().clear();
            b
        }
    }

    fn pulses(b: &[u8]) -> usize {
        b.iter().filter(|&&x| x == 0xF8).count()
    }

    /// While playing: Start from the top, 24 pulses a quarter note (48 a
    /// second at 120 BPM), Stop; nothing while stopped.
    #[test]
    fn clock_out_while_playing() {
        use mpfree_transport::Command;
        let mut r = ClockRig::new(ClockOut::WhilePlaying);
        assert!(r.m.lock().unwrap().live.built.clock_out);
        assert_eq!(r.run(0.5), []);
        r.command(Command::Play);
        let b = r.run(1.0);
        assert_eq!(b[0], 0xFA, "{b:02X?}");
        assert!(b[1..].iter().all(|&x| x == 0xF8), "{b:02X?}");
        assert!((48..=49).contains(&pulses(&b)), "{}", pulses(&b));
        r.command(Command::Stop);
        assert_eq!(r.run(0.5), [0xFC]);
        assert!(r.mo.close());
    }

    /// Always: pulses at the tempo while stopped too, then Start and
    /// pulses from the top when it plays; close sends Stop.
    #[test]
    fn clock_out_always() {
        use mpfree_transport::Command;
        let mut r = ClockRig::new(ClockOut::Always);
        let b = r.run(1.0);
        assert!(b.iter().all(|&x| x == 0xF8), "{b:02X?}");
        assert!((47..=49).contains(&b.len()), "{}", b.len());
        r.command(Command::Play);
        let b = r.run(0.5);
        assert_eq!(b[0], 0xFA, "{b:02X?}");
        assert!((24..=25).contains(&pulses(&b)));
        assert!(r.mo.close());
        assert_eq!(r.sink.bytes(), [0xFC]);
    }

    /// A locate while playing: Stop, Song Position, Continue, then pulses
    /// from there.
    #[test]
    fn clock_out_follows_a_locate() {
        use mpfree_transport::Command;
        let mut r = ClockRig::new(ClockOut::WhilePlaying);
        r.command(Command::Play);
        r.run(0.25);
        // Bar 2: 16 sixteenths in.
        r.command(Command::Locate(3840));
        let b = r.run(0.1);
        assert_eq!(&b[..5], [0xFC, 0xF2, 16, 0, 0xFB], "{b:02X?}");
        assert!(b[5..].iter().all(|&x| x == 0xF8));
        assert!(pulses(&b) >= 4);
    }

    /// A count-in sends no clock; Start comes when playback starts.
    #[test]
    fn clock_out_waits_for_the_count_in() {
        use mpfree_transport::Command;
        let mut r = ClockRig::new(ClockOut::WhilePlaying);
        r.command(Command::SetCountIn(1));
        r.command(Command::Record);
        // A bar at 120 BPM is 2 s: still counting in.
        assert_eq!(r.run(1.5), []);
        let b = r.run(1.0);
        assert_eq!(b[0], 0xFA, "{b:02X?}");
        assert!(b[1..].iter().all(|&x| x == 0xF8));
    }

    /// Clock turned on while playing joins at the next sixteenth (Song
    /// Position, Continue); turned off while playing, it sends Stop.
    #[test]
    fn clock_out_turned_on_and_off_while_playing() {
        use mpfree_transport::Command;
        let mut r = ClockRig::new(ClockOut::Off);
        r.command(Command::Play);
        assert_eq!(r.run(0.5), []);
        let set = |r: &mut ClockRig, mode| {
            let mut g = r.m.lock().unwrap();
            g.setup.set_port(DIN_NAME, |p| p.clock_out = mode);
            g.gen += 1;
        };
        set(&mut r, ClockOut::WhilePlaying);
        let b = r.run(0.5);
        assert_eq!(b[0], 0xF2, "{b:02X?}");
        assert_eq!(b[3], 0xFB);
        assert!(b[4..].iter().all(|&x| x == 0xF8));
        set(&mut r, ClockOut::Off);
        assert_eq!(r.run(0.5), [0xFC]);
    }

    /// Steady playing on MIDI Out allocates and frees nothing in the sound
    /// loop.
    #[test]
    fn steady_play_never_allocates() {
        let (mut mo, sink) = open(Duration::ZERO);
        let m = model(&[(0, TrackOut::new(DIN_NAME, 9))]);
        mo.sync(&m, true);
        let run = |mo: &mut MidiOut, periods: u32| {
            for i in 0..periods {
                mo.sync(&m, true);
                let pad = (i % 16) as u8;
                mo.send(&ev(0, 0, on(pad, 100)), NoteIs::Pad(kit_notes(pad.into())));
                mo.send(
                    &ev(0, 64, Event::NoteOff { note: pad }),
                    NoteIs::Pad(kit_notes(pad.into())),
                );
                mo.period_end(PERIOD, Instant::now());
            }
        };
        // Warm up (the writer's first wait), then count.
        run(&mut mo, 50);
        thread::sleep(Duration::from_millis(50));
        let ((), calls) = crate::sys::alloc_count::calls(|| run(&mut mo, 1000));
        assert_eq!(calls, 0, "allocations or frees in the sound loop");
        assert!(mo.close());
        assert_eq!(sink.bytes().len(), 1050 * 6);
    }
}
