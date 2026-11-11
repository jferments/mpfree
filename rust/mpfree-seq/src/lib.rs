//! The time engine's model: a project holds tracks, a track holds clips in
//! 16 slots, a clip holds notes. Plain data, edited only on the control
//! thread and compiled into a read-only play view by `mpfree-play`.
//!
//! Everything is a clip: a step pattern, a recorded phrase and (later) a
//! looper take are the same thing, and the step grid is only a view. A clip
//! has any length, so clips of different lengths drift against each other
//! (polymeter). Recorded ticks stay exact; quantise is a clip setting
//! applied on playback, so it can be changed or undone later.
//!
//! This first version holds notes only. Chance, trig conditions, ratchets,
//! parameter locks, audio clips, scenes and the arrangement come in later
//! roadmap steps without changing what is here.
#![forbid(unsafe_code)]

use mpfree_transport::time::{Swing, Tick, PPQ};

/// Clip slots per track, one per pad.
pub const SLOTS: usize = 16;
/// Tracks in a project, to start (decision 10).
pub const MAX_TRACKS: usize = 16;
/// The shortest clip: a 1/32 note.
pub const MIN_CLIP_TICKS: Tick = (PPQ / 8) as Tick;
/// The longest clip: 256 bars of 4/4.
pub const MAX_CLIP_TICKS: Tick = 256 * 4 * PPQ as Tick;
/// Notes in one clip.
pub const MAX_NOTES: usize = 8192;

/// One note in a clip, in the clip's own ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Note {
    /// Start, from the clip's start: `0 <= tick < clip length`.
    pub tick: Tick,
    /// Length in the clip's ticks, at least 1. It may run past the clip's
    /// end; it is cut if the clip or the transport jumps first.
    pub len: Tick,
    /// Pad (0 to 15) or MIDI note (0 to 127); the destination decides.
    pub note: u8,
    /// 1 to 127.
    pub velocity: u8,
}

/// How fast a clip plays against the song: `num / den` clip ticks per song
/// tick, from 1/8 to 4 (1/8, 1/4, 1/2, 3/4, 1, 3/2, 2, 4 and the rest that
/// these numbers make).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Speed {
    pub num: u8,
    pub den: u8,
}

impl Speed {
    pub const NORMAL: Speed = Speed { num: 1, den: 1 };

    pub fn new(num: u8, den: u8) -> Option<Speed> {
        let ok = |x: u8| matches!(x, 1 | 2 | 3 | 4 | 8);
        let ratio = f64::from(num) / f64::from(den.max(1));
        (ok(num) && ok(den) && (0.125..=4.0).contains(&ratio)).then_some(Speed { num, den })
    }

    /// Song ticks for `ticks` of the clip's own ticks.
    pub fn to_song(self, ticks: f64) -> f64 {
        ticks * f64::from(self.den) / f64::from(self.num)
    }

    /// Clip ticks for `ticks` song ticks.
    pub fn to_clip(self, ticks: f64) -> f64 {
        ticks * f64::from(self.num) / f64::from(self.den)
    }
}

impl Default for Speed {
    fn default() -> Self {
        Speed::NORMAL
    }
}

/// Non-destructive quantise: on playback each note moves `strength`
/// percent of the way to the nearest `grid` line. The notes keep their
/// recorded ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quantise {
    pub grid: u32,
    pub strength: u8,
}

impl Quantise {
    pub fn new(grid: u32, strength: u8) -> Option<Quantise> {
        (grid > 0 && grid <= PPQ * 4 && strength <= 100).then_some(Quantise { grid, strength })
    }

