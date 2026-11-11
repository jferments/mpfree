//! The play view: the project compiled for the audio thread.
//!
//! The control thread compiles a new view after every edit and swaps it in;
//! the audio thread only reads it. Compiling sorts each clip's notes,
//! applies quantise and speed, and resolves which swing each clip uses, so
//! playback is a binary search and a few multiplications per clip.
//!
//! **Cues** (roadmap T9). A launch made while the music plays waits for
//! the next bar line: the track's clip plays up to it, then the new clip
//! starts there from its first tick. A stop waits the same way. "Next"
//! means the next bar line the playhead plays, so in a loop it can be the
//! loop's start. The player switches on that exact tick and reports it in
//! [`crate::Player::started`]. The control thread then makes the new clip
//! the track's launch with [`Started::apply`] and compiles again. Until
//! that view arrives, the player and the recorder carry the switch
//! themselves, so nothing depends on when it arrives.

use std::ops::Range;

use mpfree_seq::{DestId, ModelError, Note, Project, Quantise, Speed, MAX_TRACKS, SLOTS};
use mpfree_transport::time::{ceil_to, Swing, Tick};

/// A clip playing on a track: which slot, and the song tick where its first
/// pass starts. Before `origin` the clip is silent; from it on, it loops.
/// A cue that starts makes one with an [`anchored`] origin, which plays
/// wherever the song goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Launch {
    pub track: usize,
    pub slot: usize,
    pub origin: Tick,
}

/// A launch waiting for the next bar line the playhead plays: the clip in
/// `slot` starts there in place of the track's clip, or with `None` the
/// track's clip stops there. See the module notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cue {
    pub track: usize,
    pub slot: Option<usize>,
}

/// A cue the player carried out: on song tick `at`, a bar line, `track`
/// switched to clip `slot`, or stopped. `origin` is the new clip's origin,
/// [`anchored`] on `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Started {
    pub track: usize,
    pub slot: Option<usize>,
    pub at: Tick,
    pub origin: Tick,
}

impl Started {
    /// The control thread's part: the started clip becomes the track's
    /// launch (after a stop the track has none), and the cue goes.
    pub fn apply(&self, launches: &mut Vec<Launch>, cues: &mut Vec<Cue>) {
        launches.retain(|l| l.track != self.track);
        if let Some(slot) = self.slot {
            launches.push(Launch { track: self.track, slot, origin: self.origin });
        }
        if let Some(i) = cues.iter().position(|c| c.track == self.track && c.slot == self.slot) {
            cues.remove(i);
        }
    }
}

/// The earliest song tick an [`anchored`] clip plays from: about 12 days
/// before the start at 120 BPM, so in practice wherever the song goes.
pub const EARLIEST: Tick = -(1 << 31);

/// The origin of a clip that starts on song tick `at` and then plays
/// wherever the song goes, back across loop jumps and locates too: `at`
/// moved back by whole passes to [`EARLIEST`] or before. The clip's tick 0
/// still falls on `at`. `num` passes of a clip are exactly `length * den`
/// song ticks, so the origin moves back by multiples of that and stays a
/// whole tick.
pub fn anchored(at: Tick, length: Tick, speed: Speed) -> Tick {
    let period = length.max(1) * Tick::from(speed.den.max(1));
    let back = (at - EARLIEST).max(0);
    at - (back + period - 1) / period * period
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileError {
    Model(ModelError),
    NoTrack(usize),
    EmptySlot { track: usize, slot: usize },
    /// One clip per track plays at a time.
    TwoClips(usize),
    /// One cue per track waits at a time.
    TwoCues(usize),
}

/// A note ready to play: where it starts in the clip (after quantise) and
/// how long it lasts in song ticks (after speed).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NoteView {
    pub local: f64,
    pub len: f64,
    pub note: u8,
    pub velocity: u8,
    /// The clip's note as stored, to name it when replace recording erases it.
    pub src: Note,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ClipView {
    pub slot: usize,
    pub origin: Tick,
    /// One pass, in the clip's own ticks.
    pub length: Tick,
    pub speed: Speed,
    pub quantise: Option<Quantise>,
    /// One pass, in song ticks.
    pub song_len: f64,
    /// The clip's own swing, else the track's; `None` follows the transport.
    pub swing: Option<Swing>,
    /// Sorted by `local`.
    pub notes: Vec<NoteView>,
}

/// A waiting cue, compiled: the clip it starts (its origin comes when it
/// starts), or none for a stop.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CueView {
    pub slot: Option<usize>,
    pub clip: Option<ClipView>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TrackView {
    pub dest: DestId,
    pub mute: bool,
    pub clip: Option<ClipView>,
    pub cue: Option<CueView>,
}

/// The project compiled for playback. Build it on the control thread; the
/// audio thread only reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayView {
    pub(crate) tracks: Vec<TrackView>,
    pub(crate) armed: Option<usize>,
}

