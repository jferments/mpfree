//! The pad player's synth: MPFree's Poly, FM, Drums, Pluck or Wavetable
//! (`mpfree-synth`) on the pads, edited on the Synth tab (`mpfree-synth-ui`).
//!
//! - The kit list ends with one more entry, "Synth: Poly" (or FM, Drums,
//!   Pluck, Wavetable). Picking it, or opening the Synth tab, makes the pads play the
//!   synth. The Synth tab's options (MENU) pick the engine; a note still
//!   ringing on another engine finishes its release.
//! - Drums is a kit, not a keyboard: pad 1 plays the kit's Pad 1 note (a
//!   Kit page knob, C2 by default) and each pad after it a semitone higher,
//!   whatever the scale, layout, bank or octave. The pads are named by the
//!   sound on them ("Kick", "Closed hat"), and the Synth tab shows the page
//!   of the pad hit last. Pluck and Wavetable play the pad layouts as Poly
//!   and FM do.
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
//! - The audio thread owns every node. The Synth tab (on the screen thread)
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

use mpfree_arp::scale::pads::{
    self, InKeyRows, Pad, PadLayout, PadRole, PadSetup, DEFAULT_BANKS, MAX_PAD_NOTES, PADS,
};
use mpfree_arp::scale::Scale;
use mpfree_instruments::preset::{engine_of, Error as PresetError, Preset as SoundPreset};
use mpfree_node::{Ctx, Node, NodeEvent, ParamInfo, Timed};
use mpfree_synth::drum_params;
use mpfree_synth::fm_params::FmPreset;
use mpfree_synth::{Drums, Fm, Pluck, Poly, Preset, Wavetable};
use mpfree_synth_ui::format::value_line;
use mpfree_synth_ui::SynthState;
use mpfree_ui::Color;
use std::time::{Duration, Instant};

/// The start of the kit list's entry for the synth ("Synth: Poly").
pub const KIT_PREFIX: &str = "Synth: ";
/// The preset each engine starts with.
pub const START_PRESET: &str = "Pluck";
pub const FM_START_PRESET: &str = "Glass Keys";
pub const DRUMS_START_PRESET: &str = "Init Kit";
pub const PLUCK_START_PRESET: &str = "Steel String";
pub const WT_START_PRESET: &str = "Harmonic Pluck";
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

/// How many engines there are.
const ENGINES: usize = 5;

/// The synth engines the pads can play.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Engine {
    /// The subtractive synth (round 6).
    #[default]
    Poly,
    /// The six-operator FM synth.
    Fm,
    /// The drum synth: a sound on each pad.
    Drums,
    /// Plucked strings and struck mallets.
    Pluck,
    /// The wavetable synth: two oscillators scanning frame stacks.
    Wavetable,
}

