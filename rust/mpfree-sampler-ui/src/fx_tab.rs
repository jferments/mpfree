//! The pad editor's FX tab: the effects on the pad's own mixer channel as
//! a pedal board (Jay, 17 Nov 2026: the Sample tab as an instrument; the
//! FX page of the mockups he approved).
//!
//! - The **sound** across the page, with the way it goes under it: the
//!   sound, each effect in order (an effect switched off drawn faint),
//!   then the track it plays into.
//! - The **pedals** in the panel under it ([`crate::panel`]): each effect
//!   is a section with its number, its name and what it is set to, a
//!   switch that turns it on or off, and its first three knobs. Four show
//!   at once; with more, the **Effects** selector beside them moves the
//!   four along, and they follow the effect picked.
//! - **+ Add effect** opens the chain editor's picker on the pad's channel,
//!   and **Whole chain** the chain editor itself, where an effect's every
//!   control is (and its order, presets and modulation).
//! - A touch on a pedal picks it: its section is outlined and **Q-Links 1
//!   to 4** turn its four main knobs (the chain editor's Q-Links for it).
//!   The **wheel** and - / + step the control touched last, and a finger
//!   resting on a control lets the wheel turn it.
//!
//! The tab holds no effects of its own: the app reads the pad's channel
//! from the mixer ([`FxState::pedals`]) and applies what the tab asks for
//! ([`FxAction`]), as the chain editor does. Kit 1's pads A1 to A16 have a
//! channel of their own; any other pad is lent one when it is given an
//! effect (+ Add effect or Whole chain), and gives it back once it is
//! plain again. With every channel lent, the tab says why
//! ([`FxState::channel`]).

use std::sync::Arc;

use mpfree_ui::qlink::{QLinkCard, QLinkColumn};
use mpfree_ui::wave::Peaks;
use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Ui, UiEvent, Widget};

use crate::chop_tab::pad_text;
use crate::panel::{Control, Item, Look, Panel, Section};
use crate::play_tab::LayerSound;
use crate::wave_tab::{self, duration_text, pad_color, TABS};
use crate::widgets::{PadBar, Setting, SettingList};

/// Widget names the tab adds. The app's own are `header` and `hint`.
pub const BAR: &str = "smp-fx-bar";
pub const SOUND: &str = "smp-fx-sound";
pub const LIST: &str = "smp-fx-list";
pub const QLINKS: &str = "smp-fx-qlinks";

/// The tab's page: the sound across the page, the pedals under it, the
/// Q-Link cards on the right.
pub const LAYOUT: &str = "col(header:60, row(smp-fx-bar:*, smp-send:330):60, \
                          row(col(smp-fx-sound:*, smp-fx-list:242):*, smp-fx-qlinks:128):*, hint:26)";

/// The tab's place in [`TABS`].
pub const TAB: usize = wave_tab::FX_TAB;

/// The hint line the app shows under the tab.
pub const HINT: &str =
    "Touch an effect: its knobs go on the Q-Links · its switch turns it on or off · \
                        + Add effect: pick one · Whole chain: every control";

/// Pedals shown at once.
pub const SHOWN: usize = 4;
/// Knobs on a pedal (the Q-Links turn a fourth).
pub const PEDAL_KNOBS: usize = 3;

/// One knob of an effect, as the app reads it from the mixer.
#[derive(Clone, Debug, PartialEq)]
pub struct FxKnob {
    pub label: String,
    /// Its value as the effect writes it ("2.00 kHz").
    pub text: String,
    /// Where the knob points, 0 to 1.
    pub pos: f32,
    pub bipolar: bool,
    /// Steps from one end to the other, for how far a drag goes a step.
    pub steps: u32,
}

/// One effect on the pad's channel: its name, what it is set to, whether
/// it is on, and its main knobs (up to four: the pedal shows three).
#[derive(Clone, Debug, PartialEq)]
pub struct FxPedal {
    pub name: String,
    pub summary: String,
    pub on: bool,
    pub knobs: Vec<FxKnob>,
}

