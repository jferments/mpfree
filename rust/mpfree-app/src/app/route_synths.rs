//! MIDI routes that name a synth (the instrument redesign's step 11e; the
//! plan's "Step 11", Joshi's "Build all three" of 22 Nov 2026).
//!
//! - **What it is.** A route in the I/O view's Routing (a port and channel
//!   that play one track, whichever is armed) can name a synth instead:
//!   one no track has of its own, the shared synth or one picked in the
//!   list of synths. Its stepper goes on past the tracks to them, by
//!   their channel names ("FM · Pads"). A second keyboard plays that
//!   synth whatever the pads and the armed track do.
//! - **How it plays** (`main.rs`, `RouteSynths`): MIDI in gives each
//!   synth the routes name a route row of the instrument host
//!   (`instruments::routed`), up to [`ROUTE_SYNTHS`] (more are left out
//!   and said; the Routing card refuses a fifth). Each period the audio
//!   loop points the rows at their synths' holders
//!   ([`App::route_synth_holders`]) and plays the shared synth by note for
//!   a row that names it ([`App::route_synth_shared`]). The MIDI monitor
//!   says where each key went ("FM · Pads, by its route").
//! - **Kept.** A synth picked in the list that a route names isn't put
//!   away while nothing else plays it ([`App::routes_hold`]).
//! - **REC** records each one as the pads' layers are (step 11c): on a
//!   track of its own, an armed track that plays its synth, else a free
//!   one pointed at it, else a new one, armed with the first in one take
//!   and named on the REC chip before you press ("REC → Track 1 + 1
//!   more"; [`App::rec_more_sounds`]). The audio loop gives the route's
//!   keys to that track ([`App::route_recs`]).
//! - **Saved** on the route's line by the synth's number (`route =
//!   instrument 4 < MIDI · ch 2`, `mpfree_midi::setup::TrackRoute`); a
//!   build from before names the line it can't read and leaves it out.
//! - **A kit** (sampler step 15f): a route can name a kit of the list of
//!   instruments by its number too ([`App::route_synth_kits`]). Each note
//!   plays the kit as its Keys play sends it: the pad that sends it (the
//!   MPC note map, any bank) or the keygroups that hold it
//!   (`RouteSynths::kit_key` in `main.rs`); the wheel bends its
//!   keygroups. REC records it on a track of its own pointed at the kit,
//!   each note as that kit takes it. The Routing card lists synths only
//!   until step 15e's pickers; a project's setup can name a kit.

use super::pad_layers::MORE;
use super::pads_focus::{PadFocus, RecFrom, RecMore, RecSound};
use super::App;
use crate::instrument_list::{InstrumentId, Kind};

/// The most synths the routes name at once.
pub const ROUTE_SYNTHS: usize = crate::instruments::ROUTE_SYNTHS;

/// What REC may record besides the first: the pads' layers, then the
/// synths the routes name, then the pads' base and banks' own (sampler
/// step 15c).
pub const REC_MORE: usize = MORE + ROUTE_SYNTHS + super::pads_focus::MAP_MORE;

impl App {
    /// The synths the routes name now, by the project's instrument number
    /// (MIDI in's `route_synths`, row by row). True when they changed.
    pub fn set_route_synths(&mut self, ids: [Option<u32>; ROUTE_SYNTHS]) -> bool {
        let ids = ids.map(|n| n.and_then(InstrumentId::from_number));
        if ids == self.route_synths {
            return false;
        }
        self.route_synths = ids;
        true
    }

    /// The holder in the instrument host each route row plays (None: no
    /// route there, the shared synth, or a synth the list hasn't got).
    /// Allocates nothing.
    pub fn route_synth_holders(&self) -> [Option<usize>; ROUTE_SYNTHS] {
        self.route_synths
            .map(|id| id.and_then(|id| self.instrument_holder(id)))
    }

    /// The kit each route row names (sampler step 15f), by its place in
    /// the list of kits (None: no route there, a synth, or one the list
    /// hasn't got). Allocates nothing.
    pub fn route_synth_kits(&self) -> [Option<u8>; ROUTE_SYNTHS] {
        self.route_synths
            .map(|id| match self.instrument_list.kind(id?)? {
                Kind::Kit { kit } => Some(kit.min(usize::from(u8::MAX)) as u8),
                _ => None,
            })
    }

