//! Audio clips (the audio tracks rethink's phase 2, step 2a): a clip whose
//! content is a short list of regions of recordings instead of notes, in
//! a cell of an audio track (docs/dev/design/audio-tracks.md, "Phase 2").
//!
//! - **A clip of regions.** [`AudioClip`] holds up to [`MAX_CLIP_REGIONS`]
//!   [`ClipRegion`]s: clip ticks `start..end` play recording `rec` from
//!   frame `src`, at the rate it was recorded at. They are the song lane's
//!   regions (`mpfree_audiotracks::Region`) on the clip's own ticks, naming
//!   a recording by its lasting id ([`RecId`]) rather than the engine's
//!   take number. A clip names its recordings; it never holds audio, so a
//!   copy names the same ones.
//! - **Fades kept through a cut.** A region cut out of a longer one (Song
//!   to scenes at a section's edge) keeps exactly its share of that
//!   region's fades: `into` and `left` are the frames of the region it was
//!   cut from before and after it (0 for one never cut), and its fades are
//!   measured on that whole region.
//! - **Once.** A once clip stops at its length like any clip, and that
//!   stop rings: what sounds plays on, the last region to
//!   [`AudioClip::last_frame`], the frame it was made to end on. So a
//!   backing track ends on its own last frame at any tempo.
//! - **One place for the rules.** [`AudioClip::check`] holds the clip's
//!   own; `Clip::check` adds that an audio clip holds nothing a note clip
//!   holds, and `Project::check` that audio clips live on audio tracks and
//!   only there. 16 regions is a budget for the MPC, not part of the
//!   format: the save writes a line a region.
//!
//! - **Loop cells** (phase 4, step 4b,
//!   docs/dev/design/loops-on-audio-tracks.md): a cell of an audio track
//!   can hold a loop instead, [`LoopClip`], which names the looper's loop
//!   by its lasting id ([`LoopId`]); the loop's audio, layers and edits are
//!   the looper's. `Clip::check` holds that a clip is a take clip or a loop
//!   cell, never both, and `Project::check` that a loop belongs to one
//!   track (`Project::loop_track`).
//!
//! Whether each recording named is in the project's recordings table, and
//! has the frames named, is the audio tracks part's to say, which is not
//! this crate's: the app checks it as a project opens (`mpfree-app`
//! `project::audio_clips`), and keeps the recordings while a clip, or a
//! clip an undo step holds, names them ([`AudioClip::recordings`],
//! `Project::recordings`, `mpfree_play::Takes::recordings`).

use mpfree_transport::time::{valid_bpm, Tick};

use crate::ModelError;

/// The most regions an audio clip holds: a budget for the MPC's audio
/// thread, not part of the saved format.
pub const MAX_CLIP_REGIONS: usize = 16;

/// The highest gain a region or a clip takes (+12 dB), as the song lane's
/// regions.
pub const MAX_GAIN: f32 = 4.0;

/// A recording's lasting id, as `mpfree_audiotracks::RecordingId` gives
/// it: what an audio clip names, so it survives saves, opens and the
/// engine's take numbers changing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecId(pub u128);

/// A loop's lasting id (phase 4): made once, when its first pass is
/// armed, by the app's loop host from the audio tracks' id maker, and never
/// given twice; [`LoopId::NONE`] (0) is none. What a loop cell names
/// ([`LoopClip`]), so it survives saves, opens and the looper's slots
/// changing: a slot is where the looper holds a loop, never its identity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LoopId(pub u128);

impl LoopId {
    /// No id given.
    pub const NONE: LoopId = LoopId(0);

    /// Whether it is a given id (not [`LoopId::NONE`]).
    pub fn is_given(self) -> bool {
        self != LoopId::NONE
    }
}

/// A loop cell on an audio track (phase 4, `Clip::loop_clip`): the loop it
/// plays, by its lasting id. The clip holds nothing else, and is as long as
/// its loop (`loops::length`); the loop's layers, its level and its history
/// are the looper's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopClip {
    pub id: LoopId,
}

