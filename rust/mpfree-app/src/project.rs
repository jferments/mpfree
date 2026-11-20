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
//! | `track-kits` | what each track plays (`sampler::TrackKits`) | `TrackKits::text`: the tracks that play another kit than kit 1, or an instrument by its id | [`track_kits_parse`]; the audio loop sets them |
//! | `instrument-list` | the project's instruments ([`InstrumentList`]) | [`InstrumentList::text`]: each instrument's lasting id and what it is, and the next id | [`InstrumentList::parse`] on the screen thread; the audio loop swaps it in |
//! | `mod` | the mod engine | `ModMix::save` | `ModMix::load`, once the inserts are in |
//! | `synth` | the synth | [`synth_text`]: the Synth tab's options and every engine's sound | the options go through the app (as at start), the sounds onto the engines |
//! | `sampler` | the Sample tab | `SamplePage::save_part`, with its sounds in `samples/` | `SamplePage::open_part` |
//! | `midi` | the I/O view | `mpfree_midi_ui::MidiPart` | the same |
//! | `looper` | the Looper | [`loops_part`], a WAV a layer in `samples/` | [`loops_open`] on a thread, then `LoopHost::open` |
//! | `arp` | the arps | `App::arp_part` | `App::open_arp` |
//! | `perform` | the Perform set (`perform_pads.rs`) | `PerformSet::text` | `PerformSet::parse`, then `App::set_perform_set` |
//! | `pads` | what the pads play (`app/pads_focus.rs`): the selected track's instrument, or one chosen for them | `App::pads_part` | `App::open_pads`; none (New, or saved before it) follows the track |
//! | `audiotracks` | the audio tracks | [`audio_part`]: each track's settings and regions, a WAV a take in `samples/` | [`audio_open`] on a thread, then [`crate::audiotracks::TrackHost::open`] |
//!
//! The project's tempo is `project.txt`'s `tempo`, and its format
//! ([`FORMAT_KEY`]) says what the parts mean together: 2 since
//! instruments have ids; a project with no format line is from before,
//! and opens as it played ([`opening`]).
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
use mpfree_instruments::sound::is_design_key;
use mpfree_library::Snapshot;
use mpfree_node::Node;

use crate::instrument_list::{InstrumentId, InstrumentList, Kind};
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
/// The Perform set: its pages and what each pad does
/// (`perform_pads::PerformSet::text`).
pub const PERFORM: &str = "perform";
/// What each track plays: a kit or an instrument of the project's list
/// (`sampler::TrackKits::text`).
pub const TRACK_KITS: &str = "track-kits";
/// The project's instruments, each with its lasting id
/// ([`InstrumentList::text`]).
pub const INSTRUMENT_LIST: &str = "instrument-list";
/// What the pads play: the selected track's instrument, or one chosen for
/// them (`App::pads_part`, the instrument redesign's step 3).
pub const PADS: &str = "pads";
/// The Song view's audio tracks and their takes ([`audio_part`]).
pub const AUDIOTRACKS: &str = "audiotracks";

/// The mixer part's first line.
pub const MIXER_FORMAT: &str = "mpfree-mixer 1";
/// The synth part's first line.
pub const SYNTH_FORMAT: &str = "mpfree-synth 1";
/// The first line of the instruments part.
/// Its version follows the preset format's ([`mpfree_node::preset::VERSION`]):
/// a part of version N holds settings in that preset format's meaning, so
/// a part from before a setting changed meaning (1: Poly's envelope times
/// and velocity) is converted as it is read ([`mpfree_node::Node::upgrade`]).
pub const INSTRUMENTS_FORMAT: &str = "mpfree-instruments 2";
/// The words before the version on the first line.
const INSTRUMENTS_WORD: &str = "mpfree-instruments";

/// The version of an instruments part's first line, if this build reads
/// it (1 to [`mpfree_node::preset::VERSION`]).
fn instruments_version(line: &str) -> Option<u32> {
    let v: u32 = line
        .trim()
        .strip_prefix(INSTRUMENTS_WORD)?
        .trim()
        .parse()
        .ok()?;
    (1..=mpfree_node::preset::VERSION).contains(&v).then_some(v)
}
/// The first line of the track kits part. Version 2 names an instrument
/// by its id (`track 4 instrument 1`); version 1, from before instruments
/// had ids, said `track 4 synth` for the shared synth, and is still read.
pub const TRACK_KITS_FORMAT: &str = "mpfree-track-kits 2";
/// The track kits part's first line before instruments had ids.
const TRACK_KITS_V1: &str = "mpfree-track-kits 1";

/// `project.txt`'s line for what the parts mean together: `format = 2`.
/// No line is format 1: saved before instruments had ids (the instrument
/// redesign's step 2a), with no list of instruments and the track kits
/// part's version 1. The library's own first line (`mpfree-project 1`) is
/// the folder's format; this one is the app's.
pub const FORMAT_KEY: &str = "format";
/// The project format this build writes, and the newest it knows.
pub const FORMAT: u32 = 2;

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
    /// The Perform set ([`crate::perform_pads::PerformSet::text`]).
    pub perform: String,
    /// Each track's own instrument ([`instruments_text`]).
    pub instruments: String,
    /// What each track plays (`sampler::TrackKits::text`).
    pub track_kits: String,
    /// The project's instruments ([`InstrumentList::text`]).
    pub instrument_list: String,
    /// What the pads play ([`crate::app::App::pads_part`]); not saved
    /// when empty.
    pub pads: String,
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
    /// The Perform set's part; None (New, or a project saved before it
    /// had one) gives the new project's set: the loop keys on page A,
    /// Play's throws on B.
    pub perform: Option<String>,
    /// Each track's own instrument, made here. Every track not in it
    /// (all of them for New, or a project saved before the part) plays
    /// the kit.
    pub instruments: Vec<Option<Loaded>>,
    /// What the tracks play other than kit 1 ([`track_kits_parse`]): a
    /// track (from 0) and its kit (from 0), or None for the shared synth.
    /// Every track not in it (all of them for New, or a project saved
    /// before the part) plays kit 1.
    pub track_kits: Vec<(usize, Option<usize>)>,
    /// The project's instruments: the audio loop takes it in place of its
    /// own, which comes back in the box to be freed on the screen thread.
    pub instrument_list: InstrumentList,
    /// What the pads play (the pads part); None (New, or a project saved
    /// before it had one): they follow the selected track.
    pub pads: Option<String>,
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
        (PERFORM.to_string(), c.perform.clone()),
    ];
    if c.instruments.lines().count() > 1 {
        parts.push((INSTRUMENTS.to_string(), c.instruments.clone()));
    }
    if c.track_kits.lines().count() > 1 {
        parts.push((TRACK_KITS.to_string(), c.track_kits.clone()));
    }
    if !c.instrument_list.is_empty() {
        parts.push((INSTRUMENT_LIST.to_string(), c.instrument_list.clone()));
    }
    if !c.pads.is_empty() {
        parts.push((PADS.to_string(), c.pads.clone()));
    }
    parts.extend(extra);
    Snapshot {
        meta: mpfree_library::Meta {
            tempo: c.tempo,
            app: format!("mpfree {}", env!("CARGO_PKG_VERSION")),
            extra: vec![
                (FORMAT_KEY.to_string(), FORMAT.to_string()),
                (SWING.to_string(), swing_text(c.swing)),
            ],
            ..Default::default()
        },
        parts,
    }
}

/// The project's format ([`FORMAT_KEY`]): 1 with no line. A line that
/// doesn't read, or a format newer than this build's, is said in `notes`
/// and read as this build's: what it can't read is left out and said
/// part by part.
fn project_format(meta: &mpfree_library::Meta, notes: &mut Vec<String>) -> u32 {
    let Some((_, v)) = meta.extra.iter().find(|(k, _)| k == FORMAT_KEY) else {
        return 1;
    };
    match v.parse::<u32>() {
        Ok(f) if (1..=FORMAT).contains(&f) => f,
        Ok(f) if f > FORMAT => {
            notes.push(format!(
                "project: saved by a newer mpfree (format {f}; this one reads {FORMAT}): what it can't read is left out and said"
            ));
            FORMAT
        }
        _ => {
            notes.push(format!(
                "project: its format line (\"{v}\") doesn't read; it opens as format {FORMAT}"
            ));
            FORMAT
        }
    }
}

/// The project's list of instruments. A project saved before instruments
/// had ids (format 1) has none: its shared synth is instrument 1, as in a
/// new project. A list that is missing from a newer project, or doesn't
/// read, is said, and the new project's list is used.
fn instrument_list(snap: &Snapshot, format: u32, notes: &mut Vec<String>) -> InstrumentList {
    match snap.part(INSTRUMENT_LIST) {
        None if format < 2 => InstrumentList::new(),
        None => {
            notes.push(
                "project: its list of instruments is missing; the synth is instrument 1"
                    .to_string(),
            );
            InstrumentList::new()
        }
        Some(text) => InstrumentList::parse(text).unwrap_or_else(|e| {
            notes.push(format!(
                "project: its list of instruments wasn't opened ({e}); the synth is instrument 1"
            ));
            InstrumentList::new()
        }),
    }
}

