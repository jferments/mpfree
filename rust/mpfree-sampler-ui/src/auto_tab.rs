//! The Rec tab's Auto-sample mode: play the Synth tab's patch note by
//! note, record each note and make a keygroup instrument of it, as the
//! MPC's Auto Sampler (the keyboard on its Sampler page), Elektron's
//! Tonverk and Logic's Auto Sampler do.
//!
//! - The **picture** shows the keyboard with the notes it samples and the
//!   keys each will play (halfway to its neighbours; the lowest and
//!   highest stretch to the ends), the velocity layers on a 1 to 127 axis,
//!   and, while it runs, the note playing and how far it has got.
//! - The **Auto-sample** list holds the plan: the source (the Synth tab's
//!   patch, by its engine and preset), lowest and highest note, every how many
//!   semitones, velocity layers and how they spread, round robins, how
//!   long each note is held and may ring, loops, channels and normalize.
//!   The bar says what it will make and its cost before anything starts.
//!   An instrument has a pad a note, so at most 16 notes (a kit's pads);
//!   it goes onto the first run of empty pads, or over the whole kit.
//! - **Start** has the app run it ([`mpfree_sampler::autosample::Job`]):
//!   offline, many times faster than real time, on a worker thread. The
//!   first button then cancels it, keeping the notes done. The instrument
//!   lands as one undo step, and the kit's pads then play notes.
//! - **Rec** goes back to recording.
//!
//! External gear (MIDI out to it, its sound into the line input) is a
//! later source, once the MPC's MIDI out is proven.

use mpfree_analysis::note_name;
use mpfree_sampler::autosample::{zones, Plan, VelocityCurve, MAX_ROUND_ROBINS};
use mpfree_sampler::model::{LoopMode, MAX_LAYERS, PADS};
use mpfree_ui::{EventKind, Ui, UiEvent};

use crate::auto_widgets::{KeyMapView, LayerBand};
use crate::chop_tab::pad_text;
use crate::rec_tab;
use crate::wave_tab::{self, pad_color, TABS};
use crate::widgets::{ButtonRow, InfoCard, PadBar, Setting, SettingList};

/// Widget names the tab adds. The app's own are `header` and `hint`.
pub const BAR: &str = "smp-auto-bar";
pub const VIEW: &str = "smp-auto-view";
pub const LIST: &str = "smp-auto-list";
pub const BUTTONS: &str = "smp-auto-buttons";
pub const WHEEL: &str = "smp-auto-wheel";

/// The page: the keyboard beside the list, the buttons under them, as the
/// Rec tab's.
pub const LAYOUT: &str = "col(header:60, smp-auto-bar:60, \
                          row(smp-auto-view:*, smp-auto-list:340):*, \
                          row(smp-auto-buttons:*, smp-auto-wheel:340):88, hint:26)";

/// The hint line the app shows under the page.
pub const HINT: &str =
    "Auto-sample plays the synth note by note and makes a keygroup of it · tap a row, then the \
                        wheel changes it · UNDO takes the instrument off";

const HOLDS_MS: [u32; 9] = [100, 250, 500, 1000, 2000, 3000, 5000, 10_000, 20_000];
const TAILS_MS: [u32; 8] = [0, 500, 1000, 2000, 4000, 8000, 12_000, 20_000];
const LOOPS: [Option<LoopMode>; 3] = [None, Some(LoopMode::Forward), Some(LoopMode::PingPong)];
const CURVES: [VelocityCurve; 3] = [
    VelocityCurve::Linear,
    VelocityCurve::Exp,
    VelocityCurve::Log,
];

/// Where the instrument goes on the kit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Onto {
    /// The first run of empty pads long enough.
    EmptyPads,
    /// From the kit's first pad, replacing what is there (one undo puts
    /// it back).
    WholeKit,
}

/// The plan as the list sets it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoSettings {
    pub lo: u8,
    pub hi: u8,
    pub every: u8,
    pub layers: u8,
    pub curve: VelocityCurve,
    pub round_robins: u8,
    pub hold_ms: u32,
    pub tail_ms: u32,
    pub loops: Option<LoopMode>,
    pub mono: bool,
    pub normalize: bool,
    pub onto: Onto,
}

