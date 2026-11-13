//! The touchscreen's doorbell: it wakes the screen thread when the panel
//! sends something, so the thread can sleep until the next frame instead of
//! waking every 4 ms to look ([`crate::pace`]).
//!
//! A thread of its own (`mpfree-touch-bell`) holds a second handle on the
//! touchscreen, opened read-only and blocking, and sleeps in read(2) on it.
//! Every evdev reader gets its own copy of each event, so the bell takes
//! nothing from the screen thread's reader (or from any other). It only says
//! "read now"; the screen thread reads its own handle as before. Never
//! grabbed, no ioctl: read-only on the MPC.
//!
//! One ring at a time: the bell rings again only after the screen thread
//! answers (just before it reads), so a burst of touch reports costs the
//! screen one wakeup, not one a report.
//!
//! If the bell's handle ends or fails, the bell stops until the screen thread
//! opens the panel again, and the screen thread reads the panel every 4 ms
//! meanwhile: logged, counted ([`Doorbell::stops`]) and shown on the touch
//! status, as every fallback is.

use std::io::{self, ErrorKind, Read};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;

/// A handle the bell sleeps on: the touchscreen, or a stand-in in tests.
pub type Handle = Box<dyn Read + Send>;

/// Room for whole events of either size (24 bytes on 64-bit, 16 on ARMv7).
const BUF: usize = 64 * 24;

#[derive(Debug, Default)]
struct State {
    /// A ring the screen thread has not answered yet.
    rung: AtomicBool,
    /// The bell is reading a handle.
    ringing: AtomicBool,
    stops: AtomicU64,
}

/// The screen thread's end of the bell.
pub struct Doorbell {
    handles: Sender<Handle>,
    state: Arc<State>,
}

impl Doorbell {
    /// Starts the bell's thread. `ring` wakes the screen thread; it says
    /// false when the screen thread has gone, and the bell then stops.
    pub fn start(ring: impl Fn() -> bool + Send + 'static) -> io::Result<Doorbell> {
        let (handles, rx) = channel();
        let state = Arc::new(State::default());
        let shared = state.clone();
        thread::Builder::new()
            .name("mpfree-touch-bell".into())
            .spawn(move || run(rx, ring, &shared))?;
        Ok(Doorbell { handles, state })
    }

    /// A handle on the panel the screen thread has just opened (again). The
    /// bell takes it at once, or after the next event on the one it holds.
    pub fn give(&self, handle: Handle) {
        // Ringing from now: the bell's thread takes it at once, or rings on
        // the old handle (the same panel) until it does.
        self.state.ringing.store(true, Ordering::SeqCst);
        let _ = self.handles.send(handle);
    }

    /// The screen thread is about to read: the next event rings again.
    pub fn answer(&self) {
        self.state.rung.store(false, Ordering::SeqCst);
    }

    /// True while the bell is reading a handle; while false, the screen
    /// thread looks at the panel every 4 ms.
    pub fn ringing(&self) -> bool {
        self.state.ringing.load(Ordering::SeqCst)
    }

    /// How many times the bell's handle has ended or failed.
    pub fn stops(&self) -> u64 {
        self.state.stops.load(Ordering::Relaxed)
    }

    /// For the touch status: how the screen thread learns of touches.
    pub fn note(&self) -> String {
        let how = if self.ringing() {
            "doorbell on"
        } else {
            "no doorbell, read every 4 ms"
        };
        match self.stops() {
            0 => how.to_string(),
            n => format!("{how} (the doorbell stopped {n} times)"),
        }
    }
}