impl LoopClip {
    /// It names a loop: `BadLoop` for [`LoopId::NONE`].
    pub fn check(&self) -> Result<(), ModelError> {
        if self.id.is_given() {
            Ok(())
        } else {
            Err(ModelError::BadLoop)
        }
    }
}

/// One region of an audio clip: clip ticks `start..end` play recording
/// `rec` from frame `src` (which may be below 0: those frames are silent).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipRegion {
    pub start: Tick,
    pub end: Tick,
    pub rec: RecId,
    pub src: i64,
    /// Where in frame `src` its recording is at `start`, -0.5 to 0.5, as
    /// on the song region it was cut from: the frame playing at clip tick
    /// `t` is `src` plus this and the frames from `start` to `t`, rounded
    /// (`mpfree_audiotracks::Region::phase`).
    pub phase: f32,
    pub gain: f32,
    /// Its fade in and fade out in frames, measured on the region it was
    /// cut from (see `into` and `left`).
    pub fade_in: u32,
    pub fade_out: u32,
    /// Frames of the region it was cut from before its start, and after
    /// its end: 0 for a region never cut.
    pub into: u32,
    pub left: u32,
}

impl ClipRegion {
    /// A region of `rec` from frame `src`, on clip ticks `start..end`, at
    /// full gain with no fades.
    pub fn new(start: Tick, end: Tick, rec: RecId, src: i64) -> ClipRegion {
        ClipRegion {
            start,
            end,
            rec,
            src,
            phase: 0.0,
            gain: 1.0,
            fade_in: 0,
            fade_out: 0,
            into: 0,
            left: 0,
        }
    }
}

/// An audio clip's content ([`crate::Clip::audio`]).
#[derive(Clone, Debug, PartialEq)]
pub struct AudioClip {
    /// In order, none overlapping, all inside the clip's length.
    pub regions: Vec<ClipRegion>,
    /// The tempo it was made at (BPM): what the card shows, and what Fit
    /// to tempo fits from.
    pub bpm: f64,
    /// The clip's own gain, on top of each region's (the card's Gain row).
    pub gain: f32,
    /// The frame of the last region's recording its sound ends on: where
    /// a once clip's ring stops.
    pub last_frame: i64,
}

impl AudioClip {
    /// A clip of one region: recording `rec` from frame `src` to frame
    /// `last_frame`, on clip ticks `0..length`, made at `bpm`.
    pub fn of(rec: RecId, src: i64, last_frame: i64, length: Tick, bpm: f64) -> AudioClip {
        AudioClip {
            regions: vec![ClipRegion::new(0, length, rec, src)],
            bpm,
            gain: 1.0,
            last_frame,
        }
    }

    /// Every value in range for a clip `length` ticks long: 1 to
    /// [`MAX_CLIP_REGIONS`] regions, in order, none overlapping, each
    /// inside `0..length` with ticks to play; gains 0 to [`MAX_GAIN`]; a
    /// tempo MPFree plays; and `last_frame` past the last region's first
    /// frame. `BadAudioClip` otherwise.
    pub fn check(&self, length: Tick) -> Result<(), ModelError> {
        let gain_ok = |g: f32| g.is_finite() && (0.0..=MAX_GAIN).contains(&g);
        let ok = !self.regions.is_empty()
            && self.regions.len() <= MAX_CLIP_REGIONS
            && valid_bpm(self.bpm)
            && gain_ok(self.gain)
            && self
                .regions
                .iter()
                .all(|r| 0 <= r.start && r.start < r.end && r.end <= length && gain_ok(r.gain))
            && self.regions.windows(2).all(|w| w[0].end <= w[1].start)
            && self.regions.last().is_some_and(|r| self.last_frame > r.src);
        if ok {
            Ok(())
        } else {
            Err(ModelError::BadAudioClip)
        }
    }