impl Default for AutoSettings {
    /// The plan's defaults, from C2 to A5 so its 16 notes fill a kit's 16
    /// pads: every 3 semitones, 3 layers (40, 90 and 127), held 2 s with
    /// up to 4 s of tail, stereo, normalized, onto empty pads.
    fn default() -> Self {
        let p = Plan::default();
        AutoSettings {
            lo: p.lo,
            hi: 81,
            every: p.every,
            layers: p.velocities.len() as u8,
            curve: VelocityCurve::Linear,
            round_robins: p.round_robins,
            hold_ms: (p.hold * 1000.0) as u32,
            tail_ms: (p.tail * 1000.0) as u32,
            loops: p.loops,
            mono: p.channels == 1,
            normalize: p.normalize,
            onto: Onto::EmptyPads,
        }
    }
}

impl AutoSettings {
    /// The velocity each layer is played at: 40, 90 and 127 for three
    /// even layers (the MPC's spread), else along the curve.
    pub fn velocities(&self) -> Vec<u8> {
        if self.layers == 3 && self.curve == VelocityCurve::Linear {
            Plan::default().velocities
        } else {
            Plan::spread_velocities(usize::from(self.layers), self.curve)
        }
    }

    /// The plan for an instrument called `name`, at `rate`.
    pub fn plan(&self, name: &str, rate: u32) -> Plan {
        Plan {
            name: name.to_string(),
            lo: self.lo,
            hi: self.hi,
            every: self.every,
            velocities: self.velocities(),
            round_robins: self.round_robins,
            hold: self.hold_ms as f32 / 1000.0,
            tail: self.tail_ms as f32 / 1000.0,
            loops: self.loops,
            channels: if self.mono { 1 } else { 2 },
            normalize: self.normalize,
            rate,
            ..Plan::default()
        }
    }

    /// Notes sampled: one pad each.
    pub fn notes(&self) -> usize {
        self.plan("", 44_100).notes().len()
    }
}

/// The list's rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Source,
    Lowest,
    Highest,
    Every,
    Layers,
    Curve,
    RoundRobins,
    Hold,
    Tail,
    Loop,
    Channels,
    Normalize,
    Onto,
}

impl Row {
    pub const ALL: [Row; 13] = [
        Row::Source,
        Row::Lowest,
        Row::Highest,
        Row::Every,
        Row::Layers,
        Row::Curve,
        Row::RoundRobins,
        Row::Hold,
        Row::Tail,
        Row::Loop,
        Row::Channels,
        Row::Normalize,
        Row::Onto,
    ];

    pub fn index(self) -> usize {
        Row::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }

    pub fn label(self) -> &'static str {
        match self {
            Row::Source => "Source",
            Row::Lowest => "Lowest note",
            Row::Highest => "Highest note",
            Row::Every => "Every",
            Row::Layers => "Layers",
            Row::Curve => "Layer spread",
            Row::RoundRobins => "Round robins",
            Row::Hold => "Hold",
            Row::Tail => "Tail",
            Row::Loop => "Loop",
            Row::Channels => "Channels",
            Row::Normalize => "Normalize",
            Row::Onto => "Onto",
        }
    }
}

