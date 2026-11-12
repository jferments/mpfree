//! CV out: the CV/gate jacks follow the tracks and the transport, as the
//! project's CV outputs say (`MidiSetup::cv`, the I/O view's CV section).
//! Step M6 of docs/dev/design/midi-io.md.
//!
//! **State-changing on the MPC.** The values go to the controller's cable 1
//! ("Private") as control-change pairs on channel 16, the way Akai's app
//! sets the jacks (`mpfree_midi::cv_wire`, from the owner's read-only
//! captures of 2026-11-12). Nothing is stored on the MPC. Only those
//! control changes are sent, never SysEx. The app opens CV out only when
//! started with `mpfree app --cv-out`, until a device round with a meter
//! has checked it.
//!
//! - Only CV 1 to 4 ([`CHECKED_OUTS`]) are sent: Akai's app was seen
//!   setting their pairs (CC 0 to 7). CV 5 to 8 follow the pattern (CC 8
//!   to 15) but were never seen, so they wait, and the I/O view says so
//!   on their rows.
//! - Full scale is taken as 5 V ([`MAX_VOLTS`]). INFERENCE: five octaves
//!   of Akai's own pitch values; no meter has been on a jack yet.
//! - The sound loop calls [`CvOut::sync`] once a period (it takes the
//!   outputs' jobs when they change, and shows the volts),
//!   [`CvOut::clock`] with the transport's period (clock, run and reset),
//!   [`CvOut::send`] with each note of a track (CV follows a track; the
//!   track still plays where it plays), and [`CvOut::period_end`], which
//!   hands the period's changes to a writer thread
//!   ([`crate::port_writer`]), each due when its frame is heard. None of
//!   them allocates, waits or writes to the port.
//! - Each change is its two control changes, written together. Several
//!   changes of one output on one frame send only the last.
//! - The app's button lights share the Private port. While CV out is
//!   open they go through its writer ([`CvOut::light`]), so a light's
//!   bytes never land inside a CV pair.
//! - When it opens, CV 1 to 4 go to their job's voltage (0 V with no job);
//!   when it closes, all four go to 0 V, so no gate is left up.

use std::io::{self, Write};
use std::sync::TryLockError;
use std::time::{Duration, Instant};

use mpfree_midi::cv::{CvAssign, CvChange, CvEngine, CV_OUTS};
use mpfree_midi::cv_wire;
use mpfree_midi::output::MAX_TRACKS;
use mpfree_midi::setup::MAX_VOLTS;
use mpfree_midi_ui::{IoModel, Shared};
use mpfree_play::{Event, Out};
use mpfree_transport::{Block, Mark, State, Tick, PPQ};

use crate::midi_out::{Chords, NoteIs};
use crate::port_writer::{Wire, Writer, STALL};

/// CV 1 to 4: the outputs whose control changes were seen.
pub const CHECKED_OUTS: usize = 4;
/// Changes one period keeps; past this (less room for each output's last
/// value) only each output's last value is sent.
const CHANGES: usize = 128;

/// One period's changes, in the order made.
struct Changes {
    list: [CvChange; CHANGES],
    n: usize,
    over: bool,
}

impl Changes {
    fn push(&mut self, c: CvChange) {
        if self.n < CHANGES - CV_OUTS {
            self.list[self.n] = c;
            self.n += 1;
        } else {
            self.over = true;
        }
    }

    /// By frame, keeping the order made within a frame (no allocation).
    fn sort(&mut self) {
        let l = &mut self.list[..self.n];
        for i in 1..l.len() {
            let mut j = i;
            while j > 0 && l[j - 1].offset > l[j].offset {
                l.swap(j - 1, j);
                j -= 1;
            }
        }
    }

    /// Whether a later change of the same output on the same frame
    /// replaces change `i` (sorted).
    fn replaced(&self, i: usize) -> bool {
        let c = self.list[i];
        self.list[i + 1..self.n]
            .iter()
            .take_while(|d| d.offset == c.offset)
            .any(|d| d.out == c.out)
    }

    fn clear(&mut self) {
        self.n = 0;
        self.over = false;
    }
}

