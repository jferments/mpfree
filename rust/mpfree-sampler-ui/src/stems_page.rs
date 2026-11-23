//! The Stems page (the stems plan's step 3; its screens are the mockups in
//! mpfree-claude `stems/mockups/`, and `docs/dev/design/stems.md` is the
//! engine's design): over the Sample page, the sound shown split into
//! four new sounds that add up to it, Vocals, Drums, Bass and Other.
//!
//! The pad bar's Stems button opens it for the pad or take the page shows
//! ([`crate::send::STEMS`]). The split itself is the sampler's
//! ([`mpfree_sampler::stems::Split`], a step a job on the sound lane); this
//! module is the page the app shows it on:
//!
//! - [`Stems`]: what the page is set to (what it reads, where the stems go,
//!   which are kept, their levels) and where the split is ([`Phase`]),
//!   with what each press asks of the app ([`Ask`]).
//! - [`StemsView`]: the picture: the title line, the sound over four
//!   lanes, one a stem in its colour with its switch, the segments the
//!   split works in marked along the top, the lanes filling segment by
//!   segment as the stems come.
//! - [`StemsPage`]: the page's widgets: the picture, the panel under it
//!   (Split, Put on, Listen) and the Q-Link column (the four stems'
//!   levels).
//!
//! What the app tells it, it doesn't work out: the sound ([`Sound`]) and
//! the room the split has ([`Room`]: memory, the kit's pads, takes, why it
//! can't start).

use std::cell::Cell;

use mpfree_sampler::model::{Layer, LayerSelect, Pad, Project, Region, Source, StemsTo};
use mpfree_sampler::stems::{segments, Made, Peaks, RATE, SEGMENT};
use mpfree_ui::qlink::{QLinkCard, QLinkColumn};
use mpfree_ui::{Align, EventKind, FontId, Fonts, PaintCx, Rect, Target, Touch, Ui, Widget};

use crate::command::{Change, Recorded};
use crate::panel::{Control, Item, Look, PadCell, Panel, Section};
use crate::slice_tool::BANK;
use crate::wave_tab::{pad_label, WaveState, WaveTab};
use crate::widgets::{Setting, SettingList};

/// The page, and the widgets it adds (the app's own are `header`, `hint`,
/// the pad bar `smp-bar` and its buttons `smp-send`).
pub const PAGE: &str = "Sample Stems";
pub const VIEW: &str = "smp-stems-view";
pub const LIST: &str = "smp-stems-list";
pub const QLINKS: &str = "smp-stems-qlinks";
/// What its widgets' names start with (inside the Sample tab's prefix
/// "smp-", so the app takes them first).
pub const WIDGETS: &str = "smp-stems-";

/// The Sample page's frame (the pad bar and its three buttons over the
/// page, the Q-Link column at the right, the hint line under), with the
/// picture over the 242 px panel, as the Wave page has its wave.
pub const LAYOUT: &str = "col(header:60, row(smp-bar:*, smp-send:460):60, \
                          row(col(smp-stems-view:*, smp-stems-list:242):*, \
                          smp-stems-qlinks:128):*, hint:26)";

/// The stems, in the model's order ([`mpfree_sampler::stems::Stem::ALL`]).
pub const NAMES: [&str; 4] = ["Vocals", "Drums", "Bass", "Other"];
/// Their names on the pad map.
pub const SHORT: [&str; 4] = ["Voc", "Drm", "Bass", "Oth"];
/// Each stem's colour: the theme's series colour this many along
/// ([`mpfree_ui::Theme::track`]), so the pad map, the lanes and the
/// Q-Links agree. In the dark theme Vocals purple, Drums yellow, Bass
/// green, Other blue.
pub const TINTS: [usize; 4] = [3, 2, 1, 0];

/// A stem's level, dB, as a pad's ([`mpfree_sampler::model::Layer`]).
pub const LEVEL_DB: (f32, f32) = (-48.0, 12.0);
pub const LEVEL_STEP_DB: f32 = 0.5;

/// Seconds a segment takes on the MPC One's two job cores at full band:
/// the plan's estimate (62 s for a minute's six segments) until a split
/// here says otherwise ([`Stems::landed`] keeps what it took).
pub const SEGMENT_S: f32 = 10.4;

/// What the page reads of the sound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reads {
    /// Start to End (Akai's "Use trimmed sample").
    #[default]
    Trimmed,
    Whole,
}

impl Reads {
    pub const CHOICES: [&'static str; 2] = ["Start to End", "Whole sound"];

    fn index(self) -> usize {
        usize::from(self == Reads::Whole)
    }
}

/// Where the stems go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Put {
    /// A pad each, the first empty ones after the sound's (or from a pad
    /// tapped on the map).
    #[default]
    Pads,
    /// The sound's own pad plays them as its layers, together, in place
    /// of the sound (Akai's "Assign to Pad").
    Layers,
    /// Takes on the page.
    Takes,
    /// A new kit's pads from A1, on a new track.
    NewKit,
    /// The library, under Samples › Stems; the kit doesn't change.
    Library,
}

impl Put {
    pub const ALL: [Put; 5] = [
        Put::Pads,
        Put::Layers,
        Put::Takes,
        Put::NewKit,
        Put::Library,
    ];

    fn index(self) -> usize {
        Put::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }
}

/// Where the split is.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Phase {
    /// Nothing split yet, or something changed since.
    #[default]
    Ready,
    /// Splitting: how far (0 to 1, a step at a time) and the seconds it
    /// has run.
    Splitting { progress: f32, ran_s: f32 },
    /// Landed where this says ("On A6–A9"); the main button stays faint
    /// until something changes.
    Landed(String),
}

/// What a finished split lands as ([`Stems::land`]).
#[derive(Debug)]
pub enum Landing {
    /// On the kit: its pads, the sound's pad's layers or takes, as one
    /// undo step.
    Kit(Change),
    /// On a new kit's pads from A1, on a new track: the app adds the kit,
    /// as a chop's New kit does.
    NewKit { pads: Vec<Pad>, sounds: Vec<Source> },
    /// Saved in the library under Samples › Stems; the kit doesn't change.
    Library(Vec<Source>),
}

/// The panel's rows, in the list's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Reads,
    /// Text: what it takes, holds and makes.
    Takes,
    Holds,
    Makes,
    /// The bank's pad map, and its ‹ › chip.
    Map,
    Bank,
    Put,
    /// The main button: Split to …, or Stop.
    Go,
    Level(usize),
    PlayAll,
}

impl Row {
    pub const ALL: [Row; 13] = [
        Row::Reads,
        Row::Takes,
        Row::Holds,
        Row::Makes,
        Row::Map,
        Row::Bank,
        Row::Put,
        Row::Go,
        Row::Level(0),
        Row::Level(1),
        Row::Level(2),
        Row::Level(3),
        Row::PlayAll,
    ];

    pub fn index(self) -> usize {
        Row::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }

    /// A row the wheel stops on (not a line of text).
    fn picks(self) -> bool {
        !matches!(self, Row::Takes | Row::Holds | Row::Makes)
    }
}

/// What a press asks of the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// Nothing more than showing the change.
    Nothing,
    /// Start splitting, as the page is set.
    Split,
    /// Stop the split; nothing lands.
    Stop,
    /// Play stem `k` (a tap on its lane, once it has landed).
    Play(usize),
    /// Play all four together at their levels.
    PlayAll,
    /// Stem `k`'s level changed: set it on its pad or layer too, once it
    /// has landed.
    Level(usize),
    /// Say this on the hint line (a faint control tapped).
    Say(String),
}

