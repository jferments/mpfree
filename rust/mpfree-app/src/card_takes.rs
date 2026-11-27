//! Recording to the SD card (S2b phase B2b of the SD card plan,
//! docs/dev/hardware/sd-card-and-internal-drive.md): when the card opens
//! with its `MPFree` folder writable, the audio tracks record each take
//! into the card's work folder, `MPFree/.work/audiotracks`, instead of
//! the stick's, and anything a cut-off run left there is first recovered
//! into the card's `MPFree/Recordings`. With no card, a card that opens
//! read-only, or the card closing, takes record on the stick as before.
//!
//! Before any take records there, the card is marked (B2c): `MPFree/`
//! gets a marker file with a random id if it has none
//! (`mpfree_stream::storage::make_marker`), and the id with the card's
//! volume serial goes to the card's lease ([`Do::Mark`]), so a take that
//! carries on on the stick when the card goes names the card it left its
//! first part on.
//!
//! The card is asked for once the stick's own recovery is done, if the
//! stick has the write window. An answer that may change (the window
//! failed: the first-start backup running, the internal drive open, no
//! answer in time) is asked again later, a few times, further apart each
//! time; an empty slot, a stick without the service or a card the window
//! refuses is not. A card opened later by the Browser's SD card place
//! takes the takes too.
//!
//! This is the screen's side, and it does no I/O but the recovery's
//! thread: it says what to do ([`Do`]) and the screen does it. The audio
//! tracks move to the new folder only with nothing in flight there
//! (`AudioHost::record_to`), trying again each period meanwhile.
//!
//! Switched off ([`RECORD_ON_CARD`]) until B2c: a card taken out while a
//! take records on it must first leave the pad player running and the
//! take going on on the stick.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use mpfree_audiotracks::files::{recover_into, Recovered};
use mpfree_stream::storage::{make_marker, CardMark};

/// The card's work folder for the audio tracks, under its `MPFree`
/// folder.
pub const WORK: &str = ".work/audiotracks";
/// Where takes a cut-off run left on the card go, under its `MPFree`
/// folder (the library's Recordings place's folder name).
pub const KEEP: &str = "Recordings";
/// How many times a card whose window failed is asked for again.
pub const TRIES: u32 = 5;
/// The first wait before asking again; each next one is twice as long.
pub const FIRST_WAIT: Duration = Duration::from_secs(30);
/// Whether takes record on the card at all. Off until S2b's B2c (the
/// owner's review, 2026-11-25): today the write window stops whatever
/// still holds a file on a card taken out, which would be the pad player,
/// and a take cut off there has nowhere to go on. Off, the card is never
/// asked for here and takes stay on the stick.
pub const RECORD_ON_CARD: bool = false;

/// What the screen is to do.
#[derive(Debug)]
pub enum Do {
    /// A line for the log.
    Say(String),
    /// A line for the musician (and the log).
    Tell(&'static str),
    /// Ask the slot for the card.
    Ask,
    /// Which card the opening is (its marker in `MPFree/`, made if it had
    /// none, and its volume serial), before any take records there: the
    /// screen tells the card's lease, so a take that carries on elsewhere
    /// when the card goes names it.
    Mark(CardMark),
    /// The audio tracks record each take into this folder from now on.
    RecordTo(PathBuf),
    /// What the card's recovery found, for System status.
    Found(Recovered),
}

type Recovering = Receiver<(PathBuf, std::io::Result<(Recovered, [u8; 16])>)>;

#[derive(Default)]
enum State {
    /// Takes record on the stick, and the card isn't asked for.
    #[default]
    Off,
    /// Asked for (or to be asked again at `again_at`); `tries` so far.
    Asked {
        tries: u32,
        again_at: Option<Instant>,
    },
    /// The card's work folder is being recovered (and the card's marker
    /// read or made), on a thread; the card's volume serial, as its window
    /// said it.
    Recovering(Recovering, Option<u32>),
    /// Takes record into the card's work folder.
    On,
}

/// Where the takes record: the stick, or the card's `MPFree` folder.
pub struct CardTakes {
    state: State,
    /// The stick's work folder, once its recovery is done: where takes go
    /// back to.
    stick: Option<PathBuf>,
    /// Takes may record on the card ([`RECORD_ON_CARD`]).
    on: bool,
}

impl Default for CardTakes {
    fn default() -> CardTakes {
        CardTakes::new(RECORD_ON_CARD)
    }
}

impl CardTakes {
    /// Takes record on the card when it opens for saving only if `on`.
    pub fn new(on: bool) -> CardTakes {
        CardTakes {
            state: State::Off,
            stick: None,
            on,
        }
    }