    /// The recordings it names, in region order (one may repeat).
    pub fn recordings(&self) -> impl Iterator<Item = RecId> + '_ {
        self.regions.iter().map(|r| r.rec)
    }

    /// Its frame counts, made at `from` frames a second, as at `to`: for
    /// recordings converted to the rate MPFree runs at as a project opens
    /// (`mpfree_audiotracks::save::Saved::convert`, which scales the song
    /// lane's regions the same way). Its ticks stay.
    pub fn convert(&mut self, from: u32, to: u32) {
        if from == to || from == 0 || to == 0 {
            return;
        }
        let scale = |n: f64| (n * f64::from(to) / f64::from(from)).round();
        let frames = |n: u32| scale(f64::from(n)) as u32;
        for r in &mut self.regions {
            let x = f64::from(to) / f64::from(from) * (r.src as f64 + f64::from(r.phase));
            (r.src, r.phase) = (x.round() as i64, (x - x.round()) as f32);
            (r.fade_in, r.fade_out) = (frames(r.fade_in), frames(r.fade_out));
            (r.into, r.left) = (frames(r.into), frames(r.left));
        }
        self.last_frame = scale(self.last_frame as f64) as i64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: Tick = 4 * mpfree_transport::time::PPQ as Tick;

    fn two_regions() -> AudioClip {
        let mut a = AudioClip::of(RecId(7), 0, 88_200, 2 * BAR, 120.0);
        a.regions[0].end = BAR;
        a.regions
            .push(ClipRegion::new(BAR, 2 * BAR, RecId(9), 44_100));
        a.last_frame = 88_200;
        a
    }

    #[test]
    fn a_sound_clip_passes_and_each_bad_value_is_refused() {
        let a = two_regions();
        assert_eq!(a.check(2 * BAR), Ok(()));
        assert_eq!(a.recordings().collect::<Vec<_>>(), [RecId(7), RecId(9)]);
        let bad = |f: &dyn Fn(&mut AudioClip)| {
            let mut b = two_regions();
            f(&mut b);
            b.check(2 * BAR)
        };
        let refused = Err(ModelError::BadAudioClip);
        assert_eq!(bad(&|a| a.regions.clear()), refused, "no regions");
        assert_eq!(
            bad(&|a| a.regions = vec![ClipRegion::new(0, 10, RecId(1), 0); 17]),
            refused,
            "17 regions"
        );
        assert_eq!(bad(&|a| a.regions.swap(0, 1)), refused, "out of order");
        assert_eq!(bad(&|a| a.regions[1].start = BAR - 1), refused, "overlap");
        assert_eq!(
            bad(&|a| a.regions[1].end = 2 * BAR + 1),
            refused,
            "past the end"
        );
        assert_eq!(bad(&|a| a.regions[0].start = -1), refused, "before 0");
        assert_eq!(bad(&|a| a.regions[0].end = 0), refused, "no ticks");
        assert_eq!(bad(&|a| a.regions[0].gain = f32::NAN), refused, "gain");
        assert_eq!(bad(&|a| a.gain = MAX_GAIN * 2.0), refused, "clip gain");
        assert_eq!(bad(&|a| a.bpm = 0.0), refused, "tempo");
        assert_eq!(bad(&|a| a.last_frame = 44_100), refused, "last frame");
        // 16 regions fit.
        let mut full = two_regions();
        full.regions = (0..16)
            .map(|i| ClipRegion::new(i * 10, i * 10 + 10, RecId(1), 0))
            .collect();
        assert_eq!(full.check(2 * BAR), Ok(()));
    }

    #[test]
    fn its_frames_convert_to_another_rate_and_its_ticks_stay() {
        let mut a = two_regions();
        a.regions[1].fade_in = 441;
        a.regions[1].into = 4410;
        a.convert(44_100, 48_000);
        let r = a.regions[1];
        assert_eq!((r.start, r.end), (BAR, 2 * BAR));
        assert_eq!(
            (r.src, r.fade_in, r.into, a.last_frame),
            (48_000, 480, 4800, 96_000)
        );
        let again = a.clone();
        a.convert(48_000, 48_000);
        assert_eq!(a, again);
    }
}
