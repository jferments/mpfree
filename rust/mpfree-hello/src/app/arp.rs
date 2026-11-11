//! The arp in the pad player: what the quick band, the Arp panel and the
//! pattern workspace do (the design is in `docs/project/arpeggiator.md`).
//!
//! - **NOTE REPEAT held**: the pads play through the arp in its mode
//!   (Repeat, Arp or Pattern; Repeat if the panel picked Direct). The wheel
//!   picks the rate, order or pattern, - and + the mode, and Q-Links 1-4 set
//!   rate, gate, swing and octaves. The screen shows the quick band over the
//!   Play page. Let go and the pads play as pressed again, unless latched.
//! - **The Arp panel** (NOTE REPEAT + MENU, or a tap on the arp chip) sits
//!   beside the pads, and the pads play through the arp in the mode its tabs
//!   pick, Direct included. The wheel, - / + and the Q-Links work as above.
//! - **The pattern workspace** (Edit pattern or Record on pads, on the
//!   panel) edits the pattern by touch; the wheel turns the selected cell.
//!   Recording writes one step per pad press (Step mode, no clock), at
//!   least 8, while the pads play as pressed; it stops when they are full. Save keeps the
//!   pattern in memory until the power goes off.
//!
//! The screens never touch the engine: touches come back as `ArpAction`s and
//! `WsAction`s, and the screens hear what changed as [`Effect::Arp`].

use super::{App, Effect, View};
use mpfree_arp::{Held, Library, Mode, Order, Pattern, Rate, Recorder, Settings, Step, Wrote};
use mpfree_arp_ui::view::{ArpAction, PatternButton};
use mpfree_arp_ui::workspace::{Editor, WsAction};

/// What the arp screens show: the panel, the quick band and the chip.
#[derive(Clone, Debug, PartialEq)]
pub struct ArpView {
    /// The engine's settings. The mode is what the pads play through now:
    /// Direct when they play as pressed.
    pub settings: Settings,
    pub pattern: Pattern,
    pub pattern_index: usize,
    /// The pattern was edited, mutated or recorded from the list's.
    pub pattern_changed: bool,
    /// Held and latched pads (slots), in the order they were pressed.
    pub notes: Vec<Held>,
    /// Steps played since the notes started.
    pub step: u64,
    pub latched: bool,
    pub bpm: f64,
}

/// The pattern workspace, as the app wants it.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceView {
    pub name: String,
    pub pattern: Pattern,
    /// Open afresh on `pattern` (Original goes back to it).
    pub open: bool,
    pub recording: bool,
}

/// The built-in patterns, by name.
pub fn built_in_patterns() -> Vec<(String, Pattern)> {
    Library::built_in().iter().map(|(n, p)| (n.to_string(), *p)).collect()
}

/// Q-Link turn units per change of rate, gate, swing and octaves: rate and
/// octaves have few values, so they need a longer turn.
const QLINK_UNITS: [i32; 4] = [3, 1, 1, 4];
/// The panel's mode tabs, and the modes NOTE REPEAT can turn on.
const MODES: [Mode; 4] = [Mode::Direct, Mode::Repeat, Mode::Arp, Mode::Pattern];
const GENERATING: [Mode; 3] = [Mode::Repeat, Mode::Arp, Mode::Pattern];
const MUTATE_AMOUNT: u8 = 35;
/// Recording writes at least this many steps (a pattern stopped early
/// keeps what was written; Length - shortens it).
const RECORD_STEPS: usize = 8;
/// Hits at or above this velocity record an accented step.
const ACCENT_FROM: u8 = 110;

/// `now` moved `d` places along `all`, stopping at the ends.
fn step_in<T: Copy + PartialEq>(all: &[T], now: T, d: i32) -> T {
    let at = all.iter().position(|&x| x == now).unwrap_or(0) as i32;
    all[(at + d).clamp(0, all.len() as i32 - 1) as usize]
}

pub fn mode_label(m: Mode) -> &'static str {
    match m {
        Mode::Direct => "Direct",
        Mode::Repeat => "Repeat",
        Mode::Arp => "Arp",
        Mode::Pattern => "Pattern",
    }
}

