//! **What the pads play** (sampler step 15e; the canvas "Pads and kits:
//! step 15e", board 3; Jay's "Build as drawn, with your picks" of 23 Nov
//! 2026 16:11Z, question 1: "a tap on the Pads chip opens What the pads
//! play, whose first row holds Follow track"): the whole pad map at once,
//! over the Synth tab's page, where the list of synths opens.
//!
//! - **Opens** with a tap on the header's "Pads: Big Kit" chip, on any
//!   page: the Synth tab shows with the panel over it
//!   ([`App::open_pad_map`]). Done, the chip again, or another tab closes
//!   it.
//! - **The base** (every bank): Follow track, which gives every bank and
//!   layer back to the track as the chip's tap did before, and Pick, which
//!   opens the list for every bank.
//! - **Each bank**: a tap on its row opens the list for that bank alone,
//!   whichever bank the pads are on ([`SynthPick`]'s `for_bank`); Follow
//!   base gives the bank back to the base; a missing one has Replace (the
//!   list) and Remove.
//! - **Each layer**: where it plays (every bank, or the bank the pads are
//!   on: a tap moves it), and Remove; + Add a layer opens the list with
//!   Layer on.
//! - **UNDO** takes back the last change to the pad map, while the panel
//!   is open: each pick kept in the list, Follow and Remove is one step
//!   ([`App::pad_map_undo`]), up to [`MAP_UNDO`] of them. Nothing else
//!   changes what the pads play (step 15b's one rule), so the history is
//!   only these; a project opened starts it again.
//!
//! The list opened from a row closes back to the panel (Keep and Cancel
//! both). The app says what each row plays by instrument ([`MapSound`]);
//! the screen names the kits (gui.rs, `Player::pad_map_panel`).

use super::pad_layers::{MORE, PAD_LAYERS};
use super::synth_pick::SynthPick;
use super::{App, Effect, PadFocus, PadMap, View, KIT_PAGES};
use crate::instrument_list::{InstrumentId, Kind};

/// The most pad map changes UNDO takes back.
pub const MAP_UNDO: usize = 32;

/// What an entry of the map plays, as the panel says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapSound {
    /// The selected track's instrument (the base's Follow track; a bank's
    /// own "the track").
    Track,
    /// The kit in this kit place.
    Kit(usize),
    /// A synth, by name ("Poly", "FM", a plugin's).
    Synth(&'static str),
    /// An instrument the project's list hasn't got.
    Missing(InstrumentId),
}

/// A layer as the panel shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapLayer {
    pub sound: MapSound,
    /// The bank it plays on alone; None: every bank.
    pub reach: Option<u8>,
    /// A kit layer's source bank.
    pub from: Option<u8>,
}

/// The last change UNDO takes back, as the panel's UNDO box says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapChange {
    /// The base plays this now (Track: it follows the track again).
    Base(MapSound),
    /// Bank `b` got this (with its source bank), or (None) plays the base
    /// again.
    Bank(usize, Option<(MapSound, Option<u8>)>),
    /// A layer added.
    LayerOn(MapSound),
    /// A layer taken off.
    LayerOff(MapSound),
    /// A layer moved: every bank (None) or one.
    LayerReach(MapSound, Option<u8>),
    /// Follow track: the whole map given back.
    FollowTrack,
    /// More than one thing at once (a list kept after several taps).
    Several,
}

/// The panel as the screen draws it, while it is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PadMapView {
    /// The selected track (0 based), whose instrument Follow track plays.
    pub track: usize,
    pub base: MapSound,
    /// Bank A to H's own entry, with a kit's source bank.
    pub banks: [Option<(MapSound, Option<u8>)>; KIT_PAGES],
    /// The bank the pads are on.
    pub current: usize,
    /// A bank's own plays now (a kit's pages); else PAD BANK picks the
    /// shared synth's layouts and the banks' own wait.
    pub banks_apply: bool,
    pub layers: Vec<MapLayer>,
    /// The most layers, the first counted ("LAYERS · 2 OF 4").
    pub most: usize,
    /// What UNDO takes back; None: nothing.
    pub last: Option<MapChange>,
}

/// A row of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapRow {
    Base,
    Bank(usize),
    Layer(usize),
}