/// Seconds as "48 s" or "3:22".
fn minutes_text(seconds: f32) -> String {
    let s = seconds.round() as u64;
    if s < 60 {
        format!("{s} s")
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn seconds_text(ms: u32) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms.is_multiple_of(1000) {
        format!("{} s", ms / 1000)
    } else {
        format!("{:.1} s", ms as f32 / 1000.0)
    }
}

/// A row's value as the list shows it. `synth`: the source's name.
pub fn value_text(s: &AutoSettings, row: Row, synth: Option<&str>) -> String {
    match row {
        Row::Source => synth.map_or_else(|| "no synth".to_string(), str::to_string),
        Row::Lowest => note_name(s.lo),
        Row::Highest => note_name(s.hi),
        Row::Every => match s.every {
            1 => "1 semitone".to_string(),
            12 => "12 semitones (octave)".to_string(),
            n => format!("{n} semitones"),
        },
        Row::Layers => {
            let v: Vec<String> = s.velocities().iter().map(u8::to_string).collect();
            format!("{} · {}", s.layers, v.join(" "))
        }
        Row::Curve if s.layers == 1 => "–".to_string(),
        Row::Curve => match s.curve {
            VelocityCurve::Linear => "Even",
            VelocityCurve::Exp => "More soft",
            VelocityCurve::Log => "More hard",
        }
        .to_string(),
        Row::RoundRobins => s.round_robins.to_string(),
        Row::Hold => seconds_text(s.hold_ms),
        Row::Tail if s.tail_ms == 0 => "none".to_string(),
        Row::Tail => format!("up to {}", seconds_text(s.tail_ms)),
        Row::Loop => match s.loops {
            None => "Off",
            Some(LoopMode::Forward) => "Forward",
            Some(LoopMode::PingPong) => "Ping-pong",
        }
        .to_string(),
        Row::Channels => if s.mono { "Mono" } else { "Stereo" }.to_string(),
        Row::Normalize => if s.normalize { "On" } else { "Off" }.to_string(),
        Row::Onto => match s.onto {
            Onto::EmptyPads => "Empty pads",
            Onto::WholeKit => "Whole kit (replacing)",
        }
        .to_string(),
    }
}

fn step_in<T: PartialEq + Copy>(list: &[T], now: T, delta: i32) -> T {
    let i = list.iter().position(|x| *x == now).unwrap_or(0) as i32;
    list[(i + delta).clamp(0, list.len() as i32 - 1) as usize]
}

fn step_u8(now: u8, delta: i32, lo: u8, hi: u8) -> u8 {
    (i32::from(now) + delta).clamp(i32::from(lo), i32::from(hi)) as u8
}

/// The settings with `row` stepped `delta` places. Lowest and highest
/// push each other; more layers take round robins away past 16 a pad.
pub fn stepped(s: &AutoSettings, row: Row, delta: i32) -> AutoSettings {
    let mut s = *s;
    match row {
        Row::Source => {}
        Row::Lowest => {
            s.lo = step_u8(s.lo, delta, 0, 127);
            s.hi = s.hi.max(s.lo);
        }
        Row::Highest => {
            s.hi = step_u8(s.hi, delta, 0, 127);
            s.lo = s.lo.min(s.hi);
        }
        Row::Every => s.every = step_u8(s.every, delta, 1, 12),
        Row::Layers => {
            s.layers = step_u8(s.layers, delta, 1, 8);
            let room = (MAX_LAYERS / usize::from(s.layers)) as u8;
            s.round_robins = s.round_robins.min(room);
        }
        Row::Curve => s.curve = step_in(&CURVES, s.curve, delta),
        Row::RoundRobins => {
            let room = (MAX_LAYERS / usize::from(s.layers)) as u8;
            s.round_robins = step_u8(s.round_robins, delta, 1, MAX_ROUND_ROBINS.min(room));
        }
        Row::Hold => s.hold_ms = step_in(&HOLDS_MS, s.hold_ms, delta),
        Row::Tail => s.tail_ms = step_in(&TAILS_MS, s.tail_ms, delta),
        Row::Loop => s.loops = step_in(&LOOPS, s.loops, delta),
        Row::Channels => s.mono = delta > 0,
        Row::Normalize => s.normalize = delta < 0,
        Row::Onto => {
            s.onto = if delta > 0 {
                Onto::WholeKit
            } else {
                Onto::EmptyPads
            }
        }
    }
    s
}

/// How far a running auto-sample has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Running {
    pub done: usize,
    pub total: usize,
    /// The note playing now.
    pub note: Option<u8>,
}

/// What the page shows besides the settings.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoState {
    /// The pad picked in the editor.
    pub pad: usize,
    /// The Synth tab's patch, by its engine and preset ("Poly Pluck");
    /// None when there is none to sample.
    pub synth: Option<String>,
    /// Pads the kit has (16): an instrument has at most one a note. 0
    /// while the pads play the synth (no kit to put it on).
    pub pads: usize,
    /// The first of the pads the instrument would go onto (None: no run
    /// of empty pads long enough, or more notes than pads).
    pub target: Option<usize>,
    pub running: Option<Running>,
    /// The last instrument made, or why not, in a line.
    pub last: String,
    pub rate: u32,
}

/// What a touch or control on the page asks for. The app does it.
#[derive(Clone, Debug, PartialEq)]
pub enum AutoAction {
    /// Start with this plan (the app names it after the synth).
    Start(Plan),
    /// Stop, keeping the notes done.
    Cancel,
    /// Back to the Rec tab's recording.
    Back,
    /// Open the editor's tab `i` of [`TABS`].
    Tab(usize),
}

