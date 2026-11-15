//! The mixer in the pad player: mpfree-fx's [`Mixer`] is the one place the
//! sounds are summed.
//!
//! - The playing kit's 16 pads each have a pad channel (Pad Mix), and all
//!   16 go to Track 1 "Drums" (the sampler renders each pad into its own
//!   channel: `render_routed`; the clips play the kit too). Track 2
//!   "Synth" is Poly, Track 3 "Loop" is the audio looper's playback, and
//!   Track 4 "Audio" is the audio tracks on the Song view
//!   (`audiotracks.rs`). The tracks go to the master.
//! - Input 1 "Line in" is the line input, as the stereo pair, going to the
//!   master. It is heard while the looper's Monitor is on (the strip's
//!   Monitor follows it), and it is off while the line input is closed. The
//!   looper records the line input from its own copy, as before.
//! - The master fader is the pad player's Level (Q-Link 1 or the slider on
//!   the Play page), never above `--gain-db`. Master safety (the safe
//!   limiter at -0.3 dBFS) takes the place of the clip at full scale.
//! - The metronome goes on after the mixer, as before: never recorded or
//!   limited (it keeps itself within ±1).
//! - What the looper records from the pads: the drums (the pads that
//!   played, summed from their channels' inputs) and the synth as the
//!   engines make them, before their strips, as before. Recording through
//!   the mixer's capture points comes with the recorder, which makes up
//!   the period they arrive late.
//!
//! The Play page's effects are inserts from the start ([`PlayFx`]): a DJ
//! filter on the Drums and Loop tracks (the Filter Q-Links), a 1/8 dotted
//! delay on Return A (the Delay send Q-Links send to it), and Perform FX
//! (`mpfree_fx::Perform`) on the master, before Master safety: the throws
//! hold its pads, the XY pad plays its DJ filter and echo, and the FX mix
//! Q-Link is its Mix. Each rests (costs next to nothing) until it is used.
//!
//! Parameter locks: the clips' locks on strip controls and inserts go
//! through the mod engine ([`MixHost::lock`], `modmix.rs`), and once a
//! control has been locked the host's own turns of it go through the
//! engine too ([`MixHost::set`]), so the end of a lock puts back the last
//! turn.
//!
//! The screen: while the mixer panel shows ([`MixHost::show`]), the audio
//! loop sends a [`MixState`] every [`STATE_EVERY`] (it takes the meters),
//! and applies what the panel sends ([`MixHost::action`]).
//!
//! Nothing in [`MixHost::period`] allocates. Its time is in the "mix" stage
//! of CPU by engine; the state for the screen allocates, a few times a
//! second while the panel shows, as the other pages' states do.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_fx::ids::mixer::output;
use mpfree_fx::ids::mixer::{self as mx, monitor, source};
use mpfree_fx::ids::{delay, dj_filter};
use mpfree_fx::{Delay, DjFilter, Edit, Insert, Mixer, MixerConfig, StripId, CAPTURES};
use mpfree_mixer_ui::{default_name, MixAction, MixState, MixView};
use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_play::mpfree_param::{Owner, ParamAddr};
use mpfree_sampler::engine::PadOut;

use crate::instruments::{self, Instruments};
use crate::plugins::Plugins;
use crate::PERIOD;
use mpfree_fx::ids::perform as perf;

/// The tracks: what each engine plays into.
pub const DRUMS: usize = 0;
pub const SYNTH: usize = 1;
pub const LOOP: usize = 2;
/// The audio tracks (`audiotracks.rs`), summed after each track's own
/// level, pan, mute and solo.
pub const AUDIO: usize = 3;
pub const TRACKS: usize = 4;
/// Pad channels: the playing kit's pads.
pub const PADS: usize = 16;
/// The line input's strip.
pub const LINE: StripId = StripId::Input(0);
/// Insert slots each strip has room for.
pub const SLOTS: usize = 8;
/// How often the panel gets a new state (meters) while it shows.
pub const STATE_EVERY: Duration = Duration::from_millis(40);
/// The Play page's inserts: the DJ filters' slot on the Drums and Loop
/// tracks, the delay's return, and the throws' slot on the master.
const FILTER_SLOT: usize = 0;
/// [`MixHost::poll_built`]'s line when the builder's queue was full and an
/// effect taken out of a chain was freed on the audio thread.
const DROPPED_HERE: &str =
    "mixer: an effect taken out was freed on the audio thread (the builder's queue was full)";
pub const DELAY_RETURN: usize = 0;
const PERFORM_SLOT: usize = 0;
/// A send this low is off.
pub const SEND_OFF_DB: f32 = -60.0;
/// The capture point the sampler resamples from (the Rec tab's Source):
/// the last of the mixer's [`CAPTURES`], so the others stay free for
/// print, stems and the looper.
pub const SAMPLER_CAPTURE: usize = CAPTURES - 1;

/// What the Play page's Q-Links and speakers show, read from the mixer (so
/// the mixer panel's changes show there too).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayLevels {
    /// The Drums and Loop tracks' sends to the delay, dB.
    pub drums_send_db: f32,
    pub loop_send_db: f32,
    /// The Loop track's and the line input's faders, dB.
    pub loop_db: f32,
    pub line_db: f32,
    /// The DJ filters, -100 (low-pass) to 100 (high-pass); 0 is off.
    pub drums_filter: f32,
    pub loop_filter: f32,
    /// The XY pad's FX mix, 0 to 1.
    pub fx_mix: f32,
    /// Pads muted (a bit each), as the speakers show them.
    pub pad_mutes: u16,
}

/// A change from the Play page for the mixer. The page says what it wants
/// (a filter's place, a throw held, where the XY pad is); which engine and
/// which parameter make it is the mixer's business ([`MixHost::play`]), so
/// another performance engine swaps in here without the page changing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayFx {
    /// A strip's parameter (a fader, a send).
    Strip { strip: StripId, id: u16, value: f32 },
    /// A track's DJ filter: -100 (low-pass) to 100 (high-pass), 0 off.
    Filter { strip: StripId, position: f32 },
    /// A speaker: mute or unmute a pad.
    PadMute { pad: usize, mute: bool },
    /// Drop drums: these pads (a bit each) muted while held, as well.
    Drop(u16),
    /// A throw held or let go.
    Throw { fx: ThrowFx, down: bool },
    /// The XY pad: where the finger is (0 to 1 each way), and whether it
    /// is down.
    Xy { x: f32, y: f32, down: bool },
}

/// The performance effects a throw holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThrowFx {
    Delay,
    Reverb,
    Stutter,
    TapeStop,
    FilterDip,
    BeatRepeat,
    VinylBrake,
}

impl ThrowFx {
    /// The Perform FX pad (0 to 15) that plays it, in the default bank:
    /// Stutter 1/16, Tape stop 1/4, Vinyl brake 1/2, Echo throw 1/8 dotted,
    /// LP sweep 1 bar, Beat repeat 1/4 and Wash throw.
    pub fn pad(self) -> usize {
        match self {
            ThrowFx::Stutter => 0,
            ThrowFx::TapeStop => 2,
            ThrowFx::VinylBrake => 3,
            ThrowFx::Delay => 9,
            ThrowFx::FilterDip => 10,
            ThrowFx::BeatRepeat => 14,
            ThrowFx::Reverb => 15,
        }
    }
}

/// A period of stereo sound.
pub type Stereo<'a> = (&'a [f32; PERIOD], &'a [f32; PERIOD]);

/// The strips' names on the panel.
pub fn name(s: StripId) -> String {
    match s {
        StripId::Track(DRUMS) => "Drums".into(),
        StripId::Track(SYNTH) => "Synth".into(),
        StripId::Track(LOOP) => "Loop".into(),
        StripId::Track(AUDIO) => "Audio".into(),
        StripId::Track(t) if t >= TRACKS => format!("Track {}", t - TRACKS + 1),
        LINE => "Line in".into(),
        s => default_name(s),
    }
}

