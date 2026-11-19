//! The shared voice core, part 1: the parts every engine's voice is made
//! of, each with its own tests, for engines to assemble
//! (`docs/dev/design/synth-architecture.md`, section 3). New engines are
//! built on it; an existing engine moves onto a part when that part fixes
//! something measured.
//!
//! - [`NoteStack`]: the keys held for a one-voice engine, and which of them
//!   sounds ([`Priority`]: last, lowest or highest).
//! - [`Voices`]: which voice a note gets, and which is stolen when they
//!   are all busy (released voices first, the quietest of them, then the
//!   oldest held).
//! - [`Glide`]: by time (every interval takes the same time) or by rate
//!   (semitones a second).
//! - [`Adsr`]: an envelope whose times mean what they say: an attack of
//!   100 ms reaches full level at 100 ms, a release of 1 s falls 60 dB in
//!   1 s.
//! - [`Lfo`]: free at a rate in Hz or locked to the song, with a delay
//!   and fade-in, restarting on each key or running on.
//! - [`velocity_gain`]: a square-law velocity curve over a range in dB
//!   (Dannenberg, ICMC 2006), set per preset.
//! - [`Route`] and [`Channel`]: the play routes, a source (velocity,
//!   pressure, per-note tone, the mod wheel, breath, the foot controller,
//!   the key, a random value per note) to a target with a depth.
//! - [`DcBlock`]: a gentle high-pass at an engine's output.
//!
//! The pedals are `mpfree_node::pedal::Pedals`, shared with the hosts.
//!
//! Plain `Copy` data, no allocation: everything here runs on the audio
//! thread.

use mpfree_node::Ctl;

use crate::dsp::{lfo_value, LfoShape, Rng};

// ------------------------------------------------------------- notes

/// Which held key a one-voice engine plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Priority {
    /// The key pressed last (and, when it is let go, the one before it).
    #[default]
    Last,
    /// The lowest key held.
    Low,
    /// The highest key held.
    High,
}

impl Priority {
    /// The choices' names, in order, for a parameter.
    pub const NAMES: &'static [&'static str] = &["Last", "Low", "High"];

    pub fn from_index(i: f32) -> Priority {
        match i as i32 {
            1 => Priority::Low,
            2 => Priority::High,
            _ => Priority::Last,
        }
    }
}

/// Most keys a [`NoteStack`] remembers; one more forgets the oldest.
pub const STACK: usize = 16;

/// The keys held for a one-voice engine, oldest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoteStack {
    keys: [u8; STACK],
    len: usize,
}

impl NoteStack {
    pub const fn new() -> NoteStack {
        NoteStack {
            keys: [0; STACK],
            len: 0,
        }
    }

    /// A key goes down: it becomes the newest (moved there if it was
    /// already held). With [`STACK`] keys held the oldest is forgotten.
    pub fn push(&mut self, note: u8) {
        self.remove(note);
        if self.len == STACK {
            self.keys.copy_within(1.., 0);
            self.len -= 1;
        }
        self.keys[self.len] = note;
        self.len += 1;
    }

    /// A key comes up. False if it wasn't held.
    pub fn remove(&mut self, note: u8) -> bool {
        let Some(i) = self.keys[..self.len].iter().position(|&k| k == note) else {
            return false;
        };
        self.keys.copy_within(i + 1..self.len, i);
        self.len -= 1;
        true
    }

    /// The key that sounds by `p`, or None with no key held.
    pub fn pick(&self, p: Priority) -> Option<u8> {
        let held = &self.keys[..self.len];
        match p {
            Priority::Last => held.last().copied(),
            Priority::Low => held.iter().min().copied(),
            Priority::High => held.iter().max().copied(),
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn contains(&self, note: u8) -> bool {
        self.keys[..self.len].contains(&note)
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }
}

// ------------------------------------------------------------- voices

/// Where a voice is in its life.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Free,
    /// Its key is down (or a pedal holds it).
    On,
    /// Its note has ended; its release is sounding.
    Release,
    /// Stolen: fading out over a few ms, then free. It doesn't count
    /// against the voice limit.
    Fade,
}

/// One voice's bookkeeping. The engine keeps `level` up to date (its
/// envelope times its gains, about 1 at full scale) so stealing can find
/// the quietest, and calls [`Voices::free`] when the voice is silent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Slot {
    pub phase: Phase,
    pub note: u8,
    /// When it started, in notes started: lower is older.
    pub age: u64,
    pub level: f32,
}

impl Slot {
    /// Counts against the voice limit: its note is on or releasing.
    pub fn sounding(&self) -> bool {
        matches!(self.phase, Phase::On | Phase::Release)
    }
}

/// Where [`Voices::start`] put a note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Start {
    /// The voice that plays the note, set to [`Phase::On`].
    pub voice: usize,
    /// A voice stolen to stay within the limit, set to [`Phase::Fade`]:
    /// the engine fades it out.
    pub stolen: Option<usize>,
    /// The voice taken was still fading (every slot was busy): the engine
    /// starts it fresh, cutting what was left of its fade.
    pub cut: bool,
}

/// `N` voices: the notes the limit allows, plus spare slots for stolen
/// voices to fade out in.
#[derive(Clone, Copy, Debug)]
pub struct Voices<const N: usize> {
    pub slots: [Slot; N],
    clock: u64,
    /// Voices stolen to stay within the limit.
    pub stolen: u64,
    /// Fades cut short because every slot was busy (should stay 0 with
    /// enough spare slots).
    pub cut: u64,
}

