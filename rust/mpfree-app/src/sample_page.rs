//! The Sample tab: the sampler's pad editor (`mpfree-sampler-ui`) on the pad
//! player's kits. It has the Pads tab (the kit's pads on a compact map, to
//! pick one, copy, paste or clear it, set it up and put the kit's sounds on
//! it), the Wave tab (Start, End, the loop, the fades and the region's
//! settings), the Chop tab (slices by transients, beats, a
//! grid, silence or by hand, put on the kit's pads) and the Play tab (the
//! layers' velocities, the amp envelope, the filter, voices, mute group,
//! pressure) and the Grain tab (the first layer's grain settings, the
//! engine's grains over the sound, a touch pad for position and size) and
//! the Rec tab (the line input onto a pad); FX comes later. The pad bar's
//! tabs switch between them.
//!
//! - The screen thread owns every kit's project, handed over by
//!   `sampler::Player::take_projects` when the kits are loaded.
//! - The tab edits the playing kit's pad that was hit last, or tapped on the
//!   Pads tab's map. SHIFT + wheel and - / + pick the previous or next pad
//!   with a sound. A pad without a sound shows the Pads tab, where a sound
//!   can be put on it.
//! - An edit changes the project here (drags and turns merge into one undo
//!   step each), then the kit's engine view is built again here and sent to
//!   the audio thread in a box ([`SamplePage::take_view`]). The audio thread
//!   swaps it in and sends the old view back in the same box, to be freed
//!   here. One view travels at a time, and edits made meanwhile go with the
//!   next one, so the audio thread never builds, allocates or frees a view.
//! - A tap on the waveform plays the pad, as a hit would. In Gate, Loop and
//!   Granular modes the sound keeps going until the next tap.
//! - Each finished drag or turn writes one `sample:` line to the log, with
//!   what changed; so does each chop put on pads.
//! - On the Play tab a hit's velocity shows on the layer map, and the wheel
//!   changes the row picked (a turn, like a Q-Link's, is one undo step).
//! - Chop to pads changes the kit like any edit (one undo step), and the
//!   Play page's pad names follow ([`SamplePage::take_renamed`]).
//! - On the Grain tab the cloud shows the grains the audio thread plays for
//!   the pad: one boxed [`Cloud`] goes there, is filled and comes back
//!   about 25 times a second, only while the tab shows
//!   ([`SamplePage::take_cloud`], [`SamplePage::cloud_back`]).
//! - The Rec tab records the line input onto a pad. While the line input
//!   is on, the page keeps the sampler's input (the last 75 s, about
//!   26 MB) and sends its writing end to the audio thread
//!   ([`SamplePage::take_input`]); turned off, the writer comes back and
//!   the buffer is freed here ([`SamplePage::input_back`]). A take keeps
//!   recording while other pages show ([`SamplePage::tick`]), and lands on
//!   its pad as one undo step.
//! - The Rec tab's Auto-sample button opens its Auto-sample mode: it plays
//!   a copy of the Synth tab's patch note by note on a worker thread
//!   ([`mpfree_sampler::autosample::Job`]), offline and many times faster
//!   than real time, and makes a keygroup instrument of it on the kit's
//!   pads, one undo step. The app makes the copy
//!   ([`SamplePage::take_auto_request`], [`SamplePage::start_auto`]); the
//!   audio thread is never asked for anything. External gear is the other
//!   source, once the app gives a MIDI out ([`SamplePage::set_gear`]): its
//!   notes go out on a MIDI channel from the job's thread and its sound is
//!   read back from the line input's rolling buffer in real time.
//! - Saving: the kits go into a project as the sampler's part
//!   ([`SamplePage::save_part`]: their text, and every sound they use for
//!   the project's `samples/`, kept losslessly as WAV and named by the
//!   sound, so the same sound is one file). Opening a project
//!   ([`SamplePage::open_part`]) reads and decodes its sounds on a thread of
//!   its own; the kits change on a later [`SamplePage::tick`], and what
//!   couldn't be used is said in the log. The browser's Load puts a WAV
//!   file on the pad shown, and Load to pad on the next pad hit
//!   ([`SamplePage::load_file`]).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_chain_ui::state::Pedal;
use mpfree_chain_ui::ChainAction;
use mpfree_node::{Held, Node};
use mpfree_play::mpfree_param::{OnTrack, ParamAddr, RelAddr};
use mpfree_sampler::autosample::{Finished, Gear, Job, Plan, PlanError};
use mpfree_sampler::engine::{EngineView, PadField};
use mpfree_sampler::export;
use mpfree_sampler::keymap::{self, RootFrom};
use mpfree_sampler::model::{
    self as model, ChannelPick, ChopSet, Fade, Layer, Levels, LoopMode, Pad, PadsPlay, PlayMode,
    Project, Region, Source, SourceId, Warp, KEPT_CHOP, PADS, SLICE_KEYS_FROM, WARP_BPM,
};
use mpfree_sampler::phrase::Phrase;
use mpfree_sampler::record::{self, Input, Looped, Progress, Recording};
use mpfree_sampler::save::{self, Opened};
use mpfree_sampler::scene;
use mpfree_sampler::warp::{CopyKey, Done, Process, Warps, Worker};
use mpfree_sampler::{sound_file, wav};
use mpfree_sampler_ui::auto_tab::{
    self, AutoAction, AutoSettings, AutoState, AutoTab, Onto, Running,
};
use mpfree_sampler_ui::chop_tab::{self, ChopAction, ChopState, ChopTab};
use mpfree_sampler_ui::fx_tab::{self, FxAction, FxKnob, FxPedal, FxState, FxTab};
use mpfree_sampler_ui::grain_tab::{self, CloudNow, GrainAction, GrainState, GrainTab};
use mpfree_sampler_ui::pads_tab::{self, PadsAction, PadsState, PadsTab};
use mpfree_sampler_ui::play_tab::{self, PlayAction, PlayState, PlayTab, SceneAction};
use mpfree_sampler_ui::rec_tab::{self, RecAction, RecState, RecTab, Skip};
use mpfree_sampler_ui::send;
use mpfree_sampler_ui::source::{cache_file, SourceView};
use mpfree_sampler_ui::warp_tab::{self, WarpTab};
use mpfree_sampler_ui::wave_tab::{
    self, duration_text, mode_label, WaveAction, WaveState, WaveTab,
};
use mpfree_sampler_ui::widgets::SettingList;
use mpfree_seq::{Clip, Lock, Note};
use mpfree_transport::PPQ;
use mpfree_ui::widgets::Card;
use mpfree_ui::{EventKind, Tone, Ui, UiEvent};

use crate::gui::FromUi;
use crate::mixing::Captured;
use crate::modpanels::PadFx;
use crate::sampler::{Cloud, Playheads, RecFeed, RecSource, ViewSwap};
use mpfree_fx::{CapturePoint, StripId};

/// The Wave tab's page.
pub const PAGE: &str = "Sample";
/// The Chop tab's page.
pub const CHOP_PAGE: &str = "Sample · Chop";
/// The Play tab's page.
pub const PLAY_PAGE: &str = "Sample · Play";
/// The Pads tab: the pads, the pad's settings and the kit's sounds.
pub const PADS_PAGE: &str = "Pads · Pad";
/// The Grain tab's page.
pub const GRAIN_PAGE: &str = "Sample · Grain";
/// The Rec tab's page.
pub const REC_PAGE: &str = "Sample · Rec";
/// The Warp tab's page.
pub const WARP_PAGE: &str = "Sample · Warp";
/// The FX tab's page.
pub const FX_PAGE: &str = "Sample · FX";
/// The Rec tab's Auto-sample mode's page.
pub const AUTO_PAGE: &str = "Sample · Auto-sample";
/// The line input the sampler keeps while it is on: enough for the
/// longest Skip back (a minute, or 8 bars down to 60 BPM) and more.
pub const INPUT_SECONDS: u32 = 75;
/// The page shown while the kits load, or while the synth has the pads.
pub const EMPTY_PAGE: &str = "Sample · no sound";
pub const EMPTY: &str = "sample-empty";
pub const EMPTY_LAYOUT: &str = "col(header:60, sample-empty:*, hint:26)";
/// The pad editor's widgets all start with this (the Chop tab's with
/// `smp-chop-`, the Play tab's with `smp-play-`).
pub const PREFIX: &str = "smp-";
/// The editors whose pad bar's title names the pad and its sound.
const NAMED_BARS: [&str; 6] = [
    wave_tab::BAR,
    warp_tab::BAR,
    chop_tab::BAR,
    play_tab::BAR,
    grain_tab::BAR,
    fx_tab::BAR,
];
/// The text keyboard's keys for a pad's name and its sound's.
pub const NAME_PAD: &str = "pad";
pub const NAME_SOUND: &str = "sound";
/// The most characters a name typed may have (one from a file may be
/// longer, until shortened).
pub const NAME_MAX: usize = 32;
const CHOP_PREFIX: &str = "smp-chop-";
const PLAY_PREFIX: &str = "smp-play-";
const PADS_PREFIX: &str = "smp-pads-";
const GRAIN_PREFIX: &str = "smp-grain-";
const REC_PREFIX: &str = "smp-rec-";
const WARP_PREFIX: &str = "smp-warp-";
const AUTO_PREFIX: &str = "smp-auto-";
const FX_PREFIX: &str = "smp-fx-";
/// A Q-Link resting this long ends its turn: the next one is a new undo step.
const TURN_REST: Duration = Duration::from_secs(1);

/// The region (Wave tab) or pad (Play tab) a drag or turn started from,
/// for its log line.
struct Gesture {
    id: u64,
    kit: usize,
    pad: usize,
    before: Before,
}

enum Before {
    /// A layer's region on the Wave, Warp or Chop tab.
    Region {
        layer: usize,
        region: Box<Region>,
    },
    /// A layer's region on the Grain tab.
    Grain {
        layer: usize,
        region: Box<Region>,
    },
    Pad(Box<Pad>),
    /// A pad on the Pads tab, which may have had no sound.
    Slot(Option<Box<Pad>>),
}

/// What a Play page Q-Link turns on the pad hit last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadKnob {
    /// Semitones (SHIFT: 10 cents).
    Tune,
    /// The amp envelope's decay to silence; at the top the sound plays out.
    Decay,
}

/// The pad editor's tab on show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    /// The Pads tab (a top-level tab of its own since 12 Nov 2026), not
    /// one of the Sample tab's.
    Pads,
    Wave,
    Warp,
    Chop,
    Play,
    Grain,
    /// The effects on the pad's channel, as pedals.
    Fx,
    Rec,
    /// The Rec tab's Auto-sample mode.
    Auto,
}

/// The effects on a kit's pad's channel as the mixer says them, for the
/// FX tab ([`crate::gui::ToUi::Pedals`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PadPedals {
    pub kit: usize,
    pub pad: usize,
    pub list: Vec<Pedal>,
    /// The pad has a channel of its own: kit 1's A1 to A16 always, any
    /// other pad once one is lent to it (`mixing::PadChannels`).
    pub own: bool,
    /// A channel is free to lend the pad, when it has none.
    pub spare: bool,
    /// Where the channel plays into (the kit's track), and how many
    /// effects it holds.
    pub into: String,
    pub room: usize,
}

/// A pedal as the FX tab shows it: its name, summary, switch and main
/// knobs (the chain editor's Q-Links for it).
fn fx_pedal(p: &Pedal) -> FxPedal {
    FxPedal {
        name: p.name.clone(),
        summary: p.summary.clone(),
        on: p.on,
        knobs: p
            .knobs
            .iter()
            .take(4)
            .map(|k| FxKnob {
                label: k.label.clone(),
                text: k.text(),
                pos: k.pos,
                bipolar: k.bipolar(),
                steps: k.steps(),
            })
            .collect(),
    }
}

/// Live chop under way: the pad whose sound plays, the cuts read so far,
/// and whether the sound has been heard marking yet.
struct LiveRun {
    kit: usize,
    pad: usize,
    layer: usize,
    seen: u64,
    heard: bool,
    kept: usize,
}

/// A take being recorded, and the pad it goes onto.
struct Take {
    rec: Recording,
    kit: usize,
    pad: usize,
    pick: record::Pick,
    /// Whole bars at the tempo: it lands warped at the project tempo.
    bars: bool,
    /// A pad take's (sampler R5.7), not the Rec tab's own.
    pad_take: bool,
}

/// A pad take (sampler R5.7: an empty kit pad held + REC): the pad it
/// lands on and the Rec tab's settings as they were when it was armed,
/// waiting for its go, then recording in [`SamplePage::take`].
#[derive(Clone, Copy, Debug)]
struct PadRun {
    kit: usize,
    pad: usize,
    settings: rec_tab::RecSettings,
    /// Its take began (it may still wait for the threshold or the bar).
    begun: bool,
    /// The arm was told it records.
    said: bool,
}

/// A Flatten running (the Rec tab's third button): the pad's own mixer
/// channel recorded while the pad plays once, then kept on `target`.
struct Flatten {
    kit: usize,
    pad: usize,
    target: usize,
    velocity: u8,
    /// Seconds the pad is held before it is let go.
    hold: f32,
    /// A Toggle pad is let go by a second hit.
    toggle: bool,
    /// The Rec tab's own source, put back when it ends.
    was: RecSource,
    /// When the buffer holds the pad's channel: at once, after the audio
    /// thread restarts it (from this count), or once a new buffer has
    /// anything in it.
    ready: Ready,
    /// Seconds waited for the channel, then since the hit.
    waited: f32,
    played: Option<f32>,
    lifted: bool,
}

/// When a Flatten's buffer holds the pad's channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ready {
    Now,
    Restarted(u64),
    Written,
}

/// How long a Flatten waits for the pad's channel, and for the pad to make
/// a sound after its hold, before it gives up.
const FLATTEN_WAIT: f32 = 2.0;

/// What lets a Flatten's pad go: a second hit for a Toggle pad, else a
/// lift.
fn flatten_lift(f: &Flatten) -> FromUi {
    FromUi::SampleHit {
        kit: f.kit,
        pad: f.pad,
        velocity: f.velocity,
        down: f.toggle,
    }
}

/// How a take with `settings` records, for the log: "start now, end
/// stop, left + right".
fn take_plan(settings: &rec_tab::RecSettings, sources: &[String]) -> String {
    format!(
        "start {}, end {}, {}",
        rec_tab::value_text(settings, rec_tab::Row::Start, sources).to_lowercase(),
        match settings.end {
            rec_tab::EndAt::Length => format!(
                "{} then {}",
                rec_tab::value_text(settings, rec_tab::Row::Length, sources),
                rec_tab::value_text(settings, rec_tab::Row::Tail, sources).to_lowercase()
            ),
            _ => rec_tab::value_text(settings, rec_tab::Row::End, sources).to_lowercase(),
        },
        settings.pick.label().to_lowercase(),
    )
}

/// What a finished take did after its bars, for its log line: "recorded",
/// with how long it rang out or how it was made a loop.
fn take_what(rec: &Recording) -> String {
    let (Some(cut), Progress::Done { frames }) = (rec.cut_frames(), rec.progress()) else {
        return "recorded".to_string();
    };
    let rate = rec.rate();
    match (rec.tail(), rec.looped()) {
        (record::Tail::Ring(_), _) => format!(
            "recorded, rang out {}",
            duration_text(frames.saturating_sub(cut), rate)
        ),
        (_, Some(Looped::GoingRound)) => "recorded as a loop, cut: it was going round".to_string(),
        (_, Some(Looped::Folded { frames })) => format!(
            "recorded as a loop, {} of ringing folded onto its start",
            duration_text(frames, rate)
        ),
        (_, Some(Looped::SecondTime)) => "recorded as a loop, its second time round".to_string(),
        _ => "recorded".to_string(),
    }
}

/// A Process under way (the Warp tab's Process button): the stretch on a
/// thread of its own, how far it has got, and how to stop it.
struct Processing {
    kit: usize,
    what: Process,
    /// The pad, as the log names it ("A5").
    at: String,
    /// What it does, in words: "stretched from 92.0 to 120.0 BPM and 3
    /// semitones higher".
    text: String,
    rx: Receiver<Result<(Vec<f32>, Duration), String>>,
    stop: Arc<AtomicBool>,
    /// How far it has got, 0 to 1, as `f32` bits.
    done: Arc<AtomicU32>,
    /// What the Process button shows, as last shown.
    shown: f32,
}

/// Output frames a Process makes between looks at whether it was stopped:
/// a few ms of work on the MPC.
const PROCESS_STEP: usize = 8192;

/// A region whose loudest frame is under this is silent: Normalize leaves
/// it as it is.
const NORMALIZE_SILENT_DB: f32 = -96.0;

/// What Process makes of `r` (layer `layer` of pad `pad`) at the project
/// tempo `tempo`, moved `semitones`: the job, and what it does in words
/// ("stretched from 92.0 to 120.0 BPM and 3 semitones higher"), or no
/// words when the new sound would come out the same as the region plays.
/// A stretched region goes from its warp tempo to the project's through
/// its pins; a repitched one is stretched the same way and moved by the
/// semitones the tape speed would, so it sounds as it plays.
fn process_of(
    pad: usize,
    layer: usize,
    r: &Region,
    tempo: f64,
    semitones: i32,
) -> (Process, Option<String>) {
    let mut p = Process {
        pad,
        layer,
        ratio: 1.0,
        semitones: semitones as f32,
    };
    let mut parts = Vec::new();
    if let Some(w) = r.warp.as_ref() {
        let from = f64::from(w.bpm);
        let moved = (from - tempo).abs() > 1e-3;
        if moved || w.pinned() {
            p.ratio = from / tempo;
            let pins = if w.pinned() {
                ", its pins on their beats"
            } else {
                ""
            };
            match w.method {
                model::WarpMethod::Stretch => {
                    parts.push(format!("stretched from {from:.1} to {tempo:.1} BPM{pins}"))
                }
                model::WarpMethod::Repitch => {
                    p.semitones += (12.0 * (tempo / from).log2()) as f32;
                    parts.push(format!("repitched from {from:.1} to {tempo:.1} BPM"));
                }
            }
        }
    }
    if semitones != 0 {
        parts.push(format!(
            "{} semitone{} {}",
            semitones.abs(),
            if semitones.abs() == 1 { "" } else { "s" },
            if semitones > 0 { "higher" } else { "lower" }
        ));
    }
    let text = (!parts.is_empty()).then(|| parts.join(" and "));
    (p, text)
}

/// An auto-sample running, and where its instrument goes.
struct AutoJob {
    job: Job,
    kit: usize,
    onto: Onto,
    /// External gear, played in real time (else the synth, offline).
    gear: bool,
    /// The notes it samples, low to high, and takes a note.
    notes: Vec<u8>,
    per_note: usize,
    /// The gear's MIDI out, told (no bytes) when the job is over, so it
    /// ends every note the job left on.
    end: Option<MidiSend>,
}

impl AutoJob {
    fn running(&self) -> Running {
        let (done, total) = self.job.progress();
        let k = (done / self.per_note.max(1)).min(self.notes.len().saturating_sub(1));
        Running {
            done,
            total,
            note: (done < total).then(|| self.notes.get(k).copied()).flatten(),
        }
    }
}

/// A MIDI out the Auto-sample mode can play external gear through: its
/// name as the page shows it ("MIDI Out (DIN)") and how a message goes out
/// on it. The app gives it once that MIDI out is open
/// ([`SamplePage::set_gear`]); the job sends from its own thread.
#[derive(Clone)]
pub struct GearOut {
    pub name: String,
    pub send: MidiSend,
}

/// Sends one MIDI message out of a port, from any thread.
pub type MidiSend = Arc<dyn Fn(&[u8]) + Send + Sync>;

/// The folder in Samples that exported pads go to, each in a folder of
/// its own named after the pad.
pub const EXPORTS: &str = "Exports";

/// Writes an exported pad's files into a new folder named after it in
/// Samples / Exports ("Break 92", or "Break 92 2" when that is taken),
/// making Exports if it isn't there. Each file is written whole or not at
/// all; a failure part way leaves the files written so far. Call it off
/// the screen's thread: a long sound's files are many megabytes. Returns
/// the folder.
pub fn save_export(
    lib: &mpfree_library::Library,
    e: &export::Export,
) -> std::io::Result<mpfree_library::Loc> {
    use mpfree_library::{Loc, Place};
    let samples = Loc::top(Place::Samples);
    let exports = samples.join(EXPORTS)?;
    if lib.list(&exports).is_err() {
        lib.new_folder(&samples, EXPORTS)?;
    }
    let folder = lib.new_folder(&exports, &e.name)?;
    for (name, bytes) in &e.files {
        lib.write_new(&folder, name, bytes)?;
    }
    Ok(folder)
}

/// What the Rec tab's Source row offers, by name: the line input, then the
/// mix as heard (the master after Master safety), each track, the delay's
/// return and the playing kit's 16 pad channels, each after its fader.
pub fn rec_sources() -> Vec<(String, RecSource)> {
    use crate::mixing::{self, DELAY_RETURN, DRUMS, LOOP, PADS, SYNTH};
    let after = |s: StripId| (mixing::name(s), RecSource::Mix(s, CapturePoint::AfterFader));
    let mut v = vec![
        ("Line in".to_string(), RecSource::Line),
        (
            "Master".to_string(),
            RecSource::Mix(StripId::Master, CapturePoint::AfterSafety),
        ),
    ];
    v.extend([DRUMS, SYNTH, LOOP].map(|t| after(StripId::Track(t))));
    v.push(after(StripId::Return(DELAY_RETURN)));
    v.extend((0..PADS).map(|p| after(StripId::Pad(p))));
    v
}

/// What the Sample tab's sounds hold in memory (Dev > CPU by engine,
/// `memory.rs`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SampleMemory {
    /// Every kit's sounds, those undo keeps included, each counted once.
    pub sounds: usize,
    pub count: usize,
    /// The stretched copies warped pads play.
    pub copies: usize,
    /// The Rec tab's input buffer, while it is held.
    pub input: usize,
}

/// The sampler's input buffer's size: stereo `f32`.
pub fn input_bytes(rate: u32) -> u64 {
    u64::from(INPUT_SECONDS) * u64::from(rate) * 2 * 4
}

/// Gives a sound file's bytes from its name in a project's `samples/`
/// ([`SamplePage::open_part`]).
/// A kit as a kit file: its name, its `kit.txt` and each sound by file name.
pub type KitFile = (String, String, Vec<(String, Source)>);

/// A kit file being read: the answer, the kit it goes on, its name.
type KitLoading = (Receiver<Read>, usize, String);

/// Kits read from a file, with what was converted to the output's rate
/// on the way ([`opened_at_rate`]), one log line each.
type Read = Result<(Opened, Vec<String>), String>;
/// A project's kits as opened: [`Read`]'s, and the views read from the
/// sounds' cache files ([`cached_views`]).
type ProjectRead = Result<(Opened, Vec<String>, Vec<Arc<SourceView>>), String>;

pub type ReadSample = Box<dyn Fn(&str) -> Result<Vec<u8>, String> + Send>;

pub struct SamplePage {
    projects: Vec<Project>,
    names: Vec<String>,
    /// The kits and their names as the app first gave them (kits.bin):
    /// what New project goes back to ([`SamplePage::reset_kits`]).
    factory: Option<(Vec<Project>, Vec<String>)>,
    rate: u32,
    /// Waveform peaks, hits, tempo and root note, per source, worked out
    /// off the screen thread the first time a pad using the source is
    /// shown ([`crate::views`]).
    sources: crate::views::Views,
    kit: usize,
    pad: usize,
    tab: Option<WaveTab>,
    state: Option<WaveState>,
    editor: Editor,
    /// The Sample tab's editor, kept while the Pads tab shows.
    sample_editor: Editor,
    /// The Chop tab, added the first time it is opened.
    chop: Option<ChopTab>,
    chop_state: Option<ChopState>,
    /// The Play tab, added the first time it is opened.
    play: Option<PlayTab>,
    play_state: Option<PlayState>,
    /// The shown pad's last hit, for the Play tab's layer map.
    velocity: Option<u8>,
    /// The Pads tab, added the first time it shows.
    pads: Option<PadsTab>,
    pads_state: Option<PadsState>,
    /// The effects on the mixer's pad channels, by kit and pad, for the
    /// Pad list's Effects row ([`SamplePage::set_pad_fx`]); None until the
    /// mixer says.
    pad_fx: Option<Vec<PadFx>>,
    /// The Grain tab, added the first time it is opened.
    grain: Option<GrainTab>,
    grain_state: Option<GrainState>,
    /// The cloud's box, while it is here and not on the audio thread.
    cloud: Option<Box<Cloud>>,
    /// The last cloud shown had grains, a playhead or grains on other pads.
    cloud_shown: bool,
    /// Kits whose pads were renamed or replaced, for the Play page.
    renamed: Vec<usize>,
    /// The kit or pad changed since the page was last built.
    stale: bool,
    /// The last touch's tab answered it with an action (see
    /// [`SamplePage::event`]).
    answered: bool,

    /// The wheel's push opened the picked row of the settings list: the
    /// wheel changes its value (`app::RowKey`). Kept here so a list made
    /// again (another pad or tab) shows it too.
    editing: bool,
    /// Kits whose new view the audio thread doesn't have yet.
    dirty: Vec<bool>,
    /// A view is on its way to the audio thread (or back).
    in_flight: bool,
    /// The scene crossfader, once moved: the kit and where it is (0 is
    /// A). A new view of that kit clears the engine's live values, so they
    /// go again after it.
    morph: Option<(usize, f32)>,
    /// The pad a tap on the waveform holds down (Gate, Loop, Granular),
    /// and whether it went to the pads as one of the 16 (else it plays
    /// as an audition, off the page on the pads).
    auditioning: Option<(usize, bool)>,
    /// The bank of the kit on the pads (PAD BANK, the Pads tab's strip);
    /// None while the synth has them.
    pad_page: Option<usize>,
    last_turn: Option<Instant>,
    gesture: Option<Gesture>,
    /// A Play page Q-Link turning a pad: the pad, the knob and the turn's
    /// undo id (one undo step a turn).
    play_turn: Option<(usize, PadKnob, u64)>,
    shown_heads: Vec<u64>,
    lines: Vec<String>,
    /// What the pad take ([`SamplePage::pad_take`]) tells the arm next.
    pad_said: Option<crate::rec_arm::PadNow>,
    /// The pad take armed, if any.
    pad_run: Option<PadRun>,
    /// Refused edits, for the screen to say in place.
    told: Vec<String>,
    /// Edits applied, and views sent to the audio thread.
    pub edits: u64,
    pub views_sent: u64,
    /// A change saved with the project that the audio thread doesn't
    /// hear of (a chop kept): the work is unsaved, though no view goes.
    unsaved: bool,
    /// The project tempo and the stretched copies warped pads play.
    warps: Warps,
    /// The thread that makes them, started the first time one is wanted.
    stretcher: Option<Worker>,
    /// Copies asked of it and not back yet.
    stretching: Vec<CopyKey>,
    /// Copies refused for want of memory at this tempo, said once each
    /// (their pads repitch).
    unstretched: Vec<CopyKey>,
    /// The memory free for new loads ([`crate::memory::free_bytes`]; a
    /// test gives its own).
    mem_free: fn() -> Option<u64>,
    /// The Warp tab, added the first time it is opened. It shows the Wave
    /// tab's state.
    warp: Option<WarpTab>,
    /// The Rec tab, added the first time it is opened.
    rec: Option<RecTab>,
    rec_state: Option<RecState>,
    /// The line input is on (its row on the Looper tab's MENU).
    line_on: bool,
    /// What the Rec tab can record from (its Source row), by name: the
    /// line input first, then points in the mix (resampling).
    rec_sources: Vec<(String, RecSource)>,
    /// The source picked, and the one the audio thread was last told of.
    source: RecSource,
    source_sent: Option<RecSource>,
    /// The sampler's input while its source comes in (the line input while
    /// it is on; a point in the mix always), and its writing end until it
    /// goes to the audio thread.
    input: Option<Input>,
    feed: Option<Box<RecFeed>>,
    /// The audio thread has the writer, and should hand it back.
    feeding: bool,
    want_back: bool,
    take: Option<Take>,
    flatten: Option<Flatten>,
    /// Hits and lifts for the audio thread (Flatten), and live chop's
    /// stop.
    hits: Vec<FromUi>,
    /// Live chop on the Chop tab, while it runs.
    live: Option<LiveRun>,
    /// The cuts the sampler has marked so far (from the playheads).
    marks_seen: u64,
    /// Takes made, for their names.
    takes: u32,
    /// Captures of an effect's memory kept, for their names.
    captures: u32,
    /// The last take, said in a line on the tab.
    last_take: String,
    /// The Auto-sample mode, added the first time it is opened.
    auto: Option<AutoTab>,
    auto_state: Option<AutoState>,
    /// The Synth tab's patch by name ("Poly Pluck"), as the app last said.
    auto_synth: Option<String>,
    /// A plan Start asked for: the app makes the patch's copy for it.
    auto_request: Option<Plan>,
    /// A number to be typed, asked for by a tab (the Warp tempo): the
    /// screen opens the number pad ([`SamplePage::take_number_ask`]).
    number_ask: Option<mpfree_ui::numpad::NumberAsk>,
    /// Names to be typed (a tap on the pad bar's title: the pad's and its
    /// sound's): the screen opens the text keyboard
    /// ([`SamplePage::take_text_ask`]).
    text_ask: Option<Vec<mpfree_ui::textpad::TextAsk>>,
    auto_job: Option<AutoJob>,
    /// The Warp tab's Process, while one runs.
    processing: Option<Processing>,
    /// The last instrument made, or why not, in a line.
    auto_last: String,
    /// The FX tab, added the first time it is opened.
    fx: Option<FxTab>,
    fx_state: Option<FxState>,
    /// The effects on a pad's channel as the mixer last said, while the
    /// FX tab shows ([`SamplePage::set_pedals`]).
    pedals: Option<PadPedals>,
    /// The kit and pad whose effects the app was asked to send
    /// ([`SamplePage::take_fx`]).
    pedals_asked: Option<(usize, usize)>,
    /// What the FX tab asks of the mixer from a Q-Link, the wheel or a
    /// lift (a touch's goes out with the touch).
    fx_out: Vec<FromUi>,
    /// The effects' knobs turned since the last lift: each is let go then.
    fx_touched: Vec<ParamAddr>,
    /// The MIDI out external gear is auto-sampled through, when the app
    /// has one open.
    gear_out: Option<GearOut>,
    /// A project's kits being read and decoded ([`SamplePage::open_part`]).
    opening: Option<Receiver<ProjectRead>>,
    /// A kit file being read and decoded ([`SamplePage::open_kit`]): it
    /// goes in place of this kit, under this name.
    kit_loading: Option<KitLoading>,
    /// A file being read for the browser's Load (true: Load to pad).
    loading: Option<(Receiver<LoadedPad>, bool)>,
    /// A sound Load to pad put on the next pad hit.
    assign: Option<LoadedSound>,
    /// Send to pad: the kit and pad whose copy goes on the next pad hit.
    sending: Option<(usize, usize)>,
    /// The app can write files (Put on Files on the Chop tab).
    files: bool,
    /// Pads exported, for the app to write.
    exports: Vec<export::Export>,
    /// The kit an opened project was playing, for the app to switch to.
    wanted_kit: Option<usize>,
    /// Put on New kit (sampler R4.6) as the app offers it: Ok with the
    /// empty kit the slices would go on, for a new track, or Err saying
    /// why not now. None until the app says, and then the Chop tab has no
    /// New kit.
    new_kit: Option<Result<usize, String>>,
    /// A chop to a new kit just made, for the app to put on a new track
    /// ([`SamplePage::take_new_kit_track`]).
    new_kit_track: Option<NewKitTrack>,
    /// The kit a chop to a new kit filled and the pad of its first slice:
    /// the tab shows that pad once the pads play the kit.
    new_kit_pad: Option<(usize, usize)>,
}