pub struct MixHost {
    pub mixer: Mixer,
    /// The mod engine between the clips' locks and the mixer.
    pub modu: crate::modmix::ModMix,
    /// The highest the master fader goes (`--gain-db`).
    max_db: f32,
    monitor: bool,
    line_open: bool,
    /// Master safety's count of frames over the ceiling, at the last period.
    overs: u64,
    /// The panel's view while it shows, and the strip selected on it.
    shown: Option<MixView>,
    /// The view shown last, for the Mix tab.
    last: MixView,
    selected: Option<StripId>,
    state_at: Option<Instant>,
    /// Pads muted by their speakers or the panel, and by Drop drums.
    pad_mutes: u16,
    dropped: u16,
    /// Builds the chain editor's effects, and drops the ones edits take
    /// out, off the audio thread (control.rs). None: here (the tests).
    builder: Option<crate::control::Builder>,
    /// A built effect the mixer had no room for yet (its edits were full),
    /// put in on a later period.
    landing: Option<crate::control::Built>,
    /// The CLAP plugins found (plugins.rs): the picker's plugin cards and
    /// what the builder makes `clap:` effects with.
    plugins: Arc<Plugins>,
    /// A project's mixer being swapped in ([`MixHost::open`]).
    opening: Option<Box<Open>>,
    /// Each track's own instrument (instruments.rs), rendered into its
    /// mixer track before the mixer runs.
    pub instruments: Instruments,
    /// How long the tracks' own instruments took in the last period (0
    /// when no track has one), for load shedding's cost a voice (shed.rs).
    pub instrument_time: Duration,
    /// Instruments made without a builder (the tests), waiting to go in.
    made: Vec<crate::control::MadeInstrument>,
    /// Each plugin synth's own presets, once listed (or why not), by its
    /// node id; and the ones asked for and not back yet.
    plugin_presets: Vec<(&'static str, Result<Vec<mpfree_plughost::Preset>, String>)>,
    presets_asked: Vec<&'static str>,
    /// Preset news made without a builder (the tests).
    preset_news: Vec<crate::control::PresetNews>,
    /// The effect picker's favourites and recents (`files/fx/`), and
    /// whether they changed since they were last saved.
    pub shelf: mpfree_fx::picker::Shelf,
    pub shelf_dirty: bool,
    /// Effects' modes (fxmodes.rs): the values each insert had in the
    /// engines it left, and the switches across engines on their way.
    pub modes: crate::fxmodes::Modes,
    /// Freeze and Print (takes.rs).
    pub takes: crate::takes::Takes,
    /// A take's memory made without a builder (the tests).
    here: crate::takes::Here,
}

/// The take host's memory: made and dropped on the builder thread, or
/// here without one (the tests).
struct TakeMem<'a> {
    builder: Option<&'a mut crate::control::Builder>,
    here: &'a mut crate::takes::Here,
}

impl crate::takes::Memory for TakeMem<'_> {
    fn ask(&mut self, rate: u32, seconds: f32) -> bool {
        match self.builder.as_deref_mut() {
            Some(b) => b.print(rate, seconds),
            None => self.here.ask(rate, seconds),
        }
    }
    fn made(&mut self) -> Option<mpfree_fx::Print> {
        match self.builder.as_deref_mut() {
            Some(b) => b.take_print(),
            None => self.here.made(),
        }
    }
    fn drop_frozen(&mut self, f: mpfree_fx::Frozen) {
        if let Some(b) = self.builder.as_deref_mut() {
            b.drop_frozen_later(f);
        }
    }
    fn drop_print(&mut self, p: mpfree_fx::Print) {
        if let Some(b) = self.builder.as_deref_mut() {
            b.drop_print_later(p);
        }
    }
}

