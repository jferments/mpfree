//! The pad player's synth: MPFree's Poly, FM, Drums, Pluck, Keys,
//! Drawbar, Squelch, Analog, Strings, Bass, Wind or Choir (`mpfree-synth`)
//! on the pads, edited on the Synth tab (`mpfree-synth-ui`). The Wavetable
//! synth went into Poly on 19 Nov: its sounds, presets and tracks load as
//! Poly's ([`poly_text`], [`as_poly`]).
//!
//! - The kit list ends with one more entry, "Synth: Poly" (or FM, Drums,
//!   Pluck, Keys, Drawbar, Squelch, Analog, Strings, Bass, Wind, Choir).
//!   Picking it, or opening the Synth tab, makes the pads play the
//!   synth. The Synth tab's options (MENU) pick the engine; a note still
//!   ringing on another engine finishes its release.
//! - Drums is a kit, not a keyboard: pad 1 plays the kit's Pad 1 note (a
//!   Kit page knob, C2 by default) and each pad after it a semitone higher,
//!   whatever the scale, layout, bank or octave. The pads are named by the
//!   sound on them ("Kick", "Closed hat"), and the Synth tab shows the page
//!   of the pad hit last. Pluck, Keys, Drawbar, Squelch, Analog,
//!   Strings, Bass, Wind and Choir play the pad layouts as Poly and FM do
//!   (Squelch and Bass one note at a time, Wind one by default).
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
use mpfree_arp::scale::{Scale, Shift, NOTE_NAMES};
use mpfree_instruments::preset::{engine_of, Error as PresetError};
use mpfree_instruments::sound::{has_design, is_design_key};
use mpfree_instruments::{Design, Designed, Sound, SoundError};
use mpfree_node::pedal::Pedals;
use mpfree_node::room::{Limits, NodeRoom};
use mpfree_node::{Ctl, Ctx, Cull, Node, NodeEvent, ParamInfo, Timed};
use mpfree_synth::drum_params;
use mpfree_synth::{
    Analog, Bass, Choir, Drawbar, Drums, Fm, Keys, Pluck, Poly, Preset, Squelch, Strings,
    Wavetable, Wind,
};
use mpfree_synth_ui::format::value_line;
use mpfree_synth_ui::SynthState;
pub use mpfree_tuning::Builtin;
use mpfree_tuning::{root_hz, scala, Tuning};
use mpfree_ui::Color;
use std::time::{Duration, Instant};

/// The start of the kit list's entry for the synth ("Synth: Poly").
pub const KIT_PREFIX: &str = "Synth: ";
/// The preset each engine starts with.
pub const START_PRESET: &str = "Pluck";
pub const FM_START_PRESET: &str = "Glass Keys";
pub const DRUMS_START_PRESET: &str = "Init Kit";
pub const PLUCK_START_PRESET: &str = "Steel String";
/// The electric piano's preset at the start.
pub const KEYS_START_PRESET: &str = "Init";
/// The tonewheel organ's preset at the start.
pub const DRAWBAR_START_PRESET: &str = "Init";
/// The acid bass's preset at the start.
pub const SQUELCH_START_PRESET: &str = "Acid Line";
/// The vintage poly's preset at the start.
pub const ANALOG_START_PRESET: &str = "Init";
/// The string machine's preset at the start.
pub const STRINGS_START_PRESET: &str = "Init";
/// The bass engine's preset at the start.
pub const BASS_START_PRESET: &str = "Init";
/// The wind engine's preset at the start.
pub const WIND_START_PRESET: &str = "Init";
/// The choir's preset at the start.
pub const CHOIR_START_PRESET: &str = "Init";
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
/// What one period of an engine's queue takes (refactor A2b,
/// `mpfree_node::room`): new notes and values have room of their own, and
/// an ending (a note-off, a pedal coming up, a lock put back, a panic)
/// always finds a place. Unlocks: two for each lock the table can hold
/// ([`crate::locks::HOLDS`]), as [`Synth::lock`] asks before each new one.
pub const ROOM: Limits = Limits {
    starts: 128,
    values: 256,
    ends: 256,
    unlocks: 2 * crate::locks::HOLDS,
    all_offs: 16,
};
/// Places in each engine's queue: all its room lets in.
pub const MAX_EVENTS: usize = ROOM.room();
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
const ENGINES: usize = 12;

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
    /// The electric piano: struck tines and reeds.
    Keys,
    /// The tonewheel organ and its rotary speaker.
    Drawbar,
    /// The mono acid bass: diode ladder, accent and slide.
    Squelch,
    /// The vintage analog poly: voice cards that differ, poly-mod, chorus.
    Analog,
    /// The string machine: a divide-down bank, registers, swell, Ensemble.
    Strings,
    /// The one-voice bass: synth basses, 808s and reeses.
    Bass,
    /// Blown instruments: a reed or an air jet on a waveguide bore.
    Wind,
    /// Sung vowels: many singers on each note, five formants, Ensemble.
    Choir,
}

impl Engine {
    pub const ALL: [Engine; ENGINES] = [
        Engine::Poly,
        Engine::Fm,
        Engine::Drums,
        Engine::Pluck,
        Engine::Keys,
        Engine::Drawbar,
        Engine::Squelch,
        Engine::Analog,
        Engine::Strings,
        Engine::Bass,
        Engine::Wind,
        Engine::Choir,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Engine::Poly => "Poly",
            Engine::Fm => "FM",
            Engine::Drums => "Drums",
            Engine::Pluck => "Pluck",
            Engine::Keys => "Keys",
            Engine::Drawbar => "Drawbar",
            Engine::Squelch => "Squelch",
            Engine::Analog => "Analog",
            Engine::Strings => "Strings",
            Engine::Bass => "Bass",
            Engine::Wind => "Wind",
            Engine::Choir => "Choir",
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    /// The engine's node id (`mpfree.poly` ...), as a track's own
    /// instrument and the project name it.
    pub fn node_id(self) -> &'static str {
        match self {
            Engine::Poly => "mpfree.poly",
            Engine::Fm => "mpfree.fm",
            Engine::Drums => "mpfree.drums",
            Engine::Pluck => "mpfree.pluck",
            Engine::Keys => "mpfree.keys",
            Engine::Drawbar => "mpfree.drawbar",
            Engine::Squelch => "mpfree.squelch",
            Engine::Analog => "mpfree.analog",
            Engine::Strings => "mpfree.strings",
            Engine::Bass => "mpfree.bass",
            Engine::Wind => "mpfree.wind",
            Engine::Choir => "mpfree.choir",
        }
    }

    /// The engine whose node id is `id`.
    pub fn from_node_id(id: &str) -> Option<Engine> {
        Engine::ALL.into_iter().find(|e| e.node_id() == id)
    }

    /// The engine whose user presets live in `folder` under the library's
    /// Instruments place: the folder is the engine's name ("Poly", "FM"),
    /// in any case. The Wavetable synth's folder is Poly's: its sounds
    /// load as Poly's ([`poly_text`]).
    pub fn from_folder(folder: &str) -> Option<Engine> {
        Engine::from_name(folder)
    }

    /// The engine named `name` ("Poly", "FM"), in any case, as the
    /// project's synth part and the Instrument pads name it; "Wavetable",
    /// which went into Poly on 19 Nov, is Poly.
    pub fn from_name(name: &str) -> Option<Engine> {
        let name = name.trim();
        if name.eq_ignore_ascii_case("Wavetable") {
            return Some(Engine::Poly);
        }
        Engine::ALL
            .into_iter()
            .find(|e| e.name().eq_ignore_ascii_case(name))
    }
}

/// The Wavetable synth's node id: it went into Poly on 19 Nov, and a
/// sound saved for it loads as Poly's ([`poly_text`], [`as_poly`]).
pub const WAVETABLE: &str = "mpfree.wavetable";

/// Preset `text` saved for the Wavetable synth as Poly's: the same sound,
/// with its effects, its macros (their maps on Poly's settings) and its
/// Q-Link sets ([`mpfree_synth::wavetable::to_poly`]). Any other preset
/// comes back as it is. Err: the Wavetable preset didn't read. Allocates.
pub fn poly_text(text: &str) -> Result<std::borrow::Cow<'_, str>, SoundError> {
    if engine_of(text) != Some(WAVETABLE) {
        return Ok(std::borrow::Cow::Borrowed(text));
    }
    let mut wt = Sound::new(Box::new(Wavetable::new(48_000)), &Design::default())?;
    let p = wt.load_text(text)?;
    let tags: Vec<&str> = p.tags.iter().map(String::as_str).collect();
    let (poly, extra) = as_poly_sound(&wt, &p.name, &tags, &p.extra)?;
    Ok(std::borrow::Cow::Owned(
        poly.to_text(&p.name, &tags, &extra),
    ))
}

/// A Wavetable synth's sound (a bare engine, or one with effects and
/// macros) as Poly's, as [`poly_text`] converts it: what a project's
/// track saved for the Wavetable synth plays on Poly. A node of another
/// engine comes back as it was. Allocates.
pub fn as_poly(node: Box<dyn Node>) -> Result<Box<dyn Node>, SoundError> {
    if node.info().id != WAVETABLE {
        return Ok(node);
    }
    let (s, _) = as_poly_sound(&*node, "", &[], &[])?;
    if s.design_lines().is_empty() {
        let mut bare = Poly::new(48_000);
        for p in s.params() {
            if !p.readonly {
                bare.set_param(p.id, s.param(p.id));
            }
        }
        return Ok(Box::new(bare));
    }
    Ok(Box::new(s))
}

/// A preset's `key = value` lines other than its settings.
type Lines = Vec<(String, String)>;

/// The Wavetable sound `wt` as a Poly sound, and its `extra` lines (other
/// than its design) with the Q-Link sets on Poly's settings.
fn as_poly_sound(
    wt: &dyn Node,
    name: &str,
    tags: &[&str],
    extra: &[(String, String)],
) -> Result<(Sound<Poly>, Lines), SoundError> {
    let mut p = mpfree_node::preset::Preset::capture(wt, name, tags);
    p.extra = wt.save_extras();
    p.extra.extend(extra.iter().cloned());
    let p = mpfree_synth::wavetable::to_poly::preset_to_poly(&p);
    let (design, extra): (Vec<_>, Vec<_>) =
        p.extra.into_iter().partition(|(k, _)| is_design_key(k));
    let mut s = Sound::new(Box::new(Poly::new(48_000)), &Design::default())?;
    if !design.is_empty() {
        s.set_design_lines(&design)?;
    }
    for &(id, v) in &p.values {
        s.set_param(id, v);
    }
    Ok((s, extra))
}

/// The lowest and highest A4 the Synth tab's options offer, in Hz.
pub const A4_MIN: u16 = 380;
pub const A4_MAX: u16 = 480;

/// The most tuning files the tunings folder offers.
pub const MAX_TUNING_FILES: usize = 64;

/// A tuning from a Scala file the musician put in the tunings folder
/// (files/tunings/ on the data partition): its scale (`NAME.scl`) and,
/// with a keyboard map beside it (`NAME.kbm`), the tuning the map fixes.
#[derive(Clone, Debug, PartialEq)]
pub struct TuningFile {
    /// The file's name without `.scl`, as the options menu shows it.
    pub name: String,
    scale: scala::Scale,
    /// The tuning the keyboard map gives, root and pitch included; None
    /// without a map.
    mapped: Option<Tuning>,
}

impl TuningFile {
    /// Reads a scale and its keyboard map, if there is one, or says why
    /// not. Allocates: call it before the sound starts.
    pub fn parse(name: &str, scl: &str, kbm: Option<&str>) -> Result<TuningFile, String> {
        let scale = scala::parse_scl(scl).map_err(|e| e.to_string())?;
        let mapped = match kbm {
            Some(text) => {
                let map = scala::parse_kbm(text).map_err(|e| format!("its .kbm: {e}"))?;
                Some(scala::tuning(&scale, &map).map_err(|e| e.to_string())?)
            }
            None => None,
        };
        Ok(TuningFile {
            name: name.to_string(),
            scale,
            mapped,
        })
    }

    /// Whether a keyboard map sets the root and the pitch, so the Tuning
    /// root and A4 options don't apply.
    pub fn mapped(&self) -> bool {
        self.mapped.is_some()
    }

    /// Notes in one repeat of the scale (12 for most).
    pub fn notes(&self) -> usize {
        self.scale.len()
    }

    /// The tuning: the keyboard map's, or else the scale with its 1/1 on
    /// pitch class `root` (0 = C, in octave 4), which keeps its 12-TET
    /// pitch for `a4_hz`, and each key the next note of the scale, as the
    /// built-in tunings do. Never allocates.
    pub fn tuning(&self, root: u8, a4_hz: f32) -> Tuning {
        match &self.mapped {
            Some(t) => t.clone(),
            None => {
                let key = 60 + i32::from(root % 12);
                self.scale.rooted(key, root_hz(key, a4_hz))
            }
        }
    }
}

/// The synth's tuning, from the Synth tab's options: a built-in tuning or
/// a tuning file, its root for the tunings that have one (0 = C to 11 = B)
/// and A4 in whole Hz. Every engine but Drums plays through it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TuningPick {
    pub builtin: Builtin,
    /// A tuning file (its place in the synth's list,
    /// [`Synth::tuning_files`]), which plays instead of `builtin`.
    pub file: Option<u8>,
    pub root: u8,
    pub a4_hz: u16,
}

impl Default for TuningPick {
    fn default() -> Self {
        TuningPick {
            builtin: Builtin::Equal12,
            file: None,
            root: 0,
            a4_hz: 440,
        }
    }
}

