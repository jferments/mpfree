//! Auto-sampling: play an instrument note by note, record every note, and
//! build a keygroup instrument from the takes.
//!
//! - [`Plan`]: the notes, velocity layers and round robins to record, how
//!   long each note is held and may ring, and what to do with the takes,
//!   with the time and memory it will take ([`Plan::estimate`]).
//! - [`Session`]: runs a plan against any instrument a block at a time. It
//!   says which notes to play in the next block ([`Session::cues`]) and
//!   takes the audio that came back for it ([`Session::feed`]), so the same
//!   session drives one of MPFree's own synths rendered offline and, later,
//!   external gear over MIDI out recorded from the line input. It opens
//!   with a level and timing test, ends each note when it has gone silent,
//!   and can be cancelled, keeping the notes already done.
//! - [`build`]: trims each take 2 ms before its own attack (so MIDI jitter
//!   never shifts a note) and where it has gone silent, looks for a loop in
//!   the held part, normalizes the instrument as a whole (so the layers
//!   keep their dynamics) and makes one keygroup pad per note, each
//!   reaching halfway to its neighbours.
//! - [`Instrument::put`]: lands it on the kit as one undo step, with the
//!   pads playing notes.
//!
//! Nothing here runs on the audio thread. Compared with the MPC's Auto
//! Sampler, Elektron's Tonverk and Logic's Auto Sampler in the sampler's
//! plan (capture, resample anything and auto-sample gear).

use crate::model::{
    Fade, FadeCurve, Keys, Layer, LayerSelect, Loop, LoopMode, ModelError, Pad, PlayMode, Project,
    Region, Source, KEYGROUP_POLYPHONY, MAX_LAYERS,
};
use mpfree_analysis::note_name;

/// Velocity layers a plan may have (the MPC's limit).
pub const MAX_VELOCITY_LAYERS: usize = 8;
/// Round robins a plan may have.
pub const MAX_ROUND_ROBINS: u8 = 4;
/// The most a session's takes may hold in memory, as recorded.
pub const MAX_SESSION_BYTES: u64 = 256 * 1024 * 1024;
/// Each take starts this long before its attack.
pub const PRE_ROLL_MS: f32 = 2.0;
/// The fade at the end of each take, where it has gone silent.
pub const END_FADE_MS: f32 = 10.0;
/// A note has ended when it has been under the silence level this long.
pub const QUIET_MS: f32 = 100.0;
/// A take's attack is where it first reaches this far below its own peak.
pub const ONSET_DB: f32 = -40.0;
/// The level the instrument's loudest take is normalized to: what MPFree's
/// instruments aim for with one full-velocity note (see `mpfree-node`).
pub const NORMALIZE_TO_DB: f32 = -12.0;
/// The level test heard nothing if its peak was under this.
pub const NO_SOUND_DB: f32 = -80.0;
/// A loop is only kept when its two ends match at least this well (0 to 1).
pub const LOOP_MATCH: f32 = 0.9;
/// The level at which a recording through the line input has clipped.
const CLIP_LEVEL: f32 = 0.999;

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn frames(seconds: f32, rate: u32) -> u64 {
    (f64::from(seconds) * f64::from(rate)).round() as u64
}

/// How a plan's velocity layers spread from soft to hard
/// ([`Plan::spread_velocities`]), as in Logic's Auto Sampler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VelocityCurve {
    /// Evenly.
    Linear,
    /// More layers among the soft hits.
    Exp,
    /// More layers among the hard hits.
    Log,
}