/// The tracks' own instruments in the list (step 2b): each track the
/// instruments part gave one (`held`, a bit each) keeps the id the list
/// gives it, or gets a new one: silently in a project saved before
/// instruments had ids, said in a newer one. An id whose track has no
/// instrument (it wasn't in the part, or couldn't be made) is gone from
/// the list, said.
fn own_ids(list: &mut InstrumentList, held: u128, format: u32, notes: &mut Vec<String>) {
    let tracks = 0..crate::instrument_list::TRACKS;
    let said = format >= 2;
    let mut missing = 0u128;
    for t in tracks.clone() {
        match (held >> t & 1 == 1, list.own_of(t)) {
            (true, None) => missing |= 1 << t,
            (false, Some(id)) if said => notes.push(format!(
                "project: instrument {id} (track {}'s own) isn't there, so it is gone from the list",
                t + 1
            )),
            _ => {}
        }
    }
    list.follow(held);
    for t in tracks.filter(|&t| said && missing >> t & 1 == 1) {
        if let Some(id) = list.own_of(t) {
            notes.push(format!(
                "project: track {}'s own instrument had no id in the list; it is instrument {id} now",
                t + 1
            ));
        }
    }
}

/// What the tracks play, from the track kits part's picks: a kit as it
/// is, and an instrument by its id, which is the shared synth (None) when
/// the list has it. A track whose instrument the list hasn't got is left
/// out, so it plays kit 1, and said.
fn tracks_play(
    picks: Vec<(usize, TrackPick)>,
    list: &InstrumentList,
    notes: &mut Vec<String>,
) -> Vec<(usize, Option<usize>)> {
    let mut out = Vec::with_capacity(picks.len());
    for (track, pick) in picks {
        match pick {
            TrackPick::Kit(k) => out.push((track, Some(k))),
            TrackPick::Instrument(id) => match list.kind(id) {
                Some(Kind::SharedSynth) => out.push((track, None)),
                Some(Kind::Own { track: of }) => notes.push(format!(
                    "project: track {} plays kit 1 (its instrument {id} is track {}'s own)",
                    track + 1,
                    of + 1
                )),
                None => notes.push(format!(
                    "project: track {} plays kit 1 (its instrument {id} isn't in the project's list)",
                    track + 1
                )),
            },
        }
    }
    out
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
    let mut note_fx = None;
    let format = project_format(&snap.meta, &mut notes);
    let since_4b = saved_since_4b(snap);
    let time = snap.part(CLIPS).and_then(|text| {
        let mut p = mpfree_seq::save::from_text(text)
            .map_err(|e| notes.push(format!("project: the clips weren't opened ({e})")))
            .ok()?;
        if !since_4b {
            let old = old_strip_targets(&mut p);
            if old != OldStripTargets::default() {
                notes.push(old.line());
            }
        }
        // Each track's lasting id (step 2c): a project saved before tracks
        // had them gets 1, 2, 3 ... silently; in a newer one (it says the
        // next id) a track without one, or sharing one, is said.
        let had_ids = p.next_track != 0;
        let given = p.give_track_ids();
        if had_ids && given > 0 {
            notes.push(format!(
                "project: {given} of its tracks had no lasting id, or shared one; they have new ones"
            ));
        }
        let mut fx = Box::new([mpfree_notefx::TrackFx::default(); mpfree_seq::MAX_TRACKS]);
        for (t, tr) in p.tracks.iter().enumerate().take(mpfree_seq::MAX_TRACKS) {
            fx[t] = tr.note_fx;
        }
        note_fx = Some(fx);
        TimeHost::with_project(rate, tempo.unwrap_or(120.0), p)
            .map_err(|e| notes.push(format!("project: the clips weren't opened ({e})")))
            .ok()
            .map(Box::new)
    });
    let mut mix = snap.part(MIXER).and_then(|text| {
        mixer_load(text, snap.part(MOD), rate, plugins)
            .map_err(|e| notes.push(format!("project: the mixer wasn't opened ({e})")))
            .ok()
            .map(|mut m| {
                m.note_fx = note_fx;
                Box::new(m)
            })
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
    // Where track 1's own instrument played, in a project saved before
    // Drums was track 1's strip (`MixLoad::track1_own`).
    if let Some(m) = mix.as_mut() {
        m.track1_own = instruments.iter().flatten().any(|l| l.track == 0);
    }
    let mut instrument_list = instrument_list(snap, format, &mut notes);
    let held = instruments
        .iter()
        .flatten()
        .filter(|l| l.track < crate::instrument_list::TRACKS)
        .fold(0u128, |bits, l| bits | 1 << l.track);
    own_ids(&mut instrument_list, held, format, &mut notes);
    let track_kits = match snap.part(TRACK_KITS).map(track_kits_parse) {
        None => Vec::new(),
        Some(Ok(picks)) => tracks_play(picks, &instrument_list, &mut notes),
        Some(Err(e)) => {
            notes.push(format!(
                "project: what the tracks play wasn't opened ({e}); they play kit 1"
            ));
            Vec::new()
        }
    };
    Opening {
        tempo,
        swing,
        time,
        mix,
        synth,
        arp: snap.part(ARP).map(str::to_string),
        perform: snap.part(PERFORM).map(str::to_string),
        instruments,
        track_kits,
        instrument_list,
        pads: snap.part(PADS).map(str::to_string),
        notes,
        what: what.to_string(),
        clear_loops: snap.part(LOOPER).is_some(),
        clear_audio: snap.part(AUDIOTRACKS).is_some(),
    }
}

// ------------------------------------------------------------ what the tracks play

/// What a track plays, as the track kits part names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackPick {
    /// A kit, from 0.
    Kit(usize),
    /// An instrument of the project's list, by its id.
    Instrument(InstrumentId),
}

/// Reads the track kits part (`sampler::TrackKits::text`):
///
/// ```text
/// mpfree-track-kits 2
/// track 2 kit 3
/// track 4 instrument 1
/// ```
///
/// Tracks and kits are numbered from 1, as the screen shows them; gives
/// them from 0. A part of version 1 (saved before instruments had ids)
/// says `track 4 synth` for the shared synth, read as its id. A track or
/// kit past what there is now is the audio loop's to refuse, and an
/// instrument the list hasn't got the opener's ([`opening`]).
pub fn track_kits_parse(text: &str) -> Result<Vec<(usize, TrackPick)>, String> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let v1 = match lines.next() {
        Some(TRACK_KITS_FORMAT) => false,
        Some(TRACK_KITS_V1) => true,
        _ => return Err(format!("not a {TRACK_KITS_FORMAT} part")),
    };
    let mut picks = Vec::new();
    for line in lines {
        let w: Vec<&str> = line.split_whitespace().collect();
        let number = |s: &str| s.parse::<usize>().ok().filter(|&n| n >= 1).map(|n| n - 1);
        let instrument = |s: &str| s.parse().ok().and_then(InstrumentId::from_number);
        let pick = match w.as_slice() {
            ["track", t, "kit", k] => number(t).zip(number(k).map(TrackPick::Kit)),
            ["track", t, "synth"] if v1 => {
                number(t).map(|t| (t, TrackPick::Instrument(InstrumentId::SHARED_SYNTH)))
            }
            ["track", t, "instrument", i] if !v1 => {
                number(t).zip(instrument(i).map(TrackPick::Instrument))
            }
            _ => None,
        };
        match pick {
            Some(p) => picks.push(p),
            None => return Err(format!("can't read \"{line}\"")),
        }
    }
    Ok(picks)
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
            extra: vec![
                (FORMAT_KEY.to_string(), FORMAT.to_string()),
                (
                    SWING.to_string(),
                    swing_text(mpfree_transport::time::Swing::default()),
                ),
            ],
            ..Default::default()
        },
        parts: vec![
            (CLIPS.to_string(), mpfree_seq::save::to_text(&clips)),
            (MIXER.to_string(), d.mixer.clone()),
            (MOD.to_string(), d.modu.clone()),
            (SYNTH.to_string(), d.synth.clone()),
            (INSTRUMENT_LIST.to_string(), InstrumentList::new().text()),
        ],
    }
}

// ------------------------------------------------------------ old strip targets

/// Whether the project was saved since Drums became track 1's strip
/// (sampler R3 part 4b): its mixer part has a `track1` line
/// ([`mixer_text_lent`]).
fn saved_since_4b(snap: &Snapshot) -> bool {
    snap.part(MIXER)
        .is_some_and(|t| t.lines().any(|l| l.trim_start().starts_with("track1 ")))
}

/// What [`old_strip_targets`] did to a project saved before part 4b.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OldStripTargets {
    /// Lanes on tracks 2 and up that named the track's strip or one of its
    /// inserts, now naming that strip by number, so they move what they did.
    pub lanes: usize,
    /// Such lanes left as they were, because the clip already had a lane on
    /// that strip's parameter (they now move the track's own strip).
    pub clashes: usize,
    /// Step locks on tracks 2 and up on the track's strip: a lock can only
    /// name its own track's ([`mpfree_seq::Lock::addr`]). Most came from the
    /// step lanes, which meant the track's own strip, so they are kept and
    /// now reach it (the sequencer thread's call, 18 Nov).
    pub strip_locks: usize,
    /// Step locks on tracks 2 and up on an insert: only a knob on another
    /// strip's effect (Synth's, Loop's, Audio's) made those, and kept they
    /// would turn whatever effect has that slot id on the track's own
    /// strip, so they are dropped.
    pub effect_locks: usize,
}

