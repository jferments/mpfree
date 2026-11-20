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
//! a track nobody set up plays the same synth (the Synth kit, no copy) and
//! the take goes there (`desk.rs`, [`synth_track`], [`App::rec_synth_on`]).
//! "Nobody set up" is more than empty (the design review of 20 Nov,
//! point B): no clips, a note track's settings and default name as a new
//! one has them ("Track 2" for the second), no
//! instrument of its own, kit 1 or the synth, no MIDI output and no mixer
//! settings ([`unfit`]); never Track 1, the Drums track. The selected track when it is one, else the first
//! one, else a new track at the end of the project; nothing at all when the
//! project is full. A pinned arm is the musician's choice (point A): REC
//! records there or nowhere, and says why. A row tapped in REC's list is a
//! choice too, and it makes the pads play that track, so it ends Play on
//! the pads ([`App::rec_list_ends_pads_preview`]). What is recorded always plays
//! what was heard, and no clip already recorded changes. With the loop
//! armed the pads keep the synth: the loop records what is heard.
//!
//! The arp (point C): Play on the pads plays the synth as pressed, so it
//! never runs while the arp plays the pads (NOTE REPEAT held, Latch, or the
//! Arp panel): it won't start then, and the arp starting ends it
//! ([`App::arp_ends_pads_preview`]). So a take never goes through an arp
//! the try-out didn't. Through the arp needs the synth on a track (pick
//! Synth in its kits) until pad focus (the design's step 3) gives the
//! pads one way to an instrument, recording or not.

use super::{App, Effect};
use mpfree_seq::{Project, Track};

/// Why track `t` can't take the pads' synth for REC without changing
/// something on it, in a few words after its name ("has clips"); None for
/// a track nobody set up. The project says what it holds (`project`), the
/// app the rest (`app_side`: an instrument of its own, a kit or MIDI output
/// picked, mixer settings).
pub fn unfit(
    project: &Project,
    t: usize,
    app_side: impl Fn(usize) -> Option<&'static str>,
) -> Option<&'static str> {
    let Some(k) = project.tracks.get(t) else {
        return Some("isn't in the project");
    };
    if k.is_loops() {
        return Some("is a Loop track");
    }
    if k.slots.iter().any(Option::is_some) {
        return Some("has clips");
    }
    // Renamed, or a setting moved (swing, MIDI effects, transpose...). The
    // name must be this track's own default, "Track 2" for the second: a
    // track renamed "Track 99" is set up (the design review of 20 Nov's
    // second pass). A track after a deleted one keeps its old name and so
    // counts as set up too, the safe way, until tracks get lasting ids
    // (the redesign's step 2). Never allocates.
    let named = k
        .name
        .strip_prefix("Track ")
        .is_some_and(|n| !n.starts_with(['0', '+']) && n.parse::<usize>() == Ok(t + 1));
    if !named || *k != Track::new(&k.name, k.dest) {
        return Some("has settings of its own");
    }
    app_side(t)
}

/// Where REC records the pads' synth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SynthTake {
    /// On this track (0 based), which nobody set up: only its kit changes,
    /// to the Synth kit.
    On(usize),
    /// On a new track at the end of the project: every track is set up.
    New,
    /// Nowhere, and nothing changes: the arm is pinned to `track`, which
    /// `why` says can't take it.
    Pinned { track: usize, why: &'static str },
    /// Nowhere, and nothing changes: every track is set up and the
    /// project holds no more.
    Full,
}

/// Where REC records the pads' synth: the armed track when nobody set it
/// up, else (unless `pinned`) the first track nobody set up, else a new
/// one ([`unfit`] says what counts).
pub fn synth_track(
    project: &Project,
    armed: usize,
    pinned: bool,
    app_side: impl Fn(usize) -> Option<&'static str>,
) -> SynthTake {
    let why = |t: usize| unfit(project, t, &app_side);
    match why(armed) {
        None => return SynthTake::On(armed),
        Some(why) if pinned => return SynthTake::Pinned { track: armed, why },
        Some(_) => {}
    }
    if let Some(t) = (0..project.tracks.len()).find(|&t| why(t).is_none()) {
        SynthTake::On(t)
    } else if project.tracks.len() < mpfree_seq::MAX_TRACKS {
        SynthTake::New
    } else {
        SynthTake::Full
    }
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
        if self.generating() {
            return vec![Effect::Tell(
                "Play on the pads plays as pressed: let go of NOTE REPEAT or Latch, or close the Arp panel, first"
                    .into(),
            )];
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
    /// heard. The caller arms `track` ([`synth_track`]), which it added
    /// for this take when `added`.
    pub fn rec_synth_on(&mut self, track: usize, added: bool) -> Vec<Effect> {
        let name = self.synth_pads.engine.name();
        let new = if added { "a new track, " } else { "" };
        // The project keeps each track's kit: unsaved work.
        self.project_edit = true;
        self.pads_preview = false;
        vec![
            Effect::PadsPreview(false),
            Effect::SynthOnTrack(track),
            Effect::Note(format!(
                "REC records {name} on {new}Track {}: the track plays the same {name} as the pads",
                track + 1
            )),
        ]
    }

    /// Once a period: the arp plays the pads now (NOTE REPEAT, Latch or
    /// the Arp panel), so Play on the pads ends and the pads play the
    /// selected track's instrument through it, as a take would. Nothing
    /// (and no allocation) otherwise.
    pub fn arp_ends_pads_preview(&mut self) -> Vec<Effect> {
        if !self.pads_preview || !self.generating() {
            return Vec::new();
        }
        self.end_pads_preview(|t| format!("The arp plays Track {t}'s kit: Play on the pads is off"))
    }

    /// Track `track`'s row tapped in REC's list: the musician's pick for
    /// REC, and the pads play what that track plays (as picking it does),
    /// so Play on the pads ends. Nothing while it is off.
    pub fn rec_list_ends_pads_preview(&mut self, track: usize) -> Vec<Effect> {
        let n = track + 1;
        self.end_pads_preview(|_| format!("Track {n} picked for REC: the pads play its instrument"))
    }

    /// The tracks with a kit (not the synth) or a MIDI output picked, a bit
    /// each, as the audio loop has them (`TrackKits::picked`,
    /// `MidiOut::routed`): set up, so REC never gives them the pads' synth.
    pub fn set_configured(&mut self, tracks: u128) {
        self.configured = tracks;
    }

    /// Whether track `track` has a kit or a MIDI output picked
    /// ([`App::set_configured`]).
    pub fn configured(&self, track: usize) -> bool {
        track < 128 && self.configured >> track & 1 == 1
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
