//! The sampler's part of a saved project, as text: each kit's pads (their
//! layers, regions, loops, fades, warps, grains, envelopes, filter,
//! pressure and keys), its chop sets, its master effect and what its pads
//! play, with each sound named by the file it is kept in.
//!
//! ```text
//! mpfree-sampler 1
//! kits = 1
//! kit = 0
//! kit.0.name = Boom bap
//! kit.0.sources = 1
//! kit.0.source.0.file = 1f0c2d3e4a5b6c7d.wav
//! kit.0.source.0.name = Break 92
//! kit.0.pad.0.name = Kick
//! kit.0.pad.0.layers = 1
//! kit.0.pad.0.layer.0.source = 0
//! kit.0.pad.0.layer.0.region.start = 0
//! ...
//! ```
//!
//! - One `key = value` a line, every setting spelled out. A key this build
//!   doesn't know is skipped and a key that's missing keeps its default,
//!   so an older or newer build's text still opens.
//! - Numbers are written so they read back to the same bits: a saved kit
//!   plays the same when it's opened.
//! - What can't be used (a sound whose file is missing or broken, a pad
//!   the model refuses) is left out with a note saying why; the rest of
//!   the kit, and every other kit, opens.
//! - The files themselves are the caller's ([`crate::wav`] writes and
//!   reads them; the library keeps them in the project's `samples/`).

use std::collections::HashMap;
use std::fmt::Display;
use std::str::FromStr;

use crate::dsp::{Ahdsr, Filter, FilterKind};
use crate::model::*;

/// The first line.
pub const FORMAT: &str = "mpfree-sampler 1";

/// What [`from_text`] gives back.
#[derive(Debug, Default)]
pub struct Opened {
    /// Each kit's name and project, in order. Their undo history is empty.
    pub kits: Vec<(String, Project)>,
    /// The kit the pads were playing.
    pub kit: usize,
    /// What was left out, and why, one line each.
    pub notes: Vec<String>,
}

/// The kits as text (their sounds named by `file_of`), with `kit` the one
/// the pads play.
pub fn to_text(
    kits: &[(&str, &Project)],
    kit: usize,
    file_of: &dyn Fn(&Source) -> String,
) -> String {
    let mut w = Writer {
        out: format!("{FORMAT}\n"),
        at: String::new(),
        ids: HashMap::new(),
    };
    w.line("kits", kits.len());
    w.line("kit", kit);
    for (k, (name, p)) in kits.iter().enumerate() {
        w.at = format!("kit.{k}.");
        w.line("name", one_line(name));
        w.ids.clear();
        w.line("sources", p.sources().len());
        for (i, s) in p.sources().iter().enumerate() {
            w.ids.insert(s.id(), i);
            w.line(&format!("source.{i}.file"), one_line(&file_of(s)));
            w.line(&format!("source.{i}.name"), one_line(&s.name));
        }
        w.line("chops", p.chop_sets.len());
        for (i, c) in p.chop_sets.iter().enumerate() {
            let at = format!("chop.{i}.");
            w.line(&format!("{at}name"), one_line(&c.name));
            w.line(
                &format!("{at}source"),
                w.ids.get(&c.source).copied().unwrap_or(usize::MAX),
            );
            w.line(&format!("{at}mode"), word_of(c.mode, CHOP_MODES));
            let marks: Vec<String> = c.markers().iter().map(u64::to_string).collect();
            w.line(&format!("{at}markers"), marks.join(" "));
        }
        let mut m = *p.master();
        w.scope("master", |w| visit_master(w, &mut m));
        w.line("pads_play", pads_play_text(p.pads_play()));
        for i in 0..PADS {
            let Some(pad) = p.pad(i) else { continue };
            write_pad(&mut w, &format!("pad.{i}"), pad);
        }
        // Scenes A and B: their pads as the kit's are written.
        for (k, scene) in p.scenes.iter().enumerate() {
            let Some(scene) = scene else { continue };
            w.line(&format!("scene.{k}"), "stored");
            for i in 0..PADS {
                let Some(pad) = scene.pad(i) else { continue };
                write_pad(&mut w, &format!("scene.{k}.pad.{i}"), pad);
            }
        }
    }
    w.out
}

