//! Playing takes from their files, the host's side (the audio tracks
//! rethink, phase 3b): the stream layer's coordinator on a thread of its
//! own (`mpfree-at-stream`), with its reads on another (`mpfree-at-io`).
//!
//! The host's worker starts it ([`start`]) when the app turns streaming
//! on, before recording to files starts, so the writer can wait for
//! playback's reads ([`Gate`]). The player goes to the audio thread, which
//! gives it to the tracks ([`crate::AudioTracks::stream`]). From then on
//! the worker sends this thread what to do ([`ToStream`]), and it answers
//! the audio thread straight away on the worker's own channel:
//!
//! - **Open** a take's finished file as its stream source (the source is
//!   the take's id), checking it holds what the take does: the take's
//!   channels and frames at the tracks' rate. The answer is the source's
//!   generation, for [`crate::Command::OnFile`] or
//!   [`crate::AudioTracks::add_file_take`], or why not.
//! - **Open growing** a recording's file still being written, at the frames
//!   committed so far (`Coordinator::open_growing`), and **grow** it as
//!   more are, under the same generation, so the take's memory goes back
//!   as its file holds it (step 5: takes longer than memory).
//! - **Close** a take's source, only if it is still the generation asked
//!   for (a newer take may have the id by then).
//! - **A new project**: every source closed and the cache emptied.
//! - **Read** a whole file behind every playback read
//!   ([`Coordinator::read_file`]), for a waveform or a fit of a take on
//!   its file; the bytes go back to the worker.
//! - **Heads**: hold the start of every region of a take on its file in
//!   the cache ([`Coordinator::hold_all`], tier Head), in the order the
//!   host gives (the ones that may play soonest first), a few to a hold,
//!   so when the cache runs short the last are given back first; the
//!   answer says how many are held, and so does a later give-back (what
//!   plays always comes first).
//! - **Cues**: hold what a cue that launches audio needs from the takes
//!   on their files ([`Coordinator::hold_all`], tier Cued: the first
//!   second of each region from where it starts), all together; the
//!   answer comes once it is all in the cache, or at once if it can't be
//!   held, and again if it is given back for room before the cue lets it
//!   go (the audio tracks rethink's phase 2, "Cues wait for their audio").
//! - **Devices**: files under a folder on another drive (the SD card's
//!   mount point, [`Stream::add_device`]) read on an I/O thread of their
//!   own, and the recording writer there waits only for playback's reads
//!   from that drive ([`Stream::gate_for`]): a stick that stalls holds up
//!   nothing on the card (the SD card plan's S2b, phase B2).
//! - **Cards**: each opening of the SD card is a numbered insertion
//!   ([`Stream::card`]), and every take under its folder opens through
//!   it (`Coordinator::open_located`): once the card goes, nothing more
//!   is read for those takes and what is in the cache plays out. A cue
//!   still waiting for their audio is short at once, and their regions'
//!   starts are not held again while the card is out. When the card opens
//!   again (a new insertion), each finished take that went with it opens
//!   again from the same file, checked as any open is: its regions' starts
//!   are held again from it, a cue already held for it is given back (its
//!   audio went with the old source), and the host moves the take to the
//!   new generation ([`crate::Command::Refile`]). A take still
//!   being written when its card went stays away (the SD card plan's
//!   phase B2c says what becomes of it). Whether the card is the same one
//!   is told by the file alone for now: its path and layout (phase B2c
//!   adds the card's marker and serial). (Phase B2a.)
//! - **Timings**: every [`LOOK`] each drive's reads (how long they take:
//!   the median, the slowest 1 in 100 and 1 in 1,000, the worst; how many
//!   wait) go on a board the worker's recording syncs share
//!   ([`Timings`]), for Dev and the device rounds' logs (phase B2a).
//! - **System status**: every [`LOOK`] the coordinator's admission (Streaming
//!   at limit, while not even the least safe read-ahead fits) and the
//!   sources whose card went (Card removed) go through a [`Watch`] to the
//!   audio thread as [`FromStream::Problem`] (phase 3b step 5d).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mpfree_stream::coord::{
    Coordinator, DriveTimes, HoldId, HoldTier, IoThread, JobId, JobTier, Plan,
};
use mpfree_stream::format::Layout;
use mpfree_stream::msg::MAX_SOURCES;
use mpfree_stream::player::{Config, Player, UNIT};
use mpfree_stream::policy::Device as Place;
use mpfree_stream::status::Watch;
use mpfree_stream::storage::{Card, CardHandle, Located};
use mpfree_stream::times::Summary;
use mpfree_stream::writer::Gate;

use crate::RecordingId;

