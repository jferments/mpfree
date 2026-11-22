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
//! - **Close** a take's source, only if it is still the generation asked
//!   for (a newer take may have the id by then).
//! - **A new project**: every source closed and the cache emptied.
//! - **Read** a whole file behind every playback read
//!   ([`Coordinator::read_file`]), for a waveform or a fit of a take on
//!   its file; the bytes go back to the worker.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use mpfree_stream::coord::{Coordinator, IoThread, JobId, JobTier, Plan};
use mpfree_stream::format::Layout;
use mpfree_stream::msg::MAX_SOURCES;
use mpfree_stream::player::{Config, Player, UNIT};
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
    mismatch(&l, channels, frames, rate).map_or(Ok(()), Err)
}

/// Why a file laid out as `l` isn't a take of `channels` and `frames` at
/// `rate` Hz, if it isn't.
fn mismatch(l: &Layout, channels: usize, frames: usize, rate: u32) -> Option<String> {
    if usize::from(l.channels) != channels {
        Some(format!(
            "it has {} channels, the take {channels}",
            l.channels
        ))
    } else if l.frames != frames as u64 {
        Some(format!("it has {} frames, the take {frames}", l.frames))
    } else if l.rate != rate {
        Some(format!("it is at {} Hz, the tracks at {rate} Hz", l.rate))
    } else if l.cut_short {
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
    Close {
        take: u16,
        gen: u32,
    },
    Project,
    Read {
        job: u32,
        path: PathBuf,
    },
}

/// What the stream thread answers the audio thread.
pub(crate) enum Opened {
    /// Take `take`'s file is open as its source, at generation `gen`.
    Open {
        era: u32,
        take: u16,
        rec: RecordingId,
        gen: u32,
    },
    /// It couldn't be opened, or doesn't hold what the take does.
    Failed {
        era: u32,
        take: u16,
        rec: RecordingId,
        why: String,
    },
}

/// What the stream thread gives back for a [`ToStream::Read`].
pub(crate) struct Read {
    pub job: u32,
    pub bytes: Result<Vec<u8>, String>,
}

/// The ends the worker keeps.
pub(crate) struct Stream {
    pub tx: Sender<ToStream>,
    pub gate: Arc<Gate>,
}

/// Start the stream layer for tracks at `rate`: the player (for the audio
/// thread), and the coordinator on a thread of its own, answering opens
/// through `opened` and reads through `read`.
pub(crate) fn start(
    rate: u32,
    cfg: StreamConfig,
    opened: impl Fn(Opened) -> bool + Send + 'static,
    read: impl Fn(Read) -> bool + Send + 'static,
) -> io::Result<(Box<Player>, Stream)> {
    let plan = Plan { rate, ..cfg.plan };
    let (player, rx, tx) = Player::new(cfg.cache);
    let dev = IoThread::start("mpfree-at-io")?;
    let coord = Coordinator::new(cfg.cache, plan, rx, tx, Box::new(dev));
    let gate = coord.gate();
    let (to, jobs) = channel::<ToStream>();
    std::thread::Builder::new()
        .name("mpfree-at-stream".into())
        .spawn(move || run(rate, coord, &jobs, opened, read))?;
    Ok((Box::new(player), Stream { tx: to, gate }))
}

/// The stream thread: the coordinator stepped as often as it has work,
/// and every 2 ms when it has none, until the worker goes.
fn run(
    rate: u32,
    c: Coordinator,
    jobs: &Receiver<ToStream>,
    opened: impl Fn(Opened) -> bool,
    read: impl Fn(Read) -> bool,
) {
    let mut s = Side {
        rate,
        c,
        gens: vec![0; MAX_SOURCES],
        reading: Vec::new(),
    };
    loop {
        let mut busy = false;
        loop {
            match jobs.try_recv() {
                Ok(m) => {
                    busy = true;
                    if !s.handle(m, &opened, &read) {
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        busy |= s.c.step();
        if !s.finish_reads(&read) {
            return;
        }
        if !busy {
            match jobs.recv_timeout(Duration::from_millis(2)) {
                Ok(m) => {
                    if !s.handle(m, &opened, &read) {
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
    /// The generation each source was opened at (0: closed).
    gens: Vec<u32>,
    /// Whole-file reads under way: the worker's job, the coordinator's.
    reading: Vec<(u32, JobId)>,
}

impl Side {
    /// One message. False once the answer can't go (the host is gone).
    fn handle(
        &mut self,
        m: ToStream,
        opened: &impl Fn(Opened) -> bool,
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
            } => opened(self.open(era, take, rec, &path, channels, frames)),
            ToStream::Close { take, gen } => {
                if self.gens.get(usize::from(take)) == Some(&gen) {
                    self.c.close(take);
                    self.gens[usize::from(take)] = 0;
                }
                true
            }
            ToStream::Project => {
                self.c.new_project();
                self.gens.fill(0);
                true
            }
            ToStream::Read { job, path } => match self.c.read_file(&path, JobTier::Background) {
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
    /// `channels` and `frames` at the tracks' rate.
    fn open(
        &mut self,
        era: u32,
        take: u16,
        rec: RecordingId,
        path: &Path,
        channels: usize,
        frames: usize,
    ) -> Opened {
        let (c, rate) = (&mut self.c, self.rate);
        let failed = |why: String| Opened::Failed {
            era,
            take,
            rec,
            why,
        };
        let tag = match c.open(take, path) {
            Ok(tag) => tag,
            Err(why) => return failed(why),
        };
        let Some(l) = c.layout(take) else {
            c.close(take);
            return failed("it isn't a sound file the stream layer reads".into());
        };
        if let Some(why) = mismatch(&l, channels, frames, rate) {
            c.close(take);
            self.gens[usize::from(take)] = 0;
            return failed(why);
        }
        self.gens[usize::from(take)] = tag.gen;
        Opened::Open {
            era,
            take,
            rec,
            gen: tag.gen,
        }
    }
}
