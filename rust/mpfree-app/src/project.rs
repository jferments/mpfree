//! Whole projects: New, Save and Open (the Phase 2 gate; Jay's ask after
//! live test 3). The folder and its crash-safe save are `mpfree-library`'s
//! (docs/dev/design/browser.md, "Projects"); this is what the app puts in
//! it and how each engine takes its part back.
//!
//! The parts, one text file each in the project's `parts/`:
//!
//! | Part | Owner | Made by | Taken back by |
//! |---|---|---|---|
//! | `clips` | the time engine | [`mpfree_seq::save`]: tracks, clips, scenes, the song | a new [`TimeHost`] built on the screen thread and swapped in |
//! | `mixer` | the mixer | [`mixer_text`]: every strip's controls, each insert with its parameters (and a plugin's state) | [`mixer_load`] builds the effects on the screen thread (a plugin through the rack, in its own process); [`MixHost::open`] swaps them in over a few periods |
//! | `instruments` | each track's own instrument (`instruments.rs`) | [`instruments_text`]: each track's instrument with its parameters (and a plugin's state) | [`instruments_load`] makes them on the screen thread; the audio loop puts them in |
//! | `mod` | the mod engine | `ModMix::save` | `ModMix::load`, once the inserts are in |
//! | `synth` | the synth | [`synth_text`]: the Synth tab's options and every engine's sound | the options go through the app (as at start), the sounds onto the engines |
//! | `sampler` | the Sample tab | `SamplePage::save_part`, with its sounds in `samples/` | `SamplePage::open_part` |
//! | `midi` | the I/O view | `mpfree_midi_ui::MidiPart` | the same |
//! | `looper` | the Looper | [`loops_part`], a WAV a layer in `samples/` | [`loops_open`] on a thread, then `LoopHost::open` |
//! | `arp` | the arps | `App::arp_part` | `App::open_arp` |
//! | `audiotracks` | the audio tracks | [`audio_part`]: each track's settings and regions, a WAV a take in `samples/` | [`audio_open`] on a thread, then [`crate::audiotracks::TrackHost::open`] |
//!
//! The project's tempo is `project.txt`'s `tempo`.
//!
//! **Saving** (SHIFT + BROWSE, or Save in the browser): the screen asks
//! the audio loop ([`crate::gui::FromUi::SaveProject`]); the loop copies what
//! its engines hold ([`Collected`], a few small allocations once) and
//! sends it to the screen, which writes the text, adds its own parts and
//! hands the lot to the browser's worker thread. Nothing is written on
//! the audio thread.
//!
//! **Opening** and **New project**: the screen reads every part and builds
//! what is costly there (the time engine with its compiled clips, the
//! mixer's effects), then sends one [`Opening`] to the audio loop, which
//! swaps the pieces in between two periods and gives the old ones back to
//! be freed on the screen thread. New project opens the parts the app
//! started with ([`Defaults`]): an empty song of eight tracks, the mixer
//! as it starts, the start presets, the factory kits.

use std::fmt::Write as _;

use mpfree_fx::ids::mixer as mx;
use mpfree_fx::{Insert, Mixer, StripId};
use mpfree_library::Snapshot;
use mpfree_node::Node;

use crate::synth::Engine;
use crate::time::TimeHost;
use crate::PERIOD;

/// The parts' owners (their file names in `parts/`).
pub const CLIPS: &str = "clips";
pub const MIXER: &str = "mixer";
pub const MOD: &str = "mod";
pub const SYNTH: &str = "synth";
pub const SAMPLER: &str = "sampler";
pub const MIDI: &str = "midi";
pub const LOOPER: &str = "looper";
/// Each track's arp and its Note Repeat rate (`App::arp_part`).
pub const ARP: &str = "arp";
/// Each track's own instrument ([`instruments_text`]).
pub const INSTRUMENTS: &str = "instruments";
/// The Song view's audio tracks and their takes ([`audio_part`]).
pub const AUDIOTRACKS: &str = "audiotracks";

/// The mixer part's first line.
pub const MIXER_FORMAT: &str = "mpfree-mixer 1";
/// The synth part's first line.
pub const SYNTH_FORMAT: &str = "mpfree-synth 1";
/// The first line of the instruments part.
pub const INSTRUMENTS_FORMAT: &str = "mpfree-instruments 1";

// ------------------------------------------------------------ the loop's side

/// What the audio loop copies out of its engines for a save: everything
/// but the parts the screen thread has itself (the kits, the I/O setup).
pub struct Collected {
    pub tempo: f64,
    /// The transport's swing (the Play page's Swing knob).
    pub swing: mpfree_transport::time::Swing,
    pub clips: mpfree_seq::Project,
    pub mixer: String,
    pub modu: String,
    pub synth: String,
    /// The arps ([`crate::app::App::arp_part`]).
    pub arp: String,
    /// Each track's own instrument ([`instruments_text`]).
    pub instruments: String,
    /// The Looper's loops with their layers' audio (read as they play:
    /// [`crate::looping::LoopHost::save`]); None when the looper couldn't
    /// save. Freed on the screen thread.
    pub loops: Option<mpfree_looper::save::Saved>,
    /// The audio tracks with every take's audio (read over a few periods:
    /// [`mpfree_audiotracks::host::AudioHost::save`]); None when they
    /// couldn't save. Freed on the screen thread.
    pub audio: Option<mpfree_audiotracks::save::Saved>,
}

impl std::fmt::Debug for Collected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Collected({} BPM, {} tracks)",
            self.tempo,
            self.clips.tracks.len()
        )
    }
}

/// What the audio loop swaps in for an open or a New project. Each piece
/// is optional: a project without a part leaves that engine as it is.
pub struct Opening {
    pub tempo: Option<f64>,
    pub swing: Option<mpfree_transport::time::Swing>,
    pub time: Option<Box<TimeHost>>,
    pub mix: Option<Box<MixLoad>>,
    /// The Synth tab's options (`settings/synth.conf`'s lines) and the
    /// engines' sounds.
    pub synth: Option<(mpfree_store::Settings, Vec<(Engine, String)>)>,
    /// The arp part's text; None (New, or a project saved before it had
    /// one) puts every arp back to its default.
    pub arp: Option<String>,
    /// Each track's own instrument, made here. Every track not in it
    /// (all of them for New, or a project saved before the part) plays
    /// the kit.
    pub instruments: Vec<Option<Loaded>>,
    /// Lines for the log: parts that didn't read.
    pub notes: Vec<String>,
    /// What it is, for the log ("Night drive", "a new project").
    pub what: String,
    /// The Looper's loops are cleared (the project has a looper part, or
    /// it is New). Its loops, read on a thread of their own, follow
    /// (`FromUi::OpenLoops`).
    pub clear_loops: bool,
    /// The audio tracks are emptied (the project has an audio tracks part,
    /// or it is New). Its tracks and takes, read on a thread of their own,
    /// follow (`FromUi::OpenAudio`).
    pub clear_audio: bool,
}

impl std::fmt::Debug for Opening {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Opening({})", self.what)
    }
}

/// [`Opening`] and [`Collected`] travel in boxes between the threads;
/// the messages compare by address only.
impl PartialEq for Opening {
    fn eq(&self, o: &Opening) -> bool {
        std::ptr::eq(self, o)
    }
}

