//! Which notes are held, and Latch.
//!
//! "Notes" here are whatever numbers the app feeds in. The pad player feeds
//! pad slots (0..=15, pad 1 = 0) so that "up" means up the pads, not up
//! Akai's scrambled pad note numbers; a melodic program would feed MIDI notes.

/// Most notes held or latched at once. 16 pads plus room for a MIDI keyboard.
pub const MAX_NOTES: usize = 32;

/// A held note and the velocity it plays at (its latest pressure, if any).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Held {
    pub note: u8,
    pub velocity: u8,
}

/// A small set of notes in the order they were pressed. Fixed size, so it can
/// live in the audio thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteSet {
    items: [Held; MAX_NOTES],
    len: usize,
}

impl Default for NoteSet {
    fn default() -> Self {
        Self::new()
    }
}

impl NoteSet {
    pub const fn new() -> Self {
        Self {
            items: [Held {
                note: 0,
                velocity: 0,
            }; MAX_NOTES],
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The notes in the order they were pressed.
    pub fn as_slice(&self) -> &[Held] {
        &self.items[..self.len]
    }

    pub fn contains(&self, note: u8) -> bool {
        self.as_slice().iter().any(|h| h.note == note)
    }

    pub fn velocity(&self, note: u8) -> Option<u8> {
        self.as_slice()
            .iter()
            .find(|h| h.note == note)
            .map(|h| h.velocity)
    }

    /// Adds a note at the end, or updates its velocity if it is already in.
    /// Returns false only when the set is full.
    pub fn insert(&mut self, note: u8, velocity: u8) -> bool {
        if let Some(h) = self.items[..self.len].iter_mut().find(|h| h.note == note) {
            h.velocity = velocity;
            return true;
        }
        if self.len == MAX_NOTES {
            return false;
        }
        self.items[self.len] = Held { note, velocity };
        self.len += 1;
        true
    }

    /// Removes a note, keeping the order of the rest.
    pub fn remove(&mut self, note: u8) -> bool {
        match self.as_slice().iter().position(|h| h.note == note) {
            Some(i) => {
                self.items.copy_within(i + 1..self.len, i);
                self.len -= 1;
                true
            }
            None => false,
        }
    }

    pub fn set_velocity(&mut self, note: u8, velocity: u8) {
        if let Some(h) = self.items[..self.len].iter_mut().find(|h| h.note == note) {
            h.velocity = velocity;
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Copies the notes, lowest first, into `out`; returns how many.
    pub fn sorted(&self, out: &mut [Held; MAX_NOTES]) -> usize {
        out[..self.len].copy_from_slice(self.as_slice());
        out[..self.len].sort_unstable_by_key(|h| h.note);
        self.len
    }
}

/// How Latch behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LatchMode {
    /// Notes play only while held.
    #[default]
    Off,
    /// Notes stay after release. A press with nothing held starts a new
    /// chord (the old one is dropped); presses while something is held add
    /// to it. This is how Latch usually works on hardware arpeggiators.
    Replace,
    /// Each press adds the note, or takes it out if it is already latched,
    /// so a chord can be built and edited one pad at a time.
    Toggle,
}

/// The pads or keys physically down, and the notes the generators use
/// (the "active" notes, which include latched ones).
#[derive(Clone, Debug, Default)]
pub struct Notes {
    physical: NoteSet,
    active: NoteSet,
    mode: LatchMode,
}

impl Notes {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mode(&self) -> LatchMode {
        self.mode
    }

    pub fn physical(&self) -> &NoteSet {
        &self.physical
    }

    pub fn active(&self) -> &NoteSet {
        &self.active
    }

    pub fn press(&mut self, note: u8, velocity: u8) {
        let nothing_held = self.physical.is_empty();
        self.physical.insert(note, velocity);
        match self.mode {
            LatchMode::Off => {
                self.active.insert(note, velocity);
            }
            LatchMode::Replace => {
                if nothing_held {
                    self.active.clear();
                }
                self.active.insert(note, velocity);
            }
            LatchMode::Toggle => {
                if !self.active.remove(note) {
                    self.active.insert(note, velocity);
                }
            }
        }
    }

    pub fn release(&mut self, note: u8) {
        self.physical.remove(note);
        if self.mode == LatchMode::Off {
            self.active.remove(note);
        }
    }

    /// Pad pressure (aftertouch) changes the velocity a held note repeats at.
    pub fn pressure(&mut self, note: u8, value: u8) {
        if self.physical.contains(note) {
            self.physical.set_velocity(note, value);
            self.active.set_velocity(note, value);
        }
    }

    /// Turning Latch off leaves only what is physically held.
    pub fn set_mode(&mut self, mode: LatchMode) {
        self.mode = mode;
        if mode == LatchMode::Off {
            self.active = self.physical.clone();
        }
    }

    /// Drops the latched notes (keeps what is physically held).
    pub fn clear_latched(&mut self) {
        self.active = self.physical.clone();
    }

    /// Forgets everything, held or not.
    pub fn clear_all(&mut self) {
        self.physical.clear();
        self.active.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes(s: &NoteSet) -> Vec<u8> {
        s.as_slice().iter().map(|h| h.note).collect()
    }

    #[test]
    fn sets_keep_press_order_and_sort_on_request() {
        let mut s = NoteSet::new();
        for n in [7, 2, 9] {
            assert!(s.insert(n, 100));
        }
        assert!(s.insert(2, 50));
        assert_eq!(notes(&s), [7, 2, 9]);
        assert_eq!(s.velocity(2), Some(50));
        let mut out = [Held::default(); MAX_NOTES];
        assert_eq!(s.sorted(&mut out), 3);
        assert_eq!(
            out[..3].iter().map(|h| h.note).collect::<Vec<_>>(),
            [2, 7, 9]
        );
        assert!(s.remove(2) && !s.remove(2));
        assert_eq!(notes(&s), [7, 9]);
        let mut full = NoteSet::new();
        for n in 0..MAX_NOTES as u8 {
            assert!(full.insert(n, 1));
        }
        assert!(!full.insert(200, 1));
    }

    #[test]
    fn without_latch_only_held_notes_are_active() {
        let mut n = Notes::new();
        n.press(1, 100);
        n.press(2, 90);
        n.release(1);
        assert_eq!(notes(n.active()), [2]);
        n.release(2);
        assert!(n.active().is_empty());
    }

    #[test]
    fn latch_keeps_a_chord_until_a_fresh_press_replaces_it() {
        let mut n = Notes::new();
        n.set_mode(LatchMode::Replace);
        n.press(1, 100);
        n.press(5, 100);
        n.release(1);
        n.press(8, 100); // still holding 5: adds
        n.release(5);
        n.release(8);
        assert_eq!(notes(n.active()), [1, 5, 8]);
        n.press(3, 100); // nothing held: a new chord
        n.release(3);
        assert_eq!(notes(n.active()), [3]);
        n.press(4, 100);
        n.set_mode(LatchMode::Off); // only what is held stays
        assert_eq!(notes(n.active()), [4]);
    }

    #[test]
    fn toggle_latch_adds_and_removes_one_pad_at_a_time() {
        let mut n = Notes::new();
        n.set_mode(LatchMode::Toggle);
        for p in [1, 2, 3] {
            n.press(p, 100);
            n.release(p);
        }
        n.press(2, 100);
        n.release(2);
        assert_eq!(notes(n.active()), [1, 3]);
        n.clear_latched();
        assert!(n.active().is_empty());
    }

    #[test]
    fn pressure_changes_the_velocity_of_held_notes_only() {
        let mut n = Notes::new();
        n.set_mode(LatchMode::Replace);
        n.press(1, 100);
        n.pressure(1, 40);
        assert_eq!(n.active().velocity(1), Some(40));
        n.release(1);
        n.pressure(1, 10); // released: ignored
        assert_eq!(n.active().velocity(1), Some(40));
    }
}
