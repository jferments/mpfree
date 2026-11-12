//! The control side: work the audio thread hands to other threads so that
//! nothing in its period sleeps, waits on a file or builds an effect.
//!
//! The outside review of 12 Nov found three things on the audio thread
//! that can take longer than a period on their own, outside every engine:
//!
//! - **The controller's lights.** Each message was written to the MIDI port
//!   by the audio thread, which slept 1 ms at a time, for up to half a
//!   second, while the kernel's buffer was full (a kit change sends a light
//!   for every pad). [`Lights`] hands each message to the `mpfree-lights`
//!   thread through a bounded queue; that thread does the writing and the
//!   waiting. A full queue drops the message and counts it.
//! - **Log lines.** Every line was written to the report file by the audio
//!   thread (a system call each, sometimes two). [`Report`] hands each line
//!   to the `mpfree-report` thread, which writes them. A full queue drops
//!   the line and counts it; the count goes in the report.
//! - **Effects added in the chain editor.** Building an effect allocates
//!   everything it will use (the Cloud reverb about 9 MB, every page of it
//!   faulted in and locked under `mlockall`) and was done on the audio
//!   thread. [`Builder`] builds it on the `mpfree-build` thread and hands it
//!   back ready; the effects an edit takes out go there to be dropped too.
//!
//! These threads start from the audio thread, which runs at SCHED_FIFO with
//! SCHED_RESET_ON_FORK (`sys::realtime`), so they start at the normal
//! policy, and each moves itself to the background cores first
//! ([`crate::cores::background`]): with the sound on cores of its own they
//! run on cores 0 and 1, never beside the audio thread or its lanes.
//!
//! Every queue is made before the sound starts, and passing a message
//! never allocates except a long controller message (over [`SHORT`] bytes,
//! which the pad player does not send) and the log line's own text.

