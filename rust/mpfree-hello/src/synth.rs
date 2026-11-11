//! The pad player's synth: MPFree's Poly or FM (`mpfree-synth`) on the
//! pads, edited on the Synth tab (`mpfree-synth-ui`).
//!
//! - The kit list ends with one more entry, "Synth: Poly" or "Synth: FM".
//!   Picking it, or opening the Synth tab, makes the pads play the synth.
//!   The Synth tab's options (MENU) pick the engine; a note still ringing
//!   on the other engine finishes its release.
//! - The pads are laid out by `mpfree_scale::pads` (owner, 2026-11-11):
//!   with no scale, pad 1 plays C3 and each pad after it a semitone higher
//!   (C3 to D#4), as in round 6. With a scale, each bank has its own
//!   layout: Chromatic (the scale lit), In key, or Chord pads, where a pad
//!   plays three or four notes. SHIFT + wheel on the Play page moves the
//!   pads an octave (C0 to C6 at pad 1).
//! - Note Repeat, Latch, Full and Half Level and the velocity curve work as
//!   they do for drums: the note engine (`mpfree-arp`) hands the synth its
//!   notes on their exact frames. Pad pressure goes to every note its pad
//!   plays. Two pads can share a note (two chords); the note ends when the
//!   last pad holding it is let go.
//! - The audio thread owns both nodes. The Synth tab (on the screen thread)
//!   keeps a copy of the values and sends changes, which apply from the
//!   next period.
//! - [`Synth::render`] adds the synth to the period after the sampler. It
//!   never allocates. A block with a non-finite sample is dropped, that
//!   node reset and the reset counted, as the node interface asks of a host.
//! - STOP fades the synth out over 10 ms, then resets it, so no release
//!   rings on.
//!
//! Everything that allocates (`new`, `values`, `state`, the labels and log
//! lines) runs before the sound starts or for the screen and the log, never
//! inside `render`.

use mpfree_arp::scale::pads::{self, InKeyRows, Pad, PadLayout, PadRole, PadSetup, DEFAULT_BANKS, MAX_PAD_NOTES, PADS};
use mpfree_arp::scale::Scale;
use mpfree_node::{Ctx, Node, NodeEvent, ParamInfo, Timed};
use mpfree_synth::fm_params::FmPreset;
use mpfree_synth::{Fm, Poly};
use mpfree_synth_ui::format::value_line;
use mpfree_synth_ui::SynthState;
use mpfree_ui::Color;
use std::time::{Duration, Instant};

/// The start of the kit list's entry for the synth ("Synth: Poly").
pub const KIT_PREFIX: &str = "Synth: ";
/// The preset each engine starts with.
pub const START_PRESET: &str = "Pluck";
pub const FM_START_PRESET: &str = "Glass Keys";
/// How far SHIFT + wheel moves the pads: pad 1 from C0 to C6.
pub const MIN_OCTAVE: i32 = -3;
pub const MAX_OCTAVE: i32 = 3;
/// Resting pad colours on the controller, all from the colours Akai's app
/// was seen sending (`akai::RESTING_COLOURS`). With no scale: C, the other
/// white keys, and the black keys. With a scale: the root, the scale's
/// other notes, and notes outside it.
pub const PAD_C: [u8; 3] = [0x2C, 0x00, 0x3F];
pub const PAD_WHITE: [u8; 3] = [0x00, 0x2A, 0x3F];
pub const PAD_BLACK: [u8; 3] = [0x00, 0x08, 0x3F];
/// Note events one period can hold, per engine; more are counted and dropped.
pub const MAX_EVENTS: usize = 128;
/// Frames a node renders at a time into the synth's own buffers.
const SCRATCH: usize = 256;
/// STOP's fade, in frames (10 ms at 44.1 kHz).
const FADE: u32 = 441;
/// How long a parameter must rest before its change is logged.
const LOG_AFTER: Duration = Duration::from_secs(1);
/// Pad banks A to H.
pub const BANKS: usize = 8;

/// A note's name: 48 is C3, 61 is C#4.
pub fn note_name(note: u8) -> String {
    mpfree_arp::scale::note_name(i16::from(note))
}

/// A black key (C#, D#, F#, G#, A#).
fn black(note: u8) -> bool {
    matches!(note % 12, 1 | 3 | 6 | 8 | 10)
}

/// The synth engines the pads can play.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Engine {
    /// The subtractive synth (round 6).
    #[default]
    Poly,
    /// The six-operator FM synth.
    Fm,
}

impl Engine {
    pub const ALL: [Engine; 2] = [Engine::Poly, Engine::Fm];

    pub fn name(self) -> &'static str {
        match self {
            Engine::Poly => "Poly",
            Engine::Fm => "FM",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// How the synth's pads are set up, from the Synth tab's options: the
/// engine, the scale (None is Off), how In key fills its rows, and each
/// bank's layout as (layout, octaves above the pads' octave).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadSettings {
    pub engine: Engine,
    pub scale: Option<Scale>,
    pub rows: InKeyRows,
    pub banks: [(PadLayout, i8); BANKS],
    pub bank: usize,
}

impl Default for PadSettings {
    fn default() -> Self {
        PadSettings { engine: Engine::Poly, scale: None, rows: InKeyRows::Sequence, banks: DEFAULT_BANKS, bank: 0 }
    }
}

impl PadSettings {
    /// The bank playing's layout.
    pub fn layout(&self) -> PadLayout {
        self.banks[self.bank % BANKS].0
    }

    /// Sets the bank playing's layout.
    pub fn set_layout(&mut self, layout: PadLayout) {
        self.banks[self.bank % BANKS].0 = layout;
    }

