//! The chain editor and the macros panel (`mpfree-chain-ui`) in the app:
//! what their actions do to the mixer and the mod engine, and the states
//! they show, read straight from the mixer's chains and the engine.
//!
//! The panels go on the app's screens after round 9 and the app's rename
//! (layout spec section 4b, approved 2026-11-11 22:34Z): the mixer's insert
//! row opens [`ChainHost::from_mixer`] on its slot with "‹ Mixer", the
//! mixer's "+" opens it with the picker up, the Pads tab's Effects row
//! opens it on the pad's own channel ([`Panels::open_pad_mix`]), and a
//! long press on a macro card opens the macros panel ([`MacrosHost`]).
//! The mixer's insert row, its "+" and the long press are wired (round
//! 10): stutters and repeats on one pad are Perform FX on its own
//! channel, bus effects the Drums track's chain with a macro as their one
//! knob.
//!
//! What each action does:
//!
//! - Chain edits go to the mixer: `Edit::Insert` (the effect made by
//!   `mpfree_fx_ui::make`, at the end of the chain; in the app it is made
//!   on the builder thread and goes in a few periods later,
//!   `MixHost::add_insert`), `Edit::Move`,
//!   `Edit::Remove` (the engine forgets the removed effect's parameters),
//!   and bypass with `Mixer::set_on`. The editor keeps the same effect
//!   selected across a move, and selects an added one when it lands.
//! - A knob turn goes to the engine by `ModEvent::hand`'s rule when the
//!   engine holds the parameter (a `Set`, or a `Touch` while recording,
//!   with `Untouch` when the finger lifts), else straight to the mixer.
//! - Putting a knob on a macro maps it from where it is now to the far end
//!   of its travel (Linear; the macros panel's target editor changes it).
//!   A knob already on another macro moves with its range. Matrix rows
//!   start at 25 % depth. These, the macros panel's maps and scenes go to
//!   the engine as `Change`s ([`ModMix::change`]), so the parameter becomes
//!   a target then.
//! - Learn, the full effect page and Back are for the app ([`ChainOut`]).
//! - A sidechain picked in the sidechain list goes to the mixer
//!   (`Mixer::set_key`) and turns the effect's control on; Off turns it
//!   off and keeps the strip fed (layout spec 4b, "Sidechain").
//!
//! Building a state, adding an effect (its node allocates) and removing one
//! allocate; they run between periods, as the mixer panel's edits do.

// `ChainHost::for_pad` and the audio-thread picture are for later; the
// tests use them now.
#![allow(dead_code)]

use mpfree_chain_ui::{ChainAction, ChainState, MacroAction, MacrosState, NoteAction, NotesState};
use mpfree_fx::{strip_params, Edit, StripId};
use mpfree_mod::{Change, MacroMap, MapCurve, Row};
use mpfree_node::{Held, NodeEvent, Timed};
use mpfree_notefx::{Fx, Item, TrackFx};
use mpfree_play::mpfree_param::{Owner, ParamAddr, Strip};

use std::time::{Duration, Instant};

use mpfree_mixer_ui::MixView;

use crate::app::{App, Effect, View};
use crate::fxmodes::Switched;
use crate::gui::ToUi;
use crate::hand::Hand;
use crate::mixing::{self, Captured, MixHost, SLOTS};
use crate::modmix::{strip_id, strip_of};
use crate::time::TimeHost;
#[cfg(test)]
use crate::PERIOD;

/// A new matrix row's depth.
pub const ROW_DEPTH: f32 = 0.25;

/// What the app does after a chain editor action, past the mixer and the
/// engine (which [`ChainHost::action`] has already told).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChainOut {
    /// Show the new state.
    Shown,
    /// Open insert `slot`'s full effect page (`mpfree_fx_ui::FxPanel`).
    OpenPage { strip: StripId, slot: usize },
    /// Arm learn: the next Q-Link turned takes `addr`
    /// (`Bindings::learn` with `mpfree_chain_ui::QLINK_PAGE`); call
    /// [`ChainHost::learned`] after.
    Learn(ParamAddr),
    /// Back to the screen the editor took the place of.
    Back,
    /// The strip list's pick: that strip's chain in the editor's place.
    OpenStrip(StripId),
    /// The action couldn't be done; the reason, for the hint line.
    Refused(&'static str),
}

/// The chain editor's side in the app: which chain it shows and what is
/// selected on it.
#[derive(Clone, Debug, PartialEq)]
pub struct ChainHost {
    /// The chains it can switch between (one from the mixer; the pad's and
    /// the track's from [`ChainHost::for_pad`]), and the one shown.
    scopes: Vec<(Strip, String)>,
    /// The strip list's group while it is open (the strip button).
    listing: Option<usize>,
    /// The sidechain list's group while it is open (the selected effect's
    /// Sidechain row, or a Ducker just added).
    sidechain_list: Option<usize>,
    scope: usize,
    selected: Option<usize>,
    group: usize,
    focus: Option<ParamAddr>,
    back: Option<String>,
    picking: bool,
    learning: bool,
    /// The chain's slot ids when an effect was added: the one not among
    /// them is the new effect, selected when it lands.
    landing: Option<Vec<u16>>,
    /// The open effect was removed: its page shut, and the editor shows
    /// the chain's list with nothing picked until an effect is tapped
    /// (otherwise no selection means the first effect's page).
    page_shut: bool,
    /// A note track's Notes stage: the row whose page shows (then no sound
    /// effect's does), and whether the MIDI effect picker is up.
    notes_row: Option<usize>,
    notes_picking: bool,
}

/// What a Notes stage action leaves for the app ([`ChainHost::note_action`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteOut {
    /// Show the new state.
    Shown,
    /// The track's MIDI effects changed: put them in the project.
    Changed,
    /// The Plays on row: the MIDI section's Tracks out.
    PlaysOn,
    /// The arp's page: its panel, on this track.
    OpenArp,
    /// It couldn't be done; why, for the hint line.
    Refused(&'static str),
}

/// What the app knows of a note track for its Notes stage: its MIDI
/// effects, and its arp's line and whether it plays.
#[derive(Clone, Debug, PartialEq)]
pub struct NotesSource {
    pub fx: TrackFx,
    pub arp: String,
    pub arp_on: bool,
}

impl NotesSource {
    /// Track `t`'s, from the project and the app's arps; None for a Loop
    /// track, an audio track or past the project's tracks.
    pub fn of(app: &App, time: &crate::time::TimeHost, t: usize) -> Option<NotesSource> {
        let track = time.project().tracks.get(t)?;
        if !track.plays_notes() {
            return None;
        }
        let s = app.arps.engine_at(t).settings();
        use mpfree_arp::Mode;
        let arp = match s.mode {
            Mode::Direct => "Off".to_string(),
            Mode::Repeat => format!("Repeat · {}", s.rate.label()),
            Mode::Arp => format!(
                "{} · {} · {} oct",
                s.order.label(),
                s.rate.label(),
                s.octaves
            ),
            Mode::Pattern => format!("Pattern · {}", s.rate.label()),
        };
        Some(NotesSource {
            fx: track.note_fx,
            arp,
            arp_on: s.mode != Mode::Direct,
        })
    }
}

impl ChainHost {
    fn new(scopes: Vec<(Strip, String)>, back: Option<String>) -> ChainHost {
        ChainHost {
            scopes,
            listing: None,
            sidechain_list: None,
            scope: 0,
            selected: None,
            group: 0,
            focus: None,
            back,
            picking: false,
            learning: false,
            landing: None,
            page_shut: false,
            notes_row: None,
            notes_picking: false,
        }
    }

    /// The sequencer track whose strip is shown (Drums is track 1's, an
    /// own channel its track's: `mpfree_param::strip_track`), for its
    /// Notes stage. Synth, Loop and Audio are no track's.
    pub fn track(&self) -> Option<usize> {
        mpfree_play::mpfree_param::strip_track(self.strip()).map(usize::from)
    }

    /// A Notes stage action on the track's MIDI effects `fx` (the
    /// project's; the app puts them back on [`NoteOut::Changed`]).
    pub fn note_action(&mut self, a: &NoteAction, fx: &mut TrackFx) -> NoteOut {
        let rows = fx.items().len();
        match *a {
            NoteAction::Select(row) if row < rows => {
                self.notes_row = Some(row);
                self.notes_picking = false;
                self.picking = false;
                self.selected = None;
                self.focus = None;
            }
            NoteAction::Select(_) => {}
            NoteAction::SetOn { row, on } => {
                let Some(Item::Fx(p)) = fx.items().get(row).copied() else {
                    return NoteOut::Shown;
                };
                match fx.slots[p].as_mut() {
                    Some(s) if s.on != on => s.on = on,
                    _ => return NoteOut::Shown,
                }
                return NoteOut::Changed;
            }
            NoteAction::Move { from, to } => {
                if !fx.move_row(from, to) {
                    return NoteOut::Shown;
                }
                if self.notes_row == Some(from) {
                    self.notes_row = Some(to);
                }
                return NoteOut::Changed;
            }
            NoteAction::Remove(row) => {
                if !fx.remove(row) {
                    return NoteOut::Refused("The arp stays: move it with Up and Down");
                }
                // Its page shuts, as a removed sound effect's does.
                self.notes_row = None;
                self.page_shut = true;
                return NoteOut::Changed;
            }
            NoteAction::Picker(on) => {
                self.notes_picking = on;
                if on {
                    self.notes_row = None;
                    self.picking = false;
                }
            }
            NoteAction::Add(i) => {
                let Some(f) = Fx::all().get(i).copied() else {
                    return NoteOut::Shown;
                };
                let Some(row) = fx.add(f) else {
                    return NoteOut::Refused("Eight MIDI effects already: take one out first");
                };
                self.notes_picking = false;
                self.notes_row = Some(row);
                self.selected = None;
                return NoteOut::Changed;
            }
            NoteAction::Set { id, k, value } => {
                let Some(s) = fx.place_of(id).and_then(|p| fx.slots[p].as_mut()) else {
                    return NoteOut::Shown;
                };
                let was = s.fx;
                s.fx.set(k, value);
                return if s.fx == was {
                    NoteOut::Shown
                } else {
                    NoteOut::Changed
                };
            }
            NoteAction::PlaysOn => return NoteOut::PlaysOn,
            NoteAction::OpenArp => return NoteOut::OpenArp,
        }
        NoteOut::Shown
    }

    /// The Notes stage on `st` for a note track (`src`): its rows, the one
    /// whose page shows (its settings as the Q-Links' parameters), the
    /// picker. Plays on is the screen's to fill (it holds the MIDI set-up).
    pub fn dress_notes(
        &mut self,
        st: &mut ChainState,
        src: Option<NotesSource>,
        engine: &mpfree_mod::ModEngine,
    ) {
        let (Some(src), Some(t)) = (src, self.track()) else {
            self.notes_row = None;
            self.notes_picking = false;
            return;
        };
        let rows = src.fx.items().len();
        self.notes_row = self.notes_row.filter(|&r| r < rows);
        let ns = NotesState {
            track: t as u8,
            fx: src.fx,
            arp: src.arp,
            arp_on: src.arp_on,
            selected: self.notes_row,
            picking: self.notes_picking,
            plays_on: String::new(),
            plays_detail: String::new(),
            ccs_send_nothing: None,
        };
        if ns.picking || self.notes_row.is_some() {
            *st = std::mem::take(st).without_page();
        }
        if let Some((_, id, fx)) = ns.page_fx().filter(|_| !ns.picking) {
            (st.params, st.main_four) = mpfree_chain_ui::notes::param_views(ns.track, id, &fx);
            // What the mod engine does to them: a macro, a ring for the
            // matrix rows, a lock; and the setting touched last, with its
            // rows, for the Modulate row.
            for p in st.params.iter_mut().chain(st.main_four.iter_mut()) {
                p.macro_ = engine.macro_of(p.addr);
                p.mod_range = engine.mod_range(p.addr);
                p.locked = engine.is_locked(p.addr);
            }
            if let Some(f) = self
                .focus
                .filter(|f| st.params.iter().any(|p| p.addr == *f))
            {
                st.focus = Some(f);
                st.rows = engine
                    .rows()
                    .enumerate()
                    .filter(|(_, r)| r.target == f)
                    .map(|(i, r)| mpfree_chain_ui::state::RowView {
                        index: i,
                        source: mpfree_chain_ui::state::source_name(r.source),
                        depth: r.depth,
                    })
                    .collect();
            }
        }
        st.notes = Some(ns);
    }

    /// From the mixer: its insert row (`MixAction::OpenInsert`, `slot`) or
    /// its "+" (`MixAction::AddInsert`, `slot` None: the picker opens).
    /// None for a strip past what an address can name.
    pub fn from_mixer(strip: StripId, slot: Option<usize>) -> Option<ChainHost> {
        let s = strip_of(strip)?;
        let mut h = ChainHost::new(vec![(s, mixing::name(strip))], Some("Mixer".into()));
        h.selected = slot;
        h.picking = slot.is_none();
        Some(h)
    }

    /// Its first scope named as the mixer names `strip` (a lent pad
    /// channel by its pad and kit: "B5 · Kit 2").
    fn named(mut self, mix: &MixHost, strip: StripId) -> ChainHost {
        if let Some(s) = self.scopes.first_mut() {
            s.1 = mix.strip_name(strip);
        }
        self
    }

    /// A pad's own chain first (its pad channel `pad`, named `name`), and
    /// the track's it plays into. No screen opens it so now: the Pads
    /// tab's Effects row opens the pad's chain alone
    /// ([`Panels::open_pad_mix`]).
    pub fn for_pad(pad: StripId, name: String, track: usize) -> Option<ChainHost> {
        let t = StripId::Track(track);
        let scopes = vec![(strip_of(pad)?, name), (strip_of(t)?, mixing::name(t))];
        Some(ChainHost::new(scopes, None))
    }

    pub fn strip(&self) -> Strip {
        self.scopes[self.scope].0
    }

    fn sid(&self) -> StripId {
        strip_id(self.strip())
    }

    /// Learn was armed and a Q-Link took the knob (or learn was dropped).
    pub fn learned(&mut self) {
        self.learning = false;
    }