use std::io::{self, ErrorKind, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mpfree_fx::{Insert, StripId};

/// Controller messages this long or shorter travel without an allocation.
pub const SHORT: usize = 32;

/// Room in the lights' queue: a kit change sends 16 pad colours and the
/// button lights, so this holds several at once.
const LIGHTS_QUEUE: usize = 512;

/// Room in the report's queue, in lines.
const REPORT_QUEUE: usize = 2048;

/// Room in the builder's queues.
const BUILD_QUEUE: usize = 64;

/// How long a write waits for room in the MIDI port's buffer before it
/// gives up on that message.
const MIDI_WAIT: Duration = Duration::from_millis(500);

/// Writes one whole message to a non-blocking MIDI output, waiting up to
/// `wait` for room in the kernel's buffer (1 ms at a time). Never call it on
/// the audio thread: it sleeps.
pub fn send_all(w: &mut impl Write, msg: &[u8], wait: Duration) -> io::Result<()> {
    let mut done = 0;
    let give_up = Instant::now() + wait;
    while done < msg.len() {
        match w.write(&msg[done..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    ErrorKind::WriteZero,
                    "the MIDI port took nothing",
                ))
            }
            Ok(n) => done += n,
            Err(e) if e.kind() == ErrorKind::WouldBlock && Instant::now() < give_up => {
                thread::sleep(Duration::from_millis(1))
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// One controller message in the lights' queue.
enum Msg {
    Short {
        private: bool,
        len: u8,
        bytes: [u8; SHORT],
    },
    Long {
        private: bool,
        bytes: Vec<u8>,
    },
}

impl Msg {
    fn new(private: bool, msg: &[u8]) -> Msg {
        if msg.len() <= SHORT {
            let mut bytes = [0u8; SHORT];
            bytes[..msg.len()].copy_from_slice(msg);
            Msg::Short {
                private,
                len: msg.len() as u8,
                bytes,
            }
        } else {
            Msg::Long {
                private,
                bytes: msg.to_vec(),
            }
        }
    }

    fn parts(&self) -> (bool, &[u8]) {
        match self {
            Msg::Short {
                private,
                len,
                bytes,
            } => (*private, &bytes[..*len as usize]),
            Msg::Long { private, bytes } => (*private, bytes),
        }
    }
}

/// Counts from the lights' thread.
#[derive(Debug, Default)]
struct LightCounts {
    written: AtomicU64,
    failed: AtomicU64,
}

/// What the lights did, for the end of the session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LightsSummary {
    /// Messages the thread wrote.
    pub written: u64,
    /// Messages the port refused (or had no room for within half a second).
    pub failed: u64,
    /// Messages dropped because the queue was full.
    pub dropped: u64,
    /// The thread was still writing when the session ended.
    pub unfinished: bool,
}

/// The controller's outputs, written by the `mpfree-lights` thread.
pub struct Lights {
    tx: Option<SyncSender<Msg>>,
    errors: Receiver<String>,
    counts: Arc<LightCounts>,
    dropped: u64,
    thread: Option<JoinHandle<()>>,
}

impl Lights {
    /// Starts the thread on the Public output and, if open, the Private
    /// one. Allocates (call it before the loop).
    pub fn start<W: Write + Send + 'static>(public: W, private: Option<W>) -> io::Result<Lights> {
        Lights::start_with(public, private, MIDI_WAIT)
    }

    fn start_with<W: Write + Send + 'static>(
        mut public: W,
        mut private: Option<W>,
        wait: Duration,
    ) -> io::Result<Lights> {
        let (tx, rx) = mpsc::sync_channel::<Msg>(LIGHTS_QUEUE);
        let (err_tx, errors) = mpsc::sync_channel::<String>(8);
        let counts = Arc::new(LightCounts::default());
        let c = Arc::clone(&counts);
        let thread = thread::Builder::new()
            .name("mpfree-lights".into())
            .spawn(move || {
                let _ = crate::cores::background();
                for m in rx {
                    let (to_private, bytes) = m.parts();
                    let port = if to_private {
                        private.as_mut()
                    } else {
                        Some(&mut public)
                    };
                    let r = match port {
                        Some(w) => send_all(w, bytes, wait),
                        None => Err(io::Error::other("the Private output is not open")),
                    };
                    match r {
                        Ok(()) => {
                            c.written.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            c.failed.fetch_add(1, Ordering::Relaxed);
                            let name = if to_private { "Private" } else { "Public" };
                            let _ = err_tx.try_send(format!(
                                "controller: sending {} on {name}: {e}",
                                crate::hex(bytes)
                            ));
                        }
                    }
                }
            })?;
        Ok(Lights {
            tx: Some(tx),
            errors,
            counts,
            dropped: 0,
            thread: Some(thread),
        })
    }

    /// Queues `msg` for the Public output, or the Private one. False if the
    /// queue was full: the message is dropped (and counted). Never waits;
    /// allocates only for a message over [`SHORT`] bytes.
    pub fn send(&mut self, private: bool, msg: &[u8]) -> bool {
        let Some(tx) = self.tx.as_ref() else {
            return false;
        };
        match tx.try_send(Msg::new(private, msg)) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.dropped += 1;
                false
            }
        }
    }

    /// A message the port refused, for the log (at most eight wait); None
    /// most of the time, without allocating.
    pub fn error(&mut self) -> Option<String> {
        self.errors.try_recv().ok()
    }

    /// Messages dropped so far because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Lets the thread write what is queued (up to `wait`) and stops it.
    pub fn finish(mut self, wait: Duration) -> LightsSummary {
        self.tx = None;
        let unfinished = !join_within(self.thread.take(), wait);
        LightsSummary {
            written: self.counts.written.load(Ordering::Relaxed),
            failed: self.counts.failed.load(Ordering::Relaxed),
            dropped: self.dropped,
            unfinished,
        }
    }
}

/// Joins `thread` if it ends within `wait`; true if it did (or there was
/// none). A thread that does not end is left to finish on its own.
fn join_within(thread: Option<JoinHandle<()>>, wait: Duration) -> bool {
    let Some(t) = thread else {
        return true;
    };
    let give_up = Instant::now() + wait;
    while !t.is_finished() {
        if Instant::now() >= give_up {
            return false;
        }
        thread::sleep(Duration::from_millis(2));
    }
    t.join().is_ok()
}

/// The report file, written by the `mpfree-report` thread.
pub struct Report<W: Write + Send + 'static> {
    tx: Option<SyncSender<String>>,
    dropped: u64,
    thread: Option<JoinHandle<W>>,
}