    /// Where a tick plays.
    pub fn apply(self, tick: f64) -> f64 {
        let g = f64::from(self.grid);
        let target = (tick / g).round() * g;
        tick + (target - tick) * f64::from(self.strength) / 100.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub name: String,
    /// Length in the clip's own ticks.
    pub length: Tick,
    pub speed: Speed,
    /// Overrides the track's and the transport's swing.
    pub swing: Option<Swing>,
    pub quantise: Option<Quantise>,
    /// Kept in the order they were written; the play view sorts them.
    pub notes: Vec<Note>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelError {
    ClipLength,
    TooManyNotes,
    /// A note's tick, length, note number or velocity is out of range.
    BadNote(usize),
    BadSpeed,
    BadSwing,
    BadQuantise,
    TooManyTracks,
    BadSlot,
}

impl Clip {
    /// An empty clip of `length` ticks at normal speed.
    pub fn new(name: &str, length: Tick) -> Clip {
        Clip { name: name.to_string(), length, speed: Speed::NORMAL, swing: None, quantise: None, notes: Vec::new() }
    }

    /// Every value in range.
    pub fn check(&self) -> Result<(), ModelError> {
        if !(MIN_CLIP_TICKS..=MAX_CLIP_TICKS).contains(&self.length) {
            return Err(ModelError::ClipLength);
        }
        if self.notes.len() > MAX_NOTES {
            return Err(ModelError::TooManyNotes);
        }
        if Speed::new(self.speed.num, self.speed.den).is_none() {
            return Err(ModelError::BadSpeed);
        }
        if self.swing.is_some_and(|s| Swing::new(s.grid, s.amount).is_none()) {
            return Err(ModelError::BadSwing);
        }
        if self.quantise.is_some_and(|q| Quantise::new(q.grid, q.strength).is_none()) {
            return Err(ModelError::BadQuantise);
        }
        for (i, n) in self.notes.iter().enumerate() {
            let ok = (0..self.length).contains(&n.tick)
                && (1..=MAX_CLIP_TICKS).contains(&n.len)
                && n.note <= 127
                && (1..=127).contains(&n.velocity);
            if !ok {
                return Err(ModelError::BadNote(i));
            }
        }
        Ok(())
    }
}

/// Where a track's notes go: a sampler program, the arp in front of it,
/// MIDI out ... The host owns the list; a track holds an index into it.
pub type DestId = u16;

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub name: String,
    pub dest: DestId,
    /// Overrides the transport's swing for every clip on the track that has
    /// none of its own.
    pub swing: Option<Swing>,
    pub mute: bool,
    pub slots: [Option<Clip>; SLOTS],
}

impl Track {
    pub fn new(name: &str, dest: DestId) -> Track {
        Track { name: name.to_string(), dest, swing: None, mute: false, slots: Default::default() }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Project {
    pub tracks: Vec<Track>,
}

impl Project {
    pub fn new() -> Project {
        Project::default()
    }

    /// Every track and clip in range.
    pub fn check(&self) -> Result<(), ModelError> {
        if self.tracks.len() > MAX_TRACKS {
            return Err(ModelError::TooManyTracks);
        }
        for t in &self.tracks {
            if t.swing.is_some_and(|s| Swing::new(s.grid, s.amount).is_none()) {
                return Err(ModelError::BadSwing);
            }
            for c in t.slots.iter().flatten() {
                c.check()?;
            }
        }
        Ok(())
    }

    /// The clip in a slot, if there is one.
    pub fn clip(&self, track: usize, slot: usize) -> Option<&Clip> {
        self.tracks.get(track)?.slots.get(slot)?.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_check_their_values() {
        let mut c = Clip::new("beat", 3840);
        c.notes.push(Note { tick: 0, len: 240, note: 0, velocity: 100 });
        assert_eq!(c.check(), Ok(()));
        c.notes.push(Note { tick: 3840, len: 240, note: 0, velocity: 100 });
        assert_eq!(c.check(), Err(ModelError::BadNote(1)));
        c.notes[1].tick = 100;
        c.notes[1].velocity = 0;
        assert_eq!(c.check(), Err(ModelError::BadNote(1)));
        c.notes.pop();
        c.length = 100;
        assert_eq!(c.check(), Err(ModelError::ClipLength));
        c.length = 3840;
        c.speed = Speed { num: 3, den: 8 };
        assert_eq!(c.check(), Ok(()));
        c.speed = Speed { num: 8, den: 1 };
        assert_eq!(c.check(), Err(ModelError::BadSpeed));
        assert!(Speed::new(1, 8).is_some() && Speed::new(4, 1).is_some() && Speed::new(5, 4).is_none());
    }

    #[test]
    fn speed_and_quantise_map_ticks() {
        let s = Speed::new(2, 1).unwrap();
        assert_eq!(s.to_song(960.0), 480.0);
        assert_eq!(s.to_clip(480.0), 960.0);
        assert_eq!(Speed::new(3, 4).unwrap().to_song(3.0), 4.0);
        let q = Quantise::new(240, 50).unwrap();
        assert_eq!(q.apply(250.0), 245.0);
        assert_eq!(q.apply(110.0), 55.0);
        assert_eq!(Quantise::new(240, 100).unwrap().apply(130.0), 240.0);
        assert!(Quantise::new(0, 50).is_none() && Quantise::new(240, 101).is_none());
    }

    #[test]
    fn projects_check_every_clip() {
        let mut p = Project::new();
        let mut t = Track::new("drums", 0);
        t.slots[3] = Some(Clip::new("bad", 10));
        p.tracks.push(t);
        assert_eq!(p.check(), Err(ModelError::ClipLength));
        p.tracks[0].slots[3] = Some(Clip::new("ok", 960));
        assert_eq!(p.check(), Ok(()));
        assert_eq!(p.clip(0, 3).map(|c| c.length), Some(960));
        assert!(p.clip(0, 4).is_none() && p.clip(1, 0).is_none() && p.clip(0, 99).is_none());
    }
}
