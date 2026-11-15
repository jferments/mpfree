//! The chain editor and the macros panel on the app's screens, the GUI's
//! side (layout spec section 4b, approved by Jay 2026-11-11 22:34Z).
//!
//! The panels are `mpfree-chain-ui`'s; what their actions do to the mixer
//! and the mod engine is `modpanels.rs`, on the audio thread. This keeps
//! what the screen needs between their states:
//!
//! - The Q-Link column's bindings ([`Bindings`]): on the chain editor the
//!   selected effect's first four knobs, with Macros 1-4 and 5-8 as banks
//!   (the Q-LINK button steps through them); on the macros panel
//!   the four macros shown; on Play, Macros 1-4 and 5-8 as banks over
//!   Play's own knobs, Play keeping its own bank. The Q-LINK button steps
//!   through the banks ([`ChainPage::next_bank`]). A turn becomes the
//!   action to send ([`ChainPage::turn`]).
//! - The long press on a macro card that opens the macros panel
//!   ([`ChainPage::press`], [`ChainPage::held`]).
//! - The selected effect's drawings, made here from the effect's values in
//!   the screen's theme ([`ChainPage::set_fx`]).

use mpfree_chain_ui::{ChainAction, ChainState, MacroAction, MacrosState};
use mpfree_fx_ui::{visual, FxState, Scratch};
use mpfree_mod::{Bindings, KNOBS};
use mpfree_play::mpfree_param::{Owner, ParamAddr};
use mpfree_ui::qlink::{QLinkCard, DRAG_DETENT};
use mpfree_ui::{Theme, Ui};

use crate::app::View;

/// How long a finger stays on a macro card before the macros panel opens.
pub const LONG_PRESS_S: f32 = 0.5;
/// The macro banks: Macros 1-4 and 5-8 (bank 0 is the page's own knobs).
pub const MACRO_BANKS: usize = 2;
/// The bindings' page key on Play (its macro banks).
pub const PLAY_PAGE: &str = "play";

/// What a Q-Link turn or a touch on the column means, for the audio thread.
#[derive(Clone, Debug, PartialEq)]
pub enum Turned {
    /// A knob of the chain editor's effect moved.
    Chain(ChainAction),
    /// A macro moved.
    Macro(MacroAction),
    /// A learn took this knob: the chain editor stops showing "learn".
    Learned,
}

/// The GUI's side of the chain editor and the macros panel.
pub struct ChainPage {
    /// The Q-Link column's banks and the values of what they move.
    pub bind: Bindings,
    /// What the chain editor shows, for learns, labels and the drawings.
    pub chain: Option<ChainState>,
    pub macros: Option<MacrosState>,
    /// A finger on a Q-Link card: (finger, card, where it came down,
    /// seconds held, moved).
    press: Option<(u64, usize, i32, f32, bool)>,
    /// The page shown, and the bank Play showed when it was left.
    at: View,
    play_bank: usize,
    /// An effect is picked on the mixer's Effects look: the column shows
    /// its knobs there.
    mix_pick: bool,
}

impl Default for ChainPage {
    fn default() -> Self {
        ChainPage::new()
    }
}

impl ChainPage {
    pub fn new() -> ChainPage {
        ChainPage {
            bind: Bindings::with_macros(&mpfree_chain_ui::state::default_macro_names()),
            chain: None,
            macros: None,
            press: None,
            at: View::Play,
            play_bank: 0,
            mix_pick: false,
        }
    }

    /// Adds the panels and their pages (after the header, the Q-Link
    /// column and the hint line).
    pub fn add(ui: &mut Ui) -> Result<(), String> {
        mpfree_chain_ui::add(ui)
    }