impl<W: Write + Send + 'static> Report<W> {
    /// Starts the thread on `w` (allocates; call it before the loop).
    pub fn start(mut w: W) -> io::Result<Report<W>> {
        let (tx, rx) = mpsc::sync_channel::<String>(REPORT_QUEUE);
        let thread = thread::Builder::new()
            .name("mpfree-report".into())
            .spawn(move || {
                let _ = crate::cores::background();
                // Each line as it comes, then whatever else is waiting, then
                // one flush: a burst of lines costs one write.
                let mut buf = io::BufWriter::with_capacity(16 * 1024, &mut w);
                while let Ok(line) = rx.recv() {
                    let _ = writeln!(buf, "{line}");
                    while let Ok(more) = rx.try_recv() {
                        let _ = writeln!(buf, "{more}");
                    }
                    let _ = buf.flush();
                }
                let _ = buf.flush();
                drop(buf);
                w
            })?;
        Ok(Report {
            tx: Some(tx),
            dropped: 0,
            thread: Some(thread),
        })
    }

    /// Queues a line; false (counted) if the queue was full. Never waits.
    pub fn line(&mut self, line: String) -> bool {
        match self.tx.as_ref().map(|tx| tx.try_send(line)) {
            Some(Ok(())) => true,
            _ => {
                self.dropped += 1;
                false
            }
        }
    }

    /// Lines dropped so far because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Writes what is queued and gives the writer back (None if the thread
    /// did not end within `wait`).
    pub fn finish(mut self, wait: Duration) -> Option<W> {
        self.tx = None;
        let t = self.thread.take()?;
        let give_up = Instant::now() + wait;
        while !t.is_finished() {
            if Instant::now() >= give_up {
                return None;
            }
            thread::sleep(Duration::from_millis(2));
        }
        t.join().ok()
    }
}

/// An effect asked for in the chain editor, built on the builder thread.
pub struct Built {
    pub strip: StripId,
    pub id: &'static str,
    /// None: no effect has that id.
    pub insert: Option<Insert>,
}

enum Job {
    Build {
        strip: StripId,
        id: &'static str,
        rate: u32,
        block: usize,
    },
    Drop(Insert),
}

/// Builds effects for the chain editor, and drops the ones edits take out,
/// on the `mpfree-build` thread.
pub struct Builder {
    tx: SyncSender<Job>,
    built: Receiver<Built>,
    /// Asked for and not back yet.
    pending: usize,
    _thread: JoinHandle<()>,
}

impl Builder {
    /// Starts the thread (allocates; call it before the loop).
    pub fn start() -> io::Result<Builder> {
        let (tx, rx) = mpsc::sync_channel::<Job>(BUILD_QUEUE);
        let (built_tx, built) = mpsc::sync_channel::<Built>(BUILD_QUEUE);
        let thread = thread::Builder::new()
            .name("mpfree-build".into())
            .spawn(move || {
                let _ = crate::cores::background();
                for job in rx {
                    match job {
                        Job::Build {
                            strip,
                            id,
                            rate,
                            block,
                        } => {
                            let insert =
                                mpfree_fx_ui::make(id).map(|node| Insert::new(node, rate, block));
                            // Room for every request (`pending` is checked
                            // against the queue), so this waits only if the
                            // audio thread stopped taking them.
                            if built_tx.send(Built { strip, id, insert }).is_err() {
                                return;
                            }
                        }
                        Job::Drop(insert) => drop(insert),
                    }
                }
            })?;
        Ok(Builder {
            tx,
            built,
            pending: 0,
            _thread: thread,
        })
    }

    /// Asks for effect `id` for `strip`, built for `rate` and blocks of
    /// `block` frames. False if too many are under way. Never waits.
    pub fn build(&mut self, strip: StripId, id: &'static str, rate: u32, block: usize) -> bool {
        if self.pending >= BUILD_QUEUE {
            return false;
        }
        let job = Job::Build {
            strip,
            id,
            rate,
            block,
        };
        if self.tx.try_send(job).is_err() {
            return false;
        }
        self.pending += 1;
        true
    }

    /// Effects asked for and not handed back yet.
    pub fn pending(&self) -> usize {
        self.pending
    }

    /// The next built effect, if one is ready. Never waits.
    pub fn take(&mut self) -> Option<Built> {
        let b = self.built.try_recv().ok()?;
        self.pending -= 1;
        Some(b)
    }