impl App {
    /// What the pads play through now.
    pub(super) fn engine_mode(&self) -> Mode {
        if self.recorder.is_some() {
            // Recording: pads write steps and sound as pressed.
            Mode::Direct
        } else if self.repeating() {
            self.held_mode()
        } else if self.panel_open() {
            self.arp_mode
        } else {
            Mode::Direct
        }
    }

    /// The mode NOTE REPEAT and Latch turn on.
    fn held_mode(&self) -> Mode {
        if self.arp_mode == Mode::Direct { Mode::Repeat } else { self.arp_mode }
    }

    /// The Arp panel or the workspace is on screen, or a menu opened from
    /// the panel.
    pub fn panel_open(&self) -> bool {
        matches!(self.view, View::Arp | View::Pattern) || (self.view == View::Options && self.back == View::Arp)
    }

    fn pattern_changed(&self) -> bool {
        self.patterns.get(self.pattern_index).is_some_and(|(_, p)| *p != self.pattern)
    }

    fn pattern_name(&self) -> String {
        self.patterns.get(self.pattern_index).map(|(n, _)| n.clone()).unwrap_or_else(|| "Pattern".to_string())
    }

    pub fn arp_view(&self) -> Effect {
        Effect::Arp(ArpView {
            settings: self.engine.settings(),
            pattern: self.pattern,
            pattern_index: self.pattern_index,
            pattern_changed: self.pattern_changed(),
            notes: self.engine.active().as_slice().to_vec(),
            step: self.engine.step(),
            latched: self.latched,
            bpm: self.settings.bpm,
        })
    }

    /// Once a period, after `process`: the arp screens' view, if the step,
    /// the notes or a setting changed since they last heard.
    pub fn arp_poll(&mut self) -> Option<Effect> {
        let step = self.engine.step();
        if !self.arp_dirty && step == self.sent_step && *self.engine.active() == self.sent_notes {
            return None;
        }
        self.arp_dirty = false;
        self.sent_step = step;
        self.sent_notes.clone_from(self.engine.active());
        Some(self.arp_view())
    }

    /// After an arp setting or Latch changed: the engine, the status and the
    /// NOTE REPEAT light follow; the screens hear on the next poll.
    pub(super) fn arp_changed(&mut self) -> Vec<Effect> {
        self.update_engine();
        let mut fx = vec![self.status()];
        fx.extend(self.repeat_light());
        if self.in_menu() {
            fx.push(self.menu_view());
        }
        fx
    }

    /// SHIFT + NOTE REPEAT, NOTE REPEAT pressed twice, the band's Latch
    /// button, or NOTE REPEAT alone while latched (which lets go).
    pub(super) fn toggle_latch(&mut self) -> Vec<Effect> {
        self.latched = !self.latched;
        let mut fx = vec![Effect::Note(if self.latched { "Latch on" } else { "Latch off" }.to_string())];
        fx.extend(self.arp_changed());
        fx
    }

    fn set_rate(&mut self, r: Rate) -> Vec<Effect> {
        if self.settings.rate == r {
            return Vec::new();
        }
        self.settings.rate = r;
        let mut fx = self.arp_changed();
        fx.push(Effect::Settings(self.settings.clone()));
        fx
    }

    fn set_mode(&mut self, m: Mode) -> Vec<Effect> {
        self.arp_mode = m;
        let mut fx = vec![Effect::Note(format!("arp: {}", mode_label(m)))];
        fx.extend(self.arp_changed());
        fx
    }

    fn set_pattern(&mut self, p: Pattern) -> Vec<Effect> {
        self.pattern = p;
        self.engine.set_pattern(p);
        self.arp_dirty = true;
        Vec::new()
    }

    fn load_pattern(&mut self, i: usize) -> Vec<Effect> {
        let Some((name, p)) = self.patterns.get(i).cloned() else { return Vec::new() };
        self.pattern_index = i;
        self.set_pattern(p);
        vec![Effect::Note(format!("arp pattern: {name} ({})", p.to_text()))]
    }