impl OldStripTargets {
    /// The open's log line.
    pub fn line(&self) -> String {
        let mut parts = Vec::new();
        if self.lanes > 0 {
            parts.push(format!(
                "{} lanes on tracks 2 and up keep the strip they moved (Synth, Loop, Audio or another, now named by number)",
                self.lanes
            ));
        }
        if self.clashes > 0 {
            parts.push(format!(
                "{} such lanes clashed with a lane already there and now move their track's own strip",
                self.clashes
            ));
        }
        if self.strip_locks > 0 {
            parts.push(format!(
                "{} strip locks on tracks 2 and up now reach their track's own strip",
                self.strip_locks
            ));
        }
        if self.effect_locks > 0 {
            parts.push(format!(
                "{} effect locks on tracks 2 and up dropped: they turned another strip's effects (Synth, Loop or Audio), not this track's",
                self.effect_locks
            ));
        }
        format!(
            "project: saved before Drums was track 1's strip: {}",
            parts.join("; ")
        )
    }
}

/// Where a clip on track `t` reached with "this track's strip" (or an
/// insert on it) before sampler R3 part 4b: mixer track `t`, by number
/// (`OnTrack::on` used the track as the strip: Synth for track 2, Loop
/// for 3, Audio for 4). Track 1's reached Drums, as it does now.
fn old_strip_target(
    target: mpfree_seq::mpfree_moves::Target,
    t: u8,
) -> Option<mpfree_seq::mpfree_moves::Target> {
    use mpfree_seq::mpfree_moves::Target;
    use mpfree_seq::mpfree_param::{OnTrack, Owner, ParamAddr, Strip};
    let Target::Track(rel) = target else {
        return None;
    };
    let owner = match rel.on {
        OnTrack::Strip => Owner::Strip(Strip::Track(t)),
        OnTrack::Insert(uid) => Owner::Insert(Strip::Track(t), uid),
        _ => return None,
    };
    (t > 0).then_some(Target::Fixed(ParamAddr::new(owner, rel.param)))
}

/// A clip's lanes with the old strip targets of track `t` named by their
/// strip (`old_strip_target`): (rewritten, clashed). Allocates, once.
fn old_strip_lanes(m: &mut mpfree_seq::mpfree_moves::Moves, t: u8) -> (usize, usize) {
    if !m
        .lanes()
        .iter()
        .any(|l| old_strip_target(l.target, t).is_some())
    {
        return (0, 0);
    }
    let (mut moved, mut clashed) = (0, 0);
    let mut out = mpfree_seq::mpfree_moves::Moves::new();
    let taken: Vec<_> = m.lanes().iter().map(|l| l.target).collect();
    for l in m.lanes() {
        let target = match old_strip_target(l.target, t) {
            Some(fixed) if !taken.contains(&fixed) => {
                moved += 1;
                fixed
            }
            Some(_) => {
                clashed += 1;
                l.target
            }
            None => l.target,
        };
        // The lanes were checked as they were read; the same points under
        // another target fit as well.
        let _ = out
            .reserve(target, l.map, l.points.len())
            .and_then(|_| out.restore(target, l.map, 0, u32::MAX, &l.points));
    }
    *m = out;
    (moved, clashed)
}

/// A clip's step locks on track 2 and up (`old_strip_targets`): those on
/// the track's strip are kept and counted, those on an insert dropped and
/// counted.
fn old_strip_locks(locks: &mut Vec<mpfree_seq::Lock>, out: &mut OldStripTargets) {
    use mpfree_seq::mpfree_param::OnTrack;
    out.strip_locks += locks.iter().filter(|l| l.addr.on == OnTrack::Strip).count();
    let before = locks.len();
    locks.retain(|l| !matches!(l.addr.on, OnTrack::Insert(_)));
    out.effect_locks += before - locks.len();
}

/// A project saved before Drums was track 1's strip (sampler R3 part 4b)
/// plays as it did: each clip on track 2 and up whose lanes named "this
/// track's strip" or one of its inserts now names the strip they moved
/// (`old_strip_target`), its Original's too. A step lock can only name its
/// own track's strip: one there now reaches it, one on an insert is
/// dropped (`old_strip_locks`). Track 1's reach Drums, as before.
pub fn old_strip_targets(p: &mut mpfree_seq::Project) -> OldStripTargets {
    let mut out = OldStripTargets::default();
    for (t, track) in p.tracks.iter_mut().enumerate().skip(1) {
        let Ok(t) = u8::try_from(t) else {
            break;
        };
        for clip in track.slots.iter_mut().flatten() {
            let (moved, clashed) = old_strip_lanes(&mut clip.moves, t);
            out.lanes += moved;
            out.clashes += clashed;
            old_strip_locks(&mut clip.locks, &mut out);
            if let Some(o) = clip.original.as_mut() {
                let (moved, clashed) = old_strip_lanes(&mut o.moves, t);
                out.lanes += moved;
                out.clashes += clashed;
                old_strip_locks(&mut o.locks, &mut out);
            }
        }
    }
    out
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
    let spare = match st {
        StripId::Track(t) => t >= crate::mixing::TRACKS,
        // A pad channel past kit 1's A1-A16: lent only while it has
        // something set (`mixing::PadChannels`).
        StripId::Pad(p) => p >= crate::mixing::PADS,
        _ => false,
    };
    spare
        && mixer.chain(st).is_none_or(|c| c.is_empty())
        && mpfree_fx::strip_params()
            .iter()
            .filter(|p| saved_control(st, p.id))
            .all(|p| mixer.param(st, p.id) == p.default)
}

/// The mixer part: every strip's controls (but [`untouched_channel`]s),
/// then each insert with its parameters (and a plugin's state, base64), and
/// the pad mutes. A mixer of the app's adds the pads its lent pad channels
/// are lent to and where track 1 plays ([`mixer_text_lent`]); here it plays
/// into Drums.
///
/// ```text
/// mpfree-mixer 1
/// padmutes 0
/// track1 t0
/// strip t0 0=-3 1=0 3=0 4=0 5=0 6=-80 ...
/// insert t0 1 1 mpfree.fx.dj_filter
/// param 0 0.5
/// insert master 4 1 clap:com.example.verb file=verb.clap key=t1
/// state AAECAw==
/// ```
pub fn mixer_text(mixer: &Mixer, pad_mutes: u16, pad_solos: u16) -> String {
    let drums = StripId::Track(crate::mixing::DRUMS);
    mixer_text_lent(mixer, pad_mutes, pad_solos, &[], drums)
}

