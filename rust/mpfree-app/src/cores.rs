//! Where the sound runs: Prefs "Sound cores" (Shared, Core 3, Cores 3 and
//! 2, Cores 3, 2 and 1; kept in the "app" settings as `sound-cores`, used
//! from the next start), or `--sound-cores shared|core3|two|three` with
//! `play` or `app` for one run. The default is `shared`, as before.
//!
//! - `shared`: the audio thread runs at SCHED_FIFO 70 on whichever core the
//!   kernel gives it.
//! - `core3`: the audio thread runs on core 3 only. The test stick's kernel
//!   keeps cores 2 and 3 free of other work (`isolcpus=2-3`), so it has the
//!   core to itself.
//! - `two`: core 3 as above, plus a second core for the sound. Half the drum
//!   voices (the sampler's split, `Engine::split`) and about half the pad
//!   channels' and tracks' inserts (`Mixer::set_lane`) run on lanes
//!   (`mpfree-lanes`) pinned to core 2 at SCHED_FIFO 69. Each period the
//!   sampler's lane works first and the mixer's after, so the two worker
//!   threads share core 2 without running at once.
//! - `three` (a test, Jay 12 Nov): `two`, plus the synth on a lane pinned
//!   to core 1 at SCHED_FIFO 69 ([`SynthSlot`]). It is posted before the
//!   drums render and joined after, so the synth, the audio thread's half
//!   of the drums and the lane's half run at the same time on three cores.
//!   Core 0 is left to the interrupts, the screen and the rest. Core 1 is
//!   not kept free by the kernel (only 2 and 3 are), so the synth's lane
//!   shares it with ordinary threads (it runs ahead of them) and with any
//!   interrupts sent there: finding out what that costs is the test.
//!
//! **Reverb returns a block late** (`two` and `three`;
//! `Mixer::set_pipelined_returns`, #260): a return whose reverb can take a
//! block back from its pre-delay (a Cloud all wet, with at least a block of
//! pre-delay, no sidechain) runs its inserts on a lane on core 2 one period
//! behind and still sounds on time, bit for bit. Every other return (the
//! Play page's delay, a reverb with no pre-delay or a dry mix) stays on the
//! audio thread. The returns lane runs at SCHED_FIFO 68, one below the
//! other lanes: the mixer posts it at the start of the block and joins it
//! at the returns, after the pad channels, tracks and submixes, so the
//! inserts lane (joined first) takes core 2 from it whenever both have work.
//!
//! **One deadline per period** ([`lane_deadline`]). Every lane join in a
//! period (the sampler's parts, the synth, each mixer stage and the
//! returns) waits until the same moment at most: when the sound card's
//! queue would run dry, less a period for the work after the joins (the end
//! of the mix and the write). The audio loop reads the card's delay right
//! after each write and sets the next period's deadline from it, so after a
//! long period the next deadlines come sooner, and however many lanes are
//! late in one period, waiting for them never empties the card's queue by
//! itself. (Before, each join waited a period from when it started, so a
//! sampler part, two mixer stages and the returns could together wait four
//! periods with three in the buffer.)
//!
//! A late lane is never silent (Jay's rule, 12 Nov). A late sampler part
//! leaves that half of the voices out of that part of the period, a late
//! mixer stage leaves those strips silent for the block, and a late synth
//! is left out of that period (Jay, 12 Nov 20:51Z): all three are
//! fallbacks. The synth's lane finishes that period on its own while the
//! sound card takes this one, and [`SynthSlot::settle`] takes the synth
//! back right after the write; only if it is still not back by the next
//! period's deadline does the audio thread wait for it (the rest of the
//! loop plays the synth), and that wait is counted too. All of these are
//! counted on the Dev tab's "CPU by engine" page and in the 30 s status
//! line, and a line goes to the log within a second of one.
//!
//! **Lanes only where they were asked for.** The lanes are strict
//! (`mpfree_lanes::Config::strict`): a lane whose worker could not be
//! pinned to its core or given SCHED_FIFO runs on the audio thread instead
//! (the same sound, one core), and says so in red. A worker that could not
//! be pinned would keep the audio thread's own core 3 and fight it for that
//! core. For the same reason, if the audio thread itself could not be put
//! on core 3, no lane starts at all.
//!
//! The mode is set once, on the audio thread, before the sound starts.
//! Only the program's own threads change (their core and priority, in
//! RAM): quitting the program undoes it. Prefs keeps the row's value in
//! the app's settings, as every Prefs row.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use mpfree_fx::Mixer;
use mpfree_lanes::{Config as LaneConfig, Job, Lane, Setup as LaneSetup};
use mpfree_ui::Tone;

use crate::sampler::Player;
use crate::synth::Synth;
use crate::PERIOD;

/// One row of the Dev page: what, its value, its colour.
pub type Row = (String, String, Tone);

/// The MPC One's plan (docs/dev/design/concurrency.md): the screen and the
/// rest on cores 0 and 1, the audio thread alone on core 3, the lanes on
/// core 2 one priority below it; with `three`, the synth's lane on core 1.
/// Another device (the MPC Gen 2's 8 cores, bookmarked for later) gets its
/// own plan.
pub const AUDIO_CORE: usize = 3;
pub const LANE_CORE: usize = 2;
pub const SYNTH_CORE: usize = 1;
pub const LANE_FIFO: i32 = 69;
/// The reverb returns' lane, one below the other lanes on core 2: it has
/// until the returns to finish, the inserts lane only until its stage ends.
pub const RETURNS_FIFO: i32 = LANE_FIFO - 1;

/// The cores the program's helper threads run on while the sound has
/// cores of its own: 0 and 1 (the screen, the writers, the loaders).
pub const BACKGROUND_CORES: [usize; 2] = [0, 1];

/// Puts the calling helper thread at the normal policy and, while the sound
/// has cores of its own, on [`BACKGROUND_CORES`]
/// ([`mpfree_lanes::os::background`]). Every thread the program starts for
/// other work (a writer, a loader, a scanner) calls it first: a thread
/// started by the audio thread begins on the audio thread's core, and would
/// run there between its periods and keep it from idling.
pub fn background() -> Result<(), String> {
    mpfree_lanes::os::background()
}

