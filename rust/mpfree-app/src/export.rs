//! Export in the app (docs/dev/design/export.md): the mixdown, the stems
//! and the MIDI file from one pass over the song, on `mpfree-export`.
//! Stems by track (the default) need a replay of the range for each track
//! that shares its strip with others ([`track_stems`]): the drum kit's
//! tracks, the synth's, the audio tracks.
//!
//! The screen asks the sound loop for the song's facts ([`Facts`]: the
//! arrangement, the tempo, the tracks, the strips, what plays outside),
//! opens the Export panel with them, and on Start makes the plan, starts
//! the job and sends the pass ([`Run`]). The sound loop's [`Host`] sets
//! the transport to the range and plays it, hands each period's capture
//! points and notes to the pass, puts the transport back when it is over
//! and sends the pass back, for the job to finish.
//!
//! Listen plays it in real time, as the song always plays. Fast runs the
//! same periods as fast as the processor allows with the speakers quiet
//! (main.rs, [`Host::fast`]).

use std::path::Path;
use std::time::Duration;

use mpfree_export::mpfree_midi::msg::Msg;
use mpfree_export::{
    choose, folder_name, safe_name, MidiPlan, Mode, Only, Pass, Plan, Replay, Rig, Step, Stream,
    Why,
};
use mpfree_fx::{Mixer, StripId};
use mpfree_play::mpfree_param::{midi, Owner, ParamAddr};
use mpfree_play::{Event, Out};
use mpfree_time_ui::TimeAction;
use mpfree_transport::time::{frames_between, Tick};
use mpfree_transport::{Block, Command};

use crate::midi_out::{Chords, NoteIs};
use crate::mixing::{self, TRACKS};
use crate::PERIOD;

/// Notes and controls one period can hand the MIDI file; more are counted
/// as left out.
pub const TAP_ROOM: usize = 1024;
/// Tracks whose controllers, bend and pressure go in the MIDI file: as
/// many as MIDI out keeps them for.
const CONTROL_TRACKS: usize = mpfree_export::mpfree_midi::output::MAX_TRACKS;
/// A control no value has gone in the file for yet.
const UNTAKEN: i16 = i16::MIN;
/// Words of a track's control bits (`midi::COUNT` controls).
const CTL_WORDS: usize = (midi::COUNT as usize).div_ceil(64);
/// Periods the transport may take to start before the export gives up
/// (about a second).
const START_PERIODS: u32 = 400;
/// Capture slots an export may use: every one below the looper's
/// (mixing.rs), Freeze and Print's (takes.rs) and the sampler's.
pub const MAX_STREAMS: usize = crate::mixing::LOOP_CAPTURE;
/// In a fast export, how long a period waits for its lanes on the other
/// cores before going on without them (as the card's deadline would).
pub const FAST_LANE_WAIT: Duration = Duration::from_millis(250);
/// In a fast export, how long the loop waits for the files to make room
/// before the pass fails as stalled.
const FAST_STALL: Duration = Duration::from_secs(5);
/// In a fast export, the loop rests a millisecond every this many periods
/// (about 0.2 s of sound), so whatever shares its core still runs.
const FAST_REST_EVERY: u64 = 64;

/// What the screen needs to know about the song to offer an export and
/// make its plan, gathered by the sound loop when Export is pressed.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts {
    pub rate: u32,
    /// Where the arrangement ends (0: there is none).
    pub arrangement_end: Tick,
    /// The song position now.
    pub position: Tick,
    pub meter: (u8, u8),
    pub bar_ticks: Tick,
    /// Each tempo and the tick it starts at.
    pub tempos: Vec<(Tick, f64)>,
    /// The sequencer's tracks' names, by index.
    pub tracks: Vec<String>,
    /// The mixer strips that can have a stem, with their names (Stems
    /// by Channel).
    pub stems: Vec<(String, StripId)>,
    /// Stems by Track: where each sounding track's stem comes from.
    pub by_track: Vec<TrackStem>,
    /// Song mode is on now (the audio tracks play only in song mode).
    pub song_on: bool,
    pub rig: Rig,
}

/// What a sequencer track plays into, for its stem by track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plays {
    /// Its own instrument, on its own strip.
    Own,
    /// The pads: the drum kit or the synth, whichever has them, on a strip
    /// other tracks share.
    Pads,
    /// Outside gear over MIDI: nothing here to record.
    Outside,
    /// Nothing: muted, or no notes in any clip.
    Nothing,
}

/// Where a stem by track comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackStem {
    pub name: String,
    pub strip: StripId,
    /// Some: the range plays again with only this track playing into
    /// `strip`, which it shares. None: the first pass hears it alone
    /// there.
    pub only: Option<Only>,
    /// An audio track's (heard only when the song mode plays).
    pub audio: bool,
}

/// Stems by Track: one per track that sounds, audio tracks among them,
/// in track order, then the other channels (the looper, the returns:
/// `rest`).
/// - Each track that sounds plays into a strip, `strip(track)` with
///   whether it is frozen: its own instrument into its track's channel
///   (`MixHost::track_channel`), the pads into their kit's (or the
///   synth's) strip.
/// - A track alone on its strip: the first pass hears it alone (a track's
///   own instrument so even while frozen, named after the track).
/// - Tracks that share a strip each get a replay, unless the strip is
///   frozen (it plays its take whatever the tracks do: one stem for the
///   strip, named after it). Track 1's own instrument shares the Drums
///   strip with the tracks on kit 1 (sampler R3 part 4b).
/// - An audio track that sounds (`audio`: its place, its strip and its
///   name) is alone on its own channel (the audio tracks rethink, phase
///   1b): the first pass hears it.
/// - A track on outside gear, a muted one and one with no notes get none.
pub fn track_stems(
    tracks: &[(String, Plays)],
    strip: &dyn Fn(usize) -> (StripId, bool),
    audio: &[(usize, StripId, String)],
    rest: &[(String, StripId)],
) -> Vec<TrackStem> {
    let stem = |name: &str, strip, only, audio| TrackStem {
        name: name.to_string(),
        strip,
        only,
        audio,
    };
    // In track order: each track's stem, by its place.
    let mut placed: Vec<(usize, TrackStem)> = Vec::new();
    // Each track that sounds: its strip, whether frozen, and whether it is
    // its own instrument.
    let sounding: Vec<(usize, StripId, bool, bool)> = tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| matches!(t.1, Plays::Pads | Plays::Own))
        .map(|(k, t)| {
            let (s, frozen) = strip(k);
            (k, s, frozen, t.1 == Plays::Own)
        })
        .collect();
    let sharing = |s: StripId| sounding.iter().filter(|p| p.1 == s).count();
    // A track's own instrument alone on its strip: its stem, frozen or not.
    let alone_own = |p: &(usize, StripId, bool, bool)| p.3 && sharing(p.1) == 1;
    for &p in &sounding {
        let (k, s, frozen, _) = p;
        let name = &tracks[k].0;
        if alone_own(&p) {
            placed.push((k, stem(name, s, None, false)));
        } else if frozen {
        } else if sharing(s) == 1 {
            placed.push((k, stem(name, s, None, false)));
        } else {
            placed.push((k, stem(name, s, Some(Only::Track(k as u16)), false)));
        }
    }
    for (k, s, name) in audio {
        placed.push((*k, stem(name, *s, None, true)));
    }
    placed.sort_by_key(|(k, _)| *k);
    let mut out: Vec<TrackStem> = placed.into_iter().map(|(_, s)| s).collect();
    // Each frozen strip the tracks play into, once.
    let mut frozen: Vec<StripId> = Vec::new();
    for p in sounding.iter().filter(|p| p.2 && !alone_own(p)) {
        if !frozen.contains(&p.1) {
            frozen.push(p.1);
            out.push(stem(&mixing::name(p.1), p.1, None, false));
        }
    }
    for (name, strip) in rest {
        out.push(stem(name, *strip, None, false));
    }
    out
}

/// While a replay plays (`only`, [`Host::only`]): whether track `track`'s
/// note (a note-on, note-off or pressure; never a panic or a lock, which
/// play as in the first pass) is kept quiet. Only the other tracks on the
/// pads are: the replay's strip is theirs, and everything else plays as
/// before, so an effect keyed from another strip hears what it heard.
/// `on_pads`: the track plays the pads (not outside gear), or it is track
/// 1 playing its own instrument into the Drums strip, which the tracks on
/// kit 1 share (sampler R3 part 4b).
pub fn quiets(only: Option<Only>, track: u16, on_pads: bool, event: Event) -> bool {
    let note = matches!(
        event,
        Event::NoteOn { .. } | Event::NoteOff { .. } | Event::Pressure { .. }
    );
    match only {
        Some(Only::Track(k)) => note && on_pads && track != k,
        None => false,
    }
}

/// The range an export plays, in song ticks, and whether it plays the
/// arrangement (song mode) or the clips as they are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: Tick,
    pub end: Tick,
    /// Some(true): song mode on for the pass (the arrangement); None: as
    /// it is.
    pub song: Option<bool>,
}

/// The sequencer's tracks' names, by index ("Track 3" for one with none).
pub fn track_names(time: &crate::time::TimeHost) -> Vec<String> {
    time.project()
        .tracks
        .iter()
        .enumerate()
        .map(|(k, t)| {
            if t.name.trim().is_empty() {
                format!("Track {}", k + 1)
            } else {
                t.name.clone()
            }
        })
        .collect()
}