/// What a tap on the panel does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapAct {
    /// The row itself, Pick, or Replace: the list, for that row.
    Pick,
    /// The base's Follow track.
    FollowTrack,
    /// A bank's Follow base, or a missing bank's Remove.
    FollowBase,
    /// A layer's Every bank / Bank D.
    Reach,
    /// A layer's Remove.
    Remove,
    /// + Add a layer.
    AddLayer,
    Done,
}

impl App {
    /// What `f` plays, as the panel says it.
    fn map_sound(&self, f: PadFocus) -> MapSound {
        match f {
            PadFocus::Track => MapSound::Track,
            PadFocus::Instrument(id) => self.id_sound(id),
        }
    }

    fn id_sound(&self, id: InstrumentId) -> MapSound {
        match self.instrument_list.kind(id) {
            None => MapSound::Missing(id),
            Some(Kind::Kit { kit }) => MapSound::Kit(kit),
            Some(_) => MapSound::Synth(self.instrument_name(id)),
        }
    }

    /// The change from `was` to the map now, as the UNDO box says it.
    fn map_change(&self, was: &PadMap) -> MapChange {
        let now = &self.pad_map;
        if *now == PadMap::default() {
            return MapChange::FollowTrack;
        }
        let base = now.base != was.base;
        let banks: Vec<usize> = (0..KIT_PAGES)
            .filter(|&b| now.banks[b] != was.banks[b])
            .collect();
        let layers = now.layers != was.layers;
        match (base, banks.as_slice(), layers) {
            (true, [], false) => MapChange::Base(self.map_sound(now.base)),
            (false, &[b], false) => {
                MapChange::Bank(b, now.banks[b].map(|e| (self.map_sound(e.play), e.from)))
            }
            (false, [], true) => {
                let ids = |m: &PadMap| m.layers.iter().flatten().map(|l| l.id).collect::<Vec<_>>();
                let (n, w) = (ids(now), ids(was));
                if let Some(&id) = n.iter().find(|id| !w.contains(id)) {
                    return MapChange::LayerOn(self.id_sound(id));
                }
                if let Some(&id) = w.iter().find(|id| !n.contains(id)) {
                    return MapChange::LayerOff(self.id_sound(id));
                }
                now.layers
                    .iter()
                    .flatten()
                    .find(|l| !was.layers.contains(&Some(**l)))
                    .map_or(MapChange::Several, |l| {
                        MapChange::LayerReach(self.id_sound(l.id), l.reach)
                    })
            }
            // A layer added over a kit (the base and the layers at once),
            // or a list kept after taps on more than one row.
            _ => MapChange::Several,
        }
    }

    /// The panel as the screen draws it, while it is open.
    pub(super) fn pad_map_view(&self) -> Option<PadMapView> {
        if !self.map_panel {
            return None;
        }
        let m = &self.pad_map;
        Some(PadMapView {
            track: self.track,
            base: self.map_sound(m.base),
            banks: std::array::from_fn(|b| m.banks[b].map(|e| (self.map_sound(e.play), e.from))),
            current: self.pad_page,
            banks_apply: self.banks_apply(),
            layers: m
                .layers
                .iter()
                .flatten()
                .map(|l| MapLayer {
                    sound: self.id_sound(l.id),
                    reach: l.reach,
                    from: l.from,
                })
                .collect(),
            most: PAD_LAYERS,
            last: self.map_undo.last().map(|was| self.map_change(was)),
        })
    }

    /// The panel's effect: its view, or None (closed).
    pub(super) fn pad_map_fx(&self) -> Effect {
        Effect::PadMap(self.pad_map_view())
    }

    /// A step of the map's history: the map before a change made here
    /// (a pick kept, Follow, Remove). Nothing when it didn't change.
    pub(super) fn map_step(&mut self, was: PadMap) {
        let waiting = self.pads_to.is_some()
            || self.bank_to.iter().any(Option::is_some)
            || self.layer_to.iter().any(Option::is_some);
        if was == self.pad_map && !waiting {
            return;
        }
        if self.map_undo.len() == MAP_UNDO {
            self.map_undo.remove(0);
        }
        self.map_undo.push(was);
    }

    /// A project opened: the history starts again.
    pub(super) fn map_history_clear(&mut self) {
        self.map_undo.clear();
    }

    /// A tap on the header's Pads chip: What the pads play opens over the
    /// Synth tab, from any page; open, it closes.
    pub fn open_pad_map(&mut self) -> Vec<Effect> {
        if self.map_panel && self.view == View::Synth {
            return self.close_pad_map();
        }
        let mut fx = Vec::new();
        if self.synth_pick.is_some() {
            fx.extend(self.close_synth_pick(true));
        }
        if self.view != View::Synth {
            fx.extend(self.show(View::Synth));
        }
        self.map_panel = true;
        fx.push(self.pad_map_fx());
        fx
    }