impl PlayView {
    /// Nothing plays.
    pub fn empty() -> PlayView {
        PlayView::default()
    }

    /// Compile `project` with the clips in `launches` playing.
    pub fn compile(project: &Project, launches: &[Launch]) -> Result<PlayView, CompileError> {
        PlayView::compile_cued(project, launches, &[])
    }

    /// Compile `project` with the clips in `launches` playing and the
    /// `cues` waiting for the next bar line.
    pub fn compile_cued(project: &Project, launches: &[Launch], cues: &[Cue]) -> Result<PlayView, CompileError> {
        project.check().map_err(CompileError::Model)?;
        let mut tracks: Vec<TrackView> =
            project.tracks.iter().map(|t| TrackView { dest: t.dest, mute: t.mute, clip: None, cue: None }).collect();
        for l in launches {
            let clip = clip_view(project, l.track, l.slot, l.origin)?;
            if tracks[l.track].clip.is_some() {
                return Err(CompileError::TwoClips(l.track));
            }
            tracks[l.track].clip = Some(clip);
        }
        for c in cues {
            let track = tracks.get(c.track).ok_or(CompileError::NoTrack(c.track))?;
            if track.cue.is_some() {
                return Err(CompileError::TwoCues(c.track));
            }
            let clip = c.slot.map(|slot| clip_view(project, c.track, slot, 0)).transpose()?;
            tracks[c.track].cue = Some(CueView { slot: c.slot, clip });
        }
        Ok(PlayView { tracks, armed: None })
    }

    /// Arm `track` for recording, or none. The recorder records into its
    /// playing clip, and replace recording erases from it. Set it before
    /// swapping the view in.
    pub fn set_armed(&mut self, track: Option<usize>) {
        self.armed = track;
    }

    pub fn armed(&self) -> Option<usize> {
        self.armed
    }

    pub fn tracks(&self) -> usize {
        self.tracks.len()
    }

    /// Notes in the playing clips.
    pub fn notes(&self) -> usize {
        self.tracks.iter().filter_map(|t| t.clip.as_ref()).map(|c| c.notes.len()).sum()
    }

    /// The cue waiting on `track`: `Some(Some(slot))` for a launch,
    /// `Some(None)` for a stop.
    pub fn cue(&self, track: usize) -> Option<Option<usize>> {
        self.tracks.get(track)?.cue.as_ref().map(|c| c.slot)
    }
}

/// One clip compiled: notes sorted, quantise and speed applied.
fn clip_view(project: &Project, track: usize, slot: usize, origin: Tick) -> Result<ClipView, CompileError> {
    let t = project.tracks.get(track).ok_or(CompileError::NoTrack(track))?;
    let clip = t
        .slots
        .get(slot)
        .and_then(Option::as_ref)
        .filter(|_| slot < SLOTS)
        .ok_or(CompileError::EmptySlot { track, slot })?;
    let length = clip.length as f64;
    let mut notes: Vec<NoteView> = clip
        .notes
        .iter()
        .map(|n| {
            let at = clip.quantise.map_or(n.tick as f64, |q| q.apply(n.tick as f64));
            NoteView {
                // Quantise can move a note at the very end onto the next pass's start.
                local: at.rem_euclid(length),
                len: clip.speed.to_song(n.len as f64),
                note: n.note,
                velocity: n.velocity,
                src: *n,
            }
        })
        .collect();
    notes.sort_by(|a, b| a.local.total_cmp(&b.local));
    Ok(ClipView {
        slot,
        origin,
        length: clip.length,
        speed: clip.speed,
        quantise: clip.quantise,
        song_len: clip.speed.to_song(length),
        swing: clip.swing.or(t.swing),
        notes,
    })
}

