//! The arp's two screens: what they show, worked out from the engine, and
//! what their touches mean.
//!
//! - The **Arp panel** shows every setting of one track's arp beside the
//!   pads: mode tabs, the list the wheel moves through, the Q-Link sliders or
//!   the pattern buttons, and the step strip while the arp is generating.
//! - The **quick band** is a slim strip at the top of the right-hand column
//!   while NOTE REPEAT is held, or while notes are latched: what the arp
//!   plays, the rate with − and +, and Latch. The rest of the column stays
//!   the app's.
//!
//! The arp is a way of playing the pads, not a place, so neither is a
//! top-level tab: the app shows the panel and the band in its Play page's
//! right-hand column, so the pads never move, and puts [`chip`] in its
//! header. The
//! design is in the arpeggiator design doc (`docs/project/arpeggiator.md`).
//!
//! The screens do not own the engine. The app copies what they need into an
//! [`ArpState`] (from [`ArpState::from_engine`]), calls [`ArpPanel::update`]
//! when it changes, and turns their [`ArpAction`]s into engine calls. The
//! pads, the header and the hint line belong to the app; the panel only adds
//! its own widgets and the layouts that place them.

use crate::widgets::{
    Button, ButtonRow, CellKind, MiniBar, Segments, SliderStack, StepCell, StepStrip, MINI_FASTER, MINI_LATCH,
    MINI_OPEN, MINI_SLOWER,
};
use mpfree_arp::{Engine, Held, Mode, Order, Pattern, Pick, Rate, Settings, Step, MAX_NOTES};
use mpfree_ui::widgets::{Chip, ListItem, ListView};
use mpfree_ui::{EventKind, Tone, Ui, UiEvent};

/// Widget names the page adds. The app's own are `header`, `pads` and `hint`.
pub const MODES: &str = "arp-modes";
pub const LIST: &str = "arp-list";
pub const QLINKS: &str = "arp-qlinks";
pub const ACTIONS: &str = "arp-actions";
pub const STEPS: &str = "arp-steps";
pub const MINI: &str = "arp-mini";

/// Every layout keeps the pads where the Play page has them: the arp's
/// widgets only ever take the right-hand column, [`SIDE`] pixels wide, so
/// nothing under the player's fingers moves or resizes when the band or the
/// panel comes up.
///
/// Direct: nothing is generated, so there is no step strip.
pub const DIRECT_LAYOUT: &str =
    "col(header:60, row(pads:*, col(arp-modes:52, arp-list:*, arp-qlinks:236):460):*, hint:26)";
/// Repeat and Arp: the list (rates or orders) over the Q-Link sliders, and
/// the step strip.
pub const ARP_LAYOUT: &str =
    "col(header:60, row(pads:*, col(arp-modes:52, arp-list:*, arp-qlinks:236, arp-steps:112):460):*, hint:26)";
/// Pattern: the pattern list over the pattern buttons, and the step strip.
pub const PATTERN_LAYOUT: &str =
    "col(header:60, row(pads:*, col(arp-modes:52, arp-list:*, arp-actions:128, arp-steps:112):460):*, hint:26)";
/// The quick band: a slim strip ([`MINI`], [`MINI_H`] pixels) at the top of
/// the right-hand column. An app puts it over its own column with
/// [`ArpPanel::set_quick_layout`] (the pad player keeps its kits, level and
/// timing under it); this default puts the next steps under it.
pub const QUICK_LAYOUT: &str = "col(header:60, row(pads:*, col(arp-mini:56, arp-steps:112):460):*, hint:26)";
/// The strip's height in the layouts.
pub const MINI_H: i32 = 56;
/// The width of the right-hand column every layout here shares with the
/// app's Play page.
pub const SIDE: i32 = 460;

/// The mode tabs, in [`Mode`] order.
pub const MODE_TABS: [&str; 4] = ["Direct", "Repeat", "Arp", "Pattern"];
/// The Q-Links, top to bottom, as Jay's plan maps them.
pub const QLINK_LABELS: [&str; 4] = ["Rate · Q1", "Gate · Q2", "Swing · Q3", "Octaves · Q4"];
/// The pattern buttons, in [`PatternButton`] order.
pub const PATTERN_BUTTONS: [(&str, Tone); 4] =
    [("Edit pattern", Tone::Accent), ("Record on pads", Tone::Bad), ("Mutate", Tone::Normal), ("Original", Tone::Normal)];
/// The most cells the step strip shows at once.
pub const STRIP_CELLS: usize = 16;

const MODES_ALL: [Mode; 4] = [Mode::Direct, Mode::Repeat, Mode::Arp, Mode::Pattern];

/// Everything the page shows.
#[derive(Clone, Debug)]
pub struct ArpState {
    pub settings: Settings,
    pub pattern: Pattern,
    /// The patterns to choose from, by name, and which one is loaded.
    pub patterns: Vec<(String, Pattern)>,
    pub pattern_index: usize,
    /// The loaded pattern was changed (mutated, edited or recorded) from the
    /// one in the list: Original puts it back.
    pub pattern_changed: bool,
    /// The notes the generator plays, in the order they were pressed.
    pub notes: Vec<Held>,
    /// Steps played since the notes started ([`Engine::step`]).
    pub step: u64,
    pub latched: bool,
    pub bpm: f64,
}