impl PartialEq for Collected {
    fn eq(&self, o: &Collected) -> bool {
        std::ptr::eq(self, o)
    }
}

/// The loop's parts at start, before anything is played or changed: what
/// New project opens. Made once, before the sound starts.
#[derive(Clone, Debug, Default)]
pub struct Defaults {
    pub tempo: f64,
    pub mixer: String,
    pub modu: String,
    pub synth: String,
}

/// `project.txt`'s line for the transport's swing: `swing = 240 58`
/// (the grid in ticks, then the amount).
pub const SWING: &str = "swing";

fn swing_text(s: mpfree_transport::time::Swing) -> String {
    format!("{} {}", s.grid, s.amount)
}

fn parse_swing(v: &str) -> Option<mpfree_transport::time::Swing> {
    let (g, a) = v.trim().split_once(' ')?;
    mpfree_transport::time::Swing::new(g.trim().parse().ok()?, a.trim().parse().ok()?)
}

/// The project as text, from what the loop collected and the screen's own
/// parts (`extra`: the sampler's, the I/O setup's, the loops').
pub fn snapshot(c: &Collected, extra: Vec<(String, String)>) -> Snapshot {
    let mut parts = vec![
        (CLIPS.to_string(), mpfree_seq::save::to_text(&c.clips)),
        (MIXER.to_string(), c.mixer.clone()),
        (MOD.to_string(), c.modu.clone()),
        (SYNTH.to_string(), c.synth.clone()),
        (ARP.to_string(), c.arp.clone()),
    ];
    if c.instruments.lines().count() > 1 {
        parts.push((INSTRUMENTS.to_string(), c.instruments.clone()));
    }
    parts.extend(extra);
    Snapshot {
        meta: mpfree_library::Meta {
            tempo: c.tempo,
            app: format!("mpfree {}", env!("CARGO_PKG_VERSION")),
            extra: vec![(SWING.to_string(), swing_text(c.swing))],
            ..Default::default()
        },
        parts,
    }
}

/// Reads the loop's parts of `snap` and builds what is costly here (the
/// time engine, the mixer's effects): an [`Opening`] for the audio loop.
/// `rate` is the sound's. A part that doesn't read is left out, with a
/// line saying so: its engine keeps what it has.
pub fn opening(
    snap: &Snapshot,
    rate: u32,
    what: &str,
    plugins: &crate::plugins::Plugins,
) -> Opening {
    let mut notes = Vec::new();
    let tempo = Some(snap.meta.tempo).filter(|t| t.is_finite() && *t >= 20.0 && *t <= 300.0);
    let swing = snap
        .meta
        .extra
        .iter()
        .find(|(k, _)| k == SWING)
        .and_then(|(_, v)| parse_swing(v));
    let time = snap.part(CLIPS).and_then(|text| {
        let p = mpfree_seq::save::from_text(text)
            .map_err(|e| notes.push(format!("project: the clips weren't opened ({e})")))
            .ok()?;
        TimeHost::with_project(rate, tempo.unwrap_or(120.0), p)
            .map_err(|e| notes.push(format!("project: the clips weren't opened ({e})")))
            .ok()
            .map(Box::new)
    });
    let mix = snap.part(MIXER).and_then(|text| {
        mixer_load(text, snap.part(MOD), rate, plugins)
            .map_err(|e| notes.push(format!("project: the mixer wasn't opened ({e})")))
            .ok()
            .map(Box::new)
    });
    let synth = snap.part(SYNTH).and_then(|text| {
        synth_parse(text)
            .map_err(|e| notes.push(format!("project: the synth wasn't opened ({e})")))
            .ok()
    });
    if let Some(m) = &mix {
        notes.extend(m.notes.iter().cloned());
    }
    let instruments = match snap.part(INSTRUMENTS) {
        None => Vec::new(),
        Some(text) => match instruments_load(text, rate, plugins) {
            Ok((slots, more)) => {
                notes.extend(more);
                slots
            }
            Err(e) => {
                notes.push(format!(
                    "project: the tracks' instruments weren't opened ({e}); they play the kit"
                ));
                Vec::new()
            }
        },
    };
    Opening {
        tempo,
        swing,
        time,
        mix,
        synth,
        arp: snap.part(ARP).map(str::to_string),
        instruments,
        notes,
        what: what.to_string(),
        clear_loops: snap.part(LOOPER).is_some(),
        clear_audio: snap.part(AUDIOTRACKS).is_some(),
    }
}

// ------------------------------------------------------------ the looper

/// The looper's part and its layers' sounds for the project's `samples/`,
/// each layer named for its audio (`loop-<id>.wav`), so a layer saved
/// before is not written again.
pub fn loops_part(mut saved: mpfree_looper::save::Saved) -> (String, Vec<(String, Vec<f32>)>) {
    let mut files = Vec::new();
    for (_, f) in &mut saved.loops {
        let n = 2 * f.frames;
        for y in &mut f.layers {
            let mut audio = std::mem::take(&mut y.audio);
            audio.truncate(n);
            y.file = format!("loop-{}.wav", audio_id(&audio, saved.rate));
            if !files.iter().any(|(name, _)| *name == y.file) {
                files.push((y.file.clone(), audio));
            }
        }
    }
    (mpfree_looper::save::to_text(&saved), files)
}