    /// The pads' notes for the bank playing, `octave` octaves from pad 1 =
    /// C3 (or the root in octave 3). With no scale every bank is plain
    /// Chromatic, its octave offset too.
    pub fn setup(&self, octave: i32) -> PadSetup {
        let (layout, up) = self.banks[self.bank % BANKS];
        let up = if self.scale.is_some() { up } else { 0 };
        PadSetup { layout, rows: self.rows, scale: self.scale, octave: (octave + i32::from(up)).clamp(-9, 9) as i8 }
    }
}

/// Counters for the status line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub notes: u64,
    pub peak_voices: usize,
    /// Note events that did not fit in a period, plus notes the synth had
    /// no voice for (both should stay 0).
    pub dropped: u64,
    /// Blocks dropped for a non-finite sample (should stay 0).
    pub resets: u64,
}

/// The notes one pad started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Held {
    notes: [u8; MAX_PAD_NOTES],
    len: u8,
}

impl Held {
    fn notes(&self) -> &[u8] {
        &self.notes[..usize::from(self.len)]
    }
}

pub struct Synth {
    poly: Poly,
    fm: Fm,
    fm_presets: Vec<FmPreset>,
    rate: u32,
    /// Note events for each engine (Poly, FM) this period.
    events: [[Timed; MAX_EVENTS]; 2],
    len: [usize; 2],
    dropped_events: u64,
    left: [f32; SCRATCH],
    right: [f32; SCRATCH],
    settings: PadSettings,
    octave: i32,
    /// The pads' notes, worked out when a setting changes.
    pads: [Pad; PADS],
    /// The notes each pad started, so its note-off ends those notes even
    /// after the layout or octave changed.
    playing: [Held; PADS],
    /// How many pads hold each note now.
    holders: [u8; 128],
    /// Each pad's last pressure, 0 to 127.
    press: [u8; PADS],
    /// The preset each engine has loaded.
    presets: [usize; 2],
    /// Frames left in STOP's fade, while `fading`.
    fade: u32,
    fading: bool,
    notes: u64,
    peak_voices: usize,
    resets: u64,
    /// Indices of the read-only parameters (meters), per engine.
    meters: [Vec<usize>; 2],
}

fn meters_of(node: &dyn Node) -> Vec<usize> {
    node.params().iter().enumerate().filter(|(_, p)| p.readonly).map(|(i, _)| i).collect()
}

impl Synth {
    /// Poly and FM at `rate`, with [`START_PRESET`] and [`FM_START_PRESET`]
    /// loaded and Poly on the pads. Allocates: call it before the sound
    /// starts.
    pub fn new(rate: u32) -> Synth {
        let mut poly = Poly::new(rate);
        let preset = Poly::presets().iter().position(|p| p.name == START_PRESET).unwrap_or(0);
        poly.load_preset(&Poly::presets()[preset]);
        let mut fm = Fm::new(rate);
        let fm_presets = Fm::presets();
        let fm_preset = fm_presets.iter().position(|p| p.name == FM_START_PRESET).unwrap_or(0);
        if let Some(p) = fm_presets.get(fm_preset) {
            fm.load_preset(p);
        }
        let meters = [meters_of(&poly), meters_of(&fm)];
        let settings = PadSettings::default();
        Synth { poly, fm, fm_presets, rate, events: [[Timed::new(0, NodeEvent::AllOff); MAX_EVENTS]; 2], len: [0; 2],
                dropped_events: 0, left: [0.0; SCRATCH], right: [0.0; SCRATCH], settings, octave: 0,
                pads: pads::layout(&settings.setup(0)), playing: [Held::default(); PADS], holders: [0; 128],
                press: [0; PADS], presets: [preset, fm_preset], fade: 0, fading: false, notes: 0, peak_voices: 0,
                resets: 0, meters }
    }

    fn node(&self) -> &dyn Node {
        match self.settings.engine {
            Engine::Poly => &self.poly,
            Engine::Fm => &self.fm,
        }
    }

    fn node_mut(&mut self) -> &mut dyn Node {
        match self.settings.engine {
            Engine::Poly => &mut self.poly,
            Engine::Fm => &mut self.fm,
        }
    }

    pub fn engine(&self) -> Engine {
        self.settings.engine
    }

    // ------------------------------------------------------------ the pads

    pub fn settings(&self) -> PadSettings {
        self.settings
    }

    /// The pads' notes for the bank playing.
    pub fn setup(&self) -> PadSetup {
        self.settings.setup(self.octave)
    }

    /// New pad settings (from the Synth tab's options). Notes already held
    /// end on their own pad's note-off. A new engine takes the pads; the
    /// old one's notes get their note-off now and finish their release.
    /// Returns whether the engine changed. Never allocates.
    pub fn set_settings(&mut self, settings: PadSettings) -> bool {
        let switched = settings.engine != self.settings.engine;
        if switched {
            self.all_off();
        }
        self.settings = settings;
        self.settings.bank %= BANKS;
        self.pads = pads::layout(&self.setup());
        switched
    }

    /// Every pad's notes now, for the arp ([`crate::app::App::set_pad_layout`]):
    /// with it the arp's events carry MIDI notes, played with
    /// [`Synth::note_on_note`].
    pub fn pad_layout(&self) -> [Pad; PADS] {
        self.pads
    }

    /// The notes pad `pad` plays now (a chord's root last); none past
    /// the 16 pads or outside MIDI's notes.
    pub fn notes(&self, pad: usize) -> &[u8] {
        self.pads.get(pad).map_or(&[], Pad::notes)
    }

    /// The pad's lowest note, or its chord's root: what pad 1 is called in
    /// the log.
    pub fn root(&self, pad: usize) -> Option<u8> {
        self.notes(pad).last().copied()
    }

    /// "C3 to D#4" (or the lowest and highest chord roots), for the log and
    /// the screen.
    pub fn range_text(&self) -> String {
        pads::range_text(&self.setup())
    }

    #[cfg(test)]
    pub fn octave(&self) -> i32 {
        self.octave
    }

