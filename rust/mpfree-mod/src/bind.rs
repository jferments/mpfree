//! Q-Link (and touch slider) bindings: which parameter or macro each of
//! the four knobs moves, in banks of four.
//!
//! Pure control-thread logic with no I/O, for the app's Q-Link column:
//! [`Bindings::turn`] turns a knob into the value to set, and
//! [`Bindings::card`] says what the knob's card shows. The card has the
//! same fields as the mixer's Q-Link cards (`mpfree_mixer_ui::qlinks`),
//! so every page's Q-Links draw the same way.
//!
//! **Pages and banks.** The page on screen sets its key and its own four
//! knobs with [`Bindings::set_page`] (the chain editor: the selected
//! effect's first four knobs). They show wherever the bank shown leaves a
//! knob free, so a user binding overrides them. [`Bindings::with_macros`] makes the usual
//! banks: bank 0 has no bindings of its own (the page's knobs show), banks
//! 1 and 2 are Macros 1-4 and 5-8; [`Bindings::next_bank`] steps through
//! them. A page that sets no knobs leaves its free knobs free, and the
//! app draws its own cards there ([`QLinkCard::target`] is `None`).
//!
//! **Learn:** [`Bindings::learn`] arms a parameter (the user touched it
//! on screen); the next knob turned takes it, in the bank shown. By
//! default ([`Scope::ThisPage`]) the knob belongs to the page it was
//! learned on, so a Q-Link learned to a reverb's mix in the chain editor
//! leaves Play's and Synth's knobs alone; [`Scope::EveryPage`] makes it
//! global, as the macro banks are.
//!
//! Bindings save as `key = value` lines ([`Bindings::save`]) for
//! `mpfree-store`'s "mod" settings.

use std::collections::{BTreeMap, HashMap};

use mpfree_node::{Curve, ParamInfo};
use mpfree_param::ParamAddr;

use crate::text::value_text;

/// Q-Links on the MPC One.
pub const KNOBS: usize = 4;
/// Knob detents for the whole range of a binding (fine: ten times more).
pub const DETENTS: f32 = 100.0;

/// One knob's binding.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub target: ParamAddr,
    pub info: ParamInfo,
    /// The card's name for it ("Wash", "Cutoff").
    pub label: String,
    /// A theme colour slot (`Theme::track(i)`), so a macro's knob wears
    /// its macro's colour; `None` for the page's accent.
    pub colour: Option<u8>,
    /// The part of the parameter's travel the knob covers, as knob
    /// positions (0 to 1). `from` above `to` turns it the other way.
    pub from: f32,
    pub to: f32,
}

impl Binding {
    /// A binding over the parameter's whole range.
    pub fn new(target: ParamAddr, info: ParamInfo, label: &str) -> Binding {
        Binding {
            target,
            info,
            label: label.to_string(),
            colour: None,
            from: 0.0,
            to: 1.0,
        }
    }
}

/// A bank: four knobs and a name ("Macros 1-4", "Drums FX").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bank {
    pub name: String,
    pub knobs: [Option<Binding>; KNOBS],
}

/// A value to set: send it as `ModEvent::Set` to the engine that looks
/// after `addr` (or straight to its owner when no engine does).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Write {
    pub addr: ParamAddr,
    pub value: f32,
}

/// What a knob's card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct QLinkCard {
    pub label: String,
    /// The value in its units ("2.4 kHz"); empty when free.
    pub text: String,
    /// "learn" while a parameter waits for this knob, else empty.
    pub sub: &'static str,
    pub value: f32,
    /// How far round the arc is lit, 0 to 1, within the binding's range.
    pub pos: f32,
    pub colour: Option<u8>,
    /// The parameter it moves; `None` when the knob is free (draw the
    /// page's own card).
    pub target: Option<ParamAddr>,
}

impl QLinkCard {
    pub fn enabled(&self) -> bool {
        self.target.is_some()
    }
}

/// Where a learned knob applies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// Only on the page it was learned on (the default).
    #[default]
    ThisPage,
    /// On every page, as the macro banks are.
    EveryPage,
}

/// The banks, which is shown, and the last value of every bound
/// parameter.
#[derive(Clone, Debug, Default)]
pub struct Bindings {
    banks: Vec<Bank>,
    shown: usize,
    values: HashMap<ParamAddr, f32>,
    learning: Option<(ParamAddr, ParamInfo, String, Scope)>,
    /// Knobs that belong to one page: (page key, bank, knob).
    scoped: BTreeMap<(String, usize, usize), Binding>,
    /// The page on screen: its key, its own knobs and its name (the knobs
    /// and name are not saved).
    page_key: String,
    page: [Option<Binding>; KNOBS],
    page_name: String,
    /// The knob being touched, for lighting its card.
    pub touched: Option<usize>,
}