/// 128 bits for a layer's audio: two FNV-1a lanes over the samples' bits,
/// the rate and the length, each mixed at the end (splitmix64).
fn audio_id(audio: &[f32], rate: u32) -> String {
    let (mut a, mut b) = (0xcbf2_9ce4_8422_2325u64, 0x6c62_272e_07bb_0142u64);
    for x in std::iter::once(rate)
        .chain(std::iter::once(audio.len() as u32))
        .chain(audio.iter().map(|v| v.to_bits()))
    {
        a = (a ^ u64::from(x)).wrapping_mul(0x0000_0100_0000_01b3);
        b = (b ^ u64::from(x.rotate_left(16))).wrapping_mul(0x9e37_79b9_7f4a_7c15 | 1);
    }
    let mix = |mut z: u64| {
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    format!("{:016x}{:016x}", mix(a), mix(b))
}

/// Reads the looper's part and each layer's sound (`read` gives a file's
/// bytes from the project's `samples/`). The files can be big: run it off
/// the screen thread. A loop whose sound can't be read is left out, with
/// a line saying so.
pub fn loops_open(
    text: &str,
    read: impl Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<(mpfree_looper::save::Saved, Vec<String>), String> {
    let mut saved = mpfree_looper::save::from_text(text)?;
    let mut notes = Vec::new();
    saved.loops.retain_mut(|(n, f)| {
        for y in &mut f.layers {
            match read(&y.file).and_then(|b| mpfree_looper::save::from_wav(&b)) {
                Ok((audio, _)) => y.audio = audio,
                Err(e) => {
                    notes.push(format!(
                        "project: loop {} wasn't opened ({}: {e})",
                        *n + 1,
                        y.file
                    ));
                    return false;
                }
            }
        }
        true
    });
    Ok((saved, notes))
}

// ------------------------------------------------------------ the audio tracks

/// A take's audio for the project's `samples/`: its file name, its
/// samples (interleaved) and channels.
pub type TakeAudio = (String, Vec<f32>, usize);

/// The audio tracks' part and their takes' sounds for the project's
/// `samples/`, each take named for its audio (`take-<id>.wav`), so a take
/// saved before is not written again. The audio is moved, not copied.
pub fn audio_part(mut saved: mpfree_audiotracks::save::Saved) -> (String, Vec<TakeAudio>) {
    let mut files: Vec<TakeAudio> = Vec::new();
    for k in &mut saved.takes {
        let audio = std::mem::take(&mut k.audio);
        // The channels count in the name: the same samples as mono and as
        // stereo are two sounds.
        let tag = saved.rate ^ ((k.channels as u32) << 30);
        k.file = format!("take-{}.wav", audio_id(&audio, tag));
        if !files.iter().any(|(name, _, _)| *name == k.file) {
            files.push((k.file.clone(), audio, k.channels));
        }
    }
    (mpfree_audiotracks::save::to_text(&saved), files)
}

/// Reads the audio tracks' part and each take's sound (`read` gives a
/// file's bytes from the project's `samples/`); `rate` is the sound's.
/// The files can be big: run it off the screen thread. A take whose sound
/// can't be read, or isn't the take's, is left out with the regions
/// playing it, and a line says so.
pub fn audio_open(
    text: &str,
    rate: u32,
    read: impl Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<(mpfree_audiotracks::save::Saved, Vec<String>), String> {
    let mut saved = mpfree_audiotracks::save::from_text(text)?;
    let mut notes = Vec::new();
    if saved.rate != rate {
        notes.push(format!(
            "project: the audio tracks were recorded at {} Hz and play at {rate} Hz, so they sound off-speed",
            saved.rate
        ));
    }
    let mut gone = Vec::new();
    saved.takes.retain_mut(|k| {
        let got = read(&k.file)
            .and_then(|b| mpfree_audiotracks::save::from_wav(&b))
            .and_then(|(audio, ch, _)| {
                if ch != k.channels || audio.len() != k.frames * k.channels {
                    Err("not this take's sound".to_string())
                } else {
                    Ok(audio)
                }
            });
        match got {
            Ok(audio) => {
                k.audio = audio;
                true
            }
            Err(e) => {
                notes.push(format!(
                    "project: a take of audio {} wasn't opened ({}: {e}); the regions playing it are left out",
                    k.track + 1,
                    k.file
                ));
                gone.push(k.id);
                false
            }
        }
    });
    if !gone.is_empty() {
        for t in &mut saved.tracks {
            t.regions.retain(|r| !gone.contains(&r.take));
        }
    }
    Ok((saved, notes))
}

/// New project: the parts the app started with, and an empty song.
pub fn new_snapshot(d: &Defaults) -> Snapshot {
    let mut clips = mpfree_seq::Project::new();
    for i in 0..crate::time::TRACKS {
        clips.tracks.push(mpfree_seq::Track::new(
            &format!("Track {}", i + 1),
            i as u16,
        ));
    }
    Snapshot {
        meta: mpfree_library::Meta {
            tempo: d.tempo,
            extra: vec![(
                SWING.to_string(),
                swing_text(mpfree_transport::time::Swing::default()),
            )],
            ..Default::default()
        },
        parts: vec![
            (CLIPS.to_string(), mpfree_seq::save::to_text(&clips)),
            (MIXER.to_string(), d.mixer.clone()),
            (MOD.to_string(), d.modu.clone()),
            (SYNTH.to_string(), d.synth.clone()),
        ],
    }
}

// ------------------------------------------------------------ the mixer

/// A strip as text: `in0`, `ps3`, `t1`, `bus0`, `ret0`, `master`.
pub fn strip_text(s: StripId) -> String {
    match s {
        StripId::Input(n) => format!("in{n}"),
        StripId::Pad(n) => format!("ps{n}"),
        StripId::Track(n) => format!("t{n}"),
        StripId::Bus(n) => format!("bus{n}"),
        StripId::Return(n) => format!("ret{n}"),
        StripId::Master => "master".to_string(),
    }
}

pub fn parse_strip(w: &str) -> Option<StripId> {
    let n = |p: &str| -> Option<usize> {
        let d = w.strip_prefix(p)?;
        (!d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
            .then(|| d.parse().ok())
            .flatten()
    };
    if w == "master" {
        return Some(StripId::Master);
    }
    n("ps")
        .map(StripId::Pad)
        .or_else(|| n("in").map(StripId::Input))
        .or_else(|| n("bus").map(StripId::Bus))
        .or_else(|| n("ret").map(StripId::Return))
        .or_else(|| n("t").map(StripId::Track))
}

/// Whether a strip control belongs to the project. Not the master fader
/// (it is the Level, the room's, never louder from a file), not the line
/// input's source, monitoring and arming (they follow Prefs and the
/// looper), not a pad channel's mute (the pad mutes are kept on their
/// own, `padmutes`).
pub(crate) fn saved_control(s: StripId, id: u16) -> bool {
    if !mpfree_fx::mixer_param_applies(s, id) {
        return false;
    }
    match s {
        StripId::Master => id != mx::VOLUME,
        StripId::Input(_) => !matches!(id, mx::SOURCE | mx::MONITOR | mx::ARM),
        StripId::Pad(_) => id != mx::MUTE,
        _ => true,
    }
}

/// Whether `st` is a track's own instrument channel with nothing set:
/// every control at its default and no effects. The mixer part leaves these
/// out (a mixer has one for every sequencer track, up to 128), and an open
/// sets the ones a project leaves out back to their defaults
/// ([`crate::mixing::MixHost::tend_open`]).
pub fn untouched_channel(mixer: &Mixer, st: StripId) -> bool {
    matches!(st, StripId::Track(t) if t >= crate::mixing::TRACKS)
        && mixer.chain(st).is_none_or(|c| c.is_empty())
        && mpfree_fx::strip_params()
            .iter()
            .filter(|p| saved_control(st, p.id))
            .all(|p| mixer.param(st, p.id) == p.default)
}

/// The mixer part: every strip's controls (but [`untouched_channel`]s),
/// then each insert with its parameters (and a plugin's state, base64), and
/// the pad mutes.
///
/// ```text
/// mpfree-mixer 1
/// padmutes 0
/// strip t0 0=-3 1=0 3=0 4=0 5=0 6=-80 ...
/// insert t0 1 1 mpfree.fx.dj_filter
/// param 0 0.5
/// insert master 4 1 clap:com.example.verb file=verb.clap key=t1
/// state AAECAw==
/// ```
pub fn mixer_text(mixer: &Mixer, pad_mutes: u16) -> String {
    let mut s = String::with_capacity(4096);
    s.push_str(MIXER_FORMAT);
    s.push('\n');
    let _ = writeln!(s, "padmutes {pad_mutes}");
    let strips: Vec<StripId> = mixer
        .strips()
        .filter(|&st| !untouched_channel(mixer, st))
        .collect();
    for &st in &strips {
        let _ = write!(s, "strip {}", strip_text(st));
        for p in mpfree_fx::strip_params() {
            if saved_control(st, p.id) {
                let _ = write!(s, " {}={}", p.id, mixer.param(st, p.id));
            }
        }
        s.push('\n');
    }
    for &st in &strips {
        let Some(chain) = mixer.chain(st) else {
            continue;
        };
        for slot in 0..chain.len() {
            let (Some(node), Some(uid)) = (chain.node(slot), chain.uid(slot)) else {
                continue;
            };
            let _ = write!(
                s,
                "insert {} {uid} {} {}",
                strip_text(st),
                u8::from(chain.is_on(slot)),
                node.info().id
            );
            if let Some(k) = mixer.key(st, slot) {
                let _ = write!(s, " key={}", strip_text(k));
            }
            for (k, v) in node.save_extras() {
                if word_ok(&k) && word_ok(&v) && k != "key" {
                    let _ = write!(s, " {k}={v}");
                }
            }
            s.push('\n');
            for p in node.params() {
                if !p.readonly {
                    let _ = writeln!(s, "param {} {}", p.id, node.param(p.id));
                }
            }
            if let Some(state) = node.save_state() {
                let _ = writeln!(s, "state {}", base64(&state));
            }
        }
    }
    s
}

/// A word that fits a `key=value` on one line.
fn word_ok(w: &str) -> bool {
    !w.is_empty() && !w.contains(char::is_whitespace) && !w.contains('=')
}

/// An insert as a project has it, with the strip keying it (if any).
pub type Keyed = (Insert, Option<StripId>);

/// An insert being read: its strip, slot id, on or off, node and key.
type Reading = (StripId, u16, bool, Box<dyn Node>, Option<StripId>);

/// The mixer as a project had it, built here (the effects made and set)
/// for the audio loop to swap in ([`crate::mixing::MixHost::open`]).
pub struct MixLoad {
    pub pad_mutes: u16,
    pub strips: Vec<(StripId, Vec<(u16, f32)>)>,
    /// Each strip's inserts, in order.
    pub chains: Vec<(StripId, Vec<Keyed>)>,
    /// The mod engine's part, loaded once the inserts are in.
    pub modu: Option<String>,
    pub notes: Vec<String>,
    /// Keys set once their inserts are in: (strip, slot, key strip).
    /// Filled by the audio loop as it puts them in.
    pub keys: Vec<(StripId, usize, StripId)>,
}

/// Makes the effect a project names: a built-in one (`mpfree_fx_ui::make`)
/// or a plugin (`clap:...`), started in its own process by the rack
/// (which waits for it: the screen thread's job). Err says why not, with
/// the plugin's file (`file=` in `extras`) when it isn't there.
pub fn make_node(
    id: &str,
    extras: &[(String, String)],
    plugins: &crate::plugins::Plugins,
) -> Result<Box<dyn Node>, String> {
    if crate::plugins::Plugins::is_plugin(id) {
        return make_plugin(id, extras, plugins);
    }
    mpfree_fx_ui::make(id).ok_or_else(|| "this build can't make it".to_string())
}

/// A plugin a project names, started by the rack; Err names its file
/// (`file=` in `extras`) when this stick hasn't got it.
fn make_plugin(
    id: &str,
    extras: &[(String, String)],
    plugins: &crate::plugins::Plugins,
) -> Result<Box<dyn Node>, String> {
    check_plugin(id, extras, plugins)?;
    plugins.make(id)
}

/// Whether plugin `id` is on this stick; if not, which file the project
/// names.
fn check_plugin(
    id: &str,
    extras: &[(String, String)],
    plugins: &crate::plugins::Plugins,
) -> Result<(), String> {
    if plugins.find(id).is_none() {
        let file = extras
            .iter()
            .find(|(k, _)| k == "file")
            .map_or(String::new(), |(_, f)| format!(" ({f})"));
        return Err(format!(
            "the plugin{file} isn't in files/plugins/ on this stick"
        ));
    }
    Ok(())
}

/// Reads the mixer part and builds its effects (allocates: not on the
/// audio thread). An effect this build can't make is left out, with a
/// line saying so; a strip the mixer hasn't got is skipped.
pub fn mixer_load(
    text: &str,
    modu: Option<&str>,
    rate: u32,
    plugins: &crate::plugins::Plugins,
) -> Result<MixLoad, String> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, l)) if l.trim() == MIXER_FORMAT => {}
        _ => return Err(format!("not an {MIXER_FORMAT} part")),
    }
    let mut out = MixLoad {
        pad_mutes: 0,
        strips: Vec::new(),
        chains: Vec::new(),
        modu: modu.map(str::to_string),
        notes: Vec::new(),
        // Room for every key, so the audio loop never grows it.
        keys: Vec::with_capacity(64),
    };
    // The insert being read: its node and slot settings, or None when it
    // was left out (its params and state are skipped too).
    let mut open: Option<Option<Reading>> = None;
    let finish = |open: &mut Option<Option<Reading>>, out: &mut MixLoad| {
        if let Some(Some((strip, uid, on, node, key))) = open.take() {
            let mut ins = Insert::new(node, rate, PERIOD).with_uid(uid);
            if !on {
                ins = ins.off();
            }
            match out.chains.iter_mut().find(|(s, _)| *s == strip) {
                Some((_, v)) => v.push((ins, key)),
                None => out.chains.push((strip, vec![(ins, key)])),
            }
        }
    };
    for (i, raw) in lines {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = || format!("line {}: {line}", i + 1);
        let mut w = line.split_whitespace();
        match w.next() {
            Some("padmutes") => {
                out.pad_mutes = w.next().and_then(|x| x.parse().ok()).ok_or_else(bad)?;
            }
            Some("strip") => {
                finish(&mut open, &mut out);
                let st = w.next().and_then(parse_strip).ok_or_else(bad)?;
                let mut values = Vec::new();
                for kv in w {
                    let (k, v) = kv.split_once('=').ok_or_else(bad)?;
                    let id: u16 = k.parse().map_err(|_| bad())?;
                    let v: f32 = v.parse().map_err(|_| bad())?;
                    if v.is_finite() && saved_control(st, id) {
                        values.push((id, v));
                    }
                }
                out.strips.push((st, values));
            }
            Some("insert") => {
                finish(&mut open, &mut out);
                let st = w.next().and_then(parse_strip).ok_or_else(bad)?;
                let uid: u16 = w.next().and_then(|x| x.parse().ok()).ok_or_else(bad)?;
                let on = match w.next() {
                    Some("0") => false,
                    Some("1") => true,
                    _ => return Err(bad()),
                };
                let id = w.next().ok_or_else(bad)?;
                let mut key = None;
                let mut extras = Vec::new();
                for kv in w {
                    let (k, v) = kv.split_once('=').ok_or_else(bad)?;
                    if k == "key" {
                        key = Some(parse_strip(v).ok_or_else(bad)?);
                    } else {
                        extras.push((k.to_string(), v.to_string()));
                    }
                }
                open = Some(match make_node(id, &extras, plugins) {
                    Ok(n) => Some((st, uid, on, n, key)),
                    Err(e) => {
                        out.notes.push(format!(
                            "project: the effect {id} on {} is left out: {e}",
                            strip_text(st)
                        ));
                        None
                    }
                });
            }
            Some("param") => {
                let Some(o) = open.as_mut() else {
                    return Err(bad());
                };
                let id: u16 = w.next().and_then(|x| x.parse().ok()).ok_or_else(bad)?;
                let v: f32 = w.next().and_then(|x| x.parse().ok()).ok_or_else(bad)?;
                if let Some((_, _, _, node, _)) = o.as_mut() {
                    // An unknown id (a newer build's) is skipped by the node.
                    if v.is_finite() && node.params().iter().any(|p| p.id == id && !p.readonly) {
                        node.set_param(id, v);
                    }
                }
            }
            Some("state") => {
                let Some(o) = open.as_mut() else {
                    return Err(bad());
                };
                let bytes = unbase64(w.next().unwrap_or("")).ok_or_else(bad)?;
                if let Some((st, _, _, node, _)) = o.as_mut() {
                    if let Err(e) = node.load_state(&bytes) {
                        out.notes.push(format!(
                            "project: the effect {} on {} didn't take its state ({e})",
                            node.info().id,
                            strip_text(*st)
                        ));
                    }
                }
            }
            // A newer build's line.
            _ => {}
        }
    }
    finish(&mut open, &mut out);
    Ok(out)
}