/// The CV/gate jacks, from the sound loop.
pub struct CvOut {
    engine: CvEngine,
    chords: Chords,
    rate: u32,
    lead: Duration,
    writer: Option<Writer>,
    changes: Changes,
    /// The clock outputs' pulses per quarter note, each once.
    clocks: [u16; CV_OUTS],
    n_clocks: usize,
    applied_gen: Option<u64>,
    shown_outs: Option<u8>,
    shown_volts: [f32; CV_OUTS],
    /// Periods whose settings update was skipped because the screen held
    /// the model.
    busy: u64,
    /// Changes (and lights) dropped because the writer's queue was full.
    full: u64,
    /// Changes handed to the writer.
    queued: u64,
    /// Periods with more than [`CHANGES`] changes.
    overran: u64,
    /// The port stopped, once the log has said so.
    lost: bool,
    /// The engine knows the song plays (run is up).
    playing: bool,
}

impl CvOut {
    /// Closed: nothing is sent until [`CvOut::open`]. `lead` is how long
    /// after a period is made its sound is heard (the sound card's buffer).
    pub fn new(rate: u32, lead: Duration) -> CvOut {
        CvOut {
            engine: CvEngine::new(rate),
            chords: Chords::new(),
            rate: rate.max(1),
            lead,
            writer: None,
            changes: Changes {
                list: [CvChange {
                    offset: 0,
                    out: 0,
                    volts: 0.0,
                }; CHANGES],
                n: 0,
                over: false,
            },
            clocks: [0; CV_OUTS],
            n_clocks: 0,
            applied_gen: None,
            shown_outs: None,
            shown_volts: [0.0; CV_OUTS],
            busy: 0,
            full: 0,
            queued: 0,
            overran: 0,
            lost: false,
            playing: false,
        }
    }