impl<const N: usize> Default for Voices<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Voices<N> {
    pub fn new() -> Voices<N> {
        Voices {
            slots: [Slot::default(); N],
            clock: 0,
            stolen: 0,
            cut: 0,
        }
    }

    /// Voices on or releasing.
    pub fn sounding(&self) -> usize {
        self.slots.iter().filter(|s| s.sounding()).count()
    }

    /// Voices doing anything (fading included).
    pub fn busy(&self) -> usize {
        self.slots.iter().filter(|s| s.phase != Phase::Free).count()
    }

    /// The voice to give up first: a released one before a held one, the
    /// quietest released one, else the oldest held.
    pub fn victim(&self) -> Option<usize> {
        let quietest_released = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.phase == Phase::Release)
            .min_by(|a, b| a.1.level.total_cmp(&b.1.level).then(a.1.age.cmp(&b.1.age)))
            .map(|(i, _)| i);
        quietest_released.or_else(|| {
            self.slots
                .iter()
                .enumerate()
                .filter(|(_, s)| s.phase == Phase::On)
                .min_by_key(|(_, s)| s.age)
                .map(|(i, _)| i)
        })
    }

    /// A voice for `note` with at most `limit` (at least 1) voices
    /// sounding: a free slot, after stealing one if the limit is reached;
    /// with no slot free, the quietest fading one.
    pub fn start(&mut self, note: u8, limit: usize) -> Start {
        let limit = limit.clamp(1, N);
        let mut stolen = None;
        if self.sounding() >= limit {
            if let Some(v) = self.victim() {
                self.slots[v].phase = Phase::Fade;
                self.stolen += 1;
                stolen = Some(v);
            }
        }
        let free = self.slots.iter().position(|s| s.phase == Phase::Free);
        let (voice, cut) = match free {
            Some(i) => (i, false),
            None => {
                // Every slot busy: take the quietest fading one (the one
                // just stolen if nothing else is fading).
                let i = self
                    .slots
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.phase == Phase::Fade)
                    .min_by(|a, b| a.1.level.total_cmp(&b.1.level))
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.cut += 1;
                (i, true)
            }
        };
        let stolen = stolen.filter(|&s| s != voice);
        self.clock += 1;
        self.slots[voice] = Slot {
            phase: Phase::On,
            note,
            age: self.clock,
            level: 0.0,
        };
        Start { voice, stolen, cut }
    }

    /// `note`'s voices that are on go to their release. Returns how many.
    pub fn release(&mut self, note: u8) -> usize {
        let mut n = 0;
        for s in &mut self.slots {
            if s.phase == Phase::On && s.note == note {
                s.phase = Phase::Release;
                n += 1;
            }
        }
        n
    }

    /// Every voice that is on goes to its release (an AllOff).
    pub fn release_all(&mut self) {
        for s in &mut self.slots {
            if s.phase == Phase::On {
                s.phase = Phase::Release;
            }
        }
    }

    /// The voice is silent: its slot is free again.
    pub fn free(&mut self, voice: usize) {
        if let Some(s) = self.slots.get_mut(voice) {
            s.phase = Phase::Free;
            s.level = 0.0;
        }
    }

    /// Every voice free at once (a reset).
    pub fn clear(&mut self) {
        for s in &mut self.slots {
            *s = Slot::default();
        }
    }
}

// ------------------------------------------------------------- glide

/// How a glide moves between notes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlideMode {
    /// Every interval takes the glide's time (99 % of the way), as an
    /// analog synth's portamento does.
    #[default]
    Time,
    /// A fixed speed in semitones a second, so a wider interval takes
    /// longer.
    Rate,
}

impl GlideMode {
    pub const NAMES: &'static [&'static str] = &["Time", "Rate"];

    pub fn from_index(i: f32) -> GlideMode {
        if i as i32 == 1 {
            GlideMode::Rate
        } else {
            GlideMode::Time
        }
    }
}

/// A glide, stepped once a control step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glide {
    mode: GlideMode,
    /// Time: the share of the way covered each step.
    coef: f32,
    /// Rate: semitones each step.
    step: f32,
}

impl Default for Glide {
    fn default() -> Self {
        Glide {
            mode: GlideMode::Time,
            coef: 1.0,
            step: f32::INFINITY,
        }
    }
}

impl Glide {
    /// By time: `ms` to cover 99 % of any interval (0: no glide).
    pub fn time(ms: f32, steps_per_sec: f32) -> Glide {
        let n = ms.max(0.0) * 0.001 * steps_per_sec;
        let coef = if n < 1.0 {
            1.0
        } else {
            1.0 - 0.01f32.powf(1.0 / n)
        };
        Glide {
            mode: GlideMode::Time,
            coef,
            step: f32::INFINITY,
        }
    }

    /// By rate: `semis_per_sec` semitones a second (0 or less: no glide).
    pub fn rate(semis_per_sec: f32, steps_per_sec: f32) -> Glide {
        let step = if semis_per_sec > 0.0 {
            semis_per_sec / steps_per_sec.max(1.0)
        } else {
            f32::INFINITY
        };
        Glide {
            mode: GlideMode::Rate,
            coef: 1.0,
            step,
        }
    }

    pub fn mode(&self) -> GlideMode {
        self.mode
    }