/// What to record and what to make of it. Times are in seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// The instrument's name; its pads and sources are named after it.
    pub name: String,
    /// The lowest note sampled (MIDI, 60 = C4).
    pub lo: u8,
    /// No note above this is sampled.
    pub hi: u8,
    /// Semitones from one sampled note to the next, from `lo` up (the
    /// MPC's Note Stride, Logic's Sample Every).
    pub every: u8,
    /// One velocity per layer, ascending. Each layer is played at its
    /// velocity and answers hits from just above the layer below's velocity
    /// up to its own (the top layer up to 127).
    pub velocities: Vec<u8>,
    /// Takes of each note and velocity, played in turn by the pad.
    pub round_robins: u8,
    /// How long each note is held.
    pub hold: f32,
    /// The longest each note may ring after it is let go; it ends sooner
    /// when it has been silent for [`QUIET_MS`].
    pub tail: f32,
    /// Silence between notes.
    pub gap: f32,
    /// Silent means this far below the loudest note, in dB.
    pub silence_db: f32,
    /// Look for a loop in the held part of each note.
    pub loops: Option<LoopMode>,
    /// 1 (mono) or 2 (stereo).
    pub channels: u8,
    /// Normalize the instrument as a whole to [`NORMALIZE_TO_DB`].
    pub normalize: bool,
    pub rate: u32,
}

impl Default for Plan {
    /// C2 (36) to C6 (84) every 3 semitones, three velocity layers, held
    /// 2 s with up to 4 s of tail, no loops, stereo, normalized.
    fn default() -> Self {
        Plan {
            name: "Auto 1".to_string(),
            lo: 36,
            hi: 84,
            every: 3,
            velocities: vec![40, 90, 127],
            round_robins: 1,
            hold: 2.0,
            tail: 4.0,
            gap: 0.2,
            silence_db: -60.0,
            loops: None,
            channels: 2,
            normalize: true,
            rate: 44_100,
        }
    }
}

/// Why a plan can't run, in words for the screen.
#[derive(Clone, Debug, PartialEq)]
pub enum PlanError {
    Notes,
    Velocities,
    RoundRobins,
    /// Velocity layers × round robins is more than a pad's layers.
    TooManyLayers,
    Times,
    Channels,
    Rate,
    /// The takes would need `bytes` of memory, more than `limit`.
    TooBig {
        bytes: u64,
        limit: u64,
    },
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::Notes => write!(f, "the lowest note must be at or below the highest, every 1 to 12 semitones"),
            PlanError::Velocities => write!(f, "1 to 8 velocity layers, each softer than the next"),
            PlanError::RoundRobins => write!(f, "1 to 4 round robins"),
            PlanError::TooManyLayers => write!(f, "velocity layers × round robins can be at most 16"),
            PlanError::Times => write!(f, "hold 0.1 to 20 s, tail 0 to 20 s, gap 0 to 5 s, silence -90 to -30 dB"),
            PlanError::Channels => write!(f, "mono or stereo"),
            PlanError::Rate => write!(f, "the sample rate must be 8 to 192 kHz"),
            PlanError::TooBig { bytes, limit } => write!(
                f,
                "this needs up to {} MB, more than the {} MB a session may use: fewer notes, layers or a shorter tail",
                bytes.div_ceil(1 << 20),
                limit / (1 << 20)
            ),
        }
    }
}
impl std::error::Error for PlanError {}

/// One note to record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Take {
    pub note: u8,
    pub velocity: u8,
    /// Index into [`Plan::velocities`].
    pub layer: u8,
    pub round_robin: u8,
}

/// What a plan costs before it starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    pub takes: usize,
    /// In real time, when each tail rings for half of [`Plan::tail`].
    pub seconds: f32,
    /// In real time, when every tail runs its full length.
    pub seconds_most: f32,
    /// The takes in memory as recorded (32-bit float), half tails.
    pub bytes: u64,
    /// The same with full tails: what [`Plan::check`] holds to
    /// [`MAX_SESSION_BYTES`].
    pub bytes_most: u64,
}

impl Plan {
    /// The notes sampled, low to high.
    pub fn notes(&self) -> Vec<u8> {
        if self.every == 0 || self.lo > self.hi {
            return Vec::new();
        }
        (self.lo..=self.hi.min(127))
            .step_by(usize::from(self.every))
            .collect()
    }

    /// Every take in the order they are played: low to high, each layer
    /// soft to hard, each round robin.
    pub fn takes(&self) -> Vec<Take> {
        let mut out = Vec::new();
        for note in self.notes() {
            for (layer, &velocity) in self.velocities.iter().enumerate() {
                for round_robin in 0..self.round_robins {
                    out.push(Take {
                        note,
                        velocity,
                        layer: layer as u8,
                        round_robin,
                    });
                }
            }
        }
        out
    }

