//! The Arp page: what it shows, worked out from the engine, and what its
//! touches mean.
//!
//! The page does not own the engine. The app copies what the page needs into
//! an [`ArpState`] (from [`ArpState::from_engine`]), calls [`ArpPage::update`]
//! when it changes, and turns the page's [`ArpAction`]s into engine calls.
//! The pads, the header and the hint line belong to the app; the page only
//! adds its own widgets and the layouts that place them.

use crate::widgets::{Button, ButtonRow, CellKind, Segments, SliderStack, StepCell, StepStrip};
use mpfree_arp::{Engine, Held, Mode, Order, Pattern, Pick, Rate, Settings, Step, MAX_NOTES};
use mpfree_ui::widgets::{Chip, ListItem, ListView};
use mpfree_ui::{EventKind, Tone, Ui, UiEvent};

/// Widget names the page adds. The app's own are `header`, `pads` and `hint`.
pub const MODES: &str = "arp-modes";
pub const LIST: &str = "arp-list";
pub const QLINKS: &str = "arp-qlinks";
pub const ACTIONS: &str = "arp-actions";
pub const STEPS: &str = "arp-steps";

/// Direct, Repeat and Arp: the list (rates or orders) over the Q-Link sliders.
pub const ARP_LAYOUT: &str =
    "col(header:60, row(pads:*, col(arp-modes:52, arp-list:*, arp-qlinks:236):460):*, arp-steps:64, hint:26)";
/// Pattern: the pattern list over the pattern buttons.
pub const PATTERN_LAYOUT: &str =
    "col(header:60, row(pads:*, col(arp-modes:52, arp-list:*, arp-actions:128):460):*, arp-steps:64, hint:26)";

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

/// The arp's header chips: latch, tempo, and what the generator does.
pub fn chips(st: &ArpState) -> Vec<Chip> {
    let s = &st.settings;
    let what = match s.mode {
        Mode::Direct => "Direct".to_string(),
        Mode::Repeat => format!("Repeat · {}", s.rate.label()),
        Mode::Arp => format!("Arp · {} · {} oct", s.order.label(), s.octaves),
        Mode::Pattern => {
            let name = st.patterns.get(st.pattern_index).map_or("pattern", |(n, _)| n.as_str());
            format!("Pattern · {name}{}", if st.pattern_changed { " *" } else { "" })
        }
    };
    // The header lays chips out from the right edge, so the last one here
    // is the leftmost: latch, tempo, then what the generator does.
    vec![
        Chip::new(what, Tone::Dim),
        Chip::new(format!("{:.1} BPM", st.bpm), Tone::Good),
        Chip::new(if st.latched { "LATCH" } else { "latch off" }, if st.latched { Tone::Warn } else { Tone::Dim }),
    ]
}

/// The hint line for each mode.
pub fn hint(mode: Mode) -> &'static str {
    match mode {
        Mode::Direct => "Pads play as pressed · SHIFT+NOTE REPEAT: latch · tap a mode to start the arp",
        Mode::Repeat => "Hold or latch pads · Wheel: rate · Q-Links: rate, gate, swing, octaves",
        Mode::Arp => "Hold or latch pads · Wheel: order · Q-Links: rate, gate, swing, octaves · TAP TEMPO",
        Mode::Pattern => "Patterns play the chord you hold: \"2\" is the 2nd lowest held pad · Wheel: pattern",
    }
}

/// The Arp page's widgets and their meaning. It keeps nothing but which mode
/// it last laid out, so the app can rebuild it at any time.
pub struct ArpPage {
    /// The page name the layouts are added under.
    pub name: String,
    mode: Option<Mode>,
}

impl ArpPage {
    /// Adds the page's widgets to `ui` (the app adds `header`, `pads` and
    /// `hint` first) and its page, laid out for `st`.
    pub fn add(ui: &mut Ui, name: &str, st: &ArpState) -> Result<ArpPage, String> {
        ui.add(MODES, Segments::new(&MODE_TABS));
        ui.add(LIST, ListView::new("Order · Wheel"));
        ui.add(QLINKS, SliderStack::new("Q-Links 1–4", &QLINK_LABELS));
        ui.add(ACTIONS, ButtonRow::new(&PATTERN_BUTTONS));
        ui.add(STEPS, StepStrip::new(8));
        let mut page = ArpPage { name: name.to_string(), mode: None };
        page.update(ui, st)?;
        Ok(page)
    }

    /// Shows `st`. The layout changes when the mode moves to or from Pattern.
    pub fn update(&mut self, ui: &mut Ui, st: &ArpState) -> Result<(), String> {
        let s = &st.settings;
        if self.mode != Some(s.mode) {
            let was_pattern = self.mode == Some(Mode::Pattern);
            if self.mode.is_none() || was_pattern != (s.mode == Mode::Pattern) {
                let layout = if s.mode == Mode::Pattern { PATTERN_LAYOUT } else { ARP_LAYOUT };
                ui.add_page(&self.name, layout)?;
            }
            self.mode = Some(s.mode);
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

    /// What a touch on one of the page's widgets asks for; None for touches
    /// on other widgets or that change nothing. `st` is what the page shows.
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

    #[test]
    fn every_sample_screen_lays_out_and_draws() {
        for mut s in samples(Fonts::embedded).unwrap() {
            let mut canvas = SoftCanvas::new(1280, 800);
            s.ui.paint(&mut canvas);
            for name in ["header", "pads", MODES, LIST, STEPS, "hint"] {
                let r = s.ui.rect_of(name).unwrap_or_else(|| panic!("{}: no {name}", s.name));
                assert!(r.w > 0 && r.h > 0 && r.right() <= 1280 && r.bottom() <= 800, "{}: {name} {r:?}", s.name);
            }
            let side = if s.state.settings.mode == Mode::Pattern { ACTIONS } else { QLINKS };
            assert!(s.ui.rect_of(side).is_some(), "{}: no {side}", s.name);
        }
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
        let mut s = samples(Fonts::embedded).unwrap().remove(0);
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
        let mut page = ArpPage::add(&mut ui, "Arp", &st).unwrap();
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
    }
}
