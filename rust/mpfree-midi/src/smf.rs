//! Standard MIDI Files: a song's notes for another sequencer or a DAW,
//! as the export's MIDI file writes them.
//!
//! - [`NoteLog`] takes the notes the player sends during the export's pass,
//!   at their ticks, on the sound thread: a fixed capacity set before the
//!   pass, no allocation, the overflow counted.
//! - [`Smf`] is a type 1 file: a conductor track with the song's name, its
//!   tempo map and its meter, then a track per MPFree track that played,
//!   named after it. [`Smf::bytes`] writes it at [`PPQ`] ticks a quarter,
//!   the transport's own, so no tick moves.
//!
//! At one tick, note-offs come before note-ons, so a note played again on
//! the tick its last one ends isn't cut short. A note still on at the end
//! gets its note-off there. Every event has its status byte (no running
//! status): a few bytes more, read by everything.
//!
//! [`read`] is the other way, for MIDI file import: a type 0, 1 or 2 file
//! as another sequencer, a DAW or the MPC's own MIDI export writes it,
//! made an [`Smf`] at [`PPQ`] ticks a quarter whatever the file's timing
//! ([`Timing`]), with what it left out counted ([`Reading`]). It reads
//! only, from bytes the caller has (no files opened), is bounded
//! ([`MAX_FILE_BYTES`], [`MAX_FILE_TRACKS`], [`MAX_FILE_EVENTS`]) and
//! never panics on what it is given: a file it can't read is an
//! [`SmfError`] that says why.

use crate::msg::Msg;

/// Ticks a quarter note: the transport's.
pub const PPQ: u16 = 960;

/// One MPFree track's events, in tick order or not ([`Smf::bytes`]
/// sorts them).
#[derive(Clone, Debug, PartialEq)]
pub struct SmfTrack {
    pub name: String,
    pub events: Vec<(u64, Msg)>,
}

/// A whole file (see the module docs).
#[derive(Clone, Debug, PartialEq)]
pub struct Smf {
    /// The song's name, on the conductor track.
    pub name: String,
    /// Each tempo and the tick it starts at; the first should be at 0
    /// (120 BPM is assumed before it).
    pub tempos: Vec<(u64, f64)>,
    /// Each meter (beats, beat value: 4 for a quarter) and the tick it
    /// starts at.
    pub meters: Vec<(u64, u8, u8)>,
    pub tracks: Vec<SmfTrack>,
    /// Where the song ends: every track's end, and where notes still on
    /// are let go. Later events move it on.
    pub end: u64,
}

/// A variable-length quantity, as delta times and lengths are written.
fn vlq(out: &mut Vec<u8>, v: u64) {
    let v = v.min(0x0FFF_FFFF);
    let mut bytes = [0u8; 4];
    let mut n = 0;
    let mut rest = v;
    loop {
        bytes[n] = (rest & 0x7F) as u8;
        n += 1;
        rest >>= 7;
        if rest == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        out.push(bytes[i] | if i > 0 { 0x80 } else { 0 });
    }
}

fn meta(out: &mut Vec<u8>, delta: u64, kind: u8, data: &[u8]) {
    vlq(out, delta);
    out.extend_from_slice(&[0xFF, kind]);
    vlq(out, data.len() as u64);
    out.extend_from_slice(data);
}

fn chunk(out: &mut Vec<u8>, body: &[u8]) {
    out.extend_from_slice(b"MTrk");
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body);
}

/// Sorts first at a tick: note-offs, then the rest, then note-ons.
fn rank(m: &Msg) -> u8 {
    match m {
        Msg::NoteOff { .. } => 0,
        Msg::NoteOn { .. } => 2,
        _ => 1,
    }
}

impl Smf {
    /// The end of the song or of its last event, whichever is later.
    fn last(&self) -> u64 {
        let events = self
            .tracks
            .iter()
            .flat_map(|t| t.events.iter().map(|e| e.0));
        let marks = self
            .tempos
            .iter()
            .map(|t| t.0)
            .chain(self.meters.iter().map(|m| m.0));
        events.chain(marks).fold(self.end, u64::max)
    }

    fn conductor(&self, end: u64) -> Vec<u8> {
        let mut out = Vec::new();
        meta(&mut out, 0, 0x03, self.name.as_bytes());
        let mut marks: Vec<(u64, u8, [u8; 4], usize)> = Vec::new();
        for &(t, num, den) in &self.meters {
            let den = den.max(1).next_power_of_two();
            let pow = den.trailing_zeros() as u8;
            // 24 MIDI clocks a click, eight 32nds a quarter.
            marks.push((t, 0x58, [num.max(1), pow, 24, 8], 4));
        }
        for &(t, bpm) in &self.tempos {
            let us = (60_000_000.0 / bpm.clamp(1.0, 1000.0)).round() as u32;
            let b = us.min(0xFF_FFFF).to_be_bytes();
            marks.push((t, 0x51, [b[1], b[2], b[3], 0], 3));
        }
        // Meter before tempo at one tick, as most files have them.
        marks.sort_by_key(|m| (m.0, std::cmp::Reverse(m.1)));
        let mut at = 0;
        for (t, kind, data, n) in marks {
            meta(&mut out, t - at, kind, &data[..n]);
            at = t;
        }
        meta(&mut out, end - at, 0x2F, &[]);
        out
    }