    /// What the editor shows now. Selects an added effect once it is in.
    pub fn state(&mut self, h: &MixHost) -> ChainState {
        let sid = self.sid();
        if let (Some(before), Some(c)) = (&self.landing, h.mixer.chain(sid)) {
            if let Some(slot) =
                (0..c.len()).find(|&i| c.uid(i).is_some_and(|u| !before.contains(&u)))
            {
                self.selected = Some(slot);
                self.landing = None;
                self.page_shut = false;
                // A Ducker starts on Sidechain with nothing picked: the
                // sidechain list opens at once (layout spec 4b).
                let n = c.node(slot);
                let waits = n.zip(n.and_then(|n| mpfree_fx::sidechain::listens(n.info().id)));
                if let Some((n, l)) = waits {
                    if l.listening(n.param(l.param)) && h.mixer.key(sid, slot).is_none() {
                        self.sidechain_list =
                            Some(mpfree_chain_ui::sidechain_group(self.strip(), None));
                    }
                }
            }
        }
        let title = format!("{} FX", self.scopes[self.scope].1);
        let mut st = match h.mixer.chain(sid) {
            Some(c) => ChainState::from_chain(
                c,
                self.strip(),
                &title,
                SLOTS,
                h.modu.engine(),
                self.selected,
                self.group,
                self.focus,
            ),
            None => ChainState {
                title,
                strip: self.strip(),
                ..ChainState::default()
            },
        };
        if self.page_shut && self.selected.is_none() {
            st = st.without_page();
        }
        if let Some(c) = h.mixer.chain(sid) {
            st.set_sidechains(c, |slot| {
                let k = h.mixer.key(sid, slot)?;
                Some((strip_of(k)?, mixing::name(k)))
            });
        }
        st.macro_names = h.modu.names().to_vec();
        st.plugins = crate::plugins::picks(h.plugins());
        st.learning = self.learning;
        st.back = self.back.clone();
        if self.scopes.len() > 1 {
            st.scopes = self.scopes.iter().map(|(_, n)| n.clone()).collect();
            st.scope = self.scope;
        }
        st.picking = self.picking;
        st.shelf = h.shelf.clone();
        if self.scopes.len() == 1 {
            st.strip_name = mixing::name(sid);
            st.strip_kind = kind(sid);
        }
        st.list = match (self.sidechain_list, &st.sidechain) {
            (Some(g), Some(sc)) => Some(sidechain_list(h, sid, sc, g)),
            _ => self.listing.map(|g| strip_list(h, g)),
        };
        xy_marks(&mut st.params, h);
        st
    }

    /// Opens the sidechain list over the selected effect's page (a tap on
    /// its "SC from ..." or "Nothing picked" on the mixer's Effects look).
    pub fn list_sidechain(&mut self, h: &MixHost) {
        let Some(slot) = self.selected else {
            return;
        };
        let sid = self.sid();
        let listens = h
            .mixer
            .chain(sid)
            .and_then(|c| c.node(slot))
            .and_then(|n| mpfree_fx::sidechain::listens(n.info().id));
        if listens.is_some() {
            let now = h.mixer.key(sid, slot).and_then(strip_of);
            self.sidechain_list = Some(mpfree_chain_ui::sidechain_group(self.strip(), now));
            self.listing = None;
        }
    }

    /// Applies one action to the mixer and the engine. `recording`: the
    /// strip's track is recording (a knob turn then beats the step locks).
    pub fn action(&mut self, h: &mut MixHost, a: &ChainAction, recording: bool) -> ChainOut {
        let sid = self.sid();
        let len = h.mixer.chain(sid).map_or(0, |c| c.len());
        match *a {
            ChainAction::Select(i) => {
                self.selected = Some(i);
                self.sidechain_list = None;
                self.page_shut = false;
                self.group = 0;
                self.focus = None;
                self.picking = false;
                self.notes_row = None;
                self.notes_picking = false;
            }
            ChainAction::SetOn { slot, on } => h.mixer.set_on(sid, slot, on),
            ChainAction::Move { from, to } => {
                if h.mixer
                    .edit(Edit::Move {
                        strip: sid,
                        from,
                        to,
                    })
                    .is_err()
                {
                    return ChainOut::Refused("The mixer is busy");
                }
                self.selected = self.selected.map(|s| moved(s, from, to));
            }
            ChainAction::Remove(slot) => {
                let uid = h.mixer.chain(sid).and_then(|c| c.uid(slot));
                if h.mixer
                    .edit(Edit::Remove {
                        strip: sid,
                        at: slot,
                    })
                    .is_err()
                {
                    return ChainOut::Refused("The mixer is busy");
                }
                if let Some(uid) = uid {
                    h.modu.forget(Owner::Insert(self.strip(), uid));
                }
                if self
                    .focus
                    .is_some_and(|f| matches!(f.owner, Owner::Insert(_, u) if Some(u) == uid))
                {
                    self.focus = None;
                }
                // The removed effect's page shuts with it (Jay, 16 Nov:
                // for every effect): back to the chain's list, nothing
                // picked, rather than another effect's page.
                self.selected = match self.selected {
                    Some(s) if s > slot => Some(s - 1),
                    Some(s) if s == slot => {
                        self.page_shut = true;
                        self.group = 0;
                        None
                    }
                    s => s,
                };
            }
            ChainAction::Add(id) | ChainAction::AddPreset { id, .. } => {
                self.picking = false;
                if len + h.mixer.edits_pending() + h.inserts_building() >= SLOTS {
                    return ChainOut::Refused("The chain is full");
                }
                // The picker's presets: the effect is built with it, so
                // it starts with that sound (no glide from its defaults).
                let preset = match *a {
                    ChainAction::AddPreset { id, preset } => mpfree_fx::picker::presets(id)
                        .get(preset)
                        .map_or(&[][..], |p| p.values),
                    _ => &[],
                };
                // Built off the audio thread in the app (control.rs), then
                // put in at the end of the chain.
                if let Err(why) = h.add_insert_with(sid, id, preset) {
                    return ChainOut::Refused(why);
                }
                // Only an effect that is on its way goes into Recent.
                h.shelf_dirty |= h.shelf.used(id);
                // It lands once it is built and its strip ducks (about 10
                // ms); until then the old selection stays.
                if self.landing.is_none() {
                    self.landing = h
                        .mixer
                        .chain(sid)
                        .map(|c| (0..c.len()).filter_map(|i| c.uid(i)).collect());
                }
            }
            ChainAction::SetMode { slot, mode } => {
                let owner = h
                    .mixer
                    .chain(sid)
                    .and_then(|c| c.uid(slot))
                    .map(|u| Owner::Insert(self.strip(), u));
                let set = |h: &mut MixHost, addr, v| set(h, Hand::Screen, addr, v, recording);
                match crate::fxmodes::switch(h, self.strip(), slot, mode, set) {
                    Err(why) => return ChainOut::Refused(why),
                    // A new engine has its own controls and tabs; it keeps
                    // the slot id, so it stays selected.
                    Ok(Switched::Across) => {
                        self.group = 0;
                        if self.focus.is_some_and(|f| Some(f.owner) == owner) {
                            self.focus = None;
                        }
                    }
                    Ok(_) => {}
                }
                self.selected = Some(slot);
            }
            ChainAction::Preset {
                slot,
                id,
                ref values,
            } => {
                // A preset from the header's list: each value set as a
                // tap on a button sets it (through the engine when it
                // holds the control), then let go. Its ids are its
                // engine's: refused if the slot holds another by now.
                let chain = h.mixer.chain(sid);
                let here = chain.and_then(|c| c.node(slot)).map(|n| n.info().id);
                let (Some(uid), true) = (chain.and_then(|c| c.uid(slot)), here == Some(id)) else {
                    return ChainOut::Refused("That effect has changed");
                };
                for &(param, v) in values {
                    let addr = ParamAddr::new(Owner::Insert(self.strip(), uid), param);
                    set(h, Hand::Screen, addr, v, recording);
                    h.modu.release(Hand::Screen, addr);
                }
                self.selected = Some(slot);
            }
            ChainAction::OpenPage(slot) => return ChainOut::OpenPage { strip: sid, slot },
            ChainAction::Group(g) => {
                self.group = g;
                self.focus = None;
            }
            ChainAction::Focus(addr) => self.focus = Some(addr),
            ChainAction::Set { addr, value } => set(h, Hand::Screen, addr, value, recording),
            ChainAction::Release(addr) => h.modu.release(Hand::Screen, addr),
            ChainAction::Macro { target, macro_ } => {
                let r = match macro_ {
                    Some(m) => {
                        let map = mapping(h, target);
                        h.modu.change(&h.mixer, Change::Map { m: m as u8, map })
                    }
                    None => h.modu.change(&h.mixer, Change::Unmap(target)),
                };
                if r.is_err() {
                    return ChainOut::Refused("That macro has 8 targets already");
                }
            }
            ChainAction::Learn(addr) => {
                self.learning = true;
                return ChainOut::Learn(addr);
            }
            // The screen sends XY → and XY ↑ to your layouts
            // (`gui::chain_msg`, `XyAction::Chip`), never here.
            ChainAction::Xy { .. } => {}
            ChainAction::AddRow { target, source } => {
                let row = Row {
                    source,
                    via: None,
                    target,
                    depth: ROW_DEPTH,
                };
                if h.modu.change(&h.mixer, Change::AddRow(row)).is_err() {
                    return ChainOut::Refused("The matrix is full");
                }
            }
            ChainAction::Depth { row, depth } => {
                let _ = h.modu.change(
                    &h.mixer,
                    Change::Depth {
                        row: row as u8,
                        depth,
                    },
                );
            }
            ChainAction::RemoveRow(row) => {
                let _ = h.modu.change(&h.mixer, Change::RemoveRow(row as u8));
            }
            ChainAction::Picker(on) => {
                self.picking = on;
                if on {
                    self.notes_row = None;
                    self.notes_picking = false;
                }
            }
            // The Notes stage is the project's: `Panels::chain` does it.
            ChainAction::Notes(_) => {}
            ChainAction::Favourite { id, on } => h.shelf_dirty |= h.shelf.set_favourite(id, on),
            ChainAction::Back => return ChainOut::Back,
            ChainAction::Scope(i) => {
                if i < self.scopes.len() {
                    *self = ChainHost {
                        scope: i,
                        ..ChainHost::new(self.scopes.clone(), self.back.clone())
                    };
                }
            }
            ChainAction::Strips(g) => {
                self.listing = g.filter(|&g| g < mpfree_chain_ui::STRIP_GROUPS.len());
                self.sidechain_list = None;
            }
            ChainAction::SidechainList(g) => {
                self.sidechain_list = g.filter(|&g| g < mpfree_chain_ui::STRIP_GROUPS.len());
                self.listing = None;
            }
            ChainAction::SetSidechain { control, strip } => {
                self.sidechain_list = None;
                return set_sidechain(h, control, strip.map(strip_id), recording);
            }
            ChainAction::OpenStrip(s) => {
                self.listing = None;
                return ChainOut::OpenStrip(strip_id(s));
            }
            // The screen sends a Capture with its buffer, as
            // `FromUi::Capture` ([`Panels::capture`]); bare, nothing.
            ChainAction::Capture(_) => {}
        }
        ChainOut::Shown
    }

    /// The selected effect's values, for the drawings the screen makes in
    /// its own theme (`ChainPage::set_fx`). None with no effect selected,
    /// or one the pages don't draw.
    pub fn fx_state(
        &self,
        h: &MixHost,
        st: &ChainState,
        bpm: f64,
    ) -> Option<mpfree_fx_ui::FxState> {
        let n = h.mixer.chain(self.sid())?.node(st.selected?)?;
        if !mpfree_fx_ui::visual::has_visuals(n.info().id) {
            return None;
        }
        Some(mpfree_fx_ui::FxState::from_node(
            n,
            "",
            mpfree_ui::Color::rgb(0),
            bpm,
            h.mixer.rate(),
        ))
    }

    /// The selected effect's drawings, every one its effect page draws, in
    /// theme `t` (`mpfree_chain_ui::set_chain_picture`: each section shows
    /// those its plan names).
    pub fn picture(
        &self,
        h: &MixHost,
        st: &ChainState,
        t: &mpfree_ui::Theme,
        bpm: f64,
    ) -> Vec<mpfree_synth_ui::widgets::Card> {
        let (Some(c), Some(slot)) = (h.mixer.chain(self.sid()), st.selected) else {
            return Vec::new();
        };
        mpfree_chain_ui::picture(c, slot, "", t, h.mixer.rate(), bpm)
    }
}

/// The XY pad's marks on `params` (layout spec 4b's XY marks): each one
/// the layout playing now moves gets which way, the stretch the pad covers
/// as knob positions, and where it really is (the knob's own position:
/// after the matrix, so after an Add setting's push too).
pub fn xy_marks(params: &mut [mpfree_chain_ui::state::ParamView], h: &MixHost) {
    let layout = h.xy_shown();
    let engine = h.modu.engine();
    for p in params.iter_mut() {
        p.xy = layout.and_then(|l| {
            let (i, t) = l.slots().find(|(_, t)| t.addr == p.addr)?;
            let (from, to) = match t.how {
                mpfree_xy::How::Set => (p.info.to_unit(t.from), p.info.to_unit(t.to)),
                // A push from wherever its own value is.
                mpfree_xy::How::Add => {
                    let own = p.info.to_unit(engine.base(p.addr).unwrap_or(p.value));
                    ((own + t.from).clamp(0.0, 1.0), (own + t.to).clamp(0.0, 1.0))
                }
            };
            Some(mpfree_chain_ui::state::XyMark {
                up: i >= mpfree_xy::TARGETS,
                from,
                to,
                now: p.pos,
                recorded: false,
            })
        });
    }
}

/// What insert `control.owner` listens to: strip `key` fed to it
/// (`Mixer::set_key`) with its sidechain control turned on, or (None) the
/// control turned off and the strip it had left fed, so a tap on its button
/// goes back to it. Its sidechain list's choice, and UNDO's.
pub fn set_sidechain(
    h: &mut MixHost,
    control: ParamAddr,
    key: Option<StripId>,
    recording: bool,
) -> ChainOut {
    let Owner::Insert(s, uid) = control.owner else {
        return ChainOut::Refused("That effect has changed");
    };
    let sid = strip_id(s);
    let found = h.mixer.chain(sid).and_then(|c| {
        let slot = c.slot_of(uid)?;
        let l = mpfree_fx::sidechain::listens(c.node(slot)?.info().id)?;
        (l.param == control.param).then_some((slot, l))
    });
    let Some((slot, l)) = found else {
        return ChainOut::Refused("That effect has changed");
    };
    let value = match key {
        Some(k) => {
            if let Some(why) = h.mixer.sidechain_refusal(sid, k) {
                return ChainOut::Refused(why.text());
            }
            if !h.mixer.set_key(sid, slot, Some(k)) {
                return ChainOut::Refused("The mixer is busy");
            }
            l.on
        }
        None => match l.off {
            Some(off) => off,
            None => return ChainOut::Shown,
        },
    };
    set(h, Hand::Screen, control, value, recording);
    h.modu.release(Hand::Screen, control);
    ChainOut::Shown
}

