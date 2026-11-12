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
//! A late lane is never silent (Jay's rule, 12 Nov). A late sampler part
//! leaves that half of the voices out of that part of the period, and a
//! late mixer stage leaves those strips silent for the block: both are
//! fallbacks. A late synth leaves nothing out: the audio thread waits for
//! it, so the period runs long. All three are counted on the Dev tab's
//! "CPU by engine" page and in the 30 s status line, and a line goes to
//! the log within a second of one.
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

/// A period: 128 frames at 44.1 kHz. The mixer waits this long for its
/// lane each stage; the synth's lane is late once it takes longer; the
/// sampler's lane gets as long as its part of the period lasts
/// (`Engine::render_routed`).
const PERIOD_TIME: Duration = Duration::from_nanos(128 * 1_000_000_000 / 44_100);

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
    /// Of those, the ones it took longer than a period for (the audio
    /// thread waited: nothing left out, the period ran long).
    pub synth_late: u64,
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
        }
    }

    fn late(&self) -> u64 {
        self.voice_late + self.mix_late + self.synth_late
    }

    fn values(&self) -> [u64; 6] {
        [
            self.voice_parts,
            self.voice_late,
            self.mix_stages,
            self.mix_late,
            self.synth_periods,
            self.synth_late,
        ]
    }
}