/// A chop to a new kit (sampler R4.6), for the app: the kit the slices
/// went on, which a new track is to play, and the phrase for that track
/// when Phrase was on.
#[derive(Debug)]
pub struct NewKitTrack {
    pub kit: usize,
    pub phrase: Option<Box<Clip>>,
}

impl SamplePage {
    /// Adds the page for a pad without a sound to `ui` (the Wave tab's page
    /// is added with the first sound shown).
    pub fn add(ui: &mut Ui) -> Result<SamplePage, String> {
        ui.add(EMPTY, Card::new("Sample"));
        ui.add_page(EMPTY_PAGE, EMPTY_LAYOUT)?;
        let mut p = SamplePage {
            projects: Vec::new(),
            names: Vec::new(),
            factory: None,
            rate: 44_100,
            sources: crate::views::Views::new(),
            kit: 0,
            pad: 0,
            tab: None,
            state: None,
            editor: Editor::Wave,
            sample_editor: Editor::Wave,
            chop: None,
            chop_state: None,
            play: None,
            play_state: None,
            velocity: None,
            pads: None,
            pads_state: None,
            pad_fx: None,
            grain: None,
            grain_state: None,
            cloud: Some(Box::default()),
            cloud_shown: false,
            renamed: Vec::new(),
            stale: true,
            answered: false,
            editing: false,
            dirty: Vec::new(),
            in_flight: false,
            morph: None,
            auditioning: None,
            pad_page: Some(0),
            last_turn: None,
            gesture: None,
            play_turn: None,
            shown_heads: Vec::new(),
            lines: Vec::new(),
            pad_said: None,
            pad_run: None,
            told: Vec::new(),
            edits: 0,
            views_sent: 0,
            unsaved: false,
            warps: Warps::default(),
            stretcher: None,
            stretching: Vec::new(),
            unstretched: Vec::new(),
            mem_free: crate::memory::free_bytes,
            warp: None,
            rec: None,
            rec_state: None,
            line_on: false,
            rec_sources: rec_sources(),
            source: RecSource::Line,
            // The audio thread starts on the line input.
            source_sent: Some(RecSource::Line),
            input: None,
            feed: None,
            feeding: false,
            want_back: false,
            take: None,
            flatten: None,
            hits: Vec::new(),
            live: None,
            marks_seen: 0,
            takes: 0,
            captures: 0,
            last_take: String::new(),
            auto: None,
            auto_state: None,
            auto_synth: None,
            auto_request: None,
            number_ask: None,
            text_ask: None,
            auto_job: None,
            processing: None,
            auto_last: String::new(),
            fx: None,
            fx_state: None,
            pedals: None,
            pedals_asked: None,
            fx_out: Vec::new(),
            fx_touched: Vec::new(),
            gear_out: None,
            opening: None,
            kit_loading: None,
            loading: None,
            assign: None,
            sending: None,
            files: false,
            exports: Vec::new(),
            wanted_kit: None,
            new_kit: None,
            new_kit_track: None,
            new_kit_pad: None,
        };
        p.show_empty(ui);
        Ok(p)
    }

    /// The kits' projects (in kit order; a kit after them, the synth, has
    /// none) and the rate their engine views are built for. Their undo
    /// history starts here, so UNDO never takes a loaded kit apart.
    pub fn set_kits(&mut self, ui: &mut Ui, mut projects: Vec<Project>, rate: u32) {
        self.lift();
        for p in &mut projects {
            p.clear_history();
        }
        self.dirty = vec![false; projects.len()];
        self.in_flight = false;
        if self.factory.is_none() {
            // The sounds are shared (Arc): this copy costs the lists only.
            self.factory = Some((projects.clone(), Vec::new()));
        }
        self.projects = projects;
        self.rate = rate;
        // Views of sounds still here are kept (a kit loaded again, or a
        // sound in more than one kit); the rest are forgotten.
        let here: HashSet<SourceId> = self
            .projects
            .iter()
            .flat_map(|p| p.sources())
            .map(|s| s.id())
            .collect();
        self.sources.keep_only(&here);
        self.warps.clear();
        self.cancel_stretching();
        if self.take.take().is_some() {
            self.lines
                .push("sample: the take was dropped (the kits were loaded again)".to_string());
        }
        if self.pad_run.take().is_some() {
            self.pad_said = Some(crate::rec_arm::PadNow::Ended);
        }
        if let Some(j) = self.auto_job.take() {
            if let Some(end) = &j.end {
                end(&[]);
            }
            // Dropping it cancels it; its thread ends on its own.
            self.lines.push(
                "sample: the auto-sample was cancelled (the kits were loaded again)".to_string(),
            );
        }
        // Kits with warped pads get their views at the project tempo.
        for k in 0..self.projects.len() {
            self.dirty[k] = self.warped(k);
        }
        self.pad = self.with_sound(self.pad, 1).unwrap_or(self.pad);
        self.refresh(ui);
    }

    /// Kit `kit`'s name as the page has it (a kit loaded or opened names
    /// it).
    pub fn kit_name(&self, kit: usize) -> Option<&str> {
        self.names.get(kit).map(String::as_str)
    }

    /// The kits' names, for the page and the log.
    pub fn set_names(&mut self, names: Vec<String>) {
        if let Some((_, n)) = self.factory.as_mut().filter(|(_, n)| n.is_empty()) {
            n.clone_from(&names);
        }
        self.names = names;
        self.stale = true;
    }

    /// The kit the pads play now. Builds the page again when it is shown.
    pub fn set_kit(&mut self, k: usize) {
        if k == self.kit {
            return;
        }
        self.lift();
        self.kit = k;
        self.pad = match self.new_kit_pad.take() {
            // A chop to a new kit: its first slice.
            Some((kit, pad)) if kit == k => pad,
            _ => self.with_sound(self.pad, 1).unwrap_or(self.pad),
        };
        if self.sending.take().is_some() {
            self.lines
                .push("sample: Send to pad called off (another kit)".into());
        }
        self.stale = true;
    }

    /// A pad was hit: the tab edits it (or, with the pads playing notes,
    /// the keygroup that plays its note) from now on.
    pub fn hit(&mut self, pad: usize) {
        if self.live.is_some() {
            // Live chop: the hit marks a cut and the pad stays.
            return;
        }
        if self.auditioning.is_some_and(|(p, _)| p == pad) {
            // Its own release lets it go.
            self.auditioning = None;
        }
        let pad = self.played(pad);
        self.pick(pad);
    }

    /// Pad `pad` was hit at `velocity`. With the Play tab on screen
    /// (`shown`) at its Stops or Also plays row, and no Load to pad or
    /// Send to pad waiting for a hit, the pad goes into that set or out of
    /// it (an undo step each); otherwise the tab edits the
    /// pad hit from now on ([`SamplePage::hit`]) and shows its velocity
    /// ([`SamplePage::hit_velocity`]).
    pub fn press(&mut self, ui: &mut Ui, pad: usize, velocity: u8, shown: bool) {
        if shown && self.play_takes_pads(ui) && self.assign.is_none() && self.sending.is_none() {
            let a = match (self.play.as_mut(), self.play_state.as_ref()) {
                (Some(tab), Some(st)) => tab.pad_hit(ui, st, pad),
                _ => None,
            };
            if let Some(a) = a {
                self.lift();
                let mut out = Vec::new();
                self.act_play(ui, a, &mut out);
                self.lift();
            }
            return;
        }
        self.hit(pad);
        self.hit_velocity(ui, pad, velocity);
    }

    /// Whether the Play tab shows, at a row that pad hits edit (Stops or
    /// Also plays): the hint says so.
    pub fn play_takes_pads(&self, ui: &Ui) -> bool {
        self.page() == PLAY_PAGE && self.play.as_ref().is_some_and(|t| t.takes_pads(ui))
    }

    /// Picks pad `pad` itself (a tap on its cell or its zone), whatever
    /// the pads play.
    /// The pads show bank `page` of the kit (PAD BANK, or the Pads tab's
    /// strip; None while the synth has them): the tabs move to the same
    /// pad of that bank, so the pad they edit is one the pads play.
    pub fn set_pad_page(&mut self, page: Option<usize>) {
        self.pad_page = page;
        if let Some(page) = page {
            self.pick(page * 16 + self.pad % 16);
        }
    }

    /// A tap on the tabs playing kit pad `pad`: one of the 16 pads while
    /// it is on the bank the pads show (so it lights, records and plays
    /// as a hit would), else an audition of the pad.
    fn pad_press(&self, pad: usize, down: bool) -> (FromUi, bool) {
        match self.pad_page {
            Some(page) if pad / 16 != page => (FromUi::Audition { pad, down }, false),
            Some(_) => (
                FromUi::Pad {
                    pad: pad % 16,
                    down,
                },
                true,
            ),
            None => (FromUi::Pad { pad, down }, true),
        }
    }

    /// Lets go of kit pad `pad` as [`SamplePage::pad_press`] pressed it.
    fn pad_release(pad: usize, on_pads: bool) -> FromUi {
        if on_pads {
            FromUi::Pad {
                pad: pad % 16,
                down: false,
            }
        } else {
            FromUi::Audition { pad, down: false }
        }
    }

    fn pick(&mut self, pad: usize) {
        if pad == self.pad || pad >= PADS {
            return;
        }
        self.lift();
        self.pad = pad;
        self.velocity = None;
        self.stale = true;
    }

    /// Whether the Play tab shows its keyboard.
    pub fn play_keys_shown(&self) -> bool {
        self.play.as_ref().is_some_and(PlayTab::keys_shown)
    }

    /// What Q-Links 1 to 4 turn on the Play page now, for the hint line
    /// (they follow its section): "Attack, Decay, Sustain, Release".
    pub fn play_qlinks_text(&self) -> String {
        self.play
            .as_ref()
            .map_or_else(String::new, PlayTab::qlinks_text)
    }

    /// The pad hit last was hit at `velocity` (after [`SamplePage::hit`]):
    /// the Play tab's layer map shows it.
    pub fn hit_velocity(&mut self, ui: &mut Ui, pad: usize, velocity: u8) {
        if self.live.is_some() {
            self.light(ui, self.played(pad), true);
            return;
        }
        if let Some(l) = self.assign.take() {
            // Load to pad: this hit picks the pad.
            let kit = self.kit;
            self.keep_loaded(ui, kit, pad, l);
        }
        if let Some((kit, from)) = self.sending.take() {
            // Send to pad: this hit picks the pad the copy goes on.
            self.send(ui, kit, from, pad);
        }
        let pad = self.played(pad);
        self.light(ui, pad, true);
        if pad != self.pad {
            return;
        }
        self.velocity = Some(velocity);
        if !self.stale {
            if let Some(tab) = self.play.as_mut() {
                tab.set_velocity(ui, self.velocity);
            }
        }
    }

    /// The previous (`delta` < 0) or next pad with a sound.
    pub fn step_pad(&mut self, delta: i32) {
        let from = (self.pad as i32 + delta.signum()).rem_euclid(PADS as i32) as usize;
        if let Some(p) = self.with_sound(from, delta.signum()) {
            self.hit(p);
        }
    }

    /// The first pad from `from` on (wrapping round, in the direction of
    /// `dir`) that has a sound in the kit.
    fn with_sound(&self, from: usize, dir: i32) -> Option<usize> {
        let p = self.projects.get(self.kit)?;
        let dir = if dir < 0 { PADS - 1 } else { 1 };
        (0..PADS)
            .map(|i| (from + i * dir) % PADS)
            .find(|&pad| p.pad(pad).is_some_and(|pd| !pd.layers.is_empty()))
    }

    #[cfg(test)]
    pub fn state(&self) -> Option<&WaveState> {
        self.state.as_ref()
    }

    /// What the sounds hold in memory: every kit's sounds (a sound two
    /// kits share counted once), the stretched copies, the input buffer.
    pub fn memory(&self) -> SampleMemory {
        let mut seen: Vec<(usize, usize)> = self
            .projects
            .iter()
            .flat_map(Project::held)
            .map(|s| (s.data().addr(), s.data().bytes()))
            .collect();
        seen.sort_unstable();
        seen.dedup();
        SampleMemory {
            sounds: seen.iter().map(|&(_, n)| n).sum(),
            count: seen.len(),
            copies: self.warps.bytes(),
            input: if self.input.is_some() {
                input_bytes(self.rate) as usize
            } else {
                0
            },
        }
    }

    /// Whether the page has kit `kit`'s sounds to edit (the synth's kit
    /// has none).
    pub fn has_kit(&self, kit: usize) -> bool {
        kit < self.projects.len()
    }

    /// Kit `kit`'s project, to change behind the page's back (tests).
    #[cfg(test)]
    pub fn project_mut(&mut self, kit: usize) -> Option<&mut Project> {
        self.projects.get_mut(kit)
    }

    #[cfg(test)]
    pub fn project(&self, kit: usize) -> Option<&Project> {
        self.projects.get(kit)
    }

    #[cfg(test)]
    pub fn chop(&self) -> Option<(&ChopTab, &ChopState)> {
        self.chop.as_ref().zip(self.chop_state.as_ref())
    }

    /// A pad let go: the Pads tab's map stops lighting it.
    pub fn pad_up(&mut self, ui: &mut Ui, pad: usize) {
        let pad = self.played(pad);
        self.light(ui, pad, false);
    }

    /// The pad that sounds when pad `pad` is hit: itself; with the kit's
    /// pads playing notes, the first keygroup holding its note; with 16
    /// Levels, the pad they play.
    fn played(&self, pad: usize) -> usize {
        let Some(p) = self.projects.get(self.kit) else {
            return pad;
        };
        match p.pads_play() {
            PadsPlay::Keys { from } if pad < PADS => p
                .keygroups_for(from.saturating_add(pad as u8))
                .first()
                .copied()
                .unwrap_or(pad),
            PadsPlay::Levels { pad: of, .. } if pad < PADS => usize::from(of),
            _ => pad,
        }
    }

    /// The kit the pads play now (past the kits: the synth).
    pub fn kit(&self) -> usize {
        self.kit
    }

    /// The pad the page edits: the one hit last (with 16 Levels on, the
    /// pad they play).
    pub fn pad(&self) -> usize {
        self.pad
    }

    /// The pad 16 Levels play and what they step through, while the
    /// playing kit's pads play 16 Levels.
    pub fn levels(&self) -> Option<(usize, Levels)> {
        match self.projects.get(self.kit)?.pads_play() {
            PadsPlay::Levels { pad, by } => Some((usize::from(pad), by)),
            _ => None,
        }
    }

    /// The pad picked as the browser's Load names it (`"A5"`), or None
    /// while the pads play the synth (no kit to load onto).
    pub fn load_target(&self) -> Option<String> {
        (self.kit < self.projects.len()).then(|| self.pad_text(self.kit, self.pad))
    }

    /// Pad `pad`'s name as the 16 Levels panel shows it ("A2") and its
    /// sound's name (empty without one).
    pub fn pad_names(&self, pad: usize) -> (String, String) {
        let name = self
            .projects
            .get(self.kit)
            .and_then(|p| p.pad(pad))
            .map(|p| p.name.clone())
            .unwrap_or_default();
        (self.pad_text(self.kit, pad), name)
    }

    /// 16 Levels of `pad` by `by` on the playing kit, or (None) its pads
    /// back to their own sounds: one undo step and a `sample:` line, as
    /// the Play tab's Pads play row. False if there is no kit to change.
    pub fn set_levels(&mut self, ui: &mut Ui, levels: Option<(usize, Levels)>) -> bool {
        if self.projects.get(self.kit).is_none() {
            return false;
        }
        let play = match levels {
            Some((pad, by)) if pad < PADS => PadsPlay::Levels { pad: pad as u8, by },
            Some(_) => return false,
            None => PadsPlay::Pads,
        };
        if self.projects[self.kit].pads_play() == play {
            return true;
        }
        let mut out = Vec::new();
        self.act_play(ui, PlayAction::PadsPlay(play), &mut out);
        self.lift();
        if let Some((pad, _)) = levels {
            self.pick(pad);
        }
        true
    }

    fn light(&mut self, ui: &mut Ui, pad: usize, down: bool) {
        if let (Some(tab), Some(st)) = (self.pads.as_ref(), self.pads_state.as_ref()) {
            tab.set_lit(ui, st, pad, down);
        }
    }

    #[cfg(test)]
    pub fn pads(&self) -> Option<(&PadsTab, &PadsState)> {
        self.pads.as_ref().zip(self.pads_state.as_ref())
    }

    #[cfg(test)]
    pub fn play(&self) -> Option<(&PlayTab, &PlayState)> {
        self.play.as_ref().zip(self.play_state.as_ref())
    }

    #[cfg(test)]
    pub fn grain(&self) -> Option<(&GrainTab, &GrainState)> {
        self.grain.as_ref().zip(self.grain_state.as_ref())
    }

    #[cfg(test)]
    pub fn fx(&self) -> Option<(&FxTab, &FxState)> {
        self.fx.as_ref().zip(self.fx_state.as_ref())
    }

    #[cfg(test)]
    pub fn editor(&self) -> Editor {
        self.editor
    }