/// A cue carried out on the audio thread that the view does not show yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Switch {
    slot: Option<usize>,
    origin: Tick,
}

/// The cues carried out on each track until the view shows them. The
/// player and the recorder each keep one and step it on the same spans, so
/// they agree on which clip plays, for any period size and whenever the
/// new view arrives.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Switches([Option<Switch>; MAX_TRACKS]);

impl Switches {
    pub fn new() -> Switches {
        Switches([None; MAX_TRACKS])
    }

    /// Forget the switches the view has caught up with: its cue is gone
    /// (the control thread applied it, or took it back).
    pub fn follow(&mut self, view: &PlayView) {
        for (i, s) in self.0.iter_mut().enumerate() {
            if let Some(sw) = *s {
                if view.tracks.get(i).and_then(|t| t.cue.as_ref()).is_none_or(|c| c.slot != sw.slot) {
                    *s = None;
                }
            }
        }
    }

    /// Where the cue on `track` starts in a span playing `ticks`, if it
    /// does: the first bar line among them.
    pub fn due(&self, view: &PlayView, track: usize, ticks: &Range<Tick>, bar: Tick) -> Option<Tick> {
        if self.0.get(track).is_none_or(Option::is_some) {
            return None;
        }
        view.tracks.get(track)?.cue.as_ref()?;
        let at = ceil_to(ticks.start, bar);
        (at < ticks.end).then_some(at)
    }

    /// Carry out the cue on `track` at song tick `at`.
    pub fn start(&mut self, view: &PlayView, track: usize, at: Tick) -> Option<Started> {
        let cue = view.tracks.get(track)?.cue.as_ref()?;
        let origin = cue.clip.as_ref().map_or(at, |c| anchored(at, c.length, c.speed));
        *self.0.get_mut(track)? = Some(Switch { slot: cue.slot, origin });
        Some(Started { track, slot: cue.slot, at, origin })
    }