    fn track(t: &SmfTrack, end: u64) -> Vec<u8> {
        let mut out = Vec::new();
        meta(&mut out, 0, 0x03, t.name.as_bytes());
        let mut events = t.events.clone();
        events.sort_by_key(|(tick, m)| (*tick, rank(m)));
        // Notes on, per channel and note, to let go at the end.
        let mut on = vec![0u16; 16 * 128];
        let mut at = 0;
        for (tick, m) in &events {
            match *m {
                Msg::NoteOn { ch, note, .. } => {
                    on[usize::from(ch & 15) * 128 + usize::from(note & 127)] += 1
                }
                Msg::NoteOff { ch, note, .. } => {
                    let n = &mut on[usize::from(ch & 15) * 128 + usize::from(note & 127)];
                    *n = n.saturating_sub(1);
                }
                _ => {}
            }
            let (b, n) = m.encode();
            vlq(&mut out, tick - at);
            out.extend_from_slice(&b[..n]);
            at = *tick;
        }
        for (i, &n) in on.iter().enumerate() {
            for _ in 0..n {
                let off = Msg::NoteOff {
                    ch: (i / 128) as u8,
                    note: (i % 128) as u8,
                    vel: 64,
                };
                let (b, len) = off.encode();
                vlq(&mut out, end - at);
                out.extend_from_slice(&b[..len]);
                at = end;
            }
        }
        meta(&mut out, end - at, 0x2F, &[]);
        out
    }

    /// The file's bytes.
    pub fn bytes(&self) -> Vec<u8> {
        let end = self.last();
        let mut out = Vec::new();
        out.extend_from_slice(b"MThd");
        out.extend_from_slice(&6u32.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        let n = (self.tracks.len() + 1).min(u16::MAX as usize) as u16;
        out.extend_from_slice(&n.to_be_bytes());
        out.extend_from_slice(&PPQ.to_be_bytes());
        chunk(&mut out, &self.conductor(end));
        for t in self.tracks.iter().take(usize::from(n) - 1) {
            chunk(&mut out, &Smf::track(t, end));
        }
        out
    }
}

/// The notes the player sends during a pass (see the module docs).
#[derive(Clone, Debug)]
pub struct NoteLog {
    events: Vec<(u64, u16, Msg)>,
    /// Events past the capacity, not kept.
    pub dropped: u64,
}

impl NoteLog {
    /// Room for `n` events (about 16 bytes each), made before the pass.
    pub fn with_capacity(n: usize) -> NoteLog {
        NoteLog {
            events: Vec::with_capacity(n),
            dropped: 0,
        }
    }

    /// `msg` on `track` at `tick`. Never allocates: past the capacity it
    /// is counted in [`NoteLog::dropped`].
    pub fn push(&mut self, tick: u64, track: u16, msg: Msg) {
        if self.events.len() < self.events.capacity() {
            self.events.push((tick, track, msg));
        } else {
            self.dropped += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Empties it for another pass, keeping its room.
    pub fn clear(&mut self) {
        self.events.clear();
        self.dropped = 0;
    }

    /// The file: a track for each MPFree track that played, in track
    /// order, named from `names` (by track index; "Track N" past them).
    pub fn smf(
        &self,
        song: &str,
        names: &[String],
        tempos: &[(u64, f64)],
        meters: &[(u64, u8, u8)],
        end: u64,
    ) -> Smf {
        let mut used: Vec<u16> = self.events.iter().map(|e| e.1).collect();
        used.sort_unstable();
        used.dedup();
        let tracks = used
            .iter()
            .map(|&t| SmfTrack {
                name: names
                    .get(usize::from(t))
                    .cloned()
                    .unwrap_or_else(|| format!("Track {}", t + 1)),
                events: self
                    .events
                    .iter()
                    .filter(|e| e.1 == t)
                    .map(|e| (e.0, e.2))
                    .collect(),
            })
            .collect();
        Smf {
            name: song.to_string(),
            tempos: tempos.to_vec(),
            meters: meters.to_vec(),
            tracks,
            end,
        }
    }
}

/// The biggest file [`read`] reads: 4 MiB, far more than any song's notes.
pub const MAX_FILE_BYTES: usize = 4 << 20;
/// Track chunks [`read`] reads; any more are counted
/// ([`Reading::tracks_left`]).
pub const MAX_FILE_TRACKS: usize = 256;
/// Channel events [`read`] keeps from a file, all its tracks together.
pub const MAX_FILE_EVENTS: usize = 1 << 20;
/// The tempo a file with frame timing ([`Timing::Frames`]) is read at, as
/// the format leaves it none.
pub const FRAMES_BPM: f64 = 120.0;

/// A file's timing, from its header.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Timing {
    /// Ticks a quarter note (most files: 96, 480, 960).
    Quarter(u16),
    /// Frames a second (24, 25, 29.97 or 30) and ticks a frame: clock
    /// time, read at [`FRAMES_BPM`].
    Frames { fps: f64, ticks: u8 },
}

/// Why a file couldn't be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SmfError {
    /// It doesn't start with `MThd`.
    NotMidi,
    /// Past [`MAX_FILE_BYTES`].
    TooBig(usize),
    /// A header shorter than the standard's 6 bytes, a format past 2, or
    /// no ticks.
    BadHeader,
    /// Past [`MAX_FILE_EVENTS`].
    TooManyEvents,
    /// Something the format doesn't allow, in track `track` (from 1) at
    /// byte `at` of the file.
    Bad {
        track: usize,
        at: usize,
        what: &'static str,
    },
    /// No track chunk at all.
    NoTracks,
}

impl std::fmt::Display for SmfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SmfError::NotMidi => write!(f, "not a MIDI file (it doesn't start with MThd)"),
            SmfError::TooBig(n) => write!(
                f,
                "too big for a MIDI file: {n} bytes, and {} MiB is the most read",
                MAX_FILE_BYTES >> 20
            ),
            SmfError::BadHeader => write!(f, "its header isn't one the standard allows"),
            SmfError::TooManyEvents => write!(
                f,
                "more than {MAX_FILE_EVENTS} notes and other messages: too many to read"
            ),
            SmfError::Bad { track, at, what } => {
                write!(f, "track {track}, byte {at}: {what}")
            }
            SmfError::NoTracks => write!(f, "it has no tracks"),
        }
    }
}