    pub fn estimate(&self) -> Estimate {
        let takes = self.notes().len() * self.velocities.len() * usize::from(self.round_robins);
        let n = takes as f32;
        let per_second = u64::from(self.rate) * u64::from(self.channels) * 4;
        let bytes = |tail: f32| (n * (self.hold + tail)) as u64 * per_second;
        Estimate {
            takes,
            seconds: n * (self.hold + self.tail / 2.0 + self.gap),
            seconds_most: n * (self.hold + self.tail + self.gap),
            bytes: bytes(self.tail / 2.0),
            bytes_most: bytes(self.tail),
        }
    }

    /// Spreads `layers` velocities from soft to hard along `curve`, the
    /// hardest always 127.
    pub fn spread_velocities(layers: usize, curve: VelocityCurve) -> Vec<u8> {
        let n = layers.clamp(1, MAX_VELOCITY_LAYERS);
        let mut out: Vec<u8> = Vec::with_capacity(n);
        for k in 0..n {
            let x = (k + 1) as f32 / n as f32;
            let y = match curve {
                VelocityCurve::Linear => x,
                VelocityCurve::Exp => x * x,
                VelocityCurve::Log => x.sqrt(),
            };
            let floor = out.last().map_or(1, |v| v + 1);
            out.push(((y * 127.0).round() as u8).max(floor));
        }
        out
    }

    /// The velocities each layer answers, soft to hard.
    pub fn velocity_ranges(&self) -> Vec<(u8, u8)> {
        let n = self.velocities.len();
        (0..n)
            .map(|i| {
                let lo = if i == 0 {
                    1
                } else {
                    self.velocities[i - 1] + 1
                };
                let hi = if i + 1 == n { 127 } else { self.velocities[i] };
                (lo, hi)
            })
            .collect()
    }

    /// Each sampled note's keys: halfway to its neighbours (the upper
    /// note takes the middle key when they are an even number apart), the
    /// lowest down to 0 and the highest up to 127, as the MPC's Extend.
    pub fn zones(&self) -> Vec<Keys> {
        zones(&self.notes())
    }

    pub fn check(&self) -> Result<(), PlanError> {
        if self.lo > self.hi || self.hi > 127 || !(1..=12).contains(&self.every) {
            return Err(PlanError::Notes);
        }
        let v = &self.velocities;
        if v.is_empty()
            || v.len() > MAX_VELOCITY_LAYERS
            || v[0] == 0
            || v.iter().any(|&x| x > 127)
            || v.windows(2).any(|w| w[0] >= w[1])
        {
            return Err(PlanError::Velocities);
        }
        if !(1..=MAX_ROUND_ROBINS).contains(&self.round_robins) {
            return Err(PlanError::RoundRobins);
        }
        if v.len() * usize::from(self.round_robins) > MAX_LAYERS {
            return Err(PlanError::TooManyLayers);
        }
        let ok = |x: f32, lo: f32, hi: f32| x.is_finite() && (lo..=hi).contains(&x);
        if !ok(self.hold, 0.1, 20.0)
            || !ok(self.tail, 0.0, 20.0)
            || !ok(self.gap, 0.0, 5.0)
            || !ok(self.silence_db, -90.0, -30.0)
        {
            return Err(PlanError::Times);
        }
        if self.channels != 1 && self.channels != 2 {
            return Err(PlanError::Channels);
        }
        if !(8000..=192_000).contains(&self.rate) {
            return Err(PlanError::Rate);
        }
        let bytes = self.estimate().bytes_most;
        if bytes > MAX_SESSION_BYTES {
            return Err(PlanError::TooBig {
                bytes,
                limit: MAX_SESSION_BYTES,
            });
        }
        Ok(())
    }
}

