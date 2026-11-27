//! The SD card in the MPC's slot, read-only: the pad player's side of step
//! S1 of the SD card plan (docs/dev/hardware/sd-card-and-internal-drive.md).
//!
//! The pad player is not root and never opens the card's device. It asks
//! the stick's root service for the card by making
//! `/run/mpfree-app/sd-card.request` (in RAM) with a word of its own in it.
//! systemd's `mpfree-setup-sd-card.path` starts
//! `/etc/mpfree-test/setup-sd-card.sh`, which binds the SD card's controller
//! alone, marks the card read-only, mounts its FAT or exFAT filesystem
//! read-only at `/run/mpfree-sd` and says so in
//! `/run/mpfree-setup/sd-card-status`. Every status gives the word back
//! (`ask=`), so an earlier opening's answer, still in the file, is never
//! taken for this one's.
//!
//! While the card is open, a thread of its own moves a counter in
//! `sd-card.alive`; the service closes the window once the counter stands
//! still for two minutes (the pad player stopped). [`SdCard::close`] asks
//! for the close with `sd-card.close` and waits for it; dropping an
//! [`SdCard`] asks without waiting.
//!
//! A pad player started again while a window it opened before is still open
//! takes that window over (the status says open and the card is mounted)
//! instead of asking again: a request made while a window is open would
//! open the card again once it closed.
//!
//! The write window (step S2a, `setup-sd-card-write.sh`) speaks the same
//! protocol with its own files (`sd-card-write.*`). It opens only an exFAT
//! card that isn't marked and checks clean, mounts the whole card
//! read-only at `/run/mpfree-sd` and only its `MPFree` folder writable,
//! and checks the card again after the close. A [`Slot`] given it
//! ([`Slot::with_writing`]) asks it first; a card it refuses (FAT, marked
//! as not safely removed, not clean, locked) opens through the read-only
//! window instead, and the [`Card`] says why. The two windows are never
//! open at once: each refuses while the other's card is there.
//!
//! Nothing here writes to the card: the files here are all in `/run`.
//! Through the write window, only the card's `MPFree` folder can be
//! written, by whoever saves there.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// The stick's SD card service; without it there is no SD card.
pub const SERVICE: &str = "/etc/mpfree-test/setup-sd-card.sh";
/// What the service reports.
pub const STATUS: &str = "/run/mpfree-setup/sd-card-status";
/// The request, in the program's own folder in /run.
pub const REQUEST: &str = "sd-card.request";
/// Made to close the card.
pub const CLOSE: &str = "sd-card.close";
/// The counter that says the pad player is still using the card.
pub const ALIVE: &str = "sd-card.alive";
/// Where this program's mounts are listed.
pub const MOUNTS: &str = "/proc/self/mounts";
/// The stick's write window (step S2a); without it the card opens
/// read-only.
pub const WRITE_SERVICE: &str = "/etc/mpfree-test/setup-sd-card-write.sh";
/// What the write window reports.
pub const WRITE_STATUS: &str = "/run/mpfree-setup/sd-card-write-status";
/// The write window's request, close and counter, beside the read-only
/// window's.
pub const WRITE_REQUEST: &str = "sd-card-write.request";
pub const WRITE_CLOSE: &str = "sd-card-write.close";
pub const WRITE_ALIVE: &str = "sd-card-write.alive";

/// Where things are; tests point them at a temporary folder.
#[derive(Clone, Debug)]
pub struct Paths {
    pub service: PathBuf,
    pub status: PathBuf,
    pub request: PathBuf,
    pub close: PathBuf,
    pub alive: PathBuf,
    pub mounts: PathBuf,
}

impl Paths {
    pub fn on_the_stick(report_dir: &Path) -> Paths {
        Paths {
            service: PathBuf::from(SERVICE),
            status: PathBuf::from(STATUS),
            request: report_dir.join(REQUEST),
            close: report_dir.join(CLOSE),
            alive: report_dir.join(ALIVE),
            mounts: PathBuf::from(MOUNTS),
        }
    }

    /// The write window's files on the stick.
    pub fn writing_on_the_stick(report_dir: &Path) -> Paths {
        Paths {
            service: PathBuf::from(WRITE_SERVICE),
            status: PathBuf::from(WRITE_STATUS),
            request: report_dir.join(WRITE_REQUEST),
            close: report_dir.join(WRITE_CLOSE),
            alive: report_dir.join(WRITE_ALIVE),
            mounts: PathBuf::from(MOUNTS),
        }
    }
}

/// How long to wait, and how often the counter moves.
#[derive(Clone, Debug)]
pub struct Timing {
    pub poll: Duration,
    /// For the card to open: the service allows a card 10 s to answer after
    /// the bind, on top of its checks.
    pub answer: Duration,
    /// For it to close: up to 10 s for the pad player's last files to go,
    /// and 10 s for the controller.
    pub close: Duration,
    pub beat: Duration,
}

impl Timing {
    pub fn on_the_mpc() -> Timing {
        Timing {
            poll: Duration::from_millis(250),
            answer: Duration::from_secs(60),
            close: Duration::from_secs(60),
            beat: Duration::from_secs(2),
        }
    }
}

/// The service's report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub state: String,
    pub message: String,
    /// The word of the request it answers.
    pub ask: String,
    pub card: String,
    pub fs: String,
    pub sectors: u64,
    pub mount: Option<PathBuf>,
    /// The card's filesystem was left marked as in use (`dirty=yes`).
    pub dirty: bool,
    /// The write window's writable folder, the card's `MPFree` (`write=`).
    pub write: Option<PathBuf>,
    /// exFAT's volume serial number (`serial=`, 8 hex digits).
    pub serial: Option<u32>,
}

pub fn parse_status(text: &str) -> Option<Status> {
    let mut s = Status::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k {
            "state" => s.state = v.to_string(),
            "message" => s.message = v.to_string(),
            "ask" => s.ask = v.to_string(),
            "card" => s.card = v.to_string(),
            "fs" => s.fs = v.to_string(),
            "sectors" => s.sectors = v.parse().unwrap_or(0),
            "mount" if !v.is_empty() => s.mount = Some(PathBuf::from(v)),
            "dirty" => s.dirty = v == "yes",
            "write" if !v.is_empty() => s.write = Some(PathBuf::from(v)),
            "serial" if v.len() == 8 && v.bytes().all(|c| c.is_ascii_hexdigit()) => {
                s.serial = u32::from_str_radix(v, 16).ok()
            }
            _ => {}
        }
    }
    (!s.state.is_empty()).then_some(s)
}

fn read_status(paths: &Paths) -> Option<Status> {
    parse_status(&fs::read_to_string(&paths.status).unwrap_or_default())
}

/// Whether this program sees a filesystem mounted at `at`.
fn mounted(mounts: &Path, at: &Path) -> bool {
    let Some(at) = at.to_str() else {
        return false;
    };
    fs::read_to_string(mounts)
        .unwrap_or_default()
        .lines()
        .any(|l| l.split(' ').nth(1) == Some(at))
}

/// The card, open: read-only, or with its `MPFree` folder writable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    /// `vfat` or `exfat`, as the kernel calls it.
    pub fs: String,
    /// Its size in 512-byte sectors.
    pub sectors: u64,
    /// Where its files are.
    pub mount: PathBuf,
    /// It wasn't safely removed last time: its filesystem is still marked as
    /// in use by the system that last wrote to it (a power cut, or a card
    /// pulled out). Reading is fine; it should be checked on a computer
    /// before anything is saved to it.
    pub dirty: bool,
    /// Its `MPFree` folder, writable, when the write window opened it: the
    /// one place on the card anything is saved. `None`: read-only.
    pub write: Option<PathBuf>,
    /// Why it opened read-only on a stick with the write window: the
    /// window's refusal (not exFAT, not safely removed, not clean, its lock
    /// switch on), for the log.
    pub read_only: Option<String>,
    /// Its filesystem's volume serial (exFAT's, as its window read it):
    /// with the marker file in `MPFree/` it says which card this is
    /// (`mpfree_stream::storage::CardMark`).
    pub serial: Option<u32>,
}

impl Card {
    /// "exFAT, 63.9 GB": what the card is, for the screen.
    pub fn describe(&self) -> String {
        let kind = match self.fs.as_str() {
            "vfat" => "FAT",
            "exfat" => "exFAT",
            other => other,
        };
        let gb = self.sectors as f64 * 512.0 / 1e9;
        format!("{kind}, {gb:.1} GB")
    }
}

