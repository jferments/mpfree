//! Recording to files, the host's side (docs/dev/design/audio-tracks.md,
//! phase 3a). Each take recorded with [`crate::AudioTracks::files`] on goes
//! to a file of its own as it records, through `mpfree-stream`'s writer
//! ([`Recorder`]), by the recording file protocol
//! (docs/dev/design/recording-files.md): the files are made when its track
//! arms (`NAME_ID.wav.recording` with its checkpoint), committed in rounds
//! while it records, and finished as `NAME_ID.wav` after it stops. NAME is
//! the track's name and ID the take's lasting id ([`RecordingId`]).
//!
//! - [`Name`]: a track's name as it goes in a file name, kept without
//!   allocating, so the audio thread can hand it over.
//! - [`FileView`] and [`FileEvent`]: what the lanes and the log are told.
//! - [`Files`] (crate): the worker thread's side: the writer, and what is
//!   known of each recording's file.
//! - [`recover_into`]: at start, before anything records, takes left in the
//!   work folder by a crash, a power cut or a session never saved are
//!   finished and moved where the musician finds them. Nothing is deleted.
//! - [`Problem`]: what System status hears of the takes' files and the
//!   drive, through `mpfree-stream`'s [`Watch`] (phase 3b step 5d): each
//!   take as the writer shows it (at risk, not saved yet, may stop,
//!   stopped), the drive's data folder looked at every few seconds on a
//!   thread of its own ([`watch_drive`]: missing, nearly full), and what
//!   recovery found ([`Recovered`]).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_stream::fsio::{Fs, StdFs};
use mpfree_stream::player::UNIT;
use mpfree_stream::rec::{recording_path, Meta, RecFormat, RecId};
pub use mpfree_stream::rec::{TakeState, Why};
use mpfree_stream::recover::{check_whole, checked, recover, Found};
use mpfree_stream::status::{Drive, Watch};
pub use mpfree_stream::writer::WriterConfig;
use mpfree_stream::writer::{Gate, RealClock, Recorder, Status, TakeNo, TakeView};

use mpfree_status::{Report, Subject};

use crate::host::TakeWave;
use crate::save::TakeFile;
use crate::{RecordingId, TakeInfo, PEAK_BLOCK};

/// Bytes of a track's name kept for its take files' names.
pub const NAME_MAX: usize = 48;

/// A track's name as its take files are named: characters a file name
/// can't hold become `-`, spaces and dots go from its ends, and it is cut
/// to [`NAME_MAX`] bytes; "Audio" if nothing is left. Made and read
/// without allocating.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Name {
    bytes: [u8; NAME_MAX],
    len: u8,
}

impl Name {
    pub fn new(s: &str) -> Name {
        let mut n = Name {
            bytes: [0; NAME_MAX],
            len: 0,
        };
        let mut len = 0;
        for c in s.trim_start_matches([' ', '.']).chars() {
            let c = if c.is_control()
                || matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*')
            {
                '-'
            } else {
                c
            };
            let w = c.len_utf8();
            if len + w > NAME_MAX {
                break;
            }
            c.encode_utf8(&mut n.bytes[len..len + w]);
            len += w;
        }
        while len > 0 && matches!(n.bytes[len - 1], b' ' | b'.') {
            len -= 1;
        }
        if len == 0 {
            let d = b"Audio";
            n.bytes[..d.len()].copy_from_slice(d);
            len = d.len();
        }
        n.len = len as u8;
        n
    }

    pub fn as_str(&self) -> &str {
        // Whole characters were written, so this never fails.
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("Audio")
    }
}

impl Default for Name {
    fn default() -> Name {
        Name::new("")
    }
}

impl std::fmt::Debug for Name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Name({:?})", self.as_str())
    }
}

/// The file name of a recording's take: `NAME_ID.wav`.
pub fn file_name(name: &str, rec: RecordingId) -> String {
    format!("{name}_{rec}.wav")
}

/// The stream layer's id for a recording: the same 128 bits, so the file
/// names carry [`RecordingId`]'s 32 hex digits.
pub fn rec_id(rec: RecordingId) -> RecId {
    RecId(rec.0.to_be_bytes())
}

/// Where a take's file stands, as its lane shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FileView {
    /// Recording, Saving, Saved, At risk or Stopped (with why).
    pub state: TakeState,
    /// Seconds recorded but not yet safe in the file.
    pub unsaved_s: f32,
    /// Show "Unsaved: N s": more than twice the writer's round interval
    /// isn't safe yet.
    pub show_unsaved: bool,
    /// The writer's memory for it is at least 90 % full: recording may stop.
    pub nearly_full: bool,
}

impl FileView {
    /// A finished file (opened with a project, or saved).
    pub const SAVED: FileView = FileView {
        state: TakeState::Saved,
        unsaved_s: 0.0,
        show_unsaved: false,
        nearly_full: false,
    };

    fn of(v: &TakeView) -> FileView {
        FileView {
            state: v.state,
            unsaved_s: v.unsaved_s as f32,
            show_unsaved: v.show_unsaved,
            nearly_full: v.nearly_full,
        }
    }

    /// Whether the lane would show it differently from `o`: another state
    /// or warning, or the unsaved seconds on another whole second.
    fn differs(&self, o: &FileView) -> bool {
        self.state != o.state
            || self.show_unsaved != o.show_unsaved
            || self.nearly_full != o.nearly_full
            || (self.show_unsaved && self.unsaved_s.floor() != o.unsaved_s.floor())
    }
}

/// Something about the files the musician hears of (in the log).
#[derive(Clone, Debug, PartialEq)]
pub enum FileEvent {
    /// Track `track`'s next take's files couldn't be made (`why`), so the
    /// track was disarmed (the recording file protocol: a track that can't
    /// record safely doesn't arm).
    ArmFailed { track: usize, why: String },
    /// They couldn't be made, and the take had started already: it records
    /// into memory only, and a save writes it from there.
    NoFile { track: usize, why: String },
    /// A save wrote a take of `track` from memory, as its file isn't all
    /// there (`state`: how far it got; None when it never had one).
    FromMemory {
        track: usize,
        state: Option<TakeState>,
    },
    /// A take's file changed state: it is At risk or Stopped, or a take
    /// that was got over it (Saved after At risk).
    State { track: usize, state: TakeState },
    /// A take's file couldn't be opened to play from (`why`): a take
    /// recorded is `kept` in memory and plays from there; a take of a
    /// project opening isn't in memory, so it is left out.
    Unopened {
        track: usize,
        why: String,
        kept: bool,
    },
    /// Playing takes from their files couldn't start (`why`): every take
    /// plays from memory, as before.
    NoStream { why: String },
    /// A project's track went in without its regions, as it was recording
    /// when its takes' files had opened.
    TrackLeftOut { track: usize },
    /// The cache holds the starts of only `held` of the `of` regions
    /// playing from the drive: the others may start late.
    HeadsShort { held: usize, of: usize },
    /// A sound brought in, or a stretched copy, plays from memory, as its
    /// file couldn't be written.
    Unwritten { track: usize, why: String },
    /// A save wrote a take of `track` from what its file holds and the
    /// rest from memory, as its file isn't all there (`state`): its memory
    /// had gone to the file as it recorded (a take longer than memory).
    FromBoth {
        track: usize,
        state: Option<TakeState>,
    },
    /// The part of a take of `track` whose memory had gone to its file
    /// couldn't be read back from there (`why`) for a save, which holds
    /// silence there.
    Unread { track: usize, why: String },
    /// The drive a take of `track` recorded on went (the SD card taken
    /// out, or closed) and the take carries on in the folder recording
    /// moved to (the stick's): its first `hole_s` seconds stay on the
    /// drive that went, and play as silence until they are brought back.
    CarriedOn { track: usize, hole_s: f32 },
}