impl ArpState {
    /// What the engine shows now. The pattern list and its state are the
    /// app's, so they are passed in.
    pub fn from_engine(e: &Engine, patterns: Vec<(String, Pattern)>, pattern_index: usize, pattern_changed: bool)
        -> ArpState {
        ArpState {
            settings: e.settings(),
            pattern: *e.pattern(),
            patterns,
            pattern_index,
            pattern_changed,
            notes: e.active().as_slice().to_vec(),
            step: e.step(),
            latched: e.latch() != mpfree_arp::LatchMode::Off,
            bpm: e.bpm(),
        }
    }

    fn sorted(&self) -> Vec<Held> {
        let mut v = self.notes.clone();
        v.sort_by_key(|h| h.note);
        v.truncate(MAX_NOTES);
        v
    }
}

/// What a touch on the page asks for. The app applies it to the engine.
#[derive(Clone, Debug, PartialEq)]
pub enum ArpAction {
    Mode(Mode),
    Rate(Rate),
    Order(Order),
    /// Load pattern `i` of [`ArpState::patterns`].
    Pattern(usize),
    /// A Q-Link slider was touched or moved: the new settings.
    Settings(Settings),
    Button(PatternButton),
    /// A cell of the step strip was tapped: step `i` of the pattern.
    Step(usize),
    /// The band's Latch button: latch on, or off (clearing the notes).
    Latch,
    /// The strip's name was tapped: open the Arp panel.
    OpenPanel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternButton {
    Edit,
    Record,
    Mutate,
    Original,
}

/// A pad or MIDI note as the screen names it: pads by number (slot 0 is
/// pad 1), other notes by name.
pub fn note_label(note: u8) -> String {
    if note < 16 {
        format!("{}", note + 1)
    } else {
        const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        format!("{}{}", NAMES[usize::from(note % 12)], i32::from(note / 12) - 1)
    }
}

fn octave_sub(oct: i32) -> String {
    match oct {
        0 => String::new(),
        o if o > 0 => format!("oct +{o}"),
        o => format!("oct {o}"),
    }
}

/// The Q-Link sliders' positions, 0.0 to 1.0, and their texts.
pub fn qlink_values(s: &Settings) -> [(f32, String); 4] {
    let rate = Rate::ALL.iter().position(|r| *r == s.rate).unwrap_or(0);
    [
        (rate as f32 / (Rate::ALL.len() - 1) as f32, s.rate.label().to_string()),
        (f32::from(s.gate - 1) / 99.0, format!("{} %", s.gate)),
        (f32::from(s.swing - 50) / 25.0, if s.swing == 50 { "off".to_string() } else { format!("{} %", s.swing) }),
        (f32::from(s.octaves - 1) / 3.0, format!("{}", s.octaves)),
    ]
}

/// The settings with Q-Link `i` moved to `v` (0.0 to 1.0).
pub fn with_qlink(mut s: Settings, i: usize, v: f32) -> Settings {
    let v = v.clamp(0.0, 1.0);
    let pick = |n: usize| ((v * (n - 1) as f32).round() as usize).min(n - 1);
    match i {
        0 => s.rate = Rate::ALL[pick(Rate::ALL.len())],
        1 => s.gate = 1 + pick(100) as u8,
        2 => s.swing = 50 + pick(26) as u8,
        3 => s.octaves = 1 + pick(4) as u8,
        _ => {}
    }
    s.clamped()
}

/// A short picture of an order played over three notes, such as "1 2 3 2".
pub fn order_preview(order: Order) -> String {
    match order {
        Order::Chord => "1+2+3".to_string(),
        Order::Played => "as pressed".to_string(),
        _ => match order.cycle(3) {
            Some(n) => (0..n.min(6)).map(|c| format!("{}", order.place(3, c).unwrap_or(0) + 1)).collect::<Vec<_>>().join(" "),
            None => "?".to_string(),
        },
    }
}

/// The step strip for the Arp and Repeat modes: the order's notes over the
/// held pads. Returns the cells, the first step's number and the playing cell.
pub fn arp_cells(st: &ArpState) -> (Vec<StepCell>, usize, Option<usize>) {
    let s = &st.settings;
    let sorted = st.sorted();
    let n = sorted.len();
    if n == 0 || s.mode == Mode::Direct {
        return (Vec::new(), 0, None);
    }
    let all = |label: &str| StepCell { kind: CellKind::Chord, label: label.to_string(), ..StepCell::default() };
    if s.mode == Mode::Repeat || s.order == Order::Chord {
        let label = if n == 1 { note_label(sorted[0].note) } else { format!("all {n}") };
        let playing = (st.step > 0).then_some(0);
        return (vec![all(&label)], 0, playing);
    }
    let base: Vec<Held> = if s.order == Order::Played { st.notes.iter().take(MAX_NOTES).copied().collect() } else { sorted };
    let total = n * usize::from(s.octaves);
    let Some(cycle) = s.order.cycle(total) else {
        // The random orders: as many cells as notes, none known ahead.
        let cells = (0..total.min(STRIP_CELLS))
            .map(|_| StepCell { kind: CellKind::Random, label: "?".to_string(), ..StepCell::default() })
            .collect();
        let playing = (st.step > 0).then(|| ((st.step - 1) as usize) % total.min(STRIP_CELLS));
        return (cells, 0, playing);
    };
    let last = (st.step > 0).then(|| ((st.step - 1) as usize) % cycle);
    let first = last.map_or(0, |l| l / STRIP_CELLS * STRIP_CELLS);
    let cells = (first..cycle.min(first + STRIP_CELLS))
        .map(|c| {
            let i = s.order.place(total, c).unwrap_or(0);
            let h = base[i % n];
            StepCell { kind: CellKind::Note, label: note_label(h.note), sub: octave_sub((i / n) as i32), accent: false }
        })
        .collect();
    (cells, first, last.map(|l| l - first))
}

/// One pattern step as a cell.
pub fn pattern_cell(step: Step) -> StepCell {
    match step {
        Step::Rest => StepCell { kind: CellKind::Rest, label: "·".into(), ..StepCell::default() },
        Step::Tie => StepCell { kind: CellKind::Tie, label: "—".into(), ..StepCell::default() },
        Step::Play { pick, octave, accent, ratchet, chance, .. } => {
            let (kind, label) = match pick {
                Pick::Next => (CellKind::Note, "x".to_string()),
                Pick::All => (CellKind::Chord, "all".to_string()),
                Pick::Index(i) => (CellKind::Note, format!("{i}")),
                Pick::Random => (CellKind::Random, "?".to_string()),
                Pick::Lowest => (CellKind::Note, "low".to_string()),
                Pick::Highest => (CellKind::Note, "high".to_string()),
            };
            let mut sub = Vec::new();
            if octave != 0 {
                sub.push(octave_sub(i32::from(octave)));
            }
            if ratchet > 1 {
                sub.push(format!("×{ratchet}"));
            }
            if chance < 100 {
                sub.push(format!("{chance}%"));
            }
            StepCell { kind, label, sub: sub.join(" "), accent }
        }
    }
}

/// The step strip for the Pattern mode: the pattern's steps, a window of
/// [`STRIP_CELLS`] around the one playing.
pub fn pattern_cells(st: &ArpState) -> (Vec<StepCell>, usize, Option<usize>) {
    let steps = st.pattern.steps();
    if steps.is_empty() {
        return (Vec::new(), 0, None);
    }
    let last = (st.step > 0 && !st.notes.is_empty()).then(|| ((st.step - 1) as usize) % steps.len());
    let first = last.map_or(0, |l| l / STRIP_CELLS * STRIP_CELLS);
    let cells = steps[first..steps.len().min(first + STRIP_CELLS)].iter().map(|s| pattern_cell(*s)).collect();
    (cells, first, last.map(|l| l - first))
}

/// The arp chip: what the arp does, in one line, for the app's header on
/// every page. Tapping it opens the panel (the app's job). Off and latched
/// look different at a glance.
pub fn chip(st: &ArpState) -> Chip {
    let s = &st.settings;
    let what = match s.mode {
        Mode::Direct => {
            return if st.latched { Chip::new("Latch on", Tone::Warn) } else { Chip::new("Arp off", Tone::Dim) };
        }
        Mode::Repeat => format!("Repeat {}", s.rate.label()),
        Mode::Arp => format!("Arp: {} {}", s.order.label(), s.rate.label()),
        Mode::Pattern => {
            let name = st.patterns.get(st.pattern_index).map_or("pattern", |(n, _)| n.as_str());
            format!("Pattern: {name}{} {}", if st.pattern_changed { "*" } else { "" }, s.rate.label())
        }
    };
    if st.latched {
        Chip::new(format!("{what}, latched"), Tone::Warn)
    } else {
        Chip::new(what, Tone::Accent)
    }
}

/// The header chips for the sample screens: the arp chip, then the tempo
/// at the right edge. (The header lays chips out from the right edge, so the
/// last one here is the leftmost.)
pub fn chips(st: &ArpState) -> Vec<Chip> {
    vec![Chip::new(format!("{:.1} BPM", st.bpm), Tone::Good), chip(st)]
}

/// The hint line under the panel, for each mode.
pub fn hint(mode: Mode) -> &'static str {
    match mode {
        Mode::Direct => "Pads play as pressed · hold NOTE REPEAT to repeat · press it twice: latch",
        Mode::Repeat => "Hold or latch pads · Wheel: rate · Q-Links: rate, gate, swing, octaves",
        Mode::Arp => "Hold or latch pads · Wheel: order · Q-Links: rate, gate, swing, octaves · TAP TEMPO",
        Mode::Pattern => "Patterns play the chord you hold: \"2\" is the 2nd lowest held pad · Wheel: pattern",
    }
}