    /// Done (or the chip again): the panel closes.
    pub fn close_pad_map(&mut self) -> Vec<Effect> {
        if !self.map_panel {
            return Vec::new();
        }
        self.map_panel = false;
        vec![Effect::PadMap(None)]
    }

    /// The list, opened from the panel for `row` (the base, a bank alone,
    /// or a layer to add). It closes back to the panel.
    fn map_pick(&mut self, row: MapRow) -> Vec<Effect> {
        let bank = match row {
            MapRow::Bank(b) => Some(b),
            _ => None,
        };
        if bank.is_some() && !self.banks_apply() {
            return vec![Effect::Tell(
                "PAD BANK picks the synth's layouts while the track plays it: a bank's own plays on a kit's pages".to_string(),
            )];
        }
        self.map_panel = false;
        let mut fx = vec![Effect::PadMap(None)];
        fx.extend(self.open_synth_pick_for(bank, matches!(row, MapRow::Layer(_))));
        if let Some(p) = self.synth_pick.as_mut() {
            p.from_map = true;
        }
        fx
    }

    /// The list closed (Keep or Cancel) after the panel opened it: the
    /// panel again.
    pub(super) fn back_to_map(&mut self, pick: &SynthPick) -> Vec<Effect> {
        if !pick.from_map || self.view != View::Synth {
            return Vec::new();
        }
        self.map_panel = true;
        vec![self.pad_map_fx()]
    }

    /// A tap on the panel: `act` on `row`.
    pub fn pad_map_tap(&mut self, row: MapRow, act: MapAct) -> Vec<Effect> {
        if !self.map_panel {
            return Vec::new();
        }
        let was = self.pad_map;
        let mut fx = match (row, act) {
            (_, MapAct::Done) => return self.close_pad_map(),
            (_, MapAct::AddLayer) => return self.map_pick(MapRow::Layer(MORE)),
            (row, MapAct::Pick) => return self.map_pick(row),
            (MapRow::Base, MapAct::FollowTrack) => self.pads_follow_track(),
            (MapRow::Bank(b), MapAct::FollowBase) if b < KIT_PAGES => self.map_follow_base(b),
            (MapRow::Layer(l), MapAct::Reach) if l < MORE => self.map_layer_reach(l),
            (MapRow::Layer(l), MapAct::Remove) if l < MORE => self.map_layer_off(l),
            _ => Vec::new(),
        };
        self.map_step(was);
        fx.push(self.pad_map_fx());
        fx
    }

    /// Bank `b` plays the base again, its own entry (a missing one too)
    /// given up.
    fn map_follow_base(&mut self, b: usize) -> Vec<Effect> {
        if self.pad_map.banks[b].is_none() && self.bank_to[b].is_none() {
            return Vec::new();
        }
        let was = self.pads_focus();
        self.pad_map.banks[b] = None;
        self.bank_to[b] = None;
        self.project_edit = true;
        let mut fx = Vec::new();
        if self.pads_focus() != was {
            fx.push(Effect::PadsFocus(self.pads_focus()));
        }
        fx.push(Effect::Note(format!(
            "Bank {} plays what every bank plays again",
            super::bank_letter(b)
        )));
        fx
    }

    /// Layer `l` plays on every bank, or on the bank the pads are on.
    fn map_layer_reach(&mut self, l: usize) -> Vec<Effect> {
        let Some(e) = self.pad_map.layers[l] else {
            return Vec::new();
        };
        let page = self.pad_page.min(KIT_PAGES - 1) as u8;
        let reach = match e.reach {
            Some(_) => None,
            None => Some(page),
        };
        self.pad_map.layers[l] = Some(super::LayerEntry { reach, ..e });
        self.project_edit = true;
        let name = self.instrument_name(e.id);
        vec![
            Effect::PadsFocus(self.pads_focus()),
            Effect::Note(match reach {
                None => format!("{name} plays on every bank"),
                Some(b) => format!(
                    "{name} plays on bank {} only",
                    super::bank_letter(usize::from(b))
                ),
            }),
        ]
    }