impl FileEvent {
    /// Engine track the event is about.
    pub fn track(&self) -> usize {
        match self {
            FileEvent::ArmFailed { track, .. }
            | FileEvent::NoFile { track, .. }
            | FileEvent::FromMemory { track, .. }
            | FileEvent::State { track, .. }
            | FileEvent::Unopened { track, .. }
            | FileEvent::Unwritten { track, .. }
            | FileEvent::FromBoth { track, .. }
            | FileEvent::Unread { track, .. }
            | FileEvent::CarriedOn { track, .. }
            | FileEvent::TrackLeftOut { track } => *track,
            FileEvent::NoStream { .. } | FileEvent::HeadsShort { .. } => 0,
        }
    }

    /// The log's line for it, in the musician's words; `who` names the
    /// track as the musician named it.
    pub fn line(&self, who: &str) -> String {
        match self {
            FileEvent::ArmFailed { why, .. } => format!(
                "audio tracks: {who} wasn't armed: its take's file couldn't be made ({why})"
            ),
            FileEvent::NoFile { why, .. } => format!(
                "audio tracks: {who}'s take records into memory only, as its file couldn't be \
                 made ({why}); saving the project keeps it"
            ),
            FileEvent::FromMemory { state, .. } => {
                format!(
                    "audio tracks: a take of {who} was saved from memory, as {}",
                    short_because(state)
                )
            }
            FileEvent::FromBoth { state, .. } => format!(
                "audio tracks: a take of {who} was saved from what reached the drive and the rest \
                 from memory, as {}",
                short_because(state)
            ),
            FileEvent::Unread { why, .. } => format!(
                "audio tracks: the start of a take of {who} couldn't be read back from the drive \
                 ({why}), so the saved take is silent there"
            ),
            FileEvent::CarriedOn { hole_s, .. } if *hole_s > 0.0 => format!(
                "audio tracks: {who}'s take carries on on the USB stick, as the SD card went; \
                 its first {hole_s:.1} s stay on the card and play as silence until it is back"
            ),
            FileEvent::CarriedOn { .. } => format!(
                "audio tracks: {who}'s take carries on on the USB stick, as the SD card went \
                 before any of it was saved there"
            ),
            FileEvent::State { state, .. } => match state {
                TakeState::AtRisk(w) => format!(
                    "audio tracks: {who}'s take is at risk ({}): if the power goes now, what \
                     isn't on the drive yet is lost; it goes on recording into memory",
                    why_words(*w)
                ),
                TakeState::Stopped(w) => format!(
                    "audio tracks: {who}'s take stopped going to the drive ({}); it goes on \
                     recording into memory, and saving the project keeps it",
                    why_words(*w)
                ),
                // Raised only after At risk.
                TakeState::Saved => {
                    format!("audio tracks: {who}'s take is safe on the drive again")
                }
                TakeState::Recording | TakeState::Saving => {
                    format!("audio tracks: {who}'s take is being written")
                }
            },
            FileEvent::Unopened {
                why, kept: true, ..
            } => format!(
                "audio tracks: {who}'s take plays from memory, as its file couldn't be opened \
                 ({why})"
            ),
            FileEvent::Unopened {
                why, kept: false, ..
            } => format!(
                "audio tracks: a take of {who} was left out, as its file couldn't be opened ({why})"
            ),
            FileEvent::Unwritten { why, .. } => format!(
                "audio tracks: {who}'s new take plays from memory, as its file couldn't be \
                 written ({why})"
            ),
            FileEvent::NoStream { why } => format!(
                "audio tracks: takes play from memory, as playing them from the drive couldn't \
                 start ({why})"
            ),
            FileEvent::HeadsShort { held, of } => format!(
                "audio tracks: the cache holds the starts of {held} of the {of} regions playing \
                 from the drive; the others may start late (a bigger Audio track memory helps)"
            ),
            FileEvent::TrackLeftOut { .. } => format!(
                "audio tracks: {who}'s regions in the project were left out, as it was recording \
                 when they went in"
            ),
        }
    }
}

/// Why a take saved from memory had no whole file, in a few words.
fn short_because(state: &Option<TakeState>) -> String {
    match state {
        Some(TakeState::AtRisk(w) | TakeState::Stopped(w)) => {
            format!("its file stopped short ({})", why_words(*w))
        }
        _ => "it has no file of its own".to_string(),
    }
}

/// Why a take's file got into trouble, in a few words.
pub fn why_words(w: Why) -> &'static str {
    match w {
        Why::Writes => "writing to the drive keeps failing",
        Why::Syncs => "the drive doesn't finish its writes",
        Why::Records => "its safety notes can't be written",
        Why::Full => "the drive is full",
        Why::Gone => "the drive is gone",
        Why::Finish => "the finished file couldn't be named",
        Why::Memory => "the drive fell too far behind",
    }
}

/// What the worker tells the audio thread about the files.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Said {
    /// The files for a track's next take are made.
    Ready(RecordingId),
    /// They couldn't be made.
    Failed { rec: RecordingId, why: String },
    /// A take's file stands at `view` now.
    View { rec: RecordingId, view: FileView },
    /// A take carried on in another folder ([`Files::carry_on`]), its
    /// first `hole` frames left on the drive that went.
    CarriedOn { rec: RecordingId, hole: u64 },
}

/// How often the worker looks at the writer's takes.
const POLL: Duration = Duration::from_millis(50);

/// One recording's file, on the worker.
struct FileRec {
    rec: RecordingId,
    /// The engine track it was armed on (0 for a take a project opened).
    track: u8,
    /// The take it is, once one records into it or opens with it.
    id: Option<u16>,
    /// 0 for a take opened with a project (it wasn't recorded here).
    channels: u8,
    /// The writer's number for it, while the writer has it.
    no: Option<TakeNo>,
    /// A take records (or recorded) into it: not only armed.
    take: bool,
    /// Its file's name (`NAME_ID.wav`, or the name a project gave it).
    name: String,
    /// The work folder its files were made in (None: a project opened
    /// it). Recording may have moved to another folder since
    /// ([`Files::start`]); its part file stays here.
    dir: Option<PathBuf>,
    /// The finished file, once saved (or as a project opened it).
    path: Option<PathBuf>,
    /// Frames the writer took for it (or the opened file holds).
    frames: usize,
    /// What the audio thread was last told.
    shown: Option<FileView>,
    /// The audio thread was told its files are made (armed only).
    ready: bool,
    /// The name the last save gave it.
    saved_as: Option<String>,
    /// Frames its file was opened or grown to while being written (0:
    /// not opened as it grows).
    grown: u64,
    /// While takes play from their files: the lowest and highest sample
    /// of each block of [`PEAK_BLOCK`] samples pushed (the last as far as
    /// it goes), so its waveform needs no read of its file, and `pushed`,
    /// the samples pushed.
    peaks: Vec<(f32, f32)>,
    pushed: usize,
    /// Asked to carry on in `dir` and not there yet ([`Files::carry_on`]):
    /// its file isn't opened or grown meanwhile.
    moving: bool,
    /// Frames at its start left on a drive that went (it carried on): its
    /// file in the work folder holds zeros there, and stays for them to be
    /// brought back, whatever a save copies.
    hole: u64,
}

/// The writer and the folder it records into.
struct Writing {
    dir: PathBuf,
    rec: Recorder,
}

/// A take's finished file to open as its stream source: the take, its
/// lasting id, the file, and the channels and frames it must hold.
pub(crate) type ToOpen = (u16, RecordingId, PathBuf, usize, usize);

/// A recording's file still being written, played from as it grows
/// (phase 3b step 5): its pool memory goes back as the file holds it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Growing {
    /// Take `take` has committed its first `frames` (whole units of the
    /// stream cache) to the file at `path`, of `channels`: open it.
    Open {
        take: u16,
        rec: RecordingId,
        path: PathBuf,
        channels: usize,
        frames: u64,
    },
    /// Its file holds its first `frames` now: more units committed.
    Grow {
        take: u16,
        rec: RecordingId,
        frames: u64,
    },
    /// It is finished, named `path` in place, and holds all `frames` it
    /// took: grown to them, or opened as a finished file if it didn't
    /// open as it grew after all.
    Finished {
        take: u16,
        rec: RecordingId,
        path: PathBuf,
        channels: usize,
        frames: usize,
    },
}

