//! Poly: a subtractive synth. Two oscillators per voice, saw, pulse,
//! triangle, sine or a seven-saw supersaw read from band-limited tables
//! ([`crate::osc`]), with up to four unison stacks, a sub oscillator and
//! noise, a drive stage
//! into a ladder or state-variable filter and a distortion stage after it,
//! two ADSR envelopes from the voice core ([`crate::voice::Adsr`]: an
//! attack of 100 ms reaches full level at 100 ms, a release of 1 s falls
//! 60 dB in 1 s and ends, with the analog curve Poly always had), the
//! voice core's velocity curve (a range in dB), a
//! tempo-syncable LFO, poly, mono and legato modes with glide.
//!
//! Before preset format 2 the envelopes' attack reached full level at a
//! third of its label and the velocity was a percentage; [`upgrade`]
//! converts a sound saved then so it plays as it did.
//!
//! Pad pressure goes through the shared shaper ([`mpfree_node::pressure`])
//! into four slots ([`crate::pressure::POLY_TARGETS`]): cutoff, resonance,
//! a level swell, vibrato (its own sine at Vib rate), the LFO's rate, pitch,
//! drive, pulse width, sub and noise.
//!
//! Timing: amp envelopes, oscillators and filters run every frame. Pitch,
//! cutoff, the LFO, the filter envelope, pressure and parameter smoothing
//! run at a control rate of one step every [`CTRL`] frames, aligned to the
//! absolute frame, so the output does not depend on how the host cuts
//! blocks. Each voice's gain is ramped across a control step.

use mpfree_node::{
    Ctx, Cull, Expr, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Pressure, PressureChord,
    PressureCoefs, PressureMode, PressureShape, Smooth, Timed, Tuning,
};

use crate::dsp::*;
use crate::expr::{expr_coef, NoteExprs, VoiceExpr};
use crate::osc::{self, Classic, Shape, Super, SAWS};
use crate::params::*;
use crate::pressure::{lfo_gate, swell_gain, Depths, Slot};
use crate::voice::{routed, Adsr, Channel, DcBlock, Inputs, Lfo, Route, Source, VelCurve};

/// Frames per control step.
pub const CTRL: u64 = 16;

/// The DC blocker's corner: low enough to leave the lowest notes alone
/// (under 0.1 dB at 32 Hz, C1).
const DC_HZ: f32 = 5.0;
/// The most frames one render pass covers (one control step).
const SEG: usize = CTRL as usize;
/// The most notes sounding at once (the Voices parameter's top).
pub const MAX_POLY: usize = 16;
/// Voice slots: the notes plus room for stolen voices to fade out.
pub const POOL: usize = MAX_POLY + 4;
const UNISON_MAX: usize = 4;
/// A stolen voice fades out over this long.
const KILL_MS: f32 = 3.0;
/// Peak of one full-velocity note at the defaults, about -12 dBFS.
const VOICE_GAIN: f32 = 0.16;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.poly",
    name: "Poly",
    kind: Kind::Instrument,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Free,
    /// The key is down (or held by the mono note stack).
    On,
    Release,
    /// Stolen: fading out fast, then free.
    Kill,
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    state: State,
    note: u8,
    vel: f32,
    age: u64,
    /// Current and target pitch in (fractional) notes, for glide.
    pitch: f32,
    target: f32,
    /// Phases as 32-bit fractions of a cycle ([`osc::phase`]); the sub's
    /// is a plain 0 to 1.
    ph1: [u32; UNISON_MAX],
    ph2: [u32; UNISON_MAX],
    ph_sub: f32,
    /// Each stack's supersaw phases, for either oscillator on Supersaw.
    ss1: [[u32; SAWS]; UNISON_MAX],
    ss2: [[u32; SAWS]; UNISON_MAX],
    /// Each stack's table level ([`osc::level_for`]), set with the phase
    /// increments.
    lv1: [u8; UNISON_MAX],
    lv2: [u8; UNISON_MAX],
    /// Per-stack phase increments and pan gains, set each control step:
    /// in cycles a sample, and as the tables read them.
    dt1: [f32; UNISON_MAX],
    dt2: [f32; UNISON_MAX],
    di1: [u32; UNISON_MAX],
    di2: [u32; UNISON_MAX],
    dt_sub: f32,
    pan_l: [f32; UNISON_MAX],
    pan_r: [f32; UNISON_MAX],
    pw1: f32,
    pw2: f32,
    amp: Adsr,
    fenv: Adsr,
    coefs: FilterCoefs,
    filt: [Filter; 2],
    /// Gain from velocity, the LFO and pressure, ramped across each control
    /// step by `gain_inc` a frame.
    gain: f32,
    gain_inc: f32,
    /// Drive, sub and noise for this voice (pressure can move them).
    drive_gain: f32,
    drive_amt: f32,
    /// Keeps the driven voice at its undriven level (dsp::DriveLevel).
    drive_level: DriveLevel,
    /// The drive's and the distortion's antialiasing memory, each side.
    drive_aa: [DriveAa; 2],
    dist_aa: [DriveAa; 2],
    /// The distortion after the filter, as the drive (pressure can move
    /// it too), with its own level keeper.
    dist_gain: f32,
    dist_amt: f32,
    dist_level: DriveLevel,
    sub: f32,
    noise: f32,
    press: Pressure,
    /// The pressure this voice uses, 0 to 1: its own, the chord's or none.
    pv: f32,
    /// Its note's own slide and tone.
    ex: VoiceExpr,
    /// What its play routes read: velocity, key and a random value from
    /// its start, pressure and tone as they move.
    inp: Inputs,
    /// Its own LFO (LFO 2).
    lfo2: Lfo,
    kill_gain: f32,
    kill_step: f32,
    /// The inputs the phase increments and the filter coefficients were
    /// last worked out from (bit patterns), so a control step whose inputs
    /// have not moved skips the work: the numbers would come out the same.
    pitch_key: [u32; 8],
    coef_key: [u32; 3],
}

/// A key no real input matches (a NaN pattern arithmetic never makes).
const NO_KEY: u32 = u32::MAX;

impl Voice {
    /// Slot `i`'s voice: its LFO's sample and hold has a seed of its own.
    fn new(i: usize) -> Voice {
        Voice {
            state: State::Free,
            note: 0,
            vel: 0.0,
            age: 0,
            pitch: 60.0,
            target: 60.0,
            ph1: [0; UNISON_MAX],
            ph2: [0; UNISON_MAX],
            ph_sub: 0.0,
            ss1: [[0; SAWS]; UNISON_MAX],
            ss2: [[0; SAWS]; UNISON_MAX],
            lv1: [0; UNISON_MAX],
            lv2: [0; UNISON_MAX],
            dt1: [0.0; UNISON_MAX],
            dt2: [0.0; UNISON_MAX],
            di1: [0; UNISON_MAX],
            di2: [0; UNISON_MAX],
            dt_sub: 0.0,
            pan_l: [1.0; UNISON_MAX],
            pan_r: [1.0; UNISON_MAX],
            pw1: 0.5,
            pw2: 0.5,
            amp: Adsr::analog_release(),
            fenv: Adsr::analog_release(),
            coefs: FilterCoefs::default(),
            filt: [Filter::default(); 2],
            gain: 0.0,
            gain_inc: 0.0,
            drive_gain: 1.0,
            drive_amt: 0.0,
            drive_level: DriveLevel::new(),
            drive_aa: [DriveAa::default(); 2],
            dist_aa: [DriveAa::default(); 2],
            dist_gain: 1.0,
            dist_amt: 0.0,
            dist_level: DriveLevel::new(),
            sub: 0.0,
            noise: 0.0,
            press: Pressure::new(),
            pv: 0.0,
            ex: VoiceExpr::default(),
            inp: Inputs::default(),
            lfo2: Lfo::new(0x1F02_0000 + i as u64),
            kill_gain: 1.0,
            kill_step: 0.0,
            pitch_key: [NO_KEY; 8],
            coef_key: [NO_KEY; 3],
        }
    }

    fn sounding(&self) -> bool {
        matches!(self.state, State::On | State::Release)
    }
}

/// Smoothed parameters, stepped once per control step.
#[derive(Clone, Copy, Debug)]
struct Smoothed {
    lvl1: Smooth,
    lvl2: Smooth,
    sub: Smooth,
    noise: Smooth,
    pw1: Smooth,
    pw2: Smooth,
    /// Cutoff in octaves (log2 Hz).
    cutoff: Smooth,
    res: Smooth,
    drive: Smooth,
    dist: Smooth,
    out_l: Smooth,
    out_r: Smooth,
    bend: Smooth,
}