/// Why the card is not open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotOpened {
    /// This stick has no SD card service (an older stick, or not the
    /// creator's).
    NoService,
    /// The slot is empty.
    NoCard(String),
    /// The service said why it could not open the card.
    Failed(String),
    /// The write window won't open this card for writing (it can still
    /// open read-only).
    Refused(String),
    /// It could not be asked, or did not answer in time.
    NoAnswer(String),
}

impl NotOpened {
    /// For the screen and the log.
    pub fn message(&self) -> String {
        match self {
            NotOpened::NoService => {
                "This stick can't open the SD card: make it again with the latest USB creator."
                    .to_string()
            }
            NotOpened::NoCard(m)
            | NotOpened::Failed(m)
            | NotOpened::Refused(m)
            | NotOpened::NoAnswer(m) => m.clone(),
        }
    }
}

/// A word for one opening: 32 hex digits.
fn new_word() -> String {
    let mut b = [0u8; 16];
    if File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_err()
    {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        b = (t ^ (u128::from(std::process::id()) << 64)).to_le_bytes();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Writes a small file in the program's own folder in /run, never through a
/// link: whole, under a hidden name, then renamed into place.
fn put(path: &Path, text: &str) -> std::io::Result<()> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = path.with_file_name(format!(".{name}.new"));
    match fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&tmp)?;
    f.write_all(text.as_bytes())?;
    drop(f);
    fs::rename(&tmp, path)
}

/// The SD card, open for as long as this lives: read-only, or through the
/// write window with its `MPFree` folder writable.
pub struct SdCard {
    card: Card,
    ask: String,
    paths: Paths,
    stop: Arc<AtomicBool>,
    beat: Option<thread::JoinHandle<()>>,
    closed: bool,
}

impl SdCard {
    /// Asks the window `paths` names for the card and waits until it is
    /// open (up to `timing.answer`). Blocks: run it off the screen thread.
    pub fn open(
        paths: &Paths,
        timing: &Timing,
        say: &mut dyn FnMut(String),
    ) -> Result<SdCard, NotOpened> {
        if !paths.service.exists() {
            return Err(NotOpened::NoService);
        }
        // A window opened before, by this program before it started again.
        if let Some(s) = read_status(paths) {
            if let (Some(card), true) = (open_card(&s), s.state == "open") {
                if mounted(&paths.mounts, &card.mount) {
                    say(format!(
                        "sd card: the card opened before is still open ({}, at {}): using it",
                        card.describe(),
                        card.mount.display()
                    ));
                    return Ok(SdCard::start(paths, timing, card, s.ask));
                }
            }
        }
        let ask = new_word();
        put(&paths.request, &format!("{ask}\n")).map_err(|e| {
            NotOpened::NoAnswer(format!(
                "The SD card could not be asked for ({}: {e}).",
                paths.request.display()
            ))
        })?;
        say(format!(
            "sd card: asked for the card ({})",
            paths.request.display()
        ));
        let start = Instant::now();
        loop {
            match read_status(paths) {
                Some(s) if s.ask == ask => match s.state.as_str() {
                    "open" => match open_card(&s) {
                        Some(card) => {
                            say(match &card.write {
                                Some(w) => format!(
                                    "sd card: open: {} on {}, at {}, read-only but for its MPFree folder, writable at {}",
                                    card.describe(),
                                    s.card,
                                    card.mount.display(),
                                    w.display()
                                ),
                                None => format!(
                                    "sd card: open, read-only: {} on {}, at {}{}",
                                    card.describe(),
                                    s.card,
                                    card.mount.display(),
                                    if card.dirty {
                                        " (it wasn't safely removed last time)"
                                    } else {
                                        ""
                                    }
                                ),
                            });
                            return Ok(SdCard::start(paths, timing, card, ask));
                        }
                        None => {
                            return Err(NotOpened::Failed(
                                "The SD card service said the card is open but not where."
                                    .to_string(),
                            ))
                        }
                    },
                    "none" => return Err(NotOpened::NoCard(s.message)),
                    "refused" => return Err(NotOpened::Refused(s.message)),
                    "failed" | "closed" => return Err(NotOpened::Failed(s.message)),
                    _ => {}
                },
                _ => {}
            }
            if start.elapsed() >= timing.answer {
                // Not picked up: taken back, so the card doesn't open later
                // unasked. Picked up, a window that opens anyway closes by
                // itself once it hears nothing.
                let _ = fs::remove_file(&paths.request);
                return Err(NotOpened::NoAnswer(
                    "The SD card didn't open in time.".to_string(),
                ));
            }
            thread::sleep(timing.poll);
        }
    }

    fn start(paths: &Paths, timing: &Timing, card: Card, ask: String) -> SdCard {
        let stop = Arc::new(AtomicBool::new(false));
        // The counter's file, never through a link. One left by this program
        // before it started again is the one the service watches.
        let alive = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .custom_flags(libc::O_NOFOLLOW)
            .mode(0o644)
            .open(&paths.alive);
        let beat = {
            let stop = Arc::clone(&stop);
            let (every, beat) = (timing.poll.min(timing.beat), timing.beat);
            thread::Builder::new()
                .name("sd-card-alive".into())
                .spawn(move || {
                    let mut n: u64 = 0;
                    let mut last: Option<Instant> = None;
                    while !stop.load(Ordering::Relaxed) {
                        if last.is_none_or(|t| t.elapsed() >= beat) {
                            n += 1;
                            // Fixed width: each count overwrites the last.
                            if let Ok(f) = &alive {
                                let _ = f.write_at(format!("{n:020}\n").as_bytes(), 0);
                            }
                            last = Some(Instant::now());
                        }
                        thread::sleep(every);
                    }
                })
                .ok()
        };
        SdCard {
            card,
            ask,
            paths: paths.clone(),
            stop,
            beat,
            closed: false,
        }
    }

    pub fn card(&self) -> &Card {
        &self.card
    }

    /// Whether the card is still open (it can be taken out, or the window
    /// closed by the system).
    pub fn is_open(&self) -> bool {
        read_status(&self.paths).is_some_and(|s| s.ask == self.ask && s.state == "open")
    }

    /// Closes the card and waits until the service says so (up to
    /// `timing.close`): its last word, or why it failed.
    pub fn close(mut self, timing: &Timing) -> Result<String, String> {
        self.closed = true;
        self.ask_close()?;
        self.stop_beat();
        let start = Instant::now();
        loop {
            match read_status(&self.paths) {
                Some(s) if s.ask == self.ask => match s.state.as_str() {
                    "closed" | "none" => return Ok(s.message),
                    "failed" => return Err(s.message),
                    _ => {}
                },
                // Another opening's status: this one is over.
                Some(s) if !s.ask.is_empty() => return Ok(s.message),
                _ => {}
            }
            if start.elapsed() >= timing.close {
                return Err("The SD card didn't say it closed.".to_string());
            }
            thread::sleep(timing.poll);
        }
    }

    fn ask_close(&self) -> Result<(), String> {
        put(&self.paths.close, "").map_err(|e| {
            format!(
                "The SD card could not be asked to close ({}: {e}); it closes by itself once its counter stops.",
                self.paths.close.display()
            )
        })
    }

    fn stop_beat(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.beat.take() {
            let _ = h.join();
        }
    }
}

impl Drop for SdCard {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.ask_close();
        }
        self.stop_beat();
    }
}

/// How long the card stays open once nothing uses it: the browser hidden
/// and its worker idle. Coming back within it finds the card still open.
pub const PUT_AWAY: Duration = Duration::from_secs(30);

/// How long an Eject waits for the pad player to let go of every file on
/// the card (a take carrying on on the stick, a take that played from it,
/// a copy from it) before closing it ([`Slot::eject`]): the window stops
/// whatever still holds a file there at its close, which would be the pad
/// player. Past that, the card stays open and the Eject says why.
pub const EJECT_WAIT: Duration = Duration::from_secs(15);

/// Lists this process's files open under a folder (the card's mount
/// point); an error where that can't be known.
type Held = Box<dyn Fn(&Path) -> std::io::Result<Vec<PathBuf>> + Send>;
/// An Eject's thread: the close's answer, or the card still open and why.
type Ejecting = thread::JoinHandle<Result<Result<String, String>, Box<(SdCard, String)>>>;