    /// One control step of `pitch` (in notes) toward `target`.
    #[inline]
    pub fn step(&self, pitch: &mut f32, target: f32) {
        match self.mode {
            GlideMode::Time => {
                *pitch += (target - *pitch) * self.coef;
                if (target - *pitch).abs() < 1e-4 {
                    *pitch = target;
                }
            }
            GlideMode::Rate => {
                let d = target - *pitch;
                if d.abs() <= self.step {
                    *pitch = target;
                } else {
                    *pitch += self.step.copysign(d);
                }
            }
        }
    }
}

// ------------------------------------------------------------- envelope

/// Where an [`Adsr`] is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    #[default]
    Idle,
    Attack,
    Decay,
    Release,
}

/// Where the attack aims past full level, so it curves like an analog
/// envelope's charge and still arrives.
const ATTACK_AIM: f32 = 1.3;
/// Where the release aims below zero, so it arrives at silence rather
/// than approaching it forever.
const RELEASE_AIM: f32 = -1e-4;
/// The release aim of an analog-style envelope ([`Adsr::analog_release`]):
/// it falls the first 40 dB more slowly, reaches -60 dB at its label and
/// silence just after (1.02 times), so its voice ends with its label.
pub const ANALOG_RELEASE_AIM: f32 = -0.01;
/// The release's label: the time to fall 60 dB.
const RELEASE_DROP: f32 = 1e-3;

/// An ADSR envelope with exponential segments whose times mean what they
/// say: Attack is the time from silence to full level, Decay the time to
/// cover 99 % of the way to Sustain, Release the time to fall 60 dB from
/// full level (it reaches silence at about 1.35 times that, or 1.02 times
/// with [`Adsr::analog_release`]). A gate on starts the attack from
/// wherever the level is, so a retrigger never clicks. Stepped once a
/// control step (or once a frame); the voice ramps its gain between steps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adsr {
    pub level: f32,
    pub stage: Stage,
    ca: f32,
    cd: f32,
    cr: f32,
    sustain: f32,
    /// Where the release aims.
    raim: f32,
}

impl Default for Adsr {
    fn default() -> Adsr {
        Adsr {
            level: 0.0,
            stage: Stage::Idle,
            ca: 1.0,
            cd: 1.0,
            cr: 1.0,
            sustain: 1.0,
            raim: RELEASE_AIM,
        }
    }
}

/// The per-step share that takes a level from `from` to `to`, aiming at
/// `aim`, in `n` steps (1 when `n` is under a step).
fn share(from: f32, to: f32, aim: f32, n: f32) -> f32 {
    if n < 1.0 {
        return 1.0;
    }
    1.0 - ((to - aim) / (from - aim)).powf(1.0 / n)
}

impl Adsr {
    /// An envelope whose release aims at [`ANALOG_RELEASE_AIM`], as an
    /// analog one's capacitor might: the same label (60 dB down in its
    /// time), a slower start and silence just after its time, so a voice
    /// lasts no longer than its release says. Poly's, which had this curve
    /// before the voice core. Call [`Adsr::set`] after it.
    pub fn analog_release() -> Adsr {
        Adsr {
            raim: ANALOG_RELEASE_AIM,
            ..Adsr::default()
        }
    }

    /// Times in ms, `sustain` 0 to 1, at `steps_per_sec` control steps a
    /// second.
    pub fn set(&mut self, a_ms: f32, d_ms: f32, sustain: f32, r_ms: f32, steps_per_sec: f32) {
        let n = |ms: f32| ms.max(0.0) * 0.001 * steps_per_sec;
        self.ca = share(0.0, 1.0, ATTACK_AIM, n(a_ms));
        self.cd = if n(d_ms) < 1.0 {
            1.0
        } else {
            1.0 - 0.01f32.powf(1.0 / n(d_ms))
        };
        self.cr = share(1.0, RELEASE_DROP, self.raim, n(r_ms));
        self.sustain = sustain.clamp(0.0, 1.0);
    }

    /// Starts the attack from the current level.
    pub fn gate_on(&mut self) {
        self.stage = Stage::Attack;
    }

    /// Starts the release (from any stage but idle).
    pub fn gate_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    pub fn idle(&self) -> bool {
        self.stage == Stage::Idle
    }

    pub fn clear(&mut self) {
        self.level = 0.0;
        self.stage = Stage::Idle;
    }