/// Control values shared by every voice for one control step.
#[derive(Clone, Copy, Debug, Default)]
struct Shared {
    lfo: f32,
    mode: u8,
    lvl1: f32,
    lvl2: f32,
    sub: f32,
    noise: f32,
    drive_gain: f32,
    drive_amt: f32,
    dist_gain: f32,
    dist_amt: f32,
    out_l: f32,
    out_r: f32,
    wave1: u8,
    wave2: u8,
    unison: usize,
    /// Each oscillator's highest saw against the note: the supersaw's top
    /// ratio on Supersaw, else 1 ([`osc::level_for`]).
    tops: [f32; 2],
}

pub struct Poly {
    rate: u32,
    values: [f32; ID_SPACE],
    /// The oscillators' tables, built before the first note.
    tables: &'static Classic,
    /// The supersaw's ratios and gains, from its two knobs.
    sup: Super,
    /// The velocity curve, from Velocity to level's range.
    vel: VelCurve,
    voices: [Voice; POOL],
    sm: Smoothed,
    sh: Shared,
    rng: Rng,
    lfo_phase: f64,
    lfo_held: f32,
    /// Notes held, oldest first, for mono and legato.
    stack: [u8; MAX_POLY],
    stack_len: usize,
    clock: u64,
    last_pitch: f32,
    /// Absolute frame of the next frame to process, for control steps.
    frame: u64,
    meter_voices: f32,
    /// The pressure shaper worked out for the control rate, and the slots.
    press_c: PressureCoefs,
    depths: Depths,
    chord: PressureChord,
    /// The pressure vibrato's phase (0 to 1) and value (-1 to 1).
    vib_phase: f32,
    vib: f32,
    /// Each stack's pan gains for the unison count and spread in `pan_key`.
    pans: [(f32, f32); UNISON_MAX],
    pan_key: [u32; 2],
    /// How notes become frequencies.
    tuning: Tuning,
    /// Each note's slide and tone for its next note-on, and the voices'
    /// smoothing coefficient (the controllers glide with it too).
    exprs: NoteExprs,
    expr_c: f32,
    /// The play routes; whether one always does something (from a note's
    /// own source, or on a swell, which dips at rest); which controllers
    /// (wheel, breath, foot) have a route that does something once they
    /// leave their rest; and whether any does something now, worked out
    /// each control step, so a sound whose routes rest costs what it did
    /// without them.
    routes: [Route; ROUTES],
    routes_always: bool,
    routes_ctl: [bool; 3],
    routes_live: bool,
    /// A pressure slot on a target only the routes had before (LFO 1 or 2
    /// amount, Pan).
    press_more: bool,
    /// Whether a route moves the LFO's rate.
    route_rate: bool,
    /// The mod wheel, breath and foot controller as they came, and as the
    /// routes read them (gliding over a few milliseconds, so a wheel's
    /// steps never zip).
    chan: Channel,
    chan_s: Channel,
    /// Each note's random value and a free LFO 2's starting place.
    route_rng: Rng,
    /// Whether LFO 2 sends anything, and the tempo its sync was set for.
    lfo2_on: bool,
    lfo2_bpm: f64,
    /// The song's position at this control step while it plays, for a
    /// synced LFO 2 that doesn't retrigger.
    step_beats: Option<f64>,
    /// Work buffers for one render pass (each frame written before read).
    scratch: Scratch,
    /// The output's DC blocker, and whether it runs: from when a voice
    /// with drive or distortion (whose asymmetry makes DC) sounds, until
    /// every voice has ended and the blocker has let go. A sound without
    /// them never runs it, so it plays and costs as it did.
    dc_l: DcBlock,
    dc_r: DcBlock,
    dc_live: bool,
    /// Whether pressure, or a route, can add drive or distortion.
    press_grit: bool,
    route_grit: bool,
    /// Notes that did not fit anywhere (should stay 0).
    pub dropped: u64,
}

#[derive(Clone, Copy, Debug)]
struct Scratch {
    gain: [f32; SEG],
    stack: [f32; SEG],
    left: [f32; SEG],
    right: [f32; SEG],
    extra: [f32; SEG],
    /// What the output held before the voices added to it, while the DC
    /// blocker runs.
    mix_l: [f32; SEG],
    mix_r: [f32; SEG],
}

impl Default for Poly {
    fn default() -> Self {
        Poly::new(48_000)
    }
}

impl Poly {
    pub fn new(rate: u32) -> Poly {
        let mut values = [0.0; ID_SPACE];
        for p in PARAMS {
            values[p.id as usize] = p.default;
        }
        let v = |id: u16| values[id as usize];
        let sm = Smoothed {
            lvl1: Smooth::new(v(OSC1_LEVEL) / 100.0),
            lvl2: Smooth::new(v(OSC2_LEVEL) / 100.0),
            sub: Smooth::new(v(SUB_LEVEL) / 100.0),
            noise: Smooth::new(v(NOISE_LEVEL) / 100.0),
            pw1: Smooth::new(v(OSC1_PW) / 100.0),
            pw2: Smooth::new(v(OSC2_PW) / 100.0),
            cutoff: Smooth::new(v(CUTOFF).log2()),
            res: Smooth::new(v(RESONANCE) / 100.0),
            drive: Smooth::new(v(DRIVE)),
            dist: Smooth::new(v(DIST)),
            out_l: Smooth::new(1.0),
            out_r: Smooth::new(1.0),
            bend: Smooth::new(0.0),
        };
        let mut s = Poly {
            rate,
            values,
            tables: osc::classic(),
            sup: Super::new(v(SUPER_DETUNE) / 100.0, v(SUPER_MIX) / 100.0),
            vel: VelCurve::new(v(VEL_AMP)),
            voices: std::array::from_fn(Voice::new),
            sm,
            sh: Shared::default(),
            rng: Rng::new(0x005E_ED0F_901F),
            lfo_phase: 0.0,
            lfo_held: 0.0,
            stack: [0; MAX_POLY],
            stack_len: 0,
            clock: 0,
            last_pitch: 60.0,
            frame: 0,
            meter_voices: 0.0,
            press_c: PressureCoefs::default(),
            depths: Depths::default(),
            chord: PressureChord::default(),
            vib_phase: 0.0,
            vib: 0.0,
            pans: [(1.0, 1.0); UNISON_MAX],
            pan_key: [NO_KEY; 2],
            tuning: Tuning::standard(),
            exprs: NoteExprs::new(),
            expr_c: 1.0,
            routes: [Route::default(); ROUTES],
            routes_always: false,
            routes_ctl: [false; 3],
            routes_live: false,
            press_more: false,
            route_rate: false,
            chan: Channel::default(),
            chan_s: Channel::default(),
            route_rng: Rng::new(0x0207_E5EE_D5A1),
            lfo2_on: false,
            lfo2_bpm: 120.0,
            step_beats: None,
            scratch: Scratch {
                gain: [0.0; SEG],
                stack: [0.0; SEG],
                left: [0.0; SEG],
                right: [0.0; SEG],
                extra: [0.0; SEG],
                mix_l: [0.0; SEG],
                mix_r: [0.0; SEG],
            },
            dc_l: DcBlock::new(rate, DC_HZ),
            dc_r: DcBlock::new(rate, DC_HZ),
            dc_live: false,
            press_grit: false,
            route_grit: false,
            dropped: 0,
        };
        s.prepare(rate, mpfree_node::MAX_BLOCK);
        s
    }

    /// Loads a factory preset: defaults, then the preset's values. Voices
    /// keep sounding; smoothed parameters glide to the new values.
    pub fn load_preset(&mut self, preset: &Preset) {
        for p in PARAMS {
            if !p.readonly {
                self.set_param(p.id, p.default);
            }
        }
        for &(id, v) in preset.values {
            self.set_param(id, v);
        }
    }