/// What [`MixHost::poll_instruments`] put in (or couldn't) for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landed {
    /// Track `track` now plays `name`.
    Playing { track: usize, name: &'static str },
    /// Instrument `id` couldn't be made for track `track`; it keeps what
    /// it had.
    Failed { track: usize, id: &'static str },
    /// Instrument `name` was made for track `track`, but
    /// [`instruments::SLOTS`] other tracks already have their own; it keeps
    /// what it had.
    Full { track: usize, name: &'static str },
}

/// A project's mixer going in over a few periods: the old inserts out,
/// then the new ones in, each as an edit the mixer makes while the strip
/// has ducked out; then the keys, the strips' controls and the mod engine.
pub struct Open {
    load: Box<crate::project::MixLoad>,
    stage: OpenStage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenStage {
    /// Taking every strip's inserts out.
    Clear,
    /// Putting the project's in.
    Fill,
    /// Waiting for the last edits, then the rest.
    Finish,
}

impl MixHost {
    /// Made before the sound starts (it allocates). `level_db` is the
    /// Level now, `max_db` the most it goes to.
    pub fn new(rate: u32, level_db: f32, max_db: f32) -> MixHost {
        let cfg = MixerConfig {
            inputs: 1,
            pads: PADS,
            // The engines' tracks, then one for each sequencer track's own
            // instrument (unused ones cost a few flag checks).
            tracks: TRACKS + instruments::TRACKS,
            slots: SLOTS,
            max_latency_ms: 50.0,
        };
        let mut mixer = Mixer::new(cfg, rate, PERIOD);
        for p in 0..PADS {
            mixer.set_now(StripId::Pad(p), mx::OUTPUT, output::TRACK);
            mixer.set_now(StripId::Pad(p), mx::OUTPUT_TRACK, DRUMS as f32);
        }
        mixer.set_now(StripId::Master, mx::VOLUME, level_db.min(max_db));
        mixer.set_now(LINE, mx::SOURCE, source::OFF);
        mixer.set_now(LINE, mx::MONITOR, monitor::OFF);
        // The Play page's effects.
        let ret = StripId::Return(DELAY_RETURN);
        let mut echo = Delay::new();
        echo.set_param(delay::MIX, 100.0);
        let inserts: [(StripId, Box<dyn Node>); 4] = [
            (StripId::Track(DRUMS), Box::new(DjFilter::new())),
            (StripId::Track(LOOP), Box::new(DjFilter::new())),
            (ret, Box::new(echo)),
            (StripId::Master, Box::new(mpfree_fx::Perform::new())),
        ];
        for (strip, node) in inserts {
            let insert = Insert::new(node, rate, PERIOD);
            // Room for these is made above (SLOTS), so this never fails.
            let _ = mixer.edit(Edit::Insert {
                strip,
                at: 0,
                insert,
            });
        }
        for t in [DRUMS, LOOP] {
            mixer.set_now(StripId::Track(t), mx::send(DELAY_RETURN), SEND_OFF_DB);
        }
        MixHost {
            mixer,
            modu: crate::modmix::ModMix::new(rate),
            max_db,
            monitor: false,
            line_open: false,
            overs: 0,
            shown: None,
            last: MixView::Tracks,
            selected: None,
            state_at: None,
            pad_mutes: 0,
            dropped: 0,
            builder: None,
            landing: None,
            plugins: Arc::new(Plugins::none()),
            opening: None,
            instruments: Instruments::new(rate, PERIOD),
            instrument_time: Duration::ZERO,
            made: Vec::new(),
            plugin_presets: Vec::new(),
            presets_asked: Vec::new(),
            preset_news: Vec::new(),
            shelf: mpfree_fx::picker::Shelf::default(),
            shelf_dirty: false,
            modes: crate::fxmodes::Modes::default(),
            takes: crate::takes::Takes::new(rate),
            here: crate::takes::Here::default(),
        }
    }

    /// Asks for a track's own instrument (`job`): made on the builder
    /// thread and put in by [`MixHost::poll_instruments`]; without a
    /// builder (the tests), made here. False if the builder is busy.
    pub fn build_instrument(&mut self, job: crate::control::InstrumentJob) -> bool {
        let rate = self.mixer.rate();
        if let Some(b) = self.builder.as_mut() {
            return b.instrument(Box::new(job), rate, PERIOD);
        }
        let made = crate::control::make_instrument(job, &self.plugins, rate, PERIOD);
        self.made.push(made);
        true
    }

    /// Once a period: puts in an instrument the builder made, if one is
    /// ready (the old one rings out), and drops retired ones off the audio
    /// thread. Never waits or allocates.
    pub fn poll_instruments(&mut self) -> Option<Landed> {
        while let Some(old) = self.instruments.pop_retired() {
            self.drop_instrument(old);
        }
        let m = match self.builder.as_mut() {
            Some(b) => b.take_instrument()?,
            None => self.made.pop()?,
        };
        let Some((inst, held)) = m.made else {
            return Some(Landed::Failed {
                track: m.track,
                id: m.id,
            });
        };
        match self
            .instruments
            .put(m.track, (inst, held), m.pads, &m.tuning)
        {
            Ok(()) => Some(Landed::Playing {
                track: m.track,
                name: held.name,
            }),
            Err((why, back)) => {
                if let Some(i) = back {
                    self.drop_instrument(i);
                }
                let track = m.track;
                Some(match why {
                    instruments::Refused::Full => Landed::Full {
                        track,
                        name: held.name,
                    },
                    _ => Landed::Failed { track, id: m.id },
                })
            }
        }
    }

    /// Plugin synth `id`'s own presets, once listed (None until then, or if
    /// they couldn't be).
    pub fn plugin_presets(&self, id: &str) -> Option<&[mpfree_plughost::Preset]> {
        self.plugin_presets
            .iter()
            .find(|(i, _)| *i == id)
            .and_then(|(_, l)| l.as_deref().ok())
    }

    /// Asks for plugin synth `id`'s presets, once: listed on the builder
    /// thread (the first time in a process of their own, which can take
    /// seconds) and kept when [`MixHost::poll_presets`] hands them back.
    pub fn ask_presets(&mut self, id: &'static str) {
        if !Plugins::is_plugin(id)
            || self.presets_asked.contains(&id)
            || self.plugin_presets.iter().any(|(i, _)| *i == id)
        {
            return;
        }
        let asked = match self.builder.as_mut() {
            Some(b) => b.presets(id),
            None => {
                let list = self.plugins.presets(id);
                self.preset_news
                    .push(crate::control::PresetNews::Listed { id, list });
                true
            }
        };
        if asked {
            self.presets_asked.push(id);
        }
    }

    /// Asks for preset `index` of track `track`'s plugin synth to be
    /// loaded (on the builder thread); [`MixHost::poll_presets`] says how
    /// it went. False if the track has no plugin, its presets aren't
    /// listed, there's no such preset, or the builder is busy.
    pub fn load_plugin_preset(&mut self, track: usize, index: usize) -> bool {
        let Some(held) = self.instruments.held(track) else {
            return false;
        };
        let Some(preset) = self
            .plugin_presets(held.id)
            .and_then(|l| l.get(index))
            .cloned()
        else {
            return false;
        };
        let job = Box::new(crate::control::PresetJob {
            track,
            instance: held.instance,
            index,
            preset,
        });
        match self.builder.as_mut() {
            Some(b) => b.load_preset(job),
            None => {
                let news = job.run(&self.plugins);
                self.preset_news.push(news);
                true
            }
        }
    }

    /// Asks for track `track`'s plugin's values and texts, for its skin (on
    /// the builder thread); [`MixHost::poll_presets`] brings them. False if
    /// the track has no plugin or the builder is busy.
    pub fn ask_values(&mut self, track: usize) -> bool {
        let Some(held) = self.instruments.held(track) else {
            return false;
        };
        if !Plugins::is_plugin(held.id) {
            return false;
        }
        let instance = held.instance;
        match self.builder.as_mut() {
            Some(b) => b.values(track, instance),
            None => {
                let values = self.plugins.values(instance);
                self.preset_news.push(crate::control::PresetNews::Values {
                    track,
                    instance,
                    values,
                });
                true
            }
        }
    }

    /// Once a period: presets listed or loaded on the builder thread, if
    /// any are back (a listed plugin's are kept for
    /// [`MixHost::plugin_presets`]). Never waits.
    pub fn poll_presets(&mut self) -> Option<crate::control::PresetNews> {
        let news = match self.builder.as_mut() {
            Some(b) => b.take_preset_news(),
            None => (!self.preset_news.is_empty()).then(|| self.preset_news.remove(0)),
        }?;
        if let crate::control::PresetNews::Listed { id, list } = &news {
            self.presets_asked.retain(|i| i != id);
            self.plugin_presets.push((id, list.clone()));
        }
        Some(news)
    }

    fn drop_instrument(&mut self, inst: mpfree_track::Instrument) {
        match self.builder.as_mut() {
            Some(b) => {
                b.drop_instrument_later(inst);
            }
            None => drop(inst),
        }
    }

    /// Starts swapping in a project's mixer (built on the screen thread:
    /// `project::mixer_load`). [`MixHost::tend_open`] does it, a few edits
    /// a period; until then the old mixer plays on. A second open replaces
    /// one under way (its effects are dropped off this thread).
    pub fn open(&mut self, load: Box<crate::project::MixLoad>) {
        // Frozen strips are not saved: everything thaws.
        let mut mem = TakeMem {
            builder: self.builder.as_mut(),
            here: &mut self.here,
        };
        self.takes.clear(&mut self.mixer, &mut mem);
        if let Some(mut old) = self.opening.take() {
            self.drop_load(&mut old.load);
        }
        // Its inserts may reuse the old ones' slot ids.
        self.modes = crate::fxmodes::Modes::default();
        self.opening = Some(Box::new(Open {
            load,
            stage: OpenStage::Clear,
        }));
    }

    /// Whether a project's mixer is still going in.
    pub fn opening(&self) -> bool {
        self.opening.is_some()
    }

    /// The effects of a load not used, dropped off the audio thread.
    fn drop_load(&mut self, load: &mut crate::project::MixLoad) {
        for (_, chain) in load.chains.drain(..) {
            for (ins, _) in chain {
                let _ = self.drop_off_thread(ins);
            }
        }
        // The rest is small (a few vectors): dropped here.
    }

    /// Once a period while [`MixHost::opening`]: the next edits of the
    /// swap. Returns a line for the log when it is done. Edits never wait:
    /// one the mixer has no room for is made on a later period. Allocates
    /// only at the end, when the mod engine reads its part.
    pub fn tend_open(&mut self) -> Option<String> {
        let mut open = self.opening.take()?;
        if self.mixer.edits_pending() > 0 {
            self.opening = Some(open);
            return None;
        }
        let strips: [StripId; 32] = std::array::from_fn(|k| self.strip_at(k));
        match open.stage {
            OpenStage::Clear => {
                let mut left = false;
                for &st in strips.iter().take(self.strip_count()) {
                    let n = self.mixer.chain(st).map_or(0, |c| c.len());
                    for _ in 0..n {
                        if self.mixer.edit(Edit::Remove { strip: st, at: 0 }).is_err() {
                            left = true;
                            break;
                        }
                    }
                }
                if !left {
                    open.stage = OpenStage::Fill;
                }
            }
            OpenStage::Fill => {
                let mut left = false;
                let load = &mut *open.load;
                for (st, chain) in load.chains.iter_mut() {
                    while !chain.is_empty() {
                        let at = self.mixer.chain(*st).map_or(0, |c| c.len());
                        let (ins, key) = chain.remove(0);
                        match self.mixer.edit(Edit::Insert {
                            strip: *st,
                            at,
                            insert: ins,
                        }) {
                            Ok(()) => {
                                // Its key once it is in (Finish).
                                if let Some(k) = key {
                                    load.keys.push((*st, at, k));
                                }
                            }
                            Err(Edit::Insert { insert, .. }) => {
                                chain.insert(0, (insert, key));
                                left = true;
                                break;
                            }
                            Err(_) => {}
                        }
                    }
                    if left {
                        break;
                    }
                }
                if !left {
                    open.stage = OpenStage::Finish;
                }
            }
            OpenStage::Finish => {
                let load = &open.load;
                for &(st, ref values) in &load.strips {
                    if self.mixer.chain(st).is_none() && !matches!(st, StripId::Master) {
                        // A strip this mixer hasn't got.
                        if self.mixer.strips().all(|s| s != st) {
                            continue;
                        }
                    }
                    for &(id, v) in values {
                        self.mixer.set_now(st, id, v);
                    }
                }
                // A track's own channel the project leaves out has nothing
                // set (`project::untouched_channel`): back to its defaults,
                // whatever the last project had there.
                for t in TRACKS..self.mixer.tracks() {
                    let st = StripId::Track(t);
                    if load.strips.iter().any(|(s, _)| *s == st) {
                        continue;
                    }
                    for p in mpfree_fx::strip_params() {
                        if crate::project::saved_control(st, p.id)
                            && self.mixer.param(st, p.id) != p.default
                        {
                            self.mixer.set_now(st, p.id, p.default);
                        }
                    }
                }
                for &(st, slot, k) in &load.keys {
                    self.mixer.set_key(st, slot, Some(k));
                }
                self.pad_mutes = load.pad_mutes;
                self.apply_mutes();
                let effects: usize = self
                    .mixer
                    .strips()
                    .map(|s| self.mixer.chain(s).map_or(0, |c| c.len()))
                    .sum();
                let skipped = match &load.modu {
                    Some(text) => self.modu.load(&self.mixer, text),
                    None => 0,
                };
                let mut line = format!("mixer: the project's mixer is in ({effects} effects)");
                if skipped > 0 {
                    line += &format!("; {skipped} mod lines skipped");
                }
                return Some(line);
            }
        }
        self.opening = Some(open);
        None
    }

    fn strip_count(&self) -> usize {
        self.mixer.strips().count().min(32)
    }

    fn strip_at(&self, k: usize) -> StripId {
        self.mixer.strips().nth(k).unwrap_or(StripId::Master)
    }

    /// One period of silence through the mixer (tests: edits land).
    #[cfg(test)]
    pub fn run_silent_period(&mut self) {
        let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
        let ctx = Ctx::new(self.mixer.rate(), 0);
        self.mixer.process(&ctx, &mut l, &mut r);
        while let Some(old) = self.mixer.pop_retired() {
            drop(old);
        }
    }

    /// The plugins the chain editor offers and the builder makes. Before
    /// [`MixHost::build_off_thread`], which hands them to the builder.
    pub fn set_plugins(&mut self, plugins: Arc<Plugins>) {
        self.plugins = plugins;
    }

    pub fn plugins(&self) -> &Arc<Plugins> {
        &self.plugins
    }

    /// From now on effects added in the chain editor are built on the
    /// `mpfree-build` thread, and the effects edits take out are dropped
    /// there (control.rs). Allocates and starts the thread: call it before
    /// the loop.
    pub fn build_off_thread(&mut self) -> std::io::Result<()> {
        self.builder = Some(crate::control::Builder::start(Arc::clone(&self.plugins))?);
        Ok(())
    }

    /// [`MixHost::add_insert_with`] at the effect's defaults (the tests).
    #[cfg(test)]
    pub fn add_insert(&mut self, strip: StripId, id: &'static str) -> Result<(), &'static str> {
        self.add_insert_with(strip, id, &[])
    }

    /// Adds effect `id` at the end of `strip`'s chain, with the values in
    /// `preset` set before it goes in (a factory preset from the picker;
    /// empty for its defaults). With a builder it is built there and goes
    /// in a few periods later ([`MixHost::poll_built`]); without one, here.
    /// Err: the mixer is busy (too many edits or builds under way), or no
    /// effect has that id.
    pub fn add_insert_with(
        &mut self,
        strip: StripId,
        id: &'static str,
        preset: &'static [(u16, f32)],
    ) -> Result<(), &'static str> {
        let rate = self.mixer.rate();
        if let Some(b) = self.builder.as_mut() {
            return if b.build_with(strip, id, preset, rate, PERIOD) {
                Ok(())
            } else {
                Err("The mixer is busy")
            };
        }
        let mut node = if Plugins::is_plugin(id) {
            self.plugins
                .make(id)
                .map_err(|_| "The plugin would not start")?
        } else {
            mpfree_fx_ui::make(id).ok_or("No such effect")?
        };
        for &(p, v) in preset {
            node.set_param(p, v);
        }
        let at = self.mixer.chain(strip).map_or(0, |c| c.len());
        self.mixer
            .edit(Edit::Insert {
                strip,
                at,
                insert: Insert::new(node, rate, PERIOD),
            })
            .map_err(|_| "The mixer is busy")
    }

    /// Switches insert `uid` on `strip` to engine `id` across engines (a
    /// family's mode, `mpfree_fx::families`): built like an added effect,
    /// set up by `switch`, and swapped in place with the same slot id
    /// ([`MixHost::poll_built`]). Err: the mixer is busy, or no effect has
    /// that id.
    pub fn replace_insert(
        &mut self,
        strip: StripId,
        uid: u16,
        id: &'static str,
        switch: mpfree_fx::families::Switch,
    ) -> Result<(), &'static str> {
        let rate = self.mixer.rate();
        if let Some(b) = self.builder.as_mut() {
            return if b.build_switch(strip, uid, id, switch, rate, PERIOD) {
                Ok(())
            } else {
                Err("The mixer is busy")
            };
        }
        let mut node = mpfree_fx_ui::make(id).ok_or("No such effect")?;
        switch.apply(node.as_mut());
        let at = self
            .mixer
            .chain(strip)
            .and_then(|c| c.slot_of(uid))
            .ok_or("That effect has gone")?;
        let insert = self.keep_bypass(strip, at, Insert::new(node, rate, PERIOD).with_uid(uid));
        self.mixer
            .edit(Edit::Replace { strip, at, insert })
            .map_err(|_| "The mixer is busy")
    }

    /// An insert taking the place of slot `at` of `strip`, bypassed if the
    /// one there is.
    fn keep_bypass(&self, strip: StripId, at: usize, insert: Insert) -> Insert {
        match self.mixer.chain(strip) {
            Some(c) if !c.is_on(at) => insert.off(),
            _ => insert,
        }
    }

    /// Effects asked for and not in the mixer's edits yet.
    pub fn inserts_building(&self) -> usize {
        self.builder.as_ref().map_or(0, |b| b.pending()) + usize::from(self.landing.is_some())
    }

    /// Once a period: puts the effects the builder made into their chains
    /// (at the end, or in the place of the insert a switch of mode
    /// replaces), and hands the ones edits took out back to be dropped
    /// off the audio thread; then a switch's macros and rows follow it
    /// (`fxmodes::land`). Returns a line for the log when an effect
    /// could not be made. No allocation while nothing is edited; never
    /// waits.
    pub fn poll_built(&mut self) -> Option<&'static str> {
        let mut said = None;
        loop {
            let built = match self.landing.take() {
                Some(b) => b,
                None => match self.builder.as_mut().and_then(|b| b.take()) {
                    Some(b) => b,
                    None => break,
                },
            };
            let Some(insert) = built.insert else {
                said = Some("mixer: the chain editor asked for an effect that has no maker");
                continue;
            };
            // A switch of mode takes its insert's slot, and its slot id;
            // an added effect goes at the end.
            let at = self
                .mixer
                .chain(built.strip)
                .and_then(|c| match built.replace {
                    Some(uid) => c.slot_of(uid),
                    None => Some(c.len()),
                });
            let Some(at) = at else {
                said = Some(match built.replace {
                    Some(_) => "mixer: an effect was switched after it was taken out",
                    None => "mixer: an effect was built for a strip the mixer does not have",
                });
                if !self.drop_off_thread(insert) {
                    said = Some(DROPPED_HERE);
                }
                continue;
            };
            let edit = match built.replace {
                Some(uid) => Edit::Replace {
                    strip: built.strip,
                    at,
                    insert: self.keep_bypass(built.strip, at, insert.with_uid(uid)),
                },
                None => Edit::Insert {
                    strip: built.strip,
                    at,
                    insert,
                },
            };
            if let Err(Edit::Insert { insert, .. } | Edit::Replace { insert, .. }) =
                self.mixer.edit(edit)
            {
                // The mixer's edits are full: it goes in on a later period.
                self.landing = Some(crate::control::Built {
                    strip: built.strip,
                    id: built.id,
                    insert: Some(insert),
                    replace: built.replace,
                });
                break;
            }
        }
        while let Some(old) = self.mixer.pop_retired() {
            if !self.drop_off_thread(old) {
                said = Some(DROPPED_HERE);
            }
        }
        crate::fxmodes::land(self);
        said
    }

    /// Drops `insert` on the builder's thread, or here without one (the
    /// tests). False if a builder's queue was full and it was dropped here.
    fn drop_off_thread(&mut self, insert: Insert) -> bool {
        match self.builder.as_mut() {
            Some(b) => b.drop_later(insert),
            None => {
                drop(insert);
                true
            }
        }
    }

    /// A change from the Play page.
    pub fn play(&mut self, fx: PlayFx) {
        match fx {
            PlayFx::Strip { strip, id, value } => {
                self.set(strip, id, value);
            }
            PlayFx::Filter { strip, position } => {
                self.insert(strip, FILTER_SLOT, dj_filter::POSITION, position);
            }
            PlayFx::Throw { fx, down } => {
                // The throw holds its Perform FX pad (the default bank's,
                // as the FX thread laid it out for the throws).
                self.insert(
                    StripId::Master,
                    PERFORM_SLOT,
                    perf::hold(fx.pad()),
                    if down { 1.0 } else { 0.0 },
                );
            }
            PlayFx::Xy { x, y, .. } => {
                let pad = mpfree_fx::XyPad::perform();
                for (axis, pos) in [(pad.x, x), (pad.y, y)] {
                    self.insert(StripId::Master, PERFORM_SLOT, axis.param, axis.value(pos));
                }
            }
            PlayFx::PadMute { pad, mute } if pad < PADS => {
                if mute {
                    self.pad_mutes |= 1 << pad;
                } else {
                    self.pad_mutes &= !(1 << pad);
                }
                self.apply_mutes();
            }
            PlayFx::PadMute { .. } => {}
            PlayFx::Drop(pads) => {
                self.dropped = pads;
                self.apply_mutes();
            }
        }
    }

    fn insert(&mut self, strip: StripId, slot: usize, id: u16, value: f32) {
        let addr = crate::modmix::strip_of(strip).and_then(|s| {
            let uid = self.mixer.chain(strip)?.uid(slot)?;
            Some(ParamAddr::new(Owner::Insert(s, uid), id))
        });
        if addr.is_some_and(|a| self.modu.set(a, value)) {
            return;
        }
        self.mixer
            .queue_insert(strip, slot, Timed::new(0, NodeEvent::Param { id, value }));
    }

    /// A strip control as the mixer panel set it, for UNDO: a pad's mute
    /// is its own, not Drop drums' held over it.
    pub fn value(&self, strip: StripId, id: u16) -> f32 {
        match strip {
            StripId::Pad(p) if id == mx::MUTE && p < PADS => {
                f32::from(u8::from(self.pad_mutes & (1 << p) != 0))
            }
            _ => self.mixer.param(strip, id),
        }
    }

    /// Sets a strip control from the start of the next period: through the
    /// mod engine once a lock has made it a target, else straight to the
    /// mixer.
    pub fn set(&mut self, strip: StripId, id: u16, value: f32) {
        let addr = crate::modmix::strip_of(strip).map(|s| ParamAddr::new(Owner::Strip(s), id));
        if !addr.is_some_and(|a| self.modu.set(a, value)) {
            self.mixer.set(strip, id, value);
        }
    }

    /// A clip's parameter lock (`value`) or its end (`None`), `offset`
    /// frames into the coming period.
    pub fn lock(&mut self, offset: u32, addr: ParamAddr, value: Option<f32>) {
        self.modu.lock(&self.mixer, offset, addr, value);
    }

    /// A clip's move lane value (an FX move), `offset` frames into the
    /// coming period.
    pub fn lane(&mut self, offset: u32, addr: ParamAddr, value: f32) {
        self.modu.lane(&self.mixer, offset, addr, value);
    }

    fn apply_mutes(&mut self) {
        for p in 0..PADS {
            let on = (self.pad_mutes | self.dropped) & (1 << p) != 0;
            let want = if on { 1.0 } else { 0.0 };
            if self.mixer.param(StripId::Pad(p), mx::MUTE) != want {
                self.set(StripId::Pad(p), mx::MUTE, want);
            }
        }
    }

    /// Master safety's limiting now, dB (0, or a positive number of dB
    /// taken off), for the master meter.
    pub fn gain_reduction_db(&self) -> f32 {
        use mpfree_node::Node as _;
        -self
            .mixer
            .safety()
            .param(mpfree_fx::ids::master::REDUCTION)
            .min(0.0)
    }

    /// The pads muted by their speakers or SHIFT + TRACK MUTE (a bit
    /// each; Drop drums' are not counted).
    pub fn pad_mutes(&self) -> u16 {
        self.pad_mutes
    }

    /// What the Play page shows. Cheap: a few reads.
    pub fn play_levels(&self) -> PlayLevels {
        let m = &self.mixer;
        let insert = |s: StripId, slot: usize, id: u16| {
            m.chain(s)
                .and_then(|c| c.node(slot))
                .map_or(0.0, |n| n.param(id))
        };
        PlayLevels {
            drums_send_db: m.param(StripId::Track(DRUMS), mx::send(DELAY_RETURN)),
            loop_send_db: m.param(StripId::Track(LOOP), mx::send(DELAY_RETURN)),
            loop_db: m.param(StripId::Track(LOOP), mx::VOLUME),
            line_db: m.param(LINE, mx::VOLUME),
            drums_filter: insert(StripId::Track(DRUMS), FILTER_SLOT, dj_filter::POSITION),
            loop_filter: insert(StripId::Track(LOOP), FILTER_SLOT, dj_filter::POSITION),
            fx_mix: insert(StripId::Master, PERFORM_SLOT, perf::MIX) / 100.0,
            pad_mutes: self.pad_mutes,
        }
    }

    /// Points the sampler's capture point at `src`, or frees it for the
    /// line input. Takes effect from the next block; false if the mixer
    /// has no such strip (the capture point is then free).
    pub fn set_rec_source(&mut self, src: crate::sampler::RecSource) -> bool {
        match src {
            crate::sampler::RecSource::Line => {
                self.mixer.set_capture(SAMPLER_CAPTURE, None);
                true
            }
            crate::sampler::RecSource::Mix(strip, point) => {
                let ok = self
                    .mixer
                    .set_capture(SAMPLER_CAPTURE, Some((strip, point)));
                if !ok {
                    self.mixer.set_capture(SAMPLER_CAPTURE, None);
                }
                ok
            }
        }
    }

    /// What the sampler's capture point took in the last period, and how
    /// many frames late it hears the transport's block; None while it is
    /// free.
    pub fn rec_capture(&self) -> Option<(&[f32], &[f32], u32)> {
        let (l, r) = self.mixer.capture(SAMPLER_CAPTURE)?;
        let latency = self.mixer.capture_latency(SAMPLER_CAPTURE)?;
        Some((l, r, latency))
    }

    /// The Level (Q-Link 1, the slider): the master fader.
    pub fn set_level(&mut self, db: f32) {
        self.set(StripId::Master, mx::VOLUME, db.min(self.max_db));
    }

    /// The master fader now, dB.
    #[cfg(test)]
    pub fn level(&self) -> f32 {
        self.mixer.param(StripId::Master, mx::VOLUME)
    }

    /// Follows the looper: whether the line input is open, and whether it
    /// is heard (Monitor). Cheap to call every period.
    pub fn follow(&mut self, line_open: bool, monitor_on: bool) {
        if line_open != self.line_open {
            self.line_open = line_open;
            self.set(
                LINE,
                mx::SOURCE,
                if line_open {
                    source::STEREO
                } else {
                    source::OFF
                },
            );
        }
        if monitor_on != self.monitor {
            self.monitor = monitor_on;
            self.set(
                LINE,
                mx::MONITOR,
                if monitor_on {
                    monitor::ON
                } else {
                    monitor::OFF
                },
            );
        }
    }

    /// The pad channels for the sampler's `render_routed` this period.
    pub fn pads(&mut self) -> Pads<'_> {
        Pads {
            mixer: &mut self.mixer,
            played: 0,
        }
    }

    /// The drums as the sampler made them: the pads that played (`played`,
    /// from [`Pads::played`]) summed from their channels' inputs into
    /// `left` and `right`, for the looper. Silence when none played.
    pub fn drums(&mut self, played: u32, left: &mut [f32; PERIOD], right: &mut [f32; PERIOD]) {
        left.fill(0.0);
        right.fill(0.0);
        for p in (0..PADS).filter(|p| played & (1 << p) != 0) {
            if let Some((l, r)) = self.mixer.pad_input(p) {
                for f in 0..PERIOD {
                    left[f] += l[f];
                    right[f] += r[f];
                }
            }
        }
    }

    /// One period: the synth and the loop into their tracks, the line input
    /// into the input (the pads are in already: [`MixHost::pads`]), the
    /// master out into `left` and `right`. Returns the output's peak and
    /// whether master safety caught anything.
    // The sounds come in as they are made; a struct of them would only be
    // taken apart again here.
    #[allow(clippy::too_many_arguments)]
    pub fn period(
        &mut self,
        ctx: &Ctx,
        synth: Stereo,
        looped: Stereo,
        audio: Stereo,
        line: Stereo,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> (f32, bool) {
        for (t, (l, r)) in [(SYNTH, synth), (LOOP, looped), (AUDIO, audio)] {
            if let Some((dl, dr)) = self.mixer.track_input(t) {
                dl[..PERIOD].copy_from_slice(l);
                dr[..PERIOD].copy_from_slice(r);
            }
        }
        // Each track's own instrument into its mixer track, timed while
        // any track has one.
        if self.instruments.in_use() != 0 {
            let t = Instant::now();
            self.instruments.render(ctx, PERIOD, &mut self.mixer);
            self.instrument_time = t.elapsed();
        } else {
            self.instruments.render(ctx, PERIOD, &mut self.mixer);
            self.instrument_time = Duration::ZERO;
        }
        if self.line_open {
            let (hl, hr) = self.mixer.hw_input();
            hl[..PERIOD].copy_from_slice(line.0);
            hr[..PERIOD].copy_from_slice(line.1);
        }
        self.modu.process(ctx, PERIOD, &mut self.mixer);
        self.mixer.process(ctx, left, right);
        let peak = left
            .iter()
            .chain(right.iter())
            .fold(0f32, |m, s| m.max(s.abs()));
        let overs = self.mixer.safety().stats().overs;
        let caught = overs != self.overs;
        self.overs = overs;
        (peak, caught)
    }

    /// The mixer panel opened on `view`, or closed (`None`).
    pub fn show(&mut self, view: Option<MixView>) {
        self.shown = view;
        if let Some(v) = view {
            self.last = v;
        }
        self.state_at = None;
    }

    /// The view the panel showed last (Track Mix at first), where the Mix
    /// tab opens it again.
    pub fn last(&self) -> MixView {
        self.last
    }

    /// The mixer view the panel shows, if it is open.
    pub fn shown(&self) -> Option<MixView> {
        self.shown
    }

    /// What the panel sent. Returns the master fader's new level when it
    /// moved, so the Play page's Level follows.
    pub fn action(&mut self, a: &MixAction) -> Option<f32> {
        match *a {
            MixAction::Set {
                strip: StripId::Master,
                id: mx::VOLUME,
                value,
            } => {
                let db = value.min(self.max_db);
                self.set(StripId::Master, mx::VOLUME, db);
                self.state_at = None;
                Some(db)
            }
            MixAction::Set {
                strip: StripId::Pad(p),
                id: mx::MUTE,
                value,
            } if p < PADS => {
                // The speakers on the Play page follow.
                self.play(PlayFx::PadMute {
                    pad: p,
                    mute: value >= 0.5,
                });
                None
            }
            MixAction::Set { strip, id, value } => {
                self.set(strip, id, value);
                None
            }
            MixAction::Select(s) => {
                self.selected = Some(s);
                self.state_at = None;
                None
            }
            MixAction::View(v) => {
                self.show(Some(v));
                None
            }
            // The insert pages come with the FX pages; the look is the
            // screen's.
            MixAction::OpenInsert { .. }
            | MixAction::AddInsert { .. }
            | MixAction::PickInsert { .. }
            | MixAction::MoveInsert { .. }
            | MixAction::Unpick
            | MixAction::Look(_) => None,
            // Freeze, Thaw and Print need the song: `MixHost::take_action`.
            MixAction::Freeze(_)
            | MixAction::Thaw(_)
            | MixAction::Print(_)
            | MixAction::CancelTake(_) => None,
        }
    }

    /// The panel's next state, while it shows and [`STATE_EVERY`] has
    /// passed (at once after a change).
    /// The tracks' strips as the mixer's Tracks view shows them: the
    /// engines' tracks, then each track's own instrument's while in use.
    pub fn track_strips(&self) -> Vec<StripId> {
        let used = self.instruments.in_use();
        (0..self.mixer.tracks())
            .filter(|&t| t < TRACKS || used & (1 << (t - TRACKS)) != 0)
            .map(StripId::Track)
            .collect()
    }

    pub fn poll_state(&mut self, now: Instant) -> Option<MixState> {
        let view = self.shown?;
        if self
            .state_at
            .is_some_and(|t| now.duration_since(t) < STATE_EVERY)
        {
            return None;
        }
        self.state_at = Some(now);
        // Tracks' own instruments' channels show only while in use; the
        // others aren't read.
        let used = self.instruments.in_use();
        let shown = |s: StripId| match s {
            StripId::Track(t) if t >= TRACKS => used & (1 << (t - TRACKS)) != 0,
            _ => true,
        };
        let mut st = MixState::from_mixer_where(&mut self.mixer, view, &name, &shown);
        st.counts[0] = TRACKS + used.count_ones() as usize;
        st.tracks = TRACKS;
        st.selected = self.selected;
        st.takes = true;
        for v in st.strips.iter_mut().chain(std::iter::once(&mut st.master)) {
            v.freezable = crate::takes::freezable(v.id);
            v.take = self.takes.view(v.id).map(|t| mpfree_mixer_ui::TakeView {
                print: t.print,
                done: t.done,
            });
        }
        Some(st)
    }

    /// The song as a take sees it: `tracks` in the project, at `bpm`, each
    /// playing what `now` says.
    fn song<'a>(
        instruments: &'a Instruments,
        tracks: usize,
        bpm: f64,
        now: &'a dyn Fn(usize) -> Option<mpfree_play::NowPlaying>,
        instrument: &'a dyn Fn(usize) -> u64,
    ) -> crate::takes::Song<'a> {
        crate::takes::Song {
            tracks,
            own: instruments.in_use(),
            bpm,
            now,
            instrument,
        }
    }

    /// The strip menu's Freeze, Thaw, Print and Cancel, with the song's
    /// `tracks` at `bpm` playing what `now` says. Returns the line for
    /// the screen (None for any other action).
    pub fn take_action(
        &mut self,
        a: &MixAction,
        tracks: usize,
        bpm: f64,
        now: &dyn Fn(usize) -> Option<mpfree_play::NowPlaying>,
    ) -> Option<String> {
        let inst = &self.instruments;
        let instrument = |k: usize| {
            inst.node(k)
                .map_or(0, |n| crate::takes::id_number(n.info().id))
        };
        let song = Self::song(inst, tracks, bpm, now, &instrument);
        let mut mem = TakeMem {
            builder: self.builder.as_mut(),
            here: &mut self.here,
        };
        let (kind, s) = match *a {
            MixAction::Freeze(s) => (crate::takes::Kind::Freeze, s),
            MixAction::Print(s) => (crate::takes::Kind::Print, s),
            MixAction::Thaw(s) => return Some(self.takes.thaw(s, &name(s), &mut self.mixer)),
            MixAction::CancelTake(s) => return self.takes.cancel(s, &mut self.mixer, &mut mem),
            _ => return None,
        };
        self.state_at = None;
        Some(
            self.takes
                .ask(kind, s, &name(s), &song, &self.mixer, &mut mem),
        )
    }

    /// A print the screen couldn't take, dropped off the audio thread.
    pub fn drop_printed(&mut self, p: Box<mpfree_fx::Printed>) {
        if let Some(b) = self.builder.as_mut() {
            b.drop_printed_later(p);
        }
    }

    /// Once a period, after [`MixHost::period`] with the same `ctx`: the
    /// take under way and the frozen strips (takes.rs), with the song's
    /// `tracks` playing what `now` says.
    pub fn takes_period(
        &mut self,
        ctx: &Ctx,
        tracks: usize,
        now: &dyn Fn(usize) -> Option<mpfree_play::NowPlaying>,
    ) -> Option<crate::takes::Said> {
        let inst = &self.instruments;
        let instrument = |k: usize| {
            inst.node(k)
                .map_or(0, |n| crate::takes::id_number(n.info().id))
        };
        let song = Self::song(inst, tracks, ctx.bpm, now, &instrument);
        let mut mem = TakeMem {
            builder: self.builder.as_mut(),
            here: &mut self.here,
        };
        let said = self.takes.period(ctx, &song, &mut self.mixer, &mut mem);
        if said.is_some() {
            self.state_at = None;
        }
        said
    }
}

