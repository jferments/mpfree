//! The list of synths on the Synth tab, picking for the pads (the
//! instrument redesign's step 6: "The list from PR #715's history, with
//! its Cancel fix, picks for the pads"; Jay's device notes of 20 Nov 2026:
//! "assign a synth ... in just 1 or 2 actions, or ... preview it on the
//! pads").
//!
//! - **The list**: a tap on the instrument card opens it over the page:
//!   the track's instrument first (the pads follow the track again), then
//!   the built-in engines, each with what it is good for, then the plugin
//!   synths ([`App::open_synth_pick`]).
//! - **A tap plays it on the pads at once**, whichever track is selected,
//!   so the musician can try several in a row ([`App::pick_pads_synth`]).
//!   No track changes (rule 1): the synth goes into a free place
//!   (`instruments::FREE`), an instrument no track has, on a mixer strip
//!   of its own ("FM · Pads"), and the pads play it ([`PadFocus`]). A
//!   second tap puts the next synth into the same place while nothing
//!   else plays it; the one the pads had when the list opened is kept for
//!   Cancel.
//! - **Keep** leaves the pads on it; **Cancel** gives them back what they
//!   played when the list opened ([`App::close_synth_pick`]). Leaving the
//!   tab or a second tap on the card keeps it. A synth tried and left goes
//!   away once nothing plays it ([`App::put_away_free`]); REC records it
//!   on a free track, which then plays it (desk.rs, the recording plan).

use super::{App, Effect, PadFocus, View};
use crate::instruments::{free, Held, FREE, SLOTS};
use crate::synth::Engine;

/// The list as the screen draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SynthPickView {
    /// The track whose instrument the first choice gives the pads back
    /// (0 based): the selected one.
    pub track: usize,
    /// The track's instrument first, then the engines, then the plugin
    /// synths: each its name and a line on what it is good for.
    pub choices: Vec<(String, String)>,
    /// The choice the pads play now, None for one the list doesn't have
    /// (the shared synth, chosen with Play on the pads).
    pub now: Option<usize>,
    /// The choice they played when the list opened (Cancel's).
    pub had: Option<usize>,
}

/// The list while it is open: what the pads played when it opened, for
/// Cancel, and the free place they played then, kept until it closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SynthPick {
    had: PadFocus,
    kept: Option<usize>,
    /// The list's choice they played then.
    had_choice: Option<usize>,
}

/// What each built-in synth is good for, in musicians' words (the manual's
/// table, short).
pub fn good_for(e: Engine) -> &'static str {
    match e {
        Engine::Poly => "Basses, leads, pads",
        Engine::Fm => "Electric pianos, bells",
        Engine::Drums => "A drum sound on each pad",
        Engine::Pluck => "Guitars, harps, marimbas",
        Engine::Keys => "Tine pianos, reeds, clav",
        Engine::Drawbar => "Organs: tonewheel, combo",
        Engine::Squelch => "Acid lines, rubbery basses",
        Engine::Analog => "Brass, warm pads, leads",
        Engine::Strings => "String machine pads",
        Engine::Bass => "Synth bass, 808s, reese",
        Engine::Wind => "Sax, clarinet, flute",
        Engine::Choir => "Choir voices, vowel pads",
    }
}

impl App {
    /// The list's synths after its first choice: the built-in engines,
    /// then the plugin synths, as (node id, name).
    fn pick_choices(&self) -> Vec<(&'static str, &'static str)> {
        Engine::ALL
            .iter()
            .map(|e| (e.node_id(), e.name()))
            .chain(self.plugin_synths.iter().copied())
            .collect()
    }

    /// The free place a synth tapped in the list would play from (`kept`:
    /// the one the pads had when it opened, never reused): the place the
    /// pads play now while no track plays it too, else an empty one. None
    /// when every place is taken or [`SLOTS`] instruments are loaded.
    fn pick_place(&self, kept: Option<usize>) -> Option<usize> {
        let sent = |h: usize| (0..mpfree_seq::MAX_TRACKS).any(|t| self.track_send(t) == Some(h));
        let now = self
            .pads_to
            .or_else(|| self.pads_holder()?.checked_sub(crate::instruments::TRACKS));
        if let Some(n) = now.filter(|&n| n < FREE && Some(n) != kept && !sent(free(n))) {
            return Some(n);
        }
        let loaded = self.track_sounds.iter().flatten().count();
        if loaded >= SLOTS {
            return None;
        }
        (0..FREE).find(|&n| self.track_sound(free(n)).is_none() && self.pads_to != Some(n))
    }

    /// Whether the open list keeps free place `n` for Cancel.
    pub(super) fn keeps_free(&self, n: usize) -> bool {
        self.synth_pick.is_some_and(|p| p.kept == Some(n))
    }