/// What [`read`] found: the file as an [`Smf`] at [`PPQ`], and what it
/// left out.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub smf: Smf,
    /// The header's format: 0 (one track, every channel), 1 (tracks played
    /// together) or 2 (tracks that are patterns of their own).
    pub format: u16,
    pub timing: Timing,
    /// SysEx messages skipped.
    pub sysex: usize,
    /// Metas skipped: lyrics, markers, key signatures and the like (track
    /// names, tempos, meters and ends are read).
    pub metas: usize,
    /// Track chunks past [`MAX_FILE_TRACKS`], not read.
    pub tracks_left: usize,
    /// The file ends inside a track (cut short when it was copied, say):
    /// that track is read up to there.
    pub cut: bool,
}

/// Bytes of one track chunk, with where they are in the file for errors.
struct Body<'a> {
    b: &'a [u8],
    i: usize,
    /// The chunk's first byte in the file.
    base: usize,
    track: usize,
}

impl Body<'_> {
    /// At the last byte read: the one that is wrong.
    fn bad(&self, what: &'static str) -> SmfError {
        SmfError::Bad {
            track: self.track,
            at: self.base + self.i.saturating_sub(1),
            what,
        }
    }

    fn byte(&mut self) -> Option<u8> {
        let c = *self.b.get(self.i)?;
        self.i += 1;
        Some(c)
    }

    /// A variable-length quantity: at most 4 bytes, as the standard says.
    /// None at the end of the bytes.
    fn vlq(&mut self) -> Result<Option<u64>, SmfError> {
        let mut v = 0u64;
        for _ in 0..4 {
            let Some(c) = self.byte() else {
                return Ok(None);
            };
            v = (v << 7) | u64::from(c & 0x7F);
            if c & 0x80 == 0 {
                return Ok(Some(v));
            }
        }
        Err(self.bad("a number longer than the format's 4 bytes"))
    }

    /// `n` bytes, or None if the chunk ends first.
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let end = self.i.checked_add(n)?;
        let s = self.b.get(self.i..end)?;
        self.i = end;
        Some(s)
    }
}

/// Ticks of the file to [`PPQ`] ticks: exact for whole multiples, else
/// rounded to the nearest.
fn to_ppq(tick: u64, timing: Timing) -> u64 {
    match timing {
        Timing::Quarter(q) => {
            let q = u128::from(q.max(1));
            ((u128::from(tick) * u128::from(PPQ) + q / 2) / q) as u64
        }
        Timing::Frames { fps, ticks } => {
            let secs = tick as f64 / (fps * f64::from(ticks.max(1)));
            (secs * FRAMES_BPM / 60.0 * f64::from(PPQ)).round() as u64
        }
    }
}

/// One track chunk's events, at the file's ticks, onto `out`.
#[derive(Default)]
struct Track {
    name: Option<String>,
    events: Vec<(u64, Msg)>,
    tempos: Vec<(u64, f64)>,
    meters: Vec<(u64, u8, u8)>,
    end: u64,
}

fn text(data: &[u8]) -> String {
    String::from_utf8_lossy(data).trim().to_string()
}

