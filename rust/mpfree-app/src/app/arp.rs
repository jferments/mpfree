//! The arp in the pad player: what the quick band, the Arp panel and the
//! pattern workspace do (the design is in `docs/dev/design/arpeggiator.md`).
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

use super::{App, Effect, Item, PadNotes, View};
use mpfree_arp::scale::{Scale, Snap, NOTE_NAMES, SCALES};
use mpfree_arp::{
    Chord, Held, Library, ModSource, Mode, Modifier, Order, Pattern, Rate, RecordMode, Recorder,
    Settings, Step, Turns, Wrote,
};
use mpfree_arp_ui::view::{ArpAction, PatternButton};
use mpfree_arp_ui::workspace::{Editor, WsAction};
use mpfree_node::{Ctl, Curve, ParamInfo, Unit};
use mpfree_play::mpfree_param::arp as arp_ids;

/// What the arp screens show: the panel, the quick band and the chip.
#[derive(Clone, Debug, PartialEq)]
pub struct ArpView {
    /// The engine's settings. The mode is what the pads play through now:
    /// Direct when they play as pressed.
    pub settings: Settings,
    /// What it plays now, as the Modifier pushes the settings.
    pub live: Settings,
    /// How far the Modifier's controller is now, 0 to 1.
    pub push: f32,
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
    /// Recording, or the next recording, is in Free time.
    pub free_time: bool,
}

/// The built-in patterns, by name.
pub fn built_in_patterns() -> Vec<(String, Pattern)> {
    Library::built_in()
        .iter()
        .map(|(n, p)| (n.to_string(), *p))
        .collect()
}

/// Which of the arp's two library files a text is, for what the log says
/// of it. Both are read the same way ([`App::load_library`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum LibraryFile {
    /// The owner's patterns and scales: the store's arp/patterns.txt, or
    /// the `--patterns` option's file.
    Patterns,
    /// The owner's phrases: the store's arp/phrases.txt.
    Phrases,
}

impl LibraryFile {
    fn label(self) -> &'static str {
        match self {
            LibraryFile::Patterns => "arp patterns file",
            LibraryFile::Phrases => "arp phrases file",
        }
    }

    /// What is left when the file is not used.
    fn stay(self) -> &'static str {
        match self {
            LibraryFile::Patterns => "the built-in patterns stay",
            LibraryFile::Phrases => "MPFree's phrases stay",
        }
    }
}

/// Q-Link turn units per change of rate, gate, swing and octaves: rate and
/// octaves have few values, so they need a longer turn.
const QLINK_UNITS: [i32; 4] = [3, 1, 1, 4];
/// The panel's mode tabs, and the modes NOTE REPEAT can turn on.
mod part;
mod phrases;
/// The arp part's first line, for the project opener's check.
pub use part::HEADER as ARP_HEADER;
pub use phrases::built_in_phrases;

const MODES: [Mode; 4] = [Mode::Direct, Mode::Repeat, Mode::Arp, Mode::Pattern];
/// Every mode a project's arp part can name: the panel's tabs and Phrase,
/// whose tab is not built yet.
const KEPT_MODES: [Mode; 5] = [
    Mode::Direct,
    Mode::Repeat,
    Mode::Arp,
    Mode::Pattern,
    Mode::Phrase,
];
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

/// The arp's rows in the Options menu, opened with MENU on the Arp panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArpItem {
    Accent,
    Scale,
    Root,
    Snap,
    Chord,
    Run,
    Turns,
    Strum,
    Retrigger,
    SwingFrom,
    /// The arp options' row that opens the Modifier's section (its value
    /// the Modifier's controller), and that section's row back.
    ModSection,
    ModBack,
    /// The Modifier's controller ([`ModSource`]), then how far its top
    /// pushes each setting ([`Modifier`]).
    ModSource,
    ModRate,
    ModOctaves,
    ModGate,
    ModSwing,
    ModRolls,
    ModAccents,
}

/// The Options menu opened from the Arp panel: the tempo, then the arp's
/// own settings, the row that opens the Modifier's section, and how Latch
/// and the sustain pedal latch (Prefs' rows, here too). (Rate, mode, gate,
/// swing and octaves are on the panel.)
pub(super) const ARP_OPTIONS: [Item; 14] = [
    Item::Tempo,
    Item::Arp(ArpItem::Accent),
    Item::Arp(ArpItem::Scale),
    Item::Arp(ArpItem::Root),
    Item::Arp(ArpItem::Snap),
    Item::Arp(ArpItem::Chord),
    Item::Arp(ArpItem::Run),
    Item::Arp(ArpItem::Turns),
    Item::Arp(ArpItem::Strum),
    Item::Arp(ArpItem::Retrigger),
    Item::Arp(ArpItem::SwingFrom),
    Item::Arp(ArpItem::ModSection),
    Item::LatchMode,
    Item::PedalLatch,
];

/// The arp options' Modifier section (the MPC's Arpeggiator modifiers,
/// 3.10.1 guide pp. 83-84): back, the controller, then how far its top
/// pushes each setting. The Modifier box on the panel opens it.
pub(super) const MOD_ROWS: [Item; 8] = [
    Item::Arp(ArpItem::ModBack),
    Item::Arp(ArpItem::ModSource),
    Item::Arp(ArpItem::ModRate),
    Item::Arp(ArpItem::ModOctaves),
    Item::Arp(ArpItem::ModGate),
    Item::Arp(ArpItem::ModSwing),
    Item::Arp(ArpItem::ModRolls),
    Item::Arp(ArpItem::ModAccents),
];

/// Accent level steps by this much per press or wheel click.
const ACCENT_STEP: i32 = 8;
/// Strum steps by this many percent.
const STRUM_STEP: i32 = 10;
/// The Modifier's gate and accents step by this many percent.
const MOD_GATE_STEP: i32 = 5;
const MOD_ACCENTS_STEP: i32 = 10;

/// Where the sustain pedal went down: the arp's latch, or the sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PedalTo {
    Arp,
    Sound,
}

/// A signed push as its row shows it: "Off", "+2 steps faster".
fn signed(v: i8, one: &str, many: &str, up: &str, down: &str) -> String {
    let n = i32::from(v).abs();
    let unit = if n == 1 { one } else { many };
    match v {
        0 => "Off".to_string(),
        v if v > 0 => format!("+{n}{unit}{up}"),
        _ => format!("-{n}{unit}{down}"),
    }
}

fn snap_text(s: Snap) -> &'static str {
    match s {
        Snap::Down => "Note below",
        Snap::Up => "Note above",
        Snap::Nearest => "Nearest note",
        Snap::Skip => "Not played",
    }
}

pub fn mode_label(m: Mode) -> &'static str {
    match m {
        Mode::Direct => "Direct",
        Mode::Repeat => "Repeat",
        Mode::Arp => "Arp",
        Mode::Pattern => "Pattern",
        Mode::Phrase => "Phrase",
    }
}

impl App {
    /// What the pads play through now.
    pub(super) fn engine_mode(&self) -> Mode {
        let mode = if self.free_time_recording() {
            // Free time: the pattern plays the latched notes while the pads
            // write steps into it.
            Mode::Pattern
        } else if self.recorder.is_some() {
            // Step time: pads write steps and sound as pressed.
            Mode::Direct
        } else if self.repeating() {
            self.held_mode()
        } else if self.panel_open() {
            self.arp_mode
        } else {
            Mode::Direct
        };
        // A phrase moves notes: on a kit the pads play as pressed.
        if mode == Mode::Phrase && !self.phrases_play() {
            Mode::Direct
        } else {
            mode
        }
    }

    /// Recording on the pads in Free time.
    pub fn free_time_recording(&self) -> bool {
        self.recorder
            .is_some_and(|r| r.mode() == RecordMode::FreeTime)
    }

    fn workspace_view(&self, open: bool) -> WorkspaceView {
        WorkspaceView {
            name: self.pattern_name(),
            pattern: self.pattern,
            open,
            recording: self.recorder.is_some(),
            free_time: self.record_mode == RecordMode::FreeTime,
        }
    }