/// The worker's side of the files: see the module notes.
pub(crate) struct Files {
    rate: u32,
    writing: Option<Writing>,
    recs: Vec<FileRec>,
    last_poll: Instant,
    /// Takes play from their files (phase 3b): each take's file is to be
    /// opened once finished ([`Files::take_opens`]), or as it grows while
    /// it records ([`Files::take_grows`]).
    streams: bool,
    opens: Vec<ToOpen>,
    grows: Vec<Growing>,
    /// The takes' problems for System status, and those not taken yet.
    watch: Watch,
    problems: Vec<Problem>,
    /// Bytes a second the takes armed or recording take (for the drive's
    /// look: nearly full by what is armed).
    armed: Arc<AtomicU64>,
    /// Loop layers' files written in the work folder ([`LayerWrite`]):
    /// each name and path, until a save holds it.
    layer_files: Vec<(String, PathBuf)>,
    /// The work folders recorded into before this one ([`Files::start`]
    /// again: the SD card's opened): a save takes files over from them
    /// too.
    was: Vec<PathBuf>,
}

impl Files {
    pub fn new(rate: u32) -> Files {
        Files {
            rate,
            writing: None,
            recs: Vec::new(),
            last_poll: Instant::now(),
            streams: false,
            opens: Vec::new(),
            grows: Vec::new(),
            watch: Watch::new(),
            problems: Vec::new(),
            armed: Arc::new(AtomicU64::new(0)),
            layer_files: Vec::new(),
            was: Vec::new(),
        }
    }

    /// A loop layer's file was written whole in the work folder.
    pub fn layer_written(&mut self, name: String, path: PathBuf) {
        self.layer_files.retain(|(n, _)| *n != name);
        self.layer_files.push((name, path));
    }