    /// The page to show: the Pads tab's view; the editor's tab; or the
    /// page that says the kits are loading, the synth has the pads or the
    /// pad has no sound.
    pub fn page(&self) -> &'static str {
        match self.editor {
            Editor::Pads => match (&self.pads, &self.pads_state) {
                (Some(_), Some(_)) => PADS_PAGE,
                _ => EMPTY_PAGE,
            },
            Editor::Chop if self.chop_state.is_some() && self.chop.is_some() => CHOP_PAGE,
            Editor::Play if self.play_state.is_some() && self.play.is_some() => PLAY_PAGE,
            Editor::Grain if self.grain_state.is_some() && self.grain.is_some() => GRAIN_PAGE,
            Editor::Rec if self.rec_state.is_some() && self.rec.is_some() => REC_PAGE,
            Editor::Fx if self.fx_state.is_some() && self.fx.is_some() => FX_PAGE,
            Editor::Auto if self.auto_state.is_some() && self.auto.is_some() => AUTO_PAGE,
            Editor::Wave if self.state.is_some() && self.tab.is_some() => PAGE,
            Editor::Warp if self.state.is_some() && self.warp.is_some() => WARP_PAGE,
            _ => EMPTY_PAGE,
        }
    }

    /// Builds the page again if the kit or pad changed since. Call it
    /// before showing the page.
    pub fn refresh_if_stale(&mut self, ui: &mut Ui) {
        if self.stale {
            self.refresh(ui);
        }
        self.show_editing(ui);
    }

    // ------------------------------------------------------------ rows

    /// The settings list of the page showing, if it has one: what the
    /// wheel picks rows of, and - / + step (`app::RowKey`).
    fn list(&self) -> Option<&'static str> {
        Some(match self.page() {
            PAGE => wave_tab::REGION,
            WARP_PAGE => warp_tab::LIST,
            CHOP_PAGE => chop_tab::LIST,
            PLAY_PAGE => play_tab::LIST,
            GRAIN_PAGE => grain_tab::LIST,
            REC_PAGE => rec_tab::LIST,
            FX_PAGE => fx_tab::LIST,
            AUTO_PAGE => auto_tab::LIST,
            PADS_PAGE => pads_tab::LIST,
            _ => return None,
        })
    }

    /// Whether the page showing has a settings list (a pad with no sound,
    /// the Browse view and the loading page have none).
    pub fn has_list(&self) -> bool {
        self.list().is_some()
    }

    /// The wheel picking rows: the list's next or previous row (`delta`
    /// rows on, stopping at the ends), and the tap on it that the page
    /// then gets, for the GUI to hand on as a touch. None when the page
    /// has no list or the row did not move.
    pub fn pick_row(&mut self, ui: &mut Ui, delta: i32) -> Option<UiEvent> {
        let name = self.list()?;
        let editing = self.editing;
        let l = ui.get_mut::<SettingList>(name)?;
        let last = l.rows.len().checked_sub(1)? as i64;
        let k = (l.selected as i64 + i64::from(delta)).clamp(0, last) as usize;
        l.editing = editing;
        if k == l.selected {
            return None;
        }
        l.selected = k;
        Some(UiEvent {
            widget: name.to_string(),
            kind: EventKind::Select(k),
        })
    }

    /// The minus and plus buttons (or the wheel, once its push opened the row): the step of the
    /// row picked, as a tap on its arrow, for the GUI to hand on. None on
    /// the pages whose wheel already stepped the row picked (Play, Grain,
    /// Rec, Auto, the Pads tab): [`SamplePage::wheel`] does it there, one
    /// undo step a turn. The Wave, Warp and Chop pages' wheel zoomed, so
    /// their rows step here.
    pub fn step_row(&mut self, ui: &mut Ui, delta: i32) -> Option<UiEvent> {
        let name = match self.page() {
            PAGE | WARP_PAGE | CHOP_PAGE => self.list()?,
            _ => return None,
        };
        let index = ui.get::<SettingList>(name)?.selected;
        Some(UiEvent {
            widget: name.to_string(),
            kind: EventKind::Step { index, delta },
        })
    }

    /// The picked row is a button (the Pad list's Effects): the wheel's
    /// push presses it, as a tap would, instead of opening it for the wheel.
    pub fn button_row(&self, ui: &Ui) -> Option<UiEvent> {
        let name = self.list()?;
        let l = ui.get::<SettingList>(name)?;
        l.buttons.contains(&l.selected).then(|| UiEvent {
            widget: name.to_string(),
            kind: EventKind::Tab(l.selected),
        })
    }

    /// The wheel's push opened (true) or closed the picked row.
    pub fn set_editing(&mut self, ui: &mut Ui, on: bool) {
        self.editing = on;
        self.show_editing(ui);
    }

    /// The page's list shows whether the wheel changes its picked row.
    pub fn show_editing(&self, ui: &mut Ui) {
        let Some(name) = self.list() else {
            return;
        };
        // Only a change is redrawn (get_mut marks the list for it).
        if ui
            .get::<SettingList>(name)
            .is_some_and(|l| l.editing != self.editing)
        {
            if let Some(l) = ui.get_mut::<SettingList>(name) {
                l.editing = self.editing;
            }
        }
    }

    fn refresh(&mut self, ui: &mut Ui) {
        self.stale = false;
        self.state = self.build_state();
        // Pads are named by their own bank, A1 to H16 (PAD BANK pages a
        // kit's 128 pads), not the kit's.
        let bank: Option<char> = None;
        match (self.state.as_ref(), self.tab.as_mut()) {
            (Some(st), Some(tab)) => {
                tab.bank = bank;
                tab.update(ui, st);
            }
            (Some(st), None) => match WaveTab::add(ui, PAGE, st) {
                Ok(mut tab) => {
                    tab.bank = bank;
                    tab.update(ui, st);
                    self.tab = Some(tab);
                }
                Err(e) => {
                    self.lines.push(format!("sample: no Wave page ({e})"));
                    self.state = None;
                }
            },
            (None, _) => {}
        }
        self.chop_state = None;
        if self.editor == Editor::Chop {
            self.refresh_chop(ui, bank);
        }
        // Another tab, pad or region: live chop stops.
        let stopped = match (self.chop.as_mut(), self.chop_state.as_ref()) {
            (Some(tab), Some(st)) if self.editor == Editor::Chop => {
                tab.take_stop(st).is_some() || !tab.live()
            }
            _ => true,
        };
        if stopped && self.live.is_some() {
            self.stop_live("stopped");
        }
        self.play_state = None;
        if self.editor == Editor::Play {
            self.refresh_play(ui, bank);
        }
        self.grain_state = None;
        if self.editor == Editor::Grain {
            self.refresh_grain(ui, bank);
        }
        if self.editor == Editor::Warp {
            self.refresh_warp(ui, bank);
        }
        self.rec_state = None;
        if self.editor == Editor::Rec {
            self.refresh_rec(ui, bank);
        }
        self.fx_state = None;
        if self.editor == Editor::Fx {
            self.refresh_fx(ui, bank);
        }
        self.auto_state = None;
        if self.editor == Editor::Auto {
            self.refresh_auto(ui, bank);
        }
        self.pads_state = None;
        if self.editor == Editor::Pads {
            self.refresh_pads(ui, bank);
        }
        if self.page() == EMPTY_PAGE {
            self.show_empty(ui);
        }
        let has_sound = self
            .projects
            .get(self.kit)
            .and_then(|p| p.pad(self.pad))
            .is_some_and(|pd| !pd.layers.is_empty());
        send::show(ui, has_sound, self.sending.is_some());
        // The pad bar's title names the pad (and its sound) on the editors
        // of a pad with a sound.
        for bar in NAMED_BARS {
            if let Some(b) = ui.get_mut::<mpfree_sampler_ui::widgets::PadBar>(bar) {
                if b.nameable != has_sound {
                    b.nameable = has_sound;
                }
            }
        }
        self.shown_heads.clear();
        if let Some(tab) = self.tab.as_ref() {
            tab.set_playheads(ui, &[]);
        }
        if let Some(tab) = self.chop.as_ref() {
            tab.set_playheads(ui, &[]);
        }
        if let Some(tab) = self.warp.as_ref() {
            tab.set_playheads(ui, &[]);
        }
    }

    /// Shows the Warp tab (the Wave tab's state), adding it the first time.
    fn refresh_warp(&mut self, ui: &mut Ui, bank: Option<char>) {
        let Some(st) = self.state.as_ref() else {
            return;
        };
        match self.warp.as_mut() {
            Some(tab) => {
                tab.bank = bank;
                tab.update(ui, st);
            }
            None => match WarpTab::add(ui, WARP_PAGE, st) {
                Ok(mut tab) => {
                    tab.bank = bank;
                    tab.update(ui, st);
                    self.warp = Some(tab);
                }
                Err(e) => self.lines.push(format!("sample: no Warp page ({e})")),
            },
        }
    }

    fn refresh_chop(&mut self, ui: &mut Ui, bank: Option<char>) {
        let Some(st) = self.state.as_ref() else {
            return;
        };
        let Some(project) = self.projects.get(self.kit) else {
            return;
        };
        self.chop_state = ChopState::from_project(project, st.pad, st.layer, st.source.clone());
        let new_kit = self
            .new_kit
            .as_ref()
            .map(|r| r.as_ref().map(|_| ()).map_err(Clone::clone));
        if let Some(cs) = self.chop_state.as_mut() {
            cs.files = self.files;
            cs.new_kit = new_kit;
        }
        let Some(cs) = self.chop_state.as_ref() else {
            return;
        };
        match self.chop.as_mut() {
            Some(tab) => {
                tab.bank = bank;
                tab.update(ui, cs);
            }
            None => match ChopTab::add(ui, CHOP_PAGE, cs) {
                Ok(mut tab) => {
                    tab.bank = bank;
                    tab.update(ui, cs);
                    self.chop = Some(tab);
                }
                Err(e) => {
                    self.lines.push(format!("sample: no Chop page ({e})"));
                    self.chop_state = None;
                }
            },
        }
        // Pitch waiting for the sound's pitch: the worker follows it. Once
        // it has, the tab's chop is new, and kept.
        self.follow_chop_pitch();
        self.keep_chop();
    }

    /// The Chop tab waits for its sound's pitch (Pitch picked,
    /// `ChopTab::wants_pitch`): the analysis worker follows it, and the
    /// page shows the view with it when it lands (`Views::take_done`).
    fn follow_chop_pitch(&mut self) {
        let (Some(tab), Some(st)) = (self.chop.as_ref(), self.chop_state.as_ref()) else {
            return;
        };
        if !tab.wants_pitch(st) {
            return;
        }
        let id = st.source.id;
        if let Some(s) = self.projects.get(self.kit).and_then(|p| p.source(id)) {
            self.sources.follow_pitch(s);
        }
    }

    fn refresh_pads(&mut self, ui: &mut Ui, bank: Option<char>) {
        let Some(project) = self.projects.get(self.kit) else {
            return;
        };
        let mut st = PadsState::from_project(project, self.pad);
        st.effects = self.pad_effects(self.kit, self.pad);
        let kit = match self.names.get(self.kit) {
            Some(n) => format!("Kit {} · {n}", self.kit + 1),
            None => format!("Kit {}", self.kit + 1),
        };
        // The bar's Kit button: the kit's name, as the list of kits has it.
        let name = match self.names.get(self.kit) {
            Some(n) => crate::gui::chip_kit_name(n, self.kit),
            None => format!("Kit {}", self.kit + 1),
        };
        match self.pads.as_mut() {
            Some(tab) => {
                (tab.bank, tab.kit, tab.kit_name) = (bank, kit, name);
                tab.update(ui, &st);
            }
            None => match PadsTab::add(ui, PADS_PAGE, &st) {
                Ok(mut tab) => {
                    (tab.bank, tab.kit, tab.kit_name) = (bank, kit, name);
                    tab.update(ui, &st);
                    self.pads = Some(tab);
                }
                Err(e) => {
                    self.lines.push(format!("sample: no Pads page ({e})"));
                    return;
                }
            },
        }
        self.pads_state = Some(st);
    }

    fn refresh_play(&mut self, ui: &mut Ui, bank: Option<char>) {
        if self.state.is_none() {
            return;
        }
        let Some(project) = self.projects.get(self.kit) else {
            return;
        };
        let sources = &mut self.sources;
        self.play_state = PlayState::from_project(project, self.pad, |s| sources.get(s));
        let Some(ps) = self.play_state.as_ref() else {
            return;
        };
        match self.play.as_mut() {
            Some(tab) => {
                tab.bank = bank;
                tab.update(ui, ps);
                tab.set_velocity(ui, self.velocity);
            }
            None => match PlayTab::add(ui, PLAY_PAGE, ps) {
                Ok(mut tab) => {
                    tab.bank = bank;
                    tab.update(ui, ps);
                    tab.set_velocity(ui, self.velocity);
                    self.play = Some(tab);
                }
                Err(e) => {
                    self.lines.push(format!("sample: no Play page ({e})"));
                    self.play_state = None;
                }
            },
        }
    }

    /// The FX tab: the layer's sound, and the effects on the pad's channel
    /// as the mixer last said. Kit 1's pads A1 to A16 have a channel of
    /// their own; any other pad is lent one when it is given an effect
    /// (R3 part 4a), and plays through its track's until then.
    fn refresh_fx(&mut self, ui: &mut Ui, bank: Option<char>) {
        let Some(ws) = self.state.as_ref() else {
            return;
        };
        let mut st = FxState {
            pad: self.pad,
            name: ws.pad_name.clone(),
            sound: mpfree_sampler_ui::play_tab::LayerSound {
                name: ws.source_name.clone(),
                rate: ws.rate,
                view: ws.source.clone(),
            },
            start: ws.region.start,
            end: ws.region.end,
            channel: Ok(()),
            pedals: None,
            room: 0,
            into: String::new(),
        };
        self.fill_pedals(&mut st);
        match self.fx.as_mut() {
            Some(tab) => {
                tab.bank = bank;
                tab.update(ui, &st);
            }
            None => match FxTab::add(ui, FX_PAGE, &st) {
                Ok(mut tab) => {
                    tab.bank = bank;
                    tab.update(ui, &st);
                    self.fx = Some(tab);
                }
                Err(e) => {
                    self.lines.push(format!("sample: no FX page ({e})"));
                    return;
                }
            },
        }
        self.fx_state = Some(st);
    }

    /// The pedals the mixer said for the FX tab's pad, if they are its
    /// (the kit's showing). A pad with no channel of its own, and none
    /// free to lend it, can't take an effect: the tab says why.
    fn fill_pedals(&self, st: &mut FxState) {
        let ours = self.pedals_of(self.kit, st.pad);
        st.pedals = ours.map(|p| p.list.iter().map(fx_pedal).collect());
        st.into = ours.map_or_else(String::new, |p| p.into.clone());
        st.room = ours.map_or(0, |p| p.room);
        st.channel = match ours {
            Some(p) if !p.own && !p.spare => Err(format!(
                "No channel free: all {} pad channels past kit 1's A1 to A{} are lent. Take \
                 another pad's effects off on Whole chain first",
                crate::mixing::LENT,
                crate::mixing::PADS
            )),
            _ => Ok(()),
        };
    }

    /// The pedals the mixer last said, if they are kit `kit`'s pad `pad`'s.
    fn pedals_of(&self, kit: usize, pad: usize) -> Option<&PadPedals> {
        self.pedals
            .as_ref()
            .filter(|p| (p.kit, p.pad) == (kit, pad))
    }

    /// The FX tab shows the pedals as they are now.
    fn show_pedals(&mut self, ui: &mut Ui) {
        let Some(mut st) = self.fx_state.take() else {
            return;
        };
        self.fill_pedals(&mut st);
        if let Some(tab) = self.fx.as_mut() {
            tab.update(ui, &st);
        }
        self.fx_state = Some(st);
    }

    /// The effects on a kit's pad's channel, as the mixer has them now.
    /// Sent while the FX tab shows, after [`FromUi::SamplePedals`].
    pub fn set_pedals(&mut self, ui: &mut Ui, pedals: PadPedals) {
        if self.pedals.as_ref() == Some(&pedals) {
            return;
        }
        self.pedals = Some(pedals);
        self.show_pedals(ui);
    }

    /// What the FX tab asks of the mixer: first, while the tab shows on
    /// the screen (`shows`), which pad's effects to send (or none, once it
    /// doesn't); then its Q-Links', wheel's and lifts' changes.
    pub fn take_fx(&mut self, shows: bool) -> Vec<FromUi> {
        let mut out = Vec::new();
        let want = self
            .fx_state
            .as_ref()
            .filter(|_| shows && self.page() == FX_PAGE)
            .map(|st| (self.kit, st.pad));
        self.ask_pedals(want, &mut out);
        out.append(&mut self.fx_out);
        out
    }

    /// Asks the mixer for kit `want.0`'s pad `want.1`'s effects (None: no
    /// more), if it wasn't asked already.
    fn ask_pedals(&mut self, want: Option<(usize, usize)>, out: &mut Vec<FromUi>) {
        if want != self.pedals_asked {
            self.pedals_asked = want;
            out.push(FromUi::SamplePedals(want));
        }
    }

    /// Applies what the FX tab asked for: the mixer hears it (as the chain
    /// editor's own touches), and the tab shows it at once.
    fn act_fx(&mut self, ui: &mut Ui, a: FxAction, out: &mut Vec<FromUi>) {
        let (kit, pad) = (self.kit, self.pad);
        let name = |p: &PadPedals, slot: usize| p.list.get(slot).map(|e| e.name.clone());
        match a {
            FxAction::Tab(i) => self.open(ui, i),
            FxAction::On { slot, on } => {
                let Some(p) = self
                    .pedals
                    .as_mut()
                    .filter(|p| (p.kit, p.pad) == (kit, pad))
                else {
                    return;
                };
                let Some(e) = p.list.get_mut(slot) else {
                    return;
                };
                e.on = on;
                let what = name(p, slot).unwrap_or_default();
                self.ask_pedals(Some((kit, pad)), out);
                out.push(FromUi::Chain(ChainAction::SetOn { slot, on }));
                self.lines.push(format!(
                    "sample: pad {}'s {what} {}",
                    self.pad_text(self.kit, pad),
                    if on { "on" } else { "off" }
                ));
                self.show_pedals(ui);
            }
            FxAction::Turn {
                slot,
                knob,
                detents,
                fine,
            } => {
                let Some(k) = self
                    .pedals
                    .as_mut()
                    .filter(|p| (p.kit, p.pad) == (kit, pad))
                    .and_then(|p| p.list.get_mut(slot))
                    .and_then(|e| e.knobs.get_mut(knob))
                else {
                    return;
                };
                let v = k.nudged(detents, fine);
                if v == k.value {
                    let way = if detents < 0 { "lower" } else { "higher" };
                    self.told
                        .push(format!("{} stays {}: it goes no {way}", k.label, k.text()));
                    return;
                }
                k.value = v;
                k.pos = k.info.to_unit(v);
                let addr = k.addr;
                self.ask_pedals(Some((kit, pad)), out);
                out.push(FromUi::Chain(ChainAction::Set { addr, value: v }));
                if !self.fx_touched.contains(&addr) {
                    self.fx_touched.push(addr);
                }
                self.show_pedals(ui);
            }
            FxAction::Add | FxAction::Whole => {
                let add = a == FxAction::Add;
                let into = self
                    .pedals_of(kit, pad)
                    .map_or_else(|| "its track".to_string(), |p| p.into.clone());
                self.lines.push(format!(
                    "sample: pad {}'s effects: {}",
                    self.pad_text(kit, pad),
                    if add {
                        "the chain editor's picker".to_string()
                    } else {
                        format!("the chain editor, then {into}")
                    }
                ));
                out.push(FromUi::SampleFx { kit, pad, add });
            }
        }
    }

    fn refresh_grain(&mut self, ui: &mut Ui, bank: Option<char>) {
        if self.state.is_none() {
            return;
        }
        let layer = self.layer();
        let bpm = self.tempo();
        let Some(project) = self.projects.get(self.kit) else {
            return;
        };
        let sources = &mut self.sources;
        self.grain_state = GrainState::from_project(project, self.pad, layer, |s| sources.get(s));
        if let Some(gs) = self.grain_state.as_mut() {
            gs.bpm = bpm;
        }
        let Some(gs) = self.grain_state.as_ref() else {
            return;
        };
        match self.grain.as_mut() {
            Some(tab) => {
                tab.bank = bank;
                tab.update(ui, gs);
            }
            None => match GrainTab::add(ui, GRAIN_PAGE, gs) {
                Ok(mut tab) => {
                    tab.bank = bank;
                    tab.update(ui, gs);
                    self.grain = Some(tab);
                }
                Err(e) => {
                    self.lines.push(format!("sample: no Grain page ({e})"));
                    self.grain_state = None;
                }
            },
        }
    }

    fn build_state(&mut self) -> Option<WaveState> {
        let layer = self.layer();
        let p = self.projects.get(self.kit)?;
        let region = &p.pad(self.pad)?.layers.get(layer)?.region;
        let source = p.source(region.source)?;
        let view = self.sources.get(source);
        WaveState::from_project(p, self.pad, layer, view)
    }

    /// The layer the Wave, Warp, Chop and Grain tabs edit: the one picked
    /// on the Play tab's layer map for the pad shown, else the first.
    fn layer(&self) -> usize {
        let n = self
            .projects
            .get(self.kit)
            .and_then(|p| p.pad(self.pad))
            .map_or(1, |p| p.layers.len().max(1));
        self.play
            .as_ref()
            .and_then(|t| t.layer_of(self.pad))
            .unwrap_or(0)
            .min(n - 1)
    }

    /// Map to keys: `pads` of the kit become one keygroup instrument, each
    /// at its sound's note (its keygroup's root, a note in its sound's or
    /// its own name, or the pitch heard), as one undo step; pads with no
    /// note found stay as they are. The log says where each note came from.
    fn map_to_keys(&mut self, pads: &[usize]) {
        self.lift();
        let kit = self.kit;
        let Some(project) = self.projects.get(kit) else {
            return;
        };
        let mut roots = Vec::new();
        let mut from: Vec<(RootFrom, usize)> = Vec::new();
        let mut left = Vec::new();
        for &pad in pads {
            let Some(p) = project.pad(pad).filter(|p| !p.layers.is_empty()) else {
                continue;
            };
            let source = project.source(p.layers[0].region.source);
            let heard = source.and_then(|s| {
                // The note heard is needed now: worked out here if the
                // worker hasn't yet.
                self.sources.full(s).root_note
            });
            let name = match source {
                Some(s) if keymap::note_in_name(&s.name).is_some() => s.name.as_str(),
                _ => p.name.as_str(),
            };
            match keymap::sound_root(p.keys.map(|k| k.root), name, heard) {
                Some((root, by)) => {
                    roots.push((pad, root));
                    match from.iter_mut().find(|(b, _)| *b == by) {
                        Some((_, n)) => *n += 1,
                        None => from.push((by, 1)),
                    }
                }
                None => left.push(self.pad_text(kit, pad)),
            }
        }
        let mut mapped = !roots.is_empty();
        let what = if roots.is_empty() {
            "no pad has a note to map to keys (none in its name, no clear pitch)".to_string()
        } else {
            let Some(project) = self.projects.get_mut(kit) else {
                return;
            };
            match project.map_to_keys(&roots) {
                Ok(_) => {
                    self.edits += 1;
                    if let Some(d) = self.dirty.get_mut(kit) {
                        *d = true;
                    }
                    let lo = roots.iter().map(|r| r.1).min().unwrap_or(0);
                    let hi = roots.iter().map(|r| r.1).max().unwrap_or(0);
                    let by = from
                        .iter()
                        .map(|(b, n)| format!("{n} by {}", b.label()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let mut what = format!(
                        "mapped to keys: {} pad{} from {} to {} ({by}), the pads play notes",
                        roots.len(),
                        if roots.len() == 1 { "" } else { "s" },
                        mpfree_sampler_ui::note_name(lo),
                        mpfree_sampler_ui::note_name(hi)
                    );
                    if !left.is_empty() {
                        what.push_str(&format!(
                            "; left as they were (no note found): {}",
                            left.join(", ")
                        ));
                    }
                    what
                }
                Err(e) => {
                    mapped = false;
                    format!("not mapped to keys ({e:?})")
                }
            }
        };
        // On the hint line too when nothing was mapped: nothing else on the
        // screen changes then. A mapping shows itself on the keys.
        if !mapped {
            let mut told = what.clone();
            if let Some(c) = told.get_mut(0..1) {
                c.make_ascii_uppercase();
            }
            self.told.push(told);
        }
        let kit = self.kit_text_of(kit);
        self.lines.push(format!("sample: {kit}: {what}"));
    }

    fn kit_text(&self) -> String {
        self.kit_text_of(self.kit)
    }

    fn kit_text_of(&self, kit: usize) -> String {
        match self.names.get(kit) {
            Some(n) => format!("kit {} ({n})", kit + 1),
            None => format!("kit {}", kit + 1),
        }
    }

    fn pad_text(&self, _kit: usize, pad: usize) -> String {
        crate::app::kit_pad_name(pad)
    }

    fn show_empty(&mut self, ui: &mut Ui) {
        let (heading, lines): (String, Vec<&str>) = if self.projects.is_empty() {
            (
                "The kits are still loading".into(),
                vec!["Their sounds show here once they are in."],
            )
        } else if self.kit >= self.projects.len() {
            (
                "The pads play the synth".into(),
                vec!["Pick a drum kit on Play (the wheel or a tap) to edit its sounds here."],
            )
        } else {
            (
                format!(
                    "Pad {} has no sound in {}",
                    self.pad_text(self.kit, self.pad),
                    self.kit_text()
                ),
                vec![
                    "Hit a pad with a sound to edit it,",
                    "or turn SHIFT + wheel or press - / + for the next one.",
                    "",
                    "To give this pad a sound: the Pads tab, Browse, then Load.",
                ],
            )
        };
        if let Some(c) = ui.get_mut::<Card>(EMPTY) {
            c.heading = heading;
            c.lines = lines
                .into_iter()
                .map(|l| (l.to_string(), Tone::Dim))
                .collect();
        }
    }

    // ------------------------------------------------------------ edits

    /// A touch on one of the editor's widgets. Taps that play the pad go
    /// in `out`, for the audio thread.
    pub fn event(&mut self, ui: &mut Ui, ev: &UiEvent, out: &mut Vec<FromUi>) {
        // A tap on the pad bar's title: type the pad's name, or its sound's.
        if ev.kind == EventKind::Type(0)
            && ui
                .get::<mpfree_sampler_ui::widgets::PadBar>(&ev.widget)
                .is_some()
        {
            self.ask_names();
            return;
        }
        // A finger dragged on a Q-Link card turns that knob, as the knob
        // would (the drag is one undo step: the finger's lift ends it).
        if let EventKind::Step { index, delta } = ev.kind {
            if ui
                .get::<mpfree_sampler_ui::widgets::KnobCard>(&ev.widget)
                .is_some()
            {
                self.qlink(ui, index, delta, false);
                return;
            }
        }
        let before = touched_row(ui, ev);
        let card = touched_card(ui, ev);
        // A faint button, panel control or Q-Link card tapped: why it is
        // faint, and nothing else.
        if let EventKind::Select(i) = ev.kind {
            let card = ui
                .get::<mpfree_sampler_ui::widgets::KnobCard>(&ev.widget)
                .filter(|c| i == 1 && !c.enabled)
                .map(|c| c.why.clone());
            let why = ui
                .get::<mpfree_sampler_ui::widgets::ButtonRow>(&ev.widget)
                .and_then(|b| b.why_faint(i))
                .or_else(|| {
                    ui.get::<mpfree_sampler_ui::widgets::SettingList>(&ev.widget)
                        .and_then(|l| l.why_faint(i))
                })
                .map(str::to_string)
                .or(card);
            if let Some(why) = why {
                self.told.push(why);
                return;
            }
        }
        self.answered = false;
        self.event_on(ui, ev, out);
        // A tap on a Q-Link card the wheel can't move to (the Chop tab's):
        // what the card is for.
        if let Some((label, focused)) = card {
            if !self.answered
                && self.told.is_empty()
                && touched_card(ui, ev).is_some_and(|(_, now)| now == focused)
            {
                self.told
                    .push(format!("{label}: turn the knob on the MPC to change it"));
            }
        }
        // A step on a settings row that nothing answered and that left the
        // row as it was says so in place (Jay, 13 Nov).
        if let Some((row, delta)) = before {
            if !self.answered
                && self.told.is_empty()
                && touched_row(ui, ev).is_some_and(|(now, _)| now == row)
            {
                let way = if delta < 0 { "lower" } else { "higher" };
                self.told.push(format!(
                    "{} stays {}: it goes no {way} here",
                    row.name, row.value
                ));
            }
        }
    }

    fn event_on(&mut self, ui: &mut Ui, ev: &UiEvent, out: &mut Vec<FromUi>) {
        if ev.widget == send::SEND {
            if let EventKind::Tab(i) = ev.kind {
                self.press_send(ui, i, out);
            }
            return;
        }
        if ev.widget.starts_with(CHOP_PREFIX) {
            let (Some(tab), Some(st)) = (self.chop.as_mut(), self.chop_state.as_ref()) else {
                return;
            };
            let a = tab.action(ui, ev, st);
            if let Some(note) = tab.take_note() {
                self.told.push(note);
            }
            self.keep_chop();
            if let Some(a) = self.took(a) {
                self.act_chop(ui, a, out);
            }
            return;
        }
        if ev.widget.starts_with(PLAY_PREFIX) {
            let (Some(tab), Some(st)) = (self.play.as_mut(), self.play_state.as_ref()) else {
                return;
            };
            let a = tab.action(ui, ev, st);
            if let Some(a) = self.took(a) {
                self.act_play(ui, a, out);
            }
            return;
        }
        if ev.widget.starts_with(GRAIN_PREFIX) {
            let (Some(tab), Some(st)) = (self.grain.as_mut(), self.grain_state.as_ref()) else {
                return;
            };
            let a = tab.action(ui, ev, st);
            if let Some(a) = self.took(a) {
                self.act_grain(ui, a);
            }
            return;
        }
        if ev.widget.starts_with(FX_PREFIX) {
            let (Some(tab), Some(st)) = (self.fx.as_mut(), self.fx_state.as_ref()) else {
                return;
            };
            let a = tab.action(ui, ev, st);
            if let Some(a) = self.took(a) {
                self.act_fx(ui, a, out);
            }
            return;
        }
        if ev.widget.starts_with(REC_PREFIX) {
            let (Some(tab), Some(st)) = (self.rec.as_mut(), self.rec_state.as_ref()) else {
                return;
            };
            let a = tab.action(ui, ev, st);
            if let Some(note) = tab.take_note() {
                self.told.push(note);
            }
            if let Some(a) = self.took(a) {
                self.act_rec(ui, a);
            }
            return;
        }
        if ev.widget.starts_with(AUTO_PREFIX) {
            let (Some(tab), Some(st)) = (self.auto.as_mut(), self.auto_state.as_ref()) else {
                return;
            };
            let a = tab.action(ui, ev, st);
            match self.took(a) {
                Some(a) => self.act_auto(ui, a),
                // A row stepped: the pads it would go onto follow.
                None => self.refresh_auto_state(ui),
            }
            return;
        }
        if ev.widget.starts_with(WARP_PREFIX) {
            let (Some(tab), Some(st)) = (self.warp.as_mut(), self.state.as_ref()) else {
                return;
            };
            let a = tab.action(ui, ev, st);
            if let Some(a) = self.took(a) {
                self.act(ui, a, out);
            }
            return;
        }
        if ev.widget.starts_with(PADS_PREFIX) {
            let (Some(tab), Some(st)) = (self.pads.as_mut(), self.pads_state.as_ref()) else {
                return;
            };
            let a = tab.action(ui, ev, st);
            if let Some(note) = tab.take_note() {
                self.told.push(note);
            }
            if let Some(a) = self.took(a) {
                self.act_pads(ui, a, out);
            }
            return;
        }
        let (Some(tab), Some(st)) = (self.tab.as_mut(), self.state.as_ref()) else {
            return;
        };
        let a = tab.action(ui, ev, st);
        if let Some(a) = self.took(a) {
            self.act(ui, a, out);
        }
    }

    /// Notes whether a tab answered the touch with an action.
    fn took<T>(&mut self, a: Option<T>) -> Option<T> {
        self.answered = a.is_some();
        a
    }

    /// A press on the buttons beside the bar ([`send::BUTTONS`]).
    fn press_send(&mut self, ui: &mut Ui, i: usize, out: &mut Vec<FromUi>) {
        match i {
            send::SEND_TO_PAD => {
                let what = format!("pad {}", self.pad_text(self.kit, self.pad));
                if self.sending.take().is_some() {
                    self.lines.push("sample: Send to pad called off".into());
                } else if !self
                    .projects
                    .get(self.kit)
                    .and_then(|p| p.pad(self.pad))
                    .is_some_and(|pd| !pd.layers.is_empty())
                {
                    self.lines
                        .push(format!("sample: {what} has nothing to send"));
                } else {
                    self.lines
                        .push(format!("sample: hit a pad to send {what} to it"));
                    self.sending = Some((self.kit, self.pad));
                }
                send::show(ui, true, self.sending.is_some());
            }
            send::CHOP_TO_PADS if self.editor == Editor::Chop => {
                // The Chop tab's own Chop to pads.
                let (Some(tab), Some(st)) = (self.chop.as_mut(), self.chop_state.as_ref()) else {
                    return;
                };
                if let Some(a) = tab.press(ui, st, chop_tab::Button::ToPads) {
                    self.act_chop(ui, a, out);
                }
            }
            send::CHOP_TO_PADS => self.open_editor(ui, Editor::Chop),
            _ => {}
        }
    }

    /// Send to pad: a copy of pad `from` of kit `kit` (its sound and every
    /// edit) onto pad `to`, one undo step.
    fn send(&mut self, ui: &mut Ui, kit: usize, from: usize, to: usize) {
        send::show(ui, true, false);
        if kit != self.kit || to == from || to >= PADS {
            self.lines.push("sample: Send to pad called off".into());
            return;
        }
        let Some(copy) = self.projects.get(kit).and_then(|p| p.pad(from)).cloned() else {
            return;
        };
        let mut none = Vec::new();
        self.act_pads(
            ui,
            PadsAction::Set {
                pad: to,
                after: Some(Box::new(copy)),
                gesture: None,
            },
            &mut none,
        );
        self.lines.push(format!(
            "sample: pad {} sent to pad {}",
            self.pad_text(kit, from),
            self.pad_text(kit, to)
        ));
    }

    /// Opens the editor's tab `i` (Wave, Warp, Chop, Play, Grain, FX or Rec).
    fn open(&mut self, ui: &mut Ui, i: usize) {
        let editor = match i {
            wave_tab::TAB => Editor::Wave,
            warp_tab::TAB => Editor::Warp,
            chop_tab::TAB => Editor::Chop,
            play_tab::TAB => Editor::Play,
            grain_tab::TAB => Editor::Grain,
            rec_tab::TAB => Editor::Rec,
            fx_tab::TAB => Editor::Fx,
            _ => return,
        };
        self.open_editor(ui, editor);
    }

    /// The Pads tab shows: the pad picked's bank on its view (the Sample
    /// tab's editor is kept for when it shows again).
    pub fn show_pads(&mut self, ui: &mut Ui) {
        if self.editor != Editor::Pads {
            self.lift();
            self.sample_editor = self.editor;
            self.editor = Editor::Pads;
            self.stale = true;
        }
        self.refresh_if_stale(ui);
    }

    /// The Sample tab shows: its editor again, if the Pads tab had the
    /// page.
    pub fn show_sample(&mut self, ui: &mut Ui) {
        if self.editor == Editor::Pads {
            self.lift();
            self.editor = self.sample_editor;
            self.stale = true;
        }
        self.refresh_if_stale(ui);
    }

    /// The effects on the mixer's pad channels changed (each pad with a
    /// channel of its own, by kit and pad): the Pad list's Effects row
    /// names them.
    pub fn set_pad_fx(&mut self, ui: &mut Ui, fx: Vec<PadFx>) {
        if self.pad_fx.as_ref() == Some(&fx) {
            return;
        }
        self.pad_fx = Some(fx);
        let effects = self.pad_effects(self.kit, self.pads_state.as_ref().map_or(0, |st| st.pad));
        if let (Some(tab), Some(st)) = (self.pads.as_mut(), self.pads_state.as_mut()) {
            st.effects = effects;
            tab.update(ui, st);
        }
    }

    /// The effects on kit `kit`'s pad `pad`'s channel, for the Pad list's
    /// Effects row: none for a pad with no channel of its own (its + FX
    /// lends it one); None until the mixer has said.
    fn pad_effects(&self, kit: usize, pad: usize) -> Option<Vec<String>> {
        let fx = self.pad_fx.as_ref()?;
        Some(
            fx.iter()
                .find(|(k, p, _)| (*k, *p) == (kit, pad))
                .map_or_else(Vec::new, |f| f.2.clone()),
        )
    }

    /// Opens `editor` (SAMPLE EDIT: Wave).
    pub fn open_editor(&mut self, ui: &mut Ui, editor: Editor) {
        if editor != self.editor {
            self.lift();
            self.editor = editor;
            self.refresh(ui);
        }
    }

    /// Keeps the Chop tab's slices for its sound once they change
    /// (`ChopTab::take_kept`): not an undo step, but saved with the
    /// project, so they are there when the tab comes back to the sound.
    fn keep_chop(&mut self) {
        self.follow_chop_pitch();
        let (Some(tab), Some(st)) = (self.chop.as_mut(), self.chop_state.as_mut()) else {
            return;
        };
        let Some((mode, cuts)) = tab.take_kept(st) else {
            return;
        };
        let id = st.source.id;
        let Some(project) = self.projects.get_mut(self.kit) else {
            return;
        };
        let Some(source) = project.source(id) else {
            return;
        };
        let kept =
            ChopSet::from_markers(KEPT_CHOP, source, mode, cuts).and_then(|c| project.keep_chop(c));
        match kept {
            Ok(()) => {
                st.kept = project.kept_chop(id).cloned();
                // The work is unsaved; the audio thread's view is the same.
                self.edits += 1;
                self.unsaved = true;
            }
            Err(e) => self
                .lines
                .push(format!("sample: the chop wasn't kept ({e})")),
        }
    }

    fn act_chop(&mut self, ui: &mut Ui, a: ChopAction, out: &mut Vec<FromUi>) {
        match &a {
            ChopAction::Tab(i) => self.open(ui, *i),
            &ChopAction::Live {
                pad,
                layer,
                start,
                end,
                on: true,
            } => {
                let kit = self.kit;
                self.live = Some(LiveRun {
                    kit,
                    pad,
                    layer,
                    seen: self.marks_seen,
                    heard: false,
                    kept: 0,
                });
                out.push(FromUi::SampleLiveChop {
                    kit,
                    pad,
                    layer,
                    start,
                    end,
                    on: true,
                });
                self.lines.push(format!(
                    "sample: live chop on pad {}: hit pads in time to drop cuts",
                    self.pad_text(kit, pad)
                ));
            }
            ChopAction::Live { on: false, .. } => self.stop_live("stopped"),
            &ChopAction::Audition {
                pad,
                layer,
                start,
                end,
            } => {
                out.push(FromUi::Slice {
                    kit: self.kit,
                    pad,
                    layer,
                    start,
                    end,
                });
            }
            ChopAction::ToPads {
                pad,
                bounds,
                mode,
                keys_from,
                one_pad,
                new_kit,
                ..
            } => {
                let kit = self.kit;
                // Chop to a new kit (R4.6): the empty kit the app offered.
                let to_kit = match (*new_kit, &self.new_kit) {
                    (false, _) => kit,
                    (true, Some(Ok(u))) if *u != kit && *u < self.projects.len() => *u,
                    (true, Some(Err(why))) => {
                        self.lines
                            .push(format!("sample: chop to a new kit not made: {why}"));
                        return;
                    }
                    (true, _) => {
                        self.lines
                            .push("sample: chop to a new kit not made: no empty kit".into());
                        return;
                    }
                };
                let Some(project) = self.projects.get(kit) else {
                    return;
                };
                let name = project.pad(*pad).map_or(String::new(), |p| p.name.clone());
                let new_name = (to_kit != kit).then(|| self.new_kit_name(project, *pad, to_kit));
                // Worked out before the pads change: the chop's source stays.
                let phrase = a.phrase(project);
                let made = if to_kit == kit {
                    a.apply(&mut self.projects[kit])
                } else {
                    let (from, to) = two_mut(&mut self.projects, kit, to_kit);
                    a.apply_to(from, to)
                };
                match made {
                    Ok(pads) => {
                        self.edits += 1;
                        if let Some(d) = self.dirty.get_mut(to_kit) {
                            *d = true;
                        }
                        if let (Some(n), Some(new)) = (self.names.get_mut(to_kit), new_name) {
                            *n = new;
                        }
                        self.mark_renamed(to_kit);
                        let n = bounds.len() - 1;
                        let to = if pads.len() == 1 {
                            self.pad_text(to_kit, pads.start)
                        } else {
                            format!(
                                "{} to {}",
                                self.pad_text(to_kit, pads.start),
                                self.pad_text(to_kit, pads.end - 1)
                            )
                        };
                        let s = |f: u64| format!("{:.3} s", f as f64 / f64::from(self.rate.max(1)));
                        let span = format!("{} to {}", s(bounds[0]), s(bounds[n]));
                        let to = match keys_from {
                            _ if to_kit != kit => {
                                format!("{} pads {to}, for a new track", self.kit_text_of(to_kit))
                            }
                            _ if *one_pad => format!(
                                "pad {to}, sliced: a hit plays slice 1, keys {} to {} a slice each",
                                mpfree_sampler_ui::note_name(SLICE_KEYS_FROM),
                                mpfree_sampler_ui::note_name(
                                    (usize::from(SLICE_KEYS_FROM) + n - 1).min(127) as u8
                                )
                            ),
                            Some(k) => format!(
                                "keys {} to {} (pads {to}; the pads now play notes)",
                                mpfree_sampler_ui::note_name(*k),
                                mpfree_sampler_ui::note_name(k.saturating_add(n as u8 - 1))
                            ),
                            None => format!("pads {to}"),
                        };
                        self.lines.push(format!("sample: {} pad {} {name}: chopped {span} into {n} slice{} ({mode:?}) on \
                                                 {to}", self.kit_text_of(kit), self.pad_text(kit, *pad),
                                                if n == 1 { "" } else { "s" }));
                        let mut for_track = None;
                        match phrase
                            .map(|p| p.map(|p| phrase_clip(&name, &p, pads.start, *one_pad)))
                        {
                            Ok(None) => {}
                            Ok(Some(Ok(clip))) => {
                                self.lines.push(format!(
                                    "sample: phrase {:?}: {} hits over {} beats, for the {} track",
                                    clip.name,
                                    clip.notes.len(),
                                    clip.length / i64::from(PPQ),
                                    if to_kit == kit { "armed" } else { "new" }
                                ));
                                if to_kit == kit {
                                    out.push(FromUi::SamplePhrase(Box::new(clip)));
                                } else {
                                    for_track = Some(Box::new(clip));
                                }
                            }
                            Ok(Some(Err(why))) => {
                                self.lines.push(format!("sample: phrase not made: {why}"))
                            }
                            Err(e) => self.lines.push(format!("sample: phrase not made ({e:?})")),
                        }
                        if to_kit != kit {
                            self.new_kit_pad = Some((to_kit, pads.start));
                            self.new_kit_track = Some(NewKitTrack {
                                kit: to_kit,
                                phrase: for_track,
                            });
                        }
                    }
                    Err(e) => self.lines.push(format!(
                        "sample: chop of pad {} not made ({e:?})",
                        self.pad_text(kit, *pad)
                    )),
                }
                self.refresh(ui);
            }
            ChopAction::Export { pad, layer, cuts } => self.export(*pad, *layer, cuts),
        }
    }

    /// Pad `pad` of the kit shown as files ([`export::pad`]), for the app
    /// to write ([`Self::take_exports`]); the log says what they are.
    fn export(&mut self, pad: usize, layer: usize, cuts: &[u64]) {
        let kit = self.kit;
        let Some(project) = self.projects.get(kit) else {
            return;
        };
        let at = self.pad_text(kit, pad);
        match export::pad(project, pad, layer, cuts) {
            Ok(e) => {
                let name = &e.name;
                let slices = match e.slices {
                    0 => String::new(),
                    n => format!(" and {n} slices ({name} 01.wav to {name} {n:02}.wav)"),
                };
                self.lines.push(format!(
                    "sample: pad {at} exported as {} files: {name}.wav with its settings ({name}.{}){slices}",
                    e.files.len(),
                    export::SIDECAR
                ));
                self.exports.push(e);
            }
            Err(e) => self
                .lines
                .push(format!("sample: pad {at} not exported ({e})")),
        }
    }

    /// Pads exported since the last call, each a folder's worth of files
    /// (named in the files; [`export::Export::name`] names the folder).
    /// The app writes them together, each in a folder of its own
    /// ([`save_export`]), since a sidecar names its sounds by file name.
    pub fn take_exports(&mut self) -> Vec<export::Export> {
        std::mem::take(&mut self.exports)
    }

    /// The app can write files: the Chop tab offers Put on Files. Off until
    /// it says so (the app has a library), so nothing is exported that
    /// would go nowhere.
    pub fn set_files(&mut self, ui: &mut Ui, on: bool) {
        if self.files != on {
            self.files = on;
            self.refresh(ui);
        }
    }

    /// What Put on New kit would do now (sampler R4.6), as the app works
    /// it out: Ok with the empty kit the slices would go on, for a new
    /// track, or Err saying why not (no empty kit left, no room for a
    /// track). The Chop tab offers New kit once the app says.
    pub fn set_new_kit(&mut self, ui: &mut Ui, new_kit: Result<usize, String>) {
        if self.new_kit.as_ref() != Some(&new_kit) {
            self.new_kit = Some(new_kit);
            self.refresh(ui);
        }
    }

    /// A chop to a new kit made since the last call: the app adds a track,
    /// and once it is armed gives it the kit and the phrase.
    pub fn take_new_kit_track(&mut self) -> Option<NewKitTrack> {
        self.new_kit_track.take()
    }

    /// The name kit `u` gets from a chop of `project`'s pad `pad`: "<the
    /// pad's name> chops", or the kit's own name for a pad without one.
    fn new_kit_name(&self, project: &Project, pad: usize, u: usize) -> String {
        match project.pad(pad).map(|p| p.name.trim()) {
            Some(n) if !n.is_empty() => format!("{n} chops"),
            _ => self
                .names
                .get(u)
                .cloned()
                .unwrap_or_else(|| format!("Kit {}", u + 1)),
        }
    }

    fn act_pads(&mut self, ui: &mut Ui, a: PadsAction, out: &mut Vec<FromUi>) {
        match &a {
            // Browse for: the app's browser, whose Load puts a sound on the
            // pad picked.
            PadsAction::Browse => out.push(FromUi::PadsBrowse),
            PadsAction::Pick(pad) => {
                self.pick(*pad);
                self.refresh(ui);
            }
            PadsAction::MapToKeys { pads } => {
                self.map_to_keys(pads);
                self.refresh(ui);
            }
            // The Effects row: the pad's chain in the Mix tab.
            PadsAction::Effects(pad) => out.push(FromUi::PadEffects {
                kit: self.kit,
                pad: *pad,
            }),
            PadsAction::Audition { pad, down } => out.push(self.pad_press(*pad, *down).0),
            // The strip over the pads: the app turns the pads' bank, and
            // the tabs follow it (`set_pad_page`).
            PadsAction::Bank(b) => out.push(FromUi::PadBank(*b)),
            PadsAction::Set { pad, gesture, .. } => {
                let (kit, pad, gesture) = (self.kit, *pad, *gesture);
                let before = self
                    .projects
                    .get(kit)
                    .and_then(|p| p.pad(pad))
                    .cloned()
                    .map(Box::new);
                if self.gesture.as_ref().is_some_and(|g| Some(g.id) != gesture) {
                    self.lift();
                }
                let one_step = gesture.is_none();
                if self.gesture.is_none() {
                    self.gesture = Some(Gesture {
                        id: gesture.unwrap_or(u64::MAX),
                        kit,
                        pad,
                        before: Before::Slot(before),
                    });
                }
                let Some(project) = self.projects.get_mut(kit) else {
                    return;
                };
                match a.apply(project) {
                    Ok(()) => {
                        self.edits += 1;
                        if let Some(d) = self.dirty.get_mut(kit) {
                            *d = true;
                        }
                        self.mark_renamed(kit);
                    }
                    Err(e) => {
                        let line = format!(
                            "sample: edit of pad {} not made ({e:?})",
                            self.pad_text(kit, pad)
                        );
                        self.told.push(line.clone());
                        self.lines.push(line);
                    }
                }
                if one_step {
                    // Copy, paste, clear and put are a step each: log it now.
                    self.lift();
                }
                self.refresh(ui);
            }
        }
    }

    fn act_play(&mut self, ui: &mut Ui, a: PlayAction, out: &mut Vec<FromUi>) {
        match &a {
            PlayAction::Tab(i) => self.open(ui, *i),
            PlayAction::Pick(pad) => {
                self.pick(*pad);
                self.refresh(ui);
            }
            PlayAction::Note { note, down } => out.push(FromUi::SampleKey {
                kit: self.kit,
                note: *note,
                velocity: play_tab::KEY_VELOCITY,
                down: *down,
            }),
            PlayAction::PadsPlay(play) => {
                self.lift();
                let kit = self.kit;
                let Some(project) = self.projects.get_mut(kit) else {
                    return;
                };
                let what = match a.apply(project) {
                    Ok(()) => {
                        self.edits += 1;
                        if let Some(d) = self.dirty.get_mut(kit) {
                            *d = true;
                        }
                        match play {
                            PadsPlay::Pads => "the pads play their own sounds".to_string(),
                            PadsPlay::Keys { from } => format!(
                                "the pads play notes, pad 1 {}",
                                mpfree_sampler_ui::note_name(*from)
                            ),
                            PadsPlay::Levels { pad, by } => format!(
                                "the pads play 16 Levels of pad {} by {}",
                                self.pad_text(kit, usize::from(*pad)),
                                by.label().to_lowercase()
                            ),
                        }
                    }
                    Err(e) => format!("pads play not changed ({e:?})"),
                };
                let kit = self.kit_text_of(kit);
                self.lines.push(format!("sample: {kit}: {what}"));
                self.refresh(ui);
            }
            PlayAction::KeyFade(fade) => {
                self.lift();
                let kit = self.kit;
                let Some(project) = self.projects.get_mut(kit) else {
                    return;
                };
                let what = match a.apply(project) {
                    Ok(()) => {
                        self.edits += 1;
                        if let Some(d) = self.dirty.get_mut(kit) {
                            *d = true;
                        }
                        match fade {
                            0 => "key fade off, the zones edge to edge".to_string(),
                            1 => {
                                "key fade 1 note, the zones that meet crossfade over it".to_string()
                            }
                            n => format!(
                                "key fade {n} notes, the zones that meet crossfade over them"
                            ),
                        }
                    }
                    Err(e) => format!("key fade not changed ({e:?})"),
                };
                let kit = self.kit_text_of(kit);
                self.lines.push(format!("sample: {kit}: {what}"));
                self.refresh(ui);
            }
            PlayAction::Scene(s) => self.scene(ui, *s, out),
            PlayAction::AddLayer { pad, .. }
            | PlayAction::RemoveLayer { pad, .. }
            | PlayAction::LayerSound { pad, .. } => {
                self.lift();
                let (kit, pad) = (self.kit, *pad);
                let Some(project) = self.projects.get_mut(kit) else {
                    return;
                };
                let before = project.pad(pad).cloned();
                let what = match a.apply(project) {
                    Ok(()) => {
                        self.edits += 1;
                        if let Some(d) = self.dirty.get_mut(kit) {
                            *d = true;
                        }
                        layers_text(&a, before.as_ref(), project)
                    }
                    Err(e) => {
                        let line = format!(
                            "sample: layers of pad {} not changed ({e:?})",
                            self.pad_text(kit, pad)
                        );
                        self.told.push(line.clone());
                        self.lines.push(line);
                        self.refresh(ui);
                        return;
                    }
                };
                let kit_text = self.kit_text_of(kit);
                self.lines.push(format!(
                    "sample: {kit_text} pad {}: {what}",
                    self.pad_text(kit, pad)
                ));
                self.refresh(ui);
            }
            PlayAction::Pad { pad, gesture, .. } => {
                let (kit, pad, gesture) = (self.kit, *pad, *gesture);
                if self.gesture.as_ref().is_some_and(|g| g.id != gesture) {
                    self.lift();
                }
                if self.gesture.is_none() {
                    if let Some(st) = self.play_state.as_ref() {
                        self.gesture = Some(Gesture {
                            id: gesture,
                            kit,
                            pad,
                            before: Before::Pad(Box::new(st.settings.clone())),
                        });
                    }
                }
                let Some(project) = self.projects.get_mut(kit) else {
                    return;
                };
                match a.apply(project) {
                    Ok(()) => {
                        self.edits += 1;
                        if let Some(d) = self.dirty.get_mut(kit) {
                            *d = true;
                        }
                    }
                    Err(e) => {
                        let line = format!(
                            "sample: edit of pad {} not made ({e:?})",
                            self.pad_text(kit, pad)
                        );
                        self.told.push(line.clone());
                        self.lines.push(line);
                    }
                }
                self.refresh(ui);
            }
        }
    }

    fn act_grain(&mut self, ui: &mut Ui, a: GrainAction) {
        match &a {
            GrainAction::Tab(i) => self.open(ui, *i),
            GrainAction::Region {
                pad,
                layer,
                gesture,
                ..
            } => {
                let (kit, pad, layer, gesture) = (self.kit, *pad, *layer, *gesture);
                if self.gesture.as_ref().is_some_and(|g| g.id != gesture) {
                    self.lift();
                }
                if self.gesture.is_none() {
                    if let Some(st) = self.grain_state.as_ref() {
                        self.gesture = Some(Gesture {
                            id: gesture,
                            kit,
                            pad,
                            before: Before::Grain {
                                layer,
                                region: Box::new(st.region.clone()),
                            },
                        });
                    }
                }
                let Some(project) = self.projects.get_mut(kit) else {
                    return;
                };
                match a.apply(project) {
                    Ok(()) => {
                        self.edits += 1;
                        if let Some(d) = self.dirty.get_mut(kit) {
                            *d = true;
                        }
                    }
                    Err(e) => {
                        let line = format!(
                            "sample: edit of pad {} not made ({e:?})",
                            self.pad_text(kit, pad)
                        );
                        self.told.push(line.clone());
                        self.lines.push(line);
                    }
                }
                self.refresh(ui);
            }
        }
    }

    /// Kit `kit`'s pad names changed: the Play page is told
    /// ([`SamplePage::take_renamed`]).
    fn mark_renamed(&mut self, kit: usize) {
        if !self.renamed.contains(&kit) {
            self.renamed.push(kit);
        }
    }

    /// A kit whose pads were renamed or replaced since the last call, with
    /// its pads' names, for the Play page.
    pub fn take_renamed(&mut self) -> Option<(usize, Vec<Option<String>>)> {
        if self.renamed.is_empty() {
            return None;
        }
        let kit = self.renamed.remove(0);
        let p = self.projects.get(kit)?;
        Some((
            kit,
            (0..mpfree_sampler::model::PADS)
                .map(|i| {
                    p.pad(i)
                        .filter(|pd| !pd.layers.is_empty())
                        .map(|pd| pd.name.clone())
                })
                .collect(),
        ))
    }

    /// Q-Link `knob` (0-3) turned by `ticks` detents; `fine` (SHIFT held)
    /// turns finer. On Wave they move Start, End, Loop start and Loop end
    /// (one frame a detent with SHIFT); on Chop the slice, its start, the
    /// setting and the first pad; on Play the open section's four rows;
    /// on Grain position, size, density and pitch (with SHIFT
    /// the sprays, and size and density finer).
    pub fn qlink(&mut self, ui: &mut Ui, knob: usize, ticks: i32, fine: bool) {
        self.turned();
        if self.page() == PADS_PAGE {
            let (Some(tab), Some(st)) = (self.pads.as_mut(), self.pads_state.as_ref()) else {
                return;
            };
            if let Some(a) = tab.qlink(ui, st, knob, ticks) {
                self.act_pads(ui, a, &mut Vec::new());
            }
            return;
        }
        if self.editor == Editor::Play {
            let (Some(tab), Some(st)) = (self.play.as_mut(), self.play_state.as_ref()) else {
                return;
            };
            if let Some(a) = tab.qlink(ui, st, knob, ticks, fine) {
                self.act_play(ui, a, &mut Vec::new());
            }
            return;
        }
        if self.editor == Editor::Grain {
            let (Some(tab), Some(st)) = (self.grain.as_mut(), self.grain_state.as_ref()) else {
                return;
            };
            if let Some(a) = tab.qlink(ui, st, knob, ticks, fine) {
                self.act_grain(ui, a);
            }
            return;
        }
        if self.editor == Editor::Fx {
            let a = self
                .fx
                .as_ref()
                .zip(self.fx_state.as_ref())
                .and_then(|(tab, st)| tab.qlink(st, knob, ticks, fine));
            if let Some(a) = a {
                let mut out = Vec::new();
                self.act_fx(ui, a, &mut out);
                self.fx_out.extend(out);
            }
            return;
        }
        if self.editor == Editor::Chop {
            if let (Some(tab), Some(st)) = (self.chop.as_mut(), self.chop_state.as_ref()) {
                tab.qlink(ui, st, knob, ticks, fine);
            }
            self.keep_chop();
            return;
        }
        if self.editor == Editor::Warp {
            let (Some(tab), Some(st)) = (self.warp.as_mut(), self.state.as_ref()) else {
                return;
            };
            if let Some(a) = tab.qlink(ui, st, knob, ticks, fine) {
                self.act(ui, a, &mut Vec::new());
            }
            return;
        }
        let (Some(tab), Some(st)) = (self.tab.as_mut(), self.state.as_ref()) else {
            return;
        };
        if let Some(a) = tab.qlink(ui, st, knob, ticks, fine) {
            self.act(ui, a, &mut Vec::new());
        }
    }

    /// The wheel turned a marker's tab that a finger rests on (Jay, 17 Nov:
    /// hold a control and turn the wheel): on the Wave page it moves as
    /// the marker's Q-Link moves it, a screen pixel a detent snapped the
    /// way it turns, or a frame unsnapped with SHIFT (`fine`). False when
    /// `widget` is not the Wave page's waveform.
    pub fn held_marker(
        &mut self,
        ui: &mut Ui,
        widget: &str,
        index: usize,
        delta: i32,
        fine: bool,
    ) -> bool {
        if widget != wave_tab::WAVE || self.page() != PAGE {
            return false;
        }
        let Some(k) = self
            .state
            .as_ref()
            .and_then(|st| wave_tab::marker_knob(st, index))
        else {
            return false;
        };
        self.qlink(ui, k, delta, fine);
        true
    }

    /// The pad's tuning (semitones) and decay (None: it plays out) in the
    /// playing kit, for the Play page's Q-Links. None when the kit has no
    /// sound on the pad, or the synth has the pads.
    pub fn pad_knobs(&self, pad: usize) -> Option<(f32, Option<f32>)> {
        let p = self.projects.get(self.kit)?.pad(pad)?;
        let decay = (p.amp.sustain < 0.999).then_some(p.amp.decay_ms);
        Some((p.tune_cents as f32 / 100.0, decay))
    }

    /// Pad `pad`'s own tune (cents), decay (ms; the longest for a sound
    /// that plays out), cutoff (Hz; open without a filter) and level (dB)
    /// on the playing kit: what a step without a lock plays, for the
    /// Sound bank of held step pads.
    pub fn pad_sound(&self, pad: usize) -> Option<[f32; 4]> {
        let p = self.projects.get(self.kit)?.pad(pad)?;
        let decay = if p.amp.sustain < 0.999 {
            p.amp.decay_ms
        } else {
            crate::play_page::DECAY_MAX_MS
        };
        let cutoff = if p.filter.kind == mpfree_sampler::dsp::FilterKind::Off {
            crate::play_page::CUTOFF_OPEN_HZ
        } else {
            p.filter.cutoff_hz
        };
        Some([p.tune_cents as f32, decay, cutoff, p.level_db])
    }

    /// A Play page Q-Link on `pad` of the playing kit: its tuning or its
    /// decay, by `d` detents (`fine`: SHIFT held). One undo step a turn,
    /// as on the Play tab; the kit's new view goes to the audio thread.
    pub fn turn_pad(&mut self, pad: usize, which: PadKnob, d: i32, fine: bool) {
        self.turned();
        let kit = self.kit;
        let Some(before) = self.projects.get(kit).and_then(|p| p.pad(pad)).cloned() else {
            return;
        };
        if self
            .play_turn
            .is_some_and(|(p, w, _)| (p, w) != (pad, which))
        {
            self.lift();
        }
        let id = match self.play_turn {
            Some((_, _, id)) => id,
            None => {
                // "PLAY" and the edit count: a new undo step.
                let id = 0x504C_4159_0000_0000 | (self.edits & 0xFFFF_FFFF);
                self.play_turn = Some((pad, which, id));
                self.gesture = Some(Gesture {
                    id,
                    kit,
                    pad,
                    before: Before::Pad(Box::new(before)),
                });
                id
            }
        };
        self.last_turn = Some(Instant::now());
        let edit = |p: &mut Pad| match which {
            PadKnob::Tune => {
                let step = if fine { 10 } else { 100 };
                p.tune_cents = (p.tune_cents + d * step).clamp(-2400, 2400);
            }
            PadKnob::Decay => {
                let max = crate::play_page::DECAY_MAX_MS;
                let open = p.amp.sustain >= 0.999;
                let ms = if open { max } else { p.amp.decay_ms.max(10.0) };
                let per = if fine { 32.0 } else { 8.0 };
                let to = (ms * (d as f32 / per).exp2()).round().clamp(10.0, max);
                if to >= max && d > 0 {
                    p.amp.sustain = 1.0;
                    p.amp.decay_ms = 0.0;
                } else {
                    p.amp.sustain = 0.0;
                    p.amp.decay_ms = to;
                }
            }
        };
        match self.projects[kit].edit_pad(pad, Some(id), edit) {
            Ok(()) => {
                self.edits += 1;
                if let Some(dirty) = self.dirty.get_mut(kit) {
                    *dirty = true;
                }
                self.stale = true;
            }
            Err(e) => self.lines.push(format!(
                "sample: pad {} not changed ({e:?})",
                self.pad_text(kit, pad)
            )),
        }
    }

    /// A Q-Link or the wheel turned: a turn after a rest is a new undo step.
    fn turned(&mut self) {
        let now = Instant::now();
        if self
            .last_turn
            .is_some_and(|t| now.duration_since(t) >= TURN_REST)
        {
            self.lift();
        }
        self.last_turn = Some(now);
    }

    /// The data wheel: zoom round the marker last moved or the slice picked
    /// (Wave, Chop), or change the row picked (Play, Grain).
    pub fn wheel(&mut self, ui: &mut Ui, delta: i32) {
        if self.page() == PADS_PAGE {
            self.turned();
            let (Some(tab), Some(st)) = (self.pads.as_mut(), self.pads_state.as_ref()) else {
                return;
            };
            if let Some(a) = tab.wheel(ui, st, delta) {
                self.act_pads(ui, a, &mut Vec::new());
            }
            return;
        }
        match self.editor {
            Editor::Pads => {}
            Editor::Play => {
                self.turned();
                let (Some(tab), Some(st)) = (self.play.as_mut(), self.play_state.as_ref()) else {
                    return;
                };
                if let Some(a) = tab.wheel(ui, st, delta) {
                    self.act_play(ui, a, &mut Vec::new());
                }
            }
            Editor::Grain => {
                self.turned();
                let (Some(tab), Some(st)) = (self.grain.as_mut(), self.grain_state.as_ref()) else {
                    return;
                };
                if let Some(a) = tab.wheel(ui, st, delta) {
                    self.act_grain(ui, a);
                }
            }
            Editor::Chop => {
                if let (Some(tab), Some(st)) = (self.chop.as_mut(), self.chop_state.as_ref()) {
                    tab.wheel(ui, st, delta);
                }
                self.keep_chop();
            }
            Editor::Wave => {
                if let (Some(tab), Some(st)) = (self.tab.as_mut(), self.state.as_ref()) {
                    tab.wheel(ui, st, delta);
                }
            }
            Editor::Warp => {
                if let (Some(tab), Some(st)) = (self.warp.as_mut(), self.state.as_ref()) {
                    tab.wheel(ui, st, delta);
                }
            }
            Editor::Fx => {
                self.turned();
                let (Some(tab), Some(st)) = (self.fx.as_mut(), self.fx_state.as_ref()) else {
                    return;
                };
                if let Some(a) = tab.wheel(ui, st, delta) {
                    let mut out = Vec::new();
                    self.act_fx(ui, a, &mut out);
                    self.fx_out.extend(out);
                }
            }
            Editor::Rec => {
                let mut source = None;
                if let (Some(tab), Some(st)) = (self.rec.as_mut(), self.rec_state.as_ref()) {
                    source = tab.wheel(ui, st, delta);
                    if let Some(note) = tab.take_note() {
                        self.told.push(note);
                    }
                }
                if let Some(a) = source {
                    self.act_rec(ui, a);
                }
                // The target pad follows the Onto row.
                self.refresh_rec_state(ui);
            }
            Editor::Auto => {
                if let (Some(tab), Some(st)) = (self.auto.as_mut(), self.auto_state.as_ref()) {
                    tab.wheel(ui, st, delta);
                }
                // The pads it would go onto follow the notes and Onto.
                self.refresh_auto_state(ui);
            }
        }
    }

    fn act(&mut self, ui: &mut Ui, a: WaveAction, out: &mut Vec<FromUi>) {
        match a {
            WaveAction::Region { pad, gesture, .. } => {
                let kit = self.kit;
                if self.gesture.as_ref().is_some_and(|g| g.id != gesture) {
                    self.lift();
                }
                if self.gesture.is_none() {
                    if let Some(st) = self.state.as_ref() {
                        self.gesture = Some(Gesture {
                            id: gesture,
                            kit,
                            pad,
                            before: Before::Region {
                                layer: st.layer,
                                region: Box::new(st.region.clone()),
                            },
                        });
                    }
                }
                let Some(project) = self.projects.get_mut(kit) else {
                    return;
                };
                match a.apply(project) {
                    Ok(()) => {
                        self.edits += 1;
                        if let Some(d) = self.dirty.get_mut(kit) {
                            *d = true;
                        }
                    }
                    Err(e) => {
                        let line = format!(
                            "sample: edit of pad {} not made ({e:?})",
                            self.pad_text(kit, pad)
                        );
                        self.told.push(line.clone());
                        self.lines.push(line);
                    }
                }
                self.refresh(ui);
            }
            WaveAction::Audition { pad } => self.audition(pad, out),
            WaveAction::Tab(i) => self.open(ui, i),
            WaveAction::Ask(ask) => self.number_ask = Some(ask),
            WaveAction::Normalize { pad, layer } => self.normalize(ui, pad, layer),
            WaveAction::Process {
                pad,
                layer,
                semitones,
            } => self.process(ui, pad, layer, semitones),
            WaveAction::StopProcess => {
                if let Some(p) = self.processing.as_ref() {
                    p.stop.store(true, Ordering::Relaxed);
                }
            }
        }
    }

    /// Normalize: layer `layer` of pad `pad` gets the gain that brings its
    /// region's loudest frame to 0 dBFS (a tenth of a dB under, rounded
    /// down, and within the Gain row's range), as one undo step. The log
    /// says the peak and the gain; a silent region is left as it is.
    fn normalize(&mut self, ui: &mut Ui, pad: usize, layer: usize) {
        self.lift();
        let kit = self.kit;
        let at = self.pad_text(kit, pad);
        let Some(project) = self.projects.get_mut(kit) else {
            return;
        };
        let Some(region) = project
            .pad(pad)
            .and_then(|p| p.layers.get(layer))
            .map(|l| l.region.clone())
        else {
            return;
        };
        let Some(peak) = project.source(region.source).map(|s| region.peak(s)) else {
            return;
        };
        let peak_db = 20.0 * peak.max(1e-12).log10();
        if peak_db < NORMALIZE_SILENT_DB {
            let line = format!("sample: pad {at} not normalized: it is silent");
            self.told
                .push(format!("Pad {at} is silent: nothing to normalize"));
            self.lines.push(line);
            return;
        }
        let (lo, hi) = wave_tab::GAIN_RANGE_DB;
        let gain = ((-peak_db * 10.0).floor() / 10.0).clamp(lo, hi);
        let was = region.gain_db;
        let r = project.edit_pad(pad, None, |p| p.layers[layer].region.gain_db = gain);
        match r {
            Ok(()) => {
                self.edits += 1;
                if let Some(d) = self.dirty.get_mut(kit) {
                    *d = true;
                }
                let most = if gain >= hi {
                    format!(" (the most), its peak now {:.1} dB", peak_db + gain)
                } else {
                    String::new()
                };
                self.lines.push(format!(
                    "sample: pad {at} normalized: its peak {peak_db:.1} dB, gain {was:+.1} -> {gain:+.1} dB{most}"
                ));
            }
            Err(e) => self
                .lines
                .push(format!("sample: pad {at} not normalized ({e:?})")),
        }
        self.refresh(ui);
    }

    /// Process: a new sound of layer `layer` of pad `pad`'s region, made on
    /// a thread of its own: stretched from its warp tempo to the project's
    /// when it stretches (its pins made part of the audio), or repitched
    /// to it when it repitches (stretched, then moved by the semitones the
    /// tape would), and moved `semitones` more. When it is done it takes
    /// the region's place ([`Self::poll_process`]), one undo step.
    fn process(&mut self, ui: &mut Ui, pad: usize, layer: usize, semitones: i32) {
        self.lift();
        if self.processing.is_some() {
            return;
        }
        let kit = self.kit;
        let at = self.pad_text(kit, pad);
        let tempo = self.tempo();
        let Some(project) = self.projects.get(kit) else {
            return;
        };
        let Some(region) = project
            .pad(pad)
            .and_then(|p| p.layers.get(layer))
            .map(|l| &l.region)
        else {
            return;
        };
        let (what, text) = process_of(pad, layer, region, tempo, semitones);
        let say_not = |s: &mut Self, why: String| {
            s.told.push(format!("Not processed: {why}"));
            s.lines
                .push(format!("sample: pad {at} not processed ({why})"));
        };
        let Some(text) = text else {
            say_not(
                self,
                "it would come out the same: set Shift, or turn Warp on".to_string(),
            );
            return;
        };
        let channels = project
            .source(region.source)
            .map_or(1, |s| u64::from(s.channels()));
        // The input's copy, the stretch's working copy and the result.
        let need = (region.frames() as f64 * (2.0 + what.ratio) * channels as f64 * 4.0) as u64;
        if let Err(why) = crate::memory::room_in((self.mem_free)(), need) {
            say_not(self, why);
            return;
        }
        let spec = match what.spec(project) {
            Ok(s) => s,
            Err(e) => {
                say_not(self, e.to_string());
                return;
            }
        };
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicU32::new(0));
        let (stopped, made) = (stop.clone(), done.clone());
        let run = std::thread::Builder::new()
            .name("mpfree-process".into())
            .spawn(move || {
                // After the sound and the screen, as the warp copies are.
                mpfree_lanes::os::idle_job();
                let t = Instant::now();
                let r = (|| {
                    let mut job = spec.start().map_err(|e| e.to_string())?;
                    loop {
                        if stopped.load(Ordering::Relaxed) {
                            return Err("stopped".to_string());
                        }
                        let p = job.run(PROCESS_STEP);
                        made.store(p.done.to_bits(), Ordering::Relaxed);
                        if p.finished {
                            break;
                        }
                    }
                    job.into_output()
                        .ok_or_else(|| "nothing came out".to_string())
                })();
                let _ = tx.send(r.map(|audio| (audio, t.elapsed())));
            });
        match run {
            Ok(_) => {
                self.lines
                    .push(format!("sample: processing pad {at}: {text}"));
                self.processing = Some(Processing {
                    kit,
                    what,
                    at,
                    text,
                    rx,
                    stop,
                    done,
                    shown: 0.0,
                });
                if let (Some(tab), Some(st)) = (self.warp.as_mut(), self.state.as_ref()) {
                    tab.set_processing(ui, st, Some(0.0));
                }
            }
            Err(e) => say_not(self, e.to_string()),
        }
        self.refresh(ui);
    }

    /// The Process under way: its progress on the Warp tab while it shows,
    /// and once it is done, the new sound in the region's place (one undo
    /// step), or why not.
    fn poll_process(&mut self, ui: &mut Ui, showing: bool) {
        let Some(p) = self.processing.as_mut() else {
            return;
        };
        let r = match p.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => {
                let done = f32::from_bits(p.done.load(Ordering::Relaxed));
                // Each per cent, while the Warp tab shows.
                if showing && self.editor == Editor::Warp && (done - p.shown).abs() >= 0.01 {
                    p.shown = done;
                    if let (Some(tab), Some(st)) = (self.warp.as_mut(), self.state.as_ref()) {
                        tab.set_processing(ui, st, Some(done));
                    }
                }
                return;
            }
            Err(TryRecvError::Disconnected) => Err("its thread stopped".to_string()),
        };
        let Some(p) = self.processing.take() else {
            return;
        };
        match r {
            Ok((audio, took)) => {
                let Some(project) = self.projects.get_mut(p.kit) else {
                    return;
                };
                match p.what.finish(project, audio) {
                    Ok(id) => {
                        self.edits += 1;
                        if let Some(d) = self.dirty.get_mut(p.kit) {
                            *d = true;
                        }
                        let made = project.source(id).map_or(String::new(), |s| {
                            format!(": {}, {}", s.name, duration_text(s.frames(), s.rate()))
                        });
                        self.lines.push(format!(
                            "sample: pad {} processed into a new sound, {}{made}, in {:.2} s",
                            p.at,
                            p.text,
                            took.as_secs_f64()
                        ));
                    }
                    Err(e) => self.lines.push(format!(
                        "sample: pad {} not processed (it changed meanwhile: {e:?})",
                        p.at
                    )),
                }
            }
            Err(why) => self
                .lines
                .push(format!("sample: pad {} not processed ({why})", p.at)),
        }
        if let (Some(tab), Some(st)) = (self.warp.as_mut(), self.state.as_ref()) {
            tab.set_processing(ui, st, None);
        }
        self.refresh(ui);
    }

    /// A number a tab asked to be typed (Jay after live test 3: the Warp
    /// tempo), for the screen's number pad.
    pub fn take_number_ask(&mut self) -> Option<mpfree_ui::numpad::NumberAsk> {
        self.number_ask.take()
    }

    /// Names a tap on the pad bar asked to be typed, for the screen's text
    /// keyboard.
    pub fn take_text_ask(&mut self) -> Option<Vec<mpfree_ui::textpad::TextAsk>> {
        self.text_ask.take()
    }

    /// The pad shown, and the sound of its layer the tabs edit, to be
    /// named: the pad first, its sound on the keyboard's switch.
    fn ask_names(&mut self) {
        let kit = self.kit;
        let at = self.pad_text(kit, self.pad);
        let layer = self.layer();
        let Some(project) = self.projects.get(kit) else {
            return;
        };
        let Some(pad) = project.pad(self.pad).filter(|p| !p.layers.is_empty()) else {
            return;
        };
        let mut asks = vec![mpfree_ui::textpad::TextAsk {
            key: NAME_PAD.to_string(),
            label: "Pad".to_string(),
            title: format!("Name pad {at}"),
            value: pad.name.clone(),
            max_len: NAME_MAX,
        }];
        let sound = pad
            .layers
            .get(layer)
            .and_then(|l| project.source(l.region.source));
        if let Some(s) = sound {
            let users = project.users_of(s.id()).len();
            let on = if users > 1 {
                format!(" (on {users} pads)")
            } else {
                String::new()
            };
            asks.push(mpfree_ui::textpad::TextAsk {
                key: NAME_SOUND.to_string(),
                label: "Sound".to_string(),
                title: format!("Name pad {at}'s sound{on}"),
                value: s.name.clone(),
                max_len: NAME_MAX.max(s.name.chars().count()),
            });
        }
        // A pad named from a long file name keeps it until shortened.
        asks[0].max_len = NAME_MAX.max(pad.name.chars().count());
        self.text_ask = Some(asks);
    }

    /// The names typed (by [`NAME_PAD`] and [`NAME_SOUND`]): the pad shown
    /// and the sound of its layer take them, one undo step, with a line
    /// in the log and on the hint line.
    pub fn named(&mut self, ui: &mut Ui, names: &[(String, String)]) {
        if names.is_empty() {
            return;
        }
        self.lift();
        let kit = self.kit;
        let pad = self.pad;
        let at = self.pad_text(kit, pad);
        let layer = self.layer();
        let Some(project) = self.projects.get_mut(kit) else {
            return;
        };
        let Some(source) = project
            .pad(pad)
            .and_then(|p| p.layers.get(layer))
            .map(|l| l.region.source)
        else {
            return;
        };
        let typed = |key: &str| {
            names
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, n)| n.as_str())
        };
        let pad_name = typed(NAME_PAD);
        let sound_name = typed(NAME_SOUND);
        match project.rename(pad_name.map(|n| (pad, n)), sound_name.map(|n| (source, n))) {
            Ok(()) => {
                self.edits += 1;
                if let Some(d) = self.dirty.get_mut(kit) {
                    *d = true;
                }
                if pad_name.is_some() {
                    self.mark_renamed(kit);
                }
                let line = match (pad_name, sound_name) {
                    (Some(p), Some(s)) => format!("pad {at} named {p}, its sound {s}"),
                    (Some(p), None) => format!("pad {at} named {p}"),
                    (None, Some(s)) => format!("pad {at}'s sound named {s}"),
                    (None, None) => return,
                };
                self.told.push(format!("P{}", &line[1..]));
                self.lines.push(format!("sample: {line}"));
            }
            Err(e) => self
                .lines
                .push(format!("sample: pad {at} not named ({e:?})")),
        }
        self.refresh(ui);
    }

    /// The number typed for `key`: the tab showing makes the change, an
    /// undo step of its own.
    pub fn typed(&mut self, ui: &mut Ui, key: &str, value: f64, out: &mut Vec<FromUi>) {
        let Some(st) = self.state.as_ref() else {
            return;
        };
        let a = match self.editor {
            Editor::Warp => self.warp.as_mut().and_then(|t| t.typed(st, key, value)),
            _ => self.tab.as_mut().and_then(|t| t.typed(st, key, value)),
        };
        if let Some(a) = a {
            self.act(ui, a, out);
            self.lift();
        }
    }

    fn audition(&mut self, pad: usize, out: &mut Vec<FromUi>) {
        if let Some((held, on_pads)) = self.auditioning.take() {
            out.push(Self::pad_release(held, on_pads));
            return;
        }
        let held = self.state.as_ref().is_some_and(|st| {
            matches!(
                st.region.mode,
                PlayMode::Gate | PlayMode::Loop | PlayMode::Granular
            )
        });
        let (press, on_pads) = self.pad_press(pad, true);
        out.push(press);
        if held {
            self.auditioning = Some((pad, on_pads));
        } else {
            out.push(Self::pad_release(pad, on_pads));
        }
    }

    /// A finger lifted or a Q-Link was let go: the drag or turn is over,
    /// so the next edit is a new undo step. Writes its log line.
    pub fn lift(&mut self) {
        if let Some(tab) = self.tab.as_mut() {
            tab.lift();
        }
        if let Some(tab) = self.warp.as_mut() {
            tab.lift();
        }
        if let Some(tab) = self.play.as_mut() {
            tab.lift();
        }
        if let Some(tab) = self.pads.as_mut() {
            tab.lift();
        }
        if let Some(tab) = self.grain.as_mut() {
            tab.lift();
        }
        // The effects' knobs turned are let go, as the chain editor's are.
        for addr in self.fx_touched.drain(..) {
            self.fx_out.push(FromUi::Chain(ChainAction::Release(addr)));
        }
        self.last_turn = None;
        self.play_turn = None;
        let Some(g) = self.gesture.take() else { return };
        if let Before::Slot(before) = &g.before {
            let after = self.projects.get(g.kit).and_then(|p| p.pad(g.pad));
            let what = describe_slot(before.as_deref(), after);
            if !what.is_empty() {
                let kit = self.kit_text_of(g.kit);
                self.lines.push(format!(
                    "sample: {kit} pad {}: {what}",
                    self.pad_text(g.kit, g.pad)
                ));
            }
            return;
        }
        let Some(after) = self.projects.get(g.kit).and_then(|p| p.pad(g.pad)) else {
            return;
        };
        let what = match &g.before {
            Before::Region { layer, region } => {
                let Some(l) = after.layers.get(*layer) else {
                    return;
                };
                describe(region, &l.region, self.rate)
            }
            Before::Grain { layer, region } => {
                let Some(l) = after.layers.get(*layer) else {
                    return;
                };
                grain_tab::describe(region, &l.region)
            }
            Before::Pad(before) => play_tab::describe(before, after),
            Before::Slot(_) => String::new(),
        };
        if !what.is_empty() {
            let name = self.projects[g.kit]
                .pad(g.pad)
                .map_or(String::new(), |p| p.name.clone());
            let kit = self.kit_text_of(g.kit);
            self.lines.push(format!(
                "sample: {kit} pad {} {name}: {what}",
                self.pad_text(g.kit, g.pad)
            ));
        }
    }

    /// Leaving the page: let go of a pad a tap holds, end the gesture.
    pub fn leave(&mut self, out: &mut Vec<FromUi>) {
        if let Some((pad, on_pads)) = self.auditioning.take() {
            out.push(Self::pad_release(pad, on_pads));
        }
        if self.live.is_some() {
            self.stop_live("stopped");
        }
        self.lift();
    }

    /// Live chop ends: the sound stops (the audio thread hears of it with
    /// the next actions) and the log says how many cuts it made.
    fn stop_live(&mut self, why: &str) {
        let Some(run) = self.live.take() else { return };
        self.hits.push(FromUi::SampleLiveChop {
            kit: run.kit,
            pad: run.pad,
            layer: run.layer,
            start: 0,
            end: 0,
            on: false,
        });
        let n = run.kept;
        self.lines.push(format!(
            "sample: live chop {why} on pad {}: {n} cut{} made",
            self.pad_text(run.kit, run.pad),
            if n == 1 { "" } else { "s" }
        ));
    }

    /// Whether the Chop tab chops live (its hint says how).
    pub fn chop_live_method(&self) -> bool {
        self.page() == CHOP_PAGE
            && self
                .chop
                .as_ref()
                .is_some_and(|t| t.method() == mpfree_sampler_ui::chop_tab::Method::Live)
    }

    /// Whether UNDO has something to take back in the playing kit (for
    /// the UNDO light).
    pub fn can_undo(&self) -> bool {
        self.projects.get(self.kit).is_some_and(Project::can_undo)
    }

    /// UNDO (or redo) in the playing kit.
    pub fn undo(&mut self, ui: &mut Ui, redo: bool) {
        self.lift();
        let kit = self.kit;
        let what = if redo { "redo" } else { "undo" };
        let Some(p) = self.projects.get_mut(kit) else {
            self.lines.push(format!(
                "sample: nothing to {what} (the synth has the pads)"
            ));
            return;
        };
        if if redo { p.redo() } else { p.undo() } {
            self.edits += 1;
            if let Some(d) = self.dirty.get_mut(kit) {
                *d = true;
            }
            self.mark_renamed(kit);
            self.lines
                .push(format!("sample: {what} in {}", self.kit_text()));
            self.refresh(ui);
        } else {
            self.lines
                .push(format!("sample: nothing to {what} in {}", self.kit_text()));
        }
    }

    /// The playing kit's undo history as the list shows it (hold UNDO):
    /// "Start", each step undo takes back, oldest first, then redo's, and
    /// the row that is now. None while the synth has the pads.
    pub fn history_rows(&self) -> Option<(String, Vec<String>, usize)> {
        let kit = self.kit;
        let p = self.projects.get(kit)?;
        let (done, redo) = p.history();
        let mut rows = vec!["Start: before these edits".to_string()];
        rows.extend(done.iter().map(|&i| self.step_name(kit, i)));
        let now = rows.len() - 1;
        rows.extend(redo.iter().map(|&i| self.step_name(kit, i)));
        let mut title = self.kit_text();
        if let Some(c) = title.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        Some((title, rows, now))
    }

    /// An undo step as the history names it: "Pad A3", "Chop to 8 pads
    /// from A5", "New sample on pad A1".
    fn step_name(&self, kit: usize, i: mpfree_sampler::model::StepInfo) -> String {
        let first = i
            .first_pad
            .map(|p| self.pad_text(kit, p))
            .unwrap_or_default();
        let pads = |n: usize| {
            if n == 1 {
                format!("pad {first}")
            } else {
                format!("{n} pads from {first}")
            }
        };
        match i {
            _ if i.chop && i.pads > 0 => format!("Chop to {}", pads(i.pads)),
            _ if i.pads_play && i.pads > 0 => format!("Keys on {}", pads(i.pads)),
            _ if i.pads_play => "What the pads play".to_string(),
            _ if i.sources > 0 && i.pads > 0 => format!("New sample on {}", pads(i.pads)),
            _ if i.sources > 0 => "New sample".to_string(),
            _ if i.chop => "New chop".to_string(),
            _ if i.master => "Master effect".to_string(),
            _ if i.pads > 0 => {
                let p = pads(i.pads);
                let mut c = p.chars();
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or(p)
            }
            _ => "Edit".to_string(),
        }
    }

    // ------------------------------------------------------------ rec

    /// What the Rec tab shows besides the input: the pad picked and where
    /// a take would go.
    fn rec_state_now(&self) -> RecState {
        let project = self.projects.get(self.kit);
        let sound = |p: usize| {
            project
                .and_then(|pr| pr.pad(p))
                .filter(|pd| !pd.layers.is_empty())
        };
        let onto = self
            .rec
            .as_ref()
            .map_or(rec_tab::Onto::NextEmpty, |t| t.settings.onto);
        let target = project.and_then(|_| rec_tab::target(self.pad, onto, |p| sound(p).is_some()));
        let can_flatten = self.flatten.is_none()
            && self.take.is_none()
            && self.pad < crate::mixing::PADS
            && sound(self.pad).is_some()
            && project.is_some_and(|p| p.pads_play() == PadsPlay::Pads)
            && target.is_some();
        RecState {
            pad: self.pad,
            pad_name: sound(self.pad).map(|pd| pd.name.clone()),
            target,
            sources: self.rec_sources.iter().map(|(n, _)| n.clone()).collect(),
            live: self.input_wanted(),
            last: self.last_take.clone(),
            can_flatten,
            flattening: self.flatten.as_ref().map(|f| {
                format!(
                    "Flattening {} onto {}",
                    self.pad_text(f.kit, f.pad),
                    self.pad_text(f.kit, f.target)
                )
            }),
        }
    }

    fn refresh_rec(&mut self, ui: &mut Ui, bank: Option<char>) {
        let st = self.rec_state_now();
        match self.rec.as_mut() {
            Some(tab) => {
                tab.bank = bank;
                tab.update(ui, &st);
            }
            None => match RecTab::add(ui, REC_PAGE, &st, self.rate) {
                Ok(mut tab) => {
                    tab.bank = bank;
                    tab.update(ui, &st);
                    self.rec = Some(tab);
                }
                Err(e) => {
                    self.lines.push(format!("sample: no Rec page ({e})"));
                    return;
                }
            },
        }
        self.rec_state = Some(st);
        self.feed_rec(ui, 0.0);
    }

    /// The Rec tab's state again: the target follows the Onto row and the
    /// pads.
    fn refresh_rec_state(&mut self, ui: &mut Ui) {
        if self.rec_state.is_none() {
            return;
        }
        let st = self.rec_state_now();
        if let Some(tab) = self.rec.as_mut() {
            tab.update(ui, &st);
        }
        self.rec_state = Some(st);
    }

    /// The Rec tab's picture: what came in over the last `dt` seconds.
    fn feed_rec(&mut self, ui: &mut Ui, dt: f32) {
        let live = self.input_wanted();
        let Some(tab) = self.rec.as_mut() else {
            return;
        };
        match self.input.as_ref() {
            Some(input) => tab.feed(ui, input, self.take.as_ref().map(|t| &t.rec), live, dt),
            None => tab.idle(ui, live),
        }
    }

    fn act_rec(&mut self, ui: &mut Ui, a: RecAction) {
        match a {
            RecAction::Tab(i) => self.open(ui, i),
            RecAction::Record => self.start_take(ui),
            RecAction::Stop if self.flatten.is_some() => {
                self.end_flatten(ui, "stopped: nothing kept");
            }
            RecAction::Stop => {
                if let (Some(t), Some(input)) = (self.take.as_mut(), self.input.as_ref()) {
                    t.rec.stop(input);
                }
                // A take that was recording ends here and lands at once.
                self.tick(ui, 0.0, true);
            }
            RecAction::Flatten => self.start_flatten(ui),
            RecAction::SkipBack => self.skip_back(ui),
            RecAction::Source(i) => self.set_source(ui, i),
            RecAction::Auto => {
                self.editor = Editor::Auto;
                self.refresh(ui);
            }
        }
    }

    /// The line input, or a point in the mix, is wanted in the buffer.
    fn input_wanted(&self) -> bool {
        self.source != RecSource::Line || self.line_on
    }

    /// The Source row picked source `i`: the audio thread is told (it
    /// starts the buffer afresh), and the buffer is made or freed as the
    /// new source needs it.
    fn set_source(&mut self, ui: &mut Ui, i: usize) {
        let Some((name, src)) = self.rec_sources.get(i).cloned() else {
            return;
        };
        if src == self.source {
            return;
        }
        self.source = src;
        self.lines.push(match src {
            RecSource::Line => "sample: the Rec tab records the line input".to_string(),
            RecSource::Mix(..) => format!("sample: the Rec tab resamples {name}"),
        });
        self.sync_input(ui);
    }

    /// The source's name, as the Source row shows it.
    pub(crate) fn source_name(&self) -> String {
        self.rec_sources
            .iter()
            .find(|(_, s)| *s == self.source)
            .map_or_else(|| "Line in".to_string(), |(n, _)| n.clone())
    }

    /// [`SamplePage::take_input`]'s source found no room: again later.
    pub fn source_unsent(&mut self, _src: RecSource) {
        self.source_sent = None;
    }

    /// A bar's length now: the transport's while it plays, else 4/4 at the
    /// project tempo.
    fn bar_frames(&self, input: &Input) -> f64 {
        input
            .bars
            .bar_frames()
            .unwrap_or_else(|| 4.0 * 60.0 / self.tempo().max(1.0) * f64::from(self.rate))
    }

    fn start_take(&mut self, ui: &mut Ui) {
        if self.take.is_some() || self.flatten.is_some() {
            return;
        }
        if let Some(run) = self.pad_run {
            self.lines.push(format!(
                "sample: no take: {} is armed for one take (REC or STOP ends it)",
                crate::rec_arm::pad_name(run.pad)
            ));
            return;
        }
        let st = self.rec_state_now();
        let (Some(input), Some(tab)) = (self.input.as_ref(), self.rec.as_ref()) else {
            self.lines
                .push("sample: no take: the line input is off".to_string());
            return;
        };
        let Some(pad) = st.target else {
            self.lines
                .push("sample: no take: no empty pad in this bank".to_string());
            return;
        };
        let settings = tab.settings;
        let rec = settings.recording(input, self.bar_frames(input));
        self.lines.push(format!(
            "sample: take armed for pad {}: {}",
            self.pad_text(self.kit, pad),
            take_plan(&settings, &st.sources)
        ));
        if self.source != RecSource::Line {
            self.lines
                .push(format!("sample: the take resamples {}", self.source_name()));
        }
        self.take = Some(Take {
            rec,
            kit: self.kit,
            pad,
            pick: settings.pick,
            bars: settings.in_bars(),
            pad_take: false,
        });
        self.feed_rec(ui, 0.0);
    }

    fn skip_back(&mut self, ui: &mut Ui) {
        if self.take.is_some() || self.flatten.is_some() {
            return;
        }
        let st = self.rec_state_now();
        let (Some(input), Some(tab)) = (self.input.as_ref(), self.rec.as_ref()) else {
            return;
        };
        let Some(pad) = st.target else {
            self.lines
                .push("sample: no skip back: no empty pad in this bank".to_string());
            return;
        };
        let settings = tab.settings;
        let data = match settings.skip {
            Skip::Seconds(n) => input.last_frames(u64::from(n) * u64::from(input.rate)),
            Skip::Bars(n) => input.last_bars(n as usize),
        };
        let what = format!("skip back {}", settings.skip.label());
        let rate = input.rate;
        let name = self.take_name();
        // Whole bars follow the project tempo.
        let bpm = matches!(settings.skip, Skip::Bars(_)).then(|| self.tempo());
        match data.and_then(|d| settings.pick.source(&name, rate, d)) {
            Ok(src) => self.keep_warped(ui, self.kit, pad, src, &what, bpm),
            Err(e) => self.lines.push(format!("sample: {what} not kept ({e:?})")),
        }
    }

    /// An effect's memory came back from the audio thread (the chain
    /// editor's Capture on the Grains effect, step C3): it goes where a
    /// recording would land (the pad picked if it is empty, else the
    /// bank's next empty pad, or the pad picked whatever it holds when
    /// the Rec tab's Onto says so), playing as grains with the settings
    /// the effect had. One undo step; the buffer is freed here.
    pub fn captured(&mut self, ui: &mut Ui, held: Held, end: Captured) {
        if end != Captured::Done {
            self.lines
                .push(format!("sample: nothing captured: {}", end.why()));
            self.told.push(format!("Nothing captured: {}", end.why()));
            return;
        }
        let Some(pad) = self.rec_state_now().target else {
            self.lines
                .push("sample: nothing captured: no empty pad in this bank".to_string());
            self.told
                .push("Nothing captured: no empty pad in this bank".to_string());
            return;
        };
        let grain = mpfree_grain::node::voice_params(held.values());
        let (rate, ch) = (held.rate(), held.channels() as u8);
        self.captures += 1;
        let name = format!("Grains {}", self.captures);
        match Source::new(&name, rate, ch, held.into_data()) {
            Ok(src) => {
                self.keep_as(
                    ui,
                    self.kit,
                    pad,
                    src,
                    "grains capture",
                    ", playing as grains",
                    |l| {
                        l.region.mode = PlayMode::Granular;
                        l.region.grain = grain;
                    },
                );
                self.told
                    .push(format!("{name} on pad {}", self.pad_text(self.kit, pad)));
            }
            Err(e) => self
                .lines
                .push(format!("sample: grains capture not kept ({e:?})")),
        }
    }

    fn take_name(&mut self) -> String {
        self.takes += 1;
        match self.source {
            RecSource::Line => format!("Take {}", self.takes),
            RecSource::Mix(..) => format!("{} take {}", self.source_name(), self.takes),
        }
    }

    /// Puts a recorded sound on pad `pad` of kit `kit` (one undo step) and
    /// picks that pad.
    fn keep(&mut self, ui: &mut Ui, kit: usize, pad: usize, src: Source, what: &str) {
        self.keep_warped(ui, kit, pad, src, what, None);
    }

    /// [`Self::keep`], warped at `bpm` when given (whole bars played at
    /// that tempo: they follow the project's from then on).
    fn keep_warped(
        &mut self,
        ui: &mut Ui,
        kit: usize,
        pad: usize,
        src: Source,
        what: &str,
        bpm: Option<f64>,
    ) {
        let warp = bpm.map(|b| Warp::new((b as f32).clamp(*WARP_BPM.start(), *WARP_BPM.end())));
        let warped = warp
            .as_ref()
            .map(|w| format!(", warped at {:.1} BPM", w.bpm))
            .unwrap_or_default();
        self.keep_as(ui, kit, pad, src, what, &warped, |layer| {
            layer.region.warp = warp
        });
    }

    /// [`Self::keep`] with `set` changing the new layer first, and `after`
    /// at the end of the line that says so.
    // The sound and where it goes, then what to say and do: one call each.
    #[allow(clippy::too_many_arguments)]
    fn keep_as(
        &mut self,
        ui: &mut Ui,
        kit: usize,
        pad: usize,
        src: Source,
        what: &str,
        after: &str,
        set: impl FnOnce(&mut Layer),
    ) {
        let (name, frames, rate, ch) = (src.name.clone(), src.frames(), src.rate(), src.channels());
        let Some(project) = self.projects.get_mut(kit) else {
            self.lines
                .push(format!("sample: {what} not kept (the kit is gone)"));
            return;
        };
        match project.put_source_as(pad, src, |name, mut layer, old| {
            set(&mut layer);
            match old {
                Some(_) => Pad::replacing(name, layer, old),
                None => crate::sampler::kit_pad_with(name, layer),
            }
        }) {
            Ok(_) => {
                self.edits += 1;
                if let Some(d) = self.dirty.get_mut(kit) {
                    *d = true;
                }
                self.mark_renamed(kit);
                let text = format!(
                    "{name}, {} {} on pad {}",
                    duration_text(frames, rate),
                    if ch == 2 { "stereo" } else { "mono" },
                    self.pad_text(kit, pad)
                );
                self.lines.push(format!("sample: {what}: {text}{after}"));
                self.last_take = text;
                if kit == self.kit {
                    self.lift();
                    self.pad = pad;
                    self.velocity = None;
                }
                self.refresh(ui);
            }
            Err(e) => self.lines.push(format!("sample: {what} not kept ({e:?})")),
        }
    }

    // ------------------------------------------------------------ saves

    /// The sampler's part of a saved project (its owner is `sampler`): the
    /// kits as text, and each sound they use once, by its file name in the
    /// project's `samples/`. The app hands the sounds to the library's save
    /// (`mpfree_library::NewSample` writing [`wav::write`]), which writes
    /// the ones the project hasn't got yet before the part.
    #[allow(dead_code)] // the app's browser wiring calls it
    pub fn save_part(&self) -> (String, Vec<(String, Source)>) {
        let kits: Vec<(&str, &Project)> = self
            .projects
            .iter()
            .enumerate()
            .map(|(k, p)| (self.names.get(k).map_or("", String::as_str), p))
            .collect();
        let text = save::to_text(&kits, self.kit, &wav::file_name);
        let mut seen = HashSet::new();
        let mut sounds = Vec::new();
        for s in self.projects.iter().flat_map(|p| p.sources()) {
            if seen.insert(s.id()) {
                sounds.push((wav::file_name(s), s.clone()));
            }
        }
        (text, sounds)
    }

    /// The analysis of each sound [`SamplePage::save_part`] names, to keep
    /// beside it in the project's `samples/` (`<id>.a1.peaks`,
    /// [`cache_file`]): each sound's view as far as it is worked out
    /// (ready, with its peaks), so the project opens again without
    /// analysing them. A sound still being worked out has none yet; the
    /// next save keeps its file.
    #[allow(dead_code)] // the app's browser wiring calls it
    pub fn cache_part(&self) -> Vec<(String, Arc<SourceView>)> {
        let mut seen = HashSet::new();
        self.projects
            .iter()
            .flat_map(|p| p.sources())
            .filter(|s| seen.insert(s.id()))
            .filter_map(|s| self.sources.ready(s.id()))
            .filter(|v| v.pyramid.is_some())
            .map(|v| (cache_file(v.id), v))
            .collect()
    }

    /// Opens the sampler's part of a project (`text`, as
    /// [`SamplePage::save_part`] made it). The sounds are read (`read` gives
    /// a file's bytes from the project's `samples/`) and decoded on a
    /// thread of their own; the kits change on a later
    /// [`SamplePage::tick`], each kit in its place, and the log says what
    /// couldn't be used. A take or auto-sample under way is dropped then,
    /// as when the kits are loaded.
    #[allow(dead_code)] // the app's browser wiring calls it
    pub fn open_part(&mut self, text: String, read: ReadSample) {
        let (tx, rx) = mpsc::channel();
        let (free, rate) = (self.mem_free, self.rate);
        let spawned = std::thread::Builder::new()
            .name("sample-open".into())
            .spawn(move || {
                let _ = crate::cores::background(); // off the sound's cores and priority
                                                    // Each sound only while it leaves the memory MPFree keeps
                                                    // free; one that doesn't is left out and the log says why.
                let r = save::from_text(&text, &mut |f| {
                    let bytes = read(f)?;
                    crate::memory::room_in(
                        free(),
                        crate::memory::decode_need(f, bytes.len() as u64),
                    )?;
                    wav::read(f, &bytes)
                });
                let _ = tx.send(r.map(|o| {
                    let (o, said) = opened_at_rate(o, rate, free);
                    let views = cached_views(&o, &read);
                    (o, said, views)
                }));
            });
        match spawned {
            Ok(_) => {
                self.opening = Some(rx);
                self.lines
                    .push("sample: opening the project's kits".to_string());
            }
            Err(e) => self
                .lines
                .push(format!("sample: the project's kits weren't opened ({e})")),
        }
    }

    /// The kit the pads play as a kit file (`mpfree_library::kit`): its
    /// name, its `kit.txt` (the sampler's part for that one kit) and each
    /// sound it names, by a file name made from the sound's name (`Kick.wav`,
    /// `Kick 2.wav` for a second sound of that name). None when the pads play
    /// no kit of the page's (the synth).
    pub fn kit_file(&self) -> Option<KitFile> {
        let k = self.kit;
        let p = self.projects.get(k)?;
        let name = self
            .names
            .get(k)
            .cloned()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| format!("Kit {}", k + 1));
        let (text, files) = kit_file_of(&name, p);
        Some((name, text, files))
    }

    /// The names of the kits the app first gave, once they have come.
    pub fn factory_names(&self) -> Option<&[String]> {
        self.factory
            .as_ref()
            .map(|(_, n)| n.as_slice())
            .filter(|n| !n.is_empty())
    }

    /// The kits the app first gave (before any edit) whose names `keep`
    /// takes, each as [`SamplePage::kit_file`] gives one.
    pub fn factory_kit_files(&self, keep: &dyn Fn(&str) -> bool) -> Vec<KitFile> {
        let Some((projects, names)) = &self.factory else {
            return Vec::new();
        };
        projects
            .iter()
            .zip(names)
            .filter(|(_, n)| keep(n))
            .map(|(p, n)| {
                let (text, files) = kit_file_of(n, p);
                (n.clone(), text, files)
            })
            .collect()
    }

    /// Loads a kit file (`dir` is its folder: `kit.txt` and its sounds) in
    /// place of the kit the pads play, named for the folder. Read and decoded
    /// on a thread of its own; the kit changes on a later
    /// [`SamplePage::tick`]. What can't be used is left out and said in the
    /// log. A load while another is under way replaces it.
    pub fn open_kit(&mut self, dir: PathBuf) {
        let Some(k) = (self.kit < self.projects.len()).then_some(self.kit) else {
            self.told
                .push("Pick a drum kit first: the pads play the synth".to_string());
            return;
        };
        self.open_kit_into(dir, k);
    }

    /// As [`SamplePage::open_kit`], into kit `k`: an empty user kit the
    /// armed track moves to when other tracks play the kit on the pads.
    pub fn open_kit_into(&mut self, dir: PathBuf, k: usize) {
        if k >= self.projects.len() {
            self.lines
                .push(format!("sample: no kit {} to load into", k + 1));
            return;
        }
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Kit")
            .to_string();
        let (tx, rx) = mpsc::channel();
        let (free, rate) = (self.mem_free, self.rate);
        let spawned = std::thread::Builder::new()
            .name("sample-kit".into())
            .spawn(move || {
                let _ = crate::cores::background(); // off the sound's cores and priority
                let r = read_kit_file(&dir, &mut |f, len| {
                    crate::memory::room_in(free(), crate::memory::decode_need(f, len))
                });
                let _ = tx.send(r.map(|o| opened_at_rate(o, rate, free)));
            });
        match spawned {
            Ok(_) => {
                self.kit_loading = Some((rx, k, name.clone()));
                self.lines.push(format!("sample: loading the kit {name}"));
            }
            Err(e) => self
                .lines
                .push(format!("sample: the kit {name} wasn't loaded ({e})")),
        }
    }

    fn poll_kit(&mut self, ui: &mut Ui) {
        let Some((rx, _, _)) = &self.kit_loading else {
            return;
        };
        let r = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("its thread stopped".to_string()),
        };
        let Some((_, k, name)) = self.kit_loading.take() else {
            return;
        };
        let (mut o, converted) = match r {
            Ok(o) => o,
            Err(e) => {
                self.lines
                    .push(format!("sample: the kit {name} wasn't loaded ({e})"));
                self.told.push(format!("The kit {name} wasn't loaded: {e}"));
                return;
            }
        };
        for n in o.notes.iter().chain(&converted) {
            self.lines.push(format!("sample: {n}"));
        }
        if o.kits.is_empty() || k >= self.projects.len() {
            self.lines
                .push(format!("sample: the kit {name} has no kit in it"));
            return;
        }
        let (_, p) = o.kits.swap_remove(0);
        let mut projects = std::mem::take(&mut self.projects);
        let mut names = self.names.clone();
        projects[k] = p;
        if let Some(n) = names.get_mut(k) {
            n.clone_from(&name);
        }
        let rate = self.rate;
        self.set_kits(ui, projects, rate);
        self.set_names(names);
        self.dirty[k] = true;
        self.mark_renamed(k);
        // The work changed: the app marks it unsaved.
        self.edits += 1;
        let left = o.notes.len();
        self.told.push(if left == 0 {
            format!("Loaded the kit {name} on kit {}", k + 1)
        } else {
            format!(
                "Loaded the kit {name} on kit {}; {left} sounds left out (the Console says why)",
                k + 1
            )
        });
        self.lines
            .push(format!("sample: kit {} is now {name}", k + 1));
    }

    /// The kit the project just opened was playing, once, for the app to
    /// switch the pads to.
    pub fn take_wanted_kit(&mut self) -> Option<usize> {
        self.wanted_kit.take()
    }

    /// The browser's Load (`to_pad` false: onto the pad shown, one undo
    /// step) and Load to pad (`to_pad`: onto the next pad hit). The WAV,
    /// AIFF or FLAC file is read and decoded on a thread of its own and
    /// lands on a later [`SamplePage::tick`]. A Load while another is under way
    /// replaces it.
    #[allow(dead_code)] // the app's browser wiring calls it
    pub fn load_file(&mut self, path: PathBuf, to_pad: bool) {
        let (tx, rx) = mpsc::channel();
        let free = self.mem_free;
        let rate = self.rate;
        let spawned = std::thread::Builder::new()
            .name("sample-load".into())
            .spawn(move || {
                let _ = crate::cores::background(); // off the sound's cores and priority
                let _ = tx.send(read_sound(&path, free, rate));
            });
        match spawned {
            Ok(_) => self.loading = Some((rx, to_pad)),
            Err(e) => self.lines.push(format!("sample: not loaded ({e})")),
        }
    }

    /// A sound Load to pad is holding for the next pad hit.
    #[allow(dead_code)] // the app's browser wiring calls it
    pub fn assigning(&self) -> Option<&str> {
        self.assign.as_ref().map(|l| l.src.name.as_str())
    }

    fn poll_open(&mut self, ui: &mut Ui) {
        let Some(rx) = &self.opening else { return };
        let r = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("its thread stopped".to_string()),
        };
        self.opening = None;
        match r {
            Ok((o, converted, views)) => {
                self.apply_opened(ui, o);
                for n in converted {
                    self.lines.push(format!("sample: {n}"));
                }
                // The views kept beside the sounds: each sound still here
                // shows its peaks, hits, tempo and note at once.
                let here: HashSet<SourceId> = self
                    .projects
                    .iter()
                    .flat_map(|p| p.sources())
                    .map(|s| s.id())
                    .collect();
                let mut n = 0;
                for v in views.into_iter().filter(|v| here.contains(&v.id)) {
                    self.sources.known(v);
                    n += 1;
                }
                if n > 0 {
                    self.lines.push(format!(
                        "sample: the analysis of {n} sound{} read from the project",
                        if n == 1 { "" } else { "s" }
                    ));
                }
            }
            Err(e) => self
                .lines
                .push(format!("sample: the project's kits weren't opened ({e})")),
        }
    }

    /// Puts the kits of an opened project in place of the ones here, kit
    /// by kit; their views go to the audio thread as edits' do.
    fn apply_opened(&mut self, ui: &mut Ui, o: Opened) {
        for n in &o.notes {
            self.lines.push(format!("sample: {n}"));
        }
        let short = o
            .notes
            .iter()
            .filter(|n| n.contains(crate::memory::SHORT))
            .count();
        if short > 0 {
            self.told.push(format!(
                "{short} of the project's sounds left out: {}; the Console lists them",
                crate::memory::SHORT
            ));
        }
        let have = self.projects.len();
        if o.kits.len() > have {
            self.lines.push(format!(
                "sample: the project has {} kits; the first {have} opened",
                o.kits.len()
            ));
        }
        let mut projects = std::mem::take(&mut self.projects);
        let mut names = self.names.clone();
        let mut opened = Vec::new();
        for (k, (name, p)) in o.kits.into_iter().enumerate().take(have) {
            projects[k] = p;
            if let (Some(n), false) = (names.get_mut(k), name.is_empty()) {
                *n = name;
            }
            opened.push(k);
        }
        let rate = self.rate;
        self.set_kits(ui, projects, rate);
        self.set_names(names);
        for &k in &opened {
            self.dirty[k] = true;
            self.mark_renamed(k);
        }
        if o.kit < have && o.kit != self.kit {
            self.wanted_kit = Some(o.kit);
        }
        self.lines.push(match opened.len() {
            1 => "sample: 1 kit opened".to_string(),
            n => format!("sample: {n} kits opened"),
        });
    }

    /// New project: every kit back as the app first had it (the factory
    /// kits, no edits, no undo), kit 1 playing. A take, auto-sample or
    /// open under way is dropped. False before the kits have come.
    pub fn reset_kits(&mut self, ui: &mut Ui) -> bool {
        let Some((projects, names)) = self.factory.clone() else {
            return false;
        };
        self.opening = None;
        self.loading = None;
        self.assign = None;
        let rate = self.rate;
        let n = projects.len();
        self.set_kits(ui, projects, rate);
        if !names.is_empty() {
            self.set_names(names);
        }
        for k in 0..n {
            self.dirty[k] = true;
            self.mark_renamed(k);
        }
        if self.kit != 0 {
            self.wanted_kit = Some(0);
        }
        self.lines
            .push("sample: the kits are back as they started (new project)".to_string());
        true
    }

    fn poll_load(&mut self, ui: &mut Ui) {
        let Some((rx, to_pad)) = &self.loading else {
            return;
        };
        let to_pad = *to_pad;
        let r = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("its thread stopped".to_string()),
        };
        self.loading = None;
        if let Ok(LoadedSound {
            src,
            note: Some(note),
            ..
        }) = &r
        {
            self.lines.push(format!("sample: {}: {note}", src.name));
        }
        match r {
            Ok(l) if to_pad => {
                self.lines
                    .push(format!("sample: hit a pad to put {} on it", l.src.name));
                self.assign = Some(l);
            }
            Ok(l) => {
                let (kit, pad) = (self.kit, self.pad);
                self.keep_loaded(ui, kit, pad, l);
            }
            Err(e) => {
                if e.starts_with(crate::memory::SHORT) {
                    self.told.push(format!("Not loaded: {e}"));
                }
                self.lines.push(format!("sample: not loaded ({e})"));
            }
        }
    }

    /// A loaded sound onto pad `pad` of kit `kit`: with its sidecar, the
    /// pad as it was exported (its sounds, settings and slices, one undo
    /// step); without, or if the sidecar can't be used (the log says
    /// why), the sound alone.
    fn keep_loaded(&mut self, ui: &mut Ui, kit: usize, pad: usize, l: LoadedSound) {
        let o = match l.sidecar {
            None => return self.keep(ui, kit, pad, l.src, "loaded"),
            Some(Err(e)) => {
                self.lines.push(format!(
                    "sample: {}: its settings ({}) not used ({e}); the sound alone is loaded",
                    l.src.name, l.sidecar_name
                ));
                return self.keep(ui, kit, pad, l.src, "loaded");
            }
            Some(Ok(o)) => o,
        };
        for n in &o.notes {
            self.lines.push(format!("sample: {}: {n}", l.sidecar_name));
        }
        let at = self.pad_text(kit, pad);
        let Some(project) = self.projects.get_mut(kit) else {
            return;
        };
        let (name, chops) = (o.pad.name.clone(), o.chops.len());
        match project.put_pad_with(pad, o.sources, o.pad, o.chops) {
            Ok(()) => {
                self.edits += 1;
                if let Some(d) = self.dirty.get_mut(kit) {
                    *d = true;
                }
                self.mark_renamed(kit);
                let chops = match chops {
                    0 => String::new(),
                    1 => " and its slices".to_string(),
                    n => format!(" and {n} chops"),
                };
                self.lines.push(format!(
                    "sample: loaded: {name} on pad {at}, with its settings{chops} from {}",
                    l.sidecar_name
                ));
                self.last_take = name;
                if kit == self.kit {
                    self.lift();
                    self.pad = pad;
                    self.velocity = None;
                }
                self.refresh(ui);
            }
            Err(e) => {
                self.lines.push(format!(
                    "sample: {}: its settings ({}) not used ({e:?}); the sound alone is loaded",
                    l.src.name, l.sidecar_name
                ));
                self.keep(ui, kit, pad, l.src, "loaded");
            }
        }
    }

    // ------------------------------------------------------------ flatten

    /// Flatten: the Rec tab's input turns to the pad picked's own channel
    /// (after its insert effects, before its fader: onto the pad itself,
    /// before its inserts, as they will play again), and once the audio
    /// thread has it, the pad plays once at the Flatten velocity and is
    /// let go after the Flatten hold. The sound is kept from its first
    /// frame over -60 dB until it has been under it for the Silence time,
    /// as a pad of its own (one undo step); the Rec tab's source comes
    /// back after.
    fn start_flatten(&mut self, ui: &mut Ui) {
        let st = self.rec_state_now();
        let (true, Some(target), Some(tab)) = (st.can_flatten, st.target, self.rec.as_ref()) else {
            return;
        };
        let settings = tab.settings;
        let (kit, pad) = (self.kit, self.pad);
        let Some(p) = self.projects.get(kit).and_then(|pr| pr.pad(pad)) else {
            return;
        };
        let toggle = p.layers.iter().any(|l| l.region.mode == PlayMode::Toggle);
        let point = if target == pad {
            CapturePoint::BeforeInserts
        } else {
            CapturePoint::BeforeFader
        };
        let src = RecSource::Mix(StripId::Pad(pad), point);
        let ready = match &self.input {
            Some(_) if self.source == src => Ready::Now,
            // A writer not yet with the audio thread only ever gets the
            // new source (it goes after the source does).
            Some(_) if self.feed.is_some() => Ready::Written,
            Some(input) => Ready::Restarted(input.audio.restarts()),
            None => Ready::Written,
        };
        self.lines.push(format!(
            "sample: flatten {} onto {}: velocity {}, held {} ms, from its channel {}",
            self.pad_text(kit, pad),
            self.pad_text(kit, target),
            settings.flatten_velocity,
            settings.flatten_hold_ms,
            if point == CapturePoint::BeforeInserts {
                "before its inserts"
            } else {
                "after its inserts"
            }
        ));
        self.flatten = Some(Flatten {
            kit,
            pad,
            target,
            velocity: settings.flatten_velocity,
            hold: settings.flatten_hold_ms as f32 / 1000.0,
            toggle,
            was: self.source,
            ready,
            waited: 0.0,
            played: None,
            lifted: false,
        });
        self.source = src;
        self.sync_input(ui);
    }

    /// A Flatten's next step, `dt` seconds on: play the pad once its
    /// channel is in the buffer, let it go after the hold, give up if the
    /// channel or the sound doesn't come.
    fn poll_flatten(&mut self, ui: &mut Ui, dt: f32) {
        let Some(f) = self.flatten.as_ref() else {
            return;
        };
        if f.kit != self.kit {
            self.end_flatten(ui, "stopped: the kit changed");
            return;
        }
        let Some(input) = self.input.as_ref() else {
            return;
        };
        match f.played {
            None => {
                let ready = match f.ready {
                    Ready::Now => true,
                    Ready::Restarted(n) => input.audio.restarts() > n,
                    Ready::Written => input.now() > 0,
                };
                if !ready {
                    let waited = f.waited + dt;
                    if waited > FLATTEN_WAIT {
                        self.end_flatten(ui, "stopped: the pad's channel didn't come in");
                    } else if let Some(f) = self.flatten.as_mut() {
                        f.waited = waited;
                    }
                    return;
                }
                let rec = self
                    .rec
                    .as_ref()
                    .map(|t| t.settings)
                    .unwrap_or_default()
                    .flatten_recording(input);
                let hit = FromUi::SampleHit {
                    kit: f.kit,
                    pad: f.pad,
                    velocity: f.velocity,
                    down: true,
                };
                self.take = Some(Take {
                    rec,
                    kit: f.kit,
                    pad: f.target,
                    pick: record::Pick::Stereo,
                    bars: false,
                    pad_take: false,
                });
                self.hits.push(hit);
                if let Some(f) = self.flatten.as_mut() {
                    f.played = Some(0.0);
                }
                self.refresh_rec_state(ui);
            }
            Some(t) => {
                let t = t + dt;
                let lift = (!f.lifted && t >= f.hold).then(|| flatten_lift(f));
                let silent = self
                    .take
                    .as_ref()
                    .is_some_and(|tk| tk.rec.progress() == Progress::Waiting);
                let give_up = silent && t > f.hold + FLATTEN_WAIT;
                if let Some(f) = self.flatten.as_mut() {
                    f.played = Some(t);
                    f.lifted |= lift.is_some();
                }
                self.hits.extend(lift);
                if give_up {
                    self.end_flatten(ui, "stopped: the pad made no sound");
                }
            }
        }
    }

    /// The Flatten's take is done: kept on its target as a pad of its own
    /// (its envelopes, filter, tune and pan are in the sound now), keeping
    /// the mute group, chokes and polyphony of a pad it replaces.
    fn keep_flatten(&mut self, ui: &mut Ui) {
        let (Some(t), Some(f)) = (self.take.take(), self.flatten.as_ref()) else {
            return;
        };
        let name = self
            .projects
            .get(f.kit)
            .and_then(|p| p.pad(f.pad))
            .map_or_else(|| "Flat".to_string(), |p| format!("{} flat", p.name));
        let (kit, pad) = (f.kit, t.pad);
        match t.rec.finish(&name, t.pick) {
            Some(Ok(src)) => {
                let (frames, rate) = (src.frames(), src.rate());
                let put = self.projects.get_mut(kit).map(|p| {
                    p.put_source_as(pad, src, |name, layer, old| {
                        let mut new = crate::sampler::kit_pad_with(name, layer);
                        if let Some(old) = old {
                            new.mute_group = old.mute_group;
                            new.chokes = old.chokes.clone();
                            new.polyphony = old.polyphony;
                        }
                        new
                    })
                });
                match put {
                    Some(Ok(_)) => {
                        self.edits += 1;
                        if let Some(d) = self.dirty.get_mut(kit) {
                            *d = true;
                        }
                        self.mark_renamed(kit);
                        let text = format!(
                            "{name}, {} on pad {}",
                            duration_text(frames, rate),
                            self.pad_text(kit, pad)
                        );
                        self.last_take = text.clone();
                        self.end_flatten(ui, &format!("kept: {text}"));
                        if kit == self.kit {
                            self.lift();
                            self.pad = pad;
                            self.velocity = None;
                        }
                        self.refresh(ui);
                    }
                    Some(Err(e)) => self.end_flatten(ui, &format!("not kept ({e:?})")),
                    None => self.end_flatten(ui, "not kept (the kit is gone)"),
                }
            }
            Some(Err(e)) => self.end_flatten(ui, &format!("not kept ({e:?})")),
            None => self.end_flatten(ui, "stopped: nothing kept"),
        }
    }

    /// Ends the Flatten (`how` goes in its log line): the pad is let go if
    /// it is still held, a take still going is dropped, and the Rec tab's
    /// own source comes back.
    fn end_flatten(&mut self, ui: &mut Ui, how: &str) {
        let Some(f) = self.flatten.take() else {
            return;
        };
        if f.played.is_some() && !f.lifted {
            self.hits.push(flatten_lift(&f));
        }
        self.take = None;
        self.lines.push(format!(
            "sample: flatten {} {how}",
            self.pad_text(f.kit, f.pad)
        ));
        self.source = f.was;
        self.sync_input(ui);
    }

    /// Hits and lifts Flatten asks the audio thread for, once.
    pub fn take_hits(&mut self) -> Vec<FromUi> {
        std::mem::take(&mut self.hits)
    }

    // ------------------------------------------------------------ auto

    /// What the Auto-sample mode shows besides its settings: the Synth
    /// tab's patch, where the instrument would go and how far it has got.
    fn auto_state_now(&self) -> AutoState {
        let project = self.projects.get(self.kit);
        let has = |p: usize| {
            project
                .and_then(|pr| pr.pad(p))
                .is_some_and(|pd| !pd.layers.is_empty())
        };
        let settings = self
            .auto
            .as_ref()
            .map_or_else(AutoSettings::default, |t| t.settings);
        // With the synth on the pads there is no kit to put it on.
        let pads = if project.is_some() { 16 } else { 0 };
        AutoState {
            pad: self.pad,
            synth: self.auto_synth.clone(),
            pads,
            target: auto_tab::target(settings.onto, settings.notes(), pads, has),
            running: self.auto_job.as_ref().map(AutoJob::running),
            gear: self.gear_out.as_ref().map(|g| g.name.clone()),
            last: self.auto_last.clone(),
            rate: self.rate,
        }
    }

    fn refresh_auto(&mut self, ui: &mut Ui, bank: Option<char>) {
        let st = self.auto_state_now();
        match self.auto.as_mut() {
            Some(tab) => {
                tab.bank = bank;
                tab.update(ui, &st);
            }
            None => match AutoTab::add(ui, AUTO_PAGE, &st) {
                Ok(mut tab) => {
                    tab.bank = bank;
                    tab.update(ui, &st);
                    self.auto = Some(tab);
                }
                Err(e) => {
                    self.lines
                        .push(format!("sample: no Auto-sample page ({e})"));
                    return;
                }
            },
        }
        self.auto_state = Some(st);
    }

    /// The Auto-sample mode's state again (the settings changed, or the
    /// job moved on).
    fn refresh_auto_state(&mut self, ui: &mut Ui) {
        if self.auto_state.is_none() {
            return;
        }
        let st = self.auto_state_now();
        if let Some(tab) = self.auto.as_mut() {
            tab.update(ui, &st);
        }
        self.auto_state = Some(st);
    }

    fn act_auto(&mut self, ui: &mut Ui, a: AutoAction) {
        match a {
            AutoAction::Tab(i) => self.open(ui, i),
            AutoAction::Back => {
                self.editor = Editor::Rec;
                self.refresh(ui);
            }
            AutoAction::Start(plan) => {
                if self.auto_job.is_none() {
                    self.auto_request = Some(plan);
                }
            }
            AutoAction::StartGear { plan, channel } => self.start_gear(ui, plan, channel),
            AutoAction::Cancel => {
                if let Some(j) = self.auto_job.as_ref() {
                    j.job.cancel();
                    self.lines
                        .push("sample: auto-sample cancelled, keeping the notes done".to_string());
                }
            }
        }
    }

    /// The Synth tab's patch, by name ("Poly Pluck"), for the Auto-sample
    /// mode; None before the synth is up.
    pub fn set_auto_synth(&mut self, ui: &mut Ui, name: Option<String>) {
        if name == self.auto_synth {
            return;
        }
        self.auto_synth = name;
        self.refresh_auto_state(ui);
    }

    /// A plan Start asked for. The app makes a fresh copy of the Synth
    /// tab's patch and hands it to [`SamplePage::start_auto`].
    pub fn take_auto_request(&mut self) -> Option<Plan> {
        self.auto_request.take()
    }

    /// Starts auto-sampling `node` (None: the app had no patch to copy)
    /// with `plan`, for the kit the pads play now.
    pub fn start_auto(&mut self, ui: &mut Ui, plan: Plan, node: Option<Box<dyn Node>>) {
        if self.auto_job.is_some() {
            return;
        }
        let Some(node) = node else {
            self.auto_last = "Not started: the synth's patch couldn't be copied".to_string();
            self.lines.push(format!(
                "sample: auto-sample of {} not started (no copy of the patch)",
                plan.name
            ));
            self.refresh_auto_state(ui);
            return;
        };
        self.begin_auto(ui, plan, String::new(), |plan| Job::start(plan, node));
    }

    /// The MIDI out external gear is auto-sampled through (None: there is
    /// none, and the Auto-sample mode offers the synth alone). A gear
    /// auto-sample already running keeps the one it started with.
    #[allow(dead_code)] // the app's MIDI out wiring calls it, after the MIDI out check
    pub fn set_gear(&mut self, ui: &mut Ui, out: Option<GearOut>) {
        self.lines.push(match &out {
            Some(o) => format!("sample: auto-sample can play outside gear on {}", o.name),
            None => {
                "sample: auto-sample can't play outside gear now (no MIDI out open and sending)"
                    .to_string()
            }
        });
        self.gear_out = out;
        self.refresh_auto_state(ui);
    }

    /// Starts auto-sampling external gear in real time: `plan`'s notes on
    /// MIDI `channel` (0 to 15) through the gear's MIDI out, its sound
    /// from the line input, for the kit the pads play now.
    fn start_gear(&mut self, ui: &mut Ui, plan: Plan, channel: u8) {
        if self.auto_job.is_some() {
            return;
        }
        let why = match (&self.gear_out, &self.input) {
            (None, _) => Some("there is no MIDI out for the gear".to_string()),
            _ if self.source != RecSource::Line => Some(format!(
                "the Rec tab records {}, not the line input",
                self.source_name()
            )),
            (_, None) => Some("the line input is off".to_string()),
            _ => None,
        };
        if let Some(why) = why {
            self.auto_last = format!("Not started: {why}");
            self.lines.push(format!(
                "sample: auto-sample of the gear not started ({why})"
            ));
            self.refresh_auto_state(ui);
            return;
        }
        let (Some(out), Some(input)) = (self.gear_out.clone(), self.input.as_ref()) else {
            return;
        };
        let gear = Gear::new(input.audio.clone(), channel);
        let how = format!(" (MIDI channel {} on {})", channel + 1, out.name);
        let send = out.send.clone();
        self.begin_auto(ui, plan, how, move |plan| {
            // The job lets go of the gear with All Notes Off; the MIDI out
            // takes notes only, and no bytes as the end of a job, which
            // ends every note the job left on.
            Job::start_gear(
                plan,
                gear,
                Box::new(move |m: [u8; 3]| {
                    if m[0] & 0xf0 == 0xb0 && m[1] == 123 {
                        send(&[]);
                    } else {
                        send(&m[..]);
                    }
                }),
            )
        });
        if let Some(j) = self.auto_job.as_mut() {
            j.gear = true;
            j.end = Some(out.send);
        }
    }

    /// Starts an auto-sample job with `start` and says so (`how`: what
    /// plays it, after the instrument's name), or says why not.
    fn begin_auto(
        &mut self,
        ui: &mut Ui,
        plan: Plan,
        how: String,
        start: impl FnOnce(Plan) -> Result<Job, PlanError>,
    ) {
        let onto = self
            .auto
            .as_ref()
            .map_or(Onto::EmptyPads, |t| t.settings.onto);
        let notes = plan.notes();
        let per_note = plan.velocities.len() * usize::from(plan.round_robins);
        let est = plan.estimate();
        if let Err(why) = crate::memory::room_in((self.mem_free)(), est.bytes_most) {
            self.auto_last = format!("Not started: {why}");
            self.lines.push(format!(
                "sample: auto-sample of {} not started ({why})",
                plan.name
            ));
            self.told.push(format!("Auto-sample not started: {why}"));
            self.refresh_auto_state(ui);
            return;
        }
        let text = format!(
            "sample: auto-sample {}{how} started: {} notes ({} to {}), {} takes, {:.0} s of notes, up to {:.0} MB",
            plan.name,
            notes.len(),
            mpfree_sampler_ui::note_name(notes.first().copied().unwrap_or(plan.lo)),
            mpfree_sampler_ui::note_name(notes.last().copied().unwrap_or(plan.hi)),
            est.takes,
            est.seconds,
            est.bytes_most as f64 / 1e6
        );
        match start(plan) {
            Ok(job) => {
                self.lines.push(text);
                self.auto_last.clear();
                self.auto_job = Some(AutoJob {
                    job,
                    kit: self.kit,
                    onto,
                    gear: false,
                    notes,
                    per_note,
                    end: None,
                });
            }
            Err(e) => {
                self.auto_last = format!("Not started: {e}");
                self.lines
                    .push(format!("sample: auto-sample not started ({e})"));
            }
        }
        self.refresh_auto_state(ui);
    }

    /// Whether an auto-sample runs.
    #[cfg(test)]
    pub fn auto_running(&self) -> bool {
        self.auto_job.is_some()
    }

    /// Moves the Auto-sample mode's progress on while it `showing`, and
    /// lands the instrument once the job is done.
    fn poll_auto(&mut self, ui: &mut Ui, showing: bool) {
        let Some(j) = self.auto_job.as_ref() else {
            return;
        };
        if !j.job.is_finished() {
            let now = Some(j.running());
            if showing
                && self.editor == Editor::Auto
                && self.auto_state.as_ref().is_some_and(|st| st.running != now)
            {
                self.refresh_auto_state(ui);
            }
            return;
        }
        let Some(j) = self.auto_job.take() else {
            return;
        };
        let (kit, onto, gear) = (j.kit, j.onto, j.gear);
        let end = j.end.clone();
        let f = j.job.finish();
        if let Some(end) = end {
            end(&[]);
        }
        self.land_auto(ui, kit, onto, gear, f);
    }

    /// Puts a finished auto-sample's instrument on kit `kit` (one undo
    /// step) and picks its first pad, or says why not.
    fn land_auto(&mut self, ui: &mut Ui, kit: usize, onto: Onto, gear: bool, f: Finished) {
        let clipped = if f.clipped { ", it clipped (kept)" } else { "" };
        let what = if gear {
            format!(
                "{:.1} s of notes recorded, built in {:.2} s{clipped}",
                f.audio_seconds, f.build_seconds
            )
        } else {
            format!(
                "{:.1} s of notes rendered in {:.2} s, built in {:.2} s{clipped}",
                f.audio_seconds, f.render_seconds, f.build_seconds
            )
        };
        // Something other than Cancel stopped the gear: said, and the notes
        // done are kept.
        if let Some(why) = f.stopped {
            self.lines.push(format!(
                "sample: auto-sample stopped: {why}; keeping the notes done"
            ));
        }
        let inst = match f.instrument {
            Ok(i) => i,
            Err(e) => {
                self.auto_last = match f.stopped {
                    Some(why) => format!("Stopped: {why}"),
                    None => format!("Nothing made: {e}"),
                };
                self.lines
                    .push(format!("sample: auto-sample made nothing ({e}; {what})"));
                self.refresh_auto_state(ui);
                return;
            }
        };
        let n = inst.pads.len();
        let Some(project) = self.projects.get_mut(kit) else {
            self.auto_last = "Not kept: the kit is gone".to_string();
            self.lines
                .push("sample: auto-sample not kept (the kit is gone)".to_string());
            self.refresh_auto_state(ui);
            return;
        };
        let to = auto_tab::target(onto, n, 16, |p| {
            project.pad(p).is_some_and(|pd| !pd.layers.is_empty())
        });
        let Some(to) = to else {
            self.auto_last = format!("Not kept: no {n} empty pads in a row now");
            self.lines.push(format!(
                "sample: auto-sample not kept (no {n} empty pads in a row now; {what})"
            ));
            self.refresh_auto_state(ui);
            return;
        };
        let (name, takes, looped, gain, mb) = (
            inst.name.clone(),
            inst.sources.len(),
            inst.looped,
            inst.gain_db,
            inst.bytes() as f64 / 1e6,
        );
        let cancelled = f.state == mpfree_sampler::autosample::State::Cancelled;
        match inst.put(project, to) {
            Ok(()) => {
                self.edits += 1;
                if let Some(d) = self.dirty.get_mut(kit) {
                    *d = true;
                }
                self.mark_renamed(kit);
                let pads = if n == 1 {
                    self.pad_text(kit, to)
                } else {
                    format!(
                        "{} to {}",
                        self.pad_text(kit, to),
                        self.pad_text(kit, to + n - 1)
                    )
                };
                self.auto_last = format!(
                    "{name}: {n} notes, {takes} takes on pads {pads}{}{} · {mb:.1} MB",
                    if looped > 0 {
                        format!(", {looped} looped")
                    } else {
                        String::new()
                    },
                    if cancelled {
                        " (cancelled: the notes done)"
                    } else {
                        ""
                    },
                );
                self.lines.push(format!(
                    "sample: auto-sampled {name} onto {} pads {pads}: {n} notes, {takes} takes, {looped} looped, \
                     {gain:+.1} dB, {mb:.1} MB ({what}); the pads play notes",
                    self.kit_text_of(kit)
                ));
                if kit == self.kit {
                    self.lift();
                    self.pad = to;
                    self.velocity = None;
                }
                self.refresh(ui);
            }
            Err(e) => {
                self.auto_last = format!("Not kept: {e:?}");
                self.lines
                    .push(format!("sample: auto-sample not kept ({e:?})"));
                self.refresh_auto_state(ui);
            }
        }
    }

    /// About 30 times a second: polls the take (when it ends it lands on
    /// its pad) and, while the Rec tab `showing`, moves its picture `dt`
    /// seconds on.
    pub fn tick(&mut self, ui: &mut Ui, dt: f32, showing: bool) {
        self.poll_take(ui);
        self.watch_pad_run();
        self.poll_flatten(ui, dt);
        if showing && self.editor == Editor::Rec && self.rec_state.is_some() {
            self.feed_rec(ui, dt);
        }
        self.poll_auto(ui, showing);
        self.poll_process(ui, showing);
        self.poll_open(ui);
        self.poll_kit(ui);
        self.poll_load(ui);
        // A sound's view came in from the worker: the page shows it (now,
        // or when it is next shown).
        if self.sources.take_done() {
            if showing && self.state.is_some() {
                self.refresh(ui);
            } else {
                self.stale = true;
            }
        }
    }

    /// The take running: what came in since the last poll, and it lands
    /// on its pad once done.
    fn poll_take(&mut self, ui: &mut Ui) {
        if let (Some(t), Some(input)) = (self.take.as_mut(), self.input.as_ref()) {
            match t.rec.poll(input) {
                Ok(Progress::Done { .. }) if self.flatten.is_some() => self.keep_flatten(ui),
                Ok(Progress::Done { .. }) => {
                    if let Some(t) = self.take.take() {
                        let name = self.take_name();
                        let what = take_what(&t.rec);
                        let bpm = t.bars.then(|| self.tempo());
                        match t.rec.finish(&name, t.pick) {
                            Some(Ok(src)) => self.keep_warped(ui, t.kit, t.pad, src, &what, bpm),
                            Some(Err(e)) => {
                                self.lines.push(format!("sample: take not kept ({e:?})"))
                            }
                            None => {}
                        }
                    }
                }
                Ok(Progress::Nothing) => {
                    self.take = None;
                    self.lines
                        .push("sample: take cancelled before it started".to_string());
                }
                Ok(_) => {}
                Err(e) => {
                    self.take = None;
                    self.lines.push(format!("sample: take lost ({e:?})"));
                }
            }
        }
    }

    /// Whether a sound's view (its peaks, hits, tempo or note) is still
    /// being worked out off the screen thread.
    #[allow(dead_code)] // the tests wait on it
    pub fn analysing(&self) -> bool {
        self.sources.busy()
    }

    /// The line input turned on or off (its row on the Looper tab's MENU).
    /// While the Rec tab records the line input: on, the sampler's input is
    /// made (once) for the audio thread; off, a take still recording ends
    /// and is kept, and the input is freed.
    pub fn set_line_input(&mut self, ui: &mut Ui, on: bool) {
        if on == self.line_on {
            return;
        }
        self.line_on = on;
        self.sync_input(ui);
    }

    /// Makes the sampler's input when its source wants it, or frees it
    /// when not (a take still recording ends there and is kept).
    fn sync_input(&mut self, ui: &mut Ui) {
        if self.input_wanted() {
            if self.input.is_none() {
                let (feed, input) = record::input(INPUT_SECONDS, self.rate);
                self.input = Some(input);
                self.feed = Some(Box::new(RecFeed(feed)));
                let what = match self.source {
                    RecSource::Line => "the line input feeds the sampler".to_string(),
                    RecSource::Mix(..) => format!("the sampler resamples {}", self.source_name()),
                };
                self.lines.push(format!(
                    "sample: {what} ({INPUT_SECONDS} s kept, {:.1} MB)",
                    input_bytes(self.rate) as f64 / 1e6
                ));
            }
        } else if self.input.is_some() || self.feed.is_some() {
            if let (Some(t), Some(input)) = (self.take.as_mut(), self.input.as_ref()) {
                t.rec.stop(input);
            }
            self.tick(ui, 0.0, false);
            self.take = None;
            self.input = None;
            // Not sent yet: freed here and now. Else the audio thread hands
            // it back first.
            if self.feed.take().is_none() && self.feeding {
                self.want_back = true;
            }
        }
        self.refresh_rec_state(ui);
        self.feed_rec(ui, 0.0);
    }

    /// What the input records from, when the audio thread hasn't been told;
    /// else the input's writing end, for the audio thread, once; or the ask
    /// to hand it back, once its source is gone.
    pub fn take_input(&mut self) -> Option<FromUi> {
        if self.source_sent != Some(self.source) {
            self.source_sent = Some(self.source);
            return Some(FromUi::SampleSource(self.source));
        }
        if let Some(f) = self.feed.take() {
            self.feeding = true;
            return Some(FromUi::SampleInput(Some(f)));
        }
        if std::mem::take(&mut self.want_back) {
            return Some(FromUi::SampleInput(None));
        }
        None
    }

    /// [`SamplePage::take_input`]'s message found no room: again later.
    pub fn input_unsent(&mut self, f: Option<Box<RecFeed>>) {
        match f {
            // Still wanted if the input it writes is still here.
            Some(f) if self.input.is_some() => {
                self.feeding = false;
                self.feed = Some(f);
            }
            Some(_) => self.feeding = false,
            None => self.want_back = true,
        }
    }

    /// The audio thread handed the writing end back: the buffer is freed
    /// here (with the input, already dropped, it was the last owner).
    pub fn input_back(&mut self, f: Box<RecFeed>) {
        let frames = f.written();
        drop(f);
        if self.feed.is_none() && self.input.is_none() {
            self.feeding = false;
        }
        self.lines.push(format!(
            "sample: the sampler's input buffer was freed ({:.1} s had come in)",
            frames as f64 / f64::from(self.rate.max(1))
        ));
    }

    // ------------------------------------------------------------ warp

    /// The project tempo (the transport's). Kits with warped pads get new
    /// views, and their copies for the old tempo stop being made.
    pub fn set_tempo(&mut self, bpm: f64) {
        if !self.warps.set_tempo(bpm) {
            return;
        }
        self.cancel_stretching();
        for k in 0..self.projects.len() {
            if self.warped(k) {
                self.dirty[k] = true;
            }
        }
    }

    /// The project tempo (the transport's).
    pub fn tempo(&self) -> f64 {
        self.warps.tempo()
    }

    /// The Rec tab's settings, once it has been opened.
    #[cfg(test)]
    pub fn rec_settings(&self) -> Option<rec_tab::RecSettings> {
        self.rec.as_ref().map(|t| t.settings)
    }

    /// Whether kit `kit` has a warped pad.
    fn warped(&self, kit: usize) -> bool {
        self.projects.get(kit).is_some_and(|p| {
            (0..mpfree_sampler::model::PADS).any(|i| {
                p.pad(i)
                    .is_some_and(|pad| pad.layers.iter().any(|l| l.region.warp.is_some()))
            })
        })
    }

    fn cancel_stretching(&mut self) {
        if let Some(w) = &self.stretcher {
            w.cancel();
        }
        self.stretching.clear();
        self.unstretched.clear();
    }

    /// Copies kit `kit` needs at the tempo go to the stretch thread.
    fn ask_for_copies(&mut self, kit: usize) {
        let Some(project) = self.projects.get(kit) else {
            return;
        };
        let missing: Vec<CopyKey> = self
            .warps
            .missing(project)
            .into_iter()
            .filter(|k| !self.stretching.contains(k) && !self.unstretched.contains(k))
            .collect();
        if missing.is_empty() {
            return;
        }
        if self.stretcher.is_none() {
            match Worker::spawn() {
                Ok(w) => self.stretcher = Some(w),
                Err(e) => {
                    self.lines.push(format!("sample: no stretch thread: {e}"));
                    return;
                }
            }
        }
        let Some(w) = &self.stretcher else { return };
        for k in missing {
            // The copy, and the source's part the thread is handed.
            let ch = project
                .source(k.source)
                .map_or(2, |s| u64::from(s.channels()));
            let need = (k.frames + k.end.saturating_sub(k.start)) * ch * 4;
            if let Err(why) = crate::memory::room_in((self.mem_free)(), need) {
                self.unstretched.push(k);
                self.lines.push(format!(
                    "sample: not stretched, the pad repitches instead ({why})"
                ));
                self.told
                    .push(format!("Not stretched, the pad repitches instead: {why}"));
                continue;
            }
            match Warps::spec(project, &k) {
                Ok(spec) => {
                    if w.send(k, spec) {
                        self.stretching.push(k);
                    }
                }
                Err(e) => self.lines.push(format!("sample: can't stretch: {e}")),
            }
        }
    }

    /// Takes in the copies the stretch thread has finished; the kits that
    /// play them get new views.
    fn take_copies(&mut self) {
        let Some(w) = &self.stretcher else { return };
        let mut done = Vec::new();
        while let Some(d) = w.try_recv() {
            done.push(d);
        }
        for d in done {
            match d {
                Done::Copy(key, data, took) => {
                    self.stretching.retain(|k| *k != key);
                    self.warps.insert_for(key, data, &self.projects);
                    for k in 0..self.projects.len() {
                        if self.warps.wanted(&self.projects[k]).contains(&key) {
                            self.dirty[k] = true;
                        }
                    }
                    self.lines.push(format!(
                        "sample: stretched {:.2} s to {:.0} BPM in {:.0} ms ({:.1} MB of copies)",
                        key.frames as f64 / f64::from(self.rate.max(1)),
                        self.warps.tempo(),
                        took.as_secs_f64() * 1000.0,
                        self.warps.bytes() as f64 / 1e6
                    ));
                }
                Done::Failed(key, e) => {
                    self.stretching.retain(|k| *k != key);
                    self.lines.push(format!("sample: stretch failed: {e}"));
                }
                Done::Cancelled(key) => self.stretching.retain(|k| *k != key),
            }
        }
    }

    // ------------------------------------------------------------ the audio thread

    /// The next kit view for the audio thread, if a kit was edited (or a
    /// stretched copy came in) and no view is on its way already. Built
    /// here, off the audio thread, with warped pads at the project tempo.
    /// The scene strip ([`scene`]): store A or B, move the crossfader,
    /// or go back to A.
    fn scene(&mut self, ui: &mut Ui, a: SceneAction, out: &mut Vec<FromUi>) {
        self.lift();
        let kit = self.kit;
        let name = self.kit_text_of(kit);
        let Some(project) = self.projects.get_mut(kit) else {
            return;
        };
        match a {
            SceneAction::Store(k) if k < 2 => {
                let stored = scene::Scene::of(project);
                let n = stored.sounds();
                project.scenes[k] = Some(stored);
                self.edits += 1;
                // Saved with the kit; the new view puts the crossfader's
                // values back (take_view).
                if let Some(d) = self.dirty.get_mut(kit) {
                    *d = true;
                }
                let what = format!(
                    "{} stored ({n} pad{})",
                    ["A", "B"][k],
                    if n == 1 { "" } else { "s" }
                );
                // On the hint line too: storing again changes nothing else
                // on the screen.
                self.told.push(format!("Scene {what}"));
                self.lines.push(format!("sample: {name}: scene {what}"));
            }
            SceneAction::Store(_) => {}
            SceneAction::Fade(x) => {
                if let [Some(sa), Some(sb)] = &project.scenes {
                    out.push(FromUi::SamplePadLive {
                        kit,
                        values: scene::morph(sa, sb, x),
                    });
                    self.morph = Some((kit, x));
                }
            }
            SceneAction::Return => {
                let Some(sa) = project.scenes[0].clone() else {
                    return;
                };
                if let Some(sb) = &project.scenes[1] {
                    out.push(FromUi::SamplePadLive {
                        kit,
                        values: scene::clear(&sa, sb),
                    });
                }
                self.morph = None;
                let (n, left) = project.return_to(&sa);
                if n > 0 {
                    self.edits += 1;
                    if let Some(d) = self.dirty.get_mut(kit) {
                        *d = true;
                    }
                    self.mark_renamed(kit);
                }
                let left = match left {
                    0 => String::new(),
                    n => format!("; {n} left as they are (their sounds are gone)"),
                };
                self.lines.push(match n {
                    0 => format!("sample: {name}: back to scene A (nothing had changed){left}"),
                    1 => format!("sample: {name}: back to scene A (1 pad put back){left}"),
                    n => format!("sample: {name}: back to scene A ({n} pads put back){left}"),
                });
            }
        }
        self.refresh(ui);
    }

    pub fn take_view(&mut self) -> Option<FromUi> {
        self.take_copies();
        if self.in_flight {
            return None;
        }
        let kit = self.dirty.iter().position(|d| *d)?;
        self.dirty[kit] = false;
        self.in_flight = true;
        self.views_sent += 1;
        let view = EngineView::build_warped(&self.projects[kit], self.rate, &mut self.warps);
        self.ask_for_copies(kit);
        // The new view clears the crossfader's values: they follow it.
        if let Some((k, x)) = self.morph.filter(|m| m.0 == kit) {
            if let [Some(sa), Some(sb)] = &self.projects[k].scenes {
                self.hits.push(FromUi::SamplePadLive {
                    kit,
                    values: scene::morph(sa, sb, x),
                });
            }
        }
        Some(FromUi::SampleView(Box::new(ViewSwap {
            kit,
            view: Some(view),
        })))
    }

    /// The audio thread is done with a view: the box holds the one it
    /// replaced, freed here.
    pub fn view_back(&mut self, swap: Box<ViewSwap>) {
        self.in_flight = false;
        drop(swap);
    }

    /// Kit `kit`'s view found no room on its way: build it again next time.
    pub fn unsent(&mut self, kit: usize) {
        self.in_flight = false;
        self.views_sent -= 1;
        if let Some(d) = self.dirty.get_mut(kit) {
            *d = true;
        }
    }

    /// Where the playing kit's voices are; the shown pad's are drawn.
    pub fn set_playheads(&mut self, ui: &mut Ui, heads: &Playheads) {
        self.live_heads(ui, heads);
        let frames: Vec<u64> = if usize::from(heads.kit) == self.kit && self.state.is_some() {
            heads.of(self.pad).collect()
        } else {
            Vec::new()
        };
        if frames == self.shown_heads {
            return;
        }
        match self.editor {
            Editor::Wave => {
                if let Some(tab) = self.tab.as_ref() {
                    tab.set_playheads(ui, &frames);
                }
            }
            Editor::Chop => {
                if let Some(tab) = self.chop.as_ref() {
                    tab.set_playheads(ui, &frames);
                }
            }
            Editor::Warp => {
                if let Some(tab) = self.warp.as_ref() {
                    tab.set_playheads(ui, &frames);
                }
            }
            // The Play, Pads and Rec tabs (and Auto-sample) draw no playheads; the Grain
            // tab's come with its cloud.
            Editor::Play
            | Editor::Pads
            | Editor::Grain
            | Editor::Fx
            | Editor::Rec
            | Editor::Auto => {}
        }
        self.shown_heads = frames;
    }

    /// Live chop's cuts and its end, from the playheads: the cuts made
    /// since the last go onto the Chop tab; the sound over (it was heard
    /// marking, and isn't now) ends it.
    fn live_heads(&mut self, ui: &mut Ui, heads: &Playheads) {
        self.marks_seen = heads.marks_made;
        let Some(run) = self.live.as_mut() else {
            return;
        };
        if usize::from(heads.kit) != run.kit {
            return;
        }
        let marks: Vec<u64> = (run.seen..heads.marks_made)
            .filter_map(|i| heads.mark(i))
            .collect();
        run.seen = heads.marks_made;
        run.heard |= heads.marking;
        let ended = run.heard && !heads.marking;
        if let (Some(tab), Some(st)) = (self.chop.as_mut(), self.chop_state.as_ref()) {
            if self.editor == Editor::Chop {
                run.kept += tab.live_marks(ui, st, &marks);
                if ended {
                    tab.live_ended(ui, st);
                }
            }
        }
        self.keep_chop();
        if ended {
            self.stop_live("over: the sound ended");
        }
    }

    /// Whether a change sent nowhere (a chop kept) left the work unsaved
    /// since the last call.
    pub fn take_unsaved(&mut self) -> bool {
        std::mem::take(&mut self.unsaved)
    }

    /// The cloud's box, naming the Grain tab's pad and layer, for the audio
    /// thread to fill; None unless the Grain tab shows and the box is here.
    pub fn take_cloud(&mut self) -> Option<FromUi> {
        if self.page() != GRAIN_PAGE {
            return None;
        }
        let st = self.grain_state.as_ref()?;
        let mut c = self.cloud.take()?;
        c.kit = self.kit;
        c.pad = st.pad.min(usize::from(u8::MAX)) as u8;
        c.layer = st.layer.min(usize::from(u8::MAX)) as u8;
        Some(FromUi::SampleGrains(c))
    }

    /// The box found no room on its way: it waits here for the next time.
    pub fn cloud_unsent(&mut self, c: Box<Cloud>) {
        self.cloud = Some(c);
    }

    /// The audio thread filled the box: the Grain tab draws its grains if
    /// they are still the ones it shows. The box stays here until the next
    /// [`SamplePage::take_cloud`].
    pub fn cloud_back(&mut self, ui: &mut Ui, c: Box<Cloud>) {
        let shows = self.page() == GRAIN_PAGE && c.kit == self.kit;
        if let (true, Some(tab), Some(st)) = (shows, self.grain.as_ref(), self.grain_state.as_ref())
        {
            if usize::from(c.pad) == st.pad && usize::from(c.layer) == st.layer {
                let busy = c.len > 0 || c.head.is_some() || c.live > 0;
                // An idle cloud is drawn once, not 25 times a second.
                if busy || self.cloud_shown {
                    let r = &st.region;
                    let head = c
                        .head
                        .map(|h| ((h as f64 - r.start as f64) / r.frames().max(1) as f64) as f32);
                    let now = CloudNow {
                        playhead: head,
                        state: c.state,
                        live: c.live,
                    };
                    tab.set_cloud(ui, st, c.grains(), now);
                }
                self.cloud_shown = busy;
            }
        }
        self.cloud = Some(c);
    }

    /// Gives the memory free for new loads from `f` (a test's).
    #[cfg(test)]
    pub fn set_mem_free(&mut self, f: fn() -> Option<u64>) {
        self.mem_free = f;
    }

    /// Lines for the log since the last call.
    /// Refusals to show in place (they are in `take_lines` too).
    pub fn take_told(&mut self) -> Vec<String> {
        std::mem::take(&mut self.told)
    }

    pub fn take_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.lines)
    }
}