/// What the Browser's SD card place hears from [`Slot::tick`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Heard {
    /// A line for the log.
    Say(String),
    /// The card is open: read-only, or with its `MPFree` folder writable.
    Open(Card),
    /// It didn't open: why, for the screen.
    NotOpened(String),
    /// It was open and isn't now: taken out, closed by the system, or put
    /// away here once nothing used it. Why, for the log.
    Closed(String),
    /// Ejected ([`Slot::eject`]): closed for taking out, its window's last
    /// word; or the close didn't finish, and why (it may not have closed
    /// cleanly).
    Ejected(Result<String, String>),
    /// Not ejected: the pad player still held files on it after
    /// [`EJECT_WAIT`] (how many, and the first). It stays open, and is
    /// said open again ([`Heard::Open`]) after this.
    NotEjected(String),
}

enum Opening {
    Say(String),
    Done(Box<Result<SdCard, NotOpened>>),
}

/// The card as the Browser's SD card place uses it: opened in a thread of
/// its own when the place asks ([`Slot::open`]), watched while it is open,
/// and closed in a thread once nothing has used it for `put_away`. A close
/// still running when the card is asked for again finishes first.
pub struct Slot {
    paths: Paths,
    /// The write window, asked first when the stick has it.
    write: Option<Paths>,
    timing: Timing,
    put_away: Duration,
    opening: Option<std::sync::mpsc::Receiver<Opening>>,
    open: Option<SdCard>,
    closing: Option<thread::JoinHandle<Result<String, String>>>,
    ejecting: Option<Ejecting>,
    unused_since: Option<Instant>,
    looked: Instant,
    heard: Vec<Heard>,
    /// The last opening's answer may change if asked again (the window
    /// failed, or didn't answer in time), as against an empty slot, no
    /// service or a refused card.
    again: bool,
}

impl Slot {
    pub fn new(paths: Paths, timing: Timing, put_away: Duration) -> Slot {
        Slot {
            paths,
            write: None,
            timing,
            put_away,
            opening: None,
            open: None,
            closing: None,
            ejecting: None,
            unused_since: None,
            looked: Instant::now(),
            heard: Vec::new(),
            again: false,
        }
    }

    /// Whether this stick has the write window, so a card can be saved to.
    pub fn writes(&self) -> bool {
        self.write.as_ref().is_some_and(|w| w.service.exists())
    }

    /// Whether the last card that didn't open might open if asked again:
    /// its window failed (the first-start backup running, the internal
    /// drive open) or didn't answer in time.
    pub fn worth_asking_again(&self) -> bool {
        self.again
    }

    /// The write window too: asked first, the read-only window when it
    /// refuses the card.
    pub fn with_writing(mut self, write: Paths) -> Slot {
        self.write = Some(write);
        self
    }

    /// The pad player's card on the MPC.
    pub fn on_the_mpc(report_dir: &Path) -> Slot {
        Slot::new(
            Paths::on_the_stick(report_dir),
            Timing::on_the_mpc(),
            PUT_AWAY,
        )
        .with_writing(Paths::writing_on_the_stick(report_dir))
    }

    /// Asks for the card, in a thread. Already open, the next tick says so
    /// again; already being opened, nothing more is asked.
    pub fn open(&mut self) {
        if let Some(c) = &self.open {
            self.heard.push(Heard::Open(c.card().clone()));
            return;
        }
        // Being ejected: its answer comes first.
        if self.opening.is_some() || self.ejecting.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let (paths, write, timing) = (self.paths.clone(), self.write.clone(), self.timing.clone());
        let before = self.closing.take();
        let spawned = thread::Builder::new()
            .name("sd-card-open".into())
            .spawn(move || {
                // The last opening's close first: its window ends before
                // this one is asked for.
                if let Some(h) = before {
                    let _ = tx.send(Opening::Say(closed_line(h.join())));
                }
                let r = open_either(&paths, write.as_ref(), &timing, &mut |m| {
                    let _ = tx.send(Opening::Say(m));
                });
                let _ = tx.send(Opening::Done(Box::new(r)));
            });
        match spawned {
            Ok(_) => self.opening = Some(rx),
            Err(e) => self.heard.push(Heard::NotOpened(format!(
                "The SD card couldn't be asked for ({e})."
            ))),
        }
    }

    /// Eject: the card closed now, for taking out, once the pad player
    /// holds no file on it (its own `/proc/self/fd`), waiting up to
    /// [`EJECT_WAIT`]; in a thread. The caller first lets go of what it
    /// can (the takes carry on on the stick, and those that played from it
    /// let go once its opening is revoked). Then [`Slot::tick`] says
    /// [`Heard::Ejected`], or [`Heard::NotEjected`] with the card still
    /// open. Nothing open: nothing happens.
    pub fn eject(&mut self) {
        self.eject_with(
            EJECT_WAIT,
            Box::new(|root: &Path| mpfree_stream::storage::open_under(root)),
        );
    }

    fn eject_with(&mut self, wait: Duration, held: Held) {
        let Some(c) = self.open.take() else { return };
        self.unused_since = None;
        let timing = self.timing.clone();
        let mount = c.card().mount.clone();
        self.heard.push(Heard::Say(
            "sd card: ejecting: waiting for MPFree to let go of its files on it".into(),
        ));
        let spawned = thread::Builder::new()
            .name("sd-card-eject".into())
            .spawn(move || {
                let t0 = Instant::now();
                loop {
                    match held(&mount) {
                        // Where open files can't be listed, it closes as a
                        // put away does.
                        Ok(v) if !v.is_empty() => {
                            if t0.elapsed() >= wait {
                                let why = format!(
                                    "MPFree still had {} file{} open on it ({}) after {} s",
                                    v.len(),
                                    if v.len() == 1 { "" } else { "s" },
                                    v[0].display(),
                                    wait.as_secs()
                                );
                                return Err(Box::new((c, why)));
                            }
                            thread::sleep(Duration::from_millis(50));
                        }
                        _ => break,
                    }
                }
                Ok(c.close(&timing))
            });
        match spawned {
            Ok(h) => self.ejecting = Some(h),
            Err(e) => self
                .heard
                .push(Heard::NotEjected(format!("its close couldn't start ({e})"))),
        }
    }

    /// An Eject's answer, once its thread is done.
    fn poll_ejecting(&mut self) {
        if !self.ejecting.as_ref().is_some_and(|h| h.is_finished()) {
            return;
        }
        let Some(h) = self.ejecting.take() else {
            return;
        };
        match h.join() {
            Ok(Ok(closed)) => self.heard.push(Heard::Ejected(closed)),
            Ok(Err(held)) => {
                let (c, why) = *held;
                self.heard.push(Heard::NotEjected(why));
                self.heard.push(Heard::Open(c.card().clone()));
                self.open = Some(c);
                self.looked = Instant::now();
            }
            Err(_) => self.heard.push(Heard::Ejected(Err(
                "its close stopped before it finished".into()
            ))),
        }
    }

    /// An Eject is under way.
    pub fn ejecting(&self) -> bool {
        self.ejecting.is_some()
    }

    /// Once a frame. `using` says whether anything shows or uses the card
    /// now (the browser shows, or its worker has jobs). What happened since
    /// the last tick, oldest first.
    pub fn tick(&mut self, using: bool) -> Vec<Heard> {
        self.poll_opening();
        self.poll_ejecting();
        if self.closing.as_ref().is_some_and(|h| h.is_finished()) {
            if let Some(h) = self.closing.take() {
                self.heard.push(Heard::Say(closed_line(h.join())));
            }
        }
        if self.open.is_some() {
            if using {
                self.unused_since = None;
            }
            let since = *self.unused_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= self.put_away {
                self.put_away();
            } else if self.looked.elapsed() >= self.timing.beat {
                self.looked = Instant::now();
                self.look();
            }
        }
        std::mem::take(&mut self.heard)
    }