/// A period: 128 frames at 44.1 kHz. The mixer's lane waits this long at
/// a stage when the period has no deadline of its own ([`lane_deadline`]).
pub const PERIOD_TIME: Duration = Duration::from_nanos(PERIOD as u64 * 1_000_000_000 / 44_100);

/// The next period's deadline for every lane join in it: when the sound
/// card's queue would run dry, less one period for the work after the
/// joins (the end of the mix and the write). `written`: when the last
/// period was written; `delay`: the card's delay read then, in frames
/// (`snd_pcm_delay`), or None if it could not be read, when a full queue of
/// `queued` periods is assumed (a write that waited leaves it full). A
/// delay outside 0..=the buffer is held to it. With three periods in the
/// buffer and the queue full, the deadline is two periods after the write.
/// No allocation.
pub fn lane_deadline(written: Instant, delay: Option<i64>, queued: u32) -> Instant {
    let full = i64::from(queued) * PERIOD as i64;
    let frames = delay.unwrap_or(full).clamp(0, full) - PERIOD as i64;
    if frames <= 0 {
        return written;
    }
    written + Duration::from_nanos(frames as u64 * 1_000_000_000 / 44_100)
}

/// The shortest time between two log lines about late lanes.
const NOTE_EVERY: Duration = Duration::from_secs(1);

/// Where the sound runs (`--sound-cores`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SoundCores {
    /// The audio thread runs where the kernel puts it (as before).
    #[default]
    Shared,
    /// The audio thread runs on core 3 only.
    Core3,
    /// The audio thread on core 3, half the drum voices and part of the
    /// mixer's inserts on core 2.
    Two,
    /// `Two`, plus the synth on core 1 (a test).
    Three,
}

impl SoundCores {
    pub const ALL: [SoundCores; 4] = [
        SoundCores::Shared,
        SoundCores::Core3,
        SoundCores::Two,
        SoundCores::Three,
    ];
    pub const NAMES: &'static str = "shared, core3, two or three";

    pub fn parse(s: &str) -> Option<SoundCores> {
        SoundCores::ALL.into_iter().find(|m| m.name() == s)
    }

    pub fn name(self) -> &'static str {
        match self {
            SoundCores::Shared => "shared",
            SoundCores::Core3 => "core3",
            SoundCores::Two => "two",
            SoundCores::Three => "three",
        }
    }

    /// Prefs' value for it.
    pub fn row(self) -> &'static str {
        match self {
            SoundCores::Shared => "Shared",
            SoundCores::Core3 => "Core 3",
            SoundCores::Two => "Cores 3 and 2",
            SoundCores::Three => "Cores 3, 2 and 1",
        }
    }

    fn label(self) -> &'static str {
        match self {
            SoundCores::Shared => "shared: the audio thread runs where the kernel puts it",
            SoundCores::Core3 => "core3: the audio thread alone on core 3",
            SoundCores::Two => "two: the audio thread on core 3, lanes on core 2",
            SoundCores::Three => "three: the audio thread on core 3, lanes on cores 2 and 1",
        }
    }

    /// The drums and the mixer have lanes on core 2.
    fn lanes(self) -> bool {
        matches!(self, SoundCores::Two | SoundCores::Three)
    }
}

/// The cores asked for (tests leave them out, so nothing is pinned).
#[derive(Clone, Copy, Debug)]
struct Pins {
    audio: Option<usize>,
    lane: Option<usize>,
    synth: Option<usize>,
    fifo: Option<i32>,
}

const MPC: Pins = Pins {
    audio: Some(AUDIO_CORE),
    lane: Some(LANE_CORE),
    synth: Some(SYNTH_CORE),
    fifo: Some(LANE_FIFO),
};

#[cfg(test)]
const NONE: Pins = Pins {
    audio: None,
    lane: None,
    synth: None,
    fifo: None,
};

/// The lanes' counts since the start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Parts of periods the sampler's lane rendered half the voices for.
    pub voice_parts: u64,
    /// Of those, the ones it was late for (those voices unheard for the part).
    pub voice_late: u64,
    /// Mixer stages whose inserts went to the lane.
    pub mix_stages: u64,
    /// Of those, the ones it was late for (those strips silent for the block).
    pub mix_late: u64,
    /// Periods the synth rendered on its lane (`three`).
    pub synth_periods: u64,
    /// Of those, the ones it missed the period's deadline for (left out of
    /// that period: a fallback).
    pub synth_late: u64,
    /// Late synth periods still not back by the next period's deadline,
    /// which the audio thread then waited for (a stall).
    pub synth_waits: u64,
    /// Blocks whose reverb returns ran a block late on the returns lane.
    pub returns_blocks: u64,
    /// Of those, the ones it was late for (the returns silent for that
    /// block and the next: a fallback).
    pub returns_late: u64,
    /// Times a return started and stopped running a block late (a knob or
    /// a new effect moved it; each seamless).
    pub returns_starts: u64,
    pub returns_stops: u64,
    /// Stops with no late input left to hand the reverb: a block of the
    /// return's input lost (a fallback; it should stay 0).
    pub returns_lost: u64,
}

impl Counts {
    pub fn read(player: &Player, mixer: &Mixer, synth: &SynthSlot) -> Counts {
        let (c, m) = (player.counters(), mixer.stats());
        Counts {
            voice_parts: c.lane_parts,
            voice_late: c.lane_late,
            mix_stages: m.lane_posts,
            mix_late: m.lane_late,
            synth_periods: synth.periods,
            synth_late: synth.late,
            synth_waits: synth.waits,
            returns_blocks: m.pipe_posts,
            returns_late: m.pipe_late,
            returns_starts: m.pipe_starts,
            returns_stops: m.pipe_stops,
            returns_lost: m.pipe_lost_input,
        }
    }

    fn late(&self) -> u64 {
        self.voice_late
            + self.mix_late
            + self.synth_late
            + self.synth_waits
            + self.returns_late
            + self.returns_lost
    }

    fn values(&self) -> [u64; COUNTS] {
        [
            self.voice_parts,
            self.voice_late,
            self.mix_stages,
            self.mix_late,
            self.synth_periods,
            self.synth_late,
            self.synth_waits,
            self.returns_blocks,
            self.returns_late,
            self.returns_starts,
            self.returns_stops,
            self.returns_lost,
        ]
    }
}