/// Keys for sampled `notes` (ascending), as [`Plan::zones`] makes them.
pub fn zones(notes: &[u8]) -> Vec<Keys> {
    let n = notes.len();
    (0..n)
        .map(|i| {
            let root = notes[i];
            let lo = if i == 0 {
                0
            } else {
                let below = notes[i - 1];
                below + (root - below - 1) / 2 + 1
            };
            let hi = if i + 1 == n {
                127
            } else {
                root + (notes[i + 1] - root - 1) / 2
            };
            Keys { lo, hi, root }
        })
        .collect()
}

/// Something to send the instrument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    NoteOn {
        note: u8,
        velocity: u8,
    },
    NoteOff {
        note: u8,
    },
    /// Every note off (a cancelled session).
    AllOff,
}

/// A [`Cue`] at a frame offset inside the block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timed {
    pub at: usize,
    pub cue: Cue,
}

/// Where a session is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Playing the level and timing test.
    Testing,
    Recording,
    Done,
    Cancelled,
    /// The level test heard nothing back: check the instrument's MIDI
    /// channel, its cables and its volume.
    NoSound,
}

/// How far a session has got, for the progress bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    /// The note playing now.
    pub note: Option<u8>,
}

/// One recorded take, from the frame its note-on was due (so it starts
/// with the instrument's latency) to where it went silent or its tail ran
/// out. Interleaved, the plan's channels.
#[derive(Clone, Debug, PartialEq)]
pub struct Recorded {
    pub take: Take,
    pub audio: Vec<f32>,
}

/// What a session recorded.
#[derive(Clone, Debug)]
pub struct Recording {
    pub plan: Plan,
    pub takes: Vec<Recorded>,
    /// Frames from note-on to sound in the level test (the longest of the
    /// tests), when there was one.
    pub latency: Option<u64>,
    /// The level test's peak, relative to full scale.
    pub test_peak: Option<f32>,
    /// Something reached full scale: through the line input that is
    /// clipping, so the instrument should be turned down. Nothing stops for
    /// it (deliberate distortion stays possible).
    pub clipped: bool,
    /// Every take was recorded (not cancelled).
    pub complete: bool,
}

/// One note played and recorded: a level test or a take.
#[derive(Clone, Copy, Debug)]
struct Shot {
    note: u8,
    velocity: u8,
    take: Option<Take>,
}

/// Runs a [`Plan`] against an instrument, a block at a time: ask for the
/// block's [`Session::cues`], send them at their offsets, then
/// [`Session::feed`] the audio that came back for the same frames.
pub struct Session {
    plan: Plan,
    shots: Vec<Shot>,
    tests: usize,
    i: usize,
    /// Frames fed so far.
    now: u64,
    /// Frames the last [`Session::cues`] covered.
    block: u64,
    on_at: u64,
    on_sent: bool,
    off_sent: bool,
    hold: u64,
    tail: u64,
    gap: u64,
    quiet_need: u64,
    threshold: f32,
    rec: Vec<f32>,
    heard: bool,
    quiet: u64,
    peak: f32,
    test_peak: f32,
    latency: u64,
    clipped: bool,
    takes: Vec<Recorded>,
    cues: Vec<Timed>,
    state: State,
}

impl Session {
    /// A session for `plan`, opening with `tests` (0 to 3) level and timing
    /// tests: the middle note at the top velocity. With none, silence is
    /// measured from full scale.
    pub fn new(plan: Plan, tests: u8) -> Result<Session, PlanError> {
        plan.check()?;
        let notes = plan.notes();
        let middle = notes[notes.len() / 2];
        let top = *plan.velocities.last().expect("checked");
        let tests = usize::from(tests.min(3));
        let mut shots: Vec<Shot> = (0..tests)
            .map(|_| Shot {
                note: middle,
                velocity: top,
                take: None,
            })
            .collect();
        shots.extend(plan.takes().into_iter().map(|t| Shot {
            note: t.note,
            velocity: t.velocity,
            take: Some(t),
        }));
        let rate = plan.rate;
        let hold = frames(plan.hold, rate).max(1);
        let tail = frames(plan.tail, rate);
        let ch = usize::from(plan.channels);
        Ok(Session {
            shots,
            tests,
            i: 0,
            now: 0,
            block: 0,
            on_at: 0,
            on_sent: false,
            off_sent: false,
            hold,
            tail,
            gap: frames(plan.gap, rate),
            quiet_need: frames(QUIET_MS / 1000.0, rate).max(1),
            threshold: db_to_gain(plan.silence_db),
            rec: Vec::with_capacity((hold + tail) as usize * ch),
            heard: false,
            quiet: 0,
            peak: 0.0,
            test_peak: 0.0,
            latency: 0,
            clipped: false,
            takes: Vec::new(),
            cues: Vec::with_capacity(4),
            state: if tests > 0 {
                State::Testing
            } else {
                State::Recording
            },
            plan,
        })
    }

    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    pub fn state(&self) -> State {
        self.state
    }

