//! A writer thread for one MIDI output: each message is written when its
//! frame is heard. MIDI out (midi_out.rs) runs one for the 5-pin jacks and
//! one for each USB device's output, and CV out (cv_out.rs) one, so the
//! sound loop never waits on a port.
//!
//! A USB device's output is opened by its writer thread
//! ([`Writer::start_opening`]), so the sound loop never waits for the
//! device either; the thread says in the log whether it opened.
//!
//! The sound loop hands each message on with [`Writer::send`], never
//! waiting: a full queue drops the message and the caller counts it. The
//! thread sleeps until the message is due and writes it whole. A port
//! that takes nothing for [`STALL`] loses that message (counted); a port
//! that fails (unplugged) stops the thread, and [`Writer::gone`] says so.

use std::collections::VecDeque;
use std::io::{self, ErrorKind, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Messages waiting for a writer: over a second of a full DIN wire.
pub const WIRE_QUEUE: usize = 4096;
/// Messages the log lists before it only counts them.
const LISTED: u64 = 64;
/// How long the writer waits for room in the kernel's buffer before it
/// drops a message and counts it.
pub const STALL: Duration = Duration::from_millis(500);
/// The most bytes one write takes (a CV output's two control changes).
const WIRE_BYTES: usize = 6;
/// The clock pulses (F8) whose gaps are measured: a beat's.
const CLOCK_GAPS: usize = 24;
/// A gap between pulses this long is a stop, not a late pulse.
const CLOCK_STOPPED: Duration = Duration::from_secs(1);

/// The writer thread's SCHED_FIFO priority on the MPC: below the audio
/// thread (70) and its lanes (69), above everything at the normal policy,
/// so each message still leaves on its moment.
const WRITER_FIFO: i32 = 60;
/// Whether the writer asks for [`WRITER_FIFO`]: on the MPC's ABI, but not
/// in the unit tests, which CI runs as 32-bit ARM under qemu without the
/// right to raise a priority (the refusal's log line would join theirs).
const ASK_FIFO: bool = crate::sys::IS_DEVICE_ABI && !cfg!(test);

/// One message (or a pair written together), and when its frame is heard.
#[derive(Clone, Copy, Debug)]
pub struct Wire {
    due: Instant,
    bytes: [u8; WIRE_BYTES],
    len: u8,
    /// A port's own clock nudged later or earlier (its page's Nudge): it
    /// waits in a lane of its own, so it neither holds up the notes nor
    /// waits behind them.
    nudged: bool,
}

impl Wire {
    /// `b` past six bytes is cut.
    pub fn new(due: Instant, b: &[u8]) -> Wire {
        let mut bytes = [0; WIRE_BYTES];
        let len = b.len().min(WIRE_BYTES);
        bytes[..len].copy_from_slice(&b[..len]);
        Wire {
            due,
            bytes,
            len: len as u8,
            nudged: false,
        }
    }

    /// The port's clock, nudged: in its own lane (see [`write_loop`]).
    pub fn nudged(due: Instant, b: &[u8]) -> Wire {
        Wire {
            nudged: true,
            ..Wire::new(due, b)
        }
    }

    fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }
}

/// What the writer counts, read by the sound loop for the summary.
#[derive(Debug, Default)]
struct Stats {
    written: AtomicU64,
    /// The most a message left after it was due, in microseconds.
    max_late_us: AtomicU64,
    /// Messages dropped because the port took nothing for [`STALL`].
    stalled: AtomicU64,
    /// The port failed (unplugged, gone): the writer stopped.
    gone: AtomicBool,
    /// The clock as it left ([`ClockGaps`]): the mean gap between pulses
    /// and their spread either side, in microseconds (0: not measured).
    clock_gap_us: AtomicU64,
    clock_spread_us: AtomicU64,
    /// When the last pulse left, in microseconds from the writer's start
    /// plus 1 (0: none yet).
    clock_last_us: AtomicU64,
}

/// The gaps between the clock pulses written, over the last beat.
#[derive(Debug)]
struct ClockGaps {
    last: Option<Instant>,
    gaps_us: [u64; CLOCK_GAPS],
    n: usize,
    at: usize,
}