/// How many numbers [`Counts`] holds.
const COUNTS: usize = 12;

/// The synth, played on the audio thread, or (`three`) on a lane on core
/// 1 while the drums render. Between periods it is always here, so the
/// rest of the audio loop uses it as a [`Synth`] (through `Deref`); it is
/// away only between [`SynthSlot::start`] and [`SynthSlot::finish`], or,
/// after a late period, until [`SynthSlot::settle`], which the audio loop
/// calls right after each write.
pub struct SynthSlot {
    home: Option<Box<Synth>>,
    lane: Option<Lane<SynthJob>>,
    frame: u64,
    bpm: f64,
    posted: bool,
    /// The last period was late: the lane still has the synth.
    away: bool,
    periods: u64,
    late: u64,
    waits: u64,
}

/// The synth's job on its lane: a period into its own pair.
struct SynthJob {
    synth: Option<Box<Synth>>,
    frame: u64,
    bpm: f64,
    left: [f32; PERIOD],
    right: [f32; PERIOD],
}

impl Job for SynthJob {
    fn run(&mut self, _block: u64) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        if let Some(s) = self.synth.as_deref_mut() {
            s.render(self.frame, self.bpm, &mut self.left, &mut self.right);
        }
    }
}

impl SynthSlot {
    /// The synth, played on the audio thread until [`Cores::start`] gives
    /// it a lane. Allocates.
    pub fn new(synth: Synth) -> SynthSlot {
        SynthSlot {
            home: Some(Box::new(synth)),
            lane: None,
            frame: 0,
            bpm: 120.0,
            posted: false,
            away: false,
            periods: 0,
            late: 0,
            waits: 0,
        }
    }

    /// Starts the period at `frame`: on its lane it renders now, while the
    /// audio thread renders the drums; else [`SynthSlot::finish`] renders
    /// it. No allocation.
    pub fn start(&mut self, frame: u64, bpm: f64) {
        (self.frame, self.bpm) = (frame, bpm);
        let Some(lane) = self.lane.as_mut() else {
            return;
        };
        if let Some(job) = lane.job() {
            job.synth = self.home.take();
            (job.frame, job.bpm) = (frame, bpm);
        }
        if lane.post(frame).is_ok() {
            self.posted = true;
        } else if let Some(job) = lane.job() {
            // Never reached (settle always takes the synth back): play here.
            self.home = job.synth.take();
        }
    }

    /// Adds the period's synth into `left` and `right` (cleared by the
    /// caller, as [`Synth::render`] adds), the same sound either way. On
    /// its lane it waits for the synth until `deadline` at most (the
    /// period's, [`lane_deadline`]): past it the synth is left out of this
    /// period (counted late) and stays on its lane, which finishes the
    /// period on its own; [`SynthSlot::settle`] takes it back. No
    /// allocation.
    pub fn finish(&mut self, deadline: Instant, left: &mut [f32], right: &mut [f32]) {
        let Some(lane) = self
            .lane
            .as_mut()
            .filter(|_| std::mem::take(&mut self.posted))
        else {
            if let Some(s) = self.home.as_deref_mut() {
                s.render(self.frame, self.bpm, left, right);
            }
            return;
        };
        self.periods += 1;
        let Ok(job) = lane.join(deadline) else {
            self.late += 1;
            self.away = true;
            return;
        };
        self.home = job.synth.take();
        for (o, s) in left.iter_mut().zip(job.left) {
            *o += s;
        }
        for (o, s) in right.iter_mut().zip(job.right) {
            *o += s;
        }
    }

    /// After a late period: takes the synth back from its lane, dropping
    /// what it made for that period. Call it right after each write, before
    /// anything plays the synth: the write's own wait has usually given the
    /// lane the time to finish. If the synth is still not back by
    /// `deadline` (the next period's), this waits for it as long as it
    /// takes, since the rest of the loop plays the synth, and counts the
    /// wait. Does nothing after a period in time. No allocation.
    pub fn settle(&mut self, deadline: Instant) {
        if !std::mem::take(&mut self.away) {
            return;
        }
        let Some(lane) = self.lane.as_mut() else {
            return;
        };
        if lane.join(deadline).is_err() {
            self.waits += 1;
            while lane.join(Instant::now() + Duration::from_secs(1)).is_err() {}
        }
        if let Some(job) = lane.job() {
            self.home = job.synth.take();
        }
    }

    /// The synth's lane, on `core` at `fifo` (`three`): allocates and
    /// starts its thread.
    fn give_lane(&mut self, config: LaneConfig) -> LaneSetup {
        let lane = Lane::new(
            SynthJob {
                synth: None,
                frame: 0,
                bpm: 120.0,
                left: [0.0; PERIOD],
                right: [0.0; PERIOD],
            },
            config,
        );
        let setup = lane.setup().clone();
        self.lane = Some(lane);
        setup
    }
}

impl std::ops::Deref for SynthSlot {
    type Target = Synth;
    fn deref(&self) -> &Synth {
        self.home
            .as_deref()
            .expect("the synth is here between periods")
    }
}

impl std::ops::DerefMut for SynthSlot {
    fn deref_mut(&mut self) -> &mut Synth {
        self.home
            .as_deref_mut()
            .expect("the synth is here between periods")
    }
}

/// What the Dev page reads: the set-up rows (once, before the sound
/// starts) and the counts (every 250 ms, with the engines' numbers). No
/// lock: the counts are atomics, and the rows are set once.
#[derive(Debug, Default)]
pub struct CoreShared {
    /// The mode, whether its lanes started, and the set-up rows.
    setup: OnceLock<(SoundCores, bool, Vec<Row>)>,
    counts: [AtomicU64; COUNTS],
}

impl CoreShared {
    /// The counts, for the screen thread.
    fn counts(&self) -> Counts {
        let get = |i: usize| self.counts[i].load(Ordering::Relaxed);
        Counts {
            voice_parts: get(0),
            voice_late: get(1),
            mix_stages: get(2),
            mix_late: get(3),
            synth_periods: get(4),
            synth_late: get(5),
            synth_waits: get(6),
            returns_blocks: get(7),
            returns_late: get(8),
            returns_starts: get(9),
            returns_stops: get(10),
            returns_lost: get(11),
        }
    }