    /// The mode NOTE REPEAT and Latch turn on.
    fn held_mode(&self) -> Mode {
        if self.arp_mode == Mode::Direct {
            Mode::Repeat
        } else {
            self.arp_mode
        }
    }

    /// The Arp panel or the workspace is on screen, or a menu opened from
    /// the panel.
    pub fn panel_open(&self) -> bool {
        matches!(self.view, View::Arp | View::Pattern)
            || (self.view == View::Options && self.back == View::Arp)
    }

    fn pattern_changed(&self) -> bool {
        self.patterns
            .get(self.pattern_index)
            .is_some_and(|(_, p)| *p != self.pattern)
    }

    fn pattern_name(&self) -> String {
        self.patterns
            .get(self.pattern_index)
            .map(|(n, _)| n.clone())
            .unwrap_or_else(|| "Pattern".to_string())
    }

    pub fn arp_view(&self) -> Effect {
        Effect::Arp(ArpView {
            settings: self.arps.engine_at(self.track).settings(),
            live: self.arps.engine_at(self.track).live(),
            push: self.arps.engine_at(self.track).modifier(),
            pattern: self.pattern,
            pattern_index: self.pattern_index,
            pattern_changed: self.pattern_changed(),
            notes: self.arps.engine_at(self.track).active().as_slice().to_vec(),
            step: self.arps.engine_at(self.track).step(),
            latched: self.latched,
            bpm: self.settings.bpm,
        })
    }

    /// Once a period, after `process`: the arp screens' view, if the step,
    /// the notes or a setting changed since they last heard.
    pub fn arp_poll(&mut self) -> Option<Effect> {
        let e = self.arps.engine_at(self.track);
        let step = e.step();
        let push = ((e.modifier() * 100.0).round() as u8, e.live());
        if !self.arp_dirty
            && step == self.sent_step
            && push == self.sent_push
            && *e.active() == self.sent_notes
        {
            return None;
        }
        self.arp_dirty = false;
        self.sent_step = step;
        self.sent_push = push;
        self.sent_notes
            .clone_from(self.arps.engine_at(self.track).active());
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
        let mut fx = vec![Effect::Note(
            if self.latched {
                "Latch on"
            } else {
                "Latch off"
            }
            .to_string(),
        )];
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
        self.arps.engine_at_mut(self.track).set_pattern(p);
        self.arp_dirty = true;
        Vec::new()
    }