/// Which strip `s` is, for the strip button ("Track 4", "Submix 2").
fn kind(s: StripId) -> String {
    match s {
        StripId::Track(t) => format!("Track {}", t + 1),
        StripId::Pad(p) => format!("Pad {}", p + 1),
        StripId::Input(i) => format!("Input {}", i + 1),
        StripId::Bus(b) => format!("Submix {}", b + 1),
        StripId::Return(r) => format!("Return {}", (b'A' + r as u8) as char),
        StripId::Master => "Master".to_string(),
    }
}

/// The strips of each group of the strip list
/// (`mpfree_chain_ui::STRIP_GROUPS`): the tracks as the mixer shows them,
/// the pads, inputs, returns, submixes and the master.
fn strip_groups(h: &MixHost) -> [Vec<StripId>; 6] {
    [
        h.track_strips(),
        (0..mixing::PADS).map(StripId::Pad).collect(),
        (0..h.mixer.inputs()).map(StripId::Input).collect(),
        (0..mpfree_fx::RETURNS).map(StripId::Return).collect(),
        (0..mpfree_fx::BUSES).map(StripId::Bus).collect(),
        vec![StripId::Master],
    ]
}

/// What strip `id` holds, for its tile: "no effects", "muted · 3 effects".
fn holds(h: &MixHost, id: StripId) -> String {
    let n = h.mixer.chain(id).map_or(0, |c| c.len());
    let fx = match n {
        0 => "no effects".to_string(),
        1 => "1 effect".to_string(),
        n => format!("{n} effects"),
    };
    if muted(h, id) {
        format!("muted · {fx}")
    } else {
        fx
    }
}

fn muted(h: &MixHost, id: StripId) -> bool {
    id != StripId::Master && h.mixer.param(id, mpfree_fx::ids::mixer::MUTE) >= 0.5
}

/// The strip list on group `g` (of `mpfree_chain_ui::STRIP_GROUPS`): the
/// strips each group has (the tracks as the mixer shows them) and the
/// group's, with what each holds.
fn strip_list(h: &MixHost, g: usize) -> mpfree_chain_ui::StripList {
    let groups = strip_groups(h);
    let counts = std::array::from_fn(|i| groups[i].len());
    let strips = groups[g.min(5)]
        .iter()
        .filter_map(|&id| {
            Some(mpfree_chain_ui::StripPick {
                strip: strip_of(id)?,
                name: h.strip_name(id),
                sub: holds(h, id),
                refused: None,
            })
        })
        .collect();
    mpfree_chain_ui::StripList {
        group: g.min(5),
        counts,
        strips,
        sidechain: None,
    }
}

/// The sidechain list for the effect `sc` on strip `sid`, on group `g`
/// (layout spec 4b, "Sidechain"): only the groups that can feed it (a
/// strip hears those before it in the mix; the master feeds none), each
/// strip greyed with why when the mixer would refuse it, the line that
/// says which groups can't and why, and what it hears now.
fn sidechain_list(
    h: &MixHost,
    sid: StripId,
    sc: &mpfree_chain_ui::SidechainView,
    g: usize,
) -> mpfree_chain_ui::StripList {
    use mpfree_fx::sidechain::{Refusal, Row};
    let groups = strip_groups(h);
    let refusal = |k: StripId| h.mixer.sidechain_refusal(sid, k);
    let can = |k: &StripId| *k != StripId::Master && refusal(*k) != Some(Refusal::Later);
    let counts: [usize; 6] = std::array::from_fn(|i| groups[i].iter().filter(|k| can(k)).count());
    let g = g.min(5);
    let strips = groups[g]
        .iter()
        .filter(|k| can(k))
        .filter_map(|&id| {
            Some(mpfree_chain_ui::StripPick {
                strip: strip_of(id)?,
                name: h.strip_name(id),
                // It is heard before its fader and mute.
                sub: if muted(h, id) {
                    "muted · still works".to_string()
                } else {
                    holds(h, id)
                },
                refused: refusal(id).map(Refusal::text),
            })
        })
        .collect();
    let effect = h
        .mixer
        .chain(sid)
        .and_then(|c| c.node(sc.slot))
        .map_or_else(String::new, |n| n.info().name.to_uppercase());
    let me = mixing::name(sid).to_uppercase();
    let title = match sc.listens.row {
        Row::Voice => format!("VOICE FOR {me} · {effect} · TAP THE VOICE IT PLAYS"),
        _ => format!("SIDECHAIN FOR {me} · {effect} · TAP WHAT IT LISTENS TO"),
    };
    let now = match (&sc.track, sc.listening, sc.listens.row) {
        (Some((_, name)), true, _) => format!("Now: {name}"),
        (None, true, _) => "Now: nothing picked".to_string(),
        (_, false, Row::Source) => "Now: Tempo".to_string(),
        (_, false, _) => "Now: Off".to_string(),
    };
    // The groups left out, but for the master (it comes last of all).
    let out: Vec<String> = mpfree_chain_ui::STRIP_GROUPS[..5]
        .iter()
        .zip(counts)
        .filter(|(_, n)| *n == 0)
        .map(|(name, _)| name.to_lowercase())
        .collect();
    let kind = match sid {
        StripId::Track(_) => "track",
        StripId::Pad(_) => "pad",
        StripId::Input(_) => "input",
        StripId::Bus(_) => "submix",
        StripId::Return(_) => "return",
        StripId::Master => "master",
    };
    let note = match out.as_slice() {
        [] => String::new(),
        [one] => {
            format!("{one} come after this {kind} in the mix, so they can't feed its sidechain.")
        }
        [rest @ .., last] => format!(
            "{} and {last} come after this {kind} in the mix, so they can't feed its sidechain.",
            rest.join(", ")
        ),
    };
    let mut note = note;
    if let Some(c) = note.get_mut(..1) {
        c.make_ascii_uppercase();
    }
    mpfree_chain_ui::StripList {
        group: g,
        counts,
        strips,
        sidechain: Some(mpfree_chain_ui::SidechainFor {
            title,
            now,
            note,
            current: sc.track.as_ref().map(|t| t.0),
        }),
    }
}

/// Where slot `s` is after a move from `from` to `to`.
fn moved(s: usize, from: usize, to: usize) -> usize {
    if s == from {
        to
    } else if from < s && s <= to {
        s - 1
    } else if to <= s && s < from {
        s + 1
    } else {
        s
    }
}

/// A knob turned on the editor: through the engine when it holds the
/// parameter, else straight to the mixer.
/// Parameter `addr` set to `value` by a hand on the screen (the chain
/// editor's knobs, the Perform view's Q-Links): through the mod engine, so
/// it records over a move lane while `recording`, else straight onto the
/// mixer.
pub fn set_param(h: &mut MixHost, addr: ParamAddr, value: f32, recording: bool) {
    set(h, Hand::Screen, addr, value, recording)
}

/// `who`'s hand set `addr` to `value` (`hand.rs`).
fn set(h: &mut MixHost, who: Hand, addr: ParamAddr, value: f32, recording: bool) {
    if h.modu.hand(who, addr, value, recording) {
        return;
    }
    match addr.owner {
        Owner::Strip(s) => h.set(strip_id(s), addr.param, value),
        Owner::Insert(s, uid) => {
            let sid = strip_id(s);
            if let Some(slot) = h.mixer.chain(sid).and_then(|c| c.slot_of(uid)) {
                h.mixer.queue_insert(
                    sid,
                    slot,
                    Timed::new(
                        0,
                        NodeEvent::Param {
                            id: addr.param,
                            value,
                        },
                    ),
                );
            }
        }
        _ => {}
    }
}

/// Sets `addr` as a knob turn would and lets it go at once (UNDO and
/// REDO of a mixer turn, `mixturns.rs`).
pub fn set_param_now(h: &mut MixHost, addr: ParamAddr, value: f32) {
    set(h, Hand::Screen, addr, value, false);
    h.modu.release(Hand::Screen, addr);
}

/// A knob's map when it goes on a macro: the one it has if it is on
/// another, else from where it is now to the far end of its travel.
fn mapping(h: &MixHost, target: ParamAddr) -> MacroMap {
    let e = h.modu.engine();
    if let Some(map) = e
        .macro_of(target)
        .and_then(|m| e.macro_maps(m).find(|map| map.target == target))
    {
        return map;
    }
    let pos = e
        .value(target)
        .zip(e.info(target))
        .map(|(v, i)| i.to_unit(v))
        .or_else(|| h.modu.param(&h.mixer, target).map(|(i, v)| i.to_unit(v)))
        .unwrap_or(0.0);
    MacroMap {
        target,
        from: pos,
        to: if pos < 0.5 { 1.0 } else { 0.0 },
        curve: MapCurve::Linear,
    }
}

/// A parameter's name on the macro cards: "Drive (Drums)", "Volume (Loop)",
/// "Macro 2 · Grit".
pub fn label(h: &MixHost, addr: ParamAddr) -> String {
    match addr.owner {
        Owner::Macro(n) => {
            let name = h.modu.names().get(n as usize).cloned().unwrap_or_default();
            format!("Macro {} · {name}", n + 1)
        }
        Owner::Strip(s) => {
            let p = strip_params()
                .iter()
                .find(|p| p.id == addr.param)
                .map_or("?", |p| p.name);
            format!("{p} ({})", mixing::name(strip_id(s)))
        }
        Owner::Insert(s, uid) => {
            let sid = strip_id(s);
            let node = h.mixer.chain(sid).and_then(|c| c.node(c.slot_of(uid)?));
            let Some(n) = node else {
                return format!("? ({})", mixing::name(sid));
            };
            let p = n
                .params()
                .iter()
                .find(|p| p.id == addr.param)
                .map_or("?", |p| p.name);
            let effect = n.info().name;
            if p.starts_with(effect) {
                format!("{p} ({})", mixing::name(sid))
            } else {
                format!("{effect} {} ({})", p.to_lowercase(), mixing::name(sid))
            }
        }
        Owner::NoteFx(..) => h.modu.note_label(addr).unwrap_or_else(|| addr.to_string()),
        // Outside gear's control on the track's MIDI out: "CC 74 (Track 3)".
        Owner::Midi(t) => match mpfree_play::mpfree_param::midi::name(addr.param) {
            Some(n) => format!("{n} (Track {})", u16::from(t) + 1),
            None => addr.to_string(),
        },
        _ => addr.to_string(),
    }
}

/// What the app does after a macros panel action.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MacroOut {
    Shown,
    /// Close the panel.
    Back,
    Refused(&'static str),
}

/// The macros panel's side in the app: the bank shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MacrosHost {
    bank: usize,
}

impl MacrosHost {
    pub fn new(bank: usize) -> MacrosHost {
        MacrosHost { bank: bank.min(1) }
    }

    pub fn state(&self, h: &MixHost) -> MacrosState {
        MacrosState::from_engine(h.modu.engine(), h.modu.names(), &|a| label(h, a), self.bank)
    }

    /// Applies one action. `recording`: a macro turn then beats the step
    /// locks on its targets.
    pub fn action(&mut self, h: &mut MixHost, a: &MacroAction, recording: bool) -> MacroOut {
        match *a {
            MacroAction::Macro { m, value } => {
                h.modu.hand(
                    Hand::Screen,
                    ParamAddr::macro_value(m as u8),
                    value,
                    recording,
                );
            }
            MacroAction::Release(m) => {
                h.modu
                    .release(Hand::Screen, ParamAddr::macro_value(m as u8));
            }
            MacroAction::Bank(b) => self.bank = b.min(1),
            MacroAction::Map { m, map } => {
                if h.modu
                    .change(&h.mixer, Change::Map { m: m as u8, map })
                    .is_err()
                {
                    return MacroOut::Refused("That macro has 8 targets already");
                }
            }
            MacroAction::Unmap(t) => {
                let _ = h.modu.change(&h.mixer, Change::Unmap(t));
            }
            MacroAction::Morph(p) => {
                h.modu.hand(Hand::Screen, ParamAddr::morph_pos(0), p, false);
            }
            MacroAction::Capture(which) => {
                let _ = h.modu.change(&h.mixer, Change::Capture(which.min(1) as u8));
            }
            MacroAction::Back => return MacroOut::Back,
        }
        MacroOut::Shown
    }
}

/// How often the open panel's state goes to the screen: its values move
/// with LFOs, followers and the morph, not only with touches.
pub const PANELS_EVERY: Duration = Duration::from_millis(100);

/// The panels in the audio loop: the chain the editor shows, the macros
/// panel's bank, the mixer view the editor took the place of (its Back
/// opens the mixer there again), and when the screen last had their
/// states. Building a state allocates; it runs between periods, as the
/// mixer panel's does.
pub struct Panels {
    chain: Option<ChainHost>,
    macros: MacrosHost,
    over_mix: MixView,
    sent_at: Option<Instant>,
    /// The chain is an effect picked on the mixer's Effects look, not the
    /// editor: its state goes to the screen while the mixer shows, for
    /// the Q-Links.
    picked: bool,
    /// The pad channels' effects as last sent for the Pads tab's Effects
    /// row.
    pad_fx: Option<Vec<PadFx>>,
}

impl Default for Panels {
    fn default() -> Self {
        Panels::new()
    }
}

impl Panels {
    pub fn new() -> Panels {
        Panels {
            chain: None,
            macros: MacrosHost::new(0),
            over_mix: MixView::Tracks,
            sent_at: None,
            picked: false,
            pad_fx: None,
        }
    }

    /// The chain editor is open with learn armed (MIDI Learn listens only
    /// while it is, M5).
    pub fn learning(&self) -> bool {
        self.chain.as_ref().is_some_and(|h| h.learning)
    }

    /// A learned MIDI control moved `addr` to `value` (in the parameter's
    /// own units; 0 to 1 for a macro), set as a chain editor knob sets it,
    /// and the open panel shows it at once (M5). Its hold while recording
    /// ends 0.5 s after its last move (`hand.rs`).
    pub fn set_param(&mut self, mix: &mut MixHost, addr: ParamAddr, value: f32, recording: bool) {
        set(mix, Hand::Midi, addr, value, recording);
        self.sent_at = None;
    }

    /// The chain editor's state as the mixer has it now, while the editor
    /// is open (the smoke tests check the screen against it).
    #[cfg(test)]
    pub fn chain_state(
        &mut self,
        mix: &MixHost,
        notes: &dyn Fn(usize) -> Option<NotesSource>,
    ) -> Option<ChainState> {
        self.chain.as_mut().map(|h| {
            let mut st = h.state(mix);
            let src = h.track().and_then(notes);
            h.dress_notes(&mut st, src, mix.modu.engine());
            st
        })
    }

    /// The macros' values, for the Q-Link column's macro banks (at the
    /// start; the panels send them again while they show).
    pub fn macros_state(&self, mix: &MixHost) -> MacrosState {
        self.macros.state(mix)
    }