/// Everything the tab shows.
#[derive(Clone)]
pub struct FxState {
    pub pad: usize,
    /// The pad's name.
    pub name: String,
    /// The layer's sound and its region, from `start` to `end` frames.
    pub sound: LayerSound,
    pub start: u64,
    pub end: u64,
    /// Ok when the pad can have effects (a channel of its own, or one free
    /// to lend it); else why not.
    pub channel: Result<(), String>,
    /// The channel's effects, once the mixer has said (None until then).
    pub pedals: Option<Vec<FxPedal>>,
    /// How many effects a channel holds.
    pub room: usize,
    /// Where the channel plays into ("Drums").
    pub into: String,
}

impl FxState {
    fn pedals(&self) -> &[FxPedal] {
        self.pedals.as_deref().unwrap_or(&[])
    }
}

/// What the app does for a touch or a turn on the tab.
#[derive(Clone, Debug, PartialEq)]
pub enum FxAction {
    /// Open the editor's tab `i` of [`TABS`].
    Tab(usize),
    /// Switch effect `slot` on or off.
    On { slot: usize, on: bool },
    /// Turn knob `knob` of effect `slot` by `detents` (a Q-Link's, finer
    /// with `fine`).
    Turn {
        slot: usize,
        knob: usize,
        detents: i32,
        fine: bool,
    },
    /// The chain editor's picker on the pad's channel.
    Add,
    /// The chain editor on the pad's channel.
    Whole,
}

/// What a row of the panel is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    On(usize),
    Knob(usize, usize),
    Add,
    Whole,
    /// Which four effects show.
    Window,
}

/// The FX tab's widgets.
pub struct FxTab {
    /// The page name the tab's layout is added under.
    pub name: String,
    /// The bank letter shown in pad names, as [`wave_tab::WaveTab::bank`].
    pub bank: Option<char>,
    /// The effect picked (its slot in the chain): the Q-Links turn it.
    picked: usize,
    /// The first effect shown.
    first: usize,
    /// The panel's rows as last shown.
    rows: Vec<Row>,
    /// The pad shown, to start a new pad on its first effect.
    pad: Option<usize>,
}

impl FxTab {
    /// Adds the widgets and the page `name` to `ui` (the app adds `header`
    /// and `hint` first) and shows `st`.
    pub fn add(ui: &mut Ui, name: &str, st: &FxState) -> Result<FxTab, String> {
        crate::send::add(ui);
        let mut bar = PadBar::new(&TABS);
        bar.enabled = TABS.iter().map(|t| wave_tab::BUILT.contains(t)).collect();
        bar.active = TAB;
        ui.add(BAR, bar);
        ui.add(SOUND, FxSound::new());
        let mut list = SettingList::new("Effects");
        list.selected = 0;
        ui.add(LIST, list);
        ui.add(QLINKS, QLinkColumn::new());
        ui.add_page(name, LAYOUT)?;
        let mut tab = FxTab {
            name: name.to_string(),
            bank: None,
            picked: 0,
            first: 0,
            rows: Vec::new(),
            pad: None,
        };
        tab.update(ui, st);
        Ok(tab)
    }

    /// The effect picked: the one whose knobs are on the Q-Links.
    pub fn picked(&self) -> usize {
        self.picked
    }

    /// The first effect shown.
    pub fn first(&self) -> usize {
        self.first
    }