    fn poll_opening(&mut self) {
        use std::sync::mpsc::TryRecvError;
        let Some(rx) = &self.opening else { return };
        loop {
            match rx.try_recv() {
                Ok(Opening::Say(m)) => self.heard.push(Heard::Say(m)),
                Ok(Opening::Done(r)) => {
                    match *r {
                        Ok(c) => {
                            self.heard.push(Heard::Open(c.card().clone()));
                            self.open = Some(c);
                            self.unused_since = None;
                            self.looked = Instant::now();
                        }
                        Err(e) => {
                            self.again = matches!(e, NotOpened::Failed(_) | NotOpened::NoAnswer(_));
                            self.heard.push(Heard::NotOpened(e.message()));
                        }
                    }
                    break;
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.again = false;
                    self.heard.push(Heard::NotOpened(
                        "The SD card didn't open: its thread stopped.".to_string(),
                    ));
                    break;
                }
            }
        }
        self.opening = None;
    }

    /// Nothing used it for `put_away`: closed in a thread.
    fn put_away(&mut self) {
        let Some(c) = self.open.take() else { return };
        self.unused_since = None;
        let timing = self.timing.clone();
        self.heard.push(Heard::Closed(format!(
            "sd card: putting it away: nothing used it for {} s",
            self.put_away.as_secs()
        )));
        match thread::Builder::new()
            .name("sd-card-close".into())
            .spawn(move || c.close(&timing))
        {
            Ok(h) => self.closing = Some(h),
            // Dropped in the failed spawn: asked to close without waiting.
            Err(e) => self.heard.push(Heard::Say(format!(
                "sd card: asked to close without waiting ({e})"
            ))),
        }
    }

    /// While open: whether it still is (taken out, or closed by the system).
    fn look(&mut self) {
        let Some(c) = &self.open else { return };
        if c.is_open() {
            return;
        }
        // Its own window's word: the write window's, when it opened the card.
        let why = read_status(&c.paths)
            .filter(|s| s.ask == c.ask)
            .map_or_else(|| "The SD card's window closed.".to_string(), |s| s.message);
        if let Some(mut c) = self.open.take() {
            // Its window is over: nothing to ask, the counter stops.
            c.closed = true;
        }
        self.heard
            .push(Heard::Closed(format!("sd card: closed: {why}")));
    }
}

fn closed_line(r: thread::Result<Result<String, String>>) -> String {
    match r {
        Ok(Ok(m)) => format!("sd card: closed: {m}"),
        Ok(Err(e)) => format!("sd card: the close didn't finish: {e}"),
        Err(_) => "sd card: the close stopped before it finished".to_string(),
    }
}

fn open_card(s: &Status) -> Option<Card> {
    Some(Card {
        fs: s.fs.clone(),
        sectors: s.sectors,
        mount: s.mount.clone()?,
        dirty: s.dirty,
        write: s.write.clone(),
        read_only: None,
        serial: s.serial,
    })
}

/// Whether the window `paths` names is open from before, its card still
/// mounted here: [`SdCard::open`] takes it over.
fn open_from_before(paths: &Paths) -> bool {
    read_status(paths).is_some_and(|s| {
        s.state == "open" && open_card(&s).is_some_and(|c| mounted(&paths.mounts, &c.mount))
    })
}

