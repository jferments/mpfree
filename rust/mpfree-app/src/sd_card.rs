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
//! Nothing here writes to the card: it is mounted read-only on a disk the
//! kernel holds read-only, and the files here are all in `/run`.

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

/// The card, open read-only.
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
            NotOpened::NoCard(m) | NotOpened::Failed(m) | NotOpened::NoAnswer(m) => m.clone(),
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

/// The SD card, open read-only for as long as this lives.
pub struct SdCard {
    card: Card,
    ask: String,
    paths: Paths,
    stop: Arc<AtomicBool>,
    beat: Option<thread::JoinHandle<()>>,
    closed: bool,
}

impl SdCard {
    /// Asks for the card and waits until it is open, read-only (up to
    /// `timing.answer`). Blocks: run it off the screen thread.
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
                            say(format!(
                                "sd card: open, read-only: {} on {}, at {}{}",
                                card.describe(),
                                s.card,
                                card.mount.display(),
                                if card.dirty {
                                    " (it wasn't safely removed last time)"
                                } else {
                                    ""
                                }
                            ));
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
                "The SD card could not be asked to close ({}: {e}); it closes by itself in two minutes.",
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

/// What the Browser's SD card place hears from [`Slot::tick`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Heard {
    /// A line for the log.
    Say(String),
    /// The card is open, read-only.
    Open(Card),
    /// It didn't open: why, for the screen.
    NotOpened(String),
    /// It was open and isn't now: taken out, closed by the system, or put
    /// away here once nothing used it. Why, for the log.
    Closed(String),
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
    timing: Timing,
    put_away: Duration,
    opening: Option<std::sync::mpsc::Receiver<Opening>>,
    open: Option<SdCard>,
    closing: Option<thread::JoinHandle<Result<String, String>>>,
    unused_since: Option<Instant>,
    looked: Instant,
    heard: Vec<Heard>,
}

impl Slot {
    pub fn new(paths: Paths, timing: Timing, put_away: Duration) -> Slot {
        Slot {
            paths,
            timing,
            put_away,
            opening: None,
            open: None,
            closing: None,
            unused_since: None,
            looked: Instant::now(),
            heard: Vec::new(),
        }
    }

    /// The pad player's card on the MPC.
    pub fn on_the_mpc(report_dir: &Path) -> Slot {
        Slot::new(
            Paths::on_the_stick(report_dir),
            Timing::on_the_mpc(),
            PUT_AWAY,
        )
    }

    /// Asks for the card, in a thread. Already open, the next tick says so
    /// again; already being opened, nothing more is asked.
    pub fn open(&mut self) {
        if let Some(c) = &self.open {
            self.heard.push(Heard::Open(c.card().clone()));
            return;
        }
        if self.opening.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let (paths, timing) = (self.paths.clone(), self.timing.clone());
        let before = self.closing.take();
        let spawned = thread::Builder::new()
            .name("sd-card-open".into())
            .spawn(move || {
                // The last opening's close first: its window ends before
                // this one is asked for.
                if let Some(h) = before {
                    let _ = tx.send(Opening::Say(closed_line(h.join())));
                }
                let r = SdCard::open(&paths, &timing, &mut |m| {
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

    /// Once a frame. `using` says whether anything shows or uses the card
    /// now (the browser shows, or its worker has jobs). What happened since
    /// the last tick, oldest first.
    pub fn tick(&mut self, using: bool) -> Vec<Heard> {
        self.poll_opening();
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
                        Err(e) => self.heard.push(Heard::NotOpened(e.message())),
                    }
                    break;
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
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
        let why = read_status(&self.paths)
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
    })
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
    }

    /// The root service as setup-sd-card.sh answers: it takes the request
    /// and its word, reports, mounts (in the stand-in mounts list) and waits
    /// for the close. Gives back the counters it saw while open.
    fn service(t: &Tmp, answer: Answer) -> thread::JoinHandle<Vec<String>> {
        let p = paths(t);
        let card = t.0.join("card");
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
                _ => {}
            }
            let mounts = fs::read_to_string(&p.mounts).unwrap();
            fs::write(
                &p.mounts,
                format!("{mounts}card {} exfat ro,nosuid 0 0\n", card.display()),
            )
            .unwrap();
            status(
                &p,
                &format!(
                    "state=open\nmessage=The SD card is open, read-only.\nask={word}\ncard=mmcblk1\nfs=exfat\nsectors=124735488\nmount={}\ndirty=no\n",
                    card.display()
                ),
            );
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
            status(
                &p,
                &format!(
                    "state=closed\nmessage=Closed. Nothing was written to the card.\nask={word}\n"
                ),
            );
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
                    &format!(
                        "{mounts}/dev/mmcblk1p1 {} exfat ro,nosuid 0 0\n",
                        card.display()
                    ),
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