    pub fn presets() -> &'static [Preset] {
        crate::factory::presets!("poly.presets", &INFO, PARAMS)
    }

    /// Voices sounding or fading.
    pub fn active_voices(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.state != State::Free)
            .count()
    }

    /// The pressure each sounding voice uses now (0 to 1), in voice order,
    /// for the Synth tab's live bars. Returns how many it wrote.
    pub fn pressures(&self, out: &mut [f32]) -> usize {
        let mut n = 0;
        for v in self.voices.iter().filter(|v| v.sounding()) {
            if n == out.len() {
                break;
            }
            out[n] = v.pv;
            n += 1;
        }
        n
    }

    #[inline]
    fn v(&self, id: u16) -> f32 {
        self.values[id as usize]
    }

    /// Works out the pressure shaper and slots from the values.
    fn update_pressure(&mut self) {
        let shape = PressureShape::from_params(PRESS_BASE, |id| self.values[id as usize]);
        self.press_c = shape.coefs(self.ctrl_rate());
        let slots: [Slot; 4] = std::array::from_fn(|n| Slot {
            target: self.v(press_target(n)) as u8,
            depth: self.v(press_depth(n)) / 100.0,
        });
        // Off: pressure does nothing, and a level swell plays at the preset's
        // full level (not at its resting dip).
        self.depths = if shape.mode == PressureMode::Off {
            Depths::default()
        } else {
            Depths::from_slots(&slots)
        };
        self.press_more = [target::LFO1_AMOUNT, target::LFO2_AMOUNT, target::PAN]
            .iter()
            .any(|&t| self.depths.get(t) != 0.0);
        self.press_grit =
            self.depths.get(target::DRIVE) != 0.0 || self.depths.get(target::DIST) != 0.0;
    }

    /// Reads the play routes from the values.
    fn update_routes(&mut self) {
        let v = |n: usize, f: u16| self.values[usize::from(route_id(n, f))];
        self.routes = std::array::from_fn(|n| Route {
            source: Source::from_index(v(n, route::SOURCE)),
            target: v(n, route::TARGET) as u8,
            depth: v(n, route::DEPTH) / 100.0,
        });
        let live = |r: &Route| r.source != Source::Off && r.target != 0 && r.depth != 0.0;
        const CTLS: [Source; 3] = [Source::ModWheel, Source::Breath, Source::Foot];
        self.routes_always = self.routes.iter().any(|r| {
            let swell = matches!(
                r.target,
                target::LEVEL | target::LFO1_AMOUNT | target::LFO2_AMOUNT
            );
            live(r) && (swell || !CTLS.contains(&r.source))
        });
        self.routes_ctl = CTLS.map(|s| self.routes.iter().any(|r| live(r) && r.source == s));
        self.route_rate = self
            .routes
            .iter()
            .any(|r| live(r) && r.target == target::LFO_RATE);
        self.route_grit = self
            .routes
            .iter()
            .any(|r| live(r) && (r.target == target::DRIVE || r.target == target::DIST));
    }

    /// Sets every voice's LFO 2 from the values (and the tempo, when it is
    /// synced).
    fn update_lfo2(&mut self) {
        let shape = LfoShape::from_index(self.v(LFO2_SHAPE));
        let sync = (self.v(LFO2_SYNC) as usize).min(LFO_SYNC_BEATS.len() - 1);
        let hz = self.v(LFO2_RATE);
        // The lowest setting, 1 ms, is no fade.
        let fade = self.v(LFO2_FADE);
        let fade = if fade <= 1.0 { 0.0 } else { fade };
        let (cr, bpm) = (self.ctrl_rate(), self.lfo2_bpm);
        for v in &mut self.voices {
            if sync > 0 {
                v.lfo2
                    .set_synced(shape, LFO_SYNC_BEATS[sync] as f32, bpm, cr);
            } else {
                v.lfo2.set_free(shape, hz, cr);
            }
            v.lfo2.set_delay(0.0, fade, cr);
        }
        self.lfo2_on = [LFO2_PITCH, LFO2_CUTOFF, LFO2_PW, LFO2_AMP]
            .iter()
            .any(|&id| self.v(id) != 0.0);
    }

    /// A note starts on a voice: its LFO 2 goes back to its start and
    /// fades in again. One that doesn't retrigger starts somewhere of its
    /// own, or, synced while the song plays, where the song is.
    fn key_lfo2(v: &mut Voice, free: Option<bool>, rng: &mut Rng) {
        let start = match free {
            Some(false) => rng.unit(),
            _ => 0.0,
        };
        v.lfo2.set_retrigger(true, start);
        v.lfo2.key();
    }

    /// LFO 2's start on a new note: `None` when it retriggers, else
    /// whether it is synced (`Some(true)`: the song places it).
    fn lfo2_free(&self) -> Option<bool> {
        (self.v(LFO2_RETRIG) < 0.5).then(|| self.v(LFO2_SYNC) >= 1.0)
    }

    fn ctrl_rate(&self) -> f32 {
        self.rate as f32 / CTRL as f32
    }

    /// Pushes parameter targets into the smoothers.
    fn update_targets(&mut self) {
        let pan = self.v(PAN) / 100.0;
        let lvl = 10f32.powf(self.v(LEVEL) / 20.0);
        let lvl = if self.v(LEVEL) <= -60.0 { 0.0 } else { lvl };
        self.sm.lvl1.target = self.v(OSC1_LEVEL) / 100.0;
        self.sm.lvl2.target = self.v(OSC2_LEVEL) / 100.0;
        self.sm.sub.target = self.v(SUB_LEVEL) / 100.0;
        self.sm.noise.target = self.v(NOISE_LEVEL) / 100.0;
        self.sm.pw1.target = self.v(OSC1_PW) / 100.0;
        self.sm.pw2.target = self.v(OSC2_PW) / 100.0;
        self.sm.cutoff.target = self.v(CUTOFF).log2();
        self.sm.res.target = self.v(RESONANCE) / 100.0;
        self.sm.drive.target = self.v(DRIVE);
        self.sm.dist.target = self.v(DIST);
        self.sm.out_l.target = lvl * (1.0 - pan).sqrt();
        self.sm.out_r.target = lvl * (1.0 + pan).sqrt();
    }

    fn update_envs(&mut self) {
        let fr = self.rate as f32;
        let cr = self.ctrl_rate();
        let (aa, ad, as_, ar) = (
            self.v(AMP_A),
            self.v(AMP_D),
            self.v(AMP_S) / 100.0,
            self.v(AMP_R),
        );
        let (fa, fd, fs, frl) = (
            self.v(FENV_A),
            self.v(FENV_D),
            self.v(FENV_S) / 100.0,
            self.v(FENV_R),
        );
        for v in &mut self.voices {
            v.amp.set(aa, ad, as_, ar, fr);
            v.fenv.set(fa, fd, fs, frl, cr);
        }
    }

    fn reset_smoothers(&mut self) {
        self.update_targets();
        let cr = self.rate / CTRL as u32;
        let sm = &mut self.sm;
        for s in [
            &mut sm.lvl1,
            &mut sm.lvl2,
            &mut sm.sub,
            &mut sm.noise,
            &mut sm.pw1,
            &mut sm.pw2,
            &mut sm.res,
            &mut sm.drive,
            &mut sm.dist,
            &mut sm.out_l,
            &mut sm.out_r,
            &mut sm.cutoff,
        ] {
            s.set_time(cr, 15.0);
            s.snap(s.target);
        }
        sm.bend.set_time(cr, 4.0);
        self.expr_c = expr_coef(cr as f32);
    }

    // ---- notes ----

    fn note_on(&mut self, note: u8, velocity: u8) {
        if velocity == 0 {
            self.note_off(note);
            return;
        }
        let vel = f32::from(velocity.min(127)) / 127.0;
        self.clock += 1;
        if self.stack_len == 0 && self.v(LFO_RETRIG) >= 0.5 {
            self.lfo_phase = 0.0;
        }
        let mode = self.v(VOICE_MODE) as u8;
        // The note stack (also kept in poly mode, for retrigger on AllOff).
        self.stack_remove(note);
        if self.stack_len == MAX_POLY {
            self.stack.copy_within(1.., 0);
            self.stack_len -= 1;
        }
        let was_held = self.stack_len > 0;
        self.stack[self.stack_len] = note;
        self.stack_len += 1;

        if mode != 0 {
            // Mono and legato use slot 0 only.
            let glide = self.v(GLIDE) > 0.0;
            let legato = mode == 2 && was_held && self.voices[0].state == State::On;
            let from = if self.voices[0].sounding() {
                self.voices[0].pitch
            } else {
                self.last_pitch
            };
            let ex = self.exprs.on(note);
            let random = self.route_rng.bipolar();
            let free2 = self.lfo2_free();
            let v = &mut self.voices[0];
            let fresh = v.state == State::Free || v.state == State::Kill;
            if fresh {
                Self::fresh(v);
            }
            v.press.start();
            if legato {
                v.ex.retarget(ex);
                v.inp.key = Inputs::key_of(note);
            } else {
                v.ex.start(ex);
                v.inp = Inputs::at_start(velocity, note, random);
                Self::key_lfo2(v, free2, &mut self.route_rng);
            }
            v.note = note;
            v.target = f32::from(note);
            v.pitch = if glide && (mode == 1 || legato) {
                from
            } else {
                v.target
            };
            if !legato {
                v.vel = vel;
                v.amp.gate_on();
                v.fenv.gate_on();
            }
            v.state = State::On;
            v.age = self.clock;
            self.last_pitch = f32::from(note);
            self.voice_control(0);
            if fresh {
                Self::snap_gain(&mut self.voices[0]);
            }
            return;
        }

        let glide = self.v(GLIDE) > 0.0;
        let from = self.last_pitch;
        self.last_pitch = f32::from(note);
        // The same note sounding again reuses its voice.
        let slot = match self
            .voices
            .iter()
            .position(|v| v.sounding() && v.note == note)
        {
            Some(i) => i,
            None => {
                let poly = (self.v(POLYPHONY) as usize).clamp(1, MAX_POLY);
                let sounding = self.voices.iter().filter(|v| v.sounding()).count();
                if sounding >= poly {
                    self.steal();
                }
                match self.voices.iter().position(|v| v.state == State::Free) {
                    Some(i) => i,
                    None => {
                        // Every slot is fading: reuse the quietest.
                        let Some(i) = (0..POOL).min_by(|&a, &b| {
                            self.voices[a]
                                .kill_gain
                                .total_cmp(&self.voices[b].kill_gain)
                        }) else {
                            self.dropped += 1;
                            return;
                        };
                        i
                    }
                }
            }
        };
        let unison = (self.v(UNISON) as usize).clamp(1, UNISON_MAX);
        let ex = self.exprs.on(note);
        let random = self.route_rng.bipolar();
        let free2 = self.lfo2_free();
        let v = &mut self.voices[slot];
        let fresh = !v.sounding();
        if fresh {
            Self::fresh(v);
        }
        v.press.start();
        v.ex.start(ex);
        v.inp = Inputs::at_start(velocity, note, random);
        Self::key_lfo2(v, free2, &mut self.route_rng);
        v.note = note;
        v.vel = vel;
        v.target = f32::from(note);
        v.pitch = if glide { from } else { v.target };
        v.state = State::On;
        v.age = self.clock;
        v.amp.gate_on();
        v.fenv.gate_on();
        if unison > 1 {
            for u in 0..unison {
                v.ph1[u] = osc::phase(self.rng.unit());
                v.ph2[u] = osc::phase(self.rng.unit());
            }
        }
        // The supersaw's saws run free, as on the original: each starts
        // somewhere of its own, so a chord doesn't open on one big edge.
        let (w1, w2) = (
            self.values[OSC1_WAVE as usize],
            self.values[OSC2_WAVE as usize],
        );
        for (wave, ss) in [(w1, &mut v.ss1), (w2, &mut v.ss2)] {
            if Shape::from_index(wave) == Shape::Supersaw {
                for p in ss[..unison].iter_mut().flatten() {
                    *p = osc::phase(self.rng.unit());
                }
            }
        }
        self.voice_control(slot);
        if fresh {
            Self::snap_gain(&mut self.voices[slot]);
        }
    }

    /// A fresh voice starts at its gain instead of ramping from the last
    /// note's.
    fn snap_gain(v: &mut Voice) {
        v.gain += v.gain_inc * CTRL as f32;
        v.gain_inc = 0.0;
    }

    /// Clears a slot's state for a new note.
    fn fresh(v: &mut Voice) {
        v.ph1 = [0; UNISON_MAX];
        v.ph2 = [0; UNISON_MAX];
        v.ph_sub = 0.0;
        v.amp.clear();
        v.fenv.clear();
        v.filt = [Filter::default(); 2];
        v.press.clear();
        v.pv = 0.0;
        v.kill_gain = 1.0;
        v.drive_level.clear();
        v.dist_level.clear();
        v.drive_aa = [DriveAa::default(); 2];
        v.dist_aa = [DriveAa::default(); 2];
    }

    /// Fades out one sounding voice: the quietest releasing one, else the
    /// oldest held one.
    fn steal(&mut self) {
        let rel = (0..POOL)
            .filter(|&i| self.voices[i].state == State::Release)
            .min_by(|&a, &b| {
                self.voices[a]
                    .amp
                    .level
                    .total_cmp(&self.voices[b].amp.level)
            });
        let victim = rel.or_else(|| {
            (0..POOL)
                .filter(|&i| self.voices[i].state == State::On)
                .min_by_key(|&i| self.voices[i].age)
        });
        if let Some(i) = victim {
            self.fade_out(i);
        }
    }

    /// Fades voice `i` out over [`KILL_MS`], then frees it: a steal, or
    /// a voice given up under load.
    fn fade_out(&mut self, i: usize) {
        let n = (KILL_MS * 0.001 * self.rate as f32).max(1.0);
        let v = &mut self.voices[i];
        v.state = State::Kill;
        v.kill_gain = 1.0;
        v.kill_step = 1.0 / n;
    }

    /// The voice load shedding gives up first: the least audible one
    /// sounding ([`Cull::audibility`]). Voices already fading are left be.
    fn cull_pick(&self) -> Option<(usize, Cull)> {
        let unison = (self.v(UNISON) as usize).clamp(1, UNISON_MAX);
        let norm = VOICE_GAIN / (unison as f32).sqrt();
        let mut best = None;
        for i in 0..POOL {
            let releasing = match self.voices[i].state {
                State::On => false,
                State::Release => true,
                State::Free | State::Kill => continue,
            };
            let level = self.voices[i].amp.level * self.voices[i].gain / norm;
            best = Cull::first(best, Some((i, Cull { level, releasing })));
        }
        best
    }

    fn stack_remove(&mut self, note: u8) {
        if let Some(i) = self.stack[..self.stack_len].iter().position(|&n| n == note) {
            self.stack.copy_within(i + 1..self.stack_len, i);
            self.stack_len -= 1;
        }
    }

    fn note_off(&mut self, note: u8) {
        self.exprs.off(note);
        self.stack_remove(note);
        let mode = self.v(VOICE_MODE) as u8;
        let glide = self.v(GLIDE) > 0.0;
        if mode != 0 {
            let v = &mut self.voices[0];
            if v.state != State::On || v.note != note {
                return;
            }
            if self.stack_len > 0 {
                // Back to the newest note still held, without a retrigger.
                let back = self.stack[self.stack_len - 1];
                v.ex.retarget(self.exprs.on(back));
                v.inp.key = Inputs::key_of(back);
                v.note = back;
                v.target = f32::from(back);
                if !glide {
                    v.pitch = v.target;
                }
                self.last_pitch = v.target;
            } else {
                Self::release(v);
            }
            return;
        }
        for v in &mut self.voices {
            if v.state == State::On && v.note == note {
                Self::release(v);
            }
        }
    }

    fn release(v: &mut Voice) {
        v.state = State::Release;
        v.amp.gate_off();
        v.fenv.gate_off();
        v.press.release();
    }

    fn all_off(&mut self) {
        self.stack_len = 0;
        self.exprs.clear();
        for v in &mut self.voices {
            if v.state == State::On {
                Self::release(v);
            }
        }
    }

    fn pressure(&mut self, note: u8, value: u8) {
        for v in &mut self.voices {
            if v.sounding() && v.note == note {
                v.press.set(value);
            }
        }
    }

    /// One note's own slide or tone: its held voices glide to it.
    fn expression(&mut self, note: u8, kind: Expr, value: f32) {
        let Some(value) = self.exprs.set(note, kind, value) else {
            return;
        };
        for v in &mut self.voices {
            if v.state == State::On && v.note == note {
                v.ex.aim(kind, value);
            }
        }
    }

    fn apply(&mut self, ev: &NodeEvent) {
        match *ev {
            NodeEvent::NoteOn { note, velocity } => self.note_on(note, velocity),
            NodeEvent::NoteOff { note } => self.note_off(note),
            NodeEvent::Pressure { note, value } => self.pressure(note, value),
            NodeEvent::Bend { value } => {
                self.sm.bend.target = if value.is_finite() {
                    value.clamp(-1.0, 1.0)
                } else {
                    0.0
                }
            }
            NodeEvent::AllOff => self.all_off(),
            NodeEvent::Param { id, value } => self.set_param(id, value),
            NodeEvent::Expression { note, kind, value } => self.expression(note, kind, value),
            // The mod wheel, breath and foot controller reach the routes.
            // The pedals are held by the host for this engine
            // (`Node::hears`), so the channel ignores them.
            NodeEvent::Control { ctl, value } => self.chan.set(ctl, value),
        }
    }

    // ---- control rate ----

    /// One control step: smoothers, the LFO, then every voice.
    fn control(&mut self, ctx: &Ctx, offset: usize) {
        let sm = &mut self.sm;
        let s = &mut self.sh;
        s.lvl1 = sm.lvl1.next();
        s.lvl2 = sm.lvl2.next();
        s.sub = sm.sub.next();
        s.noise = sm.noise.next();
        sm.pw1.next();
        sm.pw2.next();
        sm.cutoff.next();
        sm.res.next();
        let drive = sm.drive.next();
        let dist = sm.dist.next();
        s.out_l = sm.out_l.next();
        s.out_r = sm.out_r.next();
        sm.bend.next();
        s.drive_gain = 10f32.powf(drive / 20.0);
        s.drive_amt = (drive / 3.0).min(1.0);
        // Most sounds have none: no power worked out each step for it.
        (s.dist_gain, s.dist_amt) = if dist > 0.0 {
            (10f32.powf(dist / 20.0), (dist / 3.0).min(1.0))
        } else {
            (1.0, 0.0)
        };
        s.mode = self.values[FILTER_MODE as usize] as u8;
        s.wave1 = self.values[OSC1_WAVE as usize] as u8;
        s.wave2 = self.values[OSC2_WAVE as usize] as u8;
        let top = self.sup.top();
        s.tops = [s.wave1, s.wave2].map(|w| {
            if Shape::from_index(f32::from(w)) == Shape::Supersaw {
                top
            } else {
                1.0
            }
        });
        s.unison = (self.values[UNISON as usize] as usize).clamp(1, UNISON_MAX);

        // Pressure: the chord (Highest mode) aims before the voices step.
        let c = &self.press_c;
        let chord = if c.mode == PressureMode::Highest {
            self.chord.step(
                c,
                self.voices
                    .iter()
                    .filter(|v| v.sounding())
                    .map(|v| (v.state == State::On, v.press.aim(c))),
            )
        } else {
            0.0
        };
        let mut top = 0.0f32;
        for v in self.voices.iter_mut().filter(|v| v.state != State::Free) {
            let own = v.press.step(c);
            v.pv = match c.mode {
                PressureMode::PerNote => own,
                PressureMode::Highest => chord,
                PressureMode::Off => 0.0,
            };
            if v.sounding() {
                top = top.max(v.pv);
            }
        }
        let vib_hz = self.values[PRESS_VIB_RATE as usize];
        self.vib_phase = (self.vib_phase + vib_hz * CTRL as f32 / self.rate as f32).fract();
        self.vib = (std::f32::consts::TAU * self.vib_phase).sin();

        // The controllers glide to where they were sent.
        if self.chan_s != self.chan {
            let c = self.expr_c;
            let (to, at) = (self.chan, &mut self.chan_s);
            for (x, t) in [
                (&mut at.mod_wheel, to.mod_wheel),
                (&mut at.breath, to.breath),
                (&mut at.foot, to.foot),
            ] {
                *x += (t - *x) * c;
                if (t - *x).abs() < 1e-5 {
                    *x = t;
                }
            }
        }
        // Whether a route does anything this step: one from a note's own
        // source always does; one from a controller only while it is off
        // its rest, or on a swell (which dips at rest).
        let (c, ctl) = (self.chan_s, self.routes_ctl);
        self.routes_live = self.routes_always
            || (ctl[0] && c.mod_wheel != 0.0)
            || (ctl[1] && c.breath != 0.0)
            || (ctl[2] && c.foot != 0.0);

        // The song's position while it plays, for the synced LFOs; a new
        // tempo resets LFO 2's synced rate.
        self.step_beats = ctx
            .playing
            .then(|| ctx.beats + offset as f64 * ctx.bpm / 60.0 / f64::from(ctx.rate));
        if self.lfo2_on && ctx.bpm != self.lfo2_bpm {
            self.lfo2_bpm = ctx.bpm;
            if self.v(LFO2_SYNC) >= 1.0 {
                self.update_lfo2();
            }
        }

        // LFO: free-running, or locked to the transport's beat. Pressure on
        // LFO rate follows the highest pressure (the LFO is shared), and a
        // route from a note's own source follows the highest note's.
        let shape = LfoShape::from_index(self.v(LFO_SHAPE));
        let sync = (self.v(LFO_SYNC) as usize).min(LFO_SYNC_BEATS.len() - 1);
        let prev = self.lfo_phase;
        if let (true, Some(beats)) = (sync > 0, self.step_beats) {
            self.lfo_phase = (beats / LFO_SYNC_BEATS[sync]).rem_euclid(1.0);
        } else {
            let hz = if sync > 0 {
                ctx.bpm / 60.0 / LFO_SYNC_BEATS[sync]
            } else {
                f64::from(self.v(LFO_RATE))
            };
            let mut oct = self.depths.get(target::LFO_RATE) * 2.0 * top;
            if self.route_rate {
                oct += self.rate_route() * 2.0;
            }
            let hz = hz * f64::from(exp2(oct));
            self.lfo_phase =
                (self.lfo_phase + hz * CTRL as f64 / f64::from(self.rate)).rem_euclid(1.0);
        }
        if self.lfo_phase < prev {
            self.lfo_held = self.rng.bipolar();
        }
        self.sh.lfo = lfo_value(shape, self.lfo_phase as f32, self.lfo_held);

        for i in 0..POOL {
            if self.voices[i].state != State::Free {
                self.voice_control(i);
            }
        }

        // The DC blocker: on while a voice sounds with drive or distortion
        // (set, or added now by pressure or a route), off once every voice
        // has ended and it has let go. A sound without them only pays for
        // this test.
        let driven = self.sh.drive_amt > 0.0
            || self.sh.dist_amt > 0.0
            || (self.press_grit && top > 0.0)
            || (self.route_grit && self.routes_live);
        if driven || self.dc_live {
            // It lets go of what is left of its state here, at fixed frames,
            // so the output is the same however blocks are cut.
            self.dc_l.settle();
            self.dc_r.settle();
            let any = self.voices.iter().any(|v| v.state != State::Free);
            if driven && any {
                self.dc_live = true;
            } else if !any && self.dc_l.rests() && self.dc_r.rests() {
                self.dc_live = false;
            }
        }
    }

    /// The routes' amount on the LFO's rate (-1 to 1, 2 octaves at 1):
    /// the highest of the sounding notes', or the controllers' alone when
    /// none sounds.
    fn rate_route(&self) -> f32 {
        let (routes, c) = (&self.routes, &self.chan_s);
        self.voices
            .iter()
            .filter(|v| v.sounding())
            .map(|v| routed(routes, target::LFO_RATE, &v.inp, c))
            .reduce(f32::max)
            .unwrap_or_else(|| routed(routes, target::LFO_RATE, &Inputs::default(), c))
    }

    /// Pitch, filter and gains for one voice from the current control values.
    fn voice_control(&mut self, i: usize) {
        if self.routes_live || self.press_more || self.lfo2_on {
            self.voice_control_as::<true>(i);
        } else {
            self.voice_control_as::<false>(i);
        }
    }

    /// `voice_control`, with (`MORE`) or without the play routes, LFO 2 and
    /// pressure's newer targets: without them it is Poly's plain path, and
    /// costs what it did before they came.
    fn voice_control_as<const MORE: bool>(&mut self, i: usize) {
        let rate = self.rate as f32;
        let lfo = self.sh.lfo;
        let unison = (self.values[UNISON as usize] as usize).clamp(1, UNISON_MAX);
        let detune = self.v(UNISON_DETUNE) / 100.0;
        let spread = self.v(UNISON_SPREAD) / 100.0;
        let bend = self.sm.bend.value * self.v(BEND_RANGE);
        let glide_ms = self.v(GLIDE);
        let glide_c = if glide_ms > 0.0 {
            seg_coef(glide_ms, self.ctrl_rate())
        } else {
            1.0
        };
        let o1 = self.v(OSC1_OCT) * 12.0;
        let o2 = self.v(OSC2_OCT) * 12.0 + self.v(OSC2_SEMI) + self.v(OSC2_FINE) / 100.0;
        let vib = lfo * self.v(LFO_PITCH);
        let pwm = lfo * self.v(LFO_PW) / 100.0;
        let pw1 = (self.sm.pw1.value + pwm).clamp(0.05, 0.95);
        let pw2 = (self.sm.pw2.value + pwm).clamp(0.05, 0.95);
        let fenv_amt = self.v(FENV_AMOUNT);
        let key = self.v(KEY_TRACK) / 100.0;
        let vel_cut = self.v(VEL_CUTOFF);
        let lfo_cut = lfo * self.v(LFO_CUTOFF);
        let d = self.depths;
        let vib_p = self.vib;
        let drive_db = self.sm.drive.value;
        let dist_db = self.sm.dist.value;
        let (sh_drive_gain, sh_drive_amt, sh_sub, sh_noise) = (
            self.sh.drive_gain,
            self.sh.drive_amt,
            self.sh.sub,
            self.sh.noise,
        );
        let (sh_dist_gain, sh_dist_amt) = (self.sh.dist_gain, self.sh.dist_amt);
        let vel = self.vel;
        let trem = 1.0 - self.v(LFO_AMP) / 100.0 * (0.5 - 0.5 * lfo);
        // The routes, LFO 2 and pressure's newer targets, only while one of
        // them does something.
        let more_in = MORE.then(|| MoreIn {
            routes: self.routes,
            routes_live: self.routes_live,
            chan: self.chan_s,
            depths: self.depths,
            lfo,
            vib: self.vib,
            lfo1: [
                self.v(LFO_PITCH),
                self.v(LFO_PW) / 100.0,
                self.v(LFO_CUTOFF),
                self.v(LFO_AMP) / 100.0,
            ],
            lfo2: self.lfo2_on.then(|| {
                [
                    self.v(LFO2_PITCH),
                    self.v(LFO2_PW) / 100.0,
                    self.v(LFO2_CUTOFF),
                    self.v(LFO2_AMP) / 100.0,
                ]
            }),
            follow2: if self.v(LFO2_SYNC) >= 1.0 && self.v(LFO2_RETRIG) < 0.5 {
                self.step_beats
            } else {
                None
            },
        });
        let cutoff = self.sm.cutoff.value;
        let res = self.sm.res.value;
        let norm = VOICE_GAIN / (unison as f32).sqrt();
        let rate_bits = self.rate;
        // Pan gains depend only on the stack count and the spread.
        let pan_key = [unison as u32, spread.to_bits()];
        if !same_key(&self.pan_key, &pan_key) {
            for u in 0..unison {
                let pos = if unison > 1 {
                    u as f32 / (unison - 1) as f32 * 2.0 - 1.0
                } else {
                    0.0
                };
                let p = pos * spread;
                self.pans[u] = ((1.0 - p).sqrt(), (1.0 + p).sqrt());
            }
            self.pan_key = pan_key;
        }
        let pans = self.pans;
        let expr_c = self.expr_c;
        let tuning = &self.tuning;
        // A supersaw's level is set by its highest saw.
        let tops = self.sh.tops;

        let v = &mut self.voices[i];
        let p = v.pv;
        v.pitch += (v.target - v.pitch) * glide_c;
        if (v.target - v.pitch).abs() < 1e-4 {
            v.pitch = v.target;
        }
        v.ex.step(expr_c);
        let m = if MORE {
            more_in.as_ref().map(|mi| more(v, mi))
        } else {
            None
        };
        let mut base = v.pitch + bend + vib;
        if v.ex.slide != 0.0 {
            base += v.ex.slide;
        }
        if d.any {
            base += p * (d.get(target::PITCH) * 12.0 + d.get(target::VIBRATO) * vib_p);
        }
        if let Some(m) = &m {
            base += m.pitch;
        }
        let pitch_key = [
            base.to_bits(),
            o1.to_bits(),
            o2.to_bits(),
            detune.to_bits(),
            unison as u32,
            rate_bits,
            tops[0].to_bits(),
            tops[1].to_bits(),
        ];
        if !same_key(&v.pitch_key, &pitch_key) {
            for u in 0..unison {
                let pos = if unison > 1 {
                    u as f32 / (unison - 1) as f32 * 2.0 - 1.0
                } else {
                    0.0
                };
                let cents = pos * detune;
                v.dt1[u] = (tuning.hz(base + o1 + cents) / rate).min(0.45);
                v.dt2[u] = (tuning.hz(base + o2 - cents * 0.7) / rate).min(0.45);
                v.di1[u] = osc::increment(v.dt1[u]);
                v.di2[u] = osc::increment(v.dt2[u]);
                v.lv1[u] = osc::level_near(usize::from(v.lv1[u]), v.dt1[u] * tops[0]) as u8;
                v.lv2[u] = osc::level_near(usize::from(v.lv2[u]), v.dt2[u] * tops[1]) as u8;
            }
            v.dt_sub = (tuning.hz(base + o1 - 12.0) / rate).min(0.45);
            v.pitch_key = pitch_key;
        }
        match &m {
            // A route or pressure on Pan moves the whole voice, its unison
            // copies with it.
            Some(m) if m.pan != 0.0 => {
                for u in 0..unison {
                    let pos = if unison > 1 {
                        u as f32 / (unison - 1) as f32 * 2.0 - 1.0
                    } else {
                        0.0
                    };
                    let q = (pos * spread + m.pan).clamp(-1.0, 1.0);
                    v.pan_l[u] = (1.0 - q).sqrt();
                    v.pan_r[u] = (1.0 + q).sqrt();
                }
            }
            _ => {
                for (u, &(l, r)) in pans[..unison].iter().enumerate() {
                    v.pan_l[u] = l;
                    v.pan_r[u] = r;
                }
            }
        }
        let mut pw_p = p * d.get(target::WIDTH) * 0.45;
        if let Some(m) = &m {
            pw_p += m.width;
        }
        v.pw1 = (pw1 + pw_p).clamp(0.05, 0.95);
        v.pw2 = (pw2 + pw_p).clamp(0.05, 0.95);
        let fe = v.fenv.next();
        let mut oct = cutoff
            + fe * fenv_amt
            + (f32::from(v.note) - 60.0) / 12.0 * key
            + v.vel * vel_cut
            + lfo_cut
            + p * d.get(target::CUTOFF) * 4.0;
        let mut res = res + p * d.get(target::RESONANCE);
        if let Some(m) = &m {
            oct += m.oct;
            res += m.res;
        }
        let res = res.clamp(0.0, 1.0);
        let coef_key = [oct.to_bits(), res.to_bits(), rate_bits];
        if !same_key(&v.coef_key, &coef_key) {
            v.coefs = FilterCoefs::new(exp2(oct), res, rate);
            v.coef_key = coef_key;
        }
        // Drive and distortion: shared unless pressure or a route moves
        // them on this voice.
        let (md, me) = m.as_ref().map_or((0.0, 0.0), |m| (m.drive, m.dist));
        let dd = d.get(target::DRIVE);
        if dd == 0.0 && md == 0.0 {
            v.drive_gain = sh_drive_gain;
            v.drive_amt = sh_drive_amt;
        } else {
            let mut db = drive_db + p * dd * 24.0;
            if md != 0.0 {
                db += md * 24.0;
            }
            let db = db.clamp(0.0, 48.0);
            v.drive_gain = exp2(db * (1.0 / 6.020_6));
            v.drive_amt = (db / 3.0).min(1.0);
        }
        let de = d.get(target::DIST);
        let dist_was = v.dist_amt;
        if de == 0.0 && me == 0.0 {
            v.dist_gain = sh_dist_gain;
            v.dist_amt = sh_dist_amt;
        } else {
            let mut db = dist_db + p * de * 24.0;
            if me != 0.0 {
                db += me * 24.0;
            }
            let db = db.clamp(0.0, 48.0);
            v.dist_gain = exp2(db * (1.0 / 6.020_6));
            v.dist_amt = (db / 3.0).min(1.0);
        }
        let mut sub = sh_sub + p * d.get(target::SUB);
        let mut noise = sh_noise + p * d.get(target::NOISE);
        if let Some(m) = &m {
            sub += m.sub;
            noise += m.noise;
        }
        v.sub = sub.clamp(0.0, 1.0);
        v.noise = noise.clamp(0.0, 1.0);
        let vg = vel.gain(v.vel * 127.0);
        let swell = swell_gain(d.get(target::LEVEL).clamp(-1.0, 1.0), p);
        // The drive's and the distortion's level corrections, from the
        // step just rendered.
        let mut level = if v.drive_amt > 0.0 {
            v.drive_level.step(CTRL, rate_bits)
        } else {
            v.drive_level.clear();
            v.drive_aa = [DriveAa::default(); 2];
            1.0
        };
        if v.dist_amt > 0.0 {
            level *= v.dist_level.step(CTRL, rate_bits);
        } else if dist_was > 0.0 {
            // Off: nothing more is summed, so one clear does.
            v.dist_level.clear();
            v.dist_aa = [DriveAa::default(); 2];
        }
        let gain = match &m {
            Some(m) => norm * vg * m.trem * swell * level * m.gain,
            None => norm * vg * trem * swell * level,
        };
        v.gain_inc = (gain - v.gain) / CTRL as f32;
    }

    // ---- audio rate ----

    /// Renders frames `from..to` (at most one control step) of every voice.
    /// Each voice works in passes over the frames: gains (which also find
    /// the frame where a voice that ends here stops), oscillators, sub and
    /// noise, drive, filter, distortion, out. Every frame gets the same arithmetic as a
    /// frame-at-a-time loop, so the output is the same to the bit; the
    /// passes just make each choice once per step instead of once a frame.
    fn render(&mut self, from: usize, to: usize, left: &mut [f32], right: &mut [f32]) {
        let s = self.sh;
        let w1 = Shape::from_index(f32::from(s.wave1));
        let w2 = Shape::from_index(f32::from(s.wave2));
        let (tab, sup) = (self.tables, &self.sup);
        let mode = FilterMode::from_index(f32::from(s.mode));
        // Silent sources are skipped (their phases wait).
        let (use1, use2) = (s.lvl1 != 0.0, s.lvl2 != 0.0);
        let n = to - from;
        debug_assert!(n <= SEG);
        let rng = &mut self.rng;
        let Scratch {
            gain: g,
            stack: a,
            left: l,
            right: r,
            extra: c,
            mix_l: ml,
            mix_r: mr,
        } = &mut self.scratch;
        // While the DC blocker runs, what the buffers held before is kept
        // aside, so it takes Poly's sound and nothing else.
        let dc = self.dc_live;
        if dc {
            ml[..n].copy_from_slice(&left[from..to]);
            mr[..n].copy_from_slice(&right[from..to]);
        }
        for v in &mut self.voices {
            if v.state == State::Free {
                continue;
            }
            // Gains: the envelope and fades do not depend on the sound.
            let mut end = n;
            for (j, gj) in g[..n].iter_mut().enumerate() {
                let mut x = v.amp.next() * v.gain;
                v.gain += v.gain_inc;
                if v.state == State::Kill {
                    v.kill_gain -= v.kill_step;
                    if v.kill_gain <= 0.0 {
                        v.kill_gain = 0.0;
                        v.state = State::Free;
                    }
                    x *= v.kill_gain;
                }
                *gj = x;
                if v.state == State::Free {
                    end = j + 1;
                    break;
                }
                if v.amp.idle() && v.state != State::On {
                    v.state = State::Free;
                    end = j + 1;
                    break;
                }
            }
            let (g, a, l, r, c) = (
                &g[..end],
                &mut a[..end],
                &mut l[..end],
                &mut r[..end],
                &mut c[..end],
            );
            // A centred voice (one stack, or no spread) has the same left
            // and right; with the same filter state both sides stay equal,
            // so one side is worked out and used for both.
            let mono = v.pan_l[..s.unison]
                .iter()
                .zip(&v.pan_r[..s.unison])
                .all(|(x, y)| x.to_bits() == y.to_bits())
                && v.filt[0].same_as(&v.filt[1])
                && v.drive_aa[0].same_as(&v.drive_aa[1])
                && v.dist_aa[0].same_as(&v.dist_aa[1]);
            // Each stack: a = 0 + osc1 * level1 + osc2 * level2, then
            // left = 0 + a * pan for the first stack and += for the rest.
            // (Left starts at +0 and never becomes -0, so adding a silent
            // source's +0 changes nothing and is skipped.)
            if !use1 && !use2 {
                l.fill(0.0);
                r.fill(0.0);
            }
            for u in 0..s.unison {
                if !use1 && !use2 {
                    break;
                }
                let (l1, l2) = (usize::from(v.lv1[u]), usize::from(v.lv2[u]));
                if use1 {
                    if w1 == Shape::Supersaw {
                        tab.supersaw::<false>(sup, l1, a, &mut v.ss1[u], v.dt1[u], s.lvl1);
                    } else {
                        tab.run::<false>(w1, l1, a, &mut v.ph1[u], v.di1[u], v.pw1, s.lvl1);
                    }
                }
                if use2 {
                    let add = use1;
                    match (w2 == Shape::Supersaw, add) {
                        (true, true) => {
                            tab.supersaw::<true>(sup, l2, a, &mut v.ss2[u], v.dt2[u], s.lvl2)
                        }
                        (true, false) => {
                            tab.supersaw::<false>(sup, l2, a, &mut v.ss2[u], v.dt2[u], s.lvl2)
                        }
                        (false, true) => {
                            tab.run::<true>(w2, l2, a, &mut v.ph2[u], v.di2[u], v.pw2, s.lvl2)
                        }
                        (false, false) => {
                            tab.run::<false>(w2, l2, a, &mut v.ph2[u], v.di2[u], v.pw2, s.lvl2)
                        }
                    }
                }
                let (pl, pr) = (v.pan_l[u], v.pan_r[u]);
                mix_stack(u == 0, l, a, pl);
                if !mono {
                    mix_stack(u == 0, r, a, pr);
                }
            }
            // Sub and noise: c = 0 + sub + noise, added to both sides.
            let use_sub = v.sub != 0.0;
            let use_noise = v.noise != 0.0;
            if use_sub {
                osc_set(Wave::Pulse, c, &mut v.ph_sub, v.dt_sub, 0.5, v.sub);
            }
            if use_noise {
                for cj in c.iter_mut() {
                    *cj = if use_sub { *cj } else { 0.0 } + rng.bipolar() * v.noise;
                }
            }
            let use_c = use_sub || use_noise;
            if use_c {
                for (lj, cj) in l.iter_mut().zip(c.iter()) {
                    *lj += cj;
                }
            }
            // The drive sees the mix as one full oscillator, however many
            // stacks and sources feed it (dsp::drive_scales), and so does
            // the distortion. The drive goes into the filter, as on an
            // analog synth; the distortion comes after it, so a low cutoff
            // can't take its grit out (Jay after live test 3: "add both").
            // Most sounds have no distortion: one test a step skips it.
            let (dk, dn) = if v.drive_amt > 0.0 {
                let lv = mix_level(s.unison, [s.lvl1, s.lvl2], v.sub, v.noise);
                drive_scales(v.drive_gain, lv)
            } else {
                (1.0, 1.0)
            };
            drive_run_level_aa(
                l,
                dk,
                dn,
                v.drive_amt,
                &mut v.drive_level,
                &mut v.drive_aa[0],
            );
            v.filt[0].run(mode, &v.coefs, l);
            let dist = if v.dist_amt > 0.0 {
                let lv = mix_level(s.unison, [s.lvl1, s.lvl2], v.sub, v.noise);
                let (ek, en) = drive_scales(v.dist_gain, lv);
                drive_run_level_aa(l, ek, en, v.dist_amt, &mut v.dist_level, &mut v.dist_aa[0]);
                Some((ek, en))
            } else {
                None
            };
            if mono {
                v.filt[1] = v.filt[0];
                v.drive_aa[1] = v.drive_aa[0];
                v.dist_aa[1] = v.dist_aa[0];
                for j in 0..end {
                    left[from + j] += l[j] * g[j] * s.out_l;
                    right[from + j] += l[j] * g[j] * s.out_r;
                }
            } else {
                if use_c {
                    for (rj, cj) in r.iter_mut().zip(c.iter()) {
                        *rj += cj;
                    }
                }
                drive_run_aa(r, dk, dn, v.drive_amt, &mut v.drive_aa[1]);
                v.filt[1].run(mode, &v.coefs, r);
                if let Some((ek, en)) = dist {
                    drive_run_aa(r, ek, en, v.dist_amt, &mut v.dist_aa[1]);
                }
                for j in 0..end {
                    left[from + j] += l[j] * g[j] * s.out_l;
                    right[from + j] += r[j] * g[j] * s.out_r;
                }
            }
        }
        if dc {
            self.dc_l.run_added(&mut left[from..to], &ml[..n]);
            self.dc_r.run_added(&mut right[from..to], &mr[..n]);
        }
    }
}