    /// Starts the writer thread on the Private port (`w`, a copy of the
    /// app's own). `started` is the app's start, for the log's times. The
    /// next [`CvOut::sync`] sets CV 1 to 4 to their jobs' voltages.
    pub fn open<W: Write + Send + 'static>(&mut self, w: W, started: Instant) -> io::Result<()> {
        self.writer = Some(Writer::start(w, "cv-out", "CV out", describe, started)?);
        self.engine = CvEngine::new(self.rate);
        self.chords.clear();
        self.changes.clear();
        self.applied_gen = None;
        self.lost = false;
        self.playing = false;
        Ok(())
    }

    #[cfg(test)]
    pub fn is_open(&self) -> bool {
        self.writer.is_some()
    }

    /// Once a period: takes the outputs' jobs when they changed, tells the
    /// I/O view which outputs are sent, and shows each output's volts.
    /// Never waits for the screen.
    pub fn sync(&mut self, model: &Shared) {
        match model.try_lock() {
            Ok(mut m) => self.apply(&mut m),
            Err(TryLockError::Poisoned(e)) => self.apply(&mut e.into_inner()),
            Err(TryLockError::WouldBlock) => self.busy += 1,
        }
    }

    fn apply(&mut self, m: &mut IoModel) {
        let open = self.writer.is_some();
        let outs = if open { CHECKED_OUTS as u8 } else { 0 };
        if self.shown_outs != Some(outs) {
            m.live.built.cv_outs = outs;
            self.shown_outs = Some(outs);
        }
        if open && self.applied_gen != Some(m.gen) {
            // The first time, every output: one with no job goes to 0 V.
            let all = self.applied_gen.is_none();
            self.applied_gen = Some(m.gen);
            self.n_clocks = 0;
            for (o, &a) in m.setup.cv.iter().enumerate() {
                if all || a != self.engine.assign(o) {
                    let changes = &mut self.changes;
                    self.engine.set_assign(o, a, 0, &mut |c| changes.push(c));
                }
                if let CvAssign::Clock { ppqn, .. } = a {
                    if ppqn > 0 && !self.clocks[..self.n_clocks].contains(&ppqn) {
                        self.clocks[self.n_clocks] = ppqn;
                        self.n_clocks += 1;
                    }
                }
            }
        }
        let volts = if open {
            self.engine.volts()
        } else {
            [0.0; CV_OUTS]
        };
        if volts != self.shown_volts {
            m.live.cv_volts = volts;
            self.shown_volts = volts;
        }
    }

    /// One of a track's events, as MIDI Out reads it (`is`: what its note
    /// number is). Pitch, gate, trigger and velocity outputs of that track
    /// follow it; the event still plays wherever the track plays. A panic
    /// (`AllOff`) lets every gate fall.
    pub fn send(&mut self, o: &Out, is: NoteIs) {
        if self.writer.is_none() {
            return;
        }
        let (engine, changes) = (&mut self.engine, &mut self.changes);
        match o.event {
            Event::AllOff => {
                self.chords.clear();
                engine.release_all(o.offset, &mut |c| changes.push(c));
            }
            Event::Lock { .. } | Event::Lane { .. } | Event::Unlock { .. } => {}
            _ => {
                let t = usize::from(o.track);
                if t >= MAX_TRACKS {
                    return;
                }
                self.chords.messages(t, o.event, is, |m| {
                    engine.track_msg(t as u8, m, o.offset, &mut |c| changes.push(c));
                });
            }
        }
    }

    /// The transport's period: run follows play, reset pulses when play
    /// starts from the top, and each clock output pulses on its lines
    /// while the song plays (not during a count-in). Opened while the song
    /// plays, run joins at once (with no reset). Call it once a period
    /// after `TimeHost::advance`.
    pub fn clock(&mut self, block: &Block) {
        if self.writer.is_none() {
            return;
        }
        let (engine, changes) = (&mut self.engine, &mut self.changes);
        let mut push = |c: CvChange| changes.push(c);
        for m in block.markers() {
            match m.mark {
                Mark::Started { tick } => {
                    engine.started(tick == 0, m.offset, &mut push);
                    self.playing = true;
                }
                Mark::Stopped { .. } => {
                    engine.stopped(m.offset, &mut push);
                    self.playing = false;
                }
                _ => {}
            }
        }
        for span in block.spans() {
            if !matches!(span.state, State::Playing | State::Recording) {
                continue;
            }
            if !self.playing {
                engine.started(false, span.offset, &mut push);
                self.playing = true;
            }
            for &ppqn in &self.clocks[..self.n_clocks] {
                let step = Tick::from(PPQ / u32::from(ppqn));
                for (_, offset) in span.lines(step) {
                    engine.clock(ppqn, offset, &mut push);
                }
            }
        }
    }

    /// After the notes: the period's changes to CV 1 to 4, in frame order,
    /// go to the writer, each due when its frame is heard. `t0` is when
    /// the period was made (the loop's start); the sound is heard `lead`
    /// later.
    pub fn period_end(&mut self, frames: u32, t0: Instant) {
        let Some(w) = &self.writer else {
            self.changes.clear();
            return;
        };
        let changes = &mut self.changes;
        self.engine.end_period(frames, &mut |c| changes.push(c));
        if changes.over {
            // Some were lost: each output's value now goes last.
            self.overran += 1;
            let volts = self.engine.volts();
            for (o, &v) in volts.iter().enumerate() {
                changes.list[changes.n] = CvChange {
                    offset: frames.saturating_sub(1),
                    out: o as u8,
                    volts: v,
                };
                changes.n += 1;
            }
        }
        changes.sort();
        let at = t0 + self.lead;
        let rate = f64::from(self.rate);
        for i in 0..changes.n {
            let c = changes.list[i];
            if usize::from(c.out) >= CHECKED_OUTS || changes.replaced(i) {
                continue;
            }
            let counts = cv_wire::counts(c.volts, MAX_VOLTS);
            let Some([hi, lo]) = cv_wire::encode(c.out, counts) else {
                continue;
            };
            let pair = [hi[0], hi[1], hi[2], lo[0], lo[1], lo[2]];
            let due = at + Duration::from_secs_f64(f64::from(c.offset) / rate);
            if w.send(Wire::new(due, &pair)) {
                self.queued += 1;
            } else {
                self.full += 1;
            }
        }
        changes.clear();
    }

    /// A button light for the Private port. True when CV out's writer took
    /// it (CV out is open), so the bytes go out between CV pairs; false
    /// leaves it to the caller.
    pub fn light(&mut self, msg: &[u8]) -> bool {
        let Some(w) = &self.writer else {
            return false;
        };
        if !w.send(Wire::new(Instant::now(), msg)) {
            self.full += 1;
        }
        true
    }

    /// The writer's log lines, and whether the port stopped. A port that
    /// stopped is let go: the I/O view says CV out is off.
    pub fn tick(&mut self, log: &mut dyn FnMut(String)) {
        let Some(w) = &self.writer else { return };
        w.log_lines(log);
        if w.gone() {
            self.lost = true;
            log("CV out stopped: the CV/gate jacks are not driven".to_string());
            self.writer = None;
            self.chords.clear();
            self.changes.clear();
        }
    }

    /// Sets CV 1 to 4 to 0 V, lets the writer send that and stops it,
    /// waiting at most a second. True when everything queued was written
    /// or there was nothing to close.
    pub fn close(&mut self) -> bool {
        if self.writer.is_none() {
            return true;
        }
        let changes = &mut self.changes;
        for o in 0..CHECKED_OUTS {
            self.engine
                .set_assign(o, CvAssign::Off, 0, &mut |c| changes.push(c));
        }
        self.period_end(0, Instant::now() - self.lead);
        let done = self.writer.take().is_none_or(Writer::finish);
        self.shown_outs = None;
        done
    }

    /// The run's counts, for the log at exit.
    pub fn summary(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(w) = &self.writer {
            lines.push(format!(
                "CV out: {} writes, {} dropped (writer's queue full), {} dropped (port took \
                 nothing for {} ms); the latest left {:.1} ms after its frame",
                w.written(),
                self.full,
                w.stalled(),
                STALL.as_millis(),
                w.max_late_ms(),
            ));
        } else if self.queued > 0 || self.lost {
            lines.push(format!(
                "CV out: {} changes queued before the port stopped",
                self.queued
            ));
        }
        if self.overran > 0 {
            lines.push(format!(
                "CV out: {} periods had more than {} changes; each output's last value was sent",
                self.overran,
                CHANGES - CV_OUTS
            ));
        }
        if self.busy > 0 {
            lines.push(format!(
                "CV out: {} periods kept the old settings while the screen held them",
                self.busy
            ));
        }
        lines
    }
}