// ------------------------------------------------------------ the tracks' instruments

/// A track's own instrument as a project had it, made on the screen
/// thread, for the audio loop to put in (`Instruments::put`).
pub struct Loaded {
    /// The track, from 0.
    pub track: usize,
    pub instrument: mpfree_track::Instrument,
    pub held: crate::instruments::Held,
}

/// The instruments part: a `track` line for each track that has its own
/// instrument (numbered from 1, as the screen shows them) with its node
/// id and `key=value` extras, then its parameters and a plugin's state.
/// Called on the audio thread when a project is saved (a few small
/// allocations, once): a plugin's state is its last snapshot, never a
/// round trip.
///
/// ```text
/// mpfree-instruments 1
/// track 2 mpfree.fm
/// param 0 5
/// track 4 clap:org.example.synth file=synth.clap
/// param 0 0.5
/// state AAECAw==
/// ```
pub fn instruments_text(inst: &crate::instruments::Instruments) -> String {
    let mut s = String::with_capacity(1024);
    s.push_str(INSTRUMENTS_FORMAT);
    s.push('\n');
    for t in 0..crate::instruments::TRACKS {
        let Some(node) = inst.node(t) else {
            continue;
        };
        let _ = write!(s, "track {} {}", t + 1, node.info().id);
        for (k, v) in node.save_extras() {
            if word_ok(&k) && word_ok(&v) {
                let _ = write!(s, " {k}={v}");
            }
        }
        s.push('\n');
        for p in node.params() {
            if !p.readonly {
                let _ = writeln!(s, "param {} {}", p.id, node.param(p.id));
            }
        }
        if let Some(state) = node.save_state() {
            let _ = writeln!(s, "state {}", base64(&state));
        }
    }
    s
}