/// What a Pads tab edit did to a pad, for the log: "now plays Hat closed",
/// "cleared (was Break 92)", "Break 92 -> Kick 58", or the settings that
/// changed. Empty if nothing did.
pub fn describe_slot(before: Option<&Pad>, after: Option<&Pad>) -> String {
    match (before, after) {
        (None, None) => String::new(),
        (None, Some(a)) => format!("now plays {}", a.name),
        (Some(b), None) => format!("cleared (was {})", b.name),
        (Some(b), Some(a))
            if b.layers.first().map(|l| l.region.source)
                != a.layers.first().map(|l| l.region.source) =>
        {
            format!("{} -> {}", b.name, a.name)
        }
        (Some(b), Some(a)) => {
            let mut what = Vec::new();
            if b.layers.first().map(|l| l.region.mode) != a.layers.first().map(|l| l.region.mode) {
                let m = |p: &Pad| p.layers.first().map_or("", |l| mode_label(l.region.mode));
                what.push(format!("mode {} -> {}", m(b), m(a)));
            }
            let settings = play_tab::describe(b, a);
            if !settings.is_empty() {
                what.push(settings);
            }
            if what.is_empty() && b != a {
                what.push(format!("now a copy of {}", a.name));
            }
            what.join(", ")
        }
    }
}