    /// A touch on the Arp panel or the quick band.
    pub fn arp_action(&mut self, a: ArpAction) -> Vec<Effect> {
        match a {
            ArpAction::Mode(m) => self.set_mode(m),
            ArpAction::Rate(r) => self.set_rate(r),
            ArpAction::Order(o) => {
                self.arp.order = o;
                self.arp_changed()
            }
            ArpAction::Pattern(i) => self.load_pattern(i),
            ArpAction::Settings(s) => {
                self.arp.gate = s.gate;
                self.arp.swing = s.swing;
                self.arp.octaves = s.octaves;
                let mut fx = self.arp_changed();
                fx.extend(self.set_rate(s.rate));
                fx
            }
            ArpAction::Button(b) => self.pattern_button(b),
            ArpAction::Step(i) => {
                let mut e = Editor::new(self.pattern);
                if e.tap(i) { self.set_pattern(*e.pattern()) } else { Vec::new() }
            }
            ArpAction::Latch => self.toggle_latch(),
        }
    }

    fn pattern_button(&mut self, b: PatternButton) -> Vec<Effect> {
        match b {
            PatternButton::Edit => self.open_workspace(false),
            PatternButton::Record => self.open_workspace(true),
            PatternButton::Mutate => {
                self.mutations += 1;
                self.set_pattern(self.pattern.mutated(self.mutations, MUTATE_AMOUNT))
            }
            PatternButton::Original => self.load_pattern(self.pattern_index),
        }
    }

    /// The wheel while NOTE REPEAT is held or on the panel: the order (Arp),
    /// the pattern (Pattern) or the rate.
    pub(super) fn arp_wheel(&mut self, delta: i32) -> Vec<Effect> {
        match self.engine.settings().mode {
            Mode::Arp => {
                let o = step_in(&Order::ALL, self.arp.order, delta);
                if o == self.arp.order {
                    return Vec::new();
                }
                self.arp.order = o;
                let mut fx = vec![Effect::Note(format!("arp order: {}", o.label()))];
                fx.extend(self.arp_changed());
                fx
            }
            Mode::Pattern => {
                let i = (self.pattern_index as i32 + delta).clamp(0, self.patterns.len() as i32 - 1) as usize;
                if i == self.pattern_index { Vec::new() } else { self.load_pattern(i) }
            }
            Mode::Direct | Mode::Repeat => self.set_rate(step_in(&Rate::ALL, self.settings.rate, delta)),
        }
    }

    /// The - and + buttons while NOTE REPEAT is held (Repeat, Arp,
    /// Pattern) or on the panel (its four tabs).
    pub(super) fn arp_mode_step(&mut self, d: i32) -> Vec<Effect> {
        let m = if self.view == View::Arp {
            step_in(&MODES, self.arp_mode, d)
        } else {
            step_in(&GENERATING, self.held_mode(), d)
        };
        if m == self.arp_mode { Vec::new() } else { self.set_mode(m) }
    }

    /// A Q-Link turned. While NOTE REPEAT is held or on the panel, Q-Links
    /// 1-4 set rate, gate, swing and octaves; otherwise None leaves the
    /// knob to the page (Q-Link 1 is the level). `absolute` is the value
    /// (0-127) when the controller sends positions, without Akai's session.
    pub fn qlink(&mut self, knob: usize, delta: i32, absolute: Option<u8>) -> Option<Vec<Effect>> {
        if knob >= 4 || !(self.repeat_held || self.view == View::Arp) {
            return None;
        }
        let mut s = self.arp;
        s.rate = self.settings.rate;
        if let Some(v) = absolute {
            s = mpfree_arp_ui::view::with_qlink(s, knob, f32::from(v) / 127.0);
        } else {
            self.qlink_turns[knob] += delta;
            let steps = self.qlink_turns[knob] / QLINK_UNITS[knob];
            if steps == 0 {
                return Some(Vec::new());
            }
            self.qlink_turns[knob] -= steps * QLINK_UNITS[knob];
            match knob {
                0 => s.rate = step_in(&Rate::ALL, s.rate, steps),
                1 => s.gate = (i32::from(s.gate) + 2 * steps).clamp(1, 100) as u8,
                2 => s.swing = (i32::from(s.swing) + steps).clamp(50, 75) as u8,
                _ => s.octaves = (i32::from(s.octaves) + steps).clamp(1, 4) as u8,
            }
        }
        Some(self.arp_action(ArpAction::Settings(s)))
    }

