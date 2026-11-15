//! The screen's half of an export: [`start`] makes the folder, the files
//! and the first pass's writer thread, [`Job`] says how far it has got,
//! [`Job::next`] gives the next replay's pass when one comes back from
//! the sound thread, and [`Job::finish`] closes the export after the
//! last.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use mpfree_fx::{bounce, BounceOut, Pumped, StripId, WavFormat};
use mpfree_midi::smf::NoteLog;

use crate::mode::Mode;
use crate::pass::{Failure, How, Pass, Shared, Step, NOT_ENDED};
use crate::plan::{file_names, safe_name, MidiPlan, Plan, Replay};

/// Seconds of sound each file's ring holds: room for a stick that stalls
/// now and then (about 0.75 MB a file at 48 kHz).
pub const RING_S: f64 = 2.0;
/// How often the writer thread empties the rings.
pub const PUMP_EVERY: Duration = Duration::from_millis(20);

/// The writer's frames, where the sound ends, and whether each file
/// sounded; or why it stopped.
type Written = Result<(u64, u64, Vec<bool>), String>;

/// What [`Job::next`] says.
pub enum Next {
    /// The next replay's pass: send it to the sound thread.
    Play(Pass),
    /// No other pass: give this one to [`Job::finish`].
    Finish(Pass),
}

/// What a pass that played to its end gave.
struct Closed {
    /// Its files (indices into the job's).
    files: std::ops::Range<usize>,
    /// Their frames at the mixer's rate, where their sound ends, and the
    /// frame the range ended on ([`NOT_ENDED`] if it never did).
    frames: u64,
    loud: u64,
    ended: u64,
    /// Its tail was cut at the most with something still sounding: its
    /// files can't be made longer with silence.
    capped: bool,
}

impl Closed {
    /// Where its files should end: the range, and the tail as long as it
    /// sounds (never past what was written).
    fn keep(&self) -> u64 {
        if self.ended == NOT_ENDED {
            self.frames
        } else {
            self.ended.max(self.loud).min(self.frames)
        }
    }
}

/// The screen's half (see the module docs).
pub struct Job {
    /// The pass playing now: its flags, and its writer thread.
    shared: Arc<Shared>,
    writer: Option<JoinHandle<Written>>,
    folder: PathBuf,
    /// Whether [`start`] made the folder (so a cancel takes it away).
    made: bool,
    /// Every file: the first pass's, then one per replay.
    wavs: Vec<PathBuf>,
    /// Which files are stems (left out if they never sounded); the
    /// mixdown always stays.
    stems: Vec<bool>,
    /// Whether each file sounded, as its pass ends.
    heard: Vec<bool>,
    /// The first pass's files.
    first: usize,
    replays: Vec<Replay>,
    /// The replays' files, opened by [`start`], taken as each plays.
    later: Vec<Option<BufWriter<File>>>,
    /// The passes that played to their end.
    closed: Vec<Closed>,
    /// Why the export can't finish, once a pass's files failed.
    broken: Option<String>,
    /// The first pass's notes, once it is back.
    notes: Option<NoteLog>,
    song: String,
    midi: Option<MidiPlan>,
    rate: u32,
    format: WavFormat,
    length: Option<u64>,
    mode: Mode,
    first_slot: usize,
}

/// How far an export has got, for the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    /// Frames played.
    pub played: u64,
    /// Frames every file has.
    pub written: u64,
    /// The range's length, when known.
    pub length: Option<u64>,
    /// The range is over: the tail is playing.
    pub tail: bool,
    pub rate: u32,
    pub mode: Mode,
    /// The pass playing (from 0) and how many there are: the first, then
    /// a replay per stem that needs one.
    pub pass: usize,
    pub passes: usize,
}

impl Progress {
    /// "1:12 of 3:50", "1:12" with no known length, "3:52 · the tail";
    /// with replays, the pass too: "1:12 of 3:50 · pass 2 of 6".
    pub fn words(&self) -> String {
        let at = clock(self.played, self.rate);
        let now = match (self.tail, self.length) {
            (true, _) => format!("{at} · the tail"),
            (false, Some(n)) => format!("{at} of {}", clock(n, self.rate)),
            (false, None) => at,
        };
        if self.passes > 1 {
            format!("{now} · pass {} of {}", self.pass + 1, self.passes)
        } else {
            now
        }
    }

