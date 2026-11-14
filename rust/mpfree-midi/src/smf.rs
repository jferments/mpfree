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
}
