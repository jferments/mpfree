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

use super::{App, Effect, Item, View};
use mpfree_arp::scale::{Scale, Snap, NOTE_NAMES, SCALES};
use mpfree_arp::{Chord, Held, Turns, Library, Mode, Order, Pattern, Rate, RecordMode, Recorder, Settings, Step, Wrote};
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
    /// Recording, or the next recording, is in Free time.
    pub free_time: bool,
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
}

/// The Options menu opened from the Arp panel: the tempo, then the arp's
/// own settings. (Rate, mode, gate, swing and octaves are on the panel.)
pub(super) const ARP_OPTIONS: [Item; 11] = [Item::Tempo, Item::Arp(ArpItem::Accent), Item::Arp(ArpItem::Scale),
                                            Item::Arp(ArpItem::Root), Item::Arp(ArpItem::Snap), Item::Arp(ArpItem::Chord),
                                            Item::Arp(ArpItem::Run), Item::Arp(ArpItem::Turns), Item::Arp(ArpItem::Strum),
                                            Item::Arp(ArpItem::Retrigger), Item::Arp(ArpItem::SwingFrom)];

/// Accent level steps by this much per press or wheel click.
const ACCENT_STEP: i32 = 8;
/// Strum steps by this many percent.
const STRUM_STEP: i32 = 10;

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
    }
}

impl App {
    /// What the pads play through now.
    pub(super) fn engine_mode(&self) -> Mode {
        if self.free_time_recording() {
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
        }
    }

    /// Recording on the pads in Free time.
    pub fn free_time_recording(&self) -> bool {
        self.recorder.is_some_and(|r| r.mode() == RecordMode::FreeTime)
    }