    /// The clip `track` plays now, and its origin.
    pub fn playing<'a>(&self, view: &'a PlayView, track: usize) -> Option<(&'a ClipView, Tick)> {
        let t = view.tracks.get(track)?;
        match self.0.get(track).copied().flatten() {
            Some(sw) => t.cue.as_ref()?.clip.as_ref().map(|c| (c, sw.origin)),
            None => t.clip.as_ref().map(|c| (c, c.origin)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_seq::{Clip, Note, Quantise, Track};

    fn project() -> Project {
        let mut p = Project::new();
        let mut t = Track::new("drums", 3);
        let mut c = Clip::new("beat", 960);
        c.notes = vec![
            Note { tick: 500, len: 10, note: 1, velocity: 90 },
            Note { tick: 0, len: 10, note: 0, velocity: 100 },
            Note { tick: 950, len: 10, note: 2, velocity: 80 },
        ];
        c.quantise = Quantise::new(240, 100);
        c.speed = Speed::new(2, 1).unwrap();
        t.slots[2] = Some(c);
        p.tracks.push(t);
        p.tracks.push(Track::new("empty", 4));
        p
    }

    #[test]
    fn compiling_sorts_quantises_and_scales() {
        let v = PlayView::compile(&project(), &[Launch { track: 0, slot: 2, origin: 3840 }]).unwrap();
        assert_eq!(v.tracks(), 2);
        assert_eq!(v.notes(), 3);
        let c = v.tracks[0].clip.as_ref().unwrap();
        // 950 quantises to 960, the clip's end, which is the next pass's 0.
        let at: Vec<_> = c.notes.iter().map(|n| (n.local, n.note)).collect();
        assert_eq!(at, vec![(0.0, 0), (0.0, 2), (480.0, 1)]);
        assert_eq!((c.song_len, c.notes[0].len, c.origin), (480.0, 5.0, 3840));
        assert_eq!(v.tracks[0].dest, 3);
        assert!(v.tracks[1].clip.is_none());
    }

    #[test]
    fn launches_must_name_a_clip() {
        let p = project();
        let l = |track, slot| Launch { track, slot, origin: 0 };
        assert_eq!(PlayView::compile(&p, &[l(5, 0)]), Err(CompileError::NoTrack(5)));
        assert_eq!(PlayView::compile(&p, &[l(0, 1)]), Err(CompileError::EmptySlot { track: 0, slot: 1 }));
        assert_eq!(PlayView::compile(&p, &[l(0, 99)]), Err(CompileError::EmptySlot { track: 0, slot: 99 }));
        assert_eq!(PlayView::compile(&p, &[l(0, 2), l(0, 2)]), Err(CompileError::TwoClips(0)));
        assert_eq!(PlayView::compile(&p, &[]).unwrap().notes(), 0);
    }

    #[test]
    fn cues_must_name_a_clip_or_a_stop() {
        let p = project();
        let cue = |track, slot| Cue { track, slot };
        let ok = PlayView::compile_cued(&p, &[], &[cue(0, Some(2)), cue(1, None)]).unwrap();
        assert_eq!((ok.cue(0), ok.cue(1), ok.cue(7)), (Some(Some(2)), Some(None), None));
        assert_eq!(ok.notes(), 0, "a cue does not play yet");
        assert_eq!(PlayView::compile_cued(&p, &[], &[cue(4, None)]), Err(CompileError::NoTrack(4)));
        assert_eq!(PlayView::compile_cued(&p, &[], &[cue(1, Some(2))]), Err(CompileError::EmptySlot { track: 1, slot: 2 }));
        assert_eq!(PlayView::compile_cued(&p, &[], &[cue(0, None), cue(0, Some(2))]), Err(CompileError::TwoCues(0)));
    }

    #[test]
    fn an_anchored_origin_keeps_the_phase_and_lies_before_the_earliest_tick() {
        for (at, length, speed) in [(3840, 7680, Speed::NORMAL), (0, 3840, Speed::NORMAL), (11_520, 1000, Speed::new(3, 2).unwrap()),
                                    (-3840, 960, Speed::new(1, 8).unwrap()), (EARLIEST - 5, 960, Speed::NORMAL)] {
            let o = anchored(at, length, speed);
            assert!(o <= EARLIEST, "{at} {length} {speed:?}");
            let song_len = speed.to_song(length as f64);
            let passes = (at - o) as f64 / song_len;
            assert_eq!(passes, passes.round(), "{at}: whole passes back");
            assert!(at - o < (at - EARLIEST).max(0) + length * Tick::from(speed.den), "{at}: the least whole passes");
        }
    }

    #[test]
    fn a_started_cue_becomes_the_launch() {
        let mut launches = vec![Launch { track: 0, slot: 1, origin: 0 }, Launch { track: 1, slot: 0, origin: 0 }];
        let mut cues = vec![Cue { track: 0, slot: Some(2) }, Cue { track: 1, slot: None }];
        Started { track: 0, slot: Some(2), at: 7680, origin: -100 }.apply(&mut launches, &mut cues);
        assert_eq!(launches, vec![Launch { track: 1, slot: 0, origin: 0 }, Launch { track: 0, slot: 2, origin: -100 }]);
        Started { track: 1, slot: None, at: 7680, origin: 7680 }.apply(&mut launches, &mut cues);
        assert_eq!(launches, vec![Launch { track: 0, slot: 2, origin: -100 }]);
        assert!(cues.is_empty());
    }
}