impl Facts {
    /// The facts the time engine and the mixer give, with what plays
    /// outside (`rig`, which the sound loop fills in from MIDI out, CV out,
    /// the line input and the plugins; `Rig::default()` where none of
    /// those exist, as in the smoke tests' session).
    ///
    /// For stems by track: `outside`, the tracks on outside gear (MIDI
    /// Out); `pads`, the strip each track on the pads plays into (its
    /// kit's or the synth's); `audio`, the audio tracks that sound (their
    /// place, strip and name: `audiotracks::TrackHost::heard`).
    pub fn gather(
        time: &crate::time::TimeHost,
        mix: &mixing::MixHost,
        rig: Rig,
        outside: &dyn Fn(usize) -> bool,
        pads: &dyn Fn(usize) -> StripId,
        audio: &[(usize, StripId, String)],
    ) -> Facts {
        let info = time.info();
        let project = time.project();
        let tracks = track_names(time);
        let stems = stem_strips(mix, &tracks);
        let plays: Vec<(String, Plays)> = project
            .tracks
            .iter()
            .enumerate()
            .map(|(k, t)| {
                let sounds = !t.mute && t.slots.clips().any(|c| !c.notes.is_empty());
                let plays = if outside(k) {
                    Plays::Outside
                } else if !sounds {
                    Plays::Nothing
                } else if mix.instruments.holds(k) {
                    Plays::Own
                } else {
                    Plays::Pads
                };
                (tracks[k].clone(), plays)
            })
            .collect();
        let rest: Vec<(String, StripId)> = stems
            .iter()
            .filter(|(_, s)| matches!(s, StripId::Track(mixing::LOOP) | StripId::Return(_)))
            .cloned()
            .collect();
        let frozen = |s: StripId| mix.mixer.frozen(s).is_some();
        // Each track's strip: its own instrument's channel, or where its
        // pads play.
        let strip = |k: usize| {
            if mix.instruments.holds(k) {
                StripId::Track(mix.track_channel(k))
            } else {
                pads(k)
            }
        };
        let by_track = track_stems(&plays, &|k| (strip(k), frozen(strip(k))), audio, &rest);
        let arrangement_end = if project.arrangement.is_empty() {
            0
        } else {
            project.arrangement.end()
        };
        Facts {
            rate: crate::RATE,
            arrangement_end,
            position: info.position,
            meter: (info.meter.beats, info.meter.unit),
            bar_ticks: info.meter.bar_ticks(),
            // A ramp goes in as a tempo on each of its 1/16 steps.
            tempos: time.tempo_map().changes().collect(),
            stems,
            by_track,
            song_on: time.song_on(),
            tracks,
            rig,
        }
    }

    /// Stems by track for a range: how many, and how many of them need a
    /// replay (the audio tracks only when the song mode plays).
    pub fn track_count(&self, range: Range) -> (usize, usize) {
        let song = range.song.unwrap_or(self.song_on);
        let heard = || self.by_track.iter().filter(|t| song || !t.audio);
        let first = heard().filter(|t| t.only.is_none()).count();
        let replays = heard().filter(|t| t.only.is_some()).count();
        (first.min(MAX_STREAMS - 1) + replays, replays)
    }

    /// The panel's setup, and how the export would play now.
    pub fn setup(
        &self,
        pref: Mode,
        format: mpfree_browser_ui::ExportFormat,
        folder: &str,
    ) -> (mpfree_browser_ui::ExportSetup, Mode, Why) {
        let (mode, why) = choose(pref, &self.rig);
        let arrangement = (self.arrangement_end > 0).then(|| {
            let frames = self.frames(0, self.arrangement_end);
            let bars = (self.arrangement_end + self.bar_ticks - 1) / self.bar_ticks.max(1);
            format!(
                "{}, {bars} bar{}",
                mpfree_export::clock(frames, self.rate),
                if bars == 1 { "" } else { "s" }
            )
        });
        let setup = mpfree_browser_ui::ExportSetup {
            arrangement,
            from_bar: (self.bar_start() / self.bar_ticks.max(1)) as u32 + 1,
            mode: mode_words(mode, &why),
            stems: self.stems.len().min(MAX_STREAMS - 1),
            tracks_arrangement: self
                .track_count(self.range(mpfree_browser_ui::ExportRange::Arrangement)),
            tracks_bars: self.track_count(self.range(mpfree_browser_ui::ExportRange::Bars(1))),
            folder: format!("Recordings / {folder}"),
            format,
        };
        (setup, mode, why)
    }

    /// The start of the bar the playhead is in.
    fn bar_start(&self) -> Tick {
        let b = self.bar_ticks.max(1);
        self.position.max(0) - self.position.max(0) % b
    }

    /// The range a choice plays.
    pub fn range(&self, r: mpfree_browser_ui::ExportRange) -> Range {
        match r {
            mpfree_browser_ui::ExportRange::Arrangement if self.arrangement_end > 0 => Range {
                start: 0,
                end: self.arrangement_end,
                song: Some(true),
            },
            mpfree_browser_ui::ExportRange::Arrangement => self.range(
                mpfree_browser_ui::ExportRange::Bars(mpfree_browser_ui::export::DEFAULT_BARS),
            ),
            mpfree_browser_ui::ExportRange::Bars(n) => {
                let start = self.bar_start();
                Range {
                    start,
                    end: start + Tick::from(n.max(1)) * self.bar_ticks.max(1),
                    song: None,
                }
            }
        }
    }

    /// Frames from tick `a` to tick `b` on the tempo map.
    pub fn frames(&self, a: Tick, b: Tick) -> u64 {
        let mut total = 0.0;
        for (k, &(start, bpm)) in self.tempos.iter().enumerate() {
            let seg_a = if k == 0 { Tick::MIN } else { start };
            let seg_b = self.tempos.get(k + 1).map_or(Tick::MAX, |n| n.0);
            let (x, y) = (a.max(seg_a), b.min(seg_b));
            if y > x {
                total += frames_between(x, y, bpm, self.rate);
            }
        }
        total.round().max(0.0) as u64
    }

    /// The tempo map from `start` on, as the MIDI file has it (ticks from
    /// the range's start).
    fn tempos_from(&self, start: Tick) -> Vec<(u64, f64)> {
        let mut out: Vec<(u64, f64)> = Vec::new();
        for &(t, bpm) in &self.tempos {
            let at = (t - start).max(0) as u64;
            match out.last_mut() {
                Some(last) if last.0 == at => last.1 = bpm,
                _ => out.push((at, bpm)),
            }
        }
        if out.is_empty() {
            out.push((0, 120.0));
        }
        out
    }

    /// The plan for a choice: its streams, its MIDI file and its range.
    /// `folder` is the export's new folder, `song` the project's name.
    pub fn plan(
        &self,
        choice: &mpfree_browser_ui::ExportChoice,
        folder: &Path,
        song: &str,
        mode: Mode,
    ) -> (Plan, Range) {
        let range = self.range(choice.range);
        let mut streams = Vec::new();
        let mut replays = Vec::new();
        if choice.mixdown {
            streams.push(Stream::mixdown());
        }
        match (choice.stems, choice.by) {
            (false, _) => {}
            (true, mpfree_browser_ui::StemsBy::Channel) => {
                for (name, strip) in &self.stems {
                    if streams.len() < MAX_STREAMS {
                        streams.push(Stream::stem(name, *strip));
                    }
                }
            }
            (true, mpfree_browser_ui::StemsBy::Track) => {
                let song = range.song.unwrap_or(self.song_on);
                for t in self.by_track.iter().filter(|t| song || !t.audio) {
                    let stream = Stream::stem(&t.name, t.strip);
                    match t.only {
                        Some(only) => replays.push(Replay { stream, only }),
                        None if streams.len() < MAX_STREAMS => streams.push(stream),
                        None => {}
                    }
                }
            }
        }
        let midi = choice.midi.then(|| MidiPlan {
            tracks: self.tracks.clone(),
            tempos: self.tempos_from(range.start),
            meters: vec![(0, self.meter.0, self.meter.1)],
            end: (range.end - range.start).max(0) as u64,
            room: MidiPlan::ROOM,
        });
        let plan = Plan {
            folder: folder.to_path_buf(),
            song: song.to_string(),
            streams,
            replays,
            midi,
            mode,
            rate: self.rate,
            format: wav_format(choice.format),
            length: Some(self.frames(range.start, range.end)),
            first_slot: 0,
        };
        (plan, range)
    }
}

/// Prefs' Export file and rate, as the Export panel starts from them.
pub fn panel_format(bits: mpfree_fx::Bits, rate: u32) -> mpfree_browser_ui::ExportFormat {
    use mpfree_browser_ui::ExportBits as B;
    mpfree_browser_ui::ExportFormat {
        bits: match bits {
            mpfree_fx::Bits::I24 => B::I24,
            mpfree_fx::Bits::I16 => B::I16,
            mpfree_fx::Bits::F32 => B::F32,
        },
        rate,
    }
}

/// The panel's format, as the export writes it.
pub fn wav_format(f: mpfree_browser_ui::ExportFormat) -> mpfree_fx::WavFormat {
    use mpfree_browser_ui::ExportBits as B;
    mpfree_fx::WavFormat {
        bits: match f.bits {
            B::I24 => mpfree_fx::Bits::I24,
            B::I16 => mpfree_fx::Bits::I16,
            B::F32 => mpfree_fx::Bits::F32,
        },
        rate: f.rate,
    }
}

/// The Plays line on the panel: the mode, and why when it isn't the
/// preference.
pub fn mode_words(mode: Mode, why: &Why) -> String {
    match (mode, why) {
        (Mode::Fast, _) => {
            "Fast, quiet: faster than real time, the speakers quiet (Prefs > Export)".into()
        }
        (Mode::Listen, Why::Chosen) => {
            "Listen: it plays in real time and you hear it (Prefs > Export)".into()
        }
        (Mode::Listen, w) => format!(
            "Listen: it plays in real time and you hear it. {}",
            w.words(mode)
                .trim_start_matches("Listening: ")
                .chars()
                .enumerate()
                .map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { c })
                .collect::<String>()
        ),
    }
}

/// The new folder's name in Recordings: the project's and the time,
/// "Night drive 2026-11-14 01.32" (" 2" ... if taken).
pub fn new_folder_name(recordings: &Path, project: &str, now_utc: u64) -> String {
    let (y, mo, d, h, mi) = mpfree_library::project::civil(now_utc);
    let base = folder_name(project, &format!("{y:04}-{mo:02}-{d:02} {h:02}.{mi:02}"));
    mpfree_library::unique_folder_name(&base, |n| recordings.join(n).exists())
}

/// The project's name for the files ("Untitled" before its first save).
pub fn song_name(project: Option<&str>) -> String {
    safe_name(project.unwrap_or("Untitled"))
}

/// A pass on its way to the sound loop and back.
pub struct Run {
    pub pass: Pass,
    pub range: Range,
    /// Why the sound loop stopped it itself, when it did (the transport
    /// stopped, say).
    pub said: Option<String>,
}

impl std::fmt::Debug for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Run")
    }
}

/// Never equal: each pass is its own (for the screen queue's tests).
impl PartialEq for Run {
    fn eq(&self, _: &Run) -> bool {
        false
    }
}