    /// The mixer's insert row (`slot`) or its "+" (None, the picker up):
    /// the chain editor in the mixer's place.
    pub fn open_insert(
        &mut self,
        app: &mut App,
        mix: &MixHost,
        strip: StripId,
        slot: Option<usize>,
    ) -> Vec<Effect> {
        let Some(h) = ChainHost::from_mixer(strip, slot).map(|h| h.named(mix, strip)) else {
            return Vec::new();
        };
        self.over_mix = mix.shown().unwrap_or(MixView::Tracks);
        self.chain = Some(h);
        self.sent_at = None;
        self.picked = false;
        app.open_chain()
    }

    /// A tap on an effect's "SC from ..." or "Nothing picked" on the
    /// mixer's Effects look: the effect in the chain editor with its
    /// sidechain list up (layout spec 4a).
    pub fn open_sidechain(
        &mut self,
        app: &mut App,
        mix: &MixHost,
        strip: StripId,
        slot: usize,
    ) -> Vec<Effect> {
        let fx = self.open_insert(app, mix, strip, Some(slot));
        if let Some(h) = self.chain.as_mut() {
            h.list_sidechain(mix);
        }
        fx
    }

    /// An effect picked on the mixer's Effects look (`MixAction::PickInsert`):
    /// its chain is kept as the editor's would be, without the editor, so
    /// the Q-Links turn its first four knobs ([`Panels::poll`] sends its
    /// state while the mixer shows).
    pub fn pick_insert(&mut self, strip: StripId, slot: usize) {
        let Some(h) = ChainHost::from_mixer(strip, Some(slot)) else {
            return;
        };
        self.chain = Some(h);
        self.picked = true;
        self.sent_at = None;
    }

    /// The pick went (`MixAction::Unpick`): its state stops going to the
    /// screen.
    pub fn unpick(&mut self) {
        if self.picked {
            self.picked = false;
            self.chain = None;
        }
    }

    /// An effect dragged along its row on the Effects look
    /// (`MixAction::MoveInsert`): moved as the chain editor moves one.
    pub fn move_insert(
        &mut self,
        mix: &mut MixHost,
        strip: StripId,
        from: usize,
        to: usize,
    ) -> Vec<Effect> {
        let same = self
            .chain
            .as_ref()
            .is_some_and(|h| strip_of(strip) == Some(h.strip()));
        let mut own;
        let h = if same {
            self.chain.as_mut().expect("checked")
        } else {
            let Some(h) = ChainHost::from_mixer(strip, Some(from)) else {
                return Vec::new();
            };
            own = h;
            &mut own
        };
        self.sent_at = None;
        match h.action(mix, &ChainAction::Move { from, to }, false) {
            ChainOut::Refused(why) => vec![Effect::Tell(format!("mixer: {why}"))],
            _ => Vec::new(),
        }
    }

    /// "+ EQ" on the mixer's EQ look (`MixAction::AddEq`): a parametric
    /// EQ added at the end of the strip's chain (its first free slot), as
    /// the chain editor adds one.
    pub fn add_eq(&mut self, mix: &mut MixHost, strip: StripId) -> Vec<Effect> {
        let Some(mut h) = ChainHost::from_mixer(strip, None) else {
            return Vec::new();
        };
        self.sent_at = None;
        match h.action(mix, &ChainAction::Add(mpfree_chain_ui::state::EQ_ID), false) {
            ChainOut::Refused(why) => vec![Effect::Tell(format!("mixer: {why}"))],
            _ => Vec::new(),
        }
    }

    /// A band picked or a dot dragged on the mixer's EQ look
    /// (`MixAction::FocusInsert`, `SetInsert`, `ReleaseInsert`): parameter
    /// `id` of insert `slot`, as the chain editor's knobs set it (so a
    /// drag is one UNDO step and the picked EQ's Q-Links follow the band).
    /// The EQ is picked first if it wasn't.
    pub fn insert_param(
        &mut self,
        mix: &mut MixHost,
        a: &mpfree_mixer_ui::MixAction,
        recording: bool,
    ) {
        use mpfree_mixer_ui::MixAction as M;
        let (strip, slot, id) = match *a {
            M::FocusInsert { strip, slot, id }
            | M::SetInsert {
                strip, slot, id, ..
            }
            | M::ReleaseInsert { strip, slot, id } => (strip, slot, id),
            _ => return,
        };
        let (Some(s), Some(uid)) = (
            strip_of(strip),
            mix.mixer.chain(strip).and_then(|c| c.uid(slot)),
        ) else {
            return;
        };
        let addr = ParamAddr::new(Owner::Insert(s, uid), id);
        let picked = self.picked
            && self
                .chain
                .as_ref()
                .is_some_and(|h| h.strip() == s && h.selected == Some(slot));
        if !picked {
            self.pick_insert(strip, slot);
        }
        let ca = match *a {
            M::FocusInsert { .. } => ChainAction::Focus(addr),
            M::SetInsert { value, .. } => ChainAction::Set { addr, value },
            _ => ChainAction::Release(addr),
        };
        if let Some(h) = self.chain.as_mut() {
            h.action(mix, &ca, recording);
        }
        self.sent_at = None;
    }

    /// The Pads tab's Effects row: kit `kit`'s pad `pad`'s chain in the
    /// Mix tab (lent a channel if it has none), on its first effect, or
    /// the picker when it has none (Jay's Mix tab card, 14 Nov). ‹ comes
    /// back to the Pads tab.
    pub fn open_pad_mix(
        &mut self,
        app: &mut App,
        mix: &mut MixHost,
        kit: usize,
        pad: usize,
    ) -> Vec<Effect> {
        self.picked = false;
        let host = mix.lend_pad(kit, pad).and_then(|sid| {
            let first = mix.mixer.chain(sid).filter(|c| !c.is_empty()).map(|_| 0);
            Some(ChainHost::from_mixer(sid, first)?.named(mix, sid))
        });
        let Some(mut h) = host else {
            return vec![Effect::Tell(no_channel(kit, pad))];
        };
        h.back = Some("Pads".into());
        self.chain = Some(h);
        self.sent_at = None;
        app.open_chain()
    }

    /// The Perform view's Edit the effect: throw `fx`'s pad of strip
    /// `on`'s Perform FX in the Mix tab's chain editor, on that pad's page
    /// ("Slots 9-12" for the Delay throw), with ‹ back to the Pads tab.
    pub fn open_throw_mix(
        &mut self,
        app: &mut App,
        mix: &mut MixHost,
        on: StripId,
        fx: mixing::ThrowFx,
    ) -> Vec<Effect> {
        self.picked = false;
        let host = mix
            .perform_slot(on)
            .and_then(|slot| ChainHost::from_mixer(on, Some(slot)));
        let Some(h) = host else {
            return vec![Effect::Tell(format!(
                "perform: {} has no Perform FX for the throw: pick the pad's On again on the Pads tab",
                mix.strip_name(on)
            ))];
        };
        let mut h = h.named(mix, on);
        // The Perform FX's pages: Perform, then four slots to a page.
        h.group = 1 + fx.pad() / 4;
        h.back = Some("Pads".into());
        self.chain = Some(h);
        self.sent_at = None;
        app.open_chain()
    }

    /// A long press on a macro card: the macros panel on `bank` (0:
    /// Macros 1-4, 1: 5-8) over the page.
    pub fn open_macros(&mut self, app: &mut App, mix: &mut MixHost, bank: usize) -> Vec<Effect> {
        if app.view() == View::Mixer {
            self.over_mix = mix.shown().unwrap_or(MixView::Tracks);
        }
        self.macros.action(mix, &MacroAction::Bank(bank), false);
        self.sent_at = None;
        app.open_macros()
    }

    /// SHIFT + SAMPLE EDIT (the MPC's Q-Link Edit): the macros panel on
    /// Macros 1-4 over the page, or, on it, back to the page it opened
    /// over.
    pub fn toggle_macros(&mut self, app: &mut App, mix: &mut MixHost) -> Vec<Effect> {
        if app.view() == View::Macros {
            return app.close_macros(self.over_mix);
        }
        self.open_macros(app, mix, 0)
    }

    /// A chain editor action (a touch, or a Q-Link on its knobs).
    /// `show` takes what the screen needs past the next state (a learn).
    pub fn chain(
        &mut self,
        app: &mut App,
        mix: &mut MixHost,
        time: &mut TimeHost,
        a: &ChainAction,
        recording: bool,
        show: &mut impl FnMut(ToUi),
    ) -> Vec<Effect> {
        let Some(h) = self.chain.as_mut() else {
            return Vec::new();
        };
        self.sent_at = None;
        // A MIDI effect's setting turned is the Notes stage's (the project
        // keeps it); touching it, its macro, Learn and its rows are the
        // mod engine's, as for a sound's.
        let note = match *a {
            ChainAction::Notes(na) => Some(na),
            ChainAction::Set { addr, value } => match addr.owner {
                Owner::NoteFx(_, id) => Some(NoteAction::Set {
                    id,
                    k: usize::from(addr.param),
                    value: value.round() as i32,
                }),
                _ => None,
            },
            _ => None,
        };
        if let Some(na) = note {
            return notes_action(app, h, mix, time, &na, recording);
        }
        match h.action(mix, a, recording) {
            ChainOut::Shown => Vec::new(),
            ChainOut::OpenPage { .. } => vec![Effect::Tell(
                "chain: an effect's own page comes with the FX pages; its knobs are on the editor"
                    .to_string(),
            )],
            ChainOut::Learn(addr) => {
                show(ToUi::ChainLearn(addr));
                Vec::new()
            }
            ChainOut::Back => {
                // A channel lent and left plain goes back.
                mix.give_back_plain();
                app.close_chain(self.over_mix)
            }
            ChainOut::OpenStrip(sid) => {
                // The strip's chain on its first effect (the picker up on
                // one with none), Back still going where it went.
                let first = mix.mixer.chain(sid).is_some_and(|c| !c.is_empty());
                let back = h.back.clone();
                if let Some(mut nh) =
                    ChainHost::from_mixer(sid, first.then_some(0)).map(|h| h.named(mix, sid))
                {
                    nh.back = back;
                    self.chain = Some(nh);
                }
                Vec::new()
            }
            ChainOut::Refused(why) => vec![Effect::Tell(format!("chain: {why}"))],
        }
    }

    /// The chain editor's Capture on insert `slot` of the chain it shows
    /// (`FromUi::Capture`): the copy starts on the mixer
    /// ([`MixHost::capture`]); a box that can't go comes back with why.
    pub fn capture(
        &mut self,
        mix: &mut MixHost,
        slot: usize,
        held: Box<Held>,
    ) -> Result<(), (Box<Held>, Captured)> {
        let Some(h) = self.chain.as_ref() else {
            return Err((held, Captured::Gone));
        };
        mix.capture(h.sid(), slot, held)
    }

    /// A Q-Link took the knob learn was armed for.
    pub fn learned(&mut self) {
        if let Some(h) = self.chain.as_mut() {
            h.learned();
            self.sent_at = None;
        }
    }

    /// A macros panel action (a touch, or a Q-Link on a macro bank).
    pub fn macros(
        &mut self,
        app: &mut App,
        mix: &mut MixHost,
        a: &MacroAction,
        recording: bool,
    ) -> Vec<Effect> {
        let out = self.macros.action(mix, a, recording);
        if !matches!(a, MacroAction::Macro { .. }) {
            // A turn shows on its card at once; the rest needs the state.
            self.sent_at = None;
        }
        match out {
            MacroOut::Shown => Vec::new(),
            MacroOut::Back => app.close_macros(self.over_mix),
            MacroOut::Refused(why) => vec![Effect::Tell(format!("macros: {why}"))],
        }
    }

    /// While the chain editor or the macros panel shows (`view`): its
    /// state, every [`PANELS_EVERY`] and at once after a change. The chain
    /// editor sends its effect's values for the drawings too, and both send
    /// the macros' values. `bpm`: for synced effects' drawings.
    pub fn poll(
        &mut self,
        view: View,
        mix: &MixHost,
        notes: &dyn Fn(usize) -> Option<NotesSource>,
        bpm: f64,
        now: Instant,
        show: &mut impl FnMut(ToUi),
    ) {
        let picked = view == View::Mixer && self.picked && self.chain.is_some();
        if view == View::Pads {
            if self
                .sent_at
                .is_some_and(|t| now.duration_since(t) < PANELS_EVERY)
            {
                return;
            }
            self.sent_at = Some(now);
            // The Pad list's Effects row: sent when an effect comes, goes
            // or moves.
            let fx = pad_fx(mix);
            if self.pad_fx.as_ref() != Some(&fx) {
                show(ToUi::PadFx(fx.clone()));
                self.pad_fx = Some(fx);
            }
            return;
        }
        if !(matches!(view, View::Chain | View::Macros) || picked)
            || self
                .sent_at
                .is_some_and(|t| now.duration_since(t) < PANELS_EVERY)
        {
            return;
        }
        self.sent_at = Some(now);
        if picked {
            // The picked effect's knobs for the Q-Links; no drawings.
            if let Some(h) = self.chain.as_mut() {
                show(ToUi::Chain(Box::new(h.state(mix))));
            }
            show(ToUi::Macros(Box::new(self.macros.state(mix))));
            return;
        }
        if view == View::Chain {
            if let Some(h) = self.chain.as_mut() {
                let mut st = h.state(mix);
                let src = h.track().and_then(notes);
                h.dress_notes(&mut st, src, mix.modu.engine());
                let fx = h.fx_state(mix, &st, bpm);
                show(ToUi::Chain(Box::new(st)));
                show(ToUi::ChainFx(fx.map(Box::new)));
            }
        }
        show(ToUi::Macros(Box::new(self.macros.state(mix))));
    }
}

/// A Notes stage action on the track the chain editor shows: its MIDI
/// effects change in the project (saved with it), Plays on opens the
/// MIDI section, and the arp's page opens its panel on that track (arming
/// it, as picking a track does, unless a take is recording on another).
fn notes_action(
    app: &mut App,
    h: &mut ChainHost,
    mix: &mut MixHost,
    time: &mut TimeHost,
    a: &NoteAction,
    recording: bool,
) -> Vec<Effect> {
    let Some(t) = h.track() else {
        return Vec::new();
    };
    let Some(was) = time.project().tracks.get(t).map(|tr| tr.note_fx) else {
        return Vec::new();
    };
    let mut fx = was;
    match h.note_action(a, &mut fx) {
        NoteOut::Shown => Vec::new(),
        NoteOut::Changed => {
            if time.set_note_fx(t, fx) {
                note_fx_to_engine(mix, t, &was, &fx, a);
                Vec::new()
            } else {
                vec![Effect::Tell(
                    "notes: a Loop track has no MIDI effects".into(),
                )]
            }
        }
        NoteOut::PlaysOn => app.open_io(),
        NoteOut::OpenArp => {
            // The arp panel shows the selected track's arp.
            if time.selected() == Some(t) {
                return app.arp_action(mpfree_arp_ui::view::ArpAction::OpenPanel);
            }
            let armed = time.armed();
            if recording {
                return vec![Effect::Tell(format!(
                    "notes: recording on track {}: stop to open track {}'s arp",
                    armed.map_or(0, |a| a + 1),
                    t + 1
                ))];
            }
            let mut fx = vec![Effect::Time(mpfree_time_ui::TimeAction::Pick(t))];
            fx.extend(app.arp_action(mpfree_arp_ui::view::ArpAction::OpenPanel));
            fx
        }
        NoteOut::Refused(why) => vec![Effect::Tell(format!("notes: {why}"))],
    }
}

