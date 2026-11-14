//! What an export makes and where: the streams (a capture point and a file
//! each), the MIDI file, the folder, and names a USB stick takes.

use std::path::PathBuf;

use mpfree_fx::{CapturePoint, StripId};

use crate::mode::Mode;

/// One WAV file of the export: a capture point and the file's name
/// (without `.wav`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stream {
    pub name: String,
    pub strip: StripId,
    pub point: CapturePoint,
}

impl Stream {
    /// The mixdown: the master after Master safety, what the speakers get.
    pub fn mixdown() -> Stream {
        Stream {
            name: "Mixdown".into(),
            strip: StripId::Master,
            point: CapturePoint::AfterSafety,
        }
    }

    /// A stem: a track or return after its fader, pan and mute.
    pub fn stem(name: &str, strip: StripId) -> Stream {
        Stream {
            name: name.into(),
            strip,
            point: CapturePoint::AfterFader,
        }
    }
}

/// The MIDI file: the notes each track played, with the song's tempo map
/// and meter (see `mpfree_midi::smf`).
#[derive(Clone, Debug, PartialEq)]
pub struct MidiPlan {
    /// Track names by track index.
    pub tracks: Vec<String>,
    /// Each tempo and the tick it starts at, from the range's start.
    pub tempos: Vec<(u64, f64)>,
    /// Each meter (beats, beat value) and its tick.
    pub meters: Vec<(u64, u8, u8)>,
    /// The range's end, in ticks.
    pub end: u64,
    /// Events the pass can hold (about 16 bytes each), made beforehand.
    pub room: usize,
}

impl MidiPlan {
    /// Room for 65536 notes and controls: about 1 MB.
    pub const ROOM: usize = 1 << 16;
}

/// One export (see the crate docs).
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// The folder the files go in: made by `start`; it must be new or
    /// empty.
    pub folder: PathBuf,
    /// The project's name: the MIDI file's name and its song name.
    pub song: String,
    /// The WAV files, in order.
    pub streams: Vec<Stream>,
    pub midi: Option<MidiPlan>,
    pub mode: Mode,
    pub rate: u32,
    /// The range's length in frames, when known: for the time left.
    pub length: Option<u64>,
    /// The first of the mixer's capture slots the export may use; it uses
    /// one per stream from there.
    pub first_slot: usize,
}

/// Characters a FAT or exFAT stick (and Windows) refuses in a name.
fn refused(c: char) -> bool {
    c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
}

/// A name a USB stick takes: refused characters become `-`, spaces and
/// dots at the ends go, and it is at most 80 characters ("Untitled" if
/// nothing is left).
pub fn safe_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if refused(c) { '-' } else { c })
        .take(80)
        .collect();
    let s = s.trim_matches(|c: char| c == ' ' || c == '.');
    if s.is_empty() {
        "Untitled".into()
    } else {
        s.to_string()
    }
}

/// The folder's name: the project's and the time (as the app writes it,
/// for example "Night drive 2026-11-14 01.32").
pub fn folder_name(project: &str, stamp: &str) -> String {
    safe_name(&format!("{} {}", project.trim(), stamp.trim()))
}

/// Each stream's file name (`.wav` added), safe for a stick and different
/// from the others ignoring case, as FAT compares them: a second "Drums"
/// is "Drums 2".
pub fn file_names(streams: &[Stream]) -> Vec<String> {
    let mut taken: Vec<String> = Vec::with_capacity(streams.len());
    let mut out = Vec::with_capacity(streams.len());
    for s in streams {
        let base = safe_name(&s.name);
        let mut name = base.clone();
        let mut n = 2;
        while taken.contains(&name.to_lowercase()) {
            name = format!("{base} {n}");
            n += 1;
        }
        taken.push(name.to_lowercase());
        out.push(format!("{name}.wav"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_a_stick_takes() {
        assert_eq!(safe_name("Drums / Perc: 2?"), "Drums - Perc- 2-");
        assert_eq!(safe_name("  .hidden. "), "hidden");
        assert_eq!(safe_name("..."), "Untitled");
        assert_eq!(safe_name(&"a".repeat(200)).len(), 80);
        assert_eq!(
            folder_name("My song", "2026-11-14 0132"),
            "My song 2026-11-14 0132"
        );
    }

    #[test]
    fn two_streams_never_share_a_file() {
        let streams = [
            Stream::mixdown(),
            Stream::stem("Drums", StripId::Track(0)),
            Stream::stem("drums", StripId::Track(1)),
            Stream::stem("Drums", StripId::Track(2)),
            Stream::stem("Mix/down", StripId::Return(0)),
        ];
        assert_eq!(
            file_names(&streams),
            [
                "Mixdown.wav",
                "Drums.wav",
                "drums 2.wav",
                "Drums 3.wav",
                "Mix-down.wav"
            ]
        );
    }
}