    /// The part of the whole export done, 0 to 1, for the panel's bar:
    /// each pass an equal share, its tail the end of its share. None when
    /// the length isn't known (until the tail).
    pub fn part(&self) -> Option<f32> {
        let this = if self.tail {
            1.0
        } else {
            let n = self.length.filter(|&n| n > 0)?;
            (self.played as f32 / n as f32).min(1.0)
        };
        Some((self.pass as f32 + this) / self.passes.max(1) as f32)
    }
}

/// `frames` as minutes and seconds, "3:07".
pub fn clock(frames: u64, rate: u32) -> String {
    let s = frames / u64::from(rate.max(1));
    format!("{}:{:02}", s / 60, s % 60)
}

/// A finished export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Done {
    pub folder: PathBuf,
    pub wavs: Vec<PathBuf>,
    /// Stems left out because their track never sounded (their names).
    pub silent: Vec<String>,
    /// None when no notes played (or no MIDI file was asked for).
    pub midi: Option<PathBuf>,
    /// Each WAV file's frames, at the mixer's rate (`rate`).
    pub frames: u64,
    pub rate: u32,
    /// The WAV files' format.
    pub format: WavFormat,
    /// Notes past the MIDI file's room, left out.
    pub notes_dropped: u64,
}

/// How an export ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Done(Done),
    /// Cancelled: its files are gone.
    Cancelled,
    /// It stopped (why): its files are gone.
    Failed(String),
}

impl Outcome {
    /// A line for the panel and the log.
    pub fn words(&self) -> String {
        match self {
            Outcome::Done(d) => {
                let name = d
                    .folder
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let wavs = match d.wavs.len() {
                    0 => String::new(),
                    1 => format!("1 WAV file ({})", d.format.words()),
                    n => format!("{n} WAV files ({})", d.format.words()),
                };
                let what = match (wavs.is_empty(), d.midi.is_some()) {
                    (false, true) => format!("{wavs} and a MIDI file"),
                    (true, true) => "a MIDI file".into(),
                    (false, false) => wavs,
                    (true, false) => "nothing: no notes played".into(),
                };
                let mut line = format!("Exported {} to {name}: {what}", clock(d.frames, d.rate));
                match d.silent.len() {
                    0 => {}
                    1 => line += &format!(" ({} was silent, so left out)", d.silent[0]),
                    n => line += &format!(" ({n} silent tracks left out)"),
                }
                if d.notes_dropped > 0 {
                    line += &format!(
                        " ({} notes past the MIDI file's room were left out)",
                        d.notes_dropped
                    );
                }
                line
            }
            Outcome::Cancelled => "Export cancelled, and its files removed".into(),
            Outcome::Failed(why) => format!("Export stopped: {why}. Its files were removed."),
        }
    }
}