/// What a change to a pad's layers did, for the log: "layer 2 added, a
/// copy of layer 1 (Break 92) on velocities 64-127, layer 1 now 1-63".
fn layers_text(a: &PlayAction, before: Option<&Pad>, project: &Project) -> String {
    let sound = |p: &Pad, k: usize| {
        p.layers
            .get(k)
            .and_then(|l| project.source(l.region.source))
            .map_or_else(String::new, |s| s.name.clone())
    };
    let vels = |p: &Pad, k: usize| {
        p.layers
            .get(k)
            .map_or_else(String::new, |l| format!("{}-{}", l.vel_lo, l.vel_hi))
    };
    let (Some(b), Some(now)) = (
        before,
        project.pad(match a {
            PlayAction::AddLayer { pad, .. }
            | PlayAction::RemoveLayer { pad, .. }
            | PlayAction::LayerSound { pad, .. } => *pad,
            _ => return String::new(),
        }),
    ) else {
        return String::new();
    };
    match a {
        PlayAction::AddLayer { after, .. } => format!(
            "layer {} added, a copy of layer {} ({}) on velocities {}, layer {} now {}",
            after + 2,
            after + 1,
            sound(now, after + 1),
            vels(now, after + 1),
            after + 1,
            vels(now, *after)
        ),
        PlayAction::RemoveLayer { layer, .. } => format!(
            "layer {} removed ({}, velocities {}), {} left",
            layer + 1,
            sound(b, *layer),
            vels(b, *layer),
            match now.layers.len() {
                1 => "1 layer".to_string(),
                n => format!("{n} layers"),
            }
        ),
        PlayAction::LayerSound { layer, .. } => format!(
            "layer {} plays {} (was {}), start to end",
            layer + 1,
            sound(now, *layer),
            sound(b, *layer)
        ),
        _ => String::new(),
    }
}