    /// The stick's recovery is done and takes record into `work` there.
    /// With the write window on this stick (`can`), the card is asked for.
    pub fn stick_ready(&mut self, work: PathBuf, can: bool) -> Vec<Do> {
        self.stick = Some(work);
        if !self.on || !can || !matches!(self.state, State::Off) {
            return Vec::new();
        }
        self.state = State::Asked {
            tries: 0,
            again_at: None,
        };
        vec![Do::Ask]
    }

    /// The card opened, its `MPFree` folder writable at `write` (None:
    /// read-only, for `why`), its volume serial `serial` as its window
    /// said it.
    pub fn opened(
        &mut self,
        write: Option<&Path>,
        why: Option<&str>,
        serial: Option<u32>,
    ) -> Vec<Do> {
        if !self.on {
            return Vec::new();
        }
        match (&self.state, write) {
            (State::Recovering(..) | State::On, _) => Vec::new(),
            // Not until the stick's own folder is set up: it comes first.
            (_, Some(_)) if self.stick.is_none() => Vec::new(),
            (_, Some(folder)) => {
                let mpfree = folder.to_path_buf();
                let work = folder.join(WORK);
                let keep = folder.join(KEEP);
                let (tx, rx) = std::sync::mpsc::channel();
                let spawned = std::thread::Builder::new()
                    .name("card-recover".into())
                    .spawn(move || {
                        let r = make_marker(&mpfree).and_then(|id| {
                            std::fs::create_dir_all(&work)?;
                            Ok((recover_into(&work, &keep)?, id))
                        });
                        let _ = tx.send((work, r));
                    });
                match spawned {
                    Ok(_) => {
                        self.state = State::Recovering(rx, serial);
                        vec![Do::Say(format!(
                            "audio tracks: the SD card opened for saving; looking at {} first",
                            folder.join(WORK).display()
                        ))]
                    }
                    Err(e) => {
                        self.state = State::Off;
                        vec![Do::Say(format!(
                            "audio tracks: takes stay on the stick, as the SD card's recovery couldn't start ({e})"
                        ))]
                    }
                }
            }
            (State::Asked { .. }, None) => {
                self.state = State::Off;
                vec![Do::Say(format!(
                    "audio tracks: takes stay on the stick, as the SD card opened read-only ({})",
                    why.unwrap_or("its window doesn't save")
                ))]
            }
            (State::Off, None) => Vec::new(),
        }
    }

    /// The card didn't open; `again` when the answer may change.
    pub fn not_opened(&mut self, again: bool, now: Instant) -> Vec<Do> {
        let State::Asked { tries, .. } = self.state else {
            return Vec::new();
        };
        if !again || tries >= TRIES {
            self.state = State::Off;
            return vec![Do::Say(
                "audio tracks: takes stay on the stick, as the SD card didn't open".into(),
            )];
        }
        let wait = FIRST_WAIT * 2u32.pow(tries);
        self.state = State::Asked {
            tries: tries + 1,
            again_at: Some(now + wait),
        };
        vec![Do::Say(format!(
            "audio tracks: the SD card is asked for again in {} s",
            wait.as_secs()
        ))]
    }

    /// The card closed (taken out, or closed by the system): takes record
    /// on the stick again.
    pub fn closed(&mut self) -> Vec<Do> {
        if !matches!(self.state, State::Recovering(..) | State::On) {
            return Vec::new();
        }
        self.state = State::Off;
        let mut out = vec![Do::Say(
            "audio tracks: the SD card closed; takes record on the stick again".into(),
        )];
        if let Some(w) = &self.stick {
            out.push(Do::RecordTo(w.clone()));
        }
        out
    }