/// The synth, played on the audio thread, or (`three`) on a lane on core
/// 1 while the drums render. Between periods it is always here, so the
/// rest of the audio loop uses it as a [`Synth`] (through `Deref`); only
/// between [`SynthSlot::start`] and [`SynthSlot::finish`] is it away.
pub struct SynthSlot {
    home: Option<Box<Synth>>,
    lane: Option<Lane<SynthJob>>,
    frame: u64,
    bpm: f64,
    posted_at: Option<Instant>,
    periods: u64,
    late: u64,
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
            posted_at: None,
            periods: 0,
            late: 0,
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
            self.posted_at = Some(Instant::now());
        } else if let Some(job) = lane.job() {
            // Never reached (finish always takes the synth back): play here.
            self.home = job.synth.take();
        }
    }

    /// Adds the period's synth into `left` and `right` (cleared by the
    /// caller, as [`Synth::render`] adds), the same sound either way. On
    /// its lane it waits for the synth as long as it takes: past a period
    /// that counts as late. No allocation.
    pub fn finish(&mut self, left: &mut [f32], right: &mut [f32]) {
        let (Some(lane), Some(at)) = (self.lane.as_mut(), self.posted_at.take()) else {
            if let Some(s) = self.home.as_deref_mut() {
                s.render(self.frame, self.bpm, left, right);
            }
            return;
        };
        if lane.join(at + PERIOD_TIME).is_err() {
            self.late += 1;
            while lane.join(Instant::now() + Duration::from_secs(1)).is_err() {}
        }
        self.periods += 1;
        if let Some(job) = lane.job() {
            self.home = job.synth.take();
            for (o, s) in left.iter_mut().zip(job.left) {
                *o += s;
            }
            for (o, s) in right.iter_mut().zip(job.right) {
                *o += s;
            }
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
    setup: OnceLock<(SoundCores, Vec<Row>)>,
    counts: [AtomicU64; 6],
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
        let (text, tone) = match pins.audio {
            Some(core) => match mpfree_lanes::os::pin(core) {
                Ok(()) => (format!("core {core} only"), Tone::Good),
                Err(e) => (format!("not pinned to core {core}: {e}"), Tone::Bad),
            },
            None => ("left where it is (no core asked)".to_string(), Tone::Dim),
        };
        c.lines.push(format!("sound cores: audio thread {text}"));
        c.rows.push(("Audio thread".to_string(), text, tone));
        if mode.lanes() {
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

    /// `two`: runs about half the pad channels' and tracks' inserts on a
    /// lane on core 2. Call it right after the mixer is made, before the
    /// loop: it allocates and starts the mixer's lane thread.
    pub fn start_mixer(&mut self, mixer: &mut Mixer) {
        if !self.mode.lanes() {
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
    }

    fn lane(&self, name: &str, core: Option<usize>) -> LaneConfig {
        LaneConfig {
            name: name.to_string(),
            core,
            fifo: self.pins.fifo,
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
        let _ = shared.setup.set((self.mode, self.rows.clone()));
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
        let (dv, dm, ds) = (
            c.voice_late - self.noted.voice_late,
            c.mix_late - self.noted.mix_late,
            c.synth_late - self.noted.synth_late,
        );
        self.noted = c;
        self.noted_at = Some(now);
        let s = |n: u64| if n == 1 { "" } else { "s" };
        Some(format!(
            "sound cores: late: {dv} part{} with half the drum voices unheard and {dm} mixer stage{} \
             with those strips silent (fallbacks), {ds} period{} waiting past a period for the synth; \
             {}, {} and {} since the start",
            s(dv),
            s(dm),
            s(ds),
            c.voice_late,
            c.mix_late,
            c.synth_late
        ))
    }

    /// For the 30 s status line and the end of the session.
    pub fn status(&self, c: Counts) -> String {
        let lanes = || {
            format!(
                "sound cores {}: drum voices {} parts on core 2, {} late; mixer {} stages on core 2, {} late",
                self.mode.name(),
                c.voice_parts,
                c.voice_late,
                c.mix_stages,
                c.mix_late
            )
        };
        match self.mode {
            SoundCores::Two => lanes(),
            SoundCores::Three => format!(
                "{}; synth {} periods on core 1, {} late",
                lanes(),
                c.synth_periods,
                c.synth_late
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
    let Some((mode, setup)) = shared.setup.get() else {
        return Vec::new();
    };
    let mut rows = setup.clone();
    let c = shared.counts();
    let tone = |late: u64| if late == 0 { Tone::Good } else { Tone::Bad };
    if mode.lanes() {
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
    }
    if *mode == SoundCores::Three {
        rows.push((
            "Synth on core 1 · late".to_string(),
            format!("{} periods · {} late", c.synth_periods, c.synth_late),
            tone(c.synth_late),
        ));
    }
    rows
}

/// What the Dev page shows on the MPC with `two`, for the sample screens
/// (`--preview` and the golden screens): fixed set-up rows and counts,
/// with 3 late mixer stages to show how a late one looks.
pub fn sample(shared: &CoreShared) {
    let lane = |what: &str| {
        lane_text(
            &LaneSetup {
                threaded: Ok(()),
                pinned: Some(Ok(LANE_CORE)),
                fifo: Some(Ok(LANE_FIFO)),
            },
            what,
        )
    };
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
    ];
    let _ = shared.setup.set((SoundCores::Two, rows));
    shared.store(Counts {
        voice_parts: 41_210,
        voice_late: 0,
        mix_stages: 20_640,
        mix_late: 3,
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
        let lines = c.take_lines();
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(
            lines[2].contains("any core, the priority it inherits: half the drum voices"),
            "{lines:?}"
        );
        assert!(
            lines[3].starts_with("sound cores: mixer lane any core"),
            "{lines:?}"
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
                "Drum voices on core 2 · late",
                "Mixer inserts on core 2 · late"
            ]
        );
        assert_eq!(rows[4].1, format!("{} parts · 0 late", n.voice_parts));
        assert_eq!(rows[4].2, Tone::Good);
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
                "sound cores: late: 1 part with half the drum voices unheard and 0 mixer stages"
            ),
            "{line}"
        );
        n.mix_late = 2;
        assert_eq!(c.late_note(n, t + Duration::from_millis(500)), None);
        let line = c.late_note(n, t + Duration::from_millis(1000)).unwrap();
        assert!(line.contains("0 parts with half"), "{line}");
        assert!(
            line.contains("2 mixer stages with those strips silent (fallbacks), 0 periods"),
            "{line}"
        );
        assert!(line.ends_with("; 1, 2 and 0 since the start"), "{line}");
        // A late synth is waited for, not left out, and logged the same way.
        n.synth_late = 1;
        let line = c.late_note(n, t + Duration::from_secs(2)).unwrap();
        assert!(
            line.contains("1 period waiting past a period for the synth"),
            "{line}"
        );
        assert_eq!(
            c.late_note(n, t + Duration::from_secs(5)),
            None,
            "no new ones"
        );
        // Late ones show red on the Dev page.
        let shared = CoreShared::default();
        let two = Cores::start_on(SoundCores::Two, &mut p, &mut slot(), NONE);
        two.show(&shared);
        two.publish(&shared, n);
        let r = rows(&shared);
        assert_eq!(r.last().unwrap().1, "40 stages · 2 late");
        assert_eq!(r.last().unwrap().2, Tone::Bad);
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
        let lines = c.take_lines();
        assert_eq!(lines.len(), 5, "{lines:?}");
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
            here.start(i * PERIOD as u64, 120.0);
            here.finish(&mut a.0, &mut a.1);
            there.start(i * PERIOD as u64, 120.0);
            // The synth is on its lane now; the drums would render here.
            there.finish(&mut b.0, &mut b.1);
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
                "Drum voices on core 2 · late",
                "Mixer inserts on core 2 · late",
                "Synth on core 1 · late"
            ]
        );
        assert_eq!(rows[7].1, format!("60 periods · {} late", n.synth_late));
        let status = c.status(n);
        assert!(
            status.starts_with("sound cores three: drum voices "),
            "{status}"
        );
        assert!(
            status.ends_with(&format!(
                "; synth 60 periods on core 1, {} late",
                n.synth_late
            )),
            "{status}"
        );
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