/// The sound the page splits, as the app reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sound {
    /// "Soul loop".
    pub name: String,
    /// Its pad; None for a take kept on the page.
    pub pad: Option<usize>,
    /// Its frames at `rate`: the whole sound, and Start and End.
    pub rate: u32,
    pub frames: u64,
    pub trimmed: (u64, u64),
    /// It is 16-bit, so are its stems (half the memory of floats).
    pub sixteen: bool,
    /// The whole sound's picture, a column each: the lowest and highest
    /// sample of both sides.
    pub peaks: Vec<(f32, f32)>,
}

impl Sound {
    /// The frames read (`reads`), at the sound's rate.
    pub fn span(&self, reads: Reads) -> (u64, u64) {
        match reads {
            Reads::Trimmed => self.trimmed,
            Reads::Whole => (0, self.frames),
        }
    }

    /// The samples a channel the split works through: those read, at
    /// 44.1 kHz.
    pub fn samples(&self, reads: Reads) -> usize {
        let (a, b) = self.span(reads);
        ((b - a) as f64 * f64::from(RATE) / f64::from(self.rate.max(1))).round() as usize
    }

    pub fn seconds(&self, reads: Reads) -> f32 {
        self.samples(reads) as f32 / RATE as f32
    }
}

/// The room the split has, as the app sees it now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Room {
    /// The bytes the split holds while it runs (the model, the sound read,
    /// its work and the stems), and the bytes free now.
    pub holds: u64,
    pub free: u64,
    /// Which of the kit's pads have a sound (its length is the kit's).
    pub filled: Vec<bool>,
    /// The takes on the page now, and the most it keeps.
    pub takes: usize,
    pub max_takes: usize,
    /// Why Put on New kit can't be picked (the app can't add a kit on a
    /// new track now); None: it can.
    pub new_kit_off: Option<String>,
    /// Put on Library: the app can write the library.
    pub library: bool,
    /// Why no split can start now, in full: the model isn't on the stick
    /// or is damaged, another split runs.
    pub blocked: Option<String>,
}

/// What the page is set to, and where the split is.
#[derive(Clone, Debug, PartialEq)]
pub struct Stems {
    pub sound: Sound,
    pub room: Room,
    pub reads: Reads,
    pub put: Put,
    /// Kept or not, in [`NAMES`]' order. All four are worked out together,
    /// so one switched off is just not kept.
    pub keep: [bool; 4],
    /// Each stem's level, dB.
    pub levels: [f32; 4],
    /// Put on Pads: the first pad, as tapped on the map; None: the first
    /// empty pads after the sound's.
    pub from: Option<usize>,
    /// The bank the map shows.
    pub bank: usize,
    /// While it splits on Put on Pads, and once it has landed: the first
    /// pad, fixed when it started ([`Stems::started`]).
    pub fixed: Option<usize>,
    pub phase: Phase,
    /// Seconds a segment takes, as the last split here went.
    pub segment_s: f32,
}

impl Stems {
    /// Opened on `sound` with `room`: reading Start to End, on pads, all
    /// four kept at 0 dB, the map on the sound's bank.
    pub fn open(sound: Sound, room: Room) -> Stems {
        let bank = sound.pad.map_or(0, |p| p / BANK);
        Stems {
            sound,
            room,
            reads: Reads::Trimmed,
            put: Put::Pads,
            keep: [true; 4],
            levels: [0.0; 4],
            from: None,
            bank,
            fixed: None,
            phase: Phase::Ready,
            segment_s: SEGMENT_S,
        }
    }

    /// How many stems are kept.
    pub fn kept(&self) -> usize {
        self.keep.iter().filter(|k| **k).count()
    }

    /// The kept stems, in order.
    pub fn kept_stems(&self) -> Vec<usize> {
        (0..4).filter(|&k| self.keep[k]).collect()
    }

    fn pads(&self) -> usize {
        self.room.filled.len()
    }

    fn used(&self, pad: usize) -> bool {
        self.room.filled.get(pad).copied().unwrap_or(false)
    }

    /// Put on Pads: the pads the kept stems land on, in order. From a pad
    /// tapped, that pad and on (pads with a sound replaced); else the first
    /// run of empty pads after the sound's, then from A1. None when they
    /// don't fit.
    pub fn landing(&self) -> Option<Vec<usize>> {
        let n = self.kept();
        let pads = self.pads();
        if n == 0 || n > pads {
            return None;
        }
        if let Some(a) = self.fixed.or(self.from) {
            return (a + n <= pads).then(|| (a..a + n).collect());
        }
        let after = self.sound.pad.map_or(0, |p| p + 1);
        let fits = |a: usize| a + n <= pads && (a..a + n).all(|p| !self.used(p));
        (after..pads)
            .chain(0..after)
            .find(|&a| fits(a))
            .map(|a| (a..a + n).collect())
    }

