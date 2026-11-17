//! MIDI file import (the MIDI powerhouse's PR 6): a `.mid` that BROWSE
//! loads becomes clips, on the armed track and the tracks after it. It is
//! how patterns come across from the MPC (its *Track as MIDI* and *All
//! Tracks* exports) and from any other sequencer or DAW.
//!
//! - [`import`] reads the file's bytes (`mpfree_midi::smf::read`, on the
//!   screen's side in a thread of its own: [`read_in_thread`]) into a
//!   [`MidiImport`]: a [`Part`] for each file track that plays notes (one
//!   for each channel when a track plays on several, as a type 0 file's
//!   one track does), each note paired with its note-off, at 960 ticks a
//!   quarter. The clips are as long as the file, in whole bars of its first
//!   meter (4/4 if it has none), and at most a clip's longest.
//! - [`put`] (the sound loop, which has the instruments) makes each part a
//!   clip for its track, each note as the pad that plays it there: on a
//!   kit the MPC's own pad notes, so an MPC drum export comes back on the
//!   pads it was played on; on the synth or a track's own instrument the
//!   pad layout's (a pad that plays that note alone). Clips hold pads, not
//!   notes (the sequencer's design), so a melody comes in as far as the
//!   pad layout reaches; a note no pad plays is left out and counted. The
//!   clips go in as one take, so one UNDO on the Clips page takes the whole
//!   file away; none is launched.
//! - Left out and counted: controllers, pitch bend, pressure and program
//!   changes (lanes come with the sequencer), SysEx and other metas. The
//!   file's tempo is told, not set.

use std::path::PathBuf;
use std::sync::mpsc;

use mpfree_midi::msg::Msg;
use mpfree_midi::smf::{self, Timing};
use mpfree_seq::{Clip, Note, MAX_CLIP_TICKS, MAX_NOTES, MIN_CLIP_TICKS};
use mpfree_transport::time::Tick;

/// Clips one file puts on tracks at most: the armed track and the 15
/// after it. More of its parts are counted ([`MidiImport::parts_left`]).
pub const MAX_PARTS: usize = 16;

/// One note of a file, paired with its note-off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileNote {
    /// From the file's start, at 960 a quarter.
    pub tick: u64,
    /// At least 1.
    pub len: u64,
    pub note: u8,
    /// 1 to 127.
    pub velocity: u8,
}

/// A file track's notes on one channel: one clip.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub name: String,
    pub channel: u8,
    pub notes: Vec<FileNote>,
}

/// A read file, ready to be put on tracks ([`put`]).
#[derive(Clone, Debug, PartialEq)]
pub struct MidiImport {
    /// The file's name, for what is said.
    pub file: String,
    pub parts: Vec<Part>,
    /// Every clip's length: whole bars.
    pub length: Tick,
    /// Its first tempo (120 if it has none, as the standard says).
    pub bpm: f64,
    /// Its first meter (4/4 if it has none).
    pub meter: (u8, u8),
    /// Notes that start past the longest clip, left out.
    pub past_end: usize,
    /// Notes past a clip's [`MAX_NOTES`], left out.
    pub too_many: usize,
    /// Notes with no note-off: they end at the file's end.
    pub unended: usize,
    /// Controllers, pitch bend, pressure and program changes, left out.
    pub moves: usize,
    /// SysEx and metas it skipped (lyrics, markers ...).
    pub skipped: usize,
    /// Parts past [`MAX_PARTS`], and file tracks past what the reader
    /// reads, left out.
    pub parts_left: usize,
    /// The file ends inside a track: read up to there.
    pub cut: bool,
    /// A file in clock time (SMPTE frames), read at 120 BPM.
    pub frames: bool,
}