impl Bindings {
    pub fn new() -> Bindings {
        Bindings::default()
    }

    /// The usual banks: bank 0 with no bindings of its own (the page's
    /// knobs show), bank 1 "Macros 1-4" and bank 2 "Macros 5-8", each knob
    /// in its macro's colour. `names` are the macros' names.
    pub fn with_macros(names: &[String]) -> Bindings {
        let mut b = Bindings::new();
        b.show(0);
        for bank in 1..=2 {
            let first = (bank - 1) * KNOBS;
            b.bank_mut(bank).name = format!("Macros {}-{}", first + 1, first + KNOBS);
            for k in 0..KNOBS {
                let m = first + k;
                let name = names
                    .get(m)
                    .cloned()
                    .unwrap_or_else(|| format!("Macro {}", m + 1));
                let addr = ParamAddr::macro_value(m as u8);
                b.bind(
                    bank,
                    k,
                    Binding {
                        colour: Some(m as u8),
                        ..Binding::new(addr, crate::engine::macro_info(m), &name)
                    },
                );
            }
        }
        b.bank_mut(0).name = String::new();
        b
    }

    /// The page on screen: its key (a short stable word: "play",
    /// "chain"; it names the page's learned knobs in the saved lines), its
    /// name for the column's title ("Drive"), and its own knobs (`None` for
    /// a knob it leaves free). They show wherever the bank shown has no
    /// binding of its own. The name and knobs are not saved; set them
    /// again when the page or what it shows changes.
    pub fn set_page(&mut self, key: &str, name: &str, knobs: [Option<Binding>; KNOBS]) {
        for b in knobs.iter().flatten() {
            self.values.entry(b.target).or_insert(b.info.default);
        }
        self.page_key = page_key(key);
        self.page = knobs;
        self.page_name = name.to_string();
    }

    /// The page on screen's key.
    pub fn page_key(&self) -> &str {
        &self.page_key
    }

    /// Binds knob `k` of bank `b` on page `page` only.
    pub fn bind_page(&mut self, page: &str, b: usize, k: usize, binding: Binding) {
        if k >= KNOBS {
            return;
        }
        self.show_without_switching(b);
        self.values
            .entry(binding.target)
            .or_insert(binding.info.default);
        self.scoped.insert((page_key(page), b, k), binding);
    }

    /// The page's own knobs.
    pub fn page(&self) -> &[Option<Binding>; KNOBS] {
        &self.page
    }

    /// What the column's title says: the bank's name, or the page's when
    /// the bank has none ("Drive", "Macros 1-4").
    pub fn title(&self) -> &str {
        match self.banks.get(self.shown) {
            Some(b) if !b.name.is_empty() => &b.name,
            _ => &self.page_name,
        }
    }

    /// Shows the next bank (after the last, the first).
    pub fn next_bank(&mut self) {
        let n = self.banks.len().max(1);
        self.show((self.shown + 1) % n);
    }

    pub fn banks(&self) -> &[Bank] {
        &self.banks
    }
    pub fn shown(&self) -> usize {
        self.shown
    }

    /// Shows bank `b` (made empty if it is past the end).
    pub fn show(&mut self, b: usize) {
        while self.banks.len() <= b {
            let n = self.banks.len();
            self.banks.push(Bank {
                name: format!("Bank {}", n + 1),
                knobs: Default::default(),
            });
        }
        self.shown = b;
    }

    pub fn bank_mut(&mut self, b: usize) -> &mut Bank {
        self.show_without_switching(b);
        &mut self.banks[b]
    }

    fn show_without_switching(&mut self, b: usize) {
        let s = self.shown;
        self.show(b);
        self.shown = s;
    }

    /// Binds knob `k` of bank `b`.
    pub fn bind(&mut self, b: usize, k: usize, binding: Binding) {
        if k >= KNOBS {
            return;
        }
        let v = self
            .values
            .get(&binding.target)
            .copied()
            .unwrap_or(binding.info.default);
        self.values.insert(binding.target, v);
        self.bank_mut(b).knobs[k] = Some(binding);
    }