    /// Where loop layer file `name` this run wrote is now: in the work
    /// folder, or in the project's `samples/` once a save holds a copy
    /// (phase 4d.4, for UNDO to bring the layer back from it).
    pub fn layer_path(&self, name: &str) -> Option<&Path> {
        self.layer_files
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, p)| p.as_path())
    }

    /// What System status is to hear of the takes since last asked.
    pub fn take_problems(&mut self) -> Vec<Problem> {
        std::mem::take(&mut self.problems)
    }

    /// Bytes a second the takes armed or recording take, kept up to date
    /// as the writer is looked at.
    pub fn armed(&self) -> Arc<AtomicU64> {
        self.armed.clone()
    }

    /// Takes play from their files from now on: each finished take file,
    /// recorded or opened with a project, is to be opened.
    pub fn stream(&mut self) {
        self.streams = true;
    }

    /// The take files to open now, oldest first.
    pub fn take_opens(&mut self) -> Vec<ToOpen> {
        std::mem::take(&mut self.opens)
    }

    /// Recordings' files to open as they grow, or grown since, oldest
    /// first.
    pub fn take_grows(&mut self) -> Vec<Growing> {
        std::mem::take(&mut self.grows)
    }

    /// The folder takes record into, once there is one.
    pub fn work_dir(&self) -> Option<&Path> {
        self.writing.as_ref().map(|w| w.dir.as_path())
    }

    /// Where recording `rec`'s finished file is, if it has one.
    pub fn path_of(&self, rec: RecordingId) -> Option<PathBuf> {
        self.recs.iter().find(|f| f.rec == rec)?.path.clone()
    }

    /// Whether the writer runs (the worker then wakes to look at it).
    pub fn writing(&self) -> bool {
        self.writing.is_some()
    }

    /// While the writer runs: the folder it records into, and how long its
    /// rounds that wrote to the drive took (its sync times).
    pub fn syncs(&self) -> Option<(&Path, mpfree_stream::times::Summary)> {
        let w = self.writing.as_ref()?;
        Some((w.dir.as_path(), w.rec.stats().syncs))
    }

    /// Record into `dir` from now on (made if missing), the writer's
    /// rounds waiting for playback's reads through `gate` (the stream
    /// coordinator's, while takes play from their files).
    ///
    /// Again, to move to another folder: the writer before closes, once
    /// the audio thread has seen every take finished and the armed files
    /// gone ([`crate::host::AudioHost::record_to`]). Should one still be
    /// in it, it is left as a recording for the next start's recovery,
    /// and forgotten here (its number was that writer's).
    pub fn start(&mut self, dir: PathBuf, cfg: WriterConfig, gate: Arc<Gate>) -> io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        if let Some(old) = self.writing.take() {
            old.rec.close();
            let Files {
                recs,
                watch,
                problems,
                was,
                ..
            } = self;
            for f in recs.iter_mut() {
                if f.no.take().is_some() && f.take {
                    watch.take_gone(track_subject(f.track), &mut |r| {
                        problems.push(Problem::of(r))
                    });
                }
            }
            recs.retain(|f| f.take);
            if old.dir != dir && !was.contains(&old.dir) {
                was.push(old.dir);
            }
        }
        let rec = Recorder::start(
            Box::new(StdFs::new()),
            Arc::new(RealClock::new()),
            gate,
            cfg,
        )?;
        self.writing = Some(Writing { dir, rec });
        Ok(())
    }

    /// The drive takes record on went (the SD card taken out, or closed,
    /// the SD card plan's B2c): every take in flight in the folder there,
    /// recording, being finished or only armed, carries on in `to` (made
    /// if missing), the same writer going on there
    /// ([`Recorder::carry_on`]), and takes record into `to` from now on.
    /// What they committed stays on the drive that went. Each take's file
    /// opens again to play from once it is there ([`Said::CarriedOn`]).
    /// Gives how many were carried; nothing when recording is already in
    /// `to`.
    pub fn carry_on(&mut self, to: PathBuf) -> io::Result<usize> {
        let Some(w) = &mut self.writing else {
            return Ok(0);
        };
        if w.dir == to {
            return Ok(0);
        }
        std::fs::create_dir_all(&to)?;
        let old = std::mem::replace(&mut w.dir, to.clone());
        if !self.was.contains(&old) {
            self.was.push(old.clone());
        }
        let mut n = 0;
        for f in &mut self.recs {
            let Some(no) = f.no else { continue };
            if f.dir.as_deref() != Some(old.as_path()) {
                continue;
            }
            w.rec.carry_on(no, &to, [0; 16], 0);
            f.dir = Some(to.clone());
            f.moving = true;
            n += 1;
        }
        Ok(n)
    }

    fn find(&mut self, rec: RecordingId) -> Option<&mut FileRec> {
        self.recs.iter_mut().find(|f| f.rec == rec)
    }

    /// Make the files for track `track`'s next take; a failure is said now
    /// or once the writer has tried.
    pub fn arm(&mut self, track: u8, rec: RecordingId, channels: u8, name: Name) -> Option<Said> {
        let Some(w) = &mut self.writing else {
            return Some(Said::Failed {
                rec,
                why: "recording to files isn't on".into(),
            });
        };
        let recorded_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let no = w.rec.arm(
            &w.dir,
            name.as_str(),
            rec_id(rec),
            RecFormat {
                float: true,
                channels: u16::from(channels),
                rate: self.rate,
            },
            Meta {
                project: [0; 16],
                track: u16::from(track),
                start_ticks: 0,
                recorded_ms,
            },
        );
        self.recs.push(FileRec {
            rec,
            track,
            id: None,
            channels,
            no: Some(no),
            take: false,
            name: file_name(name.as_str(), rec),
            dir: Some(w.dir.clone()),
            path: None,
            frames: 0,
            shown: None,
            ready: false,
            saved_as: None,
            grown: 0,
            peaks: Vec::new(),
            pushed: 0,
            moving: false,
            hole: 0,
        });
        None
    }

    /// The files made for `rec` won't be recorded into: they go.
    pub fn disarm(&mut self, rec: RecordingId) {
        let Some(i) = self.recs.iter().position(|f| f.rec == rec && !f.take) else {
            return;
        };
        let f = self.recs.swap_remove(i);
        if let (Some(w), Some(no)) = (&mut self.writing, f.no) {
            w.rec.disarm(no);
        }
    }

    /// Take `id` started recording into `rec`'s files. None if it has
    /// none (they failed, or the request was lost): why, for the musician.
    pub fn start_take(&mut self, rec: RecordingId, id: u16) -> Result<(), String> {
        match self.find(rec) {
            Some(f) if f.no.is_some() => {
                f.take = true;
                f.id = Some(id);
                Ok(())
            }
            _ => Err("its files weren't made".into()),
        }
    }

    /// Frames of a take, interleaved.
    pub fn push(&mut self, rec: RecordingId, frames: &[f32]) {
        let Some(w) = &mut self.writing else { return };
        let Some(f) = self.recs.iter_mut().find(|f| f.rec == rec) else {
            return;
        };
        if let Some(no) = f.no {
            // The frames it took: fewer once its memory is full.
            f.frames += w.rec.push(no, frames);
        }
        if self.streams {
            note_peaks(&mut f.peaks, &mut f.pushed, frames);
        }
    }

    /// Take `take`'s waveform in about `columns` columns, as
    /// [`crate::host::AudioHost::wave`] reads it, from the peaks of what
    /// was pushed to its file (as far as that goes: a take recording
    /// grows): None if it has none (it wasn't recorded here while takes
    /// play from their files).
    pub fn wave_of(&self, take: &TakeInfo, columns: usize) -> Option<TakeWave> {
        let ch = take.channels.max(1);
        let f = self
            .recs
            .iter()
            .find(|f| f.rec == take.rec && f.id == Some(take.id) && usize::from(f.channels) == ch)
            .filter(|f| !f.peaks.is_empty())?;
        let frames = (f.pushed / ch).min(take.frames);
        let step = (PEAK_BLOCK / ch).max(1);
        let fpc = take.frames.div_ceil(columns.max(1)).div_ceil(step).max(1) * step;
        let mut out = Vec::with_capacity(frames.div_ceil(fpc));
        let mut at = 0;
        while at < frames {
            let to = (at + fpc).min(frames);
            let blocks = &f.peaks[at * ch / PEAK_BLOCK..(to * ch).div_ceil(PEAK_BLOCK)];
            out.push(
                blocks
                    .iter()
                    .fold((f32::MAX, f32::MIN), |(l, h), &(a, b)| (l.min(a), h.max(b))),
            );
            at = to;
        }
        Some(TakeWave {
            id: take.id,
            frames,
            channels: take.channels,
            frames_per_column: fpc,
            columns: out,
        })
    }

    /// The take stopped and every frame was pushed: its file is finished.
    pub fn stop(&mut self, rec: RecordingId) {
        let Some(w) = &mut self.writing else { return };
        if let Some(no) = self.recs.iter().find(|f| f.rec == rec).and_then(|f| f.no) {
            w.rec.stop(no);
        }
    }

    /// Take `id` opened with a project: its file's name there, where it
    /// is, and the channels and frames it holds. While takes play from
    /// their files, a take with a file is to be opened.
    pub fn known(
        &mut self,
        rec: RecordingId,
        id: u16,
        name: String,
        path: Option<PathBuf>,
        (channels, frames): (usize, usize),
    ) {
        self.recs.retain(|f| f.rec != rec);
        if let (true, Some(p)) = (self.streams, &path) {
            self.opens.push((id, rec, p.clone(), channels, frames));
        }
        self.recs.push(FileRec {
            rec,
            track: 0,
            id: Some(id),
            channels: 0,
            no: None,
            take: true,
            name,
            dir: None,
            path,
            frames,
            shown: Some(FileView::SAVED),
            ready: true,
            saved_as: None,
            grown: 0,
            peaks: Vec::new(),
            pushed: 0,
            moving: false,
            hole: 0,
        });
    }

    /// The tracks were emptied: every take's file is let go of (the files
    /// stay where they are). Files still only armed are left for their
    /// [`Files::disarm`].
    pub fn clear(&mut self) {
        let Files {
            writing,
            recs,
            watch,
            problems,
            ..
        } = self;
        recs.retain(|f| {
            if !f.take {
                return true;
            }
            if let (Some(w), Some(no)) = (writing.as_mut(), f.no) {
                w.rec.forget(no);
                watch.take_gone(track_subject(f.track), &mut |r| {
                    problems.push(Problem::of(r))
                });
            }
            false
        });
    }

    /// Look at the writer's takes (at most every [`POLL`]): what changed.
    pub fn poll(&mut self, force: bool) -> Vec<Said> {
        let mut out = Vec::new();
        let Some(w) = &mut self.writing else {
            return out;
        };
        if !force && self.last_poll.elapsed() < POLL {
            return out;
        }
        self.last_poll = Instant::now();
        let mut gone = Vec::new();
        let rate = u64::from(self.rate);
        let bytes: u64 = self
            .recs
            .iter()
            .filter(|f| f.no.is_some())
            .map(|f| u64::from(f.channels) * 4 * rate)
            .sum();
        self.armed.store(bytes, Ordering::Relaxed);
        let problems = &mut self.problems;
        let mut report = |r: Report| problems.push(Problem::of(r));
        for f in &mut self.recs {
            let Some(no) = f.no else { continue };
            let subject = track_subject(f.track);
            match w.rec.status(no) {
                None | Some(Status::Arming) => {}
                Some(Status::Failed(why)) => {
                    out.push(Said::Failed { rec: f.rec, why });
                    w.rec.disarm(no);
                    f.no = None;
                    if f.take {
                        self.watch.take_gone(subject, &mut report);
                    } else {
                        gone.push(f.rec);
                    }
                }
                Some(Status::Take(v)) => {
                    if f.moving && !v.carrying {
                        // The writer took the ask (its status says carrying
                        // until then): the take is in the new folder
                        // (carried on), or still where it was (saved, or
                        // being finished, there: nothing to carry).
                        let at = w.rec.final_path(no);
                        let dir = at.as_deref().and_then(Path::parent).map(Path::to_path_buf);
                        f.moving = false;
                        if dir.is_some() && dir == f.dir {
                            // Opened again from its file there.
                            f.grown = 0;
                            f.hole = v.hole;
                            if f.take {
                                out.push(Said::CarriedOn {
                                    rec: f.rec,
                                    hole: v.hole,
                                });
                            }
                        } else if dir.is_some() {
                            f.dir = dir;
                        }
                    }
                    if !f.take {
                        if !f.ready {
                            f.ready = true;
                            out.push(Said::Ready(f.rec));
                        }
                        continue;
                    }
                    let view = FileView::of(&v);
                    self.watch.take(subject, &v, &mut report);
                    let ch = usize::from(f.channels);
                    // Whole units of the cache committed: the file plays
                    // them from now on, and their memory goes.
                    let unit = (UNIT / ch.max(1)) as u64;
                    let to = v.committed / unit * unit;
                    let more = v.state != TakeState::Saved && to > f.grown && !f.moving;
                    if let Some(id) = f.id.filter(|_| self.streams && more) {
                        let stem = f.name.strip_suffix(".wav").unwrap_or(&f.name);
                        self.grows.push(if f.grown == 0 {
                            Growing::Open {
                                take: id,
                                rec: f.rec,
                                path: recording_path(f.dir.as_deref().unwrap_or(&w.dir), stem),
                                channels: ch,
                                frames: to,
                            }
                        } else {
                            Growing::Grow {
                                take: id,
                                rec: f.rec,
                                frames: to,
                            }
                        });
                        f.grown = to;
                    }
                    if v.state == TakeState::Saved {
                        f.path = w.rec.final_path(no);
                        // The writer is done with it.
                        w.rec.forget(no);
                        f.no = None;
                        self.watch.take_gone(subject, &mut report);
                        if let (true, Some(id), Some(p)) = (self.streams, f.id, &f.path) {
                            if f.grown > 0 {
                                // Opened as it grew, and named in place:
                                // the same file holds all of it now.
                                f.grown = f.frames as u64;
                                self.grows.push(Growing::Finished {
                                    take: id,
                                    rec: f.rec,
                                    path: p.clone(),
                                    channels: ch,
                                    frames: f.frames,
                                });
                            } else {
                                self.opens.push((id, f.rec, p.clone(), ch, f.frames));
                            }
                        }
                    }
                    if f.shown.is_none_or(|s| view.differs(&s)) {
                        f.shown = Some(view);
                        out.push(Said::View { rec: f.rec, view });
                    }
                }
            }
        }
        self.recs.retain(|f| !gone.contains(&f.rec));
        out
    }

    /// Name each of a save's takes and say where its audio comes from: a
    /// finished file that holds all of it (`path`, no audio to read), or
    /// memory (`audio` made room for, read by the audio thread). A take
    /// recorded with a file is named for it (`NAME_ID.wav`), one opened
    /// keeps its project's name, and any other is `take_ID.wav`. What was
    /// saved from memory though it was recorded to a file is said.
    pub fn name_save(&mut self, takes: &mut [TakeFile]) -> Vec<FileEvent> {
        let mut said = Vec::new();
        for k in takes {
            k.file.clear();
            k.path = None;
            let f = self.recs.iter_mut().find(|f| f.rec == k.rec);
            let whole = f
                .as_ref()
                .and_then(|f| f.path.as_ref().filter(|_| f.frames == k.frames));
            match (whole.cloned(), f) {
                (Some(path), Some(f)) => {
                    k.file.push_str(&f.name);
                    k.path = Some(path);
                    f.saved_as = Some(f.name.clone());
                }
                (None, Some(f)) => {
                    k.file.push_str(&f.name);
                    f.saved_as = Some(f.name.clone());
                    k.audio = vec![0.0; k.frames * k.channels];
                    let state = f.shown.map(|s| s.state);
                    if f.grown > 0 {
                        // Its memory may have gone to its file as it
                        // recorded: what the file holds comes from there,
                        // and the tracks fill in the rest from memory.
                        let stem = f.name.strip_suffix(".wav").unwrap_or(&f.name);
                        let path = f
                            .path
                            .clone()
                            .or_else(|| f.dir.as_ref().map(|d| recording_path(d, stem)))
                            .or_else(|| {
                                self.writing.as_ref().map(|w| recording_path(&w.dir, stem))
                            });
                        let read = match path {
                            Some(p) => read_frames(&p, k.channels, &mut k.audio),
                            None => Err("it has no file".into()),
                        };
                        said.push(match read {
                            Ok(()) => FileEvent::FromBoth {
                                track: k.track,
                                state,
                            },
                            Err(why) => FileEvent::Unread {
                                track: k.track,
                                why,
                            },
                        });
                    } else if f.channels > 0 {
                        said.push(FileEvent::FromMemory {
                            track: k.track,
                            state,
                        });
                    }
                }
                (_, None) => {
                    k.file.push_str(&format!("take_{}.wav", k.rec));
                    k.audio = vec![0.0; k.frames * k.channels];
                }
            }
        }
        said
    }

    /// A project save finished into `dir` (its `samples/`): the takes it
    /// holds a file of are read from there from now on, and their copies
    /// in the work folder go (the project's are safe on the stick now).
    pub fn saved_into(&mut self, dir: &Path) {
        // The work folder, and any recorded into before it this run.
        let work = self.writing.as_ref().map(|w| w.dir.as_path());
        let was = &self.was;
        let in_work = |p: &Path| {
            p.parent()
                .is_some_and(|d| Some(d) == work || was.iter().any(|w| w == d))
        };
        for f in &mut self.recs {
            let Some(name) = f.saved_as.as_ref() else {
                continue;
            };
            let there = dir.join(name);
            if !std::fs::symlink_metadata(&there).is_ok_and(|m| m.is_file()) {
                continue;
            }
            if let Some(old) = &f.path {
                if in_work(old) && *old != there && f.no.is_none() && f.hole == 0 {
                    let _ = std::fs::remove_file(old);
                }
            }
            f.name.clone_from(name);
            f.path = Some(there);
        }
        // Loop layers' files the save holds a copy of go from the work
        // folder too (phase 4c.3); the rest stay for the next save, or
        // the next start's recovery.
        // Only a file directly in the work folder is removed.
        // The copy is where it is from now on (UNDO may read it back).
        for (name, path) in &mut self.layer_files {
            let there = dir.join(&*name);
            let held = std::fs::symlink_metadata(&there).is_ok_and(|m| m.is_file());
            if !held || *path == there || !in_work(path) {
                continue;
            }
            let _ = std::fs::remove_file(&*path);
            *path = there;
        }
    }
}