    /// Moves the pads by `delta` octaves, within [`MIN_OCTAVE`] and
    /// [`MAX_OCTAVE`]. Notes already held end on their own pad's note-off.
    /// Returns whether it moved.
    pub fn shift_octave(&mut self, delta: i32) -> bool {
        let o = (self.octave + delta).clamp(MIN_OCTAVE, MAX_OCTAVE);
        let moved = o != self.octave;
        self.octave = o;
        self.pads = pads::layout(&self.setup());
        moved
    }

    /// Each pad's name, for the pad grid: the note ("C#3"), or on Chord pads
    /// the numeral and the chord ("iv7 Dm7"). None for a pad with no notes.
    pub fn pad_labels(&self) -> Vec<Option<String>> {
        let setup = self.setup();
        self.pads.iter().map(|p| {
            let (main, small) = pads::pad_name(&setup, p);
            match (main.is_empty(), small.is_empty()) {
                (true, _) => None,
                (false, true) => Some(main),
                (false, false) => Some(format!("{main} {small}")),
            }
        }).collect()
    }

    /// The header chip: "FM · C3 to D#4", with the layout when a scale is on
    /// ("Poly · D Dorian · In key · D3 to F5").
    pub fn chip(&self) -> String {
        let setup = self.setup();
        match setup.scale {
            None => format!("{} · {}", self.engine().name(), self.range_text()),
            Some(s) => format!("{} · {} · {} · {}", self.engine().name(), scale_name(&s), setup.layout.label(),
                               self.range_text()),
        }
    }

    /// The kit list's entry for the synth.
    pub fn kit_view(&self) -> crate::gui::KitView {
        crate::gui::KitView { name: format!("{KIT_PREFIX}{}", self.engine().name()), pads: self.pad_labels(),
                              colour: PAD_WHITE, chip: Some(self.chip()) }
    }

    /// Resting pad colours. With no scale: C, white and black keys. With a
    /// scale: the root (or the home chord), the scale's notes, and notes
    /// outside it. A pad with no notes is dark.
    pub fn pad_colours(&self) -> [[u8; 3]; 16] {
        let mut out = [[0u8; 3]; 16];
        for (c, p) in out.iter_mut().zip(&self.pads) {
            let Some(&n) = p.notes().last() else { continue };
            *c = match p.role {
                PadRole::Plain if n.is_multiple_of(12) => PAD_C,
                PadRole::Plain if black(n) => PAD_BLACK,
                PadRole::Plain => PAD_WHITE,
                PadRole::Root => PAD_C,
                PadRole::InScale => PAD_WHITE,
                PadRole::OutOfScale => PAD_BLACK,
            };
        }
        out
    }

    fn push(&mut self, engine: Engine, offset: u32, ev: NodeEvent) {
        let k = engine.index();
        if self.len[k] < MAX_EVENTS {
            self.events[k][self.len[k]] = Timed::new(offset, ev);
            self.len[k] += 1;
        } else {
            self.dropped_events += 1;
        }
    }

    /// Ends the notes pad `pad` started: each note's note-off goes out once
    /// no other pad holds it.
    fn release_pad(&mut self, pad: usize, offset: u32) {
        let held = std::mem::take(&mut self.playing[pad]);
        for &note in held.notes() {
            let h = &mut self.holders[usize::from(note)];
            *h = h.saturating_sub(1);
            if *h == 0 {
                self.push(self.settings.engine, offset, NodeEvent::NoteOff { note });
            }
        }
    }

    /// Pad `pad` starts its notes, `offset` frames into the next render. A
    /// pad already sounding ends its notes first (Note Repeat's next hit).
    pub fn note_on(&mut self, pad: usize, velocity: u8, offset: u32) {
        if pad >= PADS || velocity == 0 {
            return;
        }
        self.release_pad(pad, offset);
        let p = self.pads[pad];
        let mut held = Held::default();
        for &note in p.notes() {
            self.push(self.settings.engine, offset, NodeEvent::NoteOn { note, velocity: velocity.min(127) });
            let h = &mut self.holders[usize::from(note)];
            *h = h.saturating_add(1);
            held.notes[usize::from(held.len)] = note;
            held.len += 1;
        }
        if held.len > 0 {
            self.notes += 1;
        }
        self.playing[pad] = held;
        self.press[pad] = 0;
    }

    /// Pad `pad` was let go (or its repeat ended), `offset` frames in.
    pub fn note_off(&mut self, pad: usize, offset: u32) {
        if pad < PADS {
            self.release_pad(pad, offset);
            self.press[pad] = 0;
        }
    }

    /// Pad pressure, 0 to 127, on every note the pad plays. A note two pads
    /// hold takes the harder press.
    pub fn pressure(&mut self, pad: usize, value: u8) {
        if pad >= PADS || self.playing[pad].len == 0 {
            return;
        }
        self.press[pad] = value.min(127);
        let held = self.playing[pad];
        for &note in held.notes() {
            let mut v = 0u8;
            for (q, other) in self.playing.iter().enumerate() {
                if other.notes().contains(&note) {
                    v = v.max(self.press[q]);
                }
            }
            self.push(self.settings.engine, 0, NodeEvent::Pressure { note, value: v });
        }
    }

    /// A note by number, `offset` frames into the next render: the arp on
    /// a pad layout, which sends the layout's notes rather than pads. It is
    /// counted with the pads' notes, so a note a pad and the arp both hold
    /// ends with the last of them.
    pub fn note_on_note(&mut self, note: u8, velocity: u8, offset: u32) {
        if note > 127 || velocity == 0 {
            return;
        }
        self.push(self.settings.engine, offset, NodeEvent::NoteOn { note, velocity: velocity.min(127) });
        let h = &mut self.holders[usize::from(note)];
        *h = h.saturating_add(1);
        self.notes += 1;
    }

    /// Ends one [`Synth::note_on_note`]; the note-off goes out once nothing
    /// else holds the note.
    pub fn note_off_note(&mut self, note: u8, offset: u32) {
        if note > 127 {
            return;
        }
        let h = &mut self.holders[usize::from(note)];
        if *h == 0 {
            return;
        }
        *h -= 1;
        if *h == 0 {
            self.push(self.settings.engine, offset, NodeEvent::NoteOff { note });
        }
    }