/// How the stream layer is set up for the audio tracks: the cache the
/// sound thread owns, and how far ahead the coordinator reads (its `rate`
/// is set to the tracks').
#[derive(Clone, Copy, Debug, Default)]
pub struct StreamConfig {
    pub cache: Config,
    pub plan: Plan,
}

/// The fewest cache slots [`StreamConfig::for_memory`] gives (4 MiB).
pub const MIN_SLOTS: usize = 64;

impl StreamConfig {
    /// A cache as big as `seconds` of stereo at `rate` Hz (the musician's
    /// audio track memory), and at least [`MIN_SLOTS`]. The coordinator
    /// reads as far ahead as that holds, up to its plan's window.
    pub fn for_memory(seconds: u32, rate: u32) -> StreamConfig {
        let samples = seconds as usize * rate as usize * 2;
        StreamConfig {
            cache: Config {
                slots: samples.div_ceil(UNIT).max(MIN_SLOTS),
                ..Config::default()
            },
            ..StreamConfig::default()
        }
    }

    /// The cache's size in bytes.
    pub fn cache_bytes(&self) -> usize {
        self.cache.slots * UNIT * std::mem::size_of::<f32>()
    }
}

/// Whether the sound file at `path` holds a take of `channels` and
/// `frames` at `rate` Hz, by its header alone (no audio read): what a
/// project opening checks before a take opens as its file. The stream
/// thread checks the same as it opens it.
pub fn check_file(path: &Path, channels: usize, frames: usize, rate: u32) -> Result<(), String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let l = mpfree_stream::format::layout(&mut f).map_err(|e| e.to_string())?;
    mismatch(&l, channels, Some(frames), rate).map_or(Ok(()), Err)
}

/// Why a file laid out as `l` isn't a take of `channels` and `frames` at
/// `rate` Hz, if it isn't. Without `frames` (a file still being written,
/// so its length grows) only its channels and rate count.
fn mismatch(l: &Layout, channels: usize, frames: Option<usize>, rate: u32) -> Option<String> {
    if usize::from(l.channels) != channels {
        Some(format!(
            "it has {} channels, the take {channels}",
            l.channels
        ))
    } else if let Some(frames) = frames.filter(|&f| l.frames != f as u64) {
        Some(format!("it has {} frames, the take {frames}", l.frames))
    } else if l.rate != rate {
        Some(format!("it is at {} Hz, the tracks at {rate} Hz", l.rate))
    } else if frames.is_some() && l.cut_short {
        Some("it holds less audio than its header says".into())
    } else {
        None
    }
}

/// What the stream thread is asked to do (see the module notes).
pub(crate) enum ToStream {
    /// `era` is the host's generation of the tracks, given back.
    Open {
        era: u32,
        take: u16,
        rec: RecordingId,
        path: PathBuf,
        channels: usize,
        frames: usize,
    },
    /// Open a recording's file still being written, playing its first
    /// `frames` (committed): it must have the take's channels at the
    /// tracks' rate.
    OpenGrowing {
        era: u32,
        take: u16,
        rec: RecordingId,
        path: PathBuf,
        channels: usize,
        frames: u64,
    },
    /// The file opened growing for recording `rec` holds its first
    /// `frames` now.
    Grow {
        era: u32,
        take: u16,
        rec: RecordingId,
        frames: u64,
    },
    /// It is finished at `path` and holds `frames`: grown to them, or, if
    /// it isn't open as it grew (that failed, or it was closed), opened
    /// as a finished file like [`ToStream::Open`].
    Finished {
        era: u32,
        take: u16,
        rec: RecordingId,
        path: PathBuf,
        channels: usize,
        frames: usize,
    },
    Close {
        take: u16,
        gen: u32,
    },
    Project,
    Read {
        job: u32,
        path: PathBuf,
    },
    /// Hold these starts of regions in the cache from now on, in the
    /// order given (the ones that matter most first), instead of the last.
    Heads(Vec<Head>),
    /// Hold frames `from..to` of each take's source for cue `cue` (the
    /// host's number), all or none, until [`ToStream::LetCueGo`].
    Cue {
        cue: u32,
        parts: Vec<(u16, u64, u64)>,
    },
    /// Cue `cue` landed, or was cancelled: its hold goes.
    LetCueGo {
        cue: u32,
    },
    /// Files under `root` opened from now on read on `dev`, and `gate`
    /// carries playback's cushion on that drive ([`Stream::add_device`]).
    Device {
        root: PathBuf,
        dev: Box<IoThread>,
        gate: Arc<Gate>,
    },
    /// The card at `root` from now on: takes under it open through this
    /// insertion, and those that went with an earlier one open again
    /// ([`Stream::card`]).
    Card {
        root: PathBuf,
        card: Card,
    },
}

