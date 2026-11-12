//! Where the sound runs: Prefs "Sound cores" (Shared, Core 3, Cores 3 and
//! 2; kept in the "app" settings as `sound-cores`, used from the next
//! start), or `--sound-cores shared|core3|two` with `play` or `app` for one
//! run. The default is `shared`, as before.
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
//!
//! A late lane is a fallback, and never a silent one (Jay's rule, 12 Nov).
//! A late sampler part leaves that half of the voices out of that part of
//! the period; a late mixer stage leaves those strips silent for the block.
//! Both are counted on the Dev tab's "CPU by engine" page and in the 30 s
//! status line, and a line goes to the log within a second of one.
//!
//! The mode is set once, on the audio thread, before the sound starts.
//! Only the program's own threads change (their core and priority, in
//! RAM): quitting the program undoes it, and nothing is written anywhere.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use mpfree_fx::Mixer;
use mpfree_lanes::{Config as LaneConfig, Setup as LaneSetup};
use mpfree_ui::Tone;

use crate::sampler::Player;

/// One row of the Dev page: what, its value, its colour.
pub type Row = (String, String, Tone);

/// The MPC's plan (docs/dev/design/concurrency.md): the screen and the
/// rest on cores 0 and 1, the audio thread alone on core 3, the lanes on
/// core 2 one priority below it.
pub const AUDIO_CORE: usize = 3;
pub const LANE_CORE: usize = 2;
pub const LANE_FIFO: i32 = 69;

/// How long the mixer waits for its lane each stage: a period (128 frames
/// at 44.1 kHz). The sampler's lane gets as long as its part of the period
/// lasts (`Engine::render_routed`).
const MIX_WAIT: Duration = Duration::from_nanos(128 * 1_000_000_000 / 44_100);

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
}

impl SoundCores {
    pub const ALL: [SoundCores; 3] = [SoundCores::Shared, SoundCores::Core3, SoundCores::Two];
    pub const NAMES: &'static str = "shared, core3 or two";

    pub fn parse(s: &str) -> Option<SoundCores> {
        match s {
            "shared" => Some(SoundCores::Shared),
            "core3" => Some(SoundCores::Core3),
            "two" => Some(SoundCores::Two),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SoundCores::Shared => "shared",
            SoundCores::Core3 => "core3",
            SoundCores::Two => "two",
        }
    }

    /// Prefs' value for it.
    pub fn row(self) -> &'static str {
        match self {
            SoundCores::Shared => "Shared",
            SoundCores::Core3 => "Core 3",
            SoundCores::Two => "Cores 3 and 2",
        }
    }

    fn label(self) -> &'static str {
        match self {
            SoundCores::Shared => "shared: the audio thread runs where the kernel puts it",
            SoundCores::Core3 => "core3: the audio thread alone on core 3",
            SoundCores::Two => "two: the audio thread on core 3, lanes on core 2",
        }
    }
}

/// The cores asked for (tests leave them out, so nothing is pinned).
#[derive(Clone, Copy, Debug)]
struct Pins {
    audio: Option<usize>,
    lane: Option<usize>,
    fifo: Option<i32>,
}

const MPC: Pins = Pins {
    audio: Some(AUDIO_CORE),
    lane: Some(LANE_CORE),
    fifo: Some(LANE_FIFO),
};

#[cfg(test)]
const NONE: Pins = Pins {
    audio: None,
    lane: None,
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
}

impl Counts {
    pub fn read(player: &Player, mixer: &Mixer) -> Counts {
        let (c, m) = (player.counters(), mixer.stats());
        Counts {
            voice_parts: c.lane_parts,
            voice_late: c.lane_late,
            mix_stages: m.lane_posts,
            mix_late: m.lane_late,
        }
    }

    fn late(&self) -> u64 {
        self.voice_late + self.mix_late
    }
}

/// What the Dev page reads: the set-up rows (once, before the sound
/// starts) and the counts (every 250 ms, with the engines' numbers). No
/// lock: the counts are atomics, and the rows are set once.
#[derive(Debug, Default)]
pub struct CoreShared {
    setup: OnceLock<(SoundCores, Vec<Row>)>,
    counts: [AtomicU64; 4],
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
    /// Puts the calling thread (the audio thread) on core 3 and splits the
    /// sampler's voices onto a lane on core 2, as `mode` asks. Call it on
    /// the audio thread before the sound starts: it allocates and (`two`)
    /// starts the sampler's lane thread. The mixer's lane follows with
    /// [`Cores::start_mixer`] once the mixer is made.
    pub fn start(mode: SoundCores, player: &mut Player) -> Cores {
        Cores::start_on(mode, player, MPC)
    }

    /// [`Cores::start`] with nothing pinned and no priority asked (the
    /// tests: the lanes run, the test threads stay where they are).
    #[cfg(test)]
    pub fn start_unpinned(mode: SoundCores, player: &mut Player) -> Cores {
        Cores::start_on(mode, player, NONE)
    }