    fn store(&self, c: Counts) {
        for (slot, v) in self.counts.iter().zip(c.values()) {
            slot.store(v, Ordering::Relaxed);
        }
    }
}

/// The mode the sound runs in, how it started, and the late lanes so far.
#[derive(Debug)]
pub struct Cores {
    mode: SoundCores,
    pins: Pins,
    /// The lanes may start: the mode has them and the audio thread is on
    /// its core (or none was asked).
    lanes: bool,
    rows: Vec<Row>,
    lines: Vec<String>,
    /// The counts at the last log line about late lanes, and when.
    noted: Counts,
    noted_at: Option<Instant>,
}

impl Cores {
    /// Puts the calling thread (the audio thread) on core 3, splits the
    /// sampler's voices onto a lane on core 2 and gives the synth a lane on
    /// core 1, as `mode` asks. Call it on the audio thread before the sound
    /// starts: it allocates and starts the lanes' threads. The mixer's lane
    /// follows with [`Cores::start_mixer`] once the mixer is made.
    pub fn start(mode: SoundCores, player: &mut Player, synth: &mut SynthSlot) -> Cores {
        Cores::start_on(mode, player, synth, MPC)
    }

    /// [`Cores::start`] with nothing pinned and no priority asked (the
    /// tests: the lanes run, the test threads stay where they are).
    #[cfg(test)]
    pub fn start_unpinned(mode: SoundCores, player: &mut Player, synth: &mut SynthSlot) -> Cores {
        Cores::start_on(mode, player, synth, NONE)
    }

    fn start_on(mode: SoundCores, player: &mut Player, synth: &mut SynthSlot, pins: Pins) -> Cores {
        let mut c = Cores {
            mode,
            pins,
            lanes: false,
            rows: vec![(
                "Sound cores".to_string(),
                mode.label().to_string(),
                if mode == SoundCores::Shared {
                    Tone::Dim
                } else {
                    Tone::Normal
                },
            )],
            lines: vec![format!("sound cores: {}", mode.label())],
            noted: Counts::default(),
            noted_at: None,
        };
        if mode == SoundCores::Shared {
            return c;
        }
        let ((text, tone), pinned) = match pins.audio {
            Some(core) => match mpfree_lanes::os::pin(core) {
                Ok(()) => ((format!("core {core} only"), Tone::Good), true),
                Err(e) => (
                    (format!("not pinned to core {core}: {e}"), Tone::Bad),
                    false,
                ),
            },
            None => (
                ("left where it is (no core asked)".to_string(), Tone::Dim),
                true,
            ),
        };
        c.lines.push(format!("sound cores: audio thread {text}"));
        c.rows.push(("Audio thread".to_string(), text, tone));
        if pinned && pins.audio.is_some() {
            // The helper threads started from now on keep off the sound's
            // cores.
            mpfree_lanes::os::set_background(&BACKGROUND_CORES);
            let cores = BACKGROUND_CORES.map(|c| c.to_string()).join(" and ");
            c.lines.push(format!(
                "sound cores: helper threads on cores {cores}, normal priority"
            ));
            c.rows.push((
                "Helper threads".to_string(),
                format!("cores {cores}, normal priority"),
                Tone::Normal,
            ));
        }
        if mode.lanes() && !pinned {
            // A lane's worker starts with this thread's cores: without the
            // pin it could land anywhere, beside this thread included.
            let text =
                "not started: the audio thread is not on its core, so everything plays on it";
            c.lines.push(format!("sound cores: lanes {text}"));
            c.rows
                .push(("Lanes".to_string(), text.to_string(), Tone::Bad));
            return c;
        }
        c.lanes = mode.lanes();
        if c.lanes {
            let setup = player.split(c.lane("mpfree-lane-smp", pins.lane));
            let (text, tone) = match &setup {
                Some(s) => lane_text(s, "half the drum voices"),
                None => ("not split (fewer than 2 voices)".to_string(), Tone::Bad),
            };
            c.lines.push(format!("sound cores: sampler lane {text}"));
            c.rows.push(("Drum voices lane".to_string(), text, tone));
        }
        if mode == SoundCores::Three {
            let setup = synth.give_lane(c.lane("mpfree-lane-syn", pins.synth));
            let (text, tone) = lane_text(&setup, "the synth beside the drums");
            c.lines.push(format!("sound cores: synth lane {text}"));
            c.rows.push(("Synth lane".to_string(), text, tone));
        }
        c
    }

    /// `two` and `three`: runs about half the pad channels' and tracks'
    /// inserts on a lane on core 2, and the reverb returns a block late on
    /// another lane on core 2 one priority lower. Call it right after the
    /// mixer is made, before the loop: it allocates and starts the mixer's
    /// lane threads.
    pub fn start_mixer(&mut self, mixer: &mut Mixer) {
        if !self.lanes {
            return;
        }
        let cfg = self.lane("mpfree-lane-mix", self.pins.lane);
        let (text, tone) = match mixer.set_lane(Some(cfg), PERIOD_TIME) {
            Some(s) => lane_text(&s, "about half the inserts"),
            None => ("not started".to_string(), Tone::Bad),
        };
        self.lines.push(format!("sound cores: mixer lane {text}"));
        self.rows
            .push(("Mixer inserts lane".to_string(), text, tone));
        let cfg = LaneConfig {
            fifo: self.pins.fifo.map(|_| RETURNS_FIFO),
            ..self.lane("mpfree-lane-returns", self.pins.lane)
        };
        let (text, tone) = match mixer.set_pipelined_returns(Some(cfg), PERIOD_TIME) {
            Some(s) => lane_text(&s, "reverb returns a block late"),
            None => ("not started".to_string(), Tone::Bad),
        };
        self.lines.push(format!("sound cores: returns lane {text}"));
        self.rows
            .push(("Reverb returns lane".to_string(), text, tone));
    }

    fn lane(&self, name: &str, core: Option<usize>) -> LaneConfig {
        LaneConfig {
            name: name.to_string(),
            core,
            fifo: self.pins.fifo,
            strict: true,
            ..LaneConfig::default()
        }
    }