    fn load_pattern(&mut self, i: usize) -> Vec<Effect> {
        let Some((name, p)) = self.patterns.get(i).cloned() else {
            return Vec::new();
        };
        self.pattern_index = i;
        self.set_pattern(p);
        vec![Effect::Note(format!(
            "arp pattern: {name} ({})",
            p.to_text()
        ))]
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
                // Swing follows the transport until it is turned (design
                // decision 6); the Options row "Swing from" turns it back.
                let took_swing = s.swing != self.arp.swing && self.swing_from_transport;
                if took_swing {
                    self.swing_from_transport = false;
                }
                self.arp.gate = s.gate;
                self.arp.swing = s.swing;
                self.arp.octaves = s.octaves;
                let mut fx = self.arp_changed();
                if took_swing {
                    fx.push(Effect::Note(
                        "arp swing from: arp (turned; Options > Swing from goes back to the transport)".to_string(),
                    ));
                }
                fx.extend(self.set_rate(s.rate));
                fx
            }
            ArpAction::Button(b) => self.pattern_button(b),
            ArpAction::Step(i) => {
                let mut e = Editor::new(self.pattern);
                if e.tap(i) {
                    self.set_pattern(*e.pattern())
                } else {
                    Vec::new()
                }
            }
            ArpAction::Latch => self.toggle_latch(),
            ArpAction::OpenPanel if self.view != View::Arp => self.toggle_panel(),
            ArpAction::OpenPanel => Vec::new(),
            // The Modifier box: the arp options' Modifier section.
            ArpAction::Modifier => {
                let mut fx = if self.view == View::Options {
                    Vec::new()
                } else {
                    self.show(View::Options)
                };
                fx.extend(self.open_mod_section());
                fx
            }
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
    /// the pattern (Pattern), the phrase (Phrase) or the rate.
    pub(super) fn arp_wheel(&mut self, delta: i32) -> Vec<Effect> {
        match self.arps.engine_at(self.track).settings().mode {
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
                let i = (self.pattern_index as i32 + delta).clamp(0, self.patterns.len() as i32 - 1)
                    as usize;
                if i == self.pattern_index {
                    Vec::new()
                } else {
                    self.load_pattern(i)
                }
            }
            Mode::Phrase => self.phrase_wheel(delta),
            Mode::Direct | Mode::Repeat => {
                self.set_rate(step_in(&Rate::ALL, self.settings.rate, delta))
            }
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
        if m == self.arp_mode {
            Vec::new()
        } else {
            self.set_mode(m)
        }
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
        let mut fx = vec![Effect::Workspace(self.workspace_view(true))];
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
    /// the steps written so far. Free time needs notes to play the pattern
    /// over (latched before recording starts); without any it records in
    /// Step time.
    pub(super) fn set_recording(&mut self, on: bool) -> Vec<Effect> {
        let mut fx = Vec::new();
        if on {
            let free = self.record_mode == RecordMode::FreeTime;
            let notes = !self.arps.engine_at(self.track).active().is_empty();
            if free && !notes {
                fx.push(Effect::Note("Free time plays the pattern over latched notes: latch some pads first (NOTE \
                                      REPEAT twice, then the pads). Recording in Step time instead."
                    .to_string()));
            }
            let mut r = if free && notes {
                Recorder::free_time(&self.pattern)
            } else {
                Recorder::step(self.pattern.len().max(RECORD_STEPS))
            };
            r.set_accent_from(ACCENT_FROM);
            self.recorder = Some(r);
            fx.push(Effect::Note(match r.mode() {
                RecordMode::Step => format!(
                    "recording a pattern on the pads: {} steps, one per press",
                    r.length()
                ),
                RecordMode::FreeTime => {
                    format!(
                        "recording in Free time: {} steps loop; presses land on the nearest step",
                        r.length()
                    )
                }
            }));
        } else if let Some(r) = self.recorder.take() {
            if let Some(p) = self.recorded_pattern(&r) {
                self.set_pattern(p);
            }
            fx.push(Effect::Note(format!(
                "recording stopped: {}",
                self.pattern.to_text()
            )));
        } else {
            return fx;
        }
        let mut w = self.workspace_view(false);
        w.free_time =
            self.free_time_recording() || (!on && self.record_mode == RecordMode::FreeTime);
        fx.push(Effect::Workspace(w));
        fx.extend(self.arp_changed());
        fx
    }

    /// What `r` has recorded. Step time: the steps written so far, then
    /// rests to the full length. Free time: the whole loop.
    fn recorded_pattern(&self, r: &Recorder) -> Option<Pattern> {
        match r.mode() {
            RecordMode::FreeTime => r.pattern(&self.pattern),
            RecordMode::Step => {
                let mut steps = [Step::Rest; mpfree_arp::MAX_STEPS];
                steps[..r.written()].copy_from_slice(r.steps());
                let mut p = Pattern::from_steps(&steps[..r.length()])?;
                p.lanes = self.pattern.lanes;
                Some(p)
            }
        }
    }

    /// The pattern being recorded changed: play it and show it, with the
    /// cursor on `next`.
    fn show_recorded(&mut self, next: usize) -> Vec<Effect> {
        let Some(r) = self.recorder else {
            return Vec::new();
        };
        if let Some(p) = self.recorded_pattern(&r) {
            self.set_pattern(p);
        }
        vec![Effect::Recorded(
            self.pattern,
            next.min(self.pattern.len().saturating_sub(1)),
        )]
    }

    /// A pad press while recording. Step time writes the next step; Free
    /// time writes the step nearest the pattern's position now.
    pub(super) fn record_press(&mut self, slot: u8, velocity: u8) -> Vec<Effect> {
        let v = self.velocity(velocity);
        let position = self.arps.engine_at_mut(self.track).position();
        let Some(r) = self.recorder.as_mut() else {
            return Vec::new();
        };
        let wrote = match r.mode() {
            RecordMode::Step => r.press(slot, v),
            RecordMode::FreeTime => r.press_at(slot, v, position),
        };
        let (full, written, length, free) = (
            r.is_full(),
            r.written(),
            r.length(),
            r.mode() == RecordMode::FreeTime,
        );
        match wrote {
            Wrote::Step { index, .. } => {
                let next = if free { index } else { written.min(length - 1) };
                let mut fx = self.show_recorded(next);
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

    /// A pad let go while recording: in Free time a pad held past the next
    /// step ties the steps it covered.
    pub(super) fn record_release(&mut self, slot: u8) -> Vec<Effect> {
        let position = self.arps.engine_at_mut(self.track).position();
        let Some(r) = self.recorder.as_mut() else {
            return Vec::new();
        };
        let before = r.steps().to_vec();
        r.release(slot, position);
        if r.steps() == before.as_slice() {
            return Vec::new();
        }
        // The cursor goes to the last step the hold reached.
        let len = r.length().max(1);
        let next = ((position.round() - 1.0).max(0.0) as usize) % len;
        self.show_recorded(next)
    }

    /// Step time or Free time, from now on; a recording under way restarts
    /// in the new mode, keeping the pattern written so far.
    fn set_record_mode(&mut self, mode: RecordMode) -> Vec<Effect> {
        if self.record_mode == mode && self.recorder.is_none_or(|r| r.mode() == mode) {
            return Vec::new();
        }
        self.record_mode = mode;
        if self.recorder.is_none() {
            return vec![Effect::Workspace(self.workspace_view(false))];
        }
        let mut fx = self.set_recording(false);
        fx.extend(self.set_recording(true));
        fx
    }

    /// Undo (Step time) or Clear while recording.
    fn record_edit(&mut self, clear: bool) -> Vec<Effect> {
        let Some(r) = self.recorder.as_mut() else {
            return Vec::new();
        };
        let done = if clear {
            r.clear();
            true
        } else {
            r.undo()
        };
        if !done {
            return Vec::new();
        }
        let next = if r.mode() == RecordMode::Step {
            r.written().min(r.length() - 1)
        } else {
            0
        };
        let mut fx = self.show_recorded(next);
        fx.push(Effect::Note(
            if clear {
                "recording cleared"
            } else {
                "last step taken back"
            }
            .to_string(),
        ));
        fx
    }

    /// Patterns from a library file (the store's arp/patterns.txt, or the
    /// `--patterns` option): one with a
    /// built-in's name replaces it, the others come after the built-ins. A
    /// file with any bad line is not used, and every bad line is logged.
    /// Phrase lines in it are the owner's phrases, as in their own file.
    pub fn load_patterns(&mut self, text: &str) -> Vec<Effect> {
        self.load_library(text, LibraryFile::Patterns)
    }

    /// The owner's own phrases from their library file (the store's
    /// arp/phrases.txt): one with the name of a phrase in the list takes
    /// its place, the others come after MPFree's. A file with any bad line
    /// is not used, and every bad line is logged. Patterns and scales in
    /// it are read as a patterns file's are.
    pub fn load_phrases(&mut self, text: &str) -> Vec<Effect> {
        self.load_library(text, LibraryFile::Phrases)
    }

    /// One reader for both of the arp's library files: whatever a file
    /// holds is used, and Save sorts each kind into its own file
    /// (`library_text`, `phrases_text`), so a line in the other file is
    /// never lost.
    fn load_library(&mut self, text: &str, file: LibraryFile) -> Vec<Effect> {
        let what = file.label();
        let lib = match Library::parse(text) {
            Ok(lib) => lib,
            Err(errors) => {
                let mut fx: Vec<Effect> = errors
                    .iter()
                    .take(20)
                    .map(|e| Effect::Note(format!("{what}, {e}")))
                    .collect();
                fx.push(Effect::Note(format!(
                    "{what} not used ({} bad lines); {}",
                    errors.len(),
                    file.stay()
                )));
                return fx;
            }
        };
        let (mut replaced, mut added) = (0, 0);
        let mut scales = 0;
        for (name, up, down) in lib.scales() {
            match self
                .custom_scales
                .iter_mut()
                .find(|(n, ..)| n.eq_ignore_ascii_case(name))
            {
                Some(slot) => *slot = (name.to_string(), up, down),
                None => self.custom_scales.push((name.to_string(), up, down)),
            }
            scales += 1;
        }
        for (name, p) in lib.iter() {
            match self
                .patterns
                .iter_mut()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
            {
                Some(slot) => {
                    slot.1 = *p;
                    replaced += 1;
                }
                None => {
                    self.patterns.push((name.to_string(), *p));
                    added += 1;
                }
            }
        }
        let (phrases_added, phrases_replaced) = self.take_phrases(&lib);
        let mut fx = Vec::new();
        if file == LibraryFile::Patterns || added + replaced + scales > 0 {
            // A scale of the owner's in use may have changed.
            if self.scale_pick >= SCALES.len() {
                self.apply_scale_pick();
                self.update_engine();
            }
            fx.extend(self.load_pattern(self.pattern_index));
            fx.push(Effect::Patterns(self.patterns.clone()));
            fx.push(Effect::Note(format!(
                "{what}: {added} added, {replaced} built-in ones replaced, \
                                      {scales} scales of your own"
            )));
        }
        if file == LibraryFile::Phrases || phrases_added + phrases_replaced > 0 {
            fx.push(Effect::Note(format!(
                "{what}: {phrases_added} phrases added, {phrases_replaced} replaced"
            )));
        }
        fx
    }

    /// The owner's patterns as a library file: every pattern that differs
    /// from the built-in of its name, and every new one.
    pub fn library_text(&self) -> String {
        let built_in = Library::built_in();
        let mut lib = Library::default();
        for (name, p) in &self.patterns {
            if built_in.get(name) != Some(p) {
                let _ = lib.put(name, *p);
            }
        }
        for (name, up, down) in &self.custom_scales {
            let _ = lib.put_scale(name, &mpfree_arp::scale::scale_text(*up, *down));
        }
        format!(
            "# MPFree arp patterns, saved by the pad player. One per line: Name = pattern;\n\
                 # your scales after [scales]. The syntax is in docs/dev/design/arpeggiator.md.\n{}",
            lib.to_text()
        )
    }

    // ------------------------------------------------------------ transport

    /// The transport's clock for the next [`App::process`]: called once a
    /// period, before it, while the transport plays (the time engine gives
    /// the first span of the period). None, or no call, and the arp
    /// free-runs at the app's tempo.
    #[cfg(test)]
    pub fn set_clock(&mut self, clock: Option<mpfree_arp::Clock>) {
        self.clock = clock;
    }

    /// Before each period: the engine follows this period's clock, with
    /// the transport's swing or the arp's own ("Swing from").
    pub(super) fn sync_arp(&mut self) {
        match self.clock.take() {
            Some(mut c) => {
                if !self.swing_from_transport {
                    c.swing = None;
                }
                self.arps.engine_at_mut(self.track).sync(&c);
            }
            None if self.arps.engine_at(self.track).synced() => {
                self.arps
                    .engine_at_mut(self.track)
                    .sync(&mpfree_arp::Clock {
                        playing: false,
                        ..mpfree_arp::Clock::playing(0, 0, self.settings.bpm)
                    });
                self.arps
                    .engine_at_mut(self.track)
                    .set_tempo(self.settings.bpm);
            }
            None => {}
        }
    }

    // ------------------------------------------------------------ options

    /// The Scale row's choices: 0 is Off, then the built-in scales but
    /// Chromatic (which keeps every note, as Off does), then the owner's.
    fn scale_choices(&self) -> usize {
        SCALES.len() + self.custom_scales.len()
    }

    /// Choice `i`'s name and notes (up, down); None for Off.
    fn scale_entry(&self, i: usize) -> Option<(&str, u16, u16)> {
        match i {
            0 => None,
            i if i < SCALES.len() => Some((SCALES[i].name, SCALES[i].mask, SCALES[i].down)),
            i => self
                .custom_scales
                .get(i - SCALES.len())
                .map(|(n, u, d)| (n.as_str(), *u, *d)),
        }
    }

    fn scale_choice(&self) -> usize {
        if self.arp.scale.is_some() {
            self.scale_pick
        } else {
            0
        }
    }

    /// The arp's scale from the Scale row's choice and the root.
    fn apply_scale_pick(&mut self) {
        self.arp.scale = self
            .scale_entry(self.scale_pick)
            .map(|(_, up, down)| Scale::with_down(self.scale_root, up, down));
    }

    fn scale_text(&self) -> String {
        match self.scale_entry(self.scale_choice()) {
            None => "Off".to_string(),
            Some((name, ..)) => format!("{} {}", NOTE_NAMES[usize::from(self.scale_root)], name),
        }
    }

    pub(super) fn arp_item_text(&self, item: ArpItem) -> (&'static str, String) {
        let s = &self.arp;
        match item {
            ArpItem::Accent => ("Accent level", s.accent.to_string()),
            ArpItem::Scale => (
                "Scale",
                match self.scale_entry(self.scale_choice()) {
                    None => "Off".to_string(),
                    Some((name, ..)) => name.to_string(),
                },
            ),
            ArpItem::Root => (
                "Scale root",
                NOTE_NAMES[usize::from(self.scale_root)].to_string(),
            ),
            ArpItem::Snap => ("Out-of-key notes", snap_text(s.snap).to_string()),
            ArpItem::Chord => ("Chord on each pad", s.chord.label().to_string()),
            ArpItem::Run => ("Scale run", if s.run { "On" } else { "Off" }.to_string()),
            ArpItem::Turns => ("Chords take turns", s.turns.label().to_string()),
            ArpItem::Strum => (
                "Strum",
                if s.strum == 0 {
                    "Off".to_string()
                } else {
                    format!("{} % of a step", s.strum)
                },
            ),
            ArpItem::Retrigger => (
                "New notes restart",
                if s.retrigger { "On" } else { "Off" }.to_string(),
            ),
            ArpItem::SwingFrom => (
                "Swing from",
                if self.swing_from_transport {
                    "Transport"
                } else {
                    "Arp"
                }
                .to_string(),
            ),
            ArpItem::ModSection => ("Modifier", s.modifier.source.label().to_string()),
            ArpItem::ModBack => ("Back to Arp options", "Back".to_string()),
            ArpItem::ModSource => ("Controller", s.modifier.source.label().to_string()),
            ArpItem::ModRate => (
                "Pushes rate",
                signed(s.modifier.rate, " step", " steps", " faster", " slower"),
            ),
            ArpItem::ModOctaves => (
                "Pushes octaves",
                signed(s.modifier.octaves, " octave", " octaves", "", ""),
            ),
            ArpItem::ModGate => ("Pushes gate", signed(s.modifier.gate, " %", " %", "", "")),
            ArpItem::ModSwing => ("Pushes swing", signed(s.modifier.swing, " %", " %", "", "")),
            ArpItem::ModRolls => (
                "Adds rolls",
                match s.modifier.rolls {
                    0 => "Off".to_string(),
                    1 => "+1 hit a step".to_string(),
                    n => format!("+{n} hits a step"),
                },
            ),
            ArpItem::ModAccents => (
                "Adds accents",
                match s.modifier.accents {
                    0 => "Off".to_string(),
                    n => format!("{n} % of steps"),
                },
            ),
        }
    }

    pub(super) fn arp_item_help(item: ArpItem) -> Vec<&'static str> {
        match item {
            ArpItem::Accent => vec![
                "How loud accented steps play: pattern steps",
                "marked with an accent, and steps recorded",
                "from hits of 110 or more. FULL LEVEL and",
                "Half Level override it.",
            ],
            ArpItem::Scale => vec![
                "Keeps the arp's notes in key: a pad off the",
                "scale moves into it (see Out-of-key notes),",
                "and the octaves and Degree lanes move along",
                "the scale. Scales from your patterns file",
                "come after the built-in ones. For a synth on",
                "the pads; leave it Off for drum kits, whose",
                "pads are sounds, not notes.",
            ],
            ArpItem::Root => vec![
                "The scale's first note. Pad 1 counts as C,",
                "pad 2 as C# and so on up.",
            ],
            ArpItem::Snap => vec![
                "With a scale on, what a held note outside it",
                "plays: the scale note below it, the one above,",
                "the nearer of the two (below when they are as",
                "near), or nothing: its steps stay silent.",
            ],
            ArpItem::Chord => vec![
                "Each pad you hold plays a chord built from",
                "the scale (from the major scale on the pad",
                "without one), and the arp runs over all its",
                "notes: one pad is enough for an arpeggio.",
                "For a synth on the pads, not drum kits.",
            ],
            ArpItem::Run => vec![
                "Arp and Pattern: the steps go through every",
                "scale note between your lowest and highest",
                "held notes (an octave up from one note), in",
                "the order you picked. Every semitone without",
                "a scale.",
            ],
            ArpItem::Turns => vec![
                "Hold two or more pads and they take turns",
                "instead of sounding together: the steps play",
                "from one pad (its chord, with Chord on each",
                "pad), then the next, in the order you pressed",
                "them, a beat or a bar each. Hold Am, F, C and",
                "G to arpeggiate that progression.",
            ],
            ArpItem::Strum => vec![
                "Spreads the notes of a chord step across the",
                "step, lowest first, like a strum. Off plays",
                "them together.",
            ],
            ArpItem::Retrigger => vec![
                "On: each new pad press starts the arp's order",
                "and the pattern again from the first step.",
                "Off: they carry on where they are.",
            ],
            ArpItem::SwingFrom => vec![
                "Transport: while the sequencer plays with",
                "swing on, the arp swings with it, on the",
                "sequencer's grid. Arp: always the arp's own",
                "swing (Q-Link 3). Stopped, or with the",
                "sequencer's swing off, it is the arp's own.",
            ],
            ArpItem::ModSection => vec![
                "One controller pushes this track's arp while",
                "you play: faster, more octaves, longer notes,",
                "more swing, rolls or accents, and back as you",
                "let it go. Push the wheel or an arrow to pick",
                "the controller and how far it pushes (the",
                "Modifier box on the Arp panel opens it too).",
            ],
            ArpItem::ModBack => vec!["Back to the arp's other options."],
            ArpItem::ModSource => vec![
                "One controller that pushes this track's arp",
                "while you play: a keyboard's mod wheel, the",
                "pressure of the pads or keys you hold, an",
                "expression pedal (CC 11), breath (CC 2), or a",
                "knob MIDI Learn gives it (SHIFT + touch the",
                "Modifier box, then turn the knob). At its top",
                "it pushes by the amounts below; let it back",
                "and the arp plays as set. The sound still",
                "gets the controller too.",
            ],
            ArpItem::ModRate => vec![
                "How many rates faster the controller's top",
                "plays, along 1/4, 1/8, 1/8T, 1/16, 1/16T,",
                "1/32 and 1/32T: from 1/16, +2 is 1/32.",
                "Below zero slows the arp down instead. A new",
                "rate starts with the next step, on the beat.",
            ],
            ArpItem::ModOctaves => vec![
                "Octaves the controller's top adds to the",
                "order's (Q-Link 4), 1 to 4 in all. Below zero",
                "takes octaves away.",
            ],
            ArpItem::ModGate => vec![
                "Percent the controller's top adds to each",
                "note's length (Q-Link 2), 1 to 100 % in all.",
                "Below zero shortens the notes: a tighter,",
                "choppier arp as you push.",
            ],
            ArpItem::ModSwing => vec![
                "Percent the controller's top adds to the",
                "swing (Q-Link 3), up to 75 %. Below zero",
                "straightens it.",
            ],
            ArpItem::ModRolls => vec![
                "Extra hits the controller's top plays in",
                "every step, up to 7: a roll that builds as",
                "you push, for drums with Note Repeat. A",
                "pattern step that already rolls gets them",
                "on top (8 hits at most).",
            ],
            ArpItem::ModAccents => vec![
                "The chance, at the controller's top, that a",
                "step plays accented, at the Accent level.",
                "Steps already accented stay accented.",
            ],
        }
    }

    /// One of the arp's option rows stepped by `d` (-1 or +1).
    pub(super) fn step_arp_item(&mut self, item: ArpItem, d: i32) -> Vec<Effect> {
        let before = (self.arp, self.scale_root, self.scale_pick);
        match item {
            ArpItem::Accent => {
                self.arp.accent = (i32::from(self.arp.accent) + ACCENT_STEP * d).clamp(1, 127) as u8
            }
            ArpItem::Scale => {
                self.scale_pick = (self.scale_choice() as i32 + d)
                    .clamp(0, self.scale_choices() as i32 - 1)
                    as usize;
                self.apply_scale_pick();
            }
            ArpItem::Root => {
                self.scale_root = (i32::from(self.scale_root) + d).clamp(0, 11) as u8;
                self.arp.scale = self
                    .arp
                    .scale
                    .map(|s| Scale::with_down(self.scale_root, s.mask, s.down));
            }
            ArpItem::Snap => {
                let at = Snap::ALL
                    .iter()
                    .position(|&x| x == self.arp.snap)
                    .unwrap_or(0) as i32;
                self.arp.snap = Snap::ALL[(at + d).clamp(0, Snap::ALL.len() as i32 - 1) as usize];
            }
            ArpItem::Chord => self.arp.chord = step_in(&Chord::ALL, self.arp.chord, d),
            ArpItem::Run => self.arp.run = d > 0,
            ArpItem::Turns => self.arp.turns = step_in(&Turns::ALL, self.arp.turns, d),
            ArpItem::Strum => {
                self.arp.strum = (i32::from(self.arp.strum) + STRUM_STEP * d).clamp(0, 100) as u8
            }
            ArpItem::Retrigger => self.arp.retrigger = d > 0,
            ArpItem::ModSection => return self.open_mod_section(),
            ArpItem::ModBack => return self.leave_mod_section(),
            ArpItem::ModSource => {
                let m = &mut self.arp.modifier;
                m.source = step_in(&ModSource::ALL, m.source, d);
            }
            ArpItem::ModRate => self.push_amount(|m| &mut m.rate, d, Modifier::RATE),
            ArpItem::ModOctaves => self.push_amount(|m| &mut m.octaves, d, Modifier::OCTAVES),
            ArpItem::ModGate => {
                self.push_amount(|m| &mut m.gate, MOD_GATE_STEP * d, Modifier::GATE)
            }
            ArpItem::ModSwing => self.push_amount(|m| &mut m.swing, d, Modifier::SWING),
            ArpItem::ModRolls => {
                let m = &mut self.arp.modifier;
                m.rolls = (i32::from(m.rolls) + d).clamp(0, i32::from(Modifier::ROLLS)) as u8;
            }
            ArpItem::ModAccents => {
                let m = &mut self.arp.modifier;
                m.accents = (i32::from(m.accents) + MOD_ACCENTS_STEP * d)
                    .clamp(0, i32::from(Modifier::ACCENTS)) as u8;
            }
            ArpItem::SwingFrom => {
                let t = d > 0;
                if t == self.swing_from_transport {
                    return Vec::new();
                }
                self.swing_from_transport = t;
                self.update_engine();
                return vec![Effect::Note(format!(
                    "arp swing from: {}",
                    if t { "transport" } else { "arp" }
                ))];
            }
        }
        if (self.arp, self.scale_root, self.scale_pick) == before {
            return Vec::new();
        }
        self.update_engine();
        let note = match item {
            ArpItem::Accent => format!("arp accent level: {}", self.arp.accent),
            ArpItem::Scale | ArpItem::Root => format!("arp scale: {}", self.scale_text()),
            ArpItem::Snap => format!(
                "arp out-of-key notes: {}",
                snap_text(self.arp.snap).to_ascii_lowercase()
            ),
            ArpItem::Chord => format!(
                "arp chord on each pad: {}",
                self.arp.chord.label().to_ascii_lowercase()
            ),
            ArpItem::Run => format!("arp scale run: {}", if self.arp.run { "on" } else { "off" }),
            ArpItem::Turns => format!(
                "arp chords take turns: {}",
                self.arp.turns.label().to_ascii_lowercase()
            ),
            ArpItem::Strum => format!("arp strum: {} %", self.arp.strum),
            ArpItem::Retrigger => format!(
                "arp new notes restart: {}",
                if self.arp.retrigger { "on" } else { "off" }
            ),
            ArpItem::ModSource => format!(
                "arp modifier: {}",
                self.arp.modifier.source.label().to_ascii_lowercase()
            ),
            ArpItem::ModRate
            | ArpItem::ModOctaves
            | ArpItem::ModGate
            | ArpItem::ModSwing
            | ArpItem::ModRolls
            | ArpItem::ModAccents => {
                let (name, v) = self.arp_item_text(item);
                format!(
                    "arp modifier {}: {}",
                    name.to_ascii_lowercase(),
                    v.to_ascii_lowercase()
                )
            }
            ArpItem::SwingFrom | ArpItem::ModSection | ArpItem::ModBack => {
                unreachable!("returned above")
            }
        };
        vec![Effect::Note(note)]
    }

    /// The arp options' Modifier section, on its controller's row.
    pub(super) fn open_mod_section(&mut self) -> Vec<Effect> {
        self.mod_section = true;
        self.menu_editing = false;
        self.selected = 1;
        vec![self.menu_view()]
    }

    /// From the Modifier section back to the arp options, on its row.
    pub(super) fn leave_mod_section(&mut self) -> Vec<Effect> {
        self.mod_section = false;
        self.menu_editing = false;
        self.selected = ARP_OPTIONS
            .iter()
            .position(|&i| i == Item::Arp(ArpItem::ModSection))
            .unwrap_or(0);
        vec![self.menu_view()]
    }

    /// The contact sheet's Modifier: the mod wheel, with the gate +30 on
    /// top of the defaults (rate +2 steps, an octave).
    pub fn arp_modifier_demo(&mut self) -> Vec<Effect> {
        let mut fx = self.step_arp_item(ArpItem::ModSource, 1);
        for _ in 0..6 {
            fx.extend(self.step_arp_item(ArpItem::ModGate, 1));
        }
        fx
    }

    /// One of the Modifier's signed amounts stepped by `d`, kept to
    /// `(lo, hi)`.
    fn push_amount(&mut self, at: fn(&mut Modifier) -> &mut i8, d: i32, (lo, hi): (i8, i8)) {
        let v = at(&mut self.arp.modifier);
        *v = (i32::from(*v) + d).clamp(i32::from(lo), i32::from(hi)) as i8;
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
                let mut fx = vec![
                    Effect::Patterns(self.patterns.clone()),
                    // A library line, so the report carries it home.
                    Effect::Note(format!("arp-pattern-saved: {name} = {}", p.to_text())),
                    Effect::Workspace(self.workspace_view(true)),
                ];
                if self.pattern_file {
                    fx.push(Effect::SavePatterns(self.library_text()));
                    // The owner's phrases with them: one read from a
                    // patterns file is in its own file from here on.
                    fx.extend(self.phrases_text().map(Effect::SavePhrases));
                } else {
                    fx.push(Effect::Note("the pattern stays in memory until the power goes off (no --data folder to keep it in)"
                        .to_string()));
                }
                fx
            }
            WsAction::Record(on) => self.set_recording(on),
            WsAction::RecordMode(m) => self.set_record_mode(m),
            WsAction::Undo => self.record_edit(false),
            WsAction::Clear => self.record_edit(true),
        }
    }
}

// ------------------------------------------------------------ pad layouts

/// The synth's pad layout feeding the arp (agreed hook with the app and
/// synth threads, docs/dev/design/scales.md "The arp on a pad layout"). Without
/// a layout the arp is fed pad slots 0..15 as before (a kit, or the synth's
/// plain chromatic pads: slot 0 counts as C). With one, each pad presses its
/// layout notes (a chord pad presses every note of its chord, root last),
/// and the arp's events carry MIDI notes for the synth to play as they are.
/// The audio loop's side (calling these when the synth is on the pads, and
/// the synth playing a note rather than a pad) is the app thread's wiring.
#[cfg_attr(not(test), allow(dead_code))]
impl App {
    /// The synth's pads now play `layout` (from `mpfree_scale::pads::layout`;
    /// the app thread calls it when the synth goes on the pads, and again on
    /// PAD BANK or a layout change), or `None` for slots (a kit). Allocates
    /// nothing. Pads held across a change between slots and notes are let go,
    /// since their old values mean something else now; a change from one
    /// layout to another keeps them, and each lets go of what it pressed.
    pub fn set_pad_layout(&mut self, layout: Option<[PadNotes; 16]>) {
        if self.pad_layout.is_some() != layout.is_some() {
            let press = layout.is_some() || self.arp_layout.is_some();
            self.press_change(press);
        }
        self.pad_layout = layout;
        self.arp_dirty = true;
        self.phrase_follows_pads();
    }