    /// The list as the screen draws it, while it is open.
    fn synth_pick_view(&self) -> Option<SynthPickView> {
        let pick = self.synth_pick?;
        let synths = self.pick_choices();
        let track = self.track;
        let first = (
            format!("Track {}'s", track + 1),
            "The pads follow the track again".to_string(),
        );
        let choices = std::iter::once(first)
            .chain(synths.iter().map(|&(id, name)| {
                (
                    name.to_string(),
                    Engine::from_node_id(id)
                        .map_or("Plugin", good_for)
                        .to_string(),
                )
            }))
            .collect();
        let playing = self
            .pads_to
            .and_then(|n| self.track_sound(free(n)))
            .or_else(|| self.pads_held());
        let now = match (self.pads_focus, playing) {
            (_, Some(h)) => synths.iter().position(|c| c.0 == h.id).map(|i| i + 1),
            (PadFocus::Track, None) => Some(0),
            (PadFocus::Instrument(_), None) => None,
        };
        Some(SynthPickView {
            track,
            choices,
            now,
            had: pick.had_choice,
        })
    }

    /// A tap on the instrument card: the list of synths opens over the
    /// Synth tab, or, open, closes and keeps what the pads play.
    pub fn open_synth_pick(&mut self) -> Vec<Effect> {
        if self.view != View::Synth {
            return Vec::new();
        }
        if self.synth_pick.is_some() {
            return self.close_synth_pick(true);
        }
        let kept = self
            .pads_holder()
            .and_then(|h| h.checked_sub(crate::instruments::TRACKS));
        self.synth_pick = Some(SynthPick {
            had: self.pads_focus,
            kept,
            had_choice: None,
        });
        let view = self.synth_pick_view();
        if let (Some(p), Some(v)) = (self.synth_pick.as_mut(), view.as_ref()) {
            p.had_choice = v.now;
        }
        vec![Effect::SynthPick(self.synth_pick_view())]
    }

    /// A tap on choice `i` of the open list: the pads play it now (the
    /// first: the track's instrument again).
    pub fn pick_pads_synth(&mut self, i: usize) -> Vec<Effect> {
        let Some(pick) = self.synth_pick else {
            return Vec::new();
        };
        let mut fx = Vec::new();
        if i == 0 {
            fx.extend(self.pads_follow_track());
        } else if let Some(&(id, name)) = self.pick_choices().get(i - 1) {
            fx.extend(self.pads_synth(id, name, pick.kept));
        }
        fx.push(Effect::SynthPick(self.synth_pick_view()));
        fx
    }

    /// The pads play synth `id` (`name`) from a free place (not `kept`),
    /// said; or why not.
    fn pads_synth(
        &mut self,
        id: &'static str,
        name: &'static str,
        kept: Option<usize>,
    ) -> Vec<Effect> {
        if self.generating() {
            return vec![Effect::Tell(
                "The pads play a synth picked here as pressed: let go of NOTE REPEAT or Latch, or close the Arp panel, first"
                    .into(),
            )];
        }
        let Some(n) = self.pick_place(kept) else {
            return vec![Effect::Tell(format!(
                "No room for {name}: {SLOTS} instruments are loaded, the most at once. Set a track to Kit first"
            ))];
        };
        let holder = free(n);
        if self.track_sound(holder).map(|h| h.id) == Some(id) {
            return Vec::new();
        }
        self.set_track_sound(
            holder,
            Some(Held {
                id,
                name,
                instance: 0,
            }),
        );
        if self.pads_holder() != Some(holder) {
            self.pads_to = Some(n);
        }
        self.project_edit = true;
        vec![
            Effect::TrackSound {
                track: holder,
                id: Some(id),
            },
            Effect::Note(format!(
                "The pads play {name} on every track · Track {} keeps what it plays · REC records it on a free track",
                self.track + 1
            )),
        ]
    }

    /// Keep (or the page left, or the card again): the list closes and the
    /// pads keep what they play. Cancel: they play what they played when it
    /// opened again (the synths tried go away, as nothing plays them).
    pub fn close_synth_pick(&mut self, keep: bool) -> Vec<Effect> {
        let Some(pick) = self.synth_pick.take() else {
            return Vec::new();
        };
        let mut fx = Vec::new();
        let back = match pick.had {
            PadFocus::Instrument(id) => self.instrument_list.kind(id).is_some(),
            PadFocus::Track => true,
        };
        if !keep && (self.pads_focus != pick.had || self.pads_to.is_some()) && back {
            self.pads_focus = pick.had;
            self.pads_to = None;
            self.project_edit = true;
            fx.push(Effect::PadsFocus(pick.had));
        }
        fx.push(Effect::SynthPick(None));
        fx
    }
}