/// The start of a region of a take on its file, to hold in the cache:
/// frames `from..to` of take `take`'s source. `order` is the host's, for
/// sorting them by how soon they may play.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Head {
    pub order: i64,
    pub take: u16,
    pub from: u64,
    pub to: u64,
}

/// Heads held together, so the cache gives back the last ones first when
/// it runs short of room.
const HEADS_A_HOLD: usize = 16;

/// How often the stream thread tells System status how it stands.
const LOOK: Duration = Duration::from_millis(250);

/// What the stream thread answers the audio thread.
pub(crate) enum FromStream {
    /// For System status: streaming at its limit, or a card gone.
    Problem(mpfree_status::Report),
    /// Take `take`'s file is open as its source, at generation `gen`:
    /// still being written if `growing`.
    Open {
        era: u32,
        take: u16,
        rec: RecordingId,
        gen: u32,
        growing: bool,
    },
    /// It couldn't be opened, doesn't hold what the take does, or
    /// couldn't grow.
    Failed {
        era: u32,
        take: u16,
        rec: RecordingId,
        why: String,
    },
    /// The first `held` of the heads asked for are held (the rest didn't
    /// fit); the room comes back.
    Heads { heads: Vec<Head>, held: usize },
    /// Heads were given back for room (what plays needed it): `held` are
    /// still held.
    HeadsGivenBack { held: usize },
    /// What cue `cue` needs is all in the cache, and held there.
    CueHeld { cue: u32 },
    /// It can't be held: a source isn't open, or it doesn't fit beside
    /// what plays; or it was given back for room before it was all in.
    CueShort { cue: u32 },
    /// It was held, and has been given back for room.
    CueGivenBack { cue: u32 },
}

/// A cue's hold on the stream thread: the parts it needs, and whether the
/// host has heard they are all in the cache.
struct CueHold {
    cue: u32,
    ids: Vec<HoldId>,
    parts: Vec<(u16, u64, u64)>,
    ready: bool,
}

/// What the stream thread gives back for a [`ToStream::Read`].
pub(crate) struct Read {
    pub job: u32,
    pub bytes: Result<Vec<u8>, String>,
}

/// How long each drive's reads and the recording's syncs take, for Dev
/// and the device rounds' logs (the SD card plan's B2a): the stream thread
/// sets the reads every [`LOOK`], the worker the syncs as the writer runs
/// ([`crate::host::AudioHost::timings`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timings {
    /// Each drive's reads, the stick first.
    pub reads: Vec<DriveTimes>,
    /// The recording writer's syncs, while it runs.
    pub syncs: Option<Syncs>,
}

/// The recording writer's syncs: the rounds that wrote to its drive.
#[derive(Clone, Debug, PartialEq)]
pub struct Syncs {
    /// The folder it records into, and whether that is on the SD card.
    pub dir: PathBuf,
    pub on_card: bool,
    pub times: Summary,
}

/// [`Timings`] shared between the threads that set them and the host.
pub(crate) type Board = Arc<Mutex<Timings>>;

/// The ends the worker keeps.
pub(crate) struct Stream {
    pub tx: Sender<ToStream>,
    /// Playback's cushion on the first drive (the stick).
    pub gate: Arc<Gate>,
    /// Folders on a drive of their own, each with playback's cushion
    /// there.
    devices: Vec<(PathBuf, Arc<Gate>)>,
    /// The card last given at each folder, and how many insertions so far.
    cards: Vec<(PathBuf, Card)>,
    insertions: u32,
}

impl Stream {
    /// Files under `root` (a folder on another drive: the SD card's mount
    /// point) read on an I/O thread of their own from now on (those opened
    /// after this), and a recording writer there gets that drive's gate.
    /// A root given again changes nothing. If its thread can't start, its
    /// files read on the first drive's, as before.
    pub fn add_device(&mut self, root: PathBuf) -> io::Result<()> {
        if self.devices.iter().any(|(r, _)| *r == root) {
            return Ok(());
        }
        // Thread names are cut at 15 characters: "mpfree-at-io-1".
        let dev = IoThread::start(&format!("mpfree-at-io-{}", self.devices.len() + 1))?;
        let gate = Arc::new(Gate::new());
        self.devices.push((root.clone(), gate.clone()));
        let _ = self.tx.send(ToStream::Device {
            root,
            dev: Box::new(dev),
            gate,
        });
        Ok(())
    }