/// Starts an export: makes the folder and the files, starts the writer
/// thread, and gives back the pass for the sound thread and the job for
/// the screen. Off the sound thread (it allocates and opens files).
pub fn start(plan: Plan) -> io::Result<(Pass, Job)> {
    if plan.streams.is_empty() && plan.replays.is_empty() && plan.midi.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "nothing to export: no files picked",
        ));
    }
    let made = match fs::read_dir(&plan.folder) {
        Ok(mut d) => {
            if d.next().is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} isn't empty", plan.folder.display()),
                ));
            }
            false
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(&plan.folder)?;
            true
        }
        Err(e) => return Err(e),
    };
    let all = plan.all_streams();
    let wavs: Vec<PathBuf> = file_names(&all)
        .iter()
        .map(|n| plan.folder.join(n))
        .collect();
    let mut files = Vec::with_capacity(wavs.len());
    for p in &wavs {
        match File::create(p) {
            Ok(f) => files.push(BufWriter::with_capacity(1 << 16, f)),
            Err(e) => {
                remove(&plan.folder, made, &wavs, None);
                return Err(e);
            }
        }
    }
    let first = plan.streams.len();
    let later = files.split_off(first).into_iter().map(Some).collect();
    let shared = Arc::new(Shared::new());
    let (bin, writer) = match writer(first, files, plan.rate, plan.format, &shared) {
        Ok(w) => w,
        Err(e) => {
            remove(&plan.folder, made, &wavs, None);
            return Err(e);
        }
    };
    let notes = plan.midi.as_ref().map(|m| NoteLog::with_capacity(m.room));
    let stems = all.iter().map(|s| s.strip != StripId::Master).collect();
    let points = plan.streams.iter().map(|s| (s.strip, s.point)).collect();
    let pass = Pass::new(
        bin,
        points,
        notes,
        shared.clone(),
        How {
            first_slot: plan.first_slot,
            mode: plan.mode,
            rate: plan.rate,
            only: None,
            index: 0,
            passes: plan.passes(),
        },
    );
    let job = Job {
        shared,
        writer: Some(writer),
        folder: plan.folder,
        made,
        heard: vec![false; wavs.len()],
        wavs,
        stems,
        first,
        replays: plan.replays,
        later,
        closed: Vec::new(),
        broken: None,
        notes: None,
        song: plan.song,
        midi: plan.midi,
        rate: plan.rate,
        format: plan.format,
        length: plan.length,
        mode: plan.mode,
        first_slot: plan.first_slot,
    };
    Ok((pass, job))
}

/// A pass's bounce for `streams` streams into `files`, and its writer
/// thread.
fn writer(
    streams: usize,
    files: Vec<BufWriter<File>>,
    rate: u32,
    format: WavFormat,
    shared: &Arc<Shared>,
) -> io::Result<(mpfree_fx::BounceIn, JoinHandle<Written>)> {
    let ring = (RING_S * f64::from(rate)) as usize;
    let (bin, bout) = bounce(streams, ring, rate);
    let bout = bout.with_format(format);
    let theirs = shared.clone();
    let w = std::thread::Builder::new()
        .name("mpfree-export".into())
        .spawn(move || write(bout, files, &theirs))?;
    Ok((bin, w))
}

/// The writer thread: empties the rings into the files until the pass is
/// over, then makes sure they are on the stick. Its frames and where the
/// sound ends, or why it stopped.
fn write(mut bout: BounceOut, mut files: Vec<BufWriter<File>>, shared: &Shared) -> Written {
    loop {
        if shared.cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        match bout.pump(&mut files) {
            Ok(Pumped::Working(n)) => {
                shared.written.store(n, Ordering::Relaxed);
                std::thread::sleep(PUMP_EVERY);
            }
            Ok(Pumped::Done(n)) => {
                shared.written.store(n, Ordering::Relaxed);
                for f in &mut files {
                    f.flush().map_err(|e| format!("writing the files: {e}"))?;
                    f.get_ref()
                        .sync_all()
                        .map_err(|e| format!("writing the files: {e}"))?;
                }
                return Ok((n, bout.loud(), bout.heard().to_vec()));
            }
            Ok(Pumped::Overrun(k)) => {
                return Err(format!(
                    "the stick couldn't keep up, and {k} blocks of sound were lost"
                ))
            }
            Err(e) => return Err(format!("writing the files: {e}")),
        }
    }
}

/// Takes away what an export made.
fn remove(folder: &Path, made: bool, wavs: &[PathBuf], midi: Option<&Path>) {
    for p in wavs.iter().map(PathBuf::as_path).chain(midi) {
        let _ = fs::remove_file(p);
    }
    if made {
        let _ = fs::remove_dir(folder);
    }
}

/// Makes a file `frames` long (at the file's rate), cutting it or adding
/// silence (zero bytes are silence in every format), and writes its
/// header again.
fn trim(p: &Path, format: WavFormat, frames: u64) -> io::Result<()> {
    let mut f = OpenOptions::new().write(true).open(p)?;
    f.set_len(format.header_len() as u64 + frames * format.bits.frame_bytes())?;
    f.seek(SeekFrom::Start(0))?;
    f.write_all(&format.header(frames))?;
    f.sync_all()
}

