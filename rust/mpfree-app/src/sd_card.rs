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
                                "sd card: open, read-only: {} on {}, at {}",
                                card.describe(),
                                s.card,
                                card.mount.display()
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

fn open_card(s: &Status) -> Option<Card> {
    Some(Card {
        fs: s.fs.clone(),
        sectors: s.sectors,
        mount: s.mount.clone()?,
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
        fs::write(p.join("mounts"), "/dev/sda1 / ext4 ro 0 0\n").unwrap();
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
                format!(
                    "{mounts}/dev/mmcblk1p1 {} exfat ro,nosuid 0 0\n",
                    card.display()
                ),
            )
            .unwrap();
            status(
                &p,
                &format!(
                    "state=open\nmessage=The SD card is open, read-only.\nask={word}\ncard=mmcblk1\nfs=exfat\nsectors=124735488\nmount={}\n",
                    card.display()
                ),
            );
            while !p.close.exists() {
                if Instant::now() >= end {
                    return beats;
                }
                if let Ok(b) = fs::read_to_string(&p.alive) {
                    if beats.last() != Some(&b) {
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
            "state=open\nmessage=The SD card is open, read-only.\nask=ab12\ncard=mmcblk1\nfs=vfat\nsectors=4096\nmount=/run/mpfree-sd\n",
        )
        .unwrap();
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
        // The counter moves while it is open.
        thread::sleep(Duration::from_millis(100));
        let said = card.close(&quick()).unwrap();
        assert_eq!(said, "Closed. Nothing was written to the card.");
        let beats = svc.join().unwrap();
        assert!(beats.len() >= 3, "{beats:?}");
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
            format!("/dev/mmcblk1p1 {} vfat ro 0 0\n", card_dir.display()),
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
}
