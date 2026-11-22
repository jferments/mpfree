//! What the pads play: the selected track's instrument, or one chosen for
//! them (the instrument redesign's step 3, pad focus; the design doc
//! "Instruments, pads and tracks: the redesign", rules 1 and 2, Jay's yes
//! of 20 Nov 2026). Step 1 (Play on the pads, #722) is where it started.
//!
//! **The pads follow the selected track until you choose for them**
//! ([`PadFocus::Track`]): selecting a track puts its instrument on the
//! pads, as on the MPC. On the Synth tab, while the selected track's
//! instrument is a kit, the button under the instrument card chooses the
//! synth the tab shows (the shared synth, instrument 1, on its own Synth
//! channel of the mixer) for the pads ([`PadFocus::Instrument`]), and it
//! stays there whichever track is selected, until **Follow track** (the
//! button again, or the "Pads: Poly" chip in the header). The tracks keep
//! what they play and their clips play it. The pads' focus is saved with
//! the project (the `pads` part, [`App::pads_part`]); a new project, or one
//! saved before, follows the track.
//!
//! **What never moves the pads** (rule 1): arming or selecting a track, a
//! row tapped in REC's list, or REC. What does: Follow track; a kit picked
//! for the selected track in the list of kits (a choice of what the pads
//! play, as on the MPC: the pads follow the track and play it); a project
//! opened (its own focus); and the arp (below).
//!
//! **How the audio loop plays them** (`main.rs`, `preview_on`): while the
//! selected track plays the shared synth itself, the pads play it the
//! usual way, through the arp and the recorder, as the track's; while it
//! plays anything else, the loop plays the pad hits on the synth before the
//! arp and the recorder see them, so nothing records them there.
//!
//! **REC** (rule 3, step 1's way with today's per-track instruments): while
//! the pads have the synth and the armed track plays something else, REC
//! or OVERDUB on the notes record the synth where it plays: a track nobody
//! set up plays the same synth (the Synth kit, no copy) and the take goes
//! there (`desk.rs`, [`synth_track`], [`App::rec_synth_on`]). The pads keep
//! the synth (rule 1); the track now plays it too, so they play it through
//! the track, and a second REC records there again. "Nobody set up" is
//! more than empty (the design review of 20 Nov, point B): no clips, a note
//! track's settings and default name as a new one has them ("Track 2" for
//! the second), no instrument of its own, kit 1 or the synth, no MIDI
//! output and no mixer settings ([`unfit`]); never Track 1, the Drums
//! track. The selected track when it is one, else the first one, else a
//! new track at the end of the project; nothing at all when the project is
//! full. A pinned arm (or a row tapped in REC's list) is the musician's
//! choice (point A): REC records there or nowhere, and says why. What is
//! recorded always plays what was heard, and no clip already recorded
//! changes. With the loop armed the pads keep the synth: the loop records
//! what is heard. Where REC will record is the recording plan
//! ([`RecPlan`], `desk::rec_plan`, step 5b): the REC chip names it before
//! you press ("REC → Track 2 · Poly"), and REC acts on the same plan.
//!
//! **The arp** (point C): the arp plays through the selected track, so
//! while the pads have the synth over a track that plays something else,
//! it can't play them: the button won't choose the synth while the arp
//! plays the pads (NOTE REPEAT held, Latch, or the Arp panel), and the arp
//! starting there gives the pads back to the track
//! ([`App::arp_ends_pads_synth`]). So a take never goes through an arp the
//! try-out didn't. Over a track that plays the synth, the arp plays it as
//! the track's. The arp by instrument comes with a later step.

use super::{App, Effect};
use crate::instrument_list::InstrumentId;
use mpfree_seq::{Project, Track};

/// What the pads play: the selected track's instrument, or an instrument
/// chosen for them that stays whichever track is selected. Only the
/// shared synth can be chosen yet (the Synth tab's button).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PadFocus {
    /// The selected track's instrument (a new project's): "Follow track".
    #[default]
    Track,
    /// This instrument of the project's list ("Pads: Poly").
    Instrument(InstrumentId),
}

/// The pads part's first line ([`crate::project::PADS`]).
pub const PADS_FORMAT: &str = "mpfree-pads 1";

impl PadFocus {
    /// The shared synth, chosen for the pads.
    pub const SYNTH: PadFocus = PadFocus::Instrument(InstrumentId::SHARED_SYNTH);

    /// The pads part: its first line, then `play track` or `play
    /// instrument 1`.
    pub fn text(self) -> String {
        match self {
            PadFocus::Track => format!("{PADS_FORMAT}\nplay track\n"),
            PadFocus::Instrument(id) => format!("{PADS_FORMAT}\nplay instrument {id}\n"),
        }
    }

