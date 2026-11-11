//! MIDI out of the arp: its notes as MIDI messages, for another instrument,
//! a computer or a MIDI file.
//!
//! The engine's notes are pad slots on the pad player (pad 1 = 0) or MIDI
//! notes with a pitched sound; a [`NoteMap`] turns each into the MIDI note
//! to send. [`MidiOut`] keeps count of the notes it has turned on, so every
//! note-on it sends gets exactly one note-off, even when the map or the
//! channel changes while notes sound, and [`MidiOut::all_off`] ends them
//! all (STOP, or MIDI out turned off). Like the engine it never allocates,
//! so it can run in the audio thread.
//!
//! [`midi_file`] writes what was played as a Standard MIDI File. It
//! allocates and is for tools, not the audio thread.
//!
//! Nothing here opens a port: where the bytes go is the caller's choice.

use crate::engine::{Event, EventKind, PPQ};

/// One MIDI message of up to three bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    /// Frames into the period, as the engine's event.
    pub offset: u32,
    pub bytes: [u8; 3],
}

impl Message {
    pub fn note_on(offset: u32, channel: u8, note: u8, velocity: u8) -> Self {
        Self { offset, bytes: [0x90 | (channel & 0x0F), note & 0x7F, velocity.clamp(1, 127)] }
    }

    /// A note-off with release velocity 64, the value the MIDI standard
    /// gives for keyboards that do not sense it.
    pub fn note_off(offset: u32, channel: u8, note: u8) -> Self {
        Self { offset, bytes: [0x80 | (channel & 0x0F), note & 0x7F, 64] }
    }
}

/// Which MIDI note each engine note sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteMap {
    /// The engine's notes are MIDI notes already (a pitched sound).
    Same,
    /// Engine note N sends `base + N`, for pad slots: pad 1 sends `base`.
    /// Notes past 127 are not sent.
    Chromatic { base: u8 },
    /// Engine note N sends `notes[N]`, for a drum map; notes past the
    /// table are not sent.
    Table([u8; 16]),
}

impl NoteMap {
    /// Pad slots from C1 (36) up, where General MIDI drum maps and most
    /// drum machines start.
    pub const PADS: NoteMap = NoteMap::Chromatic { base: 36 };

    pub fn note(self, n: u8) -> Option<u8> {
        match self {
            NoteMap::Same => (n < 128).then_some(n),
            NoteMap::Chromatic { base } => {
                let m = u16::from(base) + u16::from(n);
                (m < 128).then_some(m as u8)
            }
            NoteMap::Table(t) => t.get(usize::from(n)).copied().filter(|&m| m < 128),
        }
    }
}

/// Turns the engine's events into MIDI messages on one channel.
#[derive(Clone, Debug)]
pub struct MidiOut {
    channel: u8,
    map: NoteMap,
    /// Note-ons sent and not yet ended, per MIDI note.
    sounding: [u8; 128],
}

impl MidiOut {
    /// `channel` is 0..=15 (channel 1 is 0).
    pub fn new(channel: u8, map: NoteMap) -> Self {
        Self { channel: channel & 0x0F, map, sounding: [0; 128] }
    }

    pub fn channel(&self) -> u8 {
        self.channel
    }

    pub fn map(&self) -> NoteMap {
        self.map
    }

    /// The MIDI message for one engine event, if it sends one. A note
    /// played again while it sounds sends another note-on; the note-off
    /// goes out with the last of its offs, so a receiver that keeps one
    /// voice per note never cuts a note that is still meant to sound.
    pub fn event(&mut self, e: &Event) -> Option<Message> {
        match e.kind {
            EventKind::On { note, velocity } => {
                let m = self.map.note(note)?;
                let n = &mut self.sounding[usize::from(m)];
                *n = n.saturating_add(1);
                Some(Message::note_on(e.offset, self.channel, m, velocity))
            }
            EventKind::Off { note } => {
                let m = self.map.note(note)?;
                let n = &mut self.sounding[usize::from(m)];
                match *n {
                    0 => None,
                    1 => {
                        *n = 0;
                        Some(Message::note_off(e.offset, self.channel, m))
                    }
                    _ => {
                        *n -= 1;
                        None
                    }
                }
            }
        }
    }

    /// Notes sounding now (sent on and not yet off).
    pub fn sounding(&self) -> usize {
        self.sounding.iter().filter(|&&n| n > 0).count()
    }

