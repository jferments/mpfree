//! The Synth tab's options (MENU on the Synth tab): which engine the pads
//! play, and how the pads are laid out from a scale (owner, 2026-11-11;
//! `docs/project/scales.md`, "The pads").
//!
//! - **Engine**: Poly or FM. The other engine's notes finish their release.
//! - **Scale** and **Scale root**: the pads' own scale, Off until one is
//!   picked (the arp has its own). With no scale every layout is plain
//!   Chromatic, as in round 6.
//! - **Pad layout**: the bank playing's layout, Chromatic (the scale lit),
//!   In key or Chord pads. Banks A to H start as `DEFAULT_BANKS`.
//! - **In key rows**: in sequence, or a fourth apart.
//!
//! The app keeps these and hands them to the synth as
//! [`Effect::SynthPads`]; the audio loop applies them. The quick switch
//! between Chromatic and In key ([`App::synth_toggle_layout`]) and the PAD
//! BANK buttons picking a bank's layout ([`App::synth_bank`]) are for the
//! layout owner to place.

use super::{App, Effect, Item};
use crate::synth::{Engine, BANKS};
use mpfree_arp::scale::pads::{InKeyRows, PadLayout};
use mpfree_arp::scale::{Scale, NOTE_NAMES, SCALES};

/// The Synth tab's rows in its options menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SynthItem {
    Engine,
    Scale,
    Root,
    Layout,
    Rows,
}

/// The options menu opened from the Synth tab: the synth's own rows, then
/// the Play rows as before.
pub(super) const SYNTH_OPTIONS: [Item; 9] = [Item::Synth(SynthItem::Engine), Item::Synth(SynthItem::Scale),
                                             Item::Synth(SynthItem::Root), Item::Synth(SynthItem::Layout),
                                             Item::Synth(SynthItem::Rows), Item::Rate, Item::Tempo, Item::PadLevel,
                                             Item::Latch];

fn step_in<T: Copy + PartialEq>(all: &[T], now: T, d: i32) -> T {
    let at = all.iter().position(|&x| x == now).unwrap_or(0) as i32;
    all[(at + d).clamp(0, all.len() as i32 - 1) as usize]
}

impl App {
    /// The Scale row's choices: 0 is Off, then the built-in scales but
    /// Chromatic (which lays the pads out as Off does), then the owner's.
    fn pad_scale_choices(&self) -> usize {
        SCALES.len() + self.custom_scales.len()
    }

    /// Choice `i`'s name and notes (up, down); None for Off.
    fn pad_scale_entry(&self, i: usize) -> Option<(&str, u16, u16)> {
        match i {
            0 => None,
            i if i < SCALES.len() => Some((SCALES[i].name, SCALES[i].mask, SCALES[i].down)),
            i => self.custom_scales.get(i - SCALES.len()).map(|(n, u, d)| (n.as_str(), *u, *d)),
        }
    }

    fn pad_scale_choice(&self) -> usize {
        if self.synth_pads.scale.is_some() { self.pad_scale_pick } else { 0 }
    }

    pub(super) fn synth_item_text(&self, item: SynthItem) -> (&'static str, String) {
        let p = &self.synth_pads;
        match item {
            SynthItem::Engine => ("Engine", p.engine.name().to_string()),
            SynthItem::Scale => ("Scale", match self.pad_scale_entry(self.pad_scale_choice()) {
                None => "Off".to_string(),
                Some((name, ..)) => name.to_string(),
            }),
            SynthItem::Root => ("Scale root", NOTE_NAMES[usize::from(self.pad_scale_root % 12)].to_string()),
            SynthItem::Layout => ("Pad layout", if p.scale.is_some() {
                format!("{} (bank {})", p.layout().label(), super::bank_letter(p.bank))
            } else {
                "Chromatic (no scale)".to_string()
            }),
            SynthItem::Rows => ("In key rows", p.rows.label().to_string()),
        }
    }