/// A track's MIDI effects changed from `was` to `now` on the screen: the
/// mod engine forgets an effect that left the track (its macros, rows and
/// locks go with it), and a setting it holds takes the new value as its
/// own (a macro or an LFO then moves it from there).
fn note_fx_to_engine(mix: &mut MixHost, t: usize, was: &TrackFx, now: &TrackFx, a: &NoteAction) {
    let Ok(track) = u8::try_from(t) else {
        return;
    };
    for p in 0..mpfree_notefx::MAX_FX {
        let id = was.ids[p];
        if was.slots[p].is_some() && now.place_of(id).is_none() {
            mix.modu.forget(Owner::NoteFx(track, id));
        }
    }
    if let NoteAction::Set { id, k, .. } = *a {
        if let (Some(v), Ok(param)) = (now.value(id, k), u16::try_from(k)) {
            mix.modu
                .set(ParamAddr::new(Owner::NoteFx(track, id), param), v as f32);
        }
    }
}

/// The effects on each of the mixer's pad channels, in order, named as the
/// chain editor names them ("Reverb · Plate").
fn pad_fx(mix: &MixHost) -> Vec<PadFx> {
    let names = |s: StripId| {
        mix.mixer.chain(s).map_or_else(Vec::new, |c| {
            (0..c.len())
                .map(|i| {
                    c.node(i)
                        .map_or_else(|| "?".to_string(), mpfree_fx::families::insert_name)
                })
                .collect()
        })
    };
    (0..mixing::PADS)
        .map(|p| (0, p, names(StripId::Pad(p))))
        .chain(mix.pad_channels.lent().map(|(s, k, p)| (k, p, names(s))))
        .collect()
}

/// A pad with a channel of its own, and the effects on it: its kit, its
/// pad and the effects' names, in order.
pub type PadFx = (usize, usize, Vec<String>);