    /// The selected track's own synth's pad map while it plays notes, or
    /// None (a kit, the Drums engine, MIDI out, the shared synth): with
    /// one, the pads press its notes into the track's arp, which then
    /// works in notes (`TrackArps::set_on_notes`, the instrument
    /// redesign's step 5d), as the synth's layout does; nothing else
    /// about the pads changes. Allocates nothing. Pads held across a
    /// change between pads and notes are let go, as with the synth's.
    pub fn set_arp_layout(&mut self, layout: Option<[PadNotes; 16]>) {
        if self.arp_layout.is_some() != layout.is_some() {
            let press = layout.is_some() || self.pad_layout.is_some();
            self.press_change(press);
        }
        self.arp_layout = layout;
        self.phrase_follows_pads();
    }

    /// What the pads press into the arp: the synth's layout's notes, or
    /// the track's own synth's, or kit pads (None).
    pub(crate) fn press_layout(&self) -> Option<&[PadNotes; 16]> {
        self.pad_layout.as_ref().or(self.arp_layout.as_ref())
    }

    /// The pads go from pressing pads to notes or back (`press`: what they
    /// will press is notes): what they and the keys hold is let go first,
    /// since its old numbers mean something else now.
    fn press_change(&mut self, press: bool) {
        if self.press_layout().is_some() != press {
            for slot in 0..16 {
                self.release_pad(slot);
            }
            self.release_keys();
            self.kit_held = [None; 16];
        }
    }

