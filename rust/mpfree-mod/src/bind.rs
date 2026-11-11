//! Q-Link (and touch slider) bindings: which parameter or macro each of
//! the four knobs moves, in banks of four.
//!
//! Pure control-thread logic with no I/O, for the app's Q-Link column:
//! [`Bindings::turn`] turns a knob into the value to set, and
//! [`Bindings::card`] says what the knob's card shows. The card has the
//! same fields as the mixer's Q-Link cards (`mpfree_mixer_ui::qlinks`),
//! so every page's Q-Links draw the same way.
//!
//! A page owner sets a page's default knobs; a user binding overrides
//! them (the app asks [`Bindings::card`] first and falls back to the
//! page's own when it says the knob is free).
//!
//! **Learn:** [`Bindings::learn`] arms a parameter (the user touched it
//! on screen); the next knob turned takes it, in the bank shown.
//!
//! Bindings save as `key = value` lines ([`Bindings::save`]) for
//! `mpfree-store`'s "mod" settings.

use std::collections::HashMap;

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
        Binding { target, info, label: label.to_string(), colour: None, from: 0.0, to: 1.0 }
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
    pub fn enabled(&self) -> bool { self.target.is_some() }
}

/// The banks, which is shown, and the last value of every bound
/// parameter.
#[derive(Clone, Debug, Default)]
pub struct Bindings {
    banks: Vec<Bank>,
    shown: usize,
    values: HashMap<ParamAddr, f32>,
    learning: Option<(ParamAddr, ParamInfo, String)>,
    /// The knob being touched, for lighting its card.
    pub touched: Option<usize>,
}

impl Bindings {
    pub fn new() -> Bindings { Bindings::default() }

    pub fn banks(&self) -> &[Bank] { &self.banks }
    pub fn shown(&self) -> usize { self.shown }