/// The hint line under the quick band.
pub fn quick_hint(mode: Mode) -> &'static str {
    match mode {
        Mode::Pattern => "Tap a rate · Q-Links: rate, gate, swing, octaves · Wheel: pattern · let go: back to your page",
        Mode::Arp => "Tap a rate · Q-Links: rate, gate, swing, octaves · Wheel: order · let go: back to your page",
        _ => "Tap a rate while you play · press harder for louder · let go: back to your page",
    }
}

/// The panel's layout for a mode.
pub fn layout_for(mode: Mode) -> &'static str {
    match mode {
        Mode::Direct => DIRECT_LAYOUT,
        Mode::Repeat | Mode::Arp => ARP_LAYOUT,
        Mode::Pattern => PATTERN_LAYOUT,
    }
}

/// The arp panel's and the quick band's widgets and their meaning. It keeps
/// nothing but the layout it last used, so the app can rebuild it at any
/// time.
pub struct ArpPanel {
    /// The page name the panel's layouts are added under.
    pub name: String,
    /// The page name of the quick band.
    pub quick: String,
    layout: Option<&'static str>,
    /// The quick band's layout ([`QUICK_LAYOUT`] unless the app gave one).
    quick_layout: String,
    /// Changes every layout before it is added, for an app that puts a bar
    /// of its own on its pages (such as the time engine's transport bar).
    wrap: Option<fn(&str) -> String>,
}