    /// Layer `l` off the pads (a missing one too).
    fn map_layer_off(&mut self, l: usize) -> Vec<Effect> {
        let Some(e) = self.pad_map.layers[l] else {
            return Vec::new();
        };
        let name = match self.id_sound(e.id) {
            MapSound::Missing(id) => format!("Instrument {id}"),
            _ => self.instrument_name(e.id).to_string(),
        };
        self.pad_map.layers[l] = None;
        self.pack_layers();
        self.project_edit = true;
        vec![
            Effect::PadsFocus(self.pads_focus()),
            Effect::Note(format!("{name} is off the pads")),
        ]
    }

    /// UNDO with the panel open: the map as it was before the last change
    /// made here, said. Nothing to take back: said.
    pub fn pad_map_undo(&mut self) -> Vec<Effect> {
        let Some(was) = self.map_undo.pop() else {
            return vec![Effect::Tell(
                "Nothing to undo here: no pick, Follow or Remove since the project opened"
                    .to_string(),
            )];
        };
        let change = self.map_change(&was);
        self.pad_map = was;
        self.pads_to = None;
        self.bank_to = [None; KIT_PAGES];
        self.layer_to = [None; MORE];
        self.project_edit = true;
        let what = match change {
            MapChange::FollowTrack => "Follow track".to_string(),
            MapChange::Base(_) => "the base's pick".to_string(),
            MapChange::Bank(b, _) => format!("bank {}'s change", super::bank_letter(b)),
            MapChange::LayerOn(_) | MapChange::LayerOff(_) | MapChange::LayerReach(..) => {
                "the layer's change".to_string()
            }
            MapChange::Several => "the last pick".to_string(),
        };
        vec![
            Effect::PadsFocus(self.pads_focus()),
            Effect::Note(format!("UNDO: {what} taken back")),
            self.pad_map_fx(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{BankEntry, LayerEntry};

    /// An app on the Synth tab with four kits in its list.
    fn app() -> App {
        let mut a = App::new(44_100, false);
        let _ = a.start();
        let mut list = crate::instrument_list::InstrumentList::new();
        list.follow_kits(4);
        let _ = a.set_instrument_list(&list);
        let _ = a.open_tab(View::Synth);
        a
    }

    fn kit(a: &App, k: usize) -> InstrumentId {
        a.instrument_list.of_kit(k).expect("a kit's id")
    }

    /// The chip opens the panel on the Synth tab from another page; Done
    /// closes it.
    #[test]
    fn the_chip_opens_the_panel_over_the_synth_tab() {
        let mut a = app();
        a.view = View::Play;
        let fx = a.open_pad_map();
        assert_eq!(a.view, View::Synth);
        assert!(fx.contains(&Effect::Show(View::Synth)), "{fx:?}");
        let Some(Effect::PadMap(Some(v))) = fx.last() else {
            panic!("{fx:?}")
        };
        assert_eq!(v.base, MapSound::Track);
        assert!(v.banks.iter().all(Option::is_none));
        assert_eq!(v.last, None);
        assert_eq!(
            a.pad_map_tap(MapRow::Base, MapAct::Done),
            vec![Effect::PadMap(None)]
        );
        assert!(a.pad_map_view().is_none());
    }

    /// Follow base gives a bank back, one step of the history; UNDO with
    /// the panel open takes it back, and says so.
    #[test]
    fn follow_base_is_one_step_and_undo_takes_it_back() {
        let mut a = app();
        let big = kit(&a, 1);
        a.pad_map.base = PadFocus::Instrument(kit(&a, 0));
        a.pad_map.banks[1] = Some(BankEntry {
            play: PadFocus::Instrument(big),
            from: Some(2),
        });
        let _ = a.open_pad_map();
        let v = a.pad_map_view().unwrap();
        assert_eq!(v.banks[1], Some((MapSound::Kit(1), Some(2))));
        let fx = a.pad_map_tap(MapRow::Bank(1), MapAct::FollowBase);
        assert_eq!(a.pad_map.banks[1], None);
        let Some(Effect::PadMap(Some(v))) = fx.last() else {
            panic!("{fx:?}")
        };
        assert_eq!(v.last, Some(MapChange::Bank(1, None)));
        let fx = a.pad_map_undo();
        assert_eq!(
            a.pad_map.banks[1],
            Some(BankEntry {
                play: PadFocus::Instrument(big),
                from: Some(2)
            })
        );
        assert!(
            fx.contains(&Effect::Note("UNDO: bank B's change taken back".into())),
            "{fx:?}"
        );
        assert!(matches!(a.pad_map_undo().as_slice(), [Effect::Tell(_)]));
    }

    /// A layer moves between every bank and the bank the pads are on, and
    /// Remove takes a missing one off.
    #[test]
    fn a_layer_moves_to_one_bank_and_a_missing_one_comes_off() {
        let mut a = app();
        a.pad_map.base = PadFocus::SYNTH;
        let gone = InstrumentId::from_number(60).unwrap();
        a.pad_map.layers = [
            Some(LayerEntry::every(kit(&a, 2))),
            Some(LayerEntry::every(gone)),
            None,
        ];
        a.pad_page = 3;
        let _ = a.open_pad_map();
        let _ = a.pad_map_tap(MapRow::Layer(0), MapAct::Reach);
        assert_eq!(a.pad_map.layers[0].unwrap().reach, Some(3));
        let v = a.pad_map_view().unwrap();
        assert_eq!(
            v.last,
            Some(MapChange::LayerReach(MapSound::Kit(2), Some(3)))
        );
        assert_eq!(v.layers[1].sound, MapSound::Missing(gone));
        let _ = a.pad_map_tap(MapRow::Layer(1), MapAct::Remove);
        assert_eq!(a.pad_map.layers[1], None);
        assert_eq!(
            a.pad_map_view().unwrap().last,
            Some(MapChange::LayerOff(MapSound::Missing(gone)))
        );
        let _ = a.pad_map_undo();
        let _ = a.pad_map_undo();
        assert_eq!(a.pad_map.layers[0].unwrap().reach, None);
        assert_eq!(a.pad_map.layers[1].unwrap().id, gone);
    }

    /// The base's Follow track gives the whole map back, and UNDO the map.
    #[test]
    fn follow_track_on_the_base_row_is_undone_whole() {
        let mut a = app();
        let map = PadMap {
            base: PadFocus::SYNTH,
            banks: std::array::from_fn(|b| {
                (b == 4).then(|| {
                    BankEntry::of(PadFocus::Instrument(InstrumentId::from_number(60).unwrap()))
                })
            }),
            layers: [None; MORE],
        };
        a.pad_map = map;
        let _ = a.open_pad_map();
        let _ = a.pad_map_tap(MapRow::Base, MapAct::FollowTrack);
        assert_eq!(a.pad_map, PadMap::default());
        assert_eq!(a.pad_map_view().unwrap().last, Some(MapChange::FollowTrack));
        let _ = a.pad_map_undo();
        assert_eq!(a.pad_map, map);
    }

    /// A bank's row opens the list for that bank alone, whichever bank the
    /// pads are on; Cancel comes back to the panel with nothing changed.
    #[test]
    fn a_banks_row_opens_the_list_for_that_bank() {
        let mut a = app();
        a.pad_map.base = PadFocus::Instrument(kit(&a, 0));
        a.pad_page = 1;
        let _ = a.open_pad_map();
        let fx = a.pad_map_tap(MapRow::Bank(4), MapAct::Pick);
        assert!(fx.contains(&Effect::PadMap(None)), "{fx:?}");
        let v = a.synth_pick_view().expect("the list");
        assert_eq!(v.bank, Some(('E', true)));
        let _ = a.synth_pick_side(true);
        let _ = a.pick_pads_kit(Some(2), "Kit 3", false, 0b1);
        assert_eq!(
            a.pad_map.banks[4],
            Some(BankEntry {
                play: PadFocus::Instrument(kit(&a, 2)),
                from: Some(0)
            })
        );
        assert_eq!(a.pad_map.banks[1], None, "not the bank the pads are on");
        let fx = a.close_synth_pick(false);
        assert_eq!(a.pad_map.banks[4], None);
        assert!(matches!(fx.last(), Some(Effect::PadMap(Some(_)))), "{fx:?}");
        assert_eq!(a.pad_map_view().unwrap().last, None, "Cancel is no step");
        // Keep is one.
        let _ = a.pad_map_tap(MapRow::Bank(4), MapAct::Pick);
        let _ = a.pick_pads_kit(Some(2), "Kit 3", false, 0b1);
        let _ = a.close_synth_pick(true);
        assert!(matches!(
            a.pad_map_view().unwrap().last,
            Some(MapChange::Bank(4, Some((MapSound::Kit(2), Some(0)))))
        ));
    }
}