    /// Ends every sounding note, at `offset`: `send` gets each note-off.
    pub fn all_off(&mut self, offset: u32, mut send: impl FnMut(Message)) {
        for (m, n) in self.sounding.iter_mut().enumerate() {
            if *n > 0 {
                *n = 0;
                send(Message::note_off(offset, self.channel, m as u8));
            }
        }
    }

    /// A new channel: notes sounding on the old one end first.
    pub fn set_channel(&mut self, channel: u8, offset: u32, send: impl FnMut(Message)) {
        if channel & 0x0F != self.channel {
            self.all_off(offset, send);
            self.channel = channel & 0x0F;
        }
    }

    /// A new note map: notes sounding under the old one end first, so the
    /// offs to come cannot miss them.
    pub fn set_map(&mut self, map: NoteMap, offset: u32, send: impl FnMut(Message)) {
        if map != self.map {
            self.all_off(offset, send);
            self.map = map;
        }
    }
}

/// A Standard MIDI File (format 0, one track, [`PPQ`] ticks per quarter
/// note) of `messages`, each at an absolute frame, in time order. `bpm`
/// goes in as the file's tempo, and frames become ticks at that tempo.
/// For tools: it allocates.
pub fn midi_file(messages: &[(u64, [u8; 3])], sample_rate: u32, bpm: f64, name: &str) -> Vec<u8> {
    let mut track = Vec::new();
    // The track's name and tempo at tick 0.
    let name = name.as_bytes();
    track.push(0);
    track.extend_from_slice(&[0xFF, 0x03]);
    var_len(&mut track, name.len() as u32);
    track.extend_from_slice(name);
    let us_per_quarter = (60_000_000.0 / bpm.clamp(1.0, 1000.0)).round() as u32;
    track.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03]);
    track.extend_from_slice(&us_per_quarter.to_be_bytes()[1..]);
    let ticks_per_frame = bpm * f64::from(PPQ) / (60.0 * f64::from(sample_rate.max(1)));
    let mut last = 0u64;
    for &(frame, bytes) in messages {
        let tick = ((frame as f64) * ticks_per_frame).round() as u64;
        let delta = tick.saturating_sub(last).min(0x0FFF_FFFF);
        last = last.max(tick);
        var_len(&mut track, delta as u32);
        track.extend_from_slice(&bytes);
    }
    track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
    let mut out = Vec::with_capacity(22 + track.len());
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // format 0
    out.extend_from_slice(&1u16.to_be_bytes()); // one track
    out.extend_from_slice(&(PPQ as u16).to_be_bytes());
    out.extend_from_slice(b"MTrk");
    out.extend_from_slice(&(track.len() as u32).to_be_bytes());
    out.extend_from_slice(&track);
    out
}