impl ClockGaps {
    fn new() -> ClockGaps {
        ClockGaps {
            last: None,
            gaps_us: [0; CLOCK_GAPS],
            n: 0,
            at: 0,
        }
    }

    /// A pulse left at `t`: the mean gap and the spread either side of it
    /// (half the widest gap less the narrowest), in microseconds, once
    /// four gaps are in. A gap of [`CLOCK_STOPPED`] starts again.
    fn pulse(&mut self, t: Instant) -> Option<(u64, u64)> {
        if let Some(was) = self.last.replace(t) {
            let gap = t.saturating_duration_since(was);
            if gap >= CLOCK_STOPPED {
                self.n = 0;
                self.at = 0;
                return None;
            }
            self.gaps_us[self.at] = gap.as_micros() as u64;
            self.at = (self.at + 1) % CLOCK_GAPS;
            self.n = (self.n + 1).min(CLOCK_GAPS);
        }
        if self.n < 4 {
            return None;
        }
        let g = &self.gaps_us[..self.n];
        let mean = g.iter().sum::<u64>() / self.n as u64;
        let lo = g.iter().copied().min().unwrap_or(0);
        let hi = g.iter().copied().max().unwrap_or(0);
        Some((mean, (hi - lo) / 2))
    }
}

/// A port's writer thread.
pub struct Writer {
    tx: SyncSender<Wire>,
    log: Receiver<String>,
    thread: JoinHandle<()>,
    stats: Arc<Stats>,
    /// The app's start, which the clock's last pulse counts from.
    started: Instant,
}