    /// The pads part read back ([`PadFocus::text`]); Err says why it
    /// doesn't read: another first line, a line it doesn't know, or an
    /// instrument that can't be chosen for the pads (only the shared synth
    /// can, yet).
    pub fn parse(text: &str) -> Result<PadFocus, String> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        match lines.next() {
            Some(PADS_FORMAT) => {}
            Some(other) => return Err(format!("its first line is \"{other}\"")),
            None => return Err("it is empty".to_string()),
        }
        let mut focus = None;
        for line in lines {
            let words: Vec<&str> = line.split_whitespace().collect();
            let f = match words.as_slice() {
                ["play", "track"] => PadFocus::Track,
                ["play", "instrument", n] => {
                    let id = n
                        .parse()
                        .ok()
                        .and_then(InstrumentId::from_number)
                        .ok_or_else(|| format!("\"{line}\" doesn't read"))?;
                    if id != InstrumentId::SHARED_SYNTH {
                        return Err(format!(
                            "instrument {id} can't be chosen for the pads yet (only the synth, instrument 1)"
                        ));
                    }
                    PadFocus::Instrument(id)
                }
                _ => return Err(format!("\"{line}\" doesn't read")),
            };
            if focus.replace(f).is_some() {
                return Err("it says twice what the pads play".to_string());
            }
        }
        focus.ok_or_else(|| "it doesn't say what the pads play".to_string())
    }
}

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
    if !k.slots.is_empty() {
        return Some("has clips");
    }
    // Renamed, or a setting moved (swing, MIDI effects, transpose...). The
    // name must be this track's own default, "Track 2" for the second: a
    // track renamed "Track 99" is set up (the design review of 20 Nov's
    // second pass). A track after a deleted one keeps its old name and so
    // counts as set up too, the safe way: tracks can't be deleted from
    // the middle yet, and when they can, the track's lasting id (step 2c)
    // is the way to tell. Never allocates (an empty name allocates
    // nothing).
    let named = k
        .name
        .strip_prefix("Track ")
        .is_some_and(|n| !n.starts_with(['0', '+']) && n.parse::<usize>() == Ok(t + 1));
    if !named || !k.same_settings(&Track::new("", k.dest)) {
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

/// The recording plan (the redesign's step 5b): where REC will record the
/// pads' synth, worked out each time something changes so the REC chip
/// names it before you press, and by REC itself, so the chip and REC
/// can't disagree (`desk.rs`, `rec_plan`). None of it changes anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecPlan {
    pub take: SynthTake,
    /// The pads' synth's name ("Poly").
    pub synth: &'static str,
}

impl RecPlan {
    /// The REC chip's words after the arrow: "Track 2 · Poly", "New track
    /// · Poly", "Track 1 · not free" (pinned to a track that can't take
    /// it), "No free track · Poly". `name` gives a track's name ("Bass"
    /// for one you named).
    pub fn chip(self, name: impl Fn(usize) -> String) -> String {
        match self.take {
            SynthTake::On(t) => format!("{} · {}", name(t), self.synth),
            SynthTake::New => format!("New track · {}", self.synth),
            SynthTake::Pinned { track, .. } => format!("{} · not free", name(track)),
            SynthTake::Full => format!("No free track · {}", self.synth),
        }
    }
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
    /// What the pads play: the selected track's instrument, or one chosen
    /// for them.
    pub fn pads_focus(&self) -> PadFocus {
        self.pads_focus
    }

    /// Whether the shared synth is chosen for the pads, whichever track is
    /// selected (the Synth tab's button, "Pads: Poly").
    pub fn pads_chose_synth(&self) -> bool {
        self.pads_focus() == PadFocus::SYNTH
    }

    /// The Synth tab's button under the instrument card: the pads play the
    /// synth the tab shows whichever track is selected, or, while they do,
    /// follow the selected track again. Nothing while the pads follow a
    /// track that plays a synth already: its own, or the shared one.
    pub fn toggle_pads_synth(&mut self) -> Vec<Effect> {
        if self.pads_chose_synth() {
            return self.pads_follow_track();
        }
        if self.pad_layout.is_some() || self.track_plays_instrument(self.track) {
            return Vec::new();
        }
        if self.generating() {
            return vec![Effect::Tell(
                "The pads play the synth as pressed: let go of NOTE REPEAT or Latch, or close the Arp panel, first"
                    .into(),
            )];
        }
        // A pad held on the kit now still lets go there: the audio loop
        // plays on the synth only the hits that start from here.
        self.pads_focus = PadFocus::SYNTH;
        self.project_edit = true;
        vec![
            Effect::PadsFocus(self.pads_focus),
            Effect::Note(format!(
                "The pads play {} on every track · Track {} keeps its kit · REC records it on an empty track",
                self.synth_pads.engine.name(),
                self.track + 1
            )),
        ]
    }