    /// Once a frame: the card asked for again when due, and the recovery's
    /// answer once it comes.
    pub fn tick(&mut self, now: Instant) -> Vec<Do> {
        match &mut self.state {
            State::Asked {
                again_at: Some(t), ..
            } if now >= *t => {
                if let State::Asked { again_at, .. } = &mut self.state {
                    *again_at = None;
                }
                vec![Do::Ask]
            }
            State::Recovering(rx, serial) => match rx.try_recv() {
                Err(TryRecvError::Empty) => Vec::new(),
                Err(TryRecvError::Disconnected) => {
                    self.state = State::Off;
                    vec![Do::Say(
                        "audio tracks: takes stay on the stick, as the SD card's recovery stopped"
                            .into(),
                    )]
                }
                Ok((work, Err(e))) => {
                    self.state = State::Off;
                    vec![Do::Say(format!(
                        "audio tracks: takes stay on the stick, as the SD card's work folder {} couldn't be looked at ({e})",
                        work.display()
                    ))]
                }
                Ok((work, Ok((found, id)))) => {
                    // A window that didn't say the serial: zeros, as for a
                    // card not known.
                    let mark = CardMark {
                        id,
                        serial: serial.unwrap_or(0),
                    };
                    let mut out: Vec<Do> = found
                        .lines
                        .iter()
                        .map(|l| Do::Say(format!("{l} (on the SD card)")))
                        .collect();
                    if !found.lines.is_empty() {
                        out.push(Do::Tell(
                            "Takes from last time were recovered into the SD card's Recordings",
                        ));
                    }
                    out.push(Do::Found(found));
                    out.push(Do::Mark(mark));
                    out.push(Do::Say(format!(
                        "audio tracks: each take is saved to a file as it records, on the SD card, in {}",
                        work.display()
                    )));
                    out.push(Do::RecordTo(work));
                    self.state = State::On;
                    out
                }
            },
            _ => Vec::new(),
        }
    }