/// Reads the instruments part and makes each track's instrument (an
/// engine, or a plugin through the rack in its own process), ready to go
/// in: allocates and waits, so not on the audio thread. An instrument that
/// can't be made is left out with a line saying so, and its track plays
/// the kit. Gives the instruments and the lines.
pub fn instruments_load(
    text: &str,
    rate: u32,
    plugins: &crate::plugins::Plugins,
) -> Result<(Vec<Option<Loaded>>, Vec<String>), String> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, l)) if l.trim() == INSTRUMENTS_FORMAT => {}
        _ => return Err(format!("not an {INSTRUMENTS_FORMAT} part")),
    }
    let mut notes = Vec::new();
    // A track, its node and its plugin instance (0 for an engine).
    type Made = (usize, Box<dyn Node>, u64);
    let mut made: Vec<Made> = Vec::new();
    // The track being read: Some(None) while it is left out (its params
    // and state are skipped too).
    let mut open: Option<Option<Made>> = None;
    for (i, raw) in lines {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = || format!("line {}: {line}", i + 1);
        let mut w = line.split_whitespace();
        match w.next() {
            Some("track") => {
                if let Some(Some(m)) = open.take() {
                    made.push(m);
                }
                let n: usize = w.next().and_then(|x| x.parse().ok()).ok_or_else(bad)?;
                let id = w.next().ok_or_else(bad)?;
                let mut extras = Vec::new();
                for kv in w {
                    let (k, v) = kv.split_once('=').ok_or_else(bad)?;
                    extras.push((k.to_string(), v.to_string()));
                }
                let track = n
                    .checked_sub(1)
                    .filter(|t| *t < crate::instruments::TRACKS && !made.iter().any(|m| m.0 == *t));
                open = Some(match track {
                    None => {
                        notes.push(format!(
                            "project: track {n}'s instrument is left out (no such track, or named twice)"
                        ));
                        None
                    }
                    // Not even made: the most at once already are.
                    Some(_) if made.len() >= crate::instruments::SLOTS => {
                        notes.push(format!(
                            "project: track {n}'s instrument {id} is left out: {} tracks already have their own, the most at once; the track plays the kit",
                            crate::instruments::SLOTS
                        ));
                        None
                    }
                    Some(t) => match make_instrument_node(id, &extras, rate, plugins) {
                        Ok((node, instance)) => Some((t, node, instance)),
                        Err(e) => {
                            notes.push(format!(
                                "project: track {n}'s instrument {id} is left out: {e}; the track plays the kit"
                            ));
                            None
                        }
                    },
                });
            }
            Some("param") => {
                let Some(o) = open.as_mut() else {
                    return Err(bad());
                };
                let id: u16 = w.next().and_then(|x| x.parse().ok()).ok_or_else(bad)?;
                let v: f32 = w.next().and_then(|x| x.parse().ok()).ok_or_else(bad)?;
                if let Some((_, node, _)) = o.as_mut() {
                    if v.is_finite() && node.params().iter().any(|p| p.id == id && !p.readonly) {
                        node.set_param(id, v);
                    }
                }
            }
            Some("state") => {
                let Some(o) = open.as_mut() else {
                    return Err(bad());
                };
                let bytes = unbase64(w.next().unwrap_or("")).ok_or_else(bad)?;
                if let Some((t, node, _)) = o.as_mut() {
                    if let Err(e) = node.load_state(&bytes) {
                        notes.push(format!(
                            "project: track {}'s instrument {} didn't take its state ({e})",
                            *t + 1,
                            node.info().id
                        ));
                    }
                }
            }
            // A newer build's line.
            _ => {}
        }
    }
    if let Some(Some(m)) = open.take() {
        made.push(m);
    }
    let slots = made
        .into_iter()
        .map(|(track, node, instance)| {
            let held = crate::instruments::Held {
                id: node.info().id,
                name: node.info().name,
                instance,
            };
            Some(Loaded {
                track,
                instrument: mpfree_track::Instrument::new(node, rate, PERIOD),
                held,
            })
        })
        .collect();
    Ok((slots, notes))
}

