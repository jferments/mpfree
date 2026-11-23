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
//!   more"; [`App::rec_more_synths`]). The audio loop gives the route's
//!   keys to that track ([`App::route_recs`]).
//! - **Saved** on the route's line by the synth's number (`route =
//!   instrument 4 < MIDI · ch 2`, `mpfree_midi::setup::TrackRoute`); a
//!   build from before names the line it can't read and leaves it out.

use super::pad_layers::MORE;
use super::App;
use crate::instrument_list::{InstrumentId, Kind};

/// The most synths the routes name at once.
pub const ROUTE_SYNTHS: usize = crate::instruments::ROUTE_SYNTHS;

/// What REC may record besides the first: the pads' layers, then the
/// synths the routes name.
pub const REC_MORE: usize = MORE + ROUTE_SYNTHS;

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

    /// The synths the routes name, for REC, row by row: each one's name
    /// and holder (None for the shared synth). One the project's list
    /// hasn't got is left out.
    fn route_rec_synths(&self) -> [Option<(&'static str, Option<usize>)>; ROUTE_SYNTHS] {
        self.route_synths.map(|id| {
            let id = id?;
            match self.instrument_list.kind(id)? {
                Kind::SharedSynth => Some((self.instrument_name(id), None)),
                k => k.holder().map(|h| (self.instrument_name(id), Some(h))),
            }
        })
    }

    /// Whether REC has a synth a route names to record.
    pub fn routes_record(&self) -> bool {
        self.route_rec_synths().iter().any(Option::is_some)
    }

    /// What REC records besides the first (steps 11c and 11e), in order:
    /// the pads' layers that play now ([`App::pads_layer_synths`]), then
    /// the synths the routes name, row by row. Allocates nothing.
    pub fn rec_more_synths(&self) -> [Option<(&'static str, Option<usize>)>; REC_MORE] {
        let layers = self.pads_layer_synths();
        let routes = self.route_rec_synths();
        std::array::from_fn(|i| {
            if i < MORE {
                layers[i]
            } else {
                routes[i - MORE]
            }
        })
    }

    /// The track each of [`App::rec_more_synths`] records on: one of the
    /// other armed tracks (`also`, a bit each: `TimeHost::also`) that
    /// plays its synth, each a track of its own, as REC sets them up
    /// (`desk.rs`, `rec_synth`). None: nothing there, or no such track.
    /// Allocates nothing.
    pub fn rec_more_tracks(&self, also: u128) -> [Option<u8>; REC_MORE] {
        let mut used = 0u128;
        self.rec_more_synths().map(|l| {
            let (_, holder) = l?;
            let t = (0..128).find(|&t| {
                also >> t & 1 == 1 && used >> t & 1 == 0 && self.track_plays_synth(t, holder)
            })?;
            used |= 1 << t;
            Some(t as u8)
        })
    }

    /// The track each route row's keys go into now
    /// ([`App::rec_more_tracks`], the routes' part). Allocates nothing.
    pub fn route_recs(&self, also: u128) -> [Option<u8>; ROUTE_SYNTHS] {
        let all = self.rec_more_tracks(also);
        std::array::from_fn(|j| all[MORE + j])
    }
}