/// The file `name`'s bytes as a [`MidiImport`]. An error says why, for the
/// musician: not a MIDI file, or one with no notes.
pub fn import(name: &str, bytes: &[u8]) -> Result<MidiImport, String> {
    let r = smf::read(bytes).map_err(|e| e.to_string())?;
    let meter = r
        .smf
        .meters
        .first()
        .filter(|m| m.0 == 0)
        .map_or((4, 4), |m| (m.1, m.2));
    let bpm = r.smf.tempos.first().map_or(120.0, |t| t.1);
    let bar = (u64::from(meter.0) * 4 * u64::from(smf::PPQ) / u64::from(meter.1.max(1))).max(1);
    let stem = name
        .rsplit_once('.')
        .map_or(name, |(s, _)| s)
        .trim()
        .to_string();
    let song = if r.smf.name.is_empty() {
        stem
    } else {
        r.smf.name.clone()
    };
    let mut parts: Vec<Part> = Vec::new();
    let (mut moves, mut unended) = (0usize, 0usize);
    let mut last = r.smf.end;
    for (k, t) in r.smf.tracks.iter().enumerate() {
        // Each channel's notes, paired first in, first out; a note still on
        // at the end ends there.
        let mut open: Vec<Vec<(u64, u8)>> = vec![Vec::new(); 16 * 128];
        let mut by_ch: [Vec<FileNote>; 16] = Default::default();
        for &(tick, m) in &t.events {
            match m {
                Msg::NoteOn { ch, note, vel } => {
                    open[usize::from(ch & 15) * 128 + usize::from(note & 127)].push((tick, vel))
                }
                Msg::NoteOff { ch, note, .. } => {
                    let q = &mut open[usize::from(ch & 15) * 128 + usize::from(note & 127)];
                    if !q.is_empty() {
                        let (start, vel) = q.remove(0);
                        by_ch[usize::from(ch & 15)].push(FileNote {
                            tick: start,
                            len: (tick - start).max(1),
                            note,
                            velocity: vel.max(1),
                        });
                        last = last.max(start + 1);
                    }
                }
                _ => moves += 1,
            }
        }
        for (i, q) in open.iter().enumerate() {
            for &(start, vel) in q {
                let end = r.smf.end.max(start + 1);
                by_ch[i / 128].push(FileNote {
                    tick: start,
                    len: end - start,
                    note: (i % 128) as u8,
                    velocity: vel.max(1),
                });
                unended += 1;
                last = last.max(start + 1);
            }
        }
        let used: Vec<usize> = (0..16).filter(|&c| !by_ch[c].is_empty()).collect();
        let base = if !t.name.is_empty() {
            t.name.clone()
        } else if r.smf.tracks.len() == 1 {
            song.clone()
        } else {
            format!("{song} {}", k + 1)
        };
        for &c in &used {
            let mut notes = std::mem::take(&mut by_ch[c]);
            notes.sort_by_key(|n| (n.tick, n.note));
            parts.push(Part {
                name: if used.len() > 1 {
                    format!("{base} ch {}", c + 1)
                } else {
                    base.clone()
                },
                channel: c as u8,
                notes,
            });
        }
    }
    if parts.is_empty() {
        return Err(if moves > 0 {
            "it has no notes, only controllers or other moves".to_string()
        } else {
            "it has no notes".to_string()
        });
    }
    let parts_left = parts.len().saturating_sub(MAX_PARTS) + r.tracks_left;
    parts.truncate(MAX_PARTS);
    // Whole bars, at least a clip's shortest, at most its longest.
    let longest = MAX_CLIP_TICKS as u64 / bar * bar;
    let shortest = (MIN_CLIP_TICKS as u64).div_ceil(bar) * bar;
    let length = (last.div_ceil(bar) * bar).clamp(shortest, longest.max(shortest));
    let (mut past_end, mut too_many) = (0usize, 0usize);
    for p in &mut parts {
        let before = p.notes.len();
        p.notes.retain(|n| n.tick < length);
        past_end += before - p.notes.len();
        if p.notes.len() > MAX_NOTES {
            too_many += p.notes.len() - MAX_NOTES;
            p.notes.truncate(MAX_NOTES);
        }
    }
    Ok(MidiImport {
        file: name.to_string(),
        parts,
        length: length as Tick,
        bpm,
        meter,
        past_end,
        too_many,
        unended,
        moves,
        skipped: r.sysex + r.metas,
        parts_left,
        cut: r.cut,
        frames: matches!(r.timing, Timing::Frames { .. }),
    })
}

/// Reads and imports the file at `path` in a thread of its own, so a slow
/// stick never holds the screen. The answer comes once on the channel.
pub fn read_in_thread(path: PathBuf) -> mpsc::Receiver<Result<MidiImport, String>> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("mpfree-midifile".into())
        .spawn({
            let tx = tx.clone();
            move || {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let r = read_file(&path).and_then(|b| import(&name, &b));
                let _ = tx.send(r);
            }
        });
    if let Err(e) = spawned {
        let _ = tx.send(Err(format!("not read ({e})")));
    }
    rx
}