    /// One control step: the level after it.
    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += (ATTACK_AIM - self.level) * self.ca;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level += (self.sustain - self.level) * self.cd;
                // Toward a sustain of 0 the level would sink through the
                // denormals for as long as the key is held; under -400 dB
                // it is exact 0.
                if self.level < 1e-20 {
                    self.level = 0.0;
                }
            }
            Stage::Release => {
                self.level += (self.raim - self.level) * self.cr;
                if self.level <= 0.0 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

// ------------------------------------------------------------- LFOs

/// The rate, in Hz, of a cycle `beats` beats long at `bpm`.
#[inline]
pub fn sync_hz(beats: f32, bpm: f64) -> f32 {
    (bpm.max(0.0) / 60.0) as f32 / beats.max(1.0 / 64.0)
}

/// An LFO: free at a rate in Hz, or synced (a cycle a number of beats
/// long), with a delay and a fade-in that start again on each key, and a
/// phase that goes back to its start on each key or runs on. One per
/// engine and two per voice.
#[derive(Clone, Copy, Debug)]
pub struct Lfo {
    shape: LfoShape,
    phase: f32,
    /// Phase per step.
    inc: f32,
    /// The cycle's length in beats; 0 when free.
    cycle: f32,
    start: f32,
    retrigger: bool,
    delay: u32,
    fade: u32,
    /// Steps since the last key.
    age: u32,
    held: f32,
    rng: Rng,
}

impl Lfo {
    /// A still sine LFO; `seed` drives its sample and hold.
    pub fn new(seed: u64) -> Lfo {
        let mut rng = Rng::new(seed);
        let held = rng.bipolar();
        Lfo {
            shape: LfoShape::Sine,
            phase: 0.0,
            inc: 0.0,
            cycle: 0.0,
            start: 0.0,
            retrigger: true,
            delay: 0,
            fade: 0,
            age: 0,
            held,
            rng,
        }
    }

    /// Free running at `hz`. `steps_per_sec` is how often
    /// [`Lfo::next`] is called. A step never moves more than half a
    /// cycle.
    pub fn set_free(&mut self, shape: LfoShape, hz: f32, steps_per_sec: f32) {
        self.shape = shape;
        self.cycle = 0.0;
        self.inc = (hz.max(0.0) / steps_per_sec.max(1.0)).min(0.5);
    }

    /// Synced: one cycle every `beats` beats at `bpm`. Call it again when
    /// the tempo changes; [`Lfo::follow`] locks the phase to the song.
    pub fn set_synced(&mut self, shape: LfoShape, beats: f32, bpm: f64, steps_per_sec: f32) {
        self.shape = shape;
        self.cycle = beats.max(1.0 / 64.0);
        self.inc = (sync_hz(self.cycle, bpm) / steps_per_sec.max(1.0)).min(0.5);
    }

    /// The delay before the LFO starts and the time it takes to fade in,
    /// both counted from the last key.
    pub fn set_delay(&mut self, delay_ms: f32, fade_ms: f32, steps_per_sec: f32) {
        let steps = |ms: f32| (ms.max(0.0) * 0.001 * steps_per_sec.max(1.0)).round() as u32;
        self.delay = steps(delay_ms);
        self.fade = steps(fade_ms);
    }

    /// Whether each key sends the phase back to `start` (0 to 1).
    pub fn set_retrigger(&mut self, on: bool, start: f32) {
        self.retrigger = on;
        self.start = start.rem_euclid(1.0);
    }

    /// A key starts: the delay and fade start again, and a retriggering
    /// LFO goes back to its start phase.
    pub fn key(&mut self) {
        self.age = 0;
        if self.retrigger {
            self.phase = self.start;
        }
    }

    /// Locks a synced LFO to the song at `beats` (the transport's
    /// position) while it plays. A free LFO is left alone.
    pub fn follow(&mut self, beats: f64) {
        if self.cycle > 0.0 {
            let p = ((beats / f64::from(self.cycle)).rem_euclid(1.0) as f32 + self.start).fract();
            if p < self.phase {
                self.held = self.rng.bipolar();
            }
            self.phase = p;
        }
    }

    /// Where the LFO is in its cycle, 0 to 1.
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// The LFO's value, -1 to 1 times its fade-in, then a step on.
    #[inline]
    pub fn next(&mut self) -> f32 {
        let gain = if self.age < self.delay {
            0.0
        } else if self.fade == 0 {
            1.0
        } else {
            ((self.age - self.delay) as f32 / self.fade as f32).min(1.0)
        };
        let v = lfo_value(self.shape, self.phase, self.held) * gain;
        self.age = self.age.saturating_add(1);
        self.phase += self.inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            self.held = self.rng.bipolar();
        }
        v
    }
}

// ------------------------------------------------------------- velocity

/// The gain for `velocity` (1 to 127) on a square-law curve spanning
/// `range_db` from velocity 1 to 127 (Dannenberg, "The interpretation of
/// MIDI velocity", ICMC 2006): 1 at 127, `-range_db` at 1, and the
/// square root of the gain a straight line in velocity between. A range
/// of 0 gives 1 for every velocity.
pub fn velocity_gain(velocity: u8, range_db: f32) -> f32 {
    VelCurve::new(range_db).gain(f32::from(velocity))
}

/// [`velocity_gain`]'s curve worked out once for a range, for an engine
/// that asks for many notes' gains: `gain(v)` is `velocity_gain(v,
/// range_db)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VelCurve {
    m: f32,
    b: f32,
}

impl VelCurve {
    pub fn new(range_db: f32) -> VelCurve {
        if range_db <= 0.0 {
            // No range: every velocity plays at full level.
            return VelCurve { m: 0.0, b: 1.0 };
        }
        let r = 10f32.powf(range_db / 20.0);
        let b = 127.0 / (126.0 * r.sqrt()) - 1.0 / 126.0;
        VelCurve {
            m: (1.0 - b) / 127.0,
            b,
        }
    }

    /// The gain for a velocity from 1 to 127 (a fraction between is fine).
    #[inline]
    pub fn gain(&self, velocity: f32) -> f32 {
        let a = self.m * velocity.clamp(1.0, 127.0) + self.b;
        a * a
    }
}

// ------------------------------------------------------------- play routes