    /// Follow track (the Synth tab's button again, or the "Pads: Poly"
    /// chip): the pads play the selected track's instrument again.
    pub fn pads_follow_track(&mut self) -> Vec<Effect> {
        self.pads_back(|t| format!("The pads follow the track: Track {t}'s instrument"))
    }

    /// A kit picked for the selected track in the list of kits (or turned
    /// with SHIFT + the wheel): a choice of what the pads play, so they
    /// follow the track again and play it, as on the MPC. Nothing (and no
    /// allocation) while they follow it already.
    pub fn kit_pick_follows_track(&mut self) -> Vec<Effect> {
        self.pads_back(|t| {
            format!("Kit picked: the pads follow the track and play Track {t}'s kit")
        })
    }

    /// The name of the synth the pads can play ("Poly").
    pub fn pads_synth_name(&self) -> &'static str {
        self.synth_pads.engine.name()
    }

    /// REC while the pads have the synth: track `track` (0 based) plays
    /// that same synth from now on, as picking Synth in the list of kits
    /// does ([`Effect::SynthOnTrack`]): one synth for the pads and the
    /// track, no copy, so the take plays the sound that was shaped and
    /// heard. The pads keep the synth (rule 1). The caller arms `track`
    /// ([`synth_track`]), which it added for this take when `added`.
    pub fn rec_synth_on(&mut self, track: usize, added: bool) -> Vec<Effect> {
        let name = self.synth_pads.engine.name();
        let new = if added { "a new track, " } else { "" };
        // The project keeps each track's kit: unsaved work.
        self.project_edit = true;
        if track < 128 {
            // Before the audio loop says so next period: the press that
            // follows records there.
            self.synth_tracks |= 1 << track;
        }
        vec![
            Effect::SynthOnTrack(track),
            Effect::Note(format!(
                "REC records {name} on {new}Track {}: the track plays the same {name} as the pads",
                track + 1
            )),
        ]
    }

    /// Once a period: the arp plays the pads now (NOTE REPEAT, Latch or
    /// the Arp panel) while they have the synth over a track that plays
    /// something else, so the pads follow the track again and play its
    /// instrument through the arp, as a take would. Nothing (and no
    /// allocation) otherwise: over a track that plays the synth the arp
    /// plays it.
    pub fn arp_ends_pads_synth(&mut self) -> Vec<Effect> {
        if !self.pads_chose_synth() || self.pad_layout.is_some() || !self.generating() {
            return Vec::new();
        }
        self.pads_back(|t| {
            format!("The arp plays Track {t}'s instrument: the pads follow the track")
        })
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

    /// The tracks that play the shared synth (the Synth kit), a bit each,
    /// as the audio loop has them (`TrackKits::on_synth`).
    pub fn set_synth_tracks(&mut self, tracks: u128) {
        self.synth_tracks = tracks;
    }

    /// Whether track `track` plays the synth the pads have: the shared
    /// synth, with no instrument of its own or MIDI output over it. REC
    /// records the pads there as they are.
    pub fn plays_pads_synth(&self, track: usize) -> bool {
        track < 128
            && self.synth_tracks >> track & 1 == 1
            && !self.configured(track)
            && !self.track_plays_instrument(track)
    }

    /// The pads part of the project ([`PadFocus::text`]).
    pub fn pads_part(&self) -> String {
        self.pads_focus.text()
    }

    /// A project's pads part (None: New, or a project saved before it had
    /// one: the pads follow the track). A part that doesn't read is said,
    /// and the pads follow the track.
    pub fn open_pads(&mut self, text: Option<&str>) -> (Vec<String>, Vec<Effect>) {
        let mut notes = Vec::new();
        let focus = match text.map(PadFocus::parse) {
            None => PadFocus::Track,
            Some(Ok(f)) => f,
            Some(Err(why)) => {
                notes.push(format!(
                    "project: the pads part wasn't opened ({why}); the pads follow the track"
                ));
                PadFocus::Track
            }
        };
        self.pads_focus = focus;
        (notes, vec![Effect::PadsFocus(focus)])
    }

    /// The pads follow the selected track again, saying what `why` writes.
    /// Nothing (and no allocation) when they do already.
    pub(super) fn pads_back(&mut self, why: impl FnOnce(usize) -> String) -> Vec<Effect> {
        if self.pads_focus == PadFocus::Track {
            return Vec::new();
        }
        self.pads_focus = PadFocus::Track;
        self.project_edit = true;
        vec![
            Effect::PadsFocus(PadFocus::Track),
            Effect::Note(why(self.track + 1)),
        ]
    }
}