/// Where an instrument of `n` pads goes on a kit of `pads` pads: the
/// first run of `n` empty pads, or the kit's first pad for
/// [`Onto::WholeKit`]. None when it can't fit.
pub fn target(
    onto: Onto,
    n: usize,
    pads: usize,
    has_sound: impl Fn(usize) -> bool,
) -> Option<usize> {
    if n == 0 || n > pads.min(PADS) {
        return None;
    }
    match onto {
        Onto::WholeKit => Some(0),
        Onto::EmptyPads => (0..=pads - n).find(|&s| (s..s + n).all(|p| !has_sound(p))),
    }
}

/// Why the instrument has nowhere to go, as the bar's title and the
/// picture's line.
fn no_room(s: &AutoSettings, st: &AutoState) -> (String, String) {
    let n = s.notes();
    if st.pads == 0 {
        (
            "Pick a kit on Play to put it on".to_string(),
            "The pads play the synth now: there is no kit to put the instrument on".to_string(),
        )
    } else if n > st.pads {
        (
            format!("{n} notes need {n} pads; a kit has {}", st.pads),
            "Sample fewer notes: a higher lowest note, a lower highest, or a wider Every"
                .to_string(),
        )
    } else {
        (
            format!("No {n} empty pads in a row on this kit"),
            "Onto: Whole kit replaces the kit's pads (UNDO puts them back), or sample fewer notes"
                .to_string(),
        )
    }
}

/// The page's widgets and settings.
pub struct AutoTab {
    pub name: String,
    pub bank: Option<char>,
    pub settings: AutoSettings,
    synth: Option<String>,
    running: bool,
}

impl AutoTab {
    /// Adds the widgets and the page `name` to `ui` (the app adds `header`
    /// and `hint` first) and shows `st`.
    pub fn add(ui: &mut Ui, name: &str, st: &AutoState) -> Result<AutoTab, String> {
        let mut bar = PadBar::new(&TABS);
        bar.enabled = TABS.iter().map(|t| wave_tab::BUILT.contains(t)).collect();
        bar.active = rec_tab::TAB;
        ui.add(BAR, bar);
        ui.add(VIEW, KeyMapView::new("Auto-sample"));
        let mut list = SettingList::new("Auto-sample");
        list.selected = Row::Lowest.index();
        ui.add(LIST, list);
        let mut buttons = ButtonRow::new(&["Start", "Rec"]);
        buttons.primary = Some(0);
        ui.add(BUTTONS, buttons);
        ui.add(WHEEL, InfoCard::new("Wheel · the row picked"));
        ui.add_page(name, LAYOUT)?;
        let mut tab = AutoTab {
            name: name.to_string(),
            bank: None,
            settings: AutoSettings::default(),
            synth: st.synth.clone(),
            running: false,
        };
        tab.update(ui, st);
        Ok(tab)
    }

    pub fn row(&self, ui: &Ui) -> Row {
        ui.get::<SettingList>(LIST)
            .and_then(|l| Row::ALL.get(l.selected).copied())
            .unwrap_or(Row::Lowest)
    }

    /// The pads the instrument would take, as "A1–B1".
    fn pads_text(&self, st: &AutoState) -> Option<String> {
        let first = st.target?;
        let n = self.settings.notes();
        Some(if n == 1 {
            pad_text(self.bank, first)
        } else {
            format!(
                "{}–{}",
                pad_text(self.bank, first),
                pad_text(self.bank, first + n - 1)
            )
        })
    }