    /// The card opened at `card.root()` (its mount point): a drive of
    /// its own ([`Stream::add_device`]), and a new insertion, numbered
    /// apart from every earlier one, so takes under it read only while it
    /// holds and those that went with the last one open again. The same
    /// opening given again (the slot heard it again) changes nothing.
    /// Its drive's thread failing to start is said, but the card counts
    /// all the same.
    pub fn card(&mut self, handle: Arc<dyn CardHandle>) -> io::Result<()> {
        let root = handle.root().to_path_buf();
        let at = self.cards.iter().position(|(r, _)| *r == root);
        if at.is_some_and(|i| Arc::ptr_eq(&self.cards[i].1.handle, &handle)) {
            return Ok(());
        }
        let thread = self.add_device(root.clone());
        self.insertions += 1;
        let card = Card {
            handle,
            insertion: self.insertions,
        };
        match at {
            Some(i) => self.cards[i].1 = card.clone(),
            None => self.cards.push((root.clone(), card.clone())),
        }
        let _ = self.tx.send(ToStream::Card { root, card });
        thread
    }

    /// The gate of the drive `dir` is on: playback's cushion there, which
    /// a recording writer in `dir` waits for.
    pub fn gate_for(&self, dir: &Path) -> Arc<Gate> {
        self.devices
            .iter()
            .filter(|(r, _)| dir.starts_with(r))
            .max_by_key(|(r, _)| r.components().count())
            .map_or_else(|| self.gate.clone(), |(_, g)| g.clone())
    }
}

/// Start the stream layer for tracks at `rate`: the player (for the audio
/// thread), and the coordinator on a thread of its own, answering opens
/// and holds through `answer` and reads through `read`.
pub(crate) fn start(
    rate: u32,
    cfg: StreamConfig,
    answer: impl Fn(FromStream) -> bool + Send + 'static,
    read: impl Fn(Read) -> bool + Send + 'static,
    board: Board,
) -> io::Result<(Box<Player>, Stream)> {
    let plan = Plan { rate, ..cfg.plan };
    let (player, rx, tx) = Player::new(cfg.cache);
    let dev = IoThread::start("mpfree-at-io")?;
    let coord = Coordinator::new(cfg.cache, plan, rx, tx, Box::new(dev));
    let gate = coord.gate();
    let (to, jobs) = channel::<ToStream>();
    std::thread::Builder::new()
        .name("mpfree-at-stream".into())
        .spawn(move || run(rate, coord, &jobs, answer, read, board))?;
    Ok((
        Box::new(player),
        Stream {
            tx: to,
            gate,
            devices: Vec::new(),
            cards: Vec::new(),
            insertions: 0,
        },
    ))
}