    /// The arp chip, or NOTE REPEAT + MENU: open or close the panel. In the
    /// workspace it goes back to the panel.
    pub fn toggle_panel(&mut self) -> Vec<Effect> {
        match self.view {
            View::Guide => Vec::new(),
            View::Pattern => self.workspace_back(),
            _ => self.toggle(View::Arp),
        }
    }

    // ------------------------------------------------------------ workspace

    fn open_workspace(&mut self, record: bool) -> Vec<Effect> {
        if self.view == View::Pattern {
            return Vec::new();
        }
        let mut fx = vec![Effect::Workspace(WorkspaceView { name: self.pattern_name(), pattern: self.pattern,
                                                            open: true, recording: false })];
        fx.extend(self.show(View::Pattern));
        if record {
            fx.extend(self.set_recording(true));
        }
        fx
    }

    fn workspace_back(&mut self) -> Vec<Effect> {
        let mut fx = Vec::new();
        if self.recorder.is_some() {
            fx.extend(self.set_recording(false));
        }
        fx.extend(self.show(View::Arp));
        fx
    }

    /// Start or stop recording on the pads. A pattern stopped early keeps
    /// the steps written so far.
    pub(super) fn set_recording(&mut self, on: bool) -> Vec<Effect> {
        let mut fx = Vec::new();
        if on {
            let mut r = Recorder::step(self.pattern.len().max(RECORD_STEPS));
            r.set_accent_from(ACCENT_FROM);
            self.recorder = Some(r);
            fx.push(Effect::Note(format!("recording a pattern on the pads: {} steps, one per press", r.length())));
        } else if let Some(r) = self.recorder.take() {
            if let Some(p) = r.pattern(&self.pattern) {
                self.set_pattern(p);
            }
            fx.push(Effect::Note(format!("recording stopped: {}", self.pattern.to_text())));
        } else {
            return fx;
        }
        fx.push(Effect::Workspace(WorkspaceView { name: self.pattern_name(), pattern: self.pattern, open: false,
                                                  recording: on }));
        fx.extend(self.arp_changed());
        fx
    }

    /// A pad press while recording writes the next step. The screen sees
    /// the pattern at full length, the steps not yet written as rests.
    pub(super) fn record_press(&mut self, slot: u8, velocity: u8) -> Vec<Effect> {
        let v = self.velocity(velocity);
        let Some(r) = self.recorder.as_mut() else { return Vec::new() };
        match r.press(slot, v) {
            Wrote::Step { index, .. } => {
                let mut steps = [Step::Rest; mpfree_arp::MAX_STEPS];
                steps[..r.written()].copy_from_slice(r.steps());
                let full = r.is_full();
                let next = r.written().min(r.length() - 1);
                if let Some(mut p) = Pattern::from_steps(&steps[..r.length()]) {
                    p.lanes = self.pattern.lanes;
                    self.set_pattern(p);
                }
                let mut fx = vec![Effect::Recorded(self.pattern, next)];
                if full {
                    fx.push(Effect::Note(format!("recorded {} steps", index + 1)));
                    fx.extend(self.set_recording(false));
                }
                fx
            }
            Wrote::Full => self.set_recording(false),
            Wrote::Nothing => Vec::new(),
        }
    }

    /// A touch in the workspace (its editor already changed the pattern).
    pub fn workspace_action(&mut self, a: WsAction) -> Vec<Effect> {
        if self.view != View::Pattern {
            return Vec::new();
        }
        match a {
            WsAction::Changed(p) => self.set_pattern(p),
            WsAction::Back => self.workspace_back(),
            WsAction::Save(p) => {
                self.set_pattern(p);
                let name = self.pattern_name();
                if let Some(slot) = self.patterns.get_mut(self.pattern_index) {
                    slot.1 = p;
                }
                vec![Effect::Patterns(self.patterns.clone()),
                     Effect::Note(format!("pattern {name} saved in memory until the power goes off: {}", p.to_text())),
                     Effect::Workspace(WorkspaceView { name, pattern: p, open: true,
                                                       recording: self.recorder.is_some() })]
            }
            WsAction::Record(on) => self.set_recording(on),
        }
    }
}