/// The notes the player sends in a period, as MIDI Out would send them,
/// for the MIDI file: filled from `SeqPads` while an export runs, handed
/// to the pass after the period. A track's controllers, bend and pressure
/// join them from the mod engine ([`Tap::control`]), thinned as MIDI Out
/// thins them. Fixed room; never allocates once made.
pub struct Tap {
    on: bool,
    chords: Box<Chords>,
    notes: Vec<(u64, u16, Msg)>,
    /// This period's controls for the file: (frame, track, control,
    /// value), the value in its control's range.
    controls: Vec<(u64, u16, u16, i16)>,
    /// What the file has of each control (`CONTROL_TRACKS` ×
    /// `midi::COUNT`, [`UNTAKEN`] before the first), so a value goes in
    /// only when it changes; kept by [`Host::notes`], which puts them in.
    in_file: Box<[i16]>,
    /// Each control's value given again after its period's first: it
    /// goes at the next period's start ([`Tap::period`]).
    given: Box<[i16]>,
    /// Bits by track ([`CTL_WORDS`] words each): a control had its value
    /// this period, and one waits for the next.
    sent: Box<[u64]>,
    waiting: Box<[u64]>,
    /// Every control with a value as the export starts (track, control,
    /// value): MIDI Out sends them again at the song's start, so the file
    /// has them at the range's start.
    at_start: Vec<(u16, u16, i16)>,
    /// Messages past [`TAP_ROOM`] in a period, and controller values that
    /// weren't a number, left out (since the export started).
    pub dropped: u64,
    /// Controller, bend and pressure values outside their range, put in
    /// the file as the nearest (since the export started).
    pub out_of_range: u64,
}

impl Default for Tap {
    fn default() -> Self {
        Self::new()
    }
}

impl Tap {
    pub fn new() -> Tap {
        let all = CONTROL_TRACKS * usize::from(midi::COUNT);
        Tap {
            on: false,
            chords: Box::default(),
            notes: Vec::with_capacity(TAP_ROOM),
            controls: Vec::with_capacity(TAP_ROOM),
            in_file: vec![UNTAKEN; all].into_boxed_slice(),
            given: vec![UNTAKEN; all].into_boxed_slice(),
            sent: vec![0; CONTROL_TRACKS * CTL_WORDS].into_boxed_slice(),
            waiting: vec![0; CONTROL_TRACKS * CTL_WORDS].into_boxed_slice(),
            at_start: Vec::with_capacity(all),
            dropped: 0,
            out_of_range: 0,
        }
    }

    /// One of a track's events, as MIDI Out reads it (`is`: what its note
    /// number is). Only while an export runs.
    pub fn send(&mut self, o: &Out, is: NoteIs) {
        if !self.on || o.event == Event::AllOff {
            return;
        }
        let (notes, dropped) = (&mut self.notes, &mut self.dropped);
        self.chords
            .messages(usize::from(o.track), o.event, is, |m| {
                if notes.len() < TAP_ROOM {
                    notes.push((o.frame, o.track, m));
                } else {
                    *dropped += 1;
                }
            });
    }

    /// At each period's start, before [`Tap::control`]: each control may
    /// have a value again, and the values that waited from the last
    /// period go in on `frame`, the period's first (as MIDI Out sends
    /// them). Never allocates.
    pub fn period(&mut self, frame: u64) {
        if !self.on {
            return;
        }
        for i in 0..self.sent.len() {
            let mut bits = std::mem::take(&mut self.waiting[i]);
            // A waiting value counts as this period's.
            self.sent[i] = bits;
            while bits != 0 {
                let b = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let (t, id) = (i / CTL_WORDS, (i % CTL_WORDS) * 64 + b);
                let k = t * usize::from(midi::COUNT) + id;
                self.put(frame, t as u16, id as u16, self.given[k]);
            }
        }
    }

    /// A track's controller, bend or pressure as the mod engine moved it
    /// (`Owner::Midi`: a step's lock, a lane, a song lane, an LFO, a
    /// macro, a learned knob; `ModMix::take_midi_values`), at `frame`,
    /// before CC scale and CC smooth shape it for the gear. Only while an
    /// export runs. Thinned as `MidiOut::control` thins them: a control's
    /// first value in a period goes on its frame, a later one waits for
    /// the next period's start (the last given), and only a change goes in
    /// the file (the 7-bit value for a controller), on channel 1 as the
    /// notes are. A value outside the control's range goes in as the
    /// nearest and is counted. Never allocates.
    pub fn control(&mut self, frame: u64, addr: ParamAddr, value: f32) {
        let Owner::Midi(t) = addr.owner else { return };
        if !self.on {
            return;
        }
        let Some((lo, hi)) = midi::range(addr.param) else {
            return;
        };
        let (t, id) = (usize::from(t), usize::from(addr.param));
        if t >= CONTROL_TRACKS {
            return;
        }
        if !value.is_finite() {
            self.dropped += 1;
            return;
        }
        let r = value.round();
        if r < f32::from(lo) || r > f32::from(hi) {
            self.out_of_range += 1;
        }
        let v = (r as i32).clamp(lo.into(), hi.into()) as i16;
        let (w, bit) = (t * CTL_WORDS + id / 64, 1u64 << (id % 64));
        if self.sent[w] & bit != 0 {
            self.given[t * usize::from(midi::COUNT) + id] = v;
            self.waiting[w] |= bit;
            return;
        }
        self.sent[w] |= bit;
        self.put(frame, t as u16, id as u16, v);
    }

    /// A clip's lock, lane value or their end on its track's own MIDI out
    /// control (`mpfree_play::is_gear_control`) that MIDI out sent ahead
    /// for slow gear (sequencer S5, `SeqPads::control_ahead`), in its own
    /// period: it never reaches the mod engine, so the file takes it here,
    /// on its frame, through [`Tap::control`]. An unlock is the control's
    /// own value, as it went out: `base` (the mod engine's), else the
    /// control's default. Never allocates.
    pub fn gear_control(&mut self, o: &Out, base: impl FnOnce(ParamAddr) -> Option<f32>) {
        let (addr, value) = match o.event {
            Event::Lock { addr, value } | Event::Lane { addr, value } => (addr, Some(value)),
            Event::Unlock { addr } => (addr, None),
            _ => return,
        };
        if !self.on {
            return;
        }
        let Some(own) = value
            .or_else(|| base(addr))
            .or_else(|| midi::default(addr.param).map(f32::from))
        else {
            return;
        };
        self.control(o.frame, addr, own);
    }

    /// A control's value for the file, unless the file has it already.
    fn put(&mut self, frame: u64, t: u16, id: u16, v: i16) {
        let k = usize::from(t) * usize::from(midi::COUNT) + usize::from(id);
        if self.in_file.get(k) == Some(&v) {
            return;
        }
        if self.controls.len() < TAP_ROOM {
            self.controls.push((frame, t, id, v));
        } else {
            self.dropped += 1;
        }
    }

    /// Control `id` of track `t` has `value` as the export starts (MIDI
    /// Out's, `MidiOut::value`): the file has it at the range's start.
    /// Call after [`Host::begin`].
    pub fn at_start(&mut self, t: u16, id: u16, value: i16) {
        let Some((lo, hi)) = midi::range(id) else {
            return;
        };
        if self.on
            && usize::from(t) < CONTROL_TRACKS
            && self.at_start.len() < self.at_start.capacity()
        {
            self.at_start.push((t, id, value.clamp(lo, hi)));
        }
    }

    /// A tap that takes notes at once (the real-time safety tests).
    #[cfg(test)]
    pub fn on() -> Tap {
        let mut t = Tap::new();
        t.start();
        t
    }

    /// The notes and controls taken this period, then empty (the
    /// real-time safety tests: what [`Host::notes`] hands the pass).
    #[cfg(test)]
    pub fn take(&mut self) -> usize {
        let n = self.notes.len() + self.controls.len();
        self.notes.clear();
        self.controls.clear();
        n
    }

    fn start(&mut self) {
        self.on = true;
        self.chords.clear();
        self.notes.clear();
        self.controls.clear();
        self.in_file.fill(UNTAKEN);
        self.sent.fill(0);
        self.waiting.fill(0);
        self.at_start.clear();
        self.dropped = 0;
        self.out_of_range = 0;
    }

    fn stop(&mut self) {
        self.on = false;
        self.notes.clear();
        self.controls.clear();
    }
}

/// Control `id` at `v` as a message on channel 1 (the file puts each
/// track on its own).
fn control_msg(id: u16, v: i16) -> Option<Msg> {
    use mpfree_play::mpfree_param::midi::Midi;
    Some(match midi::of(id)? {
        Midi::Cc(cc) => Msg::Cc {
            ch: 0,
            cc,
            value: v.clamp(0, 127) as u8,
        },
        Midi::Bend => Msg::PitchBend {
            ch: 0,
            value: v.clamp(-8192, 8191),
        },
        Midi::Pressure => Msg::ChanPressure {
            ch: 0,
            value: v.clamp(0, 127) as u8,
        },
    })
}

/// What the transport was before the export moved it.
#[derive(Clone, Copy, Debug)]
struct Before {
    song: bool,
    loop_range: Option<(Tick, Tick)>,
    position: Tick,
}

struct Running {
    run: Box<Run>,
    before: Before,
    /// The transport has started rolling from the range's start: the
    /// pass's frames are the song's from here.
    rolled: bool,
    /// The range has played (the tail plays from here).
    ended: bool,
    /// The transport is told to stop on the range's last frame, so the
    /// clips don't play on into the tail.
    stopping: bool,
    waited: u32,
    /// The controls the tracks had as the export started are in the file.
    started: bool,
}

/// The sound loop's side (see the module docs).
#[derive(Default)]
pub struct Host {
    pub tap: Tap,
    now: Option<Running>,
}

impl Host {
    pub fn new() -> Host {
        Host::default()
    }

    pub fn running(&self) -> bool {
        self.now.is_some()
    }

    /// A replay's track while one plays: the sound loop keeps the other
    /// tracks on its strip quiet ([`quiets`]). An audio track never needs
    /// one: it is alone on its own strip.
    pub fn only(&self) -> Option<Only> {
        self.now.as_ref().and_then(|r| r.run.pass.only())
    }

