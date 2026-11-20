//! The synth on the pads while the selected track keeps its instrument
//! (step 1 of the instrument redesign, Jay's pick of 20 Nov 2026, "Gear and
//! Back now, then redesign"; the design doc "Instruments, pads and tracks:
//! the redesign").
//!
//! On the Synth tab, while the selected track's instrument is a kit, the
//! button under the instrument card puts the synth the tab shows (the
//! shared synth, on its own Synth channel of the mixer) on the pads. The
//! track keeps its kit, its clips keep playing it, and nothing records
//! the pads: the audio loop plays them on the synth before the arp and the
//! recorder see them (`main.rs`, `preview_on`). The button again or
//! another track selected give the pads back to the track.
//!
//! REC or OVERDUB on the notes record the synth where it plays (the
//! design's rule 3, step 1's way, with today's per-track instruments):
//! the selected track while it has no clips, or else the first track with
//! none, plays the same synth (the Synth kit, no copy) and the take goes
//! there (`desk.rs`, [`synth_track`], [`App::rec_synth_on`]). What is
//! recorded always plays what was heard, and no clip already recorded
//! changes. With the loop armed the pads keep the synth: the loop records
//! what is heard.

use super::{App, Effect};
use mpfree_seq::Project;

/// The track REC records the pads' synth into: `armed` while it has no
/// clips (and so nothing a new instrument would change), else the first
/// note track with no clips and no instrument of its own. None while every
/// track has clips: nothing is changed then.
pub fn synth_track(project: &Project, armed: usize, own: impl Fn(usize) -> bool) -> Option<usize> {
    let empty = |t: usize| {
        project
            .tracks
            .get(t)
            .is_some_and(|k| !k.is_loops() && k.slots.iter().all(Option::is_none))
            && !own(t)
    };
    if empty(armed) {
        return Some(armed);
    }
    (0..project.tracks.len()).find(|&t| empty(t))
}

impl App {
    /// Whether the pads play the synth while the selected track keeps its
    /// kit.
    pub fn pads_preview(&self) -> bool {
        self.pads_preview
    }

    /// The Play on the pads button: the pads play the synth the Synth tab
    /// shows, or, while they do, the selected track's kit again. Nothing
    /// while the pads already play a synth: the track's own, or the synth
    /// kit.
    pub fn toggle_pads_preview(&mut self) -> Vec<Effect> {
        if self.pads_preview {
            return self.end_pads_preview(|t| format!("The pads play Track {t}'s kit again"));
        }
        if self.pad_layout.is_some() || self.track_sound(self.track).is_some() {
            return Vec::new();
        }
        // A pad held on the kit now still lets go there: the audio loop
        // plays on the synth only the hits that start from here.
        self.pads_preview = true;
        vec![
            Effect::PadsPreview(true),
            Effect::Note(format!(
                "The pads play {} · Track {} keeps its kit · REC records it on an empty track",
                self.synth_pads.engine.name(),
                self.track + 1
            )),
        ]
    }

    /// The name of the synth Play on the pads plays ("Poly").
    pub fn pads_synth_name(&self) -> &'static str {
        self.synth_pads.engine.name()
    }

    /// REC while the pads play the synth: track `track` (0 based) plays
    /// that same synth from now on, as picking Synth in the list of kits
    /// does ([`Effect::SynthOnTrack`]): one synth for the pads and the
    /// track, no copy, so the take plays the sound that was shaped and
    /// heard. The caller arms `track` ([`synth_track`]).
    pub fn rec_synth_on(&mut self, track: usize) -> Vec<Effect> {
        let name = self.synth_pads.engine.name();
        // The project keeps each track's kit: unsaved work.
        self.project_edit = true;
        self.pads_preview = false;
        vec![
            Effect::PadsPreview(false),
            Effect::SynthOnTrack(track),
            Effect::Note(format!(
                "REC records {name} on Track {}: the track plays the same {name} as the pads",
                track + 1
            )),
        ]
    }

    /// The pads go back to the selected track's instrument, saying what
    /// `why` writes. Nothing (and no allocation) when they play it already.
    pub(super) fn end_pads_preview(&mut self, why: impl FnOnce(usize) -> String) -> Vec<Effect> {
        if !self.pads_preview {
            return Vec::new();
        }
        self.pads_preview = false;
        vec![
            Effect::PadsPreview(false),
            Effect::Note(why(self.track + 1)),
        ]
    }
}
