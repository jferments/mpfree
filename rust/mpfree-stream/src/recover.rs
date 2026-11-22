//! Finishing takes after a power cut or a pulled stick, at the next start,
//! before a project opens (the recording file protocol's recovery).
//!
//! [`recover`] looks in a media folder for `.recording` and `.checkpoint`
//! files, pairs them by the recording id in their names and, for each
//! take:
//!
//! - tries its valid checkpoint records newest first; one **checks out**
//!   when the file holds its committed audio and the fast check (the last
//!   64 KiB) matches;
//! - keeps any audio past that record apart as `NAME_ID.tail.unverified.wav`
//!   before cutting the take to length, so the cut never destroys it;
//! - finishes the take by itself (header, length, sync, rename, folder
//!   sync); its checkpoint stays until the whole-take check has run
//!   ([`WholeCheck`], then [`checked`]);
//! - keeps audio it can't prove as `NAME_ID.unverified.wav` (or `.raw`
//!   when no format can be read), never removing it;
//! - removes files that hold no audio, and leaves alone anything from a
//!   newer MPFree or not named as a take.
//!
//! The steps run in a fixed order and each gives the same result if done
//! twice, so a cut during recovery just repeats it next time. Recovery
//! never needs the WAV header: the format and length come from the record.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use crate::crc32c;
use crate::fsio::Fs;
use crate::rec::{
    checkpoint_path, final_path, header, header_block, header_format, id_of_stem, recording_path,
    tail_crc, tail_path, unverified_path, HeaderBlock, RecFormat, RecId, Record, Sizes, Slot,
    DATA_AT, SLOT, TAIL_CHECK,
};

/// Bytes read at a time when copying or checking.
pub const PIECE: usize = 512 * 1024;

/// What recovery found and did for one take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// Its stop was cut off; finished from its final record.
    Finished {
        stem: String,
        path: PathBuf,
        record: Record,
        tail: Option<PathBuf>,
    },
    /// Cut off while recording; finished up to the newest record that
    /// checks out. Ask only whether to put it back on its lane.
    CutOff {
        stem: String,
        path: PathBuf,
        record: Record,
        tail: Option<PathBuf>,
    },
    /// Finished before (by its stop or by recovery) with its checkpoint
    /// still there: run the whole-take check, then [`checked`].
    Unchecked {
        stem: String,
        path: PathBuf,
        record: Record,
    },
    /// Audio that can't be proved, kept apart and never removed by itself.
    Unverified { stem: String, path: PathBuf },
    /// Nothing to keep: no audio, so its files went (logged).
    Removed { stem: String },
    /// Recorded by a newer MPFree: left alone.
    Newer { stem: String },
    /// A step failed; nothing was lost, and it is tried again next start.
    Failed { stem: String, why: String },
}

impl Found {
    pub fn stem(&self) -> &str {
        match self {
            Found::Finished { stem, .. }
            | Found::CutOff { stem, .. }
            | Found::Unchecked { stem, .. }
            | Found::Unverified { stem, .. }
            | Found::Removed { stem }
            | Found::Newer { stem }
            | Found::Failed { stem, .. } => stem,
        }
    }