    /// The pass that just began has others before or after it (replays):
    /// the arps' random notes start again from their seed (main.rs, which
    /// holds them).
    pub fn replays(&self) -> bool {
        self.now.as_ref().is_some_and(|r| r.run.pass.passes() > 1)
    }

    /// Fast mode runs now: the loop runs periods back to back with the
    /// speakers quiet (main.rs).
    pub fn fast(&self) -> bool {
        self.now
            .as_ref()
            .is_some_and(|r| r.run.pass.mode() == Mode::Fast)
    }

    /// In a fast export, in place of the card's write: waits until the
    /// files have room for the next period (they never lose one), and
    /// rests now and then (`i`: the loop's period count). A stall longer
    /// than [`FAST_STALL`] lets the loop go on, and the pass then fails.
    pub fn wait_for_room(&self, i: u64) {
        let Some(r) = self.now.as_ref() else {
            return;
        };
        r.run.pass.wait_for_room(PERIOD, FAST_STALL);
        if i.is_multiple_of(FAST_REST_EVERY) {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The pass arrives: its capture points go on the mixer, and the
    /// transport goes to the range's start (no loop; song mode on for the
    /// arrangement) and plays. With replays, every pass starts the same:
    /// the mixer's effects cleared (delay lines, free-running LFOs) and
    /// the chance rolls anchored to the start. The pass comes back at once
    /// if the mixer refused (or one already runs).
    pub fn begin(
        &mut self,
        mut run: Box<Run>,
        mixer: &mut Mixer,
        time: &mut crate::time::TimeHost,
    ) -> Result<(), Box<Run>> {
        if self.now.is_some() {
            run.pass.cancel();
            run.said = Some("another export is running".into());
            return Err(run);
        }
        let replays = run.pass.passes() > 1;
        if replays {
            mixer.reset();
        }
        if !run.pass.attach(mixer) {
            return Err(run);
        }
        time.anchor_dice(replays);
        let info = time.info();
        let before = Before {
            song: time.song_on(),
            // Following another device's position, the loop it holds
            // (set back, it is held again).
            loop_range: info.loop_range,
            position: info.position,
        };
        if info.state.rolling() {
            // Stop, then stop again: the sound cut, so nothing from before
            // rings into the export.
            time.command(Command::Stop);
            time.command(Command::Stop);
        }
        time.command(Command::SetLoop(None));
        if let Some(s) = run.range.song {
            if s != before.song {
                let _ = time.act(TimeAction::SongMode(s));
            }
        }
        time.command(Command::Locate(run.range.start));
        time.command(Command::Play);
        self.tap.start();
        self.now = Some(Running {
            run,
            before,
            rolled: false,
            ended: false,
            stopping: false,
            waited: 0,
            started: false,
        });
        Ok(())
    }

    /// After the clips and arps played this period, and the mixer: their
    /// notes and the tracks' controls go to the MIDI file, at their ticks
    /// from the range's start. The controls every track had as the export
    /// started go first, at the range's start; after that a control goes
    /// in while the song rolls, and only when it changes.
    pub fn notes(&mut self, block: &Block) {
        let Some(r) = self.now.as_mut() else {
            return;
        };
        r.see_roll(block);
        let tap = &mut self.tap;
        if r.rolled {
            let count = usize::from(midi::COUNT);
            if !r.started {
                r.started = true;
                for &(t, id, v) in &tap.at_start {
                    if let Some(m) = control_msg(id, v) {
                        tap.in_file[usize::from(t) * count + usize::from(id)] = v;
                        r.run.pass.note(0, t, m);
                    }
                }
            }
            let start = r.run.range.start as f64;
            let span_of = |frame: u64| {
                block
                    .spans()
                    .iter()
                    .find(|s| frame >= s.frame && frame < s.frame + u64::from(s.len))
            };
            for &(frame, track, msg) in &tap.notes {
                if let Some(s) = span_of(frame) {
                    let tick = (s.tick_at(frame) - start).round().max(0.0) as u64;
                    r.run.pass.note(tick, track, msg);
                }
            }
            for &(frame, t, id, v) in &tap.controls {
                // Before the song rolls and after it stops (an LFO in the
                // tail), nothing: the file ends with the range.
                let Some(s) = span_of(frame).filter(|s| s.state.rolling()) else {
                    continue;
                };
                let k = usize::from(t) * count + usize::from(id);
                let (Some(m), Some(had)) = (control_msg(id, v), tap.in_file.get_mut(k)) else {
                    continue;
                };
                if *had == v {
                    continue;
                }
                *had = v;
                let tick = (s.tick_at(frame) - start).round().max(0.0) as u64;
                r.run.pass.note(tick, t, m);
            }
        }
        tap.notes.clear();
        tap.controls.clear();
    }

    /// After the mixer: the pass takes this period's capture points. When
    /// it is over, the capture points are freed, the transport goes back
    /// to where it was, and the pass is given back for the screen.
    pub fn period(
        &mut self,
        mixer: &mut Mixer,
        time: &mut crate::time::TimeHost,
    ) -> Option<Box<Run>> {
        let r = self.now.as_mut()?;
        r.see_roll(time.block());
        if !r.rolled {
            r.waited += 1;
            if r.waited < START_PERIODS {
                return None;
            }
            r.run.pass.cancel();
            r.run.said =
                Some("the transport didn't start (does it follow another device's clock?)".into());
        }
        let info = time.info();
        if r.rolled
            && !r.ended
            && (info.position >= r.run.range.end || (r.stopping && !info.state.rolling()))
        {
            r.ended = true;
            // The range is over: the song stops (if the stop below didn't
            // already), and what still sounds rings out as the tail.
            if info.state.rolling() {
                time.command(Command::Stop);
            }
        }
        if r.rolled && !r.ended && !r.stopping && info.state.rolling() {
            // The range ends within the coming period: the song stops on
            // that frame, so nothing after the range is played or logged.
            let next = time.next_frame();
            let end = time
                .block()
                .spans()
                .last()
                .map(|s| s.frame_of(r.run.range.end as f64));
            if let Some(at) = end.filter(|&at| at < next + PERIOD as u64) {
                r.stopping = time.command_at(at, Command::Stop);
            }
        }
        if r.rolled && !r.ended && !info.state.rolling() && r.run.said.is_none() {
            // STOP while the range plays: the export is cancelled.
            r.run.pass.cancel();
            r.run.said =
                Some("Export cancelled: the transport stopped, and its files were removed".into());
        }
        let step = r.run.pass.period(mixer, PERIOD, r.ended);
        if step == Step::Go {
            return None;
        }
        let Running {
            mut run, before, ..
        } = self.now.take()?;
        run.pass.detach(mixer);
        // What the file left out or brought into range, for the summary.
        run.pass.counted(self.tap.dropped, self.tap.out_of_range);
        self.tap.stop();
        time.anchor_dice(false);
        if time.info().state.rolling() {
            time.command(Command::Stop);
        }
        time.command(Command::SetLoop(before.loop_range));
        if run.range.song.is_some_and(|s| s != before.song) {
            let _ = time.act(TimeAction::SongMode(before.song));
        }
        time.command(Command::Locate(before.position));
        Some(run)
    }
}

impl Running {
    fn see_roll(&mut self, block: &Block) {
        if !self.rolled && block.spans().iter().any(|s| s.state.rolling()) {
            self.rolled = true;
        }
    }
}

/// The mixer strips that can have a stem: the engines' tracks, each
/// track's own channel in use (`MixHost::own_channels_in_use`, named after
/// its track), each free place's in use (the instrument redesign's step 6,
/// named as the mixer names it, "FM · Pads"), each kit strip a kit holds
/// (sampler step 15d, "Kit 7"), and the returns.
pub fn stem_strips(mix: &mixing::MixHost, tracks: &[String]) -> Vec<(String, StripId)> {
    let in_use = mix.own_channels_in_use();
    let free_used = mix.instruments.free_in_use();
    let mut out = Vec::new();
    for s in mix.mixer.strips() {
        match s {
            // The shared Audio strip only while it holds settings (an older
            // project's: `MixHost::old_audio`); the audio tracks have their
            // own channels.
            StripId::Track(mixing::AUDIO) if !mix.old_audio() => {}
            StripId::Track(t) if t < TRACKS => out.push((mixing::name(s), s)),
            // A kit strip while a kit holds it (sampler step 15d): what
            // a kit no track plays sounds through.
            StripId::Track(t) if t >= mixing::KIT_FIRST => {
                if mix
                    .kit_strips()
                    .get(t - mixing::KIT_FIRST)
                    .is_some_and(Option::is_some)
                {
                    out.push((mix.strip_name(s), s));
                }
            }
            StripId::Track(t) if t >= mixing::FREE_FIRST => {
                let n = t - mixing::FREE_FIRST;
                if n < crate::instruments::FREE && free_used >> n & 1 == 1 {
                    out.push((mix.strip_name(s), s));
                }
            }
            StripId::Track(t) if in_use & (1 << (t - TRACKS)) != 0 => {
                let k = t - TRACKS;
                let name = tracks
                    .get(k)
                    .filter(|n| !n.trim().is_empty())
                    .cloned()
                    .unwrap_or_else(|| mixing::name(s));
                out.push((name, s));
            }
            StripId::Return(_) => out.push((mixing::name(s), s)),
            _ => {}
        }
    }
    out
}

/// The plugins in the song: every plugin (`clap:` or `mpc:`) effect on the mixer and
/// instrument on a track, by name, each once.
pub fn plugins_in(mixer: &Mixer, instruments: &crate::instruments::Instruments) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |name: &str| {
        if !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    };
    for s in mixer.strips() {
        if let Some(c) = mixer.chain(s) {
            for k in 0..c.len() {
                if let Some(n) = c.node(k) {
                    if crate::plugins::Plugins::is_plugin(n.info().id) {
                        add(n.info().name);
                    }
                }
            }
        }
    }
    for t in 0..crate::instruments::SLOTS {
        if let Some(h) = instruments.held(t) {
            if crate::plugins::Plugins::is_plugin(h.id) {
                add(h.name);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_browser_ui::{ExportChoice, ExportRange, StemsBy};

    fn facts() -> Facts {
        Facts {
            rate: 48_000,
            arrangement_end: 96 * 3840,
            position: 3840 * 5 + 100,
            meter: (4, 4),
            bar_ticks: 3840,
            tempos: vec![(0, 120.0), (3840 * 48, 60.0)],
            tracks: vec!["Drums".into(), "Bass".into()],
            stems: vec![
                ("Drums".into(), StripId::Track(0)),
                ("Bass".into(), StripId::Track(4)),
                ("Return A".into(), StripId::Return(0)),
            ],
            by_track: track_stems(
                &[
                    ("Kick".into(), Plays::Pads),
                    ("Bass".into(), Plays::Own),
                    ("Hats".into(), Plays::Pads),
                    ("Audio 4".into(), Plays::Nothing),
                    ("Audio 5".into(), Plays::Nothing),
                ],
                // Bass (track 2) plays its own instrument, into its own
                // channel; Kick and Hats the kit, into Drums.
                &|k| {
                    let t = if k == 1 { TRACKS + 1 } else { mixing::DRUMS };
                    (StripId::Track(t), false)
                },
                // The audio tracks (tracks 4 and 5), each into its own
                // channel.
                &[
                    (3, StripId::Track(TRACKS + 3), "Audio 4".into()),
                    (4, StripId::Track(TRACKS + 4), "Audio 5".into()),
                ],
                &[("Return A".into(), StripId::Return(0))],
            ),
            song_on: false,
            rig: Rig::default(),
        }
    }

    /// A track's controllers, bend and pressure go to the MIDI file as
    /// MIDI Out thins them: a control's first value in a period on its
    /// frame, a later one at the next period's start (the last given,
    /// which counts as that period's), only while an export runs; a value
    /// out of range goes in as the nearest and is counted, one that isn't
    /// a number is left out and counted; anything not a track's MIDI out
    /// is left alone.
    #[test]
    fn the_tap_thins_a_tracks_controls_as_midi_out_does() {
        use mpfree_play::mpfree_param::Strip;
        let cc74 = ParamAddr::new(Owner::Midi(2), 74);
        let bend = ParamAddr::new(Owner::Midi(2), midi::BEND);
        let mut t = Tap::new();
        t.period(0);
        t.control(0, cc74, 64.0);
        assert!(t.controls.is_empty(), "no export running");
        t.start();
        t.period(0);
        // Three values in one period: the first goes, the last waits.
        for (frame, v) in [(10, 64.0), (20, 64.2), (30, 65.0)] {
            t.control(frame, cc74, v);
        }
        t.control(50, bend, -9000.0);
        t.control(60, ParamAddr::new(Owner::Midi(1), midi::PRESSURE), 40.0);
        t.control(70, ParamAddr::new(Owner::Strip(Strip::Track(2)), 0), 0.5);
        t.control(80, ParamAddr::new(Owner::Midi(1), 74), f32::NAN);
        assert_eq!(
            t.controls,
            [
                (10, 2, 74, 64),
                (50, 2, midi::BEND, -8192),
                (60, 1, midi::PRESSURE, 40)
            ]
        );
        assert_eq!((t.out_of_range, t.dropped), (1, 1));
        t.controls.clear();
        // The next period: the waiting 65 on its first frame, and as it
        // counts as this period's, a new value waits again.
        t.period(128);
        t.control(140, cc74, 300.0);
        t.control(150, bend, 0.0);
        assert_eq!(t.controls, [(128, 2, 74, 65), (150, 2, midi::BEND, 0)]);
        t.controls.clear();
        t.period(256);
        assert_eq!(t.controls, [(256, 2, 74, 127)]);
        assert_eq!(t.out_of_range, 2);
        // A new export starts afresh.
        t.stop();
        t.start();
        assert_eq!((t.out_of_range, t.dropped), (0, 0));
        t.period(0);
        t.control(5, cc74, 127.0);
        assert_eq!(t.controls, [(5, 2, 74, 127)]);
    }

    #[test]
    fn the_range_and_its_length_follow_the_tempo_map() {
        let f = facts();
        // 48 bars at 120 (2 s a bar), 48 at 60 (4 s a bar).
        assert_eq!(f.frames(0, f.arrangement_end), (48 * 2 + 48 * 4) * 48_000);
        assert_eq!(f.frames(3840 * 47, 3840 * 49), 6 * 48_000);
        let r = f.range(ExportRange::Arrangement);
        assert_eq!(
            r,
            Range {
                start: 0,
                end: 96 * 3840,
                song: Some(true)
            }
        );
        // Bars: from the bar the playhead is in.
        let r = f.range(ExportRange::Bars(8));
        assert_eq!((r.start, r.end, r.song), (3840 * 5, 3840 * 13, None));
        let fmt = panel_format(mpfree_fx::Bits::I24, crate::RATE);
        let (setup, mode, why) = f.setup(Mode::Fast, fmt, "Song 2026-11-14 01.32");
        assert_eq!(setup.arrangement.as_deref(), Some("4:48, 96 bars"));
        assert_eq!(setup.from_bar, 6);
        assert_eq!(setup.stems, 3);
        // By track: Kick, Bass, Hats, two audio tracks and the return; the
        // kick and the hats each need a replay (each audio track is alone
        // on its own strip). Bars of the clips (song mode off) leave the
        // audio tracks out.
        assert_eq!(setup.tracks_arrangement, (6, 2));
        assert_eq!(setup.tracks_bars, (4, 2));
        assert_eq!((mode, why), (Mode::Fast, Why::Chosen));
        assert!(setup.mode.starts_with("Fast, quiet"));
        assert_eq!(setup.folder, "Recordings / Song 2026-11-14 01.32");
        // No arrangement: Bars only.
        let none = Facts {
            arrangement_end: 0,
            ..facts()
        };
        assert_eq!(none.setup(Mode::Fast, fmt, "x").0.arrangement, None);
        assert_eq!(none.range(ExportRange::Arrangement).song, None);
    }

    #[test]
    fn the_plan_has_what_was_ticked() {
        let f = facts();
        let c = ExportChoice {
            mixdown: true,
            stems: true,
            midi: true,
            by: StemsBy::Channel,
            range: ExportRange::Bars(50),
            format: panel_format(mpfree_fx::Bits::I16, 48_000),
        };
        let (p, r) = f.plan(&c, Path::new("/x/y"), "Night drive", Mode::Listen);
        let names: Vec<&str> = p.streams.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Mixdown", "Drums", "Bass", "Return A"]);
        let m = p.midi.unwrap();
        // The tempo change at bar 49 is 43 bars after the range's start.
        assert_eq!(m.tempos, [(0, 120.0), (3840 * 43, 60.0)]);
        assert_eq!(m.end, 50 * 3840);
        assert_eq!(m.tracks, ["Drums", "Bass"]);
        assert_eq!(r.start, 5 * 3840);
        assert_eq!(p.length, Some(f.frames(r.start, r.end)));
        assert_eq!(p.mode, Mode::Listen);
        assert_eq!(
            p.format,
            mpfree_fx::WavFormat {
                bits: mpfree_fx::Bits::I16,
                rate: 48_000
            },
            "the panel's format"
        );
        let c = ExportChoice {
            stems: false,
            midi: false,
            ..c
        };
        let (p, _) = f.plan(&c, Path::new("/x/y"), "Night drive", Mode::Fast);
        assert_eq!(p.streams.len(), 1);
        assert!(p.midi.is_none());
    }

    #[test]
    fn the_plays_line_says_why_it_listens() {
        let rig = Rig {
            external_midi: vec!["Juno".into()],
            ..Rig::default()
        };
        let (mode, why) = choose(Mode::Fast, &rig);
        assert_eq!(mode, Mode::Listen);
        assert_eq!(
            mode_words(mode, &why),
            "Listen: it plays in real time and you hear it. Juno plays outside gear over MIDI, which only plays in real time"
        );
    }

    /// Each track chunk's events: (tick, the message's bytes; metas as
    /// [0xFF, type]).
    fn read_smf(b: &[u8]) -> Vec<Vec<(u64, Vec<u8>)>> {
        fn vlq(b: &[u8], i: &mut usize) -> u64 {
            let mut v = 0u64;
            loop {
                let c = b[*i];
                *i += 1;
                v = (v << 7) | u64::from(c & 0x7F);
                if c & 0x80 == 0 {
                    return v;
                }
            }
        }
        let mut out = Vec::new();
        let mut i = 14;
        while i + 8 <= b.len() {
            assert_eq!(&b[i..i + 4], b"MTrk");
            let len = u32::from_be_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
            let (mut k, end) = (i + 8, i + 8 + len);
            let (mut t, mut evs) = (0u64, Vec::new());
            while k < end {
                t += vlq(b, &mut k);
                let st = b[k];
                k += 1;
                if st == 0xFF {
                    let ty = b[k];
                    k += 1;
                    let n = vlq(b, &mut k) as usize;
                    k += n;
                    evs.push((t, vec![0xFF, ty]));
                } else {
                    let n = if matches!(st & 0xF0, 0xC0 | 0xD0) {
                        1
                    } else {
                        2
                    };
                    let mut m = vec![st];
                    m.extend_from_slice(&b[k..k + n]);
                    k += n;
                    evs.push((t, m));
                }
            }
            out.push(evs);
            i = end;
        }
        out
    }

    struct Quiet;
    impl crate::time::Pads for Quiet {
        fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
            true
        }
        fn release_at(&mut self, _: u16, _: usize, _: u32) {}
        fn stop_all(&mut self) {}
        fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
    }

    /// The pads for a track on slow gear (sequencer S5): MIDI out takes
    /// the track's own gear controls ahead, and in their own period they
    /// reach the tap as `SeqPads::out_sent` hands them on.
    struct SlowGear<'a> {
        tap: &'a mut Tap,
        modu: &'a crate::modmix::ModMix,
        track: u16,
    }
    impl crate::time::Pads for SlowGear<'_> {
        fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
            true
        }
        fn release_at(&mut self, _: u16, _: usize, _: u32) {}
        fn stop_all(&mut self) {}
        fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
        fn out_ahead(&mut self, o: &Out) -> bool {
            o.track == self.track && mpfree_play::is_gear_control(o)
        }
        fn out_sent(&mut self, o: &Out) {
            if mpfree_play::is_gear_control(o) {
                self.tap
                    .gear_control(o, |addr| self.modu.engine().base(addr));
            }
        }
    }

