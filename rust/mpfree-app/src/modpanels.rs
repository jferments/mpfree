//! The chain editor and the macros panel (`mpfree-chain-ui`) in the app:
//! what their actions do to the mixer and the mod engine, and the states
//! they show, read straight from the mixer's chains and the engine.
//!
//! The panels go on the app's screens after round 9 and the app's rename
//! (layout spec section 4b, approved 2026-11-11 22:34Z): the mixer's insert
//! row opens [`ChainHost::from_mixer`] on its slot with "‹ Mixer", the
//! mixer's "+" opens it with the picker up, the Sample tab's FX sub-tab
//! opens [`ChainHost::for_pad`], and a long press on a macro card opens
//! the macros panel ([`MacrosHost`]). The mixer's insert row, its "+" and
//! the long press are wired (round 10), and Sample > FX
//! ([`Panels::open_pad`]): stutters and repeats on one pad are Perform FX
//! on its own channel, bus effects the Drums track's chain with a macro as
//! their one knob.
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
//!
//! Building a state, adding an effect (its node allocates) and removing one
//! allocate; they run between periods, as the mixer panel's edits do.

// Sample > FX (`for_pad`) and the audio-thread picture are for later; the
// tests use them now.
#![allow(dead_code)]

use mpfree_chain_ui::{ChainAction, ChainState, MacroAction, MacrosState};
use mpfree_fx::{strip_params, Edit, StripId};
use mpfree_mod::{Change, MacroMap, MapCurve, Row};
use mpfree_node::{NodeEvent, Timed};
use mpfree_play::mpfree_param::{Owner, ParamAddr, Strip};

use std::time::{Duration, Instant};

use mpfree_mixer_ui::MixView;

use crate::app::{App, Effect, View};
use crate::gui::ToUi;
use crate::mixing::{self, MixHost, SLOTS};
use crate::modmix::{strip_id, strip_of};
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
    /// The action couldn't be done; the reason, for the hint line.
    Refused(&'static str),
}

/// The chain editor's side in the app: which chain it shows and what is
/// selected on it.
#[derive(Clone, Debug, PartialEq)]
pub struct ChainHost {
    /// The chains it can switch between (one from the mixer; the pad's and
    /// the track's in Sample > FX), and the one shown.
    scopes: Vec<(Strip, String)>,
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
}

