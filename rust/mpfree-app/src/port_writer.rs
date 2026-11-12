//! A writer thread for one of the controller's USB-MIDI outputs: each
//! message is written when its frame is heard. MIDI Out (midi_out.rs) and
//! CV out (cv_out.rs) each run one, so the sound loop never waits on a
//! port.
//!
//! The sound loop hands each message on with [`Writer::send`], never
//! waiting: a full queue drops the message and the caller counts it. The
//! thread sleeps until the message is due and writes it whole. A port
//! that takes nothing for [`STALL`] loses that message (counted); a port
//! that fails (unplugged) stops the thread, and [`Writer::gone`] says so.

use std::io::{self, ErrorKind, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
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

/// One message (or a pair written together), and when its frame is heard.
#[derive(Clone, Copy, Debug)]
pub struct Wire {
    due: Instant,
    bytes: [u8; WIRE_BYTES],
    len: u8,
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
}

/// A port's writer thread.
pub struct Writer {
    tx: SyncSender<Wire>,
    log: Receiver<String>,
    thread: JoinHandle<()>,
    stats: Arc<Stats>,
}

impl Writer {
    /// Starts the thread `thread_name` on the port `w`. `label` begins its
    /// log lines ("MIDI Out"), `describe` says what a message is, and
    /// `started` is the app's start, for the log's times.
    pub fn start<W: Write + Send + 'static>(
        w: W,
        thread_name: &str,
        label: &'static str,
        describe: fn(&[u8]) -> String,
        started: Instant,
    ) -> io::Result<Writer> {
        let (tx, rx) = mpsc::sync_channel(WIRE_QUEUE);
        let (log_tx, log) = mpsc::channel();
        let stats = Arc::new(Stats::default());
        let s = stats.clone();
        let say = Say {
            label,
            describe,
            started,
            log: log_tx,
        };
        let thread = thread::Builder::new()
            .name(thread_name.into())
            .spawn(move || write_loop(w, rx, &s, &say))?;
        Ok(Writer {
            tx,
            log,
            thread,
            stats,
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

    /// The port failed and the thread stopped.
    pub fn gone(&self) -> bool {
        self.stats.gone.load(Ordering::Relaxed)
    }

    pub fn written(&self) -> u64 {
        self.stats.written.load(Ordering::Relaxed)
    }

    /// Messages dropped because the port took nothing for [`STALL`].
    pub fn stalled(&self) -> u64 {
        self.stats.stalled.load(Ordering::Relaxed)
    }

    /// The most a message left after it was due, in milliseconds.
    pub fn max_late_ms(&self) -> f64 {
        self.stats.max_late_us.load(Ordering::Relaxed) as f64 / 1000.0
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

/// How the thread's log lines read.
struct Say {
    label: &'static str,
    describe: fn(&[u8]) -> String,
    started: Instant,
    log: Sender<String>,
}

/// The writer thread: each message when it is due, in the order sent.
fn write_loop<W: Write>(mut w: W, rx: Receiver<Wire>, stats: &Stats, say: &Say) {
    let label = say.label;
    let mut listed = 0u64;
    while let Ok(m) = rx.recv() {
        let now = Instant::now();
        if m.due > now {
            thread::sleep(m.due - now);
        }
        let b = m.bytes();
        match write_msg(&mut w, b) {
            Ok(()) => {
                let late = Instant::now().saturating_duration_since(m.due);
                stats.written.fetch_add(1, Ordering::Relaxed);
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
                stats.gone.store(true, Ordering::Relaxed);
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
}