    /// The lines for the log, once: how each part started.
    pub fn take_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.lines)
    }

    /// Hands the set-up rows to the Dev page (once, before the loop; it
    /// allocates).
    pub fn show(&self, shared: &CoreShared) {
        let _ = shared.setup.set((self.mode, self.lanes, self.rows.clone()));
    }

    /// Publishes the counts for the Dev page (no allocation).
    pub fn publish(&self, shared: &CoreShared, c: Counts) {
        shared.store(c);
    }

    /// A line for the log when a lane was late since the last line, at
    /// most one every second; else None (and nothing allocates).
    pub fn late_note(&mut self, c: Counts, now: Instant) -> Option<String> {
        if c.late() == self.noted.late() {
            return None;
        }
        if self
            .noted_at
            .is_some_and(|t| now.saturating_duration_since(t) < NOTE_EVERY)
        {
            return None;
        }
        let (dv, dm, ds, dw) = (
            c.voice_late - self.noted.voice_late,
            c.mix_late - self.noted.mix_late,
            c.synth_late - self.noted.synth_late,
            c.synth_waits - self.noted.synth_waits,
        );
        let (dr, dl) = (
            c.returns_late - self.noted.returns_late,
            c.returns_lost - self.noted.returns_lost,
        );
        self.noted = c;
        self.noted_at = Some(now);
        let s = |n: u64| if n == 1 { "" } else { "s" };
        Some(format!(
            "sound cores: late: {dv} part{} with half the drum voices unheard, {dm} mixer stage{} \
             with those strips silent, {dr} returns block{} with the reverbs silent, {dl} block{} \
             of a return's input lost and {ds} period{} with the synth left out (fallbacks), {dw} \
             wait{} for the synth past the next deadline; {}, {}, {}, {}, {} and {} since the start",
            s(dv),
            s(dm),
            s(dr),
            s(dl),
            s(ds),
            s(dw),
            c.voice_late,
            c.mix_late,
            c.returns_late,
            c.returns_lost,
            c.synth_late,
            c.synth_waits
        ))
    }

    /// For the 30 s status line and the end of the session.
    pub fn status(&self, c: Counts) -> String {
        let lanes = || {
            format!(
                "sound cores {}: drum voices {} parts on core 2, {} late; mixer {} stages on core 2, \
                 {} late; returns {} blocks on core 2, {} late, {} starts, {} stops, {} lost",
                self.mode.name(),
                c.voice_parts,
                c.voice_late,
                c.mix_stages,
                c.mix_late,
                c.returns_blocks,
                c.returns_late,
                c.returns_starts,
                c.returns_stops,
                c.returns_lost
            )
        };
        match self.mode {
            SoundCores::Two => lanes(),
            SoundCores::Three => format!(
                "{}; synth {} periods on core 1, {} late (left out), {} waited for",
                lanes(),
                c.synth_periods,
                c.synth_late,
                c.synth_waits
            ),
            m => format!("sound cores {}", m.name()),
        }
    }
}

/// How a lane started: its core and priority, or why not.
fn lane_text(s: &LaneSetup, what: &str) -> (String, Tone) {
    if let Err(e) = &s.threaded {
        return (format!("on the audio thread ({e})"), Tone::Bad);
    }
    let mut tone = Tone::Good;
    let core = match &s.pinned {
        Some(Ok(c)) => format!("core {c}"),
        Some(Err(e)) => {
            tone = Tone::Bad;
            format!("not pinned ({e})")
        }
        None => "any core".to_string(),
    };
    let prio = match &s.fifo {
        Some(Ok(p)) => format!("FIFO {p}"),
        Some(Err(e)) => {
            if tone == Tone::Good {
                tone = Tone::Warn;
            }
            format!("normal priority ({e})")
        }
        None => "the priority it inherits".to_string(),
    };
    (format!("{core}, {prio}: {what}"), tone)
}

/// The Dev page's rows: how the sound's cores started, and (`two`,
/// `three`) the lanes' parts and late ones. Empty until the audio loop has
/// set up.
pub fn rows(shared: &CoreShared) -> Vec<Row> {
    let Some((mode, lanes, setup)) = shared.setup.get() else {
        return Vec::new();
    };
    let mut rows = setup.clone();
    if !lanes {
        return rows;
    }
    let c = shared.counts();
    let tone = |late: u64| if late == 0 { Tone::Good } else { Tone::Bad };
    {
        rows.push((
            "Drum voices on core 2 · late".to_string(),
            format!("{} parts · {} late", c.voice_parts, c.voice_late),
            tone(c.voice_late),
        ));
        rows.push((
            "Mixer inserts on core 2 · late".to_string(),
            format!("{} stages · {} late", c.mix_stages, c.mix_late),
            tone(c.mix_late),
        ));
        rows.push((
            "Reverb returns on core 2 · late · lost".to_string(),
            format!(
                "{} blocks · {} late · {} lost",
                c.returns_blocks, c.returns_late, c.returns_lost
            ),
            tone(c.returns_late + c.returns_lost),
        ));
    }
    if *mode == SoundCores::Three {
        rows.push((
            "Synth on core 1 · late · waited".to_string(),
            format!(
                "{} periods · {} late · {} waited",
                c.synth_periods, c.synth_late, c.synth_waits
            ),
            tone(c.synth_late + c.synth_waits),
        ));
    }
    rows
}