    struct Loop {
        time: crate::time::TimeHost,
        mix: crate::mixing::MixHost,
        host: Host,
        i: u64,
        /// Events the player "sends" (period, offset, event), as SeqPads
        /// hands them to the tap.
        inject: Vec<(u64, u32, Event)>,
        /// Two tracks play the synth (a kick's tone and a hat's), each
        /// kept quiet as SeqPads keeps it in a replay; else one tone.
        two: bool,
        /// Frames rolled in this pass: the tones start with the range.
        rolled: usize,
        /// A track on slow gear, and its ahead in frames.
        slow: Option<(u16, u32)>,
    }

    impl Loop {
        fn new() -> Loop {
            Loop::with(crate::time::TimeHost::new(crate::RATE, 120.0).unwrap())
        }

        fn with(time: crate::time::TimeHost) -> Loop {
            let mut l = Loop {
                time,
                mix: crate::mixing::MixHost::new(crate::RATE, 0.0, 0.0),
                host: Host::new(),
                i: 0,
                inject: Vec::new(),
                two: false,
                rolled: 0,
                slow: None,
            };
            // The app's mixer has always played before an export starts.
            l.period(false);
            l
        }

        fn facts(&self) -> Facts {
            let info = self.time.info();
            Facts {
                rate: crate::RATE,
                arrangement_end: 0,
                position: info.position,
                meter: (info.meter.beats, info.meter.unit),
                bar_ticks: info.meter.bar_ticks(),
                tempos: vec![(0, 120.0)],
                tracks: vec!["Track 1".into()],
                stems: stem_strips(&self.mix, &[]),
                by_track: Vec::new(),
                song_on: false,
                rig: Rig::default(),
            }
        }