/// What changed between two versions of a region, for the log: "Start
/// 0.000 s -> 0.023 s, gain 0.0 -> +2.0 dB". Empty if nothing did.
pub fn describe(a: &Region, b: &Region, rate: u32) -> String {
    let s = |f: u64| format!("{:.3} s", f as f64 / f64::from(rate.max(1)));
    let fade = |f: &Fade| duration_text(u64::from(f.frames), rate);
    let channels = |c: ChannelPick| match c {
        ChannelPick::Stereo => "stereo",
        ChannelPick::MonoSum => "mono sum",
        ChannelPick::Left => "left",
        ChannelPick::Right => "right",
    };
    let mut out = Vec::new();
    if a.mode != b.mode {
        out.push(format!(
            "mode {} -> {}",
            mode_label(a.mode),
            mode_label(b.mode)
        ));
    }
    if a.start != b.start {
        out.push(format!("Start {} -> {}", s(a.start), s(b.start)));
    }
    if a.end != b.end {
        out.push(format!("End {} -> {}", s(a.end), s(b.end)));
    }
    match (a.sustain_loop, b.sustain_loop) {
        (None, Some(l)) => out.push(format!("loop on, {} to {}", s(l.start), s(l.end))),
        (Some(_), None) => out.push("loop off".to_string()),
        (Some(x), Some(y)) => {
            if x.start != y.start {
                out.push(format!("Loop start {} -> {}", s(x.start), s(y.start)));
            }
            if x.end != y.end {
                out.push(format!("Loop end {} -> {}", s(x.end), s(y.end)));
            }
            if x.mode != y.mode {
                let m = |l: LoopMode| {
                    if l == LoopMode::PingPong {
                        "ping-pong"
                    } else {
                        "forward"
                    }
                };
                out.push(format!("loop {} -> {}", m(x.mode), m(y.mode)));
            }
            if x.crossfade != y.crossfade {
                out.push(format!(
                    "crossfade {} -> {}",
                    duration_text(u64::from(x.crossfade), rate),
                    duration_text(u64::from(y.crossfade), rate)
                ));
            }
        }
        (None, None) => {}
    }
    if a.fade_in != b.fade_in {
        out.push(format!(
            "fade in {} -> {}",
            fade(&a.fade_in),
            fade(&b.fade_in)
        ));
    }
    if a.fade_out != b.fade_out {
        out.push(format!(
            "fade out {} -> {}",
            fade(&a.fade_out),
            fade(&b.fade_out)
        ));
    }
    if a.gain_db != b.gain_db {
        out.push(format!("gain {:+.1} -> {:+.1} dB", a.gain_db, b.gain_db));
    }
    if a.tune_cents != b.tune_cents {
        out.push(format!(
            "tune {:+} -> {:+} cents",
            a.tune_cents, b.tune_cents
        ));
    }
    if a.reverse != b.reverse {
        out.push(
            if b.reverse {
                "reverse on"
            } else {
                "reverse off"
            }
            .to_string(),
        );
    }
    if a.channels != b.channels {
        out.push(format!(
            "channels {} -> {}",
            channels(a.channels),
            channels(b.channels)
        ));
    }
    out.join(", ")
}