    /// Whether the card is in use for the takes (it isn't put away).
    pub fn holds(&self) -> bool {
        matches!(self.state, State::Recovering(..) | State::On)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("mpfree-card-takes-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn record_to(dos: &[Do]) -> Option<&Path> {
        dos.iter().find_map(|d| match d {
            Do::RecordTo(p) => Some(p.as_path()),
            _ => None,
        })
    }

    /// Ticks until the recovery's answer, with a time limit.
    fn until_recovered(c: &mut CardTakes) -> Vec<Do> {
        let t0 = Instant::now();
        loop {
            let dos = c.tick(Instant::now());
            if !dos.is_empty() {
                return dos;
            }
            assert!(t0.elapsed() < Duration::from_secs(10), "no answer");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_card_opened_for_saving_takes_the_takes_and_gives_them_back_when_it_closes() {
        let root = scratch("open");
        let (stick, mpfree) = (root.join("stick-work"), root.join("card/MPFree"));
        std::fs::create_dir_all(&mpfree).unwrap();
        let mut c = CardTakes::new(true);
        // The stick first; the card is asked for only with the window.
        assert!(c.stick_ready(stick.clone(), false).is_empty());
        let mut c = CardTakes::new(true);
        let dos = c.stick_ready(stick.clone(), true);
        assert!(matches!(dos.as_slice(), [Do::Ask]));
        assert!(!c.holds());
        // Opened for saving: the card's work folder is made and recovered
        // on a thread, and only then do the takes move there.
        let dos = c.opened(Some(&mpfree), None, Some(0x5EC0_1A2B));
        assert!(record_to(&dos).is_none());
        assert!(c.holds(), "the card stays open while it's recovered");
        let dos = until_recovered(&mut c);
        let work = mpfree.join(WORK);
        assert_eq!(record_to(&dos), Some(work.as_path()));
        assert!(dos.iter().any(|d| matches!(d, Do::Found(_))));
        assert!(work.is_dir());
        // Which card it is, said before the takes move there: its marker,
        // made in MPFree/, and its serial.
        let id = mpfree_stream::storage::read_marker(&mpfree)
            .unwrap()
            .expect("marked");
        let mark = dos
            .iter()
            .position(|d| matches!(d, Do::Mark(m) if *m == CardMark { id, serial: 0x5EC0_1A2B }));
        let to = dos.iter().position(|d| matches!(d, Do::RecordTo(_)));
        assert!(mark.is_some() && mark < to, "{dos:?}");
        assert!(c.holds());
        // Opened again (the browser's place): nothing changes.
        assert!(c.opened(Some(&mpfree), None, Some(0x5EC0_1A2B)).is_empty());
        // Closed: back to the stick.
        let dos = c.closed();
        assert_eq!(record_to(&dos), Some(stick.as_path()));
        assert!(!c.holds());
        // A card the browser opens later takes the takes again, the same
        // card by its marker, kept.
        assert!(record_to(&c.opened(Some(&mpfree), None, Some(0x5EC0_1A2B))).is_none());
        let dos = until_recovered(&mut c);
        assert_eq!(record_to(&dos), Some(work.as_path()));
        assert!(dos.iter().any(|d| matches!(d, Do::Mark(m) if m.id == id)));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_take_cut_off_on_the_card_is_recovered_into_its_recordings_and_said() {
        let root = scratch("recover");
        let mpfree = root.join("MPFree");
        let work = mpfree.join(WORK);
        std::fs::create_dir_all(&work).unwrap();
        // A loop layer a cut-off run left in the card's work folder.
        let wav = mpfree_audiotracks::save::wav(&[0.25f32; 64], 2, 44_100);
        std::fs::write(work.join("loop_00000000000000000000000000000001.wav"), wav).unwrap();
        let mut c = CardTakes::new(true);
        c.stick_ready(root.join("stick"), true);
        c.opened(Some(&mpfree), None, Some(0x5EC0_1A2B));
        let dos = until_recovered(&mut c);
        assert!(mpfree
            .join(KEEP)
            .join("loop_00000000000000000000000000000001.wav")
            .exists());
        assert!(dos
            .iter()
            .any(|d| matches!(d, Do::Say(l) if l.ends_with("(on the SD card)"))));
        assert!(dos.iter().any(|d| matches!(d, Do::Tell(_))));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_read_only_card_leaves_the_takes_on_the_stick() {
        let mut c = CardTakes::new(true);
        c.stick_ready(PathBuf::from("/stick"), true);
        let dos = c.opened(None, Some("the card is not exFAT"), None);
        assert!(record_to(&dos).is_none());
        assert!(matches!(dos.as_slice(), [Do::Say(l)] if l.contains("not exFAT")));
        assert!(!c.holds());
        // Closing a card it never used changes nothing.
        assert!(c.closed().is_empty());
    }

    #[test]
    fn a_window_that_failed_is_asked_again_further_apart_a_few_times() {
        let mut c = CardTakes::new(true);
        let t0 = Instant::now();
        c.stick_ready(PathBuf::from("/stick"), true);
        let mut at = t0;
        let mut waits = Vec::new();
        for _ in 0..TRIES {
            let dos = c.not_opened(true, at);
            assert!(matches!(dos.as_slice(), [Do::Say(_)]));
            // Not before it's due.
            assert!(c.tick(at).is_empty());
            let State::Asked {
                again_at: Some(due),
                ..
            } = c.state
            else {
                panic!("not asked again");
            };
            waits.push(due - at);
            assert!(matches!(c.tick(due).as_slice(), [Do::Ask]));
            assert!(c.tick(due).is_empty(), "asked once");
            at = due;
        }
        assert_eq!(waits.first(), Some(&FIRST_WAIT));
        assert!(waits.windows(2).all(|w| w[1] == w[0] * 2));
        // Then it stops asking.
        c.not_opened(true, at);
        assert!(matches!(c.state, State::Off));
        // An answer that won't change isn't asked again.
        let mut c = CardTakes::new(true);
        c.stick_ready(PathBuf::from("/stick"), true);
        c.not_opened(false, t0);
        assert!(matches!(c.state, State::Off));
        assert!(c.tick(t0 + Duration::from_secs(3600)).is_empty());
    }

    /// Switched off (B2b lands so, until B2c): the card is never asked
    /// for, a card the Browser opens takes nothing, and it isn't held.
    #[test]
    fn switched_off_the_card_takes_nothing() {
        let root = scratch("off");
        let mpfree = root.join("card/MPFree");
        std::fs::create_dir_all(&mpfree).unwrap();
        let mut c = CardTakes::new(false);
        assert!(c.stick_ready(root.join("stick"), true).is_empty());
        assert!(c.opened(Some(&mpfree), None, Some(0x5EC0_1A2B)).is_empty());
        assert!(!c.holds());
        assert!(c.tick(Instant::now()).is_empty());
        assert!(c.closed().is_empty());
        assert!(!mpfree.join(WORK).exists(), "nothing made on the card");
        assert!(!mpfree.join(mpfree_stream::storage::MARKER).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn nothing_moves_before_the_sticks_own_folder_is_set_up() {
        let root = scratch("early");
        let mut c = CardTakes::new(true);
        // The browser opened the card before the stick's recovery was done.
        assert!(c.opened(Some(&root), None, None).is_empty());
        assert!(!c.holds());
        let _ = std::fs::remove_dir_all(&root);
    }
}