/// Why kit `kit`'s pad `pad` got no channel: every one is lent.
fn no_channel(kit: usize, pad: usize) -> String {
    format!(
        "chain: no channel for pad {}: all {} pad channels past kit 1's A1 to A{} are lent",
        mixing::lent_name(kit, pad),
        mixing::LENT,
        mixing::PADS
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixing::{DRUMS, LOOP};
    use mpfree_fx::ids::mixer as mx;
    use mpfree_mod::{Moved, Source};
    use mpfree_node::Ctx;

    const RATE: u32 = 44_100;

    struct Rig {
        h: MixHost,
        period: u64,
    }

    impl Rig {
        /// The app's mixer with the Play page's inserts in.
        fn new() -> Rig {
            let mut r = Rig {
                h: MixHost::new(RATE, 0.0, 6.0),
                period: 0,
            };
            r.run(20);
            assert_eq!(r.h.mixer.edits_pending(), 0);
            r
        }

        fn run(&mut self, periods: u64) {
            let z = [0f32; PERIOD];
            let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
            for _ in 0..periods {
                let ctx = Ctx::new(RATE, self.period * PERIOD as u64);
                self.h
                    .period(&ctx, (&z, &z), (&z, &z), (&z, &z), &mut l, &mut r);
                self.period += 1;
            }
        }

        fn param(&self, addr: ParamAddr) -> f32 {
            crate::modmix::describe(&self.h.mixer, addr).unwrap().1
        }
    }

    fn drums() -> StripId {
        StripId::Track(DRUMS)
    }

    /// The Pads tab's Effects row: the chain editor on the pad's own
    /// channel, so an effect added there (Perform FX: stutters, repeats,
    /// stops) plays on that pad alone; with every channel lent, another
    /// pad gets none and is told why.
    #[test]
    fn a_pads_effects_row_edits_its_own_chain_or_says_why_not() {
        let mut r = Rig::new();
        let mut app = App::new(RATE, false);
        let mut time = TimeHost::new(RATE, 120.0).unwrap();
        app.start();
        app.open_tab(View::Pads);
        let mut p = Panels::new();
        p.open_pad_mix(&mut app, &mut r.h, 0, 4);
        assert_eq!(app.view(), View::Chain);
        let st = p.chain_state(&r.h, &|_| None).unwrap();
        assert!(st.picking, "the picker, on a pad with no effects");
        let len = |r: &Rig, s: StripId| r.h.mixer.chain(s).map_or(0, |c| c.len());
        let drums_before = len(&r, drums());
        let mut quiet = |_: ToUi| {};
        p.chain(
            &mut app,
            &mut r.h,
            &mut time,
            &ChainAction::Add("mpfree.fx.perform"),
            false,
            &mut quiet,
        );
        r.run(20);
        assert_eq!(len(&r, StripId::Pad(4)), 1, "Perform on pad 5's channel");
        assert_eq!(len(&r, StripId::Pad(3)), 0);
        assert_eq!(len(&r, drums()), drums_before);
        for pad in 0..mixing::LENT {
            assert!(r.h.lend_pad(1, pad).is_some());
        }
        let fx = p.open_pad_mix(&mut app, &mut r.h, 2, 0);
        assert!(
            matches!(&fx[..], [Effect::Tell(n)] if n.contains("are lent")),
            "{fx:?}"
        );
    }

    /// The audio loop's side: the mixer's insert row opens the editor in the
    /// mixer's place, its states go out at once and then every
    /// [`PANELS_EVERY`] while it shows, a learn reaches the screen, and Back
    /// opens the mixer on the view it showed. The macros panel the same.
    #[test]
    fn the_panels_open_from_the_mixer_send_their_states_and_go_back() {
        let mut r = Rig::new();
        let mut app = App::new(RATE, false);
        let mut time = TimeHost::new(RATE, 120.0).unwrap();
        app.start();
        let mut p = Panels::new();
        for e in app.toggle_mixer(true) {
            if let Effect::Mixer(v) = e {
                r.h.show(v);
            }
        }
        assert_eq!(r.h.shown(), Some(MixView::Pads));
        let fx = p.open_insert(&mut app, &r.h, drums(), Some(0));
        assert_eq!(app.view(), View::Chain);
        assert!(fx.contains(&Effect::Mixer(None)), "{fx:?}");
        r.h.show(None);
        let kinds = |sent: &[ToUi]| {
            sent.iter()
                .map(|m| match m {
                    ToUi::Chain(st) => format!("chain {}", st.title),
                    ToUi::ChainFx(_) => "picture".to_string(),
                    ToUi::Macros(st) => format!("macros {}", st.bank),
                    ToUi::ChainLearn(_) => "learn".to_string(),
                    _ => "other".to_string(),
                })
                .collect::<Vec<_>>()
        };
        let t0 = Instant::now();
        let mut sent = Vec::new();
        p.poll(app.view(), &r.h, &|_| None, 120.0, t0, &mut |m| {
            sent.push(m)
        });
        assert_eq!(kinds(&sent), ["chain Drums FX", "picture", "macros 0"]);
        sent.clear();
        p.poll(
            app.view(),
            &r.h,
            &|_| None,
            120.0,
            t0 + PANELS_EVERY / 2,
            &mut |m| sent.push(m),
        );
        assert!(sent.is_empty(), "not yet");
        // Learn on the knob last touched: the screen arms it.
        let st = p.chain.as_mut().unwrap().state(&r.h);
        let knob = st.params[0].addr;
        p.chain(
            &mut app,
            &mut r.h,
            &mut time,
            &ChainAction::Focus(knob),
            false,
            &mut |m| sent.push(m),
        );
        p.chain(
            &mut app,
            &mut r.h,
            &mut time,
            &ChainAction::Learn(knob),
            false,
            &mut |m| sent.push(m),
        );
        assert_eq!(kinds(&sent), ["learn"]);
        sent.clear();
        p.learned();
        // A change shows at once.
        p.poll(
            app.view(),
            &r.h,
            &|_| None,
            120.0,
            t0 + PANELS_EVERY / 2,
            &mut |m| sent.push(m),
        );
        assert_eq!(sent.len(), 3);
        sent.clear();
        // Back: the mixer again, on Pad Mix.
        let fx = p.chain(
            &mut app,
            &mut r.h,
            &mut time,
            &ChainAction::Back,
            false,
            &mut |m| sent.push(m),
        );
        assert_eq!(
            fx.first(),
            Some(&Effect::Mixer(Some(MixView::Pads))),
            "{fx:?}"
        );
        assert_eq!(app.view(), View::Mixer);
        r.h.show(Some(MixView::Pads));
        p.poll(
            app.view(),
            &r.h,
            &|_| None,
            120.0,
            t0 + PANELS_EVERY * 3,
            &mut |m| sent.push(m),
        );
        assert!(sent.is_empty(), "nothing while the panels are shut");
        // The macros panel over the mixer, on Macros 5-8; Back to Pad Mix.
        p.open_macros(&mut app, &mut r.h, 1);
        assert_eq!(app.view(), View::Macros);
        p.poll(
            app.view(),
            &r.h,
            &|_| None,
            120.0,
            t0 + PANELS_EVERY * 3,
            &mut |m| sent.push(m),
        );
        assert_eq!(kinds(&sent), ["macros 1"]);
        let fx = p.macros(&mut app, &mut r.h, &MacroAction::Back, false);
        assert_eq!(
            fx.first(),
            Some(&Effect::Mixer(Some(MixView::Pads))),
            "{fx:?}"
        );
        assert_eq!(app.view(), View::Mixer);
    }

    /// The editor's state as the screen gets it now, with the track's
    /// Notes stage.
    fn shown(p: &mut Panels, app: &App, time: &TimeHost, r: &Rig) -> ChainState {
        let mut got = None;
        p.poll(
            app.view(),
            &r.h,
            &|t| NotesSource::of(app, time, t),
            120.0,
            Instant::now(),
            &mut |m| {
                if let ToUi::Chain(st) = m {
                    got = Some(*st);
                }
            },
        );
        got.expect("the editor's state")
    }

    /// A note track's chain: its Notes stage shows above the sound, a MIDI
    /// effect added from the picker lands in the project above the arp with
    /// its page open and its settings on the Q-Links, a Q-Link sets one,
    /// rows move, the arp stays, and Plays on and the arp's page open
    /// theirs (the arp's on its track, armed).
    #[test]
    fn the_notes_stage_edits_the_tracks_midi_effects() {
        use crate::mixing::SYNTH;
        let mut r = Rig::new();
        let mut app = App::new(RATE, false);
        let mut time = TimeHost::new(RATE, 120.0).unwrap();
        app.start();
        let mut p = Panels::new();
        // Track 2's own strip (R3 part 4b: the Synth strip is no track's).
        p.open_insert(
            &mut app,
            &r.h,
            StripId::Track(mixing::track_channel(SYNTH)),
            None,
        );
        let mut quiet = |_: ToUi| {};
        let act = |p: &mut Panels, app: &mut App, time: &mut TimeHost, r: &mut Rig, a| {
            p.chain(
                app,
                &mut r.h,
                time,
                &ChainAction::Notes(a),
                false,
                &mut |_| {},
            )
        };
        let st = shown(&mut p, &app, &time, &r);
        let ns = st.notes.as_ref().expect("a note track has a Notes stage");
        assert_eq!((ns.track, ns.items().len()), (SYNTH as u8, 1), "the arp");
        assert_eq!((ns.arp.as_str(), ns.arp_on), ("Off", false));
        assert_eq!(ns.selected, None);
        // The MIDI effect picker: a Chord goes above the arp, its page up.
        act(
            &mut p,
            &mut app,
            &mut time,
            &mut r,
            NoteAction::Picker(true),
        );
        assert!(shown(&mut p, &app, &time, &r).notes.unwrap().picking);
        let chord = Fx::all().iter().position(|f| f.name() == "Chord").unwrap();
        let fx = act(&mut p, &mut app, &mut time, &mut r, NoteAction::Add(chord));
        assert!(fx.is_empty(), "{fx:?}");
        let notes = |time: &TimeHost| time.project().tracks[SYNTH].note_fx;
        assert_eq!(notes(&time).items(), [Item::Fx(0), Item::Arp]);
        let id = notes(&time).ids[0];
        let st = shown(&mut p, &app, &time, &r);
        let ns = st.notes.as_ref().unwrap();
        assert!(!ns.picking);
        assert_eq!(ns.selected, Some(0));
        assert_eq!(st.selected, None, "no sound effect's page");
        // Its settings on the Q-Links: a turn sets the first (Shape).
        let q = st.main_four[0].addr;
        assert_eq!(q.owner, Owner::NoteFx(SYNTH as u8, id));
        let shape = |time: &TimeHost| notes(time).slots[0].unwrap().fx.get(0);
        let to = shape(&time) + 2;
        p.chain(
            &mut app,
            &mut r.h,
            &mut time,
            &ChainAction::Set {
                addr: q,
                value: to as f32 + 0.2,
            },
            false,
            &mut quiet,
        );
        assert_eq!(shape(&time), to);
        // Letting go of it hands it back to the engine, as a sound's knob:
        // nothing for the app to do.
        assert!(p
            .chain(
                &mut app,
                &mut r.h,
                &mut time,
                &ChainAction::Release(q),
                false,
                &mut quiet
            )
            .is_empty());
        // Off and on; below the arp and back; the arp can't go.
        act(
            &mut p,
            &mut app,
            &mut time,
            &mut r,
            NoteAction::SetOn { row: 0, on: false },
        );
        assert!(!notes(&time).slots[0].unwrap().on);
        act(
            &mut p,
            &mut app,
            &mut time,
            &mut r,
            NoteAction::Move { from: 0, to: 1 },
        );
        assert_eq!(notes(&time).items(), [Item::Arp, Item::Fx(0)]);
        assert_eq!(
            shown(&mut p, &app, &time, &r).notes.unwrap().selected,
            Some(1)
        );
        let fx = act(&mut p, &mut app, &mut time, &mut r, NoteAction::Remove(0));
        assert!(
            matches!(fx.as_slice(), [Effect::Tell(t)] if t.contains("arp stays")),
            "{fx:?}"
        );
        // Out: its page shuts.
        act(&mut p, &mut app, &mut time, &mut r, NoteAction::Remove(1));
        assert_eq!(notes(&time).items(), [Item::Arp]);
        assert_eq!(shown(&mut p, &app, &time, &r).notes.unwrap().selected, None);
        // A sound effect's page takes the place of a MIDI effect's.
        act(&mut p, &mut app, &mut time, &mut r, NoteAction::Select(0));
        p.chain(
            &mut app,
            &mut r.h,
            &mut time,
            &ChainAction::Select(0),
            false,
            &mut quiet,
        );
        assert_eq!(shown(&mut p, &app, &time, &r).notes.unwrap().selected, None);
        // The arp's page: its panel, on this track, armed.
        assert_eq!(time.armed(), Some(0));
        let fx = act(&mut p, &mut app, &mut time, &mut r, NoteAction::OpenArp);
        assert_eq!(
            fx.first(),
            Some(&Effect::Time(mpfree_time_ui::TimeAction::Pick(SYNTH))),
            "{fx:?}"
        );
        assert_eq!(app.view(), View::Arp);
        // No such track: no Notes stage.
        assert!(NotesSource::of(&app, &time, 99).is_none());
    }

    /// A MIDI effect's setting touched on its page takes a macro and an
    /// LFO row as a sound's knob does: the engine holds it (named for the
    /// macro cards), the page shows its macro and its row, a macro turn
    /// moves it in the sound loop, and the effect taken out takes all of
    /// that with it.
    #[test]
    fn a_midi_effect_setting_takes_a_macro_and_an_lfo_and_leaves_with_its_effect() {
        use crate::mixing::SYNTH;
        let mut r = Rig::new();
        let mut app = App::new(RATE, false);
        let mut time = TimeHost::new(RATE, 120.0).unwrap();
        app.start();
        let mut p = Panels::new();
        // Track 2's own strip (R3 part 4b: the Synth strip is no track's).
        p.open_insert(
            &mut app,
            &r.h,
            StripId::Track(mixing::track_channel(SYNTH)),
            None,
        );
        let act = |p: &mut Panels, app: &mut App, time: &mut TimeHost, r: &mut Rig, a| {
            p.chain(app, &mut r.h, time, &a, false, &mut |_| {})
        };
        let chance = Fx::all().iter().position(|f| f.name() == "Chance").unwrap();
        act(
            &mut p,
            &mut app,
            &mut time,
            &mut r,
            ChainAction::Notes(NoteAction::Picker(true)),
        );
        let fx = act(
            &mut p,
            &mut app,
            &mut time,
            &mut r,
            ChainAction::Notes(NoteAction::Add(chance)),
        );
        assert!(fx.is_empty(), "{fx:?}");
        let id = time.project().tracks[SYNTH].note_fx.ids[0];
        // The engine knows the tracks' MIDI effects as the sound loop has them.
        let table: [TrackFx; mpfree_seq::MAX_TRACKS] = std::array::from_fn(|t| {
            time.project()
                .tracks
                .get(t)
                .map_or_else(TrackFx::default, |tr| tr.note_fx)
        });
        r.h.modu.set_note_fx(&table);
        let addr = mpfree_chain_ui::notes::addr(SYNTH as u8, id, 0);
        // Touched, then on macro 2: from where it is to the far end.
        for a in [
            ChainAction::Focus(addr),
            ChainAction::Macro {
                target: addr,
                macro_: Some(1),
            },
        ] {
            let fx = act(&mut p, &mut app, &mut time, &mut r, a);
            assert!(fx.is_empty(), "{fx:?}");
        }
        assert_eq!(r.h.modu.engine().macro_of(addr), Some(1));
        let mut m = MacrosHost::new(0);
        let ms = m.state(&r.h);
        let t = &ms.macros[1].targets[0];
        assert_eq!(t.label, "Chance (Track 2)");
        let (from, to) = (t.from, t.to);
        assert!((from - 0.75).abs() < 1e-3 && to == 0.0, "{from} {to}");
        // An LFO row on it: the page shows the macro, the ring and the row.
        act(
            &mut p,
            &mut app,
            &mut time,
            &mut r,
            ChainAction::AddRow {
                target: addr,
                source: Source::Lfo(0),
            },
        );
        let st = shown(&mut p, &app, &time, &r);
        let pv = st.params.iter().find(|p| p.addr == addr).unwrap();
        assert_eq!(pv.macro_, Some(1));
        assert!(pv.mod_range.is_some());
        assert_eq!(st.focus, Some(addr));
        assert_eq!(st.rows.len(), 1);
        assert_eq!(st.rows[0].depth, ROW_DEPTH);
        // The row out, the macro all the way: the sound loop gets 0 %.
        act(
            &mut p,
            &mut app,
            &mut time,
            &mut r,
            ChainAction::RemoveRow(0),
        );
        r.h.modu.take_note_values().for_each(drop);
        m.action(&mut r.h, &MacroAction::Macro { m: 1, value: 1.0 }, false);
        r.run(20);
        let got: Vec<_> = r.h.modu.take_note_values().collect();
        assert!(
            got.last()
                .is_some_and(|&(a, v)| a == addr && v.abs() < 1e-3),
            "{got:?}"
        );
        // Taken out: its macro target and its page go, nothing is left.
        let fx = act(
            &mut p,
            &mut app,
            &mut time,
            &mut r,
            ChainAction::Notes(NoteAction::Remove(0)),
        );
        assert!(fx.is_empty(), "{fx:?}");
        assert!(!r.h.modu.engine().is_target(addr));
        assert!(m.state(&r.h).macros[1].targets.is_empty());
        let st = shown(&mut p, &app, &time, &r);
        assert!(st.rows.is_empty());
        assert_eq!(st.notes.unwrap().selected, None);
        assert_eq!(r.h.modu.unrouted(), 0);
    }

    /// Plays on opens the mixer's I/O view (the screen opens its MIDI
    /// section).
    #[test]
    fn plays_on_opens_the_io_view() {
        let r = Rig::new();
        let mut app = App::new(RATE, false);
        let mut time = TimeHost::new(RATE, 120.0).unwrap();
        app.start();
        let mut p = Panels::new();
        p.open_insert(&mut app, &r.h, drums(), None);
        let mut h = r;
        let fx = p.chain(
            &mut app,
            &mut h.h,
            &mut time,
            &ChainAction::Notes(NoteAction::PlaysOn),
            false,
            &mut |_| {},
        );
        assert_eq!(app.view(), View::Mixer);
        assert!(fx.contains(&Effect::Mixer(Some(MixView::Io))), "{fx:?}");
    }

    #[test]
    fn the_mixer_opens_the_editor_on_a_strip_and_adds_an_effect_from_the_picker() {
        let mut r = Rig::new();
        let mut c = ChainHost::from_mixer(drums(), None).unwrap();
        let st = c.state(&r.h);
        assert_eq!(st.title, "Drums FX");
        assert_eq!(st.back.as_deref(), Some("Mixer"));
        assert!(st.picking);
        assert_eq!(st.slots.len(), 1, "the Play page's DJ filter");
        assert_eq!(st.room, SLOTS);
        let out = c.action(&mut r.h, &ChainAction::Add("mpfree.fx.drive"), false);
        assert_eq!(out, ChainOut::Shown);
        r.run(20);
        let st = c.state(&r.h);
        assert!(!st.picking);
        assert_eq!(st.slots.len(), 2);
        assert_eq!(st.slots[1].name, "Drive · Soft");
        assert_eq!(st.selected, Some(1), "the added effect is selected");
        assert!(!st.params.is_empty());
        // Fill the strip: the next one is refused.
        for _ in 0..SLOTS - 2 {
            c.action(&mut r.h, &ChainAction::Add("mpfree.fx.crush"), false);
        }
        assert_eq!(
            c.action(&mut r.h, &ChainAction::Add("mpfree.fx.eq"), false),
            ChainOut::Refused("The chain is full")
        );
        assert_eq!(
            c.action(&mut r.h, &ChainAction::OpenPage(1), false),
            ChainOut::OpenPage {
                strip: drums(),
                slot: 1
            }
        );
        assert_eq!(
            c.action(&mut r.h, &ChainAction::Back, false),
            ChainOut::Back
        );
    }

    /// Layout spec 4b, "Sidechain": a Ducker added opens the sidechain
    /// list at once (it starts listening, with nothing picked). The list
    /// has only what can feed a track, the strip itself and one that
    /// listens to it greyed; a strip picked feeds the effect and turns it
    /// to listen, Off keeps the strip, and one the mixer would refuse is
    /// refused with why.
    #[test]
    fn a_sidechain_picked_in_the_list_feeds_the_effect() {
        use mpfree_fx::ids::{compressor, ducker};
        let mut r = Rig::new();
        let synth = StripId::Track(mixing::SYNTH);
        // The synth's compressor listens to the drums: the drums can't
        // listen to the synth (it would loop).
        let mut s = ChainHost::from_mixer(synth, None).unwrap();
        s.action(&mut r.h, &ChainAction::Add("mpfree.fx.compressor"), false);
        r.run(20);
        let st = s.state(&r.h);
        let comp = st.selected.unwrap();
        let control = st
            .params
            .iter()
            .find(|p| p.addr.param == compressor::SIDECHAIN)
            .unwrap()
            .addr;
        assert_eq!(
            s.action(
                &mut r.h,
                &ChainAction::SetSidechain {
                    control,
                    strip: Some(Strip::Track(DRUMS as u8)),
                },
                false
            ),
            ChainOut::Shown
        );
        r.run(4);
        assert_eq!(r.h.mixer.key(synth, comp), Some(drums()));
        assert_eq!(r.param(control), 1.0, "it listens");
        let st = s.state(&r.h);
        let row = &st.slots[comp];
        assert!(row.summary.starts_with("SC Drums"), "{}", row.summary);
        // The Ducker on the drums: the list opens on the tracks.
        let mut c = ChainHost::from_mixer(drums(), None).unwrap();
        c.action(&mut r.h, &ChainAction::Add("mpfree.fx.ducker"), false);
        r.run(20);
        let st = c.state(&r.h);
        let slot = st.selected.unwrap();
        let sc = st.sidechain.clone().expect("the Ducker's sidechain");
        assert!(sc.missing());
        assert_eq!(st.slots[slot].summary, "Nothing picked");
        assert!(st.slots[slot].warn);
        let list = st.list.clone().expect("the sidechain list, at once");
        let words = list.sidechain.clone().expect("its words");
        assert_eq!(list.group, 0);
        assert_eq!(
            &list.counts[3..],
            [0, 0, 0],
            "returns, submixes, the master"
        );
        assert!(list.counts[1] > 0 && list.counts[2] > 0, "pads and inputs");
        assert_eq!(
            words.note,
            "Returns and submixes come after this track in the mix, so they can't feed its sidechain."
        );
        assert_eq!(words.now, "Now: nothing picked");
        assert!(
            words.title.starts_with("SIDECHAIN FOR DRUMS · DUCKER"),
            "{}",
            words.title
        );
        let refused = |t: usize| {
            let p = list
                .strips
                .iter()
                .find(|p| p.strip == Strip::Track(t as u8));
            p.unwrap().refused
        };
        assert_eq!(refused(DRUMS), Some("this strip"));
        assert_eq!(refused(mixing::SYNTH), Some("it listens to this one"));
        // A pad picked: it feeds the Ducker, which listens.
        let control = st
            .params
            .iter()
            .find(|p| p.addr.param == ducker::SOURCE)
            .unwrap()
            .addr;
        let pick = |strip| ChainAction::SetSidechain { control, strip };
        assert_eq!(
            c.action(&mut r.h, &pick(Some(Strip::Pad(0))), false),
            ChainOut::Shown
        );
        r.run(4);
        assert_eq!(r.h.mixer.key(drums(), slot), Some(StripId::Pad(0)));
        let st = c.state(&r.h);
        assert!(st.list.is_none(), "the list shut");
        let sc = st.sidechain.unwrap();
        assert!(sc.listening && !sc.missing());
        assert_eq!(sc.track.unwrap().0, Strip::Pad(0));
        // Off (Tempo on the Ducker): the pad stays fed.
        c.action(&mut r.h, &pick(None), false);
        r.run(4);
        assert_eq!(r.param(control), 1.0, "Tempo");
        assert_eq!(r.h.mixer.key(drums(), slot), Some(StripId::Pad(0)));
        let st = c.state(&r.h);
        assert!(st.slots[slot].summary.starts_with("Tempo"));
        // Refused: the strip itself, and the synth (it listens to the drums).
        assert_eq!(
            c.action(&mut r.h, &pick(Some(Strip::Track(DRUMS as u8))), false),
            ChainOut::Refused("this strip")
        );
        assert_eq!(
            c.action(
                &mut r.h,
                &pick(Some(Strip::Track(mixing::SYNTH as u8))),
                false
            ),
            ChainOut::Refused("it listens to this one")
        );
        // The Mix tab's "SC from ..." opens the list on the pad's group.
        let mut m = ChainHost::from_mixer(drums(), Some(slot)).unwrap();
        m.list_sidechain(&r.h);
        let st = m.state(&r.h);
        assert_eq!(
            st.list.map(|l| (l.group, l.sidechain.is_some())),
            Some((1, true))
        );
        // The project keeps the pick.
        let text = crate::project::mixer_text(&r.h.mixer, r.h.pad_mutes());
        assert!(text.contains("key="), "{text}");
    }

    /// Jay, 16 Nov: a reverb or delay added on a send return starts at
    /// Mix 100 %; the same on a track keeps its default, and a preset from
    /// the picker leaves the return wet.
    #[test]
    fn an_effect_added_on_a_return_starts_wet_and_on_a_track_does_not() {
        use mpfree_fx::ids::{delay, plate};
        let mut r = Rig::new();
        let ret = StripId::Return(1);
        let mut c = ChainHost::from_mixer(ret, None).expect("Return B's chain");
        assert_eq!(c.state(&r.h).title, "Return B FX");
        c.action(&mut r.h, &ChainAction::Add("mpfree.fx.plate"), false);
        c.action(
            &mut r.h,
            &ChainAction::AddPreset {
                id: "mpfree.fx.delay",
                preset: 0,
            },
            false,
        );
        r.run(20);
        let chain = r.h.mixer.chain(ret).unwrap();
        let mix = |fx: &str, id: u16| {
            (0..chain.len())
                .filter_map(|i| chain.node(i))
                .find(|n| n.info().id == fx)
                .unwrap_or_else(|| panic!("{fx} on Return B"))
                .param(id)
        };
        assert_eq!(chain.len(), 2);
        assert_eq!(mix("mpfree.fx.plate", plate::MIX), 100.0, "the plate");
        assert_eq!(
            mix("mpfree.fx.delay", delay::MIX),
            100.0,
            "the delay's preset"
        );
        let mut c = ChainHost::from_mixer(drums(), None).unwrap();
        c.action(&mut r.h, &ChainAction::Add("mpfree.fx.plate"), false);
        r.run(20);
        let chain = r.h.mixer.chain(drums()).unwrap();
        let plate_on_drums = chain.node(chain.len() - 1).unwrap();
        assert_eq!(plate_on_drums.info().id, "mpfree.fx.plate");
        assert_eq!(plate_on_drums.param(plate::MIX), 25.0, "a track keeps it");
    }

    #[test]
    fn the_picker_adds_with_a_preset_and_keeps_favourites_and_recents() {
        let mut r = Rig::new();
        let mut c = ChainHost::from_mixer(drums(), None).unwrap();
        let preset = &mpfree_fx::picker::presets("mpfree.fx.plate")[1];
        let add = ChainAction::AddPreset {
            id: "mpfree.fx.plate",
            preset: 1,
        };
        assert_eq!(c.action(&mut r.h, &add, false), ChainOut::Shown);
        r.run(20);
        let chain = r.h.mixer.chain(drums()).unwrap();
        let slot = chain.len() - 1;
        let node = chain.node(slot).unwrap();
        assert_eq!(node.info().id, "mpfree.fx.plate");
        for &(p, v) in preset.values {
            assert_eq!(node.param(p), v, "built with {}", preset.name);
        }
        // The add went to the front of Recent, as its family (Reverb's
        // tile); a star keeps a favourite.
        assert_eq!(r.h.shelf.recent, ["mpfree.fx.room"]);
        assert!(std::mem::take(&mut r.h.shelf_dirty));
        let star = ChainAction::Favourite {
            id: "mpfree.fx.gate",
            on: true,
        };
        c.action(&mut r.h, &star, false);
        assert!(r.h.shelf.is_favourite("mpfree.fx.gate"));
        assert!(std::mem::take(&mut r.h.shelf_dirty));
        c.action(&mut r.h, &star, false);
        assert!(!r.h.shelf_dirty, "nothing changed, nothing to save");
        let st = c.state(&r.h);
        assert!(
            st.shelf.is_favourite("mpfree.fx.gate"),
            "the picker shows it"
        );
        // An add that is refused stays out of Recent, with nothing to save.
        assert_eq!(
            c.action(&mut r.h, &ChainAction::Add("mpfree.fx.nothing"), false),
            ChainOut::Refused("No such effect")
        );
        assert_eq!(r.h.shelf.recent, ["mpfree.fx.room"]);
        assert!(!r.h.shelf_dirty);
    }

    /// The Grains effect as the device checks play it (R7, round 2:
    /// `grains-repeat` and `grains-feedback` in
    /// tools/mpc-one/app_checks.py): added on the Drums row from the picker
    /// under a repeating snare, Mix up to 100 %, Repeat 1/16, 1/32 and Off,
    /// each Quality; then Feedback at 100 % with the snare stopped: the
    /// trail holds on by itself and never grows, and with Feedback back at
    /// 0 it dies away.
    #[test]
    fn the_grains_effect_repeats_and_holds_its_trail_at_full_feedback_on_the_drums() {
        use mpfree_sampler::engine::PadOut;
        let mut r = Rig::new();
        let mut c = ChainHost::from_mixer(drums(), None).unwrap();
        assert_eq!(
            c.action(&mut r.h, &ChainAction::Add("mpfree.grains"), false),
            ChainOut::Shown
        );
        r.run(20);
        let st = c.state(&r.h);
        let slot = st.selected.unwrap();
        assert!(
            st.slots[slot].name.starts_with("Grains"),
            "{}",
            st.slots[slot].name
        );
        let addr = |name: &str| {
            st.params
                .iter()
                .find(|p| p.info.name == name)
                .unwrap_or_else(|| panic!("{name} on the Main page"))
                .addr
        };
        let (mix, feedback, repeat, quality) = (
            addr("Mix"),
            addr("Feedback"),
            addr("Repeat"),
            addr("Quality"),
        );
        assert_eq!(r.param(mix), 50.0, "Grains opens at Mix 50 %");
        let set = |c: &mut ChainHost, r: &mut Rig, a: ParamAddr, value: f32| {
            c.action(&mut r.h, &ChainAction::Set { addr: a, value }, false);
            c.action(&mut r.h, &ChainAction::Release(a), false);
            r.run(2);
            assert_eq!(r.param(a), value);
        };
        // Pad 2 of kit 1 into the Drums row: a snare (a 180 Hz body and a
        // burst of softened noise, dying away in about 80 ms) on every
        // eighth at 120 BPM, or nothing. The master's left side.
        let mut noise = 0x2545_f491_u32;
        let mut soft = 0f32;
        let mut play = |r: &mut Rig, seconds: usize, snare: bool| -> Vec<f32> {
            let z = [0f32; PERIOD];
            let (mut l, mut rr) = ([0f32; PERIOD], [0f32; PERIOD]);
            let periods = seconds * RATE as usize / PERIOD;
            let mut out = Vec::with_capacity(periods * PERIOD);
            for _ in 0..periods {
                let at = r.period * PERIOD as u64;
                if snare {
                    let mut pads = r.h.pads();
                    let (pl, pr) = pads.pad(0, 1).unwrap();
                    for i in 0..PERIOD {
                        noise ^= noise << 13;
                        noise ^= noise >> 17;
                        noise ^= noise << 5;
                        let t = ((at + i as u64) % (RATE as u64 / 4)) as f32 / RATE as f32;
                        soft += 0.35 * (noise as f32 / u32::MAX as f32 - 0.5 - soft);
                        let x = (0.5 * (std::f32::consts::TAU * 180.0 * t).sin() + 0.8 * soft)
                            * (-t / 0.08).exp();
                        pl[i] += x;
                        pr[i] += x;
                    }
                }
                let ctx = Ctx::new(RATE, at);
                r.h.period(&ctx, (&z, &z), (&z, &z), (&z, &z), &mut l, &mut rr);
                r.period += 1;
                out.extend_from_slice(&l);
            }
            assert!(out.iter().all(|x| x.is_finite()));
            out
        };
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        let seconds = |x: &[f32]| x.chunks(RATE as usize).map(rms).collect::<Vec<f32>>();
        // Mix 100 %: the cloud alone, once its memory is full.
        set(&mut c, &mut r, mix, 100.0);
        play(&mut r, 10, true);
        let clean = rms(&play(&mut r, 5, true));
        assert!(clean > 1e-3, "the cloud plays: {clean}");
        // Repeat 1/16, then 1/32 while it repeats, then Off.
        for (value, name) in [(4.0, "1/16"), (5.0, "1/32"), (0.0, "Off")] {
            set(&mut c, &mut r, repeat, value);
            let level = rms(&play(&mut r, 2, true));
            assert!(
                level > clean * 0.25,
                "repeat {name}: {level} against {clean}"
            );
        }
        // Each Quality about as loud as Clean.
        for (value, name) in [(1.0, "Tape"), (2.0, "Lo-fi"), (0.0, "Clean")] {
            set(&mut c, &mut r, quality, value);
            let level = rms(&play(&mut r, 6, true)[RATE as usize..]);
            let db = 20.0 * (level / clean).log10();
            assert!(db.abs() < 2.0, "{name}: {db:+.2} dB");
        }
        // Feedback 100 %, the snare for 4 s more, then stopped: the trail.
        set(&mut c, &mut r, feedback, 100.0);
        play(&mut r, 4, true);
        let trail = seconds(&play(&mut r, 12, false));
        let worst = trail.iter().fold(0f32, |m, x| m.max(*x));
        assert!(worst < trail[0] * 1.12, "it grew: {trail:?}");
        assert!(trail[11] > trail[0] * 0.1, "it held: {trail:?}");
        // Feedback 0: it dies away.
        set(&mut c, &mut r, feedback, 0.0);
        let after = seconds(&play(&mut r, 6, false));
        assert!(after[5] < trail[11] * 0.01, "it died away: {after:?}");
    }

    /// A preset from the header's list sets the effect to it, all but its
    /// Mix, and the header then names it.
    #[test]
    fn a_preset_from_the_header_loads_and_keeps_the_mix() {
        let mut r = Rig::new();
        let mut c = ChainHost::from_mixer(drums(), Some(0)).unwrap();
        c.action(&mut r.h, &ChainAction::Add("mpfree.fx.plate"), false);
        r.run(20);
        let st = c.state(&r.h);
        let slot = st.selected.unwrap();
        let mix = st
            .params
            .iter()
            .find(|p| p.info.name == "Mix")
            .unwrap()
            .addr;
        c.action(
            &mut r.h,
            &ChainAction::Set {
                addr: mix,
                value: 40.0,
            },
            false,
        );
        c.action(&mut r.h, &ChainAction::Release(mix), false);
        r.run(2);
        let st = c.state(&r.h);
        assert_eq!(st.preset, Some(0), "a new plate is at its defaults");
        let k = st
            .presets
            .iter()
            .position(|p| p.name == "Vocal plate")
            .unwrap();
        let values = st.presets[k].values.clone();
        // Not for the engine in the slot (a mode switch on its way): refused.
        let other = ChainAction::Preset {
            slot,
            id: "mpfree.fx.room",
            values: values.clone(),
        };
        assert_eq!(
            c.action(&mut r.h, &other, false),
            ChainOut::Refused("That effect has changed")
        );
        let id = st.presets[k].id;
        let out = c.action(&mut r.h, &ChainAction::Preset { slot, id, values }, false);
        assert_eq!(out, ChainOut::Shown);
        r.run(2);
        let st = c.state(&r.h);
        assert_eq!(st.preset, Some(k));
        assert!((r.param(mix) - 40.0).abs() < 1e-3, "its Mix stays");
    }

    #[test]
    fn a_knob_on_a_macro_follows_the_macro_and_its_effect_through_a_move_and_a_remove() {
        let mut r = Rig::new();
        let mut c = ChainHost::from_mixer(drums(), Some(0)).unwrap();
        c.action(&mut r.h, &ChainAction::Add("mpfree.fx.drive"), false);
        r.run(20);
        let st = c.state(&r.h);
        let knob = st.params.iter().find(|p| p.info.name == "Drive").unwrap();
        let addr = knob.addr;
        // A turn of a knob the engine doesn't hold goes straight to the effect.
        let v = knob.info.from_unit(0.25);
        c.action(&mut r.h, &ChainAction::Focus(addr), false);
        c.action(&mut r.h, &ChainAction::Set { addr, value: v }, false);
        r.run(2);
        assert!((r.param(addr) - v).abs() < 1e-3);
        assert!(!r.h.modu.engine().is_target(addr));
        // On macro 2 (index 1): from where it is to the top of its travel.
        let out = c.action(
            &mut r.h,
            &ChainAction::Macro {
                target: addr,
                macro_: Some(1),
            },
            false,
        );
        assert_eq!(out, ChainOut::Shown);
        let st = c.state(&r.h);
        let p = st.params.iter().find(|p| p.addr == addr).unwrap();
        assert_eq!(p.macro_, Some(1));
        assert_eq!(st.slots[1].macros, [1]);
        let mut m = MacrosHost::new(0);
        let ms = m.state(&r.h);
        let t = &ms.macros[1].targets[0];
        assert_eq!(t.label, "Drive (Drums)");
        assert!(
            (t.from - 0.25).abs() < 1e-3 && t.to == 1.0,
            "{} {}",
            t.from,
            t.to
        );
        // Turn the macro all the way: the knob goes to the top.
        m.action(&mut r.h, &MacroAction::Macro { m: 1, value: 1.0 }, false);
        r.run(20);
        assert!((r.param(addr) - knob.info.from_unit(1.0)).abs() < 1e-3);
        // Put the drive first: the editor keeps it selected, the macro still moves it.
        c.action(&mut r.h, &ChainAction::Move { from: 1, to: 0 }, false);
        r.run(20);
        let st = c.state(&r.h);
        assert_eq!(st.selected, Some(0));
        assert_eq!(st.slots[0].name, "Drive · Soft");
        m.action(&mut r.h, &MacroAction::Macro { m: 1, value: 0.0 }, false);
        r.run(20);
        assert!((r.param(addr) - v).abs() < 1e-3);
        // Take it out: the engine lets go of its parameters, nothing is left dangling.
        c.action(&mut r.h, &ChainAction::Remove(0), false);
        r.run(20);
        assert!(!r.h.modu.engine().is_target(addr));
        assert_eq!(m.state(&r.h).macros[1].targets.len(), 0);
        let st = c.state(&r.h);
        assert_eq!(st.slots.len(), 1);
        assert_eq!(st.selected, None, "the removed effect's page shut");
        m.action(&mut r.h, &MacroAction::Macro { m: 1, value: 1.0 }, false);
        r.run(4);
        assert_eq!(r.h.modu.unrouted(), 0);
    }

    #[test]
    fn matrix_rows_scenes_and_the_target_editor_reach_the_engine() {
        let mut r = Rig::new();
        let mut c = ChainHost::from_mixer(drums(), Some(0)).unwrap();
        let st = c.state(&r.h);
        let addr = st.params[0].addr;
        c.action(&mut r.h, &ChainAction::Focus(addr), false);
        c.action(
            &mut r.h,
            &ChainAction::AddRow {
                target: addr,
                source: Source::Lfo(0),
            },
            false,
        );
        let st = c.state(&r.h);
        assert_eq!(st.rows.len(), 1);
        assert_eq!(st.rows[0].depth, ROW_DEPTH);
        c.action(
            &mut r.h,
            &ChainAction::Depth {
                row: 0,
                depth: -0.5,
            },
            false,
        );
        assert_eq!(c.state(&r.h).rows[0].depth, -0.5);
        c.action(&mut r.h, &ChainAction::RemoveRow(0), false);
        assert!(c.state(&r.h).rows.is_empty());
        // The target editor's change of range and curve, then Remove.
        c.action(
            &mut r.h,
            &ChainAction::Macro {
                target: addr,
                macro_: Some(0),
            },
            false,
        );
        let mut m = MacrosHost::new(0);
        let map = MacroMap {
            target: addr,
            from: 0.9,
            to: 0.1,
            curve: MapCurve::Exp,
        };
        m.action(&mut r.h, &MacroAction::Map { m: 0, map }, false);
        let t = &m.state(&r.h).macros[0].targets[0];
        assert_eq!((t.from, t.to, t.curve), (0.9, 0.1, MapCurve::Exp));
        m.action(&mut r.h, &MacroAction::Unmap(addr), false);
        assert!(m.state(&r.h).macros[0].targets.is_empty());
        // Two scenes of the Loop strip's volume, and the morph between them.
        let vol = ParamAddr::new(Owner::Strip(Strip::Track(LOOP as u8)), mx::VOLUME);
        assert!(r.h.modu.hold(&r.h.mixer, vol));
        let mut cl = ChainHost::from_mixer(StripId::Track(LOOP), Some(0)).unwrap();
        cl.action(
            &mut r.h,
            &ChainAction::Set {
                addr: vol,
                value: -20.0,
            },
            false,
        );
        r.run(2);
        m.action(&mut r.h, &MacroAction::Capture(0), false);
        cl.action(
            &mut r.h,
            &ChainAction::Set {
                addr: vol,
                value: 0.0,
            },
            false,
        );
        r.run(2);
        m.action(&mut r.h, &MacroAction::Capture(1), false);
        let ms = m.state(&r.h);
        assert_eq!(ms.changes.len(), 1);
        assert_eq!(ms.changes[0].label, "Volume (Loop)");
        m.action(&mut r.h, &MacroAction::Morph(0.0), false);
        r.run(20);
        assert!((r.h.mixer.param(StripId::Track(LOOP), mx::VOLUME) + 20.0).abs() < 0.1);
        assert_eq!(
            m.action(&mut r.h, &MacroAction::Back, false),
            MacroOut::Back
        );
    }

    #[test]
    fn a_turn_while_recording_beats_a_step_lock_until_the_finger_lifts() {
        let mut r = Rig::new();
        let mut c = ChainHost::from_mixer(drums(), Some(0)).unwrap();
        let addr = c.state(&r.h).params[0].addr;
        let info = c.state(&r.h).params[0].info;
        let (locked, hand) = (info.from_unit(0.2), info.from_unit(0.8));
        r.h.lock(0, addr, Some(locked));
        r.run(4);
        assert!((r.param(addr) - locked).abs() < 1e-3);
        // Not recording: the step's lock keeps the knob.
        c.action(&mut r.h, &ChainAction::Set { addr, value: hand }, false);
        r.run(4);
        assert!((r.param(addr) - locked).abs() < 1e-3);
        // Recording: the hand beats the lock while it holds the knob.
        c.action(&mut r.h, &ChainAction::Set { addr, value: hand }, true);
        r.run(4);
        assert!((r.param(addr) - hand).abs() < 1e-3);
        assert!(r.h.modu.engine().is_touched(addr));
        c.action(&mut r.h, &ChainAction::Release(addr), true);
        r.run(4);
        assert!(!r.h.modu.engine().is_touched(addr));
        assert!((r.param(addr) - locked).abs() < 1e-3);
    }

    #[test]
    fn a_macro_turned_while_recording_lets_go_when_the_finger_lifts() {
        let mut r = Rig::new();
        let mut m = MacrosHost::new(0);
        let addr = ParamAddr::macro_value(2);
        // Recording: the turn is a hand on the macro...
        m.action(&mut r.h, &MacroAction::Macro { m: 2, value: 0.7 }, true);
        r.run(4);
        assert!(r.h.modu.engine().is_touched(addr));
        // ...until the finger lifts off its knob (it used to stay held).
        m.action(&mut r.h, &MacroAction::Release(2), true);
        r.run(4);
        assert!(!r.h.modu.engine().is_touched(addr));
    }

    #[test]
    fn a_pads_chain_switches_between_its_own_and_the_tracks() {
        let mut r = Rig::new();
        let mut c = ChainHost::for_pad(StripId::Pad(0), "Pad 1".into(), DRUMS).unwrap();
        let st = c.state(&r.h);
        assert_eq!(st.title, "Pad 1 FX");
        assert_eq!(st.scopes, ["Pad 1", "Drums"]);
        assert_eq!(st.scope, 0);
        assert!(st.slots.is_empty());
        assert_eq!(st.back, None);
        c.action(&mut r.h, &ChainAction::Scope(1), false);
        let st = c.state(&r.h);
        assert_eq!(st.title, "Drums FX");
        assert_eq!(st.scope, 1);
        assert_eq!(st.slots.len(), 1);
        let addr = st.params[0].addr;
        assert_eq!(
            c.action(&mut r.h, &ChainAction::Learn(addr), false),
            ChainOut::Learn(addr)
        );
        assert!(c.state(&r.h).learning);
        c.learned();
        assert!(!c.state(&r.h).learning);
    }

    #[test]
    fn the_set_up_saves_and_loads_with_the_macros_names() {
        let mut r = Rig::new();
        let mut c = ChainHost::from_mixer(drums(), Some(0)).unwrap();
        let addr = c.state(&r.h).params[0].addr;
        c.action(
            &mut r.h,
            &ChainAction::Macro {
                target: addr,
                macro_: Some(2),
            },
            false,
        );
        r.h.modu.rename(2, "Sweep");
        let text = r.h.modu.save();
        assert!(text.contains("name.2 = Sweep"), "{text}");
        let mut fresh = Rig::new();
        assert_eq!(fresh.h.modu.load(&fresh.h.mixer, &text), 0);
        assert_eq!(fresh.h.modu.names()[2], "Sweep");
        assert_eq!(fresh.h.modu.engine().macro_of(addr), Some(2));
        let ms = MacrosHost::new(0).state(&fresh.h);
        assert_eq!(ms.macros[2].name, "Sweep");
        assert_eq!(ms.macros[2].targets[0].label, label(&fresh.h, addr));
        fresh.run(2);
    }

    /// Periods with the app's once-a-period tending: built effects in,
    /// retired ones out, a switch's macros and rows after it.
    fn settle(r: &mut Rig) {
        for _ in 0..20 {
            r.run(1);
            assert_eq!(r.h.poll_built(), None);
        }
    }

    /// Adds `id` at the end of the drums' chain and selects it.
    fn added(r: &mut Rig, id: &'static str) -> (ChainHost, usize, u16) {
        let mut c = ChainHost::from_mixer(drums(), Some(0)).unwrap();
        c.action(&mut r.h, &ChainAction::Add(id), false);
        settle(r);
        let chain = r.h.mixer.chain(drums()).unwrap();
        let slot = chain.len() - 1;
        let uid = chain.uid(slot).unwrap();
        c.action(&mut r.h, &ChainAction::Select(slot), false);
        (c, slot, uid)
    }

    fn knob(st: &ChainState, name: &str) -> ParamAddr {
        st.params
            .iter()
            .find(|p| p.info.name == name)
            .unwrap_or_else(|| panic!("no {name} on the page"))
            .addr
    }

    /// Room to Plate on the Mode chooser: the Plate takes the Room's place
    /// and slot id (still selected, still bypassed), keeps the controls the
    /// two share, and takes over the Room's macro and matrix rows on them;
    /// back to Room puts back what the Room had.
    #[test]
    fn a_mode_across_engines_swaps_in_place_and_switching_back_restores() {
        let mut r = Rig::new();
        let (mut c, slot, uid) = added(&mut r, "mpfree.fx.room");
        let st = c.state(&r.h);
        let mv = st.mode.clone().expect("a Reverb has modes");
        assert_eq!(mv.family, "Reverb");
        assert_eq!(mv.current, Some(0));
        assert_eq!(st.slots[slot].name, "Reverb · Room");
        // Room's Decay to 1.5 s and Size to 80 %; Mix on macro 1, a row on
        // Decay, another on Early (a Plate has no Early).
        c.action(&mut r.h, &ChainAction::Group(0), false);
        let st = c.state(&r.h);
        let (decay, size, early) = (knob(&st, "Decay"), knob(&st, "Size"), knob(&st, "Early"));
        for (addr, v) in [(decay, 1500.0), (size, 80.0)] {
            c.action(&mut r.h, &ChainAction::Set { addr, value: v }, false);
        }
        r.run(2);
        let mix = ParamAddr::new(Owner::Insert(Strip::Track(DRUMS as u8), uid), 9);
        let on_macro = ChainAction::Macro {
            target: mix,
            macro_: Some(0),
        };
        assert_eq!(c.action(&mut r.h, &on_macro, false), ChainOut::Shown);
        for target in [decay, early] {
            let row = ChainAction::AddRow {
                target,
                source: Source::Lfo(0),
            };
            assert_eq!(c.action(&mut r.h, &row, false), ChainOut::Shown);
        }
        c.action(&mut r.h, &ChainAction::SetOn { slot, on: false }, false);
        settle(&mut r);
        let to_plate = ChainAction::SetMode { slot, mode: 1 };
        assert_eq!(c.action(&mut r.h, &to_plate, false), ChainOut::Shown);
        // A second switch before the first is in is refused.
        assert_eq!(
            c.action(&mut r.h, &ChainAction::SetMode { slot, mode: 2 }, false),
            ChainOut::Refused("The mixer is busy")
        );
        settle(&mut r);
        let chain = r.h.mixer.chain(drums()).unwrap();
        let plate = chain.node(slot).unwrap();
        assert_eq!(plate.info().id, "mpfree.fx.plate");
        assert_eq!(chain.uid(slot), Some(uid), "the same slot id");
        assert!(!chain.is_on(slot), "still bypassed");
        assert!((plate.param(1) - 80.0).abs() < 1e-3, "Size carried");
        // Decay's row came along, so the engine has its own value.
        let owner = Owner::Insert(Strip::Track(DRUMS as u8), uid);
        let base = |r: &Rig, p| r.h.modu.engine().base(ParamAddr::new(owner, p));
        assert_eq!(base(&r, 2), Some(1500.0), "Decay carried");
        let st = c.state(&r.h);
        assert_eq!(st.selected, Some(slot));
        assert_eq!(st.slots[slot].name, "Reverb · Plate");
        assert_eq!(st.mode.as_ref().unwrap().current, Some(1));
        // Mix is 8 on a Plate, 9 on a Room; Decay is 2 on both.
        let e = r.h.modu.engine();
        let maps: Vec<u16> = e.macro_maps(0).map(|m| m.target.param).collect();
        assert_eq!(maps, [8]);
        let rows: Vec<ParamAddr> = e.rows().map(|r| r.target).collect();
        assert_eq!(rows, [ParamAddr::new(owner, 2)], "Early's row dropped");
        // A learned Q-Link or MIDI control is told the same: Mix to 8,
        // Decay stays 2, Early nowhere.
        let landed = r.h.modes.take_landed();
        assert_eq!(landed.len(), 1);
        let to = |p| match landed[0].target(ParamAddr::new(owner, p)) {
            Moved::To(a, info) => Some((a.param, info.name)),
            Moved::Gone => None,
            Moved::Stays => panic!("{p} is the Room's"),
        };
        assert_eq!((to(9), to(2)), (Some((8, "Mix")), Some((2, "Decay"))));
        assert_eq!(to(early.param), None);
        assert!(r.h.modes.take_landed().is_empty(), "told once");
        // A different Decay on the Plate; back to Room puts back the Room's.
        let plate_decay = ParamAddr::new(owner, 2);
        c.action(
            &mut r.h,
            &ChainAction::Set {
                addr: plate_decay,
                value: 4000.0,
            },
            false,
        );
        settle(&mut r);
        c.action(&mut r.h, &ChainAction::SetMode { slot, mode: 0 }, false);
        settle(&mut r);
        let chain = r.h.mixer.chain(drums()).unwrap();
        let room = chain.node(slot).unwrap();
        assert_eq!(room.info().id, "mpfree.fx.room");
        assert!((room.param(1) - 80.0).abs() < 1e-3);
        assert_eq!(base(&r, 2), Some(1500.0), "the Room's Decay");
        let maps: Vec<u16> =
            r.h.modu
                .engine()
                .macro_maps(0)
                .map(|m| m.target.param)
                .collect();
        assert_eq!(maps, [9], "the macro back on the Room's Mix");
        // Nothing drives a control that isn't there.
        r.run(4);
        assert_eq!(r.h.modu.unrouted(), 0);
    }

    /// Two modes of one engine (Drive's Soft and Tube) only set its Mode,
    /// which the editor hides; a mode of another engine (Console) and back
    /// (Fuzz) keeps what the Drive had.
    #[test]
    fn a_mode_of_the_same_engine_sets_its_value_and_one_of_another_comes_back() {
        use mpfree_fx::ids::drive;
        let mut r = Rig::new();
        let (mut c, slot, uid) = added(&mut r, "mpfree.fx.drive");
        let st = c.state(&r.h);
        assert_eq!(st.slots[slot].name, "Drive · Soft");
        assert!(
            !st.params.iter().any(|p| p.addr.param == drive::MODE),
            "the chooser stands for Mode"
        );
        let amount = knob(&st, "Drive");
        let v = st
            .params
            .iter()
            .find(|p| p.addr == amount)
            .unwrap()
            .info
            .from_unit(0.7);
        c.action(
            &mut r.h,
            &ChainAction::Set {
                addr: amount,
                value: v,
            },
            false,
        );
        c.action(&mut r.h, &ChainAction::SetMode { slot, mode: 2 }, false);
        r.run(2);
        let node = |r: &Rig| {
            let c = r.h.mixer.chain(drums()).unwrap();
            (c.uid(slot), c.node(slot).unwrap().info().id)
        };
        assert_eq!(node(&r), (Some(uid), "mpfree.fx.drive"));
        let st = c.state(&r.h);
        assert_eq!(st.slots[slot].name, "Drive · Tube");
        assert_eq!(st.mode.as_ref().unwrap().current, Some(2));
        assert_eq!(
            c.action(&mut r.h, &ChainAction::SetMode { slot, mode: 2 }, false),
            ChainOut::Shown,
            "already Tube: nothing to do"
        );
        c.action(&mut r.h, &ChainAction::SetMode { slot, mode: 6 }, false);
        settle(&mut r);
        assert_eq!(node(&r), (Some(uid), "mpfree.fx.character"));
        assert_eq!(c.state(&r.h).slots[slot].name, "Drive · Console");
        c.action(&mut r.h, &ChainAction::SetMode { slot, mode: 4 }, false);
        settle(&mut r);
        assert_eq!(node(&r), (Some(uid), "mpfree.fx.drive"));
        let chain = r.h.mixer.chain(drums()).unwrap();
        let n = chain.node(slot).unwrap();
        assert_eq!(n.param(drive::MODE), 4.0, "Fuzz");
        assert!(
            (n.param(amount.param) - v).abs() < 1e-3,
            "the Drive's amount"
        );
        let st = c.state(&r.h);
        assert_eq!(st.slots[slot].name, "Drive · Fuzz");
        assert_eq!(st.selected, Some(slot));
    }

    #[test]
    fn a_slot_keeps_its_place_through_moves() {
        assert_eq!(moved(1, 1, 0), 0);
        assert_eq!(moved(0, 1, 0), 1);
        assert_eq!(moved(2, 0, 3), 1);
        assert_eq!(moved(3, 0, 2), 3);
    }
}