    /// Unbinds knob `k` of bank `b`: the page on screen's own binding of
    /// it if it has one, else the binding for every page.
    pub fn unbind(&mut self, b: usize, k: usize) -> Option<Binding> {
        if let Some(x) = self.scoped.remove(&(self.page_key.clone(), b, k)) {
            return Some(x);
        }
        self.banks.get_mut(b)?.knobs.get_mut(k)?.take()
    }

    /// Knob `k`'s binding: the bank shown's for this page, else its
    /// binding for every page, else the page's own knob.
    pub fn binding(&self, k: usize) -> Option<&Binding> {
        self.scoped
            .get(&(self.page_key.clone(), self.shown, k))
            .or_else(|| {
                self.banks
                    .get(self.shown)
                    .and_then(|b| b.knobs.get(k)?.as_ref())
            })
            .or_else(|| self.page.get(k)?.as_ref())
    }

    /// Tells the bindings a parameter's value (read back from the engine
    /// or its owner), so cards and turns start from it.
    pub fn observe(&mut self, addr: ParamAddr, value: f32) {
        if let Some(v) = self.values.get_mut(&addr) {
            *v = value;
        }
    }

    pub fn value(&self, addr: ParamAddr) -> Option<f32> {
        self.values.get(&addr).copied()
    }

    /// Arms learn: the next knob turned takes `addr`, on this page only.
    pub fn learn(&mut self, addr: ParamAddr, info: ParamInfo, label: &str) {
        self.learn_for(addr, info, label, Scope::ThisPage);
    }

    /// Arms learn with a choice of where the knob applies.
    pub fn learn_for(&mut self, addr: ParamAddr, info: ParamInfo, label: &str, scope: Scope) {
        self.learning = Some((addr, info, label.to_string(), scope));
    }
    pub fn cancel_learn(&mut self) {
        self.learning = None;
    }
    pub fn learning(&self) -> Option<ParamAddr> {
        self.learning.as_ref().map(|l| l.0)
    }

    /// Knob `k` turned by `detents` (clockwise positive); `fine` while
    /// SHIFT is held. The value to set, or `None` when nothing changes
    /// (a free knob, the end of the range, or a learn that just bound
    /// the knob). A learn binds the knob in the bank shown, over the
    /// page's knob, on this page (or every page, as armed) until it is
    /// unbound.
    pub fn turn(&mut self, k: usize, detents: i32, fine: bool) -> Option<Write> {
        if k >= KNOBS {
            return None;
        }
        if let Some((addr, info, label, scope)) = self.learning.take() {
            let b = self.shown;
            let binding = Binding::new(addr, info, &label);
            match scope {
                Scope::ThisPage => {
                    let page = self.page_key.clone();
                    self.bind_page(&page, b, k, binding);
                }
                Scope::EveryPage => self.bind(b, k, binding),
            }
            return None;
        }
        if detents == 0 {
            return None;
        }
        let bnd = self.binding(k)?.clone();
        let info = bnd.info;
        let now = info.clamp(
            self.values
                .get(&bnd.target)
                .copied()
                .unwrap_or(info.default),
        );
        let value = match info.curve {
            Curve::Toggle => {
                if detents > 0 {
                    1.0
                } else {
                    0.0
                }
            }
            // One value per detent, whatever the speed.
            Curve::Stepped | Curve::Choice(_) => {
                let dir = if (bnd.to >= bnd.from) == (detents > 0) {
                    1.0
                } else {
                    -1.0
                };
                let (lo, hi) = range_values(&bnd);
                (now + dir * detents.unsigned_abs() as f32).clamp(lo, hi)
            }
            _ => {
                let span = bnd.to - bnd.from;
                if span.abs() < 1e-6 {
                    return None;
                }
                let x = ((info.to_unit(now) - bnd.from) / span).clamp(0.0, 1.0);
                let step = if fine {
                    1.0 / (DETENTS * 10.0)
                } else {
                    1.0 / DETENTS
                };
                let x = (x + detents as f32 * step).clamp(0.0, 1.0);
                info.from_unit(bnd.from + span * x)
            }
        };
        let value = info.clamp(value);
        if value == now {
            return None;
        }
        self.values.insert(bnd.target, value);
        Some(Write {
            addr: bnd.target,
            value,
        })
    }

    /// Knob `k` put at `pos` (0 to 1): a touch slider, or a knob read as a
    /// position.
    pub fn to(&mut self, k: usize, pos: f32) -> Option<Write> {
        let bnd = self.binding(k)?.clone();
        let x = pos.clamp(0.0, 1.0);
        let value = bnd.info.from_unit(bnd.from + (bnd.to - bnd.from) * x);
        let now = self.values.get(&bnd.target).copied();
        if now == Some(value) {
            return None;
        }
        self.values.insert(bnd.target, value);
        Some(Write {
            addr: bnd.target,
            value,
        })
    }