    fn start_on(mode: SoundCores, player: &mut Player, pins: Pins) -> Cores {
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
        if mode == SoundCores::Two {
            let setup = player.split(c.lane("mpfree-lane-smp"));
            let (text, tone) = match &setup {
                Some(s) => lane_text(s, "half the drum voices"),
                None => ("not split (fewer than 2 voices)".to_string(), Tone::Bad),
            };
            c.lines.push(format!("sound cores: sampler lane {text}"));
            c.rows.push(("Drum voices lane".to_string(), text, tone));
        }
        c
    }

    /// `two`: runs about half the pad channels' and tracks' inserts on a
    /// lane on core 2. Call it right after the mixer is made, before the
    /// loop: it allocates and starts the mixer's lane thread.
    pub fn start_mixer(&mut self, mixer: &mut Mixer) {
        if self.mode != SoundCores::Two {
            return;
        }
        let (text, tone) = match mixer.set_lane(Some(self.lane("mpfree-lane-mix")), MIX_WAIT) {
            Some(s) => lane_text(&s, "about half the inserts"),
            None => ("not started".to_string(), Tone::Bad),
        };
        self.lines.push(format!("sound cores: mixer lane {text}"));
        self.rows
            .push(("Mixer inserts lane".to_string(), text, tone));
    }

    fn lane(&self, name: &str) -> LaneConfig {
        LaneConfig {
            name: name.to_string(),
            core: self.pins.lane,
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
        let values = [c.voice_parts, c.voice_late, c.mix_stages, c.mix_late];
        for (slot, v) in shared.counts.iter().zip(values) {
            slot.store(v, Ordering::Relaxed);
        }
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
        let (dv, dm) = (
            c.voice_late - self.noted.voice_late,
            c.mix_late - self.noted.mix_late,
        );
        self.noted = c;
        self.noted_at = Some(now);
        Some(format!(
            "sound cores: the second core was late (a fallback): {dv} part{} with half the drum voices \
             unheard and {dm} mixer stage{} with those strips silent; {} and {} since the start",
            if dv == 1 { "" } else { "s" },
            if dm == 1 { "" } else { "s" },
            c.voice_late,
            c.mix_late
        ))
    }

    /// For the 30 s status line and the end of the session.
    pub fn status(&self, c: Counts) -> String {
        match self.mode {
            SoundCores::Two => format!(
                "sound cores two: drum voices {} parts on core 2, {} late; mixer {} stages on core 2, {} late",
                c.voice_parts, c.voice_late, c.mix_stages, c.mix_late
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

/// The Dev page's rows: how the sound's cores started, and (`two`) the
/// lanes' parts and late ones. Empty until the audio loop has set up.
pub fn rows(shared: &CoreShared) -> Vec<Row> {
    let Some((mode, setup)) = shared.setup.get() else {
        return Vec::new();
    };
    let mut rows = setup.clone();
    if *mode == SoundCores::Two {
        let c = shared.counts();
        let tone = |late: u64| if late == 0 { Tone::Good } else { Tone::Bad };
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
    let values = [41_210, 0, 20_640, 3];
    for (slot, v) in shared.counts.iter().zip(values) {
        slot.store(v, Ordering::Relaxed);
    }
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

    #[test]
    fn the_names_parse_and_shared_is_the_default() {
        for m in [SoundCores::Shared, SoundCores::Core3, SoundCores::Two] {
            assert_eq!(SoundCores::parse(m.name()), Some(m));
        }
        assert_eq!(SoundCores::parse("3"), None);
        assert_eq!(SoundCores::default(), SoundCores::Shared);
    }

    #[test]
    fn shared_changes_nothing() {
        let mut p = player();
        let mut mix = crate::mixing::MixHost::new(44_100, -6.0, -6.0);
        let mut c = Cores::start_on(SoundCores::Shared, &mut p, NONE);
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
        let mut c = Cores::start_on(SoundCores::Two, &mut p, NONE);
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
        let n = Counts::read(&p, &mix.mixer);
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
        let mut c = Cores::start_on(SoundCores::Core3, &mut p, NONE);
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
            line.contains("1 part with half the drum voices unheard and 0 mixer stages"),
            "{line}"
        );
        n.mix_late = 2;
        assert_eq!(c.late_note(n, t + Duration::from_millis(500)), None);
        let line = c.late_note(n, t + Duration::from_millis(1000)).unwrap();
        assert!(line.contains("0 parts with half"), "{line}");
        assert!(line.contains("2 mixer stages with those strips silent; 1 and 2 since the start"));
        assert_eq!(
            c.late_note(n, t + Duration::from_secs(5)),
            None,
            "no new ones"
        );
        // Late ones show red on the Dev page.
        let shared = CoreShared::default();
        let two = Cores::start_on(SoundCores::Two, &mut p, NONE);
        two.show(&shared);
        two.publish(&shared, n);
        let r = rows(&shared);
        assert_eq!(r.last().unwrap().1, "40 stages · 2 late");
        assert_eq!(r.last().unwrap().2, Tone::Bad);
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
