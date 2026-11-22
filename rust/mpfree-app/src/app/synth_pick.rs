//! The list of synths on the Synth tab, picking for the pads (the
//! instrument redesign's step 6: "The list from PR #715's history, with
//! its Cancel fix, picks for the pads"; Jay's device notes of 20 Nov 2026:
//! "assign a synth ... in just 1 or 2 actions, or ... preview it on the
//! pads").
//!
//! - **The list**: a tap on the instrument card opens it over the page:
//!   the track's instrument first (the pads follow the track again), then
//!   the built-in engines, each with what it is good for, then the plugin
//!   synths ([`App::open_synth_pick`]).
//! - **A tap plays it on the pads at once**, whichever track is selected,
//!   so the musician can try several in a row ([`App::pick_pads_synth`]).
//!   No track changes (rule 1): the synth goes into a free place
//!   (`instruments::FREE`), an instrument no track has, on a mixer strip
//!   of its own ("FM · Pads"), and the pads play it ([`PadFocus`]). A
//!   second tap puts the next synth into the same place while nothing
//!   else plays it; the one the pads had when the list opened is kept for
//!   Cancel.
//! - **Keep** leaves the pads on it; **Cancel** gives them back what they
//!   played when the list opened ([`App::close_synth_pick`]). Leaving the
//!   tab or a second tap on the card keeps it. A synth tried and left goes
//!   away once nothing plays it ([`App::put_away_free`]); REC records it
//!   on a free track, which then plays it (desk.rs, the recording plan).
//! - **All banks or one** (step 6c; the recommended answer on Jay's card
//!   of 20 Nov, "the list gets All banks or Bank B only at its top"): on
//!   a kit's pages the list's top offers **All banks** and **Bank B only**
//!   (the bank the pads are on). With the second, a tap gives that bank
//!   the synth and the other banks play on as they did; its first choice
//!   gives the bank back to the track. With the first, a tap plays on
//!   every bank, a bank's own given up ([`App::synth_pick_scope`]).

use super::{App, Effect, PadFocus, View};
use crate::instruments::{free, Held, FREE, SLOTS};
use crate::synth::Engine;

/// The list as the screen draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SynthPickView {
    /// The track whose instrument the first choice gives the pads back
    /// (0 based): the selected one.
    pub track: usize,
    /// The track's instrument first, then the engines, then the plugin
    /// synths: each its name and a line on what it is good for.
    pub choices: Vec<(String, String)>,
    /// The choice the pads play now, None for one the list doesn't have
    /// (the shared synth, chosen with Play on the pads).
    pub now: Option<usize>,
    /// The choice they played when the list opened (Cancel's).
    pub had: Option<usize>,
    /// On a kit's pages (step 6c): the bank the pads are on ('B') and
    /// whether a tap gives only it the synth ("Bank B only"), else every
    /// bank ("All banks"). None while PAD BANK picks the shared synth's
    /// layouts.
    pub bank: Option<(char, bool)>,
}