        /// One period of the sound loop, as main.rs runs it: the synth
        /// plays a tone while the transport rolls (`tone`).
        fn period(&mut self, tone: bool) -> Option<Box<Run>> {
            self.time.advance(PERIOD as u32);
            let first = self.time.block().start_frame;
            self.host.tap.period(first);
            match self.slow {
                Some((track, ahead)) => {
                    let mut ahead_of = [0u32; mpfree_seq::MAX_TRACKS];
                    ahead_of[usize::from(track)] = ahead;
                    self.time.set_ahead(&ahead_of);
                    let mut pads = SlowGear {
                        tap: &mut self.host.tap,
                        modu: &self.mix.modu,
                        track,
                    };
                    self.time.period(PERIOD as u32, &mut pads);
                }
                None => self.time.period(PERIOD as u32, &mut Quiet),
            }
            for &(i, offset, event) in &self.inject {
                if i == self.i {
                    let o = Out {
                        offset,
                        frame: first + u64::from(offset),
                        track: 0,
                        dest: 0,
                        event,
                        notes: false,
                    };
                    self.host.tap.send(&o, NoteIs::Note);
                }
            }
            // The clips' locks and lanes to the mod engine, as main.rs
            // hands them on.
            for &(offset, addr, value) in self.time.locks() {
                self.mix.lock(offset, addr, value);
            }
            for &(offset, addr, value) in self.time.lanes() {
                self.mix.lane(offset, addr, value);
            }
            self.time.clear_locks();
            let rolling = self.time.block().spans().iter().any(|s| s.state.rolling());
            let at = self.i as usize * PERIOD;
            let only = self.host.only();
            let plays = |t: u16| {
                !quiets(
                    only,
                    t,
                    true,
                    Event::NoteOn {
                        note: 60,
                        velocity: 100,
                    },
                )
            };
            let w = |hz: f32, n: usize| (std::f32::consts::TAU * hz * n as f32 / 44_100.0).sin();
            let rolled = self.rolled;
            let synth: [f32; PERIOD] = std::array::from_fn(|k| match (tone && rolling, self.two) {
                (false, _) => 0.0,
                (true, false) => 0.25 * w(440.0, at + k),
                (true, true) => {
                    let n = rolled + k;
                    let kick = if plays(0) { 0.3 * w(55.0, n) } else { 0.0 };
                    let hats = if plays(1) { 0.1 * w(6000.0, n) } else { 0.0 };
                    kick + hats
                }
            });
            self.rolled = if rolling { self.rolled + PERIOD } else { 0 };
            let z = [0.0f32; PERIOD];
            let ctx = crate::mixing::ctx(
                crate::RATE,
                self.i * PERIOD as u64,
                120.0,
                self.time.block(),
            );
            let (mut l, mut r) = ([0.0f32; PERIOD], [0.0f32; PERIOD]);
            self.mix
                .period(&ctx, (&synth, &synth), (&z, &z), (&z, &z), &mut l, &mut r);
            // The tracks' controls the mod engine moved, into the tap.
            for (offset, addr, value) in self.mix.modu.take_midi_values() {
                self.host
                    .tap
                    .control(first + u64::from(offset), addr, value);
            }
            self.i += 1;
            if self.host.running() {
                // The sound card paces a real-time pass; this loop runs
                // as fast as it can, so it waits for the writer as a fast
                // pass does (a loaded machine would otherwise lose blocks).
                self.host.wait_for_room(self.i);
                self.host.notes(self.time.block());
                return self.host.period(&mut self.mix.mixer, &mut self.time);
            }
            None
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("mpfree-app-export-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_pass_plays_the_range_then_puts_the_transport_back() {
        let mut l = Loop::new();
        // Somewhere in bar 3, with a loop on.
        l.time.command(Command::Locate(2 * 3840 + 500));
        l.time.command(Command::SetLoop(Some((0, 3840))));
        l.period(false);
        let before = l.time.info();
        assert_eq!(before.loop_range, Some((0, 3840)));
        let f = l.facts();
        let c = mpfree_browser_ui::ExportChoice {
            mixdown: true,
            stems: true,
            midi: true,
            by: StemsBy::Channel,
            range: ExportRange::Bars(1),
            format: panel_format(mpfree_fx::Bits::I24, crate::RATE),
        };
        let folder = scratch("range");
        let (plan, range) = f.plan(&c, &folder, "Night drive", Mode::Listen);
        assert_eq!((range.start, range.end), (2 * 3840, 3 * 3840));
        let (pass, job) = mpfree_export::start(plan).unwrap();
        let run = Box::new(Run {
            pass,
            range,
            said: None,
        });
        l.host.begin(run, &mut l.mix.mixer, &mut l.time).unwrap();
        // A note in the pass's 10th period, 5 frames in, let go in its
        // 100th: 1157 and 12672 frames into the song.
        let p0 = l.i;
        l.inject = vec![
            (
                p0 + 9,
                5,
                Event::NoteOn {
                    note: 60,
                    velocity: 100,
                },
            ),
            (p0 + 99, 0, Event::NoteOff { note: 60 }),
        ];
        let mut n = 0;
        let mut stopped_at = None;
        let run = loop {
            n += 1;
            assert!(n < 2000, "the pass ends");
            if n == 2 {
                // The transport plays the range, with no loop.
                let info = l.time.info();
                assert!(info.state.rolling());
                assert_eq!(info.loop_range, None);
                assert!(info.position > range.start && info.position < range.end);
            }
            // The tone plays for the range only (the bar is 2 s).
            let in_range = l.time.info().position < range.end;
            if let Some(run) = l.period(in_range) {
                break run;
            }
            // The song stops on the range's last frame, so the clips don't
            // play on into the tail.
            let info = l.time.info();
            if n > 2 && !info.state.rolling() && stopped_at.is_none() {
                stopped_at = Some(info.position);
                // (Unless the stop fell on this period's first frame.)
                let rolled = l.time.block().spans().iter().find(|s| s.state.rolling());
                if let Some(s) = rolled {
                    let last = s.tick_at(s.frame + u64::from(s.len) - 1);
                    let end = range.end as f64;
                    assert!(
                        last < end && last > end - 1.0,
                        "the last frame played: {last}"
                    );
                }
            }
        };
        assert_eq!(stopped_at, Some(range.end), "stopped where the range ends");
        assert_eq!(run.said, None);
        // The transport is back where it was, stopped, its loop on again.
        let after = l.time.info();
        l.period(false);
        let after2 = l.time.info();
        assert!(!after2.state.rolling(), "{after:?}");
        assert_eq!(after2.position, before.position);
        assert_eq!(after2.loop_range, Some((0, 3840)));
        let o = job.finish(run.pass);
        let mpfree_export::Outcome::Done(d) = &o else {
            panic!("{o:?}");
        };
        // The mixdown and the synth's stem; every silent strip left out.
        let names: Vec<String> = d
            .wavs
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Mixdown.wav", "Synth.wav"], "{o:?}");
        // One bar at 120 is 2 s; the tone stops with it, and the files end
        // where the master's last sound dies away (its DC blocker rings a
        // little under 0.2 s above -90 dBFS).
        let secs = d.frames as f64 / f64::from(crate::RATE);
        assert!((2.0..2.25).contains(&secs), "{secs}");
        // The MIDI file: the note at its tick (1920 ticks a second at 120).
        let smf = std::fs::read(d.midi.as_ref().unwrap()).unwrap();
        let tracks = read_smf(&smf);
        assert_eq!(tracks.len(), 2, "the conductor and track 1");
        let tick = |frames: u64| (frames as f64 * 1920.0 / 44_100.0).round() as u64;
        assert_eq!(
            tracks[1]
                .iter()
                .filter(|(_, b)| b[0] & 0xF0 != 0xF0)
                .cloned()
                .collect::<Vec<_>>(),
            [
                (tick(1157), vec![0x90, 60, 100]),
                (tick(12_672), vec![0x80, 60, 64])
            ]
        );
        std::fs::remove_dir_all(&folder).unwrap();
    }

    /// A MIDI file track's events, as [`read_smf`] gives them.
    type SmfEvents = Vec<(u64, Vec<u8>)>;

    /// The export of [`the_midi_file_has_a_tracks_locks_and_lanes_at_their_ticks`]'s
    /// song, with Juno on gear `slow` frames slow: the MIDI file's tracks
    /// (each as (tick, bytes)) and the outcome.
    fn export_gear(slow: Option<u32>, name: &str) -> (Vec<SmfEvents>, mpfree_export::Outcome) {
        use mpfree_play::mpfree_param::{OnTrack, RelAddr};
        use mpfree_seq::arrange::Placement;
        use mpfree_seq::mpfree_moves::{Kind, Map, Steps, Target};
        use mpfree_seq::{Clip, Lock, Note};
        const BAR: Tick = 3840;
        let map = |lo: f32, hi: f32| Map {
            min: lo,
            max: hi,
            log: false,
            steps: Steps::Whole,
        };
        let mut p = crate::time::TimeHost::new(crate::RATE, 120.0)
            .unwrap()
            .project()
            .clone();
        let mut clip = Clip::new("Gear", BAR);
        // Step 5's note (a 1/16 is 240 ticks) and its lock on CC 74.
        clip.notes.push(Note::new(960, 120, 60, 100));
        clip.locks
            .push(Lock::new(960, None, RelAddr::new(OnTrack::Midi, 74), 100.0));
        // The bend in the middle (8192 of its 16383 steps up), then all
        // the way up at the half bar.
        let bend = Target::Track(RelAddr::new(OnTrack::Midi, midi::BEND));
        clip.moves
            .put(
                bend,
                map(-8192.0, 8191.0),
                &[
                    (0.0, 8192.0 / 16383.0, Kind::Step),
                    (1920.0, 1.0, Kind::Step),
                ],
                BAR as u32,
            )
            .unwrap();
        // No pressure, then 64 from the second 1/8.
        let pressure = Target::Track(RelAddr::new(OnTrack::Midi, midi::PRESSURE));
        clip.moves
            .put(
                pressure,
                map(0.0, 127.0),
                &[(0.0, 0.0, Kind::Step), (480.0, 64.0 / 127.0, Kind::Step)],
                BAR as u32,
            )
            .unwrap();
        p.tracks[1].slots[0] = Some(clip);
        p.give_clip_ids();
        p.arrangement.placements = vec![Placement::of(&p.tracks, 1, 0, 0, BAR)];
        // The song's own lane, on track 3's volume (CC 7): full, then off
        // at the half bar.
        let volume = ParamAddr::new(Owner::Midi(2), 7);
        p.arrangement
            .lanes
            .put(
                Target::Fixed(volume),
                map(0.0, 127.0),
                &[(0.0, 1.0, Kind::Step), (1920.0, 0.0, Kind::Step)],
                mpfree_seq::arrange::SONG_LANE_LEN,
            )
            .unwrap();
        let time = crate::time::TimeHost::with_project(crate::RATE, 120.0, p).unwrap();
        let mut l = Loop::with(time);
        // Juno on gear that needs `slow` frames more than the sound's lead.
        l.slow = slow.map(|frames| (1, frames));
        let mut f = l.facts();
        f.arrangement_end = BAR;
        f.tracks = vec!["Kick".into(), "Juno".into(), "Strings".into()];
        let c = mpfree_browser_ui::ExportChoice {
            mixdown: true,
            stems: false,
            midi: true,
            by: StemsBy::Track,
            range: ExportRange::Arrangement,
            format: panel_format(mpfree_fx::Bits::I24, crate::RATE),
        };
        let folder = scratch(name);
        let (plan, range) = f.plan(&c, &folder, "Gear", Mode::Fast);
        assert_eq!((range.start, range.end, range.song), (0, BAR, Some(true)));
        let (pass, job) = mpfree_export::start(plan).unwrap();
        let run = Box::new(Run {
            pass,
            range,
            said: None,
        });
        l.host.begin(run, &mut l.mix.mixer, &mut l.time).unwrap();
        // Kick's pan and Strings' volume, set by hand before the export.
        l.host.tap.at_start(0, 10, 30);
        l.host.tap.at_start(2, 7, 127);
        // A value out of CC 74's range before the song rolls: counted for
        // the summary, and not in the file (the song hadn't started).
        l.host.tap.period(0);
        l.host
            .tap
            .control(0, ParamAddr::new(Owner::Midi(0), 74), 500.0);
        let mut n = 0;
        let run = loop {
            n += 1;
            assert!(n < 4000, "the pass ends");
            if let Some(run) = l.period(false) {
                break run;
            }
        };
        assert_eq!(run.said, None);
        let o = job.finish(run.pass);
        let mpfree_export::Outcome::Done(d) = &o else {
            panic!("{o:?}");
        };
        assert_eq!((d.notes_dropped, d.out_of_range), (0, 1), "{o:?}");
        assert!(
            o.words().ends_with(
                "a MIDI file (1 controller value was outside what MIDI can hold, so the file has the nearest)"
            ),
            "{}",
            o.words()
        );
        let tracks = read_smf(&std::fs::read(d.midi.as_ref().unwrap()).unwrap());
        std::fs::remove_dir_all(&folder).unwrap();
        (tracks, o)
    }

    /// The brief's case (sequencer split A): a clip with a CC 74 lock on
    /// its 5th step, a pitch bend lane and a pressure lane, and a song
    /// lane on another track's volume, exported from the arrangement. The
    /// MIDI file has each on its track at its tick (the lock on its step's,
    /// where the file puts a control before a note), only what changes,
    /// nothing after the range; and a control a track had as the export
    /// started at the range's start. (This loop's player sends the pads
    /// nothing, so the clip's note isn't in the file.)
    #[test]
    fn the_midi_file_has_a_tracks_locks_and_lanes_at_their_ticks() {
        let (tracks, _) = export_gear(None, "controls");
        let events = |k: usize| -> Vec<(u64, Vec<u8>)> {
            tracks[k]
                .iter()
                .filter(|(_, b)| b[0] & 0xF0 != 0xF0)
                .cloned()
                .collect()
        };
        assert_eq!(tracks.len(), 4, "the conductor and three tracks");
        assert_eq!(events(1), [(0, vec![0xB0, 10, 30])], "Kick's pan");
        // The bend's 14 bits (LSB, MSB): 8191 is 0x3FFF.
        let juno = events(2);
        let at = |tick: u64| -> Vec<Vec<u8>> {
            juno.iter()
                .filter(|(t, _)| *t == tick)
                .map(|(_, b)| b.clone())
                .collect()
        };
        // The bend in the middle and no pressure are where the controls
        // start (General MIDI's), so the engine moves neither and nothing
        // goes, in the file as to the gear.
        assert_eq!(at(0), Vec::<Vec<u8>>::new(), "{juno:?}");
        assert_eq!(at(480), [vec![0xD0, 64]], "{juno:?}");
        assert_eq!(at(960), [vec![0xB0, 74, 100]], "the lock: {juno:?}");
        assert_eq!(at(1920), [vec![0xE0, 0x7F, 0x7F]], "{juno:?}");
        assert_eq!(
            juno.len(),
            3,
            "only the changes, none past the range: {juno:?}"
        );
        // The song lane on its track; its first value is what Strings had
        // as the export started, so it goes in once.
        assert_eq!(
            events(3),
            [(0, vec![0xB0, 7, 127]), (1920, vec![0xB0, 7, 0])],
            "the song lane on its track"
        );
    }

    /// Juno on slow gear (sequencer S5, 90 ms ahead): its locks and lanes
    /// go to MIDI out ahead and never reach the mod engine, and the file
    /// still has them on their ticks, as `SeqPads::out_sent` taps them.
    /// Its lanes' first values go in too (the bend in the middle, no
    /// pressure): they reach MIDI out straight, not through the engine,
    /// which moves nothing to the value it already has. The rest is as on
    /// gear that isn't slow.
    #[test]
    fn the_midi_file_has_a_slow_gear_tracks_locks_and_lanes_too() {
        let (normal, _) = export_gear(None, "controls-normal");
        let (slow, o) = export_gear(Some(crate::RATE * 90 / 1000), "controls-slow");
        let events = |t: &[(u64, Vec<u8>)], first: bool| -> Vec<(u64, Vec<u8>)> {
            t.iter()
                .filter(|(tick, b)| b[0] & 0xF0 != 0xF0 && (*tick == 0) == first)
                .cloned()
                .collect()
        };
        assert_eq!(
            events(&slow[2], true),
            [(0, vec![0xE0, 0x00, 0x40]), (0, vec![0xD0, 0])],
            "{o:?}"
        );
        assert_eq!(events(&slow[2], false), events(&normal[2], false));
        assert_eq!(events(&normal[2], true), []);
        assert_eq!(slow[1], normal[1]);
        assert_eq!(slow[3], normal[3]);
    }

    #[test]
    fn stop_while_the_range_plays_cancels_it() {
        let mut l = Loop::new();
        let f = l.facts();
        let c = mpfree_browser_ui::ExportChoice {
            mixdown: true,
            stems: false,
            midi: false,
            by: StemsBy::Track,
            range: ExportRange::Bars(4),
            format: panel_format(mpfree_fx::Bits::I24, crate::RATE),
        };
        let folder = scratch("stop");
        let (plan, range) = f.plan(&c, &folder, "Night drive", Mode::Fast);
        let (pass, job) = mpfree_export::start(plan).unwrap();
        let run = Box::new(Run {
            pass,
            range,
            said: None,
        });
        l.host.begin(run, &mut l.mix.mixer, &mut l.time).unwrap();
        assert!(l.host.fast());
        for _ in 0..20 {
            assert!(l.period(true).is_none());
        }
        l.time.command(Command::Stop);
        let run = (0..10).find_map(|_| l.period(true)).expect("it stops");
        assert!(run
            .said
            .as_deref()
            .unwrap()
            .contains("the transport stopped"));
        assert!(!l.host.running());
        assert_eq!(job.finish(run.pass), mpfree_export::Outcome::Cancelled);
        assert!(!folder.exists());
    }

    #[test]
    fn stems_by_track_come_from_the_track_s_strip_or_a_replay() {
        let rest = [("Return A".to_string(), StripId::Return(0))];
        let drums = StripId::Track(mixing::DRUMS);
        let tracks = [
            ("Kick".to_string(), Plays::Pads),
            ("Audio 2".to_string(), Plays::Nothing),
            ("Keys".to_string(), Plays::Own),
            ("Juno".to_string(), Plays::Outside),
            ("Empty".to_string(), Plays::Nothing),
            ("Hats".to_string(), Plays::Pads),
            ("Audio 7".to_string(), Plays::Nothing),
        ];
        // Each audio track plays into its own channel.
        let a2 = StripId::Track(TRACKS + 1);
        let a7 = StripId::Track(TRACKS + 6);
        let audio = [
            (1, a2, "Audio 2".to_string()),
            (6, a7, "Audio 7".to_string()),
        ];
        // Keys (track 3) plays its own instrument, into its own channel.
        let keys = StripId::Track(TRACKS + 2);
        let on = |k: usize, s: StripId| if k == 2 { keys } else { s };
        let got = track_stems(&tracks, &|k| (on(k, drums), false), &audio, &rest);
        let short: Vec<(&str, StripId, Option<Only>)> = got
            .iter()
            .map(|t| (t.name.as_str(), t.strip, t.only))
            .collect();
        // In track order, the audio tracks among them, each from its own
        // strip in the first pass.
        assert_eq!(
            short,
            [
                ("Kick", drums, Some(Only::Track(0))),
                ("Audio 2", a2, None),
                ("Keys", keys, None),
                ("Hats", drums, Some(Only::Track(5))),
                ("Audio 7", a7, None),
                ("Return A", StripId::Return(0), None),
            ]
        );
        assert!(got[1].audio && got[4].audio && !got[0].audio && !got[2].audio);
        // One track on the pads: the first pass hears it alone.
        let got = track_stems(&tracks[..5], &|k| (on(k, drums), false), &audio[..1], &[]);
        assert_eq!(got[0].only, None);
        assert_eq!((got[0].name.as_str(), got[0].strip), ("Kick", drums));
        assert_eq!((got[1].name.as_str(), got[1].only), ("Audio 2", None));
        // A frozen kit plays its take whatever the tracks do: one stem,
        // the kit's, after the tracks.
        let got = track_stems(&tracks, &|k| (on(k, drums), true), &audio, &[]);
        let names: Vec<&str> = got.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Audio 2", "Keys", "Audio 7", "Drums"]);
        assert!(got.iter().all(|t| t.only.is_none()));
        // Hats on a kit of its own, on its own channel: each track is alone
        // on its strip, so neither needs a replay.
        let hats = StripId::Track(TRACKS + 5);
        let strip = |k: usize| (on(k, if k == 5 { hats } else { drums }), false);
        let got = track_stems(&tracks, &strip, &[], &[]);
        let short: Vec<(&str, StripId, Option<Only>)> = got
            .iter()
            .map(|t| (t.name.as_str(), t.strip, t.only))
            .collect();
        assert_eq!(
            short,
            [
                ("Kick", drums, None),
                ("Keys", keys, None),
                ("Hats", hats, None),
            ]
        );
        // Only the Drums strip frozen: its one stem, and Hats as before.
        let frozen = |k: usize| (on(k, if k == 5 { hats } else { drums }), k != 5);
        let names: Vec<String> = track_stems(&tracks, &frozen, &[], &[])
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, ["Keys", "Hats", "Drums"]);
        // Track 1 playing its own instrument shares Drums with Hats on kit
        // 1 (R3 part 4b): each gets a replay there.
        let tracks = [
            ("Lead".to_string(), Plays::Own),
            ("Hats".to_string(), Plays::Pads),
        ];
        let got = track_stems(&tracks, &|_| (drums, false), &[], &[]);
        let short: Vec<(&str, StripId, Option<Only>)> = got
            .iter()
            .map(|t| (t.name.as_str(), t.strip, t.only))
            .collect();
        assert_eq!(
            short,
            [
                ("Lead", drums, Some(Only::Track(0))),
                ("Hats", drums, Some(Only::Track(1))),
            ]
        );
        // Alone there, it is heard alone in the first pass.
        let got = track_stems(&tracks[..1], &|_| (drums, false), &[], &[]);
        assert_eq!((got[0].strip, got[0].only), (drums, None));
    }