    /// What the audio loop needs to treat the arps' notes as live playing
    /// while `App::arps` is lent to the time engine: a copy, so it borrows
    /// nothing. Allocates nothing.
    pub fn live_map(&self) -> LiveMap {
        LiveMap {
            pad_level: self.pad_level,
            half: self.settings.half_level,
            curve: self.settings.curve,
            layout: self.pad_layout.or(self.arp_layout),
        }
    }

    /// The arp's events carry MIDI notes (a pad layout is set) rather than
    /// pad slots.
    pub fn arp_plays_notes(&self) -> bool {
        self.pad_layout.is_some()
    }

    /// A keyboard's keys play notes into the armed track's arp: the pads
    /// press notes there, the shared synth's layout's or the track's own
    /// synth's (the instrument redesign's step 7). Otherwise a kit's keys
    /// hit its pads.
    pub fn keys_play_notes(&self) -> bool {
        self.press_layout().is_some()
    }

    /// The notes pad `slot` holds now (what it pressed, even if the layout
    /// changed since), for its pressure; none for a kit or a pad let go.
    pub fn pad_held_notes(&self, slot: usize) -> &[u8] {
        self.pad_pressed.get(slot).map_or(&[], |p| p.notes())
    }

    /// The first pad whose layout plays `note`, to flash on the screen.
    pub fn pad_of_note(&self, note: u8) -> Option<usize> {
        self.press_layout()?
            .iter()
            .position(|p| p.notes().contains(&note))
    }