impl Writer {
    /// Starts the thread `thread_name` on the port `w`. `label` begins its
    /// log lines ("MIDI Out"), `describe` says what a message is, and
    /// `started` is the app's start, for the log's times.
    pub fn start<W: Write + Send + 'static>(
        w: W,
        thread_name: &str,
        label: &str,
        describe: fn(&[u8]) -> String,
        started: Instant,
    ) -> io::Result<Writer> {
        Writer::spawn(move || Ok(w), false, thread_name, label, describe, started)
    }

    /// As [`Writer::start`], but the thread opens the port itself with
    /// `open` first, so the caller never waits for it, and says in the log
    /// that it opened ("{label}: open"). A port that won't open is said in
    /// the log and the writer is [`Writer::gone`]: what was sent to it is
    /// dropped.
    pub fn start_opening<W: Write + Send + 'static>(
        open: impl FnOnce() -> io::Result<W> + Send + 'static,
        thread_name: &str,
        label: &str,
        describe: fn(&[u8]) -> String,
        started: Instant,
    ) -> io::Result<Writer> {
        Writer::spawn(open, true, thread_name, label, describe, started)
    }

    fn spawn<W: Write + Send + 'static>(
        open: impl FnOnce() -> io::Result<W> + Send + 'static,
        announce: bool,
        thread_name: &str,
        label: &str,
        describe: fn(&[u8]) -> String,
        started: Instant,
    ) -> io::Result<Writer> {
        let (tx, rx) = mpsc::sync_channel(WIRE_QUEUE);
        let (log_tx, log) = mpsc::channel();
        let stats = Arc::new(Stats::default());
        let s = stats.clone();
        let say = Say {
            label: label.to_string(),
            describe,
            started,
            log: log_tx,
        };
        let thread = thread::Builder::new()
            .name(thread_name.into())
            .spawn(move || {
                // Off the sound's cores; a thread the audio thread starts
                // no longer inherits its SCHED_FIFO (sys::realtime), so on
                // the MPC it asks for its own, just below the lanes.
                let _ = crate::cores::background();
                if ASK_FIFO {
                    if let Err(e) = mpfree_lanes::os::fifo(WRITER_FIFO) {
                        let _ = say.log.send(format!(
                            "{}: the writer runs at normal priority ({e})",
                            say.label
                        ));
                    }
                }
                match open() {
                    Ok(w) => {
                        if announce {
                            let _ = say.log.send(format!("{}: open", say.label));
                        }
                        write_loop(w, rx, &s, &say)
                    }
                    Err(e) => {
                        let _ = say
                            .log
                            .send(format!("{}: could not open its output: {e}", say.label));
                        // After the line (Release): see `Writer::gone`.
                        s.gone.store(true, Ordering::Release);
                    }
                }
            })?;
        Ok(Writer {
            tx,
            log,
            thread,
            stats,
            started,
        })
    }

    /// Hands a message on without waiting. False when the queue is full
    /// (or the thread stopped): the message is dropped.
    pub fn send(&self, w: Wire) -> bool {
        self.tx.try_send(w).is_ok()
    }

    /// The thread's log lines since the last call.
    pub fn log_lines(&self, f: &mut dyn FnMut(String)) {
        while let Ok(l) = self.log.try_recv() {
            f(l);
        }
    }

    /// The port failed and the thread stopped. Acquire pairs with the
    /// thread's Release store, which comes after its last log line: a
    /// caller that sees `gone` and then calls [`Writer::log_lines`] gets
    /// the failure's line too.
    pub fn gone(&self) -> bool {
        self.stats.gone.load(Ordering::Acquire)
    }

    pub fn written(&self) -> u64 {
        self.stats.written.load(Ordering::Relaxed)
    }

    /// Messages dropped because the port took nothing for [`STALL`].
    pub fn stalled(&self) -> u64 {
        self.stats.stalled.load(Ordering::Relaxed)
    }

    /// MPFree's clock as it left over the last beat: the gap between
    /// pulses and their spread either side, in ms. None until four gaps
    /// are measured, or a second after the last pulse.
    pub fn clock_measured(&self) -> Option<(f32, f32)> {
        let last = self.stats.clock_last_us.load(Ordering::Relaxed);
        let gap = self.stats.clock_gap_us.load(Ordering::Relaxed);
        if last == 0 || gap == 0 {
            return None;
        }
        let at = self.started + Duration::from_micros(last - 1);
        if Instant::now().saturating_duration_since(at) >= CLOCK_STOPPED {
            return None;
        }
        let spread = self.stats.clock_spread_us.load(Ordering::Relaxed);
        Some((gap as f32 / 1000.0, spread as f32 / 1000.0))
    }

    /// The most a message left after it was due, in milliseconds.
    pub fn max_late_ms(&self) -> f64 {
        self.stats.max_late_us.load(Ordering::Relaxed) as f64 / 1000.0
    }

    /// Lets the thread write what is queued and stop, without waiting: for
    /// the sound loop, when Prefs turns sending off. The thread closes its
    /// port when it ends.
    pub fn let_go(self) {
        drop(self.tx);
    }

    /// Lets the thread write what is queued and stop, waiting at most a
    /// second. True when it finished.
    pub fn finish(self) -> bool {
        drop(self.tx);
        let give_up = Instant::now() + Duration::from_secs(1);
        while !self.thread.is_finished() && Instant::now() < give_up {
            thread::sleep(Duration::from_millis(2));
        }
        let done = self.thread.is_finished();
        if done {
            let _ = self.thread.join();
        }
        done
    }
}

/// A writer Prefs switched off, kept until what it was handed has left:
/// switched on again soon, the port is taken back rather than opened a
/// second time (a rawmidi output may refuse that while the old file is
/// open), and a button light still goes out between CV pairs.
pub struct Closing {
    writer: Writer,
    until: Instant,
}

/// How long a switched-off writer is kept after its last message is due.
const LINGER: Duration = Duration::from_millis(250);

impl Closing {
    /// `lead`: how long after now its last message is due.
    pub fn new(writer: Writer, lead: Duration) -> Closing {
        Closing {
            writer,
            until: Instant::now() + lead + LINGER,
        }
    }

    pub fn writer(&self) -> &Writer {
        &self.writer
    }

    /// Its log lines; true when it is time to let it go (its messages
    /// have left, or the port failed).
    pub fn tick(&self, log: &mut dyn FnMut(String)) -> bool {
        let gone = self.writer.gone();
        self.writer.log_lines(log);
        gone || Instant::now() >= self.until
    }

    /// The writer again, for a switch turned back on; none if its port
    /// failed meanwhile.
    pub fn take_back(self) -> Option<Writer> {
        if self.writer.gone() {
            self.writer.let_go();
            None
        } else {
            Some(self.writer)
        }
    }

    pub fn let_go(self) {
        self.writer.let_go();
    }

    pub fn finish(self) -> bool {
        self.writer.finish()
    }
}