/// The bell's thread: sleep on a handle, ring on whatever it sends, and take
/// a newer handle when the screen thread gives one. Ends when the screen
/// thread drops its [`Doorbell`] or `ring` says it has gone.
fn run(handles: Receiver<Handle>, ring: impl Fn() -> bool, state: &State) {
    let mut buf = vec![0u8; BUF];
    while let Ok(mut handle) = handles.recv() {
        state.ringing.store(true, Ordering::SeqCst);
        loop {
            let got = handle.read(&mut buf);
            // A newer handle (the screen thread opened the panel again)
            // takes over; the old one goes.
            let mut newer = None;
            loop {
                match handles.try_recv() {
                    Ok(h) => newer = Some(h),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            if let Some(h) = newer {
                handle = h;
            } else {
                match got {
                    Ok(n) if n > 0 => {}
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    end => {
                        let why = match end {
                            Err(e) => e.to_string(),
                            _ => "end of file".to_string(),
                        };
                        state.stops.fetch_add(1, Ordering::Relaxed);
                        state.ringing.store(false, Ordering::SeqCst);
                        println!("touch: doorbell stopped ({why}); reading the panel every 4 ms");
                        // Wake the screen thread so it starts looking.
                        if !ring() {
                            return;
                        }
                        break;
                    }
                }
            }
            if !state.rung.swap(true, Ordering::SeqCst) && !ring() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;

    const SOON: Duration = Duration::from_secs(5);

    /// As the screen thread's ring: only a closed channel means it has gone.
    fn screen_is_there(tx: &std::sync::mpsc::SyncSender<()>) -> bool {
        !matches!(
            tx.try_send(()),
            Err(std::sync::mpsc::TrySendError::Disconnected(_))
        )
    }

    #[test]
    fn the_bell_rings_once_until_answered_and_stops_when_its_handle_ends() {
        let (tx, rings) = sync_channel::<()>(16);
        let bell = Doorbell::start(move || screen_is_there(&tx)).unwrap();
        assert!(!bell.ringing(), "no handle yet");
        let (mut panel, end) = UnixStream::pair().unwrap();
        bell.give(Box::new(end));
        // Two reports before the screen answers: one ring.
        panel.write_all(&[1; 24]).unwrap();
        rings.recv_timeout(SOON).expect("a ring");
        assert!(bell.ringing());
        panel.write_all(&[2; 24]).unwrap();
        assert!(rings.recv_timeout(Duration::from_millis(200)).is_err());
        // Answered: the next report rings again.
        bell.answer();
        panel.write_all(&[3; 24]).unwrap();
        rings.recv_timeout(SOON).expect("a second ring");
        assert_eq!(bell.note(), "doorbell on");
        // The panel goes away: the bell stops, says so and wakes the screen.
        drop(panel);
        rings
            .recv_timeout(SOON)
            .expect("a ring when the bell stops");
        assert!(!bell.ringing());
        assert_eq!(bell.stops(), 1);
        assert_eq!(
            bell.note(),
            "no doorbell, read every 4 ms (the doorbell stopped 1 times)"
        );
        // The panel opened again: the bell rings on the new handle.
        let (mut panel, end) = UnixStream::pair().unwrap();
        bell.give(Box::new(end));
        bell.answer();
        panel.write_all(&[4; 24]).unwrap();
        rings.recv_timeout(SOON).expect("a ring on the new handle");
        assert!(bell.ringing());
    }

    #[test]
    fn a_newer_handle_takes_over_after_the_next_event() {
        let (tx, rings) = sync_channel::<()>(16);
        let bell = Doorbell::start(move || screen_is_there(&tx)).unwrap();
        let (mut old, end) = UnixStream::pair().unwrap();
        bell.give(Box::new(end));
        let (mut new, end) = UnixStream::pair().unwrap();
        bell.give(Box::new(end));
        // The bell may still sleep on the old handle: its next event hands
        // over to the new one (whose first ring may be that event's).
        old.write_all(&[1; 24]).unwrap();
        let _ = rings.recv_timeout(Duration::from_millis(200));
        bell.answer();
        new.write_all(&[2; 24]).unwrap();
        rings
            .recv_timeout(SOON)
            .expect("a ring from the new handle");
        // The old handle closing no longer matters.
        drop(old);
        assert!(rings.recv_timeout(Duration::from_millis(200)).is_err());
        assert!(bell.ringing());
        assert_eq!(bell.stops(), 0);
    }
}