/// The pad channels as the sampler sees them (`PadOut`): pad `p` of the
/// playing kit adds into pad channel `p`. It notes which pads played.
pub struct Pads<'a> {
    mixer: &'a mut Mixer,
    played: u32,
}

impl Pads<'_> {
    /// The pads that played this period, a bit each.
    pub fn played(&self) -> u32 {
        self.played
    }
}

impl PadOut for Pads<'_> {
    fn pad(&mut self, pad: usize) -> Option<(&mut [f32], &mut [f32])> {
        if pad >= PADS {
            return None;
        }
        self.played |= 1 << pad;
        self.mixer.pad_input(pad)
    }
}

/// The mixer's context for a period: the transport's tempo, position
/// (beats) and whether it rolls, from the period's first span.
pub fn ctx(rate: u32, frame: u64, bpm: f64, block: &mpfree_transport::Block) -> Ctx<'static> {
    let mut c = Ctx::new(rate, frame);
    c.bpm = bpm;
    if let Some(s) = block.spans().first() {
        c.bpm = s.bpm;
        c.playing = s.state.rolling();
        c.beats = s.tick_at(s.frame) / f64::from(mpfree_transport::PPQ);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;

    fn tone(f: f32, amp: f32, at: usize) -> [f32; PERIOD] {
        std::array::from_fn(|i| {
            amp * (std::f32::consts::TAU * f * (at + i) as f32 / RATE as f32).sin()
        })
    }

    /// Pad 1 plays `drums` (as the sampler would, into its channel), the
    /// line input `line`.
    fn run(h: &mut MixHost, periods: usize, drums: f32, line: f32) -> (f32, bool) {
        let z = [0f32; PERIOD];
        let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
        let (mut peak, mut caught) = (0f32, false);
        for p in 0..periods {
            let d = tone(110.0, drums, p * PERIOD);
            let i = tone(220.0, line, p * PERIOD);
            if drums > 0.0 {
                let mut pads = h.pads();
                let (pl, pr) = pads.pad(0).unwrap();
                pl[..PERIOD].iter_mut().zip(&d).for_each(|(o, x)| *o += x);
                pr[..PERIOD].iter_mut().zip(&d).for_each(|(o, x)| *o += x);
            }
            let ctx = Ctx::new(RATE, (p * PERIOD) as u64);
            let (pk, c) = h.period(&ctx, (&z, &z), (&z, &z), (&z, &z), (&i, &i), &mut l, &mut r);
            // The loudest of the last half: after the glides.
            if p >= periods / 2 {
                peak = peak.max(pk);
            }
            caught |= c;
        }
        (peak, caught)
    }

    /// UNDO on the mixer sets a pad's mute back to the pad's own, not to
    /// Drop drums' held over it.
    #[test]
    fn a_pads_mute_reads_as_its_own_under_drop_drums() {
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        h.play(PlayFx::PadMute { pad: 2, mute: true });
        h.play(PlayFx::Drop(0b11));
        assert_eq!(h.value(StripId::Pad(2), mx::MUTE), 1.0);
        assert_eq!(
            h.value(StripId::Pad(0), mx::MUTE),
            0.0,
            "dropped, not muted"
        );
    }

    #[test]
    fn the_engines_reach_the_master_at_the_level() {
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        let (peak, caught) = run(&mut h, 200, 0.5, 0.0);
        // 0.5 at -6 dB (about 0.25): pad 1 and the drums track at 0 dB, the master at the Level.
        assert!((0.24..0.26).contains(&peak), "{peak}");
        assert!(!caught);
        // The Level never goes above --gain-db.
        h.set_level(0.0);
        run(&mut h, 2, 0.5, 0.0);
        assert_eq!(h.level(), -6.0);
        h.set_level(-12.0);
        let (peak, _) = run(&mut h, 200, 0.5, 0.0);
        assert!((0.12..0.13).contains(&peak), "{peak}");
    }

    #[test]
    fn the_samplers_capture_point_hears_the_strip_picked() {
        use crate::sampler::RecSource;
        use mpfree_fx::CapturePoint;
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        assert!(h.rec_capture().is_none(), "free until a source is picked");
        let master = RecSource::Mix(StripId::Master, CapturePoint::AfterSafety);
        let pad1 = RecSource::Mix(StripId::Pad(0), CapturePoint::AfterFader);
        // One period of pad 1 playing 0.5; what came out, and what the
        // capture point took.
        let z = [0f32; PERIOD];
        let mut at = 0;
        let mut once = |h: &mut MixHost| {
            let d = tone(110.0, 0.5, at);
            {
                let mut pads = h.pads();
                let (pl, pr) = pads.pad(0).unwrap();
                pl[..PERIOD].copy_from_slice(&d);
                pr[..PERIOD].copy_from_slice(&d);
            }
            let ctx = Ctx::new(RATE, at as u64);
            let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
            h.period(&ctx, (&z, &z), (&z, &z), (&z, &z), (&z, &z), &mut l, &mut r);
            at += PERIOD;
            let peak = |x: &[f32]| x.iter().fold(0f32, |m, v| m.max(v.abs()));
            let cap = h.rec_capture().map(|(cl, cr, lat)| {
                assert_eq!((cl.len(), cr.len()), (PERIOD, PERIOD));
                (cl == l.as_slice() && cr == r.as_slice(), peak(cl), lat)
            });
            (peak(&l), cap)
        };
        assert!(h.set_rec_source(master));
        for _ in 0..200 {
            once(&mut h);
        }
        let (out, cap) = once(&mut h);
        let (same, _, lat) = cap.unwrap();
        assert!(same, "the master as heard is what comes out");
        assert!((0.24..0.26).contains(&out), "{out}");
        assert!(lat < RATE / 20, "{lat} frames late");
        // Pad 1's channel after its fader: before the Level.
        assert!(h.set_rec_source(pad1));
        for _ in 0..50 {
            once(&mut h);
        }
        let (_, cap) = once(&mut h);
        let (same, peak, _) = cap.unwrap();
        assert!(!same && (0.49..0.51).contains(&peak), "{peak}");
        // Back to the line input: the capture point is free again.
        assert!(h.set_rec_source(RecSource::Line));
        once(&mut h);
        assert!(h.rec_capture().is_none());
        // A strip the mixer doesn't have.
        let none = RecSource::Mix(StripId::Pad(40), CapturePoint::AfterFader);
        assert!(!h.set_rec_source(none));
        assert!(h.rec_capture().is_none());
    }

    #[test]
    fn the_line_input_is_heard_only_while_open_and_monitored() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        assert_eq!(run(&mut h, 100, 0.0, 0.5).0, 0.0, "closed");
        h.follow(true, false);
        assert_eq!(run(&mut h, 100, 0.0, 0.5).0, 0.0, "open, not monitored");
        h.follow(true, true);
        let (peak, _) = run(&mut h, 200, 0.0, 0.5);
        assert!((0.49..0.51).contains(&peak), "{peak}");
        h.follow(false, true);
        assert!(run(&mut h, 200, 0.0, 0.5).0 < 1e-6, "closed again");
    }

    #[test]
    fn master_safety_catches_what_would_clip() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        let (peak, caught) = run(&mut h, 400, 1.6, 0.0);
        assert!(caught);
        assert!(peak <= 1.0, "{peak}");
    }

    #[test]
    fn the_panel_gets_states_while_it_shows_and_drives_the_mixer() {
        let mut h = MixHost::new(RATE, -6.0, -6.0);
        let t0 = Instant::now();
        assert!(h.poll_state(t0).is_none(), "closed");
        h.show(Some(MixView::Tracks));
        run(&mut h, 20, 0.5, 0.0);
        let st = h.poll_state(t0).expect("at once when opened");
        assert_eq!(
            st.strips
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["Drums", "Synth", "Loop", "Audio"]
        );
        assert!(st.strips[0].peak[0] > 0.1);
        assert_eq!(st.master.volume_db, -6.0);
        assert!(
            h.poll_state(t0 + STATE_EVERY / 2).is_none(),
            "not again so soon"
        );
        assert!(h.poll_state(t0 + STATE_EVERY).is_some());
        // A fader on the panel; the master fader is the Level.
        assert_eq!(
            h.action(&MixAction::Set {
                strip: StripId::Track(DRUMS),
                id: mx::MUTE,
                value: 1.0
            }),
            None
        );
        assert_eq!(
            h.action(&MixAction::Set {
                strip: StripId::Master,
                id: mx::VOLUME,
                value: 3.0
            }),
            Some(-6.0)
        );
        assert_eq!(h.action(&MixAction::Select(StripId::Track(SYNTH))), None);
        run(&mut h, 200, 0.5, 0.0);
        let st = h.poll_state(t0 + STATE_EVERY * 3).unwrap();
        assert!(st.strips[0].mute);
        assert_eq!(st.selected, Some(StripId::Track(SYNTH)));
        // The meter held the mute's fade until that read; now it is silent.
        run(&mut h, 20, 0.5, 0.0);
        let st = h.poll_state(t0 + STATE_EVERY * 4).unwrap();
        assert_eq!(
            st.strip(StripId::Track(DRUMS)).unwrap().peak,
            [0.0; 2],
            "muted"
        );
        h.action(&MixAction::View(MixView::Inputs));
        let st = h.poll_state(t0 + STATE_EVERY * 4).unwrap();
        assert_eq!(
            (st.view, st.strips[0].name.as_str()),
            (MixView::Inputs, "Line in")
        );
        h.show(None);
        assert!(h.poll_state(t0 + STATE_EVERY * 10).is_none());
        assert_eq!(h.level(), -6.0);
    }

    #[test]
    fn the_pads_go_through_their_channels_into_the_drums_track() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        // The sampler's pads 1 and 3; pad 17 is not there.
        let mut pads = h.pads();
        pads.pad(0).unwrap().0[..PERIOD].fill(0.25);
        pads.pad(2).unwrap().0[..PERIOD].fill(0.5);
        assert!(pads.pad(PADS).is_none());
        let played = pads.played();
        assert_eq!(played, 0b101);
        let (mut l, mut r) = ([0f32; PERIOD], [0f32; PERIOD]);
        h.drums(played, &mut l, &mut r);
        assert_eq!(
            (l[0], l[PERIOD - 1], r[0]),
            (0.75, 0.75, 0.0),
            "what the looper records"
        );
        // Pad 1 muted on Pad Mix: the rest still reach the master through Drums.
        h.action(&MixAction::Set {
            strip: StripId::Pad(0),
            id: mx::MUTE,
            value: 1.0,
        });
        h.show(Some(MixView::Pads));
        let st = h.poll_state(Instant::now()).unwrap();
        assert_eq!(st.strips.len(), PADS);
        assert_eq!(
            (st.strips[0].name.as_str(), st.strips[0].output_label()),
            ("Pad 1", "Track 1".to_string())
        );
        // (Master safety's DC blocker is still forgetting the first block's
        // DC: about -100 dBFS.)
        let muted = run(&mut h, 200, 0.5, 0.0).0;
        assert!(muted < 1e-4, "pad 1 muted: {muted}");
        h.action(&MixAction::Set {
            strip: StripId::Pad(0),
            id: mx::MUTE,
            value: 0.0,
        });
        let (peak, _) = run(&mut h, 200, 0.5, 0.0);
        assert!((0.49..0.51).contains(&peak), "{peak}");
    }

    #[test]
    fn the_throws_and_the_xy_pad_play_perform_fx() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        let perform = |h: &MixHost, id: u16| {
            h.mixer
                .chain(StripId::Master)
                .and_then(|c| c.node(PERFORM_SLOT))
                .map(|n| n.param(id))
        };
        // The inserts go in once each strip has ducked out (about 10 ms).
        run(&mut h, 10, 0.0, 0.0);
        assert_eq!(h.mixer.edits_pending(), 0);
        assert_eq!(perform(&h, perf::MIX), Some(100.0));
        assert_eq!(h.play_levels().fx_mix, 1.0);
        h.play(PlayFx::Throw {
            fx: ThrowFx::Delay,
            down: true,
        });
        h.play(PlayFx::Xy {
            x: 0.0,
            y: 1.0,
            down: true,
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(
            perform(&h, perf::hold(9)),
            Some(1.0),
            "the echo throw's pad"
        );
        assert_eq!(perform(&h, perf::DJ_FILTER), Some(-100.0), "low-pass");
        assert_eq!(perform(&h, perf::ECHO), Some(100.0));
        h.play(PlayFx::Throw {
            fx: ThrowFx::Delay,
            down: false,
        });
        h.play(PlayFx::Xy {
            x: 0.5,
            y: 0.0,
            down: false,
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(perform(&h, perf::hold(9)), Some(0.0));
        assert_eq!(perform(&h, perf::DJ_FILTER), Some(0.0), "off in the middle");
        // Each throw holds a pad of its own.
        let all = [
            ThrowFx::Delay,
            ThrowFx::Reverb,
            ThrowFx::Stutter,
            ThrowFx::TapeStop,
            ThrowFx::FilterDip,
            ThrowFx::BeatRepeat,
            ThrowFx::VinylBrake,
        ];
        let mut pads: Vec<usize> = all.iter().map(|t| t.pad()).collect();
        pads.sort_unstable();
        pads.dedup();
        assert_eq!(pads.len(), all.len());
    }

    /// Jay after live test 3: several throws at once, a finger on each.
    #[test]
    fn two_throws_held_at_once_each_hold_their_pad() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        let held = |h: &MixHost, fx: ThrowFx| {
            h.mixer
                .chain(StripId::Master)
                .and_then(|c| c.node(PERFORM_SLOT))
                .map(|n| n.param(perf::hold(fx.pad())))
        };
        run(&mut h, 10, 0.0, 0.0);
        for fx in [ThrowFx::Delay, ThrowFx::Reverb] {
            h.play(PlayFx::Throw { fx, down: true });
        }
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(held(&h, ThrowFx::Delay), Some(1.0));
        assert_eq!(held(&h, ThrowFx::Reverb), Some(1.0));
        // The first finger lifts: the other throw keeps playing.
        h.play(PlayFx::Throw {
            fx: ThrowFx::Delay,
            down: false,
        });
        run(&mut h, 2, 0.0, 0.0);
        assert_eq!(held(&h, ThrowFx::Delay), Some(0.0));
        assert_eq!(held(&h, ThrowFx::Reverb), Some(1.0));
    }

    /// Runs periods (polling as the app does) until track 2's chain has
    /// `len` inserts, for at most 5 s.
    fn until_chain(h: &mut MixHost, len: usize) -> usize {
        let strip = StripId::Track(SYNTH);
        let give_up = Instant::now() + Duration::from_secs(5);
        while h.mixer.chain(strip).unwrap().len() != len && Instant::now() < give_up {
            assert_eq!(h.poll_built(), None);
            run(h, 1, 0.0, 0.0);
            // Built on the builder thread when there is one.
            std::thread::sleep(Duration::from_micros(200));
        }
        assert_eq!(h.poll_built(), None);
        h.mixer.chain(strip).unwrap().len()
    }

    /// Adds then removes a Drive on track 2 `times` times, checking the
    /// chain each time. Removes glide out over a few periods.
    fn add_and_remove(h: &mut MixHost, times: usize) {
        let strip = StripId::Track(SYNTH);
        let base = h.mixer.chain(strip).unwrap().len();
        for round in 0..times {
            h.add_insert(strip, "mpfree.fx.drive").unwrap();
            assert_eq!(until_chain(h, base + 1), base + 1, "add {round}");
            assert_eq!(h.inserts_building(), 0);
            h.mixer.edit(Edit::Remove { strip, at: base }).ok().unwrap();
            assert_eq!(until_chain(h, base), base, "remove {round}");
        }
    }

    #[test]
    fn chain_edits_keep_landing_after_many_removes() {
        // The app never took retired inserts back from the mixer, which
        // stops applying edits once 32 are waiting: the chain editor froze
        // after 32 removes. poll_built takes them back every period.
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        add_and_remove(&mut h, 40);
    }

    #[test]
    fn a_plugins_presets_are_asked_for_once_and_why_not_comes_back() {
        use crate::control::PresetNews;
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        // An engine has its presets built in: nothing is asked.
        h.ask_presets("mpfree.poly");
        assert!(h.poll_presets().is_none());
        // A plugin not in the folder: asked once, and why not comes back.
        h.ask_presets("clap:org.nope");
        h.ask_presets("clap:org.nope");
        match h.poll_presets() {
            Some(PresetNews::Listed { id, list: Err(e) }) => {
                assert_eq!(id, "clap:org.nope");
                assert!(!e.is_empty());
            }
            _ => panic!("expected the listing's error"),
        }
        assert!(h.poll_presets().is_none(), "asked once");
        assert!(h.plugin_presets("clap:org.nope").is_none());
        // Kept: not asked again.
        h.ask_presets("clap:org.nope");
        assert!(h.poll_presets().is_none());
        // A track without a plugin loads none.
        assert!(!h.load_plugin_preset(0, 0));
        assert!(h.poll_presets().is_none());
    }

    #[test]
    fn effects_built_off_the_audio_thread_land_in_their_chain() {
        let mut h = MixHost::new(RATE, 0.0, 0.0);
        h.build_off_thread().unwrap();
        add_and_remove(&mut h, 40);
        // An effect with no maker is said, and nothing lands.
        let strip = StripId::Track(SYNTH);
        let before = h.mixer.chain(strip).unwrap().len();
        h.add_insert(strip, "no.such.effect").unwrap();
        let give_up = Instant::now() + Duration::from_secs(5);
        let mut said = None;
        while said.is_none() && Instant::now() < give_up {
            said = h.poll_built();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(said.unwrap().contains("no maker"), "{said:?}");
        assert_eq!(h.mixer.chain(strip).unwrap().len(), before);
        assert_eq!(h.inserts_building(), 0);
    }

    #[test]
    fn the_context_follows_the_transport() {
        let block = mpfree_transport::Block::default();
        let c = ctx(RATE, 128, 97.0, &block);
        assert_eq!(
            (c.rate, c.frame, c.bpm, c.playing),
            (RATE, 128, 97.0, false)
        );
    }
}