/// What a play route reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    Off,
    /// The note's velocity, 0 to 1.
    Velocity,
    /// The note's pressure (after the pressure shaper), 0 to 1.
    Pressure,
    /// The note's own tone (MPE's Y), 0 to 1.
    Tone,
    /// The mod wheel, 0 to 1.
    ModWheel,
    /// A breath controller, 0 to 1.
    Breath,
    /// A foot controller, 0 to 1.
    Foot,
    /// The note's place on the keyboard: 0 at C4 (60), 1 five octaves up
    /// (120), -1 five down (0).
    Key,
    /// A value per note, -1 to 1, fixed when it starts.
    Random,
}

impl Source {
    pub const NAMES: &'static [&'static str] = &[
        "Off",
        "Velocity",
        "Pressure",
        "Tone",
        "Mod wheel",
        "Breath",
        "Foot",
        "Key",
        "Random",
    ];

    pub fn from_index(i: f32) -> Source {
        match i as i32 {
            1 => Source::Velocity,
            2 => Source::Pressure,
            3 => Source::Tone,
            4 => Source::ModWheel,
            5 => Source::Breath,
            6 => Source::Foot,
            7 => Source::Key,
            8 => Source::Random,
            _ => Source::Off,
        }
    }
}

/// How many routes a sound has.
pub const ROUTES: usize = 8;

/// The controllers the whole instrument shares, from
/// `NodeEvent::Control`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Channel {
    pub mod_wheel: f32,
    pub breath: f32,
    pub foot: f32,
}

impl Channel {
    /// Takes a controller's new value; the pedals and anything not a
    /// number change nothing here.
    pub fn set(&mut self, ctl: Ctl, value: f32) {
        let Some(v) = Ctl::clamp(value) else {
            return;
        };
        match ctl {
            Ctl::ModWheel => self.mod_wheel = v,
            Ctl::Breath => self.breath = v,
            Ctl::Foot => self.foot = v,
            Ctl::Sustain | Ctl::Sostenuto | Ctl::Soft => {}
        }
    }
}

/// What one voice's routes read: its own values, beside the [`Channel`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Inputs {
    pub velocity: f32,
    pub pressure: f32,
    pub tone: f32,
    pub key: f32,
    pub random: f32,
}

impl Inputs {
    /// A voice's inputs at its start: velocity 0 to 127, the note, and a
    /// random value -1 to 1.
    pub fn at_start(velocity: u8, note: u8, random: f32) -> Inputs {
        Inputs {
            velocity: f32::from(velocity.min(127)) / 127.0,
            pressure: 0.0,
            tone: 0.0,
            key: (f32::from(note.min(127)) - 60.0) / 60.0,
            random: random.clamp(-1.0, 1.0),
        }
    }
}

/// One play route: `source` times `depth` (-1 to 1) onto the engine's
/// target `target` (an index into the engine's own list of targets,
/// 0 meaning none).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Route {
    pub source: Source,
    pub target: u8,
    pub depth: f32,
}

impl Route {
    /// This route's amount now, -1 to 1.
    #[inline]
    pub fn amount(&self, v: &Inputs, c: &Channel) -> f32 {
        let s = match self.source {
            Source::Off => return 0.0,
            Source::Velocity => v.velocity,
            Source::Pressure => v.pressure,
            Source::Tone => v.tone,
            Source::ModWheel => c.mod_wheel,
            Source::Breath => c.breath,
            Source::Foot => c.foot,
            Source::Key => v.key,
            Source::Random => v.random,
        };
        s * self.depth.clamp(-1.0, 1.0)
    }
}

/// The sum of every route aimed at `target` (target 0 is none).
#[inline]
pub fn routed(routes: &[Route], target: u8, v: &Inputs, c: &Channel) -> f32 {
    if target == 0 {
        return 0.0;
    }
    routes
        .iter()
        .filter(|r| r.target == target)
        .map(|r| r.amount(v, c))
        .sum()
}

// ------------------------------------------------------------- output

/// A one-pole DC blocker (a high-pass at a few Hz) for an engine's
/// output: asymmetric drive and some FM pairs make DC that eats headroom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DcBlock {
    x1: f32,
    y1: f32,
    r: f32,
}

impl DcBlock {
    /// A blocker with its corner at `hz` at `rate`.
    pub fn new(rate: u32, hz: f32) -> DcBlock {
        let r = 1.0 - std::f32::consts::TAU * hz / rate.max(1) as f32;
        DcBlock {
            x1: 0.0,
            y1: 0.0,
            r: r.clamp(0.0, 1.0),
        }
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + self.r * self.y1;
        self.x1 = x;
        // Below about -400 dB the state is let go, so silence stays exact
        // and never runs on denormals.
        self.y1 = if y.abs() < 1e-20 { 0.0 } else { y };
        self.y1
    }

    pub fn run(&mut self, buf: &mut [f32]) {
        for x in buf {
            *x = self.tick(*x);
        }
    }

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decay_to_no_sustain_lands_on_exact_zero_not_the_denormals() {
        let mut e = Adsr::default();
        // A control step every 16 frames at 44.1 kHz.
        e.set(1.0, 5.0, 0.0, 100.0, 44_100.0 / 16.0);
        e.gate_on();
        for _ in 0..44_100 * 3 / 16 {
            assert!(!e.next().is_subnormal());
        }
        assert_eq!(e.level, 0.0);
        assert_eq!(e.stage, Stage::Decay, "still held");
    }

    // ---- notes