    /// The chain editor's new state: its knobs become the page's Q-Links.
    pub fn set_chain(&mut self, ui: &mut Ui, st: ChainState) {
        let (name, knobs) = st.qlinks();
        self.bind
            .set_page(mpfree_chain_ui::QLINK_PAGE, &name, knobs);
        // Main's four turn on every page, so their values come along.
        for p in st.params.iter().chain(&st.main_four) {
            self.bind.observe(p.addr, p.value);
        }
        mpfree_chain_ui::update_chain(ui, st.clone());
        self.chain = Some(st);
    }

    /// The macros' new state: their values and names for the macro banks'
    /// cards, and the panel.
    pub fn set_macros(&mut self, ui: &mut Ui, st: MacrosState) {
        for (m, mv) in st.macros.iter().enumerate() {
            self.bind.observe(ParamAddr::macro_value(m as u8), mv.value);
        }
        for bank in 1..=MACRO_BANKS {
            for knob in self.bind.bank_mut(bank).knobs.iter_mut().flatten() {
                let Owner::Macro(m) = knob.target.owner else {
                    continue;
                };
                match st.macros.get(m as usize) {
                    Some(mv) if !mv.name.is_empty() && knob.label != mv.name => {
                        knob.label = mv.name.clone();
                    }
                    _ => {}
                }
            }
        }
        mpfree_chain_ui::update_macros(ui, st.clone());
        self.macros = Some(st);
    }

    /// The selected effect's drawings, from its values (none for an effect
    /// the pages don't draw, or none selected), in theme `t`: every one its
    /// effect page draws (page `""`), each section of the chain editor
    /// showing those its plan names.
    pub fn set_fx(&self, ui: &mut Ui, fx: Option<FxState>, t: &Theme) {
        let cards = match fx {
            Some(mut st) if visual::has_visuals(st.info.id) => {
                st.color = t.accent;
                let scratch = Scratch::new(&st);
                visual::strip(&st, "", None, &scratch, t).cards
            }
            _ => Vec::new(),
        };
        mpfree_chain_ui::set_chain_picture(ui, cards);
    }

    /// The page shown changed: the chain editor starts on its own knobs,
    /// the macros panel on the bank it opened with, and Play on the bank it
    /// showed when it was left. Arrange shares Play's knobs and bank (the
    /// same column, layout spec section 4c). A finger on a card is
    /// forgotten.
    pub fn entered(&mut self, v: View) {
        if v == self.at {
            return;
        }
        if matches!(self.at, View::Play | View::Arrange) {
            self.play_bank = self.bind.shown();
        }
        self.press = None;
        self.at = v;
        match v {
            View::Chain => {
                if let Some(st) = &self.chain {
                    let (name, knobs) = st.qlinks();
                    self.bind
                        .set_page(mpfree_chain_ui::QLINK_PAGE, &name, knobs);
                }
                self.bind.show(0);
            }
            View::Macros => {
                let bank = self.macros.as_ref().map_or(0, |m| m.bank);
                self.bind.show(1 + bank.min(MACRO_BANKS - 1));
            }
            View::Play | View::Arrange => {
                // Play's own knobs are the Play page's; the bindings hold
                // only its macro banks (a knob learned on the chain editor
                // stays there).
                self.bind.set_page(PLAY_PAGE, "Play", Default::default());
                self.bind.show(self.play_bank);
            }
            _ => {}
        }
    }

    /// An effect picked on the mixer's Effects look (`on`), or the pick
    /// gone: the column shows the picked effect's knobs on the mixer once
    /// its state comes (until then, none: a turn must not reach the
    /// effect picked before).
    pub fn pick_on_mixer(&mut self, on: bool) {
        self.mix_pick = on;
        if on {
            self.chain = None;
            self.bind
                .set_page(mpfree_chain_ui::QLINK_PAGE, "", Default::default());
            self.bind.show(0);
        }
    }

    /// The macros panel picked its bank (0: Macros 1-4, 1: 5-8): the
    /// column follows while the panel shows.
    pub fn panel_bank(&mut self, bank: usize) {
        if self.at == View::Macros {
            self.bind.show(1 + bank.min(MACRO_BANKS - 1));
        }
    }