/// `samples` follow the `pushed` samples taken into `peaks` (see
/// [`FileRec`]'s).
fn note_peaks(peaks: &mut Vec<(f32, f32)>, pushed: &mut usize, samples: &[f32]) {
    let mut j = 0;
    while j < samples.len() {
        let b = *pushed / PEAK_BLOCK;
        let n = ((b + 1) * PEAK_BLOCK - *pushed).min(samples.len() - j);
        let (lo, hi) = crate::min_max(&samples[j..j + n]);
        match peaks.get_mut(b) {
            Some(p) => *p = (p.0.min(lo), p.1.max(hi)),
            None => peaks.push((lo, hi)),
        }
        *pushed += n;
        j += n;
    }
}

/// Read the frames a take's file at `path` holds (still being written, or
/// finished) into the start of `out` (interleaved, `channels`), as many
/// as both have.
fn read_frames(path: &Path, channels: usize, out: &mut [f32]) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let l = mpfree_stream::format::layout(&mut f).map_err(|e| e.to_string())?;
    if usize::from(l.channels) != channels {
        return Err(format!(
            "it has {} channels, the take {channels}",
            l.channels
        ));
    }
    let n = (l.frames as usize).min(out.len() / channels.max(1));
    let mut bytes = vec![0u8; n * l.block_align()];
    f.seek(SeekFrom::Start(l.data_at))
        .and_then(|_| f.read_exact(&mut bytes))
        .map_err(|e| e.to_string())?;
    mpfree_stream::format::decode(l.sample, l.endian, &bytes, &mut out[..n * channels]);
    Ok(())
}

/// A report for System status about the takes' files or the drive, and
/// the engine track it is about, if any: the app names it
/// (`Subject::audio_track` with the track's lasting id), as only it
/// knows which project track an engine track plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Problem {
    pub report: Report,
    pub track: Option<usize>,
}

impl Problem {
    /// A watch's report: about an engine track when its subject is one
    /// ([`track_subject`]), else about the whole system.
    pub(crate) fn of(r: Report) -> Problem {
        let track = (r.subject != Subject::NONE).then(|| r.subject.0 as usize - 1);
        Problem {
            report: Report {
                subject: Subject::NONE,
                ..r
            },
            track,
        }
    }
}

/// The subject the watch keeps engine track `track`'s takes under (one
/// take records on a track at a time, and a take's problems end before
/// the next begins); [`Problem::of`] takes it off again.
fn track_subject(track: u8) -> Subject {
    Subject(u32::from(track) + 1)
}

/// What a look at the drive holding the work folder found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Room {
    /// It answered and isn't read-only.
    pub writable: bool,
    /// Bytes free for MPFree.
    pub free: u64,
    /// The drive's size, bytes.
    pub total: u64,
}