    #[test]
    fn the_stack_picks_by_priority_and_falls_back_as_keys_come_up() {
        let mut s = NoteStack::new();
        for k in [60, 55, 67] {
            s.push(k);
        }
        assert_eq!(s.pick(Priority::Last), Some(67));
        assert_eq!(s.pick(Priority::Low), Some(55));
        assert_eq!(s.pick(Priority::High), Some(67));
        s.remove(67);
        assert_eq!(s.pick(Priority::Last), Some(55));
        assert_eq!(s.pick(Priority::High), Some(60));
        // Pressed again, a key becomes the newest.
        s.push(60);
        assert_eq!(s.pick(Priority::Last), Some(60));
        assert_eq!(s.len(), 2);
        assert!(!s.remove(99));
        s.remove(55);
        s.remove(60);
        assert_eq!(s.pick(Priority::Last), None);
    }

    #[test]
    fn a_full_stack_forgets_its_oldest_key() {
        let mut s = NoteStack::new();
        for k in 0..(STACK as u8 + 1) {
            s.push(k);
        }
        assert_eq!(s.len(), STACK);
        assert!(!s.contains(0));
        assert_eq!(s.pick(Priority::Low), Some(1));
    }

    // ---- voices

    #[test]
    fn notes_take_free_voices_then_steal_the_quietest_released_one() {
        let mut v = Voices::<6>::new();
        let a = v.start(60, 4).voice;
        let b = v.start(62, 4).voice;
        let c = v.start(64, 4).voice;
        let d = v.start(65, 4).voice;
        assert_eq!(v.sounding(), 4);
        v.release(62);
        v.release(64);
        v.slots[b].level = 0.3;
        v.slots[c].level = 0.1;
        // At the limit: the quieter released voice (64) goes.
        let s = v.start(67, 4);
        assert_eq!(s.stolen, Some(c));
        assert_eq!(v.slots[c].phase, Phase::Fade);
        assert!(s.voice != a && s.voice != d && s.voice != c);
        assert_eq!(v.sounding(), 4);
        assert_eq!(v.stolen, 1);
    }

    #[test]
    fn with_none_released_the_oldest_held_voice_is_stolen() {
        let mut v = Voices::<5>::new();
        let first = v.start(60, 3).voice;
        v.start(62, 3);
        v.start(64, 3);
        let s = v.start(65, 3);
        assert_eq!(s.stolen, Some(first));
        assert!(!s.cut);
    }

    #[test]
    fn with_every_slot_busy_a_fading_voice_is_cut() {
        let mut v = Voices::<2>::new();
        v.start(60, 2);
        v.start(62, 2);
        // Both on, limit 2, no spare: one is stolen and its slot reused.
        let s = v.start(64, 2);
        assert!(s.cut);
        assert_eq!(s.stolen, None);
        assert_eq!(v.slots[s.voice].note, 64);
        assert_eq!(v.sounding(), 2);
        assert_eq!(v.cut, 1);
    }

    #[test]
    fn release_and_free_move_voices_on() {
        let mut v = Voices::<4>::new();
        let a = v.start(60, 4).voice;
        v.start(60, 4);
        assert_eq!(v.release(60), 2);
        assert_eq!(v.slots[a].phase, Phase::Release);
        v.free(a);
        assert_eq!(v.slots[a].phase, Phase::Free);
        assert_eq!(v.sounding(), 1);
        v.start(64, 4);
        v.release_all();
        assert_eq!(v.slots.iter().filter(|s| s.phase == Phase::On).count(), 0);
        v.clear();
        assert_eq!(v.busy(), 0);
    }

    // ---- glide

    const STEPS: f32 = 44_100.0 / 16.0;

    #[test]
    fn a_timed_glide_covers_any_interval_in_its_time() {
        for interval in [1.0f32, 12.0, 24.0] {
            let g = Glide::time(200.0, STEPS);
            let mut p = 48.0;
            let target = 48.0 + interval;
            let mut n = 0;
            while (target - p) > 0.01 * interval {
                g.step(&mut p, target);
                n += 1;
            }
            let ms = n as f32 / STEPS * 1000.0;
            assert!((ms - 200.0).abs() < 200.0 * 0.03, "{interval}: {ms} ms");
        }
    }

    #[test]
    fn a_rated_glide_takes_longer_over_a_wider_interval_and_lands_exactly() {
        let g = Glide::rate(24.0, STEPS);
        for (interval, want_ms) in [(12.0f32, 500.0f32), (24.0, 1000.0)] {
            let mut p = 40.0;
            let target = 40.0 - interval;
            let mut n = 0;
            while p != target {
                g.step(&mut p, target);
                n += 1;
            }
            let ms = n as f32 / STEPS * 1000.0;
            assert!((ms - want_ms).abs() < want_ms * 0.01, "{interval}: {ms} ms");
        }
        // No glide: there at once.
        let mut p = 10.0;
        Glide::rate(0.0, STEPS).step(&mut p, 70.0);
        assert_eq!(p, 70.0);
        Glide::time(0.0, STEPS).step(&mut p, 20.0);
        assert_eq!(p, 20.0);
    }

    // ---- envelope

    fn steps_to(e: &mut Adsr, done: impl Fn(&Adsr) -> bool) -> usize {
        let mut n = 0;
        while !done(e) && n < 10_000_000 {
            e.next();
            n += 1;
        }
        n
    }