    /// With a kit, `on` (bank × 16 + place) in place of the page's pad:
    /// a Loop pad's hit that waited for the launch line plays the pad it
    /// was made on, though the bank turned since (sampler R5.3b).
    pub(crate) fn press_pad_on(&mut self, slot: usize, on: Option<u8>, velocity: u8) {
        let Some(layout) = self.press_layout() else {
            // A kit: the pad of the page, kept until it is let go (a pad
            // pressed again first lets go of what it pressed).
            if slot >= self.kit_held.len() {
                return;
            }
            self.release_pad(slot);
            let pad = on.unwrap_or(self.kit_pad(slot) as u8);
            self.kit_held[slot] = Some(pad);
            self.kit_last = Some(slot);
            self.hold_kit_pad(pad, velocity);
            return;
        };
        let Some(&pad) = layout.get(slot) else { return };
        self.release_pad(slot);
        for &n in pad.notes() {
            let holds = &mut self.note_holds[usize::from(n)];
            if *holds == 0 {
                self.arps.engine_at_mut(self.track).press(n, velocity);
            }
            *holds = holds.saturating_add(1);
        }
        self.pad_pressed[slot] = pad;
    }

    /// A key on a MIDI keyboard (MIDI In or USB) while the pads play notes
    /// (a synth pad layout is set, or the track's own synth's pad map): it
    /// presses that note into the armed track's arp, as a pad holding it
    /// would, so the arp, Latch and Note
    /// Repeat work from a keyboard too. Keys and pads share the hold count:
    /// a note held on both ends when both let go. A key already held is
    /// ignored. While a pattern records in Free time the pads only write
    /// steps, so keys do nothing then. With a kit the app hits pads instead
    /// (main.rs, by the MPC's pad notes).
    pub fn key_down(&mut self, note: u8, velocity: u8) {
        let n = usize::from(note & 0x7F);
        if self.press_layout().is_none() || self.free_time_recording() || self.keys_held[n] {
            return;
        }
        self.keys_held[n] = true;
        let holds = &mut self.note_holds[n];
        if *holds == 0 {
            self.arps
                .engine_at_mut(self.track)
                .press(n as u8, velocity.max(1));
        }
        *holds = holds.saturating_add(1);
    }

    /// A key on a MIDI keyboard (MIDI In or USB) while the armed track
    /// plays a kit: it hits kit pad `pad` (0 is A1, up to 127,
    /// `mpfree_midi::profile::kit_pad_of_note`), whatever bank PAD BANK
    /// shows, through the track's arp as a pad would, so NOTE REPEAT and
    /// Latch work from a keyboard too, and it writes a step while a pattern
    /// records. Keys and pads share the hold count: a pad held on both ends
    /// when both let go. A key already held is ignored, and while a pattern
    /// records in Free time a key only writes its step, as a pad does. A
    /// key plays: it doesn't pick STEP SEQ's sound, mute a track or
    /// transpose, as a key on the synth doesn't (main.rs).
    pub fn kit_key_down(&mut self, pad: u8, velocity: u8) -> Vec<Effect> {
        let pad = pad & 0x7F;
        let p = usize::from(pad);
        if self.press_layout().is_some() || self.keys_held[p] {
            return Vec::new();
        }
        if !self.free_time_recording() {
            self.keys_held[p] = true;
            self.hold_kit_pad(pad, velocity.max(1));
        }
        self.record_press(pad, velocity)
    }

    /// The key on kit pad `pad` let go: the pad ends unless a pad of the
    /// MPC still holds it.
    pub fn kit_key_up(&mut self, pad: u8) -> Vec<Effect> {
        let pad = pad & 0x7F;
        if self.press_layout().is_some() {
            return Vec::new();
        }
        if std::mem::take(&mut self.keys_held[usize::from(pad)]) {
            self.let_go_kit_pad(pad);
        }
        self.record_release(pad)
    }