/// How the thread's log lines read.
struct Say {
    label: String,
    describe: fn(&[u8]) -> String,
    started: Instant,
    log: Sender<String>,
}

/// The messages waiting to be written, in two lanes, each in the order
/// sent: a nudged clock's ([`Wire::nudged`]) and the rest. Of the two
/// fronts, the one due first goes first (the one sent first on a tie), so a clock
/// nudged later never holds up a note due before it, nor waits behind one
/// due after it. Each lane keeps its own order whatever its times say, so
/// a note's end never passes its start, nor Stop the pulses before it.
struct Lanes {
    /// Each with how many were sent before it.
    rest: VecDeque<(u64, Wire)>,
    clock: VecDeque<(u64, Wire)>,
    sent: u64,
}

impl Lanes {
    fn new() -> Lanes {
        Lanes {
            rest: VecDeque::with_capacity(WIRE_QUEUE),
            clock: VecDeque::new(),
            sent: 0,
        }
    }

    fn len(&self) -> usize {
        self.rest.len() + self.clock.len()
    }

    fn push(&mut self, m: Wire) {
        let n = self.sent;
        self.sent += 1;
        if m.nudged {
            self.clock.push_back((n, m));
        } else {
            self.rest.push_back((n, m));
        }
    }

    /// When the next message is due.
    fn first_due(&self) -> Option<Instant> {
        match (self.rest.front(), self.clock.front()) {
            (Some(r), Some(c)) => Some(r.1.due.min(c.1.due)),
            (r, c) => r.or(c).map(|m| m.1.due),
        }
    }

    fn pop(&mut self) -> Option<Wire> {
        let clock_first = match (self.rest.front(), self.clock.front()) {
            (Some(r), Some(c)) => (c.1.due, c.0) < (r.1.due, r.0),
            (r, _) => r.is_none(),
        };
        if clock_first {
            self.clock.pop_front().map(|m| m.1)
        } else {
            self.rest.pop_front().map(|m| m.1)
        }
    }
}