fn read_track(
    body: &mut Body,
    count: &mut usize,
    sysex: &mut usize,
    metas: &mut usize,
) -> Result<(Track, bool), SmfError> {
    let mut t = Track::default();
    let mut tick = 0u64;
    let mut running: Option<u8> = None;
    loop {
        let Some(delta) = body.vlq()? else {
            // The bytes end without an end of track: lenient, as most
            // readers are, but said if it is in the middle of an event.
            return Ok((t, false));
        };
        tick += delta;
        t.end = t.end.max(tick);
        let Some(first) = body.byte() else {
            return Ok((t, true));
        };
        match first {
            0xFF => {
                let Some(kind) = body.byte() else {
                    return Ok((t, true));
                };
                let Some(len) = body.vlq()? else {
                    return Ok((t, true));
                };
                let Some(data) = body.take(len as usize) else {
                    return Ok((t, true));
                };
                match (kind, data.len()) {
                    (0x2F, _) => return Ok((t, false)),
                    (0x03, _) if t.name.is_none() => t.name = Some(text(data)),
                    (0x51, 3) => {
                        let us = u32::from_be_bytes([0, data[0], data[1], data[2]]);
                        if us > 0 {
                            t.tempos.push((tick, 60_000_000.0 / f64::from(us)));
                        }
                    }
                    (0x58, 4..) => {
                        let den = 1u16 << data[1].min(6);
                        if data[0] > 0 {
                            t.meters.push((tick, data[0], den as u8));
                        }
                    }
                    _ => *metas += 1,
                }
            }
            0xF0 | 0xF7 => {
                let Some(len) = body.vlq()? else {
                    return Ok((t, true));
                };
                if body.take(len as usize).is_none() {
                    return Ok((t, true));
                }
                *sysex += 1;
            }
            0xF1..=0xFE => return Err(body.bad("a system message, which a file can't hold")),
            _ => {
                let (status, d1) = if first & 0x80 != 0 {
                    running = Some(first);
                    let Some(d1) = body.byte() else {
                        return Ok((t, true));
                    };
                    (first, d1)
                } else {
                    let Some(s) = running else {
                        return Err(body.bad("a data byte with no message before it"));
                    };
                    (s, first)
                };
                let d2 = match status & 0xF0 {
                    0xC0 | 0xD0 => 0,
                    _ => {
                        let Some(d2) = body.byte() else {
                            return Ok((t, true));
                        };
                        d2
                    }
                };
                if d1 & 0x80 != 0 || d2 & 0x80 != 0 {
                    return Err(body.bad("a message cut short by another"));
                }
                if *count >= MAX_FILE_EVENTS {
                    return Err(SmfError::TooManyEvents);
                }
                if let Some(m) = Msg::from_bytes(status, d1, d2) {
                    t.events.push((tick, m));
                    *count += 1;
                }
            }
        }
    }
}

