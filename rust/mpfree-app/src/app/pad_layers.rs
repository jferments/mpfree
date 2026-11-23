//! Layers: the pads play more than one synth at once (the instrument
//! redesign's step 11a; the plan's "Step 11", Joshi's "Build all three"
//! of 22 Nov 2026).
//!
//! - **What plays.** What the pads play ([`PadFocus`], their own choice for
//!   every bank) and up to three synths more, [`PAD_LAYERS`] in all, such
//!   as a pad sound and a bass. Each pad hit, and each key of a keyboard
//!   playing the pads, plays every layer at the same velocity, each synth
//!   by its own pad map, on its own mixer strip, so mute and solo stay per
//!   strip and drop one layer in the Mix tab. A layer is a synth of the
//!   project's list: the shared synth, a track's own, or one picked in the
//!   list of synths (a free place's); never a kit.
//! - **Only with a synth chosen.** Layers go with a synth chosen for the
//!   pads (`PadFocus::Instrument`). A layer added while the pads follow a
//!   track that plays a synth chooses that synth first; over a kit the
//!   synth added is what the pads play, alone ([`App::toggle_pad_layer`]).
//!   Taking off the first gives its place to the next. Follow track gives
//!   the layers up with the rest; the arp never does (sampler step 15c):
//!   each hit plays the layers once, as pressed, while the arp plays the
//!   first (layers through the arp come with step 15f). A bank
//!   with a synth of its own (step 6c) plays that alone; the layers play
//!   on the other banks.
//! - **How the audio loop plays them** (`main.rs`, `PadLayers`): each
//!   period it points the instrument host's layer players
//!   (`instruments::layer`) at the layers' holders, as it points the
//!   pads' player, and turns the shared synth's layer on or off. A hit
//!   plays the layers whichever way the first plays (on its own, or
//!   through the armed track), before the arp and the recorder, and its
//!   let-go ends each where it started.
//! - **REC** (step 11c) records each layer on a track of its own, in one
//!   take: an armed track that plays its synth, or a free one pointed at
//!   it and armed with the first (`desk.rs`, `rec_plan` and `rec_synth`;
//!   [`App::pads_layer_sounds`]): every layer of the map, whichever bank
//!   the pads are on when you press (sampler step 15c). The audio loop
//!   gives each hit to the armed tracks of the layers that play on the
//!   bank the pads are on ([`App::pads_layer_recs`]). The synths MIDI
//!   routes name (step 11e, `app/route_synths.rs`), then the pads' base
//!   and the banks' own (sampler step 15c), record after them the same
//!   way.
//! - **Saved** in the pads part with the rest of the pad map (sampler step
//!   15b, [`super::PadMap`]): a line per layer after the first (`layer
//!   instrument 4`, `bank B` for a layer on one bank). A layer whose synth
//!   has gone keeps its place, missing and silent, said
//!   ([`App::set_instrument_list`], [`App::open_pads`]).

use super::pads_focus::{RecFrom, RecMore, RecSound};
use super::{App, Effect, LayerEntry, PadFocus};
use crate::instrument_list::{InstrumentId, Kind};

/// The most synths the pads play at once: what they play and three more.
pub const PAD_LAYERS: usize = crate::instruments::PAD_LAYERS;

/// The layers beyond the first.
pub const MORE: usize = PAD_LAYERS - 1;