/// The writer thread: each message when it is due. Without a nudged
/// clock, in the order sent; with one, see [`Lanes`].
fn write_loop<W: Write>(mut w: W, rx: Receiver<Wire>, stats: &Stats, say: &Say) {
    let label = say.label.as_str();
    let mut listed = 0u64;
    let mut gaps = ClockGaps::new();
    // At most the channel's size between them, so a full channel still
    // refuses (and the sender counts) rather than these growing.
    let mut lanes = Lanes::new();
    let mut open = true;
    loop {
        // What has come takes its place first, until the next is due.
        if open && lanes.len() < WIRE_QUEUE {
            let got = match lanes.first_due() {
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
                Some(due) => match due.checked_duration_since(Instant::now()) {
                    Some(wait) if !wait.is_zero() => rx.recv_timeout(wait),
                    _ => rx.try_recv().map_err(|e| match e {
                        TryRecvError::Empty => RecvTimeoutError::Timeout,
                        TryRecvError::Disconnected => RecvTimeoutError::Disconnected,
                    }),
                },
            };
            match got {
                Ok(m) => {
                    lanes.push(m);
                    continue;
                }
                Err(RecvTimeoutError::Timeout) => {}
                // Let go: what waits still leaves, each when it is due.
                Err(RecvTimeoutError::Disconnected) => open = false,
            }
        }
        let Some(due) = lanes.first_due() else {
            if open {
                continue;
            }
            return;
        };
        let now = Instant::now();
        if due > now {
            if open && lanes.len() < WIRE_QUEUE {
                continue;
            }
            thread::sleep(due - now);
        }
        let Some(m) = lanes.pop() else {
            continue;
        };
        let b = m.bytes();
        match write_msg(&mut w, b) {
            Ok(()) => {
                let now = Instant::now();
                let late = now.saturating_duration_since(m.due);
                stats.written.fetch_add(1, Ordering::Relaxed);
                if b == [0xF8] {
                    let (gap, spread) = gaps.pulse(now).unwrap_or((0, 0));
                    stats.clock_gap_us.store(gap, Ordering::Relaxed);
                    stats.clock_spread_us.store(spread, Ordering::Relaxed);
                    let since = now.saturating_duration_since(say.started).as_micros() as u64;
                    stats.clock_last_us.store(since + 1, Ordering::Relaxed);
                }
                stats
                    .max_late_us
                    .fetch_max(late.as_micros() as u64, Ordering::Relaxed);
                if listed < LISTED {
                    listed += 1;
                    let t = m.due.saturating_duration_since(say.started).as_secs_f64();
                    let _ = say.log.send(format!(
                        "+{t:.3} s {label}: {} = {}{}",
                        hex(b),
                        (say.describe)(b),
                        if listed == LISTED {
                            " (more are counted, not listed)"
                        } else {
                            ""
                        }
                    ));
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                let n = stats.stalled.fetch_add(1, Ordering::Relaxed);
                if n == 0 {
                    let _ = say.log.send(format!(
                        "{label}: the port took nothing for {} ms; {} dropped (more are counted)",
                        STALL.as_millis(),
                        hex(b)
                    ));
                }
            }
            Err(e) => {
                let _ = say
                    .log
                    .send(format!("{label}: writing {} failed: {e}", hex(b)));
                // After the line (Release): see `Writer::gone`.
                stats.gone.store(true, Ordering::Release);
                return;
            }
        }
    }
}

/// One whole message to a non-blocking port, waiting up to [`STALL`] for
/// room in the kernel's buffer.
fn write_msg<W: Write>(w: &mut W, b: &[u8]) -> io::Result<()> {
    let give_up = Instant::now() + STALL;
    let mut done = 0;
    while done < b.len() {
        match w.write(&b[done..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    ErrorKind::WriteZero,
                    "the MIDI port took nothing",
                ))
            }
            Ok(n) => done += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) if e.kind() == ErrorKind::WouldBlock && Instant::now() < give_up => {
                thread::sleep(Duration::from_millis(1))
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<Vec<u8>>>>);

    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().push(b.to_vec());
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn what(b: &[u8]) -> String {
        format!("{} bytes", b.len())
    }

    #[test]
    fn a_pair_is_written_in_one_write_when_due() {
        let sink = Sink::default();
        let t0 = Instant::now();
        let w = Writer::start(sink.clone(), "test-writer", "Test", what, t0).unwrap();
        let due = t0 + Duration::from_millis(20);
        assert!(w.send(Wire::new(due, &[0xBF, 0, 64, 0xBF, 1, 0])));
        assert!(w.finish());
        assert!(Instant::now() >= due);
        assert_eq!(*sink.0.lock().unwrap(), vec![vec![0xBF, 0, 64, 0xBF, 1, 0]]);
    }

    /// A nudged clock waits in its own lane: a pulse nudged later doesn't
    /// hold up the notes sent after it but due first, and one nudged
    /// earlier doesn't wait behind notes due after it. Each lane keeps the
    /// order it was sent in.
    #[test]
    fn a_nudged_clock_neither_holds_up_the_notes_nor_waits_for_them() {
        let sink = Sink::default();
        let t0 = Instant::now();
        let w = Writer::start(sink.clone(), "test-writer", "Test", what, t0).unwrap();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        // Later: the pulse is sent first, the notes are due first.
        assert!(w.send(Wire::nudged(ms(700), &[0xF8])));
        assert!(w.send(Wire::new(ms(20), &[0x90, 60, 100])));
        assert!(w.send(Wire::new(ms(10), &[0x80, 60, 0])));
        assert!(w.send(Wire::nudged(ms(650), &[0xFC])));
        let give_up = t0 + Duration::from_secs(2);
        while sink.0.lock().unwrap().len() < 2 && Instant::now() < give_up {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(Instant::now() < ms(650), "the notes waited for the pulse");
        // Earlier: the pulse goes before a note sent ahead of it.
        assert!(w.send(Wire::new(ms(850), &[0x90, 62, 100])));
        assert!(w.send(Wire::nudged(ms(800), &[0xF8])));
        assert!(w.finish());
        assert_eq!(
            *sink.0.lock().unwrap(),
            vec![
                // Its end said due before its start, the lane's order holds.
                vec![0x90, 60, 100],
                vec![0x80, 60, 0],
                // So does Stop, said due before the pulse.
                vec![0xF8],
                vec![0xFC],
                vec![0xF8],
                vec![0x90, 62, 100],
            ]
        );
    }

    /// The clock's gap and spread over the last beat of pulses; a second
    /// without one starts again.
    #[test]
    fn the_clocks_gaps_are_measured_as_it_leaves() {
        let mut g = ClockGaps::new();
        let t0 = Instant::now();
        let at = |us: u64| t0 + Duration::from_micros(us);
        // 20.8 ms apart, one pulse 0.6 ms late.
        let times = [0, 20_800, 41_600, 63_000, 83_200, 104_000];
        let got: Vec<_> = times.iter().map(|&t| g.pulse(at(t))).collect();
        assert_eq!(got[..4], [None, None, None, None]);
        // Gaps 20.8, 20.8, 21.4, 20.2: mean 20.8, spread (21.4 - 20.2) / 2.
        assert_eq!(got[4], Some((20_800, 600)));
        assert_eq!(got[5], Some((20_800, 600)));
        // Stopped a second: none until four more.
        assert_eq!(g.pulse(at(1_200_000)), None);
        assert_eq!(g.pulse(at(1_220_000)), None);
        // A whole beat on, only that beat's gaps count.
        let mut g = ClockGaps::new();
        for i in 0..100u64 {
            g.pulse(at(i * 10_000 + if i == 3 { 2_000 } else { 0 }));
        }
        assert_eq!(g.pulse(at(1_000_000)), Some((10_000, 0)));
        // A writer says so, and forgets it a second after the last.
        let sink = Sink::default();
        let w = Writer::start(sink, "test-clock", "Test", what, t0).unwrap();
        assert_eq!(w.clock_measured(), None);
        let now = Instant::now();
        for i in 0..6u32 {
            assert!(w.send(Wire::new(now + Duration::from_millis(5) * i, &[0xF8])));
        }
        let give_up = Instant::now() + Duration::from_secs(2);
        while w.written() < 6 && Instant::now() < give_up {
            thread::sleep(Duration::from_millis(2));
        }
        let (gap, spread) = w.clock_measured().expect("measured");
        assert!((4.0..8.0).contains(&gap), "{gap}");
        assert!(spread < 3.0, "{spread}");
        assert!(w.finish());
    }

    #[test]
    fn the_log_uses_the_label_and_description() {
        let t0 = Instant::now();
        let w = Writer::start(Sink::default(), "test-writer", "CV out", what, t0).unwrap();
        w.send(Wire::new(t0, &[0xBF, 2, 127]));
        // Wait for the line itself: the count goes up before the line is
        // sent, so a writer thread put off between the two would leave
        // nothing to read yet.
        let mut lines = Vec::new();
        let give_up = Instant::now() + Duration::from_secs(2);
        while lines.is_empty() && Instant::now() < give_up {
            w.log_lines(&mut |l| lines.push(l));
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(w.written(), 1);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].ends_with(" s CV out: BF 02 7F = 3 bytes"),
            "{}",
            lines[0]
        );
        assert!(w.finish());
    }

    /// A writer that opens its port says so, then writes; one whose port
    /// won't open says why and is gone.
    #[test]
    fn a_writer_opens_its_port_or_says_why_not() {
        let t0 = Instant::now();
        let sink = Sink::default();
        let s = sink.clone();
        let w = Writer::start_opening(
            move || Ok(s),
            "test-writer",
            "MIDI out to Digitone",
            what,
            t0,
        )
        .unwrap();
        w.send(Wire::new(t0, &[0x90, 60, 100]));
        assert!(w.finish());
        assert_eq!(*sink.0.lock().unwrap(), vec![vec![0x90, 60, 100]]);
        let w = Writer::start_opening(
            || Err::<Sink, _>(io::Error::from_raw_os_error(16)), // EBUSY
            "test-writer",
            "MIDI out to Digitone",
            what,
            t0,
        )
        .unwrap();
        let give_up = Instant::now() + Duration::from_secs(2);
        while !w.gone() && Instant::now() < give_up {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(w.gone());
        let mut lines = Vec::new();
        w.log_lines(&mut |l| lines.push(l));
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].starts_with("MIDI out to Digitone: could not open its output: "),
            "{}",
            lines[0]
        );
        assert!(w.finish());
    }
}