impl Engine {
    pub const ALL: [Engine; ENGINES] = [
        Engine::Poly,
        Engine::Fm,
        Engine::Drums,
        Engine::Pluck,
        Engine::Wavetable,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Engine::Poly => "Poly",
            Engine::Fm => "FM",
            Engine::Drums => "Drums",
            Engine::Pluck => "Pluck",
            Engine::Wavetable => "Wavetable",
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    /// The engine whose user presets live in `folder` under the library's
    /// Instruments place: the folder is the engine's name ("Poly", "FM"),
    /// in any case.
    pub fn from_folder(folder: &str) -> Option<Engine> {
        Engine::ALL
            .into_iter()
            .find(|e| e.name().eq_ignore_ascii_case(folder.trim()))
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
        PadSettings {
            engine: Engine::Poly,
            scale: None,
            rows: InKeyRows::Sequence,
            banks: DEFAULT_BANKS,
            bank: 0,
        }
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
        PadSetup {
            layout,
            rows: self.rows,
            scale: self.scale,
            octave: (octave + i32::from(up)).clamp(-9, 9) as i8,
        }
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
    drums: Drums,
    pluck: Pluck,
    wt: Wavetable,
    fm_presets: Vec<FmPreset>,
    rate: u32,
    /// Note events for each engine (in [`Engine::ALL`]'s order) this period.
    events: [[Timed; MAX_EVENTS]; ENGINES],
    len: [usize; ENGINES],
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
    presets: [usize; ENGINES],
    /// Drums' pads' notes or names changed since the app last asked.
    pads_changed: bool,
    /// Frames left in STOP's fade, while `fading`.
    fade: u32,
    fading: bool,
    notes: u64,
    peak_voices: usize,
    resets: u64,
    /// Indices of the read-only parameters (meters), per engine.
    meters: [Vec<usize>; ENGINES],
    /// The sound from before the preset picker's first try, to put back if
    /// the picker is cancelled ([`Synth::try_preset`]).
    audition: Option<SoundPreset>,
}

/// The factory presets of an engine other than FM (whose list is built).
fn static_presets(engine: Engine) -> &'static [Preset] {
    match engine {
        Engine::Drums => Drums::presets(),
        Engine::Pluck => Pluck::presets(),
        Engine::Wavetable => Wavetable::presets(),
        Engine::Poly | Engine::Fm => Poly::presets(),
    }
}

/// A new instrument of the engine `id` (its [`mpfree_node::NodeInfo::id`])
/// at `rate`, with `values` (every parameter, in the node's order, as
/// [`Synth::values`] and the Synth tab's state hold them) set: a copy of
/// the patch for auto-sampling off the audio thread, which keeps playing
/// its own. The meters are left alone. None for an engine it doesn't know
/// or values that aren't its.
pub fn fresh_node(id: &str, values: &[f32], rate: u32) -> Option<Box<dyn Node>> {
    let mut node: Box<dyn Node> = match id {
        "mpfree.poly" => Box::new(Poly::new(rate)),
        "mpfree.fm" => Box::new(Fm::new(rate)),
        "mpfree.drums" => Box::new(Drums::new(rate)),
        "mpfree.pluck" => Box::new(Pluck::new(rate)),
        "mpfree.wavetable" => Box::new(Wavetable::new(rate)),
        _ => return None,
    };
    let params = node.params();
    if params.len() != values.len() {
        return None;
    }
    for (p, v) in params.iter().zip(values) {
        if !p.readonly {
            node.set_param(p.id, *v);
        }
    }
    Some(node)
}

/// The index of the preset called `name`, or 0.
fn start_preset(presets: &[Preset], name: &str) -> usize {
    presets.iter().position(|p| p.name == name).unwrap_or(0)
}

/// Whether `id` is a Drums parameter that changes the pads' notes (the
/// kit's Pad 1 note) or their names (a pad's sound).
fn moves_drum_pads(id: u16) -> bool {
    id == drum_params::BASE_NOTE
        || (usize::from(id) < drum_params::PADS * usize::from(drum_params::STRIDE)
            && id % drum_params::STRIDE == drum_params::field::MODEL)
}

fn meters_of(node: &dyn Node) -> Vec<usize> {
    node.params()
        .iter()
        .enumerate()
        .filter(|(_, p)| p.readonly)
        .map(|(i, _)| i)
        .collect()
}

impl Synth {
    /// Every engine at `rate`, with its start preset ([`START_PRESET`] and
    /// the ones after it) loaded and Poly on the pads. Allocates: call it
    /// before the sound starts.
    pub fn new(rate: u32) -> Synth {
        let mut poly = Poly::new(rate);
        let preset = start_preset(Poly::presets(), START_PRESET);
        poly.load_preset(&Poly::presets()[preset]);
        let mut drums = Drums::new(rate);
        let drums_preset = start_preset(Drums::presets(), DRUMS_START_PRESET);
        if let Some(p) = Drums::presets().get(drums_preset) {
            drums.load_preset(p);
        }
        let mut pluck = Pluck::new(rate);
        let pluck_preset = start_preset(Pluck::presets(), PLUCK_START_PRESET);
        if let Some(p) = Pluck::presets().get(pluck_preset) {
            pluck.load_preset(p);
        }
        // Builds the factory wavetables the first time (control thread).
        let mut wt = Wavetable::new(rate);
        let wt_preset = start_preset(Wavetable::presets(), WT_START_PRESET);
        if let Some(p) = Wavetable::presets().get(wt_preset) {
            wt.load_preset(p);
        }
        let mut fm = Fm::new(rate);
        let fm_presets = Fm::presets();
        let fm_preset = fm_presets
            .iter()
            .position(|p| p.name == FM_START_PRESET)
            .unwrap_or(0);
        if let Some(p) = fm_presets.get(fm_preset) {
            fm.load_preset(p);
        }
        let meters = [
            meters_of(&poly),
            meters_of(&fm),
            meters_of(&drums),
            meters_of(&pluck),
            meters_of(&wt),
        ];
        let settings = PadSettings::default();
        Synth {
            poly,
            fm,
            drums,
            pluck,
            wt,
            fm_presets,
            rate,
            events: [[Timed::new(0, NodeEvent::AllOff); MAX_EVENTS]; ENGINES],
            len: [0; ENGINES],
            dropped_events: 0,
            left: [0.0; SCRATCH],
            right: [0.0; SCRATCH],
            settings,
            octave: 0,
            pads: pads::layout(&settings.setup(0)),
            playing: [Held::default(); PADS],
            holders: [0; 128],
            press: [0; PADS],
            presets: [preset, fm_preset, drums_preset, pluck_preset, wt_preset],
            pads_changed: false,
            fade: 0,
            fading: false,
            notes: 0,
            peak_voices: 0,
            resets: 0,
            meters,
            audition: None,
        }
    }

    fn node(&self) -> &dyn Node {
        match self.settings.engine {
            Engine::Poly => &self.poly,
            Engine::Fm => &self.fm,
            Engine::Drums => &self.drums,
            Engine::Pluck => &self.pluck,
            Engine::Wavetable => &self.wt,
        }
    }

    fn node_mut(&mut self) -> &mut dyn Node {
        match self.settings.engine {
            Engine::Poly => &mut self.poly,
            Engine::Fm => &mut self.fm,
            Engine::Drums => &mut self.drums,
            Engine::Pluck => &mut self.pluck,
            Engine::Wavetable => &mut self.wt,
        }
    }