    /// The take's file and record, when it was finished and its whole-take
    /// check is still to run.
    pub fn to_check(&self) -> Option<(&Path, &Record)> {
        match self {
            Found::Finished { path, record, .. }
            | Found::CutOff { path, record, .. }
            | Found::Unchecked { path, record, .. } => Some((path, record)),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Files {
    recording: bool,
    checkpoint: bool,
}

/// Recover every take in `dir`, in name order.
pub fn recover(fs: &mut dyn Fs, dir: &Path) -> io::Result<Vec<Found>> {
    let mut takes: BTreeMap<String, Files> = BTreeMap::new();
    for n in fs.list(dir)? {
        if let Some(s) = n.strip_suffix(".wav.recording") {
            if id_of_stem(s).is_some() {
                takes.entry(s.to_string()).or_default().recording = true;
            }
        } else if let Some(s) = n.strip_suffix(".wav.checkpoint") {
            if id_of_stem(s).is_some() {
                takes.entry(s.to_string()).or_default().checkpoint = true;
            }
        }
    }
    let mut out = Vec::with_capacity(takes.len());
    for (stem, files) in takes {
        let id = id_of_stem(&stem).expect("checked above");
        let found = one(fs, dir, &stem, id, &files).unwrap_or_else(|e| Found::Failed {
            stem: stem.clone(),
            why: e.to_string(),
        });
        out.push(found);
    }
    Ok(out)
}

fn read_up_to(fs: &mut dyn Fs, path: &Path, at: u64, len: usize) -> io::Result<Vec<u8>> {
    let mut b = vec![0u8; len];
    let got = fs.read_at(path, at, &mut b)?;
    b.truncate(got);
    Ok(b)
}

/// Whether record `r` checks out against a file `len` bytes long.
fn checks_out(fs: &mut dyn Fs, path: &Path, len: u64, r: &Record) -> io::Result<bool> {
    let end = DATA_AT + r.bytes();
    if len < end {
        return Ok(false);
    }
    let k = r.bytes().min(TAIL_CHECK);
    let tail = read_up_to(fs, path, end - k, k as usize)?;
    Ok(tail.len() as u64 == k && tail_crc(&tail) == r.tail_crc)
}

fn one(fs: &mut dyn Fs, dir: &Path, stem: &str, id: RecId, files: &Files) -> io::Result<Found> {
    let rec = recording_path(dir, stem);
    let ckpt = checkpoint_path(dir, stem);
    let fin = final_path(dir, stem);
    let mut records = Vec::new();
    if files.checkpoint {
        let b = read_up_to(fs, &ckpt, 0, 2 * SLOT)?;
        for s in 0..2 {
            let slot = b.get(s * SLOT..(s + 1) * SLOT).unwrap_or(&[]);
            match Record::decode(slot, &id) {
                Slot::Valid(r) => records.push(r),
                Slot::Newer => return Ok(Found::Newer { stem: stem.into() }),
                Slot::Invalid => {}
            }
        }
    }
    records.sort_by_key(|r| std::cmp::Reverse(r.seq));

    if !files.recording {
        if fs.exists(&fin) {
            // Renamed already (P3): only the checkpoint is left.
            let len = fs.len(&fin)?;
            for r in &records {
                if checks_out(fs, &fin, len, r)? {
                    return Ok(Found::Unchecked {
                        stem: stem.into(),
                        path: fin,
                        record: *r,
                    });
                }
            }
        }
        // A checkpoint with no audio file it can vouch for.
        fs.remove(&ckpt)?;
        return Ok(Found::Removed { stem: stem.into() });
    }

    let head = read_up_to(fs, &rec, 0, DATA_AT as usize)?;
    let block = header_block(&head);
    if block == HeaderBlock::Newer {
        return Ok(Found::Newer { stem: stem.into() });
    }
    let len = fs.len(&rec)?;
    let mixed_up = matches!(block, HeaderBlock::Clean(other) if other != id);
    if !mixed_up {
        for r in &records {
            if checks_out(fs, &rec, len, r)? {
                if r.frames == 0 && !r.last {
                    break; // nothing committed: see below
                }
                return finish(fs, dir, stem, &rec, len, r);
            }
        }
    }
    if len <= DATA_AT {
        // No audio in it.
        fs.remove(&rec)?;
        if files.checkpoint {
            fs.remove(&ckpt)?;
        }
        return Ok(Found::Removed { stem: stem.into() });
    }
    let fmt = records
        .first()
        .map(|r| r.fmt)
        .or_else(|| header_format(&head));
    unverified(
        fs,
        dir,
        stem,
        id,
        &rec,
        files.checkpoint.then_some(&ckpt),
        len,
        fmt,
    )
}

/// Finish a take at record `r`'s length: the tail apart, then header,
/// length, sync, rename, folder sync. The checkpoint stays until the
/// whole-take check has run.
fn finish(
    fs: &mut dyn Fs,
    dir: &Path,
    stem: &str,
    rec: &Path,
    len: u64,
    r: &Record,
) -> io::Result<Found> {
    let id = r.id;
    let fmt = r.fmt;
    let end = DATA_AT + r.bytes();
    let pad = r.bytes() & 1;
    let tail_from = end + if r.last { pad } else { 0 };
    let align = fmt.block_align() as u64;
    let tail_frames = len.saturating_sub(tail_from) / align;
    let mut tail = None;
    if tail_frames > 0 {
        let t = tail_path(dir, stem);
        fs.create(&t, false)?;
        fs.write_at(&t, 0, &header(fmt, &id, Sizes::done(fmt, tail_frames)))?;
        copy(fs, rec, tail_from, &t, DATA_AT, tail_frames * align)?;
        if (tail_frames * align) & 1 == 1 {
            fs.write_at(&t, DATA_AT + tail_frames * align, &[0])?;
        }
        fs.sync(&t)?;
        fs.sync_dir(dir)?;
        tail = Some(t);
    }
    fs.write_at(rec, 0, &header(fmt, &id, Sizes::done(fmt, r.frames)))?;
    fs.set_len(rec, end + pad)?;
    if pad == 1 {
        fs.write_at(rec, end, &[0])?;
    }
    fs.sync(rec)?;
    let fin = final_path(dir, stem);
    fs.rename(rec, &fin)?;
    fs.sync_dir(dir)?;
    Ok(if r.last {
        Found::Finished {
            stem: stem.into(),
            path: fin,
            record: *r,
            tail,
        }
    } else {
        Found::CutOff {
            stem: stem.into(),
            path: fin,
            record: *r,
            tail,
        }
    })
}

fn copy(
    fs: &mut dyn Fs,
    from: &Path,
    from_at: u64,
    to: &Path,
    to_at: u64,
    bytes: u64,
) -> io::Result<()> {
    let mut done = 0u64;
    let mut buf = vec![0u8; PIECE];
    while done < bytes {
        let n = (bytes - done).min(PIECE as u64) as usize;
        let got = fs.read_at(from, from_at + done, &mut buf[..n])?;
        if got != n {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the take ended while copying its tail",
            ));
        }
        fs.write_at(to, to_at + done, &buf[..n])?;
        done += n as u64;
    }
    Ok(())
}

/// Keep a take whose audio can't be proved: header rebuilt from the
/// file's length in whole frames (when a format is known), renamed apart,
/// never removed by itself.
#[allow(clippy::too_many_arguments)]
fn unverified(
    fs: &mut dyn Fs,
    dir: &Path,
    stem: &str,
    id: RecId,
    rec: &Path,
    ckpt: Option<&PathBuf>,
    len: u64,
    fmt: Option<RecFormat>,
) -> io::Result<Found> {
    let path = unverified_path(dir, stem, fmt.is_none());
    if let Some(fmt) = fmt {
        let align = fmt.block_align() as u64;
        let frames = (len - DATA_AT) / align;
        let sizes = Sizes::Done {
            data: frames * align,
            frames,
            end: len,
        };
        fs.write_at(rec, 0, &header(fmt, &id, sizes))?;
        fs.sync(rec)?;
    }
    fs.rename(rec, &path)?;
    fs.sync_dir(dir)?;
    if let Some(c) = ckpt {
        fs.remove(c)?;
    }
    Ok(Found::Unverified {
        stem: stem.into(),
        path,
    })
}

/// The whole-take check, a piece at a time (a background read behind
/// playback): every committed byte against the record's running checksum.
pub struct WholeCheck {
    path: PathBuf,
    bytes: u64,
    want: u32,
    done: u64,
    crc: u32,
    buf: Vec<u8>,
}

impl WholeCheck {
    pub fn new(path: &Path, r: &Record) -> Self {
        WholeCheck {
            path: path.to_path_buf(),
            bytes: r.bytes(),
            want: r.whole_crc,
            done: 0,
            crc: 0,
            buf: vec![0u8; PIECE],
        }
    }