    /// Shows `st`.
    pub fn update(&mut self, ui: &mut Ui, st: &FxState) {
        let n = st.pedals().len();
        if self.pad != Some(st.pad) {
            self.pad = Some(st.pad);
            self.picked = 0;
            self.first = 0;
        }
        self.picked = self.picked.min(n.saturating_sub(1));
        self.follow();
        self.first = self.first.min(n.saturating_sub(SHOWN));
        let pad = pad_text(self.bank, st.pad);
        if let Some(bar) = ui.get_mut::<PadBar>(BAR) {
            bar.color = pad_color(st.pad);
            bar.active = TAB;
            bar.title = format!("Pad {pad} · {}", st.name);
            bar.subtitle = match (&st.channel, &st.pedals) {
                (Err(why), _) => why.clone(),
                (Ok(()), None) => "Reading its channel".to_string(),
                (Ok(()), Some(p)) if p.is_empty() => {
                    format!("No effects yet · into {}", st.into)
                }
                (Ok(()), Some(p)) => {
                    format!("{} of {} effects · into {}", p.len(), st.room, st.into)
                }
            };
        }
        if let Some(s) = ui.get_mut::<FxSound>(SOUND) {
            s.title = format!("Pad {pad} through its effects");
            s.right = format!(
                "{} · {} long",
                st.sound.name,
                duration_text(st.end.saturating_sub(st.start), st.sound.rate)
            );
            s.set_sound(Some(st.sound.view.peaks.clone()), st.start, st.end);
            s.chain = st.pedals().iter().map(|p| (p.name.clone(), p.on)).collect();
            s.into = st.into.clone();
            s.note = match &st.channel {
                Err(why) => why.clone(),
                Ok(()) => String::new(),
            };
        }
        // The control picked stays picked as the rows move (another four
        // shown, an effect added); one that went picks the picked effect.
        let was = ui
            .get::<SettingList>(LIST)
            .and_then(|l| self.rows.get(l.selected).copied());
        self.rows = self.rows_for(st);
        let rows: Vec<Setting> = self.rows.iter().map(|r| self.setting(st, *r)).collect();
        let controls = self.controls(st);
        let panel = self.panel(st);
        let selected = was
            .and_then(|w| self.rows.iter().position(|r| *r == w))
            .or_else(|| {
                self.rows
                    .iter()
                    .position(|r| matches!(r, Row::On(s) if *s == self.picked))
            })
            .unwrap_or(0);
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.right = format!("pad {pad}");
            list.buttons = self
                .rows
                .iter()
                .enumerate()
                .filter(|(_, r)| matches!(r, Row::Add | Row::Whole))
                .map(|(i, _)| i)
                .collect();
            list.rows = rows;
            list.controls = controls;
            list.panel = Some(panel);
            list.selected = selected;
        }
        self.show_cards(ui, st);
    }

    /// The Q-Link cards: the picked effect's main knobs.
    fn show_cards(&self, ui: &mut Ui, st: &FxState) {
        let pedal = st.pedals().get(self.picked);
        let mut cards: [QLinkCard; 4] = Default::default();
        if let Some(p) = pedal {
            for (card, k) in cards.iter_mut().zip(&p.knobs) {
                *card = QLinkCard::new(&k.label, &p.name, &k.text, k.pos);
                card.bipolar = k.bipolar;
            }
        }
        let title = pedal.map_or_else(
            || "Q-Links · FX".to_string(),
            |p| format!("Q-Links · {}", p.name),
        );
        if let Some(q) = ui.get_mut::<QLinkColumn>(QLINKS) {
            if q.cards != cards || q.title != title {
                q.cards = cards;
                q.title = title;
            }
        }
    }

    /// The first effect shown moves so the picked one shows.
    fn follow(&mut self) {
        if self.picked < self.first {
            self.first = self.picked;
        } else if self.picked >= self.first + SHOWN {
            self.first = self.picked + 1 - SHOWN;
        }
    }

    /// The panel's row `r`: its name and its value.
    fn setting(&self, st: &FxState, r: Row) -> Setting {
        let (name, value) = match r {
            Row::On(s) => (
                "On".to_string(),
                if st.pedals()[s].on { "On" } else { "Off" }.to_string(),
            ),
            Row::Knob(s, k) => {
                let kn = &st.pedals()[s].knobs[k];
                (knob_label(&kn.label), kn.text.clone())
            }
            Row::Add => ("Add effect".to_string(), String::new()),
            Row::Whole => ("Whole chain".to_string(), String::new()),
            Row::Window => {
                let n = st.pedals().len();
                let last = (self.first + SHOWN).min(n);
                (
                    "Effects".to_string(),
                    format!("{} to {last} of {n}", self.first + 1),
                )
            }
        };
        Setting { name, value }
    }

    fn rows_for(&self, st: &FxState) -> Vec<Row> {
        let pedals = st.pedals();
        let mut rows = Vec::new();
        for (slot, p) in pedals.iter().enumerate().skip(self.first).take(SHOWN) {
            rows.push(Row::On(slot));
            rows.extend((0..p.knobs.len().min(PEDAL_KNOBS)).map(|k| Row::Knob(slot, k)));
        }
        rows.push(Row::Add);
        rows.push(Row::Whole);
        if pedals.len() > SHOWN {
            rows.push(Row::Window);
        }
        rows
    }

    fn controls(&self, st: &FxState) -> Vec<Control> {
        let no_channel = st.channel.as_ref().err().map(String::as_str);
        let full = st.pedals().len() >= st.room;
        self.rows
            .iter()
            .map(|r| match *r {
                Row::On(s) => Control::new(Look::Toggle(st.pedals()[s].on)),
                Row::Knob(s, k) => {
                    let kn = &st.pedals()[s].knobs[k];
                    Control::new(Look::Knob {
                        pos: kn.pos,
                        steps: kn.steps,
                        bipolar: kn.bipolar,
                    })
                }
                Row::Add => Control::new(Look::Button("+ Add effect".to_string())).faint_because(
                    no_channel.is_some() || full || st.pedals.is_none(),
                    no_channel.unwrap_or(if full {
                        "The channel is full: take an effect out on Whole chain first"
                    } else {
                        "Reading the pad's channel"
                    }),
                ),
                Row::Whole => Control::new(Look::Button("Whole chain".to_string()))
                    .faint_because(no_channel.is_some(), no_channel.unwrap_or_default()),
                Row::Window => Control::new(Look::Chip),
            })
            .collect()
    }

    fn panel(&self, st: &FxState) -> Panel {
        let at = |r: Row| self.rows.iter().position(|x| *x == r).unwrap_or(0);
        let mut sections = Vec::new();
        for (slot, p) in st.pedals().iter().enumerate().skip(self.first).take(SHOWN) {
            let knobs = (0..p.knobs.len().min(PEDAL_KNOBS))
                .map(|k| Item::new(at(Row::Knob(slot, k))))
                .collect::<Vec<_>>();
            let mut lines = vec![vec![Item::new(at(Row::On(slot)))]];
            if !knobs.is_empty() {
                lines.push(knobs);
            }
            sections.push(
                Section::new(&format!("{} {}", slot + 1, p.name), 240, lines)
                    .note(&p.summary)
                    .lit(slot == self.picked),
            );
        }
        let n = st.pedals().len();
        if n == 0 {
            let (title, note) = match (&st.channel, &st.pedals) {
                (Err(_), _) => ("No channel free", String::new()),
                (Ok(()), None) => ("Effects", "reading the channel".to_string()),
                (Ok(()), Some(_)) => ("No effects yet", "+ Add effect puts one on".to_string()),
            };
            sections.push(Section::new(title, 960, Vec::new()).note(&note));
        }
        let mut lines = vec![
            vec![Item::new(at(Row::Add))],
            vec![Item::new(at(Row::Whole))],
        ];
        if n > SHOWN {
            lines.push(vec![Item::new(at(Row::Window))]);
        }
        sections.push(Section::new("Chain", 160, lines).note(&format!("{n} of {}", st.room)));
        Panel { sections }
    }

    /// What a touch on one of the tab's widgets asks for; None for touches
    /// that only change what the tab shows. A touch on a pedal's control
    /// picks the pedal: its section is outlined and the Q-Links turn it.
    pub fn action(&mut self, ui: &mut Ui, ev: &UiEvent, st: &FxState) -> Option<FxAction> {
        let index = match (ev.widget.as_str(), &ev.kind) {
            (BAR, EventKind::Tab(i)) => return Some(FxAction::Tab(*i)),
            (QLINKS, EventKind::Step { index, delta }) => {
                return self.qlink(st, *index, *delta, false)
            }
            (LIST, EventKind::Tab(i) | EventKind::Select(i) | EventKind::Step { index: i, .. }) => {
                *i
            }
            _ => return None,
        };
        let row = *self.rows.get(index)?;
        if let Row::On(slot) | Row::Knob(slot, _) = row {
            // The list picks the row a finger touched; a touch sent
            // another way does the same.
            if ui
                .get::<SettingList>(LIST)
                .is_some_and(|l| l.selected != index)
            {
                if let Some(l) = ui.get_mut::<SettingList>(LIST) {
                    l.selected = index;
                }
            }
            if slot != self.picked {
                self.picked = slot;
                self.update(ui, st);
            }
        }
        match (row, &ev.kind) {
            (Row::Add, EventKind::Tab(_)) => Some(FxAction::Add),
            (Row::Whole, EventKind::Tab(_)) => Some(FxAction::Whole),
            // A tap on a switch flips it.
            (Row::On(slot), EventKind::Step { .. }) => Some(FxAction::On {
                slot,
                on: !st.pedals().get(slot)?.on,
            }),
            (_, EventKind::Step { delta, .. }) => self.step(ui, st, row, *delta),
            _ => None,
        }
    }

    /// The data wheel (or - / +): steps the control picked by `delta`.
    pub fn wheel(&mut self, ui: &mut Ui, st: &FxState, delta: i32) -> Option<FxAction> {
        let row = ui
            .get::<SettingList>(LIST)
            .and_then(|l| self.rows.get(l.selected).copied())?;
        match row {
            Row::On(slot) => {
                let on = delta > 0;
                (st.pedals().get(slot)?.on != on).then_some(FxAction::On { slot, on })
            }
            _ => self.step(ui, st, row, delta),
        }
    }

    /// Q-Link `k` turned `ticks`: knob `k` of the effect picked.
    pub fn qlink(&self, st: &FxState, k: usize, ticks: i32, fine: bool) -> Option<FxAction> {
        let p = st.pedals().get(self.picked)?;
        (k < p.knobs.len() && ticks != 0).then_some(FxAction::Turn {
            slot: self.picked,
            knob: k,
            detents: ticks,
            fine,
        })
    }

    fn step(&mut self, ui: &mut Ui, st: &FxState, row: Row, delta: i32) -> Option<FxAction> {
        match row {
            Row::Knob(slot, knob) => {
                self.picked = slot;
                Some(FxAction::Turn {
                    slot,
                    knob,
                    detents: delta,
                    fine: false,
                })
            }
            Row::Window => {
                let n = st.pedals().len();
                let first = (self.first as i64 + i64::from(delta.signum()) * SHOWN as i64)
                    .clamp(0, n.saturating_sub(SHOWN) as i64) as usize;
                if first != self.first {
                    self.first = first;
                    self.picked = first;
                    self.update(ui, st);
                    // The selector stays picked as the pedals change.
                    let at = self.rows.iter().position(|r| *r == Row::Window);
                    if let (Some(l), Some(k)) = (ui.get_mut::<SettingList>(LIST), at) {
                        l.selected = k;
                    }
                }
                None
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- sound

/// The pad's sound across the page, and under it the way it goes: the
/// sound, each effect in order (faint when switched off), then the track.
pub struct FxSound {
    pub title: String,
    pub right: String,
    peaks: Option<Arc<dyn Peaks>>,
    span: (u64, u64),
    /// The effects in order, each with whether it is on.
    pub chain: Vec<(String, bool)>,
    /// Where the channel plays into.
    pub into: String,
    /// Said over the sound: why the pad has no effects of its own.
    pub note: String,
}

impl Default for FxSound {
    fn default() -> Self {
        FxSound::new()
    }
}

/// The way line's height, under the sound.
const WAY_H: i32 = 34;

impl FxSound {
    pub fn new() -> FxSound {
        FxSound {
            title: String::new(),
            right: String::new(),
            peaks: None,
            span: (0, 1),
            chain: Vec::new(),
            into: String::new(),
            note: String::new(),
        }
    }

    /// The sound to draw: `peaks` from frame `start` to `end`.
    pub fn set_sound(&mut self, peaks: Option<Arc<dyn Peaks>>, start: u64, end: u64) {
        self.peaks = peaks;
        self.span = (start, end.max(start + 1));
    }

    /// The words of the way line, each with whether it is lit.
    pub fn way(&self) -> Vec<(String, bool)> {
        let mut w = vec![("Sound".to_string(), true)];
        w.extend(self.chain.iter().cloned());
        if !self.into.is_empty() {
            w.push((self.into.clone(), true));
        }
        w
    }
}

impl Widget for FxSound {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let body = cx.titled_panel(r, &self.title, &self.right);
        let (way, plot) = body.take_bottom(WAY_H);
        let plot = plot.inset_xy(0, 4);
        let mid = plot.y + plot.h / 2;
        let half = (plot.h / 2) as f32;
        if let Some(p) = &self.peaks {
            let (s0, s1) = (self.span.0 as f64, self.span.1 as f64);
            let fpp = (s1 - s0) / plot.w.max(1) as f64;
            let ink = t.accent.mix(t.surface, 0.25);
            for x in 0..plot.w {
                let f0 = s0 + f64::from(x) * fpp;
                let mut c = p.column(0, f0, f0 + fpp);
                if p.channels() > 1 {
                    let c2 = p.column(1, f0, f0 + fpp);
                    (c.min, c.max) = (c.min.min(c2.min), c.max.max(c2.max));
                }
                if c.max < c.min {
                    continue;
                }
                let top = mid - (c.max.clamp(-1.0, 1.0) * half) as i32;
                let bot = mid - (c.min.clamp(-1.0, 1.0) * half) as i32;
                cx.canvas
                    .fill_rect(Rect::new(plot.x + x, top, 1, (bot - top).max(1)), ink);
            }
        }
        cx.canvas
            .fill_rect(Rect::new(plot.x, mid, plot.w, 1), t.outline);
        if !self.note.is_empty() {
            let n = Rect::new(plot.x, mid - 16, plot.w, 32);
            cx.canvas
                .fill_round_rect(n.inset_xy(plot.w / 6, 0), 8, t.surface);
            cx.text(FontId::Sans, 15.0, n, Align::Center, t.text_dim, &self.note);
        }
        // The way: chips left to right, joined by arrows.
        let mut x = way.x;
        let y = way.y + 4;
        let h = WAY_H - 8;
        let words = self.way();
        let last = words.len().saturating_sub(1);
        for (i, (w, on)) in words.iter().enumerate() {
            let label = if *on { w.clone() } else { format!("{w} (off)") };
            let tw = cx.measure(FontId::Bold, 13.0, &label);
            let chip = Rect::new(x, y, tw + 20, h);
            if chip.right() > way.right() {
                break;
            }
            let (fill, ink) = if *on {
                (t.raised, t.text)
            } else {
                (t.surface, t.text_faint)
            };
            cx.canvas.fill_round_rect(chip, h / 2, fill);
            cx.canvas.stroke_round_rect(chip, h / 2, 1, t.outline);
            cx.text(FontId::Bold, 13.0, chip, Align::Center, ink, &label);
            x = chip.right() + 6;
            if i < last {
                let a = Rect::new(x, y, 16, h);
                cx.text(FontId::Sans, 14.0, a, Align::Center, t.text_dim, "›");
                x += 22;
            }
        }
    }
}

/// A knob's name short enough for a pedal's three knobs: "Band 1 freq" is
/// "B1 freq", "Resonance" "Reso". The Q-Link cards keep the full name.
pub fn knob_label(label: &str) -> String {
    let label = match label.strip_prefix("Band ") {
        Some(rest) => format!("B{rest}"),
        None => label.to_string(),
    };
    if label.chars().count() <= 8 {
        return label;
    }
    [
        ("Resonance", "Reso"),
        ("Threshold", "Thresh"),
        ("Frequency", "Freq"),
        ("frequency", "freq"),
        ("Feedback", "Fdbk"),
        ("Release", "Rel"),
    ]
    .iter()
    .fold(label, |l, (long, short)| l.replace(long, short))
}