    /// Pressure, 0 to 127, on a note by number (while something holds it).
    pub fn pressure_note(&mut self, note: u8, value: u8) {
        if note <= 127 && self.holders[usize::from(note)] > 0 {
            self.push(self.settings.engine, 0, NodeEvent::Pressure { note, value: value.min(127) });
        }
    }

    /// Every note ends with its release (the pads went back to drums).
    pub fn all_off(&mut self) {
        self.push(self.settings.engine, 0, NodeEvent::AllOff);
        self.playing = [Held::default(); PADS];
        self.holders = [0; 128];
        self.press = [0; PADS];
    }

    /// STOP: fade out over 10 ms, then reset, so nothing rings on.
    pub fn stop(&mut self) {
        self.all_off();
        if !self.fading {
            self.fading = true;
            self.fade = FADE;
        }
    }

    // ------------------------------------------------------------ sound

    /// Adds the synth's next `left.len()` frames into `left` and `right`.
    /// `frame` is the absolute frame of the first one. The engine not on
    /// the pads renders only while it still has a note ringing (or events).
    /// Never allocates.
    pub fn render(&mut self, frame: u64, bpm: f64, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let len = self.len;
        self.len = [0; 2];
        if n == 0 {
            return;
        }
        let on = self.settings.engine.index();
        let run = [on == 0 || len[0] > 0 || self.poly.active_voices() > 0,
                   on == 1 || len[1] > 0 || self.fm.active_voices() > 0];
        // Stable insertion sort by frame (few events, no allocation); late
        // events land on the last frame.
        for (events, &len) in self.events.iter_mut().zip(&len) {
            for e in &mut events[..len] {
                e.offset = e.offset.min(n as u32 - 1);
            }
            for i in 1..len {
                let mut j = i;
                while j > 0 && events[j - 1].offset > events[j].offset {
                    events.swap(j - 1, j);
                    j -= 1;
                }
            }
        }
        let (mut start, mut e) = (0usize, [0usize; 2]);
        while start < n {
            let end = (start + SCRATCH).min(n);
            let fade0 = self.fade;
            for k in 0..2 {
                if !run[k] {
                    continue;
                }
                let first = e[k];
                while e[k] < len[k] && (self.events[k][e[k]].offset as usize) < end {
                    self.events[k][e[k]].offset -= start as u32;
                    e[k] += 1;
                }
                let (sl, sr) = (&mut self.left[..end - start], &mut self.right[..end - start]);
                sl.fill(0.0);
                sr.fill(0.0);
                let ctx = Ctx { rate: self.rate, frame: frame + start as u64, bpm, beats: 0.0, playing: false,
                                sidechain: None };
                let node: &mut dyn Node = if k == 0 { &mut self.poly } else { &mut self.fm };
                node.process(&ctx, &self.events[k][first..e[k]], sl, sr);
                if !sl.iter().chain(sr.iter()).all(|s| s.is_finite()) {
                    node.reset();
                    if k == on {
                        self.playing = [Held::default(); PADS];
                        self.holders = [0; 128];
                    }
                    self.resets += 1;
                    continue;
                }
                if self.fading {
                    for (i, (l, r)) in sl.iter_mut().zip(sr.iter_mut()).enumerate() {
                        let g = fade0.saturating_sub(i as u32) as f32 / FADE as f32;
                        *l *= g;
                        *r *= g;
                    }
                }
                for (o, s) in left[start..end].iter_mut().zip(sl.iter()) {
                    *o += *s;
                }
                for (o, s) in right[start..end].iter_mut().zip(sr.iter()) {
                    *o += *s;
                }
            }
            if self.fading {
                self.fade = fade0.saturating_sub((end - start) as u32);
                if self.fade == 0 {
                    self.poly.reset();
                    self.fm.reset();
                    self.playing = [Held::default(); PADS];
                    self.holders = [0; 128];
                    self.fading = false;
                }
            }
            start = end;
        }
        self.peak_voices = self.peak_voices.max(self.active_voices());
    }

    /// Voices sounding or fading, on both engines.
    pub fn active_voices(&self) -> usize {
        self.poly.active_voices() + self.fm.active_voices()
    }

    pub fn counters(&self) -> Counters {
        Counters { notes: self.notes, peak_voices: self.peak_voices,
                   dropped: self.dropped_events + self.poly.dropped + self.fm.dropped, resets: self.resets }
    }

    /// The engine's voice count setting, for the status line.
    pub fn polyphony(&self) -> f32 {
        match self.settings.engine {
            Engine::Poly => self.poly.param(mpfree_synth::params::POLYPHONY),
            Engine::Fm => self.fm.param(mpfree_synth::fm_params::POLYPHONY),
        }
    }

    /// Each sounding voice's pressure, 0 to 1, for the Pressure page's live
    /// bars. Returns how many it wrote.
    pub fn pressures(&self, out: &mut [f32]) -> usize {
        match self.settings.engine {
            Engine::Poly => self.poly.pressures(out),
            Engine::Fm => self.fm.pressures(out),
        }
    }

    // ------------------------------------------------------------ the Synth tab

    /// Sets a parameter of the engine on the pads (clamped by the node).
    pub fn set(&mut self, id: u16, value: f32) {
        self.node_mut().set_param(id, value);
    }

    pub fn value(&self, id: u16) -> f32 {
        self.node().param(id)
    }

    /// How many presets the engine on the pads has.
    pub fn preset_count(&self) -> usize {
        match self.settings.engine {
            Engine::Poly => Poly::presets().len(),
            Engine::Fm => self.fm_presets.len(),
        }
    }