/// Projects `a` and `b` (not the same) of `all`, `a` to read and `b` to
/// change.
fn two_mut(all: &mut [Project], a: usize, b: usize) -> (&Project, &mut Project) {
    assert_ne!(a, b);
    if a < b {
        let (lo, hi) = all.split_at_mut(b);
        (&lo[a], &mut hi[0])
    } else {
        let (lo, hi) = all.split_at_mut(a);
        (&hi[0], &mut lo[b])
    }
}

/// A phrase as a clip of the time engine, at [`PPQ`]: slice `k` is a note
/// for pad `first_pad + k` at full velocity, the clip as long as the
/// phrase's whole beats. A track's clips play any of its kit's 128 pads.
///
/// On one sliced pad (`one_pad`, R4), every hit is a note on `first_pad`
/// with a Slice lock on its step, so each plays its own slice.
fn phrase_clip(name: &str, p: &Phrase, first_pad: usize, one_pad: bool) -> Result<Clip, String> {
    let ppq = i64::from(PPQ);
    let mut clip = Clip::new(&format!("{name} phrase"), i64::from(p.beats) * ppq);
    for h in &p.hits {
        let pad = if one_pad {
            first_pad
        } else {
            first_pad + h.slice
        };
        let note = u8::try_from(pad)
            .ok()
            .filter(|&n| usize::from(n) < PADS)
            .ok_or("past the kit's last pad")?;
        let at = i64::try_from(h.tick(PPQ)).map_err(|_| "too long")?;
        let len = i64::try_from(h.ticks(PPQ)).map_err(|_| "too long")?;
        clip.add_note(Note::new(at, len, note, 127))
            .map_err(|e| format!("{e:?}"))?;
        if one_pad {
            let slice = RelAddr::new(OnTrack::Pad(note), PadField::Slice as u16);
            clip.add_lock(Lock::new(at, Some(note), slice, h.slice as f32))
                .map_err(|e| format!("{e:?}"))?;
        }
    }
    clip.check().map_err(|e| format!("{e:?}"))?;
    Ok(clip)
}