impl ArpPanel {
    /// Adds the widgets to `ui` (the app adds `header`, `pads` and `hint`
    /// first), the panel's page `name` laid out for `st`, and the quick
    /// band's page `quick`.
    pub fn add(ui: &mut Ui, name: &str, quick: &str, st: &ArpState) -> Result<ArpPanel, String> {
        ui.add(MODES, Segments::new(&MODE_TABS));
        ui.add(LIST, ListView::new("Order · Wheel"));
        ui.add(QLINKS, SliderStack::new("Q-Links 1–4", &QLINK_LABELS));
        ui.add(ACTIONS, ButtonRow::new(&PATTERN_BUTTONS));
        ui.add(STEPS, StepStrip::new(8));
        ui.add(MINI, MiniBar::default());
        ui.add_page(quick, QUICK_LAYOUT)?;
        let mut panel = ArpPanel { name: name.to_string(), quick: quick.to_string(), layout: None,
                                   quick_layout: QUICK_LAYOUT.to_string(), wrap: None };
        panel.update(ui, st)?;
        Ok(panel)
    }

    fn spec(&self, layout: &str) -> String {
        self.wrap.map_or_else(|| layout.to_string(), |f| f(layout))
    }

    /// Lays both pages out again with `wrap` applied to every layout, now
    /// and whenever the panel's layout changes. The widgets `wrap` adds must
    /// be in `ui` already.
    pub fn set_layout_wrap(&mut self, ui: &mut Ui, wrap: fn(&str) -> String) -> Result<(), String> {
        self.wrap = Some(wrap);
        ui.add_page(&self.quick, &self.spec(&self.quick_layout))?;
        if let Some(layout) = self.layout {
            ui.add_page(&self.name, &self.spec(layout))?;
        }
        Ok(())
    }

    /// The quick band's own layout: an app puts [`MINI`] at the top of the
    /// column its Play page already has, so only that column's first widget
    /// gives up [`MINI_H`] pixels and nothing else moves.
    pub fn set_quick_layout(&mut self, ui: &mut Ui, layout: &str) -> Result<(), String> {
        ui.add_page(&self.quick, &self.spec(layout))?;
        self.quick_layout = layout.to_string();
        Ok(())
    }

    /// Shows `st` on both. The panel's layout follows the mode
    /// ([`layout_for`]).
    pub fn update(&mut self, ui: &mut Ui, st: &ArpState) -> Result<(), String> {
        let s = &st.settings;
        let layout = layout_for(s.mode);
        if self.layout != Some(layout) {
            ui.add_page(&self.name, &self.spec(layout))?;
            self.layout = Some(layout);
        }
        if let Some(m) = ui.get_mut::<MiniBar>(MINI) {
            let at = Rate::ALL.iter().position(|r| *r == s.rate).unwrap_or(0);
            m.mode = match s.mode {
                Mode::Direct => "Direct".to_string(),
                Mode::Repeat => "Repeat".to_string(),
                Mode::Arp => format!("Arp · {}", s.order.label()),
                Mode::Pattern => {
                    let name = st.patterns.get(st.pattern_index).map_or("", |(n, _)| n.as_str());
                    format!("Pattern · {}{}", short(name, 14), if st.pattern_changed { "*" } else { "" })
                }
            };
            m.rate = s.rate.label().to_string();
            m.latched = st.latched;
            m.can_slower = at > 0;
            m.can_faster = at + 1 < Rate::ALL.len();
        }
        if let Some(m) = ui.get_mut::<Segments>(MODES) {
            m.active = MODES_ALL.iter().position(|x| *x == s.mode).unwrap_or(0);
        }
        if let Some(list) = ui.get_mut::<ListView>(LIST) {
            match s.mode {
                Mode::Direct | Mode::Repeat => {
                    list.title = "Rate · Wheel".into();
                    list.items = Rate::ALL
                        .iter()
                        .map(|r| ListItem { text: r.label().into(), detail: rate_detail(*r).into(), color: None })
                        .collect();
                    list.selected = Rate::ALL.iter().position(|r| *r == s.rate).unwrap_or(0);
                }
                Mode::Arp => {
                    list.title = "Order · Wheel".into();
                    list.items = Order::ALL
                        .iter()
                        .map(|o| ListItem { text: o.label().into(), detail: order_preview(*o), color: None })
                        .collect();
                    list.selected = Order::ALL.iter().position(|o| *o == s.order).unwrap_or(0);
                }
                Mode::Pattern => {
                    list.title = "Patterns · Wheel".into();
                    list.items = st
                        .patterns
                        .iter()
                        .map(|(name, p)| ListItem { text: name.clone(), detail: short(&p.to_text(), 16), color: None })
                        .collect();
                    list.selected = st.pattern_index.min(st.patterns.len().saturating_sub(1));
                }
            }
        }
        if let Some(q) = ui.get_mut::<SliderStack>(QLINKS) {
            for (row, (v, text)) in q.rows.iter_mut().zip(qlink_values(s)) {
                row.value = v;
                row.text = text;
            }
        }
        if let Some(b) = ui.get_mut::<ButtonRow>(ACTIONS) {
            b.buttons[3] = Button { disabled: !st.pattern_changed, ..b.buttons[3].clone() };
        }
        let (cells, first, playing) = if s.mode == Mode::Pattern { pattern_cells(st) } else { arp_cells(st) };
        if let Some(strip) = ui.get_mut::<StepStrip>(STEPS) {
            strip.cells = cells;
            strip.first = first;
            strip.playing = playing;
            strip.min_cells = if s.mode == Mode::Pattern { 8 } else { STRIP_CELLS };
        }
        Ok(())
    }