/// What the Dev page shows on the MPC with `two`, for the sample screens
/// (`--preview` and the golden screens): fixed set-up rows and counts,
/// with 3 late mixer stages to show how a late one looks.
pub fn sample(shared: &CoreShared) {
    let lane_at = |fifo: i32, what: &str| {
        lane_text(
            &LaneSetup {
                threaded: Ok(()),
                pinned: Some(Ok(LANE_CORE)),
                fifo: Some(Ok(fifo)),
            },
            what,
        )
    };
    let lane = |what: &str| lane_at(LANE_FIFO, what);
    let row = |label: &str, (text, tone): (String, Tone)| (label.to_string(), text, tone);
    let rows = vec![
        row(
            "Sound cores",
            (SoundCores::Two.label().to_string(), Tone::Normal),
        ),
        row(
            "Audio thread",
            (format!("core {AUDIO_CORE} only"), Tone::Good),
        ),
        row("Drum voices lane", lane("half the drum voices")),
        row("Mixer inserts lane", lane("about half the inserts")),
        row(
            "Reverb returns lane",
            lane_at(RETURNS_FIFO, "reverb returns a block late"),
        ),
    ];
    let _ = shared.setup.set((SoundCores::Two, true, rows));
    shared.store(Counts {
        voice_parts: 41_210,
        voice_late: 0,
        mix_stages: 20_640,
        mix_late: 3,
        returns_blocks: 20_610,
        returns_starts: 1,
        ..Counts::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit;
    use mpfree_sampler::engine::PadOut;

    const NOTES: [u8; 16] = [
        0x25, 0x24, 0x2A, 0x52, 0x28, 0x26, 0x2E, 0x2C, 0x30, 0x2F, 0x2D, 0x2B, 0x31, 0x37, 0x33,
        0x35,
    ];
    fn player() -> Player {
        let tone: Vec<i16> = (0..4000)
            .flat_map(|i| {
                let x = (i as f32 * 0.02).sin();
                [(x * 12000.0) as i16, (x * -9000.0) as i16]
            })
            .collect();
        let a = kit::build(&[
            (NOTES[0], "Kick", &tone[..]),
            (NOTES[5], "Snare", &tone[..]),
        ]);
        let mut kits = kit::parse_kits(&kit::build_kits(&[("House", a)])).unwrap();
        Player::new(&mut kits, &NOTES, 44_100).unwrap()
    }
    fn slot() -> SynthSlot {
        SynthSlot::new(Synth::new(44_100))
    }

    #[test]
    fn the_names_parse_and_shared_is_the_default() {
        assert_eq!(SoundCores::ALL.len(), 4);
        for m in SoundCores::ALL {
            assert_eq!(SoundCores::parse(m.name()), Some(m));
        }
        assert_eq!(SoundCores::parse("three"), Some(SoundCores::Three));
        assert_eq!(SoundCores::parse("3"), None);
        assert_eq!(SoundCores::default(), SoundCores::Shared);
    }

    #[test]
    fn shared_changes_nothing() {
        let mut p = player();
        let mut mix = crate::mixing::MixHost::new(44_100, -6.0, -6.0);
        let mut c = Cores::start_on(SoundCores::Shared, &mut p, &mut slot(), NONE);
        c.start_mixer(&mut mix.mixer);
        assert_eq!(p.lane_stats(), None);
        assert!(!mix.mixer.lane_on());
        assert!(!mix.mixer.pipelined_returns());
        let shared = CoreShared::default();
        assert!(rows(&shared).is_empty(), "nothing before the loop sets up");
        c.show(&shared);
        let r = rows(&shared);
        assert_eq!(r.len(), 1, "{r:?}");
        assert!(r[0].1.starts_with("shared: "), "{r:?}");
        assert_eq!(c.status(Counts::default()), "sound cores shared");
        assert_eq!(c.take_lines().len(), 1);
    }

    #[test]
    fn two_splits_the_voices_and_the_mixer_and_counts_their_parts() {
        let mut p = player();
        let mut mix = crate::mixing::MixHost::new(44_100, -6.0, -6.0);
        let mut c = Cores::start_on(SoundCores::Two, &mut p, &mut slot(), NONE);
        c.start_mixer(&mut mix.mixer);
        assert!(p.lane_stats().is_some(), "the sampler is split");
        assert!(mix.mixer.lane_on(), "the mixer has its lane");
        assert!(mix.mixer.pipelined_returns(), "the returns go late");
        let lines = c.take_lines();
        assert_eq!(lines.len(), 5, "{lines:?}");
        assert!(
            lines[2].contains("any core, the priority it inherits: half the drum voices"),
            "{lines:?}"
        );
        assert!(
            lines[3].starts_with("sound cores: mixer lane any core"),
            "{lines:?}"
        );
        assert_eq!(
            lines[4],
            "sound cores: returns lane any core, the priority it inherits: reverb returns a block late"
        );
        // Two hits: the second goes to the lane's half of the voices. A far
        // deadline, so a slow machine (qemu in CI) is never late here.
        struct Pads([[[f32; 128]; 2]; 16]);
        impl PadOut for Pads {
            fn pad(&mut self, pad: usize) -> Option<(&mut [f32], &mut [f32])> {
                let [l, r] = self.0.get_mut(pad)?;
                Some((&mut l[..], &mut r[..]))
            }
        }
        let mut pads = Pads([[[0.0; 128]; 2]; 16]);
        assert!(p.hit_at(0, 110, 0));
        assert!(p.hit_at(5, 90, 0));
        for _ in 0..20 {
            p.render_routed_until(128, &mut pads, Instant::now() + Duration::from_secs(5));
        }
        let n = Counts::read(&p, &mix.mixer, &slot());
        assert!(n.voice_parts > 0, "{n:?}");
        assert_eq!(n.voice_late, 0, "{n:?}");
        let shared = CoreShared::default();
        c.show(&shared);
        c.publish(&shared, n);
        let rows = rows(&shared);
        let labels: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Sound cores",
                "Audio thread",
                "Drum voices lane",
                "Mixer inserts lane",
                "Reverb returns lane",
                "Drum voices on core 2 · late",
                "Mixer inserts on core 2 · late",
                "Reverb returns on core 2 · late · lost"
            ]
        );
        assert_eq!(rows[5].1, format!("{} parts · 0 late", n.voice_parts));
        assert_eq!(rows[5].2, Tone::Good);
        assert_eq!(rows[7].1, "0 blocks · 0 late · 0 lost");
        assert_eq!(rows[7].2, Tone::Good);
        assert!(c.status(n).starts_with("sound cores two: drum voices "));
    }

    #[test]
    fn a_late_lane_is_logged_at_once_and_at_most_once_a_second() {
        let mut p = player();
        let mut c = Cores::start_on(SoundCores::Core3, &mut p, &mut slot(), NONE);
        let t = Instant::now();
        let mut n = Counts {
            voice_parts: 100,
            mix_stages: 40,
            ..Counts::default()
        };
        assert_eq!(c.late_note(n, t), None, "nothing late");
        n.voice_late = 1;
        let line = c.late_note(n, t).unwrap();
        assert!(
            line.starts_with(
                "sound cores: late: 1 part with half the drum voices unheard, 0 mixer stages"
            ),
            "{line}"
        );
        n.mix_late = 2;
        assert_eq!(c.late_note(n, t + Duration::from_millis(500)), None);
        let line = c.late_note(n, t + Duration::from_millis(1000)).unwrap();
        assert!(line.contains("0 parts with half"), "{line}");
        assert!(
            line.contains(
                "2 mixer stages with those strips silent, 0 returns blocks with the reverbs \
                 silent, 0 blocks of a return's input lost and 0 periods with the synth left out \
                 (fallbacks), 0 waits"
            ),
            "{line}"
        );
        assert!(
            line.ends_with("; 1, 2, 0, 0, 0 and 0 since the start"),
            "{line}"
        );
        // A late synth is left out, and a wait for it past the next
        // deadline is logged the same way.
        n.synth_late = 1;
        n.synth_waits = 1;
        let line = c.late_note(n, t + Duration::from_secs(2)).unwrap();
        assert!(
            line.contains(
                "1 period with the synth left out (fallbacks), 1 wait for the synth past the next deadline"
            ),
            "{line}"
        );
        assert_eq!(
            c.late_note(n, t + Duration::from_secs(5)),
            None,
            "no new ones"
        );
        // A late returns block, and a block of a return's input lost.
        n.returns_blocks = 30;
        n.returns_late = 1;
        n.returns_lost = 1;
        let line = c.late_note(n, t + Duration::from_secs(6)).unwrap();
        assert!(
            line.contains(
                "1 returns block with the reverbs silent, 1 block of a return's input lost"
            ),
            "{line}"
        );
        assert!(
            line.ends_with("; 1, 2, 1, 1, 1 and 1 since the start"),
            "{line}"
        );
        // Late ones show red on the Dev page.
        let shared = CoreShared::default();
        let mut two = Cores::start_on(SoundCores::Two, &mut p, &mut slot(), NONE);
        two.start_mixer(&mut mixing_host().mixer);
        two.show(&shared);
        two.publish(&shared, n);
        let r = rows(&shared);
        let mix = r
            .iter()
            .find(|r| r.0.starts_with("Mixer inserts on"))
            .unwrap();
        assert_eq!((mix.1.as_str(), mix.2), ("40 stages · 2 late", Tone::Bad));
        assert_eq!(r.last().unwrap().1, "30 blocks · 1 late · 1 lost");
        assert_eq!(r.last().unwrap().2, Tone::Bad);
        assert!(two
            .status(n)
            .ends_with("; returns 30 blocks on core 2, 1 late, 0 starts, 0 stops, 1 lost"));
    }

    /// The synth's notes for a few periods: two held notes and pressure.
    fn play(s: &mut SynthSlot, i: u64) {
        match i {
            0 => s.note_on(0, 110, 0),
            3 => s.note_on(5, 80, 40),
            9 => s.pressure(0, 90),
            30 => s.note_off(0, 7),
            _ => {}
        }
    }

    #[test]
    fn three_plays_the_synth_on_its_lane_with_the_same_sound() {
        let mut p = player();
        let mut mix = crate::mixing::MixHost::new(44_100, -6.0, -6.0);
        let mut here = slot();
        let mut there = slot();
        let mut c = Cores::start_on(SoundCores::Three, &mut p, &mut there, NONE);
        c.start_mixer(&mut mix.mixer);
        assert!(p.lane_stats().is_some(), "the sampler is split too");
        assert!(mix.mixer.lane_on(), "and the mixer has its lane");
        assert!(mix.mixer.pipelined_returns(), "and the returns go late");
        let lines = c.take_lines();
        assert_eq!(lines.len(), 6, "{lines:?}");
        assert_eq!(
            lines[3],
            "sound cores: synth lane any core, the priority it inherits: the synth beside the drums"
        );
        let mut heard = 0.0f32;
        for i in 0..60u64 {
            play(&mut here, i);
            play(&mut there, i);
            let (mut a, mut b) = (
                ([0.0f32; PERIOD], [0.0f32; PERIOD]),
                ([0.0; PERIOD], [0.0; PERIOD]),
            );
            // A far deadline, so a slow machine (qemu in CI) is never late.
            let far = Instant::now() + Duration::from_secs(5);
            here.start(i * PERIOD as u64, 120.0);
            here.finish(far, &mut a.0, &mut a.1);
            there.start(i * PERIOD as u64, 120.0);
            // The synth is on its lane now; the drums would render here.
            there.finish(far, &mut b.0, &mut b.1);
            there.settle(far);
            assert_eq!(
                a, b,
                "period {i}: the lane's sound is the same, bit for bit"
            );
            heard = heard.max(a.0.iter().fold(0.0, |m, x| m.max(x.abs())));
        }
        assert!(heard > 0.01, "the synth played: peak {heard}");
        assert_eq!(here.active_voices(), there.active_voices());
        let n = Counts::read(&p, &mix.mixer, &there);
        assert_eq!(n.synth_periods, 60, "{n:?}");
        assert_eq!(Counts::read(&p, &mix.mixer, &here).synth_periods, 0);
        let shared = CoreShared::default();
        c.show(&shared);
        c.publish(&shared, n);
        let rows = rows(&shared);
        let labels: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Sound cores",
                "Audio thread",
                "Drum voices lane",
                "Synth lane",
                "Mixer inserts lane",
                "Reverb returns lane",
                "Drum voices on core 2 · late",
                "Mixer inserts on core 2 · late",
                "Reverb returns on core 2 · late · lost",
                "Synth on core 1 · late · waited"
            ]
        );
        assert_eq!(rows[9].1, "60 periods · 0 late · 0 waited");
        let status = c.status(n);
        assert!(
            status.starts_with("sound cores three: drum voices "),
            "{status}"
        );
        assert!(
            status.ends_with("; synth 60 periods on core 1, 0 late (left out), 0 waited for"),
            "{status}"
        );
    }

    #[test]
    fn a_late_synth_is_left_out_and_taken_back_after_the_write() {
        let mut p = player();
        let mut there = slot();
        let _c = Cores::start_on(SoundCores::Three, &mut p, &mut there, NONE);
        there.note_on(0, 110, 0);
        there.note_on(5, 90, 0);
        let mut left_out = 0;
        for i in 0..200u64 {
            let (mut l, mut r) = ([0.0f32; PERIOD], [0.0f32; PERIOD]);
            there.start(i * PERIOD as u64, 120.0);
            // A deadline that has passed: the lane has barely woken.
            let before = there.late;
            there.finish(Instant::now(), &mut l, &mut r);
            if there.late > before {
                left_out += 1;
                assert!(there.home.is_none(), "period {i}: still on its lane");
                assert!(
                    l.iter().chain(&r).all(|&x| x == 0.0),
                    "period {i}: left out, nothing added"
                );
            }
            // Right after the write: the synth comes back, and the loop
            // can play it again.
            there.settle(Instant::now() + Duration::from_secs(5));
            assert!(there.home.is_some(), "period {i}: back");
            assert!(there.active_voices() > 0);
        }
        let n = Counts::read(&p, &mixing_host().mixer, &there);
        assert_eq!(n.synth_periods, 200);
        assert_eq!(n.synth_late, left_out);
        assert_eq!(
            n.synth_waits, 0,
            "a far deadline after the write never waits"
        );
        // The worker needs a wake-up and a render; a deadline already
        // passed is late nearly every time.
        assert!(left_out > 0, "some periods were late");
        // Settling with a deadline that has passed: a synth still away is
        // waited for (counted), and is back either way.
        for i in 200..260u64 {
            let (mut l, mut r) = ([0.0f32; PERIOD], [0.0f32; PERIOD]);
            there.start(i * PERIOD as u64, 120.0);
            there.finish(Instant::now(), &mut l, &mut r);
            there.settle(Instant::now());
            assert!(there.home.is_some());
        }
        assert!(there.waits <= there.late);
    }

    fn mixing_host() -> crate::mixing::MixHost {
        crate::mixing::MixHost::new(44_100, -6.0, -6.0)
    }

    #[test]
    fn the_deadline_is_when_the_queue_would_run_dry_less_a_period() {
        let t = Instant::now();
        let period = |n: u64| Duration::from_nanos(n * PERIOD as u64 * 1_000_000_000 / 44_100);
        // Three periods queued after the write: two periods to go.
        assert_eq!(lane_deadline(t, Some(3 * PERIOD as i64), 3), t + period(2));
        // After a long period, a period and a half queued: half a period.
        assert_eq!(
            lane_deadline(t, Some(PERIOD as i64 * 3 / 2), 3),
            t + Duration::from_nanos(PERIOD as u64 / 2 * 1_000_000_000 / 44_100)
        );
        // A period or less: the deadline is now (only what is done counts).
        assert_eq!(lane_deadline(t, Some(PERIOD as i64), 3), t);
        assert_eq!(lane_deadline(t, Some(-40), 3), t);
        // No delay read: a full queue is assumed; a delay past the buffer
        // (the card's own latency on top) is held to it.
        assert_eq!(lane_deadline(t, None, 3), t + period(2));
        assert_eq!(lane_deadline(t, Some(10_000), 3), t + period(2));
        assert_eq!(lane_deadline(t, None, 2), t + period(1));
    }

    #[test]
    fn no_lane_starts_when_the_audio_thread_is_not_on_its_core() {
        let mut p = player();
        let mut mix = mixing_host();
        let mut synth = slot();
        // Core 100000 is past any machine, so the pin fails.
        let pins = Pins {
            audio: Some(100_000),
            ..NONE
        };
        let mut c = Cores::start_on(SoundCores::Three, &mut p, &mut synth, pins);
        c.start_mixer(&mut mix.mixer);
        assert_eq!(p.lane_stats(), None, "the drums are not split");
        assert!(!mix.mixer.lane_on(), "the mixer has no lane");
        assert!(!mix.mixer.pipelined_returns(), "the returns stay on time");
        assert!(synth.lane.is_none(), "the synth has no lane");
        let shared = CoreShared::default();
        c.show(&shared);
        let r = rows(&shared);
        let labels: Vec<&str> = r.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(labels, ["Sound cores", "Audio thread", "Lanes"]);
        assert_eq!(r[1].2, Tone::Bad);
        assert!(r[2].1.starts_with("not started: "), "{r:?}");
        assert_eq!(r[2].2, Tone::Bad);
        let lines = c.take_lines();
        assert!(
            lines
                .last()
                .unwrap()
                .starts_with("sound cores: lanes not started"),
            "{lines:?}"
        );
    }

    #[test]
    fn a_lane_that_cannot_have_its_core_plays_on_the_audio_thread() {
        let mut p = player();
        let mut mix = mixing_host();
        let pins = Pins {
            lane: Some(100_000),
            ..NONE
        };
        let mut c = Cores::start_on(SoundCores::Two, &mut p, &mut slot(), pins);
        c.start_mixer(&mut mix.mixer);
        let lines = c.take_lines();
        assert!(
            lines[2].starts_with(
                "sound cores: sampler lane on the audio thread (strict lane, not pinned: "
            ),
            "{lines:?}"
        );
        assert!(
            lines[3].starts_with("sound cores: mixer lane on the audio thread (strict lane"),
            "{lines:?}"
        );
        assert!(
            lines[4].starts_with("sound cores: returns lane on the audio thread (strict lane"),
            "{lines:?}"
        );
        let shared = CoreShared::default();
        c.show(&shared);
        let r = rows(&shared);
        assert_eq!(r[2].2, Tone::Bad, "{r:?}");
        assert_eq!(r[3].2, Tone::Bad, "{r:?}");
        assert_eq!(r[4].2, Tone::Bad, "{r:?}");
    }

    #[test]
    fn a_lane_that_did_not_get_its_core_or_priority_says_so() {
        let s = LaneSetup {
            threaded: Ok(()),
            pinned: Some(Ok(2)),
            fifo: Some(Err("Operation not permitted".into())),
        };
        assert_eq!(
            lane_text(&s, "x"),
            (
                "core 2, normal priority (Operation not permitted): x".to_string(),
                Tone::Warn
            )
        );
        let s = LaneSetup {
            threaded: Ok(()),
            pinned: Some(Err("Invalid argument".into())),
            fifo: Some(Ok(69)),
        };
        assert_eq!(lane_text(&s, "x").1, Tone::Bad);
        let s = LaneSetup {
            threaded: Err("no thread".into()),
            pinned: None,
            fifo: None,
        };
        assert_eq!(
            lane_text(&s, "x"),
            ("on the audio thread (no thread)".to_string(), Tone::Bad)
        );
    }
}