/// What the play routes, LFO 2 and pressure's newer targets (LFO 1 and 2
/// amount, Pan) do to one voice this control step, beside what Poly did
/// before them.
#[derive(Clone, Copy, Debug)]
struct More {
    /// Semitones on the pitch.
    pitch: f32,
    /// On both pulse widths (0 to 1).
    width: f32,
    /// Octaves on the cutoff.
    oct: f32,
    res: f32,
    /// -1 to 1, 24 dB at 1, on the drive and the distortion.
    drive: f32,
    dist: f32,
    sub: f32,
    noise: f32,
    /// The voice's place across the speakers, -1 to 1.
    pan: f32,
    /// LFO 1's tremolo through its amount (in place of the plain one).
    trem: f32,
    /// A gain: the routes' swells and LFO 2's tremolo.
    gain: f32,
}

/// What [`more`] reads that every voice shares.
#[derive(Clone, Copy, Debug)]
struct MoreIn {
    routes: [Route; ROUTES],
    /// Whether any route does something now.
    routes_live: bool,
    chan: Channel,
    depths: Depths,
    /// LFO 1's value and the pressure vibrato's, -1 to 1.
    lfo: f32,
    vib: f32,
    /// LFO 1's and LFO 2's sends: pitch (semitones), width (0 to 1),
    /// cutoff (octaves), level (0 to 1). LFO 2's only while it sends.
    lfo1: [f32; 4],
    lfo2: Option<[f32; 4]>,
    /// The song's position, for a synced LFO 2 that doesn't retrigger.
    follow2: Option<f64>,
}

