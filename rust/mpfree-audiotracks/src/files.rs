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

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_stream::fsio::{Fs, StdFs};
use mpfree_stream::rec::{Meta, RecFormat, RecId};
pub use mpfree_stream::rec::{TakeState, Why};
use mpfree_stream::recover::{check_whole, checked, recover, Found};
pub use mpfree_stream::writer::WriterConfig;
use mpfree_stream::writer::{Gate, RealClock, Recorder, Status, TakeNo, TakeView};

use crate::save::TakeFile;
use crate::RecordingId;

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
                let file = match state {
                    Some(TakeState::AtRisk(w) | TakeState::Stopped(w)) => {
                        format!("its file stopped short ({})", why_words(*w))
                    }
                    _ => "it has no file of its own".to_string(),
                };
                format!("audio tracks: a take of {who} was saved from memory, as {file}")
            }
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
}

/// How often the worker looks at the writer's takes.
const POLL: Duration = Duration::from_millis(50);

/// One recording's file, on the worker.
struct FileRec {
    rec: RecordingId,
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
}

/// The writer and the folder it records into.
struct Writing {
    dir: PathBuf,
    rec: Recorder,
}

/// A take's finished file to open as its stream source: the take, its
/// lasting id, the file, and the channels and frames it must hold.
pub(crate) type ToOpen = (u16, RecordingId, PathBuf, usize, usize);

/// The worker's side of the files: see the module notes.
pub(crate) struct Files {
    rate: u32,
    writing: Option<Writing>,
    recs: Vec<FileRec>,
    last_poll: Instant,
    /// Takes play from their files (phase 3b): each take's file is to be
    /// opened once finished ([`Files::take_opens`]).
    streams: bool,
    opens: Vec<ToOpen>,
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
        }
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

    /// Where recording `rec`'s finished file is, if it has one.
    pub fn path_of(&self, rec: RecordingId) -> Option<PathBuf> {
        self.recs.iter().find(|f| f.rec == rec)?.path.clone()
    }

    /// Whether the writer runs (the worker then wakes to look at it).
    pub fn writing(&self) -> bool {
        self.writing.is_some()
    }

    /// Record into `dir` from now on (made if missing), the writer's
    /// rounds waiting for playback's reads through `gate` (the stream
    /// coordinator's, while takes play from their files).
    pub fn start(&mut self, dir: PathBuf, cfg: WriterConfig, gate: Arc<Gate>) -> io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        if let Some(old) = self.writing.take() {
            old.rec.close();
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
            id: None,
            channels,
            no: Some(no),
            take: false,
            name: file_name(name.as_str(), rec),
            path: None,
            frames: 0,
            shown: None,
            ready: false,
            saved_as: None,
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
            id: Some(id),
            channels: 0,
            no: None,
            take: true,
            name,
            path,
            frames,
            shown: Some(FileView::SAVED),
            ready: true,
            saved_as: None,
        });
    }

    /// The tracks were emptied: every take's file is let go of (the files
    /// stay where they are). Files still only armed are left for their
    /// [`Files::disarm`].
    pub fn clear(&mut self) {
        let Files { writing, recs, .. } = self;
        recs.retain(|f| {
            if !f.take {
                return true;
            }
            if let (Some(w), Some(no)) = (writing.as_mut(), f.no) {
                w.rec.forget(no);
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
        for f in &mut self.recs {
            let Some(no) = f.no else { continue };
            match w.rec.status(no) {
                None | Some(Status::Arming) => {}
                Some(Status::Failed(why)) => {
                    out.push(Said::Failed { rec: f.rec, why });
                    w.rec.disarm(no);
                    f.no = None;
                    if !f.take {
                        gone.push(f.rec);
                    }
                }
                Some(Status::Take(v)) => {
                    if !f.take {
                        if !f.ready {
                            f.ready = true;
                            out.push(Said::Ready(f.rec));
                        }
                        continue;
                    }
                    let view = FileView::of(&v);
                    if v.state == TakeState::Saved {
                        f.path = w.rec.final_path(no);
                        // The writer is done with it.
                        w.rec.forget(no);
                        f.no = None;
                        if let (true, Some(id), Some(p)) = (self.streams, f.id, &f.path) {
                            let ch = usize::from(f.channels);
                            self.opens.push((id, f.rec, p.clone(), ch, f.frames));
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
                    if f.channels > 0 {
                        said.push(FileEvent::FromMemory {
                            track: k.track,
                            state: f.shown.map(|s| s.state),
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
        let work = self.writing.as_ref().map(|w| w.dir.clone());
        for f in &mut self.recs {
            let Some(name) = f.saved_as.as_ref() else {
                continue;
            };
            let there = dir.join(name);
            if !std::fs::symlink_metadata(&there).is_ok_and(|m| m.is_file()) {
                continue;
            }
            if let (Some(old), Some(work)) = (&f.path, &work) {
                if old.parent() == Some(work.as_path()) && *old != there && f.no.is_none() {
                    let _ = std::fs::remove_file(old);
                }
            }
            f.name.clone_from(name);
            f.path = Some(there);
        }
    }
}

/// Takes found in a work folder at start: finished, checked, and moved
/// into `keep` (the library's Recordings place), each named as it was.
/// The lines say what was found, in the musician's words. A take whose
/// checks can't run yet stays for the next start; nothing is deleted.
pub fn recover_into(work: &Path, keep: &Path) -> io::Result<Vec<String>> {
    let mut fs = StdFs::new();
    let mut lines = Vec::new();
    let found = recover(&mut fs, work)?;
    let mut doubtful = Vec::new();
    for f in &found {
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
                    doubtful.push(path.to_path_buf());
                    checked(&mut fs, work, f.stem())?;
                }
                // Tried again next start, its checkpoint kept.
                Err(_) => {}
            }
        }
    }
    std::fs::create_dir_all(keep)?;
    // Takes, and stretches of audio that couldn't be checked.
    let (mut moved, mut apart) = (0, 0);
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
        let base = if doubtful.contains(&from) {
            format!("{stem}.unverified.wav")
        } else {
            n.clone()
        };
        let to = free_name(keep, &base);
        std::fs::rename(&from, &to)?;
        if base.contains(".unverified.") {
            apart += 1;
        } else {
            moved += 1;
        }
    }
    if moved + apart > 0 {
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
    if apart > 0 {
        lines.push(format!(
            "audio tracks: {apart} piece{} of audio that couldn't be checked {} kept in Recordings, marked unverified",
            if apart == 1 { "" } else { "s" },
            if apart == 1 { "was" } else { "were" }
        ));
    }
    Ok(lines)
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