    /// What knob `k`'s card shows.
    pub fn card(&self, k: usize) -> QLinkCard {
        let learn = if self.learning.is_some() { "learn" } else { "" };
        let Some(b) = self.binding(k) else {
            return QLinkCard {
                label: String::new(),
                text: String::new(),
                sub: learn,
                value: 0.0,
                pos: 0.0,
                colour: None,
                target: None,
            };
        };
        let v = b.info.clamp(
            self.values
                .get(&b.target)
                .copied()
                .unwrap_or(b.info.default),
        );
        let span = b.to - b.from;
        let pos = if span.abs() < 1e-6 {
            0.0
        } else {
            ((b.info.to_unit(v) - b.from) / span).clamp(0.0, 1.0)
        };
        QLinkCard {
            label: b.label.clone(),
            text: value_text(&b.info, v),
            sub: learn,
            value: v,
            pos,
            colour: b.colour,
            target: Some(b.target),
        }
    }

    pub fn cards(&self) -> [QLinkCard; KNOBS] {
        std::array::from_fn(|k| self.card(k))
    }

    /// The bindings as `key = value` lines: `bank.0 = Name`,
    /// `knob.0.1 = macro2.0 0.000 1.000 3 Wash` (target, from, to, colour
    /// or `-`, label) for a knob on every page, and `page.chain.0.1 = ...`
    /// for a knob on one page.
    pub fn save(&self) -> String {
        let line = |x: &Binding| {
            let c = x.colour.map_or("-".to_string(), |c| c.to_string());
            format!(
                "{} {:.4} {:.4} {c} {}",
                x.target,
                x.from,
                x.to,
                one_line(&x.label)
            )
        };
        let mut s = String::new();
        for (b, bank) in self.banks.iter().enumerate() {
            s += &format!("bank.{b} = {}\n", one_line(&bank.name));
            for (k, bnd) in bank.knobs.iter().enumerate() {
                let Some(x) = bnd else { continue };
                s += &format!("knob.{b}.{k} = {}\n", line(x));
            }
        }
        for ((page, b, k), x) in &self.scoped {
            s += &format!("page.{page}.{b}.{k} = {}\n", line(x));
        }
        s
    }

    /// Reads [`Bindings::save`]'s lines back. `describe` gives a target's
    /// description; a target it does not know (an effect since removed)
    /// is skipped, as is any line that does not read. Returns how many
    /// lines were skipped.
    pub fn load(
        text: &str,
        describe: &dyn Fn(ParamAddr) -> Option<ParamInfo>,
    ) -> (Bindings, usize) {
        let mut out = Bindings::new();
        let mut skipped = 0;
        for line in text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let ok = (|| {
                let (key, val) = line.split_once(" = ").or_else(|| line.split_once('='))?;
                let key: Vec<&str> = key.trim().split('.').collect();
                let val = val.trim();
                match key.as_slice() {
                    ["bank", b] => {
                        let b: usize = b.parse().ok().filter(|b| *b < 64)?;
                        out.bank_mut(b).name = val.to_string();
                    }
                    ["knob", b, k] => {
                        let (b, k) = (bank_no(b)?, knob_no(k)?);
                        out.bind(b, k, read_binding(val, describe)?);
                    }
                    ["page", page, b, k] if !page.is_empty() => {
                        let (b, k) = (bank_no(b)?, knob_no(k)?);
                        out.bind_page(page, b, k, read_binding(val, describe)?);
                    }
                    _ => return None,
                }
                Some(())
            })();
            if ok.is_none() {
                skipped += 1;
            }
        }
        (out, skipped)
    }
}

fn bank_no(s: &str) -> Option<usize> {
    s.parse().ok().filter(|b| *b < 64)
}
fn knob_no(s: &str) -> Option<usize> {
    s.parse().ok().filter(|k| *k < KNOBS)
}