    #[test]
    fn the_attack_reaches_full_level_at_its_label() {
        for ms in [5.0f32, 100.0, 2000.0] {
            let mut e = Adsr::default();
            e.set(ms, 300.0, 0.5, 500.0, STEPS);
            e.gate_on();
            let n = steps_to(&mut e, |e| e.stage == Stage::Decay);
            let got = n as f32 / STEPS * 1000.0;
            // Within a control step (5.8 ms) or 2 %.
            assert!(
                (got - ms).abs() <= (1000.0 / STEPS).max(ms * 0.02),
                "{ms}: {got}"
            );
        }
    }

    #[test]
    fn the_release_falls_60_db_in_its_label_and_ends_in_silence() {
        for ms in [50.0f32, 400.0, 3000.0] {
            let mut e = Adsr::default();
            e.set(1.0, 1.0, 1.0, ms, STEPS);
            e.gate_on();
            steps_to(&mut e, |e| e.stage == Stage::Decay);
            for _ in 0..10 {
                e.next();
            }
            assert_eq!(e.level, 1.0);
            e.gate_off();
            let n = steps_to(&mut e, |e| e.level <= 1e-3);
            let got = n as f32 / STEPS * 1000.0;
            assert!(
                (got - ms).abs() <= (1000.0 / STEPS).max(ms * 0.02),
                "{ms}: {got}"
            );
            let end = steps_to(&mut e, |e| e.idle());
            assert_eq!(e.level, 0.0);
            assert!(end > 0);
        }
    }

    #[test]
    fn an_analog_release_has_the_same_label_and_ends_with_it() {
        for ms in [50.0f32, 400.0, 3000.0] {
            let mut e = Adsr::analog_release();
            e.set(1.0, 1.0, 1.0, ms, STEPS);
            e.gate_on();
            steps_to(&mut e, |e| e.stage == Stage::Decay);
            e.gate_off();
            let n = steps_to(&mut e, |e| e.level <= 1e-3);
            let got = n as f32 / STEPS * 1000.0;
            assert!(
                (got - ms).abs() <= (1000.0 / STEPS).max(ms * 0.02),
                "{ms}: {got}"
            );
            // Silent within 3 % more; the default takes about 35 % more.
            let end = n + steps_to(&mut e, |e| e.idle());
            let at = end as f32 / STEPS * 1000.0;
            assert!(at <= ms * 1.03 + 1000.0 / STEPS, "{ms}: silent at {at}");
            assert_eq!(e.level, 0.0);
        }
        // Slower at first: at a third of the label it is louder.
        let third = |mut e: Adsr| {
            e.set(1.0, 1.0, 1.0, 900.0, STEPS);
            e.gate_on();
            steps_to(&mut e, |e| e.stage == Stage::Decay);
            e.gate_off();
            for _ in 0..(0.3 * STEPS) as usize {
                e.next();
            }
            e.level
        };
        assert!(third(Adsr::analog_release()) > 1.5 * third(Adsr::default()));
    }

    #[test]
    fn decay_covers_99_percent_and_a_retrigger_starts_where_it_is() {
        let mut e = Adsr::default();
        e.set(10.0, 200.0, 0.25, 300.0, STEPS);
        e.gate_on();
        steps_to(&mut e, |e| e.stage == Stage::Decay);
        let n = steps_to(&mut e, |e| e.level - 0.25 <= 0.01 * 0.75);
        let got = n as f32 / STEPS * 1000.0;
        assert!((got - 200.0).abs() < 6.0, "{got}");
        e.gate_off();
        for _ in 0..20 {
            e.next();
        }
        let before = e.level;
        e.gate_on();
        let after = e.next();
        assert!(
            after > before && after < before + 0.2,
            "{before} -> {after}"
        );
    }

    // ---- LFOs

    #[test]
    fn a_free_lfo_runs_at_its_rate() {
        let mut l = Lfo::new(1);
        l.set_free(LfoShape::Saw, 2.0, 3000.0);
        let mut wraps = 0;
        let mut last = l.phase();
        for i in 0..3010 {
            l.next();
            if l.phase() < last {
                wraps += 1;
            }
            last = l.phase();
            if i == 749 {
                // A quarter second at 2 Hz is half a cycle.
                assert!((l.phase() - 0.5).abs() < 1e-3, "{}", l.phase());
            }
        }
        assert_eq!(wraps, 2);
    }

    #[test]
    fn an_lfo_waits_out_its_delay_then_fades_in() {
        let mut l = Lfo::new(1);
        // Slow enough that a square stays at its top for the whole test.
        l.set_free(LfoShape::Square, 0.1, 1000.0);
        l.set_delay(100.0, 200.0, 1000.0);
        l.key();
        let out: Vec<f32> = (0..400).map(|_| l.next()).collect();
        assert!(out[..100].iter().all(|&v| v == 0.0));
        assert!((out[200] - 0.5).abs() < 1e-6, "{}", out[200]);
        assert_eq!(out[300], 1.0);
        assert_eq!(out[399], 1.0);
        // A key starts the delay again.
        l.key();
        assert_eq!(l.next(), 0.0);
    }

    #[test]
    fn a_key_sends_a_retriggering_lfo_back_to_its_start() {
        let mut l = Lfo::new(1);
        l.set_free(LfoShape::Saw, 1.0, 100.0);
        l.set_retrigger(true, 0.25);
        for _ in 0..30 {
            l.next();
        }
        l.key();
        assert_eq!(l.phase(), 0.25);
        // One that runs on keeps its place.
        l.set_retrigger(false, 0.0);
        for _ in 0..10 {
            l.next();
        }
        let p = l.phase();
        l.key();
        assert_eq!(l.phase(), p);
    }