    /// Read the next piece: `Some(matched)` once the whole take is read.
    pub fn step(&mut self, fs: &mut dyn Fs) -> io::Result<Option<bool>> {
        if self.done < self.bytes {
            let n = (self.bytes - self.done).min(PIECE as u64) as usize;
            let got = fs.read_at(&self.path, DATA_AT + self.done, &mut self.buf[..n])?;
            if got != n {
                return Ok(Some(false));
            }
            self.crc = crc32c::update(self.crc, &self.buf[..n]);
            self.done += n as u64;
        }
        Ok((self.done >= self.bytes).then_some(self.crc == self.want))
    }

    /// How far it has got, 0 to 1.
    pub fn progress(&self) -> f64 {
        if self.bytes == 0 {
            1.0
        } else {
            self.done as f64 / self.bytes as f64
        }
    }
}

/// Run the whole-take check to the end.
pub fn check_whole(fs: &mut dyn Fs, path: &Path, r: &Record) -> io::Result<bool> {
    let mut c = WholeCheck::new(path, r);
    loop {
        if let Some(ok) = c.step(fs)? {
            return Ok(ok);
        }
    }
}

/// The whole-take check has run (its answer is the caller's to keep):
/// the take's checkpoint goes.
pub fn checked(fs: &mut dyn Fs, dir: &Path, stem: &str) -> io::Result<()> {
    match fs.remove(&checkpoint_path(dir, stem)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}