/// One voice's [`More`]: the routes' amounts on each target (their
/// swells as gains), each LFO through its amount, and LFO 2's sends.
fn more(v: &mut Voice, m: &MoreIn) -> More {
    let p = v.pv;
    let mut ra = [0.0f32; 16];
    let (mut level, mut gate) = (1.0f32, [1.0f32; 2]);
    if m.routes_live {
        v.inp.pressure = p;
        v.inp.tone = v.ex.tone;
        for r in m.routes.iter().filter(|r| r.source != Source::Off) {
            match r.target {
                target::OFF => {}
                target::LEVEL => {
                    level *= swell_gain(r.depth, r.source.read_unit(&v.inp, &m.chan));
                }
                target::LFO1_AMOUNT | target::LFO2_AMOUNT => {
                    let k = usize::from(r.target - target::LFO1_AMOUNT);
                    gate[k] *= lfo_gate(r.depth, r.source.read_unit(&v.inp, &m.chan));
                }
                t => {
                    if let Some(x) = ra.get_mut(usize::from(t)) {
                        *x += r.amount(&v.inp, &m.chan);
                    }
                }
            }
        }
    }
    let d = &m.depths;
    let at = |t: u8| ra[usize::from(t)];
    let g1 = lfo_gate(d.get(target::LFO1_AMOUNT), p) * gate[0];
    let g2 = lfo_gate(d.get(target::LFO2_AMOUNT), p) * gate[1];
    // The plain path adds all of LFO 1; this takes back what its amount
    // holds back.
    let [l1_pitch, l1_width, l1_cut, l1_level] = m.lfo1;
    let held = (g1 - 1.0) * m.lfo;
    let mut out = More {
        pitch: at(target::PITCH) * 12.0 + at(target::VIBRATO) * m.vib + held * l1_pitch,
        width: at(target::WIDTH) * 0.45 + held * l1_width,
        oct: at(target::CUTOFF) * 4.0 + held * l1_cut,
        res: at(target::RESONANCE),
        drive: at(target::DRIVE),
        dist: at(target::DIST),
        sub: at(target::SUB),
        noise: at(target::NOISE),
        pan: (p * d.get(target::PAN) + at(target::PAN)).clamp(-1.0, 1.0),
        trem: 1.0 - l1_level * (0.5 - 0.5 * m.lfo) * g1,
        gain: level,
    };
    if let Some([to_pitch, to_width, to_cut, to_level]) = m.lfo2 {
        if let Some(b) = m.follow2 {
            v.lfo2.follow(b);
        }
        let x = v.lfo2.next() * g2;
        out.pitch += x * to_pitch;
        out.width += x * to_width;
        out.oct += x * to_cut;
        out.gain *= 1.0 - to_level * (0.5 - 0.5 * x);
    }
    out
}