/// The card through the write window when the stick has it, else (or when
/// it refuses the card, or fails) read-only. A window still open from
/// before is taken over, whichever it is.
fn open_either(
    read: &Paths,
    write: Option<&Paths>,
    timing: &Timing,
    say: &mut dyn FnMut(String),
) -> Result<SdCard, NotOpened> {
    let Some(w) = write.filter(|w| w.service.exists()) else {
        return SdCard::open(read, timing, say);
    };
    if open_from_before(read) {
        return SdCard::open(read, timing, say);
    }
    match SdCard::open(w, timing, say) {
        Err(NotOpened::Refused(why)) | Err(NotOpened::Failed(why)) => {
            say(format!(
                "sd card: not opened for saving: {why}; opening it read-only"
            ));
            let mut c = SdCard::open(read, timing, say)?;
            c.card.read_only = Some(why);
            Ok(c)
        }
        r => r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn tmp(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("mpfree-sd-card-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(p.join("run")).unwrap();
        fs::create_dir_all(p.join("card")).unwrap();
        fs::write(p.join("setup-sd-card.sh"), "# service\n").unwrap();
        // The device field is never read: a mount is found by its mount point.
        fs::write(p.join("mounts"), "stick / ext4 ro 0 0\n").unwrap();
        Tmp(p)
    }

    fn paths(t: &Tmp) -> Paths {
        Paths {
            service: t.0.join("setup-sd-card.sh"),
            status: t.0.join("run/sd-card-status"),
            request: t.0.join("run/sd-card.request"),
            close: t.0.join("run/sd-card.close"),
            alive: t.0.join("run/sd-card.alive"),
            mounts: t.0.join("mounts"),
        }
    }

    /// The write window's files, beside the read-only window's.
    fn write_paths(t: &Tmp) -> Paths {
        let w = t.0.join("setup-sd-card-write.sh");
        if !w.exists() {
            fs::write(&w, "# write window\n").unwrap();
        }
        Paths {
            service: w,
            status: t.0.join("run/sd-card-write-status"),
            request: t.0.join("run/sd-card-write.request"),
            close: t.0.join("run/sd-card-write.close"),
            alive: t.0.join("run/sd-card-write.alive"),
            mounts: t.0.join("mounts"),
        }
    }

    fn quick() -> Timing {
        Timing {
            poll: Duration::from_millis(5),
            answer: Duration::from_millis(1500),
            close: Duration::from_millis(1500),
            beat: Duration::from_millis(10),
        }
    }

    fn status(p: &Paths, lines: &str) {
        put(&p.status, lines).unwrap();
    }

    /// How the stand-in service answers a request.
    #[derive(Clone, Copy, PartialEq)]
    enum Answer {
        Open,
        NoCard,
        Fail,
        Silent,
        /// The write window's refusal (a FAT card).
        Refuse,
    }

    /// The root service as setup-sd-card.sh answers: it takes the request
    /// and its word, reports, mounts (in the stand-in mounts list) and waits
    /// for the close. Gives back the counters it saw while open.
    fn service(t: &Tmp, answer: Answer) -> thread::JoinHandle<Vec<String>> {
        service_at(paths(t), t.0.join("card"), answer, false)
    }

    /// The write window as setup-sd-card-write.sh answers: the same, with
    /// the card's MPFree folder writable, or a refusal.
    fn write_service(t: &Tmp, answer: Answer) -> thread::JoinHandle<Vec<String>> {
        service_at(write_paths(t), t.0.join("card"), answer, true)
    }

    fn service_at(
        p: Paths,
        card: PathBuf,
        answer: Answer,
        writing: bool,
    ) -> thread::JoinHandle<Vec<String>> {
        thread::spawn(move || {
            let mut beats = Vec::new();
            let end = Instant::now() + Duration::from_secs(5);
            while !p.request.exists() {
                if Instant::now() >= end {
                    return beats;
                }
                thread::sleep(Duration::from_millis(2));
            }
            if answer == Answer::Silent {
                return beats;
            }
            let word: String = fs::read_to_string(&p.request)
                .unwrap()
                .chars()
                .filter(char::is_ascii_hexdigit)
                .take(32)
                .collect();
            fs::remove_file(&p.request).unwrap();
            let _ = fs::remove_file(&p.close);
            let _ = fs::remove_file(&p.alive);
            status(
                &p,
                &format!("state=checking\nmessage=Checking\nask={word}\n"),
            );
            thread::sleep(Duration::from_millis(20));
            match answer {
                Answer::NoCard => {
                    status(
                        &p,
                        &format!(
                            "state=none\nmessage=There is no card in the SD slot.\nask={word}\n"
                        ),
                    );
                    return beats;
                }
                Answer::Fail => {
                    status(&p, &format!("state=failed\nmessage=the card is NTFS; MPFree reads FAT and exFAT\nask={word}\n"));
                    return beats;
                }
                Answer::Refuse => {
                    status(&p, &format!("state=refused\nmessage=the card is not exFAT: MPFree saves only to exFAT for now (FAT and others stay read-only)\nask={word}\n"));
                    return beats;
                }
                _ => {}
            }
            let mounts = fs::read_to_string(&p.mounts).unwrap();
            fs::write(
                &p.mounts,
                format!("{mounts}card {} exfat ro,nosuid 0 0\n", card.display()),
            )
            .unwrap();
            if writing {
                status(
                    &p,
                    &format!(
                        "state=open\nmessage=The SD card is open: MPFree writes inside its MPFree folder only.\nask={word}\ncard=mmcblk1\nfs=exfat\nsectors=124735488\nmount={0}\nwrite={0}/MPFree\nfsck=exfatprogs version : 1.2.2\n",
                        card.display()
                    ),
                );
            } else {
                status(
                    &p,
                    &format!(
                        "state=open\nmessage=The SD card is open, read-only.\nask={word}\ncard=mmcblk1\nfs=exfat\nsectors=124735488\nmount={}\ndirty=no\n",
                        card.display()
                    ),
                );
            }
            while !p.close.exists() {
                if Instant::now() >= end {
                    return beats;
                }
                // Empty only when just made, before its first count (the
                // script's `cat` reads it as no count yet, too).
                if let Ok(b) = fs::read_to_string(&p.alive) {
                    if !b.is_empty() && beats.last() != Some(&b) {
                        beats.push(b);
                    }
                }
                thread::sleep(Duration::from_millis(2));
            }
            fs::remove_file(&p.close).unwrap();
            status(
                &p,
                &format!("state=closing\nmessage=Closing the SD card\nask={word}\n"),
            );
            thread::sleep(Duration::from_millis(20));
            fs::write(&p.mounts, mounts).unwrap();
            let _ = fs::remove_file(&p.alive);
            let last = if writing {
                "Closed cleanly: 0 MiB written; the card checks clean."
            } else {
                "Closed. Nothing was written to the card."
            };
            status(&p, &format!("state=closed\nmessage={last}\nask={word}\n"));
            beats
        })
    }

    #[test]
    fn a_status_reads() {
        let s = parse_status(
            "state=open\nmessage=The SD card is open, read-only.\nask=ab12\ncard=mmcblk1\nfs=vfat\nsectors=4096\nmount=/run/mpfree-sd\ndirty=no\n",
        )
        .unwrap();
        assert!(!s.dirty);
        assert!(parse_status("state=open\ndirty=yes\n").unwrap().dirty);
        assert_eq!(s.serial, None);
        // exFAT's volume serial, as its window says it: 8 hex digits.
        let x = parse_status("state=open\nmount=/x\nserial=5EC01A2B\n").unwrap();
        assert_eq!(open_card(&x).unwrap().serial, Some(0x5EC0_1A2B));
        for bad in [
            "serial=5EC01A2",
            "serial=5EC01A2BB",
            "serial=5EC01A2G",
            "serial=+5EC01A2",
            "serial=",
        ] {
            let st = parse_status(&format!("state=open\n{bad}\n")).unwrap();
            assert_eq!(st.serial, None, "{bad}");
        }
        assert_eq!(s.state, "open");
        assert_eq!(s.ask, "ab12");
        assert_eq!(s.fs, "vfat");
        assert_eq!(s.sectors, 4096);
        assert_eq!(s.mount, Some(PathBuf::from("/run/mpfree-sd")));
        assert_eq!(parse_status("message=hello\n"), None);
        let card = open_card(&s).unwrap();
        assert_eq!(card.describe(), "FAT, 0.0 GB");
        let big = Card {
            fs: "exfat".into(),
            sectors: 124_735_488,
            mount: "/run/mpfree-sd".into(),
            dirty: false,
            write: None,
            read_only: None,
            serial: None,
        };
        assert_eq!(big.describe(), "exFAT, 63.9 GB");
        assert_eq!(new_word().len(), 32);
        assert!(new_word().chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(new_word(), new_word());
    }

    #[test]
    fn the_card_opens_keeps_answering_and_closes() {
        let t = tmp("open");
        let p = paths(&t);
        let svc = service(&t, Answer::Open);
        let mut log = Vec::new();
        let card = SdCard::open(&p, &quick(), &mut |m| log.push(m)).unwrap();
        assert_eq!(card.card().fs, "exfat");
        assert_eq!(card.card().mount, t.0.join("card"));
        assert!(card.is_open());
        assert!(!p.request.exists(), "the service took the request");
        // The counter moves while it is open (waited for by its count, not
        // a time, so a busy machine doesn't fail it).
        let end = Instant::now() + Duration::from_secs(5);
        while fs::read_to_string(&p.alive)
            .ok()
            .and_then(|b| b.trim().parse::<u64>().ok())
            .is_none_or(|n| n < 5)
        {
            assert!(Instant::now() < end, "the counter didn't move");
            thread::sleep(Duration::from_millis(2));
        }
        let said = card.close(&quick()).unwrap();
        assert_eq!(said, "Closed. Nothing was written to the card.");
        let beats = svc.join().unwrap();
        assert!(beats.len() >= 2, "{beats:?}");
        assert!(beats.windows(2).all(|w| w[0] < w[1]), "{beats:?}");
        assert!(beats.iter().all(|b| b.len() == 21 && b.ends_with('\n')));
        assert!(
            log.iter()
                .any(|l| l.contains("sd card: open, read-only: exFAT, 63.9 GB on mmcblk1")),
            "{log:?}"
        );
    }

    #[test]
    fn an_earlier_openings_answer_is_not_taken_for_this_ones() {
        let t = tmp("earlier");
        let p = paths(&t);
        // The last opening ended with no card; one before that was open, its
        // card no longer mounted.
        status(
            &p,
            "state=none\nmessage=There is no card in the SD slot.\nask=00aa\n",
        );
        let svc = service(&t, Answer::Open);
        let card = SdCard::open(&p, &quick(), &mut |_| {}).unwrap();
        assert_eq!(card.card().sectors, 124_735_488);
        drop(card);
        svc.join().unwrap();
        // An "open" left without its mount is not taken over either.
        let t = tmp("earlier-open");
        let p = paths(&t);
        status(
            &p,
            &format!(
                "state=open\nmessage=x\nask=00bb\nfs=exfat\nsectors=8\nmount={}\n",
                t.0.join("card").display()
            ),
        );
        let svc = service(&t, Answer::NoCard);
        assert_eq!(
            SdCard::open(&p, &quick(), &mut |_| {}).err(),
            Some(NotOpened::NoCard("There is no card in the SD slot.".into()))
        );
        svc.join().unwrap();
    }

    #[test]
    fn no_card_and_a_card_it_does_not_read_say_why() {
        let t = tmp("why");
        let p = paths(&t);
        let svc = service(&t, Answer::NoCard);
        let e = SdCard::open(&p, &quick(), &mut |_| {}).err().unwrap();
        assert_eq!(
            e,
            NotOpened::NoCard("There is no card in the SD slot.".into())
        );
        svc.join().unwrap();
        let svc = service(&t, Answer::Fail);
        let e = SdCard::open(&p, &quick(), &mut |_| {}).err().unwrap();
        assert_eq!(e.message(), "the card is NTFS; MPFree reads FAT and exFAT");
        svc.join().unwrap();
    }

    #[test]
    fn a_request_nobody_answers_is_taken_back() {
        let t = tmp("silent");
        let p = paths(&t);
        let svc = service(&t, Answer::Silent);
        let timing = Timing {
            answer: Duration::from_millis(100),
            ..quick()
        };
        assert_eq!(
            SdCard::open(&p, &timing, &mut |_| {}).err(),
            Some(NotOpened::NoAnswer(
                "The SD card didn't open in time.".into()
            ))
        );
        assert!(
            !p.request.exists(),
            "so the card doesn't open later unasked"
        );
        svc.join().unwrap();
    }

    #[test]
    fn a_stick_without_the_service_says_so() {
        let t = tmp("noservice");
        let p = paths(&t);
        fs::remove_file(&p.service).unwrap();
        let e = SdCard::open(&p, &quick(), &mut |_| {}).err().unwrap();
        assert_eq!(e, NotOpened::NoService);
        assert!(e.message().contains("make it again"));
        assert!(!p.request.exists());
    }

    #[test]
    fn a_window_opened_before_is_taken_over_without_asking_again() {
        let t = tmp("takeover");
        let p = paths(&t);
        let card_dir = t.0.join("card");
        fs::write(
            &p.mounts,
            format!("card {} vfat ro 0 0\n", card_dir.display()),
        )
        .unwrap();
        status(
            &p,
            &format!("state=open\nmessage=The SD card is open, read-only.\nask=0123abcd\ncard=mmcblk1\nfs=vfat\nsectors=4096\nmount={}\n", card_dir.display()),
        );
        fs::write(&p.alive, format!("{:020}\n", 7)).unwrap();
        let mut log = Vec::new();
        let card = SdCard::open(&p, &quick(), &mut |m| log.push(m)).unwrap();
        assert!(!p.request.exists(), "no second request");
        assert!(log[0].contains("still open"), "{log:?}");
        thread::sleep(Duration::from_millis(50));
        assert_ne!(
            fs::read_to_string(&p.alive).unwrap(),
            format!("{:020}\n", 7),
            "the same counter moves"
        );
        // The service closes it on the close; its status answers the old word.
        let closer = {
            let p = p.clone();
            thread::spawn(move || {
                while !p.close.exists() {
                    thread::sleep(Duration::from_millis(2));
                }
                status(&p, "state=closed\nmessage=Closed. Nothing was written to the card.\nask=0123abcd\n");
            })
        };
        assert_eq!(
            card.close(&quick()).unwrap(),
            "Closed. Nothing was written to the card."
        );
        closer.join().unwrap();
    }

    #[test]
    fn dropping_the_card_asks_for_the_close() {
        let t = tmp("drop");
        let p = paths(&t);
        let svc = service(&t, Answer::Open);
        let card = SdCard::open(&p, &quick(), &mut |_| {}).unwrap();
        drop(card);
        svc.join().unwrap();
        let s = read_status(&p).unwrap();
        assert_eq!(s.state, "closed");
    }

    #[test]
    fn the_counter_is_never_written_through_a_link() {
        let t = tmp("link");
        let p = paths(&t);
        let elsewhere = t.0.join("elsewhere");
        fs::write(&elsewhere, "keep\n").unwrap();
        std::os::unix::fs::symlink(&elsewhere, &p.alive).unwrap();
        let card = SdCard::start(
            &p,
            &quick(),
            open_card(&parse_status("state=open\nmount=/x\n").unwrap()).unwrap(),
            "ab".into(),
        );
        thread::sleep(Duration::from_millis(30));
        drop(card);
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "keep\n");
        // The close goes in place of a link, not through it.
        let _ = fs::remove_file(&p.close);
        std::os::unix::fs::symlink(&elsewhere, &p.close).unwrap();
        put(&p.close, "").unwrap();
        assert!(!fs::symlink_metadata(&p.close)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "keep\n");
    }

    /// Ticks until `done` holds for what was heard, up to 3 s.
    fn tick_until(slot: &mut Slot, using: bool, done: impl Fn(&[Heard]) -> bool) -> Vec<Heard> {
        let mut all = Vec::new();
        let end = Instant::now() + Duration::from_secs(3);
        while !done(&all) {
            assert!(Instant::now() < end, "not yet: {all:?}");
            all.extend(slot.tick(using));
            thread::sleep(Duration::from_millis(2));
        }
        all
    }

    #[test]
    fn the_place_opens_the_card_in_a_thread_and_puts_it_away_once_unused() {
        let t = tmp("slot");
        let p = paths(&t);
        let svc = service(&t, Answer::Open);
        let mut slot = Slot::new(p.clone(), quick(), Duration::from_millis(150));
        assert!(slot.tick(false).is_empty());
        slot.open();
        slot.open(); // a second tap while it opens asks nothing more
        let heard = tick_until(&mut slot, true, |h| {
            h.iter().any(|x| matches!(x, Heard::Open(_)))
        });
        assert!(heard
            .iter()
            .any(|h| matches!(h, Heard::Say(l) if l.contains("asked for the card"))));
        let Some(Heard::Open(card)) = heard.last() else {
            panic!("{heard:?}")
        };
        assert_eq!(card.mount, t.0.join("card"));
        assert!(slot.open.is_some());
        // Asked again while open: said again, nothing asked.
        slot.open();
        assert!(matches!(slot.tick(true).as_slice(), [Heard::Open(_)]));
        // In use, it stays open past the put-away time.
        let end = Instant::now() + Duration::from_millis(300);
        while Instant::now() < end {
            assert!(slot.tick(true).is_empty());
            thread::sleep(Duration::from_millis(5));
        }
        assert!(slot.open.is_some());
        // Unused: put away, and the close's last word follows.
        let heard = tick_until(&mut slot, false, |h| {
            h.iter()
                .any(|x| matches!(x, Heard::Say(l) if l.starts_with("sd card: closed")))
        });
        assert!(
            matches!(&heard[0], Heard::Closed(l) if l.contains("nothing used it")),
            "{heard:?}"
        );
        assert!(!slot.open.is_some());
        assert!(heard.iter().any(|h| *h
            == Heard::Say("sd card: closed: Closed. Nothing was written to the card.".into())));
        assert!(svc.join().unwrap().len() >= 2);
    }

    /// Eject (B2c): the card closes only once the pad player has let go
    /// of every file on it, at once, whatever uses it; it says so.
    #[test]
    fn an_eject_closes_the_card_once_nothing_is_open_on_it() {
        let t = tmp("eject");
        let p = paths(&t);
        let svc = service(&t, Answer::Open);
        let mut slot = Slot::new(p.clone(), quick(), Duration::from_secs(60));
        slot.open();
        tick_until(&mut slot, true, opened);
        // Two files still open on it for the first looks (a take carrying
        // on, one that played from it), then none.
        let looks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let n = looks.clone();
        let mount = t.0.join("card");
        let close = p.close.clone();
        slot.eject_with(
            Duration::from_secs(2),
            Box::new(move |root: &Path| {
                assert_eq!(root, mount.as_path());
                let k = n.fetch_add(1, Ordering::SeqCst);
                assert!(!close.exists(), "not closed while a file is open");
                Ok(if k < 3 {
                    vec![root.join("a.wav"), root.join("b.wav")]
                } else {
                    Vec::new()
                })
            }),
        );
        assert!(slot.ejecting() && slot.open.is_none());
        // Asked for meanwhile, it isn't opened again.
        slot.open();
        let heard = tick_until(&mut slot, true, |h| {
            h.iter().any(|x| matches!(x, Heard::Ejected(_)))
        });
        assert!(
            heard.iter().any(
                |h| *h == Heard::Ejected(Ok("Closed. Nothing was written to the card.".into()))
            ),
            "{heard:?}"
        );
        assert!(!heard.iter().any(|h| matches!(h, Heard::Open(_))));
        assert!(looks.load(Ordering::SeqCst) >= 4);
        assert!(!slot.ejecting() && slot.open.is_none() && slot.opening.is_none());
        svc.join().unwrap();
    }

    /// A file still open on the card past the wait: it isn't closed (its
    /// window would stop the pad player), and it is said open again.
    #[test]
    fn an_eject_leaves_the_card_open_while_a_file_stays_open_on_it() {
        let t = tmp("eject-held");
        let p = paths(&t);
        let _svc = service(&t, Answer::Open);
        let mut slot = Slot::new(p.clone(), quick(), Duration::from_secs(60));
        slot.open();
        tick_until(&mut slot, true, opened);
        slot.eject_with(
            Duration::from_millis(200),
            Box::new(|root: &Path| Ok(vec![root.join("Recordings/kept.wav")])),
        );
        let heard = tick_until(&mut slot, true, opened);
        assert!(
            matches!(&heard[..], [.., Heard::NotEjected(why), Heard::Open(_)]
                if why.contains("1 file open on it") && why.contains("kept.wav")),
            "{heard:?}"
        );
        assert!(slot.open.is_some() && !p.close.exists());
        // Nothing open: ejected, where open files can't be listed too.
        slot.eject_with(
            Duration::from_millis(200),
            Box::new(|_: &Path| Err(std::io::Error::other("no /proc"))),
        );
        let heard = tick_until(&mut slot, true, |h| {
            h.iter().any(|x| matches!(x, Heard::Ejected(_)))
        });
        assert!(heard.iter().any(|h| matches!(h, Heard::Ejected(Ok(_)))));
    }

    fn closed_said(h: &[Heard]) -> bool {
        h.iter()
            .any(|x| matches!(x, Heard::Say(l) if l.starts_with("sd card: closed")))
    }

    fn opened(h: &[Heard]) -> bool {
        h.iter().any(|x| matches!(x, Heard::Open(_)))
    }

    #[test]
    fn with_the_write_window_the_card_opens_with_its_mpfree_folder_writable() {
        let t = tmp("write");
        let (p, w) = (paths(&t), write_paths(&t));
        let svc = write_service(&t, Answer::Open);
        let mut slot = Slot::new(p.clone(), quick(), Duration::from_millis(150)).with_writing(w);
        slot.open();
        let heard = tick_until(&mut slot, true, opened);
        let Some(Heard::Open(card)) = heard.last() else {
            panic!("{heard:?}")
        };
        assert_eq!(card.mount, t.0.join("card"));
        assert_eq!(card.write, Some(t.0.join("card/MPFree")));
        assert_eq!(card.read_only, None);
        assert!(
            heard.iter().any(|h| matches!(h, Heard::Say(l)
                if l.contains("open: exFAT, 63.9 GB on mmcblk1") && l.contains("read-only but for its MPFree folder"))),
            "{heard:?}"
        );
        assert!(
            read_status(&p).is_none(),
            "the read-only window wasn't asked"
        );
        let heard = tick_until(&mut slot, false, closed_said);
        assert!(
            heard.contains(&Heard::Say(
                "sd card: closed: Closed cleanly: 0 MiB written; the card checks clean.".into()
            )),
            "{heard:?}"
        );
        assert!(svc.join().unwrap().len() >= 2, "its counter moved");
    }

    #[test]
    fn a_card_the_write_window_refuses_opens_read_only_and_says_why() {
        let t = tmp("refused");
        let (p, w) = (paths(&t), write_paths(&t));
        let wsvc = write_service(&t, Answer::Refuse);
        // The read-only window, asked only once the write window refused
        // (the two are never open at once).
        let rsvc = {
            let (w, p, card) = (w.clone(), p.clone(), t.0.join("card"));
            thread::spawn(move || {
                let end = Instant::now() + Duration::from_secs(5);
                while read_status(&w).is_none_or(|s| s.state != "refused") {
                    assert!(Instant::now() < end, "the write window wasn't asked");
                    assert!(!p.request.exists(), "the read-only window was asked first");
                    thread::sleep(Duration::from_millis(1));
                }
                service_at(p, card, Answer::Open, false).join().unwrap()
            })
        };
        let mut slot = Slot::new(p.clone(), quick(), Duration::from_millis(150)).with_writing(w);
        slot.open();
        let heard = tick_until(&mut slot, true, opened);
        let Some(Heard::Open(card)) = heard.last() else {
            panic!("{heard:?}")
        };
        let why = "the card is not exFAT: MPFree saves only to exFAT for now (FAT and others stay read-only)";
        assert_eq!(card.write, None);
        assert_eq!(card.read_only.as_deref(), Some(why));
        assert!(
            heard.contains(&Heard::Say(format!(
                "sd card: not opened for saving: {why}; opening it read-only"
            ))),
            "{heard:?}"
        );
        tick_until(&mut slot, false, closed_said);
        wsvc.join().unwrap();
        assert!(rsvc.join().unwrap().len() >= 2);
    }

    #[test]
    fn an_empty_slot_through_the_write_window_asks_nothing_more() {
        let t = tmp("write-empty");
        let (p, w) = (paths(&t), write_paths(&t));
        let svc = write_service(&t, Answer::NoCard);
        let mut slot = Slot::new(p.clone(), quick(), Duration::from_millis(150)).with_writing(w);
        slot.open();
        let heard = tick_until(&mut slot, true, |h| {
            h.iter().any(|x| matches!(x, Heard::NotOpened(_)))
        });
        assert_eq!(
            heard.last(),
            Some(&Heard::NotOpened("There is no card in the SD slot.".into()))
        );
        assert!(!p.request.exists() && read_status(&p).is_none());
        svc.join().unwrap();
    }

    #[test]
    fn a_read_only_window_open_from_before_is_taken_over_and_without_the_write_window_it_reads() {
        // Open from before: taken over, the write window not asked.
        let t = tmp("write-takeover");
        let (p, w) = (paths(&t), write_paths(&t));
        let card_dir = t.0.join("card");
        fs::write(
            &p.mounts,
            format!("card {} exfat ro 0 0\n", card_dir.display()),
        )
        .unwrap();
        status(
            &p,
            &format!("state=open\nmessage=The SD card is open, read-only.\nask=0123abcd\ncard=mmcblk1\nfs=exfat\nsectors=4096\nmount={}\n", card_dir.display()),
        );
        let mut slot =
            Slot::new(p.clone(), quick(), Duration::from_secs(60)).with_writing(w.clone());
        slot.open();
        let heard = tick_until(&mut slot, true, opened);
        assert!(
            matches!(heard.last(), Some(Heard::Open(c)) if c.write.is_none()),
            "{heard:?}"
        );
        assert!(heard
            .iter()
            .any(|h| matches!(h, Heard::Say(l) if l.contains("still open"))));
        assert!(!w.request.exists() && read_status(&w).is_none());
        drop(slot);
        // A stick without the write window: read-only, as before.
        let t = tmp("write-none");
        let (p, w) = (paths(&t), write_paths(&t));
        fs::remove_file(&w.service).unwrap();
        let svc = service(&t, Answer::Open);
        let mut slot =
            Slot::new(p.clone(), quick(), Duration::from_millis(150)).with_writing(w.clone());
        slot.open();
        let heard = tick_until(&mut slot, true, opened);
        assert!(
            matches!(heard.last(), Some(Heard::Open(c)) if c.write.is_none() && c.read_only.is_none())
        );
        assert!(!w.request.exists());
        tick_until(&mut slot, false, closed_said);
        svc.join().unwrap();
    }

    #[test]
    fn the_place_hears_when_the_card_closes_by_itself_and_why_it_didnt_open() {
        let t = tmp("slot-gone");
        let p = paths(&t);
        let svc = service(&t, Answer::Open);
        let mut slot = Slot::new(p.clone(), quick(), Duration::from_secs(60));
        slot.open();
        tick_until(&mut slot, true, |h| {
            h.iter().any(|x| matches!(x, Heard::Open(_)))
        });
        // The card is taken out: the service says so under the same word.
        let word = read_status(&p).unwrap().ask;
        status(&p, &format!("state=closed\nmessage=The card was taken out while it was open. Nothing was written to it.\nask={word}\n"));
        let heard = tick_until(&mut slot, true, |h| !h.is_empty());
        assert_eq!(heard, [Heard::Closed("sd card: closed: The card was taken out while it was open. Nothing was written to it.".into())]);
        assert!(!slot.open.is_some());
        // Nothing asked of a window that is over.
        assert!(!p.close.exists());
        // The stand-in service gives up waiting for a close; a new tap asks
        // again, and an empty slot says so.
        svc.join().unwrap();
        let svc = service(&t, Answer::NoCard);
        slot.open();
        let heard = tick_until(&mut slot, true, |h| {
            h.iter().any(|x| matches!(x, Heard::NotOpened(_)))
        });
        assert_eq!(
            heard.last(),
            Some(&Heard::NotOpened("There is no card in the SD slot.".into()))
        );
        svc.join().unwrap();
        // No service on this stick.
        fs::remove_file(&p.service).unwrap();
        slot.open();
        let heard = tick_until(&mut slot, true, |h| !h.is_empty());
        assert_eq!(heard, [Heard::NotOpened(NotOpened::NoService.message())]);
    }

    /// How a window of the storm's stand-in service ended.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum End {
        /// Closed when the pad player asked.
        Asked,
        /// The card was taken out while it was open.
        Pulled,
        /// Its counter stood still: nobody held the window.
        Quiet,
    }

    #[derive(Default)]
    struct StormLog {
        opened: Vec<String>,
        ended: Vec<(String, End)>,
        none: usize,
        failed: usize,
        busy: usize,
    }

    /// A small fixed-seed generator, so a storm that fails runs the same
    /// way again.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n
        }
    }

    /// The root service as its path unit starts it again and again: each
    /// request in turn, answered at random (open, no card, a card it doesn't
    /// read, or too busy to take it before the pad player takes it back),
    /// and an open window sometimes ended by the card being taken out. A
    /// window whose counter stands still for `quiet` closes, as
    /// setup-sd-card.sh's QUIET does.
    fn storm_service(
        t: &Tmp,
        seed: u64,
        quiet: Duration,
        stop: Arc<AtomicBool>,
        log: Arc<std::sync::Mutex<StormLog>>,
    ) -> thread::JoinHandle<()> {
        let p = paths(t);
        let card = t.0.join("card");
        let mounts = fs::read_to_string(&p.mounts).unwrap();
        thread::spawn(move || {
            let mut rng = Rng(seed);
            let note = |f: &mut dyn FnMut(&mut StormLog)| f(&mut log.lock().unwrap());
            loop {
                let Ok(text) = fs::read_to_string(&p.request) else {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(1));
                    continue;
                };
                let word: String = text
                    .chars()
                    .filter(char::is_ascii_hexdigit)
                    .take(32)
                    .collect();
                let pick = rng.below(10);
                if pick == 0 {
                    // Too busy: left until the pad player has taken it back
                    // (or asked again), so a request is never taken just as
                    // it is taken back.
                    note(&mut |l| l.busy += 1);
                    while fs::read_to_string(&p.request).is_ok_and(|now| now == text) {
                        thread::sleep(Duration::from_millis(1));
                    }
                    continue;
                }
                let _ = fs::remove_file(&p.request);
                let _ = fs::remove_file(&p.close);
                let _ = fs::remove_file(&p.alive);
                status(
                    &p,
                    &format!("state=checking\nmessage=Checking\nask={word}\n"),
                );
                thread::sleep(Duration::from_millis(rng.below(20)));
                match pick {
                    1 => {
                        note(&mut |l| l.none += 1);
                        status(
                            &p,
                            &format!(
                                "state=none\nmessage=There is no card in the SD slot.\nask={word}\n"
                            ),
                        );
                        continue;
                    }
                    2 => {
                        note(&mut |l| l.failed += 1);
                        status(&p, &format!("state=failed\nmessage=the card is NTFS; MPFree reads FAT and exFAT\nask={word}\n"));
                        continue;
                    }
                    _ => {}
                }
                put(
                    &p.mounts,
                    &format!("{mounts}card {} exfat ro,nosuid 0 0\n", card.display()),
                )
                .unwrap();
                note(&mut |l| l.opened.push(word.clone()));
                status(
                    &p,
                    &format!(
                        "state=open\nmessage=The SD card is open, read-only.\nask={word}\ncard=mmcblk1\nfs=exfat\nsectors=124735488\nmount={}\ndirty=no\n",
                        card.display()
                    ),
                );
                let pull = (rng.below(6) == 0)
                    .then(|| Instant::now() + Duration::from_millis(5 + rng.below(150)));
                let (mut beat, mut moved) = (String::new(), Instant::now());
                let end = loop {
                    if p.close.exists() {
                        let _ = fs::remove_file(&p.close);
                        break End::Asked;
                    }
                    if pull.is_some_and(|at| Instant::now() >= at) {
                        break End::Pulled;
                    }
                    match fs::read_to_string(&p.alive) {
                        Ok(b) if b != beat => (beat, moved) = (b, Instant::now()),
                        _ if moved.elapsed() >= quiet => break End::Quiet,
                        _ => {}
                    }
                    thread::sleep(Duration::from_millis(1));
                };
                let last = match end {
                    End::Asked => {
                        status(
                            &p,
                            &format!("state=closing\nmessage=Closing the SD card\nask={word}\n"),
                        );
                        thread::sleep(Duration::from_millis(rng.below(20)));
                        "Closed. Nothing was written to the card."
                    }
                    End::Pulled => {
                        "The card was taken out while it was open. Nothing was written to it."
                    }
                    End::Quiet => {
                        "Closed: MPFree stopped reading the card. Nothing was written to it."
                    }
                };
                put(&p.mounts, &mounts).unwrap();
                let _ = fs::remove_file(&p.alive);
                note(&mut |l| l.ended.push((word.clone(), end)));
                status(&p, &format!("state=closed\nmessage={last}\nask={word}\n"));
            }
        })
    }

    /// Taps, hides and shows at random for five seconds against a service
    /// that answers at random and has cards taken out: every window the
    /// service opens is the slot's, heard once and closed when the slot puts
    /// it away or the card goes; none is left for the service's quiet close;
    /// what the slot says always matches what it holds; and at the end
    /// nothing is open, asked for or still running.
    #[test]
    fn the_place_keeps_to_one_window_through_a_storm_of_taps() {
        let t = tmp("storm");
        let p = paths(&t);
        let timing = Timing {
            poll: Duration::from_millis(2),
            answer: Duration::from_millis(150),
            close: Duration::from_millis(400),
            beat: Duration::from_millis(5),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let log = Arc::new(std::sync::Mutex::new(StormLog::default()));
        let svc = storm_service(
            &t,
            0x5dca_2d17,
            Duration::from_secs(1),
            Arc::clone(&stop),
            Arc::clone(&log),
        );
        let mounts = fs::read_to_string(&p.mounts).unwrap();
        let mut slot = Slot::new(p.clone(), timing, Duration::from_millis(40));
        let mut rng = Rng(0x0bad_5eed);
        let (mut using, mut held) = (false, false);
        let (mut took, mut pulled_first) = (0, 0);
        let mut heard = Vec::new();
        let mut follow = |slot: &Slot, now: Vec<Heard>| {
            for h in &now {
                match h {
                    Heard::Open(c) => {
                        assert_eq!(c.mount, t.0.join("card"));
                        took += usize::from(!held);
                        held = true;
                    }
                    Heard::NotOpened(why) => {
                        assert!(!held, "not opened while open: {why}");
                        pulled_first += usize::from(why.contains("taken out"));
                    }
                    Heard::Closed(_) => {
                        assert!(held, "closed while not open: {now:?}");
                        held = false;
                    }
                    Heard::Ejected(_) | Heard::NotEjected(_) => panic!("no eject asked: {h:?}"),
                    Heard::Say(_) => {}
                }
            }
            assert_eq!(held, slot.open.is_some(), "{now:?}");
            if let Some(c) = &slot.open {
                assert!(log.lock().unwrap().opened.contains(&c.ask));
            }
            heard.extend(now);
        };
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            match rng.below(100) {
                0..=5 => slot.open(),
                6..=9 => using = !using,
                _ => {}
            }
            let now = slot.tick(using);
            follow(&slot, now);
            thread::sleep(Duration::from_millis(2));
        }
        // Nothing uses it now: put away, and every thread of it done.
        let settle = Instant::now() + Duration::from_secs(5);
        while slot.open.is_some() || slot.opening.is_some() || slot.closing.is_some() {
            assert!(Instant::now() < settle, "not put away");
            let now = slot.tick(false);
            follow(&slot, now);
            thread::sleep(Duration::from_millis(2));
        }
        stop.store(true, Ordering::Relaxed);
        svc.join().unwrap();
        let log = log.lock().unwrap();
        assert!(
            log.ended.iter().all(|(_, e)| *e != End::Quiet),
            "a window nobody held: {:?}",
            log.ended
        );
        assert_eq!(log.ended.len(), log.opened.len());
        assert_eq!(took + pulled_first, log.opened.len());
        assert_ne!(read_status(&p).unwrap().state, "open");
        assert!(!p.request.exists());
        assert_eq!(fs::read_to_string(&p.mounts).unwrap(), mounts);
        // The storm went every way.
        let ended = |e: End| log.ended.iter().filter(|(_, x)| *x == e).count();
        let says = |f: &dyn Fn(&Heard) -> bool| heard.iter().filter(|h| f(h)).count();
        let counts = (
            log.opened.len(),
            ended(End::Asked),
            ended(End::Pulled),
            log.none,
            log.failed,
            log.busy,
            says(&|h| matches!(h, Heard::NotOpened(m) if m == "The SD card didn't open in time.")),
            says(&|h| matches!(h, Heard::Closed(m) if m.contains("nothing used it"))),
            says(&|h| matches!(h, Heard::Closed(m) if m.contains("taken out"))),
        );
        eprintln!("storm: opened, asked, pulled, none, failed, busy, taken back, put away, heard pulled = {counts:?}");
        assert!(
            counts.0 >= 10 && counts.1 >= 3 && counts.2 >= 1,
            "{counts:?}"
        );
        assert!(
            counts.3 >= 1 && counts.4 >= 1 && counts.5 >= 1,
            "{counts:?}"
        );
        assert!(
            counts.6 >= 1 && counts.7 >= 3 && counts.8 >= 1,
            "{counts:?}"
        );
    }
}