/// [`mixer_text`], with a `lent` line for each pad channel lent to a pad
/// (its strip, kit and pad): `lent ps16 1 20` is kit 2's pad B5; and the
/// `track1` line, the strip track 1 plays into (`t0`, Drums, or `t4`, its
/// old own channel: [`MixLoad::track1`]).
pub fn mixer_text_lent(
    mixer: &Mixer,
    pad_mutes: u16,
    pad_solos: u16,
    lent: &[(StripId, usize, usize)],
    track1: StripId,
) -> String {
    let mut s = String::with_capacity(4096);
    s.push_str(MIXER_FORMAT);
    s.push('\n');
    let _ = writeln!(s, "padmutes {pad_mutes}");
    if pad_solos != 0 {
        let _ = writeln!(s, "padsolos {pad_solos}");
    }
    let _ = writeln!(s, "track1 {}", strip_text(track1));
    for &(st, kit, pad) in lent {
        let _ = writeln!(s, "lent {} {kit} {pad}", strip_text(st));
    }
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
    /// The kit's pads soloed (TRACK MUTE held over pad mutes).
    pub pad_solos: u16,
    /// The pad channels lent to pads: strip, kit and pad.
    pub lent: Vec<(StripId, u8, u8)>,
    /// The strip track 1 played into when the project was saved (its
    /// `track1` line): Drums, or its old own channel (`MixHost::old_first`).
    /// None for a project saved before Drums was track 1's strip (sampler
    /// R3 part 4b); Drums, said in `notes`, for a strip it can't be.
    pub track1: Option<StripId>,
    /// Track 1 has its own instrument in the project (set by [`opening`]
    /// from the instruments part): in a project saved before part 4b it
    /// played into its own channel, so it keeps that.
    pub track1_own: bool,
    pub strips: Vec<(StripId, Vec<(u16, f32)>)>,
    /// Each strip's inserts, in order.
    pub chains: Vec<(StripId, Vec<Keyed>)>,
    /// The mod engine's part, loaded once the inserts are in.
    pub modu: Option<String>,
    /// The project's tracks' MIDI effects, so the mod engine's part can
    /// name their settings when it loads (`ModMix::set_note_fx`).
    pub note_fx: Option<Box<[mpfree_notefx::TrackFx; mpfree_seq::MAX_TRACKS]>>,
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
        pad_solos: 0,
        lent: Vec::new(),
        track1: None,
        track1_own: false,
        strips: Vec::new(),
        chains: Vec::new(),
        modu: modu.map(str::to_string),
        note_fx: None,
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
            Some("padsolos") => {
                out.pad_solos = w.next().and_then(|x| x.parse().ok()).ok_or_else(bad)?;
            }
            Some("track1") => {
                let st = w.next().and_then(parse_strip).ok_or_else(bad)?;
                let (drums, old) = (crate::mixing::DRUMS, crate::mixing::TRACKS);
                match st {
                    StripId::Track(t) if t == drums || t == old => out.track1 = Some(st),
                    _ => {
                        out.track1 = Some(StripId::Track(drums));
                        out.notes.push(format!(
                            "project: track 1 can't play into {}; it plays into Drums",
                            strip_text(st)
                        ));
                    }
                }
            }
            Some("lent") => {
                let st = w.next().and_then(parse_strip).ok_or_else(bad)?;
                let kit: u8 = w.next().and_then(|x| x.parse().ok()).ok_or_else(bad)?;
                let pad: u8 = w.next().and_then(|x| x.parse().ok()).ok_or_else(bad)?;
                if pad < 128 && kit < 128 {
                    out.lent.push((st, kit, pad));
                }
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
/// id and `key=value` extras, then a `sound` line for each line of a
/// sound's design (its effects and macros: `mpfree_instruments::Sound`),
/// then its parameters and a plugin's state.
/// Called on the audio thread when a project is saved (a few small
/// allocations, once): a plugin's state is its last snapshot, never a
/// round trip.
///
/// ```text
/// mpfree-instruments 2
/// track 2 mpfree.fm
/// param 0 5
/// track 3 mpfree.keys
/// sound fx.1 = mpfree.fx.ensemble
/// sound macro.1 = Space
/// sound macro.1.map = fx-1.mix 0 80 linear
/// param 64512 40
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
        let extras = node.save_extras();
        let (design, words): (Vec<_>, Vec<_>) = extras.iter().partition(|(k, _)| is_design_key(k));
        for (k, v) in words {
            if word_ok(k) && word_ok(v) {
                let _ = write!(s, " {k}={v}");
            }
        }
        s.push('\n');
        for (k, v) in design {
            if word_ok(k) && !v.contains(['\n', '\r']) {
                let _ = writeln!(s, "sound {k} = {v}");
            }
        }
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
    // The preset format its settings are in.
    let format = match lines.next().and_then(|(_, l)| instruments_version(l)) {
        Some(v) => v,
        None => return Err(format!("not an {INSTRUMENTS_FORMAT} part")),
    };
    let mut notes = Vec::new();
    // A track, its node and its plugin instance (0 for an engine).
    type Made = (usize, Box<dyn Node>, u64);
    let mut made: Vec<Made> = Vec::new();
    // The track being read: Some(None) while it is left out (its params
    // and state are skipped too).
    let mut open: Option<Option<Made>> = None;
    // Its sound's design lines, put in before its first param or state
    // (a sound's macros come first among its params, so they are set
    // before what they move).
    let mut design: Vec<(String, String)> = Vec::new();
    for (i, raw) in lines {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = || format!("line {}: {line}", i + 1);
        let mut w = line.split_whitespace();
        let first = w.next();
        if !design.is_empty() && first != Some("sound") {
            if let Some(Some(m)) = open.as_mut() {
                design_in(m, &design, format, rate, &mut notes);
            }
            design.clear();
        }
        match first {
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
            Some("sound") => {
                if open.is_none() {
                    return Err(bad());
                }
                let rest = line["sound".len()..].trim();
                let (k, v) = rest.split_once('=').ok_or_else(bad)?;
                design.push((k.trim().to_string(), v.trim().to_string()));
            }
            Some("param") => {
                let Some(o) = open.as_mut() else {
                    return Err(bad());
                };
                let id: u16 = w.next().and_then(|x| x.parse().ok()).ok_or_else(bad)?;
                let v: f32 = w.next().and_then(|x| x.parse().ok()).ok_or_else(bad)?;
                if let Some((_, node, _)) = o.as_mut() {
                    if v.is_finite() && node.params().iter().any(|p| p.id == id && !p.readonly) {
                        let v = if format < mpfree_node::preset::VERSION {
                            node.upgrade(format, id, v)
                        } else {
                            v
                        };
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
    if let Some(Some(m)) = open.as_mut() {
        if !design.is_empty() {
            design_in(m, &design, format, rate, &mut notes);
        }
    }
    if let Some(Some(m)) = open.take() {
        made.push(m);
    }
    let slots = made
        .into_iter()
        .map(|(track, node, instance)| {
            // The Wavetable synth went into Poly (19 Nov): its sound, read
            // as it was saved, plays on Poly.
            let node = if node.info().id == crate::synth::WAVETABLE {
                match crate::synth::as_poly(node) {
                    Ok(n) => {
                        notes.push(format!(
                            "project: track {}'s Wavetable sound plays on Poly now (the Wavetable synth is part of Poly)",
                            track + 1
                        ));
                        n
                    }
                    Err(e) => {
                        notes.push(format!(
                            "project: track {}'s Wavetable sound didn't move to Poly ({e}); the track plays the kit",
                            track + 1
                        ));
                        return None;
                    }
                }
            } else {
                node
            };
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

/// Puts a track's sound's design (its `sound` lines, saved in preset
/// format `format`) around the engine just made: the effects and macros
/// its preset brought. One that can't be built leaves the engine plain,
/// said in `notes`.
fn design_in(
    m: &mut (usize, Box<dyn Node>, u64),
    lines: &[(String, String)],
    format: u32,
    rate: u32,
    notes: &mut Vec<String>,
) {
    let (track, node, _) = m;
    let id = node.info().id;
    if crate::plugins::Plugins::is_plugin(id) {
        return;
    }
    let Some(engine) = crate::synth::engine_node(id, rate) else {
        return;
    };
    let built = mpfree_instruments::Sound::new(engine, &mpfree_instruments::Design::default())
        .and_then(|mut s| s.set_design_lines_from(lines, format).map(|()| s));
    match built {
        Ok(mut s) => {
            s.prepare_effects(rate, mpfree_node::MAX_BLOCK);
            *node = Box::new(s);
        }
        Err(e) => notes.push(format!(
            "project: track {}'s sound didn't build ({e}); it plays without its effects and macros",
            *track + 1
        )),
    }
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
    let mut wavetable: Option<String> = None;
    // None: options; Some(Some(i)): sound i (usize::MAX: the Wavetable
    // synth's); Some(None): an unknown section.
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
                    None if name == "Wavetable" => {
                        wavetable = Some(String::new());
                        Some(Some(usize::MAX))
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
            Some(Some(usize::MAX)) => {
                if let Some(w) = wavetable.as_mut() {
                    w.push_str(line);
                    w.push('\n');
                }
            }
            Some(Some(i)) => {
                sounds[i].1.push_str(line);
                sounds[i].1.push('\n');
            }
            Some(None) => {}
        }
    }
    let settings = mpfree_store::Settings::parse(&options);
    // The Wavetable synth went into Poly (19 Nov): where the pads played
    // it, its sound is Poly's now (it loads as Poly's, synth::poly_text);
    // otherwise Poly keeps its own.
    let on_pads = settings
        .iter()
        .any(|(k, v)| k == "engine" && v.eq_ignore_ascii_case("Wavetable"));
    if let Some(text) = wavetable.filter(|_| on_pads) {
        sounds.retain(|(e, _)| *e != Engine::Poly);
        sounds.push((Engine::Poly, text));
    }
    Ok((settings, sounds))
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
        let text = mixer_text(&a.mixer, 0b101, 0b10);
        let load = mixer_load(
            &text,
            Some("x = 1\n"),
            44_100,
            &crate::plugins::Plugins::none(),
        )
        .unwrap();
        assert_eq!((load.pad_mutes, load.pad_solos), (0b101, 0b10));
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
        let again = mixer_text(&b.mixer, b.pad_mutes(), b.pad_solos());
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
        let text = mixer_text(&a.mixer, 0, 0);
        assert!(
            text.contains("strip t0 "),
            "the engines' tracks are always saved"
        );
        assert!(
            !text.contains(&format!("strip {} ", strip_text(own(0)))),
            "{text}"
        );
        a.mixer.set_now(own(2), mx::VOLUME, -9.0);
        let text = mixer_text(&a.mixer, 0, 0);
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

    /// A pad channel lent to a pad (R3 part 4a) is saved with the pad it
    /// is lent to, and opens lent to it again, playing into its kit's
    /// track; a lent line for a pad that has its own, or past the kits, is
    /// left out.
    #[test]
    fn a_lent_pad_channel_is_saved_and_opens_lent_to_its_pad() {
        use crate::mixing::{MixHost, PADS};
        let mut a = MixHost::new(44_100, -6.0, 0.0);
        let s = a.lend_pad(1, 20).unwrap();
        a.mixer.set_now(s, mx::PAN, 30.0);
        let mut text = a.mixer_text();
        assert!(
            text.contains(&format!("lent {} 1 20\n", strip_text(s))),
            "{text}"
        );
        text.push_str(&format!(
            "lent {} 0 3\n",
            strip_text(StripId::Pad(PADS + 1))
        ));
        let load = mixer_load(&text, None, 44_100, &crate::plugins::Plugins::none()).unwrap();
        assert_eq!(load.lent.len(), 2);
        let mut b = MixHost::new(44_100, -6.0, 0.0);
        let _ = b.pads_homed(&[0, 2]);
        b.open(Box::new(load));
        for _ in 0..400 {
            b.tend_open();
            b.run_silent_period();
        }
        assert!(!b.opening());
        assert_eq!(b.pad_channels.channel(1, 20), Some(s));
        assert_eq!(
            b.pad_channels.holder(StripId::Pad(PADS + 1)),
            None,
            "A4 has its own"
        );
        assert_eq!(b.mixer.param(s, mx::PAN), 30.0);
        let track = crate::instruments::first_track() + 1;
        assert_eq!(b.mixer.param(s, mx::OUTPUT_TRACK), track as f32);
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
                    starts: mpfree_looper::Starts::Bar,
                    fade_old: 1.0,
                    speed: 0,
                    rate: 0,
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
        let (fm, extras, mut values) = synth.sound_of(Engine::Fm);
        // The values are in the order of the sound's own settings: its
        // engine's, then its effects' and macros' when its preset is a
        // sound (FM's start preset is).
        let params = crate::synth::fresh_node(fm, &extras, &values, rate)
            .unwrap()
            .params();
        let k = params
            .iter()
            .position(|p| !p.readonly && p.max > p.min)
            .unwrap();
        values[k] = if values[k] == params[k].max {
            params[k].min
        } else {
            params[k].max
        };
        let made = make(&plugins, fm, &extras, &values, rate, PERIOD).unwrap();
        let tuning = synth.tuning_now();
        assert!(inst
            .put(1, made, synth.pads_for(Engine::Fm), &tuning)
            .is_ok());
        let (poly, px, pv) = synth.sound_of(Engine::Poly);
        let made = make(&plugins, poly, &px, &pv, rate, PERIOD).unwrap();
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

    /// A track's sound with its own effects and macros (a setting turned
    /// after its macro moved) is written with its design and read back to
    /// the bit; one whose design can't be built plays plain, said.
    #[test]
    fn a_tracks_sound_keeps_its_effects_and_macros() {
        use crate::instruments::{make, Instruments};
        use mpfree_instruments::sound::{FX_BASE, MACRO_BASE};
        use mpfree_instruments::{Design, Macro, MacroMap, Sound};
        let rate = 44_100;
        let plugins = crate::plugins::Plugins::none();
        let mut inst = Instruments::new(rate, PERIOD);
        let design = Design {
            fx: vec!["mpfree.fx.ensemble".into()],
            macros: vec![Macro {
                name: "Wide Space".into(),
                maps: vec![MacroMap {
                    target: FX_BASE + mpfree_fx::ids::ensemble::MIX,
                    from: 0.0,
                    to: 80.0,
                    curve: mpfree_mod::MapCurve::S,
                }],
            }],
        };
        let mut sound = Sound::new(
            crate::synth::engine_node("mpfree.keys", rate).unwrap(),
            &design,
        )
        .unwrap();
        sound.set_param(MACRO_BASE, 70.0);
        sound.set_param(FX_BASE + mpfree_fx::ids::ensemble::MIX, 33.0);
        let values: Vec<f32> = sound.params().iter().map(|p| sound.param(p.id)).collect();
        let made = make(
            &plugins,
            "mpfree.keys",
            &sound.save_extras(),
            &values,
            rate,
            PERIOD,
        )
        .unwrap();
        let pads = std::array::from_fn(|p| mpfree_arp::scale::pads::Pad::single(60 + p as i16));
        assert!(inst
            .put(2, made, pads, &mpfree_node::Tuning::standard())
            .is_ok());
        let text = instruments_text(&inst);
        assert!(
            text.contains("\ntrack 3 mpfree.keys\nsound fx.1 = mpfree.fx.ensemble\nsound macro.1 = Wide Space\nsound macro.1.map = fx-1.mix 0 80 s\nparam 64512 70\n"),
            "{text}"
        );
        let (slots, notes) = instruments_load(&text, rate, &plugins).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let back = slots[0].as_ref().unwrap().instrument.node();
        assert_eq!(back.params().len(), sound.params().len());
        for p in sound.params() {
            assert_eq!(
                back.param(p.id).to_bits(),
                sound.param(p.id).to_bits(),
                "{}",
                p.name
            );
        }
        // An effect this build hasn't got: the engine plays plain, said.
        let odd = text.replace("mpfree.fx.ensemble", "mpfree.fx.sparkle");
        let (slots, notes) = instruments_load(&odd, rate, &plugins).unwrap();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(
            notes[0].contains("track 3's sound didn't build"),
            "{notes:?}"
        );
        let plain = slots[0].as_ref().unwrap().instrument.node();
        assert_eq!(plain.info().id, "mpfree.keys");
        assert!(plain.params().iter().all(|p| p.id < FX_BASE));
    }

    /// A part saved before preset format 2 holds Poly's old envelope
    /// times and velocity share; they are converted as it is read, so the
    /// track sounds as it did, and its sound's macros too. This build
    /// writes version 2.
    #[test]
    fn an_older_instruments_part_is_converted_as_it_is_read() {
        use mpfree_synth::params as pp;
        let plugins = crate::plugins::Plugins::none();
        let old = format!(
            "mpfree-instruments 1\ntrack 2 mpfree.poly\nparam {} 100\nparam {} 300\nparam {} 60\n",
            pp::AMP_A,
            pp::AMP_D,
            pp::VEL_AMP
        );
        let (slots, notes) = instruments_load(&old, 44_100, &plugins).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let node = slots[0].as_ref().unwrap().instrument.node();
        assert!((node.param(pp::AMP_A) - 31.877).abs() < 0.01);
        assert_eq!(node.param(pp::AMP_D), 300.0, "decay keeps its meaning");
        assert!((node.param(pp::VEL_AMP) - 7.957).abs() < 0.01);
        let now = old.replace("mpfree-instruments 1", "mpfree-instruments 2");
        let (slots, _) = instruments_load(&now, 44_100, &plugins).unwrap();
        let node = slots[0].as_ref().unwrap().instrument.node();
        assert_eq!(node.param(pp::AMP_A), 100.0);
        assert_eq!(INSTRUMENTS_FORMAT, "mpfree-instruments 2");
        assert_eq!(instruments_version("mpfree-instruments 1"), Some(1));
        assert_eq!(instruments_version("mpfree-instruments 3"), None);
        assert_eq!(instruments_version("mpfree-instrumentsx 2"), None);
    }

    /// A track that played the Wavetable synth (gone into Poly on 19 Nov)
    /// opens on Poly: its settings converted, its effects and macros kept,
    /// and a line says so.
    #[test]
    fn a_wavetable_track_opens_on_poly() {
        use mpfree_instruments::sound::FX_BASE;
        use mpfree_synth::params as pp;
        use mpfree_synth::wavetable::params as wp;
        use mpfree_synth::wavetable::to_poly::poly_value;
        let plugins = crate::plugins::Plugins::none();
        let text = format!(
            "{INSTRUMENTS_FORMAT}\ntrack 2 mpfree.wavetable\n\
             sound fx.1 = mpfree.fx.ensemble\n\
             param {} 1\nparam {} 40\nparam {} 900\nparam {} -3\n",
            wp::OSC_B + wp::TABLE,
            wp::POSITION,
            wp::CUTOFF,
            wp::LEVEL,
        );
        let (slots, notes) = instruments_load(&text, 44_100, &plugins).unwrap();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("track 2's Wavetable sound plays on Poly"));
        let slot = slots[0].as_ref().unwrap();
        assert_eq!(slot.held.id, "mpfree.poly");
        let node = slot.instrument.node();
        assert_eq!(node.info().id, "mpfree.poly");
        assert_eq!(node.param(pp::OSC1_WAVE), f32::from(pp::WAVE_TABLE));
        assert_eq!(node.param(pp::OSC2_TABLE), 1.0);
        assert_eq!(node.param(pp::OSC1_POS), 40.0);
        assert_eq!(node.param(pp::CUTOFF), 900.0);
        assert_eq!(
            node.param(pp::LEVEL),
            poly_value(wp::LEVEL, -3.0).unwrap().1
        );
        assert!(
            node.params().iter().any(|p| p.id >= FX_BASE),
            "the effect came along"
        );
        // Without a design it is the bare engine, as a Poly track is.
        let bare = text.replace("sound fx.1 = mpfree.fx.ensemble\n", "");
        let (slots, _) = instruments_load(&bare, 44_100, &plugins).unwrap();
        let node = slots[0].as_ref().unwrap().instrument.node();
        assert!(node.params().iter().all(|p| p.id < FX_BASE));
        assert_eq!(node.param(pp::OSC1_POS), 40.0);
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

    /// What each track plays goes in its own part, only once a track plays
    /// something other than kit 1, and comes back on open; a part that
    /// doesn't read is said, and every track plays kit 1.
    #[test]
    fn the_tracks_kits_are_kept_and_come_back() {
        let c = |track_kits: String| Collected {
            tempo: 120.0,
            swing: mpfree_transport::time::Swing::new(240, 50).unwrap(),
            clips: mpfree_seq::Project::default(),
            mixer: String::new(),
            modu: String::new(),
            synth: String::new(),
            arp: String::new(),
            perform: String::new(),
            instruments: format!("{INSTRUMENTS_FORMAT}\n"),
            track_kits,
            instrument_list: InstrumentList::new().text(),
            pads: String::new(),
            loops: None,
            audio: None,
        };
        // Nine kits, so the synth is entry 9.
        let mut kits = crate::sampler::TrackKits::new(9);
        let none = kits.text();
        let snap = snapshot(&c(none), Vec::new());
        assert!(
            snap.part(TRACK_KITS).is_none(),
            "not written while every track plays kit 1"
        );
        let o = opening(&snap, 44_100, "a test", &crate::plugins::Plugins::none());
        assert!(o.track_kits.is_empty());
        assert!(kits.set(1, 2) && kits.set(3, 9));
        let picks = kits.text();
        let snap = snapshot(&c(picks), Vec::new());
        assert!(snap.part(TRACK_KITS).is_some());
        let o = opening(&snap, 44_100, "a test", &crate::plugins::Plugins::none());
        assert_eq!(o.track_kits, [(1, Some(2)), (3, None)]);
        let mut snap = snap;
        for (name, text) in &mut snap.parts {
            if name == TRACK_KITS {
                text.push_str("track two kit 1\n");
            }
        }
        let o = opening(&snap, 44_100, "a test", &crate::plugins::Plugins::none());
        assert!(o.track_kits.is_empty());
        assert!(
            o.notes.iter().any(|n| n.contains("they play kit 1")),
            "{:?}",
            o.notes
        );
        assert!(track_kits_parse("mpfree-track-kits 3\n").is_err());
        assert!(track_kits_parse(&format!("{TRACK_KITS_FORMAT}\ntrack 0 kit 1\n")).is_err());
    }

    /// The parts a test project saves: what a save collects, with these
    /// track kits and list of instruments.
    fn collected(track_kits: String, instrument_list: String) -> Collected {
        Collected {
            tempo: 120.0,
            swing: mpfree_transport::time::Swing::new(240, 50).unwrap(),
            clips: mpfree_seq::Project::default(),
            mixer: String::new(),
            modu: String::new(),
            synth: String::new(),
            arp: String::new(),
            perform: String::new(),
            instruments: format!("{INSTRUMENTS_FORMAT}\n"),
            track_kits,
            instrument_list,
            pads: String::new(),
            loops: None,
            audio: None,
        }
    }

    /// The lines of an open's log about the project's format and its
    /// instruments (a test project's empty parts say more).
    fn instrument_notes(o: &Opening) -> Vec<&String> {
        o.notes
            .iter()
            .filter(|n| n.contains("format") || n.contains("instrument"))
            .collect()
    }

    /// Instrument redesign step 2a: a project saved before instruments
    /// had ids (no format line, no list of instruments, and `track 4
    /// synth` in the track kits part's version 1, as the build before
    /// wrote it) opens with the same tracks on the same kits and the
    /// synth, saying nothing: nothing was lost. Saved again it is format
    /// 2, lists the shared synth as instrument 1 and points track 4 at it
    /// by that id; opened again, every track plays what it played.
    #[test]
    fn a_project_from_before_instrument_ids_opens_and_saves_as_it_played() {
        let none = &crate::plugins::Plugins::none();
        let mut old = snapshot(
            &collected(
                "mpfree-track-kits 1\ntrack 2 kit 3\ntrack 4 synth\n".to_string(),
                String::new(),
            ),
            Vec::new(),
        );
        old.meta.extra.retain(|(k, _)| k != FORMAT_KEY);
        assert!(old.part(INSTRUMENT_LIST).is_none() && old.part(TRACK_KITS).is_some());
        let o = opening(&old, 44_100, "an old project", none);
        assert_eq!(o.track_kits, [(1, Some(2)), (3, None)]);
        assert_eq!(o.instrument_list, InstrumentList::new());
        assert!(instrument_notes(&o).is_empty(), "{:?}", o.notes);

        // Saved again, as the audio loop takes and collects it.
        let mut kits = crate::sampler::TrackKits::new(9);
        for &(track, kit) in &o.track_kits {
            assert!(kits.set(track, kit.unwrap_or(kits.synth())));
        }
        let snap = snapshot(
            &collected(kits.text(), o.instrument_list.text()),
            Vec::new(),
        );
        assert!(snap
            .meta
            .extra
            .contains(&(FORMAT_KEY.to_string(), FORMAT.to_string())));
        assert_eq!(
            snap.part(TRACK_KITS),
            Some("mpfree-track-kits 2\ntrack 2 kit 3\ntrack 4 instrument 1\n")
        );
        assert_eq!(
            snap.part(INSTRUMENT_LIST),
            Some("mpfree-instrument-list 1\nnext 2\ninstrument 1 shared-synth\n")
        );
        let again = opening(&snap, 44_100, "it again", none);
        assert_eq!(again.track_kits, o.track_kits);
        assert_eq!(again.instrument_list, o.instrument_list);
        assert!(instrument_notes(&again).is_empty(), "{:?}", again.notes);
    }

    /// A track pointing at an instrument the list hasn't got plays kit 1,
    /// said; the others keep theirs. Each version of the track kits part
    /// has its own word for the shared synth: a part mixing them doesn't
    /// read, said, and every track plays kit 1.
    #[test]
    fn a_track_whose_instrument_is_not_listed_plays_kit_1_and_says_so() {
        let none = &crate::plugins::Plugins::none();
        let list = InstrumentList::new().text();
        let kits = format!("{TRACK_KITS_FORMAT}\ntrack 2 instrument 5\ntrack 3 instrument 1\n");
        let o = opening(
            &snapshot(&collected(kits, list.clone()), Vec::new()),
            44_100,
            "x",
            none,
        );
        assert_eq!(o.track_kits, [(2, None)]);
        assert_eq!(
            instrument_notes(&o),
            ["project: track 2 plays kit 1 (its instrument 5 isn't in the project's list)"]
        );
        for kits in [
            format!("{TRACK_KITS_FORMAT}\ntrack 3 synth\n"),
            "mpfree-track-kits 1\ntrack 3 instrument 1\n".to_string(),
            format!("{TRACK_KITS_FORMAT}\ntrack 3 instrument 0\n"),
        ] {
            let snap = snapshot(&collected(kits.clone(), list.clone()), Vec::new());
            let o = opening(&snap, 44_100, "x", none);
            assert!(o.track_kits.is_empty(), "{kits}");
            assert!(
                o.notes.iter().any(|n| n.contains("they play kit 1")),
                "{kits}: {:?}",
                o.notes
            );
        }
    }

    /// Every save and New project is format 2 with the list of
    /// instruments. A newer format, a format line that doesn't read, a
    /// format 2 project with no list, and a list that doesn't read are
    /// each said; the project opens with the new project's list.
    #[test]
    fn the_project_format_and_the_list_are_written_and_a_problem_is_said() {
        let none = &crate::plugins::Plugins::none();
        let kits = format!("{TRACK_KITS_FORMAT}\ntrack 4 instrument 1\n");
        let good = snapshot(
            &collected(kits.clone(), InstrumentList::new().text()),
            Vec::new(),
        );
        let new = new_snapshot(&Defaults::default());
        for snap in [&good, &new] {
            assert!(snap
                .meta
                .extra
                .contains(&(FORMAT_KEY.to_string(), FORMAT.to_string())));
            assert_eq!(
                snap.part(INSTRUMENT_LIST),
                Some(InstrumentList::new().text().as_str())
            );
            let o = opening(snap, 44_100, "x", none);
            assert!(instrument_notes(&o).is_empty(), "{:?}", o.notes);
        }
        let with = |format: Option<&str>, list: Option<&str>| {
            let mut snap = good.clone();
            snap.meta.extra.retain(|(k, _)| k != FORMAT_KEY);
            if let Some(f) = format {
                snap.meta
                    .extra
                    .push((FORMAT_KEY.to_string(), f.to_string()));
            }
            snap.parts.retain(|(o, _)| o != INSTRUMENT_LIST);
            if let Some(l) = list {
                snap.parts
                    .push((INSTRUMENT_LIST.to_string(), l.to_string()));
            }
            opening(&snap, 44_100, "x", none)
        };
        let list = InstrumentList::new().text();
        for (format, list, said) in [
            (
                Some("3"),
                Some(list.as_str()),
                "saved by a newer mpfree (format 3",
            ),
            (
                Some("two"),
                Some(list.as_str()),
                "its format line (\"two\") doesn't read",
            ),
            (Some("2"), None, "its list of instruments is missing"),
            (
                Some("2"),
                Some("mpfree-instrument-list 1\n"),
                "list of instruments wasn't opened",
            ),
        ] {
            let o = with(format, list);
            assert!(
                o.notes.iter().any(|n| n.contains(said)),
                "{said}: {:?}",
                o.notes
            );
            assert_eq!(o.instrument_list, InstrumentList::new());
            assert_eq!(o.track_kits, [(3, None)], "{said}");
        }
        // Format 1 with no list is an old project: nothing to say.
        let o = with(None, None);
        assert!(instrument_notes(&o).is_empty(), "{:?}", o.notes);
    }

    /// The instruments part with FM on track 2 and Poly on track 4, each
    /// at its start sound.
    fn fm_on_2_and_poly_on_4() -> String {
        use crate::instruments::{make, Instruments};
        use crate::synth::Engine;
        let (rate, plugins) = (44_100, crate::plugins::Plugins::none());
        let synth = crate::synth::Synth::new(rate);
        let mut inst = Instruments::new(rate, PERIOD);
        for (track, engine) in [(1, Engine::Fm), (3, Engine::Poly)] {
            let (id, extras, values) = synth.sound_of(engine);
            let made = make(&plugins, id, &extras, &values, rate, PERIOD).unwrap();
            let pads = synth.pads_for(engine);
            assert!(inst.put(track, made, pads, &synth.tuning_now()).is_ok());
        }
        instruments_text(&inst)
    }

    /// Step 2b: the tracks' own instruments join the list. In a project
    /// saved before instruments had ids they get the next ids in track
    /// order, saying nothing; saved again the list names each one's
    /// track, and opened again every one keeps its id.
    #[test]
    fn the_tracks_own_instruments_get_ids_that_come_back() {
        let none = &crate::plugins::Plugins::none();
        let mut c = collected(String::new(), String::new());
        c.instruments = fm_on_2_and_poly_on_4();
        let mut old = snapshot(&c, Vec::new());
        old.meta.extra.retain(|(k, _)| k != FORMAT_KEY);
        let o = opening(&old, 44_100, "an old project", none);
        assert_eq!(o.instruments.iter().flatten().count(), 2);
        let id = InstrumentId::from_number;
        assert_eq!(
            (o.instrument_list.own_of(1), o.instrument_list.own_of(3)),
            (id(2), id(3))
        );
        assert!(instrument_notes(&o).is_empty(), "{:?}", o.notes);

        c.instrument_list = o.instrument_list.text();
        let snap = snapshot(&c, Vec::new());
        assert_eq!(
            snap.part(INSTRUMENT_LIST),
            Some(
                "mpfree-instrument-list 1\nnext 4\ninstrument 1 shared-synth\n\
                 instrument 2 own track 2\ninstrument 3 own track 4\n"
            )
        );
        let again = opening(&snap, 44_100, "it again", none);
        assert_eq!(again.instrument_list, o.instrument_list);
        assert!(instrument_notes(&again).is_empty(), "{:?}", again.notes);
    }

    /// A newer project whose list and instruments part disagree: a track's
    /// instrument with no id gets the next one, an id whose track has no
    /// instrument is gone, and each is said. A track pointed at another
    /// track's own instrument plays kit 1, said.
    #[test]
    fn a_list_that_disagrees_with_the_instruments_part_is_put_right_and_said() {
        let none = &crate::plugins::Plugins::none();
        let mut c = collected(
            format!("{TRACK_KITS_FORMAT}\ntrack 7 instrument 5\n"),
            "mpfree-instrument-list 1\nnext 9\ninstrument 1 shared-synth\n\
             instrument 5 own track 2\ninstrument 6 own track 6\n"
                .to_string(),
        );
        c.instruments = fm_on_2_and_poly_on_4();
        let o = opening(&snapshot(&c, Vec::new()), 44_100, "x", none);
        let id = InstrumentId::from_number;
        let list = &o.instrument_list;
        assert_eq!(
            (list.own_of(1), list.own_of(3), list.own_of(5)),
            (id(5), id(9), None)
        );
        assert!(o.track_kits.is_empty());
        assert_eq!(
            instrument_notes(&o),
            [
                "project: instrument 6 (track 6's own) isn't there, so it is gone from the list",
                "project: track 4's own instrument had no id in the list; it is instrument 9 now",
                "project: track 7 plays kit 1 (its instrument 5 is track 2's own)",
            ]
        );
    }

    /// Step 2c: tracks get lasting ids as a project opens. One saved
    /// before tracks had ids gets 1, 2, 3 ... saying nothing; one that
    /// says the next id but has a track without an id, or two sharing one,
    /// gets new ones for those, said. The time engine keeps them.
    #[test]
    fn tracks_get_lasting_ids_as_a_project_opens() {
        use mpfree_seq::{Track, TrackId};
        let none = &crate::plugins::Plugins::none();
        let mut c = collected(String::new(), InstrumentList::new().text());
        for i in 0..3 {
            c.clips
                .tracks
                .push(Track::new(&format!("Track {}", i + 1), i));
        }
        let o = opening(&snapshot(&c, Vec::new()), 44_100, "old", none);
        let ids = |o: &Opening| -> Vec<TrackId> {
            let t = o.time.as_ref().expect("the clips open");
            t.project().tracks.iter().map(|t| t.id).collect()
        };
        assert_eq!(ids(&o), [TrackId(1), TrackId(2), TrackId(3)]);
        assert!(
            !o.notes.iter().any(|n| n.contains("lasting id")),
            "{:?}",
            o.notes
        );

        c.clips.next_track = 9;
        c.clips.tracks[0].id = TrackId(7);
        c.clips.tracks[1].id = TrackId(7);
        let o = opening(&snapshot(&c, Vec::new()), 44_100, "damaged", none);
        assert_eq!(ids(&o), [TrackId(7), TrackId(9), TrackId(10)]);
        assert!(
            o.notes.iter().any(|n| n
                == "project: 2 of its tracks had no lasting id, or shared one; they have new ones"),
            "{:?}",
            o.notes
        );
    }

    /// Looper redesign step 4c: a tempo a free first loop set is any
    /// number, and the loop is its length only at that tempo. A project
    /// keeps it exactly through its text and opens the song at it.
    #[test]
    fn a_tempo_from_a_loop_comes_back_exactly() {
        let bar = 4 * mpfree_transport::time::PPQ as i64;
        let (_, from_loop) = mpfree_looper::free_tempo(123_457, 44_100, bar).unwrap();
        for tempo in [117.3456, from_loop] {
            let c = Collected {
                tempo,
                swing: mpfree_transport::time::Swing::new(240, 50).unwrap(),
                clips: mpfree_seq::Project::default(),
                mixer: String::new(),
                modu: String::new(),
                synth: String::new(),
                arp: String::new(),
                perform: String::new(),
                instruments: format!("{INSTRUMENTS_FORMAT}\n"),
                track_kits: String::new(),
                instrument_list: InstrumentList::new().text(),
                pads: String::new(),
                loops: None,
                audio: None,
            };
            let snap = snapshot(&c, Vec::new());
            let text = snap.meta.to_text();
            let back = Snapshot {
                meta: mpfree_library::Meta::parse(&text).unwrap(),
                parts: snap.parts.clone(),
            };
            let o = opening(&back, 44_100, "a test", &crate::plugins::Plugins::none());
            assert_eq!(o.tempo, Some(tempo), "{text}");
            let time = o.time.expect("the clips");
            assert_eq!(time.info().bpm, tempo);
        }
    }

    /// The Perform set goes in its own part and comes back on open; a
    /// project saved before the part has none, and opens with a new
    /// project's set.
    #[test]
    fn the_perform_set_is_kept_and_comes_back() {
        use crate::perform_pads::{PadAction, PerformSet};
        let mut set = PerformSet::default();
        set.pages[2].pads[0] = PadAction::Scene(3);
        let c = Collected {
            tempo: 120.0,
            swing: mpfree_transport::time::Swing::new(240, 50).unwrap(),
            clips: mpfree_seq::Project::default(),
            mixer: String::new(),
            modu: String::new(),
            synth: String::new(),
            arp: String::new(),
            perform: set.text(),
            instruments: format!("{INSTRUMENTS_FORMAT}\n"),
            track_kits: String::new(),
            instrument_list: InstrumentList::new().text(),
            pads: String::new(),
            loops: None,
            audio: None,
        };
        let mut snap = snapshot(&c, Vec::new());
        let o = opening(&snap, 44_100, "a test", &crate::plugins::Plugins::none());
        let (back, skipped) = PerformSet::parse(o.perform.as_deref().unwrap());
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(back, set);
        snap.parts.retain(|(name, _)| name != PERFORM);
        let o = opening(&snap, 44_100, "a test", &crate::plugins::Plugins::none());
        assert_eq!(o.perform, None);
    }

    /// What the pads play is saved as the pads part and comes back; a
    /// project without it (New, or saved before it) follows the track.
    #[test]
    fn what_the_pads_play_is_kept_and_comes_back() {
        let mut c = collected(String::new(), InstrumentList::new().text());
        c.pads = crate::app::PadFocus::SYNTH.text();
        let mut snap = snapshot(&c, Vec::new());
        assert_eq!(snap.part(PADS), Some("mpfree-pads 1\nplay instrument 1\n"));
        let o = opening(&snap, 44_100, "a test", &crate::plugins::Plugins::none());
        assert_eq!(o.pads.as_deref(), Some(c.pads.as_str()));
        snap.parts.retain(|(name, _)| name != PADS);
        let o = opening(&snap, 44_100, "a test", &crate::plugins::Plugins::none());
        assert_eq!(o.pads, None);
        let o = opening(
            &new_snapshot(&Defaults::default()),
            44_100,
            "a new project",
            &crate::plugins::Plugins::none(),
        );
        assert_eq!(o.pads, None, "a new project follows the track");
    }

    /// A project saved before Drums was track 1's strip plays its clips'
    /// strip moves as it did (the sequencer thread's review of #624): an
    /// old clip on track 2 with a `t:fx1.2` lane still turns mixer track 1's
    /// (Synth's) insert 1; its strip lock now reaches track 2's own strip,
    /// and its effect lock, which turned Synth's effect, is dropped. All
    /// counted on the open's line. Track 1's, and a project saved since,
    /// are left as they are.
    #[test]
    fn an_old_projects_strip_moves_on_track_2_keep_their_strip() {
        use mpfree_seq::mpfree_moves::{Map, Point, Target};
        use mpfree_seq::mpfree_param::{track_strip, OnTrack, Owner, ParamAddr, RelAddr, Strip};
        use mpfree_seq::{Clip, Lock, Track};
        let fx = RelAddr::new(OnTrack::Insert(1), 2);
        let vol = RelAddr::new(OnTrack::Strip, 0);
        let mut clips = mpfree_seq::Project::default();
        for t in 0..3u16 {
            let mut track = Track::new(&format!("Track {}", t + 1), t);
            let mut clip = Clip::new("c", 3840);
            let lane = Target::Track(fx);
            let map = Map::linear(0.0, 1.0);
            let points = [Point::step(0, 0.25), Point::step(1920, 0.75)];
            clip.moves.reserve(lane, map, points.len()).unwrap();
            clip.moves.restore(lane, map, 0, 3840, &points).unwrap();
            clip.add_lock(Lock::new(0, None, vol, 0.5)).unwrap();
            clip.add_lock(Lock::new(960, None, fx, 0.5)).unwrap();
            track.slots[0] = Some(clip);
            clips.tracks.push(track);
        }
        let open = |mixer: String| {
            let c = Collected {
                tempo: 120.0,
                swing: mpfree_transport::time::Swing::new(240, 50).unwrap(),
                clips: clips.clone(),
                mixer,
                modu: String::new(),
                synth: String::new(),
                arp: String::new(),
                perform: String::new(),
                instruments: format!("{INSTRUMENTS_FORMAT}\n"),
                track_kits: String::new(),
                instrument_list: InstrumentList::new().text(),
                pads: String::new(),
                loops: None,
                audio: None,
            };
            let o = opening(
                &snapshot(&c, Vec::new()),
                44_100,
                "a test",
                &crate::plugins::Plugins::none(),
            );
            let Some(time) = o.time else {
                panic!("the clips: {:?}", o.notes);
            };
            (time.project().clone(), o.notes)
        };
        let clip = |p: &mpfree_seq::Project, t: usize| p.tracks[t].slots[0].clone().unwrap();

        let (p, notes) = open(format!("{MIXER_FORMAT}\npadmutes 0\n"));
        let synth_fx = ParamAddr::new(Owner::Insert(Strip::Track(1), 1), 2);
        let two = clip(&p, 1);
        assert_eq!(two.moves.lanes().len(), 1);
        let lane = two.moves.lanes()[0].target;
        assert_eq!(lane, Target::Fixed(synth_fx));
        assert_eq!(
            lane.on(1),
            synth_fx,
            "track 2's lane still turns Synth's insert 1"
        );
        assert_eq!(two.moves.lanes()[0].points.len(), 2);
        assert_eq!(two.locks, vec![Lock::new(0, None, vol, 0.5)]);
        assert_eq!(two.locks[0].addr.on(1).owner, Owner::Strip(track_strip(1)));
        // Track 3's lane keeps Loop's insert; track 1's are as they were.
        assert_eq!(
            clip(&p, 2).moves.lanes()[0].target,
            Target::Fixed(ParamAddr::new(Owner::Insert(Strip::Track(2), 1), 2))
        );
        assert_eq!(clip(&p, 0), clips.tracks[0].slots[0].clone().unwrap());
        let line = notes
            .iter()
            .find(|n| n.starts_with("project: saved before Drums was track 1's strip"))
            .unwrap_or_else(|| panic!("{notes:?}"));
        assert!(
            line.contains("2 lanes on tracks 2 and up keep the strip they moved"),
            "{line}"
        );
        assert!(
            line.contains("2 strip locks on tracks 2 and up now reach their track's own strip"),
            "{line}"
        );
        assert!(
            line.contains("2 effect locks on tracks 2 and up dropped"),
            "{line}"
        );

        // Saved since: as written.
        let (p, notes) = open(format!("{MIXER_FORMAT}\npadmutes 0\ntrack1 t0\n"));
        assert_eq!(clip(&p, 1), clips.tracks[1].slots[0].clone().unwrap());
        assert!(
            !notes.iter().any(|n| n.contains("saved before Drums")),
            "{notes:?}"
        );
    }

    /// The mixer's load knows whether track 1 has its own instrument, for
    /// a project saved before Drums was track 1's strip
    /// (`MixHost::tend_open`), and reads the `track1` line of one since.
    #[test]
    fn the_mixer_knows_where_track_1_played() {
        let open = |mixer: &str, track: usize| {
            let c = Collected {
                tempo: 120.0,
                swing: mpfree_transport::time::Swing::new(240, 50).unwrap(),
                clips: mpfree_seq::Project::default(),
                mixer: mixer.to_string(),
                modu: String::new(),
                synth: String::new(),
                arp: String::new(),
                perform: String::new(),
                instruments: format!("{INSTRUMENTS_FORMAT}\ntrack {track} mpfree.poly\n"),
                track_kits: String::new(),
                instrument_list: InstrumentList::new().text(),
                pads: String::new(),
                loops: None,
                audio: None,
            };
            let snap = snapshot(&c, Vec::new());
            let o = opening(&snap, 44_100, "a test", &crate::plugins::Plugins::none());
            assert_eq!(o.instruments.iter().flatten().count(), 1, "{:?}", o.notes);
            let m = o.mix.expect("the mixer");
            (m.track1_own, m.track1)
        };
        let before = format!("{MIXER_FORMAT}\npadmutes 0\n");
        assert_eq!(open(&before, 1), (true, None));
        assert_eq!(open(&before, 2), (false, None));
        let since = format!("{MIXER_FORMAT}\npadmutes 0\ntrack1 t4\n");
        assert_eq!(
            open(&since, 2),
            (false, Some(StripId::Track(crate::mixing::TRACKS)))
        );
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
            perform: String::new(),
            instruments: format!("{INSTRUMENTS_FORMAT}\n"),
            track_kits: String::new(),
            instrument_list: InstrumentList::new().text(),
            pads: String::new(),
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

    /// A project from before the Wavetable synth went into Poly: where the
    /// pads played it, its sound comes back as Poly's (and the pads play
    /// Poly); otherwise Poly keeps its own.
    #[test]
    fn the_pads_wavetable_sound_comes_back_on_poly() {
        use mpfree_synth::wavetable::params as wp;
        let synth = crate::synth::Synth::new(44_100);
        let mut wt = mpfree_synth::Wavetable::new(44_100);
        wt.set_param(wp::POSITION, 55.0);
        let wt_text =
            mpfree_node::preset::Preset::capture(&wt, "Old Pad", &[]).to_text(wt.params());
        let mut opts = mpfree_store::Settings::default();
        opts.set_str("engine", "Wavetable");
        let sounds = synth.sounds();
        let text = format!("{}[sound Wavetable]\n{wt_text}", synth_text(&opts, &sounds));
        let (o, s) = synth_parse(&text).unwrap();
        let poly: Vec<&String> = s
            .iter()
            .filter(|(e, _)| *e == Engine::Poly)
            .map(|(_, t)| t)
            .collect();
        assert_eq!(poly, [&wt_text], "the Wavetable sound is Poly's");
        let mut other = crate::synth::Synth::new(44_100);
        assert!(other.set_sounds(&s).is_empty());
        let mut want = crate::synth::Synth::new(44_100);
        let _ = want.set_sounds(&[(
            Engine::Poly,
            crate::synth::poly_text(&wt_text).unwrap().into_owned(),
        )]);
        assert_eq!(other.sounds(), want.sounds());
        assert_eq!(o.get_str("engine"), Some("Wavetable"));
        // Played elsewhere, Poly keeps its own.
        opts.set_str("engine", "fm");
        let text = format!("{}[sound Wavetable]\n{wt_text}", synth_text(&opts, &sounds));
        let (_, s) = synth_parse(&text).unwrap();
        assert_eq!(s, sounds);
    }
}
