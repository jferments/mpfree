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
//!   the layers up with the rest, and so does the arp starting (it plays
//!   the pads through the track, where the layers can't follow). A bank
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
//!   [`App::pads_layer_synths`]). The audio loop gives each hit to the
//!   armed tracks that play a layer ([`App::pads_layer_recs`]).
//! - **Saved** in a part of its own, `pad-layers` ([`App::pad_layers_part`],
//!   `crate::project::PAD_LAYERS`), a line per layer after the first
//!   (`layer instrument 4`), written only while there are layers. The
//!   pads part keeps the first, so a build from before plays the first
//!   only.

use super::{App, Effect, PadFocus};
use crate::instrument_list::{InstrumentId, Kind};

/// The most synths the pads play at once: what they play and three more.
pub const PAD_LAYERS: usize = crate::instruments::PAD_LAYERS;

/// The layers beyond the first.
pub const MORE: usize = PAD_LAYERS - 1;

/// The pad layers part's first line ([`crate::project::PAD_LAYERS`]).
pub const LAYERS_FORMAT: &str = "mpfree-pad-layers 1";

/// The pad layers part read back ([`App::pad_layers_part`]): the layers
/// beyond the first, in order. Err says why it doesn't read: another
/// first line, a line it doesn't know, an instrument number that can't be
/// one, one said twice, more than [`MORE`]. Whether the project's list has
/// them is [`App::open_pad_layers`]'s to check.
pub fn parse_layers(text: &str) -> Result<Vec<InstrumentId>, String> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    match lines.next() {
        Some(LAYERS_FORMAT) => {}
        Some(other) => return Err(format!("its first line is \"{other}\"")),
        None => return Err("it is empty".to_string()),
    }
    let mut ids = Vec::new();
    for line in lines {
        let id = match line.split_whitespace().collect::<Vec<_>>().as_slice() {
            ["layer", "instrument", n] => n
                .parse()
                .ok()
                .and_then(InstrumentId::from_number)
                .ok_or_else(|| format!("\"{line}\" doesn't read"))?,
            _ => return Err(format!("\"{line}\" doesn't read")),
        };
        if ids.contains(&id) {
            return Err(format!("it names instrument {id} twice"));
        }
        ids.push(id);
    }
    if ids.len() > MORE {
        return Err(format!(
            "it has {} layers, and the pads play {MORE} more at most",
            ids.len()
        ));
    }
    Ok(ids)
}

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
        matches!(self.pads_focus, PadFocus::Instrument(_)) && self.pads_bank().is_none()
    }

    /// The synths the pads play beyond the first, in order, whether they
    /// play now ([`App::layers_apply`]) or wait on a bank's own.
    pub fn pad_layers(&self) -> impl Iterator<Item = InstrumentId> + '_ {
        self.pad_layers.iter().flatten().copied()
    }

    /// Whether the pads have layers.
    pub fn has_layers(&self) -> bool {
        self.pad_layers.iter().any(Option::is_some) || self.layer_to.iter().any(Option::is_some)
    }

    /// The holder in the instrument host of each layer that plays now, in
    /// order (None: no layer there, or the shared synth, which plays in
    /// the audio loop: [`App::pads_layer_synth`]). The audio loop points
    /// the host's layer players at them. Allocates nothing.
    pub fn pads_layer_holders(&self) -> [Option<usize>; MORE] {
        if !self.layers_apply() {
            return [None; MORE];
        }
        self.pad_layers
            .map(|l| l.and_then(|id| self.instrument_holder(id)))
    }

    /// Whether the shared synth plays as a layer now.
    pub fn pads_layer_synth(&self) -> bool {
        self.layers_apply() && self.pad_layers.contains(&Some(InstrumentId::SHARED_SYNTH))
    }

    /// The layers that play now, for REC (step 11c), in order: each one's
    /// name and holder in the instrument host (None for the shared synth).
    /// A layer whose free place isn't held yet is left out. Allocates
    /// nothing.
    pub fn pads_layer_synths(&self) -> [Option<(&'static str, Option<usize>)>; MORE] {
        let mut out = [None; MORE];
        if !self.layers_apply() {
            return out;
        }
        let playing = self
            .pad_layers()
            .filter_map(|id| match self.instrument_list.kind(id)? {
                Kind::SharedSynth => Some((self.instrument_name(id), None)),
                k => k.holder().map(|h| (self.instrument_name(id), Some(h))),
            });
        for (slot, l) in out.iter_mut().zip(playing) {
            *slot = Some(l);
        }
        out
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

    /// The track each layer that plays now records on (step 11c), in the
    /// order of [`App::pads_layer_synths`]: one of the other armed tracks
    /// (`also`, a bit each: `TimeHost::also`) that plays its synth, as REC
    /// sets them up (`desk.rs`, `rec_synth`). None: no layer there, or no
    /// such track. The audio loop gives them each pad hit. Allocates
    /// nothing.
    pub fn pads_layer_recs(&self, also: u128) -> [Option<u8>; MORE] {
        let mut used = 0u128;
        self.pads_layer_synths().map(|l| {
            let (_, holder) = l?;
            let t = (0..128).find(|&t| {
                also >> t & 1 == 1 && used >> t & 1 == 0 && self.track_plays_synth(t, holder)
            })?;
            used |= 1 << t;
            Some(t as u8)
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
        let PadFocus::Instrument(first) = self.pads_focus else {
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
            .chain(self.pad_layers().map(|id| self.instrument_name(id)))
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
        for (slot, id) in packed.iter_mut().zip(self.pad_layers.iter().flatten()) {
            *slot = Some(*id);
        }
        self.pad_layers = packed;
    }

    /// A layer on or off (step 11a): instrument `id` of the project's
    /// list plays on the pads with what they play, or stops, said. Up to
    /// [`PAD_LAYERS`] in all. While the pads follow a track that plays a
    /// synth, that synth is chosen first; over a kit, `id` is what they
    /// play, alone. Taking off the first gives its place to the next
    /// layer. Not while the arp plays the pads, nor on a bank with a synth
    /// of its own.
    pub fn toggle_pad_layer(&mut self, id: InstrumentId) -> Vec<Effect> {
        if self.instrument_list.kind(id).is_none() {
            return Vec::new();
        }
        if self.generating() {
            return vec![Effect::Tell(
                "Layers play the pads as pressed: let go of NOTE REPEAT or Latch, or close the Arp panel, first"
                    .into(),
            )];
        }
        if let Some(b) = self.pads_bank() {
            return vec![Effect::Tell(format!(
                "Bank {} plays its own synth alone: layers play on the banks that play what every bank plays",
                super::bank_letter(b)
            ))];
        }
        let name = self.instrument_name(id);
        let was = self.pads_focus();
        match self.pads_focus {
            PadFocus::Track => match self.track_synth_id() {
                Some(t) if t == id => {
                    return vec![Effect::Tell(format!(
                        "The pads play {name} already: add another synth to layer it"
                    ))];
                }
                Some(t) => {
                    self.pads_focus = PadFocus::Instrument(t);
                    self.pad_layers = [Some(id), None, None];
                }
                None => {
                    self.pads_focus = PadFocus::Instrument(id);
                    self.pad_layers = [None; MORE];
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
                let Some(next) = self.pad_layers[0] else {
                    return vec![Effect::Tell(format!(
                        "{name} is all the pads play: Follow track gives them back to the track"
                    ))];
                };
                self.pads_focus = PadFocus::Instrument(next);
                self.pad_layers[0] = None;
                self.pack_layers();
            }
            PadFocus::Instrument(_) if self.pad_layers.contains(&Some(id)) => {
                for l in self.pad_layers.iter_mut().filter(|l| **l == Some(id)) {
                    *l = None;
                }
                self.pack_layers();
            }
            PadFocus::Instrument(_) => {
                let Some(slot) = self.pad_layers.iter_mut().find(|l| l.is_none()) else {
                    return vec![Effect::Tell(format!(
                        "The pads play {PAD_LAYERS} synths, the most: take one off first"
                    ))];
                };
                *slot = Some(id);
            }
        }
        self.project_edit = true;
        let mut fx = Vec::new();
        if self.pads_focus() != was {
            fx.push(Effect::PadsFocus(self.pads_focus()));
        }
        let now = self
            .pads_layer_names()
            .unwrap_or_else(|| match self.pads_focus {
                PadFocus::Instrument(f) => self.instrument_name(f).to_string(),
                PadFocus::Track => "the track's instrument".to_string(),
            });
        let on = self.pad_layers().any(|l| l == id) || self.pads_focus == PadFocus::Instrument(id);
        fx.push(Effect::Note(if on {
            format!("The pads play {now} · each on its own strip in the mixer")
        } else {
            format!("{name} is off the pads · they play {now}")
        }));
        fx
    }

    /// The layers given up (Follow track, the arp, a project opened).
    pub(super) fn clear_layers(&mut self) {
        self.pad_layers = [None; MORE];
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
    /// free place given a layer's synth has its id now, and a layer whose
    /// synth has gone is taken off, said. Allocates only to say so.
    pub(super) fn layers_follow_list(&mut self) -> Vec<Effect> {
        let mut fx = Vec::new();
        for l in 0..MORE {
            if let Some(id) = self.layer_to[l].and_then(|n| self.instrument_list.free_of(n)) {
                self.layer_to[l] = None;
                if self.pads_focus != PadFocus::Instrument(id)
                    && !self.pad_layers.contains(&Some(id))
                {
                    if let Some(slot) = self.pad_layers.iter_mut().find(|s| s.is_none()) {
                        *slot = Some(id);
                    }
                }
            }
        }
        let gone = self
            .pad_layers()
            .filter(|&id| self.instrument_list.kind(id).is_none())
            .count();
        if gone > 0 {
            for l in self.pad_layers.iter_mut() {
                if l.is_some_and(|id| self.instrument_list.kind(id).is_none()) {
                    *l = None;
                }
            }
            self.pack_layers();
            self.project_edit = true;
            fx.push(Effect::Note(if gone == 1 {
                "A layer's synth is gone: the pads play the others".to_string()
            } else {
                format!("{gone} layers' synths are gone: the pads play the others")
            }));
        }
        fx
    }

    /// The pad layers part ([`parse_layers`] reads it): its first line,
    /// then `layer instrument 4` for each layer beyond the first. Empty
    /// while there are none, so a project without stays as it was.
    pub fn pad_layers_part(&self) -> String {
        if self.pad_layers().next().is_none() {
            return String::new();
        }
        let mut text = format!("{LAYERS_FORMAT}\n");
        for id in self.pad_layers() {
            text.push_str(&format!("layer instrument {id}\n"));
        }
        text
    }

    /// A project's pad layers part, after its pads part
    /// ([`App::open_pads`]) and its list (None: no layers). A part that
    /// doesn't read, or a project whose pads follow the track, opens with
    /// no layers, said; a layer the list hasn't got, or the one the pads
    /// play first, is left out, said. Gives the lines for the log.
    pub fn open_pad_layers(&mut self, text: Option<&str>) -> Vec<String> {
        self.clear_layers();
        let Some(text) = text else {
            return Vec::new();
        };
        let ids = match parse_layers(text) {
            Ok(ids) => ids,
            Err(why) => {
                return vec![format!(
                    "project: the pads' layers weren't opened ({why}); the pads play one synth"
                )]
            }
        };
        let PadFocus::Instrument(first) = self.pads_focus else {
            return vec![
                "project: the pads' layers weren't opened (the pads follow the track)".to_string(),
            ];
        };
        let mut notes = Vec::new();
        for (l, id) in ids.into_iter().enumerate() {
            if self.instrument_list.kind(id).is_none() {
                notes.push(format!(
                    "project: layer instrument {id} isn't in the project's list; the pads play the others"
                ));
            } else if id == first {
                notes.push(format!(
                    "project: layer instrument {id} is what the pads play first; left out"
                ));
            } else {
                self.pad_layers[l] = Some(id);
            }
        }
        self.pack_layers();
        notes
    }
}