impl Job {
    /// How far it has got.
    pub fn progress(&self) -> Progress {
        let ended = self.shared.ended_at.load(Ordering::Relaxed);
        Progress {
            played: self.shared.played.load(Ordering::Relaxed),
            written: self.shared.written.load(Ordering::Relaxed),
            length: self.length,
            tail: ended != NOT_ENDED,
            rate: self.rate,
            mode: self.mode,
            pass: self.closed.len(),
            passes: self.passes(),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The passes over the range: the first, and one per replay.
    pub fn passes(&self) -> usize {
        1 + self.replays.len()
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Stops the export: the pass playing says Cancelled at its next
    /// period, and [`Job::finish`] removes every file.
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }

    /// Waits for the pass's writer and keeps what its files gave (and the
    /// first pass's notes). Err: why its files failed.
    fn close(&mut self, pass: &mut Pass) -> Result<(), String> {
        let written = match self.writer.take().map(JoinHandle::join) {
            Some(Ok(w)) => w,
            Some(Err(_)) => Err("the writer thread stopped".into()),
            None => Err("no writer thread".into()),
        };
        if pass.index() == 0 {
            self.notes = pass.notes.take();
        }
        let (frames, loud, heard) = written?;
        let at = match pass.index() {
            0 => 0,
            k => self.first + k - 1,
        };
        let files = at..at + heard.len();
        for (i, h) in files.clone().zip(heard) {
            self.heard[i] = h;
        }
        self.closed.push(Closed {
            files,
            frames,
            loud,
            ended: self.shared.ended_at.load(Ordering::Relaxed),
            capped: self.shared.capped.load(Ordering::Relaxed),
        });
        Ok(())
    }

    /// A pass is back from the sound thread (over, and detached): the
    /// next replay's pass, for the sound thread to play as it played the
    /// first; or the pass back when there is none (it was the last, or it
    /// didn't play to its end, or its files failed), for [`Job::finish`].
    pub fn next(&mut self, mut pass: Pass) -> Next {
        let k = pass.index();
        if pass.over != Some(Step::Done) || k >= self.replays.len() || k != self.closed.len() {
            return Next::Finish(pass);
        }
        if let Err(why) = self.close(&mut pass) {
            self.broken = Some(why);
            return Next::Finish(pass);
        }
        let Some(file) = self.later.get_mut(k).and_then(Option::take) else {
            self.broken = Some("a replay's file went missing".into());
            return Next::Finish(pass);
        };
        let shared = Arc::new(Shared::new());
        let (bin, writer) = match writer(1, vec![file], self.rate, self.format, &shared) {
            Ok(w) => w,
            Err(e) => {
                self.broken = Some(format!("starting the next pass: {e}"));
                return Next::Finish(pass);
            }
        };
        self.shared = shared.clone();
        self.writer = Some(writer);
        let r = &self.replays[k];
        Next::Play(Pass::new(
            bin,
            vec![(r.stream.strip, r.stream.point)],
            None,
            shared,
            How {
                first_slot: self.first_slot,
                mode: self.mode,
                rate: self.rate,
                only: Some(r.only),
                index: k + 1,
                passes: self.passes(),
            },
        ))
    }

    /// Closes the export once its last pass is back from the sound thread
    /// (its [`Pass::period`] said it is over, it was detached, and
    /// [`Job::next`] gave no other): waits for the writer, makes every
    /// file as long as the longest pass's sound (cutting the tail's
    /// silence, never into the range, and adding silence to a shorter
    /// replay), writes the MIDI file. A pass that isn't over is
    /// cancelled; a pass that failed, or a replay not played, ends it with
    /// no files.
    pub fn finish(mut self, mut pass: Pass) -> Outcome {
        let over = match pass.over {
            Some(s) => s,
            None => {
                self.cancel();
                pass.bin.finish();
                Step::Cancelled
            }
        };
        let closed = if self.writer.is_some() {
            self.close(&mut pass)
        } else {
            Ok(())
        };
        let (folder, made) = (self.folder.clone(), self.made);
        let failed = |wavs: &[PathBuf], why: String| {
            remove(&folder, made, wavs, None);
            Outcome::Failed(why)
        };
        match over {
            Step::Done => {}
            // A pass is never over on Go; taken as a cancel all the same.
            Step::Cancelled | Step::Go => {
                remove(&self.folder, self.made, &self.wavs, None);
                return Outcome::Cancelled;
            }
            Step::Failed(Failure::Overrun(k)) => {
                return failed(
                    &self.wavs,
                    closed.err().unwrap_or_else(|| {
                        format!("the stick couldn't keep up, and {k} blocks of sound were lost")
                    }),
                )
            }
            Step::Failed(Failure::Capture) => {
                return failed(
                    &self.wavs,
                    "the mixer had no free capture point for every file".into(),
                )
            }
            Step::Failed(Failure::Latency) => {
                return failed(
                    &self.wavs,
                    "an effect that delays the sound (look-ahead) was put in or taken out, \
                     so the files would no longer line up"
                        .into(),
                )
            }
        }
        if let Some(why) = self.broken.take().or(closed.err()) {
            return failed(&self.wavs, why);
        }
        if self.closed.len() < self.passes() {
            // The last pass came back before every replay had played.
            remove(&self.folder, self.made, &self.wavs, None);
            return Outcome::Cancelled;
        }
        // A stem whose track never sounded is left out.
        let mut silent = Vec::new();
        let mut kept = vec![true; self.wavs.len()];
        for (i, p) in self.wavs.iter().enumerate() {
            if self.stems[i] && !self.heard[i] {
                let _ = fs::remove_file(p);
                kept[i] = false;
                silent.push(
                    p.file_stem()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
            }
        }
        // Every file as long as the longest pass's sound: a pass whose
        // sound died earlier gets silence added, but never one cut off
        // still sounding (each pass's latency differs, so a tail cut at
        // the most ends a few frames apart in each).
        let with_files = || self.closed.iter().filter(|c| !c.files.is_empty());
        let longest = with_files().map(Closed::keep).max().unwrap_or(0);
        let keep = with_files()
            .filter(|c| c.capped)
            .map(|c| c.frames)
            .fold(longest, u64::min);
        for c in &self.closed {
            if c.frames == keep {
                continue;
            }
            let at = self.format.frames_for(keep, self.rate);
            for i in c.files.clone().filter(|&i| kept[i]) {
                if let Err(e) = trim(&self.wavs[i], self.format, at) {
                    return failed(&self.wavs, format!("ending the files: {e}"));
                }
            }
        }
        let wavs: Vec<PathBuf> = (0..self.wavs.len())
            .filter(|&i| kept[i])
            .map(|i| self.wavs[i].clone())
            .collect();
        let mut midi = None;
        let mut notes_dropped = 0;
        if let (Some(m), Some(notes)) = (&self.midi, &self.notes) {
            notes_dropped = notes.dropped;
            if !notes.is_empty() {
                let smf = notes.smf(&self.song, &m.tracks, &m.tempos, &m.meters, m.end);
                let p = self.folder.join(format!("{}.mid", safe_name(&self.song)));
                let wrote = File::create(&p).and_then(|mut f| {
                    f.write_all(&smf.bytes())?;
                    f.sync_all()
                });
                if let Err(e) = wrote {
                    remove(&self.folder, self.made, &wavs, Some(&p));
                    return Outcome::Failed(format!("writing the MIDI file: {e}"));
                }
                midi = Some(p);
            }
        }
        Outcome::Done(Done {
            folder: self.folder.clone(),
            wavs,
            silent,
            midi,
            frames: keep,
            rate: self.rate,
            format: self.format,
            notes_dropped,
        })
    }
}

impl Drop for Job {
    /// A job dropped without [`Job::finish`] stops its writer thread (the
    /// files stay as they are).
    fn drop(&mut self) {
        if self.writer.is_some() {
            self.cancel();
        }
    }
}