    fn running(&self) -> bool {
        matches!(self.state, State::Testing | State::Recording)
    }

    pub fn progress(&self) -> Progress {
        Progress {
            done: self.takes.len(),
            total: self.shots.len() - self.tests,
            note: self
                .shots
                .get(self.i)
                .filter(|_| self.running())
                .map(|s| s.note),
        }
    }

    /// What to send the instrument during the next `frames` frames. Feed
    /// exactly those frames next.
    pub fn cues(&mut self, frames: usize) -> &[Timed] {
        self.cues.clear();
        self.block = frames as u64;
        if !self.running() {
            return &self.cues;
        }
        let shot = self.shots[self.i];
        let end = self.now + frames as u64;
        if !self.on_sent && self.on_at < end {
            self.on_at = self.on_at.max(self.now);
            self.cues.push(Timed {
                at: (self.on_at - self.now) as usize,
                cue: Cue::NoteOn {
                    note: shot.note,
                    velocity: shot.velocity,
                },
            });
            self.on_sent = true;
        }
        let off_at = self.on_at + self.hold;
        if self.on_sent && !self.off_sent && off_at < end {
            self.cues.push(Timed {
                at: off_at.saturating_sub(self.now) as usize,
                cue: Cue::NoteOff { note: shot.note },
            });
            self.off_sent = true;
        }
        &self.cues
    }

    /// The audio that came back for the frames the last [`Session::cues`]
    /// covered: interleaved, the plan's channels.
    pub fn feed(&mut self, audio: &[f32]) {
        let ch = usize::from(self.plan.channels);
        let n = (audio.len() / ch) as u64;
        let block_end = self.now + n.max(self.block);
        for j in 0..n {
            if !self.running() {
                break;
            }
            let f = self.now + j;
            if !self.on_sent || f < self.on_at {
                continue;
            }
            let frame = &audio[j as usize * ch..(j as usize + 1) * ch];
            self.rec.extend_from_slice(frame);
            let level = frame.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            if level >= CLIP_LEVEL {
                self.clipped = true;
            }
            self.peak = self.peak.max(level);
            let t = f - self.on_at;
            if level >= self.threshold {
                self.heard = true;
                self.quiet = 0;
            } else if t >= self.hold {
                self.quiet += 1;
            }
            let ended = t >= self.hold
                && ((self.heard && self.quiet >= self.quiet_need)
                    || t + 1 >= self.hold + self.tail);
            if ended {
                self.end_shot(f + 1, block_end);
            }
        }
        self.now += n;
        self.block = 0;
    }