/// The list while it is open: what the pads played when it opened, for
/// Cancel, and the free place they played then, kept until it closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SynthPick {
    had: PadFocus,
    /// What each bank played of its own then (step 6c).
    had_banks: [Option<PadFocus>; super::KIT_PAGES],
    /// The free places that held an instrument then, a bit each: a tap
    /// never puts a synth in one, so Cancel finds them as they were.
    kept: u16,
    /// The list's choice they played then.
    had_choice: Option<usize>,
    /// "Bank B only" (step 6c): a tap gives the bank the pads are on the
    /// synth, not every bank.
    bank_only: bool,
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
    /// The list's synths after its first choice: the built-in engines,
    /// then the plugin synths, as (node id, name).
    fn pick_choices(&self) -> Vec<(&'static str, &'static str)> {
        Engine::ALL
            .iter()
            .map(|e| (e.node_id(), e.name()))
            .chain(self.plugin_synths.iter().copied())
            .collect()
    }

    /// The holder of what `f` plays, when the host holds it.
    fn focus_holder(&self, f: Option<PadFocus>) -> Option<usize> {
        match f? {
            PadFocus::Instrument(id) => self.instrument_list.kind(id)?.holder(),
            PadFocus::Track => None,
        }
    }

    /// The free place of holder `h`.
    fn place_of(h: Option<usize>) -> Option<usize> {
        h?.checked_sub(crate::instruments::TRACKS)
            .filter(|&n| n < FREE)
    }

    /// The free place a synth tapped in the list would play from, for
    /// every bank (`bank` None) or bank `bank` alone (step 6c): the place
    /// that choice plays now while nothing else plays it (no track, no
    /// other bank, nor every bank's choice for one bank), else an empty
    /// one. Never one that held an instrument when the list opened (kept
    /// for Cancel). None when every place is taken or [`SLOTS`]
    /// instruments are loaded.
    fn pick_place(&self, bank: Option<usize>) -> Option<usize> {
        let kept = self.synth_pick.map_or(0, |p| p.kept);
        let sent = |h: usize| (0..mpfree_seq::MAX_TRACKS).any(|t| self.track_send(t) == Some(h));
        let all = Self::place_of(self.focus_holder(Some(self.pads_focus)));
        let of_bank =
            |b: usize| self.bank_to[b].or(Self::place_of(self.focus_holder(self.bank_focus[b])));
        let (now, busy) = match bank {
            None => (
                self.pads_to.or(all),
                (0..super::KIT_PAGES)
                    .filter_map(of_bank)
                    .fold(0u16, |m, n| m | 1 << n),
            ),
            Some(b) => (
                of_bank(b),
                (0..super::KIT_PAGES)
                    .filter(|&o| o != b)
                    .filter_map(of_bank)
                    .chain(self.pads_to)
                    .chain(all)
                    .fold(0u16, |m, n| m | 1 << n),
            ),
        };
        let open = |n: usize| (kept | busy) >> n & 1 == 0;
        if let Some(n) = now.filter(|&n| open(n) && !sent(free(n))) {
            return Some(n);
        }
        let loaded = self.track_sounds.iter().flatten().count();
        if loaded >= SLOTS {
            return None;
        }
        (0..FREE).find(|&n| {
            self.track_sound(free(n)).is_none()
                && self.pads_to != Some(n)
                && !self.bank_to.contains(&Some(n))
                && open(n)
        })
    }

    /// Whether the open list keeps free place `n` for Cancel.
    pub(super) fn keeps_free(&self, n: usize) -> bool {
        self.synth_pick
            .is_some_and(|p| n < 16 && p.kept >> n & 1 == 1)
    }

    /// The bank a tap in the list gives the synth alone (step 6c): the one
    /// the pads are on, with "Bank B only" on a kit's pages.
    fn pick_bank(&self) -> Option<usize> {
        let only = self.synth_pick.is_some_and(|p| p.bank_only);
        (only && self.banks_apply()).then_some(self.pad_page)
    }

    /// "All banks" (`only` false) or "Bank B only" at the top of the open
    /// list (step 6c): where the next tap puts its synth.
    pub fn synth_pick_scope(&mut self, only: bool) -> Vec<Effect> {
        let Some(p) = self.synth_pick.as_mut() else {
            return Vec::new();
        };
        if p.bank_only == only {
            return Vec::new();
        }
        p.bank_only = only;
        vec![Effect::SynthPick(self.synth_pick_view())]
    }

    /// The list as the screen draws it, while it is open.
    pub(super) fn synth_pick_view(&self) -> Option<SynthPickView> {
        let pick = self.synth_pick?;
        let synths = self.pick_choices();
        let track = self.track;
        let bank = self.pick_bank();
        let first = (
            format!("Track {}'s", track + 1),
            match bank {
                Some(b) => format!("Bank {} follows the track", super::bank_letter(b)),
                None => "The pads follow the track again".to_string(),
            },
        );
        let choices = std::iter::once(first)
            .chain(synths.iter().map(|&(id, name)| {
                (
                    name.to_string(),
                    Engine::from_node_id(id)
                        .map_or("Plugin", good_for)
                        .to_string(),
                )
            }))
            .collect();
        // What the pads play now, on the bank they are on: a place waiting
        // for its id first.
        let page = self.pad_page;
        let own =
            self.banks_apply() && (self.bank_to[page].is_some() || self.bank_focus[page].is_some());
        let waiting = if own {
            self.bank_to[page]
        } else {
            self.pads_to
        };
        let playing = waiting
            .and_then(|n| self.track_sound(free(n)))
            .or_else(|| self.pads_held());
        let now = match (self.pads_focus(), playing) {
            (_, Some(h)) => synths.iter().position(|c| c.0 == h.id).map(|i| i + 1),
            (PadFocus::Track, None) => Some(0),
            (PadFocus::Instrument(_), None) => None,
        };
        Some(SynthPickView {
            track,
            choices,
            now,
            had: pick.had_choice,
            bank: self
                .banks_apply()
                .then(|| (super::bank_letter(page), pick.bank_only)),
        })
    }

    /// A tap on the instrument card: the list of synths opens over the
    /// Synth tab, or, open, closes and keeps what the pads play.
    pub fn open_synth_pick(&mut self) -> Vec<Effect> {
        if self.view != View::Synth {
            return Vec::new();
        }
        if self.synth_pick.is_some() {
            return self.close_synth_pick(true);
        }
        let kept = (0..FREE)
            .filter(|&n| self.track_sound(free(n)).is_some())
            .fold(0u16, |m, n| m | 1 << n);
        self.synth_pick = Some(SynthPick {
            had: self.pads_focus,
            had_banks: self.bank_focus,
            kept,
            had_choice: None,
            // On a bank with its own, the next tap changes that bank.
            bank_only: self.pads_bank().is_some(),
        });
        let view = self.synth_pick_view();
        if let (Some(p), Some(v)) = (self.synth_pick.as_mut(), view.as_ref()) {
            p.had_choice = v.now;
        }
        vec![Effect::SynthPick(self.synth_pick_view())]
    }

    /// A tap on choice `i` of the open list: the pads play it now (the
    /// first: the track's instrument again).
    pub fn pick_pads_synth(&mut self, i: usize) -> Vec<Effect> {
        if self.synth_pick.is_none() {
            return Vec::new();
        }
        let bank = self.pick_bank();
        let mut fx = Vec::new();
        match (i, bank) {
            (0, None) => fx.extend(self.pads_follow_track()),
            (0, Some(b)) => fx.extend(self.bank_follows_track(b)),
            _ => {
                if let Some(&(id, name)) = self.pick_choices().get(i - 1) {
                    fx.extend(self.pads_synth(id, name, bank));
                }
            }
        }
        fx.push(Effect::SynthPick(self.synth_pick_view()));
        fx
    }

    /// The list's first choice with "Bank B only" (step 6c): bank `b`
    /// plays the selected track's instrument, whatever the other banks
    /// play. Nothing (and no allocation) when it does already.
    fn bank_follows_track(&mut self, b: usize) -> Vec<Effect> {
        // None is "what every bank plays": the track's, while they follow it.
        let to = (self.pads_focus != PadFocus::Track).then_some(PadFocus::Track);
        if self.bank_focus[b] == to && self.bank_to[b].is_none() {
            return Vec::new();
        }
        self.bank_focus[b] = to;
        self.bank_to[b] = None;
        self.project_edit = true;
        vec![
            Effect::PadsFocus(self.pads_focus()),
            Effect::Note(format!(
                "Bank {} follows the track: Track {}'s instrument",
                super::bank_letter(b),
                self.track + 1
            )),
        ]
    }

    /// The pads play synth `id` (`name`) from a free place, on every bank
    /// (`bank` None, a bank's own given up) or on bank `bank` alone (step
    /// 6c), said; or why not.
    fn pads_synth(
        &mut self,
        id: &'static str,
        name: &'static str,
        bank: Option<usize>,
    ) -> Vec<Effect> {
        if self.generating() {
            return vec![Effect::Tell(
                "The pads play a synth picked here as pressed: let go of NOTE REPEAT or Latch, or close the Arp panel, first"
                    .into(),
            )];
        }
        let Some(n) = self.pick_place(bank) else {
            return vec![Effect::Tell(format!(
                "No room for {name}: {SLOTS} instruments are loaded, the most at once. Set a track to Kit first"
            ))];
        };
        let holder = free(n);
        // Where the choice points now: every bank's, or the bank's own.
        let points = match bank {
            None => {
                self.pads_to == Some(n) || self.focus_holder(Some(self.pads_focus)) == Some(holder)
            }
            Some(b) => {
                self.bank_to[b] == Some(n) || self.focus_holder(self.bank_focus[b]) == Some(holder)
            }
        };
        let banks_own =
            self.bank_focus.iter().any(Option::is_some) || self.bank_to.iter().any(Option::is_some);
        if self.track_sound(holder).map(|h| h.id) == Some(id)
            && points
            && (bank.is_some() || !banks_own)
        {
            return Vec::new();
        }
        if self.track_sound(holder).map(|h| h.id) != Some(id) {
            self.set_track_sound(
                holder,
                Some(Held {
                    id,
                    name,
                    instance: 0,
                }),
            );
        }
        match bank {
            None => {
                if !points {
                    self.pads_to = Some(n);
                }
                // Every bank: a bank's own is given up.
                self.bank_focus = [None; super::KIT_PAGES];
                self.bank_to = [None; super::KIT_PAGES];
            }
            Some(b) if !points => self.bank_to[b] = Some(n),
            Some(_) => {}
        }
        self.project_edit = true;
        let said = match bank {
            None => format!(
                "The pads play {name} on every track · Track {} keeps what it plays · REC records it on a free track",
                self.track + 1
            ),
            Some(b) => format!(
                "Bank {} plays {name} · the other banks play as they did · REC records it on a free track",
                super::bank_letter(b)
            ),
        };
        vec![
            Effect::TrackSound {
                track: holder,
                id: Some(id),
            },
            Effect::Note(said),
        ]
    }

    /// Keep (or the page left, or the card again): the list closes and the
    /// pads keep what they play. Cancel: they play what they played when it
    /// opened again (the synths tried go away, as nothing plays them).
    pub fn close_synth_pick(&mut self, keep: bool) -> Vec<Effect> {
        let Some(pick) = self.synth_pick.take() else {
            return Vec::new();
        };
        let mut fx = Vec::new();
        let listed = |f: PadFocus| match f {
            PadFocus::Instrument(id) => self.instrument_list.kind(id).is_some(),
            PadFocus::Track => true,
        };
        let back = listed(pick.had);
        let banks = pick.had_banks.map(|f| f.filter(|&f| listed(f)));
        let moved = self.pads_focus != pick.had
            || self.pads_to.is_some()
            || self.bank_focus != banks
            || self.bank_to.iter().any(Option::is_some);
        if !keep && moved && back {
            self.pads_focus = pick.had;
            self.pads_to = None;
            self.bank_focus = banks;
            self.bank_to = [None; super::KIT_PAGES];
            self.project_edit = true;
            fx.push(Effect::PadsFocus(self.pads_focus()));
        }
        fx.push(Effect::SynthPick(None));
        fx
    }
}