/// A MIDI file's variable-length number: 7 bits a byte, high bit set on
/// all but the last.
fn var_len(out: &mut Vec<u8>, mut v: u32) {
    let mut bytes = [0u8; 4];
    let mut n = 0;
    loop {
        bytes[n] = (v & 0x7F) as u8;
        n += 1;
        v >>= 7;
        if v == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        out.push(bytes[i] | if i > 0 { 0x80 } else { 0 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Engine, Events, Mode, Settings};
    use crate::notes::LatchMode;

    fn on(offset: u32, note: u8, velocity: u8) -> Event {
        Event { offset, kind: EventKind::On { note, velocity } }
    }

    fn off(offset: u32, note: u8) -> Event {
        Event { offset, kind: EventKind::Off { note } }
    }

    #[test]
    fn pads_send_notes_from_c1_on_their_channel() {
        let mut m = MidiOut::new(9, NoteMap::PADS);
        assert_eq!(m.event(&on(5, 0, 100)), Some(Message { offset: 5, bytes: [0x99, 36, 100] }));
        assert_eq!(m.event(&on(6, 15, 0)), Some(Message { offset: 6, bytes: [0x99, 51, 1] }), "velocity 0 would be an off");
        assert_eq!(m.event(&off(7, 0)), Some(Message { offset: 7, bytes: [0x89, 36, 64] }));
        assert_eq!(m.event(&off(8, 0)), None, "no off for a note that is not sounding");
        assert_eq!(NoteMap::Chromatic { base: 120 }.note(8), None, "past 127 is not sent");
        assert_eq!(NoteMap::Same.note(60), Some(60));
        let mut t = [0u8; 16];
        t[1] = 38;
        assert_eq!(NoteMap::Table(t).note(1), Some(38));
        assert_eq!(NoteMap::Table(t).note(16), None);
    }

    #[test]
    fn a_note_played_again_ends_with_its_last_off() {
        let mut m = MidiOut::new(0, NoteMap::Same);
        assert!(m.event(&on(0, 60, 90)).is_some());
        assert!(m.event(&on(10, 60, 90)).is_some());
        assert_eq!(m.event(&off(20, 60)), None);
        assert_eq!(m.sounding(), 1);
        assert_eq!(m.event(&off(30, 60)), Some(Message::note_off(30, 0, 60)));
        assert_eq!(m.sounding(), 0);
    }

    #[test]
    fn stop_and_changes_end_every_sounding_note_once() {
        let mut m = MidiOut::new(0, NoteMap::Same);
        for n in [60, 64, 67] {
            m.event(&on(0, n, 100));
        }
        let mut sent = Vec::new();
        m.set_channel(1, 3, |msg| sent.push(msg));
        assert_eq!(sent, [60, 64, 67].map(|n| Message::note_off(3, 0, n)), "offs go out on the old channel");
        assert_eq!(m.event(&off(4, 64)), None, "already ended");
        m.event(&on(5, 62, 100));
        sent.clear();
        m.set_map(NoteMap::PADS, 6, |msg| sent.push(msg));
        assert_eq!(sent, [Message::note_off(6, 1, 62)]);
        sent.clear();
        m.set_map(NoteMap::PADS, 7, |msg| sent.push(msg));
        m.all_off(8, |msg| sent.push(msg));
        assert_eq!(sent, [], "nothing sounding, nothing sent");
    }

    #[test]
    fn every_note_on_from_the_engine_gets_one_note_off() {
        let mut e = Engine::new(44_100, 140.0);
        e.set_settings(Settings { mode: Mode::Arp, octaves: 2, gate: 100, ..Settings::default() });
        e.set_latch(LatchMode::Replace);
        for n in [60, 64, 67] {
            e.press(n, 100);
        }
        for n in [60, 64, 67] {
            e.release(n);
        }
        let mut m = MidiOut::new(0, NoteMap::Same);
        let mut ev = Events::new();
        let (mut ons, mut offs) = (0, 0);
        for i in 0..2000 {
            if i == 1500 {
                e.clear_latched();
            }
            e.process(128, &mut ev);
            for x in ev.as_slice() {
                match m.event(x).map(|msg| msg.bytes[0] & 0xF0) {
                    Some(0x90) => ons += 1,
                    Some(0x80) => offs += 1,
                    _ => {}
                }
            }
        }
        m.all_off(0, |_| offs += 1);
        assert!(ons > 30, "{ons}");
        assert_eq!(ons, offs);
    }

    #[test]
    fn the_midi_file_has_the_header_tempo_and_ticks() {
        // 120 BPM at 48 kHz: a quarter note is 24000 frames, PPQ ticks.
        let msgs = [(0, [0x90, 60, 100]), (24_000, [0x80, 60, 64]), (24_000 + 12_000, [0x90, 62, 90])];
        let f = midi_file(&msgs, 48_000, 120.0, "arp");
        assert_eq!(&f[..14], &[b'M', b'T', b'h', b'd', 0, 0, 0, 6, 0, 0, 0, 1, (PPQ >> 8) as u8, PPQ as u8]);
        assert_eq!(&f[14..18], b"MTrk");
        let len = u32::from_be_bytes(f[18..22].try_into().unwrap()) as usize;
        let t = &f[22..];
        assert_eq!(t.len(), len);
        // Name, tempo (500000 us per quarter), then the notes.
        assert_eq!(&t[..7], &[0, 0xFF, 0x03, 3, b'a', b'r', b'p']);
        assert_eq!(&t[7..14], &[0, 0xFF, 0x51, 3, 0x07, 0xA1, 0x20]);
        let mut d = Vec::new();
        var_len(&mut d, PPQ);
        let mut want = vec![0, 0x90, 60, 100];
        want.extend_from_slice(&d);
        want.extend_from_slice(&[0x80, 60, 64]);
        let mut half = Vec::new();
        var_len(&mut half, PPQ / 2);
        want.extend_from_slice(&half);
        want.extend_from_slice(&[0x90, 62, 90, 0, 0xFF, 0x2F, 0]);
        assert_eq!(&t[14..], &want[..]);
    }

    #[test]
    fn variable_length_numbers() {
        for (v, want) in [(0u32, vec![0x00]), (0x7F, vec![0x7F]), (0x80, vec![0x81, 0x00]), (960, vec![0x87, 0x40]),
                          (0x0FFF_FFFF, vec![0xFF, 0xFF, 0xFF, 0x7F])] {
            let mut out = Vec::new();
            var_len(&mut out, v);
            assert_eq!(out, want, "{v}");
        }
    }
}