    /// Loads preset `i` of the engine on the pads; returns its name, or None
    /// if there is no such preset. Notes keep sounding; values glide to the
    /// preset's.
    pub fn load_preset(&mut self, i: usize) -> Option<&str> {
        match self.settings.engine {
            Engine::Poly => {
                let p = Poly::presets().get(i)?;
                self.poly.load_preset(p);
                self.presets[0] = i;
                Some(p.name)
            }
            Engine::Fm => {
                let p = self.fm_presets.get(i)?;
                self.fm.load_preset(p);
                self.presets[1] = i;
                Some(&p.name)
            }
        }
    }

    #[cfg(test)]
    pub fn preset(&self) -> usize {
        self.presets[self.settings.engine.index()]
    }

    pub fn preset_name(&self) -> &str {
        let i = self.presets[self.settings.engine.index()];
        match self.settings.engine {
            Engine::Poly => Poly::presets().get(i).map_or("", |p| p.name),
            Engine::Fm => self.fm_presets.get(i).map_or("", |p| p.name.as_str()),
        }
    }

    /// Every value, in the node's parameter order (for the Synth tab).
    pub fn values(&self) -> Vec<f32> {
        let node = self.node();
        node.params().iter().map(|p| node.param(p.id)).collect()
    }

    /// The meters' values, by parameter index.
    pub fn meters(&self) -> Vec<(usize, f32)> {
        let node = self.node();
        let params = node.params();
        self.meters[self.settings.engine.index()].iter().map(|&i| (i, node.param(params[i].id))).collect()
    }

    /// What the Synth tab shows.
    pub fn state(&self) -> SynthState {
        let presets = match self.settings.engine {
            Engine::Poly => Poly::presets().iter().map(|p| p.name.to_string()).collect(),
            Engine::Fm => self.fm_presets.iter().map(|p| p.name.clone()).collect(),
        };
        let preset = Some(self.presets[self.settings.engine.index()]);
        SynthState::from_node(self.node(), "Pads", Color::from_pad(PAD_WHITE), presets, preset)
    }

    pub fn param_info(&self, id: u16) -> Option<&'static ParamInfo> {
        self.node().params().iter().find(|p| p.id == id)
    }
}

/// A scale's name for the chip and the log: "D Dorian", or its notes'
/// count when it isn't one of the built-in scales ("D, 6 notes").
pub fn scale_name(s: &Scale) -> String {
    use mpfree_arp::scale::{NOTE_NAMES, SCALES};
    let root = NOTE_NAMES[usize::from(s.root % 12)];
    match SCALES.iter().find(|d| d.mask == s.mask && d.down == s.down) {
        Some(d) => format!("{root} {}", d.name),
        None => format!("{root}, {} notes", s.mask.count_ones()),
    }
}

/// Knob turns for the log: one line per parameter once it rests for a
/// second (or another one is turned), not one per detent.
#[derive(Default)]
pub struct ParamLog {
    pending: Option<(u16, f32, Instant)>,
}

impl ParamLog {
    /// A change; returns the line for the previous parameter if this is
    /// another one.
    pub fn set(&mut self, synth: &Synth, id: u16, value: f32, now: Instant) -> Option<String> {
        let line = match self.pending {
            Some((p, _, _)) if p != id => self.flush(synth),
            _ => None,
        };
        self.pending = Some((id, value, now));
        line
    }

    /// The line for a parameter that has rested long enough, if any.
    pub fn poll(&mut self, synth: &Synth, now: Instant) -> Option<String> {
        match self.pending {
            Some((_, _, t)) if now.duration_since(t) >= LOG_AFTER => self.flush(synth),
            _ => None,
        }
    }