impl ChainHost {
    fn new(scopes: Vec<(Strip, String)>, back: Option<String>) -> ChainHost {
        ChainHost {
            scopes,
            scope: 0,
            selected: None,
            group: 0,
            focus: None,
            back,
            picking: false,
            learning: false,
            landing: None,
        }
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

    /// The Sample tab's FX sub-tab: kit pad `pad`'s own chain first (the
    /// mixer's pad channel `pad`), and the track's it plays into.
    pub fn for_pad(pad: usize, track: usize) -> Option<ChainHost> {
        let scopes = [StripId::Pad(pad), StripId::Track(track)]
            .into_iter()
            .map(|id| Some((strip_of(id)?, mixing::name(id))))
            .collect::<Option<Vec<_>>>()?;
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
        st
    }

    /// Applies one action to the mixer and the engine. `recording`: the
    /// strip's track is recording (a knob turn then beats the step locks).
    pub fn action(&mut self, h: &mut MixHost, a: &ChainAction, recording: bool) -> ChainOut {
        let sid = self.sid();
        let len = h.mixer.chain(sid).map_or(0, |c| c.len());
        match *a {
            ChainAction::Select(i) => {
                self.selected = Some(i);
                self.group = 0;
                self.focus = None;
                self.picking = false;
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
                self.selected = match self.selected {
                    Some(s) if s > slot => Some(s - 1),
                    Some(s) if s == slot => s.checked_sub(1).or((len > 1).then_some(0)),
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
                h.shelf_dirty |= h.shelf.used(id);
                // Built off the audio thread in the app (control.rs), then
                // put in at the end of the chain.
                if let Err(why) = h.add_insert_with(sid, id, preset) {
                    return ChainOut::Refused(why);
                }
                // It lands once it is built and its strip ducks (about 10
                // ms); until then the old selection stays.
                if self.landing.is_none() {
                    self.landing = h
                        .mixer
                        .chain(sid)
                        .map(|c| (0..c.len()).filter_map(|i| c.uid(i)).collect());
                }
            }
            ChainAction::OpenPage(slot) => return ChainOut::OpenPage { strip: sid, slot },
            ChainAction::Group(g) => {
                self.group = g;
                self.focus = None;
            }
            ChainAction::Focus(addr) => self.focus = Some(addr),
            ChainAction::Set { addr, value } => set(h, addr, value, recording),
            ChainAction::Release(addr) => h.modu.release(addr),
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
            ChainAction::Picker(on) => self.picking = on,
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

    /// The selected effect's drawings for the page shown, in theme `t`
    /// (`mpfree_chain_ui::set_chain_picture`).
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
        let page = st.groups.get(st.group).copied().unwrap_or("");
        mpfree_chain_ui::picture(c, slot, page, t, h.mixer.rate(), bpm)
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
fn set(h: &mut MixHost, addr: ParamAddr, value: f32, recording: bool) {
    if h.modu.hand(addr, value, recording) {
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
        .or_else(|| crate::modmix::describe(&h.mixer, target).map(|(i, v)| i.to_unit(v)))
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
                h.modu
                    .hand(ParamAddr::macro_value(m as u8), value, recording);
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
                h.modu.hand(ParamAddr::morph_pos(0), p, false);
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
        }
    }

    /// The chain editor is open with learn armed (MIDI Learn listens only
    /// while it is, M5).
    pub fn learning(&self) -> bool {
        self.chain.as_ref().is_some_and(|h| h.learning)
    }

    /// A learned MIDI control moved `addr` to `value` (in the parameter's
    /// own units; 0 to 1 for a macro), set as a chain editor knob sets it,
    /// and the open panel shows it at once (M5).
    pub fn set_param(&mut self, mix: &mut MixHost, addr: ParamAddr, value: f32, recording: bool) {
        set(mix, addr, value, recording);
        self.sent_at = None;
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
        let Some(h) = ChainHost::from_mixer(strip, slot) else {
            return Vec::new();
        };
        self.over_mix = mix.shown().unwrap_or(MixView::Tracks);
        self.chain = Some(h);
        self.sent_at = None;
        app.open_chain()
    }

    /// The Sample tab's FX sub-tab: the chain editor on the playing kit's
    /// pad `pad`'s channel, with the track it plays into (`track`) as its
    /// second scope, in the Sample tab's place; its Back comes back to the
    /// Sample tab. A pad past the mixer's pad channels opens nothing and
    /// says so.
    pub fn open_pad(&mut self, app: &mut App, pad: usize, track: usize) -> Vec<Effect> {
        let host = (pad < mixing::PADS)
            .then(|| ChainHost::for_pad(pad, track))
            .flatten();
        let Some(h) = host else {
            return vec![Effect::Tell(format!(
                "chain: pad {} has no channel of its own in the mixer (only the first {} do)",
                pad + 1,
                mixing::PADS
            ))];
        };
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

    /// A chain editor action (a touch, or a Q-Link on its knobs).
    /// `show` takes what the screen needs past the next state (a learn).
    pub fn chain(
        &mut self,
        app: &mut App,
        mix: &mut MixHost,
        a: &ChainAction,
        recording: bool,
        show: &mut impl FnMut(ToUi),
    ) -> Vec<Effect> {
        let Some(h) = self.chain.as_mut() else {
            return Vec::new();
        };
        self.sent_at = None;
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
            ChainOut::Back => app.close_chain(self.over_mix),
            ChainOut::Refused(why) => vec![Effect::Tell(format!("chain: {why}"))],
        }
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
        bpm: f64,
        now: Instant,
        show: &mut impl FnMut(ToUi),
    ) {
        if !matches!(view, View::Chain | View::Macros)
            || self
                .sent_at
                .is_some_and(|t| now.duration_since(t) < PANELS_EVERY)
        {
            return;
        }
        self.sent_at = Some(now);
        if view == View::Chain {
            if let Some(h) = self.chain.as_mut() {
                let st = h.state(mix);
                let fx = h.fx_state(mix, &st, bpm);
                show(ToUi::Chain(Box::new(st)));
                show(ToUi::ChainFx(fx.map(Box::new)));
            }
        }
        show(ToUi::Macros(Box::new(self.macros.state(mix))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixing::{DRUMS, LOOP};
    use mpfree_fx::ids::mixer as mx;
    use mpfree_mod::Source;
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
                    .period(&ctx, (&z, &z), (&z, &z), (&z, &z), (&z, &z), &mut l, &mut r);
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

    /// The Sample tab's FX: the chain editor on the pad's own channel, so an
    /// effect added there (Perform FX: stutters, repeats, stops) plays on
    /// that pad alone, and on the Drums track for every pad; Back goes back
    /// to the Sample tab.
    #[test]
    fn the_sample_tabs_fx_edits_the_pads_own_chain_and_goes_back_to_the_sample_tab() {
        let mut r = Rig::new();
        let mut app = App::new(RATE, false);
        app.start();
        app.open_tab(View::Sample);
        let mut p = Panels::new();
        p.open_pad(&mut app, 4, DRUMS);
        assert_eq!(app.view(), View::Chain);
        let mut sent = Vec::new();
        p.poll(app.view(), &r.h, 120.0, Instant::now(), &mut |m| {
            sent.push(m)
        });
        let title = sent.iter().find_map(|m| match m {
            ToUi::Chain(st) => Some((st.title.clone(), st.scopes.clone())),
            _ => None,
        });
        assert_eq!(
            title,
            Some((
                "Pad 5 FX".to_string(),
                vec!["Pad 5".to_string(), "Drums".to_string()]
            ))
        );
        let len = |r: &Rig, s: StripId| r.h.mixer.chain(s).map_or(0, |c| c.len());
        let drums_before = len(&r, drums());
        let mut quiet = |_: ToUi| {};
        p.chain(
            &mut app,
            &mut r.h,
            &ChainAction::Add("mpfree.fx.perform"),
            false,
            &mut quiet,
        );
        r.run(20);
        assert_eq!(len(&r, StripId::Pad(4)), 1, "Perform on pad 5's channel");
        assert_eq!(len(&r, StripId::Pad(3)), 0);
        assert_eq!(len(&r, drums()), drums_before);
        p.chain(
            &mut app,
            &mut r.h,
            &ChainAction::Scope(1),
            false,
            &mut quiet,
        );
        p.chain(
            &mut app,
            &mut r.h,
            &ChainAction::Add("mpfree.fx.drive"),
            false,
            &mut quiet,
        );
        r.run(20);
        assert_eq!(len(&r, drums()), drums_before + 1, "Drive on the Drums bus");
        p.chain(&mut app, &mut r.h, &ChainAction::Back, false, &mut quiet);
        assert_eq!(app.view(), View::Sample);
        // A pad past the 16 channels opens nothing and says why.
        let fx = p.open_pad(&mut app, 16, DRUMS);
        assert!(
            matches!(&fx[..], [Effect::Tell(n)] if n.contains("only the first 16")),
            "{fx:?}"
        );
        assert_eq!(app.view(), View::Sample);
    }

    /// The audio loop's side: the mixer's insert row opens the editor in the
    /// mixer's place, its states go out at once and then every
    /// [`PANELS_EVERY`] while it shows, a learn reaches the screen, and Back
    /// opens the mixer on the view it showed. The macros panel the same.
    #[test]
    fn the_panels_open_from_the_mixer_send_their_states_and_go_back() {
        let mut r = Rig::new();
        let mut app = App::new(RATE, false);
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
        p.poll(app.view(), &r.h, 120.0, t0, &mut |m| sent.push(m));
        assert_eq!(kinds(&sent), ["chain Drums FX", "picture", "macros 0"]);
        sent.clear();
        p.poll(app.view(), &r.h, 120.0, t0 + PANELS_EVERY / 2, &mut |m| {
            sent.push(m)
        });
        assert!(sent.is_empty(), "not yet");
        // Learn on the knob last touched: the screen arms it.
        let st = p.chain.as_mut().unwrap().state(&r.h);
        let knob = st.params[0].addr;
        p.chain(
            &mut app,
            &mut r.h,
            &ChainAction::Focus(knob),
            false,
            &mut |m| sent.push(m),
        );
        p.chain(
            &mut app,
            &mut r.h,
            &ChainAction::Learn(knob),
            false,
            &mut |m| sent.push(m),
        );
        assert_eq!(kinds(&sent), ["learn"]);
        sent.clear();
        p.learned();
        // A change shows at once.
        p.poll(app.view(), &r.h, 120.0, t0 + PANELS_EVERY / 2, &mut |m| {
            sent.push(m)
        });
        assert_eq!(sent.len(), 3);
        sent.clear();
        // Back: the mixer again, on Pad Mix.
        let fx = p.chain(&mut app, &mut r.h, &ChainAction::Back, false, &mut |m| {
            sent.push(m)
        });
        assert_eq!(
            fx.first(),
            Some(&Effect::Mixer(Some(MixView::Pads))),
            "{fx:?}"
        );
        assert_eq!(app.view(), View::Mixer);
        r.h.show(Some(MixView::Pads));
        p.poll(app.view(), &r.h, 120.0, t0 + PANELS_EVERY * 3, &mut |m| {
            sent.push(m)
        });
        assert!(sent.is_empty(), "nothing while the panels are shut");
        // The macros panel over the mixer, on Macros 5-8; Back to Pad Mix.
        p.open_macros(&mut app, &mut r.h, 1);
        assert_eq!(app.view(), View::Macros);
        p.poll(app.view(), &r.h, 120.0, t0 + PANELS_EVERY * 3, &mut |m| {
            sent.push(m)
        });
        assert_eq!(kinds(&sent), ["macros 1"]);
        let fx = p.macros(&mut app, &mut r.h, &MacroAction::Back, false);
        assert_eq!(
            fx.first(),
            Some(&Effect::Mixer(Some(MixView::Pads))),
            "{fx:?}"
        );
        assert_eq!(app.view(), View::Mixer);
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
        assert_eq!(st.slots[1].name, "Drive");
        assert_eq!(st.selected, Some(1), "the added effect is selected");
        assert!(!st.params.is_empty());
        // Fill the strip: the next one is refused.
        for _ in 0..2 {
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
        // The add went to the front of Recent; a star keeps a favourite.
        assert_eq!(r.h.shelf.recent, ["mpfree.fx.plate"]);
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
        assert_eq!(st.slots[0].name, "Drive");
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
        assert_eq!(st.selected, Some(0));
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
    fn sample_fx_switches_between_the_pads_chain_and_the_tracks() {
        let mut r = Rig::new();
        let mut c = ChainHost::for_pad(0, DRUMS).unwrap();
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

    #[test]
    fn a_slot_keeps_its_place_through_moves() {
        assert_eq!(moved(1, 1, 0), 0);
        assert_eq!(moved(0, 1, 0), 1);
        assert_eq!(moved(2, 0, 3), 1);
        assert_eq!(moved(3, 0, 2), 3);
    }
}