/// The file's bytes, refused past the reader's most before they are read.
fn read_file(path: &std::path::Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let f = std::fs::File::open(path).map_err(|e| format!("not read ({e})"))?;
    let mut b = Vec::new();
    f.take(smf::MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut b)
        .map_err(|e| format!("not read ({e})"))?;
    if b.len() > smf::MAX_FILE_BYTES {
        return Err(smf::SmfError::TooBig(b.len()).to_string());
    }
    Ok(b)
}

/// What [`clips`] made, and what it couldn't.
#[derive(Clone, Debug, PartialEq)]
pub struct Made {
    /// (track, clip), in the file's order.
    pub clips: Vec<(usize, Clip)>,
    /// Notes no pad plays on their track.
    pub unplayed: usize,
    /// Parts past the project's last track.
    pub no_track: usize,
}

/// Each part of `m` as a clip on track `first + i` (of `tracks`), its notes
/// as `pad(track, note)` says (None: no pad plays it). A part none of
/// whose notes a pad plays makes no clip.
pub fn clips(
    m: &MidiImport,
    first: usize,
    tracks: usize,
    pad: impl Fn(usize, u8) -> Option<u8>,
) -> Made {
    let mut made = Made {
        clips: Vec::new(),
        unplayed: 0,
        no_track: 0,
    };
    for (i, p) in m.parts.iter().enumerate() {
        let track = first + i;
        if track >= tracks {
            made.no_track += 1;
            continue;
        }
        let mut c = Clip::new(&p.name, m.length);
        for n in &p.notes {
            let Some(at) = pad(track, n.note) else {
                made.unplayed += 1;
                continue;
            };
            let len = (n.len as Tick).clamp(1, MAX_CLIP_TICKS);
            if c.add_note(Note::new(n.tick as Tick, len, at, n.velocity))
                .is_err()
            {
                made.unplayed += 1;
            }
        }
        if !c.notes.is_empty() {
            made.clips.push((track, c));
        }
    }
    made
}

/// The pad that plays `note` alone on track `track`, as its clips play
/// (`SeqPads` in main.rs): a track's own instrument through its pad map;
/// else the synth's pad layout while the synth has the pads (`synth`), or
/// the kit's pads by the MPC's own pad notes (`midi_out::kit_notes`).
pub fn pad_on_track(
    own: &crate::instruments::Instruments,
    synth: Option<&crate::synth::Synth>,
    track: usize,
    note: u8,
) -> Option<u8> {
    if let Some(p) = own.pad_of_note(track, note) {
        return p;
    }
    match synth {
        Some(s) => pad_playing(note, |p| s.notes(p).to_vec()),
        None => pad_playing(note, |p| crate::midi_out::kit_notes(p).to_vec()),
    }
}

/// The pad of `notes(pad)` (pads 0 to 15) that plays `note` alone.
pub fn pad_playing(note: u8, notes: impl Fn(usize) -> Vec<u8>) -> Option<u8> {
    (0..16).find(|&p| notes(p) == [note]).map(|p| p as u8)
}

/// Puts `m` on the armed track (track 1 if none is) and the tracks after
/// it, through `pad` (see [`clips`]), as one take. The line to say: what
/// went where and what was left out, or why nothing went in.
pub fn put(
    time: &mut crate::time::TimeHost,
    m: &MidiImport,
    pad: impl Fn(usize, u8) -> Option<u8>,
) -> Result<String, String> {
    let first = time.armed().unwrap_or(0);
    let made = clips(m, first, time.track_count(), pad);
    let name = &m.file;
    if made.clips.is_empty() {
        return Err(format!(
            "MIDI file {name}: nothing went in: no pad plays its notes on {}",
            time.project()
                .tracks
                .get(first)
                .map_or("the armed track", |t| t.name.as_str())
        ));
    }
    let line = time
        .put_clips(made.clips)
        .map_err(|e| format!("MIDI file {name}: nothing went in: {e}"))?;
    let mut said = format!(
        "MIDI file {name}: {line} (UNDO takes them away); its tempo {} BPM{}",
        trim(m.bpm),
        if m.frames {
            ", read from clock time"
        } else {
            ""
        }
    );
    let mut left: Vec<String> = Vec::new();
    if made.unplayed > 0 {
        left.push(format!(
            "{} no pad plays",
            count(made.unplayed, "note", "notes")
        ));
    }
    let no_track = made.no_track + m.parts_left;
    if no_track > 0 {
        left.push(format!("{} of its tracks with no track to go on", no_track));
    }
    if m.past_end + m.too_many > 0 {
        left.push(format!(
            "{} past a clip's longest or fullest",
            count(m.past_end + m.too_many, "note", "notes")
        ));
    }
    if m.moves > 0 {
        left.push(count(
            m.moves,
            "controller, bend, pressure or program move",
            "controller, bend, pressure and program moves",
        ));
    }
    if !left.is_empty() {
        said += &format!("; left out: {}", left.join(", "));
    }
    if m.cut {
        said += "; the file ends early, read up to there";
    }
    Ok(said)
}