    /// What a touch on one of the arp's widgets asks for; None for touches
    /// on other widgets or that change nothing. `st` is what they show.
    pub fn action(&self, ev: &UiEvent, st: &ArpState, qlink: &mut Option<usize>) -> Option<ArpAction> {
        let s = st.settings;
        match (ev.widget.as_str(), &ev.kind) {
            (MODES, EventKind::Tab(i)) => MODES_ALL.get(*i).map(|m| ArpAction::Mode(*m)),
            (LIST, EventKind::Select(i)) => match s.mode {
                Mode::Direct | Mode::Repeat => Rate::ALL.get(*i).map(|r| ArpAction::Rate(*r)),
                Mode::Arp => Order::ALL.get(*i).map(|o| ArpAction::Order(*o)),
                Mode::Pattern => (*i < st.patterns.len()).then_some(ArpAction::Pattern(*i)),
            },
            (QLINKS, EventKind::Select(i)) => {
                *qlink = Some(*i);
                None
            }
            (QLINKS, EventKind::Value(v)) => qlink.map(|i| ArpAction::Settings(with_qlink(s, i, *v))),
            (ACTIONS, EventKind::Tab(i)) => [PatternButton::Edit, PatternButton::Record, PatternButton::Mutate,
                PatternButton::Original]
                .get(*i)
                .map(|b| ArpAction::Button(*b)),
            (STEPS, EventKind::Select(i)) if s.mode == Mode::Pattern => Some(ArpAction::Step(*i)),
            (MINI, EventKind::Tab(i)) => {
                let at = Rate::ALL.iter().position(|r| *r == s.rate).unwrap_or(0);
                match *i {
                    MINI_SLOWER => at.checked_sub(1).and_then(|j| Rate::ALL.get(j)).map(|r| ArpAction::Rate(*r)),
                    MINI_FASTER => Rate::ALL.get(at + 1).map(|r| ArpAction::Rate(*r)),
                    MINI_LATCH => Some(ArpAction::Latch),
                    MINI_OPEN => Some(ArpAction::OpenPanel),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

fn rate_detail(r: Rate) -> &'static str {
    match r {
        Rate::Quarter => "beat",
        Rate::Eighth => "1/2 beat",
        Rate::EighthTriplet => "1/3 beat",
        Rate::Sixteenth => "1/4 beat",
        Rate::SixteenthTriplet => "1/6 beat",
        Rate::ThirtySecond => "1/8 beat",
        Rate::ThirtySecondTriplet => "1/12 beat",
    }
}

/// `text` cut to `max` characters with an ellipsis.
fn short(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut s: String = text.chars().take(max - 1).collect();
        s.push('…');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{built_in_patterns, samples};
    use mpfree_ui::{Fonts, SoftCanvas, Touch};

    fn state(settings: Settings, held: &[u8], step: u64) -> ArpState {
        ArpState {
            settings,
            pattern: Pattern::default(),
            patterns: built_in_patterns(),
            pattern_index: 0,
            pattern_changed: false,
            notes: held.iter().map(|&note| Held { note, velocity: 100 }).collect(),
            step,
            latched: true,
            bpm: 120.0,
        }
    }

    fn sample(name: &str) -> crate::sample::Sample {
        samples(Fonts::embedded).unwrap().into_iter().find(|s| s.name == name).unwrap()
    }

    #[test]
    fn an_app_can_wrap_the_layouts_with_a_bar_of_its_own() {
        fn with_bar(layout: &str) -> String {
            layout.replacen("col(header:60, ", "col(header:60, bar:44, ", 1)
        }
        let mut s = sample("arp-up-2-oct");
        s.ui.add("bar", mpfree_ui::widgets::Label::new("bar", Tone::Dim, mpfree_ui::Align::Left));
        let before = s.ui.rect_of(LIST).unwrap();
        s.page.set_layout_wrap(&mut s.ui, with_bar).unwrap();
        assert!(s.ui.rect_of("bar").is_some());
        assert!(s.ui.rect_of(LIST).unwrap().h < before.h);
        // A new mode's layout is wrapped too, and so is the quick band.
        let mut st = s.state.clone();
        st.settings.mode = Mode::Pattern;
        s.page.update(&mut s.ui, &st).unwrap();
        assert!(s.ui.rect_of("bar").is_some() && s.ui.rect_of(ACTIONS).is_some());
        s.ui.show(&s.page.quick.clone()).unwrap();
        assert!(s.ui.rect_of("bar").is_some() && s.ui.rect_of(MINI).is_some());
    }

    #[test]
    fn every_sample_screen_lays_out_and_draws() {
        for mut s in samples(Fonts::embedded).unwrap() {
            let mut canvas = SoftCanvas::new(1280, 800);
            s.ui.paint(&mut canvas);
            let mode = s.state.settings.mode;
            let mut want = vec!["header", "pads", "hint"];
            let mut gone = vec![];
            if s.quick {
                want.extend([MINI, STEPS]);
                gone.extend([MODES, LIST, QLINKS, ACTIONS]);
            } else {
                want.extend([MODES, LIST]);
                want.push(if mode == Mode::Pattern { ACTIONS } else { QLINKS });
                // The strip shows only while the arp is generating.
                if mode == Mode::Direct { gone.push(STEPS) } else { want.push(STEPS) }
                gone.push(MINI);
            }
            for name in want {
                let r = s.ui.rect_of(name).unwrap_or_else(|| panic!("{}: no {name}", s.name));
                assert!(r.w > 0 && r.h > 0 && r.right() <= 1280 && r.bottom() <= 800, "{}: {name} {r:?}", s.name);
            }
            for name in gone {
                assert!(s.ui.rect_of(name).is_none(), "{}: {name} should not show", s.name);
            }
            // The arp is not a tab: the header holds the app's tabs and the chip.
            let header = s.ui.get::<mpfree_ui::widgets::Header>("header").unwrap();
            assert!(!header.tabs.iter().any(|t| t == "Arp"), "{}", s.name);
            assert_eq!(header.chips.last().map(|c| c.text.clone()), Some(chip(&s.state).text), "{}", s.name);
        }
    }

    #[test]
    fn the_quick_band_is_a_slim_strip_with_the_rate_and_latch() {
        let mut s = sample("quick-hat-roll");
        let r = s.ui.rect_of(MINI).unwrap();
        assert_eq!(r.h, MINI_H);
        let bar = s.ui.get::<MiniBar>(MINI).unwrap().clone();
        assert_eq!((bar.mode.as_str(), bar.rate.as_str(), bar.latched), ("Repeat", "1/16", false));
        assert!(bar.can_slower && bar.can_faster);
        let parts = bar.parts(r);
        for (i, p) in parts.iter().enumerate() {
            assert!(p.w >= 44 && p.h >= 44, "part {i}: {p:?}");
            assert!(p.x >= r.x && p.right() <= r.right() && p.y >= r.y && p.bottom() <= r.bottom(), "part {i}: {p:?}");
            for (j, o) in parts.iter().enumerate().skip(i + 1) {
                assert!(p.intersect(*o).is_empty(), "parts {i} and {j} overlap: {p:?} {o:?}");
            }
        }
        let mut tap = |i: usize| {
            let p = parts[i];
            let ev = s.ui.touch(1, Touch::Down, p.x + 10, p.y + 10);
            s.ui.touch(1, Touch::Up, 0, 0);
            ev.and_then(|ev| s.page.action(&ev, &s.state, &mut None))
        };
        assert_eq!(tap(MINI_SLOWER), Some(ArpAction::Rate(Rate::EighthTriplet)));
        assert_eq!(tap(MINI_FASTER), Some(ArpAction::Rate(Rate::SixteenthTriplet)));
        assert_eq!(tap(MINI_LATCH), Some(ArpAction::Latch));
        assert_eq!(tap(MINI_OPEN), Some(ArpAction::OpenPanel));
        // At the ends, the arrow that would go past does nothing.
        let mut st = s.state.clone();
        st.settings.rate = Rate::ThirtySecondTriplet;
        st.latched = true;
        s.page.update(&mut s.ui, &st).unwrap();
        let bar = s.ui.get::<MiniBar>(MINI).unwrap();
        assert!(bar.can_slower && !bar.can_faster && bar.latched);
        assert_eq!(s.ui.touch(2, Touch::Down, parts[MINI_FASTER].x + 10, parts[MINI_FASTER].y + 10), None);
        s.ui.touch(2, Touch::Up, 0, 0);
        // A held hat repeats; the strip under it shows the pad.
        assert_eq!(s.ui.get::<StepStrip>(STEPS).unwrap().cells[0].label, "3");
    }

    #[test]
    fn the_chip_says_what_the_arp_does_and_whether_it_is_latched() {
        let mut st = state(Settings { mode: Mode::Arp, ..Settings::default() }, &[0, 2], 1);
        assert_eq!((chip(&st).text, chip(&st).tone), ("Arp: Up 1/16, latched".to_string(), Tone::Warn));
        st.latched = false;
        assert_eq!((chip(&st).text, chip(&st).tone), ("Arp: Up 1/16".to_string(), Tone::Accent));
        st.settings.mode = Mode::Repeat;
        st.settings.rate = Rate::ThirtySecondTriplet;
        assert_eq!(chip(&st).text, "Repeat 1/32T");
        st.settings.mode = Mode::Pattern;
        st.pattern_changed = true;
        assert_eq!(chip(&st).text, format!("Pattern: {}* 1/32T", st.patterns[0].0));
        st.settings.mode = Mode::Direct;
        assert_eq!((chip(&st).text, chip(&st).tone), ("Arp off".to_string(), Tone::Dim));
        st.latched = true;
        assert_eq!(chip(&st).text, "Latch on");
    }

    #[test]
    fn the_order_previews_match_the_engine_orders() {
        let got: Vec<String> = Order::ALL.iter().map(|o| order_preview(*o)).collect();
        assert_eq!(got, ["1 2 3", "3 2 1", "1 2 3 2", "1 2 3 3 2 1", "3 1 2", "2 1 3", "as pressed", "?", "?", "?",
                         "1+2+3"]);
    }

    #[test]
    fn arp_cells_follow_the_order_and_mark_the_step_playing() {
        let s = Settings { mode: Mode::Arp, octaves: 2, ..Settings::default() };
        let (cells, first, playing) = arp_cells(&state(s, &[4, 0, 2], 5));
        let labels: Vec<(&str, &str)> = cells.iter().map(|c| (c.label.as_str(), c.sub.as_str())).collect();
        assert_eq!(labels, [("1", ""), ("3", ""), ("5", ""), ("1", "oct +1"), ("3", "oct +1"), ("5", "oct +1")]);
        assert_eq!((first, playing), (0, Some(4)));
        // As played keeps the press order.
        let s = Settings { mode: Mode::Arp, order: Order::Played, ..Settings::default() };
        let (cells, _, playing) = arp_cells(&state(s, &[4, 0, 2], 0));
        assert_eq!(cells.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["5", "1", "3"]);
        assert_eq!(playing, None);
        // Nothing held, or Direct: an empty strip.
        assert!(arp_cells(&state(s, &[], 3)).0.is_empty());
        assert!(arp_cells(&state(Settings::default(), &[1], 3)).0.is_empty());
        // MIDI notes above the pads are named.
        assert_eq!(note_label(60), "C4");
    }

    #[test]
    fn long_orders_scroll_in_windows_of_sixteen() {
        let s = Settings { mode: Mode::Arp, order: Order::UpDownInclusive, octaves: 4, ..Settings::default() };
        // 3 notes x 4 octaves up and down = 24 counts; count 19 is playing.
        let (cells, first, playing) = arp_cells(&state(s, &[0, 1, 2], 20));
        assert_eq!((cells.len(), first, playing), (8, 16, Some(3)));
    }

    #[test]
    fn pattern_cells_show_picks_and_marks() {
        let p = Pattern::parse("1 x+! . _ *:3 ?%40 l- h").unwrap();
        let cells: Vec<StepCell> = p.steps().iter().map(|s| pattern_cell(*s)).collect();
        let text: Vec<(&str, &str, bool)> = cells.iter().map(|c| (c.label.as_str(), c.sub.as_str(), c.accent)).collect();
        assert_eq!(text, [("1", "", false), ("x", "oct +1", true), ("·", "", false), ("—", "", false),
                          ("all", "×3", false), ("?", "40%", false), ("low", "oct -1", false), ("high", "", false)]);
        let mut st = state(Settings { mode: Mode::Pattern, ..Settings::default() }, &[0], 11);
        st.pattern = p;
        assert_eq!(pattern_cells(&st).2, Some(2));
    }

    #[test]
    fn qlinks_cover_every_value_and_round_trip() {
        let base = Settings::default();
        for (k, r) in Rate::ALL.iter().enumerate() {
            let s = with_qlink(base, 0, k as f32 / 6.0);
            assert_eq!(s.rate, *r);
            assert_eq!(with_qlink(base, 0, qlink_values(&s)[0].0).rate, *r);
        }
        for g in 1..=100u8 {
            let s = Settings { gate: g, ..base };
            assert_eq!(with_qlink(base, 1, qlink_values(&s)[1].0).gate, g);
        }
        for w in 50..=75u8 {
            let s = Settings { swing: w, ..base };
            assert_eq!(with_qlink(base, 2, qlink_values(&s)[2].0).swing, w);
        }
        for o in 1..=4u8 {
            let s = Settings { octaves: o, ..base };
            assert_eq!(with_qlink(base, 3, qlink_values(&s)[3].0).octaves, o);
        }
        assert_eq!(with_qlink(base, 1, 9.0).gate, 100);
        assert_eq!(qlink_values(&base)[2].1, "off");
    }

    #[test]
    fn taps_become_actions() {
        let mut s = sample("arp-up-2-oct");
        let mut qlink = None;
        let tap = |ui: &mut Ui, name: &str, fx: f32, fy: f32| {
            let r = ui.rect_of(name).unwrap();
            let (x, y) = (r.x + (r.w as f32 * fx) as i32, r.y + (r.h as f32 * fy) as i32);
            let ev = ui.touch(1, Touch::Down, x, y);
            ui.touch(1, Touch::Up, x, y);
            ev
        };
        // The Pattern tab (the last of four).
        let ev = tap(&mut s.ui, MODES, 0.9, 0.5).unwrap();
        assert_eq!(s.page.action(&ev, &s.state, &mut qlink), Some(ArpAction::Mode(Mode::Pattern)));
        // The first row of the order list is Up.
        let ev = tap(&mut s.ui, LIST, 0.5, 0.25).unwrap();
        assert_eq!(s.page.action(&ev, &s.state, &mut qlink), Some(ArpAction::Order(Order::Up)));
        // A finger on the gate slider, then moved to its right end.
        let r = s.ui.rect_of(QLINKS).unwrap();
        let (_, track) = s.ui.get::<SliderStack>(QLINKS).unwrap().row_rects(r, 1);
        let ev = s.ui.touch(2, Touch::Down, track.x + 5, track.y).unwrap();
        assert_eq!(s.page.action(&ev, &s.state, &mut qlink), None);
        assert_eq!(qlink, Some(1));
        let ev = s.ui.touch(2, Touch::Move, track.right() + 50, track.y).unwrap();
        let Some(ArpAction::Settings(set)) = s.page.action(&ev, &s.state, &mut qlink) else { panic!("{ev:?}") };
        assert_eq!(set.gate, 100);
        s.ui.touch(2, Touch::Up, 0, 0);
    }

    #[test]
    fn pattern_mode_lays_out_the_buttons_and_original_waits_for_a_change() {
        let mut st = state(Settings { mode: Mode::Pattern, ..Settings::default() }, &[0, 1], 2);
        st.pattern = Pattern::parse("1 2").unwrap();
        let mut ui = Ui::new(mpfree_ui::Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", mpfree_ui::widgets::Header::new(&["Arp"]));
        ui.add("pads", mpfree_ui::widgets::PadGrid::new(16, 4));
        ui.add("hint", mpfree_ui::widgets::Label::new("", Tone::Dim, mpfree_ui::Align::Center));
        let mut page = ArpPanel::add(&mut ui, "Arp", "Arp quick", &st).unwrap();
        ui.show("Arp").unwrap();
        assert!(ui.rect_of(ACTIONS).is_some() && ui.rect_of(QLINKS).is_none());
        let b = ui.get::<ButtonRow>(ACTIONS).unwrap();
        assert!(b.buttons[3].disabled);
        let r = ui.rect_of(ACTIONS).unwrap();
        let orig = b.button_rect(r, 3);
        assert_eq!(ui.touch(1, Touch::Down, orig.x + 5, orig.y + 5), None);
        ui.touch(1, Touch::Up, 0, 0);
        st.pattern_changed = true;
        page.update(&mut ui, &st).unwrap();
        let ev = ui.touch(1, Touch::Down, orig.x + 5, orig.y + 5).unwrap();
        assert_eq!(page.action(&ev, &st, &mut None), Some(ArpAction::Button(PatternButton::Original)));
        ui.touch(1, Touch::Up, 0, 0);
        // Back to Arp: the Q-Links come back.
        st.settings.mode = Mode::Arp;
        page.update(&mut ui, &st).unwrap();
        assert!(ui.rect_of(QLINKS).is_some() && ui.rect_of(ACTIONS).is_none());
        // Direct: the strip goes, and the list takes its room.
        let pads = ui.rect_of("pads").unwrap();
        st.settings.mode = Mode::Direct;
        page.update(&mut ui, &st).unwrap();
        assert!(ui.rect_of(STEPS).is_none() && ui.rect_of(QLINKS).is_some());
        assert_eq!(ui.rect_of("pads").unwrap(), pads);
    }

    #[test]
    fn the_pads_never_move_between_play_the_band_and_the_panel() {
        let st = state(Settings { mode: Mode::Arp, ..Settings::default() }, &[0, 4, 7], 3);
        let mut ui = Ui::new(mpfree_ui::Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", mpfree_ui::widgets::Header::new(&["Play"]));
        ui.add("pads", mpfree_ui::widgets::PadGrid::new(16, 4));
        ui.add("kits", mpfree_ui::widgets::Label::new("", Tone::Dim, mpfree_ui::Align::Center));
        ui.add("hint", mpfree_ui::widgets::Label::new("", Tone::Dim, mpfree_ui::Align::Center));
        // The pad player's Play page: pads, and a column of its own beside them.
        ui.add_page("Play", "col(header:60, row(pads:*, col(kits:*):460):*, hint:26)").unwrap();
        let mut page = ArpPanel::add(&mut ui, "Arp", "Arp quick", &st).unwrap();
        page.set_quick_layout(&mut ui, "col(header:60, row(pads:*, col(arp-mini:56, kits:*):460):*, hint:26)")
            .unwrap();
        ui.show("Play").unwrap();
        let pads = ui.rect_of("pads").unwrap();
        let kits = ui.rect_of("kits").unwrap();
        assert_eq!(kits.w, SIDE);
        ui.show("Arp quick").unwrap();
        assert_eq!(ui.rect_of("pads").unwrap(), pads, "the quick band");
        let (mini, under) = (ui.rect_of(MINI).unwrap(), ui.rect_of("kits").unwrap());
        assert_eq!((mini.x, mini.w, mini.y), (kits.x, kits.w, kits.y), "the strip sits at the top of the column");
        assert_eq!(under.bottom(), kits.bottom(), "the app's column keeps all but the strip's height");
        ui.show("Arp").unwrap();
        for mode in MODES_ALL {
            page.update(&mut ui, &state(Settings { mode, ..Settings::default() }, &[0, 4, 7], 3)).unwrap();
            assert_eq!(ui.rect_of("pads").unwrap(), pads, "the panel in {mode:?}");
        }
    }
}