/// A Standard MIDI File's bytes as an [`Smf`] at [`PPQ`] (see the module
/// docs). The song's name is the first track's name in a type 1 file whose
/// first track plays nothing (a conductor track, as most have), or the
/// only track's in a type 0; each track is named by its first name, or
/// none. Tempos and meters from every track are the song's, in order; a
/// file with none plays at 120 BPM in 4/4, as the standard says. The end is
/// the last end of track or event, whichever is later. Notes and other
/// channel messages keep their order within a tick.
pub fn read(bytes: &[u8]) -> Result<Reading, SmfError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(SmfError::TooBig(bytes.len()));
    }
    if bytes.get(0..4) != Some(&b"MThd"[..]) {
        return Err(SmfError::NotMidi);
    }
    let be16 = |at: usize| {
        bytes
            .get(at..at + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
    };
    let be32 = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let head = be32(4).ok_or(SmfError::BadHeader)?;
    let (Some(format), Some(_), Some(div)) = (be16(8), be16(10), be16(12)) else {
        return Err(SmfError::BadHeader);
    };
    if head < 6 || format > 2 || div == 0 {
        return Err(SmfError::BadHeader);
    }
    let timing = if div & 0x8000 == 0 {
        Timing::Quarter(div)
    } else {
        let fps = match (div >> 8) as u8 as i8 {
            -24 => 24.0,
            -25 => 25.0,
            -29 => 30_000.0 / 1001.0,
            -30 => 30.0,
            _ => return Err(SmfError::BadHeader),
        };
        let ticks = (div & 0xFF) as u8;
        if ticks == 0 {
            return Err(SmfError::BadHeader);
        }
        Timing::Frames { fps, ticks }
    };
    let mut at = 8usize.checked_add(head).ok_or(SmfError::BadHeader)?;
    let (mut count, mut sysex, mut metas) = (0usize, 0usize, 0usize);
    let mut tracks: Vec<Track> = Vec::new();
    let (mut left, mut cut) = (0usize, false);
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = be32(at + 4).unwrap_or(0);
        let start = at + 8;
        let end = start.saturating_add(len);
        at = end;
        if id != b"MTrk" {
            // A chunk of another kind: the standard says to skip it.
            continue;
        }
        if tracks.len() >= MAX_FILE_TRACKS {
            left += 1;
            continue;
        }
        let short = end > bytes.len();
        let mut body = Body {
            b: &bytes[start..end.min(bytes.len())],
            i: 0,
            base: start,
            track: tracks.len() + 1,
        };
        let (t, mid) = read_track(&mut body, &mut count, &mut sysex, &mut metas)?;
        cut |= short || mid;
        tracks.push(t);
    }
    if tracks.is_empty() {
        return Err(SmfError::NoTracks);
    }
    let ppq = |t: u64| to_ppq(t, timing);
    let mut tempos: Vec<(u64, f64)> = Vec::new();
    let mut meters: Vec<(u64, u8, u8)> = Vec::new();
    for t in &tracks {
        tempos.extend(t.tempos.iter().map(|&(k, b)| (ppq(k), b)));
        meters.extend(t.meters.iter().map(|&(k, n, d)| (ppq(k), n, d)));
    }
    if let Timing::Frames { .. } = timing {
        // Clock time has one tempo: the one it was read at.
        tempos.clear();
        tempos.push((0, FRAMES_BPM));
    }
    tempos.sort_by_key(|t| t.0);
    meters.sort_by_key(|m| m.0);
    let end = tracks.iter().map(|t| ppq(t.end)).max().unwrap_or(0);
    // A type 1 file's first track names the song when it plays nothing.
    let conductor = format == 1 && tracks.len() > 1 && tracks[0].events.is_empty();
    let name = if format == 0 || conductor {
        tracks[0].name.clone().unwrap_or_default()
    } else {
        String::new()
    };
    let skip = usize::from(conductor);
    let tracks = tracks
        .into_iter()
        .skip(skip)
        .map(|t| SmfTrack {
            name: if format == 0 {
                String::new()
            } else {
                t.name.unwrap_or_default()
            },
            events: t.events.into_iter().map(|(k, m)| (ppq(k), m)).collect(),
        })
        .collect();
    Ok(Reading {
        smf: Smf {
            name,
            tempos,
            meters,
            tracks,
            end,
        },
        format,
        timing,
        sysex,
        metas,
        tracks_left: left,
        cut,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One track as read back: (absolute tick, bytes) per event.
    type Events = Vec<(u64, Vec<u8>)>;

    /// A reader written apart from the writer, for the round trip: each
    /// track's events as (absolute tick, bytes), metas included.
    fn read(b: &[u8]) -> (u16, u16, Vec<Events>) {
        assert_eq!(&b[0..4], b"MThd");
        assert_eq!(u32::from_be_bytes(b[4..8].try_into().unwrap()), 6);
        let format = u16::from_be_bytes([b[8], b[9]]);
        let n = u16::from_be_bytes([b[10], b[11]]);
        let div = u16::from_be_bytes([b[12], b[13]]);
        assert_eq!(format, 1);
        let mut at = 14;
        let mut tracks = Vec::new();
        for _ in 0..n {
            assert_eq!(&b[at..at + 4], b"MTrk");
            let len = u32::from_be_bytes(b[at + 4..at + 8].try_into().unwrap()) as usize;
            let body = &b[at + 8..at + 8 + len];
            at += 8 + len;
            let mut i = 0;
            let mut tick = 0u64;
            let mut events = Vec::new();
            let num = |i: &mut usize| {
                let mut v = 0u64;
                loop {
                    let c = body[*i];
                    *i += 1;
                    v = (v << 7) | u64::from(c & 0x7F);
                    if c & 0x80 == 0 {
                        return v;
                    }
                }
            };
            while i < body.len() {
                tick += num(&mut i);
                let start = i;
                let status = body[i];
                assert!(status & 0x80 != 0, "no running status in our files");
                if status == 0xFF {
                    i += 2;
                    let l = num(&mut i) as usize;
                    i += l;
                } else {
                    i += match status & 0xF0 {
                        0xC0 | 0xD0 => 2,
                        _ => 3,
                    };
                }
                events.push((tick, body[start..i].to_vec()));
            }
            assert_eq!(
                events.last().map(|e| e.1.clone()),
                Some(vec![0xFF, 0x2F, 0])
            );
            tracks.push(events);
        }
        assert_eq!(at, b.len(), "nothing after the last track");
        (n, div, tracks)
    }

    fn on(ch: u8, note: u8, vel: u8) -> Msg {
        Msg::NoteOn { ch, note, vel }
    }

    fn off(ch: u8, note: u8) -> Msg {
        Msg::NoteOff { ch, note, vel: 64 }
    }

    #[test]
    fn a_small_song_is_byte_for_byte_what_the_standard_says() {
        let smf = Smf {
            name: "Hi".into(),
            tempos: vec![(0, 120.0)],
            meters: vec![(0, 4, 4)],
            tracks: vec![SmfTrack {
                name: "Bass".into(),
                events: vec![(0, on(0, 36, 100)), (480, off(0, 36))],
            }],
            end: 3840,
        };
        let want: Vec<u8> = [
            &b"MThd"[..],
            &[0, 0, 0, 6, 0, 1, 0, 2, 0x03, 0xC0],
            b"MTrk",
            &[0, 0, 0, 26],
            &[0x00, 0xFF, 0x03, 2, b'H', b'i'],
            &[0x00, 0xFF, 0x58, 4, 4, 2, 24, 8],
            // 500,000 microseconds a quarter.
            &[0x00, 0xFF, 0x51, 3, 0x07, 0xA1, 0x20],
            // 3840 ticks: 0x9E 0x00.
            &[0x9E, 0x00, 0xFF, 0x2F, 0],
            b"MTrk",
            &[0, 0, 0, 22],
            &[0x00, 0xFF, 0x03, 4, b'B', b'a', b's', b's'],
            &[0x00, 0x90, 36, 100],
            &[0x83, 0x60, 0x80, 36, 64],
            // 3360 ticks: 0x9A 0x20.
            &[0x9A, 0x20, 0xFF, 0x2F, 0],
        ]
        .concat();
        assert_eq!(smf.bytes(), want);
    }

    #[test]
    fn a_logged_pass_round_trips_with_tempo_changes_and_held_notes() {
        let mut log = NoteLog::with_capacity(64);
        // Drums on track 0 (channel 10), keys on track 2; track 1 silent.
        log.push(0, 0, on(9, 36, 110));
        log.push(240, 0, off(9, 36));
        log.push(960, 2, on(0, 60, 90));
        // The same note again on the tick the first ends: off first.
        log.push(1920, 2, on(0, 60, 80));
        log.push(1920, 2, off(0, 60));
        log.push(1000, 2, Msg::PitchBend { ch: 0, value: 512 });
        // Left on: let go at the end.
        log.push(3000, 2, on(0, 64, 70));
        let names = vec!["Drums".to_string(), "Bass".to_string(), "Keys".to_string()];
        let smf = log.smf(
            "Night drive",
            &names,
            &[(0, 92.0), (3840, 140.0)],
            &[(0, 7, 8)],
            7680,
        );
        let (n, div, tracks) = read(&smf.bytes());
        assert_eq!((n, div), (3, 960));
        let name = |t: &[(u64, Vec<u8>)]| String::from_utf8(t[0].1[3..].to_vec()).unwrap();
        assert_eq!(name(&tracks[0]), "Night drive");
        assert_eq!(name(&tracks[1]), "Drums");
        assert_eq!(name(&tracks[2]), "Keys");
        // Meter 7/8, then 92 and 140 BPM.
        assert_eq!(tracks[0][1], (0, vec![0xFF, 0x58, 4, 7, 3, 24, 8]));
        let us92 = (60_000_000.0f64 / 92.0).round() as u32;
        assert_eq!(tracks[0][2].1[3..], us92.to_be_bytes()[1..]);
        assert_eq!(tracks[0][3].0, 3840);
        assert_eq!(tracks[0][4], (7680, vec![0xFF, 0x2F, 0]));
        let keys: Vec<(u64, Vec<u8>)> = tracks[2][1..].to_vec();
        assert_eq!(
            keys,
            [
                (960, vec![0x90, 60, 90]),
                (1000, vec![0xE0, 0x00, 0x44]),
                (1920, vec![0x80, 60, 64]),
                (1920, vec![0x90, 60, 80]),
                (3000, vec![0x90, 64, 70]),
                (7680, vec![0x80, 60, 64]),
                (7680, vec![0x80, 64, 64]),
                (7680, vec![0xFF, 0x2F, 0]),
            ]
        );
        assert_eq!(tracks[1][1], (0, vec![0x99, 36, 110]));
    }

    #[test]
    fn the_log_never_grows_and_counts_what_it_drops() {
        let mut log = NoteLog::with_capacity(2);
        let cap = log.events.capacity();
        for i in 0..cap as u64 + 3 {
            log.push(i, 0, on(0, 60, 1));
        }
        assert_eq!(log.len(), cap);
        assert_eq!(log.dropped, 3);
        assert_eq!(log.events.capacity(), cap);
        log.clear();
        assert!(log.is_empty() && log.dropped == 0);
    }

    #[test]
    fn long_gaps_and_names_are_written_in_full() {
        let mut b = Vec::new();
        for (v, want) in [
            (0u64, vec![0x00]),
            (0x7F, vec![0x7F]),
            (0x80, vec![0x81, 0x00]),
            (0x3FFF, vec![0xFF, 0x7F]),
            (0x20_0000, vec![0x81, 0x80, 0x80, 0x00]),
            (0x0FFF_FFFF, vec![0xFF, 0xFF, 0xFF, 0x7F]),
        ] {
            b.clear();
            vlq(&mut b, v);
            assert_eq!(b, want, "{v:#x}");
        }
        let long = "é".repeat(100);
        let smf = Smf {
            name: long.clone(),
            tempos: vec![(0, 120.0)],
            meters: vec![],
            tracks: vec![],
            end: 0,
        };
        let (_, _, tracks) = read(&smf.bytes());
        assert_eq!(tracks[0][0].1[2..4], [0x81, 0x48], "200 bytes");
        assert_eq!(
            String::from_utf8(tracks[0][0].1[4..].to_vec()).unwrap(),
            long
        );
    }

    /// A file from its header fields and track bodies.
    fn file(format: u16, div: u16, tracks: &[&[u8]]) -> Vec<u8> {
        let mut b = b"MThd".to_vec();
        b.extend_from_slice(&6u32.to_be_bytes());
        b.extend_from_slice(&format.to_be_bytes());
        b.extend_from_slice(&(tracks.len() as u16).to_be_bytes());
        b.extend_from_slice(&div.to_be_bytes());
        for t in tracks {
            chunk(&mut b, t);
        }
        b
    }

    #[test]
    fn what_the_export_writes_reads_back_the_same() {
        let smf = Smf {
            name: "Night drive".into(),
            tempos: vec![(0, 92.0), (3840, 140.0)],
            meters: vec![(0, 7, 8)],
            tracks: vec![
                SmfTrack {
                    name: "Drums".into(),
                    events: vec![(0, on(9, 36, 110)), (240, off(9, 36))],
                },
                SmfTrack {
                    name: "Keys".into(),
                    events: vec![
                        (960, on(0, 60, 90)),
                        (1000, Msg::PitchBend { ch: 0, value: 512 }),
                        (1920, off(0, 60)),
                    ],
                },
            ],
            end: 7680,
        };
        let r = super::read(&smf.bytes()).unwrap();
        assert_eq!((r.format, r.timing), (1, Timing::Quarter(PPQ)));
        assert_eq!(r.smf.name, "Night drive");
        assert_eq!(r.smf.meters, smf.meters);
        assert_eq!(r.smf.end, 7680);
        assert_eq!(r.smf.tracks, smf.tracks);
        // Tempos go through whole microseconds a quarter.
        assert_eq!(r.smf.tempos.len(), 2);
        assert!((r.smf.tempos[0].1 - 92.0).abs() < 1e-3);
        assert_eq!(r.smf.tempos[1].0, 3840);
        assert_eq!((r.sysex, r.metas, r.tracks_left, r.cut), (0, 0, 0, false));
    }

    #[test]
    fn a_type_0_file_at_96_ticks_with_running_status_is_read_at_960() {
        let body: Vec<u8> = [
            &[0x00, 0xFF, 0x03, 4][..],
            b"Beat",
            // A SysEx and a marker: skipped and counted.
            &[0x00, 0xF0, 3, 0x7E, 0x09, 0xF7],
            &[0x00, 0xFF, 0x06, 1, b'A'],
            // Kick on channel 10, then a snare by running status.
            &[0x00, 0x99, 36, 100],
            &[0x18, 38, 90],
            // Note-ons at velocity 0 end them.
            &[0x18, 36, 0],
            &[0x00, 38, 0],
            // A bass note on channel 1, and a CC by running status.
            &[0x00, 0x90, 40, 80, 0x00, 0xB0, 74, 20, 0x10, 75, 30],
            &[0x81, 0x00, 0x80, 40, 0],
            &[0x00, 0xFF, 0x2F, 0],
        ]
        .concat();
        let r = super::read(&file(0, 96, &[&body])).unwrap();
        assert_eq!((r.format, r.timing), (0, Timing::Quarter(96)));
        assert_eq!(r.smf.name, "Beat");
        assert_eq!(r.smf.tracks.len(), 1);
        assert_eq!(r.smf.tracks[0].name, "", "type 0's name is the song's");
        assert_eq!(
            r.smf.tracks[0].events,
            [
                (0, on(9, 36, 100)),
                (240, on(9, 38, 90)),
                (480, off(9, 36)),
                (480, off(9, 38)),
                (480, on(0, 40, 80)),
                (
                    480,
                    Msg::Cc {
                        ch: 0,
                        cc: 74,
                        value: 20
                    }
                ),
                (
                    640,
                    Msg::Cc {
                        ch: 0,
                        cc: 75,
                        value: 30
                    }
                ),
                (
                    1920,
                    Msg::NoteOff {
                        ch: 0,
                        note: 40,
                        vel: 0
                    }
                ),
            ]
        );
        assert_eq!(r.smf.end, 1920);
        assert_eq!((r.sysex, r.metas), (1, 1));
        // No tempo or meter: the caller takes the standard's 120 and 4/4.
        assert!(r.smf.tempos.is_empty() && r.smf.meters.is_empty());
    }

    #[test]
    fn odd_ticks_round_to_the_nearest_and_frames_are_read_at_120() {
        assert_eq!(to_ppq(1, Timing::Quarter(480)), 2);
        assert_eq!(to_ppq(1, Timing::Quarter(1920)), 1, "a half rounds up");
        assert_eq!(to_ppq(1, Timing::Quarter(3840)), 0);
        assert_eq!(
            to_ppq(u64::from(u32::MAX), Timing::Quarter(1)),
            u64::from(u32::MAX) * 960
        );
        // 25 frames of 40 ticks: 1000 ticks a second, two quarters at 120.
        let body = [
            0x00, 0x90, 60, 100, 0x87, 0x68, 0x80, 60, 0, 0x00, 0xFF, 0x2F, 0,
        ];
        let div = (((-25i8) as u8 as u16) << 8) | 40;
        let r = super::read(&file(0, div, &[&body])).unwrap();
        assert_eq!(
            r.timing,
            Timing::Frames {
                fps: 25.0,
                ticks: 40
            }
        );
        assert_eq!(r.smf.tracks[0].events[1].0, 1920);
        assert_eq!(r.smf.tempos, [(0, FRAMES_BPM)]);
    }

    #[test]
    fn a_conductor_track_names_the_song_and_unknown_chunks_are_skipped() {
        let conductor: Vec<u8> = [
            &[0x00, 0xFF, 0x03, 3][..],
            b"Set",
            &[0x00, 0xFF, 0x58, 4, 3, 2, 24, 8],
            &[0x00, 0xFF, 0x51, 3, 0x09, 0x27, 0xC0],
            &[0x00, 0xFF, 0x2F, 0],
        ]
        .concat();
        let lead: Vec<u8> = [
            &[0x00, 0xFF, 0x03, 4][..],
            b"Lead",
            &[0x00, 0xFF, 0x03, 5],
            b"Other",
            &[
                0x00, 0x92, 72, 64, 0x83, 0x60, 0x82, 72, 0, 0x00, 0xFF, 0x2F, 0,
            ],
        ]
        .concat();
        let mut b = file(1, 480, &[&conductor]);
        // A header longer than 6 bytes, and a chunk of another kind.
        b[7] = 8;
        b.splice(14..14, [0, 0]);
        b.extend_from_slice(b"XFIH\0\0\0\x02ab");
        chunk(&mut b, &lead);
        let r = super::read(&b).unwrap();
        assert_eq!(r.smf.name, "Set");
        assert_eq!(r.smf.meters, [(0, 3, 4)]);
        assert_eq!(r.smf.tempos, [(0, 100.0)]);
        assert_eq!(
            r.smf.tracks.len(),
            1,
            "the conductor is the song's, not a track"
        );
        assert_eq!(r.smf.tracks[0].name, "Lead", "a track's first name");
        assert_eq!(
            r.smf.tracks[0].events[1],
            (
                960,
                Msg::NoteOff {
                    ch: 2,
                    note: 72,
                    vel: 0
                }
            )
        );
        assert_eq!(r.metas, 1, "the second name");
    }

    #[test]
    fn files_it_cant_read_say_why() {
        assert_eq!(super::read(b"RIFF....WAVE"), Err(SmfError::NotMidi));
        assert_eq!(
            super::read(b"MThd\0\0\0\x06\0\x03\0\x01\x01\xE0"),
            Err(SmfError::BadHeader)
        );
        assert_eq!(
            super::read(b"MThd\0\0\0\x06\0\x01\0\x01\0\0"),
            Err(SmfError::BadHeader)
        );
        assert_eq!(
            super::read(b"MThd\0\0\0\x06\0\x01\0\x00\x01\xE0"),
            Err(SmfError::NoTracks)
        );
        let big = vec![0u8; MAX_FILE_BYTES + 1];
        assert_eq!(super::read(&big), Err(SmfError::TooBig(MAX_FILE_BYTES + 1)));
        let e = super::read(&file(0, 96, &[&[0x00, 40, 100]])).unwrap_err();
        assert_eq!(
            e,
            SmfError::Bad {
                track: 1,
                at: 23,
                what: "a data byte with no message before it"
            }
        );
        assert_eq!(
            e.to_string(),
            "track 1, byte 23: a data byte with no message before it"
        );
        let e = super::read(&file(0, 96, &[&[0x00, 0xF2, 0, 0]])).unwrap_err();
        assert!(e.to_string().contains("system message"), "{e}");
        let e = super::read(&file(0, 96, &[&[0x80, 0x80, 0x80, 0x80, 0x00]])).unwrap_err();
        assert!(e.to_string().contains("4 bytes"), "{e}");
        let e = super::read(&file(0, 96, &[&[0x00, 0x90, 60, 0x90, 61, 1]])).unwrap_err();
        assert!(e.to_string().contains("cut short"), "{e}");
    }

    #[test]
    fn a_file_cut_short_keeps_what_came_before() {
        let body = [0x00, 0x90, 60, 100, 0x60, 0x80, 60, 0, 0x00, 0xFF, 0x2F, 0];
        let whole = file(0, 96, &[&body]);
        let r = super::read(&whole[..whole.len() - 6]).unwrap();
        assert!(r.cut);
        assert_eq!(r.smf.tracks[0].events, [(0, on(0, 60, 100))]);
        // A chunk that holds whole events but no end of track: read, and
        // not cut short.
        let r = super::read(&file(0, 96, &[&body[..8]])).unwrap();
        assert!(!r.cut);
        assert_eq!(r.smf.tracks[0].events.len(), 2);
        assert_eq!(r.smf.end, 960);
    }

    #[test]
    fn no_bytes_make_it_panic() {
        // Every prefix and every single byte changed of a real file, and
        // pseudo-random bytes after a good header.
        let smf = Smf {
            name: "x".into(),
            tempos: vec![(0, 120.0)],
            meters: vec![(0, 4, 4)],
            tracks: vec![SmfTrack {
                name: "t".into(),
                events: vec![
                    (0, on(0, 60, 100)),
                    (10, Msg::PitchBend { ch: 1, value: -3 }),
                    (20, Msg::Program { ch: 2, program: 5 }),
                    (30, off(0, 60)),
                ],
            }],
            end: 40,
        };
        let b = smf.bytes();
        for n in 0..b.len() {
            let _ = super::read(&b[..n]);
        }
        for i in 0..b.len() {
            for v in [0x00, 0x7F, 0x80, 0xF0, 0xFF] {
                let mut c = b.clone();
                c[i] = v;
                let _ = super::read(&c);
            }
        }
        let mut x = 0x2545_F491u32;
        for _ in 0..2000 {
            let mut c = file(1, 96, &[]);
            let mut t = Vec::new();
            for _ in 0..64 {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                t.push(x as u8);
            }
            chunk(&mut c, &t);
            let _ = super::read(&c);
        }
    }
}