    /// Shows bank `b` (made empty if it is past the end).
    pub fn show(&mut self, b: usize) {
        while self.banks.len() <= b {
            let n = self.banks.len();
            self.banks.push(Bank { name: format!("Bank {}", n + 1), knobs: Default::default() });
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
        if k >= KNOBS { return; }
        let v = self.values.get(&binding.target).copied().unwrap_or(binding.info.default);
        self.values.insert(binding.target, v);
        self.bank_mut(b).knobs[k] = Some(binding);
    }

    pub fn unbind(&mut self, b: usize, k: usize) -> Option<Binding> { self.banks.get_mut(b)?.knobs.get_mut(k)?.take() }

    pub fn binding(&self, k: usize) -> Option<&Binding> { self.banks.get(self.shown)?.knobs.get(k)?.as_ref() }

    /// Tells the bindings a parameter's value (read back from the engine
    /// or its owner), so cards and turns start from it.
    pub fn observe(&mut self, addr: ParamAddr, value: f32) {
        if let Some(v) = self.values.get_mut(&addr) { *v = value; }
    }

    pub fn value(&self, addr: ParamAddr) -> Option<f32> { self.values.get(&addr).copied() }

    /// Arms learn: the next knob turned takes `addr`.
    pub fn learn(&mut self, addr: ParamAddr, info: ParamInfo, label: &str) { self.learning = Some((addr, info, label.to_string())); }
    pub fn cancel_learn(&mut self) { self.learning = None; }
    pub fn learning(&self) -> Option<ParamAddr> { self.learning.as_ref().map(|l| l.0) }

    /// Knob `k` turned by `detents` (clockwise positive); `fine` while
    /// SHIFT is held. The value to set, or `None` when nothing changes
    /// (a free knob, the end of the range, or a learn that just bound
    /// the knob).
    pub fn turn(&mut self, k: usize, detents: i32, fine: bool) -> Option<Write> {
        if k >= KNOBS { return None; }
        if let Some((addr, info, label)) = self.learning.take() {
            let b = self.shown;
            self.bind(b, k, Binding::new(addr, info, &label));
            return None;
        }
        if detents == 0 { return None; }
        let bnd = self.binding(k)?.clone();
        let info = bnd.info;
        let now = info.clamp(self.values.get(&bnd.target).copied().unwrap_or(info.default));
        let value = match info.curve {
            Curve::Toggle => if detents > 0 { 1.0 } else { 0.0 },
            // One value per detent, whatever the speed.
            Curve::Stepped | Curve::Choice(_) => {
                let dir = if (bnd.to >= bnd.from) == (detents > 0) { 1.0 } else { -1.0 };
                let (lo, hi) = range_values(&bnd);
                (now + dir * detents.unsigned_abs() as f32).clamp(lo, hi)
            }
            _ => {
                let span = bnd.to - bnd.from;
                if span.abs() < 1e-6 { return None; }
                let x = ((info.to_unit(now) - bnd.from) / span).clamp(0.0, 1.0);
                let step = if fine { 1.0 / (DETENTS * 10.0) } else { 1.0 / DETENTS };
                let x = (x + detents as f32 * step).clamp(0.0, 1.0);
                info.from_unit(bnd.from + span * x)
            }
        };
        let value = info.clamp(value);
        if value == now { return None; }
        self.values.insert(bnd.target, value);
        Some(Write { addr: bnd.target, value })
    }

    /// Knob `k` put at `pos` (0 to 1): a touch slider, or a knob read as a
    /// position.
    pub fn to(&mut self, k: usize, pos: f32) -> Option<Write> {
        let bnd = self.binding(k)?.clone();
        let x = pos.clamp(0.0, 1.0);
        let value = bnd.info.from_unit(bnd.from + (bnd.to - bnd.from) * x);
        let now = self.values.get(&bnd.target).copied();
        if now == Some(value) { return None; }
        self.values.insert(bnd.target, value);
        Some(Write { addr: bnd.target, value })
    }

    /// What knob `k`'s card shows.
    pub fn card(&self, k: usize) -> QLinkCard {
        let learn = if self.learning.is_some() { "learn" } else { "" };
        let Some(b) = self.binding(k) else {
            return QLinkCard { label: String::new(), text: String::new(), sub: learn, value: 0.0, pos: 0.0, colour: None, target: None };
        };
        let v = b.info.clamp(self.values.get(&b.target).copied().unwrap_or(b.info.default));
        let span = b.to - b.from;
        let pos = if span.abs() < 1e-6 { 0.0 } else { ((b.info.to_unit(v) - b.from) / span).clamp(0.0, 1.0) };
        QLinkCard { label: b.label.clone(), text: value_text(&b.info, v), sub: learn, value: v, pos, colour: b.colour, target: Some(b.target) }
    }

    pub fn cards(&self) -> [QLinkCard; KNOBS] { std::array::from_fn(|k| self.card(k)) }

    /// The bindings as `key = value` lines: `bank.0 = Name`, and
    /// `knob.0.1 = macro2.0 0.000 1.000 3 Wash` (target, from, to, colour
    /// or `-`, label).
    pub fn save(&self) -> String {
        let mut s = String::new();
        for (b, bank) in self.banks.iter().enumerate() {
            s += &format!("bank.{b} = {}\n", one_line(&bank.name));
            for (k, bnd) in bank.knobs.iter().enumerate() {
                let Some(x) = bnd else { continue };
                let c = x.colour.map_or("-".to_string(), |c| c.to_string());
                s += &format!("knob.{b}.{k} = {} {:.4} {:.4} {c} {}\n", x.target, x.from, x.to, one_line(&x.label));
            }
        }
        s
    }

    /// Reads [`Bindings::save`]'s lines back. `describe` gives a target's
    /// description; a target it does not know (an effect since removed)
    /// is skipped, as is any line that does not read. Returns how many
    /// lines were skipped.
    pub fn load(text: &str, describe: &dyn Fn(ParamAddr) -> Option<ParamInfo>) -> (Bindings, usize) {
        let mut out = Bindings::new();
        let mut skipped = 0;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
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
                        let (b, k): (usize, usize) = (b.parse().ok().filter(|b| *b < 64)?, k.parse().ok().filter(|k| *k < KNOBS)?);
                        let mut w = val.splitn(5, ' ');
                        let target: ParamAddr = w.next()?.parse().ok()?;
                        let from: f32 = w.next()?.parse().ok()?;
                        let to: f32 = w.next()?.parse().ok()?;
                        let colour = match w.next()? { "-" => None, c => Some(c.parse().ok()?) };
                        let label = w.next().unwrap_or("");
                        let info = describe(target)?;
                        if !(from.is_finite() && to.is_finite()) { return None; }
                        out.bind(b, k, Binding { target, info, label: label.to_string(), colour, from: from.clamp(0.0, 1.0), to: to.clamp(0.0, 1.0) });
                    }
                    _ => return None,
                }
                Some(())
            })();
            if ok.is_none() { skipped += 1; }
        }
        (out, skipped)
    }
}

