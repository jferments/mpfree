//! The play view: the project compiled for the audio thread.
//!
//! The control thread compiles a new view after every edit and swaps it in;
//! the audio thread only reads it. Compiling sorts each clip's notes,
//! applies quantise and speed, and resolves which swing each clip uses, so
//! playback is a binary search and a few multiplications per clip.

use mpfree_seq::{DestId, ModelError, Note, Project, Quantise, Speed, SLOTS};
use mpfree_transport::time::{Swing, Tick};

/// A clip playing on a track: which slot, and the song tick where its first
/// pass starts. Before `origin` the clip is silent; from it on, it loops.
/// Launching on a grid (roadmap T9) produces these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Launch {
    pub track: usize,
    pub slot: usize,
    pub origin: Tick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileError {
    Model(ModelError),
    NoTrack(usize),
    EmptySlot { track: usize, slot: usize },
    /// One clip per track plays at a time.
    TwoClips(usize),
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TrackView {
    pub dest: DestId,
    pub mute: bool,
    pub clip: Option<ClipView>,
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
        project.check().map_err(CompileError::Model)?;
        let mut tracks: Vec<TrackView> =
            project.tracks.iter().map(|t| TrackView { dest: t.dest, mute: t.mute, clip: None }).collect();
        for l in launches {
            let track = project.tracks.get(l.track).ok_or(CompileError::NoTrack(l.track))?;
            let clip = track
                .slots
                .get(l.slot)
                .and_then(Option::as_ref)
                .filter(|_| l.slot < SLOTS)
                .ok_or(CompileError::EmptySlot { track: l.track, slot: l.slot })?;
            if tracks[l.track].clip.is_some() {
                return Err(CompileError::TwoClips(l.track));
            }
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
            tracks[l.track].clip = Some(ClipView {
                slot: l.slot,
                origin: l.origin,
                length: clip.length,
                speed: clip.speed,
                quantise: clip.quantise,
                song_len: clip.speed.to_song(length),
                swing: clip.swing.or(track.swing),
                notes,
            });
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
}