/// The stream thread: the coordinator stepped as often as it has work,
/// and every 2 ms when it has none, until the worker goes.
fn run(
    rate: u32,
    c: Coordinator,
    jobs: &Receiver<ToStream>,
    answer: impl Fn(FromStream) -> bool,
    read: impl Fn(Read) -> bool,
    board: Board,
) {
    let mut s = Side {
        rate,
        c,
        gens: vec![0; MAX_SOURCES],
        recs: vec![RecordingId::NONE; MAX_SOURCES],
        reading: Vec::new(),
        heads: Vec::new(),
        cues: Vec::new(),
        cards: Vec::new(),
        on_card: vec![None; MAX_SOURCES],
        board,
        watch: Watch::new(),
        looked: Instant::now(),
    };
    loop {
        let mut busy = false;
        loop {
            match jobs.try_recv() {
                Ok(m) => {
                    busy = true;
                    if !s.handle(m, &answer, &read) {
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        busy |= s.c.step();
        if !s.finish_reads(&read)
            || !s.heads_given_back(&answer)
            || !s.cues_ready(&answer)
            || !s.look(&answer)
        {
            return;
        }
        if !busy {
            match jobs.recv_timeout(Duration::from_millis(2)) {
                Ok(m) => {
                    if !s.handle(m, &answer, &read) {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

/// The stream thread's state.
struct Side {
    rate: u32,
    c: Coordinator,
    /// The generation each source was opened at (0: closed), and its
    /// recording.
    gens: Vec<u32>,
    recs: Vec<RecordingId>,
    /// Whole-file reads under way: the worker's job, the coordinator's.
    reading: Vec<(u32, JobId)>,
    /// The holds on regions' starts, and what each holds: a take's frames
    /// `from..to`.
    heads: Vec<(HoldId, (u16, u64, u64))>,
    /// The holds for cues waiting for their audio.
    cues: Vec<CueHold>,
    /// The card at each folder ([`ToStream::Card`]).
    cards: Vec<(PathBuf, Card)>,
    /// Each source opened from a card: what it was opened as, so it can
    /// open again once its card is back.
    on_card: Vec<Option<OnCard>>,
    /// Where each drive's read timings go.
    board: Board,
    /// What System status was last told, and when.
    watch: Watch,
    looked: Instant,
}

/// A take's file opened from the card, as it was opened.
#[derive(Clone, Debug)]
struct OnCard {
    era: u32,
    take: u16,
    rec: RecordingId,
    gen: u32,
    path: PathBuf,
    channels: usize,
    /// Its frames once finished; None while it is still being written.
    frames: Option<usize>,
    /// Its card went: nothing more is read for it.
    away: bool,
}

/// The card whose folder holds `path` (the deepest, if folders nest).
fn card_of<'a>(cards: &'a [(PathBuf, Card)], path: &Path) -> Option<&'a Card> {
    cards
        .iter()
        .filter(|(r, _)| path.starts_with(r))
        .max_by_key(|(r, _)| r.components().count())
        .map(|(_, c)| c)
}

/// `path` on `card`, as the coordinator opens a file the storage manager
/// found.
fn located(path: &Path, card: &Card) -> Located {
    Located {
        path: path.to_path_buf(),
        device: Place::Card,
        card: Some(card.clone()),
    }
}

impl Side {
    /// Every [`LOOK`]: the coordinator's admission and the sources whose
    /// card went, for System status. False once the answer can't go.
    fn look(&mut self, answer: &impl Fn(FromStream) -> bool) -> bool {
        if self.looked.elapsed() < LOOK {
            return true;
        }
        self.looked = Instant::now();
        if let Ok(mut t) = self.board.lock() {
            t.reads = self.c.drives();
        }
        let (mut lost, mut live) = (0, true);
        while let Some(tag) = self.c.lost() {
            lost += 1;
            let away = self
                .on_card
                .get_mut(usize::from(tag.source))
                .and_then(Option::as_mut)
                .filter(|o| o.gen == tag.gen);
            if let Some(o) = away {
                o.away = true;
                // Nothing that waits for it can come now.
                live &= self.let_go_of(tag.source, true, answer);
            }
        }
        let mut tell = |r| live &= answer(FromStream::Problem(r));
        self.watch.admission(&self.c.admission(), &mut tell);
        self.watch.card_lost(lost, &mut tell);
        live
    }

    /// One message. False once the answer can't go (the host is gone).
    fn handle(
        &mut self,
        m: ToStream,
        answer: &impl Fn(FromStream) -> bool,
        read: &impl Fn(Read) -> bool,
    ) -> bool {
        match m {
            ToStream::Open {
                era,
                take,
                rec,
                path,
                channels,
                frames,
            } => answer(self.open(era, take, rec, &path, channels, Some(frames))),
            ToStream::OpenGrowing {
                era,
                take,
                rec,
                path,
                channels,
                frames,
            } => answer(self.open_growing(era, take, rec, &path, channels, frames)),
            ToStream::Grow {
                era,
                take,
                rec,
                frames,
            } => match self.grow(take, rec, frames) {
                Some(Err(why)) => answer(FromStream::Failed {
                    era,
                    take,
                    rec,
                    why,
                }),
                _ => true,
            },
            ToStream::Finished {
                era,
                take,
                rec,
                path,
                channels,
                frames,
            } => match self.grow(take, rec, frames as u64) {
                Some(Ok(())) => {
                    let t = usize::from(take);
                    if let Some(o) = self.on_card[t].as_mut().filter(|o| o.rec == rec) {
                        o.path = path;
                        o.frames = Some(frames);
                    }
                    true
                }
                Some(Err(why)) => answer(FromStream::Failed {
                    era,
                    take,
                    rec,
                    why,
                }),
                None => answer(self.open(era, take, rec, &path, channels, Some(frames))),
            },
            ToStream::Close { take, gen } => {
                let t = usize::from(take);
                if self.gens.get(t) == Some(&gen) {
                    self.c.close(take);
                    self.gens[t] = 0;
                }
                if self
                    .on_card
                    .get(t)
                    .and_then(Option::as_ref)
                    .is_some_and(|o| o.gen == gen)
                {
                    self.on_card[t] = None;
                }
                true
            }
            ToStream::Project => {
                // Every hold goes with the sources.
                self.c.new_project();
                self.gens.fill(0);
                self.heads.clear();
                self.cues.clear();
                self.on_card.fill(None);
                true
            }
            ToStream::Heads(heads) => {
                let held = self.hold_heads(&heads, answer);
                answer(FromStream::Heads { heads, held })
            }
            ToStream::Cue { cue, parts } => self.hold_cue(cue, parts, answer),
            ToStream::LetCueGo { cue } => {
                self.let_cue_go(cue);
                true
            }
            ToStream::Device { root, dev, gate } => {
                self.c.add_device(root, dev, gate);
                true
            }
            ToStream::Card { root, card } => {
                let back = card.valid();
                match self.cards.iter_mut().find(|(r, _)| *r == root) {
                    Some((_, c)) => *c = card,
                    None => self.cards.push((root.clone(), card)),
                }
                !back || self.reopen(&root, answer)
            }
            ToStream::Read { job, path } => match match card_of(&self.cards, &path) {
                Some(card) => self
                    .c
                    .read_located(&located(&path, card), JobTier::Background),
                None => self.c.read_file(&path, JobTier::Background),
            } {
                Ok(id) => {
                    self.reading.push((job, id));
                    true
                }
                Err(why) => read(Read {
                    job,
                    bytes: Err(why),
                }),
            },
        }
    }

    /// Whether take `take`'s source is open and can be read: not one
    /// whose card went.
    fn playable(&self, take: u16) -> bool {
        let t = usize::from(take);
        self.gens.get(t).is_some_and(|&g| g > 0)
            && !self.on_card[t].as_ref().is_some_and(|o| o.away)
    }

    /// The cues that need take `take`'s audio go, and the host hears each:
    /// those still waiting for it are short, and unless `waiting_only`,
    /// those already held are given back (as when its source is
    /// replaced). False once the answer can't go.
    fn let_go_of(
        &mut self,
        take: u16,
        waiting_only: bool,
        answer: &impl Fn(FromStream) -> bool,
    ) -> bool {
        let mut live = true;
        let mut i = 0;
        while i < self.cues.len() {
            let h = &self.cues[i];
            if !h.parts.iter().any(|p| p.0 == take) || (waiting_only && h.ready) {
                i += 1;
                continue;
            }
            let h = self.cues.remove(i);
            for id in h.ids {
                self.c.release_hold(id);
            }
            live &= answer(if h.ready {
                FromStream::CueGivenBack { cue: h.cue }
            } else {
                FromStream::CueShort { cue: h.cue }
            });
        }
        live
    }

    /// The card at `root` is back (a new insertion): each finished take
    /// that went with it opens again from its file, checked as any open
    /// is, and the host hears each answer. One that doesn't open (not on
    /// this card) stays away for the next. False once the answer can't go.
    fn reopen(&mut self, root: &Path, answer: &impl Fn(FromStream) -> bool) -> bool {
        let back: Vec<OnCard> = self
            .on_card
            .iter()
            .flatten()
            .filter(|o| o.away && o.frames.is_some() && o.path.starts_with(root))
            .cloned()
            .collect();
        let mut short = false;
        for o in back {
            // Its holds go with the old source: the cues' for good, and the
            // regions' starts to be held again from the new one.
            let mut starts = Vec::new();
            let c = &mut self.c;
            self.heads.retain(|&(id, part)| {
                let keep = part.0 != o.take;
                if !keep {
                    c.release_hold(id);
                    starts.push(part);
                }
                keep
            });
            if !self.let_go_of(o.take, false, answer)
                || !answer(self.open(o.era, o.take, o.rec, &o.path, o.channels, o.frames))
            {
                return false;
            }
            if starts.is_empty() {
                continue;
            }
            match self.playable(o.take) {
                true => match self.c.hold_all(&starts, None, HoldTier::Head) {
                    Ok(ids) => self.heads.extend(ids.into_iter().zip(starts)),
                    Err(_) => short = true,
                },
                false => short = true,
            }
        }
        !short
            || answer(FromStream::HeadsGivenBack {
                held: self.heads.len(),
            })
    }

    /// Grow take `take`'s source, if it is recording `rec`'s file opened
    /// as it grew, to `frames`: None if it isn't.
    fn grow(&mut self, take: u16, rec: RecordingId, frames: u64) -> Option<Result<(), String>> {
        let t = usize::from(take);
        (self.gens.get(t).is_some_and(|&g| g > 0) && self.recs[t] == rec)
            .then(|| self.c.grow(take, frames))
    }

    /// Hold `parts` for cue `cue`, all or none, in place of any hold it
    /// had: short at once if a source isn't open or they don't fit.
    fn hold_cue(
        &mut self,
        cue: u32,
        parts: Vec<(u16, u64, u64)>,
        answer: &impl Fn(FromStream) -> bool,
    ) -> bool {
        self.let_cue_go(cue);
        if !parts.iter().all(|&(take, ..)| self.playable(take)) {
            return answer(FromStream::CueShort { cue });
        }
        match self.c.hold_all(&parts, None, HoldTier::Cued) {
            Ok(ids) => {
                self.cues.push(CueHold {
                    cue,
                    ids,
                    parts,
                    ready: false,
                });
                true
            }
            Err(_) => answer(FromStream::CueShort { cue }),
        }
    }

    /// Cue `cue`'s hold goes, if it has one.
    fn let_cue_go(&mut self, cue: u32) {
        if let Some(i) = self.cues.iter().position(|h| h.cue == cue) {
            let h = self.cues.remove(i);
            for id in h.ids {
                self.c.release_hold(id);
            }
        }
    }

    /// Cues whose parts are all in the cache now: the host hears it once.
    /// False once the answer can't go.
    fn cues_ready(&mut self, answer: &impl Fn(FromStream) -> bool) -> bool {
        for h in self.cues.iter_mut().filter(|h| !h.ready) {
            if h.parts
                .iter()
                .all(|&(take, from, to)| self.c.is_ready(take, from, to))
            {
                h.ready = true;
                if !answer(FromStream::CueHeld { cue: h.cue }) {
                    return false;
                }
            }
        }
        true
    }

    /// A hold the coordinator gave back for room: a cue's goes, and the
    /// host hears it. False once the answer can't go.
    fn cue_given_back(&mut self, id: HoldId, answer: &impl Fn(FromStream) -> bool) -> bool {
        let Some(i) = self.cues.iter().position(|h| h.ids.contains(&id)) else {
            return true;
        };
        let h = self.cues.remove(i);
        for other in h.ids.into_iter().filter(|&o| o != id) {
            self.c.release_hold(other);
        }
        answer(if h.ready {
            FromStream::CueGivenBack { cue: h.cue }
        } else {
            FromStream::CueShort { cue: h.cue }
        })
    }

    /// Hold `heads` instead of the heads held before, a few to a hold, in
    /// order, until one doesn't fit: how many are held.
    fn hold_heads(&mut self, heads: &[Head], answer: &impl Fn(FromStream) -> bool) -> usize {
        // Holds given back before now were the old heads' (or a cue's,
        // whose host hears it).
        while let Some(id) = self.c.given_back() {
            if !self.heads.iter().any(|&(h, _)| h == id) {
                self.cue_given_back(id, answer);
            }
        }
        for (id, _) in self.heads.drain(..) {
            self.c.release_hold(id);
        }
        let mut parts = Vec::with_capacity(HEADS_A_HOLD);
        for group in heads.chunks(HEADS_A_HOLD) {
            parts.clear();
            parts.extend(
                group
                    .iter()
                    .filter(|h| self.playable(h.take))
                    .map(|h| (h.take, h.from, h.to)),
            );
            match self.c.hold_all(&parts, None, HoldTier::Head) {
                Ok(ids) => self
                    .heads
                    .extend(ids.into_iter().zip(parts.iter().copied())),
                Err(_) => break,
            }
        }
        self.heads.len()
    }

    /// Heads the coordinator gave back since last looked, as what plays
    /// needed their room (the last held go first, a hold at a time): the
    /// host hears how many are still held. False once the answer can't go.
    fn heads_given_back(&mut self, answer: &impl Fn(FromStream) -> bool) -> bool {
        let mut gone = false;
        while let Some(id) = self.c.given_back() {
            if self.heads.iter().any(|&(h, _)| h == id) {
                gone = true;
                self.heads.retain(|&(h, _)| h != id);
            } else if !self.cue_given_back(id, answer) {
                return false;
            }
        }
        !gone
            || answer(FromStream::HeadsGivenBack {
                held: self.heads.len(),
            })
    }

    /// Whole-file reads finished go back.
    fn finish_reads(&mut self, read: &impl Fn(Read) -> bool) -> bool {
        let mut i = 0;
        while i < self.reading.len() {
            let (job, id) = self.reading[i];
            match self.c.take_job(id) {
                Some(bytes) => {
                    self.reading.swap_remove(i);
                    if !read(Read { job, bytes }) {
                        return false;
                    }
                }
                None => i += 1,
            }
        }
        true
    }

    /// Open take `take`'s file at `path` as its source, if it holds
    /// `channels` and `frames` (when given) at the tracks' rate.
    fn open(
        &mut self,
        era: u32,
        take: u16,
        rec: RecordingId,
        path: &Path,
        channels: usize,
        frames: Option<usize>,
    ) -> FromStream {
        let opened = match card_of(&self.cards, path) {
            Some(card) => self.c.open_located(take, &located(path, card), false),
            None => self.c.open(take, path),
        };
        let tag = match opened {
            Ok(tag) => tag,
            Err(why) => {
                return FromStream::Failed {
                    era,
                    take,
                    rec,
                    why,
                }
            }
        };
        self.opened(era, take, rec, path, tag.gen, channels, frames)
    }

    /// Open take `take`'s file at `path`, still being written, as its
    /// source playing its first `frames`, if it has `channels` at the
    /// tracks' rate.
    fn open_growing(
        &mut self,
        era: u32,
        take: u16,
        rec: RecordingId,
        path: &Path,
        channels: usize,
        frames: u64,
    ) -> FromStream {
        let opened = match card_of(&self.cards, path) {
            Some(card) => self
                .c
                .open_located_growing(take, &located(path, card), frames),
            None => self.c.open_growing(take, path, frames),
        };
        match opened {
            Ok(tag) => self.opened(era, take, rec, path, tag.gen, channels, None),
            Err(why) => FromStream::Failed {
                era,
                take,
                rec,
                why,
            },
        }
    }

    /// Take `take`'s file at `path` just opened as generation `gen`: kept
    /// if it has `channels` and `frames` (when given: a finished file) at
    /// the tracks' rate, else closed. One on the card is remembered as it
    /// was opened, to open again if its card goes and comes back.
    #[allow(clippy::too_many_arguments)]
    fn opened(
        &mut self,
        era: u32,
        take: u16,
        rec: RecordingId,
        path: &Path,
        gen: u32,
        channels: usize,
        frames: Option<usize>,
    ) -> FromStream {
        let (c, rate) = (&mut self.c, self.rate);
        let why = match c.layout(take) {
            None => Some("it isn't a sound file the stream layer reads".into()),
            Some(l) => mismatch(&l, channels, frames, rate),
        };
        let t = usize::from(take);
        if let Some(why) = why {
            c.close(take);
            self.gens[t] = 0;
            return FromStream::Failed {
                era,
                take,
                rec,
                why,
            };
        }
        self.gens[t] = gen;
        self.recs[t] = rec;
        self.on_card[t] = card_of(&self.cards, path).map(|_| OnCard {
            era,
            take,
            rec,
            gen,
            path: path.to_path_buf(),
            channels,
            frames,
            away: false,
        });
        FromStream::Open {
            era,
            take,
            rec,
            gen,
            growing: frames.is_none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder given a drive of its own gets its own gate (and its own
    /// I/O thread, sent once to the stream thread); everything else keeps
    /// the first drive's.
    #[test]
    fn a_folder_on_another_drive_gets_its_own_gate_once() {
        let (tx, rx) = channel();
        let first = Arc::new(Gate::new());
        let mut st = Stream {
            tx,
            gate: first.clone(),
            devices: Vec::new(),
            cards: Vec::new(),
            insertions: 0,
        };
        let card = PathBuf::from("/run/mpfree-sd");
        assert!(Arc::ptr_eq(
            &st.gate_for(&card.join("MPFree/.work/audiotracks")),
            &first
        ));
        st.add_device(card.clone()).expect("its thread starts");
        st.add_device(card.clone()).expect("given again");
        let sent: Vec<ToStream> = rx.try_iter().collect();
        assert_eq!(sent.len(), 1, "one device, sent once");
        let ToStream::Device { root, gate, .. } = &sent[0] else {
            panic!("not a device");
        };
        assert_eq!(*root, card);
        let there = st.gate_for(&card.join("MPFree/.work/audiotracks"));
        assert!(Arc::ptr_eq(&there, gate));
        assert!(!Arc::ptr_eq(&there, &first));
        // The stick's folder, and a folder that only starts with the same
        // letters, stay on the first drive.
        assert!(Arc::ptr_eq(
            &st.gate_for(Path::new("/run/mpfree-data/library/.work/audiotracks")),
            &first
        ));
        assert!(Arc::ptr_eq(
            &st.gate_for(Path::new("/run/mpfree-sd-other/x")),
            &first
        ));
    }

    /// Each opening of the card is a new insertion, numbered apart from
    /// the last; the same opening heard again is not; the drive's thread
    /// starts once.
    #[test]
    fn each_opening_of_the_card_is_a_new_insertion_and_one_heard_twice_is_not() {
        use mpfree_stream::storage::Lease;
        let (tx, rx) = channel();
        let mut st = Stream {
            tx,
            gate: Arc::new(Gate::new()),
            devices: Vec::new(),
            cards: Vec::new(),
            insertions: 0,
        };
        let root = PathBuf::from("/run/mpfree-sd");
        let first = Lease::new(&root);
        st.card(first.clone()).expect("its thread starts");
        st.card(first.clone()).expect("heard again");
        first.revoke();
        let second = Lease::new(&root);
        st.card(second.clone()).expect("opened again");
        let sent: Vec<ToStream> = rx.try_iter().collect();
        let said: Vec<String> = sent
            .iter()
            .map(|m| match m {
                ToStream::Device { root, .. } => format!("device {}", root.display()),
                ToStream::Card { root, card } => format!(
                    "card {} #{} {}",
                    root.display(),
                    card.insertion,
                    if card.valid() { "holds" } else { "gone" }
                ),
                _ => "other".into(),
            })
            .collect();
        assert_eq!(
            said,
            [
                "device /run/mpfree-sd",
                "card /run/mpfree-sd #1 gone",
                "card /run/mpfree-sd #2 holds",
            ]
        );
    }
}