fn write_pad(w: &mut Writer, at: &str, pad: &Pad) {
    let mut pad = pad.clone();
    w.scope(at, |w| {
        w.line("layers", pad.layers.len());
        for (j, l) in pad.layers.iter_mut().enumerate() {
            w.scope(&format!("layer.{j}"), |w| {
                let src = w.ids.get(&l.region.source).copied().unwrap_or(usize::MAX);
                w.line("source", src);
                visit_layer(w, l);
            });
        }
        visit_pad(w, &mut pad);
    });
}

/// The kits from text [`to_text`] wrote (or an older or newer build).
/// `read` gives a sound from its file name; each file is read once, however
/// many kits use it. Fails only when the text isn't a sampler part at all.
pub fn from_text(
    text: &str,
    read: &mut dyn FnMut(&str) -> Result<Source, String>,
) -> Result<Opened, String> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    if lines.next() != Some(FORMAT) {
        return Err(format!("not an {FORMAT} part"));
    }
    let mut map = HashMap::new();
    for l in lines {
        if l.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    let mut r = Reader {
        map: &map,
        at: String::new(),
        notes: Vec::new(),
    };
    let mut files: HashMap<String, Result<Source, String>> = HashMap::new();
    let mut out = Opened::default();
    let count: usize = r.get("kits").unwrap_or(0);
    out.kit = r.get("kit").unwrap_or(0);
    // Kits come from a part of at most a few MiB: a count past what the
    // text could hold is a broken file, not a reason to loop.
    for k in 0..count.min(map.len()) {
        r.at = format!("kit.{k}.");
        let name = r.text("name").unwrap_or_default();
        let p = read_kit(&mut r, &name, &mut files, read);
        out.kits.push((name, p));
    }
    out.notes = r.notes;
    Ok(out)
}

fn read_kit(
    r: &mut Reader,
    kit: &str,
    files: &mut HashMap<String, Result<Source, String>>,
    read: &mut dyn FnMut(&str) -> Result<Source, String>,
) -> Project {
    let mut p = Project::new();
    let mut ids: Vec<Option<SourceId>> = Vec::new();
    let sources: usize = r.get("sources").unwrap_or(0);
    for i in 0..sources.min(r.map.len()) {
        let Some(file) = r.text(&format!("source.{i}.file")) else {
            ids.push(None);
            continue;
        };
        let got = files
            .entry(file.clone())
            .or_insert_with(|| read(&file))
            .clone();
        match got {
            Ok(mut s) => {
                if let Some(n) = r.text(&format!("source.{i}.name")) {
                    s.name = n;
                }
                ids.push(Some(p.add_source(s)));
            }
            Err(e) => {
                r.notes.push(format!("{kit}: sound {file} left out ({e})"));
                ids.push(None);
            }
        }
    }
    let source = |p: &Project, i: Option<usize>| -> Option<Source> {
        let id = (*ids.get(i?)?)?;
        p.source(id).cloned()
    };
    let chops: usize = r.get("chops").unwrap_or(0);
    for i in 0..chops.min(r.map.len()) {
        let at = format!("chop.{i}.");
        let name = r.text(&format!("{at}name")).unwrap_or_default();
        let Some(s) = source(&p, r.get(&format!("{at}source"))) else {
            r.notes.push(format!(
                "{kit}: chop {name:?} left out (its sound is missing)"
            ));
            continue;
        };
        let mode = r
            .text(&format!("{at}mode"))
            .and_then(|m| from_word(&m, CHOP_MODES))
            .unwrap_or(ChopMode::Manual);
        let marks: Vec<u64> = r
            .text(&format!("{at}markers"))
            .map(|m| {
                m.split_whitespace()
                    .filter_map(|x| x.parse().ok())
                    .collect()
            })
            .unwrap_or_default();
        match ChopSet::from_markers(&name, &s, mode, marks).and_then(|c| p.add_chop_set(c)) {
            Ok(_) => {}
            Err(e) => r.notes.push(format!("{kit}: chop {name:?} left out ({e})")),
        }
    }
    let mut m = *p.master();
    r.scope("master", |r| visit_master(r, &mut m));
    if let Err(e) = p.set_master(m, None) {
        r.notes
            .push(format!("{kit}: master effect left as it was ({e})"));
    }
    if let Some(t) = r.text("pads_play") {
        match pads_play_from(&t) {
            Some(pp) => {
                if let Err(e) = p.set_pads_play(pp) {
                    r.notes
                        .push(format!("{kit}: pads play their own sounds ({e})"));
                }
            }
            None => r
                .notes
                .push(format!("{kit}: pads play their own sounds ({t:?} unknown)")),
        }
    }
    for i in 0..PADS {
        let Some((pad, missing)) = read_pad(r, &format!("pad.{i}"), &|n| source(&p, n)) else {
            continue;
        };
        let Some(pad) = pad else {
            if missing > 0 {
                r.notes.push(format!(
                    "{kit}: pad {} left out (its sounds are missing)",
                    i + 1
                ));
            }
            continue;
        };
        let name = pad.name.clone();
        match p.set_pad(i, Some(pad), None) {
            Ok(()) if missing > 0 => r.notes.push(format!(
                "{kit}: pad {} ({name}) lost {missing} layer(s) (their sounds are missing)",
                i + 1
            )),
            Ok(()) => {}
            Err(e) => r
                .notes
                .push(format!("{kit}: pad {} ({name}) left out ({e})", i + 1)),
        }
    }
    // Scenes A and B, after the pads (whose sounds they share).
    for k in 0..p.scenes.len() {
        if r.text(&format!("scene.{k}")).is_none() {
            continue;
        }
        let mut pads = vec![None; PADS];
        let mut lost = 0;
        for (i, slot) in pads.iter_mut().enumerate() {
            match read_pad(r, &format!("scene.{k}.pad.{i}"), &|n| source(&p, n)) {
                Some((Some(pad), _)) if p.check_pad(&pad).is_ok() => *slot = Some(pad),
                Some(_) => lost += 1,
                None => {}
            }
        }
        if lost > 0 {
            r.notes.push(format!(
                "{kit}: scene {} lost {lost} pad(s) (their sounds are missing)",
                ["A", "B"][k]
            ));
        }
        p.scenes[k] = Some(crate::scene::Scene::from_pads(pads));
    }
    p.clear_history();
    p
}