    #[test]
    fn a_replay_keeps_only_the_other_tracks_on_the_pads_quiet() {
        let on = Event::NoteOn {
            note: 36,
            velocity: 100,
        };
        let one = Some(Only::Track(2));
        assert!(quiets(one, 0, true, on));
        assert!(quiets(one, 0, true, Event::NoteOff { note: 36 }));
        assert!(quiets(one, 0, true, Event::Pressure { note: 36, value: 9 }));
        assert!(!quiets(one, 2, true, on), "the replay's own track");
        assert!(!quiets(one, 0, false, on), "its own instrument, or outside");
        assert!(!quiets(one, 0, true, Event::AllOff), "a panic as before");
        assert!(!quiets(None, 0, true, on));
    }

    #[test]
    fn the_plan_by_track_puts_shared_tracks_in_replays() {
        let f = facts();
        let c = ExportChoice {
            mixdown: true,
            stems: true,
            midi: false,
            by: StemsBy::Track,
            range: ExportRange::Arrangement,
            format: panel_format(mpfree_fx::Bits::I24, 48_000),
        };
        let (p, _) = f.plan(&c, Path::new("/x/y"), "Night market", Mode::Fast);
        let first: Vec<&str> = p.streams.iter().map(|s| s.name.as_str()).collect();
        // The audio tracks each play into a strip of their own: the first
        // pass hears them.
        assert_eq!(first, ["Mixdown", "Bass", "Audio 4", "Audio 5", "Return A"]);
        let replays: Vec<(&str, Only)> = p
            .replays
            .iter()
            .map(|r| (r.stream.name.as_str(), r.only))
            .collect();
        assert_eq!(
            replays,
            [("Kick", Only::Track(0)), ("Hats", Only::Track(2))]
        );
        assert_eq!(p.passes(), 3);
        // Bars of the clips with song mode off: the audio tracks don't
        // play, so they get no stem.
        let bars = ExportChoice {
            range: ExportRange::Bars(4),
            ..c
        };
        let (p, _) = f.plan(&bars, Path::new("/x/y"), "Night market", Mode::Fast);
        let first: Vec<&str> = p.streams.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(first, ["Mixdown", "Bass", "Return A"]);
        assert_eq!(p.passes(), 3);
        let on = Facts {
            song_on: true,
            ..facts()
        };
        let (p, _) = on.plan(&bars, Path::new("/x"), "N", Mode::Fast);
        assert_eq!((p.streams.len(), p.passes()), (5, 3));
        // By channel: one pass.
        let ch = ExportChoice {
            by: StemsBy::Channel,
            ..c
        };
        let (p, _) = f.plan(&ch, Path::new("/x/y"), "Night market", Mode::Fast);
        assert_eq!(p.passes(), 1);
        assert_eq!(p.streams.len(), 4);
    }