/// One stack into a side: `side = 0 + a * pan` for the first stack (so the
/// side needs no clearing), `side += a * pan` after.
#[inline]
fn mix_stack(first: bool, side: &mut [f32], a: &[f32], pan: f32) {
    if first {
        for (x, y) in side.iter_mut().zip(a) {
            *x = 0.0 + y * pan;
        }
    } else {
        for (x, y) in side.iter_mut().zip(a) {
            *x += y * pan;
        }
    }
}

/// What an attack's label was, as a share of the time its old envelope
/// took to reach full level: it aimed at 1.3 with a coefficient covering
/// 99 % of the way in the label (`dsp::seg_coef`), so it arrived at
/// ln(1.3 / 0.3) / 4.6 of it. The new attack aims the same way and
/// arrives at its label, so the curve is the same.
pub const OLD_ATTACK: f32 = 0.318_769;
/// The release's: the old one aimed at -0.01 with the same coefficient,
/// as the new one does ([`Adsr::analog_release`]), and fell 60 dB at
/// ln(1.01 / 0.011) / 4.6 of its label; the new one falls 60 dB at its
/// label, so the curve is the same.
pub const OLD_RELEASE: f32 = 0.982_567;

/// Setting `id`'s `value` as a sound saved in preset format `format` had
/// it, in this build's meaning ([`Node::upgrade`]). Format 1's envelope
/// times: an attack reached full level at [`OLD_ATTACK`] of its label, a
/// release fell 60 dB at [`OLD_RELEASE`] of it. Its
/// velocity to level was a share: a note at velocity 1 played at
/// `1 - share` (plus a square law), now a range in dB down to velocity 1.
/// Decay is unchanged (the old one covered 98.99 % of the way in its
/// time, the new one 99 %).
pub fn upgrade(format: u32, id: u16, value: f32) -> f32 {
    if format >= 2 {
        return value;
    }
    match id {
        AMP_A | FENV_A => value * OLD_ATTACK,
        AMP_R | FENV_R => value * OLD_RELEASE,
        VEL_AMP => {
            let share = (value / 100.0).clamp(0.0, 1.0);
            let softest = (1.0 - share) + share / (127.0 * 127.0);
            -20.0 * softest.log10()
        }
        _ => value,
    }
}