    /// A held key's pressure on kit pad `pad`, for the arp's notes on it.
    pub fn kit_key_pressure(&mut self, pad: u8, value: u8) {
        let pad = pad & 0x7F;
        if self.press_layout().is_none() && self.keys_held[usize::from(pad)] {
            self.arps.engine_at_mut(self.track).pressure(pad, value);
        }
    }

    /// A held key's pressure (poly or channel aftertouch) while the pads
    /// play notes: the arp's notes on it swell as a pad's do, and a
    /// Modifier on pressure follows it. (The sound gets it too, main.rs.)
    pub fn key_pressure(&mut self, note: u8, value: u8) {
        let n = note & 0x7F;
        if self.press_layout().is_some() && self.keys_held[usize::from(n)] {
            self.arps.engine_at_mut(self.track).pressure(n, value);
        }
    }

    /// A keyboard's mod wheel, breath controller or expression pedal, on
    /// the armed track: the arp's Modifier follows it when it is the
    /// Modifier's controller. The sound still gets it (main.rs).
    pub fn arp_controller(&mut self, ctl: Ctl, value: f32) {
        let source = match ctl {
            Ctl::ModWheel => ModSource::ModWheel,
            Ctl::Breath => ModSource::Breath,
            Ctl::Foot => ModSource::Expression,
            _ => return,
        };
        self.arps.engine_at_mut(self.track).control(source, value);
    }

    /// The sustain pedal on the armed track, with Prefs' Sustain pedal
    /// latches on: down while the arp plays (Repeat, Arp or Pattern) turns
    /// Latch on; up puts Latch back as it was, so the notes no longer held
    /// stop (the MPC's Sustain Pedal Latches, 3.10.1 guide p. 83). None
    /// when the pedal isn't the arp's: the sound sustains with it as
    /// before. A pedal comes up where it went down, whatever changed in
    /// between, so neither the arp nor a sound is left holding.
    pub fn arp_pedal(&mut self, ctl: Ctl, value: f32) -> Option<Vec<Effect>> {
        if ctl != Ctl::Sustain {
            return None;
        }
        if !Ctl::down(value) {
            return match self.pedal_down.take() {
                Some(PedalTo::Arp) => Some(self.pedal_lets_go()),
                _ => None,
            };
        }
        match self.pedal_down {
            // Still down (a half pedal sends many values): where it went.
            Some(PedalTo::Arp) => return Some(Vec::new()),
            Some(PedalTo::Sound) => return None,
            None => {}
        }
        if !self.settings.pedal_latches || !self.generating() {
            self.pedal_down = Some(PedalTo::Sound);
            return None;
        }
        self.pedal_down = Some(PedalTo::Arp);
        self.pedal_latch = Some(self.latched);
        if self.latched {
            return Some(Vec::new());
        }
        self.latched = true;
        let mut fx = vec![Effect::Note("Latch on (sustain pedal)".to_string())];
        fx.extend(self.arp_changed());
        Some(fx)
    }

    /// The sustain pedal's latch lets go: Latch as it was before the pedal
    /// went down. Also when the track changes or Prefs turns the pedal's
    /// latch off under it.
    pub(super) fn pedal_lets_go(&mut self) -> Vec<Effect> {
        if self.pedal_down == Some(PedalTo::Arp) {
            self.pedal_down = None;
        }
        let Some(before) = self.pedal_latch.take() else {
            return Vec::new();
        };
        if self.latched == before {
            return Vec::new();
        }
        self.latched = before;
        let mut fx = vec![Effect::Note("Latch off (sustain pedal)".to_string())];
        fx.extend(self.arp_changed());
        fx
    }

    /// Kit pad `pad` held by one more pad or key: the arp gets it once.
    fn hold_kit_pad(&mut self, pad: u8, velocity: u8) {
        let holds = &mut self.note_holds[usize::from(pad & 0x7F)];
        if *holds == 0 {
            self.arps.engine_at_mut(self.track).press(pad, velocity);
        }
        *holds = holds.saturating_add(1);
    }

    /// One pad or key on kit pad `pad` let go: the arp lets go of it with
    /// the last.
    fn let_go_kit_pad(&mut self, pad: u8) {
        let holds = &mut self.note_holds[usize::from(pad & 0x7F)];
        *holds = holds.saturating_sub(1);
        if *holds == 0 {
            self.arps.engine_at_mut(self.track).release(pad);
        }
    }

    /// The key let go; the note ends unless a pad still holds it.
    pub fn key_up(&mut self, note: u8) {
        let n = usize::from(note & 0x7F);
        if !std::mem::take(&mut self.keys_held[n]) {
            return;
        }
        let holds = &mut self.note_holds[n];
        *holds = holds.saturating_sub(1);
        if *holds == 0 {
            self.arps.engine_at_mut(self.track).release(n as u8);
        }
    }

    /// Every held key let go (the track or the pad layout changes).
    pub(crate) fn release_keys(&mut self) {
        for n in 0..128u8 {
            self.key_up(n);
        }
    }

    pub(crate) fn release_pad(&mut self, slot: usize) {
        if self.press_layout().is_none() {
            // The kit pad it pressed, once.
            if let Some(pad) = self.kit_held.get_mut(slot).and_then(Option::take) {
                self.let_go_kit_pad(pad);
            }
            return;
        }
        let Some(pad) = self.pad_pressed.get_mut(slot).map(std::mem::take) else {
            return;
        };
        for &n in pad.notes() {
            let holds = &mut self.note_holds[usize::from(n)];
            *holds = holds.saturating_sub(1);
            if *holds == 0 {
                self.arps.engine_at_mut(self.track).release(n);
            }
        }
    }
}

// ------------------------------------------------------------ tracks

/// One track's arp as the panel, the NOTE REPEAT band and Latch see it. The
/// engine itself (notes held and latched, its pattern, its step) stays in
/// `App::arps`; this is what the App keeps beside it.
#[derive(Clone, Copy, Debug)]
pub struct TrackUi {
    arp: Settings,
    arp_mode: Mode,
    rate: Rate,
    latched: bool,
    scale_root: u8,
    scale_pick: usize,
    swing_from_transport: bool,
    pattern_index: usize,
    pub(super) pattern: Pattern,
    mutations: u32,
    /// The track's phrase, as a place in `App::phrases` (the phrase itself
    /// is in the list and in the track's engine: `app/arp/phrases.rs`).
    phrase_index: usize,
}

impl TrackUi {
    /// A track's arp as New leaves it: the defaults on `pattern` (the
    /// first built-in one).
    pub(super) fn fresh(pattern: Pattern) -> TrackUi {
        TrackUi {
            pattern,
            ..Default::default()
        }
    }
}

impl Default for TrackUi {
    fn default() -> Self {
        TrackUi {
            arp: Settings::default(),
            arp_mode: Mode::Repeat,
            rate: Rate::Sixteenth,
            latched: false,
            scale_root: 0,
            scale_pick: 0,
            swing_from_transport: true,
            pattern_index: 0,
            pattern: Pattern::default(),
            mutations: 0,
            phrase_index: 0,
        }
    }
}

impl App {
    /// The armed track's arp as the panel holds it now.
    fn current_ui(&self) -> TrackUi {
        TrackUi {
            arp: self.arp,
            arp_mode: self.arp_mode,
            rate: self.settings.rate,
            latched: self.latched,
            scale_root: self.scale_root,
            scale_pick: self.scale_pick,
            swing_from_transport: self.swing_from_transport,
            pattern_index: self.pattern_index,
            pattern: self.pattern,
            mutations: self.mutations,
            phrase_index: self.phrase_index,
        }
    }