/// The drive's last look, for System status's stick meter: the drive's
/// own thread ([`watch_drive`]) writes it each look, and the screen reads
/// it without waiting. The free space and the size may come from two
/// looks running, which a meter doesn't mind.
#[derive(Debug, Default)]
pub struct DriveSeen {
    /// 0: not looked at yet (no work folder, or not started); 1: there;
    /// 2: it didn't answer.
    state: AtomicU8,
    writable: AtomicU8,
    free: AtomicU64,
    total: AtomicU64,
    /// The drive watched is the SD card (S2b, phase B2b).
    card: AtomicBool,
}

impl DriveSeen {
    /// Whether the drive watched is the SD card, not the stick.
    pub fn on_card(&self) -> bool {
        self.card.load(Ordering::Relaxed)
    }

    /// A look's answer: None when the drive didn't answer.
    pub fn set(&self, room: Option<Room>) {
        match room {
            Some(r) => {
                self.free.store(r.free, Ordering::Relaxed);
                self.total.store(r.total, Ordering::Relaxed);
                self.writable.store(u8::from(r.writable), Ordering::Relaxed);
                self.state.store(1, Ordering::Release);
            }
            None => self.state.store(2, Ordering::Release),
        }
    }

    /// The last look: None before the first, Some(None) when the drive
    /// didn't answer.
    pub fn get(&self) -> Option<Option<Room>> {
        match self.state.load(Ordering::Acquire) {
            1 => Some(Some(Room {
                writable: self.writable.load(Ordering::Relaxed) == 1,
                free: self.free.load(Ordering::Relaxed),
                total: self.total.load(Ordering::Relaxed),
            })),
            2 => Some(None),
            _ => None,
        }
    }
}

/// Looks at the drive holding a folder (`statvfs`, in the app): None if
/// it couldn't (the drive isn't there). It may wait on a failing drive,
/// so it runs on a thread of its own ([`watch_drive`]).
pub type DriveProbe = fn(&Path) -> Option<Room>;

/// How often the drive is looked at.
pub const DRIVE_EVERY: Duration = Duration::from_secs(3);

/// Look at the drive holding `dir` every [`DRIVE_EVERY`], on a thread of
/// its own, until `stop`'s sender goes: Stick missing (it didn't answer,
/// or is read-only) and Stick nearly full (under 1 GB free, or ten
/// minutes of what `armed` says the armed takes record, a second), each to
/// `report` as it begins and ends; Card missing and Card nearly full
/// instead when the drive is the SD card (`card`). A folder sent on
/// `moved` (the takes record somewhere else now, and whether it is on the
/// card) is looked at from then on, at once, by the same watch, so the
/// last drive's problems end as the new one's begin. It stops once
/// `moved`'s sender goes, or `report` says false (the host is gone). Each
/// look also goes in `seen`, for the page's meter.
pub fn watch_drive(
    dir: PathBuf,
    card: bool,
    probe: DriveProbe,
    armed: Arc<AtomicU64>,
    seen: Arc<DriveSeen>,
    moved: std::sync::mpsc::Receiver<(PathBuf, bool)>,
    report: impl Fn(Problem) -> bool + Send + 'static,
) -> io::Result<()> {
    std::thread::Builder::new()
        .name("mpfree-at-drive".into())
        .spawn(move || {
            let mut watch = Watch::new();
            let (mut dir, mut card) = (dir, card);
            loop {
                seen.card.store(card, Ordering::Relaxed);
                let room = probe(&dir);
                seen.set(room);
                let d = Drive {
                    card,
                    writable: room.is_some_and(|r| r.writable),
                    free: room.map_or(0, |r| r.free),
                    armed_bytes_per_s: armed.load(Ordering::Relaxed),
                };
                let mut live = true;
                watch.drive(&d, &mut |r| live &= report(Problem::of(r)));
                if !live {
                    return;
                }
                match moved.recv_timeout(DRIVE_EVERY) {
                    Ok((d, c)) => (dir, card) = (d, c),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        })
        .map(|_| ())
}

/// How far ahead of the playhead the cache must be before a take file is
/// written a piece further ([`write_take`]), and the longest it waits for
/// that, a piece at a time.
const WRITE_CUSHION_S: f64 = 1.0;
const WRITE_WAIT: Duration = Duration::from_secs(1);
/// A take file is written this many bytes at a time.
const WRITE_PIECE: usize = 1 << 20;

/// Write a sound brought in, or a stretched copy, as recording `rec`'s
/// take file in `dir`: `take_ID.wav`, 32-bit float at `rate` (as a
/// recorded take's), written to a hidden `.part` file made new, synced,
/// then renamed, so its name never holds half a file. Through the stream
/// layer's file layer, as every take file is (a `.part` a cut-off write
/// left goes first). A piece at a time, each once the cache is
/// [`WRITE_CUSHION_S`] ahead of what plays (or after [`WRITE_WAIT`]), so
/// playback's reads come first as the writer's do. Its name and path.
pub fn write_take(
    dir: &Path,
    rec: RecordingId,
    channels: usize,
    rate: u32,
    audio: &[f32],
    gate: &Gate,
) -> io::Result<(String, PathBuf)> {
    use std::io::Write;
    /// Bytes written on at the end of a file, through the file layer.
    struct At<'a> {
        fs: &'a mut StdFs,
        path: &'a Path,
        at: u64,
    }
    impl Write for At<'_> {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.fs.write_at(self.path, self.at, b)?;
            self.at += b.len() as u64;
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    /// Writes in pieces, each once playback's cushion allows.
    struct Gated<'a, W: Write> {
        w: std::io::BufWriter<W>,
        gate: &'a Gate,
        since: usize,
    }
    impl<W: Write> Write for Gated<'_, W> {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            if self.since >= WRITE_PIECE {
                self.w.flush()?;
                self.since = 0;
                let t0 = Instant::now();
                while self.gate.cushion() < WRITE_CUSHION_S && t0.elapsed() < WRITE_WAIT {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            let n = self.w.write(b)?;
            self.since += n;
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.w.flush()
        }
    }
    let name = format!("take_{rec}.wav");
    let path = dir.join(&name);
    let part = dir.join(format!(".{name}.part"));
    let mut fs = StdFs::new();
    if fs.exists(&part) {
        let _ = fs.remove(&part);
    }
    let written = (|| {
        fs.create(&part, true)?;
        let mut w = Gated {
            w: std::io::BufWriter::with_capacity(
                1 << 16,
                At {
                    fs: &mut fs,
                    path: &part,
                    at: 0,
                },
            ),
            gate,
            since: 0,
        };
        crate::save::write_wav(&mut w, audio, channels, rate)?;
        w.flush()?;
        drop(w);
        fs.sync(&part)?;
        fs.rename(&part, &path)?;
        fs.sync_dir(dir)
    })();
    if let Err(e) = written {
        let _ = fs.remove(&part);
        return Err(e);
    }
    Ok((name, path))
}

/// A loop layer's recording's file name in the work folder and a
/// project's `samples/` (phase 4c.3): `loop_ID.wav`, as the app's save
/// names it.
pub fn layer_file_name(rec: RecordingId) -> String {
    format!("loop_{rec}.wav")
}

/// A loop's layer written to its recording's file in the work folder as
/// the audio thread reads it from the looper, a piece at a time (audio
/// tracks phase 4c.3, docs/dev/design/loops-on-audio-tracks.md, rule 4):
/// `loop_ID.wav` ([`layer_file_name`]), 32-bit float, to a hidden `.part`
/// file made new, synced once all of it is in, then renamed, as
/// [`write_take`]'s files are. A piece waits while playback's cache is
/// less than [`WRITE_CUSHION_S`] ahead (at most [`WRITE_WAIT`]), so
/// playback's reads come first, without the worker waiting: it is
/// written as the worker next wakes. A run cut off after the rename
/// leaves the file for the next start to find ([`recover_into`]).
pub(crate) struct LayerWrite {
    pub rec: RecordingId,
    /// Samples the file holds once whole, and written so far.
    samples: usize,
    put: usize,
    name: String,
    part: PathBuf,
    path: PathBuf,
    fs: StdFs,
    /// Bytes for one piece, made once.
    bytes: Vec<u8>,
    /// Pieces given and not written yet (each buffer with its samples),
    /// oldest first, and since when the oldest has waited.
    queue: std::collections::VecDeque<(Vec<f32>, usize)>,
    since: Option<Instant>,
    /// Every piece has been given.
    ended: bool,
}