    /// The Q-LINK button: the next bank. The macros panel shows Macros 1-4
    /// and 5-8 only, and tells the panel which.
    pub fn next_bank(&mut self, v: View) -> Option<Turned> {
        if v == View::Macros {
            let b = if self.bind.shown() == 1 { 2 } else { 1 };
            self.bind.show(b);
            return Some(Turned::Macro(MacroAction::Bank(b - 1)));
        }
        self.bind.next_bank();
        None
    }

    /// Whether the column shows a macro bank rather than the page's own
    /// knobs.
    pub fn macro_bank_shown(&self) -> bool {
        self.bind.shown() > 0
    }

    /// Whether the column shows these bindings' cards on page `v`: always
    /// on the chain editor and the macros panel, on Play and Arrange while
    /// a macro bank shows.
    pub fn drives(&self, v: View) -> bool {
        match v {
            View::Chain | View::Macros => true,
            View::Mixer => self.mix_pick,
            View::Play | View::Arrange => self.at == v && self.macro_bank_shown(),
            _ => false,
        }
    }

    /// Q-Link `k` turned by `detents` on a page these bindings drive: the
    /// action for the audio thread, if anything moved.
    pub fn turn(&mut self, k: usize, detents: i32, fine: bool) -> Option<Turned> {
        let learning = self.bind.learning().is_some();
        let w = self.bind.turn(k, detents, fine);
        if learning && self.bind.learning().is_none() {
            return Some(Turned::Learned);
        }
        let w = w?;
        Some(match w.addr.owner {
            Owner::Macro(m) => Turned::Macro(MacroAction::Macro {
                m: m as usize,
                value: w.value,
            }),
            _ => Turned::Chain(ChainAction::Set {
                addr: w.addr,
                value: w.value,
            }),
        })
    }

    /// A finger left Q-Link `k`: a knob of the effect lets go of its
    /// parameter (the engine's `Untouch` after a recorded move).
    pub fn released(&self, k: usize) -> Option<Turned> {
        let c = self.bind.card(k);
        match c.target {
            Some(a) if !matches!(a.owner, Owner::Macro(_)) => {
                Some(Turned::Chain(ChainAction::Release(a)))
            }
            _ => None,
        }
    }

    /// Learn armed on the chain editor: the next Q-Link turned takes the
    /// knob (on the chain editor's page only).
    pub fn learn(&mut self, addr: ParamAddr) {
        let Some(p) = self
            .chain
            .as_ref()
            .and_then(|c| c.params.iter().find(|p| p.addr == addr))
        else {
            return;
        };
        self.bind.learn(addr, p.info, p.info.name);
    }

    /// Learn taken by something else (a MIDI control, M5): no Q-Link takes
    /// the knob now.
    pub fn cancel_learn(&mut self) {
        self.bind.cancel_learn();
    }

    /// The column's title and cards: what each knob moves, in the theme's
    /// colours (a macro's knob wears its macro's colour).
    pub fn cards(&self, t: &Theme) -> (String, [QLinkCard; KNOBS]) {
        let title = match self.bind.title().as_str() {
            "" => "Q-Links".to_string(),
            name => format!("Q-Links · {name}"),
        };
        let page = self.bind.title();
        let cards = std::array::from_fn(|k| {
            let c = self.bind.card(k);
            let Some(target) = c.target else {
                let mut free = QLinkCard::new("", "", "", 0.0);
                if !c.sub.is_empty() {
                    free.value = c.sub.to_string();
                }
                return free;
            };
            let scope = match target.owner {
                Owner::Macro(m) => format!("Macro {}", m + 1),
                _ => page.clone(),
            };
            let value = if c.sub.is_empty() {
                c.text.clone()
            } else {
                c.sub.to_string()
            };
            let mut card = QLinkCard::new(&c.label, &scope, &value, c.pos);
            card.color = c
                .colour
                .map(|i| mpfree_chain_ui::macro_color(t, i as usize));
            card
        });
        (title, cards)
    }