/// Names joined: "FM + Pluck" ([`App::pads_layer_list`]); None when
/// there are none.
pub fn layer_names(list: &[Option<&'static str>; PAD_LAYERS]) -> Option<String> {
    let names: Vec<&str> = list.iter().flatten().copied().collect();
    (!names.is_empty()).then(|| names.join(" + "))
}

impl App {
    /// Whether the layers play now: a synth is chosen for the pads, and
    /// the bank they are on has none of its own (step 6c).
    pub fn layers_apply(&self) -> bool {
        matches!(self.base_now(), PadFocus::Instrument(_)) && self.pads_bank().is_none()
    }

    /// The synths the pads play beyond the first, in order, whether they
    /// play now ([`App::layers_apply`]) or wait on a bank's own, a missing
    /// one among them (its entry kept, sampler step 15b).
    pub fn pad_layers(&self) -> impl Iterator<Item = InstrumentId> + '_ {
        self.pad_map.layers.iter().flatten().map(|l| l.id)
    }

    /// Layer slot `i` if it plays on the bank the pads are on now: its
    /// instrument is in the project's list and its reach takes in the
    /// bank. Allocates nothing.
    fn layer_now(&self, i: usize) -> Option<LayerEntry> {
        self.pad_map
            .layers
            .get(i)
            .copied()
            .flatten()
            .filter(|l| l.reaches(self.pad_page) && self.instrument_list.kind(l.id).is_some())
    }

    /// The layers that play on the bank the pads are on now, in order
    /// ([`App::layer_now`]), whether the layers apply or not.
    fn layers_now(&self) -> impl Iterator<Item = InstrumentId> + '_ {
        (0..MORE).filter_map(|i| self.layer_now(i)).map(|l| l.id)
    }

    /// Whether the pads have layers.
    pub fn has_layers(&self) -> bool {
        self.pad_map.layers.iter().any(Option::is_some) || self.layer_to.iter().any(Option::is_some)
    }

    /// The holder in the instrument host of each layer that plays now, in
    /// order (None: no layer there, or the shared synth, which plays in
    /// the audio loop: [`App::pads_layer_synth`]). The audio loop points
    /// the host's layer players at them. Allocates nothing.
    pub fn pads_layer_holders(&self) -> [Option<usize>; MORE] {
        if !self.layers_apply() {
            return [None; MORE];
        }
        std::array::from_fn(|i| self.layer_now(i).and_then(|l| self.instrument_holder(l.id)))
    }

    /// Whether the shared synth plays as a layer now.
    pub fn pads_layer_synth(&self) -> bool {
        self.layers_apply() && self.layers_now().any(|id| id == InstrumentId::SHARED_SYNTH)
    }

    /// The pads' layers for REC (step 11c), in order: every layer of the
    /// map whose instrument the list has, while the pads have one of their
    /// own (a layer plays only then), whatever bank it reaches and the
    /// pads are on (sampler step 15c: REC records every sound the map
    /// plays). A kit isn't a layer yet (step 15f). Allocates nothing.
    pub fn pads_layer_sounds(&self) -> [Option<RecMore>; MORE] {
        let mut out = [None; MORE];
        if !matches!(self.base_now(), PadFocus::Instrument(_)) {
            return out;
        }
        let listed = (0..MORE).filter_map(|i| {
            let l = self.pad_map.layers[i]?;
            let sound = self.sound_of(PadFocus::Instrument(l.id))?;
            matches!(sound, RecSound::Synth(_)).then(|| RecMore {
                name: self.instrument_name(l.id),
                sound,
                from: RecFrom::Layer(i),
            })
        });
        for (slot, l) in out.iter_mut().zip(listed) {
            *slot = Some(l);
        }
        out
    }

    /// Whether layer slot `i` plays on the bank the pads are on now.
    pub(super) fn layer_plays_now(&self, i: usize) -> bool {
        self.layers_apply() && self.layer_now(i).is_some()
    }

    /// Whether track `track` plays a synth: holder `holder`'s, by its own
    /// or a send, or (None) the shared synth as its Synth kit; as the
    /// audio loop last said.
    pub fn track_plays_synth(&self, track: usize, holder: Option<usize>) -> bool {
        match holder {
            None => self.plays_pads_synth(track),
            Some(h) => self.track_owner(track) == Some(h),
        }
    }

    /// The track each layer that plays now records on (step 11c): one of
    /// the other armed tracks (`also`, a bit each: `TimeHost::also`) that
    /// plays its synth, as REC sets them up (`desk.rs`, `rec_synth`). None:
    /// no layer there, one that doesn't play on the bank the pads are on,
    /// or no such track. The audio loop gives them each pad hit. Allocates
    /// nothing.
    pub fn pads_layer_recs(&self, also: u128) -> [Option<u8>; MORE] {
        let more = self.rec_more_sounds();
        let all = self.rec_more_tracks(also);
        std::array::from_fn(|i| match more[i] {
            Some(RecMore {
                from: RecFrom::Layer(l),
                ..
            }) if self.layer_plays_now(l) => all[i],
            _ => None,
        })
    }

    /// The velocity a hit of `velocity` (raw) goes into a layer's track
    /// at, as the armed track's recorder takes the pads
    /// ([`App::pressed_event`]); None while the pads only write steps.
    pub fn layer_rec_velocity(&self, velocity: u8) -> Option<u8> {
        (!self.free_time_recording()).then(|| self.live_map().velocity(velocity.max(1)))
    }

    /// Instrument `id`'s name ("FM"), as the audio loop last said.
    pub fn instrument_name(&self, id: InstrumentId) -> &'static str {
        match self.instrument_list.kind(id) {
            Some(Kind::SharedSynth) => self.synth_pads.engine.name(),
            Some(k) => k
                .holder()
                .and_then(|h| self.track_sound(h))
                .map_or("a synth", |h| h.name),
            None => "a synth",
        }
    }

    /// What the pads play with their layers, by name, in order: the first,
    /// the layers, then those waiting to go in. All None while they play
    /// one thing (no layers, or none that play now). Allocates nothing, so
    /// the audio loop asks each period ([`App::pads_layer_names`]).
    pub fn pads_layer_list(&self) -> [Option<&'static str>; PAD_LAYERS] {
        let mut out = [None; PAD_LAYERS];
        let PadFocus::Instrument(first) = self.base_now() else {
            return out;
        };
        if !self.layers_apply() || !self.has_layers() {
            return out;
        }
        let waiting = self
            .layer_to
            .iter()
            .flatten()
            .filter_map(|&n| self.track_sound(crate::instruments::free(n)))
            .map(|h| h.name);
        let names = std::iter::once(self.instrument_name(first))
            .chain(self.layers_now().map(|id| self.instrument_name(id)))
            .chain(waiting);
        for (slot, name) in out.iter_mut().zip(names) {
            *slot = Some(name);
        }
        out
    }

    /// What the pads play with their layers, by name: "FM + Pluck". None
    /// while they play one thing.
    pub fn pads_layer_names(&self) -> Option<String> {
        layer_names(&self.pads_layer_list())
    }

    /// The synth the selected track plays, as one of the project's list:
    /// its own (or another track's, by a send), or the shared synth. None
    /// for a kit.
    pub(super) fn track_synth_id(&self) -> Option<InstrumentId> {
        if let Some(h) = self.track_owner(self.track) {
            return self.instrument_list.of_holder(h);
        }
        self.plays_pads_synth(self.track)
            .then_some(InstrumentId::SHARED_SYNTH)
    }

    /// The layers moved up to fill the gaps, in their order.
    pub(super) fn pack_layers(&mut self) {
        let mut packed = [None; MORE];
        for (slot, l) in packed.iter_mut().zip(self.pad_map.layers.iter().flatten()) {
            *slot = Some(*l);
        }
        self.pad_map.layers = packed;
    }

    /// A layer on or off (step 11a): instrument `id` of the project's
    /// list plays on the pads with what they play, or stops, said. Up to
    /// [`PAD_LAYERS`] in all. While the pads follow a track that plays a
    /// synth, that synth is chosen first; over a kit, `id` is what they
    /// play, alone. Taking off the first gives its place to the next
    /// layer. Not on a bank with a synth of its own. With the arp on, a
    /// layer plays each hit once, as pressed (sampler step 15c).
    pub fn toggle_pad_layer(&mut self, id: InstrumentId) -> Vec<Effect> {
        if self.instrument_list.kind(id).is_none() {
            return Vec::new();
        }
        if let Some(b) = self.pads_bank() {
            return vec![Effect::Tell(format!(
                "Bank {} plays its own synth alone: layers play on the banks that play what every bank plays",
                super::bank_letter(b)
            ))];
        }
        let name = self.instrument_name(id);
        let was = self.pads_focus();
        match self.pad_map.base {
            PadFocus::Track => match self.track_synth_id() {
                Some(t) if t == id => {
                    return vec![Effect::Tell(format!(
                        "The pads play {name} already: add another synth to layer it"
                    ))];
                }
                Some(t) => {
                    self.pad_map.base = PadFocus::Instrument(t);
                    self.pad_map.layers = [Some(LayerEntry::every(id)), None, None];
                }
                None => {
                    self.pad_map.base = PadFocus::Instrument(id);
                    self.pad_map.layers = [None; MORE];
                    self.project_edit = true;
                    return vec![
                        Effect::PadsFocus(self.pads_focus()),
                        Effect::Note(format!(
                            "The pads play {name} · a kit can't be a layer: add another synth to layer it"
                        )),
                    ];
                }
            },
            PadFocus::Instrument(first) if first == id => {
                let Some(next) = self.pad_map.layers[0].map(|l| l.id) else {
                    return vec![Effect::Tell(format!(
                        "{name} is all the pads play: Follow track gives them back to the track"
                    ))];
                };
                self.pad_map.base = PadFocus::Instrument(next);
                self.pad_map.layers[0] = None;
                self.pack_layers();
            }
            PadFocus::Instrument(_) if self.pad_layers().any(|l| l == id) => {
                for l in self
                    .pad_map
                    .layers
                    .iter_mut()
                    .filter(|l| l.is_some_and(|l| l.id == id))
                {
                    *l = None;
                }
                self.pack_layers();
            }
            PadFocus::Instrument(_) => {
                let Some(slot) = self.pad_map.layers.iter_mut().find(|l| l.is_none()) else {
                    return vec![Effect::Tell(format!(
                        "The pads play {PAD_LAYERS} synths, the most: take one off first"
                    ))];
                };
                *slot = Some(LayerEntry::every(id));
            }
        }
        self.project_edit = true;
        let mut fx = Vec::new();
        if self.pads_focus() != was {
            fx.push(Effect::PadsFocus(self.pads_focus()));
        }
        let now = self
            .pads_layer_names()
            .unwrap_or_else(|| match self.pad_map.base {
                PadFocus::Instrument(f) => self.instrument_name(f).to_string(),
                PadFocus::Track => "the track's instrument".to_string(),
            });
        let on =
            self.pad_layers().any(|l| l == id) || self.pad_map.base == PadFocus::Instrument(id);
        fx.push(Effect::Note(if on {
            format!("The pads play {now} · each on its own strip in the mixer")
        } else {
            format!("{name} is off the pads · they play {now}")
        }));
        fx
    }

    /// The layers given up (Follow track, a project opened).
    pub(super) fn clear_layers(&mut self) {
        self.pad_map.layers = [None; MORE];
        self.layer_to = [None; MORE];
    }

    /// Whether a layer plays holder `holder`, or waits for free place
    /// `n`'s id (`n`: the holder's place), so nothing puts it away.
    pub(super) fn layers_hold(&self, holder: usize) -> bool {
        let n = holder.checked_sub(crate::instruments::TRACKS);
        self.layer_to.iter().any(|&t| t.is_some() && t == n)
            || self
                .pad_layers()
                .any(|id| self.instrument_holder(id) == Some(holder))
    }

    /// The list of instruments changed (`App::set_instrument_list`): a
    /// free place given a layer's synth has its id now. A layer whose
    /// synth has gone keeps its place, missing (sampler step 15b: nothing
    /// implicit changes the pads), and is silent meanwhile; the caller
    /// says so. Allocates nothing.
    pub(super) fn layers_follow_list(&mut self) {
        for l in 0..MORE {
            if let Some(id) = self.layer_to[l].and_then(|n| self.instrument_list.free_of(n)) {
                self.layer_to[l] = None;
                if self.pad_map.base != PadFocus::Instrument(id)
                    && !self.pad_layers().any(|x| x == id)
                {
                    if let Some(slot) = self.pad_map.layers.iter_mut().find(|s| s.is_none()) {
                        *slot = Some(LayerEntry::every(id));
                    }
                }
            }
        }
    }
}
