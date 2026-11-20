//! The list of synths on the Synth tab (Jay's device notes, 20 Nov 2026:
//! "assign a synth ... in just 1 or 2 actions, or ... preview it on the
//! pads"; the "Synth page: pick and play" mockups).
//!
//! - **Play on the pads**: while the selected track plays a kit, the Synth
//!   tab shows the synth but the pads play the kit. One tap on the button
//!   under the instrument card gives the track that synth as its own
//!   instrument (a copy of the sound shown, edits included), so the pads
//!   play it ([`App::play_synth_on_pads`]).
//! - **The list**: a tap on the instrument card opens the list of synths
//!   over the page: Kit, the built-in engines, then the plugin synths. A
//!   tap on one puts it on the track at once, so the pads play it and the
//!   musician can try several in a row ([`App::pick_synth`]). Keep leaves
//!   it there; Cancel puts back what the track played when the list
//!   opened, its own sound and all ([`App::close_synth_pick`]). Leaving the
//!   tab, selecting another track or a second tap on the card keeps it.
//!
//! Every change goes through the same path as the Instrument row of the
//! Synth options ([`App::put_track_sound`]): one rule for the 16 places,
//! one [`Effect::TrackSound`] for the audio loop.

use super::{App, Effect, View};
use crate::instruments::Held;
use crate::synth::Engine;

/// The list as the screen draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SynthPickView {
    /// The track it picks for (0 based).
    pub track: usize,
    /// Kit first, then the engines, then the plugin synths: each its name
    /// and a line on what it is good for.
    pub choices: Vec<(String, String)>,
    /// The choice the track plays now.
    pub now: usize,
    /// The choice it played when the list opened (Cancel's).
    pub had: usize,
}

/// The list open on a track: what it played when it opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SynthPick {
    track: usize,
    had: Option<Held>,
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
    /// The Instrument row's choices for a track: Kit (None), the built-in
    /// engines, then the plugin synths, as (node id, name).
    pub(super) fn sound_choices(&self) -> Vec<Option<(&'static str, &'static str)>> {
        std::iter::once(None)
            .chain(Engine::ALL.iter().map(|e| Some((e.node_id(), e.name()))))
            .chain(self.plugin_synths.iter().map(|&p| Some(p)))
            .collect()
    }

    /// Where `id` (None: the kit) is in [`App::sound_choices`].
    fn choice_of(&self, id: Option<&'static str>) -> usize {
        self.sound_choices()
            .iter()
            .position(|c| c.map(|c| c.0) == id)
            .unwrap_or(0)
    }

    /// The selected track plays choice `to` of [`App::sound_choices`]
    /// (clamped). Nothing when it plays it already; a track on the kit
    /// takes a place only while one of the 16 is free.
    pub(super) fn put_track_sound(&mut self, to: usize) -> Vec<Effect> {
        let choices = self.sound_choices();
        let now = self.track_sound(self.track).map(|h| h.id);
        let at = self.choice_of(now);
        let to = to.min(choices.len() - 1);
        if to == at {
            return Vec::new();
        }
        let pick = choices[to];
        let others = self.track_sounds.iter().flatten().count();
        if now.is_none() && others >= crate::instruments::SLOTS {
            return vec![Effect::Tell(format!(
                "Track {} plays the kit: {} tracks already have their own instrument, the most at once. Set one of them to Kit first",
                self.track + 1,
                crate::instruments::SLOTS
            ))];
        }
        let track = self.track;
        // The project keeps each track's instrument: unsaved work.
        self.project_edit = true;
        self.set_track_sound(
            track,
            pick.map(|(id, name)| Held {
                id,
                name,
                instance: 0,
            }),
        );
        vec![Effect::TrackSound {
            track,
            id: pick.map(|c| c.0),
        }]
    }

    /// The list as the screen draws it, while it is open.
    fn synth_pick_view(&self) -> Option<SynthPickView> {
        let pick = self.synth_pick?;
        let choices = self
            .sound_choices()
            .into_iter()
            .map(|c| match c {
                None => ("Kit".to_string(), "The track's kit again".to_string()),
                Some((id, name)) => (
                    name.to_string(),
                    Engine::from_node_id(id)
                        .map_or("Plugin", good_for)
                        .to_string(),
                ),
            })
            .collect();
        Some(SynthPickView {
            track: pick.track,
            choices,
            now: self.choice_of(self.track_sound(pick.track).map(|h| h.id)),
            had: self.choice_of(pick.had.map(|h| h.id)),
        })
    }

    /// A tap on the instrument card: the list of synths opens over the
    /// Synth tab, or, open, closes and keeps what the track plays.
    pub fn open_synth_pick(&mut self) -> Vec<Effect> {
        if self.view != View::Synth {
            return Vec::new();
        }
        if self.synth_pick.is_some() {
            return self.close_synth_pick(true);
        }
        let track = self.track;
        self.synth_pick = Some(SynthPick {
            track,
            had: self.track_sound(track),
        });
        // The audio loop keeps the track's sound now, for Cancel.
        vec![
            Effect::KeepTrackSound(track),
            Effect::SynthPick(self.synth_pick_view()),
        ]
    }

    /// A tap on choice `i` of the open list: the track plays it now.
    pub fn pick_synth(&mut self, i: usize) -> Vec<Effect> {
        if self.synth_pick.is_none() {
            return Vec::new();
        }
        let mut fx = self.put_track_sound(i);
        fx.push(Effect::SynthPick(self.synth_pick_view()));
        fx
    }

    /// Keep (or the page left, another track, the card again): the list
    /// closes and the track keeps what it plays. Cancel: what it played
    /// when the list opened comes back, its own sound as it was.
    pub fn close_synth_pick(&mut self, keep: bool) -> Vec<Effect> {
        let Some(pick) = self.synth_pick.take() else {
            return Vec::new();
        };
        let mut fx = Vec::new();
        let now = self.track_sound(pick.track);
        if !keep && now.map(|h| h.id) != pick.had.map(|h| h.id) {
            self.set_track_sound(pick.track, pick.had);
            self.project_edit = true;
            fx.push(Effect::TrackSoundBack(pick.track));
        }
        fx.push(Effect::SynthPick(None));
        fx
    }

    /// The Play on the pads button: the selected track, on its kit, plays
    /// the synth the tab shows (the synth on the pads' engine) as its own
    /// instrument. Says so; nothing when the track has an instrument.
    pub fn play_synth_on_pads(&mut self) -> Vec<Effect> {
        if self.track_sound(self.track).is_some() {
            return Vec::new();
        }
        let engine = self.synth_pads.engine;
        let mut fx = self.put_track_sound(self.choice_of(Some(engine.node_id())));
        if fx.iter().any(|e| matches!(e, Effect::TrackSound { .. })) {
            fx.push(Effect::Note(format!(
                "Track {} plays {} on the pads · tap {} to pick another, or Kit",
                self.track + 1,
                engine.name(),
                engine.name()
            )));
        }
        if self.synth_pick.is_some() {
            fx.push(Effect::SynthPick(self.synth_pick_view()));
        }
        fx
    }
}