/// The pad written at `at`, if there is one there: with the layers whose
/// sounds `source` has (None if it has none of them), and how many it
/// had to leave out.
fn read_pad(
    r: &mut Reader,
    at: &str,
    source: &dyn Fn(Option<usize>) -> Option<Source>,
) -> Option<(Option<Pad>, usize)> {
    let n = r.get::<usize>(&format!("{at}.layers"))?;
    let mut layers = Vec::new();
    let mut missing = 0;
    for j in 0..n.min(MAX_LAYERS) {
        let la = format!("{at}.layer.{j}");
        let Some(s) = source(r.get(&format!("{la}.source"))) else {
            missing += 1;
            continue;
        };
        let mut l = Layer::new(Region::whole(&s));
        r.scope(&la, |r| visit_layer(r, &mut l));
        layers.push(l);
    }
    let Some(first) = layers.first().cloned() else {
        return Some((None, missing));
    };
    let mut pad = Pad::new("", first);
    pad.layers = layers;
    r.scope(at, |r| visit_pad(r, &mut pad));
    Some((Some(pad), missing))
}

fn one_line(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

// ------------------------------------------------------------ the fields

/// A value that goes in a line as it reads back.
trait Value: Copy + Display + FromStr {
    fn ok(&self) -> bool {
        true
    }
}
impl Value for u8 {}
impl Value for u16 {}
impl Value for u32 {}
impl Value for u64 {}
impl Value for i32 {}
impl Value for usize {}
impl Value for f32 {
    fn ok(&self) -> bool {
        self.is_finite()
    }
}
impl Value for f64 {
    fn ok(&self) -> bool {
        self.is_finite()
    }
}

/// Writing and reading walk the same fields ([`visit_pad`] and the rest),
/// so the two can't drift apart.
trait Fields {
    fn num<T: Value>(&mut self, k: &str, v: &mut T);
    fn flag(&mut self, k: &str, v: &mut bool);
    fn text_field(&mut self, k: &str, v: &mut String);
    fn word<T: Copy + PartialEq>(&mut self, k: &str, v: &mut T, words: &[(&'static str, T)]);
    fn list<T: Value>(&mut self, k: &str, v: &mut Vec<T>);
    /// An optional number: `-` when there is none.
    fn opt_num<T: Value>(&mut self, k: &str, v: &mut Option<T>);
    /// An optional group: `k = yes` and its fields under `k.`, or `k = no`.
    fn opt<T: Clone>(
        &mut self,
        k: &str,
        v: &mut Option<T>,
        default: T,
        inner: impl FnOnce(&mut Self, &mut T),
    );
    fn scope(&mut self, k: &str, inner: impl FnOnce(&mut Self));
}

struct Writer {
    out: String,
    at: String,
    ids: HashMap<SourceId, usize>,
}

impl Writer {
    fn line(&mut self, k: &str, v: impl Display) {
        self.out.push_str(&format!("{}{k} = {v}\n", self.at));
    }
}

impl Fields for Writer {
    fn num<T: Value>(&mut self, k: &str, v: &mut T) {
        self.line(k, *v);
    }
    fn flag(&mut self, k: &str, v: &mut bool) {
        self.line(k, if *v { "yes" } else { "no" });
    }
    fn text_field(&mut self, k: &str, v: &mut String) {
        self.line(k, one_line(v));
    }
    fn word<T: Copy + PartialEq>(&mut self, k: &str, v: &mut T, words: &[(&'static str, T)]) {
        self.line(k, word_of(*v, words));
    }
    fn list<T: Value>(&mut self, k: &str, v: &mut Vec<T>) {
        let s: Vec<String> = v.iter().map(T::to_string).collect();
        self.line(k, s.join(" "));
    }
    fn opt_num<T: Value>(&mut self, k: &str, v: &mut Option<T>) {
        match v {
            Some(x) => self.line(k, *x),
            None => self.line(k, "-"),
        }
    }
    fn opt<T: Clone>(
        &mut self,
        k: &str,
        v: &mut Option<T>,
        _: T,
        inner: impl FnOnce(&mut Self, &mut T),
    ) {
        match v {
            Some(x) => {
                self.line(k, "yes");
                self.scope(k, |w| inner(w, x));
            }
            None => self.line(k, "no"),
        }
    }
    fn scope(&mut self, k: &str, inner: impl FnOnce(&mut Self)) {
        let len = self.at.len();
        self.at.push_str(k);
        self.at.push('.');
        inner(self);
        self.at.truncate(len);
    }
}

struct Reader<'a> {
    map: &'a HashMap<String, String>,
    at: String,
    notes: Vec<String>,
}

impl Reader<'_> {
    fn text(&self, k: &str) -> Option<String> {
        self.map.get(&format!("{}{k}", self.at)).cloned()
    }
    fn get<T: Value>(&self, k: &str) -> Option<T> {
        self.text(k)?.parse::<T>().ok().filter(T::ok)
    }
}

impl Fields for Reader<'_> {
    fn num<T: Value>(&mut self, k: &str, v: &mut T) {
        if let Some(x) = self.get(k) {
            *v = x;
        }
    }
    fn flag(&mut self, k: &str, v: &mut bool) {
        match self.text(k).as_deref() {
            Some("yes") => *v = true,
            Some("no") => *v = false,
            _ => {}
        }
    }
    fn text_field(&mut self, k: &str, v: &mut String) {
        if let Some(t) = self.text(k) {
            *v = t;
        }
    }
    fn word<T: Copy + PartialEq>(&mut self, k: &str, v: &mut T, words: &[(&'static str, T)]) {
        if let Some(x) = self.text(k).and_then(|t| from_word(&t, words)) {
            *v = x;
        }
    }
    fn list<T: Value>(&mut self, k: &str, v: &mut Vec<T>) {
        if let Some(t) = self.text(k) {
            *v = t
                .split_whitespace()
                .filter_map(|x| x.parse::<T>().ok().filter(T::ok))
                .collect();
        }
    }
    fn opt_num<T: Value>(&mut self, k: &str, v: &mut Option<T>) {
        match self.text(k).as_deref() {
            Some("-") => *v = None,
            Some(t) => {
                if let Some(x) = t.parse::<T>().ok().filter(T::ok) {
                    *v = Some(x);
                }
            }
            None => {}
        }
    }
    fn opt<T: Clone>(
        &mut self,
        k: &str,
        v: &mut Option<T>,
        default: T,
        inner: impl FnOnce(&mut Self, &mut T),
    ) {
        match self.text(k).as_deref() {
            Some("yes") => {
                let mut x = v.clone().unwrap_or(default);
                self.scope(k, |r| inner(r, &mut x));
                *v = Some(x);
            }
            Some("no") => *v = None,
            _ => {}
        }
    }
    fn scope(&mut self, k: &str, inner: impl FnOnce(&mut Self)) {
        let len = self.at.len();
        self.at.push_str(k);
        self.at.push('.');
        inner(self);
        self.at.truncate(len);
    }
}

// ------------------------------------------------------------ the words

fn word_of<T: Copy + PartialEq>(v: T, words: &[(&'static str, T)]) -> &'static str {
    words.iter().find(|(_, w)| *w == v).map_or("?", |(s, _)| s)
}

fn from_word<T: Copy>(s: &str, words: &[(&str, T)]) -> Option<T> {
    words.iter().find(|(w, _)| *w == s).map(|(_, v)| *v)
}

const PLAY_MODES: &[(&str, PlayMode)] = &[
    ("one-shot", PlayMode::OneShot),
    ("gate", PlayMode::Gate),
    ("toggle", PlayMode::Toggle),
    ("loop", PlayMode::Loop),
    ("granular", PlayMode::Granular),
];
const LOOP_MODES: &[(&str, LoopMode)] = &[
    ("forward", LoopMode::Forward),
    ("ping-pong", LoopMode::PingPong),
];
const CURVES: &[(&str, FadeCurve)] = &[
    ("linear", FadeCurve::Linear),
    ("log", FadeCurve::Log),
    ("exp", FadeCurve::Exp),
];
const PICKS: &[(&str, ChannelPick)] = &[
    ("stereo", ChannelPick::Stereo),
    ("mono", ChannelPick::MonoSum),
    ("left", ChannelPick::Left),
    ("right", ChannelPick::Right),
];
const WARP_METHODS: &[(&str, WarpMethod)] = &[
    ("stretch", WarpMethod::Stretch),
    ("repitch", WarpMethod::Repitch),
];
const CHOP_MODES: &[(&str, ChopMode)] = &[
    ("transient", ChopMode::Transient),
    ("grid", ChopMode::Grid),
    ("beat", ChopMode::Beat),
    ("manual", ChopMode::Manual),
    ("silence", ChopMode::Silence),
];
const SELECTS: &[(&str, LayerSelect)] = &[
    ("all", LayerSelect::All),
    ("cycle", LayerSelect::Cycle),
    ("random", LayerSelect::Random),
];
const FILTERS: &[(&str, FilterKind)] = &[
    ("off", FilterKind::Off),
    ("low-pass", FilterKind::LowPass),
    ("high-pass", FilterKind::HighPass),
    ("band-pass", FilterKind::BandPass),
];
const PRESSURE: &[(&str, PressureTarget)] = &[
    ("off", PressureTarget::Off),
    ("level", PressureTarget::Level),
    ("pitch", PressureTarget::Pitch),
    ("filter", PressureTarget::Filter),
    ("grain-density", PressureTarget::GrainDensity),
    ("grain-size", PressureTarget::GrainSize),
    ("grain-position", PressureTarget::GrainPosition),
    ("grain-spray", PressureTarget::GrainSpray),
];
const LEVELS: &[(&str, Levels)] = &[
    ("velocity", Levels::Velocity),
    ("tune", Levels::Tune),
    ("cutoff", Levels::Cutoff),
    ("decay", Levels::Decay),
    ("start", Levels::Start),
];
/// The grain window's shape, its number kept beside it.
#[derive(Clone, Copy, PartialEq)]
enum WindowKind {
    Hann,
    Tukey,
    Trapezoid,
    Exponential,
}
const WINDOWS: &[(&str, WindowKind)] = &[
    ("hann", WindowKind::Hann),
    ("tukey", WindowKind::Tukey),
    ("trapezoid", WindowKind::Trapezoid),
    ("exponential", WindowKind::Exponential),
];

fn pads_play_text(p: PadsPlay) -> String {
    match p {
        PadsPlay::Pads => "pads".into(),
        PadsPlay::Keys { from } => format!("keys {from}"),
        PadsPlay::Levels { pad, by } => format!("levels {pad} {}", word_of(by, LEVELS)),
    }
}

fn pads_play_from(t: &str) -> Option<PadsPlay> {
    let w: Vec<&str> = t.split_whitespace().collect();
    match w.as_slice() {
        ["pads"] => Some(PadsPlay::Pads),
        ["keys", from] => Some(PadsPlay::Keys {
            from: from.parse().ok()?,
        }),
        ["levels", pad, by] => Some(PadsPlay::Levels {
            pad: pad.parse().ok()?,
            by: from_word(by, LEVELS)?,
        }),
        _ => None,
    }
}

// ------------------------------------------------------------ the walks
//
// Each takes its struct apart field by field, so a field added to the
// model doesn't build until it is saved here too.

fn visit_envelope<F: Fields>(f: &mut F, a: &mut Ahdsr) {
    let Ahdsr {
        attack_ms,
        hold_ms,
        decay_ms,
        sustain,
        release_ms,
    } = a;
    f.num("attack_ms", attack_ms);
    f.num("hold_ms", hold_ms);
    f.num("decay_ms", decay_ms);
    f.num("sustain", sustain);
    f.num("release_ms", release_ms);
}

fn visit_grain<F: Fields>(f: &mut F, g: &mut GrainParams) {
    let GrainParams {
        position,
        spray,
        size_ms,
        density,
        pitch,
        pitch_spray,
        scale,
        scan,
        window,
        reverse,
        spread,
        gain,
        sync,
    } = g;
    f.num("position", position);
    f.num("spray", spray);
    f.num("size_ms", size_ms);
    f.num("density", density);
    f.num("pitch", pitch);
    f.num("pitch_spray", pitch_spray);
    f.num("scale", scale);
    f.num("scan", scan);
    let (mut kind, mut shape) = match *window {
        GrainWindow::Hann => (WindowKind::Hann, 0.0),
        GrainWindow::Tukey { taper } => (WindowKind::Tukey, taper),
        GrainWindow::Trapezoid { ramp } => (WindowKind::Trapezoid, ramp),
        GrainWindow::Exponential { decay } => (WindowKind::Exponential, decay),
    };
    f.word("window", &mut kind, WINDOWS);
    f.num("window_shape", &mut shape);
    *window = match kind {
        WindowKind::Hann => GrainWindow::Hann,
        WindowKind::Tukey => GrainWindow::Tukey { taper: shape },
        WindowKind::Trapezoid => GrainWindow::Trapezoid { ramp: shape },
        WindowKind::Exponential => GrainWindow::Exponential { decay: shape },
    };
    f.num("reverse", reverse);
    f.num("spread", spread);
    f.num("gain", gain);
    let none = TempoSync {
        frames_per_beat: 22_050.0,
        grains_per_beat: None,
        size_beats: None,
    };
    f.opt("sync", sync, none, |f, t| {
        let TempoSync {
            frames_per_beat,
            grains_per_beat,
            size_beats,
        } = t;
        f.num("frames_per_beat", frames_per_beat);
        f.opt_num("grains_per_beat", grains_per_beat);
        f.opt_num("size_beats", size_beats);
    });
}

fn visit_master<F: Fields>(f: &mut F, m: &mut MasterFx) {
    let MasterFx { mix, grain } = m;
    f.num("mix", mix);
    f.scope("grain", |f| visit_grain(f, grain));
}

/// A region, but for its source (the layer's `source` line).
fn visit_region<F: Fields>(f: &mut F, r: &mut Region) {
    let Region {
        source: _,
        start,
        end,
        sustain_loop,
        fade_in,
        fade_out,
        gain_db,
        reverse,
        channels,
        tune_cents,
        mode,
        grain,
        warp,
    } = r;
    f.num("start", start);
    f.num("end", end);
    let open = Loop {
        start: *start,
        end: *end,
        crossfade: 0,
        mode: LoopMode::Forward,
    };
    f.opt("loop", sustain_loop, open, |f, l| {
        let Loop {
            start,
            end,
            crossfade,
            mode,
        } = l;
        f.num("start", start);
        f.num("end", end);
        f.num("crossfade", crossfade);
        f.word("mode", mode, LOOP_MODES);
    });
    for (k, fade) in [("fade_in", fade_in), ("fade_out", fade_out)] {
        f.scope(k, |f| {
            let Fade { frames, curve } = fade;
            f.num("frames", frames);
            f.word("curve", curve, CURVES);
        });
    }
    f.num("gain_db", gain_db);
    f.flag("reverse", reverse);
    f.word("channels", channels, PICKS);
    f.num("tune_cents", tune_cents);
    f.word("mode", mode, PLAY_MODES);
    f.scope("grain", |f| visit_grain(f, grain));
    f.opt("warp", warp, Warp::new(120.0), |f, w| {
        let Warp { bpm, method, pins } = w;
        f.num("bpm", bpm);
        f.word("method", method, WARP_METHODS);
        let mut frames: Vec<u64> = pins.iter().map(|p| p.frame).collect();
        let mut beats: Vec<f64> = pins.iter().map(|p| p.beat).collect();
        f.list("pin_frames", &mut frames);
        f.list("pin_beats", &mut beats);
        *pins = frames
            .into_iter()
            .zip(beats)
            .map(|(frame, beat)| Pin { frame, beat })
            .collect();
    });
}

/// A layer, but for its region's source.
fn visit_layer<F: Fields>(f: &mut F, l: &mut Layer) {
    let Layer {
        region,
        vel_lo,
        vel_hi,
        level_db,
        pan,
        tune_cents,
    } = l;
    f.scope("region", |f| visit_region(f, region));
    f.num("vel_lo", vel_lo);
    f.num("vel_hi", vel_hi);
    f.num("level_db", level_db);
    f.num("pan", pan);
    f.num("tune_cents", tune_cents);
}

/// A pad, but for its layers.
fn visit_pad<F: Fields>(f: &mut F, p: &mut Pad) {
    let Pad {
        name,
        layers: _,
        select,
        polyphony,
        mute_group,
        chokes,
        amp,
        filter,
        filter_env,
        vel_to_level,
        vel_to_start,
        level_db,
        pan,
        tune_cents,
        pressure,
        keys,
        wow,
        key_fade,
        vel_fade,
    } = p;
    f.text_field("name", name);
    f.word("select", select, SELECTS);
    f.num("polyphony", polyphony);
    f.opt_num("mute_group", mute_group);
    f.list("chokes", chokes);
    f.scope("amp", |f| visit_envelope(f, amp));
    f.scope("filter", |f| {
        let Filter {
            kind,
            poles,
            cutoff_hz,
            resonance,
            env_octaves,
        } = filter;
        f.word("kind", kind, FILTERS);
        f.num("poles", poles);
        f.num("cutoff_hz", cutoff_hz);
        f.num("resonance", resonance);
        f.num("env_octaves", env_octaves);
    });
    f.scope("filter_env", |f| visit_envelope(f, filter_env));
    f.num("vel_to_level", vel_to_level);
    f.num("vel_to_start", vel_to_start);
    f.num("level_db", level_db);
    f.num("pan", pan);
    f.num("tune_cents", tune_cents);
    f.scope("pressure", |f| {
        let PressureRoute { target, depth } = pressure;
        f.word("target", target, PRESSURE);
        f.num("depth", depth);
    });
    let full = Keys {
        lo: 0,
        hi: 127,
        root: 60,
    };
    f.opt("keys", keys, full, |f, k| {
        let Keys { lo, hi, root } = k;
        f.num("lo", lo);
        f.num("hi", hi);
        f.num("root", root);
    });
    f.num("wow", wow);
    f.num("key_fade", key_fade);
    f.num("vel_fade", vel_fade);
}