impl Drop for CvOut {
    fn drop(&mut self) {
        self.close();
    }
}

/// "CV 1 = 8192 (2.500 V if full scale is 5 V)", or a button light, as
/// the log shows a write.
fn describe(b: &[u8]) -> String {
    match *b {
        [cv_wire::CV_STATUS, hi_cc, hi, cv_wire::CV_STATUS, lo_cc, lo]
            if hi_cc % 2 == 0 && lo_cc == hi_cc + 1 =>
        {
            let counts = cv_wire::decode(hi, lo);
            let volts = f32::from(counts) / f32::from(cv_wire::FULL_SCALE) * MAX_VOLTS;
            format!(
                "CV {} = {counts} ({volts:.3} V if full scale is {MAX_VOLTS} V)",
                hi_cc / 2 + 1
            )
        }
        [0xB0, note, value] => format!("button light {note} = {value}"),
        _ => "not a CV pair".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_midi::cv::{Priority, Scale};
    use mpfree_midi::setup::MidiSetup;
    use mpfree_transport::{Command, Transport};
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
        fn writes(&self) -> Vec<Vec<u8>> {
            self.0.lock().unwrap().iter().map(|w| w.1.clone()).collect()
        }
    }

    fn model(cv: &[(usize, CvAssign)]) -> Shared {
        let mut m = IoModel {
            setup: MidiSetup::default(),
            ..IoModel::default()
        };
        for &(o, a) in cv {
            m.setup.cv[o] = a;
        }
        Arc::new(Mutex::new(m))
    }

    fn open() -> (CvOut, Sink) {
        let mut cv = CvOut::new(RATE, Duration::ZERO);
        let sink = Sink::default();
        cv.open(sink.clone(), Instant::now()).unwrap();
        (cv, sink)
    }

    const PITCH: CvAssign = CvAssign::Pitch {
        track: 0,
        scale: Scale::VoltsPerOctave(1.0),
        zero_note: 36,
        bend_semis: 2,
        priority: Priority::Last,
    };
    const GATE: CvAssign = CvAssign::Gate {
        track: 0,
        volts: 5.0,
        retrigger_ms: 0.0,
    };

    fn ev(track: u16, offset: u32, event: Event) -> Out {
        Out {
            offset,
            frame: u64::from(offset),
            track,
            dest: Default::default(),
            event,
        }
    }

    fn on(note: u8) -> Event {
        Event::NoteOn {
            note,
            velocity: 100,
        }
    }

    /// The pair that sets output `out` to `counts`.
    fn pair(out: u8, counts: u16) -> Vec<u8> {
        let [h, l] = cv_wire::encode(out, counts).unwrap();
        [h, l].concat()
    }

    /// Until the writer has written `n`.
    fn wait_for(cv: &CvOut, n: u64) {
        let give_up = Instant::now() + Duration::from_secs(2);
        while cv.writer.as_ref().is_some_and(|w| w.written() < n) && Instant::now() < give_up {
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// A track's notes as Akai's app sends them: C3 (48) is a fifth of
    /// full scale (1 V), the gate full scale while held, 0 when let go.
    #[test]
    fn pitch_and_gate_follow_a_track_as_akai_sends_them() {
        let (mut cv, sink) = open();
        let m = model(&[(0, PITCH), (1, GATE)]);
        cv.sync(&m);
        assert_eq!(m.lock().unwrap().live.built.cv_outs, 4);
        cv.period_end(PERIOD, Instant::now());
        cv.send(&ev(0, 0, on(48)), NoteIs::Note);
        cv.send(&ev(0, 64, Event::NoteOff { note: 48 }), NoteIs::Note);
        cv.period_end(PERIOD, Instant::now());
        assert!(cv.close());
        let w = sink.writes();
        // At open, all four go to their job's voltage (0 V here); then
        // C3's pitch and the gate; at close, all four to 0 V again.
        let mut want = vec![pair(0, 0), pair(1, 0), pair(2, 0), pair(3, 0)];
        // Akai's app truncates C3 to 3276; MPFree rounds 1 V to 3277.
        assert_eq!(cv_wire::akai_pitch_counts(48), 3276);
        want.push(pair(0, 3277));
        want.push(pair(1, cv_wire::FULL_SCALE));
        want.push(pair(1, 0));
        want.push(pair(0, 0));
        assert_eq!(w, want);
    }

    /// CV 5 to 8 are never sent, though the engine follows them and the
    /// I/O view shows their volts.
    #[test]
    fn unchecked_outputs_are_not_sent() {
        let (mut cv, sink) = open();
        let m = model(&[(4, CvAssign::Fixed { volts: 2.5 })]);
        cv.sync(&m);
        cv.period_end(PERIOD, Instant::now());
        cv.sync(&m);
        assert_eq!(m.lock().unwrap().live.cv_volts[4], 2.5);
        assert!(cv.close());
        assert!(sink
            .writes()
            .iter()
            .all(|w| usize::from(w[1] / 2) < CHECKED_OUTS));
    }

    /// A pad's chord: the last note on one frame sets the pitch, sent once.
    #[test]
    fn one_frame_sends_one_value_an_output() {
        let (mut cv, sink) = open();
        let m = model(&[(0, PITCH)]);
        cv.sync(&m);
        cv.period_end(PERIOD, Instant::now());
        wait_for(&cv, 4);
        cv.send(&ev(0, 5, on(0)), NoteIs::Pad(&[48, 52, 55]));
        cv.period_end(PERIOD, Instant::now());
        assert!(cv.close());
        let w = sink.writes();
        assert_eq!(w[4], pair(0, cv_wire::counts(19.0 / 12.0, MAX_VOLTS)));
        assert_eq!(w.len(), 6, "{w:?}");
    }

    /// Changes leave when their frame is heard: a 5 ms trigger's fall
    /// leaves about 5 ms after its rise.
    #[test]
    fn a_trigger_falls_when_its_time_is_up() {
        let (mut cv, sink) = open();
        let trig = CvAssign::Trigger {
            track: 0,
            note: None,
            volts: 5.0,
            ms: 5.0,
        };
        let m = model(&[(2, trig)]);
        cv.sync(&m);
        cv.period_end(PERIOD, Instant::now());
        wait_for(&cv, 4);
        let t0 = Instant::now();
        cv.send(&ev(0, 0, on(60)), NoteIs::Note);
        for p in 0..4 {
            cv.period_end(
                PERIOD,
                t0 + Duration::from_secs_f64(f64::from(p * PERIOD) / 44_100.0),
            );
        }
        wait_for(&cv, 6);
        let writes = sink.0.lock().unwrap().clone();
        assert_eq!(writes[4].1, pair(2, cv_wire::FULL_SCALE));
        assert_eq!(writes[5].1, pair(2, 0));
        // The fall is never written before it is due, 5 ms after the rise
        // was due (t0, with no lead). How soon after that the writer
        // thread wakes is up to the machine: on a loaded test host the
        // rise itself can leave late, so the two writes' own gap is not
        // checked.
        let fell = writes[5].0 - t0;
        assert!(
            fell >= Duration::from_micros(4_900) && fell < Duration::from_secs(1),
            "{fell:?}"
        );
        assert!(cv.close());
    }

    /// Clock, run and reset follow the transport: a quarter-note clock at
    /// 120 BPM pulses twice a second, run is up while playing.
    #[test]
    fn clock_run_and_reset_follow_the_transport() {
        let (mut cv, sink) = open();
        let m = model(&[
            (
                0,
                CvAssign::Clock {
                    ppqn: 1,
                    volts: 5.0,
                    ms: 5.0,
                },
            ),
            (1, CvAssign::Run { volts: 5.0 }),
            (
                2,
                CvAssign::Reset {
                    volts: 5.0,
                    ms: 5.0,
                },
            ),
        ]);
        let mut t = Transport::new(RATE, 120.0).unwrap();
        let mut block = Block::new();
        cv.sync(&m);
        cv.period_end(PERIOD, Instant::now() - Duration::from_secs(1));
        t.command(t.frame(), Command::Play);
        let periods = RATE / PERIOD; // just under a second
        for _ in 0..periods {
            t.process(PERIOD, &mut block);
            cv.clock(&block);
            cv.period_end(PERIOD, Instant::now() - Duration::from_secs(1));
        }
        t.command(t.frame(), Command::Stop);
        t.process(PERIOD, &mut block);
        cv.clock(&block);
        cv.period_end(PERIOD, Instant::now() - Duration::from_secs(1));
        assert!(cv.close());
        let w = sink.writes();
        let of = |o: u8| -> Vec<u16> {
            w.iter()
                .filter(|p| p[1] == 2 * o)
                .map(|p| cv_wire::decode(p[2], p[5]))
                .collect()
        };
        let full = cv_wire::FULL_SCALE;
        // Beats 1 and 2 in the first second, each up then down; then 0
        // at close (already 0: not sent again).
        assert_eq!(of(0), [0, full, 0, full, 0]);
        assert_eq!(of(1), [0, full, 0]);
        assert_eq!(of(2), [0, full, 0]);
    }

    /// Lights go through the writer while CV out is open, and are left to
    /// the app while it is closed.
    #[test]
    fn lights_share_the_writer_while_open() {
        let mut closed = CvOut::new(RATE, Duration::ZERO);
        assert!(!closed.light(&[0xB0, 3, 127]));
        let (mut cv, sink) = open();
        assert!(cv.light(&[0xB0, 3, 127]));
        assert!(cv.close());
        // Then CV 1 to 4 to 0 V as it closes.
        let w = sink.writes();
        assert_eq!(w[0], [0xB0, 3, 127]);
        assert_eq!(w[1..], [pair(0, 0), pair(1, 0), pair(2, 0), pair(3, 0)]);
    }

    /// A port that fails is let go and the I/O view is told.
    #[test]
    fn a_failed_port_is_let_go_and_said() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("unplugged"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut cv = CvOut::new(RATE, Duration::ZERO);
        cv.open(Broken, Instant::now()).unwrap();
        let m = model(&[(1, GATE)]);
        cv.sync(&m);
        cv.period_end(PERIOD, Instant::now());
        let mut lines = Vec::new();
        let give_up = Instant::now() + Duration::from_secs(2);
        while cv.is_open() && Instant::now() < give_up {
            thread::sleep(Duration::from_millis(2));
            cv.tick(&mut |l| lines.push(l));
        }
        assert!(!cv.is_open());
        cv.sync(&m);
        assert_eq!(m.lock().unwrap().live.built.cv_outs, 0);
        assert!(lines
            .iter()
            .any(|l| l.contains("writing BF 00 00 BF 01 00 failed")));
        assert!(lines.iter().any(|l| l.starts_with("CV out stopped")));
        // The writer stops at the first failure, so how many were queued
        // before it did varies.
        let summary = cv.summary();
        assert_eq!(summary.len(), 1);
        assert!(summary[0].ends_with(" changes queued before the port stopped"));
    }

    #[test]
    fn the_log_names_each_output_and_its_volts() {
        assert_eq!(
            describe(&pair(1, 8192)),
            "CV 2 = 8192 (2.500 V if full scale is 5 V)"
        );
        assert_eq!(describe(&[0xB0, 3, 127]), "button light 3 = 127");
        assert_eq!(describe(&[0x90, 3, 127]), "not a CV pair");
    }
}