    /// The pending line now (a preset load, an engine switch or the end of
    /// the session).
    pub fn flush(&mut self, synth: &Synth) -> Option<String> {
        let (id, _, _) = self.pending.take()?;
        let p = synth.param_info(id)?;
        // The node's own (clamped) value, not the one asked for.
        let v = synth.value(id);
        let name = if p.group == p.name { p.name.to_string() } else { format!("{} {}", p.group, p.name) };
        Some(format!("synth: {name} {}", value_line(p, v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_arp::scale::by_name;
    use mpfree_synth::params as pp;

    fn run(s: &mut Synth, frame: &mut u64, periods: usize, block: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for _ in 0..periods {
            let (mut l, mut r) = (vec![0f32; block], vec![0f32; block]);
            s.render(*frame, 120.0, &mut l, &mut r);
            *frame += block as u64;
            out.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
        }
        out
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0f32, |m, s| m.max(s.abs()))
    }

    fn queued(s: &Synth) -> Vec<NodeEvent> {
        let k = s.engine().index();
        s.events[k][..s.len[k]].iter().map(|t| t.ev).collect()
    }

    fn scale(name: &str, root: u8) -> Option<Scale> {
        by_name(name).map(|d| Scale::with_down(root, d.mask, d.down))
    }

    fn with(s: &mut Synth, f: impl FnOnce(&mut PadSettings)) {
        let mut p = s.settings();
        f(&mut p);
        s.set_settings(p);
    }

    #[test]
    fn pads_play_semitones_from_c3_and_octaves_move_them() {
        let mut s = Synth::new(44_100);
        assert_eq!((s.root(0), s.root(15)), (Some(48), Some(63)));
        assert_eq!(s.range_text(), "C3 to D#4");
        assert_eq!(s.chip(), "Poly · C3 to D#4");
        assert_eq!(s.pad_labels()[1].as_deref(), Some("C#3"));
        assert!(s.shift_octave(1));
        assert_eq!(s.range_text(), "C4 to D#5");
        assert!(s.shift_octave(10));
        assert_eq!(s.octave(), MAX_OCTAVE);
        assert!(!s.shift_octave(1));
        assert!(s.shift_octave(-10));
        assert_eq!(s.notes(0), &[12]);
        assert_eq!(s.notes(15), &[27]);
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(0), "C-1");
        // With no scale every bank is Chromatic.
        for b in 0..BANKS {
            with(&mut s, |p| p.bank = b);
            assert_eq!(s.notes(0), &[12], "bank {b}");
        }
    }

    #[test]
    fn notes_by_number_share_the_count_with_the_pads() {
        let mut s = Synth::new(44_100);
        // Pad 0 plays C3 (48); the arp plays C3 and E3 by number.
        s.note_on(0, 100, 0);
        s.note_on_note(48, 90, 5);
        s.note_on_note(52, 90, 5);
        s.note_off(0, 10);
        assert!(!queued(&s).contains(&NodeEvent::NoteOff { note: 48 }), "the arp still holds C3");
        s.pressure_note(52, 70);
        s.pressure_note(55, 70);
        s.note_off_note(48, 20);
        s.note_off_note(52, 20);
        s.note_off_note(52, 21);
        assert_eq!(
            queued(&s),
            vec![
                NodeEvent::NoteOn { note: 48, velocity: 100 },
                NodeEvent::NoteOn { note: 48, velocity: 90 },
                NodeEvent::NoteOn { note: 52, velocity: 90 },
                NodeEvent::Pressure { note: 52, value: 70 },
                NodeEvent::NoteOff { note: 48 },
                NodeEvent::NoteOff { note: 52 },
            ]
        );
        let mut frame = 0;
        assert!(peak(&run(&mut s, &mut frame, 2, 256)) > 0.0);
        // A note-off with nothing holding the note sends nothing.
        s.note_off_note(60, 0);
        assert!(queued(&s).is_empty());
    }

    #[test]
    fn pad_colours_are_ones_akai_sent() {
        let mut s = Synth::new(44_100);
        let c = s.pad_colours();
        assert_eq!(c[0], PAD_C);
        assert_eq!(c[1], PAD_BLACK);
        assert_eq!(c[2], PAD_WHITE);
        assert_eq!(c[12], PAD_C);
        // D Dorian on Chromatic: the root, the scale's notes, the rest.
        with(&mut s, |p| p.scale = scale("Dorian", 2));
        let c = s.pad_colours();
        assert_eq!((c[2], c[4], c[1]), (PAD_C, PAD_WHITE, PAD_BLACK), "D, E, C#");
        assert_eq!(c[0], PAD_WHITE, "C is in D Dorian");
        for layout in PadLayout::ALL {
            with(&mut s, |p| p.set_layout(layout));
            for colour in s.pad_colours() {
                assert!(colour == [0; 3] || crate::akai::RESTING_COLOURS.contains(&colour), "{colour:02X?}");
            }
        }
    }

    #[test]
    fn a_scale_lays_the_banks_out_and_the_quick_switch_flips_one() {
        let mut s = Synth::new(44_100);
        with(&mut s, |p| p.scale = scale("Minor", 9));
        // A: Chromatic, the notes as before.
        assert_eq!(s.notes(0), &[48]);
        assert_eq!(s.chip(), "Poly · A Minor · Chromatic · C3 to D#4");
        // B: In key from A3.
        with(&mut s, |p| p.bank = 1);
        let in_key: Vec<u8> = (0..8).filter_map(|p| s.root(p)).collect();
        assert_eq!(in_key, vec![57, 59, 60, 62, 64, 65, 67, 69]);
        assert_eq!(s.pad_labels()[0].as_deref(), Some("A3"));
        // C: Chord pads, root last; pad 1 is i (A minor).
        with(&mut s, |p| p.bank = 2);
        assert_eq!(s.notes(0), &[60, 64, 57]);
        assert_eq!(s.notes(8).len(), 4, "the top rows are sevenths");
        assert_eq!(s.pad_labels()[0].as_deref(), Some("i Am"));
        // D: In key an octave up.
        with(&mut s, |p| p.bank = 3);
        assert_eq!(s.root(0), Some(69));
        // The quick switch on bank A, then back.
        with(&mut s, |p| p.bank = 0);
        with(&mut s, |p| p.set_layout(p.layout().toggled()));
        assert_eq!(s.settings().layout(), PadLayout::InKey);
        assert_eq!(s.root(0), Some(57));
        with(&mut s, |p| p.set_layout(p.layout().toggled()));
        assert_eq!(s.root(0), Some(48));
        // A fourth apart: each row starts three scale notes up.
        with(&mut s, |p| {
            p.rows = InKeyRows::Fourths;
            p.set_layout(PadLayout::InKey);
        });
        assert_eq!((s.root(0), s.root(4)), (Some(57), Some(62)));
    }

    #[test]
    fn a_chord_pad_plays_every_note_and_a_shared_note_waits_for_the_last_pad() {
        let mut s = Synth::new(44_100);
        with(&mut s, |p| {
            p.scale = scale("Major", 0);
            p.set_layout(PadLayout::Chords);
        });
        // I = E G C, iii = G B E: E and G are shared.
        assert_eq!(s.notes(0), &[52, 55, 48]);
        assert_eq!(s.notes(2), &[55, 59, 52]);
        s.note_on(0, 100, 0);
        s.note_on(2, 90, 5);
        s.pressure(0, 70);
        s.pressure(2, 30);
        s.note_off(0, 10);
        let on = |note, velocity| NodeEvent::NoteOn { note, velocity };
        let press = |note, value| NodeEvent::Pressure { note, value };
        assert_eq!(queued(&s), vec![on(52, 100), on(55, 100), on(48, 100), on(55, 90), on(59, 90), on(52, 90),
                                    press(52, 70), press(55, 70), press(48, 70),
                                    // E and G keep the harder press while pad 1 holds them.
                                    press(55, 70), press(59, 30), press(52, 70),
                                    // Pad 1 let go: only C ends; E and G still sound for pad 3.
                                    NodeEvent::NoteOff { note: 48 }]);
        let mut frame = 0;
        run(&mut s, &mut frame, 1, 128);
        s.note_off(2, 0);
        let mut offs: Vec<NodeEvent> = queued(&s);
        offs.sort_by_key(|e| match e { NodeEvent::NoteOff { note } => *note, _ => 0 });
        assert_eq!(offs, vec![NodeEvent::NoteOff { note: 52 }, NodeEvent::NoteOff { note: 55 },
                              NodeEvent::NoteOff { note: 59 }]);
        assert_eq!(s.counters().notes, 2, "a chord counts as one pad hit");
    }

    #[test]
    fn a_pad_plays_a_note_that_adds_to_the_period_and_ends() {
        for engine in Engine::ALL {
            let mut s = Synth::new(44_100);
            with(&mut s, |p| p.engine = engine);
            let mut frame = 0;
            s.note_on(0, 100, 10);
            let mut l = [0.25f32; 128];
            let mut r = [0.25f32; 128];
            s.render(frame, 120.0, &mut l, &mut r);
            frame += 128;
            assert!(l[..10].iter().all(|&x| x == 0.25), "{engine:?}: nothing before the note's frame");
            assert!(l.iter().any(|&x| x != 0.25), "{engine:?} adds to what is there");
            assert_eq!(s.active_voices(), 1);
            let sound = run(&mut s, &mut frame, 40, 128);
            let p = peak(&sound);
            assert!(p > 0.05 && p < 1.0, "{engine:?}: a note at about -12 dBFS: peak {p}");
            s.note_off(0, 0);
            run(&mut s, &mut frame, 2000, 128);
            assert_eq!(s.active_voices(), 0, "{engine:?}");
            assert!(peak(&run(&mut s, &mut frame, 2, 128)) == 0.0);
            assert_eq!(s.counters(), Counters { notes: 1, peak_voices: 1, dropped: 0, resets: 0 }, "{engine:?}");
        }
    }

    #[test]
    fn a_new_engine_takes_the_pads_while_the_old_one_finishes_its_release() {
        let mut s = Synth::new(44_100);
        let mut frame = 0;
        s.note_on(0, 110, 0);
        run(&mut s, &mut frame, 20, 128);
        assert_eq!(s.poly.active_voices(), 1);
        let mut p = s.settings();
        p.engine = Engine::Fm;
        assert!(s.set_settings(p));
        assert!(!s.set_settings(p), "the same engine again is no switch");
        assert_eq!(s.engine(), Engine::Fm);
        assert_eq!(s.state().info.id, "mpfree.fm");
        assert_eq!(s.preset_name(), FM_START_PRESET);
        // The pad's note-off after the switch ends nothing on FM.
        s.note_off(0, 0);
        assert!(queued(&s).is_empty());
        s.note_on(4, 100, 0);
        run(&mut s, &mut frame, 2, 128);
        assert_eq!((s.poly.active_voices(), s.fm.active_voices()), (1, 1), "Poly releases while FM plays");
        run(&mut s, &mut frame, 400, 128);
        assert_eq!(s.poly.active_voices(), 0, "Poly's release ended");
        assert_eq!(s.fm.active_voices(), 1);
        // The Synth tab edits FM now.
        let id = mpfree_synth::fm_params::LEVEL;
        s.set(id, -6.0);
        assert_eq!(s.value(id), -6.0);
        assert_eq!(s.poly.param(pp::LEVEL), Poly::new(44_100).param(pp::LEVEL), "Poly untouched");
        assert_eq!(s.load_preset(2), Some("Bell Tower"));
        assert_eq!(s.preset_count(), Fm::presets().len());
        assert_eq!(s.kit_view().name, "Synth: FM");
        assert!(s.chip().starts_with("FM · "));
    }

    #[test]
    fn the_same_notes_sound_the_same_in_any_block_size() {
        // (frame, pad, on); periods longer than the scratch buffers render in pieces.
        let script = [(3u64, 0usize, true), (64, 4, true), (500, 7, true), (2000, 0, false), (2100, 4, false),
                      (2600, 7, false), (3000, 2, true), (4000, 2, false)];
        for engine in Engine::ALL {
            let go = |block: usize| {
                let mut s = Synth::new(44_100);
                with(&mut s, |p| {
                    p.engine = engine;
                    p.scale = scale("Major", 0);
                    p.set_layout(PadLayout::Chords);
                });
                let mut frame = 0u64;
                let mut out = Vec::new();
                while frame < 6000 {
                    let n = block.min((6000 - frame) as usize);
                    for &(at, pad, on) in &script {
                        if (frame..frame + n as u64).contains(&at) {
                            let offset = (at - frame) as u32;
                            if on { s.note_on(pad, 90, offset) } else { s.note_off(pad, offset) }
                        }
                    }
                    out.extend(run(&mut s, &mut frame, 1, n));
                }
                out
            };
            let whole = go(128);
            assert!(peak(&whole) > 0.05, "{engine:?}");
            for block in [1, 31, 64, 1000] {
                assert!(go(block) == whole, "{engine:?}, block size {block}");
            }
        }
    }

    #[test]
    fn stop_fades_out_within_ten_ms() {
        let mut s = Synth::new(44_100);
        s.load_preset(Poly::presets().iter().position(|p| p.name == "Dust Pad").unwrap());
        let mut frame = 0;
        for pad in [0, 4, 7] {
            s.note_on(pad, 120, 0);
        }
        run(&mut s, &mut frame, 200, 128);
        assert_eq!(s.active_voices(), 3);
        // FM takes the pads while Poly's three notes ring; STOP ends both.
        with(&mut s, |p| p.engine = Engine::Fm);
        s.note_on(2, 120, 0);
        run(&mut s, &mut frame, 2, 128);
        assert_eq!(s.active_voices(), 4);
        s.stop();
        let out = run(&mut s, &mut frame, 4, 128);
        assert_eq!(s.active_voices(), 0);
        let after = &out[2 * FADE as usize..];
        assert!(after.iter().all(|&x| x == 0.0), "silent after the fade");
        // A pad let go after STOP sends nothing.
        s.note_off(0, 0);
        assert_eq!(s.len, [0, 0]);
    }

    #[test]
    fn note_repeat_retriggers_and_pressure_follows_the_pad() {
        let mut s = Synth::new(44_100);
        s.note_on(2, 100, 0);
        s.pressure(2, 90);
        s.note_on(2, 100, 50);
        s.pressure(9, 90); // not playing: nothing
        assert_eq!(queued(&s), vec![NodeEvent::NoteOn { note: 50, velocity: 100 },
                                    NodeEvent::Pressure { note: 50, value: 90 },
                                    NodeEvent::NoteOff { note: 50 }, NodeEvent::NoteOn { note: 50, velocity: 100 }]);
        // After an octave or layout change the held pad's note-off still ends its note.
        s.shift_octave(1);
        with(&mut s, |p| {
            p.scale = scale("Major", 0);
            p.set_layout(PadLayout::InKey);
        });
        s.note_off(2, 60);
        assert_eq!(queued(&s).last(), Some(&NodeEvent::NoteOff { note: 50 }));
    }

    #[test]
    fn too_many_events_are_counted_not_kept() {
        let mut s = Synth::new(44_100);
        for _ in 0..MAX_EVENTS + 5 {
            s.note_on(1, 80, 0);
        }
        assert_eq!(s.len[0], MAX_EVENTS);
        assert!(s.counters().dropped >= 5);
        let mut frame = 0;
        run(&mut s, &mut frame, 1, 128);
        assert_eq!(s.len, [0, 0]);
    }

    #[test]
    fn presets_and_values_reach_the_node_and_the_screen() {
        let mut s = Synth::new(44_100);
        assert_eq!(s.preset_name(), START_PRESET);
        let st = s.state();
        assert_eq!(st.info.id, "mpfree.poly");
        assert_eq!(st.preset, Some(s.preset()));
        assert_eq!(st.values, s.values());
        assert_eq!(s.load_preset(3), Some("Supersaw"));
        assert_eq!(s.value(pp::UNISON), 4.0);
        assert_eq!(s.load_preset(99), None);
        s.set(pp::CUTOFF, 1e9);
        assert_eq!(s.value(pp::CUTOFF), s.param_info(pp::CUTOFF).unwrap().max);
        let meters = s.meters();
        assert!(meters.iter().any(|&(i, _)| st.params[i].id == pp::METER_VOICES));
        assert_eq!(s.polyphony(), 8.0);
        // FM's own presets, values and meters.
        with(&mut s, |p| p.engine = Engine::Fm);
        let st = s.state();
        assert_eq!(st.presets.len(), s.preset_count());
        assert_eq!(st.values, s.values());
        assert!(s.meters().iter().any(|&(i, _)| st.params[i].id == mpfree_synth::fm_params::METER_VOICES));
        // Back to Poly: its preset is still Supersaw.
        with(&mut s, |p| p.engine = Engine::Poly);
        assert_eq!(s.preset_name(), "Supersaw");
    }

    #[test]
    fn the_audio_threads_part_never_allocates() {
        let mut s = Synth::new(44_100);
        let (mut l, mut r) = ([0f32; 128], [0f32; 128]);
        let minor = scale("Minor", 9);
        let mut bars = [0f32; 16];
        let ((), calls) = crate::sys::alloc_count::calls(|| {
            let mut frame = 0u64;
            for p in 0..3000usize {
                if p % 7 == 0 {
                    s.note_on(p % 16, 40 + (p % 80) as u8, (p % 128) as u32);
                }
                if p % 11 == 0 {
                    s.note_off((p + 5) % 16, 3);
                }
                if p % 5 == 0 {
                    s.pressure(p % 16, (p % 128) as u8);
                }
                if p % 97 == 0 {
                    s.set(pp::CUTOFF, 100.0 + (p % 5000) as f32);
                }
                if p % 401 == 0 {
                    s.load_preset(p / 401 % 5);
                }
                if p % 733 == 0 {
                    s.shift_octave(if p % 2 == 0 { 1 } else { -1 });
                }
                if p % 211 == 0 {
                    let mut set = s.settings();
                    set.scale = if p % 2 == 0 { minor } else { None };
                    set.bank = p / 211 % BANKS;
                    set.engine = Engine::ALL[p / 300 % 2];
                    s.set_settings(set);
                }
                if p % 307 == 0 {
                    let mut set = s.settings();
                    set.set_layout(set.layout().toggled());
                    s.set_settings(set);
                }
                if p == 1500 {
                    s.all_off();
                }
                if p == 2500 {
                    s.stop();
                }
                s.render(frame, 120.0, &mut l, &mut r);
                frame += 128;
                let _ = (s.active_voices(), s.counters(), s.pressures(&mut bars));
            }
        });
        assert_eq!(calls, 0, "allocations or frees on the audio thread's path");
        assert!(s.counters().notes > 300);
    }

    #[test]
    fn knob_turns_log_one_line_per_rest() {
        let s = Synth::new(44_100);
        let mut log = ParamLog::default();
        let t0 = Instant::now();
        assert_eq!(log.set(&s, pp::CUTOFF, 300.0, t0), None);
        assert_eq!(log.set(&s, pp::CUTOFF, 310.0, t0), None);
        assert_eq!(log.poll(&s, t0 + Duration::from_millis(500)), None);
        let line = log.set(&s, pp::RESONANCE, 40.0, t0 + Duration::from_millis(600)).unwrap();
        assert!(line.starts_with("synth: Filter Cutoff "), "{line}");
        assert!(log.poll(&s, t0 + Duration::from_millis(1700)).unwrap().starts_with("synth: Filter Resonance "));
        assert_eq!(log.poll(&s, t0 + Duration::from_secs(5)), None);
    }

    #[test]
    fn scale_names_read_as_root_and_name() {
        assert_eq!(scale_name(&scale("Dorian", 2).unwrap()), "D Dorian");
        assert_eq!(scale_name(&Scale::new(9, 0b1001_0010_1001)), "A, 5 notes");
    }
}