    fn end_shot(&mut self, at: u64, block_end: u64) {
        let shot = self.shots[self.i];
        match shot.take {
            None => {
                self.test_peak = self.test_peak.max(self.peak);
                let floor = self.peak * db_to_gain(ONSET_DB);
                let ch = usize::from(self.plan.channels);
                if let Some(k) = self
                    .rec
                    .chunks(ch)
                    .position(|fr| fr.iter().any(|x| x.abs() >= floor))
                {
                    self.latency = self.latency.max(k as u64);
                }
            }
            // Copied out at its length; the recording buffer is kept for
            // the next note.
            Some(take) => self.takes.push(Recorded {
                take,
                audio: self.rec.to_vec(),
            }),
        }
        self.rec.clear();
        self.i += 1;
        if self.i == self.tests && self.tests > 0 {
            if self.test_peak < db_to_gain(NO_SOUND_DB) {
                self.state = State::NoSound;
            } else {
                // Silence is measured from the test's peak from now on.
                self.threshold = self.test_peak * db_to_gain(self.plan.silence_db);
                self.state = State::Recording;
            }
        }
        if self.i == self.shots.len() && self.running() {
            self.state = State::Done;
        }
        self.on_at = (at + self.gap).max(block_end);
        self.on_sent = false;
        self.off_sent = false;
        self.heard = false;
        self.quiet = 0;
        self.peak = 0.0;
    }

    /// Stops the session: send the returned cue at once. The notes already
    /// done are kept ([`Session::finish`]).
    pub fn cancel(&mut self) -> Cue {
        if self.running() {
            self.state = State::Cancelled;
        }
        self.rec.clear();
        Cue::AllOff
    }

    /// What was recorded: every take when the session is done, the takes
    /// finished before a cancel otherwise.
    pub fn finish(self) -> Recording {
        Recording {
            complete: self.state == State::Done,
            latency: (self.tests > 0 && self.i >= self.tests).then_some(self.latency),
            test_peak: (self.tests > 0 && self.i >= self.tests).then_some(self.test_peak),
            clipped: self.clipped,
            takes: self.takes,
            plan: self.plan,
        }
    }
}