    /// Voices one engine has sounding or fading.
    fn voices_of(&self, engine: Engine) -> usize {
        match engine {
            Engine::Poly => self.poly.active_voices(),
            Engine::Fm => self.fm.active_voices(),
            Engine::Drums => self.drums.active_voices(),
            Engine::Pluck => self.pluck.active_voices(),
            Engine::Wavetable => self.wt.active_voices(),
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
        self.relayout();
        switched
    }

    /// Works the pads' notes out again: Drums' kit notes, or the layout.
    fn relayout(&mut self) {
        self.pads = match self.settings.engine {
            Engine::Drums => {
                let base = self.drums.param(drum_params::BASE_NOTE).round() as i16;
                std::array::from_fn(|p| Pad::single(base + p as i16))
            }
            _ => pads::layout(&self.setup()),
        };
    }

    /// The sound on each of Drums' pads.
    fn drum_names(&self) -> [&'static str; PADS] {
        std::array::from_fn(|p| self.drums.pad_name(p))
    }

    /// Whether Drums' pads' notes or names changed (the kit's Pad 1 note, a
    /// pad's sound, another kit) since the last call, so the app can send
    /// the pad grid and the arp the new ones. Never allocates.
    pub fn take_pads_changed(&mut self) -> bool {
        std::mem::take(&mut self.pads_changed)
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
    /// the screen. Drums: pad 1's note to pad 16's.
    pub fn range_text(&self) -> String {
        match self.settings.engine {
            Engine::Drums => format!(
                "{} to {}",
                self.root(0).map_or(String::new(), note_name),
                self.root(PADS - 1).map_or(String::new(), note_name)
            ),
            _ => pads::range_text(&self.setup()),
        }
    }

    #[cfg(test)]
    pub fn octave(&self) -> i32 {
        self.octave
    }

    /// Moves the pads by `delta` octaves, within [`MIN_OCTAVE`] and
    /// [`MAX_OCTAVE`]. Notes already held end on their own pad's note-off.
    /// Returns whether it moved. Drums' pads don't move: a kit has no
    /// octaves (its Pad 1 note is on the Kit page).
    pub fn shift_octave(&mut self, delta: i32) -> bool {
        if self.settings.engine == Engine::Drums {
            return false;
        }
        let o = (self.octave + delta).clamp(MIN_OCTAVE, MAX_OCTAVE);
        let moved = o != self.octave;
        self.octave = o;
        self.relayout();
        moved
    }

    /// Each pad's name, for the pad grid: the note ("C#3"), or on Chord pads
    /// the numeral and the chord ("iv7 Dm7"); with Drums, the sound
    /// ("Kick"). None for a pad with no notes.
    pub fn pad_labels(&self) -> Vec<Option<String>> {
        if self.settings.engine == Engine::Drums {
            return self
                .drum_names()
                .iter()
                .map(|n| Some(n.to_string()))
                .collect();
        }
        let setup = self.setup();
        self.pads
            .iter()
            .map(|p| {
                let (main, small) = pads::pad_name(&setup, p);
                match (main.is_empty(), small.is_empty()) {
                    (true, _) => None,
                    (false, true) => Some(main),
                    (false, false) => Some(format!("{main} {small}")),
                }
            })
            .collect()
    }

    /// The header chip: "FM · C3 to D#4", with the layout when a scale is on
    /// ("Poly · D Dorian · In key · D3 to F5"); with Drums, the kit
    /// ("Drums · Boom Kit").
    pub fn chip(&self) -> String {
        if self.settings.engine == Engine::Drums {
            return format!("Drums · {}", self.preset_name());
        }
        let setup = self.setup();
        match setup.scale {
            None => format!("{} · {}", self.engine().name(), self.range_text()),
            // As approved (Jay 12:22Z): "Poly · A minor · in key".
            Some(s) => format!(
                "{} · {} · {}",
                self.engine().name(),
                scale_name(&s),
                setup.layout.label().to_lowercase()
            ),
        }
    }

    /// The kit list's entry for the synth.
    pub fn kit_view(&self) -> crate::gui::KitView {
        let setup = self.setup();
        let synth = crate::play_page::SynthPads {
            layout: setup.layout,
            scale: setup.scale.is_some(),
            bank: self.settings.bank,
        };
        crate::gui::KitView {
            name: format!("{KIT_PREFIX}{}", self.engine().name()),
            pads: self.pad_labels(),
            colour: PAD_WHITE,
            chip: Some(self.chip()),
            synth: Some(synth),
        }
    }

    /// Resting pad colours. With no scale: C, white and black keys. With a
    /// scale: the root (or the home chord), the scale's notes, and notes
    /// outside it. Drums: the kit's colour on every pad. A pad with no notes
    /// is dark.
    pub fn pad_colours(&self) -> [[u8; 3]; 16] {
        let mut out = [[0u8; 3]; 16];
        let drums = self.settings.engine == Engine::Drums;
        for (c, p) in out.iter_mut().zip(&self.pads) {
            let Some(&n) = p.notes().last() else { continue };
            if drums {
                *c = PAD_WHITE;
                continue;
            }
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
            self.push(
                self.settings.engine,
                offset,
                NodeEvent::NoteOn {
                    note,
                    velocity: velocity.min(127),
                },
            );
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
            self.push(
                self.settings.engine,
                0,
                NodeEvent::Pressure { note, value: v },
            );
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
        self.push(
            self.settings.engine,
            offset,
            NodeEvent::NoteOn {
                note,
                velocity: velocity.min(127),
            },
        );
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
            self.push(
                self.settings.engine,
                0,
                NodeEvent::Pressure {
                    note,
                    value: value.min(127),
                },
            );
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
        self.len = [0; ENGINES];
        if n == 0 {
            return;
        }
        let on = self.settings.engine.index();
        let run: [bool; ENGINES] =
            std::array::from_fn(|k| k == on || len[k] > 0 || self.voices_of(Engine::ALL[k]) > 0);
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
        let (mut start, mut e) = (0usize, [0usize; ENGINES]);
        while start < n {
            let end = (start + SCRATCH).min(n);
            let fade0 = self.fade;
            for k in 0..ENGINES {
                if !run[k] {
                    continue;
                }
                let first = e[k];
                while e[k] < len[k] && (self.events[k][e[k]].offset as usize) < end {
                    self.events[k][e[k]].offset -= start as u32;
                    e[k] += 1;
                }
                let (sl, sr) = (
                    &mut self.left[..end - start],
                    &mut self.right[..end - start],
                );
                sl.fill(0.0);
                sr.fill(0.0);
                let ctx = Ctx {
                    rate: self.rate,
                    frame: frame + start as u64,
                    bpm,
                    beats: 0.0,
                    playing: false,
                    sidechain: None,
                };
                let node: &mut dyn Node = match Engine::ALL[k] {
                    Engine::Poly => &mut self.poly,
                    Engine::Fm => &mut self.fm,
                    Engine::Drums => &mut self.drums,
                    Engine::Pluck => &mut self.pluck,
                    Engine::Wavetable => &mut self.wt,
                };
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
                    self.drums.reset();
                    self.pluck.reset();
                    self.wt.reset();
                    self.playing = [Held::default(); PADS];
                    self.holders = [0; 128];
                    self.fading = false;
                }
            }
            start = end;
        }
        self.peak_voices = self.peak_voices.max(self.active_voices());
    }

    /// Voices sounding or fading, on every engine.
    pub fn active_voices(&self) -> usize {
        Engine::ALL.iter().map(|&e| self.voices_of(e)).sum()
    }

    pub fn counters(&self) -> Counters {
        Counters {
            notes: self.notes,
            peak_voices: self.peak_voices,
            dropped: self.dropped_events + self.poly.dropped + self.fm.dropped + self.wt.dropped,
            resets: self.resets,
        }
    }

    /// The engine's voice count setting, for the status line (Drums: its
    /// fixed pool of hits).
    pub fn polyphony(&self) -> f32 {
        match self.settings.engine {
            Engine::Poly => self.poly.param(mpfree_synth::params::POLYPHONY),
            Engine::Fm => self.fm.param(mpfree_synth::fm_params::POLYPHONY),
            Engine::Drums => mpfree_synth::drum::POOL as f32,
            Engine::Pluck => self.pluck.param(mpfree_synth::pluck_params::POLYPHONY),
            Engine::Wavetable => self.wt.param(mpfree_synth::wavetable::params::POLYPHONY),
        }
    }

    /// Each sounding voice's pressure, 0 to 1, for the Pressure page's live
    /// bars. Returns how many it wrote (none for Drums and Pluck, which
    /// don't take pressure yet).
    pub fn pressures(&self, out: &mut [f32]) -> usize {
        match self.settings.engine {
            Engine::Poly => self.poly.pressures(out),
            Engine::Fm => self.fm.pressures(out),
            Engine::Wavetable => self.wt.pressures(out),
            Engine::Drums | Engine::Pluck => 0,
        }
    }

    // ------------------------------------------------------------ the Synth tab

    /// Sets a parameter of the engine on the pads (clamped by the node).
    /// Drums' Pad 1 note and pad sounds move or rename the pads (see
    /// [`Synth::take_pads_changed`]).
    pub fn set(&mut self, id: u16, value: f32) {
        let names = self.drum_names();
        self.node_mut().set_param(id, value);
        if self.settings.engine == Engine::Drums && moves_drum_pads(id) {
            self.drums_moved(names);
        }
    }

    /// Drums' values changed (its pads were called `names` before): the
    /// pads follow, and the app hears of it if their notes or names changed.
    fn drums_moved(&mut self, names: [&'static str; PADS]) {
        let pads = self.pads;
        self.relayout();
        if (self.pads, self.drum_names()) != (pads, names) {
            self.pads_changed = true;
        }
    }

    pub fn value(&self, id: u16) -> f32 {
        self.node().param(id)
    }

    /// How many presets the engine on the pads has.
    pub fn preset_count(&self) -> usize {
        match self.settings.engine {
            Engine::Fm => self.fm_presets.len(),
            e => static_presets(e).len(),
        }
    }

    /// Loads preset `i` of the engine on the pads; returns its name, or None
    /// if there is no such preset. Notes keep sounding; values glide to the
    /// preset's. A Drums kit can move or rename the pads.
    pub fn load_preset(&mut self, i: usize) -> Option<&str> {
        let e = self.settings.engine;
        if e == Engine::Fm {
            let p = self.fm_presets.get(i)?;
            self.fm.load_preset(p);
            self.presets[e.index()] = i;
            return Some(&p.name);
        }
        let p = static_presets(e).get(i)?;
        match e {
            Engine::Drums => {
                let names = self.drum_names();
                self.drums.load_preset(p);
                self.drums_moved(names);
            }
            Engine::Pluck => self.pluck.load_preset(p),
            Engine::Wavetable => self.wt.load_preset(p),
            Engine::Poly | Engine::Fm => self.poly.load_preset(p),
        }
        self.presets[e.index()] = i;
        Some(p.name)
    }
}

/// User presets: files in the library's Instruments place, for the
/// browser's preset picker. Nothing calls these yet; the app wires the
/// picker in with the browser page.
#[allow(dead_code)]
impl Synth {
    /// The preset in `text` (a `.preset` file from `folder` under the
    /// library's Instruments place, where the folder is the engine's name)
    /// for the engine on the pads, read and checked; nothing changes.
    fn read_preset(&self, folder: &str, text: &str) -> Result<SoundPreset, PresetError> {
        let node = self.node();
        let info = node.info();
        // The folder must be this engine's, if the file sits in one; the
        // file's own engine line is checked by `parse` either way.
        if let Some(e) = Engine::from_folder(folder) {
            if e != self.settings.engine {
                return Err(PresetError::WrongEngine {
                    file: engine_of(text).unwrap_or(folder).to_string(),
                    node: info.id.to_string(),
                });
            }
        }
        SoundPreset::parse(text, info, node.params())
    }

    /// Puts a preset on the engine on the pads (notes keep sounding, values
    /// glide), as a factory preset does. Drums' pads follow their kit.
    fn put_preset(&mut self, p: &SoundPreset) -> Result<(), PresetError> {
        let names = self.drum_names();
        p.apply(self.node_mut())?;
        if self.settings.engine == Engine::Drums {
            self.drums_moved(names);
        }
        Ok(())
    }

    /// The preset picker turned to a preset (`BrowseAction::TryPreset`):
    /// plays it on the engine on the pads. The sound from before the first
    /// try is kept, for [`Synth::revert_preset`]. Returns the preset's
    /// name. A file for another engine, or one this build can't read,
    /// changes nothing.
    pub fn try_preset(&mut self, folder: &str, text: &str) -> Result<String, PresetError> {
        let p = self.read_preset(folder, text)?;
        if self.audition.is_none() {
            self.audition = Some(SoundPreset::capture(self.node(), "", &[]));
        }
        self.put_preset(&p)?;
        Ok(p.name)
    }

    /// The preset picker was cancelled (`BrowseAction::RevertPreset`): the
    /// sound from before its first try comes back, every setting as it
    /// was. Nothing happens if nothing was tried.
    pub fn revert_preset(&mut self) {
        if let Some(p) = self.audition.take() {
            // The capture is this engine's, so applying it can't fail.
            let _ = self.put_preset(&p);
        }
    }

    /// Loads a user preset (`BrowseAction::LoadPreset`, or the picker's
    /// Load after trying): the engine on the pads plays it from now on, and
    /// the sound from before any try is forgotten. Returns its name.
    pub fn load_preset_file(&mut self, folder: &str, text: &str) -> Result<String, PresetError> {
        let p = self.read_preset(folder, text)?;
        self.put_preset(&p)?;
        self.audition = None;
        Ok(p.name)
    }

    /// Whether the preset picker is trying presets (a sound is kept to go
    /// back to).
    pub fn trying_preset(&self) -> bool {
        self.audition.is_some()
    }

    /// The engine on the pads' sound now as preset text, and the folder it
    /// goes in under the library's Instruments place (the engine's name).
    /// `extra` lines ride along, such as the Q-Link sets (`qset`).
    pub fn save_preset(
        &self,
        name: &str,
        tags: &[&str],
        extra: &[(String, String)],
    ) -> (&'static str, String) {
        let node = self.node();
        let mut p = SoundPreset::capture(node, name, tags);
        p.extra.extend_from_slice(extra);
        (self.settings.engine.name(), p.to_text(node.params()))
    }
}

impl Synth {
    #[cfg(test)]
    pub fn preset(&self) -> usize {
        self.presets[self.settings.engine.index()]
    }

    pub fn preset_name(&self) -> &str {
        let i = self.presets[self.settings.engine.index()];
        match self.settings.engine {
            Engine::Fm => self.fm_presets.get(i).map_or("", |p| p.name.as_str()),
            e => static_presets(e).get(i).map_or("", |p| p.name),
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
        self.meters[self.settings.engine.index()]
            .iter()
            .map(|&i| (i, node.param(params[i].id)))
            .collect()
    }

    /// What the Synth tab shows.
    pub fn state(&self) -> SynthState {
        let presets = match self.settings.engine {
            Engine::Fm => self.fm_presets.iter().map(|p| p.name.clone()).collect(),
            e => static_presets(e)
                .iter()
                .map(|p| p.name.to_string())
                .collect(),
        };
        let preset = Some(self.presets[self.settings.engine.index()]);
        SynthState::from_node(
            self.node(),
            "Pads",
            Color::from_pad(PAD_WHITE),
            presets,
            preset,
        )
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
        let name = if p.group == p.name {
            p.name.to_string()
        } else {
            format!("{} {}", p.group, p.name)
        };
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

    /// A note on `node` from a fresh start, as auto-sampling plays it.
    fn note(node: &mut dyn Node, note: u8) -> Vec<f32> {
        const BLOCK: usize = 256;
        node.prepare(48_000, BLOCK);
        node.reset();
        let mut out = Vec::new();
        for b in 0..40 {
            let (mut l, mut r) = (vec![0f32; BLOCK], vec![0f32; BLOCK]);
            let ev = match b {
                0 => vec![Timed::new(
                    0,
                    NodeEvent::NoteOn {
                        note,
                        velocity: 100,
                    },
                )],
                24 => vec![Timed::new(0, NodeEvent::NoteOff { note })],
                _ => Vec::new(),
            };
            let ctx = Ctx::new(48_000, (b * BLOCK) as u64);
            node.process(&ctx, &ev, &mut l, &mut r);
            out.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
        }
        out
    }

    #[test]
    fn a_fresh_copy_of_every_engine_plays_the_patch_as_the_pads_do() {
        let mut s = Synth::new(48_000);
        for engine in [
            Engine::Poly,
            Engine::Fm,
            Engine::Drums,
            Engine::Pluck,
            Engine::Wavetable,
        ] {
            with(&mut s, |p| p.engine = engine);
            // A preset other than the first, with a value changed after.
            let i = 1.min(s.preset_count() - 1);
            s.load_preset(i);
            let edit = s
                .node()
                .params()
                .iter()
                .find(|p| !p.readonly && p.max > p.min)
                .map(|p| (p.id, p.min + (p.max - p.min) * 0.3));
            if let Some((id, v)) = edit {
                s.set(id, v);
            }
            let st = s.state();
            let mut copy = fresh_node(st.info.id, &st.values, 48_000).unwrap();
            assert_eq!(copy.info().id, st.info.id);
            for (p, v) in st.params.iter().zip(&st.values) {
                assert_eq!(copy.param(p.id), *v, "{} {}", st.info.name, p.name);
            }
            // The note pad 1 plays (a drum's, for Drums).
            let n = s.notes(0)[0];
            let theirs = note(s.node_mut(), n);
            let ours = note(copy.as_mut(), n);
            assert!(peak(&theirs) > 0.001, "{} sounds", st.info.name);
            assert_eq!(ours, theirs, "{} plays the same", st.info.name);
        }
        assert!(fresh_node("mpfree.nothing", &[], 48_000).is_none());
        assert!(fresh_node("mpfree.poly", &[0.5], 48_000).is_none());
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
        assert!(
            !queued(&s).contains(&NodeEvent::NoteOff { note: 48 }),
            "the arp still holds C3"
        );
        s.pressure_note(52, 70);
        s.pressure_note(55, 70);
        s.note_off_note(48, 20);
        s.note_off_note(52, 20);
        s.note_off_note(52, 21);
        assert_eq!(
            queued(&s),
            vec![
                NodeEvent::NoteOn {
                    note: 48,
                    velocity: 100
                },
                NodeEvent::NoteOn {
                    note: 48,
                    velocity: 90
                },
                NodeEvent::NoteOn {
                    note: 52,
                    velocity: 90
                },
                NodeEvent::Pressure {
                    note: 52,
                    value: 70
                },
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
        assert_eq!(
            (c[2], c[4], c[1]),
            (PAD_C, PAD_WHITE, PAD_BLACK),
            "D, E, C#"
        );
        assert_eq!(c[0], PAD_WHITE, "C is in D Dorian");
        for layout in PadLayout::ALL {
            with(&mut s, |p| p.set_layout(layout));
            for colour in s.pad_colours() {
                assert!(
                    colour == [0; 3] || crate::akai::RESTING_COLOURS.contains(&colour),
                    "{colour:02X?}"
                );
            }
        }
    }

    #[test]
    fn a_scale_lays_the_banks_out_and_the_quick_switch_flips_one() {
        let mut s = Synth::new(44_100);
        with(&mut s, |p| p.scale = scale("Minor", 9));
        // A: Chromatic, the notes as before.
        assert_eq!(s.notes(0), &[48]);
        assert_eq!(s.chip(), "Poly · A Minor · chromatic");
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
        assert_eq!(
            queued(&s),
            vec![
                on(52, 100),
                on(55, 100),
                on(48, 100),
                on(55, 90),
                on(59, 90),
                on(52, 90),
                press(52, 70),
                press(55, 70),
                press(48, 70),
                // E and G keep the harder press while pad 1 holds them.
                press(55, 70),
                press(59, 30),
                press(52, 70),
                // Pad 1 let go: only C ends; E and G still sound for pad 3.
                NodeEvent::NoteOff { note: 48 }
            ]
        );
        let mut frame = 0;
        run(&mut s, &mut frame, 1, 128);
        s.note_off(2, 0);
        let mut offs: Vec<NodeEvent> = queued(&s);
        offs.sort_by_key(|e| match e {
            NodeEvent::NoteOff { note } => *note,
            _ => 0,
        });
        assert_eq!(
            offs,
            vec![
                NodeEvent::NoteOff { note: 52 },
                NodeEvent::NoteOff { note: 55 },
                NodeEvent::NoteOff { note: 59 }
            ]
        );
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
            assert!(
                l[..10].iter().all(|&x| x == 0.25),
                "{engine:?}: nothing before the note's frame"
            );
            assert!(
                l.iter().any(|&x| x != 0.25),
                "{engine:?} adds to what is there"
            );
            assert_eq!(s.active_voices(), 1);
            let sound = run(&mut s, &mut frame, 40, 128);
            let p = peak(&sound);
            assert!(
                p > 0.05 && p < 1.0,
                "{engine:?}: a note at about -12 dBFS: peak {p}"
            );
            s.note_off(0, 0);
            run(&mut s, &mut frame, 2000, 128);
            assert_eq!(s.active_voices(), 0, "{engine:?}");
            assert!(peak(&run(&mut s, &mut frame, 2, 128)) == 0.0);
            assert_eq!(
                s.counters(),
                Counters {
                    notes: 1,
                    peak_voices: 1,
                    dropped: 0,
                    resets: 0
                },
                "{engine:?}"
            );
        }
    }

    /// Every writable setting of the engine on the pads, as bits.
    fn sound(s: &Synth) -> Vec<(u16, u32)> {
        let node = s.node();
        node.params()
            .iter()
            .filter(|p| !p.readonly)
            .map(|p| (p.id, node.param(p.id).to_bits()))
            .collect()
    }

    fn on_engine(e: Engine) -> Synth {
        let mut s = Synth::new(44_100);
        let mut p = s.settings();
        p.engine = e;
        s.set_settings(p);
        s
    }

    #[test]
    fn the_preset_picker_tries_reverts_and_loads_user_presets() {
        let mut s = Synth::new(44_100);
        let before = sound(&s);
        // Two user presets: Poly's factory sounds 3 and 5 saved as files.
        let mut other = Synth::new(44_100);
        other.load_preset(3);
        let (folder, mine) = other.save_preset("Mine", &["test"], &[]);
        assert_eq!(folder, "Poly");
        let want = sound(&other);
        other.load_preset(5);
        let (_, two) = other.save_preset("Two", &[], &[]);
        assert_ne!(before, want);
        assert_ne!(sound(&other), want);
        // Turning the dial plays each; the sound from before the first try
        // is kept.
        assert_eq!(s.try_preset("Poly", &mine).as_deref(), Ok("Mine"));
        assert!(s.trying_preset());
        assert_eq!(sound(&s), want);
        assert_eq!(s.try_preset("poly", &two).as_deref(), Ok("Two"));
        // Cancel puts the first sound back, every setting to the bit.
        s.revert_preset();
        assert!(!s.trying_preset());
        assert_eq!(sound(&s), before);
        s.revert_preset();
        assert_eq!(sound(&s), before, "nothing tried, nothing to put back");
        // Load after trying keeps what was tried.
        s.try_preset("Poly", &two).unwrap();
        assert_eq!(s.load_preset_file("Poly", &mine).as_deref(), Ok("Mine"));
        assert!(!s.trying_preset());
        s.revert_preset();
        assert_eq!(sound(&s), want);
        // Another engine's preset, a file in another engine's folder and a
        // file that isn't a preset change nothing and keep nothing.
        let (fm_folder, bell) = on_engine(Engine::Fm).save_preset("Bell", &[], &[]);
        assert_eq!(fm_folder, "FM");
        assert!(matches!(
            s.try_preset("FM", &bell),
            Err(PresetError::WrongEngine { .. })
        ));
        assert!(s.try_preset("", &bell).is_err(), "FM's engine line");
        assert!(
            s.try_preset("FM", &mine).is_err(),
            "a Poly file in FM's folder"
        );
        assert!(s.try_preset("Poly", "not a preset").is_err());
        assert!(!s.trying_preset());
        assert_eq!(sound(&s), want);
        // A file straight in Instruments goes by its engine line.
        assert!(s.try_preset("", &two).is_ok());
        s.revert_preset();
        assert_eq!(sound(&s), want);
        // Folders are engines' names, in any case; the Q-Link sets ride
        // along in a saved preset.
        assert_eq!(Engine::from_folder("wavetable"), Some(Engine::Wavetable));
        assert_eq!(Engine::from_folder("Strings"), None);
        let qset = [("qset".to_string(), "31 32 - 33 Tone".to_string())];
        let (_, text) = s.save_preset("Q", &[], &qset);
        assert!(text.contains("\nqset = 31 32 - 33 Tone\n"), "{text}");
    }

    #[test]
    fn a_drum_kit_preset_moves_the_pads_and_cancel_moves_them_back() {
        let mut s = on_engine(Engine::Drums);
        s.take_pads_changed();
        let names = s.drum_names();
        // The first factory kit whose pads differ from the one playing.
        let mut other = on_engine(Engine::Drums);
        let kit = (0..other.preset_count())
            .find(|&i| {
                other.load_preset(i);
                other.drum_names() != names
            })
            .expect("a kit with other pads");
        other.load_preset(kit);
        let (folder, text) = other.save_preset("Kit", &[], &[]);
        assert_eq!(folder, "Drums");
        s.try_preset("Drums", &text).unwrap();
        assert_eq!(s.drum_names(), other.drum_names());
        assert!(s.take_pads_changed(), "the pads follow the kit tried");
        s.revert_preset();
        assert_eq!(s.drum_names(), names);
        assert!(s.take_pads_changed(), "and come back on cancel");
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
        assert_eq!(
            (s.poly.active_voices(), s.fm.active_voices()),
            (1, 1),
            "Poly releases while FM plays"
        );
        run(&mut s, &mut frame, 400, 128);
        assert_eq!(s.poly.active_voices(), 0, "Poly's release ended");
        assert_eq!(s.fm.active_voices(), 1);
        // The Synth tab edits FM now.
        let id = mpfree_synth::fm_params::LEVEL;
        s.set(id, -6.0);
        assert_eq!(s.value(id), -6.0);
        assert_eq!(
            s.poly.param(pp::LEVEL),
            Poly::new(44_100).param(pp::LEVEL),
            "Poly untouched"
        );
        assert_eq!(s.load_preset(2), Some("Bell Tower"));
        assert_eq!(s.preset_count(), Fm::presets().len());
        assert_eq!(s.kit_view().name, "Synth: FM");
        assert!(s.chip().starts_with("FM · "));
    }

    #[test]
    fn the_same_notes_sound_the_same_in_any_block_size() {
        // (frame, pad, on); periods longer than the scratch buffers render in pieces.
        let script = [
            (3u64, 0usize, true),
            (64, 4, true),
            (500, 7, true),
            (2000, 0, false),
            (2100, 4, false),
            (2600, 7, false),
            (3000, 2, true),
            (4000, 2, false),
        ];
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
                            if on {
                                s.note_on(pad, 90, offset)
                            } else {
                                s.note_off(pad, offset)
                            }
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
        s.load_preset(
            Poly::presets()
                .iter()
                .position(|p| p.name == "Dust Pad")
                .unwrap(),
        );
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
        assert_eq!(s.len, [0; ENGINES]);
    }

    #[test]
    fn note_repeat_retriggers_and_pressure_follows_the_pad() {
        let mut s = Synth::new(44_100);
        s.note_on(2, 100, 0);
        s.pressure(2, 90);
        s.note_on(2, 100, 50);
        s.pressure(9, 90); // not playing: nothing
        assert_eq!(
            queued(&s),
            vec![
                NodeEvent::NoteOn {
                    note: 50,
                    velocity: 100
                },
                NodeEvent::Pressure {
                    note: 50,
                    value: 90
                },
                NodeEvent::NoteOff { note: 50 },
                NodeEvent::NoteOn {
                    note: 50,
                    velocity: 100
                }
            ]
        );
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
        assert_eq!(s.len, [0; ENGINES]);
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
        assert!(meters
            .iter()
            .any(|&(i, _)| st.params[i].id == pp::METER_VOICES));
        assert_eq!(s.polyphony(), 8.0);
        // FM's own presets, values and meters.
        with(&mut s, |p| p.engine = Engine::Fm);
        let st = s.state();
        assert_eq!(st.presets.len(), s.preset_count());
        assert_eq!(st.values, s.values());
        assert!(s
            .meters()
            .iter()
            .any(|&(i, _)| st.params[i].id == mpfree_synth::fm_params::METER_VOICES));
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
                if p % 113 == 0 {
                    // Drums' Pad 1 note and a pad's sound move the pads.
                    s.set(drum_params::BASE_NOTE, 30.0 + (p % 20) as f32);
                    s.set(
                        drum_params::pad_id(p % 16, drum_params::field::MODEL),
                        (p % 4) as f32,
                    );
                    let _ = s.take_pads_changed();
                }
                if p % 733 == 0 {
                    s.shift_octave(if p % 2 == 0 { 1 } else { -1 });
                }
                if p % 211 == 0 {
                    let mut set = s.settings();
                    set.scale = if p % 2 == 0 { minor } else { None };
                    set.bank = p / 211 % BANKS;
                    set.engine = Engine::ALL[p / 300 % ENGINES];
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
    fn drums_play_the_kits_notes_whatever_the_scale_layout_or_octave() {
        let mut s = Synth::new(44_100);
        with(&mut s, |p| {
            p.scale = scale("Minor", 9);
            p.set_layout(PadLayout::Chords);
        });
        s.shift_octave(1);
        with(&mut s, |p| p.engine = Engine::Drums);
        assert!(
            !s.take_pads_changed(),
            "the app sends the pads on a switch itself"
        );
        // Pad 1 is the kit's Pad 1 note (C2), a semitone a pad.
        for pad in 0..PADS {
            assert_eq!(s.notes(pad), &[36 + pad as u8]);
        }
        assert_eq!(s.range_text(), "C2 to D#3");
        assert!(!s.shift_octave(1), "a kit has no octaves");
        assert_eq!(s.notes(0), &[36]);
        // Banks and the In key switch don't move it either.
        with(&mut s, |p| {
            p.bank = 3;
            p.set_layout(PadLayout::InKey);
        });
        assert_eq!(s.notes(15), &[51]);
        // Named by their sounds, in the kit's colour; the chip names the kit.
        let labels = s.pad_labels();
        assert_eq!(labels[0].as_deref(), Some(s.drums.pad_name(0)));
        assert_eq!(labels[0].as_deref(), Some("Kick"));
        assert_eq!(s.pad_colours(), [PAD_WHITE; 16]);
        assert_eq!(s.chip(), "Drums · Init Kit");
        assert_eq!(s.kit_view().name, "Synth: Drums");
        // A pad hit plays that pad's note on Drums.
        s.note_on(1, 100, 0);
        assert_eq!(
            queued(&s),
            [NodeEvent::NoteOn {
                note: 37,
                velocity: 100
            }]
        );
        // The Pad 1 note moves every pad; the app hears of it once.
        s.set(drum_params::BASE_NOTE, 48.0);
        assert_eq!(s.notes(0), &[48]);
        assert_eq!(s.range_text(), "C3 to D#4");
        assert!(s.take_pads_changed());
        assert!(!s.take_pads_changed());
        s.set(drum_params::BASE_NOTE, 48.0);
        assert!(!s.take_pads_changed(), "no change, no news");
        // Pad 1 held before the move still ends its own note.
        s.note_off(1, 0);
        assert_eq!(queued(&s).last(), Some(&NodeEvent::NoteOff { note: 37 }));
        // A pad's sound renames it.
        let model = drum_params::pad_id(3, drum_params::field::MODEL);
        s.set(model, 0.0);
        assert_eq!(s.pad_labels()[3].as_deref(), Some("Kick"));
        assert!(s.take_pads_changed());
        // Other knobs don't touch the pads.
        s.set(drum_params::pad_id(3, drum_params::field::DECAY), 300.0);
        assert!(!s.take_pads_changed());
        // A kit: its name, and the app hears if the pads' names changed.
        assert_eq!(s.load_preset(1), Some("Boom Kit"));
        assert_eq!(s.preset_name(), "Boom Kit");
        assert_eq!(s.preset_count(), Drums::presets().len());
        assert_eq!(s.state().presets.len(), Drums::presets().len());
        assert!(s.take_pads_changed(), "pad 4 is back to the kit's sound");
        // Back to Poly: the layout, the scale and the octave are as they were.
        with(&mut s, |p| p.engine = Engine::Poly);
        assert_eq!(s.octave(), 1);
        assert_eq!(s.pad_layout(), pads::layout(&s.setup()));
        assert_ne!(s.notes(0), &[48], "bank D, In key in A minor, an octave up");
        assert!(s.shift_octave(-1));
    }

    #[test]
    fn pluck_and_wavetable_play_the_pad_layouts_as_poly_does() {
        let cases = [
            (
                Engine::Pluck,
                "Pluck",
                PLUCK_START_PRESET,
                mpfree_synth::pluck_params::POLYPHONY,
            ),
            (
                Engine::Wavetable,
                "Wavetable",
                WT_START_PRESET,
                mpfree_synth::wavetable::params::POLYPHONY,
            ),
        ];
        for (engine, name, start, polyphony) in cases {
            let mut poly = Synth::new(44_100);
            let mut other = Synth::new(44_100);
            with(&mut other, |p| p.engine = engine);
            for s in [&mut poly, &mut other] {
                with(s, |p| {
                    p.scale = scale("Dorian", 2);
                    p.set_layout(PadLayout::Chords);
                });
                s.shift_octave(-1);
            }
            for pad in 0..PADS {
                assert_eq!(poly.notes(pad), other.notes(pad), "{name} pad {pad}");
            }
            assert_eq!(other.pad_labels(), poly.pad_labels());
            assert!(other.chip().starts_with(&format!("{name} · D Dorian")));
            assert_eq!(other.preset_name(), start);
            assert_eq!(other.polyphony(), other.value(polyphony));
            // A chord pad: every note of it reaches the engine.
            other.note_on(0, 90, 0);
            assert_eq!(queued(&other).len(), other.notes(0).len());
            assert!(!other.take_pads_changed());
        }
    }

    #[test]
    fn knob_turns_log_one_line_per_rest() {
        let s = Synth::new(44_100);
        let mut log = ParamLog::default();
        let t0 = Instant::now();
        assert_eq!(log.set(&s, pp::CUTOFF, 300.0, t0), None);
        assert_eq!(log.set(&s, pp::CUTOFF, 310.0, t0), None);
        assert_eq!(log.poll(&s, t0 + Duration::from_millis(500)), None);
        let line = log
            .set(&s, pp::RESONANCE, 40.0, t0 + Duration::from_millis(600))
            .unwrap();
        assert!(line.starts_with("synth: Filter Cutoff "), "{line}");
        assert!(log
            .poll(&s, t0 + Duration::from_millis(1700))
            .unwrap()
            .starts_with("synth: Filter Resonance "));
        assert_eq!(log.poll(&s, t0 + Duration::from_secs(5)), None);
    }

    #[test]
    fn scale_names_read_as_root_and_name() {
        assert_eq!(scale_name(&scale("Dorian", 2).unwrap()), "D Dorian");
        assert_eq!(scale_name(&Scale::new(9, 0b1001_0010_1001)), "A, 5 notes");
    }
}