impl TuningPick {
    /// The tuning file picked, out of `files`.
    pub fn file<'a>(&self, files: &'a [TuningFile]) -> Option<&'a TuningFile> {
        files.get(usize::from(self.file?))
    }

    /// The tuning itself, with `files` the tuning files offered. Never
    /// allocates.
    pub fn tuning(&self, files: &[TuningFile]) -> Tuning {
        let a4 = f32::from(self.a4_hz.clamp(A4_MIN, A4_MAX));
        match self.file(files) {
            Some(f) => f.tuning(self.root % 12, a4),
            None => self.builtin.tuning(self.root % 12, a4),
        }
    }

    /// "Just (5-limit) on D, A4 432 Hz"; "12-TET" on its own at 440; a
    /// tuning file by its name ("werckmeister3 on C"), alone when its
    /// keyboard map sets the root and the pitch.
    pub fn label(&self, files: &[TuningFile]) -> String {
        let (name, has_root, fixed) = match self.file(files) {
            Some(f) => (f.name.as_str(), true, f.mapped()),
            None => (self.builtin.name(), self.builtin.has_root(), false),
        };
        if fixed {
            return name.to_string();
        }
        let root = if has_root {
            format!(" on {}", NOTE_NAMES[usize::from(self.root % 12)])
        } else {
            String::new()
        };
        let a4 = if self.a4_hz == 440 {
            String::new()
        } else {
            format!(", A4 {} Hz", self.a4_hz)
        };
        format!("{name}{root}{a4}")
    }
}

/// How the synth's pads are set up, from the Synth tab's options: the
/// engine, the scale (None is Off), how In key fills its rows, each bank's
/// layout as (layout, octaves above the pads' octave), and the tuning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadSettings {
    pub engine: Engine,
    pub scale: Option<Scale>,
    pub rows: InKeyRows,
    pub banks: [(PadLayout, i8); BANKS],
    pub bank: usize,
    pub tuning: TuningPick,
}

impl Default for PadSettings {
    fn default() -> Self {
        PadSettings {
            engine: Engine::Poly,
            scale: None,
            rows: InKeyRows::Sequence,
            banks: DEFAULT_BANKS,
            bank: 0,
            tuning: TuningPick::default(),
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
    /// Notes the synth had no voice for, and hits on a pad past the 16
    /// (both should stay 0).
    pub dropped: u64,
    /// New notes, values and locks left out of a period too busy for them
    /// (refactor A2b): something new gave way so nothing hangs.
    pub busy: u64,
    /// Endings (note-offs, pedals up, locks put back, panics) that found
    /// no room: a bug, which must stay 0.
    pub ends_lost: u64,
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

/// Who started a pad's notes on the shared synth (instrument redesign step
/// 4): the pads straight on it (the synth chosen for the pads, a step's or
/// a scale row's audition) or a track (its clip, or the pads through its
/// arp). Each keeps its own notes, so two tracks on one pad, or a track
/// and the pads, never end each other's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Player {
    Pads,
    Track(usize),
}

/// The players the synth keeps notes for: the pads, then every track.
const PLAYERS: usize = 1 + mpfree_seq::MAX_TRACKS;
/// Words of the bit set of players that hold a pad now.
const PLAYER_WORDS: usize = PLAYERS.div_ceil(128);

impl Player {
    /// Its row in the synth's tables; None for a track past the last.
    fn index(self) -> Option<usize> {
        match self {
            Player::Pads => Some(0),
            Player::Track(t) if t < mpfree_seq::MAX_TRACKS => Some(1 + t),
            Player::Track(_) => None,
        }
    }
}

/// The players in `set` (a bit per player), lowest first.
fn players(set: [u128; PLAYER_WORDS]) -> impl Iterator<Item = usize> {
    (0..PLAYER_WORDS).flat_map(move |w| {
        let mut bits = set[w];
        std::iter::from_fn(move || {
            if bits == 0 {
                return None;
            }
            let i = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            Some(w * 128 + i)
        })
    })
}

pub struct Synth {
    // Each engine as a sound ([`Sound`]): the engine, and the effects and
    // macros a preset brings (none for a factory preset yet). The engine's
    // own calls reach it through the sound.
    poly: Sound<Poly>,
    fm: Sound<Fm>,
    drums: Sound<Drums>,
    pluck: Sound<Pluck>,
    keys: Sound<Keys>,
    drawbar: Sound<Drawbar>,
    squelch: Sound<Squelch>,
    analog: Sound<Analog>,
    strings: Sound<Strings>,
    bass: Sound<Bass>,
    wind: Sound<Wind>,
    choir: Sound<Choir>,
    rate: u32,
    /// Note events for each engine (in [`Engine::ALL`]'s order) this period.
    events: Box<[[Timed; MAX_EVENTS]; ENGINES]>,
    len: [usize; ENGINES],
    /// Which events go in each engine's queue, so an ending always fits.
    rooms: [NodeRoom; ENGINES],
    /// Hits on a pad past the 16, and endings with no place in the queue
    /// though their room let them in (a bug).
    dropped_events: u64,
    ends_lost: u64,
    /// The sustain and sostenuto pedals, holding the notes of the engine on
    /// the pads while it doesn't hear them itself (`Node::hears`).
    pedals: Pedals,
    left: [f32; SCRATCH],
    right: [f32; SCRATCH],
    settings: PadSettings,
    octave: i32,
    /// The pads' notes, worked out when a setting changes.
    pads: [Pad; PADS],
    /// The notes each player's pads started ([`Player`]), so a pad's
    /// note-off ends that player's notes even after the layout or octave
    /// changed, and never another's. Made here, never on the audio thread.
    playing: Box<[[Held; PADS]; PLAYERS]>,
    /// The players that hold a pad now (a bit each), so a panic or a
    /// pressure looks only at those.
    holding: [u128; PLAYER_WORDS],
    /// How many pads hold each note now.
    holders: [u8; 128],
    /// Each player's pads' last pressure, 0 to 127.
    press: Box<[[u8; PADS]; PLAYERS]>,
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
    /// How many times each engine's design (its preset's effects and
    /// macros) changed: the synth-voice pads' copies of an engine
    /// (`pad_synths.rs`) are made again when it moves.
    designs: [u32; ENGINES],
    /// The sound from before the preset picker's first try, to put back if
    /// the picker is cancelled ([`Synth::try_preset`]).
    audition: Option<Tried>,
    /// The musician's preset each engine plays, by its name, when it came
    /// from Yours (a preset file) and not the engine's own list; shown as
    /// the Synth tab's title.
    own: [Option<String>; ENGINES],
    /// The tuning files offered after the built-in tunings.
    tuning_files: Vec<TuningFile>,
    /// The values under the clips' locks on the engines' parameters
    /// ([`Synth::lock`]).
    locks: crate::locks::Under,
    /// The note each kit pad of the Instrument type holds (the Pads tab's
    /// Instrument pads: [`Synth::pad_voice_on`]), by track and pad, and its
    /// engine: so its end and pressure reach that note whatever the pad
    /// says by then.
    pad_voices: Box<[Option<(Engine, u8)>]>,
    /// How many of those hold each engine's note: a note two pads hold
    /// ends with the last of them, and with the pads' own notes when the
    /// engine is the one on the pads.
    pad_holders: [[u8; 128]; ENGINES],
}

/// The tracks an Instrument pad can be on, and the pads of a kit.
const VOICE_TRACKS: usize = mpfree_seq::MAX_TRACKS;
const VOICE_PADS: usize = 128;

/// Where pad `pad` of track `track` keeps its voice in
/// [`Synth::pad_voices`]; None past the tracks or the kit's pads.
fn voice_key(track: u16, pad: usize) -> Option<usize> {
    let t = usize::from(track);
    (t < VOICE_TRACKS && pad < VOICE_PADS).then_some(t * VOICE_PADS + pad)
}

/// The key of engine `e`'s parameter `id` in [`Synth`]'s lock table.
fn lock_key(e: Engine, id: u16) -> u32 {
    (e.index() as u32) << 16 | u32::from(id)
}

/// An engine's factory presets.
fn static_presets(engine: Engine) -> &'static [Preset] {
    match engine {
        Engine::Drums => Drums::presets(),
        Engine::Pluck => Pluck::presets(),
        Engine::Keys => Keys::presets(),
        Engine::Drawbar => Drawbar::presets(),
        Engine::Squelch => Squelch::presets(),
        Engine::Analog => Analog::presets(),
        Engine::Strings => Strings::presets(),
        Engine::Bass => Bass::presets(),
        Engine::Wind => Wind::presets(),
        Engine::Choir => Choir::presets(),
        Engine::Fm => Fm::presets(),
        Engine::Poly => Poly::presets(),
    }
}

/// An engine as a sound with nothing added, its effects (none until a
/// preset brings some) ready for the synth's rate and blocks. Allocates.
fn plain<E: Node>(engine: E, rate: u32) -> Sound<E> {
    let mut s = Sound::new(Box::new(engine), &Design::default())
        .expect("every engine's setting ids are below a sound's own");
    s.prepare_effects(rate, SCRATCH);
    s
}

/// A new instrument of the engine `id` (its [`mpfree_node::NodeInfo::id`])
/// at `rate`, as the sound `extras` describe (its design lines, as
/// [`Node::save_extras`] gives them: none for a plain engine), with
/// `values` (every parameter, in the node's order, as [`Synth::values`]
/// and the Synth tab's state hold them) set: a copy of the patch for a
/// track or for auto-sampling off the audio thread, which keeps playing its
/// own. The meters are left alone. None for an engine it doesn't know, a
/// design it can't build, or values that aren't its.
pub fn fresh_node(
    id: &str,
    extras: &[(String, String)],
    values: &[f32],
    rate: u32,
) -> Option<Box<dyn Node>> {
    let engine = engine_node(id, rate)?;
    let mut node: Box<dyn Node> = if extras.iter().any(|(k, _)| is_design_key(k)) {
        let mut s = Sound::new(engine, &Design::default()).ok()?;
        s.set_design_lines(extras).ok()?;
        s.prepare_effects(rate, mpfree_node::MAX_BLOCK);
        Box::new(s)
    } else {
        engine
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

/// A new instrument of the engine `id` at `rate`, at its defaults (Init).
/// None for an id that isn't one of the engines. The Wavetable synth is
/// still made, only for a project's track to read its settings into
/// before it turns into Poly ([`as_poly`]).
pub fn engine_node(id: &str, rate: u32) -> Option<Box<dyn Node>> {
    Some(match id {
        "mpfree.poly" => Box::new(Poly::new(rate)),
        "mpfree.fm" => Box::new(Fm::new(rate)),
        "mpfree.drums" => Box::new(Drums::new(rate)),
        "mpfree.pluck" => Box::new(Pluck::new(rate)),
        "mpfree.wavetable" => Box::new(Wavetable::new(rate)),
        "mpfree.keys" => Box::new(Keys::new(rate)),
        "mpfree.drawbar" => Box::new(Drawbar::new(rate)),
        "mpfree.squelch" => Box::new(Squelch::new(rate)),
        "mpfree.analog" => Box::new(Analog::new(rate)),
        "mpfree.strings" => Box::new(Strings::new(rate)),
        "mpfree.bass" => Box::new(Bass::new(rate)),
        "mpfree.wind" => Box::new(Wind::new(rate)),
        "mpfree.choir" => Box::new(Choir::new(rate)),
        _ => return None,
    })
}

/// A preset as a track's own copy takes it: its name, design lines and
/// values ([`Synth::preset_values`]).
pub type PresetSound = (String, Vec<(String, String)>, Vec<f32>);

/// A sound as a track's own copy is built from it ([`fresh_node`]): its
/// design lines and every value in the node's order.
pub type SoundCopy = (Vec<(String, String)>, Vec<f32>);

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
    /// The events queued for the engine playing, not yet rendered: what
    /// the pads and the arp asked of it this period (tests).
    #[cfg(test)]
    pub(crate) fn queued(&self) -> Vec<NodeEvent> {
        let k = self.engine().index();
        self.events[k][..self.len[k]].iter().map(|t| t.ev).collect()
    }

    /// The events queued for engine `e`, not yet rendered (tests).
    #[cfg(test)]
    pub(crate) fn queued_on(&self, e: Engine) -> Vec<NodeEvent> {
        let k = e.index();
        self.events[k][..self.len[k]].iter().map(|t| t.ev).collect()
    }

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
        let mut keys = Keys::new(rate);
        let keys_preset = start_preset(Keys::presets(), KEYS_START_PRESET);
        if let Some(p) = Keys::presets().get(keys_preset) {
            keys.load_preset(p);
        }
        let mut drawbar = Drawbar::new(rate);
        let drawbar_preset = start_preset(Drawbar::presets(), DRAWBAR_START_PRESET);
        if let Some(p) = Drawbar::presets().get(drawbar_preset) {
            drawbar.load_preset(p);
        }
        let mut squelch = Squelch::new(rate);
        let squelch_preset = start_preset(Squelch::presets(), SQUELCH_START_PRESET);
        if let Some(p) = Squelch::presets().get(squelch_preset) {
            squelch.load_preset(p);
        }
        let mut analog = Analog::new(rate);
        let analog_preset = start_preset(Analog::presets(), ANALOG_START_PRESET);
        if let Some(p) = Analog::presets().get(analog_preset) {
            analog.load_preset(p);
        }
        let mut strings = Strings::new(rate);
        let strings_preset = start_preset(Strings::presets(), STRINGS_START_PRESET);
        if let Some(p) = Strings::presets().get(strings_preset) {
            strings.load_preset(p);
        }
        let mut bass = Bass::new(rate);
        let bass_preset = start_preset(Bass::presets(), BASS_START_PRESET);
        if let Some(p) = Bass::presets().get(bass_preset) {
            bass.load_preset(p);
        }
        let mut wind = Wind::new(rate);
        let wind_preset = start_preset(Wind::presets(), WIND_START_PRESET);
        if let Some(p) = Wind::presets().get(wind_preset) {
            wind.load_preset(p);
        }
        let mut choir = Choir::new(rate);
        let choir_preset = start_preset(Choir::presets(), CHOIR_START_PRESET);
        if let Some(p) = Choir::presets().get(choir_preset) {
            choir.load_preset(p);
        }
        let mut fm = Fm::new(rate);
        let fm_preset = start_preset(Fm::presets(), FM_START_PRESET);
        if let Some(p) = Fm::presets().get(fm_preset) {
            fm.load_preset(p);
        }
        let meters = [
            meters_of(&poly),
            meters_of(&fm),
            meters_of(&drums),
            meters_of(&pluck),
            meters_of(&keys),
            meters_of(&drawbar),
            meters_of(&squelch),
            meters_of(&analog),
            meters_of(&strings),
            meters_of(&bass),
            meters_of(&wind),
            meters_of(&choir),
        ];
        let settings = PadSettings::default();
        let mut s = Synth {
            poly: plain(poly, rate),
            fm: plain(fm, rate),
            drums: plain(drums, rate),
            pluck: plain(pluck, rate),
            keys: plain(keys, rate),
            drawbar: plain(drawbar, rate),
            squelch: plain(squelch, rate),
            analog: plain(analog, rate),
            strings: plain(strings, rate),
            bass: plain(bass, rate),
            wind: plain(wind, rate),
            choir: plain(choir, rate),
            rate,
            events: vec![[Timed::new(0, NodeEvent::AllOff); MAX_EVENTS]; ENGINES]
                .into_boxed_slice()
                .try_into()
                .unwrap_or_else(|_| unreachable!("ENGINES queues")),
            len: [0; ENGINES],
            rooms: [NodeRoom::new(ROOM); ENGINES],
            dropped_events: 0,
            ends_lost: 0,
            pedals: Pedals::new(),
            left: [0.0; SCRATCH],
            right: [0.0; SCRATCH],
            settings,
            octave: 0,
            pads: pads::layout(&settings.setup(0)),
            playing: Box::new([[Held::default(); PADS]; PLAYERS]),
            holding: [0; PLAYER_WORDS],
            holders: [0; 128],
            press: Box::new([[0; PADS]; PLAYERS]),
            presets: [
                preset,
                fm_preset,
                drums_preset,
                pluck_preset,
                keys_preset,
                drawbar_preset,
                squelch_preset,
                analog_preset,
                strings_preset,
                bass_preset,
                wind_preset,
                choir_preset,
            ],
            pads_changed: false,
            fade: 0,
            fading: false,
            notes: 0,
            peak_voices: 0,
            resets: 0,
            meters,
            designs: [0; ENGINES],
            audition: None,
            own: Default::default(),
            tuning_files: Vec::new(),
            locks: crate::locks::Under::new(rate),
            pad_voices: vec![None; VOICE_TRACKS * VOICE_PADS].into_boxed_slice(),
            pad_holders: [[0; 128]; ENGINES],
        };
        // A start preset that is a factory sound brings its effects and
        // macros, as picking it does.
        for e in Engine::ALL {
            let p = &static_presets(e)[s.presets[e.index()]];
            if has_design(p.text) && s.sound_mut(e).load_text(p.text).is_ok() {
                s.redesigned(e);
            }
        }
        s
    }

    fn node(&self) -> &dyn Node {
        self.node_of(self.settings.engine)
    }

    fn node_of(&self, engine: Engine) -> &dyn Node {
        self.sound(engine)
    }

    /// Engine `e` as a sound: the engine with what its preset added.
    fn sound(&self, e: Engine) -> &dyn Designed {
        match e {
            Engine::Poly => &self.poly,
            Engine::Fm => &self.fm,
            Engine::Drums => &self.drums,
            Engine::Pluck => &self.pluck,
            Engine::Keys => &self.keys,
            Engine::Drawbar => &self.drawbar,
            Engine::Squelch => &self.squelch,
            Engine::Analog => &self.analog,
            Engine::Strings => &self.strings,
            Engine::Bass => &self.bass,
            Engine::Wind => &self.wind,
            Engine::Choir => &self.choir,
        }
    }

    fn sound_mut(&mut self, e: Engine) -> &mut dyn Designed {
        match e {
            Engine::Poly => &mut self.poly,
            Engine::Fm => &mut self.fm,
            Engine::Drums => &mut self.drums,
            Engine::Pluck => &mut self.pluck,
            Engine::Keys => &mut self.keys,
            Engine::Drawbar => &mut self.drawbar,
            Engine::Squelch => &mut self.squelch,
            Engine::Analog => &mut self.analog,
            Engine::Strings => &mut self.strings,
            Engine::Bass => &mut self.bass,
            Engine::Wind => &mut self.wind,
            Engine::Choir => &mut self.choir,
        }
    }

    /// Engine `e`'s design changed: its settings' list did, so the meters
    /// are found again.
    pub(crate) fn redesigned(&mut self, e: Engine) {
        self.meters[e.index()] = meters_of(self.sound(e));
        let d = &mut self.designs[e.index()];
        *d = d.wrapping_add(1);
    }

    /// Engine `e`'s design count ([`Synth::sound_of`]'s design lines move
    /// it): a copy made at another count has other effects or macros.
    pub fn design_of(&self, e: Engine) -> u32 {
        self.designs[e.index()]
    }

    /// Parameter `id` of engine `e`'s sound as the Synth tab has it now,
    /// whichever engine is on the pads (what a synth-voice pad's copy
    /// follows, `pad_synths.rs`). Never allocates.
    pub fn value_of(&self, e: Engine, id: u16) -> f32 {
        self.node_of(e).param(id)
    }

    /// Engine `e` plays a plain preset (a factory one with no effects or
    /// macros of its own): the effects and macros an earlier preset brought
    /// go.
    fn make_plain(&mut self, e: Engine) {
        if !self.sound(e).is_plain() {
            // An empty design always builds.
            let _ = self.sound_mut(e).set_design(&Design::default());
            self.redesigned(e);
        }
    }

    /// Engine `engine`'s sound as it is now, for a copy a track plays as
    /// its own instrument ([`fresh_node`], `instruments.rs`): the node's
    /// id, its design lines (none for a plain engine) and every value in
    /// its parameter order. Allocates.
    pub fn sound_of(&self, engine: Engine) -> (&'static str, Vec<(String, String)>, Vec<f32>) {
        let node = self.node_of(engine);
        let values = node.params().iter().map(|p| node.param(p.id)).collect();
        (node.info().id, node.save_extras(), values)
    }

    /// The pads' notes for engine `engine` with the settings now: Drums'
    /// kit notes, or the layout. Never allocates.
    pub fn pads_for(&self, engine: Engine) -> [Pad; PADS] {
        match engine {
            Engine::Drums => {
                let base = self.drums.param(drum_params::BASE_NOTE).round() as i16;
                std::array::from_fn(|p| Pad::single(base + p as i16))
            }
            _ => pads::layout(&self.setup()),
        }
    }

    /// The tuning the engines play in now (the Synth tab's), for the
    /// tracks' own instruments.
    pub fn tuning_now(&self) -> Tuning {
        self.settings.tuning.tuning(&self.tuning_files)
    }

    fn node_mut(&mut self) -> &mut dyn Node {
        self.sound_mut(self.settings.engine)
    }

    fn engine_node(&self, e: Engine) -> &dyn Node {
        self.sound(e)
    }

    fn engine_node_mut(&mut self, e: Engine) -> &mut dyn Node {
        self.sound_mut(e)
    }

    /// Every engine's sound now, as preset text, for a project's `synth`
    /// part (`project.rs`). Small: a few hundred values.
    pub fn sounds(&self) -> Vec<(Engine, String)> {
        Engine::ALL
            .iter()
            .map(|&e| {
                let name = static_presets(e)
                    .get(self.presets[e.index()])
                    .map_or("", |p| p.name);
                (e, self.sound(e).to_text(name, &[], &[]))
            })
            .collect()
    }

    /// Puts back the sounds [`Synth::sounds`] saved (notes keep sounding,
    /// values glide, as with a preset). One that doesn't read leaves that
    /// engine as it is; returns a line for each.
    pub fn set_sounds(&mut self, sounds: &[(Engine, String)]) -> Vec<String> {
        let mut notes = Vec::new();
        for (e, text) in sounds {
            let names = self.drum_names();
            let p = match poly_text(text).and_then(|t| self.sound_mut(*e).load_text(&t)) {
                Ok(p) => p,
                Err(err) => {
                    notes.push(format!(
                        "synth: the project's {} sound didn't read ({err})",
                        e.name()
                    ));
                    continue;
                }
            };
            self.redesigned(*e);
            if *e == Engine::Drums {
                self.drums_moved(names);
            }
            // The preset list shows the saved name when it is a factory one.
            let at = static_presets(*e).iter().position(|x| x.name == p.name);
            if let Some(i) = at {
                self.presets[e.index()] = i;
            }
        }
        notes
    }

    /// Voices one engine has sounding or fading.
    fn voices_of(&self, engine: Engine) -> usize {
        match engine {
            Engine::Poly => self.poly.active_voices(),
            Engine::Fm => self.fm.active_voices(),
            Engine::Drums => self.drums.active_voices(),
            Engine::Pluck => self.pluck.active_voices(),
            Engine::Keys => self.keys.active_voices(),
            Engine::Drawbar => self.drawbar.active_voices(),
            Engine::Squelch => self.squelch.active_voices(),
            Engine::Analog => self.analog.active_voices(),
            Engine::Strings => self.strings.keys_sounding(),
            Engine::Bass => self.bass.active_voices(),
            Engine::Wind => self.wind.active_voices(),
            Engine::Choir => self.choir.active_voices(),
        }
    }

    pub fn engine(&self) -> Engine {
        self.settings.engine
    }

    /// The engine playing now as a node's info and controls (step
    /// sequencer refactor F1: what a track that plays the synth plays, so
    /// a clip's locks play only on the kind they were made for).
    pub fn node_ref(&self) -> (&'static mpfree_node::NodeInfo, &'static [ParamInfo]) {
        let n = self.node();
        (n.info(), n.params())
    }

    // ------------------------------------------------------------ the pads

    pub fn settings(&self) -> PadSettings {
        self.settings
    }

    /// The pads' notes for the bank playing.
    pub fn setup(&self) -> PadSetup {
        self.settings.setup(self.octave)
    }

    /// The pads play notes (not the Drums engine's sounds), so a track
    /// playing them transposes (sequencer S4).
    pub fn plays_notes(&self) -> bool {
        self.settings.engine != Engine::Drums
    }

    /// The key a live transpose moves through In key: the pads' scale,
    /// while they play In key or chords of it. None: semitones.
    pub fn transpose_key(&self) -> Option<Scale> {
        let s = self.setup();
        s.scale.filter(|_| s.layout != PadLayout::Chromatic)
    }

    /// New pad settings (from the Synth tab's options). Notes already held
    /// end on their own pad's note-off. A new engine takes the pads; the
    /// old one's notes get their note-off now and finish their release.
    /// Returns whether the engine changed. Never allocates.
    pub fn set_settings(&mut self, settings: PadSettings) -> bool {
        let switched = settings.engine != self.settings.engine;
        if switched {
            self.all_off();
            // The old engine's locks end now, its own values back.
            let old = self.settings.engine;
            let mut back = [(0u16, 0f32); crate::locks::MAX];
            let mut n = 0;
            self.locks.forget(
                |k| k >> 16 == old.index() as u32,
                |k, v| {
                    back[n] = ((k & 0xFFFF) as u16, v);
                    n += 1;
                },
            );
            for &(id, v) in &back[..n] {
                self.node_mut().set_param(id, v);
            }
        }
        let retune = settings.tuning != self.settings.tuning;
        self.settings = settings;
        self.settings.bank %= BANKS;
        if self.settings.tuning.file(&self.tuning_files).is_none() {
            // Not a file this synth has: the built-in tuning plays, and
            // the settings (and so the log line) say so.
            self.settings.tuning.file = None;
        }
        if retune {
            self.retune();
        }
        self.relayout();
        switched
    }

    /// The tuning files to offer after the built-in tunings (at most
    /// [`MAX_TUNING_FILES`]; the rest are left out). A pick of a file
    /// moves to the new list's file in its place, or to its built-in
    /// tuning when there is none. Allocates and frees: call it before the
    /// sound starts.
    pub fn set_tuning_files(&mut self, mut files: Vec<TuningFile>) {
        files.truncate(MAX_TUNING_FILES);
        self.tuning_files = files;
        if self.settings.tuning.file(&self.tuning_files).is_none() {
            self.settings.tuning.file = None;
        }
        self.retune();
    }

    /// The tuning files offered after the built-in tunings.
    pub fn tuning_files(&self) -> &[TuningFile] {
        &self.tuning_files
    }

    /// The tuning playing, for the log: "12-TET", "Just (5-limit) on D",
    /// a file's name.
    pub fn tuning_label(&self) -> String {
        self.settings.tuning.label(&self.tuning_files)
    }

    /// Hands every engine the tuning picked; sounding notes move to it on
    /// their next control step. Drums ignore tunings. Never allocates.
    fn retune(&mut self) {
        let t = self.settings.tuning.tuning(&self.tuning_files);
        self.poly.set_tuning(&t);
        self.fm.set_tuning(&t);
        self.drums.set_tuning(&t);
        self.pluck.set_tuning(&t);
        self.keys.set_tuning(&t);
        self.drawbar.set_tuning(&t);
        self.squelch.set_tuning(&t);
        self.analog.set_tuning(&t);
        self.strings.set_tuning(&t);
        self.bass.set_tuning(&t);
        self.wind.set_tuning(&t);
        self.choir.set_tuning(&t);
    }

    /// Works the pads' notes out again: Drums' kit notes, or the layout.
    fn relayout(&mut self) {
        self.pads = self.pads_for(self.settings.engine);
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
            pitches: self.pad_pitches(),
            user: false,
            key: self.transpose_key().map(|s| scale_name(&s)),
            tags: Vec::new(),
        }
    }