    /// The route rows that play the shared synth, a bit each.
    pub fn route_synth_shared(&self) -> u8 {
        (0..ROUTE_SYNTHS)
            .filter(|&j| self.route_synths[j] == Some(InstrumentId::SHARED_SYNTH))
            .fold(0, |bits, j| bits | 1 << j)
    }

    /// Whether a route names holder `holder`'s synth, so it stays.
    pub fn routes_hold(&self, holder: usize) -> bool {
        self.route_synth_holders().contains(&Some(holder))
    }

    /// The synths the routes name, for REC, row by row (step 11e), and the
    /// kits (sampler step 15f). One the project's list hasn't got is left
    /// out.
    fn route_rec_synths(&self) -> [Option<RecMore>; ROUTE_SYNTHS] {
        std::array::from_fn(|j| {
            let f = PadFocus::Instrument(self.route_synths[j]?);
            Some(RecMore {
                name: self.sound_name(f),
                sound: self
                    .sound_of(f)
                    .filter(|s| matches!(s, RecSound::Synth(_) | RecSound::Kit(_)))?,
                from: RecFrom::Route(j),
            })
        })
    }

    /// What REC records besides the first (steps 11c and 11e, sampler step
    /// 15c), in order: the pads' layers ([`App::pads_layer_sounds`]), then
    /// the synths the routes name, row by row, then the pads' base and
    /// each bank's own ([`App::rec_map_sounds`]) that isn't a layer
    /// already. A route's synth that the pads play too gets a track of its
    /// own, as since step 11e, so its take never mixes with the pads'.
    /// Allocates nothing.
    pub fn rec_more_sounds(&self) -> [Option<RecMore>; REC_MORE] {
        let layers = self.pads_layer_sounds();
        let routes = self.route_rec_synths();
        let map = self.rec_map_sounds();
        let mut out: [Option<RecMore>; REC_MORE] = std::array::from_fn(|i| {
            if i < MORE {
                layers[i]
            } else if i < MORE + ROUTE_SYNTHS {
                routes[i - MORE]
            } else {
                None
            }
        });
        let mut at = MORE + ROUTE_SYNTHS;
        for m in map.into_iter().flatten() {
            if !out[..MORE].iter().flatten().any(|o| o.sound == m.sound) {
                out[at] = Some(m);
                at += 1;
            }
        }
        out
    }

    /// The track each of [`App::rec_more_sounds`] records on: one of the
    /// other armed tracks (`also`, a bit each: `TimeHost::also`) that
    /// plays it, each a track of its own, as REC sets them up (`desk.rs`,
    /// `rec_synth`). None: nothing there, or no such track. Allocates
    /// nothing.
    pub fn rec_more_tracks(&self, also: u128) -> [Option<u8>; REC_MORE] {
        let mut used = 0u128;
        self.rec_more_sounds().map(|m| {
            let m = m?;
            let t = (0..128).find(|&t| {
                also >> t & 1 == 1 && used >> t & 1 == 0 && self.track_plays(t, m.sound)
            })?;
            used |= 1 << t;
            Some(t as u8)
        })
    }

    /// The track recording what the pads play now while they play it on
    /// their own (sampler step 15c): the armed track REC set up for the
    /// pads' base or the bank's own they are on ([`App::rec_more_tracks`]),
    /// or for a layer of the same sound; never a route's. None while they
    /// follow the track, while no armed track plays it, and while it is a
    /// layer that plays now (the layer's track takes the hits then). The
    /// audio loop gives it the hits, the keys that play it and the pads'
    /// arp's notes. Allocates nothing.
    pub fn pads_own_rec(&self, also: u128) -> Option<u8> {
        let sound = self.pads_sound()?;
        let more = self.rec_more_sounds();
        let tracks = self.rec_more_tracks(also);
        let pads = |m: &RecMore| !matches!(m.from, RecFrom::Route(_)) && m.sound == sound;
        let i = (0..REC_MORE).find(|&i| more[i].as_ref().is_some_and(pads))?;
        let layer_now = matches!(more[i], Some(RecMore { from: RecFrom::Layer(l), .. }) if self.layer_plays_now(l));
        tracks[i].filter(|_| !layer_now)
    }

    /// The track each route row's keys go into now
    /// ([`App::rec_more_tracks`], the routes' part). Allocates nothing.
    pub fn route_recs(&self, also: u128) -> [Option<u8>; ROUTE_SYNTHS] {
        let all = self.rec_more_tracks(also);
        std::array::from_fn(|j| all[MORE + j])
    }
}