    /// Drops `insert` on the builder thread. False if its queue was full
    /// (or the thread gone): then the insert was dropped here, on the
    /// calling thread, and the caller says so.
    pub fn drop_later(&mut self, insert: Insert) -> bool {
        match self.tx.try_send(Job::Drop(insert)) {
            Ok(()) => true,
            Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => {
                drop(job);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex;

    /// A port that keeps what it is given, refusing the first `busy` writes
    /// with WouldBlock (a full kernel buffer). While `held`, a write waits
    /// (a port the kernel holds up).
    #[derive(Clone, Default)]
    struct Port {
        got: Arc<Mutex<Vec<u8>>>,
        busy: Arc<Mutex<u32>>,
        held: Arc<AtomicBool>,
    }

    impl Write for Port {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            while self.held.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(1));
            }
            let mut busy = self.busy.lock().unwrap();
            if *busy > 0 {
                *busy -= 1;
                return Err(ErrorKind::WouldBlock.into());
            }
            self.got.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn lights_are_written_in_order_by_their_own_thread() {
        let public = Port::default();
        let private = Port::default();
        *public.busy.lock().unwrap() = 5;
        // The port is held up until the sends are done, then busy five
        // times. After two seconds it lets go on its own, so a send that
        // waited for it fails the test rather than hanging it.
        public.held.store(true, Ordering::Relaxed);
        let held = public.held.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(2));
            held.store(false, Ordering::Relaxed);
        });
        let mut l = Lights::start(public.clone(), Some(private.clone())).unwrap();
        let t = Instant::now();
        for k in 0..20u8 {
            assert!(l.send(false, &[0xF0, k, 0xF7]));
        }
        assert!(l.send(true, &[0x90, 1, 2]));
        let long: Vec<u8> = (0..40).collect();
        assert!(l.send(false, &long));
        // The sends never waited for the port: a wait takes the two
        // seconds. (A second leaves room for a slow runner: CI runs these
        // as 32-bit ARM under qemu.)
        assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
        public.held.store(false, Ordering::Relaxed);
        let s = l.finish(Duration::from_secs(5));
        assert_eq!(
            s,
            LightsSummary {
                written: 22,
                failed: 0,
                dropped: 0,
                unfinished: false
            }
        );
        let mut want: Vec<u8> = (0..20u8).flat_map(|k| [0xF0, k, 0xF7]).collect();
        want.extend_from_slice(&long);
        assert_eq!(*public.got.lock().unwrap(), want);
        assert_eq!(*private.got.lock().unwrap(), vec![0x90, 1, 2]);
    }

    #[test]
    fn a_port_that_stays_full_is_counted_and_said() {
        let public = Port::default();
        *public.busy.lock().unwrap() = u32::MAX;
        let mut l = Lights::start_with(public, None, Duration::from_millis(5)).unwrap();
        assert!(l.send(false, &[0xB0, 1, 2]));
        // No Private output: refused by the thread, and said.
        assert!(l.send(true, &[0xB0, 3, 4]));
        let give_up = Instant::now() + Duration::from_secs(5);
        let mut said = Vec::new();
        while said.len() < 2 && Instant::now() < give_up {
            if let Some(e) = l.error() {
                said.push(e);
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert!(
            said[0].starts_with("controller: sending B0 01 02 on Public: "),
            "{said:?}"
        );
        assert!(
            said[1].contains("the Private output is not open"),
            "{said:?}"
        );
        assert_eq!(l.finish(Duration::from_secs(5)).failed, 2);
    }

    #[test]
    fn a_full_lights_queue_drops_and_counts() {
        let public = Port::default();
        *public.busy.lock().unwrap() = u32::MAX;
        // The thread holds the first message for up to a second.
        let mut l = Lights::start_with(public, None, Duration::from_secs(1)).unwrap();
        let mut refused = 0;
        for k in 0..(LIGHTS_QUEUE + 10) {
            if !l.send(false, &[0x90, k as u8, 1]) {
                refused += 1;
            }
        }
        assert!(refused >= 9, "{refused}");
        assert_eq!(l.dropped(), refused);
    }

    #[test]
    fn report_lines_are_written_by_their_own_thread_and_the_file_comes_back() {
        let mut r = Report::start(Vec::<u8>::new()).unwrap();
        for k in 0..100 {
            assert!(r.line(format!("line {k}")));
        }
        assert_eq!(r.dropped(), 0);
        let w = r.finish(Duration::from_secs(5)).expect("the writer back");
        let text = String::from_utf8(w).unwrap();
        assert_eq!(text.lines().count(), 100);
        assert!(text.starts_with("line 0\nline 1\n"));
        assert!(text.ends_with("line 99\n"));
    }

    #[test]
    fn effects_are_built_and_dropped_on_the_builder_thread() {
        let mut b = Builder::start().unwrap();
        assert!(b.build(StripId::Track(1), "mpfree.fx.drive", 44_100, 128));
        assert!(b.build(StripId::Track(2), "no.such.effect", 44_100, 128));
        assert_eq!(b.pending(), 2);
        let give_up = Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while got.len() < 2 && Instant::now() < give_up {
            if let Some(x) = b.take() {
                got.push(x);
            }
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(b.pending(), 0);
        assert_eq!(got[0].strip, StripId::Track(1));
        assert_eq!(got[0].id, "mpfree.fx.drive");
        let insert = got[0].insert.take().expect("a drive");
        assert!(got[1].insert.is_none(), "no such effect");
        assert!(b.drop_later(insert), "dropped on the builder thread");
    }
}