    /// Each pad's note (a chord pad's root) and whether its light is the
    /// root's, for Beats' scale rows; none for the Drums engine, whose
    /// pads are sounds.
    pub fn pad_pitches(&self) -> Vec<Option<crate::play_page::PadPitch>> {
        if self.settings.engine == Engine::Drums {
            return Vec::new();
        }
        self.pads
            .iter()
            .map(|p| {
                let &note = p.notes().last()?;
                let root = match p.role {
                    PadRole::Root => true,
                    PadRole::Plain => note.is_multiple_of(12),
                    _ => false,
                };
                Some(crate::play_page::PadPitch { note, root })
            })
            .collect()
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

    /// Queues `ev` for `engine`. The engine on the pads gets it through the
    /// pedals, which hold its note-offs while a pedal keeps the notes,
    /// unless it hears the pedals itself.
    fn push(&mut self, engine: Engine, offset: u32, ev: NodeEvent) {
        if engine == self.settings.engine && !self.engine_node(engine).hears(Ctl::Sustain) {
            if self.pedals.pass(&ev) {
                self.push_raw(engine, offset, ev);
                return;
            }
            let mut pedals = self.pedals;
            pedals.filter(ev, |e| self.push_raw(engine, offset, e));
            self.pedals = pedals;
        } else {
            self.push_raw(engine, offset, ev);
        }
    }

    /// Queues `ev` for `engine` if its room takes it (refactor A2b): a new
    /// note or value in a period too busy is left out and counted; an
    /// ending always goes in.
    fn push_raw(&mut self, engine: Engine, offset: u32, ev: NodeEvent) {
        let k = engine.index();
        if self.rooms[k].admit(&ev) {
            self.store(k, offset, ev);
        }
    }

    /// Puts an event its room let in in engine `k`'s queue. The queue holds
    /// all the room lets in, so it always fits; one that doesn't is an
    /// ending lost, counted as the bug it is.
    fn store(&mut self, k: usize, offset: u32, ev: NodeEvent) {
        if self.len[k] < MAX_EVENTS {
            self.events[k][self.len[k]] = Timed::new(offset, ev);
            self.len[k] += 1;
        } else {
            debug_assert!(false, "the room let in more than the queue holds");
            self.ends_lost += 1;
        }
    }

    /// Ends the notes player `i` started on pad `pad`: each note's
    /// note-off goes out once no other pad (of any player) holds it.
    fn release_pad(&mut self, i: usize, pad: usize, offset: u32) {
        let held = std::mem::take(&mut self.playing[i][pad]);
        self.press[i][pad] = 0;
        if held.len == 0 {
            return;
        }
        let e = self.settings.engine;
        for &note in held.notes() {
            let h = &mut self.holders[usize::from(note)];
            *h = h.saturating_sub(1);
            if *h == 0 && self.pad_holders[e.index()][usize::from(note)] == 0 {
                self.push(e, offset, NodeEvent::NoteOff { note });
            }
        }
        if self.playing[i].iter().all(|h| h.len == 0) {
            self.holding[i / 128] &= !(1u128 << (i % 128));
        }
    }

    /// Every player's pads let go of their notes, with no note-offs (a
    /// panic or a reset sends its own).
    fn forget_pads(&mut self) {
        for i in players(self.holding) {
            self.playing[i] = [Held::default(); PADS];
            self.press[i] = [0; PADS];
        }
        self.holding = [0; PLAYER_WORDS];
    }

    /// Pad `pad` starts its notes on the pads' own row ([`Player::Pads`]),
    /// `offset` frames into the next render. A pad already sounding there
    /// ends its notes first (Note Repeat's next hit).
    pub fn note_on(&mut self, pad: usize, velocity: u8, offset: u32) {
        self.note_on_shifted(Player::Pads, pad, velocity, offset, &Shift::NONE);
    }

    /// Pad `pad` starts its notes for `player` (a track's clip or the pads
    /// through its arp), with its track's live transpose (sequencer S4):
    /// each of the pad's notes moves by `shift`, and one that falls off the
    /// keyboard is left out. The player's pad remembers what it started, so
    /// its note-off and pressure reach those notes whatever the shift is by
    /// then, and another player's hit on the same pad leaves them be.
    pub fn note_on_shifted(
        &mut self,
        player: Player,
        pad: usize,
        velocity: u8,
        offset: u32,
        shift: &Shift,
    ) {
        let Some(i) = player.index() else {
            return;
        };
        if velocity == 0 {
            return;
        }
        if pad >= PADS {
            // A clip's hit past the synth's 16 pads plays nothing: counted.
            self.dropped_events += 1;
            return;
        }
        self.release_pad(i, pad, offset);
        let p = self.pads[pad];
        let mut held = Held::default();
        for note in p.notes().iter().filter_map(|&n| shift.apply(n)) {
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
            self.holding[i / 128] |= 1u128 << (i % 128);
        }
        self.playing[i][pad] = held;
    }

    /// Pad `pad` was let go on the pads' own row (or its repeat ended),
    /// `offset` frames in.
    pub fn note_off(&mut self, pad: usize, offset: u32) {
        self.note_off_by(Player::Pads, pad, offset);
    }

    /// `player`'s pad `pad` was let go, `offset` frames in: the notes it
    /// started end; another player's on the same pad play on.
    pub fn note_off_by(&mut self, player: Player, pad: usize, offset: u32) {
        if let Some(i) = player.index().filter(|_| pad < PADS) {
            self.release_pad(i, pad, offset);
        }
    }

    /// The pad's own pressure, 0 to 127, on every note it holds, whoever
    /// started them (as before step 4). A note two pads hold takes the
    /// harder press.
    pub fn pressure(&mut self, pad: usize, value: u8) {
        if pad >= PADS {
            return;
        }
        for i in players(self.holding) {
            if self.playing[i][pad].len > 0 {
                self.press[i][pad] = value.min(127);
                self.send_pressure(i, pad, 0);
            }
        }
    }

    /// `player`'s pressure on pad `pad`, `offset` frames into the next
    /// render (a clip's recorded pressure, on its frame).
    pub fn pressure_at(&mut self, player: Player, pad: usize, value: u8, offset: u32) {
        let Some(i) = player.index() else {
            return;
        };
        if pad >= PADS || self.playing[i][pad].len == 0 {
            return;
        }
        self.press[i][pad] = value.min(127);
        self.send_pressure(i, pad, offset);
    }

    /// Each note player `i`'s pad `pad` holds takes the hardest press of
    /// the pads that hold it.
    fn send_pressure(&mut self, i: usize, pad: usize, offset: u32) {
        let held = self.playing[i][pad];
        for &note in held.notes() {
            let mut v = 0u8;
            for q in players(self.holding) {
                for (p, other) in self.playing[q].iter().enumerate() {
                    if other.notes().contains(&note) {
                        v = v.max(self.press[q][p]);
                    }
                }
            }
            self.push(
                self.settings.engine,
                offset,
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
        let e = self.settings.engine;
        if *h == 0 && self.pad_holders[e.index()][usize::from(note)] == 0 {
            self.push(e, offset, NodeEvent::NoteOff { note });
        }
    }

    /// A hit on a kit pad of the Instrument type (the Pads tab's: pad
    /// `pad` of track `track`'s kit): one voice of `engine` at `note`,
    /// whichever engine is on the pads, `offset` frames into the next
    /// render. A pad still holding a note ends it first. Never allocates.
    pub fn pad_voice_on(
        &mut self,
        track: u16,
        pad: usize,
        (engine, note): (Engine, u8),
        velocity: u8,
        offset: u32,
    ) {
        let Some(k) = voice_key(track, pad) else {
            return;
        };
        if let Some(was) = self.pad_voices[k].take() {
            self.end_pad_voice(was, offset);
        }
        if note > 127 || velocity == 0 {
            return;
        }
        self.push(
            engine,
            offset,
            NodeEvent::NoteOn {
                note,
                velocity: velocity.min(127),
            },
        );
        let h = &mut self.pad_holders[engine.index()][usize::from(note)];
        *h = h.saturating_add(1);
        self.pad_voices[k] = Some((engine, note));
        self.notes += 1;
    }

    /// That pad let go: its note ends once nothing else holds it.
    pub fn pad_voice_off(&mut self, track: u16, pad: usize, offset: u32) {
        if let Some(was) = voice_key(track, pad).and_then(|k| self.pad_voices[k].take()) {
            self.end_pad_voice(was, offset);
        }
    }

    /// Pressure, 0 to 127, on the note that pad holds, if any.
    pub fn pad_voice_pressure(&mut self, track: u16, pad: usize, value: u8, offset: u32) {
        if let Some((engine, note)) = voice_key(track, pad).and_then(|k| self.pad_voices[k]) {
            self.push(
                engine,
                offset,
                NodeEvent::Pressure {
                    note,
                    value: value.min(127),
                },
            );
        }
    }

    /// Ends every note track `track`'s Instrument pads hold (its mute).
    pub fn end_pad_voices(&mut self, track: u16, offset: u32) {
        let Some(first) = voice_key(track, 0) else {
            return;
        };
        for k in first..first + VOICE_PADS {
            if let Some(was) = self.pad_voices[k].take() {
                self.end_pad_voice(was, offset);
            }
        }
    }

    /// Whether an Instrument pad holds a note now.
    #[cfg(test)]
    pub fn pad_voices_held(&self) -> usize {
        self.pad_voices.iter().flatten().count()
    }

    fn end_pad_voice(&mut self, (engine, note): (Engine, u8), offset: u32) {
        let n = usize::from(note);
        let h = &mut self.pad_holders[engine.index()][n];
        if *h == 0 {
            return;
        }
        *h -= 1;
        // The pads' own notes on the same engine hold it too.
        if *h == 0 && !(engine == self.settings.engine && self.holders[n] > 0) {
            self.push(engine, offset, NodeEvent::NoteOff { note });
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

    /// A controller for the engine on the pads (a pedal, the mod wheel,
    /// breath), 0 to 1. The pedals hold its notes ([`Synth::push`]).
    pub fn control(&mut self, ctl: Ctl, value: f32, offset: u32) {
        if !value.is_nan() {
            self.push(
                self.settings.engine,
                offset,
                NodeEvent::Control {
                    ctl,
                    value: value.clamp(0.0, 1.0),
                },
            );
        }
    }

    /// A keyboard's pitch wheel, -1 to 1 of the engine's bend range, on
    /// every note.
    pub fn bend(&mut self, value: f32) {
        if value.is_finite() {
            self.push(
                self.settings.engine,
                0,
                NodeEvent::Bend {
                    value: value.clamp(-1.0, 1.0),
                },
            );
        }
    }

    /// One note's own expression (an MPE controller's slide or tone), while
    /// something holds the note: a note the MIDI effects moved elsewhere
    /// keeps nothing for a later note.
    pub fn expression_note(&mut self, note: u8, kind: mpfree_node::Expr, value: f32) {
        if note <= 127 && self.holders[usize::from(note)] > 0 {
            self.push(
                self.settings.engine,
                0,
                NodeEvent::Expression { note, kind, value },
            );
        }
    }

    /// Every note ends with its release (the pads went back to drums).
    pub fn all_off(&mut self) {
        self.push(self.settings.engine, 0, NodeEvent::AllOff);
        self.forget_pads();
        self.holders = [0; 128];
        // The Instrument pads' notes on the other engines end too.
        for e in Engine::ALL {
            if e != self.settings.engine && self.pad_holders[e.index()].iter().any(|&h| h > 0) {
                self.push(e, 0, NodeEvent::AllOff);
            }
        }
        self.pad_voices.fill(None);
        self.pad_holders = [[0; 128]; ENGINES];
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
        // No frames: the events wait for the next render, endings and all
        // (refactor A2b).
        if n == 0 {
            return;
        }
        // The locks that ended play now: forget them. A knob a new owner
        // glides to, or the hand let go of, moves from the start (knob
        // owners, locks.rs).
        self.locks.settle();
        let mut moved = [(0u32, 0f32); crate::locks::MAX];
        let mut k = 0;
        self.locks.tick(n as u32, |key, v| {
            if k < moved.len() {
                moved[k] = (key, v);
                k += 1;
            }
        });
        for &(key, value) in &moved[..k] {
            if let Some(&e) = Engine::ALL.get((key >> 16) as usize) {
                let id = (key & 0xFFFF) as u16;
                self.push(e, 0, NodeEvent::Param { id, value });
            }
        }
        let len = self.len;
        self.len = [0; ENGINES];
        // A new period's queues: what is owed stays owed.
        for room in &mut self.rooms {
            room.period();
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
                    Engine::Keys => &mut self.keys,
                    Engine::Drawbar => &mut self.drawbar,
                    Engine::Squelch => &mut self.squelch,
                    Engine::Analog => &mut self.analog,
                    Engine::Strings => &mut self.strings,
                    Engine::Bass => &mut self.bass,
                    Engine::Wind => &mut self.wind,
                    Engine::Choir => &mut self.choir,
                };
                node.process(&ctx, &self.events[k][first..e[k]], sl, sr);
                if !sl.iter().chain(sr.iter()).all(|s| s.is_finite()) {
                    node.reset();
                    // The reset ended its notes: nothing owed.
                    self.rooms[k].clear();
                    if k == on {
                        self.forget_pads();
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
                    for e in Engine::ALL {
                        self.engine_node_mut(e).reset();
                    }
                    for room in &mut self.rooms {
                        room.clear();
                    }
                    self.forget_pads();
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

    /// Load shedding (`shed.rs`): the voice to give up first across the
    /// engines (the least audible, [`Cull::audibility`]), with its engine.
    /// No allocation.
    pub fn cull_candidate(&self) -> Option<(Engine, Cull)> {
        Engine::ALL.into_iter().fold(None, |best, e| {
            Cull::first(best, self.node_of(e).cull_candidate().map(|c| (e, c)))
        })
    }

    /// Gives up the voice engine `e` names, with its steal's short fade.
    /// True if it did. No allocation.
    pub fn cull(&mut self, e: Engine) -> bool {
        match e {
            Engine::Poly => self.poly.cull(),
            Engine::Fm => self.fm.cull(),
            Engine::Drums => self.drums.cull(),
            Engine::Pluck => self.pluck.cull(),
            Engine::Keys => self.keys.cull(),
            // The organ's wheels turn whatever the keys do, so a key given
            // up saves little; the string machine's keys share its notes;
            // Squelch and Bass are one voice. None of them offers one;
            // Wind offers its voices but never its one voice while held.
            Engine::Drawbar => self.drawbar.cull(),
            Engine::Squelch => self.squelch.cull(),
            Engine::Analog => self.analog.cull(),
            Engine::Strings => self.strings.cull(),
            Engine::Bass => self.bass.cull(),
            Engine::Wind => self.wind.cull(),
            Engine::Choir => self.choir.cull(),
        }
    }

    pub fn counters(&self) -> Counters {
        Counters {
            notes: self.notes,
            peak_voices: self.peak_voices,
            dropped: self.dropped_events + self.poly.dropped + self.fm.dropped,
            busy: self.rooms.iter().map(|r| r.refused).sum(),
            ends_lost: self.ends_lost + self.rooms.iter().map(|r| r.lost).sum::<u64>(),
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
            Engine::Keys => self.keys.param(mpfree_synth::keys_params::POLYPHONY),
            // Every key plays, as on the organ.
            Engine::Drawbar => mpfree_synth::drawbar::MAX_KEYS as f32,
            // One note at a time.
            Engine::Squelch => 1.0,
            Engine::Analog => self.analog.param(mpfree_synth::analog_params::POLYPHONY),
            // Every key plays, as on the string machines.
            Engine::Strings => mpfree_synth::strings::NOTES as f32,
            Engine::Bass => 1.0,
            Engine::Wind => self.wind.param(mpfree_synth::wind_params::VOICES),
            Engine::Choir => self.choir.param(mpfree_synth::choir_params::VOICES),
        }
    }

    /// Each sounding voice's pressure, 0 to 1, for the Pressure page's live
    /// bars. Returns how many it wrote (none for Drums and Pluck, which
    /// don't take pressure yet).
    pub fn pressures(&self, out: &mut [f32]) -> usize {
        match self.settings.engine {
            Engine::Poly => self.poly.pressures(out),
            Engine::Fm => self.fm.pressures(out),
            Engine::Wind => self.wind.pressures(out),
            Engine::Choir => self.choir.pressures(out),
            Engine::Drums
            | Engine::Pluck
            | Engine::Keys
            | Engine::Drawbar
            | Engine::Squelch
            | Engine::Analog
            | Engine::Strings
            | Engine::Bass => 0,
        }
    }

    // ------------------------------------------------------------ the Synth tab

    /// Sets a parameter of the engine on the pads (clamped by the node).
    /// Drums' Pad 1 note and pad sounds move or rename the pads (see
    /// [`Synth::take_pads_changed`]).
    pub fn set(&mut self, id: u16, value: f32) {
        let names = self.drum_names();
        // Under a clip's lock, the turn is what its end comes back to, and
        // the hand holds the knob a moment (knob owners, locks.rs).
        self.locks.hand(lock_key(self.settings.engine, id), value);
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

    /// Every parameter of the engine on the pads.
    #[cfg(test)]
    pub fn param_info_list(&self) -> Vec<ParamInfo> {
        self.node().params().to_vec()
    }

    /// The engine on the pads' name ("Poly"), for MIDI Learn's names.
    pub fn engine_name(&self) -> &'static str {
        self.node().info().name
    }

    /// How many presets the engine on the pads has.
    pub fn preset_count(&self) -> usize {
        static_presets(self.settings.engine).len()
    }

    /// Loads preset `i` of the engine on the pads; returns its name, or None
    /// if there is no such preset. Notes keep sounding; values glide to the
    /// preset's. A Drums kit can move or rename the pads.
    pub fn load_preset(&mut self, i: usize) -> Option<&str> {
        let e = self.settings.engine;
        let p = static_presets(e).get(i)?;
        if has_design(p.text) {
            // A factory sound with its own effects and macros loads as a
            // user's preset does. Every factory preset loads (the tests
            // load each one), so this never fails.
            self.put_text(p.text).ok()?;
            self.presets[e.index()] = i;
            self.own[e.index()] = None;
            self.audition = None;
            return Some(p.name);
        }
        self.make_plain(e);
        match e {
            Engine::Drums => {
                let names = self.drum_names();
                self.drums.load_preset(p);
                self.drums_moved(names);
            }
            Engine::Pluck => self.pluck.load_preset(p),
            Engine::Keys => self.keys.load_preset(p),
            Engine::Drawbar => self.drawbar.load_preset(p),
            Engine::Squelch => self.squelch.load_preset(p),
            Engine::Analog => self.analog.load_preset(p),
            Engine::Strings => self.strings.load_preset(p),
            Engine::Bass => self.bass.load_preset(p),
            Engine::Wind => self.wind.load_preset(p),
            Engine::Choir => self.choir.load_preset(p),
            Engine::Fm => self.fm.load_preset(p),
            Engine::Poly => self.poly.load_preset(p),
        }
        self.presets[e.index()] = i;
        self.own[e.index()] = None;
        self.audition = None;
        self.preset_under_locks();
        Some(p.name)
    }

    /// A preset went in: its values are what the locks holding now come
    /// back to.
    fn preset_under_locks(&mut self) {
        let e = self.settings.engine;
        let mut locks = std::mem::replace(&mut self.locks, crate::locks::Under::new(0));
        let node = self.node_of(e);
        locks.reread(
            |k| k >> 16 == e.index() as u32,
            |k| Some(node.param((k & 0xFFFF) as u16)),
        );
        self.locks = locks;
    }

    /// Track `track`'s clip's lock on parameter `id` of the engine on
    /// the pads (`Owner::Inst`): `Some` sets it on its frame, `offset`
    /// frames into the next render, `None` puts back the value under it
    /// (the sound's own, or the last turn by hand). The tracks that play
    /// the synth share it, so one owns a knob at a time (step 4b,
    /// locks.rs): another track's lock is kept and plays when it owns the
    /// knob. False for a parameter the engine doesn't have, a meter,
    /// Drums' pad notes and models (they move the pads), or a full table.
    pub fn lock(&mut self, track: usize, id: u16, value: Option<f32>, offset: u32) -> bool {
        let e = self.settings.engine;
        let key = lock_key(e, id);
        let Some(value) = value else {
            // The value put back is an ending: it always has a place, as
            // each new lock asked first (below).
            if let Some(back) = self.locks.unlock(key, track) {
                let k = e.index();
                if self.rooms[k].unlock() {
                    self.store(k, offset, NodeEvent::Param { id, value: back });
                }
            }
            return true;
        };
        let smooth = match self
            .node()
            .params()
            .iter()
            .find(|p| p.id == id && !p.readonly)
        {
            Some(p) if !(e == Engine::Drums && moves_drum_pads(id)) => crate::locks::glides(p),
            _ => return false,
        };
        let now = self.node().param(id);
        // A new lock asks its engine's room first, so its value put back
        // will have a place; in a period too busy it is left out (counted)
        // and its end finds nothing to put back.
        if !self.locks.held_by(key, track) && !self.rooms[e.index()].hold() {
            return true;
        }
        match self.locks.lock(key, track, value, now, smooth) {
            Some(true) => self.push(e, offset, NodeEvent::Param { id, value }),
            Some(false) => {}
            None => return false,
        }
        true
    }

    /// Each track's lasting id (`mpfree_seq::TrackId`, 0 for none), by
    /// track: when two tracks on the synth lock one knob, the lowest owns
    /// it (step 4b, locks.rs).
    pub fn set_ranks(&mut self, ids: &[u32; crate::locks::TRACKS]) {
        self.locks.set_ranks(ids);
    }

    /// How many of the engines' parameters a clip's lock holds now.
    #[cfg(test)]
    pub fn locks_holding(&self) -> usize {
        self.locks.holding()
    }
}

/// The sound from before the preset picker's first try: the engine's sound
/// as preset text (with its design), and which preset it was.
#[derive(Clone, Debug)]
struct Tried {
    text: String,
    preset: usize,
    own: Option<String>,
}

/// User presets: files in the library's Instruments place, and the
/// browser's preset picker (opened from the Synth tab's title).
impl Synth {
    /// Whether a `.preset` file from `folder` under the library's
    /// Instruments place (where the folder is the engine's name) may play
    /// on the engine on the pads: the folder must be this engine's, if the
    /// file sits in one; the file's own engine line is checked as it loads.
    fn check_folder(&self, folder: &str, text: &str) -> Result<(), SoundError> {
        match Engine::from_folder(folder) {
            Some(e) if e != self.settings.engine => {
                Err(SoundError::Preset(PresetError::WrongEngine {
                    file: engine_of(text).unwrap_or(folder).to_string(),
                    node: self.node().info().id.to_string(),
                }))
            }
            _ => Ok(()),
        }
    }

    /// Puts preset text on the engine on the pads (notes keep sounding,
    /// values glide), as a factory preset does, with the effects and macros
    /// it brings. Drums' pads follow their kit. On an error nothing
    /// changes. Returns the preset's name.
    fn put_text(&mut self, text: &str) -> Result<String, SoundError> {
        let e = self.settings.engine;
        let names = self.drum_names();
        let text = poly_text(text)?;
        let p = self.sound_mut(e).load_text(&text)?;
        self.redesigned(e);
        if e == Engine::Drums {
            self.drums_moved(names);
        }
        self.preset_under_locks();
        Ok(p.name)
    }

    /// The preset picker turned to the musician's preset `name` (its
    /// file's name, `BrowseAction::TryPreset`): plays it on the engine on
    /// the pads, named so on the Synth tab. The sound from before the
    /// first try is kept, for [`Synth::revert_preset`]. Returns the name
    /// inside the preset. A file for another engine, or one this build
    /// can't read, changes nothing.
    pub fn try_preset(
        &mut self,
        folder: &str,
        name: &str,
        text: &str,
    ) -> Result<String, SoundError> {
        self.check_folder(folder, text)?;
        let before = self.audition.is_none().then(|| self.tried());
        let inside = self.put_text(text)?;
        if before.is_some() {
            self.audition = before;
        }
        self.own[self.settings.engine.index()] = Some(name.to_string());
        Ok(inside)
    }

    /// The preset picker turned to the engine's own preset `i`
    /// (`BrowseAction::TryFactoryPreset`): plays it, keeping the sound from
    /// before the first try as [`Synth::try_preset`] does. Returns its
    /// name; None (nothing changes) past the end of the list.
    pub fn try_factory(&mut self, i: usize) -> Option<String> {
        let kept = self.audition.take().unwrap_or_else(|| self.tried());
        let name = self.load_preset(i).map(str::to_string);
        self.audition = Some(kept);
        name
    }

    /// What the engine on the pads plays now, to come back to.
    fn tried(&self) -> Tried {
        let e = self.settings.engine;
        Tried {
            text: self.sound(e).to_text("", &[], &[]),
            preset: self.presets[e.index()],
            own: self.own[e.index()].clone(),
        }
    }

    /// The preset picker was cancelled (`BrowseAction::RevertPreset`): the
    /// sound from before its first try comes back, every setting as it
    /// was, and the name it had. Nothing happens if nothing was tried.
    pub fn revert_preset(&mut self) {
        if let Some(t) = self.audition.take() {
            // The capture is this engine's, so it always loads.
            let _ = self.put_text(&t.text);
            let e = self.settings.engine.index();
            self.presets[e] = t.preset;
            self.own[e] = t.own;
        }
    }

    /// Loads a user preset (`BrowseAction::LoadPreset`, or the picker's
    /// Load after trying): the engine on the pads plays it from now on,
    /// named `name` (its file's name) on the Synth tab, and the sound from
    /// before any try is forgotten. Returns the name inside the preset.
    pub fn load_preset_file(
        &mut self,
        folder: &str,
        name: &str,
        text: &str,
    ) -> Result<String, SoundError> {
        self.check_folder(folder, text)?;
        let inside = self.put_text(text)?;
        self.audition = None;
        self.own[self.settings.engine.index()] = Some(name.to_string());
        Ok(inside)
    }

    /// The sound on the pads was just saved as the musician's preset
    /// `name`: it is that preset now, so the Synth tab names it, and a try
    /// before the save is kept (Cancel no longer takes it back).
    pub fn saved_as(&mut self, name: &str) {
        self.own[self.settings.engine.index()] = Some(name.to_string());
        self.audition = None;
    }

    /// The musician's preset the engine on the pads plays, if it came from
    /// Yours.
    #[cfg(test)]
    pub fn own_name(&self) -> Option<&str> {
        self.own[self.settings.engine.index()].as_deref()
    }

    /// Preset `text` from `folder` as a track's own copy of `engine` takes
    /// it ([`fresh_node`]): its name, design lines and values, read into a
    /// scratch sound so this synth is left alone. Another engine's preset
    /// is refused, as on the pads. Allocates.
    pub fn text_sound(
        &self,
        engine: Engine,
        folder: &str,
        text: &str,
    ) -> Result<PresetSound, SoundError> {
        let wrong = || {
            SoundError::Preset(PresetError::WrongEngine {
                file: engine_of(text).unwrap_or(folder).to_string(),
                node: engine.node_id().to_string(),
            })
        };
        if Engine::from_folder(folder).is_some_and(|f| f != engine) {
            return Err(wrong());
        }
        let node = engine_node(engine.node_id(), self.rate).ok_or_else(wrong)?;
        let mut s = Sound::new(node, &Design::default())?;
        let p = s.load_text(&poly_text(text)?)?;
        let values = s.params().iter().map(|q| s.param(q.id)).collect();
        Ok((p.name, s.save_extras(), values))
    }

    /// A track's own copy of `engine`, its design lines and values (as
    /// [`fresh_node`] takes them), as preset text named `name` for the
    /// engine's folder. None if they don't make that engine's sound.
    pub fn copy_text(
        &self,
        engine: Engine,
        extras: &[(String, String)],
        values: &[f32],
        name: &str,
    ) -> Option<(&'static str, String)> {
        let node = engine_node(engine.node_id(), self.rate)?;
        let mut s = Sound::new(node, &Design::default()).ok()?;
        if extras.iter().any(|(k, _)| is_design_key(k)) {
            s.set_design_lines(extras).ok()?;
        }
        let params = s.params();
        if params.len() != values.len() {
            return None;
        }
        for (p, v) in params.iter().zip(values) {
            if !p.readonly {
                s.set_param(p.id, *v);
            }
        }
        Some((engine.name(), s.to_text(name, &[], &[])))
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
        let e = self.settings.engine;
        (e.name(), self.sound(e).to_text(name, tags, extra))
    }
}

impl Synth {
    #[cfg(test)]
    pub fn preset(&self) -> usize {
        self.presets[self.settings.engine.index()]
    }

    pub fn preset_name(&self) -> &str {
        let i = self.presets[self.settings.engine.index()];
        static_presets(self.settings.engine)
            .get(i)
            .map_or("", |p| p.name)
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
        let presets = static_presets(self.settings.engine)
            .iter()
            .map(|p| p.name.to_string())
            .collect();
        let preset = Some(self.presets[self.settings.engine.index()]);
        let mut st = SynthState::from_node(
            self.node(),
            "Pads",
            Color::from_pad(PAD_WHITE),
            presets,
            preset,
        );
        st.own = self.own[self.settings.engine.index()].clone();
        st
    }

    pub fn param_info(&self, id: u16) -> Option<&'static ParamInfo> {
        self.node().params().iter().find(|p| p.id == id)
    }

    /// Engine `engine`'s factory presets' names.
    pub fn preset_names(&self, engine: Engine) -> Vec<String> {
        static_presets(engine)
            .iter()
            .map(|p| p.name.to_string())
            .collect()
    }

    /// Preset `i` of engine `engine` as a track's own copy of the engine
    /// takes it ([`fresh_node`]): its name, its design lines (none for a
    /// plain preset) and every value in the node's order. Loaded into a
    /// scratch engine, so this synth's sound is left alone. Allocates.
    pub fn preset_values(&self, engine: Engine, i: usize) -> Option<PresetSound> {
        fn values(n: &dyn Node) -> Vec<f32> {
            n.params().iter().map(|p| n.param(p.id)).collect()
        }
        let rate = self.rate;
        let p = static_presets(engine).get(i)?;
        if has_design(p.text) {
            let mut s =
                Sound::new(engine_node(engine.node_id(), rate)?, &Design::default()).ok()?;
            s.load_text(p.text).ok()?;
            return Some((p.name.to_string(), s.save_extras(), values(&s)));
        }
        let v = match engine {
            Engine::Drums => {
                let mut n = Drums::new(rate);
                n.load_preset(p);
                values(&n)
            }
            Engine::Pluck => {
                let mut n = Pluck::new(rate);
                n.load_preset(p);
                values(&n)
            }
            Engine::Keys => {
                let mut n = Keys::new(rate);
                n.load_preset(p);
                values(&n)
            }
            Engine::Drawbar => {
                let mut n = Drawbar::new(rate);
                n.load_preset(p);
                values(&n)
            }
            Engine::Squelch => {
                let mut n = Squelch::new(rate);
                n.load_preset(p);
                values(&n)
            }
            Engine::Analog => {
                let mut n = Analog::new(rate);
                n.load_preset(p);
                values(&n)
            }
            Engine::Strings => {
                let mut n = Strings::new(rate);
                n.load_preset(p);
                values(&n)
            }
            Engine::Bass => {
                let mut n = Bass::new(rate);
                n.load_preset(p);
                values(&n)
            }
            Engine::Wind => {
                let mut n = Wind::new(rate);
                n.load_preset(p);
                values(&n)
            }
            Engine::Choir => {
                let mut n = Choir::new(rate);
                n.load_preset(p);
                values(&n)
            }
            Engine::Fm => {
                let mut n = Fm::new(rate);
                n.load_preset(p);
                values(&n)
            }
            Engine::Poly => {
                let mut n = Poly::new(rate);
                n.load_preset(p);
                values(&n)
            }
        };
        Some((p.name.to_string(), Vec::new(), v))
    }

    /// What the Synth tab shows for a track's own instrument `node`
    /// (`engine`'s, or a plugin's with no presets), labelled `track`.
    pub fn state_of(
        &self,
        node: &dyn Node,
        engine: Option<Engine>,
        track: &str,
        preset: Option<usize>,
    ) -> SynthState {
        let presets = engine.map(|e| self.preset_names(e)).unwrap_or_default();
        SynthState::from_node(node, track, Color::from_pad(PAD_WHITE), presets, preset)
    }
}

/// A scale's name for the chip and the log: "D Dorian", or its notes'
/// count when it isn't one of the built-in scales ("D, 6 notes").
pub fn scale_name(s: &Scale) -> String {
    use mpfree_arp::scale::SCALES;
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
    #[test]
    fn every_engines_node_id_is_its_nodes() {
        let synth = super::Synth::new(44_100);
        for e in super::Engine::ALL {
            assert_eq!(synth.sound_of(e).0, e.node_id());
            assert_eq!(super::Engine::from_node_id(e.node_id()), Some(e));
        }
    }

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
        s.queued()
    }

    #[test]
    fn the_sustain_pedal_holds_the_pads_engines_notes_until_it_comes_up() {
        let mut s = Synth::new(44_100);
        s.note_on_note(60, 100, 0);
        s.control(Ctl::Sustain, 1.0, 1);
        s.note_off_note(60, 2);
        s.control(Ctl::ModWheel, 0.25, 3);
        // The note-off waits for the pedal; the wheel goes through.
        assert_eq!(
            queued(&s),
            vec![
                NodeEvent::NoteOn {
                    note: 60,
                    velocity: 100
                },
                NodeEvent::Control {
                    ctl: Ctl::ModWheel,
                    value: 0.25
                },
            ]
        );
        s.control(Ctl::Sustain, 0.0, 4);
        assert_eq!(queued(&s).last(), Some(&NodeEvent::NoteOff { note: 60 }));
        // An engine switch ends what the pedal held on the old one.
        s.control(Ctl::Sustain, 1.0, 5);
        s.note_on_note(62, 100, 6);
        s.note_off_note(62, 7);
        with(&mut s, |p| p.engine = Engine::Fm);
        s.control(Ctl::Sustain, 0.0, 8);
        assert!(!queued(&s).contains(&NodeEvent::NoteOff { note: 62 }));
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
            Engine::Keys,
            Engine::Drawbar,
            Engine::Squelch,
            Engine::Analog,
            Engine::Strings,
            Engine::Bass,
            Engine::Wind,
            Engine::Choir,
        ] {
            with(&mut s, |p| p.engine = engine);
            // A preset other than the first, with a value changed after.
            let i = 1.min(s.preset_count() - 1);
            s.load_preset(i);
            let edit = s
                .node()
                .params()
                .iter()
                .find(|p| !p.readonly && p.max > p.min && p.curve == mpfree_node::Curve::Linear)
                .map(|p| (p.id, p.min + (p.max - p.min) * 0.3));
            if let Some((id, v)) = edit {
                s.set(id, v);
            }
            let st = s.state();
            let mut copy = fresh_node(st.info.id, &st.extras, &st.values, 48_000).unwrap();
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
        assert!(fresh_node("mpfree.nothing", &[], &[], 48_000).is_none());
        assert!(fresh_node("mpfree.poly", &[], &[0.5], 48_000).is_none());
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

    /// Live transpose (sequencer S4): a pad's notes start moved by its
    /// track's shift, and the pad's note-off and pressure reach the notes
    /// it started. In key, a pad keeps to the scale; the Drums engine's
    /// pads never move.
    /// Instrument pads (the Pads tab's): a pad's voice is of its own
    /// engine, whichever is on the pads; a note two pads hold, or a pad
    /// and the pads' own notes on the same engine, ends with the last of
    /// them; a pad hit again ends its note first; a panic ends them all.
    #[test]
    fn an_instrument_pads_note_ends_with_the_last_that_holds_it() {
        let mut s = Synth::new(44_100);
        let on = |note, velocity| NodeEvent::NoteOn { note, velocity };
        let off = |note| NodeEvent::NoteOff { note };
        // Two pads (of two tracks) hold Poly's C4; one holds FM's D4.
        s.pad_voice_on(0, 5, (Engine::Poly, 60), 100, 0);
        s.pad_voice_on(3, 9, (Engine::Poly, 60), 80, 0);
        s.pad_voice_on(0, 6, (Engine::Fm, 62), 70, 0);
        assert_eq!(s.pad_voices_held(), 3);
        s.pad_voice_off(0, 5, 0);
        assert!(!queued(&s).contains(&off(60)), "the other pad holds it");
        // Poly is on the pads: a pad layout's C4 holds it too.
        s.note_on_note(60, 90, 0);
        s.pad_voice_off(3, 9, 0);
        assert!(!queued(&s).contains(&off(60)));
        s.note_off_note(60, 0);
        assert_eq!(
            queued(&s),
            [on(60, 100), on(60, 80), on(60, 90), off(60)],
            "one note-off, from the last"
        );
        // FM's pad hit again: its note ends first.
        s.pad_voice_on(0, 6, (Engine::Fm, 64), 70, 0);
        assert_eq!(s.queued_on(Engine::Fm), [on(62, 70), off(62), on(64, 70)]);
        // A panic.
        s.all_off();
        assert_eq!(s.pad_voices_held(), 0);
        assert_eq!(s.queued_on(Engine::Fm).last(), Some(&NodeEvent::AllOff));
        // A pad past the tracks or the kit plays nothing.
        s.pad_voice_on(mpfree_seq::MAX_TRACKS as u16, 0, (Engine::Fm, 60), 70, 0);
        s.pad_voice_on(0, 128, (Engine::Fm, 60), 70, 0);
        assert_eq!(s.pad_voices_held(), 0);
    }

    #[test]
    fn two_tracks_on_one_pad_each_end_their_own_notes() {
        let mut s = Synth::new(44_100);
        let mut frame = 0u64;
        let on = |note| NodeEvent::NoteOn {
            note,
            velocity: 100,
        };
        // Pad 0 is C3 (48). Track 2's clip holds it; track 5 hits it a
        // fourth up and lets go; the pads hit it as it is.
        s.note_on_shifted(Player::Track(2), 0, 100, 0, &Shift::NONE);
        s.note_on_shifted(Player::Track(5), 0, 100, 1, &Shift::semitones(5));
        s.note_on(0, 100, 2);
        s.note_off_by(Player::Track(5), 0, 3);
        assert_eq!(
            queued(&s),
            vec![on(48), on(53), on(48), NodeEvent::NoteOff { note: 53 }],
            "track 5's let-go ends only its F3; track 2's C3 plays on"
        );
        run(&mut s, &mut frame, 1, 64);
        // The pads' let-go leaves track 2's C3 held (two hold it); track
        // 2's then ends it.
        s.note_off(0, 0);
        assert!(queued(&s).is_empty(), "C3 is still track 2's");
        s.pressure_at(Player::Track(2), 0, 70, 1);
        s.note_off_by(Player::Track(2), 0, 2);
        assert_eq!(
            queued(&s),
            vec![
                NodeEvent::Pressure {
                    note: 48,
                    value: 70
                },
                NodeEvent::NoteOff { note: 48 },
            ]
        );
        // A hit again on the same player's pad ends its last first (Note
        // Repeat), and a track past the last plays nothing.
        run(&mut s, &mut frame, 1, 64);
        s.note_on_shifted(Player::Track(2), 0, 90, 0, &Shift::NONE);
        s.note_on_shifted(Player::Track(2), 0, 90, 1, &Shift::NONE);
        s.note_on_shifted(
            Player::Track(mpfree_seq::MAX_TRACKS),
            0,
            90,
            2,
            &Shift::NONE,
        );
        let q = queued(&s);
        assert_eq!(q.len(), 3, "{q:?}");
        assert_eq!(q[1], NodeEvent::NoteOff { note: 48 });
        // The pad's own pressure reaches whoever holds it; a panic forgets
        // every player's pads.
        run(&mut s, &mut frame, 1, 64);
        s.pressure(0, 40);
        assert_eq!(
            queued(&s),
            vec![NodeEvent::Pressure {
                note: 48,
                value: 40
            }]
        );
        s.all_off();
        run(&mut s, &mut frame, 1, 64);
        s.note_off_by(Player::Track(2), 0, 0);
        assert!(queued(&s).is_empty(), "nothing held after the panic");
    }

    #[test]
    fn a_shifted_pad_starts_its_notes_moved_and_ends_them() {
        let mut s = Synth::new(44_100);
        // Pad 0 plays C3 (48): up a fourth, F3.
        s.note_on_shifted(Player::Track(0), 0, 100, 0, &Shift::semitones(5));
        s.pressure_at(Player::Track(0), 0, 60, 2);
        s.note_off_by(Player::Track(0), 0, 4);
        assert_eq!(
            queued(&s),
            vec![
                NodeEvent::NoteOn {
                    note: 53,
                    velocity: 100
                },
                NodeEvent::Pressure {
                    note: 53,
                    value: 60
                },
                NodeEvent::NoteOff { note: 53 },
            ]
        );
        assert!(s.plays_notes());
        assert_eq!(s.transpose_key(), None, "Chromatic: semitones");
        let mut set = s.settings();
        set.scale = scale("Minor", 0);
        set.set_layout(PadLayout::InKey);
        s.set_settings(set);
        assert!(s.transpose_key().is_some());
        set.set_layout(PadLayout::Chromatic);
        s.set_settings(set);
        assert_eq!(s.transpose_key(), None);
    }

    /// A keyboard's wheel bends every note; an MPE note's own slide or
    /// tone reaches only a note something holds, so a note the MIDI
    /// effects moved keeps nothing for a later one.
    #[test]
    fn the_wheel_bends_and_a_notes_expression_needs_the_note_held() {
        use mpfree_node::Expr;
        let mut s = Synth::new(44_100);
        s.note_on_note(60, 90, 0);
        s.bend(0.5);
        s.bend(f32::NAN);
        s.bend(3.0);
        s.expression_note(60, Expr::Slide, 2.0);
        s.expression_note(64, Expr::Tone, 0.5);
        s.note_off_note(60, 0);
        s.expression_note(60, Expr::Tone, 0.5);
        assert_eq!(
            queued(&s),
            vec![
                NodeEvent::NoteOn {
                    note: 60,
                    velocity: 90
                },
                NodeEvent::Bend { value: 0.5 },
                NodeEvent::Bend { value: 1.0 },
                NodeEvent::Expression {
                    note: 60,
                    kind: Expr::Slide,
                    value: 2.0
                },
                NodeEvent::NoteOff { note: 60 },
            ]
        );
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
    fn a_clips_lock_plays_on_its_frame_and_its_end_puts_the_sound_back() {
        let mut s = Synth::new(44_100);
        let p = *s
            .node()
            .params()
            .iter()
            .find(|p| !p.readonly && p.max > p.min && p.curve == mpfree_node::Curve::Linear)
            .unwrap();
        let own = s.value(p.id);
        let locked = if own < (p.min + p.max) / 2.0 {
            p.max
        } else {
            p.min
        };
        assert!(s.lock(0, p.id, Some(locked), 7));
        assert_eq!(
            queued(&s),
            vec![NodeEvent::Param {
                id: p.id,
                value: locked
            }]
        );
        let mut frame = 0;
        run(&mut s, &mut frame, 1, 256);
        assert_eq!(s.value(p.id), locked);
        // Another step's lock keeps the sound's own value under it.
        assert!(s.lock(0, p.id, Some(locked), 0));
        run(&mut s, &mut frame, 1, 256);
        assert_eq!(s.locks_holding(), 1);
        assert!(s.lock(0, p.id, None, 3));
        assert_eq!(
            queued(&s),
            vec![NodeEvent::Param {
                id: p.id,
                value: own
            }]
        );
        run(&mut s, &mut frame, 1, 256);
        assert_eq!(s.value(p.id), own);
        assert_eq!(s.locks_holding(), 0);
        // A turn on the Synth tab under a lock is what its end comes back
        // to; the hand holds the knob a moment, so the lock's end and a
        // new lock then leave the turn as it is (knob owners, locks.rs).
        let mid = (p.min + p.max) / 2.0;
        assert!(s.lock(0, p.id, Some(locked), 0));
        run(&mut s, &mut frame, 1, 256);
        s.set(p.id, mid);
        assert!(s.lock(0, p.id, None, 0));
        assert!(s.lock(0, p.id, Some(locked), 0));
        assert!(queued(&s).is_empty(), "{:?}", queued(&s));
        run(&mut s, &mut frame, 1, 256);
        assert_eq!(s.value(p.id), mid);
        // Let go, the lock takes the knob back by a short glide, and its
        // end comes back to the turn.
        let hand = (44_100.0 * crate::locks::HAND_S) as usize;
        run(&mut s, &mut frame, hand / 256 + 12, 256);
        assert_eq!(s.value(p.id), locked);
        assert!(s.lock(0, p.id, None, 0));
        assert!(queued(&s).ends_with(&[NodeEvent::Param {
            id: p.id,
            value: mid
        }]));
        // A meter can't be locked, and an end with no lock sends nothing.
        run(&mut s, &mut frame, 1, 256);
        if let Some(m) = s.node().params().iter().find(|p| p.readonly) {
            assert!(!s.lock(0, m.id, Some(1.0), 0));
        }
        assert!(s.lock(0, p.id, None, 0));
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
            // Strings' ensemble lines run a few ms behind the keys, so
            // its first sound can fall in the next block.
            let mut added = l.iter().any(|&x| x != 0.25);
            if engine == Engine::Strings && !added {
                let mut l2 = [0.25f32; 128];
                let mut r2 = [0.25f32; 128];
                for _ in 0..3 {
                    s.render(frame, 120.0, &mut l2, &mut r2);
                    frame += 128;
                    added |= l2.iter().any(|&x| x != 0.25);
                }
            }
            assert!(added, "{engine:?} adds to what is there");
            assert_eq!(s.active_voices(), 1);
            // The choir swells in over a quarter second, so it is
            // measured once the swell is up.
            if engine == Engine::Choir {
                run(&mut s, &mut frame, 100, 128);
            }
            let sound = run(&mut s, &mut frame, 40, 128);
            let p = peak(&sound);
            assert!(
                p > 0.05 && p < 1.0,
                "{engine:?}: a note at about -12 dBFS: peak {p}"
            );
            s.note_off(0, 0);
            run(&mut s, &mut frame, 2000, 128);
            assert_eq!(s.active_voices(), 0, "{engine:?}");
            let tail = peak(&run(&mut s, &mut frame, 2, 128));
            assert!(
                tail == 0.0,
                "{engine:?}: silent once its voices end, not {tail}"
            );
            assert_eq!(
                s.counters(),
                Counters {
                    notes: 1,
                    peak_voices: 1,
                    dropped: 0,
                    busy: 0,
                    ends_lost: 0,
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

    /// A Keys sound with its own Ensemble and a macro moving the tone and
    /// the Ensemble's mix, as a user preset's text.
    fn space_keys(rate: u32) -> String {
        use mpfree_instruments::sound::{FX_BASE, MACRO_BASE};
        use mpfree_instruments::{Macro, MacroMap};
        let design = Design {
            fx: vec!["mpfree.fx.ensemble".into()],
            macros: vec![Macro {
                name: "Space".into(),
                maps: vec![
                    MacroMap {
                        target: mpfree_synth::keys_params::TONE,
                        from: -20.0,
                        to: 20.0,
                        curve: mpfree_mod::MapCurve::Linear,
                    },
                    MacroMap {
                        target: FX_BASE + mpfree_fx::ids::ensemble::MIX,
                        from: 0.0,
                        to: 80.0,
                        curve: mpfree_mod::MapCurve::Linear,
                    },
                ],
            }],
        };
        let mut sound = Sound::new(Box::new(Keys::new(rate)), &design).unwrap();
        sound.set_param(MACRO_BASE, 60.0);
        sound.set_param(mpfree_synth::keys_params::DECAY, 3200.0);
        sound.to_text("Space Keys", &[], &[])
    }

    /// A user preset with its own effects and macros plays them on the
    /// pads; Cancel brings the plain sound back, a factory preset takes
    /// them away, and a project keeps them.
    #[test]
    fn a_sound_with_its_own_effects_plays_on_the_pads_and_is_kept() {
        use mpfree_instruments::sound::MACRO_BASE;
        let rate = 44_100;
        let mut s = on_engine(Engine::Keys);
        let plain = sound(&s);
        let n = s.node().params().len();
        let text = space_keys(rate);
        assert_eq!(
            s.try_preset("Keys", "Mine", &text).as_deref(),
            Ok("Space Keys")
        );
        assert_eq!(s.value(MACRO_BASE), 60.0);
        assert_eq!(s.state().params[0].name, "Space", "the macros first");
        assert!(s.node().params().len() > n, "the Ensemble's settings");
        assert!(
            s.state()
                .extras
                .iter()
                .any(|(k, v)| k == "fx.1" && v == "mpfree.fx.ensemble"),
            "{:?}",
            s.state().extras
        );
        // The macro turned on the pads moves the Ensemble.
        let mix = mpfree_instruments::sound::FX_BASE + mpfree_fx::ids::ensemble::MIX;
        s.set(MACRO_BASE, 100.0);
        assert_eq!(s.value(mix), 80.0);
        s.revert_preset();
        assert_eq!(sound(&s), plain, "Cancel: the plain sound, to the bit");
        // Loaded, saved with the project and put back.
        s.load_preset_file("Keys", "Space Keys", &text).unwrap();
        let saved = s.sounds();
        let want = sound(&s);
        let mut t = on_engine(Engine::Keys);
        assert!(t.set_sounds(&saved).is_empty());
        assert_eq!(sound(&t), want, "the project's sound, effects and all");
        // A factory preset is plain.
        s.load_preset(0);
        assert_eq!(s.node().params().len(), n);
        assert!(s.state().extras.is_empty());
    }

    /// A track's copy of a sound with effects and macros plays it as the
    /// pads do.
    #[test]
    fn a_fresh_copy_of_a_sound_plays_it_as_the_pads_do() {
        let rate = 48_000;
        let mut s = Synth::new(rate);
        with(&mut s, |p| p.engine = Engine::Keys);
        s.load_preset_file("Keys", "Space Keys", &space_keys(rate))
            .unwrap();
        let st = s.state();
        let mut copy = fresh_node(st.info.id, &st.extras, &st.values, rate).unwrap();
        for (p, v) in st.params.iter().zip(&st.values) {
            assert_eq!(copy.param(p.id).to_bits(), v.to_bits(), "{}", p.name);
        }
        let n = s.notes(0)[0];
        let theirs = note(s.node_mut(), n);
        let ours = note(copy.as_mut(), n);
        assert!(peak(&theirs) > 0.001);
        assert_eq!(ours, theirs);
        // A design it can't build makes nothing.
        let mut bad = st.extras.clone();
        bad.push(("fx.2".into(), "mpfree.fx.nothing".into()));
        assert!(fresh_node(st.info.id, &bad, &st.values, rate).is_none());
    }

    /// A factory sound brings its own effects and macros, on the pads and
    /// to a track's copy; a plain factory preset after it takes them away.
    #[test]
    fn a_factory_sound_brings_its_effects_and_macros() {
        let rate = 48_000;
        let mut s = Synth::new(rate);
        with(&mut s, |p| p.engine = Engine::Keys);
        let n = s.node().params().len();
        let i = s
            .preset_names(Engine::Keys)
            .iter()
            .position(|p| p == "Phase Tines")
            .unwrap();
        assert_eq!(s.load_preset(i), Some("Phase Tines"));
        assert_eq!(s.preset_name(), "Phase Tines");
        let st = s.state();
        assert_eq!(st.params[0].name, "Tone", "the macros first");
        assert!(
            st.extras
                .iter()
                .any(|(k, v)| k == "fx.1" && v == "mpfree.fx.phaser"),
            "{:?}",
            st.extras
        );
        // A track's copy plays the same sound.
        let (name, extras, values) = s.preset_values(Engine::Keys, i).unwrap();
        assert_eq!(name, "Phase Tines");
        assert_eq!(extras, st.extras);
        assert_eq!(values, st.values);
        let mut copy = fresh_node("mpfree.keys", &extras, &values, rate).unwrap();
        let k = s.notes(0)[0];
        let theirs = note(s.node_mut(), k);
        assert!(peak(&theirs) > 0.001);
        assert_eq!(note(copy.as_mut(), k), theirs);
        // A plain factory preset takes them away.
        assert_eq!(s.load_preset(0), Some("Init"));
        assert_eq!(s.node().params().len(), n);
        assert!(s.state().extras.is_empty());
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
        assert_eq!(s.try_preset("Poly", "Mine", &mine).as_deref(), Ok("Mine"));
        assert!(s.trying_preset());
        assert_eq!(sound(&s), want);
        assert_eq!(s.try_preset("poly", "Two", &two).as_deref(), Ok("Two"));
        assert_eq!(s.own_name(), Some("Two"), "named by its file");
        // Cancel puts the first sound back, every setting to the bit.
        s.revert_preset();
        assert!(!s.trying_preset());
        assert_eq!(sound(&s), before);
        assert_eq!(s.own_name(), None);
        s.revert_preset();
        assert_eq!(sound(&s), before, "nothing tried, nothing to put back");
        // Load after trying keeps what was tried.
        s.try_preset("Poly", "Mine", &two).unwrap();
        assert_eq!(
            s.load_preset_file("Poly", "Mine", &mine).as_deref(),
            Ok("Mine")
        );
        assert!(!s.trying_preset());
        s.revert_preset();
        assert_eq!(sound(&s), want);
        // Another engine's preset, a file in another engine's folder and a
        // file that isn't a preset change nothing and keep nothing.
        let (fm_folder, bell) = on_engine(Engine::Fm).save_preset("Bell", &[], &[]);
        assert_eq!(fm_folder, "FM");
        assert!(matches!(
            s.try_preset("FM", "Mine", &bell),
            Err(SoundError::Preset(PresetError::WrongEngine { .. }))
        ));
        assert!(s.try_preset("", "Mine", &bell).is_err(), "FM's engine line");
        assert!(
            s.try_preset("FM", "Mine", &mine).is_err(),
            "a Poly file in FM's folder"
        );
        assert!(s.try_preset("Poly", "Mine", "not a preset").is_err());
        assert!(!s.trying_preset());
        assert_eq!(sound(&s), want);
        // A file straight in Instruments goes by its engine line.
        assert!(s.try_preset("", "Mine", &two).is_ok());
        s.revert_preset();
        assert_eq!(sound(&s), want);
        // Folders are engines' names, in any case (the Wavetable synth's is
        // Poly's now); the Q-Link sets ride along in a saved preset.
        assert_eq!(Engine::from_folder("wavetable"), Some(Engine::Poly));
        assert_eq!(Engine::from_folder("Strings"), Some(Engine::Strings));
        assert_eq!(Engine::from_folder("bass"), Some(Engine::Bass));
        assert_eq!(Engine::from_folder("Wind"), Some(Engine::Wind));
        assert_eq!(Engine::from_folder("choir"), Some(Engine::Choir));
        assert_eq!(Engine::from_folder("Favourites"), None);
        let qset = [("qset".to_string(), "31 32 - 33 Tone".to_string())];
        let (_, text) = s.save_preset("Q", &[], &qset);
        assert!(text.contains("\nqset = 31 32 - 33 Tone\n"), "{text}");
    }

    /// The picker's Factory place tries the engine's own presets: Cancel
    /// puts back the sound and the preset it had, Load keeps one. A file
    /// of the musician's is named on the Synth tab by its file's name.
    /// A preset saved for the Wavetable synth (gone into Poly on 19 Nov)
    /// plays on Poly, from its old folder or straight in Instruments, with
    /// its effects and macros; a track's copy takes it the same way.
    #[test]
    fn a_wavetable_preset_file_plays_on_poly() {
        use mpfree_node::preset::key_of;
        use mpfree_synth::wavetable::params as wp;
        use mpfree_synth::wavetable::to_poly::node_to_poly;
        let mut wt = Wavetable::new(48_000);
        wt.load_preset(&Wavetable::presets()[1]);
        let mut text =
            mpfree_node::preset::Preset::capture(&wt, "Old Morph", &[]).to_text(wt.params());
        let map = format!("{} 0 90", key_of(wt.params(), wp::POSITION).unwrap());
        text.push_str(&format!(
            "fx.1 = mpfree.fx.ensemble\nmacro.1 = Sweep\nmacro.1.map = {map}\n"
        ));
        let mut s = Synth::new(48_000);
        assert_eq!(
            s.try_preset("Wavetable", "Mine", &text).unwrap(),
            "Old Morph"
        );
        // (The macro, at 0, sets osc 1's position.)
        for (id, v) in node_to_poly(&wt).values {
            if id == pp::OSC1_POS {
                assert_eq!(s.value(id), 0.0);
                continue;
            }
            assert!(
                (s.value(id) - v).abs() <= 1e-4 * v.abs().max(1.0),
                "{id}: {} != {v}",
                s.value(id)
            );
        }
        let lines = s.poly.design_lines();
        let poly_map = format!("{} 0 90 linear", key_of(pp::PARAMS, pp::OSC1_POS).unwrap());
        for want in [
            ("fx.1", "mpfree.fx.ensemble"),
            ("macro.1", "Sweep"),
            ("macro.1.map", poly_map.as_str()),
        ] {
            assert!(
                lines.iter().any(|(k, v)| k == want.0 && v == want.1),
                "{want:?} in {lines:?}"
            );
        }
        s.revert_preset();
        assert!(s.try_preset("", "Mine", &text).is_ok(), "its engine line");
        let (name, extras, values) = s.text_sound(Engine::Poly, "Wavetable", &text).unwrap();
        assert_eq!(name, "Old Morph");
        assert!(extras.iter().any(|(k, _)| k == "fx.1"));
        assert!(fresh_node("mpfree.poly", &extras, &values, 48_000).is_some());
        // Another engine on the pads doesn't take it.
        with(&mut s, |p| p.engine = Engine::Fm);
        assert!(s.try_preset("Wavetable", "Mine", &text).is_err());
    }

    #[test]
    fn the_picker_tries_the_engines_own_presets_and_names_the_musicians() {
        let mut s = Synth::new(44_100);
        s.load_preset(2);
        let p = *s.node().params().iter().find(|p| !p.readonly).unwrap();
        s.set(p.id, p.from_unit(0.9));
        let before = sound(&s);
        let names = s.preset_names(Engine::Poly);
        assert_eq!(s.try_factory(4).as_deref(), Some(names[4].as_str()));
        assert!(s.trying_preset());
        assert_eq!(s.state().preset, Some(4));
        assert_ne!(sound(&s), before);
        s.try_factory(5);
        s.revert_preset();
        assert!(!s.trying_preset());
        assert_eq!(sound(&s), before, "the edited sound, to the bit");
        assert_eq!(s.state().preset, Some(2));
        // Past the end of the list nothing changes.
        assert_eq!(s.try_factory(names.len()), None);
        s.revert_preset();
        assert_eq!(sound(&s), before);
        // Load after a try keeps the one loaded.
        s.try_factory(4);
        s.load_preset(5);
        assert!(!s.trying_preset());
        assert_eq!(s.state().preset, Some(5));
        // One of the musician's: named by its file, back after a try, gone
        // with a preset of the list; a save names the sound.
        let mut other = Synth::new(44_100);
        other.load_preset(3);
        let want = sound(&other);
        let (_, mine) = other.save_preset("Inside", &[], &[]);
        assert_eq!(
            s.load_preset_file("Poly", "On disk", &mine).as_deref(),
            Ok("Inside")
        );
        assert_eq!(s.own_name(), Some("On disk"));
        assert_eq!(s.state().own.as_deref(), Some("On disk"));
        s.try_factory(1);
        assert_eq!(s.own_name(), None);
        s.revert_preset();
        assert_eq!(s.own_name(), Some("On disk"));
        assert_eq!(sound(&s), want);
        s.load_preset(0);
        assert_eq!(s.state().own, None);
        s.saved_as("Kept");
        assert_eq!(s.own_name(), Some("Kept"));
        // A track's own copy takes a file as the pads do, and its sound
        // saves as a file that plays the same.
        let (name, extras, values) = s.text_sound(Engine::Poly, "Poly", &mine).unwrap();
        assert_eq!(name, "Inside");
        let (_, _, theirs) = other.sound_of(Engine::Poly);
        assert_eq!(values, theirs);
        let (folder, text) = s.copy_text(Engine::Poly, &extras, &values, "Back").unwrap();
        assert_eq!(folder, "Poly");
        let mut t = Synth::new(44_100);
        t.load_preset_file("Poly", "Back", &text).unwrap();
        assert_eq!(sound(&t), want);
        assert!(s.text_sound(Engine::Keys, "Poly", &mine).is_err());
        assert!(s.text_sound(Engine::Keys, "", &mine).is_err());
        assert!(s
            .copy_text(Engine::Poly, &extras, &values[1..], "Short")
            .is_none());
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
        s.try_preset("Drums", "Mine", &text).unwrap();
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

    /// Every engine, on the pads with a chord ringing, is silent after
    /// STOP's fade: nothing comes back once the fade ends (a release tail,
    /// a string machine's ensemble).
    #[test]
    fn stop_silences_every_engine() {
        for e in Engine::ALL {
            let mut s = Synth::new(44_100);
            with(&mut s, |p| p.engine = e);
            let mut frame = 0;
            for pad in [0, 4, 7] {
                s.note_on(pad, 120, 0);
            }
            let before = run(&mut s, &mut frame, 40, 128);
            assert!(peak(&before) > 1e-3, "{e:?} plays");
            s.stop();
            let out = run(&mut s, &mut frame, 200, 128);
            assert!(
                out[2 * FADE as usize..].iter().all(|&x| x == 0.0),
                "{e:?} rings on after STOP: {}",
                peak(&out[2 * FADE as usize..])
            );
            assert_eq!(s.active_voices(), 0, "{e:?}");
        }
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
    fn a_busy_period_leaves_new_notes_out_and_every_ending_in() {
        let mut s = Synth::new(44_100);
        // Every note starts (the arp's, by number), then five more hits
        // past the period's room are left out; so are bends past it.
        for note in 0..128 {
            s.note_on_note(note, 80, 0);
        }
        for note in 0..5 {
            s.note_on_note(note, 80, 0);
        }
        for _ in 0..ROOM.values + 3 {
            s.bend(0.1);
        }
        let c = s.counters();
        assert_eq!((c.busy, c.ends_lost), (5 + 3, 0));
        // Every note-off still goes in, the last holder's: the five left
        // out were counted as holders, so their notes end with them.
        for note in 0..128 {
            s.note_off_note(note, 0);
        }
        let offs = |s: &Synth| {
            s.queued()
                .iter()
                .filter(|e| matches!(e, NodeEvent::NoteOff { .. }))
                .count()
        };
        assert_eq!(offs(&s), 123);
        for note in 0..5 {
            s.note_off_note(note, 0);
        }
        assert_eq!(offs(&s), 128);
        assert!(s.len[s.engine().index()] <= MAX_EVENTS);
        assert_eq!(s.counters().ends_lost, 0);
        // A new period has room again.
        let mut frame = 0;
        run(&mut s, &mut frame, 1, 128);
        assert_eq!(s.len, [0; ENGINES]);
        s.note_on_note(60, 80, 0);
        assert_eq!(s.queued().len(), 1);
        // A hit past the 16 pads plays nothing, and is counted.
        let dropped = s.counters().dropped;
        s.note_on_shifted(Player::Pads, PADS, 80, 0, &Shift::NONE);
        assert_eq!(s.counters().dropped, dropped + 1);
    }

    #[test]
    fn a_lock_put_back_always_finds_room_and_new_locks_ask_first() {
        let mut s = Synth::new(44_100);
        let ids: Vec<u16> = s
            .node()
            .params()
            .iter()
            .filter(|p| !p.readonly && p.max > p.min)
            .map(|p| p.id)
            .take(crate::locks::MAX)
            .collect();
        assert!(ids.len() >= 32, "enough knobs to fill a period");
        // A period's values all spent: the first track's locks' values
        // (it owns the knobs) are left out, but each new lock is held, up
        // to the new holds a period grants; the rest are left out.
        for _ in 0..ROOM.values {
            s.bend(0.1);
        }
        let tracks = ROOM.unlocks / 2 / ids.len() + 1;
        let mut held = 0;
        for t in 0..tracks {
            for &id in &ids {
                assert!(s.lock(t, id, Some(0.5), 0));
                held += 1;
            }
        }
        let granted = held.min(ROOM.unlocks / 2).min(crate::locks::HOLDS);
        assert_eq!(s.locks_holding(), ids.len().min(granted));
        assert_eq!(s.counters().busy, (ids.len() + held - granted) as u64);
        // Every lock ends in the same period: each value put back fits.
        let before = s.queued().len();
        for t in 0..tracks {
            for &id in &ids {
                assert!(s.lock(t, id, None, 0));
            }
        }
        assert_eq!(s.locks_holding(), 0);
        assert!(s.queued().len() > before, "the values under came back");
        assert_eq!(s.counters().ends_lost, 0);
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
    fn the_tuning_picked_reaches_every_engine_but_drums() {
        let just = TuningPick {
            builtin: Builtin::Just,
            root: 0,
            a4_hz: 440,
            file: None,
        };
        let a432 = TuningPick {
            a4_hz: 432,
            ..TuningPick::default()
        };
        // Pad 5: E3 on the synths (386 cents above C in just intonation,
        // 400 in 12-TET), a drum on Drums.
        let play = |engine: Engine, picks: &[TuningPick]| {
            let mut s = Synth::new(44_100);
            with(&mut s, |p| p.engine = engine);
            for &t in picks {
                with(&mut s, |p| p.tuning = t);
            }
            let mut frame = 0;
            s.note_on(4, 100, 0);
            run(&mut s, &mut frame, 60, 128)
        };
        for engine in Engine::ALL {
            let plain = play(engine, &[]);
            assert!(peak(&plain) > 0.01, "{engine:?} is silent");
            let back = play(engine, &[just, a432, TuningPick::default()]);
            assert_eq!(
                back, plain,
                "{engine:?}: back to 12-TET at 440 plays as before"
            );
            for t in [just, a432] {
                let tuned = play(engine, &[t]);
                if engine == Engine::Drums {
                    assert_eq!(tuned, plain, "Drums keep their own pitch");
                } else {
                    assert_ne!(tuned, plain, "{engine:?} ignored {}", t.label(&[]));
                }
            }
        }
        assert_eq!(TuningPick::default().label(&[]), "12-TET");
        assert_eq!(
            TuningPick {
                root: 2,
                a4_hz: 432,
                ..just
            }
            .label(&[]),
            "Just (5-limit) on D, A4 432 Hz"
        );
    }

    /// Just intonation as a Scala file, and a Dorian scale with a keyboard
    /// map that puts A4 at 432 Hz.
    const JUST_SCL: &str =
        "! just.scl\nJust\n12\n16/15\n9/8\n6/5\n5/4\n4/3\n45/32\n3/2\n8/5\n5/3\n9/5\n15/8\n2/1\n";
    const DORIAN_SCL: &str = "Dorian, just\n7\n9/8\n6/5\n4/3\n3/2\n5/3\n9/5\n2/1\n";
    const DORIAN_KBM: &str = "12\n0\n127\n62\n69\n432.0\n7\n0\nx\n1\n2\nx\n3\nx\n4\nx\n5\n6\nx\n";

    fn tuning_files() -> Vec<TuningFile> {
        vec![
            TuningFile::parse("just", JUST_SCL, None).unwrap(),
            TuningFile::parse("dorian", DORIAN_SCL, Some(DORIAN_KBM)).unwrap(),
        ]
    }

    #[test]
    fn a_tuning_file_plays_on_the_root_and_a4_picked_unless_its_map_sets_them() {
        let files = tuning_files();
        assert!(!files[0].mapped() && files[1].mapped());
        assert_eq!((files[0].notes(), files[1].notes()), (12, 7));
        let pick = |file: Option<u8>, root: u8, a4_hz: u16| TuningPick {
            builtin: Builtin::Just,
            file,
            root,
            a4_hz,
        };
        // A .scl alone is the built-in tuning it spells, to the bit.
        for (root, a4) in [(0, 440), (2, 432), (11, 480)] {
            assert_eq!(
                pick(Some(0), root, a4).tuning(&files),
                pick(None, root, a4).tuning(&files)
            );
        }
        // A keyboard map fixes the root and the pitch.
        let fixed = pick(Some(1), 0, 440).tuning(&files);
        assert_eq!(pick(Some(1), 5, 400).tuning(&files), fixed);
        assert!((fixed.hz(69.0) - 432.0).abs() < 1e-3);
        // A file that isn't there plays the built-in tuning.
        assert_eq!(
            pick(Some(9), 2, 440).tuning(&files),
            pick(None, 2, 440).tuning(&files)
        );
        assert_eq!(pick(Some(0), 2, 432).label(&files), "just on D, A4 432 Hz");
        assert_eq!(pick(Some(1), 2, 432).label(&files), "dorian");

        // On the synth: pad 5 plays as with the built-in tuning.
        let play = |files: Vec<TuningFile>, t: TuningPick| {
            let mut s = Synth::new(44_100);
            s.set_tuning_files(files);
            with(&mut s, |p| p.tuning = t);
            let mut frame = 0;
            s.note_on(4, 100, 0);
            (run(&mut s, &mut frame, 60, 128), s)
        };
        let (builtin, _) = play(Vec::new(), pick(None, 2, 432));
        let (file, s) = play(tuning_files(), pick(Some(0), 2, 432));
        assert_eq!(file, builtin);
        assert_eq!(s.tuning_label(), "just on D, A4 432 Hz");
        assert_eq!(s.tuning_files().len(), 2);
        let (plain, _) = play(Vec::new(), TuningPick::default());
        let (dorian, s) = play(tuning_files(), pick(Some(1), 0, 440));
        assert_ne!(dorian, plain);
        assert_eq!(s.tuning_label(), "dorian");
        // A pick of a file the synth doesn't have falls to its built-in
        // tuning, and the settings say so.
        let (missing, s) = play(tuning_files(), pick(Some(7), 2, 432));
        assert_eq!(missing, builtin);
        assert_eq!(s.settings().tuning.file, None);
        // A new list without the file picked does the same.
        let mut s = Synth::new(44_100);
        s.set_tuning_files(tuning_files());
        with(&mut s, |p| p.tuning = pick(Some(1), 2, 432));
        assert_eq!(s.settings().tuning.file, Some(1));
        s.set_tuning_files(tuning_files()[..1].to_vec());
        assert_eq!(s.settings().tuning.file, None);
        assert_eq!(s.tuning_label(), "Just (5-limit) on D, A4 432 Hz");
    }

    #[test]
    fn bad_tuning_files_say_why() {
        let e = TuningFile::parse("x", "desc\n2\n3/2\n", None).unwrap_err();
        assert!(e.contains("2 notes promised"), "{e}");
        let e = TuningFile::parse("x", DORIAN_SCL, Some("12\n0\n127\n60\n")).unwrap_err();
        assert!(
            e.starts_with("its .kbm: ") && e.contains("reference key"),
            "{e}"
        );
    }

    #[test]
    fn the_audio_threads_part_never_allocates() {
        let mut s = Synth::new(44_100);
        s.set_tuning_files(tuning_files());
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
                    // A new tuning is worked out on this thread too,
                    // tuning files (with and without a map) among them.
                    let pick = p / 211 % (Builtin::ALL.len() + 2);
                    set.tuning = TuningPick {
                        builtin: Builtin::ALL[pick % Builtin::ALL.len()],
                        file: pick.checked_sub(Builtin::ALL.len()).map(|f| f as u8),
                        root: (p % 12) as u8,
                        a4_hz: A4_MIN + (p % 100) as u16,
                    };
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
    fn pluck_plays_the_pad_layouts_as_poly_does() {
        let cases = [(
            Engine::Pluck,
            "Pluck",
            PLUCK_START_PRESET,
            mpfree_synth::pluck_params::POLYPHONY,
        )];
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