    #[test]
    fn a_synced_lfo_follows_the_song() {
        assert_eq!(sync_hz(1.0, 120.0), 2.0);
        assert_eq!(sync_hz(4.0, 120.0), 0.5);
        let mut l = Lfo::new(1);
        l.set_synced(LfoShape::Saw, 2.0, 120.0, 1000.0);
        l.follow(5.5);
        assert!((l.phase() - 0.75).abs() < 1e-6);
        // Stopped, it runs on at the tempo: a 2-beat cycle at 120 BPM is
        // a second long.
        for _ in 0..500 {
            l.next();
        }
        assert!((l.phase() - 0.25).abs() < 1e-3, "{}", l.phase());
        // A free LFO ignores the song.
        let mut f = Lfo::new(1);
        f.set_free(LfoShape::Saw, 1.0, 1000.0);
        f.follow(5.5);
        assert_eq!(f.phase(), 0.0);
    }

    #[test]
    fn sample_and_hold_holds_a_value_for_each_cycle() {
        let run = |seed| {
            let mut l = Lfo::new(seed);
            l.set_free(LfoShape::SampleHold, 10.0, 1000.0);
            (0..1000).map(|_| l.next()).collect::<Vec<f32>>()
        };
        let a = run(7);
        assert_eq!(a, run(7));
        assert_ne!(a, run(8));
        assert!(a.iter().all(|v| (-1.0..1.0).contains(v)));
        for cycle in a.chunks(100) {
            // Each tenth of a second is one cycle and one value (float
            // steps may land the change a step either side).
            let held = cycle[50];
            assert!(cycle[2..98].iter().all(|&v| v == held));
        }
        let mut distinct: Vec<f32> = a.chunks(100).map(|c| c[50]).collect();
        distinct.dedup();
        assert!(distinct.len() > 5);
    }

    // ---- velocity

    #[test]
    fn velocity_follows_its_square_law_over_the_range() {
        for range in [10.0f32, 30.0, 40.0] {
            let top = velocity_gain(127, range);
            let bottom = velocity_gain(1, range);
            assert!((top - 1.0).abs() < 1e-6);
            let db = 20.0 * bottom.log10();
            assert!((db + range).abs() < 0.01, "{range}: {db}");
            // The square root is a straight line in velocity.
            let s = |v: u8| velocity_gain(v, range).sqrt();
            let mid = (s(1) + s(127)) / 2.0;
            assert!((s(64) - mid).abs() < 1e-5);
            // Louder with every step.
            for v in 1..127u8 {
                assert!(velocity_gain(v + 1, range) > velocity_gain(v, range));
            }
        }
        assert_eq!(velocity_gain(20, 0.0), 1.0);
        assert_eq!(velocity_gain(0, 30.0), velocity_gain(1, 30.0));
    }

    // ---- routes

    #[test]
    fn routes_add_their_sources_onto_their_target() {
        let mut routes = [Route::default(); ROUTES];
        routes[0] = Route {
            source: Source::Velocity,
            target: 1,
            depth: 0.5,
        };
        routes[1] = Route {
            source: Source::ModWheel,
            target: 1,
            depth: -1.0,
        };
        routes[2] = Route {
            source: Source::Key,
            target: 2,
            depth: 1.0,
        };
        let v = Inputs::at_start(127, 120, 0.0);
        let mut c = Channel::default();
        c.set(Ctl::ModWheel, 0.25);
        c.set(Ctl::Sustain, 1.0);
        assert!((routed(&routes, 1, &v, &c) - (0.5 - 0.25)).abs() < 1e-6);
        assert!((routed(&routes, 2, &v, &c) - 1.0).abs() < 1e-6);
        assert_eq!(routed(&routes, 3, &v, &c), 0.0);
        assert_eq!(routed(&routes, 0, &v, &c), 0.0);
        c.set(Ctl::Breath, f32::NAN);
        assert_eq!(c.breath, 0.0);
        assert_eq!(Source::NAMES.len(), Source::Random as usize + 1);
        for (i, _) in Source::NAMES.iter().enumerate() {
            assert_eq!(Source::from_index(i as f32) as usize, i);
        }
    }

    // ---- output

    #[test]
    fn the_dc_blocker_removes_an_offset_and_passes_the_music() {
        let rate = 44_100;
        let mut d = DcBlock::new(rate, 5.0);
        let mut buf = vec![0.5f32; rate as usize];
        d.run(&mut buf);
        assert!(
            buf[rate as usize - 1].abs() < 1e-3,
            "{}",
            buf[rate as usize - 1]
        );
        // A 100 Hz sine keeps its level within 0.05 dB.
        let mut d = DcBlock::new(rate, 5.0);
        let sine: Vec<f32> = (0..rate as usize)
            .map(|i| (std::f32::consts::TAU * 100.0 * i as f32 / rate as f32).sin())
            .collect();
        let mut out = sine.clone();
        d.run(&mut out);
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        let db = 20.0 * (rms(&out[22_050..]) / rms(&sine[22_050..])).log10();
        assert!(db.abs() < 0.05, "{db}");
        // Silence in, exact silence out once the state has let go.
        let mut z = vec![0.0f32; rate as usize * 30];
        d.run(&mut z);
        assert_eq!(*z.last().unwrap(), 0.0);
    }
}