impl Node for Poly {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        PARAMS
    }

    fn set_param(&mut self, id: u16, value: f32) {
        let Some(p) = info(id) else { return };
        if p.readonly {
            return;
        }
        self.values[id as usize] = p.clamp(value);
        self.update_targets();
        if matches!(
            id,
            AMP_A | AMP_D | AMP_S | AMP_R | FENV_A | FENV_D | FENV_S | FENV_R
        ) {
            self.update_envs();
        }
        if (PRESS_BASE..PRESS_BASE + mpfree_node::pressure::FIELDS as u16).contains(&id) {
            self.update_pressure();
        }
        if (LFO2_SHAPE..=LFO2_AMP).contains(&id) {
            self.update_lfo2();
        }
        if route_of(id).is_some() {
            self.update_routes();
        }
        if id == SUPER_DETUNE || id == SUPER_MIX {
            self.sup = Super::new(self.v(SUPER_DETUNE) / 100.0, self.v(SUPER_MIX) / 100.0);
        }
        if id == VEL_AMP {
            self.vel = VelCurve::new(self.v(VEL_AMP));
        }
        if id == VOICE_MODE {
            self.all_off();
        }
    }

    fn param(&self, id: u16) -> f32 {
        match id {
            METER_VOICES => self.meter_voices,
            _ if info(id).is_some() => self.values[id as usize],
            _ => f32::NAN,
        }
    }

    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(8_000);
        self.dc_l = DcBlock::new(self.rate, DC_HZ);
        self.dc_r = DcBlock::new(self.rate, DC_HZ);
        self.dc_live = false;
        self.update_envs();
        self.update_pressure();
        self.update_routes();
        self.update_lfo2();
        self.reset_smoothers();
        self.control(&Ctx::new(self.rate, 0), 0);
    }

    fn reset(&mut self) {
        for (i, v) in self.voices.iter_mut().enumerate() {
            *v = Voice::new(i);
        }
        self.update_envs();
        self.update_lfo2();
        self.reset_smoothers();
        self.chan = Channel::default();
        self.chan_s = Channel::default();
        self.stack_len = 0;
        self.exprs.clear();
        self.lfo_phase = 0.0;
        self.vib_phase = 0.0;
        self.chord.clear();
        self.meter_voices = 0.0;
        self.dc_l.reset();
        self.dc_r.reset();
        self.dc_live = false;
        self.control(&Ctx::new(self.rate, 0), 0);
    }

    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        assert_eq!(left.len(), right.len());
        if ctx.rate != self.rate {
            self.prepare(ctx.rate, left.len());
        }
        let len = left.len();
        // Events land on their frames; a control step runs on every
        // CTRL-aligned absolute frame.
        let mut at = 0usize;
        let mut i = 0usize;
        while at < len {
            while i < events.len() && events[i].offset as usize <= at {
                self.apply(&events[i].ev);
                i += 1;
            }
            let next = events.get(i).map_or(len, |e| (e.offset as usize).min(len));
            while at < next {
                let abs = ctx.frame + at as u64;
                if abs.is_multiple_of(CTRL) {
                    self.control(ctx, at);
                }
                let end = next.min(at + (CTRL - abs % CTRL) as usize);
                self.render(at, end, left, right);
                at = end;
            }
        }
        // Events at or past the end (a host error) still apply, so no note hangs.
        for e in &events[i..] {
            self.apply(&e.ev);
        }
        self.frame = ctx.frame + len as u64;
        self.meter_voices = self.voices.iter().filter(|v| v.sounding()).count() as f32;
    }

    fn tail_frames(&self) -> u32 {
        // The release reaches silence at 1.02 times its label.
        let ms = self.v(AMP_R).max(KILL_MS);
        (ms * 0.001 * self.rate as f32 * 1.5) as u32
    }

    fn upgrade(&self, format: u32, id: u16, value: f32) -> f32 {
        upgrade(format, id, value)
    }

    fn set_tuning(&mut self, tuning: &Tuning) {
        if self.tuning == *tuning {
            return;
        }
        self.tuning.clone_from(tuning);
        // Every voice works its pitch out again on its next control step.
        for v in &mut self.voices {
            v.pitch_key = [NO_KEY; 8];
        }
    }

    fn sounding_voices(&self) -> usize {
        self.active_voices()
    }

    fn cull_candidate(&self) -> Option<Cull> {
        self.cull_pick().map(|(_, c)| c)
    }

    fn cull(&mut self) -> bool {
        let Some((i, _)) = self.cull_pick() else {
            return false;
        };
        self.fade_out(i);
        true
    }
}