/// `"1 note"`, `"3 notes"`.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn trim(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_midi::smf::{Smf, SmfTrack};

    fn on(ch: u8, note: u8, vel: u8) -> Msg {
        Msg::NoteOn { ch, note, vel }
    }

    fn off(ch: u8, note: u8) -> Msg {
        Msg::NoteOff { ch, note, vel: 64 }
    }

    /// What the MPC's All Tracks export is like: a conductor track, then a
    /// drum track on its pad notes and a bass track, 2 bars at 92 BPM.
    fn mpc_export() -> Vec<u8> {
        Smf {
            name: "Sequence01".into(),
            tempos: vec![(0, 92.0)],
            meters: vec![(0, 4, 4)],
            tracks: vec![
                SmfTrack {
                    name: "Drums".into(),
                    events: vec![
                        // Pad A01 (C#2) and A02 (C2), and a note on no pad.
                        (0, on(9, 37, 120)),
                        (240, off(9, 37)),
                        (960, on(9, 36, 90)),
                        (1200, off(9, 36)),
                        (1920, on(9, 99, 90)),
                        (2000, off(9, 99)),
                    ],
                },
                SmfTrack {
                    name: "Bass".into(),
                    events: vec![
                        (0, on(0, 40, 100)),
                        (
                            100,
                            Msg::Cc {
                                ch: 0,
                                cc: 1,
                                value: 3,
                            },
                        ),
                        (480, off(0, 40)),
                        // Left on: the writer ends it at the file's end.
                        (3840, on(0, 43, 80)),
                    ],
                },
            ],
            end: 7680,
        }
        .bytes()
    }

    #[test]
    fn an_mpc_export_is_a_part_a_track_in_whole_bars() {
        let m = import("Sequence01.mid", &mpc_export()).unwrap();
        assert_eq!(m.parts.len(), 2);
        assert_eq!((m.length, m.meter, m.bpm.round()), (7680, (4, 4), 92.0));
        assert_eq!(m.parts[0].name, "Drums");
        assert_eq!(m.parts[0].channel, 9);
        assert_eq!(
            m.parts[0].notes[..2],
            [
                FileNote {
                    tick: 0,
                    len: 240,
                    note: 37,
                    velocity: 120
                },
                FileNote {
                    tick: 960,
                    len: 240,
                    note: 36,
                    velocity: 90
                }
            ]
        );
        assert_eq!(m.parts[1].name, "Bass");
        assert_eq!(m.parts[1].notes[1].len, 3840, "to the file's end");
        assert_eq!((m.unended, m.moves, m.parts_left, m.cut), (0, 1, 0, false));
    }

    #[test]
    fn a_note_with_no_note_off_ends_at_the_files_end() {
        // Type 0, 96 a quarter: a note on at 0, its track ended at 4 bars
        // with no note-off.
        let mut b = b"MThd\0\0\0\x06\0\0\0\x01\0\x60MTrk\0\0\0\x0b".to_vec();
        b.extend_from_slice(&[0x00, 0x90, 60, 100, 0x83, 0x00, 0xFF, 0x2F, 0x00]);
        let n = b.len() - 22;
        b[21] = n as u8;
        let m = import("held.mid", &b).unwrap();
        assert_eq!(m.unended, 1);
        assert_eq!(m.parts[0].notes[0].len, 3840);
        assert_eq!(m.length, 3840);
    }

    #[test]
    fn a_type_0_file_is_a_part_a_channel_and_short_files_are_one_bar() {
        let smf = Smf {
            name: "".into(),
            tempos: vec![],
            meters: vec![(0, 7, 8)],
            tracks: vec![SmfTrack {
                name: "".into(),
                events: vec![
                    (0, on(9, 36, 100)),
                    (10, off(9, 36)),
                    (0, on(1, 60, 100)),
                    (10, off(1, 60)),
                ],
            }],
            end: 0,
        };
        let m = import("groove.mid", &smf.bytes()).unwrap();
        // 7/8: a bar is 3360 ticks.
        assert_eq!((m.length, m.bpm), (3360, 120.0));
        let names: Vec<&str> = m.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["groove ch 2", "groove ch 10"]);
    }

    #[test]
    fn files_with_nothing_to_play_say_why() {
        let e = import("x.mid", b"RIFF").unwrap_err();
        assert!(e.contains("not a MIDI file"), "{e}");
        let only_cc = Smf {
            name: "".into(),
            tempos: vec![],
            meters: vec![],
            tracks: vec![SmfTrack {
                name: "".into(),
                events: vec![(
                    0,
                    Msg::Cc {
                        ch: 0,
                        cc: 7,
                        value: 1,
                    },
                )],
            }],
            end: 960,
        };
        let e = import("x.mid", &only_cc.bytes()).unwrap_err();
        assert!(e.contains("no notes"), "{e}");
    }

    #[test]
    fn notes_go_on_the_pads_that_play_them_and_the_rest_are_counted() {
        let m = import("Sequence01.mid", &mpc_export()).unwrap();
        // Track 1 a kit (the MPC's pad notes), track 2 a synth whose pad 5
        // plays E1 (40) alone.
        let kit = |n: u8| mpfree_controller::mpc_one::pad_index(n);
        let made = clips(&m, 0, 2, |t, n| {
            if t == 0 {
                kit(n)
            } else {
                pad_playing(n, |p| if p == 4 { vec![40] } else { vec![p as u8 + 70] })
            }
        });
        assert_eq!(made.clips.len(), 2);
        let drums = &made.clips[0].1;
        assert_eq!(made.clips[0].0, 0);
        let pads: Vec<u8> = drums.notes.iter().map(|n| n.note).collect();
        assert_eq!(pads, [0, 1], "pads A01 and A02");
        assert_eq!(drums.length, 7680);
        assert_eq!(made.clips[1].1.notes.len(), 1);
        assert_eq!(made.clips[1].1.notes[0].note, 4);
        // Note 99 and G1 (43) play no pad.
        assert_eq!((made.unplayed, made.no_track), (2, 0));
        // From the last track: the bass has nowhere to go.
        let made = clips(&m, 1, 2, |_, n| kit(n));
        assert_eq!((made.clips.len(), made.no_track), (1, 1));
    }

    #[test]
    fn a_file_goes_on_from_the_armed_track_as_one_take() {
        use crate::time::TimeHost;
        use mpfree_time_ui::view::TimeAction;
        let mut h = TimeHost::new(48_000, 120.0).unwrap();
        h.act(TimeAction::Arm(Some(2))).unwrap();
        let m = import("Sequence01.mid", &mpc_export()).unwrap();
        let line = put(&mut h, &m, |_, n| mpfree_controller::mpc_one::pad_index(n)).unwrap();
        assert_eq!(
            line,
            "MIDI file Sequence01.mid: 2 clips with 4 notes, from slot 1 of Track 3 \
             (UNDO takes them away); its tempo 92 BPM; left out: 1 note no pad plays, \
             1 controller, bend, pressure or program move"
        );
        assert_eq!(
            h.project().tracks[2].slots[0].as_ref().unwrap().name,
            "Drums"
        );
        assert_eq!(
            h.project().tracks[3].slots[0].as_ref().unwrap().name,
            "Bass"
        );
        h.act(TimeAction::Undo).unwrap();
        assert!(h.project().tracks[2].slots[0].is_none());
        assert!(h.project().tracks[3].slots[0].is_none());
        let e = put(&mut h, &m, |_, _| None).unwrap_err();
        assert!(e.contains("no pad plays its notes on Track 3"), "{e}");
    }
}