    /// The pads with a sound that Put on Pads would replace.
    pub fn replaced(&self) -> Vec<usize> {
        match self.put {
            Put::Pads => self
                .landing()
                .unwrap_or_default()
                .into_iter()
                .filter(|&p| self.used(p))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Why Put on `p` can't be picked now, if it can't.
    pub fn put_off(&self, p: Put) -> Option<String> {
        let n = self.kept();
        match p {
            Put::Pads => None,
            Put::Layers if self.sound.pad.is_none() => {
                Some("A take has no pad to layer: put it on a pad first".into())
            }
            Put::Layers => None,
            Put::Takes if self.room.takes + n > self.room.max_takes => Some(format!(
                "The page keeps {} takes and has {}: delete some first",
                self.room.max_takes, self.room.takes
            )),
            Put::Takes => None,
            Put::NewKit => self.room.new_kit_off.clone(),
            Put::Library if !self.room.library => Some("The library can't be written now".into()),
            Put::Library => None,
        }
    }

    /// Why no split can start as the page is set, if none can.
    pub fn cant_start(&self) -> Option<String> {
        if let Some(why) = &self.room.blocked {
            return Some(why.clone());
        }
        if self.kept() == 0 {
            return Some("Every stem is switched off: tap one to keep it".into());
        }
        if self.sound.samples(self.reads) == 0 {
            return Some("Start and End are together: nothing to split".into());
        }
        if let Some(why) = self.put_off(self.put) {
            return Some(why);
        }
        if self.put == Put::Pads && self.landing().is_none() {
            return Some(format!(
                "The stems don't fit on the pads from {}: tap another pad",
                pad_label(self.from.unwrap_or(0))
            ));
        }
        if self.room.holds > self.room.free {
            return Some(format!(
                "Needs {}, {} free",
                bytes_text(self.room.holds),
                bytes_text(self.room.free)
            ));
        }
        None
    }

    /// Pads `p` as "A6–A9", "A6".
    fn range_text(p: &[usize]) -> String {
        match (p.first(), p.last()) {
            (Some(a), Some(b)) if a != b => format!("{}–{}", pad_label(*a), pad_label(*b)),
            (Some(a), _) => pad_label(*a),
            _ => String::new(),
        }
    }

    /// Where they land, as the main button and the landed line say it:
    /// "A6–A9", "A5's layers", "4 takes", "new kit", "4 stems".
    fn where_text(&self) -> String {
        let n = self.kept();
        let s = if n == 1 { "" } else { "s" };
        match self.put {
            Put::Pads => self
                .landing()
                .map_or_else(|| "pads".into(), |p| Stems::range_text(&p)),
            Put::Layers => format!(
                "{}'s layers",
                self.sound.pad.map(pad_label).unwrap_or_default()
            ),
            Put::Takes => format!("{n} take{s}"),
            Put::NewKit => "new kit".into(),
            Put::Library => format!("{n} stem{s}"),
        }
    }

    /// The main button: what it does, or Stop while it splits.
    pub fn go_text(&self) -> String {
        match &self.phase {
            Phase::Splitting { .. } => "Stop".into(),
            Phase::Landed(on) => on.clone(),
            Phase::Ready => match self.put {
                Put::Pads | Put::Takes => format!("Split to {}", self.where_text()),
                Put::Layers => format!("Split into {}", self.where_text()),
                Put::NewKit => "Split to new kit".into(),
                Put::Library => format!("Save {}", self.where_text()),
            },
        }
    }

    /// What it says once landed: "On A6–A9", "In A5's layers", "4 takes on
    /// the page", "On a new kit", "In the library".
    pub fn landed_text(&self) -> String {
        match self.put {
            Put::Pads => format!("On {}", self.where_text()),
            Put::Layers => format!("In {}", self.where_text()),
            Put::Takes => format!("{} on the page", self.where_text()),
            Put::NewKit => "On a new kit".into(),
            Put::Library => "In the library".into(),
        }
    }

    /// Put on's note: which pads it would replace, or why it can't go.
    pub fn put_note(&self) -> (String, bool) {
        match &self.phase {
            Phase::Splitting { .. } => {
                let on = self.landed_text();
                let mut c = on.chars();
                let first = c.next().map(|f| f.to_lowercase().to_string());
                let on = first.unwrap_or_default() + c.as_str();
                return (format!("lands {on}"), false);
            }
            Phase::Landed(_) => return ("UNDO takes them back".into(), false),
            Phase::Ready => {}
        }
        if let Some(why) = self.cant_start() {
            return (why, true);
        }
        let note = match self.put {
            Put::Pads => match self.replaced().as_slice() {
                [] => "no pad replaced".to_string(),
                r => format!(
                    "replaces {}",
                    r.iter()
                        .map(|p| pad_label(*p))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            },
            Put::Layers => "in place of its sound".into(),
            Put::Takes => format!("{} of {} takes", self.room.takes, self.room.max_takes),
            Put::NewKit => "on a new track".into(),
            Put::Library => "Samples › Stems".into(),
        };
        (note, false)
    }

    /// The segments the split works through.
    pub fn segments(&self) -> usize {
        segments(self.sound.samples(self.reads))
    }

    /// About how long the split takes, in seconds.
    pub fn takes_s(&self) -> f32 {
        self.segments() as f32 * self.segment_s
    }

    /// The seconds left while it splits: from how fast it has gone, once
    /// it has gone a little way.
    pub fn left_s(&self) -> Option<f32> {
        match self.phase {
            Phase::Splitting { progress, ran_s } if progress > 0.02 => {
                Some(ran_s * (1.0 - progress) / progress)
            }
            Phase::Splitting { progress, .. } => Some(self.takes_s() * (1.0 - progress)),
            _ => None,
        }
    }

    /// The bytes the kept stems take, as they are held.
    pub fn makes(&self) -> u64 {
        let per = if self.sound.sixteen { 2 } else { 4 };
        self.kept() as u64 * self.sound.samples(self.reads) as u64 * 2 * per
    }

    /// Started: Ready → Splitting. Where they land is fixed now: on Put on
    /// Pads, from the first pad shown, whatever lands on the pads
    /// meanwhile.
    pub fn started(&mut self) {
        self.fixed = None;
        if self.put == Put::Pads {
            self.fixed = self.landing().map(|p| p[0]);
        }
        self.phase = Phase::Splitting {
            progress: 0.0,
            ran_s: 0.0,
        };
    }

    /// What a finished split lands as, made from its stems (`made`, as
    /// [`mpfree_sampler::stems::Split`] made them) and the sound's pad as
    /// the kit has it now (for Layers): each kept stem a sound named after
    /// the sound ("Soul loop Vocals"), at its level.
    pub fn land(&self, made: Made, pad: Option<&Pad>) -> Result<Landing, String> {
        let mut sounds = Vec::new();
        let mut layers = Vec::new();
        for (k, stem) in made.stems.into_iter().enumerate() {
            let Some(data) = stem.filter(|_| self.keep[k]) else {
                continue;
            };
            let name = format!("{} {}", self.sound.name, NAMES[k]);
            let source = Source::new(&name, RATE, 2, data).map_err(|e| e.to_string())?;
            let mut layer = Layer::new(Region::whole(&source));
            layer.level_db = self.levels[k];
            layers.push((name, layer));
            sounds.push(source);
        }
        if sounds.is_empty() {
            return Err("No stem was kept".into());
        }
        let pads = || -> Vec<Pad> {
            layers
                .iter()
                .map(|(name, l)| Pad::new(name, l.clone()))
                .collect()
        };
        let kit = |to| {
            Ok(Landing::Kit(Change::Stems {
                sounds: sounds.iter().cloned().map(Recorded).collect(),
                to,
            }))
        };
        match self.put {
            Put::Pads => {
                let at = self
                    .landing()
                    .ok_or("The stems don't fit on the pads from there")?;
                kit(StemsTo::Pads(at.into_iter().zip(pads()).collect()))
            }
            Put::Layers => {
                let (Some(at), Some(pad)) = (self.sound.pad, pad) else {
                    return Err("The sound's pad has gone: its layers can't take them".into());
                };
                let mut to = pad.clone();
                to.layers = layers.iter().map(|(_, l)| l.clone()).collect();
                to.select = LayerSelect::All;
                kit(StemsTo::Layers(at, Box::new(to)))
            }
            Put::Takes => kit(StemsTo::Takes(pads())),
            Put::NewKit => Ok(Landing::NewKit {
                pads: pads(),
                sounds,
            }),
            Put::Library => Ok(Landing::Library(sounds)),
        }
    }

    /// Landed after `took_s` seconds: the main button says where, and the
    /// next estimate goes by this split.
    pub fn landed(&mut self, took_s: f32) {
        let segs = self.segments().max(1) as f32;
        if took_s > 0.0 {
            self.segment_s = took_s / segs;
        }
        self.phase = Phase::Landed(self.landed_text());
    }

    /// Stopped, or it failed: Splitting → Ready.
    pub fn stopped(&mut self) {
        self.fixed = None;
        self.phase = Phase::Ready;
    }

    /// Something changed after it landed: the main button goes again.
    fn changed(&mut self) {
        if matches!(self.phase, Phase::Landed(_)) {
            self.phase = Phase::Ready;
            self.fixed = None;
        }
    }

    fn splitting(&self) -> bool {
        matches!(self.phase, Phase::Splitting { .. })
    }

    fn has_stems(&self) -> bool {
        matches!(self.phase, Phase::Landed(_))
    }

    /// Stem `k` switched: kept or not (not while it splits).
    pub fn toggle(&mut self, k: usize) -> Ask {
        if self.splitting() {
            return Ask::Say("Splitting: the stems kept are set when it starts".into());
        }
        if let Some(keep) = self.keep.get_mut(k) {
            *keep = !*keep;
            self.changed();
        }
        Ask::Nothing
    }

    /// Stem `k`'s level by `steps` of [`LEVEL_STEP_DB`].
    pub fn level(&mut self, k: usize, steps: i32) -> Ask {
        let Some(l) = self.levels.get_mut(k) else {
            return Ask::Nothing;
        };
        let v = (*l + steps as f32 * LEVEL_STEP_DB).clamp(LEVEL_DB.0, LEVEL_DB.1);
        *l = (v / LEVEL_STEP_DB).round() * LEVEL_STEP_DB;
        Ask::Level(k)
    }

    /// A pad of the map tapped (its place in the bank shown): Put on Pads
    /// from there.
    pub fn tap_pad(&mut self, col: usize) -> Ask {
        if self.splitting() {
            return Ask::Say("Splitting: where they land is set".into());
        }
        let pad = self.bank * BANK + col;
        if pad >= self.pads() {
            return Ask::Nothing;
        }
        self.put = Put::Pads;
        self.from = Some(pad);
        self.changed();
        Ask::Nothing
    }

    /// A step or a press on row `row`.
    pub fn step(&mut self, row: Row, delta: i32) -> Ask {
        if self.splitting() && matches!(row, Row::Reads | Row::Map | Row::Put) {
            return Ask::Say("Splitting: Stop it to change this".into());
        }
        match row {
            Row::Reads => {
                let at = (self.reads.index() as i32 + delta).clamp(0, 1);
                self.reads = if at == 0 {
                    Reads::Trimmed
                } else {
                    Reads::Whole
                };
                self.changed();
            }
            Row::Map => {
                // - / + step the first pad.
                let first = self.landing().map_or(0, |p| p[0]) as i64;
                let last = self.pads().saturating_sub(self.kept().max(1)) as i64;
                self.from = Some((first + i64::from(delta)).clamp(0, last.max(0)) as usize);
                self.put = Put::Pads;
                self.changed();
            }
            Row::Bank => {
                let banks = self.pads().div_ceil(BANK).max(1) as i64;
                self.bank = (self.bank as i64 + i64::from(delta)).rem_euclid(banks) as usize;
            }
            Row::Put => {
                let at = (self.put.index() as i32 + delta).clamp(0, Put::ALL.len() as i32 - 1);
                let p = Put::ALL[at as usize];
                if let Some(why) = self.put_off(p) {
                    return Ask::Say(why);
                }
                self.put = p;
                self.changed();
            }
            Row::Go => {
                return match &self.phase {
                    Phase::Splitting { .. } => Ask::Stop,
                    Phase::Landed(_) => Ask::Say(format!(
                        "{}: change something to split again",
                        self.landed_text()
                    )),
                    Phase::Ready => match self.cant_start() {
                        Some(why) => Ask::Say(why),
                        None => Ask::Split,
                    },
                }
            }
            Row::Level(k) => return self.level(k, delta),
            Row::PlayAll if self.has_stems() => return Ask::PlayAll,
            Row::PlayAll => return Ask::Say("The stems play here once split".into()),
            Row::Takes | Row::Holds | Row::Makes => {}
        }
        Ask::Nothing
    }

    /// Q-Link `k` turned `ticks`: stem `k`'s level.
    pub fn qlink(&mut self, k: usize, ticks: i32) -> Ask {
        self.level(k, ticks)
    }

    /// The hint line.
    pub fn hint(&self) -> String {
        match &self.phase {
            Phase::Splitting { .. } => "Stop: stop the split, nothing lands · other pages: it goes on, \
                                        the pad bar says how far · STOP stops the music, not the split"
                .into(),
            Phase::Landed(_) => "Tap a stem: hear it · Play all four: together, at their levels · \
                                 Q-Links 1-4: each stem's level · UNDO takes them back · Stems: back to Wave"
                .into(),
            Phase::Ready => match self.cant_start() {
                Some(why) => why,
                None => format!(
                    "{}: work out the stems · a pad on the map: start there · tap a stem: keep it or \
                     not · Q-Links 1-4: each stem's level · Stems: back to Wave",
                    self.go_text()
                ),
            },
        }
    }

    /// A stem's lane says where it is: "kept", "not kept"; while it
    /// splits "2 of 3 done", "working…" (its network runs now); once
    /// landed, where.
    pub fn lane_text(&self, k: usize) -> String {
        if !self.keep[k] {
            return "not kept".into();
        }
        match self.phase {
            Phase::Ready => "kept".into(),
            Phase::Landed(_) => match self.put {
                Put::Pads => self
                    .landing()
                    .and_then(|p| {
                        let i = self.kept_stems().iter().position(|s| *s == k)?;
                        p.get(i).map(|p| format!("on {}", pad_label(*p)))
                    })
                    .unwrap_or_else(|| "kept".into()),
                Put::Layers => format!(
                    "layer {}",
                    self.kept_stems().iter().position(|s| *s == k).unwrap_or(0) + 1
                ),
                Put::Takes => "a take".into(),
                Put::NewKit => "new kit".into(),
                Put::Library => "saved".into(),
            },
            Phase::Splitting { progress, .. } => {
                let segs = self.segments().max(1);
                let steps = segs * 6;
                let done = ((progress * steps as f32).round() as usize).min(steps);
                let (seg, at) = (done / 6, done % 6);
                if seg < segs && at == k + 1 {
                    "working…".into()
                } else {
                    format!("{seg} of {segs} done")
                }
            }
        }
    }
}

/// "176 MB", "1.1 GB".
pub fn bytes_text(b: u64) -> String {
    let mb = b as f64 / 1e6;
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else {
        format!("{mb:.0} MB")
    }
}

/// "0:24.6".
fn length_text(s: f32) -> String {
    let m = (s / 60.0).floor();
    format!("{}:{:04.1}", m as u32, s - m * 60.0)
}

/// "about 31 s", "about 1:05".
fn about_text(s: f32) -> String {
    let s = s.round().max(1.0) as u32;
    if s >= 60 {
        format!("about {}:{:02}", s / 60, s % 60)
    } else {
        format!("about {s} s")
    }
}

// ---------------------------------------------------------------- picture

/// The page's picture: the title line, the sound, and four lanes.
pub struct StemsView {
    /// "Soul loop, Start to End · 0:24.6 · four new sounds that add up to
    /// it".
    pub title: String,
    /// At the right: "about 31 s", "42 % · about 18 s left".
    pub right: String,
    /// The progress line while it splits, 0 to 1.
    pub progress: Option<f32>,
    pub name: String,
    /// The sound's columns.
    pub sound: Vec<(f32, f32)>,
    /// Where each segment starts, 0 to 1 along the sound.
    pub segments: Vec<f32>,
    /// Each stem's columns so far, and how many make the whole sound.
    pub lanes: [Vec<(f32, f32)>; 4],
    pub columns: usize,
    pub keep: [bool; 4],
    /// Under each stem's name: "kept", "2 of 3 done".
    pub states: [String; 4],
    /// Each lane says this while it has nothing: "after the split".
    pub empty: String,
    /// The switches can't change now (while it splits).
    pub locked: bool,
    painted: Cell<Option<u64>>,
}

impl Default for StemsView {
    fn default() -> StemsView {
        StemsView {
            title: String::new(),
            right: String::new(),
            progress: None,
            name: String::new(),
            sound: Vec::new(),
            segments: Vec::new(),
            lanes: Default::default(),
            columns: 1,
            keep: [true; 4],
            states: Default::default(),
            empty: "after the split".into(),
            locked: false,
            painted: Cell::new(None),
        }
    }
}

/// The sound's row's share of the box, the least it has, and between the
/// lanes.
const SOUND_SHARE: f32 = 0.26;
const SOUND_MIN_H: i32 = 60;
const LANE_GAP: i32 = 6;
/// The names' column on the left.
const NAME_W: i32 = 140;
const SWITCH_W: i32 = 40;
const SWITCH_H: i32 = 22;

impl StemsView {
    /// The areas: the title line, the progress line, the box under them.
    fn areas(r: Rect) -> (Rect, Rect, Rect) {
        let (title, rest) = r.take_top(34);
        let (line, body) = rest.take_top(6);
        (title.inset_xy(4, 0), line.inset_xy(4, 1), body)
    }

    /// The sound's row and the four lanes, each its name card and its
    /// wave.
    fn rows(r: Rect) -> (Rect, [(Rect, Rect); 4]) {
        let (_, _, body) = StemsView::areas(r);
        let inner = body.inset(10);
        let sound_h = ((inner.h as f32 * SOUND_SHARE) as i32).max(SOUND_MIN_H);
        let (sound, mut rest) = inner.take_top(sound_h);
        let lane_h = ((rest.h - 3 * LANE_GAP) / 4).max(1);
        rest = rest.take_bottom(4 * lane_h + 3 * LANE_GAP).0;
        let lanes = [0; 4].map(|_| {
            let (lane, after) = rest.take_top(lane_h + LANE_GAP);
            rest = after;
            let lane = lane.take_top(lane_h).0;
            let (name, wave) = lane.take_left(NAME_W);
            (name, wave.take_right(wave.w - 8).0)
        });
        (sound, lanes)
    }

    /// Stem `k`'s switch.
    fn switch(name: Rect) -> Rect {
        Rect::new(
            name.right() - SWITCH_W - 10,
            name.y + (name.h - SWITCH_H) / 2,
            SWITCH_W,
            SWITCH_H,
        )
    }

    /// What the picture depends on, as one number: the app redraws it only
    /// when it changes.
    pub fn look(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        (
            &self.title,
            &self.right,
            &self.states,
            self.keep,
            self.locked,
        )
            .hash(&mut h);
        self.progress.map(f32::to_bits).hash(&mut h);
        self.lanes.iter().map(Vec::len).for_each(|n| n.hash(&mut h));
        (self.sound.len(), self.columns).hash(&mut h);
        h.finish()
    }

    pub fn painted(&self) -> Option<u64> {
        self.painted.get()
    }

    fn wave(cx: &mut PaintCx, r: Rect, cols: &[(f32, f32)], of: usize, color: mpfree_ui::Color) {
        let mid = r.y + r.h / 2;
        let half = (r.h / 2 - 3).max(1) as f32;
        let of = of.max(cols.len()).max(1) as i32;
        for (i, &(lo, hi)) in cols.iter().enumerate() {
            let x0 = r.x + r.w * i as i32 / of;
            let x1 = (r.x + r.w * (i as i32 + 1) / of).max(x0 + 1);
            let up = (hi.clamp(0.0, 1.0) * half) as i32;
            let down = (-lo.clamp(-1.0, 0.0) * half) as i32;
            cx.canvas
                .fill_rect(Rect::new(x0, mid - up, x1 - x0, up + down + 1), color);
        }
    }
}

impl Widget for StemsView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        self.painted.set(Some(self.look()));
        cx.canvas.fill_rect(r, t.bg);
        let (title, line, body) = StemsView::areas(r);
        let small = t.size_small;
        let w = cx.text(FontId::Bold, small, title, Align::Left, t.accent, "STEMS");
        let rw = cx.measure(FontId::Sans, small, &self.right) + 8;
        let mid = Rect::new(title.x + w + 14, title.y, title.w - w - 14 - rw, title.h);
        cx.text(FontId::Sans, small, mid, Align::Left, t.text, &self.title);
        cx.text(
            FontId::Sans,
            small,
            title,
            Align::Right,
            t.text_dim,
            &self.right,
        );
        if let Some(p) = self.progress {
            cx.canvas.fill_round_rect(line, 2, t.raised);
            let done = (line.w as f32 * p.clamp(0.0, 1.0)).round() as i32;
            cx.canvas
                .fill_round_rect(Rect::new(line.x, line.y, done, line.h), 2, t.accent);
        }
        cx.canvas.fill_round_rect(body, t.round(12), t.surface);
        let (sound, lanes) = StemsView::rows(r);
        // The sound: its name, its wave, the segments along the top.
        let (name, wave) = sound.take_left(NAME_W);
        let wave = wave.take_right(wave.w - 8).0;
        let (n1, n2) = name.inset_xy(12, 18).take_top(26);
        cx.text(
            FontId::Bold,
            t.size_body,
            n1,
            Align::Left,
            t.text,
            &self.name,
        );
        cx.text(
            FontId::Sans,
            small - 2.0,
            n2.take_top(20).0,
            Align::Left,
            t.text_dim,
            "the sound",
        );
        let (marks, sw) = wave.take_top(20);
        StemsView::wave(cx, sw, &self.sound, self.sound.len(), t.accent);
        cx.canvas
            .fill_rect(Rect::new(sw.x, sw.y + sw.h / 2, sw.w, 1), t.outline);
        let lanes_bottom = lanes[3].1.bottom();
        for (i, &at) in self.segments.iter().enumerate() {
            let x = wave.x + (at * wave.w as f32).round() as i32;
            if i > 0 {
                cx.canvas
                    .fill_rect(Rect::new(x, marks.y, 1, lanes_bottom - marks.y), t.outline);
            }
            let label = if i == 0 || at < 0.85 {
                format!("segment {}", i + 1)
            } else {
                (i + 1).to_string()
            };
            cx.text(
                FontId::Sans,
                small - 4.0,
                Rect::new(
                    x + 6,
                    marks.y,
                    120.min(wave.right() - x - 6).max(0),
                    marks.h,
                ),
                Align::Left,
                t.text_dim,
                &label,
            );
        }
        // The lanes.
        for (k, (name, wave)) in lanes.iter().enumerate() {
            let tint = t.track(TINTS[k]);
            let kept = self.keep[k];
            cx.canvas.fill_round_rect(*name, t.round(8), t.raised);
            cx.canvas.fill_round_rect(
                Rect::new(name.x + 6, name.y + 8, 3, name.h - 16),
                1,
                if kept { tint } else { t.text_faint },
            );
            let text = Rect::new(name.x + 16, name.y + 4, name.w - SWITCH_W - 30, name.h - 8);
            let (top, under) = text.take_top(text.h / 2 + 3);
            cx.text(
                FontId::Bold,
                t.size_body - 1.0,
                top,
                Align::Left,
                if kept { t.text } else { t.text_dim },
                NAMES[k],
            );
            cx.text(
                FontId::Sans,
                small - 3.0,
                under,
                Align::Left,
                t.text_dim,
                &self.states[k],
            );
            let sw = StemsView::switch(*name);
            let on = kept && !self.locked;
            cx.canvas.fill_round_rect(
                sw,
                SWITCH_H / 2,
                if on {
                    tint
                } else if kept {
                    tint.mix(t.raised, 0.5)
                } else {
                    t.bg
                },
            );
            let kx = if kept {
                sw.right() - SWITCH_H + 2
            } else {
                sw.x + 2
            };
            cx.canvas.fill_round_rect(
                Rect::new(kx, sw.y + 2, SWITCH_H - 4, SWITCH_H - 4),
                (SWITCH_H - 4) / 2,
                t.role(mpfree_ui::Role::Handle),
            );
            cx.canvas
                .fill_round_rect(*wave, t.round(6), t.bg.mix(t.surface, 0.5));
            if self.lanes[k].is_empty() {
                cx.text(
                    FontId::Sans,
                    small - 3.0,
                    *wave,
                    Align::Center,
                    t.text_faint,
                    &self.empty,
                );
            } else {
                let color = if kept { tint } else { tint.mix(t.surface, 0.6) };
                // The rest of the lane, still to come: its middle line.
                cx.canvas.fill_rect(
                    Rect::new(wave.x, wave.y + wave.h / 2, wave.w, 1),
                    color.mix(t.surface, 0.5),
                );
                StemsView::wave(cx, *wave, &self.lanes[k], self.columns, color);
            }
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let (_, lanes) = StemsView::rows(r);
        for (k, (name, wave)) in lanes.iter().enumerate() {
            if name.contains(x, y) {
                return Some(EventKind::Toggle(k));
            }
            if wave.contains(x, y) {
                return Some(EventKind::Select(k));
            }
        }
        None
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let (_, lanes) = StemsView::rows(r);
        let mut out = Vec::new();
        for (k, (name, wave)) in lanes.iter().enumerate() {
            out.push(
                Target::tap("stem-switch", k, format!("{} kept", NAMES[k]), *name)
                    .disabled_if(self.locked),
            );
            out.push(
                Target::tap("stem-lane", k, format!("Hear {}", NAMES[k]), *wave)
                    .disabled_if(self.lanes[k].is_empty()),
            );
        }
        out
    }
}

// ---------------------------------------------------------------- page

/// The page's widgets and what they show.
pub struct StemsPage {
    pub stems: Stems,
}

/// Put on's pad map, as the Slice tool's.
const MAP_W: i32 = 210;
const BANK_H: i32 = 30;
/// The main button's height in Put on.
const GO_H: i32 = 44;

impl StemsPage {
    /// Adds the widgets and the page to `ui` once (the app adds `header`,
    /// `hint`, `smp-bar` and `smp-send` first). Nothing shows until
    /// [`StemsPage::open`].
    pub fn add(ui: &mut Ui) -> Result<(), String> {
        if ui.get::<StemsView>(VIEW).is_some() {
            return Ok(());
        }
        ui.add(VIEW, StemsView::default());
        let mut list = SettingList::new("Stems");
        list.selected = Row::Go.index();
        ui.add(LIST, list);
        ui.add(QLINKS, QLinkColumn::new());
        ui.add_page(PAGE, LAYOUT)
    }

    /// Opened on `sound` with `room` ([`Stems::open`]).
    pub fn open(ui: &mut Ui, sound: Sound, room: Room) -> StemsPage {
        let mut p = StemsPage {
            stems: Stems::open(sound, room),
        };
        p.show(ui);
        p
    }

    /// While it splits: how far, the seconds it has run and the stems'
    /// picture so far ([`mpfree_sampler::stems::Split::peaks`]).
    pub fn splitting(&mut self, ui: &mut Ui, progress: f32, ran_s: f32, peaks: &Peaks) {
        self.stems.phase = Phase::Splitting { progress, ran_s };
        self.set_lanes(ui, peaks);
        self.show(ui);
    }

    /// The stems' pictures ([`Peaks`]).
    pub fn set_lanes(&mut self, ui: &mut Ui, peaks: &Peaks) {
        if let Some(v) = ui.get_mut::<StemsView>(VIEW) {
            v.lanes = peaks.stems.clone();
            v.columns = peaks.columns.max(1);
        }
    }

    /// Shows the page as [`StemsPage::stems`] has it.
    pub fn show(&mut self, ui: &mut Ui) {
        let s = &self.stems;
        let reads = match s.reads {
            Reads::Trimmed => "Start to End",
            Reads::Whole => "whole",
        };
        let secs = s.sound.seconds(s.reads);
        let doing = match &s.phase {
            Phase::Splitting { progress, .. } => {
                let segs = s.segments().max(1);
                let seg = ((progress * segs as f32).floor() as usize + 1).min(segs);
                format!("splitting segment {seg} of {segs}")
            }
            Phase::Landed(_) => "the four add up to it".into(),
            Phase::Ready => "four new sounds that add up to it".into(),
        };
        let title = format!(
            "{}, {reads} · {} · {doing}",
            s.sound.name,
            length_text(secs)
        );
        let right = match (&s.phase, s.left_s()) {
            (Phase::Splitting { progress, .. }, Some(left)) => {
                format!("{:.0} % · {} left", progress * 100.0, about_text(left))
            }
            (Phase::Landed(_), _) => "done".into(),
            _ => about_text(s.takes_s()),
        };
        let (a, b) = s.sound.span(s.reads);
        let peaks = &s.sound.peaks;
        let span = |f: u64| (f as f64 / s.sound.frames.max(1) as f64 * peaks.len() as f64) as usize;
        let sound: Vec<(f32, f32)> = peaks
            .get(span(a).min(peaks.len())..span(b).min(peaks.len()).max(span(a).min(peaks.len())))
            .unwrap_or_default()
            .to_vec();
        let samples = s.sound.samples(s.reads).max(1);
        let segments: Vec<f32> = (0..s.segments())
            .map(|k| (k * SEGMENT) as f32 / samples as f32)
            .filter(|at| *at < 1.0)
            .collect();
        let states = [0, 1, 2, 3].map(|k| s.lane_text(k));
        let splitting = s.splitting();
        let ready = s.phase == Phase::Ready;
        if let Some(v) = ui.get_mut::<StemsView>(VIEW) {
            v.title = title;
            v.right = right;
            v.progress = match s.phase {
                Phase::Splitting { progress, .. } => Some(progress),
                _ => None,
            };
            v.name = s.sound.name.clone();
            v.sound = sound;
            v.segments = segments;
            v.keep = s.keep;
            v.states = states;
            v.locked = splitting;
            if ready {
                v.lanes = Default::default();
            }
        }
        let (rows, controls, panel) = self.panel(ui);
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.rows = rows;
            list.controls = controls;
            list.panel = Some(panel);
            if !Row::ALL.get(list.selected).is_some_and(|r| r.picks()) {
                list.selected = Row::Go.index();
            }
        }
        let t = ui.theme.clone();
        if let Some(q) = ui.get_mut::<QLinkColumn>(QLINKS) {
            q.title = "STEMS".into();
            q.cards = [0, 1, 2, 3].map(|k| {
                let mut c = QLinkCard::new(
                    NAMES[k],
                    "STEMS",
                    &format!("{:.1} dB", self.stems.levels[k]),
                    level_pos(self.stems.levels[k]),
                );
                c.color = Some(t.track(TINTS[k]));
                c
            });
        }
    }

    /// The rows, their controls and the panel.
    fn panel(&self, ui: &Ui) -> (Vec<Setting>, Vec<Control>, Panel) {
        let s = &self.stems;
        let t = &ui.theme;
        let busy = s.splitting();
        let n = s.kept();
        let row = |name: &str, value: String| Setting {
            name: name.into(),
            value,
        };
        let bank = (b'A' + (s.bank % 26) as u8) as char;
        let rows: Vec<Setting> = Row::ALL
            .iter()
            .map(|r| match r {
                Row::Reads => row("Reads", Reads::CHOICES[s.reads.index()].into()),
                Row::Takes => row(
                    "Takes",
                    format!("{} (2 cores, full band)", about_text(s.takes_s())),
                ),
                Row::Holds => row(
                    "Holds",
                    format!(
                        "{} while it runs · {} free",
                        bytes_text(s.room.holds),
                        bytes_text(s.room.free)
                    ),
                ),
                Row::Makes => row(
                    "Makes",
                    format!(
                        "{n} sound{} · {}",
                        if n == 1 { "" } else { "s" },
                        bytes_text(s.makes())
                    ),
                ),
                Row::Map => row(
                    "Pads",
                    s.landing()
                        .map_or_else(|| "None".into(), |p| Stems::range_text(&p)),
                ),
                Row::Bank => row("Map", format!("{bank}1–{bank}16")),
                Row::Put => row("Put on", s.go_text()),
                Row::Go => row("Split", s.go_text()),
                Row::Level(k) => row(NAMES[*k], format!("{:.1} dB", s.levels[*k])),
                Row::PlayAll => row("Play all four", "Play all four".into()),
            })
            .collect();
        // The map: each kept stem's name on the pad it lands on.
        let landing = (s.put == Put::Pads).then(|| s.landing()).flatten();
        let first = s.bank * BANK;
        let kept = s.kept_stems();
        let cells: Vec<PadCell> = (first..first + BANK)
            .map(|pad| {
                let at = landing
                    .as_ref()
                    .and_then(|p| p.iter().position(|q| *q == pad));
                let layers = s.put == Put::Layers && s.sound.pad == Some(pad);
                match at.and_then(|i| kept.get(i)) {
                    Some(&k) => PadCell {
                        text: SHORT[k].into(),
                        tint: Some(TINTS[k]),
                        used: s.used(pad),
                        picked: s.from == Some(pad),
                    },
                    // Layers: the sound's own pad takes them all.
                    None if layers => PadCell {
                        text: format!("{n} layers"),
                        tint: kept.first().map(|&k| TINTS[k]),
                        used: true,
                        picked: true,
                    },
                    None => PadCell {
                        used: s.used(pad),
                        ..PadCell::default()
                    },
                }
            })
            .collect();
        let off: Vec<usize> = Put::ALL
            .iter()
            .enumerate()
            .filter(|(_, p)| s.put_off(**p).is_some())
            .map(|(i, _)| i)
            .collect();
        let put_names = vec![
            format!("{n} pads"),
            "Layers".into(),
            "Takes".into(),
            "New kit".into(),
            "Library".into(),
        ];
        let cant = s.cant_start();
        let go = match &s.phase {
            Phase::Splitting { .. } => Control::new(Look::Button("Stop".into())),
            Phase::Landed(on) => Control::new(Look::Primary(on.clone()))
                .faint_because(true, &format!("{on}: change something to split again")),
            Phase::Ready => Control::new(Look::Primary(s.go_text()))
                .faint_because(cant.is_some(), cant.as_deref().unwrap_or_default()),
        };
        let busy_why = "Splitting: Stop it to change this";
        let mut controls = Vec::new();
        for r in Row::ALL {
            controls.push(match r {
                Row::Reads => Control::new(Look::Choice {
                    options: Reads::CHOICES.iter().map(|c| c.to_string()).collect(),
                    on: s.reads.index(),
                    off: Vec::new(),
                })
                .faint_because(busy, busy_why),
                Row::Takes | Row::Holds | Row::Makes => Control::new(Look::Text).faint_if(busy),
                Row::Map => Control::new(Look::Pads(cells.clone())).faint_because(
                    busy || !matches!(s.put, Put::Pads | Put::Layers),
                    match s.put {
                        _ if busy => busy_why,
                        Put::Takes => "Takes: the stems go on the page, on no pad",
                        Put::NewKit => "On a new kit the stems start on A1",
                        _ => "Library: the stems go on no pad",
                    },
                ),
                Row::Bank => Control::new(Look::Chip),
                Row::Put => Control::new(Look::Grid {
                    options: put_names.clone(),
                    on: s.put.index(),
                    off: off.clone(),
                    cols: 2,
                })
                .faint_because(busy, busy_why),
                Row::Go => go.clone(),
                Row::Level(k) => Control::new(Look::Knob {
                    pos: level_pos(s.levels[k]),
                    steps: ((LEVEL_DB.1 - LEVEL_DB.0) / LEVEL_STEP_DB) as u32,
                    bipolar: false,
                })
                .tinted(t.track(TINTS[k])),
                Row::PlayAll => Control::new(Look::Button("Play all four".into()))
                    .faint_because(!s.has_stems(), "The stems play here once split"),
            });
        }
        let it = |r: Row| Item::new(r.index());
        let (note, warn) = s.put_note();
        let listen_note = if s.has_stems() {
            "Q1–Q4: levels"
        } else {
            "Q1–Q4: levels as they land"
        };
        let panel = Panel {
            sections: vec![
                Section::new(
                    "Split",
                    300,
                    vec![
                        vec![it(Row::Reads).labelled()],
                        vec![it(Row::Takes)],
                        vec![it(Row::Holds)],
                        vec![it(Row::Makes)],
                    ],
                )
                .note(&format!("{n} of 4 kept"))
                .from_top(),
                Section::new(
                    "Put on",
                    470,
                    vec![vec![
                        it(Row::Map).wide(MAP_W),
                        it(Row::Bank).under().high(BANK_H),
                        it(Row::Put),
                        it(Row::Go).under().high(GO_H),
                    ]],
                )
                .note(&note)
                .warn(warn),
                Section::new(
                    "Listen",
                    326,
                    vec![
                        (0..4).map(|k| it(Row::Level(k))).collect(),
                        vec![it(Row::PlayAll)],
                    ],
                )
                .note(listen_note),
            ],
        };
        (rows, controls, panel)
    }

    /// A step or a press on the panel's row `row`.
    pub fn step(&mut self, ui: &mut Ui, row: usize, delta: i32) -> Ask {
        let Some(&r) = Row::ALL.get(row) else {
            return Ask::Nothing;
        };
        if r.picks() {
            if let Some(list) = ui.get_mut::<SettingList>(LIST) {
                list.selected = row;
            }
        }
        let ask = self.stems.step(r, delta);
        self.show(ui);
        ask
    }

    /// The panel's event: a press, a step, a pad of the map tapped.
    pub fn event(&mut self, ui: &mut Ui, ev: &EventKind) -> Ask {
        let faint = |ui: &Ui, k: usize| {
            ui.get::<SettingList>(LIST)
                .and_then(|l| l.controls.get(k))
                .filter(|c| c.faint && !c.why.is_empty())
                .map(|c| c.why.clone())
        };
        match *ev {
            EventKind::Step { index, delta } => self.step(ui, index, delta),
            EventKind::Tab(index) => self.step(ui, index, 1),
            EventKind::Select(index) => match faint(ui, index) {
                Some(why) => Ask::Say(why),
                None => Ask::Nothing,
            },
            EventKind::Cell { col, .. } => {
                let ask = self.stems.tap_pad(col);
                self.show(ui);
                ask
            }
            _ => Ask::Nothing,
        }
    }

    /// The picture's event: a stem's switch or its lane tapped.
    pub fn view_event(&mut self, ui: &mut Ui, ev: &EventKind) -> Ask {
        let ask = match *ev {
            EventKind::Toggle(k) => self.stems.toggle(k),
            EventKind::Select(k) if self.stems.has_stems() && self.stems.keep[k] => Ask::Play(k),
            EventKind::Select(_) => Ask::Nothing,
            _ => return Ask::Nothing,
        };
        self.show(ui);
        ask
    }

    /// Q-Link `k` turned `ticks`.
    pub fn qlink(&mut self, ui: &mut Ui, k: usize, ticks: i32) -> Ask {
        let ask = self.stems.qlink(k, ticks);
        self.show(ui);
        ask
    }

    /// The wheel: picks the next control (not a line of text), or steps the
    /// one picked while `editing`.
    pub fn wheel(&mut self, ui: &mut Ui, delta: i32, editing: bool) -> Ask {
        let selected = ui.get::<SettingList>(LIST).map_or(0, |l| l.selected);
        if editing {
            return self.step(ui, selected, delta);
        }
        let picks: Vec<usize> = Row::ALL
            .iter()
            .filter(|r| r.picks())
            .map(|r| r.index())
            .collect();
        let at = picks.iter().position(|i| *i == selected).unwrap_or(0) as i32;
        let to = picks[(at + delta).clamp(0, picks.len() as i32 - 1) as usize];
        if let Some(list) = ui.get_mut::<SettingList>(LIST) {
            list.selected = to;
        }
        Ask::Nothing
    }
}

/// A level's place round its knob, 0 to 1.
fn level_pos(db: f32) -> f32 {
    (db - LEVEL_DB.0) / (LEVEL_DB.1 - LEVEL_DB.0)
}

// ---------------------------------------------------------------- preview

/// One preview screen: its file name, what it shows, and the UI.
pub type Screen = (&'static str, &'static str, Ui);

/// How a preview screen sets the page up: it says how far along the
/// stems' pictures are drawn (0 to 1).
type SetUp = fn(&mut Stems) -> f32;

/// The Stems page's screens for the workstation preview and the tests, as
/// the mockups draw them: "Soul loop" (24.6 s, three segments) on pad A5,
/// ready, splitting, landed on four pads, as layers, and with no model on
/// the stick. The stems' pictures are made up for the preview (a split
/// needs the model, which isn't in the repository): the drums are the
/// loop itself, the rest drawn as phrases, a held bass and a pad.
pub fn samples(fonts: impl Fn() -> Fonts) -> Result<Vec<Screen>, String> {
    let mut out = Vec::new();
    let screens: [(&str, &str, SetUp); 5] = [
        ("stems-ready", "Stems: ready to split onto A6–A9", |_| 0.0),
        ("stems-splitting", "Stems: splitting, 42 %", |s| {
            s.phase = Phase::Splitting {
                progress: 0.42,
                ran_s: 13.1,
            };
            // The first segment's stems are out.
            SEGMENT as f32 / s.sound.samples(s.reads) as f32
        }),
        (
            "stems-landed",
            "Stems: landed on A6–A9, one undo step",
            |s| {
                s.levels = [1.5, 0.0, -3.0, -6.0];
                s.landed(29.5);
                1.0
            },
        ),
        (
            "stems-layers",
            "Stems: as A5's layers, drums switched off",
            |s| {
                s.put = Put::Layers;
                s.keep[1] = false;
                0.0
            },
        ),
        ("stems-no-model", "Stems: no model on the stick", |s| {
            s.room.blocked =
                Some("The stems model isn't on the stick: the USB Creator adds it".into());
            0.0
        }),
    ];
    for (name, about, set) in screens {
        let (mut ui, mut page, source, _) = sample_with(fonts())?;
        let filled = set(&mut page.stems);
        page.set_lanes(&mut ui, &made_up_stems(&source, filled));
        page.show(&mut ui);
        set_hint(&mut ui, &page.stems.hint());
        out.push((name, about, ui));
    }
    Ok(out)
}

/// The Stems page open on the preview's kit ("Soul loop" on A5, sounds on
/// A1 to A4 and A13), ready to split, in the Sample page's frame, and the
/// kit.
pub fn sample(fonts: Fonts) -> Result<(Ui, StemsPage, Project), String> {
    sample_with(fonts).map(|(ui, page, _, project)| (ui, page, project))
}

fn sample_with(fonts: Fonts) -> Result<(Ui, StemsPage, Source, Project), String> {
    use crate::sample::{self, PAD};
    let (project, view) = soul_loop();
    let state = WaveState::from_project(&project, PAD, 0, view).ok_or("no pad")?;
    let mut ui = sample::app_ui(fonts, &project);
    WaveTab::add(&mut ui, sample::PAGE, &state)?;
    StemsPage::add(&mut ui)?;
    crate::send::light_stems(&mut ui, true);
    let source = project.sources().first().ok_or("no sound")?.clone();
    let page = StemsPage::open(&mut ui, preview_sound(&source), preview_room(&project));
    set_hint(&mut ui, &page.stems.hint());
    ui.show(PAGE)?;
    Ok((ui, page, source, project))
}

/// The app's hint line says `text`.
fn set_hint(ui: &mut Ui, text: &str) {
    if let Some(l) = ui.get_mut::<mpfree_ui::widgets::Label>("hint") {
        l.text = text.to_string();
    }
}

/// The preview's kit: "Soul loop", 24.6 s of the sample drums, on A5,
/// with A1 to A4 and A13 holding sounds.
fn soul_loop() -> (Project, std::sync::Arc<crate::source::SourceView>) {
    let one = crate::sample::drum_loop();
    let frames = (24.6 * f64::from(RATE)) as usize;
    let data: Vec<f32> = one.iter().copied().cycle().take(frames * 2).collect();
    let source = Source::new("Soul loop", RATE, 2, data).expect("the preview's sound is valid");
    let view = std::sync::Arc::new(crate::source::SourceView::analyse(&source));
    let mut p = Project::new();
    let hit = Source::new("Hit", RATE, 2, one[..8820].to_vec()).expect("a hit");
    p.add_source(source.clone());
    p.add_source(hit.clone());
    for pad in [0, 1, 2, 3, 12] {
        p.set_pad(
            pad,
            Some(Pad::new("Hit", Layer::new(Region::whole(&hit)))),
            None,
        )
        .expect("a pad");
    }
    p.set_pad(
        crate::sample::PAD,
        Some(Pad::new("Soul loop", Layer::new(Region::whole(&source)))),
        None,
    )
    .expect("the loop's pad");
    (p, view)
}

/// Columns across the preview's pictures.
const PREVIEW_COLUMNS: usize = 900;

fn columns(data: &[f32], channels: usize, n: usize) -> Vec<(f32, f32)> {
    let frames = data.len() / channels.max(1);
    let per = frames.div_ceil(n).max(1);
    data.chunks(per * channels)
        .map(|c| {
            let lo = c.iter().copied().fold(0.0f32, f32::min);
            let hi = c.iter().copied().fold(0.0f32, f32::max);
            (lo, hi)
        })
        .collect()
}

fn preview_sound(s: &Source) -> Sound {
    let frames = s.frames();
    Sound {
        name: s.name.clone(),
        pad: Some(crate::sample::PAD),
        rate: s.rate(),
        frames,
        trimmed: (0, frames),
        sixteen: true,
        peaks: columns(
            &s.data().to_vec(),
            usize::from(s.channels()),
            PREVIEW_COLUMNS,
        ),
    }
}

fn preview_room(p: &Project) -> Room {
    Room {
        holds: 176_000_000,
        free: 1_100_000_000,
        filled: (0..mpfree_sampler::model::PADS)
            .map(|i| p.pad(i).is_some())
            .collect(),
        takes: 0,
        max_takes: mpfree_sampler::model::MAX_TAKES,
        new_kit_off: None,
        library: true,
        blocked: None,
    }
}

/// The preview's made-up stems, `filled` of the way along.
fn made_up_stems(s: &Source, filled: f32) -> Peaks {
    let drums = columns(
        &s.data().to_vec(),
        usize::from(s.channels()),
        PREVIEW_COLUMNS,
    );
    let n = drums.len();
    let upto = (n as f32 * filled) as usize;
    let wave = |f: &dyn Fn(usize) -> f32| -> Vec<(f32, f32)> {
        (0..upto).map(|i| (-f(i), f(i))).collect()
    };
    let phrase = |i: usize| {
        let t = i as f32 / n as f32 * 24.6;
        let bar = t % 5.2;
        if (0.6..2.4).contains(&bar) || (3.0..4.6).contains(&bar) {
            0.55 * (1.0 - ((bar * 7.0).sin() * 0.25).abs())
        } else {
            0.02
        }
    };
    let bass = |i: usize| 0.35 + 0.08 * ((i as f32 * 0.21).sin());
    let other = |i: usize| 0.12 + 0.05 * ((i as f32 * 0.05).sin());
    Peaks {
        per: (s.frames() as usize).div_ceil(PREVIEW_COLUMNS),
        columns: n,
        stems: [
            wave(&phrase),
            drums[..upto]
                .iter()
                .map(|&(lo, hi)| (lo * 0.8, hi * 0.8))
                .collect(),
            wave(&bass),
            wave(&other),
        ],
    }
}
