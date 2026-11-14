//! The screen's half of an export: [`start`] makes the folder, the files
//! and the writer thread, [`Job`] says how far it has got, and
//! [`Job::finish`] closes it once the pass comes back from the sound
//! thread.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use mpfree_fx::{bounce, wav_header, BounceOut, Pumped, StripId};
use mpfree_midi::smf::NoteLog;

use crate::mode::Mode;
use crate::pass::{Failure, Pass, Shared, Step, NOT_ENDED};
use crate::plan::{file_names, safe_name, MidiPlan, Plan};

/// Seconds of sound each file's ring holds: room for a stick that stalls
/// now and then (about 0.75 MB a file at 48 kHz).
pub const RING_S: f64 = 2.0;
/// How often the writer thread empties the rings.
pub const PUMP_EVERY: Duration = Duration::from_millis(20);

/// The writer's frames, where the sound ends, and whether each file
/// sounded; or why it stopped.
type Written = Result<(u64, u64, Vec<bool>), String>;

/// The screen's half (see the module docs).
pub struct Job {
    shared: Arc<Shared>,
    writer: Option<JoinHandle<Written>>,
    folder: PathBuf,
    /// Whether [`start`] made the folder (so a cancel takes it away).
    made: bool,
    wavs: Vec<PathBuf>,
    /// Which files are stems (left out if they never sounded); the
    /// mixdown always stays.
    stems: Vec<bool>,
    song: String,
    midi: Option<MidiPlan>,
    rate: u32,
    length: Option<u64>,
    mode: Mode,
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
}

impl Progress {
    /// "1:12 of 3:50", "1:12" with no known length, "3:52 · the tail".
    pub fn words(&self) -> String {
        let at = clock(self.played, self.rate);
        match (self.tail, self.length) {
            (true, _) => format!("{at} · the tail"),
            (false, Some(n)) => format!("{at} of {}", clock(n, self.rate)),
            (false, None) => at,
        }
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
    /// Each WAV file's frames.
    pub frames: u64,
    pub rate: u32,
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
                    1 => "1 WAV file".into(),
                    n => format!("{n} WAV files"),
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
    if plan.streams.is_empty() && plan.midi.is_none() {
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
    let wavs: Vec<PathBuf> = file_names(&plan.streams)
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
    let ring = (RING_S * f64::from(plan.rate)) as usize;
    let (bin, bout) = bounce(plan.streams.len(), ring, plan.rate);
    let shared = Arc::new(Shared::new());
    let theirs = shared.clone();
    let writer = std::thread::Builder::new()
        .name("mpfree-export".into())
        .spawn(move || write(bout, files, &theirs));
    let writer = match writer {
        Ok(w) => w,
        Err(e) => {
            remove(&plan.folder, made, &wavs, None);
            return Err(e);
        }
    };
    let notes = plan.midi.as_ref().map(|m| NoteLog::with_capacity(m.room));
    let stems = plan
        .streams
        .iter()
        .map(|s| s.strip != StripId::Master)
        .collect();
    let points = plan.streams.iter().map(|s| (s.strip, s.point)).collect();
    let pass = Pass::new(
        bin,
        points,
        plan.first_slot,
        notes,
        shared.clone(),
        plan.mode,
        plan.rate,
    );
    let job = Job {
        shared,
        writer: Some(writer),
        folder: plan.folder,
        made,
        wavs,
        stems,
        song: plan.song,
        midi: plan.midi,
        rate: plan.rate,
        length: plan.length,
        mode: plan.mode,
    };
    Ok((pass, job))
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

/// Cuts each file to `frames` and writes its header again.
fn trim(wavs: &[PathBuf], rate: u32, frames: u64) -> io::Result<()> {
    for p in wavs {
        let mut f = OpenOptions::new().write(true).open(p)?;
        f.set_len(mpfree_fx::bounce::WAV_HEADER as u64 + frames * 6)?;
        f.seek(SeekFrom::Start(0))?;
        f.write_all(&wav_header(rate, frames))?;
        f.sync_all()?;
    }
    Ok(())
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
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Stops the export: the pass says Cancelled at its next period, and
    /// [`Job::finish`] removes the files.
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }

    /// Closes the export once the pass is back from the sound thread (its
    /// [`Pass::period`] said it is over, and it was detached): waits for
    /// the writer, cuts the tail's silence, writes the MIDI file. A pass
    /// that isn't over is cancelled.
    pub fn finish(mut self, mut pass: Pass) -> Outcome {
        let over = match pass.over {
            Some(s) => s,
            None => {
                self.cancel();
                pass.bin.finish();
                Step::Cancelled
            }
        };
        let written = match self.writer.take().map(JoinHandle::join) {
            Some(Ok(w)) => w,
            Some(Err(_)) => Err("the writer thread stopped".into()),
            None => Err("no writer thread".into()),
        };
        let (folder, made) = (self.folder.clone(), self.made);
        let failed = |wavs: &[PathBuf], why: String| {
            remove(&folder, made, wavs, None);
            Outcome::Failed(why)
        };
        match (over, written) {
            (Step::Done, Ok((frames, loud, heard))) => {
                // A stem whose track never sounded is left out.
                let mut wavs = Vec::with_capacity(self.wavs.len());
                let mut silent = Vec::new();
                for (i, p) in self.wavs.iter().enumerate() {
                    if self.stems[i] && !heard.get(i).copied().unwrap_or(true) {
                        let _ = fs::remove_file(p);
                        silent.push(
                            p.file_stem()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        );
                    } else {
                        wavs.push(p.clone());
                    }
                }
                let ended = self.shared.ended_at.load(Ordering::Relaxed);
                let keep = if ended == NOT_ENDED {
                    frames
                } else {
                    ended.max(loud).min(frames)
                };
                if keep < frames {
                    if let Err(e) = trim(&wavs, self.rate, keep) {
                        return failed(&wavs, format!("ending the files: {e}"));
                    }
                }
                let mut midi = None;
                let mut notes_dropped = 0;
                if let (Some(m), Some(notes)) = (&self.midi, &pass.notes) {
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
                    notes_dropped,
                })
            }
            (Step::Done, Err(why)) => failed(&self.wavs, why),
            // A pass is never over on Go; taken as a cancel all the same.
            (Step::Cancelled | Step::Go, _) => {
                remove(&self.folder, self.made, &self.wavs, None);
                Outcome::Cancelled
            }
            (Step::Failed(Failure::Overrun(_)), Err(why)) => failed(&self.wavs, why),
            (Step::Failed(Failure::Overrun(k)), Ok(_)) => failed(
                &self.wavs,
                format!("the stick couldn't keep up, and {k} blocks of sound were lost"),
            ),
            (Step::Failed(Failure::Capture), _) => failed(
                &self.wavs,
                "the mixer had no free capture point for every file".into(),
            ),
            (Step::Failed(Failure::Latency), _) => failed(
                &self.wavs,
                "an effect that delays the sound (look-ahead) was put in or taken out, \
                 so the files would no longer line up"
                    .into(),
            ),
        }
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