    fn workspace_view(&self, open: bool) -> WorkspaceView {
        WorkspaceView { name: self.pattern_name(), pattern: self.pattern, open, recording: self.recorder.is_some(),
                        free_time: self.record_mode == RecordMode::FreeTime }
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
            ArpAction::OpenPanel if self.view != View::Arp => self.toggle_panel(),
            ArpAction::OpenPanel => Vec::new(),
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
            let notes = !self.engine.active().is_empty();
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
                RecordMode::Step => format!("recording a pattern on the pads: {} steps, one per press", r.length()),
                RecordMode::FreeTime => {
                    format!("recording in Free time: {} steps loop; presses land on the nearest step", r.length())
                }
            }));
        } else if let Some(r) = self.recorder.take() {
            if let Some(p) = self.recorded_pattern(&r) {
                self.set_pattern(p);
            }
            fx.push(Effect::Note(format!("recording stopped: {}", self.pattern.to_text())));
        } else {
            return fx;
        }
        let mut w = self.workspace_view(false);
        w.free_time = self.free_time_recording() || (!on && self.record_mode == RecordMode::FreeTime);
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
        let Some(r) = self.recorder else { return Vec::new() };
        if let Some(p) = self.recorded_pattern(&r) {
            self.set_pattern(p);
        }
        vec![Effect::Recorded(self.pattern, next.min(self.pattern.len().saturating_sub(1)))]
    }

    /// A pad press while recording. Step time writes the next step; Free
    /// time writes the step nearest the pattern's position now.
    pub(super) fn record_press(&mut self, slot: u8, velocity: u8) -> Vec<Effect> {
        let v = self.velocity(velocity);
        let position = self.engine.position();
        let Some(r) = self.recorder.as_mut() else { return Vec::new() };
        let wrote = match r.mode() {
            RecordMode::Step => r.press(slot, v),
            RecordMode::FreeTime => r.press_at(slot, v, position),
        };
        let (full, written, length, free) = (r.is_full(), r.written(), r.length(), r.mode() == RecordMode::FreeTime);
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
        let position = self.engine.position();
        let Some(r) = self.recorder.as_mut() else { return Vec::new() };
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
        let Some(r) = self.recorder.as_mut() else { return Vec::new() };
        let done = if clear {
            r.clear();
            true
        } else {
            r.undo()
        };
        if !done {
            return Vec::new();
        }
        let next = if r.mode() == RecordMode::Step { r.written().min(r.length() - 1) } else { 0 };
        let mut fx = self.show_recorded(next);
        fx.push(Effect::Note(if clear { "recording cleared" } else { "last step taken back" }.to_string()));
        fx
    }

    /// Patterns from a library file (the `--patterns` option): one with a
    /// built-in's name replaces it, the others come after the built-ins. A
    /// file with any bad line is not used, and every bad line is logged.
    pub fn load_patterns(&mut self, text: &str) -> Vec<Effect> {
        let lib = match Library::parse(text) {
            Ok(lib) => lib,
            Err(errors) => {
                let mut fx: Vec<Effect> =
                    errors.iter().take(20).map(|e| Effect::Note(format!("arp patterns file, {e}"))).collect();
                fx.push(Effect::Note(format!("arp patterns file not used ({} bad lines); the built-in patterns stay",
                                             errors.len())));
                return fx;
            }
        };
        let (mut replaced, mut added) = (0, 0);
        let mut scales = 0;
        for (name, up, down) in lib.scales() {
            match self.custom_scales.iter_mut().find(|(n, ..)| n.eq_ignore_ascii_case(name)) {
                Some(slot) => *slot = (name.to_string(), up, down),
                None => self.custom_scales.push((name.to_string(), up, down)),
            }
            scales += 1;
        }
        for (name, p) in lib.iter() {
            match self.patterns.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
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
        // A scale of the owner's in use may have changed.
        if self.scale_pick >= SCALES.len() {
            self.apply_scale_pick();
            self.update_engine();
        }
        let mut fx = self.load_pattern(self.pattern_index);
        fx.push(Effect::Patterns(self.patterns.clone()));
        fx.push(Effect::Note(format!("arp patterns file: {added} added, {replaced} built-in ones replaced, \
                                      {scales} scales of your own")));
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
        format!("# MPFree arp patterns, saved by the pad player. One per line: Name = pattern;\n\
                 # your scales after [scales]. The syntax is in docs/project/arpeggiator.md.\n{}", lib.to_text())
    }

    // ------------------------------------------------------------ transport

    /// The transport's clock for the next [`App::process`]: called once a
    /// period, before it, while the transport plays (the time engine gives
    /// the first span of the period). None, or no call, and the arp
    /// free-runs at the app's tempo.
    #[allow(dead_code)] // the time engine calls it from round 7 (the transport thread wires main.rs)
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
                self.engine.sync(&c);
            }
            None if self.engine.synced() => {
                self.engine.sync(&mpfree_arp::Clock { playing: false, ..mpfree_arp::Clock::playing(0, 0, self.settings.bpm) });
                self.engine.set_tempo(self.settings.bpm);
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
            i => self.custom_scales.get(i - SCALES.len()).map(|(n, u, d)| (n.as_str(), *u, *d)),
        }
    }

    fn scale_choice(&self) -> usize {
        if self.arp.scale.is_some() { self.scale_pick } else { 0 }
    }

    /// The arp's scale from the Scale row's choice and the root.
    fn apply_scale_pick(&mut self) {
        self.arp.scale = self.scale_entry(self.scale_pick).map(|(_, up, down)| Scale::with_down(self.scale_root, up, down));
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
            ArpItem::Scale => ("Scale", match self.scale_entry(self.scale_choice()) {
                None => "Off".to_string(),
                Some((name, ..)) => name.to_string(),
            }),
            ArpItem::Root => ("Scale root", NOTE_NAMES[usize::from(self.scale_root)].to_string()),
            ArpItem::Snap => ("Out-of-key notes", snap_text(s.snap).to_string()),
            ArpItem::Chord => ("Chord on each pad", s.chord.label().to_string()),
            ArpItem::Run => ("Scale run", if s.run { "On" } else { "Off" }.to_string()),
            ArpItem::Turns => ("Chords take turns", s.turns.label().to_string()),
            ArpItem::Strum => ("Strum", if s.strum == 0 { "Off".to_string() } else { format!("{} % of a step", s.strum) }),
            ArpItem::Retrigger => ("New notes restart", if s.retrigger { "On" } else { "Off" }.to_string()),
            ArpItem::SwingFrom => ("Swing from", if self.swing_from_transport { "Transport" } else { "Arp" }.to_string()),
        }
    }

    pub(super) fn arp_item_help(item: ArpItem) -> Vec<&'static str> {
        match item {
            ArpItem::Accent => vec!["How loud accented steps play: pattern steps",
                                    "marked with an accent, and steps recorded",
                                    "from hits of 110 or more. FULL LEVEL and",
                                    "Half Level override it."],
            ArpItem::Scale => vec!["Keeps the arp's notes in key: a pad off the",
                                   "scale moves into it (see Out-of-key notes),",
                                   "and the octaves and Degree lanes move along",
                                   "the scale. Scales from your patterns file",
                                   "come after the built-in ones. For a synth on",
                                   "the pads; leave it Off for drum kits, whose",
                                   "pads are sounds, not notes."],
            ArpItem::Root => vec!["The scale's first note. Pad 1 counts as C,",
                                  "pad 2 as C# and so on up."],
            ArpItem::Snap => vec!["With a scale on, what a held note outside it",
                                  "plays: the scale note below it, the one above,",
                                  "the nearer of the two (below when they are as",
                                  "near), or nothing: its steps stay silent."],
            ArpItem::Chord => vec!["Each pad you hold plays a chord built from",
                                   "the scale (from the major scale on the pad",
                                   "without one), and the arp runs over all its",
                                   "notes: one pad is enough for an arpeggio.",
                                   "For a synth on the pads, not drum kits."],
            ArpItem::Run => vec!["Arp and Pattern: the steps go through every",
                                 "scale note between your lowest and highest",
                                 "held notes (an octave up from one note), in",
                                 "the order you picked. Every semitone without",
                                 "a scale."],
            ArpItem::Turns => vec!["Hold two or more pads and they take turns",
                                   "instead of sounding together: the steps play",
                                   "from one pad (its chord, with Chord on each",
                                   "pad), then the next, in the order you pressed",
                                   "them, a beat or a bar each. Hold Am, F, C and",
                                   "G to arpeggiate that progression."],
            ArpItem::Strum => vec!["Spreads the notes of a chord step across the",
                                   "step, lowest first, like a strum. Off plays",
                                   "them together."],
            ArpItem::Retrigger => vec!["On: each new pad press starts the arp's order",
                                       "and the pattern again from the first step.",
                                       "Off: they carry on where they are."],
            ArpItem::SwingFrom => vec!["Transport: while the sequencer plays with",
                                       "swing on, the arp swings with it, on the",
                                       "sequencer's grid. Arp: always the arp's own",
                                       "swing (Q-Link 3). Stopped, or with the",
                                       "sequencer's swing off, it is the arp's own."],
        }
    }

    /// One of the arp's option rows stepped by `d` (-1 or +1).
    pub(super) fn step_arp_item(&mut self, item: ArpItem, d: i32) -> Vec<Effect> {
        let before = (self.arp, self.scale_root, self.scale_pick);
        match item {
            ArpItem::Accent => self.arp.accent = (i32::from(self.arp.accent) + ACCENT_STEP * d).clamp(1, 127) as u8,
            ArpItem::Scale => {
                self.scale_pick = (self.scale_choice() as i32 + d).clamp(0, self.scale_choices() as i32 - 1) as usize;
                self.apply_scale_pick();
            }
            ArpItem::Root => {
                self.scale_root = (i32::from(self.scale_root) + d).clamp(0, 11) as u8;
                self.arp.scale = self.arp.scale.map(|s| Scale::with_down(self.scale_root, s.mask, s.down));
            }
            ArpItem::Snap => {
                let at = Snap::ALL.iter().position(|&x| x == self.arp.snap).unwrap_or(0) as i32;
                self.arp.snap = Snap::ALL[(at + d).clamp(0, Snap::ALL.len() as i32 - 1) as usize];
            }
            ArpItem::Chord => self.arp.chord = step_in(&Chord::ALL, self.arp.chord, d),
            ArpItem::Run => self.arp.run = d > 0,
            ArpItem::Turns => self.arp.turns = step_in(&Turns::ALL, self.arp.turns, d),
            ArpItem::Strum => self.arp.strum = (i32::from(self.arp.strum) + STRUM_STEP * d).clamp(0, 100) as u8,
            ArpItem::Retrigger => self.arp.retrigger = d > 0,
            ArpItem::SwingFrom => {
                let t = d > 0;
                if t == self.swing_from_transport {
                    return Vec::new();
                }
                self.swing_from_transport = t;
                return vec![Effect::Note(format!("arp swing from: {}", if t { "transport" } else { "arp" }))];
            }
        }
        if (self.arp, self.scale_root, self.scale_pick) == before {
            return Vec::new();
        }
        self.update_engine();
        let note = match item {
            ArpItem::Accent => format!("arp accent level: {}", self.arp.accent),
            ArpItem::Scale | ArpItem::Root => format!("arp scale: {}", self.scale_text()),
            ArpItem::Snap => format!("arp out-of-key notes: {}", snap_text(self.arp.snap).to_ascii_lowercase()),
            ArpItem::Chord => format!("arp chord on each pad: {}", self.arp.chord.label().to_ascii_lowercase()),
            ArpItem::Run => format!("arp scale run: {}", if self.arp.run { "on" } else { "off" }),
            ArpItem::Turns => format!("arp chords take turns: {}", self.arp.turns.label().to_ascii_lowercase()),
            ArpItem::Strum => format!("arp strum: {} %", self.arp.strum),
            ArpItem::Retrigger => format!("arp new notes restart: {}", if self.arp.retrigger { "on" } else { "off" }),
            ArpItem::SwingFrom => unreachable!("returned above"),
        };
        vec![Effect::Note(note)]
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
                let mut fx = vec![Effect::Patterns(self.patterns.clone()),
                                  // A library line, so the report carries it home.
                                  Effect::Note(format!("arp-pattern-saved: {name} = {}", p.to_text())),
                                  Effect::Workspace(self.workspace_view(true))];
                if self.pattern_file {
                    fx.push(Effect::SavePatterns(self.library_text()));
                } else {
                    fx.push(Effect::Note("the pattern stays in memory until the power goes off (no patterns file)"
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