/// The lowest and highest value a binding reaches.
fn range_values(b: &Binding) -> (f32, f32) {
    let (x, y) = (b.info.from_unit(b.from), b.info.from_unit(b.to));
    (x.min(y), x.max(y))
}

fn one_line(s: &str) -> String { s.replace(['\n', '\r'], " ") }

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::Unit;
    use mpfree_param::{Owner, Strip};

    const CUT: ParamInfo = ParamInfo { id: 3, name: "Cutoff", group: "Filter", min: 20.0, max: 20000.0, default: 1000.0, unit: Unit::Hz, curve: Curve::Log, readonly: false };
    const MODE: ParamInfo = ParamInfo { id: 4, name: "Mode", group: "Filter", min: 0.0, max: 3.0, default: 0.0, unit: Unit::None, curve: Curve::Choice(&["LP", "HP", "BP", "Notch"]), readonly: false };

    fn addr(p: u16) -> ParamAddr { ParamAddr::new(Owner::Insert(Strip::Track(0), 1), p) }

    #[test]
    fn turns_move_through_the_bound_range() {
        let mut b = Bindings::new();
        b.bind(0, 0, Binding { from: 0.25, to: 0.75, ..Binding::new(addr(3), CUT, "Cutoff") });
        b.observe(addr(3), CUT.from_unit(0.25));
        let w = b.turn(0, 50, false).unwrap();
        assert!((CUT.to_unit(w.value) - 0.5).abs() < 1e-4, "half the detents, half the bound range");
        assert!(b.turn(0, 500, false).is_some());
        assert!((CUT.to_unit(b.value(addr(3)).unwrap()) - 0.75).abs() < 1e-4, "stops at the top of the range");
        assert_eq!(b.turn(0, 1, false), None, "already at the top");
        let c = b.card(0);
        assert_eq!((c.pos, c.label.as_str(), c.enabled()), (1.0, "Cutoff", true));
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
        b.bind(0, 0, Binding { colour: Some(2), ..Binding::new(ParamAddr::macro_value(0), crate::macro_info(0), "Wash") });
        b.bind(1, 3, Binding { from: 0.9, to: 0.1, ..Binding::new(addr(3), CUT, "Tone (keys)") });
        let text = b.save();
        let (c, skipped) = Bindings::load(&(text.clone() + "knob.0.1 = t9.inst.1 0 1 - Gone\nnonsense\n"), &|a| match a.owner {
            Owner::Macro(n) => Some(crate::macro_info(n as usize)),
            Owner::Insert(..) => Some(CUT),
            _ => None,
        });
        assert_eq!(skipped, 2);
        assert_eq!(c.save(), text);
        assert_eq!(c.banks()[1].knobs[3].as_ref().unwrap().label, "Tone (keys)");
    }
}