    /// The panel takes `u` for the armed track, and its engine follows.
    fn take_ui(&mut self, u: TrackUi) -> Vec<Effect> {
        self.arp = u.arp;
        self.arp_mode = u.arp_mode;
        self.settings.rate = u.rate;
        self.latched = u.latched;
        self.scale_root = u.scale_root;
        self.scale_pick = u.scale_pick;
        self.swing_from_transport = u.swing_from_transport;
        self.pattern_index = u.pattern_index;
        self.mutations = u.mutations;
        let _ = self.set_pattern(u.pattern);
        self.phrase_index = u.phrase_index;
        self.send_phrase();
        self.arp_changed()
    }

    /// The armed track, whose arp the pads play and the arp screens show.
    pub fn track(&self) -> usize {
        self.track
    }

    /// The armed track changed (owner decision 2: one arp per track). Pads
    /// held on the old track are let go there, unless its arp is latched,
    /// which keeps playing. The panel, the band and Latch then show the new
    /// track's arp, as it was left. Allocates nothing.
    pub fn set_track(&mut self, track: usize) -> Vec<Effect> {
        let track = track.min(mpfree_seq::MAX_TRACKS - 1);
        if track == self.track {
            return Vec::new();
        }
        // The sustain pedal's latch was the old track's: it lets go there,
        // and the pedal starts again on the new one.
        let mut pedal = self.pedal_lets_go();
        self.pedal_down = None;
        // What the pads play stays (the instrument redesign's rule 2): the
        // new track's instrument while they follow the track, the synth
        // chosen for them otherwise (app/pads_focus.rs).
        if self.press_layout().is_some() {
            for slot in 0..16 {
                self.release_pad(slot);
            }
            self.release_keys();
        } else {
            // A kit: what the old track's engine holds is the kit pads the
            // pads and keys hold, in any bank. Their hold counts start
            // again on the new track.
            if !self.latched {
                for pad in 0..mpfree_midi::profile::KIT_PADS as u8 {
                    if self.arps.engine_at(self.track).active().contains(pad) {
                        self.arps.engine_at_mut(self.track).release(pad);
                    }
                }
            }
            self.keys_held = [false; 128];
            self.note_holds = [0; 128];
        }
        self.track_ui[self.track] = self.current_ui();
        self.track = track;
        let mut fx = self.take_ui(self.track_ui[track]);
        fx.push(Effect::Note(format!(
            "arp: track {} ({})",
            track + 1,
            mode_label(self.arp_mode)
        )));
        pedal.append(&mut fx);
        pedal
    }

    /// Track `track`'s arp setting `id` (`mpfree_param::arp`) as MIDI
    /// Learn reads it: its description and its value now (the rate as its
    /// place in [`Rate::ALL`]). The armed track's as the panel holds it,
    /// another's as it was left.
    pub fn arp_param(&self, track: usize, id: u16) -> Option<(ParamInfo, f32)> {
        let info = arp_info(id)?;
        if id == arp_ids::MODIFIER {
            self.track_ui.get(track)?;
            return Some((info, self.arps.engine_at(track).modifier() * 100.0));
        }
        let (s, rate) = if track == self.track {
            (&self.arp, self.settings.rate)
        } else {
            let u = self.track_ui.get(track)?;
            (&u.arp, u.rate)
        };
        let v = match id {
            arp_ids::RATE => Rate::ALL.iter().position(|r| *r == rate).unwrap_or(0) as f32,
            arp_ids::GATE => f32::from(s.gate),
            arp_ids::SWING => f32::from(s.swing),
            _ => f32::from(s.octaves),
        };
        Some((info, v))
    }

    /// A learned control set track `track`'s arp setting `id` to `value`
    /// (in [`App::arp_param`]'s units): the armed track's as its Q-Link
    /// sets it, so the panel and the band follow; another track's arp
    /// plays on with it, and the panel shows it when that track is armed.
    /// Swing turned this way stops following the transport, as a Q-Link
    /// turn does. Nothing for a setting it already has.
    pub fn set_arp_param(&mut self, track: usize, id: u16, value: f32) -> Vec<Effect> {
        let Some((info, now)) = self.arp_param(track, id) else {
            return Vec::new();
        };
        let value = info.clamp(value);
        // The Modifier: the knob pushes that track's arp while it is the
        // Modifier's controller; the screens redraw on the next poll.
        if id == arp_ids::MODIFIER {
            self.arps
                .engine_at_mut(track)
                .control(ModSource::Knob, value / 100.0);
            return Vec::new();
        }
        if value == now {
            return Vec::new();
        }
        let set = |s: &mut Settings, rate: &mut Rate| {
            let v = value as usize;
            match id {
                arp_ids::RATE => *rate = Rate::ALL[v.min(Rate::ALL.len() - 1)],
                arp_ids::GATE => s.gate = v as u8,
                arp_ids::SWING => s.swing = v as u8,
                _ => s.octaves = v as u8,
            }
            *s = s.clamped();
        };
        if track == self.track {
            let mut s = self.arp;
            s.rate = self.settings.rate;
            let mut rate = s.rate;
            set(&mut s, &mut rate);
            s.rate = rate;
            return self.arp_action(ArpAction::Settings(s));
        }
        let Some(u) = self.track_ui.get_mut(track) else {
            return Vec::new();
        };
        set(&mut u.arp, &mut u.rate);
        if id == arp_ids::SWING {
            u.swing_from_transport = false;
        }
        let u = *u;
        let e = self.arps.engine_at_mut(track);
        let mut s = u.arp;
        s.mode = e.settings().mode;
        s.rate = u.rate;
        e.set_settings(s);
        e.set_own_swing(!u.swing_from_transport);
        self.arp_dirty = true;
        Vec::new()
    }
}

/// The names of [`Rate::ALL`], for MIDI Learn's description of the rate.
const RATE_NAMES: [&str; 7] = {
    let mut n = [""; 7];
    let mut i = 0;
    while i < 7 {
        n[i] = Rate::ALL[i].label();
        i += 1;
    }
    n
};

/// What an arp setting is to MIDI Learn (`Owner::Arp`, ids in
/// `mpfree_param::arp`): its name, range and units, as the arp's Q-Link
/// sliders show it. None for an id that isn't one.
pub fn arp_info(id: u16) -> Option<ParamInfo> {
    let info = |name, min, max, default, unit, curve| ParamInfo {
        id,
        name,
        group: "Arp",
        min,
        max,
        default,
        unit,
        curve,
        readonly: false,
    };
    Some(match id {
        arp_ids::RATE => info(
            "Rate",
            0.0,
            (RATE_NAMES.len() - 1) as f32,
            3.0,
            Unit::None,
            Curve::Choice(&RATE_NAMES),
        ),
        arp_ids::GATE => info("Gate", 1.0, 100.0, 50.0, Unit::Percent, Curve::Stepped),
        arp_ids::SWING => info("Swing", 50.0, 75.0, 50.0, Unit::Percent, Curve::Stepped),
        arp_ids::OCTAVES => info("Octaves", 1.0, 4.0, 1.0, Unit::None, Curve::Stepped),
        arp_ids::MODIFIER => info("Modifier", 0.0, 100.0, 0.0, Unit::Percent, Curve::Linear),
        _ => return None,
    })
}

/// The live treatment's inputs, copied out of the App each period
/// ([`App::live_map`]).
#[derive(Clone, Copy, Debug)]
pub struct LiveMap {
    pad_level: super::PadLevel,
    half: u8,
    curve: super::Curve,
    layout: Option<[PadNotes; 16]>,
}

impl LiveMap {
    /// The velocity a hit plays at, after Full or Half Level and the curve
    /// (as `App::velocity`).
    pub fn velocity(&self, v: u8) -> u8 {
        match self.pad_level {
            super::PadLevel::Full => 127,
            super::PadLevel::Half => self.half,
            super::PadLevel::Off => self.curve.apply(v),
        }
    }

    /// The arps' notes are MIDI notes (a synth pad layout), not pad slots.
    pub fn plays_notes(&self) -> bool {
        self.layout.is_some()
    }

    /// The first pad whose layout plays `note`.
    pub fn pad_of_note(&self, note: u8) -> Option<usize> {
        self.layout
            .as_ref()?
            .iter()
            .position(|p| p.notes().contains(&note))
    }
}