    pub(super) fn synth_item_help(item: SynthItem) -> Vec<&'static str> {
        match item {
            SynthItem::Engine => vec!["The synth the pads play: Poly (subtractive)",
                                      "or FM (six operators). A note still ringing",
                                      "on the other one finishes its release. The",
                                      "Synth tab shows the engine picked."],
            SynthItem::Scale => vec!["The pads' own scale (the arp has its own).",
                                     "Off: pad 1 is C3 and each pad a semitone up.",
                                     "With a scale, Pad layout picks how the pads",
                                     "use it. Scales from your patterns file come",
                                     "after the built-in ones."],
            SynthItem::Root => vec!["The scale's first note. In key and Chord pads",
                                    "start on it, in octave 3."],
            SynthItem::Layout => vec!["Chromatic: a semitone a pad, the scale lit",
                                      "(root, in the scale, outside it). In key:",
                                      "only scale notes. Chord pads: triads on the",
                                      "bottom rows, sevenths on the top rows. Each",
                                      "pad bank keeps its own layout: A Chromatic,",
                                      "B In key, C Chord pads, D In key an octave up."],
            SynthItem::Rows => vec!["In key: the pads climb the scale in sequence,",
                                    "or each row starts a fourth (three scale",
                                    "notes) above the row below, so a chord keeps",
                                    "one shape."],
        }
    }

    /// One of the synth's option rows stepped by `d` (-1 or +1).
    pub(super) fn step_synth_item(&mut self, item: SynthItem, d: i32) -> Vec<Effect> {
        let before = (self.synth_pads, self.pad_scale_pick, self.pad_scale_root);
        match item {
            SynthItem::Engine => self.synth_pads.engine = step_in(&Engine::ALL, self.synth_pads.engine, d),
            SynthItem::Scale => {
                self.pad_scale_pick = (self.pad_scale_choice() as i32 + d).clamp(0, self.pad_scale_choices() as i32 - 1)
                    as usize;
                self.synth_pads.scale = self.pad_scale_entry(self.pad_scale_pick)
                    .map(|(_, up, down)| Scale::with_down(self.pad_scale_root, up, down));
            }
            SynthItem::Root => {
                self.pad_scale_root = (i32::from(self.pad_scale_root) + d).clamp(0, 11) as u8;
                let root = self.pad_scale_root;
                self.synth_pads.scale = self.synth_pads.scale.map(|s| Scale::with_down(root, s.mask, s.down));
            }
            SynthItem::Layout => {
                let l = step_in(&PadLayout::ALL, self.synth_pads.layout(), d);
                self.synth_pads.set_layout(l);
            }
            SynthItem::Rows => self.synth_pads.rows = step_in(&InKeyRows::ALL, self.synth_pads.rows, d),
        }
        if (self.synth_pads, self.pad_scale_pick, self.pad_scale_root) == before {
            return Vec::new();
        }
        vec![Effect::SynthPads(self.synth_pads)]
    }

    /// The quick switch: the bank playing goes Chromatic to In key, and any
    /// other layout back to Chromatic. Nothing changes with no scale. For
    /// the layout owner to put on a button or a touch (round 8).
    #[allow(dead_code)]
    pub fn synth_toggle_layout(&mut self) -> Vec<Effect> {
        if self.synth_pads.scale.is_none() {
            return vec![Effect::Note("pads: pick a scale first (MENU on the Synth tab)".to_string())];
        }
        let l = self.synth_pads.layout().toggled();
        self.synth_pads.set_layout(l);
        vec![Effect::SynthPads(self.synth_pads)]
    }

    /// The pads take bank `bank`'s layout (0 = A to 7 = H). For the PAD
    /// BANK buttons while the synth has the pads (round 8, layout owner).
    #[allow(dead_code)]
    pub fn synth_bank(&mut self, bank: usize) -> Vec<Effect> {
        let bank = bank % BANKS;
        if bank == self.synth_pads.bank {
            return Vec::new();
        }
        self.synth_pads.bank = bank;
        vec![Effect::SynthPads(self.synth_pads)]
    }
}