    #[test]
    fn stems_by_track_replay_the_range_with_each_track_alone() {
        let mut l = Loop::new();
        l.two = true;
        l.time.command(Command::Locate(2 * 3840 + 500));
        l.period(false);
        let before = l.time.info();
        let synth = StripId::Track(mixing::SYNTH);
        let stem = |name: &str, k: u16| TrackStem {
            name: name.into(),
            strip: synth,
            only: Some(Only::Track(k)),
            audio: false,
        };
        let f = Facts {
            by_track: vec![stem("Kick", 0), stem("Hats", 1)],
            ..l.facts()
        };
        let c = mpfree_browser_ui::ExportChoice {
            mixdown: true,
            stems: true,
            midi: false,
            by: StemsBy::Track,
            range: ExportRange::Bars(1),
            format: panel_format(mpfree_fx::Bits::I24, crate::RATE),
        };
        let folder = scratch("by-track");
        let (mut plan, range) = f.plan(&c, &folder, "Night market", Mode::Fast);
        // The synth's strip as the first pass hears it, to check by.
        plan.streams.push(Stream::stem("Synth", synth));
        let (pass, mut job) = mpfree_export::start(plan).unwrap();
        let mut run = Box::new(Run {
            pass,
            range,
            said: None,
        });
        let mut onlys = Vec::new();
        let o = loop {
            l.host.begin(run, &mut l.mix.mixer, &mut l.time).unwrap();
            assert!(l.host.replays());
            onlys.push(l.host.only());
            let mut n = 0;
            let back = loop {
                n += 1;
                assert!(n < 3000, "the pass ends");
                let in_range = l.time.info().position < range.end;
                if let Some(b) = l.period(in_range) {
                    break b;
                }
            };
            assert_eq!(back.said, None);
            // Between passes the transport is back where it was.
            l.period(false);
            assert_eq!(l.time.info().position, before.position);
            assert!(!l.time.info().state.rolling());
            match job.next(back.pass) {
                mpfree_export::Next::Play(next) => {
                    assert_eq!(job.progress().pass, onlys.len());
                    run = Box::new(Run {
                        pass: next,
                        range,
                        said: None,
                    });
                }
                mpfree_export::Next::Finish(p) => break job.finish(p),
            }
        };
        assert_eq!(onlys, [None, Some(Only::Track(0)), Some(Only::Track(1))]);
        let mpfree_export::Outcome::Done(d) = &o else {
            panic!("{o:?}");
        };
        let names: Vec<String> = d
            .wavs
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["Mixdown.wav", "Synth.wav", "Kick.wav", "Hats.wav"]);
        let read = |n: &str| {
            let b = std::fs::read(folder.join(n)).unwrap();
            (44..b.len())
                .step_by(6)
                .map(|i| (i32::from_le_bytes([0, b[i], b[i + 1], b[i + 2]]) >> 8) as f32)
                .collect::<Vec<f32>>()
        };
        let (all, kick, hats) = (read("Synth.wav"), read("Kick.wav"), read("Hats.wav"));
        assert_eq!(all.len(), kick.len());
        assert_eq!(all.len(), hats.len());
        assert_eq!(all.len() as u64, d.frames);
        // Each replay is its track alone, sample for sample what the
        // first pass heard of it (in 24-bit steps).
        for i in 0..all.len() {
            assert!(
                (kick[i] + hats[i] - all[i]).abs() <= 2.0,
                "frame {i}: {} + {} != {}",
                kick[i],
                hats[i],
                all[i]
            );
        }
        let loud = |v: &[f32]| v.iter().map(|x| x.abs()).fold(0.0, f32::max);
        assert!(loud(&kick) > 1e5 && loud(&hats) > 1e5);
        let after = l.time.info();
        assert_eq!(after.position, before.position);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_folder_name_is_never_taken_twice() {
        let d = std::env::temp_dir().join(format!("mpfree-export-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        // 2026-11-14 01:32 UTC.
        let now = 1_794_619_920;
        let a = new_folder_name(&d, "Night drive", now);
        assert_eq!(a, "Night drive 2026-11-14 01.32");
        std::fs::create_dir(d.join(&a)).unwrap();
        assert_eq!(
            new_folder_name(&d, "Night drive", now),
            "Night drive 2026-11-14 01.32 2"
        );
        assert_eq!(song_name(None), "Untitled");
        assert_eq!(song_name(Some("A/B")), "A-B");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