/// One saved binding: `target from to colour label`.
fn read_binding(val: &str, describe: &dyn Fn(ParamAddr) -> Option<ParamInfo>) -> Option<Binding> {
    let mut w = val.splitn(5, ' ');
    let target: ParamAddr = w.next()?.parse().ok()?;
    let from: f32 = w.next()?.parse().ok()?;
    let to: f32 = w.next()?.parse().ok()?;
    let colour = match w.next()? {
        "-" => None,
        c => Some(c.parse().ok()?),
    };
    let label = w.next().unwrap_or("");
    let info = describe(target)?;
    if !(from.is_finite() && to.is_finite()) {
        return None;
    }
    Some(Binding {
        target,
        info,
        label: label.to_string(),
        colour,
        from: from.clamp(0.0, 1.0),
        to: to.clamp(0.0, 1.0),
    })
}

/// A page key as the saved lines can hold it: letters, digits, `-` and
/// `_` (anything else becomes `_`).
fn page_key(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The lowest and highest value a binding reaches.
fn range_values(b: &Binding) -> (f32, f32) {
    let (x, y) = (b.info.from_unit(b.from), b.info.from_unit(b.to));
    (x.min(y), x.max(y))
}

fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::Unit;
    use mpfree_param::{Owner, Strip};

    const CUT: ParamInfo = ParamInfo {
        id: 3,
        name: "Cutoff",
        group: "Filter",
        min: 20.0,
        max: 20000.0,
        default: 1000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    };
    const MODE: ParamInfo = ParamInfo {
        id: 4,
        name: "Mode",
        group: "Filter",
        min: 0.0,
        max: 3.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(&["LP", "HP", "BP", "Notch"]),
        readonly: false,
    };

    fn addr(p: u16) -> ParamAddr {
        ParamAddr::new(Owner::Insert(Strip::Track(0), 1), p)
    }

    #[test]
    fn turns_move_through_the_bound_range() {
        let mut b = Bindings::new();
        b.bind(
            0,
            0,
            Binding {
                from: 0.25,
                to: 0.75,
                ..Binding::new(addr(3), CUT, "Cutoff")
            },
        );
        b.observe(addr(3), CUT.from_unit(0.25));
        let w = b.turn(0, 50, false).unwrap();
        assert!(
            (CUT.to_unit(w.value) - 0.5).abs() < 1e-4,
            "half the detents, half the bound range"
        );
        assert!(b.turn(0, 500, false).is_some());
        assert!(
            (CUT.to_unit(b.value(addr(3)).unwrap()) - 0.75).abs() < 1e-4,
            "stops at the top of the range"
        );
        assert_eq!(b.turn(0, 1, false), None, "already at the top");
        let c = b.card(0);
        assert_eq!(
            (c.pos, c.label.as_str(), c.enabled()),
            (1.0, "Cutoff", true)
        );
        let fine = b.turn(0, -1, true).unwrap().value;
        let coarse = b.turn(0, -1, false).unwrap().value;
        assert!(fine > coarse);
        assert_eq!(b.turn(1, 1, false), None, "free knob");
        assert!(!b.card(1).enabled());
    }

    #[test]
    fn choices_step_one_per_detent() {
        let mut b = Bindings::new();
        b.bind(0, 2, Binding::new(addr(4), MODE, "Mode"));
        assert_eq!(b.turn(2, 1, false).unwrap().value, 1.0);
        assert_eq!(b.turn(2, 5, false).unwrap().value, 3.0);
        assert_eq!(b.card(2).text, "Notch");
        assert_eq!(b.turn(2, 1, false), None);
    }

    #[test]
    fn the_page_fills_the_knobs_a_bank_leaves_free() {
        let names: Vec<String> = ["Wash", "Grit"].map(String::from).to_vec();
        let mut b = Bindings::with_macros(&names);
        assert_eq!(b.banks().len(), 3);
        assert_eq!(b.title(), "", "no page yet");
        assert!(!b.card(0).enabled(), "bank 0 has no bindings of its own");
        b.set_page(
            "chain",
            "Filter",
            [
                Some(Binding::new(addr(4), MODE, "Mode")),
                Some(Binding::new(addr(3), CUT, "Cutoff")),
                None,
                None,
            ],
        );
        assert_eq!(
            (
                b.title(),
                b.card(0).label.as_str(),
                b.card(1).label.as_str()
            ),
            ("Filter", "Mode", "Cutoff")
        );
        assert!(!b.card(2).enabled(), "a knob the page leaves free");
        b.observe(addr(3), 2000.0);
        assert_eq!(b.card(1).text, value_text(&CUT, 2000.0));
        assert_eq!(
            b.turn(0, 1, false).unwrap(),
            Write {
                addr: addr(4),
                value: 1.0
            }
        );
        // The macro banks.
        b.next_bank();
        assert_eq!(b.title(), "Macros 1-4");
        let c = b.card(1);
        assert_eq!(
            (c.label.as_str(), c.colour, c.target),
            ("Grit", Some(1), Some(ParamAddr::macro_value(1)))
        );
        assert_eq!(b.card(2).label, "Macro 3");
        b.next_bank();
        assert_eq!(
            (b.title(), b.card(3).label.as_str()),
            ("Macros 5-8", "Macro 8")
        );
        b.next_bank();
        assert_eq!(b.shown(), 0, "round to the first");
        // A new page's knobs replace the old one's.
        b.set_page("chain", "Drive", [None, None, None, None]);
        assert!(!b.card(1).enabled());
    }

    #[test]
    fn a_learned_knob_belongs_to_its_page_unless_asked() {
        let mut b = Bindings::with_macros(&[]);
        let play = |b: &mut Bindings| {
            b.set_page(
                "play",
                "Play",
                [Some(Binding::new(addr(4), MODE, "Swing")), None, None, None],
            )
        };
        let chain = |b: &mut Bindings| {
            b.set_page(
                "chain",
                "Delay",
                [Some(Binding::new(addr(4), MODE, "Mix")), None, None, None],
            )
        };
        chain(&mut b);
        b.learn(addr(3), CUT, "Reverb mix");
        b.turn(0, 1, false);
        assert_eq!(b.card(0).label, "Reverb mix", "over the page's knob");
        play(&mut b);
        assert_eq!(b.card(0).label, "Swing", "Play keeps its own knob");
        b.learn_for(addr(3), CUT, "Cutoff", Scope::EveryPage);
        b.turn(1, 1, false);
        chain(&mut b);
        assert_eq!(
            (b.card(0).label.as_str(), b.card(1).label.as_str()),
            ("Reverb mix", "Cutoff"),
            "every page"
        );
        // The macro banks stay on every page.
        b.show(1);
        assert_eq!(b.card(0).label, "Macro 1");
        b.show(0);
        // Save and load keep both.
        let text = b.save();
        assert!(text.contains("page.chain.0.0 = "), "{text}");
        let describe = |a: ParamAddr| match a.owner {
            Owner::Macro(m) => Some(crate::engine::macro_info(m as usize)),
            _ => (a == addr(3)).then_some(CUT),
        };
        let (mut back, skipped) = Bindings::load(&text, &describe);
        assert_eq!((skipped, back.banks().len()), (0, 3));
        assert_eq!(back.banks()[1].name, "Macros 1-4");
        chain(&mut back);
        assert_eq!(
            (back.card(0).label.as_str(), back.card(1).label.as_str()),
            ("Reverb mix", "Cutoff")
        );
        play(&mut back);
        assert_eq!(back.card(0).label, "Swing");
        // Unbinding on the chain page takes its own knob off first.
        chain(&mut back);
        assert_eq!(
            back.unbind(0, 0).map(|x| x.label),
            Some("Reverb mix".into())
        );
        assert_eq!(back.card(0).label, "Mix");
    }

    #[test]
    fn learn_binds_the_next_knob_turned() {
        let mut b = Bindings::new();
        b.learn(addr(3), CUT, "Cutoff");
        assert_eq!(b.card(3).sub, "learn");
        assert_eq!(b.turn(3, 1, false), None);
        assert_eq!(b.binding(3).map(|x| x.target), Some(addr(3)));
        assert!(b.learning().is_none());
        assert!(b.turn(3, 1, false).is_some());
    }

    #[test]
    fn save_and_load() {
        let mut b = Bindings::new();
        b.bank_mut(0).name = "Macros 1-4".into();
        b.bind(
            0,
            0,
            Binding {
                colour: Some(2),
                ..Binding::new(ParamAddr::macro_value(0), crate::macro_info(0), "Wash")
            },
        );
        b.bind(
            1,
            3,
            Binding {
                from: 0.9,
                to: 0.1,
                ..Binding::new(addr(3), CUT, "Tone (keys)")
            },
        );
        let text = b.save();
        let (c, skipped) = Bindings::load(
            &(text.clone() + "knob.0.1 = t9.inst.1 0 1 - Gone\nnonsense\n"),
            &|a| match a.owner {
                Owner::Macro(n) => Some(crate::macro_info(n as usize)),
                Owner::Insert(..) => Some(CUT),
                _ => None,
            },
        );
        assert_eq!(skipped, 2);
        assert_eq!(c.save(), text);
        assert_eq!(c.banks()[1].knobs[3].as_ref().unwrap().label, "Tone (keys)");
    }
}