    /// Shows `st` and the settings.
    pub fn update(&mut self, ui: &mut Ui, st: &AutoState) {
        self.synth = st.synth.clone();
        self.running = st.running.is_some();
        let s = self.settings;
        let plan = s.plan(st.synth.as_deref().unwrap_or("Auto"), st.rate);
        let est = plan.estimate();
        let fault = plan.check().err();
        let pads = self.pads_text(st);
        if let Some(bar) = ui.get_mut::<PadBar>(BAR) {
            bar.color = pad_color(st.target.unwrap_or(st.pad));
            bar.active = rec_tab::TAB;
            bar.title = match (&st.synth, &pads) {
                (None, _) => "Auto-sample: the synth isn't up yet".to_string(),
                (Some(n), Some(p)) => format!("Auto-sample {n} onto pads {p}"),
                (Some(_), None) => no_room(&s, st).0,
            };
            bar.subtitle = if !st.last.is_empty() && st.running.is_none() {
                st.last.clone()
            } else {
                format!(
                    "{} notes × {} layer{}{} = {} takes · {} long · up to {:.0} MB",
                    plan.notes().len(),
                    s.layers,
                    if s.layers == 1 { "" } else { "s" },
                    if s.round_robins > 1 {
                        format!(" × {} round robins", s.round_robins)
                    } else {
                        String::new()
                    },
                    est.takes,
                    minutes_text(est.seconds),
                    est.bytes_most as f64 / 1e6
                )
            };
        }
        let rows = Row::ALL
            .iter()
            .map(|r| Setting {
                name: r.label().into(),
                value: value_text(&s, *r, st.synth.as_deref()),
            })
            .collect();
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.rows = rows;
            list.right = pads.clone().unwrap_or_default();
        }
        let ranges = plan.velocity_ranges();
        let notes = plan.notes();
        if let Some(v) = ui.get_mut::<KeyMapView>(VIEW) {
            v.title = st
                .synth
                .clone()
                .unwrap_or_else(|| "Auto-sample".to_string());
            v.right = format!(
                "{} notes, {}",
                notes.len(),
                value_text(&s, Row::Every, None)
            );
            v.show_range(s.lo, s.hi);
            v.zones = zones(&notes);
            v.layers = ranges
                .iter()
                .zip(&plan.velocities)
                .map(|(&(lo, hi), &played)| LayerBand { lo, hi, played })
                .collect();
            v.round_robins = s.round_robins;
            let per_note = s.layers as usize * s.round_robins as usize;
            match st.running {
                Some(r) => {
                    v.progress = Some((r.done, r.total));
                    v.current = r.note;
                    v.done = (0..notes.len())
                        .map(|k| r.done >= (k + 1) * per_note)
                        .collect();
                    v.caption = format!(
                        "take {} of {}{}",
                        (r.done + 1).min(r.total),
                        r.total,
                        r.note
                            .map(|n| format!(" · {}", note_name(n)))
                            .unwrap_or_default()
                    );
                }
                None => {
                    v.progress = None;
                    v.current = None;
                    v.done.clear();
                    v.caption =
                        "each pad plays its note's keys · the dot is the note sampled".to_string();
                }
            }
            v.note = match (&st.synth, &fault) {
                (None, _) => "The Synth tab's patch shows here once the synth is up".to_string(),
                (_, Some(e)) => e.to_string(),
                _ if st.target.is_none() => no_room(&s, st).1,
                _ => String::new(),
            };
        }
        let can = st.synth.is_some() && st.target.is_some() && fault.is_none();
        if let Some(b) = ui.get_mut::<ButtonRow>(BUTTONS) {
            b.labels = vec![
                if self.running { "Cancel" } else { "Start" }.to_string(),
                "Rec".to_string(),
            ];
            b.enabled = vec![self.running || can, !self.running];
        }
        self.show_wheel(ui);
    }

    fn show_wheel(&self, ui: &mut Ui) {
        let row = self.row(ui);
        let text = match row {
            Row::Source => "Source: the Synth tab's patch (external gear comes later)".to_string(),
            _ => format!(
                "{}: {}",
                row.label(),
                value_text(&self.settings, row, self.synth.as_deref())
            ),
        };
        if let Some(card) = ui.get_mut::<InfoCard>(WHEEL) {
            card.text = text;
        }
    }

    /// What a touch on one of the page's widgets asks for.
    pub fn action(&mut self, ui: &mut Ui, ev: &UiEvent, st: &AutoState) -> Option<AutoAction> {
        match (ev.widget.as_str(), &ev.kind) {
            (BAR, EventKind::Tab(i)) => Some(AutoAction::Tab(*i)),
            (BUTTONS, EventKind::Tab(0)) if self.running => Some(AutoAction::Cancel),
            (BUTTONS, EventKind::Tab(0)) => {
                let name = st.synth.clone()?;
                st.target?;
                let plan = self.settings.plan(&name, st.rate);
                plan.check().ok()?;
                Some(AutoAction::Start(plan))
            }
            (BUTTONS, EventKind::Tab(1)) if !self.running => Some(AutoAction::Back),
            (LIST, EventKind::Step { index, delta }) => {
                let row = *Row::ALL.get(*index)?;
                self.step(ui, st, row, *delta);
                None
            }
            (LIST, EventKind::Select(_)) => {
                self.show_wheel(ui);
                None
            }
            _ => None,
        }
    }

    /// The data wheel: steps the row picked. The settings wait while it
    /// runs.
    pub fn wheel(&mut self, ui: &mut Ui, st: &AutoState, delta: i32) {
        let row = self.row(ui);
        self.step(ui, st, row, delta);
    }

    fn step(&mut self, ui: &mut Ui, st: &AutoState, row: Row, delta: i32) {
        if self.running {
            if let Some(card) = ui.get_mut::<InfoCard>(WHEEL) {
                card.text = "Cancel first to change the plan".to_string();
            }
            return;
        }
        self.settings = stepped(&self.settings, row, delta);
        self.update(ui, st);
    }

    pub fn running(&self) -> bool {
        self.running
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_sampler::autosample::PlanError;

    #[test]
    fn the_settings_make_the_plan_and_keep_it_possible() {
        let s = AutoSettings::default();
        let p = s.plan("Poly Pluck", 44_100);
        assert_eq!(
            p,
            Plan {
                name: "Poly Pluck".into(),
                hi: 81,
                ..Plan::default()
            }
        );
        assert_eq!(s.notes(), 16);
        assert_eq!(value_text(&s, Row::Layers, None), "3 · 40 90 127");
        assert_eq!(value_text(&s, Row::Lowest, None), "C2");
        assert_eq!(value_text(&s, Row::Tail, None), "up to 4 s");
        assert_eq!(
            value_text(&s, Row::Source, Some("Poly Pluck")),
            "Poly Pluck"
        );
        assert_eq!(
            (minutes_text(48.4), minutes_text(201.6)),
            ("48 s".to_string(), "3:22".to_string())
        );
        // Lowest pushes highest.
        let s2 = stepped(&s, Row::Lowest, 60);
        assert_eq!((s2.lo, s2.hi), (96, 96));
        assert_eq!(stepped(&s, Row::Onto, 1).onto, Onto::WholeKit);
        // Layers × round robins stay within a pad's 16 layers.
        let s3 = stepped(&stepped(&s, Row::RoundRobins, 3), Row::Layers, 5);
        assert_eq!((s3.layers, s3.round_robins), (8, 2));
        // 256 takes of 6 s are past the memory budget: the page says so
        // and Start waits; a note an octave fits.
        assert!(matches!(
            s3.plan("x", 44_100).check(),
            Err(PlanError::TooBig { .. })
        ));
        let s3_octaves = stepped(&s3, Row::Every, 9);
        assert_eq!(s3_octaves.every, 12);
        assert!(s3_octaves.plan("x", 44_100).check().is_ok());
        assert_eq!(stepped(&s3, Row::RoundRobins, 1).round_robins, 2);
        let s4 = stepped(&s, Row::Curve, 1);
        assert_eq!(value_text(&s4, Row::Curve, None), "More soft");
        assert_ne!(s4.velocities(), s.velocities());
        assert_eq!(stepped(&s, Row::Hold, 1).hold_ms, 3000);
        assert_eq!(stepped(&s, Row::Loop, 1).loops, Some(LoopMode::Forward));
        assert!(stepped(&s, Row::Channels, 1).mono);
        assert!(!stepped(&s, Row::Normalize, 1).normalize);
    }

    #[test]
    fn an_instrument_goes_onto_the_first_run_of_empty_pads() {
        let full = [0usize, 1, 9];
        let has = |p: usize| full.contains(&p);
        assert_eq!(target(Onto::EmptyPads, 7, 16, has), Some(2));
        assert_eq!(target(Onto::EmptyPads, 6, 16, has), Some(2));
        assert_eq!(target(Onto::EmptyPads, 7, 16, |p| p == 4), Some(5));
        assert_eq!(target(Onto::EmptyPads, 8, 16, has), None);
        assert_eq!(target(Onto::WholeKit, 16, 16, has), Some(0));
        assert_eq!(
            target(Onto::WholeKit, 17, 16, has),
            None,
            "a kit has 16 pads"
        );
        assert_eq!(AutoSettings::default().notes(), 16);
    }
}