impl LayerWrite {
    /// Start recording `rec`'s file in `dir`: `frames` frames of
    /// `channels` at `rate`, its header written (a `.part` an earlier
    /// write left goes first).
    pub fn begin(
        dir: &Path,
        rec: RecordingId,
        frames: usize,
        channels: usize,
        rate: u32,
    ) -> io::Result<LayerWrite> {
        let name = layer_file_name(rec);
        let mut w = LayerWrite {
            rec,
            samples: frames * channels.clamp(1, 2),
            put: 0,
            part: dir.join(format!(".{name}.part")),
            path: dir.join(&name),
            name,
            fs: StdFs::new(),
            bytes: Vec::with_capacity(4 * crate::pool::CHUNK),
            queue: std::collections::VecDeque::new(),
            since: None,
            ended: false,
        };
        if w.fs.exists(&w.part) {
            let _ = w.fs.remove(&w.part);
        }
        let header = crate::save::wav_header(w.samples, channels, rate);
        let made =
            w.fs.create(&w.part, true)
                .and_then(|()| w.fs.write_at(&w.part, 0, &header));
        if let Err(e) = made {
            let _ = w.fs.remove(&w.part);
            return Err(e);
        }
        Ok(w)
    }

    /// The first `len` samples of `buf` are the next piece.
    pub fn give(&mut self, buf: Vec<f32>, len: usize) {
        if self.queue.is_empty() {
            self.since = Some(Instant::now());
        }
        self.queue.push_back((buf, len));
    }

    /// Every piece has been given.
    pub fn end(&mut self) {
        self.ended = true;
    }

    /// Write the pieces waiting, while playback's cache is `cushion`
    /// seconds ahead (None: nothing plays from files) or the oldest has
    /// waited long enough; each buffer written goes into `spent`. Writing
    /// more than the file holds is refused.
    pub fn tend(&mut self, cushion: Option<f64>, spent: &mut Vec<Vec<f32>>) -> io::Result<()> {
        while let Some((buf, len)) = self.queue.pop_front() {
            let waited = self.since.is_some_and(|t| t.elapsed() >= WRITE_WAIT);
            if cushion.is_some_and(|c| c < WRITE_CUSHION_S) && !waited {
                self.queue.push_front((buf, len));
                return Ok(());
            }
            let piece = &buf[..len.min(buf.len())];
            if self.put + piece.len() > self.samples {
                spent.push(buf);
                return Err(io::Error::other("more audio than its layer holds"));
            }
            self.bytes.clear();
            for v in piece {
                self.bytes.extend_from_slice(&v.to_le_bytes());
            }
            let at = 44 + 4 * self.put as u64;
            let wrote = self.fs.write_at(&self.part, at, &self.bytes);
            self.put += piece.len();
            spent.push(buf);
            wrote?;
            self.since = Some(Instant::now());
        }
        Ok(())
    }

    /// Every piece is given and written.
    pub fn done(&self) -> bool {
        self.ended && self.queue.is_empty()
    }

    /// The whole file synced, then named, and its folder synced: its name
    /// and path. Short of its layer's audio, it goes and that is the
    /// error.
    pub fn finish(mut self) -> io::Result<(String, PathBuf)> {
        let dir = self
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let finished = if self.put == self.samples {
            self.fs
                .sync(&self.part)
                .and_then(|()| self.fs.rename(&self.part, &self.path))
                .and_then(|()| self.fs.sync_dir(&dir))
        } else {
            Err(io::Error::other(format!(
                "{} of its {} samples came",
                self.put, self.samples
            )))
        };
        match finished {
            Ok(()) => Ok((std::mem::take(&mut self.name), self.path.clone())),
            Err(e) => {
                let _ = self.fs.remove(&self.part);
                Err(e)
            }
        }
    }

    /// Stopped before it was whole: its `.part` goes; the buffers still
    /// waiting go into `spent`.
    pub fn abandon(mut self, spent: &mut Vec<Vec<f32>>) {
        spent.extend(self.queue.drain(..).map(|(b, _)| b));
        let _ = self.fs.remove(&self.part);
    }
}

/// What [`recover_into`] found: the log's lines, System status's reports
/// (Take recovered for each take it finished; Take damaged, about
/// [`Subject::found_take`], for each whose whole check failed), and the
/// name each found take's subject stands for (`found[n - 1]` for
/// `found_take(n)`: its file's name in Recordings).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recovered {
    pub lines: Vec<String>,
    pub reports: Vec<Report>,
    pub found: Vec<String>,
}

/// Takes found in a work folder at start: finished, checked, and moved
/// into `keep` (the library's Recordings place), each named as it was.
/// What was found is said in the musician's words, and to System status. A take whose
/// checks can't run yet stays for the next start; nothing is deleted.
pub fn recover_into(work: &Path, keep: &Path) -> io::Result<Recovered> {
    let mut fs = StdFs::new();
    let mut lines = Vec::new();
    let mut out = Recovered::default();
    let mut watch = Watch::new();
    let found = recover(&mut fs, work)?;
    // Each take whose whole check failed, and its found take's number.
    let mut doubtful: Vec<(PathBuf, u32)> = Vec::new();
    for f in &found {
        watch.found(f, &mut |r| out.reports.push(r));
        match f {
            Found::CutOff { stem, record, .. } => lines.push(format!(
                "audio tracks: {} was cut off while it recorded; recovered up to {}",
                show_stem(stem),
                minutes(record.frames, record.fmt.rate)
            )),
            Found::Finished { stem, .. } => lines.push(format!(
                "audio tracks: {} was cut off while it was being saved; finished",
                show_stem(stem)
            )),
            Found::Unverified { stem, .. } => lines.push(format!(
                "audio tracks: {}: audio that couldn't be checked was kept apart",
                show_stem(stem)
            )),
            Found::Failed { stem, why } => lines.push(format!(
                "audio tracks: {} couldn't be recovered yet ({why}); it is tried again next start",
                show_stem(stem)
            )),
            Found::Removed { .. } | Found::Newer { .. } | Found::Unchecked { .. } => {}
        }
        if let Some((path, record)) = f.to_check() {
            match check_whole(&mut fs, path, record) {
                Ok(true) => checked(&mut fs, work, f.stem())?,
                Ok(false) => {
                    out.found.push(show_stem(f.stem()));
                    let n = out.found.len() as u32;
                    watch.checked(Subject::found_take(n), false, &mut |r| out.reports.push(r));
                    doubtful.push((path.to_path_buf(), n));
                    checked(&mut fs, work, f.stem())?;
                }
                // Tried again next start, its checkpoint kept.
                Err(_) => {}
            }
        }
    }
    std::fs::create_dir_all(keep)?;
    // Takes, and stretches of audio that couldn't be checked.
    let (mut moved, mut apart, mut layers) = (0, 0, 0);
    let mut names: Vec<_> = std::fs::read_dir(work)?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    for n in &names {
        let take = n.ends_with(".wav") || n.ends_with(".raw");
        let stem = n.strip_suffix(".wav").unwrap_or(n);
        let pending = work.join(format!("{stem}.wav.checkpoint")).exists();
        let from = work.join(n);
        if !take || pending || !std::fs::symlink_metadata(&from).is_ok_and(|m| m.is_file()) {
            continue;
        }
        // A take whose whole check failed is kept as unverified.
        let failed = doubtful.iter().find(|(p, _)| *p == from).map(|&(_, n)| n);
        let base = if failed.is_some() {
            format!("{stem}.unverified.wav")
        } else {
            n.clone()
        };
        let to = free_name(keep, &base);
        std::fs::rename(&from, &to)?;
        if let (Some(k), Some(name)) = (failed, to.file_name()) {
            out.found[k as usize - 1] = name.to_string_lossy().into_owned();
        }
        if base.contains(".unverified.") {
            apart += 1;
        } else if n.starts_with("loop_") {
            // A loop's layer, written once it closed (phase 4c.3).
            layers += 1;
        } else {
            moved += 1;
        }
    }
    if moved + apart + layers > 0 {
        fs.sync_dir(keep)?;
        fs.sync_dir(work)?;
    }
    if moved > 0 {
        lines.push(format!(
            "audio tracks: {moved} take{} not saved in a project {} kept in Recordings",
            if moved == 1 { "" } else { "s" },
            if moved == 1 { "was" } else { "were" }
        ));
    }
    if layers > 0 {
        lines.push(format!(
            "audio tracks: {layers} loop layer{} not saved in a project {} kept in Recordings",
            if layers == 1 { "" } else { "s" },
            if layers == 1 { "was" } else { "were" }
        ));
    }
    if apart > 0 {
        lines.push(format!(
            "audio tracks: {apart} piece{} of audio that couldn't be checked {} kept in Recordings, marked unverified",
            if apart == 1 { "" } else { "s" },
            if apart == 1 { "was" } else { "were" }
        ));
    }
    out.lines = lines;
    Ok(out)
}