/// A track's instrument a project names: an engine at its defaults (the
/// part's params follow) or a plugin through the rack, with its instance
/// there (0 for an engine), which its presets load into.
fn make_instrument_node(
    id: &str,
    extras: &[(String, String)],
    rate: u32,
    plugins: &crate::plugins::Plugins,
) -> Result<(Box<dyn Node>, u64), String> {
    if crate::plugins::Plugins::is_plugin(id) {
        check_plugin(id, extras, plugins)?;
        let (instance, node) = plugins.make_instance(id)?;
        return Ok((node, instance));
    }
    crate::synth::engine_node(id, rate)
        .map(|n| (n, 0))
        .ok_or_else(|| "this build can't make it".to_string())
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for k in 0..4 {
            if k <= c.len() {
                s.push(B64[(n >> (18 - 6 * k)) as usize & 63] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

pub fn unbase64(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = B64.iter().position(|&x| x == c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

// ------------------------------------------------------------ the synth

/// The synth part: the Synth tab's options as `settings/synth.conf` has
/// them, then each engine's sound as a preset.
///
/// ```text
/// mpfree-synth 1
/// [options]
/// engine = fm
/// scale = off
/// [sound Poly]
/// mpfree-preset 1
/// ...
/// ```
pub fn synth_text(options: &mpfree_store::Settings, sounds: &[(Engine, String)]) -> String {
    let mut s = format!("{SYNTH_FORMAT}\n[options]\n");
    s.push_str(&options.to_text());
    for (e, text) in sounds {
        let _ = writeln!(s, "[sound {}]", e.name());
        for line in text.lines() {
            // A section line inside a preset can't be told apart: never
            // made by the preset writer, dropped if it ever is.
            if !line.starts_with('[') {
                s.push_str(line);
                s.push('\n');
            }
        }
    }
    s
}

pub fn synth_parse(text: &str) -> Result<(mpfree_store::Settings, Vec<(Engine, String)>), String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(SYNTH_FORMAT) {
        return Err(format!("not an {SYNTH_FORMAT} part"));
    }
    let mut options = String::new();
    let mut sounds: Vec<(Engine, String)> = Vec::new();
    // None: options; Some(Some(i)): sound i; Some(None): an unknown section.
    let mut at: Option<Option<usize>> = None;
    for line in lines {
        if let Some(head) = line
            .trim()
            .strip_prefix('[')
            .and_then(|l| l.strip_suffix(']'))
        {
            at = if head == "options" {
                None
            } else if let Some(name) = head.strip_prefix("sound ") {
                match Engine::ALL.iter().find(|e| e.name() == name) {
                    Some(&e) => {
                        sounds.push((e, String::new()));
                        Some(Some(sounds.len() - 1))
                    }
                    None => Some(None),
                }
            } else {
                Some(None)
            };
            continue;
        }
        match at {
            None => {
                options.push_str(line);
                options.push('\n');
            }
            Some(Some(i)) => {
                sounds[i].1.push_str(line);
                sounds[i].1.push('\n');
            }
            Some(None) => {}
        }
    }
    Ok((mpfree_store::Settings::parse(&options), sounds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixing::MixHost;

    #[test]
    fn base64_round_trips() {
        for n in 0..20 {
            let b: Vec<u8> = (0..n).map(|k| (k * 37 + 11) as u8).collect();
            assert_eq!(unbase64(&base64(&b)).unwrap(), b);
        }
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(unbase64("TW!u"), None);
    }

    #[test]
    fn strips_read_back() {
        for s in [
            StripId::Input(0),
            StripId::Pad(15),
            StripId::Track(2),
            StripId::Bus(1),
            StripId::Return(0),
            StripId::Master,
        ] {
            assert_eq!(parse_strip(&strip_text(s)), Some(s));
        }
        assert_eq!(parse_strip("t"), None);
        assert_eq!(parse_strip("tx1"), None);
        assert_eq!(parse_strip("ps-1"), None);
    }

    #[test]
    fn the_mixer_part_reads_back_into_the_same_mixer() {
        let mut a = MixHost::new(44_100, -6.0, 0.0);
        a.mixer.set_now(StripId::Track(1), mx::VOLUME, -7.5);
        a.mixer.set_now(StripId::Track(0), mx::PAN, 0.25);
        a.mixer.set_now(StripId::Master, mx::VOLUME, -20.0);
        a.add_insert(StripId::Track(1), "mpfree.fx.drive").unwrap();
        a.mixer.set_now(StripId::Track(1), mx::MUTE, 1.0);
        // The inserts go in with the next periods.
        for _ in 0..50 {
            a.run_silent_period();
        }
        let text = mixer_text(&a.mixer, 0b101);
        let load = mixer_load(
            &text,
            Some("x = 1\n"),
            44_100,
            &crate::plugins::Plugins::none(),
        )
        .unwrap();
        assert_eq!(load.pad_mutes, 0b101);
        assert!(load.notes.is_empty(), "{:?}", load.notes);
        assert_eq!(load.modu.as_deref(), Some("x = 1\n"));
        // The master fader (the Level) isn't the project's.
        let master = load
            .strips
            .iter()
            .find(|(s, _)| *s == StripId::Master)
            .unwrap();
        assert!(!master.1.iter().any(|(id, _)| *id == mx::VOLUME));
        let t1 = load
            .strips
            .iter()
            .find(|(s, _)| *s == StripId::Track(1))
            .unwrap();
        assert!(t1.1.contains(&(mx::VOLUME, -7.5)));
        assert!(t1.1.contains(&(mx::MUTE, 1.0)));
        // Every insert came back in its place, with its id and parameters.
        let chain = |l: &MixLoad, s: StripId| -> Vec<(&'static str, Option<u16>)> {
            l.chains
                .iter()
                .find(|(x, _)| *x == s)
                .map(|(_, v)| {
                    v.iter()
                        .map(|(i, _)| (i.node().info().id, i.uid()))
                        .collect()
                })
                .unwrap_or_default()
        };
        let t1_chain = chain(&load, StripId::Track(1));
        assert_eq!(t1_chain.len(), 1);
        assert_eq!(t1_chain[0].0, "mpfree.fx.drive");
        assert_eq!(chain(&load, StripId::Track(0))[0].0, "mpfree.fx.dj_filter");
        // Opening it into a fresh mixer gives the same text back (but the
        // Level, which stays the room's).
        let mut b = MixHost::new(44_100, -6.0, 0.0);
        b.open(Box::new(load));
        for _ in 0..400 {
            b.tend_open();
            b.run_silent_period();
        }
        assert!(!b.opening());
        let again = mixer_text(&b.mixer, b.pad_mutes());
        let strip_master = |t: &str| {
            t.lines()
                .filter(|l| !l.starts_with("strip master"))
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        assert_eq!(strip_master(&again), strip_master(&text));
        assert_eq!(b.mixer.param(StripId::Master, mx::VOLUME), -6.0);
    }

    #[test]
    fn a_tracks_own_channel_is_saved_only_once_something_is_set_on_it() {
        let own = |t: usize| StripId::Track(crate::instruments::first_track() + t);
        let mut a = MixHost::new(44_100, -6.0, 0.0);
        let text = mixer_text(&a.mixer, 0);
        assert!(
            text.contains("strip t0 "),
            "the engines' tracks are always saved"
        );
        assert!(
            !text.contains(&format!("strip {} ", strip_text(own(0)))),
            "{text}"
        );
        a.mixer.set_now(own(2), mx::VOLUME, -9.0);
        let text = mixer_text(&a.mixer, 0);
        assert!(
            text.contains(&format!("strip {} 0=-9", strip_text(own(2)))),
            "{text}"
        );
        assert!(!text.contains(&format!("strip {} ", strip_text(own(1)))));
        // A project that leaves a channel out opens with it at its
        // defaults, whatever the last project had there.
        let mut b = MixHost::new(44_100, -6.0, 0.0);
        b.mixer.set_now(own(5), mx::PAN, -40.0);
        b.mixer.set_now(own(5), mx::MUTE, 1.0);
        let load = mixer_load(&text, None, 44_100, &crate::plugins::Plugins::none()).unwrap();
        b.open(Box::new(load));
        for _ in 0..400 {
            b.tend_open();
            b.run_silent_period();
        }
        assert!(!b.opening());
        assert_eq!(b.mixer.param(own(2), mx::VOLUME), -9.0);
        assert_eq!(b.mixer.param(own(5), mx::PAN), 0.0);
        assert_eq!(b.mixer.param(own(5), mx::MUTE), 0.0);
        assert!(untouched_channel(&b.mixer, own(5)));
    }

    #[test]
    fn an_effect_this_build_cannot_make_is_left_out_and_said() {
        let text = format!(
            "{MIXER_FORMAT}\ninsert t0 3 1 clap:com.example.nothing file=x.clap\nparam 0 1\nstate AAEC\ninsert t1 4 0 mpfree.fx.drive\nparam 0 3\nsparkle 9\n"
        );
        let load = mixer_load(&text, None, 44_100, &crate::plugins::Plugins::none()).unwrap();
        assert_eq!(load.notes.len(), 1);
        assert!(load.notes[0].contains("clap:com.example.nothing"));
        assert_eq!(load.chains.len(), 1);
        let (s, v) = &load.chains[0];
        assert_eq!(*s, StripId::Track(1));
        assert!(!v[0].0.is_on());
        assert_eq!(v[0].0.uid(), Some(4));
        assert!(mixer_load(
            "mpfree-mixer 9\n",
            None,
            44_100,
            &crate::plugins::Plugins::none()
        )
        .is_err());
        assert!(mixer_load(
            &format!("{MIXER_FORMAT}\nparam 0 1\n"),
            None,
            44_100,
            &crate::plugins::Plugins::none()
        )
        .is_err());
        assert!(mixer_load(
            &format!("{MIXER_FORMAT}\nstrip zz 0=1\n"),
            None,
            44_100,
            &crate::plugins::Plugins::none()
        )
        .is_err());
    }

    #[test]
    fn loop_layers_are_named_for_their_audio_and_a_missing_one_is_said() {
        use mpfree_looper::save::{LayerFile, LoopFile, Saved};
        let layer = |audio: Vec<f32>| LayerFile {
            level: 1.0,
            pan: 0.0,
            muted: false,
            solo: false,
            faded: 1.0,
            file: String::new(),
            audio,
        };
        let one = |n: usize, layers: Vec<LayerFile>| {
            (
                n,
                LoopFile {
                    frames: 2,
                    ticks: 96,
                    origin: 0,
                    level: 1.0,
                    muted: false,
                    bars: None,
                    after: mpfree_looper::After::Play,
                    fade_old: 1.0,
                    layers,
                },
            )
        };
        let saved = Saved {
            rate: 44_100,
            selected: 0,
            loops: vec![
                // The same audio twice is one file; a layer read longer
                // than the loop is cut to it.
                one(
                    0,
                    vec![
                        layer(vec![0.1, 0.2, 0.3, 0.4]),
                        layer(vec![0.1, 0.2, 0.3, 0.4, 9.0]),
                    ],
                ),
                one(3, vec![layer(vec![0.5; 4])]),
            ],
        };
        let (text, files) = loops_part(saved.clone());
        assert_eq!(files.len(), 2);
        assert!(files
            .iter()
            .all(|(f, a)| f.starts_with("loop-") && f.len() == 41 && a.len() == 4));
        assert_ne!(files[0].0, files[1].0);
        // Named the same each time.
        assert_eq!(loops_part(saved).1[0].0, files[0].0);
        let read = |f: &str| {
            files
                .iter()
                .find(|(n, _)| n == f)
                .map(|(_, a)| mpfree_looper::save::wav(a, 44_100))
                .ok_or_else(|| "not there".to_string())
        };
        let (back, notes) = loops_open(&text, read).unwrap();
        assert!(notes.is_empty());
        assert_eq!(back.loops.len(), 2);
        assert_eq!(back.loops[1].1.layers[0].audio, vec![0.5; 4]);
        // Loop 4's file gone: it is left out, and said.
        let (back, notes) = loops_open(&text, |f| {
            if f == files[1].0 {
                Err("gone".into())
            } else {
                read(f)
            }
        })
        .unwrap();
        assert_eq!(back.loops.len(), 1);
        assert!(
            notes[0].starts_with("project: loop 4 wasn't opened"),
            "{notes:?}"
        );
        assert!(loops_open("nope", read).is_err());
    }

    /// Two tracks with their own instruments, one with a sound of its
    /// own, written and read back: the same tracks, engines and sounds.
    #[test]
    fn the_tracks_instruments_read_back() {
        use crate::instruments::{make, Instruments};
        use crate::synth::Engine;
        let rate = 44_100;
        let plugins = crate::plugins::Plugins::none();
        let synth = crate::synth::Synth::new(rate);
        let mut inst = Instruments::new(rate, PERIOD);
        assert_eq!(instruments_text(&inst), format!("{INSTRUMENTS_FORMAT}\n"));
        let (fm, mut values) = synth.sound_of(Engine::Fm);
        let params = crate::synth::engine_node(fm, rate).unwrap().params();
        let k = params
            .iter()
            .position(|p| !p.readonly && p.max > p.min)
            .unwrap();
        values[k] = if values[k] == params[k].max {
            params[k].min
        } else {
            params[k].max
        };
        let made = make(&plugins, fm, &values, rate, PERIOD).unwrap();
        let tuning = synth.tuning_now();
        assert!(inst
            .put(1, made, synth.pads_for(Engine::Fm), &tuning)
            .is_ok());
        let (poly, pv) = synth.sound_of(Engine::Poly);
        let made = make(&plugins, poly, &pv, rate, PERIOD).unwrap();
        assert!(inst
            .put(3, made, synth.pads_for(Engine::Poly), &tuning)
            .is_ok());
        let text = instruments_text(&inst);
        assert!(text.contains("\ntrack 2 mpfree.fm\n"), "{text}");
        assert!(text.contains("\ntrack 4 mpfree.poly\n"), "{text}");

        let (slots, notes) = instruments_load(&text, rate, &plugins).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let back: Vec<&Loaded> = slots.iter().flatten().collect();
        assert_eq!(back.iter().map(|l| l.track).collect::<Vec<_>>(), [1, 3]);
        for l in back {
            assert_eq!(Some(l.held), inst.held(l.track));
            let (was, now) = (inst.node(l.track).unwrap(), l.instrument.node());
            for p in now.params().iter().filter(|p| !p.readonly) {
                assert_eq!(now.param(p.id), was.param(p.id), "{}", p.name);
            }
        }
        let fm_back = slots[0].as_ref().unwrap().instrument.node();
        assert_eq!(fm_back.param(params[k].id), values[k], "its own sound");
    }

    /// A missing plugin, a track that isn't there or is named twice, and
    /// an engine this build hasn't got are each left out and said; the
    /// rest still open.
    #[test]
    fn a_tracks_instrument_that_cannot_be_made_is_left_out_and_said() {
        let plugins = crate::plugins::Plugins::none();
        // One past the last track (numbered from 1, as the part names them).
        let past = crate::instruments::TRACKS + 1;
        let text = format!(
            "{INSTRUMENTS_FORMAT}\n\
             track 1 clap:com.example.synth file=synth.clap\nparam 0 1\nstate AAEC\n\
             track {past} mpfree.poly\nparam 0 1\n\
             track 2 mpfree.poly\nparam 9999 1\nsparkle 3\n\
             track 2 mpfree.fm\n\
             track 3 mpfree.theremin\n"
        );
        let (slots, notes) = instruments_load(&text, 44_100, &plugins).unwrap();
        let back: Vec<usize> = slots.iter().flatten().map(|l| l.track).collect();
        assert_eq!(back, [1], "only track 2's Poly");
        assert_eq!(slots[0].as_ref().unwrap().held.id, "mpfree.poly");
        assert_eq!(notes.len(), 4, "{notes:?}");
        assert!(
            notes[0].contains("clap:com.example.synth")
                && notes[0].contains("(synth.clap) isn't in files/plugins/")
                && notes[0].ends_with("the track plays the kit"),
            "{notes:?}"
        );
        assert!(notes[1].contains(&format!("track {past}")), "{notes:?}");
        assert!(notes[2].contains("track 2") && notes[2].contains("named twice"));
        assert!(notes[3].contains("mpfree.theremin") && notes[3].contains("can't make it"));

        assert!(instruments_load("mpfree-instruments 9\n", 44_100, &plugins).is_err());
        assert!(
            instruments_load(
                &format!("{INSTRUMENTS_FORMAT}\nparam 0 1\n"),
                44_100,
                &plugins
            )
            .is_err(),
            "a param before any track"
        );
        assert!(instruments_load(
            &format!("{INSTRUMENTS_FORMAT}\ntrack 1 mpfree.fm\nstate !!\n"),
            44_100,
            &plugins
        )
        .is_err());
    }

    /// A project saved before the instruments part, or with no track's
    /// own instrument, has none: every track plays the kit.
    #[test]
    fn no_instruments_part_means_every_track_plays_the_kit() {
        let c = Collected {
            tempo: 120.0,
            swing: mpfree_transport::time::Swing::new(240, 50).unwrap(),
            clips: mpfree_seq::Project::default(),
            mixer: String::new(),
            modu: String::new(),
            synth: String::new(),
            arp: String::new(),
            instruments: format!("{INSTRUMENTS_FORMAT}\n"),
            loops: None,
            audio: None,
        };
        let snap = snapshot(&c, Vec::new());
        assert!(snap.part(INSTRUMENTS).is_none(), "not written while empty");
        let o = opening(&snap, 44_100, "a test", &crate::plugins::Plugins::none());
        assert!(o.instruments.is_empty());
    }

    #[test]
    fn audio_takes_are_named_for_their_audio_and_a_missing_one_is_said() {
        use mpfree_audiotracks::save::{Saved, TakeFile, TrackFile};
        use mpfree_audiotracks::{Input, Monitor, Region};
        let take = |id: u16, channels: usize, audio: Vec<f32>| TakeFile {
            id,
            track: 0,
            group: 1,
            pass: id,
            bpm: 120.0,
            anchor: 0.0,
            frames: audio.len() / channels,
            channels,
            file: String::new(),
            audio,
        };
        let region = |start, end, take| Region {
            start,
            end,
            take,
            src: 0,
            gain: 1.0,
            fade_in: 0,
            fade_out: 0,
        };
        let a: Vec<f32> = (0..400).map(|k| k as f32 / 400.0).collect();
        let saved = Saved {
            rate: 44_100,
            tracks: vec![TrackFile {
                index: 0,
                input: Input::Stereo,
                monitor: Monitor::In,
                level: 0.8,
                pan: 0.25,
                muted: false,
                solo: false,
                regions: vec![region(0, 960, 0), region(960, 1920, 2)],
            }],
            // Take 1 has take 0's samples (one file for both); take 2 the
            // same samples as stereo (a sound of its own).
            takes: vec![
                take(0, 1, a.clone()),
                take(1, 1, a.clone()),
                take(2, 2, a.clone()),
            ],
        };
        let (text, files) = audio_part(saved);
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|(f, _, _)| f.starts_with("take-")));
        assert_ne!(files[0].0, files[1].0);
        assert_eq!((files[1].1.len(), files[1].2), (400, 2));
        let wavs: Vec<(String, Vec<u8>)> = files
            .iter()
            .map(|(f, audio, ch)| {
                let mut w = Vec::new();
                mpfree_audiotracks::save::write_wav(&mut w, audio, *ch, 44_100).unwrap();
                (f.clone(), w)
            })
            .collect();
        let read = |f: &str| {
            wavs.iter()
                .find(|(n, _)| n == f)
                .map(|(_, b)| b.clone())
                .ok_or_else(|| "not there".to_string())
        };
        let (back, notes) = audio_open(&text, 44_100, read).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(back.takes.len(), 3);
        assert_eq!(back.takes[1].audio, a);
        assert_eq!(back.takes[2].channels, 2);
        assert_eq!(back.takes[2].audio, a);
        assert_eq!(back.tracks[0].input, Input::Stereo);
        assert_eq!(back.tracks[0].regions.len(), 2);
        // Take 2's file gone: it is left out with its region, and said;
        // another rate is said too.
        let (back, notes) = audio_open(&text, 48_000, |f| {
            if f == files[1].0 {
                Err("gone".into())
            } else {
                read(f)
            }
        })
        .unwrap();
        assert_eq!(back.takes.len(), 2);
        assert_eq!(back.tracks[0].regions, vec![region(0, 960, 0)]);
        assert!(notes[0].contains("44100 Hz"), "{notes:?}");
        assert!(
            notes[1].starts_with("project: a take of audio 1 wasn't opened"),
            "{notes:?}"
        );
        // A file that isn't the take's sound (another length) is refused.
        let short = mpfree_audiotracks::save::wav(&a[..100], 1, 44_100);
        let (back, notes) = audio_open(&text, 44_100, |f| {
            if f == files[0].0 {
                Ok(short.clone())
            } else {
                read(f)
            }
        })
        .unwrap();
        assert_eq!(back.takes.len(), 1);
        assert!(notes.iter().all(|n| n.contains("not this take's sound")));
        assert!(audio_open("nope", 44_100, read).is_err());
    }

    #[test]
    fn the_synth_part_reads_back() {
        let synth = crate::synth::Synth::new(44_100);
        let mut opts = mpfree_store::Settings::default();
        opts.set_str("engine", "fm");
        let sounds = synth.sounds();
        let text = synth_text(&opts, &sounds);
        let (o, s) = synth_parse(&text).unwrap();
        assert_eq!(o.get_str("engine"), Some("fm"));
        assert_eq!(s, sounds);
        let mut other = crate::synth::Synth::new(44_100);
        assert!(other.set_sounds(&s).is_empty());
        assert_eq!(other.sounds(), sounds);
        assert!(synth_parse("nope").is_err());
    }
}
