//! Export in the app (docs/dev/design/export.md): the mixdown, the stems
//! and the MIDI file from one pass over the song, on `mpfree-export`.
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
    choose, folder_name, safe_name, MidiPlan, Mode, Pass, Plan, Rig, Step, Stream, Why,
};
use mpfree_fx::{Mixer, StripId};
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
/// Periods the transport may take to start before the export gives up
/// (about a second).
const START_PERIODS: u32 = 400;
/// Capture slots an export may use: every one below Freeze and Print's
/// (takes.rs) and the sampler's.
pub const MAX_STREAMS: usize = crate::takes::TAKE_CAPTURE;
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
    /// The mixer strips that can have a stem, with their names.
    pub stems: Vec<(String, StripId)>,
    pub rig: Rig,
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
    pub fn gather(time: &crate::time::TimeHost, mix: &mixing::MixHost, rig: Rig) -> Facts {
        let info = time.info();
        let project = time.project();
        let tracks = track_names(time);
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
            tempos: time
                .tempo_map()
                .segments()
                .iter()
                .map(|s| (s.start, s.bpm))
                .collect(),
            stems: stem_strips(&mix.mixer, mix.instruments.in_use(), &tracks),
            tracks,
            rig,
        }
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
        if choice.mixdown {
            streams.push(Stream::mixdown());
        }
        if choice.stems {
            for (name, strip) in &self.stems {
                if streams.len() < MAX_STREAMS {
                    streams.push(Stream::stem(name, *strip));
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
            "Listen: it plays once in real time and you hear it (Prefs > Export)".into()
        }
        (Mode::Listen, w) => format!(
            "Listen: it plays once in real time and you hear it. {}",
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
/// to the pass after the period. Fixed room; never allocates once made.
pub struct Tap {
    on: bool,
    chords: Box<Chords>,
    notes: Vec<(u64, u16, Msg)>,
    /// Messages past [`TAP_ROOM`] in a period, left out.
    pub dropped: u64,
}

impl Default for Tap {
    fn default() -> Self {
        Self::new()
    }
}

impl Tap {
    pub fn new() -> Tap {
        Tap {
            on: false,
            chords: Box::default(),
            notes: Vec::with_capacity(TAP_ROOM),
            dropped: 0,
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

    /// A tap that takes notes at once (the real-time safety tests).
    #[cfg(test)]
    pub fn on() -> Tap {
        let mut t = Tap::new();
        t.start();
        t
    }

    /// The notes taken this period, then empty (the real-time safety
    /// tests: what [`Host::notes`] hands the pass).
    #[cfg(test)]
    pub fn take(&mut self) -> usize {
        let n = self.notes.len();
        self.notes.clear();
        n
    }

    fn start(&mut self) {
        self.on = true;
        self.chords.clear();
        self.notes.clear();
    }

    fn stop(&mut self) {
        self.on = false;
        self.notes.clear();
    }
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
    /// arrangement) and plays. The pass comes back at once if the mixer
    /// refused (or one already runs).
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
        if !run.pass.attach(mixer) {
            return Err(run);
        }
        let info = time.info();
        let before = Before {
            song: time.song_on(),
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
        });
        Ok(())
    }

    /// After the clips and arps played this period: their notes go to
    /// the MIDI file, at their ticks from the range's start.
    pub fn notes(&mut self, block: &Block) {
        let Some(r) = self.now.as_mut() else {
            return;
        };
        r.see_roll(block);
        if r.rolled {
            let start = r.run.range.start as f64;
            for &(frame, track, msg) in &self.tap.notes {
                let span = block
                    .spans()
                    .iter()
                    .find(|s| frame >= s.frame && frame < s.frame + u64::from(s.len));
                if let Some(s) = span {
                    let tick = (s.tick_at(frame) - start).round().max(0.0) as u64;
                    r.run.pass.note(tick, track, msg);
                }
            }
        }
        self.tap.notes.clear();
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
        self.tap.stop();
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
/// track's own instrument in use (named after its track), and the
/// returns.
pub fn stem_strips(mixer: &Mixer, in_use: u128, tracks: &[String]) -> Vec<(String, StripId)> {
    let mut out = Vec::new();
    for s in mixer.strips() {
        match s {
            StripId::Track(t) if t < TRACKS => out.push((mixing::name(s), s)),
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

/// The plugins in the song: every `clap:` effect on the mixer and
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
                    if n.info().id.starts_with("clap:") {
                        add(n.info().name);
                    }
                }
            }
        }
    }
    for t in 0..crate::instruments::SLOTS {
        if let Some(h) = instruments.held(t) {
            if h.id.starts_with("clap:") {
                add(h.name);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_browser_ui::{ExportChoice, ExportRange};

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
            rig: Rig::default(),
        }
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
            "Listen: it plays once in real time and you hear it. Juno plays outside gear over MIDI, which only plays in real time"
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
        fn hit_at(&mut self, _: usize, _: u8, _: u32) -> bool {
            true
        }
        fn release_at(&mut self, _: usize, _: u32) {}
        fn stop_all(&mut self) {}
        fn pressure_at(&mut self, _: usize, _: u8, _: u32) {}
    }

    struct Loop {
        time: crate::time::TimeHost,
        mix: crate::mixing::MixHost,
        host: Host,
        i: u64,
        /// Events the player "sends" (period, offset, event), as SeqPads
        /// hands them to the tap.
        inject: Vec<(u64, u32, Event)>,
    }

    impl Loop {
        fn new() -> Loop {
            let mut l = Loop {
                time: crate::time::TimeHost::new(crate::RATE, 120.0).unwrap(),
                mix: crate::mixing::MixHost::new(crate::RATE, 0.0, 0.0),
                host: Host::new(),
                i: 0,
                inject: Vec::new(),
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
                stems: stem_strips(&self.mix.mixer, self.mix.instruments.in_use(), &[]),
                rig: Rig::default(),
            }
        }

        /// One period of the sound loop, as main.rs runs it: the synth
        /// plays a tone while the transport rolls (`tone`).
        fn period(&mut self, tone: bool) -> Option<Box<Run>> {
            self.time.advance(PERIOD as u32);
            self.time.period(PERIOD as u32, &mut Quiet);
            let first = self.time.block().start_frame;
            for &(i, offset, event) in &self.inject {
                if i == self.i {
                    let o = Out {
                        offset,
                        frame: first + u64::from(offset),
                        track: 0,
                        dest: 0,
                        event,
                    };
                    self.host.tap.send(&o, NoteIs::Note);
                }
            }
            let rolling = self.time.block().spans().iter().any(|s| s.state.rolling());
            let at = self.i as usize * PERIOD;
            let synth: [f32; PERIOD] = std::array::from_fn(|k| {
                if tone && rolling {
                    0.25 * (std::f32::consts::TAU * 440.0 * (at + k) as f32 / 44_100.0).sin()
                } else {
                    0.0
                }
            });
            let z = [0.0f32; PERIOD];
            let ctx = crate::mixing::ctx(
                crate::RATE,
                self.i * PERIOD as u64,
                120.0,
                self.time.block(),
            );
            let (mut l, mut r) = ([0.0f32; PERIOD], [0.0f32; PERIOD]);
            self.mix.period(
                &ctx,
                (&synth, &synth),
                (&z, &z),
                (&z, &z),
                (&z, &z),
                &mut l,
                &mut r,
            );
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

    #[test]
    fn stop_while_the_range_plays_cancels_it() {
        let mut l = Loop::new();
        let f = l.facts();
        let c = mpfree_browser_ui::ExportChoice {
            mixdown: true,
            stems: false,
            midi: false,
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