/// `name` in `dir`, or `name 2`, `name 3`... before its extension when
/// that is taken.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let (base, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (2..)
        .map(|k| dir.join(format!("{base} {k}.{ext}")))
        .find(|p| !p.exists())
        .expect("some number is free")
}

/// A stem `NAME_ID` as the log names it: the name, and the id's first 8
/// digits.
fn show_stem(stem: &str) -> String {
    match stem.rsplit_once('_') {
        Some((name, id)) if id.len() >= 8 => format!("a take of {name} ({})", &id[..8]),
        _ => format!("take {stem}"),
    }
}

fn minutes(frames: u64, rate: u32) -> String {
    let s = frames / u64::from(rate.max(1));
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_event_has_a_line_in_plain_words() {
        let at_risk = FileEvent::State {
            track: 1,
            state: TakeState::AtRisk(Why::Full),
        };
        assert_eq!(at_risk.track(), 1);
        assert_eq!(
            at_risk.line("Vocal"),
            "audio tracks: Vocal's take is at risk (the drive is full): if the power goes now, \
             what isn't on the drive yet is lost; it goes on recording into memory"
        );
        let failed = FileEvent::ArmFailed {
            track: 0,
            why: "No such file or directory (os error 2)".into(),
        };
        assert!(failed.line("Bass").starts_with(
            "audio tracks: Bass wasn't armed: its take's file couldn't be made (No such file"
        ));
        let memory = FileEvent::FromMemory {
            track: 2,
            state: None,
        };
        assert!(memory
            .line("Keys")
            .ends_with("as it has no file of its own"));
        let left = FileEvent::Unopened {
            track: 3,
            why: "No such file or directory (os error 2)".into(),
            kept: false,
        };
        assert_eq!(left.track(), 3);
        assert!(left.line("Gtr").starts_with(
            "audio tracks: a take of Gtr was left out, as its file couldn't be opened (No such"
        ));
    }

    #[test]
    fn a_track_name_becomes_a_file_name_without_allocating() {
        assert_eq!(Name::new("Vocal").as_str(), "Vocal");
        assert_eq!(Name::new(" .Bass: DI/amp? ").as_str(), "Bass- DI-amp-");
        assert_eq!(Name::new("...").as_str(), "Audio");
        assert_eq!(Name::new("").as_str(), "Audio");
        let long = "é".repeat(40);
        let n = Name::new(&long);
        assert_eq!(n.as_str().len(), NAME_MAX);
        assert!(n.as_str().chars().all(|c| c == 'é'));
        let rec = RecordingId(0x5b2f_0d9c_41e8_a776_3c0f_12ab_9e4d_8c01);
        assert_eq!(
            file_name("Vocal", rec),
            "Vocal_5b2f0d9c41e8a7763c0f12ab9e4d8c01.wav"
        );
        assert_eq!(rec_id(rec).hex(), rec.to_string());
        assert!(mpfree_library_name_ok(&file_name(
            Name::new(&long).as_str(),
            rec
        )));
    }

    /// A watch's report about an engine track's takes comes out naming
    /// the track, for the app to name as the musician does; one about the
    /// whole system names none.
    #[test]
    fn a_problem_names_the_engine_track_its_take_records_on() {
        use mpfree_status::kinds::{RECORDING_AT_RISK, STICK_MISSING};
        let p = Problem::of(Report::began(&RECORDING_AT_RISK).about(track_subject(3)));
        assert_eq!(p.track, Some(3));
        assert_eq!(p.report, Report::began(&RECORDING_AT_RISK));
        let p = Problem::of(Report::began(&STICK_MISSING));
        assert_eq!(p.track, None);
        assert_ne!(track_subject(0), Subject::NONE);
    }

    /// A take whose file was opened as it grew: a save reads what reached
    /// the file from there (the tracks fill in the rest from memory), and
    /// says so; a file that can't be read leaves that part silent, and
    /// says that.
    #[test]
    fn a_save_reads_what_reached_the_drive_from_the_file() {
        let dir = std::env::temp_dir().join(format!("mpfree-files-unit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let on_file: Vec<f32> = (0..300).map(|i| (i as f32 * 0.01).sin()).collect();
        let path = dir.join("Vocal_1.wav");
        std::fs::write(&path, crate::save::wav(&on_file, 1, 48_000)).unwrap();
        let rec = |n: u128, path: Option<PathBuf>| FileRec {
            rec: RecordingId(n),
            track: 0,
            id: Some(0),
            channels: 1,
            no: None,
            take: true,
            name: format!("Vocal_{n}.wav"),
            dir: None,
            path,
            frames: 300,
            shown: None,
            ready: true,
            saved_as: None,
            grown: 256,
            peaks: Vec::new(),
            pushed: 0,
            moving: false,
            hole: 0,
        };
        let mut f = Files::new(48_000);
        f.stream();
        f.recs.push(rec(1, Some(path)));
        f.recs.push(rec(2, Some(dir.join("gone.wav"))));
        let take = |n: u128, track: usize| TakeFile {
            id: 0,
            rec: RecordingId(n),
            track,
            group: 0,
            pass: 0,
            bpm: 0.0,
            varied: false,
            anchor: 0.0,
            frames: 500,
            channels: 1,
            file: String::new(),
            path: None,
            audio: Vec::new(),
        };
        let mut takes = [take(1, 0), take(2, 1)];
        let said = f.name_save(&mut takes);
        assert_eq!(takes[0].audio.len(), 500);
        assert_eq!(&takes[0].audio[..300], &on_file[..]);
        assert!(takes[0].audio[300..].iter().all(|&s| s == 0.0));
        assert_eq!(takes[0].path, None);
        assert_eq!(takes[0].file, "Vocal_1.wav");
        assert_eq!(said.len(), 2);
        assert!(matches!(said[0], FileEvent::FromBoth { track: 0, .. }));
        assert!(said[0]
            .line("Vocal")
            .starts_with("audio tracks: a take of Vocal was saved from what reached the drive"));
        assert!(matches!(said[1], FileEvent::Unread { track: 1, .. }));
        assert!(takes[1].audio.iter().all(|&s| s == 0.0));
        assert!(said[1]
            .line("Bass")
            .starts_with("audio tracks: the start of a take of Bass couldn't be read back"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The library's rule for a sample's name (mpfree-library's
    /// `valid_name`, 100 bytes): a take's file name always keeps it.
    fn mpfree_library_name_ok(s: &str) -> bool {
        s.len() <= 100
            && !s.starts_with(['.', ' '])
            && !s.ends_with(['.', ' '])
            && !s
                .chars()
                .any(|c| c.is_control() || "/\\<>:\"|?*".contains(c))
    }
}