/// A sound file read and decoded (and a note about it), or why not.
pub type Loaded = Result<(Source, Option<String>), String>;

/// A sound read for the browser's Load, a note when something was left
/// out of it (a file cut short), and the pad its sidecar describes when
/// one is beside it ([`export`]).
struct LoadedSound {
    src: Source,
    note: Option<String>,
    sidecar: Option<Result<export::Opened, String>>,
    /// The sidecar's file name, for the log.
    sidecar_name: String,
}

/// A Load's sound with its sidecar, or why not.
type LoadedPad = Result<LoadedSound, String>;

/// Reads and decodes `path` on a thread of its own; the answer comes on
/// the channel (the browser's preview). Refused, as a Load is, when it
/// would leave less than the memory MPFree keeps free.
pub fn read_in_thread(path: PathBuf) -> std::io::Result<Receiver<Loaded>> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("preview-load".into())
        .spawn(move || {
            let _ = crate::cores::background(); // off the sound's cores and priority
            let _ = tx.send(read_one(&path, crate::memory::free_bytes));
        })?;
    Ok(rx)
}

/// A sound file for the browser's Load: WAV, AIFF or FLAC, known by what
/// it holds, no bigger than the sampler holds in memory; with its sidecar
/// (`Break 92.mpfree` beside `Break 92.wav`) read too, and the other
/// sounds it names beside it. Every sound at another rate than `rate`
/// (the output's) is converted to it ([`at_rate`]).
#[allow(dead_code)] // reached through load_file
fn read_sound(path: &Path, free: fn() -> Option<u64>, rate: u32) -> LoadedPad {
    let (src, note) = read_one(path, free)?;
    let side = path.with_extension(export::SIDECAR);
    let sidecar_name = side
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sidecar = side.is_file().then(|| {
        let text = std::fs::read_to_string(&side).map_err(|e| e.to_string())?;
        let own = path.file_name().map(|s| s.to_string_lossy().into_owned());
        let dir = path.parent().unwrap_or(Path::new("."));
        export::read_sidecar(&text, &mut |name| {
            if own.as_deref() == Some(name) {
                return Ok(src.clone());
            }
            // Only sounds beside it.
            if name.is_empty() || name.contains(['/', '\\']) || name == ".." {
                return Err("not beside it".to_string());
            }
            read_one(&dir.join(name), free).map(|(s, _)| s)
        })
    });
    Ok(at_rate(
        LoadedSound {
            src,
            note,
            sidecar,
            sidecar_name,
        },
        rate,
        free,
    ))
}

/// Sounds put at the output's rate as they load or open
/// (`Source::at_rate`): each sound once, so kits that share a sound share
/// its converted copy too, and only while the copy leaves the memory
/// MPFree keeps free; a sound without room stays at its own rate (it
/// still plays at its pitch) and is listed.
struct ToRate {
    rate: u32,
    free: fn() -> Option<u64>,
    made: HashMap<SourceId, (Source, model::Rescale)>,
    /// Sounds kept at their own rate for want of memory, and their names
    /// and rates.
    skipped: HashSet<SourceId>,
    kept: Vec<(String, u32)>,
}

impl ToRate {
    fn new(rate: u32, free: fn() -> Option<u64>) -> ToRate {
        ToRate {
            rate,
            free,
            made: HashMap::new(),
            skipped: HashSet::new(),
            kept: Vec::new(),
        }
    }

    fn convert(&mut self, s: &Source) -> Option<(Source, model::Rescale)> {
        if self.rate == 0 || s.rate() == self.rate || self.skipped.contains(&s.id()) {
            return None;
        }
        if let Some(m) = self.made.get(&s.id()) {
            return Some(m.clone());
        }
        // The new sound twice over: made, then moved into place; and a
        // 16-bit sound as floats, which the conversion reads.
        let frames = s.frames() as f64 * f64::from(self.rate) / f64::from(s.rate());
        let floats = if s.data().is_16_bit() {
            s.data().len() as u64 * 4
        } else {
            0
        };
        let need = (frames.ceil() as u64) * u64::from(s.channels()) * 4 * 2 + floats;
        if crate::memory::room_in((self.free)(), need).is_err() {
            self.skipped.insert(s.id());
            self.kept.push((s.name.clone(), s.rate()));
            return None;
        }
        let m = s.at_rate(self.rate).ok()?;
        self.made.insert(s.id(), m.clone());
        Some(m)
    }

    /// The log's word on the sounds kept at their own rate, if any.
    fn kept_note(&self) -> Option<String> {
        (!self.kept.is_empty()).then(|| {
            let each: Vec<String> = self
                .kept
                .iter()
                .map(|(n, r)| format!("{n} at {}", khz(*r)))
                .collect();
            format!(
                "kept at its own rate, {}: {}",
                crate::memory::SHORT,
                each.join(", ")
            )
        })
    }
}

/// A loaded sound, and the sounds its sidecar brings, at `rate`: each one
/// at another rate is converted ([`ToRate`]), and the sidecar's pad and
/// slices move with it, so the sound plays at its own pitch through the
/// sampler's plain path instead of being read faster or slower for ever.
/// The note says what was converted. Done after the sidecar is read,
/// since its frames are on the sounds as they were.
fn at_rate(mut l: LoadedSound, rate: u32, free: fn() -> Option<u64>) -> LoadedSound {
    let mut to = ToRate::new(rate, free);
    let mut moved = Vec::new();
    let mine = l.src.id();
    if let Some(Ok(o)) = &mut l.sidecar {
        let was: Vec<(SourceId, u32)> = o.sources.iter().map(|s| (s.id(), s.rate())).collect();
        for r in model::convert_sounds(
            &mut o.sources,
            std::slice::from_mut(&mut o.pad),
            &mut o.chops,
            &mut |s| to.convert(s),
        ) {
            let old = was.iter().find(|(id, _)| *id == r.from).map(|w| w.1);
            // The sound itself goes first and unnamed (the log line
            // already names it); the others it brings by name.
            let name = if r.from == mine {
                None
            } else {
                o.sources
                    .iter()
                    .find(|s| s.id() == r.to)
                    .map(|s| s.name.clone())
            };
            moved.push((name, old.unwrap_or(0)));
        }
    }
    let old = l.src.rate();
    if let Some((s, _)) = to.convert(&l.src) {
        if !moved.iter().any(|(n, _)| n.is_none()) {
            moved.push((None, old));
        }
        l.src = s;
    }
    let mut said: Vec<String> = Vec::new();
    if !moved.is_empty() {
        moved.sort_by_key(|(n, _)| n.is_some());
        said.push(
            moved
                .iter()
                .map(|(n, r)| {
                    let what = format!("converted from {} to {}", khz(*r), khz(rate));
                    match n {
                        Some(n) => format!("{n} {what}"),
                        None => what,
                    }
                })
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    said.extend(to.kept_note());
    if !said.is_empty() {
        let said = said.join("; ");
        l.note = Some(match l.note.take() {
            Some(n) => format!("{n}; {said}"),
            None => said,
        });
    }
    l
}

/// Every kit of `o` at `rate` (`Project::convert_sounds` with a
/// [`ToRate`]: its sounds at other rates converted, its pads, scenes and
/// slices moved along), with a log line for each kit that had sounds
/// converted and one for sounds kept at their rate.
/// The views of `o`'s sounds kept in their cache files, read with `read`
/// (a file of the project's `samples/`): each one whose file is there and is
/// this sound's ([`SourceView::from_cache`]). A sound without one is
/// analysed when it is shown, as before.
fn cached_views(o: &Opened, read: &ReadSample) -> Vec<Arc<SourceView>> {
    let mut seen = HashSet::new();
    o.kits
        .iter()
        .flat_map(|(_, p)| p.sources())
        .filter(|s| seen.insert(s.id()))
        .filter_map(|s| {
            let bytes = read(&cache_file(s.id())).ok()?;
            SourceView::from_cache(s, &bytes).map(Arc::new)
        })
        .collect()
}

fn opened_at_rate(mut o: Opened, rate: u32, free: fn() -> Option<u64>) -> (Opened, Vec<String>) {
    let mut to = ToRate::new(rate, free);
    let mut said = Vec::new();
    for (k, (name, p)) in o.kits.iter_mut().enumerate() {
        let was: Vec<(SourceId, u32)> = p.sources().iter().map(|s| (s.id(), s.rate())).collect();
        let moved = p.convert_sounds(&mut |s| to.convert(s));
        if moved.is_empty() {
            continue;
        }
        let mut from: Vec<u32> = moved
            .iter()
            .filter_map(|r| was.iter().find(|w| w.0 == r.from).map(|w| w.1))
            .collect();
        from.sort_unstable();
        from.dedup();
        let from: Vec<String> = from.iter().map(|&r| khz(r)).collect();
        let kit = if name.is_empty() {
            format!("kit {}", k + 1)
        } else {
            name.clone()
        };
        let n = moved.len();
        said.push(format!(
            "{kit}: {n} sound{} converted from {} to {}",
            if n == 1 { "" } else { "s" },
            from.join(" and "),
            khz(rate)
        ));
    }
    said.extend(to.kept_note());
    (o, said)
}

/// A rate as said in the log: "48 kHz", "44.1 kHz".
fn khz(rate: u32) -> String {
    format!("{} kHz", f64::from(rate) / 1000.0)
}

/// One sound file, no bigger than the sampler holds in memory, and only
/// while that leaves the memory MPFree keeps free (`free`: what is free
/// now).
fn read_one(path: &Path, free: fn() -> Option<u64>) -> Loaded {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > wav::MAX_SAMPLES as u64 * 8 + (1 << 20) {
        return Err("too long to hold in memory".to_string());
    }
    let file = path.to_string_lossy();
    crate::memory::room_in(free(), crate::memory::decode_need(&file, len))?;
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    sound_file::read(&name, &bytes)
}

/// The Q-Link card a tap touches: its label and whether the wheel is on it.
fn touched_card(ui: &Ui, ev: &UiEvent) -> Option<(String, bool)> {
    if !matches!(ev.kind, EventKind::Select(_)) {
        return None;
    }
    let c = ui.get::<mpfree_sampler_ui::widgets::KnobCard>(&ev.widget)?;
    Some((c.label.clone(), c.focused))
}

/// The settings row a step touches (its name and value as shown), and the
/// step's way.
fn touched_row(ui: &Ui, ev: &UiEvent) -> Option<(mpfree_sampler_ui::widgets::Setting, i32)> {
    let EventKind::Step { index, delta } = ev.kind else {
        return None;
    };
    let row = ui
        .get::<mpfree_sampler_ui::widgets::SettingList>(&ev.widget)?
        .rows
        .get(index)?
        .clone();
    Some((row, delta))
}

/// One kit as a kit file: its `kit.txt` text and each sound it names, by a
/// file name made from the sound's name (`Kick.wav`, `Kick 2.wav` for a
/// second sound of that name).
pub fn kit_file_of(name: &str, p: &Project) -> (String, Vec<(String, Source)>) {
    let mut files: Vec<(String, Source)> = Vec::new();
    let mut by_id = HashMap::new();
    for src in p.sources() {
        let base = mpfree_library::clean_name(&src.name, "Sound");
        let file = mpfree_library::unique_name(&format!("{base}.wav"), |n| {
            n.eq_ignore_ascii_case(mpfree_library::KIT_FILE)
                || files.iter().any(|(f, _)| f.eq_ignore_ascii_case(n))
        });
        by_id.insert(src.id(), file.clone());
        files.push((file, src.clone()));
    }
    let text = save::to_text(&[(name, p)], 0, &|s: &Source| {
        by_id
            .get(&s.id())
            .cloned()
            .unwrap_or_else(|| wav::file_name(s))
    });
    (text, files)
}

/// Reads a kit folder: its `kit.txt` (the sampler's part, one kit) and the
/// sounds it names, each only if `room(file, bytes)` says it fits in
/// memory. Files are looked for in the folder only (no links, no paths).
fn read_kit_file(
    dir: &std::path::Path,
    room: &mut dyn FnMut(&str, u64) -> Result<(), String>,
) -> Result<Opened, String> {
    let text_path = dir.join(mpfree_library::KIT_FILE);
    let m = std::fs::symlink_metadata(&text_path).map_err(|e| format!("kit.txt: {e}"))?;
    if !m.is_file() || m.len() > mpfree_library::kit::MAX_KIT_TEXT {
        return Err("its kit.txt is not a kit file".to_string());
    }
    let text = std::fs::read_to_string(&text_path).map_err(|e| format!("kit.txt: {e}"))?;
    save::from_text(&text, &mut |f| {
        if !mpfree_library::valid_name(f) || f.eq_ignore_ascii_case(mpfree_library::KIT_FILE) {
            return Err("not a sound's name".to_string());
        }
        let p = dir.join(f);
        let m = std::fs::symlink_metadata(&p).map_err(|e| e.to_string())?;
        if !m.is_file() {
            return Err("not a file".to_string());
        }
        room(f, m.len())?;
        let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
        wav::read(f, &bytes)
    })
}

// ------------------------------------------------------------ pad take

/// The pad take's recorder (sampler R5.7): an empty kit pad held + REC
/// ([`crate::rec_arm::RecArm::arm_pad`]) records one take onto that pad
/// with the Rec tab's settings (its source, start, end and channels), and
/// it lands there as a Rec tab take does. It is the Rec tab's take while it
/// runs: the tab shows it, and its Stop ends it too.
impl SamplePage {
    /// What the arm asks of the recorder.
    pub fn pad_take(&mut self, ui: &mut Ui, d: crate::rec_arm::PadDo) {
        use crate::rec_arm::PadDo;
        match d {
            PadDo::Arm { kit, pad } => self.arm_pad_take(ui, kit, pad),
            // A take on the threshold began when it was armed.
            PadDo::Start => self.begin_pad_take(ui),
            PadDo::End => self.end_pad_take(ui),
            PadDo::Throw => self.throw_pad_take(ui),
        }
    }

    /// What the recorder has to say about the pad take since last asked:
    /// it records, or it ended.
    pub fn take_pad_now(&mut self) -> Option<crate::rec_arm::PadNow> {
        self.pad_said.take()
    }

    /// The pad take ends without a take: the log says why, and the arm
    /// hears it ended.
    fn no_pad_take(&mut self, pad: usize, why: &str) {
        self.lines.push(format!(
            "sample: no take on {}: {why}",
            crate::rec_arm::pad_name(pad)
        ));
        self.pad_run = None;
        self.pad_said = Some(crate::rec_arm::PadNow::Ended);
    }

    /// Arms a take onto `pad` of kit `kit`, refused (and ended at once)
    /// while the recorder is busy, the pad has a sound or there is no
    /// input. On the threshold it waits for the sound now; otherwise for
    /// the music ([`crate::rec_arm::PadDo::Start`]).
    fn arm_pad_take(&mut self, ui: &mut Ui, kit: usize, pad: usize) {
        let busy = self.take.is_some() || self.flatten.is_some() || self.pad_run.is_some();
        let has_sound = self
            .projects
            .get(kit)
            .map(|p| p.pad(pad).is_some_and(|pd| !pd.layers.is_empty()));
        let why = if busy {
            Some("the Rec tab is recording")
        } else if has_sound.is_none() || pad >= mpfree_sampler::model::PADS {
            Some("no such pad")
        } else if has_sound == Some(true) {
            Some("it has a sound")
        } else if self.input.is_none() {
            Some("the line input is off")
        } else {
            None
        };
        if let Some(why) = why {
            self.no_pad_take(pad, why);
            return;
        }
        let settings = self.rec.as_ref().map(|t| t.settings).unwrap_or_default();
        let sources: Vec<String> = self.rec_sources.iter().map(|(n, _)| n.clone()).collect();
        self.lines.push(format!(
            "sample: take armed for pad {} (one take, from REC): {}",
            crate::app::kit_pad_name(pad),
            take_plan(&settings, &sources)
        ));
        self.pad_run = Some(PadRun {
            kit,
            pad,
            settings,
            begun: false,
            said: false,
        });
        if settings.start == rec_tab::StartAt::Threshold {
            self.begin_pad_take(ui);
        }
    }

    /// The pad take's go: its take begins (at once, on the next bar line
    /// or on the threshold, as the Rec tab's Start says).
    fn begin_pad_take(&mut self, ui: &mut Ui) {
        let Some(run) = self.pad_run.filter(|r| !r.begun) else {
            return;
        };
        if self.take.is_some() || self.flatten.is_some() {
            self.no_pad_take(run.pad, "the Rec tab is recording");
            return;
        }
        let Some(input) = self.input.as_ref() else {
            self.no_pad_take(run.pad, "the line input is off");
            return;
        };
        let rec = run.settings.recording(input, self.bar_frames(input));
        self.take = Some(Take {
            rec,
            kit: run.kit,
            pad: run.pad,
            pick: run.settings.pick,
            bars: run.settings.in_bars(),
            pad_take: true,
        });
        self.pad_run = Some(PadRun { begun: true, ..run });
        if self.source != RecSource::Line {
            self.lines
                .push(format!("sample: the take resamples {}", self.source_name()));
        }
        self.poll_take(ui);
        self.watch_pad_run();
        self.refresh_rec_state(ui);
        self.feed_rec(ui, 0.0);
    }

    /// REC or STOP: what recorded lands on the pad; a take that hasn't
    /// started is called off.
    fn end_pad_take(&mut self, ui: &mut Ui) {
        let Some(run) = self.pad_run else {
            return;
        };
        if !run.begun {
            self.no_pad_take(run.pad, "called off before it started");
            return;
        }
        if let (Some(t), Some(input)) = (self.take.as_mut(), self.input.as_ref()) {
            if t.pad_take {
                t.rec.stop(input);
            }
        }
        self.poll_take(ui);
        self.watch_pad_run();
        self.refresh_rec_state(ui);
        self.feed_rec(ui, 0.0);
    }

    /// UNDO while it records: the take goes, and the pad stays empty.
    fn throw_pad_take(&mut self, ui: &mut Ui) {
        let Some(run) = self.pad_run else {
            return;
        };
        if self.take.as_ref().is_some_and(|t| t.pad_take) {
            self.take = None;
        }
        self.no_pad_take(run.pad, "thrown away");
        self.refresh_rec_state(ui);
        self.feed_rec(ui, 0.0);
    }

    /// After a poll: the arm hears once that the pad take records, and
    /// that it ended once its take has gone (landed, cancelled or lost).
    fn watch_pad_run(&mut self) {
        use crate::rec_arm::PadNow;
        let Some(run) = self.pad_run.filter(|r| r.begun) else {
            return;
        };
        match self.take.as_ref().filter(|t| t.pad_take) {
            Some(t) => {
                if !run.said && matches!(t.rec.progress(), Progress::Recording { .. }) {
                    self.pad_said = Some(PadNow::Recording);
                    self.pad_run = Some(PadRun { said: true, ..run });
                }
            }
            None => {
                self.pad_run = None;
                self.pad_said = Some(PadNow::Ended);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_sampler::phrase::Hit;

    #[test]
    fn a_phrase_is_a_note_per_slice_on_its_pad_up_to_pad_128() {
        let hit = |slice, beat, beats| Hit { slice, beat, beats };
        let p = Phrase {
            hits: vec![hit(0, 0.0, 0.25), hit(1, 0.25, 0.5), hit(2, 2.75, 1.0)],
            beats: 4,
        };
        let c = phrase_clip("Break", &p, 12, false).unwrap();
        assert_eq!((c.name.as_str(), c.length), ("Break phrase", 4 * 960));
        let notes: Vec<_> = c
            .notes
            .iter()
            .map(|n| (n.tick, n.len, n.note, n.velocity))
            .collect();
        assert_eq!(
            notes,
            [(0, 240, 12, 127), (240, 480, 13, 127), (2640, 960, 14, 127)]
        );
        // Pads past 16 are the kit's next page (PAD BANK B); past 128,
        // refused.
        assert!(c.locks.is_empty());
        assert_eq!(
            phrase_clip("Break", &p, 14, false).unwrap().notes[2].note,
            16
        );
        assert_eq!(
            phrase_clip("Break", &p, 126, false).unwrap_err(),
            "past the kit's last pad"
        );
    }

    #[test]
    fn a_phrase_on_one_pad_is_a_note_per_slice_each_with_a_slice_lock() {
        let hit = |slice, beat, beats| Hit { slice, beat, beats };
        let p = Phrase {
            hits: vec![hit(0, 0.0, 0.25), hit(1, 0.25, 0.5), hit(2, 2.75, 1.0)],
            beats: 4,
        };
        let c = phrase_clip("Break", &p, 20, true).unwrap();
        let notes: Vec<_> = c.notes.iter().map(|n| (n.tick, n.note)).collect();
        assert_eq!(notes, [(0, 20), (240, 20), (2640, 20)], "all on B5");
        let slice = RelAddr::new(OnTrack::Pad(20), PadField::Slice as u16);
        let locks: Vec<_> = c
            .locks
            .iter()
            .map(|l| (l.tick, l.sound, l.addr, l.value))
            .collect();
        assert_eq!(
            locks,
            [
                (0, Some(20), slice, 0.0),
                (240, Some(20), slice, 1.0),
                (2640, Some(20), slice, 2.0)
            ]
        );
        // Pad 128 holds a phrase on one pad; past it, refused.
        assert!(phrase_clip("Break", &p, 127, true).is_ok());
        assert!(phrase_clip("Break", &p, 128, true).is_err());
    }
}