    /// A finger came down at height `y` on Q-Link card `k` (None: off the
    /// cards).
    pub fn press(&mut self, finger: u64, k: Option<usize>, y: i32) {
        self.press = k.map(|k| (finger, k, y, 0.0, false));
    }

    /// The finger moved to height `y`: a drag far enough to turn the card's
    /// knob is not a long press.
    pub fn moved(&mut self, finger: u64, y: i32) {
        if let Some(p) = self.press.as_mut().filter(|p| p.0 == finger) {
            if (y - p.2).abs() >= DRAG_DETENT {
                p.4 = true;
            }
        }
    }

    /// The finger lifted.
    pub fn lifted(&mut self, finger: u64) {
        if self.press.is_some_and(|p| p.0 == finger) {
            self.press = None;
        }
    }

    /// Time passed: a finger held still on a macro's card for
    /// [`LONG_PRESS_S`] opens the macros panel on that macro's bank (0 for
    /// 1-4, 1 for 5-8). Once per press.
    pub fn held(&mut self, dt: f32) -> Option<usize> {
        let (finger, k, y, t, moved) = self.press?;
        let t = t + dt;
        self.press = Some((finger, k, y, t, moved));
        if moved || t < LONG_PRESS_S {
            return None;
        }
        self.press = Some((finger, k, y, f32::NEG_INFINITY, true));
        match self.bind.card(k).target?.owner {
            Owner::Macro(m) => Some(m as usize / KNOBS),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_chain_ui::sample::{Song, DRUMS};
    use mpfree_ui::Fonts;

    fn chain_state() -> ChainState {
        Song::new().chain_state(DRUMS, 0, 0, None)
    }

    #[test]
    fn the_chain_editors_knobs_are_the_q_links_and_turns_set_them() {
        let mut p = ChainPage::new();
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        let st = chain_state();
        let first = st.params[0].clone();
        p.set_chain(&mut ui, st);
        p.entered(View::Chain);
        let (title, cards) = p.cards(&Theme::dark());
        assert!(title.starts_with("Q-Links · "), "{title}");
        assert_eq!(cards[0].name, first.info.name);
        match p.turn(0, 1, false) {
            Some(Turned::Chain(ChainAction::Set { addr, .. })) => assert_eq!(addr, first.addr),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            p.released(0),
            Some(Turned::Chain(ChainAction::Release(first.addr)))
        );
    }

    /// On any page of an effect with more than one (the Channel strip's
    /// Gate page here) the Q-Links turn the four its plan picks, from its
    /// Main page, and their cards show their values.
    #[test]
    fn the_q_links_stay_on_main_four_on_every_page() {
        let mut p = ChainPage::new();
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        let mut song = Song::new();
        let strip = mpfree_fx::registry::make("mpfree.fx.channel_strip").unwrap();
        let slot = song.keys.push(strip);
        let st = song.chain_state(mpfree_chain_ui::sample::KEYS, slot, 1, None);
        assert_eq!(st.groups[st.group], "Gate");
        let four = st.main_four.clone();
        assert_eq!(four.len(), 4);
        p.set_chain(&mut ui, st);
        p.entered(View::Chain);
        let (_, cards) = p.cards(&Theme::dark());
        let names: Vec<&str> = cards.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Comp thresh", "Makeup", "Low cut", "High gain"]);
        match p.turn(3, 1, false) {
            Some(Turned::Chain(ChainAction::Set { addr, .. })) => assert_eq!(addr, four[3].addr),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_q_link_button_steps_to_the_macro_banks_and_back() {
        let mut p = ChainPage::new();
        p.entered(View::Chain);
        assert_eq!(p.next_bank(View::Chain), None);
        let (title, cards) = p.cards(&Theme::dark());
        assert_eq!(title, "Q-Links · Macros 1-4");
        assert_eq!(cards[0].scope, "Macro 1");
        assert!(cards[0].color.is_some());
        match p.turn(1, 2, false) {
            Some(Turned::Macro(MacroAction::Macro { m: 1, value })) => assert!(value > 0.0),
            other => panic!("{other:?}"),
        }
        assert_eq!(p.released(1), None);
        // The macros' own names, once their state comes.
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        p.set_macros(&mut ui, Song::new().macros_state(0));
        let (_, cards) = p.cards(&Theme::dark());
        assert_eq!(cards[0].name, Song::new().names[0]);
        assert_ne!(cards[0].name, "Macro 1");
        p.next_bank(View::Chain);
        assert_eq!(p.cards(&Theme::dark()).0, "Q-Links · Macros 5-8");
        p.next_bank(View::Chain);
        assert!(!p.macro_bank_shown());
    }

    #[test]
    fn the_macros_panel_shows_only_macro_banks_and_tells_the_panel() {
        let mut p = ChainPage::new();
        p.entered(View::Macros);
        assert_eq!(p.bind.shown(), 1);
        assert_eq!(
            p.next_bank(View::Macros),
            Some(Turned::Macro(MacroAction::Bank(1)))
        );
        assert_eq!(
            p.next_bank(View::Macros),
            Some(Turned::Macro(MacroAction::Bank(0)))
        );
    }

    #[test]
    fn play_keeps_its_own_bank_and_shows_the_cards_only_on_a_macro_bank() {
        let mut p = ChainPage::new();
        p.entered(View::Play);
        assert!(
            !p.drives(View::Play),
            "Play's own knobs are the Play page's"
        );
        p.next_bank(View::Play);
        assert!(p.drives(View::Play));
        assert_eq!(p.cards(&Theme::dark()).0, "Q-Links · Macros 1-4");
        // The chain editor starts on its own knobs and leaves Play's bank.
        p.entered(View::Chain);
        assert!(!p.macro_bank_shown());
        assert!(p.drives(View::Chain));
        p.entered(View::Play);
        assert_eq!(p.bind.shown(), 1);
        // The macros panel's own bank switch moves the column while it shows.
        p.entered(View::Macros);
        p.panel_bank(1);
        assert_eq!(p.cards(&Theme::dark()).0, "Q-Links · Macros 5-8");
        assert!(!p.drives(View::Synth));
    }

    #[test]
    fn a_learn_takes_the_next_knob_turned() {
        let mut p = ChainPage::new();
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        let st = chain_state();
        let addr = st.params[2].addr;
        p.set_chain(&mut ui, st);
        p.entered(View::Chain);
        p.next_bank(View::Chain);
        p.learn(addr);
        assert_eq!(p.cards(&Theme::dark()).1[3].value, "learn");
        assert_eq!(p.turn(3, 1, false), Some(Turned::Learned));
        assert_eq!(p.bind.card(3).target, Some(addr));
    }

    #[test]
    fn a_long_press_on_a_macro_card_opens_its_bank_once() {
        let mut p = ChainPage::new();
        p.entered(View::Chain);
        p.next_bank(View::Chain);
        p.next_bank(View::Chain);
        p.press(7, Some(2), 300);
        assert_eq!(p.held(0.3), None);
        assert_eq!(p.held(0.3), Some(1));
        assert_eq!(p.held(1.0), None, "once per press");
        // A finger that wobbles a little still holds...
        p.press(8, Some(0), 300);
        p.moved(8, 303);
        assert_eq!(p.held(1.0), Some(1));
        // ...but a drag that turns the card is not a long press.
        p.press(8, Some(0), 300);
        p.moved(8, 300 - DRAG_DETENT);
        assert_eq!(p.held(1.0), None);
        // The page's own knobs open nothing.
        p.next_bank(View::Chain);
        p.press(9, Some(0), 300);
        assert_eq!(p.held(1.0), None);
    }
}