/// Why a recording didn't make an instrument.
#[derive(Clone, Debug, PartialEq)]
pub enum BuildError {
    /// No note has all of its takes.
    Empty,
    /// This take is silent.
    Silent {
        note: u8,
        velocity: u8,
    },
    Model(ModelError),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::Empty => write!(f, "no note was recorded all the way through"),
            BuildError::Silent { note, velocity } => write!(
                f,
                "{} at velocity {velocity} came back silent",
                note_name(*note)
            ),
            BuildError::Model(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for BuildError {}

impl From<ModelError> for BuildError {
    fn from(e: ModelError) -> Self {
        BuildError::Model(e)
    }
}

/// An auto-sampled instrument, ready to land on a kit.
#[derive(Clone, Debug)]
pub struct Instrument {
    pub name: String,
    /// One trimmed source per take.
    pub sources: Vec<Source>,
    /// One keygroup per sampled note, low to high.
    pub pads: Vec<Pad>,
    /// The pads play notes from this one (the C at or below the lowest).
    pub from: u8,
    /// Takes that found a loop.
    pub looped: usize,
    /// The gain normalizing added to every take.
    pub gain_db: f32,
}

impl Instrument {
    /// Puts the instrument on `project` from pad `to_pad`, one undo step.
    pub fn put(self, project: &mut Project, to_pad: usize) -> Result<(), ModelError> {
        project.put_instrument(to_pad, self.sources, self.pads, self.from)
    }

    /// Bytes its audio holds.
    pub fn bytes(&self) -> u64 {
        self.sources.iter().map(|s| s.data().len() as u64 * 4).sum()
    }
}

/// A take trimmed to its sound, before it becomes a source.
struct Trimmed {
    take: Take,
    audio: Vec<f32>,
    /// Where the attack is, in the trimmed audio.
    onset: u64,
    fade_out: u32,
}

/// Makes a keygroup instrument from a recording. Notes missing any of
/// their takes (a cancelled session) are left out.
pub fn build(rec: &Recording) -> Result<Instrument, BuildError> {
    let plan = &rec.plan;
    let ch = usize::from(plan.channels);
    let per_note = plan.velocities.len() * usize::from(plan.round_robins);
    let notes: Vec<u8> = plan
        .notes()
        .into_iter()
        .filter(|&n| rec.takes.iter().filter(|r| r.take.note == n).count() == per_note)
        .collect();
    if notes.is_empty() {
        return Err(BuildError::Empty);
    }
    // A cancelled session's notes reach over the ones it didn't get to.
    let zones = zones(&notes);

    let peak_of = |x: &[f32]| x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let mut peak = 0.0f32;
    for r in &rec.takes {
        let p = peak_of(&r.audio);
        if p < db_to_gain(NO_SOUND_DB) && notes.contains(&r.take.note) {
            return Err(BuildError::Silent {
                note: r.take.note,
                velocity: r.take.velocity,
            });
        }
        peak = peak.max(p);
    }
    let silence = peak * db_to_gain(plan.silence_db);
    let gain_db = if plan.normalize {
        NORMALIZE_TO_DB - 20.0 * peak.log10()
    } else {
        0.0
    };

    // Trim every take: 2 ms before its own attack, to where it went silent.
    let rate = plan.rate;
    let pre = frames(PRE_ROLL_MS / 1000.0, rate);
    let fade = frames(END_FADE_MS / 1000.0, rate);
    let mut trimmed: Vec<Trimmed> = Vec::new();
    for r in &rec.takes {
        if !notes.contains(&r.take.note) {
            continue;
        }
        let len = (r.audio.len() / ch) as u64;
        let level = |k: u64| {
            let k = k as usize * ch;
            peak_of(&r.audio[k..k + ch])
        };
        let floor = peak_of(&r.audio) * db_to_gain(ONSET_DB);
        let onset = (0..len).find(|&k| level(k) >= floor).unwrap_or(0);
        let start = onset.saturating_sub(pre);
        let last = (onset..len)
            .rev()
            .find(|&k| level(k) >= silence)
            .unwrap_or(onset);
        let end = (last + 1 + fade).min(len).max(start + 2);
        let frames_kept = end - start;
        trimmed.push(Trimmed {
            take: r.take,
            audio: r.audio[start as usize * ch..end as usize * ch].to_vec(),
            onset: onset - start,
            fade_out: fade.min(frames_kept / 2) as u32,
        });
    }

    // Without a loop, a pad lets go like the instrument did: its release
    // is the takes' typical ring after the note-off.
    let hold = frames(plan.hold, rate);
    let mut rings: Vec<u64> = trimmed
        .iter()
        .map(|t| ((t.audio.len() / ch) as u64).saturating_sub(t.onset + hold))
        .collect();
    rings.sort_unstable();
    let ring_ms = (rings[rings.len() / 2] as f32 * 1000.0 / rate as f32).clamp(5.0, 10_000.0);

    let ranges = plan.velocity_ranges();
    let mut sources = Vec::with_capacity(trimmed.len());
    let mut pads = Vec::with_capacity(notes.len());
    let mut looped = 0;
    for (i, &note) in notes.iter().enumerate() {
        let mut layers = Vec::with_capacity(per_note);
        for t in trimmed.iter().filter(|t| t.take.note == note) {
            let mut name = format!("{} {} v{}", plan.name, note_name(note), t.take.velocity);
            if plan.round_robins > 1 {
                name.push_str(&format!(" rr{}", t.take.round_robin + 1));
            }
            let found = plan
                .loops
                .and_then(|mode| find_loop(&t.audio, ch, rate, t.onset, hold, mode));
            let s = Source::new(&name, rate, plan.channels, t.audio.clone())?;
            let mut region = Region::whole(&s);
            region.gain_db = gain_db;
            region.fade_out = Fade {
                frames: t.fade_out,
                curve: FadeCurve::Linear,
            };
            region.mode = PlayMode::Gate;
            if let Some(l) = found {
                // The fade only covers the tail after the loop.
                region.fade_out.frames = region
                    .fade_out
                    .frames
                    .min((region.end - l.end).min(u64::from(u32::MAX)) as u32);
                region.sustain_loop = Some(l);
                region.mode = PlayMode::Loop;
                looped += 1;
            }
            let (vel_lo, vel_hi) = ranges[usize::from(t.take.layer)];
            let mut layer = Layer::new(region);
            layer.vel_lo = vel_lo;
            layer.vel_hi = vel_hi;
            // A hit at the velocity a layer was played at sounds as
            // recorded (the pad's level follows velocity squared).
            let v = f32::from(t.take.velocity) / 127.0;
            layer.level_db = -40.0 * v.log10();
            layers.push(layer);
            sources.push(s);
        }
        let mut pad = Pad::new(
            &format!("{} {}", plan.name, note_name(note)),
            layers[0].clone(),
        );
        pad.layers = layers;
        pad.select = if plan.round_robins > 1 {
            LayerSelect::Cycle
        } else {
            LayerSelect::All
        };
        pad.polyphony = KEYGROUP_POLYPHONY;
        pad.keys = Some(zones[i]);
        pad.amp.release_ms = ring_ms;
        pads.push(pad);
    }
    let lowest = notes[0];
    Ok(Instrument {
        name: plan.name.clone(),
        sources,
        pads,
        from: (lowest - lowest % 12).min(127 - 15),
        looped,
        gain_db,
    })
}

/// Frames where the signal (all channels summed) crosses zero going up.
fn rising_zeros(mono: &[f32], from: usize, to: usize) -> Vec<usize> {
    let to = to.min(mono.len());
    (from.max(1)..to)
        .filter(|&k| mono[k - 1] < 0.0 && mono[k] >= 0.0)
        .collect()
}

/// How alike two stretches are, -1 to 1: 1 only when they are the same
/// shape and level.
fn likeness(a: &[f32], b: &[f32]) -> f32 {
    let (mut ab, mut aa, mut bb) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b) {
        ab += f64::from(*x) * f64::from(*y);
        aa += f64::from(*x) * f64::from(*x);
        bb += f64::from(*y) * f64::from(*y);
    }
    if aa + bb <= 0.0 {
        return 0.0;
    }
    (2.0 * ab / (aa + bb)) as f32
}

/// Looks for a loop in the held part of a take (after the attack, before
/// the note-off), from one upward zero crossing to another whose
/// surroundings match. Prefers the longest of the best matches. None when
/// nothing matches [`LOOP_MATCH`].
fn find_loop(
    audio: &[f32],
    ch: usize,
    rate: u32,
    onset: u64,
    hold: u64,
    mode: LoopMode,
) -> Option<Loop> {
    let len = audio.len() / ch;
    let mono: Vec<f32> = audio.chunks(ch).map(|f| f.iter().sum()).collect();
    let skip = (frames(0.1, rate)).max(hold / 4);
    let a = (onset + skip) as usize;
    let b = ((onset + hold) as usize)
        .saturating_sub(frames(END_FADE_MS / 1000.0, rate) as usize)
        .min(len);
    let w = (rate / 50) as usize; // 20 ms each side
    let half = w / 2;
    if b <= a + 2 * w {
        return None;
    }
    let min_len = (frames(0.1, rate) as usize).max((b - a) / 4);
    if b - a < min_len + w {
        return None;
    }
    let mut ends = rising_zeros(
        &mono,
        (b - half).saturating_sub(rate as usize / 20),
        b - half,
    );
    ends.reverse();
    ends.truncate(8);
    let mut best: Option<(f32, usize, usize)> = None;
    let window = |k: usize| &audio[(k - half) * ch..(k + half) * ch];
    for &e in &ends {
        if e < a + half + min_len {
            continue;
        }
        let starts = rising_zeros(&mono, a + half, e - min_len);
        let step = starts.len().div_ceil(192).max(1);
        for &s in starts.iter().step_by(step) {
            let sim = likeness(window(s), window(e));
            let score = sim + 0.02 * (e - s) as f32 / (b - a) as f32;
            if best.is_none_or(|(sc, ..)| score > sc) && sim >= LOOP_MATCH {
                best = Some((score, s, e));
            }
        }
    }
    let (_, s, e) = best?;
    let crossfade = match mode {
        LoopMode::Forward => ((e - s) / 4).min(frames(0.03, rate) as usize).min(s),
        LoopMode::PingPong => 0,
    };
    Some(Loop {
        start: s as u64,
        end: e as u64,
        crossfade: crossfade as u32,
        mode,
    })
}
