//! Add a part's New track (composing tools S2.6b,
//! `docs/dev/design/compose.md`; Jay's card of 23 Nov 08:33Z, "A new
//! synth, kept with Keep"): the part's synth, made in a free place
//! (`instruments::FREE`) while the panel tries the part on a track Keep
//! would add.
//!
//! - **Try** ([`AddSound::Try`]): the synth goes into a free place nothing
//!   else uses (the pads, a bank, the list of synths), as a synth picked
//!   for the pads does ([`App::pick_pads_synth`]). Once the place holds it,
//!   the track Keep would add (the project's track count: the index it
//!   will get) plays it by a send ([`Effect::AddPartSend`]), so the tries
//!   play on it; the screen holds each try until then, so none sounds on
//!   a kit. Another sound on the Sound row goes into the same place (the
//!   send stays: `Instruments::put`).
//! - **Back** ([`AddSound::Back`]): the send ends and the synth is put
//!   away, as nothing plays it.
//! - **Kept** ([`AddSound::Kept`]): Keep added the track, which plays the
//!   synth from then on, as REC's synth take on a new track does.
//! - A track added or taken away during a try moves the index: the send
//!   ends ([`App::add_sound_follow`]), the time engine drops the tries,
//!   and Try again asks for the new index.
//!
//! [`App::pick_pads_synth`]: super::App::pick_pads_synth

use super::{App, Effect};
use crate::instruments::{free, Held, FREE, SLOTS};

/// What the screen asks of New track's synth
/// ([`crate::gui::FromUi::AddPartSound`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddSound {
    /// A try on a new track: track `track` (the index Keep will give it)
    /// plays synth `id`, an engine's node id ("mpfree.bass").
    Try { track: usize, id: &'static str },
    /// The panel closed with nothing kept, or the part goes on a track
    /// that is there: the synth goes.
    Back,
    /// Keep added the track with the part: it plays the synth from now on.
    Kept,
}

/// The synth made for a new track's tries: its free place, the track that
/// will play it, its id, and whether that track plays it yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AddSynth {
    pub place: usize,
    pub track: usize,
    pub id: &'static str,
    pub sent: bool,
}

impl App {
    /// What the screen asked of New track's synth, done (or why not).
    pub fn add_part_sound(&mut self, ask: AddSound) -> Vec<Effect> {
        match ask {
            AddSound::Try { track, id } => self.add_sound_try(track, id),
            AddSound::Back => self.add_sound_back(),
            AddSound::Kept => {
                self.add_sound = None;
                Vec::new()
            }
        }
    }

    /// The holder of the synth made for a new track's tries, while there is
    /// one (desk.rs reads a Drums synth's pads there).
    pub fn add_sound_holder(&self) -> Option<usize> {
        self.add_sound.map(|s| free(s.place))
    }

    /// Whether free place `n` holds New track's synth.
    pub(super) fn holds_add_sound(&self, n: usize) -> bool {
        self.add_sound.is_some_and(|s| s.place == n)
    }

    /// Synth `id` for track `track`'s tries: in the place it has, else in
    /// an empty one; made again if the place holds another sound (or lost
    /// it, the builder busy). The send follows once the place holds it.
    fn add_sound_try(&mut self, track: usize, id: &'static str) -> Vec<Effect> {
        let name = crate::synth::Engine::from_node_id(id).map_or("new", |e| e.name());
        let mut fx = Vec::new();
        // Another track index (one was added or taken away): its send ends.
        if let Some(s) = self.add_sound.filter(|s| s.track != track) {
            fx.extend(self.end_add_send(s));
        }
        let place = match self.add_sound.map(|s| s.place).or_else(|| self.add_place()) {
            Some(n) => n,
            None => {
                return vec![Effect::Tell(format!(
                    "No room for the {name} synth: {SLOTS} instruments are loaded, the most at once. Set a track to Kit first"
                ))];
            }
        };
        let holder = free(place);
        let sent = self.add_sound.is_some_and(|s| s.track == track && s.sent);
        if self.track_sound(holder).map(|h| h.id) != Some(id) {
            self.set_track_sound(
                holder,
                Some(Held {
                    id,
                    name,
                    instance: 0,
                }),
            );
            fx.push(Effect::TrackSound {
                track: holder,
                id: Some(id),
            });
        }
        self.add_sound = Some(AddSynth {
            place,
            track,
            id,
            sent,
        });
        fx
    }

    /// An empty free place for New track's synth: none that the pads, a
    /// bank or the open list of synths uses. None when every place is
    /// taken or [`SLOTS`] instruments are loaded.
    fn add_place(&self) -> Option<usize> {
        if self.track_sounds.iter().flatten().count() >= SLOTS {
            return None;
        }
        (0..FREE).find(|&n| {
            self.track_sound(free(n)).is_none()
                && self.pads_to != Some(n)
                && !self.bank_to.contains(&Some(n))
                && !self.keeps_free(n)
        })
    }

    /// Back: the send ends and the synth is put away, nothing playing it.
    fn add_sound_back(&mut self) -> Vec<Effect> {
        let Some(s) = self.add_sound.take() else {
            return Vec::new();
        };
        let mut fx = self.end_add_send(s);
        fx.extend(self.put_away_free());
        fx
    }

    /// The send from `s`'s track to its synth ends, if it was made.
    fn end_add_send(&mut self, s: AddSynth) -> Vec<Effect> {
        if let Some(a) = self.add_sound.as_mut() {
            a.sent = false;
        }
        if !s.sent && self.track_send(s.track) != Some(free(s.place)) {
            return Vec::new();
        }
        if let Some(t) = self.track_sends.get_mut(s.track) {
            *t = None;
        }
        vec![Effect::AddPartSend {
            track: s.track,
            to: None,
        }]
    }

    /// Each period, after the list of instruments
    /// ([`App::set_instrument_list`]), with the project's track count
    /// `tracks`: once New track's place holds its synth, the track Keep
    /// would add plays it; a track added or taken away since ends that
    /// send (the tries are dropped, and Try again asks again). Nothing
    /// allocated while nothing changes.
    pub fn add_sound_follow(&mut self, tracks: usize) -> Vec<Effect> {
        let Some(s) = self.add_sound else {
            return Vec::new();
        };
        if tracks != s.track {
            return if s.sent {
                self.end_add_send(s)
            } else {
                Vec::new()
            };
        }
        if s.sent || self.instrument_list.free_of(s.place).is_none() {
            return Vec::new();
        }
        let holder = free(s.place);
        self.add_sound = Some(AddSynth { sent: true, ..s });
        // Before the audio loop says so, as REC's send does.
        if let Some(t) = self.track_sends.get_mut(s.track) {
            *t = u8::try_from(holder).ok();
        }
        vec![Effect::AddPartSend {
            track: s.track,
            to: Some(holder),
        }]
    }
}
